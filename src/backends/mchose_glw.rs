//! Read-only MCHOSE G87 V2 GLW configuration battery query.
//!
//! The PID is shared with other keyboards, so the exact product name, vendor
//! collection, 65-byte capacities and sole descriptor report IDs are mandatory.
//! Only base/profile reads are reachable; reset, repair and write commands are
//! deliberately absent.

use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct MchoseGlw;

const PRODUCT: &str = "MCHOSE G87 V2 2.4G";
const REPORT_BYTES: u16 = 65;
const PAYLOAD_BYTES: usize = 64;
const READ_TIMEOUT_MS: u32 = 1_000;

fn identity(interface: &HidInterface) -> bool {
    interface.vendor_id == 0x3837
        && interface.product_id == 0x3033
        && interface.product == PRODUCT
        && interface.usage_page == 1
        && interface.usage == 0
        && interface.input_len == REPORT_BYTES
        && interface.output_len == REPORT_BYTES
}

fn input_report_id(api: &HidApi, interface: &HidInterface) -> Option<u8> {
    if !identity(interface) {
        return None;
    }
    let (input, output) = descriptor::Descriptor::open(api, interface)?
        .single_input_output_report_ids(interface, REPORT_BYTES, REPORT_BYTES)?;
    (output == 0).then_some(input)
}

fn candidates<'a>(api: &HidApi, interfaces: &'a [HidInterface]) -> Vec<&'a HidInterface> {
    interfaces
        .iter()
        .filter(|interface| {
            input_report_id(api, interface).is_some()
                && interfaces
                    .iter()
                    .filter(|other| {
                        input_report_id(api, other).is_some()
                            && channel::collection_group(interface)
                                == channel::collection_group(other)
                    })
                    .count()
                    == 1
        })
        .collect()
}

fn payload(command: u8, offset: u16, length: u8) -> [u8; PAYLOAD_BYTES] {
    let mut frame = [0; PAYLOAD_BYTES];
    frame[0] = 0x55;
    frame[1] = command;
    frame[4] = length;
    frame[5..7].copy_from_slice(&offset.to_le_bytes());
    frame[3] = frame[4..]
        .iter()
        .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    frame
}

fn request(command: u8, offset: u16, length: u8) -> [u8; REPORT_BYTES as usize] {
    let mut frame = [0; REPORT_BYTES as usize];
    frame[1..].copy_from_slice(&payload(command, offset, length));
    frame
}

fn response_payload(frame: &[u8], input_report_id: u8) -> Option<&[u8]> {
    if input_report_id == 0 {
        (frame.len() == PAYLOAD_BYTES).then_some(frame)
    } else {
        (frame.len() == REPORT_BYTES as usize && frame.first() == Some(&input_report_id))
            .then_some(&frame[1..])
    }
}

fn response(
    frame: &[u8],
    input_report_id: u8,
    command: u8,
    offset: u16,
    length: u8,
) -> Option<Vec<u8>> {
    let reply = response_payload(frame, input_report_id)?;
    if reply.len() != PAYLOAD_BYTES
        || reply[0] != 0xaa
        || reply[1] != command
        || reply[4] != length
        || reply[5..7] != offset.to_le_bytes()
    {
        return None;
    }
    Some(reply[8..8 + usize::from(length)].to_vec())
}

fn active_profile(base: &[u8]) -> Option<usize> {
    if base.len() != 56 || base[0] > 3 {
        return None;
    }
    let index = usize::from(base[0]);
    let slots = &base[2..6];
    let mut sorted = [0; 4];
    sorted.copy_from_slice(slots);
    sorted.sort_unstable();
    if sorted == [0, 1, 2, 3] {
        Some(usize::from(slots[index]))
    } else {
        Some(index)
    }
}

fn target(interface: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::MchoseGlw, interface, 0);
    target.name = PRODUCT.into();
    target.family = DeviceFamily::Keyboard;
    target.verified_identity = true;
    target
}

fn read_with(
    target: &Target,
    mut exchange: impl FnMut(&[u8], u8, u16, u8) -> Option<Vec<u8>>,
) -> Result<Snapshot, String> {
    let fail = || crate::i18n::translate("设备无有效状态回报").to_string();
    let base = exchange(
        &request(4, 0, 56),
        4,
        0,
        56,
    )
    .ok_or_else(fail)?;
    let profile = active_profile(&base).ok_or_else(fail)?;
    let offset = (profile * 64) as u16;
    let first = exchange(
        &request(5, offset, 56),
        5,
        offset,
        56,
    )
    .ok_or_else(fail)?;
    let tail = exchange(
        &request(5, offset + 56, 8),
        5,
        offset + 56,
        8,
    )
    .ok_or_else(fail)?;
    let mut config = first;
    config.extend(tail);
    if config.len() != 64 {
        return Err(fail());
    }
    let raw = config[32];
    let level = raw & 0x7f;
    if level > 100 {
        return Err(fail());
    }
    Ok(Snapshot {
        target: target.clone(),
        battery: Some(Battery {
            percent: Some(level),
            estimated_percent: None,
            charging: Some(raw & 0x80 != 0),
            voltage_mv: None,
            approximate: false,
            level: None,
        }),
        battery_at: Some(Instant::now()),
        battery_unavailable: false,
        connected: true,
        dpi: Dpi::default(),
        polling_rate: Default::default(),
    })
}

fn query(
    device: &mut HidDevice,
    interface: &HidInterface,
    input_id: u8,
    packet: [u8; REPORT_BYTES as usize],
    command: u8,
    offset: u16,
    length: u8,
) -> Option<Vec<u8>> {
    let started = Instant::now();
    let frame = device.transact_with_input_report(
        &packet,
        input_id,
        READ_TIMEOUT_MS,
        |frame| response(frame, input_id, command, offset, length).is_some(),
    )?;
    let data = response(&frame, input_id, command, offset, length);
    trace::record(
        interface,
        BackendId::MchoseGlw,
        if command == 4 { "query-base" } else { "query-profile" },
        u16::from(input_id),
        command,
        REPORT_BYTES as usize,
        frame.len(),
        started,
        if data.is_some() {
            trace::Outcome::Ok
        } else {
            trace::Outcome::InvalidData
        },
    );
    data
}

impl DeviceBackend for MchoseGlw {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(api, interfaces)
            .into_iter()
            .map(target)
            .collect())
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let interface = target.hid()?;
        if target.backend != BackendId::MchoseGlw {
            return Err(crate::i18n::translate("设备无有效状态回报").into());
        }
        let input_id = input_report_id(api, interface)
            .ok_or_else(|| crate::i18n::translate("设备无有效状态回报").to_string())?;
        let lock = channel::lock(&channel::collection_group(interface));
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut device = HidDevice::open(interface)?;
        read_with(target, |packet, command, offset, length| {
            query(
                &mut device,
                interface,
                input_id,
                packet.try_into().ok()?,
                command,
                offset,
                length,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface() -> HidInterface {
        HidInterface {
            path: r"\\?\hid#vid_3837&pid_3033&mi_01&col01#8&unit-a&0000#{hid}".into(),
            vendor_id: 0x3837,
            product_id: 0x3033,
            usage_page: 1,
            usage: 0,
            input_len: 65,
            output_len: 65,
            feature_len: 0,
            serial: "shared".into(),
            product: PRODUCT.into(),
        }
    }

    fn reply(
        input_id: u8,
        command: u8,
        offset: u16,
        data: &[u8],
    ) -> Vec<u8> {
        let mut payload = [0u8; 64];
        payload[0] = 0xaa;
        payload[1] = command;
        payload[4] = data.len() as u8;
        payload[5..7].copy_from_slice(&offset.to_le_bytes());
        payload[8..8 + data.len()].copy_from_slice(data);
        let mut frame = Vec::with_capacity(if input_id == 0 { 64 } else { 65 });
        if input_id != 0 {
            frame.push(input_id);
        }
        frame.extend(payload);
        frame
    }

    fn frames(input_id: u8, level: u8, charging: bool) -> Vec<Vec<u8>> {
        let base = [0, 0, 0, 1, 2, 3];
        let mut base = base.into_iter().chain([0; 50]).collect::<Vec<_>>();
        base.resize(56, 0);
        let mut config = vec![0; 64];
        config[32] = level | if charging { 0x80 } else { 0 };
        vec![
            reply(input_id, 4, 0, &base),
            reply(input_id, 5, 0, &config[..56]),
            reply(input_id, 5, 56, &config[56..]),
        ]
    }

    #[test]
    fn exact_three_requests_use_only_read_commands_and_payload_checksum() {
        let base = request(4, 0, 56);
        let first = request(5, 0, 56);
        let tail = request(5, 56, 8);
        assert_eq!(&base[..9], &[0, 0x55, 4, 0, 56, 56, 0, 0, 0]);
        assert_eq!(&first[..9], &[0, 0x55, 5, 0, 56, 56, 0, 0, 0]);
        assert_eq!(&tail[..9], &[0, 0x55, 5, 0, 64, 8, 56, 0, 0]);
        for packet in [base, first, tail] {
            assert_eq!(
                packet[4],
                packet[5..]
                    .iter()
                    .fold(0u8, |sum, byte| sum.wrapping_add(*byte))
            );
            assert_eq!(packet.len(), 65);
        }
        assert!(!matches!(request(4, 0, 56)[2], 6 | 7 | 0x30 | 0x35 | 0x3a | 0x3b));
    }

    #[test]
    fn active_profile_permutation_and_legacy_index_are_preserved() {
        let mut base = vec![0; 56];
        base[0] = 2;
        base[2..6].copy_from_slice(&[3, 1, 0, 2]);
        assert_eq!(active_profile(&base), Some(0));
        base[2..6].copy_from_slice(&[0, 0, 0, 0]);
        assert_eq!(active_profile(&base), Some(2));
        base[0] = 4;
        assert_eq!(active_profile(&base), None);
        assert_eq!(active_profile(&[0; 55]), None);
    }

    #[test]
    fn zero_and_nonzero_input_report_ids_are_parsed_without_shifting_data() {
        for input_id in [0, 7] {
            let frame = reply(input_id, 4, 0, &[1, 2, 3, 4]);
            assert_eq!(
                response(&frame, input_id, 4, 0, 4),
                Some(vec![1, 2, 3, 4])
            );
            assert!(response(&frame, input_id ^ 1, 4, 0, 4).is_none());
        }
        assert!(response(&[0; 63], 0, 4, 0, 56).is_none());
        assert!(response(&[7; 65], 7, 4, 0, 56).is_none());
    }

    #[test]
    fn malformed_headers_lengths_offsets_and_levels_are_rejected() {
        let good = reply(0, 4, 0, &[0; 56]);
        for (offset, values) in [
            (0, vec![0, 0x55, 0xff]),
            (1, vec![0, 5, 0xff]),
            (4, vec![0, 7, 55, 57]),
            (5, vec![1, 0xff]),
            (6, vec![1, 0xff]),
        ] {
            for value in values {
                let mut frame = good.clone();
                frame[offset] = value;
                assert!(response(&frame, 0, 4, 0, 56).is_none());
            }
        }
        for level in [0, 1, 73, 100] {
            for charging in [false, true] {
                let target = target(&interface());
                let mut sent = Vec::new();
                let reading = read_with(&target, |packet, command, offset, length| {
                    sent.push((command, offset, length, packet[5], packet[4]));
                    let all = frames(0, level, charging);
                    Some(response(
                        &all[sent.len() - 1],
                        0,
                        command,
                        offset,
                        length,
                    )?)
                })
                .unwrap();
                assert_eq!(
                    reading.battery.as_ref().and_then(|battery| battery.percent),
                    Some(level)
                );
                assert_eq!(
                    reading.battery.as_ref().and_then(|battery| battery.charging),
                    Some(charging)
                );
                assert_eq!(
                    sent,
                    [(4, 0, 56, 56, 56), (5, 0, 56, 56, 56), (5, 56, 8, 8, 64)]
                );
            }
        }
        for level in [101, 127] {
            let target = target(&interface());
            let mut all = frames(0, level, false);
            let mut count = 0;
            assert!(read_with(&target, |_, command, offset, length| {
                let index = count;
                count += 1;
                Some(response(&all.remove(index.min(2)), 0, command, offset, length)?)
            })
            .is_err());
        }
    }

    #[test]
    fn exact_pid_product_collection_and_capacities_are_required() {
        let valid = interface();
        assert!(identity(&valid));
        for (field, values) in [
            ("vid", vec![0x5253, 0x41e4]),
            ("pid", vec![0x100b, 0x201b, 0x9999]),
            ("product", vec![0x0]),
        ] {
            for value in values {
                let mut wrong = valid.clone();
                match field {
                    "vid" => wrong.vendor_id = value as u16,
                    "pid" => wrong.product_id = value as u16,
                    _ => wrong.product = "MCHOSE K99 V3 2.4G".into(),
                }
                assert!(!identity(&wrong));
            }
        }
        let mut wrong = valid.clone();
        wrong.usage_page = 0xff31;
        assert!(!identity(&wrong));
        wrong = valid.clone();
        wrong.usage = 6;
        assert!(!identity(&wrong));
        wrong = valid.clone();
        wrong.input_len = 64;
        assert!(!identity(&wrong));
        wrong = valid.clone();
        wrong.output_len = 64;
        assert!(!identity(&wrong));
        assert_ne!(target(&valid).key, target(&interface().with_path("second")).key);
    }

    trait PathForTest {
        fn with_path(&self, suffix: &str) -> Self;
    }

    impl PathForTest for HidInterface {
        fn with_path(&self, suffix: &str) -> Self {
            let mut interface = self.clone();
            interface.path.push_str(suffix);
            interface
        }
    }
}
