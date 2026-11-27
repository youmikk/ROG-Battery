//! Dareu Jm mice. Shared receiver PIDs do not identify the paired sensor.
//! Protocol evidence and hardware limits are recorded in PROTOCOLS.md.
use super::*;
use crate::{device::{DpiStage, StageContext}, hid::HidDevice};
use std::{hash::{BuildHasher, Hasher}, time::{Duration, Instant}};

pub struct Dareu;
const LENGTH: usize = 17;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";
const CHANGED: &str = "设备档位或配置已变化，请刷新后重试";

fn management(i: &HidInterface) -> bool {
    // The official Jm selector uses FF02:2. No service collection is borrowed
    // from another USB interface or another receiver with the same VID/PID.
    i.vendor_id == 0x260d && matches!(i.product_id, 0x1114 | 0x1117 | 0x1175)
        && i.usage_page == 0xff02 && i.usage == 2
}

fn control(i: &HidInterface) -> bool {
    management(i) && i.input_len == LENGTH as u16 && i.output_len == LENGTH as u16
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|i| control(i) && interfaces.iter().filter(|other|
        management(other) && channel::collection_group(i) == channel::collection_group(other)).count() == 1).collect()
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    control(i) && descriptor::Descriptor::open(api, i)
        .is_some_and(|caps| caps.only_input_output_report(i, 8, LENGTH as u16))
}

fn candidate(i: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::Dareu, i, 0);
    target.family = DeviceFamily::Mouse;
    target.name = if i.product_id == 0x1117 { "DAREU A950 PRO Mg" } else { "DAREU 2.4G Mouse" }.into();
    target
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity { cid: u8, mid: u8, kind: u8 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Model { name: &'static str, min: u16, max: u16, legacy: bool }

fn model(pid: u16, id: Identity) -> Option<Model> {
    let (name, min, max, legacy) = match (pid, id.cid, id.mid) {
        (0x1114, 10, 1) => ("A950 PRO", 100, 26000, true),
        (0x1114 | 0x1117, 10, 3) => ("A950 PRO Mg", 100, 26000, true),
        (0x1114, 10, 4) => ("A950G", 100, 26000, true),
        (0x1114, 10, 39) => ("A950G", 100, 30000, true),
        (0x1175, 10 | 4, 9) => ("A950 AIR", 100, 26000, true),
        (0x1175, 10, 25) => ("A950 GM", 50, 26000, true),
        // PAW3950 uses another high-range encoding. Identity and battery only.
        (0x1175, 10, 37) => ("LM180T", 100, 42000, false),
        _ => return None,
    };
    Some(Model { name, min, max, legacy })
}

fn sum(bytes: &[u8]) -> u8 { bytes.iter().fold(0u8, |s, b| s.wrapping_add(*b)) }
fn pair(value: u8) -> [u8; 2] { [value, 0x55u8.wrapping_sub(value)] }

fn packet(command: u8, address: u16, count: u8, data: &[u8]) -> Option<[u8; LENGTH]> {
    let valid = match command {
        1 => address == 0 && count == 8 && data.len() == 4,
        3 | 4 | 0x0e => address == 0 && count == 0 && data.is_empty(),
        8 => (1..=10).contains(&count) && data.is_empty(),
        // The only write owned by this adapter is the active-stage pair.
        7 => address == 4 && count == 2 && data.len() == 2 && data[0] < 5 && sum(data) == 0x55,
        _ => false,
    };
    if !valid { return None; }
    let mut p = [0; LENGTH]; p[0] = 8; p[1] = command;
    p[3..5].copy_from_slice(&address.to_be_bytes()); p[5] = count;
    p[6..6 + data.len()].copy_from_slice(data);
    p[16] = 0x55u8.wrapping_sub(sum(&p[..16])); Some(p)
}

fn matches_reply(p: &[u8], r: &[u8]) -> bool {
    if p.len() != LENGTH || r.len() != LENGTH || r[0] != 8 || r[1] != p[1] { return false; }
    // Incoming checksums and identity challenge echo are not documented.
    // A write echo is only a transport hint; fresh config readback is mandatory.
    if r == p && p[1] != 7 { return false; }
    match p[1] {
        1 => !matches!(r[10], 0 | 255) && !matches!(r[11], 0 | 255),
        3 => r[6] == 1,
        4 => r[5] == 2 && r[6] <= 100,
        0x0e => r[6] < 4,
        7 | 8 => p[3..6] == r[3..6],
        _ => false,
    }
}

fn battery(r: &[u8]) -> Option<Battery> {
    let request = packet(4, 0, 0, &[])?;
    if !matches_reply(&request, r) { return None; }
    // The current official Jm energy pages display both 1 and 2 as charging.
    // Keep enum 2 unknown: another published driver describes it as full.
    let charging = match r[7] { 0 => Some(false), 1 => Some(true), _ => None };
    Some(Battery { percent: Some(r[6]), estimated_percent: None, charging,
        voltage_mv: None, approximate: false, level: None })
}

fn challenge() -> [u8; 4] {
    // A fresh vendor-defined challenge, not a response transaction identifier.
    let mut hash = std::collections::hash_map::RandomState::new().build_hasher();
    hash.write_u32(std::process::id());
    (hash.finish() as u32).to_le_bytes()
}

trait Channel {
    fn exchange(&mut self, p: &[u8], timeout: u32, accept: &mut dyn FnMut(&[u8]) -> bool) -> Option<Vec<u8>>;
}
struct Transport(HidDevice);
impl Channel for Transport {
    fn exchange(&mut self, p: &[u8], timeout: u32, accept: &mut dyn FnMut(&[u8]) -> bool) -> Option<Vec<u8>> {
        self.0.transact(p, timeout, accept)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Config { identity: Identity, profile: u8, header: [u8; 4], records: Vec<[u8; 4]>, model: Model }
impl Config {
    fn axis(&self, low: u8, high: u8) -> Option<u16> {
        let offset = match high { 0 => 0, 0x44 => 256, 0x88 => 512, _ => return None };
        let dpi = (offset + u16::from(low) + 1) * 50;
        (self.model.min..=self.model.max).contains(&dpi).then_some(dpi)
    }
    fn axes(&self, index: usize) -> Option<(u16, u16)> {
        let r = self.records.get(index)?;
        (sum(r) == 0x55).then_some(())?;
        Some((self.axis(r[0], r[2])?, self.axis(r[1], r[2])?))
    }
    fn valid(&self) -> bool {
        self.model.legacy && self.profile < 4 && (1..=5).contains(&self.header[0])
            && self.header[2] < self.header[0] && sum(&self.header[..2]) == 0x55
            && sum(&self.header[2..]) == 0x55 && self.records.len() == usize::from(self.header[0])
            && (0..self.records.len()).all(|n| self.axes(n).is_some())
    }
    fn token(&self) -> u64 {
        let mut h = 0xcbf29ce484222325u64;
        for b in [self.identity.cid, self.identity.mid, self.identity.kind, self.profile].iter()
            .chain(&self.header).chain(self.records.iter().flatten()) {
            h = (h ^ u64::from(*b)).wrapping_mul(0x100000001b3);
        }
        h
    }
    fn dpi(&self) -> Dpi {
        let mut dpi = Dpi { active_stage: Some(self.header[2]), profile: Some(u16::from(self.profile)),
            configuration_token: Some(self.token()), stage_writable: true, ..Default::default() };
        for index in 0..self.records.len() {
            if let Some((x, y)) = self.axes(index) {
                dpi.stages.push(DpiStage { index: index as u8, value: x });
                dpi.stage_y.insert(index as u8, y);
                if index == usize::from(self.header[2]) { dpi.current = Some(x); dpi.current_y = Some(y); }
            }
        }
        dpi
    }
    fn same_tables(&self, other: &Self) -> bool {
        self.identity == other.identity && self.profile == other.profile && self.model == other.model
            && self.header[..2] == other.header[..2] && self.records == other.records
    }
}

struct Session<C> { channel: C, interface: HidInterface, deadline: Instant }
impl<C: Channel> Session<C> {
    fn new(channel: C, i: &HidInterface) -> Self {
        Self { channel, interface: i.clone(), deadline: Instant::now() + Duration::from_millis(3500) }
    }
    fn renew(&mut self) { self.deadline = Instant::now() + Duration::from_millis(3500); }
    fn query(&mut self, command: u8, address: u16, count: u8, data: &[u8]) -> Result<Vec<u8>, String> {
        let p = packet(command, address, count, data).ok_or(UNAVAILABLE)?;
        let started = Instant::now();
        let remaining = self.deadline.checked_duration_since(started).ok_or(UNAVAILABLE)?;
        let timeout = remaining.as_millis().min(800) as u32;
        if timeout == 0 { return Err(UNAVAILABLE.into()); }
        let response = self.channel.exchange(&p, timeout, &mut |r| matches_reply(&p, r))
            .filter(|r| matches_reply(&p, r));
        trace::record(&self.interface, BackendId::Dareu, if command == 7 { "set-stage" } else { "query" },
            address, command, p.len(), response.as_ref().map_or(0, Vec::len), started,
            if response.is_some() { trace::Outcome::Ok } else { trace::Outcome::NoReply });
        response.ok_or_else(|| UNAVAILABLE.into())
    }
    fn active(&mut self) -> Result<(), String> { self.query(3, 0, 0, &[]).map(|_| ()) }
    fn identify(&mut self) -> Result<Identity, String> {
        let r = self.query(1, 0, 8, &challenge())?;
        Ok(Identity { cid: r[10], mid: r[11], kind: r[12] })
    }
    fn profile(&mut self) -> Result<u8, String> { self.query(0x0e, 0, 0, &[]).map(|r| r[6]) }
    fn read(&mut self, address: u16, count: u8) -> Result<Vec<u8>, String> {
        self.query(8, address, count, &[]).map(|r| r[6..6 + usize::from(count)].to_vec())
    }
    fn config(&mut self, expected: Identity) -> Result<Config, String> {
        let model = model(self.interface.product_id, expected).filter(|m| m.legacy).ok_or(CHANGED)?;
        if self.identify()? != expected { return Err(CHANGED.into()); }
        let profile = self.profile()?;
        let header: [u8; 4] = self.read(2, 4)?.try_into().map_err(|_| UNAVAILABLE)?;
        if !(1..=5).contains(&header[0]) || header[2] >= header[0]
            || sum(&header[..2]) != 0x55 || sum(&header[2..]) != 0x55 { return Err(UNAVAILABLE.into()); }
        let mut records = Vec::new();
        for n in 0..header[0] { records.push(self.read(12 + u16::from(n) * 4, 4)?.try_into().map_err(|_| UNAVAILABLE)?); }
        let config = Config { identity: expected, profile, header, records, model };
        if !config.valid() { return Err(UNAVAILABLE.into()); }
        // Verify the whole table in reverse order, not just the active value.
        for n in (0..header[0]).rev() {
            if self.read(12 + u16::from(n) * 4, 4)? != config.records[usize::from(n)] { return Err(CHANGED.into()); }
        }
        if self.read(2, 4)? != header || self.profile()? != profile || self.identify()? != expected {
            return Err(CHANGED.into());
        }
        Ok(config)
    }
    fn target(&self, id: Option<Identity>) -> Target {
        let mut target = candidate(&self.interface);
        if let Some(id) = id {
            target.key = crate::device::stable_key(&format!("{}:cidmid:{}:{}", target.key, id.cid, id.mid));
            target.verified_identity = true;
            if let Some(m) = model(self.interface.product_id, id) { target.name = format!("DAREU {}", m.name); }
        }
        target
    }
    #[cfg(test)]
    fn snapshot(&mut self) -> Result<Snapshot, String> {
        self.snapshot_with_detail(true)
    }
    fn snapshot_with_detail(&mut self, detail: bool) -> Result<Snapshot, String> {
        self.active()?;
        let mut identity = self.identify().ok();
        let battery = self.query(4, 0, 0, &[]).ok().and_then(|r| battery(&r));
        let battery_at = battery.as_ref().map(|_| Instant::now());
        if let Some(before) = identity {
            match self.identify() {
                Ok(after) if after != before => return Err(CHANGED.into()),
                Err(_) => identity = None,
                _ => (),
            }
        }
        let dpi = if detail {
            self.renew();
            identity.and_then(|id| self.config(id).ok()).map(|c| c.dpi()).unwrap_or_default()
        } else { Dpi::default() };
        if battery.is_none() && dpi.current.is_none() { return Err(UNAVAILABLE.into()); }
        Ok(Snapshot { target: self.target(identity), battery, battery_at, battery_unavailable: false,
            connected: true, dpi, polling_rate: Default::default() })
    }
    fn switch(&mut self, target: &Target, stage: DpiStage, token: u64) -> Result<(), String> {
        self.active()?;
        let identity = self.identify()?;
        if !target.verified_identity || self.target(Some(identity)).key != target.key { return Err(CHANGED.into()); }
        self.renew();
        let before = self.config(identity)?;
        if before.token() != token || before.axes(usize::from(stage.index)).map(|v| v.0) != Some(stage.value) {
            return Err(CHANGED.into());
        }
        if before.header[2] == stage.index { return Ok(()); }
        self.active()?;
        if self.identify()? != identity || self.profile()? != before.profile || self.read(2, 4)? != before.header {
            return Err(CHANGED.into());
        }
        // Send at most once. A missing ACK can still mean the write was applied.
        let _ = self.query(7, 4, 2, &pair(stage.index));
        self.renew();
        self.active()?;
        let after = self.config(identity)?;
        if after.header[2] != stage.index || !before.same_tables(&after) {
            return Err("无法确认 DPI 档位切换".into());
        }
        Ok(())
    }
}

fn with_session<T>(api: &HidApi, target: &Target, f: impl FnOnce(&mut Session<Transport>) -> Result<T, String>) -> Result<T, String> {
    let i = target.hid()?;
    if target.backend != BackendId::Dareu || !proven(api, i) { return Err(UNAVAILABLE.into()); }
    let lock = channel::lock(&channel::collection_group(i));
    let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
    f(&mut Session::new(Transport(HidDevice::open(i)?), i))
}

impl DeviceBackend for Dareu {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i)).map(candidate).collect())
    }
    fn read(&self, api: &HidApi, target: &Target, detail: bool) -> Result<Snapshot, String> {
        with_session(api, target, |s| s.snapshot_with_detail(detail))
    }
    fn set_dpi_stage(&self, api: &HidApi, target: &Target, stage: DpiStage, context: StageContext) -> Result<(), String> {
        let StageContext::Configuration(token) = context else { return Err(CHANGED.into()); };
        with_session(api, target, |s| s.switch(target, stage, token))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn interface(pid: u16, instance: &str) -> HidInterface {
        HidInterface { path: format!(r"\\?\hid#vid_260d&pid_{pid:04x}&mi_00&col03#8&{instance}&0003#{{hid}}"),
            vendor_id: 0x260d, product_id: pid, usage_page: 0xff02, usage: 2,
            input_len: 17, output_len: 17, feature_len: 0, product: "2.4G Wireless Mouse".into(), serial: "shared".into() }
    }

    // Synthetic transcripts built from the documented fields. These are not
    // hardware captures and deliberately have no invented reply checksum.
    struct Reports {
        id: Identity, profile: u8, memory: BTreeMap<u16, u8>, commands: Vec<Vec<u8>>,
        writes: Vec<Vec<u8>>, drop_command: Option<u8>, accept_write: bool, lost_ack: bool,
        wrong_address: bool, wrong_count: bool, read_count: usize, change_after_read: Option<usize>,
        replace_after_write: bool, replace_at_battery: bool, corrupt_after_write: bool,
        profile_after_write: bool, online: bool,
    }
    impl Reports {
        fn new() -> Self {
            let mut s = Self { id: Identity { cid: 10, mid: 3, kind: 1 }, profile: 0,
                memory: (0..100).map(|n| (n, 0x9d)).collect(), commands: Vec::new(), writes: Vec::new(),
                drop_command: None, accept_write: true, lost_ack: false, wrong_address: false,
                wrong_count: false, read_count: 0, change_after_read: None,
                replace_after_write: false, replace_at_battery: false, corrupt_after_write: false,
                profile_after_write: false, online: true };
            s.put(2, &pair(5)); s.put(4, &pair(2));
            for (index, dpi) in [400, 800, 1600, 12850, 26000].into_iter().enumerate() { s.stage(index as u8, dpi); }
            s
        }
        fn put(&mut self, address: u16, bytes: &[u8]) {
            for (offset, b) in bytes.iter().enumerate() { self.memory.insert(address + offset as u16, *b); }
        }
        fn bytes(&self, address: u16, count: u8) -> Vec<u8> {
            (0..count).map(|n| self.memory[&(address + u16::from(n))]).collect()
        }
        fn stage(&mut self, index: u8, dpi: u16) {
            let code = dpi / 50 - 1;
            let high = match code / 256 { 0 => 0, 1 => 0x44, _ => 0x88 };
            let mut row = [code as u8, code as u8, high, 0]; row[3] = 0x55u8.wrapping_sub(sum(&row[..3]));
            self.put(12 + u16::from(index) * 4, &row);
        }
    }
    impl Channel for Reports {
        fn exchange(&mut self, p: &[u8], timeout: u32, accept: &mut dyn FnMut(&[u8]) -> bool) -> Option<Vec<u8>> {
            assert!((1..=800).contains(&timeout)); assert_eq!(sum(p), 0x55); assert_eq!(p.len(), 17);
            self.commands.push(p.to_vec());
            if p[1] != 7 { assert!(!accept(p)); }
            let mut r = vec![0; LENGTH]; r[0] = 8; r[1] = p[1]; r[16] = 0xe7;
            match p[1] {
                1 => { r[10] = self.id.cid; r[11] = self.id.mid; r[12] = self.id.kind; }
                3 => r[6] = u8::from(self.online),
                4 => {
                    r[5] = 2; r[6] = 76; r[7] = 1;
                    if self.replace_at_battery { self.id.mid = 4; }
                }
                0x0e => r[6] = self.profile,
                8 => {
                    let address = u16::from_be_bytes([p[3], p[4]]);
                    r[3..6].copy_from_slice(&p[3..6]);
                    r[6..6 + usize::from(p[5])].copy_from_slice(&self.bytes(address, p[5]));
                    self.read_count += 1;
                    if self.change_after_read == Some(self.read_count) { self.stage(0, 450); }
                    if self.wrong_address { r[4] ^= 1; }
                    if self.wrong_count { r[5] ^= 1; }
                }
                7 => {
                    assert_eq!(&p[3..6], &[0, 4, 2]);
                    self.writes.push(p.to_vec());
                    if self.accept_write { self.put(4, &p[6..8]); }
                    if self.replace_after_write { self.id.mid = 4; }
                    if self.corrupt_after_write { self.stage(0, 450); }
                    if self.profile_after_write { self.profile = 1; }
                    if self.lost_ack { return None; }
                    r.copy_from_slice(p); // Echo alone must never mean success.
                }
                _ => panic!("unexpected command"),
            }
            if self.drop_command == Some(p[1]) { return None; }
            accept(&r).then_some(r)
        }
    }
    fn session(pid: u16) -> Session<Reports> { Session::new(Reports::new(), &interface(pid, "receiver-a")) }
    fn ready() -> (Session<Reports>, Target, u64) {
        let mut s = session(0x1114);
        let sample = s.snapshot().unwrap();
        (s, sample.target, sample.dpi.configuration_token.unwrap())
    }

    #[test]
    fn lightweight_snapshot_skips_config_and_later_detail_reads_full_table() {
        let mut session = session(0x1114);
        let lightweight = session.snapshot_with_detail(false).unwrap();
        assert_eq!(lightweight.battery.as_ref().unwrap().percent, Some(76));
        assert!(lightweight.target.verified_identity && lightweight.battery_at.is_some());
        assert_eq!(lightweight.dpi, Dpi::default());
        assert_eq!(session.channel.commands.iter().map(|packet| packet[1]).collect::<Vec<_>>(), [3, 1, 4, 1]);
        assert_eq!(session.channel.read_count, 0);
        let detailed = session.snapshot_with_detail(true).unwrap();
        assert_eq!(lightweight.target.key, detailed.target.key);
        assert_eq!(lightweight.battery, detailed.battery);
        assert_eq!(detailed.dpi.stages.len(), 5);
        assert!(detailed.dpi.stage_writable && session.channel.read_count > 0);
        assert!(session.channel.writes.is_empty());
    }

    #[test]
    fn lightweight_snapshot_keeps_identity_checks_and_unverified_devices_readonly() {
        let mut replaced = session(0x1114);
        replaced.channel.replace_at_battery = true;
        assert!(replaced.snapshot_with_detail(false).is_err());
        let mut unknown = session(0x1114);
        unknown.channel.drop_command = Some(1);
        let reading = unknown.snapshot_with_detail(false).unwrap();
        assert!(!reading.target.verified_identity);
        assert_eq!(reading.dpi, Dpi::default());
        assert_eq!(reading.battery.unwrap().percent, Some(76));
        assert_eq!(unknown.channel.read_count, 0);
        assert!(unknown.channel.writes.is_empty() && replaced.channel.writes.is_empty());
    }

    #[test]
    fn exact_command_collection_and_report_sizes_do_not_mix_receiver_instances() {
        let a = interface(0x1114, "a"); let b = interface(0x1114, "b");
        assert!(control(&a)); assert_eq!(candidates(&[a.clone(), b.clone()]).len(), 2);
        assert_ne!(candidate(&a).key, candidate(&b).key);
        for (vid, pid, page, usage, input, output) in [
            (0x3554, 0x1114, 0xff02, 2, 17, 17), (0x260d, 0x1112, 0xff02, 2, 17, 17),
            (0x260d, 0x1114, 0xff05, 0, 17, 17), (0x260d, 0x1114, 0xff02, 1, 17, 17),
            (0x260d, 0x1114, 0xff02, 2, 16, 17), (0x260d, 0x1114, 0xff02, 2, 17, 18),
        ] {
            let mut i = a.clone(); i.vendor_id = vid; i.product_id = pid; i.usage_page = page;
            i.usage = usage; i.input_len = input; i.output_len = output; assert!(!control(&i));
        }
        let mut sibling = a.clone();
        sibling.path = sibling.path.replace("col03", "col04").replace("&0003", "&0004");
        sibling.input_len = 0;
        assert!(candidates(&[a, sibling]).is_empty());
    }

    #[test]
    fn framing_rejects_echoes_wrong_addresses_lengths_and_unowned_writes() {
        assert_eq!(packet(3, 0, 0, &[]).unwrap()[16], 0x4a);
        assert_eq!(packet(4, 0, 0, &[]).unwrap()[16], 0x49);
        for (command, address, count, data) in [(9, 0, 0, vec![]), (7, 0, 2, pair(1).to_vec()),
            (7, 4, 2, pair(5).to_vec()), (7, 4, 2, vec![1, 0]), (8, 0, 11, vec![])] {
            assert!(packet(command, address, count, &data).is_none());
        }
        let p = packet(8, 12, 4, &[]).unwrap(); let mut r = p; r[6..10].copy_from_slice(&[7, 7, 0, 71]);
        assert!(matches_reply(&p, &r));
        for len in 0..LENGTH { assert!(!matches_reply(&p, &r[..len])); }
        let mut long = r.to_vec(); long.push(0); assert!(!matches_reply(&p, &long));
        for offset in [0, 1, 3, 4, 5] {
            let mut bad = r; bad[offset] ^= 1; assert!(!matches_reply(&p, &bad));
        }
        let mut s = session(0x1114); s.channel.wrong_address = true;
        assert!(s.config(s.channel.id).is_err());
        s.channel.wrong_address = false; s.channel.wrong_count = true;
        assert!(s.config(s.channel.id).is_err());
    }

    #[test]
    fn battery_keeps_exact_zero_full_and_unknown_charge_without_inventing_voltage() {
        let p = packet(4, 0, 0, &[]).unwrap(); assert!(battery(&p).is_none());
        let mut r = p; r[5] = 2;
        for percent in [0, 76, 100] {
            for (state, expected) in [(0, Some(false)), (1, Some(true)), (2, None), (255, None)] {
                r[6] = percent; r[7] = state; r[8..].fill(0xff);
                let b = battery(&r).unwrap();
                assert_eq!((b.percent, b.charging, b.voltage_mv), (Some(percent), expected, None));
            }
        }
        r[6] = 101; assert!(battery(&r).is_none());
        r[6] = 76; r[5] = 0; assert!(battery(&r).is_none());
    }

    #[test]
    fn paired_model_selects_range_and_unknown_or_3950_models_cannot_read_settings() {
        for (pid, cid, mid, name, min, max) in [
            (0x1114, 10, 1, "A950 PRO", 100, 26000), (0x1114, 10, 3, "A950 PRO Mg", 100, 26000),
            (0x1114, 10, 4, "A950G", 100, 26000), (0x1114, 10, 39, "A950G", 100, 30000),
            (0x1175, 10, 9, "A950 AIR", 100, 26000), (0x1175, 4, 9, "A950 AIR", 100, 26000),
            (0x1175, 10, 25, "A950 GM", 50, 26000),
        ] {
            let mut s = session(pid); s.channel.id = Identity { cid, mid, kind: 2 };
            s.channel.stage(0, min); s.channel.stage(4, max);
            let r = s.snapshot().unwrap();
            assert_eq!(r.target.name, format!("DAREU {name}")); assert!(r.dpi.stage_writable);
            assert_eq!(r.dpi.stages[0].value, min); assert_eq!(r.dpi.stages[4].value, max);
        }
        for (pid, mid, name) in [(0x1175, 37, "DAREU LM180T"), (0x1175, 3, "DAREU 2.4G Mouse"),
            (0x1114, 25, "DAREU 2.4G Mouse")] {
            let mut s = session(pid); s.channel.id.mid = mid;
            let r = s.snapshot().unwrap(); assert_eq!(r.target.name, name);
            assert_eq!(r.battery.unwrap().percent, Some(76)); assert!(!r.dpi.stage_writable);
            assert!(s.channel.commands.iter().all(|p| matches!(p[1], 1 | 3 | 4)));
        }
    }

    #[test]
    fn actual_five_stages_and_configuration_token_preserve_raw_order_and_axes() {
        let mut s = session(0x1114); s.channel.stage(0, 800); s.channel.stage(1, 400);
        s.channel.stage(2, 800);
        let c = s.config(s.channel.id).unwrap(); let dpi = c.dpi();
        assert_eq!(dpi.stages.iter().map(|s| s.value).collect::<Vec<_>>(), [800, 400, 800, 12850, 26000]);
        assert_eq!((dpi.current, dpi.current_y, dpi.active_stage, dpi.profile), (Some(800), Some(800), Some(2), Some(0)));
        assert!(dpi.stage_writable && !dpi.writable && !dpi.stage_editable);
        let token = c.token(); s.channel.stage(4, 25950);
        assert_ne!(s.config(s.channel.id).unwrap().token(), token);
        assert!(s.channel.writes.is_empty());
    }

    #[test]
    fn invalid_config_or_profile_does_not_drop_a_valid_battery_sample() {
        for corrupt in 0..7 {
            let mut s = session(0x1114);
            match corrupt {
                0 => s.channel.put(2, &pair(0)),
                1 => s.channel.put(2, &pair(6)),
                2 => s.channel.put(4, &pair(5)),
                3 => s.channel.put(15, &[0]),
                4 => s.channel.stage(0, 50),
                5 => s.channel.profile = 4,
                _ => s.channel.drop_command = Some(8),
            }
            let r = s.snapshot().unwrap(); assert_eq!(r.battery.unwrap().percent, Some(76));
            assert!(r.battery_at.is_some()); assert!(!r.dpi.stage_writable); assert!(r.dpi.stages.is_empty());
        }
        let mut s = session(0x1114); s.channel.drop_command = Some(1);
        let r = s.snapshot().unwrap(); assert!(!r.target.verified_identity); assert!(!r.dpi.stage_writable);
        assert_eq!(r.target.name, "DAREU 2.4G Mouse"); assert!(r.battery.is_some());
        let mut s = session(0x1114); s.channel.drop_command = Some(4);
        let r = s.snapshot().unwrap(); assert!(r.battery.is_none() && r.battery_at.is_none());
        assert!(r.dpi.stage_writable);
    }

    #[test]
    fn table_change_during_scan_and_pairing_change_during_battery_are_rejected() {
        let mut s = session(0x1114); s.channel.change_after_read = Some(2);
        let r = s.snapshot().unwrap(); assert!(r.battery.is_some()); assert!(r.dpi.stages.is_empty());
        let mut s = session(0x1114); s.channel.replace_at_battery = true;
        assert!(s.snapshot().is_err()); assert!(s.channel.writes.is_empty());
    }

    #[test]
    fn switching_only_writes_active_pair_and_readback_handles_lost_ack_without_resend() {
        for lost in [false, true] {
            let (mut s, target, token) = ready(); let original = s.channel.memory.clone(); s.channel.lost_ack = lost;
            s.switch(&target, DpiStage { index: 4, value: 26000 }, token).unwrap();
            assert_eq!(s.channel.writes.len(), 1);
            assert_eq!(&s.channel.writes[0][3..8], &[0, 4, 2, 4, 81]);
            for (address, value) in original { if ![4, 5].contains(&address) { assert_eq!(s.channel.memory[&address], value); } }
        }
    }

    #[test]
    fn echoes_ignored_writes_changed_tables_profiles_and_replacement_never_report_success() {
        for failure in 0..4 {
            let (mut s, target, token) = ready();
            match failure { 0 => s.channel.accept_write = false, 1 => s.channel.replace_after_write = true,
                2 => s.channel.corrupt_after_write = true, _ => s.channel.profile_after_write = true }
            assert!(s.switch(&target, DpiStage { index: 4, value: 26000 }, token).is_err());
            assert_eq!(s.channel.writes.len(), 1);
        }
    }

    #[test]
    fn stale_context_or_out_of_range_stage_prevents_every_write() {
        for failure in 0..8 {
            let (mut s, mut target, token) = ready();
            let mut stage = DpiStage { index: 4, value: 26000 };
            match failure {
                0 => s.channel.profile = 1, 1 => s.channel.stage(0, 450), 2 => s.channel.put(4, &pair(1)),
                3 => s.channel.id.mid = 4, 4 => stage.index = 5, 5 => stage.value = 800,
                6 => target.verified_identity = false, _ => s.channel.id.kind ^= 1,
            }
            assert!(s.switch(&target, stage, token).is_err()); assert!(s.channel.writes.is_empty());
        }
        let (mut s, target, token) = ready();
        s.switch(&target, DpiStage { index: 2, value: 1600 }, token).unwrap();
        assert!(s.channel.writes.is_empty());
    }

    #[test]
    fn sleep_silence_and_session_deadline_do_not_create_new_samples_and_wake_recovers() {
        let mut s = session(0x1114); let first = s.snapshot().unwrap();
        s.channel.online = false; assert!(s.snapshot().is_err());
        s.channel.online = true; s.renew();
        let next = s.snapshot().unwrap(); assert!(next.battery_at > first.battery_at);
        let sent = s.channel.commands.len(); s.deadline = Instant::now();
        assert!(s.active().is_err()); assert_eq!(s.channel.commands.len(), sent);
    }
}
