//! Read-only Flydigi APEX5 GET_INFO battery query.
//!
//! Only the exact controller collection and Report IDs are queried. The backend
//! has no IMU, rumble, trigger, lighting, mode or configuration operations.

use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct Flydigi;

const READ_TIMEOUT_MS: u32 = 1_000;

fn control(interface: &HidInterface) -> bool {
    interface.vendor_id == 0x37d7
        && interface.product_id == 0x2501
        && interface.usage_page == 0xffa0
        && interface.usage == 1
        && interface.input_len == 32
        && interface.output_len == 32
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces
        .iter()
        .filter(|interface| {
            control(interface)
                && interfaces
                    .iter()
                    .filter(|other| {
                        control(other)
                            && channel::collection_group(interface)
                                == channel::collection_group(other)
                    })
                    .count()
                    == 1
        })
        .collect()
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    control(interface)
        && descriptor::Descriptor::open(api, interface).is_some_and(|caps| {
            caps.only_reports(interface, 4, 32, 3, 32)
        })
}

fn request() -> [u8; 32] {
    let mut frame = [0; 32];
    frame[..6].copy_from_slice(&[3, 0x5a, 0xa5, 1, 2, 3]);
    frame
}

fn battery(reply: &[u8]) -> Option<Battery> {
    if reply.len() != 32
        || reply[..7] != [4, 0x5a, 0xa5, 1, 1, 0, 0x80]
        || !matches!(reply[7], 1 | 2)
    {
        return None;
    }
    let tier = reply[12] & 0x0f;
    let charging = reply[12] >> 4;
    if charging > 1 {
        return None;
    }
    if charging == 1 {
        // The vendor uses charging as a sentinel; it is not a sixth tier and
        // must not become a fabricated 120% or zero reading.
        return Some(Battery {
            percent: None,
            estimated_percent: None,
            charging: Some(true),
            voltage_mv: None,
            approximate: true,
            level: None,
        });
    }
    if tier > 5 {
        return None;
    }
    Some(Battery {
        percent: None,
        estimated_percent: Some(tier * 20),
        charging: Some(false),
        voltage_mv: None,
        approximate: true,
        level: None,
    })
}

fn target(interface: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::Flydigi, interface, 0);
    target.name = "Flydigi APEX5".into();
    target.family = DeviceFamily::Gamepad;
    target.verified_identity = true;
    target
}

fn read_with(
    target: &Target,
    mut exchange: impl FnMut(&[u8], u32) -> Option<Vec<u8>>,
) -> Result<Snapshot, String> {
    let reply = exchange(&request(), READ_TIMEOUT_MS)
        .ok_or_else(|| crate::i18n::translate("设备无有效状态回报").to_string())?;
    let battery = battery(&reply)
        .ok_or_else(|| crate::i18n::translate("设备无有效状态回报").to_string())?;
    Ok(Snapshot {
        target: target.clone(),
        battery: Some(battery),
        battery_at: Some(Instant::now()),
        battery_unavailable: false,
        connected: true,
        dpi: Dpi::default(),
        polling_rate: Default::default(),
    })
}

impl DeviceBackend for Flydigi {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces)
            .into_iter()
            .filter(|interface| proven(api, interface))
            .map(target)
            .collect())
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let interface = target.hid()?;
        if target.backend != BackendId::Flydigi || !proven(api, interface) {
            return Err(crate::i18n::translate("设备无有效状态回报").into());
        }
        let lock = channel::lock(&channel::collection_group(interface));
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut device = HidDevice::open(interface)?;
        read_with(target, |request, timeout| {
            let started = Instant::now();
            let response = device.transact(request, timeout, |reply| battery(reply).is_some());
            trace::record(
                interface,
                BackendId::Flydigi,
                "query-battery",
                4,
                3,
                request.len(),
                response.as_ref().map_or(0, Vec::len),
                started,
                if response.is_some() {
                    trace::Outcome::Ok
                } else {
                    trace::Outcome::NoReply
                },
            );
            response
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface() -> HidInterface {
        HidInterface {
            path: r"\\?\hid#vid_37d7&pid_2501&mi_01&col03#8&unit-a&0003#{hid}".into(),
            vendor_id: 0x37d7,
            product_id: 0x2501,
            usage_page: 0xffa0,
            usage: 1,
            input_len: 32,
            output_len: 32,
            feature_len: 0,
            serial: "shared".into(),
            product: "Flydigi APEX5".into(),
        }
    }

    fn reply(tier: u8, charging: u8) -> Vec<u8> {
        let mut frame = vec![4, 0x5a, 0xa5, 1, 1, 0, 0x80, 1];
        frame.resize(32, 0);
        frame[12] = tier | (charging << 4);
        frame
    }

    #[test]
    fn exact_get_info_request_and_five_estimated_levels() {
        assert_eq!(&request()[..8], &[3, 0x5a, 0xa5, 1, 2, 3, 0, 0]);
        assert!(request()[6..].iter().all(|byte| *byte == 0));
        for tier in 0..=5 {
            let battery = battery(&reply(tier, 0)).unwrap();
            assert_eq!(battery.percent, None);
            assert_eq!(battery.estimated_percent, Some(tier * 20));
            assert_eq!(battery.charging, Some(false));
            assert!(battery.approximate);
        }
    }

    #[test]
    fn charging_sentinel_never_becomes_a_sixth_percentage() {
        let battery = battery(&reply(6, 1)).unwrap();
        assert_eq!(battery.percent, None);
        assert_eq!(battery.estimated_percent, None);
        assert_eq!(battery.charging, Some(true));
    }

    #[test]
    fn malformed_headers_modes_flags_and_tiers_are_rejected() {
        let valid = reply(3, 0);
        for (offset, values) in [
            (0, vec![0, 3, 5, 0xff]),
            (1, vec![0, 0x59, 0xff]),
            (2, vec![0, 0xa4, 0xff]),
            (3, vec![0, 2, 0xff]),
            (4, vec![0, 2, 0xff]),
            (5, vec![1, 0xff]),
            (6, vec![0, 0x81, 0xff]),
            (7, vec![0, 3, 0xff]),
            (12, vec![0x06, 0x20, 0x25, 0xff]),
        ] {
            for value in values {
                let mut frame = valid.clone();
                frame[offset] = value;
                assert!(battery(&frame).is_none(), "{frame:?}");
            }
        }
        for frame in [
            vec![],
            valid[..31].to_vec(),
            [valid, vec![0]].concat(),
            request().to_vec(),
        ] {
            assert!(battery(&frame).is_none());
        }
    }

    #[test]
    fn exact_identity_collection_capacities_and_report_ids_are_required() {
        let valid = interface();
        assert!(control(&valid));
        for (vid, pid, page, usage, input, output) in [
            (0x37d7, 0x2502, 0xffa0, 1, 32, 32),
            (0x37d7, 0x2501, 0xffa1, 1, 32, 32),
            (0x37d7, 0x2501, 0xffa0, 2, 32, 32),
            (0x37d7, 0x2501, 0xffa0, 1, 31, 32),
            (0x37d7, 0x2501, 0xffa0, 1, 32, 31),
        ] {
            let mut wrong = valid.clone();
            wrong.vendor_id = vid;
            wrong.product_id = pid;
            wrong.usage_page = page;
            wrong.usage = usage;
            wrong.input_len = input;
            wrong.output_len = output;
            assert!(!control(&wrong));
        }
        assert_eq!(target(&valid).name, "Flydigi APEX5");
        assert_eq!(target(&valid).family, DeviceFamily::Gamepad);
    }

    #[test]
    fn one_request_returns_level_or_charging_without_extra_commands() {
        let target = target(&interface());
        let mut calls = 0;
        let reading = read_with(&target, |frame, timeout| {
            calls += 1;
            assert_eq!(frame, &request());
            assert_eq!(timeout, READ_TIMEOUT_MS);
            Some(reply(4, 0))
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(
            reading
                .battery
                .as_ref()
                .and_then(|battery| battery.display_percent())
                .map(|percent| (percent.value, percent.approximate)),
            Some((80, true))
        );
        assert!(read_with(&target, |_, _| Some(vec![0; 32])).is_err());
        assert!(read_with(&target, |_, _| None).is_err());
    }
}
