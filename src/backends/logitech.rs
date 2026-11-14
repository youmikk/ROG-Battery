//! HID++ 2.0：按 ROOT 查询功能索引，功能缺失时不猜测寄存器。
//! 协议参考：Solaar 的 HID++ 功能说明与 Logitech 公开协议。
use super::*;
use crate::hid::HidDevice;

pub struct Logitech;
const BATTERY_STATUS: u16 = 0x1000;
const UNIFIED_BATTERY: u16 = 0x1004;
const BATTERY_VOLTAGE: u16 = 0x1001;
const ADJUSTABLE_DPI: u16 = 0x2201;

trait Channel {
    fn output_len(&self) -> u16;
    fn transact(
        &mut self,
        packet: &[u8],
        timeout: u32,
        matcher: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Option<Vec<u8>>;
}

impl Channel for HidDevice {
    fn output_len(&self) -> u16 {
        HidDevice::output_len(self)
    }
    fn transact(
        &mut self,
        packet: &[u8],
        timeout: u32,
        matcher: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Option<Vec<u8>> {
        HidDevice::transact(self, packet, timeout, |reply| matcher(reply))
    }
}

struct Session<C = HidDevice> {
    device: C,
    address: u8,
    software: u8,
    legacy: bool,
    last_protocol_error: Option<(u8, u8)>,
    interface: HidInterface,
    features: std::collections::BTreeMap<u8, u16>,
}

impl Session<HidDevice> {
    fn open(target: &Target) -> Result<Self, String> {
        Self::from_channel(HidDevice::open(&target.interface)?, target)
    }
}

impl<C: Channel> Session<C> {
    fn from_channel(device: C, target: &Target) -> Result<Self, String> {
        let mut session = Self {
            device,
            address: target.address,
            software: 0,
            legacy: false,
            last_protocol_error: None,
            interface: target.interface.clone(),
            features: Default::default(),
        };
        match session.request(0, 1, &[0, 0, 0x5a]) {
            Ok(ping) if ping.len() >= 3 && ping[0] >= 2 && ping[2] == 0x5a => (),
            Err(_) if session.last_protocol_error == Some((0x8f, 1)) => session.legacy = true,
            _ => return Err("此设备未确认支持 HID++".into()),
        }
        Ok(session)
    }

    fn request(&mut self, feature: u8, function: u8, parameters: &[u8]) -> Result<Vec<u8>, String> {
        let started = std::time::Instant::now();
        static NEXT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
        self.software = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 15 + 1;
        let fn_sw = (function << 4) | self.software;
        let total = if self.device.output_len() >= 20 {
            20
        } else {
            7
        };
        let mut frame = vec![0u8; total];
        if parameters.len() > total - 4 || function > 15 {
            return Err("HID++ 请求参数过长".into());
        }
        frame[..4].copy_from_slice(&[
            if total == 20 { 0x11 } else { 0x10 },
            self.address,
            feature,
            fn_sw,
        ]);
        frame[4..4 + parameters.len()].copy_from_slice(parameters);
        let address = self.address;
        self.last_protocol_error = None;
        let logical_feature = if feature == 0 && function == 0 && parameters.len() >= 2 {
            u16::from_be_bytes([parameters[0], parameters[1]])
        } else {
            self.features.get(&feature).copied().unwrap_or(0)
        };
        let reply = self.device.transact(&frame, 160, &mut |reply| {
            reply_matches(reply, address, feature, fn_sw)
        });
        let Some(reply) = reply else {
            trace::record(
                &self.interface,
                BackendId::Logitech,
                "query",
                logical_feature,
                function,
                total,
                0,
                started,
                trace::Outcome::NoReply,
            );
            return Err("HID++ 设备无应答".into());
        };
        if reply[2] == 0xff || reply[2] == 0x8f {
            self.last_protocol_error = Some((reply[2], reply.get(5).copied().unwrap_or(0)));
            trace::record(
                &self.interface,
                BackendId::Logitech,
                "query",
                logical_feature,
                function,
                total,
                reply.len(),
                started,
                trace::Outcome::Rejected(u32::from(reply.get(5).copied().unwrap_or(0))),
            );
            return Err(format!(
                "HID++ 查询被拒绝：{:02x}",
                reply.get(5).copied().unwrap_or(0)
            ));
        }
        trace::record(
            &self.interface,
            BackendId::Logitech,
            "query",
            logical_feature,
            function,
            total,
            reply.len(),
            started,
            trace::Outcome::Ok,
        );
        Ok(reply[4..].to_vec())
    }

    fn feature(&mut self, id: u16) -> Result<Option<u8>, String> {
        if let Some((&index, _)) = self.features.iter().find(|(_, feature)| **feature == id) {
            return Ok(Some(index));
        }
        let bytes = id.to_be_bytes();
        let reply = self.request(0, 0, &bytes)?;
        let index = reply.first().copied().filter(|index| *index != 0);
        if let Some(index) = index {
            self.features.insert(index, id);
        }
        Ok(index)
    }

    fn identity_key(&mut self) -> Result<Option<String>, String> {
        if self.legacy {
            return Ok(None);
        }
        let Some(index) = self.feature(0x0003)? else {
            return Ok(None);
        };
        let payload = self.request(index, 0, &[])?;
        let Some(unit) = payload
            .get(1..5)
            .filter(|unit| unit.iter().any(|byte| *byte != 0))
        else {
            return Ok(None);
        };
        let unit = unit
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Ok(Some(crate::device::stable_key(&format!(
            "logitech-unit:{unit}"
        ))))
    }

    fn battery(&mut self) -> Result<Option<Battery>, String> {
        if self.legacy {
            for register in [0x0d, 0x07] {
                let started = std::time::Instant::now();
                let request = [0x10, self.address, 0x81, register, 0, 0, 0];
                let address = self.address;
                if let Some(reply) = self.device.transact(&request, 160, &mut |reply| {
                    reply.len() >= 7
                        && matches!(reply[0], 0x10 | 0x11)
                        && reply[1] == address
                        && ((reply[2] == 0x81 && reply[3] == register)
                            || (reply[2] == 0x8f && reply[3] == 0x81 && reply[4] == register))
                }) {
                    trace::record(
                        &self.interface,
                        BackendId::Logitech,
                        "read-register",
                        register as u16,
                        0x81,
                        request.len(),
                        reply.len(),
                        started,
                        if reply[2] == 0x81 {
                            trace::Outcome::Ok
                        } else {
                            trace::Outcome::Rejected(reply[5] as u32)
                        },
                    );
                    if reply[2] == 0x81 {
                        if let Some(battery) = decode_legacy_battery(register, &reply[4..]) {
                            return Ok(Some(battery));
                        }
                    }
                } else {
                    trace::record(
                        &self.interface,
                        BackendId::Logitech,
                        "read-register",
                        register as u16,
                        0x81,
                        request.len(),
                        0,
                        started,
                        trace::Outcome::NoReply,
                    );
                }
            }
            return Ok(None);
        }
        for (id, function) in [
            (UNIFIED_BATTERY, 1),
            (BATTERY_STATUS, 0),
            (BATTERY_VOLTAGE, 0),
        ] {
            if let Ok(Some(index)) = self.feature(id) {
                if let Ok(payload) = self.request(index, function, &[]) {
                    if let Some(battery) = decode_battery(id, &payload) {
                        return Ok(Some(battery));
                    }
                }
            }
        }
        Ok(None)
    }

    fn dpi(&mut self) -> Result<Dpi, String> {
        if self.legacy {
            return Ok(Dpi::default());
        }
        let Some(index) = self.feature(ADJUSTABLE_DPI)? else {
            return Ok(Dpi::default());
        };
        let sensors = self.request(index, 0, &[])?;
        if sensors.first() != Some(&1) {
            return Ok(Dpi::default());
        }
        let list = self.request(index, 1, &[0])?;
        let values = decode_dpi_list(list.get(1..).unwrap_or_default())?;
        let payload = self.request(index, 2, &[0])?;
        let current = decode_current_dpi(&payload);
        Ok(Dpi {
            current,
            writable: current.is_some() && !values.is_empty(),
            values,
        })
    }

    fn identify(&mut self, target: &mut Target) {
        if self.legacy {
            return;
        }
        let Ok(Some(index)) = self.feature(0x0005) else {
            return;
        };
        if let Ok(kind) = self.request(index, 2, &[]) {
            target.family = match kind.first() {
                Some(0 | 2) => DeviceFamily::Keyboard,
                Some(3 | 4 | 5) => DeviceFamily::Mouse,
                _ => target.family,
            };
        }
        let Ok(length) = self.request(index, 0, &[]) else {
            return;
        };
        let Some(length) = length
            .first()
            .copied()
            .filter(|length| *length > 0 && *length <= 128)
        else {
            return;
        };
        let mut name = Vec::new();
        while name.len() < length as usize {
            let Ok(part) = self.request(index, 1, &[name.len() as u8]) else {
                return;
            };
            if part.is_empty() {
                return;
            }
            name.extend_from_slice(&part[..part.len().min(length as usize - name.len())]);
        }
        if let Ok(name) = String::from_utf8(name) {
            let name = name.trim_matches(char::from(0)).trim();
            if !name.is_empty() && !name.chars().any(char::is_control) {
                target.name = name.to_string();
            }
        }
    }

    fn read_snapshot(mut self, target: &Target, detail: bool) -> Result<Snapshot, String> {
        let battery = self.battery()?;
        let mut target = target.clone();
        if let Ok(Some(key)) = self.identity_key() {
            target.key = key;
            target.verified_identity = true;
        }
        if detail || target.family == DeviceFamily::Unknown {
            self.identify(&mut target);
        }
        let mut dpi = if target.family == DeviceFamily::Mouse {
            self.dpi().unwrap_or_default()
        } else {
            Dpi::default()
        };
        if (0xc500..=0xc5ff).contains(&target.interface.product_id) && !target.verified_identity {
            dpi.writable = false;
        }
        Ok(Snapshot {
            target,
            battery,
            dpi,
            connected: true,
        })
    }

    fn set_dpi(&mut self, target: &Target, dpi: u16) -> Result<(), String> {
        if target.verified_identity && self.identity_key()?.as_ref() != Some(&target.key) {
            return Err("设备标识已变化，请刷新后再试".into());
        }
        if (0xc500..=0xc5ff).contains(&target.interface.product_id) && !target.verified_identity {
            return Err("未确认接收器中设备的唯一标识，不能写入 DPI".into());
        }
        let supported = self.dpi()?;
        if !supported.writable || !supported.values.contains(&dpi) {
            return Err("DPI 不在设备报告的可用范围内".into());
        }
        let index = self.feature(ADJUSTABLE_DPI)?.ok_or("设备不支持 DPI 设置")?;
        let previous = supported.current.ok_or("无法读取当前 DPI")?;
        if previous == dpi {
            return Ok(());
        }
        let value = dpi.to_be_bytes();
        let _ = self.request(index, 3, &[0, value[0], value[1]]);
        if self
            .request(index, 2, &[0])
            .ok()
            .and_then(|payload| decode_current_dpi(&payload))
            == Some(dpi)
        {
            return Ok(());
        }
        let restore = previous.to_be_bytes();
        let _ = self.request(index, 3, &[0, restore[0], restore[1]]);
        if self
            .request(index, 2, &[0])
            .ok()
            .and_then(|payload| decode_current_dpi(&payload))
            != Some(previous)
        {
            return Err("DPI 写入未确认，原值恢复也未确认".into());
        }
        Err("设备未确认新的 DPI，已恢复原值".into())
    }
}

fn reply_matches(reply: &[u8], address: u8, feature: u8, function: u8) -> bool {
    if reply.len() < 7 || !matches!(reply[0], 0x10 | 0x11) || reply[1] != address {
        return false;
    }
    (reply[2] == feature && reply[3] == function)
        || (matches!(reply[2], 0xff | 0x8f) && reply[3] == feature && reply[4] == function)
}

fn decode_battery(feature: u16, data: &[u8]) -> Option<Battery> {
    if data.len() < 3 {
        return None;
    }
    let status = data[2];
    let charging = match status {
        0 => Some(false),
        1 | 2 | 4 => Some(true),
        3 => Some(false),
        _ => None,
    };
    match feature {
        BATTERY_STATUS | UNIFIED_BATTERY => {
            if data[0] > 100 {
                return None;
            }
            // HID++ 的 0 通常表示无法给出精确百分比，不能当作电池已耗尽。
            let percent = (data[0] > 0)
                .then_some(data[0])
                .or_else(|| (status == 3).then_some(100));
            Some(Battery {
                percent,
                charging,
                voltage_mv: None,
                approximate: percent.is_none(),
                level: if feature == UNIFIED_BATTERY {
                    match data[1] {
                        1 => Some(BatteryLevel::Critical),
                        2 => Some(BatteryLevel::Low),
                        4 => Some(BatteryLevel::Good),
                        8 => Some(BatteryLevel::Full),
                        _ => None,
                    }
                } else {
                    None
                },
            })
        }
        BATTERY_VOLTAGE => Some(Battery {
            percent: None,
            voltage_mv: Some(u16::from_be_bytes([data[0], data[1]])),
            charging: Some(data[2] & 0x80 != 0),
            approximate: true,
            level: (data[2] & 0x20 != 0).then_some(BatteryLevel::Critical),
        }),
        _ => None,
    }
}

fn decode_current_dpi(data: &[u8]) -> Option<u16> {
    if data.len() < 5 || data[0] != 0 {
        return None;
    }
    let current = u16::from_be_bytes([data[1], data[2]]);
    let fallback = u16::from_be_bytes([data[3], data[4]]);
    Some(if current == 0 { fallback } else { current }).filter(|value| *value > 0)
}

fn decode_legacy_battery(register: u8, data: &[u8]) -> Option<Battery> {
    if data.len() < 3 {
        return None;
    }
    match register {
        0x0d if data[0] <= 100 => Some(Battery {
            percent: Some(data[0]),
            level: None,
            voltage_mv: None,
            approximate: false,
            charging: match data[2] & 0xf0 {
                0x50 => Some(true),
                0x30 | 0x90 => Some(false),
                _ => None,
            },
        }),
        0x07 => Some(Battery {
            percent: None,
            voltage_mv: None,
            approximate: true,
            level: match data[0] {
                1 => Some(BatteryLevel::Critical),
                3 => Some(BatteryLevel::Low),
                5 => Some(BatteryLevel::Good),
                7 => Some(BatteryLevel::Full),
                _ => None,
            },
            charging: if data[1] & 0x21 == 0x21 {
                Some(true)
            } else if data[1] == 0 || data[1] & 0x22 == 0x22 {
                Some(false)
            } else {
                None
            },
        }),
        _ => None,
    }
}

fn decode_dpi_list(data: &[u8]) -> Result<Vec<u16>, String> {
    let mut words = data
        .chunks_exact(2)
        .map(|word| u16::from_be_bytes([word[0], word[1]]));
    let mut values: Vec<u16> = Vec::new();
    let mut terminated = false;
    while let Some(word) = words.next() {
        if word == 0 {
            terminated = true;
            break;
        }
        if word & 0xe000 == 0xe000 {
            let step = word & 0x1fff;
            let previous = *values.last().ok_or("DPI 范围缺少起始值")?;
            let end = words.next().ok_or("DPI 范围缺少终止值")?;
            if step == 0 || end <= previous || (end - previous) % step != 0 {
                return Err("DPI 范围无效".into());
            }
            for value in
                (u32::from(previous) + u32::from(step)..=u32::from(end)).step_by(step as usize)
            {
                values.push(value as u16);
            }
        } else {
            values.push(word);
        }
        if values.len() > 2048 {
            return Err("DPI 范围过长".into());
        }
    }
    if !terminated {
        return Err("DPI 列表不完整".into());
    }
    values.sort_unstable();
    values.dedup();
    Ok(values)
}

impl DeviceBackend for Logitech {
    fn discover(&self, _: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        let mut targets = Vec::new();
        for interface in interfaces {
            if interface.vendor_id != 0x046d
                || interface.usage_page < 0xff00
                || interface.output_len < 7
                || interface.input_len < 7
            {
                continue;
            }
            let addresses: &[u8] = if (0xc500..=0xc5ff).contains(&interface.product_id) {
                &[1, 2, 3, 4, 5, 6]
            } else {
                &[0xff]
            };
            for address in addresses {
                targets.push(Target::new(BackendId::Logitech, interface, *address));
            }
        }
        Ok(targets)
    }

    fn read(&self, _: &HidApi, target: &Target, detail: bool) -> Result<Snapshot, String> {
        Session::open(target)?.read_snapshot(target, detail)
    }

    fn set_dpi(&self, _: &HidApi, target: &Target, dpi: u16) -> Result<(), String> {
        Session::open(target)?.set_dpi(target, dpi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    enum Reply {
        Data(Vec<u8>),
        Error(u8, u8),
    }
    struct Step {
        feature: u8,
        function: u8,
        parameters: Vec<u8>,
        reply: Reply,
    }
    struct Transcript(Arc<Mutex<VecDeque<Step>>>);

    impl Channel for Transcript {
        fn output_len(&self) -> u16 {
            20
        }
        fn transact(
            &mut self,
            packet: &[u8],
            _: u32,
            matcher: &mut dyn FnMut(&[u8]) -> bool,
        ) -> Option<Vec<u8>> {
            let step = self
                .0
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected protocol request");
            assert_eq!(packet[2], step.feature);
            let function = if packet[2] == 0x81 {
                packet[3]
            } else {
                packet[3] >> 4
            };
            assert_eq!(function, step.function);
            assert_eq!(&packet[4..4 + step.parameters.len()], step.parameters);
            assert!(packet[4 + step.parameters.len()..]
                .iter()
                .all(|byte| *byte == 0));
            let mut response = vec![0; 20];
            response[..4].copy_from_slice(&[0x11, packet[1], packet[2], packet[3]]);
            match step.reply {
                Reply::Data(data) => response[4..4 + data.len()].copy_from_slice(&data),
                Reply::Error(kind, code) => {
                    response[2..6].copy_from_slice(&[kind, packet[2], packet[3], code]);
                }
            }
            assert!(
                matcher(&response),
                "protocol response did not match its request"
            );
            Some(response)
        }
    }

    fn data(feature: u8, function: u8, parameters: &[u8], payload: &[u8]) -> Step {
        Step {
            feature,
            function,
            parameters: parameters.to_vec(),
            reply: Reply::Data(payload.to_vec()),
        }
    }
    fn ping() -> Step {
        data(0, 1, &[0, 0, 0x5a], &[2, 0, 0x5a])
    }
    fn target() -> Target {
        Target::new(
            BackendId::Logitech,
            &HidInterface {
                path: "test-channel".into(),
                serial: String::new(),
                product: "Test mouse".into(),
                vendor_id: 0x046d,
                product_id: 0xc08b,
                usage_page: 0xff00,
                usage: 1,
                input_len: 20,
                output_len: 20,
                feature_len: 0,
            },
            0xff,
        )
    }
    fn channel(steps: Vec<Step>) -> (Transcript, Arc<Mutex<VecDeque<Step>>>) {
        let steps = Arc::new(Mutex::new(steps.into()));
        (Transcript(Arc::clone(&steps)), steps)
    }
    fn dpi_queries() -> Vec<Step> {
        vec![
            ping(),
            data(0, 0, &[0x22, 1], &[2]),
            data(2, 0, &[], &[1]),
            data(2, 1, &[0], &[0, 1, 0x90, 3, 0x20, 6, 0x40, 0, 0]),
            data(2, 2, &[0], &[0, 3, 0x20, 3, 0x20]),
        ]
    }

    #[test]
    fn legacy_negotiation_reads_battery_without_feature_queries_or_dpi_writes() {
        let (channel, steps) = channel(vec![
            Step {
                feature: 0,
                function: 1,
                parameters: vec![0, 0, 0x5a],
                reply: Reply::Error(0x8f, 1),
            },
            data(0x81, 0x0d, &[], &[42, 0, 0x30]),
        ]);
        let snapshot = Session::from_channel(channel, &target())
            .unwrap()
            .read_snapshot(&target(), false)
            .unwrap();
        assert_eq!(snapshot.battery.unwrap().percent, Some(42));
        assert_eq!(snapshot.dpi, Dpi::default());
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn battery_survives_failed_device_identity_query() {
        let (channel, steps) = channel(vec![
            ping(),
            data(0, 0, &[0x10, 4], &[1]),
            data(1, 1, &[], &[65, 4, 0]),
            Step {
                feature: 0,
                function: 0,
                parameters: vec![0, 3],
                reply: Reply::Error(0xff, 2),
            },
            data(0, 0, &[0x22, 1], &[0]),
        ]);
        let snapshot = Session::from_channel(channel, &target())
            .unwrap()
            .read_snapshot(&target(), false)
            .unwrap();
        assert_eq!(snapshot.battery.unwrap().percent, Some(65));
        assert!(snapshot.connected);
        assert!(!snapshot.target.verified_identity);
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn unadvertised_dpi_is_rejected_without_a_write() {
        let (channel, steps) = channel(dpi_queries());
        let mut session = Session::from_channel(channel, &target()).unwrap();
        assert!(session.set_dpi(&target(), 1000).is_err());
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn unchanged_dpi_does_not_write() {
        let (channel, steps) = channel(dpi_queries());
        let mut session = Session::from_channel(channel, &target()).unwrap();
        session.set_dpi(&target(), 800).unwrap();
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn failed_dpi_readback_restores_and_verifies_original_value() {
        let mut script = dpi_queries();
        script.extend([
            data(2, 3, &[0, 6, 0x40], &[]),
            data(2, 2, &[0], &[0, 3, 0x20, 3, 0x20]),
            data(2, 3, &[0, 3, 0x20], &[]),
            data(2, 2, &[0], &[0, 3, 0x20, 3, 0x20]),
        ]);
        let (channel, steps) = channel(script);
        let mut session = Session::from_channel(channel, &target()).unwrap();
        assert_eq!(
            session.set_dpi(&target(), 1600).unwrap_err(),
            "设备未确认新的 DPI，已恢复原值"
        );
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn hidpp_notifications_and_other_device_replies_are_ignored() {
        assert!(reply_matches(&[0x11, 2, 7, 0x21, 0, 0, 0], 2, 7, 0x21));
        assert!(!reply_matches(&[0x11, 3, 7, 0x21, 0, 0, 0], 2, 7, 0x21));
        assert!(!reply_matches(&[0x11, 2, 7, 0x20, 0, 0, 0], 2, 7, 0x21));
        assert!(reply_matches(&[0x11, 2, 0xff, 7, 0x21, 2, 0], 2, 7, 0x21));
    }
    #[test]
    fn coarse_battery_and_voltage_are_not_invented_percentages() {
        assert_eq!(
            decode_battery(UNIFIED_BATTERY, &[78, 4, 1])
                .unwrap()
                .percent,
            Some(78)
        );
        assert_eq!(
            decode_battery(BATTERY_STATUS, &[0, 20, 0]).unwrap().percent,
            None
        );
        assert_eq!(
            decode_battery(BATTERY_VOLTAGE, &[0x10, 0, 0])
                .unwrap()
                .percent,
            None
        );
        assert_eq!(decode_battery(UNIFIED_BATTERY, &[255, 0, 0]), None);
    }
    #[test]
    fn dpi_ranges_are_validated_before_any_device_write() {
        assert_eq!(
            decode_dpi_list(&[0x01, 0x90, 0xe0, 0x64, 0x03, 0x20, 0, 0]).unwrap(),
            vec![400, 500, 600, 700, 800]
        );
        assert!(decode_dpi_list(&[0xe0, 0, 0, 0]).is_err());
        assert!(decode_dpi_list(&[0x01, 0x90]).is_err());
        assert_eq!(decode_current_dpi(&[0, 0, 0, 3, 0x20]), Some(800));
    }
}
