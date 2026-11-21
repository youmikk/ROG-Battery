//! Read-only EWEADN S9 receiver using its isolated M156 output exchange.
//!
//! This is deliberately separate from the 372E:1026 S9 PRO Feature protocol.
//! Only the exact 089D:062F vendor collection and unnumbered bidirectional
//! reports are eligible; the descriptor-proven input capacity controls reads.

use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct EweadnS9;

const OUTPUT_BYTES: u16 = 33;
const READ_TIMEOUT_MS: u32 = 1_000;

fn identity(interface: &HidInterface) -> bool {
    interface.vendor_id == 0x089d
        && interface.product_id == 0x062f
        && interface.usage_page == 0xff01
        && interface.usage == 0x0010
        && (16..=34).contains(&interface.input_len)
        && interface.output_len == OUTPUT_BYTES
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    identity(interface)
        && descriptor::Descriptor::open(api, interface)
            .and_then(|caps| {
                caps.single_input_output_report_ids(
                    interface,
                    interface.input_len,
                    OUTPUT_BYTES,
                )
            })
            == Some((0, 0))
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

fn request() -> [u8; OUTPUT_BYTES as usize] {
    let mut frame = [0; OUTPUT_BYTES as usize];
    frame[1] = 0x30;
    frame
}

fn battery(reply: &[u8]) -> Option<Battery> {
    if !(15..=33).contains(&reply.len())
        || reply[..2] != [0x30, 0]
        || reply[3] < 11
        || usize::from(reply[3]) > reply.len().saturating_sub(4)
        || reply[4..8] != *b"M156"
        || reply[13] > 100
        || reply[14] != 1
    {
        return None;
    }
    Some(Battery {
        percent: Some(reply[13]),
        estimated_percent: None,
        charging: None,
        voltage_mv: None,
        approximate: false,
        level: None,
    })
}

fn target(interface: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::EweadnS9, interface, 0);
    target.name = "EWEADN S9".into();
    target.family = DeviceFamily::Mouse;
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

impl DeviceBackend for EweadnS9 {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(api, interfaces)
            .into_iter()
            .map(target)
            .collect())
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let interface = target.hid()?;
        if target.backend != BackendId::EweadnS9 || !proven(api, interface) {
            return Err(crate::i18n::translate("设备无有效状态回报").into());
        }
        let lock = channel::lock(&channel::collection_group(interface));
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut device = HidDevice::open(interface)?;
        read_with(target, |request, timeout| {
            let started = Instant::now();
            let response = device.transact_with_input_report(
                request,
                0,
                timeout,
                |reply| battery(reply).is_some(),
            );
            trace::record(
                interface,
                BackendId::EweadnS9,
                "query-battery",
                0,
                0x30,
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

    fn interface(size: u16) -> HidInterface {
        HidInterface {
            path: r"\\?\hid#vid_089d&pid_062f&mi_01&col01#8&unit-a&0000#{hid}".into(),
            vendor_id: 0x089d,
            product_id: 0x062f,
            usage_page: 0xff01,
            usage: 0x0010,
            input_len: size,
            output_len: 33,
            feature_len: 0,
            serial: "shared".into(),
            product: "EWEADN S9".into(),
        }
    }

    fn reply(level: u8) -> Vec<u8> {
        let mut frame = vec![0x30, 0, 1, 11];
        frame.extend_from_slice(b"M156");
        frame.extend([0, 0, 0, 0, 0, level, 1]);
        frame.resize(33, 0);
        frame
    }

    #[test]
    fn exact_request_and_m156_reply_preserve_unknown_charging() {
        let request = request();
        assert_eq!(request.len(), 33);
        assert_eq!(&request[..2], &[0, 0x30]);
        assert!(request[2..].iter().all(|byte| *byte == 0));
        for level in 0..=100 {
            let battery = battery(&reply(level)).unwrap();
            assert_eq!(battery.percent, Some(level));
            assert_eq!(battery.charging, None);
        }
    }

    #[test]
    fn descriptor_capacity_bounds_and_m156_fields_are_strict() {
        for size in 15..=33 {
            let mut frame = reply(60);
            frame.resize(size, 0);
            assert_eq!(battery(&frame).unwrap().percent, Some(60));
        }
        for size in [0, 1, 14, 34, 64] {
            let mut frame = reply(60);
            frame.resize(size, 0);
            assert!(battery(&frame).is_none(), "size {size}");
        }
        for (offset, values) in [
            (0, vec![0, 2, 0xd1, 0xff]),
            (1, vec![1, 2, 0xff]),
            (3, vec![0, 10, 30, 255]),
            (4, vec![0, b'N', 0xff]),
            (5, vec![0, b'0', 0xff]),
            (6, vec![0, b'4', 0xff]),
            (7, vec![0, b'8', 0xff]),
            (13, vec![101, 127, 255]),
            (14, vec![0, 2, 255]),
        ] {
            for value in values {
                let mut frame = reply(60);
                frame[offset] = value;
                assert!(battery(&frame).is_none(), "{frame:?}");
            }
        }
        for identity in [b"M302", b"m156", b"M15\0"] {
            let mut frame = reply(60);
            frame[4..8].copy_from_slice(identity);
            assert!(battery(&frame).is_none());
        }
        assert!(battery(&request()).is_none());
    }

    #[test]
    fn exact_vendor_pid_collection_and_capacities_are_required() {
        let valid = interface(33);
        assert!(identity(&valid));
        for (vid, pid, page, usage, input, output) in [
            (0x088d, 0x062e, 0xff01, 0x10, 33, 33),
            (0x089d, 0x062e, 0xff01, 0x10, 33, 33),
            (0x089d, 0x062f, 0xff00, 0, 33, 33),
            (0x089d, 0x062f, 1, 2, 33, 33),
            (0x089d, 0x062f, 0xff01, 0x10, 15, 33),
            (0x089d, 0x062f, 0xff01, 0x10, 35, 33),
            (0x089d, 0x062f, 0xff01, 0x10, 33, 32),
            (0x089d, 0x062f, 0xff01, 0x10, 33, 34),
        ] {
            let mut wrong = valid.clone();
            wrong.vendor_id = vid;
            wrong.product_id = pid;
            wrong.usage_page = page;
            wrong.usage = usage;
            wrong.input_len = input;
            wrong.output_len = output;
            assert!(!identity(&wrong));
        }
        assert_eq!(target(&valid).name, "EWEADN S9");
        assert_eq!(target(&valid).family, DeviceFamily::Mouse);
    }

    #[test]
    fn one_exchange_can_skip_invalid_input_and_publish_the_valid_reply() {
        let target = target(&interface(33));
        let mut calls = 0;
        let reading = read_with(&target, |frame, timeout| {
            calls += 1;
            assert_eq!(frame, &request());
            assert_eq!(timeout, READ_TIMEOUT_MS);
            Some(reply(60))
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(
            reading.battery.as_ref().and_then(|battery| battery.percent),
            Some(60)
        );
        assert!(read_with(&target, |_, _| Some(vec![0; 33])).is_err());
        assert!(read_with(&target, |_, _| None).is_err());
    }
}
