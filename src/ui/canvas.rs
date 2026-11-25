//! 承载自绘界面的子窗口。
//!
//! # 为什么要自己建窗口
//!
//! `native-windows-gui` 的主窗口只能放它自己的控件，没有「整页自绘」的通道：
//! 它不转发 `WM_PAINT`，也不给鼠标滚轮/按键的原始事件。所以内容区另建一个
//! `WS_CHILD` 子窗口，自己写 `WNDPROC`，把消息喂给 [`Ui`]。
//!
//! nwg 那边只保留和绘制无关的东西：主窗口外壳、托盘图标、右键菜单、消息循环。
//! **消息循环不用自己写** —— nwg 的 `dispatch_thread_events()` 泵的是整个线程的
//! 消息，我们子窗口的消息自然会进来。
//!
//! # 输入到动作的路径
//!
//! ```text
//! WNDPROC ──► Ui::hit() ──► Vec<Act> ──► damage() ──► 主循环醒来 ──► 取走 Act
//! ```
//!
//! `WNDPROC` 里**不做任何业务**（不读设备、不写盘、不建窗），只登记
//! 「用户点了什么」。原因：`WNDPROC` 运行在消息泵里，阻塞它等于冻住整个界面，
//! 而读设备在鼠标休眠时要等满 1200ms。
//!
//! # 为什么要 `damage` 回调
//!
//! 本模块刻意不 import `nwg`：`ui` 下面的东西只该依赖自己。gui.rs 传一个
//! `move || notice_sender.notice()` 进来，就能把「有东西变了」告诉 nwg 的主循环。

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::ui::gfx::{self, Canvas as GfxCanvas, Fonts, Painter, R};
use crate::ui::layout::Frame;
use crate::ui::pages::{self, Data};
use crate::ui::theme::Theme;
use crate::ui::widget::{Act, Cursor, Page, Slide, Ui};
use crate::win32::{to_wide, Handle, PaintStruct, Rect};

// ───────────────────────────── 常量 ─────────────────────────────

const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CLIPCHILDREN: u32 = 0x0200_0000;
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
const CS_HREDRAW: u32 = 0x0002;
const CS_VREDRAW: u32 = 0x0001;
const CS_DBLCLKS: u32 = 0x0008;

const WM_DESTROY: u32 = 0x0002;
const WM_SIZE: u32 = 0x0005;
const WM_SETFOCUS: u32 = 0x0007;
const WM_KILLFOCUS: u32 = 0x0008;
const WM_PAINT: u32 = 0x000F;
const WM_ERASEBKGND: u32 = 0x0014;
/// 窗口显示状态变化。注意 wparam 是「**即将**显示/隐藏」而不是当前状态。
const WM_SHOWWINDOW: u32 = 0x0018;
const SIZE_MINIMIZED: usize = 1;
const WM_SETCURSOR: u32 = 0x0020;
const WM_GETDLGCODE: u32 = 0x0087;
const WM_KEYDOWN: u32 = 0x0100;
const WM_CHAR: u32 = 0x0102;
const WM_TIMER: u32 = 0x0113;
/// 拖边框缩放开始 / 结束（`WM_ENTERSIZEMOVE` / `WM_EXITSIZEMOVE`）。
const WM_ENTERSIZEMOVE: u32 = 0x0231;
const WM_EXITSIZEMOVE: u32 = 0x0232;

/// 尺寸安定定时器：最大化 / 还原这类「系统自己动」的尺寸变化不走
/// `WM_ENTERSIZEMOVE`，用一次短定时器把动效停到动画结束。
const SETTLE_TIMER_ID: usize = 3;
const SETTLE_MS: u32 = 260;

/// 捕获被系统收回（切窗口、弹窗抢焦点等）。
const WM_CAPTURECHANGED: u32 = 0x0215;
/// 系统要求放弃当前模态状态（拖拽中按 Esc、菜单弹出等）。
const WM_CANCELMODE: u32 = 0x001F;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_NCMOUSELEAVE: u32 = 0x02A2;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_MOUSEWHEEL: u32 = 0x020A;
const WM_LBUTTONDBLCLK: u32 = 0x0203;
const WM_NCHITTEST: u32 = 0x0084;
const WM_NCMOUSEMOVE: u32 = 0x00A0;
const WM_NCLBUTTONDOWN: u32 = 0x00A1;
const WM_NCLBUTTONDBLCLK: u32 = 0x00A3;

const VK_BACK: usize = 0x08;
const VK_TAB: usize = 0x09;
const VK_RETURN: usize = 0x0D;
const VK_ESCAPE: usize = 0x1B;

const IDC_ARROW: usize = 32512;
const IDC_IBEAM: usize = 32513;
const IDC_HAND: usize = 32649;
// 边框缩放光标（系统自带）
const IDC_SIZEWE: usize = 32644;
const IDC_SIZENS: usize = 32645;
const IDC_SIZENWSE: usize = 32642;
const IDC_SIZENESW: usize = 32643;

const GWLP_USERDATA: i32 = -21;
/// `DLGC_WANTCHARS | DLGC_WANTALLKEYS`。
///
/// 少了这个，Tab 和回车会被外层（对话框管理器风格的默认处理）截走，
/// 输入框里就永远打不进字。返回类型是 `isize`，两个标志位或起来是 5。
const DLGC_WANT_ALL: isize = 0x0004 | 0x0001;

/// 子窗口类名。带组件名前缀，避免和 nwg 的 `NativeWindowsGuiWindow` 撞车。
const CLASS_NAME: &str = "RogMouseCtlSurface";

// ───────────────────────────── FFI ─────────────────────────────

#[repr(C)]
struct WndClassExW {
    cb_size: u32,
    style: u32,
    wnd_proc: Option<unsafe extern "system" fn(Handle, u32, usize, isize) -> isize>,
    cls_extra: i32,
    wnd_extra: i32,
    h_instance: Handle,
    h_icon: Handle,
    h_cursor: Handle,
    hbr_background: Handle,
    menu_name: *const u16,
    class_name: *const u16,
    h_icon_sm: Handle,
}

#[repr(C)]
struct TrackMouse {
    size: u32,
    flags: u32,
    hwnd: Handle,
    hover_time: u32,
}

fn track_mouse_leave(hwnd: Handle, non_client: bool) {
    let mut event = TrackMouse {
        size: std::mem::size_of::<TrackMouse>() as u32,
        flags: 0x0002 | if non_client { 0x0010 } else { 0 },
        hwnd,
        hover_time: 0,
    };
    unsafe { TrackMouseEvent(&mut event) };
}

#[link(name = "user32")]
extern "system" {
    fn TrackMouseEvent(event: *mut TrackMouse) -> i32;
    fn RegisterClassExW(wc: *const WndClassExW) -> u16;
    fn CreateWindowExW(
        ex_style: u32,
        class_name: *const u16,
        window_name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: Handle,
        menu: Handle,
        inst: Handle,
        param: *mut core::ffi::c_void,
    ) -> Handle;
    fn DefWindowProcW(hwnd: Handle, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn SendMessageW(hwnd: Handle, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn SetWindowLongPtrW(hwnd: Handle, index: i32, value: isize) -> isize;
    fn GetWindowLongPtrW(hwnd: Handle, index: i32) -> isize;
    fn InvalidateRect(hwnd: Handle, rect: *const Rect, erase: i32) -> i32;
    fn SetFocus(hwnd: Handle) -> Handle;
    fn SetCapture(hwnd: Handle) -> Handle;
    fn ReleaseCapture() -> i32;
    fn SetCursor(cursor: Handle) -> Handle;
    fn LoadCursorW(inst: Handle, name: *const u16) -> Handle;
    fn BeginPaint(hwnd: Handle, ps: *mut PaintStruct) -> Handle;
    fn EndPaint(hwnd: Handle, ps: *const PaintStruct) -> i32;
    fn UpdateWindow(hwnd: Handle) -> i32;
    fn SetTimer(hwnd: Handle, id: usize, elapse_ms: u32, proc: usize) -> usize;
    fn KillTimer(hwnd: Handle, id: usize) -> i32;
    /// 窗口（含所有父窗口）当前是否可见。被 `ShowWindow(SW_HIDE)` 后返 0。
    fn IsWindowVisible(hwnd: Handle) -> i32;
}

// `GetClientRect` / `GetDC` / `ReleaseDC` / `FillRect` 从 [`crate::win32`] 拿，
// 不在这里重复声明 —— 同一个 Win32 函数在两个模块里各声明一次、
// 签名还不一致时 Rust 会报 `clashing_extern_declarations`，
// 而且**只是警告不是错误**，一不小心就调到签名不对的那个。
//
// `GetClientRect` 是个例外：`win32.rs` 里**没有**它（那里只用到
// `BeginPaint` 给的 `rc_paint`），所以只能在这里声明。
#[link(name = "user32")]
extern "system" {
    fn GetClientRect(hwnd: Handle, out: *mut Rect) -> i32;
    fn GetParent(hwnd: Handle) -> Handle;
    pub fn SetWindowPos(
        hwnd: Handle,
        insert_after: Handle,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> i32;
}

/// `SetWindowPos` 的常用标志位。
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;

// 主窗口那边（`frame.rs`）要用的几个只读查询和消息投递。
//
// 放在 canvas 里是因为 `frame.rs` 不该自己声明 extern —— 它要调的都是
// 「窗口」层面的函数，而 canvas 是全工程唯一管窗口的地方。
#[link(name = "user32")]
extern "system" {
    pub fn GetWindowRect(hwnd: Handle, out: *mut Rect) -> i32;
    /// 异步投递消息。用于 `WM_CLOSE` / `WM_SYSCOMMAND` —— **必须异步**：
    /// 在 `WM_NCLBUTTONDOWN` 的处理里同步销毁窗口，会导致后续的消息发到
    /// 已释放的窗口上。
    pub fn PostMessageW(hwnd: Handle, msg: u32, wp: usize, lp: isize) -> i32;
}

/// 读窗口样式/扩展样式的 `index`（`GWL_STYLE = -16`、`GWL_EXSTYLE = -20` …）。
///
/// 收 `&self` 外的裸句柄：`frame.rs` 在主窗口句柄上调用，
/// 那个句柄不归 canvas 管。
pub fn get_window_long(hwnd: Handle, index: i32) -> isize {
    unsafe { GetWindowLongPtrW(hwnd, index) }
}

/// 改窗口样式。`frame::go_frameless` 用它去掉 `WS_CAPTION`。
pub fn get_parent(hwnd: Handle) -> Handle {
    unsafe { GetParent(hwnd) }
}

pub fn set_window_long(hwnd: Handle, index: i32, value: isize) {
    unsafe { SetWindowLongPtrW(hwnd, index, value) };
}

/// 读窗口矩形（屏幕坐标）。
pub fn get_window_rect(hwnd: Handle, out: &mut Rect) {
    unsafe { GetWindowRect(hwnd, out) };
}

/// 读客户区矩形（尺寸单位物理像素，左上角恒为 0,0）。
pub fn get_client_rect(hwnd: Handle, out: &mut Rect) {
    unsafe { GetClientRect(hwnd, out) };
}

/// 投递一条消息，不等它被处理。
pub fn post_message(hwnd: Handle, msg: u32, wp: usize, lp: isize) {
    unsafe { PostMessageW(hwnd, msg, wp, lp) };
}

/// 调 `SetWindowPos`。改完窗口样式后必须用它发一次 `SWP_FRAMECHANGED`，
/// 否则系统还按旧的边框宽度算客户区。
pub fn set_window_pos(
    hwnd: Handle,
    x: i32,
    y: i32,
    cx: i32,
    cy: i32,
    flags: u32,
) {
    unsafe { SetWindowPos(hwnd, std::ptr::null_mut(), x, y, cx, cy, flags) };
}

const ANIM_TIMER_ID: usize = 1;
const ANIM_INTERVAL_MS: u32 = 16;

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Handle;
}

// ───────────────────────────── Canvas ─────────────────────────────

/// 画一帧时与尺寸相关的量。由 canvas 算好传给绘制过程，
/// 免得 gui.rs 再去 `GetClientRect` 一遍（那会多一次系统调用，还可能拿到
/// 和 `BeginPaint` 不一致的值）。
#[derive(Clone, Copy, Debug)]
pub struct PaintArgs {
    /// 物理像素宽高（内存位图的目标尺寸）。
    pub pixel_w: i32,
    pub pixel_h: i32,
    /// 逻辑像素宽高（页面布局的坐标系）。
    pub view_w: f32,
    pub view_h: f32,
    /// 逻辑像素 → 物理像素。
    pub scale: f32,
    /// 内容区子窗口句柄。
    ///
    /// 绘制时要用它查「父窗口是否最大化」（标题栏上的最大化按钮图标要据此
    /// 在「最大化」和「还原」之间切换），临时再 `GetParent` 一次也行，
    /// 但 `WM_PAINT` 里能省一次系统调用就省一次。
    pub hwnd: Handle,
}

/// 一帧的绘制过程，由 gui.rs 提供。
///
/// # 为什么它是「闭包存在 canvas 里」而不是「gui.rs 主动调 `present()`」
///
/// 窗口被别的窗口挡住再露出来、被最小化后恢复、拖动到另一个显示器 ——
/// 这些情况 Windows 只会发一个 `WM_PAINT`，**不会**通知我们的 gui.rs。
/// 如果绘制只在「数据变了」时发生，这些时刻客户区就是一片底色。
///
/// 所以绘制过程必须挂在窗口过程里。`gui.rs` 在启动时把「怎么画」注册进来，
/// 之后每次 `WM_PAINT` 都原样跑一遍。
///
/// 闭包里要做的事：从 `Arc<Mutex<Shared>>` 取一份数据快照，然后调
/// [`draw_frame`]。**不要**在闭包里做 HID 操作或读盘 —— 那会让拖动窗口时
/// 整个界面卡住（`WM_PAINT` 在 GUI 线程上）。
pub type PaintFn = Box<dyn Fn(&mut GfxCanvas, &mut Fonts, &mut Ui, &PaintArgs) -> crate::ui::gfx::Gdi>;

/// 画一整帧：背景、可滚动页面、固定导航、标题栏和弹窗。
///
/// 返回已经画好的**内存位图 DC**，调用方负责 `BitBlt` 到窗口 DC 上。
/// 返回 0 表示位图建不出来（GDI 句柄耗尽），此时什么都不该画。
///
/// **为什么分两趟**：页面内容要跟滚动位移，导航栏不能动。`gfx::Canvas::paint`
/// 一次只接受一个滚动量（它把缩放和滚动合成一个世界变换），所以只能画两趟。
/// 每趟多一次 `GdipCreateFromHDC`，代价在微秒级，不值得为省这一趟去手算坐标。
///
/// 写成自由函数而不是 `Canvas` 的方法：`WM_PAINT` 里那三个字段已经分别被
/// `borrow_mut` 出来了，拿不到 `&self`。
pub fn draw_frame(
    inner: &mut GfxCanvas,
    fonts: &mut Fonts,
    ui: &mut Ui,
    a: &PaintArgs,
    theme: &Theme,
    data: &Data,
) -> crate::ui::gfx::Gdi {
    let mem = inner.ensure(0, a.pixel_w, a.pixel_h);
    if mem == 0 {
        return 0;
    }

    // 视口变了就把滚动量夹回合法范围（在画内容**之前**做，这一帧就是对的）。
    ui.clamp_scroll(a.view_h);

    // **每帧清一次命中区**，而且只在第一趟之前清 —— 清两次会把页面刚登记的
    // 内容区抹掉，表现是「点了有反应但点哪儿都不准」。
    ui.begin_frame();

    let page = ui.page;
    let scroll = ui.scroll_y(page);
    let frame = Frame::new(a.view_w, a.view_h);

    // 最大化状态要问**父窗口**：canvas 是 `WS_CHILD`，它自己没有
    // `WS_MAXIMIZE` 位，查它恒为 false，标题栏上的按钮图标就永远画成「最大化」。
    let is_max = crate::ui::frame::owner_is_maximized(a.hwnd);
    let hover_btn = ui.hover_btn;

    // 整帧只开一个 Graphics：5 层复用同一套渲染状态（省 4 次建上下文 + 16 次属性设置）。
    inner.begin_frame_paint(theme);

    // ① 整窗底色铺底：从 0 开始铺满整个可视区，绝无未绘制黑框
    inner.paint(
        fonts,
        theme,
        a.scale,
        0.0,
        a.view_w,
        a.view_h,
        |p: &mut Painter| {
            p.rect(R::new(0.0, 0.0, a.view_w, a.view_h), theme.bg);
        },
    );

    // ② 页面内容（带滚动，纯世界平移 -scroll）
    inner.paint(
        fonts,
        theme,
        a.scale,
        scroll,
        a.view_w,
        a.view_h,
        |p: &mut Painter| {
            if ui.page == Page::Settings {
                let mut settings_frame = frame;
                settings_frame.content.y = crate::ui::layout::SETTINGS_BODY_TOP;
                settings_frame.content.h = (a.view_h - settings_frame.content.y).max(0.0);
                settings_frame.viewport.y = settings_frame.content.y;
                settings_frame.viewport.h = settings_frame.content.h;
                p.with_clip(R::new(0.0, scroll + settings_frame.content.y, a.view_w,
                    settings_frame.content.h), |p| pages::page(p, ui, data, &settings_frame));
            } else {
                pages::page(p, ui, data, &frame);
            }
        },
    );

    // ④ 标题栏（不滚动，固定在顶层）
    inner.paint(
        fonts,
        theme,
        a.scale,
        0.0,
        a.view_w,
        a.view_h,
        |p: &mut Painter| {
            if ui.page == Page::Settings {
                pages::settings_header(p, ui, data, &frame);
            }
            crate::ui::titlebar::draw(p, a.view_w, theme);
            crate::ui::titlebar::draw_settings(p, ui, a.view_w);
            crate::ui::titlebar::draw_buttons(p, a.view_w, theme, is_max, hover_btn);
        },
    );

    // ⑤ 模态弹窗（若有）
    if ui.show_close_dialog {
        inner.paint(
            fonts,
            theme,
            a.scale,
            0.0,
            a.view_w,
            a.view_h,
            |p: &mut Painter| {
                pages::draw_close_dialog(p, ui, a.view_w, a.view_h);
            },
        );
    }
    inner.end_frame_paint();
    mem
}

///
/// 所以状态不能 `Box::into_raw` 塞进 `GWLP_USERDATA`：那个地址的生命周期
/// 跟 `run()` 里的消息循环对不上，窗口一销毁就是野指针。改成两边各自克隆
/// `Rc`，进程内只留一份 `Ctx`。
struct Ctx {
    ui: Rc<RefCell<Ui>>,
    acts: Rc<RefCell<Vec<Act>>>,
    damage: Rc<dyn Fn()>,
    /// 逻辑像素 → 物理像素的缩放（125% 屏 = 1.25）。
    scale: f32,
    /// 上一次设的光标 **id**（`IDC_*`），避免每次 `WM_SETCURSOR` 都 `LoadCursorW` 一遍。
    /// 存 id 而不是 `Cursor` 枚举：边框缩放码不在 `Cursor` 里，用 id 才能统一缓存。
    cursor: Cell<usize>,
    /// 双缓冲的内存画布。和 [`Canvas`] 共享同一个对象。
    inner: Rc<RefCell<GfxCanvas>>,
    /// 字体缓存（`GpFont*` 每帧重建会让首帧多花 20ms）。
    fonts: Rc<RefCell<Fonts>>,
    /// 注册进来的绘制过程。`None` = gui.rs 还没注册，此时只铺底色。
    paint: Rc<RefCell<Option<PaintFn>>>,
    animation: Rc<Animation>,
}

/// 自绘内容区。
#[derive(Clone)]
pub struct Canvas {
    hwnd: Handle,
    inner: Rc<RefCell<GfxCanvas>>,
    fonts: Rc<RefCell<Fonts>>,
    /// 界面状态（当前页、滚动位置、悬停项 …）。
    ///
    /// **绘制时会被改写** —— 命中区是每帧重新登记的，所以页面代码必须先
    /// `ui.hot()` 再画那个控件，反过来就永远点不中（点击用的是上一帧的命中区）。
    ui: Rc<RefCell<Ui>>,
    /// 逻辑像素 → 物理像素的缩放。和 `Ctx.scale` 同值，两边各存一份只为免去解引用。
    scale: f32,
    /// 本轮攒下的动作，主循环 `take_acts` 取走后执行。
    acts: Rc<RefCell<Vec<Act>>>,
    /// 「有东西变了，快重画」的回调（gui.rs 里实现成 `notice_sender.notice()`）。
    damage: Rc<dyn Fn()>,
    /// 绘制过程。与 `Ctx.paint` 指向同一个 `RefCell`。
    paint: Rc<RefCell<Option<PaintFn>>>,
    animation: Rc<Animation>,
    /// 父窗口句柄：第一次用时取一次就记住（GetParent 每次拖动都会进 WM_SIZE）。
    parent: Cell<Handle>,
    /// 上一次铺满时的客户区像素尺寸。WM_SIZE 与「最大化」会为同一次尺寸变化
    /// 各发一条消息，靠它把第二次（以及拖拽时尺寸没变的帧）挡在系统调用之前。
    last_fit: Cell<Option<(i32, i32)>>,
}

impl Canvas {
    /// 在 `parent` 里建一块内容区子窗口。
    ///
    /// `damage` 会在「用户操作改变了界面状态」时被调用，用来唤醒主循环重画。
    pub fn create(
        parent: Handle,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        scale: f32,
        damage: Rc<dyn Fn()>,
    ) -> Result<Canvas, String> {
        if !gfx::start() {
            return Err("GdiplusStartup 失败".to_string());
        }
        let fonts = Fonts::from_candidates(&[
            "Microsoft YaHei UI",
            "Microsoft YaHei",
            "SimSun",
            "Segoe UI",
        ])
        .ok_or_else(|| "一个可用字体都建不出来".to_string())?;

        // SAFETY: 传空指针 = 取当前 exe 的模块句柄。
        let inst = unsafe { GetModuleHandleW(std::ptr::null()) };
        if inst.is_null() {
            return Err("GetModuleHandleW 失败".to_string());
        }

        let class = to_wide(CLASS_NAME);
        let wc = WndClassExW {
            cb_size: std::mem::size_of::<WndClassExW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            wnd_proc: Some(wnd_proc),
            cls_extra: 0,
            wnd_extra: 0,
            h_instance: inst,
            h_icon: std::ptr::null_mut(),
            h_cursor: unsafe { LoadCursorW(std::ptr::null_mut(), idc(IDC_ARROW)) },
            // 背景自己填（`WM_ERASEBKGND` 返回 1），让系统刷只会闪
            hbr_background: std::ptr::null_mut(),
            menu_name: std::ptr::null(),
            class_name: class.as_ptr(),
            h_icon_sm: std::ptr::null_mut(),
        };
        // 重复注册返回 0 并置 `ERROR_CLASS_ALREADY_EXISTS`(1410) —— 不是失败。
        unsafe { RegisterClassExW(&wc) };

        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                x,
                y,
                w,
                h,
                parent,
                std::ptr::null_mut(),
                inst,
                std::ptr::null_mut(),
            )
        };
        if hwnd.is_null() {
            let err = unsafe { crate::win32::GetLastError() };
            return Err(format!("CreateWindowExW 失败（GetLastError={err}）"));
        }

        let me = Canvas {
            hwnd,
            inner: Rc::new(RefCell::new(GfxCanvas::new())),
            fonts: Rc::new(RefCell::new(fonts)),
            ui: Rc::new(RefCell::new(Ui::default())),
            scale: if scale > 0.5 { scale } else { 1.0 },
            acts: Rc::new(RefCell::new(Vec::new())),
            damage,
            paint: Rc::new(RefCell::new(None)),
            animation: Rc::new(Animation::default()),
            parent: Cell::new(std::ptr::null_mut()),
            last_fit: Cell::new(None),
        };

        // 窗口过程那份状态。两边是**独立**的 `Rc` 克隆，不是指向 `Canvas` 的指针
        // —— 见 [`Ctx`] 的文档：这条路上任何「把 Canvas 借给窗口过程」的做法
        // 都会在 `Canvas` 被移动/释放后变成悬垂指针。
        let ctx = Box::new(Ctx {
            ui: Rc::clone(&me.ui),
            acts: Rc::clone(&me.acts),
            damage: Rc::clone(&me.damage),
            inner: Rc::clone(&me.inner),
            fonts: Rc::clone(&me.fonts),
            paint: Rc::clone(&me.paint),
            scale: me.scale,
            cursor: Cell::new(IDC_ARROW),
            animation: Rc::clone(&me.animation),
        });
        let raw = Box::into_raw(ctx);
        // SAFETY: 指针来自 `Box::into_raw`，在 `WM_DESTROY` 里用 `Box::from_raw` 收回。
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize) };
        Ok(me)
    }

    /// 读界面状态（gui.rs 处理 `Act` 时要看当前页、下拉框、输入框草稿）。
    pub fn with_ui<R>(&self, f: impl FnOnce(&Ui) -> R) -> R {
        f(&self.ui.borrow())
    }

    /// 改界面状态。改完自己负责 [`Canvas::repaint_now`]。
    ///
    /// 收 `&self` 而不是 `&mut self`：状态本来就在 `RefCell` 里，
    /// 而 gui.rs 把 canvas 放在 `Rc` 里多处分发（事件处理器要 `Fn`，不能独占）。
    pub fn with_ui_mut<R>(&self, f: impl FnOnce(&mut Ui) -> R) -> R {
        f(&mut self.ui.borrow_mut())
    }

    /// 取走这一轮攒下的动作。主循环每帧调一次。
    pub fn take_acts(&self) -> Vec<Act> {
        std::mem::take(&mut *self.acts.borrow_mut())
    }

    /// 把整窗标脏（主窗口 `WM_SIZE` 时调）。
    pub fn invalidate(&self) {
        // SAFETY: 传空矩形 = 整个客户区。
        unsafe { InvalidateRect(self.hwnd, std::ptr::null(), 0) };
    }

    pub fn set_animating(&self, on: bool) {
        self.animation.wanted.set(on);
        self.animation.sync(self.hwnd);
    }

    pub fn set_resizing(&self, resizing: bool) {
        self.animation.set_resizing(self.hwnd, resizing);
    }

    /// 把子窗口铺满父窗口客户区。主窗口 `WM_SIZE` 时调。
    ///
    /// 用 `SetWindowPos` 而不是 `MoveWindow`：后者会顺带 `InvalidateRect`，
    /// 而我们紧接着自己要重画一帧，多刷一次只会闪。
    pub fn refit(&self) {
        let mut parent = self.parent.get();
        if parent.is_null() {
            parent = unsafe { GetParent(self.hwnd) };
            if parent.is_null() {
                return;
            }
            self.parent.set(parent);
        }
        let size = client_pixel_size(parent);
        if !refit_needed(self.last_fit.get(), size) {
            return;
        }
        self.last_fit.set(Some(size));
        unsafe {
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                0,
                0,
                size.0,
                size.1,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            // 尺寸变了必须自己标脏：SetWindowPos 不替我们重画。
            InvalidateRect(self.hwnd, std::ptr::null(), 0);
        }
    }

    /// 注册「怎么画一帧」。
    ///
    /// 只调一次（建完 canvas 立刻调）。之后每次 `WM_PAINT` —— 包括系统因遮挡、
    /// 最小化恢复、跨显示器拖动而发来的那些 —— 都会跑这个闭包。
    ///
    /// 闭包要返回已经画好的内存位图 DC（调 [`draw_frame`] 的返回值），
    /// canvas 负责 `BitBlt` 到窗口上。
    pub fn set_painter(&self, f: PaintFn) {
        *self.paint.borrow_mut() = Some(f);
    }

    /// 主动重画一帧（不必等 `WM_PAINT`）。
    ///
    /// 用 `UpdateWindow` 而不是 `InvalidateRect`：数据变了要**立刻**看到，
    /// 而 `InvalidateRect` 只是标脏，真正的 `WM_PAINT` 要等消息队列排到，
    /// 拖动滑块时会明显滞后。
    pub fn repaint_now(&self) {
        repaint_hwnd(self.hwnd);
    }

    /// 窗口不可见（最小化 / 收进托盘）时把渲染内存还给系统。
    ///
    /// 释放的是那张与窗口同大的 GDI+ 位图（1100×740×4 ≈ 3.1 MiB，最大化到
    /// 2K 时约 14 MiB）和按字号缓存的字体。恢复显示后的第一次 `WM_PAINT`
    /// 会走 `ensure()` 重建位图、`Fonts` 重建字体，用户看不出区别。
    ///
    /// 只在**确实不可见**时调：窗口还露着就释放，下一帧立刻又分配回来，
    /// 白折腾一遍还要多一次 3 MB 的分配。
    pub fn release_render_memory(&self) {
        self.animation.suspend(self.hwnd);
        self.inner.borrow_mut().release();
        self.fonts.borrow_mut().clear();
        // 删掉 GDI+ 位图只是归还给私有堆，那些页仍然算在进程头上；窗口都不可见了，
        // 顺手把工作集清空，任务管理器里才看得见下降（恢复后按需重新换入）。
        unsafe {
            crate::win32::SetProcessWorkingSetSize(
                crate::win32::GetCurrentProcess(),
                usize::MAX,
                usize::MAX,
            );
        }
        // 图表几何按位图尺寸换算，一并丢掉，恢复后按新尺寸重算。
        self.ui.borrow_mut().chart = None;
    }

    /// 拿到子窗口句柄。`gui.rs` 需要在 `Canvas` 被 move 进闭包之前
    /// 留一份句柄，供「显示窗口前先画首帧」用。
    pub fn hwnd(&self) -> Handle {
        self.hwnd
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        // 子窗口不在这里 `DestroyWindow` —— 它随父窗口销毁。
        // 但位图和 DC 是 GDI 对象，进程退出前不释放会让 GDI 句柄数持续上涨
        //（默认上限 10000，反复开关窗口能撞到）。
        self.inner.borrow_mut().release();
    }
}

/// 立刻把整块客户区重画一遍（同步，不等消息队列）。
///
/// 必须先 `InvalidateRect` 再 `UpdateWindow`：`UpdateWindow` 只在**更新区
/// 非空**时才真的发一次 `WM_PAINT`，光调它、没有失效区，什么都不会画。
/// 开机首帧之所以一直是黑框，就是这里只调了 `UpdateWindow`（见 git 历史）。
pub fn repaint_hwnd(hwnd: Handle) {
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 0);
        UpdateWindow(hwnd);
    }
}

/// `IDC_*` 是 `MAKEINTRESOURCE`，也就是把整数直接当指针用。
fn idc(n: usize) -> *const u16 {
    n as *const u16
}

fn cursor_id(c: Cursor) -> usize {
    match c {
        Cursor::Arrow => IDC_ARROW,
        Cursor::Hand => IDC_HAND,
        Cursor::IBeam => IDC_IBEAM,
    }
}

fn resize_cursor_id(hit: isize) -> Option<usize> {
    use crate::ui::frame::*;
    match hit {
        HTLEFT | HTRIGHT => Some(IDC_SIZEWE),
        HTTOP | HTBOTTOM => Some(IDC_SIZENS),
        HTTOPLEFT | HTBOTTOMRIGHT => Some(IDC_SIZENWSE),
        HTTOPRIGHT | HTBOTTOMLEFT => Some(IDC_SIZENESW),
        _ => None,
    }
}

// ───────────────────────────── WNDPROC ─────────────────────────────

/// 从窗口上取回 `*mut Ctx`。
///
/// SAFETY: 只在 `WM_DESTROY` 之前调用（`WM_DESTROY` 里会把它清零），
/// 指针由 `Canvas::create` 写入。
unsafe fn ctx_of(hwnd: Handle) -> Option<&'static mut Ctx> {
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ctx;
    raw.as_mut()
}

/// 鼠标坐标解码。`lparam` 低 16 位是 x、高 16 位是 y，**都是有符号的**。
///
/// 用 `as u16 as i16 as i32` 而不是 `& 0xFFFF`：拖动时鼠标可能跑到客户区
/// 左边/上边之外（被 `SetCapture` 抓住仍在给我们发消息），那时高位全是 1，
/// 按无符号解会变成 65531 这种数，滑块就瞬移到最右。
fn lo_word(v: isize) -> i32 {
    (v as u16) as i16 as i32
}
fn hi_word(v: isize) -> i32 {
    ((v >> 16) as u16) as i16 as i32
}

/// 客户区尺寸（物理像素）。
///
/// 单独抽出来是因为 `WM_SIZE` 的 `lparam` 和 `GetClientRect` 在窗口最小化时
/// 结果不同（前者是 0×0，后者也给 0×0），而 `WM_PAINT` 里只能靠后者。
/// 尺寸没变就不用再发 SetWindowPos + InvalidateRect。
fn refit_needed(last: Option<(i32, i32)>, now: (i32, i32)) -> bool {
    last != Some(now)
}

fn client_pixel_size(hwnd: Handle) -> (i32, i32) {
    let mut r = Rect::new(0, 0, 0, 0);
    unsafe { GetClientRect(hwnd, &mut r) };
    (r.width().max(1), r.height().max(1))
}

/// 铺一遍底色。只在 painter 还没注册（启动最初几毫秒）时用到。
fn fill_background(target: Handle, hwnd: Handle) {
    let mut r = Rect::new(0, 0, 0, 0);
    unsafe { GetClientRect(hwnd, &mut r) };
    let brush = unsafe { crate::win32::CreateSolidBrush(0x00F3_F3F3) };
    if !brush.is_null() {
        unsafe {
            crate::win32::FillRect(target, &r, brush);
            crate::win32::DeleteObject(brush);
        }
    }
}

#[derive(Default)]
struct Animation {
    wanted: Cell<bool>,
    running: Cell<bool>,
    resizing: Cell<bool>,
    settle_until: Cell<Option<Instant>>,
}

impl Animation {
    fn visible(hwnd: Handle) -> bool {
        unsafe {
            IsWindowVisible(hwnd) != 0 && crate::win32::IsIconic(GetParent(hwnd)) == 0
        }
    }

    fn sync(&self, hwnd: Handle) {
        let on = self.wanted.get() && !self.resizing.get() && self.settle_until.get().is_none()
            && Self::visible(hwnd);
        if on && !self.running.get() {
            self.running.set(unsafe { SetTimer(hwnd, ANIM_TIMER_ID, ANIM_INTERVAL_MS, 0) } != 0);
        } else if !on {
            self.stop(hwnd);
        }
    }

    fn stop(&self, hwnd: Handle) {
        if self.running.replace(false) {
            unsafe { KillTimer(hwnd, ANIM_TIMER_ID) };
        }
    }

    fn cancel_settle(&self, hwnd: Handle) {
        if self.settle_until.take().is_some() {
            unsafe { KillTimer(hwnd, SETTLE_TIMER_ID) };
        }
    }

    fn suspend(&self, hwnd: Handle) {
        self.stop(hwnd);
        self.cancel_settle(hwnd);
        self.resizing.set(false);
    }

    fn size_changed(&self, hwnd: Handle) {
        self.stop(hwnd);
        self.cancel_settle(hwnd);
        if Self::visible(hwnd) {
            let deadline = Instant::now() + Duration::from_millis(u64::from(SETTLE_MS));
            if unsafe { SetTimer(hwnd, SETTLE_TIMER_ID, SETTLE_MS, 0) } != 0 {
                self.settle_until.set(Some(deadline));
            }
            self.sync(hwnd);
        }
    }

    fn set_resizing(&self, hwnd: Handle, resizing: bool) {
        self.resizing.set(resizing);
        if !resizing {
            self.cancel_settle(hwnd);
            unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
        }
        self.sync(hwnd);
    }
}

unsafe extern "system" fn wnd_proc(hwnd: Handle, msg: u32, wp: usize, lp: isize) -> isize {
    match msg {
        // 自己填背景。返回 1 = 已处理，系统不再刷一遍。
        WM_ERASEBKGND => 1,

        // 真正的绘制在这里发生。
        //
        // 为什么不能只由 gui.rs 在数据变化时调一次：窗口被别的窗口挡住再露出来、
        // 最小化后恢复、拖到另一个显示器 —— 这些情况 Windows **只发 WM_PAINT**，
        // 不通知我们的数据层。绘制挂在窗口过程里，这些时刻才有正确的像素。
        WM_PAINT => {
            let mut ps = PaintStruct::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            // `BeginPaint` 通常把同一个 DC 既返回又写进 `ps.hdc`，但两者
            // 都可能为空（窗口最小化时），所以分别判断。
            let target = if ps.hdc.is_null() { hdc } else { ps.hdc };
            if !target.is_null() {
                if let Some(c) = ctx_of(hwnd) {
                    // `paint` 是 `Fn`，但它内部要 `borrow_mut` 三个 `RefCell`
                    // —— 这里不能持有 `paint` 的借用再来借 `ui`/`fonts`/`inner`，
                    // 所以先 `take` 出来。不存在重入：绘制期间不会再来 WM_PAINT。
                    let f = c.paint.borrow_mut().take();
                    match f {
                        Some(f) => {
                            let (pw, ph) = client_pixel_size(hwnd);
                            let args = PaintArgs {
                                pixel_w: pw,
                                pixel_h: ph,
                                view_w: pw as f32 / c.scale,
                                view_h: ph as f32 / c.scale,
                                scale: c.scale,
                                hwnd,
                            };
                            let mem = f(
                                &mut c.inner.borrow_mut(),
                                &mut c.fonts.borrow_mut(),
                                &mut c.ui.borrow_mut(),
                                &args,
                            );
                            // 位图建不出来就什么都别画 —— `BitBlt` 传 0 会失败，
                            // 但失败后客户区留着上一帧的像素，比花屏更难查。
                            if mem != 0 {
                                gfx::blit(target as gfx::Gdi, mem, pw, ph);
                            }
                            let wanted = {
                                let ui = c.ui.borrow();
                                mem != 0 && (ui.anim_hint || !ui.status.is_empty())
                            };
                            c.animation.wanted.set(wanted);
                            *c.paint.borrow_mut() = Some(f);
                        }
                        None => {
                            // gui.rs 还没注册绘制过程（启动最初几毫秒）。
                            // 铺一遍底色，别让客户区是未初始化的白块。
                            fill_background(target, hwnd);
                        }
                    }
                } else {
                    fill_background(target, hwnd);
                }
            }
            EndPaint(hwnd, &ps);
            if let Some(c) = ctx_of(hwnd) {
                c.animation.sync(hwnd);
            }
            0
        }

        WM_SIZE => {
            if let Some(c) = ctx_of(hwnd) {
                if wp == SIZE_MINIMIZED {
                    // 最小化后没有可见像素：位图（最大一块）和字体缓存先还回去，
                    // 恢复时 ensure() 会按当时的尺寸重建。
                    c.animation.suspend(hwnd);
                    release_render_memory(&c);
                } else {
                    c.animation.size_changed(hwnd);
                    // SAFETY: 传空矩形 = 整个客户区。
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                    (c.damage)();
                }
            }
            0
        }

        // 捕获被系统收回（切窗口 / 弹窗抢焦点 / 拖到窗口外松手）时，
        // 必须把「按下」状态一起清掉，否则滑块会一直跟着鼠标跑。
        WM_CAPTURECHANGED | WM_CANCELMODE => {
            if let Some(c) = ctx_of(hwnd) {
                let had = {
                    let mut ui = c.ui.borrow_mut();
                    // 只有「按下状态还在」才算真的丢了拖拽；WM_LBUTTONUP 已经
            // 把 pointer.down 置 false 后再释放捕获，这里就不该再清一次。
            let had = ui.pointer.down;
                    ui.cancel_press();
                    had
                };
                if had {
                    (c.damage)();
                }
            }
            0
        }

        WM_ENTERSIZEMOVE | WM_EXITSIZEMOVE => {
            if let Some(c) = ctx_of(hwnd) {
                c.animation.set_resizing(hwnd, msg == WM_ENTERSIZEMOVE);
            }
            0
        }

        WM_TIMER => {
            if let Some(c) = ctx_of(hwnd) {
                if wp == SETTLE_TIMER_ID && c.animation.settle_until.get()
                    .is_some_and(|deadline| Instant::now() >= deadline) {
                    c.animation.cancel_settle(hwnd);
                    c.animation.sync(hwnd);
                    if Animation::visible(hwnd) {
                        InvalidateRect(hwnd, std::ptr::null(), 0);
                    }
                } else if wp == ANIM_TIMER_ID {
                    c.animation.sync(hwnd);
                    if c.animation.running.get() {
                        InvalidateRect(hwnd, std::ptr::null(), 0);
                    }
                }
            }
            0
        }

        WM_SHOWWINDOW => {
            let showing = wp != 0;
            if let Some(c) = ctx_of(hwnd) {
                if showing {
                    // 隐藏期间位图已释放：立刻补一帧，别让用户看到第一帧空白。
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                    (c.damage)();
                } else {
                    c.animation.suspend(hwnd);
                    release_render_memory(&c);
                }
            }
            0
        }

        WM_SETFOCUS => {
            if let Some(c) = ctx_of(hwnd) {
                (c.damage)();
            }
            0
        }

        WM_KILLFOCUS => {
            if let Some(c) = ctx_of(hwnd) {
                // 焦点走了就把输入框当失焦提交，否则用户点了别处草稿还挂着，
                // 界面上显示的文字和设备里的值不是一回事。
                if c.ui.borrow_mut().commit_edit_on_blur() {
                    (c.damage)();
                }
            }
            0
        }

        // 告诉对话框管理器「所有按键都给我」，否则 Tab/回车被外层截走
        WM_GETDLGCODE => DLGC_WANT_ALL,

        WM_MOUSEMOVE => {
            if let Some(c) = ctx_of(hwnd) {
                track_mouse_leave(hwnd, false);
                let x = lo_word(lp) as f32 / c.scale;
                let y = hi_word(lp) as f32 / c.scale;
                let mut ui = c.ui.borrow_mut();
                let moved = (ui.pointer.x - x).abs() > 0.01 || (ui.pointer.y - y).abs() > 0.01;
                ui.pointer.x = x;
                ui.pointer.y = y;
                if moved {
                    let (pw, _) = client_pixel_size(hwnd);
                    if ui.device_drag.is_some() {
                        let scroll = ui.scroll_y(ui.page);
                        let changed = ui.update_device_drag(x, y, scroll);
                        drop(ui);
                        if changed {
                            unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
                            (c.damage)();
                        }
                        return 0;
                    }
                    if ui.pointer.down
                        && matches!(ui.pressed, Some(Act::SelectDevice(_)))
                        && ((x - ui.pointer.press_x).powi(2)
                            + (y - ui.pointer.press_y).powi(2))
                        .sqrt()
                            > 4.0
                    {
                        let scroll = ui.scroll_y(ui.page);
                        let started = ui.activate_device_drag(x, y, scroll);
                        drop(ui);
                        if started {
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                            (c.damage)();
                        }
                        return 0;
                    }
                    let hover_changed = ui.refresh_hover_at(x, y, pw as f32 / c.scale);
                    // 正在拖滑块时，先把「这一帧该显示的值」写进 `live`，
                    // 页面下一帧就按它画 —— 这是滑块跟手的**唯一**来源。
                    //
                    // 不能走 `Act::SetSlide` 让 gui.rs 去写设备：写一轮设备
                    // 几十毫秒（还要读回断言），拖一次会积压几十条命令，
                    // 表现是滑块比鼠标慢半拍、松手后还在自己动。
                    if let Some(s) = ui.pointer.dragging {
                        if let Some(frac) = ui.fraction_at(s, x) {
                            ui.set_live_frac(s, frac);
                        }
                        drop(ui);
                        unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
                        (c.damage)();
                    } else {
                        // 悬停态变了才重画（悬停高亮），否则每次移动都重绘整页
                        if hover_changed {
                            drop(ui);
                            unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
                            (c.damage)();
                        }
                    }
                }
            }
            0
        }

        WM_MOUSELEAVE | WM_NCMOUSELEAVE => {
            if let Some(c) = ctx_of(hwnd) {
                let mut ui = c.ui.borrow_mut();
                // 只清除离开的区域，避免迟到的 leave 擦掉相邻按钮的新高亮。
                let changed = if msg == WM_MOUSELEAVE {
                    ui.hover.take().is_some()
                } else {
                    ui.hover_btn.take().is_some()
                };
                drop(ui);
                if changed {
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                    (c.damage)();
                }
            }
            0
        }

        WM_LBUTTONDOWN => {
            if let Some(c) = ctx_of(hwnd) {
                SetFocus(hwnd);
                let x = lo_word(lp) as f32 / c.scale;
                let y = hi_word(lp) as f32 / c.scale;
                // 标题栏三按钮的**兜底命中**。
                //
                // 正常路径是 WM_NCHITTEST 返回 HTCLOSE/HTMINBUTTON/HTMAXBUTTON，
                // 再由 WM_NCLBUTTONDOWN 交给系统。但子窗口的 NC 命中码并非所有
                // 情况下都被系统采纳（窗口未激活时的首次点击、命中码在按下与
                // 抬起之间被重算、以及某些远程/触控输入下），表现就是用户说的
                // 「有时候能点有时候点不动」。这里再判一次，保证只要点在按钮
                // 矩形里就一定能触发，双保险（重复触发会被 WM_CLOSE 幂等消化）。
                {
                    let (pw, _) = client_pixel_size(hwnd);
                    let cw = pw as f32 / c.scale;
                    if y >= 0.0 && y <= crate::ui::titlebar::TITLE_H {
                        if let Some(b) = crate::ui::titlebar::hit(x, y, cw) {
                            let target = unsafe { GetParent(hwnd) };
                            let target = if target.is_null() { hwnd } else { target };
                            let code = match b {
                                crate::ui::titlebar::Btn::Minimize => crate::ui::frame::HTMINBUTTON,
                                crate::ui::titlebar::Btn::Maximize => crate::ui::frame::HTMAXBUTTON,
                                crate::ui::titlebar::Btn::Close => crate::ui::frame::HTCLOSE,
                            };
                            crate::ui::frame::on_sys_button(target, code);
                            return 0;
                        }
                    }
                }
                let scroll;
                let slide;
                {
                    let mut ui = c.ui.borrow_mut();
                    ui.pointer.x = x;
                    ui.pointer.y = y;
                    ui.pointer.press_x = x;
                    ui.pointer.press_y = y;
                    ui.pointer.down = true;
                    scroll = ui.scroll_y(ui.page);
                    ui.on_press(x, y, scroll);
                    // 按在滑块上就立刻进入拖拽态：这样按住不放一路拖，
                    // 值会一直跟手（不是松手才跳一下）。
                    ui.pointer.dragging = ui.pressed_slide;
                    slide = ui.pointer.dragging;
                }
                SetCapture(hwnd);
                if let Some(s) = slide {
                    let frac = c.ui.borrow().fraction_at(s, x);
                    if let Some(frac) = frac {
                        c.acts.borrow_mut().push(Act::SetSlide(s, frac));
                    }
                }
                (c.damage)();
            }
            0
        }

        WM_LBUTTONUP => {
            if let Some(c) = ctx_of(hwnd) {
                // **不要**在这里 ReleaseCapture：它会同步派发 WM_CAPTURECHANGED，
                // 而那个分支会 cancel_press() 把 `pressed` 清掉，紧接着的
                // on_release 就什么都命中不到 —— 点击全部失效（悬停/按下有动效，
                // 但页面不切换、下拉不弹、输入框不进焦点、开关不响应）。
                // 正确的顺序：先算出 pushed，再释放捕获。
                let x = lo_word(lp) as f32 / c.scale;
                let y = hi_word(lp) as f32 / c.scale;
                let mut pushed: Option<Act> = None;
                let dragging: Option<Slide>;
                let device_reorder: Option<Act>;
                {
                    let mut ui = c.ui.borrow_mut();
                    ui.pointer.x = x;
                    ui.pointer.y = y;
                    ui.pointer.down = false;
                    if ui.device_drag.is_some() {
                        let scroll = ui.scroll_y(ui.page);
                        ui.update_device_drag(x, y, scroll);
                    }
                    device_reorder = ui.finish_device_drag();
                    dragging = ui.pointer.dragging.take();
                    if dragging.is_none() && device_reorder.is_none() {
                        let scroll = ui.scroll_y(ui.page);
                        pushed = ui.on_release(x, y, scroll);
                    }
                    ui.pressed_slide = None;
                }
                if let Some(action) = device_reorder {
                    c.acts.borrow_mut().push(action);
                } else if let Some(s) = dragging {
                    // 拖动结束：把最终值发给页面，并通知「这一拖结束了」
                    //（页面据此决定要不要落盘 —— 拖的过程中不能每次都写设备）
                    let frac = c.ui.borrow().fraction_at(s, x);
                    let mut acts = c.acts.borrow_mut();
                    if let Some(frac) = frac {
                        acts.push(Act::SetSlide(s, frac));
                    }
                    acts.push(Act::CommitSlide(s));
                } else if let Some(a) = pushed {
                    c.acts.borrow_mut().push(a);
                }
                ReleaseCapture();
                (c.damage)();
            }
            0
        }

        WM_MOUSEWHEEL => {
            if let Some(c) = ctx_of(hwnd) {
                // `wparam` 高 16 位是有符号的滚动量，一格 = 120
                let delta = ((wp >> 16) as u16) as i16 as f32 / 120.0;
                let mut r = Rect::new(0, 0, 0, 0);
                GetClientRect(hwnd, &mut r);
                let vh = r.height() as f32 / c.scale;
                let changed = c.ui.borrow_mut().wheel(delta, vh);
                if changed {
                    unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
                    (c.damage)();
                }
            }
            0
        }

        // 子窗口吃下客户区全部鼠标消息：主窗口那边只负责非客户区，命中判定
        // 只能在这里做，否则点击会被吞进消息循环、按钮全哑。
        WM_NCHITTEST | WM_NCMOUSEMOVE => {
            let c = match ctx_of(hwnd) {
                Some(c) => c,
                None => return crate::ui::frame::HTCLIENT,
            };
            // lparam 是**屏幕坐标**的 16 位有符号对，要换成客户区逻辑坐标
            let sx = lo_word(lp) as f32;
            let sy = hi_word(lp) as f32;
            let mut wr = Rect::default();
            GetWindowRect(hwnd, &mut wr);
            let x = (sx - wr.left as f32) / c.scale;
            let y = (sy - wr.top as f32) / c.scale;
            let (pw, ph) = client_pixel_size(hwnd);
            let cw = pw as f32 / c.scale;
            let ch = ph as f32 / c.scale;
            let hit = crate::ui::frame::logical_hit(
                x,
                y,
                cw,
                ch,
                crate::ui::titlebar::TITLE_H,
                crate::ui::frame::owner_is_maximized(hwnd),
            );
            let mut ui = c.ui.borrow_mut();
            let changed = ui.refresh_hover_at(x, y, cw);
            drop(ui);
            if changed {
                InvalidateRect(hwnd, std::ptr::null(), 0);
                (c.damage)();
            }
            if msg == WM_NCMOUSEMOVE {
                track_mouse_leave(hwnd, true);
                DefWindowProcW(hwnd, msg, wp, lp)
            } else {
                hit
            }
        }

        // 拖动 / 边缘缩放：把意图交回系统。
        //
        // `DefWindowProc(WM_NCLBUTTONDOWN)` 收到 `HTCAPTION` / `HTLEFT` … 之后
        // 会自己接管鼠标、自己 `SetWindowPos` —— Aero Snap、双击最大化、
        // 高 DPI 跟手全在里面。自己实现会把这些统统丢掉。
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => {
            // 注意：这里的 `wp` 是**命中码**（不是坐标），`lp` 才是坐标。
            if crate::ui::frame::is_sys_button(wp as isize) {
                // 按钮：发系统命令。要问**父窗口**的最大化状态和各段句柄。
                let target = unsafe { GetParent(hwnd) };
                let target = if target.is_null() { hwnd } else { target };
                crate::ui::frame::on_sys_button(target, wp as isize);
                return 0;
            }
            if msg == WM_NCLBUTTONDBLCLK && wp as isize == crate::ui::frame::HTCAPTION {
                let target = unsafe { GetParent(hwnd) };
                let target = if target.is_null() { hwnd } else { target };
                crate::ui::frame::toggle_maximize(target);
                return 0;
            }
            // 拖动/缩放：必须交给**顶层窗口**的 `DefWindowProc`，
            // 子窗口自己做不了非客户区拖动 —— 拖标题栏时窗口纹丝不动。
            //
            // 只有拖动/缩放码才转交；其他码不转，否则 `DefWindowProc` 会按
            // 「非客户区点击」再走一遍默认处理（比如把 `HTCLIENT` 当成无效值
            // 去 `WM_NCHITTEST` 重算），表现为点内容区偶尔闪一下焦点。
            let target = unsafe { GetParent(hwnd) };
            let target = if target.is_null() { hwnd } else { target };
            if crate::ui::frame::is_nc_drag(wp as isize) {
                DefWindowProcW(target, msg, wp, lp)
            } else {
                0
            }
        }

        WM_LBUTTONDBLCLK => {
            // 双击标题栏最大化。`WM_LBUTTONDBLCLK` 只在 class 有 `CS_DBLCLKS`
            // 时才会发（本类已设），且它先于 `WM_LBUTTONDOWN` 到达。
            if let Some(c) = ctx_of(hwnd) {
                let x = lo_word(lp) as f32 / c.scale;
                let y = hi_word(lp) as f32 / c.scale;
                let hwnd_parent = unsafe { GetParent(hwnd) };
                let target = if hwnd_parent.is_null() { hwnd } else { hwnd_parent };
                let hit = crate::ui::frame::logical_hit(
                    x,
                    y,
                    client_pixel_size(hwnd).0 as f32 / c.scale,
                    client_pixel_size(hwnd).1 as f32 / c.scale,
                    crate::ui::titlebar::TITLE_H,
                    crate::ui::frame::owner_is_maximized(hwnd),
                );
                if hit == crate::ui::frame::HTCAPTION {
                    crate::ui::frame::toggle_maximize(target);
                    return 0;
                }
            }
            // 不是标题栏就按普通点击处理。
            //
            // 双击时系统只会给第一下发 `WM_LBUTTONDOWN`，第二次按下变成这里
            // 的 `WM_LBUTTONDBLCLK`；以前直接交给 `DefWindowProc`，页面根本
            // 收不到 —— 表现是双击开关、双击「刷新」这类操作第二下被吞掉。
            // 转一手同坐标的 `WM_LBUTTONDOWN` 让按下/抬起链路完整走两遍。
            unsafe { SendMessageW(hwnd, WM_LBUTTONDOWN, wp, lp) };
            0
        }

        WM_KEYDOWN => {
            if let Some(c) = ctx_of(hwnd) {
                let mut changed = false;
                match wp {
                    VK_BACK => changed = c.ui.borrow_mut().backspace(),
                    VK_ESCAPE => changed = c.ui.borrow_mut().cancel_edit(),
                    // 回车 = 点「应用」：把草稿交给页面去校验并写设备。
                    // 制表符只收焦点，不提交（用户还在往下填）。
                    VK_RETURN => {
                        if let Some(e) = c.ui.borrow_mut().commit_focus() {
                            c.acts.borrow_mut().push(Act::ApplyEdit(e));
                            changed = true;
                        }
                    }
                    VK_TAB => changed = c.ui.borrow_mut().commit_focus().is_some(),
                    _ => {}
                }
                if changed {
                    (c.damage)();
                }
            }
            0
        }

        // 字符输入走 `WM_CHAR` 而不是 `WM_KEYDOWN`：输入法、粘贴、键盘布局
        // 映射全由系统处理完，我们只收最终字符。
        WM_CHAR => {
            if let Some(c) = ctx_of(hwnd) {
                let changed = match char::from_u32(wp as u32) {
                    // 控制字符（回车/退格/制表）已经在 `WM_KEYDOWN` 里处理过，
                    // 不挡掉会变成输入框里的方块乱码
                    Some(ch) if !ch.is_control() => c.ui.borrow_mut().type_char(ch),
                    _ => false,
                };
                if changed {
                    (c.damage)();
                }
            }
            0
        }

        WM_SETCURSOR => {
            if let Some(c) = ctx_of(hwnd) {
                let hit = lo_word(lp) as isize;
                let id = match resize_cursor_id(hit) {
                    Some(id) => id,
                    None => cursor_id(c.ui.borrow().cursor_hint()),
                };
                // **每次都 SetCursor**，不做「id 没变就跳过」的缓存。
                //
                // 踩过的坑：Explorer 在我们启动慢的时候会把光标设成
                // `IDC_APPSTARTING`（箭头 + 转圈）。我们的缓存里记的还是
                // `IDC_ARROW`，于是判断「没变」直接跳过 `SetCursor`，
                // 那个转圈光标就一直留在屏幕上 —— 正是用户说的「进入软件后
                // 鼠标一直转圈，移到侧边栏可点的地方才恢复」（因为那里
                // `cursor_hint` 变成手型，id 变了才走到 `SetCursor`）。
                // `LoadCursorW` 对系统光标只是查表，不分配，开销可忽略。
                c.cursor.set(id);
                let cur = LoadCursorW(std::ptr::null_mut(), idc(id));
                if !cur.is_null() {
                    SetCursor(cur);
                }
            }
            // 返回 1 = 已处理，别让 `DefWindowProc` 再改回箭头
            1
        }

        WM_DESTROY => {
            let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ctx;
            if !raw.is_null() {
                (*raw).animation.suspend(hwnd);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                // SAFETY: 指针由 `Canvas::create` 用 `Box::into_raw` 写入，此处只收回一次。
                drop(Box::from_raw(raw));
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }

        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// 把渲染用的内存还回去：画布位图 + 字体缓存。窗口重新可见时按需重建。
///
/// 最小化或收进托盘后没有任何可见像素，留着一张 32bpp 位图（1100x740 约
/// 3.1 MB、曾最大化则约 14 MB）纯属浪费。
fn release_render_memory(c: &Ctx) {
    c.inner.borrow_mut().release();
    c.fonts.borrow_mut().clear();
    crate::win32::trim_working_set();
}

#[cfg(test)]
mod animation_tests {
    use super::*;
    use native_windows_gui as nwg;
    use winapi::um::winuser as wu;

    struct TestCanvas {
        canvas: Canvas,
        owner: nwg::Window,
        handler: nwg::RawEventHandler,
    }

    impl TestCanvas {
        fn new() -> Self {
            nwg::init().unwrap();
            let mut owner = nwg::Window::default();
            nwg::Window::builder().title("Animation regression")
                .flags(nwg::WindowFlags::POPUP).position((-32000, -32000))
                .size((800, 600)).build(&mut owner).unwrap();
            let canvas = Canvas::create(owner.handle.hwnd().unwrap() as _,
                0, 0, 800, 600, 1.0, Rc::new(|| {})).unwrap();
            let events = canvas.clone();
            let handler = nwg::bind_raw_event_handler(&owner.handle, 0x10170,
                move |_, message, param, data| {
                    if message == wu::WM_WINDOWPOSCHANGING {
                        let position = unsafe { &mut *(data as *mut wu::WINDOWPOS) };
                        position.x = -32000;
                        position.y = -32000;
                        position.flags &= !wu::SWP_NOMOVE;
                    }
                    if matches!(message, WM_ENTERSIZEMOVE | WM_EXITSIZEMOVE) {
                        events.set_resizing(message == WM_ENTERSIZEMOVE);
                    }
                    if message == WM_SIZE && param == SIZE_MINIMIZED {
                        events.release_render_memory();
                    }
                    None
                }).unwrap();
            unsafe { wu::ShowWindow(owner.handle.hwnd().unwrap(), wu::SW_SHOWNOACTIVATE); }
            assert!(Animation::visible(canvas.hwnd()));
            Self { canvas, owner, handler }
        }

        fn paint_pages(&self) {
            let readings = [crate::device::DeviceReading {
                key: "animation-device".into(), name: "Test mouse".into(),
                family: crate::device::DeviceFamily::Mouse,
                connected: true,
                battery: Some(crate::device::Battery {
                    percent: Some(50), charging: Some(true), estimated_percent: None,
                    voltage_mv: None, approximate: false, level: None,
                }),
                ..Default::default()
            }, crate::device::DeviceReading {
                key: "second-animation-device".into(), name: "Second test mouse".into(),
                family: crate::device::DeviceFamily::Mouse, connected: true,
                ..Default::default()
            }];
            self.canvas.with_ui_mut(|ui| ui.sync_devices(&readings));
            let cfg = crate::settings::Settings::default();
            let estimate = crate::history::Estimate::default();
            let audio = crate::audio::Snapshot::default();
            self.canvas.set_painter(Box::new(move |inner, fonts, ui, args| {
                let data = Data {
                    selected: Some(&readings[0]), state: None, ident: None, link_mode: "",
                    status: String::new(), plot: None, estimate: &estimate, rows: 0,
                    cfg: &cfg, autostart_text: "", autostart_on: false,
                    native_notifications_supported: false, version: "test",
                    readings: &readings, audio: &audio, basic: None,
                    store: crate::store::View {
                        phase: crate::store::Phase::Unsupported, available: false,
                        home_visible: false, failure: None, last_checked_day: None,
                    },
                };
                draw_frame(inner, fonts, ui, args, &crate::ui::theme::LIGHT, &data)
            }));
            self.canvas.repaint_now();
            assert!(self.canvas.inner.borrow().covers(800, 600));
        }

        fn send(&self, message: u32, param: usize) {
            unsafe { SendMessageW(self.canvas.hwnd(), message, param, 0); }
        }

        fn dispatch_timer(&self, timer: usize) {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let mut message: wu::MSG = unsafe { std::mem::zeroed() };
                if unsafe { wu::PeekMessageW(&mut message, self.canvas.hwnd() as _,
                    WM_TIMER, WM_TIMER, wu::PM_REMOVE) } != 0 {
                    let matched = message.wParam == timer;
                    unsafe { wu::DispatchMessageW(&message); }
                    if matched { return; }
                }
                assert!(Instant::now() < deadline, "timer {timer} did not fire");
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        fn assert_stopped(&self) {
            assert!(!self.canvas.animation.running.get());
            assert_eq!(unsafe { KillTimer(self.canvas.hwnd(), ANIM_TIMER_ID) }, 0);
        }

        fn assert_no_settle_timer(&self) {
            assert!(self.canvas.animation.settle_until.get().is_none());
            assert_eq!(unsafe { KillTimer(self.canvas.hwnd(), SETTLE_TIMER_ID) }, 0);
        }
    }

    impl Drop for TestCanvas {
        fn drop(&mut self) {
            nwg::unbind_raw_event_handler(&self.handler).unwrap();
        }
    }

    #[test]
    fn resize_settle_restores_existing_animation_demand() {
        let fixture = TestCanvas::new();
        fixture.canvas.set_animating(true);
        fixture.dispatch_timer(ANIM_TIMER_ID);
        fixture.send(WM_SIZE, 0);
        fixture.assert_stopped();
        fixture.canvas.set_animating(true);
        fixture.assert_stopped();
        fixture.dispatch_timer(SETTLE_TIMER_ID);
        assert!(fixture.canvas.animation.running.get());
        fixture.dispatch_timer(ANIM_TIMER_ID);
        fixture.assert_no_settle_timer();
    }

    #[test]
    fn parent_resize_stays_paused_until_exit_and_is_per_canvas() {
        let first = TestCanvas::new();
        let second = TestCanvas::new();
        first.canvas.set_animating(true);
        second.canvas.set_animating(true);
        unsafe { wu::SendMessageW(first.owner.handle.hwnd().unwrap(), WM_ENTERSIZEMOVE, 0, 0); }
        first.send(WM_SIZE, 0);
        first.dispatch_timer(SETTLE_TIMER_ID);
        first.assert_stopped();
        second.dispatch_timer(ANIM_TIMER_ID);
        unsafe { wu::SendMessageW(first.owner.handle.hwnd().unwrap(), WM_EXITSIZEMOVE, 0, 0); }
        first.dispatch_timer(ANIM_TIMER_ID);
        first.assert_no_settle_timer();
    }

    #[test]
    fn queued_settle_tick_does_not_end_a_new_resize_pause() {
        let fixture = TestCanvas::new();
        fixture.canvas.set_animating(true);
        fixture.send(WM_SIZE, 0);
        assert_ne!(unsafe { wu::PostMessageW(fixture.canvas.hwnd() as _,
            WM_TIMER, SETTLE_TIMER_ID, 0) }, 0);
        fixture.send(WM_SIZE, 0);
        fixture.dispatch_timer(SETTLE_TIMER_ID);
        fixture.assert_stopped();
        assert!(fixture.canvas.animation.settle_until.get().is_some());
        fixture.dispatch_timer(SETTLE_TIMER_ID);
        fixture.dispatch_timer(ANIM_TIMER_ID);
        fixture.assert_no_settle_timer();
    }

    #[test]
    fn painted_hover_runs_to_completion_without_external_animation_requests() {
        let fixture = TestCanvas::new();
        fixture.paint_pages();
        fixture.assert_stopped();
        let card = fixture.canvas.with_ui(|ui| {
            ui.hots.iter().find(|hot| hot.act == Act::SelectDevice(0)).unwrap().rect
        });
        let position = card.cx() as isize | ((card.cy() as isize) << 16);
        unsafe { SendMessageW(fixture.canvas.hwnd(), WM_MOUSEMOVE, 0, position); }
        fixture.canvas.repaint_now();
        assert!(fixture.canvas.animation.running.get());
        for _ in 0..12 {
            if !fixture.canvas.animation.running.get() { break; }
            fixture.dispatch_timer(ANIM_TIMER_ID);
            fixture.canvas.repaint_now();
        }
        assert_eq!(fixture.canvas.with_ui(|ui| ui.card_hover_anims["animation-device"]), 1.0);
        fixture.assert_stopped();
        fixture.send(WM_MOUSELEAVE, 0);
        fixture.canvas.repaint_now();
        assert!(fixture.canvas.animation.running.get());
        for _ in 0..12 {
            if !fixture.canvas.animation.running.get() { break; }
            fixture.dispatch_timer(ANIM_TIMER_ID);
            fixture.canvas.repaint_now();
        }
        assert_eq!(fixture.canvas.with_ui(|ui| ui.card_hover_anims["animation-device"]), 0.0);
        fixture.assert_stopped();
    }

    #[test]
    fn drag_lift_and_reflow_run_only_while_the_painted_transition_is_active() {
        let fixture = TestCanvas::new();
        fixture.paint_pages();
        let source = fixture.canvas.with_ui(|ui| {
            ui.hots.iter().find(|hot| hot.act == Act::SelectDevice(0)).unwrap().rect
        });
        fixture.canvas.with_ui_mut(|ui| {
            ui.pointer.down = true;
            ui.pointer.press_x = source.cx();
            ui.pointer.press_y = source.cy();
            ui.on_press(source.cx(), source.cy(), 0.0);
        });
        let moved = (source.cx() as isize + 6) | ((source.cy() as isize) << 16);
        unsafe { SendMessageW(fixture.canvas.hwnd(), WM_MOUSEMOVE, 0, moved); }
        assert!(fixture.canvas.with_ui(|ui| ui.device_drag.is_some()));
        fixture.canvas.repaint_now();
        fixture.dispatch_timer(ANIM_TIMER_ID);
        fixture.canvas.with_ui_mut(|ui| {
            let drag = ui.device_drag.as_mut().unwrap();
            drag.started_at = Instant::now() - Duration::from_secs(1);
            drag.target_changed_at = drag.started_at;
        });
        fixture.canvas.repaint_now();
        fixture.assert_stopped();
        let target = fixture.canvas.with_ui(|ui| {
            ui.hots.iter().find(|hot| hot.act == Act::SelectDevice(1)).unwrap().rect
        });
        let moved = target.cx() as isize | ((target.cy() as isize) << 16);
        unsafe { SendMessageW(fixture.canvas.hwnd(), WM_MOUSEMOVE, 0, moved); }
        fixture.canvas.repaint_now();
        fixture.dispatch_timer(ANIM_TIMER_ID);
        fixture.canvas.with_ui_mut(|ui| {
            let drag = ui.device_drag.as_mut().unwrap();
            assert_eq!(drag.target, 1);
            drag.target_changed_at = Instant::now() - Duration::from_secs(1);
        });
        fixture.canvas.repaint_now();
        fixture.assert_stopped();
        assert!(fixture.canvas.with_ui(|ui| ui.device_drag.is_some()));
    }

    #[test]
    fn toast_expiry_stops_timer_and_static_charging_does_not_restart_it() {
        let fixture = TestCanvas::new();
        fixture.canvas.with_ui_mut(|ui| {
            ui.page = Page::Battery;
            ui.set_status("Test status");
        });
        fixture.paint_pages();
        fixture.dispatch_timer(ANIM_TIMER_ID);
        fixture.canvas.with_ui_mut(|ui| ui.status_time = Some(Instant::now() - Duration::from_secs(3)));
        fixture.canvas.repaint_now();
        assert!(fixture.canvas.with_ui(|ui| ui.status.is_empty()));
        fixture.assert_stopped();
        fixture.canvas.repaint_now();
        fixture.assert_stopped();
    }

    #[test]
    fn hiding_cancels_both_timers_and_stale_ticks_do_not_repaint() {
        let fixture = TestCanvas::new();
        fixture.canvas.set_animating(true);
        fixture.send(WM_SIZE, 0);
        unsafe { wu::ShowWindow(fixture.canvas.hwnd() as _, wu::SW_HIDE); }
        fixture.assert_stopped();
        fixture.assert_no_settle_timer();
        fixture.canvas.set_animating(true);
        fixture.send(WM_SIZE, 0);
        unsafe { wu::ValidateRect(fixture.canvas.hwnd() as _, std::ptr::null()); }
        fixture.send(WM_TIMER, ANIM_TIMER_ID);
        fixture.send(WM_TIMER, SETTLE_TIMER_ID);
        assert_eq!(unsafe { wu::GetUpdateRect(fixture.canvas.hwnd() as _, std::ptr::null_mut(), 0) }, 0);
        fixture.assert_stopped();
        fixture.assert_no_settle_timer();
        fixture.canvas.with_ui_mut(|ui| ui.set_status("Visible again"));
        unsafe { wu::ShowWindow(fixture.canvas.hwnd() as _, wu::SW_SHOWNOACTIVATE); }
        fixture.paint_pages();
        fixture.dispatch_timer(ANIM_TIMER_ID);
    }

    #[test]
    fn minimized_parent_suspends_animation_until_a_visible_frame() {
        let fixture = TestCanvas::new();
        fixture.canvas.with_ui_mut(|ui| ui.set_status("Test status"));
        fixture.paint_pages();
        fixture.dispatch_timer(ANIM_TIMER_ID);
        unsafe { wu::ShowWindow(fixture.owner.handle.hwnd().unwrap(), wu::SW_SHOWMINNOACTIVE); }
        assert_ne!(unsafe { wu::IsIconic(fixture.owner.handle.hwnd().unwrap()) }, 0);
        fixture.canvas.set_animating(true);
        fixture.send(WM_TIMER, ANIM_TIMER_ID);
        fixture.assert_stopped();
        fixture.assert_no_settle_timer();
        unsafe { wu::ShowWindow(fixture.owner.handle.hwnd().unwrap(), wu::SW_SHOWNOACTIVATE); }
        assert_eq!(unsafe { wu::IsIconic(fixture.owner.handle.hwnd().unwrap()) }, 0);
        fixture.canvas.repaint_now();
        fixture.dispatch_timer(ANIM_TIMER_ID);
    }
}
