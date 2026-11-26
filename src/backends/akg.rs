//! AKG N9 Hybrid, based on VigilWatt c97adc389b60b0e945a62cddacbaaace39887723.
//! Protocol/fixture provenance and MIT attribution: THIRD_PARTY_NOTICES.md.
use super::*;
use crate::win32::*;
use std::time::{Duration, Instant};

pub struct Akg;

const LENGTH: u16 = 62;
const INPUT_ID: u8 = 7;
const OUTPUT_ID: u8 = 6;
const MAX_FRAGMENT: usize = 59;
const MAX_MESSAGE: usize = 1024;
const IO_TIMEOUT_MS: u32 = 500;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(interface: &HidInterface) -> bool {
    interface.vendor_id == 0x0ecb && interface.product_id == 0x20d9
        && interface.usage_page == 0xff13 && interface.usage == 1
}

fn receiver_group(interface: &HidInterface) -> Option<String> {
    if !management(interface) { return None; }
    let path = interface.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid" || parts[3].is_empty()
        || path.contains('\0') || parts[1..].iter().any(|part| part.contains(['\\', '/'])) {
        return None;
    }
    let fields: Vec<_> = parts[1].split('&').collect();
    if fields.len() != 4 || fields[..3] != ["vid_0ecb", "pid_20d9", "mi_03"] {
        return None;
    }
    let collection = fields[3].strip_prefix("col")?;
    if collection.len() != 2 || !collection.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let (instance, index) = parts[2].rsplit_once('&')?;
    if instance.split('&').count() < 2 || instance.split('&').any(str::is_empty)
        || index.len() != 4 || !index.starts_with("00")
        || !index.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("{}#vid_0ecb&pid_20d9&mi_03#{instance}#{}", parts[0], parts[3]))
}

fn control(interface: &HidInterface) -> bool {
    receiver_group(interface).is_some()
        && interface.path.to_ascii_lowercase().contains("&mi_03&col02#")
        && interface.input_len == LENGTH && interface.output_len == LENGTH
        && interface.feature_len == 0
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|interface| control(interface)
        && interfaces.iter().filter(|other| receiver_group(other) == receiver_group(interface))
            .count() == 1).collect()
}

fn descriptor_gate(interface: &HidInterface, proof: impl FnOnce() -> Option<(u8, u8)>) -> bool {
    control(interface) && proof() == Some((INPUT_ID, OUTPUT_ID))
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    descriptor_gate(interface, || descriptor::Descriptor::open(api, interface)?
        .single_input_output_report_ids(interface, LENGTH, LENGTH))
}

fn target(interface: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::Akg, interface, 0);
    target.name = "AKG N9 Hybrid".into();
    target.family = DeviceFamily::Headset;
    target
}

fn valid_target(candidate: &Target) -> bool {
    candidate.backend == BackendId::Akg && candidate.address == 0
        && candidate.hid().is_ok_and(|interface| control(interface)
            && candidate.key == target(interface).key)
}

fn current_interface<'a>(original: &Target, interfaces: &'a [HidInterface]) -> Option<&'a HidInterface> {
    if !valid_target(original) { return None; }
    let path = &original.hid().ok()?.path;
    candidates(interfaces).into_iter().find(|interface| interface.path.eq_ignore_ascii_case(path))
}

fn request() -> [u8; LENGTH as usize] {
    let mut packet = [0; LENGTH as usize];
    packet[..10].copy_from_slice(&[OUTPUT_ID, 7, 0x80, 5, 0x5a, 3, 0, 0xd6, 0x0c, 0]);
    packet
}

fn payload(report: &[u8]) -> Result<Option<&[u8]>, ()> {
    if report.first().is_some_and(|id| *id != INPUT_ID) { return Ok(None); }
    if report.len() != usize::from(LENGTH) || report[2] != 0x80
        || usize::from(report[1]) > MAX_FRAGMENT { return Err(()); }
    Ok(Some(&report[3..3 + usize::from(report[1])]))
}

#[derive(Default)]
struct Assembler { pending: Vec<u8> }

impl Assembler {
    fn accept(&mut self, fragment: &[u8]) -> Result<Vec<Vec<u8>>, ()> {
        if fragment.len() > MAX_FRAGMENT || self.pending.len() + fragment.len() > MAX_MESSAGE + MAX_FRAGMENT {
            return Err(());
        }
        self.pending.extend_from_slice(fragment);
        let mut consumed = 0;
        let mut messages = Vec::new();
        while consumed < self.pending.len() {
            let remaining = &self.pending[consumed..];
            if remaining[0] != 5 { return Err(()); }
            if remaining.len() < 4 { break; }
            if !matches!(remaining[1], 0x5b..=0x5d) { return Err(()); }
            let length = 4 + usize::from(u16::from_le_bytes([remaining[2], remaining[3]]));
            if !(6..=MAX_MESSAGE).contains(&length) { return Err(()); }
            if remaining.len() < length { break; }
            messages.push(remaining[..length].to_vec());
            consumed += length;
        }
        self.pending.drain(..consumed);
        Ok(messages)
    }
}

fn battery_message(message: &[u8]) -> bool {
    message.len() >= 6 && message[0] == 5 && matches!(message[1], 0x5b..=0x5d)
        && message.len() == 4 + usize::from(u16::from_le_bytes([message[2], message[3]]))
        && message[4..6] == [0xd6, 0x0c]
}

fn percent(message: &[u8]) -> Option<u8> {
    (battery_message(message) && message.len() == 9 && message[1] == 0x5d
        && message[6..8] == [0, 0] && message[8] <= 100).then(|| message[8])
}

trait Transport {
    fn read(&mut self, timeout_ms: u32) -> Option<Vec<u8>>;
    fn write(&mut self, packet: &[u8], timeout_ms: u32) -> bool;
}

fn remaining_timeout(deadline: Duration, now: Duration) -> Option<u32> {
    let remaining = deadline.checked_sub(now)?.as_millis();
    (remaining > 0).then(|| remaining.min(u128::from(IO_TIMEOUT_MS)) as u32)
}

fn query_with(transport: &mut impl Transport, mut now: impl FnMut() -> Duration,
    mut delay: impl FnMut(Duration)) -> Option<u8> {
    let idle_deadline = now() + Duration::from_secs(1);
    let mut idle = false;
    for attempt in 0..20 {
        let report = transport.read(remaining_timeout(idle_deadline, now())?);
        remaining_timeout(idle_deadline, now())?;
        if let Some(report) = report {
            if payload(&report).ok()?.is_some_and(<[u8]>::is_empty) {
                idle = true;
                break;
            }
        }
        if attempt < 19 {
            delay(Duration::from_millis(u64::from(remaining_timeout(idle_deadline, now())?.min(50))));
        }
    }
    if !idle || !transport.write(&request(), IO_TIMEOUT_MS) { return None; }
    let response_deadline = now() + Duration::from_secs(3);
    let mut assembler = Assembler::default();
    let mut previous: Option<Vec<u8>> = None;
    let mut acknowledged = false;
    for _ in 0..60 {
        delay(Duration::from_millis(u64::from(remaining_timeout(response_deadline, now())?.min(50))));
        let report = transport.read(remaining_timeout(response_deadline, now())?);
        remaining_timeout(response_deadline, now())?;
        let Some(report) = report else { continue; };
        let Some(fragment) = payload(&report).ok()? else { continue; };
        if fragment.is_empty() {
            previous = None;
            continue;
        }
        if previous.as_deref() == Some(fragment) { continue; }
        previous = Some(fragment.to_vec());
        for message in assembler.accept(fragment).ok()? {
            if !battery_message(&message) { continue; }
            if message[1] == 0x5b {
                if message.len() != 7 || message[6] != 0 { return None; }
                acknowledged = true;
            } else if message[1] == 0x5d && acknowledged {
                return percent(&message);
            }
        }
    }
    None
}

struct ControlChannel {
    interface: HidInterface,
    handle: Handle,
    event: Handle,
    request_bytes: usize,
    reply_bytes: usize,
}

impl ControlChannel {
    fn open(interface: &HidInterface) -> Result<Self, String> {
        if !control(interface) { return Err(UNAVAILABLE.into()); }
        let handle = unsafe { CreateFileW(to_wide(&interface.path).as_ptr(), GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE, core::ptr::null_mut(), OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED, core::ptr::null_mut()) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE { return Err(UNAVAILABLE.into()); }
        let event = unsafe { CreateEventW(core::ptr::null_mut(), 0, 0, core::ptr::null()) };
        if event.is_null() || event == INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(handle); }
            return Err(UNAVAILABLE.into());
        }
        Ok(Self { interface: interface.clone(), handle, event, request_bytes: 0, reply_bytes: 0 })
    }

    fn finish(&mut self, ok: i32, mut transferred: u32, overlapped: &mut Overlapped,
        timeout_ms: u32) -> Option<usize> {
        if ok == 0 {
            if last_error() != ERROR_IO_PENDING { return None; }
            if unsafe { WaitForSingleObject(self.event, timeout_ms) } != WAIT_OBJECT_0 {
                unsafe {
                    CancelIoEx(self.handle, overlapped);
                    GetOverlappedResult(self.handle, overlapped, &mut transferred, 1);
                }
                return None;
            }
            if unsafe { GetOverlappedResult(self.handle, overlapped, &mut transferred, 0) } == 0 {
                return None;
            }
        }
        Some(transferred as usize)
    }
}

impl Transport for ControlChannel {
    fn read(&mut self, timeout_ms: u32) -> Option<Vec<u8>> {
        let report = crate::hid::query_report_with_handles(self.handle, self.event,
            usize::from(LENGTH), INPUT_ID, false, timeout_ms);
        let bytes = report.as_deref().unwrap_or_default();
        self.reply_bytes += bytes.len();
        crate::capture::record(&self.interface.path, "get-input", bytes, report.is_some());
        report
    }

    fn write(&mut self, packet: &[u8], timeout_ms: u32) -> bool {
        if packet != request() { return false; }
        self.request_bytes += packet.len();
        let mut overlapped = Overlapped { h_event: self.event, ..Default::default() };
        let mut transferred = 0;
        let ok = unsafe { WriteFile(self.handle, packet.as_ptr().cast(), packet.len() as u32,
            &mut transferred, &mut overlapped) };
        let success = self.finish(ok, transferred, &mut overlapped, timeout_ms) == Some(packet.len());
        crate::capture::record(&self.interface.path, "write", packet, success);
        success
    }
}

impl Drop for ControlChannel {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.event); CloseHandle(self.handle); }
    }
}

fn read_with(original: &Target, query: impl FnOnce() -> Option<u8>) -> Result<Snapshot, String> {
    if !valid_target(original) { return Err(UNAVAILABLE.into()); }
    let percent = query().filter(|value| *value <= 100).ok_or(UNAVAILABLE)?;
    let mut target = target(original.hid()?);
    target.verified_identity = true;
    Ok(Snapshot { target, battery: Some(Battery { percent: Some(percent), charging: None,
        estimated_percent: None, voltage_mv: None, approximate: false, level: None }),
        battery_at: Some(Instant::now()), connected: true, battery_unavailable: false,
        dpi: Dpi::default(), polling_rate: Default::default() })
}

impl DeviceBackend for Akg {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|interface| proven(api, interface)).map(target).collect())
    }

    fn read(&self, api: &HidApi, original: &Target, _: bool) -> Result<Snapshot, String> {
        if !valid_target(original) { return Err(UNAVAILABLE.into()); }
        let lock = channel::lock(&receiver_group(original.hid()?).ok_or(UNAVAILABLE)?);
        let _guard = match lock.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(UNAVAILABLE.into()),
        };
        let interfaces = enumeration::scan(api)?.interfaces;
        let interface = current_interface(original, &interfaces)
            .filter(|interface| proven(api, interface)).ok_or(UNAVAILABLE)?;
        let mut transport = ControlChannel::open(interface)?;
        read_with(&target(interface), || {
            let started = Instant::now();
            let result = query_with(&mut transport, || started.elapsed(), std::thread::sleep);
            trace::record(interface, BackendId::Akg, "query-battery", 0x0cd6, OUTPUT_ID,
                transport.request_bytes, transport.reply_bytes, started,
                if result.is_some() { trace::Outcome::Ok } else { trace::Outcome::NoReply });
            result
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, collections::VecDeque, sync::Arc};

    const ACK: [u8; 7] = [5, 0x5b, 3, 0, 0xd6, 0x0c, 0];

    fn interface(instance: &str) -> HidInterface {
        HidInterface { path: format!(r"\\?\hid#vid_0ecb&pid_20d9&mi_03&col02#8&{instance}&0&0001#{{hid}}"),
            vendor_id: 0x0ecb, product_id: 0x20d9, usage_page: 0xff13, usage: 1,
            input_len: LENGTH, output_len: LENGTH, feature_len: 0,
            product: "AKG N9 Hybrid".into(), serial: "shared-serial".into() }
    }

    fn result(value: u8) -> Vec<u8> { vec![5, 0x5d, 5, 0, 0xd6, 0x0c, 0, 0, value] }

    fn report(fragment: &[u8]) -> Vec<u8> {
        assert!(fragment.len() <= MAX_FRAGMENT);
        let mut report = vec![0; usize::from(LENGTH)];
        report[..3].copy_from_slice(&[INPUT_ID, fragment.len() as u8, 0x80]);
        report[3..3 + fragment.len()].copy_from_slice(fragment);
        report
    }

    fn response(value: u8) -> Vec<u8> { report(&[ACK.as_slice(), &result(value)].concat()) }

    #[derive(Default)]
    struct Script {
        frames: VecDeque<Option<Vec<u8>>>,
        fallback: Option<Vec<u8>>,
        reads: usize,
        writes: usize,
        fail_write: bool,
    }

    impl Script {
        fn new(frames: Vec<Vec<u8>>) -> Self {
            Self { frames: frames.into_iter().map(Some).collect(), fallback: Some(report(&[])), ..Default::default() }
        }

        fn query(&mut self) -> Option<u8> { query_with(self, || Duration::ZERO, |_| {}) }
    }

    impl Transport for Script {
        fn read(&mut self, timeout_ms: u32) -> Option<Vec<u8>> {
            assert!((1..=IO_TIMEOUT_MS).contains(&timeout_ms));
            self.reads += 1;
            self.frames.pop_front().unwrap_or_else(|| self.fallback.clone())
        }

        fn write(&mut self, packet: &[u8], timeout_ms: u32) -> bool {
            assert_eq!(packet, request());
            assert_eq!(timeout_ms, IO_TIMEOUT_MS);
            self.writes += 1;
            !self.fail_write
        }
    }

    #[test]
    fn upstream_recorded_full_report_preserves_fifty_percent_and_ignores_old_tail() {
        let hex = "071080055B0300D60C00055D0500D60C00003201090500000A0700055D08000109050000010400055D080001090500000A0700055D080001090500000104";
        let recorded: Vec<u8> = (0..hex.len()).step_by(2)
            .map(|offset| u8::from_str_radix(&hex[offset..offset + 2], 16).unwrap()).collect();
        assert_eq!(recorded.len(), 62);
        let mut script = Script::new(vec![report(&[]), recorded]);
        let sample = read_with(&target(&interface("recorded")), || script.query()).unwrap();
        let battery = sample.battery.unwrap();
        assert_eq!((battery.percent, battery.charging), (Some(50), None));
        assert!(sample.connected && !sample.battery_unavailable && sample.battery_at.is_some());
        assert!(sample.target.verified_identity);
        assert_eq!(sample.target.family, DeviceFamily::Headset);
        assert!(!sample.dpi.writable && !sample.polling_rate.writable);
        assert_eq!((script.reads, script.writes), (2, 1));
    }

    #[test]
    fn numbered_query_is_exactly_sixty_two_bytes_including_its_id() {
        let packet = request();
        assert_eq!(packet.len(), 62);
        assert_eq!(&packet[..10], &[6, 7, 0x80, 5, 0x5a, 3, 0, 0xd6, 0x0c, 0]);
        assert!(packet[10..].iter().all(|byte| *byte == 0));
        assert!(payload(&packet).unwrap().is_none());
        let valid = response(50);
        for length in 0..62 { assert!(payload(&valid[..length]).is_err()); }
        let mut longer = valid.clone(); longer.push(0);
        assert!(payload(&longer).is_err());
        let mut zero_prefixed = vec![0]; zero_prefixed.extend_from_slice(&valid);
        assert!(payload(&zero_prefixed).unwrap().is_none());
    }

    #[test]
    fn synthetic_zero_through_one_hundred_are_online_with_unknown_charging() {
        for value in 0..=100 {
            let mut script = Script::new(vec![report(&[]), response(value)]);
            let sample = read_with(&target(&interface("boundary")), || script.query()).unwrap();
            let battery = sample.battery.unwrap();
            assert_eq!((battery.percent, battery.charging), (Some(value), None));
            assert!(sample.connected && !sample.battery_unavailable && sample.battery_at.is_some());
        }
        for value in 101..=255 { assert_eq!(percent(&result(value)), None); }
        for value in [None, Some(101), Some(255)] {
            assert!(read_with(&target(&interface("invalid")), || value).is_err());
        }
    }

    #[test]
    fn synthetic_message_fields_reject_wrong_command_status_role_and_lengths() {
        for offset in 0..8 {
            let mut wrong = result(50); wrong[offset] ^= 1;
            assert_eq!(percent(&wrong), None, "offset {offset}");
        }
        for length in 0..9 { assert_eq!(percent(&result(50)[..length]), None); }
        let mut longer = result(50); longer.push(0);
        assert_eq!(percent(&longer), None);
        assert_eq!(percent(&ACK), None);
        for offset in [6, 7, 8] {
            let mut wrong = result(50); wrong[offset] = 255;
            let mut script = Script::new(vec![report(&[]), report(&ACK), report(&wrong), response(50)]);
            assert_eq!(script.query(), None);
            assert_eq!((script.reads, script.writes), (3, 1));
        }
    }

    #[test]
    fn synthetic_fragments_split_at_every_boundary_and_ignore_empty_stale_bytes() {
        let stream = [ACK.as_slice(), &result(50)].concat();
        for split in 1..stream.len() {
            let mut empty = response(90); empty[1] = 0;
            let mut script = Script::new(vec![report(&[]), report(&stream[..split]), empty, report(&stream[split..])]);
            assert_eq!(script.query(), Some(50), "split {split}");
            assert_eq!(script.writes, 1);
        }
    }

    #[test]
    fn synthetic_duplicate_snapshots_compare_only_valid_bytes() {
        for change_padding in [false, true] {
            let first = report(&[ACK.as_slice(), &result(50)[..5]].concat());
            let mut repeated = first.clone();
            if change_padding { repeated[61] ^= 0xa5; }
            let mut script = Script::new(vec![report(&[]), first, repeated, report(&result(50)[5..])]);
            assert_eq!(script.query(), Some(50));
            assert_eq!((script.reads, script.writes), (4, 1));
        }
    }

    #[test]
    fn synthetic_old_results_need_this_queries_ack_in_stream_order() {
        for same_report in [false, true] {
            let mut frames = vec![report(&result(80)), report(&[])];
            if same_report {
                frames.push(report(&[result(90).as_slice(), &ACK, &result(50)].concat()));
            } else {
                frames.extend([report(&result(90)), response(50)]);
            }
            let mut script = Script::new(frames);
            assert_eq!(script.query(), Some(50));
            assert_eq!(script.writes, 1);
        }
        let mut script = Script::new(vec![report(&[]), report(&ACK)]);
        assert_eq!(script.query(), None);
        script.frames.extend([report(&[]), report(&result(90))].into_iter().map(Some));
        assert_eq!(script.query(), None);
        for _ in 0..2 {
            script.frames.extend([report(&[]), response(50)].into_iter().map(Some));
            assert_eq!(script.query(), Some(50));
        }
        assert_eq!(script.writes, 4);
    }

    #[test]
    fn synthetic_rejected_or_malformed_ack_stops_without_retry() {
        let mut rejected = ACK.to_vec(); rejected[6] = 1;
        let malformed = vec![5, 0x5b, 4, 0, 0xd6, 0x0c, 0, 0];
        for ack in [rejected, malformed] {
            let mut script = Script::new(vec![report(&[]), report(&ack), response(50)]);
            assert_eq!(script.query(), None);
            assert_eq!((script.reads, script.writes), (2, 1));
        }
    }

    #[test]
    fn synthetic_foreign_frames_failures_and_nonempty_idle_are_bounded() {
        let mut foreign = report(&[]); foreign[0] = 6;
        for fallback in [None, Some(foreign), Some(report(&result(90)))] {
            let mut script = Script { fallback: fallback.clone(), ..Default::default() };
            assert_eq!(script.query(), None);
            assert_eq!((script.reads, script.writes), (20, 0));
            let mut script = Script { frames: [Some(report(&[]))].into(), fallback, ..Default::default() };
            assert_eq!(script.query(), None);
            assert_eq!((script.reads, script.writes), (61, 1));
        }
        let mut script = Script::new(vec![report(&[])]);
        assert_eq!(script.query(), None);
        assert_eq!((script.reads, script.writes), (61, 1));
    }

    #[test]
    fn synthetic_transient_failures_recover_and_write_failure_does_not_retry() {
        let mut foreign = report(&[]); foreign[0] = 0x66;
        let mut script = Script { frames: [None, Some(foreign.clone()), Some(report(&[])),
            None, Some(foreign), Some(response(50))].into(), ..Default::default() };
        assert_eq!(script.query(), Some(50));
        assert_eq!((script.reads, script.writes), (6, 1));
        let mut script = Script { fail_write: true, ..Script::new(vec![report(&[]), response(50)]) };
        assert_eq!(script.query(), None);
        assert_eq!((script.reads, script.writes), (1, 1));
    }

    #[test]
    fn synthetic_time_limits_stop_even_before_frame_count_limits() {
        for idle in [false, true] {
            let clock = Cell::new(Duration::ZERO);
            let mut script = Script::new(if idle { vec![report(&[])] } else { vec![report(&result(90))] });
            assert_eq!(query_with(&mut script, || clock.get(), |_| clock.set(clock.get() + Duration::from_secs(5))), None);
            assert_eq!((script.reads, script.writes), (1, usize::from(idle)));
        }
        assert_eq!(remaining_timeout(Duration::from_millis(10), Duration::from_millis(9)), Some(1));
        assert_eq!(remaining_timeout(Duration::from_secs(5), Duration::ZERO), Some(500));
        assert_eq!(remaining_timeout(Duration::from_secs(1), Duration::from_secs(1)), None);
    }

    #[test]
    fn synthetic_bad_envelopes_fail_closed_in_idle_and_response_phases() {
        let mut malformed = Vec::new();
        for (offset, value) in [(1, 60), (1, 255), (2, 0), (2, 0x81)] {
            let mut wrong = response(50); wrong[offset] = value; malformed.push(wrong);
        }
        malformed.extend([Vec::new(), response(50)[..61].to_vec(), [response(50), vec![0]].concat()]);
        for wrong in malformed {
            let mut script = Script::new(vec![wrong.clone(), report(&[]), response(50)]);
            assert_eq!(script.query(), None);
            assert_eq!((script.reads, script.writes), (1, 0));
            let mut script = Script::new(vec![report(&[]), wrong, response(50)]);
            assert_eq!(script.query(), None);
            assert_eq!((script.reads, script.writes), (2, 1));
        }
    }

    #[test]
    fn synthetic_assembler_rejects_bad_headers_and_never_scans_for_commands() {
        for header in [[4, 0x5d, 5, 0], [5, 0x5a, 5, 0], [5, 0x5d, 1, 0],
            [5, 0x5d, 0xfd, 3], [5, 0x5d, 0xff, 0xff]] {
            let mut assembler = Assembler::default();
            assert!(assembler.accept(&header).is_err());
        }
        let mut bad_stream = vec![0xff]; bad_stream.extend_from_slice(&result(50));
        assert!(Assembler::default().accept(&bad_stream).is_err());
        assert!(Assembler::default().accept(&[0; 60]).is_err());
        let mut script = Script::new(vec![report(&[]), report(&[ACK.as_slice(), &result(50), &[0xff]].concat())]);
        assert_eq!(script.query(), None);
    }

    #[test]
    fn synthetic_maximum_foreign_message_can_precede_battery_without_unbounded_buffering() {
        let mut foreign = vec![0xa5; MAX_MESSAGE];
        foreign[..6].copy_from_slice(&[5, 0x5c, 0xfc, 3, 1, 9]);
        let stream = [foreign.as_slice(), &ACK, &result(50)].concat();
        let mut assembler = Assembler::default();
        let mut messages = Vec::new();
        for fragment in stream.chunks(MAX_FRAGMENT) {
            messages.extend(assembler.accept(fragment).unwrap());
            assert!(assembler.pending.len() < MAX_MESSAGE);
        }
        assert_eq!(messages, vec![foreign, ACK.to_vec(), result(50)]);
        assert!(assembler.pending.is_empty());
        let mut foreign_command = result(90); foreign_command[4] ^= 1;
        let mut notification = result(90); notification[1] = 0x5c;
        let mut script = Script::new(vec![report(&[]), report(&[ACK.as_slice(), &foreign_command, &notification, &result(50)].concat())]);
        assert_eq!(script.query(), Some(50));
    }

    #[test]
    fn synthetic_gate_requires_exact_identity_capacities_and_numbered_descriptor_ids() {
        let valid = interface("gate");
        assert!(descriptor_gate(&valid, || Some((7, 6))));
        for ids in [None, Some((0, 6)), Some((7, 0)), Some((6, 7)), Some((7, 7)), Some((8, 6))] {
            assert!(!descriptor_gate(&valid, || ids));
        }
        for mutate in [
            |interface: &mut HidInterface| interface.vendor_id ^= 1,
            |interface: &mut HidInterface| interface.product_id ^= 1,
            |interface: &mut HidInterface| interface.usage_page = 0x0b,
            |interface: &mut HidInterface| interface.usage = 2,
            |interface: &mut HidInterface| interface.input_len = 61,
            |interface: &mut HidInterface| interface.input_len = 63,
            |interface: &mut HidInterface| interface.output_len = 61,
            |interface: &mut HidInterface| interface.output_len = 63,
            |interface: &mut HidInterface| interface.feature_len = 1,
        ] {
            let mut wrong = valid.clone(); mutate(&mut wrong);
            assert!(!descriptor_gate(&wrong, || panic!("invalid metadata must not inspect descriptor")));
            assert!(read_with(&target(&wrong), || panic!("invalid metadata must not query")).is_err());
        }
    }

    #[test]
    fn synthetic_path_gate_rejects_spoofed_tokens_and_wrong_interfaces_or_collections() {
        let valid = interface("path");
        let mut uppercase = valid.clone(); uppercase.path = uppercase.path.to_ascii_uppercase();
        assert!(control(&uppercase));
        assert_eq!(target(&valid).key, target(&uppercase).key);
        for (from, to) in [("vid_0ecb", "vid_0eca"), ("pid_20d9", "pid_20d8"),
            ("mi_03", "mi_00"), ("mi_03", "mi_030"), ("col02", "col04"),
            ("col02", "col020"), ("col02", "col02&col04"), ("mi_03", "mi_03&mi_04"),
            ("pid_20d9", "pid_20d9&pid_20d8"), ("0001", "001"), ("0001", "000z"),
            ("8&path&0", ""), ("{hid}", ""), ("#8&", "#8\\&"), ("{hid}", "{hid}#extra"),
            ("{hid}", "{hid}\0"), (r"\\?\hid", "other")] {
            let mut wrong = valid.clone(); wrong.path = wrong.path.replace(from, to);
            assert!(!control(&wrong), "{from} -> {to}");
        }
    }

    #[test]
    fn synthetic_duplicate_collections_fail_closed_even_with_mismatched_capacities() {
        let first = interface("duplicates");
        assert!(candidates(&[first.clone(), first.clone()]).is_empty());
        for collection in ["col02", "col03", "col04"] {
            let mut duplicate = first.clone();
            duplicate.path = duplicate.path.replace("col02", collection).replace("0001", "0003");
            duplicate.input_len = 63;
            assert!(candidates(&[first.clone(), duplicate]).is_empty());
        }
        let mut consumer = first.clone(); consumer.usage_page = 0x0c;
        assert_eq!(candidates(&[first, consumer]).len(), 1);
    }

    #[test]
    fn synthetic_instances_have_distinct_identity_and_locks_despite_shared_serials() {
        let first = interface("first"); let second = interface("second");
        assert_eq!(candidates(&[first.clone(), second.clone()]).len(), 2);
        assert_ne!(target(&first).key, target(&second).key);
        let first_lock = channel::lock(&receiver_group(&first).unwrap());
        let second_lock = channel::lock(&receiver_group(&second).unwrap());
        let mut uppercase = first.clone(); uppercase.path = uppercase.path.to_ascii_uppercase();
        assert!(Arc::ptr_eq(&first_lock, &channel::lock(&receiver_group(&uppercase).unwrap())));
        assert!(!Arc::ptr_eq(&first_lock, &second_lock));
        let _guard = first_lock.lock().unwrap();
        assert!(first_lock.try_lock().is_err());
        assert!(second_lock.try_lock().is_ok());
    }

    #[test]
    fn synthetic_revalidation_rejects_replaced_ambiguous_or_forged_targets() {
        let first = interface("fresh"); let original = target(&first);
        assert!(current_interface(&original, &[first.clone()]).is_some());
        assert!(current_interface(&original, &[]).is_none());
        assert!(current_interface(&original, &[interface("replaced")]).is_none());
        assert!(current_interface(&original, &[first.clone(), first.clone()]).is_none());
        let mut changed = first.clone(); changed.output_len = 63;
        assert!(current_interface(&original, &[changed]).is_none());
        for mutate in [|target: &mut Target| target.backend = BackendId::Headset,
            |target: &mut Target| target.address = 1, |target: &mut Target| target.key = "forged".into()] {
            let mut wrong = original.clone(); mutate(&mut wrong);
            assert!(current_interface(&wrong, &[first.clone()]).is_none());
            assert!(read_with(&wrong, || panic!("invalid target must not query")).is_err());
        }
    }
}
