//! HyperX battery queries, without audio settings writes.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct HyperX;

fn control(i: &HidInterface) -> bool {
    i.vendor_id == 0x03f0 && i.product_id == 0x08be
        && i.usage_page == 0xff13 && i.usage == 0xff00
        && i.input_len == 64 && i.output_len == 64
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Query { Connection, Battery }

impl Query {
    fn command(self) -> (u8, u8) {
        match self { Self::Connection => (0x52, 1), Self::Battery => (0x50, 2) }
    }

    fn packet(self) -> [u8; 64] {
        let mut packet = [0; 64];
        (packet[0], packet[1]) = self.command();
        packet
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Reply { Connection(bool), Battery(Battery), Rejected }

fn reply(frame: &[u8], query: Query) -> Option<Reply> {
    if frame.len() != 64 { return None; }
    let (command, subcommand) = query.command();
    if frame[..2] == [0xff, 2] && frame[14..16] == [command, subcommand] {
        return Some(Reply::Rejected);
    }
    // ACKs, command echoes, FB notifications and other commands are not data.
    if frame[..2] != [command + 1, subcommand] { return None; }
    match query {
        Query::Connection => match frame[2] {
            0 => Some(Reply::Connection(false)), 1 => Some(Reply::Connection(true)), _ => None,
        },
        Query::Battery => {
            if frame[2] > 100 { return None; }
            let charging = match frame[3] { 0 => Some(false), 1 => Some(true), _ => None };
            // Bytes 6/7 are little-endian voltage. A bit in the low voltage byte
            // changes with charge level and cannot establish charging state.
            let voltage = u16::from_le_bytes([frame[6], frame[7]]);
            Some(Reply::Battery(Battery { percent: Some(frame[2]), estimated_percent: None,
                charging, voltage_mv: (!matches!(voltage, 0 | 0xffff)).then_some(voltage),
                approximate: false, level: None }))
        },
    }
}

fn query(device: &mut HidDevice, interface: &HidInterface, command: Query) -> Option<Reply> {
    let started = Instant::now();
    let response = device.transact(&command.packet(), 800, |r| reply(r, command).is_some());
    let parsed = response.as_deref().and_then(|r| reply(r, command));
    trace::record(interface, BackendId::HyperX, match command {
        Query::Connection => "query-connection", Query::Battery => "query-battery",
    }, u16::from(command.command().0), command.command().1, 64,
        response.as_ref().map_or(0, Vec::len), started, match &parsed {
            Some(Reply::Rejected) => trace::Outcome::Rejected(2),
            Some(_) => trace::Outcome::Ok, None => trace::Outcome::NoReply,
        });
    parsed
}

struct Sample { connected: bool, battery: Option<Battery>, measured: Option<Instant> }

fn sample(mut query: impl FnMut(Query) -> Option<Reply>) -> Option<Sample> {
    let connected = match query(Query::Connection) {
        Some(Reply::Connection(connected)) => connected, _ => return None,
    };
    let mut sample = Sample { connected, battery: None, measured: None };
    if !connected { return Some(sample); }
    let Some(Reply::Battery(battery)) = query(Query::Battery) else { return Some(sample); };
    let measured = Instant::now();
    // The base station can retain a previous battery after the headset powers
    // off. Reconfirm connection before publishing a newly measured percentage.
    match query(Query::Connection) {
        Some(Reply::Connection(true)) => {
            sample.battery = Some(battery); sample.measured = Some(measured);
        },
        Some(Reply::Connection(false)) => sample.connected = false,
        _ => (), // Unconfirmed completion cannot renew the previous sample.
    }
    Some(sample)
}

fn snapshot(target: &Target, sample: Sample) -> Snapshot {
    Snapshot { target: target.clone(), connected: sample.connected,
        battery_unavailable: !sample.connected, battery: sample.battery, battery_at: sample.measured,
        dpi: Dpi::default(), polling_rate: Default::default() }
}

impl DeviceBackend for HyperX {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        let mut targets: Vec<_> = interfaces.iter().filter(|i| control(i)).map(|i| {
            let mut target = Target::new(BackendId::HyperX, i, 0);
            target.name = "HyperX Cloud Alpha 2 Wireless".into();
            target.family = DeviceFamily::Headset;
            target
        }).collect();
        targets.extend(cloud_ii::discover_with(interfaces, |interface| cloud_ii::proven(api, interface)));
        Ok(targets)
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let interface = target.hid()?;
        if cloud_ii::control_group(interface).is_some() { return cloud_ii::read(api, target); }
        if !control(interface) { return Err(crate::i18n::translate("设备无有效状态回报").into()); }
        let lock = channel::lock(&interface.path);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(interface)?;
        let sample = sample(|command| query(&mut device, interface, command))
            .ok_or_else(|| crate::i18n::translate("设备无有效状态回报"))?;
        Ok(snapshot(target, sample))
    }
}

mod cloud_ii {
    use super::*;
    use std::collections::BTreeMap;

    // Protocol facts: Sapd/HeadsetControl@32facec14215d3fb6c4b1db0d3b28e0b8d20aa5a,
    // lib/devices/hyperx_cloud_2_wireless.hpp (GPL-3.0; no implementation copied).
    // Reset hazard / charge states: LennardKittner/HyperHeadset@3652a5de38c100842e4b2f882af0e04897804638,
    // src/devices/{cloud_ii_wireless_dts,mod}.rs (MIT).
    const REPORT_ID: u8 = 6;
    const INPUT_LENGTH: u16 = 20;
    const OUTPUT_LENGTH: u16 = 52;

    pub(super) fn control_group(interface: &HidInterface) -> Option<String> {
        if interface.vendor_id != 0x03f0 || !matches!(interface.product_id, 0x0696 | 0x018b)
            || interface.usage_page != 0xff90 || interface.usage != 0x0303
            || interface.input_len != INPUT_LENGTH || interface.output_len != OUTPUT_LENGTH {
            return None;
        }
        let path = interface.path.to_ascii_lowercase();
        let parts: Vec<_> = path.split('#').collect();
        if parts.len() != 4 || parts[0] != r"\\?\hid" || parts[2].is_empty()
            || parts[3].is_empty() || path.contains('\0') { return None; }
        let fields: Vec<_> = parts[1].split('&').collect();
        let product = format!("pid_{:04x}", interface.product_id);
        for (prefix, expected) in [("vid_", "vid_03f0"), ("pid_", product.as_str()), ("mi_", "mi_00")] {
            let actual: Vec<_> = fields.iter().copied().filter(|field| field.starts_with(prefix)).collect();
            if actual != [expected] { return None; }
        }
        let collections: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix("col")).collect();
        let instance = match collections.as_slice() {
            [] => parts[2],
            [collection] if collection.len() == 2 && collection.bytes().all(|byte| byte.is_ascii_hexdigit()) => {
                let (instance, index) = parts[2].rsplit_once('&')?;
                if instance.split('&').count() < 2 || index.len() != 4 || !index.starts_with("00")
                    || !index.bytes().all(|byte| byte.is_ascii_hexdigit()) { return None; }
                instance
            },
            _ => return None,
        };
        let hardware = fields.into_iter().filter(|field| !field.starts_with("col"))
            .collect::<Vec<_>>().join("&");
        Some(format!("{}#{hardware}#{instance}#{}", parts[0], parts[3]))
    }

    pub(super) fn proven(api: &HidApi, interface: &HidInterface) -> bool {
        control_group(interface).is_some() && descriptor::Descriptor::open(api, interface)
            .is_some_and(|caps| caps.single_input_output_byte_report_ids(interface, INPUT_LENGTH, OUTPUT_LENGTH)
                == Some((REPORT_ID, REPORT_ID)))
    }

    pub(super) fn discover_with(interfaces: &[HidInterface], mut proof: impl FnMut(&HidInterface) -> bool)
        -> Vec<Target> {
        let candidates: Vec<_> = interfaces.iter().filter_map(|interface|
            control_group(interface).map(|group| (interface, group))).collect();
        let mut counts = BTreeMap::new();
        for (_, group) in &candidates { *counts.entry(group.clone()).or_insert(0usize) += 1; }
        candidates.iter().filter(|(interface, group)| counts[group] == 1 && proof(interface))
            .map(|(interface, _)| {
                let mut target = Target::new(BackendId::HyperX, interface, 0);
                target.name = "HyperX Cloud II Wireless".into();
                target.family = DeviceFamily::Headset;
                target
            }).collect()
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Query { Battery, Charging }

    impl Query {
        fn command(self) -> u8 { match self { Self::Battery => 2, Self::Charging => 3 } }

        fn packet(self) -> [u8; OUTPUT_LENGTH as usize] {
            let mut packet = [0; OUTPUT_LENGTH as usize];
            packet[..4].copy_from_slice(&[REPORT_ID, 0xff, 0xbb, self.command()]);
            packet
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Reply { Battery(Option<Battery>), Charging(Option<bool>) }

    fn reply(frame: &[u8], query: Query) -> Option<Reply> {
        // Both lengths include numbered Report ID 06; there is no extra zero slot.
        if frame.len() != usize::from(INPUT_LENGTH)
            || !frame.starts_with(&[REPORT_ID, 0xff, 0xbb, query.command()]) { return None; }
        Some(match query {
            Query::Battery => {
                let voltage = u16::from_be_bytes([frame[5], frame[6]]);
                let voltage = (!matches!(voltage, 0 | 0xffff)).then_some(voltage);
                // A zero-filled reply/shortened echo cannot prove an empty battery.
                let measured = frame[7] <= 100 && (frame[7] != 0 || voltage.is_some());
                Reply::Battery(measured.then_some(Battery { percent: Some(frame[7]), estimated_percent: None,
                    charging: None, voltage_mv: voltage, approximate: false, level: None }))
            },
            Query::Charging => Reply::Charging(match frame[4] {
                0 | 2 => Some(false), 1 => Some(true), _ => None,
            }),
        })
    }

    fn query(device: &mut HidDevice, interface: &HidInterface, command: Query) -> Option<Reply> {
        let started = Instant::now();
        let response = device.transact_with_input_report(&command.packet(), REPORT_ID, 1000,
            |frame| reply(frame, command).is_some());
        let parsed = response.as_deref().and_then(|frame| reply(frame, command));
        trace::record(interface, BackendId::HyperX, match command {
            Query::Battery => "cloud-ii-battery", Query::Charging => "cloud-ii-charging",
        }, 0xffbb, command.command(), usize::from(OUTPUT_LENGTH), response.as_ref().map_or(0, Vec::len),
            started, match &parsed {
                Some(Reply::Battery(Some(_)) | Reply::Charging(Some(_))) => trace::Outcome::Ok,
                Some(_) => trace::Outcome::InvalidData, None => trace::Outcome::NoReply,
            });
        parsed
    }

    fn sample(mut query: impl FnMut(Query) -> Option<Reply>) -> Option<(Battery, Instant)> {
        let mut battery = match query(Query::Battery) {
            Some(Reply::Battery(battery)) => battery, _ => None,
        };
        let mut measured = Instant::now();
        let charging = match query(Query::Charging) {
            Some(Reply::Charging(charging)) => charging, _ => None,
        };
        if let Some(battery) = &mut battery { battery.charging = charging; }
        else if charging == Some(true) {
            battery = Some(Battery { percent: None, estimated_percent: None, charging,
                voltage_mv: None, approximate: false, level: None });
            measured = Instant::now();
        }
        battery.map(|battery| (battery, measured))
    }

    fn read_with(target: &Target, query: impl FnMut(Query) -> Option<Reply>) -> Result<Snapshot, String> {
        let (battery, measured) = sample(query)
            .ok_or_else(|| crate::i18n::translate("设备无有效状态回报").to_string())?;
        Ok(Snapshot { target: target.clone(), connected: true, battery_unavailable: false,
            battery: Some(battery), battery_at: Some(measured),
            dpi: Dpi::default(), polling_rate: Default::default() })
    }

    pub(super) fn read(api: &HidApi, target: &Target) -> Result<Snapshot, String> {
        let interface = target.hid()?;
        if !proven(api, interface) { return Err(crate::i18n::translate("设备无有效状态回报").into()); }
        let lock = channel::lock(&interface.path);
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut device = HidDevice::open(interface)?;
        // Command 01 can reset audio state. Silence is a failed read, not proof of offline.
        read_with(target, |command| query(&mut device, interface, command))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn interface(product: u16, instance: &str, collection: u8) -> HidInterface {
            HidInterface {
                path: format!(r"\\?\hid#vid_03f0&pid_{product:04x}&mi_00&col{collection:02x}#8&{instance}&0&00{collection:02x}#{{fixture}}"),
                vendor_id: 0x03f0, product_id: product, usage_page: 0xff90, usage: 0x0303,
                input_len: INPUT_LENGTH, output_len: OUTPUT_LENGTH, feature_len: 0,
                product: "HyperX Cloud II Wireless".into(), serial: "000000000001".into(),
            }
        }

        fn frame(query: Query, value: u8, voltage: u16) -> [u8; INPUT_LENGTH as usize] {
            let mut frame = [0; INPUT_LENGTH as usize];
            frame[..4].copy_from_slice(&[REPORT_ID, 0xff, 0xbb, query.command()]);
            match query {
                Query::Battery => { frame[5..7].copy_from_slice(&voltage.to_be_bytes()); frame[7] = value; },
                Query::Charging => frame[4] = value,
            }
            frame
        }

        fn battery(percent: u8, voltage: u16) -> Option<Battery> {
            let Some(Reply::Battery(battery)) = reply(&frame(Query::Battery, percent, voltage), Query::Battery)
                else { panic!("unmatched response"); };
            battery
        }

        #[test]
        fn numbered_reports_include_id_once_and_queries_cannot_reset_audio() {
            let mut requests = Vec::new();
            let (reading, _) = sample(|query| {
                requests.push(query.packet());
                reply(&frame(query, if query == Query::Battery { 64 } else { 1 }, 4012), query)
            }).unwrap();
            assert_eq!((reading.percent, reading.charging, reading.voltage_mv), (Some(64), Some(true), Some(4012)));
            assert_eq!(requests.len(), 2);
            for (packet, command) in requests.iter().zip([2, 3]) {
                assert_eq!(packet.len(), 52);
                assert_eq!(&packet[..4], &[6, 0xff, 0xbb, command]);
                assert!(packet[4..].iter().all(|byte| *byte == 0));
                assert_eq!(reply(packet, Query::Battery), None);
                assert_eq!(reply(packet, Query::Charging), None);
            }
            assert_eq!(INPUT_LENGTH, 20);
        }

        #[test]
        fn exact_length_report_id_header_and_requested_command_must_match() {
            for query in [Query::Battery, Query::Charging] {
                let valid = frame(query, 1, 3800);
                for length in 0..valid.len() { assert_eq!(reply(&valid[..length], query), None); }
                let mut oversized = valid.to_vec(); oversized.push(0);
                assert_eq!(reply(&oversized, query), None);
                let mut extra_id = vec![0]; extra_id.extend_from_slice(&valid);
                assert_eq!(reply(&extra_id, query), None);
                for index in 0..4 {
                    let mut wrong = valid; wrong[index] ^= 1;
                    assert_eq!(reply(&wrong, query), None);
                }
                for command in [1, 5, 6, 7, 0x20, 0x21, 0x22, 0x23] {
                    let mut unrelated = valid; unrelated[3] = command;
                    assert_eq!(reply(&unrelated, query), None);
                }
                assert_eq!(reply(&[0; 20], query), None);
                assert_eq!(reply(&[0xff; 20], query), None);
            }
            assert_eq!(reply(&frame(Query::Charging, 1, 0), Query::Battery), None);
            assert_eq!(reply(&frame(Query::Battery, 80, 4000), Query::Charging), None);
            assert_eq!(reply(&[0x51; 64], Query::Battery), None);
        }

        #[test]
        fn valid_zero_is_distinct_from_missing_voltage_zero_fill_and_invalid_percent() {
            for percent in 0..=100 {
                let reading = battery(percent, 3500).unwrap();
                assert_eq!(reading.percent, Some(percent));
                assert_eq!(reading.charging, None);
                assert_eq!(reading.estimated_percent, None);
                assert!(!reading.approximate);
            }
            for voltage in [0, 0xffff] {
                assert_eq!(battery(0, voltage), None);
                assert_eq!(battery(81, voltage).unwrap().voltage_mv, None);
            }
            for percent in 101..=255 { assert_eq!(battery(percent, 4000), None); }
            let echo = Query::Battery.packet();
            assert_eq!(reply(&echo[..20], Query::Battery), Some(Reply::Battery(None)));
        }

        #[test]
        fn charge_error_and_full_status_do_not_invent_a_percentage_or_charging_flag() {
            for status in 0..=255 {
                let expected = match status { 0 | 2 => Some(false), 1 => Some(true), _ => None };
                let (reading, _) = sample(|query| reply(&frame(query,
                    if query == Query::Battery { 37 } else { status }, 3800), query)).unwrap();
                assert_eq!(reading.percent, Some(37));
                assert_eq!(reading.charging, expected);
                let unknown = sample(|query| reply(&frame(query,
                    if query == Query::Battery { 0xff } else { status }, 0), query));
                if status == 1 {
                    let (unknown, _) = unknown.unwrap();
                    assert_eq!((unknown.percent, unknown.charging, unknown.voltage_mv), (None, Some(true), None));
                } else { assert_eq!(unknown, None); }
            }
        }

        #[test]
        fn silence_and_partial_reads_preserve_unknown_without_fabricating_offline() {
            let target = Target::new(BackendId::HyperX, &interface(0x0696, "receiver", 1), 0);
            assert!(read_with(&target, |_| None).is_err());
            assert!(read_with(&target, |query| reply(&frame(query, 0, 0), query)).is_err());
            let mut calls = Vec::new();
            let reading = read_with(&target, |query| {
                calls.push(query);
                if query == Query::Battery { reply(&frame(query, 26, 3700), query) } else { None }
            }).unwrap();
            assert_eq!(calls, [Query::Battery, Query::Charging]);
            assert_eq!(reading.battery.unwrap().charging, None);
            assert!(reading.connected && !reading.battery_unavailable && reading.battery_at.is_some());
            assert_eq!(reading.dpi, Dpi::default());
            assert!(!reading.polling_rate.writable);
            let charging = read_with(&target, |query| if query == Query::Charging {
                reply(&frame(query, 1, 0), query)
            } else { None }).unwrap();
            assert_eq!(charging.battery.unwrap().percent, None);
        }

        #[test]
        fn charge_query_cannot_renew_an_earlier_percentage_timestamp() {
            let target = Target::new(BackendId::HyperX, &interface(0x0696, "receiver", 1), 0);
            for charge_reply in [false, true] {
                let mut charge_started = None;
                let reading = read_with(&target, |query| match query {
                    Query::Battery => reply(&frame(query, 62, 3900), query),
                    Query::Charging => {
                        charge_started = Some(Instant::now());
                        charge_reply.then(|| reply(&frame(query, 1, 0), query)).flatten()
                    },
                }).unwrap();
                assert!(reading.battery_at.unwrap() <= charge_started.unwrap());
                assert_eq!(reading.battery.unwrap().percent, Some(62));
            }
            let mut charge_started = None;
            let reading = read_with(&target, |query| match query {
                Query::Battery => None,
                Query::Charging => {
                    charge_started = Some(Instant::now());
                    reply(&frame(query, 1, 0), query)
                },
            }).unwrap();
            assert!(reading.battery_at.unwrap() >= charge_started.unwrap());
            assert_eq!(reading.battery.unwrap().percent, None);
        }

        #[test]
        fn descriptors_and_interface_identity_fail_closed_before_any_query() {
            let valid = interface(0x0696, "receiver", 1);
            for mutate in [
                |candidate: &mut HidInterface| candidate.vendor_id = 0x0951,
                |candidate: &mut HidInterface| candidate.product_id = 0x1718,
                |candidate: &mut HidInterface| candidate.usage_page = 0xff13,
                |candidate: &mut HidInterface| candidate.usage = 1,
                |candidate: &mut HidInterface| candidate.input_len = 21,
                |candidate: &mut HidInterface| candidate.input_len = 64,
                |candidate: &mut HidInterface| candidate.output_len = 53,
                |candidate: &mut HidInterface| { candidate.output_len = 0; candidate.feature_len = 52; },
                |candidate: &mut HidInterface| candidate.path = candidate.path.replace("mi_00", "mi_03"),
                |candidate: &mut HidInterface| candidate.path = candidate.path.replace("mi_00", "mi_00&mi_01"),
                |candidate: &mut HidInterface| candidate.path = candidate.path.replace("pid_0696", "pid_018b"),
                |candidate: &mut HidInterface| candidate.path = "unknown-path".into(),
            ] {
                let mut invalid = valid.clone(); mutate(&mut invalid);
                assert!(discover_with(&[invalid], |_| panic!("not a candidate")).is_empty());
            }
            assert!(discover_with(&[valid.clone()], |_| false).is_empty());
            for product in [0x0696, 0x018b] {
                let targets = discover_with(&[interface(product, "receiver", 1)], |_| true);
                assert_eq!(targets.len(), 1);
                assert_eq!(targets[0].family, DeviceFamily::Headset);
                assert_eq!(targets[0].address, 0);
            }
            for product in [0x08be, 0x098d, 0x05b7, 0x0c9d, 0x1718, 0x0d93] {
                assert!(discover_with(&[interface(product, "receiver", 1)], |_| panic!("unproven model")).is_empty());
            }
        }

        #[test]
        fn duplicate_serials_and_failures_remain_isolated_per_receiver() {
            let first = interface(0x0696, "first", 1);
            let second = interface(0x0696, "second", 1);
            assert_eq!(first.serial, second.serial);
            let targets = discover_with(&[first.clone(), second.clone()], |_| true);
            assert_eq!(targets.len(), 2);
            assert_ne!(targets[0].key, targets[1].key);
            assert_ne!(targets[0].channel_key(), targets[1].channel_key());
            assert!(!std::sync::Arc::ptr_eq(&channel::lock(&first.path), &channel::lock(&second.path)));
            assert!(read_with(&targets[0], |_| None).is_err());
            let reading = read_with(&targets[1], |query| reply(&frame(query,
                if query == Query::Battery { 72 } else { 0 }, 4000), query)).unwrap();
            assert_eq!(reading.target.key, targets[1].key);
            assert_eq!(reading.battery.unwrap().percent, Some(72));
            let remaining = discover_with(&[first.clone(), second.clone()], |candidate| candidate.path == second.path);
            assert_eq!(remaining.len(), 1);
            assert_eq!(remaining[0].key, targets[1].key);
            let duplicate = interface(0x0696, "first", 2);
            let mut zero_based = first.clone();
            zero_based.path = zero_based.path.replace("&0001#", "&0000#");
            assert_eq!(control_group(&zero_based), control_group(&first));
            assert_eq!(control_group(&duplicate), control_group(&first));
            for interfaces in [vec![first.clone(), duplicate.clone(), second.clone()], vec![second, duplicate, first]] {
                let remaining = discover_with(&interfaces, |_| true);
                assert_eq!(remaining.len(), 1);
                assert_eq!(remaining[0].key, targets[1].key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(prefix: &[u8]) -> [u8; 64] {
        let mut frame = [0; 64]; frame[..prefix.len()].copy_from_slice(prefix); frame
    }

    fn battery(frame: &[u8]) -> Battery {
        let Some(Reply::Battery(battery)) = reply(frame, Query::Battery) else { panic!("missing battery"); };
        battery
    }

    fn interface() -> HidInterface {
        HidInterface { path: "station-a".into(), vendor_id: 0x03f0, product_id: 0x08be,
            usage_page: 0xff13, usage: 0xff00, input_len: 64, output_len: 64, feature_len: 0,
            product: "Wireless Controller".into(), serial: "shared".into() }
    }

    #[test]
    fn published_battery_frames_use_independent_percentage_charge_and_voltage_fields() {
        // HeadsetControl's battery-power and USB-power captures.
        let unplugged = battery(&frame(&[0x51, 2, 93, 0, 25, 0, 0x61, 0x10, 1, 1, 0x0a]));
        assert_eq!((unplugged.percent, unplugged.charging, unplugged.voltage_mv), (Some(93), Some(false), Some(4193)));
        let charging = battery(&frame(&[0x51, 2, 91, 1, 28, 0, 0xea, 0x10, 1, 1, 0x0a]));
        assert_eq!((charging.percent, charging.charging, charging.voltage_mv), (Some(91), Some(true), Some(4330)));
        // MyFinder's 51% / 67% captures change the voltage byte, not charger state.
        for (percent, lo, voltage) in [(51, 0, 3840), (67, 0x86, 3974)] {
            let b = battery(&frame(&[0x51, 2, percent, 0, 26, 0, lo, 0x0f, 1, 1, 0x0a]));
            assert_eq!((b.percent, b.charging, b.voltage_mv), (Some(percent), Some(false), Some(voltage)));
            assert!(!b.approximate);
            assert_eq!(b.display_percent().unwrap().label(true), format!("{percent}%"));
        }
    }

    #[test]
    fn valid_zero_and_full_do_not_fabricate_charging_unknown_fields_stay_unknown() {
        for percent in 0..=100 {
            let b = battery(&frame(&[0x51, 2, percent, 0xff, 0, 0, 0xff, 0xff]));
            assert_eq!((b.percent, b.charging, b.voltage_mv), (Some(percent), None, None));
        }
        assert_eq!(battery(&frame(&[0x51, 2, 100, 0])).charging, Some(false));
        for invalid in 101..=255 { assert!(reply(&frame(&[0x51, 2, invalid, 0]), Query::Battery).is_none()); }
        for invalid in 2..=255 { assert!(reply(&frame(&[0x53, 1, invalid]), Query::Connection).is_none()); }
    }

    #[test]
    fn malformed_frames_echoes_acknowledgements_and_other_traffic_are_not_measurements() {
        for query in [Query::Battery, Query::Connection] {
            let valid = match query {
                Query::Battery => frame(&[0x51, 2, 61, 0]), Query::Connection => frame(&[0x53, 1, 1]),
            };
            for length in 0..64 { assert!(reply(&valid[..length], query).is_none()); }
            let mut too_long = valid.to_vec(); too_long.push(0);
            assert!(reply(&too_long, query).is_none());
            assert!(reply(&query.packet(), query).is_none());
            for prefix in [&[0xfb, 0x0a, 0][..], &[0xfb, 0x13, 0, 93], &[0x45, 2, 1], &[0x61, 2, 90], &[0xff, 1]] {
                assert!(reply(&frame(prefix), query).is_none());
            }
            let mut nack = frame(&[0xff, 2]);
            (nack[14], nack[15]) = query.command();
            assert_eq!(reply(&nack, query), Some(Reply::Rejected));
            nack[14] = 0x60;
            assert!(reply(&nack, query).is_none());
        }
    }

    #[test]
    fn disconnected_and_partial_queries_cannot_publish_cached_station_battery() {
        let known = battery(&frame(&[0x51, 2, 61, 0]));
        for ending in [Some(Reply::Connection(false)), None, Some(Reply::Rejected)] {
            let mut queries = Vec::new();
            let mut responses = vec![Some(Reply::Connection(true)), Some(Reply::Battery(known.clone())), ending].into_iter();
            let s = sample(|q| { queries.push(q); responses.next().unwrap() }).unwrap();
            assert!(s.battery.is_none());
            assert!(s.measured.is_none());
            assert_eq!(queries, vec![Query::Connection, Query::Battery, Query::Connection]);
        }
        let s = sample(|q| {
            assert_eq!(q, Query::Connection); Some(Reply::Connection(false))
        }).unwrap();
        let offline = snapshot(&Target::new(BackendId::HyperX, &interface(), 0), s);
        assert!(!offline.connected);
        assert!(offline.battery_unavailable);
        assert!(offline.battery.is_none());
        assert!(sample(|_| Some(Reply::Rejected)).is_none());
        let mut calls = 0;
        let partial = sample(|q| {
            calls += 1;
            match q { Query::Connection => Some(Reply::Connection(true)), Query::Battery => None }
        }).unwrap();
        assert_eq!(calls, 2);
        assert!(partial.connected);
        assert!(partial.measured.is_none());
    }

    #[test]
    fn fresh_valid_battery_is_published_only_after_connection_is_reconfirmed() {
        let mut writes = Vec::new();
        let sample = sample(|q| {
            writes.push(q.packet());
            match q {
                Query::Connection => Some(Reply::Connection(true)),
                Query::Battery => Some(Reply::Battery(battery(&frame(&[0x51, 2, 61, 0])))),
            }
        }).unwrap();
        assert!(sample.connected);
        assert_eq!(sample.battery.unwrap().percent, Some(61));
        assert!(sample.measured.is_some());
        assert_eq!(writes.iter().map(|r| &r[..2]).collect::<Vec<_>>(), vec![&[0x52, 1][..], &[0x50, 2], &[0x52, 1]]);
        assert!(writes.iter().all(|r| r[2..].iter().all(|b| *b == 0)));
    }

    #[test]
    fn only_controller_vendor_collection_is_probed_and_instances_do_not_merge() {
        let i = interface();
        assert!(control(&i));
        let mut other = i.clone(); other.product_id = 0x0abe;
        assert!(!control(&other));
        other = i.clone(); other.usage = 1;
        assert!(!control(&other));
        other = i.clone(); other.usage_page = 1; other.usage = 6;
        assert!(!control(&other));
        for pid in [0x098d, 0x05b7, 0x0c9d, 0x0696] {
            other = i.clone(); other.product_id = pid;
            assert!(!control(&other));
        }
        other = i.clone(); other.input_len = 65;
        assert!(!control(&other));
        other = i.clone(); other.output_len = 62;
        assert!(!control(&other));
        other = i.clone(); other.path = "station-b".into();
        let first = Target::new(BackendId::HyperX, &i, 0);
        let second = Target::new(BackendId::HyperX, &other, 0);
        assert_ne!(first.key, second.key);
    }
}
