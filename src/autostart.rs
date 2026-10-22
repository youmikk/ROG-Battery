//! 开机自启：MSIX 包内走 WinRT `StartupTask`，绿色版走 `HKCU\...\Run`。
//!
//! **这两条路径不能混用**，判据是「有没有包身份」，不是「WinRT 是否可用」：
//!
//! - MSIX 应用写 `HKCU\...\Run` 会被注册表虚拟化重定向进包私有 hive
//!   （`%LOCALAPPDATA%\Packages\<Identity>\SystemAppData\Helium\User.dat`），
//!   系统的启动项列表里**看不到** → 自启静默失效。
//! - 绿色版没有包身份，调 WinRT `StartupTask.GetAsync` 返回 `0x80070490`
//!   （`ERROR_ELEMENT_NOT_FOUND`），因为 TaskId 只在「有包身份 **且**
//!   manifest 里声明了 `startupTask`」的环境中存在。
//!
//! 完整的实测过程见 `docs/autostart.md`，这里只记实现要点。
//!
//! # 三个致命坑（都是真崩过一次才记住的）
//!
//! 1. **必须 `RoInitialize(1)`**。不调的话 `RoGetActivationFactory` 返回
//!    `0x800401F0`（`CO_E_NOTINITIALIZED`）。这个错误码**不代表没有包身份**
//!    —— 包外调了 `RoInitialize` 之后工厂照样能拿到 `S_OK`。
//! 2. **combase 必须动态加载**。rustlib 的 `self-contained` 里没有
//!    `libkernelbase.a` / `libcombase.a`，`#[link(name = "combase")]` 会
//!    `ld: cannot find -lcombase`。走 `LoadLibraryW` + `GetProcAddress`，
//!    与 `hid.dll` 的做法一致。
//! 3. **WinRT 接口继承 IInspectable，不是 IUnknown**，vtable 比 COM 多 3 个槽
//!    （`GetIids` / `GetRuntimeClassName` / `GetTrustLevel`）。数错一格就是
//!    运行时 `0xC0000005`，而且崩点跟错点毫无关系 —— 编译期完全不校验。
//!
//! # 另一个坑：IID 不要自己算
//!
//! 网上流传的「SHA1(接口名大写) 前 16 字节 + 改第 6/8 字节」算法**是错的**。
//! 用 `IActivationFactory` 对照验算，算出来是 `00000043-00fe-5d9d-...`，
//! 真实值是 `af86e2e0-b12d-4c6a-9c5a-d7aa65101e90`，完全不符。
//!
//! 正确来源是系统 WinRT 元数据：
//!
//! ```powershell
//! [Type]::GetType("Windows.ApplicationModel.IStartupTaskStatics, Windows.ApplicationModel, ContentType=WindowsRuntime").GUID
//!   → ee5b60bd-a148-41a7-b26e-e8b88a1e62f8
//! [Type]::GetType("Windows.ApplicationModel.IStartupTask, Windows.ApplicationModel, ContentType=WindowsRuntime").GUID
//!   → f75c23c8-b5f2-4f6c-88dd-36cb1d599d17
//! ```
//!
//! 运行时亦可自证：对工厂调 `GetIids`（vtable `[3]`），返回的第二个 IID
//! 就是 `ee5b60bd-...`。

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

use core::ffi::c_void;
use std::time::{Duration, Instant};

use crate::win32::{self, to_wide};

// ============================================================
// 对外类型
// ============================================================

/// 开机自启的当前状态。
///
/// **不要简化成 `bool`**：`DisabledByUser` 和 `DisabledByPolicy` 都是「没启用」，
/// 但用户点「开启」时行为完全不同 —— 前者再请求会**静默失败**（系统记着用户拒绝过，
/// 必须让用户去「设置 → 应用 → 启动」手动开），后者是组策略禁止、程序无解。
/// 合并成 bool 就没法给用户正确的提示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoStartState {
    /// 未启用，可以请求启用。
    Disabled,
    /// 用户明确拒绝过。**再 `enable()` 会静默失败**，只能引导用户去系统设置里开。
    DisabledByUser,
    /// 已启用。
    Enabled,
    /// 组策略禁止，程序无解。
    DisabledByPolicy,
    /// 策略强制启用。
    EnabledByPolicy,
    /// 读不到 / 调用失败。
    Unknown,
}

impl AutoStartState {
    /// 是否处于「已启用」状态（含策略强制启用）。
    pub fn is_enabled(self) -> bool {
        matches!(self, AutoStartState::Enabled | AutoStartState::EnabledByPolicy)
    }

    /// 给用户看的中文说明。
    pub fn describe(self) -> &'static str {
        match self {
            AutoStartState::Disabled => "未启用",
            AutoStartState::DisabledByUser => "已被用户关闭（需在系统设置里手动开启）",
            AutoStartState::Enabled => "已启用",
            AutoStartState::DisabledByPolicy => "已被组策略禁止",
            AutoStartState::EnabledByPolicy => "已由组策略启用",
            AutoStartState::Unknown => "状态未知",
        }
    }

    fn from_winrt(state: i32) -> AutoStartState {
        match state {
            0 => AutoStartState::Disabled,
            1 => AutoStartState::DisabledByUser,
            2 => AutoStartState::Enabled,
            3 => AutoStartState::DisabledByPolicy,
            4 => AutoStartState::EnabledByPolicy,
            _ => AutoStartState::Unknown,
        }
    }
}

/// 重新启用时的结果，比单纯 `Ok(())` 多带信息 —— 因为 WinRT 那条路
/// **成功也可能没真的启用**（用户拒绝时 `RequestEnableAsync` 返回
/// `DisabledByUser` 而不是报错）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnableOutcome {
    /// 最终状态。
    pub state: AutoStartState,
    /// 是否确实启用了（`state.is_enabled()`）。
    pub enabled: bool,
    /// 走的是哪条路径，用于日志与排错。
    pub path: AutoStartPath,
}

/// 自启走的是哪条路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoStartPath {
    /// MSIX 包内：WinRT `StartupTask`。
    WinRt,
    /// 绿色版：`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`。
    Registry,
    /// 读不到包身份（理论上不该发生）。
    Unknown,
}

impl AutoStartPath {
    /// 短名，直接给界面用。
    ///
    /// 曾经的 `describe()` 返回「绿色版（HKCU\...\Run）」这种长串，
    /// 但 UI 上那个标签只有 275 px，长串会被截断 —— 短名 + 失败时的完整解释
    /// 更实用，所以长版本删掉了（不留死代码）。
    pub fn short(self) -> &'static str {
        match self {
            AutoStartPath::WinRt => "MSIX 包内",
            AutoStartPath::Registry => "注册表",
            AutoStartPath::Unknown => "路径未知",
        }
    }
}

// ============================================================
// 包身份判定
// ============================================================

const APPMODEL_ERROR_NO_PACKAGE: i32 = 15700;
const ERROR_INSUFFICIENT_BUFFER: i32 = 122;

// `GetCurrentPackageFullName`：kernel32 直接导出（内部转发到 KernelBase，
// 但导入库里有这条转发记录，`ld` 能解，不需要 libkernelbase.a）。
//
// 注意：extern 块**不能**用 `///` 文档注释（编译器报 unused doc comment），
// 只能写成 `//` 普通注释。
#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentPackageFullName(len: *mut u32, name: *mut u16) -> i32;
}

/// 当前进程的包身份。`Some(true)` = MSIX 包内，`Some(false)` = 绿色版，
/// `None` = 查询本身失败（调不通 API，理论上不该发生）。
///
/// 为什么要三态而不是 `bool`：原先写的是 `rc != APPMODEL_ERROR_NO_PACKAGE`，
/// 那样**任何错误**（拒绝访问、API 缺失）都被当成「有包身份」，
/// 于是绿色版会错误地去等 WinRT，报一个和真实原因无关的失败。
/// 只有明确拿到 `122`（`ERROR_INSUFFICIENT_BUFFER`，说明有包但缓冲区不够）
/// 才算真有包身份。
fn package_identity() -> Option<bool> {
    let mut len: u32 = 0;
    let rc = unsafe { GetCurrentPackageFullName(&mut len, core::ptr::null_mut()) };
    if rc == APPMODEL_ERROR_NO_PACKAGE {
        Some(false)
    } else if rc == 0 || rc == ERROR_INSUFFICIENT_BUFFER {
        Some(true)
    } else {
        None
    }
}

/// 当前进程是否有 MSIX 包身份。
///
/// 实测：绿色版返回 `15700`（`APPMODEL_ERROR_NO_PACKAGE`）；
/// 有包身份且 `len` 传 `null` 时返回 `122`（`ERROR_INSUFFICIENT_BUFFER`）。
///
/// 查询失败时返回 `false`（当作绿色版）—— `autostart::detect_path()` 那条路
/// 用 [`package_identity`] 单独区分出「未知」，本函数只服务测试与快速判定。
#[cfg(test)]
pub fn is_packaged() -> bool {
    package_identity().unwrap_or(false)
}

/// 判定该走哪条自启路径。
///
/// 包身份**查询失败**时返回 `Unknown`，让上层显示「无法判定运行形态」，
/// 而不是瞎猜一条路径然后失败在别的地方（见 [`package_identity`]）。
pub fn detect_path() -> AutoStartPath {
    match package_identity() {
        Some(true) => AutoStartPath::WinRt,
        Some(false) => AutoStartPath::Registry,
        None => AutoStartPath::Unknown,
    }
}

// ============================================================
// 绿色版：注册表
// ============================================================

const HKEY_CURRENT_USER: usize = 0x8000_0001;
const KEY_QUERY_VALUE: u32 = 0x0001;
const KEY_SET_VALUE: u32 = 0x0002;
const REG_SZ: u32 = 1;
const ERROR_SUCCESS: i32 = 0;
const ERROR_FILE_NOT_FOUND: i32 = 2;

const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// 值名。用 `RogMouseCtl` 而不是 `R Battery`：注册表值名允许空格但
/// 命令行工具/脚本处理时容易漏引号。
const RUN_VALUE_NAME: &str = "RogMouseCtl";
/// 启动时传给程序的参数。GUI 是默认形态（无参数），这个参数的作用是
/// **让 `main` 把它和裸跑同等处理**（见 `src/main.rs` 的入口分流），
/// 同时便于在任务管理器里认出这条启动项。
///
/// 用 `pub` 是因为 `main.rs` 的入口分流要拿它做比较 —— 若把字符串在
/// `main.rs` 里再写一遍，两边一旦不同步就会变成「自启勾了但开机没反应」，
/// 而且编译期不会报错。
pub const RUN_ARG: &str = "--autostart";

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(key: usize, sub: *const u16, opts: u32, sam: u32, out: *mut usize) -> i32;
    fn RegCreateKeyExW(
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
    fn RegSetValueExW(key: usize, name: *const u16, reserved: u32, ty: u32, data: *const u8, len: u32) -> i32;
    fn RegQueryValueExW(
        key: usize,
        name: *const u16,
        reserved: *mut u32,
        ty: *mut u32,
        data: *mut u8,
        len: *mut u32,
    ) -> i32;
    fn RegDeleteValueW(key: usize, name: *const u16) -> i32;
    fn RegCloseKey(key: usize) -> i32;
}

/// 当前 exe 的完整路径，包上引号。路径含空格时**必须**加引号，否则系统按空格
/// 切分参数会把 `C:\Program Files\...` 切成两段。
fn quoted_exe_path() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("取 exe 路径失败：{e}"))?;
    let s = exe.to_string_lossy().to_string();
    Ok(format!("\"{s}\" {RUN_ARG}"))
}

/// 读注册表里已登记的命令行。没有条目返回 `Ok(None)`。
fn read_run_value() -> Result<Option<String>, String> {
    unsafe {
        let sub = to_wide(RUN_SUBKEY);
        let mut hk: usize = 0;
        let rc = RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_QUERY_VALUE, &mut hk);
        if rc != ERROR_SUCCESS {
            // 键不存在也算「没启用」，不该报错
            return Ok(None);
        }

        let name = to_wide(RUN_VALUE_NAME);
        let mut ty: u32 = 0;
        let mut buf = vec![0u16; 2048];
        let mut blen: u32 = (buf.len() * 2) as u32;
        let rc = RegQueryValueExW(
            hk,
            name.as_ptr(),
            core::ptr::null_mut(),
            &mut ty,
            buf.as_mut_ptr() as *mut u8,
            &mut blen,
        );
        RegCloseKey(hk);

        match rc {
            ERROR_SUCCESS => {
                let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                Ok(Some(String::from_utf16_lossy(&buf[..end])))
            }
            ERROR_FILE_NOT_FOUND => Ok(None),
            other => Err(format!("读 Run 键失败：Win32 错误码 {other}")),
        }
    }
}

/// 绿色版：查状态。
///
/// **不能只看键存在** —— 用户可能改过路径、或者 exe 被移动过。
/// 要比对命令行里的 exe 路径是否与当前进程一致，否则会出现「显示已启用
/// 但开机什么也没发生」的情况（旧路径已失效）。
fn registry_state() -> AutoStartState {
    let current = match quoted_exe_path() {
        Ok(s) => s,
        Err(_) => return AutoStartState::Unknown,
    };
    match read_run_value() {
        Ok(Some(v)) => {
            if v.trim() == current.trim() {
                AutoStartState::Enabled
            } else {
                // 有旧条目但指向别的路径（exe 被移动/旧版本残留）。
                // 当成未启用，enable() 会覆盖掉。
                AutoStartState::Disabled
            }
        }
        Ok(None) => AutoStartState::Disabled,
        Err(_) => AutoStartState::Unknown,
    }
}

/// 绿色版：写入 Run 键。
fn registry_enable() -> Result<(), String> {
    let cmd = quoted_exe_path()?;
    unsafe {
        let sub = to_wide(RUN_SUBKEY);
        let mut hk: usize = 0;
        let mut disp: u32 = 0;
        let rc = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            sub.as_ptr(),
            0,
            core::ptr::null(),
            0,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            core::ptr::null_mut(),
            &mut hk,
            &mut disp,
        );
        if rc != ERROR_SUCCESS {
            return Err(format!("打开 Run 键失败：Win32 错误码 {rc}"));
        }

        let name = to_wide(RUN_VALUE_NAME);
        let value = to_wide(&cmd);
        // REG_SZ 的长度必须**含结尾 NUL**，单位是字节
        let bytes = (value.len() * 2) as u32;
        let rc = RegSetValueExW(hk, name.as_ptr(), 0, REG_SZ, value.as_ptr() as *const u8, bytes);
        RegCloseKey(hk);

        if rc != ERROR_SUCCESS {
            return Err(format!("写 Run 键失败：Win32 错误码 {rc}"));
        }
    }
    Ok(())
}

/// 绿色版：删掉 Run 键里的值。值本来就不存在也视为成功。
fn registry_disable() -> Result<(), String> {
    unsafe {
        let sub = to_wide(RUN_SUBKEY);
        let mut hk: usize = 0;
        let rc = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            sub.as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut hk,
        );
        if rc != ERROR_SUCCESS {
            // 键都不存在，那本来就没启用
            return Ok(());
        }
        let name = to_wide(RUN_VALUE_NAME);
        let rc = RegDeleteValueW(hk, name.as_ptr());
        RegCloseKey(hk);

        match rc {
            ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
            other => Err(format!("删 Run 键值失败：Win32 错误码 {other}")),
        }
    }
}

// ============================================================
// MSIX 包内：WinRT StartupTask
// ============================================================

/// manifest 里的 `TaskId`，必须**一字不差**。
/// 见 `msix\AppxManifest.xml.template` 的 `<desktop:StartupTask TaskId="...">`。
const STARTUP_TASK_ID: &str = "RogBatteryAutoStart";

const CO_E_NOTINITIALIZED: u32 = 0x8004_01F0;
const RPC_E_CHANGED_MODE: u32 = 0x8001_0106;
const ERROR_ELEMENT_NOT_FOUND: u32 = 0x8007_0490;

/// `IActivationFactory` 的 IID，固定值。
const IID_ACTIVATION_FACTORY: Guid = Guid {
    data1: 0xAF86_E2E0,
    data2: 0xB12D,
    data3: 0x4C6A,
    data4: [0x9C, 0x5A, 0xD7, 0xAA, 0x65, 0x10, 0x1E, 0x90],
};

/// `IStartupTaskStatics` 的 IID。**不是自己算的**，是从系统 WinRT 元数据查的
/// （见模块文档）。手抄成常量以免每次都去调 PowerShell。
const IID_STARTUP_TASK_STATICS: Guid = Guid {
    data1: 0xEE5B_60BD,
    data2: 0xA148,
    data3: 0x41A7,
    data4: [0xB2, 0x6E, 0xE8, 0xB8, 0x8A, 0x1E, 0x62, 0xF8],
};

/// `IAsyncInfo` 的 IID。用它 QI 出来的对象有**独立、无继承歧义**的 vtable，
/// 轮询状态比在 `IAsyncOperation<T>` 上数槽位安全得多（我在后者身上错过三次）。
const IID_ASYNC_INFO: Guid = Guid {
    data1: 0x0000_0036,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

// ---------------------------------------------------------------------------
// vtable 槽位常量 —— **全部来自 Windows 元数据实测，不是猜的**
//
// 查证方法（PowerShell）：
//   $t = [Type]::GetType("Windows.ApplicationModel.IStartupTask, Windows.ApplicationModel, ContentType=WindowsRuntime")
//   $t.GetMethods() | % { $_.Name }
//
// 规则：WinRT 根接口的方法都从 **`[6]`** 开始（`[0..2]` IUnknown，
// `[3] GetIids` `[4] GetRuntimeClassName` `[5] GetTrustLevel` 是 IInspectable）。
//
// 我早期版本把 IStartupTask 的槽位整体搞错了：以为 `[6]` 是 `get_State`，
// 实际 `[6]` 是 **`RequestEnableAsync`** —— 会把「查状态」变成「弹系统框并真的
// 启用开机自启」。那段代码从未被执行过（WinRT 分支只在包内走），但这是个
// 必须记下来的教训：**vtable 槽位错一格，语义可能从只读变成写入。**
// ---------------------------------------------------------------------------

/// `IStartupTaskStatics`：`[6] GetForCurrentPackageAsync` `[7] GetAsync(taskId)`
const STATICS_GET_ASYNC: usize = 7;

/// `IAsyncInfo`（**独立 vtable**）：`[6] get_Id` `[7] get_Status`
/// `[8] get_ErrorCode` `[9] Cancel` `[10] Close`
const ASYNC_INFO_GET_STATUS: usize = 7;
const ASYNC_INFO_GET_ERROR_CODE: usize = 8;

/// `IAsyncOperation<T>`：COM 规则是**派生接口在前**，所以
/// `[6] put_Completed` `[7] get_Completed` `[8] GetResults`，
/// `[9..13]` 才是继承来的 `IAsyncInfo` 那五个。
///
/// 注意 `GetResults` 在 **`[8]`**（不是 `[11]`）。我一开始按「基接口在前」
/// 推断成 `[11]`，结果调到了 `IAsyncInfo::get_Status`，只好又改回来。
const ASYNC_GET_RESULTS: usize = 8;

/// `IStartupTask`：`[6] RequestEnableAsync` `[7] Disable`
/// `[8] get_State` `[9] get_TaskId`
const TASK_REQUEST_ENABLE: usize = 6;
const TASK_DISABLE: usize = 7;
const TASK_GET_STATE: usize = 8;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

type HResult = i32;

type FnRoInitialize = unsafe extern "system" fn(i32) -> HResult;
type FnRoUninitialize = unsafe extern "system" fn();
type FnWindowsCreateString = unsafe extern "system" fn(*const u16, u32, *mut *mut c_void) -> HResult;
type FnWindowsDeleteString = unsafe extern "system" fn(*mut c_void) -> HResult;
type FnRoGetActivationFactory =
    unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> HResult;

type FnRelease = unsafe extern "system" fn(*mut c_void) -> u32;
type FnQueryInterface = unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> HResult;

/// combase.dll 的函数表，进程内只加载一次。
struct RoApi {
    _module: win32::Handle,
    ro_initialize: FnRoInitialize,
    ro_uninitialize: FnRoUninitialize,
    create_string: FnWindowsCreateString,
    delete_string: FnWindowsDeleteString,
    get_factory: FnRoGetActivationFactory,
}

impl RoApi {
    fn load() -> Result<RoApi, String> {
        unsafe {
            let name = to_wide("combase.dll");
            let module = win32::LoadLibraryW(name.as_ptr());
            if module.is_null() || module == win32::INVALID_HANDLE_VALUE {
                return Err(format!("无法加载 combase.dll（GetLastError={}）", win32::last_error()));
            }

            fn sym(module: win32::Handle, name: &str) -> *mut c_void {
                let c = to_wide(name);
                // GetProcAddress 要的是 ANSI 名字（带 NUL）
                let ansi: Vec<u8> = c
                    .iter()
                    .map(|&u| if u < 128 { u as u8 } else { b'?' })
                    .collect();
                unsafe { win32::GetProcAddress(module, ansi.as_ptr()) }
            }

            macro_rules! take {
                ($name:literal, $ty:ty) => {{
                    let p = sym(module, $name);
                    if p.is_null() {
                        return Err(format!("combase.dll 缺少导出函数 {}", $name));
                    }
                    core::mem::transmute::<*mut c_void, $ty>(p)
                }};
            }

            Ok(RoApi {
                _module: module,
                ro_initialize: take!("RoInitialize", FnRoInitialize),
                ro_uninitialize: take!("RoUninitialize", FnRoUninitialize),
                create_string: take!("WindowsCreateString", FnWindowsCreateString),
                delete_string: take!("WindowsDeleteString", FnWindowsDeleteString),
                get_factory: take!("RoGetActivationFactory", FnRoGetActivationFactory),
            })
        }
    }
}

/// 取 vtable 里的第 `index` 个函数指针。
///
/// # Safety
///
/// `obj` 必须是一个活的 COM/WinRT 对象指针，且 `index` 必须落在该接口
/// （**含 IInspectable 的 3 个槽**）的真实方法范围内。写错一格不会编译报错，
/// 只会运行时崩。
unsafe fn vtable_call<T>(obj: *mut c_void, index: usize) -> T {
    let vt = *(obj as *mut *const usize);
    core::mem::transmute_copy::<usize, T>(&*vt.add(index))
}

/// 一次 WinRT 会话。`Drop` 时调 `RoUninitialize`。
///
/// 注意 `RoInitialize` 的失败要区分：`RPC_E_CHANGED_MODE`（`0x80010106`）
/// 表示该线程已用别的套间模型初始化过，**不算失败**，可以继续用。
struct RoSession {
    api: RoApi,
    needs_uninit: bool,
}

impl RoSession {
    fn open() -> Result<RoSession, String> {
        let api = RoApi::load()?;
        let rc = unsafe { (api.ro_initialize)(1) }; // RO_INIT_MULTITHREADED
        let u = rc as u32;
        let needs_uninit = if u == 0 {
            true
        } else if u == RPC_E_CHANGED_MODE {
            false // 别人已经初始化过，不归我们 uninit
        } else if u == CO_E_NOTINITIALIZED {
            return Err("RoInitialize 返回 CO_E_NOTINITIALIZED（不该发生）".to_string());
        } else {
            return Err(format!("RoInitialize 失败：0x{u:08X}"));
        };
        Ok(RoSession { api, needs_uninit })
    }

    /// 建 HSTRING。返回的句柄必须用 `delete_string` 释放。
    fn hstring(&self, s: &str) -> Result<*mut c_void, String> {
        let units: Vec<u16> = s.encode_utf16().collect();
        let mut hs: *mut c_void = core::ptr::null_mut();
        let rc = unsafe { (self.api.create_string)(units.as_ptr(), units.len() as u32, &mut hs) };
        if rc != 0 {
            return Err(format!("WindowsCreateString 失败：0x{:08X}", rc as u32));
        }
        Ok(hs)
    }

    fn free_hstring(&self, hs: *mut c_void) {
        if !hs.is_null() {
            unsafe {
                (self.api.delete_string)(hs);
            }
        }
    }

    /// 拿 `IStartupTask` 对象（调用方负责 `Release`）。
    ///
    /// vtable 槽位（**都含 IInspectable 的 3 个槽**）：
    /// - `factory[0]` = QueryInterface → `IStartupTaskStatics`
    /// - `statics[6]` = `GetAsync`（`[3]` 是 `GetIids`，别搞错）
    /// - `op[7]` = `get_Status`，`op[8]` = `get_ErrorCode`，`op[11]` = `GetResults`
    unsafe fn activate(&self, task_id: &str) -> Result<*mut c_void, String> {
        let class = "Windows.ApplicationModel.StartupTask";
        let hs_class = self.hstring(class)?;

        let mut factory: *mut c_void = core::ptr::null_mut();
        let rc = (self.api.get_factory)(hs_class, &IID_ACTIVATION_FACTORY, &mut factory);
        self.free_hstring(hs_class);
        if rc != 0 {
            let u = rc as u32;
            let hint = if u == CO_E_NOTINITIALIZED {
                "（忘了 RoInitialize）"
            } else {
                ""
            };
            return Err(format!("RoGetActivationFactory 失败：0x{u:08X}{hint}"));
        }

        // QI 到 IStartupTaskStatics
        let qi: FnQueryInterface = vtable_call(factory, 0);
        let mut statics: *mut c_void = core::ptr::null_mut();
        let rc = qi(factory, &IID_STARTUP_TASK_STATICS, &mut statics);
        let release_factory: FnRelease = vtable_call(factory, 2);
        release_factory(factory);
        if rc != 0 {
            return Err(format!("QueryInterface(IStartupTaskStatics) 失败：0x{:08X}", rc as u32));
        }

        // GetAsync。**槽位是 [7]**，`[6]` 是 `GetForCurrentPackageAsync()`
        // （不接受 taskId —— 错用会让 hs_id 被当成 out 指针写坏，rc 却是 S_OK）。
        let hs_id = self.hstring(task_id)?;
        type FnGetAsync = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HResult;
        let get_async: FnGetAsync = vtable_call(statics, STATICS_GET_ASYNC);
        let mut op: *mut c_void = core::ptr::null_mut();
        let rc = get_async(statics, hs_id, &mut op);
        self.free_hstring(hs_id);
        let release_statics: FnRelease = vtable_call(statics, 2);
        release_statics(statics);

        if rc != 0 {
            let u = rc as u32;
            let hint = if u == ERROR_ELEMENT_NOT_FOUND {
                "（TaskId 不存在：绿色版，或 manifest 没声明 startupTask）"
            } else {
                ""
            };
            return Err(format!("StartupTask.GetAsync 失败：0x{u:08X}{hint}"));
        }
        if op.is_null() {
            // 槽位错的时候就是这个症状：rc = S_OK 但 out 指针没被写。
            return Err("StartupTask.GetAsync 成功但返回了空 IAsyncOperation（vtable 槽位可能不对）".to_string());
        }

        // 等完成。照 C# 的 `.Wait(5000)`：轮询 get_Status，间隔 20ms。
        //
        // **先 QI 到 `IAsyncInfo`**，用它自己的独立 vtable 取 `get_Status`。
        // 为什么不直接在 `IAsyncOperation<T>` 上数槽位：那个接口继承 IAsyncInfo，
        // COM 规则是「派生在前、基接口在后」，`get_Status` 落在 `[10]`；我在这上面
        // 连错了三次（先当 [4]→崩、再当 [7]→拿到 get_Completed 的 null handler
        // →status 恒 0 永不完成、再当 [10] 才对）。QI 一次把歧义彻底消掉。
        let qi_info: FnQueryInterface = vtable_call(op, 0);
        let mut info: *mut c_void = core::ptr::null_mut();
        let rc = qi_info(op, &IID_ASYNC_INFO, &mut info);
        if rc != 0 || info.is_null() {
            let release_op: FnRelease = vtable_call(op, 2);
            release_op(op);
            return Err(format!("QueryInterface(IAsyncInfo) 失败：0x{:08X}", rc as u32));
        }

        type FnGetStatus = unsafe extern "system" fn(*mut c_void, *mut i32) -> HResult;
        let get_status: FnGetStatus = vtable_call(info, ASYNC_INFO_GET_STATUS);
        let deadline = Instant::now() + Duration::from_millis(5000);
        let mut status: i32 = 0;
        loop {
            if Instant::now() >= deadline {
                let release_info: FnRelease = vtable_call(info, 2);
                release_info(info);
                let release_op: FnRelease = vtable_call(op, 2);
                release_op(op);
                return Err("StartupTask.GetAsync 超时（5 秒未完成）".to_string());
            }
            let rc = get_status(info, &mut status);
            if rc != 0 {
                let release_info: FnRelease = vtable_call(info, 2);
                release_info(info);
                let release_op: FnRelease = vtable_call(op, 2);
                release_op(op);
                return Err(format!("IAsyncInfo.get_Status 失败：0x{:08X}", rc as u32));
            }
            if status != 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        match status {
            1 => {
                // Completed → GetResults。**槽位是 [8]**（派生接口在前）。
                type FnGetResults = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HResult;
                let get_results: FnGetResults = vtable_call(op, ASYNC_GET_RESULTS);
                let mut task: *mut c_void = core::ptr::null_mut();
                let rc = get_results(op, &mut task);
                let release_info: FnRelease = vtable_call(info, 2);
                release_info(info);
                let release_op: FnRelease = vtable_call(op, 2);
                release_op(op);
                if rc != 0 {
                    return Err(format!("GetResults 失败：0x{:08X}", rc as u32));
                }
                Ok(task)
            }
            2 => {
                let release_info: FnRelease = vtable_call(info, 2);
                release_info(info);
                let release_op: FnRelease = vtable_call(op, 2);
                release_op(op);
                Err("StartupTask.GetAsync 被取消".to_string())
            }
            3 => {
                type FnGetError = unsafe extern "system" fn(*mut c_void, *mut HResult) -> HResult;
                let get_err: FnGetError = vtable_call(info, ASYNC_INFO_GET_ERROR_CODE);
                let mut e: HResult = 0;
                let _ = get_err(info, &mut e);
                let release_info: FnRelease = vtable_call(info, 2);
                release_info(info);
                let release_op: FnRelease = vtable_call(op, 2);
                release_op(op);
                Err(format!("StartupTask.GetAsync 出错：0x{:08X}", e as u32))
            }
            other => {
                let release_info: FnRelease = vtable_call(info, 2);
                release_info(info);
                let release_op: FnRelease = vtable_call(op, 2);
                release_op(op);
                Err(format!("StartupTask.GetAsync 异常状态（status={other}）"))
            }
        }
    }

    /// `IStartupTask::get_State`。**只读**，槽位 `[8]`。
    unsafe fn get_state(&self, task: *mut c_void) -> Result<AutoStartState, String> {
        type FnGetState = unsafe extern "system" fn(*mut c_void, *mut i32) -> HResult;
        let f: FnGetState = vtable_call(task, TASK_GET_STATE);
        let mut st: i32 = -1;
        let rc = f(task, &mut st);
        if rc != 0 {
            return Err(format!("IStartupTask.get_State 失败：0x{:08X}", rc as u32));
        }
        Ok(AutoStartState::from_winrt(st))
    }

    /// `IStartupTask::RequestEnableAsync`。
    ///
    /// 会**弹系统确认框**。用户点「否」时不报错，而是返回 `DisabledByUser`
    /// —— 所以调用方必须检查返回值而不是只看 `Ok`。
    unsafe fn request_enable(&self, task: *mut c_void) -> Result<AutoStartState, String> {
        // IStartupTask: [6] get_State [7] get_TaskId [8] RequestEnableAsync [9] Disable
        // RequestEnableAsync 的 ABI 与 GetAsync 一样是 (this, out IAsyncOperation)
        type FnRequestEnable = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HResult;
        // **槽位 [6] 是 RequestEnableAsync**。我早期版本这里写的是 `[6]` 当
        // `get_State` —— 那会把「查状态」变成「弹系统框并真的启用自启」。
        let f: FnRequestEnable = vtable_call(task, TASK_REQUEST_ENABLE);
        let mut op: *mut c_void = core::ptr::null_mut();
        let rc = f(task, &mut op);
        if rc != 0 {
            return Err(format!("RequestEnableAsync 失败：0x{:08X}", rc as u32));
        }
        if op.is_null() {
            return Err("RequestEnableAsync 返回了空 IAsyncOperation".to_string());
        }

        // 同样等完成。RequestEnableAsync 返回 IAsyncOperation<StartupTaskState>，
        // GetResults 出来的是 enum（int）而不是对象。
        //
        // 和 `activate` 一样先 QI 到 IAsyncInfo 拿干净的 `get_Status`。
        let qi_info: FnQueryInterface = vtable_call(op, 0);
        let mut info: *mut c_void = core::ptr::null_mut();
        let rc = qi_info(op, &IID_ASYNC_INFO, &mut info);
        if rc != 0 || info.is_null() {
            let release_op: FnRelease = vtable_call(op, 2);
            release_op(op);
            return Err(format!("RequestEnableAsync QI(IAsyncInfo) 失败：0x{:08X}", rc as u32));
        }

        type FnGetStatus = unsafe extern "system" fn(*mut c_void, *mut i32) -> HResult;
        let get_status: FnGetStatus = vtable_call(info, ASYNC_INFO_GET_STATUS);
        let deadline = Instant::now() + Duration::from_millis(5000);
        let mut status: i32 = 0;
        loop {
            if Instant::now() >= deadline {
                let release_info: FnRelease = vtable_call(info, 2);
                release_info(info);
                let release_op: FnRelease = vtable_call(op, 2);
                release_op(op);
                return Err("RequestEnableAsync 超时（5 秒未完成）".to_string());
            }
            let rc = get_status(info, &mut status);
            if rc != 0 {
                let release_info: FnRelease = vtable_call(info, 2);
                release_info(info);
                let release_op: FnRelease = vtable_call(op, 2);
                release_op(op);
                return Err(format!("RequestEnableAsync 的 get_Status 失败：0x{:08X}", rc as u32));
            }
            if status != 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        let result = if status == 1 {
            // IAsyncOperation<enum T> 的 GetResults 直接写 int。槽位 [8]。
            type FnGetResultsInt = unsafe extern "system" fn(*mut c_void, *mut i32) -> HResult;
            let get_results: FnGetResultsInt = vtable_call(op, ASYNC_GET_RESULTS);
            let mut v: i32 = -1;
            let rc = get_results(op, &mut v);
            if rc != 0 {
                Err(format!("RequestEnableAsync 的 GetResults 失败：0x{:08X}", rc as u32))
            } else {
                Ok(AutoStartState::from_winrt(v))
            }
        } else {
            Err(format!("RequestEnableAsync 未完成（status={status}）"))
        };

        let release_info: FnRelease = vtable_call(info, 2);
        release_info(info);
        let release_op: FnRelease = vtable_call(op, 2);
        release_op(op);
        result
    }

    /// `IStartupTask::Disable`。无需确认，同步返回。**槽位是 [7]**
    /// （我早期写成 `[9]`，那是 `get_TaskId`）。
    unsafe fn disable(&self, task: *mut c_void) -> Result<(), String> {
        type FnDisable = unsafe extern "system" fn(*mut c_void) -> HResult;
        let f: FnDisable = vtable_call(task, TASK_DISABLE);
        let rc = f(task);
        if rc != 0 {
            return Err(format!("IStartupTask.Disable 失败：0x{:08X}", rc as u32));
        }
        Ok(())
    }
}

impl Drop for RoSession {
    fn drop(&mut self) {
        if self.needs_uninit {
            unsafe {
                (self.api.ro_uninitialize)();
            }
        }
    }
}

/// 包内：查状态。
fn winrt_state() -> Result<AutoStartState, String> {
    let session = RoSession::open()?;
    unsafe {
        let task = session.activate(STARTUP_TASK_ID)?;
        let st = session.get_state(task);
        let release: FnRelease = vtable_call(task, 2);
        release(task);
        st
    }
}

// ============================================================
// 对外接口
// ============================================================

/// 查当前自启状态。**永不返回 `Err`** —— 读不到就返回 `Unknown`，
/// 因为「查状态失败」不该阻止 GUI 渲染，界面上显示「状态未知」即可。
pub fn state() -> (AutoStartState, AutoStartPath) {
    let path = detect_path();
    let st = match path {
        AutoStartPath::WinRt => winrt_state().unwrap_or(AutoStartState::Unknown),
        AutoStartPath::Registry => registry_state(),
        AutoStartPath::Unknown => AutoStartState::Unknown,
    };
    (st, path)
}

/// 启用开机自启。
pub fn enable() -> Result<EnableOutcome, String> {
    let path = detect_path();
    match path {
        AutoStartPath::Registry => {
            registry_enable()?;
            // 写完立刻回读，不信「写成功就代表生效」
            let st = registry_state();
            Ok(EnableOutcome {
                enabled: st.is_enabled(),
                state: st,
                path,
            })
        }
        AutoStartPath::WinRt => {
            let session = RoSession::open()?;
            let new_state = unsafe {
                let task = session.activate(STARTUP_TASK_ID)?;
                let r = session.request_enable(task);
                let release: FnRelease = vtable_call(task, 2);
                release(task);
                r
            };
            // 用户拒绝时 request_enable 返回 DisabledByUser，不是 Err
            let st = new_state.unwrap_or(AutoStartState::Unknown);
            Ok(EnableOutcome {
                enabled: st.is_enabled(),
                state: st,
                path,
            })
        }
        AutoStartPath::Unknown => Err("无法判定运行形态（包身份读取失败）".to_string()),
    }
}

/// 禁用开机自启。
pub fn disable() -> Result<EnableOutcome, String> {
    let path = detect_path();
    match path {
        AutoStartPath::Registry => {
            registry_disable()?;
            let st = registry_state();
            Ok(EnableOutcome {
                enabled: st.is_enabled(),
                state: st,
                path,
            })
        }
        AutoStartPath::WinRt => {
            let session = RoSession::open()?;
            unsafe {
                let task = session.activate(STARTUP_TASK_ID)?;
                let r = session.disable(task);
                let st = match session.get_state(task) {
                    Ok(s) => s,
                    Err(_) => AutoStartState::Unknown,
                };
                let release: FnRelease = vtable_call(task, 2);
                release(task);
                r?;
                Ok(EnableOutcome {
                    enabled: st.is_enabled(),
                    state: st,
                    path,
                })
            }
        }
        AutoStartPath::Unknown => Err("无法判定运行形态（包身份读取失败）".to_string()),
    }
}

// ============================================================
// 单测
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_describe_覆盖全部变体() {
        for s in [
            AutoStartState::Disabled,
            AutoStartState::DisabledByUser,
            AutoStartState::Enabled,
            AutoStartState::DisabledByPolicy,
            AutoStartState::EnabledByPolicy,
            AutoStartState::Unknown,
        ] {
            assert!(!s.describe().is_empty(), "{s:?} 缺描述");
        }
    }

    #[test]
    fn is_enabled_只认两种() {
        assert!(AutoStartState::Enabled.is_enabled());
        assert!(AutoStartState::EnabledByPolicy.is_enabled());
        assert!(!AutoStartState::Disabled.is_enabled());
        assert!(!AutoStartState::DisabledByUser.is_enabled());
        assert!(!AutoStartState::DisabledByPolicy.is_enabled());
        assert!(!AutoStartState::Unknown.is_enabled());
    }

    #[test]
    fn winrt_状态码映射() {
        assert_eq!(AutoStartState::from_winrt(0), AutoStartState::Disabled);
        assert_eq!(AutoStartState::from_winrt(1), AutoStartState::DisabledByUser);
        assert_eq!(AutoStartState::from_winrt(2), AutoStartState::Enabled);
        assert_eq!(AutoStartState::from_winrt(3), AutoStartState::DisabledByPolicy);
        assert_eq!(AutoStartState::from_winrt(4), AutoStartState::EnabledByPolicy);
        assert_eq!(AutoStartState::from_winrt(99), AutoStartState::Unknown);
    }

    #[test]
    fn 绿色版必然探测到注册表路径() {
        // 测试进程是 cargo 起的，没有包身份
        assert!(!is_packaged(), "测试进程不该有包身份");
        assert_eq!(detect_path(), AutoStartPath::Registry);
    }

    #[test]
    fn iid_常量与系统元数据一致() {
        let f = IID_ACTIVATION_FACTORY;
        let s = format!(
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            f.data1, f.data2, f.data3,
            f.data4[0], f.data4[1], f.data4[2], f.data4[3],
            f.data4[4], f.data4[5], f.data4[6], f.data4[7]
        );
        assert_eq!(s, "af86e2e0-b12d-4c6a-9c5a-d7aa65101e90");

        let t = IID_STARTUP_TASK_STATICS;
        let s = format!(
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            t.data1, t.data2, t.data3,
            t.data4[0], t.data4[1], t.data4[2], t.data4[3],
            t.data4[4], t.data4[5], t.data4[6], t.data4[7]
        );
        assert_eq!(s, "ee5b60bd-a148-41a7-b26e-e8b88a1e62f8");

        let i = IID_ASYNC_INFO;
        let s = format!(
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            i.data1, i.data2, i.data3,
            i.data4[0], i.data4[1], i.data4[2], i.data4[3],
            i.data4[4], i.data4[5], i.data4[6], i.data4[7]
        );
        assert_eq!(s, "00000036-0000-0000-c000-000000000046");
    }

    /// 锁死 vtable 槽位常量。
    ///
    /// **这是我犯过的最大一个错误**：早期把 `IStartupTask` 的槽位整体搞错，
    /// 以为 `[6]` 是 `get_State`，实际 `[6]` 是 `RequestEnableAsync` ——
    /// 「查状态」会变成「弹系统框并真的启用开机自启」。
    /// 参数类型几乎一样（都是 `(this, out ...)`），所以编译器毫无怨言。
    ///
    /// 这组数字来自 Windows 元数据实测：
    /// ```powershell
    /// $t = [Type]::GetType("Windows.ApplicationModel.IStartupTask, Windows.ApplicationModel, ContentType=WindowsRuntime")
    /// $t.GetMethods() | % { $_.Name }
    /// # → RequestEnableAsync, Disable, get_State, get_TaskId
    /// ```
    /// 本测试不能连设备（包外 WinRT 拿不到 TaskId），
    /// 所以只能把「已知正确值」钉死在这里，改错立刻红。
    #[test]
    fn vtable_槽位常量固定() {
        // IStartupTaskStatics: [6] GetForCurrentPackageAsync, [7] GetAsync
        assert_eq!(STATICS_GET_ASYNC, 7, "GetAsync 在 [7]，[6] 是 GetForCurrentPackageAsync");

        // IStartupTask: [6] RequestEnableAsync, [7] Disable, [8] get_State, [9] get_TaskId
        assert_eq!(TASK_REQUEST_ENABLE, 6);
        assert_eq!(TASK_DISABLE, 7, "Disable 在 [7]，不是 [9]（[9] 是 get_TaskId）");
        assert_eq!(TASK_GET_STATE, 8, "get_State 在 [8]，不是 [6]（[6] 是 RequestEnableAsync）");

        // IAsyncInfo（独立 vtable）: [6] get_Id, [7] get_Status, [8] get_ErrorCode
        assert_eq!(ASYNC_INFO_GET_STATUS, 7);
        assert_eq!(ASYNC_INFO_GET_ERROR_CODE, 8);

        // IAsyncOperation<T>（派生在前）: [6] put_Completed, [7] get_Completed, [8] GetResults
        assert_eq!(ASYNC_GET_RESULTS, 8, "GetResults 在 [8]，不是 [11]");
    }

    /// 六个槽位必须两两不同 —— 只要出现重复就说明我把某个接口的方法数错了。
    #[test]
    fn vtable_槽位不重复() {
        let task_slots = [TASK_REQUEST_ENABLE, TASK_DISABLE, TASK_GET_STATE];
        for i in 0..task_slots.len() {
            for j in (i + 1)..task_slots.len() {
                assert_ne!(
                    task_slots[i], task_slots[j],
                    "IStartupTask 有两个方法映射到同一槽位 [{}]",
                    task_slots[i]
                );
            }
        }
        let info_slots = [ASYNC_INFO_GET_STATUS, ASYNC_INFO_GET_ERROR_CODE];
        assert_ne!(info_slots[0], info_slots[1], "IAsyncInfo 槽位重复");
    }

    #[test]
    fn 包外查状态不panic() {
        // 绿色版走注册表分支，不该 panic
        let (st, path) = state();
        assert_eq!(path, AutoStartPath::Registry);
        // 状态只可能是 Enabled / Disabled / Unknown（本机当前是未启用）
        assert!(
            matches!(
                st,
                AutoStartState::Enabled | AutoStartState::Disabled | AutoStartState::Unknown
            ),
            "绿色版不该出现 {st:?}"
        );
    }

    /// 真机往返：`enable()` → 确认 Run 键真出现 → `disable()` → 确认真消失。
    ///
    /// **默认 `#[ignore]`**，两个理由：
    /// 1. 它会真的写用户的 `HKCU\...\Run`（虽然立刻删掉）；
    /// 2. 多个测试进程并发跑会互相干扰。
    ///
    /// 手动跑：
    /// `cargo test --release autostart_真机往返 -- --ignored --nocapture`
    #[test]
    #[ignore = "会真实读写 HKCU\\...\\Run，需手动 --ignored 触发"]
    fn autostart_真机往返() {
        if is_packaged() {
            println!("跳过：本进程有包身份，走 WinRT 路径，无法在绿色版测试里验注册表");
            return;
        }

        println!("改动前状态：{:?}", state().0);

        // panic 也要把 Run 键恢复原状，否则会把用户机器留在「自启已开」的状态
        struct Guard {
            armed: bool,
        }
        impl Drop for Guard {
            fn drop(&mut self) {
                if self.armed {
                    let _ = disable();
                }
            }
        }
        let mut guard = Guard { armed: true };

        // ---- 开 ----
        let on = enable().expect("enable 失败");
        println!("enable  → state={:?} enabled={} path={:?}", on.state, on.enabled, on.path);
        assert!(on.enabled, "enable 之后应该 enabled");
        assert_eq!(on.path, AutoStartPath::Registry);

        // 直接读注册表确认真写进去了 —— 不信 enable() 自己的返回值
        let raw = read_run_value().expect("读 Run 键失败").expect("enable 之后 Run 键应该有值");
        println!("Run 键实际内容：{raw}");
        assert!(raw.contains("rog-mouse-ctl"), "命令行里应该有 exe 名，实际 {raw}");
        assert!(raw.contains(RUN_ARG), "应该带 {RUN_ARG} 参数，实际 {raw}");
        assert!(raw.starts_with('"'), "路径应该被引号包起来（含空格的路径必需），实际 {raw}");

        // 再查一次状态，确认读到的是 Enabled
        assert_eq!(state().0, AutoStartState::Enabled, "回读状态应该是 Enabled");

        // ---- 关 ----
        let off = disable().expect("disable 失败");
        println!("disable → state={:?} enabled={}", off.state, off.enabled);
        assert!(!off.enabled, "disable 之后应该不 enabled");

        let after = read_run_value().expect("读 Run 键失败");
        assert!(after.is_none(), "disable 之后 Run 键应该没有值了，实际 {after:?}");

        // 幂等性：再 disable 一次不该报错（值本来就不存在）
        disable().expect("重复 disable 应该成功（幂等）");

        guard.armed = false;
        println!("往返测试通过，已恢复原状");
    }
}
