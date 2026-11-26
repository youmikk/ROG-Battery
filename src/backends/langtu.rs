use super::*;
use crate::hid::HidDevice;
use std::{sync::TryLockError, time::{Duration, Instant}};

pub struct LangtuT98;

const PRODUCT: &str = "2.4 Wireless Device";
const LENGTH: u16 = 65;
const TIMEOUT_MS: u32 = 7_000;
const MAX_FRAMES: usize = 32;
const UNAVAILABLE: &str = "设备无有效状态回报";

fn management(interface: &HidInterface) -> bool {
    interface.vendor_id == 0x1a2c && interface.product_id == 0x8fff
        && interface.usage_page == 0xff02 && interface.usage == 2
}

fn receiver_group(interface: &HidInterface) -> Option<String> {
    if !management(interface) { return None; }
    let path = interface.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid"
        || parts[2].is_empty() || parts[2].contains(['\\', '/'])
        || parts[3] != "{4d1e55b2-f16f-11cf-88cb-001111000030}"
        || path.contains('\0') { return None; }
    let fields: Vec<_> = parts[1].split('&').collect();
    if !(3..=4).contains(&fields.len())
        || fields[..3] != ["vid_1a2c", "pid_8fff", "mi_00"] { return None; }
    if fields.len() == 3 { return Some(path); }
    let collection = fields[3].strip_prefix("col")?;
    if collection.len() != 2 || !collection.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let (instance, suffix) = parts[2].rsplit_once('&')?;
    if instance.split('&').count() < 2 || suffix.len() != 4 || !suffix.starts_with("00")
        || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) { return None; }
    Some(format!("{}#{}#{instance}#{}", parts[0], fields[..3].join("&"), parts[3]))
}

fn control(interface: &HidInterface) -> bool {
    management(interface) && interface.product == PRODUCT
        && interface.input_len == LENGTH && interface.output_len == LENGTH
        && receiver_group(interface).is_some()
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|interface| control(interface)
        && interfaces.iter().filter(|other| management(other)
            && receiver_group(interface) == receiver_group(other)).count() == 1).collect()
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    control(interface) && descriptor::Descriptor::open(api, interface)
        .is_some_and(|descriptor| descriptor.input_report_ids(interface, LENGTH, &[0]))
}

fn target(interface: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::LangtuT98, interface, 0);
    target.name = "LANGTU T98".into();
    target.family = DeviceFamily::Keyboard;
    target
}

fn discover_with(interfaces: &[HidInterface], mut prove: impl FnMut(&HidInterface) -> bool)
    -> Vec<Target> {
    candidates(interfaces).into_iter().filter(|interface| prove(interface)).map(target).collect()
}

fn battery(frame: &[u8]) -> Option<Battery> {
    if frame.len() != usize::from(LENGTH) || frame[..6] != [0, 0xaa, 0x55, 0xcc, 0x33, 0x0e]
        || !(1..=100).contains(&frame[6]) || frame[7] != 1
        || frame[8..].iter().any(|byte| *byte != 0) { return None; }
    Some(Battery { percent: None, estimated_percent: Some(frame[6]), approximate: true,
        charging: None, voltage_mv: None, level: None })
}

fn receive_with(mut remaining: impl FnMut() -> Option<Duration>,
    mut read: impl FnMut(u32) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    for _ in 0..MAX_FRAMES {
        let left = remaining()?;
        if left.is_zero() { return None; }
        let timeout = left.as_millis().min(u128::from(TIMEOUT_MS)).max(1) as u32;
        let frame = read(timeout)?;
        if battery(&frame).is_some() { return Some(frame); }
    }
    None
}

fn read_with(original: &Target, interfaces: &[HidInterface],
    mut prove: impl FnMut(&HidInterface) -> bool,
    receive: impl FnOnce(&HidInterface) -> Result<Option<Vec<u8>>, String>) -> Result<Snapshot, String> {
    let original_interface = original.hid()?;
    if original.backend != BackendId::LangtuT98 || original.address != 0 || !control(original_interface) {
        return Err(UNAVAILABLE.into());
    }
    let interface = candidates(interfaces).into_iter().find(|candidate|
        candidate.path.eq_ignore_ascii_case(&original_interface.path)).ok_or(UNAVAILABLE)?;
    let lock = channel::lock(&receiver_group(interface).ok_or(UNAVAILABLE)?);
    let _guard = match lock.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return Err(UNAVAILABLE.into()),
    };
    if !prove(interface) { return Err(UNAVAILABLE.into()); }
    let started = Instant::now();
    let response = receive(interface)?;
    let battery = response.as_deref().and_then(battery);
    let received = Instant::now();
    trace::record(interface, BackendId::LangtuT98, "passive-battery", 0x0e, 0,
        0, response.as_ref().map_or(0, Vec::len), started,
        match (&response, &battery) {
            (_, Some(_)) => trace::Outcome::Ok,
            (Some(_), None) => trace::Outcome::InvalidData,
            (None, None) => trace::Outcome::NoReply,
        });
    let battery = battery.ok_or(UNAVAILABLE)?;
    if !prove(interface) { return Err(UNAVAILABLE.into()); }
    Ok(Snapshot { target: target(interface), battery: Some(battery), battery_at: Some(received), connected: true,
        battery_unavailable: false, dpi: Dpi::default(), polling_rate: Default::default() })
}

impl DeviceBackend for LangtuT98 {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(discover_with(interfaces, |interface| proven(api, interface)))
    }

    fn read(&self, api: &HidApi, original: &Target, _: bool) -> Result<Snapshot, String> {
        if original.backend != BackendId::LangtuT98 || original.address != 0 || !control(original.hid()?) {
            return Err(UNAVAILABLE.into());
        }
        let interfaces = enumeration::scan(api)?.interfaces;
        read_with(original, &interfaces, |interface| proven(api, interface), |interface| {
            let mut device = HidDevice::open_read_only(interface)?;
            if device.input_len() != LENGTH || device.output_len() != LENGTH || device.writable {
                return Err(UNAVAILABLE.into());
            }
            let deadline = Instant::now() + Duration::from_millis(u64::from(TIMEOUT_MS));
            Ok(receive_with(|| deadline.checked_duration_since(Instant::now()),
                |timeout| device.read_report(timeout)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, sync::Arc};

    fn interface(instance: &str) -> HidInterface {
        HidInterface {
            path: format!(r"\\?\hid#vid_1a2c&pid_8fff&mi_00#7&{instance}&0&0000#{{4d1e55b2-f16f-11cf-88cb-001111000030}}"),
            vendor_id: 0x1a2c, product_id: 0x8fff, usage_page: 0xff02, usage: 2,
            input_len: LENGTH, output_len: LENGTH, feature_len: 0,
            product: PRODUCT.into(), serial: "same shared serial".into(),
        }
    }

    fn frame(value: u8) -> Vec<u8> {
        let mut frame = vec![0; usize::from(LENGTH)];
        frame[..8].copy_from_slice(&[0, 0xaa, 0x55, 0xcc, 0x33, 0x0e, value, 1]);
        frame
    }

    #[test]
    fn documented_values_remain_estimates_without_charge_or_writes() {
        let interface = interface("documented");
        for value in [78, 87, 92] {
            let snapshot = read_with(&target(&interface), std::slice::from_ref(&interface),
                |_| true, |_| Ok(Some(frame(value)))).unwrap();
            let battery = snapshot.battery.unwrap();
            assert_eq!(battery.percent, None);
            assert_eq!(battery.estimated_percent, Some(value));
            assert!(battery.approximate);
            assert_eq!(battery.display_percent().unwrap().label(true), format!("≈{value}%"));
            assert_eq!((battery.charging, battery.voltage_mv, battery.level), (None, None, None));
            assert_eq!(snapshot.target.family, DeviceFamily::Keyboard);
            assert!(snapshot.connected && snapshot.battery_at.is_some());
            assert!(!snapshot.target.verified_identity);
            assert_eq!(snapshot.target.key, target(&interface).key);
            assert!(!snapshot.battery_unavailable && !snapshot.dpi.writable && !snapshot.polling_rate.writable);
        }
    }

    #[test]
    fn zero_sentinel_and_out_of_range_values_never_become_battery() {
        for value in 1..=100 {
            assert_eq!(battery(&frame(value)).unwrap().estimated_percent, Some(value));
        }
        assert!(battery(&frame(0)).is_none());
        for value in 101..=255 { assert!(battery(&frame(value)).is_none()); }
        assert!(battery(&[0; LENGTH as usize]).is_none());
    }

    #[test]
    fn full_windows_frame_and_every_fixed_byte_are_required() {
        let valid = frame(78);
        for length in 0..valid.len() { assert!(battery(&valid[..length]).is_none()); }
        assert!(battery(&valid[1..]).is_none());
        let mut too_long = valid.clone();
        too_long.push(0);
        assert!(battery(&too_long).is_none());
        for offset in (0..valid.len()).filter(|offset| *offset != 6) {
            let mut wrong = valid.clone();
            wrong[offset] ^= 1;
            assert!(battery(&wrong).is_none(), "offset {offset}");
        }
    }

    #[test]
    fn exact_scope_rejects_neighbor_models_collections_names_and_lengths() {
        let valid = interface("scope");
        assert!(control(&valid));
        for mutate in [
            |interface: &mut HidInterface| interface.vendor_id = 0x1a2d,
            |interface: &mut HidInterface| interface.product_id = 0x7fff,
            |interface: &mut HidInterface| interface.usage_page = 0xff01,
            |interface: &mut HidInterface| interface.usage = 0xff01,
            |interface: &mut HidInterface| interface.input_len = 64,
            |interface: &mut HidInterface| interface.input_len = 66,
            |interface: &mut HidInterface| interface.output_len = 0,
            |interface: &mut HidInterface| interface.output_len = 64,
            |interface: &mut HidInterface| interface.product = "LANGTU T98".into(),
            |interface: &mut HidInterface| interface.path = interface.path.replace("mi_00", "mi_03"),
        ] {
            let mut wrong = valid.clone();
            mutate(&mut wrong);
            assert!(!control(&wrong));
            assert!(discover_with(&[wrong.clone()], |_| panic!("scope must precede proof")).is_empty());
            assert!(read_with(&target(&wrong), &[wrong], |_| panic!("wrong scope"),
                |_| panic!("wrong scope must not open")).is_err());
        }
    }

    #[test]
    fn malformed_or_conflicting_windows_paths_are_rejected() {
        let valid = interface("paths");
        for (from, to) in [
            ("vid_1a2c", "vid_1a2d"), ("pid_8fff", "pid_7fff"),
            ("mi_00", "mi_000"), ("mi_00", "mi_00&mi_00"),
            ("mi_00", "mi_00&vid_1a2c"), ("mi_00", "mi_00&col1"),
            ("mi_00", "mi_00&colgg"), ("7&paths&0&0000", ""),
            ("7&paths&0&0000", "7&paths\\bad&0&0000"),
            ("001111000030}", "001111000031}"),
        ] {
            let mut wrong = valid.clone();
            wrong.path = wrong.path.replace(from, to);
            assert_ne!(wrong.path, valid.path);
            assert!(!control(&wrong), "{}", wrong.path);
        }
        let mut nul = valid.clone();
        nul.path.push('\0');
        assert!(!control(&nul));
        let mut uppercase = valid.clone();
        uppercase.path = uppercase.path.to_ascii_uppercase();
        assert!(control(&uppercase));
        assert_eq!(receiver_group(&uppercase), receiver_group(&valid));
    }

    #[test]
    fn descriptor_denial_prevents_discovery_and_reading() {
        let interface = interface("proof-denied");
        assert!(discover_with(std::slice::from_ref(&interface), |_| false).is_empty());
        let found = discover_with(std::slice::from_ref(&interface), |_| true);
        assert_eq!(found.len(), 1);
        assert!(!found[0].verified_identity);
        assert!(read_with(&found[0], &[interface], |_| false,
            |_| panic!("descriptor denial must not open input")).is_err());
    }

    #[test]
    fn stale_target_and_changed_metadata_cannot_open_input() {
        let interface = interface("stale");
        let original = target(&interface);
        assert!(read_with(&original, &[], |_| panic!("removed"), |_| panic!("removed")).is_err());
        let mut changed = interface.clone();
        changed.input_len = 64;
        assert!(read_with(&original, &[changed], |_| panic!("changed"), |_| panic!("changed")).is_err());
        let mut foreign = original.clone();
        foreign.backend = BackendId::StandardHid;
        assert!(read_with(&foreign, std::slice::from_ref(&interface), |_| panic!("backend"),
            |_| panic!("backend")).is_err());
        let mut addressed = original;
        addressed.address = 1;
        assert!(read_with(&addressed, &[interface], |_| panic!("address"), |_| panic!("address")).is_err());
    }

    #[test]
    fn unplug_after_input_does_not_publish_a_connected_snapshot() {
        let interface = interface("unplug");
        let proofs = Cell::new(0);
        assert!(read_with(&target(&interface), &[interface], |_| {
            proofs.set(proofs.get() + 1);
            proofs.get() == 1
        }, |_| Ok(Some(frame(78)))).is_err());
        assert_eq!(proofs.get(), 2);
    }

    #[test]
    fn timeout_invalid_data_and_io_error_never_reuse_an_earlier_sample() {
        let interface = interface("no-cache");
        let original = target(&interface);
        let read = |response| read_with(&original, std::slice::from_ref(&interface), |_| true,
            |_| response);
        let first = read(Ok(Some(frame(78)))).unwrap();
        for response in [Ok(None), Ok(Some(frame(0))), Ok(Some(frame(255))), Err("unplugged".into())] {
            assert!(read(response).is_err());
        }
        let recovered = read(Ok(Some(frame(87)))).unwrap();
        assert_eq!(recovered.battery.unwrap().estimated_percent, Some(87));
        assert!(recovered.battery_at >= first.battery_at);
    }

    #[test]
    fn receive_stops_on_first_valid_frame_and_shrinks_its_deadline() {
        let mut durations = [7_000, 6_000, 1_000].into_iter();
        let mut inputs = [frame(255), vec![0; 65], frame(78)].into_iter();
        let mut timeouts = Vec::new();
        let received = receive_with(|| durations.next().map(Duration::from_millis), |timeout| {
            timeouts.push(timeout);
            inputs.next()
        });
        assert_eq!(received, Some(frame(78)));
        assert_eq!(timeouts, [7_000, 6_000, 1_000]);
    }

    #[test]
    fn finite_frame_and_time_budgets_stop_noise_silence_and_expiry() {
        let mut reads = 0;
        assert!(receive_with(|| Some(Duration::from_secs(7)), |_| {
            reads += 1;
            Some(frame(255))
        }).is_none());
        assert_eq!(reads, MAX_FRAMES);
        let mut reads = 0;
        assert!(receive_with(|| Some(Duration::from_secs(7)), |timeout| {
            assert_eq!(timeout, TIMEOUT_MS);
            reads += 1;
            None
        }).is_none());
        assert_eq!(reads, 1);
        for expired in [None, Some(Duration::ZERO)] {
            assert!(receive_with(|| expired, |_| panic!("expired budget")).is_none());
        }
        let mut calls = 0;
        assert!(receive_with(|| {
            calls += 1;
            (calls == 1).then_some(Duration::from_millis(1))
        }, |timeout| {
            assert_eq!(timeout, 1);
            Some(frame(255))
        }).is_none());
        assert_eq!(calls, 2);
    }

    #[test]
    fn duplicate_management_collections_are_rejected_before_descriptor_access() {
        let mut first = interface("duplicate");
        first.path = first.path.replace("mi_00#", "mi_00&col01#");
        let mut second = first.clone();
        second.path = second.path.replace("col01", "col02").replace("&0000#", "&0001#");
        second.input_len = 64;
        assert_eq!(receiver_group(&first), receiver_group(&second));
        assert!(discover_with(&[first.clone(), second], |_| panic!("ambiguous")).is_empty());
        let mut alias = first.clone();
        alias.path = alias.path.to_ascii_uppercase();
        assert!(candidates(&[first, alias]).is_empty());
    }

    #[test]
    fn same_serial_receivers_keep_separate_keys_locks_and_values() {
        let first = interface("isolation-one");
        let second = interface("isolation-two");
        let interfaces = [first.clone(), second.clone()];
        assert_eq!(candidates(&interfaces).len(), 2);
        assert_ne!(target(&first).key, target(&second).key);
        let first_lock = channel::lock(&receiver_group(&first).unwrap());
        let second_lock = channel::lock(&receiver_group(&second).unwrap());
        assert!(!Arc::ptr_eq(&first_lock, &second_lock));
        let guard = first_lock.lock().unwrap();
        assert!(read_with(&target(&first), &interfaces, |_| panic!("busy"), |_| panic!("busy")).is_err());
        let snapshot = read_with(&target(&second), &interfaces, |_| true, |selected| {
            assert_eq!(selected.path, second.path);
            Ok(Some(frame(87)))
        }).unwrap();
        assert_eq!(snapshot.target.key, target(&second).key);
        assert_eq!(snapshot.battery.unwrap().estimated_percent, Some(87));
        drop(guard);
        let recovered = read_with(&target(&first), &interfaces, |_| true,
            |_| Ok(Some(frame(78)))).unwrap();
        assert_eq!(recovered.battery.unwrap().estimated_percent, Some(78));
    }

    #[test]
    fn case_insensitive_channel_lock_spans_both_proofs_and_the_read() {
        let interface = interface("lock-scope");
        let mut uppercase = interface.clone();
        uppercase.path = uppercase.path.to_ascii_uppercase();
        let lock = channel::lock(&receiver_group(&uppercase).unwrap());
        assert!(Arc::ptr_eq(&lock, &channel::lock(&receiver_group(&interface).unwrap())));
        let checks = Cell::new(0);
        read_with(&target(&uppercase), &[interface], |_| {
            assert!(matches!(lock.try_lock(), Err(TryLockError::WouldBlock)));
            checks.set(checks.get() + 1);
            true
        }, |_| {
            assert!(matches!(lock.try_lock(), Err(TryLockError::WouldBlock)));
            Ok(Some(frame(78)))
        }).unwrap();
        assert_eq!(checks.get(), 2);
        assert!(lock.try_lock().is_ok());
    }
}
