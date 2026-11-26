//! Halo Space EC battery query; no audio settings or device identity commands.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct Edifier;

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x2d99 && i.product_id == 0xe049 && i.usage_page == 0xff14 && i.usage == 1
}

fn control(i: &HidInterface) -> bool {
    management(i) && i.input_len == 64 && i.output_len == 64
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|i| control(i) && interfaces.iter().filter(|other|
        management(other) && channel::collection_group(i) == channel::collection_group(other)).count() == 1).collect()
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    control(i) && descriptor::Descriptor::open(api, i)
        .is_some_and(|caps| caps.only_reports(i, 0x2f, 64, 0x2e, 64))
}

fn request() -> [u8; 64] {
    let mut frame = [0; 64];
    // The first byte is already the Windows output report ID. No extra 00.
    frame[..7].copy_from_slice(&[0x2e, 0xaa, 0xec, 0xd0, 0, 0, 0x66]);
    frame
}

fn battery(reply: &[u8]) -> Option<Battery> {
    if reply.len() != 64 || reply[..6] != [0x2f, 0xbb, 0xec, 0xd0, 0, 1]
        || reply[7] != reply[1..7].iter().fold(0u8, |sum, b| sum.wrapping_add(*b)) { return None; }
    let percent = reply[6] ^ 0xa5;
    if percent > 100 { return None; }
    // Only the one-byte payload is defined. Device scratch data in the tail
    // must never become charging, identity or another battery component.
    Some(Battery { percent: Some(percent), estimated_percent: None, charging: None,
        voltage_mv: None, approximate: false, level: None })
}

fn read_with(target: &Target, mut exchange: impl FnMut(&[u8]) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    let fail = || crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试").to_string();
    if !control(target.hid()?) { return Err(fail()); }
    let reply = exchange(&request()).ok_or_else(fail)?;
    let battery = battery(&reply).ok_or_else(fail)?;
    Ok(Snapshot { target: target.clone(), battery: Some(battery), battery_at: Some(Instant::now()),
        battery_unavailable: false, connected: true, dpi: Dpi::default(), polling_rate: Default::default() })
}

impl DeviceBackend for Edifier {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i)).map(|i| {
            let mut target = Target::new(BackendId::Edifier, i, 0);
            target.name = "EDIFIER Halo Space".into();
            target.family = DeviceFamily::Headset;
            target
        }).collect())
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let interface = target.hid()?;
        if !proven(api, interface) { return Err(crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试").into()); }
        let lock = channel::lock(&channel::collection_group(interface));
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(interface)?;
        read_with(target, |request| {
            let started = Instant::now();
            let response = device.transact(request, 1_200, |r| battery(r).is_some());
            trace::record(interface, BackendId::Edifier, "query-battery", 0xec, 0xd0, request.len(),
                response.as_ref().map_or(0, Vec::len), started,
                if response.is_some() { trace::Outcome::Ok } else { trace::Outcome::NoReply });
            response
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface() -> HidInterface {
        HidInterface { path: r"\\?\hid#vid_2d99&pid_e049&mi_04&col02#8&unit-a&0002#{hid}".into(),
            vendor_id: 0x2d99, product_id: 0xe049, usage_page: 0xff14, usage: 1,
            input_len: 64, output_len: 64, feature_len: 0, serial: "shared".into(),
            product: "Halo Space D1".into() }
    }

    fn response(percent: u8) -> [u8; 64] {
        let mut r = [0; 64];
        r[..7].copy_from_slice(&[0x2f, 0xbb, 0xec, 0xd0, 0, 1, percent ^ 0xa5]);
        r[7] = r[1..7].iter().fold(0u8, |sum, b| sum.wrapping_add(*b));
        r
    }

    #[test]
    fn published_battery_prefix_decodes_35_percent_and_ignores_stale_trailing_bytes() {
        // FerrisPulse's published valid eight-byte prefix, with synthetic tail.
        let mut r = [0; 64];
        r[..8].copy_from_slice(&[0x2f, 0xbb, 0xec, 0xd0, 0, 1, 0x86, 0xfe]);
        for tail in [0, 0xa5, 0xff] {
            r[8..].fill(tail);
            let b = battery(&r).unwrap();
            assert_eq!((b.percent, b.charging, b.voltage_mv), (Some(35), None, None));
            assert_eq!(b.display_percent().unwrap().label(true), "35%");
        }
        assert_eq!(&request()[..7], &[0x2e, 0xaa, 0xec, 0xd0, 0, 0, 0x66]);
        assert!(request()[7..].iter().all(|b| *b == 0));
    }

    #[test]
    fn valid_zero_full_and_unknown_payloads_never_invent_charging() {
        for percent in 0..=100 {
            let b = battery(&response(percent)).unwrap();
            assert_eq!((b.percent, b.charging), (Some(percent), None));
        }
        for percent in 101..=255 { assert!(battery(&response(percent)).is_none()); }
    }

    #[test]
    fn request_echo_wrong_headers_masking_order_and_lengths_cannot_be_battery() {
        assert!(battery(&request()).is_none());
        let valid = response(35);
        for len in 0..64 { assert!(battery(&valid[..len]).is_none()); }
        let mut long = valid.to_vec(); long.push(0); assert!(battery(&long).is_none());
        for (offset, value) in [(0, 0x2e), (0, 0x55), (1, 0xaa), (1, 0), (2, 0xed),
            (3, 0xc9), (3, 0xf2), (4, 1), (5, 0), (5, 2), (5, 57), (5, 255)] {
            let mut wrong = valid; wrong[offset] = value;
            wrong[7] = wrong[1..7].iter().fold(0u8, |sum, b| sum.wrapping_add(*b));
            assert!(battery(&wrong).is_none());
        }
        let mut bad_sum = valid; bad_sum[7] ^= 1;
        assert!(battery(&bad_sum).is_none());
        bad_sum = valid;
        bad_sum[7] = valid[7].wrapping_sub(valid[6]).wrapping_add(35);
        assert!(battery(&bad_sum).is_none()); // Checksum covers masked wire bytes.
    }

    #[test]
    fn poll_only_queries_battery_and_silence_cannot_create_a_new_sample() {
        let target = Target::new(BackendId::Edifier, &interface(), 0);
        let mut commands = Vec::new();
        let sample = read_with(&target, |frame| {
            assert_eq!(frame, request()); commands.push(frame[3]); Some(response(35).to_vec())
        }).unwrap();
        assert_eq!(commands, [0xd0]);
        assert!(sample.connected && !sample.battery_unavailable && sample.battery_at.is_some());
        assert!(sample.dpi.stages.is_empty());
        assert!(read_with(&target, |_| None).is_err());
        assert!(read_with(&target, |_| Some(request().to_vec())).is_err());
    }

    #[test]
    fn exact_model_control_and_unique_collection_exclude_upgrade_and_duplicate_targets() {
        let i = interface(); assert!(control(&i));
        for (vid, pid, page, usage, input, output) in [
            (0x2d99, 0xe048, 0xff14, 1, 64, 64),
            (0x2d99, 0xe049, 0xff13, 1, 64, 64),
            (0x2d99, 0xe049, 0x000c, 1, 64, 64),
            (0x2d99, 0xe049, 0xff14, 2, 64, 64),
            (0x2d99, 0xe049, 0xff14, 1, 62, 64),
            (0x2d99, 0xe049, 0xff14, 1, 64, 65),
            (0x37d7, 0x1002, 0xffa0, 1, 64, 64),
        ] {
            let mut wrong = i.clone();
            wrong.vendor_id = vid; wrong.product_id = pid; wrong.usage_page = page; wrong.usage = usage;
            wrong.input_len = input; wrong.output_len = output;
            assert!(!control(&wrong));
            let t = Target::new(BackendId::Edifier, &wrong, 0);
            assert!(read_with(&t, |_| panic!("must not send")).is_err());
        }
        let mut second = i.clone(); second.path = second.path.replace("unit-a", "unit-b");
        assert_eq!(candidates(&[i.clone(), second.clone()]).len(), 2);
        assert_ne!(Target::new(BackendId::Edifier, &i, 0).key, Target::new(BackendId::Edifier, &second, 0).key);
        let mut sibling = i.clone();
        sibling.path = sibling.path.replace("col02", "col03").replace("&0002", "&0003");
        sibling.input_len = 62;
        assert!(candidates(&[i, sibling]).is_empty());
    }
}
