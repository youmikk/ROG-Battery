//! HID++ 2.0：按 ROOT 查询功能索引，功能缺失时不猜测寄存器。
//! 协议参考：Solaar 的 HID++ 功能说明与 Logitech 公开协议。
use super::*;
use crate::hid::HidDevice;

pub struct Logitech;
const BATTERY_STATUS: u16 = 0x1000;
const UNIFIED_BATTERY: u16 = 0x1004;
const BATTERY_VOLTAGE: u16 = 0x1001;
const ADJUSTABLE_DPI: u16 = 0x2201;

struct Session {
    device: HidDevice,
    address: u8,
    software: u8,
}

impl Session {
    fn open(target: &Target) -> Result<Self, String> {
        let mut session = Self {
            device: HidDevice::open(&target.interface)?,
            address: target.address,
            software: 0,
        };
        let ping = session.request(0, 1, &[0, 0, 0x5a])?;
        if ping.len() < 3 || ping[0] < 2 || ping[2] != 0x5a {
            return Err("此设备未确认支持 HID++ 2.0".into());
        }
        Ok(session)
    }

    fn request(&mut self, feature: u8, function: u8, parameters: &[u8]) -> Result<Vec<u8>, String> {
        static NEXT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
        self.software = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 15 + 1;
        let fn_sw = (function << 4) | self.software;
        let mut frame = [0u8; 20];
        if parameters.len() > 16 || function > 15 {
            return Err("HID++ 请求参数过长".into());
        }
        frame[..4].copy_from_slice(&[0x11, self.address, feature, fn_sw]);
        frame[4..4 + parameters.len()].copy_from_slice(parameters);
        let address = self.address;
        let reply = self
            .device
            .transact(&frame, 160, |reply| {
                reply_matches(reply, address, feature, fn_sw)
            })
            .ok_or("HID++ 设备无应答")?;
        if reply[2] == 0xff || reply[2] == 0x8f {
            return Err(format!(
                "HID++ 查询被拒绝：{:02x}",
                reply.get(5).copied().unwrap_or(0)
            ));
        }
        Ok(reply[4..].to_vec())
    }

    fn feature(&mut self, id: u16) -> Result<Option<u8>, String> {
        let bytes = id.to_be_bytes();
        let reply = self.request(0, 0, &bytes)?;
        Ok(reply.first().copied().filter(|index| *index != 0))
    }

    fn identity_key(&mut self) -> Result<Option<String>, String> {
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
        for (id, function) in [
            (UNIFIED_BATTERY, 1),
            (BATTERY_STATUS, 0),
            (BATTERY_VOLTAGE, 0),
        ] {
            if let Some(index) = self.feature(id)? {
                let payload = self.request(index, function, &[])?;
                return decode_battery(id, &payload)
                    .map(Some)
                    .ok_or_else(|| "HID++ 电量字段无效".into());
            }
        }
        Ok(None)
    }

    fn dpi(&mut self) -> Result<Dpi, String> {
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
            })
        }
        BATTERY_VOLTAGE => Some(Battery {
            percent: None,
            voltage_mv: Some(u16::from_be_bytes([data[0], data[1]])),
            charging: Some(data[2] & 0x80 != 0),
            approximate: true,
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
                || interface.output_len < 20
                || interface.input_len < 20
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
        let mut session = Session::open(target)?;
        let battery = session.battery()?;
        let mut target = target.clone();
        if let Some(key) = session.identity_key()? {
            target.key = key;
            target.verified_identity = true;
        }
        if detail || target.family == DeviceFamily::Unknown {
            session.identify(&mut target);
        }
        let mut dpi = if target.family == DeviceFamily::Mouse {
            session.dpi().unwrap_or_default()
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

    fn set_dpi(&self, _: &HidApi, target: &Target, dpi: u16) -> Result<(), String> {
        let mut session = Session::open(target)?;
        if target.verified_identity && session.identity_key()?.as_ref() != Some(&target.key) {
            return Err("设备标识已变化，请刷新后再试".into());
        }
        if (0xc500..=0xc5ff).contains(&target.interface.product_id) && !target.verified_identity {
            return Err("未确认接收器中设备的唯一标识，不能写入 DPI".into());
        }
        let supported = session.dpi()?;
        if !supported.writable || !supported.values.contains(&dpi) {
            return Err("DPI 不在设备报告的可用范围内".into());
        }
        let index = session
            .feature(ADJUSTABLE_DPI)?
            .ok_or("设备不支持 DPI 设置")?;
        let previous = supported.current.ok_or("无法读取当前 DPI")?;
        let value = dpi.to_be_bytes();
        // 写入应答可能丢失，仍须读回判断实际状态；失败后恢复原值。
        let _ = session.request(index, 3, &[0, value[0], value[1]]);
        let confirmed = session
            .request(index, 2, &[0])
            .ok()
            .and_then(|payload| decode_current_dpi(&payload));
        if confirmed == Some(dpi) {
            return Ok(());
        }
        let restore = previous.to_be_bytes();
        let _ = session.request(index, 3, &[0, restore[0], restore[1]]);
        let restored = session
            .request(index, 2, &[0])
            .ok()
            .and_then(|payload| decode_current_dpi(&payload));
        if restored != Some(previous) {
            return Err("DPI 写入未确认，原值恢复也未确认".into());
        }
        Err("设备未确认新的 DPI，已恢复原值".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
