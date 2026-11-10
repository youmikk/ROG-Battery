//! 使用原生编辑框，保留中文输入法、选择和剪贴板操作。
use std::cell::RefCell;
use std::rc::Rc;

use native_windows_gui as nwg;

pub struct Request {
    pub key: String,
    pub name: String,
}

struct Controls {
    window: nwg::Window,
    input: nwg::TextInput,
    hint: nwg::Label,
    error: nwg::Label,
    save: nwg::Button,
    reset: nwg::Button,
    cancel: nwg::Button,
    _font: nwg::Font,
    key: RefCell<Option<String>>,
    requests: RefCell<Vec<Request>>,
}

pub struct Dialog {
    controls: Rc<Controls>,
    handler: Option<nwg::EventHandler>,
    colors_handler: Option<nwg::RawEventHandler>,
    input_handler: Option<nwg::RawEventHandler>,
    colors: Rc<RefCell<Colors>>,
}

struct Colors {
    bg: crate::win32::Handle,
    input: crate::win32::Handle,
    bg_color: u32,
    input_color: u32,
    text: u32,
    theme: super::theme::Theme,
}

fn colorref(argb: u32) -> u32 {
    (argb & 0xff) << 16 | (argb & 0xff00) | (argb >> 16 & 0xff)
}

impl Colors {
    fn new(dark: bool) -> Self {
        let theme = if dark {
            super::theme::DARK
        } else {
            super::theme::LIGHT
        };
        let bg_color = colorref(theme.bg);
        let input_color = colorref(theme.card);
        Self {
            bg: unsafe { crate::win32::CreateSolidBrush(bg_color) },
            input: unsafe { crate::win32::CreateSolidBrush(input_color) },
            bg_color,
            input_color,
            text: colorref(theme.text),
            theme,
        }
    }
}

impl Drop for Colors {
    fn drop(&mut self) {
        unsafe {
            crate::win32::DeleteObject(self.bg);
            crate::win32::DeleteObject(self.input);
        }
    }
}

#[link(name = "gdi32")]
extern "system" {
    fn SetTextColor(dc: crate::win32::Handle, color: u32) -> u32;
    fn SetBkColor(dc: crate::win32::Handle, color: u32) -> u32;
}
#[link(name = "user32")]
extern "system" {
    fn GetClientRect(hwnd: crate::win32::Handle, rect: *mut crate::win32::Rect) -> i32;
}

#[repr(C)]
struct DrawItem {
    kind: u32,
    id: u32,
    item: u32,
    action: u32,
    state: u32,
    hwnd: crate::win32::Handle,
    dc: crate::win32::Handle,
    rect: crate::win32::Rect,
    data: usize,
}
#[link(name = "uxtheme")]
extern "system" {
    fn SetWindowTheme(hwnd: crate::win32::Handle, app: *const u16, ids: *const u16) -> i32;
}

impl Dialog {
    pub fn new(owner: &nwg::Window, notice: nwg::NoticeSender) -> Result<Self, String> {
        let scale = crate::dwm::system_scale();
        let px = |v: f32| (v * scale).round() as i32;
        let mut window = nwg::Window::default();
        nwg::Window::builder()
            .title(crate::tr!("编辑名称"))
            .flags(nwg::WindowFlags::WINDOW)
            .ex_flags(0x80)
            .parent(Some(owner))
            .size((px(470.0), px(185.0)))
            .build(&mut window)
            .map_err(|e| e.to_string())?;
        super::frame::keep_client_size(window.handle.hwnd().unwrap() as _, 470.0, 185.0, scale);
        let mut font = nwg::Font::default();
        nwg::Font::builder()
            .family("Microsoft YaHei UI")
            .size_absolute(px(13.0) as u32)
            .build(&mut font)
            .map_err(|e| e.to_string())?;
        let mut input = nwg::TextInput::default();
        nwg::TextInput::builder()
            .parent(&window)
            .position((px(20.0), px(20.0)))
            .size((px(430.0), px(32.0)))
            .font(Some(&font))
            .limit(128)
            .build(&mut input)
            .map_err(|e| e.to_string())?;
        let mut hint = nwg::Label::default();
        nwg::Label::builder()
            .parent(&window)
            .position((px(20.0), px(62.0)))
            .size((px(430.0), px(24.0)))
            .font(Some(&font))
            .build(&mut hint)
            .map_err(|e| e.to_string())?;
        let mut error = nwg::Label::default();
        nwg::Label::builder()
            .parent(&window)
            .position((px(20.0), px(88.0)))
            .size((px(430.0), px(40.0)))
            .font(Some(&font))
            .build(&mut error)
            .map_err(|e| e.to_string())?;
        let mut save = nwg::Button::default();
        let mut reset = nwg::Button::default();
        let mut cancel = nwg::Button::default();
        for (button, x, width) in [
            (&mut reset, 20.0, 115.0),
            (&mut cancel, 240.0, 90.0),
            (&mut save, 340.0, 110.0),
        ] {
            nwg::Button::builder()
                .parent(&window)
                .position((px(x), px(137.0)))
                .size((px(width), px(30.0)))
                .font(Some(&font))
                .build(button)
                .map_err(|e| e.to_string())?;
        }
        let controls = Rc::new(Controls {
            window,
            input,
            hint,
            error,
            save,
            reset,
            cancel,
            _font: font,
            key: RefCell::new(None),
            requests: RefCell::new(Vec::new()),
        });
        for handle in [&controls.hint.handle, &controls.error.handle] {
            let hwnd = handle.hwnd().unwrap() as _;
            let style = super::canvas::get_window_long(hwnd, -16);
            super::canvas::set_window_long(hwnd, -16, (style & !0x1f) | 0x0d);
        }
        for handle in [
            &controls.save.handle,
            &controls.reset.handle,
            &controls.cancel.handle,
        ] {
            let hwnd = handle.hwnd().unwrap() as _;
            let style = super::canvas::get_window_long(hwnd, -16);
            super::canvas::set_window_long(hwnd, -16, (style & !0x0f) | 0x0b);
        }
        let input_handler =
            nwg::bind_raw_event_handler(&controls.input.handle, 0x10131, |hwnd, msg, wp, _lp| {
                if msg == 0x0102 && wp == 1 {
                    unsafe {
                        crate::win32::SendMessageW(hwnd as _, 0x00b1, 0, -1);
                    }
                    Some(0)
                } else {
                    None
                }
            })
            .map_err(|e| e.to_string())?;
        let c = Rc::clone(&controls);
        let handler =
            nwg::full_bind_event_handler(&controls.window.handle, move |event, data, handle| {
                use nwg::Event as E;
                match event {
                    E::OnWindowClose | E::OnKeyEsc => {
                        if let nwg::EventData::OnWindowClose(close) = data {
                            close.close(false);
                        }
                        c.window.set_visible(false);
                        *c.key.borrow_mut() = None;
                    }
                    E::OnButtonClick if handle == c.cancel.handle => {
                        c.window.set_visible(false);
                        *c.key.borrow_mut() = None;
                    }
                    E::OnKeyEnter | E::OnButtonClick
                        if event == E::OnKeyEnter
                            || handle == c.save.handle
                            || handle == c.reset.handle =>
                    {
                        if let Some(key) = c.key.borrow().clone() {
                            let name = if handle == c.reset.handle {
                                String::new()
                            } else {
                                c.input.text()
                            };
                            c.requests.borrow_mut().push(Request { key, name });
                            notice.notice();
                        }
                    }
                    _ => {}
                }
            });
        let colors = Rc::new(RefCell::new(Colors::new(false)));
        let paint_colors = Rc::clone(&colors);
        let c = Rc::clone(&controls);
        let buffer = RefCell::new(super::gfx::Canvas::new());
        let fonts = RefCell::new(
            super::gfx::Fonts::new("Microsoft YaHei UI")
                .ok_or_else(|| crate::tr!("字体初始化失败").to_string())?,
        );
        let colors_handler = nwg::bind_raw_event_handler(
            &controls.window.handle,
            0x10130,
            move |hwnd, msg, wp, lp| {
                let colors = paint_colors.borrow();
                match msg {
                    0x002b => unsafe {
                        let mut buffer = buffer.borrow_mut();
                        let mut fonts = fonts.borrow_mut();
                        let item = &*(lp as *const DrawItem);
                        let button = item.kind == 4;
                        let (text, primary, error) =
                            if item.hwnd == c.save.handle.hwnd().unwrap() as _ {
                                (c.save.text(), true, false)
                            } else if item.hwnd == c.reset.handle.hwnd().unwrap() as _ {
                                (c.reset.text(), false, false)
                            } else if item.hwnd == c.cancel.handle.hwnd().unwrap() as _ {
                                (c.cancel.text(), false, false)
                            } else if item.hwnd == c.error.handle.hwnd().unwrap() as _ {
                                (c.error.text(), false, true)
                            } else {
                                (c.hint.text(), false, false)
                            };
                        let w = item.rect.right - item.rect.left;
                        let h = item.rect.bottom - item.rect.top;
                        let mem = buffer.ensure(item.dc as isize, w, h);
                        buffer.paint(
                            &mut fonts,
                            &colors.theme,
                            scale,
                            0.0,
                            w as f32 / scale,
                            h as f32 / scale,
                            |p| {
                                use super::gfx::{Align, R};
                                let r = R::new(0.0, 0.0, w as f32 / scale, h as f32 / scale);
                                p.rect(r, p.t.bg);
                                if button {
                                    let r = r.inset(1.0);
                                    p.round(
                                        r,
                                        6.0,
                                        if primary {
                                            p.t.accent
                                        } else if item.state & 1 != 0 {
                                            p.t.card_hover
                                        } else {
                                            p.t.card
                                        },
                                    );
                                    p.round_stroke(
                                        r,
                                        6.0,
                                        if primary || item.state & 0x10 != 0 {
                                            p.t.accent
                                        } else {
                                            p.t.border_strong
                                        },
                                        1.0,
                                    );
                                    p.text(
                                        &text,
                                        r,
                                        12.5,
                                        false,
                                        if primary { p.t.accent_fg } else { p.t.text },
                                        Align::Center,
                                    );
                                } else {
                                    p.text_wrap(
                                        &text,
                                        r,
                                        11.5,
                                        if error { p.t.red } else { p.t.text2 },
                                    );
                                }
                            },
                        );
                        super::gfx::blit(item.dc as isize, mem, w, h);
                        Some(1)
                    },
                    0x0014 => unsafe {
                        let mut rect = crate::win32::Rect::default();
                        GetClientRect(hwnd as _, &mut rect);
                        crate::win32::FillRect(wp as _, &rect, colors.bg);
                        Some(1)
                    },
                    0x0133 | 0x0138 => unsafe {
                        let input = msg == 0x0133;
                        SetTextColor(wp as _, colors.text);
                        SetBkColor(
                            wp as _,
                            if input {
                                colors.input_color
                            } else {
                                colors.bg_color
                            },
                        );
                        Some(if input { colors.input } else { colors.bg } as isize)
                    },
                    _ => None,
                }
            },
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            controls,
            handler: Some(handler),
            colors_handler: Some(colors_handler),
            input_handler: Some(input_handler),
            colors,
        })
    }

    pub fn show(&self, owner: &nwg::Window, key: &str, name: &str, dark: bool) {
        let c = &self.controls;
        *self.colors.borrow_mut() = Colors::new(dark);
        *c.key.borrow_mut() = Some(key.to_string());
        c.requests.borrow_mut().clear();
        c.window.set_text(crate::tr!("编辑名称"));
        c.hint
            .set_text(crate::tr!("留空恢复原始名称，最多 64 个字符"));
        c.error.set_text("");
        c.save.set_text(crate::tr!("保存名称"));
        c.reset.set_text(crate::tr!("恢复原名"));
        c.cancel.set_text(crate::tr!("取消"));
        c.input.set_text(name);
        let (x, y) = owner.position();
        let (w, h) = owner.size();
        let (dw, dh) = c.window.size();
        c.window.set_position(
            x + (w as i32 - dw as i32) / 2,
            y + (h as i32 - dh as i32) / 2,
        );
        let hwnd = c.window.handle.hwnd().unwrap() as crate::win32::Handle;
        crate::dwm::set_dark_titlebar(hwnd, dark);
        crate::dwm::set_round_corners(hwnd);
        let theme = crate::win32::to_wide(if dark {
            "DarkMode_Explorer"
        } else {
            "Explorer"
        });
        for handle in [
            &c.input.handle,
            &c.save.handle,
            &c.reset.handle,
            &c.cancel.handle,
        ] {
            unsafe {
                SetWindowTheme(
                    handle.hwnd().unwrap() as _,
                    theme.as_ptr(),
                    std::ptr::null(),
                );
            }
        }
        c.window.set_visible(true);
        c.window.set_focus();
        c.input.set_focus();
        c.input.set_selection(0..u32::MAX);
    }

    pub fn take_requests(&self) -> Vec<Request> {
        std::mem::take(&mut *self.controls.requests.borrow_mut())
    }

    pub fn finish(&self, error: Option<&str>) {
        if let Some(error) = error {
            self.controls.error.set_text(error);
            self.controls.input.set_focus();
        } else {
            self.controls.window.set_visible(false);
            *self.controls.key.borrow_mut() = None;
        }
    }
}

impl Drop for Dialog {
    fn drop(&mut self) {
        if let Some(handler) = self.input_handler.take() {
            let _ = nwg::unbind_raw_event_handler(&handler);
        }
        if let Some(handler) = self.colors_handler.take() {
            let _ = nwg::unbind_raw_event_handler(&handler);
        }
        if let Some(handler) = self.handler.take() {
            nwg::unbind_event_handler(&handler);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[link(name = "user32")]
    extern "system" {
        fn PrintWindow(hwnd: crate::win32::Handle, dc: crate::win32::Handle, flags: u32) -> i32;
    }

    #[test]
    fn native_name_editor_keeps_unicode_and_target_until_save_or_cancel() {
        nwg::init().unwrap();
        let mut owner = nwg::Window::default();
        nwg::Window::builder()
            .flags(nwg::WindowFlags::empty())
            .build(&mut owner)
            .unwrap();
        let mut notice = nwg::Notice::default();
        nwg::Notice::builder()
            .parent(&owner)
            .build(&mut notice)
            .unwrap();
        let dialog = Dialog::new(&owner, notice.sender()).unwrap();
        let c = &dialog.controls;
        dialog.show(&owner, "device-a", "原始名称", false);
        let text = crate::win32::to_wide("办公鼠标 🖱");
        unsafe {
            crate::win32::SendMessageW(
                c.input.handle.hwnd().unwrap() as _,
                0x00c2,
                1,
                text.as_ptr() as isize,
            );
        }
        assert_eq!(c.input.text(), "办公鼠标 🖱");
        c.save.click();
        let requests = dialog.take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].key, "device-a");
        assert_eq!(requests[0].name, "办公鼠标 🖱");
        dialog.finish(Some("保存失败"));
        assert!(c.window.visible());
        assert_eq!(c.input.text(), "办公鼠标 🖱");
        assert_eq!(c.error.text(), "保存失败");
        dialog.finish(None);
        assert!(!c.window.visible());
        dialog.show(&owner, "device-b", "键盘", false);
        unsafe {
            crate::win32::SendMessageW(c.input.handle.hwnd().unwrap() as _, 0x0102, 1, 0);
        }
        assert_eq!(c.input.selection(), 0..2);
        unsafe {
            crate::win32::SendMessageW(c.window.handle.hwnd().unwrap() as _, 0x0111, 1, 0);
        }
        let requests = dialog.take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].key, "device-b");
        assert_eq!(requests[0].name, "键盘");
        c.cancel.click();
        assert!(dialog.take_requests().is_empty());
        assert!(!c.window.visible());
        dialog.show(&owner, "device-a", "办公鼠标", true);
        c.reset.click();
        let requests = dialog.take_requests();
        assert_eq!(requests[0].key, "device-a");
        assert_eq!(requests[0].name, "");
        unsafe {
            crate::win32::SendMessageW(c.window.handle.hwnd().unwrap() as _, 0x0111, 2, 0);
        }
        assert!(!c.window.visible());
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scratch");
        std::fs::create_dir_all(&out).unwrap();
        for dark in [false, true] {
            dialog.show(&owner, "device-a", "办公鼠标 🖱", dark);
            let hwnd = c.window.handle.hwnd().unwrap() as crate::win32::Handle;
            let mut rect = crate::win32::Rect::default();
            unsafe {
                GetClientRect(hwnd, &mut rect);
            }
            let (x, y) = c.save.position();
            let (w, h) = c.save.size();
            assert!(x + w as i32 <= rect.right && y + h as i32 <= rect.bottom);
            let dc = unsafe { crate::win32::GetDC(hwnd) };
            let mut canvas = super::super::gfx::Canvas::new();
            let target = canvas.ensure(dc as isize, rect.right, rect.bottom);
            assert_ne!(unsafe { PrintWindow(hwnd, target as _, 3) }, 0);
            super::super::gfx::save_bmp(
                out.join(format!(
                    "device-name-{}.bmp",
                    if dark { "dark" } else { "light" }
                ))
                .to_str()
                .unwrap(),
                target,
                rect.right,
                rect.bottom,
            );
            canvas.release();
            unsafe {
                crate::win32::ReleaseDC(hwnd, dc);
            }
        }
        dialog.finish(None);
    }
}
