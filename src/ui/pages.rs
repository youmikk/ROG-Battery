
use crate::ui::dropdown::{drop_current, drop_selected, options};
use crate::ui::gfx::{with_alpha, Align, Painter, R};
use crate::ui::layout::{self, font, Breakpoint, Frame};
use crate::ui::palette::{
    hsv_to_rgb, pack_custom_rgb, palette_colors, palette_index, rgb_to_hsv, PAL_CELL, PAL_COLS,
    PAL_GAP,
};
use crate::ui::theme::Theme;
use crate::ui::widget::{Act, Drop, Edit, Icon, Page, Pref, Slide, Ui};

/// 设置页「关于」里的开源仓库地址。
const REPO_URL: &str = "https://github.com/youmikk/ROG-Battery";
/// 设置页「关于」里的商店产品页地址。
const STORE_URL: &str = "https://apps.microsoft.com/detail/9NRC8HCD4BZ7";

/// 页面要显示的数据。由 `gui.rs` 从 `Shared` 里凑出来。
pub use crate::ui::viewdata::Data;

// ───────────────────────── 通用小工具 ─────────────────────────

fn link_text(d: &Data) -> &'static str {
    match d.link_mode {
        "Wired" => crate::tr!("有线"),
        "Wireless24G" => crate::tr!("2.4G 无线"),
        _ => {
            let has_dongle = d
                .ident
                .map(|i| !i.dongle_sn.trim().is_empty())
                .unwrap_or(false);
            if has_dongle {
                crate::tr!("2.4G 无线")
            } else {
                crate::tr!("有线")
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

/// 卡片内一行设置项的绘制区：左右各留 18px，高度固定为一行。
///
/// 这个式子在本文件里手写过 20 多次，抽样一改就得挨个改；漏一处就是那行
/// 比其他行宽 4px，肉眼几乎看不出来，但会顶到卡片边。
fn row_area(card: R, y: f32) -> R {
    R::new(card.x + 18.0, y, card.w - 36.0, layout::ROW_H)
}

/// 行与行之间的分隔线：铺到卡片左右内边界，画在这一行的底部。
///
/// 这个式子手写过 14 次，且每处都要记得取 `t.border`；漏一处就是那一段
/// 没有分隔线，看起来像是两行并成了一行。
fn row_rule(p: &mut Painter, card: R, area: R) {
    let t = p.t;
    p.hline(card.x, card.right(), area.bottom(), t.border);
}

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

// ───────────────────────── 色盘 ─────────────────────────

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
pub(crate) fn draw_palette(
    p: &mut Painter,
    ui: &mut Ui,
    slot: u8,
    anchor: R,
    cur: Option<[u8; 3]>,
    f: &Frame,
) {
    let (w, h) = palette_size(ui.palette_tab);
    let scroll = ui.scroll_y(ui.page);
    let view_h = f.viewport.bottom();
    let x = anchor
        .x
        .min((f.content.right() - w).max(f.content.x))
        .max(f.content.x);
    let y = anchor
        .y
        .min((scroll + view_h - h - 8.0).max(scroll + f.viewport.y + 8.0))
        .max(scroll + f.viewport.y + 8.0);
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
        let label = if i == 0 {
            crate::tr!("常用预设")
        } else {
            crate::tr!("自定义色环")
        };
        if ui.palette_tab == i {
            // 选中项：一枚投影 + accent 药丸，和页面里的分段控件一致
            p.round(
                R::new(tab.x, tab.y + 1.0, tab.w, tab.h),
                7.0,
                with_alpha(0x0000_0000, 0x1C),
            );
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
        p.text(
            crate::tr!("选择深浅（点击直接应用）"),
            R::new(r.x + 8.0, sy, 220.0, 14.0),
            11.0,
            false,
            p.t.text3,
            Align::Left,
        );

        let shades = [
            [255, 255, 255],                   // 0: 纯白
            hsv_to_rgb(active_hue, 0.25, 1.0), // 1: 极浅
            hsv_to_rgb(active_hue, 0.50, 1.0), // 2: 柔和
            hsv_to_rgb(active_hue, 0.75, 1.0), // 3: 靓丽
            hsv_to_rgb(active_hue, 1.0, 1.0),  // 4: 饱和纯色
            hsv_to_rgb(active_hue, 1.0, 0.80), // 5: 浓郁
            hsv_to_rgb(active_hue, 1.0, 0.60), // 6: 深暗
            hsv_to_rgb(active_hue, 1.0, 0.35), // 7: 极深
            [0, 0, 0],                         // 8: 纯黑
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
        button(
            p,
            ui,
            btn_r,
            &crate::trf!("应用当前纯色 ({hex_now})", hex_now = hex_now),
            true,
            Act::PickDrop(drop, active_packed),
        );
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

// ───────────────────────── 导航 ─────────────────────────

/// 导航用的图标。用线段画，不加载图片 —— 一个 16×16 的图标走 GDI+
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
    }
}

fn page_heading(p: &mut Painter, ui: &mut Ui, c: &mut Cursor, title: &str, back: Option<Act>) {
    let top = c.y + 8.0;
    let mut x = c.x;
    if let Some(act) = back {
        let r = R::new(x, top + (layout::H1_H - 32.0) / 2.0, 32.0, 32.0);
        if ui.hover == Some(act) {
            p.round(r, 7.0, p.t.accent_soft);
        }
        let cx = r.cx();
        let cy = r.cy();
        p.polyline(
            &[(cx + 2.0, cy - 5.0), (cx - 3.0, cy), (cx + 2.0, cy + 5.0)],
            p.t.text2,
            1.6,
        );
        p.line(cx - 3.0, cy, cx + 6.0, cy, p.t.text2, 1.6);
        ui.hot(r, act);
        x += 42.0;
    }
    p.text(
        title,
        R::new(x, top, c.x + c.w - x, layout::H1_H),
        font::H1,
        true,
        p.t.text,
        Align::Left,
    );
    c.y = top + layout::H1_H + 12.0;
}

fn device_header(p: &mut Painter, ui: &mut Ui, d: &Data, c: &mut Cursor) {
    let t = p.t;
    let name = d
        .selected
        .map(|r| d.cfg.device_name(r))
        .unwrap_or(crate::tr!("设备详情"));
    let back = (d.readings.len() > 1).then_some(Act::Nav(Page::Home));
    page_heading(p, ui, c, name, back);
    let tabs = [
        (Page::Battery, crate::tr!("电量与设置")),
        (Page::Perf, crate::tr!("性能")),
        (Page::Light, crate::tr!("灯光")),
        (Page::Keys, crate::tr!("按键")),
    ];
    let is_mouse = d
        .selected
        .is_some_and(|r| d.cfg.device_kind(r) == crate::tray_prefs::TrayKind::Mouse);
    let mut x = c.x;
    for (page, label) in tabs {
        if page != Page::Battery && !is_mouse {
            continue;
        }
        let r = R::new(
            x,
            c.y,
            (p.measure(label, 12.5, true).0 + 38.0).max(72.0),
            33.0,
        );
        let on = ui.page == page;
        let glyph = match page {
            Page::Battery => Icon::Battery,
            Page::Perf => Icon::Perf,
            Page::Light => Icon::Light,
            _ => Icon::Keys,
        };
        draw_icon(
            p,
            glyph,
            R::new(r.x + 5.0, r.cy() - 7.0, 14.0, 14.0),
            if on { t.accent } else { t.text2 },
            1.3,
        );
        p.text(
            label,
            R::new(r.x + 21.0, r.y, r.w - 21.0, r.h),
            12.5,
            on,
            if on { t.accent } else { t.text2 },
            Align::Center,
        );
        if on {
            p.round(
                R::new(r.x + 8.0, r.bottom() - 2.0, r.w - 16.0, 2.0),
                1.0,
                t.accent,
            );
        }
        ui.hot(r, Act::Nav(page));
        x += r.w + 8.0;
    }
    p.hline(c.x, c.x + c.w, c.y + 34.0, t.border);
    c.y += 55.0;
}

fn device_tray_settings(p: &mut Painter, ui: &mut Ui, d: &Data, c: &mut Cursor) {
    let Some(device) = d.selected else { return };
    let pref = d.cfg.tray_for(&device.key);
    let selected = d.cfg.tray_device.as_ref() == Some(&device.key);
    c.h2(p, crate::tr!("设备偏好"), false);
    let card = c.card(p, layout::ROW_H * 3.0);
    let area = row_area(card, card.y);
    let ctl = row_label(
        p,
        area,
        crate::tr!("托盘显示设备"),
        Some(crate::tr!("固定显示这台设备的电量")),
        150.0,
    );
    button(
        p,
        ui,
        R::new(ctl.x, ctl.cy() - 15.0, 150.0, 30.0),
        if selected {
            crate::tr!("已在托盘显示")
        } else {
            crate::tr!("设为托盘显示设备")
        },
        false,
        if selected {
            Act::None
        } else {
            Act::ToggleDeviceTray
        },
    );
    row_rule(p, card, area);
    let area = row_area(card, card.y + layout::ROW_H);
    let ctl = row_label(
        p,
        area,
        crate::tr!("设备名称"),
        Some(d.cfg.device_name(device)),
        150.0,
    );
    button(
        p,
        ui,
        R::new(ctl.x, ctl.cy() - 15.0, 150.0, 30.0),
        crate::tr!("编辑名称"),
        false,
        Act::RenameDevice,
    );
    row_rule(p, card, area);
    let area = row_area(card, card.y + layout::ROW_H * 2.0);
    let help = if pref.kind == crate::tray_prefs::TrayKind::Auto {
        crate::trf!(
            "自动识别为{}，可手动纠正",
            d.cfg.device_kind(device).label()
        )
    } else {
        crate::tr!("手动选择优先于自动识别").to_string()
    };
    let ctl = row_label(p, area, crate::tr!("设备类型"), Some(&help), 150.0);
    drop_button(
        p,
        ui,
        R::new(ctl.x, ctl.cy() - 15.0, 150.0, 30.0),
        Drop::TrayKind,
        d,
    );
    c.y = card.bottom();
}

// ───────────────────────── 入口 ─────────────────────────

/// 画当前页正文（内容坐标，滚动由 canvas 施加）。
pub fn page(p: &mut Painter, ui: &mut Ui, d: &Data, f: &Frame) {
    if matches!(ui.page, Page::Perf | Page::Light | Page::Keys)
        && d.selected
            .is_some_and(|r| d.cfg.device_kind(r) != crate::tray_prefs::TrayKind::Mouse)
    {
        ui.page = Page::Battery;
        ui.set_open_drop(None);
        ui.cancel_edit();
    }
    // 读不到设备时，只有**依赖设备状态**的页面才锁定：设置页、灯光页、电量页
    // 的内容要么是本地设置、要么是只读展示，锁了反而没法用。
    // 占位状态（`State::default()`）也算读不到 —— 否则开关/下拉照常可点。
    ui.locked = !d.state.is_some_and(|s| !placeholder_state(s));

    let mut c = Cursor::new(f);
    if !matches!(ui.page, Page::Home | Page::Settings) {
        device_header(p, ui, d, &mut c);
    }
    match ui.page {
        Page::Home => home(p, ui, d, f, &mut c),
        Page::Battery => battery(p, ui, d, f, &mut c),
        Page::Perf => perf(p, ui, d, &mut c),
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
            _ => draw_drop(p, ui, d, drop, f.viewport.bottom()),
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
    let view_h = f.viewport.bottom();

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

/// 画下拉框本体，并登记「打开弹层」。点击区域是整个矩形。
///
/// 必须在**绘制该行时**调用：它要拿到那一行的矩形，而弹层本身的矩形
/// 由 [`draw_drop`] 在最后登记（那时代码已经知道弹层该开在哪儿）。
fn drop_button(p: &mut Painter, ui: &mut Ui, r: R, drop: Drop, d: &Data) {
    let widest = options(drop, d)
        .iter()
        .map(|(label, _)| p.measure(label, 12.5, true).0 + 32.0)
        .fold(r.w, f32::max)
        .min(240.0);
    let width = if drop == Drop::TrayDevice {
        r.w
    } else {
        widest
    };
    let r = R::new(r.right() - width, r.y, width, r.h);
    let open = ui.open_drop() == Some(drop);
    select_box(p, r, &drop_current(drop, d), open);
    ui.hot(r, Act::OpenDrop(drop));
    // 记下这个下拉框的位置，弹层要贴着它开
    if open {
        let menu_w = widest.max(160.0);
        let menu_x = if drop == Drop::TrayDevice {
            r.right() - menu_w
        } else {
            r.x
        };
        ui.set_drop_rect(R::new(menu_x, r.bottom() + 4.0, menu_w, 0.0));
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
    let view_top = scroll + layout::CONTENT_TOP;
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
    if drop == Drop::TrayDevice {
        ui.tray_device_keys = d.readings.iter().map(|r| r.key.clone()).collect();
    }
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
    ui.set_drop_rect(r);
}

// ───────────────────────── 电量页 ─────────────────────────

/// 首页仅展示设备读数，点击进入该设备的详情。
fn home(p: &mut Painter, ui: &mut Ui, d: &Data, _f: &Frame, c: &mut Cursor) {
    let t = p.t;
    page_heading(p, ui, c, crate::tr!("设备"), None);
    p.text(
        crate::tr!("所有设备的电量，一眼就好。"),
        R::new(c.x, c.y - 14.0, c.w, 20.0),
        12.0,
        false,
        t.text2,
        Align::Left,
    );
    c.y += 25.0;
    if d.readings.is_empty() {
        p.text(
            crate::tr!("连接设备后会自动显示。"),
            R::new(c.x, c.y + 32.0, c.w, 30.0),
            14.0,
            false,
            t.text3,
            Align::Center,
        );
        c.y += 100.0;
        return;
    }
    let columns = if c.w < 420.0 { 1 } else { 2 };
    let gap = 18.0;
    let width = (c.w - gap * (columns - 1) as f32) / columns as f32;
    let height = 220.0;
    for (index, dev) in d.readings.iter().enumerate() {
        let r = R::new(
            c.x + (index % columns) as f32 * (width + gap),
            c.y + (index / columns) as f32 * (height + gap),
            width,
            height,
        );
        let act = Act::SelectDevice(index);
        p.card(
            r,
            14.0,
            t.card,
            if ui.hover == Some(act) {
                t.accent
            } else {
                t.border
            },
        );
        ui.hot(r, act);
        p.text(
            d.cfg.device_kind(dev).label(),
            R::new(r.x + 18.0, r.y + 12.0, 80.0, 18.0),
            10.5,
            false,
            t.text3,
            Align::Left,
        );
        let color = ring_color(t, dev.connected, dev.charging, dev.level);
        let cy = r.y + 83.0;
        p.ring(
            r.cx(),
            cy,
            48.0,
            5.0,
            dev.level as f32 / 100.0,
            color,
            color,
            t.track,
        );
        p.text(
            &format!("{}%", dev.level),
            R::new(r.cx() - 46.0, cy - 21.0, 92.0, 35.0),
            29.0,
            false,
            if dev.connected { t.text } else { t.text3 },
            Align::Center,
        );
        p.text(
            if !dev.connected {
                crate::tr!("上次电量")
            } else if dev.charging {
                crate::tr!("正在充电")
            } else if dev.level <= 20 {
                crate::tr!("电量偏低")
            } else {
                crate::tr!("使用中")
            },
            R::new(r.cx() - 45.0, cy + 16.0, 90.0, 16.0),
            10.0,
            false,
            t.text2,
            Align::Center,
        );
        p.text(
            d.cfg.device_name(dev),
            R::new(r.x + 16.0, r.y + 151.0, r.w - 32.0, 25.0),
            14.0,
            true,
            t.text,
            Align::Center,
        );
        p.text(
            if !dev.connected {
                crate::tr!("未连接")

            } else {
                crate::tr!("已连接")
            },
            R::new(r.x + 16.0, r.y + 181.0, r.w - 32.0, 18.0),
            11.0,
            false,
            t.text3,
            Align::Center,
        );
    }
    c.y += d.readings.len().div_ceil(columns) as f32 * (height + gap);
}

fn battery(p: &mut Painter, ui: &mut Ui, d: &Data, _f: &Frame, c: &mut Cursor) {
    let t = p.t;
    let selected = d.selected;
    let level = selected
        .map(|r| r.level)
        .or_else(|| d.state.map(|s| s.power))
        .unwrap_or(0);
    let connected = selected.map(|r| r.connected).unwrap_or(d.state.is_some());
    let charging = selected.map(|r| r.charging).unwrap_or(false);
    let hero = c.card(p, 175.0);
    let color = ring_color(t, connected, charging, level);
    let cx = hero.x + 84.0;
    let cy = hero.y + 87.0;
    p.ring(
        cx,
        cy,
        54.0,
        5.5,
        level as f32 / 100.0,
        color,
        color,
        t.track,
    );
    p.text(
        &format!("{level}%"),
        R::new(cx - 50.0, cy - 23.0, 100.0, 37.0),
        30.0,
        false,
        t.text,
        Align::Center,
    );
    let status = if !connected {
        crate::tr!("未连接")
    } else if charging {
        crate::tr!("正在充电")
    } else {
        crate::tr!("使用中")
    };
    p.text(
        status,
        R::new(cx - 45.0, cy + 19.0, 90.0, 17.0),
        11.0,
        false,
        t.text2,
        Align::Center,
    );
    let x = hero.x + 166.0;
    let remaining = if !connected {
        crate::tr!("等待设备连接").to_string()
    } else if charging {
        crate::tr!("正在为设备充电").to_string()
    } else if d.estimate.known {
        crate::trf!("预计还能使用 {}", d.estimate.localized_short_text())
    } else {
        crate::tr!("电量状态").to_string()
    };
    p.text(
        &remaining,
        R::new(x, hero.y + 42.0, hero.right() - x - 20.0, 28.0),
        18.0,
        false,
        t.text,
        Align::Left,
    );
    let link = if connected {
        link_text(d)
    } else {
        crate::tr!("连接后更新读数")
    };
    p.text(
        link,
        R::new(x, hero.y + 84.0, hero.right() - x - 20.0, 22.0),
        12.0,
        false,
        t.text2,
        Align::Left,
    );
    if let Some(st) = d.state {
        if selected.is_some_and(|r| d.cfg.device_kind(r) == crate::tray_prefs::TrayKind::Mouse) {
            p.text(
                &format!(
                    "{} DPI  ·  {}",
                    st.dpi(cur_stage(st)),
                    crate::proto::poll_name(st.poll_code)
                ),
                R::new(x, hero.y + 116.0, hero.right() - x - 20.0, 20.0),
                12.0,
                false,
                t.text2,
                Align::Left,
            );
        }
    }
    c.y = hero.bottom();
    device_tray_settings(p, ui, d, c);
    if selected
        .is_some_and(|r| d.cfg.device_kind(r) != crate::tray_prefs::TrayKind::Mouse)
    {
        return;
    }

    // ---- 电量历史 ----
    // ---- 设备设置 ----
    c.h2(p, crate::tr!("设备设置"), false);
    // 只管「标题 · 说明」，控件宽度由各行自己按控件类型定（下拉 150、
    // 输入框+按钮、开关 40）。曾经这里多一个宽度字段，三处调用全都无视它、
    // 直接写字面量，于是那个字段从没被读过，第三行的 60 还和实际用的 40 对不上。
    let st = known_state(d);
    let rows: [(&str, Option<&str>); 3] = [
        (
            crate::tr!("自动休眠"),
            Some(crate::tr!("设备静止多久后进入休眠以省电")),
        ),
        (
            crate::tr!("低电量阈值"),
            Some(crate::tr!("低于该电量时设备指示灯会闪")),
        ),
        (
            crate::tr!("充满效果"),
            Some(crate::tr!("充满电时播放一次灯效")),
        ),
    ];
    let total_h: f32 = rows.iter().map(|_| layout::ROW_H).sum();
    let card = c.card(p, total_h);
    let mut ry = card.y;
    let mut i = 0;

    // 自动休眠
    {
        let area = row_area(card, ry);
        let ctl = row_label(p, area, rows[i].0, rows[i].1, 150.0);
        drop_button(p, ui, R::new(ctl.x, ctl.cy() - 15.0, 150.0, 30.0), Drop::Sleep, d);
        if i + 1 < rows.len() {
            row_rule(p, card, area);
        }
        ry += layout::ROW_H;
        i += 1;
    }

    // 低电量阈值：输入框 + % + 应用
    {
        let area = row_area(card, ry);
        let ctl = row_label(p, area, rows[i].0, rows[i].1, 150.0);
        let edit = Edit::LowWarn;
        let txt = ui
            .edit_text(edit)
            .map(|s| s.to_string())
            .or_else(|| known_state(d).map(|s| s.low_warn.to_string()))
            .unwrap_or_else(|| "—".to_string());
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
        button(p, ui, btn, crate::tr!("应用"), false, Act::ApplyEdit(edit));
        row_rule(p, card, area);
        ry += layout::ROW_H;
        i += 1;
    }

    // 充满效果
    {
        let area = row_area(card, ry);
        let ctl = row_label(p, area, rows[i].0, rows[i].1, 40.0);
        let on = st.map(|s| s.full_effect != 0).unwrap_or(true);
        let sw = R::new(ctl.right() - 40.0, ctl.cy() - 10.0, 40.0, 20.0);
        p.switch(sw, on, st.is_some());
        ui.hot(sw, Act::Toggle(Pref::FullEffect));
    }
    c.y = card.bottom();

    c.h2(p, crate::tr!("电量历史"), false);

    let chart = c.card(p, 220.0);
    draw_chart(p, chart, d, ui);
    // 卡片不会自己推进游标（它的返回值才是位置），必须手动前移 ——
    // 漏了这一步，「电源管理」的标题会画在图表卡片上面。
    c.y = chart.bottom();

    // ---- 续航明细 ----
    //
    // 结论已经放到电量环下面了，这里不再重复整句，只留「怎么算出来的」，
    // 免得同一句话在一屏里出现两次。样本不足时这里才显示原因。
    if d.estimate.known {
        c.y += 14.0;
        let line = if d.estimate.stale_seconds > 0 {
            // 沿用旧结论时必须说明白，否则等于拿旧斜率冒充当前状态。
            crate::trf!(
                "沿用 {}前的估算 · 当前读数没更新，暂时没法重算",
                crate::i18n::age(d.estimate.stale_seconds)
            )
        } else {
            crate::trf!(
                "按最近 {} 个采样估算 · 变化 {:+.2} %/h",
                d.estimate.samples,
                d.estimate.percent_per_hour
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
            &d.estimate.localized_text(),
            R::new(c.x + 6.0, c.y, c.w - 12.0, 18.0),
            font::HINT,
            false,
            t.text3,
            Align::Left,
        );
        c.y += 22.0;
    }
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

fn chart_geom(
    pl: &crate::chart::Plot,
    plot: R,
    lo: i32,
    span: f32,
) -> crate::ui::widget::ChartGeom {
    let area_w = (crate::chart::AREA_W as f32).max(1.0);
    let map_x = |px: i32| -> f32 {
        let fx = (px - crate::chart::AREA_X) as f32 / area_w;
        plot.x + fx.clamp(0.0, 1.0) * plot.w
    };
    let map_y = |level: u8| -> f32 {
        let fy = (level as i32 - lo) as f32 / span;
        plot.bottom() - fy.clamp(0.0, 1.0) * plot.h
    };
    let map_fx = |fx: f32| -> f32 { plot.x + fx.clamp(0.0, 1.0) * plot.w };
    let map_yf = |level: f32| -> f32 {
        let fy = (level - lo as f32) / span;
        plot.bottom() - fy.clamp(0.0, 1.0) * plot.h
    };

    let pts: Vec<(f32, f32)> = pl
        .points
        .iter()
        .map(|pt| (map_x(pt.x), map_y(pt.level)))
        .collect();

    let mut bands: Vec<R> = Vec::new();
    for w in pl.points.windows(2) {
        if w[0].charging && w[1].charging {
            let x0 = map_x(w[0].x);
            let x1 = map_x(w[1].x);
            if x1 - x0 > 0.5 {
                bands.push(R::new(x0, plot.y, x1 - x0, plot.h));
            }
        }
    }

    let curve: Vec<(f32, f32)> = pl
        .curve
        .iter()
        .map(|c| (map_fx(c.fx), map_yf(c.level)))
        .collect();

    // 面积：曲线 + 首尾各拉到底边。起点用曲线自己的 x，不要用 plot.x ——
    // 第一个采样点可能晚于窗口起点（休眠、刚装完），从 plot.x 硬拉
    // 会多出一块三角形空填充，看着像画错了。
    let area = if curve.len() >= 2 {
        let x0 = curve.first().map(|p| p.0).unwrap_or(plot.x);
        let x1 = curve.last().map(|p| p.0).unwrap_or(plot.right());
        let mut a: Vec<(f32, f32)> = Vec::with_capacity(curve.len() + 2);
        a.push((x0, plot.bottom()));
        a.extend(curve.iter().copied());
        a.push((x1, plot.bottom()));
        a
    } else {
        Vec::new()
    };

    crate::ui::widget::ChartGeom {
        key: (
            pl as *const _ as usize,
            plot.x.to_bits(),
            plot.y.to_bits(),
            plot.w.to_bits(),
            plot.h.to_bits(),
            lo as u32,
            (lo as f32 + span) as u32,
        ),
        pts,
        curve,
        area,
        bands,
    }
}

fn draw_chart(p: &mut Painter, card: R, d: &Data, ui: &mut crate::ui::widget::Ui) {
    let t = p.t;
    let head_y = card.y + 16.0;
    p.text(
        crate::tr!("近 48 小时"),
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
            crate::tr!("充电"),
            R::new(card.x + 106.0, head_y, 34.0, 18.0),
            font::TINY,
            false,
            t.text3,
            Align::Left,
        );
    }
    let samples = crate::trf!("{} 个采样点", d.rows);
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

    // 归一化坐标 → 像素的那几步在 `chart_geom` 里做，结果按
    // (Plot, 绘图区, 纵轴范围) 缓存，不在每帧重算。
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
            // 像素几何只在「数据变了 / 绘图区尺寸变了 / 纵轴范围变了」时重算。
            // 充电动画每 16ms 重绘整帧，但采样 30 秒才更新一次 —— 不缓存的话
            // 每秒白建 60 遍三份 Vec（最多两千多点），全在 UI 线程上。
            let key = (
                pl as *const _ as usize,
                plot.x.to_bits(),
                plot.y.to_bits(),
                plot.w.to_bits(),
                plot.h.to_bits(),
                lo as u32,
                hi as u32,
            );
            if ui.chart.as_ref().map(|g| g.key) != Some(key) {
                ui.chart = Some(chart_geom(pl, plot, lo, span));
            }
            let g = ui.chart.as_ref().expect("上一行刚填过");
            let pts = &g.pts;

            // 充电时段：整列淡绿底带。比在折线上换颜色更直观 ——
            // 一眼就能看出"这段在充、那段在放"。
            for band in &g.bands {
                p.rect(*band, with_alpha(t.green, 0x12));
            }

            // 面积 + 曲线：平滑与插值已在 `chart::build` 做完。
            if g.area.len() >= 2 {
                p.fill_polygon(&g.area, with_alpha(t.accent, 0x4A), with_alpha(t.accent, 0x00));
            }
            // 整条曲线一次描边。逐段 `polyline` 在接缝处会叠出「串珠」感。
            p.stroke_path(&g.curve, t.accent, 2.0);

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

// ───────────────────────── 性能页 ─────────────────────────

/// 设备状态是不是「占位值」。
///
/// `State::default()` 解出来正好是 800 DPI + 纯黑，和真实读数长得一模一样：
/// 读不到设备时把 `enc/rgb` 原样画出来，用户会以为那就是设备当前设置，
/// 点「应用」才发现改的是个编出来的值。这里把「全零 = 没读到」判出来。
pub(crate) fn placeholder_state(st: &crate::proto::State) -> bool {
    st.enc.iter().all(|&e| e == 0) && st.rgb.iter().all(|c| *c == [0, 0, 0])
}

/// 可信的设备状态：读不到设备、或只有占位值时给 `None`。
fn known_state<'a>(d: &'a Data) -> Option<&'a crate::proto::State> {
    d.state.filter(|s| !placeholder_state(s))
}

fn perf(p: &mut Painter, ui: &mut Ui, d: &Data, c: &mut Cursor) {
    let t = p.t;
    let st = known_state(d);
    let cur = st.map(cur_stage);

    // ---- DPI 档位 ----
    c.h2(p, crate::tr!("DPI 档位"), true);
    let dpi_h = 4.0 * 46.0 + 40.0;
    let card = c.card(p, dpi_h);
    let mut y = card.y + 4.0;
    for i in 0..crate::proto::DPI_STAGES {
        let is_cur = Some(i) == cur;
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
            &crate::trf!("档位 {}", i + 1),
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
                crate::tr!("● 当前使用"),
                R::new(row.right() - 146.0, row.y, 76.0, row.h),
                11.0,
                true,
                t.accent,
                Align::Left,
            );
        } else {
            let switch_btn = R::new(row.right() - 124.0, row.cy() - 15.0, 54.0, 30.0);
            button(
                p,
                ui,
                switch_btn,
                crate::tr!("切换"),
                false,
                Act::PickDrop(Drop::Stage, i),
            );
        }
        let apply_btn = R::new(row.right() - 62.0, row.cy() - 15.0, 52.0, 30.0);
        button(
            p,
            ui,
            apply_btn,
            crate::tr!("应用"),
            false,
            Act::ApplyEdit(edit),
        );

        y += 46.0;
    }
    // 说明
    p.text_wrap(
        crate::tr!("范围 100–42000 DPI，步进 50。颜色是该档位在设备上的指示灯颜色。"),
        R::new(card.x + 18.0, y + 6.0, card.w - 36.0, 32.0),
        11.5,
        t.text3,
    );
    c.y = card.bottom();

    // ---- 回报率 ----
    c.h2(p, crate::tr!("回报率"), false);
    let card = c.card(p, layout::ROW_H * 2.0);
    {
        let area = row_area(card, card.y);
        let ctl = row_label(
            p,
            area,
            crate::tr!("轮询频率"),
            Some(crate::tr!("越高越跟手，也越耗电")),
            330.0,
        );
        // 读不到设备时不高亮任何一档（默认 1000Hz 是编出来的值）
        let sel = st.map(|s| (s.poll_code as usize).min(3)).unwrap_or(usize::MAX);
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
        row_rule(p, card, area);
    }
    {
        let area = row_area(card, card.y + layout::ROW_H);
        let ctl = row_label(
            p,
            area,
            crate::tr!("当前档位"),
            Some(crate::tr!("同时决定设备的 DPI 和档位指示灯")),
            220.0,
        );
        segmented(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, 220.0, 30.0),
            &["1", "2", "3", "4"],
            // 读不到设备时不给任何一项打高亮
            cur.unwrap_or(usize::MAX),
            |i| Act::PickDrop(Drop::Stage, i),
        );
    }
    c.y = card.bottom();

    // ---- 去抖 ----
    c.h2(p, crate::tr!("去抖"), false);
    let card = c.card(p, layout::ROW_H);
    {
        let area = row_area(card, card.y);
        let ctl = row_label(
            p,
            area,
            crate::tr!("按键去抖时间"),
            Some(crate::tr!("双击被识别成单击时调大")),
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
    c.h2(p, crate::tr!("角度"), false);
    let adv_open = ui.adv[0];
    let base_h = layout::ROW_H + if adv_open { 46.0 + 46.0 + 46.0 } else { 46.0 };
    let card = c.card(p, base_h);
    {
        let area = row_area(card, card.y);
        let ctl = row_label(
            p,
            area,
            crate::tr!("角度吸附"),
            Some(crate::tr!("斜向移动时自动贴合水平/垂直")),
            40.0,
        );
        let on = st.map(|s| s.angle_snap != 0).unwrap_or(false);
        let sw = R::new(ctl.x + 4.0, ctl.cy() - 10.0, 40.0, 20.0);
        p.switch(sw, on, st.is_some());
        ui.hot(sw, Act::Toggle(Pref::AngleSnap));
        row_rule(p, card, area);
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
            crate::tr!("高级"),
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
        let ctl = row_label(p, area, crate::tr!("角度微调"), None, 260.0);
        let track = R::new(ctl.x, ctl.cy() - 9.0, 190.0, 18.0);
        p.slider(track, frac, st.is_some());
        ui.hot(track, Act::SetSlide(Slide::Tune, frac));
        p.text(
            &format!("{val:+}"),
            R::new(ctl.right() - 40.0, ctl.y, 40.0, ctl.h),
            12.0,
            false,
            t.text2,
            Align::Right,
        );
        row_rule(p, card, area);

        // 加减速：设备不支持，明确说明而不是画一个假控件
        let ny = sy + 46.0;
        let area = R::new(card.x + 18.0, ny, card.w - 36.0, 46.0);
        row_label(p, area, crate::tr!("加减速曲线"), None, 120.0);
        p.text(
            crate::tr!("本型号不支持"),
            R::new(area.right() - 110.0, area.y, 110.0, area.h),
            12.0,
            false,
            t.text3,
            Align::Right,
        );
    }
    c.y = card.bottom();

    // ---- 抬起距离 ----
    c.h2(p, crate::tr!("抬起距离"), false);
    let card = c.card(p, layout::ROW_H);
    {
        let area = row_area(card, card.y);
        let ctl = row_label(
            p,
            area,
            crate::tr!("抬起距离（LOD）"),
            Some(crate::tr!("低省电，高更容易追踪")),
            160.0,
        );
        let sel = st.map(|s| (s.lod as usize).min(1)).unwrap_or(usize::MAX);
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
        crate::tr!("修改抬起距离会重置设备的表面标定，无法自动恢复。"),
        true,
    );
    c.y += nw;
}

// ───────────────────────── 灯光页 ─────────────────────────

fn light(p: &mut Painter, ui: &mut Ui, _d: &Data, _f: &Frame, c: &mut Cursor) {
    let t = p.t;

    let lamp = crate::dynlight::info();

    // ---- 控制方式 ----
    c.h2(p, crate::tr!("控制方式"), true);
    let card = c.card(p, layout::ROW_H + 8.0);
    {
        let area = row_area(card, card.y + 4.0);
        p.circle(area.x + 5.0, area.cy(), 4.5, t.green);
        p.text(
            crate::tr!("由 Windows 动态光效控制"),
            R::new(area.x + 20.0, area.y, (area.w - 200.0).max(80.0), area.h),
            font::BODY,
            false,
            t.text,
            Align::Left,
        );
        let btn = R::new(area.right() - 160.0, area.cy() - 15.0, 160.0, 30.0);
        button(
            p,
            ui,
            btn,
            crate::tr!("打开动态光效设置"),
            true,
            Act::OpenLightingSettings,
        );
    }
    c.y = card.bottom();
    c.y += 12.0;
    p.text_wrap(
        crate::tr!("灯效在「设置 → 个性化 → 动态光效」里调：纯色 / 呼吸 / 渐变 / 波浪 / 闪烁等都由 Windows 提供。本程序不再写厂商私有灯效报文，避免和系统互相覆盖导致闪烁。"),
        R::new(c.x, c.y, c.w, 44.0),
        font::HINT,
        t.text3,
    );
    c.y += 52.0;

    // ---- 灯效设备 ----
    c.h2(p, crate::tr!("灯效设备"), false);
    let extra = if lamp.is_some() { layout::ROW_H } else { 0.0 };
    let card = c.card(p, layout::ROW_H + extra);
    {
        let area = row_area(card, card.y);
        let found = lamp.is_some();
        p.circle(
            area.x + 5.0,
            area.cy(),
            4.5,
            if found { t.green } else { t.text3 },
        );
        p.text(
            if found {
                crate::tr!("已检测到 HID LampArray 设备")
            } else {
                crate::tr!("未检测到 HID LampArray 设备")
            },
            R::new(area.x + 20.0, area.y, (area.w - 130.0).max(80.0), area.h),
            font::BODY,
            false,
            if found { t.text } else { t.text2 },
            Align::Left,
        );
        let btn = R::new(area.right() - 96.0, area.cy() - 15.0, 96.0, 30.0);
        button(
            p,
            ui,
            btn,
            crate::tr!("重新检测"),
            false,
            Act::RefreshLighting,
        );
    }
    if let Some(l) = lamp {
        let area = R::new(
            card.x + 18.0,
            card.y + layout::ROW_H,
            card.w - 36.0,
            layout::ROW_H,
        );
        p.hline(card.x, card.right(), area.y, t.border);
        let ctl = row_label(p, area, crate::tr!("灯珠数量 / 类型"), None, 240.0);
        p.text(
            &crate::trf!("{} 个 · {}", l.lamp_count, l.kind_name()),
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
        crate::tr!("灯效不跟系统走时：Armoury Crate 的灯效可能正在抢占设备，先关掉它的灯效（或退出），再回到系统「动态光效」页面。"),
        false,
    );
    c.y += nw + 16.0;
}

// ───────────────────────── 按键页 ─────────────────────────

fn keys(p: &mut Painter, ui: &mut Ui, d: &Data, _f: &Frame, c: &mut Cursor) {
    let t = p.t;

    // 顶部警告提示条（用户反馈置顶更醒目）
    c.y += 4.0;
    let nw = note(
        p,
        R::new(c.x, c.y, c.w, 20.0),
        crate::tr!("把某个键绑成「无功能」（0000）后，该键在设备上将完全失效 —— 如需恢复必须重新写入完整映射配置。"),
        true,
    );
    c.y += nw + 16.0;

    c.h2(p, crate::tr!("按键映射"), true);
    let rows = crate::proto::BUTTON_SLOTS;
    let card = c.card(p, rows as f32 * layout::ROW_H);
    for slot in 0..rows {
        let y = card.y + slot as f32 * layout::ROW_H;
        let area = row_area(card, y);
        let src = crate::proto::BUTTON_SRC[slot];
        let name = crate::i18n::translate(crate::proto::BUTTON_NAME[slot]);

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
            row_rule(p, card, area);
        }
    }
    c.y = card.bottom() + 16.0;

    // 底部操作按钮
    let bw = 160.0;
    let br = R::new(c.x, c.y, bw, 34.0);
    button(
        p,
        ui,
        br,
        crate::tr!("从备份恢复全部按键"),
        false,
        Act::Restore,
    );
    let br2 = R::new(br.right() + 12.0, c.y, bw, 34.0);
    button(p, ui, br2, crate::tr!("导出当前配置"), false, Act::Backup);
    c.y += 34.0;
}

// ───────────────────────── 设置页 ─────────────────────────

fn settings(p: &mut Painter, ui: &mut Ui, d: &Data, _f: &Frame, c: &mut Cursor) {
    let t = p.t;
    let previous = if d.selected.is_some() {
        Page::Battery
    } else {
        Page::Home
    };
    page_heading(p, ui, c, crate::tr!("设置"), Some(Act::Nav(previous)));

    // ---- 程序 ----
    c.h2(p, crate::tr!("程序"), true);
    let card = c.card(p, layout::ROW_H * 6.0);
    let mut y = card.y;
    {
        let area = row_area(card, y);
        let ctl = row_label(
            p,
            area,
            crate::tr!("语言"),
            Some(crate::tr!("立即生效，无需重启")),
            180.0,
        );
        drop_button(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, 180.0, 30.0),
            Drop::Language,
            d,
        );
        row_rule(p, card, area);
        y += layout::ROW_H;
    }

    // 托盘
    {
        let area = row_area(card, y);
        let ctl = row_label(
            p,
            area,
            crate::tr!("显示托盘图标"),
            Some(crate::tr!("关闭后只能用窗口操作")),
            40.0,
        );
        let sw = R::new(ctl.x + 4.0, ctl.cy() - 10.0, 40.0, 20.0);
        p.switch(sw, d.cfg.tray_enabled, true);
        ui.hot(sw, Act::Toggle(Pref::TrayEnabled));
        row_rule(p, card, area);
        y += layout::ROW_H;
    }
    {
        let area = row_area(card, y);
        let ctl = row_label(
            p,
            area,
            crate::tr!("托盘显示设备"),
            Some(crate::tr!("单击数字查看全部设备")),
            180.0,
        );
        drop_button(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, 180.0, 30.0),
            Drop::TrayDevice,
            d,
        );
        row_rule(p, card, area);
        y += layout::ROW_H;
    }
    // 关闭窗口动作
    {
        let area = row_area(card, y);
        let ctl = row_label(
            p,
            area,
            crate::tr!("关闭窗口动作"),
            Some(crate::tr!("点击窗口右上角 × 按钮时的操作")),
            160.0,
        );
        drop_button(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, 160.0, 30.0),
            Drop::CloseMode,
            d,
        );
        row_rule(p, card, area);
        y += layout::ROW_H;
    }
    // 开机自启
    {
        let area = row_area(card, y);
        let ctl = row_label(
            p,
            area,
            crate::tr!("开机自动启动"),
            Some(d.autostart_text),
            40.0,
        );
        // 真实状态从 `autostart` 模块来，这里只显示；点击发 Toggle。
        // 开关的布尔量也直接用状态机的值：以前是 `autostart_text.contains("已启用")`，
        // 文案一改开关就跟着错（而它俩本来没有关系）。
        let on = d.autostart_on;
        let sw = R::new(ctl.x + 4.0, ctl.cy() - 10.0, 40.0, 20.0);
        p.switch(sw, on, true);
        ui.hot(sw, Act::Toggle(Pref::AutoStart));
        row_rule(p, card, area);
        y += layout::ROW_H;
    }
    // 刷新间隔
    {
        let area = row_area(card, y);
        let ctl = row_label(
            p,
            area,
            crate::tr!("后台刷新间隔"),
            Some(crate::tr!("读取设备状态的频率")),
            150.0,
        );
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
    c.h2(p, crate::tr!("数据"), false);
    let card = c.card(p, layout::ROW_H * 3.0);
    let mut y = card.y;
    // 缓存有效期
    {
        let area = row_area(card, y);
        let ctl = row_label(
            p,
            area,
            crate::tr!("快照缓存有效期"),
            Some(crate::tr!("设为「每次都重读」会明显更慢")),
            150.0,
        );
        drop_button(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, 150.0, 30.0),
            Drop::CacheTtl,
            d,
        );
        row_rule(p, card, area);
        y += layout::ROW_H;
    }
    // 备份与还原
    {
        let area = row_area(card, y);
        let ctl = row_label(
            p,
            area,
            crate::tr!("备份与还原"),
            Some(crate::tr!("把全部 31 项参数存成一个文本文件")),
            230.0,
        );
        let b1 = R::new(ctl.x, ctl.cy() - 15.0, 110.0, 30.0);
        button(p, ui, b1, crate::tr!("备份到文件"), false, Act::Backup);
        let b2 = R::new(b1.right() + 10.0, ctl.cy() - 15.0, 110.0, 30.0);
        button(p, ui, b2, crate::tr!("从文件还原"), false, Act::Restore);
        row_rule(p, card, area);
        y += layout::ROW_H;
    }
    // 历史记录
    {
        let area = row_area(card, y);
        let ctl = row_label(
            p,
            area,
            crate::tr!("历史记录"),
            Some(crate::tr!("电量采样点，最多保留 48 小时")),
            230.0,
        );
        let b1 = R::new(ctl.x, ctl.cy() - 15.0, 130.0, 30.0);
        button(
            p,
            ui,
            b1,
            crate::tr!("打开所在文件夹"),
            false,
            Act::OpenHistoryFolder,
        );
        let b2 = R::new(b1.right() + 10.0, ctl.cy() - 15.0, 90.0, 30.0);
        button(p, ui, b2, crate::tr!("清空"), false, Act::ClearHistory);
    }
    c.y = card.bottom();

    // ---- 存储路径 ----
    c.h2(p, crate::tr!("存储路径"), false);
    let card = c.card(p, layout::ROW_H * 2.0);
    {
        // 历史文件
        let area = row_area(card, card.y);
        let hp = crate::history::history_path().display().to_string();
        let status = if crate::history::history_path_is_custom() {
            crate::tr!("已自定义")
        } else {
            crate::tr!("默认")
        };
        let title = crate::trf!("历史文件保存位置（{status}）", status = status);
        let ctl = row_label(p, area, &title, Some(&hp), 128.0);
        let b1 = R::new(ctl.right() - 52.0, ctl.cy() - 15.0, 52.0, 30.0);
        button(p, ui, b1, crate::tr!("更改"), false, Act::ChangeHistoryPath);
        let b2 = R::new(b1.x - 64.0, ctl.cy() - 15.0, 56.0, 30.0);
        button(p, ui, b2, crate::tr!("默认"), false, Act::ResetHistoryPath);
        row_rule(p, card, area);
    }
    {
        // 设置文件
        let area = row_area(card, card.y + layout::ROW_H);
        let sp = crate::paths::settings_file().display().to_string();
        let status = if crate::paths::custom_settings_file().is_some() {
            crate::tr!("已自定义")
        } else {
            crate::tr!("默认")
        };
        let title = crate::trf!("设置文件保存位置（{status}）", status = status);
        let ctl = row_label(p, area, &title, Some(&sp), 128.0);
        let b1 = R::new(ctl.right() - 52.0, ctl.cy() - 15.0, 52.0, 30.0);
        button(
            p,
            ui,
            b1,
            crate::tr!("更改"),
            false,
            Act::ChangeSettingsPath,
        );
        let b2 = R::new(b1.x - 64.0, ctl.cy() - 15.0, 56.0, 30.0);
        button(p, ui, b2, crate::tr!("默认"), false, Act::ResetSettingsPath);
    }
    c.y = card.bottom();

    // ---- 外观 ----
    c.h2(p, crate::tr!("外观"), false);
    let card = c.card(p, layout::ROW_H);
    {
        let area = row_area(card, card.y);
        let ctl = row_label(
            p,
            area,
            crate::tr!("配色"),
            Some(crate::tr!("跟随系统或手动选择")),
            210.0,
        );
        let sel = ui.theme_pref.min(2) as usize;
        segmented(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, 210.0, 30.0),
            &[
                crate::tr!("跟随系统"),
                crate::tr!("浅色"),
                crate::tr!("深色"),
            ],
            sel,
            |i| Act::SetTheme(i as u8),
        );
    }
    c.y = card.bottom();

    // ---- 关于 ----

    c.h2(p, crate::tr!("关于"), false);
    let card = c.card(p, layout::ROW_H * 3.0);
    {
        // 版本：值右对齐，版本号来自 Cargo.toml（不再手写常量）
        let area = row_area(card, card.y);
        row_label(
            p,
            area,
            crate::tr!("版本"),
            Some(crate::tr!(
                "R Battery · 不依赖 ASUS 驱动的 ROG 设备参数工具"
            )),
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
        row_rule(p, card, area);
    }
    {
        // 开源仓库
        let area = R::new(
            card.x + 18.0,
            card.y + layout::ROW_H,
            card.w - 36.0,
            layout::ROW_H,
        );
        let ctl = row_label(p, area, crate::tr!("开源仓库"), Some(REPO_URL), 64.0);
        button(
            p,
            ui,
            R::new(ctl.x, ctl.cy() - 15.0, 64.0, 30.0),
            crate::tr!("打开"),
            false,
            Act::OpenRepo,
        );
        row_rule(p, card, area);
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
            crate::tr!("打开"),
            false,
            Act::OpenStore,
        );
    }
    c.y = card.bottom();

    let _ = Breakpoint::Wide;
    // ---- 诊断 ----
    // 放在页面最下面：平时不需要看，出问题时才知道要找它。

    c.h2(p, crate::tr!("诊断"), false);
    let dcard = c.card(p, layout::ROW_H * 2.0);
    let mut dy = dcard.y;
    {
        let area = row_area(dcard, dy);
        let _ = row_label(
            p,
            area,
            crate::tr!("诊断日志"),
            Some(crate::tr!(
                "点这一行打开日志文件夹；日志包含设备型号、VID/PID、接收器信息、操作结果和崩溃位置"
            )),
            40.0,
        );
        ui.hot(area, Act::OpenLogFolder);
        row_rule(p, dcard, area);
        dy += layout::ROW_H;
    }
    {
        let area = row_area(dcard, dy);
        let _ = row_label(p, area, crate::tr!("版本"), Some(d.version), 40.0);
    }
    c.y = dcard.bottom();
}

/// 绘制「关闭窗口」确认模态对话框。
pub fn draw_close_dialog(p: &mut Painter, ui: &mut Ui, win_w: f32, win_h: f32) {
    let t = p.t;
    // 半透明遮罩蒙层
    p.rect(R::new(0.0, 0.0, win_w, win_h), 0x7500_0000);

    // 居中卡片
    let w = if crate::i18n::english() { 500.0 } else { 420.0 };
    let h = 240.0;
    let r = R::new((win_w - w) / 2.0, (win_h - h) / 2.0, w, h);
    p.card(r, 12.0, t.card, t.border);

    // 标题与副标题
    p.text(
        crate::tr!("关闭应用"),
        R::new(r.x + 24.0, r.y + 20.0, w - 48.0, 24.0),
        16.0,
        true,
        t.text,
        Align::Left,
    );
    p.text(
        crate::tr!("请选择点击关闭按钮时的操作："),
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
        crate::tr!("最小化到系统托盘（在后台继续运行）"),
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
        crate::tr!("直接退出程序"),
        R::new(opt2.x + 26.0, opt2.y + 4.0, opt2.w - 26.0, 24.0),
        13.0,
        false,
        t.text,
        Align::Left,
    );
    ui.dialog_hot(opt2, Act::SelectCloseChoice(crate::ui::widget::CloseChoice::Quit));

    // 复选框：记住我的选择
    let cb_r = R::new(r.x + 24.0, r.y + 154.0, w - 48.0, 24.0);
    p.checkbox(
        R::new(cb_r.x + 2.0, cb_r.cy() - 8.0, 16.0, 16.0),
        ui.close_remember,
        true,
    );
    p.text(
        crate::tr!("记住我的选择，下次不再提示"),
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
    p.text(
        crate::tr!("取消"),
        cancel_r,
        13.0,
        false,
        t.text,
        Align::Center,
    );
    ui.dialog_hot(cancel_r, Act::CancelCloseDialog);

    // 确定按钮
    p.round(confirm_r, 6.0, t.accent);
    p.text(
        crate::tr!("确定"),
        confirm_r,
        13.0,
        true,
        0xFFFF_FFFF,
        Align::Center,
    );
    ui.dialog_hot(confirm_r, Act::ConfirmCloseDialog);
}
