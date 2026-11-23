//! Read-only device-info page for the original EWEADN X87 wireless keyboard.
//! Shared USB identity is accepted only with the HFD manufacturer/product pair.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct EweadnX87;

const PRODUCT: &str = "2.4G Dongle";
const TIMEOUT_MS: u32 = 600;
const UNAVAILABLE: &str = "设备无有效状态回报";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x0c45 && i.product_id == 0xfefe
        && i.usage_page == 0xff60 && i.usage == 0x0061
}

fn control(i: &HidInterface) -> bool {
    if !management(i) || i.product != PRODUCT || !matches!(i.input_len, 33 | 65)
        || i.output_len != i.input_len { return false; }
    let path = i.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    parts.len() == 4 && parts[0] == r"\\?\hid" && !parts[2].is_empty()
        && ["vid_0c45", "pid_fefe", "mi_03"].iter().all(|expected|
            parts[1].split('&').any(|field| field == *expected))
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|i| control(i) && interfaces.iter().filter(|other|
        management(other) && channel::collection_group(i) == channel::collection_group(other)
    ).count() == 1).collect()
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    control(i) && descriptor::Descriptor::open(api, i)
        .is_some_and(|d| d.only_reports(i, 0, i.input_len, 0, i.output_len))
}

fn target(i: &HidInterface) -> Target {
    // Discovery has not established that this generic receiver is an X87 yet.
    let mut t = Target::new(BackendId::EweadnX87, i, 0);
    t.family = DeviceFamily::Keyboard;
    t
}

fn request(report_bytes: u16) -> Vec<u8> {
    let mut r = vec![0; usize::from(report_bytes)];
    // Report ID 0, GET_INFO, 24 bytes at address zero, final packet marker.
    r[..9].copy_from_slice(&[0, 0xaa, 0x10, 0x18, 0, 0, 0, 1, 0]);
    r
}

fn percent(r: &[u8], report_bytes: u16) -> Option<u8> {
    if !matches!(report_bytes, 33 | 65) || r.len() != usize::from(report_bytes - 1)
        || r[..2] != [0x55, 0x10] || r[3..5] != [0, 0]
        || r[20..25] != [2, 0, 5, 0, 2] || !(1..=100).contains(&r[25]) { return None; }
    // Byte 2 may describe a type rather than a length. Model IDs are little
    // endian at payload offsets 12/14; mode 2 is wireless. Zero is a placeholder.
    Some(r[25])
}

fn read_with(t: &Target, exchange: impl FnOnce(&[u8]) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    let i = t.hid()?;
    if t.backend != BackendId::EweadnX87 || !control(i) { return Err(UNAVAILABLE.into()); }
    let response = exchange(&request(i.output_len)).ok_or(UNAVAILABLE)?;
    let value = percent(&response, i.input_len).ok_or(UNAVAILABLE)?;
    let mut target = target(i);
    target.name = "EWEADN X87".into();
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

impl DeviceBackend for EweadnX87 {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i)).map(target).collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        let i = t.hid()?;
        if t.backend != BackendId::EweadnX87 || !proven(api, i) { return Err(UNAVAILABLE.into()); }
        let lock = channel::lock(&channel::collection_group(i));
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(i)?;
        read_with(t, |request| {
            let started = Instant::now();
            let mut remaining = 32;
            let response = device.transact_with_input_report(request, 0, TIMEOUT_MS, |r| {
                remaining -= 1;
                percent(r, i.input_len).is_some() || remaining == 0
            });
            trace::record(i, BackendId::EweadnX87, "query-battery", 0, 0x10, request.len(),
                response.as_ref().map_or(0, Vec::len), started, match response.as_deref() {
                    Some(r) if percent(r, i.input_len).is_some() => trace::Outcome::Ok,
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

    fn interface(instance: &str, report_bytes: u16) -> HidInterface {
        HidInterface {
            path: format!(r"\\?\hid#vid_0c45&pid_fefe&mi_03&col01#8&{instance}&0001#{{hid}}"),
            vendor_id: 0x0c45, product_id: 0xfefe, usage_page: 0xff60, usage: 0x0061,
            input_len: report_bytes, output_len: report_bytes, feature_len: 0,
            product: PRODUCT.into(), serial: "shared".into(),
        }
    }

    fn response(report_bytes: u16, level: u8) -> Vec<u8> {
        // Synthetic fixture from the public byte layout; no hardware claim.
        let mut r = vec![0; usize::from(report_bytes - 1)];
        r[..8].copy_from_slice(&[0x55, 0x10, 0x18, 0, 0, 0, 1, 0]);
        r[20..26].copy_from_slice(&[2, 0, 5, 0, 2, level]);
        r
    }

    #[test]
    fn proven_report_sizes_share_one_readonly_info_page_and_firmware_identity() {
        for size in [33, 65] {
            let t = target(&interface("one", size));
            assert_eq!(t.name, PRODUCT);
            assert!(!t.verified_identity);
            for level in [1, 37, 100] {
                let sample = read_with(&t, |p| {
                    assert_eq!(p.len(), usize::from(size));
                    assert_eq!(&p[..9], &[0, 0xaa, 0x10, 0x18, 0, 0, 0, 1, 0]);
                    assert!(p[9..].iter().all(|b| *b == 0));
                    Some(response(size, level))
                }).unwrap();
                assert!(sample.connected && sample.battery_at.is_some());
                assert!(sample.target.verified_identity);
                assert_eq!(sample.target.name, "EWEADN X87");
                assert_eq!(sample.target.family, DeviceFamily::Keyboard);
                let battery = sample.battery.unwrap();
                assert_eq!((battery.percent, battery.charging), (Some(level), None));
                assert!(!battery.approximate);
                assert!(!sample.dpi.writable && !sample.polling_rate.writable);
            }
            for value in [0, 1, 0x18, 0xff] {
                let mut r = response(size, 37); r[2] = value; r[26] = value;
                assert_eq!(read_with(&t, |_| Some(r)).unwrap().battery.unwrap().charging, None);
            }
        }
    }

    #[test]
    fn shared_id_does_not_admit_other_models_interfaces_or_conflicting_paths() {
        let i = interface("one", 33);
        assert!(control(&i));
        for product in ["AULA F87ProV2D Dongle", "X87", "X87S", "X87 V2", "2.4G Dongle ", ""] {
            let mut wrong = i.clone(); wrong.product = product.into(); assert!(!control(&wrong));
        }
        for (vid, pid, page, usage, input, output) in [
            (0x0c45, 0xfef9, 0xff60, 0x61, 33, 33), (0x0c45, 0x8006, 0xff60, 0x61, 33, 33),
            (0x3151, 0x506e, 0xff60, 0x61, 33, 33), (0x0c45, 0xfefe, 0xff59, 0x61, 33, 33),
            (0x0c45, 0xfefe, 0xff60, 0x62, 33, 33), (0x0c45, 0xfefe, 1, 6, 33, 33),
            (0x0c45, 0xfefe, 0xff60, 0x61, 32, 33), (0x0c45, 0xfefe, 0xff60, 0x61, 33, 65),
            (0x0c45, 0xfefe, 0xff60, 0x61, 64, 64), (0x0c45, 0xfefe, 0xff60, 0x61, 34, 34),
        ] {
            let mut wrong = i.clone(); wrong.vendor_id = vid; wrong.product_id = pid;
            wrong.usage_page = page; wrong.usage = usage; wrong.input_len = input;
            wrong.output_len = output; assert!(!control(&wrong));
        }
        for path in [i.path.replace("mi_03", "mi_01"), i.path.replace("mi_03", "mi_030"),
            i.path.replace("pid_fefe", "pid_fef9"), i.path.replace("vid_0c45", "vid_3151"),
            i.path.replace("8&one&0001", ""), "invalid".into(), String::new()] {
            let mut wrong = i.clone(); wrong.path = path; assert!(!control(&wrong));
        }
        let mut upper = i.clone(); upper.path.make_ascii_uppercase(); assert!(control(&upper));
    }

    #[test]
    fn duplicate_collections_block_query_and_receiver_instances_stay_separate() {
        let first = interface("one", 33);
        let second = interface("two", 65);
        assert_eq!(candidates(&[first.clone(), second.clone()]).len(), 2);
        assert_ne!(target(&first).key, target(&second).key);
        assert!(!target(&first).same_channel(&target(&second)));
        let mut sibling = first.clone();
        sibling.path = sibling.path.replace("col01", "col02").replace("&0001#", "&0002#");
        sibling.product = "AULA F87ProV2D Dongle".into();
        assert!(candidates(&[first, sibling]).is_empty());
    }

    #[test]
    fn foreign_model_wrong_mode_placeholder_and_unrelated_reply_are_not_samples() {
        for size in [33, 65] {
            let valid = response(size, 37);
            assert!(percent(&request(size), size).is_none());
            assert!(percent(&request(size)[1..], size).is_none());
            for length in 0..valid.len() { assert!(percent(&valid[..length], size).is_none()); }
            let mut long = valid.clone(); long.push(0); assert!(percent(&long, size).is_none());
            for (offset, value) in [(0, 0xaa), (1, 0x20), (3, 0x18), (4, 1),
                (20, 3), (21, 1), (22, 6), (23, 1), (24, 0), (24, 1), (24, 3), (25, 0), (25, 101)] {
                let mut wrong = valid.clone(); wrong[offset] = value;
                assert!(percent(&wrong, size).is_none());
                let t = target(&interface("one", size));
                assert!(read_with(&t, |_| Some(wrong)).is_err());
            }
        }
        assert!(percent(&response(33, 37), 0).is_none());
        let mut wrong = target(&interface("one", 33)); wrong.backend = BackendId::Aula;
        assert!(read_with(&wrong, |_| panic!("wrong backend must not query")).is_err());
        wrong = target(&interface("one", 33)); wrong.hid_mut().product = "X87S".into();
        assert!(read_with(&wrong, |_| panic!("wrong model must not query")).is_err());
    }

    #[test]
    fn timeout_keeps_original_sample_time_until_a_fresh_valid_page_recovers() {
        let t = target(&interface("one", 33));
        let initial = read_with(&t, |_| Some(response(33, 37))).unwrap();
        let now = initial.battery_at.unwrap();
        let lifetime = Duration::from_secs(120);
        let mut status = status::Status::default();
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
        let recovered = read_with(&t, |_| Some(response(33, 36))).unwrap();
        let later = now + Duration::from_secs(130);
        status.observe_sample(Some(true), recovered.battery.as_ref(), Some(later), later, lifetime);
        assert_eq!(status.quality, crate::device::ReadingQuality::Live);
        assert_eq!(status.battery.unwrap().percent, Some(36));
    }
}
