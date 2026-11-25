//! Logitech G PRO X 2 LIGHTSPEED receiver battery over report 0x51.
use super::*;
use crate::hid::HidDevice;

pub struct LogitechHeadset;

const VENDOR: u16 = 0x046d;
const PRODUCT: u16 = 0x0af7;
const REPORT: u8 = 0x51;
const FRAME_BYTES: u16 = 64;
const KIND_BATTERY: u8 = 0x0b;
const KIND_POWER: u8 = 0x05;
const TAG_BATTERY: u8 = 0x04;
const TIMEOUT_MS: u32 = 1_500;

pub(super) fn management(i: &HidInterface) -> bool {
    i.vendor_id == VENDOR && i.product_id == PRODUCT
        && i.usage_page == 0xffa0 && i.input_len == FRAME_BYTES
        && i.output_len == FRAME_BYTES
}

fn eligible(api: &HidApi, i: &HidInterface) -> bool {
    management(i)
        && descriptor::Descriptor::open(api, i)
            .is_some_and(|descriptor| descriptor.only_input_output_report(i, REPORT, FRAME_BYTES))
}

fn request() -> [u8; FRAME_BYTES as usize] {
    let mut frame = [0; FRAME_BYTES as usize];
    frame[..10].copy_from_slice(&[REPORT, 0x08, 0, 0x03, 0x1a, 0, 0x03, 0, 0x04, 0x0a]);
    frame
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reading {
    Battery { level: u8, charging: bool },
    PoweredOff,
}

fn decode(frame: &[u8]) -> Option<Reading> {
    if frame.len() < 7 || frame[0] != REPORT { return None; }
    match frame[1] {
        KIND_BATTERY
            if frame.len() >= 13 && frame[8] == TAG_BATTERY && frame[10] <= 100 =>
            Some(Reading::Battery {
                level: frame[10],
                charging: frame[12] == 0x02,
            }),
        KIND_POWER if frame[6] == 0 => Some(Reading::PoweredOff),
        _ => None,
    }
}

fn target(i: &HidInterface) -> Option<Target> {
    if !management(i) { return None; }
    let mut target = Target::new(BackendId::LogitechHeadset, i, 0);
    target.name = "Logitech G PRO X 2 LIGHTSPEED".into();
    target.family = DeviceFamily::Headset;
    target.key = crate::device::stable_key(&format!(
        "logitech-gpro-x2-headset:{}:{}",
        i.serial.trim(),
        i.path
    ));
    target.verified_identity = true;
    Some(target)
}

fn candidates(api: &HidApi, interfaces: &[HidInterface]) -> Vec<Target> {
    interfaces.iter().filter(|i| eligible(api, i)).filter_map(target).collect()
}

fn snapshot(target: &Target, reading: Option<Reading>) -> Snapshot {
    let (battery, battery_unavailable) = match reading {
        Some(Reading::Battery { level, charging }) => (
            Some(Battery {
                percent: Some(level),
                estimated_percent: None,
                charging: Some(charging),
                voltage_mv: None,
                approximate: false,
                level: None,
            }),
            false,
        ),
        Some(Reading::PoweredOff) | None => (None, true),
    };
    Snapshot {
        target: target.clone(),
        battery,
        dpi: Dpi::default(),
        polling_rate: Default::default(),
        connected: true,
        battery_at: Some(std::time::Instant::now()),
        battery_unavailable,
    }
}

fn read_value(api: &HidApi, original: &Target) -> Result<Snapshot, String> {
    let interfaces = crate::hid::enumerate_all_report(api)?.interfaces;
    let interface = interfaces.iter().find(|i|
        eligible(api, i)
            && original.hid().is_ok_and(|source| i.path.eq_ignore_ascii_case(&source.path)))
        .ok_or_else(|| "Logitech 耳机通道未确认".to_string())?;
    let current = target(interface).ok_or_else(|| "Logitech 耳机身份无效".to_string())?;
    if current.key != original.key { return Err("Logitech 耳机身份已变化".into()); }
    let mut device = HidDevice::open(interface)
        .map_err(|_| "Logitech 耳机通道不可写".to_string())?;
    let reply = device
        .transact_with_input_report(&request(), REPORT, TIMEOUT_MS, |frame| decode(frame).is_some())
        .ok_or_else(|| "Logitech 耳机无有效电量回复".to_string())?;
    let reading = decode(&reply).ok_or_else(|| "Logitech 耳机回复无效".to_string())?;
    Ok(snapshot(&current, Some(reading)))
}

impl DeviceBackend for LogitechHeadset {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(api, interfaces))
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        if target.backend != BackendId::LogitechHeadset {
            return Err("Logitech 耳机目标无效".into());
        }
        read_value(api, target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface() -> HidInterface {
        HidInterface {
            path: r"\\?\hid#vid_046d&pid_0af7&mi_03&col02#7&fixture&0&0000#{hid}".into(),
            serial: String::new(),
            product: "G PRO X 2 LIGHTSPEED".into(),
            vendor_id: VENDOR,
            product_id: PRODUCT,
            usage_page: 0xffa0,
            usage: 1,
            input_len: FRAME_BYTES,
            output_len: FRAME_BYTES,
            feature_len: 0,
        }
    }

    fn frame(kind: u8) -> Vec<u8> {
        let mut frame = vec![0; usize::from(FRAME_BYTES)];
        frame[0] = REPORT;
        frame[1] = kind;
        frame
    }

    #[test]
    fn only_the_headset_vendor_collection_is_selected() {
        let exact = interface();
        assert!(management(&exact));
        for mutate in [
            |i: &mut HidInterface| i.product_id = 0x0af6,
            |i: &mut HidInterface| i.usage_page = 0xff00,
            |i: &mut HidInterface| i.input_len = 32,
            |i: &mut HidInterface| i.output_len = 0,
        ] {
            let mut wrong = exact.clone();
            mutate(&mut wrong);
            assert!(!management(&wrong));
        }
    }

    #[test]
    fn request_is_the_exact_fixed_64_byte_battery_query() {
        assert_eq!(
            &request()[..10],
            &[0x51, 0x08, 0x00, 0x03, 0x1a, 0x00, 0x03, 0x00, 0x04, 0x0a]
        );
        assert!(request()[10..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn battery_power_and_ignored_frames_have_distinct_meaning() {
        let mut battery = frame(KIND_BATTERY);
        battery[8] = TAG_BATTERY;
        battery[10] = 75;
        battery[12] = 0x02;
        assert_eq!(decode(&battery), Some(Reading::Battery { level: 75, charging: true }));

        battery[10] = 0;
        battery[12] = 0;
        assert_eq!(decode(&battery), Some(Reading::Battery { level: 0, charging: false }));
        battery[10] = 101;
        assert_eq!(decode(&battery), None);
        battery[10] = 75;
        battery[8] = 3;
        assert_eq!(decode(&battery), None);

        let mut power = frame(KIND_POWER);
        assert_eq!(decode(&power), Some(Reading::PoweredOff));
        power[6] = 1;
        assert_eq!(decode(&power), None);
        assert_eq!(decode(&frame(0x03)), None);
    }

    #[test]
    fn powered_off_or_unknown_frame_clears_but_keeps_the_card() {
        let target = target(&interface()).unwrap();
        for reading in [Some(Reading::PoweredOff), None] {
            let sample = snapshot(&target, reading);
            assert!(sample.connected);
            assert!(sample.battery_unavailable);
            assert!(sample.battery.is_none());
        }
    }
}
