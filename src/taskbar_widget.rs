//! Optional battery strip directly left of the Windows notification area.
//!
//! Windows has no public notification-area text API. This is our own layered,
//! non-activating top-level window positioned over the taskbar. It hides itself
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
const GWL_EXSTYLE: i32 = -20;
const WS_EX_LAYOUTRTL: u32 = 0x0040_0000;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_LAYERED: u32 = 0x0008_0000;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
const WS_POPUP: u32 = 0x8000_0000;
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
const HWND_TOPMOST: Handle = !0usize as Handle;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_SHOWWINDOW: u32 = 0x0040;
const ULW_ALPHA: u32 = 0x0000_0002;
const WM_DESTROY: u32 = 0x0002;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_MOUSEACTIVATE: u32 = 0x0021;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_CANCELMODE: u32 = 0x001f;
const WM_CAPTURECHANGED: u32 = 0x0215;
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
    fn GetWindow(hwnd: Handle, command: u32) -> Handle;
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
            if let Some(action) = action.filter(|_| !notify.is_null() && message != 0) {
                unsafe { PostMessageW(notify, message, action, 0) };
            }
            0
        }
        WM_CANCELMODE | WM_CAPTURECHANGED | WM_DESTROY => {
            CLICK_GESTURE.with(|cell| {
                let mut gesture = cell.get();
                gesture.cancel(hwnd as usize);
                cell.set(gesture);
            });
            if msg == WM_CANCELMODE && unsafe { GetCapture() } == hwnd { unsafe { ReleaseCapture(); } }
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
        Self {
            hwnd: Cell::new(ptr::null_mut()),
            taskbar: Cell::new(ptr::null_mut()),
            enabled: Cell::new(false),
            light: Cell::new(false),
            content: RefCell::new(None),
            layout: RefCell::new(None),
            rendered: Cell::new(None),
            rendered_size: Cell::new((0, 0)),
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
            self.ensure_window();
            self.paint_and_position();
        }
    }

    pub fn screen_rect(&self) -> Option<Rect> {
        let hwnd = self.hwnd.get();
        if hwnd.is_null() || unsafe { IsWindow(hwnd) } == 0 || unsafe { IsWindowVisible(hwnd) } == 0 {
            return None;
        }
        let mut rect = Rect::default();
        (unsafe { GetWindowRect(hwnd, &mut rect) } != 0).then_some(rect)
    }

    fn ensure_window(&self) {
        if !self.hwnd.get().is_null() && unsafe { IsWindow(self.hwnd.get()) } != 0
            && unsafe { IsWindow(self.taskbar.get()) } != 0 {
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
                return;
            }
        }
        let class_name = crate::win32::to_wide(CLASS_NAME);
        let window_name = crate::win32::to_wide("X-Battery Taskbar Strip");
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                class_name.as_ptr(),
                window_name.as_ptr(),
                WS_POPUP | WS_CLIPSIBLINGS,
                0,
                0,
                1,
                1,
                ptr::null_mut(),
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
            return;
        }
        self.hwnd.set(hwnd);
    }

    fn paint_and_position(&self) {
        let hwnd = self.hwnd.get();
        let taskbar = self.taskbar.get();
        if hwnd.is_null() || taskbar.is_null() {
            return;
        }
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
        let foreground = unsafe { GetForegroundWindow() };
        let mut foreground_rect = Rect::default();
        // Only a full-screen window covering this taskbar's monitor hides it.
        let monitor = unsafe { crate::win32::MonitorFromWindow(taskbar, 2) };
        let mut monitor_info: crate::win32::MonitorInfo = unsafe { core::mem::zeroed() };
        monitor_info.cb_size = core::mem::size_of::<crate::win32::MonitorInfo>() as u32;
        let foreground_fullscreen = !foreground.is_null()
            && foreground != taskbar
            && foreground != hwnd
            && foreground != unsafe { GetShellWindow() }
            && unsafe { IsWindowVisible(foreground) } != 0
            && unsafe { GetWindowRect(foreground, &mut foreground_rect) } != 0
            && !monitor.is_null()
            && unsafe { crate::win32::GetMonitorInfoW(monitor, &mut monitor_info) } != 0
            && covers_monitor(foreground_rect, monitor_info.rc_monitor);
        if tray.is_null()
            || unsafe { GetWindowRect(taskbar, &mut bar) } == 0
            || unsafe { GetWindowRect(tray, &mut tray_rect) } == 0
            || unsafe { IsWindowVisible(taskbar) } == 0
            || unsafe { GetWindowLongW(taskbar, GWL_EXSTYLE) } as u32 & WS_EX_LAYOUTRTL != 0
            || bar.bottom - bar.top > bar.right - bar.left
            || monitor.is_null()
            || unsafe { crate::win32::GetMonitorInfoW(monitor, &mut monitor_info) } == 0
            || !horizontal_taskbar_exposed(bar, tray_rect, monitor_info.rc_monitor)
            || foreground_fullscreen
        {
            unsafe { ShowWindow(hwnd, 0) };
            return;
        }
        let dpi = unsafe { GetDpiForWindow(taskbar) };
        let scale = (if dpi == 0 { 96 } else { dpi }) as f32 / 96.0;
        let content = self.content.borrow().clone();
        let Some(content) = content.filter(|content| !content.items.is_empty()) else {
            unsafe { ShowWindow(hwnd, 0) };
            return;
        };
        let mut layout = self.layout.borrow_mut();
        if !layout.as_ref().is_some_and(|(old, light, old_dpi, _)| {
            old == &content && *light == self.light.get() && *old_dpi == dpi
        }) {
            let Some(measured) = render::Layout::new(&content.items, self.light.get(), scale) else {
                unsafe { ShowWindow(hwnd, 0) };
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
            unsafe { ShowWindow(hwnd, 0) };
            return;
        }
        let height = bar.bottom - bar.top;
        let render_key = (visible, height);
        if self.rendered.get() != Some(render_key) {
            let Some(raster) = measured.raster(visible, height) else { return; };
            let Some(bitmap) = raster.bitmap() else {
                unsafe { ShowWindow(hwnd, 0) };
                return;
            };
            let screen = unsafe { crate::win32::GetDC(ptr::null_mut()) };
            if screen.is_null() {
                unsafe { crate::win32::DeleteObject(bitmap) };
                return;
            }
            let memory = unsafe { crate::win32::CreateCompatibleDC(screen) };
            if memory.is_null() {
                unsafe {
                    crate::win32::ReleaseDC(ptr::null_mut(), screen);
                    crate::win32::DeleteObject(bitmap);
                }
                return;
            }
            let previous = unsafe { crate::win32::SelectObject(memory, bitmap) };
            let size = Size {
                cx: raster.width,
                cy: raster.height,
            };
            let source = Point { x: 0, y: 0 };
            let blend = BlendFunction {
                blend_op: 0,
                blend_flags: 0,
                source_constant_alpha: 255,
                alpha_format: 1,
            };
            let painted = unsafe {
                UpdateLayeredWindow(
                    hwnd,
                    screen,
                    ptr::null(),
                    &size,
                    memory,
                    &source,
                    0,
                    &blend,
                    ULW_ALPHA,
                ) != 0
            };
            unsafe {
                crate::win32::SelectObject(memory, previous);
                crate::win32::DeleteObject(bitmap);
                crate::win32::DeleteDC(memory);
                crate::win32::ReleaseDC(ptr::null_mut(), screen);
            }
            if !painted {
                crate::diagnostics::event_once(
                    "taskbar_widget_paint_failed",
                    &format!("GetLastError={}", crate::win32::last_error()),
                );
                return;
            }
            self.rendered.set(Some(render_key));
            self.rendered_size.set((raster.width, raster.height));
        }
        let (width, height) = self.rendered_size.get();
        let x = tray_rect.left - width - (scale * 2.0).round() as i32;
        let y = bar.top + (bar.bottom - bar.top - height) / 2;
        let desired = Rect::new(x, y, x + width, y + height);
        let mut current = Rect::default();
        let visible = unsafe { IsWindowVisible(hwnd) } != 0;
        if !visible || unsafe { GetWindowRect(hwnd, &mut current) } == 0 || current != desired {
            // A timer tick must not keep raising the strip above its quick-view panel.
            let flags = SWP_NOACTIVATE | SWP_SHOWWINDOW | if visible { SWP_NOZORDER } else { 0 };
            unsafe { SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                x,
                y,
                width,
                height,
                flags,
            ); }
        }
        if taskbar_above_strip(hwnd, taskbar) {
            // Explorer may raise several windows together. Restore the strip just
            // above the taskbar while keeping our popup and other windows above it.
            let previous = unsafe { GetWindow(taskbar, 3) }; // GW_HWNDPREV
            unsafe { SetWindowPos(hwnd, if previous.is_null() { HWND_TOPMOST } else { previous },
                0, 0, 0, 0, SWP_NOACTIVATE | 0x0003); }
        }
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
    }
}

fn taskbar_above_strip(hwnd: Handle, taskbar: Handle) -> bool {
    let mut previous = unsafe { GetWindow(hwnd, 3) };
    for _ in 0..256 {
        if previous.is_null() { break; }
        if previous == taskbar { return true; }
        previous = unsafe { GetWindow(previous, 3) };
    }
    false
}

fn covers_monitor(window: Rect, monitor: Rect) -> bool {
    monitor.right > monitor.left && monitor.bottom > monitor.top
        && window.left <= monitor.left && window.top <= monitor.top
        && window.right >= monitor.right && window.bottom >= monitor.bottom
}

fn horizontal_taskbar_exposed(bar: Rect, tray: Rect, monitor: Rect) -> bool {
    bar.right > bar.left && bar.bottom > bar.top
        && bar.top >= monitor.top && bar.bottom <= monitor.bottom
        && tray.right > tray.left && tray.bottom > tray.top
        && tray.left >= bar.left && tray.right <= bar.right
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
        assert!(!horizontal_taskbar_exposed(Rect::new(-1920, 1078, 0, 1126), tray, monitor));
        assert!(!horizontal_taskbar_exposed(Rect::new(-1920, -46, 0, 2), tray, monitor));
        assert!(horizontal_taskbar_exposed(Rect::new(-1920, 0, 0, 48),
            Rect::new(-220, 0, 0, 48), monitor));
        assert!(!horizontal_taskbar_exposed(bar, Rect::default(), monitor));
    }

}
