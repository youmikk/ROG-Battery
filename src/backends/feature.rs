//! Feature 报告的限时收发，只用于已确认协议的控制接口。
use crate::{hid::HidInterface, win32::*};
use std::time::{Duration, Instant};

const GET_FEATURE: u32 = 0x000b_0192;
const SET_FEATURE: u32 = 0x000b_0191;

pub(super) struct FeatureChannel {
    handle: Handle,
    event: Handle,
    length: usize,
}

impl FeatureChannel {
    pub fn open(interface: &HidInterface) -> Result<Self, String> {
        let length = usize::from(interface.feature_len);
        if !(2..=4096).contains(&length) {
            return Err(crate::tr!("设备功能报告长度无效").into());
        }
        // Feature IOCTL 不需要键盘/鼠标输入流的读写权限。
        let handle = unsafe {
            CreateFileW(
                to_wide(&interface.path).as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                core::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                core::ptr::null_mut(),
            )
        };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(crate::tr!("无法打开设备功能接口").into());
        }
        let event = unsafe { CreateEventW(core::ptr::null_mut(), 0, 0, core::ptr::null()) };
        if event.is_null() || event == INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(handle);
            }
            return Err(crate::tr!("无法创建设备通信事件").into());
        }
        Ok(Self {
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
                return Err(crate::tr!("设备功能报告传输失败").into());
            }
            if unsafe { WaitForSingleObject(self.event, timeout) } != WAIT_OBJECT_0 {
                // 内核取消完成后才能释放缓冲区及 OVERLAPPED。
                unsafe {
                    CancelIoEx(self.handle, &mut overlapped);
                    GetOverlappedResult(self.handle, &mut overlapped, &mut returned, 1);
                }
                return Err(crate::tr!("设备功能报告传输超时").into());
            }
            if unsafe { GetOverlappedResult(self.handle, &mut overlapped, &mut returned, 0) } == 0 {
                return Err(crate::tr!("设备功能报告传输失败").into());
            }
        }
        Ok(returned as usize)
    }

    pub fn query(&mut self, packet: &[u8], timeout: u32) -> Result<Vec<u8>, String> {
        if packet.len() > self.length || packet.is_empty() {
            return Err(crate::tr!("设备功能报告长度无效").into());
        }
        let deadline = Instant::now() + Duration::from_millis(u64::from(timeout));
        let mut request = vec![0; self.length];
        request[..packet.len()].copy_from_slice(packet);
        self.transfer(SET_FEATURE, &mut request, timeout)?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| crate::tr!("设备功能报告传输超时"))?;
        let mut response = vec![0; self.length];
        response[0] = packet[0];
        let length = self.transfer(
            GET_FEATURE,
            &mut response,
            remaining.as_millis().max(1) as u32,
        )?;
        let length = length + usize::from(packet[0] == 0);
        if length == 0 || length > self.length || response[0] != packet[0] {
            return Err(crate::tr!("设备功能报告长度无效").into());
        }
        response.truncate(length);
        Ok(response)
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
