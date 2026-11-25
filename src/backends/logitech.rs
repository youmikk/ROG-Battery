//! HID++ 1.0 / 2.0：功能与寄存器按协议查询，缺失时不猜测写入方式。
//! 协议参考：Solaar 的 HID++ 功能说明与 Logitech 公开协议。
use super::*;
use crate::hid::HidDevice;

pub struct Logitech;
const BATTERY_STATUS: u16 = 0x1000;
const UNIFIED_BATTERY: u16 = 0x1004;
const BATTERY_VOLTAGE: u16 = 0x1001;
const ADJUSTABLE_DPI: u16 = 0x2201;
const EXTENDED_DPI: u16 = 0x2202;
const ONBOARD_PROFILES: u16 = 0x8100;
const REPORT_RATE: u16 = 0x8060;
const EXTENDED_REPORT_RATE: u16 = 0x8061;
const REQUEST_TIMEOUT_MS: u32 = 600;
const PING_TIMEOUT_MS: u32 = 1500;

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

struct RateInfo {
    rate: crate::device::PollingRate,
    feature: u8,
    connection: Option<u8>,
    raw: u8,
    choices: Vec<(u8,u16)>,
    mode_feature: Option<u8>,
}

impl Session<HidDevice> {
    fn open(target: &Target) -> Result<Self, String> {
        Self::from_channel(HidDevice::open(target.hid()?)?, target)
    }
}

impl<C: Channel> Session<C> {
    fn rate_connection(&mut self) -> Result<Option<u8>,String> {
        if self.address != 0xff && (0xc500..=0xc5ff).contains(&self.interface.product_id) {
            return Ok(Some(1));
        }
        let Some(feature) = self.feature(0x0003)? else { return Ok(None); };
        let info = self.request(feature,0,&[])?;
        if info.len() < 13 { return Ok(None); }
        let mut offset = 7;
        for flag in [1,2,4,8] {
            if info[6] & flag == 0 { continue; }
            let Some(bytes) = info.get(offset..offset+2) else { return Ok(None); };
            offset += 2;
            if u16::from_be_bytes([bytes[0],bytes[1]]) == self.interface.product_id {
                return Ok(match flag { 8 => Some(0), 4 => Some(1), _ => None });
            }
        }
        Ok(None)
    }
    fn rate_info(&mut self, target: &Target) -> Result<Option<RateInfo>,String> {
        if self.legacy { return Ok(None); }
        let extended = self.feature(EXTENDED_REPORT_RATE)?;
        let (feature,connection,choices,raw) = if let Some(feature) = extended {
            let Some(connection) = self.rate_connection()? else { return Ok(None); };
            let list = self.request(feature,1,&[connection])?;
            if list.len() < 2 { return Err("设备返回的回报率列表无效".into()); }
            let mask = u16::from_be_bytes([list[0],list[1]]);
            let choices = [125,250,500,1000,2000,4000,8000].into_iter().enumerate()
                .filter_map(|(raw,hz)| (mask & (1 << raw) != 0).then_some((raw as u8,hz))).collect::<Vec<_>>();
            let current = self.request(feature,2,&[connection])?;
            let raw = *current.first().ok_or("无法读取当前回报率")?;
            (feature,Some(connection),choices,raw)
        } else if let Some(feature) = self.feature(REPORT_RATE)? {
            let list = self.request(feature,0,&[])?;
            let mask = *list.first().ok_or("设备返回的回报率列表无效")?;
            let choices = (1..=8u8).filter_map(|ms| (mask & (1 << (ms-1)) != 0)
                .then_some((ms,(1000 + u16::from(ms)/2) / u16::from(ms)))).collect::<Vec<_>>();
            let current = self.request(feature,1,&[])?;
            let raw = current.first().copied().filter(|ms| (1..=8).contains(ms)).ok_or("无法读取当前回报率")?;
            (feature,None,choices,raw)
        } else { return Ok(None); };
        let current = if connection.is_some() {
            [125,250,500,1000,2000,4000,8000].get(usize::from(raw)).copied()
        } else { Some((1000 + u16::from(raw)/2) / u16::from(raw)) };
        let (mode_feature,host_control) = match self.feature(ONBOARD_PROFILES) {
            Ok(Some(feature)) => (Some(feature),self.request(feature,2,&[]).ok()
                .is_some_and(|reply|reply.first() == Some(&2))),
            Ok(None) => (None,true),
            Err(_) => (None,false),
        };
        let identity = !((0xc500..=0xc5ff).contains(&target.hid()?.product_id) && !target.verified_identity);
        let rate = crate::device::PollingRate { current,values:choices.iter().map(|(_,hz)| *hz).collect(),
            writable: identity && host_control && current.is_some() && choices.iter().any(|(v,_)| *v == raw) };
        Ok(Some(RateInfo { rate,feature,connection,raw,choices,mode_feature }))
    }
    fn set_rate(&mut self,target:&Target,hz:u16) -> Result<(),String> {
        self.check_identity(target)?;
        let info = self.rate_info(target)?.ok_or("设备不支持回报率设置")?;
        if !info.rate.writable { return Err("当前模式不允许回报率写入".into()); }
        let raw = info.choices.iter().find(|(_,v)| *v == hz).ok_or("回报率不在此连接的可用范围内")?.0;
        if raw == info.raw { return Ok(()); }
        self.check_identity(target)?;
        if let Some(feature) = info.mode_feature {
            if self.request(feature,2,&[])?.first() != Some(&2) { return Err("当前模式不允许回报率写入".into()); }
        }
        if info.connection.is_some() && self.rate_connection()? != info.connection {
            return Err("设备连接已变化，请刷新后重试".into());
        }
        // Extended get-list/get-rate select a link; set-rate takes only the rate code.
        let (function,parameters) = (if info.connection.is_some() {3} else {2},vec![raw]);
        self.request(info.feature,function,&parameters)?;
        self.check_identity(target)?;
        if let Some(feature) = info.mode_feature {
            if self.request(feature,2,&[])?.first() != Some(&2) { return Err("当前模式不允许回报率写入".into()); }
        }
        if info.connection.is_some() && self.rate_connection()? != info.connection {
            return Err("设备连接已变化，请刷新后重试".into());
        }
        let parameters = info.connection.map(|v|vec![v]).unwrap_or_default();
        let current = self.request(info.feature,if info.connection.is_some() {2} else {1},&parameters)?;
        if current.first() != Some(&raw) { return Err("无法确认设备已保存设置，请刷新后检查".into()); }
        Ok(())
    }
    fn from_channel(device: C, target: &Target) -> Result<Self, String> {
        let mut session = Self {
            device,
            address: target.address,
            software: 0,
            legacy: false,
            last_protocol_error: None,
            interface: target.hid()?.clone(),
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
        let timeout = if feature == 0 && function == 1 { PING_TIMEOUT_MS } else { REQUEST_TIMEOUT_MS };
        let reply = self.device.transact(&frame, timeout, &mut |reply| {
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
        let Some(payload) = hidpp_payload(&reply) else {
            trace::record(
                &self.interface,
                BackendId::Logitech,
                "query",
                logical_feature,
                function,
                total,
                reply.len(),
                started,
                trace::Outcome::InvalidData,
            );
            return Err("HID++ 回帧长度无效".into());
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
            return Err(crate::trf!(
                "HID++ 查询被拒绝：{code:02x}",
                code = reply.get(5).copied().unwrap_or(0)
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
        Ok(payload.to_vec())
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
                if let Ok(payload) = self.read_register(self.address, u16::from(register), &[]) {
                    if let Some(battery) = decode_legacy_battery(register, &payload) {
                        return Ok(Some(battery));
                    }
                }
            }
            return Ok(None);
        }
        let mut partial: Option<Battery> = None;
        for (id, function) in [
            (UNIFIED_BATTERY, 1),
            (BATTERY_STATUS, 0),
            (BATTERY_VOLTAGE, 0),
        ] {
            if let Ok(Some(index)) = self.feature(id) {
                if let Ok(payload) = self.request(index, function, &[]) {
                    if let Some(battery) = decode_battery(id, &payload) {
                        if battery.percent.is_some() {
                            return Ok(Some(battery));
                        }
                        if let Some(known) = &mut partial {
                            known.charging = known.charging.or(battery.charging);
                            known.voltage_mv = known.voltage_mv.or(battery.voltage_mv);
                            known.level = known.level.or(battery.level);
                            known.estimated_percent = known.estimated_percent.or(battery.estimated_percent);
                        } else {
                            partial = Some(battery);
                        }
                    }
                }
            }
        }
        Ok(partial)
    }

    fn dpi(&mut self) -> Result<Dpi, String> {
        if self.legacy {
            return Ok(Dpi::default());
        }
        let Some(index) = self.feature(ADJUSTABLE_DPI)? else {
            return self.extended_dpi();
        };
        let sensors = self.request(index, 0, &[])?;
        if sensors.first() != Some(&1) {
            return Ok(Dpi::default());
        }
        let values = self.dpi_list(index)?;
        let payload = self.request(index, 2, &[0])?;
        let current = decode_current_dpi(&payload);
        Ok(Dpi {
            current,
            writable: current.is_some() && !values.is_empty(),
            write_effect: Some(crate::device::DpiWriteEffect::Current),
            values,
            ..Default::default()
        })
    }

    fn extended_dpi(&mut self) -> Result<Dpi, String> {
        let Some(index) = self.feature(EXTENDED_DPI)? else { return Ok(Dpi::default()); };
        let count = self.request(index, 0, &[])?;
        if count.first() != Some(&1) { return Ok(Dpi::default()); }
        let info = self.request(index, 1, &[0])?;
        if info.len() < 3 || info[0] != 0 { return Err("设备返回的 DPI 列表无效".into()); }
        let separate_axes = info[2] & 1 != 0;
        let values = (|| {
            let mut values = self.extended_dpi_list(index, 0)?;
            if separate_axes {
                let y_values = self.extended_dpi_list(index, 1)?;
                values.retain(|value| y_values.binary_search(value).is_ok());
            }
            Ok::<_, String>(values)
        })().unwrap_or_default();
        let payload = self.request(index, 5, &[0])?;
        let (x, y) = decode_extended_dpi(&payload, separate_axes).ok_or("无法读取当前 DPI")?;
        let writable = values.contains(&x) && y.is_none_or(|y| y == x);
        Ok(Dpi { current: Some(x), current_y: y, values,
            write_effect: Some(crate::device::DpiWriteEffect::Current),
            // 当前界面提供单值 DPI；不同轴值保持只读，防止无意覆盖 Y 轴。
            writable, ..Default::default() })
    }

    fn extended_dpi_list(&mut self, index: u8, axis: u8) -> Result<Vec<u16>, String> {
        let mut bytes = Vec::new();
        for page in 0..=u8::MAX {
            let payload = self.request(index, 2, &[0, axis, page])?;
            if payload.len() < 5 || payload[..3] != [0, axis, page] {
                return Err("设备返回的 DPI 列表无效".into());
            }
            // 长回报有 13 字节数据区，末尾奇数字节不构成一个 DPI 值。
            let length = (payload.len() - 3) & !1;
            bytes.extend_from_slice(&payload[3..3 + length]);
            match decode_dpi_list(&bytes) {
                Ok(values) => return Ok(values),
                Err(DpiListError::Incomplete) => (),
                Err(error) => return Err(error.message().into()),
            }
        }
        Err(DpiListError::Incomplete.message().into())
    }

    fn onboard_dpi(&mut self) -> Result<Option<Dpi>, String> {
        if self.legacy {
            return Ok(None);
        }
        let Some(feature) = self.feature(ONBOARD_PROFILES)? else {
            return Ok(None);
        };
        let mode = self.request(feature, 2, &[])?;
        match mode.first() {
            Some(2) => return Ok(None),
            Some(1) => (),
            _ => return Err("板载 DPI 配置无效".into()),
        }
        let descriptor = self.request(feature, 0, &[])?;
        if descriptor.len() < 10
            || descriptor[0] != 1
            || !(1..=5).contains(&descriptor[1])
            || !(1..=16).contains(&descriptor[3])
            || descriptor[4] > 16
        {
            return Err("此板载配置格式尚未适配".into());
        }
        // 已知 1–5 格式的 DPI 区域及 CRC 使用相同的 256 字节布局。
        let size = usize::from(u16::from_be_bytes([descriptor[7], descriptor[8]]));
        if size != 256 {
            return Err("此板载配置格式尚未适配".into());
        }
        let profile = self.onboard_profile(feature)?;
        let slot = profile as u8;
        let count = if profile >> 8 == 1 {
            descriptor[4]
        } else {
            descriptor[3]
        };
        if profile >> 8 > 1 || slot == 0 || slot > count {
            return Err("板载 DPI 配置无效".into());
        }
        let mut bytes = Vec::with_capacity(size);
        for offset in (0..size).step_by(16) {
            let address = (offset as u16).to_be_bytes();
            let sector = profile.to_be_bytes();
            let chunk =
                self.request(feature, 5, &[sector[0], sector[1], address[0], address[1]])?;
            if chunk.len() != 16 {
                return Err("板载 DPI 配置无效".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        if self.onboard_profile(feature)? != profile {
            return Err("板载配置已变化，请刷新后重试".into());
        }
        let active = self
            .request(feature, 11, &[])?
            .first()
            .copied()
            .ok_or("板载 DPI 配置无效")?;
        if self.request(feature, 2, &[])?.first() != Some(&1) {
            return Err("板载配置已变化，请刷新后重试".into());
        }
        let stages = decode_onboard_stages(&bytes, active)?;
        let current = stages
            .iter()
            .find(|stage| stage.index == active)
            .map(|stage| stage.value);
        Ok(Some(Dpi {
            current,
            stages,
            active_stage: Some(active),
            profile: Some(profile),
            stage_writable: current.is_some(),
            ..Default::default()
        }))
    }

    fn onboard_profile(&mut self, feature: u8) -> Result<u16, String> {
        let response = self.request(feature, 4, &[])?;
        let bytes: [u8; 2] = response
            .get(..2)
            .ok_or("板载 DPI 配置无效")?
            .try_into()
            .unwrap();
        Ok(u16::from_be_bytes(bytes))
    }

    fn dpi_with_stages(&mut self) -> Dpi {
        match self.onboard_dpi() {
            Ok(Some(dpi)) => dpi,
            Ok(None) => self.dpi().unwrap_or_default(),
            Err(_) => {
                let mut dpi = self.dpi().unwrap_or_default();
                // 板载模式/配置查询失败不能授权数值写入。
                dpi.writable = false;
                dpi
            }
        }
    }

    fn check_identity(&mut self, target: &Target) -> Result<(), String> {
        if target.verified_identity && self.identity_key()?.as_ref() != Some(&target.key) {
            return Err("设备标识已变化，请刷新后再试".into());
        }
        if (0xc500..=0xc5ff).contains(&target.hid()?.product_id) && !target.verified_identity {
            return Err("未确认接收器中设备的唯一标识，不能写入 DPI".into());
        }
        Ok(())
    }

    fn set_stage(
        &mut self,
        target: &Target,
        stage: crate::device::DpiStage,
        profile: u16,
    ) -> Result<(), String> {
        self.check_identity(target)?;
        let dpi = self.onboard_dpi()?.ok_or("设备不支持 DPI 档位切换")?;
        if !dpi.stage_writable || dpi.profile != Some(profile) || !dpi.stages.contains(&stage) {
            return Err("板载配置已变化，请刷新后重试".into());
        }
        let previous = dpi.active_stage.ok_or("板载 DPI 配置无效")?;
        if previous == stage.index {
            return Ok(());
        }
        let feature = self
            .feature(ONBOARD_PROFILES)?
            .ok_or("设备不支持 DPI 档位切换")?;
        let _ = self.request(feature, 12, &[stage.index]);
        // 不切换 G HUB/板载模式，不写入 Flash，也不切换整套配置。
        if self.request(feature, 2, &[])?.first() != Some(&1)
            || self.onboard_profile(feature)? != profile
        {
            return Err("板载配置已变化，请刷新后重试".into());
        }
        if self
            .request(feature, 11, &[])
            .ok()
            .as_ref()
            .and_then(|reply| reply.first())
            == Some(&stage.index)
        {
            return Ok(());
        }
        let _ = self.request(feature, 12, &[previous]);
        if self
            .request(feature, 2, &[])
            .ok()
            .as_ref()
            .and_then(|reply| reply.first())
            != Some(&1)
            || self.onboard_profile(feature).ok() != Some(profile)
            || self
                .request(feature, 11, &[])
                .ok()
                .as_ref()
                .and_then(|reply| reply.first())
                != Some(&previous)
        {
            return Err("DPI 写入未确认，原值恢复也未确认".into());
        }
        Err("设备未确认新的 DPI，已恢复原值".into())
    }

    fn dpi_list(&mut self, index: u8) -> Result<Vec<u16>, String> {
        let mut bytes = Vec::new();
        for page in 0..=u8::MAX {
            let payload = self.request(index, 1, &[0, 0, page])?;
            if payload.first() != Some(&0) || payload.len() < 3 {
                return Err("设备返回的 DPI 列表无效".into());
            }
            bytes.extend_from_slice(&payload[1..]);
            match decode_dpi_list(&bytes) {
                Ok(values) => return Ok(values),
                Err(DpiListError::Incomplete) => (),
                Err(error) => return Err(error.message().into()),
            }
        }
        Err(DpiListError::Incomplete.message().into())
    }

    fn identify(&mut self, target: &mut Target) {
        if self.legacy {
            self.identify_legacy(target);
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

    fn read_register(
        &mut self,
        address: u8,
        register: u16,
        parameters: &[u8],
    ) -> Result<Vec<u8>, String> {
        if parameters.len() > 3 {
            return Err("HID++ 请求参数过长".into());
        }
        let sub_id = if register & 0x200 != 0 { 0x83 } else { 0x81 };
        let low_register = register as u8;
        let mut request = [0x10, address, sub_id, low_register, 0, 0, 0];
        request[4..4 + parameters.len()].copy_from_slice(parameters);
        let echo = if sub_id == 0x83 {
            parameters.first().copied()
        } else {
            None
        };
        let started = std::time::Instant::now();
        let reply = self.device.transact(&request, REQUEST_TIMEOUT_MS, &mut |reply| {
            register_reply_matches(reply, address, sub_id, low_register, echo)
        });
        let outcome = match reply.as_ref() {
            Some(reply) if hidpp_payload(reply).is_none() => trace::Outcome::InvalidData,
            Some(reply) if reply[2] == sub_id => trace::Outcome::Ok,
            Some(reply) => trace::Outcome::Rejected(u32::from(reply[5])),
            None => trace::Outcome::NoReply,
        };
        trace::record(
            &self.interface,
            BackendId::Logitech,
            "read-register",
            register,
            sub_id,
            request.len(),
            reply.as_ref().map_or(0, Vec::len),
            started,
            outcome,
        );
        match reply {
            Some(reply) if hidpp_payload(&reply).is_none() => Err("HID++ 回帧长度无效".into()),
            Some(reply) if reply[2] == sub_id => Ok(hidpp_payload(&reply).unwrap().to_vec()),
            Some(reply) => Err(crate::trf!("HID++ 查询被拒绝：{code:02x}", code = reply[5])),
            None => Err("HID++ 设备无应答".into()),
        }
    }

    fn identify_legacy(&mut self, target: &mut Target) {
        if !(1..=6).contains(&self.address)
            || !(0xc500..=0xc5ff).contains(&self.interface.product_id)
        {
            return;
        }
        let slot = self.address - 1;
        // 接收器信息由 0xFF 地址返回，子寄存器的回显用于排除其他槽位的迟到回帧。
        let pairing = self.read_register(0xff, 0x2b5, &[0x20 + slot]).ok();
        if let Some(kind) = pairing.as_ref().and_then(|reply| reply.get(7)) {
            target.family = match kind & 0x0f {
                1 | 3 => DeviceFamily::Keyboard,
                2 | 8 | 9 => DeviceFamily::Mouse,
                0x0b | 0x0c => DeviceFamily::Gamepad,
                _ => target.family,
            };
        }
        if let Ok(extended) = self.read_register(0xff, 0x2b5, &[0x30 + slot]) {
            if let Some(key) = pairing
                .as_ref()
                .and_then(|pairing| legacy_identity_key(pairing, &extended))
            {
                target.key = key;
                target.verified_identity = true;
            }
        }
        if let Ok(reply) = self.read_register(0xff, 0x2b5, &[0x40 + slot]) {
            if let Some(name) = decode_receiver_name(&reply) {
                target.name = name;
            }
        }
    }

    fn read_snapshot(&mut self, target: &Target, detail: bool) -> Result<Snapshot, String> {
        let battery = self.battery()?;
        let battery_at = battery.as_ref().map(|_| std::time::Instant::now());
        let mut target = target.clone();
        if let Ok(Some(key)) = self.identity_key() {
            target.key = key;
            target.verified_identity = true;
        }
        if detail || target.family == DeviceFamily::Unknown {
            self.identify(&mut target);
        }
        let mut dpi = if target.family == DeviceFamily::Mouse {
            self.dpi_with_stages()
        } else {
            Dpi::default()
        };
        if (0xc500..=0xc5ff).contains(&target.hid()?.product_id) && !target.verified_identity {
            dpi.writable = false;
            dpi.stage_writable = false;
        }
        Ok(Snapshot {
            target,
            battery,
            battery_unavailable: false, battery_at, polling_rate: Default::default(),
            dpi,
            connected: true,
        })
    }

    fn set_dpi(&mut self, target: &Target, dpi: u16) -> Result<(), String> {
        self.check_identity(target)?;
        let supported = self.dpi()?;
        if !supported.writable || !supported.values.contains(&dpi) {
            return Err("DPI 不在设备报告的可用范围内".into());
        }
        let legacy_index = self.feature(ADJUSTABLE_DPI)?;
        let previous = supported.current.ok_or("无法读取当前 DPI")?;
        if previous == dpi {
            return Ok(());
        }
        // 界面读完后模式可能被 G HUB 改变，写入前重新检查。
        if let Some(feature) = self.feature(ONBOARD_PROFILES)? {
            if self.request(feature, 2, &[])?.first() != Some(&2) {
                return Err("板载模式请使用 DPI 档位切换".into());
            }
        }
        if legacy_index.is_none() { return self.set_extended_dpi(dpi, previous); }
        let index = legacy_index.unwrap();
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

    fn set_extended_dpi(&mut self, dpi: u16, previous: u16) -> Result<(), String> {
        let index = self.feature(EXTENDED_DPI)?.ok_or("设备不支持 DPI 设置")?;
        let info = self.request(index, 1, &[0])?;
        if info.len() < 3 || info[0] != 0 { return Err("无法读取当前 DPI".into()); }
        let separate_axes = info[2] & 1 != 0;
        let original = self.request(index, 5, &[0])?;
        let (x, y) = decode_extended_dpi(&original, separate_axes).ok_or("无法读取当前 DPI")?;
        if x != previous || y.is_some_and(|y| y != x) { return Err("无法读取当前 DPI".into()); }
        let lod = if info[2] & 2 != 0 { original[9] } else { 0 };
        let parameters = |value: u16| {
            let [hi, lo] = value.to_be_bytes();
            [0, hi, lo, if separate_axes { hi } else { 0 },
                if separate_axes { lo } else { 0 }, lod]
        };
        let confirmed = |reply: &[u8], expected: u16| decode_extended_dpi(reply, separate_axes)
            .is_some_and(|(x, y)| x == expected && y.is_none_or(|y| y == expected)
                && (info[2] & 2 == 0 || reply[9] == lod));
        let _ = self.request(index, 6, &parameters(dpi));
        if self.request(index, 5, &[0]).ok().is_some_and(|reply| confirmed(&reply, dpi)) { return Ok(()); }
        let _ = self.request(index, 6, &parameters(previous));
        if !self.request(index, 5, &[0]).ok().is_some_and(|reply| confirmed(&reply, previous)) {
            return Err("DPI 写入未确认，原值恢复也未确认".into());
        }
        Err("设备未确认新的 DPI，已恢复原值".into())
    }
}

fn crc16(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0xffff, |mut crc, byte| {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
        crc
    })
}

fn decode_onboard_stages(bytes: &[u8], active: u8) -> Result<Vec<crate::device::DpiStage>, String> {
    if bytes.len() != 256 || crc16(&bytes[..254]) != u16::from_be_bytes([bytes[254], bytes[255]]) {
        return Err("板载 DPI 配置校验失败".into());
    }
    let mut stages = Vec::new();
    for index in 0..5u8 {
        let offset = 3 + usize::from(index) * 2;
        let value = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
        if value == 0 || value == u16::MAX {
            continue;
        }
        if value < 50 {
            return Err("板载 DPI 配置无效".into());
        }
        stages.push(crate::device::DpiStage { index, value });
    }
    if !stages.iter().any(|stage| stage.index == active)
        || !stages.iter().any(|stage| stage.index == bytes[1])
    {
        return Err("板载 DPI 配置无效".into());
    }
    Ok(stages)
}

fn reply_matches(reply: &[u8], address: u8, feature: u8, function: u8) -> bool {
    if reply.len() < 7 || !matches!(reply[0], 0x10 | 0x11) || reply[1] != address {
        return false;
    }
    (reply[2] == feature && reply[3] == function)
        || (matches!(reply[2], 0xff | 0x8f) && reply[3] == feature && reply[4] == function)
}

fn hidpp_payload(report: &[u8]) -> Option<&[u8]> {
    let length = match report.first()? {
        0x10 => 7,
        0x11 => 20,
        _ => return None,
    };
    report.get(4..length)
}

fn register_reply_matches(
    reply: &[u8],
    address: u8,
    sub_id: u8,
    register: u8,
    echo: Option<u8>,
) -> bool {
    reply.len() >= 7
        && matches!(reply[0], 0x10 | 0x11)
        && reply[1] == address
        && ((reply[2] == sub_id
            && reply[3] == register
            && echo.is_none_or(|echo| reply[4] == echo))
            || (reply[2] == 0x8f && reply[3] == sub_id && reply[4] == register))
}

fn legacy_identity_key(pairing: &[u8], extended: &[u8]) -> Option<String> {
    let product = pairing.get(3..5)?.try_into().ok().map(u16::from_be_bytes)?;
    let serial = extended.get(1..5)?;
    if product == 0
        || product == 0xffff
        || serial.iter().all(|byte| *byte == 0)
        || serial.iter().all(|byte| *byte == 0xff)
    {
        return None;
    }
    let serial = serial
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Some(crate::device::stable_key(&format!(
        "logitech-legacy:{product:04x}:{serial}"
    )))
}

fn decode_receiver_name(data: &[u8]) -> Option<String> {
    let length = usize::from(*data.get(1)?);
    if length == 0 || length > 128 {
        return None;
    }
    let name = std::str::from_utf8(data.get(2..2 + length)?).ok()?.trim();
    (!name.is_empty() && !name.chars().any(char::is_control)).then(|| name.to_string())
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
            if data[0] > 100 || matches!(status, 5 | 6) {
                return None;
            }
            // HID++ 的 0 通常表示无法给出精确百分比，不能当作电池已耗尽。
            let percent = (data[0] > 0).then_some(data[0]);
            let level = (if feature == UNIFIED_BATTERY {
                match data[1] {
                    1 => Some(BatteryLevel::Critical), 2 => Some(BatteryLevel::Low),
                    4 => Some(BatteryLevel::Good), 8 => Some(BatteryLevel::Full), _ => None,
                }
            } else { None }).or_else(|| (status == 3).then_some(BatteryLevel::Full));
            Some(Battery {
                percent,
                estimated_percent: if percent.is_none() { level.map(level_estimate) } else { None },
                charging,
                voltage_mv: None,
                approximate: percent.is_none(),
                level,
            })
        }
        BATTERY_VOLTAGE => Some(Battery {
            percent: None,
            estimated_percent: (data[2] & 0x20 != 0).then_some(5),
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

fn decode_extended_dpi(data: &[u8], separate_axes: bool) -> Option<(u16, Option<u16>)> {
    if data.len() < 10 || data[0] != 0 { return None; }
    let value = |start: usize| {
        let current = u16::from_be_bytes([data[start], data[start + 1]]);
        let fallback = u16::from_be_bytes([data[start + 2], data[start + 3]]);
        if current == 0 { fallback } else { current }
    };
    let x = value(1);
    let y = separate_axes.then(|| value(5));
    (x >= 50 && y.is_none_or(|y| y >= 50)).then_some((x, y))
}

fn decode_legacy_battery(register: u8, data: &[u8]) -> Option<Battery> {
    if data.len() < 3 {
        return None;
    }
    match register {
        0x0d if data[0] <= 100 => Some(Battery {
            percent: Some(data[0]),
            estimated_percent: None,
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
            estimated_percent: match data[0] { 1 => Some(5), 3 => Some(20), 5 => Some(50), 7 => Some(90), _ => None },
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

// Representative values used by Solaar for HID++ qualitative levels.
fn level_estimate(level: BatteryLevel) -> u8 {
    match level {
        BatteryLevel::Critical => 5, BatteryLevel::Low => 20,
        BatteryLevel::Medium | BatteryLevel::Good => 50, BatteryLevel::Full => 90,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum DpiListError {
    Incomplete,
    Invalid(&'static str),
}

impl DpiListError {
    fn message(&self) -> &'static str {
        match self {
            Self::Incomplete => "DPI 列表不完整",
            Self::Invalid(message) => message,
        }
    }
}

fn decode_dpi_list(data: &[u8]) -> Result<Vec<u16>, DpiListError> {
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
            let previous = *values
                .last()
                .ok_or(DpiListError::Invalid("DPI 范围缺少起始值"))?;
            let end = words.next().ok_or(DpiListError::Incomplete)?;
            if step == 0 || end <= previous || (end - previous) % step != 0 {
                return Err(DpiListError::Invalid("DPI 范围无效"));
            }
            if values.len() + usize::from((end - previous) / step) > usize::from(u16::MAX) {
                return Err(DpiListError::Invalid("DPI 范围过长"));
            }
            for value in
                (u32::from(previous) + u32::from(step)..=u32::from(end)).step_by(step as usize)
            {
                values.push(value as u16);
            }
        } else {
            values.push(word);
        }
        if values.len() > usize::from(u16::MAX) {
            return Err(DpiListError::Invalid("DPI 范围过长"));
        }
    }
    if !terminated {
        return Err(DpiListError::Incomplete);
    }
    values.sort_unstable();
    values.dedup();
    Ok(values)
}

impl DeviceBackend for Logitech {
    fn discover(&self, _: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        let mut targets = Vec::new();
        for interface in preferred_interfaces(interfaces) {
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
        let lock = super::channel::lock(&collection_group(target.hid()?));
        let _guard = lock.lock().unwrap_or_else(|p|p.into_inner());
        let mut session = Session::open(target)?;
        let mut snapshot = session.read_snapshot(target, detail)?;
        if snapshot.target.family == DeviceFamily::Mouse {
            snapshot.polling_rate = session.rate_info(&snapshot.target).ok().flatten().map(|info|info.rate).unwrap_or_default();
        }
        Ok(snapshot)
    }

    fn set_dpi(&self, _: &HidApi, target: &Target, dpi: u16) -> Result<(), String> {
        let lock = super::channel::lock(&collection_group(target.hid()?));
        let _guard = lock.lock().unwrap_or_else(|p|p.into_inner());
        Session::open(target)?.set_dpi(target, dpi)
    }
    fn set_dpi_stage(
        &self,
        _: &HidApi,
        target: &Target,
        stage: crate::device::DpiStage,
        context: crate::device::StageContext,
    ) -> Result<(), String> {
        let lock = super::channel::lock(&collection_group(target.hid()?));
        let _guard = lock.lock().unwrap_or_else(|p|p.into_inner());
        Session::open(target)?.set_stage(target, stage, context.profile()?)
    }
    fn set_polling_rate(&self, _: &HidApi,target:&Target,hz:u16) -> Result<(),String> {
        let lock = super::channel::lock(&collection_group(target.hid()?));
        let _guard = lock.lock().unwrap_or_else(|p|p.into_inner());
        Session::open(target)?.set_rate(target,hz)
    }
}

fn collection_group(interface: &HidInterface) -> String {
    let path = interface.path.to_ascii_lowercase();
    let mut parts: Vec<String> = path.split('#').map(str::to_string).collect();
    if parts.len() < 4 { return path; }
    parts[1] = parts[1].split('&').filter(|part| !part.starts_with("col"))
        .collect::<Vec<_>>().join("&");
    if let Some((instance, suffix)) = parts[2].rsplit_once('&') {
        if suffix.len() == 4 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            parts[2] = instance.to_string();
        }
    }
    parts.join("#")
}

fn preferred_interfaces(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    let mut groups = std::collections::BTreeMap::new();
    for interface in interfaces.iter().filter(|interface| interface.vendor_id == 0x046d
        && interface.usage_page >= 0xff00 && interface.input_len >= 7 && interface.output_len >= 7) {
        let rank = |interface: &HidInterface| (interface.input_len >= 20 && interface.output_len >= 20,
            interface.usage_page == 0xff00 && interface.usage == 2);
        let best = groups.entry(collection_group(interface)).or_insert(interface);
        if rank(interface) > rank(best) { *best = interface; }
    }
    groups.into_values().collect()
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
        SizedData {
            report_id: u8,
            length: usize,
            payload: Vec<u8>,
        },
    }
    struct Step {
        address: Option<u8>,
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
            if let Some(address) = step.address {
                assert_eq!(packet[1], address);
            }
            let function = if matches!(packet[2], 0x81 | 0x83) {
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
                Reply::SizedData {
                    report_id,
                    length,
                    payload,
                } => {
                    response.resize(length, 0);
                    response[0] = report_id;
                    response[4..4 + payload.len()].copy_from_slice(&payload);
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
            address: None,
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
    fn rate_queries() -> Vec<Step> {
        vec![ping(),data(0,0,&[0x80,0x61],&[0]),data(0,0,&[0x80,0x60],&[2]),
            data(2,0,&[],&[0x85]),data(2,1,&[],&[3]),data(0,0,&[0x81,0],&[0])]
    }
    #[test]
    fn report_rate_bitmap_retains_non_power_of_two_intervals() {
        let (channel,steps)=channel(rate_queries());
        let info=Session::from_channel(channel,&target()).unwrap().rate_info(&target()).unwrap().unwrap();
        assert_eq!(info.rate.current,Some(333));assert_eq!(info.rate.values,vec![1000,333,125]);
        assert!(info.rate.writable);assert!(steps.lock().unwrap().is_empty());
    }
    #[test]
    fn report_rate_write_uses_advertised_raw_interval_and_requires_readback() {
        let mut script=rate_queries();script.extend([data(2,2,&[8],&[]),data(2,1,&[],&[8])]);
        let (channel,steps)=channel(script);
        Session::from_channel(channel,&target()).unwrap().set_rate(&target(),125).unwrap();
        assert!(steps.lock().unwrap().is_empty());
        let mut script=rate_queries();script.extend([data(2,2,&[8],&[]),data(2,1,&[],&[3])]);
        let (channel,steps)=self::channel(script);
        assert!(Session::from_channel(channel,&target()).unwrap().set_rate(&target(),125).is_err());
        assert!(steps.lock().unwrap().is_empty());
    }
    #[test]
    fn unsupported_report_rate_never_sends_a_write() {
        let (channel,steps)=channel(rate_queries());
        assert!(Session::from_channel(channel,&target()).unwrap().set_rate(&target(),8000).is_err());
        assert!(steps.lock().unwrap().is_empty());
    }
    fn usb_rate_identity() -> Vec<u8> {
        vec![1,1,2,3,4,0,8,0xc0,0x8b,0,0,0,0]
    }
    fn extended_rate_queries() -> Vec<Step> {
        vec![ping(),data(0,0,&[0x80,0x61],&[2]),data(0,0,&[0,3],&[3]),
            data(3,0,&[],&usb_rate_identity()),data(2,1,&[0],&[0,0x7f]),
            data(2,2,&[0],&[3]),data(0,0,&[0x81,0],&[0])]
    }
    #[test]
    fn extended_rate_selects_usb_capabilities_from_runtime_transport_identity() {
        let (channel,steps)=channel(extended_rate_queries());
        let info=Session::from_channel(channel,&target()).unwrap().rate_info(&target()).unwrap().unwrap();
        assert_eq!(info.connection,Some(0));assert_eq!(info.rate.current,Some(1000));
        assert_eq!(info.rate.values,vec![125,250,500,1000,2000,4000,8000]);
        assert!(steps.lock().unwrap().is_empty());
    }
    #[test]
    fn extended_rate_write_takes_only_rate_code_then_verifies_the_same_link() {
        let mut script=extended_rate_queries();script.extend([data(3,0,&[],&usb_rate_identity()),
            data(2,3,&[6],&[]),data(3,0,&[],&usb_rate_identity()),data(2,2,&[0],&[6])]);
        let (channel,steps)=channel(script);
        Session::from_channel(channel,&target()).unwrap().set_rate(&target(),8000).unwrap();
        assert!(steps.lock().unwrap().is_empty());
    }
    #[test]
    fn unknown_extended_rate_transport_is_not_treated_as_wired() {
        let (channel,steps)=channel(vec![ping(),data(0,0,&[0x80,0x61],&[2]),
            data(0,0,&[0,3],&[0])]);
        assert!(Session::from_channel(channel,&target()).unwrap().rate_info(&target()).unwrap().is_none());
        assert!(steps.lock().unwrap().is_empty());
    }
    #[test]
    fn onboard_mode_exposes_current_rate_without_enabling_live_writes() {
        let mut script=rate_queries();script.pop();script.extend([data(0,0,&[0x81,0],&[4]),data(4,2,&[],&[1])]);
        let (channel,steps)=channel(script);
        let info=Session::from_channel(channel,&target()).unwrap().rate_info(&target()).unwrap().unwrap();
        assert_eq!(info.rate.current,Some(333));assert!(!info.rate.writable);
        assert!(steps.lock().unwrap().is_empty());
    }
    #[test]
    fn failed_mode_query_preserves_read_rate_without_write_permission() {
        let mut script=rate_queries();script.pop();script.push(data(0,0,&[0x81,0],&[4]));
        let mut failed=data(4,2,&[],&[]);failed.reply=Reply::Error(0xff,7);script.push(failed);
        let (channel,steps)=channel(script);
        let info=Session::from_channel(channel,&target()).unwrap().rate_info(&target()).unwrap().unwrap();
        assert_eq!(info.rate.current,Some(333));assert!(!info.rate.writable);
        assert!(steps.lock().unwrap().is_empty());
    }
    fn dpi_queries() -> Vec<Step> {
        vec![
            ping(),
            data(0, 0, &[0x22, 1], &[2]),
            data(2, 0, &[], &[1]),
            data(2, 1, &[0, 0, 0], &[0, 1, 0x90, 3, 0x20, 6, 0x40, 0, 0]),
            data(2, 2, &[0], &[0, 3, 0x20, 3, 0x20]),
        ]
    }

    fn extended_queries(separate_axes: bool, x: u16, y: u16) -> Vec<Step> {
        let flags = if separate_axes { 3 } else { 2 };
        let mut script = vec![ping(), data(0, 0, &[0x22, 1], &[0]),
            data(0, 0, &[0x22, 2], &[4]), data(4, 0, &[], &[1]),
            data(4, 1, &[0], &[0, 0, flags]),
            data(4, 2, &[0, 0, 0], &[0, 0, 0, 1, 0x90, 3, 0x20, 6, 0x40, 0, 0])];
        if separate_axes { script.push(data(4, 2, &[0, 1, 0],
            &[0, 1, 0, 1, 0x90, 3, 0x20, 6, 0x40, 0, 0])); }
        let [xh, xl] = x.to_be_bytes();
        let [yh, yl] = y.to_be_bytes();
        script.push(data(4, 5, &[0], &[0, xh, xl, xh, xl, yh, yl, yh, yl, 2]));
        script
    }

    #[test]
    fn extended_dpi_reads_real_axes_and_never_calls_the_range_an_onboard_stage_list() {
        let (channel, steps) = channel(extended_queries(true, 800, 1600));
        let dpi = Session::from_channel(channel, &target()).unwrap().dpi().unwrap();
        assert_eq!(dpi.current, Some(800));
        assert_eq!(dpi.current_y, Some(1600));
        assert_eq!(dpi.values, vec![400, 800, 1600]);
        assert!(dpi.stages.is_empty());
        assert!(!dpi.writable);
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn extended_dpi_supports_the_full_hero_two_range_without_quadratic_axis_matching() {
        let script = vec![ping(), data(0, 0, &[0x22, 1], &[0]),
            data(0, 0, &[0x22, 2], &[4]), data(4, 0, &[], &[1]),
            data(4, 1, &[0], &[0, 0, 3]),
            data(4, 2, &[0, 0, 0], &[0, 0, 0, 0, 100, 0xe0, 1, 0xab, 0xe0, 0, 0]),
            data(4, 2, &[0, 1, 0], &[0, 1, 0, 0, 100, 0xe0, 1, 0xab, 0xe0, 0, 0]),
            data(4, 5, &[0], &[0, 3, 0x20, 3, 0x20, 3, 0x20, 3, 0x20, 2])];
        let (channel, steps) = channel(script);
        let dpi = Session::from_channel(channel, &target()).unwrap().dpi().unwrap();
        assert_eq!(dpi.current, Some(800));
        assert_eq!(dpi.current_y, Some(800));
        assert_eq!(dpi.values.len(), 43_901);
        assert_eq!(dpi.values.first(), Some(&100));
        assert_eq!(dpi.values.last(), Some(&44_000));
        assert!(dpi.writable);
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn an_invalid_extended_dpi_list_does_not_hide_current_dpi_or_authorize_a_write() {
        let script = || vec![ping(), data(0, 0, &[0x22, 1], &[0]),
            data(0, 0, &[0x22, 2], &[4]), data(4, 0, &[], &[1]),
            data(4, 1, &[0], &[0, 0, 3]),
            data(4, 2, &[0, 0, 0], &[1, 0, 0, 0, 100, 0, 0]),
            data(4, 5, &[0], &[0, 3, 0x20, 3, 0x20, 3, 0x20, 3, 0x20, 2])];
        let (channel, steps) = channel(script());
        let dpi = Session::from_channel(channel, &target()).unwrap().dpi().unwrap();
        assert_eq!(dpi.current, Some(800));
        assert!(dpi.values.is_empty());
        assert!(!dpi.writable);
        assert!(steps.lock().unwrap().is_empty());
        let (channel, steps) = self::channel(script());
        assert!(Session::from_channel(channel, &target()).unwrap().set_dpi(&target(), 1600).is_err());
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn extended_dpi_write_preserves_lod_and_uses_zero_for_unsupported_y_axis() {
        let mut script = extended_queries(false, 800, 0);
        script.extend([data(0, 0, &[0x22, 1], &[0]), data(0, 0, &[0x81, 0], &[0]),
            data(4, 1, &[0], &[0, 0, 2]), data(4, 5, &[0], &[0, 3, 0x20, 3, 0x20, 0, 0, 0, 0, 2]),
            data(4, 6, &[0, 6, 0x40, 0, 0, 2], &[0]),
            data(4, 5, &[0], &[0, 6, 0x40, 3, 0x20, 0, 0, 0, 0, 2])]);
        let (channel, steps) = channel(script);
        Session::from_channel(channel, &target()).unwrap().set_dpi(&target(), 1600).unwrap();
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn extended_dpi_rejected_readback_restores_both_axes_and_original_lod() {
        let mut script = extended_queries(true, 800, 800);
        let original = [0, 3, 0x20, 3, 0x20, 3, 0x20, 3, 0x20, 2];
        script.extend([data(0, 0, &[0x22, 1], &[0]), data(0, 0, &[0x81, 0], &[0]),
            data(4, 1, &[0], &[0, 0, 3]), data(4, 5, &[0], &original),
            data(4, 6, &[0, 6, 0x40, 6, 0x40, 2], &[0]), data(4, 5, &[0], &original),
            data(4, 6, &[0, 3, 0x20, 3, 0x20, 2], &[0]), data(4, 5, &[0], &original)]);
        let (channel, steps) = channel(script);
        assert!(Session::from_channel(channel, &target()).unwrap().set_dpi(&target(), 1600)
            .unwrap_err().contains("已恢复原值"));
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn long_collection_selection_does_not_merge_two_identical_receivers() {
        let mut short = target().hid().unwrap().clone();
        short.path = r"\\?\hid#vid_046d&pid_c547&mi_01&col01#8&abc&0&0000#{hid}".into();
        short.input_len = 7; short.output_len = 7;
        let mut long = short.clone();
        long.path = r"\\?\hid#vid_046d&pid_c547&mi_01&col02#8&abc&0&0001#{hid}".into();
        long.input_len = 20; long.output_len = 20; long.usage = 2;
        let mut second = long.clone();
        second.path = r"\\?\hid#vid_046d&pid_c547&mi_01&col02#8&def&0&0001#{hid}".into();
        let interfaces = [short, long.clone(), second];
        let chosen = preferred_interfaces(&interfaces);
        assert_eq!(chosen.len(), 2);
        assert!(chosen.iter().any(|interface| interface.path == long.path));
        assert!(chosen.iter().all(|interface| interface.output_len == 20));
    }

    fn profile_bytes() -> Vec<u8> {
        let mut bytes = vec![0xff; 256];
        bytes[1] = 1;
        for (index, value) in [400u16, 800, 0, 800, 3200].into_iter().enumerate() {
            bytes[3 + 2 * index..5 + 2 * index].copy_from_slice(&value.to_le_bytes());
        }
        let crc = crc16(&bytes[..254]).to_be_bytes();
        bytes[254..].copy_from_slice(&crc);
        bytes
    }

    fn onboard_queries(bytes: &[u8], active: u8) -> Vec<Step> {
        let mut script = vec![
            ping(),
            data(0, 0, &[0x81, 0], &[3]),
            data(3, 2, &[], &[1]),
            data(3, 0, &[], &[1, 3, 0, 5, 1, 6, 0, 1, 0, 0]),
            data(3, 4, &[], &[0, 1]),
        ];
        for (index, chunk) in bytes.chunks(16).enumerate() {
            script.push(data(3, 5, &[0, 1, 0, (index * 16) as u8], chunk));
        }
        script.extend([
            data(3, 4, &[], &[0, 1]),
            data(3, 11, &[], &[active]),
            data(3, 2, &[], &[1]),
        ]);
        script
    }

    #[test]
    fn onboard_crc_rejects_corruption_and_preserves_fifth_slot_and_duplicates() {
        assert_eq!(crc16(b"123456789"), 0x29b1);
        let mut bytes = profile_bytes();
        let stages = decode_onboard_stages(&bytes, 4).unwrap();
        assert_eq!(
            stages
                .iter()
                .map(|stage| (stage.index, stage.value))
                .collect::<Vec<_>>(),
            vec![(0, 400), (1, 800), (3, 800), (4, 3200)]
        );
        assert!(decode_onboard_stages(&bytes, 2).is_err());
        bytes[12] ^= 1;
        assert!(decode_onboard_stages(&bytes, 4).is_err());
    }

    #[test]
    fn onboard_stages_are_read_without_mode_profile_or_flash_writes() {
        let (channel, steps) = channel(onboard_queries(&profile_bytes(), 4));
        let dpi = Session::from_channel(channel, &target())
            .unwrap()
            .dpi_with_stages();
        assert_eq!(dpi.current, Some(3200));
        assert_eq!(dpi.active_stage, Some(4));
        assert_eq!(dpi.profile, Some(1));
        assert!(dpi.stage_writable);
        assert!(!dpi.writable);
        assert!(dpi.values.is_empty());
        assert_eq!(dpi.stages.len(), 4);
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn host_mode_uses_numeric_capability_without_switching_to_onboard() {
        let mut script = vec![ping(), data(0, 0, &[0x81, 0], &[3]), data(3, 2, &[], &[2])];
        script.extend(dpi_queries().into_iter().skip(1));
        let (channel, steps) = channel(script);
        let dpi = Session::from_channel(channel, &target())
            .unwrap()
            .dpi_with_stages();
        assert_eq!(dpi.current, Some(800));
        assert!(dpi.stages.is_empty());
        assert!(dpi.writable);
        assert!(!dpi.stage_writable);
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn failed_onboard_query_cannot_fall_back_to_numeric_write_permission() {
        let mut script = vec![
            ping(),
            data(0, 0, &[0x81, 0], &[3]),
            Step {
                address: None,
                feature: 3,
                function: 2,
                parameters: vec![],
                reply: Reply::Error(0xff, 2),
            },
        ];
        script.extend(dpi_queries().into_iter().skip(1));
        let (channel, steps) = channel(script);
        let dpi = Session::from_channel(channel, &target())
            .unwrap()
            .dpi_with_stages();
        assert_eq!(dpi.current, Some(800));
        assert!(!dpi.writable);
        assert!(!dpi.stage_writable);
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn unsupported_profile_format_stops_before_reading_memory_or_writing() {
        let (channel, steps) = channel(vec![
            ping(),
            data(0, 0, &[0x81, 0], &[3]),
            data(3, 2, &[], &[1]),
            data(3, 0, &[], &[1, 9, 0, 5, 1, 6, 0, 1, 0, 0]),
        ]);
        assert!(Session::from_channel(channel, &target())
            .unwrap()
            .onboard_dpi()
            .is_err());
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn onboard_stage_switch_uses_function_twelve_and_verifies_the_actual_slot() {
        let mut script = onboard_queries(&profile_bytes(), 1);
        script.extend([
            data(3, 12, &[4], &[]),
            data(3, 2, &[], &[1]),
            data(3, 4, &[], &[0, 1]),
            data(3, 11, &[], &[4]),
        ]);
        let (channel, steps) = channel(script);
        Session::from_channel(channel, &target())
            .unwrap()
            .set_stage(
                &target(),
                crate::device::DpiStage {
                    index: 4,
                    value: 3200,
                },
                1,
            )
            .unwrap();
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn stale_stage_or_profile_and_corrupt_memory_cannot_authorize_a_write() {
        for (stage, profile, corrupt) in [
            (
                crate::device::DpiStage {
                    index: 4,
                    value: 1600,
                },
                1,
                false,
            ),
            (
                crate::device::DpiStage {
                    index: 4,
                    value: 3200,
                },
                2,
                false,
            ),
            (
                crate::device::DpiStage {
                    index: 2,
                    value: 800,
                },
                1,
                false,
            ),
            (
                crate::device::DpiStage {
                    index: 4,
                    value: 3200,
                },
                1,
                true,
            ),
        ] {
            let mut bytes = profile_bytes();
            if corrupt {
                bytes[12] ^= 1;
            }
            let (channel, steps) = channel(onboard_queries(&bytes, 1));
            assert!(Session::from_channel(channel, &target())
                .unwrap()
                .set_stage(&target(), stage, profile)
                .is_err());
            assert!(steps.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn unchanged_onboard_stage_does_not_write() {
        let (channel, steps) = channel(onboard_queries(&profile_bytes(), 1));
        Session::from_channel(channel, &target())
            .unwrap()
            .set_stage(
                &target(),
                crate::device::DpiStage {
                    index: 1,
                    value: 800,
                },
                1,
            )
            .unwrap();
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn failed_onboard_readback_restores_and_verifies_original_stage() {
        let mut script = onboard_queries(&profile_bytes(), 1);
        script.extend([
            data(3, 12, &[4], &[]),
            data(3, 2, &[], &[1]),
            data(3, 4, &[], &[0, 1]),
            Step {
                address: None,
                feature: 3,
                function: 11,
                parameters: vec![],
                reply: Reply::Error(0xff, 2),
            },
            data(3, 12, &[1], &[]),
            data(3, 2, &[], &[1]),
            data(3, 4, &[], &[0, 1]),
            data(3, 11, &[], &[1]),
        ]);
        let (channel, steps) = channel(script);
        assert_eq!(
            Session::from_channel(channel, &target())
                .unwrap()
                .set_stage(
                    &target(),
                    crate::device::DpiStage {
                        index: 4,
                        value: 3200
                    },
                    1
                )
                .unwrap_err(),
            "设备未确认新的 DPI，已恢复原值"
        );
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn profile_change_after_write_never_restores_into_another_profile() {
        let mut script = onboard_queries(&profile_bytes(), 1);
        script.extend([
            data(3, 12, &[4], &[]),
            data(3, 2, &[], &[1]),
            data(3, 4, &[], &[0, 2]),
        ]);
        let (channel, steps) = channel(script);
        assert_eq!(
            Session::from_channel(channel, &target())
                .unwrap()
                .set_stage(
                    &target(),
                    crate::device::DpiStage {
                        index: 4,
                        value: 3200
                    },
                    1
                )
                .unwrap_err(),
            "板载配置已变化，请刷新后重试"
        );
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn numeric_write_is_rejected_if_ghub_changed_the_mode_to_onboard() {
        let mut script = dpi_queries();
        script.extend([data(0, 0, &[0x81, 0], &[3]), data(3, 2, &[], &[1])]);
        let (channel, steps) = channel(script);
        assert_eq!(
            Session::from_channel(channel, &target())
                .unwrap()
                .set_dpi(&target(), 1600)
                .unwrap_err(),
            "板载模式请使用 DPI 档位切换"
        );
        assert!(steps.lock().unwrap().is_empty());
    }

    fn paged_dpi_queries(words: &[u16], current: u16, padded_first_page: bool) -> Vec<Step> {
        let mut script = vec![ping(), data(0, 0, &[0x22, 1], &[2]), data(2, 0, &[], &[1])];
        let bytes = words
            .iter()
            .flat_map(|word| word.to_be_bytes())
            .collect::<Vec<_>>();
        for (page, chunk) in bytes.chunks(15).enumerate() {
            let mut payload = vec![0];
            payload.extend_from_slice(chunk);
            let mut step = data(2, 1, &[0, 0, page as u8], &payload);
            if padded_first_page && page == 0 {
                step.reply = Reply::SizedData {
                    report_id: 0x11,
                    length: 64,
                    payload,
                };
            }
            script.push(step);
        }
        let [hi, lo] = current.to_be_bytes();
        script.push(data(2, 2, &[0], &[0, hi, lo, hi, lo]));
        script
    }

    #[test]
    fn paginated_dpi_list_ignores_transport_padding_and_reads_all_values() {
        let values = [400, 800, 1200, 1600, 2000, 2400, 2800, 3200];
        let mut words = values.to_vec();
        words.push(0);
        let (channel, steps) = channel(paged_dpi_queries(&words, 800, true));
        let mut session = Session::from_channel(channel, &target()).unwrap();
        let dpi = session.dpi().unwrap();
        assert_eq!(dpi.values, values);
        assert_eq!(dpi.current, Some(800));
        assert!(dpi.writable);
        assert!(!dpi.values.contains(&3072));
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn dpi_range_marker_and_endpoint_can_cross_report_boundaries() {
        let (channel, steps) = channel(paged_dpi_queries(
            &[100, 200, 300, 400, 500, 600, 1000, 0xe032, 1500, 0],
            1000,
            false,
        ));
        let mut session = Session::from_channel(channel, &target()).unwrap();
        let dpi = session.dpi().unwrap();
        assert!(dpi.values.contains(&1050));
        assert!(dpi.values.contains(&1500));
        assert!(!dpi.values.contains(&800));
        assert_eq!(dpi.current, Some(1000));
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn value_advertised_on_a_later_page_can_be_written_and_verified() {
        let mut script = paged_dpi_queries(
            &[400, 800, 1200, 1600, 2000, 2400, 2800, 3200, 0],
            800,
            false,
        );
        script.extend([
            data(0, 0, &[0x81, 0], &[0]),
            data(2, 3, &[0, 0x0c, 0x80], &[]),
            data(2, 2, &[0], &[0, 0x0c, 0x80, 3, 0x20]),
        ]);
        let (channel, steps) = channel(script);
        Session::from_channel(channel, &target())
            .unwrap()
            .set_dpi(&target(), 3200)
            .unwrap();
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn unavailable_later_dpi_page_cannot_authorize_a_write_from_a_partial_list() {
        let mut script = paged_dpi_queries(
            &[400, 800, 1200, 1600, 2000, 2400, 2800, 3200, 0],
            800,
            false,
        );
        script.truncate(4);
        script.push(Step {
            address: None,
            feature: 2,
            function: 1,
            parameters: vec![0, 0, 1],
            reply: Reply::Error(0xff, 2),
        });
        let (channel, steps) = channel(script);
        assert!(Session::from_channel(channel, &target())
            .unwrap()
            .set_dpi(&target(), 800)
            .is_err());
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn unterminated_dpi_lists_stop_at_the_protocol_page_limit_without_writing() {
        let mut script = vec![ping(), data(0, 0, &[0x22, 1], &[2]), data(2, 0, &[], &[1])];
        let mut payload = vec![1; 16];
        payload[0] = 0;
        for page in 0..=u8::MAX {
            script.push(data(2, 1, &[0, 0, page], &payload));
        }
        let (channel, steps) = channel(script);
        let error = Session::from_channel(channel, &target())
            .unwrap()
            .set_dpi(&target(), 800)
            .unwrap_err();
        assert_eq!(error, "DPI 列表不完整");
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn truncated_long_report_cannot_complete_protocol_negotiation() {
        let mut step = ping();
        step.reply = Reply::SizedData {
            report_id: 0x11,
            length: 7,
            payload: vec![2, 0, 0x5a],
        };
        let (channel, steps) = channel(vec![step]);
        assert!(Session::from_channel(channel, &target()).is_err());
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn legacy_negotiation_reads_battery_without_feature_queries_or_dpi_writes() {
        let (channel, steps) = channel(vec![
            Step {
                address: None,
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
                address: None,
                feature: 0,
                function: 0,
                parameters: vec![0, 3],
                reply: Reply::Error(0xff, 2),
            },
            data(0, 0, &[0x81, 0], &[0]),
            data(0, 0, &[0x22, 1], &[0]),
            data(0, 0, &[0x22, 2], &[0]),
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
    fn legacy_receiver_metadata_identifies_the_selected_slot() {
        let mut target = target();
        target.address = 2;
        target.hid_mut().product_id = 0xc52b;
        target.family = DeviceFamily::Unknown;
        target.name = "Unifying Receiver".into();
        let negotiation = Step {
            address: Some(2),
            feature: 0,
            function: 1,
            parameters: vec![0, 0, 0x5a],
            reply: Reply::Error(0x8f, 1),
        };
        let mut battery = data(0x81, 0x0d, &[], &[42, 0, 0x30]);
        battery.address = Some(2);
        let mut pairing = data(0x83, 0xb5, &[0x21], &[0x21, 0, 0, 0x40, 0x08, 0, 0, 1]);
        pairing.address = Some(0xff);
        let mut extended = data(0x83, 0xb5, &[0x31], &[0x31, 0x12, 0x34, 0x56, 0x78]);
        extended.address = Some(0xff);
        let mut name = data(0x83, 0xb5, &[0x41], &[0x41, 4, b'K', b'8', b'1', b'0']);
        name.address = Some(0xff);
        let (channel, steps) = channel(vec![negotiation, battery, pairing, extended, name]);
        let snapshot = Session::from_channel(channel, &target)
            .unwrap()
            .read_snapshot(&target, true)
            .unwrap();
        assert_eq!(snapshot.target.name, "K810");
        assert_eq!(snapshot.target.family, DeviceFamily::Keyboard);
        assert!(snapshot.target.verified_identity);
        assert_eq!(
            snapshot.target.key,
            crate::device::stable_key("logitech-legacy:4008:12345678")
        );
        assert_eq!(snapshot.battery.unwrap().percent, Some(42));
        assert!(!snapshot.dpi.writable);
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn legacy_metadata_failures_preserve_successful_battery_reading() {
        let mut target = target();
        target.address = 1;
        target.hid_mut().product_id = 0xc52b;
        target.family = DeviceFamily::Unknown;
        let mut script = vec![
            Step {
                address: Some(1),
                feature: 0,
                function: 1,
                parameters: vec![0, 0, 0x5a],
                reply: Reply::Error(0x8f, 1),
            },
            data(0x81, 0x0d, &[], &[42, 0, 0x30]),
        ];
        for slot in [0x20, 0x30, 0x40] {
            script.push(Step {
                address: Some(0xff),
                feature: 0x83,
                function: 0xb5,
                parameters: vec![slot],
                reply: Reply::Error(0x8f, 2),
            });
        }
        let (channel, steps) = channel(script);
        let snapshot = Session::from_channel(channel, &target)
            .unwrap()
            .read_snapshot(&target, true)
            .unwrap();
        assert!(snapshot.connected);
        assert_eq!(snapshot.battery.unwrap().percent, Some(42));
        assert_eq!(snapshot.target.key, target.key);
        assert!(!snapshot.target.verified_identity);
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn receiver_metadata_rejects_other_slots_and_incomplete_identity() {
        assert!(register_reply_matches(
            &[0x11, 0xff, 0x83, 0xb5, 0x20, 0, 0],
            0xff,
            0x83,
            0xb5,
            Some(0x20)
        ));
        assert!(!register_reply_matches(
            &[0x11, 0xff, 0x83, 0xb5, 0x21, 0, 0],
            0xff,
            0x83,
            0xb5,
            Some(0x20)
        ));
        assert!(!register_reply_matches(
            &[0x11, 2, 0x83, 0xb5, 0x20, 0, 0],
            0xff,
            0x83,
            0xb5,
            Some(0x20)
        ));
        assert_eq!(
            legacy_identity_key(&[0x20, 0, 0, 0x40, 8], &[0x30, 0, 0, 0, 0]),
            None
        );
        assert_eq!(
            legacy_identity_key(&[0x20, 0, 0, 0x40, 8], &[0x30, 0xff, 0xff, 0xff, 0xff]),
            None
        );
        assert_eq!(decode_receiver_name(&[0x40, 4, b'K']), None);
        assert_eq!(decode_receiver_name(&[0x40, 1, b'\n']), None);
    }

    #[test]
    fn charge_status_without_percentage_is_coarse_and_invalid_batteries_are_rejected() {
        let full = decode_battery(UNIFIED_BATTERY, &[0, 8, 3]).unwrap();
        assert_eq!(full.percent, None);
        assert_eq!(full.level, Some(BatteryLevel::Full));
        assert!(full.approximate);
        assert_eq!(
            decode_battery(BATTERY_STATUS, &[0, 0, 3]).unwrap().percent,
            None
        );
        assert_eq!(decode_battery(UNIFIED_BATTERY, &[45, 4, 5]), None);
        assert_eq!(decode_battery(BATTERY_STATUS, &[45, 0, 6]), None);
    }

    #[test]
    fn coarse_unified_reading_can_fall_back_to_exact_battery_feature() {
        let (channel, steps) = channel(vec![
            ping(),
            data(0, 0, &[0x10, 4], &[1]),
            data(1, 1, &[], &[0, 4, 0]),
            data(0, 0, &[0x10, 0], &[2]),
            data(2, 0, &[], &[75, 20, 0]),
        ]);
        let mut session = Session::from_channel(channel, &target()).unwrap();
        let battery = session.battery().unwrap().unwrap();
        assert_eq!(battery.percent, Some(75));
        assert!(!battery.approximate);
        assert!(steps.lock().unwrap().is_empty());
    }

    #[test]
    fn coarse_battery_is_retained_when_other_features_are_unavailable() {
        let (channel, steps) = channel(vec![
            ping(),
            data(0, 0, &[0x10, 4], &[1]),
            data(1, 1, &[], &[0, 2, 0]),
            data(0, 0, &[0x10, 0], &[0]),
            data(0, 0, &[0x10, 1], &[0]),
        ]);
        let mut session = Session::from_channel(channel, &target()).unwrap();
        let battery = session.battery().unwrap().unwrap();
        assert_eq!(battery.percent, None);
        assert_eq!(battery.level, Some(BatteryLevel::Low));
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
            data(0, 0, &[0x81, 0], &[0]),
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
        for (raw, estimated) in [(1,5), (2,20), (4,50), (8,90)] {
            let b = decode_battery(UNIFIED_BATTERY, &[0, raw, 0]).unwrap();
            assert_eq!(b.percent, None);
            assert_eq!(b.display_percent().unwrap().label(true), format!("≈{estimated}%"));
        }
        for (raw, estimated) in [(1,5), (3,20), (5,50), (7,90)] {
            let b = decode_legacy_battery(7, &[raw, 0, 0]).unwrap();
            assert_eq!(b.display_percent().unwrap().label(true), format!("≈{estimated}%"));
        }
        assert!(decode_battery(BATTERY_VOLTAGE, &[0x10, 0, 0]).unwrap().display_percent().is_none());
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

    #[test]
    fn dpi_range_expansion_is_bounded_by_the_u16_domain_before_allocation() {
        let values = decode_dpi_list(&[0, 1, 0xe0, 1, 0xff, 0xff, 0, 0]).unwrap();
        assert_eq!(values.len(), usize::from(u16::MAX));
        assert_eq!(values.last(), Some(&u16::MAX));
        assert!(decode_dpi_list(&[0, 1, 0xe0, 1, 0xff, 0xff, 0, 1, 0, 0]).is_err());
    }
}
