
#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

use core::ffi::c_void;
use std::path::PathBuf;
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

/// 这次启动是不是「开机自启」拉起来的。
///
/// 绿色版不需要猜：Run 键和 Startup 里的 VBS 都显式带 [`RUN_ARG`]。
/// MSIX 版做不到 —— `desktop:StartupTask` 是**系统激活**，命令行里没有参数，
/// 双击图标和开机启动的 argv 完全一样。所以这里看父进程：
///
/// - 用户手动启动（开始菜单 / 桌面快捷方式 / 任务栏）→ 父进程是 `explorer.exe`
/// - 登录时由 StartupTask 拉起 → 父进程是系统激活宿主（`svchost.exe` /
///   `RuntimeBroker.exe` 之类）
///
/// 判不出来（快照失败）时返回 `false`：宁可多弹一次窗口，也不要让用户
/// 点了图标以为没反应。
pub fn launched_at_logon() -> bool {
    if package_identity() != Some(true) {
        return false;
    }
    let me = std::process::id();
    let parent_is_explorer = crate::win32::parent_exe_name(me)
        .map(|n| n.eq_ignore_ascii_case("explorer.exe"))
        .unwrap_or(false);
    let gap = parent_gap_secs(me);
    let autostart_on = state().0.is_enabled();
    let hidden = decide_hidden(parent_is_explorer, autostart_on, gap);
    note_launch(parent_is_explorer, gap, autostart_on, hidden);
    hidden
}

/// 登录自启的时间窗（秒）。
///
/// 为什么需要它：Win11 的 `desktop:StartupTask` 有时由 **explorer.exe** 拉起
/// （和用户双击图标一模一样），光看父进程名分不出来。改成看「我们和父进程谁先
/// 出生」——登录自启时 explorer 刚起来、我们紧跟其后；用户手动点图标时 explorer
/// 已经跑了几小时，差值很大。
const LOGON_WINDOW_SECS: i64 = 120;

pub fn decide_hidden(
    parent_is_explorer: bool,
    autostart_on: bool,
    parent_gap_secs: Option<i64>,
) -> bool {
    // 父进程是系统激活宿主（svchost / RuntimeBroker …）→ 一定是登录自启。
    if !parent_is_explorer {
        return true;
    }
    // 父进程是 explorer：只有「自启开着 + 我们紧跟 explorer 出生」才算。
    match parent_gap_secs {
        Some(gap) if autostart_on && (0..=LOGON_WINDOW_SECS).contains(&gap) => true,
        _ => false,
    }
}

/// 我们比父进程晚出生多少秒（父进程更早为正）。
fn parent_gap_secs(me: u32) -> Option<i64> {
    let parent = crate::win32::parent_pid(me)?;
    let mine = crate::win32::process_start_ticks(me)?;
    let theirs = crate::win32::process_start_ticks(parent)?;
    Some((mine as i64 - theirs as i64) / 10_000_000)
}

/// 把这次启动的判定依据落到 `last-launch.txt`。
///
/// 为什么留这个文件：登录自启的父进程到底是谁，只有用户真重启一次才知道；
/// 没有现场记录就只能猜。写入失败一律忽略 —— 诊断信息不该影响启动。
fn note_launch(parent_is_explorer: bool, gap: Option<i64>, autostart_on: bool, hidden: bool) {
    let path = crate::paths::settings_file().with_file_name("last-launch.txt");
    let line = format!(
        "packaged=yes parent_is_explorer={parent_is_explorer} gap_secs={gap:?} autostart_on={autostart_on} hidden={hidden}\n"
    );
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
}

// ============================================================
// 绿色版：注册表
// ============================================================

const HKEY_CURRENT_USER: usize = 0xFFFF_FFFF_8000_0001;
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

use crate::win32::{RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW};

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

/// 检查 Windows 系统主题（任务栏/开始菜单/系统托盘）是否为浅色。
///
/// 读取注册表 `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\SystemUsesLightTheme`。
/// 值为 1 时系统任务栏为浅色，托盘应显示深黑/深炭色文字；
/// 值为 0（深色模式）或读取失败时，默认返回 false（深色任务栏，显示白色文字）。
pub fn system_uses_light_theme() -> bool {
    let sub = to_wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let mut hk: usize = 0;
    let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_QUERY_VALUE, &mut hk) };
    if rc != ERROR_SUCCESS {
        return false;
    }
    let name = to_wide("SystemUsesLightTheme");
    let mut ty: u32 = 0;
    let mut val: u32 = 0;
    let mut blen: u32 = std::mem::size_of::<u32>() as u32;
    let rc = unsafe {
        RegQueryValueExW(
            hk,
            name.as_ptr(),
            core::ptr::null_mut(),
            &mut ty,
            &mut val as *mut u32 as *mut u8,
            &mut blen,
        )
    };
    unsafe { RegCloseKey(hk) };
    rc == ERROR_SUCCESS && val == 1
}

fn startup_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|base| base.join(r"Microsoft\Windows\Start Menu\Programs\Startup"))
}

/// 启动文件夹里的启动脚本内容。用 VBS 的 `ws.Run ..., 0, False`
/// 隐藏窗口静默拉起，避免开机时闪一下黑框。
fn startup_script_body(exe_str: &str) -> String {
    format!("Set ws = CreateObject(\"WScript.Shell\")\r\nws.Run \"\"\"{exe_str}\"\" {RUN_ARG}\", 0, False\r\n")
}

fn startup_folder_enable_in(dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("取 exe 路径失败：{e}"))?;
    let exe_str = exe.to_string_lossy().replace('"', "");
    write_startup_script(dir, &exe_str)
}

fn write_startup_script(dir: &std::path::Path, exe_str: &str) -> Result<std::path::PathBuf, String> {
    let script = dir.join("RogMouseCtl.vbs");
    std::fs::create_dir_all(dir).map_err(|e| format!("创建启动文件夹失败：{e}"))?;
    std::fs::write(&script, startup_script_body(exe_str))
        .map_err(|e| format!("写入启动文件夹失败：{e}"))?;
    Ok(script)
}

fn startup_folder_enable() -> Result<(), String> {
    let dir = startup_dir().ok_or("取启动文件夹路径失败")?;
    startup_folder_enable_in(&dir).map(|_| ())
}

fn startup_folder_disable() {
    if let Some(d) = startup_dir() {
        remove_startup_script_in(&d);
    }
}

fn remove_startup_script_in(dir: &std::path::Path) {
    let _ = std::fs::remove_file(dir.join("RogMouseCtl.vbs"));
    let _ = std::fs::remove_file(dir.join("RogMouseCtl.cmd"));
    let _ = std::fs::remove_file(dir.join("RogMouseCtl.bat"));
}

fn startup_folder_state() -> bool {
    match startup_dir() {
        Some(d) => startup_folder_state_in(&d),
        None => false,
    }
}

fn startup_folder_state_in(dir: &std::path::Path) -> bool {
    let script = dir.join("RogMouseCtl.vbs");
    if script.exists() {
        if let Ok(content) = std::fs::read_to_string(&script) {
            if let Ok(exe) = std::env::current_exe() {
                let s = exe.to_string_lossy().to_string();
                if content.contains(&s) {
                    return true;
                }
            }
        }
    }
    false
}

/// 绿色版：查状态。
///
/// **不能只看键存在** —— 用户可能改过路径、或者 exe 被移动过。
/// 要比对命令行里的 exe 路径是否与当前进程一致，否则会出现「显示已启用
/// 但开机什么也没发生」的情况（旧路径已失效）。
/// 同时检查注册表 Run 键与用户 Startup 启动文件夹脚本。
fn registry_state() -> AutoStartState {
    let current = match quoted_exe_path() {
        Ok(s) => s,
        Err(_) => return AutoStartState::Unknown,
    };
    match read_run_value() {
        Ok(Some(v)) if v.trim() == current.trim() => AutoStartState::Enabled,
        _ => {
            if startup_folder_state() {
                AutoStartState::Enabled
            } else {
                AutoStartState::Disabled
            }
        }
    }
}

/// 绿色版：写入开机自启。
///
/// 优先用 `RegOpenKeyExW` 打开已存在的 Run 键。如果注册表受到安全策略、
/// 杀毒软件拦截或权限限制（Win32 错误码 5 ACCESS_DENIED），自动平滑降级到
/// Windows 用户的启动文件夹（`shell:startup`，生成无控制台弹窗的后台自启脚本）。
fn registry_enable() -> Result<(), String> {
    let reg_result = (|| -> Result<(), String> {
        let cmd = quoted_exe_path()?;
        unsafe {
            let sub = to_wide(RUN_SUBKEY);
            let mut hk: usize = 0;

            // ① 优先打开已存在的键（HKCU\...\Run 一定存在）
            let mut rc = RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_SET_VALUE, &mut hk);
            if rc != ERROR_SUCCESS {
                // ② 键不存在时才创建；显式要求 SET_VALUE 权限
                let mut disp: u32 = 0;
                rc = RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    sub.as_ptr(),
                    0,
                    core::ptr::null(),
                    0,
                    KEY_SET_VALUE,
                    core::ptr::null_mut(),
                    &mut hk,
                    &mut disp,
                );
                if rc != ERROR_SUCCESS {
                    return Err(format!("Win32 错误码 {rc}"));
                }
            }

            let name = to_wide(RUN_VALUE_NAME);
            let value = to_wide(&cmd);
            // REG_SZ 的长度必须**含结尾 NUL**，单位是字节
            let bytes = (value.len() * 2) as u32;
            let rc = RegSetValueExW(hk, name.as_ptr(), 0, REG_SZ, value.as_ptr() as *const u8, bytes);
            RegCloseKey(hk);

            if rc != ERROR_SUCCESS {
                return Err(format!("Win32 错误码 {rc}"));
            }
        }
        Ok(())
    })();

    if reg_result.is_ok() {
        startup_folder_disable();
        return Ok(());
    }

    // 注册表写入失败（如错误码 5 拦截）：平滑降级到启动文件夹
    startup_folder_enable()
}

/// 绿色版：删掉自启。同时清理 Run 键与启动文件夹。
fn registry_disable() -> Result<(), String> {
    startup_folder_disable();
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
        if rc == ERROR_SUCCESS {
            let name = to_wide(RUN_VALUE_NAME);
            let _ = RegDeleteValueW(hk, name.as_ptr());
            RegCloseKey(hk);
        }
    }
    Ok(())
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
