//! AJAZZ AK980 PRO wired and 2.4G GET_DEVICE_INFO battery support.
use super::*;
use crate::hid::HidDevice;

pub struct AjazzAk980;

const VENDOR: u16 = 0x0c45;
const MODEL_PID: u16 = 0x8039;
const REPORT_BYTES: u16 = 65;
const HEADER_BYTES: usize = 8;
const INFO_BYTES: u8 = 48;
const MIN_INFO_BYTES: usize = 19;
const COMMAND_INFO: u8 = 0x10;
const TIMEOUT_MS: u32 = 1_200;

const WIRED_PID: u16 = 0x8039;
const WIRELESS_PID: u16 = 0xfefe;

fn link(i: &HidInterface) -> Option<(u16, u8)> {
    let path = i.path.to_ascii_lowercase();
    match (i.vendor_id, i.product_id) {
        (VENDOR, WIRED_PID)
            if i.product == "AK980 PRO" && i.usage_page == 0xff68 && i.usage == 0x61
                && path.contains("vid_0c45&pid_8039&mi_02") =>
            Some((MODEL_PID, 0)),
        (VENDOR, WIRELESS_PID)
            if i.product == "AJAZZ AK980 PRO 2.4G"
                && i.usage_page == 0xff60 && i.usage == 0x61
                && path.contains("vid_0c45&pid_fefe&mi_03") =>
            Some((MODEL_PID, 2)),
        _ => None,
    }
}

fn eligible(api: &HidApi, i: &HidInterface) -> bool {
    link(i).is_some() && i.input_len == REPORT_BYTES && i.output_len == REPORT_BYTES
        && descriptor::Descriptor::open(api, i)
            .is_some_and(|descriptor| descriptor.only_input_output_report(i, 0, REPORT_BYTES))
}

fn request() -> [u8; REPORT_BYTES as usize] {
    let mut frame = [0; REPORT_BYTES as usize];
    frame[1] = 0xaa;
    frame[2] = COMMAND_INFO;
    frame[3] = INFO_BYTES;
    frame[7] = 1;
    frame
}

fn payload(frame: &[u8]) -> Option<&[u8]> {
    let declared = usize::from(*frame.get(2)?);
    if frame.len() < HEADER_BYTES + MIN_INFO_BYTES
        || frame[0] != 0x55 || frame[1] != COMMAND_INFO
        || !(MIN_INFO_BYTES..=usize::from(INFO_BYTES)).contains(&declared)
        || frame.len() < HEADER_BYTES + declared {
        return None;
    }
    Some(&frame[HEADER_BYTES..HEADER_BYTES + declared])
}

fn decode(frame: &[u8], expected_mode: u8) -> Option<u8> {
    let payload = payload(frame)?;
    let echo_vid = u16::from_le_bytes(payload.get(4..6)?.try_into().ok()?);
    let echo_pid = u16::from_le_bytes(payload.get(6..8)?.try_into().ok()?);
    let mode = *payload.get(16)?;
    let level = *payload.get(17)?;
    (echo_vid == VENDOR && echo_pid == MODEL_PID && mode == expected_mode && level <= 100)
        .then_some(level)
}

fn target(i: &HidInterface) -> Option<Target> {
    let _ = link(i)?;
    let mut target = Target::new(BackendId::AjazzAk980, i, 0);
    target.name = "AJAZZ AK980 PRO".into();
    target.family = DeviceFamily::Keyboard;
    target.key = crate::device::stable_key(&format!(
        "ajazz-ak980:{}:{}",
        i.product_id,
        i.serial.trim()
    ));
    Some(target)
}

fn candidates(api: &HidApi, interfaces: &[HidInterface]) -> Vec<Target> {
    interfaces.iter().filter(|i| eligible(api, i)).filter_map(target).collect()
}

fn read_value(api: &HidApi, original: &Target) -> Result<Snapshot, String> {
    let interfaces = crate::hid::enumerate_all_report(api)?.interfaces;
    let interface = interfaces
        .iter()
        .find(|i| {
            eligible(api, i)
                && original.hid().is_ok_and(|target| i.path.eq_ignore_ascii_case(&target.path))
        })
        .ok_or_else(|| "AK980 PRO 命令通道未确认".to_string())?;
    let (_, expected_mode) = link(interface).ok_or_else(|| "AK980 PRO 连接类型无效".to_string())?;
    let mut device = HidDevice::open(interface).map_err(|_| "AK980 PRO 命令通道不可写".to_string())?;
    let reply = device
        .transact_with_input_report(&request(), 0, TIMEOUT_MS, |frame| {
            decode(frame, expected_mode).is_some()
        })
        .ok_or_else(|| "AK980 PRO 无有效设备信息回复".to_string())?;
    let level = decode(&reply, expected_mode).ok_or_else(|| "AK980 PRO 回复无效".to_string())?;
    let target = target(interface).ok_or_else(|| "AK980 PRO 身份已变化".to_string())?;
    if target.key != original.key { return Err("AK980 PRO 身份已变化".into()); }
    Ok(Snapshot {
        target,
        battery: Some(Battery {
            percent: Some(level),
            estimated_percent: None,
            charging: None,
            voltage_mv: None,
            approximate: false,
            level: None,
        }),
        dpi: Dpi::default(),
        polling_rate: Default::default(),
        connected: true,
        battery_at: Some(std::time::Instant::now()),
        battery_unavailable: false,
    })
}

impl DeviceBackend for AjazzAk980 {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(api, interfaces))
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        if target.backend != BackendId::AjazzAk980 { return Err("AK980 PRO 目标无效".into()); }
        read_value(api, target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(pid: u16, usage_page: u16, product: &str, mi: u8) -> HidInterface {
        HidInterface {
            path: format!(
                r"\\?\hid#vid_0c45&pid_{pid:04x}&mi_{mi:02}&col01#7&fixture&0&0000#{{hid}}"
            ),
            serial: String::new(),
            product: product.into(),
            vendor_id: VENDOR,
            product_id: pid,
            usage_page,
            usage: 0x61,
            input_len: REPORT_BYTES,
            output_len: REPORT_BYTES,
            feature_len: 0,
        }
    }

    fn reply(mode: u8, level: u8) -> Vec<u8> {
        let mut frame = vec![0; 64];
        frame[..8].copy_from_slice(&[0x55, 0x10, 0x30, 0, 0, 0, 1, 1]);
        frame[12..14].copy_from_slice(&VENDOR.to_le_bytes());
        frame[14..16].copy_from_slice(&MODEL_PID.to_le_bytes());
        frame[24] = mode;
        frame[25] = level;
        frame
    }

    #[test]
    fn only_reviewed_wired_and_wireless_collections_are_accepted() {
        let wired = interface(WIRED_PID, 0xff68, "AK980 PRO", 2);
        let wireless = interface(WIRELESS_PID, 0xff60, "AJAZZ AK980 PRO 2.4G", 3);
        assert_eq!(link(&wired), Some((MODEL_PID, 0)));
        assert_eq!(link(&wireless), Some((MODEL_PID, 2)));
        for wrong in [
            interface(WIRED_PID, 0xff67, "AK980 PRO", 3),
            interface(WIRED_PID, 0xff68, "AK980", 2),
            interface(WIRELESS_PID, 0xff60, "AJAZZ AK980 PRO 2.4G", 4),
            interface(WIRELESS_PID, 0xff60, "Other keyboard", 3),
        ] {
            assert_eq!(link(&wrong), None);
        }
    }

    #[test]
    fn request_matches_the_verified_65_byte_device_info_query() {
        let mut expected = [0; 65];
        expected[1] = 0xaa;
        expected[2] = 0x10;
        expected[3] = 0x30;
        expected[7] = 1;
        assert_eq!(request(), expected);
        assert_eq!(request().len(), usize::from(REPORT_BYTES));
    }

    #[test]
    fn replay_requires_command_mode_echoed_model_and_bounded_percentage() {
        for (mode, level) in [(0, 0), (0, 100), (2, 0), (2, 75), (2, 100)] {
            assert_eq!(decode(&reply(mode, level), mode), Some(level));
        }
        assert!(decode(&reply(2, 75), 0).is_none());
        assert!(decode(&reply(0, 101), 0).is_none());
        let mut wrong_command = reply(2, 75);
        wrong_command[1] = 0x11;
        assert!(decode(&wrong_command, 2).is_none());
        let mut wrong_model = reply(2, 75);
        wrong_model[14] = 0x34;
        assert!(decode(&wrong_model, 2).is_none());
        let mut short = reply(2, 75);
        short.truncate(26);
        assert!(decode(&short, 2).is_none());
    }
}
