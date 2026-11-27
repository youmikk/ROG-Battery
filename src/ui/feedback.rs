//! Styled feedback window. Native edit controls retain IME, selection and accessibility.
use super::feedback_icons::{self, Icon};
use super::{
    gfx::{Align, Painter, R},
    theme::Theme,
};
use crate::feedback::{client, Candidate};
use native_windows_gui as nwg;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{Arc, Mutex},
};
use winapi::um::winuser as wu;

mod layout;
mod state;
mod capture;
use state::{DiagnosticReport, Model, Task, View};

pub fn text<'a>(zh: &'a str, en: &'a str) -> &'a str {
    if crate::i18n::english() {
        en
    } else {
        zh
    }
}

const WIDTH: f32 = 660.0;
const BODY_TOP: f32 = 91.0;
const FOOTER: f32 = 66.0;
const SLOTS: usize = 156;
const DEVICE_PAGE_SIZE: usize = 12;
const HISTORY_PAGE_SIZE: usize = 20;

fn back_view(view: View, submitted: bool) -> View {
    if matches!(view, View::Thread | View::Success) {
        View::History
    } else if submitted {
        View::Success
    } else {
        View::Form
    }
}
#[derive(Clone)]
enum Action {
    Close,
    Back,
    History,
    HistoryPage(usize),
    DeleteLocal(String),
    ClearHistory,
    Devices,
    Device(usize),
    UnknownDevice(usize),
    DevicePage(usize),
    Manual,
    Connection(usize),
    Category(usize),
    Diagnostics,
    Capture,
    CaptureStart,
    CaptureMark(usize),
    CaptureStop,
    CaptureSave,
    CaptureClear,
    CaptureFolder,
    ToggleCapture,
    ToggleDiagnostics,
    AddImage,
    RemoveImage(usize),
    Example,
    Submit,
    Fallback,
    Entry(usize),
    OpenIssue,
    Reply,
    Refresh,
    New,
}
impl Action {
    fn enabled(&self, model: &Model) -> bool {
        if matches!(model.task, Some(Task::CaptureExport | Task::HistoryDelete)) { return matches!(self, Self::Close); }
        if matches!(self, Self::Capture) && (model.software || (model.capture.is_none()
            && !model.selected.and_then(|index| model.devices.get(index))
                .is_some_and(|device| device.capture_interface.is_some()))) { return false; }
        if matches!(self, Self::CaptureStart) && (model.software
            || !model.selected.and_then(|index| model.devices.get(index))
                .is_some_and(|device| device.capture_interface.is_some())) { return false; }
        if matches!(self, Self::History) && model.task == Some(Task::History) {
            return false;
        }
        !model.busy() || matches!(self,
            Self::Close | Self::Back | Self::History | Self::New | Self::Entry(_) | Self::OpenIssue)
    }

    fn navigates(&self) -> bool {
        matches!(self, Self::Back | Self::History | Self::New | Self::Entry(_))
    }
}
#[derive(Clone, Copy)]
enum Style {
    Plain,
    Card,
    Choice(bool),
    Primary,
    Toggle(bool),
    Close,
    Connection(bool),
    Device(crate::tray_prefs::TrayKind),
    Option(crate::tray_prefs::TrayKind, bool),
    Attachment,
    Chip,
    History,
}
struct Item {
    action: Action,
    label: String,
    detail: String,
    rect: R,
    style: Style,
    root: bool,
    tag: String,
}
enum Decoration {
    Surface(R, bool),
    Glyph(Icon, R),
    Tag(String, R),
    Note(String, R),
    Count(R),
    Center(String, R, f32, bool),
    Check(R),
}
impl Decoration {
    fn rect(&self) -> R {
        match self {
            Self::Surface(rect, _) | Self::Glyph(_, rect) | Self::Tag(_, rect)
                | Self::Note(_, rect) | Self::Count(rect) | Self::Center(_, rect, _, _)
                | Self::Check(rect) => *rect,
        }
    }
}
struct Label {
    text: String,
    rect: R,
    size: f32,
    bold: bool,
    muted: bool,
    centered: bool,
}
enum ResultMessage {
    History(Result<Vec<client::Entry>, String>),
    Deleted(Vec<String>, Result<(), String>),
    Submitted(Result<client::SubmitOutcome, String>, Option<String>),
    Thread(
        Result<client::Thread, String>,
        Result<Option<String>, String>,
    ),
    Reply(u64, String, Result<client::ReplyReceipt, String>),
    Draft(String, Result<client::Draft, String>),
    Images(Result<Vec<client::Image>, String>),
    Fallback(Result<client::Fallback, String>),
    SoftwareDiagnostics(String),
    CaptureSaved(Result<std::path::PathBuf, String>),
    DeviceInventory(crate::feedback::inventory::DeviceInventory),
    Failed,
}
struct Colors {
    theme: Theme,
    brush: crate::win32::Handle,
    sunken_brush: crate::win32::Handle,
}
impl Colors {
    fn new(dark: bool) -> Self {
        let mut theme = if dark {
            super::theme::DARK
        } else {
            super::theme::LIGHT
        };
        if dark {
            theme.red = 0xFFFF8E83;
        } else {
            theme.bg_sunken = 0xFFF5F7F4;
            theme.card_hover = 0xFFF3F6F0;
            theme.red = 0xFFB74639;
        }
        Self {
            theme,
            brush: unsafe { crate::win32::CreateSolidBrush(colorref(theme.card)) },
            sunken_brush: unsafe { crate::win32::CreateSolidBrush(colorref(theme.bg_sunken)) },
        }
    }
}
impl Drop for Colors {
    fn drop(&mut self) {
        unsafe {
            crate::win32::DeleteObject(self.brush);
            crate::win32::DeleteObject(self.sunken_brush);
        }
    }
}
fn colorref(c: u32) -> u32 {
    (c & 255) << 16 | (c & 0xff00) | (c >> 16 & 255)
}

struct Controls {
    window: nwg::Window,
    body: nwg::Frame,
    buttons: Vec<nwg::Button>,
    header_buttons: Vec<nwg::Button>,
    description: nwg::TextBox,
    brand: nwg::TextInput,
    model: nwg::TextInput,
    reply: nwg::TextBox,
    _font: nwg::Font,
    state: RefCell<Model>,
    items: RefCell<Vec<Item>>,
    labels: RefCell<Vec<Label>>,
    edit_rects: RefCell<Vec<(nwg::ControlHandle, R)>>,
    boxes: RefCell<Vec<R>>,
    decorations: RefCell<Vec<Decoration>>,
    colors: RefCell<Colors>,
    scroll: Cell<f32>,
    content_height: Cell<f32>,
    height: Cell<f32>,
    max_height: Cell<f32>,
    owner: nwg::ControlHandle,
    overlay: nwg::Window,
    scale: Cell<f32>,
    native_frame: Cell<bool>,
    results: Arc<Mutex<Vec<(u64, ResultMessage)>>>,
    notice: nwg::NoticeSender,
    updating: Cell<bool>,
    description_count: Cell<usize>,
    applied: RefCell<std::collections::BTreeMap<usize, layout::NativeState>>,
    scrollbar: Cell<Option<(i32, u32, i32)>>,
    inventory_working: Cell<bool>,
    capture_folder: RefCell<Option<std::path::PathBuf>>,
    request_capture_refresh: Box<dyn Fn()>,
}
pub struct Dialog {
    controls: Rc<Controls>,
    handler: Option<nwg::EventHandler>,
    raw: Vec<nwg::RawEventHandler>,
}

fn set_rect(handle: nwg::ControlHandle, r: R, scale: f32) {
    super::canvas::set_window_pos(
        handle.hwnd().unwrap() as _,
        (r.x * scale).round() as i32,
        (r.y * scale).round() as i32,
        (r.w * scale).round() as i32,
        (r.h * scale).round() as i32,
        0x14,
    );
}
fn visible(handle: nwg::ControlHandle, show: bool) {
    unsafe {
        wu::ShowWindow(
            handle.hwnd().unwrap(),
            if show { wu::SW_SHOWNA } else { wu::SW_HIDE },
        );
    }
}

impl Controls {
    fn body_height(&self) -> f32 {
        self.height.get() - BODY_TOP - FOOTER
    }
    fn label(
        &self,
        value: impl Into<String>,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        size: f32,
        bold: bool,
        muted: bool,
    ) {
        self.labels.borrow_mut().push(Label {
            text: value.into(),
            rect: R::new(x, y, w, h),
            size,
            bold,
            muted,
            centered: false,
        });
    }
    fn center_last_label(&self) {
        if let Some(label) = self.labels.borrow_mut().last_mut() {
            label.centered = true;
        }
    }
    fn button(
        &self,
        action: Action,
        label: impl Into<String>,
        detail: impl Into<String>,
        rect: R,
        style: Style,
        root: bool,
    ) {
        self.items.borrow_mut().push(Item {
            action,
            label: label.into(),
            detail: detail.into(),
            rect,
            style,
            root,
            tag: String::new(),
        });
    }
    fn tag_last(&self, value: impl Into<String>) {
        if let Some(item) = self.items.borrow_mut().last_mut() {
            item.tag = value.into();
        }
    }
    fn surface(&self, r: R, soft: bool) {
        self.decorations
            .borrow_mut()
            .push(Decoration::Surface(r, soft));
    }
    fn glyph(&self, icon: Icon, r: R) {
        self.decorations
            .borrow_mut()
            .push(Decoration::Glyph(icon, r));
    }
    fn tag(&self, value: impl Into<String>, r: R) {
        self.decorations
            .borrow_mut()
            .push(Decoration::Tag(value.into(), r));
    }
    fn edit(&self, handle: nwg::ControlHandle, r: R) {
        self.boxes.borrow_mut().push(r);
        self.edit_rects.borrow_mut().push((
            handle,
            R::new(r.x + 12.0, r.y + 9.0, r.w - 24.0, r.h - 18.0),
        ));
    }
    fn diagnostics(&self) -> String {
        let s = self.state.borrow();
        if let Some(DiagnosticReport::Complete(value)) = &s.diagnostics_override {
            return value.clone();
        }
        if s.restored_draft.is_some() {
            return String::new();
        }
        let mut value = if let Some(DiagnosticReport::Summary(value)) = &s.diagnostics_override {
            value.clone()
        } else if s.software {
            format!("App: X-Battery {}\nSystem: {}\nLog: preparing safe summary\nSoftware log summary\n",
                crate::gui::APP_VERSION, crate::feedback::system_version())
        } else { s
            .selected
            .and_then(|i| s.devices.get(i).map(|d| d.diagnostics.clone()))
            .or_else(|| s.selected_unknown.and_then(|index| s.unknown_devices.get(index))
                .map(|device| device.diagnostics.clone()))
            .unwrap_or_else(|| {
                format!(
                    "App: X-Battery {}\nSystem: {}\nNo detected interface selected.\n",
                    crate::gui::APP_VERSION,
                    crate::feedback::system_version()
                )
            }) };
        if !s.software { value.push_str(&format!(
            "Selected connection: {}\n",
            ["2.4GHz", "USB", "Bluetooth", "unknown"][s.connection]
        )); }
        crate::feedback::with_inventory(&value, s.inventory_report.as_deref()
            .unwrap_or("HID interface inventory: preparing; not yet collected\n"))
    }
    fn draft(&self) -> client::Draft {
        let s = self.state.borrow();
        if let Some(draft) = &s.restored_draft {
            let mut draft = draft.clone();
            draft.images = s.images.clone();
            draft.capture = s.attach_capture.then(|| s.capture.clone()).flatten();
            return draft;
        }
        let mut device = if s.software {
            crate::feedback::software::device()
        } else { s.restored_device.clone().or_else(|| s
            .selected
            .and_then(|i| s.devices.get(i).map(|d| d.device.clone())))
            .unwrap_or_else(|| client::Device {
                name: format!("{} {}", self.brand.text().trim(), self.model.text().trim())
                    .trim()
                    .into(),
                family: "other".into(),
                connection: String::new(),
                brand: Some(self.brand.text().trim().into()),
                model: Some(self.model.text().trim().into()),
            }) };
        if !s.software {
            device.connection = ["2.4GHz", "USB", "Bluetooth", "unknown"][s.connection].into();
        }
        client::Draft {
            category: if s.software { "other" } else { ["device", "battery", "dpi", "other"][s.category] }.into(),
            device,
            description: self.description.text().trim().to_string(),
            diagnostics: s.attach_diagnostics.then(|| self.diagnostics()),
            images: s.images.clone(),
            capture: s.attach_capture.then(|| s.capture.clone()).flatten(),
        }
    }
    fn rebuild(&self) {
        self.rebuild_position(true);
    }

    fn rebuild_position(&self, recenter: bool) {
        self.items.borrow_mut().clear();
        self.labels.borrow_mut().clear();
        self.edit_rects.borrow_mut().clear();
        self.boxes.borrow_mut().clear();
        self.decorations.borrow_mut().clear();
        let s = self.state.borrow();
        self.window.set_text(if s.software {
            text("反馈软件问题 · X-Battery", "Software feedback · X-Battery")
        } else { text("反馈设备 · X-Battery", "Device feedback · X-Battery") });
        self.button(
            Action::Close,
            text("关闭反馈", "Close feedback"),
            "",
            R::new(609.0, 26.0, 32.0, 32.0),
            Style::Close,
            true,
        );
        let is_sub = matches!(s.view, View::Diagnostics | View::Capture | View::History | View::Thread);
        if is_sub {
            self.button(
                Action::Back,
                text("返回", "Back"),
                "",
                R::new(25.0, 27.0, 32.0, 32.0),
                Style::Plain,
                true,
            );
        } else if s.view == View::Form {
            let width = if crate::i18n::english() { 145.0 } else { 115.0 };
            self.button(
                Action::History,
                text("我的反馈", "My feedback"),
                "",
                R::new(599.0 - width, 27.0, width, 30.0),
                Style::Plain,
                true,
            );
        }
        if s.view == View::Thread {
            self.button(Action::Refresh, text("刷新回复", "Refresh replies"), "",
                R::new(454.0, 27.0, 145.0, 30.0), Style::Plain, true);
        }
        let mut y = 0.0;
        match s.view {
            View::Form => {
                if !s.software {
                self.label(
                    text("反馈哪台设备", "Which device?"),
                    25.0,
                    y,
                    390.0,
                    19.0,
                    12.0,
                    true,
                    false,
                );
                self.label(
                    if crate::i18n::english() {
                        format!("{} identified · {} other", s.devices.len(), s.unknown_devices.len())
                    } else {
                        format!("已识别 {} 台 · 待确认 {} 台", s.devices.len(), s.unknown_devices.len())
                    },
                    390.0,
                    y,
                    245.0,
                    19.0,
                    11.0,
                    false,
                    true,
                );
                y += 27.0;
                let device = s.selected.and_then(|i| s.devices.get(i));
                let unknown = s.selected_unknown.and_then(|index| s.unknown_devices.get(index));
                let restored_device = s.restored_device.as_ref();
                let device_kind = device
                    .map(|d| kind(&d.device.family))
                    .or_else(|| restored_device.map(|device| kind(&device.family)))
                    .unwrap_or(crate::tray_prefs::TrayKind::Generic);
                self.button(
                    Action::Devices,
                    device
                        .map(|d| d.label.as_str())
                        .or_else(|| unknown.map(unknown_name))
                        .or_else(|| restored_device.map(|device| device.name.as_str()))
                        .unwrap_or(text("列表中没有我的设备", "My device is not listed")),
                    device.map(device_note).unwrap_or_else(|| {
                        if let Some(unknown) = unknown {
                            format!("{} · {}", text("待确认型号", "Model unconfirmed"), unknown_identifier(unknown))
                        } else if restored_device.is_some() { text("来自待提交反馈", "From the pending report").into() }
                        else { text("手动填写品牌和型号", "Enter the brand and model").into() }
                    }),
                    R::new(25.0, y, 610.0, 62.0),
                    Style::Device(device_kind),
                    false,
                );
                if device.is_some_and(|d| d.battery == "—") {
                    self.tag_last(text("电量未知", "Unknown battery"));
                }
                y += 62.0;
                if s.menu {
                    y += 5.0;
                    let known_count = s.devices.len();
                    let total = known_count + s.unknown_devices.len();
                    let pages = total.max(1).div_ceil(DEVICE_PAGE_SIZE);
                    let page = s.device_page.min(pages - 1);
                    let start = page * DEVICE_PAGE_SIZE;
                    let end = (start + DEVICE_PAGE_SIZE).min(total);
                    let has_unknown = end > known_count;
                    let loading = s.inventory_report.is_none();
                    let section_height = if has_unknown || loading { 27.0 } else { 0.0 };
                    let page_height = if pages > 1 { 35.0 } else { 0.0 };
                    self.surface(R::new(25.0, y, 610.0,
                        (end - start + 1) as f32 * 35.0 + section_height + page_height + 8.0), false);
                    y += 4.0;
                    for (i, device) in s.devices.iter().enumerate().skip(start).take(end - start) {
                        self.button(
                            Action::Device(i),
                            &device.label,
                            candidate_identifier(device),
                            R::new(29.0, y, 602.0, 35.0),
                            Style::Option(kind(&device.device.family), s.selected == Some(i)),
                            false,
                        );
                        y += 35.0;
                    }
                    self.button(
                        Action::Manual,
                        text("列表中没有我的设备", "My device is not listed"),
                        "",
                        R::new(29.0, y, 602.0, 35.0),
                        Style::Option(crate::tray_prefs::TrayKind::Generic,
                            s.selected.is_none() && s.selected_unknown.is_none()),
                        false,
                    );
                    y += 35.0;
                    if has_unknown || loading {
                        self.label(if loading { text("正在查找其他设备…", "Finding other devices…") }
                            else { text("未识别设备 · 可能名称", "Other devices · Reported name") },
                            42.0, y + 4.0, 570.0, 23.0, 10.5, false, true);
                        y += 27.0;
                    }
                    for (index, unknown) in s.unknown_devices.iter().enumerate()
                        .filter(|(index, _)| (start..end).contains(&(known_count + index))) {
                        self.button(Action::UnknownDevice(index), unknown_name(unknown), unknown_identifier(unknown),
                            R::new(29.0, y, 602.0, 35.0),
                            Style::Option(crate::tray_prefs::TrayKind::Generic, s.selected_unknown == Some(index)), false);
                        y += 35.0;
                    }
                    if pages > 1 {
                        if page > 0 {
                            self.button(Action::DevicePage(page - 1), text("上一页", "Previous"), "",
                                R::new(42.0, y, 85.0, 31.0), Style::Plain, false);
                        }
                        self.label(format!("{} / {}", page + 1, pages), 300.0, y + 4.0, 100.0, 23.0,
                            11.0, false, true);
                        if page + 1 < pages {
                            self.button(Action::DevicePage(page + 1), text("下一页", "Next"), "",
                                R::new(530.0, y, 85.0, 31.0), Style::Plain, false);
                        }
                        y += 35.0;
                    }
                    y += 4.0;
                }
                if device.is_none() && restored_device.is_none() {
                    y += 10.0;
                    if let Some(unknown) = unknown {
                        self.label(format!("{}：{} · {} · {} {}", text("可能的名称", "Reported name"),
                            unknown_name(unknown), unknown_identifier(unknown), unknown.interfaces,
                            text("个接口", "interfaces")), 25.0, y, 610.0, 24.0, 11.0, false, true);
                        y += 30.0;
                    }
                    self.label(text("设备品牌（必填）", "Brand (required)"), 25.0, y, 300.0, 20.0, 11.0, true, false);
                    self.label(text("完整型号（必填）", "Full model (required)"), 335.0, y, 300.0, 20.0, 11.0, true, false);
                    y += 26.0;
                    self.edit(self.brand.handle, R::new(25.0, y, 300.0, 37.0));
                    self.edit(self.model.handle, R::new(335.0, y, 300.0, 37.0));
                    y += 45.0;
                    self.label(text("请按包装或机身填写，保留 Pro / Max / 代数等后缀。无需查找 VID/PID。",
                        "Use the full model from the label, including Pro / Max / generation. No VID/PID lookup needed."),
                        25.0, y, 610.0, 40.0, 10.0, false, true);
                    y += 40.0;
                } else if device.is_some_and(|device| device.battery == "—") {
                    y += 10.0;
                    self.label(text("请确认显示的型号与实物一致；如果有出入，可以选择“列表中没有我的设备”手动补充。",
                        "Check that the full model matches your device, or select 'My device is not listed' to enter it manually."),
                        25.0, y, 610.0, 36.0, 10.0, false, true);
                    y += 36.0;
                }
                y += 10.0;
                self.label(
                    text("连接方式", "Connection"),
                    25.0,
                    y,
                    80.0,
                    23.0,
                    11.0,
                    false,
                    true,
                );
                self.center_last_label();
                let mut x = if crate::i18n::english() { 105.0 } else { 82.0 };
                for (i, name) in [
                    "2.4 GHz",
                    text("USB 有线", "USB"),
                    text("蓝牙", "Bluetooth"),
                    text("不确定", "Not sure"),
                ]
                .iter()
                .enumerate()
                {
                    let w = label_width(name, 11.0) + 14.0;
                    self.button(
                        Action::Connection(i),
                        *name,
                        "",
                        R::new(x, y, w, 23.0),
                        Style::Connection(s.connection == i),
                        false,
                    );
                    x += w + 5.0;
                }
                y += 40.0;
                self.label(
                    text("遇到了什么问题", "What is the problem?"),
                    25.0,
                    y,
                    580.0,
                    19.0,
                    12.0,
                    true,
                    false,
                );
                y += 27.0;
                let mut x = 25.0;
                for (i, name) in [
                    text("设备未识别", "Not detected"),
                    text("电量异常或不显示", "Battery reading"),
                    text("DPI 无法切换", "DPI switching"),
                    text("其他问题", "Other"),
                ]
                .iter()
                .enumerate()
                {
                    let w = label_width(name, 12.0) + 24.0;
                    self.button(
                        Action::Category(i),
                        *name,
                        "",
                        R::new(x, y, w, 35.0),
                        Style::Choice(s.category == i),
                        false,
                    );
                    x += w + 6.0;
                }
                y += 52.0;
                }
                self.label(
                    text("问题描述", "Description"),
                    25.0,
                    y,
                    400.0,
                    19.0,
                    12.0,
                    true,
                    false,
                );
                self.button(
                    Action::Example,
                    text("填入示例", "Use example"),
                    "",
                    R::new(535.0, y - 3.0, 100.0, 25.0),
                    Style::Plain,
                    false,
                );
                y += 27.0;
                self.boxes.borrow_mut().push(R::new(25.0, y, 610.0, 126.0));
                self.edit_rects
                    .borrow_mut()
                    .push((self.description.handle, R::new(37.0, y + 11.0, 586.0, 85.0)));
                self.label(
                    text(
                        "何时出现？是否间歇发生？官方驱动能否显示电量？",
                        "When? Intermittent? Does the official app show battery?",
                    ),
                    37.0,
                    y + 101.0,
                    470.0,
                    19.0,
                    10.0,
                    false,
                    true,
                );
                self.decorations.borrow_mut().push(Decoration::Count(R::new(
                    540.0,
                    y + 101.0,
                    83.0,
                    19.0,
                )));
                y += 135.0;
                let attachment_y = y;
                let attachment_w = if crate::i18n::english() { 176.0 } else { 139.0 };
                self.button(
                    Action::AddImage,
                    format!(
                        "{}  {}/3",
                        text("添加截图", "Add screenshot"),
                        s.images.len()
                    ),
                    "",
                    R::new(25.0, y, attachment_w, 30.0),
                    Style::Attachment,
                    false,
                );
                let mut x = 25.0 + attachment_w + 8.0;
                for (i, image) in s.images.iter().enumerate() {
                    let w = (label_width(&image.name, 11.0) + 46.0).min(190.0);
                    if x + w > 635.0 {
                        x = 25.0;
                        y += 37.0;
                    }
                    self.button(
                        Action::RemoveImage(i),
                        &image.name,
                        "",
                        R::new(x, y, w, 30.0),
                        Style::Chip,
                        false,
                    );
                    x += w + 8.0;
                }
                let hint = text("可选，每张不超过 5 MB", "Optional, up to 5 MB each");
                let hint_w = label_width(hint, 10.0);
                if x + hint_w > 635.0 {
                    x = 25.0;
                    y += 32.0;
                }
                self.label(hint, x, y, hint_w + 8.0, 30.0, 10.0, false, true);
                self.center_last_label();
                y = y.max(attachment_y) + 48.0;
                self.surface(R::new(25.0, y, 610.0, 65.0), true);
                self.glyph(Icon::File, R::new(39.0, y + 22.0, 20.0, 20.0));
                self.label(
                    if s.software { text("附带软件诊断", "Include software diagnostics") }
                        else { text("附带设备诊断", "Include device diagnostics") },
                    70.0,
                    y + 10.0,
                    360.0,
                    20.0,
                    12.0,
                    true,
                    false,
                );
                self.label(
                    text(
                        if s.software { "自动整理近期运行记录，可预览或关闭" }
                            else { "包含所有已检测 HID 接口编号与近期读取记录" },
                        if s.software { "Recent activity is summarized automatically; preview or opt out" }
                            else { "All detected HID interface IDs and recent readings" },
                    ),
                    70.0,
                    y + 33.0,
                    398.0,
                    22.0,
                    11.0,
                    false,
                    true,
                );
                self.button(
                    Action::Diagnostics,
                    text("查看内容", "Preview"),
                    "",
                    R::new(480.0, y + 15.0, 90.0, 32.0),
                    Style::Plain,
                    false,
                );
                self.button(
                    Action::ToggleDiagnostics,
                    if s.software { text("附带软件诊断", "Include software diagnostics") }
                        else { text("附带设备诊断", "Include device diagnostics") },
                    "",
                    R::new(589.0, y + 23.0, 33.0, 19.0),
                    Style::Toggle(s.attach_diagnostics),
                    false,
                );
                y += 85.0;
                if !s.software { y = self.capture_form_layout(&s, y); }
                if !s.error.is_empty() {
                    self.button(
                        Action::Fallback,
                        text("复制内容并打开 GitHub", "Copy report & open GitHub"),
                        "",
                        R::new(25.0, y, 280.0, 34.0),
                        Style::Card,
                        false,
                    );
                    y += 44.0;
                }
            }
            View::Diagnostics => {
                self.tag(
                    if s.software { text("软件诊断", "Software diagnostics") }
                        else { text("设备诊断", "Device diagnostics") },
                    R::new(
                        25.0,
                        y,
                        if crate::i18n::english() { 150.0 } else { 96.0 },
                        26.0,
                    ),
                );
                self.glyph(Icon::File, R::new(33.0, y + 6.0, 14.0, 14.0));
                y += 42.0;
                let diagnostics = self.diagnostics();
                let (facts, queries) = diagnostic_rows(&diagnostics);
                let grid_y = y;
                let mut rows = Vec::new();
                y += 15.0;
                for (key, value) in facts {
                    let h = wrapped_height(&value, 460.0, 12.0).max(20.0);
                    rows.push((key, value, y, h));
                    y += h + 11.0;
                }
                let grid_h = y - grid_y + 4.0;
                self.surface(R::new(25.0, grid_y, 610.0, grid_h), true);
                for (key, value, row_y, h) in rows {
                    self.label(key, 42.0, row_y, 104.0, h, 12.0, false, true);
                    self.label(value, 158.0, row_y, 460.0, h, 12.0, false, false);
                }
                y += 20.0;
                self.label(
                    if s.software { text("近期运行记录", "Recent activity") }
                        else { text("读取过程与设备接口清单", "Reading history and HID interfaces") },
                    25.0,
                    y,
                    600.0,
                    20.0,
                    12.0,
                    true,
                    false,
                );
                y += 29.0;
                if queries.is_empty() {
                    self.surface(R::new(25.0, y, 610.0, 42.0), false);
                    self.label(
                        if s.software { text("暂时还没有日志记录，不影响直接提交问题", "No activity logs yet; you can still describe and submit the issue") }
                            else { text("暂时还没有设备沟通记录", "No device communication records yet") },
                        37.0,
                        y + 10.0,
                        580.0,
                        22.0,
                        11.0,
                        false,
                        true,
                    );
                    y += 50.0;
                }
                for query in queries {
                    let h = wrapped_height(&query, 586.0, 11.0).max(20.0) + 20.0;
                    self.surface(R::new(25.0, y, 610.0, h), false);
                    self.label(query, 37.0, y + 10.0, 586.0, h - 20.0, 11.0, false, true);
                    y += h + 8.0;
                }
                self.label(
                    text(
                        if s.software { "近期运行记录摘要与设备接口信息。" }
                            else { "设备接口信息、电量采样及状态变化。" },
                        if s.software { "Recent activity summary and device interface information." }
                            else { "Device interfaces, battery samples and state changes." },
                    ),
                    25.0,
                    y,
                    610.0,
                    30.0,
                    11.0,
                    false,
                    true,
                );
                y += 46.0;
                self.decorations.borrow_mut().push(Decoration::Note(text("提交前可在这里查看将附带的诊断内容。","Review the diagnostics to include with your feedback here.").into(),R::new(25.0,y,610.0,60.0)));
                y += 80.0;
            }
            View::Capture => { y = self.capture_layout(); }
            View::History => {
                if s.entries.is_empty() {
                    self.decorations.borrow_mut().push(Decoration::Center(
                        if s.task == Some(Task::History) {
                            text("正在读取反馈记录…", "Loading feedback history…")
                        } else {
                            text("还没有反馈记录", "No feedback yet")
                        }.into(),
                        R::new(25.0, 30.0, 610.0, 70.0),
                        13.0,
                        false,
                    ));
                    y = 120.0;
                }
                let page = s.history_page.min(s.entries.len().saturating_sub(1) / HISTORY_PAGE_SIZE);
                if s.entries.len() > HISTORY_PAGE_SIZE {
                    if page > 0 { self.button(Action::HistoryPage(page - 1), text("上一页", "Previous"), "",
                        R::new(25.0, y, 100.0, 32.0), Style::Card, false); }
                    self.label(format!("{} / {}", page + 1, s.entries.len().div_ceil(HISTORY_PAGE_SIZE)),
                        270.0, y, 120.0, 32.0, 12.0, false, true);
                    if (page + 1) * HISTORY_PAGE_SIZE < s.entries.len() {
                        self.button(Action::HistoryPage(page + 1), text("下一页", "Next"), "",
                            R::new(535.0, y, 100.0, 32.0), Style::Card, false);
                    }
                    y += 42.0;
                }
                for (i, entry) in s.entries.iter().enumerate().skip(page * HISTORY_PAGE_SIZE).take(HISTORY_PAGE_SIZE) {
                    self.button(
                        Action::Entry(i),
                        entry
                            .description
                            .lines()
                            .next()
                            .unwrap_or(&entry.device.name),
                        format!(
                            "{} · {}",
                            entry.device.name,
                            category_label(&entry.category)
                        ),
                        R::new(25.0, y, 490.0, 103.0),
                        Style::History,
                        false,
                    );
                    self.tag_last(format!(
                        "{} · {}",
                        entry
                            .number
                            .map(|n| format!("#{n}"))
                            .unwrap_or_else(|| text("待确认", "Unconfirmed").into()),
                        short_date(entry.created_at)
                    ));
                    self.button(Action::DeleteLocal(entry.request_id.clone()), text("删除本地记录", "Delete locally"), "",
                        R::new(523.0, y + 34.0, 112.0, 36.0), Style::Card, false);
                    y += 112.0;
                }
                self.label(
                    text(
                        "这里显示在这台电脑提交的反馈。",
                        "Feedback submitted on this computer appears here.",
                    ),
                    25.0,
                    y + 6.0,
                    610.0,
                    26.0,
                    11.0,
                    false,
                    true,
                );
                y += 50.0;
            }
            View::Thread => {
                if let Some(entry) = &s.current {
                    let title = s
                        .thread
                        .as_ref()
                        .map(|t| t.issue.title.as_str())
                        .unwrap_or(entry.device.name.as_str());
                    self.label(title, 25.0, y, 480.0, 34.0, 15.0, true, false);
                    let status = s
                        .thread
                        .as_ref()
                        .map(|t| {
                            if t.issue.state == "closed" {
                                text("已关闭", "Closed")
                            } else if t.comments.is_empty() {
                                text("等待处理", "Pending")
                            } else {
                                text("已回复", "Replied")
                            }
                        })
                        .unwrap_or(text("等待处理", "Pending"));
                    self.tag(status, R::new(535.0, y + 2.0, 100.0, 25.0));
                    y += 42.0;
                    self.label(
                        format!(
                            "{} · {} · {}",
                            entry.device.name,
                            category_label(&entry.category),
                            short_date(entry.created_at)
                        ),
                        25.0,
                        y,
                        610.0,
                        25.0,
                        12.0,
                        false,
                        true,
                    );
                    y += 39.0;
                    let height = wrapped_height(&entry.description, 578.0, 12.0);
                    self.surface(R::new(25.0, y, 610.0, height + 28.0), true);
                    self.label(
                        &entry.description,
                        41.0,
                        y + 14.0,
                        578.0,
                        height,
                        12.0,
                        false,
                        false,
                    );
                    y += height + 43.0;
                    if let Some(thread) = &s.thread {
                        if thread.comments.is_empty() {
                            self.decorations.borrow_mut().push(Decoration::Center(
                                text(
                                    "已收到你的反馈，回复会显示在这里。",
                                    "Your feedback was received. Replies will appear here.",
                                )
                                .into(),
                                R::new(25.0, y, 610.0, 74.0),
                                12.0,
                                false,
                            ));
                            y += 90.0;
                        }
                        for comment in &thread.comments {
                            let height = wrapped_height(&comment.body, 582.0, 12.0);
                            self.surface(R::new(25.0, y, 610.0, height + 70.0), true);
                            self.tag("R", R::new(39.0, y + 14.0, 20.0, 20.0));
                            self.label(
                                text("维护者", "Maintainer"),
                                67.0,
                                y + 14.0,
                                130.0,
                                22.0,
                                11.0,
                                false,
                                true,
                            );
                            self.tag(text("回复", "Reply"), R::new(156.0, y + 13.0, 52.0, 22.0));
                            self.label(
                                comment.created_at.get(..10).unwrap_or(""),
                                488.0,
                                y + 14.0,
                                130.0,
                                22.0,
                                11.0,
                                false,
                                true,
                            );
                            self.label(
                                &comment.body,
                                39.0,
                                y + 46.0,
                                582.0,
                                height,
                                12.0,
                                false,
                                false,
                            );
                            y += height + 85.0;
                        }
                        if thread.comments_truncated {
                            self.label(
                                text(
                                    "更多回复请在 GitHub 查看。",
                                    "View additional replies on GitHub.",
                                ),
                                25.0,
                                y,
                                610.0,
                                25.0,
                                11.0,
                                false,
                                true,
                            );
                            y += 35.0;
                        }
                    } else {
                        self.label(
                            if s.task == Some(Task::Thread) {
                                text("正在读取回复…", "Loading replies…")
                            } else {
                                text("暂时无法读取回复，请点击刷新回复重试。", "Replies are unavailable. Select Refresh replies to try again.")
                            },
                            25.0,
                            y,
                            610.0,
                            30.0,
                            12.0,
                            false,
                            true,
                        );
                        y += 45.0;
                    }
                    self.label(
                        text("补充信息", "Additional details"),
                        25.0,
                        y,
                        610.0,
                        20.0,
                        12.0,
                        true,
                        false,
                    );
                    y += 28.0;
                    self.edit(self.reply.handle, R::new(25.0, y, 610.0, 118.0));
                    y += 138.0;
                }
            }
            View::Success => {
                self.decorations
                    .borrow_mut()
                    .push(Decoration::Check(R::new(302.0, 27.0, 56.0, 56.0)));
                self.decorations.borrow_mut().push(Decoration::Center(
                    text("反馈已提交", "Feedback submitted").into(),
                    R::new(25.0, 100.0, 610.0, 32.0),
                    22.0,
                    true,
                ));
                self.decorations.borrow_mut().push(Decoration::Center(
                    text(
                        "谢谢你的反馈，有新回复时会在“我的反馈”中显示。",
                        "Thank you. Replies will appear in My feedback.",
                    )
                    .into(),
                    R::new(25.0, 145.0, 610.0, 40.0),
                    12.0,
                    false,
                ));
                if let Some(entry) = &s.current {
                    self.button(
                        Action::OpenIssue,
                        &entry.device.name,
                        format!(
                            "#{} · {}",
                            entry.number.unwrap_or(0),
                            category_label(&entry.category)
                        ),
                        R::new(25.0, 208.0, 610.0, 72.0),
                        Style::Card,
                        false,
                    );
                    self.tag_last(text("等待处理", "Pending"));
                }
                y = 310.0;
            }
        }
        if !s.error.is_empty() {
            let h = wrapped_height(&s.error, 610.0, 11.0);
            self.decorations.borrow_mut().push(Decoration::Note(
                s.error.clone(),
                R::new(25.0, y, 610.0, h + 22.0),
            ));
            y += h + 38.0;
        }
        let natural_height = BODY_TOP + y + FOOTER;
        let target = natural_height.min(self.max_height.get()).max(320.0);
        self.height.set(target);
        super::frame::keep_client_size(
            self.window.handle.hwnd().unwrap() as _,
            WIDTH,
            target,
            self.scale.get(),
        );
        self.update_frame();
        let fy = target - FOOTER + 15.0;
        if s.view == View::History && !s.entries.is_empty() {
            self.button(Action::ClearHistory, text("清空本地记录", "Clear local history"), "",
                R::new(25.0, fy, 160.0, 36.0), Style::Plain, true);
        }
        let (primary, label) = match s.view {
            View::Form => (
                Action::Submit,
                if s.task == Some(Task::Submit) {
                    text("提交中…", "Submitting…")
                } else {
                    text("提交反馈", "Submit feedback")
                },
            ),
            View::Diagnostics | View::Capture => (Action::Back, text("返回反馈", "Back to feedback")),
            View::History => (Action::New, text("新建反馈", "New feedback")),
            View::Thread => (
                Action::Reply,
                if s.task == Some(Task::Reply) {
                    text("发送中…", "Sending…")
                } else {
                    text("发送补充", "Send follow-up")
                },
            ),
            View::Success => (Action::History, text("查看我的反馈", "My feedback")),
        };
        let pw = (label_width(label, 12.0)
            + if matches!(primary, Action::Submit) {
                60.0
            } else {
                42.0
            })
        .max(112.0);
        self.button(
            primary,
            label,
            "",
            R::new(635.0 - pw, fy, pw, 36.0),
            Style::Primary,
            true,
        );
        let secondary = match s.view {
            View::Diagnostics | View::Capture => None,
            View::History => Some((Action::History, text("刷新", "Refresh"), Style::Card)),
            View::Thread => Some((
                Action::OpenIssue,
                text("在 GitHub 查看", "View on GitHub"),
                Style::Card,
            )),
            View::Success => Some((Action::New, text("继续反馈", "New feedback"), Style::Card)),
            View::Form => Some((Action::Close, text("取消", "Cancel"), Style::Plain)),
        };
        if let Some((action, label, style)) = secondary {
            let sw = (label_width(label, 12.0) + 44.0).max(64.0);
            self.button(
                action,
                label,
                "",
                R::new(635.0 - pw - 8.0 - sw, fy, sw, 36.0),
                style,
                true,
            );
        }
        self.content_height.set(y.max(self.body_height()));
        drop(s);
        self.layout(true);
        if recenter { self.center(); }
        self.window.invalidate();
    }
    fn update_frame(&self) {
        let hwnd = self.window.handle.hwnd().unwrap();
        let border = colorref(self.colors.borrow().theme.border_strong);
        let native = crate::dwm::set_popup_frame(hwnd as _, border);
        let was_native = self.native_frame.replace(native);
        unsafe {
            if native {
                if !was_native {
                    wu::SetWindowRgn(hwnd, std::ptr::null_mut(), 0);
                }
            } else {
                let (w, h) = self.window.size();
                let diameter = (26.0 * self.scale.get()).round() as i32;
                let region = winapi::um::wingdi::CreateRoundRectRgn(
                    0, 0, w as i32 + 1, h as i32 + 1, diameter, diameter,
                );
                if !region.is_null() && wu::SetWindowRgn(hwnd, region, 0) == 0 {
                    winapi::um::wingdi::DeleteObject(region as _);
                }
            }
        }
    }
    fn update_input_prompts(&self) {
        for (handle, prompt) in [
            (self.brand.handle, text("品牌，如雷柏、迈从", "Brand, e.g. Rapoo or MCHOSE")),
            (self.model.handle, text("完整型号，如 MT760", "Full model, e.g. MT760")),
        ] {
            unsafe {
                wu::SendMessageW(handle.hwnd().unwrap(), 0x1501, 0,
                    crate::win32::to_wide(prompt).as_ptr() as isize);
            }
        }
    }
    fn sync_control(&self, handle: nwg::ControlHandle, rect: Option<R>, label: Option<&str>,
        show: bool, enabled: bool) {
        let hwnd = handle.hwnd().unwrap();
        let changes = self.applied.borrow_mut().entry(hwnd as usize).or_default()
            .update(rect.map(|rect| layout::pixels(rect, self.scale.get())), label, show, enabled);
        let repaint = changes.rect.is_some() || changes.visible == Some(true);
        if let Some((x, y, width, height)) = changes.rect {
            super::canvas::set_window_pos(hwnd as _, x, y, width, height,
                wu::SWP_NOZORDER | wu::SWP_NOACTIVATE | wu::SWP_NOCOPYBITS);
        }
        if let Some(label) = changes.text {
            unsafe { wu::SetWindowTextW(hwnd, crate::win32::to_wide(&label).as_ptr()); }
        }
        if let Some(enabled) = changes.enabled {
            unsafe { wu::EnableWindow(hwnd, i32::from(enabled)); }
        }
        if let Some(show) = changes.visible {
            visible(handle, show);
        }
        if repaint {
            unsafe { wu::InvalidateRect(hwnd, std::ptr::null(), 1); }
        }
    }
    fn invalidate_count(&self) {
        let count = self.decorations.borrow().iter().find_map(|decoration| match decoration {
            Decoration::Count(rect) => Some(*rect),
            _ => None,
        });
        if let Some(mut rect) = count {
            rect.y -= self.scroll.get();
            let (left, top, width, height) = layout::pixels(rect, self.scale.get());
            let damage = winapi::shared::windef::RECT {
                left: left - 1, top: top - 1, right: left + width + 1, bottom: top + height + 1,
            };
            unsafe { wu::InvalidateRect(self.body.handle.hwnd().unwrap(), &damage, 0); }
        }
    }
    fn set_description(&self, value: &str) {
        let updating = self.updating.replace(true);
        self.description.set_text_unix2dos(value);
        self.description_count.set(self.description.text().chars().count());
        self.updating.set(updating);
        self.invalidate_count();
    }
    fn reset_inputs(&self) {
        self.updating.set(true);
        self.set_description("");
        self.brand.set_text("");
        self.model.set_text("");
        self.updating.set(false);
    }
    fn layout(&self, repaint_controls: bool) {
        let scroll = self.scroll.get().clamp(
            0.0,
            (self.content_height.get() - self.body_height()).max(0.0),
        );
        self.scroll.set(scroll);
        self.sync_control(
            self.body.handle,
            Some(R::new(0.0, BODY_TOP, WIDTH, self.body_height())),
            None, true, true,
        );
        let items = self.items.borrow();
        let mut body = 0;
        let mut root = 0;
        let model = self.state.borrow();
        let viewport = R::new(0.0, 0.0, WIDTH, self.body_height());
        for item in items.iter() {
            let button = if item.root {
                let b = &self.header_buttons[root];
                root += 1;
                b
            } else {
                let b = &self.buttons[body];
                body += 1;
                b
            };
            let mut r = item.rect;
            if !item.root {
                r.y -= scroll;
            }
            let show = item.root || layout::intersects(r, viewport);
            self.sync_control(button.handle, show.then_some(r), Some(&item.label), show,
                item.action.enabled(&model));
            if repaint_controls && show {
                unsafe { wu::InvalidateRect(button.handle.hwnd().unwrap(), std::ptr::null(), 0); }
            }
        }
        for b in &self.buttons[body..] {
            self.sync_control(b.handle, None, None, false, true);
        }
        for b in &self.header_buttons[root..] {
            self.sync_control(b.handle, None, None, false, true);
        }
        let editors_enabled = !model.task.is_some_and(Task::locks_editors);
        for handle in [self.description.handle, self.brand.handle, self.model.handle, self.reply.handle] {
            let rect = self.edit_rects.borrow().iter()
                .find(|(editor, _)| *editor == handle).map(|(_, rect)| {
                    let mut rect = *rect;
                    rect.y -= scroll;
                    rect
                }).filter(|rect| layout::intersects(*rect, viewport));
            self.sync_control(handle, rect, None, rect.is_some(), editors_enabled);
            if repaint_controls && rect.is_some() {
                unsafe { wu::InvalidateRect(handle.hwnd().unwrap(), std::ptr::null(), 1); }
            }
        }
        let mut info: wu::SCROLLINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<wu::SCROLLINFO>() as u32;
        info.fMask = wu::SIF_RANGE | wu::SIF_PAGE | wu::SIF_POS;
        info.nMax = self.content_height.get().ceil() as i32 - 1;
        info.nPage = self.body_height().floor() as u32;
        info.nPos = scroll as i32;
        let scrollbar = (info.nMax, info.nPage, info.nPos);
        if self.scrollbar.replace(Some(scrollbar)) != Some(scrollbar) {
            unsafe { wu::SetScrollInfo(
                self.body.handle.hwnd().unwrap(),
                wu::SB_VERT as i32,
                &info,
                1,
            ); }
        }
        unsafe {
            wu::InvalidateRect(self.body.handle.hwnd().unwrap(), std::ptr::null(), 0);
        }
    }
    fn center(&self) {
        let scale = self.scale.get();
        let mut owner = crate::win32::Rect::default();
        super::canvas::get_window_rect(self.owner.hwnd().unwrap() as _, &mut owner);
        let mut info = crate::win32::MonitorInfo {
            cb_size: std::mem::size_of::<crate::win32::MonitorInfo>() as u32,
            ..Default::default()
        };
        let monitor =
            unsafe { crate::win32::MonitorFromWindow(self.owner.hwnd().unwrap() as _, 2) };
        let valid = unsafe { crate::win32::GetMonitorInfoW(monitor, &mut info) } != 0;
        let (w, h) = self.window.size();
        let mut x = owner.left + (owner.width() - w as i32) / 2;
        let mut y = owner.top + (owner.height() - h as i32) / 2;
        if valid {
            x = x.clamp(
                info.rc_work.left,
                (info.rc_work.right - w as i32).max(info.rc_work.left),
            );
            y = y.clamp(
                info.rc_work.top,
                (info.rc_work.bottom - h as i32).max(info.rc_work.top),
            );
        }
        self.window.set_position(x, y);
        set_rect(
            self.overlay.handle,
            R::new(
                owner.left as f32 / scale,
                owner.top as f32 / scale,
                owner.width() as f32 / scale,
                owner.height() as f32 / scale,
            ),
            scale,
        );
    }
    fn set_view(&self, view: View) {
        if view != View::Capture {
            if self.state.borrow().view == View::Capture { self.finish_capture(); }
            else { self.stop_capture_timer(); }
        }
        self.state.borrow_mut().view = view;
        self.scroll.set(0.0);
        self.rebuild();
        self.prepare_software_diagnostics();
        self.prepare_inventory();
    }
    fn prepare_inventory(&self) {
        let generation = {
            let model = self.state.borrow();
            if !model.initialized || !matches!(model.view, View::Form | View::Diagnostics)
                || model.inventory_report.is_some()
                || self.inventory_working.get() { return; }
            model.inventory_generation
        };
        self.inventory_working.set(true);
        let devices = self.state.borrow().devices.clone();
        let results = self.results.clone();
        let notice = self.notice;
        let spawned = std::thread::Builder::new().name("feedback-inventory".into()).spawn(move || {
            let report = std::panic::catch_unwind(|| crate::feedback::inventory::collect_for_devices(&devices))
                .unwrap_or_else(|_| crate::feedback::inventory::DeviceInventory {
                    report: "HID interface inventory: collection failed\n".into(), unknown: Vec::new(),
                });
            results.lock().unwrap_or_else(|error| error.into_inner())
                .push((generation, ResultMessage::DeviceInventory(report)));
            notice.notice();
        });
        if spawned.is_err() {
            self.inventory_working.set(false);
            self.state.borrow_mut().inventory_report = Some("HID interface inventory: worker unavailable\n".into());
        }
    }

    fn inventory_ready(&self) -> bool {
        let model = self.state.borrow();
        !model.attach_diagnostics || model.restored_draft.is_some()
            || model.has_complete_diagnostics() || model.inventory_report.is_some()
    }
    fn prepare_software_diagnostics(&self) {
        let prepare = {
            let model = self.state.borrow();
            model.software && model.view == View::Form && !model.busy()
                && model.diagnostics_override.is_none() && model.restored_draft.is_none()
        };
        if prepare {
            self.work(Task::SoftwareDiagnostics,
                || ResultMessage::SoftwareDiagnostics(crate::feedback::software::collect()));
        }
    }
    fn error(&self, error: impl Into<String>) {
        self.state.borrow_mut().error = error.into();
        self.scroll.set(f32::MAX);
        self.rebuild();
    }
    fn work(&self, kind: Task, task: impl FnOnce() -> ResultMessage + Send + 'static) {
        let Some(generation) = self.state.borrow_mut().begin_task(kind) else { return; };
        let results = self.results.clone();
        let notice = self.notice;
        let spawn = std::thread::Builder::new()
            .name("device-feedback".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task))
                    .unwrap_or(ResultMessage::Failed);
                results
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push((generation, result));
                notice.notice();
            });
        if spawn.is_err() {
            self.state.borrow_mut().finish_task(generation);
            self.state.borrow_mut().error = text(
                "无法启动反馈任务，请重试。",
                "Could not start the feedback task. Please retry.",
            )
            .into();
        }
        self.rebuild();
    }
    fn action(&self, action: Action) {
        self.action_with_submit(action, client::configured().then_some(submit_feedback));
    }
    fn action_with_submit(&self, action: Action,
        submit: Option<impl FnOnce(client::Draft, Option<String>) -> ResultMessage + Send + 'static>) {
        if !action.enabled(&self.state.borrow()) {
            return;
        }
        if let Action::UnknownDevice(index) = &action {
            let mut model = self.state.borrow_mut();
            if model.selected_unknown == Some(*index) && model.unknown_devices.get(*index).is_some() {
                model.menu = false;
                drop(model);
                self.rebuild();
                return;
            }
        }
        if action.navigates() {
            self.remember_reply();
            self.state.borrow_mut().detach_task();
        }
        if matches!(
            action,
            Action::Device(_)
                | Action::UnknownDevice(_)
                | Action::Manual
                | Action::Connection(_)
                | Action::Category(_)
                | Action::ToggleDiagnostics
                | Action::ToggleCapture
                | Action::Example
                | Action::RemoveImage(_)
        ) {
            let mut s = self.state.borrow_mut();
            s.restored_draft = None;
            s.draft_retry = None;
        }
        match action {
            Action::Close => {
                self.finish_capture();
                self.remember_reply();
                self.window.set_visible(false);
                self.overlay.set_visible(false);
                unsafe {
                    wu::EnableWindow(self.owner.hwnd().unwrap(), 1);
                    wu::SetForegroundWindow(self.owner.hwnd().unwrap());
                }
            }
            Action::Back => {
                crate::capture::stop();
                let view = self.state.borrow().view;
                self.remember_reply();
                let target = back_view(view, self.state.borrow().submitted);
                self.set_view(target);
                if target == View::History {
                    self.work(Task::History, || ResultMessage::History(client::load_history()));
                }
            }
            Action::History => {
                self.set_view(View::History);
                self.work(Task::History, || ResultMessage::History(client::load_history()));
            }
            Action::HistoryPage(page) => {
                self.state.borrow_mut().history_page = page;
                self.scroll.set(0.0);
                self.rebuild();
            }
            Action::DeleteLocal(id) => self.delete_local(vec![id]),
            Action::ClearHistory => {
                let ids = self.state.borrow().entries.iter().map(|entry| entry.request_id.clone()).collect();
                self.delete_local(ids);
            }
            Action::Devices => {
                let mut s = self.state.borrow_mut();
                s.menu = !s.menu;
                drop(s);
                self.rebuild();
            }
            Action::Device(i) => {
                if i >= self.state.borrow().devices.len() { return; }
                crate::capture::clear();
                *self.capture_folder.borrow_mut() = None;
                let mut s = self.state.borrow_mut();
                s.selected = Some(i);
                s.selected_unknown = None;
                s.restored_device = None;
                s.capture = None;
                s.attach_capture = true;
                s.connection = connection_index(&s.devices[i].device.connection);
                s.diagnostics_override = None;
                s.draft_retry = None;
                s.menu = false;
                drop(s);
                self.set_view(View::Form);
            }
            Action::UnknownDevice(index) => {
                let Some(unknown) = self.state.borrow().unknown_devices.get(index).cloned() else { return; };
                crate::capture::clear();
                *self.capture_folder.borrow_mut() = None;
                {
                    let mut state = self.state.borrow_mut();
                    state.selected = None;
                    state.selected_unknown = Some(index);
                    state.restored_device = None;
                    state.capture = None;
                    state.attach_capture = true;
                    state.diagnostics_override = None;
                    state.draft_retry = None;
                    state.connection = 3;
                    state.category = 0;
                    state.menu = false;
                }
                self.updating.set(true);
                self.brand.set_text("");
                self.model.set_text(&unknown.name);
                self.updating.set(false);
                self.set_view(View::Form);
            }
            Action::DevicePage(page) => {
                self.state.borrow_mut().device_page = page;
                self.rebuild_position(false);
            }
            Action::Manual => {
                crate::capture::clear();
                *self.capture_folder.borrow_mut() = None;
                let mut s = self.state.borrow_mut();
                s.selected = None;
                let was_unknown = s.selected_unknown.take().is_some();
                s.restored_device = None;
                s.capture = None;
                s.attach_capture = true;
                s.diagnostics_override = None;
                s.draft_retry = None;
                s.menu = false;
                drop(s);
                if was_unknown {
                    self.updating.set(true);
                    self.brand.set_text("");
                    self.model.set_text("");
                    self.updating.set(false);
                }
                self.set_view(View::Form);
            }
            Action::Connection(i) => {
                self.state.borrow_mut().connection = i;
                self.state.borrow_mut().draft_retry = None;
                self.rebuild();
            }
            Action::Category(i) => {
                self.state.borrow_mut().category = i;
                self.state.borrow_mut().draft_retry = None;
                self.rebuild();
            }
            Action::ToggleDiagnostics => {
                let mut s = self.state.borrow_mut();
                s.attach_diagnostics = !s.attach_diagnostics;
                s.draft_retry = None;
                drop(s);
                self.rebuild();
            }
            Action::Diagnostics => {
                self.set_view(View::Diagnostics);
            }
            Action::Capture => { self.set_view(View::Capture); }
            Action::ToggleCapture => {
                let mut state = self.state.borrow_mut();
                state.attach_capture = !state.attach_capture;
                drop(state);
                self.rebuild();
            }
            Action::CaptureStart | Action::CaptureMark(_) | Action::CaptureStop
                | Action::CaptureSave | Action::CaptureClear | Action::CaptureFolder => {
                self.capture_action(action);
            }
            Action::Example => {
                let software = self.state.borrow().software;
                self.set_description(if software {
                    text("操作步骤：打开设置并修改选项。\n实际结果：保存时显示错误，重新打开后设置没有保留。\n预期结果：正常保存设置。\n出现频率：每次都会出现。",
                        "Steps: Open settings and change an option.\nActual: Saving shows an error and the change is lost after reopening.\nExpected: Settings should be saved.\nFrequency: Every time.")
                } else { text("接收器已经连接，官方驱动能看到电量，但这里一直显示未知。唤醒设备后仍然如此。","The receiver is connected and the official driver shows the battery, but X-Battery keeps showing unknown, even after waking the device.") });
                self.state.borrow_mut().error.clear();
                self.rebuild();
            }
            Action::RemoveImage(i) => {
                self.state.borrow_mut().images.remove(i);
                self.state.borrow_mut().draft_retry = None;
                self.rebuild();
            }
            Action::AddImage => self.choose_images(),
            Action::Submit => {
                let draft = self.draft();
                if !self.state.borrow().software
                    && self.state.borrow().draft_retry.is_none()
                    && self.state.borrow().selected.is_none()
                    && self.state.borrow().restored_device.is_none()
                    && (self.brand.text().trim().is_empty() || self.model.text().trim().is_empty())
                {
                    self.error(text(
                        "请填写品牌和型号。",
                        "Enter both the brand and model.",
                    ));
                    return;
                }
                if let Err(error) = client::validate_details(&draft) {
                    self.error(error);
                    return;
                }
                if !self.inventory_ready() {
                    self.prepare_inventory();
                    self.error(text("正在整理设备诊断信息，请稍等片刻；如果不希望附带诊断，也可以取消勾选后直接提交。",
                        "Collecting device diagnostics. Submit shortly, or uncheck diagnostics to submit right away."));
                    return;
                }
                let Some(submit) = submit else {
                    self.error(text("当前版本暂未开通应用内直投，请先复制报告前往 GitHub 提交。",
                        "In-app submission is not enabled in this build. Copy the report and submit it on GitHub."));
                    return;
                };
                let retry = self.state.borrow().draft_retry.clone();
                self.work(Task::Submit, move || submit(draft, retry));
            }
            Action::Fallback => {
                if !self.inventory_ready() {
                    self.prepare_inventory();
                    self.error(text("正在整理设备诊断信息，请稍等片刻；如果不希望附带诊断，也可以取消勾选后直接复制。",
                        "Collecting device diagnostics. Copy shortly, or uncheck diagnostics to copy right away."));
                    return;
                }
                let draft = self.draft();
                self.work(Task::Fallback, move || ResultMessage::Fallback(client::fallback(&draft)));
            }
            Action::Entry(i) => {
                self.remember_reply();
                let entry = self.state.borrow().entries[i].clone();
                self.state.borrow_mut().current = Some(entry.clone());
                if let Some(number) = entry.number {
                    self.reply.set_text_unix2dos(
                        self.state
                            .borrow()
                            .reply_drafts
                            .get(&number)
                            .map(String::as_str)
                            .unwrap_or(""),
                    );
                    self.state.borrow_mut().thread = None;
                    self.set_view(View::Thread);
                    self.work(Task::Thread, move || {
                        let pending = client::load_pending_reply(number);
                        ResultMessage::Thread(client::refresh(number), pending)
                    });
                } else {
                    let id = entry.request_id;
                    self.work(Task::Draft, move || {
                        let draft = client::load_draft(&id);
                        ResultMessage::Draft(id, draft)
                    });
                }
            }
            Action::Refresh => {
                let number = self.state.borrow().current.as_ref().and_then(|e| e.number);
                if let Some(number) = number {
                    self.work(Task::Thread, move || {
                        let pending = client::load_pending_reply(number);
                        ResultMessage::Thread(client::refresh(number), pending)
                    });
                }
            }
            Action::OpenIssue => {
                let url = self
                    .state
                    .borrow()
                    .current
                    .as_ref()
                    .and_then(|e| e.url.clone());
                if let Some(url) = url {
                    if crate::dynlight::shell_open(&url).is_err() {
                        self.error(text("无法打开浏览器。", "Could not open the browser."));
                    }
                }
            }
            Action::Reply => {
                let draft = self.reply.text();
                let body = draft.trim().to_string();
                if body.is_empty() || body.chars().count() > 2000 {
                    self.error(text(
                        "补充说明需为 1–2000 字。",
                        "Follow-ups must be 1–2000 characters.",
                    ));
                    return;
                }
                let number = self.state.borrow().current.as_ref().and_then(|e| e.number);
                if let Some(number) = number {
                    self.work(Task::Reply, move || {
                        let result = client::reply(number, &body);
                        ResultMessage::Reply(number, draft, result)
                    });
                }
            }
            Action::New => {
                crate::capture::clear();
                *self.capture_folder.borrow_mut() = None;
                self.state.borrow_mut().new_form();
                self.reset_inputs();
                self.set_view(View::Form);
            }
        }
    }
    fn remember_reply(&self) {
        let mut s = self.state.borrow_mut();
        if s.view == View::Thread {
            if let Some(number) = s.current.as_ref().and_then(|e| e.number) {
                s.reply_drafts.insert(number, self.reply.text());
            }
        }
    }
    fn delete_local(&self, request_ids: Vec<String>) {
        if request_ids.is_empty() { return; }
        let prompt = text("删除这些本地记录及附件？已提交的 GitHub 议题会保留，应用内的跟进入口将移除。",
            "Delete these local records and attachments? Submitted GitHub issues remain, but their in-app follow-up access will be removed.");
        let prompt: Vec<u16> = prompt.encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = text("删除本地反馈", "Delete local feedback").encode_utf16().chain(Some(0)).collect();
        if unsafe { wu::MessageBoxW(self.window.handle.hwnd().unwrap(), prompt.as_ptr(), title.as_ptr(),
            wu::MB_YESNO | wu::MB_ICONQUESTION | wu::MB_DEFBUTTON2) } != wu::IDYES { return; }
        self.work(Task::HistoryDelete, move || {
            let result = client::delete_local(&request_ids);
            ResultMessage::Deleted(request_ids, result)
        });
    }
    fn choose_images(&self) {
        let remaining = 3usize.saturating_sub(self.state.borrow().images.len());
        if remaining == 0 {
            self.error(text(
                "最多添加 3 张截图。",
                "You can attach up to 3 screenshots.",
            ));
            return;
        }
        let mut picker = nwg::FileDialog::default();
        if nwg::FileDialog::builder()
            .title(text("选择截图", "Choose screenshot"))
            .action(nwg::FileDialogAction::Open)
            .filters("Images(*.png;*.jpg;*.jpeg;*.webp)")
            .build(&mut picker)
            .is_err()
        {
            self.error(text(
                "无法打开文件选择窗口。",
                "Could not open the file picker.",
            ));
            return;
        }
        if !picker.run(Some(&self.window)) {
            return;
        }
        let Ok(path) = picker.get_selected_item() else {
            return;
        };
        let path = std::path::PathBuf::from(path);
        self.work(Task::Images, move || {
            ResultMessage::Images((|| {
                use std::io::Read;
                let file = std::fs::File::open(&path)
                    .map_err(|_| text("无法读取截图。", "Could not read the image.").to_string())?;
                let mut data = Vec::new();
                file.take(5 * 1024 * 1024 + 1)
                    .read_to_end(&mut data)
                    .map_err(|_| text("无法读取截图。", "Could not read the image.").to_string())?;
                let extension = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let mime = match extension.as_str() {
                    "png" => "image/png",
                    "jpg" | "jpeg" => "image/jpeg",
                    "webp" => "image/webp",
                    _ => "",
                };
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("screenshot");
                client::image(name, mime, &data).map(|image| vec![image])
            })())
        });
    }
    fn complete(&self) {
        let messages = std::mem::take(&mut *self.results.lock().unwrap_or_else(|e| e.into_inner()));
        for (generation, message) in messages {
            if let ResultMessage::DeviceInventory(report) = message {
                self.inventory_working.set(false);
                let mut model = self.state.borrow_mut();
                if generation == model.inventory_generation {
                    model.inventory_report = Some(report.report);
                    model.unknown_devices = report.unknown;
                }
                drop(model);
                self.rebuild_position(false);
                self.prepare_inventory();
                continue;
            }
            if let ResultMessage::Reply(number, draft, Ok(_)) = &message {
                self.state.borrow_mut().confirm_reply(*number, draft);
            }
            if !self.state.borrow_mut().finish_task(generation) {
                continue;
            }
            match message {
                ResultMessage::CaptureSaved(result) => match result {
                    Ok(path) => *self.capture_folder.borrow_mut() = Some(path),
                    Err(error) => self.state.borrow_mut().error = error,
                },
                ResultMessage::History(result) => match result {
                    Ok(entries) => self.state.borrow_mut().entries = entries,
                    Err(e) => self.state.borrow_mut().error = e,
                },
                ResultMessage::Deleted(ids, result) => match result {
                    Ok(()) => {
                        let reset = self.state.borrow_mut().remove_local_entries(&ids);
                        if reset {
                            crate::capture::clear();
                            *self.capture_folder.borrow_mut() = None;
                            self.reset_inputs();
                        }
                    }
                    Err(error) => self.state.borrow_mut().error = error,
                },
                ResultMessage::Submitted(result, pending) => match result {
                    Ok(outcome) => {
                        let mut s = self.state.borrow_mut();
                        let request_id = outcome.entry.request_id.clone();
                        s.record_entry(outcome.entry);
                        if outcome.pending {
                            s.draft_retry = Some(request_id);
                            s.error=text("反馈结果暂未确认，请稍后重试。","The feedback result is not confirmed yet. Please retry shortly.").into();
                        } else {
                            s.view = View::Success;
                            s.submitted = true;
                            s.draft_retry = None;
                            s.restored_draft = None;
                            s.restored_device = None;
                            s.images.clear();
                            s.capture = None;
                            crate::capture::clear();
                            *self.capture_folder.borrow_mut() = None;
                            self.scroll.set(0.0);
                        }
                    }
                    Err(e) => {
                        let mut s = self.state.borrow_mut();
                        s.error = e;
                        s.draft_retry = pending;
                    }
                },
                ResultMessage::Thread(result, pending) => {
                    match pending {
                        Ok(Some(pending)) if self.reply.text().trim().is_empty() => {
                            self.reply.set_text_unix2dos(&pending)
                        }
                        Err(e) => self.state.borrow_mut().error = e,
                        _ => {}
                    }
                    match result {
                        Ok(thread) => {
                            self.state.borrow_mut().thread = Some(thread);
                        }
                        Err(e) => self.state.borrow_mut().error = e,
                    }
                }
                ResultMessage::Reply(_, _, result) => match result {
                    Ok(_) => {
                        self.reply.set_text("");
                        self.action(Action::Refresh);
                    }
                    Err(e) => self.state.borrow_mut().error = e,
                },
                ResultMessage::Draft(id, result) => match result {
                    Ok(mut draft) => {
                        self.updating.set(true);
                        self.set_description(&draft.description);
                        self.brand
                            .set_text(draft.device.brand.as_deref().unwrap_or(""));
                        self.model
                            .set_text(draft.device.model.as_deref().unwrap_or(&draft.device.name));
                        self.updating.set(false);
                        let images = std::mem::take(&mut draft.images);
                        let capture = draft.capture.take();
                        self.stop_capture_timer();
                        crate::capture::clear();
                        *self.capture_folder.borrow_mut() = None;
                        self.state.borrow_mut().restored_draft = Some(draft.clone());
                        let mut s = self.state.borrow_mut();
                        s.draft_retry = Some(id);
                        s.software = crate::feedback::software::is_device(&draft.device);
                        s.latest_software = s.software;
                        s.selected = None;
                        s.selected_unknown = None;
                        s.restored_device = Some(draft.device.clone());
                        s.connection = connection_index(&draft.device.connection);
                        s.category = ["device", "battery", "dpi", "other"]
                            .iter()
                            .position(|c| *c == draft.category)
                            .unwrap_or(3);
                        s.attach_diagnostics = draft.diagnostics.is_some();
                        s.diagnostics_override = draft.diagnostics.map(DiagnosticReport::Complete);
                        s.inventory_report = None;
                        s.unknown_devices.clear();
                        s.inventory_generation = s.inventory_generation.wrapping_add(1);
                        s.images = images;
                        s.capture = capture;
                        s.attach_capture = true;
                        s.submitted = false;
                        s.view = View::Form;
                        drop(s);
                        self.prepare_inventory();
                    }
                    Err(e) => self.state.borrow_mut().error = e,
                },
                ResultMessage::Images(result) => match result {
                    Ok(images) => {
                        let mut s = self.state.borrow_mut();
                        s.images.extend(images);
                        s.draft_retry = None;
                        s.restored_draft = None;
                    }
                    Err(e) => self.state.borrow_mut().error = e,
                },
                ResultMessage::SoftwareDiagnostics(report) => {
                    self.state.borrow_mut().diagnostics_override = Some(DiagnosticReport::Summary(report));
                }
                ResultMessage::DeviceInventory(_) => unreachable!(),
                ResultMessage::Fallback(result) => match result {
                    Ok(fallback) => {
                        let error = if !copy_report(self.window.handle, &fallback.markdown) {
                            text("暂时无法复制，请重试。", "Could not copy the report. Please retry.")
                        } else if crate::dynlight::shell_open(&fallback.url).is_err() {
                            text("内容已复制到剪贴板，但没能自动打开浏览器，请手动访问 GitHub 提交。",
                                "Report copied, but the browser could not be opened automatically. Submit it on GitHub.")
                        } else if fallback.images_omitted || fallback.capture_omitted {
                            text("内容已复制。请在 GitHub 粘贴，并手动添加截图或采集文件。", "Report copied. Paste it on GitHub and attach your screenshots or capture file manually.")
                        } else {
                            text("内容已复制，请在 GitHub 确认并提交。", "Report copied. Review and submit it on GitHub.")
                        };
                        self.state.borrow_mut().error = error.into();
                    }
                    Err(error) => self.state.borrow_mut().error = error,
                },
                ResultMessage::Failed => {
                    self.state.borrow_mut().error = text("反馈处理未完成，请重试。", "Feedback processing did not finish. Please retry.").into();
                }
            }
            if !self.state.borrow().error.is_empty() {
                self.scroll.set(f32::MAX);
            }
            self.rebuild();
        }
    }
}

fn submit_feedback(draft: client::Draft, retry: Option<String>) -> ResultMessage {
    let result = if let Some(id) = &retry {
        client::retry(id)
    } else {
        client::submit(draft.clone())
    };
    let pending = if result.is_err() {
        retry.or_else(|| {
            client::find_pending(&draft)
                .ok()
                .flatten()
                .map(|entry| entry.request_id)
        })
    } else {
        None
    };
    ResultMessage::Submitted(result, pending)
}

fn connection_index(value: &str) -> usize {
    match value {
        "2.4GHz" | "Wireless24G" => 0,
        "USB" | "Wired" => 1,
        "Bluetooth" => 2,
        _ => 3,
    }
}

fn label_width(value: &str, size: f32) -> f32 {
    value
        .chars()
        .map(|c| if c.is_ascii() { size * 0.59 } else { size })
        .sum::<f32>()
}
fn wrapped_height(value: &str, width: f32, size: f32) -> f32 {
    let lines = value
        .lines()
        .map(|line| {
            (label_width(line, size) / (width - 12.0).max(size))
                .ceil()
                .max(1.0) as usize
        })
        .sum::<usize>()
        .max(1);
    lines as f32 * size * 1.8 + 2.0
}
fn kind(value: &str) -> crate::tray_prefs::TrayKind {
    use crate::tray_prefs::TrayKind::*;
    match value {
        "mouse" => Mouse,
        "keyboard" => Keyboard,
        "headset" => Headset,
        "gamepad" => Gamepad,
        _ => Generic,
    }
}
fn device_icon(p: &mut Painter, device: crate::tray_prefs::TrayKind, r: R, color: u32) {
    use crate::tray_prefs::TrayKind::*;
    let icon = match device {
        Mouse => Some(Icon::Mouse),
        Keyboard => Some(Icon::Keyboard),
        Headset => Some(Icon::Headset),
        Generic | Auto => Some(Icon::Plus),
        Gamepad => None,
    };
    if let Some(icon) = icon {
        feedback_icons::draw(p, icon, r, color);
    } else {
        super::panel_icons::draw(p, super::panel_icons::Icon::Device(device), r, color, 0.0);
    }
}
fn candidate_identifier(device: &Candidate) -> String {
    device.interface.as_ref().map(|interface|
        format!("VID {:04X} · PID {:04X}", interface.vendor_id, interface.product_id))
        .unwrap_or_else(|| "VID/PID —".into())
}

fn unknown_identifier(device: &crate::feedback::inventory::UnknownDevice) -> String {
    let hex = |value: Option<u16>| value.map(|value| format!("{value:04X}")).unwrap_or_else(|| "—".into());
    format!("VID {} · PID {}", hex(device.vendor_id), hex(device.product_id))
}

fn unknown_name(device: &crate::feedback::inventory::UnknownDevice) -> &str {
    if device.name.is_empty() { text("未知设备", "Unknown device") } else { &device.name }
}

fn device_note(device: &Candidate) -> String {
    let status = if device.battery == "—" {
        text(
            "已检测到设备 · 电量未知",
            "Device detected · Battery unknown",
        )
        .into()
    } else {
        format!(
            "{} · {} {}",
            text("已连接", "Connected"),
            text("电量", "Battery"),
            device.battery
        )
    };
    format!("{status} · {}", candidate_identifier(device))
}
fn short_date(milliseconds: u64) -> String {
    let days = (milliseconds / 86_400_000) as i64;
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}
fn diagnostic_rows(value: &str) -> (Vec<(String, String)>, Vec<String>) {
    let mut fields = Vec::new();
    let mut queries = Vec::new();
    let mut querying = false;
    for line in value.lines().filter(|line| !line.trim().is_empty()) {
        if line.starts_with("Protocol query summary") || line == "Software log summary"
            || line.starts_with("Reading timeline") || line.starts_with("HID interface inventory") {
            querying = true;
            queries.push(line.to_string());
            continue;
        }
        if querying && !line.starts_with("Selected connection:") {
            queries.push(line.to_string());
            continue;
        }
        if let Some((key, value)) = line.split_once(": ") {
            let key = match key {
                "App" => text("应用版本", "App version"),
                "System" => text("系统版本", "System version"),
                "Distribution" => text("运行版本", "Distribution"),
                "Log" => text("运行记录", "Activity log"),
                "Model" => text("设备名称", "Device name"),
                "Type" => text("设备类型", "Device type"),
                "Connection" => text("检测连接", "Detected link"),
                "Selected connection" => text("连接方式", "Connection"),
                "Battery" => text("电量状态", "Battery"),
                "Reading" => text("读数状态", "Reading"),
                "Connected" => text("连接状态", "Connected"),
                "VID/PID" => "VID / PID",
                "Provider" => text("读取器", "Provider"),
                "Usage" => text("接口用途", "Usage"),
                "Report bytes (input/output/feature)" => text("报告字节数", "Report bytes"),
                other => other,
            };
            let value = match value {
                "mouse" => text("鼠标", "Mouse"),
                "keyboard" => text("键盘", "Keyboard"),
                "headset" => text("耳机", "Headset"),
                "gamepad" => text("手柄", "Gamepad"),
                "other" => text("其他", "Other"),
                "unknown" => text("待确认", "Unknown"),
                "true" => text("已连接", "Connected"),
                "false" => text("未连接", "Disconnected"),
                "Live" => text("实时", "Live"),
                "Cached" => text("缓存", "Cached"),
                "Stale" => text("已过期", "Expired"),
                "Unknown" => text("未知", "Unknown"),
                other => other,
            };
            fields.push((key.to_string(), value.to_string()));
        } else {
            fields.push((text("备注", "Note").into(), line.into()));
        }
    }
    (fields, queries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regression_dialog() -> (nwg::Window, nwg::Notice, Dialog) {
        regression_dialog_with_capture_refresh(|| {})
    }

    fn regression_dialog_with_capture_refresh(refresh: impl Fn() + 'static) -> (nwg::Window, nwg::Notice, Dialog) {
        nwg::init().unwrap();
        let mut owner = nwg::Window::default();
        nwg::Window::builder().title("Feedback regression")
            .flags(nwg::WindowFlags::POPUP).size((760, 680)).build(&mut owner).unwrap();
        let mut notice = nwg::Notice::default();
        nwg::Notice::builder().parent(&owner).build(&mut notice).unwrap();
        let dialog = Dialog::new(&owner, notice.sender(), refresh).unwrap();
        (owner, notice, dialog)
    }

    fn offscreen_regression_dialog() -> (nwg::Window, nwg::Notice, Dialog) {
        let (owner, notice, mut dialog) = regression_dialog();
        let handle = dialog.controls.window.handle;
        dialog.raw.push(nwg::bind_raw_event_handler(&handle, 0x1015b, |_, message, _, data| {
            if message == wu::WM_WINDOWPOSCHANGING {
                let position = unsafe { &mut *(data as *mut wu::WINDOWPOS) };
                position.x = -32000;
                position.y = -32000;
                position.flags &= !wu::SWP_NOMOVE;
            }
            None
        }).unwrap());
        unsafe { wu::ShowWindow(handle.hwnd().unwrap(), wu::SW_SHOWNOACTIVATE); }
        (owner, notice, dialog)
    }

    fn update_rect(handle: nwg::ControlHandle) -> (i32, i32, i32, i32) {
        let mut rect = unsafe { std::mem::zeroed() };
        unsafe { wu::GetUpdateRect(handle.hwnd().unwrap(), &mut rect, 0); }
        (rect.left, rect.top, rect.right, rect.bottom)
    }

    #[test]
    fn feedback_children_clip_siblings_during_layout_changes() {
        let (_owner, _notice, dialog) = regression_dialog();
        let controls = &dialog.controls;
        for handle in [controls.body.handle, controls.description.handle, controls.reply.handle,
            controls.brand.handle, controls.model.handle].into_iter()
            .chain(controls.buttons.iter().map(|button| button.handle))
            .chain(controls.header_buttons.iter().map(|button| button.handle)) {
            let style = super::super::canvas::get_window_long(handle.hwnd().unwrap() as _, -16);
            assert_ne!(style & wu::WS_CLIPSIBLINGS as isize, 0);
        }
    }

    #[test]
    fn moving_feedback_editors_invalidates_their_entire_client_area() {
        let (_owner, _notice, dialog) = offscreen_regression_dialog();
        let controls = &dialog.controls;
        let handle = controls.description.handle;
        let mut rect = controls.edit_rects.borrow().iter()
            .find(|(editor, _)| *editor == handle).unwrap().1;
        let (_, _, width, height) = layout::pixels(rect, controls.scale.get());
        unsafe { wu::ValidateRect(handle.hwnd().unwrap(), std::ptr::null()); }
        assert_eq!(update_rect(handle), (0, 0, 0, 0));
        rect.y -= 25.0;
        controls.sync_control(handle, Some(rect), None, true, true);
        assert_eq!(update_rect(handle), (0, 0, width, height));
    }

    #[test]
    fn feedback_rebuild_repaints_editors_without_changing_text_or_selection() {
        let (_owner, _notice, dialog) = offscreen_regression_dialog();
        let controls = &dialog.controls;
        let handle = controls.description.handle;
        let value = "反馈重绘测试\r\n内容不能丢失";
        controls.set_description(value);
        unsafe { wu::SendMessageW(handle.hwnd().unwrap(), wu::EM_SETSEL as u32, 1, 4); }
        for menu in [true, false, true, false] {
            controls.state.borrow_mut().menu = menu;
            controls.rebuild();
        }
        unsafe { wu::ValidateRect(handle.hwnd().unwrap(), std::ptr::null()); }
        controls.state.borrow_mut().category = 1;
        controls.rebuild();
        let mut rect = unsafe { std::mem::zeroed() };
        unsafe { wu::GetClientRect(handle.hwnd().unwrap(), &mut rect); }
        assert_eq!(update_rect(handle), (0, 0, rect.right, rect.bottom));
        assert_eq!(controls.description.text(), value);
        assert_eq!(controls.description_count.get(), value.chars().count());
        let mut start = 0u32;
        let mut end = 0u32;
        unsafe { wu::SendMessageW(handle.hwnd().unwrap(), wu::EM_GETSEL as u32,
            &mut start as *mut u32 as usize, &mut end as *mut u32 as isize); }
        assert_eq!((start, end), (1, 4));
    }

    #[test]
    fn software_feedback_uses_a_short_form_and_attaches_only_the_previewed_summary() {
        let (_owner, _notice, dialog) = regression_dialog();
        let controls = &dialog.controls;
        controls.state.borrow_mut().reopen_software();
        controls.rebuild();
        controls.set_description("修改设置后保存失败，重启后未保留。");
        let generation = controls.state.borrow_mut().begin_task(Task::SoftwareDiagnostics).unwrap();
        let report = "App: X-Battery test\nSoftware log summary\noperation=settings_save_failed; os_error=17\n";
        let inventory = "HID interface inventory: complete; total=1\nVID/PID=24AE:1415\n";
        controls.state.borrow_mut().inventory_report = Some(inventory.into());
        controls.results.lock().unwrap().push((generation, ResultMessage::SoftwareDiagnostics(report.into())));
        controls.complete();
        let draft = controls.draft();
        assert_eq!(draft.category, "other");
        assert!(crate::feedback::software::is_device(&draft.device));
        let expected = crate::feedback::with_inventory(report, inventory);
        assert_eq!(draft.diagnostics.as_deref(), Some(expected.as_str()));
        assert!(client::validate_details(&draft).is_ok());
        assert!(!controls.items.borrow().iter().any(|item|
            matches!(item.action, Action::Devices | Action::Device(_) | Action::Connection(_) | Action::Category(_))));
        controls.action(Action::Diagnostics);
        assert_eq!(controls.state.borrow().view, View::Diagnostics);
        assert_eq!(controls.diagnostics(), expected);
        controls.action(Action::Back);
        controls.action(Action::ToggleDiagnostics);
        assert!(controls.draft().diagnostics.is_none());
        assert_eq!(controls.description.text(), "修改设置后保存失败，重启后未保留。");
    }

    fn submit_offline(controls: &Controls) -> client::Draft {
        let (sender, receiver) = std::sync::mpsc::channel();
        controls.action_with_submit(Action::Submit, Some(move |draft: client::Draft, retry: Option<String>| {
            assert!(retry.is_none());
            let entry = client::Entry {
                request_id: "12345678-1234-4234-9234-123456789abc".into(),
                created_at: 1, updated_at: 1, category: draft.category.clone(),
                device: draft.device.clone(), description: draft.description.clone(),
                status: client::Status::Completed, number: Some(123),
                url: Some("https://github.com/youmikk/X-Battery/issues/123".into()),
            };
            sender.send(draft).unwrap();
            ResultMessage::Submitted(Ok(client::SubmitOutcome { entry, pending: false }), None)
        }));
        assert!(controls.state.borrow().error.is_empty(), "{}", controls.state.borrow().error);
        let draft = receiver.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while controls.state.borrow().busy() {
            controls.complete();
            assert!(std::time::Instant::now() < deadline, "Submission did not complete");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(controls.state.borrow().view, View::Success);
        assert!(controls.state.borrow().submitted);
        draft
    }

    #[test]
    fn unknown_device_feedback_includes_all_interfaces_and_has_named_required_fields() {
        let (_owner, _notice, dialog) = regression_dialog();
        let controls = &dialog.controls;
        controls.state.borrow_mut().reopen(Vec::new(), false);
        let inventory = "HID interface inventory: complete\nVID/PID=24AE:1415\nVID/PID=046D:C52B\n";
        controls.state.borrow_mut().inventory_report = Some(inventory.into());
        controls.brand.set_text("Rapoo");
        controls.model.set_text("VT1 PRO MAX V2");
        controls.set_description("连接后没有电量，官方驱动可以显示。");
        controls.rebuild();
        for label in [text("设备品牌（必填）", "Brand (required)"), text("完整型号（必填）", "Full model (required)")] {
            assert!(controls.labels.borrow().iter().any(|item| item.text == label));
        }
        assert_eq!(controls.state.borrow().connection, 3);
        let preview = controls.diagnostics();
        let draft = submit_offline(controls);
        assert_eq!(draft.device.brand.as_deref(), Some("Rapoo"));
        assert_eq!(draft.device.model.as_deref(), Some("VT1 PRO MAX V2"));
        assert_eq!(draft.diagnostics.as_deref(), Some(preview.as_str()));
        assert!(preview.contains("VID/PID=24AE:1415"));
        assert!(preview.contains("VID/PID=046D:C52B"));
    }

    #[test]
    fn editing_a_restored_draft_preserves_its_complete_diagnostics() {
        for software in [false, true] {
            let (_owner, _notice, dialog) = regression_dialog();
            let controls = &dialog.controls;
            if software { controls.state.borrow_mut().reopen_software(); }
            else { controls.state.borrow_mut().reopen(Vec::new(), false); }
            controls.brand.set_text("Rapoo");
            controls.model.set_text("VT1");
            controls.set_description("Battery disappears after the device sleeps.");
            let original = "App: X-Battery test\nSelected connection: USB\nHID interface inventory\nVID/PID=24AE:1415\n";
            let mut draft = controls.draft();
            draft.diagnostics = Some(original.into());
            let generation = controls.state.borrow_mut().begin_task(Task::Draft).unwrap();
            controls.results.lock().unwrap().push((generation,
                ResultMessage::Draft("pending-request".into(), Ok(draft))));
            controls.complete();
            controls.set_description("Battery disappears after every sleep and reconnect.");
            controls.state.borrow_mut().edit_draft();
            assert!(controls.state.borrow().restored_draft.is_none());
            assert!(controls.inventory_ready());
            controls.inventory_working.set(true);
            let inventory_generation = controls.state.borrow().inventory_generation;
            controls.results.lock().unwrap().push((inventory_generation,
                ResultMessage::DeviceInventory(crate::feedback::inventory::DeviceInventory {
                    report: "HID interface inventory\nVID/PID=046D:C52B\n".into(),
                    unknown: vec![unknown_device("USB Receiver", 0x046d, 0xc52b)],
                })));
            controls.complete();
            assert_eq!(controls.state.borrow().unknown_devices.len(), 1);
            if !software {
                controls.action(Action::Devices);
                assert!(controls.items.borrow().iter().any(|item| matches!(item.action, Action::UnknownDevice(0))));
                assert!(!controls.labels.borrow().iter().any(|label|
                    label.text == text("正在查找其他设备…", "Finding other devices…")));
            }
            controls.action(Action::ToggleDiagnostics);
            assert!(controls.draft().diagnostics.is_none());
            controls.action(Action::ToggleDiagnostics);
            let preview = controls.diagnostics();
            assert_eq!(preview, original);
            assert_eq!(submit_offline(controls).diagnostics.as_deref(), Some(original));
            controls.state.borrow_mut().new_form();
            assert!(!controls.state.borrow().has_complete_diagnostics());
            assert!(controls.state.borrow().inventory_report.is_none());
        }
    }

    #[test]
    fn local_capture_is_visible_on_form_and_submits_as_optional_attachment() {
        let requests = Rc::new(Cell::new(0));
        let refresh_requests = requests.clone();
        let (_owner, _notice, dialog) = regression_dialog_with_capture_refresh(move || {
            assert!(crate::capture::status().running);
            refresh_requests.set(refresh_requests.get() + 1);
            crate::capture::record("capture-regression-path", "input", &[0xde, 0xad, 0xbe, 0xef], true);
        });
        let controls = &dialog.controls;
        let interface = crate::hid::HidInterface { path: "capture-regression-path".into(),
            vendor_id: 0x1234, product_id: 0x5678, usage_page: 0xff00, usage: 1,
            input_len: 64, output_len: 64, feature_len: 0, product: "Test".into(), serial: "private".into() };
        let candidate = Candidate { interface: Some(interface.clone()), capture_interface: Some(interface.clone()),
            label: "Test".into(), battery: "50%".into(), diagnostics: "Model: Test\n".into(),
            device: client::Device { name: "Test".into(), family: "mouse".into(),
                connection: "USB".into(), brand: None, model: None } };
        controls.state.borrow_mut().reopen(vec![candidate], false);
        controls.state.borrow_mut().inventory_report = Some("HID interface inventory\n".into());
        controls.set_description("Battery does not update after charging.");
        let before = controls.diagnostics();
        controls.rebuild();
        {
            let items = controls.items.borrow();
            let diagnostics = items.iter().find(|item| matches!(item.action, Action::Diagnostics)).unwrap();
            let capture = items.iter().find(|item| matches!(item.action, Action::Capture)).unwrap();
            assert!(capture.rect.y > diagnostics.rect.bottom());
        }
        controls.action(Action::Capture);
        assert_eq!(requests.get(), 0);
        controls.action(Action::CaptureStart);
        assert_eq!(requests.get(), 1);
        assert!(crate::capture::status().running);
        controls.action(Action::CaptureMark(1));
        controls.window.set_position(185, 163);
        let position = controls.window.position();
        unsafe { wu::SendMessageW(controls.window.handle.hwnd().unwrap(), wu::WM_TIMER, capture::TIMER, 0); }
        assert_eq!(controls.window.position(), position);
        assert_eq!(crate::capture::status().frames, 1);
        assert!(controls.labels.borrow().iter().any(|label| label.text.contains("DEADBEEF")));
        controls.action(Action::Back);
        assert_eq!(requests.get(), 1);
        assert!(!crate::capture::status().running);
        assert_eq!(controls.diagnostics(), before);
        assert_eq!(controls.draft().diagnostics.as_deref(), Some(before.as_str()));
        let capture = controls.draft().capture.unwrap();
        assert_eq!(capture.name, "raw.jsonl");
        assert!(controls.items.borrow().iter().any(|item| matches!(item.action, Action::ToggleCapture)));
        assert!(controls.items.borrow().iter().any(|item| matches!(item.action, Action::Capture)
            && item.label == text("查看记录", "View capture")));
        assert!(client::validate_draft(&controls.draft()).is_ok());
        controls.action(Action::ToggleCapture);
        assert!(controls.draft().capture.is_none());
        controls.action(Action::ToggleCapture);
        assert_eq!(controls.draft().capture.as_ref(), Some(&capture));
        controls.action(Action::ToggleDiagnostics);
        let submitted = submit_offline(controls);
        assert_eq!(submitted.capture, Some(capture));
        assert!(submitted.diagnostics.is_none());
        assert!(controls.state.borrow().capture.is_none());
        assert!(!crate::capture::status().started);
        controls.action(Action::New);
        assert!(!crate::capture::status().started);
    }

    #[test]
    fn empty_capture_shows_its_result_and_can_retry_before_attaching_data() {
        let requests = Rc::new(Cell::new(0));
        let refresh_requests = requests.clone();
        let (_owner, _notice, dialog) = regression_dialog_with_capture_refresh(move || {
            assert!(crate::capture::status().running);
            refresh_requests.set(refresh_requests.get() + 1);
        });
        let controls = &dialog.controls;
        let interface = crate::hid::HidInterface { path: "capture-first-device".into(),
            vendor_id: 0x1234, product_id: 0x5678, usage_page: 0xff00, usage: 1,
            input_len: 64, output_len: 64, feature_len: 0, product: "Test".into(), serial: String::new() };
        let candidate = Candidate { interface: Some(interface.clone()), capture_interface: Some(interface.clone()),
            label: "Test".into(), battery: "50%".into(), diagnostics: "Model: Test\n".into(),
            device: client::Device { name: "Test".into(), family: "mouse".into(),
                connection: "USB".into(), brand: None, model: None } };
        controls.state.borrow_mut().reopen(vec![candidate.clone(), candidate], false);
        controls.state.borrow_mut().inventory_report = Some("HID interface inventory\n".into());
        controls.action(Action::Capture);
        controls.action(Action::CaptureStart);
        assert_eq!(requests.get(), 1);
        for timeout in [false, true] {
            if timeout { crate::capture::record(&interface.path, "input-timeout", &[], false); }
            controls.action(Action::CaptureStop);
            assert!(controls.items.borrow().iter().any(|item| matches!(item.action, Action::CaptureStart)
                && item.label == text("重新采集", "Try capture again")));
            controls.action(Action::Back);
            assert!(controls.draft().capture.is_none());
            assert!(!controls.items.borrow().iter().any(|item| matches!(item.action, Action::ToggleCapture)));
            assert!(controls.labels.borrow().iter().any(|label|
                label.text == text("本次未采集到通信数据，可重新采集", "No traffic was captured; try again")));
            assert!(controls.items.borrow().iter().any(|item| matches!(item.action, Action::Capture)
                && item.label == text("查看结果", "View result")));
            controls.action(Action::Capture);
            *controls.capture_folder.borrow_mut() = Some(std::path::PathBuf::from("previous-empty-capture"));
            controls.action(Action::CaptureStart);
            assert!(controls.state.borrow().error.is_empty());
            assert!(crate::capture::status().running);
            assert_eq!(crate::capture::status().frames, 0);
            assert!(controls.capture_folder.borrow().is_none());
        }
        assert_eq!(requests.get(), 3);
        crate::capture::record(&interface.path, "input", &[1, 2], true);
        controls.action(Action::CaptureStart);
        assert_eq!(requests.get(), 3);
        assert_eq!(crate::capture::status().data_frames, 1);
        controls.action(Action::Close);
        assert!(controls.draft().capture.is_some());
        controls.action(Action::Back);
        controls.action(Action::Device(1));
        assert!(controls.draft().capture.is_none());
        assert!(!crate::capture::status().started);
        controls.action(Action::Manual);
        assert!(!Action::Capture.enabled(&controls.state.borrow()));
    }

    #[test]
    fn restored_capture_survives_edits_and_can_be_excluded_or_cleared() {
        let (_owner, _notice, dialog) = regression_dialog();
        let controls = &dialog.controls;
        controls.state.borrow_mut().reopen(Vec::new(), false);
        controls.brand.set_text("Example");
        controls.model.set_text("Mouse");
        controls.set_description("Device battery is not displayed.");
        let interface = crate::hid::HidInterface { path: "capture-restored".into(),
            vendor_id: 0x1234, product_id: 0x5678, usage_page: 0xff00, usage: 1,
            input_len: 64, output_len: 64, feature_len: 0, product: String::new(), serial: String::new() };
        crate::capture::start(&interface).unwrap();
        crate::capture::record(&interface.path, "input", &[1, 2], true);
        crate::capture::stop();
        let raw = crate::capture::feedback_data(&interface.path).unwrap();
        let capture = client::capture(raw.as_bytes()).unwrap();
        let mut draft = controls.draft();
        draft.device = client::Device { name: "Disconnected mouse".into(), family: "mouse".into(),
            connection: "USB".into(), brand: None, model: None };
        let original_device = draft.device.clone();
        draft.capture = Some(capture.clone());
        let generation = controls.state.borrow_mut().begin_task(Task::Draft).unwrap();
        controls.results.lock().unwrap().push((generation,
            ResultMessage::Draft("pending-capture".into(), Ok(draft))));
        controls.complete();
        assert!(!crate::capture::status().started);
        assert_eq!(controls.draft().device, original_device);
        assert!(controls.brand.text().is_empty());
        assert!(!controls.edit_rects.borrow().iter().any(|(handle, _)| *handle == controls.brand.handle));
        assert_eq!(controls.draft().capture.as_ref(), Some(&capture));
        controls.set_description("Added details about the battery issue.");
        controls.state.borrow_mut().edit_draft();
        assert_eq!(controls.draft().capture.as_ref(), Some(&capture));
        controls.action(Action::Capture);
        assert!(controls.labels.borrow().iter().any(|label| label.text.contains("0102")));
        assert!(!controls.items.borrow().iter().any(|item| matches!(item.action, Action::CaptureStart)));
        controls.action(Action::Back);
        controls.action(Action::ToggleCapture);
        assert!(controls.draft().capture.is_none());
        assert_eq!(controls.draft().device, original_device);
        controls.action(Action::ToggleCapture);
        assert_eq!(controls.draft().capture.as_ref(), Some(&capture));
        assert_eq!(controls.draft().device, original_device);
        controls.action(Action::Capture);
        controls.action(Action::CaptureClear);
        assert!(controls.draft().capture.is_none());
        assert_eq!(controls.draft().device, original_device);
        controls.action(Action::Back);
        let submitted = submit_offline(controls);
        assert_eq!(submitted.device, original_device);
        assert!(submitted.capture.is_none());
        controls.action(Action::New);
        assert!(controls.state.borrow().restored_device.is_none());
    }

    #[test]
    fn capture_export_cannot_be_detached_by_navigation_or_reopening() {
        let (_owner, _notice, dialog) = regression_dialog();
        let controls = &dialog.controls;
        controls.state.borrow_mut().reopen(Vec::new(), false);
        controls.set_view(View::Capture);
        let generation = controls.state.borrow_mut().begin_task(Task::CaptureExport).unwrap();
        for action in [Action::Back, Action::New, Action::History] {
            controls.action(action);
            assert_eq!(controls.state.borrow().view, View::Capture);
            assert_eq!(controls.state.borrow().task, Some(Task::CaptureExport));
        }
        assert!(Action::Close.enabled(&controls.state.borrow()));
        assert!(!controls.state.borrow_mut().reopen_software());
        assert!(!controls.state.borrow().software);
        controls.results.lock().unwrap().push((generation,
            ResultMessage::CaptureSaved(Err("Export failed; partial files removed".into()))));
        controls.complete();
        assert!(!controls.state.borrow().busy());
        assert!(controls.state.borrow().error.contains("Export failed"));
        controls.action(Action::Back);
        assert_eq!(controls.state.borrow().view, View::Form);
        assert!(controls.state.borrow().error.contains("Export failed"));
    }

    #[test]
    fn detected_device_feedback_also_includes_other_hid_interfaces() {
        let (_owner, _notice, dialog) = regression_dialog();
        let controls = &dialog.controls;
        let candidate = Candidate {
            interface: None,
            capture_interface: None,
            label: "Detected mouse".into(), battery: "75%".into(),
            diagnostics: "Model: Detected mouse\nVID/PID: 24AE:1415\n".into(),
            device: client::Device { name: "Detected mouse".into(), family: "mouse".into(),
                connection: "USB".into(), brand: None, model: None },
        };
        controls.state.borrow_mut().reopen(vec![candidate], false);
        controls.state.borrow_mut().inventory_report = Some(
            "HID interface inventory\nVID/PID=24AE:1415\nVID/PID=046D:C52B\n".into());
        controls.set_description("The detected mouse sometimes loses its battery reading.");
        let preview = controls.diagnostics();
        let draft = submit_offline(controls);
        assert_eq!(draft.device.name, "Detected mouse");
        assert_eq!(draft.diagnostics.as_deref(), Some(preview.as_str()));
        assert!(preview.contains("VID/PID=046D:C52B"));
    }

    #[test]
    fn inventory_collection_does_not_lock_editing_or_overwrite_a_newer_form() {
        let (_owner, _notice, dialog) = regression_dialog();
        let controls = &dialog.controls;
        controls.state.borrow_mut().reopen(Vec::new(), false);
        let old_generation = controls.state.borrow().inventory_generation;
        controls.inventory_working.set(true);
        assert!(Action::Manual.enabled(&controls.state.borrow()));
        assert!(Action::ToggleDiagnostics.enabled(&controls.state.borrow()));
        controls.state.borrow_mut().new_form();
        controls.state.borrow_mut().inventory_report = Some("new interfaces".into());
        controls.results.lock().unwrap().push((old_generation,
            ResultMessage::DeviceInventory(crate::feedback::inventory::DeviceInventory {
                report: "old interfaces".into(), unknown: vec![unknown_device("Old device", 0x1234, 0x5678)],
            })));
        controls.complete();
        assert_eq!(controls.state.borrow().inventory_report.as_deref(), Some("new interfaces"));
        assert!(controls.state.borrow().unknown_devices.is_empty());
        controls.state.borrow_mut().inventory_report = None;
        assert!(!controls.inventory_ready());
        controls.state.borrow_mut().attach_diagnostics = false;
        assert!(controls.inventory_ready());
        assert!(controls.draft().diagnostics.is_none());
    }

    fn unknown_device(name: &str, vendor_id: u16, product_id: u16) -> crate::feedback::inventory::UnknownDevice {
        crate::feedback::inventory::UnknownDevice {
            name: name.into(), vendor_id: Some(vendor_id), product_id: Some(product_id), interfaces: 3,
            diagnostics: format!("App: X-Battery test\nModel: {name}\nVID/PID: {vendor_id:04X}:{product_id:04X}\n"),
        }
    }

    #[test]
    fn device_picker_shows_ids_even_when_the_interface_cannot_be_captured() {
        let (_owner, _notice, dialog) = regression_dialog();
        let controls = &dialog.controls;
        let interface = crate::hid::HidInterface { path: "keyboard-interface".into(),
            vendor_id: 0x258a, product_id: 0x024e, usage_page: 1, usage: 6,
            input_len: 8, output_len: 0, feature_len: 0, product: "VGN V98 Pro V3".into(), serial: String::new() };
        let candidate = Candidate {
            interface: Some(interface), capture_interface: None, label: "VGN V98 Pro V3".into(),
            battery: "99%".into(), diagnostics: String::new(),
            device: client::Device { name: "VGN V98 Pro V3".into(), family: "keyboard".into(),
                connection: "USB".into(), brand: None, model: None },
        };
        controls.state.borrow_mut().reopen(vec![candidate], false);
        controls.state.borrow_mut().inventory_report = Some("HID interface inventory\n".into());
        controls.state.borrow_mut().menu = true;
        controls.rebuild();
        let items = controls.items.borrow();
        let row = items.iter().find(|item| matches!(item.action, Action::Device(0))).unwrap();
        assert_eq!(row.detail, "VID 258A · PID 024E");
        let selected = items.iter().find(|item| matches!(item.action, Action::Devices)).unwrap();
        assert!(selected.detail.contains("99%"));
        assert!(selected.detail.contains("VID 258A · PID 024E"));
        assert!(!Action::CaptureStart.enabled(&controls.state.borrow()));
    }

    #[test]
    fn unknown_device_selection_submits_its_ids_with_the_users_model() {
        let (_owner, _notice, dialog) = regression_dialog();
        let controls = &dialog.controls;
        controls.state.borrow_mut().reopen(Vec::new(), false);
        let generation = controls.state.borrow().inventory_generation;
        controls.results.lock().unwrap().push((generation,
            ResultMessage::DeviceInventory(crate::feedback::inventory::DeviceInventory {
                report: "HID interface inventory\nVID/PID=258A:024E\nVID/PID=1234:5678\n".into(),
                unknown: vec![unknown_device("VGN V98Pro V3", 0x258a, 0x024e)],
            })));
        controls.complete();
        controls.action(Action::Devices);
        {
            let items = controls.items.borrow();
            let manual = items.iter().find(|item| matches!(item.action, Action::Manual)).unwrap();
            let unknown = items.iter().find(|item| matches!(item.action, Action::UnknownDevice(0))).unwrap();
            assert!(unknown.rect.y > manual.rect.bottom());
            assert_eq!(unknown.label, "VGN V98Pro V3");
            assert_eq!(unknown.detail, "VID 258A · PID 024E");
        }
        controls.action(Action::UnknownDevice(0));
        assert_eq!(controls.model.text(), "VGN V98Pro V3");
        assert!(controls.brand.text().is_empty());
        assert_eq!(controls.state.borrow().connection, 3);
        assert!(!Action::CaptureStart.enabled(&controls.state.borrow()));
        controls.brand.set_text("VGN");
        controls.model.set_text("V98 Pro V3");
        controls.set_description("The receiver is connected but the battery is not shown.");
        controls.action(Action::Connection(1));
        controls.action(Action::Category(1));
        controls.action(Action::Devices);
        controls.action(Action::UnknownDevice(0));
        assert_eq!(controls.brand.text(), "VGN");
        assert_eq!(controls.model.text(), "V98 Pro V3");
        assert_eq!(controls.state.borrow().connection, 1);
        assert_eq!(controls.state.borrow().category, 1);
        let preview = controls.diagnostics();
        let draft = submit_offline(controls);
        assert_eq!(draft.device.name, "VGN V98 Pro V3");
        assert_eq!(draft.device.model.as_deref(), Some("V98 Pro V3"));
        assert_eq!(draft.diagnostics.as_deref(), Some(preview.as_str()));
        assert!(preview.contains("VID/PID: 258A:024E"));
        assert!(preview.contains("VID/PID=1234:5678"));
    }

    #[test]
    fn large_device_lists_remain_selectable_without_exhausting_native_controls() {
        let (_owner, _notice, dialog) = regression_dialog();
        let controls = &dialog.controls;
        controls.state.borrow_mut().reopen(Vec::new(), false);
        {
            let mut state = controls.state.borrow_mut();
            state.inventory_report = Some("HID interface inventory\n".into());
            state.unknown_devices = (0..150).map(|index|
                unknown_device(&format!("Device {index}"), 0x1234, index)).collect();
            state.menu = true;
        }
        controls.set_description("Keep this description while selecting the device.");
        for page in 0..150usize.div_ceil(DEVICE_PAGE_SIZE) {
            controls.action(Action::DevicePage(page));
            let items = controls.items.borrow();
            assert!(items.iter().filter(|item| !item.root).count() < SLOTS);
            assert!(items.iter().filter(|item| matches!(item.action, Action::UnknownDevice(_))).count() <= DEVICE_PAGE_SIZE);
        }
        assert!(controls.items.borrow().iter().any(|item| matches!(item.action, Action::UnknownDevice(149))));
        controls.action(Action::UnknownDevice(149));
        assert_eq!(controls.model.text(), "Device 149");
        assert!(controls.diagnostics().contains("VID/PID: 1234:0095"));
        controls.action(Action::Manual);
        assert!(controls.state.borrow().selected_unknown.is_none());
        assert!(controls.model.text().is_empty());
        assert!(!controls.diagnostics().contains("VID/PID: 1234:0095"));
        assert_eq!(controls.description.text(), "Keep this description while selecting the device.");
    }

    #[test]
    fn submit_action_accepts_software_feedback_without_hidden_brand_or_model() {
        let (_owner, _notice, dialog) = regression_dialog();
        let controls = &dialog.controls;
        controls.state.borrow_mut().reopen_software();
        controls.state.borrow_mut().attach_diagnostics = false;
        controls.reset_inputs();
        controls.rebuild();
        controls.set_description("Settings are lost after restarting the application.");
        assert!(controls.state.borrow().selected.is_none());
        assert!(controls.brand.text().is_empty());
        assert!(controls.model.text().is_empty());
        for handle in [controls.brand.handle, controls.model.handle] {
            assert!(!controls.edit_rects.borrow().iter().any(|(editor, _)| *editor == handle));
        }
        let draft = submit_offline(controls);
        assert!(crate::feedback::software::is_device(&draft.device));
        assert_eq!(draft.category, "other");
        assert_eq!(draft.description, "Settings are lost after restarting the application.");
        assert!(draft.device.brand.is_none());
        assert!(draft.device.model.is_none());
    }

    #[test]
    fn submit_action_still_requires_both_manual_device_fields() {
        let (_owner, _notice, dialog) = regression_dialog();
        let controls = &dialog.controls;
        controls.state.borrow_mut().reopen(Vec::new(), false);
        controls.state.borrow_mut().attach_diagnostics = false;
        controls.set_description("The connected mouse never reports its battery level.");
        for (brand, model) in [("", ""), ("Example", "  "), ("  ", "Mouse")] {
            controls.brand.set_text(brand);
            controls.model.set_text(model);
            controls.action_with_submit(Action::Submit, Some(|_, _| panic!("Invalid form submitted")));
            assert_eq!(controls.state.borrow().error,
                text("请填写品牌和型号。", "Enter both the brand and model."));
            assert!(!controls.state.borrow().busy());
        }
        controls.brand.set_text("Example");
        controls.model.set_text("Mouse");
        let draft = submit_offline(controls);
        assert_eq!(draft.device.brand.as_deref(), Some("Example"));
        assert_eq!(draft.device.model.as_deref(), Some("Mouse"));
    }

    #[test]
    fn completed_feedback_cannot_return_to_a_submittable_form() {
        let history = back_view(View::Success, true);
        assert_eq!(history, View::History);
        assert_eq!(back_view(history, true), View::Success);
        assert_eq!(back_view(View::Diagnostics, true), View::Success);
        assert_eq!(back_view(View::History, false), View::Form);
        assert_eq!(back_view(View::Thread, false), View::History);
    }

    #[test]
    fn slow_requests_leave_navigation_and_new_feedback_available() {
        let mut model = Model::default();
        for task in [Task::History, Task::Submit, Task::Thread, Task::Reply, Task::Draft, Task::Images] {
            model.detach_task();
            model.begin_task(task).unwrap();
            assert!(Action::New.enabled(&model));
            assert!(Action::Back.enabled(&model));
            assert!(Action::Close.enabled(&model));
            assert!(!Action::Submit.enabled(&model));
            assert!(!Action::Reply.enabled(&model));
        }
    }

    #[test]
    fn refreshing_history_twice_does_not_start_a_duplicate_load() {
        let mut model = Model::default();
        model.begin_task(Task::History).unwrap();
        assert!(!Action::History.enabled(&model));
        assert!(Action::Entry(0).enabled(&model));
    }

    #[test]
    fn native_feedback_controls_support_a_second_session_without_recreation() {
        let (_owner, _notice, dialog) = regression_dialog();
        let controls = &dialog.controls;
        let editor = controls.description.handle.hwnd().unwrap();
        controls.set_description("First feedback description");
        assert_eq!(controls.description_count.get(), "First feedback description".chars().count());
        let generation = controls.state.borrow_mut().begin_task(Task::Submit).unwrap();
        let entry = client::Entry {
            request_id: "12345678-1234-4234-9234-123456789abc".into(),
            created_at: 1, updated_at: 1, category: "battery".into(),
            device: client::Device {
                name: "Example mouse".into(), family: "mouse".into(), connection: "USB".into(),
                brand: None, model: None,
            },
            description: "First feedback description".into(), status: client::Status::Completed,
            number: Some(123), url: Some("https://github.com/youmikk/X-Battery/issues/123".into()),
        };
        controls.results.lock().unwrap().push((generation,
            ResultMessage::Submitted(Ok(client::SubmitOutcome { entry, pending: false }), None)));
        controls.complete();
        assert_eq!(controls.state.borrow().view, View::Success);
        controls.action(Action::New);
        assert_eq!(controls.state.borrow().view, View::Form);
        assert!(controls.description.text().is_empty());
        assert_eq!(controls.description.handle.hwnd().unwrap(), editor);
        controls.set_description("Second feedback description");
        assert_eq!(controls.description_count.get(), "Second feedback description".chars().count());
        let addition = crate::win32::to_wide(" · 补充");
        let end = controls.description.text().encode_utf16().count();
        unsafe {
            wu::SendMessageW(editor, wu::EM_SETSEL as u32, end, end as isize);
            wu::SendMessageW(editor, wu::EM_REPLACESEL as u32, 1, addition.as_ptr() as isize);
        }
        assert_eq!(controls.description.text(), "Second feedback description · 补充");
        assert_eq!(controls.description_count.get(), controls.description.text().chars().count());
        assert!(controls.state.borrow_mut().begin_task(Task::Submit).is_some());
        assert_eq!(controls.state.borrow().entries.len(), 1);
    }
}
fn category_label(value: &str) -> &'static str {
    match value {
        "device" => text("设备识别", "Device detection"),
        "battery" => text("电量读取", "Battery reading"),
        "dpi" => text("DPI 切换", "DPI switching"),
        _ => text("其他问题", "Other"),
    }
}

fn copy_report(owner: nwg::ControlHandle, content: &str) -> bool {
    use winapi::um::winbase::{GlobalAlloc, GlobalFree, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    let wide: Vec<u16> = content.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        if memory.is_null() {
            return false;
        }
        let target = GlobalLock(memory) as *mut u16;
        if target.is_null() {
            GlobalFree(memory);
            return false;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
        GlobalUnlock(memory);
        if wu::OpenClipboard(owner.hwnd().unwrap()) == 0 {
            GlobalFree(memory);
            return false;
        }
        let ok = wu::EmptyClipboard() != 0
            && !wu::SetClipboardData(wu::CF_UNICODETEXT, memory).is_null();
        wu::CloseClipboard();
        if !ok {
            GlobalFree(memory);
        }
        ok
    }
}

fn draw(c: &Controls, p: &mut Painter, body: bool, damage: R) {
    let t = p.t;
    if body {
        for r in c.boxes.borrow().iter() {
            if layout::intersects(*r, damage) {
                p.card(*r, 8.0, t.card, t.border_strong);
            }
        }
        for decoration in c.decorations.borrow().iter() {
            if !layout::intersects(decoration.rect(), damage) {
                continue;
            }
            match decoration {
                Decoration::Surface(r, soft) => {
                    p.card(*r, 8.0, if *soft { t.bg_sunken } else { t.card }, t.border)
                }
                Decoration::Glyph(icon, r) => feedback_icons::draw(p, *icon, *r, t.accent),
                Decoration::Tag(value, r) => {
                    p.round(*r, 5.0, t.accent_soft);
                    let mut content = *r;
                    if value == text("设备诊断", "Device diagnostics")
                        || value == text("软件诊断", "Software diagnostics") {
                        content.x += 18.0;
                        content.w -= 18.0;
                    }
                    p.text(value, content, 11.0, false, t.accent, Align::Center);
                }
                Decoration::Note(value, r) => {
                    let error = c.state.borrow().error == *value;
                    p.round(*r, 8.0, if error { t.red_soft } else { t.accent_soft });
                    feedback_icons::draw(
                        p,
                        if error { Icon::Info } else { Icon::Shield },
                        R::new(r.x + 12.0, r.y + 12.0, 16.0, 16.0),
                        if error { t.red } else { t.accent },
                    );
                    p.text_wrap(
                        value,
                        R::new(r.x + 36.0, r.y + 11.0, r.w - 48.0, r.h - 20.0),
                        11.0,
                        if error { t.red } else { t.accent },
                    );
                }
                Decoration::Count(r) => p.text(
                    &format!("{} / 2000", c.description_count.get()),
                    *r,
                    10.0,
                    false,
                    t.text3,
                    Align::Right,
                ),
                Decoration::Center(value, r, size, bold) => p.text(
                    value,
                    *r,
                    *size,
                    *bold,
                    if *bold { t.text } else { t.text2 },
                    Align::Center,
                ),
                Decoration::Check(r) => {
                    p.circle(r.cx(), r.cy(), r.w / 2.0, t.accent_soft);
                    feedback_icons::draw(
                        p,
                        Icon::Check,
                        R::new(r.cx() - 13.5, r.cy() - 13.5, 27.0, 27.0),
                        t.accent,
                    );
                }
            }
        }
        for label in c.labels.borrow().iter() {
            if !layout::intersects(label.rect, damage) {
                continue;
            }
            if label.bold || label.centered {
                p.text(
                    &label.text,
                    label.rect,
                    label.size,
                    label.bold,
                    if label.muted { t.text2 } else { t.text },
                    Align::Left,
                );
            } else {
                p.text_wrap(
                    &label.text,
                    label.rect,
                    label.size,
                    if label.muted { t.text2 } else { t.text },
                );
            }
        }
    } else {
        let s = c.state.borrow();
        let sub = matches!(s.view, View::Diagnostics | View::Capture | View::History | View::Thread);
        let title = match s.view {
            View::History => text("我的反馈", "My feedback").into(),
            View::Diagnostics => text("附带的诊断内容", "Included diagnostics").into(),
            View::Capture => text("本地分步骤采集", "Local guided capture").into(),
            View::Thread => s
                .current
                .as_ref()
                .and_then(|e| e.number)
                .map(|n| format!("#{n}"))
                .unwrap_or_else(|| text("反馈详情", "Feedback details").into()),
            _ => if s.software { text("反馈软件问题", "Software feedback").into() }
                else { text("反馈设备", "Device feedback").into() },
        };
        if !sub {
            p.round(R::new(25.0, 24.0, 39.0, 39.0), 11.0, t.accent_soft);
            feedback_icons::draw(p, Icon::Message, R::new(34.0, 33.0, 21.0, 21.0), t.accent);
        }
        let x = if sub { 69.0 } else { 76.0 };
        p.text(
            &title,
            R::new(x, 19.0, 405.0, 30.0),
            22.0,
            true,
            t.text,
            Align::Left,
        );
        let subtitle = match s.view {
            View::Capture => text("按步骤记录变化，结束后检查并保存。", "Mark each change, then review and save."),
            View::History => text(
                "查看回复，也可以继续补充设备信息。",
                "Read replies and add more device details.",
            ),
            View::Diagnostics => text(
                if s.software { "本次反馈的运行记录摘要。" }
                    else { "本次反馈的设备诊断信息。" },
                if s.software { "Activity summary for this report." }
                    else { "Device diagnostics for this report." },
            ),
            View::Thread => text("反馈详情", "Feedback details"),
            View::Success => "",
            View::Form => text(
                if s.software { "简单描述问题，诊断摘要会随反馈一起发送。" }
                    else { "描述遇到的问题，我们一起完善设备支持。" },
                if s.software { "Describe the problem. A safe diagnostic summary can be attached." }
                    else { "Tell us what happened. Help improve device support." },
            ),
        };
        p.text(
            subtitle,
            R::new(x, 51.0, 430.0, 22.0),
            12.0,
            false,
            t.text2,
            Align::Left,
        );
        let fy = c.height.get() - FOOTER;
        p.hline(0.0, WIDTH, fy, t.border);
        let footer = match s.view {
            View::Capture => text("采集完成后，返回反馈页面提交。", "After capture, return to the feedback form to submit."),
            View::Diagnostics => text(
                if s.software { "可以在反馈窗口关闭“附带软件诊断”。" }
                    else { "可以在反馈窗口关闭“附带设备诊断”。" },
                "You can turn off diagnostics in the feedback form.",
            ),
            View::Thread => text(
                "补充内容会公开显示在同一条反馈下。",
                "Follow-ups will be public on the same issue.",
            ),
            View::History => text(
                "这里显示在这台电脑提交的反馈。",
                "Feedback submitted on this computer.",
            ),
            View::Success => text(
                "内容已发送，可随时查看处理进度。",
                "Sent. You can check progress at any time.",
            ),
            View::Form if client::configured() => text(
                if s.software { "无需 GitHub 账号。描述及所选诊断摘要会公开显示。" }
                    else { "无需 GitHub 账号。反馈及所选附件会公开显示。" },
                "No GitHub account needed. Reports and selected diagnostics are public.",
            ),
            View::Form => text(
                "当前版本暂未开通应用内直投，可复制下方整理好的报告前往 GitHub 提交（内容与附件将公开）。",
                "In-app submission is not enabled in this build. Copy the report to submit publicly on GitHub.",
            ),
        };
        let content_y = fy + 15.0;
        let controls_x = c.items.borrow().iter()
            .filter(|item| item.root && item.rect.y >= fy)
            .map(|item| item.rect.x)
            .fold(WIDTH - 25.0, f32::min);
        feedback_icons::draw(p, Icon::Globe, R::new(25.0, content_y + 11.5, 13.0, 13.0), t.text2);
        p.text_wrap_centered(footer,
            R::new(44.0, content_y, (controls_x - 60.0).max(160.0), 36.0), 10.0, t.text2);
        if !c.native_frame.get() {
            p.round_stroke(
                R::new(0.5, 0.5, WIDTH - 1.0, c.height.get() - 1.0),
                13.0,
                t.border_strong,
                1.0,
            );
        }
    }
}

fn draw_button(p: &mut Painter, item: &Item, pressed: bool, focus: bool, enabled: bool) {
    let t = p.t;
    if matches!(item.action, Action::Diagnostics | Action::ToggleDiagnostics | Action::Capture | Action::ToggleCapture) {
        p.rect(R::new(0.0, 0.0, item.rect.w, item.rect.h), t.bg_sunken);
    }
    let r = R::new(1.0, 1.0, item.rect.w - 2.0, item.rect.h - 2.0);
    match item.style {
        Style::Close => {
            if pressed || focus {
                p.round(r, 6.0, t.card_hover);
            }
            feedback_icons::draw(
                p,
                Icon::Close,
                R::new(r.cx() - 9.0, r.cy() - 9.0, 18.0, 18.0),
                t.text2,
            );
            return;
        }
        Style::Toggle(on) => {
            p.switch(r, on, enabled);
            return;
        }
        Style::Primary => p.round(r, 8.0, if enabled { t.accent } else { t.border_strong }),
        Style::Choice(selected) => {
            p.round(r, 7.0, if selected { t.accent_soft } else { t.bg_sunken });
            if selected {
                p.round_stroke(r, 7.0, t.accent, 1.0);
            }
        }
        Style::Connection(selected) => {
            if selected || pressed {
                p.round(
                    r,
                    4.0,
                    if selected {
                        t.accent_soft
                    } else {
                        t.card_hover
                    },
                );
            }
        }
        Style::Card | Style::Device(_) | Style::History => p.card(
            r,
            8.0,
            if pressed { t.card_hover } else { t.card },
            t.border_strong,
        ),
        Style::Option(_, selected) => {
            p.round(
                r,
                6.0,
                if selected {
                    t.accent_soft
                } else if pressed {
                    t.card_hover
                } else {
                    t.card
                },
            );
        }
        Style::Chip => p.card(r, 6.0, t.bg_sunken, t.border),
        Style::Attachment => {
            p.round_stroke(r, 6.0, t.border_strong, 1.0);
        }
        Style::Plain => {
            if pressed {
                p.round(r, 6.0, t.card_hover);
            }
        }
    }
    if focus {
        p.round_stroke(r.inset(2.0), 6.0, t.accent, 1.0);
    }
    let color = if !enabled {
        t.text3
    } else if matches!(item.style, Style::Primary) {
        t.accent_fg
    } else if matches!(item.style, Style::Choice(true) | Style::Connection(true)) {
        t.accent
    } else {
        t.text
    };
    match item.style {
        Style::Device(device) => {
            let glyph = R::new(14.0, r.cy() - 16.5, 33.0, 33.0);
            p.round(glyph, 7.0, t.bg_sunken);
            device_icon(
                p,
                device,
                R::new(glyph.cx() - 10.0, glyph.cy() - 10.0, 20.0, 20.0),
                t.text2,
            );
            let tag_width = if item.tag.is_empty() {
                0.0
            } else {
                label_width(&item.tag, 11.0) + 16.0
            };
            p.text(
                &item.label,
                R::new(59.0, 9.0, r.w - 108.0 - tag_width, 23.0),
                13.0,
                true,
                color,
                Align::Left,
            );
            p.text(
                &item.detail,
                R::new(59.0, 33.0, r.w - 108.0 - tag_width, 20.0),
                11.0,
                false,
                t.text2,
                Align::Left,
            );
            if tag_width > 0.0 {
                let tag = R::new(r.w - 43.0 - tag_width, r.cy() - 12.0, tag_width, 24.0);
                p.round(tag, 5.0, t.accent_soft);
                p.text(&item.tag, tag, 11.0, false, t.accent, Align::Center);
            }
            feedback_icons::draw(
                p,
                Icon::Chevron,
                R::new(r.w - 29.0, r.cy() - 7.5, 15.0, 15.0),
                t.text2,
            );
            return;
        }
        Style::Option(device, selected) => {
            device_icon(p, device, R::new(9.0, r.cy() - 8.5, 17.0, 17.0), t.text2);
            let detail_width = if item.detail.is_empty() { 0.0 } else { 195.0 };
            p.text(
                &item.label,
                R::new(36.0, 0.0, r.w - 84.0 - detail_width, r.h),
                12.0,
                false,
                t.text,
                Align::Left,
            );
            p.text(&item.detail, R::new(r.w - 36.0 - detail_width, 0.0, detail_width, r.h),
                11.0, false, t.text2, Align::Right);
            if selected {
                feedback_icons::draw(
                    p,
                    Icon::Check,
                    R::new(r.w - 28.0, r.cy() - 8.5, 17.0, 17.0),
                    t.accent,
                );
            }
            return;
        }
        Style::Chip => {
            feedback_icons::draw(
                p,
                Icon::Image,
                R::new(8.0, r.cy() - 6.0, 12.0, 12.0),
                t.text2,
            );
            p.text(
                &item.label,
                R::new(25.0, 0.0, r.w - 49.0, r.h),
                11.0,
                false,
                t.text2,
                Align::Left,
            );
            feedback_icons::draw(
                p,
                Icon::Close,
                R::new(r.w - 18.0, r.cy() - 6.0, 12.0, 12.0),
                t.text3,
            );
            return;
        }
        Style::History => {
            p.text(
                &item.tag,
                R::new(15.0, 11.0, r.w - 140.0, 20.0),
                11.0,
                false,
                t.text2,
                Align::Left,
            );
            p.text(
                text("查看进度", "View progress"),
                R::new(r.w - 125.0, 11.0, 110.0, 20.0),
                10.0,
                false,
                t.text2,
                Align::Right,
            );
            p.text(
                &item.label,
                R::new(15.0, 37.0, r.w - 30.0, 25.0),
                13.0,
                true,
                t.text,
                Align::Left,
            );
            p.text(
                &item.detail,
                R::new(15.0, 69.0, r.w - 30.0, 20.0),
                11.0,
                false,
                t.text2,
                Align::Left,
            );
            return;
        }
        _ => {}
    }
    if !item.detail.is_empty() {
        p.text(
            &item.label,
            R::new(16.0, 10.0, r.w - 135.0, 24.0),
            13.0,
            true,
            color,
            Align::Left,
        );
        p.text(
            &item.detail,
            R::new(16.0, 36.0, r.w - 32.0, 23.0),
            11.0,
            false,
            t.text2,
            Align::Left,
        );
        if !item.tag.is_empty() {
            let tag = R::new(r.w - 108.0, 17.0, 92.0, 24.0);
            p.round(tag, 5.0, t.accent_soft);
            p.text(&item.tag, tag, 11.0, false, t.accent, Align::Center);
        }
        return;
    }
    let icon = match item.action {
        Action::Submit => Some(Icon::Send),
        Action::History => Some(Icon::History),
        Action::Back if item.root && item.rect.y < 60.0 => Some(Icon::Arrow),
        Action::AddImage => Some(Icon::Image),
        Action::Fallback => Some(Icon::Copy),
        _ => None,
    };
    if let Some(icon) = icon {
        let only = matches!(item.action, Action::Back) && item.rect.y < 60.0;
        let size = if matches!(item.style, Style::Attachment) {
            14.0
        } else {
            16.0
        };
        let font_size = if matches!(item.style, Style::Attachment) {
            11.0
        } else {
            12.0
        };
        let width = if only {
            size
        } else {
            (p.measure(&item.label, font_size, false).0.ceil() + size + 17.0).min(r.w - 12.0)
        };
        let x = r.cx() - width / 2.0;
        feedback_icons::draw(
            p,
            icon,
            R::new(x, r.cy() - size / 2.0, size, size),
            if matches!(item.style, Style::Primary) {
                color
            } else {
                t.text2
            },
        );
        if !only {
            p.text(
                &item.label,
                R::new(x + size + 7.0, 0.0, width - size - 7.0, r.h),
                font_size,
                false,
                if matches!(item.style, Style::Primary) {
                    color
                } else {
                    t.text2
                },
                Align::Left,
            );
        }
    } else {
        p.text(
            &item.label,
            r,
            if matches!(item.style, Style::Connection(_)) {
                11.0
            } else {
                12.0
            },
            false,
            if matches!(item.action, Action::Close) {
                t.text2
            } else if matches!(item.action, Action::Example | Action::Diagnostics) {
                t.accent
            } else {
                color
            },
            Align::Center,
        );
    }
}
impl Dialog {
    pub fn new(owner: &nwg::Window, notice: nwg::NoticeSender,
        request_capture_refresh: impl Fn() + 'static) -> Result<Self, String> {
        let scale = crate::dwm::system_scale();
        let px = |v: f32| (v * scale).round() as i32;
        let mut overlay = nwg::Window::default();
        nwg::Window::builder()
            .title("")
            .flags(nwg::WindowFlags::POPUP)
            .ex_flags(0x0008_0080)
            .parent(Some(owner))
            .size((1, 1))
            .build(&mut overlay)
            .map_err(|e| e.to_string())?;
        unsafe {
            wu::SetLayeredWindowAttributes(overlay.handle.hwnd().unwrap(), 0, 48, wu::LWA_ALPHA);
        }
        let mut window = nwg::Window::default();
        nwg::Window::builder()
            .title(crate::gui::WINDOW_TITLE)
            .flags(nwg::WindowFlags::POPUP)
            .ex_flags(0x80)
            .parent(Some(&overlay))
            .size((px(WIDTH), px(660.0)))
            .build(&mut window)
            .map_err(|e| e.to_string())?;
        let mut body = nwg::Frame::default();
        nwg::Frame::builder()
            .parent(&window)
            .flags(nwg::FrameFlags::VISIBLE)
            .build(&mut body)
            .map_err(|e| e.to_string())?;
        let hwnd = body.handle.hwnd().unwrap() as _;
        let style = super::canvas::get_window_long(hwnd, -16);
        super::canvas::set_window_long(hwnd, -16,
            style | (wu::WS_VSCROLL | wu::WS_CLIPCHILDREN | wu::WS_CLIPSIBLINGS) as isize);
        let hwnd = window.handle.hwnd().unwrap() as _;
        let style = super::canvas::get_window_long(hwnd, -16);
        super::canvas::set_window_long(hwnd, -16, style | 0x0200_0000);
        let mut font = nwg::Font::default();
        nwg::Font::builder()
            .family("Microsoft YaHei UI")
            .size_absolute(px(12.0) as u32)
            .build(&mut font)
            .map_err(|e| e.to_string())?;
        let new_box = || -> Result<nwg::TextBox, String> {
            let mut edit = nwg::TextBox::default();
            edit.handle = nwg::ControlBase::build_hwnd()
                .class_name("EDIT")
                .parent(Some(body.handle))
                .flags(0x4001_1044 | wu::WS_CLIPSIBLINGS)
                .build()
                .map_err(|e| e.to_string())?;
            edit.set_font(Some(&font));
            edit.set_text("");
            edit.set_limit(4000);
            Ok(edit)
        };
        let description = new_box()?;
        let reply = new_box()?;
        let new_input = |limit| -> Result<nwg::TextInput, String> {
            let mut edit = nwg::TextInput::default();
            edit.handle = nwg::ControlBase::build_hwnd()
                .class_name("EDIT")
                .parent(Some(body.handle))
                .flags(0x4001_0080 | wu::WS_CLIPSIBLINGS)
                .build()
                .map_err(|e| e.to_string())?;
            edit.set_font(Some(&font));
            edit.set_text("");
            edit.set_limit(limit);
            Ok(edit)
        };
        let brand = new_input(80)?;
        let model = new_input(120)?;
        let new_button = |parent| -> Result<nwg::Button, String> {
            let mut button = nwg::Button::default();
            button.handle = nwg::ControlBase::build_hwnd()
                .class_name("BUTTON")
                .parent(Some(parent))
                .flags(0x4001_000b | wu::WS_CLIPSIBLINGS)
                .build()
                .map_err(|e| e.to_string())?;
            button.set_font(Some(&font));
            Ok(button)
        };
        let buttons = (0..SLOTS)
            .map(|_| new_button(body.handle))
            .collect::<Result<Vec<_>, _>>()?;
        let header_buttons = (0..6)
            .map(|_| new_button(window.handle))
            .collect::<Result<Vec<_>, _>>()?;
        let c = Rc::new(Controls {
            window,
            body,
            buttons,
            header_buttons,
            description,
            brand,
            model,
            reply,
            _font: font,
            state: RefCell::new(Model::default()),
            items: RefCell::new(Vec::new()),
            labels: RefCell::new(Vec::new()),
            edit_rects: RefCell::new(Vec::new()),
            boxes: RefCell::new(Vec::new()),
            decorations: RefCell::new(Vec::new()),
            colors: RefCell::new(Colors::new(false)),
            scroll: Cell::new(0.0),
            content_height: Cell::new(0.0),
            height: Cell::new(660.0),
            max_height: Cell::new(740.0),
            owner: owner.handle,
            overlay,
            scale: Cell::new(scale),
            native_frame: Cell::new(false),
            results: Arc::new(Mutex::new(Vec::new())),
            notice,
            updating: Cell::new(false),
            description_count: Cell::new(0),
            applied: RefCell::new(Default::default()),
            scrollbar: Cell::new(None),
            inventory_working: Cell::new(false),
            capture_folder: RefCell::new(None),
            request_capture_refresh: Box::new(request_capture_refresh),
        });
        let callback = c.clone();
        let handler = nwg::full_bind_event_handler(&c.window.handle, move |event, data, handle| {
            use nwg::Event as E;
            match event {
                E::OnWindowClose => {
                    if let nwg::EventData::OnWindowClose(close) = data {
                        close.close(false);
                    }
                    callback.action(Action::Close);
                }
                E::OnKeyEsc => {
                    if callback.state.borrow().menu {
                        callback.state.borrow_mut().menu = false;
                        callback.rebuild();
                    } else {
                        callback.action(Action::Close);
                    }
                }
                E::OnButtonClick => {
                    let items = callback.items.borrow();
                    let mut bi = 0;
                    let mut ri = 0;
                    let mut action = None;
                    for item in items.iter() {
                        let button = if item.root {
                            let b = &callback.header_buttons[ri];
                            ri += 1;
                            b
                        } else {
                            let b = &callback.buttons[bi];
                            bi += 1;
                            b
                        };
                        if button.handle == handle {
                            action = Some(item.action.clone());
                            break;
                        }
                    }
                    drop(items);
                    if let Some(action) = action {
                        callback.action(action);
                    }
                }
                E::OnTextInput
                    if handle == callback.description.handle
                            || handle == callback.brand.handle
                            || handle == callback.model.handle =>
                {
                    if handle == callback.description.handle {
                        callback.description_count.set(callback.description.text().chars().count());
                        callback.invalidate_count();
                    }
                    if !callback.updating.get() {
                        let rebuild = callback.state.try_borrow_mut()
                            .map(|mut model| model.edit_draft()).unwrap_or(false);
                        if rebuild {
                            callback.rebuild();
                        }
                    }
                }
                _ => {}
            }
        });
        let mut raw = Vec::new();
        {
            let c = c.clone();
            let handle = c.overlay.handle;
            raw.push(
                nwg::bind_raw_event_handler(&handle, 0x10159, move |hwnd, msg, _, _| match msg {
                    wu::WM_PAINT => unsafe {
                        let mut ps: wu::PAINTSTRUCT = std::mem::zeroed();
                        let dc = wu::BeginPaint(hwnd, &mut ps);
                        let mut r = std::mem::zeroed();
                        wu::GetClientRect(hwnd, &mut r);
                        let brush = winapi::um::wingdi::CreateSolidBrush(colorref(0xFF202B23));
                        wu::FillRect(dc, &r, brush);
                        winapi::um::wingdi::DeleteObject(brush as _);
                        wu::EndPaint(hwnd, &ps);
                        Some(0)
                    },
                    wu::WM_LBUTTONDOWN => {
                        c.window.set_focus();
                        Some(0)
                    }
                    wu::WM_ERASEBKGND => Some(1),
                    _ => None,
                })
                .map_err(|e| e.to_string())?,
            );
        }
        c.update_input_prompts();
        for handle in [c.description.handle, c.reply.handle] {
            let c = c.clone();
            raw.push(nwg::bind_raw_event_handler(&handle,0x1015a,move|hwnd,msg,_,_|{
                if msg==wu::WM_PAINT {unsafe{
                    let result=winapi::um::commctrl::DefSubclassProc(hwnd,msg,0,0);
                    let is_description=c.description.handle.hwnd()==Some(hwnd);
                    let empty=if is_description{c.description.text().is_empty()}else{c.reply.text().is_empty()};
                    if empty&&wu::GetFocus()!=hwnd {
                        let dc=wu::GetDC(hwnd);let old=winapi::um::wingdi::SelectObject(dc,c._font.handle as _);winapi::um::wingdi::SetTextColor(dc,colorref(c.colors.borrow().theme.text3));winapi::um::wingdi::SetBkMode(dc,1);
                        let mut rect=std::mem::zeroed();wu::GetClientRect(hwnd,&mut rect);
                        let hint=if is_description{if c.state.borrow().software{text("例如：修改设置后点击保存出现报错，重新打开软件后设置没有保留。","For example: saving settings shows an error, and the changes are lost after reopening the app.")}else{text("例如：接收器已经连接，官方驱动可以看到电量，但这里一直显示未知。","For example: the receiver is connected and the official driver shows the battery, but it stays unknown here.")}}else{text("补充测试结果、固件版本或新的问题表现…","Add test results, firmware versions or new observations…")};
                        let wide=crate::win32::to_wide(hint);wu::DrawTextW(dc,wide.as_ptr(),-1,&mut rect,wu::DT_WORDBREAK|wu::DT_NOPREFIX);winapi::um::wingdi::SelectObject(dc,old);wu::ReleaseDC(hwnd,dc);
                    }
                    return Some(result);
                }}
                if msg==wu::WM_SETFOCUS||msg==wu::WM_KILLFOCUS {unsafe{wu::InvalidateRect(hwnd,std::ptr::null(),0);}}
                None
            }).map_err(|e|e.to_string())?);
        }
        for (body, handle) in [(false, c.window.handle), (true, c.body.handle)] {
            let c = c.clone();
            let content_buffer = RefCell::new(super::gfx::Canvas::new());
            let control_buffer = RefCell::new(super::gfx::Canvas::new());
            let control_buffer_size = Cell::new((0, 0));
            let fonts = RefCell::new(
                super::gfx::Fonts::new("Microsoft YaHei UI").ok_or("Font initialization failed")?,
            );
            raw.push(
                nwg::bind_raw_event_handler(
                    &handle,
                    0x10150 + usize::from(body),
                    move |hwnd, msg, wp, lp| match msg {
                        wu::WM_ERASEBKGND => Some(1),
                        wu::WM_PAINT | wu::WM_PRINTCLIENT => unsafe {
                            let mut ps: wu::PAINTSTRUCT = std::mem::zeroed();
                            let dc = if msg == wu::WM_PAINT {
                                wu::BeginPaint(hwnd, &mut ps)
                            } else {
                                wp as _
                            };
                            let mut rect = std::mem::zeroed();
                            wu::GetClientRect(hwnd, &mut rect);
                            let mut canvas = content_buffer.borrow_mut();
                            let mut fonts = fonts.borrow_mut();
                            let colors = c.colors.borrow();
                            let mem = canvas.ensure(dc as isize, rect.right, rect.bottom);
                            crate::win32::FillRect(
                                mem as _,
                                &crate::win32::Rect {
                                    left: 0,
                                    top: 0,
                                    right: rect.right,
                                    bottom: rect.bottom,
                                },
                                colors.brush,
                            );
                            let dirty = if msg == wu::WM_PAINT { ps.rcPaint } else { rect };
                            let scroll = if body { c.scroll.get() } else { 0.0 };
                            let damage = R::new(dirty.left as f32 / c.scale.get(),
                                dirty.top as f32 / c.scale.get() + scroll,
                                (dirty.right - dirty.left) as f32 / c.scale.get(),
                                (dirty.bottom - dirty.top) as f32 / c.scale.get());
                            canvas.paint(
                                &mut fonts,
                                &colors.theme,
                                c.scale.get(),
                                scroll,
                                rect.right as f32 / c.scale.get(),
                                rect.bottom as f32 / c.scale.get(),
                                |p| draw(&c, p, body, damage),
                            );
                            super::gfx::blit(dc as isize, mem, rect.right, rect.bottom);
                            if msg == wu::WM_PAINT {
                                wu::EndPaint(hwnd, &ps);
                            }
                            Some(0)
                        },
                        wu::WM_DRAWITEM => unsafe {
                            let draw = &*(lp as *const wu::DRAWITEMSTRUCT);
                            let items = c.items.borrow();
                            let mut bi = 0;
                            let mut ri = 0;
                            let item = items.iter().find(|i| {
                                let button = if i.root {
                                    let b = &c.header_buttons[ri];
                                    ri += 1;
                                    b
                                } else {
                                    let b = &c.buttons[bi];
                                    bi += 1;
                                    b
                                };
                                button.handle.hwnd() == Some(draw.hwndItem)
                            });
                            if let Some(item) = item {
                                let w = draw.rcItem.right - draw.rcItem.left;
                                let h = draw.rcItem.bottom - draw.rcItem.top;
                                let mut buffer = control_buffer.borrow_mut();
                                let mut fonts = fonts.borrow_mut();
                                let colors = c.colors.borrow();
                                let size = layout::buffer_size(control_buffer_size.get(), (w, h));
                                control_buffer_size.set(size);
                                let mem = buffer.ensure(draw.hDC as isize, size.0, size.1);
                                crate::win32::FillRect(
                                    mem as _,
                                    &crate::win32::Rect {
                                        left: 0,
                                        top: 0,
                                        right: w,
                                        bottom: h,
                                    },
                                    if matches!(item.action, Action::Diagnostics | Action::ToggleDiagnostics | Action::Capture | Action::ToggleCapture) {
                                        colors.sunken_brush
                                    } else {
                                        colors.brush
                                    },
                                );
                                buffer.paint(
                                    &mut fonts,
                                    &colors.theme,
                                    c.scale.get(),
                                    0.0,
                                    w as f32 / c.scale.get(),
                                    h as f32 / c.scale.get(),
                                    |p| {
                                        draw_button(
                                            p,
                                            item,
                                            draw.itemState & 1 != 0,
                                            draw.itemState & 0x10 != 0,
                                            draw.itemState & 4 == 0,
                                        )
                                    },
                                );
                                super::gfx::blit(draw.hDC as isize, mem, w, h);
                            }
                            Some(1)
                        },
                        wu::WM_CTLCOLOREDIT | wu::WM_CTLCOLORSTATIC => unsafe {
                            let colors = c.colors.borrow();
                            winapi::um::wingdi::SetTextColor(wp as _, colorref(colors.theme.text));
                            winapi::um::wingdi::SetBkColor(wp as _, colorref(colors.theme.card));
                            Some(colors.brush as isize)
                        },
                        wu::WM_MOUSEWHEEL => {
                            let delta = (wp >> 16) as i16 as f32;
                            c.scroll.set(c.scroll.get() - delta / 120.0 * 66.0);
                            c.layout(false);
                            Some(0)
                        }
                        wu::WM_VSCROLL if body => {
                            let action = wp as u16 as u32;
                            let mut scroll = c.scroll.get();
                            match action {
                                0 => scroll -= 24.0,
                                1 => scroll += 24.0,
                                2 => scroll -= c.body_height(),
                                3 => scroll += c.body_height(),
                                4 | 5 => unsafe {
                                    let mut info: wu::SCROLLINFO = std::mem::zeroed();
                                    info.cbSize = std::mem::size_of::<wu::SCROLLINFO>() as u32;
                                    info.fMask = wu::SIF_TRACKPOS;
                                    wu::GetScrollInfo(hwnd, wu::SB_VERT as i32, &mut info);
                                    scroll = info.nTrackPos as f32;
                                },
                                _ => {}
                            }
                            c.scroll.set(scroll);
                            c.layout(false);
                            Some(0)
                        }
                        wu::WM_TIMER if !body && wp == capture::TIMER => {
                            if c.state.borrow().view == View::Capture {
                                if !crate::capture::status().running { c.finish_capture(); }
                                c.rebuild_position(false);
                            } else { c.stop_capture_timer(); }
                            Some(0)
                        }
                        wu::WM_NCHITTEST if !body => {
                            let mut point = winapi::shared::windef::POINT {
                                x: lp as i16 as i32,
                                y: (lp >> 16) as i16 as i32,
                            };
                            unsafe {
                                wu::ScreenToClient(hwnd, &mut point);
                            }
                            if point.y as f32 / c.scale.get() < 70.0
                                && point.x as f32 / c.scale.get() < WIDTH - 190.0
                            {
                                Some(wu::HTCAPTION as isize)
                            } else {
                                None
                            }
                        }
                        _ => None,
                    },
                )
                .map_err(|e| e.to_string())?,
            );
        }
        c.rebuild();
        Ok(Self {
            controls: c,
            handler: Some(handler),
            raw,
        })
    }
    pub fn show(&self, owner: &nwg::Window, devices: Vec<Candidate>, dark: bool, history: bool) {
        self.show_mode(owner, devices, dark, history, false);
    }
    pub fn show_software(&self, owner: &nwg::Window, dark: bool) {
        self.show_mode(owner, Vec::new(), dark, false, true);
    }
    fn show_mode(&self, owner: &nwg::Window, devices: Vec<Candidate>, dark: bool, history: bool, software: bool) {
        let c = &self.controls;
        c.remember_reply();
        if c.state.borrow().view == View::Capture { c.finish_capture(); }
        *c.colors.borrow_mut() = Colors::new(dark);
        c.update_input_prompts();
        let reset = if software { c.state.borrow_mut().reopen_software() }
            else { c.state.borrow_mut().reopen(devices, history) };
        if reset {
            c.stop_capture_timer();
            crate::capture::clear();
            *c.capture_folder.borrow_mut() = None;
            c.reset_inputs();
        }
        let hwnd = c.window.handle.hwnd().unwrap() as _;
        let monitor =
            unsafe { crate::win32::MonitorFromWindow(owner.handle.hwnd().unwrap() as _, 2) };
        let mut info = crate::win32::MonitorInfo {
            cb_size: std::mem::size_of::<crate::win32::MonitorInfo>() as u32,
            ..Default::default()
        };
        let scale = c.scale.get();
        let valid = unsafe { crate::win32::GetMonitorInfoW(monitor, &mut info) } != 0;
        let height = if valid {
            ((info.rc_work.height() as f32 - 32.0).min(owner.size().1 as f32 - 72.0) / scale)
                .min(740.0)
                .max(320.0)
        } else {
            740.0
        };
        c.max_height.set(height);
        let (x, y) = owner.position();
        let (w, h) = owner.size();
        let (dw, dh) = c.window.size();
        let mut x = x + (w as i32 - dw as i32) / 2;
        let mut y = y + (h as i32 - dh as i32) / 2;
        if valid {
            x = x
                .max(info.rc_work.left)
                .min((info.rc_work.right - dw as i32).max(info.rc_work.left));
            y = y
                .max(info.rc_work.top)
                .min((info.rc_work.bottom - dh as i32).max(info.rc_work.top));
        }
        c.window.set_position(x, y);
        crate::dwm::set_dark_titlebar(hwnd, dark);
        c.rebuild();
        c.overlay.set_visible(true);
        unsafe {
            wu::EnableWindow(owner.handle.hwnd().unwrap(), 0);
        }
        c.window.set_visible(true);
        c.window.set_focus();
        c.prepare_software_diagnostics();
        c.prepare_inventory();
        if history && !c.state.borrow().busy() {
            c.work(Task::History, || ResultMessage::History(client::load_history()));
        }
    }
    pub fn poll(&self) {
        self.controls.complete();
    }
}
impl Drop for Dialog {
    fn drop(&mut self) {
        self.controls.stop_capture_timer();
        crate::capture::clear();
        for raw in self.raw.drain(..) {
            let _ = nwg::unbind_raw_event_handler(&raw);
        }
        if let Some(handler) = self.handler.take() {
            nwg::unbind_event_handler(&handler);
        }
    }
}
