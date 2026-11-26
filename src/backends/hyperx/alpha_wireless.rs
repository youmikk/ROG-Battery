use super::*;
use std::{collections::BTreeMap, time::Duration};

const REPORT_ID: u8 = 0x21;
const REPORT_LENGTH: u16 = 31;

pub(super) fn control_group(interface: &HidInterface) -> Option<String> {
    if interface.vendor_id != 0x03f0 || interface.product_id != 0x098d
        || interface.usage_page != 0xff43 || interface.usage != 0x0202 { return None; }
    let path = interface.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid" || parts[3].is_empty()
        || path.contains('\0') { return None; }
    let fields: Vec<_> = parts[1].split('&').collect();
    for (prefix, expected) in [("vid_", "vid_03f0"), ("pid_", "pid_098d"), ("mi_", "mi_03")] {
        let actual: Vec<_> = fields.iter().copied().filter(|field| field.starts_with(prefix)).collect();
        if actual != [expected] { return None; }
    }
    let collections: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix("col")).collect();
    let [collection] = collections.as_slice() else { return None; };
    if collection.len() != 2 || !collection.bytes().all(|byte| byte.is_ascii_hexdigit()) { return None; }
    let (instance, index) = parts[2].rsplit_once('&')?;
    if instance.split('&').count() < 2 || instance.split('&').any(str::is_empty)
        || index.len() != 4 || !index.starts_with("00")
        || !index.bytes().all(|byte| byte.is_ascii_hexdigit()) { return None; }
    let hardware = fields.into_iter().filter(|field| !field.starts_with("col"))
        .collect::<Vec<_>>().join("&");
    Some(format!("{}#{hardware}#{instance}#{}", parts[0], parts[3]))
}

fn control(interface: &HidInterface) -> bool {
    control_group(interface).is_some()
        && interface.path.split('#').nth(1).is_some_and(|hardware|
            hardware.split('&').any(|field| field.eq_ignore_ascii_case("col01")))
        && interface.input_len == REPORT_LENGTH && interface.output_len == REPORT_LENGTH
        && interface.feature_len == 0
}

fn report_ids(api: &HidApi, interface: &HidInterface) -> Option<(u8, u8)> {
    descriptor::Descriptor::open(api, interface)?
        .single_input_output_byte_report_ids(interface, REPORT_LENGTH, REPORT_LENGTH)
}

fn proven_with(interface: &HidInterface, proof: impl FnOnce(&HidInterface) -> Option<(u8, u8)>) -> bool {
    control(interface) && proof(interface) == Some((REPORT_ID, REPORT_ID))
}

fn discover_with(interfaces: &[HidInterface], mut proof: impl FnMut(&HidInterface) -> Option<(u8, u8)>)
    -> Vec<Target> {
    let candidates: Vec<_> = interfaces.iter().filter_map(|interface|
        control_group(interface).map(|group| (interface, group))).collect();
    let mut counts = BTreeMap::new();
    for (_, group) in &candidates { *counts.entry(group.clone()).or_insert(0usize) += 1; }
    candidates.into_iter().filter(|(interface, group)| counts[group] == 1
        && proven_with(interface, &mut proof)).map(|(interface, _)| {
            let mut target = Target::new(BackendId::HyperX, interface, 0);
            target.name = "HyperX Cloud Alpha Wireless".into();
            target.family = DeviceFamily::Headset;
            target
        }).collect()
}

pub(super) fn discover(api: &HidApi, interfaces: &[HidInterface]) -> Vec<Target> {
    discover_with(interfaces, |interface| report_ids(api, interface))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Query { Connection, Battery, Charging }

impl Query {
    fn command(self) -> u8 {
        match self { Self::Connection => 0x03, Self::Battery => 0x0b, Self::Charging => 0x0c }
    }

    fn packet(self) -> [u8; REPORT_LENGTH as usize] {
        let mut packet = [0; REPORT_LENGTH as usize];
        packet[..3].copy_from_slice(&[REPORT_ID, 0xbb, self.command()]);
        packet
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Reply { Connection(bool), Battery(Option<u8>), Charging(Option<bool>) }

fn reply(frame: &[u8], query: Query) -> Option<Reply> {
    if frame.len() != usize::from(REPORT_LENGTH)
        || !frame.starts_with(&[REPORT_ID, 0xbb, query.command()])
        || frame == query.packet().as_slice() { return None; }
    match query {
        Query::Connection => match frame[3] {
            1 => Some(Reply::Connection(false)), 2 => Some(Reply::Connection(true)), _ => None,
        },
        Query::Battery => Some(Reply::Battery((1..=100).contains(&frame[3]).then_some(frame[3]))),
        Query::Charging => Some(Reply::Charging(match frame[3] {
            0 => Some(false), 1 => Some(true), _ => None,
        })),
    }
}

fn query(device: &mut HidDevice, interface: &HidInterface, command: Query, deadline: Instant) -> Option<Reply> {
    let started = Instant::now();
    let remaining = deadline.checked_duration_since(started)?;
    let timeout = remaining.as_millis().min(2000) as u32;
    if timeout == 0 { return None; }
    let response = device.transact_with_input_report(&command.packet(), REPORT_ID, timeout,
        |frame| reply(frame, command).is_some());
    let parsed = response.as_deref().and_then(|frame| reply(frame, command));
    trace::record(interface, BackendId::HyperX, match command {
        Query::Connection => "alpha-wireless-connection", Query::Battery => "alpha-wireless-battery",
        Query::Charging => "alpha-wireless-charging",
    }, 0x21bb, command.command(), usize::from(REPORT_LENGTH), response.as_ref().map_or(0, Vec::len),
        started, match parsed {
            Some(Reply::Connection(false)) => trace::Outcome::Offline,
            Some(Reply::Battery(None) | Reply::Charging(None)) => trace::Outcome::InvalidData,
            Some(_) => trace::Outcome::Ok, None => trace::Outcome::NoReply,
        });
    parsed
}

fn sample(mut query: impl FnMut(Query) -> Option<Reply>) -> Option<Sample> {
    let connected = match query(Query::Connection) {
        Some(Reply::Connection(connected)) => connected, _ => return None,
    };
    let mut result = Sample { connected, battery: None, measured: None };
    if !connected { return Some(result); }
    let percent = match query(Query::Battery) {
        Some(Reply::Battery(Some(percent))) => Some((percent, Instant::now())), _ => None,
    };
    let charging = match query(Query::Charging) {
        Some(Reply::Charging(Some(charging))) => Some((charging, Instant::now())), _ => None,
    };
    let measured = percent.map(|(_, measured)| measured)
        .or_else(|| charging.filter(|(charging, _)| *charging).map(|(_, measured)| measured));
    if measured.is_none() { return Some(result); }
    match query(Query::Connection) {
        Some(Reply::Connection(true)) => {
            result.battery = Some(Battery { percent: percent.map(|(percent, _)| percent),
                estimated_percent: None, charging: charging.map(|(charging, _)| charging),
                voltage_mv: None, approximate: false, level: None });
            result.measured = measured;
        },
        Some(Reply::Connection(false)) => result.connected = false,
        _ => (),
    }
    Some(result)
}

fn read_with(target: &Target, query: impl FnMut(Query) -> Option<Reply>) -> Result<Snapshot, String> {
    sample(query).map(|sample| snapshot(target, sample))
        .ok_or_else(|| crate::i18n::translate("设备无有效状态回报").into())
}

pub(super) fn read(api: &HidApi, target: &Target) -> Result<Snapshot, String> {
    let interface = target.hid()?;
    if !proven_with(interface, |interface| report_ids(api, interface)) {
        return Err(crate::i18n::translate("设备无有效状态回报").into());
    }
    let lock = channel::lock(&interface.path);
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut device = HidDevice::open(interface)?;
    let deadline = Instant::now() + Duration::from_secs(3);
    read_with(target, |command| query(&mut device, interface, command, deadline))
}

#[cfg(test)]
mod tests {
    //! All interfaces, reports and query transcripts below are synthetic, not hardware captures.
    use super::*;

    fn interface(instance: &str) -> HidInterface {
        HidInterface {
            path: format!(r"\\?\hid#vid_03f0&pid_098d&mi_03&col01#8&{instance}&0&0000#{{fixture}}"),
            vendor_id: 0x03f0, product_id: 0x098d, usage_page: 0xff43, usage: 0x0202,
            input_len: REPORT_LENGTH, output_len: REPORT_LENGTH, feature_len: 0,
            product: "HyperX Cloud Alpha Wireless".into(), serial: "00000001".into(),
        }
    }

    fn frame(query: Query, value: u8) -> [u8; REPORT_LENGTH as usize] {
        let mut frame = query.packet();
        frame[3] = value;
        frame
    }

    fn target(instance: &str) -> Target {
        Target::new(BackendId::HyperX, &interface(instance), 0)
    }

    fn successful(query: Query) -> Option<Reply> {
        reply(&frame(query, match query {
            Query::Connection => 2, Query::Battery => 61, Query::Charging => 1,
        }), query)
    }

    #[test]
    fn exact_model_collection_and_windows_capacities_gate_descriptor_access() {
        let valid = interface("first");
        assert!(control(&valid));
        let mut upper = valid.clone(); upper.path = upper.path.to_uppercase();
        assert!(control(&upper));
        for mutate in [
            |candidate: &mut HidInterface| candidate.vendor_id = 0x0951,
            |candidate: &mut HidInterface| candidate.product_id = 0x1743,
            |candidate: &mut HidInterface| candidate.usage_page = 0xff13,
            |candidate: &mut HidInterface| candidate.usage = 1,
            |candidate: &mut HidInterface| candidate.input_len = 32,
            |candidate: &mut HidInterface| candidate.output_len = 64,
            |candidate: &mut HidInterface| candidate.feature_len = 31,
            |candidate: &mut HidInterface| candidate.path = candidate.path.replace("mi_03", "mi_00"),
            |candidate: &mut HidInterface| candidate.path = candidate.path.replace("col01", "col02"),
            |candidate: &mut HidInterface| candidate.path = candidate.path.replace("pid_098d", "pid_05b7"),
            |candidate: &mut HidInterface| candidate.path = candidate.path.replace("mi_03", "mi_03&mi_00"),
            |candidate: &mut HidInterface| candidate.path = candidate.path.replace("col01", "col01&col02"),
            |candidate: &mut HidInterface| candidate.path = candidate.path.replace("&0000#", "&zzzz#"),
            |candidate: &mut HidInterface| candidate.path = "unknown".into(),
        ] {
            let mut invalid = valid.clone(); mutate(&mut invalid);
            assert!(!proven_with(&invalid, |_| panic!("invalid interface must not be inspected")));
        }
        for length in [0, 20, 30, 32, 52, 62, 64, 96, 256] {
            let mut invalid = valid.clone(); invalid.input_len = length;
            assert!(!control(&invalid));
            invalid = valid.clone(); invalid.output_len = length;
            assert!(!control(&invalid));
        }
        for product in [0x0696, 0x018b, 0x08be, 0x0abe, 0x05b7, 0x0c9d, 0x089d, 0x03cc, 0x1765] {
            let mut other = valid.clone(); other.product_id = product;
            other.path = other.path.replace("098d", &format!("{product:04x}"));
            assert!(discover_with(&[other], |_| panic!("excluded model")).is_empty());
        }
    }

    #[test]
    fn descriptor_failure_or_wrong_report_id_never_proves_the_channel() {
        let valid = interface("first");
        for proof in [None, Some((0, 0)), Some((0x21, 0)), Some((0, 0x21)), Some((6, 6))] {
            assert!(!proven_with(&valid, |_| proof));
            assert!(discover_with(&[valid.clone()], |_| proof).is_empty());
        }
        let discovered = discover_with(&[valid], |_| Some((REPORT_ID, REPORT_ID)));
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].name, "HyperX Cloud Alpha Wireless");
        assert_eq!(discovered[0].family, DeviceFamily::Headset);
        assert_eq!(discovered[0].address, 0);
    }

    #[test]
    fn ambiguous_collections_are_counted_before_descriptor_or_capacity_filtering() {
        let first = interface("first");
        let second = interface("second");
        let mut sibling = first.clone();
        sibling.path = sibling.path.replace("col01", "col02").replace("0000#", "0001#");
        sibling.input_len = 64;
        assert_eq!(control_group(&first), control_group(&sibling));
        for candidates in [vec![first.clone(), sibling.clone(), second.clone()],
            vec![second.clone(), sibling, first.clone()], vec![first.clone(), first, second.clone()]] {
            let discovered = discover_with(&candidates, |candidate| {
                assert_eq!(candidate.path, second.path);
                Some((REPORT_ID, REPORT_ID))
            });
            assert_eq!(discovered.len(), 1);
            assert_eq!(discovered[0].channel_key(), second.path);
        }
    }

    #[test]
    fn same_serial_receivers_keep_keys_locks_and_read_failures_separate() {
        let first = interface("first");
        let second = interface("second");
        let targets = discover_with(&[first.clone(), second.clone()], |_| Some((REPORT_ID, REPORT_ID)));
        assert_eq!(first.serial, second.serial);
        assert_eq!(targets.len(), 2);
        assert_ne!(targets[0].key, targets[1].key);
        assert!(!targets[0].same_channel(&targets[1]));
        assert!(!std::sync::Arc::ptr_eq(&channel::lock(&first.path), &channel::lock(&second.path)));
        assert!(read_with(&targets[0], |_| None).is_err());
        let reading = read_with(&targets[1], successful).unwrap();
        assert_eq!(reading.target.key, targets[1].key);
        assert_eq!(reading.battery.unwrap().percent, Some(61));
        let remaining = discover_with(&[first, second.clone()], |candidate|
            (candidate.path == second.path).then_some((REPORT_ID, REPORT_ID)));
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].key, targets[1].key);
    }

    #[test]
    fn packets_only_query_status_and_include_numbered_report_id_once() {
        let mut packets = Vec::new();
        let reading = read_with(&target("first"), |query| {
            packets.push(query.packet()); successful(query)
        }).unwrap();
        assert_eq!(packets.len(), 4);
        for (packet, command) in packets.iter().zip([3, 0x0b, 0x0c, 3]) {
            assert_eq!(packet.len(), 31);
            assert_eq!(&packet[..3], &[0x21, 0xbb, command]);
            assert!(packet[3..].iter().all(|byte| *byte == 0));
        }
        let battery = reading.battery.unwrap();
        assert_eq!((battery.percent, battery.charging), (Some(61), Some(true)));
        assert_eq!((battery.estimated_percent, battery.voltage_mv, battery.level), (None, None, None));
        assert!(!battery.approximate);
        assert_eq!(reading.dpi, Dpi::default());
        assert!(!reading.polling_rate.writable);
    }

    #[test]
    fn short_long_wrong_id_wrong_command_and_notifications_cannot_be_query_replies() {
        for query in [Query::Connection, Query::Battery, Query::Charging] {
            let valid = frame(query, 1);
            for length in 0..valid.len() { assert_eq!(reply(&valid[..length], query), None); }
            let mut long = valid.to_vec(); long.push(0);
            assert_eq!(reply(&long, query), None);
            let mut prefixed = vec![0]; prefixed.extend_from_slice(&valid);
            assert_eq!(reply(&prefixed, query), None);
            for offset in 0..3 {
                let mut wrong = valid; wrong[offset] ^= 0x80;
                assert_eq!(reply(&wrong, query), None);
            }
            for command in [0x0a, 0x10, 0x11, 0x12, 0x13, 0x15, 0x23, 0x24, 0x25, 0x26] {
                let mut other = valid; other[2] = command;
                assert_eq!(reply(&other, query), None);
            }
            assert_eq!(reply(&[0; 31], query), None);
            assert_eq!(reply(&[0xff; 31], query), None);
        }
        assert_eq!(reply(&frame(Query::Battery, 60), Query::Charging), None);
        assert_eq!(reply(&frame(Query::Charging, 1), Query::Battery), None);
    }

    #[test]
    fn zero_echo_and_invalid_values_never_become_zero_percent_or_offline() {
        for value in 0..=255 {
            let expected = (value != 0).then_some(Reply::Battery((1..=100).contains(&value).then_some(value)));
            assert_eq!(reply(&frame(Query::Battery, value), Query::Battery), expected);
            let charging = match value { 0 => None, 1 => Some(Reply::Charging(Some(true))),
                _ => Some(Reply::Charging(None)) };
            assert_eq!(reply(&frame(Query::Charging, value), Query::Charging), charging);
            let connected = match value { 1 => Some(Reply::Connection(false)),
                2 => Some(Reply::Connection(true)), _ => None };
            assert_eq!(reply(&frame(Query::Connection, value), Query::Connection), connected);
        }
        assert!(read_with(&target("first"), |query| reply(&query.packet(), query)).is_err());
        let reading = read_with(&target("first"), |query| match query {
            Query::Connection => successful(query), _ => reply(&query.packet(), query),
        }).unwrap();
        assert!(reading.connected && !reading.battery_unavailable);
        assert!(reading.battery.is_none() && reading.battery_at.is_none());
    }

    #[test]
    fn valid_percentage_with_only_charging_request_echo_keeps_charging_unknown() {
        let mut commands = Vec::new();
        let reading = read_with(&target("first"), |query| {
            commands.push(query);
            if query == Query::Charging { reply(&query.packet(), query) } else { successful(query) }
        }).unwrap();
        assert_eq!(commands, [Query::Connection, Query::Battery, Query::Charging, Query::Connection]);
        assert!(reading.connected && reading.battery_at.is_some() && !reading.battery_unavailable);
        let battery = reading.battery.unwrap();
        assert_eq!((battery.percent, battery.charging), (Some(61), None));
    }

    #[test]
    fn charging_request_echo_does_not_finish_matching_before_a_real_charging_reply() {
        let mut matched = Vec::new();
        let reading = read_with(&target("first"), |query| {
            if query != Query::Charging { return successful(query); }
            let response = [query.packet(), frame(query, 1)].into_iter().find(|response| {
                let accepted = reply(response, query).is_some();
                matched.push(accepted);
                accepted
            });
            response.and_then(|response| reply(&response, query))
        }).unwrap();
        assert_eq!(matched, [false, true]);
        assert!(reading.connected && reading.battery_at.is_some());
        let battery = reading.battery.unwrap();
        assert_eq!((battery.percent, battery.charging), (Some(61), Some(true)));
    }

    #[test]
    fn explicit_offline_stops_queries_but_silence_cannot_invent_disconnection() {
        let reading = read_with(&target("first"), |query| {
            assert_eq!(query, Query::Connection); reply(&frame(query, 1), query)
        }).unwrap();
        assert!(!reading.connected && reading.battery_unavailable);
        assert!(reading.battery.is_none() && reading.battery_at.is_none());
        assert!(read_with(&target("first"), |_| None).is_err());
        let reading = read_with(&target("first"), |query|
            if query == Query::Connection { successful(query) } else { None }).unwrap();
        assert!(reading.connected && !reading.battery_unavailable);
        assert!(reading.battery.is_none() && reading.battery_at.is_none());
    }

    #[test]
    fn connection_must_be_confirmed_after_the_measurement() {
        for last in [None, Some(false)] {
            let mut connections = 0;
            let reading = read_with(&target("first"), |query| {
                if query == Query::Connection {
                    connections += 1;
                    if connections == 2 { return last.map(Reply::Connection); }
                }
                successful(query)
            }).unwrap();
            assert_eq!(connections, 2);
            assert_eq!(reading.connected, last.is_none());
            assert_eq!(reading.battery_unavailable, last == Some(false));
            assert!(reading.battery.is_none() && reading.battery_at.is_none());
        }
    }

    #[test]
    fn charging_failure_keeps_unknown_and_charge_only_cannot_reuse_percentage() {
        let missing_charge = read_with(&target("first"), |query|
            if query == Query::Charging { None } else { successful(query) }).unwrap();
        let battery = missing_charge.battery.unwrap();
        assert_eq!((battery.percent, battery.charging), (Some(61), None));
        for charge in [0, 1, 2, 0xff] {
            let reading = read_with(&target("first"), |query| match query {
                Query::Connection => successful(query), Query::Battery => None,
                Query::Charging => reply(&frame(query, charge), query),
            }).unwrap();
            assert!(reading.connected);
            if charge == 1 {
                let battery = reading.battery.unwrap();
                assert_eq!((battery.percent, battery.charging), (None, Some(true)));
                assert!(reading.battery_at.is_some());
            } else { assert!(reading.battery.is_none() && reading.battery_at.is_none()); }
        }
    }

    #[test]
    fn later_charging_and_connection_checks_do_not_move_percentage_measurement_time() {
        let mut charge_started = None;
        let mut final_connection_started = None;
        let mut connections = 0;
        let reading = read_with(&target("first"), |query| {
            if query == Query::Charging { charge_started = Some(Instant::now()); }
            if query == Query::Connection {
                connections += 1;
                if connections == 2 { final_connection_started = Some(Instant::now()); }
            }
            successful(query)
        }).unwrap();
        assert!(reading.battery_at.unwrap() <= charge_started.unwrap());
        assert!(reading.battery_at.unwrap() <= final_connection_started.unwrap());
    }

    #[test]
    fn repeated_publication_partial_reads_and_failures_do_not_extend_sample_lifetime() {
        let reading = read_with(&target("first"), successful).unwrap();
        let measured = reading.battery_at.unwrap();
        let lifetime = BackendId::HyperX.sample_lifetime();
        let mut status = crate::backends::status::Status::default();
        for elapsed in [Duration::ZERO, Duration::from_secs(10), lifetime] {
            status.observe_sample(Some(true), reading.battery.as_ref(), reading.battery_at,
                measured + elapsed, lifetime);
            assert_eq!(status.measured_at, Some(measured));
        }
        assert_eq!(status.quality, crate::device::ReadingQuality::Stale);
        let partial = read_with(&reading.target, |query|
            if query == Query::Connection { successful(query) } else { None }).unwrap();
        status.observe_sample(Some(partial.connected), partial.battery.as_ref(), partial.battery_at,
            measured + lifetime, lifetime);
        assert_eq!(status.measured_at, Some(measured));
        assert_eq!(status.quality, crate::device::ReadingQuality::Stale);
        assert!(read_with(&reading.target, |_| None).is_err());
        assert!(partial.battery.is_none() && partial.battery_at.is_none());
    }
}
