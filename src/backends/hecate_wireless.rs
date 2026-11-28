//! HECATE 35BB:D3FB: documented 17-byte STATUS/POWER queries only.
//! See docs/PROTOCOLS.md for recorded frames and unverified hardware boundaries.
use super::*;
use crate::hid::HidDevice;
use std::{sync::TryLockError, time::{Duration, Instant}};

pub struct HecateWireless;

const LENGTH: u16 = 17;
const REPORT_ID: u8 = 8;
const STATUS: u8 = 3;
const POWER: u8 = 4;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(interface: &HidInterface) -> bool {
    interface.vendor_id == 0x35bb && interface.product_id == 0xd3fb
        && interface.usage_page == 0xff02 && interface.usage == 2
}

fn control(interface: &HidInterface) -> bool {
    management(interface) && interface.input_len == LENGTH && interface.output_len == LENGTH
        && interface.path.to_ascii_lowercase().contains("&mi_01&col05#")
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|interface| control(interface)
        && interfaces.iter().filter(|other| management(other)
            && channel::collection_group(interface) == channel::collection_group(other)).count() == 1).collect()
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    control(interface) && descriptor::Descriptor::open(api, interface)
        .is_some_and(|descriptor| descriptor.only_input_output_report(interface, REPORT_ID, LENGTH))
}

fn target(interface: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::HecateWireless, interface, 0);
    target.name = if interface.product.trim().is_empty() { "HECATE Wireless Receiver".into() }
        else { interface.product.trim().into() };
    target.family = DeviceFamily::Mouse;
    target
}

fn valid_target(original: &Target) -> bool {
    original.backend == BackendId::HecateWireless && original.address == 0
        && original.hid().is_ok_and(|interface| control(interface) && original.key == target(interface).key)
}

fn discover_with(interfaces: &[HidInterface], mut prove: impl FnMut(&HidInterface) -> bool) -> Vec<Target> {
    candidates(interfaces).into_iter().filter(|interface| prove(interface)).map(target).collect()
}

fn checksum(frame: &[u8]) -> u8 {
    frame.iter().fold(0x55u8, |sum, byte| sum.wrapping_sub(*byte))
}

fn request(command: u8) -> Option<[u8; LENGTH as usize]> {
    if !matches!(command, STATUS | POWER) { return None; }
    let mut packet = [0; LENGTH as usize];
    packet[0] = REPORT_ID;
    packet[1] = command;
    packet[16] = checksum(&packet[..16]);
    Some(packet)
}

fn matches_reply(packet: &[u8], response: &[u8]) -> bool {
    packet.len() == usize::from(LENGTH) && response.len() == usize::from(LENGTH)
        && response[0] == REPORT_ID && response[1] == packet[1] && response != packet
        && response[16] == checksum(&response[..16])
        && match response[1] {
            STATUS => response[6] <= 1,
            POWER => response[6] <= 100,
            _ => false,
        }
}

fn exchange_with(packet: &[u8],
    exchange: impl FnOnce(&mut dyn FnMut(&[u8]) -> bool) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    let mut invalid = None;
    let response = exchange(&mut |response| {
        if matches_reply(packet, response) { return true; }
        invalid = Some(response.to_vec());
        false
    });
    response.or(invalid)
}

fn battery(response: &[u8]) -> Option<Battery> {
    if !matches_reply(&request(POWER)?, response) { return None; }
    let voltage = u16::from_be_bytes([response[8], response[9]]);
    Some(Battery { percent: Some(response[6]), estimated_percent: None, approximate: false,
        // The shared Pulsar layout does not establish this receiver's charging semantics.
        charging: None,
        voltage_mv: (!matches!(voltage, 0 | u16::MAX)).then_some(voltage), level: None })
}

fn snapshot(interface: &HidInterface, sample: Option<(Battery, Instant)>) -> Snapshot {
    let connected = sample.is_some();
    let (battery, battery_at) = sample.map_or((None, None), |(battery, at)| (Some(battery), Some(at)));
    Snapshot { target: target(interface), battery, battery_at, connected,
        battery_unavailable: !connected, dpi: Dpi::default(), polling_rate: Default::default() }
}

fn query_with(interface: &HidInterface,
    mut exchange: impl FnMut(&[u8], u32) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    let deadline = Instant::now() + Duration::from_millis(3200);
    let mut query = |command| -> Result<Vec<u8>, String> {
        let packet = request(command).ok_or(UNAVAILABLE)?;
        let started = Instant::now();
        let timeout = deadline.checked_duration_since(started).ok_or(UNAVAILABLE)?.as_millis().min(1200) as u32;
        if timeout == 0 { return Err(UNAVAILABLE.into()); }
        let response = exchange(&packet, timeout);
        let valid = response.as_deref().is_some_and(|response| matches_reply(&packet, response))
            && Instant::now() < deadline;
        trace::record(interface, BackendId::HecateWireless, "query", 0, command, packet.len(),
            response.as_ref().map_or(0, Vec::len), started,
            if valid { trace::Outcome::Ok } else if response.is_some() { trace::Outcome::InvalidData }
            else { trace::Outcome::NoReply });
        response.filter(|_| valid).ok_or_else(|| UNAVAILABLE.into())
    };
    if query(STATUS)?[6] == 0 { return Ok(snapshot(interface, None)); }
    let power = battery(&query(POWER)?).ok_or(UNAVAILABLE)?;
    let measured = Instant::now();
    if query(STATUS)?[6] == 0 { return Ok(snapshot(interface, None)); }
    Ok(snapshot(interface, Some((power, measured))))
}

fn read_with(original: &Target, interfaces: &[HidInterface],
    mut prove: impl FnMut(&HidInterface) -> bool,
    query: impl FnOnce(&HidInterface) -> Result<Snapshot, String>) -> Result<Snapshot, String> {
    if !valid_target(original) { return Err(UNAVAILABLE.into()); }
    let interface = candidates(interfaces).into_iter().find(|interface|
        interface.path.eq_ignore_ascii_case(&original.hid().unwrap().path)).ok_or(UNAVAILABLE)?;
    let lock = channel::lock(&channel::collection_group(interface));
    let _guard = match lock.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return Err(UNAVAILABLE.into()),
    };
    if !prove(interface) { return Err(UNAVAILABLE.into()); }
    let snapshot = query(interface)?;
    if !prove(interface) { return Err(UNAVAILABLE.into()); }
    Ok(snapshot)
}

impl DeviceBackend for HecateWireless {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(discover_with(interfaces, |interface| proven(api, interface)))
    }

    fn read(&self, api: &HidApi, original: &Target, _: bool) -> Result<Snapshot, String> {
        if !valid_target(original) { return Err(UNAVAILABLE.into()); }
        let interfaces = enumeration::scan(api)?.interfaces;
        read_with(original, &interfaces, |interface| proven(api, interface), |interface| {
            let mut device = HidDevice::open(interface)?;
            if !device.writable || device.input_len() != LENGTH || device.output_len() != LENGTH {
                return Err(UNAVAILABLE.into());
            }
            query_with(interface, |packet, timeout| exchange_with(packet, |accept|
                device.transact(packet, timeout, accept)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    // Complete upstream frames; modified boundary/transport cases below are synthetic.
    const RECORDED_STATUS: [u8; 17] = [8, 3, 0, 0, 0, 1, 1, 0x31, 0xaf, 0xf9, 0, 0, 0, 0, 0, 0, 0x6f];
    const RECORDED_POWER: [u8; 17] = [8, 4, 0, 0, 0, 2, 0x32, 0, 0x0f, 0x08, 0, 0, 0, 0, 0, 0, 0xfe];

    fn interface(instance: &str) -> HidInterface {
        HidInterface {
            path: format!(r"\\?\hid#vid_35bb&pid_d3fb&mi_01&col05#7&{instance}&0&0004#{{4d1e55b2-f16f-11cf-88cb-001111000030}}"),
            vendor_id: 0x35bb, product_id: 0xd3fb, usage_page: 0xff02, usage: 2,
            input_len: LENGTH, output_len: LENGTH, feature_len: 0,
            product: "HECATE Wireless Receiver".into(), serial: "shared serial".into(),
        }
    }

    fn modified(mut frame: [u8; 17], offset: usize, value: u8) -> Vec<u8> {
        frame[offset] = value;
        frame[16] = checksum(&frame[..16]);
        frame.to_vec()
    }

    fn online() -> Vec<u8> { RECORDED_STATUS.to_vec() }

    fn offline() -> Vec<u8> { modified(RECORDED_STATUS, 6, 0) }

    #[test]
    fn recorded_requests_and_replies_match() {
        assert_eq!(request(STATUS).unwrap(), [8, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x4a]);
        assert_eq!(request(POWER).unwrap(), [8, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x49]);
        assert!(matches_reply(&request(STATUS).unwrap(), &RECORDED_STATUS));
        let battery = battery(&RECORDED_POWER).unwrap();
        assert_eq!((battery.percent, battery.charging, battery.voltage_mv), (Some(50), None, Some(3848)));
        assert!(!battery.approximate);
        assert_eq!((battery.estimated_percent, battery.level), (None, None));
        for command in 0..=255 {
            assert_eq!(request(command).is_some(), matches!(command, STATUS | POWER));
        }
    }

    #[test]
    fn malformed_and_echo_frames_never_become_zero_percent_or_offline() {
        for command in [STATUS, POWER] {
            let packet = request(command).unwrap();
            assert!(!matches_reply(&packet, &packet));
            assert!(!matches_reply(&packet, &[0; 17]));
        }
        for length in 0..17 { assert!(battery(&RECORDED_POWER[..length]).is_none()); }
        let mut oversized = RECORDED_POWER.to_vec();
        oversized.push(0);
        assert!(battery(&oversized).is_none());
        for offset in 0..17 {
            let mut corrupt = RECORDED_POWER;
            corrupt[offset] ^= 1;
            assert!(battery(&corrupt).is_none());
        }
        assert!(battery(&modified(RECORDED_POWER, 0, 9)).is_none());
        assert!(battery(&modified(RECORDED_POWER, 1, STATUS)).is_none());
        for percent in 101..=255 { assert!(battery(&modified(RECORDED_POWER, 6, percent)).is_none()); }
        for state in 2..=255 {
            assert!(!matches_reply(&request(STATUS).unwrap(), &modified(RECORDED_STATUS, 6, state)));
        }
    }

    #[test]
    fn rejected_frames_are_retained_for_diagnostics_without_hiding_a_valid_reply() {
        let packet = request(POWER).unwrap();
        let mut corrupt = RECORDED_POWER;
        corrupt[16] ^= 1;
        assert!(exchange_with(&packet, |_| None).is_none());
        let invalid = exchange_with(&packet, |accept| {
            assert!(!accept(&corrupt));
            None
        }).unwrap();
        assert_eq!(invalid, corrupt);
        assert!(battery(&invalid).is_none());
        let valid = exchange_with(&packet, |accept| {
            assert!(!accept(&corrupt));
            assert!(!accept(&packet));
            assert!(accept(&RECORDED_POWER));
            Some(RECORDED_POWER.to_vec())
        }).unwrap();
        assert_eq!(valid, RECORDED_POWER);
        assert_eq!(battery(&valid).unwrap().percent, Some(50));
        assert!(query_with(&interface("invalid-data"), |packet, _| exchange_with(packet, |accept| {
            assert!(!accept(&corrupt));
            None
        })).is_err());
    }

    #[test]
    fn zero_full_and_all_unverified_charge_flags_remain_distinct() {
        for percent in [0, 1, 99, 100] {
            assert_eq!(battery(&modified(RECORDED_POWER, 6, percent)).unwrap().percent, Some(percent));
        }
        for flag in 0..=255 {
            assert_eq!(battery(&modified(RECORDED_POWER, 7, flag)).unwrap().charging, None);
        }
        let mut no_voltage = RECORDED_POWER;
        no_voltage[8..10].fill(0);
        let sample = battery(&modified(no_voltage, 6, 50)).unwrap();
        assert_eq!(sample.percent, Some(50));
        assert_eq!(sample.voltage_mv, None);
    }

    #[test]
    fn active_queries_confirm_connection_around_power() {
        let interface = interface("sequence");
        let mut commands = Vec::new();
        let snapshot = query_with(&interface, |packet, timeout| {
            commands.push(packet[1]);
            assert!((1..=1200).contains(&timeout));
            Some(if packet[1] == STATUS { online() } else { RECORDED_POWER.to_vec() })
        }).unwrap();
        assert_eq!(commands, [STATUS, POWER, STATUS]);
        assert_eq!(snapshot.battery.unwrap().percent, Some(50));
        assert!(snapshot.connected && snapshot.battery_at.is_some() && !snapshot.battery_unavailable);
        assert!(!snapshot.dpi.writable && !snapshot.dpi.stage_writable && !snapshot.polling_rate.writable);
    }

    #[test]
    fn offline_and_disconnect_during_query_discard_battery() {
        for disconnect_at in [0, 2] {
            let mut count = 0;
            let snapshot = query_with(&interface("offline"), |packet, _| {
                let reply = if count == disconnect_at { offline() }
                    else if packet[1] == STATUS { online() } else { RECORDED_POWER.to_vec() };
                count += 1;
                Some(reply)
            }).unwrap();
            assert_eq!(count, disconnect_at + 1);
            assert!(!snapshot.connected && snapshot.battery_unavailable);
            assert!(snapshot.battery.is_none() && snapshot.battery_at.is_none());
        }
    }

    #[test]
    fn timeout_at_any_step_never_publishes_a_sample() {
        for fail_at in 0..3 {
            let mut count = 0;
            let result = query_with(&interface("timeout"), |packet, _| {
                let failed = count == fail_at;
                count += 1;
                if failed { None } else { Some(if packet[1] == STATUS { online() } else { RECORDED_POWER.to_vec() }) }
            });
            assert!(result.is_err());
            assert_eq!(count, fail_at + 1);
        }
    }

    #[test]
    fn discovery_is_metadata_only_and_requires_proof() {
        let first = interface("discovery-a");
        let second = interface("discovery-b");
        let targets = discover_with(&[first, second], |_| true);
        assert_eq!(targets.len(), 2);
        assert_ne!(targets[0].key, targets[1].key);
        assert!(targets.iter().all(|target| target.family == DeviceFamily::Mouse && !target.verified_identity));
        assert!(discover_with(&[interface("unproven")], |_| false).is_empty());
        assert!(!super::super::has_configuration(BackendId::HecateWireless));
    }

    #[test]
    fn unrelated_models_interfaces_and_ambiguous_collections_are_excluded() {
        for change in [
            |interface: &mut HidInterface| interface.vendor_id = 0x260d,
            |interface: &mut HidInterface| interface.product_id = 0xd00b,
            |interface: &mut HidInterface| interface.product_id = 0xd3fd,
            |interface: &mut HidInterface| interface.usage_page = 0xff03,
            |interface: &mut HidInterface| interface.usage = 0xff02,
            |interface: &mut HidInterface| interface.input_len = 64,
            |interface: &mut HidInterface| interface.output_len = 0,
            |interface: &mut HidInterface| interface.path = interface.path.replace("mi_01", "mi_02"),
            |interface: &mut HidInterface| interface.path = interface.path.replace("col05", "col04"),
        ] {
            let mut other = interface("wrong-scope");
            change(&mut other);
            assert!(discover_with(&[other], |_| panic!("must not probe outside scope")).is_empty());
        }
        let first = interface("ambiguous");
        let mut sibling = first.clone();
        sibling.path = sibling.path.replace("col05", "col06").replace("&0004#", "&0005#");
        assert!(discover_with(&[first.clone(), sibling], |_| true).is_empty());
        assert!(discover_with(&[first.clone(), first], |_| true).is_empty());
    }

    #[test]
    fn original_target_never_falls_back_to_another_receiver() {
        let first = interface("removed");
        let second = interface("remaining");
        assert!(read_with(&target(&first), std::slice::from_ref(&second), |_| true,
            |_| panic!("wrong receiver")).is_err());
        for change in [
            |target: &mut Target| target.backend = BackendId::Dareu,
            |target: &mut Target| target.address = 1,
            |target: &mut Target| target.key = "wrong-key".into(),
        ] {
            let mut original = target(&second);
            change(&mut original);
            assert!(read_with(&original, std::slice::from_ref(&second), |_| true,
                |_| panic!("invalid target")).is_err());
        }
    }

    #[test]
    fn descriptor_changes_and_busy_channel_prevent_publication() {
        let interface = interface("proof-and-lock");
        let original = target(&interface);
        let calls = Cell::new(0);
        assert!(read_with(&original, std::slice::from_ref(&interface), |_| false,
            |_| panic!("unproven")).is_err());
        assert!(read_with(&original, std::slice::from_ref(&interface), |_| {
            calls.set(calls.get() + 1);
            calls.get() == 1
        }, |interface| Ok(snapshot(interface, Some((battery(&RECORDED_POWER).unwrap(), Instant::now()))))).is_err());
        assert_eq!(calls.get(), 2);
        let lock = channel::lock(&channel::collection_group(&interface));
        let _held = lock.lock().unwrap();
        assert!(read_with(&original, std::slice::from_ref(&interface), |_| true,
            |_| panic!("busy channel")).is_err());
    }

    #[test]
    fn failed_receiver_does_not_block_its_identical_neighbor() {
        let first = interface("independent-a");
        let second = interface("independent-b");
        let interfaces = [first.clone(), second.clone()];
        let lock = channel::lock(&channel::collection_group(&first));
        let _held = lock.lock().unwrap();
        assert!(read_with(&target(&first), &interfaces, |_| true, |_| panic!("busy")).is_err());
        let snapshot = read_with(&target(&second), &interfaces, |_| true, |interface|
            query_with(interface, |packet, _| Some(if packet[1] == STATUS { online() }
                else { modified(RECORDED_POWER, 6, 80) }))).unwrap();
        assert_eq!(snapshot.target.key, target(&second).key);
        assert_ne!(snapshot.target.key, target(&first).key);
        assert_eq!(snapshot.battery.unwrap().percent, Some(80));
    }

    #[test]
    fn receiver_name_does_not_claim_an_unverified_mouse_model() {
        let mut receiver = interface("name");
        assert_eq!(target(&receiver).name, "HECATE Wireless Receiver");
        receiver.product = "   ".into();
        assert_eq!(target(&receiver).name, "HECATE Wireless Receiver");
        receiver.product = "  HECATE custom product  ".into();
        assert_eq!(target(&receiver).name, "HECATE custom product");
        assert!(!target(&receiver).verified_identity);
        let pulsar = HidInterface { vendor_id: 0x3554, product_id: 0xf508, ..receiver.clone() };
        assert!(discover_with(&[pulsar], |_| panic!("Pulsar is a separate backend")).is_empty());
        let mut sibling = receiver.clone();
        sibling.path = sibling.path.replace("col05", "col06").replace("&0004#", "&0005#");
        sibling.usage_page = 0xff04;
        sibling.input_len = 0;
        sibling.output_len = 0;
        sibling.feature_len = 8;
        assert_eq!(discover_with(&[receiver, sibling], |_| true).len(), 1);
    }

    #[test]
    fn echoes_and_wrong_commands_at_each_step_never_refresh_battery() {
        for fail_at in 0..3 {
            for echo in [false, true] {
                let mut count = 0;
                let result = query_with(&interface("invalid-sequence"), |packet, _| {
                    let failed = count == fail_at;
                    count += 1;
                    Some(if failed {
                        if echo { packet.to_vec() }
                        else if packet[1] == STATUS { RECORDED_POWER.to_vec() } else { online() }
                    } else if packet[1] == STATUS { online() } else { RECORDED_POWER.to_vec() })
                });
                assert!(result.is_err());
                assert_eq!(count, fail_at + 1);
            }
        }
    }
}
