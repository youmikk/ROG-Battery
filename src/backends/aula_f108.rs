//! Original AULA F108 Pro receiver, using its native driver's battery command.
//! This is not the HFD info-page protocol used by F99 MAX or F108 Pro V2.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct AulaF108;

const PRODUCT: &str = "F108Pro Dongle";
const REPORT_BYTES: u16 = 33;
const PAYLOAD_BYTES: usize = 32;
const TIMEOUT_MS: u32 = 600;
const UNAVAILABLE: &str = "设备无有效状态回报";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x05ac && i.product_id == 0x024f
        && i.usage_page == 0xff60 && i.usage == 0x0061
}

fn receiver_group(i: &HidInterface) -> Option<String> {
    if !management(i) { return None; }
    let path = i.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid" || parts[2].is_empty()
        || parts[3].is_empty() || path.contains('\0') { return None; }
    let fields: Vec<_> = parts[1].split('&').collect();
    for (prefix, expected) in [("vid_", "05ac"), ("pid_", "024f"), ("mi_", "03")] {
        let values: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix(prefix)).collect();
        if values.as_slice() != [expected] { return None; }
    }
    let collections: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix("col")).collect();
    if collections.is_empty() { return Some(path); }
    let [collection] = collections.as_slice() else { return None; };
    if collection.len() != 2 || !collection.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
    let (instance, index) = parts[2].rsplit_once('&')?;
    // Windows can use a zero-based suffix while the collection number is
    // one-based. Remove the suffix only after validating the full path shape.
    if instance.split('&').count() < 2 || index.len() != 4 || !index.starts_with("00")
        || !index.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
    let hardware = fields.into_iter().filter(|field| !field.starts_with("col"))
        .collect::<Vec<_>>().join("&");
    Some(format!("{}#{hardware}#{instance}#{}", parts[0], parts[3]))
}

fn control(i: &HidInterface) -> bool {
    management(i) && i.product == PRODUCT && i.input_len == REPORT_BYTES
        && i.output_len == REPORT_BYTES && receiver_group(i).is_some()
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|i| control(i) && interfaces.iter().filter(|other|
        management(other) && receiver_group(i) == receiver_group(other)
    ).count() == 1).collect()
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    control(i) && descriptor::Descriptor::open(api, i)
        .is_some_and(|d| d.only_reports(i, 0, REPORT_BYTES, 0, REPORT_BYTES))
}

fn target(i: &HidInterface) -> Target {
    let mut t = Target::new(BackendId::AulaF108, i, 0);
    t.name = "AULA F108 Pro".into();
    t.family = DeviceFamily::Keyboard;
    t
}

fn request() -> [u8; REPORT_BYTES as usize] {
    let mut r = [0; REPORT_BYTES as usize];
    // Report ID 0, command 0x20, read operation 1, address 0. The native
    // driver stores the low-byte sum at Windows buffer index 32.
    r[1] = 0x20;
    r[2] = 1;
    r[32] = 0x21;
    r
}

fn percent(r: &[u8]) -> Option<u8> {
    if r.len() != PAYLOAD_BYTES || r[..3] != [0x20, 1, 0]
        || !(1..=100).contains(&r[3]) { return None; }
    // The official UI ignores zero and supplies a fixed charging argument;
    // it establishes neither a charging flag nor a response checksum here.
    Some(r[3])
}

fn read_with(t: &Target, exchange: impl FnOnce(&[u8]) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    let i = t.hid()?;
    if t.backend != BackendId::AulaF108 || !control(i) { return Err(UNAVAILABLE.into()); }
    let response = exchange(&request()).ok_or(UNAVAILABLE)?;
    let value = percent(&response).ok_or(UNAVAILABLE)?;
    let mut target = target(i);
    target.verified_identity = true;
    Ok(Snapshot {
        target,
        battery: Some(Battery { percent: Some(value), estimated_percent: None,
            charging: None, voltage_mv: None, approximate: false, level: None }),
        battery_at: Some(Instant::now()),
        battery_unavailable: false,
        connected: true,
        dpi: Dpi::default(),
        polling_rate: Default::default(),
    })
}

impl DeviceBackend for AulaF108 {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i)).map(target).collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        let i = t.hid()?;
        if t.backend != BackendId::AulaF108 || !proven(api, i) { return Err(UNAVAILABLE.into()); }
        let lock = channel::lock(&receiver_group(i).ok_or(UNAVAILABLE)?);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(i)?;
        read_with(t, |request| {
            let started = Instant::now();
            let mut remaining = 32;
            let response = device.transact_with_input_report(request, 0, TIMEOUT_MS, |r| {
                remaining -= 1;
                // A matching zero/invalid value ends the exchange; never wait
                // for an older valid report to substitute for the current one.
                (r.len() >= 3 && r[..3] == [0x20, 1, 0]) || remaining == 0
            });
            trace::record(i, BackendId::AulaF108, "query-battery", 0, 0x20, request.len(),
                response.as_ref().map_or(0, Vec::len), started, match response.as_deref() {
                    Some(r) if percent(r).is_some() => trace::Outcome::Ok,
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
        HidInterface {
            path: format!(r"\\?\hid#vid_05ac&pid_024f&mi_03&col01#8&{instance}&0000#{{hid}}"),
            vendor_id: 0x05ac, product_id: 0x024f, usage_page: 0xff60, usage: 0x0061,
            input_len: REPORT_BYTES, output_len: REPORT_BYTES, feature_len: 0,
            product: PRODUCT.into(), serial: "shared".into(),
        }
    }

    fn response(value: u8) -> Vec<u8> {
        // Synthetic packet derived from the exact official executable's read
        // and display path. No physical F108 Pro capture is claimed.
        let mut r = vec![0; PAYLOAD_BYTES];
        r[..4].copy_from_slice(&[0x20, 1, 0, value]);
        r
    }

    #[test]
    fn native_battery_query_is_one_complete_report_with_the_original_checksum() {
        let t = target(&interface("one"));
        assert!(!t.verified_identity);
        for value in [1, 37, 99, 100] {
            let s = read_with(&t, |r| {
                assert_eq!(r.len(), 33);
                assert_eq!(&r[..4], &[0, 0x20, 1, 0]);
                assert!(r[4..32].iter().all(|b| *b == 0));
                assert_eq!(r[32], r[..32].iter().fold(0u8, |sum, b| sum.wrapping_add(*b)));
                Some(response(value))
            }).unwrap();
            assert!(s.connected && s.target.verified_identity && s.battery_at.is_some());
            assert_eq!(s.target.name, "AULA F108 Pro");
            assert_eq!(s.target.family, DeviceFamily::Keyboard);
            let b = s.battery.unwrap();
            assert_eq!((b.percent, b.charging), (Some(value), None));
            assert!(!b.approximate && !s.dpi.writable && !s.polling_rate.writable);
        }
    }

    #[test]
    fn shared_usb_ids_and_adjacent_aula_models_do_not_authorize_the_query() {
        let good = interface("one");
        assert!(control(&good));
        for product in ["F99 MAX Dongle", "F108Pro", "AULA F108Pro", "F108ProV2 Dongle",
            "AULA F108ProV2C Dongle", "Smart Wireless Device", "F108Pro Dongle ", ""] {
            let mut i = good.clone(); i.product = product.into(); assert!(!control(&i));
        }
        for (vid, pid, page, usage, input, output) in [
            (0x0c45, 0x800a, 0xff60, 0x61, 33, 33), (0x0c45, 0xfefc, 0xff68, 0x61, 33, 33),
            (0x05ac, 0x024f, 0xff59, 0x61, 65, 65), (0x05ac, 0x024f, 1, 6, 33, 33),
            (0x05ac, 0x024f, 0xff60, 0x62, 33, 33), (0x05ac, 0x024f, 0xff60, 0x61, 65, 65),
            (0x05ac, 0x024f, 0xff60, 0x61, 32, 33), (0x05ac, 0x024f, 0xff60, 0x61, 33, 32),
        ] {
            let mut i = good.clone(); i.vendor_id = vid; i.product_id = pid;
            i.usage_page = page; i.usage = usage; i.input_len = input; i.output_len = output;
            assert!(!control(&i));
        }
    }

    #[test]
    fn full_path_identity_is_required_before_opening_a_command_channel() {
        let good = interface("one");
        for path in [good.path.replace("mi_03", "mi_01"), good.path.replace("mi_03", "mi_030"),
            good.path.replace("pid_024f", "pid_024f&pid_800a"),
            good.path.replace("vid_05ac", "vid_05ac&vid_0c45"),
            good.path.replace("mi_03", "mi_03&mi_01"), good.path.replace("col01", "colzz"),
            good.path.replace("col01", "col01&col02"), good.path.replace("8&one&0000", ""),
            good.path.replace("8&one&0000", "one"), good.path.replace("0000#", "abcd#"),
            good.path.rsplit_once('#').map(|(head, _)| format!("{head}#")).unwrap(), format!("{}\0", good.path), String::new()] {
            let mut i = good.clone(); i.path = path; assert!(!control(&i));
        }
        let mut upper = good.clone(); upper.path.make_ascii_uppercase(); assert!(control(&upper));
        let mut single = good; single.path = single.path.replace("&col01", ""); assert!(control(&single));
    }

    #[test]
    fn duplicate_management_collections_are_rejected_without_merging_receivers() {
        let first = interface("one");
        let second = interface("two");
        assert_eq!(candidates(&[first.clone(), second.clone()]).len(), 2);
        assert_ne!(target(&first).key, target(&second).key);
        assert!(!target(&first).same_channel(&target(&second)));
        let mut sibling = first.clone();
        sibling.path = sibling.path.replace("col01", "col02").replace("&0000#", "&0001#");
        sibling.product = "Smart Wireless Device".into();
        assert_eq!(receiver_group(&first), receiver_group(&sibling));
        assert!(candidates(&[first, sibling]).is_empty());
    }

    #[test]
    fn strict_full_response_rejects_sleep_unavailable_and_other_protocols() {
        for value in [0, 101, 127, 128, 255] { assert!(percent(&response(value)).is_none()); }
        for length in 0..PAYLOAD_BYTES { assert!(percent(&response(37)[..length]).is_none()); }
        let mut padded = response(37); padded.push(0); assert!(percent(&padded).is_none());
        let mut with_id = vec![0]; with_id.extend(response(37)); assert!(percent(&with_id).is_none());
        for (index, value) in [(0, 0x55), (0, 0xaa), (0, 0x21), (1, 0), (1, 2), (2, 1)] {
            let mut r = response(37); r[index] = value; assert!(percent(&r).is_none());
        }
        let mut unrelated_tail = response(37); unrelated_tail[4..].fill(0x80);
        assert_eq!(percent(&unrelated_tail), Some(37));
    }

    #[test]
    fn invalid_target_or_missing_reply_never_creates_a_battery_sample() {
        let t = target(&interface("one"));
        assert!(read_with(&t, |_| None).is_err());
        assert!(read_with(&t, |_| Some(response(0))).is_err());
        assert!(read_with(&t, |_| Some(response(101))).is_err());
        let mut wrong = t.clone(); wrong.backend = BackendId::AulaF99;
        assert!(read_with(&wrong, |_| panic!("foreign backend queried")).is_err());
        wrong = t; wrong.hid_mut().product = "F99 MAX Dongle".into();
        assert!(read_with(&wrong, |_| panic!("foreign product queried")).is_err());
    }
}
