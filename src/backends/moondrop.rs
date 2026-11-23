//! GM01 Pro receiver battery, through its descriptor-proven Report 4B collection.
//! The read command has no charging field or device-setting side effects.
use super::*;
use crate::hid::HidDevice;
use std::{collections::BTreeMap, time::Instant};

pub struct Moondrop;
const REPORT: u8 = 0x4b;
const MAX_LENGTH: u16 = 4096;
const TIMEOUT_MS: u32 = 800;
const UNAVAILABLE: &str = "设备无有效状态回报";

fn priority(i: &HidInterface) -> Option<u8> {
    match (i.usage_page, i.usage) {
        (0xff02, 2) => Some(0),
        (0x000c, 1) => Some(1),
        _ => None,
    }
}

fn receiver_group(i: &HidInterface) -> Option<String> {
    if i.vendor_id != 0x35d8 || i.product_id != 0x0133 || priority(i).is_none() {
        return None;
    }
    // The hardware record places all four receiver collections on MI_03.
    // Restrict grouping to that interface until another layout has evidence.
    let path = i.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid" || parts[2].is_empty()
        || parts[3].is_empty() || path.contains('\0') { return None; }
    let fields: Vec<_> = parts[1].split('&').collect();
    for required in ["vid_35d8", "pid_0133", "mi_03"] {
        if fields.iter().filter(|field| **field == required).count() != 1 { return None; }
    }
    if fields.iter().filter(|field| field.starts_with("mi_")).count() != 1 { return None; }
    let collections: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix("col")).collect();
    let [collection] = collections.as_slice() else { return None; };
    if collection.len() != 2 || !collection.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
    // HID's trailing collection index may be zero- or one-based relative to ColNN.
    // Preserve the preceding instance and MI_03, rather than requiring equality.
    let (instance, index) = parts[2].rsplit_once('&')?;
    if instance.split('&').count() < 2 || index.len() != 4
        || !index.starts_with("00") || !index.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
    let hardware = fields.into_iter().filter(|field| !field.starts_with("col")).collect::<Vec<_>>().join("&");
    Some(format!("{}#{hardware}#{instance}#{}", parts[0], parts[3]))
}

fn capacities(i: &HidInterface) -> bool {
    (10..=MAX_LENGTH).contains(&i.input_len) && (4..=MAX_LENGTH).contains(&i.output_len)
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    receiver_group(i).is_some() && capacities(i)
        && descriptor::Descriptor::open(api, i).is_some_and(|d|
            d.input_output_byte_reports(i, REPORT, i.input_len, i.output_len))
}

fn select_with<'a>(interfaces: &'a [HidInterface], mut proof: impl FnMut(&HidInterface) -> bool)
    -> Vec<&'a HidInterface> {
    let mut groups: BTreeMap<String, Vec<&HidInterface>> = BTreeMap::new();
    for i in interfaces {
        if let Some(group) = receiver_group(i) { groups.entry(group).or_default().push(i); }
    }
    let mut selected = Vec::new();
    for members in groups.values() {
        // Duplicate raw vendor collections are ambiguous even if one fails proof.
        if members.iter().filter(|i| priority(i) == Some(0)).count() > 1 { continue; }
        for rank in [0, 1] {
            let eligible: Vec<_> = members.iter().copied().filter(|i|
                priority(i) == Some(rank) && capacities(i) && proof(i)).collect();
            if eligible.len() == 1 { selected.push(eligible[0]); break; }
            if eligible.len() > 1 { break; }
        }
    }
    selected
}

fn target(i: &HidInterface) -> Target {
    let mut t = Target::new(BackendId::Moondrop, i, 0);
    // Same receiver stays one device when its selected collection changes.
    let group = receiver_group(i).unwrap_or_else(|| i.path.to_ascii_lowercase());
    t.key = crate::device::stable_key(&format!("moondrop:{group}"));
    t.name = "MOONDROP GM01 Pro".into();
    t.family = DeviceFamily::Headset;
    t
}

fn request(length: u16) -> Option<Vec<u8>> {
    if !(4..=MAX_LENGTH).contains(&length) { return None; }
    let mut r = vec![0; usize::from(length)];
    r[..4].copy_from_slice(&[REPORT, 0x80, 0x27, 0]);
    Some(r)
}

fn battery_frame(r: &[u8], input_len: u16) -> bool {
    r.len() == usize::from(input_len) && (10..=MAX_LENGTH).contains(&input_len)
        && r[0] == REPORT && r[2] == 0x27
        && usize::from(r[3]) >= 6 && usize::from(r[3]) <= r.len() - 4
}

fn battery(r: &[u8], input_len: u16) -> Option<Battery> {
    if !battery_frame(r, input_len) || r[8] == 0 || r[9] > 100 { return None; }
    Some(Battery { percent: Some(r[9]), charging: None, estimated_percent: None,
        voltage_mv: None, approximate: false, level: None })
}

fn read_with(t: &Target, exchange: impl FnOnce(&[u8]) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    let i = t.hid()?;
    if t.backend != BackendId::Moondrop || receiver_group(i).is_none() || !capacities(i) {
        return Err(UNAVAILABLE.into());
    }
    let r = exchange(&request(i.output_len).ok_or(UNAVAILABLE)?).ok_or(UNAVAILABLE)?;
    let battery = battery(&r, i.input_len).ok_or(UNAVAILABLE)?;
    let mut target = target(i);
    target.verified_identity = true;
    Ok(Snapshot { target, battery: Some(battery), battery_at: Some(Instant::now()),
        battery_unavailable: false, connected: true, dpi: Dpi::default(), polling_rate: Default::default() })
}

impl DeviceBackend for Moondrop {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(select_with(interfaces, |i| proven(api, i)).into_iter().map(target).collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        let i = t.hid()?;
        if t.backend != BackendId::Moondrop || !proven(api, i) { return Err(UNAVAILABLE.into()); }
        let lock = channel::lock(&receiver_group(i).ok_or(UNAVAILABLE)?);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(i)?;
        read_with(t, |request| {
            let started = Instant::now();
            let mut remaining = 32;
            let response = device.transact(request, TIMEOUT_MS, |r| {
                remaining -= 1;
                // A correlated validity-zero reply is unknown, not a fresh 0%.
                battery_frame(r, i.input_len) || remaining == 0
            });
            trace::record(i, BackendId::Moondrop, "query-battery", 0x27, REPORT,
                request.len(), response.as_ref().map_or(0, Vec::len), started, match response.as_deref() {
                    Some(r) if battery(r, i.input_len).is_some() => trace::Outcome::Ok,
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

    fn interface(instance: &str, collection: u8, page: u16, usage: u16) -> HidInterface {
        HidInterface { path: format!(r"\\?\hid#vid_35d8&pid_0133&mi_03&col{collection:02x}#8&{instance}&00{collection:02x}#{{hid}}"),
            vendor_id: 0x35d8, product_id: 0x0133, usage_page: page, usage,
            input_len: 64, output_len: 64, feature_len: 0,
            product: "MOONDROP GM01 Pro".into(), serial: "shared".into() }
    }

    fn frame(level: u8, valid: u8, length: usize) -> Vec<u8> {
        // #127's public 50% prefix; trailing transport padding is reconstructed.
        let mut r = vec![0; length];
        r[..10].copy_from_slice(&[REPORT, 0x80, 0x27, 6, 0, 0, 0, 0, valid, level]);
        r
    }

    #[test]
    fn four_collection_receiver_selects_battery_collection_without_querying_volume() {
        let vendor = interface("one", 4, 0xff02, 2);
        let control = interface("one", 2, 0x000c, 1);
        let telephony = interface("one", 1, 0x000b, 5);
        let mut volume = interface("one", 3, 0x000c, 1);
        volume.input_len = 3; volume.output_len = 0;
        let interfaces = [vendor.clone(), control.clone(), telephony, volume];
        let selected = select_with(&interfaces, |i| {
            assert_ne!(i.input_len, 3, "volume collection is never passed to proof");
            i.path == control.path
        });
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].path, control.path);
        let sample = read_with(&target(selected[0]), |r| {
            assert_eq!(&r[..4], &[0x4b, 0x80, 0x27, 0]);
            assert!(r[4..].iter().all(|b| *b == 0));
            Some(frame(50, 1, 64))
        }).unwrap();
        assert_eq!(sample.battery.unwrap().percent, Some(50));
        assert_eq!(sample.target.family, DeviceFamily::Headset);
        assert_eq!(sample.target.key, target(&vendor).key);
    }

    #[test]
    fn selection_prefers_vendor_but_rejects_duplicate_vendor_and_proven_consumer_paths() {
        let a = interface("one", 1, 0xff02, 2);
        let b = interface("one", 2, 0x000c, 1);
        let c = interface("one", 3, 0x000c, 1);
        let all = [a.clone(), b.clone(), c.clone()];
        assert_eq!(select_with(&all, |_| true)[0].path, a.path);
        assert!(select_with(&[b.clone(), c], |_| true).is_empty());
        let duplicate = interface("one", 4, 0xff02, 2);
        assert!(select_with(&[a.clone(), b, duplicate], |i| i.path == a.path).is_empty());
        let other = interface("two", 1, 0xff02, 2);
        assert_eq!(select_with(&[a.clone(), other.clone()], |_| true).len(), 2);
        assert_ne!(target(&a).key, target(&other).key);
        assert!(select_with(&[a], |_| false).is_empty());
        let mut vendor = interface("indexed", 1, 0xff02, 2);
        vendor.path = vendor.path.replace("&0001#", "&0000#");
        let mut consumer = interface("indexed", 2, 0x000c, 1);
        consumer.path = consumer.path.replace("&0002#", "&0001#");
        assert_eq!(receiver_group(&vendor), receiver_group(&consumer));
        assert_eq!(select_with(&[vendor, consumer], |_| true).len(), 1);
    }

    #[test]
    fn only_exact_receiver_interface_and_collection_scope_is_eligible() {
        let base = interface("one", 2, 0x000c, 1);
        assert!(receiver_group(&base).is_some());
        for path in [base.path.replace("mi_03", "mi_02"), base.path.replace("&col02", ""),
            base.path.replace("&0002#", "&not-an-index#"), "unknown".into(), base.path.replace("0133", "0134")] {
            let mut wrong = base.clone(); wrong.path = path;
            assert!(receiver_group(&wrong).is_none());
        }
        for (vendor, product, page, usage) in [(0x35d8, 0x133, 0xff01, 1),
            (0x35d8, 0x133, 0x000b, 5), (0x35d8, 0x134, 0x000c, 1),
            (0x3837, 0x133, 0x000c, 1), (0x35d8, 0x133, 0x000c, 2)] {
            let mut wrong = base.clone(); wrong.vendor_id = vendor; wrong.product_id = product;
            wrong.usage_page = page; wrong.usage = usage;
            assert!(receiver_group(&wrong).is_none());
        }
        for (input, output) in [(9, 64), (64, 3), (4097, 64), (64, 4097)] {
            let mut wrong = base.clone(); wrong.input_len = input; wrong.output_len = output;
            assert!(select_with(&[wrong], |_| panic!("invalid capacity must not reach proof")).is_empty());
        }
    }

    #[test]
    fn percentage_zero_is_valid_but_gate_zero_and_invalid_frames_are_not_measurements() {
        for level in 0..=100 {
            let b = battery(&frame(level, 1, 64), 64).unwrap();
            assert_eq!(b.percent, Some(level));
            assert_eq!(b.charging, None);
            assert!(!b.approximate && b.estimated_percent.is_none());
        }
        assert!(battery(&frame(50, 0, 64), 64).is_none());
        for level in 101..=255 { assert!(battery(&frame(level, 1, 64), 64).is_none()); }
        for length in 0..64 { assert!(battery(&frame(50, 1, 64)[..length], 64).is_none()); }
        assert!(battery(&frame(50, 1, 65), 64).is_none());
        assert!(battery(&request(64).unwrap(), 64).is_none());
        for (offset, value) in [(0, 2), (0, 0x54), (2, 0x31), (2, 0x4f), (3, 0), (3, 5), (3, 61)] {
            let mut r = frame(50, 1, 64); r[offset] = value;
            assert!(battery(&r, 64).is_none());
        }
    }

    #[test]
    fn descriptor_capacities_are_independent_and_failed_reads_never_produce_fresh_samples() {
        let mut i = interface("one", 2, 0x000c, 1);
        i.input_len = 32; i.output_len = 16;
        let t = target(&i);
        let first = read_with(&t, |r| { assert_eq!(r.len(), 16); Some(frame(50, 1, 32)) }).unwrap();
        assert!(first.battery_at.is_some() && first.connected && !first.battery_unavailable);
        assert!(first.dpi.stages.is_empty() && !first.dpi.writable);
        assert!(read_with(&t, |_| None).is_err());
        assert!(read_with(&t, |_| Some(frame(50, 0, 32))).is_err());
        assert!(read_with(&t, |_| Some(frame(50, 1, 64))).is_err());
        let wake = read_with(&t, |_| Some(frame(49, 1, 32))).unwrap();
        assert_eq!(wake.battery.unwrap().percent, Some(49));
        assert!(wake.battery_at.is_some());
        let mut wrong = t; wrong.backend = BackendId::Headset;
        assert!(read_with(&wrong, |_| panic!("wrong backend must not send")).is_err());
    }
}
