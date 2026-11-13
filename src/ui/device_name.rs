//! 自绘改名弹窗；仅文字编辑交给系统处理输入法和剪贴板。
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use native_windows_gui as nwg;

pub struct Request {
    pub key: String,
    pub name: String,
}

struct Controls {
    window: nwg::Window,
    input: nwg::TextInput,
    error: RefCell<String>,
    save: nwg::Button,
    reset: nwg::Button,
    cancel: nwg::Button,
    close: nwg::Button,
    _font: nwg::Font,
    key: RefCell<Option<String>>,
    requests: RefCell<Vec<Request>>,
    pending: Cell<bool>,
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
        let input_color = colorref(theme.bg_sunken);
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

fn draw_background(p: &mut super::gfx::Painter, focused: bool, error: &str) {
    use super::gfx::{Align, R};
    let t = p.t;
    p.rect(R::new(0.0, 0.0, 480.0, 266.0), t.bg);
    p.round_stroke(R::new(0.5, 0.5, 479.0, 265.0), 13.0, t.border, 1.0);
    p.text(
        crate::tr!("编辑名称"),
        R::new(24.0, 18.0, 390.0, 29.0),
        20.0,
        true,
        t.text,
        Align::Left,
    );
    p.text(
        crate::tr!("设备名称"),
        R::new(24.0, 60.0, 432.0, 22.0),
        12.5,
        false,
        t.text2,
        Align::Left,
    );
    let input = R::new(24.0, 88.0, 432.0, 46.0);
    p.round(input, 8.0, t.bg_sunken);
    p.round_stroke(
        input,
        8.0,
        if focused { t.accent } else { t.border_strong },
        1.0,
    );
    p.text_wrap(
        crate::tr!("留空恢复原始名称，最多 64 个字符"),
        R::new(24.0, 144.0, 432.0, 34.0),
        11.5,
        t.text2,
    );
    p.text_wrap(error, R::new(24.0, 181.0, 432.0, 28.0), 11.5, t.red);
}

#[link(name = "gdi32")]
extern "system" {
    fn SetTextColor(dc: crate::win32::Handle, color: u32) -> u32;
    fn SetBkColor(dc: crate::win32::Handle, color: u32) -> u32;
}
#[link(name = "user32")]
extern "system" {
    fn GetClientRect(hwnd: crate::win32::Handle, rect: *mut crate::win32::Rect) -> i32;
    fn BeginPaint(
        hwnd: crate::win32::Handle,
        paint: *mut crate::win32::PaintStruct,
    ) -> crate::win32::Handle;
    fn EndPaint(hwnd: crate::win32::Handle, paint: *const crate::win32::PaintStruct) -> i32;
    fn InvalidateRect(
        hwnd: crate::win32::Handle,
        rect: *const crate::win32::Rect,
        erase: i32,
    ) -> i32;
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
            .flags(nwg::WindowFlags::POPUP)
            .ex_flags(0x80)
            .parent(Some(owner))
            .size((px(480.0), px(266.0)))
            .build(&mut window)
            .map_err(|e| e.to_string())?;
        super::frame::keep_client_size(window.handle.hwnd().unwrap() as _, 480.0, 266.0, scale);
        let mut font = nwg::Font::default();
        nwg::Font::builder()
            .family("Microsoft YaHei UI")
            .size_absolute(px(13.0) as u32)
            .build(&mut font)
            .map_err(|e| e.to_string())?;
        let mut input = nwg::TextInput::default();
        // 创建时即无边框，避免系统缓存旧的非客户区样式。
        input.handle = nwg::ControlBase::build_hwnd()
            .class_name("EDIT")
            .parent(Some(window.handle))
            .flags(0x5001_0080)
            .position((px(37.0), px(99.0)))
            .size((px(406.0), px(24.0)))
            .build()
            .map_err(|e| e.to_string())?;
        input.set_font(Some(&font));
        input.set_limit(128);
        let input_hwnd = input.handle.hwnd().unwrap() as _;
        let mut save = nwg::Button::default();
        let mut reset = nwg::Button::default();
        let mut cancel = nwg::Button::default();
        let mut close = nwg::Button::default();
        for (button, x, width) in [
            (&mut reset, 24.0, 116.0),
            (&mut cancel, 244.0, 90.0),
            (&mut save, 344.0, 112.0),
        ] {
            button.handle = nwg::ControlBase::build_hwnd()
                .class_name("BUTTON")
                .parent(Some(window.handle))
                .flags(0x5001_000b)
                .position((px(x), px(212.0)))
                .size((px(width), px(32.0)))
                .build()
                .map_err(|e| e.to_string())?;
            button.set_font(Some(&font));
        }
        close.handle = nwg::ControlBase::build_hwnd()
            .class_name("BUTTON")
            .parent(Some(window.handle))
            .flags(0x5001_000b)
            .position((px(428.0), px(18.0)))
            .size((px(28.0), px(28.0)))
            .build()
            .map_err(|e| e.to_string())?;
        close.set_font(Some(&font));
        let controls = Rc::new(Controls {
            window,
            input,
            error: RefCell::new(String::new()),
            save,
            reset,
            cancel,
            close,
            _font: font,
            key: RefCell::new(None),
            requests: RefCell::new(Vec::new()),
            pending: Cell::new(false),
        });
        let parent = controls.window.handle.hwnd().unwrap() as crate::win32::Handle;
        let input_handler = nwg::bind_raw_event_handler(
            &controls.input.handle,
            0x10131,
            move |hwnd, msg, wp, _lp| {
                if msg == 0x0007 || msg == 0x0008 {
                    unsafe {
                        InvalidateRect(parent, std::ptr::null(), 0);
                    }
                }
                if msg == 0x0102 && wp == 1 {
                    unsafe {
                        crate::win32::SendMessageW(hwnd as _, 0x00b1, 0, -1);
                    }
                    Some(0)
                } else {
                    None
                }
            },
        )
        .map_err(|e| e.to_string())?;
        super::canvas::set_window_pos(input_hwnd, 0, 0, 0, 0, 0x37);
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
                        c.requests.borrow_mut().clear();
                        c.pending.set(false);
                    }
                    E::OnButtonClick if handle == c.cancel.handle || handle == c.close.handle => {
                        c.window.set_visible(false);
                        *c.key.borrow_mut() = None;
                        c.requests.borrow_mut().clear();
                        c.pending.set(false);
                    }
                    E::OnKeyEnter | E::OnButtonClick
                        if event == E::OnKeyEnter
                            || handle == c.save.handle
                            || handle == c.reset.handle =>
                    {
                        if !c.pending.replace(true) {
                            let Some(key) = c.key.borrow().clone() else {
                                c.pending.set(false);
                                return;
                            };
                            let name = if handle == c.reset.handle {
                                String::new()
                            } else {
                                c.input.text()
                            };
                            c.requests.borrow_mut().push(Request { key, name });
                            let sender = notice;
                            if let Err(error) = std::thread::Builder::new()
                                .name("device-name-save".to_string())
                                .spawn(move || sender.notice())
                            {
                                c.pending.set(false);
                                *c.error.borrow_mut() = error.to_string();
                                c.window.invalidate();
                            }
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
                    0x000f | 0x0318 => unsafe {
                        let mut paint = std::mem::zeroed();
                        let dc = if msg == 0x000f {
                            BeginPaint(hwnd as _, &mut paint)
                        } else {
                            wp as _
                        };
                        let mut rect = crate::win32::Rect::default();
                        GetClientRect(hwnd as _, &mut rect);
                        let mut buffer = buffer.borrow_mut();
                        let mut fonts = fonts.borrow_mut();
                        let mem = buffer.ensure(dc as isize, rect.right, rect.bottom);
                        crate::win32::FillRect(mem as _, &rect, colors.bg);
                        buffer.paint(
                            &mut fonts,
                            &colors.theme,
                            scale,
                            0.0,
                            rect.right as f32 / scale,
                            rect.bottom as f32 / scale,
                            |p| {
                                draw_background(p, c.input.focus(), &c.error.borrow());
                            },
                        );
                        super::gfx::blit(dc as isize, mem, rect.right, rect.bottom);
                        if msg == 0x000f {
                            EndPaint(hwnd as _, &paint);
                        }
                        Some(0)
                    },
                    0x002b => unsafe {
                        let mut buffer = buffer.borrow_mut();
                        let mut fonts = fonts.borrow_mut();
                        let item = &*(lp as *const DrawItem);
                        let close = item.hwnd == c.close.handle.hwnd().unwrap() as _;
                        let (text, primary) = if item.hwnd == c.save.handle.hwnd().unwrap() as _ {
                            (c.save.text(), true)
                        } else if item.hwnd == c.reset.handle.hwnd().unwrap() as _ {
                            (c.reset.text(), false)
                        } else {
                            (c.cancel.text(), false)
                        };
                        let w = item.rect.right - item.rect.left;
                        let h = item.rect.bottom - item.rect.top;
                        let mem = buffer.ensure(item.dc as isize, w, h);
                        let clear = crate::win32::Rect {
                            left: 0,
                            top: 0,
                            right: w,
                            bottom: h,
                        };
                        crate::win32::FillRect(mem as _, &clear, colors.bg);
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
                                if close {
                                    if item.state & 0x11 != 0 {
                                        p.round(r.inset(1.0), 6.0, p.t.card_hover);
                                    }
                                    let (x, y) = (r.cx(), r.cy());
                                    p.polyline(
                                        &[(x - 4.0, y - 4.0), (x + 4.0, y + 4.0)],
                                        p.t.text2,
                                        1.3,
                                    );
                                    p.polyline(
                                        &[(x + 4.0, y - 4.0), (x - 4.0, y + 4.0)],
                                        p.t.text2,
                                        1.3,
                                    );
                                } else {
                                    let r = r.inset(1.0);
                                    p.round(
                                        r,
                                        8.0,
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
                                        8.0,
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
                                }
                            },
                        );
                        super::gfx::blit(item.dc as isize, mem, w, h);
                        Some(1)
                    },
                    0x0014 => Some(1),
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
        c.pending.set(false);
        c.window.set_text(crate::tr!("编辑名称"));
        c.error.borrow_mut().clear();
        c.save.set_text(crate::tr!("保存名称"));
        c.reset.set_text(crate::tr!("恢复原名"));
        c.cancel.set_text(crate::tr!("取消"));
        c.close.set_text(crate::tr!("取消"));
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
        let theme = crate::win32::to_wide("");
        for handle in [
            &c.input.handle,
            &c.save.handle,
            &c.reset.handle,
            &c.cancel.handle,
            &c.close.handle,
        ] {
            unsafe {
                SetWindowTheme(handle.hwnd().unwrap() as _, theme.as_ptr(), theme.as_ptr());
            }
            super::canvas::set_window_pos(handle.hwnd().unwrap() as _, 0, 0, 0, 0, 0x37);
        }
        c.window.set_visible(true);
        c.window.set_focus();
        c.input.set_focus();
        c.input.set_selection(0..u32::MAX);
        c.window.invalidate();
    }

    pub fn take_requests(&self) -> Vec<Request> {
        std::mem::take(&mut *self.controls.requests.borrow_mut())
    }

    pub fn finish(&self, error: Option<&str>) {
        self.controls.pending.set(false);
        if let Some(error) = error {
            *self.controls.error.borrow_mut() = error.to_string();
            self.controls.window.invalidate();
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
