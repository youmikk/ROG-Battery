//! ATK/VXE NANO receiver 373B:1216, BITMOUSE battery GET on FF05:0001.
//! The shared receiver does not identify its paired retail mouse model.
use super::*;
use crate::{hid::HidDevice, win32::to_wide};
use std::{collections::BTreeMap, time::Instant};
use winapi::um::cfgmgr32::{CM_Locate_DevNodeW, CM_Get_Parent, CM_Get_Device_IDW};

pub struct AtkNano;
const OUTPUT_ID: u8 = 8;
const OUTPUT_LENGTH: u16 = 64;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x373b && i.product_id == 0x1216
        && i.usage_page == 0xff05 && i.usage == 1
}

fn capacities(i: &HidInterface) -> bool {
    management(i) && (8..=64).contains(&i.input_len) && i.output_len == OUTPUT_LENGTH
}

fn device_instance(i: &HidInterface) -> Option<String> {
    if !management(i) { return None; }
    let path = i.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid" || parts[2].is_empty()
        || parts[3].is_empty() || path.contains('\0')
        || parts[1].contains('\\') || parts[2].contains('\\') { return None; }
    let fields: Vec<_> = parts[1].split('&').collect();
    for required in ["vid_373b", "pid_1216"] {
        if fields.iter().filter(|field| **field == required).count() != 1 { return None; }
    }
    if fields.iter().filter(|field| field.starts_with("vid_")).count() != 1
        || fields.iter().filter(|field| field.starts_with("pid_")).count() != 1 { return None; }
    for prefix in ["mi_", "col"] {
        let indices: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix(prefix)).collect();
        if indices.len() > 1 || indices.iter().any(|index|
            index.len() != 2 || !index.bytes().all(|b| b.is_ascii_hexdigit())) { return None; }
    }
    Some(format!("hid\\{}\\{}", parts[1], parts[2]))
}

fn physical_parent_name(name: &str) -> bool {
    let parts: Vec<_> = name.split('\\').collect();
    if parts.len() != 3 || parts[0] != "usb" || parts[2].is_empty() { return false; }
    let fields: Vec<_> = parts[1].split('&').collect();
    fields.len() == 2 && fields.contains(&"vid_373b") && fields.contains(&"pid_1216")
}

fn usb_parent(i: &HidInterface) -> Option<String> {
    let mut instance = to_wide(&device_instance(i)?);
    let mut node = 0;
    if unsafe { CM_Locate_DevNodeW(&mut node, instance.as_mut_ptr(), 0) } != 0 { return None; }
    // Resolve the actual USB parent rather than joining unrelated MI paths or
    // assuming shared product/serial strings identify one physical receiver.
    for _ in 0..8 {
        let mut parent = 0;
        if unsafe { CM_Get_Parent(&mut parent, node, 0) } != 0 { return None; }
        let mut buffer = [0u16; 512];
        if unsafe { CM_Get_Device_IDW(parent, buffer.as_mut_ptr(), buffer.len() as u32, 0) } != 0 {
            return None;
        }
        let length = buffer.iter().position(|c| *c == 0)?;
        let name = String::from_utf16(&buffer[..length]).ok()?.to_ascii_lowercase();
        if physical_parent_name(&name) { return Some(name); }
        node = parent;
    }
    None
}

fn report_id(api: &HidApi, i: &HidInterface) -> Option<u8> {
    if !capacities(i) || device_instance(i).is_none() { return None; }
    let d = descriptor::Descriptor::open(api, i)?;
    let (input, output) = d.single_input_output_byte_report_ids(i, i.input_len, OUTPUT_LENGTH)?;
    (output == OUTPUT_ID).then_some(input)
}

fn select_with<'a>(interfaces: &'a [HidInterface],
    mut parent: impl FnMut(&HidInterface) -> Option<String>,
    mut proof: impl FnMut(&HidInterface) -> Option<u8>) -> Vec<(&'a HidInterface, String, u8)> {
    let mut groups: BTreeMap<String, Vec<&HidInterface>> = BTreeMap::new();
    for i in interfaces.iter().filter(|i| management(i)) {
        // Unknown topology could conceal a second collection of a known parent.
        let Some(key) = parent(i).filter(|key| physical_parent_name(key)) else { return Vec::new(); };
        groups.entry(key).or_default().push(i);
    }
    let mut result = Vec::new();
    for (parent, members) in groups {
        let [i] = members.as_slice() else { continue; };
        if !capacities(i) || device_instance(i).is_none() { continue; }
        if let Some(input) = proof(i) { result.push((*i, parent, input)); }
    }
    result
}

fn target(i: &HidInterface, parent: &str) -> Target {
    let mut t = Target::new(BackendId::AtkNano, i, 0);
    t.key = crate::device::stable_key(&format!("atk-nano:{parent}"));
    t.name = "ATK / VXE NANO".into();
    t.family = DeviceFamily::Mouse;
    t
}

fn request() -> [u8; OUTPUT_LENGTH as usize] {
    let mut r = [0; OUTPUT_LENGTH as usize];
    r[..8].copy_from_slice(&[OUTPUT_ID, 0x7d, 0x72, 2, 0, 1, 7, 1]);
    r
}

fn battery(r: &[u8], input: u8, capacity: u16) -> Option<Battery> {
    if !(8..=64).contains(&capacity) { return None; }
    let body = if input == 0 {
        // HidDevice strips the Windows zero ID slot after reading full capacity.
        if r.len() != usize::from(capacity) - 1 { return None; }
        r
    } else {
        if r.len() != usize::from(capacity) || r[0] != input { return None; }
        &r[1..]
    };
    // Published receiver reply: marker, success, sequence, host target, GET07,
    // one data byte. Sequence is not a charging flag; trailing bytes are old data.
    if body[0] != 0x72 || body[1] != 0 || body[3] != 0 || body[4] != 7 || body[5] != 1
        || !(1..=100).contains(&body[6]) { return None; }
    Some(Battery { percent: Some(body[6]), charging: None, estimated_percent: None,
        voltage_mv: None, approximate: false, level: None })
}

fn read_with(t: &Target, input: u8, exchange: impl FnOnce(&[u8]) -> Option<Vec<u8>>)
    -> Result<Snapshot, String> {
    let i = t.hid()?;
    if t.backend != BackendId::AtkNano || !capacities(i) || device_instance(i).is_none() {
        return Err(UNAVAILABLE.into());
    }
    let response = exchange(&request()).ok_or(UNAVAILABLE)?;
    let battery = battery(&response, input, i.input_len).ok_or(UNAVAILABLE)?;
    let mut target = t.clone();
    target.verified_identity = true;
    Ok(Snapshot { target, battery: Some(battery), battery_at: Some(Instant::now()),
        battery_unavailable: false, connected: true, dpi: Dpi::default(),
        polling_rate: Default::default() })
}

impl DeviceBackend for AtkNano {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(select_with(interfaces, usb_parent, |i| report_id(api, i)).into_iter()
            .map(|(i, parent, _)| target(i, &parent)).collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        if t.backend != BackendId::AtkNano { return Err(UNAVAILABLE.into()); }
        // Recheck the selected physical receiver and report shape before each
        // query, so a changed or duplicated collection cannot reuse old proof.
        let interfaces = crate::hid::enumerate_all_report(api)?.interfaces;
        let candidates = select_with(&interfaces, usb_parent, |i| report_id(api, i));
        let (i, parent, input) = candidates.into_iter()
            .find(|(i, _, _)| t.hid().is_ok_and(|original| i.path.eq_ignore_ascii_case(&original.path)))
            .ok_or(UNAVAILABLE)?;
        let lock = channel::lock(&parent);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let current = target(i, &parent);
        if current.key != t.key { return Err(UNAVAILABLE.into()); }
        let mut device = HidDevice::open(i)?;
        read_with(&current, input, |request| {
            let started = Instant::now();
            let mut remaining = 32;
            let response = device.transact_with_input_report(request, input, 1_000, |r| {
                remaining -= 1;
                battery(r, input, i.input_len).is_some() || remaining == 0
            });
            trace::record(i, BackendId::AtkNano, "query-battery", 7, OUTPUT_ID,
                request.len(), response.as_ref().map_or(0, |r| r.len() + usize::from(input == 0)),
                started, match response.as_deref() {
                    Some(r) if battery(r, input, i.input_len).is_some() => trace::Outcome::Ok,
                    Some(_) => trace::Outcome::InvalidData,
                    None => trace::Outcome::NoReply,
                });
            response
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(instance: &str) -> HidInterface {
        HidInterface { path: format!(r"\\?\hid#vid_373b&pid_1216&mi_01&col01#8&{instance}&0&0000#{{hid}}"),
            vendor_id: 0x373b, product_id: 0x1216, usage_page: 0xff05, usage: 1,
            input_len: 64, output_len: 64, feature_len: 0,
            product: "NK mouse NANO dongle".into(), serial: "shared".into() }
    }

    fn parent(instance: &str) -> String { format!(r"usb\vid_373b&pid_1216\{instance}") }

    fn synthetic_frame(input: u8, capacity: u16, percent: u8) -> Vec<u8> {
        let mut body = vec![0; usize::from(capacity) - 1];
        body[..7].copy_from_slice(&[0x72, 0, 5, 0, 7, 1, percent]);
        if input == 0 { body } else { let mut r = vec![input]; r.extend(body); r }
    }

    #[test]
    fn fixed_query_is_only_the_official_receiver_battery_get() {
        let r = request();
        assert_eq!(&r[..8], &[8, 0x7d, 0x72, 2, 0, 1, 7, 1]);
        assert_eq!(r[1], r[2..].iter().fold(0u8, |sum, b| sum.wrapping_add(*b)));
        assert!(r[8..].iter().all(|b| *b == 0));
        assert!(battery(&r, 8, 64).is_none());
    }

    #[test]
    fn public_nano_prefix_retains_percent_but_not_unproven_charging_or_voltage() {
        // atk-tool #15 publishes this prefix; the remainder is synthetic padding.
        let mut r = vec![0; 64];
        r[..11].copy_from_slice(&[8, 0x72, 0, 5, 0, 7, 1, 0x5a, 0xbc, 0x10, 0]);
        let b = battery(&r, 8, 64).unwrap();
        assert_eq!((b.percent, b.charging, b.voltage_mv), (Some(90), None, None));
        r[3] = 0x3b;
        assert_eq!(battery(&r, 8, 64).unwrap().charging, None);
        for percent in 1..=100 {
            assert_eq!(battery(&synthetic_frame(8, 64, percent), 8, 64).unwrap().percent, Some(percent));
        }
        for percent in [0, 101, 127, 128, 255] {
            assert!(battery(&synthetic_frame(8, 64, percent), 8, 64).is_none());
        }
    }

    #[test]
    fn runtime_input_id_and_exact_capacity_control_normalization() {
        for input in [0, 1, 8, 0x80, 0xff] {
            for capacity in 8..=64 {
                let r = synthetic_frame(input, capacity, 90);
                assert_eq!(battery(&r, input, capacity).unwrap().percent, Some(90));
                let mut short = r.clone(); short.pop();
                assert!(battery(&short, input, capacity).is_none());
                let mut long = r.clone(); long.push(0);
                assert!(battery(&long, input, capacity).is_none());
                if input != 0 {
                    let mut wrong = r; wrong[0] = input.wrapping_add(1);
                    assert!(battery(&wrong, input, capacity).is_none());
                }
            }
        }
        for (offset, value) in [(1, 0), (2, 2), (2, 0xff), (4, 1), (5, 0xf), (6, 0), (6, 2)] {
            let mut r = synthetic_frame(8, 64, 90); r[offset] = value;
            assert!(battery(&r, 8, 64).is_none());
        }
    }

    #[test]
    fn physical_parent_and_unique_collection_are_required_before_report_proof() {
        let one = interface("one");
        let two = interface("two");
        let pick_parent = |i: &HidInterface| Some(parent(if i.path.contains("&two&") { "two" } else { "one" }));
        assert_eq!(select_with(&[one.clone(), two.clone()], pick_parent, |_| Some(8)).len(), 2);
        assert_ne!(target(&one, &parent("one")).key, target(&two, &parent("two")).key);
        let mut sibling = one.clone();
        sibling.path = sibling.path.replace("mi_01", "mi_02").replace("col01", "col02");
        sibling.input_len = 7;
        assert!(select_with(&[one.clone(), sibling], pick_parent,
            |_| panic!("ambiguous parent must not inspect a command report")).is_empty());
        assert!(select_with(&[one.clone()], |_| None, |_| panic!("unknown parent")).is_empty());
        assert!(select_with(&[one.clone()], |_| Some("shared".into()), |_| panic!("invalid parent")).is_empty());
        assert!(select_with(&[one], pick_parent, |_| None).is_empty());
        assert!(!physical_parent_name(r"usb\vid_373b&pid_1216&mi_01\one"));
        assert!(!physical_parent_name(r"usb\vid_373b&pid_1178\one"));
    }

    #[test]
    fn compx_other_nano_receivers_and_firmware_collections_never_use_this_command() {
        let i = interface("one");
        assert!(capacities(&i));
        for (vid, pid, page, usage, input, output) in [
            (0x373b, 0x1216, 0xff02, 2, 17, 17),
            (0x373b, 0x1216, 0xffb1, 1, 64, 64),
            (0x373b, 0x1216, 0xffb3, 1, 64, 64),
            (0x373b, 0x1178, 0xff10, 1, 65, 65),
            (0x373b, 0x1263, 0xff05, 1, 64, 64),
            (0x373b, 0x1216, 0xff05, 2, 64, 64),
            (0x373b, 0x1216, 0xff05, 1, 7, 64),
            (0x373b, 0x1216, 0xff05, 1, 65, 64),
            (0x373b, 0x1216, 0xff05, 1, 64, 17),
        ] {
            let mut wrong = i.clone();
            wrong.vendor_id = vid; wrong.product_id = pid; wrong.usage_page = page;
            wrong.usage = usage; wrong.input_len = input; wrong.output_len = output;
            assert!(read_with(&target(&wrong, &parent("one")), 8,
                |_| panic!("ineligible interface must not send")).is_err());
        }
        for path in ["unstructured", r"\\?\hid#vid_373b&pid_1178&mi_01#one#{hid}",
            r"\\?\hid#vid_373b&pid_1216&pid_1178&mi_01#one#{hid}"] {
            let mut wrong = i.clone(); wrong.path = path.into();
            assert!(device_instance(&wrong).is_none());
        }
    }

    #[test]
    fn one_query_publishes_only_valid_samples_and_recovers_after_silence() {
        let t = target(&interface("one"), &parent("one"));
        for reply in [None, Some(vec![0; 64]), Some(synthetic_frame(8, 64, 0))] {
            let mut calls = 0;
            assert!(read_with(&t, 8, |request_bytes| {
                calls += 1; assert_eq!(request_bytes, request()); reply
            }).is_err());
            assert_eq!(calls, 1);
        }
        let sample = read_with(&t, 8, |_| Some(synthetic_frame(8, 64, 90))).unwrap();
        assert_eq!(sample.battery.unwrap().percent, Some(90));
        assert!(sample.battery_at.is_some() && sample.connected && !sample.battery_unavailable);
        assert!(sample.dpi.stages.is_empty() && !sample.dpi.writable && !sample.polling_rate.writable);
        let mut wrong = t; wrong.backend = BackendId::CompX;
        assert!(read_with(&wrong, 8, |_| panic!("wrong backend must not send")).is_err());
    }
}
