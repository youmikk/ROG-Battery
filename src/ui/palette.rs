//! 颜色的数值层：HSV 换算、预设色板、以及「色板下标 / 打包 RGB」的双表示。
//!
//! 这里原来长在 `pages.rs`（绘制层）里 —— 于是「颜色怎么编码」这件事被
//! 绑在「颜色怎么画」上：`gui.rs` 想拿一个 `usize` 下标对应的 RGB，得先
//! `use pages`。抽出来以后绘制层只剩画，事件层直接问这里要值。

/// 色盘格子边长与间距（逻辑像素）。
pub const PAL_CELL: f32 = 30.0;

pub const PAL_GAP: f32 = 6.0;

pub const PAL_COLS: usize = 12;

/// HSV → RGB。`h` 是 0–360 度，`s`/`v` 是 0–1。
///
/// 自己算而不是查表：色盘要的是「色相均分」这个性质，写成 12 个角度的循环
/// 比手抄 24 个十六进制更不容易抄错，也顺手给出两档明度。
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
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
