
use std::collections::HashMap;
use std::ffi::c_void;

use super::theme::Theme;

// ==================== 类型 ====================

// 句柄类型的唯一定义在 `crate::win32`；这里再导出，保持 `ui::gfx::Gdi` 这条路可用。
pub use crate::win32::Gdi;

/// 成功的 GDI+ 返回值。
pub const OK: i32 = 0;

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

// ==================== FFI ====================

#[link(name = "gdiplus")]
extern "system" {
    // 位图图标专用的 5 个入口。托盘图标（`crate::tray_icon`）要用，但它不该
    // 为这 5 个再开一份 `#[link(name = "gdiplus")]` —— 一个 DLL 只有一份声明。
    pub(crate) fn GdipCreateBitmapFromScan0(
        width: i32,
        height: i32,
        stride: i32,
        format: i32,
        scan0: *const u8,
        bitmap: *mut *mut c_void,
    ) -> i32;
    pub(crate) fn GdipDisposeImage(image: *mut c_void) -> i32;
    fn GdipCreateBitmapFromHICON(hicon: crate::win32::Handle, out: *mut *mut c_void) -> i32;
    fn GdipSetClipPath(g: *mut c_void, path: *mut c_void, combine: i32) -> i32;
    fn GdipDrawImageRect(
        g: *mut c_void,
        image: *mut c_void,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) -> i32;
    pub(crate) fn GdipGetImageGraphicsContext(image: *mut c_void, g: *mut *mut c_void) -> i32;
    pub(crate) fn GdipGraphicsClear(g: *mut c_void, color: u32) -> i32;
    pub(crate) fn GdipCreateHICONFromBitmap(bitmap: *mut c_void, hicon: *mut *mut c_void) -> i32;

    fn GdiplusStartup(
        token: *mut usize,
        input: *const GdiplusStartupInput,
        out: *mut c_void,
    ) -> i32;

    fn GdipCreateFromHDC(hdc: Gdi, g: *mut *mut c_void) -> i32;
    pub fn GdipDeleteGraphics(g: *mut c_void) -> i32;
    pub fn GdipSetSmoothingMode(g: *mut c_void, mode: i32) -> i32;
    pub fn GdipSetTextRenderingHint(g: *mut c_void, hint: i32) -> i32;
    fn GdipSetClipRect(
        g: *mut c_void,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        combine: i32,
    ) -> i32;

/// 清掉裁剪面。共享 Graphics 时上一层装过的裁剪必须复位，否则下一层被误裁。
fn GdipResetClip(g: *mut c_void) -> i32;
    pub(crate) fn GdipSetPixelOffsetMode(g: *mut c_void, mode: i32) -> i32;
    pub(crate) fn GdipDrawImageRectI(
        g: *mut c_void,
        img: *mut c_void,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> i32;
    pub(crate) fn GdipFillRectangleI(
        g: *mut c_void,
        brush: *mut c_void,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> i32;
    pub(crate) fn GdipSetCompositingMode(g: *mut c_void, mode: i32) -> i32;
    pub(crate) fn GdipSetInterpolationMode(g: *mut c_void, mode: i32) -> i32;

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
    fn GdipResetWorldTransform(g: *mut c_void) -> i32;

    pub fn GdipCreateSolidFill(argb: u32, brush: *mut *mut c_void) -> i32;
    pub fn GdipDeleteBrush(brush: *mut c_void) -> i32;
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
    fn GdipAddPathString(
        path: *mut c_void,
        text: *const u16,
        len: i32,
        family: *mut c_void,
        style: i32,
        size: f32,
        layout: *const RectF,
        format: *mut c_void,
    ) -> i32;
    fn GdipGetPathWorldBounds(
        path: *mut c_void,
        bounds: *mut RectF,
        matrix: *mut c_void,
        pen: *mut c_void,
    ) -> i32;
    fn GdipTransformPath(path: *mut c_void, matrix: *mut c_void) -> i32;
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
    pub(crate) fn GdipFillEllipse(
        g: *mut c_void,
        brush: *mut c_void,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) -> i32;

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
    pub fn GdipSetStringFormatAlign(fmt: *mut c_void, align: i32) -> i32;
    pub fn GdipSetStringFormatLineAlign(fmt: *mut c_void, align: i32) -> i32;
    fn GdipSetStringFormatTrimming(fmt: *mut c_void, trimming: i32) -> i32;
    pub fn GdipSetStringFormatFlags(fmt: *mut c_void, flags: i32) -> i32;
    pub fn GdipDeleteStringFormat(fmt: *mut c_void) -> i32;

    pub fn GdipDrawString(
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

    /// 释放所有已缓存的字号，**保留字体家族**。
    ///
    /// 窗口最小化/收进托盘时调用：`GpFont` 只有绘制时才用得上，藏着等于白占
    /// 内存。恢复后 `get()` 按需重建（每个字号一次 `GdipCreateFont`）。
    pub fn clear(&mut self) {
        for (_, f) in self.cache.drain() {
            if !f.is_null() {
                unsafe { GdipDeleteFont(f) };
            }
        }
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

    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

}

/// 文字在矩形里的对齐方式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// 应用图标解码一次就缓存：exe 资源 1 号 ICON（`assets/app.rc` 里的
/// `1 ICON "tray.ico"`，也就是用户给的那张原画生成的 ico）。
///
/// 不用 `include_bytes!` + 流解码：图标已经在资源里，`LoadImageW` 直接给
/// HICON，`GdipCreateBitmapFromHICON` 再转成可绘制位图，省掉解码器。
pub fn app_icon_bitmap() -> *mut c_void {
    static CACHE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| unsafe {
        let hicon = crate::win32::LoadImageW(
            crate::win32::GetModuleHandleW(std::ptr::null()),
            // 资源 2 = 标题栏专用的小图标（从原画裁到角色占满画面），
            // 资源 1 是应用/托盘图标，两者构图不同。
            2 as *const u16,
            crate::win32::IMAGE_ICON,
            48,
            48,
            0,
        );
        if hicon.is_null() {
            return 0;
        }
        let mut bmp: *mut c_void = std::ptr::null_mut();
        if GdipCreateBitmapFromHICON(hicon, &mut bmp) != 0 {
            return 0;
        }
        // HICON 留着不销毁：位图可能仍引用它，进程退出时一并回收。
        bmp as usize
    }) as *mut c_void
}

// ==================== 绘制上下文 ====================

/// 一次绘制的上下文：一个 GDI+ Graphics + 一套配色 + 字体表。
///
/// 页面代码只拿 `&mut Painter`，拿不到裸指针，也就不会误建 Graphics。
///
/// **所有坐标都是逻辑像素、内容坐标**（y=0 是内容顶部，不是窗口顶部）。
/// 缩放和滚动由 Graphics 的世界变换处理，见模块文档。
/// 一帧内复用的 GDI+ 对象表。
///
/// 绘制层原来每个图元都现场 `GdipCreateSolidFill` / `GdipCreatePen1` 再立刻
/// `Delete`，单帧数百对 FFI 调用。同一帧里颜色种类其实很少（十几种），所以按
/// 参数缓存起来、帧末统一释放，命中率接近 100%。
///
/// 生命周期严格限定在一帧内（`paint()` 建、画完 `clear()`）：GDI+ 对象跨帧
/// 持有会被 DPI 变化或设备丢失搞成野句柄，收益不值这个风险。
/// 一层的可视区（世界坐标，滚动已经折算进去）。
///
/// 和 `Painter.clip`（图元剔除用的那个）**逐字一致**，包括高度多带的
/// `scroll`：裁剪面若比剔除面小，就会出现「visible() 说该画、GDI+ 却裁掉」
/// 的半屏空白。宁可多画一点点，也不让两者分叉。
fn clip_rect_for(scroll: f32, view_w: f32, view_h: f32) -> R {
    R::new(0.0, scroll, view_w, view_h + scroll)
}

#[derive(Default)]
struct ObjCache {
    brushes: Vec<(u32, *mut c_void)>,
    /// 键：`(argb, width 的位模式, 是否圆头圆角)`。
    pens: Vec<((u32, u32, bool), *mut c_void)>,
    /// 键：`(水平对齐, 是否单行)`。
    fmts: Vec<((i32, bool), *mut c_void)>,
}

impl ObjCache {
    fn brush(&mut self, argb: u32) -> *mut c_void {
        for (c, h) in &self.brushes {
            if *c == argb {
                return *h;
            }
        }
        let mut b: *mut c_void = std::ptr::null_mut();
        if unsafe { GdipCreateSolidFill(argb, &mut b) } != OK {
            return std::ptr::null_mut();
        }
        self.brushes.push((argb, b));
        b
    }

    fn pen(&mut self, argb: u32, width: f32) -> *mut c_void {
        self.pen_inner(argb, width, false)
    }

    /// 圆头圆角笔：折线与曲线要它，否则拐角出尖刺。
    ///
    /// 圆头和方头必须有各自的缓存条目 —— 圆头笔帽会把线端各延长半个线宽，
    /// 拿圆头笔去画 UI 边框会在四个角上露出小凸起。
    fn pen_round(&mut self, argb: u32, width: f32) -> *mut c_void {
        self.pen_inner(argb, width, true)
    }

    fn pen_inner(&mut self, argb: u32, width: f32, round: bool) -> *mut c_void {
        let key = (argb, width.to_bits(), round);
        for (k, h) in &self.pens {
            if *k == key {
                return *h;
            }
        }
        let mut p: *mut c_void = std::ptr::null_mut();
        if unsafe { GdipCreatePen1(argb, width, 2, &mut p) } != OK {
            return std::ptr::null_mut();
        }
        if round {
            unsafe {
                // 2 = Round
                GdipSetPenLineJoin(p, 2);
                GdipSetPenStartCap(p, 2);
                GdipSetPenEndCap(p, 2);
            }
        }
        self.pens.push((key, p));
        p
    }

    /// 单行文字格式（超宽省略号 + 禁折行）。`align` 0=左 1=中 2=右。
    fn fmt_oneline(&mut self, align: i32) -> *mut c_void {
        self.fmt_inner(align, true)
    }

    /// 多行文字格式（竖直靠上、允许折行）。
    fn fmt_wrap(&mut self) -> *mut c_void {
        self.fmt_inner(0, false)
    }

    fn fmt_inner(&mut self, align: i32, oneline: bool) -> *mut c_void {
        let key = (align, oneline);
        for (k, h) in &self.fmts {
            if *k == key {
                return *h;
            }
        }
        let mut fmt: *mut c_void = std::ptr::null_mut();
        if unsafe { GdipCreateStringFormat(0, 0, &mut fmt) } != OK {
            return std::ptr::null_mut();
        }
        unsafe {
            GdipSetStringFormatAlign(fmt, align);
            if oneline {
                // 1 = Center（竖直居中）
                GdipSetStringFormatLineAlign(fmt, 1);
                // 3 = EllipsisCharacter：超宽截断加省略号
                GdipSetStringFormatTrimming(fmt, 3);
                // 0x0000_1000 = StringFormatFlagsNoWrap。光设 Trimming 不够：
                // GDI+ 默认允许自动折行，一串没有空格的连续字符（输入框里的
                // `400900`）会在框宽处硬断成两行并溢出框外。
                GdipSetStringFormatFlags(fmt, 0x0000_1000);
            } else {
                // 0 = Near（竖直靠上）
                GdipSetStringFormatLineAlign(fmt, 0);
            }
        }
        self.fmts.push((key, fmt));
        fmt
    }

    fn clear(&mut self) {
        for (_, h) in self.brushes.drain(..) {
            unsafe {
                GdipDeleteBrush(h);
            }
        }
        for (_, h) in self.pens.drain(..) {
            unsafe {
                GdipDeletePen(h);
            }
        }
        for (_, h) in self.fmts.drain(..) {
            unsafe {
                GdipDeleteStringFormat(h);
            }
        }
    }
}

pub struct Painter<'a> {
    g: *mut c_void,
    /// 本帧的 GDI+ 对象缓存（见 [`ObjCache`]）。
    obj: &'a mut ObjCache,
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
    pub(crate) unsafe fn on_graphics<F: FnOnce(&mut Painter<'_>)>(
        g: *mut c_void,
        theme: &Theme,
        size: f32,
        scale: f32,
        draw: F,
    ) -> Option<()> {
        let mut fonts = Fonts::from_candidates(&["Segoe UI Semibold", "Segoe UI", "Arial"])?;
        let mut matrix = std::ptr::null_mut();
        if GdipCreateMatrix2(scale, 0.0, 0.0, scale, 0.0, 0.0, &mut matrix) != OK {
            return None;
        }
        let status = GdipSetWorldTransform(g, matrix);
        GdipDeleteMatrix(matrix);
        if status != OK {
            return None;
        }
        let mut obj = ObjCache::default();
        {
            let mut p = Painter {
                g,
                obj: &mut obj,
                fonts: &mut fonts,
                t: theme,
                clip: R::new(0.0, 0.0, size, size),
                scale,
            };
            draw(&mut p);
        }
        obj.clear();
        Some(())
    }
    /// 整块是否在可见区内。
    fn visible(&self, r: R) -> bool {
        r.y < self.clip.bottom()
            && r.bottom() > self.clip.y
            && r.x < self.clip.right()
            && r.right() > self.clip.x
    }

    /// 文字落在物理像素栅格上。GDI+ 在半个像素上栅格化时，
    /// GridFit 会被迫做灰度抗锯齿，笔画越细越像“糊了一层”。
    fn snap_scalar(&self, value: f32) -> f32 {
        let scale = if self.scale > 0.01 { self.scale } else { 1.0 };
        (value * scale).round() / scale
    }

    fn snapped_text_rect(&self, r: R) -> R {
        let x = self.snap_scalar(r.x);
        let y = self.snap_scalar(r.y);
        let right = self.snap_scalar(r.right()).max(x + 1.0);
        let bottom = self.snap_scalar(r.bottom()).max(y + 1.0);
        R::new(x, y, right - x, bottom - y)
    }

    fn snapped_font_size(&self, size: f32) -> f32 {
        self.snap_scalar(size).max(1.0)
    }

    /// 在矩形内临时裁剪绘制。
    pub(crate) fn with_clip(&mut self, rect: R, draw: impl FnOnce(&mut Self)) {
        let left = self.clip.x.max(rect.x);
        let top = self.clip.y.max(rect.y);
        let right = self.clip.right().min(rect.right());
        let bottom = self.clip.bottom().min(rect.bottom());
        if right <= left || bottom <= top {
            return;
        }
        let old = self.clip;
        let clipped = R::new(left, top, right - left, bottom - top);
        self.clip = clipped;
        unsafe {
            GdipSetClipRect(self.g, clipped.x, clipped.y, clipped.w, clipped.h, 0);
        }
        draw(self);
        self.clip = old;
        unsafe {
            GdipSetClipRect(self.g, old.x, old.y, old.w, old.h, 0);
        }
    }

    // ---- 基本图元 ----

    /// 实心矩形。走 GDI+ 而不是 GDI 的 `FillRect`，
    /// 因为这里可能是带 alpha 的颜色。
    pub fn rect(&mut self, r: R, color: u32) {
        if !self.visible(r) {
            return;
        }
        let b = self.obj.brush(color);
        if b.is_null() {
            return;
        }
        unsafe {
            GdipFillRectangle(self.g, b, r.x, r.y, r.w, r.h);
        }
    }

    /// 画位图并裁成圆角。
    ///
    /// 应用图标自带的浅蓝底是方的，直接贴到侧栏上会露出一块生硬的白边方角；
    /// 裁成圆角后和整个界面的圆角语言一致。
    pub fn image_rounded(&mut self, img: *mut c_void, r: R, radius: f32) {
        if img.is_null() || !self.visible(r) {
            return;
        }
        let g = self.g;
        self.with_path(r, radius, |path| unsafe {
            // 1 = Intersect：与已有裁剪取交集，不会把外面画脏
            GdipSetClipPath(g, path, 1);
            // 256px 原图缩到 24px，默认插值会糊成一团：换高质量双三次 +
            // 高质量像素偏移，边缘才立得住。
            GdipSetInterpolationMode(g, 7);
            GdipSetPixelOffsetMode(g, 2);
            GdipDrawImageRect(g, img, r.x, r.y, r.w, r.h);
            GdipSetInterpolationMode(g, 0);
            GdipSetPixelOffsetMode(g, 3);
            GdipResetClip(g);
        });
    }

    /// 圆角实心矩形。
    pub fn round(&mut self, r: R, radius: f32, color: u32) {
        if !self.visible(r) {
            return;
        }
        let b = self.obj.brush(color);
        if b.is_null() {
            return;
        }
        let g = self.g;
        self.with_path(r, radius, |p| unsafe {
            GdipFillPath(g, b, p);
        });
    }

    /// 圆角矩形：填充 + 1px 描边（卡片就是这个）。
    pub fn card(&mut self, r: R, radius: f32, fill: u32, stroke: u32) {
        if !self.visible(r) {
            return;
        }
        let g = self.g;
        let b = self.obj.brush(fill);
        // alpha = 0 的描边等于「不描边」。透明色的边框画出来其实看不见，
        // 但会多一次 path stroke，能省就省。
        let pen = if stroke >> 24 != 0 {
            self.obj.pen(stroke, 1.0)
        } else {
            std::ptr::null_mut()
        };
        self.with_path(r, radius, |p| unsafe {
            if !b.is_null() {
                GdipFillPath(g, b, p);
            }
            if !pen.is_null() {
                GdipDrawPath(g, pen, p);
            }
        });
    }

    /// 圆角描边（无填充）。用于「悬停边框」「输入框」。
    pub fn round_stroke(&mut self, r: R, radius: f32, color: u32, width: f32) {
        if !self.visible(r) || color >> 24 == 0 {
            return;
        }
        let g = self.g;
        let pen = self.obj.pen(color, width);
        if pen.is_null() {
            return;
        }
        self.with_path(r, radius, |p| unsafe {
            GdipDrawPath(g, pen, p);
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
        let pen = self.obj.pen(color, width);
        if pen.is_null() {
            return;
        }
        unsafe {
            GdipDrawLine(self.g, pen, x1, y1, x2, y2);
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
        if !self.visible(Painter::bbox(pts)) {
            return;
        }
        let pen = self.obj.pen_round(color, width);
        if pen.is_null() {
            return;
        }
        unsafe {
            for w in pts.windows(2) {
                GdipDrawLine(self.g, pen, w[0].0, w[0].1, w[1].0, w[1].1);
            }
        }
    }

    /// 一串点的包围盒。四遍 `map().fold()` 遍历同一串点纯属浪费。
    fn bbox(pts: &[(f32, f32)]) -> R {
        let (mut x0, mut y0) = (f32::INFINITY, f32::INFINITY);
        let (mut x1, mut y1) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        for p in pts {
            x0 = x0.min(p.0);
            y0 = y0.min(p.1);
            x1 = x1.max(p.0);
            y1 = y1.max(p.1);
        }
        R::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// 整条折线当作**一条路径**一次描边。
    ///
    /// `polyline` 是逐段 `GdipDrawLine`，抗锯齿按段独立计算，接缝处的覆盖
    /// 会被叠两次或漏一次，采样点密集时看起来像一串小珠子。曲线（几百到
    /// 上千个点）用这个版本：光栅化一次完成，边缘连续。
    pub fn stroke_path(&mut self, pts: &[(f32, f32)], color: u32, width: f32) {
        if pts.len() < 2 {
            return;
        }
        if !self.visible(Painter::bbox(pts)) {
            return;
        }
        unsafe {
            let mut path: *mut c_void = std::ptr::null_mut();
            if GdipCreatePath(0, &mut path) != OK {
                return;
            }
            // `GdipAddPathLines` 不在 vendor/mingw-import-libs 的导入表里
            // （链接期 undefined reference），只能逐段加。
            for w in pts.windows(2) {
                GdipAddPathLine(path, w[0].0, w[0].1, w[1].0, w[1].1);
            }
            let pen = self.obj.pen_round(color, width);
            if !pen.is_null() {
                GdipDrawPath(self.g, pen, path);
            }
            GdipDeletePath(path);
        }
    }

    pub fn fill_polygon(&mut self, pts: &[(f32, f32)], top: u32, bottom: u32) {
        if pts.len() < 3 {
            return;
        }
        let bb = Painter::bbox(pts);
        if !self.visible(bb) {
            return;
        }
        unsafe {
            let mut path: *mut c_void = std::ptr::null_mut();
            if GdipCreatePath(0, &mut path) != OK {
                return;
            }
            // 每条边显式从上一个点连到当前点；不要只传 (p, p) 指望库自动接。
            for w in pts.windows(2) {
                GdipAddPathLine(path, w[0].0, w[0].1, w[1].0, w[1].1);
            }
            GdipClosePathFigure(path);

            let mut brush: *mut c_void = std::ptr::null_mut();
            // wrap 用 0 不用 4：线性渐变画刷不支持 Clamp，传 4 返 InvalidParameter
            // 会让整块面积填充静默消失。详见 `WRAP_TILE`。
            let rc = GdipCreateLineBrush(
                &PointF { x: 0.0, y: bb.y },
                &PointF { x: 0.0, y: bb.bottom() },
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
            // 平头笔：色环由 36 段圆弧拼成，圆头笔帽会让相邻段互相咬进去
            let pen = self.obj.pen(color, width);
            if !pen.is_null() {
                let mut path: *mut c_void = std::ptr::null_mut();
                if GdipCreatePath(0, &mut path) == OK {
                    GdipAddPathArc(path, box_r.x, box_r.y, box_r.w, box_r.h, start_deg, sweep_deg);
                    GdipDrawPath(self.g, pen, path);
                    GdipDeletePath(path);
                }
            }
        }
    }

    // ---- 文字 ----
    pub(crate) fn icon_text(&mut self, text: &str, r: R, size: f32, color: u32, compact: bool) {
        let brush = self.obj.brush(color);
        if text.is_empty() || brush.is_null() {
            return;
        }
        unsafe {
            let mut path = std::ptr::null_mut();
            if GdipCreatePath(0, &mut path) != OK {
                return;
            }
            let digits: Vec<u16> = text.encode_utf16().collect();
            let layout = RectF {
                x: 0.0,
                y: 0.0,
                w: 512.0,
                h: 512.0,
            };
            let mut bounds = RectF::default();
            if GdipAddPathString(
                path,
                digits.as_ptr(),
                digits.len() as i32,
                self.fonts.family,
                0,
                size,
                &layout,
                std::ptr::null_mut(),
            ) == OK
                && GdipGetPathWorldBounds(
                    path,
                    &mut bounds,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                ) == OK
                && bounds.w > 0.0
                && bounds.h > 0.0
            {
                // 纯数字保持字形比例；组合模式保留窄体，给设备图案留出空间。
                let mut reference = std::ptr::null_mut();
                let mut reference_bounds = bounds;
                if !compact && GdipCreatePath(0, &mut reference) == OK {
                    let zeros = [b'0' as u16; 3];
                    if GdipAddPathString(
                        reference,
                        zeros.as_ptr(),
                        digits.len().clamp(2, 3) as i32,
                        self.fonts.family,
                        0,
                        size,
                        &layout,
                        std::ptr::null_mut(),
                    ) == OK
                    {
                        GdipGetPathWorldBounds(
                            reference,
                            &mut reference_bounds,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                        );
                    }
                    GdipDeletePath(reference);
                }
                let cap_height = reference_bounds.h.max(bounds.h);
                let scale = (r.w / bounds.w.max(reference_bounds.w)).min(r.h / cap_height);
                let (sx, sy) = if compact {
                    (r.w / bounds.w, r.h / bounds.h)
                } else {
                    (scale, scale)
                };
                let x = r.cx() - (bounds.x + bounds.w / 2.0) * sx;
                let y = r.cy() - (bounds.y + bounds.h / 2.0) * sy;
                let mut matrix = std::ptr::null_mut();
                if GdipCreateMatrix2(sx, 0.0, 0.0, sy, x, y, &mut matrix) == OK {
                    GdipTransformPath(path, matrix);
                    GdipFillPath(self.g, brush, path);
                    GdipDeleteMatrix(matrix);
                }
            }
            GdipDeletePath(path);
        }
    }

    /// 画一行字。矩形决定对齐范围，垂直居中。
    pub fn text(&mut self, s: &str, r: R, size: f32, bold: bool, color: u32, align: Align) {
        let r = self.snapped_text_rect(r);
        let size = self.snapped_font_size(size);
        if !self.visible(r) || s.is_empty() {
            return;
        }
        let font = self.fonts.get(size, bold);
        if font.is_null() {
            return;
        }
        let u: Vec<u16> = s.encode_utf16().collect();
        let layout = RectF { x: r.x, y: r.y, w: r.w, h: r.h };
        let a = match align {
            Align::Left => 0,
            Align::Center => 1,
            Align::Right => 2,
        };
        let fmt = self.obj.fmt_oneline(a);
        let brush = self.obj.brush(color);
        if fmt.is_null() || brush.is_null() {
            return;
        }
        unsafe {
            GdipDrawString(self.g, u.as_ptr(), u.len() as i32, font, &layout, fmt, brush);
        }
    }

    /// 画多行文字，从矩形顶部开始排。
    pub fn text_wrap(&mut self, s: &str, r: R, size: f32, color: u32) {
        let r = self.snapped_text_rect(r);
        let size = self.snapped_font_size(size);
        if !self.visible(r) || s.is_empty() {
            return;
        }
        let font = self.fonts.get(size, false);
        if font.is_null() {
            return;
        }
        let u: Vec<u16> = s.encode_utf16().collect();
        let layout = RectF { x: r.x, y: r.y, w: r.w, h: r.h };
        let fmt = self.obj.fmt_wrap();
        let brush = self.obj.brush(color);
        if fmt.is_null() || brush.is_null() {
            return;
        }
        unsafe {
            GdipDrawString(self.g, u.as_ptr(), u.len() as i32, font, &layout, fmt, brush);
        }
    }

    /// 量一段文字的尺寸，返回**逻辑单位**。
    ///
    /// 世界变换在生效，`GdipMeasureString` 返回的是设备单位，
    /// 所以这里除以 `scale` 换算回去 —— 页面代码不用知道 DPI。
    ///
    /// **必须用和 `text()` 同一个格式**（`fmt_oneline`：不折行、尾部省略）。
    /// 以前这里传的是 `null`，GDI+ 按默认格式排版 —— 允许折行，于是同样一段
    /// 长文字量出来是多行高度、宽度被 4096 的版面宽截住，而 `text()` 实际画的是
    /// 一行带省略号。调用方拿这个数去判断「放不放得下」，结论自然是错的。
    pub fn measure(&mut self, s: &str, size: f32, bold: bool) -> (f32, f32) {
        let size = self.snapped_font_size(size);
        let font = self.fonts.get(size, bold);
        if font.is_null() || s.is_empty() {
            return (0.0, 0.0);
        }
        let fmt = self.obj.fmt_oneline(0);
        if fmt.is_null() {
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
                fmt,
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
        let pen = self.obj.pen(color, width);
        if pen.is_null() {
            return;
        }
        unsafe {
            GdipDrawEllipse(self.g, pen, r.x, r.y, r.w, r.h);
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
    /// 本帧共享的 GDI+ 上下文（`begin_frame_paint` 建、`end_frame_paint` 删）。
    /// 为空时 `paint` 自己建一次性上下文，画完即删。
    frame_g: *mut c_void,
    /// 上一帧请求的尺寸，用来判断「尺寸还在变」。
    last_need: (i32, i32),
    /// 尺寸连续没变的帧数（见 `STABLE_FRAMES`）。
    stable: u32,
}

#[derive(Debug, PartialEq)]
pub(crate) enum BitmapAction {
    Keep,
    Grow,
    Recreate,
}

/// 已有位图 `have` 对需求 `need` 该怎么处理。
///
/// 比需要的大 25% 以上就换小：32bpp 下 2560x1440 约 14 MB，曾最大化过的窗口
/// 还原后白背着这块内存没有收益。阈值取 25% 是为了避免拖拽缩放时反复重建位图
/// （内核对象分配比复用贵得多）。
pub(crate) fn bitmap_action(have: Option<(i32, i32)>, need: (i32, i32)) -> BitmapAction {
    let (nw, nh) = (need.0.max(1), need.1.max(1));
    let Some((hw, hh)) = have else {
        return BitmapAction::Grow;
    };
    if hw > nw + nw / 4 || hh > nh + nh / 4 {
        return BitmapAction::Recreate;
    }
    if hw >= nw && hh >= nh {
        BitmapAction::Keep
    } else {
        BitmapAction::Grow
    }
}

/// 尺寸连续这么多帧没变，才认为「缩放停了」，允许把位图换小。
///
/// 60fps 下约 0.3 秒；空闲时每帧间隔几百毫秒，也就是一两帧就够。
const STABLE_FRAMES: u32 = 20;

impl Canvas {
    /// 建一个空画布。真正的位图在第一次 `ensure` 时创建。
    pub fn new() -> Canvas {
        Canvas {
            dc: 0,
            bmp: 0,
            old: 0,
            w: 0,
            h: 0,
            frame_g: std::ptr::null_mut(),
            last_need: (0, 0),
            stable: 0,
        }
    }

    /// 保证画布尺寸至少是 `w × h`，返回真正的绘制 DC。
    ///
    /// **位图只会变大不会变小**：缩窗口时不重建，避免拖拽时每帧
    /// `CreateCompatibleBitmap`（那是内核对象分配，很慢）。
    /// 多出来的部分不 `BitBlt`，所以看不到。
    pub fn ensure(&mut self, screen_dc: Gdi, w: i32, h: i32) -> Gdi {
        let w = w.max(1);
        let h = h.max(1);
        // 尺寸还在变（拖边框、最大化/还原动画）就先别缩：`CreateCompatibleBitmap`
        // 是内核对象分配，全屏 2560×1440 那张要 14 MB，塞在动画帧里就是明显卡顿。
        // 连续 `STABLE_FRAMES` 帧尺寸没变，才认为「停下来了」，此时再换小位图。
        if (w, h) == self.last_need {
            self.stable = self.stable.saturating_add(1);
        } else {
            self.last_need = (w, h);
            self.stable = 0;
        }
        let settled = self.stable >= STABLE_FRAMES;
        let action = if self.dc == 0 {
            BitmapAction::Grow
        } else {
            match bitmap_action(Some((self.w, self.h)), (w, h)) {
                BitmapAction::Recreate if !settled => BitmapAction::Keep,
                other => other,
            }
        };
        if action == BitmapAction::Keep {
            return self.dc;
        }
        if action == BitmapAction::Recreate {
            self.release();
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
        self.end_frame_paint();
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

    /// 位图是否已经够大（够大就不必重新要屏幕 DC 当格式模板）。
    ///
    /// `ensure` 在位图够大时直接返回，但调用方为了给它递模板 DC 会先
    /// `GetDC(NULL)` —— 那是每帧两次系统调用，尺寸不变时纯属白花。
    /// 60fps 的充电动效下这是每秒 120 次多余的 GDI 调用。
    pub fn covers(&self, w: i32, h: i32) -> bool {
        self.dc != 0 && self.w >= w.max(1) && self.h >= h.max(1)
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
    /// 开一帧：建一个 `Graphics` 并把渲染状态设好，供本帧后续 [`Canvas::paint`] 复用。
    ///
    /// 一帧要画 5 层（底色 / 页面 / 导航 / 标题栏 / 弹窗）。以前每层各建一次
    /// `Graphics`，等于每帧 5 次 `GdipCreateFromHDC` + 20 次属性设置 + 5 次底色
    /// 亮度计算 —— 这些跟「画哪一层」毫无关系，纯浪费。
    pub fn begin_frame_paint(&mut self, theme: &Theme) {
        if self.dc == 0 || !self.frame_g.is_null() {
            return;
        }
        unsafe {
            let mut g: *mut c_void = std::ptr::null_mut();
            if GdipCreateFromHDC(self.dc, &mut g) != OK || g.is_null() {
                return;
            }
            // 4 = AntiAlias
            GdipSetSmoothingMode(g, 4);
            // 浅底 ClearTypeGridFit(5)，深底 AntiAliasGridFit(3)
            let bg = theme.bg;
            let r = ((bg >> 16) & 0xFF) as u32;
            let gg = ((bg >> 8) & 0xFF) as u32;
            let b = (bg & 0xFF) as u32;
            let lum = (r * 299 + gg * 587 + b * 114) / 1000;
            GdipSetTextRenderingHint(g, if lum < 128 { 3 } else { 5 });
            // 3 = None：不做 0.5px 偏移，避免整片发糊
            GdipSetPixelOffsetMode(g, 3);
            self.frame_g = g;
        }
    }

    /// 收一帧：删掉 [`Canvas::begin_frame_paint`] 建的 `Graphics`。
    pub fn end_frame_paint(&mut self) {
        if self.frame_g.is_null() {
            return;
        }
        unsafe { GdipDeleteGraphics(self.frame_g) };
        self.frame_g = std::ptr::null_mut();
    }

    /// 画一层。`scale` 是 DPI 缩放（96 DPI = 1.0），`scroll` 是纵向滚动（逻辑像素），
    /// `view_w` / `view_h` 是视口尺寸，只用于 [`Painter`] 的可见性剔除。
    ///
    /// 一帧内：第一层之前 [`Canvas::begin_frame_paint`]，最后一层之后
    /// [`Canvas::end_frame_paint`]，中间各层复用同一个 `Graphics`。
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
        let own = self.frame_g.is_null();
        unsafe {
            let mut g = self.frame_g;
            if own {
                if GdipCreateFromHDC(self.dc, &mut g) != OK || g.is_null() {
                    return;
                }
                GdipSetSmoothingMode(g, 4);
                let bg = theme.bg;
                let r = ((bg >> 16) & 0xFF) as u32;
                let gg = ((bg >> 8) & 0xFF) as u32;
                let b = (bg & 0xFF) as u32;
                let lum = (r * 299 + gg * 587 + b * 114) / 1000;
                GdipSetTextRenderingHint(g, if lum < 128 { 3 } else { 5 });
                GdipSetPixelOffsetMode(g, 3);
            }

            // 世界变换 = 缩放 + 滚动。
            // **单位变换时千万别挂**：GDI+ 只要挂着 world transform 就会丢整数
            // GridFit，字会发糊。共享上下文时上一层的变换必须显式复位。
            let needs_xf = (scale - 1.0).abs() > 1e-3 || scroll.abs() > 1e-3;
            if needs_xf {
                let mut m: *mut c_void = std::ptr::null_mut();
                if GdipCreateMatrix2(scale, 0.0, 0.0, scale, 0.0, -scroll * scale, &mut m) == OK
                    && !m.is_null()
                {
                    GdipSetWorldTransform(g, m);
                    GdipDeleteMatrix(m);
                }
            } else if !own {
                GdipResetWorldTransform(g);
            }

            let clip = clip_rect_for(scroll, view_w, view_h);
            if scroll.abs() > 1e-3 {
                let _ = GdipSetClipRect(g, clip.x, clip.y, clip.w, clip.h, 0);
            } else if !own {
                // 共享上下文：上一层装过的裁剪面必须复位，否则这一层被误裁。
                let _ = GdipResetClip(g);
            }
            let mut obj = ObjCache::default();
            {
                let mut p = Painter { g, obj: &mut obj, fonts, t: theme, clip, scale };
                f(&mut p);
            }
            // 先释放画刷/画笔，再删 Graphics —— 反过来它们会失去所属设备上下文
            obj.clear();
            if own {
                GdipDeleteGraphics(g);
            }
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
