//! Kraken V4: one query per round; paired publication retains battery measurement time.
use super::*;
use crate::hid::HidDevice;
use std::{collections::BTreeMap, sync::{Arc, Mutex, OnceLock, TryLockError}, time::{Duration, Instant}};

pub struct RazerKraken;
const LENGTH: u16 = 64;
const REPORT_ID: u8 = 2;
const POWER: u8 = 0x21;
const CHARGE: u8 = 0x2a;
const QUERY_INTERVAL: Duration = Duration::from_secs(60);
const ROUND_BUDGET: Duration = Duration::from_millis(1500);
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(interface: &HidInterface) -> bool {
    interface.vendor_id == 0x1532 && interface.product_id == 0x056c
        && interface.usage_page == 0xff14 && interface.usage == 1
}

fn control(interface: &HidInterface) -> bool {
    management(interface) && interface.input_len == LENGTH && interface.output_len == LENGTH
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
    let mut target = Target::new(BackendId::RazerKraken, interface, 0);
    target.name = "Razer Kraken V4".into();
    target.family = DeviceFamily::Headset;
    target
}

fn valid_target(original: &Target) -> bool {
    original.backend == BackendId::RazerKraken && original.address == 0
        && original.hid().is_ok_and(|interface| control(interface) && original.key == target(interface).key)
}

fn discover_with(interfaces: &[HidInterface], mut prove: impl FnMut(&HidInterface) -> bool) -> Vec<Target> {
    candidates(interfaces).into_iter().filter(|interface| prove(interface)).map(target).collect()
}

fn checksum(bytes: &[u8]) -> u8 { bytes.iter().fold(0, |value, byte| value ^ byte) }

fn request(command: u8) -> Option<[u8; 64]> {
    if !matches!(command, POWER | CHARGE) { return None; }
    let mut packet = [0; 64];
    packet[0] = REPORT_ID; packet[2] = 0x60; packet[6] = 4;
    packet[9] = 0x80; packet[10] = command;
    packet[62] = checksum(&packet[..62]); Some(packet)
}

fn reply(command: u8, frame: &[u8]) -> Option<u8> {
    if !matches!(command, POWER | CHARGE) || frame.len() != 64
        || frame[..3] != [2, 2, 0x60] || frame[10..13] != [command, 1, 1]
        || frame[62] != checksum(&frame[1..62]) || frame[63] != 0
        || command == POWER && frame[13] > 100 { return None; }
    Some(frame[13])
}

fn exchange_with(command: u8,
    exchange: impl FnOnce(&mut dyn FnMut(&[u8]) -> bool) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    let mut invalid = None;
    let response = exchange(&mut |response| {
        if reply(command, response).is_some() { return true; }
        invalid = Some(response.to_vec()); false
    });
    response.or(invalid)
}

fn query_with(interface: &HidInterface, command: u8, deadline: Instant, mut now: impl FnMut() -> Instant,
    exchange: impl FnOnce(&[u8], u32) -> Option<Vec<u8>>) -> Result<(u8, Instant), String> {
    let started = now();
    let timeout = deadline.checked_duration_since(started).ok_or(UNAVAILABLE)?.as_millis().min(1200) as u32;
    if timeout == 0 { return Err(UNAVAILABLE.into()); }
    let packet = request(command).ok_or(UNAVAILABLE)?;
    let response = exchange(&packet, timeout);
    let measured = now();
    let sample = response.as_deref().and_then(|frame| reply(command, frame)).filter(|_| measured < deadline);
    trace::record(interface, BackendId::RazerKraken, "query", u16::from(REPORT_ID), command,
        packet.len(), response.as_ref().map_or(0, Vec::len), started,
        if sample.is_some() { trace::Outcome::Ok } else if response.is_some() { trace::Outcome::InvalidData }
        else { trace::Outcome::NoReply });
    Ok((sample.ok_or(UNAVAILABLE)?, measured))
}

#[derive(Default)]
struct Schedule {
    pending: Option<(u8, Instant)>,
    next_attempt: Option<Instant>,
    cached: Option<Snapshot>,
}

impl Schedule {
    fn accept(&mut self, interface: &HidInterface, prior: Option<(u8, Instant)>,
        result: Result<(u8, Instant), String>) -> Result<Snapshot, String> {
        let (value, at) = result?;
        let Some((percent, measured)) = prior else { self.pending = Some((value, at)); return Err(UNAVAILABLE.into()); };
        if at.checked_duration_since(measured).is_none_or(|age| age >= Duration::from_secs(120)) {
            return Err(UNAVAILABLE.into());
        }
        Ok(Snapshot { target: target(interface), battery: Some(Battery { percent: Some(percent),
            charging: match value { 0 => Some(false), 1 => Some(true), _ => None },
            estimated_percent: None, approximate: false, voltage_mv: None, level: None }),
            battery_at: Some(measured), connected: true, battery_unavailable: false,
            dpi: Dpi::default(), polling_rate: Default::default() })
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
    query: impl FnOnce(&HidInterface, u8, Instant) -> Result<(u8, Instant), String>) -> Result<Snapshot, String> {
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
            started.checked_duration_since(at).is_some_and(|age| age < BackendId::RazerKraken.sample_lifetime())))
            .ok_or_else(|| UNAVAILABLE.into());
    }
    // Reserve before opening the device, including failed opens and unanswered queries.
    schedule.next_attempt = Some(started + QUERY_INTERVAL);
    let prior = schedule.pending.take();
    let command = if prior.is_some() { CHARGE } else { POWER };
    let result = query(interface, command, deadline).and_then(|reading| {
        if !prove(interface) || now() >= deadline { return Err(UNAVAILABLE.into()); }
        Ok(reading)
    });
    let result = schedule.accept(interface, prior, result);
    schedule.cached = result.as_ref().ok().cloned();
    result
}

impl DeviceBackend for RazerKraken {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        schedules().lock().unwrap_or_else(|p| p.into_inner()).retain_present(interfaces);
        Ok(discover_with(interfaces, |interface| proven(api, interface)))
    }

    fn read(&self, api: &HidApi, original: &Target, _: bool) -> Result<Snapshot, String> {
        let deadline = Instant::now() + ROUND_BUDGET;
        if !valid_target(original) { return Err(UNAVAILABLE.into()); }
        let interfaces = enumeration::scan(api)?.interfaces;
        read_with(original, &interfaces, deadline, Instant::now, |interface| proven(api, interface), |interface, command, deadline| {
            let mut device = HidDevice::open(interface)?;
            if !device.writable || device.input_len() != LENGTH || device.output_len() != LENGTH {
                return Err(UNAVAILABLE.into());
            }
            query_with(interface, command, deadline, Instant::now, |packet, timeout|
                exchange_with(command, |accept| device.transact(packet, timeout, accept)))
        })
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    fn interface(id: &str) -> HidInterface {
        HidInterface { path: format!("hid#vid_1532&pid_056c&mi_03&col01#{id}&0000#hid"), vendor_id:0x1532,
            product_id:0x056c, usage_page:0xff14, usage:1, input_len:64, output_len:64, feature_len:0,
            product:"Kraken".into(),serial:"shared".into() }
    }
    fn hex(s: &str) -> Vec<u8> { s.as_bytes().chunks_exact(2).map(|pair|u8::from_str_radix(std::str::from_utf8(pair).unwrap(),16).unwrap()).collect() }
    fn recorded_power() -> Vec<u8> { hex("020260000000050080802101013F0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000007900") }
    fn recorded_charge() -> Vec<u8> { hex("020260000000050080802A0101000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000004D00") }
    #[test]
    fn official_queries_and_recorded_replies() {
        let q=request(POWER).unwrap();assert_eq!(&q[..13],hex("02006000000004000080210000"));assert_eq!(q[62],0xc7);
        assert_eq!(request(CHARGE).unwrap()[62],0xcc);
        for cmd in 0..=255 {assert_eq!(request(cmd).is_some(),matches!(cmd,POWER|CHARGE));}
        assert_eq!(reply(POWER,&recorded_power()),Some(63));assert_eq!(reply(CHARGE,&recorded_charge()),Some(0));
        assert!(reply(POWER,&recorded_charge()).is_none());assert!(reply(CHARGE,&recorded_power()).is_none());
    }
    #[test]
    fn echoes_corruption_headers_and_length_rejected() {
        for command in [POWER,CHARGE] {assert!(reply(command,&request(command).unwrap()).is_none());}
        for offset in 0..64 {let mut r=recorded_power();r[offset]^=1;assert!(reply(POWER,&r).is_none());}
        for offset in [0,1,2,10,11,12] {
            let mut r=recorded_power();r[offset]^=1;r[62]=checksum(&r[1..62]);assert!(reply(POWER,&r).is_none());
        }
        for len in 0..64 {assert!(reply(POWER,&recorded_power()[..len]).is_none());}
        let mut long=recorded_power();long.push(0);assert!(reply(POWER,&long).is_none());
        for percent in [0,100] {let mut r=recorded_power();r[13]=percent;r[62]=checksum(&r[1..62]);assert_eq!(reply(POWER,&r),Some(percent));}
        for percent in 101..=255 {let mut r=recorded_power();r[13]=percent;r[62]=checksum(&r[1..62]);assert!(reply(POWER,&r).is_none());}
    }
    #[test]
    fn charge_reply_keeps_battery_timestamp_and_unknown_flags_unknown() {
        let i=interface("pair");let now=Instant::now();
        for charge in [0,1,2,255] {
            let mut schedule=Schedule::default();
            assert!(schedule.accept(&i,None,Ok((63,now))).is_err());let prior=schedule.pending.take();
            let s=schedule.accept(&i,prior,Ok((charge,now+QUERY_INTERVAL))).unwrap();
            assert_eq!(s.battery_at,Some(now));let b=s.battery.unwrap();assert_eq!(b.percent,Some(63));
            assert_eq!(b.charging,match charge {0=>Some(false),1=>Some(true),_=>None});
        }
        let mut schedule=Schedule::default();
        assert!(schedule.accept(&i,Some((63,now)),Ok((1,now+Duration::from_secs(120)))).is_err());
        assert!(schedule.accept(&i,None,Err("timeout".into())).is_err());assert!(schedule.pending.is_none());
    }
    #[test]
    fn one_query_per_round_and_no_early_or_failed_pair_publication() {
        let i=interface("schedule");let now=Instant::now();let interfaces=[i.clone()];let original=target(&i);
        assert!(read_with(&original,&interfaces,now+ROUND_BUDGET,||now, |_|true, |_,command,_| {
            assert_eq!(command,POWER);Ok((63,now))
        }).is_err());
        let early=now+Duration::from_secs(59);
        assert!(read_with(&original,&interfaces,early+ROUND_BUDGET,||early, |_|true, |_,_,_|panic!("early")).is_err());
        let next=now+QUERY_INTERVAL;
        let s=read_with(&original,&interfaces,next+ROUND_BUDGET,||next, |_|true, |_,command,_| {
            assert_eq!(command,CHARGE);Ok((1,next))
        }).unwrap();assert_eq!(s.battery_at,Some(now));assert_eq!(s.battery.unwrap().charging,Some(true));
        let cached_at=next+Duration::from_secs(10);
        assert_eq!(read_with(&original,&interfaces,cached_at+ROUND_BUDGET,||cached_at, |_|true, |_,_,_|panic!("cached")).unwrap().battery_at,Some(now));
        let failed=next+QUERY_INTERVAL;
        assert!(read_with(&original,&interfaces,failed+ROUND_BUDGET,||failed, |_|true, |_,command,_| {
            assert_eq!(command,POWER);Err("failed".into())
        }).is_err());
        let reset=failed+QUERY_INTERVAL;
        assert!(read_with(&original,&interfaces,reset+ROUND_BUDGET,||reset, |_|true, |_,command,_| {
            assert_eq!(command,POWER);Ok((62,reset))
        }).is_err());
    }
    #[test]
    fn timeout_echo_and_late_response_never_return_a_measurement() {
        let i=interface("timeout");let now=Instant::now();
        assert!(query_with(&i,POWER,now+ROUND_BUDGET,||now, |_,_|None).is_err());
        assert!(query_with(&i,POWER,now+ROUND_BUDGET,||now, |p,_|Some(p.to_vec())).is_err());
        let clock=Cell::new(now);
        assert!(query_with(&i,POWER,now+ROUND_BUDGET,||clock.get(), |_,_| {clock.set(now+ROUND_BUDGET);Some(recorded_power())}).is_err());
        let result=exchange_with(POWER, |accept| {assert!(!accept(&request(POWER).unwrap()));assert!(accept(&recorded_power()));Some(recorded_power())}).unwrap();
        assert_eq!(reply(POWER,&result),Some(63));
    }
    #[test]
    fn narrow_discovery_and_unproven_or_changed_descriptors_prevent_queries() {
        let i=interface("scope");let now=Instant::now();
        for change in [|i: &mut HidInterface|i.product_id=0x056d, |i: &mut HidInterface|i.usage_page=0xff00,
            |i: &mut HidInterface|i.usage=2, |i: &mut HidInterface|i.input_len=65, |i: &mut HidInterface|i.output_len=63] {
            let mut wrong=i.clone();change(&mut wrong);assert!(discover_with(&[wrong], |_|panic!("scope")).is_empty());
        }
        assert!(discover_with(&[i.clone(),i.clone()], |_|true).is_empty());
        assert!(read_with(&target(&i),&[i.clone()],now+ROUND_BUDGET,||now, |_|false, |_,_,_|panic!("descriptor")).is_err());
        let proof=Cell::new(0);
        assert!(read_with(&target(&i),&[i.clone()],now+ROUND_BUDGET,||now, |_| {proof.set(proof.get()+1);proof.get()==1}, |_,_,_|Ok((63,now))).is_err());
        let schedule=schedules().lock().unwrap().for_interface(&i);assert!(schedule.lock().unwrap().pending.is_none());
        assert!(!super::super::has_configuration(BackendId::RazerKraken));
    }
    #[test]
    fn receiver_removal_replacement_and_locks_keep_pending_pairs_isolated() {
        let i=interface("one");let j=interface("two");let now=Instant::now();
        assert!(read_with(&target(&i),&[j.clone()],now+ROUND_BUDGET,||now, |_|true, |_,_,_|panic!("removed")).is_err());
        let lock=channel::lock(&channel::collection_group(&i));let _guard=lock.lock().unwrap();
        assert!(read_with(&target(&i),&[i.clone()],now+ROUND_BUDGET,||now, |_|true, |_,_,_|panic!("busy")).is_err());
        assert!(read_with(&target(&j),&[j.clone()],now+ROUND_BUDGET,||now, |_|true, |_,command,_| {assert_eq!(command,POWER);Ok((80,now))}).is_err());
        let mut schedules=Schedules::default();let first=schedules.for_interface(&i);first.lock().unwrap().pending=Some((63,now));
        assert!(schedules.for_interface(&j).lock().unwrap().pending.is_none());
        let mut replaced=i.clone();replaced.serial="new".into();assert!(schedules.for_interface(&replaced).lock().unwrap().pending.is_none());
        schedules.retain_present(&[]);assert!(schedules.0.is_empty());
    }
}
