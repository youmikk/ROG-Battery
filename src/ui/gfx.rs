//! GDI+ 绘制层。
//!
//! # 这一层存在的理由
//!
//! 原来 `gui.rs` 用的是 nwg 的原生控件：按钮是系统画的方角灰框，下拉框是
//! `ComboBox`，滑动条是 `TrackBar`。它们能用，但**颜色、圆角、间距全都改不了**
//! —— 用户看过网页原型后要求的就是原型那个样子（8px 圆角卡片、4px 圆角控件、
//! 蓝色强调、悬停高亮）。
//!
//! nwg 没有提供「自绘控件」的通道，所以这一页的内容区改成：
//! **一个自建的子窗口 + 整页 GDI+ 自绘**。nwg 只保留窗口外壳、托盘、菜单、
//! 消息循环这些和绘制无关的部分。
//!
//! # 已实测的性能结论（不要凭直觉推翻）
//!
//! 双缓冲不是「为了不闪」的奢侈品，它**更快**：
//!
//! | 画法 | 一帧耗时（729×487，含卡片/圆环/曲线/中文）|
//! |---|---|
//! | 直接画窗口 DC | 8.22 – 8.53 ms |
//! | 画内存 DC 再 `BitBlt` | **2.63 – 2.68 ms** |
//!
//! 快了 3.1 倍。原因是 GDI+ 在窗口 DC 上做抗锯齿混合时必须**回读目标像素**
//! （结果 = `alpha*新色 + (1-alpha)*旧色`，"旧色"在窗口 DC 上是屏幕像素，
//! 回读要穿透 DWM 合成器）。双缓冲把它变成纯内存读，最后一次 `BitBlt`
//! 用 `SRCCOPY` 直接拷、不做混合。
//!
//! 测量方法见 `docs\gdiplus-window-dc.md`，三轮独立运行结果一致。
//!
//! # DPI 与滚动：用世界变换做，不用手算
//!
//! 页面代码全部按**逻辑像素**布局（和 CSS 的 px 一一对应）。缩放和滚动
//! 由 GDI+ 的世界变换统一施加：
//!
//! ```text
//! matrix = [scale 0; 0 scale] + translate(0, -scroll*scale)
//! ```
//!
//! 这样带来三个好处：
//! 1. 页面代码里没有一处 `* scale`，改缩放不影响任何布局代码。
//! 2. **文字也被正确缩放** —— 这正是之前「字体发虚」的根因：
//!    进程没声明 DPI 感知时，Windows 把整个窗口当位图拉伸 125%，
//!    所有文字都糊了。声明感知 + 世界变换之后，字是重新栅格化的，清晰。
//! 3. 滚动只需要改一个 translate 值，不需要给每个绘制调用传偏移。
//!
//! # 六个「编译期零警告、运行时不报错、结果错」的坑
//!
//! 全部实测踩过，写在这里省下一次试错：
//!
//! 1. **`GdiplusStartup` 的 `suppressBackgroundThread` 必须是 0**。
//!    设 1 且 `output` 传 `null` 会让 startup 返回 2（InvalidParameter），
//!    之后每一次 GDI+ 调用都返回 18（GdiplusNotInitialized），画面全空、
//!    没有任何错误提示。
//! 2. **`GdipDrawString` 是 7 个参数**。漏掉 `stringFormat` 时编译不报错
//!    （都是我们自己声明的外部函数），运行时直接在 gdiplus.dll 里
//!    `0xC0000005` 崩。`stringFormat` 可以传空，但**位置必须占**。
//! 3. **`BitmapInfo` 的 `colors` 数组必须留位置**（这里用 `[u32; 3]`），
//!    否则 `CreateDIBSection` 按变长调色板算布局时会读越界。
//! 4. **`GetDIBits` 那条路是死路**：调用全返回成功，缓冲区却全是 0。
//!    要读像素只能用 `CreateDIBSection` 直接拿 `bits` 指针。
//! 5. **`SelectObject` 的顺序**：`gdi_delete_object(bmp)` 必须在
//!    `gdi_select_object(dc, old_bmp)` **之后**。反过来会让 `DeleteObject`
//!    静默返回 0（位图还被 DC 选着），**位图永久泄漏**。
//! 6. 所有 GDI+ 函数都返回 `Status`（0 = Ok）。**不看返回值就会得到
//!    一张纯底色的图，且没有任何提示**。

#![allow(dead_code)]

use std::collections::HashMap;
use std::ffi::c_void;

use super::theme::Theme;

// ==================== 类型 ====================

/// GDI 句柄（`HDC` / `HBITMAP` / `HBRUSH` / `HPEN` …）。
///
/// 统一用 `isize` 而不是 `*mut c_void`：GDI 句柄是**不透明值**，不是指针。
/// 用指针类型会诱使人写 `std::ptr::null_mut()`，而 `null_mut()` 是
/// `*mut _`，赋给 `isize` 直接 `expected isize, found *mut _` 编译失败
/// —— 这个坑我在写探针时踩过。
pub type Gdi = isize;

/// 成功的 GDI+ 返回值。
pub const OK: i32 = 0;

/// `GdipCreateLineBrush` 的 `wrap` 参数：平铺模式。
///
/// **不能用 4（Clamp）**。虽然 `WrapMode` 枚举里 Clamp = 4 且 C# 的
/// `new LinearGradientBrush(...)` 允许传它，但**底层线性渐变画刷不支持 Clamp**：
/// 实测 `GdipCreateLineBrush(..., 4, ...)` 稳定返回 **2（InvalidParameter）**，
/// 而 0/1/2/3/5/6 全部返回 0。
///
/// 这个返回值一旦不检查就是最坏的一类 bug：进度弧、卡片渐变、图表面积渐变
/// 三处全部**静默不画**，而代码看起来完全正常（电量环只剩一圈灰底环，
/// 却没有一行报错）。实测 2026-10-22 就是这样上线的。
///
/// 语义上的替代：0（Tile）在「渐变轴恰好覆盖绘制区域」时与 Clamp 等价 ——
/// 我们三处都满足（渐变两端点就是形状的上下边界），所以平铺不会被看到。
pub const WRAP_TILE: i32 = 0;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PointF {
    pub x: f32,
    pub y: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[repr(C)]
struct GdiplusStartupInput {
    version: u32,
    callback: *mut c_void,
    suppress_background_thread: i32,
    suppress_external_codecs: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BitmapInfoHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pels: i32,
    y_pels: i32,
    clr_used: u32,
    clr_important: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BitmapInfo {
    header: BitmapInfoHeader,
    /// 坑 3：这个数组必须存在。`CreateDIBSection` 会按「头 + 调色板」
    /// 算结构总大小，没有它就会读到结构外面的栈上垃圾。
    colors: [u32; 3],
}

// ==================== FFI ====================

#[link(name = "gdiplus")]
extern "system" {
    fn GdiplusStartup(
        token: *mut usize,
        input: *const GdiplusStartupInput,
        out: *mut c_void,
    ) -> i32;
    fn GdiplusShutdown(token: usize);

    fn GdipCreateFromHDC(hdc: Gdi, g: *mut *mut c_void) -> i32;
    fn GdipDeleteGraphics(g: *mut c_void) -> i32;
    fn GdipSetSmoothingMode(g: *mut c_void, mode: i32) -> i32;
    fn GdipSetTextRenderingHint(g: *mut c_void, hint: i32) -> i32;
    fn GdipSetPixelOffsetMode(g: *mut c_void, mode: i32) -> i32;

    fn GdipCreateMatrix2(
        m11: f32,
        m12: f32,
        m21: f32,
        m22: f32,
        dx: f32,
        dy: f32,
        matrix: *mut *mut c_void,
    ) -> i32;
    fn GdipDeleteMatrix(matrix: *mut c_void) -> i32;
    fn GdipSetWorldTransform(g: *mut c_void, matrix: *mut c_void) -> i32;

    fn GdipCreateSolidFill(argb: u32, brush: *mut *mut c_void) -> i32;
    fn GdipDeleteBrush(brush: *mut c_void) -> i32;
    fn GdipCreateLineBrush(
        p1: *const PointF,
        p2: *const PointF,
        c1: u32,
        c2: u32,
        wrap: i32,
        brush: *mut *mut c_void,
    ) -> i32;

    fn GdipCreatePen1(argb: u32, width: f32, unit: i32, pen: *mut *mut c_void) -> i32;
    fn GdipCreatePen2(brush: *mut c_void, width: f32, unit: i32, pen: *mut *mut c_void) -> i32;
    fn GdipDeletePen(pen: *mut c_void) -> i32;
    fn GdipSetPenLineJoin(pen: *mut c_void, join: i32) -> i32;
    fn GdipSetPenStartCap(pen: *mut c_void, cap: i32) -> i32;
    fn GdipSetPenEndCap(pen: *mut c_void, cap: i32) -> i32;

    fn GdipCreatePath(mode: i32, path: *mut *mut c_void) -> i32;
    fn GdipDeletePath(path: *mut c_void) -> i32;
    fn GdipAddPathArc(
        path: *mut c_void,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        start: f32,
        sweep: f32,
    ) -> i32;
    fn GdipAddPathLine(path: *mut c_void, x1: f32, y1: f32, x2: f32, y2: f32) -> i32;
    fn GdipClosePathFigure(path: *mut c_void) -> i32;
    fn GdipFillPath(g: *mut c_void, brush: *mut c_void, path: *mut c_void) -> i32;
    fn GdipDrawPath(g: *mut c_void, pen: *mut c_void, path: *mut c_void) -> i32;
    fn GdipFillRectangle(
        g: *mut c_void,
        brush: *mut c_void,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) -> i32;
    fn GdipDrawLine(g: *mut c_void, pen: *mut c_void, x1: f32, y1: f32, x2: f32, y2: f32) -> i32;
    fn GdipDrawEllipse(g: *mut c_void, pen: *mut c_void, x: f32, y: f32, w: f32, h: f32) -> i32;

    fn GdipCreateFontFamilyFromName(
        name: *const u16,
        collection: *mut c_void,
        family: *mut *mut c_void,
    ) -> i32;
    fn GdipDeleteFontFamily(family: *mut c_void) -> i32;
    fn GdipCreateFont(
        family: *mut c_void,
        size: f32,
        style: i32,
        unit: i32,
        font: *mut *mut c_void,
    ) -> i32;
    fn GdipDeleteFont(font: *mut c_void) -> i32;

    fn GdipCreateStringFormat(attrs: i32, lang: u16, fmt: *mut *mut c_void) -> i32;
    fn GdipSetStringFormatAlign(fmt: *mut c_void, align: i32) -> i32;
    fn GdipSetStringFormatLineAlign(fmt: *mut c_void, align: i32) -> i32;
    fn GdipSetStringFormatTrimming(fmt: *mut c_void, trimming: i32) -> i32;
    fn GdipSetStringFormatFlags(fmt: *mut c_void, flags: i32) -> i32;
    fn GdipDeleteStringFormat(fmt: *mut c_void) -> i32;

    fn GdipDrawString(
        g: *mut c_void,
        s: *const u16,
        len: i32,
        font: *mut c_void,
        layout: *const RectF,
        fmt: *mut c_void,
        brush: *mut c_void,
    ) -> i32;

    fn GdipMeasureString(
        g: *mut c_void,
        s: *const u16,
        len: i32,
        font: *mut c_void,
        layout: *const RectF,
        fmt: *mut c_void,
        bounds: *mut RectF,
        fitted: *mut i32,
        lines: *mut i32,
    ) -> i32;
}

// `CreateCompatibleDC` / `SelectObject` / `DeleteObject` / `DeleteDC` 在
// [`crate::win32`] 里已经声明过一次了（那里用 `Handle = *mut c_void`）。
// 如果在这里再用 `Gdi = isize` 声明一次，Rust 会报
// `clashing_extern_declarations` —— **只警告不报错**，两个声明还各自能被调用。
// 两者确实都是指针宽度、运行时行为一样，但「靠碰巧布局一致才对」不该留着：
// 万一哪天 `Handle` 换成别的表示，编译器不会拦，只会在运行时炸。
// 所以这里不再重复 extern，改为包一层窄转换函数（见下方 `gdi_*`）。
#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleBitmap(hdc: Gdi, w: i32, h: i32) -> Gdi;
    fn BitBlt(dst: Gdi, x: i32, y: i32, w: i32, h: i32, src: Gdi, sx: i32, sy: i32, rop: u32) -> i32;
}

/// 读一个像素的 `COLORREF`（`0x00BBGGRR`，注意**不是** ARGB）。读失败返回 `None`。
///
/// 只在测试里用 —— 生产路径不需要回读自己刚画的像素。加它是因为
/// `ring()` 曾因 `wrap=4` 整段静默不画，而当时唯一的断言是「不崩」。
#[cfg(test)]
fn gdi_get_pixel(hdc: Gdi, x: i32, y: i32) -> Option<u32> {
    #[link(name = "gdi32")]
    extern "system" {
        fn GetPixel(hdc: Gdi, x: i32, y: i32) -> u32;
    }
    let c = unsafe { GetPixel(hdc, x, y) };
    // CLR_INVALID = 0xFFFFFFFF（GetPixel 失败）
    if c == 0xFFFF_FFFF {
        None
    } else {
        Some(c)
    }
}

/// 把内存位图的前 `w`×`h` 像素存成 BMP，用于人工目视检查。
///
/// # 为什么自己拼文件头
///
/// Rust 的 `#[repr(C)]` 会把 14 字节的 `BITMAPFILEHEADER` 补齐成 16 字节
/// （`bfType`+`bfSize`+两个保留字+`bfOffBits` 之后要对齐 4），直接把它当
/// 结构体写出去会让整个头错位 2 字节，读图工具报「内存不足」这种完全
/// 误导人的错误。所以这里用 `to_le_bytes()` 一个字段一个字段拼。
///
/// 只在测试里用 —— 生产路径画完就 `BitBlt`，从不需要落盘。
#[cfg(test)]
pub(crate) fn save_bmp(path: &str, hdc: Gdi, w: i32, h: i32) {
    // 每行字节数必须是 4 的倍数（BMP 规定）。
    let row_bytes = (w * 3 + 3) & !3;
    let pixel_bytes = row_bytes * h;
    let file_size = 14 + 40 + pixel_bytes;

    let mut out: Vec<u8> = Vec::with_capacity(file_size as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // bfReserved1
    out.extend_from_slice(&0u16.to_le_bytes()); // bfReserved2
    out.extend_from_slice(&(54u32).to_le_bytes()); // bfOffBits

    out.extend_from_slice(&(40u32).to_le_bytes()); // biSize
    out.extend_from_slice(&w.to_le_bytes());
    // biHeight 为正是「自下而上」存，这是 BMP 的原生顺序。
    out.extend_from_slice(&h.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    out.extend_from_slice(&24u16.to_le_bytes()); // biBitCount
    out.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    out.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes()); // 72 DPI
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    out.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant

    let pad = (row_bytes - w * 3) as usize;
    for y in (0..h).rev() {
        for x in 0..w {
            let c = gdi_get_pixel(hdc, x, y).unwrap_or(0);
            // `GetPixel` 返回的是 `COLORREF` = `0x00BBGGRR`，**低字节是红**。
            // BMP 的 24 位像素按 B、G、R 顺序存，所以这里要反过来取。
            // 第一版我按 R、G、B 写（和内存顺序一致，看着更自然），
            // 结果整张图红蓝互换：蓝环变橙、红环变蓝，绿色因为对称看不出来
            // —— 差点误判成「`ring()` 的颜色参数传错了」。
            out.push(((c >> 16) & 0xFF) as u8); // B
            out.push(((c >> 8) & 0xFF) as u8); // G
            out.push((c & 0xFF) as u8); // R
        }
        for _ in 0..pad {
            out.push(0);
        }
    }
    if let Err(e) = std::fs::write(path, &out) {
        eprintln!("save_bmp 写失败 {path}: {e}");
    }
}


///
/// 空句柄 `0` 必须映射成空指针 —— 所以这里**不能**直接 `as`，
/// 要走 `0 => null_mut()`，否则 `0` 会变成一个有效地址。
fn gdi_to_handle(g: Gdi) -> crate::win32::Handle {
    if g == 0 {
        std::ptr::null_mut()
    } else {
        g as crate::win32::Handle
    }
}

fn handle_to_gdi(h: crate::win32::Handle) -> Gdi {
    h as Gdi
}

fn gdi_create_compatible_dc(hdc: Gdi) -> Gdi {
    // SAFETY: `CreateCompatibleDC` 接受任意 DC 或 null。
    handle_to_gdi(unsafe { crate::win32::CreateCompatibleDC(gdi_to_handle(hdc)) })
}

fn gdi_select_object(hdc: Gdi, obj: Gdi) -> Gdi {
    handle_to_gdi(unsafe { crate::win32::SelectObject(gdi_to_handle(hdc), gdi_to_handle(obj)) })
}

fn gdi_delete_object(obj: Gdi) -> i32 {
    unsafe { crate::win32::DeleteObject(gdi_to_handle(obj)) }
}

fn gdi_delete_dc(hdc: Gdi) -> i32 {
    unsafe { crate::win32::DeleteDC(gdi_to_handle(hdc)) }
}

const SRCCOPY: u32 = 0x00CC_0020;

// ==================== 进程级初始化 ====================

/// 全局 GDI+ 令牌。
///
/// 全进程只 `GdiplusStartup` **一次**。多次 startup 不会立刻出错，
/// 但每次都要配对 shutdown，漏一个就泄漏；而且字体缓存会跟着分裂。
static TOKEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 启动 GDI+。重复调用是安全的（后面的直接返回 true）。
///
/// 返回 `false` 表示启动失败 —— 调用方**必须真的处理这个返回值**：
/// 启动失败时后面所有绘制都是静默的空画面，用户会看到一个纯色窗口。
pub fn start() -> bool {
    use std::sync::atomic::Ordering;
    if TOKEN.load(Ordering::Relaxed) != 0 {
        return true;
    }
    let input = GdiplusStartupInput {
        version: 1,
        callback: std::ptr::null_mut(),
        // 坑 1：必须 0。
        suppress_background_thread: 0,
        suppress_external_codecs: 0,
    };
    let mut token: usize = 0;
    let st = unsafe { GdiplusStartup(&mut token, &input, std::ptr::null_mut()) };
    if st != OK || token == 0 {
        return false;
    }
    TOKEN.store(token, Ordering::Relaxed);
    true
}

/// 关闭 GDI+。只在程序退出时调一次。
pub fn stop() {
    use std::sync::atomic::Ordering;
    let token = TOKEN.swap(0, Ordering::Relaxed);
    if token != 0 {
        unsafe { GdiplusShutdown(token) };
    }
}

// ==================== 字体 ====================

/// 一个字体缓存表。
///
/// 探针里每画一行字都 `GdipCreateFont` + `GdipDeleteFont`，
/// 一帧建了几十个 —— 这是首帧 22–25ms 的主要开销之一。
/// 字号的取值是有限集合（11、11.5、12、12.5 …），缓存起来即可。
pub struct Fonts {
    family: *mut c_void,
    /// key = `(字号 × 10) as u32` + 是否粗体。
    cache: HashMap<(u32, bool), *mut c_void>,
    /// 只读缓存不是线程安全的 —— 用 PhantomData 明确禁止它跨线程。
    _not_send: std::marker::PhantomData<*mut ()>,
}

impl Fonts {
    /// 建字体表。`face` 是字体族名，例如 `"Microsoft YaHei UI"`。
    ///
    /// 返回 `None` 说明系统里没有这个字体族 —— 调用方应该回落，
    /// **不能默默继续**：那样所有文字都画不出来，用户看到一堆空卡片。
    ///
    /// 内部先 `start()`：`GdipCreateFontFamilyFromName` 在 GDI+ 没初始化时
    /// 返回 `GdiplusNotInitialized(18)`，`family` 保持空，于是这里返回 `None`
    /// —— 调用方会以为是「字体不存在」，一路回落到最后也全是 `None`，
    /// 界面一个字都画不出来。这是个只在**调用顺序**上才暴露的坑，
    /// 与其要求每个调用方记得先 `start()`，不如在这里自己保证。
    /// `start()` 幂等且极便宜（已启动时直接返回 true）。
    pub fn new(face: &str) -> Option<Fonts> {
        if !start() {
            return None;
        }
        let name: Vec<u16> = face.encode_utf16().chain(std::iter::once(0)).collect();
        let mut family: *mut c_void = std::ptr::null_mut();
        let st =
            unsafe { GdipCreateFontFamilyFromName(name.as_ptr(), std::ptr::null_mut(), &mut family) };
        if st != OK || family.is_null() {
            return None;
        }
        Some(Fonts {
            family,
            cache: HashMap::new(),
            _not_send: std::marker::PhantomData,
        })
    }

    /// 按顺序试几个字体族，返回第一个能建出来的。
    ///
    /// 顺序是「雅黑 UI → 雅黑 → 宋体」：本机实测 `Microsoft YaHei`
    /// （不带 UI）**不存在**，只有 `Microsoft YaHei UI`。
    /// 宋体（SimSun）任何中文 Windows 都有，作最后兜底。
    pub fn from_candidates(faces: &[&str]) -> Option<Fonts> {
        for f in faces {
            if let Some(fonts) = Fonts::new(f) {
                return Some(fonts);
            }
        }
        None
    }

    /// 取一个字体句柄（**不要**由调用方释放，它归本表所有）。
    fn get(&mut self, size: f32, bold: bool) -> *mut c_void {
        let key = (((size * 10.0).round() as u32), bold);
        if let Some(&f) = self.cache.get(&key) {
            return f;
        }
        let mut font: *mut c_void = std::ptr::null_mut();
        // style: 0 = Regular, 1 = Bold。unit: 2 = Pixel（跟 CSS 的 px 对齐）。
        let st = unsafe { GdipCreateFont(self.family, size, i32::from(bold), 2, &mut font) };
        let font = if st == OK { font } else { std::ptr::null_mut() };
        self.cache.insert(key, font);
        font
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        for (_, f) in self.cache.drain() {
            if !f.is_null() {
                unsafe { GdipDeleteFont(f) };
            }
        }
        if !self.family.is_null() {
            unsafe { GdipDeleteFontFamily(self.family) };
        }
    }
}

// ==================== 布局用的矩形 ====================

/// 用 `f32` 的矩形。GDI+ 全程按 `f32` 算坐标，用整数矩形会到处 `as f32`。
///
/// 单位是**逻辑像素**（96 DPI 下的 1px）。实际设备像素由世界变换换算。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct R {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl R {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> R {
        R { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }

    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }

    /// 四周内缩（负值向外扩）。
    pub fn inset(&self, d: f32) -> R {
        R::new(
            self.x + d,
            self.y + d,
            (self.w - d * 2.0).max(0.0),
            (self.h - d * 2.0).max(0.0),
        )
    }

    /// 扣掉左边一段。
    pub fn skip_x(&self, dx: f32) -> R {
        R::new(self.x + dx, self.y, (self.w - dx).max(0.0), self.h)
    }

    /// 扣掉右边一段。
    pub fn trim_x(&self, dx: f32) -> R {
        R::new(self.x, self.y, (self.w - dx).max(0.0), self.h)
    }

    /// 在矩形内右侧取一段。
    pub fn tail(&self, w: f32) -> R {
        R::new(self.right() - w, self.y, w, self.h)
    }

    /// 在矩形内左侧取一段。
    pub fn head(&self, w: f32) -> R {
        R::new(self.x, self.y, w, self.h)
    }

    /// 竖直方向上重新定位、改高度。
    pub fn at_y(&self, y: f32, h: f32) -> R {
        R::new(self.x, y, self.w, h)
    }

    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    /// 两个矩形是否相交（裁剪用）。
    pub fn intersects(&self, o: &R) -> bool {
        self.x < o.right() && self.right() > o.x && self.y < o.bottom() && self.bottom() > o.y
    }
}

/// 文字在矩形里的对齐方式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

// ==================== 绘制上下文 ====================

/// 一次绘制的上下文：一个 GDI+ Graphics + 一套配色 + 字体表。
///
/// 页面代码只拿 `&mut Painter`，拿不到裸指针，也就不会误建 Graphics。
///
/// **所有坐标都是逻辑像素、内容坐标**（y=0 是内容顶部，不是窗口顶部）。
/// 缩放和滚动由 Graphics 的世界变换处理，见模块文档。
pub struct Painter<'a> {
    g: *mut c_void,
    fonts: &'a mut Fonts,
    pub t: &'a Theme,
    /// 可见区域（内容坐标）。页面用它跳过滚出屏幕的卡片。
    ///
    /// 这不只是性能优化：GDI+ 在双缓冲位图上画到区域外会污染位图内容，
    /// 而位图比可视区大（见 `Canvas::ensure`），脏了就露出旧帧。
    clip: R,
    /// DPI 缩放系数（96 DPI = 1.0）。只用于 `measure` 把
    /// 设备单位换算回逻辑单位 —— 绘制本身由世界变换处理。
    scale: f32,
}

impl<'a> Painter<'a> {
    /// 整块是否在可见区内。
    fn visible(&self, r: R) -> bool {
        r.y < self.clip.bottom() && r.bottom() > self.clip.y && r.x < self.clip.right() && r.right() > self.clip.x
    }

    // ---- 基本图元 ----

    /// 实心矩形。走 GDI+ 而不是 GDI 的 `FillRect`，
    /// 因为这里可能是带 alpha 的颜色。
    pub fn rect(&mut self, r: R, color: u32) {
        if !self.visible(r) {
            return;
        }
        unsafe {
            let mut b: *mut c_void = std::ptr::null_mut();
            if GdipCreateSolidFill(color, &mut b) != OK {
                return;
            }
            GdipFillRectangle(self.g, b, r.x, r.y, r.w, r.h);
            GdipDeleteBrush(b);
        }
    }

    /// 圆角实心矩形。
    pub fn round(&mut self, r: R, radius: f32, color: u32) {
        if !self.visible(r) {
            return;
        }
        self.with_path(r, radius, |p| unsafe {
            let mut b: *mut c_void = std::ptr::null_mut();
            if GdipCreateSolidFill(color, &mut b) == OK {
                GdipFillPath(self.g as *mut c_void, b, p);
                GdipDeleteBrush(b);
            }
        });
    }

    /// 圆角矩形：填充 + 1px 描边（卡片就是这个）。
    pub fn card(&mut self, r: R, radius: f32, fill: u32, stroke: u32) {
        if !self.visible(r) {
            return;
        }
        let g = self.g;
        self.with_path(r, radius, |p| unsafe {
            let mut b: *mut c_void = std::ptr::null_mut();
            if GdipCreateSolidFill(fill, &mut b) == OK {
                GdipFillPath(g, b, p);
                GdipDeleteBrush(b);
            }
            // alpha = 0 的描边等于「不描边」。透明色的边框画出来其实看不见，
            // 但会多一次 path stroke，能省就省。
            if stroke >> 24 != 0 {
                let mut pen: *mut c_void = std::ptr::null_mut();
                if GdipCreatePen1(stroke, 1.0, 2, &mut pen) == OK {
                    GdipDrawPath(g, pen, p);
                    GdipDeletePen(pen);
                }
            }
        });
    }

    /// 圆角描边（无填充）。用于「悬停边框」「输入框」。
    pub fn round_stroke(&mut self, r: R, radius: f32, color: u32, width: f32) {
        if !self.visible(r) || color >> 24 == 0 {
            return;
        }
        let g = self.g;
        self.with_path(r, radius, |p| unsafe {
            let mut pen: *mut c_void = std::ptr::null_mut();
            if GdipCreatePen1(color, width, 2, &mut pen) == OK {
                GdipDrawPath(g, pen, p);
                GdipDeletePen(pen);
            }
        });
    }

    /// 竖直渐变填充的圆角矩形（卡片顶部高光、选中行）。
    pub fn round_gradient(&mut self, r: R, radius: f32, top: u32, bottom: u32) {
        if !self.visible(r) {
            return;
        }
        let g = self.g;
        self.with_path(r, radius, |p| unsafe {
            let p1 = PointF { x: 0.0, y: r.y };
            let p2 = PointF { x: 0.0, y: r.bottom() };
            let mut b: *mut c_void = std::ptr::null_mut();
            // wrap 必须用 0（Tile）而不是 4（Clamp）—— 线性渐变画刷拒绝 Clamp，
            // 传 4 会返回 InvalidParameter 并让整个填充静默跳过。详见 `WRAP_TILE`。
            if GdipCreateLineBrush(&p1, &p2, top, bottom, WRAP_TILE, &mut b) == OK {
                GdipFillPath(g, b, p);
                GdipDeleteBrush(b);
            }
        });
    }

    /// 直线。
    pub fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: u32, width: f32) {
        let r = R::new(
            x1.min(x2),
            y1.min(y2),
            (x2 - x1).abs().max(0.1),
            (y2 - y1).abs().max(0.1),
        );
        if !self.visible(r) || color >> 24 == 0 {
            return;
        }
        unsafe {
            let mut pen: *mut c_void = std::ptr::null_mut();
            if GdipCreatePen1(color, width, 2, &mut pen) != OK {
                return;
            }
            GdipDrawLine(self.g, pen, x1, y1, x2, y2);
            GdipDeletePen(pen);
        }
    }

    /// 横向分隔线（1 像素，落在半像素上才不糊）。
    pub fn hline(&mut self, x1: f32, x2: f32, y: f32, color: u32) {
        self.line(x1, y + 0.5, x2, y + 0.5, color, 1.0);
    }

    /// 折线（曲线图用）。`pts` 是内容坐标。
    pub fn polyline(&mut self, pts: &[(f32, f32)], color: u32, width: f32) {
        if pts.len() < 2 {
            return;
        }
        let y_min = pts.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
        let y_max = pts.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max);
        let x_min = pts.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
        let x_max = pts.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max);
        if !self.visible(R::new(x_min, y_min, x_max - x_min, y_max - y_min)) {
            return;
        }
        unsafe {
            let mut pen: *mut c_void = std::ptr::null_mut();
            if GdipCreatePen1(color, width, 2, &mut pen) != OK {
                return;
            }
            // 2 = Round join：折线拐角是圆的，不会出现尖角的毛刺
            GdipSetPenLineJoin(pen, 2);
            for w in pts.windows(2) {
                GdipDrawLine(self.g, pen, w[0].0, w[0].1, w[1].0, w[1].1);
            }
            GdipDeletePen(pen);
        }
    }

    /// 多边形填充（曲线图的面积填充）。
    ///
    /// `pts` 至少 3 个点，自动闭合。`top`/`bottom` 是竖直渐变的两端颜色
    /// （原型里面积是 `#4CC2FF` 从 34% 不透明渐变到 2%）。
    pub fn fill_polygon(&mut self, pts: &[(f32, f32)], top: u32, bottom: u32) {
        if pts.len() < 3 {
            return;
        }
        let y_min = pts.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
        let y_max = pts.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max);
        let x_min = pts.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
        let x_max = pts.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max);
        if !self.visible(R::new(x_min, y_min, x_max - x_min, y_max - y_min)) {
            return;
        }
        unsafe {
            let mut path: *mut c_void = std::ptr::null_mut();
            if GdipCreatePath(0, &mut path) != OK {
                return;
            }
            // 每条边都显式从上一个点连到当前点。
            // 不要图省事只传 `(p, p)` 让库自己接 —— `GdipAddPathLine`
            // 在路径已有当前点时会不会自动连一段，是文档没写清的行为，
            // 而错的表现是「面积图少画几条边」，看图未必看得出。
            for w in pts.windows(2) {
                GdipAddPathLine(path, w[0].0, w[0].1, w[1].0, w[1].1);
            }
            GdipClosePathFigure(path);

            let mut brush: *mut c_void = std::ptr::null_mut();
            // wrap 用 0 不用 4：线性渐变画刷不支持 Clamp，传 4 返 InvalidParameter
            // 会让整块面积填充静默消失。详见 `WRAP_TILE`。
            let rc = GdipCreateLineBrush(
                &PointF { x: 0.0, y: y_min },
                &PointF { x: 0.0, y: y_max },
                top,
                bottom,
                WRAP_TILE,
                &mut brush,
            );
            if rc == OK {
                // 3 = FillModeWinding
                GdipFillPath(self.g, brush, path);
                GdipDeleteBrush(brush);
            }
            GdipDeletePath(path);
        }
    }

    /// 圆环进度弧（电量环）。
    ///
    /// `frac` 是 0.0–1.0 的填充比例，从正上方（12 点）顺时针。
    /// 用圆头笔帽，两端是圆的 —— 这是原型里的样子。
    pub fn ring(
        &mut self,
        cx: f32,
        cy: f32,
        radius: f32,
        width: f32,
        frac: f32,
        from: u32,
        to: u32,
        track: u32,
    ) {
        let box_r = R::new(cx - radius, cy - radius, radius * 2.0, radius * 2.0);
        if !self.visible(box_r) {
            return;
        }
        let frac = frac.clamp(0.0, 1.0);
        unsafe {
            // 底环（整圈）
            if track >> 24 != 0 {
                let mut pen: *mut c_void = std::ptr::null_mut();
                if GdipCreatePen1(track, width, 2, &mut pen) == OK {
                    GdipDrawEllipse(self.g, pen, box_r.x, box_r.y, box_r.w, box_r.h);
                    GdipDeletePen(pen);
                }
            }
            if frac <= 0.001 {
                return;
            }
            // 进度弧：GDI+ 的 0° 在 3 点方向、角度增大是顺时针，
            // 所以要「从 12 点顺时针」就起点 -90°、扫正角度。
            let start = -90.0f32;
            let sweep = 360.0 * frac;
            let p1 = PointF { x: cx - radius, y: cy };
            let p2 = PointF { x: cx + radius, y: cy };
            let mut brush: *mut c_void = std::ptr::null_mut();
            let mut pen: *mut c_void = std::ptr::null_mut();
            if GdipCreateLineBrush(&p1, &p2, from, to, WRAP_TILE, &mut brush) == OK {
                if GdipCreatePen2(brush, width, 2, &mut pen) == OK {
                    // cap 2 = Round
                    GdipSetPenStartCap(pen, 2);
                    GdipSetPenEndCap(pen, 2);
                    let mut path: *mut c_void = std::ptr::null_mut();
                    if GdipCreatePath(0, &mut path) == OK {
                        GdipAddPathArc(path, box_r.x, box_r.y, box_r.w, box_r.h, start, sweep);
                        GdipDrawPath(self.g, pen, path);
                        GdipDeletePath(path);
                    }
                    GdipDeletePen(pen);
                }
                GdipDeleteBrush(brush);
            }
        }
    }

    /// 充电中的电量环：进度弧 + 一条沿整圈跑动的彗尾。
    ///
    /// # 为什么需要单独的绘制函数
    ///
    /// 需求是「充电的时候是绿色加动效」。动效不能用「整圈呼吸」——
    /// 呼吸看起来像环在自己闪，用户分不清是「在充电」还是「界面卡了」。
    /// 跑动的头只有一个方向，配合静态进度弧，语义很清楚：
    /// **进度弧表示电量，跑动的头表示正在充**。
    ///
    /// # 相位从哪来
    ///
    /// 不用额外传参数，直接读 `GetTickCount`（系统运行毫秒数，1ms 分辨率）。
    /// 理由：动效是纯装饰，只要**每帧不重样**就够了；引入一个「动画时钟」
    /// 状态意味着 canvas 要维护定时器和生命周期，出错会导致整块界面不再刷新。
    /// 现在这样即使时钟被暂停（系统睡眠后 GetTickCount 不跳），
    /// 最坏情况是彗尾停一下，恢复后自动继续。
    ///
    /// 画充电动效：在电量进度弧上叠加呼吸光晕与流光能量波。
    pub fn arc_comet(
        &mut self,
        cx: f32,
        cy: f32,
        radius: f32,
        width: f32,
        frac: f32,
        color: u32,
        progress_color: u32,
        track: u32,
    ) {
        // 先画底环和基础电量进度弧
        self.ring(cx, cy, radius, width, frac, progress_color, progress_color, track);

        let box_r = R::new(cx - radius, cy - radius, radius * 2.0, radius * 2.0);
        if !self.visible(box_r) {
            return;
        }

        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let start = START.get_or_init(std::time::Instant::now);
        let now = start.elapsed().as_secs_f32() * 1000.0;
        // 柔和呼吸光晕：沿有效电量弧做柔和的光亮呼吸
        let breath = (now * 0.0032).sin() * 0.5 + 0.5;
        let glow_alpha = (0x60 as f32 + breath * 0x70 as f32) as u8;
        let glow_color = with_alpha(color, glow_alpha);

        let active_span = frac.clamp(0.06, 1.0) * 360.0;

        unsafe {
            let mut brush: *mut c_void = std::ptr::null_mut();
            let mut pen: *mut c_void = std::ptr::null_mut();
            if GdipCreateSolidFill(glow_color, &mut brush) == OK {
                if GdipCreatePen2(brush, width + 2.0, 2, &mut pen) == OK {
                    GdipSetPenStartCap(pen, 2);
                    GdipSetPenEndCap(pen, 2);
                    let mut path: *mut c_void = std::ptr::null_mut();
                    if GdipCreatePath(0, &mut path) == OK {
                        GdipAddPathArc(path, box_r.x, box_r.y, box_r.w, box_r.h, -90.0, active_span);
                        GdipDrawPath(self.g, pen, path);
                        GdipDeletePath(path);
                    }
                    GdipDeletePen(pen);
                }
                GdipDeleteBrush(brush);
            }

            // 能量流动光波：沿有效电量弧平滑往复穿行
            const PERIOD_MS: f32 = 1500.0;
            let phase = (now % PERIOD_MS) / PERIOD_MS;
            let wave_len = (active_span * 0.4).clamp(24.0, 64.0);
            let wave_start = -90.0 + (active_span - wave_len) * phase;

            let mut w_brush: *mut c_void = std::ptr::null_mut();
            let mut w_pen: *mut c_void = std::ptr::null_mut();
            if GdipCreateSolidFill(with_alpha(color, 0xF5), &mut w_brush) == OK {
                if GdipCreatePen2(w_brush, width + 0.5, 2, &mut w_pen) == OK {
                    GdipSetPenStartCap(w_pen, 2);
                    GdipSetPenEndCap(w_pen, 2);
                    let mut path: *mut c_void = std::ptr::null_mut();
                    if GdipCreatePath(0, &mut path) == OK {
                        GdipAddPathArc(path, box_r.x, box_r.y, box_r.w, box_r.h, wave_start, wave_len);
                        GdipDrawPath(self.g, w_pen, path);
                        GdipDeletePath(path);
                    }
                    GdipDeletePen(w_pen);
                }
                GdipDeleteBrush(w_brush);
            }
        }
    }

    /// 画实心圆。
    pub fn circle(&mut self, cx: f32, cy: f32, radius: f32, color: u32) {
        let r = R::new(cx - radius, cy - radius, radius * 2.0, radius * 2.0);
        self.round(r, radius, color);
    }

    /// 画圆描边。
    pub fn circle_stroke(&mut self, cx: f32, cy: f32, radius: f32, color: u32, width: f32) {
        let r = R::new(cx - radius, cy - radius, radius * 2.0, radius * 2.0);
        self.round_stroke(r, radius, color, width);
    }

    /// 画圆弧（度数顺时针，可用于绘制色环、扇区等）。
    pub fn arc(
        &mut self,
        cx: f32,
        cy: f32,
        radius: f32,
        width: f32,
        start_deg: f32,
        sweep_deg: f32,
        color: u32,
    ) {
        let box_r = R::new(cx - radius, cy - radius, radius * 2.0, radius * 2.0);
        if !self.visible(box_r) || color >> 24 == 0 {
            return;
        }
        unsafe {
            let mut pen: *mut c_void = std::ptr::null_mut();
            if GdipCreatePen1(color, width, 2, &mut pen) == OK {
                let mut path: *mut c_void = std::ptr::null_mut();
                if GdipCreatePath(0, &mut path) == OK {
                    GdipAddPathArc(path, box_r.x, box_r.y, box_r.w, box_r.h, start_deg, sweep_deg);
                    GdipDrawPath(self.g, pen, path);
                    GdipDeletePath(path);
                }
                GdipDeletePen(pen);
            }
        }
    }

    // ---- 文字 ----
    /// 画一行字。矩形决定对齐范围，垂直居中。
    pub fn text(&mut self, s: &str, r: R, size: f32, bold: bool, color: u32, align: Align) {
        if !self.visible(r) || s.is_empty() {
            return;
        }
        let font = self.fonts.get(size, bold);
        if font.is_null() {
            return;
        }
        let u: Vec<u16> = s.encode_utf16().collect();
        let layout = RectF { x: r.x, y: r.y, w: r.w, h: r.h };
        unsafe {
            let mut brush: *mut c_void = std::ptr::null_mut();
            if GdipCreateSolidFill(color, &mut brush) != OK {
                return;
            }
            let mut fmt: *mut c_void = std::ptr::null_mut();
            if GdipCreateStringFormat(0, 0, &mut fmt) == OK {
                let a = match align {
                    Align::Left => 0,
                    Align::Center => 1,
                    Align::Right => 2,
                };
                GdipSetStringFormatAlign(fmt, a);
                // 1 = Center（竖直居中）
                GdipSetStringFormatLineAlign(fmt, 1);
                // 3 = EllipsisCharacter：超宽截断加省略号，不换行
                GdipSetStringFormatTrimming(fmt, 3);
                // 0x0000_1000 = StringFormatFlagsNoWrap。
                //
                // 光设 Trimming **不够**：GDI+ 默认允许自动折行，一串没有空格
                // 的连续字符（输入框里的 `400900`）会在框宽处硬断成两行，
                // 第二行还溢出到框外。`text()` 语义就是「画一行」，必须显式禁折行。
                GdipSetStringFormatFlags(fmt, 0x0000_1000);
            }
            GdipDrawString(self.g, u.as_ptr(), u.len() as i32, font, &layout, fmt, brush);
            if !fmt.is_null() {
                GdipDeleteStringFormat(fmt);
            }
            GdipDeleteBrush(brush);
        }
    }

    /// 画多行文字，从矩形顶部开始排。
    pub fn text_wrap(&mut self, s: &str, r: R, size: f32, color: u32) {
        if !self.visible(r) || s.is_empty() {
            return;
        }
        let font = self.fonts.get(size, false);
        if font.is_null() {
            return;
        }
        let u: Vec<u16> = s.encode_utf16().collect();
        let layout = RectF { x: r.x, y: r.y, w: r.w, h: r.h };
        unsafe {
            let mut brush: *mut c_void = std::ptr::null_mut();
            if GdipCreateSolidFill(color, &mut brush) != OK {
                return;
            }
            let mut fmt: *mut c_void = std::ptr::null_mut();
            if GdipCreateStringFormat(0, 0, &mut fmt) == OK {
                GdipSetStringFormatAlign(fmt, 0);
                // 0 = Near（竖直靠上）
                GdipSetStringFormatLineAlign(fmt, 0);
            }
            GdipDrawString(self.g, u.as_ptr(), u.len() as i32, font, &layout, fmt, brush);
            if !fmt.is_null() {
                GdipDeleteStringFormat(fmt);
            }
            GdipDeleteBrush(brush);
        }
    }

    /// 量一段文字的尺寸，返回**逻辑单位**。
    ///
    /// 世界变换在生效，`GdipMeasureString` 返回的是设备单位，
    /// 所以这里除以 `scale` 换算回去 —— 页面代码不用知道 DPI。
    pub fn measure(&mut self, s: &str, size: f32, bold: bool) -> (f32, f32) {
        let font = self.fonts.get(size, bold);
        if font.is_null() || s.is_empty() {
            return (0.0, 0.0);
        }
        let u: Vec<u16> = s.encode_utf16().collect();
        let layout = RectF { x: 0.0, y: 0.0, w: 4096.0, h: 4096.0 };
        let mut bounds = RectF::default();
        let mut fitted: i32 = 0;
        let mut lines: i32 = 0;
        let st = unsafe {
            GdipMeasureString(
                self.g,
                u.as_ptr(),
                u.len() as i32,
                font,
                &layout,
                std::ptr::null_mut(),
                &mut bounds,
                &mut fitted,
                &mut lines,
            )
        };
        if st != OK {
            return (0.0, 0.0);
        }
        let k = if self.scale > 0.01 { self.scale } else { 1.0 };
        (bounds.w / k, bounds.h / k)
    }

    // ---- 复合控件 ----

    /// 胶囊标签（`2.4G` / `1000 Hz` / `19 帧`）。
    pub fn pill(&mut self, r: R, label: &str, fg: u32, bg: u32, size: f32) {
        self.round(r, r.h / 2.0, bg);
        self.text(label, r, size, false, fg, Align::Center);
    }

    /// 开关（36×20）。`on` 决定滑块位置和底色。
    ///
    /// 关态用 `track` 做底、近白色做滑块；开态用 `accent` 做底。
    /// 这是 Win11 的样子。
    pub fn switch(&mut self, r: R, on: bool, enabled: bool) {
        let t = self.t;
        let bg = match (on, enabled) {
            (true, true) => t.accent,
            (false, true) => t.track,
            (true, false) => with_alpha(t.accent, 0x60),
            (false, false) => with_alpha(t.track, 0x40),
        };
        self.round(r, r.h / 2.0, bg);
        let knob_d = r.h - 4.0;
        let knob_x = if on { r.right() - 2.0 - knob_d } else { r.x + 2.0 };
        let knob = if enabled {
            0xFFFF_FFFF
        } else {
            0x80FF_FFFF
        };
        self.round(R::new(knob_x, r.y + 2.0, knob_d, knob_d), knob_d / 2.0, knob);
    }

    /// 复选框（16×16）。
    pub fn checkbox(&mut self, r: R, checked: bool, enabled: bool) {
        let t = self.t;
        if checked {
            let c = if enabled { t.accent } else { with_alpha(t.accent, 0x70) };
            self.round(r, 3.0, c);
            // 勾：两段折线
            let (x, y) = (r.x, r.y);
            let ink = if enabled { 0xFFFF_FFFF } else { 0x90FF_FFFF };
            self.line(x + 3.5, y + 8.0, x + 6.5, y + 11.0, ink, 1.8);
            self.line(x + 6.5, y + 11.0, x + 12.0, y + 5.0, ink, 1.8);
        } else {
            if enabled {
                self.round(r, 3.0, t.bg_sunken);
                self.round_stroke(r, 3.0, t.border_strong, 1.0);
            } else {
                self.round(r, 3.0, with_alpha(t.bg_sunken, 0x60));
                self.round_stroke(r, 3.0, with_alpha(t.border_strong, 0x60), 1.0);
            }
        }
    }

    /// 滑块（水平）。返回滑块圆的中心 x，方便画提示气泡。
    ///
    /// `r` 是**整条控件**的矩形；轨道竖直居中在 `r.cy()`。
    pub fn slider(&mut self, r: R, frac: f32, enabled: bool) -> f32 {
        let t = self.t;
        let frac = frac.clamp(0.0, 1.0);
        let track_h = 4.0;
        let track_y = r.cy() - track_h / 2.0;
        let knob_d = 18.0;
        // 两端各留半个滑块，滑块中心才不会跑出控件
        let x0 = r.x + knob_d / 2.0;
        let x1 = r.right() - knob_d / 2.0;
        let knob_x = x0 + (x1 - x0) * frac;

        let track = if enabled { t.track } else { with_alpha(t.track, 0x50) };
        self.round(R::new(x0, track_y, (x1 - x0).max(1.0), track_h), track_h / 2.0, track);
        if frac > 0.001 {
            let c = if enabled { t.accent } else { with_alpha(t.accent, 0x70) };
            self.round(
                R::new(x0, track_y, (knob_x - x0).max(1.0), track_h),
                track_h / 2.0,
                c,
            );
        }
        // 滑块：蓝底 + 一圈卡底色描边（Win11 的样子）
        let knob = R::new(knob_x - knob_d / 2.0, r.cy() - knob_d / 2.0, knob_d, knob_d);
        let c = if enabled { t.accent } else { with_alpha(t.accent, 0x70) };
        self.round(knob, knob_d / 2.0, c);
        self.round_stroke(knob, knob_d / 2.0, t.card, 3.0);
        knob_x
    }

    /// 下拉箭头（▾），画在矩形中心。
    pub fn chevron(&mut self, r: R, color: u32) {
        let cx = r.cx();
        let cy = r.cy();
        self.line(cx - 4.0, cy - 2.0, cx, cy + 2.0, color, 1.4);
        self.line(cx, cy + 2.0, cx + 4.0, cy - 2.0, color, 1.4);
    }

    /// 圆形小圆点（状态点、数据点）。
    pub fn dot(&mut self, cx: f32, cy: f32, radius: f32, color: u32) {
        let r = R::new(cx - radius, cy - radius, radius * 2.0, radius * 2.0);
        self.round(r, radius, color);
    }

    /// 空心圆环（数据点的高亮圈）。
    pub fn dot_ring(&mut self, cx: f32, cy: f32, radius: f32, color: u32, width: f32) {
        let r = R::new(cx - radius, cy - radius, radius * 2.0, radius * 2.0);
        if !self.visible(r) {
            return;
        }
        unsafe {
            let mut pen: *mut c_void = std::ptr::null_mut();
            if GdipCreatePen1(color, width, 2, &mut pen) == OK {
                GdipDrawEllipse(self.g, pen, r.x, r.y, r.w, r.h);
                GdipDeletePen(pen);
            }
        }
    }

    // ---- 内部 ----

    /// 建一条圆角矩形路径，交给闭包用完即删。
    fn with_path<F: FnOnce(*mut c_void)>(&self, r: R, radius: f32, f: F) {
        let radius = radius.min(r.h / 2.0).min(r.w / 2.0).max(0.0);
        unsafe {
            let mut p: *mut c_void = std::ptr::null_mut();
            if GdipCreatePath(0, &mut p) != OK || p.is_null() {
                return;
            }
            if radius <= 0.01 {
                // 直角：用四段线围起来
                GdipAddPathLine(p, r.x, r.y, r.right(), r.y);
                GdipAddPathLine(p, r.right(), r.y, r.right(), r.bottom());
                GdipAddPathLine(p, r.right(), r.bottom(), r.x, r.bottom());
                GdipAddPathLine(p, r.x, r.bottom(), r.x, r.y);
                GdipClosePathFigure(p);
            } else {
                let d = radius * 2.0;
                // 从一个角开始，顺时针走一圈
                GdipAddPathArc(p, r.x, r.y, d, d, 180.0, 90.0);
                GdipAddPathLine(p, r.x + radius, r.y, r.right() - radius, r.y);
                GdipAddPathArc(p, r.right() - d, r.y, d, d, 270.0, 90.0);
                GdipAddPathLine(p, r.right(), r.y + radius, r.right(), r.bottom() - radius);
                GdipAddPathArc(p, r.right() - d, r.bottom() - d, d, d, 0.0, 90.0);
                GdipAddPathLine(p, r.right() - radius, r.bottom(), r.x + radius, r.bottom());
                GdipAddPathArc(p, r.x, r.bottom() - d, d, d, 90.0, 90.0);
                GdipAddPathLine(p, r.x, r.bottom() - radius, r.x, r.y + radius);
                GdipClosePathFigure(p);
            }
            f(p);
            GdipDeletePath(p);
        }
    }
}

/// 把颜色的 alpha 通道改掉。
pub fn with_alpha(color: u32, alpha: u8) -> u32 {
    (color & 0x00FF_FFFF) | ((alpha as u32) << 24)
}

// ==================== 画布 ====================

/// 双缓冲画布。
///
/// 生命周期（照抄顺序，漏一步就泄漏）：
///
/// ```text
/// 窗口创建 / 尺寸变化：
///   hdc = GetDC(hwnd)                  ← 只为拿位图格式模板
///   dc  = gdi_create_compatible_dc(hdc)
///   bmp = CreateCompatibleBitmap(hdc, w, h)
///   old = gdi_select_object(dc, bmp)
///   ReleaseDC(hwnd, hdc)
///
/// 每次 WM_PAINT：
///   g = GdipCreateFromHDC(dc)
///   ...画...
///   GdipDeleteGraphics(g)
///   BitBlt(窗口DC, 0,0,w,h, dc, 0,0, SRCCOPY)
///
/// 尺寸变化 / 退出：
///   gdi_select_object(dc, old)              ← 必须先换出
///   gdi_delete_object(bmp)                  ← 再删位图
///   gdi_delete_dc(dc)
/// ```
pub struct Canvas {
    dc: Gdi,
    bmp: Gdi,
    old: Gdi,
    w: i32,
    h: i32,
}

impl Canvas {
    /// 建一个空画布。真正的位图在第一次 `ensure` 时创建。
    pub fn new() -> Canvas {
        Canvas { dc: 0, bmp: 0, old: 0, w: 0, h: 0 }
    }

    /// 保证画布尺寸至少是 `w × h`，返回真正的绘制 DC。
    ///
    /// **位图只会变大不会变小**：缩窗口时不重建，避免拖拽时每帧
    /// `CreateCompatibleBitmap`（那是内核对象分配，很慢）。
    /// 多出来的部分不 `BitBlt`，所以看不到。
    pub fn ensure(&mut self, screen_dc: Gdi, w: i32, h: i32) -> Gdi {
        let w = w.max(1);
        let h = h.max(1);
        if self.dc != 0 && self.w >= w && self.h >= h {
            return self.dc;
        }
        let nw = if self.w == 0 { w } else { self.w.max(w) };
        let nh = if self.h == 0 { h } else { self.h.max(h) };
        self.release();
        unsafe {
            let dc = gdi_create_compatible_dc(screen_dc);
            if dc == 0 {
                return 0;
            }
            let bmp = CreateCompatibleBitmap(screen_dc, nw, nh);
            if bmp == 0 {
                gdi_delete_dc(dc);
                return 0;
            }
            let old = gdi_select_object(dc, bmp);
            self.dc = dc;
            self.bmp = bmp;
            self.old = old;
            self.w = nw;
            self.h = nh;
            dc
        }
    }

    /// 释放所有 GDI 对象。顺序见类型文档。
    ///
    /// 这里没有 `unsafe`：三个调用都走 safe 包装
    /// （`gdi_select_object` / `gdi_delete_object` / `gdi_delete_dc`），
    /// 它们内部才碰 FFI 并负责 `Gdi` ↔ `Handle` 的空值转换。
    pub fn release(&mut self) {
        if self.dc != 0 {
            if self.old != 0 {
                gdi_select_object(self.dc, self.old);
            }
            if self.bmp != 0 {
                gdi_delete_object(self.bmp);
            }
            gdi_delete_dc(self.dc);
        }
        self.dc = 0;
        self.bmp = 0;
        self.old = 0;
        self.w = 0;
        self.h = 0;
    }

    /// 当前位图尺寸。
    pub fn size(&self) -> (i32, i32) {
        (self.w, self.h)
    }

    /// 在画布上开一次 GDI+ 绘制。
    ///
    /// - `scale`：DPI 缩放（96 DPI = 1.0）
    /// - `scroll`：内容竖直滚动量（逻辑像素）
    /// - `oy`：整块的纵向偏移（逻辑像素）。自绘标题栏占了顶部 [`super::titlebar::TITLE_H`]，
    ///   导航和页面整体要往下让出这块 —— 用世界变换做，页面代码里的坐标仍是
    ///   「从内容区顶部算起」，不用到处加常数。
    /// - `view_h`：可视区高度（逻辑像素）
    ///
    /// 闭包拿到 `Painter`，坐标全是**逻辑像素、内容坐标**。
    pub fn paint<F: FnOnce(&mut Painter<'_>)>(
        &mut self,
        fonts: &mut Fonts,
        theme: &Theme,
        scale: f32,
        scroll: f32,
        view_w: f32,
        view_h: f32,
        f: F,
    ) {
        if self.dc == 0 {
            return;
        }
        let scale = if scale > 0.1 { scale } else { 1.0 };
        unsafe {
            let mut g: *mut c_void = std::ptr::null_mut();
            if GdipCreateFromHDC(self.dc, &mut g) != OK || g.is_null() {
                return;
            }
            // 4 = AntiAlias
            GdipSetSmoothingMode(g, 4);

            // 文字渲染：按底色亮度挑 hint。
            //   浅色底用 5 = ClearTypeGridFit，256 灰阶 + 像素网格吸附，最锐利；
            //   深色底用 3 = AntiAliasGridFit，避免 ClearType 次像素在深底上留红/青彩边。
            // 用 theme.bg 的 0xAARRGGBB 取 RGB 算亮度。
            let bg = theme.bg;
            let r = ((bg >> 16) & 0xFF) as u32;
            let gg = ((bg >> 8) & 0xFF) as u32;
            let b = (bg & 0xFF) as u32;
            let lum = (r * 299 + gg * 587 + b * 114) / 1000;
            let dark = lum < 128;
            GdipSetTextRenderingHint(g, if dark { 3 } else { 5 });

            // **3 = None**：不再做 0.5px 偏移。原来写 2（注释误标 Half，其实 2 =
            // HighQuality）会让所有文字整体挪半格、糊成一片；None 才让字落回像素网格。
            GdipSetPixelOffsetMode(g, 3);

            // 世界变换：先缩放，再按 scroll 上移（内容坐标 → 设备坐标）。
            //
            // **单位变换时千万别设**：GDI+ 只要挂了 world transform，就会丢掉文字的
            // GridFit（哪怕矩阵是单位阵），文字立刻变柔和——这正是"整个软件发糊"的
            // 主因之一。scale≈1 且 scroll≈0 时本来就不需要变换，直接跳过。
            let needs_xf = (scale - 1.0).abs() > 1e-3 || scroll.abs() > 1e-3;
            if needs_xf {
                let mut m: *mut c_void = std::ptr::null_mut();
                if GdipCreateMatrix2(scale, 0.0, 0.0, scale, 0.0, -scroll * scale, &mut m) == OK
                    && !m.is_null()
                {
                    GdipSetWorldTransform(g, m);
                    GdipDeleteMatrix(m);
                }
            }

            let clip = R::new(0.0, scroll, view_w, view_h + scroll);
            let mut p = Painter { g, fonts, t: theme, clip, scale };
            f(&mut p);
            GdipDeleteGraphics(g);
        }
    }
}

impl Default for Canvas {
    fn default() -> Self {
        Canvas::new()
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        self.release();
    }
}

/// 把画布内容拷到窗口 DC 上。
///
/// 用 `SRCCOPY`（不做混合）—— 这正是双缓冲快 3 倍的原因：
/// 最后一步是纯内存拷贝，不用穿透 DWM 回读屏幕。
pub fn blit(dst: Gdi, src: Gdi, w: i32, h: i32) {
    if dst == 0 || src == 0 {
        return;
    }
    unsafe {
        BitBlt(dst, 0, 0, w, h, src, 0, 0, SRCCOPY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 矩形几何() {
        let r = R::new(10.0, 20.0, 100.0, 50.0);
        assert_eq!(r.right(), 110.0);
        assert_eq!(r.bottom(), 70.0);
        assert_eq!(r.cx(), 60.0);
        assert_eq!(r.cy(), 45.0);
        assert!(r.contains(10.0, 20.0));
        assert!(r.contains(109.9, 69.9));
        // 右边界和下边界是开区间 —— 相邻矩形不会都认为自己被点中
        assert!(!r.contains(110.0, 45.0));
        assert!(!r.contains(60.0, 70.0));
        assert!(!r.contains(9.9, 45.0));
    }

    #[test]
    fn 矩形切片() {
        let r = R::new(10.0, 20.0, 100.0, 50.0);
        assert_eq!(r.inset(5.0), R::new(15.0, 25.0, 90.0, 40.0));
        assert_eq!(r.skip_x(30.0), R::new(40.0, 20.0, 70.0, 50.0));
        assert_eq!(r.trim_x(30.0), R::new(10.0, 20.0, 70.0, 50.0));
        assert_eq!(r.tail(20.0), R::new(90.0, 20.0, 20.0, 50.0));
        assert_eq!(r.head(20.0), R::new(10.0, 20.0, 20.0, 50.0));
        // 内缩过多不能变成负尺寸（会画出反向的框）
        let tiny = r.inset(60.0);
        assert_eq!(tiny.w, 0.0);
        assert_eq!(tiny.h, 0.0);
    }

    #[test]
    fn 相交判断() {
        let a = R::new(0.0, 0.0, 100.0, 100.0);
        assert!(a.intersects(&R::new(50.0, 50.0, 100.0, 100.0)));
        assert!(a.intersects(&R::new(-50.0, -50.0, 100.0, 100.0)));
        // 只贴边不算相交（开区间）
        assert!(!a.intersects(&R::new(100.0, 0.0, 10.0, 10.0)));
        assert!(!a.intersects(&R::new(200.0, 200.0, 10.0, 10.0)));
    }

    #[test]
    fn 改alpha不改rgb() {
        assert_eq!(with_alpha(0xFFFF_0000, 0x13), 0x13FF_0000);
        assert_eq!(with_alpha(0x0040_90D9, 0xFF), 0xFF40_90D9);
    }

    #[test]
    fn 画布初始是空的且不自带位图() {
        let c = Canvas::new();
        assert_eq!(c.size(), (0, 0));
        assert_eq!(c.dc, 0);
        assert_eq!(c.bmp, 0);
    }

    #[test]
    fn gdiplus_可以启动() {
        // 这个测试在无头环境里也能过：GdiplusStartup 不依赖窗口。
        assert!(
            start(),
            "GdiplusStartup 失败 —— 检查 suppress_background_thread 是不是 0"
        );
        // 重复启动必须安全（第二次直接返回 true，不新建令牌）
        assert!(start());
    }

    #[test]
    fn 字体表能建出中文字体() {
        // 字体族找不到时 new() 返回 None，这里必须能建出来 ——
        // 否则整套界面全是空白（画字会静默跳过）。
        assert!(Fonts::new("Microsoft YaHei UI").is_some(), "Microsoft YaHei UI 建不出来");
        // 回落链必须至少有一个能成
        let f = Fonts::from_candidates(&["这个名字不存在", "Microsoft YaHei UI", "SimSun"]);
        assert!(f.is_some(), "回落链一个都建不出来");
    }

    #[test]
    fn 量文字宽度返回正数() {
        assert!(start());
        let mut fonts = Fonts::new("Microsoft YaHei UI").expect("字体");
        let mut g: *mut c_void = std::ptr::null_mut();
        unsafe {
            // 用内存 DC 建一个最小画布来量字
            let dc = gdi_create_compatible_dc(0);
            assert_ne!(dc, 0, "CreateCompatibleDC 失败");
            assert_eq!(GdipCreateFromHDC(dc, &mut g), OK);
            let t = &super::super::theme::LIGHT;
            let mut p = Painter {
                g,
                fonts: &mut fonts,
                t,
                clip: R::new(0.0, 0.0, 800.0, 600.0),
                scale: 1.0,
            };
            let (w1, h1) = p.measure("电量历史", 13.0, false);
            let (w2, _) = p.measure("电量历史电量历史", 13.0, true);
            assert!(w1 > 10.0, "中文宽度算成了 {w1}");
            assert!(h1 > 8.0, "行高算成了 {h1}");
            assert!(w2 > w1 * 1.5, "粗体且加倍的文字没有明显更宽：{w1} vs {w2}");
            GdipDeleteGraphics(g);
            gdi_delete_dc(dc);
        }
    }

    #[test]
    fn 画到内存dc不崩() {
        assert!(start());
        let mut fonts = Fonts::new("Microsoft YaHei UI").expect("字体");
        let mut canvas = Canvas::new();
        let dc = gdi_create_compatible_dc(0);
        assert_ne!(dc, 0);
        let mem = canvas.ensure(dc, 400, 300);
        assert_ne!(mem, 0, "双缓冲位图建不出来");
        assert_eq!(canvas.size(), (400, 300));

        let t = &super::super::theme::LIGHT;
        canvas.paint(&mut fonts, t, 1.0, 0.0, 400.0, 300.0, |p| {
            p.rect(R::new(0.0, 0.0, 400.0, 300.0), t.bg);
            p.card(R::new(10.0, 10.0, 200.0, 100.0), 8.0, t.card, t.border);
            p.text("电量 82%", R::new(20.0, 20.0, 180.0, 24.0), 13.0, true, t.text, Align::Left);
            p.pill(R::new(20.0, 60.0, 60.0, 20.0), "2.4G", t.green, t.green_soft, 11.0);
            p.ring(300.0, 80.0, 40.0, 8.0, 0.82, t.accent, t.accent, t.track);
            p.line(10.0, 260.0, 390.0, 280.0, t.accent, 2.0);
            p.switch(R::new(10.0, 130.0, 36.0, 20.0), true, true);
            p.checkbox(R::new(60.0, 130.0, 16.0, 16.0), true, true);
            p.slider(R::new(90.0, 130.0, 200.0, 20.0), 0.5, true);
            p.polyline(&[(10.0, 200.0), (100.0, 180.0), (200.0, 210.0)], t.accent, 2.0);
            // 面积填充（曲线图用）。传错参数通常直接 0xC0000005 或静默无输出，
            // 具体像素结果靠真机截图验收 —— 在离屏位图上断言抗锯齿边缘的颜色
            // 收益不抵成本。
            p.fill_polygon(
                &[(10.0, 200.0), (100.0, 180.0), (200.0, 210.0), (200.0, 250.0), (10.0, 250.0)],
                with_alpha(t.accent, 0x57),
                with_alpha(t.accent, 0x05),
            );
            p.round_gradient(R::new(10.0, 140.0, 100.0, 40.0), 6.0, t.accent_soft, with_alpha(t.accent_soft, 0));
        });

        // 再画一次（尺寸变化路径：位图只会变大不会变小）
        let mem2 = canvas.ensure(dc, 200, 150);
        assert_eq!(mem2, mem, "缩小尺寸时不该重建位图");
        assert_eq!(canvas.size(), (400, 300));

        // 变大会重建
        let mem3 = canvas.ensure(dc, 800, 600);
        assert_ne!(mem3, 0);
        assert_eq!(canvas.size(), (800, 600));

        canvas.release();
        assert_eq!(canvas.size(), (0, 0));
        gdi_delete_dc(dc);
    }

    /// 开一块**真彩**离屏位图模板，返回屏幕 DC。
    ///
    /// 测试里绝不能写 `gdi_create_compatible_dc(0)` 再把结果传给 `ensure()`：
    /// `CreateCompatibleDC(NULL)` 建出的内存 DC 默认选中一张 **1x1 单色位图**，
    /// 而 `ensure()` 内部用收到的 DC 去 `CreateCompatibleBitmap`
    /// —— 拿到的是 1bpp 单色画布，任何彩色都被压成黑或白
    /// （实测：红/绿/蓝填充读回全是 0xFFFFFF，只有纯黑是 0）。
    ///
    /// 生产路径没这个问题（`draw_frame` 传的就是 `GetDC(NULL)`），
    /// 但测试里传错会得到「环没画出来」的假象，非常费时间。
    #[cfg(test)]
    fn test_screen_dc() -> crate::win32::Handle {
        let screen = unsafe { crate::win32::GetDC(std::ptr::null_mut()) };
        assert!(!screen.is_null(), "GetDC(NULL) 失败");
        screen
    }

    #[cfg(test)]
    fn release_screen_dc(screen: crate::win32::Handle) {
        unsafe { crate::win32::ReleaseDC(std::ptr::null_mut(), screen) };
    }

    #[test]
    fn 滚动通过世界变换生效_不崩() {
        assert!(start());
        let mut fonts = Fonts::new("Microsoft YaHei UI").expect("字体");
        let mut canvas = Canvas::new();
        let dc = gdi_create_compatible_dc(0);
        let _ = canvas.ensure(dc, 400, 300);
        let t = &super::super::theme::DARK;
        // 滚到 1500 的位置画，内容坐标远在可视区之外 —— 不该崩，
        // 也不该把位图搞脏（visible 会跳过它们）。
        canvas.paint(&mut fonts, t, 1.5, 1500.0, 400.0, 300.0, |p| {
            p.rect(R::new(0.0, 0.0, 400.0, 100.0), t.bg);
            p.card(R::new(10.0, 1510.0, 200.0, 60.0), 8.0, t.card, t.border);
        });
        canvas.release();
        gdi_delete_dc(dc);
    }

    /// 电量环必须真的画出进度弧。
    ///
    /// 这条测试的存在理由：`ring()` 曾经用 `wrap=4` 调 `GdipCreateLineBrush`，
    /// 返回 InvalidParameter 后整段进度弧被静默跳过 —— 界面上只剩一圈灰底环，
    /// 而所有测试都通过（原测试只断言「不崩」），截图里发现时已经上线。
    /// 所以这里**必须回读像素**，不能只测不崩。
    #[test]
    fn 电量环真的画出进度弧() {
        assert!(start());
        let mut fonts = Fonts::new("Microsoft YaHei UI").expect("字体");
        let mut canvas = Canvas::new();
        let screen = test_screen_dc();
        let mem = canvas.ensure(screen as Gdi, 200, 200);
        assert_ne!(mem, 0);

        // 白底 + 75% 环，环色用纯蓝便于按通道判定。
        // COLORREF 是 0x00BBGGRR：蓝 = 0xFF0000。
        canvas.paint(&mut fonts, &super::super::theme::LIGHT, 1.0, 0.0, 200.0, 200.0, |p| {
            p.rect(R::new(0.0, 0.0, 200.0, 200.0), 0xFFFF_FFFF);
            p.ring(
                100.0,
                100.0,
                55.0,
                10.0,
                0.75,
                0xFF00_00FF, // 蓝（ARGB）
                0xFF00_00FF,
                0x0000_0000, // 不画底环，避免干扰
            );
        });

        // 弧从 12 点顺时针扫 360*0.75 = 270°，覆盖 12 → 3 → 6 → 9 点。
        // 所以 12 点（起点）、3 点、9 点（终点）都在弧内，
        // **没被覆盖的是 9 点到 12 点之间的左上象限**（GDI+ 角度 180°–270°）。
        let top = gdi_get_pixel(mem, 100, 48);
        let right = gdi_get_pixel(mem, 152, 100);
        let left = gdi_get_pixel(mem, 48, 100);
        let upper_left = gdi_get_pixel(mem, 61, 61);
        assert!(
            top.is_some() && right.is_some() && left.is_some() && upper_left.is_some(),
            "GetPixel 失败"
        );

        // COLORREF 是 0x00BBGGRR —— **低字节是红**，蓝在高字节。
        // 第一版这里写反了（按 ARGB 顺序判），报「12 点没画出进度弧」，
        // 实际弧早就画好了。
        let is_blue = |c: u32| ((c >> 16) & 0xFF) > 150 && (c & 0xFF) < 100;

        assert!(is_blue(top.unwrap()), "12 点（弧起点）没有弧，得到 {:#08x}", top.unwrap());
        assert!(is_blue(right.unwrap()), "3 点应有弧（270° 覆盖到），得到 {:#08x}", right.unwrap());
        assert!(is_blue(left.unwrap()), "9 点（弧终点）没有弧，得到 {:#08x}", left.unwrap());
        assert!(
            !is_blue(upper_left.unwrap()),
            "左上象限不该有弧（75% 只扫到 9 点），得到 {:#08x}",
            upper_left.unwrap()
        );

        canvas.release();
        release_screen_dc(screen);
    }

    /// 充电动效的彗尾必须真的画出来。
    ///
    /// 为什么单独测：`arc_comet` 是给 `ring` 叠一条弧，**两段都可能失败**——
    /// 底层进度弧走 `GdipCreateLineBrush`（曾经 wrap=4 静默失败过），
    /// 彗尾走 `GdipCreateSolidFill`。只测「不崩」的话，两者都失败时测试照样绿。
    ///
    /// 因为彗尾位置随时间变，这里**整圈扫一遍**找绿色像素，而不是钉在某一点：
    /// 钉点会让测试在特定毫秒数上偶发失败（相位每 1200ms 循环一次）。
    #[test]
    fn 充电动效的彗尾真的画出来() {
        assert!(start());
        let mut fonts = Fonts::new("Microsoft YaHei UI").expect("字体");
        let mut canvas = Canvas::new();
        let screen = test_screen_dc();
        let mem = canvas.ensure(screen as Gdi, 200, 200);
        assert_ne!(mem, 0);

        canvas.paint(&mut fonts, &super::super::theme::LIGHT, 1.0, 0.0, 200.0, 200.0, |p| {
            p.rect(R::new(0.0, 0.0, 200.0, 200.0), 0xFFFF_FFFF);
            // 绿色进度弧 + 绿色彗尾，底环不画。
            p.arc_comet(100.0, 100.0, 55.0, 10.0, 0.5, 0xFF00_FF00, 0xFF00_FF00, 0x0000_0000);
        });

        // 沿半径 55 的圆周取 360 个点（每度一个），数绿色像素。
        // 彗尾 60° + 进度弧 180°(50%) 一共约 240° 应该有色，取 60 作阈值，
        // 余量足够宽 —— 只要求「真的画了」，不锁具体角度。
        let mut green = 0;
        for deg in 0..360 {
            let a = (deg as f32).to_radians();
            // GDI+ 的 0° 在 3 点方向，屏幕 y 向下，所以 sin 直接加。
            let x = (100.0 + 55.0 * a.cos()).round() as i32;
            let y = (100.0 + 55.0 * a.sin()).round() as i32;
            if let Some(c) = gdi_get_pixel(mem, x, y) {
                // COLORREF 0x00BBGGRR：绿 = 0x00FF00 → G 通道在中间。
                if ((c >> 8) & 0xFF) > 150 && (c & 0xFF) < 100 && ((c >> 16) & 0xFF) < 100 {
                    green += 1;
                }
            }
        }
        assert!(
            green >= 60,
            "充电动效没画出来：圆周 360 点里只有 {green} 个绿色",
        );

        canvas.release();
        release_screen_dc(screen);
    }

    /// 电量环四种状态并排渲染，落一张 BMP 供人工目视检查。
    ///
    /// 用 `#[ignore]` 是因为它只产出「给人看的图」，断言价值有限；
    /// 真正的断言在 `电量环真的画出进度弧` 和 `充电动效的彗尾真的画出来`。
    /// 运行：`cargo test --release 电量环四态图 -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn 电量环四态图() {
        assert!(start());
        let mut fonts = Fonts::new("Microsoft YaHei UI").expect("字体");
        let mut canvas = Canvas::new();
        let screen = test_screen_dc();
        let (w, h) = (480, 140);
        let mem = canvas.ensure(screen as Gdi, w, h);
        assert_ne!(mem, 0);

        let t = &super::super::theme::LIGHT;
        // 四格：正常蓝 75% / 低电量红 15% / 充电绿 45% / 休眠灰。
        let cells: [(f32, u32, u32, bool); 4] = [
            (0.75, t.accent, t.accent, false),
            (0.15, t.red, t.red, false),
            (0.45, t.green, t.green, true),
            (0.0, t.track, t.track, false),
        ];
        canvas.paint(&mut fonts, t, 1.0, 0.0, w as f32, h as f32, |p| {
            p.rect(R::new(0.0, 0.0, w as f32, h as f32), 0xFFFF_FFFF);
            for (i, (frac, from, to, comet)) in cells.iter().enumerate() {
                let cx = 60.0 + i as f32 * 120.0;
                let cy = 70.0;
                if *comet {
                    p.arc_comet(cx, cy, 45.0, 9.0, *frac, *from, *to, t.track);
                } else {
                    p.ring(cx, cy, 45.0, 9.0, *frac, *from, *to, t.track);
                }
            }
        });

        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/target/test-scratch/ring-four-states.bmp"
        );
        std::fs::create_dir_all(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("test-scratch"),
        )
        .ok();
        save_bmp(path, mem, w, h);
        println!("已写出 {path}");

        canvas.release();
        release_screen_dc(screen);
    }

    /// 卡片渐变也必须真的画出来（同一批 `wrap=4` 受害者）。
    #[test]
    fn 卡片渐变真的画出颜色() {
        assert!(start());
        let mut fonts = Fonts::new("Microsoft YaHei UI").expect("字体");
        let mut canvas = Canvas::new();
        let screen = test_screen_dc();
        let mem = canvas.ensure(screen as Gdi, 120, 120);
        assert_ne!(mem, 0);

        canvas.paint(&mut fonts, &super::super::theme::LIGHT, 1.0, 0.0, 120.0, 120.0, |p| {
            p.rect(R::new(0.0, 0.0, 120.0, 120.0), 0xFFFF_FFFF);
            // 上端纯红、下端纯蓝，中点应当既不是白也不是红/蓝端点。
            p.round_gradient(
                R::new(20.0, 20.0, 80.0, 80.0),
                0.0,
                0xFF00_0000, // 不透明黑（上）
                0xFFFF_0000, // 不透明蓝（下，ARGB 顺序）
            );
        });

        let mid = gdi_get_pixel(mem, 60, 60).expect("GetPixel 失败");
        // 渐变没画出来时这里会是白底 0x00FFFFFF。
        assert_ne!(mid, 0x00FF_FFFF, "卡片渐变没画出来（还是白底）");

        canvas.release();
        release_screen_dc(screen);
    }

    #[test]
    fn 渲染前端ui效果图() {
        assert!(start());
        let mut fonts = Fonts::new("Microsoft YaHei UI").expect("字体");
        let mut canvas = Canvas::new();
        let screen = test_screen_dc();
        let w = 1100;
        let h = 740;
        let mem = canvas.ensure(screen as Gdi, w, h);
        assert_ne!(mem, 0);

        let ident = crate::proto::Identity {
            model_name: "ROG HARPE ACE MINI".to_string(),
            wired_pid: 0x1B63,
            wireless_pid: 0x1B65,
            mouse_sn: "B00000000000".to_string(),
            dongle_sn: "C00000000001".to_string(),
            mouse_mac: "00:00:00:00:00:00".to_string(),
            dongle_mac: "00:00:00:00:00:00".to_string(),
            dongle_firmware: "7.00.07".to_string(),
            booster: false,
        };
        let state = crate::proto::State {
            power: 78,
            charging: 0,
            sleep_mode: 4,
            low_warn: 20,
            poll_code: 3,
            // 去抖给 4 = 20 ms（出厂默认）。留 0 会显示成「关闭」，快照里像没启用。
            debounce_code: 4,
            active_stage: 2,
            // enc = (dpi - 50) / 50，写真实鼠标的默认四档，快照才看得懂
            enc: [7, 15, 31, 63], // 400 / 800 / 1600 / 3200
            rgb: [
                [0xFF, 0x19, 0x29],
                [0xFF, 0x00, 0xFF],
                [0x00, 0x00, 0xFF],
                [0x00, 0xFF, 0x00],
            ],
            // 按键用出厂默认（每个键映射到自己）。留全 0 会让按键页八行都显示
            // 「无功能」，看着像界面把鼠标按键全废掉了 —— 快照/上架图都不能这样。
            buttons: crate::proto::BUTTON_SRC,
            ..Default::default()
        };
        let cfg = crate::settings::Settings::default();
        // 已知结论的估算，快照才看得到「环下方那行」的真实排版。
        let estimate = crate::history::Estimate {
            known: true,
            text: "预计还能用 3 小时 12 分钟".to_string(),
            percent_per_hour: -6.2,
            samples: 14,
            span_minutes: 96.0,
            hours: 3.2,
            stale_seconds: 0,
        };
        let data = crate::ui::pages::Data {
            state: Some(&state),
            ident: Some(&ident),
            status: String::new(),
            plot: None,
            estimate: &estimate,
            rows: 0,
            cfg: &cfg,
            autostart_text: "已开机自启",
            version: "v0.1.0",
            frames: 19,
            elapsed_ms: 588,
            from_cache: false,
        };

        let out_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scratch");
        std::fs::create_dir_all(&out_dir).ok();

        let render_full_view = |canvas: &mut Canvas, fonts: &mut Fonts, ui: &mut super::super::widget::Ui, data_ref: &super::super::pages::Data, theme: &super::super::theme::Theme, out_name: &str, draw_extra: Option<&dyn Fn(&mut Painter, &super::super::layout::Frame)>, scroll: f32| {
            let oy = super::super::titlebar::TITLE_H;
            let nav_w = ui.nav_width();
            let mut frame = super::super::layout::Frame::new(w as f32, h as f32, nav_w);
            frame.content.x += nav_w;
            frame.content.y += oy;
            frame.content.h -= oy;

            // ① 整窗铺底
            canvas.paint(fonts, theme, 1.0, 0.0, w as f32, h as f32, |p| {
                p.rect(super::R::new(0.0, 0.0, w as f32, h as f32), theme.bg);
            });
            // ② 页面
            canvas.paint(fonts, theme, 1.0, scroll, w as f32, h as f32, |p| {
                super::super::pages::page(p, ui, data_ref, &frame);
                if let Some(extra) = draw_extra {
                    extra(p, &frame);
                }
            });
            // ③ 导航栏
            canvas.paint(fonts, theme, 1.0, 0.0, nav_w, h as f32, |p| {
                super::super::pages::nav(p, ui, data_ref, nav_w, h as f32);
            });
            // ④ 标题栏
            canvas.paint(fonts, theme, 1.0, 0.0, w as f32, h as f32, |p| {
                super::super::titlebar::draw(p, w as f32, nav_w, theme);
                super::super::titlebar::draw_buttons(p, w as f32, theme, false, ui.hover_btn);
            });
            // ⑤ 模态弹窗（若有）
            if ui.show_close_dialog {
                canvas.paint(fonts, theme, 1.0, 0.0, w as f32, h as f32, |p| {
                    super::super::pages::draw_close_dialog(p, ui, w as f32, h as f32);
                });
            }

            let out_file = out_dir.join(out_name);
            save_bmp(out_file.to_str().unwrap(), mem, w, h);
        };

        // 1. 电量页 浅色模式
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Battery;
            ui.rail = true;
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::LIGHT, "ui-battery-light.bmp", None, 0.0);
        }

        // 2. 电量页 深色模式
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Battery;
            ui.rail = true;
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::DARK, "ui-battery-dark.bmp", None, 0.0);
        }

        // 3. 性能页 + 预设色盘
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Perf;
            ui.rail = true;
            ui.palette_tab = 0;
            let extra = |p: &mut Painter, f: &super::super::layout::Frame| {
                super::super::pages::draw_palette(
                    p,
                    &mut super::super::widget::Ui::default(),
                    0,
                    super::R::new(320.0, 180.0, 36.0, 24.0),
                    Some([255, 0, 0]),
                    f,
                );
            };
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::LIGHT, "ui-palette-preset.bmp", Some(&extra), 0.0);
        }

        // 4. 性能页 + 高级自定义色环
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Perf;
            ui.rail = true;
            ui.palette_tab = 1;
            ui.custom_hue = 330.0;
            let extra = |p: &mut Painter, f: &super::super::layout::Frame| {
                let mut ui_p = super::super::widget::Ui::default();
                ui_p.palette_tab = 1;
                ui_p.custom_hue = 330.0;
                super::super::pages::draw_palette(
                    p,
                    &mut ui_p,
                    0,
                    super::R::new(320.0, 180.0, 36.0, 24.0),
                    Some([255, 0, 128]),
                    f,
                );
            };
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::LIGHT, "ui-palette-ring.bmp", Some(&extra), 0.0);
        }

        // 5. 电量页 正在充电状态（动效光晕与徽标）
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Battery;
            ui.rail = true;
            let state_charging = crate::proto::State {
                charging: 1,
                power: 58,
                ..state.clone()
            };
            let estimate_charging = crate::history::Estimate {
                known: true,
                text: "预计 40 分钟后充满".to_string(),
                percent_per_hour: 62.0,
                samples: 9,
                span_minutes: 41.0,
                hours: 0.68,
                stale_seconds: 0,
            };
            let data_charging = crate::ui::pages::Data {
                state: Some(&state_charging),
                status: String::new(),
                estimate: &estimate_charging,
                ..data.clone()
            };
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data_charging, &super::super::theme::DARK, "ui-battery-charging.bmp", None, 0.0);
        }

        // 6. 关闭确认模态对话框
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Battery;
            ui.rail = true;
            ui.show_close_dialog = true;
            ui.close_choice = super::super::widget::CloseChoice::Minimize;
            ui.close_remember = true;
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::DARK, "ui-close-dialog.bmp", None, 0.0);
        }

        // 7. 电量页 展开导航（rail=false，验证设备卡不再被标题栏裁切，#1）
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Battery;
            ui.rail = false;
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::LIGHT, "ui-battery-expanded.bmp", None, 0.0);
        }

        // 8. 性能页 高级折叠收起（排查折叠区色差）
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Perf;
            ui.rail = true;
            ui.adv = [false, false];
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::LIGHT, "ui-perf-adv-closed.bmp", None, 560.0);
        }

        // 9. 性能页 高级折叠展开（排查展开后色差）
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Perf;
            ui.rail = true;
            ui.adv = [true, false];
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::LIGHT, "ui-perf-adv-open.bmp", None, 560.0);
        }

        // 10. 按键页（警告条置顶 + 现代行样式）
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Keys;
            ui.rail = true;
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::LIGHT, "ui-keys.bmp", None, 0.0);
        }

        // 11. 按键页 深色
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Keys;
            ui.rail = true;
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::DARK, "ui-keys-dark.bmp", None, 0.0);
        }

        // 12. 成功提示 toast
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Perf;
            ui.rail = true;
            ui.set_status("✔ 回报率已生效");
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::LIGHT, "ui-toast-ok.bmp", None, 0.0);
        }

        // 13. 错误提示 toast
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Perf;
            ui.rail = true;
            ui.set_status("✘ 开机自启失败：权限不足 (错误 5)");
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::LIGHT, "ui-toast-err.bmp", None, 0.0);
        }

        // 14. 深色 toast
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Perf;
            ui.rail = true;
            ui.set_status("✔ 灯光颜色已生效");
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::DARK, "ui-toast-dark.bmp", None, 0.0);
        }

        // 15. 性能页 LOD 卡片（警告条在卡片外）
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Perf;
            ui.rail = true;
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::LIGHT, "ui-perf-lod.bmp", None, 620.0);
        }

        // 16. 去抖下拉框：靠近视口底部时必须向上翻开（否则整块弹层落到窗外）
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Perf;
            ui.rail = true;
            ui.set_open_drop(Some(super::super::widget::Drop::Debounce));
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::LIGHT, "ui-perf-drop-up.bmp", None, 0.0);
        }

        // 17. DPI 数值框正在编辑：草稿顶到上限（5 位）也不能溢出框外
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Perf;
            ui.rail = true;
            ui.begin_edit(super::super::widget::Edit::Dpi(0), "42000".into());
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::LIGHT, "ui-dpi-edit.bmp", None, 0.0);
        }

        // 18. 灯光页：交给 Windows 动态光效（方案 A）
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Light;
            ui.rail = true;
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::LIGHT, "ui-light-dyn.bmp", None, 0.0);
        }

        // 19. 设置页：存储路径卡 + 外观「跟随系统/浅色/深色」三段
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Settings;
            ui.rail = true;
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::LIGHT, "ui-settings.bmp", None, 620.0);
        }

        // 20. 性能页顶部：DPI 档位每行带「切换」+「应用」
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Perf;
            ui.rail = true;
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::LIGHT, "ui-perf-top.bmp", None, 0.0);
        }

        // 21. 设置页顶部（通用设置：开机自启等）
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Settings;
            ui.rail = true;
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::LIGHT, "ui-settings-top.bmp", None, 0.0);
        }

        // 22. 标题栏关闭按钮悬停：红底 + 白叉（必须都可见，不能是透明 alpha）
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Battery;
            ui.rail = true;
            ui.hover_btn = Some(super::super::titlebar::Btn::Close);
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::LIGHT, "ui-titlebar-hover.bmp", None, 0.0);
        }

        // 23. 性能页深色：上架图里的深色主题示例（四档彩色指示灯 + 药丸分段控件）
        {
            let mut ui = super::super::widget::Ui::default();
            ui.page = super::super::widget::Page::Perf;
            ui.rail = true;
            render_full_view(&mut canvas, &mut fonts, &mut ui, &data, &super::super::theme::DARK, "ui-perf-dark.bmp", None, 0.0);
        }

        canvas.release();
        release_screen_dc(screen);
    }
}
