//! AULA V9Pro+ 320F:22BA receiver, FF1C:0092 Output/Input 04 status query.
//! Protocol evidence: MyFinder aa0228abbc850b693620d466147b35dfe50c8dca,
//! engine/providers/aula_mouse.py and engine/upstream-protocols.md. The public
//! 2026-11-18 captures cover 80/82 percent and unplugged/charging/unplugged.
//! Only the battery GET is implemented; no DPI, settings or firmware writes.
use super::*;
use crate::{hid::HidDevice, win32::to_wide};
use std::{collections::BTreeMap, time::Instant};
use winapi::um::cfgmgr32::{CM_Get_Device_IDW, CM_Get_Parent, CM_Locate_DevNodeW};

pub struct AulaV9Pro;

const PRODUCT: &str = "AULA-V9Pro+";
const REPORT: u8 = 0x04;
const LENGTH: u16 = 64;
const COMMAND: u8 = 0x1a;
const TIMEOUT_MS: u32 = 1_000;
const MAX_REPLIES: usize = 32;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x320f && i.product_id == 0x22ba && i.usage_page == 0xff1c && i.usage == 0x0092
}

fn has_unresolved_collection(skipped: &[crate::hid::Skipped]) -> bool {
    skipped.iter().any(|entry| {
        let path = entry.path.to_ascii_lowercase();
        path.contains("vid_320f") && path.contains("pid_22ba")
    })
}

fn device_instance(i: &HidInterface) -> Option<String> {
    if !management(i) {
        return None;
    }
    let path = i.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4
        || parts[0] != r"\\?\hid"
        || parts[2].is_empty()
        || parts[3].is_empty()
        || path.contains('\0')
        || parts[1].contains('\\')
        || parts[2].contains('\\')
    {
        return None;
    }
    let fields: Vec<_> = parts[1].split('&').collect();
    for required in ["vid_320f", "pid_22ba"] {
        if fields.iter().filter(|field| **field == required).count() != 1 {
            return None;
        }
    }
    if fields
        .iter()
        .filter(|field| field.starts_with("vid_"))
        .count()
        != 1
        || fields
            .iter()
            .filter(|field| field.starts_with("pid_"))
            .count()
            != 1
    {
        return None;
    }
    for prefix in ["mi_", "col"] {
        let indices: Vec<_> = fields
            .iter()
            .filter_map(|field| field.strip_prefix(prefix))
            .collect();
        if indices.len() > 1
            || indices.iter().any(|index| {
                index.len() != 2 || !index.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return None;
        }
    }
    Some(format!("hid\\{}\\{}", parts[1], parts[2]))
}

fn eligible(i: &HidInterface) -> bool {
    i.product == PRODUCT
        && i.input_len == LENGTH
        && i.output_len == LENGTH
        && device_instance(i).is_some_and(|instance| {
            instance
                .split('\\')
                .nth(1)
                .is_some_and(|fields| fields.split('&').any(|field| field == "mi_01"))
        })
}

fn physical_parent_name(value: &str) -> bool {
    let parts: Vec<_> = value.split('\\').collect();
    if parts.len() != 3 || parts[0] != "usb" || parts[2].is_empty() {
        return false;
    }
    let fields: Vec<_> = parts[1].split('&').collect();
    fields.len() == 2 && fields.contains(&"vid_320f") && fields.contains(&"pid_22ba")
}

fn usb_parent(i: &HidInterface) -> Option<String> {
    let mut instance = to_wide(&device_instance(i)?);
    let mut node = 0;
    if unsafe { CM_Locate_DevNodeW(&mut node, instance.as_mut_ptr(), 0) } != 0 {
        return None;
    }
    for _ in 0..8 {
        let mut parent = 0;
        if unsafe { CM_Get_Parent(&mut parent, node, 0) } != 0 {
            return None;
        }
        let mut buffer = [0u16; 512];
        if unsafe { CM_Get_Device_IDW(parent, buffer.as_mut_ptr(), buffer.len() as u32, 0) } != 0 {
            return None;
        }
        let length = buffer.iter().position(|c| *c == 0)?;
        let name = String::from_utf16(&buffer[..length])
            .ok()?
            .to_ascii_lowercase();
        if physical_parent_name(&name) {
            return Some(name);
        }
        node = parent;
    }
    None
}

fn report_proof(api: &HidApi, i: &HidInterface) -> bool {
    eligible(i)
        && descriptor::Descriptor::open(api, i).is_some_and(|d| {
            d.only_input_output_report(i, REPORT, LENGTH)
                && d.input_output_byte_reports(i, REPORT, LENGTH, LENGTH)
        })
}

fn select_with<'a>(
    interfaces: &'a [HidInterface],
    mut parent: impl FnMut(&HidInterface) -> Option<String>,
    mut proof: impl FnMut(&HidInterface) -> bool,
) -> Vec<(&'a HidInterface, String)> {
    let mut groups: BTreeMap<String, Vec<&HidInterface>> = BTreeMap::new();
    for i in interfaces.iter().filter(|i| management(i)) {
        // A second management collection cannot disappear merely because its
        // product string, MI or report size fails the model-specific filter.
        let Some(parent) = parent(i).filter(|parent| physical_parent_name(parent)) else {
            return Vec::new();
        };
        groups.entry(parent).or_default().push(i);
    }
    groups
        .into_iter()
        .filter_map(|(parent, members)| {
            let [i] = members.as_slice() else {
                return None;
            };
            (eligible(i) && proof(i)).then_some((*i, parent))
        })
        .collect()
}

fn target(i: &HidInterface, parent: &str) -> Target {
    let mut t = Target::new(BackendId::AulaV9Pro, i, 0);
    t.key = crate::device::stable_key(&format!("aula-v9pro:{parent}"));
    t.name = "AULA V9Pro+".into();
    t.family = DeviceFamily::Mouse;
    t
}

fn request() -> [u8; LENGTH as usize] {
    let mut packet = [0; LENGTH as usize];
    packet[0] = REPORT;
    packet[3] = COMMAND;
    packet
}

fn battery(frame: &[u8]) -> Option<Battery> {
    if frame.len() != usize::from(LENGTH)
        || frame[..4] != [REPORT, 0, 0, COMMAND]
        || frame[8] > 100
        || frame[9] > 1
    {
        return None;
    }
    // Zero while discharging is identical to the request in all documented
    // status fields. Unknown reserved bytes cannot turn an echo into a sample.
    if frame[8] == 0 && frame[9] == 0 {
        return None;
    }
    // Bytes 4..7 are not a transaction identifier. In the captures the device
    // does not echo the parameter byte; no guessed checksum is imposed.
    Some(Battery {
        percent: Some(frame[8]),
        charging: Some(frame[9] == 1),
        estimated_percent: None,
        voltage_mv: None,
        approximate: false,
        level: None,
    })
}

fn read_with(
    t: &Target,
    exchange: impl FnOnce(&[u8]) -> Option<Vec<u8>>,
) -> Result<Snapshot, String> {
    if t.backend != BackendId::AulaV9Pro || !eligible(t.hid()?) {
        return Err(UNAVAILABLE.into());
    }
    let response = exchange(&request()).ok_or(UNAVAILABLE)?;
    let battery = battery(&response).ok_or(UNAVAILABLE)?;
    let mut target = t.clone();
    target.verified_identity = true;
    Ok(Snapshot {
        target,
        battery: Some(battery),
        battery_at: Some(Instant::now()),
        battery_unavailable: false,
        connected: true,
        dpi: Dpi::default(),
        polling_rate: Default::default(),
    })
}

impl DeviceBackend for AulaV9Pro {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(
            select_with(interfaces, usb_parent, |i| report_proof(api, i))
                .into_iter()
                .map(|(i, parent)| target(i, &parent))
                .collect(),
        )
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        if t.backend != BackendId::AulaV9Pro {
            return Err(UNAVAILABLE.into());
        }
        let original = t.hid()?;
        if !eligible(original) {
            return Err(UNAVAILABLE.into());
        }
        let locked_parent = usb_parent(original).ok_or(UNAVAILABLE)?;
        let lock = channel::lock(&locked_parent);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        // Prove topology after acquiring the lock. An unreadable sibling may
        // be another management collection, so it cannot prove uniqueness.
        let inventory = crate::hid::enumerate_all_report(api)?;
        if has_unresolved_collection(&inventory.skipped) {
            return Err(UNAVAILABLE.into());
        }
        let selected = select_with(&inventory.interfaces, usb_parent, |i| report_proof(api, i));
        let (i, parent) = selected
            .into_iter()
            .find(|(i, _)| {
                t.hid()
                    .is_ok_and(|original| original.path.eq_ignore_ascii_case(&i.path))
            })
            .ok_or(UNAVAILABLE)?;
        let current = target(i, &parent);
        if parent != locked_parent || current.key != t.key {
            return Err(UNAVAILABLE.into());
        }
        let mut device = HidDevice::open(i)?;
        read_with(&current, |packet| {
            let started = Instant::now();
            let mut remaining = MAX_REPLIES;
            // HidDevice drains queued input before the one battery query. A
            // silent/invalid reply waits for the next poll, never another path.
            let response = device.transact_with_input_report(packet, REPORT, TIMEOUT_MS, |frame| {
                remaining -= 1;
                battery(frame).is_some() || remaining == 0
            });
            trace::record(
                i,
                BackendId::AulaV9Pro,
                "query-battery",
                COMMAND.into(),
                REPORT,
                packet.len(),
                response.as_ref().map_or(0, Vec::len),
                started,
                match response.as_deref() {
                    Some(frame) if battery(frame).is_some() => trace::Outcome::Ok,
                    Some(_) => trace::Outcome::InvalidData,
                    None => trace::Outcome::NoReply,
                },
            );
            response
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn interface(instance: &str) -> HidInterface {
        HidInterface {
            path: format!(r"\\?\hid#vid_320f&pid_22ba&mi_01&col01#8&{instance}&0&0000#{{hid}}"),
            vendor_id: 0x320f,
            product_id: 0x22ba,
            usage_page: 0xff1c,
            usage: 0x0092,
            input_len: LENGTH,
            output_len: LENGTH,
            feature_len: 0,
            product: PRODUCT.into(),
            serial: "shared".into(),
        }
    }

    fn parent(instance: &str) -> String {
        format!(r"usb\vid_320f&pid_22ba\{instance}")
    }

    fn capture(percent: u8, charging: u8) -> Vec<u8> {
        // The complete zero-padded wire shape is published in the protocol note.
        let mut frame = request().to_vec();
        frame[8] = percent;
        frame[9] = charging;
        frame
    }

    #[test]
    fn published_unplugged_charging_unplugged_sequence_preserves_exact_numbers() {
        let target = target(&interface("one"), &parent("one"));
        for (percent, charging) in [(80, 0), (82, 1), (82, 0)] {
            let sample = read_with(&target, |packet| {
                assert_eq!(packet.len(), 64);
                assert_eq!(&packet[..4], &[4, 0, 0, 0x1a]);
                assert!(packet[4..].iter().all(|byte| *byte == 0));
                Some(capture(percent, charging))
            })
            .unwrap();
            let battery = sample.battery.unwrap();
            assert_eq!(
                (battery.percent, battery.charging),
                (Some(percent), Some(charging == 1))
            );
            assert!(!battery.approximate && battery.level.is_none());
            assert!(
                sample.target.verified_identity && sample.connected && sample.battery_at.is_some()
            );
            assert!(sample.dpi.stages.is_empty() && !sample.dpi.writable);
            assert!(!sample.polling_rate.writable);
        }
    }

    #[test]
    fn exact_product_interface_usage_and_capacities_are_required() {
        let valid = interface("one");
        assert!(eligible(&valid));
        for product in [
            "AULA V9Pro+",
            "AULA-V9Pro",
            "AULA-V9Pro+ ",
            "USB Receiver",
            "",
        ] {
            let mut wrong = valid.clone();
            wrong.product = product.into();
            assert!(!eligible(&wrong));
        }
        for (vid, pid, page, usage, input, output) in [
            (0x320f, 0x706e, 0xff1c, 0x92, 64, 64),
            (0x320f, 0x5088, 0xff1c, 0x92, 64, 64),
            (0x258a, 0x22ba, 0xff1c, 0x92, 64, 64),
            (0x320f, 0x22ba, 0xff00, 0x92, 64, 64),
            (0x320f, 0x22ba, 0xff1c, 1, 64, 64),
            (0x320f, 0x22ba, 0xff1c, 0x92, 63, 64),
            (0x320f, 0x22ba, 0xff1c, 0x92, 64, 65),
        ] {
            let mut wrong = valid.clone();
            wrong.vendor_id = vid;
            wrong.product_id = pid;
            wrong.usage_page = page;
            wrong.usage = usage;
            wrong.input_len = input;
            wrong.output_len = output;
            assert!(!eligible(&wrong));
        }
        for path in [
            valid.path.replace("mi_01", "mi_02"),
            valid.path.replace("mi_01", "mi_010"),
            valid.path.replace("&mi_01", ""),
            valid.path.replace("vid_320f", "vid_320f&vid_373b"),
            valid.path.replace("pid_22ba", "pid_22ba&pid_5088"),
            "invalid".into(),
        ] {
            let mut wrong = valid.clone();
            wrong.path = path;
            assert!(!eligible(&wrong));
        }
        let mut upper = valid;
        upper.path.make_ascii_uppercase();
        assert!(eligible(&upper));
    }

    #[test]
    fn ambiguous_or_unresolved_parent_never_reaches_descriptor_proof() {
        let first = interface("one");
        let mut sibling = first.clone();
        sibling.path = sibling
            .path
            .replace("mi_01", "mi_02")
            .replace("col01", "col02");
        sibling.product = "USB Receiver".into();
        sibling.input_len = 65;
        assert!(select_with(
            &[first.clone(), sibling],
            |_| Some(parent("one")),
            |_| panic!("filtering the sibling must not manufacture uniqueness")
        )
        .is_empty());
        for key in [
            None,
            Some("collection:one".into()),
            Some(r"usb\vid_320f&pid_5088\one".into()),
            Some(r"usb\vid_320f&pid_22ba&mi_01\one".into()),
        ] {
            assert!(select_with(
                &[first.clone()],
                |_| key.clone(),
                |_| panic!("unproven parent cannot open a query")
            )
            .is_empty());
        }
        assert!(select_with(&[first], |_| Some(parent("one")), |_| false).is_empty());
        assert!(has_unresolved_collection(&[crate::hid::Skipped {
            path: interface("hidden").path,
            reason: "descriptor inaccessible".into(),
        }]));
        assert!(!has_unresolved_collection(&[crate::hid::Skipped {
            path: interface("other").path.replace("pid_22ba", "pid_5088"),
            reason: "unavailable".into(),
        }]));
    }

    #[test]
    fn receivers_with_identical_serials_remain_independent() {
        let first = interface("one");
        let second = interface("two");
        let interfaces = [first, second];
        let selected = select_with(
            &interfaces,
            |i| {
                Some(parent(if i.path.contains("&one&") {
                    "one"
                } else {
                    "two"
                }))
            },
            |_| true,
        );
        assert_eq!(selected.len(), 2);
        let a = target(selected[0].0, &selected[0].1);
        let b = target(selected[1].0, &selected[1].1);
        assert_ne!(a.key, b.key);
        assert!(!a.same_channel(&b));
    }

    #[test]
    fn unrelated_reports_bad_states_and_echoes_are_not_battery_samples() {
        let frame = capture(80, 0);
        for length in 0..64 {
            assert!(battery(&frame[..length]).is_none());
        }
        let mut long = frame.clone();
        long.push(0);
        assert!(battery(&long).is_none());
        for (offset, value) in [
            (0, 5),
            (1, 1),
            (2, 1),
            (3, 0x1b),
            (8, 101),
            (9, 2),
            (9, 0xff),
        ] {
            let mut wrong = frame.clone();
            wrong[offset] = value;
            assert!(battery(&wrong).is_none());
        }
        assert!(battery(&request()).is_none());
        let mut echo = request();
        echo[63] = 1;
        assert!(battery(&echo).is_none());
        for percent in 1..=100 {
            for charging in [0, 1] {
                assert_eq!(
                    battery(&capture(percent, charging)).unwrap().percent,
                    Some(percent)
                );
            }
        }
        assert_eq!(battery(&capture(0, 1)).unwrap().percent, Some(0));
        let mut parameter = frame;
        parameter[4] = 0xa5;
        assert_eq!(battery(&parameter).unwrap().percent, Some(80));
    }

    #[test]
    fn query_failure_does_not_refresh_cached_reading_and_success_recovers() {
        let target = target(&interface("one"), &parent("one"));
        let initial = read_with(&target, |_| Some(capture(80, 0))).unwrap();
        let measured = initial.battery_at.unwrap();
        let lifetime = Duration::from_secs(120);
        let mut status = status::Status::default();
        status.observe_sample(
            Some(true),
            initial.battery.as_ref(),
            initial.battery_at,
            measured,
            lifetime,
        );
        for response in [None, Some(request().to_vec()), Some(capture(80, 2))] {
            let mut calls = 0;
            assert!(read_with(&target, |_| {
                calls += 1;
                response
            })
            .is_err());
            assert_eq!(calls, 1);
        }
        status.observe_sample(None, None, None, measured + lifetime, lifetime);
        assert_eq!(status.measured_at, Some(measured));
        assert_eq!(status.quality, crate::device::ReadingQuality::Stale);
        let recovered = read_with(&target, |_| Some(capture(79, 0))).unwrap();
        let later = measured + Duration::from_secs(130);
        status.observe_sample(
            Some(true),
            recovered.battery.as_ref(),
            Some(later),
            later,
            lifetime,
        );
        assert_eq!(status.quality, crate::device::ReadingQuality::Live);
        assert_eq!(status.battery.unwrap().percent, Some(79));
    }

    #[test]
    fn invalid_target_never_calls_transport() {
        let mut invalid = target(&interface("one"), &parent("one"));
        invalid.backend = BackendId::Aula;
        assert!(read_with(&invalid, |_| panic!("wrong backend")).is_err());
        invalid.backend = BackendId::AulaV9Pro;
        invalid.hid_mut().product = "USB Receiver".into();
        assert!(read_with(&invalid, |_| panic!("unproven product")).is_err());
    }
}
