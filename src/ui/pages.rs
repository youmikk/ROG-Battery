//! 五个页面的绘制。
//!
//! # 坐标约定（改这里之前先读）
//!
//! 所有页面代码都在**内容坐标**里工作：`y = 0` 是页面顶部，滚动由
//! `gfx::Canvas::paint` 的世界变换统一施加。所以这里**没有任何一处**出现
//! `* scale` 或 `- scroll`。想看某个元素现在在屏幕上的哪里，把逻辑 y 减掉
//! 滚动量再乘缩放，那是 canvas 的事，不是页面的事。
//!
//! `x` 有两个体系：
//! - [`Frame::content`] 的 `x` 已经含左侧内边距，页面元素从它开始排；
//! - 侧栏（[`nav`]）用的是**窗口坐标**，因为它不随滚动移动。
//!
//! # 为什么绘制和命中共用一个函数
//!
//! 即时模式：画到哪个可交互元素，就顺手 `ui.hot(rect, act)` 登记。
//! 顺序很重要 —— **先登记再画**，这样「画出来的」和「能点的」必然同一份矩形。
//! 反过来（先画再登记）也没错，但一旦中间有 `return` 就会漏登记，
//! 表现是「这一块看得见但点不动」，很难查。
//!
//! # 单位
//!
//! 全部是**逻辑像素**（96 DPI 下的 px），和设计稿 `ui-prototype.html` 的
//! CSS px 一一对应。这样对着原型调间距时不用换算。

#![allow(dead_code)]

use crate::ui::gfx::{with_alpha, Align, Painter, R};
use crate::ui::layout::{self, font, Breakpoint, Frame};
use crate::ui::theme::Theme;
use crate::ui::widget::{
    Act, Drop, Edit, Icon, Page, Pref, Slide, Ui,
};

/// 页面要显示的数据。由 `gui.rs` 从 `Shared` 里凑出来。
pub struct Data<'a> {
    /// 设备状态。`None` = 还没读到（首帧 / 后台线程正在读）。
    pub state: Option<&'a crate::proto::State>,
    /// 设备身份（型号、SN、固件）。
    pub ident: Option<&'a crate::proto::Identity>,
    /// 状态行文字（「已连接 · 84%」「鼠标休眠中，移动一下即可唤醒」…）。
    pub status: String,
    /// 曲线数据。`None` = 还没读完第一遍（该画「正在读取」而不是「样本不足」）。
    pub plot: Option<&'a crate::chart::Plot>,
    /// 续航估算。
    pub estimate: &'a crate::history::Estimate,
    /// 历史样本条数（`plot` 里可能包含断档前的旧点，这里给的是有效条数）。
    pub rows: usize,
    /// 本地设置（托盘开关、刷新间隔…）。
    pub cfg: &'a crate::settings::Settings,
    /// 开机自启的当前状态文字。
    pub autostart_text: &'a str,
    /// 版本号（侧栏底部）。
    pub version: &'a str,
    /// 帧耗时（侧栏底部「19 帧 · 588 ms」）。
    pub frames: usize,
    pub elapsed_ms: u128,
    /// 数据是否来自缓存。
    pub from_cache: bool,
}

// ───────────────────────── 通用小工具 ─────────────────────────

/// 垂直推进游标。页面的所有 y 都靠它往前走。
///
/// 用游标而不是一堆手算的常量：加一行不用改后面所有数字，
/// 而且「这一行到底多高」只有一处定义。
struct Cursor {
    y: f32,
    x: f32,
    w: f32,
}

impl Cursor {
    fn new(f: &Frame) -> Cursor {
        Cursor {
            y: f.content.y,
            x: f.content.x,
            w: f.content.w,
        }
    }

    fn skip(&mut self, h: f32) -> f32 {
        let top = self.y;
        self.y += h;
        top
    }

    /// 页标题（h1）。原型：26px / 600 / letter-spacing −.4 / margin 10px 0 20px。
    fn h1(&mut self, p: &mut Painter, s: &str) {
        let top = self.y + 10.0;
        p.text(
            s,
            R::new(self.x, top, self.w, layout::H1_H),
            font::H1,
            true,
            p.t.text,
            Align::Left,
        );
        self.y = top + layout::H1_H + 20.0;
    }

    /// 分组标题（h2）。原型：14px / 600 / text2 / margin 26px 0 10px，
    /// 但紧跟 h1 的第一个 h2 上边距只有 4px（原型 `first-of-type`）。
    fn h2(&mut self, p: &mut Painter, s: &str, first: bool) {
        let top = self.y + if first { 4.0 } else { 26.0 };
        p.text(
            s,
            R::new(self.x, top, self.w, layout::H2_H),
            font::H2,
            true,
            p.t.text2,
            Align::Left,
        );
        self.y = top + layout::H2_H + 10.0;
    }

    /// 卡片背景。返回卡片的矩形（内容坐标）。
    fn card(&mut self, p: &mut Painter, h: f32) -> R {
        self.card_w(p, self.w, h)
    }

    /// 指定宽度的卡片。用于并排的两张卡（hero + 侧栏）。
    ///
    /// 不推进游标 —— 并排的卡片共用一个 `y`，谁都不该先跳走。
    /// 调用方要么自己 `c.y = card.bottom()`，要么再画右卡。
    fn card_w(&mut self, p: &mut Painter, w: f32, h: f32) -> R {
        let r = R::new(self.x, self.y, w, h);
        p.card(r, layout::R_CARD, p.t.card, p.t.border);
        r
    }
}

/// 一行「标签 + 副标题」，左侧，返回剩余给控件的矩形。
///
/// 原型 `.row > .lbl`：`flex: 1`，`.t` 13.5px、`.d` 11.5px text3（`margin-top: 2px`）。
fn row_label(p: &mut Painter, area: R, title: &str, desc: Option<&str>, ctl_w: f32) -> R {
    let text_w = (area.w - ctl_w - 18.0).max(40.0);
    if let Some(d) = desc {
        p.text(
            title,
            R::new(area.x, area.y + 8.0, text_w, 20.0),
            font::BODY,
            false,
            p.t.text,
            Align::Left,
        );
        p.text(
            d,
            R::new(area.x, area.y + 28.0, text_w, 16.0),
            font::HINT,
            false,
            p.t.text3,
            Align::Left,
        );
    } else {
        p.text(
            title,
            R::new(area.x, area.y, text_w, area.h),
            font::BODY,
            false,
            p.t.text,
            Align::Left,
        );
    }
    R::new(area.right() - ctl_w, area.y, ctl_w, area.h)
}

/// 分段控件（`.seg`）。返回是否命中，命中时给出第几段。
///
/// 原型：外层 `padding:2px`、`gap:2px`、圆角 4；内层按钮高 26、padding 0 12、圆角 3。
fn segmented(
    p: &mut Painter,
    ui: &mut Ui,
    r: R,
    labels: &[&str],
    selected: usize,
    act_of: impl Fn(usize) -> Act,
) {
    p.round(r, layout::R_CTL, p.t.bg_sunken);
    p.round_stroke(r, layout::R_CTL, p.t.border_strong, 1.0);
    let inner = r.inset(2.0);
    let n = labels.len().max(1);
    let seg_w = (inner.w - 2.0 * (n as f32 - 1.0)) / n as f32;
    for (i, label) in labels.iter().enumerate() {
        let x = inner.x + i as f32 * (seg_w + 2.0);
        let sr = R::new(x, inner.y, seg_w, inner.h);
        let on = i == selected;
        if on {
            p.round(sr, 3.0, p.t.accent);
        } else if ui.hover == Some(act_of(i)) {
            p.round(sr, 3.0, with_alpha(p.t.text, 0x0A));
        }
        let fg = if on { p.t.accent_fg } else { p.t.text2 };
        p.text(label, sr, 12.0, on, fg, Align::Center);
        ui.hot(sr, act_of(i));
    }
}

/// 下拉框外观（`.select`）。点击区域是整个矩形。
fn select_box(p: &mut Painter, r: R, text: &str, open: bool) {
    p.round(r, layout::R_CTL, p.t.bg_sunken);
    p.round_stroke(
        r,
        layout::R_CTL,
        if open { p.t.accent } else { p.t.border_strong },
        1.0,
    );
    p.text(
        text,
        R::new(r.x + 10.0, r.y, r.w - 30.0, r.h),
        12.5,
        false,
        p.t.text,
        Align::Left,
    );
    // 右侧小三角（原型 `.caret`，9px text3）
    let cx = r.right() - 14.0;
    let cy = r.cy();
    p.polyline(
        &[(cx - 4.0, cy - 2.0), (cx, cy + 2.5), (cx + 4.0, cy - 2.0)],
        p.t.text3,
        1.4,
    );
}

/// 主按钮 / 次按钮（`.btn`）。
fn button(p: &mut Painter, ui: &mut Ui, r: R, label: &str, primary: bool, act: Act) {
    let hovered = ui.hover == Some(act);
    if primary {
        p.round(r, layout::R_CTL, p.t.accent);
        p.text(label, r, 12.5, true, p.t.accent_fg, Align::Center);
    } else {
        let bg = if hovered { p.t.card_hover } else { p.t.card };
        p.round(r, layout::R_CTL, bg);
        p.round_stroke(r, layout::R_CTL, p.t.border_strong, 1.0);
        p.text(label, r, 12.5, false, p.t.text, Align::Center);
    }
    ui.hot(r, act);
}

/// 数字输入框（`.num`）。`active` = 正在编辑，边框变强调色。
fn num_box(p: &mut Painter, r: R, text: &str, active: bool) {
    p.round(r, layout::R_CTL, p.t.bg_sunken);
    p.round_stroke(
        r,
        layout::R_CTL,
        if active { p.t.accent } else { p.t.border_strong },
        1.0,
    );
    // 原型 text-align: right + padding 0 9
    p.text(
        text,
        R::new(r.x + 4.0, r.y, r.w - 13.0, r.h),
        12.5,
        false,
        p.t.text,
        Align::Right,
    );
}

/// 颜色小方块（`.swatch`，30×30 圆角 4）。`rgb == None` = 读不到设备，
/// 画成空框 + 斜线，而不是黑块 —— 黑块是个**合法的颜色**，用户会以为
/// 那真是鼠标当前的颜色。
fn swatch(p: &mut Painter, r: R, rgb: Option<[u8; 3]>) {
    match rgb {
        Some(c) => p.round(
            r,
            layout::R_CTL,
            with_alpha(
                0xFF00_0000 | ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32,
                0xFF,
            ),
        ),
        None => p.round(r, layout::R_CTL, p.t.bg_sunken),
    }
    p.round_stroke(r, layout::R_CTL, p.t.border_strong, 1.0);
    if rgb.is_none() {
        // 一道从左上到右下的细线，跟「无颜色」的通用画法一致
        p.line(
            r.x + 6.0,
            r.y + 6.0,
            r.right() - 6.0,
            r.bottom() - 6.0,
            p.t.text3,
            1.0,
        );
    }
}

/// 胶囊标签（`.badge`）。
fn badge(p: &mut Painter, r: R, label: &str, fg: u32, bg: u32) {
    p.pill(r, label, fg, bg, 11.0);
}

/// 电量环的颜色，三态加一个「读不到」的降级态。
///
/// 抽成独立函数只为一件事：**让优先级可以被单独测试**。
/// 它有条容易写错、写错了也不会报错的规则 ——
///
/// 1. 读不到设备 → `track`（灰）。不能落到红色低电量分支：0 是「电量耗尽」
///    的意思，而休眠时真实电量可能是 77%，显示红色等于报假警。
/// 2. 充电中 → `green`。**必须先于低电量判断**：充电时电量往往正好很低，
///    顺序反了会把「正在充」显示成红色。
/// 3. ≤20% → `red`。
/// 4. 其余 → `accent`（蓝）。
fn ring_color(t: &Theme, has_state: bool, charging: bool, level: u8) -> u32 {
    if !has_state {
        t.track
    } else if charging {
        t.green
    } else if level <= 20 {
        t.red
    } else {
        t.accent
    }
}

/// 说明条（`.note`）。警告用 warn 配色，中性用 sunken。
///
/// 返回它占的高度，方便调用方推进游标。
fn note(p: &mut Painter, r: R, text: &str, warn: bool) -> f32 {
    let (bg, fg) = if warn {
        (p.t.warn_soft, p.t.warn)
    } else {
        (p.t.bg_sunken, p.t.text2)
    };
    // 高度按文本长度估：13.5px 中文一行约 40 字，每行 1.5 倍行高。
    // 这不是精确排版，但说明条只有一两行，误差在 2px 内看不出来。
    let per_line = ((r.w - 60.0) / 13.5).max(12.0);
    let lines = (text.chars().count() as f32 / per_line).ceil().max(1.0);
    let h = 10.0 * 2.0 + lines * 12.0 * 1.5;
    let rr = R::new(r.x, r.y, r.w, h);
    p.round(rr, layout::R_CTL, bg);
    if warn {
        // 加粗的「!」——原型 `.ni`
        p.text(
            "!",
            R::new(rr.x + 13.0, rr.y + 10.0, 8.0, 16.0),
            12.0,
            true,
            fg,
            Align::Center,
        );
    }
    p.text_wrap(
        text,
        R::new(rr.x + 28.0, rr.y + 10.0, rr.w - 42.0, rr.h - 20.0),
        12.0,
        fg,
    );
    h
}

// ───────────────────────── 侧栏 ─────────────────────────

/// 侧栏用的图标。用线段画，不加载图片 —— 一个 16×16 的图标走 GDI+
/// 只要 5 条线，比解一个 PNG 再缓存的成本低得多。
fn draw_icon(p: &mut Painter, icon: Icon, r: R, color: u32, width: f32) {
    let x = r.x;
    let y = r.y;
    let w = r.w;
    let h = r.h;
    match icon {
        // 电池：外框 + 正极小凸起 + 内部电量条
        Icon::Battery => {
            let body = R::new(x, y + h * 0.22, w * 0.82, h * 0.56);
            p.round_stroke(body, 2.5, color, width);
            p.rect(R::new(x + w * 0.84, y + h * 0.38, w * 0.10, h * 0.24), color);
            p.round(
                R::new(body.x + 2.5, body.y + 2.5, body.w * 0.45, body.h - 5.0),
                1.2,
                color,
            );
        }
        // 性能：三条上升的柱子
        Icon::Perf => {
            for (i, frac) in [0.45f32, 0.72, 1.0].iter().enumerate() {
                let bh = h * 0.62 * frac;
                let bx = x + w * (0.12 + 0.30 * i as f32);
                p.round(
                    R::new(bx, y + h * 0.80 - bh, w * 0.18, bh),
                    1.0,
                    color,
                );
            }
        }
        // 灯光：灯泡轮廓 + 底座
        Icon::Light => {
            p.dot_ring(x + w * 0.5, y + h * 0.42, w * 0.30, color, width);
            p.line(
                x + w * 0.38,
                y + h * 0.76,
                x + w * 0.62,
                y + h * 0.76,
                color,
                width,
            );
            p.line(
                x + w * 0.42,
                y + h * 0.88,
                x + w * 0.58,
                y + h * 0.88,
                color,
                width,
            );
        }
        // 按键：一个圆角方框 + 中间一个小凹
        Icon::Keys => {
            p.round_stroke(
                R::new(x + w * 0.10, y + h * 0.20, w * 0.80, h * 0.58),
                3.0,
                color,
                width,
            );
            p.line(
                x + w * 0.30,
                y + h * 0.50,
                x + w * 0.70,
                y + h * 0.50,
                color,
                width,
            );
        }
        // 设置：齿轮用「圆 + 四个方向齿」近似
        Icon::Settings => {
            p.dot_ring(x + w * 0.5, y + h * 0.5, w * 0.24, color, width);
            for (dx, dy) in [(0.0f32, -1.0f32), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
                p.line(
                    x + w * 0.5 + dx * w * 0.28,
                    y + h * 0.5 + dy * h * 0.28,
                    x + w * 0.5 + dx * w * 0.42,
                    y + h * 0.5 + dy * h * 0.42,
                    color,
                    width,
                );
            }
        }
    }
}

/// 画左侧导航（**窗口坐标**，不随滚动移动）。
pub fn nav(p: &mut Painter, ui: &mut Ui, d: &Data, nav_w: f32, win_h: f32) {
    let t = p.t;
    // 底色 + 右边一条 1px 分隔线
    p.rect(R::new(0.0, 0.0, nav_w, win_h), t.bg_nav);
    p.line(nav_w - 0.5, 0.0, nav_w - 0.5, win_h, t.border, 1.0);

    let rail = ui.rail;
    let pad = if rail { 8.0 } else { 8.0 };
    let item_h = layout::NAV_ITEM_H;

    // ---- 设备卡（仅展开时）----
    let mut y = 14.0;
    if !rail {
        let dc = R::new(4.0, y, nav_w - 12.0, 66.0);
        p.card(dc, 8.0, t.card, t.border);
        let model = d
            .ident
            .map(|i| i.model_name.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("ROG 鼠标");
        p.text(
            model,
            R::new(dc.x + 12.0, dc.y + 10.0, dc.w - 24.0, 18.0),
            13.0,
            true,
            t.text,
            Align::Left,
        );
        p.text(
            "无线 · 2.4G",
            R::new(dc.x + 12.0, dc.y + 28.0, dc.w - 24.0, 14.0),
            font::TINY,
            false,
            t.text3,
            Align::Left,
        );
        // 「使用中」+ 电量。绿点用 dot，文字靠右。
        let line_y = dc.y + 46.0;
        let connected = d.state.is_some();
        p.dot(
            dc.x + 15.0,
            line_y + 6.0,
            3.0,
            if connected { t.green } else { t.text3 },
        );
        p.text(
            if connected { "使用中" } else { "休眠中" },
            R::new(dc.x + 23.0, line_y - 2.0, 60.0, 16.0),
            12.0,
            false,
            t.text2,
            Align::Left,
        );
        if let Some(st) = d.state {
            let pct = format!("{}%", st.power);
            p.text(
                &pct,
                R::new(dc.right() - 50.0, line_y - 2.0, 42.0, 16.0),
                12.0,
                true,
                t.text,
                Align::Right,
            );
        }
        y = dc.bottom() + 12.0;
    }

    // ---- 五个导航项 ----
    for page in Page::ALL {
        let r = if rail {
            R::new((nav_w - 38.0) / 2.0, y, 38.0, item_h)
        } else {
            R::new(pad, y, nav_w - pad * 2.0, item_h)
        };
        let act = Act::Nav(page);
        let on = ui.page == page;
        if on {
            p.round(r, layout::R_CTL, t.card);
            // 左侧 3×15 圆角 2 的强调竖条，贴边
            if rail {
                p.round(
                    R::new(r.x + 2.0, r.cy() - 7.5, 3.0, 15.0),
                    1.5,
                    t.accent,
                );
            } else {
                p.round(R::new(0.0, r.cy() - 7.5, 3.0, 15.0), 1.5, t.accent);
            }
        } else if ui.hover == Some(act) {
            p.round(r, layout::R_CTL, with_alpha(t.text, 0x0C));
        }
        let fg = if on { t.accent } else { t.text };
        if rail {
            let ir = R::new(r.cx() - 8.0, r.cy() - 8.0, 16.0, 16.0);
            draw_icon(p, page.icon(), ir, fg, 1.5);
        } else {
            let ir = R::new(r.x + 12.0, r.cy() - 8.0, 16.0, 16.0);
            draw_icon(p, page.icon(), ir, fg, 1.5);
            p.text(
                page.title(),
                R::new(ir.right() + 11.0, r.y, r.w - 45.0, r.h),
                font::NAV,
                on,
                fg,
                Align::Left,
            );
        }
        ui.chrome_hot(r, act);
        y += item_h + 1.0;
    }

    // ---- 底部：折叠按钮 + 版本 ----
    // 用户 m09924：「切换收缩的功能放在左侧功能栏最下方」
    let foot_y = (win_h - 30.0 - 10.0).max(y + 12.0);
    let toggle = if rail {
        R::new((nav_w - 38.0) / 2.0, foot_y, 38.0, 30.0)
    } else {
        R::new(pad, foot_y, nav_w - pad * 2.0, 30.0)
    };
    let tact = Act::ToggleRail;
    if ui.hover == Some(tact) {
        p.round(toggle, layout::R_CTL, with_alpha(t.text, 0x0C));
    }
    // 折叠箭头：展开时指左（「收起来」），收起时指右（「展开」）
    {
        let cx = if rail { toggle.cx() } else { toggle.x + 14.0 };
        let cy = toggle.cy();
        let dir = if rail { 1.0 } else { -1.0 };
        p.polyline(
            &[
                (cx + 2.5 * dir, cy - 4.0),
                (cx - 2.5 * dir, cy),
                (cx + 2.5 * dir, cy + 4.0),
            ],
            t.text2,
            1.6,
        );
    }
    if !rail {
        p.text(
            "收起侧栏",
            R::new(toggle.x + 30.0, toggle.y, toggle.w - 40.0, toggle.h),
            font::HINT,
            false,
            t.text2,
            Align::Left,
        );
    }
    ui.chrome_hot(toggle, tact);

    // 版本 + 帧统计（只在展开时显示，收起时没有空间）
    if !rail {
        let stat_y = foot_y + 34.0;
        if stat_y + 14.0 < win_h - 4.0 {
            let s = if d.frames > 0 {
                format!("{} 帧 · {} ms", d.frames, d.elapsed_ms)
            } else {
                String::new()
            };
            p.text(
                &s,
                R::new(pad + 4.0, stat_y, nav_w - 80.0, 14.0),
                font::TINY,
                false,
                t.text3,
                Align::Left,
            );
            p.text(
                d.version,
                R::new(pad, stat_y, nav_w - pad * 2.0 - 4.0, 14.0),
                font::TINY,
                false,
                t.text3,
                Align::Right,
            );
        }
    }
}

// ───────────────────────── 入口 ─────────────────────────

/// 画当前页正文（内容坐标，滚动由 canvas 施加）。
pub fn page(p: &mut Painter, ui: &mut Ui, d: &Data, f: &Frame) {
    let mut c = Cursor::new(f);
    match ui.page {
        Page::Battery => battery(p, ui, d, f, &mut c),
        Page::Perf => perf(p, ui, d, f, &mut c),
        Page::Light => light(p, ui, d, f, &mut c),
        Page::Keys => keys(p, ui, d, f, &mut c),
        Page::Settings => settings(p, ui, d, f, &mut c),
    }
    // 底部留白：不留的话最后一个卡片会紧贴窗口底边，
    // 而且滚到底时「最后一行是否完整可见」会算错。
    let total = c.y + layout::PAGE_PAD_BOTTOM;
    ui.set_content_h(ui.page, total);

    // 下拉弹层画在最上层（它的登记项最后加入 → 命中优先级最高）
    if let Some(drop) = ui.open_drop() {
        draw_drop(p, ui, d, drop);
    }

    // 提示浮层画在最后面、且**钉在视口底部**（不随滚动走）。
    draw_toast(p, ui, d, f);
}

/// 底部提示浮层。
///
/// 三路文字都汇到这里：
/// 1. `gui.rs` 的 `say(...)`（「✔ 已保存」「✘ 保存设置失败：…」）；
/// 2. 后台命令的结果（「写入生效 已还原」）；
/// 3. `d.status` —— 后台读设备**失败**的原因。
///
/// 第 3 路以前根本没有出口：`Shared::status` 里的 `Error(String)` 那个
/// payload 谁也没读，表现为「读不出设备」和「设备休眠」在界面上长得一样，
/// 用户不知道是该动鼠标还是该排查线缆。判据是 `d.status` 非空。
///
/// 优先级：**瞬时提示（`ui.status`）压过常驻错误**。用户刚点了「应用」，
/// 他此刻要看的是这次点击的结果，不是上一轮读盘失败的那句话。
fn draw_toast(p: &mut Painter, ui: &mut Ui, d: &Data, f: &Frame) {
    let transient = ui.status.trim();
    let s = if !transient.is_empty() {
        transient
    } else {
        d.status.trim()
    };
    if s.is_empty() {
        return;
    }
    let t = p.t;
    let scroll = ui.scroll_y(ui.page);
    let view_h = f.content.h;

    // 宽度跟着文字走，最宽不超过内容区。
    let (tw, _) = p.measure(s, font::HINT, false);
    let w = (tw + 40.0).min(f.width.max(160.0));
    let h = 36.0;
    // 视口底部往上 18px。用 `scroll + …` 把它换算到内容坐标 ——
    // canvas 给整页施加了 `-scroll` 的平移，加回来就钉住了。
    let r = R::new(
        f.content.x + (f.width - w) / 2.0,
        scroll + view_h - h - 18.0,
        w,
        h,
    );

    // 深色胶囊比浅色卡片显眼，又不会像红色警告那样把正常提示也说成错。
    p.round(r, 8.0, with_alpha(t.text, 0xF2));
    p.text(
        s,
        R::new(r.x + 16.0, r.y, r.w - 32.0, r.h),
        font::HINT,
        false,
        t.bg,
        Align::Center,
    );
    // 吃掉点击：否则浮层底下的按钮会被误触（`ui.hot` 登记后
    // `hit()` 倒序取最后一个，这里登记得晚所以优先）。
    ui.hot(r, Act::None);
}

// ───────────────────────── 下拉弹层 ─────────────────────────

/// 某个下拉框的候选项（文字, 值）。
fn options(drop: Drop, d: &Data) -> Vec<(String, usize)> {
    // 候选文字目前不依赖 `d`，保留参数是为了将来加「当前值」标记时不用改所有调用点。
    let _ = d;
    raw_options(drop)
}

/// 下拉框第 `idx` 项对应的**语义值**。
///
/// 语义值不等于下标：灯光模式的表是 `[0, 1, 2, 4, 6, 0xF0]`（第 3 项的值是 4），
/// 刷新间隔的表是 `[15, 30, 60, 120, 300]` 秒。谁要按下标去推语义值，
/// 就一定会在「灯光选第 3 项」或「刷新间隔选第 4 项」上错位。
///
/// 越界返 `None`。
pub fn option_value(drop: Drop, idx: usize) -> Option<usize> {
    raw_options(drop).get(idx).map(|(_, v)| *v)
}

/// 候选项本身。
///
/// 抽成独立函数是因为 [`option_value`] 也要用同一张表 —— 表一旦分成两份，
/// 改了一处忘了另一处，界面显示的和实际写进设备的值就会错位。
fn raw_options(drop: Drop) -> Vec<(String, usize)> {
    // 每个分支写全 `Vec<(String, usize)>`：不加这个标注，
    // arms 各自是不同的 iterator 类型，`collect()` 推不出目标类型。
    let items: Vec<(String, usize)> = match drop {
        Drop::Sleep => (0u8..=4)
            .map(|c| (crate::proto::sleep_mode_name(c), c as usize))
            .chain(std::iter::once((
                crate::proto::sleep_mode_name(0xFF),
                0xFFusize,
            )))
            .collect(),
        // 用 `u16` 不是 `u8`：1000 > 255，写 `u8` 编译期就报 literal out of range。
        // 这里的值只是显示用的 Hz 数，和 `poll_code`（0..3）是两回事。
        Drop::Poll => [125u16, 250, 500, 1000]
            .iter()
            .enumerate()
            .map(|(i, hz)| (format!("{hz} Hz"), i))
            .collect(),
        // value 必须是**协议里的 1 基档位**，不是 0 基下标。
        //
        // 原来写的是 `(0..4).map(|i| (..., i))`，于是：
        //   点「档位 1」→ 传 stage=0 → `set_stage` 报「档位必须是 1-4」
        //   点「档位 2」→ 传 stage=1 → 设备真的切到 **档位 1**
        //   点「档位 3」→ 传 stage=2 → 设备真的切到 **档位 2**
        //   点「档位 4」→ 传 stage=3 → 设备真的切到 **档位 3**
        // 用户原话「切换 dpi 没有反应，写的是切换了实际没有切换档位」——
        // 真机点「档位 3」时设备正好当前是 2，切到 2 等于没变，
        // 界面高亮也跟着留在 2，看起来完全没反应。
        Drop::Stage => (0..4)
            .map(|i| (format!("档位 {}", i + 1), i + 1))
            .collect(),
        Drop::Debounce => (0u8..=7)
            .map(|c| (crate::proto::debounce_name(c), c as usize))
            .collect(),
        Drop::Lod => (0u8..=1)
            .map(|c| (crate::proto::lod_name(c), c as usize))
            .collect(),
        // **只列设备认的模式**：Rainbow(3) 与 Comet(5) 写进去会被静默回落到
        // ColorCycle，1.1.0 的 CLI 干脆不提供这两个值。列出来只会让用户以为能选。
        Drop::LightMode => [0x00u8, 0x01, 0x02, 0x04, 0x06, 0xF0]
            .iter()
            .map(|&c| (crate::proto::light_mode_name(c), c as usize))
            .collect(),
        Drop::Refresh => [15u64, 30, 60, 120, 300]
            .iter()
            .map(|s| (format!("{s} 秒"), *s as usize))
            .collect(),
        Drop::CacheTtl => [0u64, 15, 30, 60, 300]
            .iter()
            .map(|s| {
                if *s == 0 {
                    ("每次都重读".to_string(), 0)
                } else {
                    (format!("{s} 秒"), *s as usize)
                }
            })
            .collect(),
        // value 是**目标码本身**（如 `0x01F0`），不是下标 —— 这样
        // [`option_value`] 的返回值可以直接写进设备，中间不用再查一次表。
        // 8 个槽位共用同一张候选表，让 `Drop` 保持无参数
        // （`ButtonTarget(slot)` 只区分槽位）。
        Drop::ButtonTarget(_) => BUTTON_TARGETS
            .iter()
            .map(|&c| (button_target_name(c), c as usize))
            .collect(),
    };
    items
}

/// 某个下拉框当前该显示什么文字。
fn drop_current(drop: Drop, d: &Data) -> String {
    let st = d.state;
    match drop {
        Drop::Sleep => st
            .map(|s| crate::proto::sleep_mode_name(s.sleep_mode))
            .unwrap_or_else(|| "—".to_string()),
        Drop::Poll => st
            .map(|s| crate::proto::poll_name(s.poll_code))
            .unwrap_or_else(|| "—".to_string()),
        Drop::Stage => st
            .map(|s| format!("档位 {}", s.active_stage.max(1)))
            .unwrap_or_else(|| "—".to_string()),
        Drop::Debounce => st
            .map(|s| crate::proto::debounce_name(s.debounce_code))
            .unwrap_or_else(|| "—".to_string()),
        Drop::Lod => st
            .map(|s| crate::proto::lod_name(s.lod))
            .unwrap_or_else(|| "—".to_string()),
        Drop::LightMode => st
            .map(|s| crate::proto::light_mode_name(s.light_mode))
            .unwrap_or_else(|| "—".to_string()),
        Drop::Refresh => format!("{} 秒", d.cfg.refresh_interval_sec),
        Drop::CacheTtl => {
            if d.cfg.cache_ttl_sec == 0 {
                "每次都重读".to_string()
            } else {
                format!("{} 秒", d.cfg.cache_ttl_sec)
            }
        }
        Drop::ButtonTarget(slot) => d
            .state
            .map(|s| button_target_name(s.buttons[(slot as usize) & 7]))
            .unwrap_or_else(|| "—".to_string()),
    }
}

/// 当前选中的下标。
///
/// 一律「拿语义值反查下标」，不按下标正查 —— 后者要求「表格顺序恰好等于设备
/// 编码顺序」，而灯光模式的表是 `[0, 1, 2, 4, 6, 0xF0]`（中间缺 3 和 5），
/// 一旦按下标正查，选 Rainbow 会显示成 React。
fn drop_selected(drop: Drop, d: &Data) -> usize {
    let opts = options(drop, d);
    let st = d.state;
    let cur: usize = match drop {
        Drop::Sleep => st.map(|s| s.sleep_mode as usize).unwrap_or(0),
        Drop::Poll => st.map(|s| s.poll_code as usize).unwrap_or(0),
        // `active_stage` 是 1 基的（0 表示设备没报），候选值也是 1..=4
        Drop::Stage => st.map(|s| s.active_stage.max(1) as usize).unwrap_or(1),
        Drop::Debounce => st.map(|s| s.debounce_code as usize).unwrap_or(0),
        Drop::Lod => st.map(|s| s.lod as usize).unwrap_or(0),
        Drop::LightMode => st.map(|s| s.light_mode as usize).unwrap_or(0),
        Drop::Refresh => d.cfg.refresh_interval_sec as usize,
        Drop::CacheTtl => d.cfg.cache_ttl_sec as usize,
        Drop::ButtonTarget(slot) => st
            .map(|s| s.buttons[(slot as usize) & 7] as usize)
            .unwrap_or(0),
    };
    opts.iter()
        .position(|(_, v)| *v == cur)
        // 反查不到就落到第一项。宁可让用户重选，也不伪造一个「看起来对」的名字 ——
        // 比如备份里还原进来一个未知按键码，显示成「左键点击」就是骗人。
        .unwrap_or(0)
}

/// 画下拉框本体，并登记「打开弹层」。点击区域是整个矩形。
///
/// 必须在**绘制该行时**调用：它要拿到那一行的矩形，而弹层本身的矩形
/// 由 [`draw_drop`] 在最后登记（那时代码已经知道弹层该开在哪儿）。
fn drop_button(p: &mut Painter, ui: &mut Ui, r: R, drop: Drop, d: &Data) {
    let open = ui.open_drop() == Some(drop);
    select_box(p, r, &drop_current(drop, d), open);
    ui.hot(r, Act::OpenDrop(drop));
    // 记下这个下拉框的位置，弹层要贴着它开
    if open {
        ui.set_drop_rect(R::new(r.x, r.bottom() + 4.0, r.w.max(160.0), 0.0));
    }
}

/// 画已经打开的弹层。
///
/// 弹层用**内容坐标**登记（和页面同一套），因为它是跟着内容滚动的。
fn draw_drop(p: &mut Painter, ui: &mut Ui, d: &Data, drop: Drop) {
    let Some(anchor) = ui.drop_rect() else {
        return;
    };
    let opts = options(drop, d);
    let item_h = 30.0;
    let h = opts.len() as f32 * item_h + 8.0;
    let r = R::new(anchor.x, anchor.y, anchor.w, h);

    // 阴影用两层淡色框近似（GDI+ 没有 box-shadow）
    p.round(
        R::new(r.x + 1.0, r.y + 2.0, r.w, r.h),
        layout::R_CARD,
        with_alpha(0x0000_0000, 0x28),
    );
    p.card(r, layout::R_CARD, p.t.card, p.t.border_strong);

    let sel = drop_selected(drop, d);
    for (i, (label, _)) in opts.iter().enumerate() {
        let ir = R::new(r.x + 4.0, r.y + 4.0 + i as f32 * item_h, r.w - 8.0, item_h);
        let act = Act::PickDrop(drop, i);
        let on = i == sel;
        if on {
            p.round(ir, 3.0, p.t.accent_soft);
        } else if ui.hover == Some(act) {
            p.round(ir, 3.0, with_alpha(p.t.text, 0x0C));
        }
        p.text(
            label,
            R::new(ir.x + 8.0, ir.y, ir.w - 16.0, ir.h),
            12.5,
            on,
            if on { p.t.accent } else { p.t.text },
            Align::Left,
        );
        // 选中项右侧打勾
        if on {
            let cx = ir.right() - 16.0;
            let cy = ir.cy();
            p.polyline(
                &[
                    (cx - 4.0, cy),
                    (cx - 1.0, cy + 3.0),
                    (cx + 5.0, cy - 4.0),
                ],
                p.t.accent,
                1.6,
            );
        }
        ui.hot(ir, act);
    }
    // 覆盖 `drop_button` 里登记的那个 0 高度占位矩形：命中测试要用真实高度。
    ui.set_drop_rect(r);
}

// ───────────────────────── 电量页 ─────────────────────────

fn battery(p: &mut Painter, ui: &mut Ui, d: &Data, f: &Frame, c: &mut Cursor) {
    let t = p.t;
    c.h1(p, "电量");

    let st = d.state;
    let level = st.map(|s| s.power).unwrap_or(0);
    let charging = st.map(|s| s.charging != 0).unwrap_or(false);

    // ---- hero：电量环 + KPI ----
    //
    // 原型里 hero 是**两张并排的卡片**：左边 `.hero-main`（环 + KPI），
    // 右边 `.hero-side`（设备信息，固定 246px）。不是一张大卡片里塞三块 ——
    // 塞在一起时 KPI 的第二列会压到「序列号 / 接收器固件」上。
    let side_by_side = f.bp.hero_side_by_side();
    let show_info = side_by_side && c.w > 760.0;
    let ring_d = f.bp.ring_diameter();
    let hero_h = if side_by_side { ring_d + 44.0 } else { ring_d + 130.0 };
    let info_w = 246.0;
    let gap = 18.0;
    let main_w = if show_info {
        c.w - info_w - gap
    } else {
        c.w
    };
    let hero = c.card_w(p, main_w, hero_h);

    // 左卡片里环之后剩下的宽度
    let kpi_x = if side_by_side {
        hero.x + 24.0 + ring_d + 26.0
    } else {
        hero.x + 24.0
    };
    let right_w = (hero.right() - 24.0 - kpi_x).max(80.0);
    let kv_y = if side_by_side {
        hero.y + 30.0
    } else {
        hero.y + ring_d + 34.0
    };
    let _ = kv_y;

    // 电量环。
    //
    // 读不到设备时（鼠标休眠）**不能画 0% 加红环** —— 0 是「电量耗尽」的意思，
    // 用户会以为鼠标没电了。实测休眠时界面显示「0% + 电量偏低」红色徽章，
    // 而真实电量其实是 77%。读不到就画一条灰色空环 + 「—」。
    {
        let cx = hero.x + 24.0 + ring_d * 0.5;
        let cy = if side_by_side {
            hero.cy()
        } else {
            hero.y + 22.0 + ring_d * 0.5
        };
        let frac = if st.is_some() {
            level as f32 / 100.0
        } else {
            0.0
        };

        // 环色三态，规则见 [`ring_color`]（那边有单测锁优先级）。
        let ring_color = ring_color(t, st.is_some(), charging, level);

        if charging && st.is_some() {
            // 充电动效：一段固定长度的亮色弧沿整圈匀速跑动。
            //
            // 为什么不用「整圈呼吸」：呼吸动画在不满一圈的进度弧上会让人分不清
            // 「电量在涨」还是「环在自己闪」。跑动的彗尾更明确 —— 它只有一个方向，
            // 而且不影响静态读数（进度弧本身照常画，另叠一条尾巴）。
            p.arc_comet(
                cx,
                cy,
                ring_d * 0.5 - 5.0,
                7.0,
                frac,
                t.green,
                ring_color,
                t.track,
            );
        } else {
            p.ring(
                cx,
                cy,
                ring_d * 0.5 - 5.0,
                7.0,
                frac,
                ring_color,
                ring_color,
                t.track,
            );
        }
        let txt = if st.is_some() {
            format!("{level}")
        } else {
            "—".to_string()
        };
        p.text(
            &txt,
            R::new(cx - 50.0, cy - 22.0, 100.0, 40.0),
            font::NUM_RING,
            true,
            t.text,
            Align::Center,
        );
        // 「—」时下面不能再写「%」，否则是「— %」，看着像单位缺失。
        if st.is_some() {
            p.text(
                "%",
                R::new(cx - 50.0, cy + 16.0, 100.0, 14.0),
                font::TINY,
                false,
                t.text3,
                Align::Center,
            );
        }
    }

    // KPI：电压 / 回报率 / 当前档位 / 续航
    {
        let two_cols = f.bp.kv_two_cols() && right_w > 180.0;
        let items: [(&str, String); 4] = if let Some(s) = st {
            [
                ("电压", format!("{} mV", s.voltage_mv)),
                ("回报率", crate::proto::poll_name(s.poll_code)),
                (
                    "当前档位",
                    format!("{} · {} DPI", s.active_stage.max(1), s.dpi(cur_stage(s))),
                ),
                (
                    "续航估算",
                    if d.estimate.known {
                        format!("{:+.2} %/h", d.estimate.percent_per_hour)
                    } else {
                        "样本不足".to_string()
                    },
                ),
            ]
        } else {
            [
                ("电压", "—".into()),
                ("回报率", "—".into()),
                ("当前档位", "—".into()),
                ("续航估算", "—".into()),
            ]
        };
        let col_w = if two_cols {
            (right_w - 26.0) / 2.0
        } else {
            right_w
        };
        for (i, (k, v)) in items.iter().enumerate() {
            let (col, row) = if two_cols {
                (i % 2, i / 2)
            } else {
                (0, i)
            };
            let x = kpi_x + col as f32 * (col_w + 26.0);
            let y = kv_y + row as f32 * 52.0;
            p.text(
                k,
                R::new(x, y, col_w, 14.0),
                font::LABEL,
                false,
                t.text3,
                Align::Left,
            );
            p.text(
                v,
                R::new(x, y + 15.0, col_w, 26.0),
                font::NUM_BIG,
                true,
                t.text,
                Align::Left,
            );
        }
    }

    // 设备信息：**另起一张卡片**放在 hero 右边。
    //
    // 原型里就是这样（`.hero` 是 flex 容器，`.hero-main` + `.hero-side` 两张卡），
    // 一开始我把它画在 hero 卡片内部右侧，结果 KPI 第二列的数字直接压上来
    // （实测「1000Hz」和序列号糊在一起），而且卡片右边还有 200px 空白。
    if show_info {
        let side = R::new(hero.right() + gap, hero.y, info_w, hero_h);
        p.card(side, layout::R_CARD, t.card, t.border);
        let sx = side.x + 20.0;
        let inner_w = info_w - 40.0;
        let sy = side.y + 18.0;
        p.text(
            "设备信息",
            R::new(sx, sy, inner_w, 16.0),
            font::LABEL,
            true,
            t.text3,
            Align::Left,
        );
        let mut ly = sy + 24.0;
        let ident = d.ident;
        let lines: [(&str, String); 3] = [
            (
                "序列号",
                ident.map(|i| i.mouse_sn.clone()).unwrap_or_default(),
            ),
            (
                "接收器固件",
                ident
                    .map(|i| i.dongle_firmware.clone())
                    .unwrap_or_default(),
            ),
            (
                "链路",
                ident
                    .map(|i| i.model_name.clone())
                    .filter(|m| !m.is_empty())
                    .map(|m| m)
                    .unwrap_or_else(|| "2.4G".to_string()),
            ),
        ];
        for (k, v) in lines.iter() {
            if v.is_empty() {
                continue;
            }
            // 标签区 78px：「接收器固件」五个汉字在 12px 字号下实测约 60px，
            // 给 62px 时被 `Trimming` 省略成「接收器…」（真机截图确认过）。
            // 78 留出余量，值仍然右对齐占满剩余宽度。
            p.text(k, R::new(sx, ly, 78.0, 16.0), 12.0, false, t.text3, Align::Left);
            p.text(
                v,
                R::new(sx + 84.0, ly, inner_w - 84.0, 16.0),
                12.0,
                false,
                t.text2,
                Align::Right,
            );
            ly += 20.0;
        }
        // 状态胶囊。
        //
        // 读不到设备（休眠）时显示「休眠中」灰胶囊 —— **不能**因为 `level`
        // 默认成 0 就落到「电量偏低」分支，那是在没有任何数据的情况下报警。
        let mut bx = sx;
        let by = ly + 6.0;
        let pill_h = 21.0;
        let (label, fg, bg) = if st.is_none() {
            ("休眠中", t.text3, t.bg_sunken)
        } else if charging {
            ("充电中", t.warn, t.warn_soft)
        } else if level <= 15 {
            ("电量偏低", t.red, t.red_soft)
        } else {
            ("使用中", t.green, t.green_soft)
        };
        let w = label.chars().count() as f32 * 11.0 + 16.0;
        badge(p, R::new(bx, by, w, pill_h), label, fg, bg);
        bx += w + 6.0;
        if let Some(s) = st {
            let l = format!("{} 档休眠", crate::proto::sleep_mode_name(s.sleep_mode));
            let w2 = l.chars().count() as f32 * 11.0 + 16.0;
            if bx + w2 <= side.right() - 20.0 {
                badge(p, R::new(bx, by, w2, pill_h), &l, t.text3, t.bg_sunken);
            }
        }
    }

    c.y = hero.bottom();

    // ---- 电量历史 ----
    c.h2(p, "电量历史", false);
    // 卡片高度按 `draw_chart` 的实际内容算：16（上边距）+ 18（标题行）+ 8 +
    // 132（绘图区）+ 20（X 轴时间）+ 12（下边距）= 206。
    // 原来写 `168 + 56 + 28 = 252` 是照抄原型的 SVG viewBox，
    // 底下会空出 46px 一条白边。
    let chart = c.card(p, 206.0);
    draw_chart(p, chart, d);
    // 卡片不会自己推进游标（它的返回值才是位置），必须手动前移 ——
    // 漏了这一步，「电源管理」的标题会画在图表卡片上面。
    c.y = chart.bottom();

    // ---- 电源管理 ----
    c.h2(p, "电源管理", false);
    let rows: [(&str, Option<&str>, f32); 3] = [
        ("自动休眠", Some("鼠标静止多久后进入休眠以省电"), 150.0),
        ("低电量阈值", Some("低于该电量时鼠标上的指示灯会闪"), 150.0),
        ("充满效果", Some("充满电时播放一次灯效"), 60.0),
    ];
    let total_h: f32 = rows.iter().map(|(_, _, _)| layout::ROW_H).sum();
    let card = c.card(p, total_h);
    let mut ry = card.y;
    let mut i = 0;

    // 自动休眠
    {
        let area = R::new(card.x + 18.0, ry, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(p, area, rows[i].0, rows[i].1, 150.0);
        drop_button(p, ui, R::new(ctl.x, ctl.cy() - 15.0, 150.0, 30.0), Drop::Sleep, d);
        if i + 1 < rows.len() {
            p.hline(card.x, card.right(), area.bottom(), t.border);
        }
        ry += layout::ROW_H;
        i += 1;
    }

    // 低电量阈值：输入框 + % + 应用
    {
        let area = R::new(card.x + 18.0, ry, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(p, area, rows[i].0, rows[i].1, 150.0);
        let edit = Edit::LowWarn;
        let cur = st.map(|s| s.low_warn).unwrap_or(20);
        let txt = ui
            .edit_text(edit)
            .map(|s| s.to_string())
            .unwrap_or_else(|| cur.to_string());
        let box_r = R::new(ctl.x, ctl.cy() - 15.0, 62.0, 30.0);
        num_box(p, box_r, &txt, ui.focus == Some(edit));
        ui.hot(box_r, Act::Focus(edit));
        p.text(
            "%",
            R::new(box_r.right() + 4.0, ctl.y, 14.0, ctl.h),
            12.0,
            false,
            t.text2,
            Align::Left,
        );
        let btn = R::new(ctl.right() - 56.0, ctl.cy() - 15.0, 56.0, 30.0);
        button(p, ui, btn, "应用", false, Act::ApplyEdit(edit));
        p.hline(card.x, card.right(), area.bottom(), t.border);
        ry += layout::ROW_H;
        i += 1;
    }

    // 充满效果
    {
        let area = R::new(card.x + 18.0, ry, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(p, area, rows[i].0, rows[i].1, 40.0);
        let on = st.map(|s| s.full_effect != 0).unwrap_or(true);
        let sw = R::new(ctl.x + 4.0, ctl.cy() - 10.0, 40.0, 20.0);
        p.switch(sw, on, st.is_some());
        ui.hot(sw, Act::Toggle(Pref::FullEffect));
    }
    c.y = card.bottom();

    // ---- 续航估算文字 ----
    if !d.estimate.text.is_empty() {
        c.y += 14.0;
        p.text(
            &d.estimate.text,
            R::new(c.x, c.y, c.w, 18.0),
            font::HINT,
            false,
            t.text3,
            Align::Left,
        );
        c.y += 22.0;
    }
    let _ = layout::GAP;
}

/// 当前档位下标（`active_stage` 是 1-based，可能为 0）。
///
/// `active_stage == 0` 表示还没读到（不是「第 0 档」），
/// 直接 `- 1` 会变成 255 或 panic，所以先 `saturating_sub` 再夹上限。
/// 用完 `as usize` 再 `.min()`：`DPI_STAGES` 是 `usize`，`active_stage` 是 `u8`，
/// 两个类型不能直接比。
fn cur_stage(s: &crate::proto::State) -> usize {
    let idx = s.active_stage.saturating_sub(1) as usize;
    idx.min(crate::proto::DPI_STAGES - 1)
}

/// 画电量曲线（含标题行、网格、面积、折线、数据点）。
fn draw_chart(p: &mut Painter, card: R, d: &Data) {
    let t = p.t;
    let head_y = card.y + 16.0;
    p.text(
        "近 48 小时",
        R::new(card.x + 18.0, head_y, 90.0, 18.0),
        13.0,
        true,
        t.text,
        Align::Left,
    );
    // 采样点数 / 斜率靠右
    let slope = if d.estimate.known {
        format!("{:+.2} %/h", d.estimate.percent_per_hour)
    } else {
        String::new()
    };
    if !slope.is_empty() {
        p.text(
            &slope,
            R::new(card.right() - 100.0, head_y, 82.0, 18.0),
            12.0,
            true,
            if d.estimate.percent_per_hour < 0.0 {
                t.green
            } else {
                t.warn
            },
            Align::Right,
        );
    }
    let samples = format!("{} 个采样点", d.rows);
    p.text(
        &samples,
        R::new(card.right() - 200.0, head_y, 94.0, 18.0),
        11.5,
        false,
        t.text3,
        Align::Right,
    );

    // 绘图区：左边留 42px 给 Y 轴标签，底部留 20px 给 X 轴时间。
    // 高度定死 132，和上面 `c.card(p, 206.0)` 的算法对应。
    let plot = R::new(card.x + 42.0, head_y + 26.0, card.w - 42.0 - 18.0, 132.0);

    // 网格 + Y 轴刻度（92 / 88 / 84 / 80 是原型里那组，这里按实际数据取整）
    let (lo, hi) = match d.plot.filter(|p| p.enough()) {
        Some(pl) => {
            let mut mn = 100u8;
            let mut mx = 0u8;
            for pt in &pl.points {
                mn = mn.min(pt.level);
                mx = mx.max(pt.level);
            }
            // 上下各留一点余量，否则线贴着框边
            (mn.saturating_sub(2), (mx + 2).min(100))
        }
        None => (0, 100),
    };
    let span = (hi as i32 - lo as i32).max(1);
    for i in 0..5 {
        let frac = i as f32 / 4.0;
        let gy = plot.bottom() - frac * plot.h;
        p.hline(plot.x, plot.right(), gy, with_alpha(t.text, 0x10));
        let level = lo as i32 + (span as f32 * frac) as i32;
        p.text(
            &level.to_string(),
            R::new(card.x + 8.0, gy - 8.0, 28.0, 16.0),
            font::TINY,
            false,
            t.text3,
            Align::Right,
        );
    }

    match d.plot {
        None => {
            // 还没读完第一遍 —— 说「正在读取」，不要说「样本不足」。
            // 后者会让用户以为历史文件是空的（其实可能有 7 条）。
            p.text(
                crate::chart::LOADING_TEXT,
                R::new(plot.x, plot.cy() - 10.0, plot.w, 20.0),
                12.0,
                false,
                t.text3,
                Align::Center,
            );
        }
        Some(pl) if !pl.enough() => {
            p.text(
                crate::chart::NOT_ENOUGH_TEXT,
                R::new(plot.x, plot.cy() - 10.0, plot.w, 20.0),
                12.0,
                false,
                t.text3,
                Align::Center,
            );
        }
        Some(pl) => {
            // 点集直接取自 `chart::build`（它已经按数据时间跨度算好 x），
            // 这里只把 i32 转成 f32 并映射到当前 plot 矩形。
            // 不自己再算一遍时间→x：两份实现一定会漂移。
            let n = pl.points.len();
            let pts: Vec<(f32, f32)> = pl
                .points
                .iter()
                .map(|pt| {
                    // `build` 用的是它自己接收的 plot 矩形，宽度不一定和我们一样，
                    // 所以按比例搬运而不是直接照抄坐标。
                    let fx = if crate::chart::AREA_W > 0 {
                        (pt.x - crate::chart::AREA_X) as f32 / crate::chart::AREA_W as f32
                    } else {
                        0.0
                    };
                    let fy = (pt.level.saturating_sub(lo)) as f32 / span as f32;
                    (plot.x + fx.clamp(0.0, 1.0) * plot.w, plot.bottom() - fy * plot.h)
                })
                .collect();

            // 面积：折线点 + 右下角 + 左下角
            if pts.len() >= 2 {
                let mut area: Vec<(f32, f32)> = Vec::with_capacity(pts.len() + 2);
                area.push((plot.x, plot.bottom()));
                area.extend(pts.iter().copied());
                area.push((plot.right(), plot.bottom()));
                p.fill_polygon(&area, with_alpha(t.accent, 0x3D), with_alpha(t.accent, 0x05));
            }
            p.polyline(&pts, t.accent, 2.0);

            // 数据点：普通点是「卡片色填充 + 强调色描边」，最新点额外一圈淡光
            for (i, (x, y)) in pts.iter().enumerate() {
                if i + 1 == n {
                    p.dot(*x, *y, 9.0, with_alpha(t.accent, 0x2E));
                    p.dot(*x, *y, 4.6, t.accent);
                } else {
                    p.dot(*x, *y, 3.4, t.card);
                    p.dot_ring(*x, *y, 3.4, t.accent, 2.0);
                }
            }

            // X 轴：首尾时间。
            //
            // `format_unix_local` 返回 `MM-DD HH:MM`（11 个字符，12px 约 74px 宽），
            // 给 60px 会被省略成「10-21 1…」。给 84px 且不加右对齐留白才放得下。
            let first = crate::settings::format_unix_local(pl.first_ts).unwrap_or_default();
            let last = crate::settings::format_unix_local(pl.last_ts).unwrap_or_default();
            let ty = plot.bottom() + 6.0;
            p.text(
                &first,
                R::new(plot.x, ty, 84.0, 14.0),
                font::TINY,
                false,
                t.text3,
                Align::Left,
            );
            p.text(
                &last,
                R::new(plot.right() - 84.0, ty, 84.0, 14.0),
                font::TINY,
                false,
                t.text3,
                Align::Right,
            );
        }
    }
}

// ───────────────────────── 性能页 ─────────────────────────

fn perf(p: &mut Painter, ui: &mut Ui, d: &Data, f: &Frame, c: &mut Cursor) {
    let t = p.t;
    c.h1(p, "性能");
    let st = d.state;
    let cur = st.map(cur_stage).unwrap_or(0);

    // ---- DPI 档位 ----
    c.h2(p, "DPI 档位", true);
    let dpi_h = 4.0 * 46.0 + 40.0;
    let card = c.card(p, dpi_h);
    let mut y = card.y + 4.0;
    for i in 0..crate::proto::DPI_STAGES {
        let is_cur = i == cur && st.is_some();
        let row = R::new(card.x, y, card.w, 46.0);
        if is_cur {
            p.rect(row, t.accent_soft);
        }

        // 整行的「底板」命中区**必须先登记**。
        //
        // `Ui::hit()` 在内容区里是**倒序**取的（后登记的赢），所以这条
        // 宽区域一旦在输入框之后登记，就会把输入框、颜色框、应用按钮
        // 统统盖掉 —— 表现是「点 DPI 数字框没反应、点颜色框没反应、
        // 点应用没反应」，而每次点击都触发了一个 `Act::Nav(Page::Perf)`
        // （导航到当前页，等价于什么都不做）。真机点一遍才能发现。
        //
        // 动作用 `Act::None` 而不是「切到此档」：要切档位有行首标签区
        // （下面那段），把整行都做成切档会让「没点准输入框」变成
        // 意外改设备。
        ui.hot(row, Act::None);

        // 档位标签
        p.text(
            &format!("档位 {}", i + 1),
            R::new(card.x + 18.0, row.y, 52.0, row.h),
            11.5,
            is_cur,
            if is_cur { t.accent } else { t.text3 },
            Align::Left,
        );
        // 数值输入框
        //
        // 读不到设备时显示「—」而不是编一个 800 出来：`State::default()` 的
        // `enc` 也解得出 800 DPI、`rgb` 也解得出 `#000000`，两者都是**合法值**，
        // 画出来和真读数一模一样。用户在休眠时看到「档位 1 = 800 DPI」会以为
        // 那是鼠标当前的设置，改完点「应用」才发现改的是他不知道的东西。
        let edit = Edit::Dpi(i as u8);
        let val = st.map(|s| s.dpi(i));
        let txt = ui
            .edit_text(edit)
            .map(|s| s.to_string())
            .unwrap_or_else(|| val.map(|v| v.to_string()).unwrap_or_else(|| "—".into()));
        let box_r = R::new(card.x + 74.0, row.cy() - 15.0, 76.0, 30.0);
        num_box(p, box_r, &txt, ui.focus == Some(edit));
        ui.hot(box_r, Act::Focus(edit));
        p.text(
            "DPI",
            R::new(box_r.right() + 8.0, row.y, 30.0, row.h),
            12.0,
            false,
            t.text3,
            Align::Left,
        );
        // 颜色小方块 + 颜色框
        let ce = Edit::DpiColor(i as u8);
        let rgb = st.map(|s| s.rgb[i]);
        let sw = R::new(box_r.right() + 46.0, row.cy() - 15.0, 30.0, 30.0);
        swatch(p, sw, rgb);
        ui.hot(sw, Act::Focus(ce));
        let ctxt = ui.edit_text(ce).map(|s| s.to_string()).unwrap_or_else(|| {
            rgb.map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
                .unwrap_or_else(|| "—".into())
        });
        let cbox = R::new(sw.right() + 8.0, row.cy() - 15.0, 84.0, 30.0);
        num_box(p, cbox, &ctxt, ui.focus == Some(ce));
        ui.hot(cbox, Act::Focus(ce));

        // 右侧：当前标记 / 应用按钮
        if is_cur {
            p.text(
                "● 当前使用",
                R::new(row.right() - 150.0, row.y, 76.0, row.h),
                11.0,
                true,
                t.accent,
                Align::Left,
            );
        }
        let btn = R::new(row.right() - 68.0, row.cy() - 15.0, 50.0, 30.0);
        button(p, ui, btn, "应用", false, Act::ApplyEdit(edit));

        // 「设为当前档」用行首标签区。
        //
        // 这里**不能**再登记一条覆盖整行的命中区：`hit()` 倒序取，
        // 后登记的会把上面的输入框 / 颜色框 / 应用按钮全盖掉。
        // 整行底板已经在循环开头登记成 `Act::None` 了。
        ui.hot(
            R::new(card.x + 18.0, row.y, 52.0, row.h),
            Act::PickDrop(Drop::Stage, i),
        );

        y += 46.0;
    }
    // 说明
    p.text_wrap(
        "范围 100–42000 DPI，步进 50。颜色是该档位在鼠标上的指示灯颜色。",
        R::new(card.x + 18.0, y + 6.0, card.w - 36.0, 32.0),
        11.5,
        t.text3,
    );
    c.y = card.bottom();

    // ---- 回报率 ----
    c.h2(p, "回报率", false);
    let card = c.card(p, layout::ROW_H * 2.0);
    {
        let area = R::new(card.x + 18.0, card.y, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(
            p,
            area,
            "轮询频率",
            Some("越高越跟手，也越耗电"),
            330.0,
        );
        let sel = st.map(|s| s.poll_code as usize).unwrap_or(3).min(3);
        // 分段控件让出 30px 给右边的「Hz」。
        //
        // 原先把 330px 全给分段、再把「Hz」画在 `ctl.right() - 2.0` ——
        // 那是**压在最后一段上面**（截图里「1000」蓝底右侧压着一截「Hz」），
        // 同时视口右边界把最后几像素裁掉了，看起来像控件溢出了卡片。
        let seg_w = 300.0;
        segmented(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, seg_w, 30.0),
            &["125", "250", "500", "1000"],
            sel,
            |i| Act::PickDrop(Drop::Poll, i),
        );
        p.text(
            "Hz",
            R::new(ctl.x + seg_w + 8.0, ctl.y, 22.0, ctl.h),
            11.0,
            false,
            t.text3,
            Align::Left,
        );
        p.hline(card.x, card.right(), area.bottom(), t.border);
    }
    {
        let area = R::new(card.x + 18.0, card.y + layout::ROW_H, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(p, area, "当前档位", Some("同时决定鼠标上的档位指示灯"), 220.0);
        segmented(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, 220.0, 30.0),
            &["1", "2", "3", "4"],
            cur,
            |i| Act::PickDrop(Drop::Stage, i),
        );
    }
    c.y = card.bottom();

    // ---- 去抖 ----
    c.h2(p, "去抖", false);
    let card = c.card(p, layout::ROW_H);
    {
        let area = R::new(card.x + 18.0, card.y, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(
            p,
            area,
            "按键去抖时间",
            Some("双击被识别成单击时调大"),
            150.0,
        );
        drop_button(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, 150.0, 30.0),
            Drop::Debounce,
            d,
        );
    }
    c.y = card.bottom();

    // ---- 角度 ----
    c.h2(p, "角度", false);
    let adv_open = ui.adv[0];
    let base_h = layout::ROW_H + if adv_open { 46.0 + 46.0 + 46.0 } else { 46.0 };
    let card = c.card(p, base_h);
    {
        let area = R::new(card.x + 18.0, card.y, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(
            p,
            area,
            "角度吸附",
            Some("斜向移动时自动贴合水平/垂直"),
            40.0,
        );
        let on = st.map(|s| s.angle_snap != 0).unwrap_or(false);
        let sw = R::new(ctl.x + 4.0, ctl.cy() - 10.0, 40.0, 20.0);
        p.switch(sw, on, st.is_some());
        ui.hot(sw, Act::Toggle(Pref::AngleSnap));
        p.hline(card.x, card.right(), area.bottom(), t.border);
    }
    // 「高级」折叠头
    {
        let hy = card.y + layout::ROW_H;
        let head = R::new(card.x + 18.0, hy, card.w - 36.0, 46.0);
        let act = Act::ToggleAdv(0);
        if ui.hover == Some(act) {
            p.round(head, layout::R_CTL, with_alpha(t.text, 0x08));
        }
        // 三角：展开时朝下
        let cx = head.x + 6.0;
        let cy = head.cy();
        if adv_open {
            p.polyline(
                &[(cx - 3.5, cy - 1.5), (cx, cy + 2.0), (cx + 3.5, cy - 1.5)],
                t.text3,
                1.3,
            );
        } else {
            p.polyline(
                &[(cx - 1.5, cy - 3.5), (cx + 2.0, cy), (cx - 1.5, cy + 3.5)],
                t.text3,
                1.3,
            );
        }
        p.text(
            "高级",
            R::new(head.x + 18.0, head.y, head.w - 30.0, head.h),
            13.0,
            false,
            t.text2,
            Align::Left,
        );
        ui.hot(head, act);
        p.hline(card.x, card.right(), head.bottom(), t.border);
    }
    if adv_open {
        // 角度微调滑块
        let sy = card.y + layout::ROW_H + 46.0;
        let area = R::new(card.x + 18.0, sy, card.w - 36.0, 46.0);
        let dev_val = st.map(|s| s.angle_tuning).unwrap_or(0);
        // 拖拽中优先用瞬时值：写设备要几十毫秒，只显示设备值的话滑块不跟手。
        let frac = ui
            .live_frac(Slide::Tune)
            .unwrap_or(((dev_val as f32) + 20.0) / 40.0);
        // 数字也跟着走，否则「滑块动了但数字不动」比不跟手更让人困惑。
        let val = (frac * 40.0 - 20.0).round() as i16;
        let ctl = row_label(p, area, "角度微调", None, 260.0);
        let track = R::new(ctl.x, ctl.cy() - 9.0, 190.0, 18.0);
        let knob_x = p.slider(track, frac, st.is_some());
        ui.hot(track, Act::SetSlide(Slide::Tune, frac));
        let _ = knob_x;
        p.text(
            &format!("{val:+}"),
            R::new(ctl.right() - 40.0, ctl.y, 40.0, ctl.h),
            12.0,
            false,
            t.text2,
            Align::Right,
        );
        p.hline(card.x, card.right(), area.bottom(), t.border);

        // 加减速：设备不支持，明确说明而不是画一个假控件
        let ny = sy + 46.0;
        let area = R::new(card.x + 18.0, ny, card.w - 36.0, 46.0);
        row_label(p, area, "加减速曲线", None, 120.0);
        p.text(
            "本型号不支持",
            R::new(area.right() - 110.0, area.y, 110.0, area.h),
            12.0,
            false,
            t.text3,
            Align::Right,
        );
    }
    c.y = card.bottom();

    // ---- 抬起距离 ----
    c.h2(p, "抬起距离", false);
    let card = c.card(p, layout::ROW_H + 34.0);
    {
        let area = R::new(card.x + 18.0, card.y, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(p, area, "抬起距离（LOD）", Some("低省电，高更容易追踪"), 160.0);
        let sel = st.map(|s| s.lod as usize).unwrap_or(0).min(1);
        segmented(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, 160.0, 30.0),
            &["Low", "High"],
            sel,
            |i| Act::PickDrop(Drop::Lod, i),
        );
    }
    let nw = {
        let nr = R::new(card.x, card.y + layout::ROW_H, card.w, 34.0);
        note(
            p,
            R::new(nr.x + 18.0, nr.y, nr.w - 36.0, 20.0),
            "写 LOD 会重置鼠标的表面标定（厂商确认没有恢复命令）。本机标定已被重置过。",
            true,
        )
    };
    let _ = nw;
    c.y = card.bottom();

    let _ = f;
    let _ = t;
}

// ───────────────────────── 灯光页 ─────────────────────────

fn light(p: &mut Painter, ui: &mut Ui, d: &Data, _f: &Frame, c: &mut Cursor) {
    let t = p.t;
    c.h1(p, "灯光");
    let st = d.state;

    // ---- 模式 ----
    c.h2(p, "模式", true);
    let card = c.card(p, layout::ROW_H + 8.0);
    {
        let area = R::new(card.x + 18.0, card.y + 4.0, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(p, area, "灯效模式", None, 180.0);
        drop_button(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, 180.0, 30.0),
            Drop::LightMode,
            d,
        );
    }
    c.y = card.bottom();
    c.y += 10.0;
    p.text_wrap(
        "可选：静置 / 呼吸 / 循环 / 响应 / 跟随电量 / 关闭。Rainbow 与 Comet 本型号不认，设备会静默回落到循环，因此不列出。",
        R::new(c.x, c.y, c.w, 34.0),
        11.5,
        t.text3,
    );
    c.y += 38.0;

    // ---- 亮度与颜色 ----
    c.h2(p, "亮度与颜色", false);
    let card = c.card(p, layout::ROW_H * 2.0);
    {
        let area = R::new(card.x + 18.0, card.y, card.w - 36.0, layout::ROW_H);
        let dev_bright = st.map(|s| s.light_bright).unwrap_or(50);
        // 同一行里数字框和滑块必须显示**同一个值**。否则拖滑块时框里还是旧数字。
        // 正在编辑框时以草稿为准（用户手输的优先），否则看拖拽值，最后才落到设备值。
        let live = ui.live_frac(Slide::LightBright);
        let bright = match (live, ui.edit_text(Edit::LightBright)) {
            (_, Some(t)) => t.parse::<u8>().unwrap_or(dev_bright),
            (Some(f), None) => (f * 100.0).round() as u8,
            (None, None) => dev_bright,
        };
        let ctl = row_label(p, area, "亮度", None, 320.0);
        let edit = Edit::LightBright;
        let txt = ui
            .edit_text(edit)
            .map(|s| s.to_string())
            .unwrap_or_else(|| bright.to_string());
        let box_r = R::new(ctl.x, ctl.cy() - 15.0, 52.0, 30.0);
        num_box(p, box_r, &txt, ui.focus == Some(edit));
        ui.hot(box_r, Act::Focus(edit));
        p.text(
            "%",
            R::new(box_r.right() + 4.0, ctl.y, 14.0, ctl.h),
            12.0,
            false,
            t.text2,
            Align::Left,
        );
        let track = R::new(box_r.right() + 26.0, ctl.cy() - 9.0, 220.0, 18.0);
        let frac = live.unwrap_or(bright as f32 / 100.0);
        p.slider(track, frac, st.is_some());
        ui.hot(track, Act::SetSlide(Slide::LightBright, frac));
        p.hline(card.x, card.right(), area.bottom(), t.border);
    }
    {
        let area = R::new(card.x + 18.0, card.y + layout::ROW_H, card.w - 36.0, layout::ROW_H);
        let rgb = st.map(|s| s.light_rgb);
        let ctl = row_label(p, area, "颜色", None, 180.0);
        let sw = R::new(ctl.x, ctl.cy() - 15.0, 30.0, 30.0);
        swatch(p, sw, rgb);
        let edit = Edit::LightColor;
        ui.hot(sw, Act::Focus(edit));
        let txt = ui.edit_text(edit).map(|s| s.to_string()).unwrap_or_else(|| {
            rgb.map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
                .unwrap_or_else(|| "—".into())
        });
        let box_r = R::new(sw.right() + 8.0, ctl.cy() - 15.0, 92.0, 30.0);
        num_box(p, box_r, &txt, ui.focus == Some(edit));
        ui.hot(box_r, Act::Focus(edit));
        let btn = R::new(box_r.right() + 8.0, ctl.cy() - 15.0, 50.0, 30.0);
        button(p, ui, btn, "应用", false, Act::ApplyEdit(edit));
    }
    c.y = card.bottom();

    // ---- 高级 ----
    c.h2(p, "高级", false);
    let open = ui.adv[1];
    let h = 46.0 + if open { layout::ROW_H * 2.0 + 46.0 } else { 0.0 };
    let card = c.card(p, h);
    {
        let head = R::new(card.x + 18.0, card.y, card.w - 36.0, 46.0);
        let act = Act::ToggleAdv(1);
        if ui.hover == Some(act) {
            p.round(head, layout::R_CTL, with_alpha(t.text, 0x08));
        }
        let cx = head.x + 6.0;
        let cy = head.cy();
        if open {
            p.polyline(
                &[(cx - 3.5, cy - 1.5), (cx, cy + 2.0), (cx + 3.5, cy - 1.5)],
                t.text3,
                1.3,
            );
        } else {
            p.polyline(
                &[(cx - 1.5, cy - 3.5), (cx + 2.0, cy), (cx - 1.5, cy + 3.5)],
                t.text3,
                1.3,
            );
        }
        p.text(
            "高级",
            R::new(head.x + 18.0, head.y, head.w - 30.0, head.h),
            13.0,
            false,
            t.text2,
            Align::Left,
        );
        ui.hot(head, act);
    }
    if open {
        // 方向
        let dy = card.y + 46.0;
        let area = R::new(card.x + 18.0, dy, card.w - 36.0, layout::ROW_H);
        p.hline(card.x, card.right(), area.y, t.border);
        let ctl = row_label(p, area, "方向", None, 160.0);
        let sel = st.map(|s| s.light_dir as usize).unwrap_or(0).min(1);
        segmented(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, 160.0, 30.0),
            &["正向", "反向"],
            sel,
            |i| Act::PickDrop(Drop::LightMode, 100 + i),
        );

        // 随机颜色
        let ry = dy + layout::ROW_H;
        let area = R::new(card.x + 18.0, ry, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(p, area, "随机颜色", None, 40.0);
        let rnd = st.map(|s| s.light_random != 0).unwrap_or(false);
        let sw = R::new(ctl.x + 4.0, ctl.cy() - 10.0, 40.0, 20.0);
        p.switch(sw, rnd, st.is_some());
        ui.hot(sw, Act::Toggle(Pref::LightRandom));

        // 速度
        let sy = ry + layout::ROW_H;
        let area = R::new(card.x + 18.0, sy, card.w - 36.0, 46.0);
        let dev_spd = st.map(|s| s.light_speed).unwrap_or(50);
        let frac = ui
            .live_frac(Slide::LightSpeed)
            .unwrap_or(dev_spd as f32 / 100.0);
        let spd = (frac * 100.0).round() as u8;
        let ctl = row_label(p, area, "速度", None, 280.0);
        let track = R::new(ctl.x, ctl.cy() - 9.0, 220.0, 18.0);
        p.slider(track, frac, st.is_some());
        ui.hot(track, Act::SetSlide(Slide::LightSpeed, frac));
        p.text(
            &format!("{spd}"),
            R::new(ctl.right() - 34.0, ctl.y, 34.0, ctl.h),
            12.5,
            false,
            t.text2,
            Align::Right,
        );
    }
    c.y = card.bottom();
}

// ───────────────────────── 按键页 ─────────────────────────

/// 按键目标码 → 显示名。
///
/// `proto::BUTTON_SRC` 只有 8 个**源**键的码，没有目标码表（`cli.rs` 里也没有），
/// 所以这张表在这里定义。值来自抓包确认的目标码；不认识的码原样显示成十六进制，
/// 不猜 —— 猜错会让用户把键绑到别处去。
fn button_target_name(code: u16) -> String {
    match code {
        0x0000 => "无功能".to_string(),
        0x01F0 => "左键点击".to_string(),
        0x01F1 => "右键点击".to_string(),
        0x01F2 => "中键点击".to_string(),
        0x01E4 => "浏览器后退".to_string(),
        0x01E5 => "浏览器前进".to_string(),
        0x01E6 => "切换 DPI 档位".to_string(),
        0x01E8 => "滚轮上滚".to_string(),
        0x01E9 => "滚轮下滚".to_string(),
        other => format!("0x{other:04X}"),
    }
}

/// 按键页下拉框的候选目标码。
///
/// **只列抓包确认过的码**，外加 `0x0000`（无功能）。`button_target_name` 对未知码
/// 会原样显示成十六进制 —— 从备份还原进来的未知码因此仍能正确显示，
/// 但它不在候选表里，下拉打开时选不中任何一项（`unwrap_or(0)` 落到「无功能」）。
/// 这是有意的：宁可显示成「无功能」让用户重选，也不要伪造一个名字。
const BUTTON_TARGETS: [u16; 9] = [
    0x0000, 0x01F0, 0x01F1, 0x01F2, 0x01E4, 0x01E5, 0x01E6, 0x01E8, 0x01E9,
];

fn keys(p: &mut Painter, ui: &mut Ui, d: &Data, _f: &Frame, c: &mut Cursor) {
    let t = p.t;
    c.h1(p, "按键");

    c.h2(p, "按键映射", true);
    let rows = crate::proto::BUTTON_SLOTS;
    let card = c.card(p, rows as f32 * layout::ROW_H);
    for slot in 0..rows {
        let y = card.y + slot as f32 * layout::ROW_H;
        let area = R::new(card.x + 18.0, y, card.w - 36.0, layout::ROW_H);
        // 左：源键名 + 码
        let src = crate::proto::BUTTON_SRC[slot];
        let name = crate::proto::BUTTON_NAME[slot];
        p.text(
            name,
            R::new(area.x, area.y + 8.0, 200.0, 20.0),
            13.5,
            false,
            t.text,
            Align::Left,
        );
        p.text(
            &format!("{src:04X}"),
            R::new(area.x + 74.0, area.y + 10.0, 48.0, 16.0),
            font::TINY,
            false,
            t.text3,
            Align::Left,
        );
        // 右：当前绑定。
        //
        // 这里原来是 `select_box(...)` + `ui.hot(dst, Act::ResetButton(slot))` ——
        // 画一个长得像下拉的框，点下去却是「重置成无功能」。那条路把键绑成 0000
        // 以后就再也回不来了（鼠标上那个键彻底失效），所以改成真下拉。
        let dst = R::new(area.right() - 150.0, area.cy() - 15.0, 150.0, 30.0);
        drop_button(p, ui, dst, Drop::ButtonTarget(slot as u8), d);
        if slot + 1 < rows {
            p.hline(card.x, card.right(), area.bottom(), t.border);
        }
    }
    c.y = card.bottom();

    c.y += 12.0;
    let nw = note(
        p,
        R::new(c.x + 18.0, c.y, c.w - 36.0, 20.0),
        "把某个键绑成「无功能」（0000）以后，那个键在鼠标上就再也回不来了 —— 恢复必须重新写一遍完整映射。",
        true,
    );
    c.y += nw + 12.0;

    // 底部两个按钮
    let bw = 150.0;
    let br = R::new(c.x, c.y, bw, 32.0);
    button(p, ui, br, "从备份恢复全部按键", false, Act::Restore);
    let br2 = R::new(br.right() + 10.0, c.y, bw, 32.0);
    button(p, ui, br2, "导出当前配置", false, Act::Backup);
    c.y += 32.0;
}

// ───────────────────────── 设置页 ─────────────────────────

fn settings(p: &mut Painter, ui: &mut Ui, d: &Data, _f: &Frame, c: &mut Cursor) {
    let t = p.t;
    c.h1(p, "设置");

    // ---- 程序 ----
    c.h2(p, "程序", true);
    let card = c.card(p, layout::ROW_H * 4.0);
    let mut y = card.y;

    // 托盘
    {
        let area = R::new(card.x + 18.0, y, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(p, area, "显示托盘图标", Some("关闭后只能用窗口操作"), 40.0);
        let sw = R::new(ctl.x + 4.0, ctl.cy() - 10.0, 40.0, 20.0);
        p.switch(sw, d.cfg.tray_enabled, true);
        ui.hot(sw, Act::Toggle(Pref::TrayEnabled));
        p.hline(card.x, card.right(), area.bottom(), t.border);
        y += layout::ROW_H;
    }
    // 关闭到托盘
    {
        let area = R::new(card.x + 18.0, y, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(
            p,
            area,
            "关闭窗口时最小化到托盘",
            Some("关掉则点 × 直接退出"),
            40.0,
        );
        let sw = R::new(ctl.x + 4.0, ctl.cy() - 10.0, 40.0, 20.0);
        // 托盘关掉以后这个开关**必须是点不动的**，不能只画淡。
        // `switch` 的 `enabled` 只影响颜色，命中区还得自己决定要不要登记 ——
        // 原先这里无条件 `ui.hot`，结果是灰的开关照样能点，点完
        // `close_to_tray` 变了但用户不知道它什么时候才会生效。
        let can_toggle = d.cfg.tray_enabled;
        p.switch(sw, d.cfg.close_to_tray, can_toggle);
        if can_toggle {
            ui.hot(sw, Act::Toggle(Pref::CloseToTray));
        } else {
            // 登记成 `None` 而不是不登记：让 `hit()` 在这里**命中并吞掉**点击。
            // 不登记的话点击会穿透到下层（比如把整张卡片当成空白），
            // 虽然目前下层没有别的控件，但这个假设一旦被后来的改动打破
            // 就是「点灰开关触发了别的东西」。
            ui.hot(sw, Act::None);
        }
        p.hline(card.x, card.right(), area.bottom(), t.border);
        y += layout::ROW_H;
    }
    // 开机自启
    {
        let area = R::new(card.x + 18.0, y, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(p, area, "开机自动启动", Some(d.autostart_text), 40.0);
        // 真实状态从 `autostart` 模块来，这里只显示；点击发 Toggle
        let on = d.autostart_text.contains("已启用");
        let sw = R::new(ctl.x + 4.0, ctl.cy() - 10.0, 40.0, 20.0);
        p.switch(sw, on, true);
        ui.hot(sw, Act::Toggle(Pref::AutoStart));
        p.hline(card.x, card.right(), area.bottom(), t.border);
        y += layout::ROW_H;
    }
    // 刷新间隔
    {
        let area = R::new(card.x + 18.0, y, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(p, area, "后台刷新间隔", Some("读取鼠标状态的频率"), 150.0);
        drop_button(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, 150.0, 30.0),
            Drop::Refresh,
            d,
        );
    }
    c.y = card.bottom();

    // ---- 数据 ----
    c.h2(p, "数据", false);
    let card = c.card(p, layout::ROW_H * 3.0);
    let mut y = card.y;
    // 缓存有效期
    {
        let area = R::new(card.x + 18.0, y, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(p, area, "快照缓存有效期", Some("设为「每次都重读」会明显更慢"), 150.0);
        drop_button(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, 150.0, 30.0),
            Drop::CacheTtl,
            d,
        );
        p.hline(card.x, card.right(), area.bottom(), t.border);
        y += layout::ROW_H;
    }
    // 备份与还原
    {
        let area = R::new(card.x + 18.0, y, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(p, area, "备份与还原", Some("把全部 31 项参数存成一个文本文件"), 230.0);
        let b1 = R::new(ctl.x, ctl.cy() - 15.0, 110.0, 30.0);
        button(p, ui, b1, "备份到文件", false, Act::Backup);
        let b2 = R::new(b1.right() + 10.0, ctl.cy() - 15.0, 110.0, 30.0);
        button(p, ui, b2, "从文件还原", false, Act::Restore);
        p.hline(card.x, card.right(), area.bottom(), t.border);
        y += layout::ROW_H;
    }
    // 历史记录
    {
        let area = R::new(card.x + 18.0, y, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(p, area, "历史记录", Some("电量采样点，最多保留 48 小时"), 230.0);
        let b1 = R::new(ctl.x, ctl.cy() - 15.0, 130.0, 30.0);
        button(p, ui, b1, "打开所在文件夹", false, Act::OpenHistoryFolder);
        let b2 = R::new(b1.right() + 10.0, ctl.cy() - 15.0, 90.0, 30.0);
        button(p, ui, b2, "清空", false, Act::ClearHistory);
    }
    c.y = card.bottom();

    // 路径说明。
    //
    // 原来这里写的是 `%APPDATA%\rog-mouse-ctl\settings.ini` 这样的**字面量**，
    // 用户没法直接拿去地址栏粘 —— 想打开看看设置里到底存了什么，还得自己
    // 把 `%APPDATA%` 在脑子里展开。直接给解析后的绝对路径。
    //
    // 宽度的问题：`crate::paths::settings_file()` 展开后接近 70 个字符，
    // 用 `font::TINY` 在 860px 内容区里放不下会被 Ellipsis 截掉尾部 ——
    // 而路径**尾部才是文件名**，截掉就完全没用了。所以换成 `HINT` 之外
    // 的做法：允许它占满内容宽度，并且从**中间**截断（保留 `…\settings.ini`
    // 这个信息量最大的尾巴由 `Painter::text` 的 EllipsisCharacter 天然完成，
    // 这里只保证给够宽度）。
    c.y += 10.0;
    let settings_path = crate::paths::settings_file().display().to_string();
    p.text(
        &format!("设置文件：{settings_path}"),
        R::new(c.x + 18.0, c.y, c.w - 36.0, 16.0),
        font::TINY,
        false,
        t.text3,
        Align::Left,
    );
    c.y += 18.0;
    let history_path = crate::history::history_path_display();
    p.text(
        &format!("历史文件：{history_path}"),
        R::new(c.x + 18.0, c.y, c.w - 36.0, 16.0),
        font::TINY,
        false,
        t.text3,
        Align::Left,
    );
    c.y += 18.0;

    // ---- 外观 ----
    c.h2(p, "外观", false);
    let card = c.card(p, layout::ROW_H);
    {
        let area = R::new(card.x + 18.0, card.y, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(p, area, "配色", Some("跟随系统或手动选择"), 160.0);
        let sel = match ui.mode {
            crate::ui::theme::Mode::Light => 0,
            crate::ui::theme::Mode::Dark => 1,
        };
        segmented(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, 160.0, 30.0),
            &["浅色", "深色"],
            sel,
            |i| {
                Act::SetTheme(if i == 0 {
                    crate::ui::theme::Mode::Light
                } else {
                    crate::ui::theme::Mode::Dark
                })
            },
        );
    }
    c.y = card.bottom();

    let _ = Breakpoint::Wide;
}

// ───────────────────────── 测试 ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 下拉候选不含设备不认的灯光模式() {
        // Rainbow(3) 与 Comet(5) 写进去会被设备静默回落成 ColorCycle。
        // 列在候选里等于骗用户「可以选」。
        let mut cfg = crate::settings::Settings::default();
        cfg.refresh_interval_sec = 60;
        let est = crate::history::Estimate::default();
        let d = Data {
            state: None,
            ident: None,
            status: String::new(),
            plot: None,
            estimate: &est,
            rows: 0,
            cfg: &cfg,
            autostart_text: "",
            version: "test",
            frames: 0,
            elapsed_ms: 0,
            from_cache: false,
        };
        let modes: Vec<usize> = options(Drop::LightMode, &d).into_iter().map(|(_, v)| v).collect();
        assert!(!modes.contains(&3), "3 = Rainbow 不该出现在候选里");
        assert!(!modes.contains(&5), "5 = Comet 不该出现在候选里");
        assert!(modes.contains(&0));
        assert!(modes.contains(&0xF0));
    }

    #[test]
    fn 电量环颜色按优先级判() {
        // 用户 m12899：「20%以下为红色，充电的时候是绿色」。
        // 难点全在优先级上，逐条钉死。
        let t = &crate::ui::theme::LIGHT;

        // 1. 读不到设备 → 灰，**不能**是红色。
        //    休眠时真实电量可能是 77%，画红环等于报假警。
        assert_eq!(ring_color(t, false, false, 0), t.track);
        assert_eq!(ring_color(t, false, true, 0), t.track);

        // 2. 充电中永远绿，**即使电量很低**。
        //    这条是顺序陷阱：先判 `level <= 20` 会把「正在充」显示成红色。
        assert_eq!(ring_color(t, true, true, 5), t.green);
        assert_eq!(ring_color(t, true, true, 20), t.green);
        assert_eq!(ring_color(t, true, true, 100), t.green);

        // 3. ≤20% 才是红色（边界含 20）。
        assert_eq!(ring_color(t, true, false, 0), t.red);
        assert_eq!(ring_color(t, true, false, 20), t.red);

        // 4. 21% 起回到蓝。
        assert_eq!(ring_color(t, true, false, 21), t.accent);
        assert_eq!(ring_color(t, true, false, 100), t.accent);
    }

    #[test]
    fn 按键目标码有名字_未知的原样显示() {
        assert_eq!(button_target_name(0x01F0), "左键点击");
        assert_eq!(button_target_name(0x0000), "无功能");
        // 不认识的码不能猜成别的功能
        assert_eq!(button_target_name(0x1234), "0x1234");
    }

    #[test]
    fn 休眠档位候选包含永不() {
        let cfg = crate::settings::Settings::default();
        let est = crate::history::Estimate::default();
        let d = Data {
            state: None,
            ident: None,
            status: String::new(),
            plot: None,
            estimate: &est,
            rows: 0,
            cfg: &cfg,
            autostart_text: "",
            version: "test",
            frames: 0,
            elapsed_ms: 0,
            from_cache: false,
        };
        let opts = options(Drop::Sleep, &d);
        // 0..4 五档 + 永不
        assert_eq!(opts.len(), 6);
        assert_eq!(opts[5].1, 0xFF);
        assert_eq!(opts[5].0, "永不");
    }

    #[test]
    fn 当前档位下标不会越界() {
        let mut s = crate::proto::State::default();
        // active_stage 是 1-based，读到 0 表示「还没读到」—— 不能拿出 -1
        s.active_stage = 0;
        assert_eq!(cur_stage(&s), 0);
        s.active_stage = 4;
        assert_eq!(cur_stage(&s), 3);
        s.active_stage = 9;
        assert_eq!(cur_stage(&s), 3, "超范围要夹到最后一档");
    }
}
