//! Feature 报告的限时收发，只用于已确认协议的控制接口。
use crate::{hid::HidInterface, win32::*};
use std::time::{Duration, Instant};

const GET_FEATURE: u32 = 0x000b_0192;
const SET_FEATURE: u32 = 0x000b_0191;

pub(super) struct FeatureChannel {
    capture_path: String,
    handle: Handle,
    event: Handle,
    length: usize,
}

impl FeatureChannel {
    pub fn open(interface: &HidInterface) -> Result<Self, String> {
        Self::open_with_length(interface, usize::from(interface.feature_len))
    }

    /// Only a protocol with independently confirmed report size may use this for
    /// an unadvertised control report (some Razer mouse collections).
    pub(super) fn open_with_length(interface: &HidInterface, length: usize) -> Result<Self, String> {
        Self::open_access(interface,length,0)
    }

    fn open_access(interface: &HidInterface, length: usize, access: u32) -> Result<Self, String> {
        if !(2..=4096).contains(&length) {
            return Err(crate::tr!("设备通信数据格式异常").into());
        }
        // Feature queries need no input-stream access; Output Report uses write access.
        let handle = unsafe {
            CreateFileW(
                to_wide(&interface.path).as_ptr(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                core::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                core::ptr::null_mut(),
            )
        };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(crate::tr!("暂时无法连接设备，请确认未被其他软件占用").into());
        }
        let event = unsafe { CreateEventW(core::ptr::null_mut(), 0, 0, core::ptr::null()) };
        if event.is_null() || event == INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(handle);
            }
            return Err(crate::tr!("暂时无法建立设备通信，请重试").into());
        }
        Ok(Self {
            capture_path: interface.path.clone(),
            handle,
            event,
            length,
        })
    }

    fn transfer(&mut self, code: u32, data: &mut [u8], timeout: u32) -> Result<usize, String> {
        let mut overlapped = Overlapped {
            h_event: self.event,
            ..Default::default()
        };
        let mut returned = 0;
        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                code,
                data.as_mut_ptr().cast(),
                data.len() as u32,
                data.as_mut_ptr().cast(),
                data.len() as u32,
                &mut returned,
                &mut overlapped,
            )
        };
        if ok == 0 {
            if last_error() != ERROR_IO_PENDING {
                return Err(crate::tr!("与设备通信失败，请检查连接后重试").into());
            }
            if unsafe { WaitForSingleObject(self.event, timeout) } != WAIT_OBJECT_0 {
                // 内核取消完成后才能释放缓冲区及 OVERLAPPED。
                unsafe {
                    CancelIoEx(self.handle, &mut overlapped);
                    GetOverlappedResult(self.handle, &mut overlapped, &mut returned, 1);
                }
                return Err(crate::tr!("等待设备响应超时，请轻动设备唤醒后再试").into());
            }
            if unsafe { GetOverlappedResult(self.handle, &mut overlapped, &mut returned, 0) } == 0 {
                return Err(crate::tr!("与设备通信失败，请检查连接后重试").into());
            }
        }
        Ok(returned as usize)
    }

    pub fn send(&mut self, packet: &[u8], timeout: u32) -> Result<(), String> {
        if packet.len() > self.length || packet.is_empty() {
            return Err(crate::tr!("设备通信数据格式异常").into());
        }
        let mut request = vec![0; self.length];
        request[..packet.len()].copy_from_slice(packet);
        let result = self.transfer(SET_FEATURE, &mut request, timeout);
        crate::capture::record(&self.capture_path, "set-feature", &request, result.is_ok());
        result?;
        Ok(())
    }

    pub fn receive(&mut self, report_id: u8, timeout: u32) -> Result<Vec<u8>, String> {
        let response = self.receive_raw(report_id, timeout)?;
        if response[0] != report_id {
            return Err(crate::tr!("设备通信数据格式异常").into());
        }
        Ok(response)
    }

    /// Rapoo acknowledges in the report-ID byte on some Windows transports.
    /// Its protocol parser validates the status and payload after this transfer.
    pub(super) fn receive_raw(&mut self, report_id: u8, timeout: u32) -> Result<Vec<u8>, String> {
        self.receive_raw_with(report_id, timeout, |channel, response, timeout| {
            channel.transfer(GET_FEATURE, response, timeout)
        })
    }

    fn receive_raw_with(&mut self, report_id: u8, timeout: u32,
        transfer: impl FnOnce(&mut Self, &mut [u8], u32) -> Result<usize, String>) -> Result<Vec<u8>, String> {
        let mut response = vec![0; self.length];
        response[0] = report_id;
        let result = transfer(self, &mut response, timeout);
        crate::hid::capture_control_result(&self.capture_path, "get-feature", &response,
            result.as_ref().ok().copied(), report_id == 0);
        let returned = result?;
        let length = returned.saturating_add(usize::from(report_id == 0));
        if returned == 0 || length > self.length {
            return Err(crate::tr!("设备通信数据格式异常").into());
        }
        response.truncate(length);
        Ok(response)
    }

    pub(super) fn open_output(interface: &HidInterface) -> Result<Self, String> {
        Self::open_access(interface, usize::from(interface.output_len), GENERIC_WRITE)
    }

    pub(super) fn send_output(&mut self, packet: &[u8], timeout: u32) -> Result<(), String> {
        if packet.is_empty() || packet.len() != self.length {
            return Err(crate::tr!("设备通信数据格式异常").into());
        }
        let mut request = packet.to_vec();
        let result = self.transfer(winapi::shared::hidclass::IOCTL_HID_SET_OUTPUT_REPORT, &mut request, timeout);
        crate::capture::record(&self.capture_path, "set-output", &request, result.is_ok());
        result?;
        Ok(())
    }

    pub fn query(&mut self, packet: &[u8], timeout: u32) -> Result<Vec<u8>, String> {
        let deadline = Instant::now() + Duration::from_millis(u64::from(timeout));
        self.send(packet, timeout)?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| crate::tr!("等待设备响应超时，请轻动设备唤醒后再试"))?;
        self.receive(packet[0], remaining.as_millis().max(1) as u32)
    }
}

impl Drop for FeatureChannel {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.event);
            CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_capture_records_zero_short_oversized_and_failed_native_results() {
        let interface = HidInterface { path: "feature-native-capture".into(), vendor_id: 0x1234,
            product_id: 0x5678, usage_page: 0xff00, usage: 1, input_len: 0,
            output_len: 0, feature_len: 4, product: String::new(), serial: String::new() };
        for (report_id, returned, accepted, expected_hex) in [
            (7, Some(3), true, "08ABCD"),
            (7, Some(0), false, ""),
            (0, Some(0), false, ""),
            (7, Some(5), false, "08ABCDEE"),
            (0, Some(3), true, "08ABCDEE"),
            (0, Some(4), false, "08ABCDEE"),
            (7, None, false, ""),
        ] {
            let capture = crate::hid::capture_test_support::CaptureGuard::start(&interface);
            let mut channel = std::mem::ManuallyDrop::new(FeatureChannel {
                capture_path: interface.path.clone(), handle: core::ptr::null_mut(),
                event: core::ptr::null_mut(), length: 4,
            });
            let result = channel.receive_raw_with(report_id, 160, |_, response, timeout| {
                assert_eq!(timeout, 160);
                assert_eq!(response, &[report_id, 0, 0, 0]);
                response.copy_from_slice(&[8, 0xab, 0xcd, 0xee]);
                returned.ok_or_else(|| "cancelled".into())
            });
            assert_eq!(result.is_ok(), accepted);
            let records = capture.transfers();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0]["api"], "get-feature");
            assert_eq!(records[0]["hex"], expected_hex);
            assert_eq!(records[0]["success"], returned.is_some());
        }
    }
}
