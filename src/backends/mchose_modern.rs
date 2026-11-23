//! MCHOSE 4D mouse receiver status, independent of Feature mice and QHW keyboards.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct MchoseModern;
const REPORT: u8 = 0x4d;
const LENGTH: u16 = 64;
const UNAVAILABLE: &str = "设备无有效状态回报";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x3837 && matches!(i.product_id, 0x1014 | 0x1016 | 0x1018)
        && i.usage_page == 0xff01 && i.usage == 1
}

fn control(i: &HidInterface) -> bool {
    management(i) && i.input_len == LENGTH && i.output_len == LENGTH
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|i| control(i) && interfaces.iter().filter(|other|
        management(other) && channel::collection_group(i) == channel::collection_group(other)
    ).count() == 1).collect()
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    control(i) && descriptor::Descriptor::open(api, i)
        .is_some_and(|d| d.input_output_byte_reports(i, REPORT, LENGTH, LENGTH)
            || windows_array_variant(i) && d.initialized_input_output_report(i, REPORT, LENGTH))
}

fn windows_array_variant(i: &HidInterface) -> bool {
    control(i) && i.product_id == 0x1018
        && matches!(i.product.trim(), "MCHOSE A7 V3 Ultra+" | "MCHOSE A7 V3 Pro")
        && i.path.to_ascii_lowercase().split('#').nth(1)
            .is_some_and(|hardware| hardware.split('&').any(|field| field == "mi_02"))
}

fn target(i: &HidInterface, interfaces: &[HidInterface]) -> Target {
    let mut t = Target::new(BackendId::MchoseModern, i, 0);
    let name = std::iter::once(i).chain(interfaces.iter().filter(|other|
        channel::collection_group(i) == channel::collection_group(other)))
        .map(|i| i.product.trim())
        .find(|name| name.to_ascii_uppercase().starts_with("MCHOSE "));
    // Shared receiver and reply PIDs cannot identify the paired retail model.
    t.name = name.unwrap_or("MCHOSE Mouse").into();
    t.family = DeviceFamily::Mouse;
    t
}

fn request() -> [u8; LENGTH as usize] {
    let mut p = [0; LENGTH as usize];
    p[..9].copy_from_slice(&[REPORT, 1, 1, 0, 0, 9, 0, 0, 8]);
    p
}

#[derive(Debug, PartialEq, Eq)]
enum Reply { Battery { percent: u8, charging: Option<bool> }, Offline }

fn reply(r: &[u8]) -> Option<Reply> {
    if r.len() != usize::from(LENGTH) || r[..2] != [REPORT, 1] || r[2] > 1
        || r[4..8] != [0, 9, 0, 0] { return None; }
    let length = usize::from(r[3]);
    let end = 8 + length;
    if length < 14 || end + usize::from(r[2] == 1) > r.len() { return None; }
    if r[2] == 1 && r[end] != r[2..end].iter().fold(0, |xor, b| xor ^ b) { return None; }
    let data = &r[8..end];
    if data[..2] != [0x37, 0x38] || data[12] > 100 { return None; }
    match data[10] {
        0 => Some(Reply::Offline),
        1 | 2 => Some(Reply::Battery { percent: data[12], charging: match data[11] {
            0 => Some(false), 1 => Some(true), _ => None,
        }}),
        _ => None,
    }
}

fn read_with(t: &Target, exchange: impl FnOnce(&[u8]) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    if t.backend != BackendId::MchoseModern || !control(t.hid()?) { return Err(UNAVAILABLE.into()); }
    let response = exchange(&request()).ok_or(UNAVAILABLE)?;
    let state = reply(&response).ok_or(UNAVAILABLE)?;
    let battery = match state {
        Reply::Battery { percent, charging } => Some(Battery { percent: Some(percent), charging,
            estimated_percent: None, voltage_mv: None, approximate: false, level: None }),
        Reply::Offline => None,
    };
    let mut target = t.clone();
    target.verified_identity = true;
    Ok(Snapshot { target, battery_at: battery.as_ref().map(|_| Instant::now()), battery,
        battery_unavailable: state == Reply::Offline, connected: state != Reply::Offline,
        dpi: Dpi::default(), polling_rate: Default::default() })
}

impl DeviceBackend for MchoseModern {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i))
            .map(|i| target(i, interfaces)).collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        let i = t.hid()?;
        if t.backend != BackendId::MchoseModern || !proven(api, i) { return Err(UNAVAILABLE.into()); }
        let lock = channel::lock(&channel::collection_group(i));
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(i)?;
        read_with(t, |request| {
            let started = Instant::now();
            let mut remaining = 32;
            let response = device.transact(request, 1500, |r| {
                remaining -= 1;
                reply(r).is_some() || remaining == 0
            });
            trace::record(i, BackendId::MchoseModern, "query-battery", 0x0900, REPORT,
                request.len(), response.as_ref().map_or(0, Vec::len), started, match response.as_deref() {
                    Some(r) if reply(r).is_some() => trace::Outcome::Ok,
                    Some(_) => trace::Outcome::InvalidData,
                    None => trace::Outcome::NoReply,
                });
            response
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(pid: u16, instance: &str) -> HidInterface {
        HidInterface { path: format!(r"\\?\hid#vid_3837&pid_{pid:04x}&mi_02&col01#8&{instance}&0001#{{hid}}"),
            vendor_id: 0x3837, product_id: pid, usage_page: 0xff01, usage: 1,
            input_len: LENGTH, output_len: LENGTH, feature_len: 0,
            product: "USB Receiver".into(), serial: "shared".into() }
    }

    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut r = request().to_vec();
        r[3] = payload.len() as u8;
        r[8..8 + payload.len()].copy_from_slice(payload);
        checksum(&mut r);
        r
    }

    fn checksum(r: &mut [u8]) {
        let end = 8 + usize::from(r[3]);
        r[end] = r[2..end].iter().fold(0, |xor, b| xor ^ b);
    }

    fn captured_payload() -> Vec<u8> {
        // Public Ultra+ payload; framing and zero padding are reconstructed.
        vec![0x37, 0x38, 0x26, 0x40, 4, 0, 0, 0, 0, 0x10, 2, 1, 85, 0, 8, 0xe4]
    }

    #[test]
    fn device_info_payload_preserves_percentage_charge_and_does_not_infer_model_from_pid() {
        let r = frame(&captured_payload());
        assert_eq!(r[24], 0xdf);
        assert_eq!(reply(&r), Some(Reply::Battery { percent: 85, charging: Some(true) }));
        let mut data = captured_payload(); data[11] = 0; data[12] = 42;
        let r = frame(&data);
        assert_eq!(r[24], 0xa1);
        assert_eq!(reply(&r), Some(Reply::Battery { percent: 42, charging: Some(false) }));
        let mut i = interface(0x1014, "one");
        i.product = "MCHOSE K7 V2 Ultra+".into();
        let sample = read_with(&target(&i, &[]), |_| Some(r)).unwrap();
        assert_eq!(sample.target.name, i.product);
        assert!(sample.dpi.stages.is_empty() && !sample.dpi.writable);
        assert_eq!(target(&interface(0x1014, "one"), &[]).name, "MCHOSE Mouse");
    }

    #[test]
    fn receiver_generations_and_keyboard_protocols_are_isolated() {
        for pid in [0x1014, 0x1016, 0x1018] {
            let i = interface(pid, "one"); assert!(control(&i));
            let other = interface(pid, "two");
            assert_ne!(target(&i, &[]).key, target(&other, &[]).key);
            assert_eq!(candidates(&[i.clone(), other]).len(), 2);
            let mut sibling = i.clone();
            sibling.path = sibling.path.replace("col01", "col02").replace("&0001#", "&0002#");
            assert!(candidates(&[i, sibling]).is_empty());
        }
        for pid in [0x100b, 0x1020, 0x1015, 0x3033, 0x600a, 0x4026] {
            assert!(!control(&interface(pid, "one")));
        }
        for (page, usage, input, output) in [(0xff0b, 0xc4, 64, 64), (0xff01, 1, 41, 41),
            (0xff01, 1, 65, 65), (0xff80, 1, 64, 64), (1, 2, 64, 64)] {
            let mut i = interface(0x1018, "one");
            i.usage_page = page; i.usage = usage; i.input_len = input; i.output_len = output;
            assert!(!control(&i));
        }
    }

    #[test]
    fn windows_array_exception_is_limited_to_documented_pro_and_ultra_receiver() {
        let mut i = interface(0x1018, "one");
        for name in ["MCHOSE A7 V3 Ultra+", "MCHOSE A7 V3 Pro"] {
            i.product = name.into(); assert!(windows_array_variant(&i));
        }
        i.product = "MCHOSE K7 V2 Ultra+".into(); assert!(!windows_array_variant(&i));
        i.product = "MCHOSE A7 V3 Ultra+".into();
        i.product_id = 0x1014; assert!(!windows_array_variant(&i));
        i.product_id = 0x1018;
        i.path = i.path.replace("mi_02", "mi_01"); assert!(!windows_array_variant(&i));
        i.path = i.path.replace("mi_01", "mi_02"); i.input_len = 41;
        assert!(!windows_array_variant(&i));
    }

    #[test]
    fn malformed_busy_echo_and_other_commands_cannot_publish_battery() {
        let valid = frame(&captured_payload());
        assert!(reply(&request()).is_none());
        assert!(reply(&frame(&[0xff])).is_none());
        for length in 0..64 { assert!(reply(&valid[..length]).is_none()); }
        for (offset, value) in [(0, 0x11), (1, 2), (2, 3), (3, 13), (3, 56),
            (4, 4), (5, 0), (6, 1), (7, 1), (8, 0x36), (18, 3), (20, 101), (24, 0)] {
            let mut r = valid.clone(); r[offset] = value;
            assert!(reply(&r).is_none(), "offset {offset}");
        }
        for (offset, value) in [(8, 0x36), (18, 0xff), (20, 101)] {
            let mut r = valid.clone(); r[offset] = value; checksum(&mut r);
            assert!(reply(&r).is_none());
        }
        let mut no_checksum = valid.clone(); no_checksum[2] = 0;
        assert!(reply(&no_checksum).is_some());
        for level in 0..=100 {
            let mut r = valid.clone(); r[20] = level; checksum(&mut r);
            assert_eq!(reply(&r), Some(Reply::Battery { percent: level, charging: Some(true) }));
        }
        let mut unknown_charge = valid; unknown_charge[19] = 0xff; checksum(&mut unknown_charge);
        assert_eq!(reply(&unknown_charge), Some(Reply::Battery { percent: 85, charging: None }));
    }

    #[test]
    fn offline_clears_frozen_level_and_background_wake_recovers_new_sample() {
        let t = target(&interface(0x1018, "one"), &[]);
        let mut data = captured_payload(); data[10] = 0;
        let offline = read_with(&t, |_| Some(frame(&data))).unwrap();
        assert!(!offline.connected && offline.battery_unavailable);
        assert!(offline.battery.is_none() && offline.battery_at.is_none());
        assert!(read_with(&t, |_| None).is_err());
        data[10] = 2; data[12] = 84;
        let awake = read_with(&t, |p| { assert_eq!(p, request()); Some(frame(&data)) }).unwrap();
        assert!(awake.connected && !awake.battery_unavailable && awake.battery_at.is_some());
        assert_eq!(awake.battery.unwrap().percent, Some(84));
        let mut wrong = t; wrong.backend = BackendId::MchoseMouse;
        assert!(read_with(&wrong, |_| panic!("legacy protocol must not send")).is_err());
    }
}
