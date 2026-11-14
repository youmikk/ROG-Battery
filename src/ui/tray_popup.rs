//! 托盘设备面板。窗口只绘制快照和派发动作，设备读取留在后台。
use std::cell::RefCell;
use std::collections::BTreeMap;
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
const ACCORDION_TIMER: usize = 72;
const DRAG_ANIM_TIMER: usize = 73;
const AUDIO_TIMER: usize = 74;
const ACCORDION_FRAME_MS: u32 = 10;
const DRAG_START_DISTANCE: f32 = 4.0;
const DRAG_LIFT_MS: f32 = 120.0;
const DRAG_REFLOW_MS: f32 = 150.0;
const DPI_PANEL_HEIGHT: f32 = 84.0;
const AUDIO_PANEL_HEIGHT: f32 = 152.0;
const ACCORDION_COLLAPSE_MS: f32 = 190.0;
const ACCORDION_EXPAND_MS: f32 = 210.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Refresh,
    Settings,
    Main,
    Quit,
    ReorderDevices(Vec<String>),
    TogglePanel(String),
    SetDpiStage(String, u8),
    AudioSlot(AudioSlot),
    AudioSlider(AudioSlot),
    ToggleMicrophone,
    OpenSoundSettings,
}

pub use crate::audio::Slot as AudioSlot;

#[derive(Clone, Debug, Default)]
pub struct DpiState {
    pub stages: [i32; 4],
    pub active: u8,
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
    mouse_down: Option<PressState>,
    drag: Option<DragState>,
    focus: Option<usize>,
    acts: Vec<Action>,
    refreshing: Option<(u64, Instant)>,
    updated: bool,
    visible: bool,
    dismissed: Option<Instant>,
    anchor: Rect,
    work_area: Rect,
    last_window_rect: Option<(i32, i32, i32, i32)>,
    expanded: Option<String>,
    panel_progress: BTreeMap<String, f32>,
    animation: Option<AccordionAnimation>,
    dpi: BTreeMap<String, DpiState>,
    audio: crate::audio::Snapshot,
    audio_controls: crate::audio::Controls,
    audio_slot: AudioSlot,
    audio_error: String,
    audio_slider: R,
}

#[derive(Clone)]
struct PressState {
    action: Action,
    start: (f32, f32),
}

#[derive(Clone)]
struct DragState {
    key: String,
    source: usize,
    grab_offset_y: f32,
    pointer_y: f32,
    target: usize,
    previous_target: usize,
    started_at: Instant,
    target_changed_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PanelKind {
    Dpi,
    Audio,
}

impl PanelKind {
    fn full_height(self) -> f32 {
        match self {
            Self::Dpi => DPI_PANEL_HEIGHT,
            Self::Audio => AUDIO_PANEL_HEIGHT,
        }
    }
}

struct AccordionStep {
    key: String,
    from: f32,
    to: f32,
    start_ms: f32,
    end_ms: f32,
}

struct AccordionAnimation {
    steps: Vec<AccordionStep>,
    total_ms: f32,
    target: Option<String>,
    started_at: Instant,
    _timer_resolution: AnimationTimerResolution,
}

struct AnimationTimerResolution(bool);

impl AnimationTimerResolution {
    fn new() -> Self {
        Self(unsafe { timeBeginPeriod(1) == 0 })
    }
}

impl Drop for AnimationTimerResolution {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                timeEndPeriod(1);
            }
        }
    }
}

#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
    fn timeEndPeriod(period: u32) -> u32;
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
    fn UpdateWindow(hwnd: Handle) -> i32;
    fn SetFocus(hwnd: Handle) -> Handle;
    fn SetCapture(hwnd: Handle) -> Handle;
    fn ReleaseCapture() -> i32;
    fn TrackMouseEvent(event: *mut TrackMouse) -> i32;
    fn MonitorFromRect(rect: *const Rect, flags: u32) -> Handle;
    fn GetClientRect(hwnd: Handle, out: *mut Rect) -> i32;
}

impl Popup {
    pub fn new(notice: nwg::NoticeSender, audio_controls: crate::audio::Controls) -> Result<Self, String> {
        let mut window = nwg::Window::default();
        nwg::Window::builder()
            .title("R Battery · Devices")
            .flags(nwg::WindowFlags::POPUP)
            .ex_flags(0x80)
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
            mouse_down: None,
            drag: None,
            focus: None,
            acts: Vec::new(),
            refreshing: None,
            updated: false,
            visible: false,
            dismissed: None,
            anchor: Rect::default(),
            work_area: Rect::default(),
            last_window_rect: None,
            expanded: None,
            panel_progress: BTreeMap::new(),
            animation: None,
            dpi: BTreeMap::new(),
            audio: crate::audio::Snapshot::default(),
            audio_controls,
            audio_slot: AudioSlot::Speaker,
            audio_error: String::new(),
            audio_slider: R::default(),
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
                        if state.drag.is_some() {
                            let moved = move_dragged_device(&mut state, y);
                            state.hover = None;
                            drop(state);
                            if moved {
                                win32::SetTimer(hwnd, DRAG_ANIM_TIMER, 16, 0);
                                InvalidateRect(hwnd, std::ptr::null(), 0);
                            }
                            let mut track = TrackMouse {
                                size: std::mem::size_of::<TrackMouse>() as u32,
                                flags: 2,
                                hwnd,
                                hover: 0,
                            };
                            TrackMouseEvent(&mut track);
                            return Some(0);
                        }
                        let moved = state.mouse_down.as_ref().map(|down| {
                            ((x - down.start.0).powi(2) + (y - down.start.1).powi(2)).sqrt()
                        });
                        if moved.is_some_and(|distance| distance > DRAG_START_DISTANCE) {
                            let can_drag = state.mouse_down.as_ref().is_some_and(|down| {
                                !matches!(&down.action, Action::AudioSlider(_))
                                    && device_key_for_action(&down.action).is_some()
                            });
                            if can_drag && begin_device_drag(&mut state, y) {
                                move_dragged_device(&mut state, y);
                                win32::KillTimer(hwnd, ACCORDION_TIMER);
                                position_window(hwnd, &mut state, false);
                                win32::SetTimer(hwnd, DRAG_ANIM_TIMER, 16, 0);
                                drop(state);
                                InvalidateRect(hwnd, std::ptr::null(), 0);
                                return Some(0);
                            }
                            if !matches!(
                                state.mouse_down.as_ref().map(|down| &down.action),
                                Some(Action::AudioSlider(_))
                            ) {
                                state.pressed = None;
                                state.mouse_down = None;
                            }
                        }
                        let hit = state.hits.iter().position(|(r, _)| r.contains(x, y));
                        let slider = match state.pressed.as_ref() {
                            Some(Action::AudioSlider(slot)) if state.audio_slider.w > 0.0 => {
                                Some((
                                    *slot,
                                    ((x - state.audio_slider.x) / state.audio_slider.w.max(1.0))
                                        .clamp(0.0, 1.0),
                                ))
                            }
                            _ => None,
                        };
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
                        drop(state);
                        if let Some((slot, fraction)) = slider {
                            apply_local_action(hwnd, &s, Action::AudioSlider(slot), Some(fraction));
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                        }
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
                            let action = state
                                .hits
                                .iter()
                                .find(|(r, _)| r.contains(x, y))
                                .map(|(_, a)| a.clone());
                            state.pressed = action.clone();
                            state.mouse_down = action.map(|action| PressState {
                                action,
                                start: (x, y),
                            });
                        }
                        SetFocus(hwnd);
                        SetCapture(hwnd);
                        Some(0)
                    }
                    0x0202 => {
                        let (order, target) = {
                            let mut state = s.borrow_mut();
                            let (x, y) = point(lp, state.scale);
                            let hit = state
                                .hits
                                .iter()
                                .find(|(r, _)| r.contains(x, y))
                                .map(|(r, a)| (*r, a.clone()));
                            let moved = state.mouse_down.take().is_some_and(|down| {
                                ((x - down.start.0).powi(2) + (y - down.start.1).powi(2)).sqrt()
                                    > DRAG_START_DISTANCE
                            });
                            let order = state.drag.take().and_then(|drag| {
                                let source = state
                                    .readings
                                    .iter()
                                    .position(|reading| reading.key == drag.key)?;
                                if source != drag.target {
                                    let item = state.readings.remove(source);
                                    let target = drag.target.min(state.readings.len());
                                    state.readings.insert(target, item);
                                    Some(
                                        state
                                            .readings
                                            .iter()
                                            .map(|reading| reading.key.clone())
                                            .collect::<Vec<_>>(),
                                    )
                                } else {
                                    None
                                }
                            });
                            let pressed = state.pressed.take();
                            state.hover = None;
                            state.focus = None;
                            let target = if order.is_none() && !moved {
                                hit.filter(|(_, a)| Some(a) == pressed.as_ref())
                            } else {
                                None
                            };
                            (order, target.map(|(r, a)| (r, a, x)))
                        };
                        win32::KillTimer(hwnd, DRAG_ANIM_TIMER);
                        ReleaseCapture();
                        if let Some(order) = order {
                            {
                                let mut state = s.borrow_mut();
                                state.acts.push(Action::ReorderDevices(order));
                            }
                            notice.notice();
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                            return Some(0);
                        }
                        if let Some((rect, action, x)) = target {
                            let fraction = if let Action::AudioSlider(_) = &action {
                                let state = s.borrow();
                                (x - state.audio_slider.x) / state.audio_slider.w.max(1.0)
                            } else {
                                (x - rect.x) / rect.w.max(1.0)
                            };
                            let local = apply_local_action(hwnd, &s, action, Some(fraction));
                            if local {
                                if s.borrow().animation.is_some() {
                                    win32::SetTimer(hwnd, ACCORDION_TIMER, ACCORDION_FRAME_MS, 0);
                                }
                                InvalidateRect(hwnd, std::ptr::null(), 0);
                            } else {
                                notice.notice();
                            }
                        } else {
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                        }
                        Some(0)
                    }
                    0x0215 => {
                        let mut state = s.borrow_mut();
                        state.pressed = None;
                        state.mouse_down = None;
                        state.drag = None;
                        win32::KillTimer(hwnd, DRAG_ANIM_TIMER);
                        None
                    }
                    0x020a => {
                        let delta = ((wp >> 16) as u16) as i16;
                        let mut state = s.borrow_mut();
                        if state.animation.is_some() {
                            return Some(0);
                        }
                        if state.drag.is_some() {
                            return Some(0);
                        }
                        // 设备行高度会随内嵌面板变化。滚轮切换设备时先收起，
                        // 避免已展开的面板继续占据窗口高度却已滚出可视区。
                        let collapsed = state.expanded.take().is_some();
                        if collapsed {
                            state.panel_progress.clear();
                        }
                        let visible = state.visible_device_count();
                        let max = state.readings.len().saturating_sub(visible);
                        state.offset = if delta > 0 {
                            state.offset.saturating_sub(1)
                        } else {
                            (state.offset + 1).min(max)
                        };
                        state.hover = None;
                        state.focus = None;
                        state.pressed = None;
                        state.mouse_down = None;
                        win32::KillTimer(hwnd, DRAG_ANIM_TIMER);
                        InvalidateRect(hwnd, std::ptr::null(), 0);
                        if collapsed {
                            position_window(hwnd, &mut state, false);
                        }
                        Some(0)
                    }
                    0x0100 if wp == 0x1b => {
                        let collapsing = {
                            let mut state = s.borrow_mut();
                            state
                                .expanded
                                .clone()
                                .is_some_and(|key| start_accordion(&mut state, key))
                        };
                        if collapsing {
                            win32::SetTimer(hwnd, ACCORDION_TIMER, ACCORDION_FRAME_MS, 0);
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                        } else {
                            hide(hwnd, &s, false);
                        }
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
                        let action = {
                            let state = s.borrow();
                            state
                                .focus
                                .and_then(|i| state.hits.get(i))
                                .map(|(_, action)| action.clone())
                        };
                        if let Some(action) = action {
                            if apply_local_action(hwnd, &s, action, None) {
                                if s.borrow().animation.is_some() {
                                    win32::SetTimer(hwnd, ACCORDION_TIMER, ACCORDION_FRAME_MS, 0);
                                }
                                InvalidateRect(hwnd, std::ptr::null(), 0);
                            } else {
                                notice.notice();
                            }
                        }
                        Some(0)
                    }
                    0x0100 if wp == 0x25 || wp == 0x27 => {
                        let action = {
                            let state = s.borrow();
                            state
                                .focus
                                .and_then(|i| state.hits.get(i))
                                .map(|(_, action)| action.clone())
                        };
                        if let Some(Action::AudioSlider(slot)) = action {
                            apply_audio_delta(&s, slot, if wp == 0x25 { -5 } else { 5 });
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                        }
                        Some(0)
                    }
                    0x0100 if wp == 0x4d => {
                        let action = {
                            let state = s.borrow();
                            state
                                .focus
                                .and_then(|i| state.hits.get(i))
                                .map(|(_, action)| action.clone())
                        };
                        if let Some(Action::AudioSlider(slot)) = action {
                            if let Err(error) = toggle_audio_mute(&s, slot) {
                                s.borrow_mut().audio_error = error;
                            }
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                        }
                        Some(0)
                    }
                    0x0087 => Some(4),
                    0x0113 if wp == DRAG_ANIM_TIMER => {
                        let state = s.borrow();
                        let active = state.drag.as_ref().is_some_and(|drag| {
                            drag.started_at.elapsed().as_secs_f32() * 1000.0 < DRAG_LIFT_MS
                                || drag.target_changed_at.elapsed().as_secs_f32() * 1000.0
                                    < DRAG_REFLOW_MS
                        });
                        if active {
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                        } else {
                            win32::KillTimer(hwnd, DRAG_ANIM_TIMER);
                        }
                        Some(0)
                    }
                    0x0113 if wp == SPIN_TIMER => {
                        InvalidateRect(hwnd, std::ptr::null(), 0);
                        Some(0)
                    }
                    0x0113 if wp == AUDIO_TIMER => {
                        let mut state = s.borrow_mut();
                        let audio_open = state.visible && state.expanded.as_ref()
                            .is_some_and(|key| state.panel_kind(key) == Some(PanelKind::Audio));
                        state.audio_controls.set_active(2, audio_open);
                        if audio_open {
                            let snapshot = state.audio_controls.snapshot();
                            if state.audio != snapshot {
                                state.audio_error = snapshot.error.clone().unwrap_or_default();
                                state.audio = snapshot;
                                if endpoint(&state, state.audio_slot).is_none() {
                                    state.audio_slot = if state.audio.headset.is_some() {
                                        AudioSlot::Headset
                                    } else {
                                        AudioSlot::Speaker
                                    };
                                }
                                InvalidateRect(hwnd, std::ptr::null(), 0);
                            }
                        } else if state.animation.is_none() {
                            win32::KillTimer(hwnd, AUDIO_TIMER);
                        }
                        Some(0)
                    }
                    0x0113 if wp == ACCORDION_TIMER => {
                        let finished = {
                            let mut state = s.borrow_mut();
                            let finished = advance_accordion(&mut state);
                            position_window(hwnd, &mut state, false);
                            finished
                        };
                        UpdateWindow(hwnd);
                        if finished {
                            win32::KillTimer(hwnd, ACCORDION_TIMER);
                        }
                        Some(0)
                    }
                    0x02e0 => {
                        // 接受系统在跨屏时建议的 DPI 和位置。
                        {
                            let mut state = s.borrow_mut();
                            state.scale = (wp & 0xffff) as f32 / 96.0;
                            state.work_area = Rect::default();
                            state.last_window_rect = None;
                        }
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
    pub fn update(
        &self,
        readings: &[DeviceReading],
        cfg: &Settings,
        dpi: &BTreeMap<String, DpiState>,
        generation: u64,
    ) {
        let mut state = self.state.borrow_mut();
        let mut layout_changed = state.readings.len() != readings.len();
        let anchor = state.anchor;
        let drag_order = state.drag.as_ref().map(|_| {
            state
                .readings
                .iter()
                .map(|reading| reading.key.clone())
                .collect::<Vec<_>>()
        });
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
        if let Some(order) = drag_order {
            state.readings.sort_by_key(|reading| {
                order
                    .iter()
                    .position(|key| key == &reading.key)
                    .unwrap_or(usize::MAX)
            });
        }
        if state
            .drag
            .as_ref()
            .is_some_and(|drag| !state.readings.iter().any(|reading| reading.key == drag.key))
        {
            state.drag = None;
            state.mouse_down = None;
            unsafe {
                win32::KillTimer(self.hwnd(), DRAG_ANIM_TIMER);
            }
        }
        state.cfg = cfg.clone();
        state.dpi = dpi.clone();
        if state.expanded.as_ref().is_some_and(|key| {
            !state.readings.iter().any(|reading| &reading.key == key)
                || state.panel_kind(key).is_none()
        }) {
            state.expanded = None;
            state.panel_progress.clear();
            state.animation = None;
            layout_changed = true;
            unsafe {
                win32::KillTimer(self.hwnd(), ACCORDION_TIMER);
            }
        }
        let max = state
            .readings
            .len()
            .saturating_sub(state.visible_device_count());
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
        if layout_changed && self.state.borrow().visible {
            let mut state = self.state.borrow_mut();
            state.anchor = anchor;
            position_window(self.hwnd(), &mut state, false);
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
        let mut state = self.state.borrow_mut();
        state.visible = true;
        state.dismissed = None;
        state.hover = None;
        state.focus = None;
        state.pressed = None;
        state.mouse_down = None;
        state.drag = None;
        state.anchor = anchor;
        state.last_window_rect = None;
        position_window(self.hwnd(), &mut state, true);
    }
}

fn position_window(hwnd: Handle, state: &mut State, reveal: bool) {
    if reveal || state.work_area.right <= state.work_area.left {
        let monitor = unsafe { MonitorFromRect(&state.anchor, 2) };
        if monitor.is_null() {
            return;
        }
        let mut info: win32::MonitorInfo = unsafe { std::mem::zeroed() };
        info.cb_size = std::mem::size_of::<win32::MonitorInfo>() as u32;
        if unsafe { win32::GetMonitorInfoW(monitor, &mut info) } == 0 {
            return;
        }
        state.scale = monitor_scale(monitor);
        state.work_area = info.rc_work;
        state.last_window_rect = None;
        let dark = theme::resolve(&state.cfg.theme) == theme::Mode::Dark;
        crate::dwm::set_dark_titlebar(hwnd, dark);
    }
    let work = state.work_area;
    let scale = state.scale.max(0.1);
    state.width = WIDTH.min((work.right - work.left) as f32 / scale - 16.0);
    let max_height = ((work.bottom - work.top) as f32 / scale - 16.0).max(HEADER + FOOTER + ROW);
    state.height = state.content_height().min(max_height);
    if let Some(expanded) = state.expanded.clone() {
        ensure_expanded_visible(state, &expanded);
    }
    let max_offset = state
        .readings
        .len()
        .saturating_sub(state.visible_device_count());
    state.offset = state.offset.min(max_offset);
    let (width, height) = (
        (state.width * scale).round() as i32,
        (state.height * scale).round() as i32,
    );
    let (x, y) = place(state.anchor, work, width, height, (8.0 * scale) as i32);
    let window_rect = (x, y, width, height);
    unsafe {
        if reveal || state.last_window_rect != Some(window_rect) {
            super::canvas::SetWindowPos(
                hwnd,
                if reveal {
                    -1isize as Handle
                } else {
                    std::ptr::null_mut()
                },
                x,
                y,
                width,
                height,
                if reveal { 0x10 } else { 0x114 },
            );
            state.last_window_rect = Some(window_rect);
        }
        InvalidateRect(hwnd, std::ptr::null(), 0);
        if reveal {
            win32::ShowWindow(hwnd, 5);
            win32::SetForegroundWindow(hwnd);
            SetFocus(hwnd);
            if state.refreshing.is_some() {
                win32::SetTimer(hwnd, SPIN_TIMER, 80, 0);
            }
        }
    }
}

fn ensure_expanded_visible(state: &mut State, key: &str) {
    let Some(index) = state.readings.iter().position(|reading| reading.key == key) else {
        return;
    };
    if index < state.offset {
        state.offset = index;
        return;
    }
    let viewport = (state.height - HEADER - FOOTER).max(ROW);
    let before = state
        .readings
        .iter()
        .take(index)
        .skip(state.offset)
        .map(|reading| ROW + state.panel_height(&reading.key))
        .sum::<f32>();
    let target = ROW + state.panel_height(key);
    if before + target > viewport + 0.5 {
        state.offset = index;
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
    fn visible_device_count(&self) -> usize {
        if self.readings.is_empty() {
            return 0;
        }
        let viewport = (self.height - HEADER - FOOTER).max(ROW);
        let mut used = 0.0;
        let mut count = 0;
        for reading in self.readings.iter().skip(self.offset) {
            let height = ROW + self.panel_height(&reading.key);
            if count > 0 && used + height > viewport + 0.5 {
                break;
            }
            used += height;
            count += 1;
            if used >= viewport {
                break;
            }
        }
        count.max(1)
    }

    fn panel_kind(&self, key: &str) -> Option<PanelKind> {
        let reading = self.readings.iter().find(|reading| reading.key == key)?;
        match self.cfg.device_kind(reading) {
            crate::tray_prefs::TrayKind::Mouse => Some(PanelKind::Dpi),
            crate::tray_prefs::TrayKind::Headset => Some(PanelKind::Audio),
            _ => None,
        }
    }

    fn panel_height(&self, key: &str) -> f32 {
        let full = self
            .panel_kind(key)
            .map(PanelKind::full_height)
            .unwrap_or(0.0);
        full * self
            .panel_progress
            .get(key)
            .copied()
            .unwrap_or(0.0)
            .clamp(0.0, 1.0)
    }

    fn content_height(&self) -> f32 {
        let rows = self
            .readings
            .iter()
            .map(|reading| ROW + self.panel_height(&reading.key))
            .sum::<f32>();
        HEADER + FOOTER + rows.max(ROW)
    }
}
fn point(lp: isize, scale: f32) -> (f32, f32) {
    (
        (lp as u16 as i16) as f32 / scale,
        ((lp as u32 >> 16) as u16 as i16) as f32 / scale,
    )
}

fn device_key_for_action(action: &Action) -> Option<&str> {
    match action {
        Action::TogglePanel(key) => Some(key),
        _ => None,
    }
}

fn begin_device_drag(state: &mut State, pointer_y: f32) -> bool {
    let Some(key) = state
        .mouse_down
        .as_ref()
        .and_then(|down| device_key_for_action(&down.action))
        .map(str::to_string)
    else {
        return false;
    };
    let Some(source) = state.readings.iter().position(|reading| reading.key == key) else {
        return false;
    };
    let mut row_top = HEADER + 1.0;
    for reading in state
        .readings
        .iter()
        .skip(state.offset)
        .take(source.saturating_sub(state.offset))
    {
        row_top += ROW + state.panel_height(&reading.key);
    }
    let now = Instant::now();
    state.drag = Some(DragState {
        key,
        source,
        grab_offset_y: (pointer_y - row_top).clamp(0.0, ROW),
        pointer_y,
        target: source,
        previous_target: source,
        started_at: now,
        target_changed_at: now,
    });
    state.pressed = None;
    state.mouse_down = None;
    state.hover = None;
    state.focus = None;
    state.expanded = None;
    state.panel_progress.clear();
    state.animation = None;
    true
}

fn move_dragged_device(state: &mut State, y: f32) -> bool {
    let Some(mut drag) = state.drag.take() else {
        return false;
    };
    drag.pointer_y = y;
    let target = drag_target_index(state, y);
    if target != drag.target {
        drag.previous_target = drag.target;
        drag.target = target;
        drag.target_changed_at = Instant::now();
    }
    state.drag = Some(drag);
    true
}

fn drag_target_index(state: &State, y: f32) -> usize {
    if state.readings.is_empty() {
        return 0;
    }
    let last = state.readings.len() - 1;
    let mut top = HEADER + 1.0;
    for index in state.offset..state.readings.len() {
        let height = ROW;
        if y < top + height / 2.0 {
            return index;
        }
        top += height;
    }
    last
}

fn start_accordion(state: &mut State, key: String) -> bool {
    if state.panel_kind(&key).is_none() {
        return false;
    }
    // 从当前已绘出的高度继续，连续点击也不会跳回旧动画的起点。
    advance_accordion(state);
    let target = (state.expanded.as_deref() != Some(key.as_str())).then_some(key);
    let mut steps = Vec::new();
    for (device, &from) in &state.panel_progress {
        if from > 0.0 && target.as_deref() != Some(device.as_str()) {
            steps.push(AccordionStep {
                key: device.clone(),
                from,
                to: 0.0,
                start_ms: 0.0,
                end_ms: ACCORDION_COLLAPSE_MS,
            });
        }
    }
    if let Some(key) = &target {
        let from = state.panel_progress.get(key).copied().unwrap_or(0.0);
        if from < 1.0 {
            steps.push(AccordionStep {
                key: key.clone(),
                from,
                to: 1.0,
                start_ms: 0.0,
                end_ms: ACCORDION_EXPAND_MS,
            });
        }
    }
    if steps.is_empty() {
        return false;
    }
    let total_ms = steps.iter().map(|step| step.end_ms).fold(0.0, f32::max);
    state.expanded = target.clone();
    state.animation = Some(AccordionAnimation {
        steps,
        total_ms,
        target,
        started_at: Instant::now(),
        _timer_resolution: AnimationTimerResolution::new(),
    });
    true
}

fn advance_accordion(state: &mut State) -> bool {
    let Some(animation) = &mut state.animation else {
        return true;
    };
    let elapsed_ms =
        (animation.started_at.elapsed().as_secs_f32() * 1000.0).min(animation.total_ms);
    for step in &animation.steps {
        let progress =
            ((elapsed_ms - step.start_ms) / (step.end_ms - step.start_ms).max(1.0)).clamp(0.0, 1.0);
        let eased = progress * progress * (3.0 - 2.0 * progress);
        state
            .panel_progress
            .insert(step.key.clone(), step.from + (step.to - step.from) * eased);
    }
    if elapsed_ms >= animation.total_ms {
        let target = animation.target.clone();
        state.animation = None;
        state.panel_progress.clear();
        if let Some(target) = target {
            state.panel_progress.insert(target.clone(), 1.0);
            state.expanded = Some(target);
        } else {
            state.expanded = None;
        }
        true
    } else {
        false
    }
}

fn endpoint(state: &State, slot: AudioSlot) -> Option<crate::audio::Endpoint> {
    match slot {
        AudioSlot::Speaker => state.audio.speaker.clone(),
        AudioSlot::Headset => state.audio.headset.clone(),
    }
}

fn endpoint_mut(state: &mut State, slot: AudioSlot) -> Option<&mut crate::audio::Endpoint> {
    match slot {
        AudioSlot::Speaker => state.audio.speaker.as_mut(),
        AudioSlot::Headset => state.audio.headset.as_mut(),
    }
}

fn apply_audio_delta(state: &Rc<RefCell<State>>, slot: AudioSlot, delta: i16) {
    let endpoint = endpoint(&state.borrow(), slot);
    if let Some(endpoint) = endpoint {
        let percent = (i16::from(endpoint.volume) + delta).clamp(0, 100) as u8;
        let mut state = state.borrow_mut();
        state.audio_controls.set_volume(slot, percent);
        if let Some(value) = endpoint_mut(&mut state, slot) {
            value.volume = percent;
        }
        state.audio_error.clear();
    }
}

fn toggle_audio_mute(state: &Rc<RefCell<State>>, slot: AudioSlot) -> Result<(), String> {
    let mut state = state.borrow_mut();
    if endpoint(&state, slot).is_none() {
        return Err(crate::tr!("未检测到对应音频设备").to_string());
    }
    state.audio_controls.toggle_output(slot);
    state.audio = state.audio_controls.snapshot();
    state.audio_error.clear();
    Ok(())
}

fn apply_local_action(
    hwnd: Handle,
    state: &Rc<RefCell<State>>,
    action: Action,
    fraction: Option<f32>,
) -> bool {
    match action {
        Action::TogglePanel(key) => {
            let kind = state.borrow().panel_kind(&key);
            let mut state = state.borrow_mut();
            if kind.is_none() {
                return true;
            }
            if kind == Some(PanelKind::Audio)
                && state.expanded.as_deref() != Some(key.as_str())
            {
                state.audio_controls.set_active(2, true);
                state.audio_controls.refresh();
                state.audio = state.audio_controls.snapshot();
                unsafe {
                    win32::SetTimer(hwnd, AUDIO_TIMER, 50, 0);
                }
            }
            let started = start_accordion(&mut state, key);
            if started {
                state.hover = None;
                state.focus = None;
            }
            started || state.animation.is_some()
        }
        Action::AudioSlot(slot) => {
            state.borrow_mut().audio_slot = slot;
            true
        }
        Action::AudioSlider(slot) => {
            let endpoint = endpoint(&state.borrow(), slot);
            if let Some(endpoint) = endpoint {
                let percent = fraction
                    .map(|value| (value.clamp(0.0, 1.0) * 100.0).round() as u8)
                    .unwrap_or(endpoint.volume);
                if percent == endpoint.volume {
                    return true;
                }
                let mut state = state.borrow_mut();
                state.audio_controls.set_volume(slot, percent);
                if let Some(value) = endpoint_mut(&mut state, slot) {
                    value.volume = percent;
                }
                state.audio_error.clear();
            }
            true
        }
        Action::ToggleMicrophone => {
            let mut state = state.borrow_mut();
            state.audio_controls.toggle_microphone();
            state.audio = state.audio_controls.snapshot();
            state.audio_error.clear();
            true
        }
        Action::OpenSoundSettings => {
            if let Err(error) = crate::audio::open_sound_settings() {
                state.borrow_mut().audio_error = error;
            }
            true
        }
        external => {
            state.borrow_mut().acts.push(external);
            false
        }
    }
}
fn hide(hwnd: Handle, state: &Rc<RefCell<State>>, deactivated: bool) {
    {
        let mut s = state.borrow_mut();
        if !s.visible {
            return;
        }
        if s.animation.is_some() {
            let target = s
                .animation
                .as_ref()
                .and_then(|animation| animation.target.clone());
            s.panel_progress.clear();
            if let Some(key) = target {
                s.panel_progress.insert(key, 1.0);
            }
            s.animation = None;
        }
        s.visible = false;
        s.pressed = None;
        s.mouse_down = None;
        s.drag = None;
        s.audio_controls.set_active(2, false);
        s.hover = None;
        s.dismissed = deactivated.then(Instant::now);
        s.canvas.release();
        s.fonts.clear();
    }
    unsafe {
        win32::KillTimer(hwnd, SPIN_TIMER);
        win32::KillTimer(hwnd, ACCORDION_TIMER);
        win32::KillTimer(hwnd, DRAG_ANIM_TIMER);
        win32::KillTimer(hwnd, AUDIO_TIMER);
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
    // 预留展开后的可见高度，避免每一帧长高时重建 GDI 位图。
    let max_height = (state.work_area.bottom - state.work_area.top).max(h);
    let mem = state.canvas.ensure(dc as isize, w, max_height);
    let t = theme::resolve(&state.cfg.theme).theme();
    state.hits.clear();
    state.audio_slider = R::default();
    let show_hover = state.animation.is_none();
    let visible_count = state.visible_device_count();
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
        drag,
        focus,
        refreshing,
        updated,
        expanded,
        panel_progress,
        dpi,
        audio,
        audio_slot,
        audio_error,
        audio_slider,
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
            if show_hover { hover.or(*focus) } else { None },
            drag.as_ref(),
            refreshing.map(|(_, t)| t.elapsed().as_secs_f32()),
            *updated,
            expanded.as_deref(),
            panel_progress,
            dpi,
            audio,
            *audio_slot,
            audio_error,
            audio_slider,
            visible_count,
        )
    });
    gfx::blit(dc as isize, mem, w, h);
    EndPaint(hwnd, &ps);
}

fn draw_dpi(
    p: &mut Painter,
    panel: R,
    visible: R,
    key: &str,
    dpi: Option<&DpiState>,
    hits: &mut Vec<(R, Action)>,
    hover: Option<usize>,
) {
    let t = *p.t;
    p.text(
        "DPI",
        R::new(panel.x + 18.0, panel.y + 6.0, 42.0, 18.0),
        10.5,
        true,
        t.text2,
        Align::Left,
    );
    let current = dpi
        .map(|dpi| {
            format!(
                "{} DPI",
                dpi.stages[(dpi.active.saturating_sub(1) as usize).min(3)]
            )
        })
        .unwrap_or_else(|| "—".to_string());
    p.text(
        &current,
        R::new(panel.x + 54.0, panel.y + 6.0, panel.w - 72.0, 18.0),
        12.0,
        true,
        t.text,
        Align::Right,
    );
    if let Some(dpi) = dpi {
        let segment = R::new(panel.x + 18.0, panel.y + 28.0, panel.w - 36.0, 40.0);
        p.round(segment, 9.0, t.bg_sunken);
        let item_w = segment.w / 4.0;
        for (index, level) in dpi.stages.into_iter().enumerate() {
            let r = R::new(
                segment.x + index as f32 * item_w + 3.0,
                segment.y + 3.0,
                item_w - 6.0,
                segment.h - 6.0,
            );
            let active = dpi.active as usize == index + 1;
            if active || hover == Some(hits.len()) {
                p.round(r, 7.0, if active { t.accent_soft } else { t.card_hover });
            }
            p.text(
                &level.to_string(),
                R::new(r.x, r.y - 2.0, r.w, r.h),
                12.0,
                active,
                if active { t.accent } else { t.text2 },
                Align::Center,
            );
            push_visible_hit(
                hits,
                visible,
                r,
                Action::SetDpiStage(key.to_string(), (index + 1) as u8),
            );
        }
    } else {
        p.text(
            crate::tr!("正在读取 DPI 档位…"),
            R::new(panel.x + 18.0, panel.y + 31.0, panel.w - 36.0, 30.0),
            11.0,
            false,
            t.text3,
            Align::Center,
        );
    }
}

fn draw_audio(
    p: &mut Painter,
    panel: R,
    visible: R,
    audio: &crate::audio::Snapshot,
    slot: AudioSlot,
    error: &str,
    hits: &mut Vec<(R, Action)>,
    hover: Option<usize>,
    slider: &mut R,
) {
    let t = *p.t;
    let inset = panel.x + 16.0;
    let tabs = R::new(inset, panel.y + 10.0, panel.w - 32.0, 33.0);
    p.round(tabs, 8.0, t.bg_sunken);
    let tab_w = tabs.w / 2.0;
    for (index, (label, icon, target, available)) in [
        (
            crate::tr!("扬声器"),
            Icon::Speaker,
            AudioSlot::Speaker,
            audio.speaker.is_some(),
        ),
        (
            crate::tr!("耳机"),
            Icon::Device(crate::tray_prefs::TrayKind::Headset),
            AudioSlot::Headset,
            audio.headset.is_some(),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let r = R::new(
            tabs.x + index as f32 * tab_w + 3.0,
            tabs.y + 3.0,
            tab_w - 6.0,
            tabs.h - 6.0,
        );
        let active = slot == target && available;
        if active || (available && hover == Some(hits.len())) {
            p.round(r, 7.0, if active { t.accent_soft } else { t.card });
        }
        let label_w = (p.measure(label, 11.0, active).0 + 8.0).min((r.w - 36.0).max(0.0));
        let group_x = r.cx() - (18.0 + 7.0 + label_w) / 2.0;
        panel_icons::draw(
            p,
            icon,
            R::new(group_x, r.cy() - 9.0, 18.0, 18.0),
            if active { t.accent } else { t.text3 },
            0.0,
        );
        p.text(
            label,
            R::new(group_x + 25.0, r.y, label_w, r.h),
            11.0,
            active,
            if available { t.text } else { t.text3 },
            Align::Center,
        );
        if available {
            push_visible_hit(hits, visible, r, Action::AudioSlot(target));
        }
    }

    let selected = match slot {
        AudioSlot::Speaker => audio.speaker.as_ref(),
        AudioSlot::Headset => audio.headset.as_ref(),
    };
    let volume = R::new(inset, panel.y + 50.0, panel.w - 32.0, 58.0);
    p.card(volume, 9.0, t.card, t.border);
    p.text(
        crate::tr!("音量"),
        R::new(volume.x + 13.0, volume.y + 6.0, 43.0, 18.0),
        10.0,
        true,
        t.text2,
        Align::Left,
    );
    if !error.is_empty() {
        p.text(
            error,
            R::new(volume.x + 60.0, volume.y + 6.0, volume.w - 122.0, 18.0),
            9.5,
            false,
            t.red,
            Align::Left,
        );
    } else if let Some(endpoint) = selected {
        p.text(
            &endpoint.name,
            R::new(volume.x + 60.0, volume.y + 6.0, volume.w - 122.0, 18.0),
            9.5,
            false,
            t.text3,
            Align::Left,
        );
    }
    if let Some(endpoint) = selected {
        let track = R::new(volume.x + 15.0, volume.y + 38.0, volume.w - 91.0, 5.0);
        *slider = track;
        p.round(track, 3.0, t.track);
        let fill_w = track.w * endpoint.volume as f32 / 100.0;
        if fill_w > 0.0 {
            p.round(R::new(track.x, track.y, fill_w, track.h), 3.0, t.accent);
        }
        let knob_x = track.x + fill_w.clamp(6.0, track.w - 6.0);
        p.round(
            R::new(knob_x - 6.0, track.cy() - 6.0, 12.0, 12.0),
            6.0,
            t.card,
        );
        p.round_stroke(
            R::new(knob_x - 6.0, track.cy() - 6.0, 12.0, 12.0),
            6.0,
            t.accent,
            2.0,
        );
        let percent = R::new(volume.right() - 61.0, volume.y + 26.0, 48.0, 26.0);
        p.text(
            &format!("{}%", endpoint.volume),
            percent,
            13.0,
            true,
            if endpoint.muted { t.text3 } else { t.text },
            Align::Right,
        );
        push_visible_hit(
            hits,
            visible,
            R::new(track.x, track.y - 12.0, track.w, 26.0),
            Action::AudioSlider(slot),
        );
    } else {
        p.text(
            crate::tr!("未检测到对应音频设备"),
            R::new(volume.x + 13.0, volume.y + 28.0, volume.w - 26.0, 22.0),
            11.0,
            false,
            t.text3,
            Align::Left,
        );
    }
    let microphone = audio.microphone.as_ref();
    let muted = microphone.is_some_and(|value| value.muted);
    let mic_rect = R::new(inset, panel.y + 116.0, 204.0, 27.0);
    let mic_hovered = microphone.is_some() && hover == Some(hits.len());
    if mic_hovered {
        p.round(mic_rect, 7.0, t.card_hover);
    }
    panel_icons::draw(
        p,
        Icon::Microphone,
        R::new(mic_rect.x + 2.0, mic_rect.y + 4.0, 18.0, 18.0),
        if muted { t.red } else { t.text2 },
        0.0,
    );
    if muted {
        p.line(
            mic_rect.x + 3.0,
            mic_rect.y + 6.0,
            mic_rect.x + 19.0,
            mic_rect.y + 22.0,
            t.red,
            1.8,
        );
    }
    p.text(
        crate::tr!("麦克风"),
        R::new(mic_rect.x + 26.0, mic_rect.y, 67.0, mic_rect.h),
        10.5,
        false,
        if microphone.is_some() {
            t.text2
        } else {
            t.text3
        },
        Align::Left,
    );
    p.text(
        if microphone.is_none() {
            "—"
        } else if muted {
            crate::tr!("已静音")
        } else {
            crate::tr!("已开启")
        },
        R::new(mic_rect.x + 98.0, mic_rect.y, 56.0, mic_rect.h),
        10.0,
        true,
        if muted { t.red } else { t.accent },
        Align::Left,
    );
    let mic_switch = R::new(mic_rect.right() - 37.0, mic_rect.cy() - 8.0, 30.0, 16.0);
    let enabled = microphone.is_some() && !muted;
    p.round(mic_switch, 8.0, if enabled { t.accent } else { t.track });
    p.circle(
        if enabled {
            mic_switch.right() - 8.0
        } else {
            mic_switch.x + 8.0
        },
        mic_switch.cy(),
        5.0,
        if enabled { t.accent_fg } else { t.text3 },
    );
    if microphone.is_some() {
        push_visible_hit(hits, visible, mic_rect, Action::ToggleMicrophone);
    }
    let settings = R::new(panel.right() - 131.0, panel.y + 116.0, 115.0, 27.0);
    let settings_hovered = hover == Some(hits.len());
    if settings_hovered {
        p.round(settings, 7.0, t.card_hover);
    }
    panel_icons::draw(
        p,
        Icon::Settings,
        R::new(settings.x + 8.0, settings.cy() - 8.0, 16.0, 16.0),
        t.text2,
        0.0,
    );
    p.text(
        crate::tr!("声音设置"),
        R::new(settings.x + 30.0, settings.y, settings.w - 34.0, settings.h),
        10.5,
        false,
        if settings_hovered { t.text } else { t.text2 },
        Align::Left,
    );
    push_visible_hit(hits, visible, settings, Action::OpenSoundSettings);
}

fn push_visible_hit(hits: &mut Vec<(R, Action)>, visible: R, hit: R, action: Action) {
    if hit.x >= visible.x
        && hit.y >= visible.y
        && hit.right() <= visible.right() + 0.01
        && hit.bottom() <= visible.bottom() + 0.01
    {
        hits.push((hit, action));
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_device_row(
    p: &mut Painter,
    w: f32,
    list: R,
    r: R,
    d: &DeviceReading,
    cfg: &Settings,
    kind: crate::tray_prefs::TrayKind,
    panel_kind: Option<PanelKind>,
    progress: f32,
    expanded: Option<&str>,
    dpi: &BTreeMap<String, DpiState>,
    audio: &crate::audio::Snapshot,
    audio_slot: AudioSlot,
    audio_error: &str,
    hits: &mut Vec<(R, Action)>,
    hover: Option<usize>,
    row_hovered: bool,
    is_dragging: bool,
    drag_lift: f32,
    push_hit: bool,
    audio_slider: &mut R,
) {
    let t = *p.t;
    if is_dragging {
        let lift = drag_lift.clamp(0.0, 1.0);
        let shadow = gfx::with_alpha(t.text, (18.0 + lift * 18.0) as u8);
        p.round(
            R::new(r.x + 2.0, r.y + 4.0 + lift * 2.0, r.w, r.h),
            9.0,
            shadow,
        );
    }
    if is_dragging {
        p.round(r, 8.0, t.accent_soft);
    } else if progress > 0.01 || expanded == Some(d.key.as_str()) || row_hovered {
        p.round(r, 8.0, t.card_hover);
    }
    if row_hovered || is_dragging {
        let handle_color = if is_dragging { t.accent } else { t.text3 };
        for dx in [-3.2, 3.2] {
            for dy in [-5.5, 0.0, 5.5] {
                p.circle(r.x + 13.0 + dx, r.cy() + dy, 1.15, handle_color);
            }
        }
    }
    let ink = if d.connected { t.text } else { t.text3 };
    panel_icons::draw(
        p,
        Icon::Device(kind),
        R::new(r.x + 32.0, r.cy() - 12.5, 25.0, 25.0),
        if d.connected { t.text2 } else { t.text3 },
        0.0,
    );
    p.text(
        cfg.device_name(d),
        R::new(r.x + 67.0, r.y + 10.0, w - 193.0, 21.0),
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
        R::new(r.x + 67.0, r.y + 33.0, w - 188.0, 17.0),
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
    let open_fraction = panel_kind
        .map(|kind| progress / kind.full_height())
        .unwrap_or(0.0);
    if panel_kind.is_some() {
        panel_icons::draw(
            p,
            Icon::Chevron,
            R::new(w - 30.0, r.cy() - 6.0, 12.0, 12.0),
            t.text3,
            open_fraction * 90.0,
        );
    }
    if push_hit {
        hits.push((r, Action::TogglePanel(d.key.clone())));
    }
    if progress > 0.01 {
        let visible = R::new(list.x + 1.0, r.bottom() + 1.0, list.w - 2.0, progress);
        let panel = R::new(
            visible.x,
            visible.y,
            visible.w,
            panel_kind.map(PanelKind::full_height).unwrap_or(progress),
        );
        p.with_clip(visible, |p| {
            p.rect(visible, t.bg);
            match panel_kind {
                Some(PanelKind::Dpi) => {
                    draw_dpi(p, panel, visible, &d.key, dpi.get(&d.key), hits, hover)
                }
                Some(PanelKind::Audio) => draw_audio(
                    p,
                    panel,
                    visible,
                    audio,
                    audio_slot,
                    audio_error,
                    hits,
                    hover,
                    audio_slider,
                ),
                None => {}
            }
        });
    }
}

fn drag_flow_index(source: usize, target: usize, index: usize) -> usize {
    if source < target && index > source && index <= target {
        index - 1
    } else if source > target && index >= target && index < source {
        index + 1
    } else {
        index
    }
}

fn drag_row_y(list: R, offset: usize, drag: &DragState, index: usize) -> f32 {
    let progress =
        (drag.target_changed_at.elapsed().as_secs_f32() * 1000.0 / DRAG_REFLOW_MS).clamp(0.0, 1.0);
    let eased = progress * progress * (3.0 - 2.0 * progress);
    let before = drag_flow_index(drag.source, drag.previous_target, index);
    let after = drag_flow_index(drag.source, drag.target, index);
    let y_before = list.y + 1.0 + (before.saturating_sub(offset)) as f32 * ROW;
    let y_after = list.y + 1.0 + (after.saturating_sub(offset)) as f32 * ROW;
    y_before + (y_after - y_before) * eased
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
    dragging: Option<&DragState>,
    refreshing: Option<f32>,
    updated: bool,
    expanded: Option<&str>,
    panel_progress: &BTreeMap<String, f32>,
    dpi: &BTreeMap<String, DpiState>,
    audio: &crate::audio::Snapshot,
    audio_slot: AudioSlot,
    audio_error: &str,
    audio_slider: &mut R,
    visible_count: usize,
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
    p.with_clip(list, |p| {
        let mut y = list.y + 1.0;
        let drag_source =
            dragging.and_then(|drag| readings.iter().position(|reading| reading.key == drag.key));
        for (absolute_index, d) in readings.iter().enumerate().skip(offset) {
            if dragging.is_some() && drag_source == Some(absolute_index) {
                continue;
            }
            let row_y = dragging
                .map(|drag| drag_row_y(list, offset, drag, absolute_index))
                .unwrap_or(y);
            if row_y >= list.bottom() - 1.0 {
                break;
            }
            let kind = cfg.device_kind(d);
            let panel_kind = match kind {
                crate::tray_prefs::TrayKind::Mouse => Some(PanelKind::Dpi),
                crate::tray_prefs::TrayKind::Headset => Some(PanelKind::Audio),
                _ => None,
            };
            let progress = if dragging.is_some() {
                0.0
            } else {
                panel_kind
                    .and_then(|kind| {
                        panel_progress
                            .get(&d.key)
                            .map(|progress| (kind.full_height(), *progress))
                    })
                    .map(|(height, progress)| height * progress.clamp(0.0, 1.0))
                    .unwrap_or(0.0)
            };
            let row_height = ROW + progress;
            if absolute_index > offset {
                p.hline(list.x + 1.0, list.right() - 1.0, row_y, t.border);
            }
            let r = R::new(list.x + 1.0, row_y, list.w - 2.0, ROW - 2.0);
            let row_hovered = hover == Some(hits.len());
            if dragging.is_some_and(|drag| absolute_index == drag.target) {
                p.round(r, 8.0, t.card_hover);
                p.round_stroke(r, 8.0, t.accent, 1.0);
            }
            draw_device_row(
                p,
                w,
                list,
                r,
                d,
                cfg,
                kind,
                panel_kind,
                progress,
                expanded,
                dpi,
                audio,
                audio_slot,
                audio_error,
                hits,
                hover,
                row_hovered,
                false,
                0.0,
                dragging.is_none(),
                audio_slider,
            );
            y += row_height;
        }
        if let (Some(drag), Some(source)) = (dragging, drag_source) {
            if let Some(d) = readings.get(source) {
                let kind = cfg.device_kind(d);
                let ghost_y = (drag.pointer_y - drag.grab_offset_y)
                    .clamp(list.y + 1.0, (list.bottom() - ROW).max(list.y + 1.0));
                let ghost = R::new(list.x + 1.0, ghost_y, list.w - 2.0, ROW - 2.0);
                let lift = (drag.started_at.elapsed().as_secs_f32() * 1000.0 / DRAG_LIFT_MS)
                    .clamp(0.0, 1.0);
                draw_device_row(
                    p,
                    w,
                    list,
                    ghost,
                    d,
                    cfg,
                    kind,
                    None,
                    0.0,
                    None,
                    dpi,
                    audio,
                    audio_slot,
                    audio_error,
                    hits,
                    hover,
                    true,
                    true,
                    lift,
                    false,
                    audio_slider,
                );
            }
        }
    });
    let count = visible_count;
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
