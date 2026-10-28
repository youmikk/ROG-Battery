//! 手写 Win32 FFI 声明。
//!
//! 刻意**不依赖** `windows-sys` / `winapi`：整个工程零外部 crate，`cargo build` 不需要访问 crates.io。
//!
//! 链接方式（实测确认）：
//! - `kernel32` / `setupapi` 在 MinGW 的 rustlib 里有导入库（`libsetupapi.a` 等），直接静态链接；
//! - `hid.dll` **没有** `libhid.a`，因此走 `LoadLibraryW` + `GetProcAddress` 动态取函数指针。
//!
//! 全部使用 Unicode 版本的 API（`CreateFileW`、`SetupDiGetClassDevsW` 等）。

#![allow(non_snake_case, non_camel_case_types)]

use core::ffi::c_void;

pub type Handle = *mut c_void;

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
    #[allow(dead_code)]
    pub fn GetTickCount() -> u32;

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

/// `PAINTSTRUCT`。**x64 上是 72 字节**（`hdc` 8 + `fErase` 4 + 对齐 0 + `rcPaint` 16
/// + `fRestore` 4 + `fIncUpdate` 4 + `rgbReserved[32]`，尾部补到 8 字节对齐）。
///
/// 这个结构**必须自己声明**：nwg 的 `PaintData::begin_paint()` 返回的是 winapi 的
/// `PAINTSTRUCT`，而 winapi 不是本工程的直接依赖（见文件头「零外部 crate」），
/// 拿不到那个类型名。下面的单测钉死了大小和字段偏移，改错会当场失败。
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

/// `ICONINFO`。`CreateIconIndirect` 要它。
///
/// **必须五个字段**，一个都不能少：
/// ```c
/// typedef struct { BOOL fIcon; DWORD xHotspot; DWORD yHotspot;
///                  HBITMAP hbmMask; HBITMAP hbmColor; } ICONINFO;
/// ```
/// 少了 `xHotspot`/`yHotspot` 两个字段会怎样？`CreateIconIndirect`
/// 直接把 `hbmColor` 那 8 字节读成热点坐标，于是把 `hbmMask` 当成
/// `hbmColor`、把 `hbmColor` 当成越界垃圾 —— **返回 NULL，`GetLastError` = 203
/// （`ERROR_ENVVAR_NOT_FOUND`，完全是误导性的错误码）**。
/// 实测：3 字段版恒失败，补上两个 DWORD 立刻成功。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct IconInfo {
    /// 1 = 图标，0 = 光标。托盘要图标，固定填 1。
    pub is_icon: i32,
    /// 光标热点。图标用不到，填 0。
    pub x_hotspot: i32,
    pub y_hotspot: i32,
    pub mask: Handle,
    pub color: Handle,
}

/// `BITMAPINFOHEADER`（`BITMAPINFO` 的头部，后面紧跟调色板）。
///
/// 32bpp 的 DIB 不需要调色板，所以只申请 header 就够
/// —— 但 `biSizeImage` 必须给对，否则某些驱动会拒绝。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct BitmapInfoHeader {
    pub size: u32,
    pub width: i32,
    pub height: i32,
    /// 必须是 1。
    pub planes: u16,
    /// 32 = BGRA，每像素自带 alpha。
    pub bit_count: u16,
    /// 0 = 不压缩。
    pub compression: u32,
    pub size_image: u32,
    pub x_pels_per_meter: i32,
    pub y_pels_per_meter: i32,
    pub clr_used: u32,
    pub clr_important: u32,
}

/// `BITMAPINFO` = header + 1 个调色板项（32bpp 时不用，但要留出空间）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct BitmapInfo {
    pub header: BitmapInfoHeader,
    pub colors: [u32; 3],
}

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
    /// 取系统度量。托盘自绘只用一个值：`SM_CXSMICON`（49）——
    /// 系统在通知区里**实际**用的小图标边长。
    ///
    /// 不查这个值、硬画 32x32 也能work，但系统会把 32 缩到 16，
    /// 双线性缩放会把笔画糊掉（实测「82」边缘发虚）。
    /// 按原生尺寸画就完全避开缩放。
    pub fn GetSystemMetrics(index: i32) -> i32;
    /// 取当前进程的**伪句柄**（不需要 CloseHandle）。
    /// 配合 `GetGuiResources` 读自己的 GDI/USER 对象数，用来测泄漏。
    ///
    /// `cfg(test)`：只有泄漏回归测试用它，正式构建留着就是死代码（会 warning）。
    #[cfg(test)]
    pub fn GetCurrentProcess() -> Handle;
    /// 读进程当前持有的 GDI（`flag = 0`）或 USER（`flag = 1`）对象数。
    ///
    /// 托盘图标自绘的回归测试靠它：`Gdi::drop` 漏一个 `DeleteObject`
    /// 这个数字就会单调上涨。
    #[cfg(test)]
    pub fn GetGuiResources(process: Handle, flag: u32) -> u32;
    /// `CreateIconIndirect` 在 **user32**（不是 gdi32）。
    /// 用它把内存位图 + 掩码合成 HICON。
    pub fn CreateIconIndirect(info: *const IconInfo) -> Handle;
    /// 自建的 HICON **必须** `DestroyIcon`：图标句柄属于 User 对象，
    /// 不回收会一直涨（每刷新一次多一个，几十次就能看出来）。
    pub fn DestroyIcon(icon: Handle) -> i32;
    pub fn DrawTextW(
        hdc: Handle,
        text: *const u16,
        len: i32,
        rect: *mut Rect,
        format: u32,
    ) -> i32;
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
    pub fn SetBkMode(hdc: Handle, mode: i32) -> i32;
    pub fn SetTextColor(hdc: Handle, color: u32) -> u32;
    pub fn CreateFontW(
        height: i32,
        width: i32,
        escapement: i32,
        orientation: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strike_out: u32,
        char_set: u32,
        out_precision: u32,
        clip_precision: u32,
        quality: u32,
        pitch_and_family: u32,
        face: *const u16,
    ) -> Handle;

    // ---- 托盘图标自绘 ----
    //
    // 目标是「在托盘上显示电量数字」：GDI 画到内存位图 → 包成 HICON →
    // `TrayNotification::set_icon`。
    //
    // 注意 `CreateDIBSection` 在 **gdi32**，但 `CreateIconIndirect` 在 **user32**。
    pub fn CreateCompatibleDC(hdc: Handle) -> Handle;
    pub fn CreateDIBSection(
        hdc: Handle,
        bmi: *const BitmapInfo,
        usage: u32,
        bits: *mut *mut c_void,
        section: Handle,
        offset: u32,
    ) -> Handle;
    pub fn DeleteDC(hdc: Handle) -> i32;
    pub fn CreateBitmap(w: i32, h: i32, planes: u32, bits_per_pixel: u32, bits: *const c_void)
        -> Handle;
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

/// hid.dll 的函数表。进程内只加载一次。
pub struct HidApi {
    _module: Handle,
    pub get_hid_guid: FnHidDGetHidGuid,
    pub get_attributes: FnHidDGetAttributes,
    pub get_preparsed_data: FnHidDGetPreparsedData,
    pub free_preparsed_data: FnHidDFreePreparsedData,
    pub get_caps: FnHidPGetCaps,
    pub get_product_string: FnHidDGetProductString,
    pub get_serial_number_string: FnHidDGetSerialNumberString,
}

impl HidApi {
    pub fn load() -> Result<HidApi, String> {
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

/// 把父进程（cmd / PowerShell）的控制台接到本进程上，让 CLI 的输出能看见。
///
/// 本程序的子系统是 `windows`（见 `src/main.rs` 的 `windows_subsystem`）——
/// 双击、从开始菜单、从商店启动时**根本不会创建控制台**，所以不会闪黑框。
/// 代价是命令行调用时进程默认没有控制台，`println!` 无处可写。这里在 `main`
/// 最开头补回来，顺序很关键：
///
/// 1. 先看 `GetStdHandle(STD_OUTPUT_HANDLE)` 是否有效。`cmd /c "x info > r.txt"`
///    这类重定向、`| Out-File`、`Start-Process -RedirectStandardOutput` 都会把
///    句柄继承下来，此时**不能**用 `CONOUT$` 覆盖它，否则那些用法全写成空文件。
/// 2. 没有继承句柄时才 `AttachConsole(ATTACH_PARENT_PROCESS)`，接上父控制台后
///    再把 `CONOUT$` / `CONIN$` 设成标准句柄，`println!` 就落到那个窗口里。
///
/// 双击启动时没有父控制台，第 1、2 步都失败，函数返回 `false`，什么也不做。
///
/// 已知限制（实测，非本函数的锅）：PowerShell 自己的 `>` 操作符把 GUI 子系统
/// 程序的标准输出接成一个**写不进去**的句柄 —— `& x --help > f.txt` 得到 0 字节，
/// 而 `cmd /c "x --help > f.txt"`、`& x --help | Out-File f.txt`、
/// `Start-Process -RedirectStandardOutput` 三种都正常。脚本里重定向请用后三者。
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
