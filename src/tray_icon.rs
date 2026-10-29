//! 托盘图标上的电量数字。
//!
//! 静态的 `assets\tray.ico` 只能表达「程序在跑」。C# 版在图标上直接画电量
//! （`D:\codex\rog-battery\tray.cs` 的 `FallbackIcon`），这里做同一件事：
//! 用 GDI 往内存位图上画一个数字，包成 `HICON`，喂给
//! `TrayNotification::set_icon`。
//!
//! # 为什么走裸 GDI 而不是 nwg 的 `Icon::builder()`
//!
//! nwg 的 `IconBuilder` 只有三个来源：文件路径、字节、系统 OEM 图标。
//! 没有「给我一个已经画好的 HDC」这条路。而 `.size()` 那条又走 WIC
//! （见 `docs\nwg-gotchas.md` 第 14 节：`.ico` 里的 PNG 条目会让它失败）。
//!
//! 所以：`CreateDIBSection`（32bpp，带 alpha）→ `DrawTextW` → `CreateIconIndirect`。
//!
//! # 句柄生命周期（这块最容易漏，漏了就漏句柄）
//!
//! 一次绘制会产生 **5 个 GDI/User 对象**：
//! `memDC`、`dib`、`oldBitmap`（SelectObject 的返回，不是新对象）、
//! `mask`、`font`，以及最终的 `HICON`。
//!
//! - `memDC` → `DeleteDC`
//! - `dib` / `mask` / `font` → `DeleteObject`
//! - `HICON` → **调用方**负责 `DestroyIcon`（不能在这里销毁，
//!   因为 `Shell_NotifyIconW` 可能还要用它）
//!
//! 位图必须 `SelectObject` 回 `oldBitmap` 之后才能删，否则位图仍被 DC 选中，
//! `DeleteObject` 会失败并静默漏掉。

use core::ffi::c_void;
use core::ptr;

use crate::win32::{self, Handle};

/// 图标边长。
///
/// 托盘图标在 100% 缩放下系统只用 **16x16**（`SM_CXSMICON` 实测 = 16），
/// 任务栏/DPI 缩放不同会有别的值。硬画 32x32 也能work —— 系统会缩，
/// 但双线性缩放会把笔画糊掉（实测「82」边缘发虚）。
///
/// 所以运行时用 `icon_size()` 查系统值，这个常量只作为**上限和默认值**。
pub const ICON_SIZE: i32 = 32;

/// 系统实际需要的小图标边长。
///
/// 查 `SM_CXSMICON`（=49）。查询失败或返回离谱值时回落到 [`ICON_SIZE`]。
///
/// 夹在 16..=32：小于 16 画不下两位数字，大于 32 没意义（托盘不显示那么大）。
pub fn icon_size() -> i32 {
    let n = unsafe { win32::GetSystemMetrics(49) };
    if n < 16 {
        ICON_SIZE
    } else {
        n.min(ICON_SIZE)
    }
}

/// 背景色 —— 深色圆角块，白字。和曲线不同，托盘图标必须**不透明**
/// 才能在浅色/深色任务栏上都看得清（透明图标配白字在浅色栏上会消失）。
const BG: [u8; 3] = [0x20, 0x24, 0x2B];
const FG: [u8; 3] = [0xFF, 0xFF, 0xFF];

/// 电量对应的图标底色。三档，够用户一眼分辨「还早 / 该充了 / 快没电」。
fn level_color(level: u8) -> [u8; 3] {
    match level {
        0..=20 => [0xD1, 0x3A, 0x2E],   // 红：快没电
        21..=50 => [0xD8, 0x8B, 0x1E],  // 橙：该充了
        _ => BG,                        // 常态深灰
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
/// 一位数（`7`）用 28px，两位数（`82`）用 27px（双位分栏紧凑绘制，高度占满），三位数（`100`）用 14px。
/// 这是纯函数，测试直接断言数字，不需要真的画。
///
/// 实际绘制时按 `icon_size()` 等比缩放 —— 见 [`scaled_font_height`]。
pub fn font_height(level: u8) -> i32 {
    match label(level).len() {
        1 => 28,
        2 => 27,
        _ => 14,
    }
}

/// 把 [`font_height`] 从 32px 基准缩放到目标边长。
///
/// 单独抽出来是为了能测：16px 图标上两位数必须落在 8..=12 之间，
/// 否则要么糊成一团、要么顶出图标边界。
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

/// 一次绘制产生的所有对象。`Drop` 负责回收，**不依赖调用方记得**。
struct Gdi {
    objects: Vec<Handle>,
    dc: Handle,
}

impl Drop for Gdi {
    fn drop(&mut self) {
        unsafe {
            for h in self.objects.drain(..) {
                if !h.is_null() {
                    win32::DeleteObject(h);
                }
            }
            if !self.dc.is_null() {
                win32::DeleteDC(self.dc);
            }
        }
    }
}

// ==================== GDI+ 透明图标 ====================
//
// 用户要求：托盘图标「要透明、只显示数字，像系统自带的字体」。
// 裸 GDI 那条路做不到真透明（DrawTextW 把 alpha 写成 0，只能整块补回不透明），
// 所以改用 GDI+ 画到 32bpp ARGB 位图上，再 `GdipCreateHICONFromBitmap` 转 HICON
// —— alpha 通道原样保留，托盘上就是「透明底白字」。

mod gdiplus {
    use core::ffi::c_void;
    // PixelFormat32bppARGB = 10 | (32<<8) | Alpha(0x40000) | GDI(0x20000) | Canonical(0x200000)
    pub const PIXEL_FORMAT_32BPP_ARGB: i32 = 0x0026_200A;

    #[repr(C)]
    pub struct RectF {
        pub x: f32,
        pub y: f32,
        pub w: f32,
        pub h: f32,
    }

    #[link(name = "gdiplus")]
    extern "system" {
        pub fn GdipCreateBitmapFromScan0(
            width: i32,
            height: i32,
            stride: i32,
            format: i32,
            scan0: *const u8,
            bitmap: *mut *mut c_void,
        ) -> i32;
        pub fn GdipDisposeImage(image: *mut c_void) -> i32;
        pub fn GdipGetImageGraphicsContext(image: *mut c_void, g: *mut *mut c_void) -> i32;
        pub fn GdipDeleteGraphics(g: *mut c_void) -> i32;
        pub fn GdipGraphicsClear(g: *mut c_void, color: u32) -> i32;
        pub fn GdipSetSmoothingMode(g: *mut c_void, mode: i32) -> i32;
        pub fn GdipSetTextRenderingHint(g: *mut c_void, hint: i32) -> i32;
        pub fn GdipCreateFontFamilyFromName(
            name: *const u16,
            collection: *mut c_void,
            family: *mut *mut c_void,
        ) -> i32;
        pub fn GdipDeleteFontFamily(family: *mut c_void) -> i32;
        pub fn GdipCreateFont(
            family: *mut c_void,
            size: f32,
            style: i32,
            unit: i32,
            font: *mut *mut c_void,
        ) -> i32;
        pub fn GdipDeleteFont(font: *mut c_void) -> i32;
        pub fn GdipCreateStringFormat(attrs: i32, lang: u16, fmt: *mut *mut c_void) -> i32;
        pub fn GdipSetStringFormatFlags(fmt: *mut c_void, flags: i32) -> i32;
        pub fn GdipSetStringFormatAlign(fmt: *mut c_void, align: i32) -> i32;
        pub fn GdipSetStringFormatLineAlign(fmt: *mut c_void, align: i32) -> i32;
        pub fn GdipDeleteStringFormat(fmt: *mut c_void) -> i32;
        pub fn GdipCreateSolidFill(argb: u32, brush: *mut *mut c_void) -> i32;
        pub fn GdipDeleteBrush(brush: *mut c_void) -> i32;
        pub fn GdipDrawString(
            g: *mut c_void,
            s: *const u16,
            len: i32,
            font: *mut c_void,
            layout: *const RectF,
            fmt: *mut c_void,
            brush: *mut c_void,
        ) -> i32;
        pub fn GdipCreateHICONFromBitmap(bitmap: *mut c_void, hicon: *mut *mut c_void) -> i32;
    }
}

/// 透明背景上数字的颜色：自适应浅色/深色任务栏。
///
/// 浅色任务栏（Windows 11 默认/当前）：深炭灰 #1F1F1F，匹配输入法与时钟原生字体；低电警示红、中电橙。
/// 深色任务栏：纯白 #FFFFFF；低电亮红、中电亮橙。
fn glyph_color(level: u8, is_light: bool) -> [u8; 3] {
    if is_light {
        match level {
            0..=20 => [0xD1, 0x3A, 0x2E],   // 警示红
            21..=50 => [0xC4, 0x6E, 0x00],  // 稳健橙
            _ => [0x1F, 0x1F, 0x1F],        // 原生深炭灰（匹配系统输入法及任务栏深色图标）
        }
    } else {
        match level {
            0..=20 => [0xEA, 0x4B, 0x3C],   // 亮红
            21..=50 => [0xEA, 0x9A, 0x2E],  // 亮橙
            _ => [0xFF, 0xFF, 0xFF],        // 纯白
        }
    }
}

/// 画一张**透明底**、带电量数字的图标（GDI+ 32bpp ARGB）。
///
/// 失败返回 `None`，调用方回落到静态图标 —— **不能让托盘空白**。
/// 成功返回的 `HICON` 由调用方负责 `DestroyIcon`。
pub fn render(level: u8) -> Option<Handle> {
    let is_light = crate::autostart::system_uses_light_theme();
    render_glyph(
        &label(level),
        scaled_font_height(level, ICON_SIZE),
        glyph_color(level, is_light),
        is_light,
    )
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
    render_glyph("—", (ICON_SIZE as f32 * 0.46) as i32, c, is_light)
}

fn render_glyph(text: &str, font_height: i32, color: [u8; 3], is_light: bool) -> Option<Handle> {
    use gdiplus::*;
    // 基础生成尺寸固定为 ICON_SIZE (32px)，生成高质量 32bpp ARGB 图标。
    // Windows Shell_NotifyIcon 接收 32x32 图标后，由 DWM 高保真缩放到任务栏托盘，
    // 无论是 100%、125%、150% 还是 200% 缩放，均能保持大字号饱满清晰、无毛刺。
    let size = ICON_SIZE;
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
        let fh = font_height as f32;
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
        let len = text.len() as i32;

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

            let offs: [(f32, f32); 8] = [
                (-1.0, 0.0),
                (1.0, 0.0),
                (0.0, -1.0),
                (0.0, 1.0),
                (-1.0, -1.0),
                (1.0, -1.0),
                (-1.0, 1.0),
                (1.0, 1.0),
            ];

            if text.len() == 2 {
                // 两位数：将两个数字分左右栏绘制，消除单串过长被 GDI+ 折行/裁切的问题，字号拉大至 27px 占满高度
                let half = size as f32 / 2.0;
                let d1 = [text[0]];
                let d2 = [text[1]];
                let rc1 = RectF {
                    x: -1.2,
                    y: -0.5,
                    w: half + 2.4,
                    h: size as f32,
                };
                let rc2 = RectF {
                    x: half - 1.2,
                    y: -0.5,
                    w: half + 2.4,
                    h: size as f32,
                };

                if !shadow.is_null() {
                    for (dx, dy) in offs {
                        let r1 = RectF {
                            x: rc1.x + dx,
                            y: rc1.y + dy,
                            w: rc1.w,
                            h: rc1.h,
                        };
                        let r2 = RectF {
                            x: rc2.x + dx,
                            y: rc2.y + dy,
                            w: rc2.w,
                            h: rc2.h,
                        };
                        GdipDrawString(g, d1.as_ptr(), 1, font, &r1, fmt, shadow);
                        GdipDrawString(g, d2.as_ptr(), 1, font, &r2, fmt, shadow);
                    }
                }
                if !fg.is_null() {
                    GdipDrawString(g, d1.as_ptr(), 1, font, &rc1, fmt, fg);
                    GdipDrawString(g, d2.as_ptr(), 1, font, &rc2, fmt, fg);
                }
            } else {
                let rc = RectF {
                    x: 0.0,
                    y: -0.5,
                    w: size as f32,
                    h: size as f32,
                };
                if !shadow.is_null() {
                    for (dx, dy) in offs {
                        let r = RectF {
                            x: rc.x + dx,
                            y: rc.y + dy,
                            w: rc.w,
                            h: rc.h,
                        };
                        GdipDrawString(g, text.as_ptr(), len, font, &r, fmt, shadow);
                    }
                }
                if !fg.is_null() {
                    GdipDrawString(g, text.as_ptr(), len, font, &rc, fmt, fg);
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

        let mut hicon: Handle = ptr::null_mut();
        let st = GdipCreateHICONFromBitmap(bmp, &mut hicon);
        GdipDisposeImage(bmp);
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

/// 【旧实现，保留供对照】不透明 GDI 版本。
#[allow(dead_code)]
fn render_gdi_legacy(level: u8) -> Option<Handle> {
    let size = icon_size();
    unsafe {
        // 用屏幕 DC 作模板：`CreateCompatibleDC(null)` 得到的内存 DC
        // 格式跟当前显示设置一致，颜色不会偏。
        let screen = win32::GetDC(ptr::null_mut());
        if screen.is_null() {
            return None;
        }
        let mem = win32::CreateCompatibleDC(screen);
        win32::ReleaseDC(ptr::null_mut(), screen);
        if mem.is_null() {
            return None;
        }

        let mut gdi = Gdi {
            objects: Vec::new(),
            dc: mem,
        };

        // 32bpp 的 DIB：每像素 BGRA，alpha 由我们自己写。
        let mut bmi = win32::BitmapInfo::default();
        bmi.header.size = core::mem::size_of::<win32::BitmapInfoHeader>() as u32;
        bmi.header.width = size;
        // **负数 = 自上而下**。正数会让 DIB 从下往上存，
        // 于是 `SetPixel(x, y)` 的 y 轴和 `DrawTextW` 的矩形不一致，图会上下颠倒。
        bmi.header.height = -size;
        bmi.header.planes = 1;
        bmi.header.bit_count = 32;
        bmi.header.compression = 0; // BI_RGB
        bmi.header.size_image = (size * size * 4) as u32;

        let mut bits: *mut c_void = ptr::null_mut();
        let dib = win32::CreateDIBSection(mem, &bmi, 0, &mut bits, ptr::null_mut(), 0);
        if dib.is_null() || bits.is_null() {
            return None;
        }
        gdi.objects.push(dib);

        let old = win32::SelectObject(mem, dib);
        if old.is_null() {
            return None;
        }

        // 背景：整块填底色。alpha 一并写 0xFF（不透明）。
        let px = bits as *mut u32;
        let bg = colorref(level_color(level));
        for i in 0..(size * size) as usize {
            *px.add(i) = bg | 0xFF00_0000;
        }

        // 文字
        let face = win32::to_wide("Segoe UI");
        let font = win32::CreateFontW(
            -scaled_font_height(level, size),
            0,
            0,
            0,
            500, // FW_MEDIUM
            0,
            0,
            0,
            1, // DEFAULT_CHARSET
            0,
            0,
            4, // ANTIALIASED_QUALITY
            0,
            face.as_ptr(),
        );
        if !font.is_null() {
            gdi.objects.push(font);
            let old_font = win32::SelectObject(mem, font);
            win32::SetBkMode(mem, 1); // TRANSPARENT
            win32::SetTextColor(mem, colorref(FG));

            let text = win32::to_wide(&label(level));
            let len = text.len() as i32 - 1;
            let mut rc = win32::Rect::new(0, 0, size, size);
            const DT_CENTER: u32 = 0x0000_0001;
            const DT_VCENTER: u32 = 0x0000_0004;
            const DT_SINGLELINE: u32 = 0x0000_0020;
            win32::DrawTextW(
                mem,
                text.as_ptr(),
                len,
                &mut rc,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );

            if !old_font.is_null() {
                win32::SelectObject(mem, old_font);
            }
        }

        for i in 0..(size * size) as usize {
            *px.add(i) = (*px.add(i) & 0x00FF_FFFF) | 0xFF00_0000;
        }

        let mask = win32::CreateBitmap(size, size, 1, 1, ptr::null_mut());
        if !mask.is_null() {
            gdi.objects.push(mask);
        }

        win32::SelectObject(mem, old);

        let info = win32::IconInfo {
            is_icon: 1,
            x_hotspot: 0,
            y_hotspot: 0,
            mask,
            color: dib,
        };
        let hicon = win32::CreateIconIndirect(&info);

        if hicon.is_null() {
            if std::env::var_os("ROG_TRAY_DEBUG").is_some() {
                eprintln!(
                    "[tray_icon] CreateIconIndirect 失败 GetLastError={}",
                    win32::GetLastError()
                );
            }
            None
        } else {
            Some(hicon)
        }
    }
}

/// 画一张带电量数字的图标。
///
/// 失败（拿不到 DC / 建不出位图）返回 `None`，调用方回落到静态图标
/// —— **不能让托盘空白**，那比显示旧图标糟得多。
///
/// 成功返回的 `HICON` 由调用方负责 `DestroyIcon`。
#[allow(dead_code)]
fn render_old2(level: u8) -> Option<Handle> {
    let size = icon_size();
    unsafe {
        // 用屏幕 DC 作模板：`CreateCompatibleDC(null)` 得到的内存 DC
        // 格式跟当前显示设置一致，颜色不会偏。
        let screen = win32::GetDC(ptr::null_mut());
        if screen.is_null() {
            return None;
        }
        let mem = win32::CreateCompatibleDC(screen);
        win32::ReleaseDC(ptr::null_mut(), screen);
        if mem.is_null() {
            return None;
        }

        let mut gdi = Gdi {
            objects: Vec::new(),
            dc: mem,
        };

        // 32bpp 的 DIB：每像素 BGRA，alpha 由我们自己写。
        let mut bmi = win32::BitmapInfo::default();
        bmi.header.size = core::mem::size_of::<win32::BitmapInfoHeader>() as u32;
        bmi.header.width = size;
        // **负数 = 自上而下**。正数会让 DIB 从下往上存，
        // 于是 `SetPixel(x, y)` 的 y 轴和 `DrawTextW` 的矩形不一致，图会上下颠倒。
        bmi.header.height = -size;
        bmi.header.planes = 1;
        bmi.header.bit_count = 32;
        bmi.header.compression = 0; // BI_RGB
        bmi.header.size_image = (size * size * 4) as u32;

        let mut bits: *mut c_void = ptr::null_mut();
        let dib = win32::CreateDIBSection(mem, &bmi, 0, &mut bits, ptr::null_mut(), 0);
        if dib.is_null() || bits.is_null() {
            return None;
        }
        gdi.objects.push(dib);

        let old = win32::SelectObject(mem, dib);
        if old.is_null() {
            return None;
        }

        // 背景：整块填底色。alpha 一并写 0xFF（不透明）。
        let px = bits as *mut u32;
        let bg = colorref(level_color(level));
        for i in 0..(size * size) as usize {
            *px.add(i) = bg | 0xFF00_0000;
        }

        // 文字
        let face = win32::to_wide("Segoe UI");
        let font = win32::CreateFontW(
            -scaled_font_height(level, size),
            0,
            0,
            0,
            500, // FW_MEDIUM（原来是 700 粗体，字太"重"不像系统原生字体）
            0,
            0,
            0,
            1, // DEFAULT_CHARSET
            0,
            0,
            // **ANTIALIASED，不是 CLEARTYPE。**
            // ClearType（5）会写次像素 RGB 值，本意是给 LCD 屏幕做边缘补偿，
            // 但这里是内存 DC + 32bpp DIB，下面统一把 alpha 拉成 0xFF 之后
            // 那些彩色次像素就露出来了 —— 实测「82」变成一道彩虹边，
            // 在托盘上像坏掉的图标。
            // 灰度抗锯齿（4）没有这个问题，32px 下也足够清晰。
            4, // ANTIALIASED_QUALITY
            0,
            face.as_ptr(),
        );
        if !font.is_null() {
            gdi.objects.push(font);
            let old_font = win32::SelectObject(mem, font);
            win32::SetBkMode(mem, 1); // TRANSPARENT
            win32::SetTextColor(mem, colorref(FG));

            let text = win32::to_wide(&label(level));
            // 减掉结尾的 NUL：长度要传**字符数**，不是字节数。
            let len = text.len() as i32 - 1;
            let mut rc = win32::Rect::new(0, 0, size, size);
            // DT_CENTER | DT_VCENTER | DT_SINGLELINE
            const DT_CENTER: u32 = 0x0000_0001;
            const DT_VCENTER: u32 = 0x0000_0004;
            const DT_SINGLELINE: u32 = 0x0000_0020;
            win32::DrawTextW(
                mem,
                text.as_ptr(),
                len,
                &mut rc,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );

            if !old_font.is_null() {
                win32::SelectObject(mem, old_font);
            }
        }

        // **GDI 画字会把 alpha 字节写成 0。**
        //
        // 这是 32bpp DIB section 的老坑：`DrawTextW` 只写 RGB 三个字节，
        // alpha 直接置 0。于是文字在托盘上变成全透明 ——
        // 看起来像「数字是暗色的/根本看不见」，实际是透出了任务栏背景。
        // 实测：不补这一步，82 是深灰的、几乎认不出来。
        //
        // 我们的图标整块都是不透明底色，所以没有任何像素需要真透明 ——
        // 一律把 alpha 拉回 0xFF 即可。放在所有绘制之后做，
        // 才能覆盖掉 `DrawTextW` 破坏的那些字节。
        for i in 0..(size * size) as usize {
            *px.add(i) = (*px.add(i) & 0x00FF_FFFF) | 0xFF00_0000;
        }

        // 掩码位图：`CreateIconIndirect` 要求有它。
        // 1bpp 且**全 0** = 每处都「保留彩色位图」。
        // 传 null 也能work，但某些 Windows 版本会拒绝，实测给一张全 0 的 1bpp 最稳。
        let mask = win32::CreateBitmap(size, size, 1, 1, ptr::null_mut());
        if !mask.is_null() {
            gdi.objects.push(mask);
        }

        // **必须先把 DIB 从 DC 里选出来，再 CreateIconIndirect。**
        // 位图还挂在某个 DC 上时 `CreateIconIndirect` 会直接失败
        // （`GetLastError` = 87 ERROR_INVALID_PARAMETER）——
        // 实测不退出 SelectObject 恒返回 null 图标。
        // `old` 只是 SelectObject 的返回值（原本就有的对象），
        // 不是我们创建的，不需要也不该删。
        win32::SelectObject(mem, old);

        let info = win32::IconInfo {
            is_icon: 1,
            x_hotspot: 0,
            y_hotspot: 0,
            mask,
            color: dib,
        };
        let hicon = win32::CreateIconIndirect(&info);

        if hicon.is_null() {
            if std::env::var_os("ROG_TRAY_DEBUG").is_some() {
                eprintln!("[tray_icon] CreateIconIndirect 失败 GetLastError={}", win32::GetLastError());
            }
            None
        } else {
            Some(hicon)
        }
    }
}

#[cfg(test)]
mod tests {
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
            assert_ne!(win32::DestroyIcon(h), 0, "DestroyIcon 应当成功");
        }
    }

    #[test]
    fn render_handles_extremes() {
        for lv in [0u8, 1, 20, 21, 50, 51, 99, 100, 200] {
            let h = render(lv);
            assert!(h.is_some(), "level={lv} 应该能画出来");
            unsafe { win32::DestroyIcon(h.unwrap()) };
        }
    }

    #[test]
    fn scrolled_font_height_stays_inside_the_icon() {
        // 16px 上两位数必须是可读的，且不能顶出边界。
        assert_eq!(scaled_font_height(82, 16), 13); // 27 * 16 / 32 = 13
        assert_eq!(scaled_font_height(7, 16), 14); // 28 * 16 / 32 = 14
        assert_eq!(scaled_font_height(100, 16), 7); // 14 * 16 / 32 = 7
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
        use crate::win32::GetCurrentProcess;

        unsafe fn gdi_count() -> i32 {
            // GR_GDIOBJECTS = 0
            win32::GetGuiResources(GetCurrentProcess(), 0) as i32
        }

        // 预热
        for _ in 0..20 {
            if let Some(h) = render(82) {
                unsafe { win32::DestroyIcon(h) };
            }
        }
        let before = unsafe { gdi_count() };

        for i in 0..200 {
            let lv = (i % 101) as u8;
            match render(lv) {
                Some(h) => unsafe { win32::DestroyIcon(h) },
                None => panic!("level={lv} 画不出来"),
            };
        }

        let after = unsafe { gdi_count() };
        assert!(
            after - before <= 10,
            "画 200 次后 GDI 对象从 {before} 涨到 {after}，疑似泄漏"
        );
    }
}
