
use std::cell::Cell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use native_windows_gui as nwg;

use crate::commands::{run_cmd, run_file_cmd};
use crate::hid::{self, HidDevice};
use crate::proto;
use crate::settings::{self, Settings};
use crate::snapshot::{self, FetchOutcome};
use crate::ui::canvas::{self, Canvas};
use crate::ui::widget::{Act, Drop, Edit, Page, Pref, Slide};
use crate::win32;

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
pub const WINDOW_TITLE: &str = "R Battery";

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
    poll_code: u8,
    active_stage: u8,
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
    /// 结果行 / 提示里用的中文短名。
    pub(crate) fn label(&self) -> String {
        match self {
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

#[derive(Clone)]
struct DeviceSnapshot {
    reading: crate::device::DeviceReading,
    state: Option<proto::State>,
    ident: proto::Identity,
    link: Option<hid::Link>,
    full_at: std::time::Instant,
}

fn refresh_devices(shared: &Arc<Mutex<Shared>>, full: bool, generation: u64) -> Option<Quick> {
    let links = crate::win32::HidApi::load().and_then(|api| hid::open_links(&api));
    let links = match links {
        Ok(links) => links,
        Err(e) => {
            let mut s = lock_shared(shared);
            for device in s.devices.values_mut() {
                device.reading.connected = false;
                device.state = None;
            }
            for reading in &mut s.readings {
                reading.connected = false;
            }
            s.quick = None;
            s.full = None;
            s.status = Status::Error(e);
            return None;
        }
    };
    let (mut records, active, mut primary, ttl) = {
        let s = lock_shared(shared);
        (
            s.devices.clone(),
            s.active_key.clone(),
            s.primary_key.clone(),
            Duration::from_secs(s.cfg.cache_ttl_sec),
        )
    };
    for record in records.values_mut() {
        record.reading.connected = false;
    }
    let mut sample = None;
    let mut seen = std::collections::BTreeSet::new();
    for link in links {
        let link_key = link.device_key();
        let previous = records
            .values()
            .find(|d| d.link.as_ref().is_some_and(|l| l.device_key() == link_key))
            .cloned();
        let needs_full = previous.as_ref().is_none_or(|d| d.state.is_none())
            || (full
                && previous
                    .as_ref()
                    .is_some_and(|d| d.full_at.elapsed() >= ttl)
                && active
                    .as_ref()
                    .is_none_or(|k| previous.as_ref().is_some_and(|d| &d.reading.key == k)));
        let out = match HidDevice::open(&link.iface) {
            Ok(mut dev) => {
                if needs_full {
                    snapshot::fetch(&mut dev, &link.all_interfaces, link.report_id, &link.mode)
                } else {
                    snapshot::fetch_light(&mut dev, link.report_id, &link.mode)
                }
            }
            Err(_) => continue,
        };
        let is_connected = out.snapshot.frames.contains_key("12:07:00");
        let mut ident = if needs_full {
            out.snapshot.identity.clone()
        } else {
            previous
                .as_ref()
                .map(|d| d.ident.clone())
                .unwrap_or_default()
        };
        if ident.model_name.is_empty() {
            ident.model_name = link.iface.product.clone();
        }
        let key = if !ident.mouse_sn.trim().is_empty() {
            crate::device::stable_key(&format!("device:{}", ident.mouse_sn.trim()))
        } else if !ident.mouse_mac.trim().is_empty() {
            crate::device::stable_key(&format!("device:{}", ident.mouse_mac.trim()))
        } else {
            previous
                .as_ref()
                .map(|d| d.reading.key.clone())
                .unwrap_or(link_key)
        };
        if let Some(old) = previous.as_ref().filter(|old| {
            old.reading.key != key
                && old.ident.mouse_sn.trim().is_empty()
                && old.ident.mouse_mac.trim().is_empty()
        }) {
            let mut s = lock_shared(shared);
            if outcome_is_stale(generation, s.write_gen) {
                return None;
            }
            records.remove(&old.reading.key);
            if s.promote_device_key(&old.reading.key, &key) {
                if settings::save(&s.cfg).is_err() {
                    tray_log("device identity preferences save failed");
                }
            }
            primary = s.primary_key.clone();
        }
        if !seen.insert(key.clone()) {
            continue;
        }
        let name = ident.display_name();
        let family = crate::device::family_of(&name);
        let complete = out.snapshot.frames.contains_key("12:04:02")
            && out.snapshot.frames.contains_key("12:05:00")
            && out.snapshot.frames.contains_key("12:03:00");
        let mut state = if needs_full && complete {
            Some(out.snapshot.state.clone())
        } else {
            previous.as_ref().and_then(|d| d.state.clone())
        };
        if let Some(st) = &mut state {
            st.power = out.snapshot.state.power;
            st.charging = out.snapshot.state.charging;
            st.voltage_mv = out.snapshot.state.voltage_mv;
            st.poll_code = out.snapshot.state.poll_code;
            st.active_stage = out.snapshot.state.active_stage;
        }
        let level = if is_connected {
            out.snapshot.state.power
        } else {
            previous.as_ref().map(|d| d.reading.level).unwrap_or(0)
        };
        let record = DeviceSnapshot {
            reading: crate::device::DeviceReading {
                key: key.clone(),
                name: if name.is_empty() {
                    family.noun().to_string()
                } else {
                    name
                },
                family,
                level,
                charging: is_connected && out.snapshot.state.charging != 0,
                connected: is_connected,
            },
            state: if is_connected { state } else { None },
            ident,
            link: Some(link),
            full_at: if needs_full && complete {
                std::time::Instant::now()
            } else {
                previous
                    .as_ref()
                    .map(|d| d.full_at)
                    .unwrap_or_else(std::time::Instant::now)
            },
        };
        {
            let mut s = lock_shared(shared);
            if outcome_is_stale(generation, s.write_gen) {
                return None;
            }
            if primary.as_ref().is_none_or(|k| k == &key)
                && (s.primary_key.is_none() || s.primary_key.as_ref() == Some(&key))
            {
                s.primary_key = Some(key.clone());
                sample = apply_outcome(&out, &mut s, generation);
                s.ident = Some(record.ident.clone());
                s.device_family = family;
            }
        }
        records.insert(key, record);
    }
    let mut s = lock_shared(shared);
    if !outcome_is_stale(generation, s.write_gen) {
        for d in records.values_mut() {
            if !d.reading.connected {
                d.state = None;
            }
        }
        s.readings = records.values().map(|d| d.reading.clone()).collect();
        s.sort_readings();
        s.devices = records;
    }
    sample
}

struct Shared {
    refresh_generation: u64,
    devices: std::collections::BTreeMap<String, DeviceSnapshot>,
    key_aliases: std::collections::BTreeMap<String, String>,
    active_key: Option<String>,
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
    low_alert_armed: bool,
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
    /// 历史样本（由历史线程每 30 秒重读一次）。
    rows: Vec<crate::history::HistoryRow>,
    /// 曲线点集。`None` = 历史线程还没读完第一遍。
    ///
    /// 用 `Arc` 而不是裸 `Plot`：绘制闭包每帧都要把数据拷出来（不能在锁里画），
    /// 48 小时的采样点是上千个，每帧深拷贝一次纯属浪费。
    plot: Option<Arc<crate::chart::Plot>>,
    estimate: crate::history::Estimate,
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
    pending: VecDeque<(Cmd, Option<hid::Link>)>,
    /// 最近一条命令的结果，GUI 线程取走一次就清空。
    last: Option<CmdResult>,
    /// 下一轮要不要读全量。
    want_full: bool,
}

impl Shared {
    fn sort_readings(&mut self) {
        let cfg = &self.cfg;
        self.readings.sort_by(|a, b| {
            (cfg.device_kind(a).order(), cfg.device_name(a), &a.key).cmp(&(
                cfg.device_kind(b).order(),
                cfg.device_name(b),
                &b.key,
            ))
        });
    }

    fn new(cfg: Settings) -> Shared {
        Shared {
            refresh_generation: 0,
            devices: Default::default(),
            key_aliases: Default::default(),
            active_key: None,
            primary_key: None,
            full: None,
            ident: None,
            link_mode: String::new(),
            quick: None,
            last_ok_at: None,
            low_alert_armed: true,
            device_family: crate::device::DeviceFamily::Unknown,
            readings: Vec::new(),
            write_gen: 0,
            status: Status::Idle,
            frames: 0,
            elapsed_ms: 0,
            rows: Vec::new(),
            plot: None,
            estimate: crate::history::Estimate::default(),
            cfg,
            autostart: (crate::autostart::AutoStartState::Unknown, crate::autostart::AutoStartPath::Unknown),
            pending: VecDeque::new(),
            last: None,
            want_full: false,
        }
    }

    // 完整身份晚于首次电量返回时，保留选择和托盘偏好。
    fn promote_device_key(&mut self, old: &str, new: &str) -> bool {
        for key in self.key_aliases.values_mut() {
            if key == old {
                *key = new.to_string();
            }
        }
        self.key_aliases.insert(old.to_string(), new.to_string());
        for selected in [&mut self.active_key, &mut self.primary_key] {
            if selected.as_deref() == Some(old) {
                *selected = Some(new.to_string());
            }
        }
        let tray_changed = self.cfg.tray_device.as_deref() == Some(old);
        if tray_changed {
            self.cfg.tray_device = Some(new.to_string());
        }
        let mut changed = tray_changed;
        if !self.cfg.device_names.contains_key(new) {
            if let Some(name) = self.cfg.device_names.remove(old) {
                self.cfg.device_names.insert(new.to_string(), name);
                changed = true;
            }
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
            st.power = q.level;
            st.charging = if q.charging { 1 } else { 0 };
            st.voltage_mv = q.voltage_mv;
            st.poll_code = q.poll_code;
            st.active_stage = q.active_stage;
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

/// 「有活了」的信号 + 关闭开关。
///
/// 用 `Condvar` + 计数而不是「睡固定时长」：用户点「立即刷新」时能立刻醒，
/// 改刷新间隔也能立刻生效（`interval_ms` 是原子量，改完 `notify_all` 一次）。
struct Signal {
    poke: Mutex<u64>,
    cv: Condvar,
    interval_ms: AtomicU64,
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
            shutdown: AtomicBool::new(false),
        }
    }

    /// 「请求退出」的唯一出口：置标志 + 唤醒后台线程。
    /// 结束消息循环（`nwg::stop_thread_dispatch()`）留给调用点 —— 那是 UI 线程的事。
    fn request_quit(&self) {
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

    /// 等到「有新工作」或「过了一个间隔」。返回 `false` = 该退出了。
    fn wait_for_work(&self, seen: &mut u64) -> bool {
        let interval = self.interval_ms.load(Ordering::Relaxed).max(1000);
        let mut n = lock_poke(&self.poke);
        if *n == *seen {
            let (g, _) = self
                .cv
                .wait_timeout(n, Duration::from_millis(interval))
                .unwrap_or_else(|p| p.into_inner());
            n = g;
        }
        if self.shutdown.load(Ordering::Relaxed) {
            return false;
        }
        *seen = *n;
        true
    }
}

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

// ==================== 界面动作的执行 ====================

/// 等一个后台线程收尾，最多 `budget`。
///
/// 不用裸 `join()`：刷新线程可能正卡在 `record_sample` 的跨进程锁上
/// （超时 3 秒），裸 join 会让「点退出」变成最长 3 秒毫无反应；也不能只
/// `sleep(50ms)` 就走 —— 那会把最后一次采样丢在半路。
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
    let _ = std::fs::write(&path, text);
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
                        "{} · {}%{}",
                        cfg.device_name(r),
                        r.level,
                        if !r.connected {
                            crate::tr!("（上次电量，未连接）")
                        } else if r.charging {
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
                        crate::tr!("R Battery · 打开设备页面").to_string()
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
                    if r.connected { Some(r.level) } else { None },
                    r.charging,
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

/// 处理一个 `Act` 时要用到的一堆句柄。
///
/// 打成一个结构而不是给 `handle_act` 传七个参数：这条链上每加一个能力
/// （比如新加一个「打开日志」按钮要 `ShellExecute`），改签名会波及每一处调用。
struct UiEnv<'a> {
    canvas: &'a Canvas,
    shared: &'a Arc<Mutex<Shared>>,
    signal: &'a Signal,
    window: &'a nwg::Window,
    tray: &'a TrayCtl,
    name_dialog: &'a crate::ui::device_name::Dialog,
}

impl<'a> UiEnv<'a> {
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

    /// 改一项本地设置并立刻落盘，同时让界面用上新值。
    fn set_cfg(&self, f: impl FnOnce(&mut Settings)) {
        let mut shared = lock_shared(self.shared);
        let mut snapshot = shared.cfg.clone();
        f(&mut snapshot);
        if let Err(e) = settings::save(&snapshot) {
            drop(shared);
            self.say(crate::trf!("✘ 保存设置失败：{e}", e = e));
        } else {
            shared.cfg = snapshot;
            shared.sort_readings();
            crate::i18n::apply(shared.cfg.language);
        }
    }

    fn submit(&self, cmd: Cmd) -> bool {
        lock_shared(self.shared).active_key = self.canvas.with_ui(|u| u.selected_device.clone());
        let ok = submit(self.shared, self.signal, cmd);
        if !ok {
            // 以前这里是什么都不说就返回 false —— 用户点了一下，
            // 界面上完全没有变化，只会以为功能坏了。现在至少给一句话。
            self.say(crate::tr!(
                "✘ 操作排不进去了（前面的还没写完），稍等一下再试"
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
        Act::ToggleDeviceTray => {
            if let Some(key) = env.canvas.with_ui(|u| u.selected_device.clone()) {
                let reading = lock_shared(env.shared)
                    .readings
                    .iter()
                    .find(|r| r.key == key)
                    .cloned();
                if let Some(reading) = reading {
                    env.set_cfg(|c| {
                        c.tray_device = Some(key);
                        c.tray_device_name = reading.name;
                        c.tray_enabled = true;
                    });
                    env.canvas.repaint_now();
                }
            }
        }
        Act::None => {}

        Act::Nav(p) => {
            env.canvas.with_ui_mut(|u| {
                u.page = p;
                u.initial_route_done = true;
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
            env.canvas.with_ui_mut(|u| u.show_close_dialog = false);
            env.canvas.repaint_now();

            if remember {
                let mode = if choice == crate::ui::widget::CloseChoice::Minimize {
                    "minimize"
                } else {
                    "quit"
                };
                env.set_cfg(|c| c.close_mode = mode.to_string());
            }

            if choice == crate::ui::widget::CloseChoice::Minimize {
                env.window.set_visible(false);
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

        Act::SetSlide(s, frac) => {
            // 拖拽中只改「这一帧显示什么」，**不写设备**。
            // 写一轮设备几十毫秒，拖一次会积压几十条命令，滑块会慢半拍。
            env.canvas.with_ui_mut(|u| u.set_live_frac(s, frac));
            env.canvas.repaint_now();
        }

        Act::CommitSlide(s) => commit_slide(env, s),

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
        Act::OpenLogFolder => {
            let dir = crate::paths::settings_file()
                .parent()
                .map(|d| d.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            open_in_explorer(env, &dir);
        }

        Act::OpenLightingSettings => {
            if let Err(e) = crate::dynlight::open_settings() {
                env.say(format!("✘ {e}"));
            }
        }

        // 重新探测 LampArray。这里会真的枚举 HID（约几十毫秒），
        // 但只在用户点按钮时发生，不是每帧，也不会在绘制路径上。
        Act::RefreshLighting => match crate::dynlight::refresh() {
            Some(l) => env.say(crate::trf!(
                "✔ 已检测到灯效设备（{} 个灯 · {}）",
                l.lamp_count,
                l.kind_name()
            )),
            None => env.say(crate::tr!("✘ 未检测到 HID LampArray 灯效设备")),
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
            if p.exists() {
                if let Err(e) = std::fs::remove_file(&p) {
                    env.say(crate::trf!("✘ 删除失败：{e}", e = e));
                    return;
                }
            }
            {
                let mut s = lock_shared(env.shared);
                s.rows.clear();
                s.plot = None;
                s.estimate = crate::history::Estimate::default();
            }
            env.say(crate::tr!("✔ 历史记录已清空"));
        }

        Act::OpenRepo => open_url(env, "https://github.com/youmikk/ROG-Battery"),

        Act::OpenStore => open_url(env, "https://apps.microsoft.com/detail/9NRC8HCD4BZ7"),
    }
}

/// 换配色偏好：改 `Ui`、改 `settings.ini`、改标题栏，三处必须同时改。
///
/// `pref` 是 `system` / `light` / `dark`。`system` 时按系统深浅色实时换算。
/// 只改 `Ui` → 重启后配色回到默认；只改 `Settings` → 当次切换不生效。
fn apply_theme_pref(env: &UiEnv, pref: &str) {
    let pref = crate::ui::theme::normalize_pref(pref);
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
    env.set_cfg(|c| c.theme = pref.to_string());
    if let Some(h) = env.window.handle.hwnd() {
        // 必须走 `refresh_titlebar`：DWM 只在非客户区重算时才重新合成标题栏，
        // 光调 `DwmSetWindowAttribute` 在 Win11 上要等用户拖窗口颜色才出现
        // （用户看到的正是「点了开关没反应，于是再点第二次」）。
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
            env.set_cfg(|cfg| {
                cfg.tray_device = Some(reading.key);
                cfg.tray_device_name = reading.name;
            });
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
                env.set_cfg(|cfg| cfg.language = *language);
                env.canvas.with_ui_mut(|ui| ui.clear_status());
                env.canvas.repaint_now();
            }
        }
        Drop::TrayDevice => {}
        Drop::TrayKind => {
            if let Some(key) = env.canvas.with_ui(|u| u.selected_device.clone()) {
                if let Some(kind) = crate::tray_prefs::TrayKind::ALL.get(code) {
                    env.set_cfg(|c| c.device_tray.entry(key).or_default().kind = *kind);
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
            env.set_cfg(|c| c.refresh_interval_sec = code as u64);
            env.signal
                .interval_ms
                .store((code as u64).saturating_mul(1000), Ordering::Relaxed);
            env.signal.request_refresh();
            env.say(crate::trf!("✔ 刷新间隔改为 {code} 秒", code = code));
        }
        Drop::CacheTtl => {
            env.set_cfg(|c| c.cache_ttl_sec = code as u64);
            env.say(crate::trf!(
                "✔ 缓存有效期改为 {}",
                if code == 0 {
                    crate::tr!("每次都重读").to_string()
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
                    crate::tr!("把按键绑成「无功能」"),
                    &crate::trf!("{name} 将被绑成 0000（无功能）。\n\n绑好以后这个键在鼠标上就再也按不出任何作用，要恢复必须重新写一遍完整映射。\n\n确定继续？" , name = name),
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
            env.set_cfg(|c| c.close_mode = mode.to_string());
        }
    }
}

/// 「应用」按钮：读草稿 → 校验 → 提交。
fn apply_edit(env: &UiEnv, e: Edit) {
    let st = env.state();

    match e {
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
    }
}

/// 开关类选项。
fn toggle_pref(env: &UiEnv, p: Pref) {
    let st = env.state();
    match p {
        Pref::TrayEnabled => {
            let on = !env.cfg().tray_enabled;
            env.set_cfg(|c| c.tray_enabled = on);
            env.tray.set_visibility(on);
            // 关掉托盘时如果窗口是隐藏的，用户就再也找不到程序了。
            if !on {
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
        Pref::AutoStart => {
            let on = lock_shared(env.shared).autostart_on();
            if !env.submit(Cmd::AutoStart(!on)) {
                env.say(crate::tr!("✘ 后台正在执行别的操作，这次开机自启没有提交"));
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
        .filters(crate::tr!("rog-mouse-ctl 备份(*.txt)|所有文件(*.*)"))
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
        env.say(crate::tr!("✘ 没拿到选中的文件"));
        return;
    };
    let path = PathBuf::from(item);
    let cmd = if save {
        Cmd::Backup(path)
    } else {
        Cmd::Restore(path)
    };
    if !env.submit(cmd) {
        env.say(crate::tr!("✘ 后台正在执行别的操作，请稍后再试"));
    }
}

/// 用系统 shell 打开一个文件夹（`explorer.exe` 的 spawn 在部分受限环境下
/// 返回 Ok 但窗口并不出现，`ShellExecuteW` 走的是真正的 shell 激活，可靠）。
fn open_in_explorer(env: &UiEnv, dir: &std::path::Path) {
    let _ = crate::paths::ensure_dir(dir);
    if let Err(e) = crate::dynlight::shell_open(&dir.to_string_lossy()) {
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
        env.say(crate::tr!("✘ 没拿到选中的路径"));
        return;
    };
    let new_path = std::path::PathBuf::from(item);
    if new_path == old {
        env.say(crate::tr!("位置没变"));
        return;
    }
    // 旧文件有内容、新位置还是空的 → 搬过去，别让用户丢历史/配置。
    let old_has = std::fs::metadata(&old).map(|m| m.len() > 0).unwrap_or(false);
    let new_empty = std::fs::metadata(&new_path).map(|m| m.len() == 0).unwrap_or(true);
    if let Some(parent) = new_path.parent() {
        let _ = crate::paths::ensure_dir(parent);
    }
    if old_has && new_empty {
        let _ = std::fs::copy(&old, &new_path);
    }
    let res = match kind {
        SavePathKind::History => crate::history::set_custom_history_path(Some(new_path.clone())),
        SavePathKind::Settings => {
            // 先把当前配置写到新位置，再切换指针，避免切换后读到空文件。
            let cur = crate::settings::load();
            let _ = std::fs::write(&new_path, crate::settings::Settings::render(&cur));
            crate::paths::set_custom_settings_file(Some(new_path.clone()))
        }
    };
    match res {
        Ok(()) => {
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
    let res = match kind {
        SavePathKind::History => crate::history::set_custom_history_path(None),
        SavePathKind::Settings => crate::paths::set_custom_settings_file(None),
    };
    match res {
        Ok(()) => {
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
///
/// 只在「接收器都不在」时写：休眠是正常状态，不该反复覆盖文件。
fn write_diagnostics(status: &str, link_mode: &str, family: &str, frames: usize, elapsed_ms: u128) {
    let Some(dir) = crate::paths::settings_file().parent().map(|d| d.to_path_buf()) else {
        return;
    };
    let f = crate::diagnostics::Facts {
        version: env!("CARGO_PKG_VERSION"),
        status,
        link_mode,
        family,
        frames,
        elapsed_ms,
        history_rows: crate::history::read_rows().len(),
    };
    let _ = crate::diagnostics::append(&dir.join("diagnostics.log"), &f);
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
            .unwrap_or_else(|| crate::tr!("没有任何帧有应答").to_string());
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
                write_diagnostics(
                    text,
                    if snap.identity.dongle_sn.trim().is_empty() {
                        ""
                    } else {
                        crate::tr!("2.4G 接收器")
                    },
                    crate::device::family_of(&snap.identity.model_name).noun(),
                    s.frames,
                    s.elapsed_ms,
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

    let have_power = snap.frames.contains_key("12:07:00");
    if !have_power {
        // 保留上一轮的电量显示（比清成「—」更不吓人），但**不产生新采样**。
        s.frames = out.frames_read;
        s.elapsed_ms = out.elapsed.as_millis();
        s.status = Status::Asleep;
        return None;
    }

    let st = &snap.state;
    let q = Quick {
        level: st.power,
        charging: st.charging != 0,
        voltage_mv: st.voltage_mv,
        poll_code: st.poll_code,
        active_stage: st.active_stage,
    };
    s.quick = Some(q);
    s.last_ok_at = Some(std::time::Instant::now());

    // 全量快照才填 `full`：3 帧轻量读取里 DPI 颜色、按键表都是默认值，
    // 拿它刷性能/灯光/按键页会把界面刷成假的（颜色全黑、按键全 0）。
    //
    // 要求 DPI 帧 / 按键帧 / 灯光帧都在 —— 少任何一帧都说明不是全量。
    let is_full = snap.frames.contains_key("12:04:02")
        && snap.frames.contains_key("12:05:00")
        && snap.frames.contains_key("12:03:00");
    if is_full {
        s.device_family = crate::device::family_of(&snap.identity.model_name);
        s.full = Some(st.clone());
        s.ident = Some(snap.identity.clone());
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
    {
        let mut s = lock_shared(shared);
        if s.pending.len() >= MAX_PENDING {
            return false;
        }
        let target = s
            .active_key
            .as_ref()
            .and_then(|k| s.devices.get(k))
            .and_then(|d| d.link.clone());
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
            S::DisabledByUser => crate::trf!("系统记着你之前拒绝过自启，程序再请求会被静默忽略。请到 {where_} 手动把「{WINDOW_TITLE}」打开。" , WINDOW_TITLE = WINDOW_TITLE, where_ = where_),
            S::DisabledByPolicy => {
                crate::tr!("组策略禁止了开机自启，本程序无法绕过。请联系管理员。").to_string()
            }
            S::Disabled => crate::trf!("系统没有接受这次请求，请到 {where_} 手动打开。" , where_ = where_),
            S::Unknown => crate::trf!("读不到自启状态，请到 {where_} 确认。" , where_ = where_),
        }
    } else {
        match state {
            S::Enabled | S::EnabledByPolicy => {
                crate::trf!(
                    "没能关掉（{}），请到 {where_} 手动关闭。",
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

/// 历史文件第四列的链路编码：有线 = 1，2.4G = 2（C# 版就是这个编码）。
fn link_code(s: &Shared) -> i32 {
    match s.link_mode.as_str() {
        "Wired" => 1,
        "Wireless24G" => 2,
        _ => {
            if s.ident.as_ref().map(|i| i.dongle_sn.is_empty()).unwrap_or(true) {
                1
            } else {
                2
            }
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
    const MB_YESNO: u32 = 0x0000_0004;
    const MB_ICONWARNING: u32 = 0x0000_0030;
    const MB_DEFBUTTON2: u32 = 0x0000_0100;
    const IDYES: i32 = 6;

    let t = crate::win32::to_wide(text);
    let c = crate::win32::to_wide(title);
    let r = unsafe {
        crate::win32::MessageBoxW(
            std::ptr::null_mut(),
            t.as_ptr(),
            c.as_ptr(),
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
        )
    };
    r == IDYES
}

// ==================== 入口 ====================

/// 启动 GUI。返回时进程就要退出了。
pub fn run(start_hidden: bool) -> Result<(), String> {
    let cfg = settings::load();
    crate::i18n::apply(cfg.language);
    nwg::init().map_err(|e| crate::trf!("nwg 初始化失败：{e}", e = e))?;
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
        .title(crate::gui::WINDOW_TITLE)
        .size(((WIN_W * scale) as i32, (WIN_H * scale) as i32))
        .center(true)
        // **先隐藏**：全部初始化（读设置/托盘图标/字体/画布/后台线程）都在主线程
        // 同步做完才开消息循环，这期间窗口若可见就是"显示着但不响应"，
        // Explorer 会把鼠标转成转圈。`MAIN_WINDOW` = WS_OVERLAPPEDWINDOW **但不含 WS_VISIBLE**，
        // 初始化完、dispatch 前再 ShowWindow。
        .flags(nwg::WindowFlags::MAIN_WINDOW)
        .build(&mut window)
        .map_err(|e| crate::trf!("建主窗口失败：{e}", e = e))?;

    let hwnd = window
        .handle
        .hwnd()
        .ok_or_else(|| crate::tr!("主窗口没有句柄").to_string())?
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
            .map_err(|e| crate::trf!("绑定客户区重算子类失败：{e}", e = e))?;

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
            .map_err(|e| crate::trf!("绑定最大化尺寸子类失败：{e}", e = e))?;

        let ncact =
            nwg::bind_raw_event_handler(&window.handle, ID_NCACTIVATE, |_h, msg, _wp, _lp| {
                const WM_NCACTIVATE: u32 = 0x0086;
                if msg == WM_NCACTIVATE {
                    // 返回 Some(1) 阻止 DefWindowProcW 绘制系统默认非客户区（经典蓝条标题栏）
                    return Some(1);
                }
                None
            })
            .map_err(|e| crate::trf!("绑定非客户区激活子类失败：{e}", e = e))?;

        let ncpaint =
            nwg::bind_raw_event_handler(&window.handle, ID_NCPAINT, |_h, msg, _wp, _lp| {
                const WM_NCPAINT: u32 = 0x0085;
                if msg == WM_NCPAINT {
                    // 返回 Some(0) 阻止系统绘制默认外边框
                    return Some(0);
                }
                None
            })
            .map_err(|e| crate::trf!("绑定非客户区绘制子类失败：{e}", e = e))?;

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
        .map_err(|e| crate::trf!("建消息窗口失败：{e}", e = e))?;

    let mut menu = nwg::Menu::default();
    nwg::Menu::builder()
        .popup(true)
        .parent(&msg_window)
        .build(&mut menu)
        .map_err(|e| crate::trf!("建托盘菜单失败：{e}", e = e))?;

    // Menu 没有 append()，每个 MenuItem 都要显式 parent。
    //
    // 菜单里只留「退出」：左键单击托盘图标已经是「打开主窗口」，
    // 「显示主窗口 / 立即刷新」两个菜单项就没有存在意义了。
    let mut mi_quit = nwg::MenuItem::default();
    nwg::MenuItem::builder()
        .text(crate::tr!("退出"))
        .parent(&menu)
        .build(&mut mi_quit)
        .map_err(|e| crate::trf!("菜单项：{e}", e = e))?;

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

    let mut notice = nwg::Notice::default();
    nwg::Notice::builder()
        .parent(&window)
        .build(&mut notice)
        .map_err(|e| crate::trf!("建通知通道失败：{e}", e = e))?;

    // ── 共享状态 ────────────────────────────────────────────
    let shared = Arc::new(Mutex::new(Shared::new(cfg.clone())));
    let signal = Arc::new(Signal::new(cfg.refresh_interval_sec));
    let sender = notice.sender();
    let popup = Rc::new(crate::ui::tray_popup::Popup::new(&window, sender)?);
    let name_dialog = crate::ui::device_name::Dialog::new(&window, sender)?;

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
        .map_err(|e| crate::trf!("建自绘内容区失败：{e}", e = e))?;
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
    });

    {
        let shared_p = Arc::clone(&shared);
        canvas.set_painter(Box::new(move |inner, fonts, ui, args| {
            // 只在锁里拷一份数据，**不在锁里画** —— 画一帧要几毫秒，
            // 握着锁会让后台线程写结果时多等这段。
            let (
                mut state,
                mut ident,
                mut link_mode,
                plot,
                estimate,
                cfg,
                autostart_text,
                autostart_on,
                rows_n,
                err_text,
            ) = {
                let s = lock_shared(&shared_p);
                (
                    s.ui_state(),
                    s.ident.clone(),
                    s.link_mode.clone(),
                    s.plot.clone(),
                    s.estimate.clone(),
                    s.cfg.clone(),
                    s.autostart_text(),
                    s.autostart_on(),
                    s.rows.len(),
                    s.error_text(),
                )
            };
            let theme = *ui.mode.theme();
            let readings = {
                let s = lock_shared(&shared_p);
                if let Some(key) = ui.selected_device.as_ref().and_then(|k| s.key_aliases.get(k)) {
                    if s.devices.contains_key(key) {
                        ui.selected_device = Some(key.clone());
                    }
                }
                s.readings.clone()
            };
            ui.sync_devices(&readings);
            let selected = ui
                .selected_device
                .as_ref()
                .and_then(|key| readings.iter().find(|r| &r.key == key));
            let (record, is_primary) = {
                let mut s = lock_shared(&shared_p);
                s.active_key = ui.selected_device.clone();
                (
                    ui.selected_device
                        .as_ref()
                        .and_then(|k| s.devices.get(k))
                        .cloned(),
                    s.primary_key.as_ref() == ui.selected_device.as_ref(),
                )
            };
            if let Some(record) = &record {
                state = record.state.clone();
                ident = Some(record.ident.clone());
                link_mode = record
                    .link
                    .as_ref()
                    .map(|l| l.mode.clone())
                    .unwrap_or_default();
            }
            let empty_estimate = crate::history::Estimate::default();
            let data = crate::ui::viewdata::Data {
                selected,
                state: state.as_ref(),
                ident: ident.as_ref(),
                link_mode: &link_mode,
                // 常驻错误文字（读不到设备的原因 / 正在写设备）。
                // 瞬时提示走 `ui.status`，由 `pages::draw_toast` 优先显示。
                status: err_text,
                plot: if is_primary { plot.as_deref() } else { None },
                estimate: if is_primary {
                    &estimate
                } else {
                    &empty_estimate
                },
                rows: if is_primary { rows_n } else { 0 },
                cfg: &cfg,
                autostart_text: &autostart_text,
                autostart_on,
                version: APP_VERSION,
                readings: &readings,
            };
            canvas::draw_frame(inner, fonts, ui, args, &theme, &data)
        }));
    }

    let history_thread = {
        let shared_c = Arc::clone(&shared);
        let signal_c = Arc::clone(&signal);
        let sender_c = sender;
        std::thread::spawn(move || loop {
            let rows = crate::history::read_rows();
            let plot = crate::chart::build(&rows);
            {
                let mut s = lock_shared(&shared_c);
                s.rows = rows;
                s.plot = Some(Arc::new(plot));
            }
            sender_c.notice();

            for _ in 0..30 {
                if signal_c.shutdown.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(1000));
            }
        })
    };

    // ── 主刷新线程 ──────────────────────────────────────────
    let refresh_thread = {
        let shared_t = Arc::clone(&shared);
        let signal_t = Arc::clone(&signal);
        let sender_t = sender;
        std::thread::spawn(move || {
            let mut seen = 0u64;
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
                    let cmd = { lock_shared(&shared_t).pending.pop_front() };
                    let Some((cmd, target)) = cmd else { break };
                    let res = match &cmd {
                        Cmd::Backup(_) | Cmd::Restore(_) => run_file_cmd(&cmd, target.as_ref()),
                        _ => run_cmd(&cmd, target.as_ref()),
                    };
                    let mut s = lock_shared(&shared_t);
                    if let Some(target) = &target {
                        for device in s.devices.values_mut() {
                            if device
                                .link
                                .as_ref()
                                .is_some_and(|link| link.device_key() == target.device_key())
                            {
                                device.state = None;
                            }
                        }
                    }
                    // 自启命令改的就是「系统里那个开关」，改完必须把新状态
                    // 抄回 `Shared`，否则设置页那一行还停在旧文案上。
                    // 放在这里而不是 `run_autostart` 里：那个函数不持锁、也不该持。
                    if matches!(cmd, Cmd::AutoStart(_)) {
                        s.autostart = crate::autostart::state();
                    }
                    s.last = Some(res);
                    // 写完必须读一次全量：DPI / 灯光 / 按键都可能变了。
                    s.want_full = true;
                    drop(s);
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
                let sample = refresh_devices(&shared_t, full, gen_at_read);
                lock_shared(&shared_t).refresh_generation += 1;

                // ④ 记一条历史采样 + 重算续航估算。
                //
                // `record_sample` 会拿一个 3 秒超时的全局 mutex，**绝不能握着
                // `Shared` 的锁去调** —— 另一个实例正在写的时候这里会白等 3 秒，
                // 期间界面完全刷不出来。
                if let Some(q) = sample {
                    let mode_code = link_code(&lock_shared(&shared_t));
                    let _ = crate::history::record_sample(q.level, q.charging, mode_code);

                    let est = {
                        let s = lock_shared(&shared_t);
                        crate::history::predict_sticky(
                            &s.rows,
                            q.level,
                            q.charging,
                            mode_code,
                            snapshot::now_unix() as i64,
                        )
                    };
                    lock_shared(&shared_t).estimate = est;
                }

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
    let tray_broadcast = Arc::clone(&tray_tb);
    // 第二个参数是**处理器 id**（nwg 要求 > 0xFFFF），要过滤的消息在回调里判。
    let min_handler =
        nwg::bind_raw_event_handler(&window.handle, 0x10006, move |_h, msg, wp, _lp| {
            if taskbar_created != 0 && msg == taskbar_created {
                tray_broadcast.re_add();
            }

            // SIZE_MINIMIZED = 1
            if msg == 0x0005 && wp as usize == 1 {
                canvas_min.release_render_memory();
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
        .map_err(|e| crate::trf!("绑 WM_SIZE 处理器失败：{e}", e = e))?;

    let wh = window.handle.clone();
    let handler = nwg::full_bind_event_handler(&wh, move |evt, evt_data, _handle| {
        use nwg::Event as E;

        match evt {
            E::OnWindowClose => {
                let s = lock_shared(&shared_e);
                let mode = s.cfg.close_mode.clone();
                let tray_on = s.cfg.tray_enabled;
                drop(s);

                if mode == "ask" {
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
                } else if mode == "minimize" && tray_on {
                    // 最小化到托盘
                    if let nwg::EventData::OnWindowClose(d) = evt_data {
                        d.close(false);
                    }
                    window.set_visible(false);
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
                for request in name_dialog.take_requests() {
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
                    }
                    drop(s);
                    name_dialog.finish(result.as_ref().err().map(String::as_str));
                    canvas.repaint_now();
                }
                for action in popup_events.take_actions() {
                    use crate::ui::tray_popup::Action;
                    match action {
                        Action::Refresh => {
                            let generation = lock_shared(&shared_e).refresh_generation;
                            popup_events.refresh_started(generation);
                            signal_e.request_refresh();
                        }
                        Action::Quit => {
                            popup_events.hide();
                            signal_e.request_quit();
                            nwg::stop_thread_dispatch();
                        }
                        Action::Settings | Action::Main | Action::Device(_) => {
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
                                Action::Device(key) => {
                                    canvas.with_ui_mut(|u| u.select_device(&key));
                                    let mut s = lock_shared(&shared_e);
                                    s.active_key = Some(key);
                                    s.want_full = true;
                                    signal_e.request_refresh();
                                }
                                _ => {}
                            }
                            unsafe {
                                win32::ShowWindow(hwnd, 9);
                                win32::SetForegroundWindow(hwnd);
                            }
                            canvas.repaint_now();
                        }
                    }
                }

                // ⓪ 充电动效开关。
                //
                // 判据是「当前页 == 电量页 **且** 设备报告正在充电」。
                // 放在这里而不是 painter 里：`set_animating` 要 `&Canvas`，
                // 而 painter 闭包只有 `Ui` / `Fonts` / `GfxCanvas`。
                // 代价是判断基于上一轮读到的 `state`（最多落后一个刷新间隔），
                // 但动效从「该出现」到「真出现」晚 60 秒用户是感觉不到的。
                let want_anim = {
                    let s = lock_shared(&shared_e);
                    let on_battery = canvas.with_ui(|u| u.page == crate::ui::widget::Page::Battery);
                    let charging = on_battery && s.quick.map(|q| q.charging).unwrap_or(false);
                    let toast_active = canvas.with_ui(|u| !u.status.is_empty());
                    charging || toast_active
                };
                canvas.set_animating(want_anim);

                // ① canvas 攒下的交互动作
                let acts = canvas.take_acts();
                if !acts.is_empty() {
                    let env = UiEnv {
                        canvas: &canvas,
                        shared: &shared_e,
                        signal: &signal_e,
                        window: &window,
                        tray: &tray,
                        name_dialog: &name_dialog,
                    };
                    for a in acts {
                        handle_act(a, &env);
                    }
                    // 用户在动界面：顺手补读一次，读数不该停在几分钟前。
                    signal_e.note_activity();
                }

                // ② 后台回报的命令结果
                let result = { lock_shared(&shared_e).last.take() };
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
                    if let Some(q) = s.quick {
                        if low_battery_alert(q.level, q.charging, &mut s.low_alert_armed) {
                            canvas.with_ui_mut(|u| {
                                u.set_status(crate::tr!("设备电量低，建议充电").to_string())
                            });
                        }
                    }
                    let (readings, cfg, generation) =
                        (s.readings.clone(), s.cfg.clone(), s.refresh_generation);
                    drop(s);
                    popup_events.update(&readings, &cfg, generation);
                    if !cfg.tray_enabled {
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
    let pending_click = Cell::new(false);
    let suppress_release = Cell::new(false);
    let click_was_open = Cell::new(false);
    let anchor = Cell::new(win32::Rect::default());
    let tray_handler = unsafe {
        nwg::bind_raw_event_handler(&msg_window.handle, 0x10005, move |h, msg, wp, lp| {
            let message_hwnd = h as win32::Handle;
            if taskbar_created != 0 && msg == taskbar_created {
                tray_tb.re_add();
                return Some(0);
            }
            if msg == 0x0113 && wp == CLICK_TIMER {
                win32::KillTimer(message_hwnd, CLICK_TIMER);
                if pending_click.replace(false) {
                    if click_was_open.get() {
                        popup_tray.hide();
                    } else {
                        let (readings, cfg, generation) = {
                            let s = lock_shared(&shared_tray);
                            (s.readings.clone(), s.cfg.clone(), s.refresh_generation)
                        };
                        popup_tray.update(&readings, &cfg, generation);
                        if cfg.tray_enabled {
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
                    pending_click.set(false);
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
                    suppress_release.set(false);
                    click_was_open.set(popup_tray.was_open());
                    anchor.set(current);
                }
                0x0203 => {
                    pending_click.set(false);
                    suppress_release.set(true);
                    win32::KillTimer(message_hwnd, CLICK_TIMER);
                    popup_tray.hide();
                    canvas_tray.with_ui_mut(|u| {
                        u.page = Page::Home;
                        u.initial_route_done = true;
                    });
                    win32::ShowWindow(hwnd, 9);
                    win32::SetForegroundWindow(hwnd);
                    canvas_tray.repaint_now();
                }
                0x0202 | 0x0400 => {
                    if !suppress_release.get() && !pending_click.replace(true) {
                        anchor.set(current);
                        win32::SetTimer(message_hwnd, CLICK_TIMER, win32::GetDoubleClickTime(), 0);
                    }
                }
                0x0401 => {
                    pending_click.set(false);
                    win32::KillTimer(message_hwnd, CLICK_TIMER);
                    if popup_tray.was_open() {
                        popup_tray.hide();
                    } else {
                        let (readings, cfg, generation) = {
                            let s = lock_shared(&shared_tray);
                            (s.readings.clone(), s.cfg.clone(), s.refresh_generation)
                        };
                        popup_tray.update(&readings, &cfg, generation);
                        popup_tray.show(current);
                    }
                }
                _ => {}
            }
            Some(0)
        })
    }
    .map_err(|e| crate::trf!("绑托盘回调失败：{e}", e = e))?;

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

    // 初始化全部就绪、消息循环马上开始 —— 这一刻才把窗口显示出来。
    // 若是开机自启拉起（且托盘已开启），就留在托盘后台，不弹窗打扰用户。
    // SW_SHOWNORMAL = 1。
    if !(start_hidden && cfg.tray_enabled) {
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

    // 退出：通知两条后台线程，并等它们真的收尾（`record_sample` 可能在写盘）。
    signal.request_quit();
    join_bounded(history_thread, Duration::from_millis(4000));
    join_bounded(refresh_thread, Duration::from_millis(4000));

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
