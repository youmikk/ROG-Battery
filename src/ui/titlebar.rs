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

/// 标题栏逻辑高度（96 DPI 下的像素）。
///
/// 36 而不是 32：11.5 号字（≈15px）上下各留 10px，视觉上和 Win11 原生
/// 标题栏接近，而三条按钮的 46px 宽度按老惯例取。
pub const TITLE_H: f32 = 36.0;

pub const BTN_W: f32 = 46.0;

/// 自绘标题栏上三个可点的按钮，从左到右。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Btn {
    Minimize,
    Maximize,
    Close,
}

impl Btn {
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

pub fn hit(x: f32, y: f32, win_w: f32) -> Option<Btn> {
    if !(0.0..=TITLE_H).contains(&y) {
        return None;
    }
    Btn::ALL.iter().copied().find(|b| {
        let r = btn_rect(*b, win_w);
        x >= r.x && x <= r.right() && y >= r.y && y <= r.bottom()
    })
}

/// 品牌位置固定，与页面宽度和滚动无关。
pub fn draw(p: &mut Painter, win_w: f32, theme: &Theme) {
    p.rect(R::new(0.0, 0.0, win_w, TITLE_H), theme.bg);
    let r = logo_rect();
    p.image_rounded(crate::ui::gfx::app_icon_bitmap(), r, 5.0);
    p.text(
        crate::i18n::translate(crate::gui::WINDOW_TITLE),
        R::new(r.right() + 9.0, 0.0, 260.0, TITLE_H),
        12.5,
        true,
        theme.text,
        crate::ui::gfx::Align::Left,
    );
}

pub fn logo_rect() -> R {
    let d = 22.0;
    R::new(14.0, (TITLE_H - d) / 2.0, d, d)
}

pub fn settings_rect(win_w: f32) -> R {
    R::new(win_w - BTN_W * 4.0, 0.0, BTN_W, TITLE_H)
}

pub fn draw_settings(p: &mut Painter, ui: &mut crate::ui::widget::Ui, win_w: f32) {
    use crate::ui::widget::{Act, Page};
    ui.chrome_hot(R::new(0.0, 0.0, win_w, TITLE_H), Act::None);
    let r = settings_rect(win_w);
    let act = Act::Nav(Page::Settings);
    let active = ui.page == Page::Settings;
    if ui.hover == Some(act) {
        p.rect(r, crate::ui::gfx::with_alpha(p.t.text, 0x14));
    }
    let color = if active { p.t.accent } else { p.t.text2 };
    let mut teeth = Vec::with_capacity(37);
    for tooth in 0..6 {
        for (offset, radius) in [
            (-0.5, GLYPH_HALF * 0.76),
            (-0.28, GLYPH_HALF * 0.76),
            (-0.16, GLYPH_HALF),
            (0.16, GLYPH_HALF),
            (0.28, GLYPH_HALF * 0.76),
            (0.5, GLYPH_HALF * 0.76),
        ] {
            let angle = (tooth as f32 + offset) * std::f32::consts::TAU / 6.0
                - std::f32::consts::FRAC_PI_2;
            teeth.push((r.cx() + angle.cos() * radius, r.cy() + angle.sin() * radius));
        }
    }
    teeth.push(teeth[0]);
    p.stroke_path(&teeth, color, 1.0);
    p.dot_ring(r.cx(), r.cy(), 1.8, color, 1.0);
    ui.chrome_hot(r, act);
}

/// 三条按钮图标的笔宽与几何（逻辑像素）。
///
/// 取值照 Win11 原生按钮的观感：主笔 1.2、细笔 1.1，图标半边长 5 —— 也就是
/// 最宽 10px 的图形摆在 46px 宽的按钮正中，左右各留 18px 空白。
const GLYPH_HALF: f32 = 5.0;
const BOX_HALF: f32 = 4.5;
const RESTORE_BOX: f32 = 7.0;
const STROKE: f32 = 1.2;
const STROKE_THIN: f32 = 1.1;

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
            Btn::Minimize => p.line(
                cx - GLYPH_HALF,
                cy,
                cx + GLYPH_HALF,
                cy,
                fg,
                STROKE,
            ),
            // 最大化/还原：方框（还原是两个错开的方框）
            Btn::Maximize => {
                if is_max {
                    // 还原：两个错开的方框
                    // 后框（右下错开）先画、前框后画，与原生一致
                    p.round_stroke(
                        R::new(cx - GLYPH_HALF, cy - 3.0, RESTORE_BOX, RESTORE_BOX),
                        1.0,
                        fg,
                        STROKE_THIN,
                    );
                    p.line(
                        cx - 3.0,
                        cy - GLYPH_HALF,
                        cx + GLYPH_HALF,
                        cy - GLYPH_HALF,
                        fg,
                        STROKE_THIN,
                    );
                    p.line(
                        cx + GLYPH_HALF,
                        cy - GLYPH_HALF,
                        cx + GLYPH_HALF,
                        cy + 3.0,
                        fg,
                        STROKE_THIN,
                    );
                    p.line(
                        cx + GLYPH_HALF,
                        cy + 3.0,
                        cx + 2.0,
                        cy + 3.0,
                        fg,
                        STROKE_THIN,
                    );
                } else {
                    p.round_stroke(
                        R::new(cx - BOX_HALF, cy - BOX_HALF, BOX_HALF * 2.0, BOX_HALF * 2.0),
                        1.0,
                        fg,
                        STROKE_THIN,
                    );
                }
            }
            // 关闭：X
            Btn::Close => {
                p.line(cx - BOX_HALF, cy - BOX_HALF, cx + BOX_HALF, cy + BOX_HALF, fg, STROKE);
                p.line(cx + BOX_HALF, cy - BOX_HALF, cx - BOX_HALF, cy + BOX_HALF, fg, STROKE);
            }
        }
    }
}
