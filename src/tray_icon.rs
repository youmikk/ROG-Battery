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
                    R::new(1.0, 4.0, 30.0, 24.0)
                };
                if level.is_some() {
                    p.icon_text(
                        &text,
                        rect,
                        if combined { 23.0 } else { 28.0 },
                        ink,
                        combined,
                    );
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

/// 图标上要写的文字。
///
/// 只放数字（不带 `%`）：32x32 里 `100%` 四个字形会缩到不可读。
/// 100 写 `99` 会撒谎 —— 所以保留三位，只是把字号按位数缩小。
pub fn label(level: u8) -> String {
    level.min(100).to_string()
}

/// 把 `[r, g, b]` 转 `COLORREF`（`0x00BBGGRR`）。
///
/// 顺序写反是最常见的 GDI 错误：红色 `(255,0,0)` 写成 `0x0000FF` 会变蓝。
fn colorref(c: [u8; 3]) -> u32 {
    (c[0] as u32) | ((c[1] as u32) << 8) | ((c[2] as u32) << 16)
}

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

pub fn render_unknown() -> Option<Handle> {
    let is_light = crate::autostart::system_uses_light_theme();
    let c = if is_light {
        [0x5A, 0x5A, 0x5A]
    } else {
        [0xD8, 0xD8, 0xD8]
    };
    render_glyph("—", (ICON_SIZE as f32 * 0.46) as i32, c, is_light, None, false)
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
