//! AULA F99 MAX receiver, using its 48-byte GET_DEVICE_INFO transaction.
//! The shared USB ID is gated by product name, report proof and firmware ID.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct AulaF99;

const PRODUCT: &str = "F99 MAX Dongle";
const REPORT_BYTES: u16 = 33;
const PAYLOAD_BYTES: usize = 32;
const TIMEOUT_MS: u32 = 600;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x05ac && i.product_id == 0x024f
        && i.usage_page == 0xff60 && i.usage == 0x0061
}

fn control(i: &HidInterface) -> bool {
    if !management(i) || i.product != PRODUCT || i.input_len != REPORT_BYTES
        || i.output_len != REPORT_BYTES { return false; }
    let path = i.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    parts.len() == 4 && parts[0] == r"\\?\hid" && !parts[2].is_empty()
        && ["vid_05ac", "pid_024f", "mi_03"].iter().all(|expected|
            parts[1].split('&').any(|field| field == *expected))
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
    let mut t = Target::new(BackendId::AulaF99, i, 0);
    t.family = DeviceFamily::Keyboard;
    t
}

fn request(page: u8) -> Option<[u8; REPORT_BYTES as usize]> {
    if !matches!(page, 0 | 0x18) { return None; }
    let mut r = [0; REPORT_BYTES as usize];
    // Windows' zero ID slot plus a complete 32-byte payload. F99 MAX reads
    // 48 info bytes in two pages, unlike the later 56-byte generic HFD reader.
    r[..9].copy_from_slice(&[0, 0xaa, 0x10, 0x18, page, 0, 0, u8::from(page == 0x18), 0]);
    Some(r)
}

fn page_reply(r: &[u8], page: u8) -> bool {
    r.len() == PAYLOAD_BYTES
        && r[..8] == [0x55, 0x10, 0x18, page, 0, 0, u8::from(page == 0x18), 0]
}

fn first_percent(r: &[u8]) -> Option<u8> {
    if !page_reply(r, 0) || r[12..16] != [0x45, 0x0c, 0x0a, 0x80]
        || r[24] != 2 || !(1..=100).contains(&r[25]) || r[26] > 1 { return None; }
    Some(r[25])
}

fn read_with(t: &Target, mut exchange: impl FnMut(&[u8], u8) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    let i = t.hid()?;
    if t.backend != BackendId::AulaF99 || !control(i) { return Err(UNAVAILABLE.into()); }
    let first = exchange(&request(0).unwrap(), 0).ok_or(UNAVAILABLE)?;
    // A foreign or placeholder first page cannot authorize the second write.
    let value = first_percent(&first).ok_or(UNAVAILABLE)?;
    let second = exchange(&request(0x18).unwrap(), 0x18).ok_or(UNAVAILABLE)?;
    if !page_reply(&second, 0x18) { return Err(UNAVAILABLE.into()); }
    let mut target = target(i);
    target.name = "AULA F99 MAX".into();
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

impl DeviceBackend for AulaF99 {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i)).map(target).collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        let i = t.hid()?;
        if t.backend != BackendId::AulaF99 || !proven(api, i) { return Err(UNAVAILABLE.into()); }
        let lock = channel::lock(&channel::collection_group(i));
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(i)?;
        read_with(t, |request, page| {
            let started = Instant::now();
            let mut remaining = 32;
            let response = device.transact_with_input_report(request, 0, TIMEOUT_MS, |r| {
                remaining -= 1;
                // Stop on the requested info page, even if it reports another
                // model. read_with will reject it without sending the next page.
                (r.len() >= 5 && r[..2] == [0x55, 0x10] && r[3..5] == [page, 0])
                    || remaining == 0
            });
            let valid = response.as_deref().is_some_and(|r| if page == 0 {
                first_percent(r).is_some()
            } else { page_reply(r, page) });
            trace::record(i, BackendId::AulaF99, "query-battery", u16::from(page), 0x10,
                request.len(), response.as_ref().map_or(0, Vec::len), started,
                if valid { trace::Outcome::Ok } else if response.is_some() {
                    trace::Outcome::InvalidData
                } else { trace::Outcome::NoReply });
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
            path: format!(r"\\?\hid#vid_05ac&pid_024f&mi_03&col01#8&{instance}&0001#{{hid}}"),
            vendor_id: 0x05ac, product_id: 0x024f, usage_page: 0xff60, usage: 0x0061,
            input_len: REPORT_BYTES, output_len: REPORT_BYTES, feature_len: 0,
            product: PRODUCT.into(), serial: "shared".into(),
        }
    }

    fn captured() -> [Vec<u8>; 2] {
        // Full two-page 68% capture in MyFinder issue #858, independently
        // compared by the contributor with the official app and keyboard display.
        [vec![0x55, 0x10, 0x18, 0, 0, 0, 0, 0, 0x40, 0, 0, 0x0c, 0x45, 0x0c, 0x0a, 0x80,
            0x18, 1, 0xff, 0xff, 0x11, 0, 6, 0, 2, 0x44, 0, 0, 0, 0, 0x68, 0],
         vec![0x55, 0x10, 0x18, 0x18, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 2, 0x44, 0, 0, 0, 0, 0x68, 0]]
    }

    fn sample(t: &Target, replies: [Vec<u8>; 2]) -> Result<Snapshot, String> {
        let mut replies = replies.into_iter();
        read_with(t, |_, _| replies.next())
    }

    #[test]
    fn full_68_percent_capture_uses_two_complete_windows_reports_in_order() {
        let t = target(&interface("one"));
        let mut replies = captured().into_iter();
        let mut pages = vec![];
        let reading = read_with(&t, |p, page| {
            pages.push(page);
            assert_eq!(p.len(), 33);
            assert_eq!(&p[..9], &[0, 0xaa, 0x10, 0x18, page, 0, 0, u8::from(page == 0x18), 0]);
            assert!(p[9..].iter().all(|b| *b == 0));
            replies.next()
        }).unwrap();
        assert_eq!(pages, [0, 0x18]);
        assert_eq!(reading.target.name, "AULA F99 MAX");
        assert_eq!(reading.target.family, DeviceFamily::Keyboard);
        assert!(reading.target.verified_identity && reading.connected && reading.battery_at.is_some());
        let battery = reading.battery.unwrap();
        assert_eq!((battery.percent, battery.charging), (Some(68), None));
        assert!(!battery.approximate && !reading.dpi.writable && !reading.polling_rate.writable);
        for page in [1, 0x30, 0xff] { assert!(request(page).is_none()); }
        for charge in [0, 1] {
            let mut r = captured(); r[0][26] = charge;
            assert_eq!(sample(&t, r).unwrap().battery.unwrap().charging, None);
        }
    }

    #[test]
    fn only_exact_f99_max_interface_is_eligible_on_the_shared_usb_id() {
        let i = interface("one");
        assert!(control(&i));
        for product in ["AULA F99 Dongle", "AULA F99 MAX", "Smart Wireless Device",
            "F99 MAX Dongle ", "2.4G Dongle", ""] {
            let mut wrong = i.clone(); wrong.product = product.into(); assert!(!control(&wrong));
        }
        for (vid, pid, page, usage, input, output) in [
            (0x0c45, 0x800a, 0xff60, 0x61, 33, 33), (0x0c45, 0xfefe, 0xff60, 0x61, 33, 33),
            (0x05ac, 0x024f, 0xff59, 0x61, 65, 65), (0x05ac, 0x024f, 0xffff, 1, 33, 33),
            (0x05ac, 0x024f, 1, 6, 33, 33), (0x05ac, 0x024f, 0xff60, 0x62, 33, 33),
            (0x05ac, 0x024f, 0xff60, 0x61, 32, 33), (0x05ac, 0x024f, 0xff60, 0x61, 33, 32),
        ] {
            let mut wrong = i.clone(); wrong.vendor_id = vid; wrong.product_id = pid;
            wrong.usage_page = page; wrong.usage = usage; wrong.input_len = input;
            wrong.output_len = output; assert!(!control(&wrong));
        }
        for path in [i.path.replace("mi_03", "mi_01"), i.path.replace("mi_03", "mi_030"),
            i.path.replace("pid_024f", "pid_027f"), i.path.replace("vid_05ac", "vid_0c45"),
            i.path.replace("8&one&0001", ""), "invalid".into(), String::new()] {
            let mut wrong = i.clone(); wrong.path = path; assert!(!control(&wrong));
        }
        let mut upper = i.clone(); upper.path.make_ascii_uppercase(); assert!(control(&upper));
    }

    #[test]
    fn duplicate_management_collection_blocks_queries_and_instances_remain_independent() {
        let first = interface("one");
        let second = interface("two");
        assert_eq!(candidates(&[first.clone(), second.clone()]).len(), 2);
        assert_ne!(target(&first).key, target(&second).key);
        assert!(!target(&first).same_channel(&target(&second)));
        let mut sibling = first.clone();
        sibling.path = sibling.path.replace("col01", "col02").replace("&0001#", "&0002#");
        sibling.product = "Smart Wireless Device".into();
        assert!(candidates(&[first, sibling]).is_empty());
    }

    #[test]
    fn foreign_identity_sleep_or_invalid_first_page_stops_before_the_second_write() {
        let t = target(&interface("one"));
        let capture = captured()[0].clone();
        let mut invalid = vec![vec![], vec![0; 32], request(0).unwrap()[1..].to_vec()];
        for length in 0..PAYLOAD_BYTES { invalid.push(capture[..length].to_vec()); }
        let mut long = capture.clone(); long.push(0); invalid.push(long);
        for (offset, value) in [(0, 0xaa), (1, 0x13), (2, 0x10), (3, 0x18), (4, 1),
            (6, 1), (12, 0xac), (13, 5), (14, 0x4f), (15, 2), (24, 1),
            (25, 0), (25, 101), (26, 2), (26, 255)] {
            let mut wrong = capture.clone(); wrong[offset] = value; invalid.push(wrong);
        }
        for r in invalid {
            let mut calls = 0;
            assert!(read_with(&t, |_, page| { calls += 1; assert_eq!(page, 0); Some(r.clone()) }).is_err());
            assert_eq!(calls, 1);
        }
    }

    #[test]
    fn second_page_must_finish_with_its_own_address_and_final_packet_marker() {
        let t = target(&interface("one"));
        for (offset, value) in [(0, 0xaa), (1, 0x13), (2, 0x10), (3, 0), (4, 1), (6, 0)] {
            let mut r = captured(); r[1][offset] = value;
            assert!(sample(&t, r).is_err());
        }
        for length in 0..PAYLOAD_BYTES {
            let mut r = captured(); r[1].truncate(length);
            assert!(sample(&t, r).is_err());
        }
        let mut calls = 0;
        assert!(read_with(&t, |_, page| {
            calls += 1; if page == 0 { Some(captured()[0].clone()) } else { None }
        }).is_err());
        assert_eq!(calls, 2);
        let mut wrong = t; wrong.backend = BackendId::Aula;
        assert!(read_with(&wrong, |_, _| panic!("wrong backend must not query")).is_err());
    }

    #[test]
    fn incomplete_exchange_never_renews_a_previous_battery_sample() {
        let t = target(&interface("one"));
        let initial = sample(&t, captured()).unwrap();
        let now = initial.battery_at.unwrap();
        let lifetime = Duration::from_secs(120);
        let mut status = status::Status::default();
        status.observe_sample(Some(true), initial.battery.as_ref(), initial.battery_at, now, lifetime);
        for seconds in [10, 119, 120] {
            assert!(read_with(&t, |_, page|
                if page == 0 { Some(captured()[0].clone()) } else { None }).is_err());
            status.observe_sample(None, None, None, now + Duration::from_secs(seconds), lifetime);
            assert_eq!(status.measured_at, Some(now));
        }
        assert_eq!(status.quality, crate::device::ReadingQuality::Stale);
        let mut r = captured(); r[0][25] = 67;
        let recovered = sample(&t, r).unwrap();
        let later = now + Duration::from_secs(130);
        status.observe_sample(Some(true), recovered.battery.as_ref(), Some(later), later, lifetime);
        assert_eq!(status.quality, crate::device::ReadingQuality::Live);
        assert_eq!(status.battery.unwrap().percent, Some(67));
    }
}
