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

#[derive(Clone, Copy, PartialEq, Debug)]
enum View {
    Form,
    Diagnostics,
    History,
    Thread,
    Success,
}

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
    Devices,
    Device(usize),
    Manual,
    Connection(usize),
    Category(usize),
    Diagnostics,
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
    Submitted(Result<client::SubmitOutcome, String>, Option<String>),
    Thread(
        Result<client::Thread, String>,
        Result<Option<String>, String>,
    ),
    Reply(Result<client::ReplyReceipt, String>),
    Draft(String, Result<client::Draft, String>),
    Images(Result<Vec<client::Image>, String>),
}
struct Model {
    view: View,
    devices: Vec<Candidate>,
    latest_devices: Vec<Candidate>,
    initialized: bool,
    menu: bool,
    selected: Option<usize>,
    connection: usize,
    category: usize,
    attach_diagnostics: bool,
    images: Vec<client::Image>,
    entries: Vec<client::Entry>,
    current: Option<client::Entry>,
    thread: Option<client::Thread>,
    draft_retry: Option<String>,
    error: String,
    busy: bool,
    submitted: bool,
    generation: u64,
    diagnostics_override: Option<String>,
    restored_draft: Option<client::Draft>,
    reply_drafts: std::collections::BTreeMap<u64, String>,
}
impl Default for Model {
    fn default() -> Self {
        Self {
            view: View::Form,
            devices: Vec::new(),
            selected: None,
            connection: 0,
            category: 0,
            attach_diagnostics: true,
            images: Vec::new(),
            entries: Vec::new(),
            current: None,
            thread: None,
            draft_retry: None,
            error: String::new(),
            busy: false,
            submitted: false,
            generation: 0,
            diagnostics_override: None,
            latest_devices: Vec::new(),
            initialized: false,
            menu: false,
            restored_draft: None,
            reply_drafts: Default::default(),
        }
    }
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
    viewer: nwg::TextBox,
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
        if let Some(value) = &s.diagnostics_override {
            return value.clone();
        }
        let mut value = s
            .selected
            .and_then(|i| s.devices.get(i).map(|d| d.diagnostics.clone()))
            .unwrap_or_else(|| {
                format!(
                    "App: X-Battery {}\nSystem: {}\nNo detected interface selected.\n",
                    crate::gui::APP_VERSION,
                    crate::feedback::system_version()
                )
            });
        value.push_str(&format!(
            "Selected connection: {}\n",
            ["2.4GHz", "USB", "Bluetooth", "unknown"][s.connection]
        ));
        value
    }
    fn draft(&self) -> client::Draft {
        let s = self.state.borrow();
        if let Some(draft) = &s.restored_draft {
            return draft.clone();
        }
        let mut device = s
            .selected
            .and_then(|i| s.devices.get(i).map(|d| d.device.clone()))
            .unwrap_or_else(|| client::Device {
                name: format!("{} {}", self.brand.text().trim(), self.model.text().trim())
                    .trim()
                    .into(),
                family: "other".into(),
                connection: String::new(),
                brand: Some(self.brand.text().trim().into()),
                model: Some(self.model.text().trim().into()),
            });
        device.connection = ["2.4GHz", "USB", "Bluetooth", "unknown"][s.connection].into();
        client::Draft {
            category: ["device", "battery", "dpi", "other"][s.category].into(),
            device,
            description: self.description.text().trim().to_string(),
            diagnostics: s.attach_diagnostics.then(|| self.diagnostics()),
            images: s.images.clone(),
        }
    }
    fn rebuild(&self) {
        self.items.borrow_mut().clear();
        self.labels.borrow_mut().clear();
        self.edit_rects.borrow_mut().clear();
        self.boxes.borrow_mut().clear();
        self.decorations.borrow_mut().clear();
        for edit in [
            self.description.handle,
            self.brand.handle,
            self.model.handle,
            self.reply.handle,
            self.viewer.handle,
        ] {
            visible(edit, false);
        }
        let s = self.state.borrow();
        self.window
            .set_text(text("反馈设备 · X-Battery", "Device feedback · X-Battery"));
        self.button(
            Action::Close,
            text("关闭反馈", "Close feedback"),
            "",
            R::new(609.0, 26.0, 32.0, 32.0),
            Style::Close,
            true,
        );
        let is_sub = matches!(s.view, View::Diagnostics | View::History | View::Thread);
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
                        format!("{} devices detected", s.devices.len())
                    } else {
                        format!("已发现 {} 台设备", s.devices.len())
                    },
                    502.0,
                    y,
                    133.0,
                    19.0,
                    11.0,
                    false,
                    true,
                );
                y += 27.0;
                let device = s.selected.and_then(|i| s.devices.get(i));
                let device_kind = device
                    .map(|d| kind(&d.device.family))
                    .unwrap_or(crate::tray_prefs::TrayKind::Generic);
                self.button(
                    Action::Devices,
                    device
                        .map(|d| d.label.as_str())
                        .unwrap_or(text("列表中没有我的设备", "My device is not listed")),
                    device.map(device_note).unwrap_or_else(|| {
                        text("手动填写品牌和型号", "Enter the brand and model").into()
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
                    let count = s.devices.len().min(128) + 1;
                    self.surface(R::new(25.0, y, 610.0, count as f32 * 35.0 + 8.0), false);
                    y += 4.0;
                    for (i, device) in s.devices.iter().enumerate().take(128) {
                        self.button(
                            Action::Device(i),
                            &device.label,
                            "",
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
                        Style::Option(crate::tray_prefs::TrayKind::Generic, s.selected.is_none()),
                        false,
                    );
                    y += 39.0;
                }
                if device.is_none() {
                    y += 10.0;
                    self.edit(self.brand.handle, R::new(25.0, y, 300.0, 37.0));
                    self.edit(self.model.handle, R::new(335.0, y, 300.0, 37.0));
                    y += 37.0;
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
                    text("电量不准确", "Battery reading"),
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
                        "什么时候出现？你期待看到什么？",
                        "When does it happen? What did you expect?",
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
                    text("附带设备诊断", "Include device diagnostics"),
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
                        "设备信息与近期读取记录，帮助定位问题",
                        "Device details and recent queries to help investigate",
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
                    text("附带设备诊断", "Include device diagnostics"),
                    "",
                    R::new(589.0, y + 23.0, 33.0, 19.0),
                    Style::Toggle(s.attach_diagnostics),
                    false,
                );
                y += 85.0;
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
                    text("设备诊断", "Device diagnostics"),
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
                    text("近期读取记录", "Recent queries"),
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
                        text("暂无查询记录", "No query records yet"),
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
                        "记录请求结果和耗时，不包含原始通信数据。",
                        "Query results and timing only, without raw communication data.",
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
                self.decorations.borrow_mut().push(Decoration::Note(text("不包含序列号、电脑用户名、文件路径或个人配置。提交时仅附带你在此看到的内容。","No serials, account names, file paths or personal settings. Only the details shown here are attached.").into(),R::new(25.0,y,610.0,60.0)));
                y += 80.0;
            }
            View::History => {
                if s.entries.is_empty() {
                    self.decorations.borrow_mut().push(Decoration::Center(
                        text("还没有反馈记录", "No feedback yet").into(),
                        R::new(25.0, 30.0, 610.0, 70.0),
                        13.0,
                        false,
                    ));
                    y = 120.0;
                }
                for (i, entry) in s.entries.iter().enumerate().take(128) {
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
                        R::new(25.0, y, 610.0, 103.0),
                        Style::History,
                        false,
                    );
                    self.tag_last(format!(
                        "{} · {}",
                        entry
                            .number
                            .map(|n| format!("#{n}"))
                            .unwrap_or_else(|| text("待提交", "Pending").into()),
                        short_date(entry.created_at)
                    ));
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
                            if s.busy {
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
        let (primary, label) = match s.view {
            View::Form => (
                Action::Submit,
                if s.busy {
                    text("提交中…", "Submitting…")
                } else {
                    text("提交反馈", "Submit feedback")
                },
            ),
            View::Diagnostics => (Action::Back, text("返回反馈", "Back to feedback")),
            View::History => (Action::New, text("新建反馈", "New feedback")),
            View::Thread => (
                Action::Reply,
                if s.busy {
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
            View::Diagnostics => None,
            View::History => Some((Action::History, text("刷新", "Refresh"), Style::Card)),
            View::Thread => Some((
                Action::OpenIssue,
                text("在 GitHub 查看", "View on GitHub"),
                Style::Card,
            )),
            View::Success => Some((Action::Close, text("完成", "Done"), Style::Plain)),
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
        self.layout();
        self.center();
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
    fn layout(&self) {
        let scale = self.scale.get();
        let scroll = self.scroll.get().clamp(
            0.0,
            (self.content_height.get() - self.body_height()).max(0.0),
        );
        self.scroll.set(scroll);
        set_rect(
            self.body.handle,
            R::new(0.0, BODY_TOP, WIDTH, self.body_height()),
            scale,
        );
        let items = self.items.borrow();
        let mut body = 0;
        let mut root = 0;
        let busy = self.state.borrow().busy;
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
            set_rect(button.handle, r, scale);
            button.set_text(&item.label);
            button.set_enabled(!busy || matches!(item.action, Action::Close));
            button.set_visible(true);
        }
        for b in &self.buttons[body..] {
            b.set_visible(false);
        }
        for b in &self.header_buttons[root..] {
            b.set_visible(false);
        }
        for (handle, rect) in self.edit_rects.borrow().iter() {
            let mut r = *rect;
            r.y -= scroll;
            set_rect(*handle, r, scale);
            visible(*handle, true);
            unsafe {
                wu::EnableWindow(handle.hwnd().unwrap(), (!busy) as i32);
            }
        }
        let mut info: wu::SCROLLINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<wu::SCROLLINFO>() as u32;
        info.fMask = wu::SIF_RANGE | wu::SIF_PAGE | wu::SIF_POS;
        info.nMax = self.content_height.get().ceil() as i32 - 1;
        info.nPage = self.body_height().floor() as u32;
        info.nPos = scroll as i32;
        unsafe {
            wu::SetScrollInfo(
                self.body.handle.hwnd().unwrap(),
                wu::SB_VERT as i32,
                &info,
                1,
            );
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
        self.state.borrow_mut().view = view;
        self.scroll.set(0.0);
        self.rebuild();
    }
    fn error(&self, error: impl Into<String>) {
        self.state.borrow_mut().error = error.into();
        self.scroll.set(f32::MAX);
        self.rebuild();
    }
    fn work(&self, task: impl FnOnce() -> ResultMessage + Send + 'static) {
        if self.state.borrow().busy {
            return;
        }
        let generation = {
            let mut s = self.state.borrow_mut();
            s.busy = true;
            s.error.clear();
            s.generation += 1;
            s.generation
        };
        let results = self.results.clone();
        let notice = self.notice;
        let spawn = std::thread::Builder::new()
            .name("device-feedback".into())
            .spawn(move || {
                let result = task();
                results
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push((generation, result));
                notice.notice();
            });
        if spawn.is_err() {
            self.state.borrow_mut().busy = false;
            self.state.borrow_mut().error = text(
                "无法启动反馈任务，请重试。",
                "Could not start the feedback task. Please retry.",
            )
            .into();
        }
        self.rebuild();
    }
    fn action(&self, action: Action) {
        if self.state.borrow().busy && !matches!(action, Action::Close) {
            return;
        }
        if matches!(
            action,
            Action::Device(_)
                | Action::Manual
                | Action::Connection(_)
                | Action::Category(_)
                | Action::ToggleDiagnostics
                | Action::Example
                | Action::RemoveImage(_)
        ) {
            let mut s = self.state.borrow_mut();
            s.restored_draft = None;
            s.draft_retry = None;
        }
        match action {
            Action::Close => {
                self.remember_reply();
                self.window.set_visible(false);
                self.overlay.set_visible(false);
                unsafe {
                    wu::EnableWindow(self.owner.hwnd().unwrap(), 1);
                    wu::SetForegroundWindow(self.owner.hwnd().unwrap());
                }
            }
            Action::Back => {
                let view = self.state.borrow().view;
                self.remember_reply();
                let target = back_view(view, self.state.borrow().submitted);
                self.set_view(target);
                if target == View::History {
                    self.work(|| ResultMessage::History(client::load_history()));
                }
            }
            Action::History => {
                self.set_view(View::History);
                self.work(|| ResultMessage::History(client::load_history()));
            }
            Action::Devices => {
                let mut s = self.state.borrow_mut();
                s.menu = !s.menu;
                drop(s);
                self.rebuild();
            }
            Action::Device(i) => {
                let mut s = self.state.borrow_mut();
                s.selected = Some(i);
                s.connection = connection_index(&s.devices[i].device.connection);
                s.diagnostics_override = None;
                s.draft_retry = None;
                s.menu = false;
                drop(s);
                self.set_view(View::Form);
            }
            Action::Manual => {
                let mut s = self.state.borrow_mut();
                s.selected = None;
                s.diagnostics_override = None;
                s.draft_retry = None;
                s.menu = false;
                drop(s);
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
                self.viewer.set_text_unix2dos(&self.diagnostics());
                self.set_view(View::Diagnostics);
            }
            Action::Example => {
                self.description.set_text(text("接收器已经连接，官方驱动能看到电量，但这里一直显示未知。唤醒设备后仍然如此。","The receiver is connected and the official driver shows the battery, but X-Battery keeps showing unknown, even after waking the device."));
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
                if self.state.borrow().draft_retry.is_none()
                    && self.state.borrow().selected.is_none()
                    && (self.brand.text().trim().is_empty() || self.model.text().trim().is_empty())
                {
                    self.error(text(
                        "请填写品牌和型号。",
                        "Enter both the brand and model.",
                    ));
                    return;
                }
                if let Err(error) = client::validate_draft(&draft) {
                    self.error(error);
                    return;
                }
                if !client::configured() {
                    self.error(text("应用内提交尚未启用。内容已保留，可复制后前往 GitHub 提交。","In-app submission is not enabled. Your draft is preserved; copy it and submit on GitHub."));
                    return;
                }
                let retry = self.state.borrow().draft_retry.clone();
                self.work(move || {
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
                                .map(|e| e.request_id)
                        })
                    } else {
                        None
                    };
                    ResultMessage::Submitted(result, pending)
                });
            }
            Action::Fallback => match client::fallback(&self.draft()) {
                Ok(fallback) => {
                    if !copy_report(self.window.handle, &fallback.markdown) {
                        self.error(text("剪贴板暂时不可用，请重试。内容仍保留在窗口中。", "The clipboard is unavailable. Retry; your report is still in this window."));
                        return;
                    }
                    if crate::dynlight::shell_open(&fallback.url).is_err() {
                        self.error(text(
                            "内容已复制，但无法打开浏览器。",
                            "Report copied, but the browser could not be opened.",
                        ));
                    } else {
                        self.error(if fallback.images_omitted{text("内容已复制。请在 GitHub 粘贴，并手动添加所选截图。","Report copied. Paste it on GitHub and attach your screenshots manually.")}else{text("内容已复制，请在 GitHub 确认并提交。","Report copied. Review and submit it on GitHub.")});
                    }
                }
                Err(error) => self.error(error),
            },
            Action::Entry(i) => {
                self.remember_reply();
                let entry = self.state.borrow().entries[i].clone();
                self.state.borrow_mut().current = Some(entry.clone());
                if let Some(number) = entry.number {
                    self.viewer.set_text_unix2dos(&entry.description);
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
                    self.work(move || {
                        let pending = client::load_pending_reply(number);
                        ResultMessage::Thread(client::refresh(number), pending)
                    });
                } else {
                    let id = entry.request_id;
                    self.work(move || {
                        let draft = client::load_draft(&id);
                        ResultMessage::Draft(id, draft)
                    });
                }
            }
            Action::Refresh => {
                let number = self.state.borrow().current.as_ref().and_then(|e| e.number);
                if let Some(number) = number {
                    self.work(move || {
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
                let body = self.reply.text().trim().to_string();
                if body.is_empty() || body.chars().count() > 2000 {
                    self.error(text(
                        "补充说明需为 1–2000 字。",
                        "Follow-ups must be 1–2000 characters.",
                    ));
                    return;
                }
                let number = self.state.borrow().current.as_ref().and_then(|e| e.number);
                if let Some(number) = number {
                    self.work(move || ResultMessage::Reply(client::reply(number, &body)));
                }
            }
            Action::New => {
                let mut s = self.state.borrow_mut();
                s.submitted = false;
                s.draft_retry = None;
                s.restored_draft = None;
                s.diagnostics_override = None;
                s.error.clear();
                s.images.clear();
                s.menu = false;
                s.devices = s.latest_devices.clone();
                s.selected = (!s.devices.is_empty()).then_some(0);
                s.connection = s
                    .selected
                    .map(|i| connection_index(&s.devices[i].device.connection))
                    .unwrap_or(0);
                drop(s);
                self.description.set_text("");
                self.brand.set_text("");
                self.model.set_text("");
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
        self.work(move || {
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
            if generation != self.state.borrow().generation {
                continue;
            }
            self.state.borrow_mut().busy = false;
            match message {
                ResultMessage::History(result) => match result {
                    Ok(entries) => self.state.borrow_mut().entries = entries,
                    Err(e) => self.state.borrow_mut().error = e,
                },
                ResultMessage::Submitted(result, pending) => match result {
                    Ok(outcome) => {
                        let mut s = self.state.borrow_mut();
                        s.current = Some(outcome.entry.clone());
                        if outcome.pending {
                            s.draft_retry = Some(outcome.entry.request_id);
                            s.error=text("服务正在确认结果，请稍后重试。重试不会重复创建反馈。","The service is confirming the result. Retry shortly; the same request will be reused.").into();
                        } else {
                            s.view = View::Success;
                            s.submitted = true;
                            s.draft_retry = None;
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
                            let mut content =
                                format!("{}\n\n{}\n\n", thread.issue.title, thread.issue.body);
                            if thread.comments.is_empty() {
                                content.push_str(text(
                                    "暂无维护者回复。",
                                    "No maintainer replies yet.",
                                ));
                            }
                            for comment in &thread.comments {
                                content.push_str(&format!(
                                    "────────────\n{} · {}\n{}\n\n",
                                    comment.author, comment.created_at, comment.body
                                ));
                            }
                            if thread.comments_truncated {
                                content.push_str(text(
                                    "更多回复请在 GitHub 查看。",
                                    "Open GitHub to see more replies.",
                                ));
                            }
                            self.viewer.set_text_unix2dos(&content);
                            self.state.borrow_mut().thread = Some(thread);
                        }
                        Err(e) => self.state.borrow_mut().error = e,
                    }
                }
                ResultMessage::Reply(result) => match result {
                    Ok(_) => {
                        self.reply.set_text("");
                        self.remember_reply();
                        self.action(Action::Refresh);
                    }
                    Err(e) => self.state.borrow_mut().error = e,
                },
                ResultMessage::Draft(id, result) => match result {
                    Ok(draft) => {
                        self.updating.set(true);
                        self.description.set_text_unix2dos(&draft.description);
                        self.brand
                            .set_text(draft.device.brand.as_deref().unwrap_or(""));
                        self.model
                            .set_text(draft.device.model.as_deref().unwrap_or(&draft.device.name));
                        self.updating.set(false);
                        self.state.borrow_mut().restored_draft = Some(draft.clone());
                        let mut s = self.state.borrow_mut();
                        s.draft_retry = Some(id);
                        s.selected = s
                            .devices
                            .iter()
                            .position(|d| d.device.name == draft.device.name);
                        s.connection = connection_index(&draft.device.connection);
                        s.category = ["device", "battery", "dpi", "other"]
                            .iter()
                            .position(|c| *c == draft.category)
                            .unwrap_or(3);
                        s.attach_diagnostics = draft.diagnostics.is_some();
                        s.diagnostics_override = draft.diagnostics;
                        s.images = draft.images;
                        s.view = View::Form;
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
            }
            if !self.state.borrow().error.is_empty() {
                self.scroll.set(f32::MAX);
            }
            self.rebuild();
        }
    }
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
fn device_note(device: &Candidate) -> String {
    if device.battery == "—" {
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
    }
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
        if line.starts_with("Protocol query summary") {
            querying = true;
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
    #[test]
    fn completed_feedback_cannot_return_to_a_submittable_form() {
        let history = back_view(View::Success, true);
        assert_eq!(history, View::History);
        assert_eq!(back_view(history, true), View::Success);
        assert_eq!(back_view(View::Diagnostics, true), View::Success);
        assert_eq!(back_view(View::History, false), View::Form);
        assert_eq!(back_view(View::Thread, false), View::History);
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

fn draw(c: &Controls, p: &mut Painter, body: bool) {
    let t = p.t;
    if body {
        for r in c.boxes.borrow().iter() {
            p.card(*r, 8.0, t.card, t.border_strong);
        }
        for decoration in c.decorations.borrow().iter() {
            match decoration {
                Decoration::Surface(r, soft) => {
                    p.card(*r, 8.0, if *soft { t.bg_sunken } else { t.card }, t.border)
                }
                Decoration::Glyph(icon, r) => feedback_icons::draw(p, *icon, *r, t.accent),
                Decoration::Tag(value, r) => {
                    p.round(*r, 5.0, t.accent_soft);
                    let mut content = *r;
                    if value == text("设备诊断", "Device diagnostics") {
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
                    &format!("{} / 2000", c.description.text().chars().count()),
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
        let sub = matches!(s.view, View::Diagnostics | View::History | View::Thread);
        let title = match s.view {
            View::History => text("我的反馈", "My feedback").into(),
            View::Diagnostics => text("附带的诊断内容", "Included diagnostics").into(),
            View::Thread => s
                .current
                .as_ref()
                .and_then(|e| e.number)
                .map(|n| format!("#{n}"))
                .unwrap_or_else(|| text("反馈详情", "Feedback details").into()),
            _ => text("反馈设备", "Device feedback").into(),
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
            View::History => text(
                "查看回复，也可以继续补充设备信息。",
                "Read replies and add more device details.",
            ),
            View::Diagnostics => text(
                "只包含这次反馈需要的设备信息。",
                "Only the device details needed for this report.",
            ),
            View::Thread => text("反馈详情", "Feedback details"),
            View::Success => "",
            View::Form => text(
                "描述遇到的问题，我们一起完善设备支持。",
                "Tell us what happened. Help improve device support.",
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
            View::Diagnostics => text(
                "可以在反馈窗口关闭“附带设备诊断”。",
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
                "无需 GitHub 账号。反馈及所选附件会公开显示。",
                "No GitHub account needed. Reports and attachments are public.",
            ),
            View::Form => text(
                "反馈和附件将公开。应用内提交服务尚未启用。",
                "Reports and attachments are public. In-app submission is not enabled.",
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
    if matches!(item.action, Action::Diagnostics | Action::ToggleDiagnostics) {
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
            p.text(
                &item.label,
                R::new(36.0, 0.0, r.w - 72.0, r.h),
                12.0,
                false,
                t.text,
                Align::Left,
            );
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
    pub fn new(owner: &nwg::Window, notice: nwg::NoticeSender) -> Result<Self, String> {
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
        super::canvas::set_window_long(hwnd, -16, style | 0x0020_0000 | 0x0200_0000);
        let hwnd = window.handle.hwnd().unwrap() as _;
        let style = super::canvas::get_window_long(hwnd, -16);
        super::canvas::set_window_long(hwnd, -16, style | 0x0200_0000);
        let mut font = nwg::Font::default();
        nwg::Font::builder()
            .family("Microsoft YaHei UI")
            .size_absolute(px(12.0) as u32)
            .build(&mut font)
            .map_err(|e| e.to_string())?;
        let new_box = |readonly: bool| -> Result<nwg::TextBox, String> {
            let mut edit = nwg::TextBox::default();
            edit.handle = nwg::ControlBase::build_hwnd()
                .class_name("EDIT")
                .parent(Some(body.handle))
                .flags(0x5001_1044)
                .build()
                .map_err(|e| e.to_string())?;
            edit.set_font(Some(&font));
            edit.set_text("");
            edit.set_limit(if readonly { 256 * 1024 } else { 4000 });
            edit.set_readonly(readonly);
            Ok(edit)
        };
        let description = new_box(false)?;
        let reply = new_box(false)?;
        let viewer = new_box(true)?;
        let new_input = |limit| -> Result<nwg::TextInput, String> {
            let mut edit = nwg::TextInput::default();
            edit.handle = nwg::ControlBase::build_hwnd()
                .class_name("EDIT")
                .parent(Some(body.handle))
                .flags(0x5001_0080)
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
                .flags(0x5001_000b)
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
            viewer,
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
                    if !callback.updating.get()
                        && (handle == callback.description.handle
                            || handle == callback.brand.handle
                            || handle == callback.model.handle) =>
                {
                    if let Ok(mut s) = callback.state.try_borrow_mut() {
                        s.draft_retry = None;
                        s.restored_draft = None;
                    }
                    unsafe {
                        wu::InvalidateRect(
                            callback.body.handle.hwnd().unwrap(),
                            std::ptr::null(),
                            0,
                        );
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
                        let hint=if is_description{text("例如：接收器已经连接，官方驱动可以看到电量，但这里一直显示未知。","For example: the receiver is connected and the official driver shows the battery, but it stays unknown here.")}else{text("补充测试结果、固件版本或新的问题表现…","Add test results, firmware versions or new observations…")};
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
            let buffer = RefCell::new(super::gfx::Canvas::new());
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
                            let mut canvas = buffer.borrow_mut();
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
                            canvas.paint(
                                &mut fonts,
                                &colors.theme,
                                c.scale.get(),
                                if body { c.scroll.get() } else { 0.0 },
                                rect.right as f32 / c.scale.get(),
                                rect.bottom as f32 / c.scale.get(),
                                |p| draw(&c, p, body),
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
                                let mut buffer = buffer.borrow_mut();
                                let mut fonts = fonts.borrow_mut();
                                let colors = c.colors.borrow();
                                let mem = buffer.ensure(draw.hDC as isize, w, h);
                                crate::win32::FillRect(
                                    mem as _,
                                    &crate::win32::Rect {
                                        left: 0,
                                        top: 0,
                                        right: w,
                                        bottom: h,
                                    },
                                    if matches!(item.action, Action::Diagnostics | Action::ToggleDiagnostics) {
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
                            c.layout();
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
                            c.layout();
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
        let c = &self.controls;
        c.remember_reply();
        *c.colors.borrow_mut() = Colors::new(dark);
        c.update_input_prompts();
        if !c.state.borrow().busy {
            let mut s = c.state.borrow_mut();
            s.latest_devices = devices;
            if !s.initialized {
                s.devices = s.latest_devices.clone();
                s.selected = (!s.devices.is_empty()).then_some(0);
                s.connection = s
                    .selected
                    .map(|i| connection_index(&s.devices[i].device.connection))
                    .unwrap_or(0);
                s.initialized = true;
            }
            if history {
                s.view = View::History;
            } else if s.submitted {
                s.view = View::Success;
            } else {
                s.view = View::Form;
            }
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
        if history && !c.state.borrow().busy {
            c.work(|| ResultMessage::History(client::load_history()));
        }
    }
    pub fn poll(&self) {
        self.controls.complete();
    }
}
impl Drop for Dialog {
    fn drop(&mut self) {
        for raw in self.raw.drain(..) {
            let _ = nwg::unbind_raw_event_handler(&raw);
        }
        if let Some(handler) = self.handler.take() {
            nwg::unbind_event_handler(&handler);
        }
    }
}
