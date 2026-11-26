//! INZONE receiver battery queries. No headset setting commands are sent.
use super::*;
use crate::hid::HidDevice;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Instant;

pub struct Inzone;

fn control(i: &HidInterface) -> Option<&'static str> {
    if i.vendor_id != 0x054c || i.usage_page != 0xff04
        || i.input_len != 64 || i.output_len != 64 { return None; }
    match (i.product_id, i.usage) {
        (0x0ebf, 2) => Some("Sony INZONE H5"),
        (0x0fa8, 1) if i.path.to_ascii_lowercase().split(['#', '&']).any(|p| p == "mi_05") =>
            Some("Sony INZONE H9 II"),
        _ => None,
    }
}

fn next_transaction(counter: &AtomicU16) -> u16 {
    // TID 1 belongs to unsolicited receiver notifications; 0 is reserved.
    loop {
        let next = counter.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        if next > 1 { return next; }
    }
}

fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte))
}

fn battery_query(transaction: u16) -> [u8; 64] {
    let mut packet = [0; 64];
    packet[..11].copy_from_slice(&[2, 12, 1, 0, 0xfc, 8, 0x96, 0xc3, 0x41, 4, 1]);
    packet[11..13].copy_from_slice(&transaction.to_le_bytes());
    packet[13] = checksum(&packet[6..13]);
    packet
}

#[derive(Debug, PartialEq, Eq)]
enum Report { Offline, Online(Battery) }

fn battery_reply(frame: &[u8], transaction: u16) -> Option<Report> {
    // These two headsets return exactly two payload bytes. Buds use a different
    // component layout and must not be collapsed into this percentage field.
    if transaction < 2 || frame.len() != 64 || frame[..4] != [2, 14, 4, 0xff]
        || frame[5..10] != [0, 0x96, 0xc3, 0x14, 4]
        || !matches!(frame[10], 0x10 | 0xa0)
        || u16::from_le_bytes([frame[11], frame[12]]) != transaction
        || checksum(&frame[5..15]) != frame[15] { return None; }
    // The HCI parameter-length byte varies in captures; the enclosing length,
    // source, event, transaction and checksum establish the frame boundaries.
    let percent = match frame[14] {
        0xff => return Some(Report::Offline),
        value @ 0..=100 => value,
        _ => return None,
    };
    let charging = match frame[13] { 0 => Some(false), 1 => Some(true), _ => None };
    Some(Report::Online(Battery { percent: Some(percent), estimated_percent: None,
        charging, voltage_mv: None, approximate: false, level: None }))
}

fn snapshot(target: &Target, report: Report, received: Instant) -> Snapshot {
    let (connected, battery) = match report {
        Report::Offline => (false, None), Report::Online(battery) => (true, Some(battery)),
    };
    Snapshot { target: target.clone(), connected, battery_unavailable: !connected,
        battery_at: battery.as_ref().map(|_| received), battery,
        dpi: Dpi::default(), polling_rate: Default::default() }
}

impl DeviceBackend for Inzone {
    fn discover(&self, _: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(interfaces.iter().filter_map(|i| {
            let name = control(i)?;
            let mut target = Target::new(BackendId::Inzone, i, 0);
            target.name = name.into();
            target.family = DeviceFamily::Headset;
            Some(target)
        }).collect())
    }

    fn read(&self, _: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let interface = target.hid()?;
        control(interface).ok_or_else(|| crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试"))?;
        let lock = channel::lock(&interface.path);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(interface)?;
        static TRANSACTION: AtomicU16 = AtomicU16::new(1);
        let transaction = next_transaction(&TRANSACTION);
        let query = battery_query(transaction);
        let started = Instant::now();
        let response = device.transact(&query, 800, |r| battery_reply(r, transaction).is_some());
        trace::record(interface, BackendId::Inzone, "query-battery", 0, 4,
            query.len(), response.as_ref().map_or(0, Vec::len), started,
            if response.is_some() { trace::Outcome::Ok } else { trace::Outcome::NoReply });
        let report = response.as_deref().and_then(|r| battery_reply(r, transaction))
            .ok_or_else(|| crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试"))?;
        Ok(snapshot(target, report, Instant::now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(transaction: u16, charger: u8, percent: u8) -> [u8; 64] {
        let mut frame = [0; 64];
        frame[..11].copy_from_slice(&[2, 14, 4, 0xff, 11, 0, 0x96, 0xc3, 0x14, 4, 0x10]);
        frame[11..13].copy_from_slice(&transaction.to_le_bytes());
        frame[13] = charger; frame[14] = percent;
        frame[15] = checksum(&frame[5..15]);
        frame
    }

    fn interface() -> HidInterface {
        HidInterface { path: "headset-a".into(), vendor_id: 0x054c, product_id: 0x0ebf,
            usage_page: 0xff04, usage: 2, input_len: 64, output_len: 64, feature_len: 0,
            product: "USB Headset".into(), serial: "shared".into() }
    }

    #[test]
    fn queries_match_hci_wire_format_and_transactions_skip_reserved_values() {
        let query = battery_query(2);
        assert_eq!(&query[..14], &[2, 12, 1, 0, 0xfc, 8, 0x96, 0xc3, 0x41, 4, 1, 2, 0, 0xa1]);
        assert!(query[14..].iter().all(|b| *b == 0));
        assert_eq!(&battery_query(0x1234)[11..14], &[0x34, 0x12, 0xe5]);
        let counter = AtomicU16::new(u16::MAX - 1);
        assert_eq!(next_transaction(&counter), u16::MAX);
        assert_eq!(next_transaction(&counter), 2);
        assert_eq!(next_transaction(&counter), 3);
    }

    #[test]
    fn percentages_charging_and_offline_have_independent_meanings() {
        for percent in 0..=100 {
            for charger in [0, 1, 2, 0xff] {
                let Report::Online(battery) = battery_reply(&response(2, charger, percent), 2).unwrap()
                    else { panic!("expected online"); };
                assert_eq!(battery.percent, Some(percent));
                assert_eq!(battery.estimated_percent, None);
                assert_eq!(battery.charging, match charger { 0 => Some(false), 1 => Some(true), _ => None });
                assert_eq!(battery.display_percent().unwrap().label(true), format!("{percent}%"));
                assert!(!battery.approximate);
            }
        }
        for invalid in 101..=254 { assert!(battery_reply(&response(2, 1, invalid), 2).is_none()); }
        assert_eq!(battery_reply(&response(2, 0xff, 0xff), 2), Some(Report::Offline));
    }

    #[test]
    fn captured_notification_and_old_transactions_cannot_answer_a_query() {
        // Published H5 charger notification: [charger=1, percent=100], TID=1.
        let mut notification = [0; 64];
        notification[..16].copy_from_slice(&[2, 14, 4, 0xff, 11, 0, 0x96, 0xc3, 0x14, 4, 0xa0, 1, 0, 1, 100, 0x77]);
        assert_eq!(checksum(&notification[5..15]), 0x77);
        assert!(battery_reply(&notification, 1).is_none());
        assert!(battery_reply(&notification, 2).is_none());
        assert!(battery_reply(&response(0, 0, 50), 0).is_none());
        assert!(battery_reply(&response(2, 0, 50), 3).is_none());
        assert!(battery_reply(&battery_query(2), 2).is_none());
        notification[11] = 2; notification[15] = checksum(&notification[5..15]);
        assert!(matches!(battery_reply(&notification, 2), Some(Report::Online(_))));
    }

    #[test]
    fn unrelated_events_wrong_sources_bad_checksums_and_partial_reports_are_rejected() {
        let valid = response(0x1234, 0, 61);
        for length in 0..64 { assert!(battery_reply(&valid[..length], 0x1234).is_none()); }
        let mut long = valid.to_vec(); long.push(0);
        assert!(battery_reply(&long, 0x1234).is_none());
        for (index, byte) in [(0, 0), (1, 13), (1, 15), (2, 1), (3, 0), (5, 1),
            (6, 0), (7, 0), (8, 0x12), (8, 0x41), (9, 0x24), (10, 0x20), (11, 1), (12, 0)] {
            let mut corrupt = valid; corrupt[index] = byte;
            corrupt[15] = checksum(&corrupt[5..15]);
            assert!(battery_reply(&corrupt, 0x1234).is_none(), "field {index}");
        }
        let mut corrupt = valid; corrupt[15] ^= 1;
        assert!(battery_reply(&corrupt, 0x1234).is_none());
        let mut variant = valid; variant[4] = 0;
        assert_eq!(battery_reply(&variant, 0x1234), battery_reply(&valid, 0x1234));
    }

    #[test]
    fn model_specific_interfaces_exclude_buds_and_keep_same_model_instances_separate() {
        let h5 = interface();
        assert_eq!(control(&h5), Some("Sony INZONE H5"));
        let mut other = h5.clone(); other.usage = 1;
        assert!(control(&other).is_none());
        other.product_id = 0x0fa8;
        assert!(control(&other).is_none());
        other.path = "\\\\?\\HID#VID_054C&PID_0FA8&MI_05&COL03#headset-b".into();
        assert_eq!(control(&other), Some("Sony INZONE H9 II"));
        other.path = other.path.replace("MI_05", "MI_04");
        assert!(control(&other).is_none());
        for pid in [0x0ec2, 0x0ce6, 0x0e53] {
            other = h5.clone(); other.product_id = pid;
            assert!(control(&other).is_none());
        }
        other = h5.clone(); other.input_len = 65;
        assert!(control(&other).is_none());
        other = h5.clone(); other.output_len = 0;
        assert!(control(&other).is_none());
        other = h5.clone(); other.usage_page = 0xff13;
        assert!(control(&other).is_none());
        other = h5.clone(); other.path = "headset-b".into();
        let first = Target::new(BackendId::Inzone, &h5, 0);
        let second = Target::new(BackendId::Inzone, &other, 0);
        assert_ne!(first.key, second.key);
        assert!(!first.same_channel(&second));
    }

    #[test]
    fn offline_placeholder_clears_battery_without_recording_a_measurement() {
        let target = Target::new(BackendId::Inzone, &interface(), 0);
        let now = Instant::now();
        let offline = snapshot(&target, battery_reply(&response(2, 0xff, 0xff), 2).unwrap(), now);
        assert!(!offline.connected);
        assert!(offline.battery_unavailable);
        assert!(offline.battery.is_none());
        assert!(offline.battery_at.is_none());
        let wake = snapshot(&target, battery_reply(&response(3, 0, 51), 3).unwrap(), now);
        assert!(wake.connected);
        assert!(!wake.battery_unavailable);
        assert_eq!(wake.battery_at, Some(now));
        assert_eq!(wake.battery.unwrap().percent, Some(51));
        assert!(!wake.dpi.writable);
        assert!(!wake.polling_rate.writable);
    }
}
