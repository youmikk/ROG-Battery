//! ML903 NearLink Se 369B:0234 receiver battery query only.
//! Protocol and fixture: VigilWatt c97adc389b60b0e945a62cddacbaaace39887723, KNA7022 (MIT).
use super::*;
use crate::hid::HidDevice;
use std::{sync::TryLockError, time::Instant};

pub struct Thunderobot;

const LENGTH: u16 = 64;
const INPUT_REPORT: u8 = 0xa1;
const OUTPUT_REPORT: u8 = 0xa0;
const TIMEOUT_MS: u32 = 1_500;
const MAX_FRAMES: usize = 32;
const UNAVAILABLE: &str = "设备无有效状态回报";

fn control(interface: &HidInterface) -> bool {
    if interface.vendor_id != 0x369b || interface.product_id != 0x0234
        || interface.usage_page != 0xffa0 || interface.usage != 2
        || interface.input_len != LENGTH || interface.output_len != LENGTH { return false; }
    let path = interface.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    parts.len() == 4 && parts[0] == r"\\?\hid"
        && {
            let fields: Vec<_> = parts[1].split('&').collect();
            (2..=4).contains(&fields.len()) && fields[..2] == ["vid_369b", "pid_0234"]
                && fields.get(2).is_none_or(|field| field.strip_prefix("mi_")
                    .is_some_and(|value| value.len() == 2 && value.bytes().all(|byte| byte.is_ascii_hexdigit())))
                && fields.get(3).is_none_or(|field| field.strip_prefix("col")
                    .is_some_and(|value| value.len() == 2 && value.bytes().all(|byte| byte.is_ascii_hexdigit())))
        }
        && !parts[2].is_empty() && !parts[2].contains(['\\', '/'])
        && parts[3] == "{4d1e55b2-f16f-11cf-88cb-001111000030}"
        && !path.contains('\0')
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|interface| control(interface)
        && interfaces.iter().filter(|other| control(other)
            && channel::collection_group(interface) == channel::collection_group(other)).count() == 1).collect()
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    control(interface) && descriptor::Descriptor::open(api, interface)
        .is_some_and(|descriptor| descriptor.single_input_output_report_ids(interface, LENGTH, LENGTH)
            == Some((INPUT_REPORT, OUTPUT_REPORT)))
}

fn target(interface: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::Thunderobot, interface, 0);
    target.name = "THUNDEROBOT ML903 NearLink Se".into();
    target.family = DeviceFamily::Mouse;
    target
}

fn discover_with(interfaces: &[HidInterface], mut prove: impl FnMut(&HidInterface) -> bool)
    -> Vec<Target> {
    candidates(interfaces).into_iter().filter(|interface| prove(interface)).map(target).collect()
}

fn request() -> [u8; LENGTH as usize] {
    let mut frame = [0; LENGTH as usize];
    frame[..2].copy_from_slice(&[OUTPUT_REPORT, 2]);
    frame
}

fn battery(frame: &[u8]) -> Option<Battery> {
    if frame.len() != usize::from(LENGTH) || frame[0] != INPUT_REPORT { return None; }
    let mut offset = 1;
    let mut percent = None;
    let mut seen = std::collections::BTreeSet::new();
    while offset < frame.len() && frame[offset] != 0 {
        let header = frame.get(offset..offset + 2)?;
        let tag = u16::from_be_bytes([header[0], header[1]]);
        let length = match tag { 0x0204 => 2, 0x0211 => 4, 0x0213 => 6, 0x0225 | 0x0226 => 1, _ => return None };
        if !seen.insert(tag) { return None; }
        let payload = frame.get(offset + 2..offset + 2 + length)?;
        if tag == 0x0225 {
            if payload[0] > 100 { return None; }
            percent = Some(payload[0]);
        }
        offset += 2 + length;
    }
    if frame[offset..].iter().any(|byte| *byte != 0) { return None; }
    Some(Battery { percent: Some(percent?), estimated_percent: None, charging: None,
        voltage_mv: None, approximate: false, level: None })
}

fn read_with(original: &Target, prove: impl FnOnce(&HidInterface) -> bool,
    exchange: impl FnOnce(&HidInterface, &[u8], u32, &mut dyn FnMut(&[u8]) -> bool)
        -> Result<Option<Vec<u8>>, String>) -> Result<Snapshot, String> {
    let interface = original.hid()?;
    if original.backend != BackendId::Thunderobot || original.address != 0 || !control(interface) {
        return Err(UNAVAILABLE.into());
    }
    let lock = channel::lock(&interface.path);
    let _guard = match lock.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return Err(UNAVAILABLE.into()),
    };
    if !prove(interface) { return Err(UNAVAILABLE.into()); }
    let started = Instant::now();
    let mut frames = 0;
    let response = exchange(interface, &request(), TIMEOUT_MS, &mut |frame| {
        frames += 1;
        frame.first() == Some(&INPUT_REPORT) || frames >= MAX_FRAMES
    })?;
    let battery = response.as_deref().and_then(battery);
    trace::record(interface, BackendId::Thunderobot, "query-battery", 0x0225, 2,
        usize::from(LENGTH), response.as_ref().map_or(0, Vec::len), started,
        match (&response, &battery) {
            (_, Some(_)) => trace::Outcome::Ok,
            (Some(_), None) => trace::Outcome::InvalidData,
            (None, None) => trace::Outcome::NoReply,
        });
    let battery = battery.ok_or(UNAVAILABLE)?;
    let mut target = target(interface);
    target.verified_identity = true;
    Ok(Snapshot { target, battery: Some(battery), battery_at: Some(Instant::now()),
        battery_unavailable: false, connected: true, dpi: Dpi::default(), polling_rate: Default::default() })
}

impl DeviceBackend for Thunderobot {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(discover_with(interfaces, |interface| proven(api, interface)))
    }

    fn read(&self, api: &HidApi, original: &Target, _: bool) -> Result<Snapshot, String> {
        let interface = original.hid()?;
        if original.backend != BackendId::Thunderobot || original.address != 0 || !control(interface) {
            return Err(UNAVAILABLE.into());
        }
        let interfaces = enumeration::scan(api)?.interfaces;
        let current = candidates(&interfaces).into_iter().find(|candidate|
            candidate.path.eq_ignore_ascii_case(&interface.path)).ok_or(UNAVAILABLE)?;
        read_with(&target(current), |interface| proven(api, interface), |interface, request, timeout, matches| {
            let mut device = HidDevice::open(interface)?;
            if device.input_len() != LENGTH || device.output_len() != LENGTH || !device.writable {
                return Err(UNAVAILABLE.into());
            }
            Ok(device.transact(request, timeout, matches))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, sync::Arc};

    fn interface(instance: &str) -> HidInterface {
        HidInterface {
            path: format!(r"\\?\hid#vid_369b&pid_0234&mi_01&col04#7&{instance}&0&0004#{{4d1e55b2-f16f-11cf-88cb-001111000030}}"),
            vendor_id: 0x369b, product_id: 0x0234, usage_page: 0xffa0, usage: 2,
            input_len: LENGTH, output_len: LENGTH, feature_len: 0,
            product: "same receiver product".into(), serial: "same serial".into(),
        }
    }

    fn upstream_recorded_35_percent() -> Vec<u8> {
        let mut frame = vec![0; usize::from(LENGTH)];
        frame[..25].copy_from_slice(&[
            0xa1, 0x02, 0x04, 0x01, 0x11, 0x02, 0x11, 0xe3, 0x01, 0x9b, 0x36,
            0x02, 0x13, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x02, 0x25, 0x23,
            0x02, 0x26, 0x01,
        ]);
        frame
    }

    fn synthetic_read(instance: &str, frames: Vec<Vec<u8>>) -> (Result<Snapshot, String>, usize) {
        let mut consumed = 0;
        let result = read_with(&target(&interface(instance)), |_| true, |_, packet, timeout, matches| {
            assert_eq!(packet, request());
            assert_eq!(timeout, TIMEOUT_MS);
            for frame in frames {
                consumed += 1;
                if matches(&frame) { return Ok(Some(frame)); }
            }
            Ok(None)
        });
        (result, consumed)
    }

    #[test]
    fn parses_upstream_recorded_35_percent_without_guessing_other_fields() {
        let parsed = battery(&upstream_recorded_35_percent()).unwrap();
        assert_eq!(parsed, Battery { percent: Some(35), estimated_percent: None, charging: None,
            voltage_mv: None, approximate: false, level: None });
    }

    #[test]
    fn synthetic_percent_endpoints_and_workmode_remain_independent() {
        for percent in [0, 1, 27, 99, 100] {
            for workmode in [0, 1, 2, 100, 255] {
                let mut frame = upstream_recorded_35_percent();
                frame[21] = percent;
                frame[24] = workmode;
                let parsed = battery(&frame).unwrap();
                assert_eq!(parsed.percent, Some(percent));
                assert_eq!(parsed.charging, None);
            }
        }
        for invalid in 101..=255 {
            let mut frame = upstream_recorded_35_percent();
            frame[21] = invalid;
            assert!(battery(&frame).is_none());
        }
    }

    #[test]
    fn synthetic_embedded_serial_tag_is_not_battery_or_identity() {
        let mut frame = upstream_recorded_35_percent();
        frame[13..19].copy_from_slice(&[2, 0x25, 99, 2, 0x26, 1]);
        let (result, _) = synthetic_read("embedded", vec![frame]);
        let snapshot = result.unwrap();
        assert_eq!(snapshot.battery.unwrap().percent, Some(35));
        assert_eq!(snapshot.target.key, target(&interface("embedded")).key);
    }

    #[test]
    fn synthetic_wrong_ids_lengths_tags_and_nonzero_tails_are_rejected() {
        let sample = upstream_recorded_35_percent();
        for length in 0..64 { assert!(battery(&sample[..length]).is_none()); }
        let mut oversized = sample.clone();
        oversized.push(0);
        assert!(battery(&oversized).is_none());
        for offset in [0, 1, 2, 5, 6, 11, 12, 19, 20, 22, 23] {
            let mut changed = sample.clone();
            changed[offset] ^= 1;
            assert!(battery(&changed).is_none(), "offset {offset}");
        }
        for offset in 25..64 {
            let mut changed = sample.clone();
            changed[offset] = 1;
            assert!(battery(&changed).is_none(), "tail {offset}");
        }
        assert!(battery(&request()).is_none());
    }

    #[test]
    fn known_tag_boundaries_accept_optional_fields_and_order_but_reject_duplicates() {
        let sample = upstream_recorded_35_percent();
        for offset in [1, 5, 11, 19] {
            let mut missing = sample.clone();
            missing[offset..].fill(0);
            assert!(battery(&missing).is_none());
        }
        let mut reordered = sample.clone();
        reordered[19..25].copy_from_slice(&[2, 0x26, 1, 2, 0x25, 35]);
        assert_eq!(battery(&reordered).unwrap().percent, Some(35));
        let mut no_mode = sample.clone();
        no_mode[22..].fill(0);
        assert_eq!(battery(&no_mode).unwrap().percent, Some(35));
        for duplicate in [[2, 0x25, 35], [2, 0x26, 1], [2, 4, 0]] {
            let mut frame = sample.clone();
            frame[25..28].copy_from_slice(&duplicate);
            assert!(battery(&frame).is_none());
        }
        let mut duplicate = sample;
        duplicate[23] = 0x25;
        assert!(battery(&duplicate).is_none());
    }

    #[test]
    fn synthetic_interface_variants_never_reach_descriptor_or_transport() {
        let changes: &[fn(&mut HidInterface)] = &[
            |interface| interface.vendor_id ^= 1,
            |interface| interface.product_id ^= 1,
            |interface| interface.usage_page ^= 1,
            |interface| interface.usage = 1,
            |interface| interface.input_len = 63,
            |interface| interface.input_len = 65,
            |interface| interface.output_len = 63,
            |interface| interface.output_len = 65,
            |interface| interface.path = interface.path.replace("pid_0234", "pid_0235"),
            |interface| interface.path = interface.path.replace("mi_01", "mi_01&mi_00"),
            |interface| interface.path = interface.path.replace("col04", "col04&col05"),
            |interface| interface.path = interface.path.replace("7&gates&0&0004", ""),
            |interface| interface.path.push('\0'),
            |interface| interface.path.push('#'),
            |interface| interface.path = interface.path.replace("hid#", "bth#"),
            |interface| interface.path = interface.path.replace("4d1e55b2", "00000000"),
        ];
        for change in changes {
            let mut interface = interface("gates");
            change(&mut interface);
            assert!(!control(&interface));
            assert!(discover_with(&[interface.clone()], |_| panic!("descriptor reached")).is_empty());
            assert!(read_with(&target(&interface), |_| panic!("descriptor reached"),
                |_, _, _, _| panic!("transport reached")).is_err());
        }
    }

    #[test]
    fn synthetic_descriptor_denial_and_ambiguous_paths_never_query() {
        let interface = interface("descriptor");
        assert!(discover_with(&[interface.clone()], |_| false).is_empty());
        assert!(read_with(&target(&interface), |_| false,
            |_, _, _, _| panic!("transport reached")).is_err());
        let mut alias = interface.clone();
        alias.path.make_ascii_uppercase();
        assert!(discover_with(&[interface, alias], |_| panic!("ambiguous descriptor")).is_empty());
    }

    #[test]
    fn descriptor_proof_not_synthetic_interface_numbers_decides_compatibility() {
        let original = interface("descriptor-layout");
        let mut another = original.clone();
        another.path = another.path.replace("mi_01&col04", "mi_00&col03").replace("&0004#", "&0003#");
        another.feature_len = 65;
        assert_eq!(discover_with(&[another.clone()], |_| true).len(), 1);
        assert!(discover_with(&[another], |_| false).is_empty());
        let mut sibling = original.clone();
        sibling.path = sibling.path.replace("col04", "col03").replace("&0004#", "&0003#");
        assert!(discover_with(&[original, sibling], |_| panic!("ambiguous collections")).is_empty());
    }

    #[test]
    fn synthetic_foreign_backend_slot_and_endpoint_never_query() {
        let mut wrong = target(&interface("wrong-target"));
        wrong.backend = BackendId::StandardHid;
        assert!(read_with(&wrong, |_| panic!("descriptor reached"),
            |_, _, _, _| panic!("transport reached")).is_err());
        wrong.backend = BackendId::Thunderobot;
        wrong.address = 1;
        assert!(read_with(&wrong, |_| panic!("descriptor reached"),
            |_, _, _, _| panic!("transport reached")).is_err());
        wrong.address = 0;
        wrong.endpoint = Endpoint::XInput(0);
        assert!(read_with(&wrong, |_| panic!("descriptor reached"),
            |_, _, _, _| panic!("transport reached")).is_err());
    }

    #[test]
    fn synthetic_transcript_sends_one_fixed_query_and_skips_foreign_reports() {
        let mut other = upstream_recorded_35_percent();
        other[0] = 0xa2;
        let (result, consumed) = synthetic_read("transcript",
            vec![request().to_vec(), other, upstream_recorded_35_percent()]);
        let snapshot = result.unwrap();
        assert_eq!(consumed, 3);
        assert!(snapshot.connected && snapshot.target.verified_identity);
        assert!(snapshot.battery_at.is_some());
        assert!(!snapshot.battery_unavailable);
        assert!(!snapshot.polling_rate.writable);
        assert_eq!(&request()[..2], &[0xa0, 2]);
        assert!(request()[2..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn synthetic_invalid_matching_response_ends_transaction_before_later_sample() {
        for bad in [vec![INPUT_REPORT], {
            let mut frame = upstream_recorded_35_percent();
            frame[21] = 101;
            frame
        }, {
            let mut frame = upstream_recorded_35_percent();
            frame[23] = 0x25;
            frame
        }] {
            let (result, consumed) = synthetic_read("invalid-first", vec![bad, upstream_recorded_35_percent()]);
            assert!(result.is_err());
            assert_eq!(consumed, 1);
        }
    }

    #[test]
    fn synthetic_noise_budget_stops_before_later_sample() {
        let mut frames = vec![request().to_vec(); MAX_FRAMES];
        frames.push(upstream_recorded_35_percent());
        let (result, consumed) = synthetic_read("noise", frames);
        assert!(result.is_err());
        assert_eq!(consumed, MAX_FRAMES);
    }

    #[test]
    fn synthetic_timeout_and_write_error_do_not_publish_offline_or_reuse_battery() {
        let (first, _) = synthetic_read("timeout", vec![upstream_recorded_35_percent()]);
        assert!(first.is_ok());
        assert!(synthetic_read("timeout", vec![]).0.is_err());
        let calls = Cell::new(0);
        let result = read_with(&target(&interface("write-error")), |_| true, |_, _, _, _| {
            calls.set(calls.get() + 1);
            Err("synthetic write failure".into())
        });
        assert!(result.is_err());
        assert_eq!(calls.get(), 1);
        let (recovered, _) = synthetic_read("timeout", vec![upstream_recorded_35_percent()]);
        assert!(recovered.unwrap().battery_at.is_some());
    }

    #[test]
    fn synthetic_identical_serials_have_distinct_keys_and_channel_locks() {
        let first = interface("receiver-a");
        let second = interface("receiver-b");
        let targets = discover_with(&[first.clone(), second.clone()], |_| true);
        assert_eq!(targets.len(), 2);
        assert_ne!(targets[0].key, targets[1].key);
        assert!(targets.iter().all(|target| !target.verified_identity));
        assert!(!targets[0].same_channel(&targets[1]));
        let first_lock = channel::lock(&first.path);
        assert!(Arc::ptr_eq(&first_lock, &channel::lock(&first.path.to_ascii_uppercase())));
        assert!(!Arc::ptr_eq(&first_lock, &channel::lock(&second.path)));
        let _guard = first_lock.lock().unwrap();
        assert!(read_with(&target(&first), |_| panic!("locked descriptor"),
            |_, _, _, _| panic!("locked transport")).is_err());
        assert!(synthetic_read("receiver-b", vec![upstream_recorded_35_percent()]).0.is_ok());
        let mut alias = second.clone();
        alias.path.make_ascii_uppercase();
        assert!(control(&alias));
        assert_eq!(target(&second).key, target(&alias).key);
    }
}
