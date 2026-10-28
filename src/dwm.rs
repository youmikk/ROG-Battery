//! DPI 感知、显示器缩放比、DWM 深色标题栏。
//!
//! ## 为什么需要这个模块
//!
//! 进程**不**声明 DPI 感知时，Windows 会把整个窗口按 96 DPI 的逻辑坐标画好，
//! 再由 DWM 把成品位图放大到屏幕的真实像素。125% 缩放下就是 1.25 倍双线性
//! 插值 —— 字形边缘被抹平、笔画粗细不均，这就是用户说的「字体发虚」。
//! 在 GUI 里调字号、换字体、开 ClearType 都治不好它：糊发生在窗口画完之后，
//! 而我们手上只有放大后的位图。
//!
//! 唯一的解法是在**创建任何窗口之前**调 `SetProcessDpiAwarenessContext`
//! （或下面两个降级替代），让 GDI / DirectWrite 直接在物理像素上排版。
//!
//! 同一个进程的 DPI 感知**只能设定一次**：第一个成功的调用生效，之后再调
//! 一律失败（`ERROR_ACCESS_DENIED` / `E_ACCESSDENIED`）。这不是错误，反而是
//! 「进程已经是感知的」的证据 —— 本模块里好几处「失败也算成功」的判断都
//! 来源于此，改动前请先读那几段注释。
//!
//! ## 为什么这些 API 全是动态解析
//!
//! 用户群里有 Win7 / Win8.1 的老机器。`SetProcessDpiAwarenessContext` 是
//! Win10 1703+ 的导出，`GetDpiForSystem` 是 Win10 1607+ 的导出，`shcore.dll`
//! 本身在 Win7 上就不存在。把这些符号写进 `#[link]` 会让**进程启动**就因
//! 解析不到导入而失败 —— 用户看到的是「双击没反应」，比字体发虚还糟。
//! 所以它们统一走 `LoadLibraryW` + `GetProcAddress`，取不到就降级。
//!
//! `dwmapi.dll` 是 Vista+ 的系统组件，且 `vendor\mingw-import-libs\libdwmapi.a`
//! 已由 `build.rs` 加进链接器的搜索路径，所以 `DwmSetWindowAttribute` 可以
//! 静态链接；它引用的属性号则在运行时按系统版本逐个试。

// 声明的都是 Win32 导出的原名（`SetWindowPos` 之类），改成 snake_case 就没法
// 和 MSDN 对照了。
#![allow(non_snake_case)]
// 本模块的 `pub` 函数要等 `main.rs` 里加上 `mod dwm;` 和调用点之后才有人用。
// 在那之前 `dead_code` 会对每个函数报一遍。调用点全部接好之后可以删掉这一行。
#![allow(dead_code)]

use core::ffi::c_void;
use core::mem::size_of;
use std::sync::OnceLock;

use crate::win32::{to_wide, GetDC, GetLastError, GetProcAddress, Handle, LoadLibraryW, ReleaseDC};

// ============ DPI 感知相关的常量 ============

/// `DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2`。SDK 里的定义是 `(HANDLE)-4`。
///
/// 用 `isize` 存而不是 `Handle`：负整数转指针在 const 里不直观，调用点一处
/// `as Handle` 更清楚，也避免有人误以为这是个可以解引用的真句柄。
const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4;

/// `PROCESS_PER_MONITOR_DPI_AWARE`，`shcore!SetProcessDpiAwareness` 的参数。
const PROCESS_PER_MONITOR_DPI_AWARE: i32 = 2;

/// `ERROR_ACCESS_DENIED`。DPI 感知已经设定过时，Win32 路径返回的就是它。
const ERROR_ACCESS_DENIED: u32 = 5;

/// `E_ACCESSDENIED`。`SetProcessDpiAwareness` 返回 HRESULT，同一个语义在 COM
/// 里是这个值，**不是** Win32 的 5。两个都要认，否则 Win8.1 上会误判成失败。
const E_ACCESSDENIED: i32 = 0x8007_0005u32 as i32;

/// `LOGPIXELSX`：`GetDeviceCaps` 里「每英寸逻辑像素」的索引。
const LOGPIXELSX: i32 = 88;

/// 缩放比的合理区间。见 `scale_from_dpi` 的注释。
const MIN_SCALE: f32 = 0.5;
const MAX_SCALE: f32 = 6.0;

// ============ DWM 属性号 ============

/// 深色标题栏属性，Win10 1809 ~ 1909 用的编号。
const DWMWA_USE_IMMERSIVE_DARK_MODE_LEGACY: u32 = 19;
/// 深色标题栏属性，Win10 2004+ / Win11 的编号。
const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;

/// 边框颜色，Win11 22000+。
const DWMWA_BORDER_COLOR: u32 = 34;
/// 标题栏颜色，Win11 22000+。
const DWMWA_CAPTION_COLOR: u32 = 35;
/// 标题栏文字颜色，Win11 22000+。
const DWMWA_TEXT_COLOR: u32 = 36;
/// 窗口圆角偏好，Win11 22000+（`DWMWA_WINDOW_CORNER_PREFERENCE`）。
const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
/// `DWMWCP_ROUND`：四角微圆，跟系统原生窗口同一档手感。
const DWMWCP_ROUND: u32 = 2;

/// `DWMWA_COLOR_DEFAULT`：「不指定颜色，用系统默认」。
/// 注意它是 `0xFFFFFFFF`，不是黑色 —— 传 0 会得到纯黑标题栏。
///
/// 现在浅色也用显式颜色了，所以这个常量只在「不想管某种状态」时才用得上。
/// 留着并加 `#[allow(dead_code)]` 是为了让后来的人一眼看到「想恢复系统默认
/// 该传什么」，而不是去猜 `0` 或 `0xFFFFFF`。
#[allow(dead_code)]
const DWMWA_COLOR_DEFAULT: u32 = 0xFFFF_FFFF;

/// 深色主题的标题栏 / 边框色，`COLORREF` = `0x00BBGGRR`，即 RGB(32, 32, 32)。
const COLORREF_DARK_CAPTION: u32 = 0x0020_2020;
/// 深色主题的标题栏文字色，`COLORREF`，即 RGB(232, 232, 234)。
/// 用略偏冷的灰而不是纯白：纯白在深灰底上过亮，长时间看会累。
const COLORREF_DARK_TEXT: u32 = 0x00EA_E8E8;

/// 浅色主题的标题栏 / 边框色，`COLORREF`，即 RGB(243, 243, 243)。
///
/// 必须**显式**等于界面背景 `--bg #F3F3F3`，不能交给系统默认。
/// 系统默认浅色标题栏是 `#EFF4F9`（带一点冷蓝的白），和内容区 `#F3F3F3`
/// 差 4~6 个色阶 —— 在 100% 缩放下看是一条清楚的分界线，
/// 用户的原话就是「菜单栏依旧没有沉浸」。
const COLORREF_LIGHT_CAPTION: u32 = 0x00F3_F3F3;
/// 浅色主题的标题栏文字色，`COLORREF`，即 RGB(27, 27, 27)。
/// 与 `--text #1B1B1B` 一致。
const COLORREF_LIGHT_TEXT: u32 = 0x001B_1B1B;

// ============ SetWindowPos 标志 ============
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_FRAMECHANGED: u32 = 0x0020;

// ============ 静态链接的 FFI（系统组件，所有目标系统都有）============
//
// user32 / gdi32 在所有受支持的系统上都存在，导入库也在 rustlib 的
// self-contained 目录里，所以这三个可以静态链接，不必动态解析。
#[link(name = "user32")]
extern "system" {
    /// Vista+。DPI 感知降级链的最后一环。
    fn SetProcessDPIAware() -> i32;

    /// 只用来「踢」一下窗口让 DWM 重画非客户区，见 `refresh_titlebar`。
    fn SetWindowPos(
        hwnd: Handle,
        insert_after: Handle,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    /// 取 DC 的能力值。这里只用 `LOGPIXELSX`。
    fn GetDeviceCaps(hdc: Handle, index: i32) -> i32;
}

#[link(name = "dwmapi")]
extern "system" {
    /// `HRESULT DwmSetWindowAttribute(HWND, DWORD dwAttribute, LPCVOID, DWORD)`
    ///
    /// 失败时返回负的 HRESULT（`E_INVALIDARG` / `E_HANDLE`），不抛异常。
    /// 老系统上不认识新属性号时会安静地返回失败，正是我们要的降级行为。
    fn DwmSetWindowAttribute(hwnd: Handle, attribute: u32, value: *const c_void, size: u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    /// 清 last-error。
    ///
    /// **必须在「调用 API → 读 `GetLastError`」之前清零**：这几个 DPI API 的
    /// 部分失败路径不写 last-error（例如 `SetProcessDpiAwareness` 失败时
    /// 只给 HRESULT）。不清零的话，读到的是更早的残留值，可能恰好是 5，
    /// 于是把真正的失败误判成「已经设过了」而返回 true。
    fn SetLastError(err: u32);
}

// ============ 动态解析的 FFI ============
//
// 签名必须和 DLL 里完全一致；这里都是 `extern "system"`（stdcall 在 x64 上
// 与 C 调用约定相同，但显式写出来在 32 位目标上才是对的）。
type FnSetProcessDpiAwarenessContext = unsafe extern "system" fn(Handle) -> i32;
type FnSetProcessDpiAwareness = unsafe extern "system" fn(i32) -> i32;
type FnGetDpiForSystem = unsafe extern "system" fn() -> u32;

/// user32.dll 的模块句柄，进程内只 `LoadLibraryW` 一次。
///
/// 为什么缓存：`system_scale` 会被界面重绘反复调用，每次重新
/// `LoadLibraryW` 都会给模块抬一次引用计数（永不归还），属于无意义的泄漏。
///
/// 为什么**不** `FreeLibrary`：user32 / shcore 一旦被加载就常驻，我们取到的
/// 函数指针要求在整个进程生命期内有效。释放引用计数只会让我们手里的指针
/// 变成悬空指针，没有任何好处。
///
/// 缓存类型是 `usize` 而不是 `Handle`：裸指针没有 `Sync`，塞不进 `OnceLock`。
fn user32() -> Handle {
    static MODULE: OnceLock<usize> = OnceLock::new();
    let raw = *MODULE.get_or_init(|| {
        let wide = to_wide("user32.dll");
        // SAFETY: `wide` 是 NUL 结尾的 UTF-16 且在本表达式内保持存活；
        // LoadLibraryW 只读取该缓冲区，不持有它。系统 DLL 句柄不归我们释放。
        let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
        handle as usize
    });
    raw as Handle
}

/// shcore.dll 的模块句柄。Win7 上没有这个 DLL，`LoadLibraryW` 返回空。
fn shcore() -> Handle {
    static MODULE: OnceLock<usize> = OnceLock::new();
    let raw = *MODULE.get_or_init(|| {
        let wide = to_wide("shcore.dll");
        // SAFETY: 同 `user32`：NUL 结尾的 UTF-16、只读、不持有。
        let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
        handle as usize
    });
    raw as Handle
}

/// 取模块里的导出地址。`name` 必须是 NUL 结尾的字节串。
fn symbol(module: Handle, name: &[u8]) -> *mut c_void {
    if module.is_null() {
        // 模块不存在（老系统）直接短路，不必让 GetProcAddress 去碰空句柄。
        return core::ptr::null_mut();
    }
    // SAFETY: `module` 是上面 LoadLibraryW 得到的、仍然有效的模块句柄
    // （我们从不 FreeLibrary）；`name` 以 NUL 结尾，满足 GetProcAddress 的要求。
    unsafe { GetProcAddress(module, name.as_ptr()) }
}

fn resolve_set_dpi_awareness_context(module: Handle) -> Option<FnSetProcessDpiAwarenessContext> {
    let p = symbol(module, b"SetProcessDpiAwarenessContext\0");
    if p.is_null() {
        return None;
    }
    // SAFETY: user32.dll 中该导出的真实签名是
    // `BOOL WINAPI SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT)`，
    // 而 DPI_AWARENESS_CONTEXT 就是 HANDLE（指针宽度），与这里的类型逐位一致。
    Some(unsafe { core::mem::transmute::<*mut c_void, FnSetProcessDpiAwarenessContext>(p) })
}

fn resolve_set_process_dpi_awareness(module: Handle) -> Option<FnSetProcessDpiAwareness> {
    let p = symbol(module, b"SetProcessDpiAwareness\0");
    if p.is_null() {
        return None;
    }
    // SAFETY: shcore!SetProcessDpiAwareness 的签名是
    // `HRESULT WINAPI SetProcessDpiAwareness(PROCESS_DPI_AWARENESS)`；
    // 该枚举按 int 传递（4 字节），HRESULT 是 32 位整数，与这里一致。
    Some(unsafe { core::mem::transmute::<*mut c_void, FnSetProcessDpiAwareness>(p) })
}

fn resolve_get_dpi_for_system(module: Handle) -> Option<FnGetDpiForSystem> {
    let p = symbol(module, b"GetDpiForSystem\0");
    if p.is_null() {
        return None;
    }
    // SAFETY: user32!GetDpiForSystem 的签名是 `UINT WINAPI GetDpiForSystem(void)`，
    // 无参数、返回 32 位无符号整数。
    Some(unsafe { core::mem::transmute::<*mut c_void, FnGetDpiForSystem>(p) })
}

// ============ 能力 1：进程 DPI 感知 ============

/// 在创建任何窗口之前调用一次。返回是否成功。
///
/// 顺序是「新 API → 次新 → 老 API」，每一环失败的原因都可能是「系统太老」
/// 或「已经有人设过了」。后者算成功，见各分支注释。
pub fn enable_dpi_awareness() -> bool {
    if try_set_dpi_awareness_context().unwrap_or(false) {
        return true;
    }
    if try_set_process_dpi_awareness().unwrap_or(false) {
        return true;
    }
    try_set_process_dpi_aware()
}

/// 第一选择：per-monitor v2（Win10 1703+）。
///
/// 为什么优先 v2 而不是 v1：v1 只缩放客户区，非客户区（标题栏、菜单栏、滚动条）
/// 由 DWM 按旧比例画。窗口拖到另一块不同 DPI 的显示器上时，v1 的标题栏会
/// 停在旧尺寸，出现「字变大了但标题栏没变」的割裂感。v2 会一并处理非客户区。
///
/// 返回 `None` = 这台系统没有这个 API；`Some(true)` = 成功或已经是感知的；
/// `Some(false)` = 有 API 但设置失败（例如参数被拒），应该继续往下降级。
fn try_set_dpi_awareness_context() -> Option<bool> {
    let f = resolve_set_dpi_awareness_context(user32())?;
    let context = DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 as Handle;

    // SAFETY: `f` 是按真实签名解析出的 user32 导出；`context` 是系统约定的
    // 伪句柄 -4，API 只比较它的数值，不会解引用。
    unsafe { SetLastError(0) };
    // SAFETY: 同上。
    let ok = unsafe { f(context) } != 0;
    if ok {
        return Some(true);
    }

    // 失败**可能**是好事：manifest 里声明过 DPI 感知、或者别的库先调过一次时，
    // 系统拒绝第二次设置并给出 ERROR_ACCESS_DENIED。此时进程已经是感知的，
    // 我们要的效果已经达成，返回 false 只会让上层以为坏掉了。
    // SAFETY: GetLastError 无参数、无副作用。
    Some(unsafe { GetLastError() } == ERROR_ACCESS_DENIED)
}

/// 第二选择：`shcore!SetProcessDpiAwareness(2)`（Win8.1+）。
///
/// 这个函数必须动态解析：rustlib 的 self-contained 目录里**没有**
/// `libshcore.a`，`vendor\mingw-import-libs` 里也没有，写 `#[link(name = "shcore")]`
/// 会在链接期变成 `ld: cannot find -lshcore`。
fn try_set_process_dpi_awareness() -> Option<bool> {
    let f = resolve_set_process_dpi_awareness(shcore())?;

    unsafe { SetLastError(0) };
    // SAFETY: `f` 是 shcore!SetProcessDpiAwareness 的真实签名，
    // `PROCESS_PER_MONITOR_DPI_AWARE` 是合法枚举值。
    let hr = unsafe { f(PROCESS_PER_MONITOR_DPI_AWARE) };
    if hr == 0 {
        return Some(true);
    }

    // 两个「已经设过」的表示都要认：HRESULT 是 E_ACCESSDENIED，
    // 但个别系统会把 Win32 的 ERROR_ACCESS_DENIED 留在 last-error 里。
    // SAFETY: GetLastError 无参数、无副作用。
    let denied = hr == E_ACCESSDENIED || unsafe { GetLastError() } == ERROR_ACCESS_DENIED;
    Some(denied)
}

/// 最后一环：`SetProcessDPIAware()`（Vista+，永远存在）。
///
/// 它只能做到「系统 DPI 感知」：整个进程按主显示器的 DPI 缩放，换到别的
/// 显示器不会重新排版。作为兜底够用 —— 至少字体在物理像素上渲染，不会糊。
fn try_set_process_dpi_aware() -> bool {
    unsafe { SetLastError(0) };
    // SAFETY: SetProcessDPIAware 自 Vista 起就在 user32 里，无参数；
    // 重复调用只会失败并置 last-error，不会改变已生效的设置。
    let ok = unsafe { SetProcessDPIAware() } != 0;
    ok || unsafe { GetLastError() } == ERROR_ACCESS_DENIED
}

// ============ 能力 2：主显示器缩放比 ============

/// 取主显示器的缩放比（96 DPI = 1.0）。失败时返回 1.0。
///
/// 注意这是「主显示器」的比例，不是「窗口所在显示器」的比例。本工具的窗口
/// 不跨屏，够用；真要跟窗口走应该用 `GetDpiForWindow(hwnd)`（Win10 1607+），
/// 那需要把 HWND 传进来，接口就复杂了。
pub fn system_scale() -> f32 {
    if let Some(dpi) = get_dpi_for_system() {
        return scale_from_dpi(dpi as f32);
    }
    if let Some(dpi) = screen_dpi_from_dc() {
        return scale_from_dpi(dpi as f32);
    }
    // 两条路都拿不到就假定 100%：这是最保守的错误值，界面只会略小，不会错位。
    1.0
}

/// 第一选择：`GetDpiForSystem()`（Win10 1607+）。
///
/// 它直接返回 DPI，不依赖任何 DC，也就没有「忘记 ReleaseDC 漏 GDI 句柄」的风险。
fn get_dpi_for_system() -> Option<u32> {
    let f = resolve_get_dpi_for_system(user32())?;
    // SAFETY: `f` 是 GetDpiForSystem 的真实签名（无参数，返回 UINT）。
    let dpi = unsafe { f() };
    // 文档说失败返回 0（正常值不会小于 48）。
    if dpi == 0 {
        None
    } else {
        Some(dpi)
    }
}

/// 第二选择：屏幕 DC 的 `LOGPIXELSX`（Win2000+ 都行）。
///
/// `GetDC(NULL)` 取的是整个屏幕的 DC，Win32 明确允许传空 hwnd。拿到的 DC
/// **必须**配对 `ReleaseDC`：每个 DC 都是 GDI 资源，进程有 10000 个的上限，
/// 漏掉的话界面刷新几千次就会开始绘制失败。
fn screen_dpi_from_dc() -> Option<i32> {
    // SAFETY: GetDC(NULL) 取屏幕 DC 是合法调用；返回非空即需 ReleaseDC(NULL, hdc)
    // 配对释放（第二个参数必须是同一个 hwnd，这里是 NULL）。GetDeviceCaps 只读，
    // 不会消耗或失效 DC。
    unsafe {
        let hdc = GetDC(core::ptr::null_mut());
        if hdc.is_null() {
            return None;
        }
        let dpi = GetDeviceCaps(hdc, LOGPIXELSX);
        ReleaseDC(core::ptr::null_mut(), hdc);
        if dpi > 0 {
            Some(dpi)
        } else {
            None
        }
    }
}

/// DPI 数值 → 缩放比，并夹到 `MIN_SCALE..=MAX_SCALE`。
///
/// 为什么要夹：`GetDeviceCaps` 在远程桌面、虚拟机、坏掉的显示驱动上会返回
/// 0 或荒谬的大数。上层的调用方式大致是 `字号 * scale`：0 会让整页文字消失，
/// 几万会直接把布局撑到屏幕外。夹到 0.5~6.0 覆盖了 Windows 支持的全部
/// 缩放档位（50% ~ 600%），超出这个范围的输入一定是坏数据，退回 1.0。
fn scale_from_dpi(dpi: f32) -> f32 {
    if !dpi.is_finite() || dpi <= 0.0 {
        return 1.0;
    }
    (dpi / 96.0).clamp(MIN_SCALE, MAX_SCALE)
}

// ============ 能力 3：DWM 深色标题栏 ============

/// 把窗口标题栏切成深色（true）或浅色（false）。失败时静默返回 false。
///
/// 老系统（Win7 / Win8.1 / Win10 1803 及更早）没有这个能力，返回值 false 属于
/// 正常情况，调用方应当忽略而不是弹错误框。
pub fn set_dark_titlebar(hwnd: Handle, dark: bool) -> bool {
    if hwnd.is_null() {
        return false;
    }
    let value: u32 = u32::from(dark);

    // 为什么两个属性号都试：`DWMWA_USE_IMMERSIVE_DARK_MODE` 在 Win10 1809 ~ 1909
    // 的实现里编号是 **19**，2004 起才改成 **20** 并固定下来（中间改号是
    // 微软自己的历史遗留，不是文档笔误）。只试 20 → 1909 用户永远切不过去；
    // 只试 19 → 新系统上无效。两次调用各是一次很便宜的 DWM 调用，按新→旧试，
    // 任意一次成功即视为系统支持。
    // 这段**不要**「优化」成只留一个常量，那会静默砍掉一整代系统的支持。
    let mut applied = dwm_attr(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &value);
    if !applied {
        applied = dwm_attr(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE_LEGACY, &value);
    }

    // 下面三个颜色属性是 Win11 22000+ 才有的。老系统上会返回 E_INVALIDARG，
    // 我们忽略即可 —— 所以**不**用 `applied` 去决定要不要调，也不用它们的
    // 结果去影响返回值。
    //
    // 为什么深色时要显式设这三个：只开 immersive dark mode 的话，Win11 的
    // 边框和标题栏文字仍是系统强调色/浅色，深色标题栏边缘会留一条亮线，
    // 看起来像「没切干净」。
    //
    // 为什么**浅色时也要设**：这是「标题栏不沉浸」的根因。系统默认浅色标题栏
    // 是 `#EFF4F9`（冷蓝偏白），内容区是 `#F3F3F3`（中性灰），两者差 4~6 个
    // 色阶，在标题栏下沿形成一条看得见的分界。只切深色不给浅色指定颜色时，
    // 表现就是「深色模式看着挺好，浅色模式上下两块不一样白」。
    let (caption, text, border) = if dark {
        (
            COLORREF_DARK_CAPTION,
            COLORREF_DARK_TEXT,
            COLORREF_DARK_CAPTION,
        )
    } else {
        (
            COLORREF_LIGHT_CAPTION,
            COLORREF_LIGHT_TEXT,
            COLORREF_LIGHT_CAPTION,
        )
    };
    dwm_attr(hwnd, DWMWA_CAPTION_COLOR, &caption);
    dwm_attr(hwnd, DWMWA_TEXT_COLOR, &text);
    dwm_attr(hwnd, DWMWA_BORDER_COLOR, &border);

    applied
}

/// 调一次 `DwmSetWindowAttribute`，`hr == S_OK` 时返回 true。
///
/// 参数值统一按 4 字节 DWORD 传：深色开关是 BOOL（4 字节），颜色是 COLORREF
/// （也是 4 字节）。`cbAttribute` 必须**精确等于**属性对应的类型大小，给大了
/// 会被当成 `E_INVALIDARG`。
fn dwm_attr(hwnd: Handle, attribute: u32, value: &u32) -> bool {
    // SAFETY: `hwnd` 由调用方保证（无效句柄只会让 DWM 返回 E_HANDLE，不会崩）；
    // `value` 指向一个存活的 u32，长度按 `size_of::<u32>()` 上报，符合 API 约定；
    // DWM 只在本次调用内读取这 4 个字节，不保留指针。
    let hr = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            attribute,
            value as *const u32 as *const c_void,
            size_of::<u32>() as u32,
        )
    };
    hr == 0
}

/// 切深色标题栏并立刻重画，供界面上的「深色模式」开关调用。
///
/// 为什么需要这个包装：DWM 只会在窗口的下一次非客户区重算时重新合成标题栏。
/// 单靠 `DwmSetWindowAttribute`，Win11 上颜色要等用户拖动或缩放窗口才出现，
/// 用户看到的现象是「开关点了没反应」，然后去点第二次、第三次。
///
/// `SWP_FRAMECHANGED` 让 DWM 丢掉缓存的框架位图并重发 `WM_NCCALCSIZE` /
/// `WM_NCPAINT`；其余四个标志保证这次调用不改动任何可见状态：不移动、不改
/// 大小、不改 z 序、不抢焦点。
pub fn refresh_titlebar(hwnd: Handle, dark: bool) {
    let _ = set_dark_titlebar(hwnd, dark);
    if hwnd.is_null() {
        return;
    }

    // SAFETY: `hwnd` 非空；SetWindowPos 对失效句柄只返回 FALSE 并置 last-error，
    // 不会崩溃。插入顺序参数传 NULL 且带 SWP_NOZORDER 时被系统忽略。
    unsafe {
        SetWindowPos(
            hwnd,
            core::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

/// 给无边框窗口开系统原生圆角。Win11 22000+ 生效，老系统静默返回 false。
///
/// 为什么要开：我们是自绘无边框窗口（`go_frameless` 去掉了标题栏），DWM 默认
/// 把它当普通矩形合成，四角是直角，窗口边缘与桌面之间是一条硬切的直线 ——
/// 用户的原话是「下方边框没有衔接，很生硬」。设置 `DWMWCP_ROUND` 后由 DWM
/// 负责按系统风格裁出圆角并补上阴影，比自绘 `SetWindowRgn` 更贴合 Win11 观感
/// （自绘 region 会把阴影一起裁掉，反而更生硬）。
pub fn set_round_corners(hwnd: Handle) -> bool {
    if hwnd.is_null() {
        return false;
    }
    dwm_attr(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &DWMWCP_ROUND)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scale_matches_windows_dpi_steps() {
        // Windows 的缩放档位都是 96 的整数倍分之一，这里挑几档常见的。
        assert!((scale_from_dpi(96.0) - 1.0).abs() < f32::EPSILON);
        assert!((scale_from_dpi(120.0) - 1.25).abs() < f32::EPSILON);
        assert!((scale_from_dpi(144.0) - 1.5).abs() < f32::EPSILON);
        assert!((scale_from_dpi(192.0) - 2.0).abs() < f32::EPSILON);
        assert!((scale_from_dpi(288.0) - 3.0).abs() < f32::EPSILON);
    }

    #[test]
    fn scale_is_clamped_to_sane_range() {
        // 坏数据：0、负数、NaN → 退回 1.0，而不是夹到 0.5。
        assert_eq!(scale_from_dpi(0.0), 1.0);
        assert_eq!(scale_from_dpi(-96.0), 1.0);
        assert_eq!(scale_from_dpi(f32::NAN), 1.0);
        assert_eq!(scale_from_dpi(f32::INFINITY), 1.0);

        // 有效但极端的数据才夹取。
        assert_eq!(scale_from_dpi(48.0), MIN_SCALE); // 50%
        assert_eq!(scale_from_dpi(1.0), MIN_SCALE);
        assert_eq!(scale_from_dpi(96.0 * 100.0), MAX_SCALE); // 假想的 10000%
    }
}
