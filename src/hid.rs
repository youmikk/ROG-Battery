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
    pub index: usize,
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
/// 枚举本机所有 ASUS HID 接口。
pub fn enumerate(hid: &HidApi) -> Result<Vec<HidInterface>, String> {
    let mut list = Vec::new();

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
                Ok(mut iface) => {
                    if iface.vendor_id != VID_ASUS {
                        continue;
                    }
                    iface.index = list.len();
                    list.push(iface);
                }
                // 单个接口打不开（权限 / 被占用）不算致命，跳过
                Err(_) => continue,
            }
        }
        Ok(())
    })();

    unsafe { SetupDiDestroyDeviceInfoList(dev_info) };
    result?;
    Ok(list)
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
            index: 0,
        })
    })();

    unsafe { CloseHandle(h) };
    out
}

/// 读 HID 字符串（0=产品名，1=序列号）。失败返回空串，不报错。
///
/// ## 为什么尾部要清洗
///
/// 本机的 OMNI 接收器（`ROG OMNI RECEIVER`）HID 字符串描述符有固件 bug：
/// 序列号前 12 字符是正常的 `TCMPKRD0000L`，但 `bLength` 多声明了 2 个字符，
/// Windows 照 `bLength` 读出来后面就跟着**固件 RAM 的残留数据**。
///
/// 实测（2026-10-21，`info` 连跑 10 次，同一接口）：
/// ```text
/// 第 1 次  U+0004 U+002B
/// 第 2 次  U+0044 U+002B
/// 第 3 次  U+0084 U+002B
/// 第 4 次  U+00C4 U+002B
/// 第 5 次  U+0004 U+002B      ← 回到起点
/// ...（4 个值按 0x40 步长循环，第二字符恒为 0x2B）
/// ```
/// 第一个字符是某个指针的低字节（步长 64 = 分配粒度），第二字符固定。
/// 而**协议字段**（`A1 00` 的前 12 字符）永远是干净的 —— 所以协议字段才是权威值。
///
/// 另外 12 个 HID 接口里 8 个报 `U+000C U+0028`、4 个报 `U+0027 U+002A`，
/// 同一物理设备不同接口给不同垃圾值，进一步确认是越界读而非真实数据。
///
/// 清洗策略：**只保留开头的连续 ASCII 字母数字串**。
///
/// 试过但**不行**的方案（记下来免得再走一遍）：`trim_end_matches(|c| !c.is_ascii_graphic())`
/// —— 垃圾字节的第二字符实测可能是 `+`(0x2B)、`*`(0x2A)，都是可打印 ASCII，
/// 从尾部 trim 一个字符都去不掉（其中一种取值是 `U+0044 U+002B` 即 `"D+"`，全可打印）。
///
/// 判据依据：ASUS 序列号是 **12 位字母数字**。三重印证 ——
/// 协议字段 `A1 00` 给出 `TCMPKRD0000L`（干净 12 位）、鼠标 SN `B00000000000`、
/// ASUS 自己的 `UnifyDGDevice.xml` / `deviceinfo.ini` 也只记 12 位。
///
/// 代价：若将来遇到序列号里带 `-` 或空格的设备会被截断。可接受 ——
/// **权威值来自协议字段，HID 字符串只用于显示**（见 `proto.rs::print_identity`）。
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
                0,
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

        Ok(HidDevice {
            write_handle,
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
    /// 最多 8 轮、每轮 15ms；正常情况下第一轮就抽空。
    fn drain(&mut self) {
        for _ in 0..8 {
            if !self.start_read(4096) {
                break;
            }
            if self.finish_read(15).is_none() {
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

        let mut written: u32 = 0;
        let ok = unsafe {
            WriteFile(
                self.write_handle,
                pkt.as_ptr() as *const c_void,
                pkt.len() as u32,
                &mut written,
                core::ptr::null_mut(),
            )
        };
        if ok == 0 {
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

            // 应答自带 report id
            if r.len() >= 3 && r[0] == report_id && r[1] == cmd && r[2] == sub {
                return Some(r);
            }
            // 应答从 cmd 字节开始，需要补一个 report id 到下标 0
            if r.len() >= 2 && r[0] == cmd && r[1] == sub {
                let mut m = Vec::with_capacity(r.len() + 1);
                m.push(report_id);
                m.extend_from_slice(&r);
                return Some(m);
            }
            // 其它帧（例如主动推送的电量帧）丢掉，继续等
        }
        None
    }
}

impl Drop for HidDevice {
    fn drop(&mut self) {
        unsafe {
            if !self.read_event.is_null() && self.read_event != INVALID_HANDLE_VALUE {
                CloseHandle(self.read_event);
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

/// 找命令通道并确定 report id。
///
/// 优先有线（PID != 接收器 && usage page == 0xFF01），
/// 否则走接收器：用 `A0 00` 读配对表，从里面找出鼠标的 report id。
pub fn open_link(hid: &HidApi) -> Result<Link, String> {
    let all = enumerate(hid)?;
    if all.is_empty() {
        return Err("没有找到任何 ASUS HID 接口（VID 0x0B05）".to_string());
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
                if scan.len() > 5 {
                    let count = scan[4] as usize | ((scan[5] as usize) << 8);
                    let mut at = 6;
                    for _ in 0..count {
                        if at + 4 > scan.len() {
                            break;
                        }
                        let pid = scan[at] as u16 | ((scan[at + 1] as u16) << 8);
                        if pid == PID_MOUSE_WIRELESS || pid == PID_MOUSE_WIRED {
                            report_id = scan[at + 2];
                            break;
                        }
                        at += 4;
                    }
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
