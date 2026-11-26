//! MCHOSE V9 Pro 2 receiver battery over the MC report protocol.
//! The battery GET and battery notification layouts come from M HUB's MC codec.
//! This is separate from the older FF90 V9 protocol and sends no settings.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct MchoseMc;

const PRODUCT: &str = "MCHOSE V9 Pro 2";
const REPORT: u8 = 0x55;
const LENGTH: u16 = 64;
const TIMEOUT_MS: u32 = 1_000;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x3837 && i.product_id == 0x6011
        && i.usage_page == 0xff07 && i.usage == 0x0222
}

fn receiver_group(i: &HidInterface) -> Option<String> {
    if !management(i) { return None; }
    // The documented battery collection is on MI_03. Keep that interface and
    // the full Windows instance; shared serial strings cannot merge receivers.
    let path = i.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid" || parts[2].is_empty()
        || parts[3].is_empty() || path.contains('\0') { return None; }
    let fields: Vec<_> = parts[1].split('&').collect();
    for required in ["vid_3837", "pid_6011", "mi_03"] {
        if fields.iter().filter(|field| **field == required).count() != 1 { return None; }
    }
    if fields.iter().filter(|field| field.starts_with("mi_")).count() != 1 { return None; }
    let collections: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix("col")).collect();
    let instance = match collections.as_slice() {
        [] => parts[2],
        [collection] => {
            if collection.len() != 2 || !collection.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
            let (instance, index) = parts[2].rsplit_once('&')?;
            // Windows collection indices may be zero- or one-based relative
            // to ColNN. Only normalize this documented device/interface.
            if instance.split('&').count() < 2 || index.len() != 4
                || !index.starts_with("00") || !index.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            instance
        }
        _ => return None,
    };
    let hardware = fields.into_iter().filter(|field| !field.starts_with("col"))
        .collect::<Vec<_>>().join("&");
    Some(format!("{}#{hardware}#{instance}#{}", parts[0], parts[3]))
}

fn control(i: &HidInterface) -> bool {
    management(i) && i.product.trim().eq_ignore_ascii_case(PRODUCT)
        && i.input_len == LENGTH && i.output_len == LENGTH && receiver_group(i).is_some()
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|i| control(i) && interfaces.iter().filter(|other|
        management(other) && receiver_group(i) == receiver_group(other)
    ).count() == 1).collect()
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    control(i) && descriptor::Descriptor::open(api, i)
        .is_some_and(|d| d.input_output_byte_reports(i, REPORT, LENGTH, LENGTH))
}

fn target(i: &HidInterface) -> Target {
    let mut t = Target::new(BackendId::MchoseMc, i, 0);
    t.name = PRODUCT.into();
    t.family = DeviceFamily::Headset;
    t
}

fn request() -> [u8; LENGTH as usize] {
    let mut r = [0; LENGTH as usize];
    // Report 55; MC start/version; no checksum; length 1; GET type 0001;
    // subcommand 0B; the unchecked frame's checksum byte is still zero.
    r[..9].copy_from_slice(&[REPORT, 0xaa, 1, 0, 1, 0, 1, 0x0b, 0]);
    r
}

#[derive(Debug, PartialEq, Eq)]
enum Reply { Battery { percent: u8, charging: bool }, Sleeping }

fn reply(r: &[u8]) -> Option<Reply> {
    if r.len() != usize::from(LENGTH) || r[..3] != [REPORT, 0xaa, 1]
        || r[3] > 1 || r[4] < 3 { return None; }
    // The count includes the subcommand and body, not the six-byte MC header.
    let checksum = 7 + usize::from(r[4]);
    if checksum >= r.len() || !matches!(&r[5..8], [0, 1, 0x0b] | [0, 4, 0x22]) {
        return None;
    }
    let expected = if r[3] == 1 { r[3..checksum].iter().fold(0, |xor, b| xor ^ b) } else { 0 };
    if r[checksum] != expected { return None; }
    if r[8] == 0 { return Some(Reply::Sleeping); }
    let percent = r[9] & 0x7f;
    (percent <= 100).then_some(Reply::Battery { percent, charging: r[9] & 0x80 != 0 })
}

fn read_with(t: &Target, exchange: impl FnOnce(&[u8]) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    let i = t.hid()?;
    if t.backend != BackendId::MchoseMc || !control(i) { return Err(UNAVAILABLE.into()); }
    let r = exchange(&request()).ok_or(UNAVAILABLE)?;
    let state = reply(&r).ok_or(UNAVAILABLE)?;
    let battery = match state {
        Reply::Battery { percent, charging } => Some(Battery {
            percent: Some(percent), charging: Some(charging), estimated_percent: None,
            voltage_mv: None, approximate: false, level: None,
        }),
        Reply::Sleeping => None,
    };
    let mut target = target(i);
    target.verified_identity = true;
    Ok(Snapshot { target, battery_at: battery.as_ref().map(|_| Instant::now()), battery,
        battery_unavailable: state == Reply::Sleeping, connected: state != Reply::Sleeping,
        dpi: Dpi::default(), polling_rate: Default::default() })
}

impl DeviceBackend for MchoseMc {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i)).map(target).collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        let i = t.hid()?;
        if t.backend != BackendId::MchoseMc || !proven(api, i) { return Err(UNAVAILABLE.into()); }
        let lock = channel::lock(&receiver_group(i).ok_or(UNAVAILABLE)?);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(i)?;
        read_with(t, |request| {
            let started = Instant::now();
            let mut remaining = 32;
            let response = device.transact(request, TIMEOUT_MS, |r| {
                remaining -= 1;
                reply(r).is_some() || remaining == 0
            });
            trace::record(i, BackendId::MchoseMc, "query-battery", 0x0b, REPORT,
                request.len(), response.as_ref().map_or(0, Vec::len), started,
                match response.as_deref() {
                    Some(r) if reply(r).is_some() => trace::Outcome::Ok,
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
        HidInterface { path: format!(r"\\?\hid#vid_3837&pid_6011&mi_03&col04#8&{instance}&0&0003#{{hid}}"),
            vendor_id: 0x3837, product_id: 0x6011, usage_page: 0xff07, usage: 0x0222,
            input_len: LENGTH, output_len: LENGTH, feature_len: 0,
            product: PRODUCT.into(), serial: "shared".into() }
    }

    fn frame(percent: u8, charging: bool, awake: bool, checked: bool, notify: bool) -> Vec<u8> {
        // Synthetic fixtures derived from the official MC frame layout.
        let mut r = vec![0; usize::from(LENGTH)];
        r[..10].copy_from_slice(&[REPORT, 0xaa, 1, u8::from(checked), 3, 0,
            if notify { 4 } else { 1 }, if notify { 0x22 } else { 0x0b },
            u8::from(awake), percent | if charging { 0x80 } else { 0 }]);
        if checked { r[10] = r[3..10].iter().fold(0, |xor, b| xor ^ b); }
        r
    }

    #[test]
    fn mc_battery_query_is_distinct_from_old_v9_and_contains_no_configuration_body() {
        assert_eq!(&request()[..9], &[0x55, 0xaa, 1, 0, 1, 0, 1, 0x0b, 0]);
        assert!(request()[9..].iter().all(|b| *b == 0));
        let t = target(&interface("one"));
        let mut sent = 0;
        let snapshot = read_with(&t, |p| {
            sent += 1;
            assert_eq!(p, request());
            Some(frame(72, true, true, false, false))
        }).unwrap();
        assert_eq!(sent, 1);
        assert!(snapshot.connected && snapshot.battery_at.is_some());
        assert_eq!((snapshot.battery.as_ref().unwrap().percent, snapshot.battery.unwrap().charging),
            (Some(72), Some(true)));
        assert!(!snapshot.dpi.writable && snapshot.dpi.stages.is_empty());
    }

    #[test]
    fn checked_and_unchecked_replies_and_notifications_keep_precise_percent_and_charge() {
        for checked in [false, true] {
            for notify in [false, true] {
                for charging in [false, true] {
                    for percent in 0..=100 {
                        assert_eq!(reply(&frame(percent, charging, true, checked, notify)),
                            Some(Reply::Battery { percent, charging }));
                    }
                    for percent in 101..=127 {
                        assert!(reply(&frame(percent, charging, true, checked, notify)).is_none());
                    }
                }
            }
        }
        let checked = frame(40, false, true, true, false);
        assert_eq!(&checked[..11], &[0x55, 0xaa, 1, 1, 3, 0, 1, 0x0b, 1, 40, 0x21]);
    }

    #[test]
    fn sleep_discards_cached_payload_and_wake_resumes_without_manual_refresh() {
        let t = target(&interface("one"));
        let sleeping = read_with(&t, |_| Some(frame(127, true, false, true, false))).unwrap();
        assert!(!sleeping.connected && sleeping.battery_unavailable);
        assert!(sleeping.battery.is_none() && sleeping.battery_at.is_none());
        assert!(read_with(&t, |_| None).is_err());
        let awake = read_with(&t, |_| Some(frame(39, false, true, true, true))).unwrap();
        assert!(awake.connected && !awake.battery_unavailable && awake.battery_at.is_some());
        assert_eq!(awake.battery.unwrap().percent, Some(39));
    }

    #[test]
    fn echoes_other_commands_bad_checksums_and_truncated_frames_are_rejected() {
        assert!(reply(&request()).is_none());
        let valid = frame(40, false, true, true, false);
        for length in 0..64 { assert!(reply(&valid[..length]).is_none()); }
        let mut long = valid.clone(); long.push(0);
        assert!(reply(&long).is_none());
        for (offset, value) in [(0, 0x54), (1, 0xab), (2, 2), (3, 2), (4, 2),
            (4, 57), (5, 1), (6, 2), (6, 4), (7, 0x22), (7, 0x0c), (10, 0)] {
            let mut wrong = valid.clone(); wrong[offset] = value;
            assert!(reply(&wrong).is_none());
        }
        let mut unchecked = frame(40, false, true, false, false);
        unchecked[10] = 1;
        assert!(reply(&unchecked).is_none());
        let mut set_reply = valid.clone();
        set_reply[6] = 2;
        set_reply[10] = set_reply[3..10].iter().fold(0, |xor, b| xor ^ b);
        assert!(reply(&set_reply).is_none());
    }

    #[test]
    fn exact_receiver_identity_interface_and_report_capacities_are_required() {
        let i = interface("one");
        assert!(control(&i));
        let mut upper = i.clone(); upper.product = "MCHOSE V9 PRO 2".into();
        assert!(control(&upper));
        for (vid, pid, name, page, usage, input, output) in [
            (0x3837, 0x6013, "MCHOSE R9", 0xff07, 0x0222, 64, 64),
            (0x3837, 0x600a, "MCHOSE V9 Turbo+", 0xff90, 1, 64, 64),
            (0x291d, 0x385d, "MCHOSE V9 PRO", 0xff90, 1, 64, 64),
            (0x3837, 0x6011, "MCHOSE V9 PRO", 0xff07, 0x0222, 64, 64),
            (0x3837, 0x6011, PRODUCT, 0xff07, 1, 64, 64),
            (0x3837, 0x6011, PRODUCT, 0xff90, 0x0222, 64, 64),
            (0x3837, 0x6011, PRODUCT, 0xff07, 0x0222, 63, 64),
            (0x3837, 0x6011, PRODUCT, 0xff07, 0x0222, 64, 65),
        ] {
            let mut wrong = i.clone();
            wrong.vendor_id = vid; wrong.product_id = pid; wrong.product = name.into();
            wrong.usage_page = page; wrong.usage = usage; wrong.input_len = input; wrong.output_len = output;
            assert!(!control(&wrong));
            assert!(read_with(&target(&wrong), |_| panic!("ineligible collection must not send")).is_err());
        }
        for path in ["unstructured", r"\\?\hid#vid_3837&pid_6011&mi_00#instance#{hid}",
            r"\\?\hid#vid_3837&pid_6011&mi_03&mi_04#instance#{hid}",
            r"\\?\hid#vid_3837&pid_6013&mi_03#instance#{hid}"] {
            let mut wrong = i.clone(); wrong.path = path.into(); assert!(!control(&wrong));
        }
        let mut wrong_backend = target(&i); wrong_backend.backend = BackendId::Headset;
        assert!(read_with(&wrong_backend, |_| panic!("wrong protocol must not send")).is_err());
    }

    #[test]
    fn sibling_collections_are_ambiguous_and_two_receivers_keep_separate_keys() {
        let one = interface("one");
        let two = interface("two");
        assert_eq!(candidates(&[one.clone(), two.clone()]).len(), 2);
        assert_ne!(target(&one).key, target(&two).key);
        for suffix in ["0004", "0005"] {
            let mut sibling = one.clone();
            sibling.path = sibling.path.replace("col04", "col05").replace("0003", suffix);
            // An unknown sibling with the same vendor usage still makes the
            // collection ambiguous, even if its lengths/name fail control.
            sibling.input_len = 63; sibling.product = "USB Receiver".into();
            assert!(candidates(&[one.clone(), sibling]).is_empty());
        }
    }
}
