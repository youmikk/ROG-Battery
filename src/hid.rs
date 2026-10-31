//! HID 设备枚举、打开与收发。
//!
//! 整体照 `_research\rw-hid.cs` 的 `Enumerate` / `Dev` / `Xfer` 三段做等价实现：
//! - 用 setupapi 枚举 HID 接口，取 `\\?\hid#...` 路径；
//! - `CreateFile` 打开时**必须** `FILE_SHARE_READ | FILE_SHARE_WRITE`（Armoury Crate 可能同时开着）；
//! - 写用 `WriteFile`（同步句柄），读用「overlapped 句柄 + 独立事件 + 超时」；
//! - 应答定位：先找 `r[1]==cmd && r[2]==sub`，找不到再找 `r[0]==cmd && r[1]==sub`（后者要补一个 report id 到下标 0）。

use crate::win32::*;
use core::ffi::c_void;

// ---------------- 常量 ----------------
pub const VID_ASUS: u16 = 0x0B05;
pub const PID_DONGLE: u16 = 0x1ACE;
pub const PID_MOUSE_WIRELESS: u16 = 0x1B65;
pub const PID_MOUSE_WIRED: u16 = 0x1B63;

/// 接收器控制通道（report id 0x01）
pub const USAGE_PAGE_DONGLE_CTRL: u16 = 0xFF02;
/// 鼠标命令通道（report id 0x03）—— 本程序只走这条
pub const USAGE_PAGE_MOUSE_CTRL: u16 = 0xFF01;

/// 默认单帧超时。实测一次正常往返 ~27ms，给 1200ms 已经非常宽裕。
pub const DEFAULT_TIMEOUT_MS: u32 = 1200;

/// 否定应答特征：`03 FF AA`
pub fn is_nak(reply: &[u8]) -> bool {
    reply.len() >= 3 && reply[1] == 0xFF && reply[2] == 0xAA
}

// ---------------- 接口描述 ----------------
#[derive(Clone, Debug)]
pub struct HidInterface {
    pub path: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub usage_page: u16,
    pub usage: u16,
    pub input_len: u16,
    pub output_len: u16,
    pub product: String,
    pub serial: String,
}

impl HidInterface {
    /// 是否是命令通道：usage page 为 0xFF01/0xFF02、usage=1、输入长度至少 64。
    pub fn is_command_channel(&self) -> bool {
        (self.usage_page == USAGE_PAGE_MOUSE_CTRL || self.usage_page == USAGE_PAGE_DONGLE_CTRL)
            && self.usage == 1
            && self.input_len >= 64
    }
}

// ---------------- 枚举 ----------------

/// 枚举时被跳过的接口：路径 + 打不开的原因（权限 / 被占用 / 取不到路径）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}

/// 枚举结果：拿到的接口，以及**打不开的那些**及其原因。
///
/// 两者必须分开：「一个接口都没打开」和「根本没有 ASUS 设备」是两件事 ——
/// 前者通常是 Armoury Crate 占着设备或权限不足，后者才是真的没插。
/// 原来 `Err(_) => continue` 把两者糊成同一个「找不到设备」，用户完全没法排查。
pub struct EnumerateReport {
    pub interfaces: Vec<HidInterface>,
    pub skipped: Vec<Skipped>,
}

/// 枚举本机所有 ASUS HID 接口（只关心拿到的那些）。
pub fn enumerate(hid: &HidApi) -> Result<Vec<HidInterface>, String> {
    Ok(enumerate_report(hid)?.interfaces)
}

/// 枚举本机所有 ASUS HID 接口，同时带回打不开的接口清单。
pub fn enumerate_report(hid: &HidApi) -> Result<EnumerateReport, String> {
    let mut list = Vec::new();
    let mut skipped: Vec<Skipped> = Vec::new();

    let mut guid = Guid::default();
    unsafe { (hid.get_hid_guid)(&mut guid) };

    let dev_info = unsafe {
        SetupDiGetClassDevsW(
            &guid,
            core::ptr::null(),
            core::ptr::null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    };
    if dev_info == INVALID_HANDLE_VALUE || dev_info.is_null() {
        return Err(format!(
            "SetupDiGetClassDevsW 失败（GetLastError={}）",
            last_error()
        ));
    }

    // 无论成败都要销毁 dev_info
    let result = (|| -> Result<(), String> {
        let mut index: u32 = 0;
        loop {
            let mut did = SpDeviceInterfaceData::default();
            let ok = unsafe {
                SetupDiEnumDeviceInterfaces(dev_info, core::ptr::null_mut(), &guid, index, &mut did)
            };
            if ok == 0 {
                let err = last_error();
                // ERROR_NO_MORE_ITEMS 是正常结束
                if err == ERROR_NO_MORE_ITEMS || index > 0 {
                    break;
                }
                return Err(format!(
                    "SetupDiEnumDeviceInterfaces 失败（GetLastError={err}）"
                ));
            }
            index += 1;

            let Some(path) = interface_path(dev_info, &mut did)? else {
                continue;
            };
            match probe_interface(hid, &path) {
                Ok(iface) => {
                    if iface.vendor_id != VID_ASUS {
                        continue;
                    }
                    list.push(iface);
                }
                // 单个接口打不开（权限 / 被占用）不算致命，但要留下原因，
                // 否则「设备插着却报找不到」没法解释。
                Err(reason) => skipped.push(Skipped { path, reason }),
            }
        }
        Ok(())
    })();

    unsafe { SetupDiDestroyDeviceInfoList(dev_info) };
    result?;
    Ok(EnumerateReport {
        interfaces: list,
        skipped,
    })
}

/// 一个可用接口都没有时该报什么。
///
/// 有跳过记录说明设备在、但打开失败（Armoury Crate 抢占 / 权限），这时
/// 报「没有找到设备」会把人带偏，必须把第一条原因原样带出来。
fn empty_reason(skipped: &[Skipped]) -> String {
    match skipped.first() {
        None => "没有找到任何 ASUS HID 接口（VID 0x0B05）".to_string(),
        Some(first) => format!(
            "找到 {} 个 HID 接口但一个都打不开（最常见是 Armoury Crate 占着设备，其次是权限）：{}",
            skipped.len(),
            first.reason
        ),
    }
}

/// `SetupDiGetDeviceInterfaceDetailW` 需要两次调用：先问长度，再取内容。
fn interface_path(
    dev_info: Handle,
    did: &mut SpDeviceInterfaceData,
) -> Result<Option<String>, String> {
    let mut required: u32 = 0;
    unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            dev_info,
            did,
            core::ptr::null_mut(),
            0,
            &mut required,
            core::ptr::null_mut(),
        )
    };
    if required == 0 {
        return Ok(None);
    }

    // SP_DEVICE_INTERFACE_DETAIL_DATA_W：cbSize(4) 之后是 WCHAR 路径。
    // x64 下 cbSize = 8（含 4 字节对齐填充），x86 下是 6。
    let cb_size: u32 = if core::mem::size_of::<usize>() == 8 {
        8
    } else {
        6
    };
    let mut buf = vec![0u8; required as usize];
    unsafe {
        core::ptr::write_unaligned(buf.as_mut_ptr() as *mut u32, cb_size);
    }

    let ok = unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            dev_info,
            did,
            buf.as_mut_ptr() as *mut c_void,
            required,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Ok(None);
    }

    let wchars: Vec<u16> = buf[4..]
        .chunks_exact(2)
        .map(|c| u16::from_ne_bytes([c[0], c[1]]))
        .collect();
    let end = wchars.iter().position(|&c| c == 0).unwrap_or(wchars.len());
    Ok(Some(String::from_utf16_lossy(&wchars[..end])))
}

/// 打开一个接口，读它的属性与 caps，然后立刻关闭。
fn probe_interface(hid: &HidApi, path: &str) -> Result<HidInterface, String> {
    let wide = to_wide(path);
    let h = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            core::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            core::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE || h.is_null() {
        return Err(format!("CreateFileW 失败（GetLastError={}）", last_error()));
    }

    let out = (|| -> Result<HidInterface, String> {
        let mut attr = HiddAttributes {
            size: core::mem::size_of::<HiddAttributes>() as u32,
            ..Default::default()
        };
        if unsafe { (hid.get_attributes)(h, &mut attr) } == 0 {
            return Err(format!(
                "HidD_GetAttributes 失败（GetLastError={}）",
                last_error()
            ));
        }

        let mut pp: *mut c_void = core::ptr::null_mut();
        if unsafe { (hid.get_preparsed_data)(h, &mut pp) } == 0 {
            return Err(format!(
                "HidD_GetPreparsedData 失败（GetLastError={}）",
                last_error()
            ));
        }
        let mut caps = HidpCaps::default();
        let rc = unsafe { (hid.get_caps)(pp, &mut caps) };
        unsafe { (hid.free_preparsed_data)(pp) };
        if rc < 0 {
            // HIDP_STATUS_* 为负即失败
            return Err(format!("HidP_GetCaps 返回 {rc}"));
        }

        Ok(HidInterface {
            path: path.to_string(),
            vendor_id: attr.vendor_id,
            product_id: attr.product_id,
            usage_page: caps.usage_page,
            usage: caps.usage,
            input_len: caps.input_report_byte_length,
            output_len: caps.output_report_byte_length,
            product: read_hid_string(hid, h, 0),
            serial: read_hid_string(hid, h, 1),
        })
    })();

    unsafe { CloseHandle(h) };
    out
}

/// 读 HID 字符串（0=产品名，1=序列号）。失败返回空串，不报错。
///
// 只保留开头连续 ASCII 字母数字：OMNI 接收器的 HID 字符串描述符 bLength 虚高，
// 尾部是固件残留。实验记录与「试过但不行」的方案见 docs/hardware-notes.md。
fn read_hid_string(hid: &HidApi, h: Handle, which: u8) -> String {
    let mut buf = vec![0u16; 256];
    let cap = (buf.len() * 2) as u32;
    let ok = unsafe {
        if which == 0 {
            (hid.get_product_string)(h, buf.as_mut_ptr(), cap)
        } else {
            (hid.get_serial_number_string)(h, buf.as_mut_ptr(), cap)
        }
    };
    if ok == 0 {
        return String::new();
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let s = String::from_utf16_lossy(&buf[..end]).trim().to_string();

    // 只对序列号做清洗（产品名没观察到这个问题，不动它）
    if which == 1 {
        let leading: String = s
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        return leading;
    }
    s
}

// ---------------- 打开的设备 ----------------
/// 一个已打开的命令通道。持有两个句柄：同步写句柄 + overlapped 读句柄。
pub struct HidDevice {
    /// 同步句柄，只用来 `WriteFile`
    write_handle: Handle,
    /// overlapped 读句柄
    read_handle: Handle,
    read_event: Handle,
    read_buf: Vec<u8>,
    /// 一次读挂起期间必须复用同一个 OVERLAPPED，否则 GetOverlappedResult 拿不到结果
    read_ov: Overlapped,
    /// 写也走 overlapped：同步句柄上 WriteFile 没有超时，驱动卡住就会把调用线程
    /// （GUI 的刷新线程）钉死。
    write_event: Handle,
    write_ov: Overlapped,
    /// 这次读是否在 ReadFile 里同步完成了（少数情况，HID 一般都会 pending）
    read_completed_sync: bool,
    /// 同步完成时实际读到的字节数
    sync_read_len: u32,
    in_len: u16,
    out_len: u16,
    /// 只有带 GENERIC_WRITE 打开成功时才允许写
    pub writable: bool,
    pub product: String,
    pub serial: String,
}

impl HidDevice {
    /// 打开指定路径的命令通道。写句柄开不了时自动降级为只读（`writable=false`）。
    pub fn open(iface: &HidInterface) -> Result<HidDevice, String> {
        let wide = to_wide(&iface.path);

        let mut writable = true;
        let mut write_handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                core::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                core::ptr::null_mut(),
            )
        };
        if write_handle == INVALID_HANDLE_VALUE || write_handle.is_null() {
            writable = false;
            write_handle = unsafe {
                CreateFileW(
                    wide.as_ptr(),
                    GENERIC_READ,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    core::ptr::null_mut(),
                    OPEN_EXISTING,
                    0,
                    core::ptr::null_mut(),
                )
            };
        }
        if write_handle == INVALID_HANDLE_VALUE || write_handle.is_null() {
            return Err(format!(
                "无法打开设备（GetLastError={}）；可能被 Armoury Crate 独占或设备已拔出",
                last_error()
            ));
        }

        let read_handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                core::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                core::ptr::null_mut(),
            )
        };
        if read_handle == INVALID_HANDLE_VALUE || read_handle.is_null() {
            unsafe { CloseHandle(write_handle) };
            return Err(format!(
                "无法打开 overlapped 读句柄（GetLastError={}）",
                last_error()
            ));
        }

        let read_event = unsafe { CreateEventW(core::ptr::null_mut(), 0, 0, core::ptr::null()) };
        if read_event.is_null() || read_event == INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(read_handle);
                CloseHandle(write_handle);
            }
            return Err(format!(
                "CreateEventW 失败（GetLastError={}）",
                last_error()
            ));
        }

        let write_event = unsafe { CreateEventW(core::ptr::null_mut(), 0, 0, core::ptr::null()) };
        if write_event.is_null() || write_event == INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(read_event);
                CloseHandle(read_handle);
                CloseHandle(write_handle);
            }
            return Err(format!(
                "CreateEventW 失败（GetLastError={}）",
                last_error()
            ));
        }

        Ok(HidDevice {
            write_handle,
            write_event,
            write_ov: Overlapped {
                h_event: write_event,
                ..Default::default()
            },
            read_handle,
            read_event,
            read_buf: vec![0u8; 4096],
            read_ov: Overlapped {
                h_event: read_event,
                ..Default::default()
            },
            read_completed_sync: false,
            sync_read_len: 0,
            in_len: if iface.input_len > 0 {
                iface.input_len
            } else {
                64
            },
            out_len: if iface.output_len > 0 {
                iface.output_len
            } else {
                64
            },
            writable,
            product: iface.product.clone(),
            serial: iface.serial.clone(),
        })
    }

    pub fn input_len(&self) -> u16 {
        self.in_len
    }

    pub fn output_len(&self) -> u16 {
        self.out_len
    }

    /// 在枚举结果里挑一个指定 usage page 的命令通道打开。
    /// 用于 `info` 额外读接收器侧信息（0xFF02 通道）。
    pub fn open_matching(interfaces: &[HidInterface], usage_page: u16) -> Result<HidDevice, String> {
        let iface = interfaces
            .iter()
            .find(|x| x.usage_page == usage_page && x.is_command_channel())
            .ok_or_else(|| format!("没有找到 usage page 0x{usage_page:04X} 的命令通道"))?;
        HidDevice::open(iface)
    }

    /// 把输入缓冲里的陈旧推送帧抽干（`0xFF01` 会主动推送电量帧）。
    ///
    /// 每轮用 0ms 等待：队列里真有积压时数据已经就绪，`WaitForSingleObject` 立刻返回；
    /// 队列空则立刻超时退出。原来每轮固定等 15ms，`read_state` 的 8 帧光抽干就白花
    /// 120ms。迟到的推送帧仍会被 `align_reply` 按 cmd/sub 过滤掉，不影响正确性。
    fn drain(&mut self) {
        for _ in 0..8 {
            if !self.start_read(4096) {
                break;
            }
            if self.finish_read(0).is_none() {
                break;
            }
        }
    }

    /// 发起一次异步读。返回 false 表示句柄不可用。
    fn start_read(&mut self, want: usize) -> bool {
        let want = want.min(self.read_buf.len()).max(1) as u32;
        // 每次发起读之前重置 OVERLAPPED（句柄本身可以复用）
        self.read_ov = Overlapped {
            h_event: self.read_event,
            ..Default::default()
        };
        let mut read: u32 = 0;
        let ok = unsafe {
            ReadFile(
                self.read_handle,
                self.read_buf.as_mut_ptr() as *mut c_void,
                want,
                &mut read,
                &mut self.read_ov,
            )
        };
        if ok != 0 {
            // 同步完成：数据已经就绪，可以直接取。
            self.read_completed_sync = true;
            self.sync_read_len = read;
            return true;
        }
        self.read_completed_sync = false;
        last_error() == ERROR_IO_PENDING
    }

    /// 等一次读完成。超时返回 None（并取消掉这次读）。
    fn finish_read(&mut self, timeout_ms: u32) -> Option<Vec<u8>> {
        if self.read_completed_sync {
            let n = self.sync_read_len as usize;
            if n == 0 || n > self.read_buf.len() {
                return None;
            }
            return Some(self.read_buf[..n].to_vec());
        }

        let wait = unsafe { WaitForSingleObject(self.read_event, timeout_ms) };
        if wait != WAIT_OBJECT_0 {
            unsafe {
                CancelIoEx(self.read_handle, &mut self.read_ov);
                let mut xfer: u32 = 0;
                GetOverlappedResult(self.read_handle, &mut self.read_ov, &mut xfer, 1);
            }
            return None;
        }
        let mut xfer: u32 = 0;
        let ok = unsafe { GetOverlappedResult(self.read_handle, &mut self.read_ov, &mut xfer, 0) };
        if ok == 0 || xfer == 0 {
            return None;
        }
        Some(self.read_buf[..xfer as usize].to_vec())
    }

    /// 收尾一次写：同步完成直接成功；挂起则等到 `timeout_ms`，超时取消并判失败。
    ///
    /// 写句柄是 overlapped 打开的，所以驱动卡住时能把控制权抢回来——以前用同步
    /// 句柄写，`WriteFile` 没有超时，设备一挂就把整个刷新线程钉死。
    fn finish_write(&mut self, ok: i32, written: u32, timeout_ms: u32) -> bool {
        if ok != 0 {
            return written > 0;
        }
        if last_error() != ERROR_IO_PENDING {
            return false;
        }
        let wait = unsafe { WaitForSingleObject(self.write_event, timeout_ms) };
        if wait != WAIT_OBJECT_0 {
            unsafe {
                CancelIoEx(self.write_handle, &mut self.write_ov);
                let mut xfer: u32 = 0;
                GetOverlappedResult(self.write_handle, &mut self.write_ov, &mut xfer, 1);
            }
            return false;
        }
        let mut xfer: u32 = 0;
        let done = unsafe { GetOverlappedResult(self.write_handle, &mut self.write_ov, &mut xfer, 0) };
        done != 0 && xfer > 0
    }

    /// 发一帧并等应答。`tail` 从 `pkt[3]` 开始写（可以为空）。
    ///
    /// 返回的数组**已经把 report id 放在下标 0**（与文档对齐约定一致）。
    /// 超时返回 `None`——调用方应把它当成「鼠标休眠中」，**不要重试**。
    pub fn xfer(
        &mut self,
        report_id: u8,
        cmd: u8,
        sub: u8,
        tail: &[u8],
        timeout_ms: u32,
    ) -> Option<Vec<u8>> {
        if !self.writable {
            return None;
        }
        self.drain();

        let total = self.out_len.max(8) as usize;
        let mut pkt = vec![0u8; total];
        pkt[0] = report_id;
        pkt[1] = cmd;
        pkt[2] = sub;
        for (i, &b) in tail.iter().enumerate() {
            if 3 + i < total {
                pkt[3 + i] = b;
            }
        }

        self.write_ov = Overlapped {
            h_event: self.write_event,
            ..Default::default()
        };
        let mut written: u32 = 0;
        let ok = unsafe {
            WriteFile(
                self.write_handle,
                pkt.as_ptr() as *const c_void,
                pkt.len() as u32,
                &mut written,
                &mut self.write_ov,
            )
        };
        if !self.finish_write(ok, written, timeout_ms) {
            return None;
        }

        let want = self.in_len.max(1) as usize;
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms as u64);
        loop {
            let now = std::time::Instant::now();
            if now >= deadline {
                return None;
            }
            let left = (deadline - now).as_millis().max(1) as u32;
            if !self.start_read(want) {
                break;
            }
            let Some(r) = self.finish_read(left) else {
                continue;
            };

            if let Some(m) = align_reply(report_id, cmd, sub, &r) {
                return Some(m);
            }
            // 其它帧（例如主动推送的电量帧）丢掉，继续等
        }
        None
    }
}

/// 把一帧读到的数据对齐成「下标 0 = report id」的形式；与本次请求无关返回 `None`。
///
/// 四种合法形状（`rid` 为本次请求的 report id）：
/// - `<rid> <cmd> <sub> ...` —— 正常应答，原样返回
/// - `<rid> FF AA` —— 否定应答，原样返回（`is_nak` 认得）
/// - `FF AA` —— 省略 report id 的否定应答，补一个 rid
/// - `<cmd> <sub> ...` —— 省略 report id 的正常应答，补一个 rid
///
/// 否定应答必须显式放行：它的第二字节是 `0xFF` 而不是 `cmd`，只比 cmd/sub 会把它
/// 当「其它帧」丢掉，于是 proto/snapshot/ops/cli 里所有 `is_nak` 判定都不可达，
/// 设备明确拒绝会被误报成「鼠标无应答」并白等满一次超时。
fn align_reply(report_id: u8, cmd: u8, sub: u8, r: &[u8]) -> Option<Vec<u8>> {
    if r.len() >= 3 && r[0] == report_id && r[1] == cmd && r[2] == sub {
        return Some(r.to_vec());
    }
    if is_nak(r) && r[0] == report_id {
        return Some(r.to_vec());
    }
    let needs_prefix = (r.len() >= 2 && r[0] == 0xFF && r[1] == 0xAA)
        || (r.len() >= 2 && r[0] == cmd && r[1] == sub);
    if needs_prefix {
        let mut m = Vec::with_capacity(r.len() + 1);
        m.push(report_id);
        m.extend_from_slice(r);
        return Some(m);
    }
    None
}

impl Drop for HidDevice {
    fn drop(&mut self) {
        unsafe {
            if !self.read_event.is_null() && self.read_event != INVALID_HANDLE_VALUE {
                CloseHandle(self.read_event);
            }
            if !self.write_event.is_null() && self.write_event != INVALID_HANDLE_VALUE {
                CloseHandle(self.write_event);
            }
            if !self.read_handle.is_null() && self.read_handle != INVALID_HANDLE_VALUE {
                CloseHandle(self.read_handle);
            }
            if !self.write_handle.is_null() && self.write_handle != INVALID_HANDLE_VALUE {
                CloseHandle(self.write_handle);
            }
        }
    }
}

// ---------------- 链路 ----------------
/// 已建立的命令通道连接。
pub struct Link {
    /// 人类可读的模式：Wired / Wireless24G
    pub mode: String,
    pub report_id: u8,
    pub iface: HidInterface,
    /// 枚举到的全部 ASUS 接口，供 `info` 顺带读接收器侧信息
    pub all_interfaces: Vec<HidInterface>,
}

/// 解析接收器配对表（`A0 00` 的应答），找出 `wanted` 里第一个命中的 PID 的 report id。
///
/// 布局：`01 A0 00 <count LE16>` 之后紧跟 `count` 项、每项 4 字节
/// `{PID LE16, report_id, notify_id}`。表被截断（count 说的是 5 项却只回了 2 项）
/// 或没有匹配项都返回 `None` —— 调用方保留默认 report id 3，不要猜。
///
/// 单独抽出来是因为这段解析原先内联在 `open_link` 里，是全文件唯一没有单测的
/// 协议解析：`count` 是设备给的，越界读不 panic 但会读到垃圾 PID，把 report id
/// 定成 0 之后所有指令都会静默超时。
fn parse_pairing_table(scan: &[u8], wanted: &[u16]) -> Option<u8> {
if scan.len() < 6 {
    return None;
}
let count = scan[4] as usize | ((scan[5] as usize) << 8);
let mut at = 6;
for _ in 0..count {
    if at + 4 > scan.len() {
        return None;
    }
    let pid = scan[at] as u16 | ((scan[at + 1] as u16) << 8);
    if wanted.contains(&pid) {
        return Some(scan[at + 2]);
    }
    at += 4;
}
None
}

/// 找命令通道并确定 report id。
///
/// 优先有线（PID != 接收器 && usage page == 0xFF01），
/// 否则走接收器：用 `A0 00` 读配对表，从里面找出鼠标的 report id。
pub fn open_link(hid: &HidApi) -> Result<Link, String> {
    let report = enumerate_report(hid)?;
    let all = report.interfaces;
    if all.is_empty() {
        return Err(empty_reason(&report.skipped));
    }

    // 1) 有线模式：直接是 0xFF01 且能回答电量帧。
    //
    // report id 必须试两个：实测有线（PID 0x1B63）走 **report id 0**（无编号帧，
    // 回复首字节 00），无线才走 1 / 配对表给的 id。原来只试 1，有线模式一律超时，
    // 于是链路开不起来、界面只能显示上一次的缓存值。
    for iface in &all {
        if iface.product_id == PID_DONGLE || !iface.is_command_channel() {
            continue;
        }
        if iface.usage_page != USAGE_PAGE_MOUSE_CTRL {
            continue;
        }
        let Ok(mut dev) = HidDevice::open(iface) else {
            continue;
        };
        for rid in [1u8, 0u8] {
            if let Some(m) = dev.xfer(rid, 0x12, 0x07, &[0, 0], DEFAULT_TIMEOUT_MS) {
                if !is_nak(&m) && m.len() >= 12 {
                    if std::env::var_os("ROG_HID_DEBUG").is_some() {
                        eprintln!("[hid] 有线命中 report id {rid}：{}", iface.path);
                    }
                    return Ok(Link {
                        mode: "Wired".to_string(),
                        report_id: rid,
                        iface: iface.clone(),
                        all_interfaces: all.clone(),
                    });
                }
            }
        }
    }

    // 2) 无线：先找接收器控制通道（0xFF02），读配对表拿 report id
    let dongle_ctrl = all.iter().find(|x| {
        x.product_id == PID_DONGLE
            && x.usage_page == USAGE_PAGE_DONGLE_CTRL
            && x.is_command_channel()
    });
    let dongle_mouse = all.iter().find(|x| {
        x.product_id == PID_DONGLE
            && x.usage_page == USAGE_PAGE_MOUSE_CTRL
            && x.is_command_channel()
    });

    if dongle_ctrl.is_none() {
        return Err("没有找到接收器控制通道（usage page 0xFF02）".to_string());
    }

    let mut report_id: u8 = 3; // 文档实测的默认值
    if let Some(ctrl) = dongle_ctrl {
        if let Ok(mut dev) = HidDevice::open(ctrl) {
            // A0 00 -> 01 A0 00 count(2) 之后每项 4 字节 {PID(LE16), report_id, notify_id}
            if let Some(scan) = dev.xfer(1, 0xA0, 0x00, &[], 2500) {
                if let Some(rid) =
                    parse_pairing_table(&scan, &[PID_MOUSE_WIRELESS, PID_MOUSE_WIRED])
                {
                    report_id = rid;
                }
            }
        }
    }

    let iface = dongle_mouse
        .or(dongle_ctrl)
        .ok_or("接收器上没有可用的命令通道")?;
    Ok(Link {
        mode: "Wireless24G".to_string(),
        report_id,
        iface: iface.clone(),
        all_interfaces: all.clone(),
    })
}

#[cfg(test)]
mod tests {

    /// 配对表：命中项返回它的 report id，前面的非鼠标项要跳过。
    #[test]
    fn pairing_table_finds_mouse_report_id() {
        // count = 2：第一项是键盘（不匹配），第二项是鼠标
        let scan = [
            0x01, 0xA0, 0x00, 0x00, 0x02, 0x00,
            0x34, 0x12, 0x07, 0x00, // PID 0x1234 report 7
            0x65, 0x1B, 0x03, 0x01, // PID 0x1B65（无线鼠标）report 3
        ];
        assert_eq!(
            parse_pairing_table(&scan, &[super::PID_MOUSE_WIRELESS, super::PID_MOUSE_WIRED]),
            Some(3)
        );
    }

    /// 表里没有我们要的 PID 时不能瞎认一个。
    #[test]
    fn pairing_table_without_match_returns_none() {
        let scan = [0x01, 0xA0, 0x00, 0x00, 0x01, 0x00, 0x34, 0x12, 0x07, 0x00];
        assert_eq!(parse_pairing_table(&scan, &[0x1B65, 0x1B63]), None);
    }

    /// count 说 5 项、实际只回了 1 项：必须停下返回 None，不能读到缓冲区外面。
    #[test]
    fn pairing_table_truncated_returns_none() {
        let scan = [0x01, 0xA0, 0x00, 0x00, 0x05, 0x00, 0x34, 0x12, 0x07, 0x00];
        assert_eq!(parse_pairing_table(&scan, &[0x1B65]), None);
    }

    /// 连 count 字段都没回全。
    #[test]
    fn pairing_table_too_short_returns_none() {
        assert_eq!(parse_pairing_table(&[0x01, 0xA0, 0x00, 0x00, 0x00], &[0x1B65]), None);
        assert_eq!(parse_pairing_table(&[], &[0x1B65]), None);
    }

    /// 有线 PID 也要能命中（同型号有线/无线各一个 PID）。
    #[test]
    fn pairing_table_matches_wired_pid_too() {
        let scan = [0x01, 0xA0, 0x00, 0x00, 0x01, 0x00, 0x63, 0x1B, 0x00, 0x00];
        assert_eq!(parse_pairing_table(&scan, &[0x1B65, 0x1B63]), Some(0));
    }
    use super::*;

    #[test]
    fn nak_frame_is_detected() {
        assert!(is_nak(&[0x03, 0xFF, 0xAA]));
        assert!(!is_nak(&[0x03, 0x12, 0x07]));
        assert!(!is_nak(&[0xFF]));
    }

    #[test]
    fn align_reply_keeps_report_id_frame() {
        let r = [0x03, 0x12, 0x07, 0x00, 0x5F];
        let m = align_reply(0x03, 0x12, 0x07, &r).expect("正常应答应被接受");
        assert_eq!(m, r);
    }

    #[test]
    fn align_reply_prefixes_bare_answer() {
        // 有线模式实测：回复不带 report id
        let r = [0x12, 0x07, 0x00, 0x5F];
        let m = align_reply(0x00, 0x12, 0x07, &r).expect("裸应答应被接受");
        assert_eq!(m, vec![0x00, 0x12, 0x07, 0x00, 0x5F]);
    }

    #[test]
    fn align_reply_accepts_nak_with_report_id() {
        // 回归：NAK 的 r[1] 是 0xFF 而非 cmd，过去会被当「其它帧」丢掉
        let m = align_reply(0x03, 0x12, 0x07, &[0x03, 0xFF, 0xAA]).expect("NAK 必须被接受");
        assert!(is_nak(&m), "接受后仍要能被 is_nak 认出");
        assert_eq!(m[0], 0x03);
    }

    #[test]
    fn align_reply_prefixes_bare_nak() {
        let m = align_reply(0x00, 0x12, 0x07, &[0xFF, 0xAA]).expect("裸 NAK 必须被接受");
        assert_eq!(m, vec![0x00, 0xFF, 0xAA]);
        assert!(is_nak(&m));
    }

    #[test]
    fn align_reply_ignores_unrelated_frame() {
        // 主动推送的电量帧（cmd 0x12 之外）不应被当成应答
        assert_eq!(align_reply(0x03, 0x12, 0x07, &[0x03, 0x99, 0x99, 0x00]), None);
        assert_eq!(align_reply(0x03, 0x12, 0x07, &[]), None);
    }

    #[test]
    fn empty_reason_prefers_the_open_failure() {
        // 什么接口都没枚举到 → 真·没插设备
        let none: Vec<Skipped> = Vec::new();
        assert!(empty_reason(&none).contains("没有找到任何 ASUS HID 接口"));

        // 枚举到了但打不开 → 必须把原因带出来，不能报「找不到设备」
        let skipped = vec![
            Skipped {
                path: "hid#vid_0b05#pid_1b63#1".into(),
                reason: "CreateFileW 失败（GetLastError=5 拒绝访问）".into(),
            },
            Skipped {
                path: "hid#vid_0b05#pid_1ace#2".into(),
                reason: "CreateFileW 失败（GetLastError=32 另一个程序正在使用）".into(),
            },
        ];
        let msg = empty_reason(&skipped);
        assert!(msg.contains("2 个 HID 接口"), "{msg}");
        assert!(msg.contains("Armoury Crate"), "{msg}");
        assert!(msg.contains("拒绝访问"), "第一条原因要原样带出来：{msg}");
    }
}
