//! MCHOSE G98 V3 battery-only QHW command. See PROTOCOLS.md for evidence.
use super::*;
use crate::hid::HidDevice;
use std::{sync::atomic::{AtomicU8, Ordering}, time::Instant};

pub struct MchoseQhw;
static SEQUENCE: AtomicU8 = AtomicU8::new(0);

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x3837 && i.product_id == 0x1015 && i.usage_page == 0xff80 && i.usage == 1
}

fn control(i: &HidInterface) -> bool {
    management(i) && i.product == "MCHOSE G98 V3 2.4G"
        && (11..=4096).contains(&i.input_len) && (9..=4096).contains(&i.output_len)
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|i| control(i) && interfaces.iter().filter(|other|
        management(other) && channel::collection_group(i) == channel::collection_group(other)).count() == 1).collect()
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    control(i) && descriptor::Descriptor::open(api, i)
        .is_some_and(|caps| caps.only_input_output_report_lengths(i, 0x4d, i.input_len, i.output_len))
}

fn request(sequence: u8, output_len: u16) -> Vec<u8> {
    let mut frame = vec![0; usize::from(output_len)];
    frame[..9].copy_from_slice(&[0x4d, 1, 3, 0, 4, 9, 0, sequence, 0x0e ^ sequence]);
    frame
}

fn battery(reply: &[u8], sequence: u8, input_len: u16) -> Option<Battery> {
    if reply.len() != usize::from(input_len) || reply.len() < 11
        || reply[..8] != [0x4d, 1, 3, 2, 4, 9, 0, sequence] || reply[9] > 100 { return None; }
    let charging = match reply[8] { 0 | 2 => false, 1 => true, _ => return None };
    // The official parser extracts, but does not validate, the reply checkCode.
    // Do not assume the output XOR also describes incoming firmware reports.
    Some(Battery { percent: Some(reply[9]), estimated_percent: None, charging: Some(charging),
        voltage_mv: None, approximate: false, level: None })
}

fn read_with(target: &Target, sequence: u8, mut exchange: impl FnMut(&[u8]) -> Option<Vec<u8>>)
    -> Result<Snapshot, String> {
    let i = target.hid()?;
    let fail = || crate::i18n::translate("设备无有效状态回报").to_string();
    if !control(i) { return Err(fail()); }
    let reply = exchange(&request(sequence, i.output_len)).ok_or_else(fail)?;
    let battery = battery(&reply, sequence, i.input_len).ok_or_else(fail)?;
    Ok(Snapshot { target: target.clone(), battery: Some(battery), battery_at: Some(Instant::now()),
        battery_unavailable: false, connected: true, dpi: Dpi::default(), polling_rate: Default::default() })
}

impl DeviceBackend for MchoseQhw {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i)).map(|i| {
            let mut target = Target::new(BackendId::MchoseQhw, i, 0);
            target.name = "MCHOSE G98 V3".into();
            target.family = DeviceFamily::Keyboard;
            target
        }).collect())
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let interface = target.hid()?;
        if !proven(api, interface) { return Err(crate::i18n::translate("设备无有效状态回报").into()); }
        let lock = channel::lock(&channel::collection_group(interface));
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(interface)?;
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        read_with(target, sequence, |request| {
            let started = Instant::now();
            let response = device.transact(request, 1_500, |r| battery(r, sequence, interface.input_len).is_some());
            trace::record(interface, BackendId::MchoseQhw, "query-battery", 0x0904, 0x4d, request.len(),
                response.as_ref().map_or(0, Vec::len), started,
                if response.is_some() { trace::Outcome::Ok } else { trace::Outcome::NoReply });
            response
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface() -> HidInterface {
        HidInterface { path: r"\\?\hid#vid_3837&pid_1015&mi_01&col02#8&unit-a&0002#{hid}".into(),
            vendor_id: 0x3837, product_id: 0x1015, usage_page: 0xff80, usage: 1,
            input_len: 32, output_len: 32, feature_len: 0, serial: "shared".into(),
            product: "MCHOSE G98 V3 2.4G".into() }
    }

    fn response(sequence: u8, percent: u8, charge: u8, length: usize) -> Vec<u8> {
        // Synthetic protocol vector; no hardware reply length is assumed.
        let mut r = vec![0; length];
        r[..11].copy_from_slice(&[0x4d, 1, 3, 2, 4, 9, 0, sequence, charge, percent, 0x83]);
        r
    }

    #[test]
    fn request_checksum_sequence_wrap_and_padding_match_official_wire_fields() {
        let sequence = AtomicU8::new(0);
        for index in 0..=256 {
            let seq = sequence.fetch_add(1, Ordering::Relaxed);
            assert_eq!(seq, index as u8);
            let frame = request(seq, 32);
            assert_eq!(frame.len(), 32);
            assert_eq!(&frame[..8], &[0x4d, 1, 3, 0, 4, 9, 0, seq]);
            assert_eq!(frame[8], frame[2..8].iter().fold(0, |sum, b| sum ^ b));
            assert!(frame[9..].iter().all(|b| *b == 0));
        }
    }

    #[test]
    fn all_percentages_preserve_full_and_charging_without_an_invented_reply_checksum() {
        for percent in 0..=100 {
            for charge in 0..=2 {
                let mut r = response(7, percent, charge, 32);
                for check_code in [0, 0x83, 0xff] {
                    r[10] = check_code;
                    let b = battery(&r, 7, 32).unwrap();
                    assert_eq!((b.percent, b.charging), (Some(percent), Some(charge == 1)));
                    assert_eq!(b.display_percent().unwrap().label(true), format!("{percent}%"));
                }
            }
        }
    }

    #[test]
    fn stale_sequences_echoes_wrong_headers_lengths_and_invalid_values_are_ignored() {
        let valid = response(7, 56, 0, 32);
        assert!(battery(&request(7, 32), 7, 32).is_none());
        assert!(battery(&valid, 6, 32).is_none());
        for length in 0..32 { assert!(battery(&valid[..length], 7, 32).is_none()); }
        let mut long = valid.clone(); long.push(0); assert!(battery(&long, 7, 32).is_none());
        for (offset, value) in [(0, 0x4e), (1, 0), (2, 0), (3, 3), (4, 5), (5, 8),
            (6, 1), (7, 6), (8, 3), (8, 0xff), (9, 101), (9, 0xff)] {
            let mut r = valid.clone(); r[offset] = value;
            assert!(battery(&r, 7, 32).is_none());
        }
    }

    #[test]
    fn query_uses_separate_input_output_capacities_and_failure_has_no_new_timestamp() {
        let mut i = interface(); i.input_len = 64; i.output_len = 16;
        let target = Target::new(BackendId::MchoseQhw, &i, 0);
        let mut calls = 0;
        let sample = read_with(&target, 9, |frame| {
            calls += 1; assert_eq!(frame.len(), 16); assert_eq!(frame[7], 9);
            Some(response(9, 67, 1, 64))
        }).unwrap();
        assert_eq!(calls, 1);
        assert_eq!(sample.battery.unwrap().percent, Some(67));
        assert!(sample.battery_at.is_some() && !sample.battery_unavailable && sample.connected);
        assert!(sample.dpi.stages.is_empty());
        assert!(read_with(&target, 9, |_| None).is_err());
        assert!(read_with(&target, 9, |_| Some(response(8, 67, 1, 64))).is_err());
        assert!(read_with(&target, 9, |_| Some(response(9, 67, 1, 32))).is_err());
    }

    #[test]
    fn exact_keyboard_identity_excludes_boot_mouse_and_other_collections() {
        let i = interface(); assert!(control(&i));
        for (vid, pid, page, usage, input, output, product) in [
            (0x3837, 0x1017, 0xff80, 1, 32, 32, "MCHOSE G98 V3 2.4G"),
            (0x3837, 0x100b, 0xff80, 1, 32, 32, "MCHOSE G98 V3 2.4G"),
            (0x3837, 0x1015, 0xff70, 0x71, 32, 32, "MCHOSE G98 V3 2.4G"),
            (0x3837, 0x1015, 0xff81, 1, 32, 32, "MCHOSE G98 V3 2.4G"),
            (0x3837, 0x1015, 0xff80, 1, 32, 32, "CH585"),
            (0x3837, 0x1015, 0xff80, 1, 10, 32, "MCHOSE G98 V3 2.4G"),
            (0x3837, 0x1015, 0xff80, 1, 32, 8, "MCHOSE G98 V3 2.4G"),
            (0x3837, 0x1015, 0xff80, 1, 4097, 32, "MCHOSE G98 V3 2.4G"),
        ] {
            let mut wrong = i.clone();
            wrong.vendor_id = vid; wrong.product_id = pid; wrong.usage_page = page; wrong.usage = usage;
            wrong.input_len = input; wrong.output_len = output; wrong.product = product.into();
            assert!(!control(&wrong));
            let t = Target::new(BackendId::MchoseQhw, &wrong, 0);
            assert!(read_with(&t, 0, |_| panic!("must not send")).is_err());
        }
        let mut other = i.clone(); other.path = other.path.replace("unit-a", "unit-b");
        assert_eq!(candidates(&[i.clone(), other.clone()]).len(), 2);
        assert_ne!(Target::new(BackendId::MchoseQhw, &i, 0).key, Target::new(BackendId::MchoseQhw, &other, 0).key);
        let mut sibling = i.clone();
        sibling.path = sibling.path.replace("col02", "col03").replace("&0002", "&0003");
        sibling.product = "Unknown".into();
        assert!(candidates(&[i, sibling]).is_empty());
    }
}
