//! V98 Pro V3 的有线北应 Feature 电量查询。
use super::*;

pub struct Vgn;
const VID: u16 = 0x258a;
const WIRED_REPORT: u8 = 9;
const WIRED_QUERY: [u8; 8] = [WIRED_REPORT, 0x87, 0, 0, 1, 0, 2, 0];

fn wired(pid: u16) -> bool {
    matches!(pid, 0x024e | 0x027e)
}

fn is_control(interface: &HidInterface) -> bool {
    interface.vendor_id == VID
        && interface.usage_page == 0xff02
        && interface.usage == 1
        && wired(interface.product_id)
        && (10..=4096).contains(&interface.feature_len)
}

fn parse_battery(frame: &[u8]) -> Option<Battery> {
    if frame.len() < 10 || frame[..8] != WIRED_QUERY { return None; }
    let (percent, status) = (frame[8], frame[9]);
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
        estimated_percent: None,
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
        if target.backend != BackendId::Vgn || !is_control(target.hid()?) {
            return Err(crate::tr!("此 VGN 型号尚未适配").into());
        }
        let started = std::time::Instant::now();
        let reply = feature::FeatureChannel::open(target.hid()?)
            .and_then(|mut channel| channel.query(&WIRED_QUERY, 350));
        let battery = reply
            .as_ref()
            .ok()
            .and_then(|frame| parse_battery(frame));
        trace::record(
            target.hid()?,
            BackendId::Vgn,
            "query-battery",
            0x87,
            WIRED_REPORT,
            usize::from(target.hid()?.feature_len),
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
            battery_unavailable: false, battery_at: Some(std::time::Instant::now()), polling_rate: Default::default(),
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
        let battery = parse_battery(&frame).unwrap();
        assert_eq!(battery.percent, Some(0));
        assert_eq!(battery.charging, Some(true));
        frame[8] = 101;
        assert!(parse_battery(&frame).is_none());
        frame[8] = 70;
        frame[1] = 0x82;
        assert!(parse_battery(&frame).is_none());
        assert!(parse_battery(&WIRED_QUERY).is_none());
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
        interface.product_id = 0x024f;
        assert!(!is_control(&interface));
    }
}
