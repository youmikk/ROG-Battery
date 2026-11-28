use super::*;
use super::feature::FeatureChannel;
use std::{collections::BTreeMap, sync::{Arc, Mutex, OnceLock, TryLockError}, time::{Duration, Instant}};

pub struct InphicIn9;

pub(super) const SAMPLE_LIFETIME: Duration = Duration::from_secs(120);
const LENGTH: u16 = 33;
const REPORT_ID: u8 = 0x0c;
const ROUND_BUDGET: Duration = Duration::from_millis(3200);
const IO_TIMEOUT_MS: u32 = 400;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(interface: &HidInterface) -> bool {
    interface.vendor_id == 0x248a && interface.product_id == 0x5b2f
        && interface.usage_page == 0xff00 && interface.usage == 1
}

fn control(interface: &HidInterface) -> bool {
    if !management(interface) || interface.input_len != 0 || interface.output_len != 0
        || interface.feature_len != LENGTH { return false; }
    let path = interface.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    parts.len() == 4 && parts[0] == r"\\?\hid"
        && parts[1] == "vid_248a&pid_5b2f&mi_01&col04"
        && !parts[2].is_empty() && !parts[2].contains(['\\', '/', '\0'])
        && parts[3] == "{4d1e55b2-f16f-11cf-88cb-001111000030}"
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|interface| control(interface)
        && interfaces.iter().filter(|other| management(other) && other.feature_len > 0
            && channel::collection_group(interface) == channel::collection_group(other)).count() == 1).collect()
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    control(interface) && descriptor::Descriptor::open(api, interface).is_some_and(|descriptor|
        descriptor.feature_reports(interface, LENGTH, &[REPORT_ID], &[REPORT_ID]))
}

fn target(interface: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::InphicIn9, interface, 0);
    target.name = "Inphic IN9".into();
    target.family = DeviceFamily::Mouse;
    target
}

fn valid_target(original: &Target) -> bool {
    original.backend == BackendId::InphicIn9 && original.address == 0
        && original.hid().is_ok_and(|interface| control(interface) && original.key == target(interface).key)
}

#[derive(Default)]
struct Schedule {
    sequence: u8,
    failures: u8,
    next_attempt: Option<Instant>,
    cached: Option<Snapshot>,
}

impl Schedule {
    fn next_sequence(&mut self) -> u8 {
        self.sequence = if self.sequence == u8::MAX { 1 } else { self.sequence + 1 };
        self.sequence
    }

    fn begin(&mut self, now: Instant) -> bool {
        if self.next_attempt.is_some_and(|next| now < next) { return false; }
        self.next_attempt = Some(now + Duration::from_secs(10));
        true
    }

    fn finish(&mut self, success: bool, now: Instant) {
        if success { self.failures = 0; return; }
        self.cached = None;
        self.failures = self.failures.saturating_add(1);
        let seconds = (10u64 << self.failures.saturating_sub(1).min(3)).min(60);
        self.next_attempt = Some(now + Duration::from_secs(seconds));
    }
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
        self.0.retain(|path, (serial, _)| interfaces.iter().any(|interface|
            control(interface) && interface.path.eq_ignore_ascii_case(path) && &interface.serial == serial));
    }
}

fn schedules() -> &'static Mutex<Schedules> {
    static SCHEDULES: OnceLock<Mutex<Schedules>> = OnceLock::new();
    SCHEDULES.get_or_init(Default::default)
}

fn request(sequence: u8) -> Option<[u8; LENGTH as usize]> {
    if sequence == 0 { return None; }
    let mut packet = [0; LENGTH as usize];
    packet[..5].copy_from_slice(&[REPORT_ID, 1, 0x20, 0, sequence]);
    Some(packet)
}

fn battery(frame: &[u8]) -> Option<Battery> {
    if frame.len() != usize::from(LENGTH) || frame[..4] != [REPORT_ID, 1, 0x20, 0]
        || &frame[8..13] != b"M9094" { return None; }
    let (percent, charging) = match (frame[19], frame[21]) {
        (1, _) => (None, Some(true)),
        (0, 1) if (1..=100).contains(&frame[18]) => (Some(frame[18]), Some(false)),
        (0, 1) => return None,
        _ => (None, None),
    };
    Some(Battery { percent, charging, estimated_percent: None, voltage_mv: None,
        approximate: false, level: None })
}

trait Transport {
    fn send(&mut self, packet: &[u8], timeout: u32) -> Result<(), String>;
    fn receive(&mut self, timeout: u32) -> Result<Vec<u8>, String>;
    fn now(&self) -> Instant;
    fn wait(&mut self, duration: Duration);
}

impl Transport for FeatureChannel {
    fn send(&mut self, packet: &[u8], timeout: u32) -> Result<(), String> {
        FeatureChannel::send(self, packet, timeout)
    }
    fn receive(&mut self, timeout: u32) -> Result<Vec<u8>, String> {
        self.receive_raw(REPORT_ID, timeout)
    }
    fn now(&self) -> Instant { Instant::now() }
    fn wait(&mut self, duration: Duration) { std::thread::sleep(duration); }
}

fn remaining(deadline: Instant, now: Instant) -> Result<u32, String> {
    let millis = deadline.checked_duration_since(now).unwrap_or_default().as_millis()
        .min(u128::from(IO_TIMEOUT_MS)) as u32;
    if millis == 0 { Err(UNAVAILABLE.into()) } else { Ok(millis) }
}

fn settle(transport: &mut impl Transport, deadline: Instant, millis: u64) -> Result<(), String> {
    let duration = Duration::from_millis(millis);
    if transport.now().checked_add(duration).is_none_or(|end| end >= deadline) {
        return Err(UNAVAILABLE.into());
    }
    transport.wait(duration);
    remaining(deadline, transport.now()).map(|_| ())
}

fn read_sample(interface: &HidInterface, transport: &mut impl Transport, deadline: Instant)
    -> Result<Option<Snapshot>, String> {
    let started = transport.now();
    let response = transport.receive(remaining(deadline, started)?);
    let measured = transport.now();
    let sample = response.as_ref().ok().and_then(|frame| battery(frame))
        .filter(|_| measured < deadline);
    trace::record(interface, BackendId::InphicIn9, "query-battery", u16::from(REPORT_ID), 1,
        usize::from(LENGTH), response.as_ref().map_or(0, Vec::len), started,
        if sample.is_some() { trace::Outcome::Ok } else if response.is_ok() { trace::Outcome::InvalidData }
        else { trace::Outcome::NoReply });
    remaining(deadline, measured)?;
    Ok(sample.map(|battery| Snapshot { target: target(interface),
        battery_unavailable: battery.percent.is_none() && battery.charging.is_none(),
        battery: Some(battery), battery_at: Some(measured), connected: true,
        dpi: Dpi::default(), polling_rate: Default::default() }))
}

fn query_with(interface: &HidInterface, schedule: &mut Schedule,
    transport: &mut impl Transport, deadline: Instant) -> Result<Snapshot, String> {
    for attempt in 0..2 {
        if attempt > 0 { settle(transport, deadline, 500)?; }
        let timeout = remaining(deadline, transport.now())?;
        let packet = request(schedule.next_sequence()).ok_or(UNAVAILABLE)?;
        let started = transport.now();
        let sent = transport.send(&packet, timeout);
        trace::record(interface, BackendId::InphicIn9, "refresh-battery", u16::from(REPORT_ID), 1,
            packet.len(), 0, started, if sent.is_ok() { trace::Outcome::Ok } else { trace::Outcome::NoReply });
        sent?;
        settle(transport, deadline, 30)?;
        if let Some(sample) = read_sample(interface, transport, deadline)? { return Ok(sample); }
    }
    for delay in [80, 160] {
        settle(transport, deadline, delay)?;
        if let Some(sample) = read_sample(interface, transport, deadline)? { return Ok(sample); }
    }
    Err(UNAVAILABLE.into())
}

fn discover_with(interfaces: &[HidInterface], mut prove: impl FnMut(&HidInterface) -> bool) -> Vec<Target> {
    candidates(interfaces).into_iter().filter(|interface| prove(interface)).map(target).collect()
}

fn read_with(original: &Target, interfaces: &[HidInterface], deadline: Instant,
    mut now: impl FnMut() -> Instant, mut prove: impl FnMut(&HidInterface) -> bool,
    query: impl FnOnce(&HidInterface, &mut Schedule, Instant) -> Result<Snapshot, String>) -> Result<Snapshot, String> {
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
    remaining(deadline, now())?;
    if !prove(interface) { return Err(UNAVAILABLE.into()); }
    remaining(deadline, now())?;
    let schedule = schedules().lock().unwrap_or_else(|error| error.into_inner()).for_interface(interface);
    let mut schedule = match schedule.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return Err(UNAVAILABLE.into()),
    };
    if !schedule.begin(now()) { return schedule.cached.clone().ok_or_else(|| UNAVAILABLE.into()); }
    let result = query(interface, &mut schedule, deadline).and_then(|snapshot| {
        remaining(deadline, now())?;
        if !prove(interface) { return Err(UNAVAILABLE.into()); }
        remaining(deadline, now())?;
        Ok(snapshot)
    });
    schedule.cached = result.as_ref().ok().cloned();
    schedule.finish(result.is_ok(), now());
    result
}

impl DeviceBackend for InphicIn9 {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        schedules().lock().unwrap_or_else(|error| error.into_inner()).retain_present(interfaces);
        Ok(discover_with(interfaces, |interface| proven(api, interface)))
    }

    fn read(&self, api: &HidApi, original: &Target, _: bool) -> Result<Snapshot, String> {
        let deadline = Instant::now() + ROUND_BUDGET;
        if !valid_target(original) { return Err(UNAVAILABLE.into()); }
        let interfaces = enumeration::scan(api)?.interfaces;
        read_with(original, &interfaces, deadline, Instant::now, |interface| proven(api, interface),
            |interface, schedule, deadline| {
                let mut transport = FeatureChannel::open(interface)?;
                query_with(interface, schedule, &mut transport, deadline)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, collections::VecDeque};

    const RECORDED_DISCHARGING: &str = "0C012000010110004D3930393401002514FF5100FF010000B7B76238FFC767096D";
    const RECORDED_CHARGING: &str = "0C012000030110004D3930393401002546FF6401FF01000032963238FFC767096D";

    fn frame(hex: &str) -> Vec<u8> {
        hex.as_bytes().chunks_exact(2).map(|pair|
            u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect()
    }

    fn interface(instance: &str) -> HidInterface {
        HidInterface { path: format!(r"\\?\hid#vid_248a&pid_5b2f&mi_01&col04#7&{instance}&0&0003#{{4d1e55b2-f16f-11cf-88cb-001111000030}}"),
            vendor_id: 0x248a, product_id: 0x5b2f, usage_page: 0xff00, usage: 1,
            input_len: 0, output_len: 0, feature_len: LENGTH,
            product: "2.4G Wireless Receiver".into(), serial: "shared".into() }
    }

    struct MockTransport {
        now: Instant,
        replies: VecDeque<Result<Vec<u8>, String>>,
        writes: Vec<Vec<u8>>,
        reads: usize,
        waits: Vec<u64>,
        write_error: Option<usize>,
        read_cost: Duration,
    }

    impl MockTransport {
        fn new(replies: Vec<Vec<u8>>) -> Self {
            Self { now: Instant::now(), replies: replies.into_iter().map(Ok).collect(),
                writes: Vec::new(), reads: 0, waits: Vec::new(), write_error: None, read_cost: Duration::ZERO }
        }
    }

    impl Transport for MockTransport {
        fn send(&mut self, packet: &[u8], timeout: u32) -> Result<(), String> {
            assert!((1..=IO_TIMEOUT_MS).contains(&timeout));
            self.writes.push(packet.to_vec());
            if self.write_error == Some(self.writes.len()) { Err("write failed".into()) } else { Ok(()) }
        }
        fn receive(&mut self, timeout: u32) -> Result<Vec<u8>, String> {
            assert!((1..=IO_TIMEOUT_MS).contains(&timeout));
            self.reads += 1;
            self.now += self.read_cost;
            self.replies.pop_front().unwrap_or_else(|| Err("no reply".into()))
        }
        fn now(&self) -> Instant { self.now }
        fn wait(&mut self, duration: Duration) {
            self.waits.push(duration.as_millis() as u64);
            self.now += duration;
        }
    }

    fn query(transport: &mut MockTransport) -> Result<Snapshot, String> {
        let deadline = transport.now + ROUND_BUDGET;
        query_with(&interface("query"), &mut Schedule::default(), transport, deadline)
    }

    #[test]
    fn recorded_discharge_and_charging_placeholder_have_distinct_meanings() {
        let sample = battery(&frame(RECORDED_DISCHARGING)).unwrap();
        assert_eq!((sample.percent, sample.charging), (Some(81), Some(false)));
        assert_eq!((sample.estimated_percent, sample.voltage_mv, sample.level), (None, None, None));
        assert!(!sample.approximate);
        let mut transport = MockTransport::new(vec![frame(RECORDED_CHARGING)]);
        let sample = query(&mut transport).unwrap();
        assert!(sample.connected && !sample.battery_unavailable && !sample.target.verified_identity);
        assert_eq!(sample.target.key, target(&interface("query")).key);
        let power = sample.battery.unwrap();
        assert_eq!((power.percent, power.charging), (None, Some(true)));
        assert_eq!(sample.battery_at, Some(transport.now));
    }

    #[test]
    fn malformed_short_unfilled_and_wrong_model_frames_are_rejected() {
        let captured = frame(RECORDED_DISCHARGING);
        for length in 0..captured.len() { assert!(battery(&captured[..length]).is_none()); }
        let mut oversized = captured.clone(); oversized.push(0);
        assert!(battery(&oversized).is_none());
        assert!(battery(&[0; LENGTH as usize]).is_none());
        for offset in [0, 1, 2, 3, 8, 9, 10, 11, 12] {
            let mut changed = captured.clone(); changed[offset] ^= 1;
            assert!(battery(&changed).is_none());
        }
        for percent in [0, 101, 128, 255] {
            let mut changed = captured.clone(); changed[18] = percent;
            assert!(battery(&changed).is_none());
        }
        for percent in [1, 20, 99, 100] {
            let mut changed = captured.clone(); changed[18] = percent;
            assert_eq!(battery(&changed).unwrap().percent, Some(percent));
        }
    }

    #[test]
    fn unknown_power_flags_do_not_reuse_percent_or_infer_charging_from_link() {
        for (plug, link) in [(0, 0), (0, 3), (0, 255), (2, 1), (255, 3)] {
            for percent in [0, 100, 255] {
                let mut changed = frame(RECORDED_CHARGING);
                changed[18] = percent; changed[19] = plug; changed[21] = link;
                let mut transport = MockTransport::new(vec![changed]);
                let sample = query(&mut transport).unwrap();
                assert!(sample.connected && sample.battery_unavailable);
                let power = sample.battery.unwrap();
                assert_eq!((power.percent, power.charging), (None, None));
            }
        }
        for link in [0, 1, 3, 255] {
            for percent in [0, 100, 255] {
                let mut changed = frame(RECORDED_CHARGING); changed[21] = link; changed[18] = percent;
                let power = battery(&changed).unwrap();
                assert_eq!((power.percent, power.charging), (None, Some(true)));
            }
        }
    }

    #[test]
    fn refresh_packet_uses_only_documented_bytes_and_wraps_without_zero() {
        let mut schedule = Schedule::default();
        for number in 1..=511 {
            let sequence = schedule.next_sequence();
            assert_eq!(usize::from(sequence), (number - 1) % 255 + 1);
            let packet = request(sequence).unwrap();
            assert_eq!(packet.len(), 33);
            assert_eq!(&packet[..5], &[0x0c, 1, 0x20, 0, sequence]);
            assert!(packet[5..].iter().all(|byte| *byte == 0));
            assert!(battery(&packet).is_none());
        }
        assert!(request(0).is_none());
    }

    #[test]
    fn blank_register_triggers_one_new_sequence_then_waits_for_fill() {
        let blank = vec![0; LENGTH as usize];
        let mut transport = MockTransport::new(vec![blank.clone(), blank.clone(), blank, frame(RECORDED_DISCHARGING)]);
        let sample = query(&mut transport).unwrap();
        assert_eq!(transport.writes.iter().map(|packet| packet[4]).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(transport.waits, [30, 500, 30, 80, 160]);
        assert_eq!(transport.reads, 4);
        assert_eq!(sample.battery.unwrap().percent, Some(81));
    }

    #[test]
    fn first_reply_is_used_only_after_a_successful_refresh_and_settle() {
        let mut transport = MockTransport::new(vec![frame(RECORDED_DISCHARGING)]);
        let mut schedule = Schedule { sequence: 200, ..Default::default() };
        let deadline = transport.now + ROUND_BUDGET;
        let sample = query_with(&interface("register-not-nonce"), &mut schedule, &mut transport, deadline).unwrap();
        assert_eq!(transport.writes[0][4], 201);
        assert_eq!(transport.waits, [30]);
        assert_eq!(transport.reads, 1);
        assert_eq!(sample.battery.unwrap().percent, Some(81));
    }

    #[test]
    fn failed_refresh_never_reads_an_old_register() {
        for failed_write in [1, 2] {
            let mut transport = MockTransport::new(vec![vec![0; 33], frame(RECORDED_DISCHARGING)]);
            transport.write_error = Some(failed_write);
            assert!(query(&mut transport).is_err());
            assert_eq!(transport.writes.len(), failed_write);
            assert_eq!(transport.reads, failed_write - 1);
        }
    }

    #[test]
    fn no_reply_and_invalid_data_have_finite_retry_limits() {
        for replies in [Vec::new(), vec![vec![0; 33]; 10]] {
            let mut transport = MockTransport::new(replies);
            assert!(query(&mut transport).is_err());
            assert_eq!((transport.writes.len(), transport.reads), (2, 4));
            assert_eq!(transport.waits, [30, 500, 30, 80, 160]);
        }
    }

    #[test]
    fn late_valid_data_and_expired_rounds_are_rejected() {
        let mut transport = MockTransport::new(vec![frame(RECORDED_DISCHARGING)]);
        transport.read_cost = ROUND_BUDGET;
        assert!(query(&mut transport).is_err());
        assert_eq!((transport.writes.len(), transport.reads), (1, 1));
        let deadline = transport.now;
        assert!(query_with(&interface("deadline"), &mut Schedule::default(), &mut transport, deadline).is_err());
        assert_eq!((transport.writes.len(), transport.reads), (1, 1));
        assert!(settle(&mut transport, deadline + Duration::from_millis(30), 30).is_err());
    }

    #[test]
    fn per_device_schedule_limits_refreshes_and_recovers_after_failure() {
        let mut schedule = Schedule::default();
        let mut now = Instant::now();
        assert!(schedule.begin(now));
        assert!(!schedule.begin(now + Duration::from_secs(1)));
        for delay in [10, 20, 40, 60, 60] {
            schedule.finish(false, now);
            assert!(!schedule.begin(now + Duration::from_secs(delay - 1)));
            now += Duration::from_secs(delay);
            assert!(schedule.begin(now));
        }
        schedule.finish(true, now + Duration::from_millis(500));
        assert!(!schedule.begin(now + Duration::from_secs(9)));
        assert!(schedule.begin(now + Duration::from_secs(10)));
        assert_eq!(schedule.failures, 0);
    }

    #[test]
    fn refresh_cooldown_preserves_sample_time_but_failure_never_republishes_cache() {
        let interface = interface("cooldown-cache");
        let original = target(&interface);
        let started = Instant::now();
        let measured = started + Duration::from_millis(30);
        let sample = Snapshot { target: original.clone(), battery: battery(&frame(RECORDED_CHARGING)),
            battery_at: Some(measured), battery_unavailable: false, connected: true,
            dpi: Dpi::default(), polling_rate: Default::default() };
        read_with(&original, &[interface.clone()], started + ROUND_BUDGET, || started,
            |_| true, |_, _, _| Ok(sample)).unwrap();
        for seconds in [1, 3, 5, 9] {
            let now = started + Duration::from_secs(seconds);
            let cached = read_with(&original, &[interface.clone()], now + ROUND_BUDGET, || now,
                |_| true, |_, _, _| panic!("cooldown must not issue refresh")).unwrap();
            assert_eq!(cached.battery_at, Some(measured));
            assert_eq!(cached.battery.unwrap().charging, Some(true));
        }
        let now = started + Duration::from_secs(10);
        assert!(read_with(&original, &[interface.clone()], now + ROUND_BUDGET, || now,
            |_| true, |_, _, _| Err("actual read failed".into())).is_err());
        let now = now + Duration::from_secs(1);
        assert!(read_with(&original, &[interface], now + ROUND_BUDGET, || now,
            |_| true, |_, _, _| panic!("failure backoff")).is_err());
    }

    #[test]
    fn two_receivers_with_equal_serials_have_independent_sequences_and_cleanup() {
        let first = interface("first");
        let second = interface("second");
        assert_ne!(target(&first).key, target(&second).key);
        let mut schedules = Schedules::default();
        let first_schedule = schedules.for_interface(&first);
        let second_schedule = schedules.for_interface(&second);
        assert!(!Arc::ptr_eq(&first_schedule, &second_schedule));
        assert_eq!(first_schedule.lock().unwrap().next_sequence(), 1);
        assert_eq!(first_schedule.lock().unwrap().next_sequence(), 2);
        assert_eq!(second_schedule.lock().unwrap().next_sequence(), 1);
        let mut uppercase = first.clone(); uppercase.path.make_ascii_uppercase();
        assert!(Arc::ptr_eq(&first_schedule, &schedules.for_interface(&uppercase)));
        schedules.retain_present(&[second.clone()]);
        assert_eq!(schedules.0.len(), 1);
        assert_eq!(schedules.for_interface(&first).lock().unwrap().next_sequence(), 1);
        let mut replaced = second.clone(); replaced.serial = "replacement".into();
        assert_eq!(schedules.for_interface(&replaced).lock().unwrap().next_sequence(), 1);
        schedules.retain_present(&[]);
        assert!(schedules.0.is_empty());
    }

    #[test]
    fn discovery_requires_exact_feature_collection_without_querying() {
        let good = interface("discovery");
        assert_eq!(discover_with(&[good.clone()], |_| true).len(), 1);
        assert!(discover_with(&[good.clone()], |_| false).is_empty());
        for change in 0..9 {
            let mut wrong = good.clone();
            match change {
                0 => wrong.vendor_id = 0x1d57,
                1 => wrong.product_id = 0x5b2e,
                2 => wrong.usage_page = 0xffef,
                3 => wrong.usage = 2,
                4 => wrong.feature_len = 34,
                5 => wrong.input_len = 33,
                6 => wrong.output_len = 33,
                7 => wrong.path = wrong.path.replace("mi_01", "mi_02"),
                _ => wrong.path = wrong.path.replace("col04", "col06"),
            }
            assert!(discover_with(&[wrong], |_| panic!("invalid metadata")).is_empty());
        }
        let mut input_only = good.clone();
        input_only.path = input_only.path.replace("col04", "col06").replace("&0003#", "&0005#");
        input_only.input_len = 33; input_only.feature_len = 0;
        assert_eq!(discover_with(&[good.clone(), input_only.clone()], |_| true).len(), 1);
        input_only.feature_len = LENGTH;
        assert!(discover_with(&[good.clone(), input_only], |_| panic!("ambiguous feature")).is_empty());
        assert_eq!(discover_with(&[good, interface("another")], |_| true).len(), 2);
    }

    #[test]
    fn stale_paths_identity_and_locked_channels_never_start_queries() {
        let original = interface("blocked-path");
        let target = target(&original);
        let now = Instant::now();
        for interfaces in [vec![], vec![interface("different-path")], {
            let mut replacement = original.clone(); replacement.serial = "replacement".into(); vec![replacement]
        }] {
            assert!(read_with(&target, &interfaces, now + ROUND_BUDGET, || now,
                |_| panic!("stale target"), |_, _, _| panic!("stale query")).is_err());
        }
        assert!(read_with(&target, &[original.clone()], now + ROUND_BUDGET, || now,
            |_| false, |_, _, _| panic!("unproven query")).is_err());
        let lock = channel::lock(&channel::collection_group(&original));
        let _guard = lock.lock().unwrap();
        assert!(read_with(&target, &[original], now + ROUND_BUDGET, || now,
            |_| panic!("busy channel"), |_, _, _| panic!("busy query")).is_err());
    }

    #[test]
    fn after_read_descriptor_failure_discards_data_and_other_device_still_works() {
        let first = interface("post-proof-first");
        let second = interface("post-proof-second");
        let now = Instant::now();
        let proofs = Cell::new(0);
        let query_sample = |interface: &HidInterface, _: &mut Schedule, _: Instant| {
            let mut transport = MockTransport::new(vec![frame(RECORDED_DISCHARGING)]);
            let deadline = transport.now + ROUND_BUDGET;
            query_with(interface, &mut Schedule::default(), &mut transport, deadline)
        };
        assert!(read_with(&target(&first), &[first.clone()], now + ROUND_BUDGET, || now,
            |_| { proofs.set(proofs.get() + 1); proofs.get() == 1 }, query_sample).is_err());
        assert_eq!(proofs.get(), 2);
        assert!(read_with(&target(&second), &[second.clone()], now + ROUND_BUDGET, || now,
            |_| true, query_sample).is_ok());
        assert!(read_with(&target(&first), &[first], now + ROUND_BUDGET, || now,
            |_| true, |_, _, _| panic!("failed round backoff")).is_err());
    }
}
