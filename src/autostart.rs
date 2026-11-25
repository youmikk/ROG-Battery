
#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

use std::path::PathBuf;

use crate::win32::to_wide;

mod startup;

// ============================================================
// 对外类型
// ============================================================

/// 区分用户禁用与策略限制，便于提示正确的恢复方式。
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

/// 请求成功后仍需检查最终状态，用户拒绝不属于 API 错误。
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
    /// 设置页显示的路径名称。
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

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentPackageFullName(len: *mut u32, name: *mut u16) -> i32;
}

/// 包身份查询失败时保留未知状态。
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

pub fn is_packaged() -> bool {
    package_identity() == Some(true)
}

/// 按包身份选择自启入口。
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
/// 值名。用 `RogMouseCtl` 而不是 `X-Battery`：注册表值名允许空格但
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
// 对外接口
// ============================================================

/// 查询失败时以未知状态显示。
pub fn state() -> (AutoStartState, AutoStartPath) {
    let path = detect_path();
    let st = match path {
        AutoStartPath::WinRt => startup::state().unwrap_or(AutoStartState::Unknown),
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
            let st = startup::enable()?;
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
            let st = startup::disable()?;
            Ok(EnableOutcome {
                enabled: st.is_enabled(),
                state: st,
                path,
            })
        }
        AutoStartPath::Unknown => Err("无法判定运行形态（包身份读取失败）".to_string()),
    }
}
