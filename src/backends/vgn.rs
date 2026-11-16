//! V98 Pro V3 的北应控制协议；有线 Feature 与接收器 Output 分开处理。
use super::*;
use crate::hid::HidDevice;

pub struct Vgn;
const VID: u16 = 0x258a;
const WIRED_REPORT: u8 = 9;
const RECEIVER_REPORT: u8 = 19;
const WIRED_QUERY: [u8; 8] = [WIRED_REPORT, 0x87, 0, 0, 1, 0, 2, 0];

fn receiver(pid: u16) -> Option<bool> {
    match pid {
        0x024e | 0x027e => Some(false),
        0x024f | 0x027f => Some(true),
        _ => None,
    }
}

fn is_control(interface: &HidInterface) -> bool {
    interface.vendor_id == VID
        && interface.usage_page == 0xff02
        && interface.usage == 1
        && match receiver(interface.product_id) {
            Some(false) => (10..=4096).contains(&interface.feature_len),
            Some(true) => interface.input_len >= 20 && interface.output_len >= 20,
            None => false,
        }
}

fn receiver_query() -> [u8; 20] {
    let mut frame = [0; 20];
    frame[..6].copy_from_slice(&[RECEIVER_REPORT, 0x4a, 1, 0, 0, 0]);
    frame[19] = frame[..19]
        .iter()
        .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    frame
}

fn parse_battery(frame: &[u8], wireless: bool) -> Option<Battery> {
    let (percent, status) = if wireless {
        if frame.len() < 20
            || frame[0] != RECEIVER_REPORT
            || frame[1] != 0x4a
            || frame[19]
                != frame[..19]
                    .iter()
                    .fold(0u8, |sum, byte| sum.wrapping_add(*byte))
        {
            return None;
        }
        (frame[5], frame[6])
    } else {
        if frame.len() < 10 || frame[..8] != WIRED_QUERY {
            return None;
        }
        (frame[8], frame[9])
    };
    if percent > 100 {
        return None;
    }
    // 充电标志在高半字节；未定义的状态不能被解释成正在放电。
    let charging = match status >> 4 {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    };
    Some(Battery {
        percent: Some(percent),
        charging,
        voltage_mv: None,
        approximate: false,
        level: None,
    })
}

impl DeviceBackend for Vgn {
    fn discover(&self, _: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(interfaces
            .iter()
            .filter(|interface| is_control(interface))
            .map(|interface| {
                let mut target = Target::new(BackendId::Vgn, interface, 0);
                target.name = "VGN V98 Pro V3".into();
                target.family = DeviceFamily::Keyboard;
                target
            })
            .collect())
    }

    fn read(&self, _: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let wireless = receiver(target.interface.product_id)
            .ok_or_else(|| crate::tr!("此 VGN 型号尚未适配"))?;
        if !is_control(&target.interface) {
            return Err(crate::tr!("此 VGN 型号尚未适配").into());
        }
        let started = std::time::Instant::now();
        let reply = if wireless {
            HidDevice::open(&target.interface).and_then(|mut device| {
                device
                    .transact(&receiver_query(), 350, |frame| {
                        frame.len() >= 20 && frame[0] == RECEIVER_REPORT && frame[1] == 0x4a
                    })
                    .ok_or_else(|| crate::tr!("无法读取 VGN 电量").to_string())
            })
        } else {
            feature::FeatureChannel::open(&target.interface)
                .and_then(|mut channel| channel.query(&WIRED_QUERY, 350))
        };
        let battery = reply
            .as_ref()
            .ok()
            .and_then(|frame| parse_battery(frame, wireless));
        trace::record(
            &target.interface,
            BackendId::Vgn,
            "query-battery",
            if wireless { 0x4a } else { 0x87 },
            if wireless {
                RECEIVER_REPORT
            } else {
                WIRED_REPORT
            },
            if wireless {
                20
            } else {
                usize::from(target.interface.feature_len)
            },
            reply.as_ref().map(Vec::len).unwrap_or(0),
            started,
            match (&reply, &battery) {
                (Err(_), _) => trace::Outcome::NoReply,
                (_, None) => trace::Outcome::InvalidData,
                _ => trace::Outcome::Ok,
            },
        );
        reply?;
        let battery = battery.ok_or_else(|| crate::tr!("VGN 电量响应无效"))?;
        Ok(Snapshot {
            target: target.clone(),
            battery: Some(battery),
            dpi: Dpi::default(),
            connected: true,
            battery_at: Some(std::time::Instant::now()), polling_rate: Default::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wired_battery_requires_request_echo_and_valid_percentage() {
        let mut frame = WIRED_QUERY.to_vec();
        frame.extend([0, 0x10]);
        let battery = parse_battery(&frame, false).unwrap();
        assert_eq!(battery.percent, Some(0));
        assert_eq!(battery.charging, Some(true));
        frame[8] = 101;
        assert!(parse_battery(&frame, false).is_none());
        frame[8] = 70;
        frame[1] = 0x82;
        assert!(parse_battery(&frame, false).is_none());
        assert!(parse_battery(&WIRED_QUERY, false).is_none());
    }
    #[test]
    fn wireless_battery_never_invents_percentage_or_charging() {
        let mut frame = receiver_query();
        frame[5] = 100;
        frame[6] = 0x20;
        frame[19] = frame[..19]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        let battery = parse_battery(&frame, true).unwrap();
        assert_eq!(battery.percent, Some(100));
        assert_eq!(battery.charging, None);
        frame[19] ^= 1;
        assert!(parse_battery(&frame, true).is_none());
        assert!(parse_battery(&[19, 0x4a, 1, 0, 0, 255, 0], true).is_none());
    }
    #[test]
    fn unrelated_vendor_interfaces_are_not_probed() {
        let mut interface = HidInterface {
            vendor_id: VID,
            product_id: 0x024e,
            usage_page: 0xff02,
            usage: 1,
            feature_len: 520,
            input_len: 0,
            output_len: 0,
            path: String::new(),
            serial: String::new(),
            product: String::new(),
        };
        assert!(is_control(&interface));
        interface.usage_page = 0xff00;
        assert!(!is_control(&interface));
        interface.usage_page = 0xff02;
        interface.product_id = 0x024c;
        assert!(!is_control(&interface));
    }
}
