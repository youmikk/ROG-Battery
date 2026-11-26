//! MCHOSE inverted Feature11/12 protocol, separate from QHW and BY keyboards.
use super::*;
use super::feature::FeatureChannel;
use crate::device::{DpiStage, StageContext};
use std::time::{Duration, Instant};

pub struct MchoseMouse;
const LENGTH: u16 = 65;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";
const CHANGED: &str = "设备档位或配置已变化，请刷新后重试";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Model { name: &'static str, max: u16 }
fn model(pid: u16) -> Option<Model> {
    let (name, max) = match pid {
        0x4018 => ("A7 V2 Pro", 26000), 0x4023 => ("A7 V2 Pro+", 26000),
        0x4019 => ("A7 V2 Ultra", 42000), 0x4021 => ("A7 V2 Ultra+", 42000),
        0x4015 => ("L7 Pro+", 26000), 0x00b0 => ("L7 Pro", 26000),
        0x1011 => ("A7 Ultra", 42000),
        _ => return None,
    };
    Some(Model { name, max })
}
fn wireless(i: &HidInterface) -> bool { matches!(i.product_id, 0x100b | 0x1020) }
fn management(i: &HidInterface) -> bool {
    let product = match i.vendor_id {
        0x3837 => matches!(i.product_id, 0x100b | 0x1020 | 0x4018 | 0x4023 | 0x4019 | 0x4021 | 0x4015),
        0x5253 => matches!(i.product_id, 0x1020 | 0x1011),
        _ => false,
    };
    product && i.usage_page == 0xff01 && i.usage == 1
}
fn control(i: &HidInterface) -> bool { management(i) && i.feature_len == LENGTH }
fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|i| control(i) && interfaces.iter().filter(|other|
        management(other) && channel::collection_group(i) == channel::collection_group(other)).count() == 1).collect()
}
fn proven(api: &HidApi, i: &HidInterface) -> bool {
    control(i) && descriptor::Descriptor::open(api, i)
        .is_some_and(|d| d.feature_reports(i, LENGTH, &[0x11, 0x12], &[0x11, 0x12, 0x14]))
}
fn candidate(i: &HidInterface) -> Target {
    let mut t = Target::new(BackendId::MchoseMouse, i, 0);
    t.family = DeviceFamily::Mouse;
    t.name = "MCHOSE Mouse".into(); t
}
fn request(command: u8) -> Option<[u8; LENGTH as usize]> {
    let (report, count) = match command { 3 | 6 => (0x11, 20), 0x67 => (0x12, 64), _ => return None };
    let mut p = [0; LENGTH as usize]; p[0] = report;
    p[1..=count].fill(0xff); p[1] = !command; Some(p)
}
fn payload(r: &[u8], command: u8) -> Option<Vec<u8>> {
    let (report, meaningful) = match command { 3 => (0x11, 7), 6 => (0x11, 11), 0x67 => (0x12, 63), _ => return None };
    // Report11 may return its 20-byte body or the collection-wide capacity.
    if !(r.len() == 65 || report == 0x11 && r.len() == 21) || r[0] != report || r[1] != !command { return None; }
    Some(r[2..2 + meaningful].iter().map(|b| !b).collect())
}
fn word(bytes: &[u8], offset: usize) -> u16 { u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) }

fn link(data: &[u8], i: &HidInterface) -> Option<bool> {
    if data.len() != 7 || data[0] > 1 || data[5] > 1
        || word(data, 1) != i.vendor_id || word(data, 3) != i.product_id { return None; }
    Some(data[0] == 1 && data[5] == 1)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct State { vid: u16, pid: u16, firmware: [u8; 4], online: bool, battery: Option<Battery> }
impl State {
    fn decode(data: &[u8], i: &HidInterface) -> Option<Self> {
        if data.len() != 11 { return None; }
        let vid = word(data, 0); let pid = word(data, 2);
        if !matches!(vid, 0x3837 | 0x5253) || matches!(pid, 0 | 0xffff | 0x100a | 0x100b | 0x1020 | 0x1021)
            || !wireless(i) && (vid != i.vendor_id || pid != i.product_id) || data[9] > 100 { return None; }
        // The vendor parser's little-endian bit fields run from bit zero.
        // Public L7 captures use 09 for an online RF link and 00 on cable.
        let mode = data[8] & 7;
        if mode != u8::from(wireless(i)) { return None; }
        let online = !wireless(i) || data[8] & 8 != 0;
        let battery = (online && (!wireless(i) || data[9] > 0)).then_some(Battery {
            percent: Some(data[9]), estimated_percent: None,
            charging: match data[10] { 0 => Some(false), 1 => Some(true), _ => None },
            voltage_mv: None, approximate: false, level: None,
        });
        Some(Self { vid, pid, firmware: data[4..8].try_into().ok()?, online, battery })
    }
    fn same_device(&self, other: &Self) -> bool {
        self.vid == other.vid && self.pid == other.pid && self.firmware == other.firmware
    }
}

fn dpi(data: &[u8], m: Model, is_wireless: bool) -> Option<Dpi> {
    if data.len() != 63 || data[0] >= 3 || !(1..=6).contains(&data[16]) { return None; }
    let index = data[if is_wireless { 2 } else { 1 }] & 15;
    if index >= data[16] { return None; }
    let mut dpi = Dpi { active_stage: Some(index), profile: Some(u16::from(data[0])), ..Default::default() };
    for n in 0..data[16] {
        let value = word(data, 4 + usize::from(n) * 2);
        if value < 50 || value > m.max || value % 50 != 0 { return None; }
        dpi.stages.push(DpiStage { index: n, value });
        if n == index { dpi.current = Some(value); }
    }
    Some(dpi)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Config { raw: Vec<u8>, vid: u16, pid: u16, firmware: [u8; 4], wireless: bool }
impl Config {
    fn new(raw: Vec<u8>, state: &State, wireless: bool) -> Option<Self> {
        dpi(&raw, model(state.pid)?, wireless)?;
        Some(Self { raw, vid: state.vid, pid: state.pid, firmware: state.firmware, wireless })
    }
    fn token(&self) -> u64 {
        let mut h = 0xcbf29ce484222325u64;
        for b in self.vid.to_le_bytes().iter().chain(self.pid.to_le_bytes().iter())
            .chain(&self.firmware).chain([u8::from(self.wireless)].iter()).chain(&self.raw) {
            h = (h ^ u64::from(*b)).wrapping_mul(0x100000001b3);
        }
        h
    }
    fn can_switch(&self) -> bool {
        // A7 V2 Ultra+ and L7 Pro+ have published whole-config write evidence.
        matches!(self.pid, 0x4021 | 0x4015)
    }
    fn dpi(&self) -> Dpi {
        let mut d = model(self.pid).and_then(|m| dpi(&self.raw, m, self.wireless)).unwrap_or_default();
        d.configuration_token = Some(self.token()); d.stage_writable = self.can_switch(); d
    }
    fn switch_packet(&self, stage: DpiStage) -> Option<([u8; 65], Vec<u8>)> {
        if !self.can_switch() || !self.dpi().stages.contains(&stage) { return None; }
        let mut expected = self.raw.clone(); let offset = if self.wireless { 2 } else { 1 };
        expected[offset] = (expected[offset] & 0xf0) | stage.index;
        let mut p = [0; 65]; p[0] = 0x12; p[1] = !0x57;
        for (n, b) in expected.iter().enumerate() { p[2 + n] = !b; }
        Some((p, expected))
    }
}

trait Channel {
    fn send(&mut self, p: &[u8]) -> Result<(), String>;
    fn receive(&mut self, id: u8) -> Result<Vec<u8>, String>;
    fn pause(&self) {}
    fn settle(&self) {}
}
struct Transport(FeatureChannel);
impl Channel for Transport {
    fn send(&mut self, p: &[u8]) -> Result<(), String> { self.0.send(p, 150) }
    fn receive(&mut self, id: u8) -> Result<Vec<u8>, String> { self.0.receive(id, 150) }
    fn pause(&self) { std::thread::sleep(Duration::from_millis(90)); }
    fn settle(&self) { std::thread::sleep(Duration::from_millis(400)); }
}

struct Session<C> { channel: C, interface: HidInterface }
impl<C: Channel> Session<C> {
    fn query(&mut self, command: u8, mut valid: impl FnMut(&[u8]) -> bool) -> Option<Vec<u8>> {
        let p = request(command)?;
        let started = Instant::now(); let deadline = started + Duration::from_millis(1900);
        let mut reply_bytes = 0;
        let result = (|| {
            self.channel.send(&p).ok()?;
            let mut previous: Option<Vec<u8>> = None;
            for _ in 0..20 {
                if Instant::now() >= deadline { break; }
                self.channel.pause();
                if Instant::now() >= deadline { break; }
                let data = self.channel.receive(p[0]).ok().and_then(|r| {
                    let data = payload(&r, command)?;
                    if !valid(&data) { return None; }
                    reply_bytes = r.len(); Some(data)
                });
                if let Some(ref data) = data {
                    if previous.as_ref() == Some(data) { return Some(data.clone()); }
                }
                // A gap or another command resets the consecutive-sample proof.
                previous = data;
            }
            None
        })();
        trace::record(&self.interface, BackendId::MchoseMouse, "query", u16::from(p[0]), command,
            p.len(), if result.is_some() { reply_bytes } else { 0 }, started,
            if result.is_some() { trace::Outcome::Ok } else { trace::Outcome::NoReply });
        result
    }
    fn link(&mut self) -> Option<bool> {
        let i = self.interface.clone();
        let r = self.query(3, |r| link(r, &i).is_some())?;
        link(&r, &i)
    }
    fn state(&mut self) -> Option<State> {
        let i = self.interface.clone();
        let r = self.query(6, |r| State::decode(r, &i).is_some())?;
        State::decode(&r, &i)
    }
    fn config(&mut self, state: &State) -> Option<Config> {
        let m = model(state.pid)?; let is_wireless = wireless(&self.interface);
        let raw = self.query(0x67, |r| dpi(r, m, is_wireless).is_some())?;
        Config::new(raw, state, is_wireless)
    }
    fn snapshot(&mut self) -> Result<Snapshot, String> {
        let is_wireless = wireless(&self.interface);
        if is_wireless {
            match self.link() {
                Some(true) => (),
                Some(false) => return Ok(self.offline(None)),
                None => return Err(UNAVAILABLE.into()),
            }
        }
        let state = self.state().ok_or(UNAVAILABLE)?;
        let battery_at = state.battery.as_ref().map(|_| Instant::now());
        if !state.online { return Ok(self.offline(Some(&state))); }
        let mut dpi_reading = Dpi::default();
        if let Some(config) = state.battery.as_ref().and_then(|_| self.config(&state)) {
            // Alternate Report11 commands before checking the paired identity
            // again. A repeated same-command buffer alone is not freshness.
            if is_wireless {
                match self.link() {
                    Some(true) => (),
                    Some(false) => return Ok(self.offline(Some(&state))),
                    None => return Err(UNAVAILABLE.into()),
                }
            }
            if let Some(after) = self.state() {
                if !state.same_device(&after) { return Err(CHANGED.into()); }
                if !after.online { return Ok(self.offline(Some(&after))); }
                // A profile or table changed while identity was checked.
                if after.battery.is_some() && self.config(&after).as_ref() == Some(&config) { dpi_reading = config.dpi(); }
            }
        }
        if is_wireless {
            match self.link() {
                Some(true) => (),
                Some(false) => return Ok(self.offline(Some(&state))),
                None => return Err(UNAVAILABLE.into()),
            }
        }
        Ok(Snapshot { target: self.target(Some(&state)), battery: state.battery, battery_at,
            battery_unavailable: false, connected: true, dpi: dpi_reading, polling_rate: Default::default() })
    }
    fn target(&self, state: Option<&State>) -> Target {
        let mut t = candidate(&self.interface);
        if let Some(s) = state {
            t.key = crate::device::stable_key(&format!("{}:mouse:{:04x}:{:04x}", t.key, s.vid, s.pid));
            t.verified_identity = true;
            if let Some(m) = model(s.pid) { t.name = format!("MCHOSE {}", m.name); }
        }
        t
    }
    fn offline(&self, state: Option<&State>) -> Snapshot {
        Snapshot { target: self.target(state), battery: None, battery_at: None, battery_unavailable: true,
            connected: false, dpi: Dpi::default(), polling_rate: Default::default() }
    }
    fn switch(&mut self, target: &Target, stage: DpiStage, token: u64) -> Result<(), String> {
        let is_wireless = wireless(&self.interface);
        if is_wireless && self.link() != Some(true) { return Err(UNAVAILABLE.into()); }
        let state = self.state().filter(|s| s.online && s.battery.is_some()).ok_or(UNAVAILABLE)?;
        if !target.verified_identity || self.target(Some(&state)).key != target.key { return Err(CHANGED.into()); }
        let before = self.config(&state).ok_or(UNAVAILABLE)?;
        if before.token() != token { return Err(CHANGED.into()); }
        let (p, expected) = before.switch_packet(stage).ok_or(CHANGED)?;
        if expected == before.raw { return Ok(()); }
        if is_wireless && self.link() != Some(true) { return Err(UNAVAILABLE.into()); }
        let checked = self.state().filter(|s| s.online && s.battery.is_some() && state.same_device(s)).ok_or(CHANGED)?;
        if self.config(&checked).as_ref() != Some(&before) { return Err(CHANGED.into()); }
        let started = Instant::now();
        let sent = self.channel.send(&p);
        trace::record(&self.interface, BackendId::MchoseMouse, "set-stage", 0x12, 0x57, p.len(), 0, started,
            if sent.is_ok() { trace::Outcome::Ok } else { trace::Outcome::NoReply });
        // A send timeout may follow an applied write. Never resend or roll back.
        self.channel.settle();
        if is_wireless && self.link() != Some(true) { return Err(UNAVAILABLE.into()); }
        let after_state = self.state().filter(|s| s.online && s.battery.is_some() && state.same_device(s)).ok_or(CHANGED)?;
        let after = self.config(&after_state).ok_or(UNAVAILABLE)?;
        if after.raw != expected { return Err("无法确认 DPI 档位切换".into()); }
        if is_wireless && self.link() != Some(true) { return Err(UNAVAILABLE.into()); }
        Ok(())
    }
}

fn with_session<T>(api: &HidApi, target: &Target, f: impl FnOnce(&mut Session<Transport>) -> Result<T, String>) -> Result<T, String> {
    let i = target.hid()?;
    if target.backend != BackendId::MchoseMouse || !proven(api, i) { return Err(UNAVAILABLE.into()); }
    let lock = channel::lock(&channel::collection_group(i));
    let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
    f(&mut Session { channel: Transport(FeatureChannel::open(i)?), interface: i.clone() })
}

impl DeviceBackend for MchoseMouse {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i)).map(candidate).collect())
    }
    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        with_session(api, target, |s| s.snapshot())
    }
    fn set_dpi_stage(&self, api: &HidApi, target: &Target, stage: DpiStage, context: StageContext) -> Result<(), String> {
        let StageContext::Configuration(token) = context else { return Err(CHANGED.into()); };
        with_session(api, target, |s| s.switch(target, stage, token))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn interface(vid: u16, pid: u16, instance: &str) -> HidInterface {
        HidInterface { path: format!(r"\\?\hid#vid_{vid:04x}&pid_{pid:04x}&mi_02&col05#8&{instance}&0005#{{hid}}"),
            vendor_id: vid, product_id: pid, usage_page: 0xff01, usage: 1,
            input_len: 65, output_len: 65, feature_len: 65, serial: "shared".into(), product: "MCHOSE Wireless".into() }
    }
    fn hex(s: &str) -> Vec<u8> { s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect() }
    // Published L7 Pro+ battery replies, unchanged; no user configuration/name bytes.
    const WIRED: &str = "11 f9 c8 c7 ea bf df fa fa ff ff c6 fe c6 ff ff ff ff ff ff ff";
    const WIRELESS: &str = "11 f9 c8 c7 ea bf df fa fa ff f6 9b ff 9b ff ff ff ff ff ff ff";
    fn frame(command: u8, data: &[u8], length: usize) -> Vec<u8> {
        let mut r = vec![0; length]; r[0] = if command == 0x67 { 0x12 } else { 0x11 }; r[1] = !command;
        for (n, b) in data.iter().enumerate() { r[2 + n] = !b; } r
    }
    fn config() -> Vec<u8> {
        // Distinct wired/wireless stages, duplicate DPI values and six stored slots.
        let mut bytes = vec![0, 0x20, 0x21, 0, 0x20, 3, 0x20, 3, 0x40, 6, 0x80, 12, 0, 25, 0x90, 0x65, 3, 0x81, 0, 2];
        bytes.extend((20..63).map(|n| n * 3)); bytes
    }
    struct Reports {
        interface: HidInterface, status: Vec<u8>, config: Vec<u8>, sent: Vec<u8>, pending: u8,
        reads: usize, link_reads: usize, connected: bool, disconnect_after: bool,
        drop_command: Option<u8>, invalid_config: bool, replace_after_config: bool, wrong_command: bool,
        writes: Vec<Vec<u8>>, apply: bool, lost_send: bool, corrupt_after: bool, replace_after: bool,
        change_before_write: bool, config_queries: usize,
    }
    impl Reports {
        fn new(i: HidInterface) -> Self {
            Self { status: payload(&hex(if wireless(&i) { WIRELESS } else { WIRED }), 6).unwrap(),
                interface: i, config: config(), sent: Vec::new(), pending: 0, reads: 0, link_reads: 0,
                connected: true, disconnect_after: false, drop_command: None, invalid_config: false,
                replace_after_config: false, wrong_command: false, writes: Vec::new(), apply: true,
                lost_send: false, corrupt_after: false, replace_after: false,
                change_before_write: false, config_queries: 0 }
        }
    }
    impl Channel for Reports {
        fn send(&mut self, p: &[u8]) -> Result<(), String> {
            assert_eq!(p.len(), 65); let command = !p[1];
            if command == 0x57 {
                assert_eq!(p[0], 0x12); self.writes.push(p.to_vec());
                if self.apply { self.config = p[2..].iter().map(|b| !b).collect(); }
                if self.corrupt_after { self.config[23] ^= 1; }
                if self.replace_after { self.status[2..4].copy_from_slice(&0x4021u16.to_le_bytes()); }
                return if self.lost_send { Err("lost write completion".into()) } else { Ok(()) };
            }
            assert!(matches!(command, 3 | 6 | 0x67)); assert_eq!(request(command).unwrap(), p);
            self.sent.push(command); self.pending = command;
            if command == 3 { self.link_reads += 1; }
            if command == 6 && self.replace_after_config && self.sent.contains(&0x67) {
                self.status[2..4].copy_from_slice(&0x4018u16.to_le_bytes());
            }
            if command == 0x67 {
                self.config_queries += 1;
                if self.change_before_write && self.config_queries == 2 { self.config[27] ^= 1; }
            }
            Ok(())
        }
        fn receive(&mut self, id: u8) -> Result<Vec<u8>, String> {
            self.reads += 1;
            if self.drop_command == Some(self.pending) { return Err("unavailable".into()); }
            let r = match self.pending {
                3 => {
                    let mut data = vec![1]; data.extend(self.interface.vendor_id.to_le_bytes());
                    data.extend(self.interface.product_id.to_le_bytes());
                    data.extend([u8::from(self.connected && !(self.disconnect_after && self.link_reads > 1)), 0]);
                    frame(3, &data, 21)
                }
                6 => frame(6, &self.status, 21),
                0x67 => {
                    let mut data = self.config.clone(); if self.invalid_config { data[16] = 0; }
                    frame(0x67, &data, 65)
                }
                _ => panic!("unexpected query"),
            };
            assert_eq!(r[0], id);
            if self.wrong_command { return Ok(frame(4, &[9; 11], 21)); }
            Ok(r)
        }
    }
    fn session() -> Session<Reports> {
        let i = interface(0x5253, 0x1020, "a");
        Session { channel: Reports::new(i.clone()), interface: i }
    }

    #[test]
    fn published_l7_frames_decode_real_identity_battery_charge_and_little_endian_flags() {
        let cable = interface(0x3837, 0x4015, "cable");
        let radio = interface(0x5253, 0x1020, "radio");
        for (i, captured, percent, charging) in [(&cable, WIRED, 57, true), (&radio, WIRELESS, 100, false)] {
            let raw = hex(captured); assert_eq!(raw.len(), 21);
            let state = State::decode(&payload(&raw, 6).unwrap(), i).unwrap();
            assert_eq!((state.vid, state.pid, state.online), (0x3837, 0x4015, true));
            let b = state.battery.unwrap(); assert_eq!(b.percent, Some(percent)); assert_eq!(b.charging, Some(charging));
            assert_eq!(b.voltage_mv, None);
            let mut padded = raw.clone(); padded.resize(65, 0x3a);
            assert_eq!(payload(&raw, 6), payload(&padded, 6));
        }
        let r = hex("11 fc fe ac ad df ef fe ff ff ff ff ff ff ff ff ff ff ff ff ff");
        assert_eq!(link(&payload(&r, 3).unwrap(), &radio), Some(true));
        assert_eq!(link(&payload(&r, 3).unwrap(), &cable), None);
    }

    #[test]
    fn command_echo_empty_payload_short_wrong_report_and_wrong_command_are_never_status() {
        let i = interface(0x5253, 0x1020, "radio");
        for command in [3, 6] {
            let decoded = payload(&request(command).unwrap(), command).unwrap();
            assert!(if command == 3 { link(&decoded, &i).is_none() } else { State::decode(&decoded, &i).is_none() });
        }
        let raw = hex(WIRELESS);
        for len in 0..21 { assert!(payload(&raw[..len], 6).is_none()); }
        let mut r = raw.clone(); r[0] = 0x12; assert!(payload(&r, 6).is_none());
        r = raw.clone(); r[1] = !4; assert!(payload(&r, 6).is_none());
        r = raw; r.push(0); assert!(payload(&r, 6).is_none());
        assert!(request(0x57).is_none()); assert!(request(0x58).is_none()); assert!(request(0x40).is_none());
    }

    #[test]
    fn feature_queries_require_two_consecutive_matching_meaningful_payloads_not_tail_equality() {
        struct Stream { replies: VecDeque<Result<Vec<u8>, String>>, sent: usize, reads: usize }
        impl Channel for Stream {
            fn send(&mut self, p: &[u8]) -> Result<(), String> { assert_eq!(p, request(6).unwrap()); self.sent += 1; Ok(()) }
            fn receive(&mut self, _: u8) -> Result<Vec<u8>, String> { self.reads += 1; self.replies.pop_front().unwrap_or(Err("empty".into())) }
        }
        let i = interface(0x5253, 0x1020, "radio"); let raw = hex(WIRELESS);
        let mut tail = raw.clone(); tail.resize(65, 0x5a);
        let mut s = Session { interface: i, channel: Stream { replies: [Ok(raw.clone()), Err("gap".into()),
            Ok(raw.clone()), Ok(frame(4, &[1; 11], 21)), Ok(raw), Ok(tail)].into(), sent: 0, reads: 0 } };
        assert_eq!(s.state().unwrap().battery.unwrap().percent, Some(100));
        assert_eq!(s.channel.reads, 6); assert_eq!(s.channel.sent, 1);
    }

    #[test]
    fn unknown_charge_sleep_zero_invalid_percent_and_wrong_transport_are_distinct() {
        let i = interface(0x5253, 0x1020, "a"); let mut data = payload(&hex(WIRELESS), 6).unwrap();
        data[10] = 7; assert_eq!(State::decode(&data, &i).unwrap().battery.unwrap().charging, None);
        data[9] = 0; let state = State::decode(&data, &i).unwrap(); assert!(state.online && state.battery.is_none());
        data[9] = 101; assert!(State::decode(&data, &i).is_none());
        data[9] = 90; data[8] = 1; let state = State::decode(&data, &i).unwrap(); assert!(!state.online && state.battery.is_none());
        data[8] = 0x30; assert!(State::decode(&data, &i).is_none()); // The upstream high-bit interpretation is not used.
        data[8] = 9; data[..2].fill(0); assert!(State::decode(&data, &i).is_none());
        data = payload(&hex(WIRED), 6).unwrap(); data[9] = 0;
        assert_eq!(State::decode(&data, &interface(0x3837, 0x4015, "cable")).unwrap().battery.unwrap().percent, Some(0));
        assert!(State::decode(&data, &interface(0x3837, 0x4021, "other-cable")).is_none());
    }

    #[test]
    fn read_only_config_honors_enabled_count_raw_order_link_nibble_and_model_range() {
        let m = model(0x4015).unwrap(); let data = config();
        let d = dpi(&data, m, true).unwrap();
        assert_eq!(d.stages, vec![DpiStage { index: 0, value: 800 }, DpiStage { index: 1, value: 800 }, DpiStage { index: 2, value: 1600 }]);
        assert_eq!(d.active_stage, Some(1)); assert_eq!(d.current, Some(800));
        assert_eq!(dpi(&data, m, false).unwrap().active_stage, Some(0));
        assert!(!d.stage_writable && !d.writable && !d.stage_editable && d.values.is_empty());
        for (offset, byte) in [(0, 3), (16, 0), (16, 7), (2, 0x23), (4, 1), (5, 0xff)] {
            let mut bad = data.clone(); bad[offset] = byte; assert!(dpi(&bad, m, true).is_none());
        }
        let mut high = data; high[16] = 6; high[14..16].copy_from_slice(&42000u16.to_le_bytes());
        assert!(dpi(&high, m, true).is_none()); assert!(dpi(&high, model(0x4021).unwrap(), true).is_some());
    }

    #[test]
    fn published_l7_config_keeps_one_enabled_stage_despite_six_populated_slots() {
        let raw = hex("12 98 ff df df ff df fc df fc bf f9 7f f3 ff e6 6f 9a fe 7e ff fd ff ff ff ff ef ff ff ff df ff ff ff cf ff ff ff bf ff ff ff af ff ff ff ff ff ff ff ff ff 7f df fc df fc bf f9 7f f3 ff e6 6f 9a");
        let bytes = payload(&raw, 0x67).unwrap(); let d = dpi(&bytes, model(0x4015).unwrap(), true).unwrap();
        assert_eq!(d.stages, [DpiStage { index: 0, value: 800 }]);
        assert_eq!((d.current, d.active_stage, d.profile), (Some(800), Some(0), Some(0)));
        assert_eq!(bytes.len(), 63); assert_eq!(word(&bytes, 14), 26000);
    }

    #[test]
    fn poll_keeps_battery_when_config_fails_and_never_queries_other_settings() {
        let mut s = session(); s.channel.invalid_config = true;
        let sample = s.snapshot().unwrap(); assert_eq!(sample.battery.unwrap().percent, Some(100));
        assert!(sample.battery_at.is_some()); assert!(sample.dpi.stages.is_empty());
        assert_eq!(s.channel.sent, [3, 6, 0x67, 3]);
        let mut s = session(); let sample = s.snapshot().unwrap();
        assert_eq!(sample.target.name, "MCHOSE L7 Pro+"); assert!(sample.target.verified_identity);
        assert_eq!(sample.dpi.stages.len(), 3); assert!(sample.dpi.stage_writable);
        assert_eq!(s.channel.sent, [3, 6, 0x67, 3, 6, 0x67, 3]);
    }

    #[test]
    fn unknown_runtime_model_keeps_generic_battery_and_no_config_reads() {
        let mut s = session(); s.channel.status[2..4].copy_from_slice(&0x4567u16.to_le_bytes());
        let r = s.snapshot().unwrap(); assert_eq!(r.target.name, "MCHOSE Mouse");
        assert_eq!(r.battery.unwrap().percent, Some(100)); assert!(r.dpi.stages.is_empty());
        assert_eq!(s.channel.sent, [3, 6, 3]);
    }

    #[test]
    fn wireless_zero_placeholder_cannot_enable_settings_even_with_cached_config() {
        let (mut s, target, token) = ready(); s.channel.status[9] = 0; s.channel.sent.clear();
        let r = s.snapshot().unwrap(); assert!(r.connected && r.battery.is_none());
        assert!(!r.dpi.stage_writable && r.dpi.stages.is_empty()); assert_eq!(s.channel.sent, [3, 6, 3]);
        assert!(s.switch(&target, DpiStage { index: 2, value: 1600 }, token).is_err());
        assert!(s.channel.writes.is_empty());
    }

    #[test]
    fn explicit_offline_clears_battery_and_replacement_during_read_does_not_publish_mixed_data() {
        let mut s = session(); s.channel.connected = false;
        let r = s.snapshot().unwrap(); assert!(!r.connected && r.battery_unavailable && r.battery.is_none());
        assert_eq!(s.channel.sent, [3]);
        let mut s = session(); s.channel.disconnect_after = true;
        let r = s.snapshot().unwrap(); assert!(!r.connected && r.battery_unavailable && r.battery_at.is_none());
        let mut s = session(); s.channel.replace_after_config = true;
        assert!(s.snapshot().is_err());
        s.channel.replace_after_config = false;
        let r = s.snapshot().unwrap(); assert_eq!(r.target.name, "MCHOSE A7 V2 Pro");
    }

    #[test]
    fn empty_or_foreign_replies_are_bounded_and_the_next_poll_can_recover() {
        let mut s = session(); s.channel.wrong_command = true;
        assert!(s.snapshot().is_err()); assert_eq!(s.channel.reads, 20); assert_eq!(s.channel.sent, [3]);
        s.channel.wrong_command = false;
        assert!(s.snapshot().unwrap().battery.is_some());
        let mut s = session(); s.channel.drop_command = Some(6);
        assert!(s.snapshot().is_err()); assert_eq!(s.channel.sent, [3, 6]);
    }

    #[test]
    fn exact_vid_pid_feature_collection_excludes_v3_keyboards_firmware_and_duplicate_instances() {
        let a = interface(0x5253, 0x1020, "a"); let b = interface(0x5253, 0x1020, "b");
        assert!(control(&a)); assert_eq!(candidates(&[a.clone(), b.clone()]).len(), 2);
        assert_ne!(candidate(&a).key, candidate(&b).key);
        for (vid, pid, page, usage, feature) in [(0x3837, 0x1015, 0xff80, 1, 65),
            (0x3837, 0x1014, 0xff01, 1, 65), (0x3837, 0x100a, 0xff01, 1, 65),
            (0x5253, 0x1020, 0xff0b, 0x104, 65), (0x5253, 0x1020, 0xff01, 2, 65),
            (0x5253, 0x1020, 0xff01, 1, 64), (0x5253, 0x1020, 0xff01, 1, 0)] {
            let mut wrong = a.clone(); wrong.vendor_id = vid; wrong.product_id = pid; wrong.usage_page = page;
            wrong.usage = usage; wrong.feature_len = feature; assert!(!control(&wrong));
        }
        let mut sibling = a.clone(); sibling.path = sibling.path.replace("col05", "col06").replace("&0005", "&0006");
        sibling.feature_len = 64; assert!(candidates(&[a, sibling]).is_empty());
    }

    fn ready() -> (Session<Reports>, Target, u64) {
        let mut s = session(); let sample = s.snapshot().unwrap();
        s.channel.config_queries = 0;
        (s, sample.target, sample.dpi.configuration_token.unwrap())
    }

    #[test]
    fn stage_switch_preserves_all_63_config_bytes_except_live_link_index_and_never_resends() {
        for (vid, pid) in [(0x3837, 0x4015), (0x5253, 0x1020)] {
            for lost in [false, true] {
                let i = interface(vid, pid, "unit"); let mut s = Session { channel: Reports::new(i.clone()), interface: i };
                let sample = s.snapshot().unwrap(); let original = s.channel.config.clone();
                s.channel.lost_send = lost;
                s.switch(&sample.target, DpiStage { index: 2, value: 1600 }, sample.dpi.configuration_token.unwrap()).unwrap();
                assert_eq!(s.channel.writes.len(), 1);
                let written = &s.channel.writes[0]; assert_eq!(&written[..2], &[0x12, 0xa8]);
                let link = if pid == 0x1020 { 2 } else { 1 };
                for n in 0..63 {
                    assert_eq!(!written[2 + n], if n == link { (original[n] & 0xf0) | 2 } else { original[n] });
                }
                assert_eq!(s.channel.config[link] >> 4, original[link] >> 4);
            }
        }
    }

    #[test]
    fn stale_profile_tables_buttons_identity_or_stage_value_prevent_all_writes() {
        for failure in 0..8 {
            let (mut s, mut target, token) = ready(); let mut stage = DpiStage { index: 2, value: 1600 };
            match failure {
                0 => s.channel.config[0] = 1,
                1 => s.channel.config[23] ^= 1,
                2 => s.channel.config[61] ^= 1,
                3 => s.channel.status[2..4].copy_from_slice(&0x4021u16.to_le_bytes()),
                4 => target.verified_identity = false,
                5 => stage.index = 3,
                6 => stage.value = 800,
                _ => s.channel.change_before_write = true,
            }
            assert!(s.switch(&target, stage, token).is_err()); assert!(s.channel.writes.is_empty());
        }
    }

    #[test]
    fn ignored_write_changed_neighbor_and_repaired_receiver_fail_readback_without_rollback() {
        for failure in 0..3 {
            let (mut s, target, token) = ready();
            match failure { 0 => s.channel.apply = false, 1 => s.channel.corrupt_after = true, _ => s.channel.replace_after = true }
            assert!(s.switch(&target, DpiStage { index: 2, value: 1600 }, token).is_err());
            assert_eq!(s.channel.writes.len(), 1);
        }
    }

    #[test]
    fn read_only_models_cannot_inherit_whole_config_write_and_current_stage_needs_no_write() {
        let (mut s, target, token) = ready();
        s.switch(&target, DpiStage { index: 1, value: 800 }, token).unwrap(); assert!(s.channel.writes.is_empty());
        for pid in [0x4018u16, 0x4023, 0x4019, 0x00b0, 0x1011] {
            let mut s = session(); s.channel.status[2..4].copy_from_slice(&pid.to_le_bytes());
            let r = s.snapshot().unwrap(); assert!(r.battery.is_some()); assert!(!r.dpi.stage_writable);
            assert!(s.switch(&r.target, DpiStage { index: 2, value: 1600 }, r.dpi.configuration_token.unwrap()).is_err());
            assert!(s.channel.writes.is_empty());
        }
    }
}
