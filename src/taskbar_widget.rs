//! Optional battery strip directly left of the Windows notification area.
//!
//! Windows has no public notification-area text API. This is our own layered,
//! non-activating child window embedded in the taskbar. It hides itself
//! if the taskbar is missing, vertical, RTL or covered by a full-screen app.

use crate::win32::{Handle, Rect};
use core::ffi::c_void;
mod render;
use std::{
    cell::{Cell, RefCell},
    ptr,
    sync::atomic::{AtomicU32, AtomicUsize, Ordering},
};

const CLASS_NAME: &str = "RBatteryTaskbarStrip";
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;
const WS_EX_LAYOUTRTL: u32 = 0x0040_0000;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_LAYERED: u32 = 0x0008_0000;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
const WS_CHILD: u32 = 0x4000_0000;
#[cfg(test)]
const WS_POPUP: u32 = 0x8000_0000;
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
#[cfg(test)]
const HWND_TOPMOST: Handle = !0usize as Handle;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOREDRAW: u32 = 0x0008;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_SHOWWINDOW: u32 = 0x0040;
const SWP_NOOWNERZORDER: u32 = 0x0200;
const ULW_ALPHA: u32 = 0x0000_0002;
const WM_DESTROY: u32 = 0x0002;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_DISPLAYCHANGE: u32 = 0x007e;
const WM_MOUSEACTIVATE: u32 = 0x0021;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_CANCELMODE: u32 = 0x001f;
const WM_DPICHANGED: u32 = 0x02e0;
const WM_THEMECHANGED: u32 = 0x031a;
const WM_DWMCOMPOSITIONCHANGED: u32 = 0x031e;
const WM_DWMCOLORIZATIONCOLORCHANGED: u32 = 0x0320;
const MA_NOACTIVATE: isize = 3;

pub const CLICK_QUERY_OPEN: usize = 1;
pub const CLICK_OPEN: usize = 2;
pub const CLICK_CLOSE: usize = 3;

#[derive(Clone, Copy, Default)]
struct ClickGesture {
    pressed: Option<(usize, bool)>,
}

impl ClickGesture {
    fn press(&mut self, window: usize, popup_visible: bool) {
        self.pressed = Some((window, popup_visible));
    }

    fn release(&mut self, window: usize, inside: bool) -> Option<usize> {
        let (pressed_window, was_visible) = self.pressed.take()?;
        (inside && pressed_window == window).then_some(if was_visible { CLICK_CLOSE } else { CLICK_OPEN })
    }

    fn cancel(&mut self, window: usize) {
        if self.pressed.is_some_and(|(pressed_window, _)| pressed_window == window) {
            self.pressed = None;
        }
    }
}

thread_local! {
    static CLICK_GESTURE: Cell<ClickGesture> = Cell::new(ClickGesture::default());
}

static NOTIFY_WINDOW: AtomicUsize = AtomicUsize::new(0);
static CLICK_MESSAGE: AtomicU32 = AtomicU32::new(0);
static CLASS_REGISTERED: AtomicU32 = AtomicU32::new(0);
static REPAINT_REQUESTED: AtomicU32 = AtomicU32::new(0);

type WndProc = unsafe extern "system" fn(Handle, u32, usize, isize) -> isize;

#[repr(C)]
struct WndClassExW {
    cb_size: u32,
    style: u32,
    wnd_proc: Option<WndProc>,
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
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
struct Size {
    cx: i32,
    cy: i32,
}

#[repr(C)]
struct BlendFunction {
    blend_op: u8,
    blend_flags: u8,
    source_constant_alpha: u8,
    alpha_format: u8,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassExW(class: *const WndClassExW) -> u16;
    fn CreateWindowExW(
        ex_style: u32,
        class_name: *const u16,
        window_name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: Handle,
        menu: Handle,
        instance: Handle,
        parameter: *mut c_void,
    ) -> Handle;
    fn DefWindowProcW(hwnd: Handle, msg: u32, wp: usize, lp: isize) -> isize;
    fn FindWindowExW(
        parent: Handle,
        after: Handle,
        class_name: *const u16,
        title: *const u16,
    ) -> Handle;
    fn GetWindowRect(hwnd: Handle, out: *mut Rect) -> i32;
    fn GetClientRect(hwnd: Handle, out: *mut Rect) -> i32;
    fn GetParent(hwnd: Handle) -> Handle;
    fn ScreenToClient(hwnd: Handle, point: *mut Point) -> i32;
    fn SetCapture(hwnd: Handle) -> Handle;
    fn GetCapture() -> Handle;
    fn ReleaseCapture() -> i32;
    fn SetWindowPos(
        hwnd: Handle,
        after: Handle,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> i32;
    fn ShowWindow(hwnd: Handle, command: i32) -> i32;
    fn IsWindow(hwnd: Handle) -> i32;
    fn DestroyWindow(hwnd: Handle) -> i32;
    fn GetWindowLongW(hwnd: Handle, index: i32) -> i32;
    fn GetForegroundWindow() -> Handle;
    fn GetShellWindow() -> Handle;
    fn GetWindowThreadProcessId(hwnd: Handle, process_id: *mut u32) -> u32;
    fn GetCurrentProcessId() -> u32;
    fn IsWindowVisible(hwnd: Handle) -> i32;
    fn GetDpiForWindow(hwnd: Handle) -> u32;
    fn PostMessageW(hwnd: Handle, msg: u32, wp: usize, lp: isize) -> i32;
    fn LoadCursorW(instance: Handle, cursor: *const u16) -> Handle;
    fn UpdateLayeredWindow(
        hwnd: Handle,
        dst: Handle,
        dst_point: *const Point,
        size: *const Size,
        src: Handle,
        src_point: *const Point,
        key: u32,
        blend: *const BlendFunction,
        flags: u32,
    ) -> i32;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StripContent {
    pub items: Vec<StripItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StripItem {
    pub level: Option<crate::device::BatteryPercent>,
    pub charging: bool,
    pub kind: crate::tray_prefs::TrayKind,
}

pub struct TaskbarWidget {
    hwnd: Cell<Handle>,
    taskbar: Cell<Handle>,
    enabled: Cell<bool>,
    light: Cell<bool>,
    content: RefCell<Option<StripContent>>,
    layout: RefCell<Option<(StripContent, bool, u32, render::Layout)>>,
    rendered: Cell<Option<(usize, i32)>>,
    rendered_size: Cell<(i32, i32)>,
    hide_ticks: Cell<u8>,
    last_pos: Cell<(i32, i32)>,
}

unsafe extern "system" fn window_proc(hwnd: Handle, msg: u32, wp: usize, lp: isize) -> isize {
    match msg {
        WM_LBUTTONDOWN => {
            let notify = NOTIFY_WINDOW.load(Ordering::Relaxed) as Handle;
            let message = CLICK_MESSAGE.load(Ordering::Relaxed);
            if !notify.is_null() && message != 0 {
                // Capture actual visibility at mouse-down, before focus changes.
                let visible = unsafe { crate::win32::SendMessageW(notify, message, CLICK_QUERY_OPEN, 0) } != 0;
                CLICK_GESTURE.with(|cell| {
                    let mut gesture = cell.get();
                    gesture.press(hwnd as usize, visible);
                    cell.set(gesture);
                });
                unsafe { SetCapture(hwnd); }
            }
            0
        }
        WM_LBUTTONUP => {
            let mut rect = Rect::default();
            let x = (lp as u16) as i16 as i32;
            let y = ((lp >> 16) as u16) as i16 as i32;
            let inside = unsafe { GetClientRect(hwnd, &mut rect) } != 0
                && x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom;
            let action = CLICK_GESTURE.with(|cell| {
                let mut gesture = cell.get();
                let action = gesture.release(hwnd as usize, inside);
                cell.set(gesture);
                action
            });
            if unsafe { GetCapture() } == hwnd { unsafe { ReleaseCapture(); } }
            let notify = NOTIFY_WINDOW.load(Ordering::Relaxed) as Handle;
            let message = CLICK_MESSAGE.load(Ordering::Relaxed);
            let action = action.or_else(|| {
                if !inside || notify.is_null() || message == 0 {
                    return None;
                }
                let visible = unsafe {
                    crate::win32::SendMessageW(notify, message, CLICK_QUERY_OPEN, 0)
                } != 0;
                Some(if visible { CLICK_CLOSE } else { CLICK_OPEN })
            });
            if let Some(action) = action.filter(|_| !notify.is_null() && message != 0) {
                unsafe { PostMessageW(notify, message, action, 0) };
            }
            0
        }
        WM_CANCELMODE | WM_DESTROY => {
            CLICK_GESTURE.with(|cell| {
                let mut gesture = cell.get();
                gesture.cancel(hwnd as usize);
                cell.set(gesture);
            });
            if msg == WM_CANCELMODE && unsafe { GetCapture() } == hwnd { unsafe { ReleaseCapture(); } }
            0
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED | WM_THEMECHANGED | WM_DWMCOMPOSITIONCHANGED
        | WM_DWMCOLORIZATIONCOLORCHANGED => {
            REPAINT_REQUESTED.store(1, Ordering::Release);
            0
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE,
        WM_ERASEBKGND => 1,
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

impl TaskbarWidget {
    pub fn new(notify: Handle, click_message: u32) -> Self {
        NOTIFY_WINDOW.store(notify as usize, Ordering::Relaxed);
        CLICK_MESSAGE.store(click_message, Ordering::Relaxed);
        REPAINT_REQUESTED.store(1, Ordering::Release);
        Self {
            hwnd: Cell::new(ptr::null_mut()),
            taskbar: Cell::new(ptr::null_mut()),
            enabled: Cell::new(false),
            light: Cell::new(false),
            content: RefCell::new(None),
            layout: RefCell::new(None),
            rendered: Cell::new(None),
            rendered_size: Cell::new((0, 0)),
            hide_ticks: Cell::new(0),
            last_pos: Cell::new((i32::MIN, i32::MIN)),
        }
    }

    fn hide_now(&self, hwnd: Handle) {
        if !hwnd.is_null() && unsafe { IsWindowVisible(hwnd) } != 0 {
            unsafe { ShowWindow(hwnd, 0) };
        }
        self.hide_ticks.set(0);
        self.rendered.set(None);
        self.rendered_size.set((0, 0));
        self.last_pos.set((i32::MIN, i32::MIN));
        REPAINT_REQUESTED.store(1, Ordering::Release);
    }

    fn request_hide(&self, hwnd: Handle, is_visible: bool) {
        if !is_visible {
            self.hide_ticks.set(0);
            return;
        }
        let count = self.hide_ticks.get().saturating_add(1);
        self.hide_ticks.set(count);
        if count >= 2 {
            self.hide_now(hwnd);
        }
    }

    pub fn sync(&self, enabled: bool, content: Option<StripContent>, light: bool) {
        self.enabled.set(enabled);
        self.light.set(light);
        *self.content.borrow_mut() = content;
        if !enabled {
            self.destroy();
            return;
        }
        self.ensure_window();
        self.paint_and_position();
    }

    pub fn tick(&self) {
        if !self.enabled.get() {
            self.destroy();
            return;
        }
        self.ensure_window();
        self.paint_and_position();
    }

    pub fn recreate(&self) {
        self.destroy();
        if self.enabled.get() {
            REPAINT_REQUESTED.store(1, Ordering::Release);
            self.ensure_window();
            self.paint_and_position();
        }
    }

    pub fn invalidate(&self) {
        REPAINT_REQUESTED.store(1, Ordering::Release);
    }

    pub fn screen_rect(&self) -> Option<Rect> {
        let hwnd = self.hwnd.get();
        if hwnd.is_null()
            || unsafe { IsWindow(hwnd) } == 0
            || unsafe { IsWindowVisible(hwnd) } == 0
        {
            return None;
        }
        let mut rect = Rect::default();
        if unsafe { GetWindowRect(hwnd, &mut rect) } != 0
            && rect.right > rect.left
            && rect.bottom > rect.top
        {
            return Some(rect);
        }
        let (screen_x, screen_y) = self.last_pos.get();
        let (width, height) = self.rendered_size.get();
        if screen_x != i32::MIN && width > 0 && height > 0 {
            Some(Rect::new(screen_x, screen_y, screen_x + width, screen_y + height))
        } else {
            None
        }
    }

    pub fn host_available(&self) -> bool {
        if !self.enabled.get()
            || !self
                .content
                .borrow()
                .as_ref()
                .is_some_and(|content| !content.items.is_empty())
        {
            return false;
        }
        let taskbar = self.taskbar.get();
        if taskbar.is_null() || unsafe { IsWindow(taskbar) } == 0 {
            return false;
        }
        let tray = unsafe {
            FindWindowExW(
                taskbar,
                ptr::null_mut(),
                crate::win32::to_wide("TrayNotifyWnd").as_ptr(),
                ptr::null(),
            )
        };
        if tray.is_null() {
            return false;
        }
        let mut bar = Rect::default();
        let mut tray_rect = Rect::default();
        if unsafe { GetWindowRect(taskbar, &mut bar) } == 0
            || unsafe { GetWindowRect(tray, &mut tray_rect) } == 0
            || unsafe { GetWindowLongW(taskbar, GWL_EXSTYLE) } as u32 & WS_EX_LAYOUTRTL != 0
            || bar.bottom - bar.top > bar.right - bar.left
        {
            return false;
        }
        horizontal_taskbar_supported(bar, tray_rect)
    }

    fn ensure_window(&self) {
        if !self.hwnd.get().is_null() && unsafe { IsWindow(self.hwnd.get()) } != 0
            && unsafe { IsWindow(self.taskbar.get()) } != 0
            && unsafe { GetParent(self.hwnd.get()) } == self.taskbar.get()
        {
            return;
        }
        self.destroy();
        let taskbar = unsafe {
            crate::win32::FindWindowW(crate::win32::to_wide("Shell_TrayWnd").as_ptr(), ptr::null())
        };
        if taskbar.is_null()
            || unsafe {
                FindWindowExW(
                    taskbar,
                    ptr::null_mut(),
                    crate::win32::to_wide("TrayNotifyWnd").as_ptr(),
                    ptr::null(),
                )
            }
            .is_null()
        {
            crate::diagnostics::event_once(
                "taskbar_widget_unavailable",
                "Shell_TrayWnd or TrayNotifyWnd was not found",
            );
            return;
        }
        self.taskbar.set(taskbar);
        self.hwnd.set(create_strip_window(taskbar));
    }

    fn paint_and_position(&self) {
        let hwnd = self.hwnd.get();
        let taskbar = self.taskbar.get();
        if hwnd.is_null() || taskbar.is_null() {
            return;
        }
        let was_visible = unsafe { IsWindowVisible(hwnd) } != 0;
        let mut bar = Rect::default();
        let tray = unsafe {
            FindWindowExW(
                taskbar,
                ptr::null_mut(),
                crate::win32::to_wide("TrayNotifyWnd").as_ptr(),
                ptr::null(),
            )
        };
        let mut tray_rect = Rect::default();
        // Only a full-screen window covering this taskbar's monitor hides it.
        let monitor = unsafe { crate::win32::MonitorFromWindow(taskbar, 2) };
        let mut monitor_info: crate::win32::MonitorInfo = unsafe { core::mem::zeroed() };
        monitor_info.cb_size = core::mem::size_of::<crate::win32::MonitorInfo>() as u32;
        if tray.is_null()
            || unsafe { GetWindowRect(taskbar, &mut bar) } == 0
            || unsafe { GetWindowRect(tray, &mut tray_rect) } == 0
            || unsafe { IsWindowVisible(taskbar) } == 0
            || unsafe { GetWindowLongW(taskbar, GWL_EXSTYLE) } as u32 & WS_EX_LAYOUTRTL != 0
            || bar.bottom - bar.top > bar.right - bar.left
            || monitor.is_null()
            || unsafe { crate::win32::GetMonitorInfoW(monitor, &mut monitor_info) } == 0
            || !horizontal_taskbar_exposed(bar, tray_rect, monitor_info.rc_monitor)
        {
            self.request_hide(hwnd, was_visible);
            return;
        }
        let foreground = unsafe { GetForegroundWindow() };
        let mut foreground_process = 0u32;
        if !foreground.is_null() {
            unsafe { GetWindowThreadProcessId(foreground, &mut foreground_process) };
        }
        let mut foreground_rect = Rect::default();
        let foreground_fullscreen = !foreground.is_null()
            && foreground_process != unsafe { GetCurrentProcessId() }
            && foreground != taskbar
            && foreground != hwnd
            && foreground != tray
            && foreground != unsafe { GetShellWindow() }
            && unsafe { IsWindowVisible(foreground) } != 0
            && unsafe { GetWindowRect(foreground, &mut foreground_rect) } != 0
            && covers_monitor(foreground_rect, monitor_info.rc_monitor);
        if foreground_fullscreen {
            self.hide_now(hwnd);
            return;
        }
        self.hide_ticks.set(0);
        let invalidated = REPAINT_REQUESTED.swap(0, Ordering::AcqRel) != 0;
        let dpi = unsafe { GetDpiForWindow(taskbar) };
        let scale = (if dpi == 0 { 96 } else { dpi }) as f32 / 96.0;
        let content = self.content.borrow().clone();
        let Some(content) = content.filter(|content| !content.items.is_empty()) else {
            self.hide_now(hwnd);
            return;
        };
        let mut layout = self.layout.borrow_mut();
        if !layout.as_ref().is_some_and(|(old, light, old_dpi, _)| {
            old == &content && *light == self.light.get() && *old_dpi == dpi
        }) {
            let Some(measured) = render::Layout::new(&content.items, self.light.get(), scale) else {
                self.hide_now(hwnd);
                return;
            };
            *layout = Some((content, self.light.get(), dpi, measured));
            self.rendered.set(None);
        }
        let measured = &layout.as_ref().unwrap().3;
        let available_width =
            (tray_rect.left - bar.left - (scale * 2.0).round() as i32).max(0);
        let visible = measured.visible_count(available_width);
        if visible == 0 {
            self.request_hide(hwnd, was_visible);
            return;
        }
        let height = bar.bottom - bar.top;
        let render_key = (visible, height);
        let needs_repaint =
            !was_visible || invalidated || self.rendered.get() != Some(render_key);
        let raster = if needs_repaint {
            let Some(raster) = measured.raster(visible, height) else {
                self.hide_now(hwnd);
                return;
            };
            Some(raster)
        } else {
            None
        };
        let (width, height) = if let Some(raster) = &raster {
            (raster.width, raster.height)
        } else {
            let size = self.rendered_size.get();
            if size.0 <= 0 || size.1 <= 0 {
                self.hide_now(hwnd);
                return;
            }
            size
        };
        let x = tray_rect.left - width - (scale * 2.0).round() as i32;
        let y = bar.top + (bar.bottom - bar.top - height) / 2;
        let current_pos = (x, y);
        let desired = Rect::new(x, y, x + width, y + height);
        let mut current = Rect::default();
        let geometry_changed = !was_visible
            || unsafe { GetWindowRect(hwnd, &mut current) } == 0
            || current != desired;
        if was_visible && !needs_repaint && !geometry_changed {
            return;
        }
        if let Some(raster) = &raster {
            if !paint_raster(hwnd, raster, current_pos) {
                self.hide_now(hwnd);
                return;
            }
            self.rendered.set(Some(render_key));
            self.rendered_size.set((raster.width, raster.height));
        }
        if !was_visible || (geometry_changed && raster.is_none()) {
            let mut client_position = Point { x, y };
            if unsafe { ScreenToClient(taskbar, &mut client_position) } == 0 {
                self.hide_now(hwnd);
                return;
            }
            let flags = SWP_NOACTIVATE
                | SWP_NOOWNERZORDER
                | if was_visible {
                    SWP_NOZORDER | SWP_NOREDRAW
                } else {
                    SWP_SHOWWINDOW | SWP_NOMOVE | SWP_NOSIZE
                };
            let positioned = unsafe {
                SetWindowPos(hwnd, ptr::null_mut(), client_position.x, client_position.y,
                    width, height, flags)
            };
            if positioned == 0 {
                crate::diagnostics::event_once(
                    "taskbar_widget_position_failed",
                    &format!("GetLastError={}", crate::win32::last_error()),
                );
                self.hide_now(hwnd);
                return;
            }
        }
        self.last_pos.set(current_pos);
    }

    fn destroy(&self) {
        let hwnd = self.hwnd.replace(ptr::null_mut());
        if !hwnd.is_null() && unsafe { IsWindow(hwnd) } != 0 {
            unsafe { DestroyWindow(hwnd) };
        }
        self.taskbar.set(ptr::null_mut());
        self.layout.borrow_mut().take();
        self.rendered.set(None);
        self.rendered_size.set((0, 0));
        self.hide_ticks.set(0);
        self.last_pos.set((i32::MIN, i32::MIN));
        REPAINT_REQUESTED.store(1, Ordering::Release);
    }
}

fn create_strip_window(taskbar: Handle) -> Handle {
    if CLASS_REGISTERED.swap(1, Ordering::AcqRel) == 0 {
        let class_name = crate::win32::to_wide(CLASS_NAME);
        let cursor = unsafe { LoadCursorW(ptr::null_mut(), 32512usize as *const u16) };
        let class = WndClassExW {
            cb_size: core::mem::size_of::<WndClassExW>() as u32,
            style: 0,
            wnd_proc: Some(window_proc),
            cls_extra: 0,
            wnd_extra: 0,
            h_instance: unsafe { crate::win32::GetModuleHandleW(ptr::null()) },
            h_icon: ptr::null_mut(),
            h_cursor: cursor,
            hbr_background: ptr::null_mut(),
            menu_name: ptr::null(),
            class_name: class_name.as_ptr(),
            h_icon_sm: ptr::null_mut(),
        };
        if unsafe { RegisterClassExW(&class) } == 0 {
            crate::diagnostics::event_once(
                "taskbar_widget_register_failed",
                &format!("GetLastError={}", crate::win32::last_error()),
            );
            CLASS_REGISTERED.store(0, Ordering::Release);
            return ptr::null_mut();
        }
    }
    let class_name = crate::win32::to_wide(CLASS_NAME);
    let window_name = crate::win32::to_wide("X-Battery Taskbar Strip");
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            window_name.as_ptr(),
            WS_CHILD | WS_CLIPSIBLINGS,
            0,
            0,
            1,
            1,
            taskbar,
            ptr::null_mut(),
            crate::win32::GetModuleHandleW(ptr::null()),
            ptr::null_mut(),
        )
    };
    if hwnd.is_null() {
        crate::diagnostics::event_once(
            "taskbar_widget_create_failed",
            &format!("GetLastError={}", crate::win32::last_error()),
        );
    }
    hwnd
}

fn paint_raster(hwnd: Handle, raster: &render::Raster, position: (i32, i32)) -> bool {
    let mut destination = Point { x: position.0, y: position.1 };
    if unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32 & WS_CHILD != 0
        && unsafe { ScreenToClient(GetParent(hwnd), &mut destination) } == 0
    {
        return false;
    }
    let Some(bitmap) = raster.bitmap() else { return false; };
    let screen = unsafe { crate::win32::GetDC(ptr::null_mut()) };
    if screen.is_null() {
        unsafe { crate::win32::DeleteObject(bitmap) };
        return false;
    }
    let memory = unsafe { crate::win32::CreateCompatibleDC(screen) };
    if memory.is_null() {
        unsafe {
            crate::win32::ReleaseDC(ptr::null_mut(), screen);
            crate::win32::DeleteObject(bitmap);
        }
        return false;
    }
    let previous = unsafe { crate::win32::SelectObject(memory, bitmap) };
    let size = Size { cx: raster.width, cy: raster.height };
    let source = Point { x: 0, y: 0 };
    let blend = BlendFunction {
        blend_op: 0,
        blend_flags: 0,
        source_constant_alpha: 255,
        alpha_format: 1,
    };
    let painted = unsafe {
        UpdateLayeredWindow(
            hwnd, screen, &destination, &size, memory, &source, 0, &blend, ULW_ALPHA,
        ) != 0
    };
    let error = (!painted).then(crate::win32::last_error);
    unsafe {
        crate::win32::SelectObject(memory, previous);
        crate::win32::DeleteObject(bitmap);
        crate::win32::DeleteDC(memory);
        crate::win32::ReleaseDC(ptr::null_mut(), screen);
    }
    if let Some(error) = error {
        crate::diagnostics::event_once(
            "taskbar_widget_paint_failed", &format!("GetLastError={error}"),
        );
    }
    painted
}

fn covers_monitor(window: Rect, monitor: Rect) -> bool {
    monitor.right > monitor.left && monitor.bottom > monitor.top
        && window.left <= monitor.left && window.top <= monitor.top
        && window.right >= monitor.right && window.bottom >= monitor.bottom
}

fn horizontal_taskbar_exposed(bar: Rect, tray: Rect, monitor: Rect) -> bool {
    horizontal_taskbar_supported(bar, tray)
        && bar.top >= monitor.top && bar.bottom <= monitor.bottom + 8
}

fn horizontal_taskbar_supported(bar: Rect, tray: Rect) -> bool {
    bar.right > bar.left && bar.bottom > bar.top
        && bar.bottom - bar.top <= bar.right - bar.left
        && tray.right > tray.left && tray.bottom > tray.top
        && tray.left >= bar.left && tray.right <= bar.right + 16
        && tray.top < bar.bottom && tray.bottom > bar.top
}

impl Drop for TaskbarWidget {
    fn drop(&mut self) {
        self.destroy();
        NOTIFY_WINDOW.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_click_uses_visibility_at_press_and_allows_immediate_reopen() {
        let mut gesture = ClickGesture::default();
        gesture.press(10, true);
        assert_eq!(gesture.release(10, true), Some(CLICK_CLOSE));
        // No time-based suppression after an outside click, Escape or another entry.
        gesture.press(10, false);
        assert_eq!(gesture.release(10, true), Some(CLICK_OPEN));
        assert_eq!(gesture.release(10, true), None);
    }

    #[test]
    fn strip_click_cancels_when_capture_is_lost_or_released_outside() {
        let mut gesture = ClickGesture::default();
        gesture.press(10, false);
        gesture.cancel(10);
        assert_eq!(gesture.release(10, true), None);
        gesture.press(10, true);
        assert_eq!(gesture.release(10, false), None);
        gesture.press(10, false);
        assert_eq!(gesture.release(20, true), None);
    }

    #[test]
    fn embedded_strip_keeps_surface_and_click_bounds_when_taskbar_or_popup_is_raised() {
        struct NativeWindow(Handle);
        impl Drop for NativeWindow {
            fn drop(&mut self) {
                unsafe { DestroyWindow(self.0); }
            }
        }
        let create_host = || {
            let class = crate::win32::to_wide("STATIC");
            let window = unsafe {
                CreateWindowExW(WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    class.as_ptr(), ptr::null(), WS_POPUP, 0, 0, 1, 1,
                    ptr::null_mut(), ptr::null_mut(),
                    crate::win32::GetModuleHandleW(ptr::null()), ptr::null_mut())
            };
            assert!(!window.is_null());
            NativeWindow(window)
        };
        let taskbar = create_host();
        assert!(!taskbar.0.is_null());
        let strip = NativeWindow(create_strip_window(taskbar.0));
        let popup = create_host();
        assert!(!strip.0.is_null() && !popup.0.is_null());
        assert_eq!(unsafe { GetParent(strip.0) }, taskbar.0);
        let mut raster = render::Raster {
            width: 128,
            height: 48,
            pixels: vec![0x0100_0000; 128 * 48],
        };
        let mut expected = Rect::new(-32000, -32000, -31872, -31952);
        let host_raster = render::Raster {
            width: 400, height: 48, pixels: vec![0x0100_0000; 400 * 48],
        };
        assert!(paint_raster(taskbar.0, &host_raster, (-32000, -32000)));
        assert!(paint_raster(strip.0, &raster, (-32000, -32000)));
        assert!(paint_raster(popup.0, &raster, (-32000, -32000)));
        let unchanged = SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOOWNERZORDER;
        for window in [taskbar.0, popup.0] {
            assert_ne!(unsafe {
                SetWindowPos(window, HWND_TOPMOST, 0, 0, 0, 0,
                    unchanged | SWP_SHOWWINDOW)
            }, 0);
        }
        assert_ne!(unsafe {
            SetWindowPos(strip.0, ptr::null_mut(), 0, 0, 0, 0,
                unchanged | SWP_SHOWWINDOW)
        }, 0);
        for _ in 0..8 {
            for window in [popup.0, taskbar.0] {
                assert_ne!(unsafe {
                    SetWindowPos(window, HWND_TOPMOST, 0, 0, 0, 0, unchanged)
                }, 0);
                assert_eq!(unsafe { GetParent(strip.0) }, taskbar.0);
                assert_ne!(unsafe { IsWindowVisible(strip.0) }, 0);
                let mut actual = Rect::default();
                assert_ne!(unsafe { GetWindowRect(strip.0, &mut actual) }, 0);
                assert_eq!(actual, expected);
                assert_ne!(unsafe { GetClientRect(strip.0, &mut actual) }, 0);
                assert_eq!(actual, Rect::new(0, 0, raster.width, raster.height));
            }
        }
        raster.width = 160;
        raster.pixels = vec![0x0100_0000; 160 * 48];
        expected = Rect::new(-31800, -32000, -31640, -31952);
        assert!(paint_raster(strip.0, &raster, (-31800, -32000)));
        let mut actual = Rect::default();
        assert_ne!(unsafe { GetWindowRect(strip.0, &mut actual) }, 0);
        assert_eq!(actual, expected);
        assert_ne!(unsafe { GetClientRect(strip.0, &mut actual) }, 0);
        assert_eq!(actual, Rect::new(0, 0, 160, 48));
        assert_ne!(unsafe {
            SetWindowPos(taskbar.0, HWND_TOPMOST, -31900, -31900, 0, 0,
                SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOOWNERZORDER)
        }, 0);
        assert_ne!(unsafe { GetWindowRect(strip.0, &mut actual) }, 0);
        assert_eq!(actual, Rect::new(-31700, -31900, -31540, -31852));
        unsafe { ShowWindow(taskbar.0, 0); }
        assert_eq!(unsafe { IsWindowVisible(strip.0) }, 0);
        assert_ne!(unsafe {
            SetWindowPos(taskbar.0, HWND_TOPMOST, 0, 0, 0, 0,
                unchanged | SWP_SHOWWINDOW)
        }, 0);
        assert_ne!(unsafe { IsWindowVisible(strip.0) }, 0);
        unsafe { ShowWindow(strip.0, 0); }
        assert_ne!(unsafe {
            SetWindowPos(strip.0, ptr::null_mut(), 0, 0, 0, 0,
                unchanged | SWP_NOZORDER | SWP_SHOWWINDOW)
        }, 0);
        assert_eq!(unsafe { GetParent(strip.0) }, taskbar.0);
        assert_ne!(unsafe { GetClientRect(strip.0, &mut actual) }, 0);
        assert_eq!(actual, Rect::new(0, 0, 160, 48));
    }

    #[test]
    fn only_fullscreen_on_the_taskbar_monitor_hides_the_strip() {
        let primary = Rect::new(0, 0, 1920, 1080);
        let secondary = Rect::new(-1920, 0, 0, 1080);
        assert!(covers_monitor(primary, primary));
        assert!(!covers_monitor(secondary, primary));
        assert!(!covers_monitor(Rect::new(0, 0, 1920, 1032), primary));
        assert!(covers_monitor(Rect::new(-1920, 0, 1920, 1080), primary));
        assert!(!covers_monitor(primary, Rect::default()));
    }

    #[test]
    fn autohidden_taskbar_does_not_leave_floating_text_on_the_desktop() {
        let monitor = Rect::new(-1920, 0, 0, 1080);
        let bar = Rect::new(-1920, 1032, 0, 1080);
        let tray = Rect::new(-220, 1032, 0, 1080);
        assert!(horizontal_taskbar_exposed(bar, tray, monitor));
        let hidden_bar = Rect::new(-1920, 1078, 0, 1126);
        let hidden_tray = Rect::new(-220, 1078, 0, 1126);
        assert!(horizontal_taskbar_supported(hidden_bar, hidden_tray));
        assert!(!horizontal_taskbar_exposed(hidden_bar, hidden_tray, monitor));
        assert!(!horizontal_taskbar_exposed(Rect::new(-1920, 1078, 0, 1126), tray, monitor));
        assert!(!horizontal_taskbar_exposed(Rect::new(-1920, -46, 0, 2), tray, monitor));
        assert!(horizontal_taskbar_exposed(Rect::new(-1920, 0, 0, 48),
            Rect::new(-220, 0, 0, 48), monitor));
        assert!(!horizontal_taskbar_exposed(bar, Rect::default(), monitor));
    }

}
