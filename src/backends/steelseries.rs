//! Read-only Nova B0 status. Model-specific layouts are documented in PROTOCOLS.md.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct Nova;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout { Nova5, Nova7Percent, Nova7Level }

fn model(pid: u16) -> Option<(&'static str, Layout)> {
    use Layout::*;
    Some(match pid {
        0x2232 => ("Arctis Nova 5", Nova5),
        0x2253 => ("Arctis Nova 5X", Nova5),
        0x2269 => ("Arctis Nova 3P Wireless", Nova5),
        0x226d => ("Arctis Nova 3X Wireless", Nova5),
        0x2202 => ("Arctis Nova 7", Nova7Level),
        0x2206 | 0x22a4 => ("Arctis Nova 7X", Nova7Level),
        0x220a => ("Arctis Nova 7P", Nova7Level),
        0x223a => ("Arctis Nova 7 Diablo IV", Nova7Level),
        0x227a => ("Arctis Nova 7 WoW Edition", Nova7Level),
        0x22a1 => ("Arctis Nova 7", Nova7Percent),
        0x227e => ("Arctis Nova 7 Gen 2", Nova7Percent),
        0x2258 | 0x229e | 0x22ad | 0x22a5 => ("Arctis Nova 7X", Nova7Percent),
        0x22a9 => ("Arctis Nova 7 Diablo IV", Nova7Percent),
        0x22a7 | 0x2298 => ("Arctis Nova 7P", Nova7Percent),
        _ => return None,
    })
}

fn control(i: &HidInterface) -> Option<(&'static str, Layout)> {
    if i.vendor_id != 0x1038 || i.usage_page != 0xffc0 || i.usage != 1
        || i.input_len != 65 || i.output_len != 65 { return None; }
    model(i.product_id)
}

#[derive(Debug, PartialEq, Eq)]
enum Status { Offline, Online(Battery) }

fn status(frame: &[u8], layout: Layout) -> Option<Status> {
    // Windows includes report ID 0; hidapi captures may omit it. Strip it once.
    let body = match frame {
        [0, rest @ ..] if frame.len() == 65 => rest,
        _ if frame.len() == 64 => frame,
        _ => return None,
    };
    if body[0] != 0xb0 || body[1..].iter().all(|byte| *byte == 0) {
        // A padded query is indistinguishable from an all-zero offline reply.
        // Leave that case to sample expiry instead of claiming disconnection.
        return None;
    }
    let (raw, charging) = match layout {
        Layout::Nova5 => {
            match body[1] { 2 => return Some(Status::Offline), 3 => (), _ => return None }
            (body[3], match body[4] { 1 => Some(true), 3 => Some(false), _ => None })
        }
        Layout::Nova7Level | Layout::Nova7Percent => {
            let charging = match body[3] {
                0 => return Some(Status::Offline),
                1 | 2 => Some(true),
                3 => Some(false),
                _ => return None,
            };
            (body[2], charging)
        }
    };
    let (percent, level) = if layout == Layout::Nova7Level {
        let level = match raw {
            0 => BatteryLevel::Critical, 1 => BatteryLevel::Low, 2 => BatteryLevel::Medium,
            3 => BatteryLevel::Good, 4 => BatteryLevel::Full, _ => return None,
        };
        (None, Some(level))
    } else {
        if raw > 100 { return None; }
        (Some(raw), None)
    };
    Some(Status::Online(Battery { percent,
        estimated_percent: (layout == Layout::Nova7Level).then(|| raw * 25), charging, voltage_mv: None,
        approximate: layout == Layout::Nova7Level, level }))
}

fn snapshot(target: &Target, status: Status) -> Snapshot {
    let (connected, battery) = match status {
        Status::Offline => (false, None), Status::Online(battery) => (true, Some(battery)),
    };
    Snapshot { target: target.clone(), battery_unavailable: false, battery_at: battery.as_ref().map(|_| Instant::now()),
        battery, connected, dpi: Dpi::default(), polling_rate: Default::default() }
}

impl DeviceBackend for Nova {
    fn discover(&self, _: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(interfaces.iter().filter_map(|i| {
            let (name, _) = control(i)?;
            let mut target = Target::new(BackendId::Nova, i, 0);
            target.name = name.into();
            target.family = DeviceFamily::Headset;
            Some(target)
        }).collect())
    }
    fn read(&self, _: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let (_, layout) = control(target.hid()?)
            .ok_or_else(|| crate::i18n::translate("设备无有效状态回报"))?;
        let lock = channel::lock(&target.hid()?.path);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(target.hid()?)?;
        let started = Instant::now();
        let response = device.transact(&[0, 0xb0], 800, |r| status(r, layout).is_some());
        trace::record(target.hid()?, BackendId::Nova, "query-battery", 0, 0xb0,
            usize::from(target.hid()?.output_len), response.as_ref().map_or(0, Vec::len), started,
            if response.is_some() { trace::Outcome::Ok } else { trace::Outcome::NoReply });
        let status = response.as_deref().and_then(|r| status(r, layout))
            .ok_or_else(|| crate::i18n::translate("设备无有效状态回报"))?;
        Ok(snapshot(target, status))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(prefix: &[u8]) -> Vec<u8> {
        let mut r = vec![0; 65]; r[..prefix.len()].copy_from_slice(prefix); r
    }
    fn battery(frame: &[u8], layout: Layout) -> Battery {
        match status(frame, layout).unwrap() {
            Status::Online(battery) => battery, Status::Offline => panic!("expected online"),
        }
    }
    fn interface() -> HidInterface {
        HidInterface { path: "headset-a".into(), vendor_id: 0x1038, product_id: 0x2232,
            usage_page: 0xffc0, usage: 1, input_len: 65, output_len: 65, feature_len: 0,
            product: String::new(), serial: "shared".into() }
    }

    #[test]
    fn nova5_windows_vectors_cover_full_charging_offline_and_optional_report_zero() {
        let full = frame(&[0, 0xb0, 3, 2, 100, 3]);
        let b = battery(&full, Layout::Nova5);
        assert_eq!((b.percent, b.charging), (Some(100), Some(false)));
        assert_eq!(status(&full, Layout::Nova5), status(&full[1..], Layout::Nova5));
        let b = battery(&frame(&[0, 0xb0, 3, 2, 42, 1]), Layout::Nova5);
        assert_eq!((b.percent, b.charging), (Some(42), Some(true)));
        assert_eq!(status(&frame(&[0, 0xb0, 2, 2, 100, 3]), Layout::Nova5), Some(Status::Offline));
        let b = battery(&frame(&[0, 0xb0, 3, 2, 0, 0xff]), Layout::Nova5);
        assert_eq!((b.percent, b.charging), (Some(0), None));
    }

    #[test]
    fn nova7_protocol_selected_by_pid_preserves_low_exact_values_and_five_levels() {
        for pid in [0x2202, 0x2206, 0x220a, 0x223a, 0x227a, 0x22a4] {
            let (_, layout) = model(pid).unwrap();
            for (raw, level) in [BatteryLevel::Critical, BatteryLevel::Low, BatteryLevel::Medium,
                BatteryLevel::Good, BatteryLevel::Full].into_iter().enumerate() {
                let b = battery(&frame(&[0, 0xb0, 0, raw as u8, 3]), layout);
                assert_eq!((b.percent, b.level), (None, Some(level)));
                assert_eq!(b.display_percent().unwrap().label(true), format!("≈{}%", raw * 25));
                assert!(b.approximate);
            }
            assert!(status(&frame(&[0, 0xb0, 0, 5, 3]), layout).is_none());
        }
        for pid in [0x22a1, 0x227e, 0x2258, 0x229e, 0x22ad, 0x22a5, 0x22a9, 0x22a7, 0x2298] {
            let (_, layout) = model(pid).unwrap();
            for raw in 0..=100 {
                let b = battery(&frame(&[0, 0xb0, 0, raw, 3]), layout);
                assert_eq!((b.percent, b.level), (Some(raw), None));
                assert!(!b.approximate);
            }
            for state in [1, 2] {
                assert_eq!(battery(&frame(&[0, 0xb0, 0, 42, state]), layout).charging, Some(true));
            }
            assert_eq!(status(&frame(&[0, 0xb0, 0, 42, 0]), layout), Some(Status::Offline));
        }
    }

    #[test]
    fn echoes_unknown_states_bad_headers_invalid_values_and_truncation_are_ignored() {
        for layout in [Layout::Nova5, Layout::Nova7Level, Layout::Nova7Percent] {
            assert!(status(&frame(&[0, 0xb0]), layout).is_none());
            assert!(status(&frame(&[0, 0x20, 3, 42, 3]), layout).is_none());
            assert!(status(&frame(&[0, 0xb0, 0, 42, 0xff]), layout).is_none());
            let valid = frame(&[0, 0xb0, 3, 2, 42, 1]);
            for len in 0..65 { assert!(status(&valid[..len], layout).is_none()); }
        }
        assert!(status(&frame(&[0, 0xb0, 3, 2, 101, 3]), Layout::Nova5).is_none());
        assert!(status(&frame(&[0, 0xb0, 0, 255, 3]), Layout::Nova7Percent).is_none());
    }

    #[test]
    fn catalog_excludes_other_protocols_and_preserves_identical_receiver_instances() {
        let i = interface();
        assert!(control(&i).is_some());
        for pid in [0x2269, 0x226d] {
            let (_, layout) = model(pid).unwrap(); assert_eq!(layout, Layout::Nova5);
        }
        let mut other = i.clone(); other.product_id = 0x2210;
        assert!(control(&other).is_none());
        other = i.clone(); other.usage = 2;
        assert!(control(&other).is_none());
        other = i.clone(); other.input_len = 64;
        assert!(control(&other).is_none());
        other = i.clone(); other.path = "headset-b".into();
        let first = Target::new(BackendId::Nova, &i, 0);
        let second = Target::new(BackendId::Nova, &other, 0);
        assert_ne!(first.key, second.key);
        assert!(!first.same_channel(&second));
    }

    #[test]
    fn offline_and_wake_refresh_shared_status_without_reusing_stale_battery() {
        let target = Target::new(BackendId::Nova, &interface(), 0);
        let now = Instant::now();
        let lifetime = BackendId::Nova.sample_lifetime();
        let mut state = super::super::status::Status::default();
        for (second, reply, connected, expected) in [
            (0, frame(&[0, 0xb0, 3, 2, 60, 3]), true, 60),
            (1, frame(&[0, 0xb0, 2, 2, 60, 3]), false, 60),
            (2, frame(&[0, 0xb0, 3, 2, 59, 3]), true, 59),
        ] {
            let reading = snapshot(&target, status(&reply, Layout::Nova5).unwrap());
            assert_eq!(reading.connected, connected);
            assert_eq!(reading.battery_at.is_some(), connected);
            let at = now + std::time::Duration::from_secs(second);
            state.observe(Some(reading.connected), reading.battery.as_ref(), at, lifetime);
            assert_eq!(state.connected, connected);
            assert_eq!(state.battery.as_ref().unwrap().percent, Some(expected));
            assert_eq!(state.quality, if connected { crate::device::ReadingQuality::Live }
                else { crate::device::ReadingQuality::Stale });
        }
    }
}
