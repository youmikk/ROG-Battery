use super::*;
use crate::hid::HidDevice;
use std::{sync::TryLockError, time::{Duration, Instant}};

pub struct Mojhon;

pub(super) const SAMPLE_LIFETIME: Duration = Duration::from_secs(120);
const LENGTH: u16 = 65;
const WIRE_LENGTH: usize = 64;
const IDENTITY: u8 = 0xe2;
const POWER: u8 = 0x04;
const ROUND_BUDGET: Duration = Duration::from_millis(3200);
const REPLY_TIMEOUT_MS: u32 = 1200;
const MAX_FRAMES: usize = 32;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(interface: &HidInterface) -> bool {
    interface.vendor_id == 0xe2b7 && interface.product_id == 0x122c
        && interface.usage_page == 0xffab && interface.usage == 1
}

fn receiver_group(interface: &HidInterface) -> Option<String> {
    if !management(interface) { return None; }
    let path = interface.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid"
        || parts[2].contains(['\\', '/']) || parts[2].split('&').any(str::is_empty)
        || parts[3] != "{4d1e55b2-f16f-11cf-88cb-001111000030}"
        || path.contains('\0') { return None; }
    let fields: Vec<_> = parts[1].split('&').collect();
    if !(3..=4).contains(&fields.len()) || fields[..3] != ["vid_e2b7", "pid_122c", "mi_02"] {
        return None;
    }
    if let Some(collection) = fields.get(3) {
        let number = collection.strip_prefix("col")?;
        if number.len() != 2 || number == "00"
            || !number.bytes().all(|byte| byte.is_ascii_hexdigit()) { return None; }
    }
    let (instance, suffix) = parts[2].rsplit_once('&')?;
    if instance.split('&').count() < 3 || suffix.len() != 4
        || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) { return None; }
    Some(format!("{}#{}#{instance}#{}", parts[0], fields[..3].join("&"), parts[3]))
}

fn control(interface: &HidInterface) -> bool {
    interface.input_len == LENGTH && interface.output_len == LENGTH && interface.feature_len == 0
        && receiver_group(interface).is_some()
        && interface.path.split('#').nth(1).is_some_and(|part|
            part.eq_ignore_ascii_case("vid_e2b7&pid_122c&mi_02"))
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|interface| control(interface)
        && interfaces.iter().filter(|other| management(other)
            && receiver_group(other) == receiver_group(interface)).count() == 1).collect()
}

fn descriptor_gate(interface: &HidInterface,
    proof: impl FnOnce(u16, u16) -> Option<(u8, u8)>) -> bool {
    control(interface) && proof(LENGTH, LENGTH) == Some((0, 0))
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    descriptor_gate(interface, |input, output| descriptor::Descriptor::open(api, interface)?
        .single_input_output_byte_report_ids(interface, input, output))
}

fn target(interface: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::Mojhon, interface, 0);
    target.name = if crate::i18n::english() { "MOJHON Controller" } else { "墨将手柄" }.into();
    target.family = DeviceFamily::Gamepad;
    target.verified_identity = false;
    target
}

fn valid_target(original: &Target) -> bool {
    original.backend == BackendId::Mojhon && original.address == 0
        && original.hid().is_ok_and(|interface| control(interface) && original.key == target(interface).key)
}

fn discover_with(interfaces: &[HidInterface], mut prove: impl FnMut(&HidInterface) -> bool) -> Vec<Target> {
    candidates(interfaces).into_iter().filter(|interface| prove(interface)).map(target).collect()
}

fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte))
}

fn request(register: u8) -> Option<[u8; LENGTH as usize]> {
    if !matches!(register, IDENTITY | POWER) { return None; }
    let mut packet = [0; LENGTH as usize];
    packet[1..4].copy_from_slice(&[0xa5, 4, register]);
    packet[4] = checksum(&packet[1..4]);
    Some(packet)
}

fn payload(response: &[u8], register: u8) -> Option<&[u8]> {
    let wire = match response.len() {
        WIRE_LENGTH => response,
        length if length == usize::from(LENGTH) && response[0] == 0 => &response[1..],
        _ => return None,
    };
    let length = usize::from(wire[1]);
    if wire[0] != 0xa5 || !(4..=WIRE_LENGTH).contains(&length) || wire[2] != register
        || checksum(&wire[..length - 1]) != wire[length - 1] { return None; }
    Some(&wire[3..length - 1])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark { Server, Dongle }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity { mark: Mark, prefix: [u8; 2] }

fn identity(response: &[u8]) -> Option<Identity> {
    let payload = payload(response, IDENTITY)?;
    if payload.len() != 12 || payload[2] != 0 { return None; }
    let mark = match &payload[3..] {
        b"C4_SERVER" => Mark::Server,
        b"C4_DONGLE" => Mark::Dongle,
        _ => return None,
    };
    Some(Identity { mark, prefix: [payload[0], payload[1]] })
}

fn battery(response: &[u8]) -> Option<Battery> {
    let payload = payload(response, POWER)?;
    if payload.len() != 2 || payload[0] > 100 { return None; }
    Some(Battery { percent: Some(payload[0]), estimated_percent: None, approximate: false,
        charging: match payload[1] { 0 => Some(false), 1 => Some(true), _ => None },
        voltage_mv: None, level: None })
}

fn matches_reply(packet: &[u8], response: &[u8]) -> bool {
    let Some(expected) = packet.get(3).and_then(|register| request(*register)) else { return false; };
    if packet != expected { return false; }
    match packet[3] {
        IDENTITY => identity(response).is_some(),
        POWER => battery(response).is_some(),
        _ => false,
    }
}

fn remaining(deadline: Instant, now: Instant) -> Option<u32> {
    let millis = deadline.checked_duration_since(now)?.as_millis().min(u128::from(REPLY_TIMEOUT_MS)) as u32;
    (millis > 0).then_some(millis)
}

trait Transport {
    fn pending(&mut self) -> Option<Vec<u8>>;
    fn exchange(&mut self, packet: &[u8], timeout: u32,
        accept: &mut dyn FnMut(&[u8]) -> bool) -> Option<Vec<u8>>;
}

impl Transport for HidDevice {
    fn pending(&mut self) -> Option<Vec<u8>> { self.read_report(0) }

    fn exchange(&mut self, packet: &[u8], timeout: u32,
        accept: &mut dyn FnMut(&[u8]) -> bool) -> Option<Vec<u8>> {
        self.transact_with_input_report(packet, 0, timeout, accept)
    }
}

fn exchange_with(transport: &mut impl Transport, packet: &[u8], timeout: u32,
    mut now: impl FnMut() -> Instant) -> Option<Vec<u8>> {
    let expected = request(*packet.get(3)?)?;
    if packet != expected || timeout == 0 || timeout > REPLY_TIMEOUT_MS { return None; }
    let deadline = now() + Duration::from_millis(u64::from(timeout));
    let mut drained = false;
    for _ in 0..MAX_FRAMES {
        remaining(deadline, now())?;
        if transport.pending().is_none() { drained = true; break; }
    }
    if !drained { return None; }
    let timeout = remaining(deadline, now())?;
    let mut count = 0;
    let mut invalid = None;
    let response = transport.exchange(packet, timeout, &mut |frame| {
        count += 1;
        if matches_reply(packet, frame) { return true; }
        invalid = Some(frame.to_vec());
        count >= MAX_FRAMES
    });
    if now() >= deadline {
        return response.filter(|frame| !matches_reply(packet, frame)).or(invalid);
    }
    response.or(invalid)
}

fn outcome(register: u8, response: Option<&[u8]>, valid: bool) -> trace::Outcome {
    if !valid {
        return if response.is_some() { trace::Outcome::InvalidData } else { trace::Outcome::NoReply };
    }
    if register == IDENTITY && response.and_then(identity).is_some_and(|identity| identity.mark == Mark::Dongle) {
        trace::Outcome::Offline
    } else {
        trace::Outcome::Ok
    }
}

fn snapshot(interface: &HidInterface, sample: Option<(Battery, Instant)>) -> Snapshot {
    let mut target = target(interface);
    target.name = if crate::i18n::english() { "MOJHON GALE2" } else { "墨将烈风2 GALE2" }.into();
    let connected = sample.is_some();
    let (battery, battery_at) = sample.map_or((None, None), |(battery, at)| (Some(battery), Some(at)));
    Snapshot { target, battery, battery_at, connected, battery_unavailable: !connected,
        dpi: Dpi::default(), polling_rate: Default::default() }
}

fn query_with(interface: &HidInterface, deadline: Instant, mut now: impl FnMut() -> Instant,
    mut exchange: impl FnMut(&[u8], u32) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    let mut query = |register| -> Result<(Vec<u8>, Instant), String> {
        let packet = request(register).ok_or(UNAVAILABLE)?;
        let started = now();
        let timeout = remaining(deadline, started).ok_or(UNAVAILABLE)?;
        let response = exchange(&packet, timeout);
        let received = now();
        let valid = received < deadline
            && received.saturating_duration_since(started) < Duration::from_millis(u64::from(timeout))
            && response.as_deref().is_some_and(|response| matches_reply(&packet, response));
        trace::record_sample(interface, BackendId::Mojhon, "query", 0, register, packet.len(),
            response.as_ref().map_or(0, Vec::len), started,
            outcome(register, response.as_deref(), valid),
            (valid && register == POWER).then_some(received));
        response.filter(|_| valid).map(|response| (response, received)).ok_or_else(|| UNAVAILABLE.into())
    };
    let before = identity(&query(IDENTITY)?.0).ok_or(UNAVAILABLE)?;
    if before.mark == Mark::Dongle { return Ok(snapshot(interface, None)); }
    let power = query(POWER);
    let after = identity(&query(IDENTITY)?.0).ok_or(UNAVAILABLE)?;
    if after.mark == Mark::Dongle { return Ok(snapshot(interface, None)); }
    if before != after { return Err(UNAVAILABLE.into()); }
    let (response, measured) = power?;
    Ok(snapshot(interface, Some((battery(&response).ok_or(UNAVAILABLE)?, measured))))
}

fn read_with(original: &Target, interfaces: &[HidInterface], deadline: Instant,
    mut now: impl FnMut() -> Instant, mut prove: impl FnMut(&HidInterface) -> bool,
    query: impl FnOnce(&HidInterface, Instant) -> Result<Snapshot, String>) -> Result<Snapshot, String> {
    if !valid_target(original) || remaining(deadline, now()).is_none() { return Err(UNAVAILABLE.into()); }
    let original_interface = original.hid()?;
    let interface = candidates(interfaces).into_iter().find(|interface|
        interface.path.eq_ignore_ascii_case(&original_interface.path)
            && interface.serial == original_interface.serial).ok_or(UNAVAILABLE)?;
    let lock = channel::lock(&receiver_group(interface).ok_or(UNAVAILABLE)?);
    let _guard = match lock.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return Err(UNAVAILABLE.into()),
    };
    if remaining(deadline, now()).is_none() || !prove(interface) || remaining(deadline, now()).is_none() {
        return Err(UNAVAILABLE.into());
    }
    let snapshot = query(interface, deadline)?;
    if now() >= deadline || !prove(interface) || now() >= deadline { return Err(UNAVAILABLE.into()); }
    Ok(snapshot)
}

impl DeviceBackend for Mojhon {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(discover_with(interfaces, |interface| proven(api, interface)))
    }

    fn read(&self, api: &HidApi, original: &Target, _: bool) -> Result<Snapshot, String> {
        let deadline = Instant::now() + ROUND_BUDGET;
        if !valid_target(original) { return Err(UNAVAILABLE.into()); }
        let interfaces = enumeration::scan(api)?.interfaces;
        read_with(original, &interfaces, deadline, Instant::now, |interface| proven(api, interface),
            |interface, deadline| {
                let mut device = HidDevice::open(interface)?;
                if !device.writable || device.input_len() != LENGTH || device.output_len() != LENGTH {
                    return Err(UNAVAILABLE.into());
                }
                query_with(interface, deadline, Instant::now, |packet, timeout| {
                    let timeout = remaining(deadline, Instant::now())?.min(timeout);
                    exchange_with(&mut device, packet, timeout, Instant::now)
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::ReadingQuality;
    use std::{cell::Cell, collections::VecDeque};

    const CAPTURED_SERVER: &str = "A510E258520043345F534552564552EE";
    const CAPTURED_POWER: &str = "A506041200C1";

    fn interface(instance: &str) -> HidInterface {
        HidInterface {
            path: format!(r"\\?\hid#vid_e2b7&pid_122c&mi_02#8&{instance}&0&0000#{{4d1e55b2-f16f-11cf-88cb-001111000030}}"),
            vendor_id: 0xe2b7, product_id: 0x122c, usage_page: 0xffab, usage: 1,
            input_len: LENGTH, output_len: LENGTH, feature_len: 0,
            product: "XBOX 360 For Windows".into(), serial: "shared serial".into(),
        }
    }

    fn recorded(hex: &str) -> Vec<u8> {
        let mut wire = vec![0; WIRE_LENGTH];
        for (index, byte) in wire.iter_mut().enumerate().take(hex.len() / 2) {
            *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).unwrap();
        }
        wire
    }

    fn synthetic(register: u8, payload: &[u8]) -> Vec<u8> {
        let length = payload.len() + 4;
        let mut wire = vec![0; WIRE_LENGTH];
        wire[..3].copy_from_slice(&[0xa5, length as u8, register]);
        wire[3..length - 1].copy_from_slice(payload);
        wire[length - 1] = checksum(&wire[..length - 1]);
        wire
    }

    fn dongle() -> Vec<u8> { synthetic(IDENTITY, b"\x58\x52\x00C4_DONGLE") }

    fn windows(wire: &[u8]) -> Vec<u8> {
        let mut frame = vec![0];
        frame.extend_from_slice(wire);
        frame
    }

    fn replay(interface: &HidInterface, replies: Vec<Option<Vec<u8>>>) -> (Result<Snapshot, String>, Vec<u8>) {
        let now = Instant::now();
        let mut replies = VecDeque::from(replies);
        let mut commands = Vec::new();
        let result = query_with(interface, now + ROUND_BUDGET, || now, |packet, timeout| {
            assert!((1..=REPLY_TIMEOUT_MS).contains(&timeout));
            commands.push(packet[3]);
            replies.pop_front().expect("unexpected request")
        });
        assert!(replies.is_empty());
        (result, commands)
    }

    fn online(interface: &HidInterface) -> Snapshot {
        replay(interface, vec![Some(recorded(CAPTURED_SERVER)), Some(recorded(CAPTURED_POWER)),
            Some(recorded(CAPTURED_SERVER))]).0.unwrap()
    }

    #[derive(Default)]
    struct Scripted {
        queued: VecDeque<Vec<u8>>,
        replies: VecDeque<Vec<u8>>,
        writes: Vec<Vec<u8>>,
        drains: usize,
        received: usize,
    }

    impl Transport for Scripted {
        fn pending(&mut self) -> Option<Vec<u8>> {
            self.drains += 1;
            self.queued.pop_front()
        }

        fn exchange(&mut self, packet: &[u8], _: u32,
            accept: &mut dyn FnMut(&[u8]) -> bool) -> Option<Vec<u8>> {
            self.writes.push(packet.to_vec());
            while let Some(frame) = self.replies.pop_front() {
                self.received += 1;
                if accept(&frame) { return Some(frame); }
            }
            None
        }
    }

    #[test]
    fn captured_reads_use_zero_slot_and_only_two_allowlisted_registers() {
        for (register, prefix) in [(IDENTITY, [0, 0xa5, 4, 0xe2, 0x8b]), (POWER, [0, 0xa5, 4, 4, 0xad])] {
            let packet = request(register).unwrap();
            assert_eq!(packet.len(), 65);
            assert_eq!(&packet[..5], prefix);
            assert!(packet[5..].iter().all(|byte| *byte == 0));
            assert!(!matches_reply(&packet, &packet));
            assert!(!matches_reply(&packet, &packet[1..]));
        }
        for register in 0..=255 {
            assert_eq!(request(register).is_some(), matches!(register, IDENTITY | POWER));
        }
    }

    #[test]
    fn captured_server_and_battery_replay_both_transport_layouts() {
        for frame in [recorded(CAPTURED_SERVER), windows(&recorded(CAPTURED_SERVER))] {
            assert_eq!(identity(&frame), Some(Identity { mark: Mark::Server, prefix: [0x58, 0x52] }));
            assert!(matches_reply(&request(IDENTITY).unwrap(), &frame));
            assert!(!matches_reply(&request(POWER).unwrap(), &frame));
        }
        for (hex, percent, charging) in [(CAPTURED_POWER, 18, false), ("A506041100C0", 17, false),
            ("A506042400D3", 36, false), ("A506041800C7", 24, false),
            ("A506041601C6", 22, true), ("A506042401D4", 36, true), ("A506042101D1", 33, true)] {
            for frame in [recorded(hex), windows(&recorded(hex))] {
                assert_eq!(battery(&frame), Some(Battery { percent: Some(percent), charging: Some(charging),
                    estimated_percent: None, approximate: false, voltage_mv: None, level: None }));
                assert!(!matches_reply(&request(IDENTITY).unwrap(), &frame));
            }
        }
    }

    #[test]
    fn zero_full_and_unknown_charge_are_independent() {
        for percent in [0, 1, 99, 100] {
            for charge in 0..=255 {
                let sample = battery(&synthetic(POWER, &[percent, charge])).unwrap();
                assert_eq!(sample.percent, Some(percent));
                assert_eq!(sample.charging, match charge { 0 => Some(false), 1 => Some(true), _ => None });
                assert_eq!((sample.level, sample.estimated_percent, sample.voltage_mv), (None, None, None));
            }
        }
        for percent in 101..=255 { assert!(battery(&synthetic(POWER, &[percent, 0])).is_none()); }
        for payload in [&[][..], &[0][..], &[100][..], &[18, 0, 0][..]] {
            assert!(battery(&synthetic(POWER, payload)).is_none());
        }
    }

    #[test]
    fn declared_length_checksum_header_and_register_are_required() {
        let power = recorded(CAPTURED_POWER);
        for length in 0..WIRE_LENGTH { assert!(battery(&power[..length]).is_none()); }
        let mut too_long = windows(&power);
        too_long.push(0);
        assert!(battery(&too_long).is_none());
        for offset in 0..6 {
            let mut corrupt = power.clone();
            corrupt[offset] ^= 1;
            assert!(battery(&corrupt).is_none());
        }
        for length in [0, 1, 2, 3, 4, 5, 7, 64, 65, 255] {
            let mut changed = power.clone();
            changed[1] = length;
            assert!(battery(&changed).is_none());
        }
        let mut long_header = power.clone();
        long_header[0] = 0xa4;
        long_header[5] = checksum(&long_header[..5]);
        assert!(battery(&long_header).is_none());
        assert!(battery(&synthetic(0x0b, &[18, 0])).is_none());
        for id in 1..=255 {
            let mut frame = windows(&power);
            frame[0] = id;
            assert!(battery(&frame).is_none());
        }
        let mut padding = power.clone();
        padding[6..].fill(0xff);
        assert_eq!(battery(&padding), battery(&power));
    }

    #[test]
    fn identity_requires_exact_mark_and_delimiter_not_a_model_substring() {
        assert_eq!(identity(&dongle()).unwrap().mark, Mark::Dongle);
        for payload in [&b"\x58\x52\x00C4_SERVER_EXTRA"[..], &b"\x58\x52\x00C4_SERVER\0"[..],
            &b"\x58\x52\x00C4_SERVEX"[..], &b"\x58\x52\x00c4_server"[..],
            &b"\x58\x52\x01C4_SERVER"[..], &b"\x58\x52C4_SERVER"[..], &b"\0C4_SERVER"[..],
            &b"\x58\x52\x00C4_SERV\xffR"[..], &b"\x58\x52\x00"[..], &b""[..]] {
            let frame = synthetic(IDENTITY, payload);
            assert!(identity(&frame).is_none());
            assert!(!matches_reply(&request(IDENTITY).unwrap(), &frame));
        }
        let mut changed_prefix = recorded(CAPTURED_SERVER);
        changed_prefix[3] ^= 1;
        changed_prefix[15] = checksum(&changed_prefix[..15]);
        assert_ne!(identity(&changed_prefix), identity(&recorded(CAPTURED_SERVER)));
    }

    #[test]
    fn model_confirmation_keeps_discovery_key_and_unverified_instance_identity() {
        let interface = interface("model-name");
        let neutral = target(&interface);
        assert!(!neutral.name.contains("GALE2") && !neutral.name.contains("XBOX"));
        let (result, commands) = replay(&interface, vec![Some(recorded(CAPTURED_SERVER)),
            Some(recorded(CAPTURED_POWER)), Some(recorded(CAPTURED_SERVER))]);
        let result = result.unwrap();
        assert_eq!(commands, [IDENTITY, POWER, IDENTITY]);
        assert_eq!(result.target.key, neutral.key);
        assert!(result.target.same_channel(&neutral));
        assert!(result.target.name.contains("GALE2"));
        assert!(!result.target.verified_identity && !neutral.verified_identity);
        assert_eq!(result.target.family, DeviceFamily::Gamepad);
        assert!(result.connected && result.battery_at.is_some() && !result.battery_unavailable);
        assert!(!result.dpi.writable && !result.dpi.stage_writable && !result.polling_rate.writable);
    }

    #[test]
    fn dongle_before_or_after_power_clears_battery_with_the_same_key() {
        let interface = interface("dongle-disconnect");
        let first = online(&interface);
        for replies in [vec![Some(dongle())], vec![Some(recorded(CAPTURED_SERVER)),
            Some(recorded(CAPTURED_POWER)), Some(dongle())],
            vec![Some(recorded(CAPTURED_SERVER)), None, Some(dongle())],
            vec![Some(recorded(CAPTURED_SERVER)), Some(synthetic(POWER, &[255, 0])), Some(dongle())]] {
            let expected = replies.len();
            let (result, commands) = replay(&interface, replies);
            let result = result.unwrap();
            assert_eq!(commands.len(), expected);
            assert!(!result.connected && result.battery_unavailable);
            assert!(result.battery.is_none() && result.battery_at.is_none());
            assert_eq!(result.target.key, first.target.key);
            assert!(!result.target.verified_identity && !first.target.verified_identity);
            let at = first.battery_at.unwrap();
            let mut state = status::Status::default();
            state.observe_sample(Some(true), first.battery.as_ref(), Some(at), at, SAMPLE_LIFETIME);
            state.observe_sample(Some(result.connected), result.battery.as_ref(), result.battery_at,
                at + Duration::from_secs(1), SAMPLE_LIFETIME);
            if result.battery_unavailable { state.clear_battery(); }
            assert!(state.battery.is_none() && state.measured_at.is_none());
            assert!(!state.connected);
        }
    }

    #[test]
    fn unknown_identity_or_changed_prefix_cannot_authorize_a_sample() {
        let unknown = synthetic(IDENTITY, b"\x58\x52\x00C4_OTHER!");
        let (result, commands) = replay(&interface("unknown-before"), vec![Some(unknown.clone())]);
        assert!(result.is_err());
        assert_eq!(commands, [IDENTITY]);
        for after in [unknown, synthetic(IDENTITY, b"\x59\x52\x00C4_SERVER")] {
            assert!(replay(&interface("changed-after"), vec![Some(recorded(CAPTURED_SERVER)),
                Some(recorded(CAPTURED_POWER)), Some(after)]).0.is_err());
        }
    }

    #[test]
    fn timeouts_bad_frames_and_request_echoes_never_renew_a_sample() {
        let interface = interface("failed-sample");
        let first = online(&interface);
        let measured = first.battery_at.unwrap();
        let mut state = status::Status::default();
        state.observe_sample(Some(true), first.battery.as_ref(), Some(measured), measured, SAMPLE_LIFETIME);
        for fail_at in 0..3 {
            for bad in [None, Some(vec![0; 64]), Some(request(if fail_at == 1 { POWER } else { IDENTITY }).unwrap().to_vec())] {
                let mut replies = vec![Some(recorded(CAPTURED_SERVER)), Some(recorded(CAPTURED_POWER)),
                    Some(recorded(CAPTURED_SERVER))];
                replies[fail_at] = bad;
                if fail_at == 0 { replies.truncate(1); }
                assert!(replay(&interface, replies).0.is_err());
                state.observe_sample(None, None, None, measured + Duration::from_secs(3), SAMPLE_LIFETIME);
                assert_eq!(state.measured_at, Some(measured));
                assert_eq!(state.battery.as_ref().unwrap().percent, Some(18));
                assert_eq!(state.quality, ReadingQuality::Cached);
            }
        }
        assert!(state.expire(measured + SAMPLE_LIFETIME, SAMPLE_LIFETIME));
        assert_eq!(state.quality, ReadingQuality::Stale);
    }

    #[test]
    fn queued_old_frames_are_drained_and_wrong_registers_and_echoes_are_skipped() {
        let packet = request(POWER).unwrap();
        let mut transport = Scripted {
            queued: vec![windows(&synthetic(POWER, &[99, 1])); 12].into(),
            replies: vec![recorded(CAPTURED_SERVER), packet.to_vec(), synthetic(POWER, &[255, 1]),
                recorded(CAPTURED_POWER)].into(), ..Default::default()
        };
        let frame = exchange_with(&mut transport, &packet, REPLY_TIMEOUT_MS, Instant::now).unwrap();
        assert_eq!(battery(&frame).unwrap().percent, Some(18));
        assert_eq!(transport.drains, 13);
        assert_eq!(transport.received, 4);
        assert_eq!(transport.writes, [packet.to_vec()]);
        assert!(transport.queued.is_empty());
    }

    #[test]
    fn saturated_queue_blocks_writes_and_invalid_reply_flood_is_bounded() {
        let packet = request(POWER).unwrap();
        let mut queued = Scripted { queued: vec![recorded(CAPTURED_POWER); MAX_FRAMES + 1].into(),
            ..Default::default() };
        assert!(exchange_with(&mut queued, &packet, REPLY_TIMEOUT_MS, Instant::now).is_none());
        assert!(queued.writes.is_empty());
        assert_eq!(queued.drains, MAX_FRAMES);
        let mut flood = Scripted { replies: vec![packet.to_vec(); MAX_FRAMES + 1].into(), ..Default::default() };
        let rejected = exchange_with(&mut flood, &packet, REPLY_TIMEOUT_MS, Instant::now).unwrap();
        assert!(!matches_reply(&packet, &rejected));
        assert_eq!(flood.received, MAX_FRAMES);
        assert_eq!(flood.writes.len(), 1);
        let mut invalid = Scripted { replies: vec![vec![0; 64]].into(), ..Default::default() };
        assert!(battery(&exchange_with(&mut invalid, &packet, REPLY_TIMEOUT_MS, Instant::now).unwrap()).is_none());
    }

    #[test]
    fn expired_exchange_preserves_invalid_diagnostics_but_never_returns_late_valid_power() {
        struct Timed<'clock> { inner: Scripted, clock: &'clock Cell<Instant>, finished: Instant }
        impl Transport for Timed<'_> {
            fn pending(&mut self) -> Option<Vec<u8>> { self.inner.pending() }
            fn exchange(&mut self, packet: &[u8], timeout: u32,
                accept: &mut dyn FnMut(&[u8]) -> bool) -> Option<Vec<u8>> {
                let response = self.inner.exchange(packet, timeout, accept);
                self.clock.set(self.finished);
                response
            }
        }
        let packet = request(POWER).unwrap();
        let started = Instant::now();
        for (replies, invalid) in [(vec![vec![0; 64]], true),
            (vec![vec![0; 64], recorded(CAPTURED_POWER)], true),
            (vec![recorded(CAPTURED_POWER)], false), (vec![], false)] {
            let clock = Cell::new(started);
            let mut transport = Timed {
                inner: Scripted { replies: replies.into(), ..Default::default() }, clock: &clock,
                finished: started + Duration::from_millis(u64::from(REPLY_TIMEOUT_MS)),
            };
            let response = exchange_with(&mut transport, &packet, REPLY_TIMEOUT_MS, || clock.get());
            assert_eq!(response.is_some(), invalid);
            assert!(response.as_deref().and_then(battery).is_none());
            assert!(matches!(outcome(POWER, response.as_deref(), false), trace::Outcome::InvalidData) == invalid);
            assert_eq!(transport.inner.writes.len(), 1);
        }
    }

    #[test]
    fn confirmed_dongle_traces_offline_and_invalid_or_late_identity_does_not() {
        let dongle = dongle();
        assert!(matches!(outcome(IDENTITY, Some(&dongle), true), trace::Outcome::Offline));
        assert!(matches!(outcome(IDENTITY, Some(&dongle), false), trace::Outcome::InvalidData));
        assert!(matches!(outcome(IDENTITY, Some(&recorded(CAPTURED_SERVER)), true), trace::Outcome::Ok));
        assert!(matches!(outcome(POWER, Some(&recorded(CAPTURED_POWER)), true), trace::Outcome::Ok));
        assert!(matches!(outcome(IDENTITY, None, false), trace::Outcome::NoReply));
    }

    #[test]
    fn invalid_outbound_frames_and_expired_drain_budget_never_write() {
        for packet in [vec![], request(POWER).unwrap()[1..].to_vec(), vec![0; 65],
            windows(&synthetic(0x0b, &[]))] {
            let mut transport = Scripted::default();
            assert!(exchange_with(&mut transport, &packet, REPLY_TIMEOUT_MS, Instant::now).is_none());
            assert_eq!(transport.drains, 0);
            assert!(transport.writes.is_empty());
        }
        let started = Instant::now();
        let clock = Cell::new(started);
        let mut transport = Scripted::default();
        assert!(exchange_with(&mut transport, &request(POWER).unwrap(), 1, || {
            let at = clock.get();
            clock.set(at + Duration::from_millis(1));
            at
        }).is_none());
        assert_eq!(transport.drains, 0);
        assert!(transport.writes.is_empty());
    }

    #[test]
    fn one_round_budget_covers_all_queries_and_keeps_battery_measurement_time() {
        let started = Instant::now();
        let clock = Cell::new(started);
        let mut timeouts = Vec::new();
        let result = query_with(&interface("round-budget"), started + ROUND_BUDGET, || clock.get(), |packet, timeout| {
            timeouts.push(timeout);
            clock.set(clock.get() + Duration::from_millis(if timeouts.len() < 3 { 1100 } else { 500 }));
            Some(recorded(if packet[3] == IDENTITY { CAPTURED_SERVER } else { CAPTURED_POWER }))
        }).unwrap();
        assert_eq!(timeouts, [1200, 1200, 1000]);
        assert_eq!(result.battery_at, Some(started + Duration::from_millis(2200)));
        assert_eq!(clock.get(), started + Duration::from_millis(2700));
        clock.set(started);
        let mut calls = 0;
        assert!(query_with(&interface("late-round"), started + ROUND_BUDGET, || clock.get(), |packet, timeout| {
            calls += 1;
            clock.set(clock.get() + Duration::from_millis(if calls < 3 { 1100 } else { u64::from(timeout) }));
            Some(recorded(if packet[3] == IDENTITY { CAPTURED_SERVER } else { CAPTURED_POWER }))
        }).is_err());
        assert_eq!(calls, 3);
        assert!(query_with(&interface("expired-round"), started, || started, |_, _| panic!("expired")).is_err());
    }

    #[test]
    fn descriptor_gate_requires_exact_unnumbered_reports_and_discovery_is_neutral() {
        let interface = interface("gate");
        for ids in [None, Some((1, 0)), Some((0, 1)), Some((1, 1)), Some((0xa5, 0xa5))] {
            assert!(!descriptor_gate(&interface, |input, output| {
                assert_eq!((input, output), (65, 65)); ids
            }));
        }
        assert!(descriptor_gate(&interface, |_, _| Some((0, 0))));
        assert!(discover_with(std::slice::from_ref(&interface), |_| false).is_empty());
        let discovered = discover_with(std::slice::from_ref(&interface), |_| true);
        assert_eq!(discovered.len(), 1);
        assert!(!discovered[0].verified_identity && !discovered[0].name.contains("GALE2"));
        assert_eq!(discovered[0].family, DeviceFamily::Gamepad);
    }

    #[test]
    fn nearby_pids_xinput_and_malformed_interfaces_never_reach_descriptor_proof() {
        for change in [
            |interface: &mut HidInterface| interface.vendor_id = 0x3537,
            |interface: &mut HidInterface| interface.product_id = 0x122d,
            |interface: &mut HidInterface| interface.product_id = 0x10bd,
            |interface: &mut HidInterface| interface.usage_page = 1,
            |interface: &mut HidInterface| interface.usage = 0xffab,
            |interface: &mut HidInterface| interface.input_len = 64,
            |interface: &mut HidInterface| interface.output_len = 64,
            |interface: &mut HidInterface| interface.feature_len = 65,
            |interface: &mut HidInterface| interface.path = interface.path.replace("mi_02", "mi_01"),
            |interface: &mut HidInterface| interface.path = interface.path.replace("mi_02", "ig_01"),
            |interface: &mut HidInterface| interface.path = interface.path.replace("mi_02", "mi_020"),
            |interface: &mut HidInterface| interface.path = interface.path.replace("mi_02", "mi_02&col01"),
            |interface: &mut HidInterface| interface.path = interface.path.replace("pid_122c", "pid_122d"),
            |interface: &mut HidInterface| interface.path.push('\0'),
            |interface: &mut HidInterface| interface.path = "unknown".into(),
            |interface: &mut HidInterface| interface.path = interface.path.replace("8&scope&0&0000", ""),
        ] {
            let mut other = interface("scope");
            change(&mut other);
            assert!(!descriptor_gate(&other, |_, _| panic!("out of scope")));
            assert!(discover_with(&[other], |_| panic!("out of scope")).is_empty());
        }
        let first = interface("ambiguous");
        let mut sibling = first.clone();
        sibling.path = sibling.path.replace("mi_02", "mi_02&col02").replace("&0000#", "&0001#");
        assert!(discover_with(&[first.clone(), sibling], |_| panic!("ambiguous")).is_empty());
        assert!(discover_with(&[first.clone(), first], |_| panic!("duplicate")).is_empty());
    }

    #[test]
    fn equal_serial_receivers_remain_distinct_and_changed_paths_cannot_be_reused() {
        let first = interface("removed");
        let second = interface("remaining");
        let mut blank = second.clone();
        blank.serial.clear();
        let discovered = discover_with(&[first.clone(), second.clone()], |_| true);
        assert_eq!(discovered.len(), 2);
        assert_ne!(discovered[0].key, discovered[1].key);
        assert_eq!(target(&blank).key, target(&second).key);
        let now = Instant::now();
        assert!(read_with(&target(&first), std::slice::from_ref(&second), now + ROUND_BUDGET,
            || now, |_| true, |_, _| panic!("wrong receiver")).is_err());
        assert!(read_with(&target(&second), &[blank], now + ROUND_BUDGET,
            || now, |_| true, |_, _| panic!("changed serial")).is_err());
        let mut upper = second.clone();
        upper.path.make_ascii_uppercase();
        let result = read_with(&target(&second), &[upper], now + ROUND_BUDGET,
            || now, |_| true, |interface, _| Ok(online(interface))).unwrap();
        assert_eq!(result.target.key, target(&second).key);
    }

    #[test]
    fn invalid_targets_and_descriptor_changes_prevent_io_or_publication() {
        let interface = interface("target-proof");
        let now = Instant::now();
        for change in [
            |target: &mut Target| target.backend = BackendId::XInput,
            |target: &mut Target| target.address = 1,
            |target: &mut Target| target.key = "other".into(),
            |target: &mut Target| target.endpoint = Endpoint::XInput(0),
        ] {
            let mut original = target(&interface);
            change(&mut original);
            assert!(read_with(&original, std::slice::from_ref(&interface), now + ROUND_BUDGET,
                || now, |_| panic!("invalid target"), |_, _| panic!("invalid target")).is_err());
        }
        assert!(read_with(&target(&interface), std::slice::from_ref(&interface), now + ROUND_BUDGET,
            || now, |_| false, |_, _| panic!("unproven")).is_err());
        let calls = Cell::new(0);
        assert!(read_with(&target(&interface), std::slice::from_ref(&interface), now + ROUND_BUDGET,
            || now, |_| { calls.set(calls.get() + 1); calls.get() == 1 },
            |interface, _| Ok(online(interface))).is_err());
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn enumeration_proof_and_publication_consume_the_same_round_budget() {
        let interface = interface("metadata-budget");
        let now = Instant::now();
        let deadline = now + ROUND_BUDGET;
        assert!(read_with(&target(&interface), std::slice::from_ref(&interface), deadline,
            || deadline, |_| panic!("enumeration exhausted budget"), |_, _| panic!("expired")).is_err());
        let clock = Cell::new(now);
        assert!(read_with(&target(&interface), std::slice::from_ref(&interface), deadline,
            || clock.get(), |_| { clock.set(deadline); true }, |_, _| panic!("proof exhausted budget")).is_err());
        clock.set(now);
        assert!(read_with(&target(&interface), std::slice::from_ref(&interface), deadline,
            || clock.get(), |_| true, |interface, _| { clock.set(deadline); Ok(online(interface)) }).is_err());
    }

    #[test]
    fn channel_lock_spans_the_round_and_does_not_block_another_receiver() {
        let first = interface("locked");
        let second = interface("unlocked");
        let interfaces = [first.clone(), second.clone()];
        let now = Instant::now();
        let lock = channel::lock(&receiver_group(&first).unwrap());
        let held = lock.lock().unwrap();
        assert!(read_with(&target(&first), &interfaces, now + ROUND_BUDGET, || now,
            |_| panic!("locked"), |_, _| panic!("locked")).is_err());
        let result = read_with(&target(&second), &interfaces, now + ROUND_BUDGET, || now, |_| true,
            |interface, _| Ok(online(interface))).unwrap();
        assert_eq!(result.target.key, target(&second).key);
        drop(held);
        let result = read_with(&target(&first), &interfaces, now + ROUND_BUDGET, || now,
            |_| { assert!(lock.try_lock().is_err()); true }, |interface, _| {
                assert!(lock.try_lock().is_err()); Ok(online(interface))
            }).unwrap();
        assert_eq!(result.target.key, target(&first).key);
        assert!(lock.try_lock().is_ok());
    }
}
