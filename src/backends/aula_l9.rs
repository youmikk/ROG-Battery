//! AULA L9 Ultra: documented status81 query only; evidence: docs/PROTOCOLS.md.
use super::*;
use crate::hid::HidDevice;
use std::{collections::BTreeMap, sync::{Arc, Mutex, OnceLock, TryLockError}, time::{Duration, Instant}};

pub struct AulaL9;
const REPORT_ID: u8 = 0x0b;
const QUERY_INTERVAL: Duration = Duration::from_secs(60);
const ROUND_BUDGET: Duration = Duration::from_millis(1500);
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(interface: &HidInterface) -> bool {
    interface.vendor_id == 0x38a6 && interface.product_id == 0x28fd
        && interface.usage_page == 0xffa8
}

fn control(interface: &HidInterface) -> bool {
    management(interface) && (19..=64).contains(&interface.input_len)
        && (3..=64).contains(&interface.output_len)
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|interface| control(interface)
        && interfaces.iter().filter(|other| management(other)
            && channel::collection_group(interface) == channel::collection_group(other)).count() == 1).collect()
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    control(interface) && descriptor::Descriptor::open(api, interface)
        .is_some_and(|descriptor| descriptor.only_reports(interface, 0x0a, interface.input_len, REPORT_ID, interface.output_len))
}

fn target(interface: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::AulaL9, interface, 0);
    target.name = "AULA L9 Ultra".into();
    target.family = DeviceFamily::Mouse;
    target
}

fn valid_target(original: &Target) -> bool {
    original.backend == BackendId::AulaL9 && original.address == 0
        && original.hid().is_ok_and(|interface| control(interface) && original.key == target(interface).key)
}

fn discover_with(interfaces: &[HidInterface], mut prove: impl FnMut(&HidInterface) -> bool) -> Vec<Target> {
    candidates(interfaces).into_iter().filter(|interface| prove(interface)).map(target).collect()
}

fn request(interface: &HidInterface) -> Vec<u8> {
    let mut packet = vec![0; usize::from(interface.output_len)];
    packet[..2].copy_from_slice(&[REPORT_ID, 0x81]);
    packet
}

fn battery(interface: &HidInterface, response: &[u8]) -> Option<Battery> {
    if response.len() != usize::from(interface.input_len) || response.len() < 19
        || response[..2] != [0x0a, 0x81] || response[18] > 100 { return None; }
    Some(Battery { percent: Some(response[18]), estimated_percent: None, approximate: false,
        charging: match response[17] { 0 => Some(false), 1 => Some(true), _ => None },
        voltage_mv: None, level: None })
}

fn exchange_with(interface: &HidInterface,
    exchange: impl FnOnce(&mut dyn FnMut(&[u8]) -> bool) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    let mut invalid = None;
    let response = exchange(&mut |response| {
        if battery(interface, response).is_some() { return true; }
        invalid = Some(response.to_vec()); false
    });
    response.or(invalid)
}

fn query_with(interface: &HidInterface, deadline: Instant, mut now: impl FnMut() -> Instant,
    exchange: impl FnOnce(&[u8], u32) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    let started = now();
    let timeout = deadline.checked_duration_since(started).ok_or(UNAVAILABLE)?.as_millis().min(1200) as u32;
    if timeout == 0 { return Err(UNAVAILABLE.into()); }
    let packet = request(interface);
    let response = exchange(&packet, timeout);
    let measured = now();
    let sample = response.as_deref().and_then(|frame| battery(interface, frame)).filter(|_| measured < deadline);
    trace::record(interface, BackendId::AulaL9, "query-battery", u16::from(REPORT_ID), 0x81,
        packet.len(), response.as_ref().map_or(0, Vec::len), started,
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
            started.checked_duration_since(at).is_some_and(|age| age < BackendId::AulaL9.sample_lifetime())))
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

impl DeviceBackend for AulaL9 {
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
            if !device.writable || device.input_len() != interface.input_len || device.output_len() != interface.output_len {
                return Err(UNAVAILABLE.into());
            }
            query_with(interface, deadline, Instant::now, |packet, timeout|
                exchange_with(interface, |accept| device.transact(packet, timeout, accept)))
        })
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    fn interface(id: &str) -> HidInterface {
        HidInterface { path: format!("hid#vid_38a6&pid_28fd&mi_01&col01#{id}&0000#hid"),
            vendor_id: 0x38a6, product_id: 0x28fd, usage_page: 0xffa8, usage: 1,
            input_len: 32, output_len: 64, feature_len: 0, product: "L9".into(), serial: "shared".into() }
    }
    fn frame(i: &HidInterface, percent: u8, flag: u8) -> Vec<u8> {
        let mut r = vec![0; usize::from(i.input_len)]; r[0] = 0x0a; r[1] = 0x81; r[17] = flag; r[18] = percent; r
    }
    fn sample(i: &HidInterface, now: Instant) -> Snapshot {
        query_with(i, now + ROUND_BUDGET, || now, |_, _| Some(frame(i, 64, 1))).unwrap()
    }
    #[test]
    fn reconstructed_sample_and_descriptor_sized_transfers() {
        for (input, output) in [(19, 3), (32, 64), (64, 64)] {
            let i = HidInterface { input_len: input, output_len: output, ..interface("sizes") };
            assert!(control(&i)); let q = request(&i);
            assert_eq!(q.len(), usize::from(output)); assert_eq!(&q[..2], &[0x0b, 0x81]);
            assert!(q[2..].iter().all(|b| *b == 0));
            let b = battery(&i, &frame(&i, 64, 1)).unwrap();
            assert_eq!((b.percent, b.charging), (Some(64), Some(true)));
            assert!(battery(&i, &q).is_none());
        }
    }
    #[test]
    fn boundaries_unknown_flags_and_malformed_responses() {
        let i = interface("parser");
        for percent in [0, 1, 100] { assert_eq!(battery(&i, &frame(&i, percent, 0)).unwrap().percent, Some(percent)); }
        for flag in 2..=255 { assert_eq!(battery(&i, &frame(&i, 64, flag)).unwrap().charging, None); }
        for percent in 101..=255 { assert!(battery(&i, &frame(&i, percent, 1)).is_none()); }
        for at in [0, 1] { let mut r = frame(&i, 64, 1); r[at] ^= 1; assert!(battery(&i, &r).is_none()); }
        for len in 0..32 { assert!(battery(&i, &frame(&i, 64, 1)[..len]).is_none()); }
        let mut r = frame(&i, 64, 1); r.push(0); assert!(battery(&i, &r).is_none());
    }
    #[test]
    fn discovery_ignores_other_models_and_ambiguous_collections() {
        let i = interface("scope");
        for change in [|i: &mut HidInterface| i.product_id = 0x0030, |i: &mut HidInterface| i.vendor_id = 0x320f,
            |i: &mut HidInterface| i.usage_page = 0xffab, |i: &mut HidInterface| i.input_len = 18,
            |i: &mut HidInterface| i.output_len = 2, |i: &mut HidInterface| i.output_len = 65] {
            let mut wrong = i.clone(); change(&mut wrong);
            assert!(discover_with(&[wrong], |_| panic!("scope")).is_empty());
        }
        assert!(discover_with(&[i.clone()], |_| false).is_empty());
        assert!(discover_with(&[i.clone(), i.clone()], |_| true).is_empty());
        assert_eq!(discover_with(&[i, interface("other")], |_| true).len(), 2);
        assert!(!super::super::has_configuration(BackendId::AulaL9));
    }
    #[test]
    fn invalid_input_does_not_hide_a_valid_reply_and_timeouts_fail() {
        let i = interface("exchange"); let now = Instant::now();
        let result = exchange_with(&i, |accept| {
            assert!(!accept(&request(&i))); assert!(accept(&frame(&i, 64, 1))); Some(frame(&i, 64, 1))
        }).unwrap(); assert_eq!(battery(&i, &result).unwrap().percent, Some(64));
        assert!(query_with(&i, now + ROUND_BUDGET, || now, |_, _| None).is_err());
        assert!(query_with(&i, now, || now, |_, _| panic!("expired")).is_err());
        let clock = Cell::new(now);
        assert!(query_with(&i, now + ROUND_BUDGET, || clock.get(), |_, _| {
            clock.set(now + ROUND_BUDGET); Some(frame(&i, 64, 1))
        }).is_err());
    }
    #[test]
    fn query_schedule_retains_sample_time_and_retries_after_failure() {
        let i = interface("schedule"); let now = Instant::now(); let original = target(&i); let interfaces = [i.clone()];
        let first = read_with(&original, &interfaces, now + ROUND_BUDGET, || now, |_| true, |i, _| Ok(sample(i, now))).unwrap();
        let later = now + Duration::from_secs(59);
        let cached = read_with(&original, &interfaces, later + ROUND_BUDGET, || later, |_| true, |_, _| panic!("rate limit")).unwrap();
        assert_eq!(cached.battery_at, first.battery_at);
        let failed = now + QUERY_INTERVAL;
        assert!(read_with(&original, &interfaces, failed + ROUND_BUDGET, || failed, |_| true, |_, _| Err("failed".into())).is_err());
        let retry = failed + Duration::from_secs(1);
        assert!(read_with(&original, &interfaces, retry + ROUND_BUDGET, || retry, |_| true, |_, _| panic!("limited")).is_err());
        let recovered = failed + QUERY_INTERVAL;
        assert!(read_with(&original, &interfaces, recovered + ROUND_BUDGET, || recovered, |_| true, |i, _| Ok(sample(i, recovered))).is_ok());
    }
    #[test]
    fn target_descriptor_and_neighbor_isolation() {
        let i = interface("isolation-a"); let j = interface("isolation-b"); let now = Instant::now();
        assert!(read_with(&target(&i), &[j.clone()], now + ROUND_BUDGET, || now, |_| true, |_, _| panic!("removed")).is_err());
        let mut wrong = target(&i); wrong.address = 1;
        assert!(read_with(&wrong, &[i.clone()], now + ROUND_BUDGET, || now, |_| true, |_, _| panic!("invalid")).is_err());
        assert!(read_with(&target(&i), &[i.clone()], now + ROUND_BUDGET, || now, |_| false, |_, _| panic!("descriptor")).is_err());
        let proof = Cell::new(0);
        assert!(read_with(&target(&i), &[i.clone()], now + ROUND_BUDGET, || now, |_| { proof.set(proof.get()+1); proof.get()==1 }, |i, _| Ok(sample(i, now))).is_err());
        let lock = channel::lock(&channel::collection_group(&i)); let _guard = lock.lock().unwrap();
        assert!(read_with(&target(&i), &[i.clone()], now + ROUND_BUDGET, || now, |_| true, |_, _| panic!("busy")).is_err());
        let s = read_with(&target(&j), &[j.clone()], now + ROUND_BUDGET, || now, |_| true, |i, _| Ok(sample(i, now))).unwrap();
        assert_ne!(s.target.key, target(&i).key);
    }
    #[test]
    fn removal_serial_change_and_ambiguity_reset_scheduling() {
        let mut schedules = Schedules::default(); let i = interface("reset"); let old = schedules.for_interface(&i);
        schedules.retain_present(&[]); assert!(!Arc::ptr_eq(&old, &schedules.for_interface(&i)));
        let old = schedules.for_interface(&i); let mut changed = i.clone(); changed.serial = "replacement".into();
        assert!(!Arc::ptr_eq(&old, &schedules.for_interface(&changed)));
        schedules.retain_present(&[i.clone(), i]); assert!(schedules.0.is_empty());
    }
}
