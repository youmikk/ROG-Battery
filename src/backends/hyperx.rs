//! Cloud Alpha 2 base-station battery queries, without audio settings writes.
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
    fn discover(&self, _: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(interfaces.iter().filter(|i| control(i)).map(|i| {
            let mut target = Target::new(BackendId::HyperX, i, 0);
            target.name = "HyperX Cloud Alpha 2 Wireless".into();
            target.family = DeviceFamily::Headset;
            target
        }).collect())
    }

    fn read(&self, _: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let interface = target.hid()?;
        if !control(interface) { return Err(crate::i18n::translate("设备无有效状态回报").into()); }
        let lock = channel::lock(&interface.path);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(interface)?;
        let sample = sample(|command| query(&mut device, interface, command))
            .ok_or_else(|| crate::i18n::translate("设备无有效状态回报"))?;
        Ok(snapshot(target, sample))
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
