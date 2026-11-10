
#![allow(non_snake_case, non_camel_case_types)]

use core::ffi::c_void;

pub type Handle = *mut c_void;

/// GDI 句柄（`HDC` / `HBITMAP` / `HBRUSH` / `HPEN` …）。
///
/// 与 [`Handle`] 同名同源的都是不透明句柄，但 GDI 句柄在 Win32 头文件里是
/// `HANDLE`/`HGDIOBJ` 的整数形式，所以这里仍然用 `isize` 而不是指针 ——
/// 用指针类型会诱使人写 `std::ptr::null_mut()`，而 `null_mut()` 是 `*mut _`，
/// 赋给 `isize` 直接编译失败。
pub type Gdi = isize;

/// 宽度必须与 [`Handle`] 相同：边界上会以 `as` 在整数与指针之间互转（如
/// `GdipCreateFromHDC` 的 HDC 参数）。用编译期断言把这条约束钉住。
///
/// 复核（审计 85）：保持 `isize`。`HDC`/`HBITMAP` 是不透明整数值，改成指针
/// 类型会让全部 `== 0` 哨兵判定与 `ensure(0, ...)` 调用点被迫改写成
/// `null_mut()`，换来只是拼写统一。
const _: () = assert!(core::mem::size_of::<Gdi>() == core::mem::size_of::<Handle>());

pub const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;

// ---- 访问权限 / 打开方式 ----
pub const GENERIC_READ: u32 = 0x8000_0000;
pub const GENERIC_WRITE: u32 = 0x4000_0000;
/// 必须同时给 READ 和 WRITE：Armoury Crate / GearLink 可能同时开着这个句柄。
pub const FILE_SHARE_READ: u32 = 0x0000_0001;
pub const FILE_SHARE_WRITE: u32 = 0x0000_0002;
pub const OPEN_EXISTING: u32 = 3;
pub const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;

// ---- 标准设备 / 控制台 ----
/// `AttachConsole` 的参数：接到父进程的控制台上。
pub const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
pub const STD_INPUT_HANDLE: u32 = 0xFFFF_FFF6;
pub const STD_OUTPUT_HANDLE: u32 = 0xFFFF_FFF5;
pub const STD_ERROR_HANDLE: u32 = 0xFFFF_FFF4;

// ---- SetupDi 标志 ----
pub const DIGCF_PRESENT: u32 = 0x0000_0002;
pub const DIGCF_DEVICEINTERFACE: u32 = 0x0000_0010;

// ---- 错误码 ----
pub const ERROR_NO_MORE_ITEMS: u32 = 259;
pub const ERROR_IO_PENDING: u32 = 997;

pub const WAIT_OBJECT_0: u32 = 0;
/// 上一个持有者崩溃了，内核把互斥体交给我们。**要当成拿到锁继续干活**，
/// 不是失败 —— 不处理会导致崩溃后整个功能永久卡死。
pub const WAIT_ABANDONED: u32 = 0x80;
/// 超时。等不到就放弃本次操作，不要阻塞。
pub const WAIT_TIMEOUT: u32 = 0x102;

/// HID 的设备接口 GUID：{4d1e55b2-f16f-11cf-88cb-001111000030}
/// 运行时由 `HidD_GetHidGuid` 取，避免手抄出错。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

#[repr(C)]
pub struct SpDeviceInterfaceData {
    pub cb_size: u32,
    pub interface_class_guid: Guid,
    pub flags: u32,
    pub reserved: usize,
}

impl Default for SpDeviceInterfaceData {
    fn default() -> Self {
        Self {
            cb_size: core::mem::size_of::<Self>() as u32,
            interface_class_guid: Guid::default(),
            flags: 0,
            reserved: 0,
        }
    }
}

#[repr(C)]
#[derive(Default)]
pub struct HiddAttributes {
    pub size: u32,
    pub vendor_id: u16,
    pub product_id: u16,
    pub version_number: u16,
}

#[repr(C)]
#[derive(Default)]
pub struct HidpCaps {
    pub usage: u16,
    pub usage_page: u16,
    pub input_report_byte_length: u16,
    pub output_report_byte_length: u16,
    pub feature_report_byte_length: u16,
    pub reserved: [u16; 17],
    pub number_link_collection_nodes: u16,
    pub number_input_button_caps: u16,
    pub number_input_value_caps: u16,
    pub number_input_data_indices: u16,
    pub number_output_button_caps: u16,
    pub number_output_value_caps: u16,
    pub number_output_data_indices: u16,
    pub number_feature_button_caps: u16,
    pub number_feature_value_caps: u16,
    pub number_feature_data_indices: u16,
}

/// `OVERLAPPED` 在 x64 上 32 字节：2 个 ULONG_PTR + 2 个 DWORD + 1 个 HANDLE。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Overlapped {
    pub internal: usize,
    pub internal_high: usize,
    pub offset: u32,
    pub offset_high: u32,
    pub h_event: Handle,
}

impl Default for Overlapped {
    fn default() -> Self {
        Self {
            internal: 0,
            internal_high: 0,
            offset: 0,
            offset_high: 0,
            h_event: core::ptr::null_mut(),
        }
    }
}

// ============ kernel32 ============
#[link(name = "kernel32")]
extern "system" {
    pub fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sec: *mut c_void,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;

    pub fn ReadFile(h: Handle, buf: *mut c_void, to_read: u32, read: *mut u32, ov: *mut Overlapped) -> i32;

    pub fn WriteFile(h: Handle, buf: *const c_void, to_write: u32, written: *mut u32, ov: *mut Overlapped) -> i32;

    pub fn CloseHandle(h: Handle) -> i32;
    pub fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    pub fn GetProcessTimes(
        proc: Handle,
        creation: *mut u64,
        exit: *mut u64,
        kernel: *mut u64,
        user: *mut u64,
    ) -> i32;

    pub fn CreateEventW(sec: *mut c_void, manual_reset: i32, initial: i32, name: *const u16) -> Handle;

    pub fn WaitForSingleObject(h: Handle, ms: u32) -> u32;

    pub fn GetOverlappedResult(h: Handle, ov: *mut Overlapped, transferred: *mut u32, wait: i32) -> i32;

    pub fn CancelIoEx(h: Handle, ov: *mut Overlapped) -> i32;

    pub fn GetLastError() -> u32;

    /// 系统运行毫秒数（32 位，约 49.7 天回绕）。用于动画相位，不做时间差计算。
    ///
    /// 为什么不用 `Instant::now()`：动效只需要「每帧不重样」，用单调时钟更省事，
    /// 但 `std::time::Instant` 在某些老平台上走的是不保证单调的路径。这里只取模
    /// 算相位，回绕一次会让彗尾跳一下，无感。

    /// 本地时间。备份文件名用它，避免出现 UTC `14:26` 与本地 `22:26` 不一致的困惑。
    pub fn GetLocalTime(out: *mut SystemTime);

    /// FILETIME（UTC）→ FILETIME（本地）。时区换算交给系统，不自己实现。
    pub fn FileTimeToLocalFileTime(in_ft: *const FileTime, out_ft: *mut FileTime) -> i32;

    /// FILETIME → SYSTEMTIME。
    pub fn FileTimeToSystemTime(in_ft: *const FileTime, out_st: *mut SystemTime) -> i32;

    pub fn LoadLibraryW(name: *const u16) -> Handle;

    pub fn GetProcAddress(module: Handle, name: *const u8) -> *mut c_void;

    /// 脱离当前控制台。GUI 模式启动时调一次，双击 exe 才不会挂一个黑框。
    ///
    /// 子系统已经是 `windows`（见 `src/main.rs` 的 `windows_subsystem`），所以
    /// 裸跑时进程本来就没有控制台；这里保留调用只是为了「从终端启动 GUI」这一
    /// 种情况：那时 `attach_console` 已经把父控制台接上了，进 GUI 前要脱开。
    pub fn FreeConsole() -> i32;

    /// 把本进程接到指定进程的控制台上（`ATTACH_PARENT_PROCESS` = 父进程）。
    pub fn AttachConsole(process_id: u32) -> i32;

    pub fn GetStdHandle(which: u32) -> Handle;

    pub fn SetStdHandle(which: u32, h: Handle) -> i32;

    /// 命名互斥体。`history.tsv` 的跨进程保护用它。
    ///
    /// `initial_owner = 0`（FALSE）：创建者**不**立刻持有，必须显式 `WaitForSingleObject`。
    /// C# 版用的是 `new Mutex(false, @"Local\RogBatteryHistory")`，语义一致。
    pub fn CreateMutexW(sec: *mut c_void, initial_owner: i32, name: *const u16) -> Handle;

    /// 释放互斥体。返回 0 表示成功。
    pub fn ReleaseMutex(h: Handle) -> i32;

    /// 伪句柄 `(HANDLE)-1`，恒指当前进程，不需要关闭。
    pub fn GetCurrentProcess() -> Handle;
    /// 传 `-1, -1` 让系统把工作集里的页尽量换出（`EmptyWorkingSet` 的内核实现）。
    pub fn SetProcessWorkingSetSize(process: Handle, min: usize, max: usize) -> i32;
}

/// `SYSTEMTIME`（wYear..wMilliseconds 共 8 个 WORD）。只用到年月日时分秒。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SystemTime {
    pub year: u16,
    pub month: u16,
    pub day_of_week: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub milliseconds: u16,
}

/// `FILETIME`：1601-01-01 起的 100 纳秒计数，拆成高低两个 DWORD。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FileTime {
    pub low: u32,
    pub high: u32,
}

/// `RECT`。鼠标坐标用 `left/top`，尺寸用 `right - left`。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PaintStruct {
    pub hdc: Handle,
    pub erase: i32,
    pub paint: Rect,
    pub restore: i32,
    pub inc_update: i32,
    pub reserved: [u8; 32],
}

impl Default for PaintStruct {
    fn default() -> Self {
        Self {
            hdc: core::ptr::null_mut(),
            erase: 0,
            paint: Rect::default(),
            restore: 0,
            inc_update: 0,
            reserved: [0; 32],
        }
    }
}

// ============ 托盘图标自绘用到的结构 ============

/// `MONITORINFO`。`GetMonitorInfoW` 用它返回显示器的工作区（去掉任务栏那块）。
///
/// 无边框窗口最大化时必须自己算工作区：`WS_THICKFRAME` 让系统以为有 8px 边框，
/// 最大化后右侧/底部会各多出 8px 跑到屏幕外，任务栏也被盖掉。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MonitorInfo {
    /// **必须先填** `size_of::<MonitorInfo>()`，否则 `GetMonitorInfoW` 直接返回 0。
    pub cb_size: u32,
    /// 整个显示器矩形（含任务栏区域）。
    pub rc_monitor: Rect,
    /// 工作区矩形（去掉任务栏、去掉停靠的工具栏）。
    pub rc_work: Rect,
    /// `MONITORINFOF_PRIMARY` = 1。
    pub flags: u32,
}

// ============ Toolhelp（父进程探测）============
//
// 用途只有一个：判断「这次是不是开机自启拉起来的」。MSIX 的 `desktop:StartupTask`
// 是系统激活，命令行里拿不到 `--autostart`，只能看父进程是不是 explorer.exe。

/// `TH32CS_SNAPPROCESS`
pub const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;

/// `PROCESSENTRY32W`。字段顺序/宽度严格按 `tlhelp32.h`，`dw_size` 必须先填。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ProcessEntry32W {
    pub dw_size: u32,
    pub cnt_usage: u32,
    pub th32_process_id: u32,
    /// `ULONG_PTR`：x64 上是 8 字节，写 `u32` 会让后面所有字段错位。
    pub th32_default_heap_id: usize,
    pub th32_module_id: u32,
    pub cnt_threads: u32,
    pub th32_parent_process_id: u32,
    pub pc_pri_class_base: i32,
    pub dw_flags: u32,
    pub sz_exe_file: [u16; 260],
}

#[link(name = "kernel32")]
extern "system" {
    pub fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
    pub fn Process32FirstW(snap: Handle, entry: *mut ProcessEntry32W) -> i32;
    pub fn Process32NextW(snap: Handle, entry: *mut ProcessEntry32W) -> i32;
}

/// 取某个进程的父进程可执行文件名（如 `explorer.exe`，小写）。
///
/// 一次快照把「pid → (父 pid, 名字)」全读出来再走链，比
/// `NtQueryInformationProcess` 稳 —— 后者结构体布局随系统版本变。
fn process_table() -> Vec<(u32, u32, String)> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap.is_null() || snap == INVALID_HANDLE_VALUE {
            return Vec::new();
        }
        let mut rows: Vec<(u32, u32, String)> = Vec::new();
        let mut e: ProcessEntry32W = std::mem::zeroed();
        e.dw_size = std::mem::size_of::<ProcessEntry32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e) != 0;
        while ok {
            let end = e.sz_exe_file.iter().position(|c| *c == 0).unwrap_or(0);
            let name = String::from_utf16_lossy(&e.sz_exe_file[..end]).to_lowercase();
            rows.push((e.th32_process_id, e.th32_parent_process_id, name));
            e.dw_size = std::mem::size_of::<ProcessEntry32W>() as u32;
            ok = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
        rows
    }
}

/// 父进程 pid。拿不到（快照失败 / pid 不存在）返回 `None`。
pub fn parent_pid(pid: u32) -> Option<u32> {
    process_table().iter().find(|(p, _, _)| *p == pid).map(|(_, pp, _)| *pp)
}

pub fn parent_exe_name(pid: u32) -> Option<String> {
    let rows = process_table();
    let ppid = rows.iter().find(|(p, _, _)| *p == pid)?.1;
    rows.iter().find(|(p, _, _)| *p == ppid).map(|(_, _, n)| n.clone())
}

/// 进程创建时间（`FILETIME` 的 100ns 刻度）。拿不到返回 `None`。
pub fn process_start_ticks(pid: u32) -> Option<u64> {
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let (mut created, mut exited, mut kernel, mut user) = (0u64, 0u64, 0u64, 0u64);
        let ok = GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user);
        CloseHandle(h);
        if ok == 0 {
            None
        } else {
            Some(created)
        }
    }
}

// ============ 托盘图标（shell32）============
//
// 为什么要自己调 `Shell_NotifyIconW`，而不是用 nwg 的 `TrayNotification`：
//
// 托盘图标的「显示/折叠」状态，Windows 是按 **exe 路径** 记的。MSIX 每次更新
// 都把程序装进带版本号的新目录（`...\Sonder.RBattery_1.2.1.0_x64__<hash>\`），
// 于是新路径在系统眼里是个全新程序 → 又落回溢出区，用户看到的就是
// 「更新之后托盘又被折叠掉」。
//
// 给图标注册一个**固定 GUID**（`NIF_GUID`）就能让系统按 GUID 记状态，
// 跨版本保持用户的固定/折叠选择。而 nwg 不暴露 `guidItem` 字段，所以这里
// 自己发通知消息。

pub const NIM_ADD: u32 = 0x0000_0000;
pub const NIM_MODIFY: u32 = 0x0000_0001;
pub const NIM_DELETE: u32 = 0x0000_0002;
/// 设托盘版本。不设也能显示，但拿不到 `NIN_*` 通知、菜单定位会退化。
pub const NIM_SETVERSION: u32 = 0x0000_0004;

pub const NIF_MESSAGE: u32 = 0x0000_0001;
pub const NIF_ICON: u32 = 0x0000_0002;
pub const NIF_TIP: u32 = 0x0000_0004;
pub const NIF_GUID: u32 = 0x0000_0020;

/// `GUID`。`#[repr(C)]` 布局与 `guiddef.h` 一致。
///
/// 类型本身定义在文件上方（`SpDeviceInterfaceData` 那一段要用），这里只补一条
/// 说明：托盘图标的 `guidItem` 字段就是它。

/// 本程序的托盘图标 GUID —— **一旦发布就不能改**，改了等于换了个新图标。
pub const TRAY_GUID: Guid = Guid {
    data1: 0x9A7C_3E10,
    data2: 0x4B21,
    data3: 0x4E9C,
    data4: [0x9B, 0x2A, 0x11, 0x7D, 0xC4, 0x35, 0x8F, 0x60],
};

/// 第 `slot` 个托盘图标的 GUID。
///
/// `slot == 0` 就是历史沿用的 [`TRAY_GUID`] —— 升级前的折叠状态挂在它上面，
/// 不能改；其余槽位只动最后一个字节，保证互不相同。
pub fn tray_guid_at(slot: u8) -> Guid {
    let mut g = TRAY_GUID;
    g.data4[7] = TRAY_GUID.data4[7].wrapping_add(slot);
    g
}

/// `NOTIFYICONDATAW`（x64 上 976 字节）。
///
/// `timeout_or_version` 对应 C 里的匿名 union（`uTimeout` / `uVersion`）。
/// 字段顺序必须与 `shellapi.h` 完全一致，`cb_size` 交给
/// [`notify_icon_data`] 填，别手写。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NotifyIconDataW {
    pub cb_size: u32,
    pub hwnd: Handle,
    pub uid: u32,
    pub flags: u32,
    pub callback_message: u32,
    pub hicon: Handle,
    pub sz_tip: [u16; 128],
    pub state: u32,
    pub state_mask: u32,
    pub sz_info: [u16; 256],
    pub timeout_or_version: u32,
    pub sz_info_title: [u16; 64],
    pub info_flags: u32,
    pub guid_item: Guid,
    pub balloon_icon: Handle,
}

#[link(name = "shell32")]
extern "system" {
    pub fn Shell_NotifyIconW(message: u32, data: *const NotifyIconDataW) -> i32;
}

/// 造一份填好 `cb_size`/`hwnd`/`guid` 的通知结构。其余字段由调用方按需填。
fn notify_icon_data(hwnd: Handle, callback_message: u32, guid: Guid) -> NotifyIconDataW {
    // SAFETY: 全零是 `NOTIFYICONDATAW` 的合法初值（所有字段都是 POD）。
    let mut n: NotifyIconDataW = unsafe { std::mem::zeroed() };
    n.cb_size = std::mem::size_of::<NotifyIconDataW>() as u32;
    n.hwnd = hwnd;
    n.callback_message = callback_message;
    n.guid_item = guid;
    n
}

fn copy_tip(dst: &mut [u16; 128], tip: &str) {
    let mut i = 0;
    for c in tip.encode_utf16() {
        if i + 1 >= dst.len() {
            break; // 留一个位置给结尾的 NUL
        }
        dst[i] = c;
        i += 1;
    }
    dst[i] = 0;
}

/// 托盘图标的身份。
///
/// 优先按固定 GUID 记（Windows 按 GUID 记住用户把图标放在哪，MSIX 换目录
/// 也不丢）。但 Windows 10 上只要同 GUID 被旧进程留下幽灵项，`NIM_ADD`
/// 会一直失败、任务栏什么都不出现 —— 所以加不上时退回「窗口 + 编号」，
/// 至少图标还在，只是失去跨版本的位置记忆。
#[derive(Clone, Copy, Debug)]
pub struct TrayId {
    pub guid: Guid,
    pub uid: u32,
    pub uses_guid: bool,
}

pub fn tray_rect(hwnd: Handle, id: TrayId) -> Option<Rect> {
    #[repr(C)]
    struct Identifier {
        size: u32,
        hwnd: Handle,
        uid: u32,
        guid: Guid,
    }
    #[link(name = "shell32")]
    extern "system" {
        fn Shell_NotifyIconGetRect(id: *const Identifier, rect: *mut Rect) -> i32;
    }
    let identifier = Identifier {
        size: std::mem::size_of::<Identifier>() as u32,
        hwnd,
        uid: id.uid,
        guid: if id.uses_guid {
            id.guid
        } else {
            unsafe { std::mem::zeroed() }
        },
    };
    let mut rect = Rect::default();
    (unsafe { Shell_NotifyIconGetRect(&identifier, &mut rect) } == 0).then_some(rect)
}

#[link(name = "user32")]
extern "system" {
    pub fn GetDoubleClickTime() -> u32;
    pub fn SetTimer(hwnd: Handle, id: usize, elapse_ms: u32, proc: usize) -> usize;
    pub fn KillTimer(hwnd: Handle, id: usize) -> i32;
}

/// 非 GUID 模式用的编号，挑一个不会和别的程序撞的值。
pub const TRAY_FALLBACK_UID: u32 = 1;

impl TrayId {
    /// 按身份填字段：GUID 模式只看 `guidItem`，另一模式只看 `uID`。
    fn apply(self, n: &mut NotifyIconDataW) {
        if self.uses_guid {
            n.flags |= NIF_GUID;
            n.guid_item = self.guid;
            n.uid = self.uid;
        } else {
            n.uid = self.uid;
        }
    }
}

/// 设一次托盘版本（`uVersion = 4`）。
fn tray_set_version(hwnd: Handle, id: TrayId) -> bool {
    let mut n = notify_icon_data(hwnd, 0, id.guid);
    id.apply(&mut n);
    n.timeout_or_version = 4;
    // SAFETY: `n` 已按 `shellapi.h` 布局初始化，指针只在调用期间有效。
    unsafe { Shell_NotifyIconW(NIM_SETVERSION, &n) != 0 }
}

/// 添加第 `slot` 个托盘图标。返回实际用上的身份，`None` 表示两种模式都没加上。
///
/// 多设备时每台一个图标，靠 `slot` 区分：`0` 是主图标（沿用 [`TRAY_GUID`]），
/// 其余按 [`tray_guid_at`] 派生，非 GUID 模式下用 `uID = TRAY_FALLBACK_UID + slot`。
pub fn tray_add(
    hwnd: Handle,
    callback_message: u32,
    hicon: Handle,
    tip: &str,
    slot: u8,
) -> Option<TrayId> {
    let guid = tray_guid_at(slot);
    let mut n = notify_icon_data(hwnd, callback_message, guid);
    n.flags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_GUID;
    n.uid = TRAY_FALLBACK_UID + slot as u32;
    n.hicon = hicon;
    copy_tip(&mut n.sz_tip, tip);
    // SAFETY: 同上。
    let mut ok = unsafe { Shell_NotifyIconW(NIM_ADD, &n) != 0 };
    if !ok {
        // 上一个进程异常退出时，同 GUID 的幽灵项还挂在任务栏上，此时
        // `NIM_ADD` 会一直失败 —— 先删掉再试一次。
        let mut d = notify_icon_data(hwnd, 0, guid);
        d.flags = NIF_GUID;
        // SAFETY: 同上。
        unsafe { Shell_NotifyIconW(NIM_DELETE, &d) };
        // SAFETY: 同上。
        ok = unsafe { Shell_NotifyIconW(NIM_ADD, &n) != 0 };
    }
    if ok {
        let id = TrayId {
            guid,
            uid: TRAY_FALLBACK_UID + slot as u32,
            uses_guid: true,
        };
        tray_set_version(hwnd, id);
        return Some(id);
    }

    let id = TrayId { guid, uid: TRAY_FALLBACK_UID + slot as u32, uses_guid: false };
    let mut p = notify_icon_data(hwnd, callback_message, guid);
    p.flags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    p.uid = id.uid;
    p.hicon = hicon;
    copy_tip(&mut p.sz_tip, tip);
    // SAFETY: 同上。
    if unsafe { Shell_NotifyIconW(NIM_ADD, &p) } == 0 {
        return None;
    }
    tray_set_version(hwnd, id);
    Some(id)
}

/// 改图标和/或提示文字（都传 `None` 等于只刷新一次，不改内容）。
pub fn tray_update(hwnd: Handle, id: TrayId, hicon: Option<Handle>, tip: Option<&str>) -> bool {
    let mut n = notify_icon_data(hwnd, 0, id.guid);
    id.apply(&mut n);
    if let Some(h) = hicon {
        n.flags |= NIF_ICON;
        n.hicon = h;
    }
    if let Some(t) = tip {
        n.flags |= NIF_TIP;
        copy_tip(&mut n.sz_tip, t);
    }
    // SAFETY: 同上。
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &n) != 0 }
}

/// 移除托盘图标。退出时调用，否则图标会变成「幽灵」留在任务栏上。
pub fn tray_remove(hwnd: Handle, id: TrayId) -> bool {
    let mut n = notify_icon_data(hwnd, 0, id.guid);
    id.apply(&mut n);
    // SAFETY: 同上。
    unsafe { Shell_NotifyIconW(NIM_DELETE, &n) != 0 }
}

/// 注册一个自定义窗口消息，取回消息号。
///
/// 现在只用一次：`"TaskbarCreated"` —— 资源管理器重启后所有托盘图标都会消失，
/// 系统会**广播**这条消息给所有顶层窗口，收到就得重新 `NIM_ADD`。
/// 它在 `user32`，返回值是消息 ID（同一名字全系统一致，0 表示失败）。
pub fn register_window_message(name: &str) -> u32 {
    #[link(name = "user32")]
    extern "system" {
        fn RegisterWindowMessageW(name: *const u16) -> u32;
    }
    let w = crate::win32::to_wide(name);
    // SAFETY: `w` 是以 NUL 结尾的宽字符串，调用期间一直有效。
    unsafe { RegisterWindowMessageW(w.as_ptr()) }
}

// ============ user32 ============
#[link(name = "user32")]
extern "system" {    /// 查找顶层窗口（单实例检测用）。
    pub fn FindWindowW(class: *const u16, title: *const u16) -> Handle;
    /// 把已有窗口置顶激活。
    pub fn SetForegroundWindow(hwnd: Handle) -> i32;
    /// 显示/还原窗口。
    pub fn ShowWindow(hwnd: Handle, cmd: i32) -> i32;

    /// 简单的消息框。GUI 还没成型时用来告诉用户「这里还没做好」，
    /// 免得双击 exe 后什么都不发生。
    pub fn MessageBoxW(hwnd: Handle, text: *const u16, caption: *const u16, flags: u32) -> i32;

    /// 取离窗口最近的显示器。无边框窗口最大化要它来查工作区。
    pub fn MonitorFromWindow(hwnd: Handle, flags: u32) -> Handle;
    /// 读显示器信息。**`MonitorInfo.cb_size` 必须先填**，否则返回 0。
    pub fn GetMonitorInfoW(monitor: Handle, out: *mut MonitorInfo) -> i32;

    // ---- 自绘（托盘图标）----
    //
    // `GetDC` / `ReleaseDC` / `FillRect` / `DrawTextW` 在 **user32**，不在 gdi32
    // —— 这条容易记错，记错的表现是链接期 `undefined reference`。
    //
    // 内容区（`src/ui/canvas.rs`）**不在这里取 DC**：它自己声明了
    // `BeginPaint` / `EndPaint` / `InvalidateRect` / `GetClientRect`，
    // 因为那边全程用 `Gdi = isize` 而这里是 `Handle = *mut c_void`，
    // 两份声明签名不一致会触发 `clashing_extern_declarations`（只警告不报错，
    // 一旦调错那个是运行时崩溃）。两边各管一段，不要互相复用。
    pub fn FillRect(hdc: Handle, rect: *const Rect, brush: Handle) -> i32;
    /// 拿屏幕 DC。托盘图标自绘用它当「格式模板」
    /// （`CreateCompatibleDC` 要一个参考 DC，否则内存位图的色彩格式可能和屏幕不一致）。
    pub fn GetDC(hwnd: Handle) -> Handle;
    pub fn ReleaseDC(hwnd: Handle, hdc: Handle) -> i32;

    /// 自建的 HICON **必须** `DestroyIcon`：图标句柄属于 User 对象，
    /// 不回收会一直涨（每刷新一次多一个，几十次就能看出来）。
    pub fn DestroyIcon(icon: Handle) -> i32;

    /// 从 exe 资源里取图标。`name` 传 `1 as *const u16`（MAKEINTRESOURCE）。
    pub fn LoadImageW(
        hinst: Handle,
        name: *const u16,
        typ: u32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> Handle;

    pub fn GetModuleHandleW(name: *const u16) -> Handle;
    pub fn SendMessageW(hwnd: Handle, msg: u32, wparam: usize, lparam: isize) -> isize;
    /// 取系统度量（`SM_CXSMICON = 49`），按托盘实际尺寸绘制。
    pub fn GetSystemMetrics(index: i32) -> i32;
    pub fn ModifyMenuW(menu: Handle, item: u32, flags: u32, new_id: usize, text: *const u16) -> i32;

    /// 窗口是否处于最小化。最小化后不再重绘，可据此回收渲染内存。
    pub fn IsIconic(hwnd: Handle) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    pub fn GetUserDefaultUILanguage() -> u16;
    pub fn FreeLibrary(module: Handle) -> i32;
}

/// `LoadImageW` 的图标类型。
pub const IMAGE_ICON: u32 = 1;

/// `WM_SETICON` 的 wParam：0 = 小图标（标题栏/alt+tab），1 = 大图标。
pub const ICON_SMALL: usize = 0;
pub const ICON_BIG: usize = 1;

/// 从 exe 资源里取应用图标（资源 1）。失败返回空句柄。
pub fn load_app_icon(size: i32) -> Handle {
    unsafe {
        LoadImageW(
            GetModuleHandleW(std::ptr::null()),
            1 as *const u16,
            IMAGE_ICON,
            size,
            size,
            0,
        )
    }
}

/// 给窗口装上任务栏 / alt+tab 用的图标。
///
/// 窗口类没设图标、窗口自己也没设过的话，alt+tab 里就是一块空白 ——
/// 必须显式 `WM_SETICON`，系统不会自动回落到 exe 图标。
pub fn set_window_icon(hwnd: Handle, big: Handle, small: Handle) {
    unsafe {
        if !big.is_null() {
            SendMessageW(hwnd, 0x0080, ICON_BIG, big as isize);
        }
        if !small.is_null() {
            SendMessageW(hwnd, 0x0080, ICON_SMALL, small as isize);
        }
    }
}

// ============ gdi32 ============
//
// self-contained 的 rustlib 里有 `libgdi32.a`（750,604 B），所以这条路能链接。
//
// 这一块**只服务托盘图标那 32x32 的小图**（`src/tray_icon.rs`）：
// 画刷 + 字体 + `CreateDIBSection` 包成 HICON。
//
// 界面内容不走这里，走 GDI+（`src/ui/gfx.rs`，`#[link(name = "gdiplus")]`，
// import library 从 `vendor/mingw-import-libs/` 补）。两边刻意分开：
// GDI+ 的句柄是 `Gdi = isize`，这里是 `Handle = *mut c_void`，
// 混用会触发 `clashing_extern_declarations`。
#[link(name = "gdi32")]
extern "system" {
    pub fn CreateSolidBrush(color: u32) -> Handle;
    pub fn DeleteObject(obj: Handle) -> i32;
    pub fn SelectObject(hdc: Handle, obj: Handle) -> Handle;

    // ---- 托盘图标自绘 ----
    //
    // 目标是「在托盘上显示电量数字」：GDI 画到内存位图 → 包成 HICON →
    // `TrayNotification::set_icon`。
    //
    // 注意 `CreateDIBSection` 在 **gdi32**，但 `CreateIconIndirect` 在 **user32**。
    pub fn CreateCompatibleDC(hdc: Handle) -> Handle;
    pub fn DeleteDC(hdc: Handle) -> i32;
}

// ============ setupapi ============
#[link(name = "setupapi")]
extern "system" {
    pub fn SetupDiGetClassDevsW(
        class_guid: *const Guid,
        enumerator: *const u16,
        parent: Handle,
        flags: u32,
    ) -> Handle;

    pub fn SetupDiEnumDeviceInterfaces(
        dev_info: Handle,
        dev_info_data: *mut c_void,
        class_guid: *const Guid,
        index: u32,
        data: *mut SpDeviceInterfaceData,
    ) -> i32;

    pub fn SetupDiGetDeviceInterfaceDetailW(
        dev_info: Handle,
        data: *mut SpDeviceInterfaceData,
        detail: *mut c_void,
        detail_size: u32,
        required: *mut u32,
        dev_info_data: *mut c_void,
    ) -> i32;

    pub fn SetupDiDestroyDeviceInfoList(dev_info: Handle) -> i32;
}

// ============ hid.dll（运行时加载）============
// 注意返回类型：`BOOLEAN` 是 1 字节（BYTE），不是 4 字节的 `BOOL`。
// 用 u8 承接才能保证只读 AL 寄存器，不会被 EAX 高位的垃圾影响。
pub type FnHidDGetHidGuid = unsafe extern "system" fn(*mut Guid);
pub type FnHidDGetAttributes = unsafe extern "system" fn(Handle, *mut HiddAttributes) -> u8;
pub type FnHidDGetPreparsedData = unsafe extern "system" fn(Handle, *mut *mut c_void) -> u8;
pub type FnHidDFreePreparsedData = unsafe extern "system" fn(*mut c_void) -> u8;
pub type FnHidPGetCaps = unsafe extern "system" fn(*mut c_void, *mut HidpCaps) -> i32;
pub type FnHidDGetProductString = unsafe extern "system" fn(Handle, *mut u16, u32) -> u8;
pub type FnHidDGetSerialNumberString = unsafe extern "system" fn(Handle, *mut u16, u32) -> u8;
pub type FnHidDGetFeature = unsafe extern "system" fn(Handle, *mut u8, u32) -> u8;

/// hid.dll 的函数表。进程内只加载一次（见 [`HidApi::load`]）。
///
/// `Copy` 是故意的：表里全是函数指针与模块句柄，复制一份是零成本的，
/// 这样调用方拿到的是「缓存表的副本」而不是引用，签名不用改。
///
/// 模块句柄**故意不释放**：多个 `HidDevice` 可能还持有这里的函数指针，
/// `FreeLibrary` 会在退出路径上踩空；进程结束时由系统回收即可。
#[derive(Clone, Copy)]
pub struct HidApi {
    _module: Handle,
    pub get_hid_guid: FnHidDGetHidGuid,
    pub get_attributes: FnHidDGetAttributes,
    pub get_preparsed_data: FnHidDGetPreparsedData,
    pub free_preparsed_data: FnHidDFreePreparsedData,
    pub get_caps: FnHidPGetCaps,
    pub get_product_string: FnHidDGetProductString,
    pub get_serial_number_string: FnHidDGetSerialNumberString,
    pub get_feature: FnHidDGetFeature,
}

// 表里只有模块句柄与函数指针，跨线程共享是安全的（hid.dll 的导出函数无状态）。
unsafe impl Send for HidApi {}
unsafe impl Sync for HidApi {}

/// 进程级缓存。用 `Mutex<Option<_>>` 而不是 `OnceLock<Result<_>>`：
/// 加载失败（比如 hid.dll 被临时挡住）不能变成永久失败，要允许下次重试。
static HID_API: std::sync::Mutex<Option<HidApi>> = std::sync::Mutex::new(None);

impl HidApi {
    /// 取 hid.dll 函数表。首次调用加载并缓存，之后只复制一份。
    pub fn load() -> Result<HidApi, String> {
        let mut g = HID_API.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(api) = *g {
            return Ok(api);
        }
        let api = unsafe { Self::load_uncached() }?; // 失败故意不写缓存
        *g = Some(api);
        Ok(api)
    }

    unsafe fn load_uncached() -> Result<HidApi, String> {
        unsafe {
            let name = to_wide("hid.dll");
            let module = LoadLibraryW(name.as_ptr());
            if module.is_null() || module == INVALID_HANDLE_VALUE {
                return Err(format!("无法加载 hid.dll（GetLastError={}）", GetLastError()));
            }

            fn sym(module: Handle, name: &[u8]) -> *mut c_void {
                unsafe { GetProcAddress(module, name.as_ptr()) }
            }

            macro_rules! take {
                ($name:literal, $ty:ty) => {{
                    let p = sym(module, concat!($name, "\0").as_bytes());
                    if p.is_null() {
                        return Err(format!("hid.dll 缺少导出函数 {}", $name));
                    }
                    core::mem::transmute::<*mut c_void, $ty>(p)
                }};
            }

            Ok(HidApi {
                _module: module,
                get_hid_guid: take!("HidD_GetHidGuid", FnHidDGetHidGuid),
                get_attributes: take!("HidD_GetAttributes", FnHidDGetAttributes),
                get_preparsed_data: take!("HidD_GetPreparsedData", FnHidDGetPreparsedData),
                free_preparsed_data: take!("HidD_FreePreparsedData", FnHidDFreePreparsedData),
                get_caps: take!("HidP_GetCaps", FnHidPGetCaps),
                get_product_string: take!("HidD_GetProductString", FnHidDGetProductString),
                get_serial_number_string: take!(
                    "HidD_GetSerialNumberString",
                    FnHidDGetSerialNumberString
                ),
                get_feature: take!("HidD_GetFeature", FnHidDGetFeature),
            })
        }
    }
}

// ============ 小工具 ============
/// Rust 字符串转以 NUL 结尾的 UTF-16（Win32 W 系列 API 用）。
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

/// 取 last-error 并清掉（格式化成人类可读的错误码）。
pub fn last_error() -> u32 {
    unsafe { GetLastError() }
}

/// 句柄是否可用（既不是 NULL 也不是 `INVALID_HANDLE_VALUE`）。
fn handle_ok(h: Handle) -> bool {
    !h.is_null() && h != INVALID_HANDLE_VALUE
}

pub fn attach_console() -> bool {
    unsafe {
        if handle_ok(GetStdHandle(STD_OUTPUT_HANDLE)) {
            return true;
        }
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return false;
        }
        let share = FILE_SHARE_READ | FILE_SHARE_WRITE;
        let out = to_wide("CONOUT$");
        let h_out = CreateFileW(
            out.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            share,
            core::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            core::ptr::null_mut(),
        );
        if handle_ok(h_out) {
            SetStdHandle(STD_OUTPUT_HANDLE, h_out);
            SetStdHandle(STD_ERROR_HANDLE, h_out);
        }
        let inp = to_wide("CONIN$");
        let h_in = CreateFileW(
            inp.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            share,
            core::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            core::ptr::null_mut(),
        );
        if handle_ok(h_in) {
            SetStdHandle(STD_INPUT_HANDLE, h_in);
        }
        true
    }
}

// 注册表读写（`advapi32`）。
//
// 原来 `autostart.rs` 与 `ui/theme.rs` 各声明了一份 —— 同一批 Win32 函数
// 写两遍，参数名/类型一漂就是 UB（`extern` 声明不做类型检查，只信你写的）。
// 现在只有这里一份，两边 `use crate::win32::{...}`。
// （`extern` 块不接收文档注释，写了会报 `unused doc comment`。）
#[link(name = "advapi32")]
extern "system" {
    pub fn RegOpenKeyExW(key: usize, sub: *const u16, opts: u32, sam: u32, out: *mut usize) -> i32;
    pub fn RegCreateKeyExW(
        key: usize,
        sub: *const u16,
        reserved: u32,
        class: *const u16,
        opts: u32,
        sam: u32,
        sec: *mut c_void,
        out: *mut usize,
        disposition: *mut u32,
    ) -> i32;
    pub fn RegSetValueExW(
        key: usize,
        name: *const u16,
        reserved: u32,
        ty: u32,
        data: *const u8,
        len: u32,
    ) -> i32;
    pub fn RegQueryValueExW(
        key: usize,
        name: *const u16,
        reserved: *mut u32,
        ty: *mut u32,
        data: *mut u8,
        len: *mut u32,
    ) -> i32;
    pub fn RegDeleteValueW(key: usize, name: *const u16) -> i32;
    pub fn RegCloseKey(key: usize) -> i32;
}
