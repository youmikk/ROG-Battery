//! Rapoo 24AE:1851 receiver battery query over Output BA and GET_REPORT(Input).
//! The nRF54L Feature channel and EEPROM/configuration commands are not used.
use super::*;
use crate::win32::*;
use std::time::{Duration, Instant};
use winapi::shared::hidclass::IOCTL_HID_GET_INPUT_REPORT;

pub struct RapooLegacy;
const REPORT: u8 = 0xba;
const LENGTH: u16 = 32;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x24ae && i.product_id == 0x1851
        && i.usage_page == 0xff00 && i.usage == 0x0e
}

fn control(i: &HidInterface) -> bool {
    if !management(i) || i.input_len != LENGTH || i.output_len != LENGTH || i.feature_len != 0 {
        return false;
    }
    // The community capture identifies col04 as this receiver's command channel.
    // Its col05 status stream and other 33/513-byte interfaces are unrelated.
    let path = i.path.to_ascii_lowercase();
    path.split('#').nth(1).is_some_and(|part| part.split('&').any(|field| field == "col04"))
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
    let mut t = Target::new(BackendId::RapooLegacy, i, 0);
    // Neither the shared USB name nor 1851 identifies the paired retail model.
    t.name = "Rapoo VT 2.4G".into();
    t.family = DeviceFamily::Mouse;
    t
}

fn request() -> [u8; LENGTH as usize] {
    let mut p = [0; LENGTH as usize];
    p[..4].copy_from_slice(&[REPORT, 0xa5, 0xaa, 0]);
    p
}

fn battery(r: &[u8]) -> Option<Battery> {
    // Windows may replace the report-ID slot with the status byte. The other
    // documented layout retains BA before the 31-byte response body.
    let data = match (r.len(), r.first()) {
        (32, Some(&REPORT)) => &r[1..],
        (31 | 32, Some(&1)) => r,
        _ => return None,
    };
    if data[0] != 1 || !matches!(data[1], 1 | 2) || data[2] > 100 { return None; }
    // Marker 0 is an EEPROM block response, not a zero-percent battery.
    Some(Battery { percent: Some(data[2]), charging: Some(data[1] == 2),
        estimated_percent: None, voltage_mv: None, approximate: false, level: None })
}

trait Transport {
    fn send(&mut self, packet: &[u8], timeout_ms: u32) -> Result<(), String>;
    fn receive(&mut self, report_id: u8, timeout_ms: u32) -> Result<Vec<u8>, String>;
    fn pause(&self) {}
}

struct NativeTransport { handle: Handle, reply_bytes: Option<usize> }

impl NativeTransport {
    fn open(i: &HidInterface) -> Result<Self, String> {
        let handle = unsafe { CreateFileW(to_wide(&i.path).as_ptr(), GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE, core::ptr::null_mut(), OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED, core::ptr::null_mut()) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(crate::tr!("暂时无法连接设备，请确认未被其他软件占用").into());
        }
        Ok(Self { handle, reply_bytes: None })
    }

    fn transfer(&mut self, timeout_ms: u32,
        operation: impl FnOnce(Handle, &mut Overlapped, &mut u32) -> i32) -> Result<usize, String> {
        let event = unsafe { CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null()) };
        if event.is_null() || event == INVALID_HANDLE_VALUE {
            return Err(crate::tr!("暂时无法建立设备通信，请重试").into());
        }
        let mut ov = Overlapped { h_event: event, ..Default::default() };
        let mut count = 0;
        let result = (|| {
            if operation(self.handle, &mut ov, &mut count) == 0 {
                if last_error() != ERROR_IO_PENDING {
                    return Err(crate::tr!("与设备通信失败，请检查连接后重试").into());
                }
                if unsafe { WaitForSingleObject(event, timeout_ms) } != WAIT_OBJECT_0 {
                    // The kernel must finish cancellation before the caller's
                    // buffer, this OVERLAPPED or its event is released.
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
        // This transport exposes no general register/write command path.
        if packet != request() { return Err(UNAVAILABLE.into()); }
        let sent = self.transfer(timeout_ms, |handle, ov, count| unsafe {
            WriteFile(handle, packet.as_ptr().cast(), packet.len() as u32, count, ov)
        })?;
        if sent != packet.len() { return Err(crate::tr!("与设备通信失败，请检查连接后重试").into()); }
        Ok(())
    }

    fn receive(&mut self, report_id: u8, timeout_ms: u32) -> Result<Vec<u8>, String> {
        if report_id != REPORT { return Err(UNAVAILABLE.into()); }
        let mut packet = vec![0; usize::from(LENGTH)];
        packet[0] = report_id;
        let received = self.transfer(timeout_ms, |handle, ov, count| unsafe {
            DeviceIoControl(handle, IOCTL_HID_GET_INPUT_REPORT,
                packet.as_mut_ptr().cast(), packet.len() as u32,
                packet.as_mut_ptr().cast(), packet.len() as u32, count, ov)
        })?;
        self.reply_bytes = Some(received);
        if !matches!(received, 31 | 32) { return Err(UNAVAILABLE.into()); }
        packet.truncate(received);
        Ok(packet)
    }

    fn pause(&self) { std::thread::sleep(Duration::from_millis(250)); }
}

fn read_with(t: &Target, transport: &mut impl Transport) -> Result<Snapshot, String> {
    let i = t.hid()?;
    if t.backend != BackendId::RapooLegacy || !control(i) { return Err(UNAVAILABLE.into()); }
    // No retry: this family does not echo a transaction or the battery command.
    transport.send(&request(), 250)?;
    transport.pause();
    let response = transport.receive(REPORT, 500)?;
    let battery = battery(&response).ok_or(UNAVAILABLE)?;
    let mut target = target(i);
    target.verified_identity = true;
    Ok(Snapshot { target, battery: Some(battery), battery_at: Some(Instant::now()),
        battery_unavailable: false, connected: true, dpi: Dpi::default(),
        polling_rate: Default::default() })
}

impl DeviceBackend for RapooLegacy {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i)).map(target).collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        let i = t.hid()?;
        if t.backend != BackendId::RapooLegacy || !proven(api, i) { return Err(UNAVAILABLE.into()); }
        let lock = channel::lock(&channel::collection_group(i));
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let started = Instant::now();
        let mut transport = match NativeTransport::open(i) {
            Ok(transport) => transport,
            Err(error) => {
                trace::record(i, BackendId::RapooLegacy, "query-battery", 0xaa, REPORT,
                    0, 0, started, trace::Outcome::NoReply);
                return Err(error);
            }
        };
        let result = read_with(t, &mut transport);
        trace::record(i, BackendId::RapooLegacy, "query-battery", 0xaa, REPORT,
            usize::from(LENGTH), transport.reply_bytes.unwrap_or(0), started,
            if result.is_ok() { trace::Outcome::Ok }
            else if transport.reply_bytes.is_some() { trace::Outcome::InvalidData }
            else { trace::Outcome::NoReply });
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(instance: &str) -> HidInterface {
        HidInterface { path: format!(r"\\?\hid#vid_24ae&pid_1851&mi_01&col04#8&{instance}&0004#{{hid}}"),
            vendor_id: 0x24ae, product_id: 0x1851, usage_page: 0xff00, usage: 0x0e,
            input_len: 32, output_len: 32, feature_len: 0,
            product: "Rapoo Gaming Device".into(), serial: "shared".into() }
    }

    fn frame(percent: u8, state: u8, length: usize) -> Vec<u8> {
        let mut r = vec![0; length];
        r[..3].copy_from_slice(&[1, state, percent]);
        r
    }

    struct Fake {
        sent: Vec<Vec<u8>>,
        read: usize,
        result: Result<Vec<u8>, String>,
        fail_send: bool,
    }
    impl Transport for Fake {
        fn send(&mut self, packet: &[u8], timeout: u32) -> Result<(), String> {
            assert_eq!(timeout, 250);
            self.sent.push(packet.to_vec());
            if self.fail_send { Err("write failed".into()) } else { Ok(()) }
        }
        fn receive(&mut self, id: u8, timeout: u32) -> Result<Vec<u8>, String> {
            assert_eq!((id, timeout), (REPORT, 500));
            self.read += 1;
            self.result.clone()
        }
    }

    #[test]
    fn legacy_receiver_control_is_separate_from_nrf54l_status_and_firmware_interfaces() {
        let i = interface("one");
        assert!(control(&i));
        for (vid, pid, page, usage, input, output, feature) in [
            (0x24ae, 0x1415, 0xff00, 2, 19, 19, 0),
            (0x24ae, 0x1851, 0xff00, 2, 7, 0, 0),
            (0x24ae, 0x1851, 0xff00, 0x0f, 0, 0, 33),
            (0x24ae, 0x1851, 0xff00, 0x0e, 33, 33, 0),
            (0x24ae, 0x1851, 0xff00, 0x0e, 513, 513, 0),
            (0x24ae, 0x1851, 1, 2, 32, 32, 0),
            (0x24ae, 0x1205, 0xff00, 0x0e, 32, 32, 0),
            (0x046d, 0x1851, 0xff00, 0x0e, 32, 32, 0),
        ] {
            let mut wrong = i.clone();
            wrong.vendor_id = vid; wrong.product_id = pid; wrong.usage_page = page;
            wrong.usage = usage; wrong.input_len = input; wrong.output_len = output;
            wrong.feature_len = feature;
            assert!(!control(&wrong));
        }
        let mut sibling = i.clone();
        sibling.path = sibling.path.replace("col04", "col06").replace("&0004#", "&0006#");
        assert!(candidates(&[i.clone(), sibling]).is_empty());
        let mut wrong = interface("one");
        wrong.path = "unstructured".into();
        assert!(!control(&wrong));
        let other = interface("two");
        assert_eq!(candidates(&[i.clone(), other.clone()]).len(), 2);
        assert_ne!(target(&i).key, target(&other).key);
    }

    #[test]
    fn published_battery_prefixes_and_report_id_layouts_decode_without_config_confusion() {
        // PR #76 supplies the 01 01 5B and 01 02 64 prefixes; padding is synthetic.
        for length in [31, 32] {
            let discharge = battery(&frame(91, 1, length)).unwrap();
            assert_eq!((discharge.percent, discharge.charging), (Some(91), Some(false)));
            let charged = battery(&frame(100, 2, length)).unwrap();
            assert_eq!((charged.percent, charged.charging), (Some(100), Some(true)));
        }
        let mut with_id = vec![REPORT]; with_id.extend(frame(91, 1, 31));
        assert_eq!(battery(&with_id).unwrap().percent, Some(91));
        for percent in 0..=100 {
            for state in [1, 2] {
                let b = battery(&frame(percent, state, 32)).unwrap();
                assert_eq!((b.percent, b.charging), (Some(percent), Some(state == 2)));
            }
        }
        for percent in 101..=255 { assert!(battery(&frame(percent, 1, 32)).is_none()); }
        for state in [0, 3, 0x7f, 0xff] { assert!(battery(&frame(91, state, 32)).is_none()); }
        let valid = frame(91, 1, 32);
        for length in 0..31 { assert!(battery(&valid[..length]).is_none()); }
        assert!(battery(&frame(91, 1, 33)).is_none());
        assert!(battery(&request()).is_none());
        assert!(battery(&[0; 32]).is_none());
        for status in [0, 2, 3, 0xbb] {
            let mut r = valid.clone(); r[0] = status; assert!(battery(&r).is_none());
        }
    }

    #[test]
    fn failed_busy_and_malformed_reads_never_retry_or_publish_a_new_sample() {
        let t = target(&interface("one"));
        for response in [Err("timeout".into()), Ok(vec![2; 32]), Ok(frame(91, 0, 32)), Ok(vec![1, 1, 91])] {
            let mut fake = Fake { sent: vec![], read: 0, result: response, fail_send: false };
            assert!(read_with(&t, &mut fake).is_err());
            assert_eq!((fake.sent.len(), fake.read), (1, 1));
            assert_eq!(fake.sent[0], request());
            assert!(fake.sent[0][4..].iter().all(|b| *b == 0));
        }
        let mut fake = Fake { sent: vec![], read: 0, result: Ok(frame(91, 1, 32)), fail_send: true };
        assert!(read_with(&t, &mut fake).is_err());
        assert_eq!((fake.sent.len(), fake.read), (1, 0));
        fake.fail_send = false;
        let sample = read_with(&t, &mut fake).unwrap();
        assert!(sample.connected && sample.battery_at.is_some() && !sample.battery_unavailable);
        assert_eq!(sample.battery.unwrap().percent, Some(91));
        assert!(sample.dpi.stages.is_empty() && !sample.dpi.writable && !sample.polling_rate.writable);
        let mut wrong = t; wrong.backend = BackendId::Rapoo;
        let before = fake.sent.len();
        assert!(read_with(&wrong, &mut fake).is_err());
        assert_eq!(before, fake.sent.len());
    }
}
