//! K99 V3 receiver battery query, separate from the shared-PID G87 V2 protocol.
//! Byte layout: mchose-hid-bridge ad74f3e, K99 V3 capture and HID descriptor.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct MchoseK99;

const PRODUCT: &str = "MCHOSE K99 V3 2.4G";
const REPORT_BYTES: u16 = 65;
const PAYLOAD_BYTES: usize = 64;
const TIMEOUT_MS: u32 = 1_000;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x3837 && i.product_id == 0x3033 && i.usage_page == 1 && i.usage == 0
}

fn control(i: &HidInterface) -> bool {
    if !management(i) || i.product != PRODUCT || i.input_len != REPORT_BYTES
        || i.output_len != REPORT_BYTES { return false; }
    let path = i.path.to_ascii_lowercase();
    path.split('#').nth(1).is_some_and(|hardware|
        hardware.split('&').any(|field| field == "mi_01"))
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|i| control(i) && interfaces.iter().filter(|other|
        management(other) && channel::collection_group(i) == channel::collection_group(other)
    ).count() == 1).collect()
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    control(i) && descriptor::Descriptor::open(api, i)
        .is_some_and(|d| d.only_reports(i, 0, REPORT_BYTES, 0, REPORT_BYTES))
}

fn target(i: &HidInterface) -> Target {
    let mut t = Target::new(BackendId::MchoseK99, i, 0);
    t.name = PRODUCT.into();
    t.family = DeviceFamily::Keyboard;
    t
}

fn request() -> [u8; REPORT_BYTES as usize] {
    let mut r = [0; REPORT_BYTES as usize];
    // Windows supplies the zero Report ID slot; the capture is a 64-byte payload.
    r[..6].copy_from_slice(&[0, 0x55, 3, 0, 0x38, 0x38]);
    r
}

fn percent(r: &[u8]) -> Option<u8> {
    if r.len() != PAYLOAD_BYTES || r[..2] != [0xaa, 3] || r[8] > 100 { return None; }
    // Captured headers differ at bytes 2/3. No charge or checksum interpretation
    // is established; only the percentage at byte 8 is shared by both captures.
    Some(r[8])
}

fn read_with(t: &Target, exchange: impl FnOnce(&[u8]) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    if t.backend != BackendId::MchoseK99 || !control(t.hid()?) { return Err(UNAVAILABLE.into()); }
    let response = exchange(&request()).ok_or(UNAVAILABLE)?;
    let value = percent(&response).ok_or(UNAVAILABLE)?;
    let mut target = target(t.hid()?);
    target.verified_identity = true;
    Ok(Snapshot {
        target,
        battery: Some(Battery { percent: Some(value), estimated_percent: None,
            charging: None, voltage_mv: None, approximate: false, level: None }),
        battery_at: Some(Instant::now()),
        battery_unavailable: false,
        connected: true,
        dpi: Dpi::default(),
        polling_rate: Default::default(),
    })
}

impl DeviceBackend for MchoseK99 {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i)).map(target).collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        let i = t.hid()?;
        if t.backend != BackendId::MchoseK99 || !proven(api, i) { return Err(UNAVAILABLE.into()); }
        let lock = channel::lock(&channel::collection_group(i));
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(i)?;
        read_with(t, |request| {
            let started = Instant::now();
            let mut remaining = 32;
            let response = device.transact_with_input_report(request, 0, TIMEOUT_MS, |r| {
                remaining -= 1;
                percent(r).is_some() || remaining == 0
            });
            trace::record(i, BackendId::MchoseK99, "query-battery", 0, 3, request.len(),
                response.as_ref().map_or(0, Vec::len), started, match response.as_deref() {
                    Some(r) if percent(r).is_some() => trace::Outcome::Ok,
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
    use std::time::Duration;

    fn interface(instance: &str) -> HidInterface {
        HidInterface {
            path: format!(r"\\?\hid#vid_3837&pid_3033&mi_01&col01#8&{instance}&0001#{{hid}}"),
            vendor_id: 0x3837, product_id: 0x3033, usage_page: 1, usage: 0,
            input_len: REPORT_BYTES, output_len: REPORT_BYTES, feature_len: 0,
            product: PRODUCT.into(), serial: "shared".into(),
        }
    }

    fn captured_40() -> Vec<u8> {
        // Full public k99v3-battery-response.hex, including its zero tail.
        let mut r = vec![0; PAYLOAD_BYTES];
        r[..9].copy_from_slice(&[0xaa, 3, 0, 0x38, 0, 0, 0, 0, 0x28]);
        r
    }

    fn captured_100() -> Vec<u8> {
        // Full second capture published in the upstream K99 V3 README.
        let mut r = vec![0; PAYLOAD_BYTES];
        r[..9].copy_from_slice(&[0xaa, 3, 1, 0, 0, 0, 0, 0, 0x64]);
        r
    }

    #[test]
    fn captured_percentages_and_zero_are_read_without_inventing_charge_flags() {
        let t = target(&interface("one"));
        for (r, expected) in [(captured_40(), 40), (captured_100(), 100)] {
            let sample = read_with(&t, |p| {
                assert_eq!(&p[..6], &[0, 0x55, 3, 0, 0x38, 0x38]);
                assert_eq!(p.len(), 65);
                assert!(p[6..].iter().all(|b| *b == 0));
                Some(r)
            }).unwrap();
            assert!(sample.connected && sample.battery_at.is_some() && !sample.battery_unavailable);
            let battery = sample.battery.unwrap();
            assert_eq!((battery.percent, battery.charging), (Some(expected), None));
            assert_eq!(sample.target.family, DeviceFamily::Keyboard);
            assert!(sample.dpi.stages.is_empty() && !sample.dpi.writable);
            assert!(!sample.polling_rate.writable);
        }
        for value in 0..=100 {
            let mut r = captured_40(); r[8] = value;
            assert_eq!(percent(&r), Some(value));
        }
        for value in [0, 1, 0x80, 0xff] {
            let mut r = captured_40(); r[2] = value; r[6] = value; r[7] = value;
            let sample = read_with(&t, |_| Some(r)).unwrap();
            assert_eq!(sample.battery.unwrap().charging, None);
        }
    }

    #[test]
    fn shared_pid_requires_exact_k99_name_interface_and_report_capacity() {
        let i = interface("one");
        assert!(control(&i));
        for name in ["MCHOSE G87 V2 2.4G", "MCHOSE K87S 2.4G", "MCHOSE K99 V3",
            "MCHOSE K99 V3 2.4G ", "USB Receiver", ""] {
            let mut wrong = i.clone(); wrong.product = name.into(); assert!(!control(&wrong));
        }
        for (vid, pid, page, usage, input, output) in [
            (0x3837, 0x1015, 1, 0, 65, 65), (0x3554, 0xfa09, 1, 0, 65, 65),
            (0x3837, 0x3033, 1, 2, 65, 65), (0x3837, 0x3033, 1, 6, 65, 65),
            (0x3837, 0x3033, 0xff80, 1, 65, 65), (0x3837, 0x3033, 1, 0, 64, 65),
            (0x3837, 0x3033, 1, 0, 65, 64), (0x3837, 0x3033, 1, 0, 66, 65),
        ] {
            let mut wrong = i.clone();
            wrong.vendor_id = vid; wrong.product_id = pid; wrong.usage_page = page;
            wrong.usage = usage; wrong.input_len = input; wrong.output_len = output;
            assert!(!control(&wrong));
        }
        for path in [i.path.replace("mi_01", "mi_00"), i.path.replace("mi_01", "mi_010"),
            "invalid".into(), String::new()] {
            let mut wrong = i.clone(); wrong.path = path; assert!(!control(&wrong));
        }
        let mut upper = i.clone(); upper.path.make_ascii_uppercase(); assert!(control(&upper));
    }

    #[test]
    fn duplicate_management_collections_refuse_query_and_receiver_instances_stay_separate() {
        let first = interface("one");
        let second = interface("two");
        assert_eq!(candidates(&[first.clone(), second.clone()]).len(), 2);
        assert_ne!(target(&first).key, target(&second).key);
        assert!(!target(&first).same_channel(&target(&second)));
        let mut sibling = first.clone();
        sibling.path = sibling.path.replace("col01", "col02").replace("&0001#", "&0002#");
        // A conflicting sibling does not become safe by changing its product string.
        sibling.product = "MCHOSE G87 V2 2.4G".into();
        assert!(candidates(&[first, sibling]).is_empty());
    }

    #[test]
    fn echoes_other_commands_truncation_and_out_of_range_are_not_battery_samples() {
        assert!(percent(&request()).is_none());
        assert!(percent(&request()[1..]).is_none());
        assert!(percent(&[0; PAYLOAD_BYTES]).is_none());
        let valid = captured_40();
        for length in 0..PAYLOAD_BYTES { assert!(percent(&valid[..length]).is_none()); }
        let mut long = valid.clone(); long.push(0); assert!(percent(&long).is_none());
        for (offset, value) in [(0, 0), (0, 0x55), (1, 4), (1, 5), (1, 0x4a)] {
            let mut wrong = valid.clone(); wrong[offset] = value; assert!(percent(&wrong).is_none());
        }
        for value in 101..=255 {
            let mut wrong = valid.clone(); wrong[8] = value; assert!(percent(&wrong).is_none());
        }
        let mut wrong = target(&interface("one")); wrong.backend = BackendId::MchoseGlw;
        assert!(read_with(&wrong, |_| panic!("wrong backend must not query")).is_err());
        wrong = target(&interface("one")); wrong.hid_mut().product = "MCHOSE G87 V2 2.4G".into();
        assert!(read_with(&wrong, |_| panic!("shared PID must not authorize another model")).is_err());
    }

    #[test]
    fn missing_reply_keeps_original_sample_age_and_a_later_reply_restores_battery() {
        let t = target(&interface("one"));
        let initial = read_with(&t, |_| Some(captured_40())).unwrap();
        let now = initial.battery_at.unwrap();
        let mut status = status::Status::default();
        let lifetime = Duration::from_secs(120);
        status.observe_sample(Some(true), initial.battery.as_ref(), initial.battery_at, now, lifetime);
        assert_eq!(status.quality, crate::device::ReadingQuality::Live);
        for seconds in [10, 119, 120] {
            let mut calls = 0;
            assert!(read_with(&t, |_| { calls += 1; None }).is_err());
            assert_eq!(calls, 1);
            status.observe_sample(None, None, None, now + Duration::from_secs(seconds), lifetime);
            assert_eq!(status.measured_at, Some(now));
        }
        assert_eq!(status.quality, crate::device::ReadingQuality::Stale);
        let mut r = captured_40(); r[8] = 39;
        let recovered = read_with(&t, |_| Some(r)).unwrap();
        let later = now + Duration::from_secs(130);
        status.observe_sample(Some(true), recovered.battery.as_ref(), Some(later), later, lifetime);
        assert_eq!(status.quality, crate::device::ReadingQuality::Live);
        assert_eq!(status.battery.unwrap().percent, Some(39));
    }
}
