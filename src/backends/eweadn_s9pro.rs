//! Read-only EWEADN S9 PRO 8K receiver battery query.
//!
//! The receiver is selected only when a runtime descriptor independently proves
//! Feature report `03` is 64 bytes on one of the two reviewed vendor pages.
//! No UUID, wireless-online, reset, settings, pairing or firmware command is
//! reachable from this backend.

use super::*;
use std::time::{Duration, Instant};

pub struct EweadnS9Pro;

const REPORT_ID: u8 = 0x03;
const REPORT_BYTES: usize = 64;
const TRANSFER_TIMEOUT_MS: u32 = 600;
const RECEIVE_DELAY_MS: u64 = 100;

fn supported(interface: &HidInterface) -> bool {
    interface.vendor_id == 0x372e
        && interface.product_id == 0x1026
        && interface.usage == 1
        && matches!(interface.usage_page, 0xff00 | 0xff06)
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    supported(interface)
        && descriptor::Descriptor::open(api, interface).is_some_and(|caps| {
            caps.feature_byte_report(interface, REPORT_ID, REPORT_BYTES as u16)
        })
}

fn candidates<'a>(api: &HidApi, interfaces: &'a [HidInterface]) -> Vec<&'a HidInterface> {
    interfaces
        .iter()
        .filter(|interface| {
            proven(api, interface)
                && interfaces
                    .iter()
                    .filter(|other| {
                        proven(api, other)
                            && channel::collection_group(interface)
                                == channel::collection_group(other)
                    })
                    .count()
                    == 1
        })
        .collect()
}

fn request() -> [u8; REPORT_BYTES] {
    let mut frame = [0; REPORT_BYTES];
    frame[..7].copy_from_slice(&[REPORT_ID, 0x20, 0x50, 0, 0, 0x4f, 0x81]);
    frame
}

fn battery(reply: &[u8]) -> Option<Battery> {
    let (level, charging) = if reply.len() == 16 {
        if reply[0] != REPORT_ID
            || reply[1] != 0x50
            || reply[2] != 0
            || reply[5] != 1
        {
            return None;
        }
        let level = reply[6] & 0x7f;
        if !(1..=100).contains(&level) {
            return None;
        }
        (level, reply[6] & 0x80 != 0)
    } else if reply.len() == REPORT_BYTES {
        if reply[0] != REPORT_ID
            || reply[2] != 0
            || reply[4] != 0
            || !(7..=REPORT_BYTES as u8 - 6).contains(&reply[3])
        {
            return None;
        }
        let level = reply[6] & 0x7f;
        if level > 100 {
            return None;
        }
        (level, reply[6] & 0x80 != 0)
    } else {
        return None;
    };
    Some(Battery {
        percent: Some(level),
        estimated_percent: None,
        charging: Some(charging),
        voltage_mv: None,
        approximate: false,
        level: None,
    })
}

fn target(interface: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::EweadnS9Pro, interface, 0);
    target.name = "EWEADN S9 PRO 8K".into();
    target.family = DeviceFamily::Mouse;
    target
}

fn read_with(
    target: &Target,
    mut exchange: impl FnMut(&[u8], u8, u32) -> Result<Vec<u8>, String>,
) -> Result<Snapshot, String> {
    if !supported(target.hid()?) {
        return Err(crate::i18n::translate("设备无有效状态回报").into());
    }
    let reply = exchange(
        &request(),
        REPORT_ID,
        TRANSFER_TIMEOUT_MS,
    )?;
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

impl DeviceBackend for EweadnS9Pro {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(api, interfaces)
            .into_iter()
            .map(target)
            .collect())
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let interface = target.hid()?;
        if target.backend != BackendId::EweadnS9Pro || !proven(api, interface) {
            return Err(crate::i18n::translate("设备无有效状态回报").into());
        }
        let lock = channel::lock(&channel::collection_group(interface));
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        read_with(target, |request, report_id, timeout| {
            let started = Instant::now();
            let result = (|| {
                let mut channel =
                    feature::FeatureChannel::open_with_length(interface, REPORT_BYTES)?;
                channel.send(request, timeout)?;
                std::thread::sleep(Duration::from_millis(RECEIVE_DELAY_MS));
                channel.receive(report_id, timeout)
            })();
            trace::record(
                interface,
                BackendId::EweadnS9Pro,
                "query-battery",
                u16::from(REPORT_ID),
                0x4f,
                REPORT_BYTES,
                result.as_ref().map_or(0, Vec::len),
                started,
                match result.as_deref() {
                    Ok(reply) if battery(reply).is_some() => trace::Outcome::Ok,
                    Ok(_) => trace::Outcome::InvalidData,
                    Err(_) => trace::Outcome::NoReply,
                },
            );
            result
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(page: u16, instance: &str) -> HidInterface {
        HidInterface {
            path: format!(
                r"\\?\hid#vid_372e&pid_1026&mi_01&col02#8&{instance}&0002#{{hid}}"
            ),
            vendor_id: 0x372e,
            product_id: 0x1026,
            usage_page: page,
            usage: 1,
            input_len: 0,
            output_len: 0,
            feature_len: 64,
            serial: "shared".into(),
            product: "EWEADN S9".into(),
        }
    }

    fn full_reply(level: u8, charging: bool) -> Vec<u8> {
        let mut frame = [0; REPORT_BYTES];
        frame[0] = REPORT_ID;
        frame[1] = 0x50;
        frame[3] = 7;
        frame[4] = 0;
        frame[5] = 0x81;
        frame[6] = level | if charging { 0x80 } else { 0 };
        frame.to_vec()
    }

    fn short_reply(level: u8, charging: bool) -> Vec<u8> {
        let mut frame = vec![
            0x03, 0x50, 0, 0x0b, 0, 1, 0, 0x21, 0x07, 1, 0x90, 0x65, 0x04, 0xa5, 0x5a, 0,
        ];
        frame[6] = level | if charging { 0x80 } else { 0 };
        frame
    }

    #[test]
    fn exact_request_contains_the_published_basics_command_and_checksum() {
        assert_eq!(
            request(),
            [
                0x03, 0x20, 0x50, 0, 0, 0x4f, 0x81, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ]
        );
        assert_eq!(request()[1], request()[2..].iter().fold(0u8, |sum, b| sum.wrapping_add(*b)));
    }

    #[test]
    fn short_and_full_hardware_formats_decode_percentage_and_charging() {
        for (level, charging) in [(0, false), (0, true), (73, false), (100, true)] {
            let full = battery(&full_reply(level, charging)).unwrap();
            assert_eq!(
                (full.percent, full.charging),
                (Some(level), Some(charging))
            );
        }
        for (level, charging) in [(1, false), (23, false), (98, true), (100, false)] {
            let short = battery(&short_reply(level, charging)).unwrap();
            assert_eq!(
                (short.percent, short.charging),
                (Some(level), Some(charging))
            );
        }
    }

    #[test]
    fn malformed_short_full_and_empty_packets_are_rejected() {
        let short = short_reply(73, false);
        for (offset, values) in [
            (0, vec![0, 2, 9]),
            (1, vec![0, 0x20, 0x51]),
            (2, vec![1, 0xff]),
            (5, vec![0, 2, 0x35]),
            (6, vec![0, 0x80, 101, 127, 0xe5]),
        ] {
            for value in values {
                let mut frame = short.clone();
                frame[offset] = value;
                assert!(battery(&frame).is_none(), "{frame:?}");
            }
        }
        let full = full_reply(73, false);
        for (offset, values) in [
            (0, vec![0, 2]),
            (2, vec![1, 0xff]),
            (3, vec![0, 6, 59, 255]),
            (4, vec![1, 0xff]),
            (6, vec![101, 127, 0xe5, 0xff]),
        ] {
            for value in values {
                let mut frame = full.clone();
                frame[offset] = value;
                assert!(battery(&frame).is_none(), "{frame:?}");
            }
        }
        for frame in [vec![], vec![0; 16], vec![0; 64], request().to_vec()] {
            assert!(battery(&frame).is_none());
        }
    }

    #[test]
    fn only_exact_vendor_pid_and_reviewed_vendor_collections_are_eligible() {
        assert!(supported(&interface(0xff00, "one")));
        assert!(supported(&interface(0xff06, "one")));
        for (vid, pid, page, usage) in [
            (0x089d, 0x1026, 0xff00, 1),
            (0x372e, 0x103d, 0xff00, 1),
            (0x372e, 0x106e, 0xff00, 1),
            (0x372e, 0x1029, 0xff00, 1),
            (0x372e, 0x1026, 1, 2),
            (0x372e, 0x1026, 0xff60, 1),
            (0x372e, 0x1026, 0xff00, 2),
        ] {
            let mut wrong = interface(page, "one");
            wrong.vendor_id = vid;
            wrong.product_id = pid;
            wrong.usage = usage;
            assert!(!supported(&wrong));
        }
        let target_one = target(&interface(0xff00, "one"));
        assert_eq!(target_one.name, "EWEADN S9 PRO 8K");
        assert_eq!(target_one.family, DeviceFamily::Mouse);
        assert!(!target_one.verified_identity);
        assert_ne!(
            target_one.key,
            target(&interface(0xff00, "two")).key
        );
    }

    #[test]
    fn one_forward_then_receive_exchange_creates_a_snapshot() {
        let target = target(&interface(0xff00, "one"));
        let mut calls = 0;
        let reading = read_with(&target, |frame, id, timeout| {
            calls += 1;
            assert_eq!(frame, request());
            assert_eq!((id, timeout), (REPORT_ID, TRANSFER_TIMEOUT_MS));
            Ok(short_reply(73, true))
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(
            reading.battery.as_ref().and_then(|battery| battery.percent),
            Some(73)
        );
        assert_eq!(
            reading.battery.as_ref().and_then(|battery| battery.charging),
            Some(true)
        );
        assert!(reading.connected && !reading.battery_unavailable);
        assert!(read_with(&target, |_, _, _| Ok(vec![0; 64])).is_err());
        assert!(read_with(&target, |_, _, _| Err("io".into())).is_err());
    }
}
