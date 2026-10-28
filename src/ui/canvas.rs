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

use crate::ui::gfx::{self, Canvas as GfxCanvas, Fonts, Painter, R};
use crate::ui::layout::Frame;
use crate::ui::pages::{self, Data};
use crate::ui::theme::Theme;
use crate::ui::widget::{Act, Cursor, Slide, Ui};
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
const WM_SETCURSOR: u32 = 0x0020;
const WM_GETDLGCODE: u32 = 0x0087;
const WM_KEYDOWN: u32 = 0x0100;
const WM_CHAR: u32 = 0x0102;
/// `SetTimer` 的节拍。
///
/// **这个常量没写进 match 之前绝不能省** —— 少一个 `const` 声明时，
/// `WM_TIMER => ...` 会被 Rust 当成「新绑定变量」，静默匹配**所有**消息，
/// 编译只报 `warning: unused variable`，运行起来整个窗口过程全走那一支。
/// 我这次真踩了：加完 `WM_TIMER` 分支就收到这两条警告。
const WM_TIMER: u32 = 0x0113;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_MOUSEWHEEL: u32 = 0x020A;
const WM_LBUTTONDBLCLK: u32 = 0x0203;
/// 非客户区命中测试。内容区铺满整个客户区，所以这个也发到我们这里。
const WM_NCHITTEST: u32 = 0x0084;
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

// `PAINTSTRUCT` 用 [`crate::win32::PaintStruct`]（那里已有单测钉死大小和偏移）。
// 本文件不重复声明 —— 同一个 Win32 函数在两个模块里各声明一次、
// 签名还不一致时，Rust 会报 `clashing_extern_declarations`，
// 而且**编译器只警告不报错**，一不小心就会调错那个。

#[link(name = "user32")]
extern "system" {
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
    fn SetWindowPos(
        hwnd: Handle,
        after: Handle,
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
    /// 取窗口矩形（**屏幕坐标**，含边框）。命中测试要拿它把屏幕坐标换成客户区坐标。
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

/// 充电动效定时器的 ID。`SetTimer` 的 id 只要在**这个窗口**内唯一即可，
/// 取 1 是因为本窗口只有一个定时器。注意必须 >0：传 0 会让系统自动分配 id
/// 并返回它，那样 `KillTimer(0)` 就杀不掉，定时器一直跑。
const ANIM_TIMER_ID: usize = 1;
/// 动效帧间隔。16ms ≈ 60fps，和显示器刷新对齐就够了；
/// 更快不会更顺（合成器本来就在 60Hz 合成），只会白烧 CPU。
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

/// 画一整帧：背景 + 当前页（可滚动）+ 左侧导航（不滚动）。
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
    // 建内存位图要一个「参考 DC」。**不能用空句柄**：
    // `CreateCompatibleBitmap(NULL, w, h)` 会返回一张 **1bpp 单色**位图 ——
    // 它不报错，只是画出来全是黑白噪点。所以必须取一次屏幕 DC。
    // 取/还各一次系统调用，比缓存一个长期句柄安全得多（屏幕 DC 不该长持）。
    let sdc = unsafe { crate::win32::GetDC(std::ptr::null_mut()) };
    let mem = inner.ensure(sdc as crate::ui::gfx::Gdi, a.pixel_w, a.pixel_h);
    if !sdc.is_null() {
        unsafe { crate::win32::ReleaseDC(std::ptr::null_mut(), sdc) };
    }
    if mem == 0 {
        return 0;
    }

    // **每帧清一次命中区**，而且只在第一趟之前清 —— 清两次会把页面刚登记的
    // 内容区抹掉，表现是「点了有反应但点哪儿都不准」。
    ui.begin_frame();

    let nav_w = ui.nav_width();
    let page = ui.page;
    let scroll = ui.scroll_y(page);

    // 自绘标题栏占了顶部一条，页面和导航整体下移 —— 用 `oy` 交给世界变换，
    // 页面代码里的坐标仍然「从内容区顶部算起」。
    let oy = crate::ui::titlebar::TITLE_H;

    // `Frame::new` 算的是「相对页面区域」的边距（它会先把导航栏宽度从
    // 可用宽度里刨掉），所以把导航栏宽度补回 `content.x`，让页面和导航
    // 共用同一个坐标系 —— 世界变换只需要一个。
    let mut frame = Frame::new(a.view_w, a.view_h, nav_w);
    frame.content.x += nav_w;
    // 内容区顶部也要让出标题栏：页面从 `oy` 开始排
    frame.content.y += oy;
    frame.content.h -= oy;

    // 最大化状态要问**父窗口**：canvas 是 `WS_CHILD`，它自己没有
    // `WS_MAXIMIZE` 位，查它恒为 false，标题栏上的按钮图标就永远画成「最大化」。
    let parent = unsafe { GetParent(a.hwnd) };
    let is_max = !parent.is_null() && crate::ui::frame::is_maximized(parent);
    let hover_btn = ui.hover_btn;

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
            pages::page(p, ui, data, &frame);
        },
    );

    // ③ 导航侧栏（不滚动，固定在左侧）
    inner.paint(
        fonts,
        theme,
        a.scale,
        0.0,
        nav_w,
        a.view_h,
        |p: &mut Painter| {
            pages::nav(p, ui, data, nav_w, a.view_h);
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
            crate::ui::titlebar::draw(p, a.view_w, nav_w, theme);
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
    mem
}


/// `WNDPROC` 需要的全部状态。
///
/// # 为什么和 [`Canvas`] 分成两份
///
/// 窗口过程和 gui.rs **同时**需要 `ui` / `acts` / `damage`，但两者的生命周期
/// 不同：`WM_DESTROY` 一定早于 `run()` 返回，而 `Canvas` 活到消息循环结束。
///
/// 早先的写法是「`Box::into_raw` 塞进 `GWLP_USERDATA`，再把 `*Box::from_raw`
/// 移出来返回给调用方」—— 那个 Box 在 return 的那一刻就被释放了，
/// 而 `GWLP_USERDATA` 还留着那个地址：**每一个鼠标消息都在解引用已释放的内存**。
/// 现在改成两边各持 `Rc` 克隆，窗口过程独占这一份 `Ctx`，谁先死都安全。
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
    /// 「当前页想要动效」。由 `Canvas::set_animating` 写，
    /// `WM_SHOWWINDOW`（窗口重新可见时）读它来决定要不要恢复定时器。
    ///
    /// 用 `Rc<Cell<bool>>` 而不是 `AtomicBool`：只在 GUI 线程上读写，
    /// 原子类型会让人误以为可以跨线程，而且得多加一层 `Arc`。
    anim_wanted: Rc<Cell<bool>>,
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
    /// 与 `Ctx.anim_wanted` 指向同一个 `Cell`。
    anim_wanted: Rc<Cell<bool>>,
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
            anim_wanted: Rc::new(Cell::new(false)),
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
            anim_wanted: Rc::clone(&me.anim_wanted),
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

    /// 告诉 canvas「当前页要不要一直重画」。
    ///
    /// 充电动效是唯一的使用者：它靠 `GetTickCount` 算相位，必须有人不停
    /// 触发重画才会动。调用点在 gui.rs 每帧画完后根据「当前页 + 充电状态」
    /// 算一次 —— 重复传同一个值没有额外代价（内部先比一下再决定动不动系统）。
    pub fn set_animating(&self, on: bool) {
        if self.anim_wanted.get() == on {
            return;
        }
        self.anim_wanted.set(on);
        // 不可见时先记下来，`WM_SHOWWINDOW` 会补上。
        //
        // **必须用 `IsWindowVisible` 而不是 `GetClientRect`**：窗口被隐藏
        // （收进托盘 `ShowWindow(hwnd, SW_HIDE)`）时客户区尺寸依然是
        // 1100×740，`r.width() > 0` 照样成立 —— 我第一版就是这么写的，
        // 结果托盘里躺着的窗口每 16ms 醒一次做无效重绘。
        // SAFETY: 句柄来自本结构，`IsWindowVisible` 对已销毁句柄只返回 0。
        let visible = unsafe { IsWindowVisible(self.hwnd) } != 0;
        set_timer(self.hwnd, on && visible);
    }

    /// 把子窗口铺满父窗口客户区。主窗口 `WM_SIZE` 时调。
    ///
    /// 用 `SetWindowPos` 而不是 `MoveWindow`：后者会顺带 `InvalidateRect`，
    /// 而我们紧接着自己要重画一帧，多刷一次只会闪。
    pub fn fit_parent(&self) {
        let parent = unsafe { GetParent(self.hwnd) };
        if parent.is_null() {
            return;
        }
        let (w, h) = client_pixel_size(parent);
        unsafe {
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                0,
                0,
                w,
                h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
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

/// 把 `WM_NCHITTEST` 的缩放命中码映射成系统缩放光标 id。
///
/// `WM_SETCURSOR` 的 `lparam` 低 16 位就是命中测试码 —— 边框上它会是
/// `HTLEFT/HTTOP/…`，以前完全没处理，所以边框永远是箭头。现在按方向给对应光标。
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

/// 打开或关掉动画定时器。
///
/// **重复调用要幂等**：`SetTimer` 用同一个 id 再调一次不会叠加，只是改间隔；
/// 但 `KillTimer` 一个不存在的 id 会失败（无害）。所以两个方向都不需要
/// 先查询当前状态，直接调即可。
fn set_timer(hwnd: Handle, on: bool) {
    unsafe {
        if on {
            SetTimer(hwnd, ANIM_TIMER_ID, ANIM_INTERVAL_MS, 0);
        } else {
            KillTimer(hwnd, ANIM_TIMER_ID);
        }
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
            0
        }

        WM_SIZE => {
            if let Some(c) = ctx_of(hwnd) {
                // SAFETY: 传空矩形 = 整个客户区。
                InvalidateRect(hwnd, std::ptr::null(), 0);
                (c.damage)();
            }
            0
        }

        // 充电动效的节拍。
        //
        // 只在「当前页有动效」时才会被 `set_animating` 打开，定时器一旦触发就
        // 重画一次 —— 用 `InvalidateRect` 而不是 `UpdateWindow`：定时器每 16ms
        // 来一次，同步重画会把消息队列堵住（用户拖窗口时明显卡）。
        // 定时器回调只负责「标脏」，真正的绘制仍是 WM_PAINT 那次。
        WM_TIMER => {
            if wp == ANIM_TIMER_ID {
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
            0
        }

        // 窗口被隐藏（最小化 / 收进托盘）时停掉定时器。
        //
        // 不这么做的话，最小化期间每 16ms 醒一次去标脏一个看不见的窗口，
        // 笔记本上能实测到 1~2% 的额外 CPU。
        //
        // `WM_SHOWWINDOW` 的 wparam 是「即将显示/隐藏」，**不是**「当前状态」。
        // 所以这里读 `anim_wanted`（用户想要动效吗）而不是查窗口可见性。
        WM_SHOWWINDOW => {
            let showing = wp != 0;
            if let Some(c) = ctx_of(hwnd) {
                set_timer(hwnd, showing && c.anim_wanted.get());
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
                let x = lo_word(lp) as f32 / c.scale;
                let y = hi_word(lp) as f32 / c.scale;
                let mut ui = c.ui.borrow_mut();
                let moved = (ui.pointer.x - x).abs() > 0.01 || (ui.pointer.y - y).abs() > 0.01;
                ui.pointer.x = x;
                ui.pointer.y = y;
                if moved {
                    // 标题栏按钮的悬停也在这里刷一遍：
                    // `hover_btn` 原本只在 `WM_NCHITTEST` 里更新，而命中测试
                    // 消息在某些输入路径（远程桌面、触控、合成消息）下不一定
                    // 每次都来，表现就是「鼠标移上去按钮不高亮」。这里补一次。
                    let hb = if y <= crate::ui::titlebar::TITLE_H && y >= 0.0 {
                        let (pw, _) = client_pixel_size(hwnd);
                        crate::ui::titlebar::hit(x, y, pw as f32 / c.scale)
                    } else {
                        None
                    };
                    let mut hb_changed = false;
                    if ui.hover_btn != hb {
                        ui.hover_btn = hb;
                        hb_changed = true;
                    }
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
                        let hover_changed = ui.refresh_hover();
                        if hover_changed || hb_changed {
                            drop(ui);
                            unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
                            (c.damage)();
                        }
                    }
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
                ReleaseCapture();
                let x = lo_word(lp) as f32 / c.scale;
                let y = hi_word(lp) as f32 / c.scale;
                let mut pushed: Option<Act> = None;
                let dragging: Option<Slide>;
                {
                    let mut ui = c.ui.borrow_mut();
                    ui.pointer.x = x;
                    ui.pointer.y = y;
                    ui.pointer.down = false;
                    dragging = ui.pointer.dragging.take();
                    if dragging.is_none() {
                        let scroll = ui.scroll_y(ui.page);
                        pushed = ui.on_release(x, y, scroll);
                    }
                    ui.pressed_slide = None;
                }
                if let Some(s) = dragging {
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

        // ── 自绘标题栏的窗口行为 ──────────────────────────────
        //
        // 内容区子窗口铺满整个客户区，所以**鼠标消息全先到这里**，主窗口
        // 收不到 `WM_NCHITTEST`。这就是为什么标题栏的命中测试必须在这里做。
        //
        // 我第一版把子类挂到主窗口上（`nwg::bind_raw_event_handler`），
        // 结果整个标题栏拖不动、三条按钮点不动 —— 消息压根没到主窗口。
        WM_NCHITTEST => {
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
                crate::ui::frame::is_maximized(hwnd),
            );
            // 悬停高亮：命中变化时重画一帧
            let want_btn = crate::ui::titlebar::hit(x, y, cw);
            let mut ui = c.ui.borrow_mut();
            if ui.hover_btn != want_btn {
                ui.hover_btn = want_btn;
                drop(ui);
                (c.damage)();
            }
            hit
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
                    crate::ui::frame::is_maximized(target),
                );
                if hit == crate::ui::frame::HTCAPTION {
                    crate::ui::frame::toggle_maximize(target);
                    return 0;
                }
            }
            // 不是标题栏就按普通点击处理（让页面拿到这次点击）
            DefWindowProcW(hwnd, msg, wp, lp)
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

        // 悬停换光标：边框上是缩放箭头；内容里是手型/输入 I 型。
        //
        // `lparam` 低 16 位 = 命中测试码（就是 `WM_NCHITTEST` 的返回值）。
        // 边框拖拽区会传 `HTLEFT/HTTOP/…`，以前这里只按内容提示设光标，
        // 于是边框上的鼠标一直保持箭头 —— 这就是"边框上鼠标不变缩放图标"的原因。
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
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                // SAFETY: 指针由 `Canvas::create` 用 `Box::into_raw` 写入，此处只收回一次。
                drop(Box::from_raw(raw));
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }

        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 鼠标坐标按有符号16位解码() {
        let lp = ((200isize) << 16) | 100;
        assert_eq!(lo_word(lp), 100);
        assert_eq!(hi_word(lp), 200);
        // 负坐标：拖到客户区左上之外，高位补 1
        let lp = ((-5isize) & 0xFFFF) | (((-7isize) & 0xFFFF) << 16);
        assert_eq!(lo_word(lp), -5, "无符号解码会把 -5 变成 65531");
        assert_eq!(hi_word(lp), -7);
    }

    #[test]
    fn paintstruct_尺寸够大() {
        // 少算字段会让 `BeginPaint` 越界写栈，崩溃点离现场很远、极难查。
        assert!(
            std::mem::size_of::<PaintStruct>() >= 72,
            "PaintStruct 只有 {} 字节，比 Win32 的 72 字节小",
            std::mem::size_of::<PaintStruct>()
        );
    }

    #[test]
    fn wndclassexw_尺寸正确() {
        // Win64 下 WNDCLASSEXW = 80 字节。漏字段时 `RegisterClassExW` 返回 0，
        // 错误码只是 ERROR_INVALID_PARAMETER，看不出是哪个字段。
        assert_eq!(
            std::mem::size_of::<WndClassExW>(),
            80,
            "WNDCLASSEXW 尺寸不对，结构体定义可能漏了字段"
        );
    }

    #[test]
    #[allow(non_snake_case)]
    fn 资源ID按指针传() {
        // `IDC_ARROW` 是 `MAKEINTRESOURCE(32512)`，即把整数当地址
        assert_eq!(idc(IDC_ARROW) as usize, 32512);
        assert_eq!(idc(IDC_HAND) as usize, 32649);
    }

    #[test]
    fn 输入法字符能过控制字符的筛() {
        // `WM_CHAR` 里那个 `!ch.is_control()` 的判断不能把中文挡掉
        assert!(!'\u{4E2D}'.is_control());
        assert!(!'a'.is_control());
        assert!('\r'.is_control());
        assert!('\t'.is_control());
    }
}
