//! HID 设备枚举、打开与收发。
//!
//! - 用 setupapi 枚举 HID 接口，取 `\\?\hid#...` 路径；
//! - `CreateFile` 打开时**必须** `FILE_SHARE_READ | FILE_SHARE_WRITE`（Armoury Crate 可能同时开着）；
//! - 读写使用 overlapped 句柄、独立事件及超时；部分协议使用控制端点 Output Report；
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
    pub feature_len: u16,
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
#[derive(Clone)]
pub struct EnumerateReport {
    pub interfaces: Vec<HidInterface>,
    pub skipped: Vec<Skipped>,
}

pub struct GroupedEnumerateReport {
    pub report: EnumerateReport,
    pub physical_groups: std::collections::BTreeMap<String, String>,
}

pub fn enumerate(hid: &HidApi) -> Result<Vec<HidInterface>, String> {
    Ok(enumerate_report(hid)?.interfaces)
}

pub fn enumerate_report(hid: &HidApi) -> Result<EnumerateReport, String> {
    let mut report = enumerate_all_report(hid)?;
    report.interfaces.retain(|iface| iface.vendor_id == VID_ASUS);
    Ok(report)
}

/// 只读描述符枚举，品牌协议决定后续可用的控制接口。
pub fn enumerate_all_report(hid: &HidApi) -> Result<EnumerateReport, String> {
    enumerate_all_report_with(hid, |_, _| {})
}

pub fn enumerate_all_grouped_report(hid: &HidApi) -> Result<GroupedEnumerateReport, String> {
    let mut instances = std::collections::BTreeMap::new();
    let report = enumerate_all_report_with(hid, |path, instance| {
        instances.insert(path.to_ascii_lowercase(), instance);
    })?;
    Ok(group_report_with(report, &instances, |instance, vendor_id, product_id| {
        usb_physical_group_with(instance, vendor_id, product_id, |node| {
            let identity = device_instance_id(node)?;
            usb_ancestor_matches(&identity, vendor_id, product_id).then_some(identity)
        }, parent_instance)
    }))
}

fn group_report_with(
    report: EnumerateReport,
    instances: &std::collections::BTreeMap<String, u32>,
    mut physical_group: impl FnMut(u32, u16, u16) -> Option<String>,
) -> GroupedEnumerateReport {
    let mut physical_groups = std::collections::BTreeMap::new();
    let paths = report.interfaces.iter().map(|interface|
        (&interface.path, (Some(interface.vendor_id), Some(interface.product_id))))
        .chain(report.skipped.iter().map(|skipped| (&skipped.path, path_ids(&skipped.path))));
    for (path, (vendor_id, product_id)) in paths {
        let path = path.to_ascii_lowercase();
        let Some(&instance) = instances.get(&path) else { continue; };
        let (Some(vendor_id), Some(product_id)) = (vendor_id, product_id) else { continue; };
        if let Some(group) = physical_group(instance, vendor_id, product_id).filter(|group| !group.is_empty()) {
            physical_groups.insert(path, group.to_ascii_lowercase());
        }
    }
    GroupedEnumerateReport { report, physical_groups }
}

pub(crate) fn path_ids(value: &str) -> (Option<u16>, Option<u16>) {
    let lower = value.to_ascii_lowercase();
    let Some(remainder) = lower.strip_prefix(r"\\?\hid#") else { return (None, None); };
    let Some((hardware, _)) = remainder.split_once('#') else { return (None, None); };
    let field = |prefix: &str| {
        let mut fields = hardware.split('&').filter_map(|field| field.strip_prefix(prefix));
        let value = fields.next()?;
        if fields.next().is_some() || value.len() != 4 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        u16::from_str_radix(value, 16).ok()
    };
    (field("vid_"), field("pid_"))
}

fn enumerate_all_report_with(
    hid: &HidApi,
    mut devnode: impl FnMut(&str, u32),
) -> Result<EnumerateReport, String> {
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

    let result = enumerate_interfaces_with(
        |index| {
            let mut did = SpDeviceInterfaceData::default();
            let ok = unsafe {
                SetupDiEnumDeviceInterfaces(dev_info, core::ptr::null_mut(), &guid, index, &mut did)
            };
            if ok == 0 {
                Err(last_error())
            } else {
                Ok(did)
            }
        },
        |interface| {
            Ok(interface_path(dev_info, interface)?.map(|(path, instance)| {
                devnode(&path, instance);
                path
            }))
        },
        |path| probe_interface(hid, path),
    );

    unsafe { SetupDiDestroyDeviceInfoList(dev_info) };
    result
}

fn enumerate_interfaces_with(
    mut next: impl FnMut(u32) -> Result<SpDeviceInterfaceData, u32>,
    mut path: impl FnMut(&mut SpDeviceInterfaceData) -> Result<Option<String>, String>,
    mut probe: impl FnMut(&str) -> Result<HidInterface, String>,
) -> Result<EnumerateReport, String> {
    let mut list = Vec::new();
    let mut skipped = Vec::new();
    let mut index = 0;
    loop {
        let mut interface = match next(index) {
            Ok(interface) => interface,
            Err(error) if error == ERROR_NO_MORE_ITEMS => break,
            Err(error) => return Err(format!(
                "SetupDiEnumDeviceInterfaces 失败（GetLastError={error}）"
            )),
        };
        index += 1;
        let Some(path) = path(&mut interface)? else { continue; };
        match probe(&path) {
            Ok(interface) => list.push(interface),
            Err(reason) => skipped.push(Skipped { path, reason }),
        }
    }
    Ok(EnumerateReport {
        interfaces: list,
        skipped,
    })
}

/// `SetupDiGetDeviceInterfaceDetailW` 需要两次调用：先问长度，再取内容。
fn interface_path(
    dev_info: Handle,
    did: &mut SpDeviceInterfaceData,
) -> Result<Option<(String, u32)>, String> {
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
    if required < 6 {
        return Err(format!("SetupDiGetDeviceInterfaceDetailW 长度查询失败（GetLastError={}）", last_error()));
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

    let mut device_info = SpDevInfoData::default();
    let ok = unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            dev_info,
            did,
            buf.as_mut_ptr() as *mut c_void,
            required,
            core::ptr::null_mut(),
            (&mut device_info as *mut SpDevInfoData).cast(),
        )
    };
    if ok == 0 {
        return Err(format!("SetupDiGetDeviceInterfaceDetailW 失败（GetLastError={}）", last_error()));
    }

    let wchars: Vec<u16> = buf[4..]
        .chunks_exact(2)
        .map(|c| u16::from_ne_bytes([c[0], c[1]]))
        .collect();
    let end = wchars.iter().position(|&c| c == 0).unwrap_or(wchars.len());
    Ok(Some((String::from_utf16_lossy(&wchars[..end]), device_info.dev_inst)))
}

fn usb_physical_group(instance: u32, interface: &HidInterface) -> Option<String> {
    usb_physical_group_with(instance, interface.vendor_id, interface.product_id,
        device_instance_id, parent_instance)
}

fn device_instance_id(instance: u32) -> Option<String> {
    use winapi::um::cfgmgr32::{CM_Get_Device_IDW, CR_SUCCESS, MAX_DEVICE_ID_LEN};
    let mut buffer = [0u16; MAX_DEVICE_ID_LEN];
    if unsafe { CM_Get_Device_IDW(instance, buffer.as_mut_ptr(), buffer.len() as u32, 0) } != CR_SUCCESS {
        return None;
    }
    let end = buffer.iter().position(|&unit| unit == 0)?;
    String::from_utf16(&buffer[..end]).ok()
}

fn parent_instance(instance: u32) -> Option<u32> {
    use winapi::um::cfgmgr32::{CM_Get_Parent, CR_SUCCESS};
    let mut parent = 0;
    (unsafe { CM_Get_Parent(&mut parent, instance, 0) } == CR_SUCCESS).then_some(parent)
}

fn usb_ancestor_matches(identity: &str, vendor_id: u16, product_id: u16) -> bool {
    let identity = identity.to_ascii_lowercase();
    let parts: Vec<_> = identity.split('\\').collect();
    if parts.len() != 3 || parts[2].is_empty() || identity.contains('\0') { return false; }
    let expected = format!("vid_{vendor_id:04x}&pid_{product_id:04x}");
    match parts[0] {
        "hid" => true,
        "usb" if parts[1] == expected => true,
        "usb" => parts[1].strip_prefix(&format!("{expected}&mi_"))
            .is_some_and(|suffix| suffix.len() == 2 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())),
        _ => false,
    }
}

fn usb_physical_group_with(
    mut instance: u32,
    vendor_id: u16,
    product_id: u16,
    mut device_id: impl FnMut(u32) -> Option<String>,
    mut parent: impl FnMut(u32) -> Option<u32>,
) -> Option<String> {
    let expected = format!("vid_{vendor_id:04x}&pid_{product_id:04x}");
    let mut visited = std::collections::BTreeSet::new();
    for _ in 0..32 {
        if !visited.insert(instance) { return None; }
        let identity = device_id(instance)?.to_ascii_lowercase();
        let parts: Vec<_> = identity.split('\\').collect();
        if parts.len() == 3 && parts[0] == "usb" && parts[1] == expected && !parts[2].is_empty() {
            return Some(identity);
        }
        instance = parent(instance)?;
    }
    None
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
            feature_len: caps.feature_report_byte_length,
            product: read_hid_string(hid, h, 0),
            serial: read_hid_string(hid, h, 1),
        })
    })();

    unsafe { CloseHandle(h) };
    out
}

fn read_hid_string(hid: &HidApi, h: Handle, which: u8) -> String {
    read_hid_string_with(which, |which, buf, cap| unsafe {
        if which == 0 {
            (hid.get_product_string)(h, buf.as_mut_ptr(), cap)
        } else {
            (hid.get_serial_number_string)(h, buf.as_mut_ptr(), cap)
        }
    })
}

fn read_hid_string_with(which: u8, read: impl FnOnce(u8, &mut [u16], u32) -> u8) -> String {
    let mut buf = vec![0u16; 256];
    let cap = (buf.len() * 2) as u32;
    let ok = read(which, &mut buf, cap);
    if ok == 0 {
        return String::new();
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end]).trim().to_string()
}

// ---------------- 打开的设备 ----------------
/// 查询描述符公开的 Feature/Input report，使用零访问权限与可取消的 I/O。
/// 不需要取得系统键盘/鼠标的输入流访问权限，也不会写 Output report。
pub fn query_report(iface: &HidInterface, report_id: u8, feature: bool, timeout_ms: u32) -> Option<Vec<u8>> {
    let length = if feature { iface.feature_len } else { iface.input_len } as usize;
    if length == 0 { return None; }
    let path = to_wide(&iface.path);
    let handle = unsafe { CreateFileW(path.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE,
        core::ptr::null_mut(), OPEN_EXISTING, FILE_FLAG_OVERLAPPED, core::ptr::null_mut()) };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE { return None; }
    let event = unsafe { CreateEventW(core::ptr::null_mut(), 0, 0, core::ptr::null()) };
    if event.is_null() || event == INVALID_HANDLE_VALUE { unsafe { CloseHandle(handle); } return None; }
    let result = query_report_captured(iface, report_id, feature, |report| {
        query_report_transfer(handle, event, report, feature, timeout_ms)
    });
    unsafe { CloseHandle(event); CloseHandle(handle); }
    result
}

pub(crate) fn query_report_with_handles(handle: Handle, event: Handle, length: usize,
    report_id: u8, feature: bool, timeout_ms: u32) -> Option<Vec<u8>> {
    if length == 0 || handle.is_null() || handle == INVALID_HANDLE_VALUE
        || event.is_null() || event == INVALID_HANDLE_VALUE { return None; }
    query_report_with(length, report_id, |report| {
        query_report_transfer(handle, event, report, feature, timeout_ms)
    }, |_, _| {})
}

fn query_report_captured(iface: &HidInterface, report_id: u8, feature: bool,
    transfer: impl FnOnce(&mut [u8]) -> Option<usize>) -> Option<Vec<u8>> {
    let length = usize::from(if feature { iface.feature_len } else { iface.input_len });
    query_report_with(length, report_id, transfer, |report, returned| {
        capture_control_result(&iface.path, if feature { "get-feature" } else { "get-input" },
            report, returned, report_id == 0);
    })
}

fn query_report_with(length: usize, report_id: u8,
    transfer: impl FnOnce(&mut [u8]) -> Option<usize>,
    observe: impl FnOnce(&[u8], Option<usize>)) -> Option<Vec<u8>> {
    if length == 0 { return None; }
    let mut report = vec![0u8; length];
    report[0] = report_id;
    let result = transfer(&mut report);
    observe(&report, result);
    let returned = result?;
    if returned == 0 || returned > length || report[0] != report_id { return None; }
    let actual = returned.saturating_add(usize::from(report_id == 0)).min(length);
    report.truncate(actual);
    Some(report)
}

pub(crate) fn capture_control_result(path: &str, api: &'static str, report: &[u8],
    returned: Option<usize>, unnumbered: bool) {
    let actual = returned.filter(|count| *count > 0)
        .map_or(0, |count| count.saturating_add(usize::from(unnumbered)).min(report.len()));
    crate::capture::record(path, api, &report[..actual], returned.is_some());
}

fn query_report_transfer(handle: Handle, event: Handle, report: &mut [u8],
    feature: bool, timeout_ms: u32) -> Option<usize> {
    use winapi::shared::hidclass::{IOCTL_HID_GET_FEATURE, IOCTL_HID_GET_INPUT_REPORT};
    let mut overlapped = Overlapped { h_event: event, ..Default::default() };
    let mut returned = 0;
    let ok = unsafe { DeviceIoControl(handle,
        if feature { IOCTL_HID_GET_FEATURE } else { IOCTL_HID_GET_INPUT_REPORT },
        report.as_mut_ptr().cast(), report.len() as u32, report.as_mut_ptr().cast(), report.len() as u32,
        &mut returned, &mut overlapped) };
    if ok == 0 {
        if last_error() != ERROR_IO_PENDING { return None; }
        if unsafe { WaitForSingleObject(event, timeout_ms) } != WAIT_OBJECT_0 {
            // 缓冲区和 OVERLAPPED 必须活到取消完成，不能在内核仍持有时释放。
            unsafe { CancelIoEx(handle, &mut overlapped); GetOverlappedResult(handle, &mut overlapped, &mut returned, 1); }
            return None;
        }
        if unsafe { GetOverlappedResult(handle, &mut overlapped, &mut returned, 0) } == 0 { return None; }
    }
    Some(returned as usize)
}

/// 一个已打开的命令通道。读写分别使用可取消的 overlapped 句柄。
pub struct HidDevice {
    capture_path: String,
    /// 写句柄，用于 WriteFile 或 HID Output Report 控制传输。
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
    /// 原样收发品牌协议帧；匹配必须包含设备索引、功能及事务标识。
    pub fn transact(
        &mut self,
        packet: &[u8],
        timeout_ms: u32,
        matches: impl FnMut(&[u8]) -> bool,
    ) -> Option<Vec<u8>> {
        self.transact_output(packet, timeout_ms, false, None, matches)
    }

    /// 部分接收器只接受控制端点 SET_REPORT，不能使用 WriteFile 的中断 OUT。
    /// 不在超时后自动重发到另一端点，避免重复执行已送达的设置命令。
    pub fn transact_control_output(
        &mut self,
        packet: &[u8],
        timeout_ms: u32,
        matches: impl FnMut(&[u8]) -> bool,
    ) -> Option<Vec<u8>> {
        self.transact_output(packet, timeout_ms, true, None, matches)
    }

    /// Interrupt Output exchange for a collection with one descriptor-proven
    /// input report ID. ReadFile needs the full Windows capacity, including the
    /// zero ID slot; unnumbered reports are normalized after receiving them.
    pub fn transact_with_input_report(
        &mut self,
        packet: &[u8],
        input_report_id: u8,
        timeout_ms: u32,
        matches: impl FnMut(&[u8]) -> bool,
    ) -> Option<Vec<u8>> {
        self.transact_output(
            packet,
            timeout_ms,
            false,
            Some(input_report_id),
            matches,
        )
    }

    fn transact_output(
        &mut self,
        packet: &[u8],
        timeout_ms: u32,
        control: bool,
        input_report_id: Option<u8>,
        mut matches: impl FnMut(&[u8]) -> bool,
    ) -> Option<Vec<u8>> {
        if !self.writable || packet.is_empty() || packet.len() > self.out_len as usize {
            return None;
        }
        self.drain();
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(u64::from(timeout_ms));
        let mut padded = vec![0; self.out_len as usize];
        padded[..packet.len()].copy_from_slice(packet);
        self.write_ov = Overlapped { h_event: self.write_event, ..Default::default() };
        let mut written = 0;
        let ok = unsafe {
            if control {
                DeviceIoControl(self.write_handle, winapi::shared::hidclass::IOCTL_HID_SET_OUTPUT_REPORT,
                    padded.as_mut_ptr().cast(), padded.len() as u32,
                    core::ptr::null_mut(), 0, &mut written, &mut self.write_ov)
            } else {
                WriteFile(self.write_handle, padded.as_ptr().cast(), padded.len() as u32,
                    &mut written, &mut self.write_ov)
            }
        };
        // SET_OUTPUT_REPORT 成功可以返回零字节；WriteFile 仍要求实际写入。
        let success = self.finish_output(ok, written, timeout_ms, control);
        crate::capture::record(&self.capture_path, if control { "set-output" } else { "write" }, &padded, success);
        if !success {
            return None;
        }
        while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
            let read_len = self.in_len as usize;
            if read_len == 0 || !self.start_read(read_len) { return None; }
            let raw = self.finish_read(left.as_millis().max(1) as u32)?;
            let Some(frame) = protocol_input(raw, input_report_id, read_len) else { continue; };
            if matches(&frame) { return Some(frame); }
        }
        None
    }

    /// 标准 Input report 的被动读取，不向键盘或鼠标发送私有命令。
    pub fn read_report(&mut self, timeout_ms: u32) -> Option<Vec<u8>> {
        if !self.start_read(self.in_len as usize) { return None; }
        self.finish_read(timeout_ms)
    }

    /// 打开指定路径的命令通道。写句柄开不了时自动降级为只读（`writable=false`）。
    pub fn open(iface: &HidInterface) -> Result<HidDevice, String> {
        Self::open_access(iface, true)
    }

    /// 被动状态通道不申请写权限，也不创建输出句柄。
    pub fn open_read_only(iface: &HidInterface) -> Result<HidDevice, String> {
        Self::open_access(iface, false)
    }

    fn open_access(iface: &HidInterface, allow_write: bool) -> Result<HidDevice, String> {
        let wide = to_wide(&iface.path);

        let mut writable = allow_write;
        let mut write_handle = if allow_write { unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                core::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                core::ptr::null_mut(),
            )
        } } else { core::ptr::null_mut() };
        if allow_write && (write_handle == INVALID_HANDLE_VALUE || write_handle.is_null()) {
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
        if allow_write && (write_handle == INVALID_HANDLE_VALUE || write_handle.is_null()) {
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
            if !write_handle.is_null() { unsafe { CloseHandle(write_handle) }; }
            return Err(format!(
                "无法打开 overlapped 读句柄（GetLastError={}）",
                last_error()
            ));
        }

        let read_event = unsafe { CreateEventW(core::ptr::null_mut(), 0, 0, core::ptr::null()) };
        if read_event.is_null() || read_event == INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(read_handle);
                if !write_handle.is_null() { CloseHandle(write_handle); }
            }
            return Err(format!(
                "CreateEventW 失败（GetLastError={}）",
                last_error()
            ));
        }

        let write_event = if allow_write {
            unsafe { CreateEventW(core::ptr::null_mut(), 0, 0, core::ptr::null()) }
        } else { core::ptr::null_mut() };
        if allow_write && (write_event.is_null() || write_event == INVALID_HANDLE_VALUE) {
            unsafe {
                CloseHandle(read_event);
                CloseHandle(read_handle);
                if !write_handle.is_null() { CloseHandle(write_handle); }
            }
            return Err(format!(
                "CreateEventW 失败（GetLastError={}）",
                last_error()
            ));
        }

        Ok(HidDevice {
            capture_path: iface.path.clone(),
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
        self.finish_read_checked(timeout_ms).ok().flatten()
    }

    fn finish_read_checked(&mut self, timeout_ms: u32) -> Result<Option<Vec<u8>>, ()> {
        let result = self.finish_read_result(timeout_ms);
        capture_input_result(&self.capture_path, timeout_ms, &result);
        result
    }

    fn finish_read_result(&mut self, timeout_ms: u32) -> Result<Option<Vec<u8>>, ()> {
        if self.read_completed_sync {
            let n = self.sync_read_len as usize;
            if n == 0 || n > self.read_buf.len() {
                return Err(());
            }
            return Ok(Some(self.read_buf[..n].to_vec()));
        }

        let wait = unsafe { WaitForSingleObject(self.read_event, timeout_ms) };
        if wait != WAIT_OBJECT_0 {
            let mut xfer: u32 = 0;
            let completed = unsafe {
                CancelIoEx(self.read_handle, &mut self.read_ov);
                GetOverlappedResult(self.read_handle, &mut self.read_ov, &mut xfer, 1)
            };
            // 若报告在超时临界点已由内核完成且未被取消，保留该完整报文而不是丢弃。
            if wait == WAIT_TIMEOUT && completed != 0 && xfer > 0 && xfer as usize <= self.read_buf.len() {
                return Ok(Some(self.read_buf[..xfer as usize].to_vec()));
            }
            return if wait == WAIT_TIMEOUT { Ok(None) } else { Err(()) };
        }
        let mut xfer: u32 = 0;
        let ok = unsafe { GetOverlappedResult(self.read_handle, &mut self.read_ov, &mut xfer, 0) };
        if ok == 0 || xfer == 0 || xfer as usize > self.read_buf.len() {
            return Err(());
        }
        Ok(Some(self.read_buf[..xfer as usize].to_vec()))
    }

    /// 收尾一次写：同步完成直接成功；挂起则等到 `timeout_ms`，超时取消并判失败。
    ///
    /// 写句柄是 overlapped 打开的，所以驱动卡住时能把控制权抢回来——以前用同步
    /// 句柄写，`WriteFile` 没有超时，设备一挂就把整个刷新线程钉死。
    fn finish_write(&mut self, ok: i32, written: u32, timeout_ms: u32) -> bool {
        self.finish_output(ok, written, timeout_ms, false)
    }

    fn finish_output(&mut self, ok: i32, written: u32, timeout_ms: u32, control: bool) -> bool {
        if ok != 0 {
            return control || written > 0;
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
        done != 0 && (control || xfer > 0)
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
        self.xfer_checked(report_id, cmd, sub, tail, timeout_ms).ok().flatten()
    }

    fn xfer_checked(
        &mut self,
        report_id: u8,
        cmd: u8,
        sub: u8,
        tail: &[u8],
        timeout_ms: u32,
    ) -> Result<Option<Vec<u8>>, ()> {
        if !self.writable {
            return Err(());
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
        let success = self.finish_write(ok, written, timeout_ms);
        crate::capture::record(&self.capture_path, "write", &pkt, success);
        if !success {
            return Err(());
        }

        let want = self.in_len.max(1) as usize;
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms as u64);
        loop {
            let now = std::time::Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            let left = (deadline - now).as_millis().max(1) as u32;
            if !self.start_read(want) {
                return Err(());
            }
            let Some(r) = self.finish_read_checked(left)? else {
                continue;
            };

            if let Some(m) = align_reply(report_id, cmd, sub, &r) {
                return Ok(Some(m));
            }
            // 其它帧（例如主动推送的电量帧）丢掉，继续等
        }
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

fn capture_input_result(path: &str, timeout_ms: u32, result: &Result<Option<Vec<u8>>, ()>) {
    match result {
        Ok(Some(frame)) => crate::capture::record(path, "input", frame, true),
        Err(()) => crate::capture::record(path, "input", &[], false),
        Ok(None) if timeout_ms > 0 => crate::capture::record(path, "input-timeout", &[], false),
        Ok(None) => (),
    }
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
#[derive(Clone)]
pub struct Link {
    /// 人类可读的模式：Wired / Wireless24G
    pub mode: String,
    pub report_id: u8,
    pub read_only: bool,
    pub iface: HidInterface,
    /// 同一物理设备的接口；无法确认分组时仅保留当前接口。
    pub all_interfaces: Vec<HidInterface>,
}

pub struct LinkScan {
    pub links: Vec<Link>,
    pub present_channels: std::collections::BTreeSet<String>,
}

impl Link {
    pub fn device_key(&self) -> String {
        crate::device::stable_key(&format!("{}:{}", self.iface.path.to_ascii_lowercase(), self.report_id))
    }
}

fn parse_pairing_table(scan: &[u8], wanted: &[u16]) -> Option<u8> {
    if scan.len() < 5 || !scan.starts_with(&[1, 0xA0, 0]) { return None; }
    let count = u16::from_le_bytes([scan[3], scan[4]]) as usize;
    let entries = scan.get(5..5 + count * 4)?;
    let mut report_id = None;
    for entry in entries.chunks_exact(4) {
        let pid = u16::from_le_bytes([entry[0], entry[1]]);
        if wanted.contains(&pid) {
            if report_id.is_some() || entry[2] == 0 { return None; }
            report_id = Some(entry[2]);
        }
    }
    report_id
}

/// 可能是命令通道的 usage page。
///
/// 华硕的私有页集中在 `0xFF00..=0xFF02`：鼠标/键盘/耳机是 `0xFF01`，接收器是
/// `0xFF02`，老一点的型号用 `0xFF00`。只在这三页上试探，不碰标准 HID 页。
fn is_command_page(page: u16) -> bool {
    (0xFF00..=0xFF02).contains(&page)
}

/// 找命令通道并确定 report id。
///
/// 优先有线（PID != 接收器 && usage page == 0xFF01），
/// 否则走接收器：用 `A0 00` 读配对表，从里面找出鼠标的 report id。
pub fn open_links(hid: &HidApi) -> Result<Vec<Link>, String> {
    Ok(scan_links(hid)?.links)
}

pub fn scan_links(hid: &HidApi) -> Result<LinkScan, String> {
    scan_links_with_discovery(hid, |_, _| {})
}

pub fn scan_links_with_discovery(
    hid: &HidApi,
    discovered: impl FnMut(&EnumerateReport, &std::collections::BTreeMap<String, String>),
) -> Result<LinkScan, String> {
    let mut instances = std::collections::BTreeMap::new();
    let mut report = enumerate_all_report_with(hid, |path, instance| {
        instances.insert(path.to_string(), instance);
    })?;
    report.interfaces.retain(|interface| interface.vendor_id == VID_ASUS);
    scan_links_observed_with(
        report,
        |interface| usb_physical_group(*instances.get(&interface.path)?, interface),
        |iface| {
            let mut device = HidDevice::open(iface).ok()?;
            [1, 0].into_iter().find(|&report_id| {
                device.xfer(report_id, 0x12, 0x07, &[0, 0], DEFAULT_TIMEOUT_MS)
                    .is_some_and(|reply| !is_nak(&reply) && reply.len() >= 12)
            })
        },
        |iface| {
            let mut device = HidDevice::open(iface).map_err(|_| ())?;
            device.xfer_checked(1, 0xA0, 0x00, &[], 2500)
        },
        discovered,
    )
}

#[cfg(test)]
fn scan_links_with(
    report: EnumerateReport,
    physical_group: impl FnMut(&HidInterface) -> Option<String>,
    wired_report_id: impl FnMut(&HidInterface) -> Option<u8>,
    pairing_table: impl FnMut(&HidInterface) -> Result<Option<Vec<u8>>, ()>,
) -> Result<LinkScan, String> {
    scan_links_observed_with(report, physical_group, wired_report_id, pairing_table, |_, _| {})
}

fn scan_links_observed_with(
    report: EnumerateReport,
    mut physical_group: impl FnMut(&HidInterface) -> Option<String>,
    wired_report_id: impl FnMut(&HidInterface) -> Option<u8>,
    pairing_table: impl FnMut(&HidInterface) -> Result<Option<Vec<u8>>, ()>,
    mut discovered: impl FnMut(&EnumerateReport, &std::collections::BTreeMap<String, String>),
) -> Result<LinkScan, String> {
    let groups = report.interfaces.iter().filter_map(|interface|
        physical_group(interface).filter(|group| !group.is_empty())
            .map(|group| (interface.path.to_ascii_lowercase(), group.to_ascii_lowercase())))
        .collect::<std::collections::BTreeMap<_, _>>();
    discovered(&report, &groups);
    let present_channels = report.interfaces.iter().map(|interface| interface.path.to_ascii_lowercase())
        .chain(report.skipped.iter().map(|skipped| skipped.path.to_ascii_lowercase())).collect();
    let links = discover_links_with(report,
        |interface| groups.get(&interface.path.to_ascii_lowercase()).cloned(), wired_report_id, pairing_table)?;
    Ok(LinkScan { links, present_channels })
}

#[cfg(test)]
fn open_links_with(
    report: EnumerateReport,
    physical_group: impl FnMut(&HidInterface) -> Option<String>,
    wired_report_id: impl FnMut(&HidInterface) -> Option<u8>,
    mut pairing_table: impl FnMut(&HidInterface) -> Option<Vec<u8>>,
) -> Result<Vec<Link>, String> {
    discover_links_with(report, physical_group, wired_report_id, |interface| Ok(pairing_table(interface)))
}

fn discover_links_with(
    report: EnumerateReport,
    mut physical_group: impl FnMut(&HidInterface) -> Option<String>,
    mut wired_report_id: impl FnMut(&HidInterface) -> Option<u8>,
    mut pairing_table: impl FnMut(&HidInterface) -> Result<Option<Vec<u8>>, ()>,
) -> Result<Vec<Link>, String> {
    let all = report.interfaces;
    if all.is_empty() { return Ok(Vec::new()); }

    let mut groups = std::collections::BTreeMap::<(u16, u16, bool, String), Vec<HidInterface>>::new();
    for iface in &all {
        let physical = physical_group(iface).filter(|group| !group.is_empty());
        let trusted = physical.is_some();
        let group = physical.unwrap_or_else(|| iface.path.clone()).to_ascii_lowercase();
        groups.entry((iface.vendor_id, iface.product_id, trusted, group))
            .or_default().push(iface.clone());
    }
    let mut links = Vec::new();
    for interfaces in groups.values() {
        for iface in interfaces {
            if iface.product_id == PID_DONGLE || !iface.is_command_channel()
                || !is_command_page(iface.usage_page) { continue; }
            if let Some(report_id) = wired_report_id(iface) {
                links.push(Link {
                    mode: "Wired".into(), report_id, read_only: false, iface: iface.clone(),
                    all_interfaces: interfaces.clone(),
                });
            }
        }
    }
    for (group, interfaces) in &groups {
        let controls: Vec<_> = interfaces.iter().filter(|iface| iface.product_id == PID_DONGLE
            && iface.usage_page == USAGE_PAGE_DONGLE_CTRL && iface.is_command_channel()).collect();
        let mice: Vec<_> = interfaces.iter().filter(|iface| iface.product_id == PID_DONGLE
            && iface.usage_page == USAGE_PAGE_MOUSE_CTRL && iface.is_command_channel()).collect();
        let [control] = controls.as_slice() else { continue; };
        let iface = match mice.as_slice() {
            [] => *control,
            [mouse] => *mouse,
            _ => continue,
        };
        let (report_id, read_only) = match pairing_table(control) {
            Ok(Some(scan)) => {
                let Some(report_id) = parse_pairing_table(&scan, &[PID_MOUSE_WIRELESS, PID_MOUSE_WIRED])
                    else { continue; };
                (report_id, false)
            }
            Ok(None) if group.2 && control.vendor_id == VID_ASUS
                && control.product_id == PID_DONGLE && mice.len() == 1 => (3, true),
            _ => continue,
        };
        links.push(Link {
            mode: "Wireless24G".into(), report_id, read_only, iface: iface.clone(),
            all_interfaces: interfaces.clone(),
        });
    }
    Ok(links)
}

/// 只取第一条链路。命令行走的是这条 —— 一次只操作一台设备。
pub fn open_link(hid: &HidApi) -> Result<Link, String> {
    open_links(hid)?
        .into_iter()
        .next()
        .ok_or_else(|| "没有可用的命令通道".to_string())
}

fn protocol_input(mut frame: Vec<u8>, report_id: Option<u8>, capacity: usize) -> Option<Vec<u8>> {
    if report_id == Some(0) {
        if frame.len() < 2 || frame.len() > capacity || frame[0] != 0 { return None; }
        frame.remove(0);
    }
    Some(frame)
}

#[cfg(test)]
mod link_tests {
    use super::*;

    fn interface(path: &str, page: u16) -> HidInterface {
        HidInterface {
            path: path.into(), vendor_id: VID_ASUS, product_id: PID_DONGLE,
            usage_page: page, usage: 1, input_len: 64, output_len: 64,
            feature_len: 0, product: "identical receiver".into(), serial: String::new(),
        }
    }

    fn pairing(report_id: u8) -> Vec<u8> {
        vec![1, 0xA0, 0, 1, 0, 0x65, 0x1B, report_id, 0]
    }

    fn report(interfaces: Vec<HidInterface>) -> EnumerateReport {
        EnumerateReport { interfaces, skipped: Vec::new() }
    }

    fn captured_omni_pairing() -> Vec<u8> {
        let mut frame = vec![0x01, 0xA0, 0x00, 0x01, 0x00, 0x65, 0x1B, 0x03, 0x05];
        frame.resize(64, 0);
        frame
    }

    #[test]
    fn discovery_callback_precedes_all_protocol_probes_and_preserves_presence() {
        let events = std::cell::RefCell::new(Vec::new());
        let mut wired = interface("wired", USAGE_PAGE_MOUSE_CTRL);
        wired.product_id = PID_MOUSE_WIRED;
        let mut discovered = report(vec![wired, interface("control", USAGE_PAGE_DONGLE_CTRL),
            interface("mouse", USAGE_PAGE_MOUSE_CTRL)]);
        discovered.skipped.push(Skipped { path: "blocked".into(), reason: "busy".into() });
        let scan = scan_links_observed_with(discovered,
            |interface| Some(if interface.path == "wired" { "Wired" } else { "Receiver" }.into()),
            |_| { events.borrow_mut().push("wired"); Some(1) },
            |_| { events.borrow_mut().push("pairing"); Ok(Some(captured_omni_pairing())) },
            |report, groups| {
                assert!(events.borrow().is_empty());
                assert_eq!(report.interfaces.len(), 3);
                assert_eq!(report.skipped[0].path, "blocked");
                assert_eq!(groups["control"], "receiver");
                assert_eq!(groups["mouse"], "receiver");
                assert_eq!(groups["wired"], "wired");
                events.borrow_mut().push("discovered");
            }).unwrap();
        assert_eq!(*events.borrow(), vec!["discovered", "wired", "pairing"]);
        assert_eq!(scan.links.len(), 2);
        assert_eq!(scan.present_channels, ["wired", "control", "mouse", "blocked"]
            .into_iter().map(String::from).collect());
    }

    #[test]
    fn omni_firmware_80007_pairing_reply_uses_the_report_id_before_notify_id() {
        let frame = captured_omni_pairing();
        assert_eq!(parse_pairing_table(&frame, &[PID_MOUSE_WIRELESS, PID_MOUSE_WIRED]), Some(3));
        for length in 0..9 {
            assert_eq!(parse_pairing_table(&frame[..length], &[PID_MOUSE_WIRELESS]), None);
        }
    }

    #[test]
    fn omni_firmware_80007_reply_establishes_the_paired_mouse_channel() {
        let scan = scan_links_with(report(vec![
            interface("control", USAGE_PAGE_DONGLE_CTRL),
            interface("mouse", USAGE_PAGE_MOUSE_CTRL),
        ]), |_| Some("physical".into()), |_| None, |_| Ok(Some(captured_omni_pairing()))).unwrap();
        assert_eq!(scan.links.len(), 1);
        assert_eq!(scan.links[0].report_id, 3);
        assert_eq!(scan.links[0].iface.path, "mouse");
        assert!(!scan.links[0].read_only);
    }

    #[test]
    fn pairing_table_requires_a_valid_header_and_all_counted_entries() {
        let frame = vec![1, 0xA0, 0, 2, 0, 0x34, 0x12, 5, 6, 0x65, 0x1B, 7, 8];
        assert_eq!(parse_pairing_table(&frame, &[PID_MOUSE_WIRELESS]), Some(7));
        assert_eq!(parse_pairing_table(&frame[..12], &[0x1234]), None);
        for header_index in 0..3 {
            let mut invalid = frame.clone();
            invalid[header_index] ^= 1;
            assert_eq!(parse_pairing_table(&invalid, &[PID_MOUSE_WIRELESS]), None);
        }
        let mut high_count = frame;
        high_count[4] = 1;
        assert_eq!(parse_pairing_table(&high_count, &[PID_MOUSE_WIRELESS]), None);
    }

    #[test]
    fn every_receiver_keeps_its_own_report_id_and_interfaces_in_any_order() {
        let interfaces = [
            interface("A-control", USAGE_PAGE_DONGLE_CTRL),
            interface("B-mouse", USAGE_PAGE_MOUSE_CTRL),
            interface("B-control", USAGE_PAGE_DONGLE_CTRL),
            interface("A-mouse", USAGE_PAGE_MOUSE_CTRL),
        ];
        for rotation in 0..4 {
            let mut interfaces = interfaces.to_vec();
            interfaces.rotate_left(rotation);
            let links = open_links_with(report(interfaces),
                |interface| Some(interface.path[..1].into()),
                |_| panic!("receiver is not wired"),
                |interface| Some(pairing(if interface.path.starts_with('A') { 3 } else { 7 })),
            ).unwrap();
            assert_eq!(links.len(), 2);
            for link in links {
                let group = &link.iface.path[..1];
                assert!(link.iface.path.ends_with("-mouse"));
                assert_eq!(link.report_id, if group == "A" { 3 } else { 7 });
                assert_eq!(link.all_interfaces.len(), 2);
                assert!(link.all_interfaces.iter().all(|interface| interface.path.starts_with(group)));
            }
        }
    }

    #[test]
    fn unresolved_parent_never_attaches_another_interfaces_control_channel() {
        let links = open_links_with(report(vec![
            interface("control", USAGE_PAGE_DONGLE_CTRL),
            interface("mouse", USAGE_PAGE_MOUSE_CTRL),
        ]), |_| None, |_| None, |_| Some(pairing(7))).unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].iface.path, "control");
        assert_eq!(links[0].all_interfaces.len(), 1);
        assert_eq!(links[0].all_interfaces[0].path, "control");
    }

    #[test]
    fn unconfirmed_pairing_never_invents_a_default_report_id() {
        for scan in [None, Some(vec![1, 0xFF, 0xAA]), Some(vec![1, 0xA0, 0, 0, 0]),
            Some(vec![1, 0xA0, 0, 1, 0, 1, 2, 7, 0])] {
            let links = open_links_with(report(vec![interface("control", USAGE_PAGE_DONGLE_CTRL)]),
                |_| Some("receiver".into()), |_| None, |_| scan.clone());
            assert!(links.unwrap().is_empty());
        }
    }

    #[test]
    fn wired_link_carries_only_its_physical_interfaces() {
        let mut wired = interface("wired", USAGE_PAGE_MOUSE_CTRL);
        wired.product_id = PID_MOUSE_WIRED;
        let links = open_links_with(report(vec![wired, interface("receiver", USAGE_PAGE_DONGLE_CTRL)]),
            |interface| Some(interface.path.clone()), |_| Some(1), |_| None).unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].all_interfaces.len(), 1);
        assert_eq!(links[0].all_interfaces[0].path, "wired");
    }

    #[test]
    fn ambiguous_sibling_control_channels_are_not_selected_by_order() {
        let result = open_links_with(report(vec![
            interface("control-a", USAGE_PAGE_DONGLE_CTRL),
            interface("control-b", USAGE_PAGE_DONGLE_CTRL),
            interface("mouse", USAGE_PAGE_MOUSE_CTRL),
        ]), |_| Some("receiver".into()), |_| None, |_| Some(pairing(3)));
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn complete_empty_enumeration_is_distinct_from_unreadable_candidates() {
        assert!(open_links_with(report(Vec::new()), |_| None, |_| None, |_| None).unwrap().is_empty());
        let report = EnumerateReport { interfaces: Vec::new(),
            skipped: vec![Skipped { path: "occupied".into(), reason: "sharing violation".into() }] };
        assert!(open_links_with(report, |_| None, |_| None, |_| None).unwrap().is_empty());
        let mut standard = interface("standard", 1);
        standard.usage = 2;
        assert!(open_links_with(self::report(vec![standard]), |_| None, |_| None, |_| None).unwrap().is_empty());
    }

    #[test]
    fn incomplete_or_ambiguous_pairing_table_does_not_select_a_slot() {
        let mut truncated = pairing(3);
        truncated[3] = 2;
        let mut ambiguous = truncated.clone();
        ambiguous.extend_from_slice(&[0x63, 0x1B, 7, 0]);
        for scan in [truncated, ambiguous, pairing(0)] {
            assert!(parse_pairing_table(&scan, &[PID_MOUSE_WIRELESS, PID_MOUSE_WIRED]).is_none());
        }
    }

    #[test]
    fn usb_ancestry_joins_interfaces_but_preserves_physical_instances() {
        let nodes = [
            (1, "HID\\VID_0B05&PID_1ACE&MI_00&COL01\\collection-a", Some(2)),
            (2, "USB\\VID_0B05&PID_1ACE&MI_00\\interface-a", Some(3)),
            (3, "USB\\VID_0B05&PID_1ACE\\physical-a", None),
            (4, "HID\\VID_0B05&PID_1ACE&MI_01&COL01\\collection-b", Some(5)),
            (5, "USB\\VID_0B05&PID_1ACE&MI_01\\interface-b", Some(3)),
            (6, "USB\\VID_0B05&PID_1ACE\\physical-b", None),
        ];
        let group = |start| usb_physical_group_with(start, VID_ASUS, PID_DONGLE,
            |node| nodes.iter().find(|entry| entry.0 == node).map(|entry| entry.1.into()),
            |node| nodes.iter().find(|entry| entry.0 == node).and_then(|entry| entry.2));
        assert_eq!(group(1), Some("usb\\vid_0b05&pid_1ace\\physical-a".into()));
        assert_eq!(group(1), group(4));
        assert_ne!(group(1), group(6));
        assert_eq!(group(99), None);
    }

    #[test]
    fn usb_ancestry_fails_closed_on_missing_wrong_or_cyclic_parents() {
        assert_eq!(usb_physical_group_with(1, VID_ASUS, PID_DONGLE,
            |_| Some("USB\\VID_1234&PID_5678\\other".into()), |_| None), None);
        assert_eq!(usb_physical_group_with(1, VID_ASUS, PID_DONGLE,
            |_| Some("USB\\VID_0B05&PID_1ACE&MI_00\\interface".into()), |_| Some(1)), None);
        assert_eq!(usb_physical_group_with(1, VID_ASUS, PID_DONGLE,
            |_| Some("USB\\VID_0B05&PID_1ACE\\".into()), |_| None), None);
        let mut calls = 0;
        assert_eq!(usb_physical_group_with(1, VID_ASUS, PID_DONGLE,
            |_| { calls += 1; Some("ROOT\\unknown\\node".into()) }, |node| Some(node + 1)), None);
        assert_eq!(calls, 32);
    }

    #[test]
    fn grouped_enumeration_keeps_failed_interfaces_and_normalizes_instance_keys() {
        let first = interface("FIRST", USAGE_PAGE_MOUSE_CTRL);
        let failed = Skipped { path: r"\\?\HID#VID_0B05&PID_1ACE#second".into(), reason: "occupied".into() };
        let partial = Skipped { path: r"\\?\hid#vid_0b05&pid_invalid#third".into(), reason: "occupied".into() };
        let instances = [(first.path.to_ascii_lowercase(), 1), (failed.path.to_ascii_lowercase(), 2),
            (partial.path.to_ascii_lowercase(), 3)].into_iter().collect();
        let report = EnumerateReport { interfaces: vec![first.clone()], skipped: vec![failed.clone(), partial] };
        let mut visited = Vec::new();
        let grouped = group_report_with(report, &instances, |instance, vendor, product| {
            assert_eq!((vendor, product), (VID_ASUS, PID_DONGLE));
            visited.push(instance);
            Some(format!("USB\\VID_0B05&PID_1ACE\\PHYSICAL-{instance}"))
        });
        assert_eq!(visited, vec![1, 2]);
        assert_eq!(grouped.report.interfaces.len(), 1);
        assert_eq!(grouped.report.skipped.len(), 2);
        assert_eq!(grouped.physical_groups.len(), 2);
        assert_eq!(grouped.physical_groups["first"], "usb\\vid_0b05&pid_1ace\\physical-1");
        assert_eq!(grouped.physical_groups[&failed.path.to_ascii_lowercase()], "usb\\vid_0b05&pid_1ace\\physical-2");
        let unresolved = group_report_with(grouped.report, &instances, |_, _, _| None);
        assert!(unresolved.physical_groups.is_empty());
    }

    #[test]
    fn feedback_grouping_stops_at_non_usb_and_unrelated_ancestors() {
        for identity in [r"BTHENUM\DEV_112233445566\remote", r"BTHLEDEVICE\unknown\remote",
            r"ROOT\unknown\node", r"USB\VID_0B05&PID_FFFF\other",
            r"USB\VID_0B05&PID_1ACE&MI_00&extra\malformed", r"USB\VID_0B05&PID_1ACE\",
            "USB\\VID_0B05&PID_1ACE\\bad\0"] {
            assert!(usb_physical_group_with(1, VID_ASUS, PID_DONGLE,
                |_| usb_ancestor_matches(identity, VID_ASUS, PID_DONGLE).then(|| identity.into()),
                |_| panic!("uncertain ancestry must not reach a shared USB adapter")).is_none());
        }
        for identity in [r"HID\VID_0B05&PID_1ACE&MI_00&COL01\collection",
            r"USB\VID_0B05&PID_1ACE&MI_00\interface", r"USB\VID_0B05&PID_1ACE\physical"] {
            assert!(usb_ancestor_matches(identity, VID_ASUS, PID_DONGLE));
        }
    }

    #[test]
    fn link_scan_keeps_present_channels_when_one_receiver_cannot_answer() {
        let report = EnumerateReport { interfaces: vec![
            interface("A-control", USAGE_PAGE_DONGLE_CTRL),
            interface("B-control", USAGE_PAGE_DONGLE_CTRL),
        ], skipped: vec![Skipped { path: "C-Occupied".into(), reason: "busy".into() }] };
        let scan = scan_links_with(report, |interface| Some(interface.path.clone()), |_| None,
            |interface| if interface.path.starts_with('A') { Ok(Some(pairing(7))) } else { Err(()) }).unwrap();
        assert_eq!(scan.links.len(), 1);
        assert_eq!(scan.present_channels, ["a-control", "b-control", "c-occupied"]
            .into_iter().map(str::to_string).collect());
    }

    #[test]
    fn complete_unreadable_scan_still_returns_presence_evidence() {
        let scan = scan_links_with(EnumerateReport { interfaces: Vec::new(),
            skipped: vec![Skipped { path: "Occupied".into(), reason: "busy".into() }] },
            |_| None, |_| None, |_| panic!("unreadable interface")).unwrap();
        assert!(scan.links.is_empty());
        assert!(scan.present_channels.contains("occupied"));
    }

    #[test]
    fn verified_omni_group_keeps_the_read_only_default_on_pairing_timeout() {
        let scan = scan_links_with(report(vec![
            interface("control", USAGE_PAGE_DONGLE_CTRL),
            interface("mouse", USAGE_PAGE_MOUSE_CTRL),
        ]), |_| Some("physical".into()), |_| None, |_| Ok(None)).unwrap();
        assert_eq!(scan.links.len(), 1);
        assert_eq!(scan.links[0].report_id, 3);
        assert_eq!(scan.links[0].iface.path, "mouse");
        assert!(scan.links[0].read_only);
    }

    #[test]
    fn duplicate_receiver_serials_keep_distinct_unverified_link_keys() {
        let mut first = Link { mode: "Wireless24G".into(), report_id: 3, read_only: false,
            iface: interface("receiver-a", USAGE_PAGE_MOUSE_CTRL), all_interfaces: Vec::new() };
        first.iface.serial = "DUPLICATE".into();
        let mut second = first.clone();
        second.iface.path = "receiver-b".into();
        assert_ne!(first.device_key(), second.device_key());
        let key = first.device_key();
        first.iface.serial = "DUPLICATED+".into();
        assert_eq!(key, first.device_key());
        first.iface.path = "RECEIVER-A".into();
        assert_eq!(key, first.device_key());
        first.report_id = 7;
        assert_ne!(key, first.device_key());
    }

    #[test]
    fn pairing_default_does_not_apply_to_failed_unknown_or_ambiguous_groups() {
        let interfaces = vec![interface("control", USAGE_PAGE_DONGLE_CTRL),
            interface("mouse", USAGE_PAGE_MOUSE_CTRL)];
        let unknown = scan_links_with(report(interfaces.clone()), |_| None, |_| None, |_| Ok(None)).unwrap();
        assert!(unknown.links.is_empty());
        let failed = scan_links_with(report(interfaces.clone()), |_| Some("physical".into()), |_| None, |_| Err(())).unwrap();
        assert!(failed.links.is_empty());
        for frame in [vec![1, 0xA0, 0, 0, 0], vec![1, 0xFF, 0xAA],
            vec![1, 0xA0, 0, 2, 0, 0x65, 0x1B, 3, 0, 0x63, 0x1B, 7, 0]] {
            let scan = scan_links_with(report(interfaces.clone()), |_| Some("physical".into()),
                |_| None, |_| Ok(Some(frame.clone()))).unwrap();
            assert!(scan.links.is_empty());
        }
    }
}

#[cfg(test)]
mod string_tests {
    use super::*;
    use crate::backends::{BackendId, Target};

    fn read(value: &str, which: u8) -> String {
        read_hid_string_with(which, |selected, buffer, capacity| {
            assert_eq!(selected, which);
            assert_eq!(capacity, 512);
            let value = to_wide(value);
            buffer[..value.len()].copy_from_slice(&value);
            1
        })
    }

    #[test]
    fn distinct_serial_suffixes_produce_distinct_standard_hid_keys() {
        let mut interface = HidInterface {
            path: "first".into(), vendor_id: 1, product_id: 2,
            usage_page: 1, usage: 2, input_len: 64, output_len: 0, feature_len: 0,
            product: String::new(), serial: read("ABC-1", 1),
        };
        let first = Target::new(BackendId::StandardHid, &interface, 0);
        interface.path = "second".into();
        interface.serial = read("ABC-2", 1);
        let second = Target::new(BackendId::StandardHid, &interface, 0);
        assert_ne!(first.key, second.key);
        assert_eq!(first.hid().unwrap().serial, "ABC-1");
        assert_eq!(second.hid().unwrap().serial, "ABC-2");
    }

    #[test]
    fn descriptor_strings_preserve_punctuation_and_unicode() {
        for which in [0, 1] {
            for value in ["ABC-1", "ABC_2", "ABC.3", "SN:42/7", "设备-四"] {
                assert_eq!(read(&format!("  {value}  "), which), value);
            }
        }
    }

    #[test]
    fn descriptor_strings_stop_at_nul_and_ignore_failed_reads() {
        assert_eq!(read("ABC\0discarded", 1), "ABC");
        assert_eq!(read("", 1), "");
        assert_eq!(read_hid_string_with(1, |_, buffer, _| {
            buffer[0] = b'A' as u16;
            0
        }), "");
    }
}

#[cfg(test)]
mod enumeration_tests {
    use super::*;

    fn interface(path: &str) -> HidInterface {
        HidInterface {
            path: path.into(), vendor_id: VID_ASUS, product_id: PID_DONGLE,
            usage_page: USAGE_PAGE_DONGLE_CTRL, usage: 1,
            input_len: 64, output_len: 64, feature_len: 0,
            product: String::new(), serial: String::new(),
        }
    }

    #[test]
    fn enumeration_error_after_a_valid_interface_rejects_the_partial_scan() {
        let mut indices = Vec::new();
        let result = enumerate_interfaces_with(
            |index| {
                indices.push(index);
                if index == 0 { Ok(SpDeviceInterfaceData::default()) } else { Err(5) }
            },
            |_| Ok(Some("first".into())),
            |path| Ok(interface(path)),
        );
        assert!(result.is_err(), "a partial scan must not report other devices absent");
        assert!(result.err().unwrap().contains("GetLastError=5"));
        assert_eq!(indices, [0, 1]);
    }

    #[test]
    fn enumeration_error_after_a_skipped_interface_is_not_an_empty_success() {
        let result = enumerate_interfaces_with(
            |index| if index == 0 { Ok(SpDeviceInterfaceData::default()) } else { Err(31) },
            |_| Ok(Some("occupied".into())),
            |_| Err("sharing violation".into()),
        );
        assert!(result.is_err());
    }

    #[test]
    fn normal_enumeration_end_preserves_present_and_skipped_interfaces() {
        let report = enumerate_interfaces_with(
            |index| if index < 2 {
                Ok(SpDeviceInterfaceData { flags: index, ..Default::default() })
            } else { Err(ERROR_NO_MORE_ITEMS) },
            |interface| Ok(Some(format!("path-{}", interface.flags))),
            |path| if path == "path-0" { Ok(interface(path)) } else { Err("occupied".into()) },
        ).unwrap();
        assert_eq!(report.interfaces.len(), 1);
        assert_eq!(report.interfaces[0].path, "path-0");
        assert_eq!(report.skipped, [Skipped { path: "path-1".into(), reason: "occupied".into() }]);
    }

    #[test]
    fn initial_enumeration_failure_and_normal_empty_result_remain_distinct() {
        for error in [5, ERROR_NO_MORE_ITEMS] {
            let result = enumerate_interfaces_with(
                |_| Err(error), |_| panic!("no interface"), |_| panic!("no path"),
            );
            if error == ERROR_NO_MORE_ITEMS {
                let report = result.unwrap();
                assert!(report.interfaces.is_empty() && report.skipped.is_empty());
            } else {
                assert!(result.is_err());
            }
        }
    }

    #[test]
    fn interface_path_failure_is_not_a_successful_empty_scan() {
        let result = enumerate_interfaces_with(
            |_| Ok(SpDeviceInterfaceData::default()),
            |_| Err("interface disappeared during detail lookup".into()),
            |_| panic!("no path was resolved"),
        );
        assert_eq!(result.err().as_deref(), Some("interface disappeared during detail lookup"));
    }
}

#[cfg(test)]
pub(crate) mod capture_test_support {
    pub(crate) struct CaptureGuard { path: String }

    impl CaptureGuard {
        pub(crate) fn start(interface: &super::HidInterface) -> Self {
            crate::capture::clear();
            crate::capture::start(interface).unwrap();
            Self { path: interface.path.clone() }
        }

        pub(crate) fn transfers(&self) -> Vec<serde_json::Value> {
            crate::capture::stop();
            let raw = crate::capture::feedback_data(&self.path)
                .unwrap_or_else(|| crate::capture::status().preview.join("\n"));
            raw.lines().map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
                .filter(|record| record["kind"] == "transfer").collect()
        }
    }

    impl Drop for CaptureGuard {
        fn drop(&mut self) { crate::capture::clear(); }
    }
}

#[cfg(test)]
mod input_tests {
    use super::protocol_input;

    #[test]
    fn control_capture_preserves_wrong_ids_and_rejects_invalid_lengths_after_recording() {
        let interface = super::HidInterface { path: "control-capture-test".into(), vendor_id: 0x1234,
            product_id: 0x5678, usage_page: 0xff00, usage: 1, input_len: 4,
            output_len: 4, feature_len: 4, product: String::new(), serial: String::new() };
        for feature in [false, true] {
            for (returned, report_id, bytes, accepted, expected_hex) in [
                (Some(3), 7, [8, 0xab, 0xcd, 0xee], false, "08ABCD"),
                (Some(3), 7, [7, 0xab, 0xcd, 0xee], true, "07ABCD"),
                (Some(3), 0, [0, 0xab, 0xcd, 0xee], true, "00ABCDEE"),
                (Some(0), 0, [0, 0xab, 0xcd, 0xee], false, ""),
                (Some(5), 7, [7, 0xab, 0xcd, 0xee], false, "07ABCDEE"),
                (None, 7, [7, 0xab, 0xcd, 0xee], false, ""),
            ] {
                let capture = super::capture_test_support::CaptureGuard::start(&interface);
                let result = super::query_report_captured(&interface, report_id, feature, |report| {
                    assert_eq!(report, &[report_id, 0, 0, 0]);
                    report.copy_from_slice(&bytes);
                    returned
                });
                assert_eq!(result.is_some(), accepted);
                let records = capture.transfers();
                assert_eq!(records.len(), 1);
                assert_eq!(records[0]["api"], if feature { "get-feature" } else { "get-input" });
                assert_eq!(records[0]["hex"], expected_hex);
                assert_eq!(records[0]["success"], returned.is_some());
            }
        }
        let rejected = super::query_report_with(4, 7, |report| {
            report.copy_from_slice(&[8, 0xab, 0xcd, 0xee]);
            Some(3)
        }, |_, _| {});
        assert!(rejected.is_none());
        assert!(super::query_report_with(0, 7, |_| panic!("empty report must not reach I/O"),
            |_, _| panic!("no transfer occurred")).is_none());
    }

    #[test]
    fn capture_records_wait_timeouts_but_excludes_empty_queue_checks() {
        let interface = super::HidInterface { path: "timeout-capture-test".into(), vendor_id: 0x1234,
            product_id: 0x5678, usage_page: 0xff00, usage: 1, input_len: 64,
            output_len: 64, feature_len: 0, product: String::new(), serial: String::new() };
        crate::capture::clear();
        crate::capture::start(&interface).unwrap();
        super::capture_input_result(&interface.path, 0, &Ok(None));
        assert_eq!(crate::capture::status().frames, 0);
        super::capture_input_result(&interface.path, 500, &Ok(None));
        super::capture_input_result(&interface.path, 500, &Err(()));
        super::capture_input_result(&interface.path, 500, &Ok(Some(vec![1, 2])));
        let status = crate::capture::status();
        assert_eq!(status.frames, 3);
        assert!(status.preview.iter().any(|line| line.contains("input-timeout") && line.contains("\"success\":false")));
        assert!(status.preview.iter().any(|line| line.contains("0102")));
        crate::capture::clear();
    }

    #[test]
    fn windows_unnumbered_input_removes_id_only_after_receiving_full_buffer() {
        let mut windows = vec![0; 65];
        windows[1] = 0xaa; windows[2] = 3; windows[9] = 40;
        let payload = protocol_input(windows.clone(), Some(0), 65).unwrap();
        assert_eq!(payload.len(), 64);
        assert_eq!((&payload[..2], payload[8]), (&[0xaa, 3][..], 40));
        // Short reads stay short so each protocol can reject them itself.
        assert_eq!(protocol_input(windows[..12].to_vec(), Some(0), 65).unwrap().len(), 11);
        assert!(protocol_input(vec![0], Some(0), 65).is_none());
        assert!(protocol_input(vec![0xaa; 64], Some(0), 65).is_none());
        assert!(protocol_input(vec![0; 66], Some(0), 65).is_none());
    }

    #[test]
    fn numbered_and_unspecified_inputs_preserve_existing_byte_layout() {
        for report_id in [None, Some(4), Some(0x4d)] {
            let frame = vec![0x4d, 1, 1, 14, 0, 9];
            assert_eq!(protocol_input(frame.clone(), report_id, 64), Some(frame));
        }
        let frame = vec![0, 0xb0, 0, 42, 1];
        assert_eq!(protocol_input(frame.clone(), None, 65), Some(frame));
    }
}
