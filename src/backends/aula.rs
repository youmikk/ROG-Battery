//! SC380 PRO receiver. Other AULA mice and keyboards use different protocols.
//! Model/transport evidence and unverified hardware limits: PROTOCOLS.md.
use super::feature::FeatureChannel;
use super::*;
use crate::device::{DpiStage, PollingRate, StageContext};
use std::time::{Duration, Instant};

pub struct Aula;
const LENGTH: usize = 520;
const MODEL: [u8; 6] = [4, 0, 0, 0, 0, 0x2e];
const UNAVAILABLE: &str = "设备无有效状态回报";
const CHANGED: &str = "设备档位或配置已变化，请刷新后重试";
type Packet = [u8; LENGTH];

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x258a && i.product_id == 0x0138 && i.usage_page == 0xff02 && i.usage == 2
}
fn control(i: &HidInterface) -> bool {
    management(i) && i.feature_len == LENGTH as u16
}
fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces
        .iter()
        .filter(|i| {
            control(i)
                && interfaces
                    .iter()
                    .filter(|other| {
                        management(other)
                            && channel::collection_group(i) == channel::collection_group(other)
                    })
                    .count()
                    == 1
        })
        .collect()
}
fn proven(api: &HidApi, i: &HidInterface) -> bool {
    control(i)
        && descriptor::Descriptor::open(api, i)
            .is_some_and(|d| d.feature_reports(i, LENGTH as u16, &[9], &[9]))
}
fn candidate(i: &HidInterface) -> Target {
    let mut t = Target::new(BackendId::Aula, i, 0);
    t.name = "2.4G Mouse".into();
    t.family = DeviceFamily::Mouse;
    t
}
fn sum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |s, b| s.wrapping_add(*b))
}
fn request(command: u8) -> Option<Packet> {
    let (argument, length): (u8, u16) = match command {
        5 => (1, 6),
        7 => (0, 1),
        9 => (1, 1),
        0x44 => (0, 400),
        _ => return None,
    };
    let mut p = [0; LENGTH];
    p[0] = 9;
    p[2] = command;
    p[3] = argument;
    p[4] = 1;
    p[6..8].copy_from_slice(&length.to_le_bytes());
    p[1] = sum(&p[2..]);
    Some(p)
}
fn reply<'a>(r: &'a [u8], p: &Packet) -> Option<&'a [u8]> {
    let end = 8 + usize::from(u16::from_le_bytes([p[6], p[7]]));
    if r.len() != LENGTH || r[0] != 9 || r[2..8] != p[2..8] || r[..end] == p[..end] {
        return None;
    }
    // Short replies retain the query's header checksum; only settings replies
    // include the data in the sum. Unused trailing bytes prove neither freshness
    // nor a real zero battery/offline response.
    let checksum = if p[2] == 0x44 { sum(&r[2..end]) } else { p[1] };
    (r[1] == checksum).then_some(&r[8..end])
}
fn battery(value: u8) -> Option<Battery> {
    // A zero payload cannot be distinguished from an echoed query on this path.
    (1..=100).contains(&value).then_some(Battery {
        percent: Some(value),
        estimated_percent: None,
        charging: None,
        voltage_mv: None,
        approximate: false,
        level: None,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Config([u8; 400]);
impl Config {
    fn decode(data: &[u8]) -> Option<Self> {
        let c = Self(data.try_into().ok()?);
        let count = c.count();
        let active = c.0[3] >> 4;
        if c.0[0] != 0x64
            || c.0[1] & 0x1f != 0x18
            || !(1..=6).contains(&count)
            || active == 0
            || active > count
            || c.rate().current.is_none()
        {
            return None;
        }
        for n in 0..count {
            c.axes(n)?;
        }
        Some(c)
    }
    fn count(&self) -> u8 {
        self.0[3] & 0xf
    }
    fn active(&self) -> u8 {
        (self.0[3] >> 4) - 1
    }
    fn axes(&self, index: u8) -> Option<(u16, u16)> {
        if index >= self.count() {
            return None;
        }
        let read = |base: usize| {
            let offset = base + usize::from(index) * 2;
            let word = u16::from_le_bytes([self.0[offset], self.0[offset + 1]]);
            (word < 120).then(|| (word + 1) * 100)
        };
        Some((read(7)?, read(27)?))
    }
    fn token(&self) -> u64 {
        let mut h = 0xcbf29ce484222325u64;
        for b in MODEL.iter().chain(&self.0) {
            h = (h ^ u64::from(*b)).wrapping_mul(0x100000001b3);
        }
        h
    }
    fn dpi(&self) -> Dpi {
        let mut d = Dpi {
            active_stage: Some(self.active()),
            stage_writable: true,
            configuration_token: Some(self.token()),
            ..Default::default()
        };
        for index in 0..self.count() {
            if let Some((x, y)) = self.axes(index) {
                d.stages.push(DpiStage { index, value: x });
                d.stage_y.insert(index, y);
                if index == self.active() {
                    d.current = Some(x);
                    d.current_y = Some(y);
                }
            }
        }
        d
    }
    fn rate(&self) -> PollingRate {
        let current = match self.0[2] & 0xf {
            1 => Some(125),
            2 => Some(250),
            3 => Some(500),
            4 => Some(1000),
            _ => None,
        };
        PollingRate {
            current,
            ..Default::default()
        }
    }
    fn switch(&self, stage: DpiStage) -> Option<Self> {
        if self.axes(stage.index)?.0 != stage.value {
            return None;
        }
        let mut next = self.clone();
        next.0[3] = ((stage.index + 1) << 4) | self.count();
        Some(next)
    }
    fn write_packet(&self) -> Packet {
        let mut p = request(0x44).expect("known settings query");
        p[2] = 4;
        p[8..408].copy_from_slice(&self.0);
        p[1] = sum(&p[2..]);
        p
    }
}

trait Channel {
    fn send(&mut self, p: &[u8]) -> Result<(), String>;
    fn receive(&mut self) -> Result<Vec<u8>, String>;
    fn pause(&self, _ms: u64) {}
}
struct Transport(FeatureChannel);
impl Channel for Transport {
    fn send(&mut self, p: &[u8]) -> Result<(), String> {
        self.0.send(p, 150)
    }
    fn receive(&mut self) -> Result<Vec<u8>, String> {
        self.0.receive(9, 150)
    }
    fn pause(&self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}
struct Session<C> {
    channel: C,
    interface: HidInterface,
    deadline: Instant,
}
impl<C: Channel> Session<C> {
    fn new(channel: C, i: &HidInterface) -> Self {
        Self {
            channel,
            interface: i.clone(),
            deadline: Instant::now() + Duration::from_millis(3500),
        }
    }
    fn renew(&mut self) {
        self.deadline = Instant::now() + Duration::from_millis(3500);
    }
    fn query(&mut self, command: u8) -> Option<Vec<u8>> {
        let p = request(command)?;
        let started = Instant::now();
        let mut bytes = 0;
        let result = (|| {
            if started >= self.deadline {
                return None;
            }
            self.channel.send(&p).ok()?;
            self.channel.pause(30);
            let r = self.channel.receive().ok()?;
            bytes = r.len();
            reply(&r, &p).map(<[u8]>::to_vec)
        })();
        trace::record(
            &self.interface,
            BackendId::Aula,
            "query",
            9,
            command,
            LENGTH,
            bytes,
            started,
            if result.is_some() {
                trace::Outcome::Ok
            } else {
                trace::Outcome::NoReply
            },
        );
        result
    }
    fn identify(&mut self) -> Option<()> {
        (self.query(5)? == MODEL).then_some(())
    }
    fn online(&mut self) -> Option<()> {
        (self.query(7)? == [1]).then_some(())
    }
    fn config(&mut self) -> Option<Config> {
        let c = Config::decode(&self.query(0x44)?)?;
        (self.query(0x44)? == c.0).then_some(c)
    }
    fn target(&self) -> Target {
        let mut t = candidate(&self.interface);
        t.key = crate::device::stable_key(&format!("{}:model:04000000002e", t.key));
        t.name = "AULA SC380 PRO".into();
        t.verified_identity = true;
        t
    }
    fn snapshot(&mut self) -> Result<Snapshot, String> {
        self.identify().ok_or(UNAVAILABLE)?;
        self.online().ok_or(UNAVAILABLE)?;
        let battery = self.query(9).and_then(|r| battery(r[0]));
        let battery_at = battery.as_ref().map(|_| Instant::now());
        let config = self.config();
        self.identify().ok_or(CHANGED)?;
        self.online().ok_or(UNAVAILABLE)?;
        if battery.is_none() && config.is_none() {
            return Err(UNAVAILABLE.into());
        }
        Ok(Snapshot {
            target: self.target(),
            battery,
            battery_at,
            battery_unavailable: false,
            connected: true,
            dpi: config.as_ref().map_or_else(Dpi::default, Config::dpi),
            polling_rate: config
                .as_ref()
                .map_or_else(PollingRate::default, Config::rate),
        })
    }
    fn switch(&mut self, target: &Target, stage: DpiStage, token: u64) -> Result<(), String> {
        if !target.verified_identity || target.key != self.target().key {
            return Err(CHANGED.into());
        }
        self.identify().ok_or(CHANGED)?;
        self.online().ok_or(UNAVAILABLE)?;
        let before = self.config().ok_or(UNAVAILABLE)?;
        if before.token() != token {
            return Err(CHANGED.into());
        }
        let expected = before.switch(stage).ok_or(CHANGED)?;
        if before == expected {
            return Ok(());
        }
        self.renew();
        if self.config().as_ref() != Some(&before) {
            return Err(CHANGED.into());
        }
        self.identify().ok_or(CHANGED)?;
        self.online().ok_or(UNAVAILABLE)?;
        let p = expected.write_packet();
        let started = Instant::now();
        let sent = self.channel.send(&p);
        trace::record(
            &self.interface,
            BackendId::Aula,
            "set-stage",
            9,
            4,
            LENGTH,
            0,
            started,
            if sent.is_ok() {
                trace::Outcome::Ok
            } else {
                trace::Outcome::NoReply
            },
        );
        // Sending can apply the write even when its completion is lost. Read
        // back the entire preserved block; never retry or roll it back blindly.
        self.channel.pause(250);
        self.renew();
        self.identify().ok_or(CHANGED)?;
        self.online().ok_or(UNAVAILABLE)?;
        if self.config().as_ref() != Some(&expected) {
            return Err("无法确认 DPI 档位切换".into());
        }
        self.identify().ok_or(CHANGED)?;
        self.online().ok_or(UNAVAILABLE)?;
        Ok(())
    }
}

fn with_session<T>(
    api: &HidApi,
    target: &Target,
    f: impl FnOnce(&mut Session<Transport>) -> Result<T, String>,
) -> Result<T, String> {
    let i = target.hid()?;
    if target.backend != BackendId::Aula || !proven(api, i) {
        return Err(UNAVAILABLE.into());
    }
    let lock = channel::lock(&channel::collection_group(i));
    let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
    f(&mut Session::new(Transport(FeatureChannel::open(i)?), i))
}
impl DeviceBackend for Aula {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces)
            .into_iter()
            .filter(|i| proven(api, i))
            .map(candidate)
            .collect())
    }
    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        with_session(api, target, |s| s.snapshot())
    }
    fn set_dpi_stage(
        &self,
        api: &HidApi,
        target: &Target,
        stage: DpiStage,
        context: StageContext,
    ) -> Result<(), String> {
        let StageContext::Configuration(token) = context else {
            return Err(CHANGED.into());
        };
        with_session(api, target, |s| s.switch(target, stage, token))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(pid: u16, instance: &str) -> HidInterface {
        HidInterface {
            vendor_id: 0x258a,
            product_id: pid,
            usage_page: 0xff02,
            usage: 2,
            feature_len: 520,
            input_len: 0,
            output_len: 0,
            product: "Gaming Mouse".into(),
            serial: "shared".into(),
            path: format!(r"\\?\hid#vid_258a&pid_{pid:04x}&mi_01&col02#8&{instance}&0002#{{hid}}"),
        }
    }
    // Synthetic protocol fixtures, not local hardware captures. The non-DPI
    // bytes deliberately stay nonzero to catch destructive block rewrites.
    fn config() -> Config {
        let mut raw = [0xa5; 400];
        raw[0] = 0x64;
        raw[1] = 0xb8;
        raw[2] = 0xb4;
        raw[3] = 0x26;
        for (n, x) in [100u16, 800, 800, 1600, 12000, 3200]
            .into_iter()
            .enumerate()
        {
            raw[7 + n * 2..9 + n * 2].copy_from_slice(&(x / 100 - 1).to_le_bytes());
            raw[27 + n * 2..29 + n * 2]
                .copy_from_slice(&((if n == 1 { 1600 } else { x }) / 100 - 1).to_le_bytes());
        }
        Config::decode(&raw).unwrap()
    }
    struct Fake {
        raw: Config,
        id: [u8; 6],
        battery: u8,
        online: u8,
        pending: Option<Vec<u8>>,
        sent: Vec<Vec<u8>>,
        echo: bool,
        bad_config: bool,
        skip_battery: bool,
        apply: bool,
        lost_completion: bool,
        corrupt_neighbor: bool,
        replace_after_write: bool,
        sleep_after_write: bool,
        config_reads: usize,
        change_on_read: Option<usize>,
        replace_at_battery: bool,
    }
    impl Fake {
        fn new() -> Self {
            Self {
                raw: config(),
                id: MODEL,
                battery: 63,
                online: 1,
                pending: None,
                sent: Vec::new(),
                echo: false,
                bad_config: false,
                skip_battery: false,
                apply: true,
                lost_completion: false,
                corrupt_neighbor: false,
                replace_after_write: false,
                sleep_after_write: false,
                config_reads: 0,
                change_on_read: None,
                replace_at_battery: false,
            }
        }
        fn writes(&self) -> Vec<&Vec<u8>> {
            self.sent.iter().filter(|p| p[2] == 4).collect()
        }
    }
    impl Channel for Fake {
        fn send(&mut self, p: &[u8]) -> Result<(), String> {
            assert_eq!(p.len(), LENGTH);
            assert_eq!(p[0], 9);
            assert_eq!(p[1], sum(&p[2..]));
            self.sent.push(p.to_vec());
            self.pending = None;
            if self.echo {
                self.pending = Some(p.to_vec());
                return Ok(());
            }
            let mut r = p.to_vec();
            match p[2] {
                5 => r[8..14].copy_from_slice(&self.id),
                7 => r[8] = self.online,
                9 => {
                    if self.skip_battery {
                        return Ok(());
                    }
                    r[8] = self.battery;
                    if self.replace_at_battery {
                        self.id[5] ^= 1;
                    }
                }
                0x44 => {
                    self.config_reads += 1;
                    if self.change_on_read == Some(self.config_reads) {
                        self.raw.0[399] ^= 1;
                    }
                    r[8..408].copy_from_slice(&self.raw.0);
                    r[1] = sum(&r[2..408]);
                    if self.bad_config {
                        r[1] ^= 1;
                    }
                }
                4 => {
                    assert_eq!(&p[2..8], &[4, 0, 1, 0, 0x90, 1]);
                    if self.apply {
                        self.raw = Config::decode(&p[8..408]).unwrap();
                        if self.corrupt_neighbor {
                            self.raw.0[399] ^= 1;
                        }
                    }
                    if self.replace_after_write {
                        self.id[5] ^= 1;
                    }
                    if self.sleep_after_write {
                        self.online = 0;
                    }
                    if self.lost_completion {
                        return Err("completion lost".into());
                    }
                }
                _ => panic!("unexpected command"),
            }
            self.pending = Some(r);
            Ok(())
        }
        fn receive(&mut self) -> Result<Vec<u8>, String> {
            self.pending.take().ok_or("no reply".into())
        }
    }
    fn session() -> Session<Fake> {
        Session::new(Fake::new(), &interface(0x0138, "one"))
    }
    fn switch(s: &mut Session<Fake>) -> Result<(), String> {
        s.switch(
            &s.target(),
            DpiStage {
                index: 4,
                value: 12000,
            },
            config().token(),
        )
    }

    #[test]
    fn exact_receiver_collection_and_capacity_do_not_borrow_sibling_interfaces() {
        let a = interface(0x0138, "one");
        let b = interface(0x0138, "two");
        let mut sibling = a.clone();
        sibling.path = a.path.replace("col02", "col03").replace("&0002#", "&0003#");
        let mut bad_capacity = a.clone();
        bad_capacity.feature_len = 519;
        let mut wrong_usage = a.clone();
        wrong_usage.usage = 1;
        assert_eq!(candidates(&[a.clone(), b.clone()]).len(), 2);
        assert!(candidates(&[a.clone(), sibling]).is_empty());
        assert!(candidates(&[a.clone(), bad_capacity]).is_empty());
        assert_eq!(
            candidates(&[a.clone(), wrong_usage, interface(0x0136, "cable")]).len(),
            1
        );
        assert_ne!(candidate(&a).key, candidate(&b).key);
        assert!(!control(&interface(0x0136, "cable")));
    }
    #[test]
    fn documented_request_headers_and_checksum_scopes_are_distinct() {
        for (command, header) in [
            (5, [9, 13, 5, 1, 1, 0, 6, 0]),
            (7, [9, 9, 7, 0, 1, 0, 1, 0]),
            (9, [9, 12, 9, 1, 1, 0, 1, 0]),
            (0x44, [9, 0xd6, 0x44, 0, 1, 0, 0x90, 1]),
        ] {
            let p = request(command).unwrap();
            assert_eq!(p[..8], header);
            assert!(p[8..].iter().all(|b| *b == 0));
        }
        assert!(request(4).is_none());
        assert!(request(0x87).is_none());
        let p = request(9).unwrap();
        let mut r = p;
        r[8] = 63;
        assert_eq!(reply(&r, &p), Some([63].as_slice()));
        r[1] = sum(&r[2..]);
        assert!(reply(&r, &p).is_none());
        let p = request(0x44).unwrap();
        let mut r = p;
        r[8..408].copy_from_slice(&config().0);
        r[1] = sum(&r[2..408]);
        assert_eq!(reply(&r, &p), Some(config().0.as_slice()));
        r[510] = 0xff;
        assert!(reply(&r, &p).is_some());
        r[407] ^= 1;
        assert!(reply(&r, &p).is_none());
    }
    #[test]
    fn wrong_command_length_header_checksum_and_echo_never_become_readings() {
        let p = request(5).unwrap();
        let mut valid = p;
        valid[8..14].copy_from_slice(&MODEL);
        assert_eq!(reply(&valid, &p), Some(MODEL.as_slice()));
        for index in [0, 1, 2, 3, 4, 5, 6, 7] {
            let mut r = valid;
            r[index] ^= 1;
            assert!(reply(&r, &p).is_none());
        }
        assert!(reply(&valid[..519], &p).is_none());
        assert!(reply(&valid[1..], &p).is_none());
        assert!(reply(&[0; LENGTH], &p).is_none());
        for command in [5, 7, 9, 0x44] {
            let p = request(command).unwrap();
            let mut echo = p;
            echo[519] = 0xff;
            assert!(reply(&p, &p).is_none());
            assert!(reply(&echo, &p).is_none());
        }
    }
    #[test]
    fn actual_tables_preserve_axes_duplicates_and_order_without_defaults() {
        let c = config();
        let d = c.dpi();
        assert_eq!(
            d.stages.iter().map(|s| s.value).collect::<Vec<_>>(),
            [100, 800, 800, 1600, 12000, 3200]
        );
        assert_eq!(d.active_stage, Some(1));
        assert_eq!(d.current, Some(800));
        assert_eq!(d.current_y, Some(1600));
        assert_eq!(d.stages[2].index, 2);
        assert_eq!(d.stage_y[&1], 1600);
        assert_eq!(c.rate().current, Some(1000));
        assert!(d.stage_writable);
        assert!(!d.writable && !d.stage_editable);
        assert_eq!(d.profile, None);
        assert_eq!(d.configuration_token, Some(c.token()));
        let mut raw = c.0;
        raw[399] ^= 1;
        assert_ne!(Config::decode(&raw).unwrap().token(), c.token());
    }
    #[test]
    fn invalid_sensor_marker_stage_fields_rate_and_out_of_range_axes_disable_config() {
        let c = config();
        for (offset, value) in [
            (0, 0),
            (1, 0x19),
            (2, 0),
            (2, 0x15),
            (3, 0x06),
            (3, 0x10),
            (3, 0x17),
            (3, 0x76),
            (7, 120),
            (8, 1),
            (27, 120),
            (28, 0xff),
        ] {
            let mut raw = c.0;
            raw[offset] = value;
            assert!(Config::decode(&raw).is_none(), "field {offset}");
        }
        assert!(Config::decode(&c.0[..399]).is_none());
        assert!(c
            .switch(DpiStage {
                index: 6,
                value: 800
            })
            .is_none());
        assert!(c
            .switch(DpiStage {
                index: 1,
                value: 1600
            })
            .is_none());
    }
    #[test]
    fn identity_online_battery_and_config_form_one_verified_snapshot() {
        let mut s = session();
        let r = s.snapshot().unwrap();
        assert_eq!(r.target.name, "AULA SC380 PRO");
        assert!(r.target.verified_identity && r.connected);
        assert_eq!(r.battery.as_ref().unwrap().percent, Some(63));
        assert_eq!(r.battery.unwrap().charging, None);
        assert!(r.battery_at.is_some());
        assert!(!r.battery_unavailable);
        assert_eq!(r.dpi.current, Some(800));
        assert!(s.channel.writes().is_empty());
        assert_eq!(
            s.channel.sent.iter().map(|p| p[2]).collect::<Vec<_>>(),
            [5, 7, 9, 0x44, 0x44, 5, 7]
        );
        let other = Session::new(Fake::new(), &interface(0x0138, "two"));
        assert_ne!(s.target().key, other.target().key);
        assert_ne!(candidate(&s.interface).key, s.target().key);
    }
    #[test]
    fn identity_mismatch_unknown_link_and_replacement_do_not_publish_battery() {
        let mut s = session();
        s.channel.id[5] ^= 1;
        assert!(s.snapshot().is_err());
        assert_eq!(s.channel.sent.len(), 1);
        for online in [0, 2, 255] {
            let mut s = session();
            s.channel.online = online;
            assert!(s.snapshot().is_err());
            assert!(!s.channel.sent.iter().any(|p| p[2] == 9));
        }
        let mut s = session();
        s.channel.replace_at_battery = true;
        assert!(s.snapshot().is_err());
    }
    #[test]
    fn battery_and_settings_failures_remain_independent_and_never_fabricate_charging() {
        let mut s = session();
        s.channel.bad_config = true;
        let r = s.snapshot().unwrap();
        assert_eq!(r.battery.unwrap().percent, Some(63));
        assert_eq!(r.dpi, Dpi::default());
        let mut s = session();
        s.channel.skip_battery = true;
        let r = s.snapshot().unwrap();
        assert!(r.battery.is_none() && r.battery_at.is_none());
        assert!(!r.battery_unavailable);
        assert!(r.dpi.stage_writable);
        for value in [0, 101, 255] {
            let mut s = session();
            s.channel.battery = value;
            let r = s.snapshot().unwrap();
            assert!(r.battery.is_none() && r.battery_at.is_none());
            assert_eq!(r.dpi.current, Some(800));
        }
        for value in [1, 100] {
            assert_eq!(battery(value).unwrap().charging, None);
        }
    }
    #[test]
    fn failed_or_inconsistent_reads_do_not_enable_writes_and_recover_later() {
        let mut s = session();
        s.channel.echo = true;
        assert!(s.snapshot().is_err());
        s.channel.echo = false;
        s.channel.change_on_read = Some(2);
        let r = s.snapshot().unwrap();
        assert!(!r.dpi.stage_writable);
        assert_eq!(r.battery.unwrap().percent, Some(63));
        s.channel.change_on_read = None;
        let r = s.snapshot().unwrap();
        assert!(r.dpi.stage_writable);
        assert!(r.battery_at.is_some());
        s.deadline = Instant::now() - Duration::from_millis(1);
        let before = s.channel.sent.len();
        assert!(s.snapshot().is_err());
        assert_eq!(before, s.channel.sent.len());
    }
    #[test]
    fn switch_changes_only_active_nibble_and_preserves_all_other_settings() {
        let mut s = session();
        let before = s.channel.raw.clone();
        assert!(switch(&mut s).is_ok());
        let writes = s.channel.writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0][11], 0x56);
        for n in 0..400 {
            if n != 3 {
                assert_eq!(s.channel.raw.0[n], before.0[n], "field {n}");
            }
        }
        assert_eq!(s.channel.raw.dpi().current, Some(12000));
        assert_eq!(s.channel.raw.rate(), before.rate());
        let counts = s.channel.sent.iter().filter(|p| p[2] == 0x44).count();
        assert_eq!(counts, 6);
    }
    #[test]
    fn stale_ui_or_configuration_identity_change_prevents_all_writes() {
        let mut s = session();
        let mut target = s.target();
        target.verified_identity = false;
        assert!(s
            .switch(&target, config().dpi().stages[4], config().token())
            .is_err());
        assert!(s.channel.sent.is_empty());
        let mut s = session();
        let mut target = s.target();
        target.key.push('x');
        assert!(s
            .switch(&target, config().dpi().stages[4], config().token())
            .is_err());
        assert!(s.channel.sent.is_empty());
        let mut s = session();
        s.channel.raw.0[399] ^= 1;
        assert!(switch(&mut s).is_err());
        assert!(s.channel.writes().is_empty());
        for at in [2, 3, 4] {
            let mut s = session();
            s.channel.change_on_read = Some(at);
            assert!(switch(&mut s).is_err());
            assert!(s.channel.writes().is_empty());
        }
        let mut s = session();
        s.channel.id[5] ^= 1;
        assert!(switch(&mut s).is_err());
        assert!(s.channel.writes().is_empty());
    }
    #[test]
    fn unchanged_stage_never_writes_and_lost_send_completion_is_not_retried() {
        let mut s = session();
        assert!(s
            .switch(&s.target(), config().dpi().stages[1], config().token())
            .is_ok());
        assert!(s.channel.writes().is_empty());
        let mut s = session();
        s.channel.lost_completion = true;
        assert!(switch(&mut s).is_ok());
        assert_eq!(s.channel.writes().len(), 1);
    }
    #[test]
    fn full_readback_rejects_nonapplied_write_neighbor_changes_replacement_or_sleep() {
        for failure in 0..4 {
            let mut s = session();
            match failure {
                0 => s.channel.apply = false,
                1 => s.channel.corrupt_neighbor = true,
                2 => s.channel.replace_after_write = true,
                _ => s.channel.sleep_after_write = true,
            }
            assert!(switch(&mut s).is_err());
            assert_eq!(s.channel.writes().len(), 1);
        }
    }
}
