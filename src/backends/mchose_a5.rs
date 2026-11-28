//! MCHOSE A5 V2 Ultra Feature11 + notification13; evidence: docs/PROTOCOLS.md.
use super::*;
use crate::hid::HidDevice;
use super::feature::FeatureChannel;
use std::{collections::BTreeMap, sync::{Arc, Mutex, OnceLock, TryLockError}, time::{Duration, Instant}};

pub struct MchoseA5;
const LENGTH: u16 = 21;
const REPORT_ID: u8 = 0x11;
const QUERY_INTERVAL: Duration = Duration::from_secs(60);
const ROUND_BUDGET: Duration = Duration::from_millis(1500);
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(interface: &HidInterface) -> bool {
    interface.vendor_id == 0x3837 && interface.product_id == 0x100c
        && interface.usage_page == 0xff01 && interface.usage == 1
}

fn control(interface: &HidInterface) -> bool {
    management(interface) && (3..=65).contains(&interface.input_len)
        && (LENGTH..=65).contains(&interface.feature_len)
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|interface| control(interface)
        && interfaces.iter().filter(|other| management(other)
            && channel::collection_group(interface) == channel::collection_group(other)).count() == 1).collect()
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    control(interface) && descriptor::Descriptor::open(api, interface)
        .is_some_and(|descriptor| descriptor.feature_byte_report(interface, REPORT_ID, LENGTH)
            && descriptor.input_report_ids(interface, interface.input_len, &[0x13]))
}

fn target(interface: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::MchoseA5, interface, 0);
    target.name = "MCHOSE A5 V2 Ultra".into();
    target.family = DeviceFamily::Mouse;
    target
}

fn valid_target(original: &Target) -> bool {
    original.backend == BackendId::MchoseA5 && original.address == 0
        && original.hid().is_ok_and(|interface| control(interface) && original.key == target(interface).key)
}

fn discover_with(interfaces: &[HidInterface], mut prove: impl FnMut(&HidInterface) -> bool) -> Vec<Target> {
    candidates(interfaces).into_iter().filter(|interface| prove(interface)).map(target).collect()
}

fn request() -> [u8; LENGTH as usize] {
    let mut packet = [0xff; LENGTH as usize];
    packet[0] = REPORT_ID; packet[1] = 0xf9; packet
}

fn battery(frame: &[u8]) -> Option<Battery> {
    if frame.len() != usize::from(LENGTH) || frame[..2] != [REPORT_ID, 0xf9]
        || [!frame[2], !frame[3]] != [0xe4, 0x41]
        || !matches!(u16::from_le_bytes([!frame[4], !frame[5]]), 0x1101 | 0x1102)
        || !frame[11] > 100 || !frame[12] > 2 { return None; }
    let percent = !frame[11];
    Some(Battery { percent: Some(percent), estimated_percent: None, approximate: false,
        charging: match !frame[12] { 0 => Some(false), 1 => Some(true), 2 if percent == 100 => Some(false), _ => None },
        voltage_mv: None, level: None })
}

trait Transport {
    fn now(&self) -> Instant;
    fn send(&mut self, packet: &[u8], timeout: u32) -> Result<(), String>;
    fn notification(&mut self, timeout: u32) -> Option<Vec<u8>>;
    fn feature(&mut self, timeout: u32) -> Result<Vec<u8>, String>;
}

struct Channel { input: HidDevice, feature: FeatureChannel }
impl Channel {
    fn open(interface: &HidInterface) -> Result<Self, String> {
        let mut input = HidDevice::open_read_only(interface)?;
        if input.writable || input.input_len() != interface.input_len { return Err(UNAVAILABLE.into()); }
        // Drain a bounded old queue before SET; no GET is allowed before a new notification.
        let mut quiet = false;
        for _ in 0..8 { if input.read_report(0).is_none() { quiet = true; break; } }
        if !quiet { return Err(UNAVAILABLE.into()); }
        Ok(Self { input, feature: FeatureChannel::open_with_length(interface, usize::from(LENGTH))? })
    }
}
impl Transport for Channel {
    fn now(&self) -> Instant { Instant::now() }
    fn send(&mut self, packet: &[u8], timeout: u32) -> Result<(), String> { self.feature.send(packet, timeout) }
    fn notification(&mut self, timeout: u32) -> Option<Vec<u8>> { self.input.read_report(timeout) }
    fn feature(&mut self, timeout: u32) -> Result<Vec<u8>, String> { self.feature.receive(REPORT_ID, timeout) }
}

fn remaining(deadline: Instant, now: Instant, cap: u32) -> Result<u32, String> {
    let millis = deadline.checked_duration_since(now).unwrap_or_default().as_millis().min(u128::from(cap)) as u32;
    if millis == 0 { Err(UNAVAILABLE.into()) } else { Ok(millis) }
}

fn query_with(interface: &HidInterface, deadline: Instant, io: &mut impl Transport) -> Result<Snapshot, String> {
    let start = io.now();
    let timeout = remaining(deadline, start, 400)?;
    let sent = io.send(&request(), timeout);
    trace::record(interface, BackendId::MchoseA5, "query-battery", u16::from(REPORT_ID), 6,
        usize::from(LENGTH), 0, start, if sent.is_ok() { trace::Outcome::Ok } else { trace::Outcome::NoReply });
    sent?;
    let response_deadline = deadline.min(io.now() + Duration::from_millis(600));
    for _ in 0..32 {
        let timeout = remaining(response_deadline, io.now(), 100)?;
        let Some(notification) = io.notification(timeout) else { continue; };
        if notification.len() != usize::from(interface.input_len) || notification.len() < 3
            || notification[0] != 0x13 || notification[2] != 0xf9 { continue; }
        let timeout = remaining(response_deadline, io.now(), 400)?;
        let started = io.now();
        let response = io.feature(timeout);
        let measured = io.now();
        let sample = response.as_ref().ok().and_then(|frame| battery(frame)).filter(|_| measured < response_deadline);
        trace::record(interface, BackendId::MchoseA5, "read-battery", u16::from(REPORT_ID), 6, 0,
            response.as_ref().map_or(0, Vec::len), started,
            if sample.is_some() { trace::Outcome::Ok } else if response.is_ok() { trace::Outcome::InvalidData }
            else { trace::Outcome::NoReply });
        return Ok(Snapshot { target: target(interface), battery: Some(sample.ok_or(UNAVAILABLE)?),
            battery_at: Some(measured), connected: true, battery_unavailable: false,
            dpi: Dpi::default(), polling_rate: Default::default() });
    }
    Err(UNAVAILABLE.into())
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
            started.checked_duration_since(at).is_some_and(|age| age < BackendId::MchoseA5.sample_lifetime())))
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

impl DeviceBackend for MchoseA5 {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        schedules().lock().unwrap_or_else(|p| p.into_inner()).retain_present(interfaces);
        Ok(discover_with(interfaces, |interface| proven(api, interface)))
    }

    fn read(&self, api: &HidApi, original: &Target, _: bool) -> Result<Snapshot, String> {
        let deadline = Instant::now() + ROUND_BUDGET;
        if !valid_target(original) { return Err(UNAVAILABLE.into()); }
        let interfaces = enumeration::scan(api)?.interfaces;
        read_with(original, &interfaces, deadline, Instant::now, |interface| proven(api, interface), |interface, deadline| {
            query_with(interface, deadline, &mut Channel::open(interface)?)
        })
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    fn interface(id: &str) -> HidInterface {
        HidInterface { path: format!("hid#vid_3837&pid_100c&mi_01&col01#{id}&0000#hid"), vendor_id: 0x3837,
            product_id: 0x100c, usage_page: 0xff01, usage: 1, input_len: 7, output_len: 0,
            feature_len: 65, product: "A5 receiver".into(), serial: "shared".into() }
    }
    fn upstream_recorded() -> Vec<u8> {
        "11F91BBEFEEEF9FDF9FFF69BFF9BFFFFFFFFFFFFFFFF".as_bytes().chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect()
    }
    fn recorded() -> Vec<u8> {
        let published = upstream_recorded();
        // Upstream's published vector has one extra FF beyond its documented transfer size.
        assert_eq!(published.len(), 22); assert_eq!(published[21], 0xff);
        published[..21].to_vec()
    }
    struct Io { time: Instant, notifications: VecDeque<Option<Vec<u8>>>, response: Vec<u8>,
        operations: Vec<&'static str>, failed_send: bool }
    impl Io {
        fn new(now: Instant) -> Self { Self { time: now, notifications: [Some(vec![0x13,0x1a,0xf9,0xff,0xff,0xff,0xff])].into(),
            response: recorded(), operations: Vec::new(), failed_send: false } }
    }
    impl Transport for Io {
        fn now(&self) -> Instant { self.time }
        fn send(&mut self, packet: &[u8], _: u32) -> Result<(), String> {
            self.operations.push("SET"); assert_eq!(packet, request()); assert_eq!(packet.len(), 21);
            if self.failed_send { Err("send failed".into()) } else { Ok(()) }
        }
        fn notification(&mut self, _: u32) -> Option<Vec<u8>> {
            self.operations.push("INPUT"); self.time += Duration::from_millis(100); self.notifications.pop_front().flatten()
        }
        fn feature(&mut self, _: u32) -> Result<Vec<u8>, String> { self.operations.push("GET21"); Ok(self.response.clone()) }
    }
    #[test]
    fn recorded_frame_identity_and_exact21_byte_request() {
        let q = request(); assert_eq!(&q[..2], &[0x11,0xf9]); assert!(q[2..].iter().all(|b| *b==0xff));
        assert!(battery(&upstream_recorded()).is_none());
        let b = battery(&recorded()).unwrap(); assert_eq!((b.percent,b.charging),(Some(100),Some(false)));
        assert!(battery(&q).is_none()); let mut r = recorded(); r.resize(65,0); assert!(battery(&r).is_none());
        for at in [0,1,2,3,4,5] { let mut r=recorded(); r[at]^=0x80; assert!(battery(&r).is_none()); }
        for len in 0..21 { assert!(battery(&recorded()[..len]).is_none()); }
    }
    #[test]
    fn zero_full_charge_and_bad_values() {
        for (percent,flag,charging) in [(0,0,Some(false)),(1,1,Some(true)),(100,2,Some(false)),(50,2,None)] {
            let mut r=recorded(); r[11]=!percent; r[12]=!flag;
            let b=battery(&r).unwrap(); assert_eq!((b.percent,b.charging),(Some(percent),charging));
        }
        for value in 101..=255 { let mut r=recorded(); r[11]=!value; assert!(battery(&r).is_none()); }
        for value in 3..=255 { let mut r=recorded(); r[12]=!value; assert!(battery(&r).is_none()); }
        let mut other_pid=recorded(); other_pid[4]=!2; assert!(battery(&other_pid).is_some());
    }
    #[test]
    fn set_notification_get_order_and_measurement_time() {
        let now=Instant::now(); let i=interface("order"); let mut io=Io::new(now);
        let s=query_with(&i,now+ROUND_BUDGET,&mut io).unwrap();
        assert_eq!(io.operations,["SET","INPUT","GET21"]);
        assert_eq!(s.battery_at,Some(now+Duration::from_millis(100)));
        assert!(!s.target.verified_identity && !s.dpi.writable && !s.polling_rate.writable);
    }
    #[test]
    fn no_notification_or_failed_set_never_reads_feature() {
        let now=Instant::now(); let i=interface("no-notify");
        let mut io=Io::new(now); io.notifications.clear();
        assert!(query_with(&i,now+ROUND_BUDGET,&mut io).is_err());
        assert!(!io.operations.contains(&"GET21"));
        let mut io=Io::new(now); io.failed_send=true;
        assert!(query_with(&i,now+ROUND_BUDGET,&mut io).is_err()); assert_eq!(io.operations,["SET"]);
        let mut io=Io::new(now); io.notifications=[Some(vec![0x13,0x1a,0xfc,0,0,0,0]),Some(vec![0x13,0x1a,0xf9])].into();
        assert!(query_with(&i,now+ROUND_BUDGET,&mut io).is_err()); assert!(!io.operations.contains(&"GET21"));
    }
    #[test]
    fn echo_and_deadline_cannot_publish() {
        let now=Instant::now(); let i=interface("echo"); let mut io=Io::new(now); io.response=request().to_vec();
        assert!(query_with(&i,now+ROUND_BUDGET,&mut io).is_err());
        let mut io=Io::new(now); assert!(query_with(&i,now,&mut io).is_err()); assert!(io.operations.is_empty());
        let mut io=Io::new(now); assert!(query_with(&i,now+Duration::from_millis(100),&mut io).is_err());
        assert!(!io.operations.contains(&"GET21"));
    }
    #[test]
    fn metadata_gate_proof_and_duplicate_collection() {
        let i=interface("scope"); assert_eq!(discover_with(&[i.clone()], |_|true).len(),1);
        assert!(discover_with(&[i.clone()], |_|false).is_empty());
        assert!(discover_with(&[i.clone(),i.clone()], |_|true).is_empty());
        for change in [|i: &mut HidInterface|i.product_id=0x100b, |i: &mut HidInterface|i.usage_page=0xff0b,
            |i: &mut HidInterface|i.usage=2, |i: &mut HidInterface|i.feature_len=20, |i: &mut HidInterface|i.input_len=0] {
            let mut wrong=i.clone();change(&mut wrong);assert!(discover_with(&[wrong], |_|panic!("scope")).is_empty());
        }
        assert!(!super::super::has_configuration(BackendId::MchoseA5));
    }
    #[test]
    fn cached_samples_and_failed_queries_remain_rate_limited() {
        let i=interface("schedule"); let now=Instant::now();let original=target(&i);let interfaces=[i.clone()];
        let first=read_with(&original,&interfaces,now+ROUND_BUDGET,||now, |_|true,
            |i,deadline|query_with(i,deadline,&mut Io::new(now))).unwrap();
        let later=now+Duration::from_secs(59);
        let cached=read_with(&original,&interfaces,later+ROUND_BUDGET,||later, |_|true, |_,_|panic!("limited")).unwrap();
        assert_eq!(cached.battery_at,first.battery_at);
        let failed=now+QUERY_INTERVAL;
        assert!(read_with(&original,&interfaces,failed+ROUND_BUDGET,||failed, |_|true, |_,_|Err("failed".into())).is_err());
        let next=failed+Duration::from_secs(1);
        assert!(read_with(&original,&interfaces,next+ROUND_BUDGET,||next, |_|true, |_,_|panic!("limited")).is_err());
    }
    #[test]
    fn receiver_isolation_removed_target_and_schedule_reset() {
        let i=interface("one");let j=interface("two");let now=Instant::now();
        assert!(read_with(&target(&i), &[j.clone()], now+ROUND_BUDGET,||now, |_|true, |_,_|panic!("removed")).is_err());
        let lock=channel::lock(&channel::collection_group(&i));let _guard=lock.lock().unwrap();
        assert!(read_with(&target(&i), &[i.clone()], now+ROUND_BUDGET,||now, |_|true, |_,_|panic!("busy")).is_err());
        let s=read_with(&target(&j), &[j.clone()], now+ROUND_BUDGET,||now, |_|true,
            |i,deadline|query_with(i,deadline,&mut Io::new(now))).unwrap();assert_ne!(s.target.key,target(&i).key);
        let mut schedules=Schedules::default();let old=schedules.for_interface(&i);schedules.retain_present(&[]);
        assert!(!Arc::ptr_eq(&old,&schedules.for_interface(&i)));
    }
}
