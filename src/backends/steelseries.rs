//! Read-only Arctis/Nova B0 status. Model-specific layouts are documented in docs/PROTOCOLS.md.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct Nova;

const BATTERY_QUERY: [u8; 2] = [0, 0xb0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout { Nova5, Nova7Percent, Nova7Level, Arctis7Plus }

fn model(pid: u16) -> Option<(&'static str, Layout)> {
    use Layout::*;
    Some(match pid {
        0x220e => ("Arctis 7+", Arctis7Plus),
        0x2212 => ("Arctis 7P+", Arctis7Plus),
        0x2216 => ("Arctis 7X+", Arctis7Plus),
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
    let model = model(i.product_id)?;
    if model.1 == Layout::Arctis7Plus {
        let path = i.path.to_ascii_lowercase();
        let hardware = path.split('#').nth(1)?;
        let mut interfaces = hardware.split('&').filter(|field| field.starts_with("mi_"));
        if interfaces.next() != Some("mi_03") || interfaces.next().is_some() { return None; }
    }
    Some(model)
}

fn confirmed(i: &HidInterface, mut prove_reports: impl FnMut(&HidInterface) -> bool)
    -> Option<(&'static str, Layout)> {
    let model = control(i)?;
    if model.1 == Layout::Arctis7Plus && !prove_reports(i) { return None; }
    Some(model)
}

fn unnumbered_reports(api: &HidApi, i: &HidInterface) -> bool {
    descriptor::Descriptor::open(api, i)
        .is_some_and(|caps| caps.only_input_output_report(i, 0, 65))
}

fn candidates(interfaces: &[HidInterface], mut prove_reports: impl FnMut(&HidInterface) -> bool) -> Vec<Target> {
    interfaces.iter().filter_map(|i| {
        let (name, _) = confirmed(i, &mut prove_reports)?;
        let mut target = Target::new(BackendId::Nova, i, 0);
        target.name = name.into();
        target.family = DeviceFamily::Headset;
        Some(target)
    }).collect()
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
        Layout::Arctis7Plus => {
            match body[1] { 1 => return Some(Status::Offline), 0 | 2 | 3 => (), _ => return None }
            (body[2], match body[3] { 0 => Some(false), 1 => Some(true), _ => None })
        }
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
    let approximate = matches!(layout, Layout::Nova7Level | Layout::Arctis7Plus);
    let (percent, level) = if approximate {
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
        estimated_percent: approximate.then(|| raw * 25), charging, voltage_mv: None,
        approximate, level }))
}

fn snapshot(target: &Target, status: Status) -> Snapshot {
    let (connected, battery) = match status {
        Status::Offline => (false, None), Status::Online(battery) => (true, Some(battery)),
    };
    Snapshot { target: target.clone(), battery_unavailable: false, battery_at: battery.as_ref().map(|_| Instant::now()),
        battery, connected, dpi: Dpi::default(), polling_rate: Default::default() }
}

fn response_snapshot(target: &Target, response: Option<&[u8]>, layout: Layout) -> Result<Snapshot, String> {
    let status = response.and_then(|frame| status(frame, layout))
        .ok_or_else(|| crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试"))?;
    Ok(snapshot(target, status))
}

impl DeviceBackend for Nova {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces, |i| unnumbered_reports(api, i)))
    }
    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let (_, layout) = confirmed(target.hid()?, |i| unnumbered_reports(api, i))
            .ok_or_else(|| crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试"))?;
        let lock = channel::lock(&target.hid()?.path);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(target.hid()?)?;
        let started = Instant::now();
        let response = device.transact(&BATTERY_QUERY, 800, |r| status(r, layout).is_some());
        trace::record(target.hid()?, BackendId::Nova, "query-battery", 0, 0xb0,
            usize::from(target.hid()?.output_len), response.as_ref().map_or(0, Vec::len), started,
            if response.is_some() { trace::Outcome::Ok } else { trace::Outcome::NoReply });
        response_snapshot(target, response.as_deref(), layout)
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

    fn plus_interface(pid: u16, instance: &str) -> HidInterface {
        HidInterface {
            path: format!(r"\\?\HID#VID_1038&PID_{pid:04X}&MI_03#{instance}#{{hid}}"),
            product_id: pid,
            ..interface()
        }
    }

    #[test]
    fn arctis_plus_models_require_the_proven_unnumbered_control_collection() {
        for (pid, name) in [(0x220e, "Arctis 7+"), (0x2212, "Arctis 7P+"), (0x2216, "Arctis 7X+")] {
            let interface = plus_interface(pid, "receiver-a");
            assert_eq!(control(&interface), Some((name, Layout::Arctis7Plus)));
            assert!(candidates(&[interface.clone()], |_| false).is_empty());
            let targets = candidates(&[interface.clone()], |_| true);
            assert_eq!(targets.len(), 1);
            assert_eq!(targets[0].name, name);
            assert_eq!(targets[0].family, DeviceFamily::Headset);
            assert_eq!(targets[0].backend, BackendId::Nova);
            for mutate in [
                |i: &mut HidInterface| i.vendor_id = 0x046d,
                |i: &mut HidInterface| i.product_id = 0x220c,
                |i: &mut HidInterface| i.product_id = 0x2236,
                |i: &mut HidInterface| i.usage_page = 0xff00,
                |i: &mut HidInterface| i.usage = 2,
                |i: &mut HidInterface| i.input_len = 64,
                |i: &mut HidInterface| i.output_len = 64,
                |i: &mut HidInterface| i.output_len = 0,
                |i: &mut HidInterface| i.path = i.path.replace("MI_03", "MI_00"),
                |i: &mut HidInterface| i.path = i.path.replace("MI_03", "MI_03&MI_05"),
                |i: &mut HidInterface| i.path = "headset-a".into(),
            ] {
                let mut wrong = interface.clone(); mutate(&mut wrong);
                assert!(candidates(&[wrong], |_| panic!("unrelated interface must not be opened")).is_empty());
            }
        }
        assert_eq!(candidates(&[interface()], |_| false).len(), 1);
    }

    #[test]
    fn arctis_plus_levels_and_connection_states_do_not_reuse_nova7_semantics() {
        assert_eq!(BATTERY_QUERY, [0, 0xb0]);
        for pid in [0x220e, 0x2212, 0x2216] {
            let (_, layout) = model(pid).unwrap();
            for (raw, level) in [BatteryLevel::Critical, BatteryLevel::Low, BatteryLevel::Medium,
                BatteryLevel::Good, BatteryLevel::Full].into_iter().enumerate() {
                for connection in [0, 2, 3] {
                    for charger in [0, 1, 2, 0xff] {
                        let reply = frame(&[0, 0xb0, connection, raw as u8, charger, 100, 100]);
                        let reading = battery(&reply, layout);
                        assert_eq!((reading.percent, reading.level), (None, Some(level)));
                        assert_eq!(reading.estimated_percent, Some(raw as u8 * 25));
                        assert_eq!(reading.display_percent().unwrap().label(true), format!("≈{}%", raw * 25));
                        assert_eq!(reading.charging, match charger { 0 => Some(false), 1 => Some(true), _ => None });
                        assert!(reading.approximate);
                        assert_eq!(status(&reply, layout), status(&reply[1..], layout));
                    }
                }
            }
            assert_eq!(status(&frame(&[0, 0xb0, 1, 4, 1]), layout), Some(Status::Offline));
        }
        assert!(matches!(status(&frame(&[0, 0xb0, 3, 4, 0]), Layout::Arctis7Plus), Some(Status::Online(_))));
        assert_eq!(status(&frame(&[0, 0xb0, 3, 4, 0]), Layout::Nova7Level), Some(Status::Offline));
    }

    #[test]
    fn arctis_plus_truncation_echoes_and_unknown_values_never_become_measurements() {
        let layout = Layout::Arctis7Plus;
        let valid = frame(&[0, 0xb0, 3, 4, 1, 100, 100]);
        for length in 0..65 { assert!(status(&valid[..length], layout).is_none()); }
        let mut oversized = valid.clone(); oversized.push(0);
        assert!(status(&oversized, layout).is_none());
        for prefix in [&[0, 0xb0][..], &[0, 0x20, 3, 4, 1], &[1, 0xb0, 3, 4, 1],
            &[0, 0, 0xb0, 3, 4, 1], &[0, 0xb9, 3, 4, 1]] {
            assert!(status(&frame(prefix), layout).is_none());
        }
        for invalid in 5..=255 { assert!(status(&frame(&[0, 0xb0, 3, invalid, 1]), layout).is_none()); }
        for invalid in 4..=255 { assert!(status(&frame(&[0, 0xb0, invalid, 4, 1]), layout).is_none()); }
    }

    #[test]
    fn arctis_plus_discovery_keeps_identical_receivers_and_readings_separate() {
        for pid in [0x220e, 0x2212, 0x2216] {
            let first = plus_interface(pid, "receiver-a");
            let second = plus_interface(pid, "receiver-b");
            for interfaces in [[first.clone(), second.clone()], [second.clone(), first.clone()]] {
                let targets = candidates(&interfaces, |_| true);
                assert_eq!(targets.len(), 2);
                assert_ne!(targets[0].key, targets[1].key);
                assert!(!targets[0].same_channel(&targets[1]));
                for target in targets {
                    let raw = if target.hid().unwrap().path == first.path { 1 } else { 4 };
                    let reading = response_snapshot(&target, Some(&frame(&[0, 0xb0, 3, raw, 0])), Layout::Arctis7Plus).unwrap();
                    assert_eq!(reading.target.key, target.key);
                    assert_eq!(reading.battery.unwrap().estimated_percent, Some(raw * 25));
                    assert!(!reading.dpi.writable);
                    assert!(!reading.polling_rate.writable);
                }
            }
        }
    }

    #[test]
    fn arctis_plus_errors_and_offline_reports_do_not_renew_or_zero_old_measurements() {
        let target = candidates(&[plus_interface(0x220e, "receiver-a")], |_| true).remove(0);
        let layout = Layout::Arctis7Plus;
        let initial = response_snapshot(&target, Some(&frame(&[0, 0xb0, 3, 3, 0])), layout).unwrap();
        let measured = initial.battery_at.unwrap();
        let lifetime = BackendId::Nova.sample_lifetime();
        let mut state = super::super::status::Status::default();
        state.observe_sample(Some(true), initial.battery.as_ref(), Some(measured), measured, lifetime);
        let echo = frame(&BATTERY_QUERY);
        let invalid = frame(&[0, 0xb0, 3, 5, 0]);
        for response in [None, Some(echo.as_slice()), Some(invalid.as_slice())] {
            let failed = response_snapshot(&target, response, layout);
            assert_eq!(failed.unwrap_err(), crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试"));
            state.observe_sample(None, None, None, measured + lifetime, lifetime);
            assert_eq!(state.measured_at, Some(measured));
            assert_eq!(state.battery.as_ref().unwrap().estimated_percent, Some(75));
            assert_eq!(state.quality, crate::device::ReadingQuality::Stale);
            let mut unknown = super::super::status::Status::default();
            unknown.observe_sample(None, None, None, measured, lifetime);
            assert!(unknown.battery.is_none());
            assert_eq!(unknown.quality, crate::device::ReadingQuality::Unknown);
        }
        let offline = response_snapshot(&target, Some(&frame(&[0, 0xb0, 1, 4, 1])), layout).unwrap();
        assert!(!offline.connected);
        assert!(offline.battery.is_none());
        assert!(offline.battery_at.is_none());
        state.observe_sample(Some(false), None, offline.battery_at, measured + lifetime, lifetime);
        assert_eq!(state.measured_at, Some(measured));
        assert_eq!(state.battery.as_ref().unwrap().estimated_percent, Some(75));
        let wake = response_snapshot(&target, Some(&frame(&[0, 0xb0, 2, 2, 0])), layout).unwrap();
        let received = measured + lifetime + std::time::Duration::from_secs(1);
        state.observe_sample(Some(wake.connected), wake.battery.as_ref(), Some(received), received, lifetime);
        assert_eq!(state.measured_at, Some(received));
        assert_eq!(state.battery.unwrap().estimated_percent, Some(50));
        assert_eq!(state.quality, crate::device::ReadingQuality::Live);
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
