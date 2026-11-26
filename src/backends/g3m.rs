//! HECATE G3M PRO battery only. No configuration session or DPI writes.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct G3m;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x320f && i.product_id == 0x706e && i.usage_page == 0xff1c && i.usage == 0x0092
}

fn control(i: &HidInterface) -> bool {
    management(i) && i.input_len == 64 && i.output_len == 64
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    control(i)
        && descriptor::Descriptor::open(api, i)
            .is_some_and(|d| d.only_input_output_report(i, 4, 64))
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces
        .iter()
        .filter(|i| {
            control(i)
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

fn candidate(i: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::G3m, i, 0);
    target.name = "HECATE G3M PRO".into();
    target.family = DeviceFamily::Mouse;
    target
}

fn request() -> [u8; 64] {
    let mut packet = [0; 64];
    packet[..5].copy_from_slice(&[0x04, 0x20, 0x00, 0x1a, 0x06]);
    packet
}

fn reply(frame: &[u8]) -> Option<Battery> {
    if frame.len() != 64
        || frame[0] != 0x04
        || frame[3] != 0x1a
        || matches!(frame[7], 0xfe | 0xff)
        || frame[8] > 100
        || frame == request()
    {
        return None;
    }
    Some(Battery {
        percent: Some(frame[8]),
        estimated_percent: None,
        charging: None,
        voltage_mv: None,
        approximate: false,
        level: None,
    })
}

impl DeviceBackend for G3m {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces)
            .into_iter()
            .filter(|i| proven(api, i))
            .map(candidate)
            .collect())
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let interface = target.hid()?;
        if target.backend != BackendId::G3m || !proven(api, interface) {
            return Err(UNAVAILABLE.into());
        }
        let lock = channel::lock(&channel::collection_group(interface));
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(interface)?;
        let started = Instant::now();
        let response = device
            .transact(&request(), 1000, |r| reply(r).is_some())
            .and_then(|r| reply(&r));
        trace::record(
            interface,
            BackendId::G3m,
            "query-battery",
            4,
            0x1a,
            64,
            if response.is_some() { 64 } else { 0 },
            started,
            if response.is_some() {
                trace::Outcome::Ok
            } else {
                trace::Outcome::NoReply
            },
        );
        let battery = response.ok_or(UNAVAILABLE)?;
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
            path: format!(r"\\?\hid#vid_320f&pid_706e&mi_01&col05#8&{instance}&0005#{{hid}}"),
            vendor_id: 0x320f,
            product_id: 0x706e,
            usage_page: 0xff1c,
            usage: 0x0092,
            input_len: 64,
            output_len: 64,
            feature_len: 0,
            product: "HECATE G3M PRO".into(),
            serial: "shared".into(),
        }
    }

    fn frame(percent: u8, status: u8) -> Vec<u8> {
        let mut frame = vec![0xa5; 64];
        frame[..4].copy_from_slice(&[0x04, 0x20, 0x00, 0x1a]);
        frame[7] = status;
        frame[8] = percent;
        frame
    }

    #[test]
    fn exact_product_collection_report_and_capacity_are_required() {
        let i = interface("one");
        assert!(management(&i) && control(&i));
        for (vendor, product, page, usage, input, output) in [
            (0x320f, 0x5088, 0xff1c, 0x0092, 64, 64),
            (0x320f, 0x706e, 0xff00, 0x0092, 64, 64),
            (0x320f, 0x706e, 0xff1c, 0x0092, 63, 64),
            (0x320f, 0x706e, 0xff1c, 0x0092, 64, 65),
        ] {
            let mut wrong = i.clone();
            wrong.vendor_id = vendor;
            wrong.product_id = product;
            wrong.usage_page = page;
            wrong.usage = usage;
            wrong.input_len = input;
            wrong.output_len = output;
            assert!(!control(&wrong));
        }
        assert_eq!(candidate(&i).family, DeviceFamily::Mouse);
    }

    #[test]
    fn request_contains_documented_redundant_checksum() {
        let packet = request();
        assert_eq!(&packet[..5], &[0x04, 0x20, 0x00, 0x1a, 0x06]);
        assert_eq!(
            packet[3..]
                .iter()
                .fold(0u8, |sum, byte| sum.wrapping_add(*byte)),
            0x20
        );
        assert!(packet[5..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn reply_accepts_valid_levels_and_unknown_charging_only() {
        for percent in [0, 1, 50, 100] {
            let battery = reply(&frame(percent, 0)).unwrap();
            assert_eq!(battery.percent, Some(percent));
            assert_eq!(battery.charging, None);
        }
        for status in [0xfe, 0xff] {
            assert!(reply(&frame(50, status)).is_none());
        }
        assert!(reply(&frame(101, 0)).is_none());
    }

    #[test]
    fn echo_wrong_id_command_and_truncation_are_rejected() {
        assert!(reply(&request()).is_none());
        for index in [0, 3] {
            let mut wrong = frame(50, 0);
            wrong[index] ^= 1;
            assert!(reply(&wrong).is_none());
        }
        for length in 0..64 {
            assert!(reply(&frame(50, 0)[..length]).is_none());
        }
    }

    #[test]
    fn identical_receivers_keep_separate_interface_instances() {
        let first = candidate(&interface("one"));
        let second = candidate(&interface("two"));
        assert_ne!(first.key, second.key);
        assert!(!first.same_channel(&second));
    }
}
