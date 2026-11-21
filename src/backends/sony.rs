//! DualSense battery input reports. No Output or SetFeature commands are sent.
use super::*;
use crate::hid::HidDevice;
use std::time::{Duration, Instant};

pub struct DualSense;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transport { Usb, Bluetooth }

impl Transport {
    fn report_id(self) -> u8 { match self { Self::Usb => 0x01, Self::Bluetooth => 0x31 } }
    fn length(self) -> usize { match self { Self::Usb => 64, Self::Bluetooth => 78 } }
}

fn control(i: &HidInterface) -> Option<(&'static str, Transport)> {
    if i.vendor_id != 0x054c || i.usage_page != 1 || i.usage != 5 { return None; }
    let name = match i.product_id {
        0x0ce6 => "Sony DualSense", 0x0df2 => "Sony DualSense Edge", _ => return None,
    };
    let transport = match i.input_len { 64 => Transport::Usb, 78 => Transport::Bluetooth, _ => return None };
    Some((name, transport))
}

fn crc32(seed: u8, bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in std::iter::once(&seed).chain(bytes) {
        crc ^= u32::from(*byte);
        for _ in 0..8 { crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1)); }
    }
    !crc
}

fn valid_crc(seed: u8, frame: &[u8]) -> bool {
    let Some(offset) = frame.len().checked_sub(4) else { return false; };
    crc32(seed, &frame[..offset]) == u32::from_le_bytes(frame[offset..].try_into().unwrap())
}

#[derive(Debug, PartialEq, Eq)]
struct Report { battery: Option<Battery> }

fn status(frame: &[u8], transport: Transport) -> Option<Report> {
    if frame.len() != transport.length() || frame[0] != transport.report_id() { return None; }
    if transport == Transport::Bluetooth && !valid_crc(0xa1, frame) { return None; }
    let status = frame[if transport == Transport::Usb { 53 } else { 54 }];
    let raw = status & 0x0f;
    let charging = status >> 4;
    let estimated = match charging {
        0 | 1 if raw <= 10 => (raw * 10 + 5).min(100),
        2 => 100,
        // These report an electrical / temperature / charging fault, not empty charge.
        0xa | 0xb | 0xf => return Some(Report { battery: None }),
        _ => return None,
    };
    let level = match estimated {
        0..=9 => BatteryLevel::Critical, 10..=20 => BatteryLevel::Low,
        21..=50 => BatteryLevel::Medium, 51..=99 => BatteryLevel::Good,
        _ => BatteryLevel::Full,
    };
    Some(Report { battery: Some(Battery { percent: None, estimated_percent: Some(estimated),
        charging: Some(charging == 1), voltage_mv: None, approximate: true, level: Some(level) }) })
}

#[derive(Default)]
struct InputBatch {
    report: Option<Report>,
    bytes: usize,
    minimal: bool,
}

fn receive_status(transport: Transport, timeout: u32, stop_on_minimal: bool,
    mut receive: impl FnMut(u32) -> Option<Vec<u8>>) -> InputBatch {
    let deadline = Instant::now() + Duration::from_millis(u64::from(timeout));
    let mut batch = InputBatch::default();
    // Bound both elapsed time and a stream of immediately available invalid reports.
    for _ in 0..64 {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else { break; };
        let Some(frame) = receive(left.as_millis().max(1) as u32) else { break; };
        batch.bytes = frame.len();
        if transport == Transport::Bluetooth && frame.first() == Some(&1) {
            // Windows can pad the short Bluetooth control report to the descriptor size.
            // It has no battery field, even when the padded length is 78 bytes.
            batch.minimal = true;
            if stop_on_minimal { break; }
            continue;
        }
        batch.report = status(&frame, transport);
        if batch.report.is_some() { break; }
    }
    batch
}

fn calibration_valid(frame: &[u8]) -> bool {
    frame.len() == 41 && frame[0] == 5 && valid_crc(0xa3, frame)
}

fn snapshot(target: &Target, report: Report, received: Instant) -> Snapshot {
    Snapshot { target: target.clone(), connected: true,
        battery_unavailable: false, battery_at: report.battery.as_ref().map(|_| received), battery: report.battery,
        dpi: Dpi::default(), polling_rate: Default::default() }
}

impl DeviceBackend for DualSense {
    fn discover(&self, _: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(interfaces.iter().filter_map(|i| {
            let (name, _) = control(i)?;
            let mut target = Target::new(BackendId::DualSense, i, 0);
            target.name = name.into();
            target.family = DeviceFamily::Gamepad;
            Some(target)
        }).collect())
    }

    fn read(&self, _: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let (_, transport) = control(target.hid()?)
            .ok_or_else(|| crate::i18n::translate("设备无有效状态回报"))?;
        let lock = channel::lock(&target.hid()?.path);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open_read_only(target.hid()?)?;
        let started = Instant::now();
        let mut batch = receive_status(transport, 300, true, |timeout| device.read_report(timeout));
        if batch.minimal {
            // A GET of calibration data requests extended Bluetooth reports. Do this
            // only after observing minimal mode; never send rumble, LED or setup output.
            let feature_started = Instant::now();
            let calibration = feature::FeatureChannel::open_with_length(target.hid()?, 41)
                .and_then(|mut channel| channel.receive(5, 250));
            trace::record(target.hid()?, BackendId::DualSense, "get-calibration", 0, 5,
                41, calibration.as_ref().map_or(0, Vec::len), feature_started, match &calibration {
                    Ok(frame) if calibration_valid(frame) => trace::Outcome::Ok,
                    Ok(_) => trace::Outcome::InvalidData, Err(_) => trace::Outcome::NoReply,
                });
            // Input reports are independently checked even if the feature read fails.
            // Skip queued minimal frames while waiting for the first extended report.
            batch = receive_status(transport, 300, false, |timeout| device.read_report(timeout));
        }
        let outcome = match &batch.report {
            Some(Report { battery: Some(_) }) => trace::Outcome::Ok,
            Some(_) => trace::Outcome::InvalidData,
            None if batch.bytes > 0 => trace::Outcome::InvalidData,
            None => trace::Outcome::NoReply,
        };
        trace::record(target.hid()?, BackendId::DualSense, "input-battery", 0,
            transport.report_id(), 0, batch.bytes, started, outcome);
        let report = batch.report.ok_or_else(|| crate::i18n::translate("设备无有效状态回报"))?;
        Ok(snapshot(target, report, Instant::now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(transport: Transport, power: u8) -> Vec<u8> {
        let mut r = vec![0; transport.length()];
        r[0] = transport.report_id();
        r[1] = 128; // resting axis / Bluetooth report header
        r[if transport == Transport::Usb { 53 } else { 54 }] = power;
        if transport == Transport::Bluetooth {
            let offset = r.len() - 4;
            let checksum = crc32(0xa1, &r[..offset]);
            r[offset..].copy_from_slice(&checksum.to_le_bytes());
        }
        r
    }

    fn interface() -> HidInterface {
        HidInterface { path: "controller-a".into(), vendor_id: 0x054c, product_id: 0x0ce6,
            usage_page: 1, usage: 5, input_len: 64, output_len: 63, feature_len: 64,
            product: "Wireless Controller".into(), serial: "shared".into() }
    }

    #[test]
    fn usb_and_bluetooth_ranges_remain_estimates_and_full_is_not_charging() {
        for transport in [Transport::Usb, Transport::Bluetooth] {
            for raw in 0..=10 {
                for charging in [0, 1] {
                    let b = status(&frame(transport, charging * 16 + raw), transport).unwrap().battery.unwrap();
                    assert_eq!(b.percent, None);
                    assert_eq!(b.charging, Some(charging == 1));
                    assert_eq!(b.display_percent().unwrap().label(true), format!("≈{}%", (raw * 10 + 5).min(100)));
                }
            }
            let b = status(&frame(transport, 0x20), transport).unwrap().battery.unwrap();
            assert_eq!(b.estimated_percent, Some(100));
            assert_eq!(b.charging, Some(false));
            for error in [0xa0, 0xb5, 0xff] {
                assert_eq!(status(&frame(transport, error), transport).unwrap().battery, None);
            }
            for invalid in [0x0b, 0x0f, 0x1b, 0x1f, 0x35, 0xc0] {
                assert!(status(&frame(transport, invalid), transport).is_none());
            }
        }
    }

    #[test]
    fn lengths_transport_ids_and_bluetooth_crc_are_strict() {
        for transport in [Transport::Usb, Transport::Bluetooth] {
            let valid = frame(transport, 0x15);
            for len in 0..valid.len() { assert!(status(&valid[..len], transport).is_none()); }
            let mut oversized = valid.clone(); oversized.push(0);
            assert!(status(&oversized, transport).is_none());
            let mut wrong_id = valid.clone(); wrong_id[0] ^= 0x10;
            assert!(status(&wrong_id, transport).is_none());
        }
        assert!(status(&frame(Transport::Usb, 5), Transport::Bluetooth).is_none());
        let mut bt = frame(Transport::Bluetooth, 5);
        bt[54] ^= 1;
        assert!(status(&bt, Transport::Bluetooth).is_none());
        let crc = crc32(0xa3, &bt[..74]); bt[74..].copy_from_slice(&crc.to_le_bytes());
        assert!(status(&bt, Transport::Bluetooth).is_none());
        let mut padded = vec![0; 78]; padded[0] = 1; padded[53] = 5;
        assert!(status(&padded, Transport::Bluetooth).is_none());
    }

    #[test]
    fn crc_matches_standard_vector_and_feature_seed_is_separate() {
        assert_eq!(crc32(b'1', b"23456789"), 0xcbf4_3926);
        let mut feature = vec![0; 41]; feature[0] = 5;
        let checksum = crc32(0xa3, &feature[..37]);
        feature[37..].copy_from_slice(&checksum.to_le_bytes());
        assert!(calibration_valid(&feature));
        assert!(!valid_crc(0xa1, &feature));
        feature[9] ^= 1;
        assert!(!calibration_valid(&feature));
        assert!(!calibration_valid(&feature[..40]));
    }

    #[test]
    fn receiver_skips_corruption_bounds_noise_and_detects_minimal_mode() {
        let valid = frame(Transport::Bluetooth, 5);
        let mut corrupt = valid.clone(); corrupt[54] ^= 1;
        let mut queue = [vec![0x55; 78], corrupt, valid].into_iter();
        let batch = receive_status(Transport::Bluetooth, 300, true, |_| queue.next());
        assert_eq!(batch.report.unwrap().battery.unwrap().estimated_percent, Some(55));
        let mut calls = 0;
        let batch = receive_status(Transport::Bluetooth, 300, true, |_| { calls += 1; Some(vec![0; 78]) });
        assert!(batch.report.is_none()); assert_eq!(calls, 64);
        let batch = receive_status(Transport::Bluetooth, 300, true, |_| Some(vec![1; 78]));
        assert!(batch.minimal); assert!(batch.report.is_none());
        let mut queue = [vec![1; 78], frame(Transport::Bluetooth, 4)].into_iter();
        let batch = receive_status(Transport::Bluetooth, 300, false, |_| queue.next());
        assert!(batch.minimal);
        assert_eq!(batch.report.unwrap().battery.unwrap().estimated_percent, Some(45));
        assert!(receive_status(Transport::Usb, 300, true, |_| None).report.is_none());
    }

    #[test]
    fn catalog_restricts_model_and_collection_and_separates_controller_instances() {
        let i = interface();
        assert_eq!(control(&i), Some(("Sony DualSense", Transport::Usb)));
        let mut other = i.clone(); other.product_id = 0x0df2; other.input_len = 78;
        assert_eq!(control(&other), Some(("Sony DualSense Edge", Transport::Bluetooth)));
        for (vid, pid, page, usage, length) in [(0x054c, 0x05c4, 1, 5, 64),
            (0x054c, 0x0ce6, 1, 4, 64), (0x054c, 0x0ce6, 0xff00, 5, 64),
            (0x054c, 0x0ce6, 1, 5, 65), (0x1234, 0x0ce6, 1, 5, 64)] {
            let mut wrong = i.clone(); wrong.vendor_id = vid; wrong.product_id = pid;
            wrong.usage_page = page; wrong.usage = usage; wrong.input_len = length;
            assert!(control(&wrong).is_none());
        }
        other = i.clone(); other.path = "controller-b".into();
        let first = Target::new(BackendId::DualSense, &i, 0);
        let second = Target::new(BackendId::DualSense, &other, 0);
        assert_ne!(first.key, second.key); assert!(!first.same_channel(&second));
    }

    #[test]
    fn silence_and_faults_do_not_renew_battery_and_wake_recovers() {
        let now = Instant::now();
        let lifetime = BackendId::DualSense.sample_lifetime();
        let target = Target::new(BackendId::DualSense, &interface(), 0);
        let mut state = super::super::status::Status::default();
        for (seconds, raw, expected, quality) in [(0, 5, 55, crate::device::ReadingQuality::Live),
            (2, 0xa0, 55, crate::device::ReadingQuality::Cached),
            (8, 0xf0, 55, crate::device::ReadingQuality::Stale),
            (9, 4, 45, crate::device::ReadingQuality::Live)] {
            let at = now + Duration::from_secs(seconds);
            let reading = snapshot(&target, status(&frame(Transport::Usb, raw), Transport::Usb).unwrap(), at);
            assert_eq!(reading.battery.is_some(), reading.battery_at.is_some());
            state.observe_sample(Some(reading.connected), reading.battery.as_ref(), reading.battery_at, at, lifetime);
            assert!(state.connected); assert_eq!(state.quality, quality);
            assert_eq!(state.battery.as_ref().unwrap().estimated_percent, Some(expected));
            assert_eq!(state.measured_at, Some(if seconds < 9 { now } else { at }));
        }
        state.observe(None, None, now + Duration::from_secs(17), lifetime);
        assert_eq!(state.quality, crate::device::ReadingQuality::Stale);
        state.observe(Some(false), None, now + Duration::from_secs(18), lifetime);
        assert!(!state.connected);
    }
}
