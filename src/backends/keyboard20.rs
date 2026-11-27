//! Read-only Report 0x13 keyboard status. Transport evidence is in docs/PROTOCOLS.md.
use super::*;
use crate::hid::HidDevice;
use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

pub struct Keyboard20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Model {
    SharedReceiver,
    IdentifiedReceiver,
    Z75sCandidate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wire {
    Aula,
    Mchose,
}

#[derive(Clone, Copy)]
struct WireHint {
    wire: Wire,
    seen: Instant,
}

fn wire_hints() -> &'static Mutex<BTreeMap<String, WireHint>> {
    static HINTS: OnceLock<Mutex<BTreeMap<String, WireHint>>> = OnceLock::new();
    HINTS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn wire_key(i: &HidInterface) -> String {
    crate::device::stable_key(&channel::collection_group(i))
}

fn wire_hint(i: &HidInterface) -> Option<Wire> {
    let mut hints = wire_hints().lock().unwrap_or_else(|p| p.into_inner());
    let key = wire_key(i);
    let hint = hints.get(&key).copied();
    if hint.is_some_and(|h| h.seen.elapsed() >= Duration::from_secs(120)) {
        hints.remove(&key);
        return None;
    }
    hint.map(|h| h.wire)
}

fn remember_wire(i: &HidInterface, wire: Wire) {
    wire_hints()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(
            wire_key(i),
            WireHint {
                wire,
                seen: Instant::now(),
            },
        );
}

fn model(i: &HidInterface) -> Option<Model> {
    if !matches!(i.usage_page, 0xff02 | 0xff04) || i.usage != 2 || i.input_len != 20 || i.output_len != 20 {
        return None;
    }
    match (i.vendor_id, i.product_id) {
        (0x3554, 0xfa09) => Some(Model::SharedReceiver),
        (0x41e4, 0x2001) if i.usage_page == 0xff04 && generic_receiver(&i.product) => Some(Model::IdentifiedReceiver),
        (0x258a, 0x01bb) if i.usage_page == 0xff02 => Some(Model::Z75sCandidate),
        _ => None,
    }
}

fn generic_receiver(name: &str) -> bool {
    matches!(name, "2.4G Wireless Receiver" | "MCHOSE 2.4G Wireless")
}

fn name(i: &HidInterface) -> String {
    // Product substrings and the shared FA09 ID cannot establish the brand.
    if generic_receiver(&i.product) || i.product.trim().is_empty() {
        format!("2.4G {}", crate::tr!("键盘"))
    } else {
        i.product.clone()
    }
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|i| model(i).is_some() && interfaces.iter().filter(|other|
        other.vendor_id == i.vendor_id && other.product_id == i.product_id
            && matches!(other.usage_page, 0xff02 | 0xff04) && other.usage == 2
            && other.input_len == 20 && other.output_len == 20
            && channel::collection_group(i) == channel::collection_group(other)).count() == 1).collect()
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    model(i).is_some() && descriptor::Descriptor::open(api, i)
        .is_some_and(|caps| caps.only_input_output_report(i, 0x13, 20))
}

fn checksum(frame: &[u8]) -> u8 {
    frame.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte))
}

pub(super) fn query(command: u8) -> [u8; 20] {
    let mut frame = [0; 20];
    frame[..3].copy_from_slice(&[0x13, command, 1]);
    frame[19] = checksum(&frame[..19]);
    frame
}

fn aula_query(command: u8) -> [u8; 20] {
    let mut frame = [0; 20];
    frame[..3].copy_from_slice(&[0x13, command, 1]);
    if command == 5 {
        // AULA's identity checksum covers the report ID and header but sits
        // before the final padding. Its battery request carries no checksum.
        frame[8] = checksum(&frame[..8]);
    }
    frame
}

fn reply(frame: &[u8], command: u8, length: u8) -> bool {
    frame.len() == 20 && frame[..5] == [0x13, command, 1, 0, length]
        && frame[19] == checksum(&frame[..19])
}

// Six-byte firmware model identifiers, independently checked against M HUB's
// keyboardPwds table. These are identification data, not authentication keys.
const MODEL_CODES: [([u8; 6], &str); 18] = [
    ([3, 0, 0, 0, 2, 0x1c], "MCHOSE G87"),
    ([3, 0, 0, 0, 0, 0x77], "MCHOSE K99"),
    ([3, 0, 0, 0, 1, 0xdf], "MCHOSE K99 V2"),
    ([3, 0, 0, 0, 1, 0x32], "MCHOSE G98"),
    ([3, 0, 0, 0, 0, 0xd6], "MCHOSE X75"),
    ([10, 0, 0, 0, 0, 3], "MCHOSE G98 Pro"),
    ([3, 0, 0, 0, 1, 0xc6], "MCHOSE X75 V2"),
    ([3, 0, 0, 0, 1, 0x15], "MCHOSE G75 Pro"),
    ([10, 0, 0, 0, 0, 0x18], "MCHOSE G98 V2"),
    ([10, 0, 0, 0, 0, 0x2b], "MCHOSE UT98"),
    ([10, 0, 0, 0, 0, 1], "MCHOSE Z75"),
    ([10, 0, 0, 0, 0, 9], "MCHOSE Z75S"),
    ([3, 0, 0, 0, 1, 0xc7], "MCHOSE KX75"),
    ([3, 0, 0, 0, 1, 0x74], "MCHOSE GX87"),
    ([3, 0, 0, 0, 1, 0xed], "MCHOSE GX87 V2"),
    ([3, 0, 0, 0, 1, 0x35], "MCHOSE K87"),
    ([3, 0, 0, 0, 3, 0x35], "MCHOSE G75 V2"),
    ([10, 0, 0, 0, 0, 0x34], "MCHOSE G87 V2"),
];

const AULA_MODEL_CODES: [([u8; 6], &str); 2] = [
    ([3, 0, 0, 0, 0, 0xcd], "AULA F75"),
    ([3, 0, 0, 0, 3, 0xe3], "AULA F87 PRO"),
];

fn by_reply(frame: &[u8], command: u8) -> bool {
    // M HUB reassembles one packet by total/index and takes bytes 5..19.
    // Byte 4 is packet metadata; neither its length meaning nor a reply checksum
    // is established for shared BY receivers. Keep VGN's strict decoder separate.
    frame.len() == 20 && frame[..4] == [0x13, command, 1, 0] && frame != query(command)
}

fn model_name(frame: &[u8]) -> Option<&'static str> {
    if !by_reply(frame, 5) { return None; }
    MODEL_CODES.iter().find_map(|(code, name)| (frame[5..11] == *code).then_some(*name))
}

fn aula_model_name(frame: &[u8]) -> Option<&'static str> {
    if !by_reply(frame, 5) {
        return None;
    }
    AULA_MODEL_CODES
        .iter()
        .find_map(|(code, name)| (frame[5..11] == *code).then_some(*name))
}

pub(super) fn battery(frame: &[u8]) -> Option<Battery> {
    if !reply(frame, 0x4a, 2) || frame[5] > 100 { return None; }
    Some(Battery {
        estimated_percent: None,
        percent: Some(frame[5]),
        charging: match frame[6] >> 4 { 0 => Some(false), 1 => Some(true), _ => None },
        voltage_mv: None,
        approximate: false,
        level: None,
    })
}

fn by_battery(frame: &[u8], identified: bool) -> Option<Battery> {
    let (percent, flags) = if by_reply(frame, 0x4a) {
        (frame[5], frame[6])
    } else if identified && frame.len() == 20 && frame[..2] == [0x13, 0x0a] && frame[5] == 5 {
        // M HUB's subscribed battery notification skips bytes 2..4. It is not
        // an answer to the identity query, and must not identify unknown FA09s.
        (frame[6], frame[7])
    } else { return None; };
    if !(1..=100).contains(&percent) { return None; }
    let charging = if identified {
        if flags & 15 > 1 || flags >> 4 > 1 { return None; }
        Some(flags >> 4 == 1 && percent < 100)
    } else { None };
    Some(Battery { percent: Some(percent), estimated_percent: None, charging,
        voltage_mv: None, approximate: false, level: None })
}

fn aula_battery(frame: &[u8]) -> Option<Battery> {
    if !by_reply(frame, 0x4a) {
        return None;
    }
    let percent = frame[5];
    let charging = frame[6] >> 4;
    let full = frame[6] & 15;
    if !(1..=100).contains(&percent) || charging > 1 || full > 1 {
        return None;
    }
    Some(Battery {
        percent: Some(percent),
        estimated_percent: None,
        charging: Some(charging == 1),
        voltage_mv: None,
        approximate: false,
        level: None,
    })
}

fn keyboard_battery(frame: &[u8], selected: Model, identified: bool) -> Option<Battery> {
    if selected == Model::Z75sCandidate {
        battery(frame).filter(|b| b.percent != Some(0))
    } else {
        by_battery(frame, identified)
    }
}

fn read_with(target: &Target, mut exchange: impl FnMut(u8, bool) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    let fail = || crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试").to_string();
    let selected = model(target.hid()?).ok_or_else(fail)?;
    let mut target = target.clone();
    let identify = selected != Model::SharedReceiver || generic_receiver(&target.hid()?.product)
        || target.hid()?.product.starts_with("MCHOSE ");
    let identified = if identify {
        exchange(5, false).as_deref().filter(|r| selected != Model::Z75sCandidate || reply(r, 5, 6)).and_then(model_name)
    } else { None };
    match selected {
        Model::Z75sCandidate if identified != Some("MCHOSE Z75S") => return Err(fail()),
        Model::IdentifiedReceiver if identified.is_none() => return Err(fail()),
        _ => (),
    }
    target.verified_identity = false;
    if let Some(name) = identified {
        target.name = name.into();
        target.verified_identity = true;
    } else {
        target.name = name(target.hid()?);
    }
    let response = exchange(0x4a, identified.is_some()).ok_or_else(fail)?;
    let battery = keyboard_battery(&response, selected, identified.is_some()).ok_or_else(fail)?;
    // Some live receivers return zero for an asleep keyboard. This wireless
    // path has no independent zero-battery confirmation. VGN's separate reader
    // keeps its own policy and still uses the shared frame decoder above.
    Ok(Snapshot {
        target, battery: Some(battery), battery_unavailable: false, battery_at: Some(Instant::now()),
        dpi: Dpi::default(), polling_rate: Default::default(), connected: true,
    })
}

fn read_aula_with(
    target: &Target,
    first_identity: &[u8],
    mut exchange: impl FnMut(u8) -> Option<Vec<u8>>,
) -> Result<Snapshot, String> {
    let fail = || crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试").to_string();
    let name = aula_model_name(first_identity).ok_or_else(fail)?;
    let battery = exchange(0x4a)
        .and_then(|r| aula_battery(&r))
        .ok_or_else(fail)?;
    let after = exchange(5).ok_or_else(fail)?;
    if aula_model_name(&after) != Some(name) {
        return Err(fail());
    }
    let mut target = target.clone();
    target.name = name.into();
    target.verified_identity = true;
    Ok(Snapshot {
        target,
        battery: Some(battery),
        battery_at: Some(Instant::now()),
        battery_unavailable: false,
        connected: true,
        dpi: Dpi::default(),
        polling_rate: Default::default(),
    })
}

fn control_query(
    device: &mut HidDevice,
    target: &Target,
    packet: &[u8],
    label: &'static str,
    matches: impl FnMut(&[u8]) -> bool,
) -> Option<Vec<u8>> {
    let started = Instant::now();
    let response = device.transact_control_output(packet, 2200, matches);
    trace::record(
        target.hid().ok()?,
        BackendId::Keyboard20,
        label,
        0x13,
        packet[1],
        20,
        response.as_ref().map_or(0, Vec::len),
        started,
        if response.is_some() {
            trace::Outcome::Ok
        } else {
            trace::Outcome::NoReply
        },
    );
    response
}

fn read_mchose(
    device: &mut HidDevice,
    target: &Target,
    selected: Model,
) -> Result<Snapshot, String> {
    read_with(target, |command, identified| {
        let started = Instant::now();
        let matches = |r: &[u8]| {
            if command == 5 {
                if selected == Model::Z75sCandidate {
                    reply(r, 5, 6)
                } else {
                    by_reply(r, 5)
                }
            } else {
                keyboard_battery(r, selected, identified).is_some()
            }
        };
        // Keep FA09's captured SET_REPORT transport. Z75S uses interrupt
        // OUT; the new 41E4 path follows the published BY read sequence.
        // Never fall back to ordinary key input or a feature-only sibling.
        let response = match selected {
            Model::SharedReceiver => {
                device.transact_control_output(&query(command), 2200, matches)
            }
            Model::Z75sCandidate | Model::IdentifiedReceiver => {
                device.transact(&query(command), 2200, matches)
            }
        };
        trace::record(
            target.hid().ok()?,
            BackendId::Keyboard20,
            "query-keyboard",
            0x13,
            command,
            20,
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

impl DeviceBackend for Keyboard20 {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i)).map(|i| {
            let mut target = Target::new(BackendId::Keyboard20, i, 0);
            target.name = name(i);
            target.family = DeviceFamily::Keyboard;
            target
        }).collect())
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        if !proven(api, target.hid()?) { return Err(crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试").into()); }
        let selected = model(target.hid()?)
            .ok_or_else(|| crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试"))?;
        let lock = channel::lock(&channel::collection_group(target.hid()?));
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(target.hid()?)?;
        if selected != Model::SharedReceiver {
            return read_mchose(&mut device, target, selected);
        }

        let hint = wire_hint(target.hid()?);
        let mut order = [Wire::Aula, Wire::Mchose];
        if hint == Some(Wire::Mchose) {
            order.swap(0, 1);
        }
        let mut last_error = crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试").to_string();
        for wire in order {
            if wire == Wire::Mchose {
                match read_mchose(&mut device, target, selected) {
                    Ok(reading) => {
                        remember_wire(target.hid()?, Wire::Mchose);
                        return Ok(reading);
                    }
                    Err(error) => last_error = error,
                }
                continue;
            }

            let identity = control_query(
                &mut device,
                target,
                &aula_query(5),
                "query-aula-identity",
                |r| by_reply(r, 5),
            );
            let Some(identity) = identity.filter(|r| aula_model_name(r).is_some()) else {
                let error = crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试").to_string();
                // Once a definitive AULA model has been seen, a failed wake
                // response must not be reinterpreted by the MCHOSE framing.
                if hint == Some(Wire::Aula) {
                    return Err(error);
                }
                last_error = error;
                continue;
            };
            // A valid six-byte model makes this protocol definitive. If the
            // battery or wake-up read then fails, do not reinterpret it as
            // another brand merely to obtain a number.
            remember_wire(target.hid()?, Wire::Aula);
            return read_aula_with(target, &identity, |command| {
                let packet = aula_query(command);
                control_query(
                    &mut device,
                    target,
                    &packet,
                    "query-aula",
                    |r| {
                        if command == 5 {
                            by_reply(r, 5)
                        } else {
                            aula_battery(r).is_some()
                        }
                    },
                )
            });
        }
        Err(last_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface() -> HidInterface {
        HidInterface { path: "keyboard-a".into(), vendor_id: 0x3554, product_id: 0xfa09,
            usage_page: 0xff02, usage: 2, input_len: 20, output_len: 20, feature_len: 0,
            product: "2.4G Wireless Receiver".into(), serial: "shared".into() }
    }
    fn response(command: u8, payload: &[u8]) -> Vec<u8> {
        let mut frame = query(command);
        frame[4] = payload.len() as u8;
        frame[5..5 + payload.len()].copy_from_slice(payload);
        frame[19] = checksum(&frame[..19]);
        frame.to_vec()
    }
    fn aula_response(command: u8, payload: &[u8]) -> Vec<u8> {
        let mut frame = [0; 20];
        frame[..4].copy_from_slice(&[0x13, command, 1, 0]);
        frame[5..5 + payload.len()].copy_from_slice(payload);
        frame.to_vec()
    }

    #[test]
    fn aula_identity_and_battery_requests_use_different_checksum_positions() {
        let identity = aula_query(5);
        assert_eq!(&identity[..9], &[0x13, 5, 1, 0, 0, 0, 0, 0, 0x19]);
        assert!(identity[9..].iter().all(|b| *b == 0));
        let battery = aula_query(0x4a);
        assert_eq!(&battery[..4], &[0x13, 0x4a, 1, 0]);
        assert!(battery[4..].iter().all(|b| *b == 0));
        assert_eq!(query(5)[19], checksum(&query(5)[..19]));
        assert_ne!(aula_query(5), query(5));
    }

    #[test]
    fn aula_identity_requires_the_two_published_wireless_model_codes() {
        assert_eq!(
            aula_model_name(&aula_response(5, &[3, 0, 0, 0, 0, 0xcd])),
            Some("AULA F75")
        );
        assert_eq!(
            aula_model_name(&aula_response(5, &[3, 0, 0, 0, 3, 0xe3])),
            Some("AULA F87 PRO")
        );
        // F87's catalog entry uses a conflicting collection and is excluded.
        assert!(aula_model_name(&aula_response(5, &[3, 0, 0, 0, 0, 0x8f])).is_none());
        assert!(aula_model_name(&aula_response(5, &[3, 0, 0, 0, 0, 0xce])).is_none());
        assert!(aula_model_name(&aula_query(5)).is_none());
    }

    #[test]
    fn aula_battery_preserves_its_charging_rule_and_rejects_unknown_flags_or_zero() {
        let charging = aula_battery(&aula_response(0x4a, &[100, 0x10])).unwrap();
        assert_eq!((charging.percent, charging.charging), (Some(100), Some(true)));
        let full = aula_battery(&aula_response(0x4a, &[100, 0x01])).unwrap();
        assert_eq!(full.charging, Some(false));
        assert_eq!(
            aula_battery(&aula_response(0x4a, &[55, 0x11]))
                .unwrap()
                .charging,
            Some(true)
        );
        for (percent, flags) in [(0, 0), (101, 0), (55, 0x21), (55, 0x12)] {
            assert!(aula_battery(&aula_response(0x4a, &[percent, flags])).is_none());
        }
        assert!(aula_battery(&aula_query(0x4a)).is_none());
        // The MCHOSE adapter deliberately suppresses charging at exactly 100%.
        let mchose = response(0x4a, &[100, 0x10]);
        assert_eq!(by_battery(&mchose, true).unwrap().charging, Some(false));
    }

    #[test]
    fn aula_read_rechecks_identity_and_returns_model_with_live_battery() {
        let target = Target::new(BackendId::Keyboard20, &interface(), 0);
        let first = aula_response(5, &[3, 0, 0, 0, 0, 0xcd]);
        let mut commands = Vec::new();
        let reading = read_aula_with(&target, &first, |command| {
            commands.push(command);
            Some(if command == 0x4a {
                aula_response(0x4a, &[72, 0x10])
            } else {
                first.clone()
            })
        })
        .unwrap();
        assert_eq!(commands, [0x4a, 5]);
        assert_eq!(reading.target.name, "AULA F75");
        assert_eq!(reading.target.key, target.key);
        assert!(reading.target.verified_identity && reading.connected);
        let battery = reading.battery.unwrap();
        assert_eq!((battery.percent, battery.charging), (Some(72), Some(true)));
        assert!(reading.battery_at.is_some());
    }

    #[test]
    fn aula_read_rejects_replacement_between_battery_and_final_identity() {
        let target = Target::new(BackendId::Keyboard20, &interface(), 0);
        let first = aula_response(5, &[3, 0, 0, 0, 0, 0xcd]);
        let result = read_aula_with(&target, &first, |command| {
            Some(if command == 0x4a {
                aula_response(0x4a, &[64, 0])
            } else {
                aula_response(5, &[3, 0, 0, 0, 3, 0xe3])
            })
        });
        assert!(result.is_err());
    }

    #[test]
    fn captured_reply_distinguishes_full_charging_zero_and_unknown_flags() {
        // Voltide's M HUB capture: 13 4A 01 00 02 64 01 ... C5.
        let full = response(0x4a, &[100, 1]);
        assert_eq!(full[19], 0xc5);
        let b = battery(&full).unwrap();
        assert_eq!((b.percent, b.charging), (Some(100), Some(false)));
        let charging = battery(&response(0x4a, &[0, 0x10])).unwrap();
        assert_eq!((charging.percent, charging.charging), (Some(0), Some(true)));
        let unknown = battery(&response(0x4a, &[99, 0x21])).unwrap();
        assert_eq!((unknown.percent, unknown.charging), (Some(99), None));
        assert!(battery(&response(0x4a, &[101, 0])).is_none());
    }

    #[test]
    fn echo_corruption_wrong_command_address_and_truncation_are_not_measurements() {
        assert!(battery(&query(0x4a)).is_none());
        let valid = response(0x4a, &[64, 0]);
        for end in 0..20 { assert!(battery(&valid[..end]).is_none()); }
        for index in [0, 1, 2, 3, 4, 19] {
            let mut changed = valid.clone();
            changed[index] ^= 1;
            if index != 19 { changed[19] = checksum(&changed[..19]); }
            assert!(battery(&changed).is_none(), "field {index}");
        }
    }

    #[test]
    fn shared_pid_requires_identity_before_battery_and_never_accepts_other_model() {
        let mut i = interface();
        i.vendor_id = 0x258a;
        i.product_id = 0x01bb;
        let target = Target::new(BackendId::Keyboard20, &i, 0);
        let mut commands = Vec::new();
        let reading = read_with(&target, |command, _| {
            commands.push(command);
            Some(if command == 5 { response(5, &[10, 0, 0, 0, 0, 9]) }
                else { response(0x4a, &[57, 0]) })
        }).unwrap();
        assert_eq!(commands, [5, 0x4a]);
        assert_eq!(reading.target.name, "MCHOSE Z75S");
        assert!(reading.target.verified_identity);
        commands.clear();
        assert!(read_with(&target, |command, _| {
            commands.push(command);
            Some(response(5, &[10, 0, 0, 0, 0, 8]))
        }).is_err());
        assert_eq!(commands, [5]);
        assert!(read_with(&target, |_, _| None).is_err());
    }

    #[test]
    fn catalog_restricts_control_endpoint_and_keeps_shared_receiver_name_neutral() {
        let i = interface();
        assert_eq!(model(&i), Some(Model::SharedReceiver));
        assert!(!name(&i).contains("MCHOSE"));
        let mut other = i.clone();
        other.product = "MCHOSE GX87".into();
        assert_eq!(name(&other), "MCHOSE GX87");
        other.usage = 1;
        assert!(model(&other).is_none());
        other = i.clone(); other.input_len = 17;
        assert!(model(&other).is_none());
        other = i.clone(); other.product_id = 0xfa08;
        assert!(model(&other).is_none());
        other = i.clone(); other.path = "keyboard-b".into();
        let first = Target::new(BackendId::Keyboard20, &i, 0);
        let second = Target::new(BackendId::Keyboard20, &other, 0);
        assert_ne!(first.key, second.key);
        assert!(!first.same_channel(&second));
    }

    #[test]
    fn official_model_codes_are_complete_frame_matches_without_substring_names() {
        for (code, name) in MODEL_CODES {
            let valid = response(5, &code);
            assert_eq!(model_name(&valid), Some(name));
            for len in 0..20 { assert!(model_name(&valid[..len]).is_none()); }
            let mut metadata = valid.clone(); metadata[4] = 0; metadata[19] = 0;
            assert_eq!(model_name(&metadata), Some(name));
        }
        assert_eq!(model_name(&response(5, &[10, 0, 0, 0, 0, 9])), Some("MCHOSE Z75S"));
        assert_eq!(model_name(&response(5, &[3, 0, 0, 0, 1, 0x74])), Some("MCHOSE GX87"));
        assert!(model_name(&query(5)).is_none());
        assert!(model_name(&response(5, &[3, 0, 0, 0, 0, 9])).is_none());
        let mut other = interface(); other.product = "Other brand GX87-compatible".into();
        assert_eq!(name(&other), other.product);
    }

    #[test]
    fn fa09_identifies_each_poll_but_keeps_percentage_when_identity_is_unavailable() {
        let t = Target::new(BackendId::Keyboard20, &interface(), 0);
        for identity in [None, Some(query(5).to_vec()), Some(response(5, &[3, 0, 0, 0, 0, 9]))] {
            let mut commands = Vec::new();
            let sample = read_with(&t, |cmd, identified| {
                commands.push(cmd);
                assert!(!identified);
                if cmd == 5 { identity.clone() } else { Some(response(0x4a, &[76, 0x10])) }
            }).unwrap();
            assert_eq!(commands, [5, 0x4a]);
            assert_eq!(sample.battery.as_ref().unwrap().percent, Some(76));
            assert_eq!(sample.battery.as_ref().unwrap().charging, None);
            assert!(!sample.target.verified_identity);
            assert!(!sample.target.name.contains("MCHOSE"));
        }
        let sample = read_with(&t, |cmd, identified| Some(if cmd == 5 {
            assert!(!identified);
            response(5, &[3, 0, 0, 0, 1, 0x74])
        } else { assert!(identified); response(0x4a, &[76, 0x10]) })).unwrap();
        assert_eq!(sample.target.name, "MCHOSE GX87");
        assert_eq!(sample.target.key, t.key);
        assert!(sample.target.verified_identity);
        assert_eq!(sample.battery.unwrap().charging, Some(true));
    }

    #[test]
    fn additional_receiver_requires_verified_model_before_issuing_battery_query() {
        let mut i = interface(); i.vendor_id = 0x41e4; i.product_id = 0x2001; i.usage_page = 0xff04;
        let t = Target::new(BackendId::Keyboard20, &i, 0);
        for identity in [None, Some(query(5).to_vec()), Some(response(5, &[3, 0, 0, 0, 0, 9]))] {
            let mut commands = Vec::new();
            assert!(read_with(&t, |cmd, _| { commands.push(cmd); identity.clone() }).is_err());
            assert_eq!(commands, [5]);
        }
        let reading = read_with(&t, |cmd, _| Some(if cmd == 5 { response(5, &[3, 0, 0, 0, 2, 0x1c]) }
            else { response(0x4a, &[51, 0]) })).unwrap();
        assert_eq!(reading.target.name, "MCHOSE G87");
        i.usage_page = 0xff02; assert!(model(&i).is_none());
        i.usage_page = 0xff04; i.product.push_str(" Other"); assert!(model(&i).is_none());
        i.product = "MCHOSE 2.4G Wireless".into(); assert!(model(&i).is_some());
        i.product_id = 0x2203; assert!(model(&i).is_none());
    }

    #[test]
    fn zero_from_sleeping_keyboard_cannot_replace_a_fresh_percentage() {
        let t = Target::new(BackendId::Keyboard20, &interface(), 0);
        for percent in [0, 101, 255] {
            assert!(read_with(&t, |cmd, _| Some(if cmd == 5 {
                response(5, &[3, 0, 0, 0, 1, 0x74])
            } else { response(0x4a, &[percent, 0]) })).is_err());
        }
        let mut named = interface(); named.product = "Another brand Keyboard".into();
        let t = Target::new(BackendId::Keyboard20, &named, 0);
        let sample = read_with(&t, |cmd, _| {
            assert_eq!(cmd, 0x4a); Some(response(0x4a, &[46, 0x10]))
        }).unwrap();
        assert_eq!(sample.target.name, named.product);
        assert_eq!(sample.battery.unwrap().charging, None);
    }

    #[test]
    fn competing_management_collections_are_rejected_but_feature_only_marker_is_allowed() {
        let mut i = interface();
        i.path = r"\\?\hid#vid_3554&pid_fa09&mi_01&col02#8&unit-a&0002#{hid}".into();
        let mut sibling = i.clone(); sibling.usage_page = 0xff04;
        sibling.path = sibling.path.replace("col02", "col03").replace("&0002", "&0003");
        assert!(candidates(&[i.clone(), sibling.clone()]).is_empty());
        sibling.input_len = 0; sibling.output_len = 0; sibling.feature_len = 8;
        assert_eq!(candidates(&[i.clone(), sibling.clone()]).len(), 1);
        assert!(model(&sibling).is_none());
        let mut second = i.clone(); second.path = second.path.replace("unit-a", "unit-b");
        assert_eq!(candidates(&[i, second, sibling]).len(), 2);
    }

    #[test]
    fn published_frames_keep_unknown_identity_without_discarding_valid_generic_battery() {
        // MyFinder #476 complete recorded frames; do not shift the model bytes
        // to make them match the issue author's proposed G75 Pro identity.
        let identity = [0x13, 5, 1, 0, 3, 0, 0, 0, 1, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let full = [0x13, 0x4a, 1, 0, 2, 100, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert!(by_reply(&identity, 5));
        assert!(model_name(&identity).is_none());
        let t = Target::new(BackendId::Keyboard20, &interface(), 0);
        let sample = read_with(&t, |cmd, identified| {
            assert!(!identified);
            Some(if cmd == 5 { identity.to_vec() } else { full.to_vec() })
        }).unwrap();
        assert!(!sample.target.verified_identity);
        assert!(!sample.target.name.contains("G75"));
        let b = sample.battery.unwrap();
        assert_eq!((b.percent, b.charging), (Some(100), None));
        assert!(battery(&full).is_none()); // The separate VGN decoder stays strict.
    }

    #[test]
    fn recognized_model_notifications_are_battery_only_after_identification() {
        // Published #312 prefix; trailing bytes are synthetic test padding.
        let mut notification = [0; 20];
        notification[..8].copy_from_slice(&[0x13, 0x0a, 1, 0, 4, 5, 95, 1]);
        let b = by_battery(&notification, true).unwrap();
        assert_eq!((b.percent, b.charging), (Some(95), Some(false)));
        assert!(by_battery(&notification, false).is_none());
        assert!(model_name(&notification).is_none());
        notification[2] = 0xa7; notification[3] = 9; notification[4] = 0;
        assert!(by_battery(&notification, true).is_some());
        for len in 0..20 { assert!(by_battery(&notification[..len], true).is_none()); }
        for (index, value) in [(0, 9), (1, 0x0b), (5, 2), (6, 0), (6, 101), (7, 0x21), (7, 2)] {
            let mut wrong = notification; wrong[index] = value;
            assert!(by_battery(&wrong, true).is_none());
        }
        let t = Target::new(BackendId::Keyboard20, &interface(), 0);
        let sample = read_with(&t, |cmd, identified| {
            if cmd == 5 { Some(response(5, &[3, 0, 0, 0, 1, 0x15])) }
            else { assert!(identified); Some(notification.to_vec()) }
        }).unwrap();
        assert_eq!(sample.target.name, "MCHOSE G75 Pro");
        assert_eq!(sample.battery.unwrap().percent, Some(95));
    }

    #[test]
    fn by_framing_charge_flags_and_full_status_do_not_invent_values() {
        let mut valid = response(0x4a, &[65, 0x11]);
        valid[4] = 0; valid[19] = 0;
        let b = by_battery(&valid, true).unwrap();
        assert_eq!((b.percent, b.charging), (Some(65), Some(true)));
        valid[5] = 100;
        assert_eq!(by_battery(&valid, true).unwrap().charging, Some(false));
        for (index, value) in [(0, 9), (1, 5), (2, 2), (3, 1), (5, 0), (5, 101), (6, 0x21)] {
            let mut wrong = valid.clone(); wrong[index] = value;
            assert!(by_battery(&wrong, true).is_none());
        }
        valid[6] = 0xf2;
        assert_eq!(by_battery(&valid, false).unwrap().charging, None);
        assert!(by_battery(&query(0x4a), false).is_none());
        assert!(by_battery(&query(0x4a), true).is_none());
    }
}
