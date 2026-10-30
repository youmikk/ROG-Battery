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

/// 设置页「关于」里的开源仓库地址。
const REPO_URL: &str = "https://github.com/youmikk/ROG-Battery";
/// 设置页「关于」里的商店产品页地址。
const STORE_URL: &str = "https://apps.microsoft.com/detail/9NRC8HCD4BZ7";

/// 页面要显示的数据。由 `gui.rs` 从 `Shared` 里凑出来。
#[derive(Clone)]
pub struct Data<'a> {
    /// 设备状态。`None` = 还没读到（首帧 / 后台线程正在读）。
    pub state: Option<&'a crate::proto::State>,
    /// 设备身份（型号、SN、固件）。
    pub ident: Option<&'a crate::proto::Identity>,
    /// 实测链路模式（`"Wired"` / `"Wireless24G"`，空串 = 还没读到）。
    ///
    /// 不能用 `ident.dongle_sn` 是否为空来判有线无线 —— 2.4G 下那个字段
    /// 也可能读成空串，会把无线误显示成有线。
    pub link_mode: &'a str,
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

/// 连接方式文字。优先用实测链路模式（`hid::open_link` 给的 `Wired` /
/// `Wireless24G`），只有还没读到实测值时才退回 `dongle_sn` 判据。
fn link_text(d: &Data) -> &'static str {
    match d.link_mode {
        "Wired" => "有线",
        "Wireless24G" => "2.4G 无线",
        _ => {
            let has_dongle = d
                .ident
                .map(|i| !i.dongle_sn.trim().is_empty())
                .unwrap_or(false);
            if has_dongle {
                "2.4G 无线"
            } else {
                "有线"
            }
        }
    }
}

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

    /// 包含设备型号徽章的页标题（用户需求：标题栏不显示名字，在电量页展示鼠标名称）。
    fn h1_with_model(&mut self, p: &mut Painter, s: &str, model: &str) {
        let top = self.y + 10.0;
        let (tw, _) = p.measure(s, font::H1, true);
        p.text(
            s,
            R::new(self.x, top, tw + 8.0, layout::H1_H),
            font::H1,
            true,
            p.t.text,
            Align::Left,
        );
        if !model.is_empty() {
            let (mw, _) = p.measure(model, 12.0, true);
            let badge_w = mw + 26.0;
            let badge_r = R::new(self.x + tw + 16.0, top + 4.0, badge_w, layout::H1_H - 8.0);
            p.round(badge_r, 6.0, p.t.bg_sunken);
            p.round_stroke(badge_r, 6.0, p.t.border, 1.0);
            // 绿色状态圆点
            p.round(R::new(badge_r.x + 8.0, badge_r.cy() - 3.0, 6.0, 6.0), 3.0, p.t.green);
            p.text(
                model,
                R::new(badge_r.x + 18.0, badge_r.y + (badge_r.h - 16.0) / 2.0, mw + 4.0, 16.0),
                12.0,
                true,
                p.t.text2,
                Align::Left,
            );
        }
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
            R::new(area.x, area.y + 10.0, text_w, 20.0),
            font::BODY,
            false,
            p.t.text,
            Align::Left,
        );
        p.text(
            d,
            R::new(area.x, area.y + 30.0, text_w, 16.0),
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
    // Win11 分段控件：外层凹陷底 + 3px 内边距 + 浮起的选中胶囊。
    p.round(r, 9.0, p.t.bg_sunken);
    p.round_stroke(r, 9.0, p.t.border, 1.0);
    let inner = r.inset(3.0);
    let n = labels.len().max(1);
    let seg_w = (inner.w - 2.0 * (n as f32 - 1.0)) / n as f32;
    for (i, label) in labels.iter().enumerate() {
        let x = inner.x + i as f32 * (seg_w + 2.0);
        let sr = R::new(x, inner.y, seg_w, inner.h);
        let on = i == selected;
        if on {
            // 选中项浮起：一层投影 + 强调色填充
            p.round(
                R::new(sr.x, sr.y + 1.0, sr.w, sr.h),
                7.0,
                with_alpha(0x0000_0000, 0x1C),
            );
            p.round(sr, 7.0, p.t.accent);
        } else if ui.hover == Some(act_of(i)) {
            p.round(sr, 7.0, with_alpha(p.t.text, 0x0C));
        }
        let fg = if on { p.t.accent_fg } else { p.t.text2 };
        p.text(label, sr, 12.0, on, fg, Align::Center);
        ui.hot(sr, act_of(i));
    }
}

/// 下拉框外观（`.select`）。点击区域是整个矩形。
fn select_box(p: &mut Painter, r: R, text: &str, open: bool) {
    let rad = 8.0;
    p.round(r, rad, p.t.bg_sunken);
    p.round_stroke(
        r,
        rad,
        if open { p.t.accent } else { p.t.border_strong },
        1.0,
    );
    p.text(
        text,
        R::new(r.x + 11.0, r.y, r.w - 32.0, r.h),
        12.5,
        false,
        p.t.text,
        Align::Left,
    );
    // 右侧小三角（原型 `.caret`）：打开时朝上，收起时朝下。
    let cx = r.right() - 14.0;
    let cy = r.cy();
    let color = if open { p.t.accent } else { p.t.text3 };
    if open {
        p.polyline(
            &[(cx - 4.0, cy + 2.0), (cx, cy - 2.5), (cx + 4.0, cy + 2.0)],
            color,
            1.5,
        );
    } else {
        p.polyline(
            &[(cx - 4.0, cy - 2.0), (cx, cy + 2.5), (cx + 4.0, cy - 2.0)],
            color,
            1.5,
        );
    }
}

/// 主按钮 / 次按钮（`.btn`）。
fn button(p: &mut Painter, ui: &mut Ui, r: R, label: &str, primary: bool, act: Act) {
    let hovered = ui.hover == Some(act);
    let rad = 8.0;
    if primary {
        let bg = if hovered {
            with_alpha(p.t.accent, 0xE0)
        } else {
            p.t.accent
        };
        p.round(
            R::new(r.x, r.y + 1.0, r.w, r.h),
            rad,
            with_alpha(0x0000_0000, 0x1E),
        );
        p.round(r, rad, bg);
        p.text(label, r, 12.5, true, p.t.accent_fg, Align::Center);
    } else {
        let bg = if hovered { p.t.card_hover } else { p.t.card };
        p.round(r, rad, bg);
        p.round_stroke(
            r,
            rad,
            if hovered { p.t.accent } else { p.t.border_strong },
            1.0,
        );
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
    // 数字水平居中对齐，排版更对称平衡。
    // 左右各让 6px：万一草稿比框还宽（用户硬敲到上限），字也不会压到描边上。
    p.text(
        text,
        R::new(r.x + 6.0, r.y, (r.w - 12.0).max(8.0), r.h),
        12.5,
        false,
        p.t.text,
        Align::Center,
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

// ───────────────────────── 色盘 ─────────────────────────

/// 色盘格子边长与间距（逻辑像素）。
const PAL_CELL: f32 = 30.0;
const PAL_GAP: f32 = 6.0;
/// 一行几个色块。
///
/// **必须是 12**：色相每 30 度一格正好 12 格，一行才是一个完整的色相环。
/// 一开始写的 8，结果 12 个饱和色排在 8 列里要折行 —— 第二行前 4 格是
/// 饱和色、后 4 格已经是淡色，界面上看是两种色系糊在一起（真机截图确认）。
const PAL_COLS: usize = 12;

/// HSV → RGB。`h` 是 0–360 度，`s`/`v` 是 0–1。
///
/// 自己算而不是查表：色盘要的是「色相均分」这个性质，写成 12 个角度的循环
/// 比手抄 24 个十六进制更不容易抄错，也顺手给出两档明度。
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let c = v * s;
    let hp = (h / 60.0).clamp(0.0, 6.0);
    let x = c * (1.0 - ((hp % 2.0) - 1.0).abs());
    let (r1, g1, b1) = match hp as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let f = |u: f32| ((u + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    [f(r1), f(g1), f(b1)]
}

/// 色盘的候选颜色（36 个，12 列 × 3 行）。
///
/// 组成：12 个色相 × 2 档明度（饱和 + 淡色）+ 12 级灰阶。
///
/// 为什么不是「二十四色手抄表」：色相均分是这里唯一要的性质，手抄一边
/// 多写一个少写一个都不报错，只会让某个色相在盘上占两格 —— 看不见的错。
///
/// 灰阶放最后一行：灯光常设成白色/关闭，给一整行 0–255 的灰比在彩色里
/// 找白色直观。**12 级灰而不是 8 级**：这样三行都正好填满 12 列，
/// 最后一行右边不会空出 4 格（真机截图里那 4 个空位看着像画漏了）。
pub fn palette_colors() -> Vec<[u8; 3]> {
    let mut out = Vec::with_capacity(PAL_COLS * 3);
    // 前 12：色相每 30 度一格，饱和
    for i in 0..PAL_COLS {
        out.push(hsv_to_rgb(i as f32 * 30.0, 1.0, 1.0));
    }
    // 中 12：同一组色相的淡色
    for i in 0..PAL_COLS {
        out.push(hsv_to_rgb(i as f32 * 30.0, 0.45, 1.0));
    }
    // 后 12：灰阶（含纯黑与纯白）
    for i in 0..PAL_COLS {
        let v = i as f32 / (PAL_COLS - 1) as f32;
        let g = (v * 255.0).round() as u8;
        out.push([g, g, g]);
    }
    out
}

/// 某个颜色在色盘里的下标。找不到返 `None`（用户从十六进制框输入了盘上没有的颜色）。
pub fn palette_index(rgb: [u8; 3]) -> Option<usize> {
    palette_colors().iter().position(|c| *c == rgb)
}

/// 色盘第 `idx` 个颜色。越界返 `None`。
///
/// 支持 0..36 预设下标，以及 >= 0x0100_0000 的 24-bit 自定义 RGB 编码。
pub fn palette_color(idx: usize) -> Option<[u8; 3]> {
    if idx >= 0x0100_0000 {
        let r = ((idx >> 16) & 0xFF) as u8;
        let g = ((idx >> 8) & 0xFF) as u8;
        let b = (idx & 0xFF) as u8;
        Some([r, g, b])
    } else {
        palette_colors().get(idx).copied()
    }
}

/// 把自定义 RGB 颜色打包进 `usize`（保留高字节标志位）。
pub fn pack_custom_rgb(rgb: [u8; 3]) -> usize {
    0x0100_0000 | ((rgb[0] as usize) << 16) | ((rgb[1] as usize) << 8) | (rgb[2] as usize)
}

/// RGB 转 HSV（H: 0..360, S: 0..1, V: 0..1）。
pub fn rgb_to_hsv(rgb: [u8; 3]) -> (f32, f32, f32) {
    let r = rgb[0] as f32 / 255.0;
    let g = rgb[1] as f32 / 255.0;
    let b = rgb[2] as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let v = max;
    let s = if max > 1e-5 { delta / max } else { 0.0 };
    let h = if delta < 1e-5 {
        0.0
    } else if (max - r).abs() < 1e-5 {
        60.0 * (((g - b) / delta) % 6.0)
    } else if (max - g).abs() < 1e-5 {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };
    let h = if h < 0.0 { h + 360.0 } else { h };
    (h, s, v)
}

/// 色盘弹层的逻辑尺寸（含内边距）。
fn palette_size(tab: u8) -> (f32, f32) {
    let w = PAL_COLS as f32 * PAL_CELL + (PAL_COLS - 1) as f32 * PAL_GAP + 16.0;
    let h = if tab == 1 {
        // 标签栏(28) + 间距 + 色环(136) + 标题 + 9 色阶(26) + 应用按钮(28) + 内边距
        282.0
    } else {
        // 标签栏(28) + 间距 + 3 行网格(102) + 内边距
        156.0
    };
    (w, h)
}

/// 画色盘本体，支持「常用预设」网格与「自定义色环+深浅色阶」双模式。
pub(crate) fn draw_palette(p: &mut Painter, ui: &mut Ui, slot: u8, anchor: R, cur: Option<[u8; 3]>, f: &Frame) {
    let (w, h) = palette_size(ui.palette_tab);
    let scroll = ui.scroll_y(ui.page);
    let view_h = f.content.h;
    let x = anchor.x.min((f.content.right() - w).max(f.content.x)).max(f.content.x);
    let y = anchor
        .y
        .min((scroll + view_h - h - 8.0).max(scroll + 8.0))
        .max(scroll + 8.0);
    let r = R::new(x, y, w, h);

    // 阴影 + 卡片，和文字弹层同一套观感
    p.round(
        R::new(r.x + 1.0, r.y + 2.0, r.w, r.h),
        layout::R_CARD,
        with_alpha(0x0000_0000, 0x28),
    );
    p.card(r, layout::R_CARD, p.t.card, p.t.border_strong);

    let drop = Drop::Palette(slot);

    // 顶部分段标签：[ 常用预设 | 自定义色环 ]
    // 和页面里的 `segmented` 用同一套观感：沉底槽 + 选中项 accent 药丸。
    let pad = 8.0;
    let bar = R::new(r.x + pad, r.y + pad, r.w - pad * 2.0, 28.0);
    p.round(bar, 9.0, p.t.bg_sunken);
    p.round_stroke(bar, 9.0, p.t.border, 1.0);

    let inner = bar.inset(3.0);
    let tab_w = (inner.w - 2.0) / 2.0;
    let tab0 = R::new(inner.x, inner.y, tab_w, inner.h);
    let tab1 = R::new(tab0.right() + 2.0, inner.y, tab_w, inner.h);

    for (i, tab) in [(0u8, tab0), (1u8, tab1)] {
        let label = if i == 0 { "常用预设" } else { "自定义色环" };
        if ui.palette_tab == i {
            // 选中项：一枚投影 + accent 药丸，和页面里的分段控件一致
            p.round(R::new(tab.x, tab.y + 1.0, tab.w, tab.h), 7.0, with_alpha(0x0000_0000, 0x1C));
            p.round(tab, 7.0, p.t.accent);
            p.text(label, tab, 11.5, true, p.t.accent_fg, Align::Center);
        } else {
            p.text(label, tab, 11.5, false, p.t.text2, Align::Center);
            ui.hot(tab, Act::SetPaletteTab(i));
        }
    }

    let body_top = bar.bottom() + 10.0;

    if ui.palette_tab == 0 {
        // ---- 模式 0：常用预设网格 ----
        let colors = palette_colors();
        let cur_idx = cur.and_then(palette_index);
        for (i, c) in colors.iter().enumerate() {
            let col = i % PAL_COLS;
            let row = i / PAL_COLS;
            let cr = R::new(
                r.x + 8.0 + col as f32 * (PAL_CELL + PAL_GAP),
                body_top + row as f32 * (PAL_CELL + PAL_GAP),
                PAL_CELL,
                PAL_CELL,
            );
            let fill = 0xFF00_0000 | ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32;
            p.round(cr, 4.0, fill);
            p.round_stroke(cr, 4.0, with_alpha(p.t.text, 0x24), 1.0);
            let act = Act::PickDrop(drop, i);
            if Some(i) == cur_idx {
                p.round_stroke(
                    R::new(cr.x - 2.0, cr.y - 2.0, cr.w + 4.0, cr.h + 4.0),
                    6.0,
                    p.t.accent,
                    2.0,
                );
            } else if ui.hover == Some(act) {
                p.round_stroke(
                    R::new(cr.x - 2.0, cr.y - 2.0, cr.w + 4.0, cr.h + 4.0),
                    6.0,
                    with_alpha(p.t.text, 0x60),
                    2.0,
                );
            }
            ui.hot(cr, act);
        }
    } else {
        // ---- 模式 1：自定义色环 + 深浅色阶 ----
        let active_hue = if ui.custom_hue > 0.0 {
            ui.custom_hue
        } else if let Some(c) = cur {
            rgb_to_hsv(c).0
        } else {
            0.0
        };

        // 1. 色环区：中心位于卡片水平中央
        let cx = r.cx();
        let ring_radius = 60.0;
        let ring_width = 16.0;
        let cy = body_top + ring_radius + ring_width * 0.5 + 2.0;

        // 绘制 36 段连续彩虹色环
        for seg in 0..36 {
            let start = seg as f32 * 10.0;
            let c = hsv_to_rgb(start + 5.0, 1.0, 1.0);
            let argb = 0xFF00_0000 | ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | (c[2] as u32);
            p.arc(cx, cy, ring_radius, ring_width, start, 10.5, argb);
        }

        // 环上交互点：24 个径向扇区（命中盒按环带宽放大，窄环也好点）
        let hit = ring_width + 12.0;
        for a in 0..24 {
            let deg = a as f32 * 15.0;
            let rad = deg.to_radians();
            let bx = cx + ring_radius * rad.cos() - hit * 0.5;
            let by = cy + ring_radius * rad.sin() - hit * 0.5;
            ui.hot(R::new(bx, by, hit, hit), Act::SetCustomHue(drop, deg));
        }

        // 当前色相指示标（外圈白色圆环标针）
        let rad = active_hue.to_radians();
        let hx = cx + ring_radius * rad.cos();
        let hy = cy + ring_radius * rad.sin();
        p.circle(hx, hy, 6.5, 0xFFFFFFFF);
        p.circle_stroke(hx, hy, 6.5, 0xFF222222, 1.5);

        // 色环中心：当前色相的纯色圆片 + 十六进制预览
        let active_rgb = hsv_to_rgb(active_hue, 1.0, 1.0);
        let active_packed = pack_custom_rgb(active_rgb);
        let center_r = 30.0;
        let center_argb = 0xFF00_0000 | ((active_rgb[0] as u32) << 16) | ((active_rgb[1] as u32) << 8) | (active_rgb[2] as u32);
        p.circle(cx, cy, center_r, center_argb);
        p.circle_stroke(cx, cy, center_r, p.t.border_strong, 1.0);
        ui.hot(R::new(cx - center_r, cy - center_r, center_r * 2.0, center_r * 2.0), Act::PickDrop(drop, active_packed));

        let hex_now = format!("#{:02X}{:02X}{:02X}", active_rgb[0], active_rgb[1], active_rgb[2]);
        p.text(&hex_now, R::new(cx - 44.0, cy + 34.0, 88.0, 14.0), 10.5, true, p.t.text3, Align::Center);

        // 2. 深浅色阶区：9 级渐变深浅档位
        let sy = cy + ring_radius + ring_width * 0.5 + 8.0;
        p.text("选择深浅（点击直接应用）", R::new(r.x + 8.0, sy, 220.0, 14.0), 11.0, false, p.t.text3, Align::Left);

        let shades = [
            [255, 255, 255],                      // 0: 纯白
            hsv_to_rgb(active_hue, 0.25, 1.0),    // 1: 极浅
            hsv_to_rgb(active_hue, 0.50, 1.0),    // 2: 柔和
            hsv_to_rgb(active_hue, 0.75, 1.0),    // 3: 靓丽
            hsv_to_rgb(active_hue, 1.0, 1.0),     // 4: 饱和纯色
            hsv_to_rgb(active_hue, 1.0, 0.80),    // 5: 浓郁
            hsv_to_rgb(active_hue, 1.0, 0.60),    // 6: 深暗
            hsv_to_rgb(active_hue, 1.0, 0.35),    // 7: 极深
            [0, 0, 0],                            // 8: 纯黑
        ];

        // 9 格等分卡片宽度，避免右侧留一截空白
        let swatch_y = sy + 18.0;
        let swatch_h = 26.0;
        let swatch_gap = 6.0;
        let inner_w = r.w - 16.0;
        let swatch_w = (inner_w - swatch_gap * 8.0) / 9.0;

        for (k, sc) in shades.iter().enumerate() {
            let sx = r.x + 8.0 + k as f32 * (swatch_w + swatch_gap);
            let sr = R::new(sx, swatch_y, swatch_w, swatch_h);
            let s_argb = 0xFF00_0000 | ((sc[0] as u32) << 16) | ((sc[1] as u32) << 8) | (sc[2] as u32);
            p.round(sr, 5.0, s_argb);
            p.round_stroke(sr, 5.0, with_alpha(p.t.text, 0x24), 1.0);

            let s_act = Act::PickDrop(drop, pack_custom_rgb(*sc));
            if cur == Some(*sc) {
                p.round_stroke(R::new(sr.x - 2.0, sr.y - 2.0, sr.w + 4.0, sr.h + 4.0), 6.0, p.t.accent, 2.0);
            } else if ui.hover == Some(s_act) {
                p.round_stroke(R::new(sr.x - 2.0, sr.y - 2.0, sr.w + 4.0, sr.h + 4.0), 6.0, with_alpha(p.t.text, 0x60), 2.0);
            }
            ui.hot(sr, s_act);
        }

        // 3. 底部快捷栏：一键应用当前色相的饱和纯色
        let by = swatch_y + swatch_h + 8.0;
        let btn_r = R::new(r.x + 8.0, by, r.w - 16.0, 28.0);
        button(p, ui, btn_r, &format!("应用当前纯色 ({hex_now})"), true, Act::PickDrop(drop, active_packed));
    }

    ui.set_drop_rect(r);
}

/// 色块按钮：画一个颜色方块，点它开色盘。
fn color_button(p: &mut Painter, ui: &mut Ui, r: R, slot: u8, rgb: Option<[u8; 3]>) {
    swatch(p, r, rgb);
    let drop = Drop::Palette(slot);
    ui.hot(r, Act::OpenDrop(drop));
    if ui.open_drop() == Some(drop) {
        let (w, _) = palette_size(ui.palette_tab);
        ui.set_drop_rect(R::new(r.x, r.bottom() + 4.0, w, 0.0));
    }
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
    let t = p.t;
    let (bg, icon) = if warn {
        (t.warn_soft, t.warn)
    } else {
        (t.accent_soft, t.accent)
    };
    let pad = 12.0;
    let icon_d = 16.0;
    let gap = 10.0;
    let text_x = r.x + pad + icon_d + gap;
    let text_w = (r.w - (text_x - r.x) - pad).max(80.0);
    // 高度按文本长度估：12px 中文一行约 40 字，行高 1.55 倍。
    // 这不是精确排版，但说明条只有一两行，误差在 2px 内看不出来。
    let per_line = (text_w / 12.0).max(10.0);
    let lines = (text.chars().count() as f32 / per_line).ceil().max(1.0);
    let h = pad * 2.0 + lines * 12.0 * 1.55;
    let rr = R::new(r.x, r.y, r.w, h);

    // 圆角胶囊 + 与图标同色的细描边，比纯色块更有层次。
    p.round(rr, 10.0, bg);
    p.round_stroke(rr, 10.0, with_alpha(icon, 0x33), 1.0);

    // 左侧圆形徽标
    let cx = rr.x + pad + icon_d / 2.0;
    let cy = rr.cy();
    p.circle(cx, cy, icon_d / 2.0, with_alpha(icon, 0x26));
    p.text(
        if warn { "!" } else { "i" },
        R::new(cx - 6.0, cy - 7.0, 12.0, 14.0),
        10.5,
        true,
        icon,
        Align::Center,
    );

    p.text_wrap(
        text,
        R::new(text_x, rr.y + pad, text_w, rr.h - pad * 2.0),
        12.0,
        with_alpha(t.text, 0xD2),
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
    // 顶部 36px 是标题栏带，导航**不能**画进去 —— 否则最后一趟标题栏会用底色
    // 把导航顶部（设备卡）盖掉一半，看起来"顶部被切了一刀"。
    let top = crate::ui::titlebar::TITLE_H;
    // 底色 + 右边一条 1px 分隔线（都只从标题栏之下开始铺）
    p.rect(R::new(0.0, top, nav_w, win_h - top), t.bg_nav);
    p.line(nav_w - 0.5, top, nav_w - 0.5, win_h, t.border, 1.0);

    let rail = ui.rail;
    let pad = if rail { 8.0 } else { 8.0 };
    let item_h = layout::NAV_ITEM_H;

    // ---- 设备卡（仅展开时）----
    let mut y = top + 12.0;
    if !rail {
        let dc = R::new(4.0, y, nav_w - 12.0, 66.0);
        p.card(dc, 8.0, t.card, t.border);
        let model = d
            .ident
            .map(|i| i.display_name())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "ROG 鼠标".to_string());
        p.text(
            &model,
            R::new(dc.x + 12.0, dc.y + 10.0, dc.w - 24.0, 18.0),
            13.0,
            true,
            t.text,
            Align::Left,
        );
        p.text(
            link_text(d),
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
                &format!("v{}", d.version),
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
        match drop {
            // 色盘是网格布局，和文字下拉不是同一套定位，走单独一支
            Drop::Palette(slot) => {
                if let Some(anchor) = ui.drop_rect() {
                    let cur = match slot {
                        crate::ui::widget::LIGHT_COLOR_SLOT => d.state.map(|s| s.light_rgb),
                        i => d.state.map(|s| s.rgb[(i as usize) & 3]),
                    };
                    draw_palette(p, ui, slot, anchor, cur, f);
                }
            }
            _ => draw_drop(p, ui, d, drop, f.content.h),
        }
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
    /// 完全显示的时长。
    const HOLD: f32 = 2.0;
    /// 淡出时长。
    const FADE: f32 = 0.32;

    // 自动消退：瞬时提示（有 `status_time`）到点自己消失，
    // 常驻错误（`d.status`）没有时间戳，一直留着直到问题消失。
    let mut fade = 1.0f32;
    if let Some(t0) = ui.status_time {
        let el = t0.elapsed().as_secs_f32();
        if el >= HOLD + FADE {
            ui.clear_status();
        } else if el > HOLD {
            fade = 1.0 - (el - HOLD) / FADE;
        }
    }

    let transient = ui.status.trim();
    let raw = if !transient.is_empty() {
        transient
    } else {
        d.status.trim()
    };
    if raw.is_empty() {
        return;
    }

    // 剥掉正文里的 ✔ / ✘ 前缀 —— 状态图标已经表达了同一件事，
    // 留着就是「✘ 打叉图标 + ✘ 字符」双份。
    let mut kind = 2u8; // 0=错误 1=成功 2=普通
    let mut body = raw;
    for (pre, k) in [("✘", 0u8), ("✔", 1u8), ("⚠", 2u8), ("!", 2u8)] {
        if let Some(rest) = body.strip_prefix(pre) {
            kind = k;
            body = rest.trim_start();
            break;
        }
    }
    if body.is_empty() {
        return;
    }

    let t = p.t;
    let scroll = ui.scroll_y(ui.page);
    let view_h = f.content.h;

    let (fg_icon, bg_icon) = match kind {
        0 => (t.red, t.red_soft),
        1 => (t.green, t.green_soft),
        _ => (t.accent, t.accent_soft),
    };
    let glyph = match kind {
        0 => "×",
        1 => "✓",
        _ => "i",
    };

    // 宽度跟着文字走，最宽不超过内容区。
    let (tw, _) = p.measure(body, font::HINT, false);
    let icon_d = 18.0;
    let pad_x = 12.0;
    let gap = 9.0;
    let w = (tw + pad_x * 2.0 + icon_d + gap).min(f.width.max(180.0));
    let h = 40.0;
    let r = R::new(
        f.content.x + (f.width - w) / 2.0,
        scroll + view_h - h - 20.0,
        w,
        h,
    );

    let a = |c: u32| with_alpha(c, ((c >> 24) as f32 * fade) as u8);

    // 投影：两层偏移淡色，近似 Win11 的浮起感（GDI+ 没有 box-shadow）。
    if fade > 0.99 {
        p.round(
            R::new(r.x + 1.0, r.y + 3.0, r.w, r.h),
            11.0,
            with_alpha(0x0000_0000, 0x22),
        );
    }
    // 卡片本体：比页面底色高一层，靠描边和投影区分，不用刺眼的纯黑胶囊。
    p.round(r, 11.0, a(t.card));
    p.round_stroke(r, 11.0, a(t.border_strong), 1.0);

    // 左侧圆形状态徽标
    let cx = r.x + pad_x + icon_d / 2.0;
    let cy = r.cy();
    p.circle(cx, cy, icon_d / 2.0, a(bg_icon));
    p.text(
        glyph,
        R::new(cx - 7.0, cy - 8.0, 14.0, 16.0),
        11.0,
        true,
        a(fg_icon),
        Align::Center,
    );

    p.text(
        body,
        R::new(r.x + pad_x + icon_d + gap, r.y, r.w - pad_x * 2.0 - icon_d - gap, r.h),
        font::HINT,
        false,
        a(t.text),
        Align::Left,
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
        // 色盘不是文字列表，`draw_palette` 走单独一支。这里给空表而不是
        // `palette_colors()`：万一有人误走文字下拉，空表会显示成「没有选项」
        // （一眼看出走错了），而颜色列表会被画成 32 行没法读的文字。
        Drop::Palette(_) => Vec::new(),
        Drop::CloseMode => vec![
            ("每次询问".to_string(), 0),
            ("最小化到系统托盘".to_string(), 1),
            ("直接退出程序".to_string(), 2),
        ],
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
        // 色盘自己不显示文字（色块本身就是显示）。走到这里是走错分支了，
        // 给「—」而不是编一个色号。
        Drop::Palette(_) => "—".to_string(),
        Drop::CloseMode => match d.cfg.close_mode.as_str() {
            "minimize" => "最小化到系统托盘".to_string(),
            "quit" => "直接退出程序".to_string(),
            _ => "每次询问".to_string(),
        },
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
        Drop::Palette(_) => 0,
        Drop::CloseMode => match d.cfg.close_mode.as_str() {
            "minimize" => 1,
            "quit" => 2,
            _ => 0,
        },
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
        ui.set_drop_btn(r);
    }
}

/// 决定下拉弹层最终落在哪（翻上还是翻下）。
///
/// `anchor` 是 `drop_button` 初次估的矩形（紧贴按钮下方）；`h` 是弹层完整高度；
/// `view_h` 是当前视口高度。
///
/// 判定在**视口空间**做（`btn_bottom - scroll` 等）：
/// - 默认往下开（`btn.bottom() + 4.0`）；
/// - 如果往下开会戳出视口底边、且往上开的空间更大，就**翻到按钮上方**（`btn.y - h - 4.0`）；
/// - 这样在页面滚到任何位置、任何窗口高度下，弹层的全部选项都一定在可视区内。
fn place_drop(ui: &Ui, anchor: R, h: f32, view_h: f32) -> R {
    let scroll = ui.scroll_y(ui.page);
    let view_top = scroll;
    let view_bottom = scroll + view_h;
    let btn = ui.drop_btn().unwrap_or(anchor);

    let down_y = btn.bottom() + 4.0;
    let up_y = btn.y - h - 4.0;

    // 默认往下。只有当「下面放不下」且「上面空间更大」时才翻到上面
    let down_overflow = (down_y + h) > (view_bottom - 8.0);
    let y = if down_overflow {
        let space_below = (view_bottom - 8.0) - down_y;
        let space_above = btn.y - (view_top + 8.0);
        if space_above >= h || space_above > space_below {
            up_y.max(view_top + 8.0)
        } else {
            down_y
        }
    } else {
        down_y
    };

    R::new(anchor.x, y, anchor.w, h)
}

/// 画已经打开的弹层。
///
/// 弹层用**内容坐标**登记（和页面同一套），因为它是跟着内容滚动的。
fn draw_drop(p: &mut Painter, ui: &mut Ui, d: &Data, drop: Drop, view_h: f32) {
    let Some(anchor) = ui.drop_rect() else {
        return;
    };
    let opts = options(drop, d);
    // 空表直接返回：色盘走 `draw_palette`，但它也经过这里（`page()` 里
    // 按 `Drop` 分派）。不返回的话会画出一个 8px 高的空壳，
    // 而且 `set_drop_rect` 会把它登记成一个几乎点不到的矩形。
    if opts.is_empty() {
        return;
    }
    let item_h = 30.0;
    let h = opts.len() as f32 * item_h + 8.0;
    let r = place_drop(ui, anchor, h, view_h);

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
    let model = d
        .ident
        .map(|i| i.display_name())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "未检测到设备".to_string());
    c.h1_with_model(p, "电量", &model);

    let st = d.state;
    let level = st.map(|s| s.power).unwrap_or(0);
    let charging = st.map(|s| s.charging != 0).unwrap_or(false);

    // ---- hero：电量环 + KPI ----
    //
    // 原型里 hero 是**两张并排的卡片**：左边 `.hero-main`（环 + KPI），
    // 右边 `.hero-side`（设备信息，扩展为 268px 以免型号或序列号截断）。
    let side_by_side = f.bp.hero_side_by_side();
    let show_info = side_by_side && c.w > 760.0;
    let ring_d = f.bp.ring_diameter();
    // 环下面要放一行「预计可用 / 预计充满」的结论，所以比原来多留 26px。
    let hero_h = if side_by_side {
        ring_d + 68.0
    } else {
        ring_d + 156.0
    };
    let info_w = 268.0;
    let gap = 16.0;
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
        // KPI 只有两行（≈104px），hero 变高后靠上排会显得下面空一截，居中更像样。
        hero.y + 38.0
    } else {
        hero.y + ring_d + 62.0
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
            // 不再整卡垂直居中：上面要留 20px 边，下面要留一行结论的位置。
            hero.y + 20.0 + ring_d * 0.5
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
        // 把「数字 + 单位/徽章」当作**一组**在环心竖直居中，而不是各自摆放 ——
        // 之前数字框中心在 cy-2、充电徽章中心在 cy+27，整组重心明显偏下，
        // 徽章底边还压到环的下侧笔画，用户反馈「字不居中」。
        if st.is_some() && charging {
            // 数字 + 闪电胶囊：整组高 ≈ 40 + 4 + 16 = 60，从 cy-30 起排。
            p.text(
                &txt,
                R::new(cx - 50.0, cy - 30.0, 100.0, 40.0),
                font::NUM_RING,
                true,
                t.text,
                Align::Center,
            );
            let bw = 56.0;
            let bh = 16.0;
            let br = R::new(cx - bw / 2.0, cy + 10.0, bw, bh);
            p.round(br, 8.0, with_alpha(t.green, 0x2A));
            p.text("⚡充电中", br, 9.5, true, t.green, Align::Center);
        } else if st.is_some() {
            // 数字 + 「%」：数字中心落在 cy-4，「%」贴在其下方，整组居中。
            p.text(
                &txt,
                R::new(cx - 50.0, cy - 24.0, 100.0, 40.0),
                font::NUM_RING,
                true,
                t.text,
                Align::Center,
            );
            p.text(
                "%",
                R::new(cx - 50.0, cy + 12.0, 100.0, 14.0),
                font::TINY,
                false,
                t.text3,
                Align::Center,
            );
        } else {
            // 读不到电量：只画「—」，居中。
            p.text(
                &txt,
                R::new(cx - 50.0, cy - 22.0, 100.0, 40.0),
                font::NUM_RING,
                true,
                t.text,
                Align::Center,
            );
        }
    }

    // ---- 环下方的结论：预计可用 / 预计充满 ----
    //
    // 这是整页最想知道的一句话，所以放在环正下方、用正文色加粗，而不是塞进
    // 下面 2×2 的 KPI 里跟「电压」「回报率」平起平坐。充电 / 放电 / 样本不足
    // 三种说法由 `Estimate::short_text` 统一给出，这里不判断。
    {
        let cx = hero.x + 24.0 + ring_d * 0.5;
        let w = (ring_d + 40.0).max(120.0);
        let y = hero.y + 22.0 + ring_d + 4.0;
        let txt = d.estimate.short_text();
        let known = d.estimate.known;
        p.text(
            &txt,
            R::new(cx - w * 0.5, y, w, 18.0),
            if known { 13.0 } else { 12.0 },
            known,
            if known { t.text } else { t.text3 },
            Align::Center,
        );
    }

    // KPI：电压 / 回报率 / 当前档位 / 变化速率
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
                    "变化速率",
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
                ("变化速率", "—".into()),
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

    // 设备信息：**另起一张卡片**放在 hero 右边（在电量页清晰展示读取到的鼠标型号与状态）。
    if show_info {
        let side = R::new(hero.right() + gap, hero.y, info_w, hero_h);
        p.card(side, layout::R_CARD, t.card, t.border);
        let sx = side.x + 18.0;
        let inner_w = info_w - 36.0;
        let sy = side.y + 16.0;

        // 顶部醒目展示读取到的设备型号名字
        p.text(
            "当前设备",
            R::new(sx, sy, inner_w, 13.0),
            font::TINY,
            false,
            t.text3,
            Align::Left,
        );
        p.text(
            &model,
            R::new(sx, sy + 15.0, inner_w, 18.0),
            13.5,
            true,
            t.text,
            Align::Left,
        );

        let mut ly = sy + 38.0;
        let ident = d.ident;
        // 设备大类：PID 在对照表里就能说出是鼠标 / 键盘 / 手柄。
        let kind_text = {
            let pid = ident
                .map(|i| if i.wired_pid != 0 { i.wired_pid } else { i.wireless_pid })
                .unwrap_or(0);
            crate::devices::find(pid)
                .map(|dv| dv.kind.as_str().to_string())
                .unwrap_or_else(|| "—".to_string())
        };
        let lines: [(&str, String); 4] = [
            (
                "序列号",
                ident.map(|i| i.mouse_sn.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| "—".to_string()),
            ),
            (
                "接收器固件",
                ident
                    .map(|i| i.dongle_firmware.clone())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "—".to_string()),
            ),
            (
                "连接方式",
                if d.state.is_some() {
                    link_text(d).to_string()
                } else {
                    "离线 / 休眠".to_string()
                },
            ),
            ("设备类型", kind_text),
        ];
        for (k, v) in lines.iter() {
            p.text(k, R::new(sx, ly, 74.0, 16.0), 12.0, false, t.text3, Align::Left);
            p.text(
                v,
                R::new(sx + 76.0, ly, inner_w - 76.0, 16.0),
                12.0,
                false,
                t.text2,
                Align::Right,
            );
            ly += 19.0;
        }

        // 状态胶囊。
        //
        // 读不到设备（休眠）时显示「休眠中」灰胶囊 —— **不能**因为 `level`
        // 默认成 0 就落到「电量偏低」分支，那是在没有任何数据的情况下报警。
        let mut bx = sx;
        let by = ly + 4.0;
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
            if bx + w2 <= side.right() - 16.0 {
                badge(p, R::new(bx, by, w2, pill_h), &l, t.text3, t.bg_sunken);
            }
        }
    }

    c.y = hero.bottom();

    // ---- 电量历史 ----
    c.h2(p, "电量历史", false);
    // 卡片高度按 `draw_chart` 的实际内容算：16（上边距）+ 18（标题行）+ 8 +
    // 146（绘图区）+ 20（X 轴时间）+ 12（下边距）= 220。
    // 原来写 `168 + 56 + 28 = 252` 是照抄原型的 SVG viewBox，
    // 底下会空出 46px 一条白边；132 的绘图区又太扁，曲线被压成一条带子。
    let chart = c.card(p, 220.0);
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
        let sw = R::new(ctl.right() - 40.0, ctl.cy() - 10.0, 40.0, 20.0);
        p.switch(sw, on, st.is_some());
        ui.hot(sw, Act::Toggle(Pref::FullEffect));
    }
    c.y = card.bottom();

    // ---- 续航明细 ----
    //
    // 结论已经放到电量环下面了，这里不再重复整句，只留「怎么算出来的」，
    // 免得同一句话在一屏里出现两次。样本不足时这里才显示原因。
    if d.estimate.known {
        c.y += 14.0;
        let line = if d.estimate.stale_seconds > 0 {
            // 沿用旧结论时必须说明白，否则等于拿旧斜率冒充当前状态。
            format!(
                "沿用 {}前的估算 · 当前读数没更新，暂时没法重算",
                crate::history::ago_text(d.estimate.stale_seconds)
            )
        } else {
            format!(
                "按最近 {} 个采样估算 · 变化 {:+.2} %/h",
                d.estimate.samples, d.estimate.percent_per_hour
            )
        };
        p.text(
            &line,
            R::new(c.x + 6.0, c.y, c.w - 12.0, 18.0),
            font::HINT,
            false,
            t.text3,
            Align::Left,
        );
        c.y += 22.0;
    } else if !d.estimate.text.is_empty() {
        c.y += 14.0;
        p.text(
            &d.estimate.text,
            R::new(c.x + 6.0, c.y, c.w - 12.0, 18.0),
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
    // 有充电段才画图例，否则纯放电历史不需要解释那条绿带。
    if d.plot
        .filter(|p| p.enough())
        .map(|p| p.points.iter().any(|pt| pt.charging))
        .unwrap_or(false)
    {
        p.dot(card.x + 98.0, head_y + 9.0, 4.0, with_alpha(t.green, 0xB0));
        p.text(
            "充电",
            R::new(card.x + 106.0, head_y, 34.0, 18.0),
            font::TINY,
            false,
            t.text3,
            Align::Left,
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
    let plot = R::new(card.x + 42.0, head_y + 26.0, card.w - 42.0 - 18.0, 146.0);

    // ── 坐标轴 ────────────────────────────────────────────────────────
    // 刻度要「整除」：四条线等距，否则会出现 92 / 95 / 97 / 100 这种
    // 间距不齐的刻度，一眼看着就毛糙。做法是先按数据跨度取一个整数步长，
    // 再把上下界对齐到步长的整数倍，顶格不超过 100%。
    let (lo, hi) = match d.plot.filter(|p| p.enough()) {
        Some(pl) => {
            let mut mn = 100u8;
            let mut mx = 0u8;
            for pt in &pl.points {
                mn = mn.min(pt.level);
                mx = mx.max(pt.level);
            }
            // 上下各留 2% 余量，否则折线贴着框边
            let (mn, mx) = (mn.saturating_sub(2) as i32, (mx as i32 + 2).min(100));
            let step = (((mx - mn).max(1) as f32) / 4.0).ceil() as i32;
            let step = step.max(1);
            let mut lo = (mn / step) * step;
            let mut hi = lo + step * 4;
            while hi < mx {
                lo += step;
                hi += step;
            }
            if hi > 100 {
                hi = 100;
                lo = hi - step * 4;
            }
            if lo < 0 {
                lo = 0;
                hi = step * 4;
            }
            (lo, hi)
        }
        None => (0, 100),
    };
    let span = (hi - lo).max(1) as f32;

    // x / y 映射：`build` 用它自己的矩形算 x，这里按比例搬到当前 plot，
    // 两份实现各自算时间→x 一定会漂移，所以只搬比例。
    let area_w = (crate::chart::AREA_W as f32).max(1.0);
    let map_x = |px: i32| -> f32 {
        let fx = (px - crate::chart::AREA_X) as f32 / area_w;
        plot.x + fx.clamp(0.0, 1.0) * plot.w
    };
    let map_y = |level: u8| -> f32 {
        let fy = (level as i32 - lo) as f32 / span;
        plot.bottom() - fy.clamp(0.0, 1.0) * plot.h
    };

    // 底轴：比网格线略深一点，给图表一条"地面"
    p.line(
        plot.x,
        plot.bottom(),
        plot.right(),
        plot.bottom(),
        with_alpha(t.text, 0x1E),
        1.0,
    );
    // 4 条水平网格 + 左侧刻度（顶格带 % 号，其余纯数字，少点视觉噪音）
    for i in 1..=4 {
        let frac = i as f32 / 4.0;
        let gy = plot.bottom() - frac * plot.h;
        p.hline(plot.x, plot.right(), gy, with_alpha(t.text, 0x0D));
        let level = lo + (span * frac) as i32;
        // 四条刻度都带 %：只给顶格加会让 92 看起来像个编号。
        let label = format!("{level}%");
        p.text(
            &label,
            R::new(card.x + 2.0, gy - 8.0, 38.0, 16.0),
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
            let n = pl.points.len();
            let pts: Vec<(f32, f32)> = pl
                .points
                .iter()
                .map(|pt| (map_x(pt.x), map_y(pt.level)))
                .collect();

            // 充电时段：整列淡绿底带。比在折线上换颜色更直观 ——
            // 一眼就能看出"这段在充、那段在放"。
            for w in pl.points.windows(2) {
                if w[0].charging && w[1].charging {
                    let x0 = map_x(w[0].x);
                    let x1 = map_x(w[1].x);
                    if x1 - x0 > 0.5 {
                        p.rect(
                            R::new(x0, plot.y, x1 - x0, plot.h),
                            with_alpha(t.green, 0x12),
                        );
                    }
                }
            }

            // 电量是 1% 量化的，原始折线是一格一格的锯齿。先做一次窗口 5 的
            // 移动平均把台阶磨圆（末点保持原值，免得和右下角那个标记对不上），
            // 再用 Catmull-Rom 插值：y 用样条、x 保持线性 —— 采样点疏密不均时
            // 对 x 也插值会让曲线打结甚至回折。
            let soft = soften(&pts, 5);
            let curve = smooth_curve(&soft, 10);

            // 面积：曲线 + 首尾各拉到底边。起点用曲线自己的 x，不要用 plot.x ——
            // 第一个采样点可能晚于窗口起点（休眠、刚装完），从 plot.x 硬拉
            // 会多出一块三角形空填充，看着像画错了。
            if curve.len() >= 2 {
                let x0 = curve.first().map(|p| p.0).unwrap_or(plot.x);
                let x1 = curve.last().map(|p| p.0).unwrap_or(plot.right());
                let mut area: Vec<(f32, f32)> = Vec::with_capacity(curve.len() + 2);
                area.push((x0, plot.bottom()));
                area.extend(curve.iter().copied());
                area.push((x1, plot.bottom()));
                p.fill_polygon(&area, with_alpha(t.accent, 0x4A), with_alpha(t.accent, 0x00));
            }
            // 整条曲线一次描边。逐段 `polyline` 在接缝处会叠出「串珠」感。
            p.stroke_path(&curve, t.accent, 2.0);

            // 数据点：点多了就只标最新那个，否则满屏小圈
            if n <= 14 {
                for (i, (x, y)) in pts.iter().enumerate() {
                    if i + 1 == n {
                        continue;
                    }
                    p.dot(*x, *y, 3.0, t.card);
                    p.dot_ring(*x, *y, 3.0, with_alpha(t.accent, 0xD8), 1.8);
                }
            }
            if let Some((x, y)) = pts.last() {
                // 末点常正好落在绘图区右边界上，圆环会被裁掉一半 → 往里收 5px
                let mx = x.clamp(plot.x + 5.0, plot.right() - 5.0);
                p.dot(mx, *y, 9.0, with_alpha(t.accent, 0x28));
                p.dot(mx, *y, 5.0, t.card);
                p.dot_ring(mx, *y, 4.6, t.accent, 2.6);
            }

            // X 轴：起点 / 中点 / 终点三个时间，比只有首尾更容易定位。
            // 每处配一条极淡的竖线，把时间刻度和曲线对上。
            let ty = plot.bottom() + 6.0;
            for gx in [plot.x, plot.cx(), plot.right() - 0.5] {
                p.line(gx, plot.y, gx, plot.bottom(), with_alpha(t.text, 0x08), 1.0);
            }
            let fmt = crate::settings::format_unix_local;
            if let Some(first) = fmt(pl.first_ts) {
                p.text(
                    &first,
                    R::new(plot.x, ty, 84.0, 14.0),
                    font::TINY,
                    false,
                    t.text3,
                    Align::Left,
                );
            }
            if let Some(last) = fmt(pl.last_ts) {
                p.text(
                    &last,
                    R::new(plot.right() - 84.0, ty, 84.0, 14.0),
                    font::TINY,
                    false,
                    t.text3,
                    Align::Right,
                );
            }
            if pl.last_ts > pl.first_ts + 600 {
                if let Some(mid) = fmt((pl.first_ts + pl.last_ts) / 2) {
                    p.text(
                        &mid,
                        R::new(plot.cx() - 42.0, ty, 84.0, 14.0),
                        font::TINY,
                        false,
                        with_alpha(t.text3, 0xB0),
                        Align::Center,
                    );
                }
            }
        }
    }
}

/// 窗口移动平均：把 1% 量化的台阶磨成缓坡。
///
/// 首尾各留一半窗口不参与（用 saturating 边界），并且强制保留末点原值 ——
/// 末点要跟「最新值」那个圆点标记重合，被平均掉就错位了。
fn soften(pts: &[(f32, f32)], win: usize) -> Vec<(f32, f32)> {
    if win < 3 || pts.len() < win {
        return pts.to_vec();
    }
    let h = win / 2;
    let n = pts.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let lo = i.saturating_sub(h);
        let hi = (i + h).min(n - 1);
        let mut sum = 0.0f32;
        let mut cnt = 0.0f32;
        for j in lo..=hi {
            sum += pts[j].1;
            cnt += 1.0;
        }
        out.push((pts[i].0, sum / cnt));
    }
    out[n - 1] = pts[n - 1];
    out
}

/// Catmull-Rom 平滑：只对 y 插值，x 线性。
///
/// 采样点按时间递增，x 间隔可能很不均匀（比如休眠期间两小时才一个点）。
/// 如果对 x 也做样条，曲线会在密集处打结、在稀疏处回折，所以 x 一律线性，
/// y 用样条并把结果夹在数据自身的极值内，避免过冲跑出绘图区。
fn smooth_curve(pts: &[(f32, f32)], seg: usize) -> Vec<(f32, f32)> {
    if pts.len() < 3 || seg < 2 {
        return pts.to_vec();
    }
    let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
    for (_, y) in pts {
        min_y = min_y.min(*y);
        max_y = max_y.max(*y);
    }
    let n = pts.len();
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(n * seg + 1);
    for i in 0..n - 1 {
        let p0 = pts[i.saturating_sub(1)];
        let p1 = pts[i];
        let p2 = pts[i + 1];
        let p3 = pts[(i + 2).min(n - 1)];
        for s in 0..seg {
            let t = s as f32 / seg as f32;
            let t2 = t * t;
            let t3 = t2 * t;
            let y = 0.5
                * (2.0 * p1.1
                    + (p2.1 - p0.1) * t
                    + (2.0 * p0.1 - 5.0 * p1.1 + 4.0 * p2.1 - p3.1) * t2
                    + (3.0 * p1.1 - p0.1 - 3.0 * p2.1 + p3.1) * t3);
            out.push((p1.0 + (p2.0 - p1.0) * t, y.clamp(min_y, max_y)));
        }
    }
    out.push(*pts.last().unwrap());
    out
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
        // 颜色小方块（点它开色盘）+ 颜色框（精确输入）
        let ce = Edit::DpiColor(i as u8);
        let rgb = st.map(|s| s.rgb[i]);
        let sw = R::new(box_r.right() + 46.0, row.cy() - 15.0, 30.0, 30.0);
        color_button(p, ui, sw, i as u8, rgb);
        let ctxt = ui.edit_text(ce).map(|s| s.to_string()).unwrap_or_else(|| {
            rgb.map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
                .unwrap_or_else(|| "—".into())
        });
        let cbox = R::new(sw.right() + 8.0, row.cy() - 15.0, 84.0, 30.0);
        num_box(p, cbox, &ctxt, ui.focus == Some(ce));
        ui.hot(cbox, Act::Focus(ce));

        // 右侧：当前标记 / 切换与应用按钮
        if is_cur {
            p.text(
                "● 当前使用",
                R::new(row.right() - 146.0, row.y, 76.0, row.h),
                11.0,
                true,
                t.accent,
                Align::Left,
            );
        } else {
            let switch_btn = R::new(row.right() - 124.0, row.cy() - 15.0, 54.0, 30.0);
            button(p, ui, switch_btn, "切换", false, Act::PickDrop(Drop::Stage, i));
        }
        let apply_btn = R::new(row.right() - 62.0, row.cy() - 15.0, 52.0, 30.0);
        button(p, ui, apply_btn, "应用", false, Act::ApplyEdit(edit));

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
        let ctl = row_label(p, area, "当前档位", Some("同时决定鼠标的 DPI 和档位指示灯"), 220.0);
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
    let card = c.card(p, layout::ROW_H);
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
    c.y = card.bottom() + 10.0;

    let nw = note(
        p,
        R::new(c.x, c.y, c.w, 20.0),
        "写 LOD 会重置鼠标的表面标定（厂商确认没有恢复命令）。本机标定已被重置过。",
        true,
    );
    c.y += nw;

    let _ = f;
    let _ = t;
}

// ───────────────────────── 灯光页 ─────────────────────────

/// 灯光页 —— 方案 A：交给 Windows 自带的「动态光效」。
///
/// 实测这只鼠标暴露的是标准 **HID LampArray**（Usage Page 0x59，Feature Report
/// 51 字节，1 个灯珠，Kind=Mouse），而 Windows 动态光效认的就是这个协议。
/// 所以这里不自己写灯效，只负责：显示设备在不在、一键跳到系统设置页。
///
/// 详见 `src/dynlight.rs` 顶部。
fn light(p: &mut Painter, ui: &mut Ui, _d: &Data, _f: &Frame, c: &mut Cursor) {
    let t = p.t;
    c.h1(p, "灯光");

    let lamp = crate::dynlight::info();

    // ---- 控制方式 ----
    c.h2(p, "控制方式", true);
    let card = c.card(p, layout::ROW_H + 8.0);
    {
        let area = R::new(card.x + 18.0, card.y + 4.0, card.w - 36.0, layout::ROW_H);
        p.circle(area.x + 5.0, area.cy(), 4.5, t.green);
        p.text(
            "由 Windows 动态光效控制",
            R::new(area.x + 20.0, area.y, (area.w - 200.0).max(80.0), area.h),
            font::BODY,
            false,
            t.text,
            Align::Left,
        );
        let btn = R::new(area.right() - 160.0, area.cy() - 15.0, 160.0, 30.0);
        button(p, ui, btn, "打开动态光效设置", true, Act::OpenLightingSettings);
    }
    c.y = card.bottom();
    c.y += 12.0;
    p.text_wrap(
        "灯效在「设置 → 个性化 → 动态光效」里调：纯色 / 呼吸 / 渐变 / 波浪 / 闪烁等都由 Windows 提供。本程序不再写厂商私有灯效报文，避免和系统互相覆盖导致闪烁。",
        R::new(c.x, c.y, c.w, 44.0),
        font::HINT,
        t.text3,
    );
    c.y += 52.0;

    // ---- 灯效设备 ----
    c.h2(p, "灯效设备", false);
    let extra = if lamp.is_some() { layout::ROW_H } else { 0.0 };
    let card = c.card(p, layout::ROW_H + extra);
    {
        let area = R::new(card.x + 18.0, card.y, card.w - 36.0, layout::ROW_H);
        let found = lamp.is_some();
        p.circle(
            area.x + 5.0,
            area.cy(),
            4.5,
            if found { t.green } else { t.text3 },
        );
        p.text(
            if found {
                "已检测到 HID LampArray 设备"
            } else {
                "未检测到 HID LampArray 设备"
            },
            R::new(area.x + 20.0, area.y, (area.w - 130.0).max(80.0), area.h),
            font::BODY,
            false,
            if found { t.text } else { t.text2 },
            Align::Left,
        );
        let btn = R::new(area.right() - 96.0, area.cy() - 15.0, 96.0, 30.0);
        button(p, ui, btn, "重新检测", false, Act::RefreshLighting);
    }
    if let Some(l) = lamp {
        let area = R::new(
            card.x + 18.0,
            card.y + layout::ROW_H,
            card.w - 36.0,
            layout::ROW_H,
        );
        p.hline(card.x, card.right(), area.y, t.border);
        let ctl = row_label(p, area, "灯珠数量 / 类型", None, 240.0);
        p.text(
            &format!("{} 个 · {}", l.lamp_count, l.kind_name()),
            R::new(ctl.x, ctl.y, ctl.w, ctl.h),
            font::BODY,
            false,
            t.text2,
            Align::Right,
        );
    }
    c.y = card.bottom();
    c.y += 14.0;

    // ---- 提示 ----
    let nw = note(
        p,
        R::new(c.x, c.y, c.w, 20.0),
        "灯效不跟系统走时：Armoury Crate 的灯效可能正在抢占设备，先关掉它的灯效（或退出），再回到系统「动态光效」页面。",
        false,
    );
    c.y += nw + 16.0;
}

/// 【已停用】旧的厂商私有协议灯效界面。
///
/// 保留是因为 CLI 的 `setlight` 仍然走 `ops::set_light`，这里是它的可视化版本；
/// 但实测灯根本不在 0xFF01 集合上（见 `dynlight.rs` 顶部），点这些控件不会有效果，
/// 所以 UI 不再给入口。留着是为了以后要回退时不用重新写一遍。
#[allow(dead_code)]
fn light_vendor(p: &mut Painter, ui: &mut Ui, d: &Data, _f: &Frame, c: &mut Cursor) {
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
        color_button(p, ui, sw, crate::ui::widget::LIGHT_COLOR_SLOT, rgb);
        let edit = Edit::LightColor;
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

    // 顶部警告提示条（用户反馈置顶更醒目）
    c.y += 4.0;
    let nw = note(
        p,
        R::new(c.x, c.y, c.w, 20.0),
        "把某个键绑成「无功能」（0000）后，该键在鼠标上将完全失效 —— 如需恢复必须重新写入完整映射配置。",
        true,
    );
    c.y += nw + 16.0;

    c.h2(p, "按键映射", true);
    let rows = crate::proto::BUTTON_SLOTS;
    let card = c.card(p, rows as f32 * layout::ROW_H);
    for slot in 0..rows {
        let y = card.y + slot as f32 * layout::ROW_H;
        let area = R::new(card.x + 18.0, y, card.w - 36.0, layout::ROW_H);
        let src = crate::proto::BUTTON_SRC[slot];
        let name = crate::proto::BUTTON_NAME[slot];

        // 左：源键名（统一宽 96px）
        p.text(
            name,
            R::new(area.x, area.cy() - 10.0, 96.0, 20.0),
            13.5,
            false,
            t.text,
            Align::Left,
        );

        // 中：十六进制原码（现代代码胶囊微标，垂直与水平居中对齐）
        let tag_r = R::new(area.x + 102.0, area.cy() - 9.0, 48.0, 18.0);
        p.round(tag_r, 4.0, t.bg_sunken);
        p.round_stroke(tag_r, 4.0, t.border, 1.0);
        p.text(
            &format!("{src:04X}"),
            tag_r,
            10.5,
            false,
            t.text3,
            Align::Center,
        );

        // 右：当前绑定下拉框
        let dst = R::new(area.right() - 160.0, area.cy() - 15.0, 160.0, 30.0);
        drop_button(p, ui, dst, Drop::ButtonTarget(slot as u8), d);
        if slot + 1 < rows {
            p.hline(card.x, card.right(), area.bottom(), t.border);
        }
    }
    c.y = card.bottom() + 16.0;

    // 底部操作按钮
    let bw = 160.0;
    let br = R::new(c.x, c.y, bw, 34.0);
    button(p, ui, br, "从备份恢复全部按键", false, Act::Restore);
    let br2 = R::new(br.right() + 12.0, c.y, bw, 34.0);
    button(p, ui, br2, "导出当前配置", false, Act::Backup);
    c.y += 34.0;
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
    // 关闭窗口动作
    {
        let area = R::new(card.x + 18.0, y, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(
            p,
            area,
            "关闭窗口动作",
            Some("点击窗口右上角 × 按钮时的操作"),
            160.0,
        );
        drop_button(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, 160.0, 30.0),
            Drop::CloseMode,
            d,
        );
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

    // ---- 存储路径 ----
    c.h2(p, "存储路径", false);
    let card = c.card(p, layout::ROW_H * 2.0);
    {
        // 历史文件
        let area = R::new(card.x + 18.0, card.y, card.w - 36.0, layout::ROW_H);
        let hp = crate::history::history_path().display().to_string();
        let status = if crate::history::history_path_is_custom() { "已自定义" } else { "默认" };
        let title = format!("历史文件保存位置（{status}）");
        let ctl = row_label(p, area, &title, Some(&hp), 128.0);
        let b1 = R::new(ctl.right() - 52.0, ctl.cy() - 15.0, 52.0, 30.0);
        button(p, ui, b1, "更改", false, Act::ChangeHistoryPath);
        let b2 = R::new(b1.x - 64.0, ctl.cy() - 15.0, 56.0, 30.0);
        button(p, ui, b2, "默认", false, Act::ResetHistoryPath);
        p.hline(card.x, card.right(), area.bottom(), t.border);
    }
    {
        // 设置文件
        let area = R::new(card.x + 18.0, card.y + layout::ROW_H, card.w - 36.0, layout::ROW_H);
        let sp = crate::paths::settings_file().display().to_string();
        let status = if crate::paths::custom_settings_file().is_some() { "已自定义" } else { "默认" };
        let title = format!("设置文件保存位置（{status}）");
        let ctl = row_label(p, area, &title, Some(&sp), 128.0);
        let b1 = R::new(ctl.right() - 52.0, ctl.cy() - 15.0, 52.0, 30.0);
        button(p, ui, b1, "更改", false, Act::ChangeSettingsPath);
        let b2 = R::new(b1.x - 64.0, ctl.cy() - 15.0, 56.0, 30.0);
        button(p, ui, b2, "默认", false, Act::ResetSettingsPath);
    }
    c.y = card.bottom();

    // ---- 外观 ----
    c.h2(p, "外观", false);
    let card = c.card(p, layout::ROW_H);
    {
        let area = R::new(card.x + 18.0, card.y, card.w - 36.0, layout::ROW_H);
        let ctl = row_label(p, area, "配色", Some("跟随系统或手动选择"), 210.0);
        let sel = ui.theme_pref.min(2) as usize;
        segmented(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, 210.0, 30.0),
            &["跟随系统", "浅色", "深色"],
            sel,
            |i| Act::SetTheme(i as u8),
        );
    }
    c.y = card.bottom();

    // ---- 关于 ----
    c.h2(p, "关于", false);
    let card = c.card(p, layout::ROW_H * 3.0);
    {
        // 版本：值右对齐，版本号来自 Cargo.toml（不再手写常量）
        let area = R::new(card.x + 18.0, card.y, card.w - 36.0, layout::ROW_H);
        row_label(
            p,
            area,
            "版本",
            Some("R Battery · 不依赖 ASUS 驱动的 ROG 鼠标参数工具"),
            96.0,
        );
        p.text(
            &format!("v{}", d.version),
            R::new(area.right() - 140.0, area.y, 140.0, area.h),
            12.5,
            true,
            t.text,
            Align::Right,
        );
        p.hline(card.x, card.right(), area.bottom(), t.border);
    }
    {
        // 开源仓库
        let area = R::new(
            card.x + 18.0,
            card.y + layout::ROW_H,
            card.w - 36.0,
            layout::ROW_H,
        );
        let ctl = row_label(p, area, "开源仓库", Some(REPO_URL), 64.0);
        button(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, 64.0, 30.0),
            "打开",
            false,
            Act::OpenRepo,
        );
        p.hline(card.x, card.right(), area.bottom(), t.border);
    }
    {
        // 商店页
        let area = R::new(
            card.x + 18.0,
            card.y + layout::ROW_H * 2.0,
            card.w - 36.0,
            layout::ROW_H,
        );
        let ctl = row_label(p, area, "Microsoft Store", Some(STORE_URL), 64.0);
        button(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, 64.0, 30.0),
            "打开",
            false,
            Act::OpenStore,
        );
    }
    c.y = card.bottom();

    let _ = Breakpoint::Wide;
}

/// 绘制「关闭窗口」确认模态对话框。
pub fn draw_close_dialog(p: &mut Painter, ui: &mut Ui, win_w: f32, win_h: f32) {
    let t = p.t;
    // 半透明遮罩蒙层
    p.rect(R::new(0.0, 0.0, win_w, win_h), 0x7500_0000);

    // 居中卡片
    let w = 420.0;
    let h = 240.0;
    let r = R::new((win_w - w) / 2.0, (win_h - h) / 2.0, w, h);
    p.card(r, 12.0, t.card, t.border);

    // 标题与副标题
    p.text(
        "关闭应用",
        R::new(r.x + 24.0, r.y + 20.0, w - 48.0, 24.0),
        16.0,
        true,
        t.text,
        Align::Left,
    );
    p.text(
        "请选择点击关闭按钮时的操作：",
        R::new(r.x + 24.0, r.y + 48.0, w - 48.0, 20.0),
        12.5,
        false,
        t.text2,
        Align::Left,
    );

    // 单选项 1：最小化到系统托盘
    let opt1 = R::new(r.x + 24.0, r.y + 78.0, w - 48.0, 32.0);
    let sel1 = ui.close_choice == crate::ui::widget::CloseChoice::Minimize;
    p.circle_stroke(opt1.x + 10.0, opt1.cy(), 7.0, if sel1 { t.accent } else { t.border }, 1.5);
    if sel1 {
        p.circle(opt1.x + 10.0, opt1.cy(), 4.0, t.accent);
    }
    p.text(
        "最小化到系统托盘（在后台继续运行）",
        R::new(opt1.x + 26.0, opt1.y + 4.0, opt1.w - 26.0, 24.0),
        13.0,
        false,
        t.text,
        Align::Left,
    );
    ui.dialog_hot(opt1, Act::SelectCloseChoice(crate::ui::widget::CloseChoice::Minimize));

    // 单选项 2：直接退出程序
    let opt2 = R::new(r.x + 24.0, r.y + 114.0, w - 48.0, 32.0);
    let sel2 = ui.close_choice == crate::ui::widget::CloseChoice::Quit;
    p.circle_stroke(opt2.x + 10.0, opt2.cy(), 7.0, if sel2 { t.accent } else { t.border }, 1.5);
    if sel2 {
        p.circle(opt2.x + 10.0, opt2.cy(), 4.0, t.accent);
    }
    p.text(
        "直接退出程序",
        R::new(opt2.x + 26.0, opt2.y + 4.0, opt2.w - 26.0, 24.0),
        13.0,
        false,
        t.text,
        Align::Left,
    );
    ui.dialog_hot(opt2, Act::SelectCloseChoice(crate::ui::widget::CloseChoice::Quit));

    // 复选框：记住我的选择
    let cb_r = R::new(r.x + 24.0, r.y + 154.0, w - 48.0, 24.0);
    p.checkbox(R::new(cb_r.x + 2.0, cb_r.cy() - 8.0, 16.0, 16.0), ui.close_remember, true);
    p.text(
        "记住我的选择，下次不再提示",
        R::new(cb_r.x + 26.0, cb_r.y, cb_r.w - 26.0, 24.0),
        12.5,
        false,
        t.text2,
        Align::Left,
    );
    ui.dialog_hot(cb_r, Act::ToggleCloseRemember);

    // 底部按钮栏：确定 / 取消
    let btn_w = 84.0;
    let btn_h = 32.0;
    let btn_y = r.bottom() - 44.0;
    let confirm_r = R::new(r.right() - 24.0 - btn_w, btn_y, btn_w, btn_h);
    let cancel_r = R::new(confirm_r.x - 12.0 - btn_w, btn_y, btn_w, btn_h);

    // 取消按钮
    p.round(cancel_r, 6.0, t.card);
    p.round_stroke(cancel_r, 6.0, t.border, 1.0);
    p.text("取消", cancel_r, 13.0, false, t.text, Align::Center);
    ui.dialog_hot(cancel_r, Act::CancelCloseDialog);

    // 确定按钮
    p.round(confirm_r, 6.0, t.accent);
    p.text("确定", confirm_r, 13.0, true, 0xFFFF_FFFF, Align::Center);
    ui.dialog_hot(confirm_r, Act::ConfirmCloseDialog);
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
            link_mode: "",
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
            link_mode: "",
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

    #[test]
    fn 色盘自定义rgb打包与解包() {
        let rgb = [0x12, 0x34, 0x56];
        let packed = pack_custom_rgb(rgb);
        assert_eq!(palette_color(packed), Some(rgb));

        // 预设下标应正常返回
        assert!(palette_color(0).is_some());
        assert!(palette_color(35).is_some());
        assert_eq!(palette_color(100), None);
    }

    #[test]
    fn 色相计算基本性质() {
        let (h_red, s_red, v_red) = rgb_to_hsv([255, 0, 0]);
        assert!((h_red - 0.0).abs() < 1e-3);
        assert!((s_red - 1.0).abs() < 1e-3);
        assert!((v_red - 1.0).abs() < 1e-3);

        let (h_green, _, _) = rgb_to_hsv([0, 255, 0]);
        assert!((h_green - 120.0).abs() < 1e-3);

        let (h_blue, _, _) = rgb_to_hsv([0, 0, 255]);
        assert!((h_blue - 240.0).abs() < 1e-3);
    }

    #[test]
    fn 色盘尺寸随标签变化() {
        let (w0, h0) = palette_size(0);
        let (w1, h1) = palette_size(1);
        assert_eq!(w0, w1);
        assert!(h1 > h0, "色环模式需要更高的高度容纳色盘与色阶");
    }

    #[test]
    fn 弹层靠近视口底部时向上翻开() {
        let mut ui = Ui::default();

        // 去抖下拉框在客户区 y≈704..734，视口 740 高、8 项弹层要 248px：
        // 往下开必然戳出窗外，必须翻到按钮上方，而且整块要留在视口内。
        ui.set_drop_btn(R::new(865.0, 704.0, 150.0, 30.0));
        let r = place_drop(&ui, R::new(865.0, 0.0, 150.0, 0.0), 248.0, 740.0);
        assert!(r.y < 704.0, "应翻到按钮上方: {r:?}");
        assert!(r.bottom() <= 732.0 + 0.01, "弹层不该戳出视口: {r:?}");
        assert!(r.y >= 8.0, "也不该顶到视口上沿之外: {r:?}");

        // 上方空间充裕时默认仍然往下开
        ui.set_drop_btn(R::new(865.0, 100.0, 150.0, 30.0));
        let r2 = place_drop(&ui, R::new(865.0, 0.0, 150.0, 0.0), 248.0, 740.0);
        assert_eq!(r2.y, 134.0, "空间够就往下开");
    }
}
