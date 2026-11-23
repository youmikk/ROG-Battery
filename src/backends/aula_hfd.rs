//! F87ProV2 HFD receiver status page, isolated from shared-ID SONiX keyboards.
//! Protocol evidence: MyFinder aa0228a F87ProV2 capture and AULA HFD SDK.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct AulaHfd;

const PRODUCT: &str = "AULA F87ProV2D Dongle";
const REPORT_BYTES: u16 = 33;
const PAYLOAD_BYTES: usize = 32;
const TIMEOUT_MS: u32 = 1_000;
const UNAVAILABLE: &str = "设备无有效状态回报";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x0c45 && i.product_id == 0xfefe
        && i.usage_page == 0xff60 && i.usage == 0x0061
}

fn control(i: &HidInterface) -> bool {
    if !management(i) || i.product != PRODUCT || i.input_len != REPORT_BYTES
        || i.output_len != REPORT_BYTES { return false; }
    let path = i.path.to_ascii_lowercase();
    path.split('#').nth(1).is_some_and(|hardware|
        hardware.split('&').any(|field| field == "mi_03"))
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
    let mut t = Target::new(BackendId::AulaHfd, i, 0);
    t.name = "AULA F87ProV2".into();
    t.family = DeviceFamily::Keyboard;
    t
}

fn request() -> [u8; REPORT_BYTES as usize] {
    let mut r = [0; REPORT_BYTES as usize];
    // Unnumbered 32-byte payload plus Windows' zero Report ID slot.
    r[..5].copy_from_slice(&[0, 0xaa, 0x10, 0x18, 0]);
    r
}

fn percent(r: &[u8]) -> Option<u8> {
    if r.len() != PAYLOAD_BYTES || r[..5] != [0x55, 0x10, 0x18, 0, 0]
        || r[25] > 100 { return None; }
    // The official app also polls page 0x18. Compare both address bytes so its
    // delayed reply cannot be mistaken for this page's battery reading.
    Some(r[25])
}

fn read_with(t: &Target, exchange: impl FnOnce(&[u8]) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    if t.backend != BackendId::AulaHfd || !control(t.hid()?) { return Err(UNAVAILABLE.into()); }
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

impl DeviceBackend for AulaHfd {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i)).map(target).collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        let i = t.hid()?;
        if t.backend != BackendId::AulaHfd || !proven(api, i) { return Err(UNAVAILABLE.into()); }
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
            trace::record(i, BackendId::AulaHfd, "query-battery", 0, 0x10, request.len(),
                response.as_ref().map_or(0, Vec::len), started, match response.as_deref() {
                    Some(r) if percent(r).is_some() => trace::Outcome::Ok,
                    Some(_) => trace::Outcome::InvalidData,
                    None => trace::Outcome::NoReply,
                });
            // Wait for the next poll after failure. The transport does not
            // distinguish a silent accepted output from a failed write.
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
            path: format!(r"\\?\hid#vid_0c45&pid_fefe&mi_03&col01#8&{instance}&0001#{{hid}}"),
            vendor_id: 0x0c45, product_id: 0xfefe, usage_page: 0xff60, usage: 0x0061,
            input_len: REPORT_BYTES, output_len: REPORT_BYTES, feature_len: 0,
            product: PRODUCT.into(), serial: "shared".into(),
        }
    }

    fn captured_48() -> Vec<u8> {
        // Complete contributor capture published on 2026-11-18, not a local
        // hardware validation. The byte at index 24 is not a charging flag.
        vec![0x55, 0x10, 0x18, 0, 0, 0, 0, 0, 0, 0, 0, 0x12, 0xa6, 0x38, 0x11, 0x27,
            0x66, 1, 0, 0, 0x66, 1, 0, 0, 2, 0x30, 0, 0, 0, 0, 0, 0]
    }

    #[test]
    fn captured_status_reports_48_percent_without_claiming_charging_or_controls() {
        let t = target(&interface("one"));
        let sample = read_with(&t, |p| {
            assert_eq!(p.len(), 33);
            assert_eq!(&p[..5], &[0, 0xaa, 0x10, 0x18, 0]);
            assert!(p[5..].iter().all(|b| *b == 0));
            Some(captured_48())
        }).unwrap();
        assert!(sample.connected && sample.battery_at.is_some() && !sample.battery_unavailable);
        assert!(sample.target.verified_identity);
        assert_eq!(sample.target.family, DeviceFamily::Keyboard);
        let battery = sample.battery.unwrap();
        assert_eq!((battery.percent, battery.charging), (Some(48), None));
        assert!(!battery.approximate);
        assert!(sample.dpi.stages.is_empty() && !sample.dpi.writable);
        assert!(!sample.polling_rate.writable);
        for value in 0..=100 {
            let mut r = captured_48(); r[25] = value;
            assert_eq!(percent(&r), Some(value));
        }
        for value in [0, 1, 2, 0x80, 0xff] {
            let mut r = captured_48(); r[24] = value; r[26] = value;
            assert_eq!(read_with(&t, |_| Some(r)).unwrap().battery.unwrap().charging, None);
        }
    }

    #[test]
    fn shared_usb_identity_requires_exact_model_interface_and_report_sizes() {
        let i = interface("one");
        assert!(control(&i));
        for product in ["AULA F87ProV2C Dongle", "AULA F108ProV2 Dongle", "F99 MAX Dongle",
            "2.4G Dongle", "AULA F87ProV2D Dongle ", ""] {
            let mut wrong = i.clone(); wrong.product = product.into(); assert!(!control(&wrong));
        }
        for (vid, pid, page, usage, input, output) in [
            (0x05ac, 0x024f, 0xff60, 0x61, 33, 33), (0x0c45, 0xfefc, 0xff60, 0x61, 33, 33),
            (0x0c45, 0xfefe, 0xff68, 0x61, 33, 33), (0x0c45, 0xfefe, 0xff60, 0x62, 33, 33),
            (0x0c45, 0xfefe, 1, 6, 33, 33), (0x0c45, 0xfefe, 0xff60, 0x61, 32, 33),
            (0x0c45, 0xfefe, 0xff60, 0x61, 33, 32), (0x0c45, 0xfefe, 0xff60, 0x61, 65, 65),
        ] {
            let mut wrong = i.clone(); wrong.vendor_id = vid; wrong.product_id = pid;
            wrong.usage_page = page; wrong.usage = usage; wrong.input_len = input;
            wrong.output_len = output; assert!(!control(&wrong));
        }
        for path in [i.path.replace("mi_03", "mi_01"), i.path.replace("mi_03", "mi_030"),
            i.path.replace("&mi_03", ""), "invalid".into(), String::new()] {
            let mut wrong = i.clone(); wrong.path = path; assert!(!control(&wrong));
        }
        let mut upper = i.clone(); upper.path.make_ascii_uppercase(); assert!(control(&upper));
    }

    #[test]
    fn duplicate_management_collections_block_query_and_receivers_remain_independent() {
        let first = interface("one");
        let second = interface("two");
        assert_eq!(candidates(&[first.clone(), second.clone()]).len(), 2);
        assert_ne!(target(&first).key, target(&second).key);
        assert!(!target(&first).same_channel(&target(&second)));
        let mut sibling = first.clone();
        sibling.path = sibling.path.replace("col01", "col02").replace("&0001#", "&0002#");
        sibling.product = "2.4G Dongle".into();
        assert!(candidates(&[first, sibling]).is_empty());
    }

    #[test]
    fn other_pages_echoes_truncation_and_invalid_percentages_are_rejected() {
        let valid = captured_48();
        assert!(percent(&request()).is_none());
        assert!(percent(&request()[1..]).is_none());
        assert!(percent(&[0; PAYLOAD_BYTES]).is_none());
        for length in 0..PAYLOAD_BYTES { assert!(percent(&valid[..length]).is_none()); }
        let mut long = valid.clone(); long.push(0); assert!(percent(&long).is_none());
        for (offset, value) in [(0, 0xaa), (1, 0x13), (2, 0x10), (3, 0x18), (4, 1)] {
            let mut wrong = valid.clone(); wrong[offset] = value; assert!(percent(&wrong).is_none());
        }
        for value in 101..=255 {
            let mut wrong = valid.clone(); wrong[25] = value; assert!(percent(&wrong).is_none());
        }
        let mut wrong = target(&interface("one")); wrong.backend = BackendId::Aula;
        assert!(read_with(&wrong, |_| panic!("wrong backend must not query")).is_err());
        wrong = target(&interface("one")); wrong.hid_mut().product = "2.4G Dongle".into();
        assert!(read_with(&wrong, |_| panic!("shared PID does not prove a model")).is_err());
    }

    #[test]
    fn failed_query_does_not_refresh_old_sample_and_later_status_recovers() {
        let t = target(&interface("one"));
        let initial = read_with(&t, |_| Some(captured_48())).unwrap();
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
        let mut r = captured_48(); r[25] = 47;
        let recovered = read_with(&t, |_| Some(r)).unwrap();
        let later = now + Duration::from_secs(130);
        status.observe_sample(Some(true), recovered.battery.as_ref(), Some(later), later, lifetime);
        assert_eq!(status.quality, crate::device::ReadingQuality::Live);
        assert_eq!(status.battery.unwrap().percent, Some(47));
    }
}
