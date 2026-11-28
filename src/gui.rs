
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use native_windows_gui as nwg;

use crate::commands::{run_cmd, run_file_cmd};
use crate::hid;
use crate::proto;
use crate::settings::{self, Settings};
use crate::snapshot::{self, FetchOutcome};
use crate::ui::canvas::{self, Canvas};
use crate::ui::widget::{Act, Drop, Edit, Page, Pref, Slide};
use crate::win32;

mod rapoo_devices;
mod device_workers;
mod rog_refresh;

/// 托盘图标。**编进 exe**，不是运行时读文件。
///
/// 三个理由：绿色版是单文件发布（旁边不会有 `assets\`）、MSIX 里 exe 所在的
/// install 目录只读、`Icon::from_file` 在 strict=false 时读不到会 panic。
///
/// 注意：托盘图标现在由 `TrayCtl` 自己画（要固定 GUID），不再走这个字节流。
/// 留着它是为了**编译期校验** `assets/tray.ico` 一定存在且被打进 exe ——
/// `assets/app.rc` 靠它生成 exe 的图标资源，删了会让构建悄悄少一份资源。
#[used]
static TRAY_ICON: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/tray.ico"));

/// 窗口标题。用产品名，不写具体型号 —— 这是个通用 ROG 鼠标工具。
/// `main.rs` 的单实例检查靠这个标题找已有窗口，两处必须一致。
pub const WINDOW_TITLE: &str = "X-Battery";

/// 应用版本号。
///
/// 来源是 `build.rs` 烧进来的 `ROG_APP_VERSION`：平时等于 `Cargo.toml` 的
/// `version`，CI 发版时被 `ROG_BUILD_VERSION` 覆盖成 tag 的版本号。
/// 以前这里是手写常量，结果 GitHub 发到 v1.1.2 了、软件里还写着 1.1.0。
pub const APP_VERSION: &str = env!("ROG_APP_VERSION");
/// 窗口逻辑尺寸（用户定的 1100×740）。
///
/// 逻辑像素 —— 真正传给 `CreateWindowExW` 时要乘 `scale`
/// （nwg 没开 `high-dpi` feature，它的 `size` 是物理像素）。
const WIN_W: f32 = 1100.0;
const WIN_H: f32 = 740.0;

// ==================== 后台线程 ↔ GUI 线程的交接 ====================

/// 一次读取里「会变的那几项」。每轮轻量读取都会更新。
#[derive(Clone, Copy, Debug)]
struct Quick {
    level: u8,
    charging: bool,
    voltage_mv: u16,
    poll_code: Option<u8>,
    active_stage: Option<u8>,
}

impl Quick {
    fn from_snapshot(snapshot: &snapshot::Snapshot) -> Option<Self> {
        snapshot.battery()?;
        let state = &snapshot.state;
        Some(Self {
            level: state.power,
            charging: state.charging != 0,
            voltage_mv: state.voltage_mv,
            poll_code: snapshot.parsed_frame("12:04:00").then_some(state.poll_code),
            active_stage: snapshot.parsed_frame("12:00:00").then_some(state.active_stage),
        })
    }

    fn apply_to(self, state: &mut proto::State) {
        state.power = self.level;
        state.charging = u8::from(self.charging);
        state.voltage_mv = self.voltage_mv;
        if let Some(code) = self.poll_code { state.poll_code = code; }
        if let Some(stage) = self.active_stage { state.active_stage = stage; }
    }
}

#[derive(Default)]
enum Status {
    #[default]
    Idle,
    /// 正常读到数据。
    Ok,
    /// 设备在休眠 —— 这是**正常状态**，不是错误。UI 上要显示「移动一下鼠标」。
    Asleep,
    /// 后台正在执行一条写命令。
    Busy,
    /// 真的出错了，要用户去排查。
    Error(String),
}

/// 一条要写设备的命令。
///
/// **所有校验和写入逻辑都在 [`crate::ops`] 里**，这里只负责把界面上的意图
/// 编码成一个可以跨线程传递的值。
#[derive(Clone)]
pub(crate) enum Cmd {
    BasicDpi { target: crate::backends::Target, dpi: u16 },
    BasicDpiStage { target: crate::backends::Target, stage: crate::device::DpiStage, context: crate::device::StageContext },
    BasicDpiStageAdd { target: crate::backends::Target, context: crate::device::StageContext, x: u16, y: u16 },
    BasicDpiStageRemove { target: crate::backends::Target, stage: crate::device::DpiStage, context: crate::device::StageContext },
    BasicDpiEdit { target: crate::backends::Target, stage: crate::device::DpiStage, context: crate::device::StageContext, expected_y: u16, x: u16, y: u16 },
    BasicDpiPreset { target: crate::backends::Target, stage: crate::device::DpiStage, context: crate::device::StageContext, expected_y: u16, value: u16 },
    BasicPollingRate { target: crate::backends::Target, hz: u16 },
    Poll(i32),
    Dpi {
        stage: usize,
        dpi: i32,
        rgb: Option<[u8; 3]>,
    },
    Stage(u8),
    Debounce(u8),
    Snap(u8),
    Tuning(i16),
    Lod(u8),
    /// 灯光。三个 `Option` 字段为 `None` 表示「沿用设备当前值」。
    ///
    /// 不能传 `Some(0)`：`ops::set_light` 的 `probe`/`expect` 会把这三个字段
    /// 纳入读回断言，写死成 0 既会把设备原有值清零，又会让断言永不相等
    /// （报「写进去了但读回不一致」）。
    Light {
        mode: u8,
        bright: Option<u8>,
        rgb: Option<[u8; 3]>,
        dir: Option<u8>,
        random: Option<u8>,
        speed: Option<u8>,
    },
    Button {
        slot: usize,
        dst: u16,
        confirmed: bool,
    },
    /// 能量块三项。写它们会顺带 commit（`ops::set_*` 内部处理）。
    Sleep(u8),
    LowBatt(u8),
    FullEffect(u8),
    /// 开机自启。**不碰 HID**，在 `run_cmd` 里最先分叉。
    AutoStart(bool),
    /// 备份到文件（需要一次 17 帧的全量读取，所以放后台线程）。
    Backup(PathBuf),
    /// 从文件还原。
    Restore(PathBuf),
}

impl Cmd {
    fn basic_target(&self) -> Option<&crate::backends::Target> {
        match self {
            Self::BasicDpi { target, .. } | Self::BasicDpiStage { target, .. }
                | Self::BasicDpiStageAdd { target, .. } | Self::BasicDpiStageRemove { target, .. }
                | Self::BasicDpiEdit { target, .. } | Self::BasicDpiPreset { target, .. }
                | Self::BasicPollingRate { target, .. } => Some(target),
            _ => None,
        }
    }
    /// 结果行 / 提示里用的中文短名。
    pub(crate) fn label(&self) -> String {
        match self {
            Cmd::BasicDpi { dpi, .. } => format!("DPI {dpi}"),
            Cmd::BasicDpiPreset { value, .. } => format!("DPI {value}"),
            Cmd::BasicPollingRate { hz, .. } => crate::trf!("回报率 {hz} Hz", hz = hz),
            Cmd::BasicDpiEdit { stage, x, y, .. } => format!("DPI {} · X {x} / Y {y}", stage.index + 1),
            Cmd::BasicDpiStage { stage, .. } => crate::trf!("DPI 档 {stage} → {dpi}",
                stage = usize::from(stage.index) + 1, dpi = stage.value),
            Cmd::BasicDpiStageAdd { x, y, .. } => format!("DPI + · X {x} / Y {y}"),
            Cmd::BasicDpiStageRemove { stage, .. } => format!("− DPI {} · {}", stage.index + 1, stage.value),
            Cmd::Poll(hz) => crate::trf!("回报率 {hz} Hz", hz = hz),
            Cmd::Dpi { stage, dpi, .. } => {
                crate::trf!("DPI 档 {stage} → {dpi}", dpi = dpi, stage = stage)
            }
            Cmd::Stage(s) => crate::trf!("当前档位 {s}", s = s),
            Cmd::Debounce(c) => crate::trf!("去抖 {c}", c = c),
            Cmd::Snap(v) => crate::trf!("角度吸附 {v}", v = v),
            Cmd::Tuning(v) => crate::trf!("角度微调 {v}", v = v),
            Cmd::Lod(v) => crate::trf!("抬起距离 {}", proto::lod_name(*v)),
            Cmd::Light { mode, .. } => crate::trf!("灯光 {}", crate::i18n::translate(&proto::light_mode_name(*mode))),
            Cmd::Button { slot, dst, .. } => {
                crate::trf!("按键槽 {slot} → 0x{dst:04X}", dst = dst, slot = slot)
            }
            Cmd::Sleep(c) => crate::trf!("自动休眠 {}", crate::i18n::translate(&proto::sleep_mode_name(*c))),
            Cmd::LowBatt(p) => crate::trf!("低电量阈值 {p}%", p = p),
            Cmd::FullEffect(v) => crate::trf!(
                "充满效果 {}",
                if *v != 0 {
                    crate::tr!("开")
                } else {
                    crate::tr!("关")
                }
            ),
            Cmd::AutoStart(on) => {
                if *on {
                    crate::tr!("开机自启开启").to_string()
                } else {
                    crate::tr!("开机自启关闭").to_string()
                }
            }
            Cmd::Backup(_) => crate::tr!("导出配置").to_string(),
            Cmd::Restore(_) => crate::tr!("还原配置").to_string(),
        }
    }
}

/// 后台线程执行一条命令后的结果。
///
/// `hint` 在那一刻就查好，避免把 [`crate::ops::ErrorKind`] 搬过线程边界
/// （它没有跨线程的必要，而 hint 是一张静态表）。
pub(crate) struct CmdResult {
    pub(crate) label: String,
    pub(crate) ok: Option<String>,
    pub(crate) err: Option<String>,
    /// 失败时该提醒用户做什么（「请确认鼠标接收器已插好」这类）。
    /// 由后台线程在出事那一刻查好，避免把 [`crate::ops::ErrorKind`] 搬过线程边界。
    pub(crate) hint: &'static str,
}

impl CmdResult {
    /// 变成界面底部那条浮层提示（精简文案）。
    fn toast(&self) -> String {
        match (&self.ok, &self.err) {
            (Some(m), _) => {
                if m == "已生效" || m == "Applied" {
                    crate::trf!("✔ {}已生效", self.label)
                } else {
                    crate::trf!("✔ {}：{m}", self.label, m = m)
                }
            }
            (None, Some(m)) => {
                let base = if m.contains("错误码 5") || m.contains("Win32 错误码 5") {
                    crate::trf!("✘ {}失败：权限不足 (错误 5)", self.label)
                } else if m.starts_with("无法写入") || m.starts_with("打开") {
                    crate::trf!("✘ {}：{m}", self.label, m = m)
                } else {
                    crate::trf!("✘ {}失败：{m}", self.label, m = m)
                };
                // `hint` 以前只写不读 —— 后台线程辛苦查出的「请确认接收器已插好」
                // 从来没到过用户眼前。失败时补在后面，成功时无关。
                if self.hint.is_empty() {
                    base
                } else {
                    crate::trf!("{base}；{}", self.hint, base = base)
                }
            }
            (None, None) => format!("✔ {}", self.label),
        }
    }
}

/// 低于这个电量提醒一次。
const LOW_ALERT_PCT: u8 = 20;
/// 回充到这个电量以上才重新允许提醒 —— 否则在 20% 附近上下抖动会反复弹。
const LOW_ALERT_REARM_PCT: u8 = 25;

/// 低电量提醒的判据（纯函数，便于测）。
///
/// `armed` 是「还没提醒过」的闸门：跌到阈值以下提醒一次后关闸，回充到
/// `LOW_ALERT_REARM_PCT` 以上、或者开始充电，才重新上膛。
fn low_battery_alert(level: u8, charging: bool, armed: &mut bool) -> bool {
    if charging || level > LOW_ALERT_REARM_PCT {
        *armed = true;
        return false;
    }
    if level > LOW_ALERT_PCT || !*armed {
        return false;
    }
    *armed = false;
    true
}

fn persist_device_reorder(shared: &mut Shared, displayed: &[String], from: usize, to: usize) {
    // 排列是用户偏好，与品牌读数分开保存。
    // Validate and save while holding the same lock as backend publication.
    let Some(order) = reordered_device_keys(displayed, &shared.readings, from, to) else { return; };
    let mut cfg = shared.cfg.clone();
    cfg.set_device_order(order);
    match settings::save(&cfg) {
        Ok(()) => {
            shared.cfg = cfg;
            shared.sort_readings();
        }
        Err(error) => crate::diagnostics::event(
            "device_order_save_failed",
            &format!("error={error}"),
        ),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowCloseAction {
    Ask,
    Hide,
    Minimize,
    Quit,
}

fn window_close_action(mode: &str, tray_enabled: bool, taskbar_visible: bool) -> WindowCloseAction {
    match mode {
        "ask" => WindowCloseAction::Ask,
        "minimize" if tray_enabled || taskbar_visible => WindowCloseAction::Hide,
        "minimize" => WindowCloseAction::Minimize,
        _ => WindowCloseAction::Quit,
    }
}

fn reordered_device_keys(
    displayed: &[String], readings: &[crate::device::DeviceReading], from: usize, to: usize,
) -> Option<Vec<String>> {
    if from == to || from >= displayed.len() || to >= displayed.len()
        || displayed.iter().ne(readings.iter().map(|reading| &reading.key)) {
        return None;
    }
    let mut order = displayed.to_vec();
    let key = order.remove(from);
    order.insert(to, key);
    Some(order)
}

fn low_battery_reading(reading: &crate::device::DeviceReading, armed: &mut bool) -> Option<u8> {
    if reading.quality != crate::device::ReadingQuality::Live
        || reading.battery.as_ref().is_some_and(|b| b.approximate) { return None; }
    let percent = reading.percent()?;
    low_battery_alert(percent, reading.is_charging(), armed).then_some(percent)
}

#[derive(Clone)]
struct DeviceSnapshot {
    reading: crate::device::DeviceReading,
    state: Option<proto::State>,
    ident: proto::Identity,
    link: Option<hid::Link>,
    full_at: std::time::Instant,
    basic: Option<crate::backends::Snapshot>,
    basic_status: Option<crate::backends::status::Status>,
    battery_at: Option<std::time::Instant>,
}

fn feedback_candidates(shared: &Shared) -> Vec<crate::feedback::Candidate> {
    shared.readings.iter().map(|reading| {
        let record = shared.devices.get(&reading.key);
        let basic = record.and_then(|record| record.basic.as_ref());
        let link = record.and_then(|record| record.link.as_ref());
        let interface = basic.and_then(|basic| basic.target.hid().ok()).or_else(|| link.map(|link| &link.iface));
        let backend = basic.map(|basic| basic.target.backend).or_else(|| link.map(|_| crate::backends::BackendId::Rog));
        let family = match shared.cfg.device_kind(reading) {
            crate::tray_prefs::TrayKind::Mouse => crate::device::DeviceFamily::Mouse,
            crate::tray_prefs::TrayKind::Keyboard => crate::device::DeviceFamily::Keyboard,
            crate::tray_prefs::TrayKind::Headset => crate::device::DeviceFamily::Headset,
            crate::tray_prefs::TrayKind::Gamepad => crate::device::DeviceFamily::Gamepad,
            _ => crate::device::DeviceFamily::Unknown,
        };
        let connection = link.map(|link| match link.mode.as_str() {
            "Wireless24G" => "2.4GHz", "Wired" => "USB", "Bluetooth" => "Bluetooth", _ => "unknown",
        }).unwrap_or("unknown");
        let lifetime = backend.filter(|backend| *backend != crate::backends::BackendId::Rog)
            .map(|backend| backend.sample_lifetime()).unwrap_or_else(|| rog_sample_lifetime(&shared.cfg));
        let address = basic.map(|basic| basic.target.address);
        let (dpi, polling_rate) = if let Some(basic) = basic {
            (basic.dpi.clone(), basic.polling_rate.clone())
        } else if let Some(state) = record.and_then(|record| record.state.as_ref())
            .filter(|_| family == crate::device::DeviceFamily::Mouse) {
            let stages: Vec<_> = (0..proto::DPI_STAGES).filter_map(|index|
                u16::try_from(state.dpi(index)).ok().map(|value| crate::device::DpiStage { index: index as u8, value })).collect();
            let active = state.active_stage.checked_sub(1).filter(|index| usize::from(*index) < proto::DPI_STAGES);
            let dpi = crate::device::Dpi {
                current: active.and_then(|index| stages.iter().find(|stage| stage.index == index).map(|stage| stage.value)),
                active_stage: active, stages, profile: Some(u16::from(state.profile)),
                writable: link.is_some_and(|link| !link.read_only), ..Default::default()
            };
            let polling_rate = crate::device::PollingRate {
                current: [125, 250, 500, 1000, 2000, 4000, 8000].get(usize::from(state.poll_code)).copied(),
                ..Default::default()
            };
            (dpi, polling_rate)
        } else { Default::default() };
        crate::feedback::candidate(shared.cfg.device_name(reading).to_string(), reading, interface,
            backend, address, connection, family, record.and_then(|record| record.battery_at), lifetime,
            &dpi, &polling_rate)
    }).collect()
}

fn record_reading_diagnostic(surface: crate::diagnostics::readings::Surface,
    reading: &crate::device::DeviceReading, record: Option<&DeviceSnapshot>, cfg: &Settings,
    enabled: bool, now: std::time::Instant) {
    let matching = record.filter(|record| &record.reading == reading);
    let interface = matching.and_then(|record| record.basic.as_ref()
        .and_then(|snapshot| snapshot.target.hid().ok()).or_else(|| record.link.as_ref().map(|link| &link.iface)));
    let lifetime = matching.map(|record| record.basic.as_ref().map(|snapshot| snapshot.target.backend.sample_lifetime())
        .unwrap_or_else(|| rog_sample_lifetime(cfg)));
    crate::diagnostics::readings::observe(surface, reading, matching.and_then(|record| record.battery_at),
        interface.map(|interface| interface.path.as_str()), lifetime,
        enabled && (reading.display_percent().is_some() || reading.coarse_level().is_some()), now);
}

fn rog_sample_lifetime(settings: &Settings) -> Duration {
    Duration::from_secs(settings.refresh_interval_sec.saturating_mul(3).max(180))
}

impl DeviceSnapshot {
    fn command_target(&self) -> Option<crate::commands::RogTarget> {
        Some(crate::commands::RogTarget {
            link: self.link.clone()?,
            identity: snapshot::device_identity(&self.ident),
        })
    }
}

fn diagnostic_link(link: &hid::Link) -> crate::diagnostics::DeviceInfo {
    let receiver = (link.mode == "Wireless24G").then(|| {
        link.all_interfaces
            .iter()
            .find(|iface| crate::devices::find_dongle(iface.product_id).is_some())
            .unwrap_or(&link.iface)
    });
    crate::diagnostics::DeviceInfo {
        model: link.iface.product.clone(),
        key: link.device_key(),
        link_mode: link.mode.clone(),
        wired_pid: 0,
        wireless_pid: 0,
        receiver_name: receiver
            .map(|iface| {
                crate::devices::find_dongle(iface.product_id)
                    .map(|dongle| dongle.name_zh.to_string())
                    .unwrap_or_else(|| iface.product.clone())
            })
            .unwrap_or_default(),
        receiver_vendor_id: receiver.map(|iface| iface.vendor_id).unwrap_or_default(),
        receiver_product_id: receiver.map(|iface| iface.product_id).unwrap_or_default(),
        receiver_serial: String::new(),
        receiver_mac: String::new(),
        receiver_firmware: String::new(),
    }
}

fn diagnostic_device(record: &DeviceSnapshot) -> crate::diagnostics::DeviceInfo {
    let mut info = record
        .link
        .as_ref()
        .map(diagnostic_link)
        .unwrap_or_default();
    info.model = record.ident.display_name();
    info.key = record.reading.key.clone();
    info.wired_pid = record.ident.wired_pid;
    info.wireless_pid = record.ident.wireless_pid;
    info.receiver_serial = record.ident.dongle_sn.clone();
    info.receiver_mac = record.ident.dongle_mac.clone();
    info.receiver_firmware = record.ident.dongle_firmware.clone();
    info
}

fn diagnostic_facts<'a>(
    operation: &'a str,
    status: &'a str,
    family: &'a str,
    frames: usize,
    elapsed_ms: u128,
    device: Option<&'a crate::diagnostics::DeviceInfo>,
) -> crate::diagnostics::Facts<'a> {
    crate::diagnostics::Facts {
        version: APP_VERSION,
        operation,
        status,
        family,
        frames,
        elapsed_ms,
        history_rows: crate::history::cached_row_count(),
        device,
    }
}

fn rog_record(link: &hid::Link, outcome: &FetchOutcome, previous: Option<&DeviceSnapshot>) -> Option<DeviceSnapshot> {
    let current = &outcome.snapshot.identity;
    if current.device_id().is_none() && previous.is_some_and(|device| device.ident.device_id().is_some()) {
        return None;
    }
    let battery = outcome.snapshot.battery()?;
    let same = previous.filter(|device| device.ident.same_device(current)
        || (device.ident.device_id().is_none() && current.device_id().is_none()));
    let mut ident = same.map(|device| device.ident.clone()).unwrap_or_default();
    for (stored, observed) in [
        (&mut ident.model_name, &current.model_name),
        (&mut ident.mouse_sn, &current.mouse_sn),
        (&mut ident.dongle_sn, &current.dongle_sn),
        (&mut ident.dongle_mac, &current.dongle_mac),
        (&mut ident.dongle_firmware, &current.dongle_firmware),
    ] {
        if !observed.is_empty() { stored.clone_from(observed); }
    }
    if (proto::Identity { mouse_mac: current.mouse_mac.clone(), ..Default::default() }).device_id().is_some() {
        ident.mouse_mac.clone_from(&current.mouse_mac);
    }
    if current.wired_pid != 0 { ident.wired_pid = current.wired_pid; }
    if current.wireless_pid != 0 { ident.wireless_pid = current.wireless_pid; }
    if outcome.snapshot.frames.contains_key("7D:20:02") { ident.booster = current.booster; }
    if ident.model_name.is_empty() { ident.model_name = link.iface.product.clone(); }
    let key = crate::backends::rog::identity_key(link, &ident);
    let name = ident.display_name();
    let family = crate::device::family_of(&name);
    let configuration = outcome.snapshot.configuration().ok();
    let mut state = configuration.as_ref().map(|configuration| configuration.state().clone())
        .or_else(|| same.and_then(|device| device.state.clone()));
    if let (Some(state), Some(quick)) = (&mut state, Quick::from_snapshot(&outcome.snapshot)) {
        quick.apply_to(state);
    }
    if link.read_only || ident.device_id().is_none() { state = None; }
    Some(DeviceSnapshot {
        reading: crate::device::DeviceReading {
            key, name: if name.is_empty() { family.noun().into() } else { name }, family,
            battery: Some(battery), connected: true, quality: crate::device::ReadingQuality::Live,
        },
        state, ident, link: Some(link.clone()), basic: None, basic_status: None,
        battery_at: outcome.snapshot.taken,
        full_at: if configuration.is_some() { std::time::Instant::now() }
            else { same.map(|device| device.full_at).unwrap_or_else(std::time::Instant::now) },
    })
}

fn mark_rog_unread(device: &mut DeviceSnapshot, present: Option<&std::collections::BTreeSet<String>>,
    now: std::time::Instant, lifetime: Duration) {
    if present.is_some_and(|paths| device.link.as_ref().is_some_and(|link|
        !paths.iter().any(|path| path.eq_ignore_ascii_case(&link.iface.path)))) {
        device.reading.connected = false;
    }
    device.state = None;
    device.reading.quality = match device.battery_at {
        Some(measured) if device.reading.connected && now.saturating_duration_since(measured) < lifetime =>
            crate::device::ReadingQuality::Cached,
        Some(_) => crate::device::ReadingQuality::Stale,
        None => crate::device::ReadingQuality::Unknown,
    };
}

fn reconcile_rog_identity(records: &mut BTreeMap<String, DeviceSnapshot>, link: &hid::Link,
    identity: &proto::Identity) -> Option<DeviceSnapshot> {
    let link_key = link.device_key();
    let same = records.values().filter(|previous| previous.ident.same_device(identity))
        .max_by_key(|previous| previous.battery_at).cloned();
    if same.is_some() {
        records.retain(|_, previous| !(previous.ident.device_id().is_none()
            && previous.link.as_ref().is_some_and(|previous| previous.device_key() == link_key)));
    }
    for previous in records.values_mut().filter(|previous|
        previous.link.as_ref().is_some_and(|previous| previous.device_key() == link_key)
            && previous.ident.conflicts_with(identity)) {
        previous.reading.connected = false;
        previous.reading.quality = crate::device::ReadingQuality::Stale;
        previous.state = None;
    }
    same
}


fn merge_additional(shared: &mut Shared, backend: crate::backends::BackendId, poll: crate::backends::Poll) {
    merge_additional_at(shared, backend, poll, std::time::Instant::now());
}

fn legacy_rog_history_owner(
    devices: &std::collections::BTreeMap<String, DeviceSnapshot>,
) -> Option<String> {
    let owners: Vec<_> = devices
        .values()
        .filter(|device| {
            device.basic.is_none()
                && device.link.is_some()
                && device.reading.family == crate::device::DeviceFamily::Mouse
        })
        .collect();
    (owners.len() == 1).then(|| owners[0].reading.key.clone())
}

fn coalesce_rapoo_paired_poll(shared: &mut Shared, poll: &mut crate::backends::Poll) {
    let existing = shared.devices.keys().cloned().collect();
    let changes = shared.rapoo_devices.coalesce(poll, &existing);
    if !changes.retired_keys.is_empty() {
        shared.history_epoch = shared.history_epoch.wrapping_add(1);
        shared.key_aliases.retain(|old, _| !changes.retired_keys.contains(old));
        let (samples, mut aliases) = shared.history_tracker.take_pending();
        aliases.retain(|(old, _)| !changes.retired_keys.contains(old));
        shared.history_tracker.retry(samples, aliases);
        for key in &changes.interrupted_keys { shared.history_tracker.interrupt(key); }
        shared.devices.retain(|key, device| !device.basic.as_ref().is_some_and(|basic|
            shared.rapoo_devices.card_key(&basic.target).is_some_and(|owner| owner != key.as_str())));
        if let Some(queue) = shared.source_pending.get_mut(&crate::backends::BackendId::Rapoo) {
            queue.retain(|cmd| {
                let changed = cmd.basic_target().is_some_and(|target|
                    shared.rapoo_devices.card_key(target).is_some_and(|owner| owner != target.key.as_str()));
                if changed {
                    shared.results.push_back(CmdResult { label: cmd.label(), ok: None,
                        err: Some("设备关联已变化，请重新选择后重试".into()), hint: "" });
                }
                !changed
            });
        }
    }
    for (old, new) in changes.migrations {
        if shared.promote_device_key(&old, &new) { shared.device_keys_dirty = true; }
        if let Some(mut device) = shared.devices.remove(&old) {
            device.reading.key = new.clone();
            if let Some(basic) = &mut device.basic { basic.target.key = new.clone(); }
            shared.devices.entry(new).or_insert(device);
        }
    }
}

fn merge_additional_at(shared: &mut Shared, backend: crate::backends::BackendId, poll: crate::backends::Poll,
    now: std::time::Instant) {
    merge_additional_update(shared, backend, poll, now, Publication::Complete);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Publication { Complete, Presence, Battery, Detail }

fn dpi_context_changed(live: &crate::device::Dpi, stored: &crate::device::Dpi) -> bool {
    [(live.current, stored.current), (live.current_y, stored.current_y), (live.profile, stored.profile)]
        .iter().any(|(new, old)| new.is_some() && new != old)
        || live.active_stage.is_some_and(|stage| Some(stage) != stored.active_stage)
}

fn preserve_configuration(snapshot: &mut crate::backends::Snapshot, previous: &DeviceSnapshot,
    now: std::time::Instant) {
    let Some(stored) = &previous.basic else { return; };
    if !snapshot.connected || !stored.connected || !snapshot.target.same_channel(&stored.target)
        || snapshot.target.key != stored.target.key || snapshot.target.verified_identity != stored.target.verified_identity {
        return;
    }
    let live = snapshot.dpi.clone();
    if dpi_context_changed(&live, &stored.dpi) { return; }
    snapshot.dpi = stored.dpi.clone();
    for (stored, live) in [(&mut snapshot.dpi.current, live.current),
        (&mut snapshot.dpi.current_y, live.current_y), (&mut snapshot.dpi.profile, live.profile)] {
        if live.is_some() { *stored = live; }
    }
    if live.active_stage.is_some() { snapshot.dpi.active_stage = live.active_stage; }
    snapshot.polling_rate = stored.polling_rate.clone();
    if now.saturating_duration_since(previous.full_at) >= Duration::from_secs(30) {
        snapshot.disable_writes();
    }
}

fn merge_additional_update(shared: &mut Shared, backend: crate::backends::BackendId,
    mut poll: crate::backends::Poll, now: std::time::Instant, publication: Publication) {
    let incoming: Vec<_> = poll.snapshots.iter().map(|snapshot| snapshot.target.clone()).collect();
    if backend==crate::backends::BackendId::Rapoo {
        if let Some(paths) = &poll.present_channels {
            shared.rapoo_endpoint_snapshots.retain(|(path, _), _|
                paths.iter().any(|present| present.eq_ignore_ascii_case(path)));
        }
        for snapshot in &poll.snapshots {
            let mut endpoint = snapshot.clone();
            endpoint.disable_writes();
            shared.rapoo_endpoint_snapshots.insert(
                (endpoint.target.channel_key().to_ascii_lowercase(), endpoint.target.address), endpoint);
        }
        if matches!(publication, Publication::Battery | Publication::Detail) {
            let siblings: Vec<_> = shared.rapoo_endpoint_snapshots.values()
                .filter(|snapshot| snapshot.target.backend == backend
                    && !incoming.iter().any(|target| target.same_channel(&snapshot.target))
                    && poll.present_channels.as_ref().is_some_and(|paths| paths.iter()
                        .any(|path| path.eq_ignore_ascii_case(&snapshot.target.channel_key())))
                    && crate::backends::rapoo::device_signature(&snapshot.target).is_some_and(|signature|
                        !signature.serial.trim().is_empty() && incoming.iter().any(|target|
                            crate::backends::rapoo::device_signature(target).is_some_and(|other|
                                other.model == signature.model && other.serial == signature.serial))))
                .cloned().map(|mut snapshot| {
                    if let Some(previous) = shared.rapoo_devices.card_key(&snapshot.target)
                        .and_then(|key| shared.devices.get(key)) {
                        snapshot.target.key = previous.reading.key.clone();
                        preserve_configuration(&mut snapshot, previous, now);
                    }
                    snapshot.target.key = snapshot.target.unverified_key();
                    snapshot
                }).collect();
            poll.snapshots.extend(siblings);
        }
        for snapshot in &poll.snapshots {
            let Ok(i)=snapshot.target.hid() else {continue;};
            let old=crate::device::stable_key(&format!("{}:{:04x}:{:04x}:{}:{}",backend.name(),i.vendor_id,i.product_id,i.serial,snapshot.target.address));
            if poll.legacy_key_migrations.get(&old) != Some(&snapshot.target.key) { continue; }
            if old!=snapshot.target.key && (shared.cfg.device_names.contains_key(&old)
                || shared.cfg.device_tray.contains_key(&old) || shared.cfg.dpi_presets.contains_key(&old)
                || shared.cfg.device_order.contains(&old) || shared.cfg.tray_device.as_ref()==Some(&old)) {
                if shared.promote_device_key(&old,&snapshot.target.key) {shared.device_keys_dirty=true;}
                shared.cfg.device_names.remove(&old);
                shared.cfg.device_tray.remove(&old);
                shared.devices.remove(&old);
            }
        }
        coalesce_rapoo_paired_poll(shared, &mut poll);
    }
    for device in shared.devices.values_mut() {
        if matches!(publication, Publication::Battery | Publication::Detail) { continue; }
        if let Some(basic) = &mut device.basic {
            if basic.target.backend != backend { continue; }
            let connection = poll.present_channels.as_ref().and_then(|paths| {
                let present = if backend == crate::backends::BackendId::Rapoo {
                    shared.rapoo_devices.is_present(&device.reading.key)
                        .unwrap_or_else(|| paths.contains(&basic.target.channel_key()))
                } else { paths.contains(&basic.target.channel_key()) };
                (!present).then_some(false)
            });
            let status = device.basic_status.get_or_insert_with(Default::default);
            if publication == Publication::Presence && connection.is_none()
                && shared.source_discoveries.get(&backend).is_some_and(|scan|
                    scan.targets.iter().any(|target| target.same_channel(&basic.target))) { continue; }
            status.observe(connection, None, now, backend.sample_lifetime());
            if backend == crate::backends::BackendId::XInput && connection == Some(false) {
                // A slot can be reused by a different controller after disconnection.
                status.clear_battery();
                basic.battery = None;
                basic.battery_at = None;
                device.reading.battery = None;
            }
            basic.connected = status.connected;
            basic.disable_writes();
            device.reading.connected = status.connected;
            device.reading.quality = status.quality;
        }
    }
    for mut snapshot in poll.snapshots {
        if backend == crate::backends::BackendId::Bluetooth
            && shared.devices.values().any(|device| {
                device.basic.as_ref().is_some_and(|other| {
                    other.connected
                        && other.target.backend != crate::backends::BackendId::Bluetooth
                        && crate::backends::bluetooth::same_physical_device(
                            &snapshot.target,
                            &other.target,
                        )
                })
            })
        {
            continue;
        }
        if backend != crate::backends::BackendId::Bluetooth {
            let duplicates: Vec<String> = shared
                .devices
                .iter()
                .filter_map(|(key, device)| {
                    device.basic.as_ref().and_then(|other| {
                        (other.target.backend == crate::backends::BackendId::Bluetooth
                            && crate::backends::bluetooth::same_physical_device(
                                &other.target,
                                &snapshot.target,
                            ))
                        .then(|| key.clone())
                    })
                })
                .collect();
            for old_key in duplicates {
                if shared.promote_device_key(&old_key, &snapshot.target.key) {
                    shared.device_keys_dirty = true;
                }
                shared.devices.remove(&old_key);
            }
        }
        if backend == crate::backends::BackendId::StandardHid && shared.devices.values().any(|device|
            device.basic.as_ref().is_some_and(|other| other.target.backend.precedes_standard()
                && other.connected && other.target.same_hid_path(&snapshot.target))) { continue; }
        if backend.precedes_standard() {
            shared.devices.retain(|_, device| !device.basic.as_ref().is_some_and(|other|
                other.target.backend == crate::backends::BackendId::StandardHid && other.target.same_hid_path(&snapshot.target)));
        }
        let previous = shared.devices.values().filter(|device| device.basic.as_ref().is_some_and(|other|
            other.target.same_channel(&snapshot.target)))
            .max_by_key(|device| (device.reading.connected, device.full_at))
            .and_then(|device| device.basic.as_ref()).map(|other| other.target.clone());
        if previous.is_none() && snapshot.target.verified_identity {
            let discovery_key = snapshot.target.unverified_key();
            if discovery_key != snapshot.target.key && !shared.key_aliases.contains_key(&discovery_key)
                && shared.promote_device_key(&discovery_key, &snapshot.target.key) {
                shared.device_keys_dirty = true;
            }
        }
        if previous.as_ref().is_some_and(|previous| previous.verified_identity && !snapshot.target.verified_identity) {
            for device in shared.devices.values_mut().filter(|device| device.basic.as_ref()
                .is_some_and(|basic| basic.target.same_channel(&snapshot.target))) {
                if let Some(basic) = &mut device.basic { basic.disable_writes(); }
                if let Some(status) = &mut device.basic_status {
                    status.observe(None, None, now, backend.sample_lifetime());
                    device.reading.quality = status.quality;
                }
            }
            continue;
        }
        if let Some(previous) = previous.filter(|previous| previous.key != snapshot.target.key) {
            if snapshot.target.verified_identity && !previous.verified_identity {
                if shared.promote_device_key(&previous.key, &snapshot.target.key) { shared.device_keys_dirty = true; }
                shared.devices.remove(&previous.key);
            } else if let Some(device) = shared.devices.get_mut(&previous.key) {
                if let Some(status) = &mut device.basic_status {
                    status.observe(Some(false), None, now, backend.sample_lifetime());
                }
                device.reading.connected = false;
                device.reading.quality = crate::device::ReadingQuality::Stale;
                if let Some(basic) = &mut device.basic { basic.connected = false; }
            }
        }
        let key = snapshot.target.key.clone();
        if backend == crate::backends::BackendId::XInput && !shared.devices.contains_key(&key) {
            // XInput identifies a slot, not a physical controller across restarts.
            shared.history_tracker.interrupt(&key);
        }
        let previous = shared.devices.get(&key);
        let battery_only = publication == Publication::Battery || (publication == Publication::Detail
            && !incoming.iter().any(|target| target.same_channel(&snapshot.target)));
        if battery_only {
            if let Some(previous) = previous { preserve_configuration(&mut snapshot, previous, now); }
        }
        let full_at = if battery_only {
            previous.map(|previous| previous.full_at).unwrap_or(now)
        } else { now };
        let mut status = previous.and_then(|device| device.basic_status.clone()).unwrap_or_default();
        status.observe_sample(Some(snapshot.connected), snapshot.battery.as_ref(), snapshot.battery_at,
            now, backend.sample_lifetime());
        if snapshot.battery_unavailable { status.clear_battery(); }
        if snapshot.dpi.current.is_none() && !(battery_only && snapshot.dpi.profile.is_some()) {
            if let Some(previous) = previous.and_then(|device| device.basic.as_ref()) {
                snapshot.dpi = previous.dpi.clone();
                snapshot.dpi.disable_writes();
            }
        }
        let reading = crate::device::DeviceReading {
            key: key.clone(), name: snapshot.target.name.clone(), family: snapshot.target.family,
            battery: status.battery.clone(),
            connected: status.connected,
            quality: status.quality,
        };
        let ident = proto::Identity { model_name: snapshot.target.name.clone(), ..Default::default() };
        let link = (backend == crate::backends::BackendId::Bluetooth)
            .then(|| snapshot.target.hid().ok().cloned())
            .flatten()
            .map(|iface| hid::Link {
                mode: "Bluetooth".into(),
                report_id: 0,
                read_only: true,
                iface,
                all_interfaces: Vec::new(),
            });
        shared.devices.insert(key, DeviceSnapshot { reading, ident, state: None, link,
            battery_at: status.measured_at,
            full_at, basic: Some(snapshot), basic_status: Some(status) });
    }
    shared.rebuild_readings();
}

struct Shared {
    store_update_pending: bool,
    rog_write_active: bool,
    device_keys_dirty: bool,
    refresh_generation: u64,
    completed_requests: BTreeMap<crate::backends::BackendId, u64>,
    source_write_gen: BTreeMap<crate::backends::BackendId, u64>,
    source_channel_gen: BTreeMap<(crate::backends::BackendId, String), u64>,
    source_pending: BTreeMap<crate::backends::BackendId, VecDeque<Cmd>>,
    source_busy: std::collections::BTreeSet<crate::backends::BackendId>,
    source_running: BTreeMap<crate::backends::BackendId, usize>,
    source_discoveries: BTreeMap<crate::backends::BackendId, crate::backends::BackendScan>,
    source_completed: BTreeMap<(crate::backends::BackendId, String), u64>,
    discovery_pending: std::collections::BTreeSet<crate::backends::BackendId>,
    rog_discoveries: Vec<hid::HidInterface>,
    devices: std::collections::BTreeMap<String, DeviceSnapshot>,
    rapoo_devices: rapoo_devices::Devices,
    rapoo_endpoint_snapshots: BTreeMap<(String, u8), crate::backends::Snapshot>,
    key_aliases: std::collections::BTreeMap<String, String>,
    active_key: Option<String>,
    detail_key: Option<String>,
    primary_key: Option<String>,
    /// 最近一次**全量**快照里解析出的状态。`None` = 还没读全过。
    ///
    /// 性能/灯光/按键三页只能用它 —— 3 帧轻量读取里 DPI 编码、颜色、按键表
    /// 都是默认值，拿它刷那三页会把界面刷成假的（DPI 全 100、颜色全黑）。
    full: Option<proto::State>,
    /// 全量快照里的身份信息（型号、SN、固件）。
    ident: Option<proto::Identity>,
    link_mode: String,
    /// 每轮都会更新的那几项。叠在 `full` 上，电量才是活的。
    quick: Option<Quick>,
    /// 最近一次**成功**读到电量的时刻。
    ///
    /// 托盘要能区分「这是现在」和「这是几分钟前的缓存」：刷新线程在设备休眠、
    /// 接收器掉线时会沿用上一次读数，界面上不标注就变成撒谎。
    last_ok_at: Option<std::time::Instant>,
    /// 低电量提醒的闸门：`true` = 还没提醒过。
    low_alert_armed: std::collections::BTreeMap<String, bool>,
    /// 当前设备的族（鼠标/耳机/键盘），提示语按它说话。
    device_family: crate::device::DeviceFamily,
    /// 设备列表与当前详情独立更新。
    pub readings: Vec<crate::device::DeviceReading>,
    /// 写操作代次。每提交一条写命令自增。
    ///
    /// 后台读取是「先读设备、再回填」，中间可能隔着几百毫秒。这段时间里用户
    /// 提交了写命令，读回来的就是**写之前**的状态；不回填还好，回填了界面会
    /// 把刚写进去的值显示成写之前的（用户看到「设置又跳回去了」）。
    write_gen: u64,
    status: Status,
    frames: usize,
    elapsed_ms: u128,
    history_tracker: crate::history::Tracker,
    histories: BTreeMap<String, crate::history::HistoryView>,
    /// Serializes disk work with clear/path changes without holding the UI lock.
    history_gate: Arc<std::sync::Mutex<()>>,
    history_epoch: u64,
    /// 本地设置。界面改了就更新这里再落盘。
    cfg: Settings,
    autostart: (crate::autostart::AutoStartState, crate::autostart::AutoStartPath),
    /// 待执行的写命令队列。
    ///
    /// **以前是单个 `Option<Cmd>`**：后台正在写设备或正在读全量快照时
    /// （全量要 590~1000ms，休眠时更久），用户再点一下就被 `submit` 静默丢掉，
    /// 界面上一点反馈都没有 —— 表现就是「我选了下拉项，但什么都没发生」。
    /// 用户报的「去抖切换没反应」有一个真实成因就是它：点开下拉选值时
    /// 恰好上一轮全量读取还没完。
    ///
    /// 改成队列后连点几下都会依次执行。上限 [`MAX_PENDING`] 防止用户
    /// 疯狂拖滑块时堆几百条命令、松手后设备还在慢慢追。
    pending: VecDeque<(Cmd, Option<crate::commands::RogTarget>)>,
    /// 最近一条命令的结果，GUI 线程取走一次就清空。
    results: VecDeque<CmdResult>,
    /// 下一轮要不要读全量。
    want_full: bool,
}

impl Shared {
    fn expire_readings(&mut self) {
        self.expire_readings_at(std::time::Instant::now());
    }

    fn expire_readings_at(&mut self, now: std::time::Instant) {
        let mut changed = false;
        let rog_lifetime = rog_sample_lifetime(&self.cfg);
        for device in self.devices.values_mut() {
            if let (Some(basic), Some(status)) = (&mut device.basic, &mut device.basic_status) {
                if status.expire(now, basic.target.backend.sample_lifetime()) {
                    device.reading.quality = status.quality;
                    basic.disable_writes();
                    record_reading_diagnostic(crate::diagnostics::readings::Surface::Expired,
                        &device.reading, Some(device), &self.cfg, true, now);
                    changed = true;
                }
            } else if device.basic.is_none()
                && device.battery_at.is_some_and(|at| now.saturating_duration_since(at) >= rog_lifetime)
                && device.reading.quality != crate::device::ReadingQuality::Stale {
                device.reading.quality = crate::device::ReadingQuality::Stale;
                record_reading_diagnostic(crate::diagnostics::readings::Surface::Expired,
                    &device.reading, Some(device), &self.cfg, true, now);
                device.state = None;
                if self.primary_key.as_ref() == Some(&device.reading.key) {
                    self.full = None;
                    self.quick = None;
                }
                changed = true;
            }
        }
        if changed { self.rebuild_readings(); }
    }

    #[cfg(test)]
    fn replace_rog_records(&mut self, records: BTreeMap<String, DeviceSnapshot>) {
        self.devices.retain(|_, device| device.basic.is_some());
        self.devices.extend(records);
        self.rebuild_readings();
    }

    fn publish_additional(&mut self, backend: crate::backends::BackendId, poll: impl Into<crate::backends::Poll>,
        generation: u64, request: u64) -> bool {
        if generation != *self.source_write_gen.get(&backend).unwrap_or(&0) { return false; }
        merge_additional(self, backend, poll.into());
        self.complete_refresh(backend, request);
        true
    }

    fn replace_mock_readings(&mut self, readings: Vec<crate::device::DeviceReading>) {
        self.discovery_pending.clear();
        let now = std::time::Instant::now();
        let mut old_devices = std::mem::take(&mut self.devices);
        self.devices = readings
            .into_iter()
            .map(|reading| {
                let key = reading.key.clone();
                let ident = proto::Identity {
                    model_name: reading.name.clone(),
                    ..Default::default()
                };
                let previous = old_devices.remove(&key);
                let (device_state, basic) = if let Some(previous) = previous {
                    let mut basic = previous.basic;
                    if let Some(snapshot) = &mut basic {
                        snapshot.battery = reading.battery.clone();
                        snapshot.connected = reading.connected;
                    }
                    (previous.state, basic)
                } else if reading.family == crate::device::DeviceFamily::Mouse {
                    if is_mock_rog_mouse(&reading.name) {
                        (Some(mock_rog_state()), None)
                    } else {
                        (None, Some(mock_basic_snapshot(&reading)))
                    }
                } else {
                    (None, None)
                };
                (
                    key,
                    DeviceSnapshot {
                        reading,
                        state: device_state,
                        ident,
                        link: None,
                        full_at: now,
                        basic,
                        basic_status: None,
                        battery_at: Some(now),
                    },
                )
            })
            .collect();
        if self.primary_key.as_ref().map_or(true, |key| !self.devices.contains_key(key)) {
            self.primary_key = None;
        }
        self.rebuild_readings();
    }

    fn rebuild_readings(&mut self) {
        let now = std::time::Instant::now();
        for device in self.devices.values() {
            record_reading_diagnostic(crate::diagnostics::readings::Surface::Published,
                &device.reading, Some(device), &self.cfg, true, now);
        }
        self.readings = self.devices.values().map(|device| device.reading.clone()).collect();
        self.readings.extend(device_workers::pending_readings(self));
        self.readings.extend(rog_refresh::pending_readings(self));
        self.sort_readings();
        self.observe_history();
    }

    fn observe_history(&mut self) {
        if self.store_update_pending {
            return;
        }
        let now = std::time::Instant::now();
        let unix = crate::snapshot::now_unix() as i64;
        for (key, device) in &self.devices {
            let mode = match device.link.as_ref().map(|link| link.mode.as_str()) {
                Some("Wired") => 1, Some("Wireless24G") => 2, Some("Bluetooth") => 3, _ => 0,
            };
            let lifetime = device.basic.as_ref().map(|basic| basic.target.backend.sample_lifetime())
                .unwrap_or_else(|| rog_sample_lifetime(&self.cfg));
            self.history_tracker.observe(&device.reading, device.battery_at, mode, now, unix, lifetime);
            if !device.reading.connected || device.reading.display_percent().is_none() {
                if let Some(view) = self.histories.get_mut(key) { view.estimate = Default::default(); }
            }
        }
    }

    fn reset_history(&mut self) {
        self.history_epoch = self.history_epoch.wrapping_add(1);
        self.histories.clear();
        self.history_tracker.reset();
    }

    fn reload_history(&mut self) {
        self.history_epoch = self.history_epoch.wrapping_add(1);
        self.histories.clear();
    }

    fn complete_refresh(&mut self, backend: crate::backends::BackendId, request: u64) {
        let completed = self.completed_requests.entry(backend).or_default();
        *completed = (*completed).max(request);
        self.refresh_generation = self.completed_requests.values().copied().min().unwrap_or(0);
    }
    fn complete_rog_refresh(&mut self, generation: u64, request: u64) -> bool {
        if outcome_is_stale(generation, self.write_gen) { return false; }
        self.complete_refresh(crate::backends::BackendId::Rog, request);
        true
    }
    fn sort_readings(&mut self) {
        let cfg = &self.cfg;
        self.readings.sort_by(|a, b| {
            (
                cfg.device_order_index(&a.key).unwrap_or(usize::MAX),
                cfg.device_kind(a).order(),
                cfg.device_name(a),
                &a.key,
            )
                .cmp(&(
                    cfg.device_order_index(&b.key).unwrap_or(usize::MAX),
                    cfg.device_kind(b).order(),
                    cfg.device_name(b),
                    &b.key,
                ))
        });
    }

    fn new(cfg: Settings) -> Shared {
        Shared {
            store_update_pending: false,
            rog_write_active: false,
            device_keys_dirty: false,
            refresh_generation: 0,
            completed_requests: std::iter::once(crate::backends::BackendId::Rog).chain(crate::backends::ADDITIONAL_BACKENDS)
                .map(|backend| (backend, 0)).collect(),
            source_write_gen: Default::default(),
            source_channel_gen: Default::default(),
            source_pending: Default::default(),
            source_busy: Default::default(),
            source_running: Default::default(),
            source_discoveries: Default::default(),
            source_completed: Default::default(),
            discovery_pending: std::iter::once(crate::backends::BackendId::Rog)
                .chain(crate::backends::ADDITIONAL_BACKENDS).collect(),
            rog_discoveries: Vec::new(),
            devices: Default::default(),
            rapoo_devices: Default::default(),
            rapoo_endpoint_snapshots: Default::default(),
            key_aliases: Default::default(),
            active_key: None,
            detail_key: None,
            primary_key: None,
            full: None,
            ident: None,
            link_mode: String::new(),
            quick: None,
            last_ok_at: None,
            low_alert_armed: Default::default(),
            device_family: crate::device::DeviceFamily::Unknown,
            readings: Vec::new(),
            write_gen: 0,
            status: Status::Idle,
            frames: 0,
            elapsed_ms: 0,
            history_tracker: Default::default(),
            histories: Default::default(),
            history_gate: Arc::new(std::sync::Mutex::new(())),
            history_epoch: 0,
            cfg,
            autostart: (crate::autostart::AutoStartState::Unknown, crate::autostart::AutoStartPath::Unknown),
            pending: VecDeque::new(),
            results: VecDeque::new(),
            want_full: false,
        }
    }

    fn writes_pending(&self) -> bool {
        self.rog_write_active || !self.pending.is_empty() || !self.source_busy.is_empty()
            || self.source_pending.values().any(|queue| !queue.is_empty())
    }

    // 完整身份晚于首次电量返回时，保留选择和托盘偏好。
    fn promote_device_key(&mut self, old: &str, new: &str) -> bool {
        self.history_epoch = self.history_epoch.wrapping_add(1);
        self.history_tracker.promote(old, new);
        if let Some(view) = self.histories.remove(old) {
            self.histories.entry(new.into()).or_insert(view);
        }
        for key in self.key_aliases.values_mut() {
            if key == old {
                *key = new.to_string();
            }
        }
        self.key_aliases.insert(old.to_string(), new.to_string());
        for selected in [&mut self.active_key, &mut self.primary_key, &mut self.detail_key] {
            if selected.as_deref() == Some(old) {
                *selected = Some(new.to_string());
            }
        }
        let tray_changed = self.cfg.tray_device.as_deref() == Some(old);
        if tray_changed {
            self.cfg.tray_device = Some(new.to_string());
        }
        let mut changed = tray_changed;
        let mut order = self.cfg.device_order.clone();
        for key in &mut order {
            if key == old { *key = new.to_string(); changed = true; }
        }
        self.cfg.set_device_order(order);
        if !self.cfg.device_names.contains_key(new) {
            if let Some(name) = self.cfg.device_names.remove(old) {
                self.cfg.device_names.insert(new.to_string(), name);
                changed = true;
            }
        }
        if let Some(presets) = self.cfg.dpi_presets.remove(old) {
            self.cfg.dpi_presets.entry(new.to_string()).or_insert(presets);
            changed = true;
        }
        if self.cfg.device_tray.contains_key(new) {
            return changed;
        }
        if let Some(pref) = self.cfg.device_tray.get(old).copied() {
            self.cfg.device_tray.insert(new.to_string(), pref);
            return true;
        }
        changed
    }

    /// 界面要显示的状态：全量快照 + 活电量。
    fn ui_state(&self) -> Option<proto::State> {
        let mut st = self.full.clone()?;
        if let Some(q) = self.quick {
            q.apply_to(&mut st);
        }
        Some(st)
    }

    fn autostart_text(&self) -> String {
        crate::trf!(
            "{}（{}）",
            auto_state_short(self.autostart.0),
            crate::i18n::translate(self.autostart.1.short())
        )
    }

    /// 开机自启当前是否打开。开关状态必须来自状态机，不能靠 `autostart_text()`
    /// 里的「已开启」四个字去反推 —— 文案一改（比如加个「已开启（由策略）」）
    /// 开关就跟着错，而这两件事本来没有关系。
    fn autostart_on(&self) -> bool {
        self.autostart.0.is_enabled()
    }

    /// 底部浮层要常驻显示的**异常**文字。
    ///
    /// 只有「真的出错」和「正在写设备」才返回非空：
    /// - `Asleep` 不返回 —— 那是正常状态，电量页上已经有「休眠中」胶囊了，
    ///   再来一条浮层就是重复；而且它会一直挂着，用户以为出了什么问题。
    /// - `Idle` / `Ok` 不返回 —— 没什么可说的。
    ///
    /// `Error(String)` 的 payload 以前**没有任何出口**（`Status::Error` 只被
    /// 匹配成一句「读取出错」，原因丢掉）。用户看到的状态和「鼠标休眠」一模一样，
    /// 不知道是该动鼠标还是该查线缆。
    fn error_text(&self) -> String {
        match &self.status {
            Status::Error(e) => crate::trf!("读取设备失败：{e}", e = e),
            Status::Busy => crate::tr!("正在写设备 ……").to_string(),
            _ => String::new(),
        }
    }
}

fn is_mock_rog_mouse(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.contains("ROG")
        || upper.contains("ASUS")
        || upper.contains("GLADIUS")
        || upper.contains("CHAKRAM")
        || upper.contains("SPATHA")
        || upper.contains("KERIS")
        || upper.contains("HARPE")
}

fn mock_rog_state() -> proto::State {
    let mut device_state = proto::State::default();
    device_state.enc = [
        proto::encode_dpi(400),
        proto::encode_dpi(800),
        proto::encode_dpi(1600),
        proto::encode_dpi(3200),
    ];
    device_state.rgb = [
        [255, 60, 60],
        [60, 220, 80],
        [60, 140, 255],
        [255, 200, 40],
    ];
    device_state.active_stage = 2;
    device_state.poll_code = 3;
    device_state.debounce_code = 2;
    device_state.power = 1;
    device_state.light_rgb = [0, 160, 255];
    device_state
}

fn mock_basic_snapshot(reading: &crate::device::DeviceReading) -> crate::backends::Snapshot {
    let stages = vec![
        crate::device::DpiStage { index: 0, value: 400 },
        crate::device::DpiStage { index: 1, value: 800 },
        crate::device::DpiStage { index: 2, value: 1600 },
        crate::device::DpiStage { index: 3, value: 3200 },
    ];
    let stage_y = std::collections::BTreeMap::from([
        (0, 400), (1, 800), (2, 1600), (3, 3200),
    ]);
    let values = (100..=32000).step_by(50).collect();
    crate::backends::Snapshot {
        target: crate::backends::Target {
            key: reading.key.clone(),
            name: reading.name.clone(),
            family: crate::device::DeviceFamily::Mouse,
            backend: crate::backends::BackendId::Logitech,
            endpoint: crate::backends::Endpoint::XInput(0),
            address: 0,
            verified_identity: true,
        },
        battery: reading.battery.clone(),
        dpi: crate::device::Dpi {
            current: Some(800),
            current_y: Some(800),
            values,
            writable: true,
            write_effect: Some(crate::device::DpiWriteEffect::Current),
            stages,
            active_stage: Some(1),
            profile: Some(1),
            configuration_token: None,
            stage_writable: true,
            stage_editable: true,
            max_stages: 0,
            stage_y,
        },
        polling_rate: crate::device::PollingRate {
            current: Some(1000),
            values: vec![125, 250, 500, 1000, 2000, 4000, 8000],
            writable: true,
            ..Default::default()
        },
        connected: reading.connected,
        battery_at: Some(std::time::Instant::now()),
        battery_unavailable: false,
    }
}

fn apply_mock_cmd(state: &mut Shared, cmd: &Cmd, target_link: Option<&hid::Link>) {
    let link_key = target_link.map(|target| target.device_key().to_string());
    let target_key = match cmd {
        Cmd::BasicDpi { target, .. }
        | Cmd::BasicDpiStage { target, .. }
        | Cmd::BasicDpiStageAdd { target, .. }
        | Cmd::BasicDpiStageRemove { target, .. }
        | Cmd::BasicDpiEdit { target, .. }
        | Cmd::BasicDpiPreset { target, .. }
        | Cmd::BasicPollingRate { target, .. } => Some(target.key.as_str()),
        _ => link_key.as_deref().or(state.active_key.as_deref()),
    };

    match cmd {
        Cmd::BasicDpiStage { stage, .. } => {
            if let Some(key) = target_key {
                if let Some(device) = state.devices.get_mut(key) {
                    if let Some(basic) = &mut device.basic {
                        basic.dpi.active_stage = Some(stage.index);
                        basic.dpi.current = Some(stage.value);
                        let vertical = basic.dpi.stage_y.get(&stage.index).copied().unwrap_or(stage.value);
                        basic.dpi.current_y = Some(vertical);
                    }
                }
            }
        }
        Cmd::BasicDpiStageAdd { x, y, .. } => {
            if let Some(key) = target_key {
                if let Some(device) = state.devices.get_mut(key) {
                    if let Some(basic) = &mut device.basic {
                        let idx = basic.dpi.stages.len() as u8;
                        basic.dpi.stages.push(crate::device::DpiStage { index: idx, value: *x });
                        basic.dpi.stage_y.insert(idx, *y);
                    }
                }
            }
        }
        Cmd::BasicDpiStageRemove { stage, .. } => {
            if let Some(key) = target_key {
                if let Some(device) = state.devices.get_mut(key) {
                    if let Some(basic) = &mut device.basic {
                        basic.dpi.stages.retain(|saved| saved.index != stage.index);
                        basic.dpi.stage_y.remove(&stage.index);
                    }
                }
            }
        }
        Cmd::BasicDpiEdit { stage, x, y, .. } => {
            if let Some(key) = target_key {
                if let Some(device) = state.devices.get_mut(key) {
                    if let Some(basic) = &mut device.basic {
                        if let Some(saved) = basic.dpi.stages.iter_mut().find(|saved| saved.index == stage.index) {
                            saved.value = *x;
                        }
                        basic.dpi.stage_y.insert(stage.index, *y);
                        if basic.dpi.active_stage == Some(stage.index) {
                            basic.dpi.current = Some(*x);
                            basic.dpi.current_y = Some(*y);
                        }
                    }
                }
            }
        }
        Cmd::BasicDpiPreset { stage, value, .. } => {
            if let Some(key) = target_key {
                if let Some(device) = state.devices.get_mut(key) {
                    if let Some(basic) = &mut device.basic {
                        if let Some(saved) = basic.dpi.stages.iter_mut().find(|saved| saved.index == stage.index) {
                            saved.value = *value;
                        }
                        basic.dpi.stage_y.insert(stage.index, *value);
                        if basic.dpi.active_stage == Some(stage.index) {
                            basic.dpi.current = Some(*value);
                            basic.dpi.current_y = Some(*value);
                        }
                    }
                }
            }
        }
        Cmd::BasicDpi { dpi, .. } => {
            if let Some(key) = target_key {
                if let Some(device) = state.devices.get_mut(key) {
                    if let Some(basic) = &mut device.basic {
                        basic.dpi.current = Some(*dpi);
                        basic.dpi.current_y = Some(*dpi);
                    }
                }
            }
        }
        Cmd::BasicPollingRate { hz, .. } => {
            if let Some(key) = target_key {
                if let Some(device) = state.devices.get_mut(key) {
                    if let Some(basic) = &mut device.basic {
                        basic.polling_rate.current = Some(*hz);
                    }
                }
            }
        }
        Cmd::Stage(stage) => {
            for device in state.devices.values_mut() {
                if let Some(device_state) = &mut device.state {
                    device_state.active_stage = *stage;
                }
            }
        }
        Cmd::Dpi { stage, dpi, rgb } => {
            for device in state.devices.values_mut() {
                if let Some(device_state) = &mut device.state {
                    if *stage < proto::DPI_STAGES {
                        device_state.enc[*stage] = proto::encode_dpi(*dpi);
                        if let Some(color) = rgb {
                            device_state.rgb[*stage] = *color;
                        }
                    }
                }
            }
        }
        Cmd::Poll(hz) => {
            let code = proto::poll_code_of(*hz).unwrap_or(3);
            for device in state.devices.values_mut() {
                if let Some(device_state) = &mut device.state {
                    device_state.poll_code = code;
                }
            }
        }
        Cmd::Debounce(code) => {
            for device in state.devices.values_mut() {
                if let Some(device_state) = &mut device.state {
                    device_state.debounce_code = *code;
                }
            }
        }
        Cmd::Snap(enabled) => {
            for device in state.devices.values_mut() {
                if let Some(device_state) = &mut device.state {
                    device_state.angle_snap = *enabled;
                }
            }
        }
        Cmd::Tuning(enabled) => {
            for device in state.devices.values_mut() {
                if let Some(device_state) = &mut device.state {
                    device_state.angle_tuning = *enabled;
                }
            }
        }
        Cmd::Light { mode, rgb, .. } => {
            for device in state.devices.values_mut() {
                if let Some(device_state) = &mut device.state {
                    device_state.power = *mode;
                    if let Some(color) = rgb {
                        device_state.light_rgb = *color;
                    }
                }
            }
        }
        Cmd::AutoStart(enabled) => {
            state.autostart = (
                if *enabled {
                    crate::autostart::AutoStartState::Enabled
                } else {
                    crate::autostart::AutoStartState::Disabled
                },
                state.autostart.1.clone(),
            );
        }
        _ => {}
    }

    state.results.push_back(CmdResult {
        label: cmd.label(),
        ok: Some("已生效".into()),
        err: None,
        hint: "",
    });
}

fn popup_dpi_states(shared: &Shared) -> BTreeMap<String, crate::ui::tray_popup::DpiState> {
    shared
        .devices
        .iter()
        .filter_map(|(key, device)| {
            if let Some(basic) = &device.basic {
                let presets = shared.cfg.dpi_presets.get(key);
                let custom = presets.is_some_and(|p| p.enabled);
                let (stages, value_control) = basic.dpi.shortcut_stages(presets);
                if stages.is_empty() && basic.dpi.current.is_none() { return None; }
                let active = if !value_control { basic.dpi.active_stage } else {
                    stages.iter().find(|stage| Some(stage.value) == basic.dpi.current
                        && basic.dpi.current_y.is_none_or(|y| y == stage.value)).map(|stage| stage.index)
                };
                let disabled = if value_control { stages.iter().filter(|stage| !basic.dpi.values.contains(&stage.value))
                    .map(|stage|stage.index).collect() } else { Vec::new() };
                return Some((key.clone(), crate::ui::tray_popup::DpiState { stages, active,
                    current: basic.dpi.current, current_y: basic.dpi.current_y,
                    stage_y:if value_control {BTreeMap::new()} else {basic.dpi.stage_y.clone()},context: basic.dpi.stage_context(),
                    custom, disabled, value_control, writable: basic.connected &&
                        if !value_control { basic.dpi.stage_writable } else { basic.dpi.writable
                            && (!custom || basic.dpi.write_effect.is_some()) } }));
            }
            let state = device.state.as_ref()?;
            Some((
                key.clone(),
                crate::ui::tray_popup::DpiState {
                    stages: (0..4).map(|index| crate::device::DpiStage {
                        index: index as u8 + 1, value: state.dpi(index) as u16 }).collect(),
                    current: Some(state.dpi(usize::from(state.active_stage.clamp(1, 4) - 1)) as u16),
                    current_y: None,
                    stage_y: BTreeMap::new(),
                    active: Some(state.active_stage.clamp(1, 4)),
                    context: None,
                    writable: true,
                    value_control: false,
                    custom: false,
                    disabled: Vec::new(),
                },
            ))
        })
        .collect()
}

fn taskbar_strip_content(
    readings: &[crate::device::DeviceReading],
    cfg: &Settings,
) -> Option<crate::taskbar_widget::StripContent> {
    let items = readings
        .iter()
        .filter(|reading| {
            reading.display_percent().is_some() || reading.coarse_level().is_some()
        })
        .map(|reading| crate::taskbar_widget::StripItem {
            level: reading.display_percent(),
            charging: reading.is_charging(),
            kind: cfg.device_kind(reading),
        })
        .collect::<Vec<_>>();
    Some(if items.is_empty() {
        crate::taskbar_widget::StripContent {
            items,
            empty_label: Some(crate::tr!("暂无设备").to_string()),
        }
    } else {
        crate::taskbar_widget::StripContent {
            items,
            empty_label: None,
        }
    })
}

fn dpi_preset_command(cfg: &Settings, basic: &crate::backends::Snapshot, value: u16) -> Option<Cmd> {
    let presets = cfg.dpi_presets.get(&basic.target.key)?;
    if !presets.enabled || !presets.values.contains(&value) || !basic.connected
        || !basic.dpi.writable || basic.dpi.write_effect.is_none() || !basic.dpi.values.contains(&value) {
        return None;
    }
    if basic.target.backend == crate::backends::BackendId::CompX {
        // Numerical writes on this family rewrite the active onboard row. Bind
        // that row now, so a queued command cannot edit a newly selected stage.
        let stage = *basic.dpi.stages.iter().find(|s| Some(s.index) == basic.dpi.active_stage)?;
        if !basic.dpi.stage_editable { return None; }
        return Some(Cmd::BasicDpiPreset { target:basic.target.clone(),stage,context:basic.dpi.stage_context()?,
            expected_y:*basic.dpi.stage_y.get(&stage.index)?,value });
    }
    Some(Cmd::BasicDpi {target:basic.target.clone(),dpi:value})
}

/// 「有活了」的信号 + 关闭开关。
///
/// 用 `Condvar` + 计数而不是「睡固定时长」：用户点「立即刷新」时能立刻醒，
/// 改刷新间隔也能立刻生效（`interval_ms` 是原子量，改完 `notify_all` 一次）。
struct Signal {
    poke: Mutex<u64>,
    cv: Condvar,
    interval_ms: AtomicU64,
    wake_probe: AtomicBool,
    shutdown: AtomicBool,
    /// 最近一次用户活动触发的补读时刻（节流用）。
    last_activity: Mutex<Option<std::time::Instant>>,
}

impl Signal {
    fn new(interval_sec: u64) -> Signal {
        Signal {
            poke: Mutex::new(0),
            last_activity: Mutex::new(None),
            cv: Condvar::new(),
            interval_ms: AtomicU64::new(interval_sec.saturating_mul(1000)),
            wake_probe: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
        }
    }

    /// 「请求退出」的唯一出口：置标志 + 唤醒后台线程。
    /// 结束消息循环（`nwg::stop_thread_dispatch()`）留给调用点 —— 那是 UI 线程的事。
    fn request_quit(&self) {
        let _guard = lock_poke(&self.poke);
        self.shutdown.store(true, Ordering::Relaxed);
        self.cv.notify_all();
    }

    /// 用户刚动过界面 —— 提前补读一次，别等到下一个心跳。
    ///
    /// 节流 10 秒：连点几个设置不该把设备刷爆。返回 `true` 表示这次真的触发了。
    fn note_activity(&self) -> bool {
        let now = std::time::Instant::now();
        let due = {
            let mut last = self
                .last_activity
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let due = activity_due(*last, now);
            if due {
                *last = Some(now);
            }
            due
        };
        if due {
            self.request_refresh();
        }
        due
    }

    fn request_refresh(&self) {
        let mut n = lock_poke(&self.poke);
        *n += 1;
        self.cv.notify_all();
    }

    fn current_request(&self) -> u64 { *lock_poke(&self.poke) }

    fn set_wake_probe(&self, active: bool) {
        self.wake_probe.store(active, Ordering::Relaxed);
    }

    /// 等到「有新工作」或「过了一个间隔」。返回 `false` = 该退出了。
    fn wait_for_work(&self, seen: &mut u64) -> bool {
        self.wait_for_work_with_limit(seen, None)
    }

    fn work_interval_ms(&self, limit: Option<u64>) -> u64 {
        let configured = self.interval_ms.load(Ordering::Relaxed).max(1000);
        let interval = if self.wake_probe.load(Ordering::Relaxed) {
            configured.min(WAKE_PROBE_INTERVAL_MS)
        } else {
            configured
        };
        limit.map_or(interval, |limit| interval.min(limit.max(1)))
    }

    fn wait_for_work_with_limit(&self, seen: &mut u64, limit: Option<u64>) -> bool {
        let interval = self.work_interval_ms(limit);
        let (request, _) = self.cv.wait_timeout_while(
            lock_poke(&self.poke), Duration::from_millis(interval),
            |request| *request == *seen && !self.shutdown.load(Ordering::Relaxed),
        ).unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.shutdown.load(Ordering::Relaxed) {
            return false;
        }
        *seen = *request;
        true
    }
}

/// 休眠时缩短轮询间隔，让鼠标移动唤醒后能自动恢复托盘电量。
const WAKE_PROBE_INTERVAL_MS: u64 = 3_000;

fn lock_poke(m: &Mutex<u64>) -> std::sync::MutexGuard<'_, u64> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

/// 取共享状态。**锁中毒也继续用** —— 宁可数据旧一点，也不要整个 UI 停摆。
fn lock_shared(s: &Arc<Mutex<Shared>>) -> std::sync::MutexGuard<'_, Shared> {
    match s.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn complete_rog_command(
    shared: &Arc<Mutex<Shared>>, cmd: &Cmd, target: Option<&hid::Link>, result: CmdResult,
    read_autostart: impl FnOnce() -> (crate::autostart::AutoStartState, crate::autostart::AutoStartPath),
) {
    let autostart = matches!(cmd, Cmd::AutoStart(_)).then(read_autostart);
    let mut state = lock_shared(shared);
    state.rog_write_active = false;
    if let Some(target) = target {
        for device in state.devices.values_mut() {
            if device.link.as_ref().is_some_and(|link| link.device_key() == target.device_key()) {
                device.state = None;
            }
        }
    }
    if let Some(autostart) = autostart {
        state.autostart = autostart;
    }
    state.results.push_back(result);
    state.want_full = true;
}

// ==================== 界面动作的执行 ====================

/// 等一个后台线程收尾，最多 `budget`。
///
/// 限制退出时等待HID查询和历史文件锁的时间；结束轮询后再保存待写采样。
fn join_bounded(h: std::thread::JoinHandle<()>, budget: Duration) {
    let start = std::time::Instant::now();
    while !h.is_finished() && start.elapsed() < budget {
        std::thread::sleep(Duration::from_millis(20));
    }
    if h.is_finished() {
        let _ = h.join();
    }
}

/// 托盘图标回调消息号。`WM_APP + 100`，避开 nwg 自己用的那些。
const TRAY_CALLBACK_MSG: u32 = 0x8000 + 100;
/// Taskbar-adjacent strip callback. It routes into the same quick-view popup.
const TASKBAR_CALLBACK_MSG: u32 = 0x8000 + 101;

#[derive(Clone, Copy, Default)]
struct TrayClicks {
    pending: Option<(u32, bool)>,
    pressed_was_open: Option<bool>,
    suppress_release: bool,
}

impl TrayClicks {
    fn press(&mut self, was_open: bool) {
        self.pressed_was_open = Some(was_open);
        self.suppress_release = false;
    }

    fn release(&mut self, timestamp: u32, was_open: bool) -> bool {
        if self.suppress_release || self.pending.is_some() {
            return false;
        }
        self.pending = Some((timestamp, self.pressed_was_open.take().unwrap_or(was_open)));
        true
    }

    fn double_click(&mut self, timestamp: u32, interval: u32) -> bool {
        if self.suppress_release {
            return false;
        }
        let valid = self.pending.take().is_some_and(|(first, _)| {
            timestamp.wrapping_sub(first) <= interval
        });
        self.pressed_was_open = None;
        self.suppress_release = valid;
        valid
    }

    fn finish_single(&mut self) -> Option<bool> {
        self.pending.take().map(|(_, was_open)| was_open)
    }

    fn cancel(&mut self) {
        *self = Self::default();
    }
}

/// 自己管的托盘图标。
///
/// 为什么不用 nwg 的 `TrayNotification`：它不暴露 `guidItem`，而托盘图标的
/// 显示/折叠状态 Windows 是按 **exe 路径** 记的 —— MSIX 每次更新换目录，
/// 系统就当成新程序，又折叠回溢出区（用户报的「更新之后托盘又被折叠」）。
/// 注册固定 GUID 后系统按 GUID 记，跨版本保持用户的选择。
/// 托盘诊断日志：只在异常路径写一行，正常时一个字节都不写。
///
/// 用户报「没有托盘」时我们这边复现不了 —— Win11 的 shell 会自己清理残留项，
/// 只有 Win10 才会因为幽灵 GUID 一直拒绝 `NIM_ADD`。把每次添加的落点记下来，
/// 让用户发一个文件就能定位，不用靠猜。
fn tray_log(line: &str) {
    let Some(dir) = crate::paths::settings_file().parent().map(|p| p.to_path_buf()) else {
        return;
    };
    let path = dir.join("tray.log");
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<&str> = old.lines().collect();
    // 只留最近 20 行，避免一个长期开着的程序把日志写成大文件。
    if lines.len() >= 20 {
        lines.drain(..lines.len() - 19);
    }
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    text.push_str(&format!("{secs} {line}\n"));
    let _ = crate::paths::write_atomic(&path, text.as_bytes());
}

struct TrayCtl {
    hwnd: win32::Handle,
    enabled: Cell<bool>,
    entries: std::cell::RefCell<Vec<TrayEntry>>,
}

struct TrayEntry {
    key: String,
    slot: u8,
    id: win32::TrayId,
    icon: win32::Handle,
    signature: String,
    tip: String,
}

impl TrayCtl {
    fn new(hwnd: win32::Handle) -> Self {
        Self {
            hwnd,
            enabled: Cell::new(false),
            entries: Default::default(),
        }
    }

    fn set_visibility(&self, enabled: bool) {
        self.enabled.set(enabled);
        if !enabled {
            for entry in self.entries.borrow_mut().drain(..) {
                win32::tray_remove(self.hwnd, entry.id);
                unsafe {
                    win32::DestroyIcon(entry.icon);
                }
            }
        }
    }

    fn sync(&self, readings: &[crate::device::DeviceReading], cfg: &Settings) {
        self.set_visibility(cfg.tray_enabled);
        if !self.enabled.get() {
            return;
        }
        let wanted = vec![cfg.tray_device.clone().unwrap_or_default()];
        let mut entries = self.entries.borrow_mut();
        for key in wanted {
            let reading = readings.iter().find(|r| r.key == key);
            let pref = cfg.tray_for(&key);
            let content = crate::tray_prefs::TrayContent::Number;
            let tip = reading
                .map(|r| {
                    format!(
                        "{} · {}{}",
                        cfg.device_name(r),
                        r.battery.as_ref().and_then(|battery| battery.display_percent()).map(|level| level.label(true))
                            .or_else(|| r.coarse_level().map(|level| crate::i18n::battery_level(level).to_string()))
                            .unwrap_or_else(|| crate::tr!("电量未知").to_string()),
                        if r.is_discovering() {
                            crate::ui::feedback::text("（正在确认连接与电量）", " (checking connection and battery)")
                        } else if !r.connected {
                            crate::tr!("（上次记录，未连接）")
                        } else if r.quality == crate::device::ReadingQuality::Cached {
                            crate::tr!("（上次记录，等待更新）")
                        } else if r.quality == crate::device::ReadingQuality::Stale {
                            crate::tr!("（上次记录已过期）")
                        } else if r.is_charging() {
                            crate::tr!("（充电中）")
                        } else {
                            ""
                        }
                    )
                })
                .unwrap_or_else(|| {
                    if cfg.tray_device.is_some() && !readings.is_empty() {
                        format!("{} · {}", cfg.tray_name(), crate::tr!("未连接"))
                    } else {
                        crate::tr!("X-Battery · 打开设备页面").to_string()
                    }
                });
            let signature = format!(
                "{:?}:{pref:?}:{content:?}:{}:{}:{tip}",
                reading,
                crate::tray_icon::icon_size(),
                crate::autostart::system_uses_light_theme()
            );
            if entries
                .iter()
                .any(|entry| entry.key == key && entry.signature == signature)
            {
                continue;
            }
            let icon = if let Some(r) = reading {
                crate::tray_icon::render_device(
                    r.display_percent(),
                    r.is_charging(),
                    cfg.device_kind(r),
                    content,
                )
            } else if cfg.tray_device.is_some() && !readings.is_empty() {
                crate::tray_icon::render_device(None, false, pref.kind, content)
            } else {
                let h = win32::load_app_icon(32);
                if h.is_null() {
                    crate::tray_icon::render_unknown()
                } else {
                    Some(h)
                }
            };
            let Some(icon) = icon else { continue };
            if let Some(entry) = entries.first_mut() {
                if win32::tray_update(self.hwnd, entry.id, Some(icon), Some(&tip)) {
                    unsafe {
                        win32::DestroyIcon(entry.icon);
                    }
                    entry.key = key;
                    entry.icon = icon;
                    entry.signature = signature;
                    entry.tip = tip;
                } else {
                    unsafe {
                        win32::DestroyIcon(icon);
                    }
                }
            } else {
                let slot = 0;
                if let Some(id) = win32::tray_add(self.hwnd, TRAY_CALLBACK_MSG, icon, &tip, slot) {
                    entries.push(TrayEntry {
                        key,
                        slot,
                        id,
                        icon,
                        signature,
                        tip,
                    });
                } else {
                    unsafe {
                        win32::DestroyIcon(icon);
                    }
                    tray_log("add failed");
                }
            }
        }
    }

    fn device_for_callback(&self, wp: usize, lp: isize) -> Option<String> {
        let v4 = ((lp as u32) >> 16) as u32;
        let uid = if v4 != 0 { v4 } else { wp as u32 };
        self.entries
            .borrow()
            .iter()
            .find(|e| e.id.uid == uid)
            .map(|e| e.key.clone())
    }

    fn re_add(&self) {
        for entry in self.entries.borrow_mut().iter_mut() {
            if let Some(id) = win32::tray_add(
                self.hwnd,
                TRAY_CALLBACK_MSG,
                entry.icon,
                &entry.tip,
                entry.slot,
            ) {
                entry.id = id;
            }
        }
    }
}

impl std::ops::Drop for TrayCtl {
    fn drop(&mut self) {
        self.set_visibility(false);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StorePreparationAction {
    Install,
    Restart,
    ExitToStore,
}

struct StorePreparation {
    action: StorePreparationAction,
    started: std::time::Instant,
    worker: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    result: Option<Result<(), String>>,
    canceled: bool,
}

impl StorePreparation {
    fn can_finish(&self) -> bool {
        let complete = self.canceled || self.result.is_some();
        complete && (self.worker.is_none() || self.result.is_some())
    }
}

struct StoreUpdates {
    controller: crate::store::Controller,
    preparation: RefCell<Option<StorePreparation>>,
    writes_blocked: Cell<bool>,
    ticking: Cell<bool>,
}

fn save_store_checkpoint(shared: &Arc<Mutex<Shared>>) -> Result<(), String> {
    let gate = lock_shared(shared).history_gate.clone();
    let _guard = gate.lock().unwrap_or_else(|error| error.into_inner());
    let (samples, aliases) = {
        let mut state = lock_shared(shared);
        settings::save(&state.cfg)?;
        let (samples, mut aliases) = state.history_tracker.take_pending();
        aliases.extend(state.key_aliases.iter().map(|(old, new)| (old.clone(), new.clone())));
        (samples, aliases)
    };
    if !samples.is_empty() || !aliases.is_empty() {
        if let Err(error) = crate::history::update_device_history(&samples, &aliases, None) {
            lock_shared(shared).history_tracker.retry(samples, aliases);
            return Err(error);
        }
    }
    Ok(())
}

impl StoreUpdates {
    fn new(hwnd: win32::Handle) -> Self {
        Self {
            controller: crate::store::Controller::new(hwnd),
            preparation: RefCell::new(None),
            writes_blocked: Cell::new(false),
            ticking: Cell::new(false),
        }
    }

    fn view(&self) -> crate::store::View {
        let mut view = self.controller.view();
        if self.preparation.borrow().is_some() {
            view.phase = crate::store::Phase::Preparing;
            view.failure = None;
        }
        view
    }

    fn busy(&self) -> bool {
        self.controller.installing() || self.preparation.borrow().is_some()
    }

    fn close_action(&self, mode: &str, tray_enabled: bool, taskbar_available: bool) -> WindowCloseAction {
        if self.busy() { WindowCloseAction::Quit }
        else { window_close_action(mode, tray_enabled, taskbar_available) }
    }

    fn begin(&self, action: StorePreparationAction, shared: &Arc<Mutex<Shared>>) -> bool {
        if self.busy() {
            return false;
        }
        if action == StorePreparationAction::Install {
            if !self.controller.begin_install() {
                return false;
            }
        } else if action == StorePreparationAction::Restart {
            if self.controller.view().phase != crate::store::Phase::RestartFailed { return false; }
        } else {
            self.controller.cancel();
        }
        let mut state = lock_shared(shared);
        state.observe_history();
        state.store_update_pending = true;
        drop(state);
        self.writes_blocked.set(true);
        self.preparation.replace(Some(StorePreparation {
            action,
            started: std::time::Instant::now(),
            worker: None,
            result: None,
            canceled: false,
        }));
        true
    }

    fn cancel(&self) {
        let mut preparation = self.preparation.borrow_mut();
        if let Some(job) = preparation.as_mut() {
            job.canceled = true;
            return;
        }
        drop(preparation);
        self.controller.cancel();
    }

    fn tick(&self, shared: &Arc<Mutex<Shared>>, signal: &Signal) -> (bool, Option<String>) {
        if self.ticking.replace(true) {
            return (false, None);
        }
        let before = self.view();
        self.controller.tick();
        if self.controller.view().phase == crate::store::Phase::AwaitingPreparation {
            self.begin(StorePreparationAction::Install, shared);
        }
        let mut error = None;
        let preparation = self.preparation.borrow_mut().take();
        if let Some(mut job) = preparation {
            if job.worker.is_none() && job.result.is_none() {
                if job.canceled {
                    job.result = Some(Ok(()));
                } else if job.action == StorePreparationAction::Install && !self.controller.restart_ready() {
                    job.started = std::time::Instant::now();
                } else if lock_shared(shared).writes_pending() {
                    if job.started.elapsed() >= Duration::from_secs(30) {
                        job.result = Some(Err(crate::ui::feedback::text(
                            "设备操作仍未完成，请稍后再安装。", "Device operations have not finished. Try installing later.",
                        ).into()));
                    }
                } else {
                    let (sender, receiver) = std::sync::mpsc::channel();
                    let shared_save = Arc::clone(shared);
                    std::thread::spawn(move || {
                        let _ = sender.send(save_store_checkpoint(&shared_save));
                    });
                    job.worker = Some(receiver);
                }
            }
            if job.result.is_none() {
                if let Some(worker) = &job.worker {
                    match worker.try_recv() {
                        Ok(result) => job.result = Some(result),
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => job.result = Some(Err(
                            crate::ui::feedback::text("保存任务未完成，请重试。", "The save task did not finish. Please retry.").into(),
                        )),
                        Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    }
                }
            }
            if job.can_finish() {
                let result = job.result.take().unwrap_or(Ok(()));
                if job.canceled || result.is_err() {
                    if job.action == StorePreparationAction::Install {
                        if job.canceled { self.controller.preparation_canceled(); }
                        else { self.controller.preparation_failed(); }
                    }
                    error = result.err();
                    if let Some(error) = &error {
                        crate::diagnostics::event("store_update_prepare_failed", error);
                    }
                } else if job.action == StorePreparationAction::Install {
                    self.controller.install();
                } else if job.action == StorePreparationAction::Restart {
                    self.controller.restart();
                } else if let Err(open_error) = crate::dynlight::shell_open(crate::store::UPDATES_URL) {
                    error = Some(open_error);
                } else {
                    signal.request_quit();
                    nwg::stop_thread_dispatch();
                }
            } else {
                self.preparation.replace(Some(job));
            }
        }
        if self.view().phase == crate::store::Phase::Restarting {
            signal.request_quit();
            nwg::stop_thread_dispatch();
        }
        if !self.busy() && self.writes_blocked.replace(false) {
            let mut state = lock_shared(shared);
            state.store_update_pending = false;
            state.observe_history();
        }
        let changed = before != self.view();
        self.ticking.set(false);
        (changed, error)
    }
}

/// 处理一个 `Act` 时要用到的一堆句柄。
///
/// 打成一个结构而不是给 `handle_act` 传七个参数：这条链上每加一个能力
/// （比如新加一个「打开日志」按钮要 `ShellExecute`），改签名会波及每一处调用。
struct UiEnv<'a> {
    audio: &'a crate::audio::Controls,
    canvas: &'a Canvas,
    shared: &'a Arc<Mutex<Shared>>,
    signal: &'a Signal,
    window: &'a nwg::Window,
    tray: &'a TrayCtl,
    taskbar: &'a crate::taskbar_widget::TaskbarWidget,
    name_dialog: &'a crate::ui::device_name::Dialog,
    feedback_dialog: &'a crate::ui::feedback::Dialog,
    store: &'a StoreUpdates,
    save_settings: &'a dyn Fn(&Settings) -> Result<(), String>,
    notifications_supported: fn() -> bool,
}

impl<'a> UiEnv<'a> {
    fn basic(&self) -> Option<crate::backends::Snapshot> {
        let key = self.canvas.with_ui(|ui| ui.selected_device.clone())?;
        lock_shared(self.shared).devices.get(&key)?.basic.clone()
    }
    /// 在界面底部浮层显示一条提示，并立刻重画。
    fn say(&self, text: impl Into<String>) {
        let t = text.into();
        self.canvas.with_ui_mut(|u| u.set_status(t));
        self.canvas.set_animating(true);
        self.canvas.repaint_now();
    }

    fn state(&self) -> Option<proto::State> {
        let s = lock_shared(self.shared);
        let key = self.canvas.with_ui(|u| u.selected_device.clone());
        match key.as_ref().and_then(|k| s.devices.get(k)) {
            Some(d) => d.state.clone(),
            None if key.is_none() => s.ui_state(),
            None => None,
        }
    }

    fn cfg(&self) -> Settings {
        lock_shared(self.shared).cfg.clone()
    }

    fn set_cfg(&self, f: impl FnOnce(&mut Settings)) -> Result<(), String> {
        let mut shared = lock_shared(self.shared);
        if shared.store_update_pending {
            drop(shared);
            let error = crate::ui::feedback::text("更新准备期间暂时不能修改设置。", "Settings cannot be changed while preparing an update.").to_string();
            self.say(error.clone());
            return Err(error);
        }
        let mut snapshot = shared.cfg.clone();
        f(&mut snapshot);
        if let Err(error) = (self.save_settings)(&snapshot) {
            drop(shared);
            let error = crate::trf!("✘ 保存设置失败：{e}", e = error);
            self.say(error.clone());
            return Err(error);
        }
        shared.cfg = snapshot;
        shared.sort_readings();
        let language = shared.cfg.language;
        drop(shared);
        crate::i18n::apply(language);
        Ok(())
    }

    fn submit(&self, cmd: Cmd) -> bool {
        lock_shared(self.shared).active_key = self.canvas.with_ui(|u| u.selected_device.clone());
        let ok = submit(self.shared, self.signal, cmd);
        if !ok {
            // 以前这里是什么都不说就返回 false —— 用户点了一下，
            // 界面上完全没有变化，只会以为功能坏了。现在至少给一句话。
            self.say(crate::tr!(
                "✘ 上一项设置仍在写入设备，请稍等片刻再试"
            ));
        }
        ok
    }
}

/// 把一个 `Act` 执行掉。
///
/// 界面本地的动作（切页、开弹层）在这里改 `Ui` 并重画；
/// 要写设备的交给后台线程。**这里没有一行写设备的代码** —— 全部转 [`Cmd`]。
fn handle_act(a: Act, env: &UiEnv) {
    // 任何新动作都先把上一条提示清掉：否则写完设备的「✔ 已生效」会一直挂着，
    // 用户接着点别的东西时还以为是这次的结果。
    if !matches!(a, Act::None) {
        env.canvas.with_ui_mut(|u| u.clear_status());
    }

    if env.store.busy() && !matches!(a, Act::None | Act::Nav(_) | Act::SettingsSection(_)
        | Act::OpenStore | Act::CancelUpdate | Act::DismissUpdate) {
        env.say(crate::ui::feedback::text("正在准备或安装更新，请稍候。", "An update is being prepared or installed. Please wait."));
        return;
    }

    // 读不到设备时，凡是要读写设备的动作都只回一句话：
    // 控件照常响应点击（有按压反馈），但不进队列、不改任何状态。
    // 以前是整页锁死不登记命中，点了毫无反应，用户只会以为界面坏了。
    if a.needs_device() && env.canvas.with_ui(|u| u.locked) {
        env.say(crate::tr!("✘ 读不到设备：插好接收器，或动一下设备再试"));
        return;
    }

    match a {
        Act::RenameDevice => {
            let key = env.canvas.with_ui(|u| u.selected_device.clone());
            let s = lock_shared(env.shared);
            if let Some(device) = key
                .as_ref()
                .and_then(|key| s.readings.iter().find(|r| &r.key == key))
            {
                let name = s.cfg.device_name(device).to_string();
                let key = device.key.clone();
                let dark = crate::ui::theme::resolve(&s.cfg.theme) == crate::ui::theme::Mode::Dark;
                drop(s);
                env.name_dialog.show(env.window, &key, &name, dark);
            }
        }
        Act::SelectDevice(index) => {
            let key = env.canvas.with_ui(|u| u.device_keys.get(index).cloned());
            if let Some(key) = key {
                env.canvas.with_ui_mut(|u| u.select_device(&key));
                let mut s = lock_shared(env.shared);
                s.active_key = Some(key);
                s.want_full = true;
            }
            env.signal.request_refresh();
            env.canvas.repaint_now();
        }
        Act::ReorderDevice(from, to, revision) => {
            let displayed = env.canvas.with_ui(|ui| {
                (ui.device_list_revision == revision).then(|| ui.device_keys.clone())
            });
            if let Some(displayed) = displayed {
                persist_device_reorder(&mut lock_shared(env.shared), &displayed, from, to);
            }
            env.canvas.with_ui_mut(|ui| {
                ui.device_drag = None;
                ui.hover = None;
                ui.card_hover_anims.clear();
            });
            env.canvas.repaint_now();
        }
        Act::ToggleDeviceTray => {
            if let Some(key) = env.canvas.with_ui(|u| u.selected_device.clone()) {
                let reading = lock_shared(env.shared)
                    .readings
                    .iter()
                    .find(|r| r.key == key)
                    .cloned();
                if let Some(reading) = reading {
                    if env.set_cfg(|c| {
                        c.tray_device = Some(key);
                        c.tray_device_name = reading.name;
                        c.tray_enabled = true;
                    }).is_err() { return; }
                    env.canvas.repaint_now();
                }
            }
        }
        Act::None => {}

        Act::Nav(p) => {
            env.canvas.with_ui_mut(|u| {
                u.page = p;
                u.initial_route_done = true;
                u.dpi_delete_mode = false;
                u.set_open_drop(None);
            });
            // 切到需要全量数据的页 → 立刻补一次全量读取。
            // 否则第一次进性能页看到的是「还没读到」或者几分钟前的旧值。
            if matches!(p, Page::Perf | Page::Light | Page::Keys) {
                {
                    let mut s = lock_shared(env.shared);
                    s.want_full = true;
                }
                env.signal.request_refresh();
            }
            env.canvas.repaint_now();
        }

        Act::SettingsSection(section) => {
            env.canvas.with_ui_mut(|ui| ui.select_settings_section(section));
            env.canvas.repaint_now();
        }

        Act::SelectCloseChoice(c) => {
            env.canvas.with_ui_mut(|u| u.close_choice = c);
            env.canvas.repaint_now();
        }

        Act::ToggleCloseRemember => {
            env.canvas.with_ui_mut(|u| u.close_remember = !u.close_remember);
            env.canvas.repaint_now();
        }

        Act::CancelCloseDialog => {
            env.canvas.with_ui_mut(|u| u.show_close_dialog = false);
            env.canvas.repaint_now();
        }

        Act::ConfirmCloseDialog => {
            let (choice, remember) = env.canvas.with_ui(|u| (u.close_choice, u.close_remember));
            if remember {
                let mode = if choice == crate::ui::widget::CloseChoice::Minimize {
                    "minimize"
                } else {
                    "quit"
                };
                if env.set_cfg(|c| c.close_mode = mode.to_string()).is_err() { return; }
            }
            env.canvas.with_ui_mut(|u| u.show_close_dialog = false);
            env.canvas.repaint_now();

            if choice == crate::ui::widget::CloseChoice::Minimize {
                let cfg = env.cfg();
                let action = window_close_action("minimize", cfg.tray_enabled,
                    cfg.taskbar_widget_enabled && env.taskbar.host_available());
                if action == WindowCloseAction::Hide {
                    env.window.set_visible(false);
                } else if let Some(hwnd) = env.window.handle.hwnd() {
                    unsafe { win32::ShowWindow(hwnd as _, 6); }
                }
                env.audio.set_active(1, false);
                // 藏起来了，把位图和字体还回去；恢复后首帧自动重建。
                env.canvas.release_render_memory();
            } else {
                env.signal.request_quit();
                nwg::stop_thread_dispatch();
            }
        }

        Act::SetTheme(idx) => {
            let pref = match idx {
                1 => crate::ui::theme::PREF_LIGHT,
                2 => crate::ui::theme::PREF_DARK,
                _ => crate::ui::theme::PREF_SYSTEM,
            };
            apply_theme_pref(env, pref);
        }

        Act::OpenDrop(d) => {
            env.canvas.with_ui_mut(|u| u.set_open_drop(Some(d)));
            env.canvas.repaint_now();
        }

        Act::CloseDrop => {
            env.canvas.with_ui_mut(|u| u.set_open_drop(None));
            env.canvas.repaint_now();
        }

        Act::SetPaletteTab(tab) => {
            env.canvas.with_ui_mut(|u| u.palette_tab = tab);
            env.canvas.repaint_now();
        }

        Act::SetCustomHue(_d, hue) => {
            env.canvas.with_ui_mut(|u| u.custom_hue = hue);
            env.canvas.repaint_now();
        }

        Act::ToggleAdv(i) => {
            let idx = (i as usize).min(1);
            env.canvas.with_ui_mut(|u| u.adv[idx] = !u.adv[idx]);
            env.canvas.repaint_now();
        }

        Act::PickDrop(d, idx) => pick_drop(env, d, idx),

        Act::Focus(e) => {
            if let Edit::DpiPreset(index) = e {
                if let Some(basic) = env.basic() {
                    let presets = env.cfg().presets_for(&basic.target.key, &basic.dpi);
                    if let Some(value) = presets.values.get(usize::from(index)) {
                        env.canvas.with_ui_mut(|u| u.begin_edit(e, value.to_string()));
                    }
                }
                env.canvas.repaint_now();
                return;
            }
            if let Edit::NewBasicDpiX(_) | Edit::NewBasicDpiY(_) = e {
                if env.basic().is_some_and(|b| b.connected && b.dpi.stage_editable && b.dpi.max_stages > 1) {
                    env.canvas.with_ui_mut(|u| u.begin_edit(e, String::new()));
                }
                env.canvas.repaint_now();
                return;
            }
            if let Edit::BasicDpiX(index) | Edit::BasicDpiY(index) = e {
                if let Some(basic) = env.basic().filter(|b| b.connected && b.dpi.stage_editable) {
                    if let Some(original) = crate::ui::widget::BasicDpiDraft::from_snapshot(&basic,index) {
                        env.canvas.with_ui_mut(|u| {
                            let original = u.basic_dpi_drafts.entry(index).or_insert(original).clone();
                            let value = if matches!(e,Edit::BasicDpiX(_)) {original.stage.value} else {original.y};
                            u.begin_edit(e,value.to_string());
                        });
                    }
                }
                env.canvas.repaint_now();
                return;
            }
            if e == Edit::Dpi(0) {
                if let Some(basic) = env.basic() {
                    if basic.connected && basic.dpi.writable {
                        let init = basic.dpi.current.map(|dpi| dpi.to_string()).unwrap_or_default();
                        env.canvas.with_ui_mut(|ui| ui.begin_edit(e, init));
                        env.canvas.repaint_now();
                    }
                    return;
                }
            }
            // **这里必须自己建草稿**：`Ui::on_press` 命中 `Act::Focus` 时只设
            // `focus`，`edits` 里还是空的。不建草稿的话 `type_char` 找不到槽位
            // 直接返回 `false`，表现是「点进输入框后敲字没反应」。
            let st = env.state();
            // 读不到设备时进编辑态等于让用户改一个编出来的值（800 / 000000），
            // 改完写回设备才知道读的是假的 —— 直接拦住。
            if e.is_device() && !device_known(st.as_ref()) {
                // `on_press` 已经把焦点移过来了，这里要退回去 —— 留着焦点环
                // 用户会以为能输入，敲字却毫无反应。
                env.canvas.with_ui_mut(|u| u.focus = None);
                env.canvas.repaint_now();
                env.say(crate::tr!("✘ 读不到设备：插好接收器，或动一下设备再试"));
                return;
            }
            let init = edit_initial(e, st.as_ref());
            env.canvas.with_ui_mut(|u| u.begin_edit(e, init));
            env.canvas.repaint_now();
        }

        Act::ApplyEdit(e) => apply_edit(env, e),
        Act::UseDpiPreset(value) => {
            if let Some(basic) = env.basic() {
                if let Some(cmd) = dpi_preset_command(&env.cfg(), &basic, value) { env.submit(cmd); }
                else { env.say(crate::tr!("当前连接不支持 DPI 预设写入")); }
            }
        }
        Act::DpiPresetMode(_) | Act::DpiPresetCount(_) => {
            update_dpi_presets(env, a, None);
        }
        Act::BasicDpi(value) => {
            let basic = env.basic();
            if let Some(basic) = basic.filter(|basic| basic.connected && basic.dpi.writable && basic.dpi.values.contains(&value)) {
                drop_draft(env, Edit::Dpi(0));
                env.submit(Cmd::BasicDpi { target: basic.target, dpi: value });
            }
        }
        Act::BasicDpiStage(stage, context) => {
            let basic = env.basic();
            if let Some(basic) = basic.filter(|basic| basic.connected && basic.dpi.stage_writable
                && basic.dpi.stage_context() == Some(context) && basic.dpi.stages.contains(&stage)) {
                env.submit(Cmd::BasicDpiStage { target: basic.target, stage, context });
            }
        }
        Act::ToggleDpiDeleteMode => {
            env.canvas.with_ui_mut(|u| {
                u.dpi_delete_mode = !u.dpi_delete_mode;
            });
            env.canvas.repaint_now();
        }
        Act::BasicDpiStageRemove(stage, context) => {
            if let Some(basic) = env.basic().filter(|b| b.connected && b.dpi.stage_editable
                && b.dpi.max_stages > 1 && b.dpi.stages.len() > 1
                && b.dpi.active_stage != Some(stage.index)
                && b.dpi.stage_context() == Some(context)
                && b.dpi.stages.contains(&stage)) {
                if basic.dpi.stages.len() <= 2 {
                    env.canvas.with_ui_mut(|u| u.dpi_delete_mode = false);
                }
                env.submit(Cmd::BasicDpiStageRemove { target: basic.target, stage, context });
            }
        }
        Act::AddBasicDpiStage => {
            if let Some(basic) = env.basic().filter(|b| b.connected && b.dpi.stage_editable && b.dpi.max_stages > 1) {
                env.canvas.with_ui_mut(|u| {
                    let used = basic.dpi.stages.len() + usize::from(u.new_basic_dpi_count);
                    if used < usize::from(basic.dpi.max_stages) {
                        u.new_basic_dpi_count += 1;
                    }
                });
                env.canvas.repaint_now();
            }
        }
        Act::RemoveNewBasicDpiStage(slot) => {
            env.canvas.with_ui_mut(|u| u.remove_new_basic_dpi(slot));
            env.canvas.repaint_now();
        }
        Act::BasicPollingRate(hz) => {
            if let Some(basic) = env.basic().filter(|b| b.connected && b.polling_rate.writable
                && b.polling_rate.values.contains(&hz)) {
                env.submit(Cmd::BasicPollingRate { target: basic.target, hz });
            }
        }

        Act::SetSlide(s, frac) => {
            // 拖拽中只改「这一帧显示什么」，**不写设备**。
            // 写一轮设备几十毫秒，拖一次会积压几十条命令，滑块会慢半拍。
            env.canvas.with_ui_mut(|u| u.set_live_frac(s, frac));
            if s == Slide::AudioVolume {
                let slot = env.canvas.with_ui(|u| u.audio_slot);
                env.audio.set_volume(slot, (frac.clamp(0.0, 1.0) * 100.0).round() as u8);
            }
            env.canvas.repaint_now();
        }

        Act::CommitSlide(s) => commit_slide(env, s),

        Act::SelectAudio(slot) => {
            env.canvas.with_ui_mut(|u| {
                u.audio_slot = slot;
                u.clear_live_frac(Slide::AudioVolume);
            });
            env.audio.refresh();
            env.canvas.repaint_now();
        }
        Act::ToggleMicrophone => {
            env.audio.toggle_microphone();
            env.canvas.repaint_now();
        }
        Act::OpenSoundSettings => {
            if let Err(error) = crate::audio::open_sound_settings() {
                env.say(format!("✘ {error}"));
            }
        }

        // `on_press` 只把它写进 `pressed_slide`，不需要额外处理。

        Act::Toggle(p) => toggle_pref(env, p),

        // 按键页已经改成 `Drop::ButtonTarget` 真下拉，这条只为穷尽 match 保留。

        Act::Backup => pick_backup_file(env, true),
        Act::Restore => pick_backup_file(env, false),

        Act::OpenHistoryFolder => {
            let p = crate::history::history_path();
            let dir = p.parent().map(|d| d.to_path_buf()).unwrap_or(p);
            open_in_explorer(env, &dir);
        }

        Act::ChangeHistoryPath => pick_save_path(env, SavePathKind::History),
        Act::ChangeSettingsPath => pick_save_path(env, SavePathKind::Settings),
        Act::ResetHistoryPath => reset_custom_path(env, SavePathKind::History),
        Act::ResetSettingsPath => reset_custom_path(env, SavePathKind::Settings),

        Act::OpenLightingSettings => {
            if let Err(e) = crate::dynlight::open_settings() {
                env.say(format!("✘ {e}"));
            }
        }

        // 重新探测 LampArray。这里会真的枚举 HID（约几十毫秒），
        // 但只在用户点按钮时发生，不是每帧，也不会在绘制路径上。
        Act::RefreshLighting => match crate::dynlight::refresh() {
            Some(l) => env.say(crate::trf!(
                "✔ 已识别灯效设备（{} 颗灯珠 · {}）",
                l.lamp_count,
                l.kind_name()
            )),
            None => env.say(crate::tr!("✘ 未发现支持系统动态光效的设备")),
        },

        Act::ClearHistory => {
            let p = crate::history::history_path();
            let q = crate::trf!(
                "确定清空历史记录？\n\n{}\n\n清空后续航估算会重新积累样本。",
                p.display()
            );
            if !ask_confirm(crate::tr!("清空历史记录"), &q) {
                return;
            }
            let gate = lock_shared(env.shared).history_gate.clone();
            let _guard = gate.lock().unwrap_or_else(|e| e.into_inner());
            if let Err(e) = crate::history::clear_device_history() {
                env.say(crate::trf!("✘ 删除失败：{e}", e = e));
                return;
            }
            lock_shared(env.shared).reset_history();
            env.say(crate::tr!("✔ 历史记录已清空"));
        }

        Act::OpenRepo => open_url(env, "https://github.com/youmikk/X-Battery"),
        Act::CheckUpdate => {
            env.store.controller.check();
            env.canvas.repaint_now();
        }
        Act::UpdateNow => {
            if !env.store.view().available || env.store.view().phase.busy() {
                return;
            }
            env.canvas.with_ui_mut(|ui| ui.show_update_dialog = false);
            env.store.controller.update();
            env.canvas.repaint_now();
        }
        Act::ExitToStoreUpdate => {
            if env.store.begin(StorePreparationAction::ExitToStore, env.shared) {
                env.canvas.repaint_now();
            }
        }
        Act::RestartUpdatedApp => {
            if env.store.begin(StorePreparationAction::Restart, env.shared) {
                env.canvas.repaint_now();
            }
        }
        Act::DismissUpdate => {
            env.canvas.with_ui_mut(|ui| ui.show_update_dialog = false);
            env.store.controller.dismiss();
            env.canvas.repaint_now();
        }
        Act::CancelUpdate => {
            env.store.cancel();
            env.canvas.repaint_now();
        }
        Act::ReviewStore => open_url(env, crate::store::REVIEW_URL),
        Act::SoftwareFeedback => {
            let dark = crate::ui::theme::resolve(&lock_shared(env.shared).cfg.theme) == crate::ui::theme::Mode::Dark;
            env.feedback_dialog.show_software(env.window, dark);
        }
        Act::DeviceFeedback | Act::FeedbackHistory => {
            let s = lock_shared(env.shared);
            let devices = feedback_candidates(&s);
            let dark = crate::ui::theme::resolve(&s.cfg.theme) == crate::ui::theme::Mode::Dark;
            drop(s);
            env.feedback_dialog.show(env.window, devices, dark, matches!(a, Act::FeedbackHistory));
        }

        Act::OpenStore => open_url(env, crate::store::PRODUCT_URL),
    }
}

fn apply_theme_pref(env: &UiEnv, pref: &str) {
    let pref = crate::ui::theme::normalize_pref(pref);
    if env.set_cfg(|c| c.theme = pref.to_string()).is_err() { return; }
    let m = crate::ui::theme::resolve(pref);
    let idx = match pref {
        crate::ui::theme::PREF_LIGHT => 1u8,
        crate::ui::theme::PREF_DARK => 2,
        _ => 0,
    };
    env.canvas.with_ui_mut(|u| {
        u.mode = m;
        u.theme_pref = idx;
    });
    if let Some(h) = env.window.handle.hwnd() {
        crate::dwm::refresh_titlebar(h as crate::win32::Handle, m == crate::ui::theme::Mode::Dark);
    }
    env.canvas.repaint_now();
}

/// 色盘选中一个颜色。`slot` 是 `LIGHT_COLOR_SLOT`（灯光）或 0..3（DPI 档位）。
///
/// `idx` 在预设模式下是颜色下标，在色环模式下是打包的 24-bit RGB ——
/// `palette_color` 两种都认。
fn pick_palette(env: &UiEnv, slot: u8, idx: usize) {
    let Some(rgb) = crate::ui::palette::palette_color(idx) else {
        return;
    };
    if slot == crate::ui::widget::LIGHT_COLOR_SLOT {
        let st = env.state();
        let mode = st.as_ref().map(|s| s.light_mode).unwrap_or(1);
        // 颜色变了要把草稿清掉：输入框里留着旧色号的话，
        // 用户接着点「应用」会把刚挑的颜色又改回去。
        drop_draft(env, Edit::LightColor);
        env.submit(Cmd::Light {
            mode,
            bright: None,
            rgb: Some(rgb),
            dir: None,
            random: None,
            speed: None,
        });
    } else {
        let i = (slot as usize) & 3;
        // 读不到设备就**不写**：`Cmd::Dpi` 必须给一整套值，拿不到当前 DPI
        // 就只能猜（以前猜 800），猜错就是把用户那一档的 DPI 一起改掉 ——
        // 用户点的是颜色，结果灵敏度变了，这是最糟的一种「顺手改坏」。
        let Some(st) = env.state() else {
            env.say(crate::tr!("✘ 读不到设备，动一下设备再选颜色"));
            return;
        };
        let dpi = st.dpi(i);
        drop_draft(env, Edit::DpiColor(i as u8));
        env.submit(Cmd::Dpi {
            stage: i + 1,
            dpi,
            rgb: Some(rgb),
        });
    }
}

/// 下拉框选中一项。
fn pick_drop(env: &UiEnv, d: Drop, idx: usize) {
    // 先关弹层：无论这一项能不能执行，点完都该收起来。
    env.canvas.with_ui_mut(|u| u.set_open_drop(None));

    // 灯光页「方向」借 `LightMode` + 100 当哨兵（`pages.rs:1718`）。
    // **不先分叉的话**会把方向点击当成「选了一个不存在的灯效模式」。
    if d == Drop::LightMode && idx >= 100 {
        let dir = (idx - 100) as u8;
        let st = env.state();
        let mode = st.as_ref().map(|s| s.light_mode).unwrap_or(1);
        env.submit(Cmd::Light {
            mode,
            bright: None,
            rgb: None,
            dir: Some(dir),
            random: None,
            speed: None,
        });
        return;
    }

    // 色盘**必须先于** `option_value` 分叉。
    //
    // 色盘的「值」不是 `option_value` 那套「下标 → 协议值」映射：
    // 预设模式下 `idx` 是 0..36 的颜色下标，要查表换成 RGB；
    // 色环模式下 `idx` 本身就是打包好的 24-bit RGB。
    //
    // 原来这段写在下面 `match d` 里，而 `option_value(Palette, _)` 恒返回
    // `None`（`raw_options` 对 Palette 故意返回空表），于是 `let Some(code)
    // = ... else { return }` 直接把它挡掉了 —— `Drop::Palette` 那一支是
    // **永远走不到的死代码**。表现就是「点颜色完全没反应，弹层还照常关掉」。
    if let Drop::Palette(slot) = d {
        pick_palette(env, slot, idx);
        return;
    }

    if d == Drop::TrayDevice {
        let key = env.canvas.with_ui(|u| u.tray_device_keys.get(idx).cloned());
        let reading = key.and_then(|key| {
            lock_shared(env.shared)
                .readings
                .iter()
                .find(|r| r.key == key)
                .cloned()
        });
        if let Some(reading) = reading {
            if env.set_cfg(|cfg| {
                cfg.tray_device = Some(reading.key);
                cfg.tray_device_name = reading.name;
            }).is_err() { return; }
            env.canvas.repaint_now();
        }
        return;
    }

    let Some(code) = crate::ui::dropdown::option_value(d, idx) else {
        return;
    };

    match d {
        Drop::Language => {
            if let Some(language) = crate::i18n::Language::ALL.get(code) {
                if env.set_cfg(|cfg| cfg.language = *language).is_err() { return; }
                env.canvas.with_ui_mut(|ui| ui.clear_status());
                env.canvas.repaint_now();
            }
        }
        Drop::TrayDevice => {}
        Drop::TrayKind => {
            if let Some(key) = env.canvas.with_ui(|u| u.selected_device.clone()) {
                if let Some(kind) = crate::tray_prefs::TrayKind::ALL.get(code) {
                    if env.set_cfg(|c| c.device_tray.entry(key).or_default().kind = *kind).is_err() { return; }
                    env.canvas.repaint_now();
                }
            }
        }
        Drop::Sleep => {
            env.submit(Cmd::Sleep(code as u8));
        }
        Drop::Poll => {
            // 值就是 poll_code（0..3），ops 要的是 Hz。
            const HZ: [i32; 4] = [125, 250, 500, 1000];
            if let Some(hz) = HZ.get(code) {
                env.submit(Cmd::Poll(*hz));
            }
        }
        Drop::Stage => {
            // 值是 1 基档位（1..=4），协议也是 1..=4。
            env.submit(Cmd::Stage(code as u8));
        }
        Drop::Debounce => {
            env.submit(Cmd::Debounce(code as u8));
        }
        Drop::Lod => {
            env.submit(Cmd::Lod(code as u8));
        }
        Drop::LightMode => {
            let st = env.state();
            let bright = st.as_ref().map(|s| s.light_bright);
            let rgb = st.as_ref().map(|s| s.light_rgb);
            env.submit(Cmd::Light {
                mode: code as u8,
                bright,
                rgb,
                dir: None,
                random: None,
                speed: None,
            });
        }
        Drop::Refresh => {
            if env.set_cfg(|c| c.refresh_interval_sec = code as u64).is_err() { return; }
            env.signal
                .interval_ms
                .store((code as u64).saturating_mul(1000), Ordering::Relaxed);
            env.signal.request_refresh();
            env.say(crate::trf!("✔ 刷新间隔改为 {code} 秒", code = code));
        }
        Drop::CacheTtl => {
            if env.set_cfg(|c| c.cache_ttl_sec = code as u64).is_err() { return; }
            env.say(crate::trf!(
                "✔ 缓存有效期改为 {}",
                if code == 0 {
                    crate::tr!("不缓存（每次实时读取）").to_string()
                } else {
                    crate::trf!("{code} 秒", code = code)
                }
            ));
        }
        Drop::ButtonTarget(slot) => {
            let dst = code as u16;
            let slot = (slot as usize) & 7;
            // 绑成 `0000` 以后鼠标上那个键就彻底失效了，必须二次确认。
            let confirmed = if dst == 0 {
                let name = crate::i18n::translate(proto::BUTTON_NAME[slot]);
                ask_confirm(
                    crate::tr!("停用鼠标按键"),
                    &crate::trf!("确定停用「{name}」吗？\n\n停用后该按键会暂时屏蔽操作，之后随时可以恢复或还原备份。", name = name),
                )
            } else {
                true
            };
            if !confirmed {
                return;
            }
            env.submit(Cmd::Button {
                slot,
                dst,
                confirmed,
            });
        }
        Drop::Palette(_) => {
            // 已在函数开头分叉处理（见那里的注释），这里只为让 match 穷尽。
            unreachable!("Palette 在 option_value 之前已分叉")
        }
        Drop::CloseMode => {
            let mode = match idx {
                1 => "minimize",
                2 => "quit",
                _ => "ask",
            };
            if env.set_cfg(|c| c.close_mode = mode.to_string()).is_err() { return; }
        }
    }
}

fn update_dpi_presets(env: &UiEnv, action: Act, value: Option<u16>) {
    let Some(basic) = env.basic() else { return; };
    let key = basic.target.key.clone();
    let mut shared = lock_shared(env.shared);
    let mut cfg = shared.cfg.clone();
    let mut presets = cfg.presets_for(&key, &basic.dpi);
    let changed = match action {
        Act::DpiPresetMode(enabled) => {
            if enabled && basic.dpi.write_effect.is_none() { false }
            else { presets.enabled = enabled; true }
        }
        Act::DpiPresetCount(count) => presets.set_count(usize::from(count), &basic.dpi),
        Act::ApplyEdit(Edit::DpiPreset(index)) => value.is_some_and(|v| presets.edit(usize::from(index), v, &basic.dpi)),
        _ => false,
    };
    if !changed || !presets.valid() {
        drop(shared);
        env.say(crate::tr!("预设需使用设备支持的 DPI，不能重复，可设置 1～4 档"));
        return;
    }
    cfg.dpi_presets.insert(key, presets);
    if let Err(error) = settings::save(&cfg) {
        drop(shared);
        env.say(crate::trf!("✘ 保存设置失败：{e}", e = error));
        return;
    }
    shared.cfg = cfg;
    drop(shared);
    env.canvas.with_ui_mut(|u| {
        for index in 0..crate::device::MAX_DPI_PRESETS as u8 { u.drop_edit(Edit::DpiPreset(index)); }
    });
    env.canvas.repaint_now();
}

/// 「应用」按钮：读草稿 → 校验 → 提交。
fn apply_edit(env: &UiEnv, e: Edit) {
    if let Edit::DpiPreset(index) = e {
        let Some(text) = env.canvas.with_ui(|u| u.edit_text(e).map(str::to_string)) else {return;};
        let value = text.trim().parse::<u16>().ok();
        update_dpi_presets(env, Act::ApplyEdit(Edit::DpiPreset(index)), value);
        return;
    }
    if let Edit::NewBasicDpiX(slot) | Edit::NewBasicDpiY(slot) = e {
        if let Some(basic) = env.basic().filter(|b| b.connected && b.dpi.stage_editable && b.dpi.max_stages > 1) {
            if let Some(context) = basic.dpi.stage_context() {
                let (raw_x, raw_y) = env.canvas.with_ui(|u| (
                    u.edit_text(Edit::NewBasicDpiX(slot)).unwrap_or("").trim().to_string(),
                    u.edit_text(Edit::NewBasicDpiY(slot)).unwrap_or("").trim().to_string(),
                ));
                let parsed = match (raw_x.is_empty(), raw_y.is_empty()) {
                    (false, true) => raw_x.parse::<u16>().ok().map(|v| (v, v)),
                    (true, false) => raw_y.parse::<u16>().ok().map(|v| (v, v)),
                    (false, false) => raw_x.parse::<u16>().ok().zip(raw_y.parse::<u16>().ok()),
                    (true, true) => None,
                };
                if let Some((x, y)) = parsed.filter(|(x, y)| basic.dpi.values.contains(x) && basic.dpi.values.contains(y)) {
                    if env.submit(Cmd::BasicDpiStageAdd { target: basic.target, context, x, y }) {
                        env.canvas.with_ui_mut(|u| u.remove_new_basic_dpi(slot));
                    }
                    return;
                }
            }
        }
        env.say(crate::tr!("DPI 不在设备报告的可用范围内"));
        return;
    }
    if let Edit::BasicDpiX(index) | Edit::BasicDpiY(index) = e {
        if let Some(basic) = env.basic().filter(|b| b.connected && b.dpi.stage_editable) {
            let current = crate::ui::widget::BasicDpiDraft::from_snapshot(&basic,index);
            let original = env.canvas.with_ui(|u|u.basic_dpi_drafts.get(&index).cloned()).or_else(||current.clone());
            if original != current {
                drop_draft(env,Edit::BasicDpiX(index)); drop_draft(env,Edit::BasicDpiY(index));
                env.canvas.with_ui_mut(|u| {u.basic_dpi_drafts.remove(&index);});
                env.say(crate::i18n::translate("设备档位或配置已变化，请刷新后重试"));
                return;
            }
            if let Some(original) = original {
                let (stage,context,current_y) = (original.stage,original.context,original.y);
                let draft = |edit, value: u16| env.canvas.with_ui(|u| u.edit_text(edit)
                    .map(|s| s.trim().parse::<u16>().ok()).unwrap_or(Some(value)));
                if let (Some(x),Some(y)) = (draft(Edit::BasicDpiX(index),stage.value),draft(Edit::BasicDpiY(index),current_y)) {
                    if basic.dpi.values.contains(&x) && basic.dpi.values.contains(&y) {
                        env.submit(Cmd::BasicDpiEdit { target: basic.target, stage, context, expected_y: current_y, x, y });
                        drop_draft(env,Edit::BasicDpiX(index)); drop_draft(env,Edit::BasicDpiY(index));
                        env.canvas.with_ui_mut(|u| {u.basic_dpi_drafts.remove(&index);});
                        return;
                    }
                }
            }
        }
        env.say(crate::tr!("DPI 不在设备报告的可用范围内"));
        return;
    }
    if e == Edit::Dpi(0) {
        if let Some(basic) = env.basic() {
            let text = env.canvas.with_ui(|ui| ui.edit_text(e).map(str::to_string));
            let dpi = text.as_deref().and_then(|text| text.trim().parse::<u16>().ok());
            if let Some(dpi) = dpi.filter(|dpi| basic.connected && basic.dpi.writable && basic.dpi.values.contains(dpi)) {
                env.submit(Cmd::BasicDpi { target: basic.target, dpi });
                drop_draft(env, e);
            } else { env.say(crate::tr!("DPI 不在设备报告的可用范围内")); }
            return;
        }
    }
    let st = env.state();

    match e {
        Edit::BasicDpiX(_) | Edit::BasicDpiY(_) | Edit::NewBasicDpiX(_) | Edit::NewBasicDpiY(_) | Edit::DpiPreset(_) => return,
        Edit::LowWarn => {
            let txt = env.canvas.with_ui(|u| u.edit_text(e).map(str::to_string));
            let Some(v) = txt.as_deref().and_then(parse_num) else {
                env.say(crate::tr!("✘ 请输入 0–100 的整数"));
                return;
            };
            if !(0..=100).contains(&v) {
                env.say(crate::tr!("✘ 低电量阈值必须在 0–100 之间"));
                return;
            }
            drop_draft(env, e);
            env.submit(Cmd::LowBatt(v as u8));
        }

        Edit::Dpi(i) | Edit::DpiColor(i) => {
            let i = (i as usize) & 3;
            let dpi_txt = env.canvas.with_ui(|u| u.edit_text(Edit::Dpi(i as u8)).map(str::to_string));
            let col_txt =
                env.canvas.with_ui(|u| u.edit_text(Edit::DpiColor(i as u8)).map(str::to_string));

            // 用户没填的那一半要沿用设备当前值。设备读不到时**不能**编一个
            // 800 DPI / 纯黑出来写回去 —— 那会把用户那一档 DPI 和指示灯颜色
            // 一起改掉，而界面上看不出发生过什么。
            let typed_dpi = dpi_txt.as_deref().and_then(parse_num);
            let typed_rgb = match col_txt.as_deref() {
                Some(t) if !t.trim().is_empty() => Some(t.to_string()),
                _ => None,
            };
            if st.is_none() && (typed_dpi.is_none() || typed_rgb.is_none()) {
                env.say(crate::tr!("✘ 读不到设备，动一下设备再改"));
                return;
            }

            let dpi = match typed_dpi {
                Some(v) if (proto::DPI_MIN as i64..=proto::DPI_MAX as i64).contains(&v) => v as i32,
                Some(_) => {
                    env.say(crate::trf!(
                        "✘ DPI 必须在 {}–{} 之间（步进 {}）",
                        proto::DPI_MIN,
                        proto::DPI_MAX,
                        proto::DPI_STEP
                    ));
                    return;
                }
                None => st.as_ref().map(|s| s.dpi(i)).unwrap_or(proto::DPI_MIN as i32),
            };

            let rgb = match typed_rgb.as_deref() {
                Some(t) => match parse_color(t) {
                    Ok(Some(c)) => c,
                    Ok(None) => st.as_ref().map(|s| s.rgb[i]).unwrap_or([0, 0, 0]),
                    Err(msg) => {
                        env.say(format!("✘ {msg}"));
                        return;
                    }
                },
                None => st.as_ref().map(|s| s.rgb[i]).unwrap_or([0, 0, 0]),
            };

            drop_draft(env, Edit::Dpi(i as u8));
            drop_draft(env, Edit::DpiColor(i as u8));
            env.submit(Cmd::Dpi {
                stage: i + 1,
                dpi,
                rgb: Some(rgb),
            });
        }

        Edit::LightColor => {
            let txt = env.canvas.with_ui(|u| u.edit_text(e).map(str::to_string));
            let cur = st.as_ref().map(|s| s.light_rgb).unwrap_or([0, 0, 0]);
            let rgb = match txt.as_deref() {
                Some(t) if !t.trim().is_empty() => match parse_color(t) {
                    Ok(Some(c)) => c,
                    Ok(None) => cur,
                    Err(msg) => {
                        env.say(format!("✘ {msg}"));
                        return;
                    }
                },
                _ => {
                    env.say(crate::tr!("✘ 请输入形如 ff1929 的颜色"));
                    return;
                }
            };
            let mode = st.as_ref().map(|s| s.light_mode).unwrap_or(1);
            drop_draft(env, e);
            env.submit(Cmd::Light {
                mode,
                bright: None,
                rgb: Some(rgb),
                dir: None,
                random: None,
                speed: None,
            });
        }

    }
}

fn drop_draft(env: &UiEnv, e: Edit) {
    env.canvas.with_ui_mut(|u| u.drop_edit(e));
}

/// 松开滑块：把最终值写进设备。
///
/// 只有这一处会写 —— 拖动过程中的几十个 `SetSlide` 只改显示。
fn commit_slide(env: &UiEnv, s: Slide) {
    let frac = env.canvas.with_ui(|u| u.live_frac(s));
    match s {
        Slide::Tune => {
            let f = frac.unwrap_or(0.5);
            // 滑块映射 −20..20（步进 5），与 `pages.rs` 的 `(val+20)/40` 对称。
            let raw = (f * 40.0 - 20.0).round() as i16;
            let v = (raw / 5).clamp(-4, 4) * 5;
            env.canvas.with_ui_mut(|u| u.clear_live_frac(s));
            env.submit(Cmd::Tuning(v));
        }
        Slide::AudioVolume => {
            env.canvas.with_ui_mut(|u| u.clear_live_frac(s));
            env.canvas.repaint_now();
        }
    }
}

/// 开关类选项。
fn toggle_pref(env: &UiEnv, p: Pref) {
    let st = env.state();
    match p {
        Pref::TrayEnabled => {
            let on = !env.cfg().tray_enabled;
            if env.set_cfg(|c| c.tray_enabled = on).is_err() { return; }
            env.tray.set_visibility(on);
            if !on {
                env.canvas.with_ui_mut(|u| {
                    if u.open_drop() == Some(Drop::TrayDevice) {
                        u.set_open_drop(None);
                    }
                });
            }
            // 关掉托盘时如果窗口是隐藏的，用户就再也找不到程序了。
            if !on && !env.cfg().taskbar_widget_enabled {
                env.window.set_visible(true);
            }
            env.say(crate::trf!(
                "✔ 托盘图标已{}",
                if on {
                    crate::tr!("开启")
                } else {
                    crate::tr!("关闭")
                }
            ));
        }
        Pref::TaskbarWidget => {
            let on = !env.cfg().taskbar_widget_enabled;
            if env.set_cfg(|c| c.taskbar_widget_enabled = on).is_err() { return; }
            if !on && !env.cfg().tray_enabled {
                env.window.set_visible(true);
            }
            env.say(crate::trf!(
                "✔ 任务栏电量条已{}",
                if on {
                    crate::tr!("开启")
                } else {
                    crate::tr!("关闭")
                }
            ));
        }
        Pref::LowBatteryNotify => {
            if !(env.notifications_supported)() {
                return;
            }
            let on = !env.cfg().notify_low_battery;
            if env.set_cfg(|c| c.notify_low_battery = on).is_err() { return; }
            env.say(crate::trf!(
                "✔ 低电量系统通知已{}",
                if on {
                    crate::tr!("开启")
                } else {
                    crate::tr!("关闭")
                }
            ));
        }
        Pref::AutoStart => {
            let on = lock_shared(env.shared).autostart_on();
            if !env.submit(Cmd::AutoStart(!on)) {
                env.say(crate::tr!("✘ 当前有其他设置正在处理，开机自启暂未修改，请稍后再试"));
            }
        }
        Pref::FullEffect => {
            let cur = st.as_ref().map(|s| s.full_effect != 0).unwrap_or(true);
            env.submit(Cmd::FullEffect(if cur { 0 } else { 1 }));
        }
        Pref::AngleSnap => {
            let cur = st.as_ref().map(|s| s.angle_snap).unwrap_or(0);
            env.submit(Cmd::Snap(if cur == 0 { 1 } else { 0 }));
        }
    }
}

/// 打开文件对话框，然后把备份/还原交给后台线程。
///
/// 对话框本身必须在 GUI 线程上跑（它自己会跑一个嵌套消息循环），但**读写设备
/// 不能**在这里做：备份要读 17 帧，约 590ms，冻结窗口这么久用户会以为卡死。
fn pick_backup_file(env: &UiEnv, save: bool) {
    let mut fd = nwg::FileDialog::default();
    let (title, action) = if save {
        (crate::tr!("选择备份保存位置"), nwg::FileDialogAction::Save)
    } else {
        (
            crate::tr!("选择要还原的备份文件"),
            nwg::FileDialogAction::Open,
        )
    };
    let folder = crate::paths::backup_dir().to_string_lossy().into_owned();
    if let Err(e) = nwg::FileDialog::builder()
        .title(title)
        .action(action)
        .filters(crate::tr!("X-Battery 备份(*.txt)|所有文件(*.*)"))
        .default_folder(folder)
        .build(&mut fd)
    {
        env.say(crate::trf!("✘ 打开文件对话框失败：{e}", e = e));
        return;
    }
    if !fd.run(Some(env.window)) {
        return;
    }
    let Ok(item) = fd.get_selected_item() else {
        env.say(crate::tr!("✘ 未读取到所选文件"));
        return;
    };
    let path = PathBuf::from(item);
    let cmd = if save {
        Cmd::Backup(path)
    } else {
        Cmd::Restore(path)
    };
    if !env.submit(cmd) {
        env.say(crate::tr!("✘ 当前有其他设置正在处理，请稍后再试"));
    }
}

/// 用系统 shell 打开一个文件夹（`explorer.exe` 的 spawn 在部分受限环境下
/// 返回 Ok 但窗口并不出现，`ShellExecuteW` 走的是真正的 shell 激活，可靠）。
fn open_in_explorer(env: &UiEnv, dir: &std::path::Path) {
    let directory = match crate::paths::directory_for_shell(dir) {
        Ok(directory) => directory,
        Err(error) => {
            env.say(format!("✘ {error}"));
            return;
        }
    };
    if let Err(e) = crate::dynlight::shell_open(&directory.to_string_lossy()) {
        env.say(crate::trf!("✘ 打不开文件夹：{e}", e = e));
    }
}

/// 用默认浏览器打开一个 http(s) 链接（「关于」里的仓库 / 商店页）。
fn open_url(env: &UiEnv, url: &str) {
    if let Err(e) = crate::dynlight::shell_open(url) {
        env.say(crate::trf!("✘ 打不开链接：{e}", e = e));
    }
}

/// 设置页里「更改保存位置」的两类目标。
#[derive(Clone, Copy, PartialEq, Eq)]
enum SavePathKind {
    History,
    Settings,
}

impl SavePathKind {
    fn label(self) -> &'static str {
        match self {
            SavePathKind::History => crate::tr!("历史文件"),
            SavePathKind::Settings => crate::tr!("设置文件"),
        }
    }
    fn current(self) -> std::path::PathBuf {
        match self {
            SavePathKind::History => crate::history::history_path(),
            SavePathKind::Settings => crate::paths::settings_file(),
        }
    }
    fn filters(self) -> &'static str {
        match self {
            SavePathKind::History => crate::tr!("历史记录(*.tsv)|所有文件(*.*)"),
            SavePathKind::Settings => crate::tr!("配置文件(*.ini)|所有文件(*.*)"),
        }
    }
}

/// 让用户选一个新的保存位置，并把旧数据搬过去。
fn pick_save_path(env: &UiEnv, kind: SavePathKind) {
    let mut fd = nwg::FileDialog::default();
    let old = kind.current();
    let default_folder = old
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Err(e) = nwg::FileDialog::builder()
        .title(crate::trf!("选择{}的保存位置", kind.label()))
        .action(nwg::FileDialogAction::Save)
        .filters(kind.filters())
        .default_folder(default_folder)
        .build(&mut fd)
    {
        env.say(crate::trf!("✘ 打开文件对话框失败：{e}", e = e));
        return;
    }
    if !fd.run(Some(env.window)) {
        return;
    }
    let Ok(item) = fd.get_selected_item() else {
        env.say(crate::tr!("✘ 未读取到所选路径"));
        return;
    };
    let new_path = std::path::PathBuf::from(item);
    if new_path == old {
        env.say(crate::tr!("保存位置与当前相同"));
        return;
    }
    let gate = lock_shared(env.shared).history_gate.clone();
    let _guard = (kind == SavePathKind::History).then(|| gate.lock().unwrap_or_else(|e| e.into_inner()));
    let res = match kind {
        SavePathKind::History => crate::history::set_custom_history_path(Some(new_path.clone())),
        SavePathKind::Settings => {
            let current = lock_shared(env.shared).cfg.clone();
            crate::settings::relocate(&current, Some(new_path.clone()))
        }
    };
    match res {
        Ok(()) => {
            if kind == SavePathKind::History { lock_shared(env.shared).reload_history(); }
            env.say(crate::trf!(
                "✔ {}已改到 {}",
                kind.label(),
                new_path.display()
            ));
            env.signal.request_refresh();
            env.canvas.repaint_now();
        }
        Err(e) => env.say(format!("✘ {e}")),
    }
}

/// 恢复默认保存位置。
fn reset_custom_path(env: &UiEnv, kind: SavePathKind) {
    let gate = lock_shared(env.shared).history_gate.clone();
    let _guard = (kind == SavePathKind::History).then(|| gate.lock().unwrap_or_else(|e| e.into_inner()));
    let res = match kind {
        SavePathKind::History => crate::history::set_custom_history_path(None),
        SavePathKind::Settings => {
            let current = lock_shared(env.shared).cfg.clone();
            crate::settings::relocate(&current, None)
        }
    };
    match res {
        Ok(()) => {
            if kind == SavePathKind::History { lock_shared(env.shared).reload_history(); }
            env.say(crate::trf!("✔ {}已恢复默认位置", kind.label()));
            env.signal.request_refresh();
            env.canvas.repaint_now();
        }
        Err(e) => env.say(format!("✘ {e}")),
    }
}

/// 输入框的初始文本。**必须和 `pages.rs` 里 `unwrap_or_else` 显示的值逐字一致**，
/// 否则点进输入框的一瞬间数字会跳一下。
/// 设备状态是不是真的读到了（占位值不算）。
fn device_known(st: Option<&proto::State>) -> bool {
    st.is_some_and(|s| !crate::ui::pages::placeholder_state(s))
}

/// 进编辑态时的草稿初值。
///
/// 读不到设备时返回空串 —— 编一个 800 / 000000 出来，用户会以为那就是设备
/// 当前值，改完写回设备才发现读的是假的。
fn edit_initial(e: Edit, st: Option<&proto::State>) -> String {
    let known = st.filter(|s| !crate::ui::pages::placeholder_state(s));
    let hex = |c: [u8; 3]| format!("{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
    match e {
        Edit::BasicDpiX(_) | Edit::BasicDpiY(_) | Edit::NewBasicDpiX(_) | Edit::NewBasicDpiY(_) | Edit::DpiPreset(_) => String::new(),
        Edit::LowWarn => known.map(|s| s.low_warn.to_string()).unwrap_or_default(),
        Edit::Dpi(i) => known
            .map(|s| s.dpi((i as usize) & 3).to_string())
            .unwrap_or_default(),
        Edit::DpiColor(i) => known.map(|s| hex(s.rgb[(i as usize) & 3])).unwrap_or_default(),
        Edit::LightColor => known.map(|s| hex(s.light_rgb)).unwrap_or_default(),
    }
}

// ==================== 结果处理 ====================

/// 用户活动触发的补读要不要真的发一次。
///
/// 第一次活动总是触发；之后 `ACTIVITY_MIN_GAP` 内不再重复，免得连点几个
/// 下拉就把设备刷爆。
fn activity_due(last: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    match last {
        None => true,
        Some(t) => now.saturating_duration_since(t) >= ACTIVITY_MIN_GAP,
    }
}

/// 两次活动补读的最小间隔。
const ACTIVITY_MIN_GAP: std::time::Duration = std::time::Duration::from_secs(10);

/// 这次读取是不是「写命令之前」的旧结果。
///
/// 单独抽出来是因为它只有两个数字，但判错方向会让界面永远显示旧值或永远不刷新。
fn outcome_is_stale(gen_at_read: u64, current: u64) -> bool {
    gen_at_read != current
}

/// 出错时追加一段诊断日志（`<设置目录>/diagnostics.log`）。
fn write_diagnostics(
    status: &str,
    family: &str,
    frames: usize,
    elapsed_ms: u128,
    device: Option<&crate::diagnostics::DeviceInfo>,
) {
    let facts = diagnostic_facts(
        "device_read_failed",
        status,
        family,
        frames,
        elapsed_ms,
        device,
    );
    let _ = crate::diagnostics::append(&crate::diagnostics::log_path(), &facts);
}

fn apply_rog_outcome(outcome: &FetchOutcome, record: &DeviceSnapshot, shared: &mut Shared,
    generation: u64) -> Option<Quick> {
    if outcome_is_stale(generation, shared.write_gen) { return None; }
    let sample = apply_outcome(outcome, shared, generation);
    shared.primary_key = Some(record.reading.key.clone());
    shared.full = record.state.clone();
    shared.ident = Some(record.ident.clone());
    shared.device_family = record.reading.family;
    sample
}

fn apply_outcome(out: &FetchOutcome, s: &mut Shared, gen_at_read: u64) -> Option<Quick> {
    // `elapsed` 为零 = 缓存命中（`SnapshotCache` 的约定）
    let snap = &out.snapshot;
    // 链路模式跟着快照一起更新（`fetch` / `fetch_light` 都会写这个字段）。
    if !snap.link_mode.is_empty() {
        s.link_mode = snap.link_mode.clone();
    }

    if out.frames_read == 0 {
        // 一帧都没读到。区分「休眠」和「真错」靠 failed 里的原因文字。
        let reason = snap
            .failed
            .values()
            .next()
            .cloned()
            .unwrap_or_else(|| crate::tr!("设备暂未响应，请确认接收器已插稳或轻动设备唤醒后再试").to_string());
        // 三态：接收器在、设备不应答 = 休眠（动一下就能唤醒）；接收器都不在 =
        // 真错误（要用户去查线和口）。两件事用一句话说会把人引向错误方向。
        let link = crate::linkstate::classify(0, snap.identity.dongle_sn.as_str());
        s.status = match link {
            crate::linkstate::LinkState::ReceiverOnly => Status::Asleep,
            _ => Status::Error(format!(
                "{}（{reason}）",
                crate::linkstate::describe(link)
            )),
        };
        // 真读不到设备时落一份诊断文本，用户能直接把事实贴过来。
        if matches!(link, crate::linkstate::LinkState::Absent) {
            if let Status::Error(ref text) = s.status {
                let device = s
                    .primary_key
                    .as_ref()
                    .and_then(|key| s.devices.get(key))
                    .map(diagnostic_device);
                write_diagnostics(
                    text,
                    crate::device::family_of(&snap.identity.model_name).noun(),
                    s.frames,
                    s.elapsed_ms,
                    device.as_ref(),
                );
            }
        }
        s.full = None;
        s.quick = None;
        s.last_ok_at = None;
        s.elapsed_ms = 0;
        return None;
    }

    // 读的过程中用户提交了写命令 → 这份结果是写之前的状态，整轮丢弃。
    if outcome_is_stale(gen_at_read, s.write_gen) {
        return None;
    }

    let Some(q) = Quick::from_snapshot(snap) else {
        // 保留上一轮的电量显示（比清成「—」更不吓人），但**不产生新采样**。
        s.frames = out.frames_read;
        s.elapsed_ms = out.elapsed.as_millis();
        s.status = Status::Asleep;
        return None;
    };
    s.quick = Some(q);
    s.last_ok_at = Some(std::time::Instant::now());

    if let Ok(configuration) = snap.configuration() {
        s.device_family = crate::device::family_of(&snap.identity.model_name);
        s.full = Some(configuration.state().clone());
        s.ident = Some(snap.identity.clone());
    }
    if let Some(state) = &mut s.full {
        q.apply_to(state);
    }

    s.frames = out.frames_read;
    s.elapsed_ms = out.elapsed.as_millis();
    s.status = Status::Ok;
    Some(q)
}

/// 命令队列上限。
///
/// 拖滑块时 `Act::CommitSlide` 一次只发一条，但用户可能连点几个下拉、
/// 再按几下「应用」。给到 4 就够覆盖正常手速，又不会在后台卡住时
/// 堆出一长串「等会儿再写」的旧值（那会让松手后的设备慢慢追你早就改掉的数）。
const MAX_PENDING: usize = 4;

/// 把请求排进队列。
///
/// 返回 `false` = **这条命令没进队列**（队列满了）。调用方该把这个告诉用户，
/// 不能像以前那样静默丢掉 —— 用户点了一下却什么都没发生，只会以为是坏的。
///
/// `Status::Busy` 只表示「后台正在写设备」。读快照期间**不再拒绝新命令**：
/// 那时设备是空闲的，命令完全可以排队等这一轮读完。
fn submit(shared: &Arc<Mutex<Shared>>, signal: &Signal, cmd: Cmd) -> bool {
    if let Some(target) = cmd.basic_target() {
        let backend = target.backend;
        if backend != crate::backends::BackendId::Rog {
            let group = crate::backends::communication_group(target);
            let mut state = lock_shared(shared);
            if state.store_update_pending { return false; }
            let queue = state.source_pending.entry(backend).or_default();
            if queue.len() >= MAX_PENDING { return false; }
            queue.push_back(cmd);
            let generation = state.source_write_gen.entry(backend).or_default();
            *generation = generation.wrapping_add(1);
            let generation = state.source_channel_gen.entry((backend, group)).or_default();
            *generation = generation.wrapping_add(1);
            state.source_busy.insert(backend);
            drop(state);
            signal.request_refresh();
            return true;
        }
    }
    {
        let mut s = lock_shared(shared);
        if s.store_update_pending { return false; }
        if s.pending.len() >= MAX_PENDING {
            return false;
        }
        if !matches!(cmd, Cmd::AutoStart(_)) && cmd.basic_target().is_none()
            && s.active_key.as_ref().and_then(|key| s.devices.get(key)).is_some_and(|record| record.basic.is_some()) {
            return false;
        }
        let target = match cmd.basic_target() {
            Some(target) => s.devices.values().find(|device|
                device.link.as_ref().is_some_and(|link| target.hid().is_ok_and(|i| link.iface.path == i.path)
                    && link.report_id == target.address)).and_then(DeviceSnapshot::command_target),
            None => s.active_key.as_ref().and_then(|key| s.devices.get(key)).and_then(DeviceSnapshot::command_target),
        };
        s.pending.push_back((cmd, target));
        s.status = Status::Busy;
        s.write_gen = s.write_gen.wrapping_add(1);
    }
    signal.request_refresh();
    true
}

/// 后台线程做一次读取。
///
/// `full == true` 读 17 帧（身份 + 性能 + 灯光 + 按键全都有），
/// `false` 只读 3 帧（电量 / 回报率 / 档位）。3 帧约 95 ms，17 帧约 590 ms。
///
/// 每次重新 `open_link` 而不长期持有句柄：鼠标休眠/唤醒、接收器重连都会让
/// 旧句柄失效，长期持有反而要处理一堆失效重开逻辑。
// ==================== 后台线程执行命令 ====================

/// 写设备时 `ops` 的 `keep` / `save` 两个开关。
///
/// 抽成常量不是为了省字，是因为 **`keep` 这个名字的正反语义极容易写反**：
/// `keep = false` 会走 `ops::cycle` 的步骤 4，**把旧值写回去**。
// ==================== 辅助函数 ====================

/// 自启状态的**短**名（设置页那一行）。
///
/// 不用 `AutoStartState::describe()`：那句最长 23 个汉字，放不下会被截断。
fn auto_state_short(s: crate::autostart::AutoStartState) -> &'static str {
    use crate::autostart::AutoStartState as S;
    match s {
        S::Disabled => crate::tr!("未启用"),
        S::DisabledByUser => crate::tr!("已被用户关闭"),
        S::Enabled => crate::tr!("已启用"),
        S::DisabledByPolicy => crate::tr!("组策略禁止"),
        S::EnabledByPolicy => crate::tr!("组策略启用"),
        S::Unknown => crate::tr!("状态未知"),
    }
}

/// 自启失败/无法生效时给用户的完整解释。
pub(crate) fn autostart_hint(
    requested_on: bool,
    state: crate::autostart::AutoStartState,
    path: crate::autostart::AutoStartPath,
) -> String {
    use crate::autostart::{AutoStartPath as P, AutoStartState as S};

    let where_ = match path {
        P::WinRt => crate::tr!("系统「设置 → 应用 → 启动」"),
        P::Registry => crate::tr!("系统「设置 → 应用 → 启动」或任务管理器「启动」页"),
        P::Unknown => crate::tr!("系统「设置 → 应用 → 启动」"),
    };

    if requested_on {
        match state {
            S::Enabled | S::EnabledByPolicy => {
                crate::trf!("已开启开机自启（{}）。" , crate::i18n::translate(state.describe()))
            }
            S::DisabledByUser => crate::trf!("你在系统设置中关闭过开机自启，软件无法直接替你改回，请前往 {where_} 开启「{WINDOW_TITLE}」。" , WINDOW_TITLE = WINDOW_TITLE, where_ = where_),
            S::DisabledByPolicy => {
                crate::tr!("受系统策略限制，暂无法由应用开启开机自启，如需调整请联系管理员。").to_string()
            }
            S::Disabled => crate::trf!("未能自动开启开机自启，请前往 {where_} 手动开启。" , where_ = where_),
            S::Unknown => crate::trf!("读不到自启状态，请到 {where_} 确认。" , where_ = where_),
        }
    } else {
        match state {
            S::Enabled | S::EnabledByPolicy => {
                crate::trf!(
                    "未能自动关闭开机自启（{}），请前往 {where_} 手动关闭。",
                    crate::i18n::translate(state.describe()),
                    where_ = where_
                )
            }
            _ => crate::trf!(
                "已关闭开机自启（{}）。",
                crate::i18n::translate(state.describe())
            ),
        }
    }
}

/// 托盘悬停提示（多行）：鼠标名称 / 连接方式 / 电量 / 当前 DPI 与档位。

/// 从输入框读一个十进制/十六进制整数。
///
/// 空白返回 `None`（调用方据此保留原值，而不是当成 0）。
fn parse_num(s: &str) -> Option<i64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return i64::from_str_radix(hex, 16).ok();
    }
    t.parse::<i64>().ok()
}

/// 解析按键页的目标值。
///
/// **裸写的数字一律当十六进制**，和 CLI 的 `setbutton <slot> <dstHex>` 一致。
/// 这里不能复用 [`parse_num`]：那个把裸数字当十进制（DPI 需要那样），
/// 于是 `01F0` 会解析失败 —— 用户照着提示输，程序却说输入框是空的。

/// 解析用户输入的颜色。允许 `ff1929` / `#ff1929` / `FF1929`。
fn parse_color(s: &str) -> Result<Option<[u8; 3]>, String> {
    let t = s.trim().trim_start_matches('#');
    if t.is_empty() {
        return Ok(None);
    }
    crate::backup::parse_rgb(t).map(Some)
}

/// 二次确认框。返回 `true` = 用户点了「是」。
///
/// 按钮组合 `MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2`：
/// 默认按钮放在**第二个**（否）—— 用户手快按回车时不会误把按键绑成「无功能」。
fn ask_confirm(title: &str, text: &str) -> bool {
    ask_confirm_owned(std::ptr::null_mut(), title, text)
}

fn ask_confirm_owned(hwnd: win32::Handle, title: &str, text: &str) -> bool {
    const MB_YESNO: u32 = 0x0000_0004;
    const MB_ICONWARNING: u32 = 0x0000_0030;
    const MB_DEFBUTTON2: u32 = 0x0000_0100;
    const IDYES: i32 = 6;

    let t = crate::win32::to_wide(text);
    let c = crate::win32::to_wide(title);
    let r = unsafe {
        crate::win32::MessageBoxW(
            hwnd,
            t.as_ptr(),
            c.as_ptr(),
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
        )
    };
    r == IDYES
}

// ==================== 入口 ====================

/// 启动 GUI。返回时进程就要退出了。
pub fn run(start_hidden: bool, after_update: bool) -> Result<(), String> {
    crate::diagnostics::install_panic_hook(APP_VERSION);
    let cfg = settings::load();
    crate::i18n::apply(cfg.language);
    nwg::init().map_err(|e| crate::trf!("界面框架初始化失败：{e}", e = e))?;
    Settings::write_default_if_missing();
    // 盘上写的配色和界面默认值必须一开始就一致，否则首帧是浅色、
    // 用户一拖窗口就变成深色。`system` 在这里按系统深浅色换算成具体配色。
    let mode = crate::ui::theme::resolve(&cfg.theme);

    // 缩放比要自己算：nwg 没开 `high-dpi` feature 时 `scale_factor()` 恒返 1.0，
    // 传给 `CreateWindowExW` 的尺寸就是物理像素。
    let mut scale = crate::dwm::system_scale();
    if !(scale.is_finite() && scale > 0.2 && scale < 8.0) {
        scale = 1.0;
    }

    // 灯光页要显示「有没有 HID LampArray 设备」。枚举 HID 要几十毫秒，
    // 绝不能落在第一次 WM_PAINT 里，所以在这里（消息循环之前）先探一次。
    crate::dynlight::init();

    // ── 主窗口 ──────────────────────────────────────────────
    let mut window = nwg::Window::default();
    nwg::Window::builder()
        .title(crate::mock::window_title())
        .size(((WIN_W * scale) as i32, (WIN_H * scale) as i32))
        .center(true)
        // **先隐藏**：全部初始化（读设置/托盘图标/字体/画布/后台线程）都在主线程
        // 同步做完才开消息循环，这期间窗口若可见就是"显示着但不响应"，
        // Explorer 会把鼠标转成转圈。`MAIN_WINDOW` = WS_OVERLAPPEDWINDOW **但不含 WS_VISIBLE**，
        // 初始化完、dispatch 前再 ShowWindow。
        .flags(nwg::WindowFlags::MAIN_WINDOW)
        .build(&mut window)
        .map_err(|e| crate::trf!("无法创建应用主窗口：{e}", e = e))?;

    let hwnd = window
        .handle
        .hwnd()
        .ok_or_else(|| crate::tr!("主窗口未就绪").to_string())?
        as crate::win32::Handle;

    // 深色标题栏要和界面配色一起决定，否则浅色界面配深色标题栏很割裂。
    crate::dwm::set_dark_titlebar(hwnd, mode == crate::ui::theme::Mode::Dark);

    // alt+tab / 任务栏取的是窗口图标，窗口不显式设置就是一块空白。
    crate::win32::set_window_icon(
        hwnd,
        crate::win32::load_app_icon(32),
        crate::win32::load_app_icon(16),
    );

    // ── 去掉系统标题栏 ──────────────────────────────────────
    //
    // 顺序是**承重的**，不能调换：
    //   ① 先挂子类拦 `WM_NCCALCSIZE` / `WM_GETMINMAXINFO`；
    //   ② 再改窗口样式（`go_frameless` 内部的 `SWP_FRAMECHANGED` 会立刻触发
    //      一次非客户区重算，没挂子类的话那一次会按旧样式算出带标题栏的客户区，
    //      内容区一启动就少 39 像素高）；
    //   ③ 最后按「窗口比客户区多出来的部分」把窗口补到正好 1100×740。
    //
    // 为什么不是 `WS_POPUP` 重建窗口：那会连 `WS_THICKFRAME` 一起丢掉，
    // 边缘缩放、Aero Snap、最大化动画、任务栏右键菜单全要自己实现。
    // 只拿掉 caption、保留 thickframe，这些系统行为一个都不丢。
    let raw_handler = {
        const ID_NCCALCSIZE: usize = 0x10001;
        const ID_MINMAXINFO: usize = 0x10002;
        const ID_NCACTIVATE: usize = 0x10003;
        const ID_NCPAINT: usize = 0x10004;
        // handler_id 必须 > 0xFFFF，否则 nwg 直接 panic（那是它自己保留的区间）
        let nccalcs =
            nwg::bind_raw_event_handler(&window.handle, ID_NCCALCSIZE, |_h, msg, wp, _lp| {
                const WM_NCCALCSIZE: u32 = 0x0083;
                if msg == WM_NCCALCSIZE {
                    // 返回 0 = 客户区铺满整个窗口矩形，标题栏视觉上消失，
                    // 但 DWM 的圆角/投影/动画照旧
                    return crate::ui::frame::on_nccalcsize(wp).map(|r| r as isize);
                }
                None
            })
            .map_err(|e| crate::trf!("窗口布局组件初始化失败：{e}", e = e))?;

        let minmax =
            nwg::bind_raw_event_handler(&window.handle, ID_MINMAXINFO, |h, msg, _wp, lp| {
                const WM_GETMINMAXINFO: u32 = 0x0024;
                if msg == WM_GETMINMAXINFO {
                    // nwg 的 `h` 是 winapi 的 `HWND`，我们自己的签名用 `*mut c_void`
                    // —— 两者都是指针，`as` 转一下就对了。
                    crate::ui::frame::on_getminmaxinfo(h as crate::win32::Handle, lp);
                }
                None
            })
            .map_err(|e| crate::trf!("窗口尺寸调整初始化失败：{e}", e = e))?;

        let ncact =
            nwg::bind_raw_event_handler(&window.handle, ID_NCACTIVATE, |_h, msg, _wp, _lp| {
                const WM_NCACTIVATE: u32 = 0x0086;
                if msg == WM_NCACTIVATE {
                    // 返回 Some(1) 阻止 DefWindowProcW 绘制系统默认非客户区（经典蓝条标题栏）
                    return Some(1);
                }
                None
            })
            .map_err(|e| crate::trf!("窗口激活处理初始化失败：{e}", e = e))?;

        let ncpaint =
            nwg::bind_raw_event_handler(&window.handle, ID_NCPAINT, |_h, msg, _wp, _lp| {
                const WM_NCPAINT: u32 = 0x0085;
                if msg == WM_NCPAINT {
                    // 返回 Some(0) 阻止系统绘制默认外边框
                    return Some(0);
                }
                None
            })
            .map_err(|e| crate::trf!("窗口边框绘制初始化失败：{e}", e = e))?;

        (nccalcs, minmax, ncact, ncpaint)
    };
    let (_raw_nccalcs, _raw_minmax, _raw_ncact, _raw_ncpaint) = raw_handler;

    crate::ui::frame::go_frameless(hwnd);
    // 客户区要从 1100×740 起算。改样式之后客户区已经变成「整个窗口」，
    // 所以这一步是把窗口本身收到「客户区正好 1100×740」。
    crate::ui::frame::keep_client_size(hwnd, WIN_W, WIN_H, scale);
    // 无边框窗口默认是直角硬边，开系统原生圆角让它和桌面柔和衔接（#5）。
    // Win10 等老系统上这一步静默失败，退回直角，属于正常降级。
    crate::dwm::set_round_corners(hwnd);

    // ── 托盘 ────────────────────────────────────────────────
    // parent 必须是 MessageWindow
    let mut msg_window = nwg::MessageWindow::default();
    nwg::MessageWindow::builder()
        .build(&mut msg_window)
        .map_err(|e| crate::trf!("后台消息窗口创建失败：{e}", e = e))?;

    let mut menu = nwg::Menu::default();
    nwg::Menu::builder()
        .popup(true)
        .parent(&msg_window)
        .build(&mut menu)
        .map_err(|e| crate::trf!("系统托盘菜单创建失败：{e}", e = e))?;

    // Menu 没有 append()，每个 MenuItem 都要显式 parent。
    //
    // 菜单里只留「退出」：左键单击托盘图标已经是「打开主窗口」，
    // 「显示主窗口 / 立即刷新」两个菜单项就没有存在意义了。
    let mut mi_quit = nwg::MenuItem::default();
    nwg::MenuItem::builder()
        .text(crate::tr!("退出"))
        .parent(&menu)
        .build(&mut mi_quit)
        .map_err(|e| crate::trf!("托盘菜单项初始化失败：{e}", e = e))?;

    // 托盘图标自己管（见 `TrayCtl` 注释：nwg 不暴露 guidItem，做不到跨版本固定）。
    // 图标在消息循环起来之后再添加 —— 那时 `msg_window` 才能收回调消息。
    //
    // 用 `Arc` 包一层：主窗口处理器（要建 `UiEnv`，里面借 `&TrayCtl`）和托盘
    // 回调处理器（要处理 explorer 重启后的 `TaskbarCreated`）都得拿到它。
    let tray = std::sync::Arc::new(TrayCtl::new(
        msg_window.handle.hwnd().unwrap_or(std::ptr::null_mut()) as win32::Handle
    ));
    let tray_exit = std::sync::Arc::clone(&tray);
    if cfg.tray_enabled {
        tray.set_visibility(true);
    }
    let message_hwnd = msg_window.handle.hwnd().unwrap_or(std::ptr::null_mut()) as win32::Handle;
    let taskbar_widget = Rc::new(crate::taskbar_widget::TaskbarWidget::new(
        message_hwnd,
        TASKBAR_CALLBACK_MSG,
    ));
    taskbar_widget.sync(
        cfg.taskbar_widget_enabled,
        None,
        crate::autostart::system_uses_light_theme(),
    );

    let mut notice = nwg::Notice::default();
    nwg::Notice::builder()
        .parent(&window)
        .build(&mut notice)
        .map_err(|e| crate::trf!("界面消息通道创建失败：{e}", e = e))?;

    // ── 共享状态 ────────────────────────────────────────────
    let shared = Arc::new(Mutex::new(Shared::new(cfg.clone())));
    if let Some(readings) = crate::mock::readings() {
        lock_shared(&shared).replace_mock_readings(readings);
    }
    let signal = Arc::new(Signal::new(cfg.refresh_interval_sec));
    let store_updates = Rc::new(StoreUpdates::new(hwnd));
    let sender = notice.sender();
    let audio = crate::audio::Controls::new(move || sender.notice());
    let popup = Rc::new(crate::ui::tray_popup::Popup::new(sender, audio.clone())?);
    let name_dialog = crate::ui::device_name::Dialog::new(&window, sender)?;
    let capture_signal = Arc::clone(&signal);
    let feedback_shared = Arc::clone(&shared);
    let feedback_dialog = crate::ui::feedback::Dialog::new(&window, sender,
        move || capture_signal.request_refresh(), Some(Box::new(move || feedback_candidates(&lock_shared(&feedback_shared)))))?;

    {
        let shared_a = Arc::clone(&shared);
        std::thread::spawn(move || {
            let st = crate::autostart::state();
            lock_shared(&shared_a).autostart = st;
            sender.notice();
        });
    }

    // ── 内容区 ──────────────────────────────────────────────
    //
    // 整个客户区交给 canvas。`damage` 只是「有东西变了，快重画」，
    // 真正的绘制由注册进去的 painter 在 `WM_PAINT` 里跑。
    let sender_d = sender;
    let damage: Rc<dyn Fn()> = Rc::new(move || sender_d.notice());
    let (cw, ch) = window.size();
    let canvas = Canvas::create(hwnd, 0, 0, cw as i32, ch as i32, scale, damage)
        .map_err(|e| crate::trf!("窗口显示画布创建失败：{e}", e = e))?;
    // 句柄先留一份：`canvas` 后面会被 move 进事件闭包，但「显示窗口前先画
    // 首帧」那一步在闭包外，只能靠这个裸句柄。
    let canvas_hwnd = canvas.hwnd();
    // 首帧就带上配色：`Ui::default()` 里是 Light，盘上可能是 Dark/system。
    let theme_pref = match crate::ui::theme::normalize_pref(&cfg.theme) {
        crate::ui::theme::PREF_LIGHT => 1u8,
        crate::ui::theme::PREF_DARK => 2,
        _ => 0,
    };
    canvas.with_ui_mut(|u| {
        u.mode = mode;
        u.theme_pref = theme_pref;
        if after_update {
            u.open_after_update();
        }
    });

    {
        let shared_p = Arc::clone(&shared);
        let audio_p = audio.clone();
        let store_p = Rc::clone(&store_updates);
        canvas.set_painter(Box::new(move |inner, fonts, ui, args| {
            // 只在锁里拷一份数据，**不在锁里画** —— 画一帧要几毫秒，
            // 握着锁会让后台线程写结果时多等这段。
            let (
                mut state,
                mut ident,
                mut link_mode,
                cfg,
                autostart_text,
                autostart_on,
                mut err_text,
            ) = {
                let s = lock_shared(&shared_p);
                (
                    s.ui_state(),
                    s.ident.clone(),
                    s.link_mode.clone(),
                    s.cfg.clone(),
                    s.autostart_text(),
                    s.autostart_on(),
                    s.error_text(),
                )
            };
            let theme = *ui.mode.theme();
            let (readings, record, history) = {
                let mut s = lock_shared(&shared_p);
                if let Some(key) = ui.selected_device.as_ref().and_then(|k| s.key_aliases.get(k)) {
                    if s.devices.contains_key(key) {
                        ui.selected_device = Some(key.clone());
                    }
                }
                let readings = s.readings.clone();
                ui.sync_devices(&readings);
                s.active_key = ui.selected_device.clone();
                s.detail_key = matches!(ui.page, Page::Perf | Page::Light | Page::Keys)
                    .then(|| ui.selected_device.clone()).flatten();
                let now = std::time::Instant::now();
                for reading in readings.iter().filter(|reading| ui.page == crate::ui::widget::Page::Home
                    || ui.selected_device.as_ref() == Some(&reading.key)) {
                    record_reading_diagnostic(crate::diagnostics::readings::Surface::Main,
                        reading, s.devices.get(&reading.key), &s.cfg, true, now);
                }
                (
                    readings,
                    ui.selected_device
                        .as_ref()
                        .and_then(|k| s.devices.get(k))
                        .cloned(),
                    ui.selected_device.as_ref().and_then(|key| s.histories.get(key)).cloned().unwrap_or_default(),
                )
            };
            let selected = ui
                .selected_device
                .as_ref()
                .and_then(|key| readings.iter().find(|r| &r.key == key));
            if let Some(record) = &record {
                if let Some(basic) = &record.basic {
                    err_text = if lock_shared(&shared_p).source_busy.contains(&basic.target.backend) {
                        crate::tr!("正在写设备 ……").to_string()
                    } else { String::new() };
                }
                state = record.state.clone();
                ident = Some(record.ident.clone());
                link_mode = record
                    .link
                    .as_ref()
                    .map(|l| l.mode.clone())
                    .unwrap_or_default();
            } else if selected.is_some() {
                state = None;
                ident = None;
                link_mode.clear();
                err_text.clear();
            }
            let audio = audio_p.snapshot();
            let data = crate::ui::viewdata::Data {
                discovery_pending: !lock_shared(&shared_p).discovery_pending.is_empty(),
                selected,
                state: state.as_ref(),
                ident: ident.as_ref(),
                link_mode: &link_mode,
                // 常驻错误文字（读不到设备的原因 / 正在写设备）。
                // 瞬时提示走 `ui.status`，由 `pages::draw_toast` 优先显示。
                status: err_text,
                plot: history.plot.as_deref(),
                estimate: &history.estimate,
                rows: history.rows,
                cfg: &cfg,
                autostart_text: &autostart_text,
                autostart_on,
                native_notifications_supported: crate::notifications::supported(),
                version: APP_VERSION,
                readings: &readings,
                audio: &audio,
                basic: record.as_ref().and_then(|record| record.basic.as_ref()),
                store: store_p.view(),
            };
            canvas::draw_frame(inner, fonts, ui, args, &theme, &data)
        }));
    }

    let history_thread = {
        let shared_c = Arc::clone(&shared);
        let signal_c = Arc::clone(&signal);
        let sender_c = sender;
        std::thread::spawn(move || {
            let gate = lock_shared(&shared_c).history_gate.clone();
            let mut cached: crate::history::DeviceHistoryFile = Default::default();
            let mut plotted: crate::history::DeviceRows = Default::default();
            let mut plots: BTreeMap<String, Arc<crate::chart::Plot>> = Default::default();
            let mut loaded_epoch = None;
            let mut reload_at = std::time::Instant::now();
            while !signal_c.shutdown.load(Ordering::Relaxed) {
                {
                    let _guard = gate.lock().unwrap_or_else(|e| e.into_inner());
                    let (samples, aliases, aliases_changed, owner, epoch, keys) = {
                        let mut s = lock_shared(&shared_c);
                        s.observe_history();
                        let (samples, pending_aliases) = s.history_tracker.take_pending();
                        let aliases_changed = !pending_aliases.is_empty();
                        let mut aliases: Vec<_> = s.key_aliases.iter().map(|(old,new)| (old.clone(),new.clone())).collect();
                        aliases.extend(pending_aliases);
                        // Legacy files had no key and only sampled the primary
                        // ROG mouse. Do not assign them when ownership is ambiguous.
                        let owner = legacy_rog_history_owner(&s.devices);
                        (samples, aliases, aliases_changed, owner, s.history_epoch,
                            s.devices.keys().cloned().collect::<Vec<_>>())
                    };
                    let reloading = loaded_epoch != Some(epoch) || reload_at.elapsed() >= Duration::from_secs(30);
                    let result = if !samples.is_empty() || aliases_changed
                        || (reloading && !aliases.is_empty())
                        || (owner.is_some() && !cached.legacy.is_empty()) {
                        Some(crate::history::update_device_history(&samples, &aliases, owner.as_deref()))
                    } else if reloading { Some(crate::history::load_device_history()) } else { None };
                    if let Some(result) = result {
                        match result {
                            Ok(file) => {
                                cached = file;
                                loaded_epoch = Some(epoch);
                                reload_at = std::time::Instant::now();
                            }
                            Err(error) => {
                                crate::diagnostics::event_once("history_update_failed", &error);
                                lock_shared(&shared_c).history_tracker.retry(samples, aliases);
                            }
                        }
                    }
                    if loaded_epoch == Some(epoch) {
                        let mut views = BTreeMap::new();
                        for key in keys {
                            let rows = cached.devices.get(&key).map(Vec::as_slice).unwrap_or(&[]);
                            if plotted.get(&key).map(Vec::as_slice) != Some(rows) {
                                plots.insert(key.clone(), Arc::new(crate::chart::build(rows)));
                                plotted.insert(key.clone(), rows.to_vec());
                            }
                            views.insert(key.clone(), crate::history::HistoryView {
                                plot: plots.get(&key).cloned(), rows: rows.len(),
                                estimate: Default::default(),
                            });
                        }
                        let mut s = lock_shared(&shared_c);
                        if s.history_epoch == epoch {
                            let now = snapshot::now_unix() as i64;
                            for (key, view) in &mut views {
                                let rows = cached.devices.get(key).map(Vec::as_slice).unwrap_or(&[]);
                                view.estimate = s.history_tracker.estimate(key, rows, now);
                            }
                            s.histories = views;
                        }
                    }
                }
                sender_c.notice();
                std::thread::sleep(Duration::from_secs(1));
            }
        })
    };

    // 每个品牌有独立的读取与写入队列，慢接口不会拖住 ROG 刷新。
    let additional_threads = crate::backends::ADDITIONAL_BACKENDS.into_iter().map(|backend| {
        let shared_t = Arc::clone(&shared);
        let signal_t = Arc::clone(&signal);
        let sender_t = sender;
        std::thread::spawn(move || {
            if crate::mock::enabled() { return; }
            let mut seen = 0;
            let mut workers = device_workers::Workers::default();
            while signal_t.wait_for_work_with_limit(&mut seen, backend.publish_interval_limit_ms()) {
                if matches!(backend, crate::backends::BackendId::Bluetooth | crate::backends::BackendId::XInput) {
                    let generation = *lock_shared(&shared_t).source_write_gen.get(&backend).unwrap_or(&0);
                    let poll = crate::backends::poll_backend(backend, false).unwrap_or_default();
                    let mut state = lock_shared(&shared_t);
                    state.discovery_pending.remove(&backend);
                    state.publish_additional(backend, poll, generation, seen);
                } else {
                    match crate::backends::discover_backend(backend) {
                        Ok(scan) => {
                            {
                                let mut state = lock_shared(&shared_t);
                                state.discovery_pending.remove(&backend);
                                if scan.targets.is_empty() { state.source_discoveries.remove(&backend); }
                                else { state.source_discoveries.insert(backend, scan.clone()); }
                                state.source_completed.retain(|(source, group), _| *source != backend
                                    || scan.targets.iter().any(|target| crate::backends::communication_group(target) == *group));
                                merge_additional_update(&mut state, backend, scan.presence.clone(),
                                    std::time::Instant::now(), Publication::Presence);
                                device_workers::cancel_missing_commands(&mut state, backend, &scan.targets);
                                if scan.targets.is_empty() { state.complete_refresh(backend, seen); }
                            }
                            workers.update(backend, &scan, seen, &shared_t, &signal_t, sender_t);
                        }
                        Err(error) => {
                            crate::diagnostics::event_once("device_discovery_failed", &error);
                            let mut state = lock_shared(&shared_t);
                            state.discovery_pending.remove(&backend);
                            merge_additional(&mut state, backend, Default::default());
                            state.complete_refresh(backend, seen);
                        }
                    }
                }
                {
                    let mut state = lock_shared(&shared_t);
                    if state.device_keys_dirty {
                        if settings::save(&state.cfg).is_ok() { state.device_keys_dirty = false; }
                        else { tray_log("device identity preferences save failed"); }
                    }
                }
                sender_t.notice();
            }
        })
    }).collect::<Vec<_>>();

    // ── 主刷新线程 ──────────────────────────────────────────
    let refresh_thread = {
        let shared_t = Arc::clone(&shared);
        let signal_t = Arc::clone(&signal);
        let sender_t = sender;
        std::thread::spawn(move || {
            let mut seen = 0u64;
            if crate::mock::enabled() {
                while signal_t.wait_for_work(&mut seen) {
                    let readings = crate::mock::readings().unwrap_or_default();
                    let mut state = lock_shared(&shared_t);
                    state.replace_mock_readings(readings);
                    let mut ran = 0usize;
                    while let Some((cmd, target)) = state.pending.pop_front() {
                        apply_mock_cmd(&mut state, &cmd, target.as_ref().map(|target| &target.link));
                        ran += 1;
                        if ran >= MAX_PENDING * 2 {
                            break;
                        }
                    }
                    state.want_full = false;
                    state.status = Status::Ok;
                    state.frames = 0;
                    state.elapsed_ms = 0;
                    drop(state);
                    sender_t.notice();
                }
                return;
            }
            loop {
                if !signal_t.wait_for_work(&mut seen) {
                    return;
                }

                // ① 把队列里的写命令**全部**执行掉，一条一轮。
                //
                // 以前每轮只取一条，并且取的是单个 `Option`，所以第二条
                // 要么被丢掉要么等一整轮（读完 17 帧 590~1000ms）。
                // 现在按顺序排空：用户连点两个下拉，两条都会落地，
                // 而且后一条能正确覆盖前一条。
                //
                // 循环上限 `MAX_PENDING` 再多给几条余量：`drain` 期间
                // 界面线程可能又塞进新的（那是好事，说明用户还在操作），
                // 但不能让这个 loop 无限跑下去、把读取饿死。
                let mut ran = 0usize;
                loop {
                    let cmd = {
                        let mut state = lock_shared(&shared_t);
                        let command = state.pending.pop_front();
                        state.rog_write_active = command.is_some();
                        command
                    };
                    let Some((cmd, target)) = cmd else { break };
                    let res = match &cmd {
                        Cmd::Backup(_) | Cmd::Restore(_) => run_file_cmd(&cmd, target.as_ref()),
                        _ => run_cmd(&cmd, target.as_ref()),
                    };
                    complete_rog_command(&shared_t, &cmd, target.as_ref().map(|target| &target.link), res, crate::autostart::state);
                    sender_t.notice();
                    ran += 1;
                    if ran >= MAX_PENDING * 2 {
                        break;
                    }
                }

                // 队列空了才把 Busy 放下 —— 否则界面会闪一下「空闲」，
                // 下一轮又变回「正在写设备」。
                {
                    let mut s = lock_shared(&shared_t);
                    if s.pending.is_empty() && matches!(s.status, Status::Busy) {
                        s.status = Status::Ok;
                    }
                }

                let (full, gen_at_read) = {
                    let mut s = lock_shared(&shared_t);
                    let full = s.want_full;
                    s.want_full = false;
                    (full, s.write_gen)
                };
                rog_refresh::run(&shared_t, full, gen_at_read, sender_t);
                lock_shared(&shared_t).complete_rog_refresh(gen_at_read, seen);

                let wake_probe = {
                    let s = lock_shared(&shared_t);
                    matches!(s.status, Status::Asleep)
                };
                signal_t.set_wake_probe(wake_probe);

                sender_t.notice();
            }
        })
    };

    // ── 事件处理 ────────────────────────────────────────────
    //
    // 闭包是 `Fn`，所以所有可变状态都要放进 `Cell` / `RefCell`。
    // 多图标集合的签名：变了才重建，否则每拍都要重画 N 张图标。

    let h_mi_quit = mi_quit.handle.clone();

    let shared_e = Arc::clone(&shared);
    let signal_e = Arc::clone(&signal);
    let canvas_e = canvas.clone();
    let canvas_tray = canvas.clone();
    let popup_events = Rc::clone(&popup);
    let popup_tray = Rc::clone(&popup);

    // explorer 重启后系统广播这条消息，收到要把托盘图标重新加回去。
    // 消息号要在主处理器 move 走 `tray` 之前先备好一份 Arc。
    let taskbar_created = crate::win32::register_window_message("TaskbarCreated");
    let tray_tb = std::sync::Arc::clone(&tray);

    let canvas_min = canvas_e.clone();
    let shared_language = Arc::clone(&shared);
    let audio_min = audio.clone();
    let tray_broadcast = Arc::clone(&tray_tb);
    let taskbar_broadcast = Rc::clone(&taskbar_widget);
    let store_min = Rc::clone(&store_updates);
    let signal_min = Arc::clone(&signal);
    let store_e = Rc::clone(&store_updates);
    let session_ending = Cell::new(false);
    const STORE_TIMER: usize = 90;
    // 第二个参数是**处理器 id**（nwg 要求 > 0xFFFF），要过滤的消息在回调里判。
    let min_handler =
        nwg::bind_raw_event_handler(&window.handle, 0x10006, move |_h, msg, wp, _lp| {
            if msg == 0x0011 {
                session_ending.set(true);
                lock_shared(&shared_language).store_update_pending = true;
                return Some(1);
            }
            if msg == 0x0016 {
                if wp != 0 {
                    signal_min.request_quit();
                    nwg::stop_thread_dispatch();
                } else {
                    session_ending.set(false);
                    if !store_min.busy() {
                        lock_shared(&shared_language).store_update_pending = false;
                    }
                }
                return Some(0);
            }
            if msg == 0x0113 && wp == STORE_TIMER {
                if !session_ending.get() {
                    store_min.controller.startup();
                    let main_active = unsafe {
                        win32::IsWindowVisible(hwnd) != 0 && win32::IsIconic(hwnd) == 0
                            && win32::GetForegroundWindow() == hwnd
                    };
                    let (changed, error) = store_min.tick(&shared_language, &signal_min);
                    let home_active = canvas_min.with_ui(|ui| ui.can_prompt_update(main_active));
                    if store_min.controller.take_prompt(home_active) {
                        canvas_min.with_ui_mut(|ui| {
                            ui.cancel_press();
                            ui.dialog_hots.clear();
                            ui.show_update_dialog = true;
                        });
                        canvas_min.focus();
                        canvas_min.invalidate();
                    }
                    if let Some(error) = error {
                        canvas_min.with_ui_mut(|ui| ui.set_status(format!("{} {error}", crate::ui::feedback::text("✘ 更新准备失败：", "✘ Update preparation failed:"))));
                        canvas_min.set_animating(true);
                    }
                    if changed { canvas_min.invalidate(); }
                }
                return Some(0);
            }
            if taskbar_created != 0 && msg == taskbar_created {
                tray_broadcast.re_add();
                taskbar_broadcast.recreate();
            }
            if matches!(msg, 0x007e | 0x02e0 | 0x031a | 0x031e | 0x0320) {
                taskbar_broadcast.invalidate();
            }
            if matches!(msg, 0x0231 | 0x0232) {
                canvas_min.set_resizing(msg == 0x0231);
            }

            // SIZE_MINIMIZED = 1
            if msg == 0x0005 && wp as usize == 1 {
                canvas_min.release_render_memory();
                audio_min.set_active(1, false);
            }
            if (msg == 0x0005 && wp as usize != 1) || (msg == 0x0018 && wp != 0) {
                sender.notice();
            }
            if msg == 0x001A {
                let cfg = lock_shared(&shared_language).cfg.clone();
                if crate::i18n::apply(cfg.language) {
                    canvas_min.invalidate();
                }
                sender.notice();
            }
            None
        })
        .map_err(|e| crate::trf!("窗口缩放处理初始化失败：{e}", e = e))?;

    let wh = window.handle.clone();
    let taskbar_updates = Rc::clone(&taskbar_widget);
    let handler = nwg::full_bind_event_handler(&wh, move |evt, evt_data, _handle| {
        use nwg::Event as E;

        match evt {
            E::OnWindowClose => {
                let s = lock_shared(&shared_e);
                let action = store_e.close_action(&s.cfg.close_mode, s.cfg.tray_enabled,
                    s.cfg.taskbar_widget_enabled && taskbar_updates.host_available());
                drop(s);

                if action == WindowCloseAction::Ask {
                    // 取消直接关闭，弹出模态确认对话框
                    if let nwg::EventData::OnWindowClose(d) = evt_data {
                        d.close(false);
                    }
                    canvas_e.with_ui_mut(|u| {
                        u.show_close_dialog = true;
                        u.close_choice = crate::ui::widget::CloseChoice::Minimize;
                        u.close_remember = false;
                    });
                    canvas_e.repaint_now();
                } else if matches!(action, WindowCloseAction::Hide | WindowCloseAction::Minimize) {
                    // 最小化到托盘
                    if let nwg::EventData::OnWindowClose(d) = evt_data {
                        d.close(false);
                    }
                    if action == WindowCloseAction::Hide {
                        window.set_visible(false);
                    } else {
                        unsafe { win32::ShowWindow(hwnd, 6); }
                    }
                    audio.set_active(1, false);
                    // 同上：收进托盘后不再重绘，渲染内存全部归还。
                    canvas_e.release_render_memory();
                } else {
                    signal_e.request_quit();
                    nwg::stop_thread_dispatch();
                }
            }

            // 拉窗口大小 / 最大化 → 内容区子窗口必须跟着长。
            E::OnResize | E::OnWindowMaximize => {
                // 最小化时窗口不可见，回收渲染内存（恢复时首帧会重新分配）。
                if unsafe { crate::win32::IsIconic(hwnd) } != 0 {
                    canvas_e.release_render_memory();
                }
                canvas.refit();
            }

            E::OnNotice => {
                feedback_dialog.poll();
                lock_shared(&shared_e).expire_readings();
                for request in name_dialog.take_requests() {
                    crate::diagnostics::event(
                        "device_rename_requested",
                        &format!(
                            "key={} name_chars={}",
                            request.key,
                            request.name.chars().count()
                        ),
                    );
                    let mut s = lock_shared(&shared_e);
                    let key = s
                        .key_aliases
                        .get(&request.key)
                        .cloned()
                        .unwrap_or(request.key);
                    let mut cfg = s.cfg.clone();
                    let result = cfg
                        .set_device_name(&key, &request.name)
                        .map_err(str::to_string)
                        .and_then(|()| settings::save(&cfg));
                    if result.is_ok() {
                        s.cfg = cfg;
                        s.sort_readings();
                        crate::diagnostics::event(
                            "device_rename_saved",
                            &format!("key={key}"),
                        );
                    } else if let Err(error) = &result {
                        crate::diagnostics::event(
                            "device_rename_failed",
                            &format!("key={key} error={error}"),
                        );
                    }
                    drop(s);
                    name_dialog.finish(result.as_ref().err().map(String::as_str));
                    canvas.repaint_now();
                }
                for action in popup_events.take_actions() {
                    use crate::ui::tray_popup::Action;
                    match action {
                        Action::Refresh => {
                            let generation = signal_e.current_request();
                            popup_events.refresh_started(generation);
                            signal_e.request_refresh();
                        }
                        Action::Quit => {
                            popup_events.hide();
                            signal_e.request_quit();
                            nwg::stop_thread_dispatch();
                        }
                        Action::SetDpiStage(key, stage) => {
                            if lock_shared(&shared_e).devices.get(&key).is_some_and(|record| record.basic.is_some()) { continue; }
                            canvas.with_ui_mut(|ui| ui.select_device(&key));
                            {
                                let mut shared = lock_shared(&shared_e);
                                shared.active_key = Some(key);
                                shared.want_full = true;
                            }
                            if !submit(&shared_e, &signal_e, Cmd::Stage(stage)) {
                                popup_events.hide();
                            }
                        }
                        Action::SetDpiValue(key, dpi) => {
                            let basic = lock_shared(&shared_e).devices.get(&key).and_then(|record| record.basic.clone());
                            if let Some(basic) = basic.filter(|basic| basic.connected && basic.dpi.writable && basic.dpi.values.contains(&dpi)) {
                                submit(&shared_e, &signal_e, Cmd::BasicDpi { target: basic.target, dpi });
                            }
                        }
                        Action::SetDpiPreset(key, dpi) => {
                            let cmd = {
                                let shared = lock_shared(&shared_e);
                                shared.devices.get(&key).and_then(|r| r.basic.as_ref())
                                    .and_then(|basic| dpi_preset_command(&shared.cfg,basic,dpi))
                            };
                            if let Some(cmd) = cmd { submit(&shared_e,&signal_e,cmd); }
                        }
                        Action::SetOnboardDpiStage(key, stage, context) => {
                            let basic = lock_shared(&shared_e).devices.get(&key).and_then(|record| record.basic.clone());
                            if let Some(basic) = basic.filter(|basic| basic.connected && basic.dpi.stage_writable
                                && basic.dpi.stage_context() == Some(context) && basic.dpi.stages.contains(&stage)) {
                                submit(&shared_e, &signal_e, Cmd::BasicDpiStage { target: basic.target, stage, context });
                            }
                        }
                        Action::ReorderDevices(displayed, from, to) => {
                            persist_device_reorder(&mut lock_shared(&shared_e), &displayed, from, to);
                        }
                        Action::Settings | Action::Main => {
                            popup_events.hide();
                            match action {
                                Action::Settings => canvas.with_ui_mut(|u| {
                                    u.page = Page::Settings;
                                    u.initial_route_done = true;
                                }),
                                Action::Main => canvas.with_ui_mut(|u| {
                                    u.page = Page::Home;
                                    u.initial_route_done = true;
                                }),
                                _ => {}
                            }
                            unsafe {
                                win32::ShowWindow(hwnd, 9);
                                win32::SetForegroundWindow(hwnd);
                            }
                            canvas.repaint_now();
                        }
                        _ => {}
                    }
                }

                // ① canvas 攒下的交互动作
                let acts = canvas.take_acts();
                if !acts.is_empty() {
                    let env = UiEnv {
                        audio: &audio,
                        canvas: &canvas,
                        shared: &shared_e,
                        signal: &signal_e,
                        window: &window,
                        tray: &tray,
                        taskbar: &taskbar_updates,
                        name_dialog: &name_dialog,
                        feedback_dialog: &feedback_dialog,
                        store: &store_e,
                        save_settings: &settings::save,
                        notifications_supported: crate::notifications::supported,
                    };
                    for a in acts {
                        handle_act(a, &env);
                    }
                    // 用户在动界面：顺手补读一次，读数不该停在几分钟前。
                    signal_e.note_activity();
                }

                let audio_active = window.visible() && unsafe { crate::win32::IsIconic(hwnd) } == 0 && canvas.with_ui(|ui| {
                    let shared = lock_shared(&shared_e);
                    ui.page == Page::Battery && shared.readings.iter().any(|r| {
                        Some(&r.key) == ui.selected_device.as_ref()
                            && shared.cfg.device_kind(r) == crate::tray_prefs::TrayKind::Headset
                    })
                });
                audio.set_active(1, audio_active);

                // ② 后台回报的命令结果
                let result = { lock_shared(&shared_e).results.pop_front() };
                if let Some(r) = result {
                    let text = r.toast();
                    canvas.with_ui_mut(|u| u.set_status(text));
                    canvas.set_animating(true);
                    canvas.repaint_now();
                }

                {
                    let mut s = lock_shared(&shared_e);
                    let readings = s.readings.clone();
                    if s.cfg.ensure_tray_device(&readings) {
                        if let Err(e) = settings::save(&s.cfg) {
                            tray_log(&format!("save selection failed: {e}"));
                        }
                    }
                    tray.sync(&s.readings, &s.cfg);
                    let mut low_alerts = Vec::new();
                    for reading in &readings {
                        let key = reading.key.clone();
                        let name = s.cfg.device_name(reading).to_string();
                        let armed = s.low_alert_armed.entry(key.clone()).or_insert(true);
                        if let Some(level) = low_battery_reading(reading, armed) {
                            low_alerts.push((key, name, level));
                        }
                    }
                    if !low_alerts.is_empty() {
                        canvas.with_ui_mut(|u| {
                            u.set_status(crate::tr!("设备电量低，建议充电").to_string())
                        });
                    }
                    let notify_low = s.cfg.notify_low_battery;
                    let (readings, cfg, dpi, generation) = (
                        s.readings.clone(),
                        s.cfg.clone(),
                        popup_dpi_states(&s),
                        s.refresh_generation,
                    );
                    let now = std::time::Instant::now();
                    for reading in &readings {
                        record_reading_diagnostic(crate::diagnostics::readings::Surface::Taskbar,
                            reading, s.devices.get(&reading.key), &cfg, cfg.taskbar_widget_enabled, now);
                    }
                    drop(s);
                    taskbar_updates.sync(
                        cfg.taskbar_widget_enabled,
                        taskbar_strip_content(&readings, &cfg),
                        crate::autostart::system_uses_light_theme(),
                    );
                    if notify_low && crate::notifications::supported() {
                        for (key, name, level) in low_alerts {
                            if let Err(error) = crate::notifications::show_low_battery(
                                &key,
                                &name,
                                level,
                                LOW_ALERT_PCT,
                            ) {
                                crate::diagnostics::event_once(
                                    "low_battery_notification_failed",
                                    &error,
                                );
                            }
                        }
                    }
                    popup_events.update(&readings, &cfg, &dpi, generation);
                    if !cfg.tray_enabled && !cfg.taskbar_widget_enabled {
                        popup_events.hide();
                    }
                }

                canvas.invalidate();
            }

            _ => {}
        }
    });

    // ── 托盘图标回调 ────────────────────────────────────────
    //
    // 图标是我们自己用 `Shell_NotifyIconW` 加的（见 `TrayCtl`），鼠标事件会
    // 发到 `TRAY_CALLBACK_MSG`，由 `msg_window` 接收。nwg 那条「托盘事件」
    // 通道已经不参与，所以这里用裸处理器。
    let shared_tray = Arc::clone(&shared);
    const CLICK_TIMER: usize = 87;
    const TASKBAR_TIMER: usize = 89;
    let clicks = Cell::new(TrayClicks::default());
    let anchor = Cell::new(win32::Rect::default());
    let taskbar_strip = Rc::clone(&taskbar_widget);
    let tray_handler = unsafe {
        nwg::bind_raw_event_handler(&msg_window.handle, 0x10005, move |h, msg, wp, lp| {
            let message_hwnd = h as win32::Handle;
            if taskbar_created != 0 && msg == taskbar_created {
                clicks.set(TrayClicks::default());
                win32::KillTimer(message_hwnd, CLICK_TIMER);
                tray_tb.re_add();
                return Some(0);
            }
            if msg == 0x0113 && wp == TASKBAR_TIMER {
                taskbar_strip.tick();
                // A hidden window must remain reachable when the last battery
                // expires, Explorer restarts or the taskbar strip is unavailable.
                let tray_enabled = lock_shared(&shared_tray).cfg.tray_enabled;
                if !tray_enabled && !taskbar_strip.host_available()
                    && win32::IsWindowVisible(hwnd) == 0 {
                    win32::ShowWindow(hwnd, 7); // SW_SHOWMINNOACTIVE
                }
                return Some(0);
            }
            if msg == TASKBAR_CALLBACK_MSG {
                // A previous standard-tray click must not later toggle this popup.
                clicks.set(TrayClicks::default());
                win32::KillTimer(message_hwnd, CLICK_TIMER);
                if wp == crate::taskbar_widget::CLICK_QUERY_OPEN {
                    return Some(isize::from(popup_tray.was_open()));
                }
                if wp == crate::taskbar_widget::CLICK_CLOSE {
                    popup_tray.hide();
                } else if wp == crate::taskbar_widget::CLICK_OPEN_MAIN {
                    popup_tray.hide();
                    canvas_tray.with_ui_mut(|u| {
                        u.page = Page::Home;
                        u.initial_route_done = true;
                    });
                    win32::ShowWindow(hwnd, 9);
                    win32::SetForegroundWindow(hwnd);
                    canvas_tray.repaint_now();
                } else if wp == crate::taskbar_widget::CLICK_OPEN {
                    let Some(rect) = taskbar_strip.screen_rect() else { return Some(0); };
                    let (readings, cfg, dpi, generation) = {
                        let s = lock_shared(&shared_tray);
                        (
                            s.readings.clone(),
                            s.cfg.clone(),
                            popup_dpi_states(&s),
                            s.refresh_generation,
                        )
                    };
                    popup_tray.update(&readings, &cfg, &dpi, generation);
                    if cfg.tray_enabled || cfg.taskbar_widget_enabled {
                        popup_tray.show(rect);
                    }
                }
                return Some(0);
            }
            if msg == 0x0113 && wp == CLICK_TIMER {
                win32::KillTimer(message_hwnd, CLICK_TIMER);
                let mut click = clicks.get();
                let was_open = click.finish_single();
                clicks.set(click);
                if let Some(was_open) = was_open {
                    if was_open {
                        popup_tray.hide();
                    } else {
                        let (readings, cfg, dpi, generation) = {
                            let s = lock_shared(&shared_tray);
                            (
                                s.readings.clone(),
                                s.cfg.clone(),
                                popup_dpi_states(&s),
                                s.refresh_generation,
                            )
                        };
                        popup_tray.update(&readings, &cfg, &dpi, generation);
                        if cfg.tray_enabled || cfg.taskbar_widget_enabled {
                            popup_tray.show(anchor.get());
                        }
                    }
                }
                return Some(0);
            }
            if msg != TRAY_CALLBACK_MSG {
                return None;
            }
            if tray_tb.device_for_callback(wp, lp).is_none() {
                return Some(0);
            }
            let event = (lp as u32) & 0xffff;
            let timestamp = win32::GetMessageTime() as u32;
            let icon_rect = tray_tb
                .entries
                .borrow()
                .first()
                .and_then(|entry| win32::tray_rect(message_hwnd, entry.id));
            let (x, y) = nwg::GlobalCursor::position();
            let current = icon_rect.unwrap_or(win32::Rect {
                left: x,
                top: y,
                right: x + 16,
                bottom: y + 16,
            });
            match event {
                0x0205 | 0x007b => {
                    let mut click = clicks.get();
                    click.cancel();
                    clicks.set(click);
                    win32::KillTimer(message_hwnd, CLICK_TIMER);
                    popup_tray.hide();
                    if let Some((parent, id)) = mi_quit.handle.hmenu_item() {
                        let text = win32::to_wide(crate::tr!("退出"));
                        win32::ModifyMenuW(
                            parent as win32::Handle,
                            id,
                            0,
                            id as usize,
                            text.as_ptr(),
                        );
                    }
                    menu.popup(x, y);
                }
                0x0201 => {
                    let mut click = clicks.get();
                    click.press(popup_tray.was_open());
                    clicks.set(click);
                    anchor.set(current);
                }
                0x0203 => {
                    let mut click = clicks.get();
                    let double = click.double_click(timestamp, win32::GetDoubleClickTime());
                    if !double && !click.suppress_release {
                        click.press(popup_tray.was_open());
                        anchor.set(current);
                    }
                    clicks.set(click);
                    win32::KillTimer(message_hwnd, CLICK_TIMER);
                    if double {
                        popup_tray.hide();
                        canvas_tray.with_ui_mut(|u| {
                            u.page = Page::Home;
                            u.initial_route_done = true;
                        });
                        win32::ShowWindow(hwnd, 9);
                        win32::SetForegroundWindow(hwnd);
                        canvas_tray.repaint_now();
                    }
                }
                0x0202 | 0x0400 => {
                    let mut click = clicks.get();
                    let single = click.release(timestamp, popup_tray.was_open());
                    clicks.set(click);
                    if single {
                        anchor.set(current);
                        win32::SetTimer(message_hwnd, CLICK_TIMER, win32::GetDoubleClickTime(), 0);
                    }
                }
                0x0401 => {
                    let mut click = clicks.get();
                    click.cancel();
                    clicks.set(click);
                    win32::KillTimer(message_hwnd, CLICK_TIMER);
                    if popup_tray.was_open() {
                        popup_tray.hide();
                    } else {
                        let (readings, cfg, dpi, generation) = {
                            let s = lock_shared(&shared_tray);
                            (
                                s.readings.clone(),
                                s.cfg.clone(),
                                popup_dpi_states(&s),
                                s.refresh_generation,
                            )
                        };
                        popup_tray.update(&readings, &cfg, &dpi, generation);
                        popup_tray.show(current);
                    }
                }
                _ => {}
            }
            Some(0)
        })
    }
    .map_err(|e| crate::trf!("托盘交互初始化失败：{e}", e = e))?;
    unsafe {
        win32::SetTimer(message_hwnd, TASKBAR_TIMER, 1000, 0);
        win32::SetTimer(hwnd, STORE_TIMER, 1000, 0);
    }

    // ── 菜单事件处理器 ──────────────────────────────────────
    let signal_m = Arc::clone(&signal);
    let msg_handler = nwg::full_bind_event_handler(&msg_window.handle, move |evt, _evt_data, handle| {
        use nwg::Event as E;
        match evt {
            E::OnMenuItemSelected if handle == h_mi_quit => {
                signal_m.request_quit();
                nwg::stop_thread_dispatch();
            }
            _ => {}
        }
    });

    {
        let mut s = lock_shared(&shared);
        s.want_full = true;
    }
    signal.request_refresh();

    // 自启保留后台；没有托盘或任务栏入口时显示主窗口。
    let show_main = !(start_hidden && (cfg.tray_enabled || cfg.taskbar_widget_enabled));
    crate::diagnostics::event("app_launch_window", &format!(
        "start_hidden={start_hidden} tray_enabled={} taskbar_widget_enabled={} show_main={show_main}",
        cfg.tray_enabled, cfg.taskbar_widget_enabled,
    ));
    if show_main {
        unsafe {
            let _ = crate::win32::ShowWindow(hwnd, 1);
        }
        // 显示之后**在同一条线程里立刻**把首帧画完，再进消息循环。
        //
        // 为什么不能放在 ShowWindow 之前：`WM_PAINT` 只会投递给**可见**的
        // 窗口，父窗口还藏着的时候对子窗口 `UpdateWindow` 是空操作，画不出来。
        // 而放在这里，窗口刚可见、消息循环还没转，中间只隔一条语句，
        // 用户看不到那个「黑框之后再变正常」的闪跳。
        crate::ui::canvas::repaint_hwnd(canvas_hwnd);
        unsafe {
            let _ = crate::win32::SetForegroundWindow(hwnd);
        }
    }

    nwg::dispatch_thread_events();

    unsafe { win32::KillTimer(hwnd, STORE_TIMER); }
    store_updates.controller.shutdown();

    // Stop polling before the final history flush. A slow HID worker may outlive
    // its bounded join, so reject publications after this final cutoff.
    signal.request_quit();
    join_bounded(history_thread, Duration::from_millis(4000));
    join_bounded(refresh_thread, Duration::from_millis(4000));
    for thread in additional_threads { join_bounded(thread, Duration::from_millis(500)); }
    {
        let gate = lock_shared(&shared).history_gate.clone();
        let _guard = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (samples, aliases) = {
            let mut s = lock_shared(&shared);
            if let Err(error) = settings::save(&s.cfg) {
                crate::diagnostics::event_once("settings_shutdown_failed", &error);
            }
            s.write_gen = s.write_gen.wrapping_add(1);
            for generation in s.source_channel_gen.values_mut() {
                *generation = generation.wrapping_add(1);
            }
            for backend in crate::backends::ADDITIONAL_BACKENDS {
                let generation = s.source_write_gen.entry(backend).or_default();
                *generation = generation.wrapping_add(1);
            }
            let (samples, mut aliases) = s.history_tracker.take_pending();
            aliases.extend(s.key_aliases.iter().map(|(old, new)| (old.clone(), new.clone())));
            (samples, aliases)
        };
        if !samples.is_empty() || !aliases.is_empty() {
            if let Err(error) = crate::history::update_device_history(&samples, &aliases, None) {
                crate::diagnostics::event_once("history_shutdown_failed", &error);
            }
        }
    }

    nwg::unbind_event_handler(&handler);
    nwg::unbind_event_handler(&msg_handler);
    let _ = nwg::unbind_raw_event_handler(&tray_handler);
    let _ = nwg::unbind_raw_event_handler(&min_handler);
    // 托盘图标必须显式移除，否则任务栏上留一个点不动的幽灵图标。
    // 用独立克隆的 `Arc` 走 `set_visibility(false)`：它会按当前身份
    // （GUID 或退回的编号）删除，也会释放图标句柄。
    tray_exit.set_visibility(false);
    Ok(())
}

#[cfg(test)]
mod config_action_tests {
    use super::*;

    const SAVE_ERROR: &str = "injected settings save failure";
    const FIRST_DEVICE: &str = "1111111111111111";
    const SECOND_DEVICE: &str = "2222222222222222";
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    struct Fixture {
        canvas: Canvas,
        audio: crate::audio::Controls,
        shared: Arc<Mutex<Shared>>,
        signal: Signal,
        tray: TrayCtl,
        taskbar: crate::taskbar_widget::TaskbarWidget,
        name_dialog: crate::ui::device_name::Dialog,
        feedback_dialog: crate::ui::feedback::Dialog,
        store: StoreUpdates,
        _notice: nwg::Notice,
        window: nwg::Window,
        original_language: crate::i18n::Language,
    }

    impl Fixture {
        fn new() -> Self {
            nwg::init().unwrap();
            let mut window = nwg::Window::default();
            nwg::Window::builder().title("Configuration regression")
                .flags(nwg::WindowFlags::POPUP).ex_flags(0x0800_0080)
                .position((-32000, -32000)).size((800, 600)).build(&mut window).unwrap();
            let mut notice = nwg::Notice::default();
            nwg::Notice::builder().parent(&window).build(&mut notice).unwrap();
            let hwnd = window.handle.hwnd().unwrap() as win32::Handle;
            let canvas = Canvas::create(hwnd, 0, 0, 800, 600, 1.0, Rc::new(|| {})).unwrap();
            let language = crate::i18n::current();
            let cfg = Settings {
                theme: "light".into(), language,
                tray_device: Some(FIRST_DEVICE.into()), tray_device_name: "First mouse".into(),
                ..Settings::default()
            };
            let signal = Signal::new(cfg.refresh_interval_sec);
            let tray = TrayCtl::new(hwnd);
            tray.set_visibility(cfg.tray_enabled);
            let mut shared = Shared::new(cfg);
            shared.readings = [(SECOND_DEVICE, "Second mouse"), (FIRST_DEVICE, "First mouse")]
                .into_iter().map(|(key, name)| crate::device::DeviceReading {
                    key: key.into(), name: name.into(), family: crate::device::DeviceFamily::Mouse,
                    ..Default::default()
                }).collect();
            canvas.with_ui_mut(|ui| {
                ui.sync_devices(&shared.readings);
                ui.selected_device = Some(SECOND_DEVICE.into());
                ui.tray_device_keys = vec![FIRST_DEVICE.into(), SECOND_DEVICE.into()];
                ui.mode = crate::ui::theme::Mode::Light;
                ui.theme_pref = 1;
                ui.close_remember = true;
            });
            Self {
                canvas, audio: crate::audio::Controls::new(|| {}),
                shared: Arc::new(Mutex::new(shared)), signal, tray,
                taskbar: crate::taskbar_widget::TaskbarWidget::new(hwnd, 0),
                name_dialog: crate::ui::device_name::Dialog::new(&window, notice.sender()).unwrap(),
                feedback_dialog: crate::ui::feedback::Dialog::new(&window, notice.sender(), || {}, None).unwrap(),
                store: StoreUpdates {
                    controller: crate::store::Controller::for_test(),
                    preparation: RefCell::new(None), writes_blocked: Cell::new(false),
                    ticking: Cell::new(false),
                },
                _notice: notice, window, original_language: language,
            }
        }

        fn env<'a>(&'a self, save: &'a dyn Fn(&Settings) -> Result<(), String>) -> UiEnv<'a> {
            UiEnv {
                audio: &self.audio, canvas: &self.canvas, shared: &self.shared,
                signal: &self.signal, window: &self.window, tray: &self.tray,
                taskbar: &self.taskbar, name_dialog: &self.name_dialog,
                feedback_dialog: &self.feedback_dialog, store: &self.store,
                save_settings: save, notifications_supported: || true,
            }
        }

        fn snapshot(&self) -> RuntimeSnapshot {
            let state = lock_shared(&self.shared);
            let (theme, theme_pref, close_dialog) = self.canvas.with_ui(|ui|
                (ui.mode, ui.theme_pref, ui.show_close_dialog));
            RuntimeSnapshot {
                config: state.cfg.to_text(),
                devices: state.readings.iter().map(|reading| reading.key.clone()).collect(),
                theme, theme_pref, close_dialog, tray_enabled: self.tray.enabled.get(),
                language: crate::i18n::current(),
                interval_ms: self.signal.interval_ms.load(Ordering::Relaxed),
                request: self.signal.current_request(),
                shutdown: self.signal.shutdown.load(Ordering::Relaxed),
                visible: self.window.visible(),
            }
        }

        fn status(&self) -> String {
            self.canvas.with_ui(|ui| ui.status.clone())
        }
    }

    impl std::ops::Drop for Fixture {
        fn drop(&mut self) {
            crate::i18n::apply(self.original_language);
        }
    }

    #[derive(Debug, PartialEq)]
    struct RuntimeSnapshot {
        config: String,
        devices: Vec<String>,
        theme: crate::ui::theme::Mode,
        theme_pref: u8,
        close_dialog: bool,
        tray_enabled: bool,
        language: crate::i18n::Language,
        interval_ms: u64,
        request: u64,
        shutdown: bool,
        visible: bool,
    }

    fn config_actions() -> Vec<Act> {
        let language = if crate::i18n::english() { 1 } else { 2 };
        vec![
            Act::ToggleDeviceTray, Act::SetTheme(2), Act::SetTheme(0),
            Act::PickDrop(Drop::Language, language), Act::PickDrop(Drop::TrayDevice, 1),
            Act::PickDrop(Drop::TrayKind, 2), Act::PickDrop(Drop::Refresh, 0),
            Act::PickDrop(Drop::CacheTtl, 0), Act::PickDrop(Drop::CloseMode, 2),
            Act::Toggle(Pref::TrayEnabled), Act::Toggle(Pref::TaskbarWidget),
            Act::Toggle(Pref::LowBatteryNotify),
        ]
    }

    #[test]
    fn new_dpi_stage_keeps_input_when_queue_is_full_and_can_retry() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let fixture = Fixture::new();
        let backend = crate::backends::BackendId::Rapoo;
        let reading = crate::device::DeviceReading {
            key: SECOND_DEVICE.into(), name: "Rapoo VT1".into(), connected: true,
            family: crate::device::DeviceFamily::Mouse, ..Default::default()
        };
        let mut basic = mock_basic_snapshot(&reading);
        basic.target.backend = backend;
        basic.dpi.max_stages = 7;
        basic.dpi.configuration_token = Some(42);
        let target = basic.target.clone();
        {
            let mut shared = lock_shared(&fixture.shared);
            shared.publish_additional(backend, vec![basic], 0, 1);
            shared.source_pending.insert(backend, (0..MAX_PENDING).map(|_| Cmd::BasicDpiStageAdd {
                target: target.clone(), context: crate::device::StageContext::Configuration(42), x: 400, y: 400,
            }).collect());
        }
        fixture.canvas.with_ui_mut(|ui| {
            ui.selected_device = Some(SECOND_DEVICE.into());
            ui.new_basic_dpi_count = 2;
            ui.begin_edit(Edit::NewBasicDpiX(0), "1200".into());
            ui.begin_edit(Edit::NewBasicDpiY(0), "1400".into());
            ui.begin_edit(Edit::NewBasicDpiX(1), "2000".into());
        });
        let save = |_: &Settings| Ok(());
        apply_edit(&fixture.env(&save), Edit::NewBasicDpiX(0));
        fixture.canvas.with_ui(|ui| {
            assert_eq!(ui.new_basic_dpi_count, 2);
            assert_eq!(ui.edit_text(Edit::NewBasicDpiX(0)), Some("1200"));
            assert_eq!(ui.edit_text(Edit::NewBasicDpiY(0)), Some("1400"));
            assert_eq!(ui.edit_text(Edit::NewBasicDpiX(1)), Some("2000"));
        });
        assert_eq!(lock_shared(&fixture.shared).source_pending[&backend].len(), MAX_PENDING);
        lock_shared(&fixture.shared).source_pending.get_mut(&backend).unwrap().clear();
        apply_edit(&fixture.env(&save), Edit::NewBasicDpiX(0));
        fixture.canvas.with_ui(|ui| {
            assert_eq!(ui.new_basic_dpi_count, 1);
            assert_eq!(ui.edit_text(Edit::NewBasicDpiX(0)), Some("2000"));
            assert_eq!(ui.edit_text(Edit::NewBasicDpiY(0)), None);
        });
        let shared = lock_shared(&fixture.shared);
        assert!(matches!(shared.source_pending[&backend].front(),
            Some(Cmd::BasicDpiStageAdd { x: 1200, y: 1400, .. })));
    }

    #[test]
    fn set_cfg_returns_the_save_error_without_publishing_the_candidate() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let fixture = Fixture::new();
        let before = fixture.snapshot();
        let attempts = RefCell::new(Vec::new());
        let save = |cfg: &Settings| {
            attempts.borrow_mut().push(cfg.to_text());
            Err(SAVE_ERROR.into())
        };
        let error = fixture.env(&save).set_cfg(|cfg| cfg.theme = "dark".into()).unwrap_err();
        assert_eq!(error, crate::trf!("✘ 保存设置失败：{e}", e = SAVE_ERROR));
        assert_eq!(fixture.status(), error);
        assert_eq!(fixture.snapshot(), before);
        assert_eq!(attempts.borrow().len(), 1);
        assert_ne!(attempts.borrow()[0], before.config);
    }

    #[test]
    fn config_actions_preserve_runtime_and_failure_message_when_saving_fails() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        for action in config_actions() {
            let fixture = Fixture::new();
            let before = fixture.snapshot();
            let attempts = RefCell::new(Vec::new());
            let save = |cfg: &Settings| {
                attempts.borrow_mut().push(cfg.to_text());
                Err(SAVE_ERROR.into())
            };
            handle_act(action, &fixture.env(&save));
            assert_eq!(attempts.borrow().len(), 1, "{action:?}");
            assert_ne!(attempts.borrow()[0], before.config, "{action:?}");
            assert_eq!(fixture.snapshot(), before, "{action:?}");
            assert_eq!(fixture.status(), crate::trf!("✘ 保存设置失败：{e}", e = SAVE_ERROR), "{action:?}");
        }
    }

    #[test]
    fn update_preparation_blocks_config_actions_without_calling_the_saver() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        for action in config_actions().into_iter().chain([Act::ConfirmCloseDialog]) {
            let fixture = Fixture::new();
            lock_shared(&fixture.shared).store_update_pending = true;
            fixture.canvas.with_ui_mut(|ui| ui.show_close_dialog = action == Act::ConfirmCloseDialog);
            let before = fixture.snapshot();
            let save = |_: &Settings| -> Result<(), String> { panic!("blocked action reached saver: {action:?}") };
            handle_act(action, &fixture.env(&save));
            assert_eq!(fixture.snapshot(), before, "{action:?}");
            assert_eq!(fixture.status(), crate::ui::feedback::text(
                "更新准备期间暂时不能修改设置。", "Settings cannot be changed while preparing an update."), "{action:?}");
        }
    }

    #[test]
    fn config_actions_publish_runtime_only_after_a_successful_save() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        for action in config_actions().into_iter().filter(|action| *action != Act::SetTheme(0)) {
            let fixture = Fixture::new();
            let before = fixture.snapshot();
            let saved = RefCell::new(Vec::new());
            let save = |cfg: &Settings| {
                assert_eq!(fixture.canvas.with_ui(|ui| (ui.mode, ui.theme_pref)),
                    (before.theme, before.theme_pref), "{action:?}");
                assert_eq!(fixture.tray.enabled.get(), before.tray_enabled, "{action:?}");
                assert_eq!(crate::i18n::current(), before.language, "{action:?}");
                assert_eq!(fixture.signal.interval_ms.load(Ordering::Relaxed), before.interval_ms, "{action:?}");
                saved.borrow_mut().push(cfg.clone());
                Ok(())
            };
            handle_act(action, &fixture.env(&save));
            assert_eq!(saved.borrow().len(), 1, "{action:?}");
            let saved = saved.borrow();
            let cfg = &saved[0];
            assert_eq!(lock_shared(&fixture.shared).cfg.to_text(), cfg.to_text(), "{action:?}");
            assert!(!fixture.status().contains(SAVE_ERROR), "{action:?}");
            match action {
                Act::SetTheme(2) => assert_eq!(fixture.canvas.with_ui(|ui| (ui.mode, ui.theme_pref)),
                    (crate::ui::theme::Mode::Dark, 2)),
                Act::PickDrop(Drop::Language, _) => assert_eq!(crate::i18n::current(), cfg.language),
                Act::PickDrop(Drop::Refresh, _) => {
                    assert_eq!(fixture.signal.interval_ms.load(Ordering::Relaxed), cfg.refresh_interval_sec * 1000);
                    assert_eq!(fixture.signal.current_request(), before.request + 1);
                    assert!(fixture.status().starts_with('✔'));
                }
                Act::Toggle(Pref::TrayEnabled) => {
                    assert_eq!(fixture.tray.enabled.get(), cfg.tray_enabled);
                    assert!(fixture.status().starts_with('✔'));
                }
                Act::PickDrop(Drop::CacheTtl, _) | Act::Toggle(Pref::TaskbarWidget | Pref::LowBatteryNotify) => {
                    assert!(fixture.status().starts_with('✔'));
                }
                _ => {}
            }
        }
    }

    #[test]
    fn remembering_close_mode_must_save_before_hiding_or_quitting() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        use crate::ui::widget::CloseChoice;
        for choice in [CloseChoice::Minimize, CloseChoice::Quit] {
            let fixture = Fixture::new();
            unsafe { winapi::um::winuser::ShowWindow(fixture.window.handle.hwnd().unwrap(),
                winapi::um::winuser::SW_SHOWNOACTIVATE); }
            fixture.canvas.with_ui_mut(|ui| {
                ui.show_close_dialog = true;
                ui.close_choice = choice;
            });
            let before = fixture.snapshot();
            let save = |_: &Settings| Err(SAVE_ERROR.into());
            handle_act(Act::ConfirmCloseDialog, &fixture.env(&save));
            assert_eq!(fixture.snapshot(), before);
            assert_eq!(fixture.status(), crate::trf!("✘ 保存设置失败：{e}", e = SAVE_ERROR));
        }
    }

    #[test]
    fn failed_last_entry_toggle_keeps_the_hidden_window_hidden() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        for preference in [Pref::TrayEnabled, Pref::TaskbarWidget] {
            let fixture = Fixture::new();
            {
                let mut shared = lock_shared(&fixture.shared);
                shared.cfg.tray_enabled = preference == Pref::TrayEnabled;
                shared.cfg.taskbar_widget_enabled = preference == Pref::TaskbarWidget;
                fixture.tray.set_visibility(shared.cfg.tray_enabled);
            }
            let before = fixture.snapshot();
            assert!(!before.visible);
            let save = |_: &Settings| Err(SAVE_ERROR.into());
            handle_act(Act::Toggle(preference), &fixture.env(&save));
            assert_eq!(fixture.snapshot(), before);
            assert_eq!(fixture.status(), crate::trf!("✘ 保存设置失败：{e}", e = SAVE_ERROR));
        }
    }
}

#[cfg(test)]
mod autostart_publication_tests {
    use super::*;
    use crate::autostart::{AutoStartPath, AutoStartState};

    fn command_result() -> CmdResult {
        CmdResult { label: "test".into(), ok: Some("applied".into()), err: None, hint: "" }
    }

    #[test]
    fn slow_autostart_query_keeps_shared_available_until_atomic_publication() {
        let shared = Arc::new(Mutex::new(Shared::new(Settings::default())));
        {
            let mut state = lock_shared(&shared);
            state.rog_write_active = true;
            state.want_full = false;
        }
        let worker_shared = Arc::clone(&shared);
        let (started, entered) = std::sync::mpsc::channel();
        let (resume, paused) = std::sync::mpsc::channel();
        let expected = (AutoStartState::Enabled, AutoStartPath::WinRt);
        let worker = std::thread::spawn(move || {
            complete_rog_command(&worker_shared, &Cmd::AutoStart(true), None, command_result(), || {
                started.send(()).unwrap();
                paused.recv_timeout(Duration::from_secs(3)).unwrap();
                expected
            });
        });
        entered.recv_timeout(Duration::from_secs(3)).unwrap();
        let during_query = shared.try_lock().ok().map(|state|
            (state.rog_write_active, state.results.is_empty(), state.want_full, state.autostart));
        resume.send(()).unwrap();
        worker.join().unwrap();
        assert_eq!(during_query, Some((true, true, false,
            (AutoStartState::Unknown, AutoStartPath::Unknown))));
        let state = lock_shared(&shared);
        assert_eq!(state.autostart, expected);
        assert!(!state.rog_write_active);
        assert!(state.want_full);
        assert_eq!(state.results.len(), 1);
        assert_eq!(state.results[0].ok.as_deref(), Some("applied"));
    }

    #[test]
    fn device_command_completion_does_not_query_autostart() {
        let shared = Arc::new(Mutex::new(Shared::new(Settings::default())));
        let expected = (AutoStartState::Disabled, AutoStartPath::WinRt);
        lock_shared(&shared).autostart = expected;
        complete_rog_command(&shared, &Cmd::Stage(0), None, command_result(),
            || panic!("Device commands must not query autostart"));
        let state = lock_shared(&shared);
        assert_eq!(state.autostart, expected);
        assert_eq!(state.results.len(), 1);
    }
}

#[cfg(test)]
mod signal_tests {
    use super::*;

    #[test]
    fn quit_before_wait_skips_the_polling_interval_even_with_pending_work() {
        for pending in [false, true] {
            let signal = Signal::new(1);
            if pending { signal.request_refresh(); }
            signal.request_quit();
            let started = std::time::Instant::now();
            assert!(!signal.wait_for_work(&mut 0));
            assert!(started.elapsed() < Duration::from_millis(500));
        }
    }

    #[test]
    fn quit_wakes_all_concurrent_waiters() {
        let signal = Arc::new(Signal::new(60));
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let (sender, receiver) = std::sync::mpsc::channel();
        let workers: Vec<_> = (0..2).map(|_| {
            let signal = Arc::clone(&signal);
            let barrier = Arc::clone(&barrier);
            let sender = sender.clone();
            std::thread::spawn(move || {
                barrier.wait();
                sender.send(signal.wait_for_work_with_limit(&mut 0, Some(2000))).unwrap();
            })
        }).collect();
        barrier.wait();
        std::thread::sleep(Duration::from_millis(20));
        signal.request_quit();
        let results: Vec<_> = (0..2).map(|_| receiver.recv_timeout(Duration::from_millis(500))).collect();
        for worker in workers { worker.join().unwrap(); }
        assert_eq!(results, vec![Ok(false), Ok(false)]);
    }

    #[test]
    fn quit_serializes_with_the_wait_predicate_lock() {
        let signal = Arc::new(Signal::new(60));
        let guard = lock_poke(&signal.poke);
        let worker_signal = Arc::clone(&signal);
        let (started, entered) = std::sync::mpsc::channel();
        let (finished, done) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            started.send(()).unwrap();
            worker_signal.request_quit();
            finished.send(()).unwrap();
        });
        entered.recv_timeout(Duration::from_secs(1)).unwrap();
        let blocked = done.recv_timeout(Duration::from_millis(100));
        let shutdown_before_unlock = signal.shutdown.load(Ordering::Relaxed);
        drop(guard);
        worker.join().unwrap();
        assert_eq!(blocked, Err(std::sync::mpsc::RecvTimeoutError::Timeout));
        assert!(!shutdown_before_unlock);
        assert!(signal.shutdown.load(Ordering::Relaxed));
        assert!(!signal.wait_for_work(&mut 0));
    }
}

#[cfg(test)]
mod backend_tests {
    use super::*;
    use crate::backends::{BackendId, Snapshot, Target};
    use crate::device::{Battery, BatteryLevel, DeviceFamily, DeviceReading, Dpi};

    fn rog_energy_outcome() -> FetchOutcome {
        snapshot::fetch_light_with(3, "Wireless24G", |report_id, command, sub, _| {
            (sub == 0x07).then(|| vec![report_id, command, sub, 0, 0, 47, 4, 20, 0, 0, 0, 0, 0])
        })
    }

    #[test]
    fn rog_invalid_configuration_frames_never_publish_default_settings() {
        let mut shared = Shared::new(Settings::default());
        let mut outcome = rog_energy_outcome();
        let snapshot = Arc::make_mut(&mut outcome.snapshot);
        for key in ["12:04:02", "12:05:00", "12:03:00"] {
            snapshot.frames.insert(key.into(), vec![3, 0x12]);
            snapshot.failed.insert(key.into(), "short frame".into());
        }
        apply_outcome(&outcome, &mut shared, 0);
        assert!(shared.full.is_none());
        assert_eq!(shared.quick.unwrap().level, 47);
    }

    #[test]
    fn rog_partial_light_read_preserves_independent_settings() {
        let mut shared = Shared::new(Settings::default());
        let mut previous = proto::State::default();
        previous.poll_code = 3;
        previous.active_stage = 2;
        shared.full = Some(previous);
        apply_outcome(&rog_energy_outcome(), &mut shared, 0);
        let state = shared.ui_state().unwrap();
        assert_eq!(state.power, 47);
        assert_eq!(state.poll_code, 3);
        assert_eq!(state.active_stage, 2);
    }

    #[test]
    fn rog_partial_light_read_preserves_the_latest_valid_light_settings() {
        let mut shared = Shared::new(Settings::default());
        shared.full = Some(proto::State::default());
        let update = snapshot::fetch_light_with(3, "Wireless24G", |report_id, command, sub, _| {
            let mut frame = vec![0; 21];
            frame[..3].copy_from_slice(&[report_id, command, sub]);
            match sub {
                0x07 => frame[5] = 48,
                0x04 => frame[13] = 3,
                0x00 => frame[12] = 2,
                _ => return None,
            }
            Some(frame)
        });
        apply_outcome(&update, &mut shared, 0);
        apply_outcome(&rog_energy_outcome(), &mut shared, 0);
        let state = shared.ui_state().unwrap();
        assert_eq!(state.power, 47);
        assert_eq!(state.poll_code, 3);
        assert_eq!(state.active_stage, 2);
    }

    #[test]
    fn rog_unknown_mac_keeps_distinct_devices_and_cannot_verify_identity() {
        let first = history_owner_device("receiver-a", DeviceFamily::Mouse, true, None);
        let second = history_owner_device("receiver-b", DeviceFamily::Mouse, true, None);
        for mac in ["", "00:00:00:00:00:00", "FF:FF:FF:FF:FF:FF", "ff:ff:ff:ff:ff:ff"] {
            let identity = proto::Identity { mouse_mac: mac.into(), ..Default::default() };
            assert!(identity.device_id().is_none());
            let first_key = crate::backends::rog::identity_key(first.link.as_ref().unwrap(), &identity);
            let second_key = crate::backends::rog::identity_key(second.link.as_ref().unwrap(), &identity);
            assert_ne!(first_key, second_key);
        }
        let mut identity = proto::Identity { mouse_mac: "12:34:56:78:9A:BC".into(), ..Default::default() };
        let first_key = crate::backends::rog::identity_key(first.link.as_ref().unwrap(), &identity);
        let second_key = crate::backends::rog::identity_key(second.link.as_ref().unwrap(), &identity);
        assert_eq!(first_key, second_key);
        assert_eq!(first_key, crate::device::stable_key("device:12:34:56:78:9A:BC"));
        identity.mouse_sn = " synthetic-serial ".into();
        assert_eq!(identity.device_id(), Some("synthetic-serial"));
        assert_eq!(crate::backends::rog::identity_key(first.link.as_ref().unwrap(), &identity),
            crate::device::stable_key("device:synthetic-serial"));
    }

    #[test]
    fn rog_light_samples_require_current_identity_before_reusing_settings() {
        let mut previous = history_owner_device("receiver", DeviceFamily::Mouse, true, None);
        previous.ident.mouse_sn = "first-mouse".into();
        previous.ident.mouse_mac = "12:34:56:78:9A:BC".into();
        previous.ident.model_name = "Previous model".into();
        previous.state = Some(proto::State { poll_code: 3, active_stage: 2, ..Default::default() });
        let link = previous.link.as_ref().unwrap();
        previous.reading.key = crate::backends::rog::identity_key(link, &previous.ident);
        let mut outcome = rog_energy_outcome();
        assert!(rog_record(link, &outcome, Some(&previous)).is_none());
        Arc::make_mut(&mut outcome.snapshot).identity.mouse_sn = "first-mouse".into();
        let same = rog_record(link, &outcome, Some(&previous)).unwrap();
        assert_eq!(same.reading.key, previous.reading.key);
        assert_eq!(same.state.as_ref().unwrap().poll_code, 3);
        assert_eq!(same.reading.percent(), Some(47));
        Arc::make_mut(&mut outcome.snapshot).identity = proto::Identity {
            mouse_mac: "12:34:56:78:9a:bc".into(), ..Default::default()
        };
        let same = rog_record(link, &outcome, Some(&previous)).unwrap();
        assert_eq!(same.reading.key, previous.reading.key);
        assert_eq!(same.ident.mouse_sn, "first-mouse");
        assert!(same.state.is_some());
        Arc::make_mut(&mut outcome.snapshot).identity.mouse_sn = "replacement-mouse".into();
        let replacement = rog_record(link, &outcome, Some(&previous)).unwrap();
        assert_ne!(replacement.reading.key, previous.reading.key);
        assert_ne!(replacement.ident.model_name, "Previous model");
        assert!(replacement.state.is_none());
        assert_eq!(replacement.reading.percent(), Some(47));
    }

    #[test]
    fn rog_unrelated_identity_kinds_and_placeholder_macs_do_not_prove_continuity() {
        let serial = proto::Identity { mouse_sn: "first".into(), ..Default::default() };
        let mac = proto::Identity { mouse_mac: "12:34:56:78:9A:BC".into(), ..Default::default() };
        assert!(!serial.same_device(&mac));
        for value in ["", "00:00:00:00:00:00", "ff:ff:ff:ff:ff:ff"] {
            let unknown = proto::Identity { mouse_mac: value.into(), ..Default::default() };
            assert!(!unknown.same_device(&unknown));
        }
        let previous = history_owner_device("receiver", DeviceFamily::Mouse, true, None);
        let link = previous.link.as_ref().unwrap();
        let record = rog_record(link, &rog_energy_outcome(), Some(&previous)).unwrap();
        assert_eq!(record.reading.key, link.device_key());
        assert!(record.ident.device_id().is_none());
    }

    #[test]
    fn rog_failed_reads_preserve_measurement_age_and_require_presence_evidence_to_disconnect() {
        let mut device = history_owner_device("receiver", DeviceFamily::Mouse, true, None);
        let measured = std::time::Instant::now();
        let lifetime = Duration::from_secs(180);
        device.battery_at = Some(measured);
        device.reading.battery = snapshot(BackendId::Rog, "receiver", 75).battery;
        device.state = Some(proto::State::default());
        mark_rog_unread(&mut device, None, measured + Duration::from_secs(1), lifetime);
        assert!(device.reading.connected);
        assert_eq!(device.reading.quality, crate::device::ReadingQuality::Cached);
        assert_eq!(device.battery_at, Some(measured));
        assert!(device.state.is_none());
        let present = ["RECEIVER".into(), "independent-receiver".into()].into();
        mark_rog_unread(&mut device, Some(&present), measured + lifetime, lifetime);
        assert!(device.reading.connected);
        assert_eq!(device.reading.quality, crate::device::ReadingQuality::Stale);
        assert_eq!(device.reading.percent(), None);
        let present = ["independent-receiver".into()].into();
        mark_rog_unread(&mut device, Some(&present), measured + lifetime, lifetime);
        assert!(!device.reading.connected);
    }

    #[test]
    fn rog_fallback_channels_never_expose_or_execute_configuration_writes() {
        let mut previous = history_owner_device("nonexistent-test-path", DeviceFamily::Mouse, true, None);
        previous.ident.mouse_sn = "first-mouse".into();
        previous.state = Some(proto::State::default());
        previous.link.as_mut().unwrap().read_only = true;
        let mut outcome = rog_energy_outcome();
        Arc::make_mut(&mut outcome.snapshot).identity = previous.ident.clone();
        for (key, sub) in [("12:04:00", 4), ("12:04:02", 4), ("12:04:03", 4),
            ("12:00:00", 0), ("12:06:00", 6), ("12:03:00", 3), ("12:05:00", 5)] {
            let mut frame = vec![0; 64];
            frame[..3].copy_from_slice(&[3, 0x12, sub]);
            let snapshot = Arc::make_mut(&mut outcome.snapshot);
            snapshot.frames.insert(key.into(), frame);
            snapshot.failed.remove(key);
        }
        assert!(outcome.snapshot.configuration().is_ok());
        let record = rog_record(previous.link.as_ref().unwrap(), &outcome, Some(&previous)).unwrap();
        assert_eq!(record.reading.percent(), Some(47));
        assert!(record.state.is_none());
        let mut shared = Shared::new(Settings::default());
        apply_rog_outcome(&outcome, &record, &mut shared, 0);
        assert!(shared.full.is_none());
        assert!(shared.ui_state().is_none());
        let target = record.command_target();
        for result in [crate::commands::run_cmd(&Cmd::Stage(1), target.as_ref()),
            crate::commands::run_file_cmd(&Cmd::Restore("nonexistent-backup".into()), target.as_ref())] {
            assert!(result.ok.is_none());
            assert!(result.err.as_ref().unwrap().contains("配对信息未确认"));
        }
    }

    #[test]
    fn rog_replacement_identity_retires_the_old_device_even_without_a_battery_reply() {
        let mut old = history_owner_device("receiver-a", DeviceFamily::Mouse, true, None);
        old.ident.mouse_sn = "first-mouse".into();
        old.reading.battery = snapshot(BackendId::Rog, "receiver-a", 47).battery;
        old.battery_at = Some(std::time::Instant::now());
        old.state = Some(proto::State::default());
        let independent = history_owner_device("receiver-b", DeviceFamily::Mouse, true, None);
        let link = old.link.clone().unwrap();
        let mut records = BTreeMap::from([("first".into(), old), ("other".into(), independent)]);
        let mut outcome = rog_energy_outcome();
        let snapshot = Arc::make_mut(&mut outcome.snapshot);
        snapshot.identity.mouse_sn = "replacement-mouse".into();
        snapshot.frames.remove("12:07:00");
        assert!(reconcile_rog_identity(&mut records, &link, &snapshot.identity).is_none());
        assert!(rog_record(&link, &outcome, None).is_none());
        assert!(!records["first"].reading.connected);
        assert!(records["first"].state.is_none());
        assert_eq!(records["first"].reading.quality, crate::device::ReadingQuality::Stale);
        assert!(records["other"].reading.connected);
    }

    #[test]
    fn rog_serial_promotion_and_path_change_preserve_one_identity_and_preferences() {
        let mut previous = history_owner_device("receiver-a", DeviceFamily::Mouse, true, None);
        previous.ident.mouse_mac = "12:34:56:78:9A:BC".into();
        let mut link = previous.link.clone().unwrap();
        previous.reading.key = crate::backends::rog::identity_key(&link, &previous.ident);
        let old_key = previous.reading.key.clone();
        let mut outcome = rog_energy_outcome();
        let identity = &mut Arc::make_mut(&mut outcome.snapshot).identity;
        identity.mouse_mac = previous.ident.mouse_mac.clone();
        identity.mouse_sn = "confirmed-serial".into();
        let promoted = rog_record(&link, &outcome, Some(&previous)).unwrap();
        assert_ne!(promoted.reading.key, old_key);
        let mut shared = Shared::new(Settings::default());
        shared.active_key = Some(old_key.clone());
        shared.cfg.device_names.insert(old_key.clone(), "My device".into());
        shared.promote_device_key(&old_key, &promoted.reading.key);
        assert_eq!(shared.active_key.as_ref(), Some(&promoted.reading.key));
        assert_eq!(shared.cfg.device_names[&promoted.reading.key], "My device");
        let mut records = BTreeMap::from([(promoted.reading.key.clone(), promoted)]);
        link.iface.path = "receiver-c".into();
        let previous = reconcile_rog_identity(&mut records, &link, &outcome.snapshot.identity).unwrap();
        let reconnected = rog_record(&link, &outcome, Some(&previous)).unwrap();
        records.insert(reconnected.reading.key.clone(), reconnected);
        assert_eq!(records.len(), 1);
        assert_eq!(shared.active_key.as_ref(), records.keys().next());
        assert_eq!(records.values().next().unwrap().link.as_ref().unwrap().iface.path, "receiver-c");
    }

    #[test]
    fn rog_known_identity_retires_anonymous_channel_records_without_migrating_their_history() {
        let mut anonymous = history_owner_device("receiver", DeviceFamily::Mouse, true, None);
        let link = anonymous.link.clone().unwrap();
        anonymous.reading.key = link.device_key();
        let anonymous_key = anonymous.reading.key.clone();
        anonymous.reading.battery = snapshot(BackendId::Rog, "receiver", 47).battery;
        let mut known = history_owner_device("wired", DeviceFamily::Mouse, true, None);
        known.ident.mouse_sn = "known-device".into();
        known.reading.key = crate::backends::rog::identity_key(known.link.as_ref().unwrap(), &known.ident);
        let known_key = known.reading.key.clone();
        let mut shared = Shared::new(Settings::default());
        shared.cfg.device_names.insert(anonymous_key.clone(), "Unconfirmed device".into());
        shared.cfg.device_names.insert(known_key.clone(), "Known device".into());
        let mut records = BTreeMap::from([(anonymous_key.clone(), anonymous), (known_key.clone(), known)]);
        let mut outcome = rog_energy_outcome();
        Arc::make_mut(&mut outcome.snapshot).identity.mouse_sn = "known-device".into();
        let previous = reconcile_rog_identity(&mut records, &link, &outcome.snapshot.identity);
        let record = rog_record(&link, &outcome, previous.as_ref()).unwrap();
        records.insert(record.reading.key.clone(), record);
        shared.replace_rog_records(records);
        assert_eq!(shared.devices.len(), 1);
        assert!(shared.devices[&known_key].reading.connected);
        assert!(!shared.devices.contains_key(&anonymous_key));
        assert!(shared.key_aliases.is_empty());
        assert_eq!(shared.cfg.device_names[&known_key], "Known device");
        assert_eq!(shared.cfg.device_names[&anonymous_key], "Unconfirmed device");
    }

    #[test]
    fn rog_stalled_reader_expires_battery_without_disconnecting_other_devices() {
        let mut shared = Shared::new(Settings::default());
        let now = std::time::Instant::now();
        let mut device = history_owner_device("rog", DeviceFamily::Mouse, true, None);
        device.reading.battery = snapshot(BackendId::Rog, "rog", 70).battery;
        device.battery_at = Some(now);
        device.state = Some(proto::State::default());
        shared.full = device.state.clone();
        shared.primary_key = Some("rog".into());
        shared.devices.insert("rog".into(), device.clone());
        shared.expire_readings_at(now + rog_sample_lifetime(&shared.cfg) - Duration::from_secs(1));
        assert_eq!(shared.devices["rog"].reading.percent(), Some(70));
        let expired = now + rog_sample_lifetime(&shared.cfg);
        let mut other = snapshot(BackendId::Logitech, "other", 80);
        other.battery_at = Some(expired);
        merge_additional_at(&mut shared, BackendId::Logitech, vec![other].into(), expired);
        shared.expire_readings_at(expired);
        assert!(shared.devices["rog"].reading.connected);
        assert_eq!(shared.devices["rog"].reading.quality, crate::device::ReadingQuality::Stale);
        assert_eq!(shared.devices["rog"].reading.percent(), None);
        assert!(shared.devices["rog"].state.is_none());
        assert!(shared.full.is_none());
        assert_eq!(shared.devices["other"].reading.percent(), Some(80));
        device.battery_at = Some(expired + Duration::from_secs(1));
        shared.replace_rog_records(BTreeMap::from([("rog".into(), device)]));
        assert_eq!(shared.devices["rog"].reading.percent(), Some(70));
        assert_eq!(shared.devices["other"].reading.percent(), Some(80));
    }

    fn dormant_store_updates() -> StoreUpdates {
        StoreUpdates {
            controller: crate::store::Controller::for_test(),
            preparation: RefCell::new(None),
            writes_blocked: Cell::new(false),
            ticking: Cell::new(false),
        }
    }

    fn preparation(action: StorePreparationAction) -> StorePreparation {
        StorePreparation {
            action, started: std::time::Instant::now(), worker: None, result: None, canceled: false,
        }
    }

    #[test]
    fn store_preparation_does_not_stop_monitoring_workers() {
        let shared = Arc::new(Mutex::new(Shared::new(Settings::default())));
        let signal = Signal::new(60);
        lock_shared(&shared).store_update_pending = true;
        assert!(!submit(&shared, &signal, Cmd::Stage(0)));
        assert!(!signal.shutdown.load(Ordering::Relaxed));
        assert!(lock_shared(&shared).pending.is_empty());
    }

    #[test]
    fn restart_retry_waits_for_device_writes_and_remains_cancelable() {
        let shared = Arc::new(Mutex::new(Shared::new(Settings::default())));
        let signal = Signal::new(60);
        let updates = StoreUpdates {
            controller: crate::store::Controller::for_test_phase(crate::store::Phase::RestartFailed),
            ..dormant_store_updates()
        };
        lock_shared(&shared).pending.push_back((Cmd::Stage(0), None));
        assert!(updates.begin(StorePreparationAction::Restart, &shared));
        assert!(lock_shared(&shared).store_update_pending);
        assert!(updates.tick(&shared, &signal).1.is_none());
        assert!(updates.preparation.borrow().as_ref().unwrap().worker.is_none());
        assert_eq!(lock_shared(&shared).pending.len(), 1);
        assert!(!signal.shutdown.load(Ordering::Relaxed));
        updates.cancel();
        updates.tick(&shared, &signal);
        assert_eq!(updates.view().phase, crate::store::Phase::RestartFailed);
        assert!(!updates.busy());
        assert!(!lock_shared(&shared).store_update_pending);
        assert!(!signal.shutdown.load(Ordering::Relaxed));
    }

    #[test]
    fn store_preparation_waits_for_queued_and_in_flight_device_writes() {
        let mut state = Shared::new(Settings::default());
        assert!(!state.writes_pending());
        state.rog_write_active = true;
        assert!(state.writes_pending());
        state.rog_write_active = false;
        state.pending.push_back((Cmd::Stage(0), None));
        assert!(state.writes_pending());
        state.pending.clear();
        state.source_busy.insert(BackendId::CompX);
        assert!(state.writes_pending());
        state.source_busy.clear();
        state.source_pending.insert(BackendId::CompX, VecDeque::from([Cmd::Stage(0)]));
        assert!(state.writes_pending());
        state.source_pending.clear();
        assert!(!state.writes_pending());
    }

    #[test]
    fn canceling_store_preparation_restores_commands_without_setting_shutdown() {
        let shared = Arc::new(Mutex::new(Shared::new(Settings::default())));
        let signal = Signal::new(60);
        let updates = dormant_store_updates();
        lock_shared(&shared).store_update_pending = true;
        updates.writes_blocked.set(true);
        updates.preparation.replace(Some(preparation(StorePreparationAction::ExitToStore)));
        updates.cancel();
        assert!(updates.tick(&shared, &signal).1.is_none());
        assert!(!updates.busy());
        assert!(!lock_shared(&shared).store_update_pending);
        assert!(!signal.shutdown.load(Ordering::Relaxed));
        assert!(submit(&shared, &signal, Cmd::Stage(0)));
    }

    #[test]
    fn failed_checkpoint_keeps_the_app_running_and_restores_writes() {
        let shared = Arc::new(Mutex::new(Shared::new(Settings::default())));
        let signal = Signal::new(60);
        let updates = dormant_store_updates();
        let mut job = preparation(StorePreparationAction::Install);
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(Err("save failed".into())).unwrap();
        job.worker = Some(receiver);
        updates.preparation.replace(Some(job));
        updates.writes_blocked.set(true);
        lock_shared(&shared).store_update_pending = true;
        assert_eq!(updates.tick(&shared, &signal).1.as_deref(), Some("save failed"));
        assert!(!updates.busy());
        assert!(!lock_shared(&shared).store_update_pending);
        assert!(!signal.shutdown.load(Ordering::Relaxed));
    }

    #[test]
    fn accepted_update_continues_after_a_successful_checkpoint() {
        let mut job = preparation(StorePreparationAction::Install);
        job.result = Some(Ok(()));
        assert!(job.can_finish());
        job.canceled = true;
        assert!(job.can_finish());
    }

    #[test]
    fn canceled_checkpoint_worker_finishes_before_writes_resume() {
        let mut job = preparation(StorePreparationAction::Install);
        let (_sender, receiver) = std::sync::mpsc::channel();
        job.worker = Some(receiver);
        job.canceled = true;
        assert!(!job.can_finish());
        job.result = Some(Ok(()));
        assert!(job.can_finish());
    }

    #[test]
    fn store_write_wait_timeout_does_not_discard_commands_or_stop_workers() {
        let shared = Arc::new(Mutex::new(Shared::new(Settings::default())));
        let signal = Signal::new(60);
        let updates = dormant_store_updates();
        let mut job = preparation(StorePreparationAction::Install);
        job.started -= Duration::from_secs(31);
        updates.preparation.replace(Some(job));
        updates.writes_blocked.set(true);
        let mut state = lock_shared(&shared);
        state.store_update_pending = true;
        state.pending.push_back((Cmd::Stage(0), None));
        drop(state);
        assert!(updates.tick(&shared, &signal).1.is_some());
        assert_eq!(lock_shared(&shared).pending.len(), 1);
        assert!(!lock_shared(&shared).store_update_pending);
        assert!(!signal.shutdown.load(Ordering::Relaxed));
    }

    #[test]
    fn store_installation_overrides_close_to_tray_and_close_confirmation() {
        let updates = dormant_store_updates();
        updates.preparation.replace(Some(preparation(StorePreparationAction::Install)));
        for mode in ["ask", "minimize", "quit"] {
            assert_eq!(updates.close_action(mode, true, true), WindowCloseAction::Quit);
        }
        updates.preparation.replace(None);
        assert_eq!(updates.close_action("minimize", true, true), WindowCloseAction::Hide);
        assert_eq!(updates.close_action("ask", true, true), WindowCloseAction::Ask);
    }

    #[test]
    fn close_to_background_keeps_the_window_reachable_with_either_indicator() {
        for (tray, taskbar) in [(true, false), (false, true), (true, true)] {
            assert_eq!(window_close_action("minimize", tray, taskbar), WindowCloseAction::Hide);
            assert_eq!(window_close_action("ask", tray, taskbar), WindowCloseAction::Ask);
            assert_eq!(window_close_action("quit", tray, taskbar), WindowCloseAction::Quit);
        }
        assert_eq!(window_close_action("minimize", false, false), WindowCloseAction::Minimize);
        assert_eq!(window_close_action("quit", false, false), WindowCloseAction::Quit);
    }

    #[test]
    fn feedback_candidates_read_existing_shared_samples_and_configuration_without_refreshing_hardware() {
        let reading = DeviceReading { key: "feedback-device".into(), name: "Logitech G304".into(),
            family: DeviceFamily::Mouse, connected: true, battery: Some(Battery { percent: Some(25),
                estimated_percent: None, charging: Some(false), voltage_mv: None, approximate: false, level: None }),
            ..Default::default() };
        let mut shared = Shared::new(Settings::default());
        shared.replace_mock_readings(vec![reading]);
        let before = shared.refresh_generation;
        let current = feedback_candidates(&shared);
        assert_eq!(current.len(), 1);
        assert!(current[0].diagnostics.contains("Battery: 25%"));
        assert!(current[0].diagnostics.contains("Current DPI: 800"));
        assert!(current[0].diagnostics.contains("Polling rate: 1000 Hz"));
        assert!(current[0].diagnostics.contains("Captured at (Unix ms):"));
        shared.readings[0].battery.as_mut().unwrap().percent = Some(35);
        shared.readings[0].battery.as_mut().unwrap().charging = Some(true);
        let updated = feedback_candidates(&shared);
        assert!(updated[0].same_source(&current[0]));
        assert!(updated[0].diagnostics.contains("Battery: 35%"));
        assert!(updated[0].diagnostics.contains("Charging: yes"));
        let record = shared.devices.get_mut("feedback-device").unwrap();
        record.basic = None;
        record.state = Some(proto::State { active_stage: 2, poll_code: 2,
            enc: [proto::encode_dpi(400), proto::encode_dpi(800), proto::encode_dpi(1600), proto::encode_dpi(3200)],
            ..Default::default() });
        let rog = feedback_candidates(&shared);
        assert!(rog[0].diagnostics.contains("Current DPI: 800"));
        assert!(rog[0].diagnostics.contains("Polling rate: 500 Hz"));
        shared.devices.get_mut("feedback-device").unwrap().state = None;
        let unknown = feedback_candidates(&shared);
        assert!(unknown[0].diagnostics.contains("Current DPI: unknown"));
        assert!(unknown[0].diagnostics.contains("Polling rate: unknown"));
        assert_eq!(shared.refresh_generation, before);
        assert!(shared.pending.is_empty());
        assert!(shared.source_pending.values().all(VecDeque::is_empty));
    }

    #[test]
    fn mock_mouse_settings_survive_battery_refresh_without_changing_other_devices() {
        let mouse = DeviceReading {
            key: "mock-mouse".into(), name: "Logitech G304".into(),
            family: DeviceFamily::Mouse, connected: true, ..Default::default()
        };
        let other = DeviceReading {
            key: "mock-other".into(), name: "Razer mouse".into(),
            family: DeviceFamily::Mouse, connected: true, ..Default::default()
        };
        let mut shared = Shared::new(Settings::default());
        shared.replace_mock_readings(vec![mouse.clone(), other.clone()]);
        let target = shared.devices["mock-mouse"].basic.as_ref().unwrap().target.clone();
        apply_mock_cmd(&mut shared, &Cmd::BasicDpi { target: target.clone(), dpi: 2400 }, None);
        apply_mock_cmd(&mut shared, &Cmd::BasicPollingRate { target, hz: 500 }, None);
        shared.replace_mock_readings(vec![mouse, other]);
        let basic = shared.devices["mock-mouse"].basic.as_ref().unwrap();
        assert_eq!(basic.dpi.current, Some(2400));
        assert_eq!(basic.dpi.current_y, Some(2400));
        assert_eq!(basic.polling_rate.current, Some(500));
        let untouched = shared.devices["mock-other"].basic.as_ref().unwrap();
        assert_eq!(untouched.dpi.current, Some(800));
        assert_eq!(untouched.polling_rate.current, Some(1000));
    }

    #[test]
    fn mock_rog_mouse_keeps_its_hardware_controls_after_refresh() {
        let mouse = DeviceReading {
            key: "mock-rog".into(), name: "ROG Keris".into(),
            family: DeviceFamily::Mouse, connected: true, ..Default::default()
        };
        let mut shared = Shared::new(Settings::default());
        shared.replace_mock_readings(vec![mouse.clone()]);
        assert!(shared.devices["mock-rog"].basic.is_none());
        assert!(shared.devices["mock-rog"].state.is_some());
        apply_mock_cmd(&mut shared, &Cmd::Dpi { stage: 2, dpi: 2400, rgb: None }, None);
        apply_mock_cmd(&mut shared, &Cmd::Poll(500), None);
        shared.replace_mock_readings(vec![mouse]);
        let device_state = shared.devices["mock-rog"].state.as_ref().unwrap();
        assert_eq!(device_state.enc[2], proto::encode_dpi(2400));
        assert_eq!(device_state.poll_code, proto::poll_code_of(500).unwrap());
    }

    #[test]
    fn queued_device_reorder_cannot_reorder_a_changed_list() {
        let displayed = vec!["mouse".into(), "keyboard".into(), "headset".into()];
        let mut readings: Vec<_> = displayed.iter().map(|key: &String| DeviceReading {
            key: key.clone(), ..Default::default()
        }).collect();
        assert_eq!(reordered_device_keys(&displayed, &readings, 2, 0),
            Some(vec!["headset".into(), "mouse".into(), "keyboard".into()]));
        readings.swap(0, 1);
        assert!(reordered_device_keys(&displayed, &readings, 2, 0).is_none());
        readings.swap(0, 1);
        readings[1].key = "replacement".into();
        assert!(reordered_device_keys(&displayed, &readings, 2, 0).is_none());
        assert!(reordered_device_keys(&displayed, &[], 2, 0).is_none());
    }

    fn snapshot(backend: BackendId, key: &str, percent: u8) -> Snapshot {
        Snapshot { target: Target { key: key.into(), name: "Test mouse".into(), family: DeviceFamily::Mouse,
            backend, address: 1, verified_identity: false, endpoint: crate::backends::Endpoint::Hid(hid::HidInterface {
                path: key.into(), serial: String::new(), product: "Test mouse".into(),
                vendor_id: 0, product_id: 0, usage_page: 1, usage: 2,
                input_len: 20, output_len: 20, feature_len: 0,
            }) }, connected: true, battery_unavailable: false, battery_at: Some(std::time::Instant::now()), polling_rate: Default::default(), dpi: Dpi::default(), battery: Some(Battery { estimated_percent: None, percent: Some(percent),
                charging: Some(false), voltage_mv: None, approximate: false, level: None }) }
    }

    fn history_owner_device(
        key: &str,
        family: DeviceFamily,
        linked: bool,
        basic: Option<Snapshot>,
    ) -> DeviceSnapshot {
        let iface = hid::HidInterface {
            path: key.into(), serial: String::new(), product: "Test mouse".into(),
            vendor_id: 0, product_id: 0, usage_page: 1, usage: 2,
            input_len: 20, output_len: 20, feature_len: 0,
        };
        DeviceSnapshot {
            reading: DeviceReading {
                key: key.into(), name: "Test mouse".into(), family, battery: None,
                connected: true, quality: crate::device::ReadingQuality::Live,
            },
            ident: Default::default(),
            state: None,
            link: linked.then(|| hid::Link {
                mode: "Wireless24G".into(), report_id: 0, read_only: false, iface,
                all_interfaces: Vec::new(),
            }),
            battery_at: None,
            full_at: std::time::Instant::now(),
            basic,
            basic_status: None,
        }
    }

    #[test]
    fn legacy_rog_history_ignores_bluetooth_mice_with_link_metadata() {
        let bluetooth = history_owner_device(
            "bluetooth",
            DeviceFamily::Mouse,
            true,
            Some(snapshot(BackendId::Bluetooth, "bluetooth", 80)),
        );
        let mut devices = std::collections::BTreeMap::from([("bluetooth".into(), bluetooth)]);
        assert_eq!(legacy_rog_history_owner(&devices), None);

        devices.insert(
            "rog".into(),
            history_owner_device("rog", DeviceFamily::Mouse, true, None),
        );
        assert_eq!(legacy_rog_history_owner(&devices).as_deref(), Some("rog"));

        devices.insert(
            "rog-two".into(),
            history_owner_device("rog-two", DeviceFamily::Mouse, true, None),
        );
        assert_eq!(legacy_rog_history_owner(&devices), None);
    }

    #[test]
    fn history_collects_each_device_family_without_republishing_cached_samples() {
        let mut shared = Shared::new(Settings::default());
        let at = std::time::Instant::now();
        for (backend, key, family) in [
            (BackendId::Rapoo, "mouse", DeviceFamily::Mouse),
            (BackendId::MchoseK87s, "keyboard", DeviceFamily::Keyboard),
            (BackendId::Moondrop, "headset", DeviceFamily::Headset),
            (BackendId::XInput, "gamepad", DeviceFamily::Gamepad),
        ] {
            let mut sample = snapshot(backend, key, 70);
            sample.target.family = family; sample.battery_at = Some(at);
            shared.publish_additional(backend, vec![sample.clone()], 0, 1);
            let (rows, _) = shared.history_tracker.take_pending();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].0, key);
            assert_eq!(rows[0].1.level, 70);
            shared.publish_additional(backend, vec![sample], 0, 2);
            assert!(shared.history_tracker.take_pending().0.is_empty());
        }
        assert_eq!(shared.devices.len(), 4);
    }

    #[test]
    fn history_alias_on_first_verified_report_survives_a_path_reload() {
        let mut shared = Shared::new(Settings::default());
        let mut sample = snapshot(BackendId::Logitech, "channel", 70);
        let old = sample.target.unverified_key();
        sample.target.key = "verified-device".into(); sample.target.verified_identity = true;
        shared.publish_additional(BackendId::Logitech, vec![sample.clone()], 0, 1);
        shared.reload_history();
        let (rows, aliases) = shared.history_tracker.take_pending();
        assert_eq!(rows[0].0, "verified-device");
        assert!(aliases.contains(&(old, "verified-device".into())));
        sample.target.key = "replacement".into();
        shared.publish_additional(BackendId::Logitech, vec![sample], 0, 2);
        let (_, aliases) = shared.history_tracker.take_pending();
        assert!(!aliases.iter().any(|(from, to)| from == "verified-device" && to == "replacement"));
    }

    #[test]
    fn history_clear_drops_queued_samples_and_does_not_reuse_the_current_measurement() {
        let mut shared = Shared::new(Settings::default());
        let sample = snapshot(BackendId::Rapoo, "mouse", 70);
        shared.publish_additional(BackendId::Rapoo, vec![sample.clone()], 0, 1);
        shared.reset_history();
        shared.publish_additional(BackendId::Rapoo, vec![sample], 0, 2);
        assert!(shared.history_tracker.take_pending().0.is_empty());
    }

    #[test]
    fn taskbar_strip_lists_every_connected_device_with_battery_in_display_order() {
        let reading = |key: &str, family: DeviceFamily, percent: Option<u8>,
            connected: bool| DeviceReading {
            key: key.into(),
            name: key.into(),
            family,
            battery: percent.map(|percent| Battery {
                percent: Some(percent),
                estimated_percent: None,
                charging: Some(key == "mouse"),
                voltage_mv: None,
                approximate: false,
                level: None,
            }),
            connected,
            quality: crate::device::ReadingQuality::Live,
        };
        let readings = vec![
            reading("mouse", DeviceFamily::Mouse, Some(81), true),
            reading("keyboard", DeviceFamily::Keyboard, Some(64), true),
            reading("offline", DeviceFamily::Headset, Some(50), false),
        ];
        let mut cfg = Settings::default();
        cfg.device_tray
            .entry("keyboard".into())
            .or_default()
            .kind = crate::tray_prefs::TrayKind::Headset;
        let strip = taskbar_strip_content(&readings, &cfg).unwrap();
        assert_eq!(strip.items.len(), 2);
        assert_eq!(strip.items[0].level.map(|level| level.value), Some(81));
        assert!(strip.items[0].charging);
        assert_eq!(strip.items[0].kind, crate::tray_prefs::TrayKind::Mouse);
        assert_eq!(strip.items[1].level.map(|level| level.value), Some(64));
        assert!(!strip.items[1].charging);
        assert_eq!(strip.items[1].kind, crate::tray_prefs::TrayKind::Headset);
        let empty = taskbar_strip_content(&readings[2..], &cfg).unwrap();
        assert!(empty.items.is_empty());
        assert_eq!(empty.empty_label.as_deref(), Some(crate::tr!("暂无设备")));
    }

    #[test]
    fn rog_refresh_preserves_concurrent_additional_reading() {
        let mut shared = Shared::new(Settings::default());
        shared.publish_additional(BackendId::Logitech, vec![snapshot(BackendId::Logitech, "logitech", 75)], 0, 1);
        shared.replace_rog_records(BTreeMap::new());
        assert_eq!(shared.readings.len(), 1);
        assert_eq!(shared.readings[0].percent(), Some(75));
    }

    #[test]
    fn compx_stage_edit_and_rate_share_their_backend_queue_and_stale_poll_guard() {
        let shared=Arc::new(Mutex::new(Shared::new(Settings::default())));let signal=Signal::new(60);
        let target=snapshot(BackendId::CompX,"compx",75).target;
        let stage=crate::device::DpiStage {index:1,value:1600};
        assert!(submit(&shared,&signal,Cmd::BasicDpiEdit {target:target.clone(),stage,context:crate::device::StageContext::Profile(0),expected_y:1600,x:800,y:1600}));
        assert!(submit(&shared,&signal,Cmd::BasicPollingRate {target,hz:500}));
        let mut state=lock_shared(&shared);
        assert_eq!(state.source_pending[&BackendId::CompX].len(),2);
        assert_eq!(state.source_write_gen[&BackendId::CompX],2);
        assert!(!state.publish_additional(BackendId::CompX,vec![snapshot(BackendId::CompX,"compx",75)],1,1));
        assert!(state.publish_additional(BackendId::Rapoo,vec![snapshot(BackendId::Rapoo,"rapoo",70)],0,1));
    }
    #[test]
    fn dpi_draft_remembers_device_profile_and_both_original_axes() {
        let mut reading=snapshot(BackendId::CompX,"compx",75);
        reading.dpi.profile=Some(0);
        reading.dpi.stages=vec![crate::device::DpiStage {index:1,value:800}];
        reading.dpi.stage_y.insert(1,1600);
        let original=crate::ui::widget::BasicDpiDraft::from_snapshot(&reading,1).unwrap();
        reading.dpi.profile=Some(1);
        assert_ne!(Some(original.clone()),crate::ui::widget::BasicDpiDraft::from_snapshot(&reading,1));
        reading.dpi.profile=Some(0);reading.dpi.stage_y.insert(1,800);
        assert_ne!(Some(original.clone()),crate::ui::widget::BasicDpiDraft::from_snapshot(&reading,1));
        reading.dpi.stage_y.insert(1,1600);reading.target.key="replacement".into();
        assert_ne!(Some(original),crate::ui::widget::BasicDpiDraft::from_snapshot(&reading,1));
    }
    #[test]
    fn tray_custom_presets_keep_order_disable_invalid_values_and_never_fake_readback() {
        let mut shared = Shared::new(Settings::default());
        let mut reading = snapshot(BackendId::Logitech,"preset-mouse",75);
        reading.dpi = Dpi { current:Some(800), current_y:Some(800),values:vec![400,800,1600],
            writable:true,write_effect:Some(crate::device::DpiWriteEffect::Current),..Default::default() };
        shared.cfg.dpi_presets.insert("preset-mouse".into(),crate::device::DpiPresets {
            enabled:true,values:vec![800,1200,1600], ..Default::default() });
        shared.publish_additional(BackendId::Logitech,vec![reading.clone()],0,1);
        let popup = popup_dpi_states(&shared);
        let p = &popup["preset-mouse"];
        assert!(p.custom && p.value_control && p.writable);
        assert_eq!(p.stages.iter().map(|s|s.value).collect::<Vec<_>>(),vec![800,1200,1600]);
        assert_eq!(p.active,Some(0)); assert_eq!(p.disabled,vec![1]);
        reading.dpi.current = Some(1600); reading.dpi.current_y = Some(800);
        shared.publish_additional(BackendId::Logitech,vec![reading],0,2);
        assert_eq!(popup_dpi_states(&shared)["preset-mouse"].active,None);
        shared.publish_additional(BackendId::Logitech,crate::backends::Poll::default(),0,3);
        assert!(!popup_dpi_states(&shared)["preset-mouse"].writable);
    }
    #[test]
    fn stored_preset_command_binds_original_stage_and_rejects_deleted_or_disabled_presets() {
        let mut reading=snapshot(BackendId::CompX,"stored",75);
        let stage=crate::device::DpiStage {index:2,value:800};
        reading.dpi=Dpi {current:Some(800),current_y:Some(800),values:vec![800,1600],writable:true,
            stage_editable:true,write_effect:Some(crate::device::DpiWriteEffect::Stored),stages:vec![stage],active_stage:Some(2),
            profile:Some(1),stage_y:[(2,800)].into(),..Default::default()};
        let mut cfg=Settings::default();
        cfg.dpi_presets.insert("stored".into(),crate::device::DpiPresets {enabled:true,values:vec![1600], ..Default::default()});
        assert!(matches!(dpi_preset_command(&cfg,&reading,1600),Some(Cmd::BasicDpiPreset {
            stage:s,context:crate::device::StageContext::Profile(1),expected_y:800,value:1600,.. }) if s == stage));
        assert!(dpi_preset_command(&cfg,&reading,800).is_none());
        cfg.dpi_presets.get_mut("stored").unwrap().enabled=false;
        assert!(dpi_preset_command(&cfg,&reading,1600).is_none());
        cfg.dpi_presets.get_mut("stored").unwrap().enabled=true;
        reading.dpi.disable_writes();
        assert!(dpi_preset_command(&cfg,&reading,1600).is_none());
    }
    #[test]
    fn razer_config_context_reaches_main_window_tray_and_its_own_worker() {
        let mut reading=snapshot(BackendId::Razer,"razer",75);
        let stage=crate::device::DpiStage {index:1,value:800};
        reading.dpi.configuration_token=Some(0x1234);reading.dpi.stages=vec![stage];reading.dpi.stage_y.insert(1,1600);
        reading.dpi.stage_writable=true;reading.dpi.stage_editable=true;
        let draft=crate::ui::widget::BasicDpiDraft::from_snapshot(&reading,1).unwrap();
        assert_eq!(draft.context,crate::device::StageContext::Configuration(0x1234));
        let mut state=Shared::new(Settings::default());state.publish_additional(BackendId::Razer,vec![reading.clone()],0,1);
        let popup=popup_dpi_states(&state);assert_eq!(popup["razer"].context,Some(draft.context));assert!(popup["razer"].writable);
        reading.dpi.configuration_token=Some(0x5678);
        assert_ne!(Some(draft.clone()),crate::ui::widget::BasicDpiDraft::from_snapshot(&reading,1));
        let shared=Arc::new(Mutex::new(state));let signal=Signal::new(60);
        assert!(submit(&shared,&signal,Cmd::BasicDpiStage {target:reading.target.clone(),stage,context:draft.context}));
        assert!(submit(&shared,&signal,Cmd::BasicDpiEdit {target:reading.target,stage,context:draft.context,expected_y:1600,x:1600,y:3200}));
        let state=lock_shared(&shared);assert_eq!(state.source_pending[&BackendId::Razer].len(),2);
        assert_eq!(state.source_write_gen[&BackendId::Razer],2);
    }
    #[test]
    fn razer_first_serial_identity_promotes_preferences_from_the_discovery_key() {
        let mut first=snapshot(BackendId::Razer,"temporary",75);
        first.target=Target::new(BackendId::Razer,first.target.hid().unwrap(),0);
        let discovery_key=first.target.key.clone();
        let mut shared=Shared::new(Settings::default());
        shared.cfg.device_names.insert(discovery_key.clone(),"My mouse".into());
        shared.publish_additional(BackendId::Razer,vec![first.clone()],0,1);
        first.target.key="confirmed-unit".into();first.target.verified_identity=true;
        shared.publish_additional(BackendId::Razer,vec![first],0,2);
        assert!(!shared.devices.contains_key(&discovery_key));
        assert_eq!(shared.cfg.device_names.get("confirmed-unit").map(String::as_str),Some("My mouse"));
    }
    #[test]
    fn richer_hid_reading_replaces_identity_verified_bluetooth_and_preserves_preferences() {
        let address = ["AA", "BB", "01", "02", "03", "04"].concat();
        let bluetooth_interface = hid::HidInterface {
            path: format!("bluetooth:{address}"),
            serial: address.clone(),
            product: "Example Headphones".into(),
            vendor_id: 0,
            product_id: 0,
            usage_page: 0,
            usage: 0,
            input_len: 0,
            output_len: 0,
            feature_len: 0,
        };
        let standard_interface = hid::HidInterface {
            path: format!("\\\\?\\HID#BTHENUM#7&{address}&0&0000#{{hid}}"),
            serial: String::new(),
            product: "Example Headphones".into(),
            vendor_id: 0x1234,
            product_id: 0x5678,
            usage_page: 1,
            usage: 2,
            input_len: 8,
            output_len: 0,
            feature_len: 8,
        };
        let mut standard = snapshot(BackendId::StandardHid, "standard", 70);
        standard.target = Target::new(BackendId::StandardHid, &standard_interface, 0);
        let mut bluetooth = snapshot(BackendId::Bluetooth, "bluetooth", 71);
        bluetooth.target = Target::new(BackendId::Bluetooth, &bluetooth_interface, 0);
        let standard_key = standard.target.key.clone();
        let bluetooth_key = bluetooth.target.key.clone();

        let mut shared = Shared::new(Settings::default());
        shared.cfg.device_names.insert(standard_key.clone(), "My headphones".into());
        shared.cfg.device_order = vec![standard_key.clone()];
        shared.cfg.tray_device = Some(standard_key.clone());
        let now = std::time::Instant::now();
        merge_additional_at(&mut shared, BackendId::StandardHid, vec![standard.clone()].into(), now);
        merge_additional_at(&mut shared, BackendId::Bluetooth, vec![bluetooth.clone()].into(), now + Duration::from_secs(1));

        assert_eq!(shared.devices.len(), 1);
        assert!(shared.devices.contains_key(&standard_key));
        shared
            .devices
            .get_mut(&standard_key)
            .unwrap()
            .basic
            .as_mut()
            .unwrap()
            .connected = false;
        merge_additional_at(
            &mut shared,
            BackendId::Bluetooth,
            vec![bluetooth.clone()].into(),
            now + Duration::from_secs(2),
        );
        assert!(shared.devices.contains_key(&bluetooth_key));

        let mut shared = Shared::new(Settings::default());
        shared.cfg.device_names.insert(bluetooth_key.clone(), "My headphones".into());
        shared.cfg.device_order = vec![bluetooth_key.clone()];
        shared.cfg.tray_device = Some(bluetooth_key.clone());
        merge_additional_at(&mut shared, BackendId::Bluetooth, vec![bluetooth].into(), now);
        merge_additional_at(&mut shared, BackendId::StandardHid, vec![standard].into(), now + Duration::from_secs(1));
        assert_eq!(shared.devices.len(), 1);
        assert!(shared.devices.contains_key(&standard_key));
        assert_eq!(shared.cfg.device_names.get(&standard_key).map(String::as_str), Some("My headphones"));
        assert_eq!(shared.cfg.device_order, vec![standard_key.clone()]);
        assert_eq!(shared.cfg.tray_device.as_deref(), Some(standard_key.as_str()));
    }
    #[test]
    fn razer_sibling_fallback_preserves_preferences_and_latest_receiver_identity() {
        let mut first = snapshot(BackendId::Razer, "temporary", 75);
        first.target.hid_mut().path = "hid#vid_1532&pid_00c1&mi_00&col01#7&aaaa&0&0001#{hid}".into();
        first.target = Target::new(BackendId::Razer, first.target.hid().unwrap(), 0);
        let discovery_key = first.target.key.clone();
        let mut shared = Shared::new(Settings::default());
        let now = std::time::Instant::now();
        shared.cfg.device_names.insert(discovery_key.clone(), "Custom mouse".into());
        merge_additional_at(&mut shared, BackendId::Razer, vec![first.clone()].into(), now);
        first.target.hid_mut().path = first.target.hid().unwrap().path.replace("col01", "col02").replace("0001#", "0002#");
        first.target.key = "a-first-serial".into();
        first.target.verified_identity = true;
        merge_additional_at(&mut shared, BackendId::Razer, vec![first.clone()].into(), now + Duration::from_secs(1));
        assert_eq!(shared.devices.len(), 1);
        assert_eq!(shared.cfg.device_names["a-first-serial"], "Custom mouse");
        first.target.key = "z-replacement-serial".into();
        merge_additional_at(&mut shared, BackendId::Razer, vec![first.clone()].into(), now + Duration::from_secs(2));
        assert!(!shared.devices["a-first-serial"].reading.connected);
        first.target = Target::new(BackendId::Razer, first.target.hid().unwrap(), 0);
        first.target.hid_mut().path = first.target.hid().unwrap().path.replace("col02", "col01").replace("0002#", "0001#");
        first.dpi.writable = true;
        merge_additional_at(&mut shared, BackendId::Razer, vec![first].into(), now + Duration::from_secs(3));
        assert_eq!(shared.devices.len(), 2);
        assert!(!shared.devices["a-first-serial"].reading.connected);
        assert_ne!(shared.devices["z-replacement-serial"].reading.quality, crate::device::ReadingQuality::Live);
        assert!(!shared.devices["z-replacement-serial"].basic.as_ref().unwrap().dpi.writable);
        assert!(!shared.cfg.device_names.contains_key("z-replacement-serial"));
    }
    #[test]
    fn incremental_identity_loss_revokes_writes_without_renewing_the_previous_sample() {
        for backend in [BackendId::Logitech, BackendId::Razer] {
            for publication in [Publication::Battery, Publication::Detail] {
                for same_key in [false, true] {
                    let mut known = snapshot(backend, "known", 75);
                    if backend == BackendId::Razer {
                        known.target.hid_mut().path = "hid#vid_1532&pid_00c1&mi_00&col01#7&aaaa&0&0001#{hid}".into();
                    }
                    known.target.verified_identity = true;
                    known.dpi = Dpi { current: Some(800), writable: true, stage_writable: true,
                        stage_editable: true, ..Default::default() };
                    known.polling_rate = crate::device::PollingRate { current: Some(1000),
                        writable: true, ..Default::default() };
                    let mut other = known.clone();
                    other.target.key = "other".into();
                    other.target.hid_mut().path = "other-interface".into();
                    let mut shared = Shared::new(Settings::default());
                    let now = std::time::Instant::now();
                    known.battery_at = Some(now);
                    merge_additional_at(&mut shared, backend, vec![known.clone(), other.clone()].into(), now);
                    let before = shared.devices[&known.target.key].clone();
                    let untouched = shared.devices[&other.target.key].clone();
                    let mut unknown = known.clone();
                    unknown.target.verified_identity = false;
                    if backend == BackendId::Razer {
                        unknown.target.hid_mut().path = "hid#vid_1532&pid_00c1&mi_00&col02#7&aaaa&0&0002#{hid}".into();
                    }
                    if !same_key { unknown.target.key = unknown.target.unverified_key(); }
                    unknown.battery.as_mut().unwrap().percent = Some(12);
                    unknown.battery_at = Some(now + Duration::from_secs(1));
                    unknown.dpi = Dpi::default();
                    unknown.polling_rate = Default::default();
                    merge_additional_update(&mut shared, backend, vec![unknown].into(),
                        now + Duration::from_secs(1), publication);
                    assert_eq!(shared.devices.len(), 2);
                    let current = &shared.devices[&known.target.key];
                    let basic = current.basic.as_ref().unwrap();
                    assert!(!basic.dpi.writable && !basic.dpi.stage_writable
                        && !basic.dpi.stage_editable && !basic.polling_rate.writable);
                    assert_eq!(basic.dpi.current, Some(800));
                    assert_eq!(basic.battery, known.battery);
                    assert_eq!(basic.battery_at, known.battery_at);
                    assert_eq!(current.reading.percent(), Some(75));
                    assert_eq!(current.reading.quality, crate::device::ReadingQuality::Cached);
                    assert!(current.reading.connected);
                    assert!(!current.basic_status.as_ref().unwrap().connection_known);
                    assert_eq!(current.battery_at, before.battery_at);
                    assert_eq!(current.full_at, before.full_at);
                    assert_eq!(shared.devices[&other.target.key].reading, untouched.reading);
                    assert_eq!(shared.devices[&other.target.key].basic.as_ref().unwrap().dpi, other.dpi);
                    assert!(shared.devices[&other.target.key].basic.as_ref().unwrap().polling_rate.writable);
                }
            }
        }
    }

    #[test]
    fn partial_dpi_read_preserves_independent_live_rate_but_failed_poll_disables_all_writes() {
        let mut shared=Shared::new(Settings::default());
        let mut first=snapshot(BackendId::CompX,"compx",75);
        first.dpi.current=Some(800);first.dpi.stage_editable=true;
        first.polling_rate=crate::device::PollingRate {current:Some(1000),values:vec![500,1000],writable:true,..Default::default()};
        shared.publish_additional(BackendId::CompX,vec![first.clone()],0,1);
        first.dpi=Dpi::default();first.polling_rate.current=Some(500);
        shared.publish_additional(BackendId::CompX,vec![first],0,2);
        let basic=shared.devices["compx"].basic.as_ref().unwrap();
        assert_eq!(basic.dpi.current,Some(800));assert!(!basic.dpi.stage_editable);
        assert_eq!(basic.polling_rate.current,Some(500));assert!(basic.polling_rate.writable);
        shared.publish_additional(BackendId::CompX,crate::backends::Poll::default(),0,3);
        let basic=shared.devices["compx"].basic.as_ref().unwrap();
        assert!(!basic.dpi.stage_editable);assert!(!basic.polling_rate.writable);
    }

    #[test]
    fn failed_poll_keeps_last_battery_and_card_but_never_write_permission() {
        let mut shared = Shared::new(Settings::default());
        let mut first = snapshot(BackendId::Logitech, "logitech", 75);
        first.dpi.writable = true;
        shared.publish_additional(BackendId::Logitech, vec![first], 0, 1);
        let poll = crate::backends::Poll { snapshots: vec![],
            present_channels: Some(["logitech".to_string()].into_iter().collect()), ..Default::default() };
        shared.publish_additional(BackendId::Logitech, poll, 0, 2);
        let device = &shared.devices["logitech"];
        assert!(device.reading.connected);
        assert_eq!(device.reading.percent(), Some(75));
        assert_eq!(device.reading.quality, crate::device::ReadingQuality::Cached);
        assert!(!device.basic.as_ref().unwrap().dpi.writable);
        assert!(!device.basic_status.as_ref().unwrap().connection_known);
        shared.publish_additional(BackendId::Logitech, crate::backends::Poll::default(), 0, 3);
        assert!(shared.devices["logitech"].reading.connected);
        shared.publish_additional(BackendId::Logitech, crate::backends::Poll {
            snapshots: vec![], present_channels: Some(Default::default()), ..Default::default() }, 0, 4);
        assert!(!shared.devices["logitech"].reading.connected);
        assert_eq!(shared.devices["logitech"].reading.battery.as_ref().unwrap().percent, Some(75));
    }

    #[test]
    fn partial_read_and_invalid_battery_do_not_erase_or_refresh_previous_valid_charge() {
        let mut shared = Shared::new(Settings::default());
        let first = snapshot(BackendId::Logitech, "logitech", 75);
        shared.publish_additional(BackendId::Logitech, vec![first.clone()], 0, 1);
        let measured = shared.devices["logitech"].basic_status.as_ref().unwrap().measured_at;
        let mut partial = first.clone(); partial.battery = None;
        shared.publish_additional(BackendId::Logitech, vec![partial], 0, 2);
        assert_eq!(shared.devices["logitech"].reading.percent(), Some(75));
        let mut invalid = first;
        invalid.battery.as_mut().unwrap().percent = Some(255);
        shared.publish_additional(BackendId::Logitech, vec![invalid], 0, 3);
        assert_eq!(shared.devices["logitech"].reading.percent(), Some(75));
        assert_eq!(shared.devices["logitech"].basic_status.as_ref().unwrap().measured_at, measured);
    }

    #[test]
    fn cached_low_battery_does_not_create_or_rearm_a_notification() {
        let mut reading = DeviceReading { connected: true, quality: crate::device::ReadingQuality::Cached,
            battery: snapshot(BackendId::Logitech, "logitech", 10).battery, ..Default::default() };
        let mut armed = true;
        assert_eq!(low_battery_reading(&reading, &mut armed), None);
        assert!(armed);
        reading.quality = crate::device::ReadingQuality::Live;
        assert_eq!(low_battery_reading(&reading, &mut armed), Some(10));
        assert!(!armed);
    }

    #[test]
    fn rapoo_reports_stay_visible_past_global_refresh_interval_and_recover_after_silence() {
        let now = std::time::Instant::now();
        let signal = Signal::new(60);
        let interval_ms = signal.work_interval_ms(BackendId::Rapoo.publish_interval_limit_ms());
        assert_eq!(interval_ms, 2000);
        assert_eq!(signal.work_interval_ms(BackendId::Logitech.publish_interval_limit_ms()), 60000);
        let mut shared = Shared::new(Settings::default());
        let first = snapshot(BackendId::Rapoo, "rapoo", 75);
        let lifetime = BackendId::Rapoo.sample_lifetime().as_secs();
        let pause_end = 90 + lifetime + 14;
        let mut measurement = now;
        for second in 0..=(pause_end + 16) {
            let tick = now + Duration::from_secs(second);
            let reporting = second <= 90 || second >= pause_end + 1;
            if reporting && second % 3 == 0 { measurement = tick; }
            if second * 1000 % interval_ms == 0 {
                let snapshots = if tick.duration_since(measurement) < BackendId::Rapoo.sample_lifetime() {
                    let mut reading = first.clone(); reading.battery_at = Some(measurement);
                    vec![reading]
                } else { Vec::new() };
                merge_additional_at(&mut shared, BackendId::Rapoo, crate::backends::Poll { snapshots,
                    present_channels: Some(["rapoo".into()].into_iter().collect()), ..Default::default() }, tick);
            }
            shared.expire_readings_at(tick);
            let device = &shared.devices["rapoo"];
            assert!(device.reading.connected);
            if second <= 90 + lifetime - 2 || second >= pause_end + 2 {
                assert_eq!(device.reading.percent(), Some(75), "battery lost at second {second}");
            } else if (90 + lifetime + 2..=pause_end).contains(&second) {
                assert_eq!(device.reading.percent(), None);
                assert_eq!(device.reading.battery.as_ref().unwrap().percent, Some(75));
            }
        }
        signal.wake_probe.store(true, Ordering::Relaxed);
        assert_eq!(signal.work_interval_ms(BackendId::Rapoo.publish_interval_limit_ms()), 2000);
        assert_eq!(signal.work_interval_ms(None), WAKE_PROBE_INTERVAL_MS);
    }

    #[test]
    fn gamepad_estimates_refresh_independently_expire_and_recover_in_device_readings() {
        let backend = BackendId::DualSense;
        let now = std::time::Instant::now();
        let signal = Signal::new(60);
        assert_eq!(signal.work_interval_ms(backend.publish_interval_limit_ms()), 2000);
        let mut shared = Shared::new(Settings::default());
        let mut sample = snapshot(backend, "controller", 55);
        sample.target.family = DeviceFamily::Gamepad;
        sample.battery_at = Some(now);
        sample.battery = Some(Battery { percent: None, estimated_percent: Some(55), charging: Some(false),
            voltage_mv: None, approximate: true, level: Some(BatteryLevel::Good) });
        merge_additional_at(&mut shared, backend, vec![sample.clone()].into(), now);
        assert_eq!(shared.readings[0].battery_text(true), "≈55%");
        assert_eq!(shared.readings[0].family, DeviceFamily::Gamepad);
        let mut armed = true;
        assert_eq!(low_battery_reading(&shared.readings[0], &mut armed), None);
        let poll = crate::backends::Poll { snapshots: Vec::new(),
            present_channels: Some(["controller".into()].into_iter().collect()), ..Default::default() };
        merge_additional_at(&mut shared, backend, poll, now + Duration::from_secs(3));
        assert_eq!(shared.readings[0].battery_text(false), "≈55");
        shared.expire_readings_at(now + backend.sample_lifetime());
        assert_eq!(shared.readings[0].battery_text(true), "—");
        sample.battery_at = Some(now + Duration::from_secs(9));
        sample.battery.as_mut().unwrap().estimated_percent = Some(45);
        merge_additional_at(&mut shared, backend, vec![sample].into(), now + Duration::from_secs(9));
        assert_eq!(shared.readings[0].battery_text(true), "≈45%");
        assert!(shared.devices["controller"].basic.as_ref().unwrap().dpi.stages.is_empty());
        merge_additional_at(&mut shared, backend, crate::backends::Poll {
            snapshots: Vec::new(), present_channels: Some(Default::default()),
            ..Default::default()
        }, now + Duration::from_secs(10));
        assert!(!shared.readings[0].connected);
        assert_eq!(shared.readings[0].battery_text(true), "—");
    }

    #[test]
    fn identical_headset_receivers_keep_battery_charge_and_taskbar_lifecycles_independent() {
        let backend = BackendId::HyperX;
        let now = std::time::Instant::now();
        let headset = |instance: &str, percent| {
            let mut sample = snapshot(backend, instance, percent);
            let interface = sample.target.hid_mut();
            interface.path = format!(r"\\?\hid#vid_03f0&pid_0696&mi_00#synthetic-{instance}#{{test}}");
            interface.serial = "same-receiver-serial".into();
            interface.product = "HyperX Cloud II Wireless".into();
            interface.vendor_id = 0x03f0; interface.product_id = 0x0696;
            interface.usage_page = 0xff90; interface.usage = 0x0303;
            interface.input_len = 20; interface.output_len = 52;
            sample.target = Target::new(backend, interface, 0);
            sample.target.family = DeviceFamily::Headset;
            sample.battery_at = Some(now);
            sample
        };
        let mut first = headset("first", 20);
        let mut second = headset("second", 80);
        let mut other = snapshot(BackendId::Logitech, "other-brand", 55);
        other.battery_at = Some(now);
        let first_key = first.target.key.clone();
        let second_key = second.target.key.clone();
        assert_ne!(first_key, second_key);
        let mut shared = Shared::new(Settings::default());
        shared.cfg.device_order = vec![first_key.clone(), second_key.clone(), "other-brand".into()];
        let poll = |samples: Vec<Snapshot>, both_present| crate::backends::Poll {
            snapshots: samples,
            present_channels: Some(if both_present {
                [first.target.channel_key(), second.target.channel_key()].into_iter().collect()
            } else { [second.target.channel_key()].into_iter().collect() }),
            ..Default::default()
        };
        let strip = |shared: &Shared| taskbar_strip_content(&shared.readings, &shared.cfg).unwrap()
            .items.into_iter().map(|item| (item.level.unwrap().value, item.charging)).collect::<Vec<_>>();
        merge_additional_at(&mut shared, backend, poll(vec![first.clone(), second.clone()], true), now);
        merge_additional_at(&mut shared, BackendId::Logitech, vec![other.clone()].into(), now);
        assert_eq!(strip(&shared), vec![(20, false), (80, false), (55, false)]);

        let next = now + Duration::from_secs(10);
        second.battery_at = Some(next);
        second.battery.as_mut().unwrap().percent = Some(79);
        second.battery.as_mut().unwrap().charging = Some(true);
        merge_additional_at(&mut shared, backend, poll(vec![second.clone()], true), next);
        assert_eq!(shared.devices[&first_key].basic_status.as_ref().unwrap().measured_at, Some(now));
        assert_eq!(shared.devices[&first_key].reading.quality, crate::device::ReadingQuality::Cached);
        assert_eq!(strip(&shared), vec![(20, false), (79, true), (55, false)]);

        let expired = now + backend.sample_lifetime();
        second.battery_at = Some(expired);
        other.battery_at = Some(expired);
        merge_additional_at(&mut shared, backend, poll(vec![second.clone()], true), expired);
        merge_additional_at(&mut shared, BackendId::Logitech, vec![other].into(), expired);
        shared.expire_readings_at(expired);
        assert!(shared.devices[&first_key].reading.connected);
        assert_eq!(shared.devices[&first_key].reading.battery_text(true), "—");
        assert_eq!(strip(&shared), vec![(79, true), (55, false)]);

        second.battery_at = Some(expired + Duration::from_secs(1));
        merge_additional_at(&mut shared, backend, poll(vec![second.clone()], false), expired + Duration::from_secs(1));
        assert!(!shared.devices[&first_key].reading.connected);
        assert!(shared.devices[&second_key].reading.connected);
        assert!(shared.devices["other-brand"].reading.connected);
        assert_eq!(strip(&shared), vec![(79, true), (55, false)]);

        let restored = expired + Duration::from_secs(2);
        first.battery_at = Some(restored);
        first.battery.as_mut().unwrap().percent = Some(21);
        second.battery_at = Some(restored);
        merge_additional_at(&mut shared, backend, poll(vec![first.clone(), second.clone()], true), restored);
        assert_eq!(shared.devices.len(), 3);
        assert_eq!(shared.devices[&first_key].reading.quality, crate::device::ReadingQuality::Live);
        assert_eq!(strip(&shared), vec![(21, false), (79, true), (55, false)]);
    }

    #[test]
    fn langtu_silent_receiver_keeps_sample_age_and_wake_restores_estimated_taskbar_level() {
        let backend = BackendId::LangtuT98;
        let now = std::time::Instant::now();
        assert_eq!(Signal::new(60).work_interval_ms(backend.publish_interval_limit_ms()), 15000);
        let mut shared = Shared::new(Settings::default());
        let mut sample = snapshot(backend, "langtu", 78);
        sample.target.family = DeviceFamily::Keyboard;
        sample.battery_at = Some(now);
        sample.battery = Some(Battery { percent: None, estimated_percent: Some(78),
            charging: None, voltage_mv: None, approximate: true, level: None });
        merge_additional_at(&mut shared, backend, vec![sample.clone()].into(), now);
        assert_eq!(shared.readings[0].battery_text(true), "≈78%");
        for seconds in [15, 60, 119, 120, 240] {
            let time = now + Duration::from_secs(seconds);
            merge_additional_at(&mut shared, backend, crate::backends::Poll {
                snapshots: Vec::new(), present_channels: Some(["langtu".into()].into_iter().collect()),
                ..Default::default()
            }, time);
            shared.expire_readings_at(time);
            assert!(shared.readings[0].connected);
            assert_eq!(shared.devices["langtu"].basic_status.as_ref().unwrap().measured_at, Some(now));
            assert_eq!(shared.readings[0].battery_text(true), if seconds < 120 { "≈78%" } else { "—" });
            let strip = taskbar_strip_content(&shared.readings, &shared.cfg).unwrap();
            assert_eq!(!strip.items.is_empty(), seconds < 120);
            assert_eq!(strip.empty_label.as_deref(), (seconds >= 120).then(|| crate::tr!("暂无设备")));
        }
        let woke = now + Duration::from_secs(241);
        sample.battery_at = Some(woke);
        sample.battery.as_mut().unwrap().estimated_percent = Some(77);
        merge_additional_at(&mut shared, backend, vec![sample].into(), woke);
        assert_eq!(shared.readings[0].battery_text(true), "≈77%");
        let item = &taskbar_strip_content(&shared.readings, &shared.cfg).unwrap().items[0];
        assert_eq!(item.level.unwrap().label(true), "≈77%");
        assert!(!item.charging);
        assert_eq!(item.kind, crate::tray_prefs::TrayKind::Keyboard);
    }

    #[test]
    fn mouse_adapter_samples_expire_independently_and_recover_after_disconnect() {
        for backend in [BackendId::Pulsar, BackendId::Inphic] {
        assert_eq!(Signal::new(60).work_interval_ms(backend.publish_interval_limit_ms()), 10_000);
        let now = std::time::Instant::now();
        let mut shared = Shared::new(Settings::default());
        let mut first = snapshot(backend, "pulsar-first", 20);
        let mut second = snapshot(backend, "pulsar-second", 80);
        let mut other = snapshot(BackendId::Logitech, "other-mouse", 55);
        for sample in [&mut first, &mut second, &mut other] { sample.battery_at = Some(now); }
        merge_additional_at(&mut shared, backend, vec![first.clone(), second.clone()].into(), now);
        merge_additional_at(&mut shared, BackendId::Logitech, vec![other].into(), now);
        let poll = |snapshots, present| crate::backends::Poll {
            snapshots, present_channels: Some(if present {
                ["pulsar-first".into(), "pulsar-second".into()].into_iter().collect()
            } else { ["pulsar-second".into()].into_iter().collect() }), ..Default::default()
        };
        let next = now + Duration::from_secs(10);
        second.battery_at = Some(next);
        second.battery.as_mut().unwrap().charging = Some(true);
        merge_additional_at(&mut shared, backend, poll(vec![second.clone()], true), next);
        assert_eq!(shared.devices["pulsar-first"].basic_status.as_ref().unwrap().measured_at, Some(now));
        assert_eq!(shared.devices["pulsar-first"].reading.battery_text(true), "20%");
        assert_eq!(shared.devices["other-mouse"].reading.battery_text(true), "55%");
        let expired = now + backend.sample_lifetime();
        second.battery_at = Some(expired);
        merge_additional_at(&mut shared, backend, poll(vec![second.clone()], true), expired);
        shared.expire_readings_at(expired);
        assert!(shared.devices["pulsar-first"].reading.connected);
        assert_eq!(shared.devices["pulsar-first"].reading.battery_text(true), "—");
        assert_eq!(shared.devices["pulsar-second"].reading.battery_text(true), "80%");
        assert!(taskbar_strip_content(&shared.readings, &shared.cfg).unwrap().items.iter()
            .any(|item| item.charging && item.level.is_some_and(|level| level.label(true) == "80%")));
        first.connected = false;
        first.battery = None;
        first.battery_at = None;
        first.battery_unavailable = true;
        merge_additional_at(&mut shared, backend, poll(vec![first], true), expired);
        assert!(!shared.devices["pulsar-first"].reading.connected);
        assert!(shared.devices["pulsar-second"].reading.connected);
        let woke = expired + Duration::from_secs(1);
        let mut first = snapshot(backend, "pulsar-first", 19);
        first.battery_at = Some(woke);
        merge_additional_at(&mut shared, backend, poll(vec![first], true), woke);
        assert!(shared.devices["pulsar-first"].reading.connected);
        assert_eq!(shared.devices["pulsar-first"].reading.battery_text(true), "19%");
        merge_additional_at(&mut shared, backend, poll(vec![second], false), woke);
        assert!(!shared.devices["pulsar-first"].reading.connected);
        assert!(shared.devices["pulsar-second"].reading.connected);
        }
    }

    #[test]
    fn headset_query_timeouts_expire_and_explicit_offline_clears_before_wake() {
        for backend in [BackendId::Inzone, BackendId::HyperX] {
        let now = std::time::Instant::now();
        assert_eq!(Signal::new(60).work_interval_ms(backend.publish_interval_limit_ms()), 10000);
        let mut shared = Shared::new(Settings::default());
        let mut sample = snapshot(backend, "headset", 61);
        sample.target.family = DeviceFamily::Headset;
        sample.battery_at = Some(now);
        merge_additional_at(&mut shared, backend, vec![sample.clone()].into(), now);
        assert_eq!(shared.devices["headset"].reading.battery_text(true), "61%");
        let silent = crate::backends::Poll { snapshots: Vec::new(),
            present_channels: Some(["headset".into()].into_iter().collect()), ..Default::default() };
        merge_additional_at(&mut shared, backend, silent, now + Duration::from_secs(10));
        assert!(shared.devices["headset"].reading.connected);
        assert_eq!(shared.devices["headset"].reading.quality, crate::device::ReadingQuality::Cached);
        assert_eq!(shared.devices["headset"].basic_status.as_ref().unwrap().measured_at, Some(now));
        shared.expire_readings_at(now + backend.sample_lifetime());
        assert_eq!(shared.devices["headset"].reading.battery_text(true), "—");
        let mut offline = sample.clone();
        offline.connected = false; offline.battery = None; offline.battery_at = None;
        offline.battery_unavailable = true;
        merge_additional_at(&mut shared, backend, vec![offline].into(), now + Duration::from_secs(121));
        assert!(!shared.devices["headset"].reading.connected);
        assert!(shared.devices["headset"].reading.battery.is_none());
        sample.battery_at = Some(now + Duration::from_secs(122));
        sample.battery.as_mut().unwrap().percent = Some(59);
        merge_additional_at(&mut shared, backend, vec![sample].into(), now + Duration::from_secs(122));
        assert!(shared.devices["headset"].reading.connected);
        assert_eq!(shared.devices["headset"].reading.battery_text(true), "59%");
        assert_eq!(shared.devices["headset"].reading.quality, crate::device::ReadingQuality::Live);
        assert_eq!(shared.devices["headset"].reading.family, DeviceFamily::Headset);
        }
    }

    #[test]
    fn queried_device_silence_keeps_sample_age_then_wake_restores_tray_without_manual_refresh() {
        for (backend, family) in [(BackendId::VgnKeyboard, DeviceFamily::Keyboard),
            (BackendId::VgnBeiying, DeviceFamily::Keyboard),
            (BackendId::MchoseQhw, DeviceFamily::Keyboard), (BackendId::Keyboard20, DeviceFamily::Keyboard),
            (BackendId::MchoseGlw, DeviceFamily::Keyboard),
            (BackendId::MchoseK99, DeviceFamily::Keyboard),
            (BackendId::Moondrop, DeviceFamily::Headset),
            (BackendId::RapooLegacy, DeviceFamily::Mouse),
            (BackendId::RapooOffice, DeviceFamily::Mouse),
            (BackendId::AulaHfd, DeviceFamily::Keyboard),
            (BackendId::MchoseMc, DeviceFamily::Headset),
            (BackendId::MchoseR9, DeviceFamily::Headset),
            (BackendId::RapooMt560, DeviceFamily::Mouse),
            (BackendId::AulaV9Pro, DeviceFamily::Mouse),
            (BackendId::DareuEm901, DeviceFamily::Mouse),
            (BackendId::AulaHfdMechanical, DeviceFamily::Keyboard),
            (BackendId::AulaF99, DeviceFamily::Keyboard),
            (BackendId::AulaF108, DeviceFamily::Keyboard),
            (BackendId::AtkNano, DeviceFamily::Mouse),
            (BackendId::AtkHoltek, DeviceFamily::Mouse),
            (BackendId::Lingbao, DeviceFamily::Mouse),
            (BackendId::MchoseK87s, DeviceFamily::Keyboard),
            (BackendId::FifineHt6, DeviceFamily::Headset),
            (BackendId::EweadnX87, DeviceFamily::Keyboard),
            (BackendId::Xiberia, DeviceFamily::Headset),
            (BackendId::Edifier, DeviceFamily::Headset), (BackendId::EweadnS9Pro, DeviceFamily::Mouse),
            (BackendId::EweadnS9, DeviceFamily::Mouse),
            (BackendId::Dareu, DeviceFamily::Mouse),
            (BackendId::VgnDragonfly3, DeviceFamily::Mouse),
            (BackendId::VgnRy, DeviceFamily::Mouse),
            (BackendId::MchoseModern, DeviceFamily::Mouse),
            (BackendId::MchoseMouse, DeviceFamily::Mouse), (BackendId::GearHub, DeviceFamily::Mouse), (BackendId::G3m, DeviceFamily::Mouse), (BackendId::Aula, DeviceFamily::Mouse),
            (BackendId::EightBitDo, DeviceFamily::Gamepad),
            (BackendId::Flydigi, DeviceFamily::Gamepad),
            (BackendId::GameSir, DeviceFamily::Gamepad)] {
        let now = std::time::Instant::now();
        let interval = match backend {
            BackendId::AulaF108 => 15000,
            BackendId::AulaHfdMechanical => 30000,
            BackendId::RapooMt560 => 2000,
            _ => 10000,
        };
        assert_eq!(Signal::new(60).work_interval_ms(backend.publish_interval_limit_ms()), interval);
        let mut shared = Shared::new(Settings::default());
        let mut sample = snapshot(backend, "keyboard", 93);
        sample.target.family = family;
        sample.battery_at = Some(now);
        sample.battery.as_mut().unwrap().charging = None;
        merge_additional_at(&mut shared, backend, vec![sample.clone()].into(), now);
        for second in [10, 60, 119, 120, 130] {
            let time = now + Duration::from_secs(second);
            let sleeping = crate::backends::Poll { snapshots: Vec::new(),
                present_channels: Some(["keyboard".into()].into_iter().collect()), ..Default::default() };
            merge_additional_at(&mut shared, backend, sleeping, time);
            shared.expire_readings_at(time);
            let device = &shared.devices["keyboard"];
            assert!(device.reading.connected);
            assert_eq!(device.basic_status.as_ref().unwrap().measured_at, Some(now));
            assert_eq!(device.reading.battery_text(true), if second < 120 { "93%" } else { "—" });
        }
        let woke = now + Duration::from_secs(140);
        sample.battery_at = Some(woke); sample.battery.as_mut().unwrap().percent = Some(92);
        merge_additional_at(&mut shared, backend, vec![sample].into(), woke);
        assert_eq!(shared.readings[0].battery_text(true), "92%");
        assert_eq!(shared.readings[0].quality, crate::device::ReadingQuality::Live);
        assert_eq!(shared.readings[0].family, family);
        }
    }

    #[test]
    fn new_receiver_explicit_sleep_clears_tray_level_and_wake_returns_it() {
        for backend in [BackendId::VgnRy, BackendId::MchoseModern, BackendId::MchoseMc,
            BackendId::AtkHoltek, BackendId::FifineHt6, BackendId::MchoseR9] {
            let mut shared = Shared::new(Settings::default());
            let now = std::time::Instant::now();
            let mut sample = snapshot(backend, "mouse", 85);
            sample.target.family = if matches!(backend, BackendId::MchoseMc | BackendId::FifineHt6) {
                DeviceFamily::Headset
            } else { DeviceFamily::Mouse };
            sample.battery_at = Some(now);
            merge_additional_at(&mut shared, backend, vec![sample.clone()].into(), now);
            assert_eq!(shared.readings[0].battery_text(true), "85%");
            let mut sleep = sample.clone();
            sleep.connected = false;
            sleep.battery = None;
            sleep.battery_at = None;
            sleep.battery_unavailable = true;
            merge_additional_at(&mut shared, backend, vec![sleep].into(), now + Duration::from_secs(10));
            assert!(!shared.devices["mouse"].reading.connected);
            assert!(shared.devices["mouse"].reading.battery.is_none());
            let woke = now + Duration::from_secs(20);
            sample.battery_at = Some(woke);
            sample.battery.as_mut().unwrap().percent = Some(84);
            merge_additional_at(&mut shared, backend, vec![sample].into(), woke);
            assert!(shared.readings[0].connected);
            assert_eq!(shared.readings[0].battery_text(true), "84%");
        }
    }

    #[test]
    fn fresh_charging_unknown_clears_percentage_and_shows_charge_in_main_and_tray() {
        for backend in [BackendId::RapooOffice, BackendId::Headset, BackendId::GameSir] {
            let mut shared = Shared::new(Settings::default());
            let now = std::time::Instant::now();
            let mut sample = snapshot(backend, "charging", 47);
            sample.battery_at = Some(now);
            merge_additional_at(&mut shared, backend, vec![sample.clone()].into(), now);
            assert_eq!(shared.readings[0].battery_text(true), "47%");
            let measured = now + Duration::from_secs(10);
            sample.battery.as_mut().unwrap().percent = None;
            sample.battery.as_mut().unwrap().charging = Some(true);
            sample.battery_at = Some(measured);
            merge_additional_at(&mut shared, backend, vec![sample].into(), measured);
            assert_eq!(shared.readings[0].battery_text(true), "—");
            assert!(shared.readings[0].is_charging());
            assert!(shared.devices["charging"].reading.is_charging());
            merge_additional_at(&mut shared, backend, Vec::new().into(), measured + Duration::from_secs(120));
            assert!(!shared.readings[0].is_charging());
            assert_eq!(shared.readings[0].battery_text(true), "—");
        }
    }

    #[test]
    fn xinput_slots_clear_wired_and_replaced_battery_without_affecting_other_devices() {
        let mut shared = Shared::new(Settings::default());
        let backend = BackendId::XInput;
        let now = std::time::Instant::now();
        let mut first = snapshot(backend, "unused", 50);
        first.target = crate::backends::xinput::target(0);
        first.battery_at = Some(now);
        first.battery = Some(Battery { percent: None, estimated_percent: Some(50), charging: None,
            voltage_mv: None, approximate: true, level: Some(BatteryLevel::Medium) });
        let key = first.target.key.clone();
        let mut other = first.clone(); other.target = crate::backends::xinput::target(1);
        let other_key = other.target.key.clone();
        merge_additional_at(&mut shared, backend, vec![first.clone(), other.clone()].into(), now);
        assert_eq!(shared.devices[&key].reading.battery_text(true), "≈50%");
        let mut wired = first.clone(); wired.battery = None; wired.battery_at = None;
        wired.battery_unavailable = true;
        merge_additional_at(&mut shared, backend, vec![wired, other.clone()].into(), now + Duration::from_secs(1));
        assert_eq!(shared.devices[&key].reading.battery_text(true), "—");
        assert!(shared.devices[&key].reading.connected);
        assert_eq!(shared.devices[&other_key].reading.battery_text(true), "≈50%");
        first.battery_at = Some(now + Duration::from_secs(2));
        merge_additional_at(&mut shared, backend, vec![first, other.clone()].into(), now + Duration::from_secs(2));
        merge_additional_at(&mut shared, backend, vec![other.clone()].into(), now + Duration::from_secs(3));
        assert!(!shared.devices[&key].reading.connected);
        let mut replacement = snapshot(backend, "unused", 99);
        replacement.target = crate::backends::xinput::target(0);
        replacement.battery = None; replacement.battery_at = None;
        merge_additional_at(&mut shared, backend, vec![replacement, other].into(), now + Duration::from_secs(4));
        assert!(shared.devices[&key].reading.connected);
        assert_eq!(shared.devices[&key].reading.battery_text(true), "—");
        assert_eq!(shared.devices[&other_key].reading.battery_text(true), "≈50%");
    }

    #[test]
    fn standard_topology_resolution_recovers_without_reusing_old_identity() {
        assert_standard_topology_transition(None, Some("usb\\vid_1234&pid_5678\\receiver-a"));
    }

    #[test]
    fn standard_topology_loss_recovers_without_reusing_old_identity() {
        assert_standard_topology_transition(Some("usb\\vid_1234&pid_5678\\receiver-a"), None);
    }

    fn assert_standard_topology_transition(first_group: Option<&str>, next_group: Option<&str>) {
        let mut shared = Shared::new(Settings::default());
        let now = std::time::Instant::now();
        let interface = hid::HidInterface {
            path: "receiver-a-col01".into(), serial: "shared-serial".into(), product: "HID mouse".into(),
            vendor_id: 0x1234, product_id: 0x5678, usage_page: 1, usage: 2,
            input_len: 8, output_len: 0, feature_len: 8,
        };
        let other_interface = hid::HidInterface { path: "receiver-b-col01".into(), ..interface.clone() };
        let first = crate::backends::standard::snapshot_for_test(&interface, first_group, 90, now);
        let other = crate::backends::standard::snapshot_for_test(&other_interface,
            Some("usb\\vid_1234&pid_5678\\receiver-b"), 70, now);
        let old_key = first.target.key.clone();
        let other_key = other.target.key.clone();
        merge_additional_at(&mut shared, BackendId::StandardHid, vec![first, other.clone()].into(), now);
        shared.cfg.device_names.insert(old_key.clone(), "Old discovery preference".into());
        shared.history_tracker.take_pending();

        let measured = now + Duration::from_secs(1);
        let next = crate::backends::standard::snapshot_for_test(&interface, next_group, 25, measured);
        let new_key = next.target.key.clone();
        assert_ne!(new_key, old_key);
        assert_ne!(new_key, other_key);
        merge_additional_at(&mut shared, BackendId::StandardHid, vec![next, other.clone()].into(), measured);
        let current = shared.devices.get(&new_key).expect("successful Standard HID sample must be published");
        assert_eq!(current.reading.percent(), Some(25));
        assert_eq!(current.battery_at, Some(measured));
        assert_eq!(current.reading.quality, crate::device::ReadingQuality::Live);
        let basic = current.basic.as_ref().unwrap();
        assert!(!basic.target.verified_identity);
        assert!(!basic.dpi.writable && !basic.dpi.stage_writable && !basic.polling_rate.writable);
        assert!(!shared.devices[&old_key].reading.connected);
        assert_eq!(shared.devices[&other_key].reading.percent(), Some(70));
        assert!(!shared.cfg.device_names.contains_key(&new_key));
        assert_eq!(shared.cfg.device_names[&old_key], "Old discovery preference");
        assert!(shared.key_aliases.is_empty());
        let (samples, aliases) = shared.history_tracker.take_pending();
        assert!(aliases.is_empty());
        assert!(samples.iter().all(|(key, _)| key != &old_key));

        let measured = measured + Duration::from_secs(1);
        let next = crate::backends::standard::snapshot_for_test(&interface, next_group, 24, measured);
        merge_additional_at(&mut shared, BackendId::StandardHid, vec![next, other].into(), measured);
        assert_eq!(shared.devices[&new_key].reading.percent(), Some(24));
        assert_eq!(shared.devices[&new_key].battery_at, Some(measured));
        assert!(!shared.devices[&old_key].reading.connected);
        assert_eq!(shared.devices.len(), 3);
    }

    #[test]
    fn same_channel_refresh_never_promotes_a_retired_discovery_record() {
        let mut shared = Shared::new(Settings::default());
        let now = std::time::Instant::now();
        let mut sample = snapshot(BackendId::Logitech, "receiver", 90);
        sample.battery_at = Some(now);
        sample.target.key = "retired-discovery".into();
        merge_additional_at(&mut shared, BackendId::Logitech, vec![sample.clone()].into(), now);
        shared.cfg.device_names.insert(sample.target.key.clone(), "Retired device".into());
        sample.target.key = "current-discovery".into();
        merge_additional_at(&mut shared, BackendId::Logitech, vec![sample.clone()].into(), now + Duration::from_secs(1));
        sample.target.key = "confirmed-current".into();
        sample.target.verified_identity = true;
        merge_additional_at(&mut shared, BackendId::Logitech, vec![sample.clone()].into(), now + Duration::from_secs(2));
        shared.history_tracker.take_pending();
        sample.battery.as_mut().unwrap().percent = Some(25);
        sample.battery_at = Some(now + Duration::from_secs(3));
        merge_additional_at(&mut shared, BackendId::Logitech, vec![sample].into(), now + Duration::from_secs(3));
        assert_eq!(shared.devices["confirmed-current"].reading.percent(), Some(25));
        assert!(!shared.devices["retired-discovery"].reading.connected);
        assert_eq!(shared.cfg.device_names["retired-discovery"], "Retired device");
        assert!(!shared.cfg.device_names.contains_key("confirmed-current"));
        assert!(!shared.key_aliases.contains_key("retired-discovery"));
        assert!(shared.history_tracker.take_pending().1.is_empty());
        assert_eq!(shared.devices.len(), 2);
    }

    #[test]
    fn additional_identity_promotion_preserves_name_type_tray_and_order() {
        let mut shared = Shared::new(Settings::default());
        let mut first = snapshot(BackendId::Logitech, "interface", 75);
        first.target.key = Target::new(BackendId::Logitech, first.target.hid().unwrap(), first.target.address).key;
        let old = first.target.key.clone();
        shared.publish_additional(BackendId::Logitech, vec![first.clone()], 0, 1);
        shared.active_key = Some(old.clone());
        shared.primary_key = Some(old.clone());
        shared.cfg.tray_device = Some(old.clone());
        shared.cfg.device_names.insert(old.clone(), "Custom device".into());
        let presets = crate::device::DpiPresets {enabled:true,values:vec![800,1600], ..Default::default()};
        shared.cfg.dpi_presets.insert(old.clone(),presets.clone());
        shared.cfg.device_order = vec![old.clone()];
        let pref = crate::tray_prefs::DeviceTray { kind: crate::tray_prefs::TrayKind::Generic,
            ..Default::default() };
        shared.cfg.device_tray.insert(old.clone(), pref);
        let new = crate::device::stable_key("verified-unit");
        first.target.key = new.clone();
        first.target.verified_identity = true;
        shared.publish_additional(BackendId::Logitech, vec![first], 0, 2);
        assert!(!shared.devices.contains_key(&old));
        assert_eq!(shared.key_aliases[&old], new);
        assert_eq!(shared.active_key.as_ref(), Some(&new));
        assert_eq!(shared.primary_key.as_ref(), Some(&new));
        assert_eq!(shared.cfg.tray_device.as_ref(), Some(&new));
        assert_eq!(shared.cfg.device_names[&new], "Custom device");
        assert_eq!(shared.cfg.dpi_presets[&new],presets);
        assert!(!shared.cfg.dpi_presets.contains_key(&old));
        assert_eq!(shared.cfg.device_tray[&new], pref);
        assert_eq!(shared.cfg.device_order, vec![new]);
        assert!(shared.device_keys_dirty);
    }

    #[test]
    fn missing_identity_keeps_card_without_authorizing_writes_or_inheriting_on_replacement() {
        let mut shared = Shared::new(Settings::default());
        let mut first = snapshot(BackendId::Logitech, "interface", 75);
        let link_key = Target::new(BackendId::Logitech, first.target.hid().unwrap(), first.target.address).key;
        let old = crate::device::stable_key("first-unit");
        first.target.key = old.clone();
        first.target.verified_identity = true;
        first.dpi.writable = true;
        shared.publish_additional(BackendId::Logitech, vec![first.clone()], 0, 1);
        shared.cfg.device_names.insert(old.clone(), "Custom first device".into());
        shared.cfg.dpi_presets.insert(old.clone(),crate::device::DpiPresets {enabled:true,values:vec![800], ..Default::default()});
        first.target.key = link_key;
        first.target.verified_identity = false;
        shared.publish_additional(BackendId::Logitech, vec![first.clone()], 0, 2);
        assert_eq!(shared.devices.len(), 1);
        assert!(shared.devices[&old].reading.connected);
        assert!(!shared.devices[&old].basic.as_ref().unwrap().dpi.writable);
        let new = crate::device::stable_key("replacement-unit");
        first.target.key = new.clone();
        first.target.verified_identity = true;
        shared.publish_additional(BackendId::Logitech, vec![first], 0, 3);
        assert!(!shared.devices[&old].reading.connected);
        assert!(shared.devices[&new].reading.connected);
        assert!(!shared.cfg.device_names.contains_key(&new));
        assert!(!shared.cfg.dpi_presets.contains_key(&new));
        assert_eq!(shared.cfg.dpi_presets[&old].values,vec![800]);
        assert_eq!(shared.cfg.device_names[&old], "Custom first device");
        assert!(!shared.device_keys_dirty);
    }

    #[test]
    fn missing_identity_never_attributes_a_new_measurement_to_the_previous_device() {
        for backend in [BackendId::Logitech, BackendId::Razer] {
            let mut shared = Shared::new(Settings::default());
            let now = std::time::Instant::now();
            let mut sample = snapshot(backend, "receiver", 75);
            sample.target.key = "confirmed-first".into();
            sample.target.verified_identity = true;
            sample.battery_at = Some(now);
            merge_additional_at(&mut shared, backend, vec![sample.clone()].into(), now);
            shared.history_tracker.take_pending();
            sample.target.key = sample.target.unverified_key();
            sample.target.verified_identity = false;
            sample.battery.as_mut().unwrap().percent = Some(12);
            sample.battery_at = Some(now + Duration::from_secs(1));
            merge_additional_at(&mut shared, backend, vec![sample.clone()].into(), now + Duration::from_secs(1));
            let previous = &shared.devices["confirmed-first"];
            assert_eq!(previous.reading.percent(), Some(75));
            assert_eq!(previous.reading.quality, crate::device::ReadingQuality::Cached);
            assert_eq!(previous.battery_at, Some(now));
            assert!(shared.history_tracker.take_pending().0.is_empty());
            sample.target.key = "confirmed-replacement".into();
            sample.target.verified_identity = true;
            merge_additional_at(&mut shared, backend, vec![sample].into(), now + Duration::from_secs(2));
            assert!(!shared.devices["confirmed-first"].reading.connected);
            assert_eq!(shared.devices["confirmed-replacement"].reading.percent(), Some(12));
        }
    }

    #[test]
    fn writes_reject_only_their_own_backend_stale_readings() {
        let shared = Arc::new(Mutex::new(Shared::new(Settings::default())));
        let signal = Signal::new(60);
        let target = snapshot(BackendId::Logitech, "logitech", 75).target;
        assert!(submit(&shared, &signal, Cmd::BasicDpi { target, dpi: 800 }));
        let mut state = lock_shared(&shared);
        assert_eq!(state.write_gen, 0);
        assert!(!state.publish_additional(BackendId::Logitech, vec![snapshot(BackendId::Logitech, "logitech", 50)], 0, 1));
        assert_eq!(state.completed_requests[&BackendId::Logitech], 0);
        assert!(state.publish_additional(BackendId::StandardHid, vec![snapshot(BackendId::StandardHid, "standard", 70)], 0, 1));
        assert!(state.publish_additional(BackendId::Logitech, vec![snapshot(BackendId::Logitech, "logitech", 75)], 1, 1));
        assert_eq!(state.devices["logitech"].reading.percent(), Some(75));
    }

    #[test]
    fn rog_basic_dpi_reaches_the_rog_command_queue() {
        let shared = Arc::new(Mutex::new(Shared::new(Settings::default())));
        let signal = Signal::new(60);
        let target = snapshot(BackendId::Rog, "rog", 75).target;
        assert!(submit(&shared, &signal, Cmd::BasicDpi { target, dpi: 800 }));
        let state = lock_shared(&shared);
        assert_eq!(state.pending.len(), 1);
        assert_eq!(state.write_gen, 1);
        assert!(state.source_pending.is_empty());
    }

    #[test]
    fn rog_queue_keeps_selected_identity_when_receiver_record_changes() {
        let shared = Arc::new(Mutex::new(Shared::new(Settings::default())));
        let signal = Signal::new(60);
        let mut selected = history_owner_device("selected", DeviceFamily::Mouse, true, None);
        selected.ident.mouse_sn = "first-mouse".into();
        {
            let mut state = lock_shared(&shared);
            state.devices.insert("selected".into(), selected);
            state.active_key = Some("selected".into());
        }
        assert!(submit(&shared, &signal, Cmd::Stage(2)));
        let mut state = lock_shared(&shared);
        state.devices.get_mut("selected").unwrap().ident.mouse_sn = "replacement-mouse".into();
        let (_, target) = state.pending.front().unwrap();
        assert_eq!(target.as_ref().unwrap().identity.as_deref(), Some("serial:first-mouse"));
        assert_eq!(target.as_ref().unwrap().link.iface.path, "selected");
    }

    #[test]
    fn onboard_dpi_preserves_fifth_slot_and_uses_its_backend_queue() {
        let shared = Arc::new(Mutex::new(Shared::new(Settings::default())));
        let signal = Signal::new(60);
        let mut reading = snapshot(BackendId::Logitech, "logitech", 75);
        let stage = crate::device::DpiStage { index: 4, value: 3200 };
        reading.dpi = Dpi { current: Some(3200),
            stages: vec![crate::device::DpiStage { index: 0, value: 800 }, stage],
            active_stage: Some(4), profile: Some(1), stage_writable: true, ..Default::default() };
        let target = reading.target.clone();
        lock_shared(&shared).publish_additional(BackendId::Logitech, vec![reading], 0, 1);
        let popup = popup_dpi_states(&lock_shared(&shared));
        assert_eq!(popup["logitech"].stages[1], stage);
        assert_eq!(popup["logitech"].active, Some(4));
        assert!(!popup["logitech"].value_control);
        assert!(submit(&shared, &signal, Cmd::BasicDpiStage { target, stage, context: crate::device::StageContext::Profile(1) }));
        let state = lock_shared(&shared);
        assert!(state.pending.is_empty());
        assert_eq!(state.source_pending[&BackendId::Logitech].len(), 1);
        assert_eq!(state.source_write_gen[&BackendId::Logitech], 1);
    }

    #[test]
    fn configuration_bound_stages_use_their_backend_queue_and_disable_writes_after_partial_reads() {
        for (backend, key, last) in [(BackendId::Rapoo, "rapoo", 6), (BackendId::Dareu, "dareu", 4),
            (BackendId::MchoseMouse, "mchose", 5), (BackendId::GearHub, "gearhub", 7), (BackendId::Aula, "aula", 5)] {
        let shared=Arc::new(Mutex::new(Shared::new(Settings::default())));let signal=Signal::new(60);
        let mut reading=snapshot(backend,key,70);
        reading.dpi=Dpi {current:Some(800),current_y:Some(1600),active_stage:Some(1),profile:Some(2),
            configuration_token:Some(42),stage_writable:true,
            stages:vec![crate::device::DpiStage {index:1,value:800},crate::device::DpiStage {index:last,value:25600}],
            stage_y:[(1,1600),(last,30000)].into(),..Default::default()};
        let target=reading.target.clone();
        lock_shared(&shared).publish_additional(backend,vec![reading.clone()],0,1);
        let p=popup_dpi_states(&lock_shared(&shared));let dpi=&p[key];
        assert!(!dpi.custom && !dpi.value_control && dpi.writable);
        assert_eq!(dpi.stages[1].index,last);assert_eq!(dpi.context,Some(crate::device::StageContext::Configuration(42)));
        assert_eq!(dpi.current_y,Some(1600));assert_eq!(dpi.stage_y[&last],30000);
        assert!(submit(&shared,&signal,Cmd::BasicDpiStage {target,stage:dpi.stages[1],context:dpi.context.unwrap()}));
        assert_eq!(lock_shared(&shared).source_pending[&backend].len(),1);
        reading.dpi=Dpi {current:Some(800),current_y:Some(1600),..Default::default()};
        lock_shared(&shared).publish_additional(backend,vec![reading],1,2);
        assert!(!popup_dpi_states(&lock_shared(&shared))[key].writable);
        assert_eq!(lock_shared(&shared).readings[0].percent(),Some(70));
        }
    }

    #[test]
    fn rapoo_serial_preference_migration_requires_an_unambiguous_receiver_and_runs_once() {
        let mut a=snapshot(BackendId::Rapoo,"a",70);a.target.hid_mut().serial ="same".into();
        a.target.hid_mut().vendor_id =0x24ae;a.target.hid_mut().product_id =0x1415;
        let old=crate::device::stable_key("Rapoo nRF54L:24ae:1415:same:1");
        let mut cfg=Settings::default();cfg.device_names.insert(old.clone(),"Custom".into());
        cfg.device_tray.insert(old.clone(),Default::default());cfg.tray_device=Some(old.clone());
        let mut shared=Shared::new(cfg.clone());
        shared.publish_additional(BackendId::Rapoo,vec![a.clone()],0,1);
        assert_eq!(shared.cfg.device_names["a"],"Custom");
        assert_eq!(shared.cfg.tray_device.as_deref(),Some("a"));
        assert!(!shared.cfg.device_tray.contains_key(&old));
        shared.device_keys_dirty=false;
        shared.publish_additional(BackendId::Rapoo,vec![a.clone()],0,2);
        assert!(!shared.device_keys_dirty);
        let mut b=a.clone();b.target.key="b".into();b.target.hid_mut().path ="b".into();
        let mut shared=Shared::new(cfg.clone());
        let mut partial = crate::backends::Poll::from_snapshots(vec![a.clone(), b.clone()]);
        partial.snapshots.retain(|sample| sample.target.key == "a");
        shared.publish_additional(BackendId::Rapoo, partial, 0, 1);
        assert!(shared.cfg.device_names.contains_key(&old));
        assert!(!shared.cfg.device_names.contains_key("a"));
        assert_eq!(shared.cfg.tray_device.as_deref(), Some(old.as_str()));
        let mut shared=Shared::new(cfg);
        shared.publish_additional(BackendId::Rapoo,vec![a,b],0,1);
        assert!(shared.cfg.device_names.contains_key(&old));
        assert!(!shared.cfg.device_names.contains_key("a"));assert!(!shared.cfg.device_names.contains_key("b"));
        assert_eq!(shared.readings.len(),2);
    }

    #[test]
    fn rapoo_paired_wired_and_receiver_coalesce_and_replug_replaces_unplugged_card() {
        let mk_rapoo = |pid: u16, path: &str, serial: &str, percent: u8, connected: bool| {
            let iface = hid::HidInterface {
                vendor_id: 0x24ae, product_id: pid, usage_page: 0xff00, usage: 2,
                input_len: 19, output_len: 0, feature_len: 0, serial: serial.into(),
                product: "Rapoo VT1".into(), path: path.into(),
            };
            let mut snap = snapshot(BackendId::Rapoo, path, percent);
            snap.target = Target::new(BackendId::Rapoo, &iface, 0);
            snap.target.name = "Rapoo VT1".into();
            snap.connected = connected;
            if !connected {
                snap.battery = None;
                snap.battery_unavailable = true;
                snap.battery_at = None;
            }
            snap
        };

        let rx_a = mk_rapoo(0x1415, r"\\?\hid#vid_24ae&pid_1415&mi_01&col09#a&1111&0&0008#hid", "2507-54L", 58, true);
        let rx_a_key = rx_a.target.key.clone();
        let wired_offline_rx = mk_rapoo(0x1415, r"\\?\hid#vid_24ae&pid_1415&mi_01&col09#a&1111&0&0008#hid", "2507-54L", 0, false);
        let wired = mk_rapoo(0x4615, r"\\?\hid#vid_24ae&pid_4615&mi_01&col09#a&2222&0&0008#hid", "2507-54L", 59, true);
        let rx_b = mk_rapoo(0x1415, r"\\?\hid#vid_24ae&pid_1415&mi_01&col09#a&3333&0&0008#hid", "2507-54L", 60, true);
        let rx_b_key = rx_b.target.key.clone();

        let mut cfg = Settings::default();
        cfg.device_names.insert(rx_a_key.clone(), "My VT1".into());
        cfg.device_order = vec![rx_a_key.clone()];
        let mut shared = Shared::new(cfg);

        // 1. 仅 2.4G 接收器连接：产生 1 张卡。
        shared.publish_additional(BackendId::Rapoo, vec![rx_a], 0, 1);
        assert_eq!(shared.readings.len(), 1);
        assert_eq!(shared.readings[0].percent(), Some(58));

        // 2. 同时插上有线充电且 2.4G 暂停上报：仍然只显示 1 张卡并沿用有线电量。
        shared.publish_additional(BackendId::Rapoo, vec![wired_offline_rx, wired], 0, 2);
        assert_eq!(shared.readings.len(), 1);
        assert_eq!(shared.readings[0].key, rx_a_key);
        assert!(shared.readings[0].connected);
        assert_eq!(shared.readings[0].percent(), Some(59));

        // 3. 拔出后将 2.4G 接收器插到另一 USB 端口：旧离线卡平滑迁移，不留重复离线卡。
        shared.publish_additional(BackendId::Rapoo, crate::backends::Poll {
            present_channels: Some(Default::default()), ..Default::default()
        }, 0, 3);
        assert_eq!(shared.readings.len(), 1);
        assert!(!shared.readings[0].connected);

        shared.publish_additional(BackendId::Rapoo, vec![rx_b], 0, 4);
        assert_eq!(shared.readings.len(), 1);
        assert_eq!(shared.readings[0].key, rx_a_key);
        assert!(shared.readings[0].connected);
        assert_eq!(shared.readings[0].percent(), Some(60));
        assert_eq!(shared.cfg.device_names.get(&rx_a_key).map(String::as_str), Some("My VT1"));
        assert_eq!(shared.cfg.device_order, vec![rx_a_key.clone()]);
        assert_eq!(shared.key_aliases.get(&rx_b_key), Some(&rx_a_key));
    }

    fn rapoo_endpoint(pid: u16, path: &str, serial: &str, connected: bool) -> Snapshot {
        let interface = hid::HidInterface {
            vendor_id: 0x24ae, product_id: pid, usage_page: 0xff00, usage: 2,
            input_len: 19, output_len: 0, feature_len: 0, serial: serial.into(),
            product: "Rapoo VT1".into(), path: path.into(),
        };
        let mut sample = snapshot(BackendId::Rapoo, path, 58);
        sample.target = Target::new(BackendId::Rapoo, &interface, 0);
        sample.connected = connected;
        sample.dpi = Dpi {
            current: Some(800), current_y: Some(900), profile: Some(1),
            configuration_token: Some(u64::from(pid)), active_stage: Some(0),
            stages: vec![crate::device::DpiStage { index: 0, value: 800 }],
            stage_y: [(0, 900)].into(), stage_writable: connected, stage_editable: connected,
            max_stages: 7, ..Default::default()
        };
        sample.polling_rate = crate::device::PollingRate {
            current: Some(1000), values: vec![500, 1000], writable: connected, ..Default::default()
        };
        if !connected {
            sample.battery = None;
            sample.battery_at = None;
            sample.battery_unavailable = true;
        }
        sample
    }

    fn rapoo_poll(snapshots: Vec<Snapshot>, present: &[&Snapshot]) -> crate::backends::Poll {
        let mut poll = crate::backends::Poll::from_snapshots(present.iter().map(|sample| (*sample).clone()).collect());
        poll.snapshots = snapshots;
        poll
    }

    #[test]
    fn rapoo_incremental_both_endpoints_offline_in_either_order_keeps_one_offline_card() {
        for reverse in [false, true] {
            for publication in [Publication::Battery, Publication::Detail] {
                let mut receiver = rapoo_endpoint(0x1415, "receiver", "pair", true);
                let mut wired = rapoo_endpoint(0x4615, "wired", "pair", true);
                let now = std::time::Instant::now();
                receiver.battery_at = Some(now);
                wired.battery_at = Some(now);
                let mut shared = Shared::new(Settings::default());
                shared.source_discoveries.insert(BackendId::Rapoo, crate::backends::BackendScan {
                    targets: vec![receiver.target.clone(), wired.target.clone()],
                    presence: rapoo_poll(Vec::new(), &[&receiver, &wired]),
                });
                merge_additional_at(&mut shared, BackendId::Rapoo,
                    rapoo_poll(vec![receiver.clone(), wired.clone()], &[&receiver, &wired]), now);
                assert_eq!(shared.devices.len(), 1);
                assert_eq!(shared.readings.len(), 1);
                let key = shared.readings[0].key.clone();
                let endpoints = if reverse { [&wired, &receiver] } else { [&receiver, &wired] };
                for (index, endpoint) in endpoints.iter().enumerate() {
                    let mut offline = (*endpoint).clone();
                    offline.connected = false;
                    offline.battery = None;
                    offline.battery_at = None;
                    offline.battery_unavailable = true;
                    offline.dpi = Dpi::default();
                    offline.polling_rate = Default::default();
                    merge_additional_update(&mut shared, BackendId::Rapoo,
                        rapoo_poll(vec![offline], &[&receiver, &wired]),
                        now + Duration::from_secs(index as u64 + 1), publication);
                    assert_eq!(shared.devices.len(), 1);
                    assert_eq!(shared.readings.len(), 1);
                    assert_eq!(shared.readings[0].key, key);
                    let current = &shared.devices[&key];
                    assert_eq!(current.reading.connected, index == 0);
                    assert_eq!(current.basic.as_ref().unwrap().connected, index == 0);
                    if index == 0 {
                        assert!(current.basic.as_ref().unwrap().target.same_channel(&endpoints[1].target));
                        assert_eq!(current.reading.percent(), Some(58));
                        assert_eq!(current.full_at, now);
                    } else {
                        let basic = current.basic.as_ref().unwrap();
                        assert!(!basic.dpi.writable && !basic.dpi.stage_writable
                            && !basic.dpi.stage_editable && !basic.polling_rate.writable);
                        assert!(current.reading.battery.is_none());
                        assert!(current.basic_status.as_ref().unwrap().connection_known);
                    }
                }
                let mut recovered = endpoints[0].clone();
                recovered.battery.as_mut().unwrap().percent = Some(59);
                recovered.battery_at = Some(now + Duration::from_secs(3));
                recovered.dpi = Dpi::default();
                recovered.polling_rate = Default::default();
                merge_additional_update(&mut shared, BackendId::Rapoo,
                    rapoo_poll(vec![recovered.clone()], &[&receiver, &wired]),
                    now + Duration::from_secs(3), Publication::Battery);
                assert_eq!(shared.readings.len(), 1);
                assert!(shared.readings[0].connected);
                assert_eq!(shared.readings[0].percent(), Some(59));
                assert_eq!(shared.readings[0].key, key);
                assert!(shared.devices[&key].basic.as_ref().unwrap().target.same_channel(&recovered.target));
            }
        }
    }

    #[test]
    fn rapoo_cached_endpoint_never_restores_revoked_writes_or_survives_removal() {
        let receiver = rapoo_endpoint(0x1415, "receiver", "pair", true);
        let wired = rapoo_endpoint(0x4615, "wired", "pair", true);
        let now = std::time::Instant::now();
        let mut shared = Shared::new(Settings::default());
        shared.source_discoveries.insert(BackendId::Rapoo, crate::backends::BackendScan {
            targets: vec![receiver.target.clone(), wired.target.clone()],
            presence: rapoo_poll(Vec::new(), &[&receiver, &wired]),
        });
        merge_additional_at(&mut shared, BackendId::Rapoo,
            rapoo_poll(vec![receiver.clone(), wired.clone()], &[&receiver, &wired]), now);
        let key = shared.readings[0].key.clone();
        shared.devices.get_mut(&key).unwrap().basic.as_mut().unwrap().disable_writes();
        let mut offline = receiver.clone();
        offline.connected = false;
        offline.battery = None;
        offline.battery_at = None;
        offline.battery_unavailable = true;
        offline.dpi = Dpi::default();
        offline.polling_rate = Default::default();
        merge_additional_update(&mut shared, BackendId::Rapoo,
            rapoo_poll(vec![offline.clone()], &[&receiver, &wired]),
            now + Duration::from_secs(1), Publication::Battery);
        let current = &shared.devices[&key];
        assert!(current.reading.connected);
        assert_eq!(current.full_at, now);
        assert_eq!(shared.readings.len(), 1);
        let basic = current.basic.as_ref().unwrap();
        assert!(!basic.dpi.stage_writable && !basic.dpi.stage_editable && !basic.polling_rate.writable);
        assert_eq!(basic.battery_at, wired.battery_at);
        merge_additional_update(&mut shared, BackendId::Rapoo,
            rapoo_poll(Vec::new(), &[&receiver]), now + Duration::from_secs(2), Publication::Presence);
        assert_eq!(shared.rapoo_endpoint_snapshots.len(), 1);
        merge_additional_update(&mut shared, BackendId::Rapoo,
            rapoo_poll(vec![offline.clone()], &[&receiver]), now + Duration::from_secs(2), Publication::Battery);
        merge_additional_update(&mut shared, BackendId::Rapoo,
            rapoo_poll(vec![offline], &[&receiver, &wired]),
            now + Duration::from_secs(3), Publication::Battery);
        assert_eq!(shared.readings.len(), 1);
        assert!(!shared.devices[&key].reading.connected);
        assert_eq!(shared.rapoo_endpoint_snapshots.len(), 1);
    }

    #[test]
    fn rapoo_incremental_battery_keeps_paired_configuration_without_refreshing_its_age() {
        let receiver = rapoo_endpoint(0x1415, "receiver", "pair", true);
        let wired = rapoo_endpoint(0x4615, "wired", "pair", true);
        let mut shared = Shared::new(Settings::default());
        merge_additional(&mut shared, BackendId::Rapoo, vec![receiver.clone(), wired.clone()].into());
        let key = shared.readings[0].key.clone();
        let before = shared.devices[&key].clone();
        for endpoint in [&receiver, &wired] {
            let mut light = endpoint.clone();
            light.dpi = Default::default();
            light.polling_rate = Default::default();
            merge_additional_update(&mut shared, BackendId::Rapoo,
                rapoo_poll(vec![light], &[&receiver, &wired]), std::time::Instant::now(), Publication::Battery);
            assert_eq!(shared.devices.len(), 1);
            assert_eq!(shared.readings[0].key, key);
            assert_eq!(shared.devices[&key].full_at, before.full_at);
            let snapshot = shared.devices[&key].basic.as_ref().unwrap();
            assert!(snapshot.target.same_channel(&wired.target));
            assert_eq!(snapshot.dpi, wired.dpi);
            assert_eq!(snapshot.polling_rate, wired.polling_rate);
        }
    }

    #[test]
    fn rapoo_incremental_offline_receiver_preserves_wired_device_and_unplug_switches_endpoint() {
        let receiver = rapoo_endpoint(0x1415, "receiver", "pair", false);
        let wired = rapoo_endpoint(0x4615, "wired", "pair", true);
        let mut shared = Shared::new(Settings::default());
        merge_additional(&mut shared, BackendId::Rapoo, vec![receiver.clone(), wired.clone()].into());
        let key = shared.readings[0].key.clone();
        merge_additional_update(&mut shared, BackendId::Rapoo,
            rapoo_poll(vec![receiver.clone()], &[&receiver, &wired]), std::time::Instant::now(), Publication::Battery);
        assert_eq!(shared.devices.len(), 1);
        assert!(shared.devices[&key].reading.connected);
        assert_eq!(shared.devices[&key].reading.percent(), Some(58));
        assert!(shared.devices[&key].basic.as_ref().unwrap().dpi.stage_editable);
        let mut live_receiver = rapoo_endpoint(0x1415, "receiver", "pair", true);
        live_receiver.dpi = Default::default();
        live_receiver.polling_rate = Default::default();
        merge_additional_update(&mut shared, BackendId::Rapoo,
            rapoo_poll(vec![live_receiver.clone()], &[&live_receiver]), std::time::Instant::now(), Publication::Battery);
        assert_eq!(shared.devices.len(), 1);
        let snapshot = shared.devices[&key].basic.as_ref().unwrap();
        assert!(snapshot.target.same_channel(&receiver.target));
        assert!(!snapshot.dpi.stage_editable && !snapshot.polling_rate.writable);
    }

    #[test]
    fn rapoo_pair_survives_alternating_read_failures_and_recovery() {
        let receiver = rapoo_endpoint(0x1415, "receiver", "pair", true);
        let wired = rapoo_endpoint(0x4615, "wired", "pair", true);
        let key = receiver.target.key.clone();
        let now = std::time::Instant::now();
        let mut shared = Shared::new(Settings::default());
        for (step, samples) in [
            vec![receiver.clone(), wired.clone()], vec![wired.clone()],
            vec![receiver.clone()], Vec::new(), vec![receiver.clone(), wired.clone()],
        ].into_iter().enumerate() {
            merge_additional_at(&mut shared, BackendId::Rapoo,
                rapoo_poll(samples, &[&receiver, &wired]), now + Duration::from_secs(step as u64));
            assert_eq!(shared.devices.len(), 1);
            assert_eq!(shared.readings[0].key, key);
            assert!(shared.readings[0].connected);
            let basic = shared.devices[&key].basic.as_ref().unwrap();
            assert_eq!(basic.dpi.stage_editable, step != 3);
            assert_eq!(basic.polling_rate.writable, step != 3);
            if step == 1 { assert_eq!(basic.target.channel_key(), "wired"); }
            if step == 2 { assert_eq!(basic.target.channel_key(), "receiver"); }
        }
    }

    #[test]
    fn rapoo_pair_keeps_writable_endpoint_and_configuration_for_each_arrival_order() {
        let receiver = rapoo_endpoint(0x1415, "receiver", "pair", false);
        let wired = rapoo_endpoint(0x4615, "wired", "pair", true);
        for first in [None, Some(receiver.clone()), Some(wired.clone())] {
            let mut shared = Shared::new(Settings::default());
            let key = first.as_ref().unwrap_or(&receiver).target.key.clone();
            if let Some(first) = first { merge_additional(&mut shared, BackendId::Rapoo, vec![first].into()); }
            merge_additional(&mut shared, BackendId::Rapoo, vec![receiver.clone(), wired.clone()].into());
            assert_eq!(shared.devices.len(), 1);
            let basic = shared.devices[&key].basic.as_ref().unwrap();
            assert_eq!(basic.target.key, key);
            assert!(basic.target.same_channel(&wired.target));
            assert_eq!(basic.target.hid().unwrap().product_id, 0x4615);
            assert_eq!(basic.dpi, wired.dpi);
            assert_eq!(basic.polling_rate, wired.polling_rate);
            let command = Cmd::BasicDpiStage {
                target: basic.target.clone(), stage: basic.dpi.stages[0],
                context: basic.dpi.stage_context().unwrap(),
            };
            let Cmd::BasicDpiStage { target, context, .. } = command else { unreachable!() };
            assert!(target.same_channel(&wired.target));
            assert_eq!(context, wired.dpi.stage_context().unwrap());
        }
    }

    #[test]
    fn rapoo_pair_switches_writes_on_unplug_and_never_reuses_failed_endpoint_writes() {
        let receiver = rapoo_endpoint(0x1415, "receiver", "pair", true);
        let wired = rapoo_endpoint(0x4615, "wired", "pair", true);
        let mut shared = Shared::new(Settings::default());
        merge_additional(&mut shared, BackendId::Rapoo, vec![receiver.clone(), wired.clone()].into());
        let key = shared.readings[0].key.clone();
        for endpoint in [&receiver, &wired, &receiver] {
            merge_additional(&mut shared, BackendId::Rapoo, vec![endpoint.clone()].into());
            let basic = shared.devices[&key].basic.as_ref().unwrap();
            assert_eq!(shared.devices.len(), 1);
            assert!(basic.target.same_channel(&endpoint.target));
            assert_eq!(basic.dpi, endpoint.dpi);
        }
        merge_additional(&mut shared, BackendId::Rapoo, rapoo_poll(Vec::new(), &[&wired]));
        assert!(shared.readings[0].connected);
        assert!(!shared.devices[&key].basic.as_ref().unwrap().dpi.stage_editable);
        merge_additional(&mut shared, BackendId::Rapoo, Vec::new().into());
        assert!(!shared.readings[0].connected);
        merge_additional(&mut shared, BackendId::Rapoo, vec![wired.clone()].into());
        assert_eq!(shared.devices.len(), 1);
        assert_eq!(shared.devices[&key].basic.as_ref().unwrap().dpi, wired.dpi);
    }

    #[test]
    fn rapoo_offline_receiver_does_not_clear_failed_wired_sample() {
        let receiver = rapoo_endpoint(0x1415, "receiver", "pair", false);
        let wired = rapoo_endpoint(0x4615, "wired", "pair", true);
        let mut shared = Shared::new(Settings::default());
        merge_additional(&mut shared, BackendId::Rapoo, vec![receiver.clone(), wired.clone()].into());
        let key = shared.readings[0].key.clone();
        let measured = shared.devices[&key].battery_at;
        merge_additional(&mut shared, BackendId::Rapoo, rapoo_poll(vec![receiver.clone()], &[&receiver, &wired]));
        assert_eq!(shared.devices.len(), 1);
        assert!(shared.readings[0].connected);
        assert_eq!(shared.devices[&key].battery_at, measured);
        let basic = shared.devices[&key].basic.as_ref().unwrap();
        assert!(basic.target.same_channel(&wired.target));
        assert!(!basic.dpi.stage_editable);
        assert!(!basic.polling_rate.writable);
        merge_additional(&mut shared, BackendId::Rapoo, vec![receiver].into());
        assert!(!shared.readings[0].connected);
    }

    #[test]
    fn rapoo_unknown_or_different_serial_never_inherits_preferences_or_history() {
        for (old_serial, new_serial) in [("known", ""), ("", "known"), ("", ""), ("known", "other"), (" ", " ")] {
            let old = rapoo_endpoint(0x1415, "old", old_serial, true);
            let next = rapoo_endpoint(0x1415, "next", new_serial, true);
            let old_key = old.target.key.clone();
            let next_key = next.target.key.clone();
            let mut shared = Shared::new(Settings::default());
            shared.cfg.device_names.insert(old_key.clone(), "Mouse A".into());
            shared.cfg.device_order = vec![old_key.clone()];
            shared.cfg.device_tray.insert(old_key.clone(), Default::default());
            shared.cfg.tray_device = Some(old_key.clone());
            shared.cfg.dpi_presets.insert(old_key.clone(), Default::default());
            merge_additional(&mut shared, BackendId::Rapoo, vec![old].into());
            shared.history_tracker.take_pending();
            merge_additional(&mut shared, BackendId::Rapoo, vec![next].into());
            assert_eq!(shared.devices.len(), 2);
            assert_eq!(shared.cfg.device_names[&old_key], "Mouse A");
            assert_eq!(shared.cfg.device_order, vec![old_key.clone()]);
            assert_eq!(shared.cfg.tray_device.as_ref(), Some(&old_key));
            assert!(!shared.cfg.device_names.contains_key(&next_key));
            assert!(!shared.cfg.device_tray.contains_key(&next_key));
            assert!(!shared.cfg.dpi_presets.contains_key(&next_key));
            assert!(!shared.key_aliases.contains_key(&old_key));
            assert!(shared.history_tracker.take_pending().1.is_empty());
        }
    }

    #[test]
    fn rapoo_unknown_serial_never_pairs_wired_and_receiver() {
        for (receiver_serial, wired_serial) in [("known", ""), ("", "known"), ("", "")] {
            let receiver = rapoo_endpoint(0x1415, "receiver", receiver_serial, false);
            let wired = rapoo_endpoint(0x4615, "wired", wired_serial, true);
            let mut shared = Shared::new(Settings::default());
            merge_additional(&mut shared, BackendId::Rapoo, vec![receiver, wired].into());
            assert_eq!(shared.devices.len(), 2);
            assert!(shared.key_aliases.is_empty());
        }
    }

    #[test]
    fn rapoo_identical_receivers_block_pairing_even_after_one_disappears() {
        let first = rapoo_endpoint(0x1415, "first", "duplicate", true);
        let second = rapoo_endpoint(0x1415, "second", "duplicate", true);
        let wired = rapoo_endpoint(0x4615, "wired", "duplicate", true);
        let mut shared = Shared::new(Settings::default());
        merge_additional(&mut shared, BackendId::Rapoo, vec![first.clone(), second.clone()].into());
        merge_additional(&mut shared, BackendId::Rapoo,
            rapoo_poll(vec![first.clone(), wired.clone()], &[&first, &second, &wired]));
        assert_eq!(shared.devices.len(), 3);
        merge_additional(&mut shared, BackendId::Rapoo, vec![first.clone(), wired.clone()].into());
        assert_eq!(shared.devices.len(), 3);
        assert!(shared.key_aliases.is_empty());
        let moved = rapoo_endpoint(0x1415, "moved", "duplicate", true);
        merge_additional(&mut shared, BackendId::Rapoo, vec![moved].into());
        assert_eq!(shared.devices.len(), 4);
        assert!(shared.key_aliases.is_empty());
    }

    #[test]
    fn rapoo_unread_second_receiver_blocks_first_pairing() {
        let first = rapoo_endpoint(0x1415, "first", "duplicate", true);
        let second = rapoo_endpoint(0x1415, "second", "duplicate", true);
        let wired = rapoo_endpoint(0x4615, "wired", "duplicate", true);
        let mut shared = Shared::new(Settings::default());
        merge_additional(&mut shared, BackendId::Rapoo,
            rapoo_poll(vec![first.clone(), wired.clone()], &[&first, &second, &wired]));
        assert_eq!(shared.devices.len(), 2);
        assert!(shared.key_aliases.is_empty());
        merge_additional(&mut shared, BackendId::Rapoo, vec![first, second, wired].into());
        assert_eq!(shared.devices.len(), 3);
        assert!(shared.key_aliases.is_empty());
    }

    #[test]
    fn rapoo_pair_separates_when_second_identical_receiver_arrives() {
        let receiver = rapoo_endpoint(0x1415, "receiver", "pair", true);
        let wired = rapoo_endpoint(0x4615, "wired", "pair", true);
        let other = rapoo_endpoint(0x1415, "other", "pair", true);
        let key = receiver.target.key.clone();
        let mut shared = Shared::new(Settings::default());
        shared.cfg.device_names.insert(key.clone(), "Receiver A".into());
        merge_additional(&mut shared, BackendId::Rapoo, vec![receiver.clone(), wired.clone()].into());
        shared.histories.insert(key.clone(), crate::history::HistoryView { rows: 7, ..Default::default() });
        merge_additional(&mut shared, BackendId::Rapoo,
            vec![receiver.clone(), wired.clone(), other.clone()].into());
        let wired_key = shared.rapoo_devices.card_key(&wired.target).unwrap().to_string();
        assert_eq!(shared.devices.len(), 3);
        assert_ne!(wired_key, wired.target.key);
        assert_ne!(wired_key, key);
        assert!(shared.devices.contains_key(&key));
        assert!(shared.devices.contains_key(&other.target.key));
        assert_eq!(shared.devices[&wired_key].basic.as_ref().unwrap().dpi, wired.dpi);
        assert_eq!(shared.devices[&key].basic.as_ref().unwrap().dpi, receiver.dpi);
        assert_eq!(shared.cfg.device_names[&key], "Receiver A");
        assert!(!shared.cfg.device_names.contains_key(&wired_key));
        assert_eq!(shared.histories[&key].rows, 7);
        assert!(!shared.histories.contains_key(&wired_key));
        assert!(!shared.key_aliases.contains_key(&wired.target.key));
        assert!(!shared.key_aliases.contains_key(&other.target.key));
        assert!(shared.history_tracker.take_pending().1.is_empty());
        merge_additional(&mut shared, BackendId::Rapoo, vec![receiver, wired.clone()].into());
        assert_eq!(shared.devices.len(), 3);
        assert_eq!(shared.rapoo_devices.card_key(&wired.target), Some(wired_key.as_str()));
    }

    #[test]
    fn rapoo_unread_second_receiver_revokes_pair_and_discards_misowned_cache() {
        let receiver = rapoo_endpoint(0x1415, "receiver", "pair", false);
        let wired = rapoo_endpoint(0x4615, "wired", "pair", true);
        let other = rapoo_endpoint(0x1415, "other", "pair", true);
        let key = receiver.target.key.clone();
        let mut shared = Shared::new(Settings::default());
        shared.active_key = Some(key.clone());
        merge_additional(&mut shared, BackendId::Rapoo, vec![receiver.clone(), wired.clone()].into());
        assert!(shared.devices[&key].basic.as_ref().unwrap().target.same_channel(&wired.target));
        merge_additional(&mut shared, BackendId::Rapoo,
            rapoo_poll(vec![wired.clone()], &[&receiver, &wired, &other]));
        let wired_key = shared.rapoo_devices.card_key(&wired.target).unwrap().to_string();
        assert_eq!(shared.devices.len(), 1);
        assert_eq!(shared.active_key.as_ref(), Some(&key));
        assert!(!shared.devices.contains_key(&key));
        assert!(!shared.devices.contains_key(&other.target.key));
        assert!(!shared.readings.iter().any(|reading| reading.key == key));
        assert_eq!(shared.devices[&wired_key].basic.as_ref().unwrap().dpi, wired.dpi);
        assert!(shared.key_aliases.is_empty());
        assert!(shared.history_tracker.take_pending().1.is_empty());
        merge_additional(&mut shared, BackendId::Rapoo, vec![receiver, wired.clone(), other].into());
        assert_eq!(shared.devices.len(), 3);
        assert_eq!(shared.rapoo_devices.card_key(&wired.target), Some(wired_key.as_str()));
    }

    #[test]
    fn rapoo_separation_without_selected_endpoint_sample_never_reuses_its_configuration() {
        let receiver = rapoo_endpoint(0x1415, "receiver", "pair", false);
        let wired = rapoo_endpoint(0x4615, "wired", "pair", true);
        let other = rapoo_endpoint(0x1415, "other", "pair", true);
        let key = receiver.target.key.clone();
        let mut shared = Shared::new(Settings::default());
        shared.active_key = Some(key.clone());
        merge_additional(&mut shared, BackendId::Rapoo, vec![receiver.clone(), wired.clone()].into());
        merge_additional(&mut shared, BackendId::Rapoo,
            rapoo_poll(vec![other.clone()], &[&receiver, &wired, &other]));
        let wired_key = shared.rapoo_devices.card_key(&wired.target).unwrap().to_string();
        assert!(!shared.devices.contains_key(&key));
        assert!(!shared.devices.contains_key(&wired_key));
        assert_eq!(shared.devices.len(), 1);
        let mut partial_receiver = receiver.clone();
        partial_receiver.connected = true;
        partial_receiver.dpi = Dpi::default();
        partial_receiver.polling_rate = Default::default();
        merge_additional(&mut shared, BackendId::Rapoo,
            rapoo_poll(vec![partial_receiver, other.clone()], &[&receiver, &wired, &other]));
        let basic = shared.devices[&key].basic.as_ref().unwrap();
        assert!(basic.target.same_channel(&receiver.target));
        assert_eq!(basic.dpi, Dpi::default());
        assert_eq!(basic.polling_rate, crate::device::PollingRate::default());
        assert!(shared.devices[&key].reading.battery.is_none());
        assert!(!shared.devices.contains_key(&wired_key));
        assert_eq!(shared.active_key.as_ref(), Some(&key));
        merge_additional(&mut shared, BackendId::Rapoo, vec![receiver, wired.clone(), other].into());
        assert_eq!(shared.devices.len(), 3);
        assert_eq!(shared.devices[&wired_key].basic.as_ref().unwrap().dpi, wired.dpi);
    }

    #[test]
    fn rapoo_old_and_new_ports_returning_together_revoke_the_port_alias() {
        let old = rapoo_endpoint(0x1415, "old-port", "same", true);
        let moved = rapoo_endpoint(0x1415, "new-port", "same", true);
        let key = old.target.key.clone();
        let mut shared = Shared::new(Settings::default());
        merge_additional(&mut shared, BackendId::Rapoo, vec![old.clone()].into());
        merge_additional(&mut shared, BackendId::Rapoo, vec![moved.clone()].into());
        assert_eq!(shared.devices.len(), 1);
        assert_eq!(shared.key_aliases.get(&moved.target.key), Some(&key));
        merge_additional(&mut shared, BackendId::Rapoo, rapoo_poll(vec![moved.clone()], &[&old, &moved]));
        let moved_key = shared.rapoo_devices.card_key(&moved.target).unwrap().to_string();
        assert_ne!(moved_key, moved.target.key);
        assert_ne!(moved_key, key);
        assert!(!shared.devices.contains_key(&key));
        assert!(shared.devices.contains_key(&moved_key));
        assert!(shared.key_aliases.is_empty());
        assert!(shared.history_tracker.take_pending().1.is_empty());
        merge_additional(&mut shared, BackendId::Rapoo, vec![old.clone(), moved.clone()].into());
        assert_eq!(shared.devices.len(), 2);
        assert!(shared.devices[&key].basic.as_ref().unwrap().target.same_channel(&old.target));
        assert!(shared.devices[&moved_key].basic.as_ref().unwrap().target.same_channel(&moved.target));
        merge_additional(&mut shared, BackendId::Rapoo, vec![moved.clone()].into());
        assert_eq!(shared.devices.len(), 2);
        assert_eq!(shared.rapoo_devices.card_key(&moved.target), Some(moved_key.as_str()));
    }

    #[test]
    fn rapoo_detached_samples_use_fresh_keys_despite_in_flight_history_aliases() {
        let receiver = rapoo_endpoint(0x1415, "receiver", "pair", false);
        let wired = rapoo_endpoint(0x4615, "wired", "pair", true);
        let other = rapoo_endpoint(0x1415, "other", "pair", true);
        let key = receiver.target.key.clone();
        let mut shared = Shared::new(Settings::default());
        merge_additional(&mut shared, BackendId::Rapoo, vec![receiver.clone(), wired.clone()].into());
        let (old_samples, in_flight_aliases) = shared.history_tracker.take_pending();
        assert!(in_flight_aliases.contains(&(wired.target.key.clone(), key.clone())));
        merge_additional(&mut shared, BackendId::Rapoo,
            rapoo_poll(vec![wired.clone(), other.clone()], &[&receiver, &wired, &other]));
        let independent = shared.rapoo_devices.card_key(&wired.target).unwrap().to_string();
        assert_ne!(independent, wired.target.key);
        assert_ne!(independent, key);
        assert!(!in_flight_aliases.iter().any(|(from, to)| from == &independent || to == &independent));
        let (new_samples, new_aliases) = shared.history_tracker.take_pending();
        assert!(new_aliases.is_empty());
        assert!(new_samples.iter().any(|(sample_key, _)| sample_key == &independent));
        shared.history_tracker.retry(old_samples, in_flight_aliases);
        shared.history_tracker.retry(new_samples, new_aliases);
        let (retried_samples, retried_aliases) = shared.history_tracker.take_pending();
        assert!(retried_samples.iter().any(|(sample_key, _)| sample_key == &independent));
        assert!(!retried_aliases.iter().any(|(from, to)| from == &independent || to == &independent));
        merge_additional(&mut shared, BackendId::Rapoo, vec![receiver, wired.clone(), other].into());
        assert_eq!(shared.rapoo_devices.card_key(&wired.target), Some(independent.as_str()));
    }

    #[test]
    fn rapoo_incomplete_discovery_does_not_permanently_mark_a_unique_pair_ambiguous() {
        let receiver = rapoo_endpoint(0x1415, "receiver", "pair", true);
        let wired = rapoo_endpoint(0x4615, "wired", "pair", true);
        let mut shared = Shared::new(Settings::default());
        merge_additional(&mut shared, BackendId::Rapoo, vec![receiver.clone(), wired.clone()].into());
        let mut incomplete = rapoo_poll(vec![receiver.clone(), wired.clone()], &[&receiver, &wired]);
        incomplete.legacy_key_migrations.clear();
        merge_additional(&mut shared, BackendId::Rapoo, incomplete);
        assert_eq!(shared.devices.len(), 2);
        merge_additional(&mut shared, BackendId::Rapoo, vec![receiver.clone(), wired.clone()].into());
        assert_eq!(shared.devices.len(), 1);
        assert_eq!(shared.rapoo_devices.card_key(&receiver.target), shared.rapoo_devices.card_key(&wired.target));
        assert!(shared.readings[0].connected);
    }

    #[test]
    fn rapoo_separation_cancels_only_queued_commands_with_changed_endpoint_ownership() {
        let receiver = rapoo_endpoint(0x1415, "receiver", "pair", true);
        let wired = rapoo_endpoint(0x4615, "wired", "pair", true);
        let other = rapoo_endpoint(0x1415, "other", "pair", true);
        let key = receiver.target.key.clone();
        let mut shared = Shared::new(Settings::default());
        merge_additional(&mut shared, BackendId::Rapoo, vec![receiver.clone(), wired.clone()].into());
        let selected = shared.devices[&key].basic.as_ref().unwrap();
        assert!(selected.target.same_channel(&wired.target));
        let stale_stage = Cmd::BasicDpiStage {
            target: selected.target.clone(), stage: selected.dpi.stages[0],
            context: selected.dpi.stage_context().unwrap(),
        };
        let stale_rate = Cmd::BasicPollingRate { target: selected.target.clone(), hz: 500 };
        let cancelled_labels = vec![stale_stage.label(), stale_rate.label()];
        let anchor = Cmd::BasicDpiStage {
            target: receiver.target.clone(), stage: receiver.dpi.stages[0],
            context: receiver.dpi.stage_context().unwrap(),
        };
        let logitech = snapshot(BackendId::Logitech, "logitech", 70);
        shared.source_pending.insert(BackendId::Rapoo, [stale_stage, anchor, stale_rate].into());
        shared.source_pending.insert(BackendId::Logitech,
            [Cmd::BasicDpi { target: logitech.target.clone(), dpi: 1600 }].into());
        shared.source_write_gen.insert(BackendId::Rapoo, 3);
        merge_additional(&mut shared, BackendId::Rapoo,
            vec![receiver.clone(), wired.clone(), other].into());
        let remaining = &shared.source_pending[&BackendId::Rapoo];
        assert_eq!(remaining.len(), 1);
        let target = remaining[0].basic_target().unwrap();
        assert_eq!(target.key, key);
        assert!(target.same_channel(&receiver.target));
        assert_eq!(shared.source_pending[&BackendId::Logitech].len(), 1);
        assert!(shared.source_pending[&BackendId::Logitech][0].basic_target().unwrap().same_channel(&logitech.target));
        assert_eq!(shared.source_write_gen[&BackendId::Rapoo], 3);
        assert_eq!(shared.results.iter().map(|result| result.label.clone()).collect::<Vec<_>>(), cancelled_labels);
        assert!(shared.results.iter().all(|result| result.ok.is_none()
            && result.err.as_deref() == Some("设备关联已变化，请重新选择后重试") && result.hint.is_empty()));
    }

    #[test]
    fn rapoo_two_distinct_pairs_keep_endpoints_and_histories_separate() {
        let first = rapoo_endpoint(0x1415, "first", "one", true);
        let first_wired = rapoo_endpoint(0x4615, "first-wired", "one", true);
        let second = rapoo_endpoint(0x1415, "second", "two", true);
        let second_wired = rapoo_endpoint(0x4615, "second-wired", "two", true);
        let mut shared = Shared::new(Settings::default());
        merge_additional(&mut shared, BackendId::Rapoo,
            vec![first.clone(), first_wired.clone(), second.clone(), second_wired.clone()].into());
        merge_additional(&mut shared, BackendId::Rapoo, rapoo_poll(vec![first_wired.clone(), second.clone()],
            &[&first, &first_wired, &second, &second_wired]));
        assert_eq!(shared.devices.len(), 2);
        assert!(shared.devices[&first.target.key].basic.as_ref().unwrap().target.same_channel(&first_wired.target));
        assert!(shared.devices[&second.target.key].basic.as_ref().unwrap().target.same_channel(&second.target));
        let aliases = shared.history_tracker.take_pending().1;
        assert!(aliases.contains(&(first_wired.target.key, first.target.key)));
        assert!(aliases.contains(&(second_wired.target.key, second.target.key)));
        assert_eq!(aliases.len(), 2);
    }

    #[test]
    fn rapoo_port_move_preserves_card_selection_preferences_and_history() {
        let old = rapoo_endpoint(0x1415, "old", "unique", true);
        let moved = rapoo_endpoint(0x1415, "moved", "unique", true);
        let key = old.target.key.clone();
        let mut shared = Shared::new(Settings::default());
        shared.active_key = Some(key.clone());
        shared.cfg.device_names.insert(key.clone(), "My mouse".into());
        shared.cfg.tray_device = Some(key.clone());
        shared.cfg.device_order = vec![key.clone()];
        merge_additional(&mut shared, BackendId::Rapoo, vec![old.clone()].into());
        shared.history_tracker.take_pending();
        merge_additional(&mut shared, BackendId::Rapoo, vec![moved.clone()].into());
        assert_eq!(shared.devices.len(), 1);
        assert_eq!(shared.active_key.as_ref(), Some(&key));
        assert_eq!(shared.cfg.device_names[&key], "My mouse");
        assert_eq!(shared.cfg.tray_device.as_ref(), Some(&key));
        assert_eq!(shared.cfg.device_order, vec![key.clone()]);
        assert!(shared.devices[&key].basic.as_ref().unwrap().target.same_channel(&moved.target));
        assert!(shared.history_tracker.take_pending().1.contains(&(moved.target.key.clone(), key.clone())));
        merge_additional(&mut shared, BackendId::Rapoo, vec![old].into());
        assert_eq!(shared.devices.len(), 1);
        assert!(!shared.key_aliases.contains_key(&key));
    }

    #[test]
    fn rapoo_enumeration_failure_preserves_pair_but_cannot_infer_port_move() {
        let receiver = rapoo_endpoint(0x1415, "receiver", "pair", true);
        let wired = rapoo_endpoint(0x4615, "wired", "pair", true);
        let mut shared = Shared::new(Settings::default());
        merge_additional(&mut shared, BackendId::Rapoo, vec![receiver.clone(), wired.clone()].into());
        merge_additional(&mut shared, BackendId::Rapoo, crate::backends::Poll::default());
        assert_eq!(shared.devices.len(), 1);
        assert!(shared.readings[0].connected);
        assert!(!shared.devices[&receiver.target.key].basic.as_ref().unwrap().dpi.stage_editable);
        merge_additional(&mut shared, BackendId::Rapoo, crate::backends::Poll {
            snapshots: vec![wired], ..Default::default()
        });
        assert_eq!(shared.devices.len(), 1);
        let moved = rapoo_endpoint(0x1415, "moved", "pair", true);
        merge_additional(&mut shared, BackendId::Rapoo, crate::backends::Poll {
            snapshots: vec![moved.clone()], ..Default::default()
        });
        assert_eq!(shared.devices.len(), 2);
        assert!(shared.devices.contains_key(&moved.target.key));
    }

    #[test]
    fn rapoo_reused_path_with_changed_or_unknown_identity_keeps_cards_separate() {
        let old = rapoo_endpoint(0x1415, "same-path", "known", true);
        for serial in ["", "different"] {
            let next = rapoo_endpoint(0x1415, "same-path", serial, true);
            let mut shared = Shared::new(Settings::default());
            shared.cfg.device_names.insert(old.target.key.clone(), "Old mouse".into());
            merge_additional(&mut shared, BackendId::Rapoo, vec![old.clone()].into());
            merge_additional(&mut shared, BackendId::Rapoo, vec![next.clone()].into());
            assert_eq!(shared.devices.len(), 2);
            assert!(!shared.devices[&old.target.key].reading.connected);
            let current = shared.readings.iter().find(|reading| reading.connected).unwrap();
            assert_ne!(current.key, old.target.key);
            assert!(!shared.cfg.device_names.contains_key(&current.key));
            assert!(!shared.key_aliases.contains_key(&old.target.key));
            merge_additional(&mut shared, BackendId::Rapoo, vec![next].into());
            assert_eq!(shared.devices.len(), 2);
        }
    }

    #[test]
    fn rog_read_started_before_a_write_cannot_complete_refresh() {
        let mut state = Shared::new(Settings::default());
        state.write_gen = 1;
        assert!(!state.complete_rog_refresh(0, 2));
        assert_eq!(state.completed_requests[&BackendId::Rog], 0);
        assert!(state.complete_rog_refresh(1, 2));
        assert_eq!(state.completed_requests[&BackendId::Rog], 2);
    }

    #[test]
    fn refresh_finishes_only_after_all_backends_reach_requested_epoch() {
        let mut shared = Shared::new(Settings::default());
        shared.complete_refresh(BackendId::Rog, 3);
        for backend in crate::backends::ADDITIONAL_BACKENDS {
            if backend != BackendId::StandardHid { shared.complete_refresh(backend, 3); }
        }
        assert_eq!(shared.refresh_generation, 0);
        shared.complete_refresh(BackendId::StandardHid, 2);
        assert_eq!(shared.refresh_generation, 2);
        shared.complete_refresh(BackendId::StandardHid, 3);
        shared.complete_refresh(BackendId::Rog, 1);
        assert_eq!(shared.refresh_generation, 3);
    }

    #[test]
    fn refresh_signal_is_observed_by_multiple_subscribers() {
        let signal = Arc::new(Signal::new(60));
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let threads: Vec<_> = (0..2).map(|_| {
            let signal = Arc::clone(&signal);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let mut seen = 0;
                barrier.wait();
                assert!(signal.wait_for_work(&mut seen));
                seen
            })
        }).collect();
        barrier.wait();
        signal.request_refresh();
        for thread in threads { assert_eq!(thread.join().unwrap(), 1); }
    }

    #[test]
    fn unknown_and_coarse_battery_cannot_create_percentage_notifications() {
        let mut reading = DeviceReading { connected: true, ..Default::default() };
        let mut armed = true;
        assert_eq!(low_battery_reading(&reading, &mut armed), None);
        reading.battery = Some(Battery { estimated_percent: Some(20), percent: None, charging: None, voltage_mv: None,
            approximate: true, level: Some(BatteryLevel::Low) });
        assert!(reading.is_low());
        assert_eq!(reading.battery_text(true), "≈20%");
        assert_eq!(low_battery_reading(&reading, &mut armed), None);
        assert!(armed);
        for level in [BatteryLevel::Critical, BatteryLevel::Low, BatteryLevel::Medium,
            BatteryLevel::Good, BatteryLevel::Full] {
            reading.battery.as_mut().unwrap().level = Some(level);
            assert_eq!(reading.is_low(), matches!(level, BatteryLevel::Critical | BatteryLevel::Low));
            assert_eq!(low_battery_reading(&reading, &mut armed), None);
            assert!(armed);
        }
        reading.battery.as_mut().unwrap().percent = Some(80);
        reading.battery.as_mut().unwrap().approximate = false;
        assert!(!reading.is_low());
        reading.battery.as_mut().unwrap().percent = Some(0);
        reading.battery.as_mut().unwrap().approximate = true;
        assert_eq!(low_battery_reading(&reading, &mut armed), None);
        reading.battery.as_mut().unwrap().approximate = false;
        assert_eq!(low_battery_reading(&reading, &mut armed), Some(0));
        assert!(!armed);
        armed = true;
        reading.connected = false;
        assert_eq!(low_battery_reading(&reading, &mut armed), None);
    }
}

#[cfg(test)]
mod tray_click_tests {
    use super::TrayClicks;

    #[test]
    fn single_click_and_duplicate_select_only_toggle_popup_once() {
        let mut clicks = TrayClicks::default();
        clicks.press(false);
        assert!(clicks.release(1000, false));
        assert!(!clicks.release(1001, false));
        assert_eq!(clicks.finish_single(), Some(false));
        assert_eq!(clicks.finish_single(), None);
        assert!(!clicks.double_click(1200, 500));
    }

    #[test]
    fn double_notification_without_first_local_click_falls_back_to_single() {
        let mut clicks = TrayClicks::default();
        assert!(!clicks.double_click(1000, 500));
        clicks.press(false);
        assert!(clicks.release(1010, false));
        assert_eq!(clicks.finish_single(), Some(false));
    }

    #[test]
    fn genuine_double_click_consumes_pending_single_and_trailing_releases() {
        let mut clicks = TrayClicks::default();
        clicks.press(false);
        assert!(clicks.release(1000, false));
        assert!(clicks.double_click(1100, 500));
        assert!(!clicks.double_click(1101, 500));
        assert!(!clicks.release(1110, false));
        assert_eq!(clicks.finish_single(), None);
    }

    #[test]
    fn double_click_does_not_reopen_popup_on_release() {
        let mut clicks = TrayClicks::default();
        clicks.press(false);
        assert!(clicks.release(1000, false));
        assert!(clicks.double_click(1100, 500));
        assert_eq!(clicks.finish_single(), None);
        assert!(!clicks.release(1110, false));
        assert!(!clicks.release(1111, false));
        clicks.press(false);
        assert!(clicks.release(1300, false));
        assert_eq!(clicks.finish_single(), Some(false));
    }

    #[test]
    fn expired_click_and_cancelled_click_cannot_open_main_window() {
        let mut clicks = TrayClicks::default();
        assert!(clicks.release(1000, false));
        assert!(!clicks.double_click(1600, 500));
        assert!(clicks.release(1700, false));
        clicks.cancel();
        assert!(!clicks.double_click(1750, 500));
        assert_eq!(clicks.finish_single(), None);
    }

    #[test]
    fn popup_open_state_is_captured_before_deactivation() {
        let mut clicks = TrayClicks::default();
        clicks.press(true);
        assert!(clicks.release(1000, false));
        assert_eq!(clicks.finish_single(), Some(true));
    }

    #[test]
    fn system_message_timestamp_wrap_keeps_double_click_valid() {
        let mut clicks = TrayClicks::default();
        assert!(clicks.release(u32::MAX - 20, false));
        assert!(clicks.double_click(20, 500));
    }
}
