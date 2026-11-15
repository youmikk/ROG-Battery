//! VT1 / VT1 MAX（nRF54L）状态报告，只读电量及当前 DPI。
use super::*;
use crate::hid::HidDevice;

pub struct Rapoo;
const VID: u16 = 0x24ae;
const STATUS_REPORT: u8 = 7;

fn model(pid: u16) -> Option<&'static str> {
    match pid {
        0x4615 | 0x1415 => Some("Rapoo VT1"),
        0x4644 | 0x1444 => Some("Rapoo VT1 MAX"),
        _ => None,
    }
}

fn is_control(interface: &HidInterface) -> bool {
    interface.vendor_id == VID
        && model(interface.product_id).is_some()
        && interface.usage_page == 0xff00
        && interface.usage == 2
        && (9..=65).contains(&interface.input_len)
}

fn parse_status(frame: &[u8]) -> Option<(Battery, Dpi)> {
    // 电量和 X/Y DPI 需要完整的前八个数据字节；配置索引属于可选扩展。
    if frame.len() < 9 || frame[0] != STATUS_REPORT {
        return None;
    }
    let data = &frame[1..];
    let charging = match data[6] {
        1 => false,
        2 | 3 => true,
        _ => return None,
    };
    if data[7] > 100 || data[1] > 6 || data.get(11).is_some_and(|index| *index > 7) {
        return None;
    }
    let x = u16::from_le_bytes([data[2], data[3]]);
    let y = u16::from_le_bytes([data[4], data[5]]);
    // 公共 DPI 控件尚不能表达独立 X/Y，保留未知，避免显示不完整的设置。
    let current = (x == y && x >= 50 && x <= 45000 && x % 50 == 0).then_some(x);
    Some((
        Battery {
            percent: Some(data[7]),
            charging: Some(charging),
            voltage_mv: None,
            approximate: false,
            level: None,
        },
        Dpi {
            current,
            values: Vec::new(),
            writable: false,
        },
    ))
}

impl DeviceBackend for Rapoo {
    fn discover(&self, _: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(interfaces
            .iter()
            .filter(|interface| is_control(interface))
            .map(|interface| {
                let mut target = Target::new(BackendId::Rapoo, interface, 0);
                target.name = model(interface.product_id).unwrap().into();
                target.family = DeviceFamily::Mouse;
                target
            })
            .collect())
    }

    fn read(&self, _: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        if !is_control(&target.interface) {
            return Err(crate::tr!("此雷柏型号尚未适配").into());
        }
        let started = std::time::Instant::now();
        let mut reply = crate::hid::query_report(&target.interface, STATUS_REPORT, false, 120);
        if reply
            .as_ref()
            .and_then(|frame| parse_status(frame))
            .is_none()
        {
            let mut device = match HidDevice::open(&target.interface) {
                Ok(device) => device,
                Err(error) => {
                    trace::record(
                        &target.interface,
                        BackendId::Rapoo,
                        "open-status",
                        0,
                        STATUS_REPORT,
                        0,
                        0,
                        started,
                        trace::Outcome::NoReply,
                    );
                    return Err(error);
                }
            };
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(180);
            while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
                let Some(frame) = device.read_report(remaining.as_millis().max(1) as u32) else {
                    break;
                };
                if parse_status(&frame).is_some() {
                    reply = Some(frame);
                    break;
                }
            }
        }
        let parsed = reply.as_ref().and_then(|frame| parse_status(frame));
        trace::record(
            &target.interface,
            BackendId::Rapoo,
            "read-status",
            0,
            STATUS_REPORT,
            0,
            reply.as_ref().map(Vec::len).unwrap_or(0),
            started,
            if parsed.is_some() {
                trace::Outcome::Ok
            } else if reply.is_some() {
                trace::Outcome::InvalidData
            } else {
                trace::Outcome::NoReply
            },
        );
        let (battery, dpi) =
            parsed.ok_or_else(|| crate::tr!("雷柏状态报告不可用，请唤醒设备后重试"))?;
        Ok(Snapshot {
            target: target.clone(),
            battery: Some(battery),
            dpi,
            connected: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn report() -> Vec<u8> {
        vec![7, 0, 1, 0x20, 3, 0x20, 3, 1, 75, 0, 0, 0, 0]
    }
    #[test]
    fn status_has_real_percentage_and_read_only_dpi() {
        let (battery, dpi) = parse_status(&report()).unwrap();
        assert_eq!(battery.percent, Some(75));
        assert_eq!(battery.charging, Some(false));
        assert_eq!(dpi.current, Some(800));
        assert!(!dpi.writable);
        assert_eq!(parse_status(&report()[..9]).unwrap().1.current, Some(800));
        let mut frame = report();
        frame[7] = 3;
        frame[8] = 0;
        assert_eq!(parse_status(&frame).unwrap().0.percent, Some(0));
        assert_eq!(parse_status(&frame).unwrap().0.charging, Some(true));
    }
    #[test]
    fn offline_truncated_and_invalid_reports_are_rejected() {
        let mut frame = report();
        assert!(parse_status(&frame[..8]).is_none());
        frame[7] = 0;
        assert!(parse_status(&frame).is_none());
        frame[7] = 1;
        frame[8] = 101;
        assert!(parse_status(&frame).is_none());
        frame[8] = 75;
        frame[12] = 8;
        assert!(parse_status(&frame).is_none());
    }
    #[test]
    fn independent_axes_do_not_claim_a_single_dpi() {
        let mut frame = report();
        frame[5] = 0x40;
        assert_eq!(parse_status(&frame).unwrap().1.current, None);
    }
}
