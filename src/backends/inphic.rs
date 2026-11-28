//! ZHEN passive report 03; evidence: VigilWatt c97adc389b60b0e945a62cddacbaaace39887723.
use super::*;
use crate::hid::HidDevice;
use std::{sync::TryLockError, time::{Duration, Instant}};

pub struct Inphic;

pub(super) const SAMPLE_LIFETIME: Duration = Duration::from_secs(90);
const LENGTH: u16 = 5;
const REPORT_ID: u8 = 3;
const TIMEOUT_MS: u32 = 5_000;
const MAX_FRAMES: usize = 32;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(interface: &HidInterface) -> bool {
    interface.vendor_id == 0x1d57 && interface.product_id == 0xfa65
        && interface.usage_page == 0xff00 && interface.usage == 1
}

fn receiver_group(interface: &HidInterface) -> Option<String> {
    if !management(interface) { return None; }
    let path = interface.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid"
        || parts[2].is_empty() || parts[2].contains(['\\', '/'])
        || parts[2].split('&').any(str::is_empty)
        || parts[3] != "{4d1e55b2-f16f-11cf-88cb-001111000030}"
        || path.contains('\0') { return None; }
    let fields: Vec<_> = parts[1].split('&').collect();
    if !(2..=4).contains(&fields.len()) || fields[..2] != ["vid_1d57", "pid_fa65"] {
        return None;
    }
    let hex_pair = |value: &str| value.len() == 2
        && value.bytes().all(|byte| byte.is_ascii_hexdigit());
    let mut next = 2;
    if let Some(number) = fields.get(next).and_then(|field| field.strip_prefix("mi_")) {
        if !hex_pair(number) { return None; }
        next += 1;
    }
    if next == fields.len() { return Some(path); }
    let collection = fields[next].strip_prefix("col")?;
    if next + 1 != fields.len() || !hex_pair(collection) || collection == "00" {
        return None;
    }
    let (instance, suffix) = parts[2].rsplit_once('&')?;
    if instance.split('&').count() < 2 || suffix.len() != 4 || !suffix.starts_with("00")
        || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) { return None; }
    Some(format!("{}#{}#{instance}#{}", parts[0], fields[..next].join("&"), parts[3]))
}

fn control(interface: &HidInterface) -> bool {
    management(interface) && interface.input_len == LENGTH && receiver_group(interface).is_some()
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|interface| control(interface)
        && interfaces.iter().filter(|other| management(other)
            && receiver_group(other) == receiver_group(interface)).count() == 1).collect()
}

fn descriptor_gate(interface: &HidInterface,
    proof: impl FnOnce(u16, &[u8]) -> bool) -> bool {
    control(interface) && proof(LENGTH, &[REPORT_ID])
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    descriptor_gate(interface, |length, ids| descriptor::Descriptor::open(api, interface)
        .is_some_and(|descriptor| descriptor.input_report_ids(interface, length, ids)))
}

fn target(interface: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::Inphic, interface, 0);
    target.name = "INPHIC ZHEN".into();
    target.family = DeviceFamily::Mouse;
    target
}

fn valid_target(original: &Target) -> bool {
    original.backend == BackendId::Inphic && original.address == 0
        && original.hid().is_ok_and(|interface|
            control(interface) && original.key == target(interface).key)
}

fn discover_with(interfaces: &[HidInterface], mut prove: impl FnMut(&HidInterface) -> bool)
    -> Vec<Target> {
    candidates(interfaces).into_iter().filter(|interface| prove(interface)).map(target).collect()
}

fn battery(frame: &[u8]) -> Option<Battery> {
    if frame.len() != usize::from(LENGTH) || frame[..3] != [REPORT_ID, 0x76, 0x41]
        || frame[4] > 100 { return None; }
    Some(Battery {
        percent: Some(frame[4]), estimated_percent: None, approximate: false,
        charging: match frame[3] { 1 => Some(false), 3 => Some(true), _ => None },
        voltage_mv: None, level: None,
    })
}

fn receive_with(mut remaining: impl FnMut() -> Option<Duration>,
    mut read: impl FnMut(u32) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    let mut last_invalid = None;
    for _ in 0..MAX_FRAMES {
        let Some(left) = remaining().filter(|left| !left.is_zero()) else { break; };
        let timeout = left.as_millis().min(u128::from(TIMEOUT_MS)).max(1) as u32;
        let Some(frame) = read(timeout) else { break; };
        if battery(&frame).is_some() { return Some(frame); }
        last_invalid = Some(frame);
    }
    last_invalid
}

struct Channel { device: HidDevice }

impl Channel {
    fn open(interface: &HidInterface) -> Result<Self, String> {
        let device = HidDevice::open_read_only(interface)?;
        if device.input_len() != LENGTH || device.writable { return Err(UNAVAILABLE.into()); }
        Ok(Self { device })
    }

    fn receive(&mut self) -> Option<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_millis(u64::from(TIMEOUT_MS));
        receive_with(|| deadline.checked_duration_since(Instant::now()),
            |timeout| self.device.read_report(timeout))
    }
}

fn read_with(original: &Target, interfaces: &[HidInterface],
    mut prove: impl FnMut(&HidInterface) -> bool,
    receive: impl FnOnce(&HidInterface) -> Result<Option<Vec<u8>>, String>) -> Result<Snapshot, String> {
    if !valid_target(original) { return Err(UNAVAILABLE.into()); }
    let original_interface = original.hid()?;
    let interface = candidates(interfaces).into_iter().find(|candidate|
        candidate.path.eq_ignore_ascii_case(&original_interface.path)
            && candidate.serial == original_interface.serial).ok_or(UNAVAILABLE)?;
    let lock = channel::lock(&receiver_group(interface).ok_or(UNAVAILABLE)?);
    let _guard = match lock.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return Err(UNAVAILABLE.into()),
    };
    if !prove(interface) { return Err(UNAVAILABLE.into()); }
    let started = Instant::now();
    let response = receive(interface);
    let received = Instant::now();
    let frame = response.as_ref().ok().and_then(|frame| frame.as_deref());
    let sample = frame.and_then(battery);
    trace::record_sample(interface, BackendId::Inphic, "passive-battery", u16::from(REPORT_ID), 0,
        0, frame.map_or(0, <[u8]>::len), started,
        match (frame, &sample) {
            (_, Some(_)) => trace::Outcome::Ok,
            (Some(_), None) => trace::Outcome::InvalidData,
            (None, None) => trace::Outcome::NoReply,
        }, sample.as_ref().map(|_| received));
    let _ = response?;
    let battery = sample.ok_or(UNAVAILABLE)?;
    if !prove(interface) { return Err(UNAVAILABLE.into()); }
    Ok(Snapshot {
        target: target(interface), battery: Some(battery), battery_at: Some(received),
        connected: true, battery_unavailable: false,
        dpi: Dpi::default(), polling_rate: Default::default(),
    })
}

impl DeviceBackend for Inphic {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(discover_with(interfaces, |interface| proven(api, interface)))
    }

    fn read(&self, api: &HidApi, original: &Target, _: bool) -> Result<Snapshot, String> {
        if !valid_target(original) { return Err(UNAVAILABLE.into()); }
        let interfaces = enumeration::scan(api)?.interfaces;
        read_with(original, &interfaces, |interface| proven(api, interface),
            |interface| Ok(Channel::open(interface)?.receive()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::ReadingQuality;
    use std::{cell::Cell, sync::Arc};

    const DISCHARGING: [u8; 5] = [3, 0x76, 0x41, 1, 0x55];
    const CHARGING: [u8; 5] = [3, 0x76, 0x41, 3, 0x54];

    // Windows paths are synthetic; the two report fixtures are recorded upstream.
    fn interface(instance: &str) -> HidInterface {
        HidInterface {
            path: format!(r"\\?\hid#vid_1d57&pid_fa65&mi_01&col03#7&{instance}&0&0002#{{4d1e55b2-f16f-11cf-88cb-001111000030}}"),
            vendor_id: 0x1d57, product_id: 0xfa65, usage_page: 0xff00, usage: 1,
            input_len: LENGTH, output_len: 0, feature_len: 0,
            product: "Inphic KP 8K".into(), serial: "shared receiver serial".into(),
        }
    }

    fn read_frame(interface: &HidInterface, frame: &[u8]) -> Snapshot {
        read_with(&target(interface), std::slice::from_ref(interface), |_| true,
            |_| Ok(Some(frame.to_vec()))).unwrap()
    }

    #[test]
    fn recorded_frames_publish_exact_percentage_and_only_proven_charge_states() {
        let interface = interface("recorded");
        for (frame, percent, charging) in [(DISCHARGING, 85, false), (CHARGING, 84, true)] {
            let snapshot = read_frame(&interface, &frame);
            let battery = snapshot.battery.unwrap();
            assert_eq!(battery.percent, Some(percent));
            assert_eq!(battery.charging, Some(charging));
            assert!(!battery.approximate);
            assert_eq!((battery.estimated_percent, battery.voltage_mv, battery.level), (None, None, None));
            assert!(snapshot.connected && snapshot.battery_at.is_some() && !snapshot.battery_unavailable);
            assert_eq!(snapshot.target.family, DeviceFamily::Mouse);
            assert_eq!(snapshot.target.key, target(&interface).key);
            assert!(!snapshot.target.verified_identity);
            assert!(!snapshot.dpi.writable && snapshot.dpi.stages.is_empty());
            assert!(!snapshot.polling_rate.writable);
        }
    }

    #[test]
    fn zero_and_full_are_percentages_and_unrecorded_flags_stay_unknown() {
        for value in 0..=100 {
            let mut frame = DISCHARGING;
            frame[4] = value;
            assert_eq!(battery(&frame).unwrap().percent, Some(value));
        }
        for flag in 0..=255 {
            if matches!(flag, 1 | 3) { continue; }
            let frame = [REPORT_ID, 0x76, 0x41, flag, 100];
            let sample = battery(&frame).unwrap();
            assert_eq!(sample.charging, None);
            assert_eq!(sample.percent, Some(100));
            assert_eq!(sample.level, None);
        }
    }

    #[test]
    fn truncation_extra_bytes_wrong_id_markers_and_overflow_are_rejected() {
        for length in 0..DISCHARGING.len() {
            assert!(battery(&DISCHARGING[..length]).is_none());
        }
        assert!(battery(&DISCHARGING[1..]).is_none());
        let mut extended = DISCHARGING.to_vec();
        extended.push(0);
        assert!(battery(&extended).is_none());
        assert!(battery(&[0; 5]).is_none());
        for offset in 0..3 {
            let mut frame = DISCHARGING;
            frame[offset] ^= 1;
            assert!(battery(&frame).is_none());
        }
        for value in 101..=255 {
            let mut frame = DISCHARGING;
            frame[4] = value;
            assert!(battery(&frame).is_none());
        }
    }

    #[test]
    fn exact_scope_excludes_in9_neighbor_pids_and_non_battery_collections() {
        let valid = interface("scope");
        for mutate in [
            |interface: &mut HidInterface| interface.vendor_id = 0x248a,
            |interface: &mut HidInterface| interface.product_id = 0x5b2f,
            |interface: &mut HidInterface| interface.product_id = 0xfa64,
            |interface: &mut HidInterface| interface.usage_page = 1,
            |interface: &mut HidInterface| interface.usage_page = 0x000c,
            |interface: &mut HidInterface| interface.usage_page = 0xff01,
            |interface: &mut HidInterface| interface.usage = 2,
            |interface: &mut HidInterface| interface.input_len = 4,
            |interface: &mut HidInterface| interface.input_len = 6,
            |interface: &mut HidInterface| interface.input_len = 64,
        ] {
            let mut wrong = valid.clone();
            mutate(&mut wrong);
            assert!(!control(&wrong));
            assert!(discover_with(&[wrong.clone()], |_| panic!("wrong scope")).is_empty());
            assert!(read_with(&target(&wrong), &[wrong], |_| panic!("wrong scope"),
                |_| panic!("must not open input")).is_err());
        }
        let mut in9 = valid;
        in9.vendor_id = 0x248a;
        in9.product_id = 0x5b2f;
        in9.path = in9.path.replace("vid_1d57&pid_fa65", "vid_248a&pid_5b2f");
        assert!(!control(&in9));
    }

    #[test]
    fn no_unproven_product_interface_number_or_output_capability_is_required() {
        let valid = interface("metadata");
        for hardware in ["vid_1d57&pid_fa65", "vid_1d57&pid_fa65&mi_00",
            "vid_1d57&pid_fa65&col03", "vid_1d57&pid_fa65&mi_02&col03"] {
            let mut changed = valid.clone();
            changed.path = changed.path.replace("vid_1d57&pid_fa65&mi_01&col03", hardware);
            changed.product.clear();
            changed.output_len = 64;
            changed.feature_len = 33;
            assert!(control(&changed));
            assert_eq!(discover_with(&[changed], |_| true).len(), 1);
        }
    }

    #[test]
    fn conflicting_or_malformed_windows_identity_is_not_claimed() {
        let valid = interface("paths");
        for (from, to) in [
            ("vid_1d57", "vid_248a"), ("pid_fa65", "pid_fa64"),
            ("mi_01", "mi_001"), ("mi_01", "mi_gg"),
            ("mi_01", "mi_01&mi_01"), ("mi_01", "rev_0001"),
            ("col03", "col3"), ("col03", "colgg"), ("col03", "col00"),
            ("col03", "col03&col04"), ("7&paths&0&0002", ""),
            ("7&paths&0&0002", "7&&0&0002"), ("7&paths&0&0002", "paths&0002"),
            ("7&paths&0&0002", "7&paths\\bad&0&0002"),
            ("&0002#", "&0102#"), ("&0002#", "&000g#"),
            ("001111000030}", "001111000031}"),
        ] {
            let mut wrong = valid.clone();
            wrong.path = wrong.path.replace(from, to);
            assert!(!control(&wrong), "{}", wrong.path);
        }
        let mut wrong = valid.clone();
        wrong.path.push('\0');
        assert!(!control(&wrong));
        let mut uppercase = valid.clone();
        uppercase.path = uppercase.path.to_ascii_uppercase();
        assert!(control(&uppercase));
        assert_eq!(receiver_group(&valid), receiver_group(&uppercase));
    }

    #[test]
    fn descriptor_gate_requests_only_the_documented_input_length_and_id() {
        let interface = interface("descriptor");
        for accepted in [true, false] {
            let called = Cell::new(false);
            assert_eq!(descriptor_gate(&interface, |length, ids| {
                called.set(true);
                assert_eq!(length, 5);
                assert_eq!(ids, [3]);
                accepted
            }), accepted);
            assert!(called.get());
        }
        let mut wrong = interface;
        wrong.input_len = 64;
        assert!(!descriptor_gate(&wrong, |_, _| panic!("wrong collection")));
    }

    #[test]
    fn lightweight_discovery_needs_descriptor_proof_but_never_a_battery_frame() {
        let interface = interface("discovery");
        assert!(discover_with(std::slice::from_ref(&interface), |_| false).is_empty());
        let mut proofs = 0;
        let found = discover_with(std::slice::from_ref(&interface), |candidate| {
            proofs += 1;
            assert_eq!(candidate.path, interface.path);
            true
        });
        assert_eq!(proofs, 1);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].key, target(&interface).key);
        assert!(!found[0].verified_identity);
        assert!(read_with(&found[0], &[interface], |_| false,
            |_| panic!("descriptor denial must not open input")).is_err());
    }

    #[test]
    fn duplicate_battery_collections_are_ambiguous_even_if_one_has_the_wrong_length() {
        let first = interface("ambiguous");
        let mut second = first.clone();
        second.path = second.path.replace("col03", "col04").replace("&0002#", "&0003#");
        second.input_len = 64;
        assert_eq!(receiver_group(&first), receiver_group(&second));
        assert!(discover_with(&[first.clone(), second.clone()], |_| panic!("ambiguous")).is_empty());
        assert!(read_with(&target(&first), &[first.clone(), second], |_| panic!("ambiguous"),
            |_| panic!("ambiguous")).is_err());
        let mut alias = first.clone();
        alias.path = alias.path.to_ascii_uppercase();
        assert!(candidates(&[first, alias]).is_empty());
    }

    #[test]
    fn ordinary_sibling_collections_do_not_hide_the_battery_interface() {
        let battery_interface = interface("siblings");
        let mut system = battery_interface.clone();
        system.path = system.path.replace("col03", "col01").replace("&0002#", "&0000#");
        system.usage_page = 1;
        system.usage = 0x80;
        system.input_len = 2;
        let mut consumer = battery_interface.clone();
        consumer.path = consumer.path.replace("col03", "col02").replace("&0002#", "&0001#");
        consumer.usage_page = 0x0c;
        consumer.input_len = 3;
        let mut output = battery_interface.clone();
        output.path = output.path.replace("col03", "col04").replace("&0002#", "&0003#");
        output.usage_page = 0xff01;
        output.usage = 2;
        output.input_len = 64;
        output.output_len = 64;
        let found = discover_with(&[system, consumer, output, battery_interface.clone()], |candidate| {
            assert_eq!(candidate.path, battery_interface.path);
            true
        });
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].key, target(&battery_interface).key);
    }

    #[test]
    fn stale_path_changed_serial_and_invalid_target_cannot_open_a_neighbor() {
        let interface = interface("stale");
        let original = target(&interface);
        assert!(read_with(&original, &[], |_| panic!("removed"), |_| panic!("removed")).is_err());
        for mutate in [
            |interface: &mut HidInterface| interface.path = interface.path.replace("stale", "other-port"),
            |interface: &mut HidInterface| interface.serial = "replacement receiver".into(),
            |interface: &mut HidInterface| interface.input_len = 64,
            |interface: &mut HidInterface| interface.usage_page = 0xff01,
        ] {
            let mut changed = interface.clone();
            mutate(&mut changed);
            assert!(read_with(&original, &[changed], |_| panic!("stale target"),
                |_| panic!("must rediscover")).is_err());
        }
        for mutate in [
            |target: &mut Target| target.backend = BackendId::StandardHid,
            |target: &mut Target| target.address = 1,
            |target: &mut Target| target.key = "another device".into(),
            |target: &mut Target| target.endpoint = Endpoint::XInput(0),
        ] {
            let mut wrong = original.clone();
            mutate(&mut wrong);
            assert!(read_with(&wrong, std::slice::from_ref(&interface), |_| panic!("invalid target"),
                |_| panic!("invalid target")).is_err());
        }
    }

    #[test]
    fn unplug_during_receive_cannot_publish_a_connected_sample() {
        let interface = interface("unplug");
        let proofs = Cell::new(0);
        assert!(read_with(&target(&interface), &[interface], |_| {
            proofs.set(proofs.get() + 1);
            proofs.get() == 1
        }, |_| Ok(Some(DISCHARGING.to_vec()))).is_err());
        assert_eq!(proofs.get(), 2);
    }

    #[test]
    fn failing_reads_never_renew_the_sample_and_silence_expires_without_fake_disconnect() {
        assert_eq!(BackendId::Inphic.sample_lifetime(), SAMPLE_LIFETIME);
        let interface = interface("expiry");
        let first = read_frame(&interface, &CHARGING);
        let measured = first.battery_at.unwrap();
        let mut state = status::Status::default();
        state.observe_sample(Some(true), first.battery.as_ref(), first.battery_at, measured, SAMPLE_LIFETIME);
        let mut invalid = DISCHARGING;
        invalid[4] = 255;
        for (index, response) in [Ok(None), Ok(Some(invalid.to_vec())),
            Ok(Some(vec![0; 5])), Err("input unavailable".into())].into_iter().enumerate() {
            assert!(read_with(&first.target, std::slice::from_ref(&interface), |_| true,
                |_| response).is_err());
            let now = measured + Duration::from_secs((index + 1) as u64);
            state.observe_sample(None, None, None, now, SAMPLE_LIFETIME);
            assert_eq!(state.measured_at, Some(measured));
            assert_eq!(state.battery.as_ref().unwrap().percent, Some(84));
            assert_eq!(state.quality, ReadingQuality::Cached);
            assert!(state.connected && !state.connection_known);
        }
        assert!(!state.expire(measured + SAMPLE_LIFETIME - Duration::from_millis(1), SAMPLE_LIFETIME));
        assert!(state.expire(measured + SAMPLE_LIFETIME, SAMPLE_LIFETIME));
        assert_eq!(state.measured_at, Some(measured));
        assert_eq!(state.quality, ReadingQuality::Stale);
        assert!(state.connected);
        let woke = measured + SAMPLE_LIFETIME + Duration::from_secs(1);
        let fresh = battery(&DISCHARGING).unwrap();
        state.observe_sample(Some(true), Some(&fresh), Some(woke), woke, SAMPLE_LIFETIME);
        assert_eq!(state.quality, ReadingQuality::Live);
        assert_eq!(state.battery.as_ref().unwrap().percent, Some(85));
        assert_eq!(state.battery.as_ref().unwrap().charging, Some(false));
        assert_eq!(state.measured_at, Some(woke));
    }

    #[test]
    fn unknown_charge_replaces_old_charging_instead_of_reusing_it() {
        let mut state = status::Status::default();
        let now = Instant::now();
        state.observe(Some(true), battery(&CHARGING).as_ref(), now, SAMPLE_LIFETIME);
        let unknown = battery(&[REPORT_ID, 0x76, 0x41, 2, 84]).unwrap();
        state.observe(Some(true), Some(&unknown), now + Duration::from_secs(1), SAMPLE_LIFETIME);
        assert_eq!(state.battery.as_ref().unwrap().percent, Some(84));
        assert_eq!(state.battery.as_ref().unwrap().charging, None);
        assert_eq!(state.quality, ReadingQuality::Live);
    }

    #[test]
    fn receive_skips_noise_uses_a_single_deadline_and_stops_at_the_first_sample() {
        let mut remaining = [5_000, 4_000, 1_000].into_iter();
        let mut reports = [vec![4, 0x76, 0x41, 1, 85], vec![3, 0x76, 0x41, 1, 255],
            CHARGING.to_vec(), DISCHARGING.to_vec()].into_iter();
        let mut timeouts = Vec::new();
        let result = receive_with(|| remaining.next().map(Duration::from_millis), |timeout| {
            timeouts.push(timeout);
            reports.next()
        });
        assert_eq!(result, Some(CHARGING.to_vec()));
        assert_eq!(reports.next(), Some(DISCHARGING.to_vec()));
        assert_eq!(timeouts, [5_000, 4_000, 1_000]);
    }

    #[test]
    fn silence_noise_and_expired_deadlines_have_finite_read_budgets() {
        let mut reads = 0;
        assert!(receive_with(|| Some(Duration::from_secs(5)), |timeout| {
            reads += 1;
            assert_eq!(timeout, TIMEOUT_MS);
            None
        }).is_none());
        assert_eq!(reads, 1);
        reads = 0;
        let result = receive_with(|| Some(Duration::from_secs(5)), |_| {
            reads += 1;
            Some(vec![0; 5])
        });
        assert_eq!(reads, MAX_FRAMES);
        assert!(result.as_deref().and_then(battery).is_none());
        for remaining in [None, Some(Duration::ZERO)] {
            assert!(receive_with(|| remaining, |_| panic!("expired deadline")).is_none());
        }
        let mut budgets = [Some(Duration::from_micros(1)), None].into_iter();
        reads = 0;
        let result = receive_with(|| budgets.next().flatten(), |timeout| {
            reads += 1;
            assert_eq!(timeout, 1);
            Some(vec![0; 5])
        });
        assert_eq!(reads, 1);
        assert!(result.as_deref().and_then(battery).is_none());
    }

    #[test]
    fn invalid_input_does_not_trigger_extra_reads_after_silence_or_pollute_the_next_poll() {
        let mut inputs = [Some(vec![0; 5]), None, Some(DISCHARGING.to_vec())].into_iter();
        let result = receive_with(|| Some(Duration::from_secs(5)), |_| inputs.next().unwrap());
        assert_eq!(result, Some(vec![0; 5]));
        assert_eq!(inputs.next(), Some(Some(DISCHARGING.to_vec())));
        assert_eq!(receive_with(|| Some(Duration::from_secs(5)), |_| Some(CHARGING.to_vec())),
            Some(CHARGING.to_vec()));
    }

    #[test]
    fn identical_serial_receivers_keep_independent_targets_locks_and_battery_values() {
        let first = interface("isolation-first");
        let second = interface("isolation-second");
        let interfaces = [first.clone(), second.clone()];
        let found = discover_with(&interfaces, |_| true);
        assert_eq!(found.len(), 2);
        assert_ne!(found[0].key, found[1].key);
        assert!(!found[0].same_channel(&found[1]));
        let first_lock = channel::lock(&receiver_group(&first).unwrap());
        let second_lock = channel::lock(&receiver_group(&second).unwrap());
        assert!(!Arc::ptr_eq(&first_lock, &second_lock));
        let guard = first_lock.lock().unwrap();
        assert!(read_with(&target(&first), &interfaces, |_| panic!("busy"), |_| panic!("busy")).is_err());
        let second_sample = read_with(&target(&second), &interfaces, |_| true, |selected| {
            assert_eq!(selected.path, second.path);
            Ok(Some(CHARGING.to_vec()))
        }).unwrap();
        assert_eq!(second_sample.target.key, target(&second).key);
        assert_eq!(second_sample.battery.as_ref().unwrap().percent, Some(84));
        assert_eq!(second_sample.battery.as_ref().unwrap().charging, Some(true));
        drop(guard);
        let first_sample = read_with(&target(&first), &interfaces, |_| true, |selected| {
            assert_eq!(selected.path, first.path);
            Ok(Some(DISCHARGING.to_vec()))
        }).unwrap();
        assert_eq!(first_sample.target.key, target(&first).key);
        assert_eq!(first_sample.battery.as_ref().unwrap().percent, Some(85));
        assert_eq!(first_sample.battery.as_ref().unwrap().charging, Some(false));
        assert!(read_with(&target(&first), &interfaces, |_| true, |_| Err("unplugged".into())).is_err());
        let second_again = read_frame(&second, &CHARGING);
        assert_eq!(second_again.target.key, second_sample.target.key);
        assert_eq!(second_again.battery.as_ref().unwrap().percent, Some(84));
    }

    #[test]
    fn case_insensitive_lock_covers_both_descriptor_checks_and_receive() {
        let interface = interface("lock-scope");
        let mut uppercase = interface.clone();
        uppercase.path = uppercase.path.to_ascii_uppercase();
        let lock = channel::lock(&receiver_group(&uppercase).unwrap());
        assert!(Arc::ptr_eq(&lock, &channel::lock(&receiver_group(&interface).unwrap())));
        let proofs = Cell::new(0);
        let sample = read_with(&target(&uppercase), std::slice::from_ref(&interface), |_| {
            assert!(matches!(lock.try_lock(), Err(TryLockError::WouldBlock)));
            proofs.set(proofs.get() + 1);
            true
        }, |_| {
            assert!(matches!(lock.try_lock(), Err(TryLockError::WouldBlock)));
            Ok(Some(DISCHARGING.to_vec()))
        }).unwrap();
        assert_eq!(proofs.get(), 2);
        assert_eq!(sample.target.key, target(&interface).key);
        assert!(lock.try_lock().is_ok());
        assert!(read_with(&sample.target, &[interface], |_| true, |_| Err("read failed".into())).is_err());
        assert!(lock.try_lock().is_ok());
    }

    #[test]
    fn unplug_replug_and_port_changes_require_current_discovery() {
        let old = interface("old-port");
        let mut current = old.clone();
        current.path = current.path.replace("old-port", "new-port");
        let original = target(&old);
        assert!(read_with(&original, std::slice::from_ref(&current), |_| panic!("wrong port"),
            |_| panic!("wrong port")).is_err());
        let found = discover_with(std::slice::from_ref(&current), |_| true);
        assert_eq!(found.len(), 1);
        assert_ne!(found[0].key, original.key);
        let recovered = read_with(&found[0], &[current], |_| true,
            |_| Ok(Some(CHARGING.to_vec()))).unwrap();
        assert_eq!(recovered.battery.unwrap().percent, Some(84));
        assert!(read_with(&original, &[], |_| panic!("unplugged"), |_| panic!("unplugged")).is_err());
        let replugged = read_frame(&old, &DISCHARGING);
        assert_eq!(replugged.target.key, original.key);
        assert_eq!(replugged.battery.unwrap().percent, Some(85));
    }
}
