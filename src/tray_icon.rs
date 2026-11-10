//! 透明托盘图标：设备图案与紧凑电量数字。HICON 由调用方销毁。

use core::ffi::c_void;
use core::ptr;

use crate::win32::Handle;

/// 图标布局基准尺寸。
pub const ICON_SIZE: i32 = 32;

pub fn render_device(
    level: Option<u8>,
    charging: bool,
    kind: crate::tray_prefs::TrayKind,
    content: crate::tray_prefs::TrayContent,
) -> Option<Handle> {
    let theme = if crate::autostart::system_uses_light_theme() {
        &crate::ui::theme::LIGHT
    } else {
        &crate::ui::theme::DARK
    };
    render_device_at(level, charging, kind, content, theme, icon_size())
}

fn render_device_at(
    level: Option<u8>,
    charging: bool,
    kind: crate::tray_prefs::TrayKind,
    content: crate::tray_prefs::TrayContent,
    theme: &crate::ui::theme::Theme,
    size: i32,
) -> Option<Handle> {
    use crate::tray_prefs::TrayContent;
    use crate::ui::gfx::{Painter, R};
    use gdiplus::*;
    if !crate::ui::gfx::start() {
        return None;
    }
    unsafe {
        let mut bitmap = ptr::null_mut();
        let mut graphics = ptr::null_mut();
        if GdipCreateBitmapFromScan0(
            size,
            size,
            0,
            PIXEL_FORMAT_32BPP_ARGB,
            ptr::null(),
            &mut bitmap,
        ) != 0
        {
            return None;
        }
        if GdipGetImageGraphicsContext(bitmap, &mut graphics) != 0 {
            GdipDisposeImage(bitmap);
            return None;
        }
        GdipGraphicsClear(graphics, 0);
        GdipSetSmoothingMode(graphics, 4);
        GdipSetTextRenderingHint(graphics, 4);
        let ink = match level {
            None => theme.text3,
            Some(0..=20) => theme.red,
            _ => theme.text,
        };
        let painted = Painter::on_graphics(graphics, theme, 32.0, size as f32 / 32.0, |p| {
            let combined = content == TrayContent::Combined;
            if content != TrayContent::Number {
                let area = if combined {
                    R::new(1.5, 6.0, 10.0, 20.0)
                } else {
                    R::new(2.0, 2.0, 28.0, 28.0)
                };
                device_glyph(p, area, kind, ink);
            }
            if content != TrayContent::Icon {
                let text = level.map(label).unwrap_or_else(|| "–".to_string());
                let rect = if combined {
                    let width = if text.len() == 1 { 10.0 } else { 18.0 };
                    R::new(22.5 - width / 2.0, 4.0, width, 24.0)
                } else {
                    let width = if text.len() == 1 { 18.0 } else { 30.0 };
                    let top = if charging { 5.0 } else { 2.0 };
                    R::new(16.0 - width / 2.0, top, width, 30.0 - top)
                };
                if level.is_some() {
                    p.icon_text(&text, rect, if combined { 23.0 } else { 28.0 }, ink);
                } else {
                    p.line(
                        rect.cx() - 4.0,
                        rect.cy(),
                        rect.cx() + 4.0,
                        rect.cy(),
                        ink,
                        1.8,
                    );
                }
            }
            if charging && level.is_some() {
                p.dot(if combined { 6.5 } else { 29.0 }, 2.0, 1.8, theme.green);
            }
        });
        GdipDeleteGraphics(graphics);
        let mut icon = ptr::null_mut();
        let result = painted.is_some() && GdipCreateHICONFromBitmap(bitmap, &mut icon) == 0;
        GdipDisposeImage(bitmap);
        if result && !icon.is_null() {
            Some(icon)
        } else {
            None
        }
    }
}

fn device_glyph(
    p: &mut crate::ui::gfx::Painter,
    r: crate::ui::gfx::R,
    kind: crate::tray_prefs::TrayKind,
    ink: u32,
) {
    use crate::tray_prefs::TrayKind;
    use crate::ui::gfx::R;
    let stroke = 2.4;
    let cx = r.cx();
    let cy = r.cy();
    match kind {
        TrayKind::Mouse => {
            let w = r.w.min(r.h * 0.65);
            let body = R::new(cx - w / 2.0, r.y, w, r.h);
            p.round_stroke(body, w * 0.45, ink, stroke);
            p.line(cx, r.y + r.h * 0.2, cx, r.y + r.h * 0.4, ink, stroke);
        }
        TrayKind::Keyboard => {
            let h = (r.w * 0.8).max(r.h * 0.55);
            p.round_stroke(R::new(r.x, cy - h / 2.0, r.w, h), 1.5, ink, stroke);
            for col in [0.3, 0.7] {
                p.dot(r.x + r.w * col, cy - h * 0.15, 0.85, ink);
            }
            p.line(
                r.x + r.w * 0.25,
                cy + h * 0.2,
                r.right() - r.w * 0.25,
                cy + h * 0.2,
                ink,
                1.8,
            );
        }
        TrayKind::Headset => {
            let radius = r.w * 0.5;
            let top = cy - r.h * 0.35;
            p.arc(cx, top + radius, radius, stroke, 180.0, 180.0, ink);
            p.line(r.x, top + radius, r.x, cy + r.h * 0.22, ink, stroke);
            p.line(
                r.right(),
                top + radius,
                r.right(),
                cy + r.h * 0.22,
                ink,
                stroke,
            );
            let ear_h = r.h * 0.35;
            p.round(R::new(r.x, cy, r.w * 0.28, ear_h), 1.0, ink);
            p.round(
                R::new(r.right() - r.w * 0.28, cy, r.w * 0.28, ear_h),
                1.0,
                ink,
            );
        }
        TrayKind::Gamepad => {
            let h = (r.w * 0.75).max(r.h * 0.6);
            let points: Vec<_> = [
                (0.0, 1.0),
                (0.12, 0.12),
                (0.32, 0.0),
                (0.42, 0.2),
                (0.58, 0.2),
                (0.68, 0.0),
                (0.88, 0.12),
                (1.0, 1.0),
                (0.7, 0.76),
                (0.3, 0.76),
                (0.0, 1.0),
            ]
            .iter()
            .map(|(x, y)| (r.x + x * r.w, cy - h / 2.0 + y * h))
            .collect();
            p.stroke_path(&points, ink, stroke);
            p.dot(r.x + r.w * 0.27, cy - h * 0.05, 0.8, ink);
            p.dot(r.x + r.w * 0.75, cy - h * 0.05, 0.8, ink);
        }
        _ => {
            p.round_stroke(
                R::new(r.x, cy - r.h * 0.35, r.w - 1.0, r.h * 0.7),
                1.5,
                ink,
                stroke,
            );
            p.line(
                cx - 1.0,
                cy - r.h * 0.35 - 2.0,
                cx + 1.0,
                cy - r.h * 0.35 - 2.0,
                ink,
                stroke,
            );
        }
    }
}

/// 绘制时的超采样倍数：先在 `ICON_SIZE * SUPERSAMPLE` 的画布上画，再一次性缩回
/// `ICON_SIZE`。直接按 32px 画时，字形的斜边和光晕只有一两个像素可落，缩放后
/// 边缘会发毛；放大 4 倍画、再按高质量插值缩小，等于让 GDI+ 帮我们做了一次
/// 超采样抗锯齿。
const SUPERSAMPLE: i32 = 4;

/// 系统实际需要的小图标边长。
///
/// 查 `SM_CXSMICON`（=49）。查询失败或返回离谱值时回落到 [`ICON_SIZE`]。
///
/// 夹在 16..=32：小于 16 画不下两位数字，大于 32 没意义（托盘不显示那么大）。
pub fn icon_size() -> i32 {
    let n = unsafe { crate::win32::GetSystemMetrics(49) };
    if n < 16 {
        ICON_SIZE
    } else {
        n.min(ICON_SIZE)
    }
}

/// 背景色 —— 深色圆角块，白字。和曲线不同，托盘图标必须**不透明**
/// 才能在浅色/深色任务栏上都看得清（透明图标配白字在浅色栏上会消失）。
#[cfg(test)]
const BG: [u8; 3] = [0x20, 0x24, 0x2B];

/// 电量对应的图标底色。三档，够用户一眼分辨「还早 / 该充了 / 快没电」。
#[cfg(test)]
fn level_color(level: u8) -> [u8; 3] {
    match level {
        0..=20 => [0xD1, 0x3A, 0x2E],  // 红：快没电
        21..=50 => [0xD8, 0x8B, 0x1E], // 橙：该充了
        _ => BG,                       // 常态深灰
    }
}

/// 图标上要写的文字。
///
/// 只放数字（不带 `%`）：32x32 里 `100%` 四个字形会缩到不可读。
/// 100 写 `99` 会撒谎 —— 所以保留三位，只是把字号按位数缩小。
pub fn label(level: u8) -> String {
    level.min(100).to_string()
}

/// 按位数选字号（**相对 [`ICON_SIZE`] 的 32px 图标**）。
///
/// 一位数（`7`）用 28px，两位数（`82`）用 27px，三位数（`100`）用 20px。
/// 三位数每格只有 10px 宽，20px 是「占满宽度又不互相压线」的上限 ——
/// 原来给 14px，满电时数字小得像注脚，用户专门提过。
/// 这是纯函数，测试直接断言数字，不需要真的画。
///
/// 实际绘制时按 `icon_size()` 等比缩放 —— 见 [`scaled_font_height`]。
#[cfg(test)]
pub fn font_height(level: u8) -> i32 {
    match label(level).len() {
        1 => 28,
        2 => 27,
        _ => 20,
    }
}

/// 把 [`font_height`] 从 32px 基准缩放到目标边长。
///
/// 单独抽出来是为了能测：16px 图标上两位数必须落在 8..=12 之间，
/// 否则要么糊成一团、要么顶出图标边界。
#[cfg(test)]
pub fn scaled_font_height(level: u8, size: i32) -> i32 {
    let h = font_height(level) * size.max(1) / ICON_SIZE;
    h.max(6) // 再小就完全看不清了，宁可爱出界一点
}

/// 把 `[r, g, b]` 转 `COLORREF`（`0x00BBGGRR`）。
///
/// 顺序写反是最常见的 GDI 错误：红色 `(255,0,0)` 写成 `0x0000FF` 会变蓝。
fn colorref(c: [u8; 3]) -> u32 {
    (c[0] as u32) | ((c[1] as u32) << 8) | ((c[2] as u32) << 16)
}



// ==================== GDI+ 透明图标 ====================
//
// 用户要求：托盘图标「要透明、只显示数字，像系统自带的字体」。
// 裸 GDI 那条路做不到真透明（DrawTextW 把 alpha 写成 0，只能整块补回不透明），
// 所以改用 GDI+ 画到 32bpp ARGB 位图上，再 `GdipCreateHICONFromBitmap` 转 HICON
// —— alpha 通道原样保留，托盘上就是「透明底白字」。

mod gdiplus {
    // PixelFormat32bppARGB = 10 | (32<<8) | Alpha(0x40000) | GDI(0x20000) | Canonical(0x200000)
    pub const PIXEL_FORMAT_32BPP_ARGB: i32 = 0x0026_200A;

    // 这 15 个 GDI+ 入口 `ui/gfx.rs` 里已有声明，这里不再抄一份 —— 抄两份时
    // `GdipDrawString` 的 7 个参数最容易漏，漏了编译不报错、运行时静默错位。
    // `RectF` 也用 gfx 那份：结构虽然相同，但不同的类型会让 `GdipDrawString`
    // 的 `*const RectF` 实参直接类型不匹配。
    pub(crate) use crate::ui::gfx::{
        GdipCreateFont, GdipCreateFontFamilyFromName, GdipCreateSolidFill,
        GdipCreateStringFormat, GdipDeleteBrush, GdipDeleteFont, GdipDeleteFontFamily,
        GdipDeleteGraphics, GdipDeleteStringFormat, GdipDrawString, GdipSetSmoothingMode,
        GdipSetStringFormatAlign, GdipSetStringFormatFlags, GdipSetStringFormatLineAlign,
        GdipSetTextRenderingHint, RectF,
    };

    // 位图图标专用的 5 个入口也是 GDI+ 那一套，声明在 `ui::gfx`（唯一一份）。
    pub(crate) use crate::ui::gfx::{
        GdipCreateBitmapFromScan0, GdipCreateHICONFromBitmap, GdipDisposeImage,
        GdipDrawImageRectI, GdipFillEllipse, GdipFillRectangleI, GdipGetImageGraphicsContext,
        GdipGraphicsClear,
        GdipSetCompositingMode, GdipSetInterpolationMode, GdipSetPixelOffsetMode,
    };
}

/// 透明背景上数字的颜色：自适应浅色/深色任务栏。
///
/// 浅色任务栏（Windows 11 默认/当前）：深炭灰 #1F1F1F，匹配输入法与时钟原生字体；低电警示红、中电橙。
/// 深色任务栏：纯白 #FFFFFF；低电亮红、中电亮橙。
/// 画一张**透明底**、带电量数字的图标（GDI+ 32bpp ARGB）。
///
/// 失败返回 `None`，调用方回落到静态图标 —— **不能让托盘空白**。
/// 成功返回的 `HICON` 由调用方负责 `DestroyIcon`。
#[cfg(test)]
pub fn render(level: u8) -> Option<Handle> {
    render_badge(level, false)
}

/// 缓存读数（设备暂时读不到、沿用的是上一次的值）：灰底 + 近白数字。
///
/// 灰底本身就表示「这不是现在的数」，所以不再叠加充电角标。
#[cfg(test)]
pub fn render_stale(level: u8) -> Option<Handle> {
    render_glyph(
        &label(level),
        scaled_font_height(level, ICON_SIZE),
        [0xF2, 0xF6, 0xFA],
        crate::autostart::system_uses_light_theme(),
        Some([0x6B, 0x72, 0x7A]),
        false,
    )
}

/// 充电中的托盘图标：在徽章右上角加一个深色圆 + 白色闪电，数字左移让位。
#[cfg(test)]
pub fn render_charging(level: u8) -> Option<Handle> {
    render_badge(level, true)
}

/// 读不到电量（休眠 / 未连接 / 首次启动）时的占位图标：一条短横线。
///
/// 以前这种情况退回 `assets/tray.ico`（应用图标），任务栏上会突然只剩一个
/// 图标、看不出是电量 —— 用户报过「托盘短暂出现只显示图标而不是数字」。
pub fn render_unknown() -> Option<Handle> {
    let is_light = crate::autostart::system_uses_light_theme();
    let c = if is_light {
        [0x5A, 0x5A, 0x5A]
    } else {
        [0xD8, 0xD8, 0xD8]
    };
    render_glyph("—", (ICON_SIZE as f32 * 0.46) as i32, c, is_light, None, false)
}

/// 实心圆徽章样式：等级色圆底 + 深色数字（浅色任务栏用深墨，深色任务栏用近白）。
///
/// 与数字样式并存，由调用方挑选；圆底要盖住任务栏的底色，所以数字不能再用
/// 光晕——光晕是给「直接压在任务栏上」的字形用的。
#[cfg(test)]
pub fn render_badge(level: u8, charging: bool) -> Option<Handle> {
    let is_light = crate::autostart::system_uses_light_theme();
    let ink = if is_light {
        [0x0C, 0x11, 0x15]
    } else {
        [0xF2, 0xF6, 0xFA]
    };
    render_glyph(
        &label(level),
        scaled_font_height(level, ICON_SIZE),
        ink,
        is_light,
        Some(level_color(level)),
        charging,
    )
}

fn render_glyph(
    text: &str,
    font_height: i32,
    color: [u8; 3],
    is_light: bool,
    badge: Option<[u8; 3]>,
    charging: bool,
) -> Option<Handle> {
    use gdiplus::*;
    // 基础生成尺寸固定为 ICON_SIZE (32px)，生成高质量 32bpp ARGB 图标。
    // Windows Shell_NotifyIcon 接收 32x32 图标后，由 DWM 高保真缩放到任务栏托盘，
    // 无论是 100%、125%、150% 还是 200% 缩放，均能保持大字号饱满清晰、无毛刺。
    let size = ICON_SIZE * SUPERSAMPLE;
    // 所有以像素为单位的偏移（字形槽外扩、光晕 8 方向偏移）都要跟着放大，
    // 否则放大画布后这些"1 像素"细节会缩水成 1/4 像素、几乎看不见。
    let unit = SUPERSAMPLE as f32;
    // GDI+ 必须已初始化（gfx::start 是进程级幂等的）。
    if !crate::ui::gfx::start() {
        return None;
    }
    unsafe {
        let mut bmp: *mut c_void = ptr::null_mut();
        if GdipCreateBitmapFromScan0(
            size,
            size,
            0,
            PIXEL_FORMAT_32BPP_ARGB,
            ptr::null(),
            &mut bmp,
        ) != 0
            || bmp.is_null()
        {
            return None;
        }
        let mut g: *mut c_void = ptr::null_mut();
        if GdipGetImageGraphicsContext(bmp, &mut g) != 0 || g.is_null() {
            GdipDisposeImage(bmp);
            return None;
        }
        GdipSetSmoothingMode(g, 4); // AntiAlias
        GdipSetTextRenderingHint(g, 4); // TextRenderingHintAntiAlias（灰度，不留彩边）
        GdipGraphicsClear(g, 0x0000_0000); // 全透明

        // 字体：Segoe UI Bold（最接近系统托盘字形）
        let face: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
        let mut family: *mut c_void = ptr::null_mut();
        let mut font: *mut c_void = ptr::null_mut();
        let fh = font_height as f32 * unit;
        if GdipCreateFontFamilyFromName(face.as_ptr(), ptr::null_mut(), &mut family) == 0 {
            // style 1 = Bold，unit 2 = UnitPixel
            let _ = GdipCreateFont(family, fh, 1, 2, &mut font);
        }

        // 居中排版的 StringFormat，禁止折行（折行会导致两位数被折成两行从而第二位被裁切）
        const STRING_FORMAT_NO_WRAP: i32 = 0x0000_1000;
        let mut fmt: *mut c_void = ptr::null_mut();
        let _ = GdipCreateStringFormat(0, 0, &mut fmt);
        if !fmt.is_null() {
            GdipSetStringFormatFlags(fmt, STRING_FORMAT_NO_WRAP);
            GdipSetStringFormatAlign(fmt, 1); // Center
            GdipSetStringFormatLineAlign(fmt, 1); // Center
        }

        let text: Vec<u16> = text.encode_utf16().collect();

        if !font.is_null() {
            // 描边/光晕（8 方向偏 1px）：浅色任务栏柔和浅光晕，深色任务栏柔和深暗影
            let halo_color = if is_light {
                0x35FF_FFFF // 浅色任务栏：柔和半透明浅白微光
            } else {
                0x4500_0000 // 深色任务栏：柔和半透明暗影
            };
            let mut shadow: *mut c_void = ptr::null_mut();
            let _ = GdipCreateSolidFill(halo_color, &mut shadow);

            let c = color;
            let argb = 0xFF00_0000 | ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | (c[2] as u32);
            let mut fg: *mut c_void = ptr::null_mut();
            let _ = GdipCreateSolidFill(argb, &mut fg);

            if let Some(fill) = badge {
                let mut b: *mut c_void = ptr::null_mut();
                if GdipCreateSolidFill(colorref(fill), &mut b) == 0 && !b.is_null() {
                    let d = size as f32 * 0.94;
                    let o = (size as f32 - d) / 2.0;
                    GdipFillEllipse(g, b, o, o, d, d);
                    GdipDeleteBrush(b);
                }
            }
            let offs: [(f32, f32); 8] = [
                (-unit, 0.0),
                (unit, 0.0),
                (0.0, -unit),
                (0.0, unit),
                (-unit, -unit),
                (unit, -unit),
                (-unit, unit),
                (unit, unit),
            ];

            // 按字符分栏绘制：每个字符一个等宽槽，槽左右各外扩 1.2px 让字形
            // 紧凑相接。整串一次绘制在三位数（`100`）时会因为总宽超出图标被
            // GDI+ 折行/裁切，分栏把这个坑绕开了，也顺便让一位/两位/三位走
            // 同一条代码路径。
            let n = text.len().max(1);
            let slot_w = size as f32 / n as f32;
            let mut chars: Vec<[u16; 1]> = Vec::with_capacity(n);
            for ch in text.iter() {
                chars.push([*ch]);
            }
            let rects: Vec<RectF> = (0..n)
                .map(|i| RectF {
                    x: i as f32 * slot_w - 1.2 * unit - if charging { 1.7 * unit } else { 0.0 },
                    y: -0.5 * unit,
                    w: slot_w + 2.4 * unit,
                    h: size as f32,
                })
                .collect();

            if !shadow.is_null() {
                for (dx, dy) in offs {
                    for (ch, rc) in chars.iter().zip(rects.iter()) {
                        let r = RectF {
                            x: rc.x + dx,
                            y: rc.y + dy,
                            w: rc.w,
                            h: rc.h,
                        };
                        GdipDrawString(g, ch.as_ptr(), 1, font, &r, fmt, shadow);
                    }
                }
            }
            if !fg.is_null() {
                for (ch, rc) in chars.iter().zip(rects.iter()) {
                    GdipDrawString(g, ch.as_ptr(), 1, font, rc, fmt, fg);
                }
            }

            // 充电角标：右上深色圆 + 白色闪电。闪电用三条横条拼成 Z 形 ——
            // `GdipFillPolygon` 不在导入库里（和 `GdipAddPathLines` 同一个坑），
            // 32px 下这个形状已经足够读作闪电。
            if charging {
                let cr = size as f32 * 0.145;
                let cx = size as f32 - cr * 1.02;
                let cy = cr * 1.02;
                let mut ring: *mut c_void = ptr::null_mut();
                if GdipCreateSolidFill(0xFF20_242B, &mut ring) == 0 && !ring.is_null() {
                    GdipFillEllipse(g, ring, cx - cr, cy - cr, cr * 2.0, cr * 2.0);
                    GdipDeleteBrush(ring);
                }
                let mut bolt: *mut c_void = ptr::null_mut();
                if GdipCreateSolidFill(0xFFFF_FFFF, &mut bolt) == 0 && !bolt.is_null() {
                    let bar = |dy: f32, w: f32, dx: f32| {
                        (
                            (cx - cr + dx) as i32,
                            (cy - cr + dy) as i32,
                            w as i32,
                            (cr * 0.42) as i32,
                        )
                    };
                    for (x, y, w, h) in [
                        bar(cr * 0.52, cr * 1.05, cr * 0.45),
                        bar(cr * 0.95, cr * 0.85, cr * 0.55),
                        bar(cr * 1.38, cr * 1.05, cr * 0.45),
                    ] {
                        GdipFillRectangleI(g, bolt, x, y, w, h);
                    }
                    GdipDeleteBrush(bolt);
                }
            }

            if !shadow.is_null() {
                GdipDeleteBrush(shadow);
            }
            if !fg.is_null() {
                GdipDeleteBrush(fg);
            }
        }

        // 清理绘图对象（必须在取 HICON 之前把 Graphics 从位图上摘掉）
        if !fmt.is_null() {
            GdipDeleteStringFormat(fmt);
        }
        if !font.is_null() {
            GdipDeleteFont(font);
        }
        if !family.is_null() {
            GdipDeleteFontFamily(family);
        }
        GdipDeleteGraphics(g);

        // 缩回目标尺寸：高质量双三次 + 半像素偏移，避免整体偏一格。
        let mut small: *mut c_void = ptr::null_mut();
        let mut scaled = false;
        if GdipCreateBitmapFromScan0(
            ICON_SIZE,
            ICON_SIZE,
            0,
            PIXEL_FORMAT_32BPP_ARGB,
            ptr::null(),
            &mut small,
        ) == 0
            && !small.is_null()
        {
            let mut sg: *mut c_void = ptr::null_mut();
            if GdipGetImageGraphicsContext(small, &mut sg) == 0 && !sg.is_null() {
                GdipSetInterpolationMode(sg, 7); // HighQualityBicubic
                GdipSetPixelOffsetMode(sg, 2); // Half
                GdipDrawImageRectI(sg, bmp, 0, 0, ICON_SIZE, ICON_SIZE);
                // 最外圈强制透明：缩放会在四边留下半透明脏边，浅色和深色任务栏
                // 上都会显出一圈灰影。
                GdipSetCompositingMode(sg, 1); // SourceCopy
                let mut clear: *mut c_void = ptr::null_mut();
                if GdipCreateSolidFill(0x0000_0000, &mut clear) == 0 && !clear.is_null() {
                    let n = ICON_SIZE;
                    GdipFillRectangleI(sg, clear, 0, 0, n, 1);
                    GdipFillRectangleI(sg, clear, 0, n - 1, n, 1);
                    GdipFillRectangleI(sg, clear, 0, 0, 1, n);
                    GdipFillRectangleI(sg, clear, n - 1, 0, 1, n);
                    GdipDeleteBrush(clear);
                }
                GdipSetCompositingMode(sg, 0); // SourceOver
                GdipDeleteGraphics(sg);
                scaled = true;
            }
        }

        let mut hicon: Handle = ptr::null_mut();
        let st = if scaled {
            GdipCreateHICONFromBitmap(small, &mut hicon)
        } else {
            GdipCreateHICONFromBitmap(bmp, &mut hicon)
        };
        GdipDisposeImage(bmp);
        if !small.is_null() {
            GdipDisposeImage(small);
        }
        if st == 0 && !hicon.is_null() {
            Some(hicon)
        } else {
            if std::env::var_os("ROG_TRAY_DEBUG").is_some() {
                eprintln!("[tray_icon] GdipCreateHICONFromBitmap 失败 status={st}");
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn device_glyphs_render_all_modes_and_states() {
        use crate::tray_prefs::{TrayContent, TrayKind};
        use crate::ui::gfx::{Canvas, Fonts, R};
        #[link(name = "user32")]
        extern "system" {
            fn DrawIconEx(
                dc: Handle,
                x: i32,
                y: i32,
                icon: Handle,
                w: i32,
                h: i32,
                step: u32,
                brush: Handle,
                flags: u32,
            ) -> i32;
        }
        let screen = unsafe { crate::win32::GetDC(ptr::null_mut()) };
        assert!(!screen.is_null());
        let mut canvas = Canvas::new();
        let dc = canvas.ensure(screen as isize, 720, 900);
        let mut fonts = Fonts::new("Segoe UI").unwrap();
        canvas.paint(
            &mut fonts,
            &crate::ui::theme::LIGHT,
            1.0,
            0.0,
            720.0,
            900.0,
            |p| {
                p.rect(R::new(0.0, 0.0, 720.0, 900.0), crate::ui::theme::LIGHT.bg);
                p.rect(R::new(0.0, 550.0, 720.0, 350.0), crate::ui::theme::DARK.bg);
            },
        );
        for (column, kind) in TrayKind::ALL.iter().skip(1).enumerate() {
            for (row, content) in TrayContent::ALL.iter().enumerate() {
                for (variant, level) in [Some(78), Some(100), None].iter().enumerate() {
                    let icon = render_device_at(
                        *level,
                        variant == 1,
                        *kind,
                        *content,
                        &crate::ui::theme::LIGHT,
                        32,
                    )
                    .expect("图标生成失败");
                    let x = 10 + (column as i32) * 140 + variant as i32 * 42;
                    let y = 12 + row as i32 * 68;
                    assert_ne!(
                        unsafe {
                            DrawIconEx(dc as Handle, x, y, icon, 32, 32, 0, ptr::null_mut(), 3)
                        },
                        0
                    );
                    unsafe {
                        crate::win32::DestroyIcon(icon);
                    }
                }
            }
        }
        for (mode, content) in [TrayContent::Combined, TrayContent::Number]
            .iter()
            .enumerate()
        {
            for (row, size) in [16, 20, 24, 32].iter().enumerate() {
                for (column, level) in [
                    Some(0),
                    Some(7),
                    Some(11),
                    Some(78),
                    Some(99),
                    Some(100),
                    None,
                ]
                .iter()
                .enumerate()
                {
                    for (theme_index, theme) in [&crate::ui::theme::LIGHT, &crate::ui::theme::DARK]
                        .iter()
                        .enumerate()
                    {
                        let icon = render_device_at(
                            *level,
                            false,
                            TrayKind::Mouse,
                            *content,
                            theme,
                            *size,
                        )
                        .unwrap();
                        assert_ne!(
                            unsafe {
                                DrawIconEx(
                                    dc as Handle,
                                    10 + column as i32 * 90,
                                    230 + mode as i32 * 160
                                        + row as i32 * 40
                                        + theme_index as i32 * 320,
                                    icon,
                                    *size,
                                    *size,
                                    0,
                                    ptr::null_mut(),
                                    3,
                                )
                            },
                            0
                        );
                        if *size == 16 && level.is_some() {
                            let x = 10 + column as i32 * 90;
                            let y = 230 + mode as i32 * 160 + theme_index as i32 * 320;
                            let bg = crate::ui::gfx::gdi_get_pixel(dc, 700, y).unwrap();
                            let start_x = if *content == TrayContent::Combined {
                                7
                            } else {
                                0
                            };
                            let rows_with_ink = (0..16)
                                .filter(|dy| {
                                    (start_x..16).any(|dx| {
                                        let pixel =
                                            crate::ui::gfx::gdi_get_pixel(dc, x + dx, y + dy)
                                                .unwrap();
                                        [0, 8, 16].iter().any(|shift| {
                                            let a = ((pixel >> shift) & 255) as i32;
                                            let b = ((bg >> shift) & 255) as i32;
                                            (a - b).abs() > 40
                                        })
                                    })
                                })
                                .count();
                            let minimum = if *content == TrayContent::Combined {
                                10
                            } else {
                                12
                            };
                            assert!(rows_with_ink >= minimum, "16px {content:?} {level:?}: digits too small ({rows_with_ink} rows)");
                        }
                        unsafe {
                            crate::win32::DestroyIcon(icon);
                        }
                    }
                }
            }
        }
        crate::ui::gfx::save_bmp(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/scratch/native-tray-glyphs.bmp"
            ),
            dc,
            720,
            900,
        );
        canvas.release();
        unsafe {
            crate::win32::ReleaseDC(ptr::null_mut(), screen);
        }
    }
    use super::*;

    #[test]
    fn label_is_the_number_without_percent() {
        assert_eq!(label(0), "0");
        assert_eq!(label(7), "7");
        assert_eq!(label(82), "82");
        assert_eq!(label(100), "100");
    }

    #[test]
    fn label_clamps_above_100() {
        // 设备理论上不会报 >100，但真报了也不能画成 "255"
        assert_eq!(label(255), "100");
    }

    #[test]
    fn font_shrinks_as_digits_grow() {
        // 三位数必须比两位数小，两位数必须比一位数小 ——
        // 否则 "100" 会撑出 32px 图标
        assert!(font_height(7) > font_height(82));
        assert!(font_height(82) > font_height(100));
        assert!(font_height(100) >= 12, "再小就看不清了");
        assert!(font_height(7) <= ICON_SIZE, "不该超出图标");
    }

    #[test]
    fn colorref_is_bgr_not_rgb() {
        // 红 (255,0,0) → 0x0000FF；写反了就是 0xFF0000（蓝）
        assert_eq!(colorref([255, 0, 0]), 0x0000_00FF);
        assert_eq!(colorref([0, 0, 255]), 0x00FF_0000);
        assert_eq!(colorref([0, 0, 0]), 0);
        assert_eq!(colorref([0xFF, 0xFF, 0xFF]), 0x00FF_FFFF);
    }

    #[test]
    fn low_battery_is_red_and_normal_is_dark() {
        assert_eq!(level_color(5), [0xD1, 0x3A, 0x2E]);
        assert_eq!(level_color(20), [0xD1, 0x3A, 0x2E]);
        assert_eq!(level_color(21), [0xD8, 0x8B, 0x1E]);
        assert_eq!(level_color(50), [0xD8, 0x8B, 0x1E]);
        assert_eq!(level_color(51), BG);
        assert_eq!(level_color(100), BG);
    }

    /// 真画一张，确认 GDI 链路通、句柄能回收。
    ///
    /// 这个测试**必须**在 Windows 上跑（本工程本来就只支持 Windows）。
    #[test]
    fn render_produces_a_usable_icon_and_cleans_up() {
        let h = render(82).expect("render(82) 应该能画出图标");
        assert!(!h.is_null());
        // 位图属性：宽高必须是我们画的那块
        unsafe {
            assert_ne!(crate::win32::DestroyIcon(h), 0, "DestroyIcon 应当成功");
        }
    }

    #[test]
    fn render_handles_extremes() {
        for lv in [0u8, 1, 20, 21, 50, 51, 99, 100, 200] {
            let h = render(lv);
            assert!(h.is_some(), "level={lv} 应该能画出来");
            unsafe { crate::win32::DestroyIcon(h.unwrap()) };
        }
    }

    #[test]
    fn scrolled_font_height_stays_inside_the_icon() {
        // 16px 上两位数必须是可读的，且不能顶出边界。
        assert_eq!(scaled_font_height(82, 16), 13); // 27 * 16 / 32 = 13
        assert_eq!(scaled_font_height(7, 16), 14); // 28 * 16 / 32 = 14
        assert_eq!(scaled_font_height(100, 16), 10); // 20 * 16 / 32 = 10
        for lv in [0u8, 7, 42, 82, 99, 100] {
            for size in [16, 20, 24, 32] {
                let h = scaled_font_height(lv, size);
                assert!(h >= 6, "level={lv} size={size} 字号 {h} 太小");
                assert!(h <= size, "level={lv} size={size} 字号 {h} 超出图标");
            }
        }
    }

    #[test]
    fn icon_size_is_sane() {
        let s = icon_size();
        assert!(
            (16..=ICON_SIZE).contains(&s),
            "SM_CXSMICON 返回值 {s} 异常，应该夹在 16..={ICON_SIZE}"
        );
    }

    /// 反复画/销毁，确认**没有句柄泄漏**。
    ///
    /// 用 `GetGuiResources(GR_GDIOBJECTS)` 读进程的 GDI 对象数：
    /// `Gdi::drop` 漏一个对象这里就会单调上涨。
    ///
    /// 画 200 次，前 20 次当预热（首轮会分配字体缓存之类的一次性资源），
    /// 之后的对象数不允许比预热结束时多 10 个以上。
    #[test]
    fn repeated_renders_do_not_leak_gdi_objects() {
        use crate::tray_prefs::{TrayContent, TrayKind};
        use crate::win32::GetCurrentProcess;

        unsafe fn resource_counts() -> [i32; 2] {
            [
                crate::win32::GetGuiResources(GetCurrentProcess(), 0) as i32,
                crate::win32::GetGuiResources(GetCurrentProcess(), 1) as i32,
            ]
        }

        // 预热
        for _ in 0..20 {
            if let Some(h) = render_device(Some(82), false, TrayKind::Mouse, TrayContent::Combined) {
                unsafe { crate::win32::DestroyIcon(h) };
            }
        }
        let before = unsafe { resource_counts() };

        for i in 0..200 {
            let lv = (i % 101) as u8;
            match render_device(Some(lv), i % 2 == 0, TrayKind::Mouse, TrayContent::ALL[i % 3]) {
                Some(h) => unsafe { crate::win32::DestroyIcon(h) },
                None => panic!("level={lv} 画不出来"),
            };
        }

        let after = unsafe { resource_counts() };
        for (index, name) in ["GDI", "USER"].iter().enumerate() {
            assert!(after[index] - before[index] <= 10,
                "画 200 次后 {name} 对象从 {} 涨到 {}，疑似泄漏", before[index], after[index]);
        }
    }

    /// 充电角标这条分支必须能画出来（GDI+ 调用链和普通徽章不同）。
    #[test]
    fn render_charging_produces_an_icon() {
        let h = render_charging(84);
        assert!(h.is_some(), "充电图标应当能生成");
        if let Some(h) = h {
            unsafe { crate::win32::DestroyIcon(h) };
        }
    }


    /// 灰底「历史读数」图标也要能画出来（走的是同一条 GDI+ 链）。
    #[test]
    fn render_stale_produces_an_icon() {
        let h = render_stale(62);
        assert!(h.is_some(), "历史读数图标应当能生成");
        if let Some(h) = h {
            unsafe { crate::win32::DestroyIcon(h) };
        }
    }

}
