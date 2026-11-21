//! Optional battery strip directly left of the Windows notification area.
//!
//! Windows has no public notification-area text API. This is our own layered,
//! non-activating top-level window positioned over the taskbar. It hides itself
//! if the taskbar is missing, vertical, RTL or covered by a full-screen app.

use crate::win32::{Handle, Rect};
use core::ffi::c_void;
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
const SWP_SHOWWINDOW: u32 = 0x0040;
const ULW_ALPHA: u32 = 0x0000_0002;
const WM_DESTROY: u32 = 0x0002;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_MOUSEACTIVATE: u32 = 0x0021;
const WM_LBUTTONUP: u32 = 0x0202;
const MA_NOACTIVATE: isize = 3;

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
    rendered: RefCell<Option<(Option<StripContent>, bool, u32, usize)>>,
}

unsafe extern "system" fn window_proc(hwnd: Handle, msg: u32, wp: usize, lp: isize) -> isize {
    match msg {
        WM_LBUTTONUP => {
            let notify = NOTIFY_WINDOW.load(Ordering::Relaxed) as Handle;
            let message = CLICK_MESSAGE.load(Ordering::Relaxed);
            if !notify.is_null() && message != 0 {
                unsafe { PostMessageW(notify, message, 0, 0) };
            }
            0
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE,
        WM_ERASEBKGND => 1,
        WM_DESTROY => 0,
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
            rendered: RefCell::new(None),
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
        if hwnd.is_null() || unsafe { IsWindow(hwnd) } == 0 {
            return None;
        }
        let mut rect = Rect::default();
        (unsafe { GetWindowRect(hwnd, &mut rect) } != 0).then_some(rect)
    }

    fn ensure_window(&self) {
        if !self.hwnd.get().is_null() && unsafe { IsWindow(self.hwnd.get()) } != 0 {
            return;
        }
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
        let window_name = crate::win32::to_wide("R Battery Taskbar Strip");
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                class_name.as_ptr(),
                window_name.as_ptr(),
                WS_POPUP | WS_CLIPSIBLINGS,
                0,
                0,
                crate::tray_icon::TASKBAR_STRIP_MIN_WIDTH,
                crate::tray_icon::TASKBAR_STRIP_HEIGHT,
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
        let monitor = unsafe { crate::win32::MonitorFromWindow(foreground, 2) };
        let mut monitor_info: crate::win32::MonitorInfo = unsafe { core::mem::zeroed() };
        monitor_info.cb_size = core::mem::size_of::<crate::win32::MonitorInfo>() as u32;
        let foreground_fullscreen = !foreground.is_null()
            && foreground != taskbar
            && foreground != hwnd
            && unsafe { GetWindowRect(foreground, &mut foreground_rect) } != 0
            && !monitor.is_null()
            && unsafe { crate::win32::GetMonitorInfoW(monitor, &mut monitor_info) } != 0
            && foreground_rect.left <= monitor_info.rc_monitor.left
            && foreground_rect.top <= monitor_info.rc_monitor.top
            && foreground_rect.right >= monitor_info.rc_monitor.right
            && foreground_rect.bottom >= monitor_info.rc_monitor.bottom;
        if tray.is_null()
            || unsafe { GetWindowRect(taskbar, &mut bar) } == 0
            || unsafe { GetWindowRect(tray, &mut tray_rect) } == 0
            || unsafe { IsWindowVisible(taskbar) } == 0
            || unsafe { GetWindowLongW(taskbar, GWL_EXSTYLE) } as u32 & WS_EX_LAYOUTRTL != 0
            || bar.bottom - bar.top > bar.right - bar.left
            || foreground_fullscreen
        {
            unsafe { ShowWindow(hwnd, 0) };
            return;
        }
        let dpi = unsafe { GetDpiForWindow(taskbar) };
        let scale = (if dpi == 0 { 96 } else { dpi }) as f32 / 96.0;
        let content = self.content.borrow().clone();
        let available_width =
            (tray_rect.left - bar.left - (scale * 2.0).round() as i32).max(0);
        let visible = content.as_ref().map_or(0, |content| {
            visible_item_count(&content.items, available_width, scale)
        });
        if visible == 0 {
            unsafe { ShowWindow(hwnd, 0) };
            return;
        }
        let render_key = (content.clone(), self.light.get(), dpi, visible);
        if self.rendered.borrow().as_ref() != Some(&render_key) {
            let items = content
                .as_ref()
                .map(|content| content.items[..visible].to_vec())
                .unwrap_or_default();
            let Some(bitmap) = crate::tray_icon::render_taskbar_strip(
                &items,
                self.light.get(),
                scale,
            ) else {
                unsafe { ShowWindow(hwnd, 0) };
                return;
            };
            let width = (crate::tray_icon::taskbar_strip_width(&items) as f32 * scale).round() as i32;
            let height = (crate::tray_icon::TASKBAR_STRIP_HEIGHT as f32 * scale).round() as i32;
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
                cx: width,
                cy: height,
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
            *self.rendered.borrow_mut() = Some(render_key);
        }
        let items = content
            .as_ref()
            .map(|content| &content.items[..visible])
            .unwrap_or_default();
        let width = (crate::tray_icon::taskbar_strip_width(items) as f32 * scale).round() as i32;
        let height = (crate::tray_icon::TASKBAR_STRIP_HEIGHT as f32 * scale).round() as i32;
        let x = tray_rect.left - width - (scale * 2.0).round() as i32;
        let y = bar.top + (bar.bottom - bar.top - height) / 2;
        unsafe {
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }

    fn destroy(&self) {
        let hwnd = self.hwnd.replace(ptr::null_mut());
        if !hwnd.is_null() && unsafe { IsWindow(hwnd) } != 0 {
            unsafe { DestroyWindow(hwnd) };
        }
        self.taskbar.set(ptr::null_mut());
        *self.rendered.borrow_mut() = None;
    }
}

fn visible_item_count(
    items: &[StripItem],
    available_width: i32,
    scale: f32,
) -> usize {
    (1..=items.len())
        .rev()
        .find(|count| {
            (crate::tray_icon::taskbar_strip_width(&items[..*count]) as f32 * scale).round()
                as i32
                <= available_width
        })
        .unwrap_or(0)
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

    fn item(value: u8) -> StripItem {
        StripItem {
            level: Some(crate::device::BatteryPercent {
                value,
                approximate: false,
            }),
            charging: false,
            kind: crate::tray_prefs::TrayKind::Mouse,
        }
    }

    #[test]
    fn only_whole_items_are_shown_in_available_space() {
        let items = [item(75), item(100), item(50), item(5), item(88)];
        assert_eq!(crate::tray_icon::taskbar_strip_width(&[]), 0);
        let one = crate::tray_icon::taskbar_strip_width(&items[..1]);
        let two = crate::tray_icon::taskbar_strip_width(&items[..2]);
        let five = crate::tray_icon::taskbar_strip_width(&items);
        assert!(one >= 32 && one < two && two < five);
        assert_eq!(visible_item_count(&items, one - 1, 1.0), 0);
        assert_eq!(visible_item_count(&items, one, 1.0), 1);
        assert_eq!(visible_item_count(&items, two, 1.0), 2);
        assert_eq!(visible_item_count(&items, five, 1.0), 5);
        assert_eq!(
            visible_item_count(&items, (two as f32 * 1.5).round() as i32 - 1, 1.5),
            1
        );
        assert_eq!(visible_item_count(&[], 400, 1.0), 0);
    }
}
