//! Razer 90-byte protocol over Windows Feature Reports (report ID + payload).
//! Capability data, live unit identity and complete readback gate settings writes.
use super::razer_capabilities::{self, Capabilities, RateEncoding};
use super::*;
use crate::device::{DpiStage, PollingRate, StageContext};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

pub struct Razer;
const LENGTH: usize = 91;
const TRANSACTIONS: [u8; 3] = [0xff, 0x1f, 0x3f];

fn control(i: &HidInterface) -> bool {
    i.vendor_id == 0x1532
        && (i.feature_len == LENGTH as u16
            || i.feature_len == 0
                && i.usage_page == 1
                && i.usage == 2
                && razer_capabilities::for_product(i.product_id).is_some())
}

// Keep the Windows instance and USB interface; only sibling collection suffixes are removed.
pub(super) fn collection_group(i: &HidInterface) -> String {
    super::channel::collection_group(i)
}

fn packet(tx: u8, class: u8, command: u8, length: u8, data: &[u8]) -> [u8; LENGTH] {
    let mut p = [0; LENGTH];
    p[2] = tx;
    p[6] = length;
    p[7] = class;
    p[8] = command;
    p[9..9 + data.len()].copy_from_slice(data);
    p[89] = checksum(&p);
    p
}
fn checksum(p: &[u8]) -> u8 {
    p[3..89].iter().fold(0, |value, byte| value ^ byte)
}

#[derive(Debug, PartialEq, Eq)]
enum Reply {
    Waiting,
    Rejected(u8),
    Data(Vec<u8>),
}
fn parse(p: &[u8], request: &[u8], minimum: usize) -> Option<Reply> {
    if p.len() != LENGTH
        || p[0] != 0
        || p[2] != request[2]
        || p[3..6] != [0, 0, 0]
        || p[7..9] != request[7..9]
        || p[6] > 80
        || p[89] != checksum(p)
        || p[90] != 0
    {
        return None;
    }
    match p[1] {
        0 | 1 => Some(Reply::Waiting),
        2 if usize::from(p[6]) >= minimum => {
            Some(Reply::Data(p[9..9 + usize::from(p[6])].to_vec()))
        }
        3..=5 => Some(Reply::Rejected(p[1])),
        _ => None,
    }
}

trait Channel {
    fn send(&mut self, packet: &[u8], timeout: u32) -> Result<(), String>;
    fn receive(&mut self, timeout: u32) -> Result<Vec<u8>, String>;
    fn pause(&self, _: u64) {}
}
impl Channel for feature::FeatureChannel {
    fn send(&mut self, p: &[u8], timeout: u32) -> Result<(), String> {
        self.send(p, timeout)
    }
    fn receive(&mut self, timeout: u32) -> Result<Vec<u8>, String> {
        self.receive(0, timeout)
    }
    fn pause(&self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Failure {
    NoReply,
    Rejected(u8),
}
struct Session<C> {
    channel: C,
    interface: HidInterface,
    transactions: BTreeMap<(u8, u8), u8>,
}
impl<C: Channel> Session<C> {
    fn new(channel: C, interface: &HidInterface) -> Self {
        Self {
            channel,
            interface: interface.clone(),
            transactions: BTreeMap::new(),
        }
    }
    fn exchange(&mut self, p: &[u8], minimum: usize) -> Result<Vec<u8>, Failure> {
        let started = Instant::now();
        let result = (|| {
            self.channel.send(p, 150).map_err(|_| Failure::NoReply)?;
            // Poll a pending response before resending. Some receivers return the previous
            // command or an unprocessed request while the wireless device wakes up.
            for attempt in 0..5 {
                self.channel.pause(if attempt == 0 { 60 } else { 40 });
                let Some(left) = Duration::from_millis(600).checked_sub(started.elapsed()) else {
                    break;
                };
                let response = match self
                    .channel
                    .receive(left.as_millis().max(1).min(150) as u32)
                {
                    Ok(response) => response,
                    Err(_) => continue,
                };
                match parse(&response, p, minimum) {
                    Some(Reply::Data(data)) => return Ok(data),
                    Some(Reply::Rejected(status)) => return Err(Failure::Rejected(status)),
                    _ => {}
                }
            }
            Err(Failure::NoReply)
        })();
        trace::record(
            &self.interface,
            BackendId::Razer,
            if p[8] & 0x80 != 0 { "query" } else { "set" },
            u16::from(p[7]),
            p[8],
            LENGTH,
            if result.is_ok() { LENGTH } else { 0 },
            started,
            match &result {
                Ok(_) => trace::Outcome::Ok,
                Err(Failure::NoReply) => trace::Outcome::NoReply,
                Err(Failure::Rejected(status)) => trace::Outcome::Rejected(u32::from(*status)),
            },
        );
        result
    }
    fn query(
        &mut self,
        class: u8,
        command: u8,
        length: u8,
        data: &[u8],
        minimum: usize,
    ) -> Result<Vec<u8>, Failure> {
        let cached = self.transactions.get(&(class, command)).copied();
        let ids: Vec<_> = cached
            .into_iter()
            .chain(TRANSACTIONS.into_iter().filter(|id| Some(*id) != cached))
            .collect();
        for tx in ids {
            match self.exchange(&packet(tx, class, command, length, data), minimum) {
                Ok(data) => {
                    self.transactions.insert((class, command), tx);
                    return Ok(data);
                }
                Err(Failure::Rejected(5)) => return Err(Failure::Rejected(5)),
                // Failure/timeout does not become a persistent negative capability cache.
                Err(_) => {}
            }
        }
        Err(Failure::NoReply)
    }
    fn battery(&mut self) -> Option<(Battery, Instant)> {
        let data = self.query(7, 0x80, 2, &[], 2).ok()?;
        let measured = Instant::now();
        let charging = self
            .query(7, 0x84, 2, &[], 2)
            .ok()
            .and_then(|r| match r[1] {
                0 => Some(false),
                1 => Some(true),
                _ => None,
            });
        Some((
            Battery {
                percent: Some(percent(data[1])),
                charging,
                voltage_mv: None,
                approximate: false,
                level: None,
            },
            measured,
        ))
    }
    fn dpi(&mut self) -> Dpi {
        let mut dpi = Dpi::default();
        let caps = razer_capabilities::for_product(self.interface.product_id);
        let storage = caps.map_or(0, |c| c.dpi_storage);
        if let Some(r) = self.current_dpi(storage) {
            if let Some((x, y)) = axes(&r[1..5]) {
                dpi.current = Some(x);
                dpi.current_y = Some(y);
            }
        }
        // Only a confirmed mouse DPI reply authorizes a stage-table query.
        if dpi.current.is_some() {
            if let Some(table) = self.table(caps.map_or(1, |c| c.stage_storage)) {
                table.populate(&mut dpi);
            }
        }
        dpi
    }
    fn current_dpi(&mut self, storage: u8) -> Option<Vec<u8>> {
        self.query(4, 0x85, 7, &[storage], 7)
            .ok()
            .filter(|r| r.len() == 7 && r[0] <= 1 && axes(&r[1..5]).is_some())
            .map(|mut r| {
                r[0] = storage;
                r
            })
    }
    fn table(&mut self, storage: u8) -> Option<StageTable> {
        StageTable::decode(self.query(4, 0x86, 38, &[storage], 38).ok()?, storage)
    }
    fn rate(&mut self) -> PollingRate {
        let known = razer_capabilities::for_product(self.interface.product_id);
        let current = if let Some(caps) = known {
            self.rate_value(caps.rate)
        } else {
            match self.query(0, 0xc0, 2, &[0], 2) {
                Ok(r) => decode_rate(RateEncoding::Extended { commit: false }, &r),
                Err(Failure::Rejected(5)) => self
                    .query(0, 0x85, 1, &[], 1)
                    .ok()
                    .and_then(|r| decode_rate(RateEncoding::Legacy, &r)),
                _ => None,
            }
        };
        // A current-rate code is not a capability list; keep settings unavailable.
        PollingRate {
            current,
            ..Default::default()
        }
    }
    fn rate_value(&mut self, encoding: RateEncoding) -> Option<u16> {
        let (cmd, len) = match encoding {
            RateEncoding::Legacy => (0x85, 1),
            _ => (0xc0, 2),
        };
        self.query(0, cmd, len, &[], usize::from(len))
            .ok()
            .and_then(|r| decode_rate(encoding, &r))
    }
    fn identity(&mut self) -> Option<String> {
        let data = self.query(0, 0x82, 22, &[], 22).ok()?;
        if data.len() != 22 {
            return None;
        }
        let end = data.iter().position(|v| *v == 0).unwrap_or(data.len());
        if data[end..].iter().any(|v| *v != 0) {
            return None;
        }
        let serial = &data[..end];
        if serial.len() < 6
            || !serial
                .iter()
                .all(|v| v.is_ascii_alphanumeric() || matches!(v, b'-' | b'_'))
            || serial.iter().all(|v| *v == serial[0])
        {
            return None;
        }
        let serial = std::str::from_utf8(serial).ok()?;
        if ["unknown", "default", "serial", "1234567890", "000000000000"]
            .iter()
            .any(|v| serial.eq_ignore_ascii_case(v))
        {
            return None;
        }
        Some(crate::device::stable_key(&format!(
            "Razer:{}:{serial}",
            collection_group(&self.interface)
        )))
    }
    fn normal_mode(&mut self) -> bool {
        self.query(0, 0x84, 2, &[], 2).is_ok_and(|r| r == [0, 0])
    }
    fn check(&mut self, target: &Target) -> Result<Capabilities, String> {
        let caps = razer_capabilities::for_product(self.interface.product_id)
            .ok_or("此连接尚未确认设置写入协议")?;
        if !target.verified_identity || self.identity().as_deref() != Some(target.key.as_str()) {
            return Err("设备身份已变化，请刷新后重试".into());
        }
        if !self.normal_mode() {
            return Err("设备模式不允许设置，请刷新后重试".into());
        }
        Ok(caps)
    }
    fn write_once(&mut self, class: u8, getter: u8, data: &[u8]) -> Result<(), String> {
        let tx = *self
            .transactions
            .get(&(class, getter))
            .ok_or("此连接尚未确认设置写入协议")?;
        match self.exchange(&packet(tx, class, getter & 0x7f, data.len() as u8, data), 0) {
            Err(Failure::Rejected(_)) => Err("设备拒绝设置，请刷新后检查".into()),
            // A lost ACK is followed by a read, never another configuration write.
            _ => Ok(()),
        }
    }
    fn numeric(&mut self, target: &Target, x: u16) -> Result<(), String> {
        let caps = self.check(target)?;
        if !(100..=caps.max_dpi).contains(&x) {
            return Err("DPI 不在设备报告的可用范围内".into());
        }
        let original = self
            .current_dpi(caps.dpi_storage)
            .ok_or("无法读取当前 DPI")?;
        let (old_x, old_y) = axes(&original[1..5]).ok_or("无法读取当前 DPI")?;
        if old_x != old_y {
            return Err("请分别设置 X/Y DPI".into());
        }
        if old_x == x {
            return Ok(());
        }
        let mut expected = original.clone();
        expected[1..3].copy_from_slice(&x.to_be_bytes());
        expected[3..5].copy_from_slice(&x.to_be_bytes());
        self.check(target)?;
        if self.current_dpi(caps.dpi_storage).as_ref() != Some(&original) {
            return Err("设备配置已变化，请刷新后重试".into());
        }
        self.write_once(4, 0x85, &expected)?;
        let confirmed = self.current_dpi(caps.dpi_storage);
        self.check(target)?;
        if confirmed.as_ref() != Some(&expected) {
            return Err("无法确认设备已保存设置，请刷新后检查".into());
        }
        Ok(())
    }
    fn change_stage(
        &mut self,
        target: &Target,
        stage: DpiStage,
        context: StageContext,
        edit: Option<(u16, u16, u16)>,
    ) -> Result<(), String> {
        let caps = self.check(target)?;
        let original = self
            .table(caps.stage_storage)
            .ok_or("设备档位或配置已变化，请刷新后重试")?;
        if context != StageContext::Configuration(original.token())
            || !original.stages().contains(&stage)
        {
            return Err("设备档位或配置已变化，请刷新后重试".into());
        }
        if !(0..original.raw[2]).all(|i| {
            original.axes(i).is_some_and(|(x, y)| {
                (100..=caps.max_dpi).contains(&x) && (100..=caps.max_dpi).contains(&y)
            })
        }) {
            return Err("DPI 不在设备报告的可用范围内".into());
        }
        let mut expected = original.clone();
        let offset = 3 + usize::from(stage.index) * 7;
        if let Some((old_y, x, y)) = edit {
            if original.axes(stage.index).map(|v| v.1) != Some(old_y) {
                return Err("设备档位或配置已变化，请刷新后重试".into());
            }
            if ![x, y].iter().all(|v| (100..=caps.max_dpi).contains(v)) {
                return Err("DPI 不在设备报告的可用范围内".into());
            }
            expected.raw[offset + 1..offset + 3].copy_from_slice(&x.to_be_bytes());
            expected.raw[offset + 3..offset + 5].copy_from_slice(&y.to_be_bytes());
        } else {
            expected.raw[1] = stage.index + 1;
        }
        if expected == original {
            return Ok(());
        }
        // Re-read the entire configuration, not only the edited row. This guards
        // other stages, the active stage, reserved bytes and the storage selector.
        self.check(target)?;
        if self.table(caps.stage_storage).as_ref() != Some(&original) {
            return Err("设备档位或配置已变化，请刷新后重试".into());
        }
        self.write_once(4, 0x86, &expected.write_bytes())?;
        let confirmed = self.table(caps.stage_storage);
        let current = self
            .current_dpi(caps.dpi_storage)
            .and_then(|r| axes(&r[1..5]));
        self.check(target)?;
        if !confirmed.is_some_and(|table| table.write_bytes() == expected.write_bytes())
            || current != expected.axes(expected.raw[1] - 1)
        {
            return Err("无法确认设备已保存设置，请刷新后检查".into());
        }
        Ok(())
    }
    fn set_rate(&mut self, target: &Target, hz: u16) -> Result<(), String> {
        let caps = self.check(target)?;
        if !caps.rates.contains(&hz) {
            return Err("回报率不在此连接的可用范围内".into());
        }
        let original = self.rate_value(caps.rate).ok_or("无法读取当前回报率")?;
        if original == hz {
            return Ok(());
        }
        self.check(target)?;
        if self.rate_value(caps.rate) != Some(original) {
            return Err("设备配置已变化，请刷新后重试".into());
        }
        let getter = match caps.rate {
            RateEncoding::Legacy => 0x85,
            _ => 0xc0,
        };
        let data = match caps.rate {
            RateEncoding::Legacy => vec![(1000 / hz) as u8],
            _ => vec![0, (8000 / hz) as u8],
        };
        self.write_once(0, getter, &data)?;
        self.channel.pause(150);
        if self.rate_value(caps.rate) != Some(hz) {
            return Err("无法确认设备已保存设置，请刷新后检查".into());
        }
        self.check(target)?;
        if caps.rate == (RateEncoding::Extended { commit: true }) {
            if self.rate_value(caps.rate) != Some(hz) {
                return Err("设备配置已变化，请刷新后重试".into());
            }
            // Viper V3 Pro requires a separate persistent selector, on 0x1f.
            // Never retry either write if the receiver loses the response.
            if matches!(
                self.exchange(&packet(0x1f, 0, 0x40, 2, &[1, data[1]]), 0),
                Err(Failure::Rejected(_))
            ) {
                return Err("设备拒绝设置，请刷新后检查".into());
            }
            self.channel.pause(150);
            if self.rate_value(caps.rate) != Some(hz) {
                return Err("无法确认设备已保存设置，请刷新后检查".into());
            }
            self.check(target)?;
        }
        Ok(())
    }
}
fn percent(raw: u8) -> u8 {
    ((u16::from(raw) * 100 + 127) / 255) as u8
}
fn axes(bytes: &[u8]) -> Option<(u16, u16)> {
    if bytes.len() != 4 {
        return None;
    }
    let x = u16::from_be_bytes([bytes[0], bytes[1]]);
    let y = u16::from_be_bytes([bytes[2], bytes[3]]);
    (x != 0 && y != 0 && x != u16::MAX && y != u16::MAX).then_some((x, y))
}
fn decode_rate(encoding: RateEncoding, r: &[u8]) -> Option<u16> {
    match encoding {
        RateEncoding::Legacy if r.len() == 1 => match r[0] {
            1 => Some(1000),
            2 => Some(500),
            8 => Some(125),
            _ => None,
        },
        RateEncoding::Extended { .. } if r.len() == 2 && r[0] <= 1 => match r[1] {
            1 => Some(8000),
            2 => Some(4000),
            4 => Some(2000),
            8 => Some(1000),
            16 => Some(500),
            32 => Some(250),
            64 => Some(125),
            _ => None,
        },
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StageTable {
    raw: Vec<u8>,
    storage: u8,
}
impl StageTable {
    fn decode(raw: Vec<u8>, storage: u8) -> Option<Self> {
        // Real captures return 0 here even when the caller selected persistent storage.
        // Keep the request selector separately; this field is not a profile identity.
        if raw.len() != 38
            || raw[0] > 1
            || storage > 1
            || !(1..=5).contains(&raw[2])
            || !(1..=raw[2]).contains(&raw[1])
        {
            return None;
        }
        let base = raw[3];
        if base > 1 {
            return None;
        }
        for (index, row) in raw[3..3 + usize::from(raw[2]) * 7]
            .chunks_exact(7)
            .enumerate()
        {
            if row[0] != index as u8 + base || axes(&row[1..5]).is_none() {
                return None;
            }
        }
        Some(Self { raw, storage })
    }
    fn axes(&self, index: u8) -> Option<(u16, u16)> {
        if index >= self.raw[2] {
            return None;
        }
        let offset = 4 + usize::from(index) * 7;
        axes(&self.raw[offset..offset + 4])
    }
    fn stages(&self) -> Vec<DpiStage> {
        (0..self.raw[2])
            .map(|index| DpiStage {
                index,
                value: self.axes(index).unwrap().0,
            })
            .collect()
    }
    fn token(&self) -> u64 {
        std::iter::once(&self.storage)
            .chain(self.raw.iter())
            .fold(0xcbf29ce484222325u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
            })
    }
    fn write_bytes(&self) -> Vec<u8> {
        let mut bytes = self.raw.clone();
        bytes[0] = self.storage;
        // The write protocol numbers row records from zero; active remains one-based.
        for index in 0..bytes[2] {
            bytes[3 + usize::from(index) * 7] = index;
        }
        bytes
    }
    fn populate(&self, dpi: &mut Dpi) {
        dpi.stages = self.stages();
        dpi.active_stage = Some(self.raw[1] - 1);
        dpi.stage_y = (0..self.raw[2])
            .map(|i| (i, self.axes(i).unwrap().1))
            .collect();
        dpi.configuration_token = Some(self.token());
    }
}

impl DeviceBackend for Razer {
    fn discover(&self, _: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        // Probe all matching collections; a silent collection cannot permanently hide its sibling.
        Ok(interfaces
            .iter()
            .filter(|i| control(i))
            .map(|i| {
                let mut target = Target::new(BackendId::Razer, i, 0);
                target.family = crate::device::family_of(&i.product);
                if target.name.is_empty() {
                    target.name = "Razer".into();
                }
                target
            })
            .collect())
    }
    fn read(&self, _: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        if !control(&target.interface) {
            return Err(crate::tr!("设备功能报告长度无效").into());
        }
        let lock = channel::lock(&collection_group(&target.interface));
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut s = Session::new(
            feature::FeatureChannel::open_with_length(&target.interface, LENGTH)?,
            &target.interface,
        );
        let (battery, battery_at) = match s.battery() {
            Some((battery, at)) => (Some(battery), Some(at)),
            None => (None, None),
        };
        let mut dpi = s.dpi();
        let mut target = target.clone();
        let mut polling_rate = if dpi.current.is_some() {
            target.family = DeviceFamily::Mouse;
            s.rate()
        } else {
            PollingRate::default()
        };
        if battery.is_none() && dpi.current.is_none() {
            return Err(crate::i18n::translate("Razer 设备无有效应答").into());
        }
        if let Some(key) = s.identity() {
            target.key = key;
            target.verified_identity = true;
        }
        if target.verified_identity {
            if let Some(caps) = razer_capabilities::for_product(target.interface.product_id)
                .filter(|_| s.normal_mode())
            {
                dpi.values = (100..=caps.max_dpi).collect();
                dpi.write_effect = Some(if caps.dpi_storage == 0 { crate::device::DpiWriteEffect::Current }
                    else { crate::device::DpiWriteEffect::Stored });
                dpi.writable = dpi.current.is_some() && dpi.current == dpi.current_y;
                dpi.stage_editable = !dpi.stages.is_empty()
                    && dpi.stages.iter().all(|stage| {
                        (100..=caps.max_dpi).contains(&stage.value)
                            && dpi
                                .stage_y
                                .get(&stage.index)
                                .is_some_and(|y| (100..=caps.max_dpi).contains(y))
                    });
                dpi.stage_writable = dpi.stage_editable;
                if polling_rate
                    .current
                    .is_some_and(|hz| caps.rates.contains(&hz))
                {
                    polling_rate.values = caps.rates.to_vec();
                    polling_rate.writable = true;
                }
            }
        }
        Ok(Snapshot {
            target,
            battery,
            battery_at,
            dpi,
            polling_rate,
            connected: true,
        })
    }
    fn set_dpi(&self, _: &HidApi, target: &Target, dpi: u16) -> Result<(), String> {
        with_session(target, |s| s.numeric(target, dpi))
    }
    fn set_dpi_stage(
        &self,
        _: &HidApi,
        target: &Target,
        stage: DpiStage,
        context: StageContext,
    ) -> Result<(), String> {
        with_session(target, |s| s.change_stage(target, stage, context, None))
    }
    fn edit_dpi_stage(
        &self,
        _: &HidApi,
        target: &Target,
        stage: DpiStage,
        context: StageContext,
        expected_y: u16,
        x: u16,
        y: u16,
    ) -> Result<(), String> {
        with_session(target, |s| {
            s.change_stage(target, stage, context, Some((expected_y, x, y)))
        })
    }
    fn set_polling_rate(&self, _: &HidApi, target: &Target, hz: u16) -> Result<(), String> {
        with_session(target, |s| s.set_rate(target, hz))
    }
}

fn with_session(
    target: &Target,
    f: impl FnOnce(&mut Session<feature::FeatureChannel>) -> Result<(), String>,
) -> Result<(), String> {
    if !control(&target.interface) {
        return Err(crate::tr!("设备功能报告长度无效").into());
    }
    let lock = channel::lock(&collection_group(&target.interface));
    let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
    f(&mut Session::new(
        feature::FeatureChannel::open_with_length(&target.interface, LENGTH)?,
        &target.interface,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    fn interface(path: &str) -> HidInterface {
        HidInterface {
            path: path.into(),
            vendor_id: 0x1532,
            product_id: 0x007b,
            usage_page: 1,
            usage: 2,
            input_len: 8,
            output_len: 0,
            feature_len: 91,
            product: "Razer Mouse".into(),
            serial: "shared".into(),
        }
    }
    fn reply(request: &[u8], status: u8, data: &[u8]) -> Vec<u8> {
        let mut p = packet(request[2], request[7], request[8], data.len() as u8, data);
        p[1] = status;
        p.to_vec()
    }
    #[test]
    fn percentage_is_monotonic_across_the_raw_100_boundary() {
        assert_eq!(
            (percent(0), percent(100), percent(101), percent(255)),
            (0, 39, 40, 100)
        );
        for raw in 0..255 {
            assert!(percent(raw) <= percent(raw + 1));
        }
    }
    #[test]
    fn matching_requires_complete_crc_transaction_command_and_payload() {
        let request = packet(0xff, 7, 0x80, 2, &[]);
        let good = reply(&request, 2, &[0, 128]);
        assert_eq!(parse(&good, &request, 2), Some(Reply::Data(vec![0, 128])));
        for offset in [0, 2, 3, 4, 5, 7, 8, 10, 89, 90] {
            let mut bad = good.clone();
            bad[offset] ^= 1;
            assert!(parse(&bad, &request, 2).is_none());
        }
        assert!(parse(&good[..90], &request, 2).is_none());
        assert!(parse(&reply(&request, 2, &[0]), &request, 2).is_none());
        assert_eq!(
            parse(&reply(&request, 5, &[]), &request, 2),
            Some(Reply::Rejected(5))
        );
    }
    struct Stream {
        frames: VecDeque<Vec<u8>>,
        sent: Vec<Vec<u8>>,
    }
    impl Channel for Stream {
        fn send(&mut self, p: &[u8], _: u32) -> Result<(), String> {
            self.sent.push(p.to_vec());
            Ok(())
        }
        fn receive(&mut self, _: u32) -> Result<Vec<u8>, String> {
            self.frames.pop_front().ok_or("empty".into())
        }
    }
    #[test]
    fn old_busy_and_echo_reports_are_polled_without_resending() {
        let request = packet(0xff, 7, 0x80, 2, &[]);
        let old = reply(&packet(0xff, 7, 0x84, 2, &[]), 2, &[0, 1]);
        let channel = Stream {
            frames: [
                old,
                request.to_vec(),
                reply(&request, 1, &[]),
                reply(&request, 2, &[0, 0]),
            ]
            .into(),
            sent: Vec::new(),
        };
        let mut s = Session::new(channel, &interface("one"));
        assert_eq!(s.query(7, 0x80, 2, &[], 2), Ok(vec![0, 0]));
        assert_eq!(s.channel.sent.len(), 1);
    }
    #[test]
    fn charging_uses_separate_query_and_can_remain_unknown() {
        let b = packet(0xff, 7, 0x80, 2, &[]);
        let c = packet(0xff, 7, 0x84, 2, &[]);
        let channel = Stream {
            frames: [reply(&b, 2, &[1, 128]), reply(&c, 5, &[])].into(),
            sent: Vec::new(),
        };
        let mut s = Session::new(channel, &interface("one"));
        let (battery, _) = s.battery().unwrap();
        assert_eq!(battery.percent, Some(50));
        assert_eq!(battery.charging, None);
    }
    #[test]
    fn silent_read_is_bounded_and_can_recover_on_next_poll() {
        let mut s = Session::new(
            Stream {
                frames: VecDeque::new(),
                sent: Vec::new(),
            },
            &interface("one"),
        );
        assert_eq!(s.query(7, 0x80, 2, &[], 2), Err(Failure::NoReply));
        assert_eq!(s.channel.sent.len(), 3);
        s.channel
            .frames
            .push_back(reply(&packet(0xff, 7, 0x80, 2, &[]), 2, &[0, 200]));
        assert!(s.query(7, 0x80, 2, &[], 2).is_ok());
    }
    #[test]
    fn stages_keep_ids_duplicates_and_independent_axes_without_inventing_a_profile() {
        let mut dpi = Dpi::default();
        let mut r = vec![1, 2, 2, 0, 3, 32, 6, 64, 0, 0, 1, 3, 32, 3, 32, 0, 0];
        r.resize(38, 0);
        StageTable::decode(r.clone(), 1).unwrap().populate(&mut dpi);
        assert_eq!(
            dpi.stages,
            vec![
                DpiStage {
                    index: 0,
                    value: 800
                },
                DpiStage {
                    index: 1,
                    value: 800
                }
            ]
        );
        assert_eq!(dpi.stage_y[&0], 1600);
        assert_eq!(dpi.active_stage, Some(1));
        assert!(dpi.profile.is_none());
        assert!(dpi.configuration_token.is_some());
        assert!(!dpi.stage_writable && !dpi.stage_editable);
        let mut one_based = r.clone();
        one_based[3] = 1;
        one_based[10] = 2;
        assert_eq!(StageTable::decode(one_based, 1).unwrap().write_bytes(), r);
        r[10] = 0;
        assert!(StageTable::decode(r.clone(), 1).is_none());
        r[10] = 1;
        r.truncate(16);
        assert!(StageTable::decode(r, 1).is_none());
    }
    #[test]
    fn rate_generations_are_distinct_and_do_not_invent_a_supported_list() {
        let newer = packet(0xff, 0, 0xc0, 1, &[]);
        let older = packet(0xff, 0, 0x85, 1, &[]);
        let mut s = Session::new(
            Stream {
                frames: [reply(&newer, 5, &[]), reply(&older, 2, &[8])].into(),
                sent: Vec::new(),
            },
            &interface("one"),
        );
        let rate = s.rate();
        assert_eq!(rate.current, Some(125));
        assert!(rate.values.is_empty());
        assert!(!rate.writable);
        s.channel.frames.push_back(reply(&newer, 2, &[0, 8]));
        assert_eq!(s.rate().current, Some(1000));
    }
    #[test]
    fn same_model_receivers_keep_separate_instance_identity() {
        let a = interface("hid#vid_1532&pid_007b&mi_00&col01#7&aaaa&0&0001#{hid}");
        let sibling = interface("hid#vid_1532&pid_007b&mi_00&col02#7&aaaa&0&0002#{hid}");
        let b = interface("hid#vid_1532&pid_007b&mi_00&col01#7&bbbb&0&0001#{hid}");
        assert_eq!(collection_group(&a), collection_group(&sibling));
        assert_ne!(collection_group(&a), collection_group(&b));
        let mut unsupported = a.clone();
        unsupported.feature_len = 90;
        assert!(!control(&unsupported));
        unsupported.feature_len = 0;
        unsupported.product_id = 0x00c1;
        assert!(control(&unsupported));
        unsupported.usage = 6;
        assert!(!control(&unsupported));
    }

    struct Mouse {
        current: Vec<u8>,
        table: Vec<u8>,
        rate: u8,
        serial: Vec<u8>,
        mode: u8,
        pending: Option<Vec<u8>>,
        writes: Vec<(u8, u8, Vec<u8>, u8)>,
        apply: bool,
        ack: bool,
        reject: bool,
        replace_after_write: bool,
        table_reads: usize,
        change_table_at: usize,
        corrupt_neighbor: bool,
    }
    impl Mouse {
        fn new() -> Self {
            let mut table = vec![
                1, 1, 2, 0, 3, 32, 6, 64, 0xa1, 0xb2, 1, 6, 64, 12, 128, 0xc3, 0xd4,
            ];
            table.resize(38, 0);
            table[37] = 0x7a;
            let mut serial = b"RZUNIT123456".to_vec();
            serial.resize(22, 0);
            Self {
                current: vec![1, 3, 32, 6, 64, 0, 0],
                table,
                rate: 8,
                serial,
                mode: 0,
                pending: None,
                writes: Vec::new(),
                apply: true,
                ack: true,
                reject: false,
                replace_after_write: false,
                table_reads: 0,
                change_table_at: usize::MAX,
                corrupt_neighbor: false,
            }
        }
    }
    impl Channel for Mouse {
        fn send(&mut self, p: &[u8], _: u32) -> Result<(), String> {
            assert_eq!(p.len(), 91);
            assert_eq!(p[89], checksum(p));
            let (class, cmd) = (p[7], p[8]);
            let getter = cmd | 0x80;
            let tx = if class == 4 && getter == 0x86 {
                0xff
            } else {
                0x1f
            };
            self.pending = None;
            if p[2] != tx {
                return Ok(());
            }
            let args = &p[9..9 + usize::from(p[6])];
            let data = match (class, cmd) {
                (0, 0x82) => self.serial.clone(),
                (0, 0x84) => vec![self.mode, 0],
                (4, 0x85) => {
                    let mut data = self.current.clone();
                    data[0] = args[0];
                    data
                }
                (4, 0x86) => {
                    self.table_reads += 1;
                    if self.table_reads == self.change_table_at {
                        self.table[16] ^= 1;
                    }
                    let mut data = self.table.clone();
                    data[0] = args[0];
                    data
                }
                (0, 0xc0) => {
                    assert_eq!(p[6], 2);
                    vec![0, self.rate]
                }
                (0, 0x85) => vec![self.rate],
                (_, 0x05 | 0x06 | 0x40) => {
                    self.writes.push((class, cmd, args.to_vec(), p[2]));
                    if self.reject {
                        self.pending = Some(reply(p, 3, &[]));
                        return Ok(());
                    }
                    if self.apply {
                        match (class, cmd) {
                            (4, 0x05) => self.current = args.to_vec(),
                            (4, 0x06) => {
                                assert_eq!(args.len(), 38);
                                self.table = args.to_vec();
                                if self.corrupt_neighbor {
                                    self.table[16] ^= 1;
                                }
                                let offset = 4 + usize::from(self.table[1] - 1) * 7;
                                self.current[1..5].copy_from_slice(&self.table[offset..offset + 4]);
                            }
                            (0, 0x05) => self.rate = args[0],
                            (0, 0x40) => self.rate = args[1],
                            _ => panic!("unexpected write"),
                        }
                    }
                    if self.replace_after_write {
                        self.serial[6] = b'X';
                    }
                    if !self.ack {
                        return Ok(());
                    }
                    args.to_vec()
                }
                _ => {
                    self.pending = Some(reply(p, 5, &[]));
                    return Ok(());
                }
            };
            self.pending = Some(reply(p, 2, &data));
            Ok(())
        }
        fn receive(&mut self, _: u32) -> Result<Vec<u8>, String> {
            self.pending.take().ok_or("no report".into())
        }
    }
    fn mouse() -> (Session<Mouse>, Target) {
        let mut i = interface("hid#vid_1532&pid_00c1&mi_00&col01#unit&0001#{hid}");
        i.product_id = 0x00c1;
        let mut s = Session::new(Mouse::new(), &i);
        let mut target = Target::new(BackendId::Razer, &i, 0);
        target.key = s.identity().unwrap();
        target.verified_identity = true;
        (s, target)
    }
    fn edit_context(s: &mut Session<Mouse>) -> (DpiStage, StageContext) {
        let table = s.table(1).unwrap();
        (
            table.stages()[1],
            StageContext::Configuration(table.token()),
        )
    }
    #[test]
    fn stage_edit_preserves_other_axes_reserved_bytes_unused_slots_and_active_stage() {
        let (mut s, target) = mouse();
        let (stage, context) = edit_context(&mut s);
        let mut expected = s.channel.table.clone();
        expected[11..15].copy_from_slice(&[12, 128, 25, 0]);
        s.change_stage(&target, stage, context, Some((3200, 3200, 6400)))
            .unwrap();
        assert_eq!(s.channel.table, expected);
        assert_eq!(s.channel.writes.len(), 1);
        assert_eq!(s.channel.writes[0], (4, 6, expected, 0xff));
        assert_eq!(s.channel.current[1..5], [3, 32, 6, 64]);
    }
    #[test]
    fn switch_uses_one_based_active_and_zero_based_records_then_verifies_current_axes() {
        let (mut s, target) = mouse();
        let (stage, context) = edit_context(&mut s);
        s.change_stage(&target, stage, context, None).unwrap();
        assert_eq!(s.channel.table[1], 2);
        assert_eq!(s.channel.table[3], 0);
        assert_eq!(s.channel.table[10], 1);
        assert_eq!(s.channel.current[1..5], [6, 64, 12, 128]);
        assert_eq!(s.channel.writes.len(), 1);
    }
    #[test]
    fn stale_full_table_and_last_moment_neighbor_changes_block_writes() {
        let (mut s, target) = mouse();
        let (stage, context) = edit_context(&mut s);
        s.channel.table[9] ^= 1;
        assert!(s.change_stage(&target, stage, context, None).is_err());
        assert!(s.channel.writes.is_empty());
        let (stage, context) = edit_context(&mut s);
        s.channel.change_table_at = s.channel.table_reads + 2;
        assert!(s.change_stage(&target, stage, context, None).is_err());
        assert!(s.channel.writes.is_empty());
    }
    #[test]
    fn lost_ack_reads_back_once_ignored_write_and_neighbor_corruption_fail_without_rollback() {
        for (apply, corrupt, success) in [
            (true, false, true),
            (false, false, false),
            (true, true, false),
        ] {
            let (mut s, target) = mouse();
            let (stage, context) = edit_context(&mut s);
            s.channel.ack = false;
            s.channel.apply = apply;
            s.channel.corrupt_neighbor = corrupt;
            assert_eq!(
                s.change_stage(&target, stage, context, None).is_ok(),
                success
            );
            assert_eq!(s.channel.writes.len(), 1);
        }
    }
    #[test]
    fn identity_mode_range_and_old_y_are_checked_before_any_stage_write() {
        for case in 0..5 {
            let (mut s, mut target) = mouse();
            let (stage, context) = edit_context(&mut s);
            let mut expected_y = 3200;
            let mut x = 800;
            match case {
                0 => target.verified_identity = false,
                1 => s.channel.serial[6] = b'X',
                2 => s.channel.mode = 3,
                3 => expected_y = 1600,
                _ => x = 35001,
            }
            assert!(s
                .change_stage(&target, stage, context, Some((expected_y, x, 800)))
                .is_err());
            assert!(s.channel.writes.is_empty());
        }
    }
    #[test]
    fn replaced_device_or_explicit_rejection_never_reports_write_success() {
        for reject in [false, true] {
            let (mut s, target) = mouse();
            let (stage, context) = edit_context(&mut s);
            s.channel.replace_after_write = !reject;
            s.channel.reject = reject;
            assert!(s.change_stage(&target, stage, context, None).is_err());
            assert_eq!(s.channel.writes.len(), 1);
        }
    }
    #[test]
    fn numeric_write_requires_equal_axes_and_confirms_both_without_changing_modes() {
        let (mut s, target) = mouse();
        assert!(s.numeric(&target, 1600).is_err());
        assert!(s.channel.writes.is_empty());
        s.channel.current[3..5].copy_from_slice(&[3, 32]);
        s.numeric(&target, 1600).unwrap();
        assert_eq!(s.channel.current[1..5], [6, 64, 6, 64]);
        assert_eq!(s.channel.writes.len(), 1);
        assert_eq!(s.channel.writes[0].1, 5);
    }
    #[test]
    fn extended_rate_requires_connection_limits_and_separate_persistent_commit() {
        let (mut s, target) = mouse();
        s.set_rate(&target, 4000).unwrap();
        assert_eq!(
            s.channel.writes,
            vec![(0, 0x40, vec![0, 2], 0x1f), (0, 0x40, vec![1, 2], 0x1f)]
        );
        assert!(s.set_rate(&target, 250).is_err());
        assert_eq!(s.channel.writes.len(), 2);
        let (mut s, mut target) = mouse();
        s.interface.product_id = 0x00a6;
        target.interface.product_id = 0x00a6;
        assert!(s.set_rate(&target, 4000).is_err());
        assert!(s.channel.writes.is_empty());
        s.set_rate(&target, 500).unwrap();
        assert_eq!(s.channel.writes, vec![(0, 0x40, vec![0, 16], 0x1f)]);
    }
    #[test]
    fn legacy_rate_uses_divisor_of_1000_and_failed_readback_never_commits() {
        let (mut s, mut target) = mouse();
        s.interface.product_id = 0x00b7;
        target.interface.product_id = 0x00b7;
        s.channel.rate = 1;
        s.set_rate(&target, 500).unwrap();
        assert_eq!(s.channel.writes, vec![(0, 5, vec![2], 0x1f)]);
        let (mut s, target) = mouse();
        s.channel.apply = false;
        assert!(s.set_rate(&target, 4000).is_err());
        assert_eq!(s.channel.writes.len(), 1);
    }
    #[test]
    fn unknown_product_remains_read_only_even_with_valid_reports() {
        let (mut s, mut target) = mouse();
        s.interface.product_id = 0xffff;
        target.interface.product_id = 0xffff;
        assert!(s.numeric(&target, 800).is_err());
        assert!(s.set_rate(&target, 500).is_err());
        assert!(s.channel.writes.is_empty());
    }
    #[test]
    fn captured_one_based_stage_reply_has_zero_storage_without_losing_fifth_stage() {
        let raw = vec![
            0, 3, 5, 1, 1, 144, 1, 144, 0, 0, 2, 3, 32, 3, 32, 0, 0, 3, 6, 64, 6, 64, 0, 0, 4, 12,
            128, 12, 128, 0, 0, 5, 25, 0, 25, 0, 0, 0,
        ];
        let table = StageTable::decode(raw, 1).unwrap();
        let mut dpi = Dpi::default();
        table.populate(&mut dpi);
        assert_eq!(dpi.active_stage, Some(2));
        assert_eq!(dpi.stages[2].value, 1600);
        assert_eq!(dpi.stages[4].value, 6400);
        assert_eq!(table.write_bytes()[..4], [1, 3, 5, 0]);
        assert!(dpi.profile.is_none());
    }
    #[test]
    fn unsupported_neighbor_value_cannot_be_rewritten_by_a_valid_stage_edit() {
        let (mut s, target) = mouse();
        s.channel.table[4..6].copy_from_slice(&40000u16.to_be_bytes());
        let (stage, context) = edit_context(&mut s);
        assert!(s
            .change_stage(&target, stage, context, Some((3200, 800, 800)))
            .is_err());
        assert!(s.channel.writes.is_empty());
    }
    #[test]
    fn serial_is_hashed_and_placeholder_identity_does_not_authorize_settings() {
        let (mut s, target) = mouse();
        assert!(!target.key.contains("RZUNIT"));
        s.channel.serial.fill(0);
        assert!(s.identity().is_none());
        assert!(s.numeric(&target, 800).is_err());
        assert!(s.channel.writes.is_empty());
    }
}
