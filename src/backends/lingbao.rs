//! Lingbao M1 Pro 088D:052F receiver: one read-only Feature 0F query.
//! Receiver replies do not prove that the paired mouse is awake or RF-online.
use super::*;
use crate::win32::to_wide;
use std::{collections::BTreeMap, time::Instant};
use winapi::um::cfgmgr32::{CM_Get_Device_IDW, CM_Get_Parent, CM_Locate_DevNodeW};

pub struct Lingbao;
const REPORT_ID: u8 = 0x0f;
const REPORT_LENGTH: u16 = 33;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x088d
        && i.product_id == 0x052f
        && i.usage_page == 0xff00
        && i.usage == 1
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
    for required in ["vid_088d", "pid_052f"] {
        if fields.iter().filter(|field| **field == required).count() != 1 {
            return None;
        }
    }
    if fields.iter().filter(|field| field.starts_with("vid_")).count() != 1
        || fields.iter().filter(|field| field.starts_with("pid_")).count() != 1
    {
        return None;
    }
    for prefix in ["mi_", "col"] {
        let indices: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix(prefix)).collect();
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
    i.product == "Wireless-Receiver"
        && i.feature_len == REPORT_LENGTH
        && device_instance(i).is_some_and(|instance| {
            instance.split('\\').nth(1)
                .is_some_and(|fields| fields.split('&').any(|field| field == "mi_01"))
        })
}

fn physical_parent_name(name: &str) -> bool {
    let parts: Vec<_> = name.split('\\').collect();
    if parts.len() != 3 || parts[0] != "usb" || parts[2].is_empty() {
        return false;
    }
    let fields: Vec<_> = parts[1].split('&').collect();
    fields.len() == 2 && fields.contains(&"vid_088d") && fields.contains(&"pid_052f")
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
        let name = String::from_utf16(&buffer[..length]).ok()?.to_ascii_lowercase();
        if physical_parent_name(&name) {
            return Some(name);
        }
        node = parent;
    }
    None
}

fn report_proof(api: &HidApi, i: &HidInterface) -> bool {
    if !eligible(i) {
        return false;
    }
    descriptor::Descriptor::open(api, i).is_some_and(|d| {
        d.feature_reports(i, REPORT_LENGTH, &[REPORT_ID], &[REPORT_ID])
            && d.feature_byte_report(i, REPORT_ID, REPORT_LENGTH)
    })
}

fn select_with<'a>(
    interfaces: &'a [HidInterface],
    mut parent: impl FnMut(&HidInterface) -> Option<String>,
    mut proof: impl FnMut(&HidInterface) -> bool,
) -> Vec<(&'a HidInterface, String)> {
    let mut groups: BTreeMap<String, Vec<&HidInterface>> = BTreeMap::new();
    for i in interfaces.iter().filter(|i| management(i)) {
        // Count all matching vendor collections before checking name, MI, or
        // report shape. A malformed sibling must not manufacture uniqueness.
        let Some(key) = parent(i).filter(|key| physical_parent_name(key)) else {
            return Vec::new();
        };
        groups.entry(key).or_default().push(i);
    }
    groups.into_iter().filter_map(|(parent, members)| {
        let [i] = members.as_slice() else { return None; };
        (eligible(i) && proof(i)).then_some((*i, parent))
    }).collect()
}

fn target(i: &HidInterface, parent: &str) -> Target {
    let mut t = Target::new(BackendId::Lingbao, i, 0);
    t.key = crate::device::stable_key(&format!("lingbao:{parent}"));
    t.name = "Lingbao M1 Pro".into();
    t.family = DeviceFamily::Mouse;
    t
}

#[derive(Debug, PartialEq, Eq)]
enum Reply {
    Percent(u8),
    ChargingPlaceholder,
}

fn parse(r: &[u8]) -> Option<Reply> {
    if r.len() != usize::from(REPORT_LENGTH)
        || r[0] != REPORT_ID
        || &r[7..11] != b"M536"
        || &r[13..15] != b"LX"
    {
        return None;
    }
    match r[19] {
        0 if (1..=100).contains(&r[18]) => Some(Reply::Percent(r[18])),
        // Published charging observations report a fixed 100 here. Do not
        // publish it as a measurement or extend a previous sample's lifetime.
        1 => Some(Reply::ChargingPlaceholder),
        _ => None,
    }
}

fn read_with(
    t: &Target,
    get_feature: impl FnOnce(u8, u16) -> Option<Vec<u8>>,
) -> Result<Snapshot, String> {
    let i = t.hid().map_err(|_| UNAVAILABLE)?;
    if t.backend != BackendId::Lingbao || !eligible(i) {
        return Err(UNAVAILABLE.into());
    }
    // The callback exposes only a GET operation; no request payload or write
    // handle is needed. Failure and placeholders do not cause an extra query.
    let response = get_feature(REPORT_ID, REPORT_LENGTH).ok_or(UNAVAILABLE)?;
    let Some(Reply::Percent(percent)) = parse(&response) else {
        return Err(UNAVAILABLE.into());
    };
    let mut target = t.clone();
    target.verified_identity = true;
    Ok(Snapshot {
        target,
        battery: Some(Battery {
            percent: Some(percent),
            charging: None,
            estimated_percent: None,
            voltage_mv: None,
            approximate: false,
            level: None,
        }),
        battery_at: Some(Instant::now()),
        battery_unavailable: false,
        // The current snapshot schema has no unknown connection state. This
        // means an accepted receiver reply, not independent mouse RF evidence.
        connected: true,
        dpi: Dpi::default(),
        polling_rate: Default::default(),
    })
}

impl DeviceBackend for Lingbao {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(select_with(interfaces, usb_parent, |i| report_proof(api, i)).into_iter()
            .map(|(i, parent)| target(i, &parent)).collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        if t.backend != BackendId::Lingbao {
            return Err(UNAVAILABLE.into());
        }
        // Re-enumerate and prove the actual USB parent and report on every read.
        let interfaces = crate::hid::enumerate_all_report(api)?.interfaces;
        let candidates = select_with(&interfaces, usb_parent, |i| report_proof(api, i));
        let (i, parent) = candidates.into_iter().find(|(i, _)| {
            t.hid().is_ok_and(|original| i.path.eq_ignore_ascii_case(&original.path))
        }).ok_or(UNAVAILABLE)?;
        let current = target(i, &parent);
        if current.key != t.key {
            return Err(UNAVAILABLE.into());
        }
        let lock = channel::lock(&parent);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        read_with(&current, |id, _length| {
            let started = Instant::now();
            let response = crate::hid::query_report(i, id, true, 1_000);
            let outcome = match response.as_deref() {
                Some(r) => match parse(r) {
                    Some(Reply::Percent(_)) => trace::Outcome::Ok,
                    Some(Reply::ChargingPlaceholder) => trace::Outcome::Rejected(1),
                    None => trace::Outcome::InvalidData,
                },
                None => trace::Outcome::NoReply,
            };
            trace::record(i, BackendId::Lingbao, "get-battery", u16::from(id), 0,
                0, response.as_ref().map_or(0, Vec::len), started, outcome);
            response
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The two complete 33-byte public captures in MyFinder issue #171 both
    // report 65%, flag 0; only byte 4 differs. Other test frames are synthetic.
    const CAPTURE_A: [u8; 33] = [
        0x0f, 0x00, 0x30, 0x02, 0xec, 0x01, 0x10, 0x4d, 0x35, 0x33, 0x36,
        0x01, 0x06, 0x4c, 0x58, 0x11, 0x11, 0x01, 0x41, 0x00, 0xff, 0x00,
        0xff, 0xd0, 0x00, 0x0c, 0x31, 0xbe, 0x00, 0x5b, 0x81, 0x2c, 0xc8,
    ];
    const CAPTURE_B: [u8; 33] = [
        0x0f, 0x00, 0x30, 0x02, 0xeb, 0x01, 0x10, 0x4d, 0x35, 0x33, 0x36,
        0x01, 0x06, 0x4c, 0x58, 0x11, 0x11, 0x01, 0x41, 0x00, 0xff, 0x00,
        0xff, 0xd0, 0x00, 0x0c, 0x31, 0xbe, 0x00, 0x5b, 0x81, 0x2c, 0xc8,
    ];

    fn interface(instance: &str) -> HidInterface {
        HidInterface {
            path: format!(r"\\?\hid#vid_088d&pid_052f&mi_01&col01#8&{instance}&0&0000#{{hid}}"),
            vendor_id: 0x088d, product_id: 0x052f, usage_page: 0xff00, usage: 1,
            input_len: 0, output_len: 0, feature_len: 33,
            product: "Wireless-Receiver".into(), serial: "shared".into(),
        }
    }

    fn parent(instance: &str) -> String {
        format!(r"usb\vid_088d&pid_052f\{instance}")
    }

    #[test]
    fn public_captures_provide_percent_without_unproven_charging_or_voltage() {
        let t = target(&interface("one"), &parent("one"));
        for capture in [CAPTURE_A, CAPTURE_B] {
            assert_eq!(parse(&capture), Some(Reply::Percent(65)));
            let sample = read_with(&t, |id, length| {
                assert_eq!((id, length), (0x0f, 33));
                Some(capture.to_vec())
            }).unwrap();
            let b = sample.battery.unwrap();
            assert_eq!((b.percent, b.charging, b.voltage_mv), (Some(65), None, None));
            assert!(!b.approximate);
            assert!(sample.target.verified_identity && sample.battery_at.is_some());
            assert!(!sample.battery_unavailable);
            assert!(sample.dpi.stages.is_empty() && !sample.dpi.writable);
            assert!(!sample.polling_rate.writable);
        }
    }

    #[test]
    fn complete_length_report_id_fingerprints_and_valid_percentage_are_required() {
        for length in 0..33 {
            assert!(parse(&CAPTURE_A[..length]).is_none());
        }
        let mut long = CAPTURE_A.to_vec();
        long.push(0);
        assert!(parse(&long).is_none());
        for offset in [0, 7, 8, 9, 10, 13, 14] {
            let mut synthetic = CAPTURE_A;
            synthetic[offset] ^= 1;
            assert!(parse(&synthetic).is_none());
        }
        for percent in 0..=255 {
            let mut synthetic = CAPTURE_A;
            synthetic[18] = percent;
            assert_eq!(parse(&synthetic), (1..=100).contains(&percent).then_some(Reply::Percent(percent)));
        }
        for flag in 2..=255 {
            let mut synthetic = CAPTURE_A;
            synthetic[19] = flag;
            assert!(parse(&synthetic).is_none());
        }
    }

    #[test]
    fn charging_placeholders_never_create_a_new_sample_or_retry() {
        let t = target(&interface("one"), &parent("one"));
        for percent in 0..=255 {
            let mut synthetic = CAPTURE_A;
            synthetic[18] = percent;
            synthetic[19] = 1;
            let mut gets = 0;
            assert_eq!(parse(&synthetic), Some(Reply::ChargingPlaceholder));
            assert_eq!(read_with(&t, |id, length| {
                gets += 1;
                assert_eq!((id, length), (0x0f, 33));
                Some(synthetic.to_vec())
            }).unwrap_err(), UNAVAILABLE);
            assert_eq!(gets, 1);
        }
        for response in [None, Some(Vec::new()), Some(vec![0; 33])] {
            let mut gets = 0;
            assert!(read_with(&t, |_, _| { gets += 1; response }).is_err());
            assert_eq!(gets, 1);
        }
        assert_eq!(read_with(&t, |_, _| Some(CAPTURE_B.to_vec())).unwrap()
            .battery.unwrap().percent, Some(65));
    }

    #[test]
    fn same_parent_ambiguity_is_checked_before_product_mi_or_report_filtering() {
        let one = interface("one");
        let two = interface("two");
        let pick_parent = |i: &HidInterface| {
            Some(parent(if i.path.contains("&two&") { "two" } else { "one" }))
        };
        assert_eq!(select_with(&[one.clone(), two.clone()], pick_parent, |_| true).len(), 2);
        assert_ne!(target(&one, &parent("one")).key, target(&two, &parent("two")).key);
        for variant in 0..3 {
            let mut sibling = one.clone();
            sibling.path = sibling.path.replace("col01", "col02");
            match variant {
                0 => sibling.product = "Other receiver".into(),
                1 => sibling.path = sibling.path.replace("mi_01", "mi_02"),
                _ => sibling.feature_len = 64,
            }
            assert!(select_with(&[one.clone(), sibling], pick_parent,
                |_| panic!("ambiguous parent must not reach report proof")).is_empty());
        }
        assert!(select_with(&[one.clone()], |_| None, |_| panic!("unknown topology")).is_empty());
        assert!(select_with(&[one.clone()], |_| Some("collection:one".into()),
            |_| panic!("synthetic topology")).is_empty());
        assert!(select_with(&[one.clone()], pick_parent, |_| false).is_empty());
        assert_eq!(select_with(&[one], pick_parent, |_| true).len(), 1);
        assert!(!physical_parent_name(r"usb\vid_088d&pid_052f&mi_01\one"));
        assert!(!physical_parent_name(r"usb\vid_088d&pid_0530\one"));
    }

    #[test]
    fn other_models_names_interfaces_and_unproven_reports_never_query() {
        let original = interface("one");
        assert!(eligible(&original));
        for variant in 0..9 {
            let mut wrong = original.clone();
            match variant {
                0 => wrong.vendor_id = 0x088e,
                1 => wrong.product_id = 0x0530,
                2 => wrong.usage_page = 1,
                3 => wrong.usage = 2,
                4 => wrong.product = "wireless-receiver".into(),
                5 => wrong.product.push(' '),
                6 => wrong.path = wrong.path.replace("mi_01", "mi_02"),
                7 => wrong.path = wrong.path.replace("&mi_01", ""),
                _ => wrong.feature_len = 64,
            }
            assert!(!eligible(&wrong));
            assert!(read_with(&target(&wrong, &parent("one")),
                |_, _| panic!("ineligible receiver must not query")).is_err());
        }
        for path in ["unstructured", r"\\?\hid#vid_088d&pid_052f&mi_01&mi_02#one#{hid}",
            r"\\?\hid#vid_088d&pid_052f&pid_0530&mi_01#one#{hid}"] {
            let mut wrong = original.clone();
            wrong.path = path.into();
            assert!(device_instance(&wrong).is_none());
        }
        let mut wrong = target(&original, &parent("one"));
        wrong.backend = BackendId::CompX;
        assert!(read_with(&wrong, |_, _| panic!("wrong backend must not query")).is_err());
    }
}
