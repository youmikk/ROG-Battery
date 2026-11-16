//! Report 8 / 16-byte CompX configuration channel, independent of the keyboard protocol.
//! Wire format and sensor layouts are documented in PROTOCOLS.md.
use super::*;
use crate::{device::{DpiStage, PollingRate}, hid::HidDevice};
use std::{sync::{Arc, Mutex}, time::{Duration, Instant}};

pub struct CompX;
const REPORT: u8 = 8;
const TIMEOUT: u32 = 700;
const RATES: [(u8, u16); 7] = [(8, 125), (4, 250), (2, 500), (1, 1000),
    (16, 2000), (32, 4000), (64, 8000)];

fn control(i: &HidInterface) -> bool {
    matches!(i.vendor_id, 0x3554 | 0x373b) && i.usage_page == 0xff02 && i.usage == 2
        && i.input_len == 17 && i.output_len == 17
}

fn candidate(i: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::CompX,i,0);
    // Shared receiver serial strings are not evidence that two interfaces are one mouse.
    target.family = DeviceFamily::Mouse;
    if target.name.is_empty() { target.name = "CompX Mouse".into(); }
    target
}

// The guard spans the complete read/modify/verify operation, including diagnostic reads.
fn channel_lock(path: &str) -> Arc<Mutex<()>> {
    super::channel::lock(path)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Codec { Legacy, Step50, Ultra, Master, Se }

#[derive(Clone, Copy, Debug)]
struct Layout { codec: Codec, min: u16, max: u16, stages: u8, r1: bool }

impl Layout {
    fn for_identity(i: &HidInterface, id: Option<[u8; 2]>) -> Option<Self> {
        if i.vendor_id == 0x3554 && matches!(i.product_id, 0xfb56 | 0xfb57) {
            return Some(Self { codec: Codec::Legacy, min: 50, max: 26000, stages: 8, r1: false });
        }
        // Runtime CID/MID selects a sensor layout, rather than assuming one from a brand.
        let (codec, min, max, r1) = match id {
            Some([1, 8]) => (Codec::Ultra, 10, 42000, false),
            Some([1, 31 | 52]) => (Codec::Master, 50, 40000, false),
            Some([2, 11 | 12 | 27]) => (Codec::Step50, 100, 30000, true),
            Some([2, 32]) => (Codec::Se, 200, 18000, true),
            Some([2, 39]) => (Codec::Step50, 50, 36000, false),
            Some([2, 83]) => (Codec::Step50, 100, 30000, false),
            _ => return None,
        };
        Some(Self { codec, min, max, stages: if r1 || codec == Codec::Legacy { 8 } else { 6 }, r1 })
    }

    fn record_len(self) -> usize { if self.codec == Codec::Master { 6 } else { 4 } }
    fn address(self, stage: u8) -> u16 {
        (if self.codec == Codec::Master { 0x1b00 } else { 0x0c }) + u16::from(stage) * self.record_len() as u16
    }
    fn valid(self, dpi: u16) -> bool {
        if !(self.min..=self.max).contains(&dpi) { return false; }
        match self.codec {
            Codec::Ultra => dpi % (if dpi <= 10000 { 10 } else if dpi <= 30000 { 50 } else { 100 }) == 0,
            Codec::Legacy => dpi % 50 == 0,
            Codec::Step50 => dpi % (if dpi <= 30000 { 50 } else { 100 }) == 0,
            Codec::Master => dpi % 10 == 0,
            Codec::Se => dpi % (if dpi <= 10000 { 50 } else { 100 }) == 0,
        }
    }
    fn options(self) -> Vec<u16> { (self.min..=self.max).filter(|dpi| self.valid(*dpi)).collect() }

    fn axis(self, low: u8, flags: u8) -> Option<u16> {
        let code = u32::from(low) | (u32::from(flags >> 2) << 8);
        let value = match self.codec {
            Codec::Legacy if flags & 3 == 0 => (code + 1) * 50,
            Codec::Step50 if flags & 2 == 0 => (code + 1) * if flags & 1 == 0 { 50 } else { 100 },
            Codec::Ultra => {
                let base = if flags & 2 == 0 { (code + 1) * 10 } else { 10050 + code * 50 };
                base * if flags & 1 == 0 { 1 } else { 2 }
            }
            Codec::Se if flags & !2 == 0 => {
                let position = se_codes().position(|v| v == low)? as u32;
                let base = (position + 1) * 50;
                if flags == 2 && base <= 5000 { return None; }
                base * if flags == 2 { 2 } else { 1 }
            }
            _ => return None,
        };
        u16::try_from(value).ok().filter(|v| self.valid(*v))
    }

    fn decode(self, bytes: &[u8]) -> Option<(u16, u16)> {
        if bytes.len() != self.record_len() || sum(bytes) != 0x55 { return None; }
        if self.codec == Codec::Master {
            let flags = bytes[4];
            if flags & !0x11 != 0 { return None; }
            let x = (u32::from(u16::from_le_bytes([bytes[0], bytes[1]])) + 1)
                * if flags & 1 != 0 { 2 } else { 1 };
            let y = (u32::from(u16::from_le_bytes([bytes[2], bytes[3]])) + 1)
                * if flags & 0x10 != 0 { 2 } else { 1 };
            let (x, y) = (u16::try_from(x).ok()?, u16::try_from(y).ok()?);
            return (self.valid(x) && self.valid(y)).then_some((x, y));
        }
        Some((self.axis(bytes[0], bytes[2] & 15)?, self.axis(bytes[1], bytes[2] >> 4)?))
    }

    fn encode(self, x: u16, y: u16) -> Option<Vec<u8>> {
        if !self.valid(x) || !self.valid(y) { return None; }
        let mut result = if self.codec == Codec::Master {
            let mut row = Vec::from((x - 1).to_le_bytes());
            row.extend((y - 1).to_le_bytes()); row.push(0); row
        } else {
            let axis = |dpi: u16| -> Option<(u8, u8)> {
                if self.codec == Codec::Se {
                    let doubled = dpi > 10000;
                    let index = usize::from(dpi / if doubled { 100 } else { 50 } - 1);
                    return Some((se_codes().nth(index)?, if doubled { 2 } else { 0 }));
                }
                let doubled = dpi > 30000;
                let base = u32::from(dpi) / if doubled { 2 } else { 1 };
                let (code, mode) = if self.codec == Codec::Ultra {
                    if base <= 10000 { (base / 10 - 1, 0) } else { ((base - 10050) / 50, 2) }
                } else { (base / 50 - 1, 0) };
                Some((code as u8, ((code >> 8) as u8) << 2 | mode | u8::from(doubled)))
            };
            let (lx, fx) = axis(x)?; let (ly, fy) = axis(y)?;
            vec![lx, ly, fx | (fy << 4)]
        };
        result.push(0x55u8.wrapping_sub(sum(&result)));
        (self.decode(&result) == Some((x, y))).then_some(result)
    }
}

// PAW3395SE's non-linear register values; these are protocol data, not a model catalog.
fn se_codes() -> impl Iterator<Item = u8> {
    const UNUSED: [u8; 35] = [7,13,20,26,33,40,46,53,60,66,73,80,86,93,100,106,113,
        120,126,133,140,146,153,160,166,173,180,186,193,200,206,213,220,226,233];
    (1..=235).filter(|v| !UNUSED.contains(v))
}
fn sum(bytes: &[u8]) -> u8 { bytes.iter().fold(0u8, |s, b| s.wrapping_add(*b)) }
fn packet(command: u8, address: u16, data: &[u8], length: usize) -> [u8; 17] {
    let mut p = [0; 17];
    p[0] = REPORT; p[1] = command;
    p[3..5].copy_from_slice(&address.to_be_bytes()); p[5] = length as u8;
    p[6..6 + data.len()].copy_from_slice(data);
    p[16] = 0x55u8.wrapping_sub(sum(&p[..16])); p
}
fn valid_frame(r: &[u8], command: u8) -> bool {
    r.len() == 17 && r[0] == REPORT && r[1] == command && r[2] == 0 && sum(r) == 0x55
}
fn pair(v: u8) -> [u8; 2] { [v, 0x55u8.wrapping_sub(v)] }
fn scalar(bytes: &[u8]) -> Option<u8> {
    (bytes.len() == 2 && sum(bytes) == 0x55).then(|| bytes[0])
}

fn control_output(i: &HidInterface) -> bool {
    // Confirmed Windows SET_REPORT transport; other CompX generations keep their transport.
    i.vendor_id == 0x373b && matches!(i.product_id, 0x10c9 | 0x1278)
}

trait Channel {
    fn exchange(&mut self, p: &[u8], matcher: &mut dyn FnMut(&[u8]) -> bool) -> Option<Vec<u8>>;
    fn settle(&self) {}
}
struct DeviceChannel { device: HidDevice, control: bool }
impl DeviceChannel {
    fn open(i: &HidInterface) -> Result<Self, String> {
        Ok(Self { device: HidDevice::open(i)?, control: control_output(i) })
    }
}
impl Channel for DeviceChannel {
    fn exchange(&mut self, p: &[u8], matcher: &mut dyn FnMut(&[u8]) -> bool) -> Option<Vec<u8>> {
        if self.control { self.device.transact_control_output(p, TIMEOUT, matcher) }
        else { self.device.transact(p, TIMEOUT, matcher) }
    }
    fn settle(&self) { std::thread::sleep(Duration::from_millis(10)); }
}

struct Session<C> { channel: C, interface: HidInterface, id: Option<[u8; 2]>, layout: Option<Layout> }
struct Config { dpi: Dpi, rate: PollingRate, profile: Option<u16> }

impl<C: Channel> Session<C> {
    fn open(channel: C, i: &HidInterface) -> Self {
        Self { channel, interface: i.clone(), id: None, layout: None }
    }
    fn query(&mut self, command: u8, address: u16, data: &[u8], len: usize) -> Result<Vec<u8>, String> {
        if len > 10 || data.len() > 10 { return Err("CompX 请求长度无效".into()); }
        let started = Instant::now();
        let p = packet(command, address, data, len);
        let r = self.channel.exchange(&p, &mut |r| {
            if !valid_frame(r, command) || r[5] > 10 { return false; }
            match command {
                // A checksum-valid zero-body query echo is not a battery measurement.
                4 => r[5] >= 1 && r[6] <= 100,
                0x10 => r[5] >= 2 && r[6..8] != [0, 0] && r[6..8] != [255, 255],
                0x0e => r[5] >= 1 && r[6] < 4,
                8 => r != p && r[3..5] == address.to_be_bytes() && usize::from(r[5]) == len,
                // Write echoes cannot prove success; write_verified also reads back the data.
                7 => r[3..5] == address.to_be_bytes() && usize::from(r[5]) == len,
                _ => false,
            }
        });
        trace::record(&self.interface, BackendId::CompX, "query", address, command,
            17, r.as_ref().map_or(0, Vec::len), started,
            if r.is_some() { trace::Outcome::Ok } else { trace::Outcome::NoReply });
        self.channel.settle();
        r.ok_or_else(|| "CompX 设备无有效应答".into())
    }
    fn read(&mut self, address: u16, len: usize) -> Result<Vec<u8>, String> {
        if len == 0 { return Err("CompX 请求长度无效".into()); }
        self.query(8, address, &[], len).map(|r| r[6..6 + len].to_vec())
    }
    fn identify(&mut self) {
        self.id = self.query(0x10, 0, &[], 0).ok().filter(|r| (2..=10).contains(&r[5]))
            .map(|r| [r[6], r[7]]).filter(|id| *id != [0, 0] && *id != [255, 255]);
        self.layout = Layout::for_identity(&self.interface, self.id);
    }
    fn target(&self, original: &Target) -> Target {
        let mut target = original.clone();
        if let Some(id) = self.id {
            target.key = crate::device::stable_key(&format!("{}:cidmid:{}:{}",
                candidate(&self.interface).key, id[0], id[1]));
            target.verified_identity = true;
            let name = match id {
                [1, 8] => Some("ATK F1 Ultimate 2.0"),
                [1, 31 | 52] => Some("ATK A9 Mini +"),
                [2, 11 | 12] => Some("VXE R1"),
                [2, 27] => Some("VXE R1 Pro Max"),
                [2, 32] => Some("VXE R1 SE+"),
                [2, 39] => Some("ATK X1 Pro Max"),
                [2, 83] => Some("ATK A9 Plus Nearlink"),
                _ => None,
            };
            if let Some(name) = name { target.name = name.into(); }
        }
        if self.interface.vendor_id == 0x3554 && matches!(self.interface.product_id, 0xfb56 | 0xfb57) {
            target.name = "VGN Dragonfly F2 Master+".into();
        }
        target
    }
    fn profile(&mut self) -> Option<u16> {
        self.query(0x0e, 0, &[], 0).ok().filter(|r| (1..=10).contains(&r[5]) && r[6] < 4)
            .map(|r| u16::from(r[6]))
    }
    fn live_r1(&self) -> bool {
        self.layout.is_some_and(|l| l.r1) && (self.interface.product_id == 0x1085 ||
            self.interface.vendor_id == 0x3554 && self.interface.product_id == 0xf58e)
    }
    fn group_write(&self) -> bool {
        self.interface.vendor_id == 0x3554 && self.interface.product_id == 0xf58a && self.id == Some([2,27])
    }
    fn write_supported(&self) -> bool {
        !control_output(&self.interface) && self.id.is_some() && self.layout.is_some_and(|l| !l.r1 || self.group_write() ||
            self.interface.vendor_id == 0x3554 && matches!((self.interface.product_id,self.id),
                (0xf58c,Some([2,27])) | (0xf58f,Some([2,32]))))
    }
    fn config(&mut self) -> Result<Config, String> {
        let layout = self.layout.ok_or("CompX 传感器布局尚未确认")?;
        let profile = self.profile();
        let system = self.read(0, 6)?;
        let mut rate = self.read_rate(&system,profile);
        let mut dpi = self.read_stages(layout,profile,&system).unwrap_or_default();
        if self.profile() != profile { rate.writable = false; dpi.disable_writes(); }
        Ok(Config { dpi, rate, profile })
    }
    fn read_stages(&mut self, layout: Layout, profile: Option<u16>, system: &[u8]) -> Result<Dpi,String> {
        let count = scalar(&system[2..4]).filter(|n| (1..=layout.stages).contains(n))
            .ok_or("CompX DPI 档位数无效")?;
        let active = scalar(&system[4..6]).filter(|n| *n < count).ok_or("CompX 当前档位无效")?;
        let mut dpi = Dpi { values: layout.options(), active_stage: Some(active), profile,
            ..Default::default() };
        for index in 0..count {
            let bytes = self.read(layout.address(index), layout.record_len())?;
            let (x,y) = layout.decode(&bytes).ok_or("CompX DPI 档位响应无效")?;
            dpi.stages.push(DpiStage { index, value: x }); dpi.stage_y.insert(index, y);
            if index == active { dpi.current = Some(x); dpi.current_y = Some(y); }
        }
        // A bank change during the scan invalidates the list rather than mixing two profiles.
        if self.read(0,6)?[2..6] != system[2..6] || self.profile() != profile {
            return Err("设备档位或配置已变化，请刷新后重试".into());
        }
        let writable = self.write_supported() && profile.is_some();
        dpi.writable = writable && dpi.current == dpi.current_y;
        dpi.write_effect = writable.then_some(crate::device::DpiWriteEffect::Stored);
        dpi.stage_writable = writable; dpi.stage_editable = writable;
        Ok(dpi)
    }
    fn read_rate(&mut self, system: &[u8], profile: Option<u16>) -> PollingRate {
        let current = if self.live_r1() {
            self.read(0x70, 4).ok().filter(|r| r[0] == 0x0b && r[2] == 0 &&
                r[1].wrapping_add(r[3]) == 0x55).and_then(|r| match r[1] {
                    3 => Some(250), 2 => Some(500), 1 => Some(1000), _ => None })
        } else { scalar(&system[..2]).and_then(|raw| RATES.iter().find(|(v,_)| *v == raw).map(|(_,hz)| *hz)) };
        // Higher rates require evidence about the actual transport, not just the mouse sensor.
        let ceiling = if self.interface.vendor_id == 0x3554 && matches!(self.interface.product_id, 0xfb56 | 0xfb57) {
            8000
        } else { 1000 };
        let profile_stable = profile.is_some() && self.profile() == profile;
        PollingRate { current, values: if current.is_some() && !self.live_r1() {
            RATES.iter().filter_map(|(_,hz)| (*hz <= ceiling).then_some(*hz)).collect()
        } else { Vec::new() }, writable: self.write_supported() && profile_stable && current.is_some() && !self.live_r1() }
    }
    fn battery(&mut self) -> Option<Battery> {
        let r = self.query(4,0,&[],0).ok()?;
        if !(1..=10).contains(&r[5]) || r[6] > 100 { return None; }
        let mv = (r[5] >= 4).then(|| u16::from_be_bytes([r[8],r[9]])).filter(|v| *v > 0);
        Some(Battery { percent: Some(r[6]), charging: (r[5] >= 2).then(|| match r[7] {
            0 => Some(false), 1 => Some(true), _ => None }).flatten(), voltage_mv: mv,
            approximate: false, level: None })
    }
    fn check(&mut self, target: &Target) -> Result<(), String> {
        self.identify();
        if !target.verified_identity || self.target(&candidate(&self.interface)).key != target.key {
            return Err("设备身份已变化，请刷新后重试".into());
        }
        if !self.write_supported() { return Err("此连接尚未确认设置写入协议".into()); }
        Ok(())
    }
    fn write_verified(&mut self, address: u16, data: &[u8], original: &[u8]) -> Result<(), String> {
        if self.read(address,original.len())?.as_slice() != original {
            return Err("设备配置已变化，请刷新后重试".into());
        }
        if data == original { return Ok(()); }
        // Some firmware does not acknowledge writes; only a subsequent read can confirm success.
        let _ = self.query(7,address,data,data.len());
        if self.read(address,data.len()).as_deref() == Ok(data) { return Ok(()); }
        // No blind rollback: another application may have changed this row in the meantime.
        Err("无法确认设备已保存设置，请刷新后检查".into())
    }
    fn checked_config(&mut self, target: &Target, profile: u16) -> Result<Config, String> {
        self.check(target)?;
        let config = self.config()?;
        if config.dpi.profile != Some(profile) { return Err("设备配置已变化，请刷新后重试".into()); }
        Ok(config)
    }
    fn switch(&mut self, target: &Target, stage: DpiStage, profile: u16) -> Result<(), String> {
        let config = self.checked_config(target, profile)?;
        if !config.dpi.stage_writable || !config.dpi.stages.contains(&stage) {
            return Err("设备档位已变化，请刷新后重试".into());
        }
        let (address,len,offset) = if self.group_write() { (0,10,4) } else { (4,2,0) };
        let old = self.read(address,len)?; let mut next = old.clone();
        if scalar(&old[offset..offset+2]) != config.dpi.active_stage || self.profile() != Some(profile) {
            return Err("设备档位或配置已变化，请刷新后重试".into());
        }
        next[offset..offset+2].copy_from_slice(&pair(stage.index));
        self.check(target)?;
        if self.profile() != Some(profile) { return Err("设备配置已变化，请刷新后重试".into()); }
        self.write_verified(address,&next,&old)?;
        self.check(target)?;
        if self.profile() != Some(profile) || self.read(4,2)?.as_slice() != pair(stage.index) {
            return Err("无法确认 DPI 档位切换".into());
        }
        Ok(())
    }
    fn edit(&mut self, target: &Target, stage: DpiStage, profile: u16, expected_y: u16, x: u16, y: u16) -> Result<(), String> {
        self.edit_checked(target,stage,profile,expected_y,x,y,false)
    }
    fn edit_checked(&mut self, target: &Target, stage: DpiStage, profile: u16, expected_y: u16, x: u16, y: u16, active_only: bool) -> Result<(), String> {
        let config = self.checked_config(target,profile)?;
        if !config.dpi.stage_editable || !config.dpi.stages.contains(&stage)
            || (active_only && (!config.dpi.writable || config.dpi.active_stage != Some(stage.index)))
            || config.dpi.stage_y.get(&stage.index) != Some(&expected_y) {
            return Err("设备档位已变化，请刷新后重试".into());
        }
        let layout = self.layout.ok_or("CompX 传感器布局尚未确认")?;
        let encoded = layout.encode(x,y).ok_or("DPI 不在设备报告的可用范围内")?;
        let (address,len,offset) = if self.group_write() {
            (layout.address(stage.index / 2 * 2),8,usize::from(stage.index % 2)*4)
        } else { (layout.address(stage.index), layout.record_len(),0) };
        let old = self.read(address,len)?; let mut next = old.clone();
        if layout.decode(&old[offset..offset+encoded.len()]) != Some((stage.value,config.dpi.stage_y[&stage.index]))
            || self.profile() != Some(profile) { return Err("设备档位或配置已变化，请刷新后重试".into()); }
        next[offset..offset+encoded.len()].copy_from_slice(&encoded);
        self.check(target)?;
        if self.profile() != Some(profile) { return Err("设备配置已变化，请刷新后重试".into()); }
        if active_only && scalar(&self.read(4,2)?) != Some(stage.index) {
            return Err("设备档位已变化，请刷新后重试".into());
        }
        self.write_verified(address,&next,&old)?;
        self.check(target)?;
        if self.profile() != Some(profile) { return Err("设备配置已变化，请刷新后检查".into()); }
        if active_only {
            let after = self.config()?;
            if after.dpi.active_stage != Some(stage.index) || after.dpi.current != Some(x)
                || after.dpi.current_y != Some(y) {
                return Err("无法确认 DPI 档位切换".into());
            }
        }
        Ok(())
    }
    fn rate(&mut self, target: &Target, hz: u16) -> Result<(), String> {
        self.check(target)?;
        let config = self.config()?;
        if !config.rate.writable || !config.rate.values.contains(&hz) {
            return Err("回报率不在此连接的可用范围内".into());
        }
        let raw = RATES.iter().find(|(_,v)| *v == hz).ok_or("回报率无效")?.0;
        let len = if self.group_write() {10} else {2};
        let old = self.read(0,len)?; let mut next = old.clone();
        if scalar(&old[..2]).and_then(|v| RATES.iter().find(|(raw,_)| *raw == v).map(|(_,v)| *v)) != config.rate.current
            || self.profile() != config.profile { return Err("设备配置已变化，请刷新后重试".into()); }
        next[..2].copy_from_slice(&pair(raw));
        self.check(target)?;
        if self.profile() != config.profile { return Err("设备配置已变化，请刷新后重试".into()); }
        self.write_verified(0,&next,&old)?;
        self.check(target)?;
        if self.profile() != config.profile { return Err("设备配置已变化，请刷新后检查".into()); }
        Ok(())
    }
}

impl DeviceBackend for CompX {
    fn discover(&self, _: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>,String> {
        Ok(interfaces.iter().filter(|i| control(i)).map(candidate).collect())
    }
    fn read(&self, _: &HidApi, target: &Target, _: bool) -> Result<Snapshot,String> {
        if !control(&target.interface) { return Err("CompX 控制接口无效".into()); }
        let lock = channel_lock(&target.interface.path);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut s = Session::open(DeviceChannel::open(&target.interface)?, &target.interface);
        s.identify();
        let battery = s.battery(); let battery_at = battery.as_ref().map(|_| Instant::now());
        let config = s.config().ok();
        if battery.is_none() && config.is_none() { return Err("CompX 设备无有效应答".into()); }
        let target = s.target(target);
        let (dpi,polling_rate) = config.map(|c| (c.dpi,c.rate)).unwrap_or_default();
        if battery.is_none() && dpi.current.is_none() && polling_rate.current.is_none() {
            return Err("CompX 设备无有效应答".into());
        }
        Ok(Snapshot { target, battery, battery_at, dpi, polling_rate, connected: true })
    }
    fn set_dpi(&self, api: &HidApi, target: &Target, dpi: u16) -> Result<(),String> {
        with_session(target, |s| {
            s.check(target)?; let c = s.config()?;
            if !c.dpi.writable { return Err("请分别设置 X/Y DPI".into()); }
            let active = c.dpi.active_stage.ok_or("CompX 当前档位无效")?;
            let stage = *c.dpi.stages.iter().find(|v| v.index == active).ok_or("CompX 当前档位无效")?;
            let _ = api;
            let expected_y = *c.dpi.stage_y.get(&stage.index).ok_or("CompX DPI 档位响应无效")?;
            s.edit(target,stage,c.dpi.profile.ok_or("设备配置未知")?,expected_y,dpi,dpi)
        })
    }
    fn set_dpi_stage(&self, _: &HidApi, target: &Target, stage: DpiStage, context: crate::device::StageContext) -> Result<(),String> {
        with_session(target, |s| s.switch(target,stage,context.profile()?))
    }
    fn edit_dpi_stage(&self, _: &HidApi, target: &Target, stage: DpiStage, context: crate::device::StageContext, expected_y:u16, x:u16, y:u16) -> Result<(),String> {
        with_session(target, |s| s.edit(target,stage,context.profile()?,expected_y,x,y))
    }
    fn set_polling_rate(&self, _: &HidApi, target: &Target, hz:u16) -> Result<(),String> {
        with_session(target, |s| s.rate(target,hz))
    }
}
pub fn set_preset(target: &Target, stage: DpiStage, context: crate::device::StageContext, expected_y: u16, value: u16) -> Result<(),String> {
    if target.backend != BackendId::CompX { return Err("此协议不提供通用 DPI 写入".into()); }
    with_session(target, |s| s.edit_checked(target,stage,context.profile()?,expected_y,value,value,true))
}

fn with_session(target: &Target, f: impl FnOnce(&mut Session<DeviceChannel>) -> Result<(),String>) -> Result<(),String> {
    if !control(&target.interface) { return Err("CompX 控制接口无效".into()); }
    let lock = channel_lock(&target.interface.path);
    let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
    f(&mut Session::open(DeviceChannel::open(&target.interface)?, &target.interface))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    struct Reports {
        id: [u8;2], profile: u8, memory: BTreeMap<u16,u8>,
        writes: Vec<(u16,Vec<u8>)>, ack: bool, accept: bool,
        change_profile: bool, change_identity: bool, wrong_address: bool, change_stage: bool,
    }
    impl Reports {
        fn new(id: [u8;2]) -> Self {
            let mut memory = BTreeMap::new();
            // Protocol fixtures with deliberately non-zero unrelated system bytes.
            for (address,bytes) in [(0,vec![1,84,2,83,0,85,0xaa,0xbb,0xcc,0xdd]),
                (12,vec![15,31,0,39,31,31,0,23])] {
                for (offset,byte) in bytes.into_iter().enumerate() { memory.insert(address+offset as u16,byte); }
            }
            Self { id,profile:0,memory,writes:Vec::new(),ack:false,accept:true,
                change_profile:false,change_identity:false,wrong_address:false,change_stage:false }
        }
        fn bytes(&self, address:u16, len:usize) -> Vec<u8> {
            (0..len).map(|offset| *self.memory.get(&(address + offset as u16)).unwrap_or(&0)).collect()
        }
        fn response(command:u8,address:u16,data:&[u8]) -> Vec<u8> {
            let mut r=vec![0;17];r[0]=8;r[1]=command;
            r[3..5].copy_from_slice(&address.to_be_bytes());r[5]=data.len() as u8;
            r[6..6+data.len()].copy_from_slice(data);
            r[16]=85u8.wrapping_sub(sum(&r[..16]));r
        }
    }
    impl Channel for Reports {
        fn exchange(&mut self,p:&[u8],matcher:&mut dyn FnMut(&[u8])->bool) -> Option<Vec<u8>> {
            assert_eq!(p.len(),17); assert_eq!(sum(p),85);
            let address=u16::from_be_bytes([p[3],p[4]]);let len=usize::from(p[5]);
            let response=match p[1] {
                0x10 => Self::response(0x10,0,&self.id),
                0x0e => Self::response(0x0e,0,&[self.profile]),
                4 => Self::response(4,0,&[0,0,0x0e,0x10]),
                8 => Self::response(8,if self.wrong_address {address.wrapping_add(2)} else {address},&self.bytes(address,len)),
                7 => {
                    let data=&p[6..6+len]; self.writes.push((address,data.to_vec()));
                    if self.accept { for (offset,byte) in data.iter().enumerate() {self.memory.insert(address+offset as u16,*byte);} }
                    if self.change_profile {self.profile=1;}
                    if self.change_identity {self.id=[2,39];}
                    if self.change_stage {self.memory.insert(4,1);self.memory.insert(5,84);}
                    if !self.ack {return None;}
                    Self::response(7,address,data)
                }
                _ => panic!("Unexpected command: {}",p[1]),
            };
            // Simulate unrelated reports arriving ahead of the matching reply.
            assert!(!matcher(&Self::response(0x12,0,&[1,23])));
            if p[1] != 7 { assert!(!matcher(p), "query echo must not complete a read"); }
            matcher(&response).then_some(response)
        }
    }
    fn receiver() -> HidInterface {
        let mut i=interface();i.vendor_id=0x3554;i.product_id=0xf58a;i
    }
    fn session() -> (Session<Reports>,Target) {
        let mut s=Session::open(Reports::new([2,27]),&receiver());s.identify();
        let target=s.target(&candidate(&s.interface));(s,target)
    }
    fn interface() -> HidInterface { HidInterface { path:"transcript".into(),serial:String::new(),
        product:String::new(),vendor_id:0x373b,product_id:0x1000,usage_page:0xff02,usage:2,
        input_len:17,output_len:17,feature_len:0 } }
    #[test]
    fn layouts_roundtrip_every_supported_value_with_independent_axes() {
        for id in [[1,8],[1,52],[2,12],[2,32],[2,39]] {
            let layout = Layout::for_identity(&interface(),Some(id)).unwrap();
            for x in layout.options() {
                let bytes = layout.encode(x,800).unwrap();
                assert_eq!(layout.decode(&bytes),Some((x,800)),"{id:?}: {x}");
                let mut corrupt = bytes.clone(); corrupt[0] ^= 1;
                assert!(layout.decode(&corrupt).is_none());
            }
        }
    }
    #[test]
    fn sensor_specific_vectors_do_not_share_dpi_decoder() {
        let i = interface();
        let ultra = Layout::for_identity(&i,Some([1,8])).unwrap();
        let step = Layout::for_identity(&i,Some([2,39])).unwrap();
        assert_eq!(ultra.decode(&[79,159,0,103]),Some((800,1600)));
        assert_eq!(step.decode(&[15,31,0,39]),Some((800,1600)));
        let master = Layout::for_identity(&i,Some([1,52])).unwrap();
        assert_eq!(master.decode(&[31,3,63,6,0,238]),Some((800,1600)));
        assert_eq!(master.address(2),0x1b0c);
        assert!(Layout::for_identity(&i,Some([99,99])).is_none());
    }
    #[test]
    fn wrong_collections_and_large_reports_are_not_probed() {
        let mut i = interface(); assert!(control(&i));
        i.usage = 1; assert!(!control(&i)); i.usage = 2;
        i.input_len = 65; assert!(!control(&i));
        i.input_len = 17; i.vendor_id = 0x258a; assert!(!control(&i));
    }
    #[test]
    fn control_report_receivers_do_not_inherit_unverified_settings_writes() {
        for pid in [0x10c9, 0x1278] {
            let mut i = interface(); i.product_id = pid;
            assert!(control(&i)); assert!(control_output(&i));
            let mut s = Session::open(Reports::new([1,8]), &i); s.identify();
            assert!(s.layout.is_some()); assert!(!s.write_supported());
        }
        assert!(!control_output(&receiver()));
    }
    #[test]
    fn battery_echo_corruption_and_truncation_do_not_hide_valid_zero_or_live_reply() {
        struct Stream { replies: Vec<Vec<u8>> }
        impl Channel for Stream {
            fn exchange(&mut self, p: &[u8], matcher: &mut dyn FnMut(&[u8])->bool) -> Option<Vec<u8>> {
                assert!(!matcher(p));
                self.replies.iter().find(|r| matcher(r)).cloned()
            }
        }
        let live = vec![8,4,0,0,0,2,40,0,14,80,0,0,0,0,0,0,193];
        let mut corrupt = live.clone(); corrupt[6] = 99;
        let mut s = Session::open(Stream { replies: vec![live[..16].to_vec(),corrupt,live] }, &interface());
        assert_eq!(s.battery().unwrap().percent, Some(40));
        s.channel.replies = vec![Reports::response(4,0,&[0,0])];
        assert_eq!(s.battery().unwrap().percent, Some(0));
        s.channel.replies = vec![Reports::response(4,0,&[40,3])];
        let battery = s.battery().unwrap();
        assert_eq!(battery.percent,Some(40)); assert_eq!(battery.charging,None);
        s.channel.replies = vec![Reports::response(4,0,&[255,0])];
        assert!(s.battery().is_none());
    }
    #[test]
    fn response_validation_rejects_old_address_wrong_command_and_corruption() {
        let request = packet(8,0x1b00,&[],6);
        assert_eq!(request,[8,8,0,27,0,6,0,0,0,0,0,0,0,0,0,0,36]);
        let mut reply = packet(8,0x1b00,&[31,3,63,6,0,238],6);
        assert!(valid_frame(&reply,8)); reply[0]=9; assert!(!valid_frame(&reply,8));
        assert!(!valid_frame(&request[..16],8));
        assert_eq!(scalar(&[0,85]),Some(0)); assert_eq!(scalar(&[2,2]),None);
    }
    #[test]
    fn receiver_reads_real_xy_stages_and_separates_rate_from_dpi_validity() {
        let (mut s,_)=session();
        let c=s.config().unwrap();
        assert_eq!(c.dpi.current,Some(800));assert_eq!(c.dpi.current_y,Some(1600));
        assert!(!c.dpi.writable);assert!(c.dpi.stage_editable);
        assert_eq!(c.dpi.stage_y.get(&0),Some(&1600));
        assert_eq!(c.rate.current,Some(1000));assert_eq!(c.rate.values,vec![125,250,500,1000]);
        s.channel.memory.insert(13,255);
        let c=s.config().unwrap();assert!(c.dpi.current.is_none());assert!(!c.dpi.stage_editable);
        assert_eq!(c.rate.current,Some(1000));assert!(c.rate.writable);
        s.channel.memory.insert(1,0);
        assert!(s.config().unwrap().rate.current.is_none());
    }
    #[test]
    fn stage_switch_preserves_full_receiver_system_row_and_verifies_readback() {
        let (mut s,target)=session();
        s.switch(&target,DpiStage {index:1,value:1600},0).unwrap();
        assert_eq!(s.channel.writes,vec![(0,vec![1,84,2,83,1,84,0xaa,0xbb,0xcc,0xdd])]);
        assert_eq!(s.channel.bytes(4,2),[1,84]);
    }
    #[test]
    fn stage_edit_preserves_neighbor_xy_record_and_does_not_change_active_stage() {
        let (mut s,target)=session();
        s.edit(&target,DpiStage {index:1,value:1600},0,1600,3200,6400).unwrap();
        assert_eq!(s.channel.writes,vec![(12,vec![15,31,0,39,63,127,0,151])]);
        assert_eq!(s.channel.bytes(4,2),[0,85]);
    }
    #[test]
    fn preset_rejects_a_different_active_stage_before_writing_and_verifies_current_axes() {
        let (mut s,target)=session();
        let stage=DpiStage {index:1,value:1600};
        assert!(s.edit_checked(&target,stage,0,1600,3200,3200,true).is_err());
        assert!(s.channel.writes.is_empty());
        s.channel.memory.insert(4,1);s.channel.memory.insert(5,84);
        s.edit_checked(&target,stage,0,1600,3200,3200,true).unwrap();
        assert_eq!(s.channel.writes.len(),1);
        let after=s.config().unwrap();
        assert_eq!(after.dpi.current,Some(3200));assert_eq!(after.dpi.current_y,Some(3200));
    }
    #[test]
    fn preset_stage_changed_during_write_is_not_reported_as_an_active_switch() {
        let (mut s,target)=session();
        // Equal axes permit scalar presets; changing stage while writing must fail readback.
        s.channel.memory.insert(13,15);s.channel.memory.insert(15,55);
        s.channel.change_stage=true;
        assert!(s.edit_checked(&target,DpiStage {index:0,value:800},0,800,3200,3200,true).is_err());
        assert_eq!(s.channel.writes.len(),1);
    }
    #[test]
    fn rate_write_preserves_stage_fields_and_rejects_unadvertised_transport_rates() {
        let (mut s,target)=session();
        assert!(s.rate(&target,8000).is_err());assert!(s.channel.writes.is_empty());
        s.rate(&target,500).unwrap();
        assert_eq!(s.channel.writes,vec![(0,vec![2,83,2,83,0,85,0xaa,0xbb,0xcc,0xdd])]);
    }
    #[test]
    fn ignored_write_is_an_error_even_without_ack_and_never_triggers_blind_rollback() {
        let (mut s,target)=session();s.channel.accept=false;
        assert!(s.switch(&target,DpiStage {index:1,value:1600},0).unwrap_err().contains("无法确认"));
        assert_eq!(s.channel.writes.len(),1);assert_eq!(s.channel.bytes(4,2),[0,85]);
    }
    #[test]
    fn stale_profile_stage_and_changed_receiver_identity_cannot_authorize_settings() {
        let (mut s,target)=session();
        assert!(s.switch(&target,DpiStage {index:1,value:800},0).is_err());
        assert!(s.edit(&target,DpiStage {index:1,value:1600},1,1600,800,800).is_err());
        assert!(s.edit(&target,DpiStage {index:0,value:800},0,800,800,800).is_err());
        assert!(s.channel.writes.is_empty());
        s.channel.change_identity=true;
        assert!(s.switch(&target,DpiStage {index:1,value:1600},0).is_err());
        assert_eq!(s.channel.writes.len(),1);
        let (mut s,target)=session();s.channel.change_profile=true;
        assert!(s.rate(&target,500).is_err());assert_eq!(s.channel.writes.len(),1);
    }
    #[test]
    fn invalid_scalars_and_wrong_address_do_not_create_default_dpi() {
        let (mut s,_)=session();s.channel.memory.insert(3,0);
        let dpi=s.config().unwrap().dpi;assert_eq!(dpi,Dpi::default());
        s.channel.wrong_address=true;assert!(s.read(0,6).is_err());
        assert!(s.channel.writes.is_empty());
    }
    #[test]
    fn unknown_identity_keeps_valid_zero_battery_without_guessing_dpi_or_charging() {
        let mut s=Session::open(Reports::new([90,90]),&interface());s.identify();
        let battery=s.battery().unwrap();assert_eq!(battery.percent,Some(0));assert_eq!(battery.charging,Some(false));
        assert!(s.config().is_err());assert!(s.channel.writes.is_empty());
    }
    #[test]
    fn live_r1_receiver_does_not_use_link_mode_as_polling_rate_or_enable_eeprom_writes() {
        let mut i=interface();i.product_id=0x1085;
        let mut channel=Reports::new([2,32]);
        // The selector row uses a different rate encoding from the system register.
        for (offset,byte) in [0x0b,2,0,83].into_iter().enumerate() {channel.memory.insert(0x70+offset as u16,byte);}
        // Read this test with the PAW3395 sensor to avoid assuming the SE's register encoding.
        channel.id=[2,12];
        let mut s=Session::open(channel,&i);s.identify();
        let c=s.config().unwrap();assert_eq!(c.rate.current,Some(500));
        assert!(!c.rate.writable);assert!(!c.dpi.stage_writable);
        assert!(s.channel.writes.is_empty());
    }
    #[test]
    fn identical_receiver_serials_do_not_merge_channels() {
        let mut a=interface();a.serial="shared-serial".into();let mut b=a.clone();b.path="another-channel".into();
        assert_ne!(candidate(&a).key,candidate(&b).key);
        assert!(Arc::ptr_eq(&channel_lock(&a.path),&channel_lock(&a.path.to_uppercase())));
        assert!(!Arc::ptr_eq(&channel_lock(&a.path),&channel_lock(&b.path)));
    }
    #[test]
    fn confirmed_identity_is_stable_when_the_same_target_is_read_again() {
        let (s,target)=session();
        assert_eq!(s.target(&target).key,target.key);
    }
    #[test]
    fn externally_changed_row_is_not_overwritten_by_a_stale_prepared_write() {
        let (mut s,_)=session();let original=s.channel.bytes(0,10);
        let mut next=original.clone();next[..2].copy_from_slice(&pair(2));
        s.channel.memory.insert(7,0xfe);
        assert!(s.write_verified(0,&next,&original).is_err());
        assert!(s.channel.writes.is_empty());
    }
}
