//! 托盘设备面板。窗口只绘制快照和派发动作，设备读取留在后台。
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use super::panel_icons::{self, Icon};
use super::{
    gfx::{self, Align, Canvas, Fonts, Painter, R},
    theme,
};
use crate::{
    device::DeviceReading,
    settings::Settings,
    win32::{self, Handle, Rect},
};
use native_windows_gui as nwg;

const WIDTH: f32 = 410.0;
const HEADER: f32 = 57.0;
const ROW: f32 = 67.0;
const FOOTER: f32 = 42.0;
const SPIN_TIMER: usize = 71;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Refresh,
    Settings,
    Main,
    Quit,
    Device(String),
}

struct State {
    readings: Vec<DeviceReading>,
    cfg: Settings,
    canvas: Canvas,
    fonts: Fonts,
    scale: f32,
    width: f32,
    height: f32,
    offset: usize,
    hits: Vec<(R, Action)>,
    hover: Option<usize>,
    pressed: Option<Action>,
    focus: Option<usize>,
    acts: Vec<Action>,
    refreshing: Option<(u64, Instant)>,
    updated: bool,
    visible: bool,
    dismissed: Option<Instant>,
    anchor: Rect,
}

pub struct Popup {
    window: nwg::Window,
    handler: Option<nwg::RawEventHandler>,
    state: Rc<RefCell<State>>,
}

#[repr(C)]
struct TrackMouse {
    size: u32,
    flags: u32,
    hwnd: Handle,
    hover: u32,
}
#[link(name = "user32")]
extern "system" {
    fn BeginPaint(hwnd: Handle, paint: *mut win32::PaintStruct) -> Handle;
    fn EndPaint(hwnd: Handle, paint: *const win32::PaintStruct) -> i32;
    fn InvalidateRect(hwnd: Handle, rect: *const Rect, erase: i32) -> i32;
    fn SetFocus(hwnd: Handle) -> Handle;
    fn SetCapture(hwnd: Handle) -> Handle;
    fn ReleaseCapture() -> i32;
    fn TrackMouseEvent(event: *mut TrackMouse) -> i32;
    fn MonitorFromRect(rect: *const Rect, flags: u32) -> Handle;
    fn GetClientRect(hwnd: Handle, out: *mut Rect) -> i32;
}

impl Popup {
    pub fn new(owner: &nwg::Window, notice: nwg::NoticeSender) -> Result<Self, String> {
        let mut window = nwg::Window::default();
        nwg::Window::builder()
            .title("R Battery · Devices")
            .flags(nwg::WindowFlags::POPUP)
            .ex_flags(0x80)
            .parent(Some(owner))
            .size((410, 367))
            .build(&mut window)
            .map_err(|e| e.to_string())?;
        let hwnd = window.handle.hwnd().unwrap() as Handle;
        crate::dwm::set_round_corners(hwnd);
        let fonts = Fonts::from_candidates(&["Microsoft YaHei UI", "Segoe UI", "Arial"])
            .ok_or_else(|| crate::tr!("字体初始化失败").to_string())?;
        let state = Rc::new(RefCell::new(State {
            readings: Vec::new(),
            cfg: Settings::default(),
            canvas: Canvas::new(),
            fonts,
            scale: 1.0,
            width: WIDTH,
            height: HEADER + ROW * 4.0 + FOOTER,
            offset: 0,
            hits: Vec::new(),
            hover: None,
            pressed: None,
            focus: None,
            acts: Vec::new(),
            refreshing: None,
            updated: false,
            visible: false,
            dismissed: None,
            anchor: Rect::default(),
        }));
        let s = Rc::clone(&state);
        let handler = unsafe {
            nwg::bind_raw_event_handler(&window.handle, 0x10120, move |h, msg, wp, lp| {
                let hwnd = h as Handle;
                match msg {
                    0x000f => {
                        paint(hwnd, &mut s.borrow_mut());
                        Some(0)
                    }
                    0x0014 => Some(1),
                    0x0010 => {
                        hide(hwnd, &s, false);
                        Some(0)
                    }
                    0x0006 if wp & 0xffff == 0 => {
                        hide(hwnd, &s, true);
                        None
                    }
                    0x0200 => {
                        let mut state = s.borrow_mut();
                        let (x, y) = point(lp, state.scale);
                        let hit = state.hits.iter().position(|(r, _)| r.contains(x, y));
                        if state.hover != hit {
                            state.hover = hit;
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                        }
                        let mut track = TrackMouse {
                            size: std::mem::size_of::<TrackMouse>() as u32,
                            flags: 2,
                            hwnd,
                            hover: 0,
                        };
                        TrackMouseEvent(&mut track);
                        Some(0)
                    }
                    0x02a3 => {
                        s.borrow_mut().hover = None;
                        InvalidateRect(hwnd, std::ptr::null(), 0);
                        Some(0)
                    }
                    0x0201 => {
                        {
                            let mut state = s.borrow_mut();
                            let (x, y) = point(lp, state.scale);
                            state.pressed = state
                                .hits
                                .iter()
                                .find(|(r, _)| r.contains(x, y))
                                .map(|(_, a)| a.clone());
                        }
                        SetFocus(hwnd);
                        SetCapture(hwnd);
                        Some(0)
                    }
                    0x0202 => {
                        let action = {
                            let mut state = s.borrow_mut();
                            let (x, y) = point(lp, state.scale);
                            let hit = state
                                .hits
                                .iter()
                                .find(|(r, _)| r.contains(x, y))
                                .map(|(_, a)| a.clone());
                            let pressed = state.pressed.take();
                            hit.filter(|a| Some(a) == pressed.as_ref())
                        };
                        ReleaseCapture();
                        if let Some(action) = action {
                            s.borrow_mut().acts.push(action);
                            notice.notice();
                        }
                        Some(0)
                    }
                    0x0215 => {
                        s.borrow_mut().pressed = None;
                        None
                    }
                    0x020a => {
                        let delta = ((wp >> 16) as u16) as i16;
                        let mut state = s.borrow_mut();
                        let max = state.readings.len().saturating_sub(state.visible_rows());
                        state.offset = if delta > 0 {
                            state.offset.saturating_sub(1)
                        } else {
                            (state.offset + 1).min(max)
                        };
                        state.hover = None;
                        state.focus = None;
                        state.pressed = None;
                        InvalidateRect(hwnd, std::ptr::null(), 0);
                        Some(0)
                    }
                    0x0100 if wp == 0x1b => {
                        hide(hwnd, &s, false);
                        Some(0)
                    }
                    0x0100 if wp == 9 || wp == 0x28 || wp == 0x26 => {
                        let mut state = s.borrow_mut();
                        let n = state.hits.len();
                        if n > 0 {
                            state.focus = Some(match state.focus {
                                None => 0,
                                Some(i) if wp == 0x26 => (i + n - 1) % n,
                                Some(i) => (i + 1) % n,
                            });
                        }
                        InvalidateRect(hwnd, std::ptr::null(), 0);
                        Some(0)
                    }
                    0x0100 if wp == 13 || wp == 32 => {
                        let mut state = s.borrow_mut();
                        if let Some(action) = state
                            .focus
                            .and_then(|i| state.hits.get(i))
                            .map(|(_, a)| a.clone())
                        {
                            state.acts.push(action);
                            notice.notice();
                        }
                        Some(0)
                    }
                    0x0087 => Some(4),
                    0x0113 if wp == SPIN_TIMER => {
                        InvalidateRect(hwnd, std::ptr::null(), 0);
                        Some(0)
                    }
                    0x02e0 => {
                        // 接受系统在跨屏时建议的 DPI 和位置。
                        s.borrow_mut().scale = (wp & 0xffff) as f32 / 96.0;
                        let r = &*(lp as *const Rect);
                        super::canvas::SetWindowPos(
                            hwnd,
                            std::ptr::null_mut(),
                            r.left,
                            r.top,
                            r.right - r.left,
                            r.bottom - r.top,
                            0x14,
                        );
                        Some(0)
                    }
                    _ => None,
                }
            })
        }
        .map_err(|e| e.to_string())?;
        Ok(Self {
            window,
            handler: Some(handler),
            state,
        })
    }
    fn hwnd(&self) -> Handle {
        self.window.handle.hwnd().unwrap() as Handle
    }
    pub fn update(&self, readings: &[DeviceReading], cfg: &Settings, generation: u64) {
        let mut state = self.state.borrow_mut();
        let resize = state.visible && state.readings.len() != readings.len();
        let anchor = state.anchor;
        if state
            .readings
            .iter()
            .map(|r| &r.key)
            .ne(readings.iter().map(|r| &r.key))
        {
            state.pressed = None;
            state.focus = None;
        }
        state.readings = readings.to_vec();
        state.cfg = cfg.clone();
        let max = state.readings.len().saturating_sub(state.visible_rows());
        state.offset = state.offset.min(max);
        if state
            .refreshing
            .is_some_and(|(start, _)| generation > start)
        {
            state.refreshing = None;
            state.updated = true;
            unsafe {
                win32::KillTimer(self.hwnd(), SPIN_TIMER);
            }
        }
        if state.visible {
            unsafe {
                InvalidateRect(self.hwnd(), std::ptr::null(), 0);
            }
        }
        drop(state);
        if resize {
            self.show(anchor);
        }
    }
    pub fn refresh_started(&self, generation: u64) {
        let mut s = self.state.borrow_mut();
        s.refreshing = Some((generation, Instant::now()));
        unsafe {
            win32::SetTimer(self.hwnd(), SPIN_TIMER, 80, 0);
            InvalidateRect(self.hwnd(), std::ptr::null(), 0);
        }
    }
    pub fn take_actions(&self) -> Vec<Action> {
        std::mem::take(&mut self.state.borrow_mut().acts)
    }
    pub fn hide(&self) {
        hide(self.hwnd(), &self.state, false)
    }
    pub fn was_open(&self) -> bool {
        let s = self.state.borrow();
        s.visible || s.dismissed.is_some_and(|t| t.elapsed().as_millis() < 250)
    }
    pub fn show(&self, anchor: Rect) {
        let monitor = unsafe { MonitorFromRect(&anchor, 2) };
        let mut info: win32::MonitorInfo = unsafe { std::mem::zeroed() };
        info.cb_size = std::mem::size_of::<win32::MonitorInfo>() as u32;
        if unsafe { win32::GetMonitorInfoW(monitor, &mut info) } == 0 {
            return;
        }
        let scale = monitor_scale(monitor);
        let (width, height, dark) = {
            let mut s = self.state.borrow_mut();
            s.scale = scale;
            s.width = WIDTH.min((info.rc_work.right - info.rc_work.left) as f32 / scale - 16.0);
            let rows =
                ((info.rc_work.bottom - info.rc_work.top) as f32 / scale - HEADER - FOOTER - 24.0)
                    .div_euclid(ROW)
                    .clamp(1.0, 6.0) as usize;
            s.height = HEADER + FOOTER + ROW * s.readings.len().max(1).min(rows) as f32;
            s.visible = true;
            s.dismissed = None;
            s.hover = None;
            s.focus = None;
            s.pressed = None;
            s.anchor = anchor;
            s.offset = s.offset.min(s.readings.len().saturating_sub(rows));
            (
                (s.width * scale).round() as i32,
                (s.height * scale).round() as i32,
                theme::resolve(&s.cfg.theme) == theme::Mode::Dark,
            )
        };
        let (x, y) = place(anchor, info.rc_work, width, height, (8.0 * scale) as i32);
        crate::dwm::set_dark_titlebar(self.hwnd(), dark);
        unsafe {
            super::canvas::SetWindowPos(self.hwnd(), -1isize as Handle, x, y, width, height, 0x10);
            InvalidateRect(self.hwnd(), std::ptr::null(), 0);
            win32::ShowWindow(self.hwnd(), 5);
            win32::SetForegroundWindow(self.hwnd());
            SetFocus(self.hwnd());
            if self.state.borrow().refreshing.is_some() {
                win32::SetTimer(self.hwnd(), SPIN_TIMER, 80, 0);
            }
        }
    }
}
impl Drop for Popup {
    fn drop(&mut self) {
        if let Some(h) = self.handler.take() {
            let _ = nwg::unbind_raw_event_handler(&h);
        }
        self.state.borrow_mut().canvas.release();
    }
}
impl State {
    fn visible_rows(&self) -> usize {
        ((self.height - HEADER - FOOTER) / ROW).round().max(1.0) as usize
    }
}
fn point(lp: isize, scale: f32) -> (f32, f32) {
    (
        (lp as u16 as i16) as f32 / scale,
        ((lp as u32 >> 16) as u16 as i16) as f32 / scale,
    )
}
fn hide(hwnd: Handle, state: &Rc<RefCell<State>>, deactivated: bool) {
    {
        let mut s = state.borrow_mut();
        if !s.visible {
            return;
        }
        s.visible = false;
        s.pressed = None;
        s.hover = None;
        s.dismissed = deactivated.then(Instant::now);
        s.canvas.release();
        s.fonts.clear();
    }
    unsafe {
        win32::KillTimer(hwnd, SPIN_TIMER);
        win32::ShowWindow(hwnd, 0);
    }
}

fn monitor_scale(monitor: Handle) -> f32 {
    unsafe {
        let dll = win32::LoadLibraryW(win32::to_wide("shcore.dll").as_ptr());
        if !dll.is_null() {
            let proc = win32::GetProcAddress(dll, b"GetDpiForMonitor\0".as_ptr());
            if !proc.is_null() {
                let f: unsafe extern "system" fn(Handle, i32, *mut u32, *mut u32) -> i32 =
                    std::mem::transmute(proc);
                let (mut x, mut y) = (96, 96);
                let result = f(monitor, 0, &mut x, &mut y);
                win32::FreeLibrary(dll);
                if result == 0 {
                    return (x as f32 / 96.0).clamp(0.5, 6.0);
                }
            } else {
                win32::FreeLibrary(dll);
            }
        }
    }
    crate::dwm::system_scale()
}

fn place(anchor: Rect, work: Rect, width: i32, height: i32, gap: i32) -> (i32, i32) {
    let x = (anchor.left + (anchor.right - anchor.left - width) / 2).clamp(
        work.left + gap,
        (work.right - width - gap).max(work.left + gap),
    );
    let above = anchor.top - height - gap;
    let y = if above >= work.top + gap {
        above
    } else {
        anchor.bottom + gap
    };
    (
        x,
        y.clamp(
            work.top + gap,
            (work.bottom - height - gap).max(work.top + gap),
        ),
    )
}

unsafe fn paint(hwnd: Handle, state: &mut State) {
    let mut ps: win32::PaintStruct = std::mem::zeroed();
    let dc = BeginPaint(hwnd, &mut ps);
    let mut rect = Rect::default();
    GetClientRect(hwnd, &mut rect);
    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
    state.width = w as f32 / state.scale;
    state.height = h as f32 / state.scale;
    let mem = state.canvas.ensure(dc as isize, w, h);
    let t = theme::resolve(&state.cfg.theme).theme();
    state.hits.clear();
    let State {
        canvas,
        fonts,
        readings,
        cfg,
        width,
        height,
        offset,
        hits,
        hover,
        focus,
        refreshing,
        updated,
        scale,
        ..
    } = state;
    canvas.paint(fonts, t, *scale, 0.0, *width, *height, |p| {
        draw(
            p,
            *width,
            *height,
            readings,
            cfg,
            *offset,
            hits,
            hover.or(*focus),
            refreshing.map(|(_, t)| t.elapsed().as_secs_f32()),
            *updated,
        )
    });
    gfx::blit(dc as isize, mem, w, h);
    EndPaint(hwnd, &ps);
}

fn draw(
    p: &mut Painter,
    w: f32,
    h: f32,
    readings: &[DeviceReading],
    cfg: &Settings,
    offset: usize,
    hits: &mut Vec<(R, Action)>,
    hover: Option<usize>,
    refreshing: Option<f32>,
    updated: bool,
) {
    let t = *p.t;
    p.rect(R::new(0.0, 0.0, w, h), t.bg);
    p.round_stroke(R::new(0.5, 0.5, w - 1.0, h - 1.0), 11.0, t.border, 1.0);
    p.image_rounded(gfx::app_icon_bitmap(), R::new(17.0, 16.0, 25.0, 25.0), 5.0);
    p.text(
        "R Battery",
        R::new(51.0, 13.0, 200.0, 32.0),
        14.0,
        true,
        t.text,
        Align::Left,
    );
    for (i, action) in [Action::Refresh, Action::Settings].into_iter().enumerate() {
        let r = R::new(w - 82.0 + i as f32 * 36.0, 13.0, 32.0, 32.0);
        if hover == Some(hits.len()) {
            p.round(r, 6.0, t.card_hover);
        }
        panel_icons::draw(
            p,
            if i == 0 {
                Icon::Refresh
            } else {
                Icon::Settings
            },
            r.inset(8.0),
            t.text2,
            if i == 0 {
                refreshing.unwrap_or(0.0) * 360.0
            } else {
                0.0
            },
        );
        hits.push((r, action));
    }
    let list = R::new(11.0, HEADER, w - 22.0, h - HEADER - FOOTER);
    p.card(list, 10.0, t.card, t.border);
    if readings.is_empty() {
        p.text(
            crate::tr!("等待设备连接"),
            list.inset(8.0),
            13.0,
            false,
            t.text2,
            Align::Center,
        );
    }
    let count = (list.h / ROW).round() as usize;
    for (index, d) in readings.iter().skip(offset).take(count).enumerate() {
        let r = R::new(
            list.x + 1.0,
            list.y + index as f32 * ROW + 1.0,
            list.w - 2.0,
            ROW - 2.0,
        );
        if hover == Some(hits.len()) {
            p.round(r, 8.0, t.card_hover);
        }
        if index > 0 {
            p.hline(list.x + 1.0, list.right() - 1.0, r.y - 1.0, t.border);
        }
        let ink = if d.connected { t.text } else { t.text3 };
        let kind = cfg.device_kind(d);
        panel_icons::draw(
            p,
            Icon::Device(kind),
            R::new(27.0, r.cy() - 12.5, 25.0, 25.0),
            if d.connected { t.text2 } else { t.text3 },
            0.0,
        );
        p.text(
            cfg.device_name(d),
            R::new(65.0, r.y + 10.0, w - 180.0, 21.0),
            13.0,
            true,
            ink,
            Align::Left,
        );
        let status = if !d.connected {
            crate::tr!("未连接")
        } else if d.charging {
            crate::tr!("正在充电")
        } else if d.level <= 20 {
            crate::tr!("电量偏低")
        } else {
            crate::tr!("已连接")
        };
        let mut meta = format!("{} · {status}", kind.label());
        if cfg.tray_device.as_ref() == Some(&d.key) {
            meta.push_str(" · ");
            meta.push_str(crate::tr!("托盘显示"));
        }
        p.text(
            &meta,
            R::new(65.0, r.y + 33.0, w - 175.0, 17.0),
            10.0,
            false,
            if d.connected && d.level <= 20 && !d.charging {
                t.red
            } else {
                t.text3
            },
            Align::Left,
        );
        let color = if !d.connected {
            t.text3
        } else if d.level <= 20 {
            t.red
        } else {
            t.text
        };
        let text = if d.connected {
            d.level.to_string()
        } else {
            "—".into()
        };
        p.text(
            &text,
            R::new(w - 94.0, r.y, 49.0, r.h),
            20.0,
            true,
            color,
            Align::Right,
        );
        if d.connected {
            p.text(
                "%",
                R::new(w - 45.0, r.y + 4.0, 13.0, r.h - 4.0),
                10.0,
                false,
                t.text3,
                Align::Left,
            );
        }
        if d.connected && d.charging {
            panel_icons::draw(
                p,
                Icon::Bolt,
                R::new(w - 109.0, r.cy() - 6.5, 12.0, 13.0),
                t.accent,
                0.0,
            );
        }
        panel_icons::draw(
            p,
            Icon::Chevron,
            R::new(w - 30.0, r.cy() - 6.0, 12.0, 12.0),
            t.text3,
            0.0,
        );
        hits.push((r, Action::Device(d.key.clone())));
    }
    if readings.len() > count && count > 0 {
        let thumb = list.h * count as f32 / readings.len() as f32;
        let y = list.y + (list.h - thumb) * offset as f32 / (readings.len() - count) as f32;
        p.round(
            R::new(list.right() - 4.0, y + 2.0, 2.0, (thumb - 4.0).max(5.0)),
            1.0,
            t.border_strong,
        );
    }
    let footer_y = h - FOOTER;
    let label = if refreshing.is_some() {
        crate::tr!("正在刷新…").to_string()
    } else if updated {
        crate::tr!("已更新").to_string()
    } else {
        crate::trf!("{} 台设备", readings.len())
    };
    p.text(
        &label,
        R::new(17.0, footer_y, 130.0, FOOTER),
        10.0,
        false,
        t.text3,
        Align::Left,
    );
    for (r, label, action) in [
        (
            R::new(w - 175.0, footer_y + 5.0, 120.0, 32.0),
            crate::tr!("打开主窗口"),
            Action::Main,
        ),
        (
            R::new(w - 51.0, footer_y + 5.0, 40.0, 32.0),
            crate::tr!("退出"),
            Action::Quit,
        ),
    ] {
        let color = if hover == Some(hits.len()) {
            t.text
        } else {
            t.text2
        };
        p.text(label, r, 11.0, false, color, Align::Center);
        hits.push((r, action));
    }
}
