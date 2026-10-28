//! 无边框窗口（去掉系统标题栏，保留原生缩放 / Aero Snap / 系统阴影）。
//!
//! # 为什么不是「用 `WS_POPUP` 重建窗口」
//!
//! 用户想要的是 Win11 计算器那样：标题栏那一条由程序自己画，但**窗口行为**
//! 还是系统的 —— 拖边缘能缩放、拖到屏幕顶能最大化、双击标题栏能最大化、
//! 点关闭有系统关窗流程（含我们的「关闭到托盘」判断）。
//!
//! 用 `WS_POPUP` 会把这些全丢掉，得自己实现一堆边缘命中、吸附、动画。
//! 正确的做法是**只拿掉 caption，保留 `WS_THICKFRAME`**：
//!
//! - `WS_THICKFRAME` 还在 → 系统仍然做边缘缩放和 Aero Snap。
//! - `WM_NCCALCSIZE` 返回 0 → 客户区铺满整个窗口矩形，标题栏视觉上消失。
//! - DWM 的圆角、投影、最大化动画都由系统照旧处理。
//!
//! # 命中测试为什么在 canvas 里做（本模块只出算法）
//!
//! 内容区是覆盖**整个客户区**的子窗口，所以鼠标消息和 `WM_NCHITTEST`
//! 全部先到子窗口 —— 主窗口根本收不到。于是「这一块属于哪类」在
//! [`logical_hit`] 里算（纯函数，可单测），
//! 真正把意图转交系统（`WM_NCLBUTTONDOWN`）由 `canvas.rs` 做。
//!
//! 这一点我第一版想反了：在 `gui.rs` 里给主窗口挂 `WM_NCHITTEST` 子类，
//! 结果整个标题栏拖不动 —— 消息压根没到主窗口。
//!
//! # 拖动为什么不自己实现
//!
//! 把 `HTCAPTION` 交给 `DefWindowProc(WM_NCLBUTTONDOWN)` 之后，拖动、Aero Snap
//! （拖到屏幕顶最大化、拖到左右边缘半屏）、双击最大化、高 DPI 跟手
//! 全是系统原生那份。自己 `SetWindowPos` 跟鼠标会把这些全部丢掉。
//!
//! 我第一版就是自己算偏移跟手，实测：拖到屏幕顶不吸附、拖到边缘不贴靠、
//! 双屏之间拖不过去、125% 缩放下抖。

use crate::win32::Handle;
use crate::win32::{GetMonitorInfoW, MonitorFromWindow, MonitorInfo};

// ── 窗口样式位 ──
const GWL_STYLE: i32 = -16;
const WS_CAPTION: isize = 0x00C0_0000;
const WS_SYSMENU: isize = 0x0008_0000;
const WS_THICKFRAME: isize = 0x0004_0000;
const WS_MINIMIZEBOX: isize = 0x0002_0000;
const WS_MAXIMIZEBOX: isize = 0x0001_0000;
const WS_MAXIMIZE: isize = 0x0100_0000;

// ── 消息 ──
const WM_CLOSE: u32 = 0x0010;
const WM_SYSCOMMAND: u32 = 0x0112;

// ── 命中测试返回值 ──
pub const HTCLIENT: isize = 1;
pub const HTCAPTION: isize = 2;
pub const HTMINBUTTON: isize = 8;
pub const HTMAXBUTTON: isize = 9;
pub const HTLEFT: isize = 10;
pub const HTRIGHT: isize = 11;
pub const HTTOP: isize = 12;
pub const HTTOPLEFT: isize = 13;
pub const HTTOPRIGHT: isize = 14;
pub const HTBOTTOM: isize = 15;
pub const HTBOTTOMLEFT: isize = 16;
pub const HTBOTTOMRIGHT: isize = 17;
pub const HTCLOSE: isize = 20;

/// 边缘缩放的抓取宽度（**逻辑像素**）。
///
/// 8 是 Windows 的惯用值（`SM_CXPADDEDBORDER` + 边框宽度之和）。
/// 太窄用户抓不住，太宽会把卡片边缘的点击吃掉。
const RESIZE_BORDER: f32 = 8.0;

// ── 系统命令 ──
pub const SC_MINIMIZE: usize = 0xF020;
pub const SC_MAXIMIZE: usize = 0xF030;
pub const SC_RESTORE: usize = 0xF120;

/// 拿掉系统标题栏，保留缩放边框。
///
/// 必须在窗口建好之后、`dispatch_thread_events` 之前调。
/// 返回 `true` = 样式改成功（或本来就是对的）。
pub fn go_frameless(hwnd: Handle) -> bool {
    let style = crate::ui::canvas::get_window_long(hwnd, GWL_STYLE);
    if style == 0 {
        return false;
    }
    let mut new_style = style;
    // 去掉 caption 和系统菜单（左上角那个图标菜单）
    new_style &= !WS_CAPTION;
    new_style &= !WS_SYSMENU;
    // **保留并补齐**这几个：没有 WS_THICKFRAME 就没有边缘缩放和 Aero Snap；
    // 没有 MIN/MAXBOX 的话，最大化按钮点了也没用，任务栏右键菜单也是灰的。
    new_style |= WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
    if new_style == style {
        return true;
    }
    crate::ui::canvas::set_window_long(hwnd, GWL_STYLE, new_style);
    // 改完样式要通知系统重算非客户区，否则那一圈 8px 边框仍然按旧样式留着，
    // 客户区比窗口小一圈、页面右下角露白。
    crate::ui::canvas::set_window_pos(
        hwnd,
        0,
        0,
        0,
        0,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
    );
    true
}

const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_FRAMECHANGED: u32 = 0x0020;

/// 窗口当前是不是最大化（`WS_MAXIMIZE` 位）。
pub fn is_maximized(hwnd: Handle) -> bool {
    crate::ui::canvas::get_window_long(hwnd, GWL_STYLE) & WS_MAXIMIZE != 0
}

/// 去掉标题栏之后把**客户区**校正回指定逻辑尺寸。
///
/// 为什么需要这一步：`WM_NCCALCSIZE` 让客户区 = 整个窗口矩形之后，
/// 原来那圈边框（本机 16 宽 / 39 高）就变成了客户区的一部分 —— 于是内容区
/// 从 1100×740 变成 1116×779，页面右下角多出一截空白、卡片被拉伸。
///
/// 修法是按「窗口比客户区多出来的部分」补回窗口尺寸：
/// 有边框时（16/39）补上就回到 1100×740；已经无边框时这个差值是 0，
/// 窗口尺寸本身就是客户区尺寸，直接设成目标值即可。
///
/// 用两次测量而不是硬编码 16/39：边框宽度随 DPI 和系统主题
/// （`SM_CXPADDEDBORDER`）变，硬编码在 125% 缩放下会差几像素。
pub fn keep_client_size(hwnd: Handle, want_w: f32, want_h: f32, scale: f32) {
    let want_pw = (want_w * scale).round() as i32;
    let want_ph = (want_h * scale).round() as i32;
    let mut cr = crate::win32::Rect::default();
    let mut wr = crate::win32::Rect::default();
    crate::ui::canvas::get_client_rect(hwnd, &mut cr);
    crate::ui::canvas::get_window_rect(hwnd, &mut wr);
    let dw = (wr.width() - cr.width()).max(0);
    let dh = (wr.height() - cr.height()).max(0);
    crate::ui::canvas::set_window_pos(
        hwnd,
        0,
        0,
        want_pw + dw,
        want_ph + dh,
        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
    );
}

const SWP_NOACTIVATE: u32 = 0x0010;

/// 处理 `WM_NCCALCSIZE`。返回 `Some(r)` 表示已处理，`None` 交给 `DefWindowProc`。
///
/// **无论 wparam 为何值，一律返回 0（客户区 = 整个窗口矩形）**。
/// 这一条是彻底去掉系统标题栏的核心：
/// 以前在 `wparam == 0` 时返回了 `None`，导致在窗口隐藏重显或某些 `SetWindowPos` 时
/// Windows 默认过程介入、重新算出了 21px 经典蓝条标题栏。
/// 全部返回 0 后，系统永远不会为本窗口生成非客户区标题栏。
pub fn on_nccalcsize(_wparam: usize) -> Option<isize> {
    Some(0)
}

/// 命中测试（**纯函数**，入参全是逻辑像素的客户区坐标）。
///
/// 顺序很重要：**先判按钮，再判边缘，最后才判标题栏**。反过来的话，
/// 右上角的关闭按钮会先被「顶部边缘缩放区」吃掉，用户点关闭变成拉窗口 ——
/// 我第一版就是这个顺序，实测点关闭会把窗口拉高。
pub fn logical_hit(
    x: f32,
    y: f32,
    client_w: f32,
    client_h: f32,
    title_h: f32,
    maximized: bool,
) -> isize {
    // ① 三条按钮
    if (0.0..=title_h).contains(&y) {
        if let Some(b) = crate::ui::titlebar::hit(x, y, client_w) {
            return match b {
                crate::ui::titlebar::Btn::Minimize => HTMINBUTTON,
                crate::ui::titlebar::Btn::Maximize => HTMAXBUTTON,
                crate::ui::titlebar::Btn::Close => HTCLOSE,
            };
        }
    }

    // ② 四边 + 四角缩放。**最大化时不给边缘命中码** —— 用户想把窗口往下
    //    拖回来时，碰到的是「上边缘缩放」而不是标题栏拖动，窗口纹丝不动。
    if !maximized {
        let b = RESIZE_BORDER;
        let left = x < b;
        let right = x > client_w - b;
        let top = y < b;
        let bottom = y > client_h - b;
        // 角的优先级高于边（先判两组都成立的情况）
        match (left, right, top, bottom) {
            (true, _, true, _) => return HTTOPLEFT,
            (_, true, true, _) => return HTTOPRIGHT,
            (true, _, _, true) => return HTBOTTOMLEFT,
            (_, true, _, true) => return HTBOTTOMRIGHT,
            (true, _, _, _) => return HTLEFT,
            (_, true, _, _) => return HTRIGHT,
            (_, _, true, _) => return HTTOP,
            (_, _, _, true) => return HTBOTTOM,
            _ => {}
        }
    }

    // ③ 标题栏其余部分 = 可拖动
    if (0.0..=title_h).contains(&y) {
        return HTCAPTION;
    }

    HTCLIENT
}

/// 这个命中码要不要交给系统做非客户区操作（拖动 / 缩放）。
pub fn is_nc_drag(hit: isize) -> bool {
    hit == HTCAPTION || (HTLEFT..=HTBOTTOMRIGHT).contains(&hit)
}

/// 这个命中码是不是三条窗口按钮之一。
pub fn is_sys_button(hit: isize) -> bool {
    matches!(hit, HTMINBUTTON | HTMAXBUTTON | HTCLOSE)
}

/// 点三条按钮时发对应的系统命令。返回 `true` = 已处理。
///
/// 最小化/最大化走 `WM_SYSCOMMAND`（系统动画、任务栏状态同步都在里面）；
/// 关闭走 `WM_CLOSE` —— **不能直接 `DestroyWindow`**，那样会绕过
/// gui.rs 的「关闭到托盘」判断，用户勾了托盘也会被直接杀掉。
pub fn on_sys_button(hwnd: Handle, hit: isize) -> bool {
    match hit {
        HTMINBUTTON => {
            sys_command(hwnd, SC_MINIMIZE);
            true
        }
        HTMAXBUTTON => {
            if is_maximized(hwnd) {
                sys_command(hwnd, SC_RESTORE);
            } else {
                sys_command(hwnd, SC_MAXIMIZE);
            }
            true
        }
        HTCLOSE => {
            crate::ui::canvas::post_message(hwnd, WM_CLOSE, 0, 0);
            true
        }
        _ => false,
    }
}

/// 双击标题栏 → 最大化 / 还原。
pub fn toggle_maximize(hwnd: Handle) {
    if is_maximized(hwnd) {
        sys_command(hwnd, SC_RESTORE);
    } else {
        sys_command(hwnd, SC_MAXIMIZE);
    }
}

/// 发一个系统命令。
///
/// 用 `PostMessageW(WM_SYSCOMMAND)` 而不是直接调 `ShowWindow`：
/// 前者会走完整的系统流程（含 `WM_GETMINMAXINFO`、任务栏状态同步、动画）。
pub fn sys_command(hwnd: Handle, cmd: usize) {
    crate::ui::canvas::post_message(hwnd, WM_SYSCOMMAND, cmd, 0);
}

/// 处理 `WM_GETMINMAXINFO`：修正最大化时的位置和尺寸，别盖住任务栏。
///
/// 无边框窗口的经典坑：`WS_THICKFRAME` 让系统以为边框宽度是 8px，
/// 最大化后窗口比工作区大一圈 —— 任务栏被盖掉、右侧多出 8px 在屏幕外。
/// 修法是拿 `MonitorInfo.rc_work` 自己算。
///
/// `lparam` 指向 `MINMAXINFO`。字段布局（x64，每个字段 4 字节）：
/// ```c
/// 偏移 0  POINT ptReserved
/// 偏移 8  POINT ptMaxSize
/// 偏移 16 POINT ptMaxPosition
/// 偏移 24 POINT ptMinTrackSize
/// 偏移 32 POINT ptMaxTrackSize
/// ```
pub fn on_getminmaxinfo(hwnd: Handle, lparam: isize) {
    if lparam == 0 {
        return;
    }
    const MONITOR_DEFAULTTONEAREST: u32 = 2;
    let mon = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if mon.is_null() {
        return;
    }
    let mut mi = MonitorInfo::default();
    mi.cb_size = std::mem::size_of::<MonitorInfo>() as u32;
    if unsafe { GetMonitorInfoW(mon, &mut mi) } == 0 {
        return;
    }
    let work = mi.rc_work;
    let full = mi.rc_monitor;

    // SAFETY: `lparam` 是系统给的 `MINMAXINFO*`，按文档布局写。
    // 用 `*mut i32` 逐字段写而不是声明一个 Rust 结构体：那个结构体
    // 只在 Win32 这里用一次，多一个 `#[repr(C)]` 类型就多一处可能对不齐的地方。
    unsafe {
        let base = lparam as *mut i32;
        // ptMaxSize（偏移 8 = 第 2 个 i32 起）
        *base.add(2) = work.width();
        *base.add(3) = work.height();
        // ptMaxPosition（偏移 16 = 第 4 个 i32 起）：相对**显示器**原点的偏移，
        // 不是屏幕绝对坐标 —— 多显示器时写绝对坐标会让窗口跳到主屏左上角。
        *base.add(4) = work.left - full.left;
        *base.add(5) = work.top - full.top;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TW: f32 = 1100.0;
    const TH: f32 = 740.0;
    const TITLE: f32 = 36.0;

    #[test]
    fn nccalcsize_一律铺满整窗() {
        // 无论 wparam 为何值，都返回 0（客户区 = 整个窗口矩形）。
        // wparam==0 也拦，是为了杜绝窗口重显/SetWindowPos 时默认过程重新
        // 算出 21px 经典蓝条标题栏（用户实测的偶发蓝条）。
        assert_eq!(on_nccalcsize(1), Some(0));
        assert_eq!(on_nccalcsize(0), Some(0));
    }

    #[test]
    fn 关闭按钮优先于顶边缩放() {
        // 关闭按钮中心 ≈ (1100-23, 18)。若先判边缘，这里会返回 HTTOP 或
        // HTTOPRIGHT，用户点关闭变成拉窗口。
        let hit = logical_hit(TW - 23.0, 18.0, TW, TH, TITLE, false);
        assert_eq!(hit, HTCLOSE, "右上角被边缘缩放吃掉了");
        // 最小化按钮中心
        assert_eq!(logical_hit(TW - 115.0, 18.0, TW, TH, TITLE, false), HTMINBUTTON);
        // 最大化按钮中心
        assert_eq!(logical_hit(TW - 69.0, 18.0, TW, TH, TITLE, false), HTMAXBUTTON);
    }

    #[test]
    fn 标题栏空白处可拖动() {
        assert_eq!(logical_hit(300.0, 18.0, TW, TH, TITLE, false), HTCAPTION);
        // 内容区是普通客户区
        assert_eq!(logical_hit(300.0, 300.0, TW, TH, TITLE, false), HTCLIENT);
    }

    #[test]
    fn 四边四角缩放码正确() {
        // 左下角
        assert_eq!(logical_hit(2.0, TH - 2.0, TW, TH, TITLE, false), HTBOTTOMLEFT);
        // 右下角
        assert_eq!(logical_hit(TW - 2.0, TH - 2.0, TW, TH, TITLE, false), HTBOTTOMRIGHT);
        // 上边中点（避开标题栏按钮那一侧，且不在角上）
        assert_eq!(logical_hit(500.0, 2.0, TW, TH, TITLE, false), HTTOP);
        // 左边中点
        assert_eq!(logical_hit(2.0, 300.0, TW, TH, TITLE, false), HTLEFT);
        // 右边中点
        assert_eq!(logical_hit(TW - 2.0, 300.0, TW, TH, TITLE, false), HTRIGHT);
        // 下边中点
        assert_eq!(logical_hit(500.0, TH - 2.0, TW, TH, TITLE, false), HTBOTTOM);
    }

    #[test]
    fn 最大化时不给缩放码() {
        // 最大化后四边都该是标题栏/客户区，不能是缩放 ——
        // 否则想把窗口往下拖回来时碰到的是「上边缘缩放」，拖不动。
        assert_eq!(logical_hit(500.0, 2.0, TW, TH, TITLE, true), HTCAPTION);
        assert_eq!(logical_hit(500.0, TH - 2.0, TW, TH, TITLE, true), HTCLIENT);
        assert_eq!(logical_hit(2.0, 300.0, TW, TH, TITLE, true), HTCLIENT);
        // 但按钮仍然要能点
        assert_eq!(logical_hit(TW - 23.0, 18.0, TW, TH, TITLE, true), HTCLOSE);
    }

    #[test]
    fn 拖动码与系统定义一致() {
        // 这几个数字错了会静默变成「什么都不做」
        assert_eq!(HTMINBUTTON, 8);
        assert_eq!(HTMAXBUTTON, 9);
        assert_eq!(HTCLOSE, 20);
        assert_eq!(HTCAPTION, 2);
        assert_eq!(HTCLIENT, 1);
        assert_eq!((HTLEFT, HTRIGHT, HTTOP, HTBOTTOM), (10, 11, 12, 15));
        assert_eq!(
            (HTTOPLEFT, HTTOPRIGHT, HTBOTTOMLEFT, HTBOTTOMRIGHT),
            (13, 14, 16, 17)
        );
    }

    #[test]
    fn 拖动码与按钮码分类不重叠() {
        // HTCAPTION 和四边要交给系统做非客户区拖动
        assert!(is_nc_drag(HTCAPTION));
        assert!(is_nc_drag(HTLEFT));
        assert!(!is_nc_drag(HTCLIENT));
        // 三条按钮是「点一下就执行命令」，不是拖动
        assert!(is_sys_button(HTCLOSE));
        assert!(!is_nc_drag(HTCLOSE));
        assert!(!is_sys_button(HTCAPTION));
    }

    #[test]
    fn 样式位与win32头文件一致() {
        assert_eq!(WS_CAPTION, 0x00C0_0000);
        assert_eq!(WS_THICKFRAME, 0x0004_0000);
        assert_eq!(WS_SYSMENU, 0x0008_0000);
        // 去掉 caption 之后不能把 thickframe 一起带走
        let style: isize = WS_CAPTION | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
        let new_style =
            (style & !WS_CAPTION & !WS_SYSMENU) | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
        assert_eq!(new_style & WS_THICKFRAME, WS_THICKFRAME);
        assert_eq!(new_style & WS_CAPTION, 0);
    }
}
