//! 自绘标题栏。
//!
//! # 为什么要自绘
//!
//! 用户连提三次「菜单栏就是放大缩小的地方依旧没有沉浸」。前两次我都把它理解成
//! 「标题栏颜色和页面背景不一致」，于是给 `dwm.rs` 加了
//! `COLORREF_LIGHT_CAPTION = 0x00F3F3F3`，像素采样确认标题栏已经是 `#F3F3F3`。
//!
//! 那**不是他要的**。系统标题栏哪怕颜色一致，仍然带着自己的图标、标题、
//! 三条按钮 —— Win11 计算器/设置页的做法是**把系统标题栏整个去掉**，
//! 用自己的绘制填满那一条。这就是本模块存在的理由。
//!
//! # 窗口那边怎么配合
//!
//! 去掉系统标题栏在 `frame.rs` 里（`go_frameless`：拿掉 `WS_CAPTION`，
//! 保留 `WS_THICKFRAME` 以保住边缘缩放和 Aero Snap）。
//!
//! 去掉之后客户区顶部就空出一条，画的就是本模块。**拖动、双击最大化、
//! 三个按钮都由 `canvas.rs` 处理** —— 它们是窗口操作，不是业务逻辑，
//! 走 `frame::begin_drag` / `frame::sys_command` 转交给系统，
//! 这样 Aero Snap、最小化动画、关闭确认全都还是系统原生的行为。
//!
//! # 坐标
//!
//! 全部是**逻辑像素**，和页面同一套。`canvas.rs` 的世界变换会统一乘 `scale`。

use crate::ui::gfx::{Painter, R};
use crate::ui::theme::Theme;

/// 窗口标题（任务栏与系统元数据用）。
#[allow(dead_code)]
pub const APP_TITLE: &str = "R Battery";

/// 标题栏逻辑高度（96 DPI 下的像素）。
///
/// 36 而不是 32：11.5 号字（≈15px）上下各留 10px，视觉上和 Win11 原生
/// 标题栏接近，而三条按钮的 46px 宽度按老惯例取。
pub const TITLE_H: f32 = 36.0;

/// 三条按钮的逻辑宽度。
///
/// 46 是 Win11 的实测值（`SystemParametersInfo` 不直接给，是从原生窗口
/// 反推的：关闭按钮热区在标题栏最右侧 46px 内）。三条合计 138。
pub const BTN_W: f32 = 46.0;

/// 自绘标题栏上三个可点的按钮，从左到右。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Btn {
    Minimize,
    Maximize,
    Close,
}

impl Btn {
    /// 从左到右的顺序。命中测试和绘制都靠它，**两处必须一致**。
    pub const ALL: [Btn; 3] = [Btn::Minimize, Btn::Maximize, Btn::Close];
}

/// 某个按钮的逻辑矩形（窗口坐标，原点在客户区左上角）。
///
/// `win_w` 是逻辑宽度。按钮从右往左排：关闭最右。
pub fn btn_rect(b: Btn, win_w: f32) -> R {
    let i = match b {
        Btn::Minimize => 0,
        Btn::Maximize => 1,
        Btn::Close => 2,
    };
    // 关闭贴右边，往左依次排
    let from_right = (Btn::ALL.len() - 1 - i) as f32;
    R::new(win_w - BTN_W * (from_right + 1.0), 0.0, BTN_W, TITLE_H)
}

/// 命中测试：客户区坐标 `(x, y)` 落在哪个按钮上。
///
/// 返回 `None` 表示不是按钮 —— 落在标题栏其他位置就是**拖动区**，
/// 落在标题栏以下才是页面。两种情况调用方要分开处理。
pub fn hit(x: f32, y: f32, win_w: f32) -> Option<Btn> {
    if !(0.0..=TITLE_H).contains(&y) {
        return None;
    }
    Btn::ALL.iter().copied().find(|b| {
        let r = btn_rect(*b, win_w);
        x >= r.x && x <= r.right() && y >= r.y && y <= r.bottom()
    })
}

/// `(x, y)` 是不是落在标题栏那一条里（含按钮）。
///
/// 生产代码不用它 —— `frame::logical_hit` 里的 `title_h` 是参数（要按缩放
/// 传值），不能用这个常量。留着是给测试当判据：`命中测试按按钮边界` 用它
/// 确认「不是按钮但仍属于标题栏」这一档，正是拖动区。
#[allow(dead_code)]
pub fn in_bar(y: f32) -> bool {
    (0.0..=TITLE_H).contains(&y)
}

/// 画标题栏底色。标题栏去掉了原有的文字和图标，保持极简沉浸式背景。
/// 三条窗口控制按钮见 [`draw_buttons`]。
///
/// `nav_w` 是左侧导航栏宽度：导航栏是整列 `bg_nav`（比内容略深的灰），
/// 标题栏这一条也必须把左栏那一段单独铺成 `bg_nav`，否则左栏顶部 36px
/// 会是一块 `bg` 色 —— 与下方整列 `bg_nav` 形成一道横向色差缝（用户反馈
/// 「折叠区和上方颜色不一样，很丑」）。内容区那一段仍是 `bg`，与内容连成一片。
pub fn draw(p: &mut Painter, win_w: f32, nav_w: f32, theme: &Theme) {
    let t = theme;
    let nw = nav_w.clamp(0.0, win_w);
    if nw > 0.0 {
        // 左栏那一段：与导航栏同色
        p.rect(R::new(0.0, 0.0, nw, TITLE_H), t.bg_nav);
        // 栏与内容之间的 1px 分隔线，也贯穿标题栏这一条
        p.line(nw - 0.5, 0.0, nw - 0.5, TITLE_H, t.border, 1.0);
    }
    // 内容那一段：和页面背景同一个色，连成一片
    p.rect(R::new(nw, 0.0, win_w - nw, TITLE_H), t.bg);
}

/// 画 exe 的图标（电池形状，和托盘图标同一套笔画）。
///
/// 不复用 `LoadIconW` 再 `DrawIconEx` 这条路：那是 GDI 的 16×16 位图，
/// 在 125%/150% 缩放屏上会被位图拉伸得发虚 —— 正是用户抱怨的「图标分辨率低」。
/// 用矢量笔画重新画一遍，任何缩放下都是清的。
#[allow(dead_code)]
fn draw_app_icon(p: &mut Painter, r: R, t: &Theme) {
    // 深色圆角底 + 白色电池，和 `assets/tray.ico` 的 256px 版一致
    p.round(r, 3.0, t.accent);
    let body = R::new(r.x + 2.5, r.cy() - 3.5, r.w - 6.0, 7.0);
    p.round_stroke(body, 1.5, t.accent_fg, 1.4);
    // 正极触点
    p.round(
        R::new(body.right() + 0.5, r.cy() - 1.5, 1.6, 3.0),
        0.6,
        t.accent_fg,
    );
    // 电量条（实心，表示「有电」）
    p.round(
        R::new(body.x + 1.8, body.y + 1.8, body.w * 0.55, body.h - 3.6),
        0.6,
        t.accent_fg,
    );
}

/// Win11 关闭按钮的 hover 底色。
///
/// **必须带不透明 alpha（`0xFF…`）**：`Painter` 的颜色是 `0xAARRGGBB`，
/// 传 `0x00C42B1C` 会被 GDI+ 当成全透明 —— 悬停时红底和白色的叉都画不出来，
/// 表现就是用户反馈的「鼠标移到关闭按钮上 X 会消失」。同理前景色也得是
/// `0xFFFFFFFF`，`0x00FFFFFF` 画出来是透明的。
const CLOSE_HOVER: u32 = 0xFFC4_2B1C;

/// 画三条窗口按钮。
///
/// 关闭按钮在 hover 时是红底（`#C42B1C`，Win11 的关闭色），另外两个是
/// 淡灰底。这是 Windows 的既定习惯，不做自定义。
pub fn draw_buttons(p: &mut Painter, win_w: f32, t: &Theme, is_max: bool, hover: Option<Btn>) {
    for b in Btn::ALL {
        let r = btn_rect(b, win_w);
        let hot = hover == Some(b);
        if hot {
            let bg = if b == Btn::Close {
                CLOSE_HOVER
            } else {
                crate::ui::gfx::with_alpha(t.text, 0x14)
            };
            p.rect(r, bg);
        }
        let fg = if hot && b == Btn::Close {
            0xFFFF_FFFF
        } else {
            t.text
        };
        let cx = r.cx();
        let cy = r.cy();
        match b {
            // 最小化：一条横线
            Btn::Minimize => p.line(cx - 5.0, cy, cx + 5.0, cy, fg, 1.2),
            // 最大化/还原：方框（还原是两个错开的方框）
            Btn::Maximize => {
                if is_max {
                    // 还原：两个错开的方框
                    p.round_stroke(R::new(cx - 5.0, cy - 3.0, 7.0, 7.0), 1.0, fg, 1.1);
                    p.line(cx - 3.0, cy - 5.0, cx + 5.0, cy - 5.0, fg, 1.1);
                    p.line(cx + 5.0, cy - 5.0, cx + 5.0, cy + 3.0, fg, 1.1);
                    p.line(cx + 5.0, cy + 3.0, cx + 2.0, cy + 3.0, fg, 1.1);
                } else {
                    p.round_stroke(R::new(cx - 4.5, cy - 4.5, 9.0, 9.0), 1.0, fg, 1.1);
                }
            }
            // 关闭：X
            Btn::Close => {
                p.line(cx - 4.5, cy - 4.5, cx + 4.5, cy + 4.5, fg, 1.2);
                p.line(cx + 4.5, cy - 4.5, cx - 4.5, cy + 4.5, fg, 1.2);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 三条按钮从右往左排且不重叠() {
        let w = 1100.0;
        let mn = btn_rect(Btn::Minimize, w);
        let mx = btn_rect(Btn::Maximize, w);
        let cl = btn_rect(Btn::Close, w);
        // 关闭贴右边界
        assert!(
            (cl.right() - w).abs() < 0.01,
            "关闭按钮没贴右边: {}",
            cl.right()
        );
        // 依次左移，宽度一致。
        //
        // 断言写成「**前一个的右边界 == 后一个的左边界**」而不是
        // 「`mx.right() - mn.x`」—— 后者把「右边界」和「左边界」两回事混在
        // 一起算，差了一个按钮宽度，恒不成立（我第一版就是这么写的）。
        assert!(
            (mn.right() - mx.x).abs() < 0.01,
            "最小化和最大化之间有空隙或重叠: {} vs {}",
            mn.right(),
            mx.x
        );
        assert!(
            (mx.right() - cl.x).abs() < 0.01,
            "最大化和关闭之间有空隙或重叠: {} vs {}",
            mx.right(),
            cl.x
        );
        // 三条合计 138，最小化左边还给标题文字留出空间
        assert!((cl.right() - mn.x).abs() - BTN_W * 3.0 < 0.01);
        // 不重叠
        assert!(mn.x < mx.x && mx.x < cl.x);
        assert!(mn.w == BTN_W && mx.w == BTN_W && cl.w == BTN_W);
    }

    #[test]
    fn 命中测试按按钮边界() {
        let w = 1100.0;
        let cl = btn_rect(Btn::Close, w);
        // 关闭按钮中心
        assert_eq!(hit(cl.cx(), cl.cy(), w), Some(Btn::Close));
        // 标题栏左侧 = 可拖动区（没有按钮）
        assert_eq!(hit(60.0, cl.cy(), w), None);
        // 但仍在标题栏里 → 调用方据此走拖动而不是页面点击
        assert!(in_bar(cl.cy()));
        // 标题栏下方 = 不算标题栏
        assert_eq!(hit(cl.cx(), TITLE_H + 1.0, w), None);
        assert!(!in_bar(TITLE_H + 1.0));
        // 负 y（拖出窗口）
        assert_eq!(hit(cl.cx(), -1.0, w), None);
    }

    #[test]
    fn 标题栏高度能容纳按钮() {
        // 按钮高度等于标题栏高度：矮了按钮会有一半在标题栏外面，命中测试
        // 用的是同一个 y 范围，那种情况下点不到
        for b in Btn::ALL {
            assert_eq!(btn_rect(b, 800.0).h, TITLE_H);
        }
    }
}
