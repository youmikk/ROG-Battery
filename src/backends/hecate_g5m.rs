//! HECATE G5M Pro 35BB:D00B. Protocol evidence: docs/PROTOCOLS.md.
use super::*;
use crate::hid::HidDevice;
use std::{collections::BTreeMap, sync::{Arc, Mutex, OnceLock, TryLockError}, time::{Duration, Instant}};

pub struct HecateG5m;
const LENGTH: u16 = 64;
const REPORT_ID: u8 = 4;
const HEADER: [u8; 8] = [4, 0, 0, 0x1a, 0, 0, 0, 0];
const QUERY_INTERVAL: Duration = Duration::from_secs(60);
const ROUND_BUDGET: Duration = Duration::from_millis(1500);
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(interface: &HidInterface) -> bool {
    interface.vendor_id == 0x35bb && interface.product_id == 0xd00b
        && interface.usage_page == 0xff1c && interface.usage == 0x0092
}

fn control(interface: &HidInterface) -> bool {
    management(interface) && interface.input_len == LENGTH && interface.output_len == LENGTH
        && interface.feature_len == 0
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|interface| control(interface)
        && interfaces.iter().filter(|other| management(other)
            && channel::collection_group(interface) == channel::collection_group(other)).count() == 1).collect()
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    control(interface) && descriptor::Descriptor::open(api, interface)
        .is_some_and(|descriptor| descriptor.only_input_output_report(interface, REPORT_ID, LENGTH))
}

fn target(interface: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::HecateG5m, interface, 0);
    target.name = "HECATE G5M Pro".into();
    target.family = DeviceFamily::Mouse;
    target
}

fn valid_target(original: &Target) -> bool {
    original.backend == BackendId::HecateG5m && original.address == 0
        && original.hid().is_ok_and(|interface| control(interface) && original.key == target(interface).key)
}

fn discover_with(interfaces: &[HidInterface], mut prove: impl FnMut(&HidInterface) -> bool) -> Vec<Target> {
    candidates(interfaces).into_iter().filter(|interface| prove(interface)).map(target).collect()
}

fn request() -> [u8; LENGTH as usize] {
    let mut packet = [0; LENGTH as usize];
    packet[..8].copy_from_slice(&HEADER);
    packet
}

fn battery(response: &[u8]) -> Option<Battery> {
    if response.len() != usize::from(LENGTH) || response[..8] != HEADER
        || !(1..=100).contains(&response[8]) { return None; }
    Some(Battery { percent: Some(response[8]), estimated_percent: None, approximate: false,
        charging: match response[9] {
            0 => Some(false), 1 => Some(true), 2 if response[8] == 100 => Some(false), _ => None,
        }, voltage_mv: None, level: None })
}

fn exchange_with(exchange: impl FnOnce(&mut dyn FnMut(&[u8]) -> bool) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    let mut invalid = None;
    let response = exchange(&mut |response| {
        if battery(response).is_some() { return true; }
        invalid = Some(response.to_vec());
        false
    });
    response.or(invalid)
}

fn query_with(interface: &HidInterface, deadline: Instant, mut now: impl FnMut() -> Instant,
    exchange: impl FnOnce(&[u8], u32) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    let started = now();
    let timeout = deadline.checked_duration_since(started).ok_or(UNAVAILABLE)?.as_millis().min(1200) as u32;
    if timeout == 0 { return Err(UNAVAILABLE.into()); }
    let response = exchange(&request(), timeout);
    let measured = now();
    let sample = response.as_deref().and_then(battery).filter(|_| measured < deadline);
    trace::record(interface, BackendId::HecateG5m, "query-battery", u16::from(REPORT_ID), 0x1a,
        usize::from(LENGTH), response.as_ref().map_or(0, Vec::len), started,
        if sample.is_some() { trace::Outcome::Ok } else if response.is_some() { trace::Outcome::InvalidData }
        else { trace::Outcome::NoReply });
    Ok(Snapshot { target: target(interface), battery: Some(sample.ok_or(UNAVAILABLE)?),
        battery_at: Some(measured), connected: true, battery_unavailable: false,
        dpi: Dpi::default(), polling_rate: Default::default() })
}

#[derive(Default)]
struct Schedule {
    next_attempt: Option<Instant>,
    cached: Option<Snapshot>,
}

#[derive(Default)]
struct Schedules(BTreeMap<String, (String, Arc<Mutex<Schedule>>)>);

impl Schedules {
    fn for_interface(&mut self, interface: &HidInterface) -> Arc<Mutex<Schedule>> {
        let entry = self.0.entry(interface.path.to_ascii_lowercase())
            .or_insert_with(|| (interface.serial.clone(), Default::default()));
        if entry.0 != interface.serial { *entry = (interface.serial.clone(), Default::default()); }
        entry.1.clone()
    }

    fn retain_present(&mut self, interfaces: &[HidInterface]) {
        let present = candidates(interfaces);
        self.0.retain(|path, (serial, _)| present.iter().any(|interface|
            interface.path.eq_ignore_ascii_case(path) && interface.serial == *serial));
    }
}

fn schedules() -> &'static Mutex<Schedules> {
    static SCHEDULES: OnceLock<Mutex<Schedules>> = OnceLock::new();
    SCHEDULES.get_or_init(Default::default)
}

fn read_with(original: &Target, interfaces: &[HidInterface], deadline: Instant,
    mut now: impl FnMut() -> Instant, mut prove: impl FnMut(&HidInterface) -> bool,
    query: impl FnOnce(&HidInterface, Instant) -> Result<Snapshot, String>) -> Result<Snapshot, String> {
    if !valid_target(original) { return Err(UNAVAILABLE.into()); }
    let original_interface = original.hid()?;
    let interface = candidates(interfaces).into_iter().find(|interface|
        interface.path.eq_ignore_ascii_case(&original_interface.path)
            && interface.serial == original_interface.serial).ok_or(UNAVAILABLE)?;
    let lock = channel::lock(&channel::collection_group(interface));
    let _guard = match lock.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return Err(UNAVAILABLE.into()),
    };
    if now() >= deadline || !prove(interface) || now() >= deadline { return Err(UNAVAILABLE.into()); }
    let schedule = schedules().lock().unwrap_or_else(|p| p.into_inner()).for_interface(interface);
    let mut schedule = match schedule.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return Err(UNAVAILABLE.into()),
    };
    let started = now();
    if schedule.next_attempt.is_some_and(|next| started < next) {
        return schedule.cached.clone().filter(|snapshot| snapshot.battery_at.is_some_and(|at|
            started.checked_duration_since(at).is_some_and(|age| age < BackendId::HecateG5m.sample_lifetime())))
            .ok_or_else(|| UNAVAILABLE.into());
    }
    // Reserve before opening the device, including failed opens and unanswered queries.
    schedule.next_attempt = Some(started + QUERY_INTERVAL);
    let result = query(interface, deadline).and_then(|snapshot| {
        if !prove(interface) || now() >= deadline { return Err(UNAVAILABLE.into()); }
        Ok(snapshot)
    });
    schedule.cached = result.as_ref().ok().cloned();
    result
}

impl DeviceBackend for HecateG5m {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        schedules().lock().unwrap_or_else(|p| p.into_inner()).retain_present(interfaces);
        Ok(discover_with(interfaces, |interface| proven(api, interface)))
    }

    fn read(&self, api: &HidApi, original: &Target, _: bool) -> Result<Snapshot, String> {
        let deadline = Instant::now() + ROUND_BUDGET;
        if !valid_target(original) { return Err(UNAVAILABLE.into()); }
        let interfaces = enumeration::scan(api)?.interfaces;
        read_with(original, &interfaces, deadline, Instant::now, |interface| proven(api, interface), |interface, deadline| {
            let mut device = HidDevice::open(interface)?;
            if !device.writable || device.input_len() != LENGTH || device.output_len() != LENGTH {
                return Err(UNAVAILABLE.into());
            }
            query_with(interface, deadline, Instant::now, |packet, timeout|
                exchange_with(|accept| device.transact(packet, timeout, accept)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn interface(instance: &str) -> HidInterface {
        // Synthetic path: upstream records the usage/capacities, not a collection index.
        HidInterface {
            path: format!(r"\\?\hid#vid_35bb&pid_d00b&mi_01&col05#7&{instance}&0&0004#{{4d1e55b2-f16f-11cf-88cb-001111000030}}"),
            vendor_id: 0x35bb, product_id: 0xd00b, usage_page: 0xff1c, usage: 0x0092,
            input_len: LENGTH, output_len: LENGTH, feature_len: 0,
            product: "HECATE Wireless Receiver".into(), serial: "shared".into(),
        }
    }

    // Reconstructed documented prefix plus zero padding, not a complete raw capture.
    fn frame(percent: u8, charging: u8) -> Vec<u8> {
        let mut frame = request().to_vec();
        frame[8] = percent;
        frame[9] = charging;
        frame
    }

    fn heartbeat() -> Vec<u8> {
        let mut frame = vec![0; 64];
        frame[..9].copy_from_slice(&[4, 0xaa, 0, 0xaa, 0, 0, 0, 0, 0xff]);
        frame
    }

    fn sample(interface: &HidInterface, at: Instant, percent: u8) -> Snapshot {
        query_with(interface, at + ROUND_BUDGET, || at, |_, _| Some(frame(percent, 0))).unwrap()
    }

    #[test]
    fn fixed_query_and_documented_charge_transitions() {
        let packet = request();
        assert_eq!(&packet[..4], &[4, 0, 0, 0x1a]);
        assert!(packet[4..].iter().all(|byte| *byte == 0));
        for (percent, flag, charging) in [(20, 1, true), (21, 1, true), (22, 1, true),
            (31, 1, true), (31, 0, false), (100, 2, false)] {
            let sample = battery(&frame(percent, flag)).unwrap();
            assert_eq!((sample.percent, sample.charging), (Some(percent), Some(charging)));
            assert!(!sample.approximate);
            assert_eq!((sample.estimated_percent, sample.voltage_mv, sample.level), (None, None, None));
        }
    }

    #[test]
    fn unknown_or_inconsistent_charge_flags_do_not_invent_state() {
        for percent in [1, 50, 99, 100] {
            for flag in 3..=255 { assert_eq!(battery(&frame(percent, flag)).unwrap().charging, None); }
            assert_eq!(battery(&frame(percent, 1)).unwrap().charging, Some(true));
        }
        for percent in [1, 50, 99] {
            let sample = battery(&frame(percent, 2)).unwrap();
            assert_eq!(sample.percent, Some(percent));
            assert_eq!(sample.charging, None);
        }
    }

    #[test]
    fn zero_echo_heartbeat_and_out_of_range_never_become_measurements() {
        assert!(battery(&request()).is_none());
        assert!(battery(&heartbeat()).is_none());
        for percent in std::iter::once(0).chain(101..=255) {
            for flag in [0, 1, 2, 255] { assert!(battery(&frame(percent, flag)).is_none()); }
        }
        for percent in [1, 100] { assert_eq!(battery(&frame(percent, 0)).unwrap().percent, Some(percent)); }
    }

    #[test]
    fn full_header_and_exact_length_are_required() {
        for index in 0..8 {
            let mut wrong = frame(22, 1);
            wrong[index] ^= 1;
            assert!(battery(&wrong).is_none());
        }
        let valid = frame(22, 1);
        for length in 0..64 { assert!(battery(&valid[..length]).is_none()); }
        let mut oversized = valid;
        oversized.push(0);
        assert!(battery(&oversized).is_none());
    }

    #[test]
    fn invalid_frames_remain_diagnostic_but_do_not_hide_a_valid_response() {
        assert!(exchange_with(|_| None).is_none());
        let invalid = exchange_with(|accept| {
            assert!(!accept(&heartbeat()));
            assert!(!accept(&request()));
            None
        }).unwrap();
        assert_eq!(invalid, request());
        assert!(battery(&invalid).is_none());
        let valid = exchange_with(|accept| {
            assert!(!accept(&heartbeat()));
            assert!(!accept(&request()));
            assert!(accept(&frame(22, 1)));
            Some(frame(22, 1))
        }).unwrap();
        assert_eq!(battery(&valid).unwrap().percent, Some(22));
    }

    #[test]
    fn single_query_has_a_deadline_and_uses_measurement_time() {
        let interface = interface("query");
        let start = Instant::now();
        let clock = Cell::new(start);
        let measured = start + Duration::from_millis(40);
        let sample = query_with(&interface, start + ROUND_BUDGET, || clock.get(), |packet, timeout| {
            assert_eq!(packet, request());
            assert_eq!(timeout, 1200);
            clock.set(measured);
            Some(frame(22, 1))
        }).unwrap();
        assert_eq!(sample.battery_at, Some(measured));
        assert!(sample.connected && !sample.battery_unavailable && !sample.target.verified_identity);
        assert!(!sample.dpi.writable && !sample.dpi.stage_writable && !sample.polling_rate.writable);
        assert!(query_with(&interface, start, || start, |_, _| panic!("expired")).is_err());
        let calls = Cell::new(0);
        assert!(query_with(&interface, start + ROUND_BUDGET, || {
            calls.set(calls.get() + 1);
            if calls.get() == 1 { start } else { start + ROUND_BUDGET }
        }, |_, _| Some(frame(22, 1))).is_err());
    }

    #[test]
    fn silence_heartbeat_and_echo_never_supply_connection_or_battery() {
        let interface = interface("no-response");
        let now = Instant::now();
        for response in [None, Some(heartbeat()), Some(request().to_vec()), Some(frame(0, 1))] {
            assert!(query_with(&interface, now + ROUND_BUDGET, || now, |_, _| response).is_err());
        }
    }

    #[test]
    fn discovery_is_metadata_only_and_narrow() {
        let first = interface("discovery-a");
        let second = interface("discovery-b");
        let targets = discover_with(&[first, second], |_| true);
        assert_eq!(targets.len(), 2);
        assert_ne!(targets[0].key, targets[1].key);
        assert!(targets.iter().all(|target| target.family == DeviceFamily::Mouse && !target.verified_identity));
        assert!(discover_with(&[interface("unproven")], |_| false).is_empty());
        assert!(!super::super::has_configuration(BackendId::HecateG5m));
        for change in [
            |i: &mut HidInterface| i.vendor_id = 0x320f,
            |i: &mut HidInterface| i.product_id = 0xd3fb,
            |i: &mut HidInterface| i.product_id = 0x706e,
            |i: &mut HidInterface| i.usage_page = 0xff00,
            |i: &mut HidInterface| i.usage = 2,
            |i: &mut HidInterface| i.input_len = 65,
            |i: &mut HidInterface| i.output_len = 65,
            |i: &mut HidInterface| i.feature_len = 8,
        ] {
            let mut wrong = interface("wrong-scope");
            change(&mut wrong);
            assert!(discover_with(&[wrong], |_| panic!("outside scope")).is_empty());
        }
    }

    #[test]
    fn ambiguous_management_collections_are_rejected_but_standard_siblings_are_allowed() {
        let first = interface("ambiguous");
        let mut sibling = first.clone();
        sibling.path = sibling.path.replace("col05", "col06").replace("&0004#", "&0005#");
        assert!(discover_with(&[first.clone(), sibling.clone()], |_| true).is_empty());
        assert!(discover_with(&[first.clone(), first.clone()], |_| true).is_empty());
        sibling.usage_page = 1;
        sibling.usage = 6;
        sibling.input_len = 21;
        sibling.output_len = 2;
        assert_eq!(discover_with(&[first, sibling], |_| true).len(), 1);
    }

    #[test]
    fn removed_or_changed_target_never_falls_back_to_a_neighbor() {
        let original = interface("removed");
        let remaining = interface("remaining");
        let now = Instant::now();
        assert!(read_with(&target(&original), std::slice::from_ref(&remaining), now + ROUND_BUDGET,
            || now, |_| true, |_, _| panic!("neighbor")).is_err());
        let mut replaced = original.clone();
        replaced.serial = "replacement".into();
        assert!(read_with(&target(&original), &[replaced], now + ROUND_BUDGET,
            || now, |_| true, |_, _| panic!("replaced")).is_err());
        for change in [
            |t: &mut Target| t.backend = BackendId::G3m,
            |t: &mut Target| t.address = 1,
            |t: &mut Target| t.key = "wrong-key".into(),
        ] {
            let mut wrong = target(&remaining);
            change(&mut wrong);
            assert!(read_with(&wrong, std::slice::from_ref(&remaining), now + ROUND_BUDGET,
                || now, |_| true, |_, _| panic!("invalid target")).is_err());
        }
    }

    #[test]
    fn descriptor_loss_busy_channel_and_overdue_round_prevent_publication() {
        let interface = interface("descriptor-lock");
        let now = Instant::now();
        let original = target(&interface);
        let interfaces = std::slice::from_ref(&interface);
        assert!(read_with(&original, interfaces, now + ROUND_BUDGET, || now,
            |_| false, |_, _| panic!("unproven")).is_err());
        let calls = Cell::new(0);
        assert!(read_with(&original, interfaces, now + ROUND_BUDGET, || now, |_| {
            calls.set(calls.get() + 1);
            calls.get() == 1
        }, |i, _| Ok(sample(i, now, 22))).is_err());
        assert_eq!(calls.get(), 2);
        let next = now + QUERY_INTERVAL;
        let clock = Cell::new(next);
        assert!(read_with(&original, interfaces, next + ROUND_BUDGET, || clock.get(), |_| true, |i, _| {
            clock.set(next + ROUND_BUDGET);
            Ok(sample(i, next, 22))
        }).is_err());
        let lock = channel::lock(&channel::collection_group(&interface));
        let _guard = lock.lock().unwrap();
        assert!(read_with(&original, interfaces, next + ROUND_BUDGET, || next,
            |_| true, |_, _| panic!("busy")).is_err());
    }

    #[test]
    fn query_limit_keeps_cached_timestamp_and_failed_query_cannot_republish_it() {
        let interface = interface("limited");
        let original = target(&interface);
        let interfaces = std::slice::from_ref(&interface);
        let now = Instant::now();
        let first = read_with(&original, interfaces, now + ROUND_BUDGET, || now, |_| true,
            |i, _| Ok(sample(i, now, 22))).unwrap();
        for seconds in [1, 10, 59] {
            let later = now + Duration::from_secs(seconds);
            let cached = read_with(&original, interfaces, later + ROUND_BUDGET, || later, |_| true,
                |_, _| panic!("query too soon")).unwrap();
            assert_eq!(cached.battery_at, first.battery_at);
        }
        let failed = now + QUERY_INTERVAL;
        assert!(read_with(&original, interfaces, failed + ROUND_BUDGET, || failed, |_| true,
            |_, _| Err(UNAVAILABLE.into())).is_err());
        let retry = failed + Duration::from_secs(10);
        assert!(read_with(&original, interfaces, retry + ROUND_BUDGET, || retry, |_| true,
            |_, _| panic!("failed query still limited")).is_err());
        let woke = failed + QUERY_INTERVAL;
        let recovered = read_with(&original, interfaces, woke + ROUND_BUDGET, || woke, |_| true,
            |i, _| Ok(sample(i, woke, 21))).unwrap();
        assert_eq!(recovered.battery_at, Some(woke));
        assert_eq!(recovered.battery.unwrap().percent, Some(21));
    }

    #[test]
    fn receiver_schedules_reset_on_removal_replacement_and_ambiguity() {
        let first = interface("schedule-a");
        let second = interface("schedule-b");
        let mut schedules = Schedules::default();
        let old = schedules.for_interface(&first);
        let neighbor = schedules.for_interface(&second);
        let now = Instant::now();
        old.lock().unwrap().next_attempt = Some(now + QUERY_INTERVAL);
        assert!(neighbor.lock().unwrap().next_attempt.is_none());
        schedules.retain_present(std::slice::from_ref(&second));
        assert_eq!(schedules.0.len(), 1);
        assert!(!Arc::ptr_eq(&old, &schedules.for_interface(&first)));
        let mut replaced = second.clone();
        replaced.serial = "replacement".into();
        assert!(!Arc::ptr_eq(&neighbor, &schedules.for_interface(&replaced)));
        schedules.retain_present(&[first.clone(), first]);
        assert!(schedules.0.is_empty());
    }

    #[test]
    fn busy_receiver_does_not_block_its_identical_neighbor() {
        let first = interface("isolated-a");
        let second = interface("isolated-b");
        let interfaces = [first.clone(), second.clone()];
        let now = Instant::now();
        let lock = channel::lock(&channel::collection_group(&first));
        let _guard = lock.lock().unwrap();
        assert!(read_with(&target(&first), &interfaces, now + ROUND_BUDGET, || now,
            |_| true, |_, _| panic!("busy")).is_err());
        let sample = read_with(&target(&second), &interfaces, now + ROUND_BUDGET, || now,
            |_| true, |i, _| Ok(sample(i, now, 80))).unwrap();
        assert_eq!(sample.target.key, target(&second).key);
        assert_ne!(sample.target.key, target(&first).key);
        assert_eq!(sample.battery.unwrap().percent, Some(80));
    }
}
