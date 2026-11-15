//! nRF54L 鼠标状态报告，只读电量及当前 DPI。
use super::*;
use crate::hid::HidDevice;

pub struct Rapoo;
const VID: u16 = 0x24ae;
const STATUS_REPORT: u8 = 7;

// 型号与协议归属：Rapoo AHUB 官方 VT_nrf54L 清单。
fn model(pid: u16) -> Option<&'static str> {
    match pid {
        0x464d => Some("Rapoo ESM612ACE"),
        0x4681 => Some("Rapoo ESM612MAX"),
        0x4631 => Some("Rapoo ESM612PRO"),
        0x4637 => Some("Rapoo ESM622MAX"),
        0x4632 => Some("Rapoo ESM622PRO"),
        0x4633 => Some("Rapoo ESM652PRO"),
        0x4640 => Some("Rapoo ESM672MAX"),
        0x4635 => Some("Rapoo ESM672PRO"),
        0x4610 | 0x1410 => Some("Rapoo VT0"),
        0x46a1 | 0x14a1 => Some("Rapoo VT0 Air MAX"),
        0x4620 | 0x1420 => Some("Rapoo VT0 MAX"),
        0x464a | 0x144a => Some("Rapoo VT0 MAX MASTER"),
        0x4615 | 0x1415 => Some("Rapoo VT1"),
        0x4644 | 0x1444 => Some("Rapoo VT1 MAX"),
        0x4b0a | 0x1b0a => Some("Rapoo VT1S MAX V2"),
        0x466a | 0x146a => Some("Rapoo VT1S V2"),
        0x4622 | 0x1422 => Some("Rapoo VT2"),
        0x4b12 | 0x1b12 => Some("Rapoo VT2 Air MAX V2"),
        0x4623 | 0x1423 => Some("Rapoo VT2 MAX"),
        0x4675 | 0x1475 => Some("Rapoo VT2 MAX MASTER V2"),
        0x4b09 | 0x1b09 => Some("Rapoo VT2 MAX V2"),
        0x4668 | 0x1468 => Some("Rapoo VT2 V2"),
        0x46a4 | 0x14a4 => Some("Rapoo VT2K MAX"),
        0x4b13 | 0x1b13 => Some("Rapoo VT2S Air MAX V2"),
        0x4676 | 0x1476 => Some("Rapoo VT2S MAX MASTER V2"),
        0x4b04 | 0x1b04 => Some("Rapoo VT2S MAX V2"),
        0x465d | 0x145d => Some("Rapoo VT2S V2"),
        0x4612 | 0x1412 => Some("Rapoo VT3"),
        0x4b0c | 0x1b0c => Some("Rapoo VT3 Air MAX V2"),
        0x4617 | 0x1417 => Some("Rapoo VT3 MAX"),
        0x466d | 0x146d => Some("Rapoo VT3 MAX MASTER V2"),
        0x4b06 | 0x1b06 => Some("Rapoo VT3 MAX V2"),
        0x46a5 | 0x14a5 => Some("Rapoo VT3 PRO V2"),
        0x4662 | 0x1462 => Some("Rapoo VT3 V2"),
        0x4611 | 0x1411 => Some("Rapoo VT3S"),
        0x46a2 | 0x14a2 => Some("Rapoo VT3S Air Max"),
        0x4b0d | 0x1b0d => Some("Rapoo VT3S Air MAX V2"),
        0x4621 | 0x1421 => Some("Rapoo VT3S MAX"),
        0x464b | 0x144b => Some("Rapoo VT3S MAX MASTER"),
        0x466e | 0x146e => Some("Rapoo VT3S MAX MASTER V2"),
        0x4b07 | 0x1b07 => Some("Rapoo VT3S MAX V2"),
        0x46a6 | 0x14a6 => Some("Rapoo VT3S PRO V2"),
        0x4664 | 0x1464 => Some("Rapoo VT3S V2"),
        0x4b14 | 0x1b14 => Some("Rapoo VT4 Air MAX V2"),
        0x4677 | 0x1477 => Some("Rapoo VT4 MAX MASTER V2"),
        0x4b03 | 0x1b03 => Some("Rapoo VT4 MAX V2"),
        0x465b | 0x145b => Some("Rapoo VT4 V2"),
        0x4651 | 0x1451 => Some("Rapoo VT6"),
        0x4652 | 0x1452 => Some("Rapoo VT6 MAX"),
        0x4613 | 0x1413 => Some("Rapoo VT7"),
        0x4b0e | 0x1b0e => Some("Rapoo VT7 Air MAX V2"),
        0x4616 | 0x1416 => Some("Rapoo VT7 MAX"),
        0x4670 | 0x1470 => Some("Rapoo VT7 MAX MASTER V2"),
        0x4b05 | 0x1b05 => Some("Rapoo VT7 MAX V2"),
        0x46a7 | 0x14a7 => Some("Rapoo VT7 PRO V2"),
        0x4660 | 0x1460 => Some("Rapoo VT7 V2"),
        0x4b0f | 0x1b0f => Some("Rapoo VT7S Air MAX V2"),
        0x4671 | 0x1471 => Some("Rapoo VT7S MAX MASTER V2"),
        0x4b02 | 0x1b02 => Some("Rapoo VT7S MAX V2"),
        0x46a8 | 0x14a8 => Some("Rapoo VT7S PRO V2"),
        0x4659 | 0x1459 => Some("Rapoo VT7S V2"),
        0x4b15 | 0x1b15 => Some("Rapoo VT8 Air MAX V2"),
        0x4678 | 0x1478 => Some("Rapoo VT8 MAX MASTER V2"),
        0x4b0b | 0x1b0b => Some("Rapoo VT8 MAX V2"),
        0x466c | 0x146c => Some("Rapoo VT8 V2"),
        0x4614 | 0x1414 => Some("Rapoo VT9"),
        0x4b10 | 0x1b10 => Some("Rapoo VT9 Air MAX V2"),
        0x4618 | 0x1418 => Some("Rapoo VT9 MAX"),
        0x4673 | 0x1473 => Some("Rapoo VT9 MAX MASTER V2"),
        0x4b08 | 0x1b08 => Some("Rapoo VT9 MAX V2"),
        0x46a9 | 0x14a9 => Some("Rapoo VT9 PRO V2"),
        0x4666 | 0x1466 => Some("Rapoo VT9 V2"),
        0x4b11 | 0x1b11 => Some("Rapoo VT9S Air MAX V2"),
        0x4674 | 0x1474 => Some("Rapoo VT9S MAX MASTER V2"),
        0x4b01 | 0x1b01 => Some("Rapoo VT9S MAX V2"),
        0x46aa | 0x14aa => Some("Rapoo VT9S PRO V2"),
        0x4653 | 0x1453 => Some("Rapoo VT9S V2"),
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
            ..Default::default()
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
    #[test]
    fn expanded_catalog_requires_the_nrf54l_status_interface() {
        let mut interface = HidInterface {
            vendor_id: VID,
            product_id: 0x4b01,
            usage_page: 0xff00,
            usage: 2,
            input_len: 19,
            output_len: 0,
            feature_len: 0,
            path: String::new(),
            serial: String::new(),
            product: String::new(),
        };
        assert_eq!(model(interface.product_id), Some("Rapoo VT9S MAX V2"));
        assert!(is_control(&interface));
        interface.product_id = 0x1b01;
        assert!(is_control(&interface));
        interface.usage = 14;
        assert!(!is_control(&interface));
        interface.usage = 2;
        interface.product_id = 0x0001;
        assert!(!is_control(&interface));
        interface.product_id = 0x4b01;
        interface.input_len = 8;
        assert!(!is_control(&interface));
    }
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
