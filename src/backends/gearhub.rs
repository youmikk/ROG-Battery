//! AJ179 PRO on the GearHub relay. Host VID/PID identifies transport, not model.
use super::*;
use super::feature::FeatureChannel;
use crate::device::{DpiStage, PollingRate, StageContext};
use std::time::{Duration, Instant};

pub struct GearHub;
const LENGTH: u16 = 65;
const AJ179_PRO: u32 = 1851;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";
const CHANGED: &str = "设备档位或配置已变化，请刷新后重试";
type Block = [u8; 64];

fn wireless(i: &HidInterface) -> bool { i.product_id == 0x402d }
fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x3151 && matches!(i.product_id, 0x4026 | 0x402d)
        && i.usage_page == 0xffff && i.usage == 2
}
fn control(i: &HidInterface) -> bool { management(i) && i.feature_len == LENGTH }
fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|i| control(i) && interfaces.iter().filter(|other|
        management(other) && channel::collection_group(i) == channel::collection_group(other)).count() == 1).collect()
}
fn proven(api: &HidApi, i: &HidInterface) -> bool {
    control(i) && descriptor::Descriptor::open(api, i)
        .is_some_and(|d| d.feature_reports(i, LENGTH, &[0], &[0]))
}
fn candidate(i: &HidInterface) -> Target {
    let mut t = Target::new(BackendId::GearHub, i, 5);
    t.family = DeviceFamily::Mouse; t.name = "GearHub Mouse".into(); t
}

fn checksum(p: &mut Block) {
    p[7] = 0xffu8.wrapping_sub(p[..7].iter().fold(0u8, |sum, b| sum.wrapping_add(*b)));
}
fn request(command: u8, profile: u8) -> Option<Block> {
    if !matches!(command, 0x8f | 0xd3 | 0xd4) || profile >= 8 || command != 0xd4 && profile != 0 { return None; }
    let mut p = [0; 64]; p[0] = command; p[1] = profile; checksum(&mut p); Some(p)
}
fn wire(block: &Block) -> [u8; 65] {
    let mut p = [0; 65]; p[1..].copy_from_slice(block); p
}
fn payload(r: &[u8]) -> Option<Block> {
    // Windows includes the zero report-ID byte; do not shift a raw/short frame.
    if r.len() != 65 || r[0] != 0 { return None; }
    r[1..].try_into().ok()
}
fn reply(r: Block, p: &Block) -> Option<Block> {
    // Reply byte 7 is not a checksum of the returned data. Do not invent one.
    (r[0] == p[0] && r != *p).then_some(r)
}
fn identity(r: &Block) -> Option<u32> {
    if r[0] != 0x8f { return None; }
    let id = u32::from_le_bytes(r[1..5].try_into().ok()?);
    (!matches!(id, 0 | u32::MAX)).then_some(id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Status { online: bool, percent: Option<u8>, can_read: bool, can_send: bool }
impl Status {
    fn decode(r: &Block) -> Option<Self> {
        if [0, 3, 4, 5].iter().any(|n| r[*n] > 1) || r[..6].iter().all(|b| *b == 0) { return None; }
        let online = r[4] == 0;
        if online && r[2] > 100 { return None; }
        Some(Self { online, percent: online.then_some(r[2]), can_read: r[0] == 1, can_send: r[5] == 1 })
    }
    fn battery(self) -> Option<Battery> {
        self.percent.map(|percent| Battery { percent: Some(percent), estimated_percent: None,
            charging: None, voltage_mv: None, approximate: false, level: None })
    }
}

fn profile(options: &Block) -> Option<u8> {
    (options[0] == 0xd3 && options[8] < 8).then_some(options[8])
}
fn stages(raw: &Block, id: u32, bank: u8) -> Option<Dpi> {
    if id != AJ179_PRO || bank >= 8 || raw[0] != 0xd4 || raw[1] != bank
        || !(1..=8).contains(&raw[3]) || raw[2] >= raw[3] { return None; }
    let mut d = Dpi { profile: Some(u16::from(bank)), active_stage: Some(raw[2]), ..Default::default() };
    for n in 0..raw[3] {
        let offset = usize::from(n) * 2;
        let x = u16::from_le_bytes([raw[8 + offset], raw[9 + offset]]);
        let y = u16::from_le_bytes([raw[24 + offset], raw[25 + offset]]);
        // The official layout marks each all-zero pair disabled independently.
        if x == 0 && y == 0 { continue; }
        if [x, y].iter().any(|v| !(50..=32000).contains(v) || v % 50 != 0) { return None; }
        d.stages.push(DpiStage { index: n, value: x }); d.stage_y.insert(n, y);
        if n == raw[2] { d.current = Some(x); d.current_y = Some(y); }
    }
    d.current.map(|_| d)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Config { id: u32, options: Block, raw: Block }
impl Config {
    fn new(id: u32, options: Block, raw: Block) -> Option<Self> {
        stages(&raw, id, profile(&options)?)?;
        Some(Self { id, options, raw })
    }
    fn token(&self) -> u64 {
        let mut h = 0xcbf29ce484222325u64;
        for b in self.id.to_le_bytes().iter().chain(&self.options).chain(&self.raw) {
            h = (h ^ u64::from(*b)).wrapping_mul(0x100000001b3);
        }
        h
    }
    fn dpi(&self) -> Dpi {
        let mut d = stages(&self.raw, self.id, self.options[8]).unwrap_or_default();
        d.configuration_token = Some(self.token());
        // Current vendor catalog extends to 32000; published hardware write
        // evidence only covers 26000. Preserve higher valid readings as read-only.
        d.stage_writable = self.id == AJ179_PRO && d.stages.iter().all(|s| s.value <= 26000)
            && d.stage_y.values().all(|v| *v <= 26000);
        d
    }
    fn rate(&self) -> PollingRate {
        let current = match self.options[9] {
            1 => Some(1000), 2 => Some(500), 4 => Some(250), 8 => Some(125),
            129 => Some(8000), 130 => Some(4000), 132 => Some(2000), _ => None,
        };
        PollingRate { current, ..Default::default() }
    }
    fn switch_packet(&self, stage: DpiStage) -> Option<(Block, Self)> {
        let dpi = self.dpi();
        if !dpi.stage_writable || !dpi.stages.contains(&stage) { return None; }
        let mut expected = self.clone(); expected.raw[2] = stage.index;
        let mut p = expected.raw; p[0] = 0x54; checksum(&mut p);
        Some((p, expected))
    }
}

trait Channel {
    fn send(&mut self, p: &[u8]) -> Result<(), String>;
    fn receive(&mut self) -> Result<Vec<u8>, String>;
    fn pause(&self, _ms: u64) {}
}
struct Transport(FeatureChannel);
impl Channel for Transport {
    fn send(&mut self, p: &[u8]) -> Result<(), String> { self.0.send(p, 150) }
    fn receive(&mut self) -> Result<Vec<u8>, String> { self.0.receive(0, 150) }
    fn pause(&self, ms: u64) { std::thread::sleep(Duration::from_millis(ms)); }
}
struct Session<C> { channel: C, interface: HidInterface, selected: bool }
impl<C: Channel> Session<C> {
    fn raw(&mut self, command: u8, argument: u8) -> Result<(), String> {
        let mut p = [0; 64]; p[0] = command; p[1] = argument; self.channel.send(&wire(&p))
    }
    fn status(&mut self) -> Option<Status> {
        if !wireless(&self.interface) { return None; }
        let started = Instant::now(); let mut bytes = 0;
        let result = (|| {
            self.raw(0xf7, 0).ok()?; self.channel.pause(10);
            let r = self.channel.receive().ok()?; bytes = r.len(); Status::decode(&payload(&r)?)
        })();
        trace::record(&self.interface, BackendId::GearHub, "status", 0, 0xf7, 65, bytes, started,
            if result.is_some() { trace::Outcome::Ok } else { trace::Outcome::NoReply });
        result
    }
    fn select(&mut self) -> Option<()> {
        if wireless(&self.interface) && !self.selected {
            self.raw(0xf6, 5).ok()?; self.channel.pause(50); self.selected = true;
        }
        Some(())
    }
    fn ready(&mut self, reading: bool) -> Option<()> {
        if !wireless(&self.interface) { return Some(()); }
        let deadline = Instant::now() + Duration::from_millis(2200);
        for _ in 0..8 {
            if Instant::now() >= deadline { break; }
            self.channel.pause(100);
            let status = self.status()?;
            if !status.online { return None; }
            if if reading { status.can_read } else { status.can_send } { return Some(()); }
        }
        None
    }
    fn query(&mut self, command: u8, bank: u8) -> Option<Block> {
        let p = request(command, bank)?;
        let started = Instant::now(); let mut bytes = 0;
        let result = (|| {
            self.select()?; self.ready(false)?;
            self.channel.send(&wire(&p)).ok()?;
            self.ready(true)?;
            if wireless(&self.interface) { self.raw(0xfc, 0).ok()?; }
            self.channel.pause(10);
            let r = self.channel.receive().ok()?; bytes = r.len(); reply(payload(&r)?, &p)
        })();
        trace::record(&self.interface, BackendId::GearHub, "query", 0, command, 65, bytes, started,
            if result.is_some() { trace::Outcome::Ok } else { trace::Outcome::NoReply });
        result
    }
    fn id(&mut self) -> Option<u32> { identity(&self.query(0x8f, 0)?) }
    fn config(&mut self, id: u32) -> Option<Config> {
        if id != AJ179_PRO { return None; }
        let options = self.query(0xd3, 0)?;
        let bank = profile(&options)?;
        let config = Config::new(id, options, self.query(0xd4, bank)?)?;
        if self.query(0xd4, bank)? != config.raw || self.query(0xd3, 0)? != options { return None; }
        Some(config)
    }
    fn target(&self, id: Option<u32>) -> Target {
        let mut t = candidate(&self.interface);
        if let Some(id) = id {
            t.key = crate::device::stable_key(&format!("{}:mouse:{id}", t.key)); t.verified_identity = true;
            if id == AJ179_PRO { t.name = "AJAZZ AJ179 PRO".into(); }
        }
        t
    }
    fn offline(&self, id: Option<u32>) -> Snapshot {
        Snapshot { target: self.target(id), connected: false, battery: None, battery_at: None,
            battery_unavailable: true, dpi: Dpi::default(), polling_rate: Default::default() }
    }
    fn snapshot(&mut self) -> Result<Snapshot, String> {
        let is_wireless = wireless(&self.interface);
        if is_wireless && !self.status().ok_or(UNAVAILABLE)?.online { return Ok(self.offline(None)); }
        let id = self.id().ok_or(UNAVAILABLE)?;
        let config = self.config(id);
        if self.id() != Some(id) { return Err(CHANGED.into()); }
        let (battery, battery_at) = if is_wireless {
            let status = self.status().ok_or(UNAVAILABLE)?;
            if !status.online { return Ok(self.offline(Some(id))); }
            (status.battery(), Some(Instant::now()))
        } else { (None, None) };
        Ok(Snapshot { target: self.target(Some(id)), connected: true, battery, battery_at,
            battery_unavailable: !is_wireless, dpi: config.as_ref().map_or_else(Dpi::default, Config::dpi),
            polling_rate: config.as_ref().map_or_else(PollingRate::default, Config::rate) })
    }
    fn switch(&mut self, target: &Target, stage: DpiStage, token: u64) -> Result<(), String> {
        let id = self.id().ok_or(UNAVAILABLE)?;
        if !target.verified_identity || self.target(Some(id)).key != target.key { return Err(CHANGED.into()); }
        let before = self.config(id).ok_or(UNAVAILABLE)?;
        if before.token() != token { return Err(CHANGED.into()); }
        let (p, expected) = before.switch_packet(stage).ok_or(CHANGED)?;
        if before == expected { return Ok(()); }
        if self.id() != Some(id) || self.config(id).as_ref() != Some(&before) { return Err(CHANGED.into()); }
        self.select().ok_or(UNAVAILABLE)?; self.ready(false).ok_or(UNAVAILABLE)?;
        let started = Instant::now(); let sent = self.channel.send(&wire(&p));
        trace::record(&self.interface, BackendId::GearHub, "set-stage", 0, 0x54, 65, 0, started,
            if sent.is_ok() { trace::Outcome::Ok } else { trace::Outcome::NoReply });
        // A lost send completion may follow an applied write. Do not resend it.
        self.channel.pause(400);
        if self.id() != Some(id) { return Err(CHANGED.into()); }
        if self.config(id).as_ref() != Some(&expected) { return Err("无法确认 DPI 档位切换".into()); }
        if self.id() != Some(id) { return Err(CHANGED.into()); }
        if wireless(&self.interface) && !self.status().ok_or(UNAVAILABLE)?.online { return Err(UNAVAILABLE.into()); }
        Ok(())
    }
}

fn with_session<T>(api: &HidApi, target: &Target, f: impl FnOnce(&mut Session<Transport>) -> Result<T, String>) -> Result<T, String> {
    let i = target.hid()?;
    if target.backend != BackendId::GearHub || !proven(api, i) { return Err(UNAVAILABLE.into()); }
    let lock = channel::lock(&channel::collection_group(i));
    let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
    f(&mut Session { channel: Transport(FeatureChannel::open(i)?), interface: i.clone(), selected: false })
}
impl DeviceBackend for GearHub {
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

    // Original public AJ179 PRO reply bodies, with no personal identifiers.
    const ID: &str = "8f3b0700000500700000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
    const DPI: &str = "d40002080000002b58022003e803b004dc0500000000000058022003e803b004dc05000000000000ff000000ff000000ffbd10e0ffff00ff00ffff00ffffffff";
    const OPTIONS: &str = "d30000000000002c0001020000000c01010000000000000003030407000000000000000000000000140000001400000000006464010000000000000000000000";
    const STATUS: &str = "01005501000102000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
    fn hex(s: &str) -> Block {
        assert_eq!(s.len(), 128);
        let mut b = [0; 64];
        for n in 0..64 { b[n] = u8::from_str_radix(&s[n * 2..n * 2 + 2], 16).unwrap(); }
        b
    }
    fn interface(pid: u16, instance: &str) -> HidInterface {
        HidInterface { vendor_id: 0x3151, product_id: pid, usage_page: 0xffff, usage: 2,
            feature_len: 65, input_len: 0, output_len: 0, product: "2.4G Mouse".into(), serial: "shared".into(),
            path: format!(r"\\?\hid#vid_3151&pid_{pid:04x}&mi_02&col01#8&{instance}&0001#{{hid}}") }
    }
    fn captured_config() -> Config { Config::new(AJ179_PRO, hex(OPTIONS), hex(DPI)).unwrap() }

    struct Fake {
        sent: Vec<Block>, options: Block, dpi: Block, status: Block, id: Block,
        pending: Option<Block>, relay: Option<Block>, selected: bool, wireless: bool,
        echo: bool, apply: bool, lose_completion: bool, corrupt_neighbor: bool,
        swap_identity: bool, change_options: bool, reads: usize, bad_dpi: bool,
    }
    impl Fake {
        fn new(wireless: bool) -> Self {
            Self { options: hex(OPTIONS), dpi: hex(DPI), status: hex(STATUS), id: hex(ID),
                wireless, apply: true, sent: Vec::new(), pending: None, relay: None, selected: false,
                echo: false, lose_completion: false, corrupt_neighbor: false, swap_identity: false,
                change_options: false, reads: 0, bad_dpi: false }
        }
    }
    impl Channel for Fake {
        fn send(&mut self, p: &[u8]) -> Result<(), String> {
            assert_eq!(p.len(), 65); assert_eq!(p[0], 0);
            let body = payload(p).unwrap(); self.sent.push(body);
            if self.echo { self.pending = Some(body); return Ok(()); }
            match body[0] {
                0xf6 => { assert!(self.wireless); assert_eq!(body[1], 5); self.selected = true; }
                0xf7 => { assert!(self.wireless); self.pending = Some(self.status); }
                0xfc => { assert!(self.wireless); self.pending = self.relay.take(); }
                0x8f | 0xd3 | 0xd4 => {
                    assert_eq!(body[..8].iter().fold(0u8, |sum, b| sum.wrapping_add(*b)), 255);
                    let r = match body[0] {
                        0x8f => self.id,
                        0xd3 => self.options,
                        0xd4 => {
                            assert_eq!(body[1], self.options[8], "must query the live profile");
                            self.reads += 1;
                            if self.change_options && self.reads == 2 { self.options[10] ^= 1; }
                            if self.bad_dpi { request(0xd4, body[1]).unwrap() } else { self.dpi }
                        }
                        _ => unreachable!(),
                    };
                    if self.wireless { assert!(self.selected); self.relay = Some(r); }
                    else { self.pending = Some(r); }
                }
                0x54 => {
                    assert_eq!(body[1], self.options[8]);
                    assert_eq!(body[..8].iter().fold(0u8, |sum, b| sum.wrapping_add(*b)), 255);
                    if self.apply {
                        let header = self.dpi[7]; self.dpi = body; self.dpi[0] = 0xd4; self.dpi[7] = header;
                        if self.corrupt_neighbor { self.dpi[63] ^= 1; }
                    }
                    if self.swap_identity { self.id[1..5].copy_from_slice(&2285u32.to_le_bytes()); }
                    if self.lose_completion { return Err("send completion lost".into()); }
                }
                _ => panic!("unexpected command"),
            }
            Ok(())
        }
        fn receive(&mut self) -> Result<Vec<u8>, String> {
            self.pending.take().map(|p| wire(&p).to_vec()).ok_or("no reply".into())
        }
    }
    fn session(wireless: bool) -> Session<Fake> {
        Session { interface: interface(if wireless { 0x402d } else { 0x4026 }, "one"),
            channel: Fake::new(wireless), selected: false }
    }
    fn choose_last(s: &mut Session<Fake>) -> Result<(), String> {
        let target = s.target(Some(AJ179_PRO)); let config = captured_config();
        s.switch(&target, config.dpi().stages[4], config.token())
    }

    #[test]
    fn published_frames_prove_model_battery_and_five_enabled_stages() {
        assert_eq!(identity(&hex(ID)), Some(AJ179_PRO));
        let status = Status::decode(&hex(STATUS)).unwrap();
        assert_eq!(status.battery().unwrap().percent, Some(85));
        assert_eq!(status.battery().unwrap().charging, None);
        let c = captured_config(); let d = c.dpi();
        assert_eq!(d.current, Some(1000)); assert_eq!(d.current_y, Some(1000));
        assert_eq!(d.active_stage, Some(2)); assert_eq!(d.profile, Some(0));
        assert_eq!(d.stages.iter().map(|s| s.value).collect::<Vec<_>>(), vec![600, 800, 1000, 1200, 1500]);
        assert_eq!(c.raw[3], 8); assert_eq!(c.rate().current, Some(1000));
        assert!(d.stage_writable); assert!(!d.writable); assert!(!d.stage_editable);
    }
    #[test]
    fn exact_collection_and_instance_proof_excludes_bluetooth_and_duplicate_collections() {
        let a = interface(0x402d, "one"); let b = interface(0x402d, "two");
        let mut sibling = a.clone(); sibling.path = a.path.replace("col01", "col02").replace("&0001#", "&0002#");
        let mut wrong_usage = a.clone(); wrong_usage.usage = 1;
        let mut wrong_length = a.clone(); wrong_length.feature_len = 64;
        assert_eq!(candidates(&[a.clone(), b.clone()]).len(), 2);
        assert_eq!(candidates(&[a.clone(), sibling]).len(), 0);
        assert_eq!(candidates(&[a.clone(), wrong_length]).len(), 0);
        assert_eq!(candidates(&[a.clone(), wrong_usage, interface(0x402c, "bt")]).len(), 1);
        assert_ne!(candidate(&a).key, candidate(&b).key);
        assert!(!control(&interface(0x5043, "newer")));
    }
    #[test]
    fn windows_framing_command_checks_and_request_echo_rejection_are_strict() {
        let id = hex(ID); let p = request(0x8f, 0).unwrap();
        assert_eq!(&p[..8], &[0x8f, 0, 0, 0, 0, 0, 0, 0x70]);
        assert_eq!(payload(&wire(&id)), Some(id));
        assert!(payload(&id).is_none());
        let mut wrong = wire(&id); wrong[0] = 1; assert!(payload(&wrong).is_none());
        assert!(payload(&wire(&id)[..64]).is_none());
        assert!(reply(p, &p).is_none()); assert!(reply(hex(DPI), &p).is_none());
        assert!(reply([0; 64], &p).is_none());
        assert!(request(0x54, 0).is_none()); assert!(request(0xd4, 8).is_none());
        assert!(identity(&request(0x8f, 0).unwrap()).is_none());
        let mut invalid = id; invalid[1..5].fill(0xff); assert!(identity(&invalid).is_none());
        // Captured identity reply does not carry a checksum of its changed fields.
        assert_ne!(id[..8].iter().fold(0u8, |sum, b| sum.wrapping_add(*b)), 255);
    }
    #[test]
    fn status_rejects_echo_and_unknown_flags_but_preserves_valid_zero() {
        assert!(Status::decode(&[0; 64]).is_none());
        let mut p = hex(STATUS); p[2] = 0;
        assert_eq!(Status::decode(&p).unwrap().percent, Some(0));
        p[2] = 100; assert_eq!(Status::decode(&p).unwrap().percent, Some(100));
        p[2] = 101; assert!(Status::decode(&p).is_none());
        p[4] = 1; assert_eq!(Status::decode(&p).unwrap().percent, None);
        for offset in [0, 3, 4, 5] {
            let mut p = hex(STATUS); p[offset] = 2; assert!(Status::decode(&p).is_none());
        }
    }
    #[test]
    fn nonzero_current_profile_is_read_before_its_table_and_never_selected() {
        let mut s = session(true); s.channel.options[8] = 2; s.channel.dpi[1] = 2;
        let reading = s.snapshot().unwrap(); assert_eq!(reading.dpi.profile, Some(2));
        assert_eq!(reading.battery.unwrap().percent, Some(85));
        assert!(s.channel.sent.iter().all(|p| matches!(p[0], 0xf6 | 0xf7 | 0xfc | 0x8f | 0xd3 | 0xd4)));
        assert_eq!(s.channel.sent.iter().filter(|p| p[0] == 0xf6).count(), 1);
        let commands: Vec<_> = s.channel.sent.iter().map(|p| p[0]).collect();
        let query = commands.iter().position(|c| *c == 0x8f).unwrap();
        assert_eq!(commands[query - 1], 0xf7);
        assert_eq!(&commands[query + 1..query + 3], &[0xf7, 0xfc]);
    }
    #[test]
    fn direct_mode_has_settings_without_inventing_a_battery_or_relay() {
        let mut s = session(false); let r = s.snapshot().unwrap();
        assert_eq!(r.target.name, "AJAZZ AJ179 PRO"); assert!(r.connected);
        assert!(r.battery.is_none()); assert!(r.battery_at.is_none()); assert!(r.battery_unavailable);
        assert_eq!(r.dpi.current, Some(1000));
        assert!(s.channel.sent.iter().all(|p| matches!(p[0], 0x8f | 0xd3 | 0xd4)));
    }
    #[test]
    fn unknown_paired_model_keeps_battery_without_guessing_sensor_or_brand() {
        let mut s = session(true); s.channel.id[1..5].copy_from_slice(&2285u32.to_le_bytes());
        let r = s.snapshot().unwrap(); assert_eq!(r.target.name, "GearHub Mouse");
        assert_eq!(r.battery.unwrap().percent, Some(85)); assert_eq!(r.dpi, Dpi::default());
        assert!(s.channel.sent.iter().all(|p| !matches!(p[0], 0xd3 | 0xd4 | 0x54)));
        assert_ne!(r.target.key, s.target(Some(AJ179_PRO)).key);
    }
    #[test]
    fn offline_is_explicit_and_wake_restores_a_new_sample() {
        let mut s = session(true); s.channel.status[4] = 1;
        let r = s.snapshot().unwrap(); assert!(!r.connected); assert!(r.battery_unavailable);
        assert!(r.battery.is_none()); assert_eq!(s.channel.sent.len(), 1);
        s.channel.status[4] = 0; s.channel.status[2] = 84;
        let r = s.snapshot().unwrap(); assert!(r.connected); assert_eq!(r.battery.unwrap().percent, Some(84));
        assert!(r.battery_at.is_some());
        s.channel.echo = true; assert!(s.snapshot().is_err());
    }
    #[test]
    fn failed_or_changing_dpi_keeps_independent_battery_and_disables_settings() {
        for change in [false, true] {
            let mut s = session(true); s.channel.bad_dpi = !change; s.channel.change_options = change;
            let r = s.snapshot().unwrap(); assert_eq!(r.battery.unwrap().percent, Some(85));
            assert_eq!(r.dpi, Dpi::default());
        }
    }
    #[test]
    fn invalid_axes_counts_profiles_and_disabled_selection_cannot_authorize_writes() {
        for (offset, value) in [(3, 0), (3, 9), (2, 5), (1, 2), (8, 0), (9, 0xff), (24, 1)] {
            let mut raw = hex(DPI); raw[offset] = value;
            assert!(Config::new(AJ179_PRO, hex(OPTIONS), raw).is_none(), "offset {offset}");
        }
        let mut raw = hex(DPI); raw[12..14].fill(0); raw[28..30].fill(0);
        assert!(Config::new(AJ179_PRO, hex(OPTIONS), raw).is_none());
        let mut options = hex(OPTIONS); options[8] = 8;
        assert!(Config::new(AJ179_PRO, options, hex(DPI)).is_none());
        assert!(Config::new(2285, hex(OPTIONS), hex(DPI)).is_none());
    }
    #[test]
    fn disabled_slots_in_the_middle_keep_original_numbers_and_bytes() {
        let mut raw = hex(DPI); raw[10..12].fill(0); raw[26..28].fill(0);
        let config = Config::new(AJ179_PRO, hex(OPTIONS), raw).unwrap();
        let d = config.dpi();
        assert_eq!(d.stages.iter().map(|s| s.index).collect::<Vec<_>>(), vec![0, 2, 3, 4]);
        let (p, _) = config.switch_packet(d.stages[3]).unwrap();
        assert_eq!(p[2], 4); assert_eq!(p[3], 8);
        assert_eq!(&p[10..12], &[0, 0]); assert_eq!(&p[26..28], &[0, 0]);
        assert_eq!(&p[43..46], &raw[43..46]);
    }
    #[test]
    fn newer_catalog_range_is_read_only_until_its_whole_table_write_is_verified() {
        for offset in [8, 24] {
            let mut raw = hex(DPI); raw[offset..offset + 2].copy_from_slice(&32000u16.to_le_bytes());
            let c = Config::new(AJ179_PRO, hex(OPTIONS), raw).unwrap(); let d = c.dpi();
            assert_eq!(d.current, Some(1000)); assert!(!d.stage_writable);
            assert!(c.switch_packet(d.stages[4]).is_none());
            raw[offset..offset + 2].copy_from_slice(&32050u16.to_le_bytes());
            assert!(Config::new(AJ179_PRO, hex(OPTIONS), raw).is_none());
        }
    }
    #[test]
    fn switching_preserves_all_slots_colors_reserved_bytes_and_separate_axes() {
        let mut config = captured_config();
        config.raw[4] = 0x33; config.raw[5] = 0xaa;
        config.raw[24..26].copy_from_slice(&1600u16.to_le_bytes());
        // Duplicate X values retain their original stage indices.
        config.raw[10..12].copy_from_slice(&600u16.to_le_bytes());
        let d = config.dpi(); assert_eq!(d.stages[0].value, d.stages[1].value);
        assert_eq!(d.stage_y[&0], 1600);
        let (p, expected) = config.switch_packet(d.stages[4]).unwrap();
        for n in 0..64 {
            if ![0, 2, 7].contains(&n) { assert_eq!(p[n], config.raw[n], "byte {n}"); }
            if n != 2 { assert_eq!(expected.raw[n], config.raw[n]); }
        }
        assert_eq!(expected.raw[3], 8); assert_eq!(expected.options, config.options);
        assert_eq!(p[2], 4); assert_eq!(p[..8].iter().fold(0u8, |sum, b| sum.wrapping_add(*b)), 255);
        assert!(config.switch_packet(DpiStage { index: 5, value: 0 }).is_none());
        assert!(config.switch_packet(DpiStage { index: 4, value: 800 }).is_none());
        let mut changed = config.clone(); changed.raw[63] ^= 1; assert_ne!(config.token(), changed.token());
        let mut changed = config.clone(); changed.options[20] ^= 1; assert_ne!(config.token(), changed.token());
    }
    #[test]
    fn stage_write_is_sent_once_and_confirmed_even_after_lost_completion() {
        for is_wireless in [false, true] { for lost in [false, true] {
            let mut s = session(is_wireless); s.channel.lose_completion = lost;
            choose_last(&mut s).unwrap();
            assert_eq!(s.channel.dpi[2], 4);
            assert_eq!(s.channel.sent.iter().filter(|p| p[0] == 0x54).count(), 1);
            assert_eq!(s.channel.options, hex(OPTIONS));
        } }
    }
    #[test]
    fn stale_identity_profile_token_or_table_prevents_the_only_settings_write() {
        for fault in 0..5 {
            let mut s = session(true);
            match fault {
                0 => s.channel.id[1..5].copy_from_slice(&2285u32.to_le_bytes()),
                1 => { s.channel.options[8] = 2; s.channel.dpi[1] = 2; }
                2 => s.channel.dpi[2] = 1,
                3 => s.channel.dpi[63] ^= 1,
                _ => s.channel.change_options = true,
            }
            assert!(choose_last(&mut s).is_err());
            assert!(s.channel.sent.iter().all(|p| p[0] != 0x54));
        }
    }
    #[test]
    fn unchanged_selection_needs_no_write_and_busy_receiver_has_a_bound() {
        let mut s = session(true); let config = captured_config();
        s.switch(&s.target(Some(AJ179_PRO)), config.dpi().stages[2], config.token()).unwrap();
        assert!(s.channel.sent.iter().all(|p| p[0] != 0x54));
        s.channel.sent.clear(); s.channel.status[5] = 0;
        assert!(s.query(0x8f, 0).is_none());
        assert_eq!(s.channel.sent.iter().filter(|p| p[0] == 0xf7).count(), 8);
        assert!(s.channel.sent.iter().all(|p| p[0] != 0x8f));
    }
    #[test]
    fn no_effect_neighbor_changes_and_replacement_never_report_write_success() {
        for fault in 0..3 {
            let mut s = session(true);
            match fault { 0 => s.channel.apply = false, 1 => s.channel.corrupt_neighbor = true, _ => s.channel.swap_identity = true }
            assert!(choose_last(&mut s).is_err());
            assert_eq!(s.channel.sent.iter().filter(|p| p[0] == 0x54).count(), 1);
        }
    }
}
