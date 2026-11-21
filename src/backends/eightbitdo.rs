//! 8BitDo Pro 3 D-input mode. Receive-only: no mode, rumble or lighting writes.
use super::*;
use crate::hid::HidDevice;
use std::time::{Duration, Instant};

pub struct EightBitDo;
const REPORT_ID: u8 = 0x04;
const BATTERY_OFFSET: usize = 14;
const UNAVAILABLE: &str = "设备无有效状态回报";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x2dc8
        && i.product_id == 0x6009
        && i.usage_page == 1
        && i.usage == 5
        && (15..=64).contains(&i.input_len)
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    management(i)
        && descriptor::Descriptor::open(api, i)
            .is_some_and(|d| d.input_report_ids(i, i.input_len, &[REPORT_ID]))
}

fn candidate(i: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::EightBitDo, i, 0);
    target.name = "8BitDo Pro 3".into();
    target.family = DeviceFamily::Gamepad;
    target
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces
        .iter()
        .filter(|i| {
            management(i)
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

fn battery(frame: &[u8]) -> Option<Battery> {
    if frame.len() <= BATTERY_OFFSET || frame[0] != REPORT_ID {
        return None;
    }
    let raw = frame[BATTERY_OFFSET];
    let percent = raw & 0x7f;
    if percent > 100 {
        return None;
    }
    Some(Battery {
        percent: Some(percent),
        estimated_percent: None,
        // At exactly 100% the controller is reported as charged, not actively
        // charging, matching SDL's state mapping.
        charging: Some(raw & 0x80 != 0 && percent < 100),
        voltage_mv: None,
        approximate: false,
        level: None,
    })
}

fn receive(mut read: impl FnMut(u32) -> Option<Vec<u8>>) -> Option<(Battery, usize)> {
    let deadline = Instant::now() + Duration::from_millis(400);
    for _ in 0..64 {
        let left = deadline.checked_duration_since(Instant::now())?;
        let frame = read(left.as_millis().max(1) as u32)?;
        if let Some(battery) = battery(&frame) {
            return Some((battery, frame.len()));
        }
    }
    None
}

impl DeviceBackend for EightBitDo {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces)
            .into_iter()
            .filter(|i| proven(api, i))
            .map(candidate)
            .collect())
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let interface = target.hid()?;
        if target.backend != BackendId::EightBitDo || !proven(api, interface) {
            return Err(UNAVAILABLE.into());
        }
        let lock = channel::lock(&channel::collection_group(interface));
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open_read_only(interface)?;
        let started = Instant::now();
        let expected = usize::from(interface.input_len);
        let sample = receive(|timeout| {
            device
                .read_report(timeout)
                .filter(|frame| frame.len() == expected)
        });
        trace::record(
            interface,
            BackendId::EightBitDo,
            "read-input",
            u16::from(REPORT_ID),
            BATTERY_OFFSET as u8,
            usize::from(interface.input_len),
            sample.as_ref().map_or(0, |(_, bytes)| *bytes),
            started,
            if sample.is_some() {
                trace::Outcome::Ok
            } else {
                trace::Outcome::NoReply
            },
        );
        let (battery, _) = sample.ok_or(UNAVAILABLE)?;
        let mut target = candidate(interface);
        target.verified_identity = true;
        Ok(Snapshot {
            target,
            battery: Some(battery),
            battery_at: Some(Instant::now()),
            battery_unavailable: false,
            connected: true,
            dpi: Dpi::default(),
            polling_rate: Default::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(instance: &str) -> HidInterface {
        HidInterface {
            path: format!(r"\\?\hid#vid_2dc8&pid_6009&mi_00#8&{instance}&0000#{{hid}}"),
            vendor_id: 0x2dc8,
            product_id: 0x6009,
            usage_page: 1,
            usage: 5,
            input_len: 34,
            output_len: 0,
            feature_len: 0,
            product: "8BitDo Pro 3".into(),
            serial: "AA:BB:CC:DD:EE:FF".into(),
        }
    }

    fn frame(raw: u8) -> Vec<u8> {
        let mut frame = vec![0; 34];
        frame[0] = REPORT_ID;
        frame[BATTERY_OFFSET] = raw;
        frame
    }

    #[test]
    fn exact_pro3_gamepad_identity_and_receive_only_boundary_are_required() {
        let i = interface("one");
        assert!(management(&i));
        for (vendor, product, page, usage, length) in [
            (0x2dc8, 0x6003, 1, 5, 34),
            (0x2dc8, 0x6009, 1, 6, 34),
            (0x2dc8, 0x6009, 1, 5, 14),
            (0x2dc8, 0x6009, 1, 5, 65),
        ] {
            let mut wrong = i.clone();
            wrong.vendor_id = vendor;
            wrong.product_id = product;
            wrong.usage_page = page;
            wrong.usage = usage;
            wrong.input_len = length;
            assert!(!management(&wrong));
        }
        assert_eq!(candidate(&i).family, DeviceFamily::Gamepad);
        assert_eq!(candidate(&i).backend, BackendId::EightBitDo);
        assert!(!candidate(&i).verified_identity);
    }

    #[test]
    fn published_power_byte_decodes_exact_percent_and_charge_bit() {
        let discharging = battery(&frame(77)).unwrap();
        assert_eq!(
            (discharging.percent, discharging.charging),
            (Some(77), Some(false))
        );
        let charging = battery(&frame(0x80 | 64)).unwrap();
        assert_eq!(
            (charging.percent, charging.charging),
            (Some(64), Some(true))
        );
        let empty = battery(&frame(0)).unwrap();
        assert_eq!((empty.percent, empty.charging), (Some(0), Some(false)));
        let full_on_cable = battery(&frame(0x80 | 100)).unwrap();
        assert_eq!(
            (full_on_cable.percent, full_on_cable.charging),
            (Some(100), Some(false))
        );
    }

    #[test]
    fn wrong_report_short_frame_and_out_of_range_level_never_create_battery() {
        let mut wrong = frame(50);
        wrong[0] = 3;
        assert!(battery(&wrong).is_none());
        for length in 0..=BATTERY_OFFSET {
            assert!(battery(&frame(50)[..length]).is_none());
        }
        for raw in [101, 0x80 | 101, 0x7f] {
            assert!(battery(&frame(raw)).is_none());
        }
    }

    #[test]
    fn receiver_skips_noise_and_stops_on_the_first_valid_power_report() {
        let mut count = 0;
        let sample = receive(|_| {
            count += 1;
            Some(match count {
                1 => vec![0xff; 34],
                2 => {
                    let mut wrong = frame(44);
                    wrong[0] = 1;
                    wrong
                }
                _ => frame(0x80 | 55),
            })
        })
        .unwrap();
        assert_eq!(count, 3);
        assert_eq!(sample.0.percent, Some(55));
        assert_eq!(sample.0.charging, Some(true));
        assert_eq!(sample.1, 34);
        assert!(receive(|_| None).is_none());
    }

    #[test]
    fn identical_models_keep_separate_interface_instances() {
        assert_ne!(
            candidate(&interface("one")).key,
            candidate(&interface("two")).key
        );
    }
}
