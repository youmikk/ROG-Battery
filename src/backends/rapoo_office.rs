//! MT760-family receiver 24AE:1870: one battery query, then GET_REPORT(Input).
//! Only the documented FF00:000E command collection is eligible.
use super::*;
use crate::win32::*;
use std::time::{Duration, Instant};
use winapi::shared::hidclass::{IOCTL_HID_GET_INPUT_REPORT, IOCTL_HID_SET_OUTPUT_REPORT};

pub struct RapooOffice;
const REPORT: u8 = 0xba;
const LENGTH: u16 = 32;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";
const MAX_READS: usize = 24;

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x24ae && i.product_id == 0x1870
        && i.usage_page == 0xff00 && i.usage == 0x000e
}

fn control(i: &HidInterface) -> bool {
    management(i) && i.input_len == LENGTH && i.output_len == LENGTH && i.feature_len == 0
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|i| control(i) && interfaces.iter().filter(|other|
        management(other) && channel::collection_group(i) == channel::collection_group(other)
    ).count() == 1).collect()
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    control(i) && descriptor::Descriptor::open(api, i)
        .is_some_and(|d| d.only_input_output_report(i, REPORT, LENGTH))
}

fn target(i: &HidInterface) -> Target {
    let mut t = Target::new(BackendId::RapooOffice, i, 0);
    // The receiver PID does not distinguish MT760, L and Mini marketing names.
    t.name = "Rapoo MT760 series".into();
    t.family = DeviceFamily::Mouse;
    t
}

fn request() -> [u8; LENGTH as usize] {
    let mut bytes = [0; LENGTH as usize];
    bytes[..3].copy_from_slice(&[REPORT, 0xa5, 0xaa]);
    bytes
}

fn battery(bytes: &[u8]) -> Option<Battery> {
    // The Windows capture is exactly 32 bytes and replaces the BA slot with
    // status. An assumed BA-prefixed alternate layout is deliberately rejected.
    if bytes.len() != usize::from(LENGTH) || bytes[0] != 1 || !matches!(bytes[1], 1 | 2) {
        return None;
    }
    let percent = match (bytes[1], bytes[2]) {
        (_, 1..=100) => Some(bytes[2]),
        (2, 0x7f) => None,
        _ => return None,
    };
    Some(Battery { percent, charging: Some(bytes[1] == 2), estimated_percent: None,
        voltage_mv: None, approximate: false, level: None })
}

fn busy(bytes: &[u8]) -> bool { bytes.len() == usize::from(LENGTH) && bytes[0] == 2 }

trait Transport {
    fn send(&mut self, packet: &[u8], timeout_ms: u32) -> Result<(), String>;
    fn receive(&mut self, report_id: u8, timeout_ms: u32) -> Result<Vec<u8>, String>;
    fn pause(&self, _ms: u64) {}
}

struct NativeTransport { handle: Handle, reply_bytes: Option<usize>, status: Option<u8> }

impl NativeTransport {
    fn open(i: &HidInterface) -> Result<Self, String> {
        let handle = unsafe { CreateFileW(to_wide(&i.path).as_ptr(), GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE, core::ptr::null_mut(), OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED, core::ptr::null_mut()) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(crate::tr!("暂时无法连接设备，请确认未被其他软件占用").into());
        }
        Ok(Self { handle, reply_bytes: None, status: None })
    }

    fn transfer(&mut self, input: bool, packet: &mut [u8], timeout_ms: u32) -> Result<usize, String> {
        let event = unsafe { CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null()) };
        if event.is_null() || event == INVALID_HANDLE_VALUE {
            return Err(crate::tr!("暂时无法建立设备通信，请重试").into());
        }
        let mut ov = Overlapped { h_event: event, ..Default::default() };
        let mut count = 0;
        let result = (|| {
            let ok = unsafe { DeviceIoControl(self.handle,
                if input { IOCTL_HID_GET_INPUT_REPORT } else { IOCTL_HID_SET_OUTPUT_REPORT },
                packet.as_mut_ptr().cast(), packet.len() as u32,
                if input { packet.as_mut_ptr().cast() } else { core::ptr::null_mut() },
                if input { packet.len() as u32 } else { 0 }, &mut count, &mut ov) };
            if ok == 0 {
                if last_error() != ERROR_IO_PENDING {
                    return Err(crate::tr!("与设备通信失败，请检查连接后重试").into());
                }
                if unsafe { WaitForSingleObject(event, timeout_ms) } != WAIT_OBJECT_0 {
                    // Cancellation has to finish while the buffers and event
                    // still exist, including a device unplugged during a query.
                    unsafe {
                        CancelIoEx(self.handle, &mut ov);
                        GetOverlappedResult(self.handle, &mut ov, &mut count, 1);
                    }
                    return Err(crate::tr!("等待设备响应超时，请轻动设备唤醒后再试").into());
                }
                if unsafe { GetOverlappedResult(self.handle, &mut ov, &mut count, 0) } == 0 {
                    return Err(crate::tr!("与设备通信失败，请检查连接后重试").into());
                }
            }
            Ok(count as usize)
        })();
        unsafe { CloseHandle(event); }
        result
    }
}

impl Drop for NativeTransport {
    fn drop(&mut self) { unsafe { CloseHandle(self.handle); } }
}

impl Transport for NativeTransport {
    fn send(&mut self, packet: &[u8], timeout_ms: u32) -> Result<(), String> {
        if packet != request() { return Err(UNAVAILABLE.into()); }
        // SET_OUTPUT_REPORT success may return zero transferred bytes.
        self.transfer(false, &mut packet.to_vec(), timeout_ms)?;
        Ok(())
    }

    fn receive(&mut self, report_id: u8, timeout_ms: u32) -> Result<Vec<u8>, String> {
        if report_id != REPORT { return Err(UNAVAILABLE.into()); }
        self.reply_bytes = None;
        self.status = None;
        let mut packet = vec![0; usize::from(LENGTH)];
        packet[0] = report_id;
        let received = self.transfer(true, &mut packet, timeout_ms)?;
        self.reply_bytes = Some(received);
        if received != usize::from(LENGTH) { return Err(UNAVAILABLE.into()); }
        self.status = packet.first().copied();
        Ok(packet)
    }

    fn pause(&self, ms: u64) { std::thread::sleep(Duration::from_millis(ms)); }
}

fn read_with(t: &Target, transport: &mut impl Transport) -> Result<Snapshot, String> {
    let i = t.hid()?;
    if t.backend != BackendId::RapooOffice || !control(i) { return Err(UNAVAILABLE.into()); }
    // Give a previous command time to settle even after reopening the handle.
    transport.pause(30);
    transport.send(&request(), 250)?;
    let deadline = Instant::now() + Duration::from_millis(700);
    for attempt in 0..MAX_READS {
        transport.pause(if attempt == 0 { 20 } else { 18 });
        let remaining = deadline.checked_duration_since(Instant::now()).ok_or(UNAVAILABLE)?;
        if remaining.as_millis() == 0 { return Err(UNAVAILABLE.into()); }
        let response = transport.receive(REPORT, remaining.as_millis().min(200) as u32)?;
        if busy(&response) { continue; }
        let battery = battery(&response).ok_or(UNAVAILABLE)?;
        let mut target = target(i);
        target.verified_identity = true;
        return Ok(Snapshot { target, battery: Some(battery), battery_at: Some(Instant::now()),
            battery_unavailable: false, connected: true, dpi: Dpi::default(),
            polling_rate: Default::default() });
    }
    // No repeated query writes: this family has no transaction or opcode echo.
    Err(UNAVAILABLE.into())
}

impl DeviceBackend for RapooOffice {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i)).map(target).collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        let i = t.hid()?;
        if t.backend != BackendId::RapooOffice || !proven(api, i) { return Err(UNAVAILABLE.into()); }
        let lock = channel::lock(&channel::collection_group(i));
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let started = Instant::now();
        let mut transport = match NativeTransport::open(i) {
            Ok(transport) => transport,
            Err(error) => {
                trace::record(i, BackendId::RapooOffice, "query-battery", 0xaa, REPORT,
                    0, 0, started, trace::Outcome::NoReply);
                return Err(error);
            }
        };
        let result = read_with(t, &mut transport);
        let outcome = if result.is_ok() { trace::Outcome::Ok }
            else if transport.status == Some(2) { trace::Outcome::Rejected(2) }
            else if transport.reply_bytes.is_some() { trace::Outcome::InvalidData }
            else { trace::Outcome::NoReply };
        trace::record(i, BackendId::RapooOffice, "query-battery", 0xaa, REPORT,
            usize::from(LENGTH), transport.reply_bytes.unwrap_or(0), started, outcome);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn interface(instance: &str) -> HidInterface {
        // Synthetic Windows paths exercise sibling-collection and instance grouping.
        HidInterface { path: format!(r"\\?\hid#vid_24ae&pid_1870&mi_01&col04#8&{instance}&0004#{{hid}}"),
            vendor_id: 0x24ae, product_id: 0x1870, usage_page: 0xff00, usage: 0x000e,
            input_len: 32, output_len: 32, feature_len: 0,
            product: "Rapoo Gaming Device".into(), serial: "shared".into() }
    }

    fn synthetic_frame(status: u8, state: u8, percent: u8) -> Vec<u8> {
        let mut r = vec![0; usize::from(LENGTH)];
        r[..3].copy_from_slice(&[status, state, percent]);
        r
    }

    struct Fake {
        requests: Vec<Vec<u8>>,
        reads: usize,
        responses: VecDeque<Result<Vec<u8>, String>>,
        fail_send: bool,
    }

    impl Fake {
        fn new(responses: Vec<Result<Vec<u8>, String>>) -> Self {
            Self { requests: vec![], reads: 0, responses: responses.into(), fail_send: false }
        }
    }

    impl Transport for Fake {
        fn send(&mut self, packet: &[u8], timeout: u32) -> Result<(), String> {
            assert_eq!(timeout, 250);
            self.requests.push(packet.to_vec());
            if self.fail_send { Err("output failed".into()) } else { Ok(()) }
        }
        fn receive(&mut self, id: u8, timeout: u32) -> Result<Vec<u8>, String> {
            assert_eq!(id, REPORT);
            assert!((1..=200).contains(&timeout));
            self.reads += 1;
            self.responses.pop_front().expect("unexpected input query")
        }
    }

    #[test]
    fn office_receiver_excludes_gaming_telemetry_keyboard_and_ambiguous_collections() {
        let i = interface("one");
        assert!(control(&i));
        for (vid, pid, page, usage, input, output, feature) in [
            (0x24ae, 0x1851, 0xff00, 0x0e, 32, 32, 0),
            (0x24ae, 0x1415, 0xff00, 2, 19, 19, 0),
            (0x24ae, 0x1870, 0xff00, 2, 7, 7, 0),
            (0x24ae, 0x1870, 0xff01, 0, 29, 29, 29),
            (0x24ae, 0x1870, 0xff00, 0x0e, 33, 32, 0),
            (0x24ae, 0x1870, 0xff00, 0x0e, 32, 33, 0),
            (0x24ae, 0x1870, 0xff00, 0x0e, 32, 32, 33),
            (0x046d, 0x1870, 0xff00, 0x0e, 32, 32, 0),
        ] {
            let mut wrong = i.clone();
            wrong.vendor_id = vid; wrong.product_id = pid; wrong.usage_page = page;
            wrong.usage = usage; wrong.input_len = input; wrong.output_len = output;
            wrong.feature_len = feature;
            assert!(!control(&wrong));
        }
        let mut sibling = i.clone();
        sibling.path = sibling.path.replace("col04", "col06").replace("&0004#", "&0006#");
        sibling.input_len = 33;
        assert!(candidates(&[i.clone(), sibling]).is_empty());
        let other = interface("two");
        assert_eq!(candidates(&[i.clone(), other.clone()]).len(), 2);
        assert_ne!(target(&i).key, target(&other).key);
    }

    #[test]
    fn public_windows_capture_decodes_status_instead_of_a_report_id() {
        // PyFlat/Device-Battery-Info #36: full public Windows GET_INPUT_REPORT
        // capture, passed BA to the driver and received 01 01 2F plus 29 zeros.
        let capture = [1, 1, 0x2f, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let result = battery(&capture).unwrap();
        assert_eq!((result.percent, result.charging), (Some(47), Some(false)));
        // FerrisPulse #75 publishes the 01 01 59 prefix; this padding is synthetic.
        assert_eq!(battery(&synthetic_frame(1, 1, 89)).unwrap().percent, Some(89));
        let mut prefixed = vec![REPORT]; prefixed.extend_from_slice(&capture[..31]);
        assert!(battery(&prefixed).is_none());
        for length in 0..32 { assert!(battery(&capture[..length]).is_none()); }
    }

    #[test]
    fn charging_unknown_never_becomes_full_and_invalid_fields_are_rejected() {
        for percent in 1..=100 {
            for state in [1, 2] {
                let b = battery(&synthetic_frame(1, state, percent)).unwrap();
                assert_eq!((b.percent, b.charging), (Some(percent), Some(state == 2)));
            }
        }
        let unknown = battery(&synthetic_frame(1, 2, 0x7f)).unwrap();
        assert_eq!((unknown.percent, unknown.charging), (None, Some(true)));
        for percent in [0, 101, 126, 127, 128, 255] {
            assert!(battery(&synthetic_frame(1, 1, percent)).is_none());
        }
        for state in [0, 3, 0xff] { assert!(battery(&synthetic_frame(1, state, 47)).is_none()); }
        for status in [0, 2, 3, 0xba] { assert!(battery(&synthetic_frame(status, 1, 47)).is_none()); }
        assert!(battery(&request()).is_none());
        assert!(!busy(&[2]));
    }

    #[test]
    fn busy_polling_only_fetches_input_and_never_resends_the_query() {
        let t = target(&interface("one"));
        let mut fake = Fake::new(vec![Ok(synthetic_frame(2, 0xff, 0xff)),
            Ok(synthetic_frame(2, 0, 0)), Ok(synthetic_frame(1, 2, 0x7f))]);
        let sample = read_with(&t, &mut fake).unwrap();
        assert_eq!((fake.requests.len(), fake.reads), (1, 3));
        assert_eq!(fake.requests[0], request());
        assert!(fake.requests[0][3..].iter().all(|b| *b == 0));
        assert_eq!(sample.battery.unwrap().percent, None);
        assert!(sample.connected && sample.battery_at.is_some() && !sample.battery_unavailable);
        assert!(sample.dpi.stages.is_empty() && !sample.dpi.writable && !sample.polling_rate.writable);
        let mut forever_busy = Fake::new(vec![Ok(synthetic_frame(2, 1, 47)); MAX_READS]);
        assert!(read_with(&t, &mut forever_busy).is_err());
        assert_eq!((forever_busy.requests.len(), forever_busy.reads), (1, MAX_READS));
    }

    #[test]
    fn malformed_or_failed_queries_publish_no_sample_and_can_recover() {
        let t = target(&interface("one"));
        for response in [Err("timeout".into()), Ok(vec![1, 1, 47]),
            Ok(synthetic_frame(1, 0, 47)), Ok(synthetic_frame(1, 1, 0))] {
            let mut fake = Fake::new(vec![response]);
            assert!(read_with(&t, &mut fake).is_err());
            assert_eq!((fake.requests.len(), fake.reads), (1, 1));
        }
        let mut fake = Fake::new(vec![Ok(synthetic_frame(1, 1, 47))]);
        fake.fail_send = true;
        assert!(read_with(&t, &mut fake).is_err());
        assert_eq!(fake.reads, 0);
        fake.fail_send = false;
        assert_eq!(read_with(&t, &mut fake).unwrap().battery.unwrap().percent, Some(47));
        let before = fake.requests.len();
        let mut wrong = t; wrong.backend = BackendId::Rapoo;
        assert!(read_with(&wrong, &mut fake).is_err());
        assert_eq!(before, fake.requests.len());
    }
}
