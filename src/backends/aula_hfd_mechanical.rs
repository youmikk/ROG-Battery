//! Exact AULA mechanical receivers using the official 56-byte HFD info reader.
//! Other shared-ID models and auxiliary channels are not protocol fallbacks.
use super::*;
use crate::{hid::HidDevice, win32::*};
use std::{collections::BTreeMap, time::{Duration, Instant}};
use winapi::um::cfgmgr32::{CM_Get_Device_IDW, CM_Get_Parent, CM_Locate_DevNodeW};

pub struct AulaHfdMechanical;

const INFO_BYTES: usize = 56;
const TRANSACTION_LIMIT: Duration = Duration::from_secs(2);
const UNAVAILABLE: &str = "设备无有效状态回报";

#[derive(Clone, Copy)]
struct Model {
    product: &'static str,
    name: &'static str,
    page: u16,
    usage: u16,
    firmware: &'static [u16],
}

const MODELS: &[Model] = &[
    Model { product: "AULA F108ProV2C Dongle", name: "AULA F108 Pro V2C",
        page: 0xff68, usage: 0x61, firmware: &[0x273a, 0x2749] },
    Model { product: "AULA F87ProV2C Dongle", name: "AULA F87 Pro V2C",
        page: 0xff68, usage: 0x61, firmware: &[0x2712, 0x2738, 0x2747] },
    Model { product: "AULA NOVA98 Dongle", name: "AULA NOVA98",
        page: 0xff80, usage: 0x71, firmware: &[0x273b, 0x275d] },
];

fn model(i: &HidInterface) -> Option<&'static Model> {
    if i.vendor_id != 0x0c45 || i.product_id != 0xfefc { return None; }
    MODELS.iter().find(|m| i.product == m.product && i.usage_page == m.page && i.usage == m.usage)
}

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x0c45 && i.product_id == 0xfefc
        && MODELS.iter().any(|m| i.usage_page == m.page && i.usage == m.usage)
}

fn hidden_management(skipped: &[crate::hid::Skipped]) -> bool {
    skipped.iter().any(|entry| {
        let path = entry.path.to_ascii_lowercase();
        path.split('#').nth(1).is_some_and(|hardware| {
            let fields: Vec<_> = hardware.split('&').collect();
            fields.contains(&"vid_0c45") && fields.contains(&"pid_fefc")
        })
    })
}

fn device_instance(i: &HidInterface) -> Option<String> {
    if !management(i) { return None; }
    let path = i.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid" || parts[2].is_empty() || parts[3].is_empty()
        || path.contains('\0') || parts[1].contains('\\') || parts[2].contains('\\') { return None; }
    let fields: Vec<_> = parts[1].split('&').collect();
    for (prefix, expected) in [("vid_", "0c45"), ("pid_", "fefc")] {
        if fields.iter().filter_map(|field| field.strip_prefix(prefix)).collect::<Vec<_>>() != [expected] {
            return None;
        }
    }
    for prefix in ["mi_", "col"] {
        let indices: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix(prefix)).collect();
        if indices.len() > 1 || indices.iter().any(|index|
            index.len() != 2 || !index.bytes().all(|b| b.is_ascii_hexdigit())) { return None; }
    }
    Some(format!("hid\\{}\\{}", parts[1], parts[2]))
}

fn physical_parent(name: &str) -> bool {
    let parts: Vec<_> = name.split('\\').collect();
    if parts.len() != 3 || parts[0] != "usb" || parts[2].is_empty() { return false; }
    let fields: Vec<_> = parts[1].split('&').collect();
    fields.len() == 2 && fields.contains(&"vid_0c45") && fields.contains(&"pid_fefc")
}

fn usb_parent(i: &HidInterface) -> Option<String> {
    let mut instance = to_wide(&device_instance(i)?);
    let mut node = 0;
    if unsafe { CM_Locate_DevNodeW(&mut node, instance.as_mut_ptr(), 0) } != 0 { return None; }
    for _ in 0..8 {
        let mut parent = 0;
        if unsafe { CM_Get_Parent(&mut parent, node, 0) } != 0 { return None; }
        let mut buffer = [0u16; 512];
        if unsafe { CM_Get_Device_IDW(parent, buffer.as_mut_ptr(), buffer.len() as u32, 0) } != 0 { return None; }
        let end = buffer.iter().position(|c| *c == 0)?;
        let name = String::from_utf16(&buffer[..end]).ok()?.to_ascii_lowercase();
        if physical_parent(&name) { return Some(name); }
        node = parent;
    }
    None
}

fn control(i: &HidInterface) -> bool {
    model(i).is_some() && matches!(i.input_len, 33 | 65)
        && i.output_len == i.input_len && device_instance(i).is_some()
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    control(i) && descriptor::Descriptor::open(api, i).is_some_and(|d|
        d.single_input_output_byte_report_ids(i, i.input_len, i.output_len) == Some((0, 0)))
}

fn select_with<'a>(interfaces: &'a [HidInterface],
    mut parent: impl FnMut(&HidInterface) -> Option<String>,
    mut proof: impl FnMut(&HidInterface) -> bool) -> Vec<(&'a HidInterface, String)> {
    let mut groups: BTreeMap<String, Vec<&HidInterface>> = BTreeMap::new();
    for i in interfaces.iter().filter(|i| management(i)) {
        let Some(key) = parent(i).filter(|key| physical_parent(key)) else { return Vec::new(); };
        groups.entry(key).or_default().push(i);
    }
    groups.into_iter().filter_map(|(parent, members)| {
        let [i] = members.as_slice() else { return None; };
        (control(i) && proof(i)).then_some((*i, parent))
    }).collect()
}

// Callers hold the actual USB parent lock while refreshing this selection and
// proving its descriptor. Other receivers' descriptors are not opened here.
fn current<'a>(api: &HidApi, interfaces: &'a [HidInterface], original: &HidInterface,
    parent: &str) -> Option<&'a HidInterface> {
    let candidates = select_with(interfaces, usb_parent, |_| true);
    let (i, _) = candidates.into_iter().find(|(i, current_parent)|
        i.path.eq_ignore_ascii_case(&original.path) && current_parent.as_str() == parent
            && i.product == original.product)?;
    proven(api, i).then_some(i)
}

fn target(i: &HidInterface) -> Target {
    let mut t = Target::new(BackendId::AulaHfdMechanical, i, 0);
    if let Some(model) = model(i) { t.name = model.name.into(); }
    t.family = DeviceFamily::Keyboard;
    t
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Page { offset: u8, length: u8, last: bool }

fn pages(report_bytes: u16) -> Option<Vec<Page>> {
    let capacity = match report_bytes { 33 => 24usize, 65 => 56, _ => return None };
    Some((0..INFO_BYTES).step_by(capacity).map(|offset| {
        let length = (INFO_BYTES - offset).min(capacity);
        Page { offset: offset as u8, length: length as u8, last: offset + length == INFO_BYTES }
    }).collect())
}

fn request(report_bytes: u16, page: Page) -> Option<Vec<u8>> {
    if !pages(report_bytes)?.contains(&page) { return None; }
    let mut frame = vec![0; usize::from(report_bytes)];
    frame[..9].copy_from_slice(&[0, 0xaa, 0x10, page.length, page.offset, 0, 0, u8::from(page.last), 0]);
    Some(frame)
}

fn response(frame: &[u8], report_bytes: u16, page: Page) -> Option<&[u8]> {
    if !pages(report_bytes)?.contains(&page) || frame.len() != usize::from(report_bytes - 1)
        || frame[..2] != [0x55, 0x10] || frame[3..5] != [page.offset, 0] { return None; }
    // The SDK calls byte2 lenOrType and does not establish reply-tail flags or
    // checksum. Match command/address and require the complete requested data.
    frame.get(8..8 + usize::from(page.length))
}

fn identity(head: &[u8], model: &Model) -> bool {
    if head.len() < 19 { return false; }
    u16::from_le_bytes([head[4], head[5]]) == 0x38a6
        && model.firmware.contains(&u16::from_le_bytes([head[6], head[7]]))
        && (1..=100).contains(&head[17])
}

fn read_with(t: &Target, mut exchange: impl FnMut(Page, &[u8]) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    let i = t.hid()?;
    if t.backend != BackendId::AulaHfdMechanical || !control(i) { return Err(UNAVAILABLE.into()); }
    let model = model(i).ok_or(UNAVAILABLE)?;
    let mut data = Vec::with_capacity(INFO_BYTES);
    let mut measured = None;
    for page in pages(i.output_len).ok_or(UNAVAILABLE)? {
        let request = request(i.output_len, page).ok_or(UNAVAILABLE)?;
        let frame = exchange(page, &request).ok_or(UNAVAILABLE)?;
        let part = response(&frame, i.input_len, page).ok_or(UNAVAILABLE)?;
        if page.offset == 0 {
            // A foreign model or unavailable battery must stop
            // before any subsequent GET_INFO page is sent.
            if !identity(part, model) { return Err(UNAVAILABLE.into()); }
            measured = Some(Instant::now());
        }
        data.extend_from_slice(part);
    }
    if data.len() != INFO_BYTES || !identity(&data, model) { return Err(UNAVAILABLE.into()); }
    let mut target = target(i);
    target.verified_identity = true;
    Ok(Snapshot { target, battery: Some(Battery { percent: Some(data[17]), estimated_percent: None,
        charging: None, voltage_mv: None, approximate: false, level: None }),
        battery_at: measured, battery_unavailable: false, connected: true,
        dpi: Dpi::default(), polling_rate: Default::default() })
}

impl DeviceBackend for AulaHfdMechanical {
    fn discover(&self, api: &HidApi, _: &[HidInterface]) -> Result<Vec<Target>, String> {
        let report = crate::hid::enumerate_all_report(api)?;
        if hidden_management(&report.skipped) { return Ok(Vec::new()); }
        let candidates = select_with(&report.interfaces, usb_parent, |_| true);
        let mut targets = Vec::new();
        for (original, parent) in candidates {
            let lock = channel::lock(&parent);
            let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
            let fresh = crate::hid::enumerate_all_report(api)?;
            if hidden_management(&fresh.skipped) { continue; }
            if let Some(i) = current(api, &fresh.interfaces, original, &parent) {
                targets.push(target(i));
            }
        }
        Ok(targets)
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        if t.backend != BackendId::AulaHfdMechanical || !control(t.hid()?) { return Err(UNAVAILABLE.into()); }
        let original = t.hid()?;
        let parent = usb_parent(original).ok_or(UNAVAILABLE)?;
        let lock = channel::lock(&parent);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        // Re-enumerate after taking the physical receiver lock. A skipped
        // same-ID interface can hide a second management collection.
        let report = crate::hid::enumerate_all_report(api)?;
        if hidden_management(&report.skipped) { return Err(UNAVAILABLE.into()); }
        let i = current(api, &report.interfaces, original, &parent).ok_or(UNAVAILABLE)?;
        let model = model(i).ok_or(UNAVAILABLE)?;
        let mut device = HidDevice::open(i)?;
        let transaction = Instant::now();
        read_with(&target(i), |page, request| {
            let left = TRANSACTION_LIMIT.checked_sub(transaction.elapsed())?;
            let timeout = left.as_millis().min(600) as u32;
            if timeout == 0 { return None; }
            let started = Instant::now();
            let mut remaining = 32;
            let frame = device.transact_with_input_report(request, 0, timeout, |frame| {
                remaining -= 1;
                (frame.len() >= 5 && frame[..2] == [0x55, 0x10] && frame[3..5] == [page.offset, 0])
                    || remaining == 0
            });
            let valid = frame.as_deref().and_then(|r| response(r, i.input_len, page))
                .is_some_and(|data| page.offset != 0 || identity(data, model));
            trace::record(i, BackendId::AulaHfdMechanical, "query-battery", u16::from(page.offset), 0x10,
                request.len(), frame.as_ref().map_or(0, Vec::len), started,
                if valid { trace::Outcome::Ok } else if frame.is_some() { trace::Outcome::InvalidData }
                else { trace::Outcome::NoReply });
            frame
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(instance: &str, bytes: u16) -> HidInterface {
        interface_for(&MODELS[0], instance, bytes)
    }
    fn interface_for(model: &Model, instance: &str, bytes: u16) -> HidInterface {
        // Synthetic MI/collection values, not captured hardware descriptors.
        HidInterface { path: format!(r"\\?\hid#vid_0c45&pid_fefc&mi_03&col01#8&{instance}&0&0000#{{hid}}"),
            vendor_id: 0x0c45, product_id: 0xfefc, usage_page: model.page, usage: model.usage,
            input_len: bytes, output_len: bytes, feature_len: 0, product: model.product.into(), serial: "shared".into() }
    }
    fn parent(instance: &str) -> String { format!(r"usb\vid_0c45&pid_fefc\{instance}") }
    fn data(pid: u16, percent: u8) -> [u8; INFO_BYTES] {
        let mut bytes = [0; INFO_BYTES];
        bytes[4..6].copy_from_slice(&0x38a6u16.to_le_bytes());
        bytes[6..8].copy_from_slice(&pid.to_le_bytes());
        bytes[16] = 2; bytes[17] = percent;
        bytes
    }
    fn response_fixture(bytes: u16, page: Page, data: &[u8; INFO_BYTES]) -> Vec<u8> {
        let mut frame = vec![0; usize::from(bytes - 1)];
        frame[..8].copy_from_slice(&[0x55, 0x10, page.length, page.offset, 0, 0, u8::from(page.last), 0]);
        frame[8..8 + usize::from(page.length)].copy_from_slice(
            &data[usize::from(page.offset)..usize::from(page.offset + page.length)]);
        frame
    }

    #[test]
    fn runtime_report_size_drives_complete_56_byte_page_schedule() {
        for model in MODELS {
            for (bytes, pid) in [33, 65].into_iter().flat_map(|bytes| model.firmware.iter().map(move |pid| (bytes, *pid))) {
                let t = target(&interface_for(model, "one", bytes));
                let mut calls = vec![];
                let snapshot = read_with(&t, |page, packet| {
                    calls.push(page.offset);
                    assert_eq!(packet.len(), usize::from(bytes));
                    assert_eq!(&packet[..9], &[0, 0xaa, 0x10, page.length, page.offset, 0, 0, u8::from(page.last), 0]);
                    assert!(packet[9..].iter().all(|b| *b == 0));
                    Some(response_fixture(bytes, page, &data(pid, 64)))
                }).unwrap();
                assert_eq!(calls, if bytes == 33 { vec![0, 24, 48] } else { vec![0] });
                assert!(snapshot.connected && snapshot.target.verified_identity && snapshot.battery_at.is_some());
                assert_eq!(snapshot.target.name, model.name);
                assert_eq!(snapshot.battery.unwrap().percent, Some(64));
                assert!(!snapshot.dpi.writable && !snapshot.polling_rate.writable);
            }
        }
        assert_eq!(pages(33).unwrap().iter().map(|p| p.length).collect::<Vec<_>>(), [24, 24, 8]);
        for invalid in [0, 32, 34, 64, 66, 4096] { assert!(pages(invalid).is_none()); }
    }

    #[test]
    fn exact_product_channel_and_actual_usb_parent_exclude_shared_ids() {
        let i = interface("one", 33);
        assert!(control(&i));
        for name in ["AULA F108ProV2 Dongle", "F108Pro Dongle", "F99 MAX Dongle", "", "AULA F108ProV2C"] {
            let mut other = i.clone(); other.product = name.into(); assert!(!control(&other));
        }
        for model in MODELS {
            let valid = interface_for(model, "one", 33);
            assert!(control(&valid));
            for (page, usage) in [(0xff67, 0x61), (0xff70, 0x71), (model.page, model.usage + 1)] {
                let mut other = valid.clone(); other.usage_page = page; other.usage = usage;
                assert!(!control(&other));
                assert!(read_with(&target(&other), |_, _| panic!("unsupported interface")).is_err());
            }
        }
        let mut nova_on_wrong_channel = i.clone(); nova_on_wrong_channel.product = MODELS[2].product.into();
        assert!(!control(&nova_on_wrong_channel));
        let mut c_on_wrong_channel = interface_for(&MODELS[2], "one", 33);
        c_on_wrong_channel.product = MODELS[0].product.into(); assert!(!control(&c_on_wrong_channel));
        for (vid, pid, page, usage, input, output) in [(0x05ac, 0x024f, 0xff68, 0x61, 33, 33),
            (0x38a6, 0x273a, 0xff68, 0x61, 33, 33), (0x0c45, 0xfefe, 0xff68, 0x61, 33, 33),
            (0x0c45, 0xfefc, 0xff67, 0x61, 33, 33), (0x0c45, 0xfefc, 0xff60, 0x61, 33, 33),
            (0x0c45, 0xfefc, 1, 6, 33, 33), (0x0c45, 0xfefc, 0xff68, 0x61, 32, 33),
            (0x0c45, 0xfefc, 0xff68, 0x61, 33, 65)] {
            let mut other = i.clone(); other.vendor_id = vid; other.product_id = pid;
            other.usage_page = page; other.usage = usage; other.input_len = input; other.output_len = output;
            assert!(!control(&other));
        }
        assert!(physical_parent(&parent("one")));
        for bad in [r"usb\vid_0c45&pid_fefc&mi_03\one", r"hid\vid_0c45&pid_fefc\one", r"usb\vid_0c45&pid_fefe\one"] {
            assert!(!physical_parent(bad));
        }
    }

    #[test]
    fn ambiguity_is_checked_before_report_proof_and_instances_are_independent() {
        let one = interface("one", 33); let two = interface("two", 33);
        let pick = |i: &HidInterface| Some(parent(if i.path.contains("&two&") { "two" } else { "one" }));
        assert_eq!(select_with(&[one.clone(), two.clone()], pick, |_| true).len(), 2);
        assert_ne!(target(&one).key, target(&two).key);
        let mut sibling = one.clone(); sibling.product = "generic".into(); sibling.input_len = 6;
        sibling.path = sibling.path.replace("col01", "col02").replace("0000#", "0001#");
        assert!(select_with(&[one.clone(), sibling], pick, |_| panic!("ambiguous receiver")).is_empty());
        let mut other_usage = interface_for(&MODELS[2], "one", 33);
        other_usage.path = other_usage.path.replace("col01", "col03");
        assert!(select_with(&[one.clone(), other_usage], pick, |_| panic!("ambiguous management usages")).is_empty());
        assert!(select_with(&[one.clone()], |_| None, |_| panic!("unknown parent")).is_empty());
        assert!(select_with(&[one], pick, |_| false).is_empty());
    }

    #[test]
    fn skipped_shared_id_collection_blocks_proof_but_unrelated_ids_do_not() {
        let blocked = |path: String| hidden_management(&[crate::hid::Skipped { path, reason: "busy".into() }]);
        let i = interface("one", 33);
        assert!(blocked(i.path.clone()));
        assert!(blocked(i.path.to_ascii_uppercase()));
        assert!(blocked(i.path.replace("mi_03&col01", "mi_04&col02")));
        assert!(!blocked(i.path.replace("pid_fefc", "pid_fefe")));
        assert!(!blocked(i.path.replace("0c45", "05ac")));
        assert!(!blocked(i.path.replace("fefc", "fefca")));
        assert!(!hidden_management(&[]));
    }

    #[test]
    fn shared_receiver_ids_cannot_accept_another_models_firmware() {
        for model in MODELS {
            let t = target(&interface_for(model, "one", 33));
            for foreign in MODELS.iter().filter(|other| other.product != model.product) {
                for pid in foreign.firmware {
                    let mut queries = 0;
                    assert!(read_with(&t, |page, _| {
                        queries += 1;
                        Some(response_fixture(33, page, &data(*pid, 64)))
                    }).is_err());
                    assert_eq!(queries, 1);
                }
            }
        }
    }

    #[test]
    fn foreign_firmware_or_invalid_battery_stops_before_next_query() {
        let t = target(&interface("one", 33));
        let good = data(0x273a, 40);
        for (index, value) in [(4, 0x45), (6, 0x39), (6, 0x4b), (17, 0), (17, 101), (17, 255)] {
            let mut invalid = good; invalid[index] = value;
            let mut queries = 0;
            assert!(read_with(&t, |page, _| { queries += 1; Some(response_fixture(33, page, &invalid)) }).is_err());
            assert_eq!(queries, 1);
        }
        for charge in [0, 1, 2, 255] {
            let mut info = good; info[16] = charge; info[18] = charge;
            let sample = read_with(&t, |page, _| Some(response_fixture(33, page, &info))).unwrap();
            assert_eq!(sample.battery.unwrap().charging, None);
        }
    }

    #[test]
    fn stale_pages_truncation_and_missing_tail_cannot_create_a_sample() {
        let t = target(&interface("one", 33));
        let info = data(0x273a, 66);
        let list = pages(33).unwrap();
        let good = response_fixture(33, list[0], &info);
        for length in 0..good.len() { assert!(response(&good[..length], 33, list[0]).is_none()); }
        let mut long = good.clone(); long.push(0); assert!(response(&long, 33, list[0]).is_none());
        let mut with_id = vec![0]; with_id.extend(good); assert!(response(&with_id, 33, list[0]).is_none());
        assert!(response(&response_fixture(33, list[1], &info), 33, list[0]).is_none());
        let mut high_address = response_fixture(33, list[0], &info); high_address[4] = 1;
        assert!(response(&high_address, 33, list[0]).is_none());
        let mut queries = 0;
        assert!(read_with(&t, |page, _| { queries += 1;
            (page.offset == 0).then(|| response_fixture(33, page, &info)) }).is_err());
        assert_eq!(queries, 2);
        let mut wrong = t; wrong.backend = BackendId::AulaF108;
        assert!(read_with(&wrong, |_, _| panic!("wrong backend")).is_err());
    }
}
