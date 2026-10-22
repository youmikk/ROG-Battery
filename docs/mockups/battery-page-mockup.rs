// 电量页 GDI+ 美化样例 —— 用真实设备数据渲染，尺寸与真实 Tab 内容区一致（729x487）
//
// 目的：给用户看「nwg + GDI+ 到底能做到什么观感」，作为「换不换 iced」的决策依据。
//
// 真实数据（2026-10-22 14:19 实测）：
//   电量 82%  电压 3974 mV  使用中  回报率 1000 Hz  当前档位 2 (800 DPI)
//   休眠 10 分钟  低电量阈值 20%
//   历史 8 采样点：89, 88, 87, 84, 84, 84, 82, 82
//   10-21 15:48 → 10-22 00:33
//   型号 ROG HARPE ACE MINI / 接收器 ROG OMNI RECEIVER / SN B00000000000 / 固件 7.00.07
//
// 编译：
//   rustc -O --edition 2021 mockup.rs -o mockup.exe -L ../vendor/mingw-import-libs
//   （-L 指向只含 libgdiplus.a 等 5 个 .a 的目录，见 tools/vendor-import-libs.ps1）

use std::os::raw::{c_int, c_void};

type Hdc = isize;
type Hbmp = isize;
type Hgdiobj = isize;
type Handle = isize;

#[repr(C)]
struct BitmapInfoHeader {
    bi_size: u32,
    bi_width: i32,
    bi_height: i32,
    bi_planes: u16,
    bi_bit_count: u16,
    bi_compression: u32,
    bi_size_image: u32,
    bi_x_pels: i32,
    bi_y_pels: i32,
    bi_clr_used: u32,
    bi_clr_important: u32,
}

#[repr(C)]
struct BitmapInfo {
    header: BitmapInfoHeader,
    colors: [u32; 3],
}

#[repr(C)]
struct GdiplusStartupInput {
    gdiplus_version: u32,
    debug_event_callback: *mut c_void,
    suppress_background_thread: c_int,
    suppress_external_codecs: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PointF {
    x: f32,
    y: f32,
}

#[repr(C)]
struct RectF {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(hdc: Hdc) -> Hdc;
    fn SelectObject(hdc: Hdc, obj: Hgdiobj) -> Hgdiobj;
    fn DeleteDC(hdc: Hdc) -> c_int;
    fn DeleteObject(obj: Hgdiobj) -> c_int;
    fn CreateDIBSection(
        hdc: Hdc,
        info: *const BitmapInfo,
        usage: u32,
        bits: *mut *mut c_void,
        section: Handle,
        offset: u32,
    ) -> Hbmp;
    fn GdiFlush() -> c_int;
}

#[link(name = "gdiplus")]
extern "system" {
    fn GdiplusStartup(token: *mut usize, input: *const GdiplusStartupInput, output: *mut c_void) -> c_int;
    fn GdiplusShutdown(token: usize);
    fn GdipCreateFromHDC(hdc: Hdc, graphics: *mut *mut c_void) -> c_int;
    fn GdipDeleteGraphics(graphics: *mut c_void) -> c_int;
    fn GdipSetSmoothingMode(graphics: *mut c_void, mode: c_int) -> c_int;
    fn GdipSetTextRenderingHint(graphics: *mut c_void, hint: c_int) -> c_int;
    fn GdipSetPixelOffsetMode(graphics: *mut c_void, mode: c_int) -> c_int;

    fn GdipCreateSolidFill(argb: u32, brush: *mut *mut c_void) -> c_int;
    fn GdipCreateLineBrush(
        p1: *const PointF,
        p2: *const PointF,
        c1: u32,
        c2: u32,
        wrap: c_int,
        brush: *mut *mut c_void,
    ) -> c_int;
    fn GdipDeleteBrush(brush: *mut c_void) -> c_int;

    fn GdipCreatePen1(argb: u32, width: f32, unit: c_int, pen: *mut *mut c_void) -> c_int;
    fn GdipCreatePen2(brush: *mut c_void, width: f32, unit: c_int, pen: *mut *mut c_void) -> c_int;
    fn GdipSetPenStartCap(pen: *mut c_void, cap: c_int) -> c_int;
    fn GdipSetPenEndCap(pen: *mut c_void, cap: c_int) -> c_int;
    fn GdipSetPenLineJoin(pen: *mut c_void, join: c_int) -> c_int;
    fn GdipDeletePen(pen: *mut c_void) -> c_int;

    fn GdipCreatePath(fill_mode: c_int, path: *mut *mut c_void) -> c_int;
    fn GdipDeletePath(path: *mut c_void) -> c_int;
    fn GdipAddPathArc(path: *mut c_void, x: f32, y: f32, w: f32, h: f32, start: f32, sweep: f32) -> c_int;
    fn GdipAddPathLine(path: *mut c_void, x1: f32, y1: f32, x2: f32, y2: f32) -> c_int;
    fn GdipStartPathFigure(path: *mut c_void) -> c_int;
    fn GdipClosePathFigure(path: *mut c_void) -> c_int;
    fn GdipFillPath(g: *mut c_void, brush: *mut c_void, path: *mut c_void) -> c_int;
    fn GdipDrawPath(g: *mut c_void, pen: *mut c_void, path: *mut c_void) -> c_int;

    fn GdipDrawArc(
        g: *mut c_void,
        pen: *mut c_void,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        start: f32,
        sweep: f32,
    ) -> c_int;
    fn GdipDrawEllipse(g: *mut c_void, pen: *mut c_void, x: f32, y: f32, w: f32, h: f32) -> c_int;
    fn GdipDrawLine(g: *mut c_void, pen: *mut c_void, x1: f32, y1: f32, x2: f32, y2: f32) -> c_int;

    fn GdipCreateFontFamilyFromName(name: *const u16, coll: *mut c_void, fam: *mut *mut c_void) -> c_int;
    fn GdipDeleteFontFamily(fam: *mut c_void) -> c_int;
    fn GdipCreateFont(fam: *mut c_void, size: f32, style: c_int, unit: c_int, font: *mut *mut c_void) -> c_int;
    fn GdipDeleteFont(font: *mut c_void) -> c_int;

    fn GdipCreateStringFormat(attrs: c_int, lang: u16, fmt: *mut *mut c_void) -> c_int;
    fn GdipSetStringFormatAlign(fmt: *mut c_void, align: c_int) -> c_int;
    fn GdipSetStringFormatLineAlign(fmt: *mut c_void, align: c_int) -> c_int;
    fn GdipDeleteStringFormat(fmt: *mut c_void) -> c_int;

    fn GdipDrawString(
        g: *mut c_void,
        s: *const u16,
        len: c_int,
        font: *mut c_void,
        layout: *const RectF,
        fmt: *mut c_void,
        brush: *mut c_void,
    ) -> c_int;
}

// ── 配色（深色主题）────────────────────────────────────
const BG: u32 = 0xFF16_181C; // 窗口底
const CARD: u32 = 0xFF1E_222A; // 卡片底
const CARD2: u32 = 0xFF23_2831; // 卡片底（略亮）
const BORDER: u32 = 0xFF2C_323D; // 卡片描边
const TEXT: u32 = 0xFFE8_EAED; // 主文字
const TEXT2: u32 = 0xFF9A_A0A6; // 次要文字
const TEXT3: u32 = 0xFF63_6975; // 更弱文字
const ACCENT: u32 = 0xFF4A_90D9; // 主强调色（蓝）
const ACCENT_HI: u32 = 0xFF6B_B6FF; // 高亮蓝
const GREEN: u32 = 0xFF4C_AF7D; // 使用中 / 正向

fn argb(a: u8, r: u8, g: u8, b: u8) -> u32 {
    ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn log(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("mockup.log")
    {
        let _ = writeln!(f, "{msg}");
        let _ = f.flush();
    }
}

/// 一批共用同一个字族的字体句柄
struct Fonts {
    fam: *mut c_void,
}

impl Fonts {
    unsafe fn new() -> Self {
        let name: Vec<u16> = "Microsoft YaHei UI".encode_utf16().chain(std::iter::once(0)).collect();
        let mut fam: *mut c_void = std::ptr::null_mut();
        let st = GdipCreateFontFamilyFromName(name.as_ptr(), std::ptr::null_mut(), &mut fam);
        log(&format!("  字族 Create={st}"));
        Fonts { fam }
    }

    unsafe fn get(&self, size: f32, bold: bool) -> *mut c_void {
        let mut f: *mut c_void = std::ptr::null_mut();
        let style = if bold { 1 } else { 0 };
        GdipCreateFont(self.fam, size, style, 2 /* Pixel */, &mut f);
        f
    }
}

/// 画文字。align: 0=左 1=中 2=右
unsafe fn text(
    g: *mut c_void,
    fonts: &Fonts,
    s: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    size: f32,
    bold: bool,
    color: u32,
    align: c_int,
) {
    let font = fonts.get(size, bold);
    if font.is_null() {
        return;
    }
    let mut brush: *mut c_void = std::ptr::null_mut();
    GdipCreateSolidFill(color, &mut brush);
    let mut fmt: *mut c_void = std::ptr::null_mut();
    GdipCreateStringFormat(0, 0, &mut fmt);
    if !fmt.is_null() {
        GdipSetStringFormatAlign(fmt, align);
        GdipSetStringFormatLineAlign(fmt, 1);
    }
    let u = wide(s);
    let rc = RectF { x, y, w, h };
    GdipDrawString(g, u.as_ptr(), u.len() as c_int, font, &rc, fmt, brush);
    if !fmt.is_null() {
        GdipDeleteStringFormat(fmt);
    }
    GdipDeleteBrush(brush);
    GdipDeleteFont(font);
}

/// 圆角矩形路径
unsafe fn round_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> *mut c_void {
    let mut p: *mut c_void = std::ptr::null_mut();
    GdipCreatePath(0, &mut p);
    let d = r * 2.0;
    GdipAddPathArc(p, x, y, d, d, 180.0, 90.0);
    GdipAddPathLine(p, x + r, y, x + w - r, y);
    GdipAddPathArc(p, x + w - d, y, d, d, 270.0, 90.0);
    GdipAddPathLine(p, x + w, y + r, x + w, y + h - r);
    GdipAddPathArc(p, x + w - d, y + h - d, d, d, 0.0, 90.0);
    GdipAddPathLine(p, x + w - r, y + h, x + r, y + h);
    GdipAddPathArc(p, x, y + h - d, d, d, 90.0, 90.0);
    GdipAddPathLine(p, x, y + h - r, x, y + r);
    GdipClosePathFigure(p);
    p
}

/// 一张卡片：填充 + 1px 描边
unsafe fn card(g: *mut c_void, x: f32, y: f32, w: f32, h: f32, fill: u32) {
    let p = round_rect(x, y, w, h, 10.0);
    let mut b: *mut c_void = std::ptr::null_mut();
    if GdipCreateSolidFill(fill, &mut b) == 0 {
        GdipFillPath(g, b, p);
        GdipDeleteBrush(b);
    }
    let mut pen: *mut c_void = std::ptr::null_mut();
    if GdipCreatePen1(BORDER, 1.0, 2, &mut pen) == 0 {
        GdipDrawPath(g, pen, p);
        GdipDeletePen(pen);
    }
    GdipDeletePath(p);
}

/// 胶囊标签
unsafe fn pill(
    g: *mut c_void,
    fonts: &Fonts,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: &str,
    fg: u32,
    bg: u32,
) {
    let p = round_rect(x, y, w, h, h / 2.0);
    let mut b: *mut c_void = std::ptr::null_mut();
    if GdipCreateSolidFill(bg, &mut b) == 0 {
        GdipFillPath(g, b, p);
        GdipDeleteBrush(b);
    }
    GdipDeletePath(p);
    text(g, fonts, label, x, y, w, h, 11.0, false, fg, 1);
}

fn main() {
    unsafe {
        let mut token: usize = 0;
        // suppress_background_thread 必须 0（设 1 且 output 传 null 会让 startup 返回 2）
        let input = GdiplusStartupInput {
            gdiplus_version: 1,
            debug_event_callback: std::ptr::null_mut(),
            suppress_background_thread: 0,
            suppress_external_codecs: 0,
        };
        let st = GdiplusStartup(&mut token, &input, std::ptr::null_mut());
        log(&format!("GdiplusStartup={st}"));

        let w = 729i32;
        let h = 487i32;
        let dc = CreateCompatibleDC(0);
        let mut info = BitmapInfo {
            header: BitmapInfoHeader {
                bi_size: 40,
                bi_width: w,
                bi_height: -h,
                bi_planes: 1,
                bi_bit_count: 32,
                bi_compression: 0,
                bi_size_image: (w * h * 4) as u32,
                bi_x_pels: 0,
                bi_y_pels: 0,
                bi_clr_used: 0,
                bi_clr_important: 0,
            },
            colors: [0; 3],
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(dc, &info, 0, &mut bits, 0, 0);
        if bmp == 0 || bits.is_null() {
            log("CreateDIBSection 失败");
            return;
        }
        let old = SelectObject(dc, bmp);

        // 背景手填（BGRA）
        let px = bits as *mut u8;
        for i in 0..(w * h) as usize {
            *px.add(i * 4) = 0x1C;
            *px.add(i * 4 + 1) = 0x18;
            *px.add(i * 4 + 2) = 0x16;
            *px.add(i * 4 + 3) = 0xFF;
        }

        let mut g: *mut c_void = std::ptr::null_mut();
        if GdipCreateFromHDC(dc, &mut g) != 0 || g.is_null() {
            log("GdipCreateFromHDC 失败");
            return;
        }
        GdipSetSmoothingMode(g, 4); // AntiAlias
        GdipSetTextRenderingHint(g, 3); // AntiAliasGridFit（小字更锐，ClearType 会留彩边）
        GdipSetPixelOffsetMode(g, 2); // Half（让 1px 线落在像素中心）

        let fonts = Fonts::new();

        // ══ 顶部：型号 + 状态胶囊 ══════════════════════════
        text(g, &fonts, "ROG HARPE ACE MINI", 20.0, 12.0, 400.0, 22.0, 14.0, true, TEXT, 0);
        text(g, &fonts, "ROG OMNI RECEIVER · 2.4G 无线 · 无驱动读取", 20.0, 33.0, 460.0, 18.0, 10.5, false, TEXT3, 0);
        pill(g, &fonts, 610.0, 14.0, 99.0, 24.0, "● 使用中", GREEN, argb(0xFF, 0x1B, 0x33, 0x28));

        // ══ 卡片 A：电量环 + KPI ═══════════════════════════
        // 几何约束（这是上一版踩的坑）：卡片 A 是 x=20 w=420 → 右边界 440。
        // 上一版把「回报率 1000 Hz」放在 x=408 宽 90，右缘到 498，直接越过卡片边界
        // 被裁掉。所以这里把右侧内容区限定在 x∈[168,420]，两列起点 168 / 294。
        let (ax, ay, aw, ah) = (20.0f32, 58.0f32, 420.0f32, 170.0f32);
        card(g, ax, ay, aw, ah, CARD);
        let right_edge = ax + aw - 20.0; // 420

        // 电量环
        let (cx, cy, r) = (92.0f32, ay + 86.0, 44.0f32);
        let (ex, ey, ew, eh) = (cx - r, cy - r, r * 2.0, r * 2.0);
        let mut pen: *mut c_void = std::ptr::null_mut();
        if GdipCreatePen1(argb(0xFF, 0x2E, 0x34, 0x40), 9.0, 2, &mut pen) == 0 {
            GdipDrawArc(g, pen, ex, ey, ew, eh, 0.0, 360.0);
            GdipDeletePen(pen);
        }
        let sweep = 0.82f32 * 360.0;
        let p1 = PointF { x: ex, y: ey };
        let p2 = PointF { x: ex + ew, y: ey + eh };
        let mut lg: *mut c_void = std::ptr::null_mut();
        let mut pen2: *mut c_void = std::ptr::null_mut();
        if GdipCreateLineBrush(&p1, &p2, ACCENT_HI, ACCENT, 0, &mut lg) == 0 {
            if GdipCreatePen2(lg, 9.0, 2, &mut pen2) == 0 {
                GdipSetPenStartCap(pen2, 2);
                GdipSetPenEndCap(pen2, 2);
                GdipDrawArc(g, pen2, ex, ey, ew, eh, -90.0, sweep);
                GdipDeletePen(pen2);
            }
            GdipDeleteBrush(lg);
        }
        text(g, &fonts, "82", cx - 40.0, cy - 22.0, 72.0, 36.0, 27.0, true, TEXT, 1);
        text(g, &fonts, "%", cx + 20.0, cy - 8.0, 22.0, 20.0, 12.5, false, TEXT2, 0);
        text(g, &fonts, "剩余电量", cx - 40.0, cy + 15.0, 80.0, 16.0, 10.0, false, TEXT3, 1);

        // KPI 两列：起点 168 / 294，右缘不超过 420
        //
        // 单位处理和上一版的区别：上一版把单位作为独立 text 放在「数字 x + 46」，
        // 但 18px bold 的「3974」实际宽约 50px，于是 mV 直接压在数字上。
        // 用固定偏移去猜另一个文本块的实际宽度是不可靠的（字体、DPI、字号一变就错）。
        // 这里改成把单位写进标签行：「电压 mV」/「回报率 Hz」，值行只有数字。
        let c1 = 168.0f32;
        let c2 = 294.0f32;
        text(g, &fonts, "电压 mV", c1, ay + 16.0, 90.0, 16.0, 10.0, false, TEXT3, 0);
        text(g, &fonts, "3974", c1, ay + 32.0, 100.0, 28.0, 18.5, true, TEXT, 0);

        text(g, &fonts, "回报率 Hz", c2, ay + 16.0, 90.0, 16.0, 10.0, false, TEXT3, 0);
        text(g, &fonts, "1000", c2, ay + 32.0, 100.0, 28.0, 18.5, true, TEXT, 0);

        // 分隔线（严格在 card 内）
        let mut pen3: *mut c_void = std::ptr::null_mut();
        if GdipCreatePen1(BORDER, 1.0, 2, &mut pen3) == 0 {
            GdipDrawLine(g, pen3, c1, ay + 72.0, right_edge, ay + 72.0);
            GdipDeletePen(pen3);
        }

        // 第三行：当前档位 / 休眠
        text(g, &fonts, "当前档位", c1, ay + 84.0, 52.0, 18.0, 10.0, false, TEXT3, 0);
        text(g, &fonts, "2", c1 + 52.0, ay + 82.0, 16.0, 20.0, 12.5, true, ACCENT_HI, 0);
        text(g, &fonts, "· 800 DPI", c1 + 66.0, ay + 84.0, 60.0, 18.0, 10.5, false, TEXT2, 0);

        text(g, &fonts, "休眠", c2, ay + 84.0, 34.0, 18.0, 10.0, false, TEXT3, 0);
        text(g, &fonts, "10 分钟", c2 + 34.0, ay + 82.0, 70.0, 20.0, 12.0, true, TEXT, 0);

        // 第四行：低电量阈值 + 样本提示
        text(g, &fonts, "低电量阈值 20%", c1, ay + 112.0, 100.0, 18.0, 10.0, false, TEXT2, 0);
        text(g, &fonts, "历史 8 个采样点，样本不足以估算", c1, ay + 134.0, 240.0, 18.0, 10.0, false, TEXT3, 0);

        // ══ 卡片 B：设备信息 ═══════════════════════════════
        let (bx, by, bw, bh) = (452.0f32, 58.0f32, 257.0f32, 170.0f32);
        card(g, bx, by, bw, bh, CARD2);
        let bl = bx + 18.0;
        let bv = bx + 88.0; // 值列
        text(g, &fonts, "设备信息", bl, by + 14.0, 120.0, 18.0, 10.0, false, TEXT3, 0);
        text(g, &fonts, "鼠标 SN", bl, by + 38.0, 66.0, 16.0, 10.0, false, TEXT3, 0);
        text(g, &fonts, "B00000000000", bv, by + 37.0, 150.0, 18.0, 11.0, true, TEXT, 0);
        text(g, &fonts, "接收器固件", bl, by + 60.0, 66.0, 16.0, 10.0, false, TEXT3, 0);
        text(g, &fonts, "7.00.07", bv, by + 59.0, 150.0, 18.0, 11.0, true, TEXT, 0);
        text(g, &fonts, "链路", bl, by + 82.0, 66.0, 16.0, 10.0, false, TEXT3, 0);
        text(g, &fonts, "0x1B65 无线", bv, by + 81.0, 150.0, 18.0, 11.0, true, TEXT, 0);
        pill(g, &fonts, bl, by + 112.0, 58.0, 22.0, "2.4G", ACCENT_HI, argb(0xFF, 0x1A, 0x27, 0x38));
        pill(g, &fonts, bl + 64.0, by + 112.0, 76.0, 22.0, "1000 Hz", ACCENT_HI, argb(0xFF, 0x1A, 0x27, 0x38));
        pill(g, &fonts, bl + 146.0, by + 112.0, 74.0, 22.0, "19 帧", TEXT2, argb(0xFF, 0x26, 0x2B, 0x33));
        // ══ 卡片 C：电量历史曲线 ════════════════════════════
        let (hx, hy, hw, hh) = (20.0f32, 242.0f32, 689.0f32, 228.0f32);
        card(g, hx, hy, hw, hh, CARD);
        text(g, &fonts, "电量历史", hx + 20.0, hy + 14.0, 120.0, 20.0, 12.5, true, TEXT, 0);
        text(
            g,
            &fonts,
            "8 个采样点  ·  10-21 15:48 → 10-22 00:33",
            hx + 100.0,
            hy + 16.0,
            320.0,
            18.0,
            10.5,
            false,
            TEXT3,
            0,
        );
        // 右上角趋势
        text(g, &fonts, "-1.26 %/h", hx + 560.0, hy + 15.0, 110.0, 18.0, 11.5, true, GREEN, 2);

        // 图表区
        let (gx, gy, gw, gh) = (hx + 52.0, hy + 52.0, hw - 108.0, 138.0f32);
        let bottom = gy + gh;
        let vmin = 80.0f32;
        let vmax = 92.0f32;
        let vy = |v: f32| bottom - (v - vmin) / (vmax - vmin) * gh;

        // 横向网格 + Y 轴标签
        let mut gp: *mut c_void = std::ptr::null_mut();
        if GdipCreatePen1(argb(0x40, 0x2E, 0x34, 0x40), 1.0, 2, &mut gp) == 0 {
            for v in [80.0f32, 84.0, 88.0, 92.0] {
                let y = vy(v);
                GdipDrawLine(g, gp, gx, y, gx + gw, y);
                text(
                    g,
                    &fonts,
                    &format!("{}", v as i32),
                    gx - 30.0,
                    y - 8.0,
                    24.0,
                    16.0,
                    9.5,
                    false,
                    TEXT3,
                    2,
                );
            }
            GdipDeletePen(gp);
        }

        // 真实数据
        let vals = [89.0f32, 88.0, 87.0, 84.0, 84.0, 84.0, 82.0, 82.0];
        let n = vals.len();
        let step = gw / (n as f32 - 1.0);
        let pxv: Vec<PointF> = (0..n)
            .map(|i| PointF {
                x: gx + i as f32 * step,
                y: vy(vals[i]),
            })
            .collect();

        // 折线下方的渐变填充（面积图，向下淡出）
        let mut area: *mut c_void = std::ptr::null_mut();
        GdipCreatePath(0, &mut area);
        GdipAddPathLine(area, pxv[0].x, bottom, pxv[0].x, pxv[0].y);
        for i in 1..n {
            GdipAddPathLine(area, pxv[i - 1].x, pxv[i - 1].y, pxv[i].x, pxv[i].y);
        }
        GdipAddPathLine(area, pxv[n - 1].x, pxv[n - 1].y, pxv[n - 1].x, bottom);
        GdipClosePathFigure(area);
        let ap1 = PointF { x: gx, y: gy };
        let ap2 = PointF { x: gx, y: bottom };
        let mut ab: *mut c_void = std::ptr::null_mut();
        if GdipCreateLineBrush(&ap1, &ap2, argb(0x66, 0x4A, 0x90, 0xD9), argb(0x06, 0x4A, 0x90, 0xD9), 0, &mut ab)
            == 0
        {
            GdipFillPath(g, ab, area);
            GdipDeleteBrush(ab);
        }
        GdipDeletePath(area);

        // 折线
        let mut lp: *mut c_void = std::ptr::null_mut();
        if GdipCreatePen1(ACCENT_HI, 2.0, 2, &mut lp) == 0 {
            GdipSetPenStartCap(lp, 2);
            GdipSetPenEndCap(lp, 2);
            GdipSetPenLineJoin(lp, 2); // Round
            for i in 1..n {
                GdipDrawLine(g, lp, pxv[i - 1].x, pxv[i - 1].y, pxv[i].x, pxv[i].y);
            }
            GdipDeletePen(lp);
        }

        // 数据点：外圈深色 + 内圈实心
        for p in &pxv {
            let mut outer: *mut c_void = std::ptr::null_mut();
            if GdipCreateSolidFill(BG, &mut outer) == 0 {
                let mut pen: *mut c_void = std::ptr::null_mut();
                // 用实心小圆的近似：填充圆
                GdipCreateSolidFill(BG, &mut outer);
                // 半径 4 的底色圆
                let path = round_rect(p.x - 4.5, p.y - 4.5, 9.0, 9.0, 4.5);
                GdipFillPath(g, outer, path);
                GdipDeletePath(path);
                GdipDeleteBrush(outer);
                if GdipCreateSolidFill(ACCENT_HI, &mut pen) == 0 {
                    let path2 = round_rect(p.x - 2.5, p.y - 2.5, 5.0, 5.0, 2.5);
                    GdipFillPath(g, pen, path2);
                    GdipDeletePath(path2);
                    GdipDeleteBrush(pen);
                }
            }
        }

        // 最新点高亮 + 数值标签
        let last = pxv[n - 1];
        let mut hb: *mut c_void = std::ptr::null_mut();
        if GdipCreateSolidFill(argb(0x33, 0x6B, 0xB6, 0xFF), &mut hb) == 0 {
            let p = round_rect(last.x - 9.0, last.y - 9.0, 18.0, 18.0, 9.0);
            GdipFillPath(g, hb, p);
            GdipDeletePath(p);
            GdipDeleteBrush(hb);
        }
        pill(g, &fonts, last.x - 26.0, last.y - 38.0, 52.0, 21.0, "82%", TEXT, argb(0xFF, 0x2A, 0x3A, 0x50));

        // X 轴首尾时间
        text(g, &fonts, "15:48", gx - 10.0, bottom + 6.0, 60.0, 16.0, 9.5, false, TEXT3, 0);
        text(g, &fonts, "00:33", gx + gw - 40.0, bottom + 6.0, 60.0, 16.0, 9.5, false, TEXT3, 2);

        GdiFlush();

        // 存 BMP（手拼 14 字节头，不用 repr(C) 结构体）
        let file_size = 54u32 + (w * h * 4) as u32;
        let mut out: Vec<u8> = Vec::with_capacity(file_size as usize);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&(-h).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&((w * h * 4) as u32).to_le_bytes());
        out.extend_from_slice(&0i32.to_le_bytes());
        out.extend_from_slice(&0i32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(std::slice::from_raw_parts(px, (w * h * 4) as usize));
        match std::fs::write("mockup.bmp", &out) {
            Ok(_) => log(&format!("已写 mockup.bmp {} 字节", out.len())),
            Err(e) => log(&format!("写 BMP 失败: {e}")),
        }

        GdipDeleteGraphics(g);
        GdipDeleteFontFamily(fonts.fam);
        SelectObject(dc, old);
        DeleteObject(bmp);
        DeleteDC(dc);
        GdiplusShutdown(token);
        log("完成");
    }
}
