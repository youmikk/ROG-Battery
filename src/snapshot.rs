
use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::hid::{HidDevice, Link};
use crate::proto::{self, Identity, State};

/// 默认缓存有效期（秒）。可在 `settings.ini` 里用 `cache_ttl_sec` 覆盖。
pub const DEFAULT_TTL_SEC: u64 = 30;

/// 原始帧及部分解析结果；配置操作须通过 `configuration` 检查完整性。
#[derive(Clone, Default)]
pub struct Snapshot {
    /// 抓取时刻（进程内相对时间，用于判断内存缓存是否过期）
    pub taken: Option<Instant>,
    /// 抓取时刻（wall clock，写入磁盘缓存用）
    pub taken_unix: u64,
    /// 原始帧，key 形如 `12:04:00`（idx 跟在 sub 后面）
    pub frames: BTreeMap<String, Vec<u8>>,
    /// 解析后的状态
    pub state: State,
    /// 解析后的身份
    pub identity: Identity,
    /// 链路模式与 report id（缓存跨进程复用时要还原）
    pub link_mode: String,
    pub report_id: u8,
    pub cache_key: Option<String>,
    /// 解析失败的帧（key -> 原因）。不让一帧读失败就整体失败。
    pub failed: BTreeMap<String, String>,
}

impl Snapshot {
    pub fn parsed_frame(&self, key: &str) -> bool {
        self.frames.contains_key(key) && !self.failed.contains_key(key)
    }

    pub fn configuration(&self) -> Result<Configuration, String> {
        let mut state = State::default();
        for &(key, sub, _, parse) in CONFIGURATION_FRAMES {
            if let Some(reason) = self.failed.get(key) {
                return Err(format!("配置帧 {key} 不可用：{reason}"));
            }
            let frame = self.frames.get(key).ok_or_else(|| format!("缺少配置帧 {key}"))?;
            parse_configuration_frame(self.report_id, key, sub, frame, parse, &mut state)?;
        }
        Ok(Configuration { state })
    }

    pub fn battery(&self) -> Option<crate::device::Battery> {
        if !self.parsed_frame("12:07:00") || self.state.power > 100 { return None; }
        Some(crate::device::Battery {
            estimated_percent: None,
            percent: Some(self.state.power), charging: Some(self.state.charging != 0),
            voltage_mv: Some(self.state.voltage_mv), approximate: false, level: None,
        })
    }

    pub fn age(&self) -> Option<Duration> {
        self.taken.map(|t| t.elapsed())
    }

    pub fn is_fresh(&self, ttl: Duration) -> bool {
        match self.age() {
            Some(a) => a < ttl,
            None => false,
        }
    }

    /// 有帧解析失败时给出人话摘要（GUI 状态栏 / CLI 警告用）。
    pub fn warnings(&self) -> Vec<String> {
        self.failed
            .iter()
            .map(|(k, v)| format!("{k}：{v}"))
            .collect()
    }
}

/// 配置帧全部有效后的只读状态，不能由展示用的默认值直接构造。
pub struct Configuration {
    state: State,
}

impl Configuration {
    pub fn state(&self) -> &State {
        &self.state
    }
}

type StateParser = fn(&[u8], &mut State) -> Result<(), String>;
const CONFIGURATION_FRAMES: &[(&str, u8, &[u8], StateParser)] = &[
    ("12:04:00", 0x04, &[0, 0], proto::parse_perf),
    ("12:04:02", 0x04, &[2, 0], proto::parse_dpi),
    ("12:04:03", 0x04, &[3, 0], proto::parse_colors),
    ("12:00:00", 0x00, &[0, 0], proto::parse_profile),
    ("12:07:00", 0x07, &[0, 0], proto::parse_energy),
    ("12:06:00", 0x06, &[0, 0], proto::parse_lod),
    ("12:03:00", 0x03, &[0, 0], proto::parse_light),
    ("12:05:00", 0x05, &[0, 0], proto::parse_buttons),
];

fn parse_configuration_frame(
    report_id: u8,
    key: &str,
    sub: u8,
    frame: &[u8],
    parse: StateParser,
    state: &mut State,
) -> Result<(), String> {
    if crate::hid::is_nak(frame) {
        return Err(format!("配置帧 {key} 被设备 NAK"));
    }
    if !frame.starts_with(&[report_id, 0x12, sub]) {
        return Err(format!("配置帧 {key} 的应答头不匹配"));
    }
    parse(frame, state).map_err(|error| format!("配置帧 {key} 无效：{error}"))
}

pub fn read_configuration(dev: &mut HidDevice, report_id: u8) -> Result<Configuration, String> {
    read_configuration_with(report_id, |rid, cmd, sub, tail| {
        dev.xfer(rid, cmd, sub, tail, crate::hid::DEFAULT_TIMEOUT_MS)
    })
}

pub(crate) fn read_configuration_with(
    report_id: u8,
    mut ask: impl FnMut(u8, u8, u8, &[u8]) -> Option<Vec<u8>>,
) -> Result<Configuration, String> {
    let mut snapshot = Snapshot { report_id, ..Default::default() };
    for &(key, sub, tail, _) in CONFIGURATION_FRAMES {
        let frame = ask(report_id, 0x12, sub, tail)
            .ok_or_else(|| format!("配置帧 {key} 无应答"))?;
        snapshot.frames.insert(key.into(), frame);
    }
    snapshot.configuration()
}

// ==================== 抓取 ====================

#[cfg(test)]
mod battery_tests {
    use super::*;

    #[test]
    fn invalid_energy_frames_are_unknown_while_valid_zero_is_reported() {
        let mut snapshot = Snapshot::default();
        assert!(snapshot.battery().is_none());
        snapshot.frames.insert("12:07:00".into(), vec![0x12, 7, 0]);
        parse_all(&mut snapshot);
        assert!(snapshot.battery().is_none());
        snapshot.failed.clear();
        snapshot.frames.insert("12:07:00".into(), vec![0x12, 7, 0, 0, 0, 0, 0, 20, 0, 0, 0, 0, 0]);
        parse_all(&mut snapshot);
        assert_eq!(snapshot.battery().unwrap().percent, Some(0));
        snapshot.state.power = 255;
        assert!(snapshot.battery().is_none());
    }
}

/// 要读的帧清单 —— **这就是文档 §0 表里的 17 帧**。
///
/// 顺序刻意与文档表格一致（1→12），方便和 `hid-features.md` 对照。
/// 第 3 项（休眠/低电量阈值）与电量同一帧，零额外成本，所以不计入 17。
///
/// `(key, cmd, sub, tail)`：
/// - 鼠标通道 12 帧 + 身份 8 帧 = 20，但文档把「型号名+有线PID」「双端MAC+无线PID」
///   各算作一组，实际往返数是 17。
const MOUSE_FRAMES: &[(&str, u8, u8, &[u8])] = &[
    // —— §0 表 1-9：鼠标通道 ——
    ("12:04:00", 0x12, 0x04, &[0x00, 0x00]), // 回报率/去抖/角度校正/角度微调
    ("12:06:00", 0x12, 0x06, &[0x00, 0x00]), // LOD 与表面校准
    ("12:07:00", 0x12, 0x07, &[0x00, 0x00]), // 电量 + 休眠 + 低电量阈值 + 充满效果
    ("12:03:00", 0x12, 0x03, &[0x00, 0x00]), // 灯光（zone 0）
    ("12:05:00", 0x12, 0x05, &[0x00, 0x00]), // 8 键映射
    ("12:04:02", 0x12, 0x04, &[0x02, 0x00]), // 4 档 DPI
    ("12:04:03", 0x12, 0x04, &[0x03, 0x00]), // 4 档颜色
    ("12:00:00", 0x12, 0x00, &[0x00, 0x00]), // profile 与当前 DPI 档位
    ("12:04:01", 0x12, 0x04, &[0x01, 0x00]), // 加减速度（本型号恒 0）
    ("12:12:02", 0x12, 0x12, &[0x02, 0x00]), // 鼠标 SN
    // —— §0 表 10-12：身份与链路 ——
    ("7D:02:00", 0x7D, 0x02, &[0x00, 0x00]), // 有线模式 PID
    ("7D:02:01", 0x7D, 0x02, &[0x01, 0x00]), // 型号名
    ("7D:20:02", 0x7D, 0x20, &[0x02, 0x00]), // booster（2000Hz+ 支持标志）
    ("FB:02:00", 0xFB, 0x02, &[0x00, 0x00]), // 鼠标端 MAC
    ("FB:02:01", 0xFB, 0x02, &[0x01, 0x00]), // 接收器端 MAC
    ("FB:02:02", 0xFB, 0x02, &[0x02, 0x00]), // 无线 PID
    ("12:10:00", 0x12, 0x10, &[0x00, 0x00]), // 恒 0，读来当健康探针
];

const LIGHT_FRAMES: &[(&str, u8, u8, &[u8])] = &[
    ("12:07:00", 0x12, 0x07, &[0x00, 0x00]), // 电量（+ 休眠/低电量阈值/充满效果，同帧白拿）
    ("12:04:00", 0x12, 0x04, &[0x00, 0x00]), // 回报率 / 去抖 / 角度校正
    ("12:00:00", 0x12, 0x00, &[0x00, 0x00]), // profile 与当前 DPI 档位
];

/// 接收器通道（`0xFF02`，report id 0x01）的帧。鼠标休眠时这几帧**依然有应答**，
/// 因此它们是区分「鼠标睡了」和「接收器掉了」的关键证据。
const DONGLE_FRAMES: &[(&str, u8, u8)] = &[
    ("A1:00", 0xA1, 0x00), // 接收器 SN
    ("A1:01", 0xA1, 0x01), // 接收器固件
];

/// 抓取结果：快照 + 实际耗时。
pub struct FetchOutcome {
    /// 用 `Arc` 而不是值：命中缓存时要交给调用方一份快照，值语义就得整份
    /// 深拷贝（17 帧 `BTreeMap<String, Vec<u8>>` + 两个解析结构），而这份
    /// 数据在多个消费者手里都是只读的。
    pub snapshot: std::sync::Arc<Snapshot>,
    pub elapsed: Duration,
    pub frames_read: usize,
}

pub fn fetch_light(dev: &mut HidDevice, report_id: u8, link_mode: &str) -> FetchOutcome {
    fetch_light_with(report_id, link_mode, |rid, cmd, sub, tail| {
        dev.xfer(rid, cmd, sub, tail, crate::hid::DEFAULT_TIMEOUT_MS)
    })
}

pub fn fetch_light_with<F>(
    report_id: u8,
    link_mode: &str,
    mut ask: F,
) -> FetchOutcome
where
    F: FnMut(u8, u8, u8, &[u8]) -> Option<Vec<u8>>,
{
    let started = Instant::now();
    let mut snap = Snapshot {
        link_mode: link_mode.to_string(),
        report_id,
        ..Default::default()
    };

    let mut frames_read = 0usize;
    for (i, (key, cmd, sub, tail)) in LIGHT_FRAMES.iter().enumerate() {
        match ask(report_id, *cmd, *sub, tail) {
            Some(m) if !crate::hid::is_nak(&m) => {
                snap.frames.insert((*key).to_string(), m);
                frames_read += 1;
            }
            Some(m) => {
                snap.failed
                    .insert((*key).to_string(), format!("设备 NAK（{} 字节）", m.len()));
            }
            None => {
                snap.failed
                    .insert((*key).to_string(), "无应答（鼠标休眠中？）".to_string());
                // 第一帧就没应答 = 鼠标睡了（或链路断了）。别再花两次超时去确认。
                if i == 0 {
                    snap.taken = Some(Instant::now());
                    snap.taken_unix = unix_now();
                    parse_all(&mut snap);
                    return FetchOutcome {
                        snapshot: std::sync::Arc::new(snap),
                        elapsed: started.elapsed(),
                        frames_read,
                    };
                }
            }
        }
    }

    snap.taken = Some(Instant::now());
    snap.taken_unix = unix_now();
    parse_all(&mut snap);

    FetchOutcome {
        snapshot: std::sync::Arc::new(snap),
        elapsed: started.elapsed(),
        frames_read,
    }
}

/// **一次读全 17 帧**，返回完整快照。
///
/// 单帧失败不会中断整体：失败的 key 记进 `snapshot.failed`，
/// 其余帧照常解析。这样鼠标半休眠（少数帧丢失）时仍能给出可用结果。
///
/// 刻意**不读 MotionSync（`12 04 04`）**：本型号不响应，一次 MISS 要等满 2400ms，
/// 为永远不来的值付这个代价没有意义。同理不读 `12 05 01`（第二层全 FF）以外的探索性帧。
pub fn fetch(
    dev: &mut HidDevice,
    all_interfaces: &[crate::hid::HidInterface],
    report_id: u8,
    link_mode: &str,
) -> FetchOutcome {
    let started = Instant::now();
    let mut snap = Snapshot {
        link_mode: link_mode.to_string(),
        report_id,
        ..Default::default()
    };

    let mut frames_read = 0usize;
    for (key, cmd, sub, tail) in MOUSE_FRAMES {
        match dev.xfer(report_id, *cmd, *sub, tail, crate::hid::DEFAULT_TIMEOUT_MS) {
            Some(m) if !crate::hid::is_nak(&m) => {
                snap.frames.insert((*key).to_string(), m);
                frames_read += 1;
            }
            Some(m) => {
                snap.failed
                    .insert((*key).to_string(), format!("设备 NAK（{} 字节）", m.len()));
            }
            None => {
                snap.failed
                    .insert((*key).to_string(), "无应答（鼠标休眠中？）".to_string());
            }
        }
    }

    // 接收器侧两帧：走另一条通道，失败也不影响鼠标侧数据
    if link_mode == "Wireless24G" {
        if let Ok(mut dongle) =
            crate::hid::HidDevice::open_matching(all_interfaces, crate::hid::USAGE_PAGE_DONGLE_CTRL)
        {
            for (key, cmd, sub) in DONGLE_FRAMES {
                if let Some(m) = dongle.xfer(1, *cmd, *sub, &[], crate::hid::DEFAULT_TIMEOUT_MS) {
                    if !crate::hid::is_nak(&m) {
                        snap.frames.insert((*key).to_string(), m);
                        frames_read += 1;
                    }
                }
            }
        }
    }

    snap.taken = Some(Instant::now());
    snap.taken_unix = unix_now();

    // 解析：一帧失败只影响它自己负责的字段
    parse_all(&mut snap);

    FetchOutcome {
        snapshot: std::sync::Arc::new(snap),
        elapsed: started.elapsed(),
        frames_read,
    }
}

/// 把 `snap.frames` 里的原始帧解析进 `snap.state` / `snap.identity`。
///
/// 这是**纯函数**（只读 frames，写 state/identity），所以从磁盘缓存恢复的帧
/// 也走同一套解析，不存在「实时读」和「读缓存」结果不一致的可能。
fn parse_all(snap: &mut Snapshot) {
    let mut st = State::default();
    let mut id = Identity::default();

    // 一个帧解析失败不影响其它帧
    let mut try_parse = |key: &str, f: &dyn Fn(&[u8], &mut State) -> Result<(), String>| {
        if let Some(bytes) = snap.frames.get(key) {
            let bytes = bytes.clone();
            if let Err(e) = f(&bytes, &mut st) {
                snap.failed.insert(key.to_string(), e);
            }
        }
    };

    for &(key, _, _, parse) in CONFIGURATION_FRAMES {
        try_parse(key, &parse);
    }

    // 身份字段：逐帧取，缺哪帧就留空，不算失败
    if let Some(m) = snap.frames.get("7D:02:01") {
        id.model_name = proto::parse_model_name(m);
    }
    if let Some(m) = snap.frames.get("7D:02:00") {
        id.wired_pid = proto::parse_pid_at5(m);
    }
    if let Some(m) = snap.frames.get("7D:20:02") {
        id.booster = proto::parse_booster(m);
    }
    if let Some(m) = snap.frames.get("FB:02:00") {
        id.mouse_mac = proto::parse_mac(m);
    }
    if let Some(m) = snap.frames.get("FB:02:01") {
        id.dongle_mac = proto::parse_mac(m);
    }
    if let Some(m) = snap.frames.get("FB:02:02") {
        id.wireless_pid = proto::parse_pid_at5(m);
    }
    if let Some(m) = snap.frames.get("12:12:02") {
        id.mouse_sn = proto::parse_sn(m);
    }
    if let Some(m) = snap.frames.get("A1:00") {
        id.dongle_sn = proto::parse_dongle_sn(m);
    }
    if let Some(m) = snap.frames.get("A1:01") {
        id.dongle_firmware = proto::parse_dongle_fw(m);
    }

    snap.state = st;
    snap.identity = id;
}

// ==================== 磁盘缓存 ====================

/// 磁盘缓存的文本格式。刻意做成人类可读的，出问题时能直接看。
///
/// ```text
/// # rog-mouse-ctl snapshot cache v1
/// taken_unix=1761061234
/// link_mode=Wireless24G
/// report_id=3
/// frame 12:07:00 12070000005504...      （十六进制）
/// ```
const CACHE_MAGIC: &str = "# rog-mouse-ctl snapshot cache v1";

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 当前 wall-clock 秒数。`cache` 子命令用它推算缓存年龄。
pub fn now_unix() -> u64 {
    unix_now()
}

impl Snapshot {
    pub fn to_cache_text(&self) -> String {
        // 17 帧 × 64 字节 = 1088 次格式化。每字节一次 `format!` 就是 1088 次堆分配，
        // 而这张表只用来写一个 30 秒一次的缓存文件 —— 用查表直接拼，零分配。
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut s = String::with_capacity(96 + self.frames.len() * 160);
        s.push_str(CACHE_MAGIC);
        s.push('\n');
        s.push_str("taken_unix=");
        s.push_str(&self.taken_unix.to_string());
        s.push('\n');
        s.push_str("link_mode=");
        s.push_str(&self.link_mode);
        s.push('\n');
        s.push_str("report_id=");
        s.push_str(&self.report_id.to_string());
        s.push('\n');
        if let Some(key) = &self.cache_key {
            s.push_str("cache_key=");
            s.push_str(key);
            s.push('\n');
        }
        for (k, v) in &self.frames {
            s.push_str("frame ");
            s.push_str(k);
            s.push(' ');
            for b in v {
                s.push(HEX[(b >> 4) as usize] as char);
                s.push(HEX[(b & 0x0F) as usize] as char);
            }
            s.push('\n');
        }
        s
    }

    /// 从缓存文本恢复。
    ///
    /// `taken` 由 `taken_unix` 的墙钟年龄反推出来：留 `None` 的话，这份快照被
    /// `get()` 塞进内存槽之后 `is_fresh()` 永远判过期，每帧都重新读盘解析。
    pub fn from_cache_text(text: &str) -> Result<Snapshot, String> {
        let mut lines = text.lines();
        let first = lines.next().unwrap_or("");
        if first.trim() != CACHE_MAGIC {
            return Err("缓存文件格式不匹配（版本不同？）".to_string());
        }

        let mut snap = Snapshot::default();
        let mut wall = 0u64;
        for line in lines {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(v) = line.strip_prefix("taken_unix=") {
                wall = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("link_mode=") {
                snap.link_mode = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("report_id=") {
                snap.report_id = v.trim().parse().unwrap_or(3);
            } else if let Some(v) = line.strip_prefix("cache_key=") {
                if v.len() == 16 && v.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    snap.cache_key = Some(v.to_string());
                }
            } else if let Some(rest) = line.strip_prefix("frame ") {
                let mut it = rest.splitn(2, ' ');
                let key = it.next().unwrap_or("").to_string();
                // 按字节处理：缓存文件被手改/写坏时可能混进多字节字符，用 &str 切片
                // （`&hex[i..i+2]`）会在非字符边界 panic，而 load_disk_cache 用 .ok()
                // 接不住 panic。
                let hex = it.next().unwrap_or("").trim().as_bytes();
                if key.is_empty() || hex.len() % 2 != 0 {
                    continue;
                }
                let mut bytes = Vec::with_capacity(hex.len() / 2);
                let mut ok = true;
                for i in (0..hex.len()).step_by(2) {
                    let hi = (hex[i] as char).to_digit(16);
                    let lo = (hex[i + 1] as char).to_digit(16);
                    match (hi, lo) {
                        (Some(h), Some(l)) => bytes.push(((h as u8) << 4) | l as u8),
                        _ => {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    snap.frames.insert(key, bytes);
                }
            }
        }

        snap.taken_unix = wall;
        // 用墙钟年龄反推 `Instant`；`checked_sub` 防止年龄超出开机时长时 panic。
        let now = unix_now();
        if wall > 0 && wall <= now {
            let age = std::time::Duration::from_secs(now - wall);
            snap.taken = std::time::Instant::now().checked_sub(age);
        }
        parse_all(&mut snap);
        Ok(snap)
    }

    /// 磁盘缓存的年龄（秒）。用于判断是否还能用。
    pub fn wall_age_sec(&self) -> u64 {
        unix_now().saturating_sub(self.taken_unix)
    }
}

/// 缓存管理器：内存 + 磁盘双层，带显式失效。
pub struct SnapshotCache {
    pub snapshot: Option<std::sync::Arc<Snapshot>>,
    pub ttl: Duration,
    /// 轻量读取的独立槽位（只含 3 帧）。
    ///
    /// **刻意与 `snapshot` 分开、也刻意不写磁盘**：3 帧的快照不是完整快照，
    /// 写进磁盘缓存会被下一个进程误当成 17 帧全量读出来用，
    /// 导致 DPI 档位、灯光、按键全显示默认值。
    pub light: Option<std::sync::Arc<Snapshot>>,
}

fn cached_hit(
    slot: &Option<std::sync::Arc<Snapshot>>,
    ttl: Duration,
    key: Option<&str>,
    link: &Link,
) -> Option<std::sync::Arc<Snapshot>> {
    let s = slot.as_ref()?;
    if cache_matches(s, ttl, key, link) {
        Some(std::sync::Arc::clone(s))
    } else {
        None
    }
}

fn cache_matches(snapshot: &Snapshot, ttl: Duration, key: Option<&str>, link: &Link) -> bool {
    key.is_some()
        && snapshot.cache_key.as_deref() == key
        && snapshot.report_id == link.report_id
        && snapshot.link_mode == link.mode
        && snapshot.is_fresh(ttl)
}

pub(crate) fn device_identity(identity: &Identity) -> Option<String> {
    let value = identity.device_id()?;
    let kind = if identity.mouse_sn.trim().is_empty() { "mac" } else { "serial" };
    Some(format!("{kind}:{value}"))
}

fn cache_key(link: &Link, identity: &str) -> String {
    crate::device::stable_key(&format!(
        "{:?}:{:04x}:{:04x}:{:04x}:{}:{:?}:{}:{identity}",
        link.iface.path.to_ascii_lowercase(), link.iface.vendor_id, link.iface.product_id,
        link.iface.usage_page, link.iface.usage, link.mode, link.report_id,
    ))
}

fn live_cache_key(dev: &mut HidDevice, link: &Link) -> Option<String> {
    live_cache_key_with(link, |rid, cmd, sub, tail| {
        dev.xfer(rid, cmd, sub, tail, crate::hid::DEFAULT_TIMEOUT_MS)
    })
}

fn live_cache_key_with(
    link: &Link,
    ask: impl FnMut(u8, u8, u8, &[u8]) -> Option<Vec<u8>>,
) -> Option<String> {
    read_device_identity_with(link.report_id, ask).map(|identity| cache_key(link, &identity))
}

pub(crate) fn read_device_identity(dev: &mut HidDevice, report_id: u8) -> Option<String> {
    read_device_identity_with(report_id, |rid, cmd, sub, tail| {
        dev.xfer(rid, cmd, sub, tail, crate::hid::DEFAULT_TIMEOUT_MS)
    })
}

fn read_device_identity_with(
    report_id: u8,
    mut ask: impl FnMut(u8, u8, u8, &[u8]) -> Option<Vec<u8>>,
) -> Option<String> {
    let mut identity = Identity::default();
    if let Some(frame) = ask(report_id, 0x12, 0x12, &[2, 0]) {
        if frame.starts_with(&[report_id, 0x12, 0x12]) {
            identity.mouse_sn = proto::parse_sn(&frame);
        }
    }
    if device_identity(&identity).is_none() {
        if let Some(frame) = ask(report_id, 0xFB, 0x02, &[0, 0]) {
            if frame.starts_with(&[report_id, 0xFB, 0x02]) {
                identity.mouse_mac = proto::parse_mac(&frame);
            }
        }
    }
    device_identity(&identity)
}

impl SnapshotCache {
    pub fn new(ttl_sec: u64) -> Self {
        SnapshotCache {
            snapshot: None,
            ttl: Duration::from_secs(ttl_sec.max(1)),
            light: None,
        }
    }

    /// CLI quick：先查询身份再匹配缓存；未命中时读 3 帧并复核身份。
    /// 身份查询优先 SN，失败后尝试 MAC；没有有效回帧时返回 `None`。
    pub fn get_light(&mut self, dev: &mut HidDevice, link: &Link) -> Option<FetchOutcome> {
        let key = live_cache_key(dev, link);
        // 1) 身份匹配的全量读取结果
        if let Some(s) = cached_hit(&self.snapshot, self.ttl, key.as_deref(), link) {
            return Some(FetchOutcome {
                frames_read: s.frames.len(),
                elapsed: Duration::ZERO,
                snapshot: s,
            });
        }

        // 2) 轻量快照新鲜
        if let Some(s) = cached_hit(&self.light, self.ttl, key.as_deref(), link) {
            return Some(FetchOutcome {
                frames_read: s.frames.len(),
                elapsed: Duration::ZERO,
                snapshot: s,
            });
        }

        // 3) 真读
        let mut out = fetch_light(dev, link.report_id, &link.mode);
        if out.frames_read == 0 {
            // 一帧都没读到：鼠标睡了。不缓存「空」，否则界面会卡在休眠态
            self.light = None;
            return None;
        }
        if key.is_some() && live_cache_key(dev, link) == key {
            std::sync::Arc::make_mut(&mut out.snapshot).cache_key = key;
        }
        self.light = Some(out.snapshot.clone());
        Some(out)
    }

    /// 取一份可用的快照：内存命中 → 磁盘命中 → 真读设备。
    ///
    pub fn get(
        &mut self,
        dev: &mut HidDevice,
        link: &Link,
    ) -> FetchOutcome {
        let key = live_cache_key(dev, link);
        self.get_with(link, key.as_deref(), load_disk_cache,
            || fetch(dev, &link.all_interfaces, link.report_id, &link.mode), save_disk_cache)
    }

    fn get_with(
        &mut self,
        link: &Link,
        key: Option<&str>,
        load: impl FnOnce() -> Option<Snapshot>,
        fetch: impl FnOnce() -> FetchOutcome,
        save: impl FnOnce(&Snapshot),
    ) -> FetchOutcome {
        // 1) 内存
        if let Some(s) = cached_hit(&self.snapshot, self.ttl, key, link) {
            return FetchOutcome {
                frames_read: s.frames.len(),
                elapsed: Duration::ZERO,
                snapshot: s,
            };
        }

        // 2) 磁盘（跨进程复用，CLI 每次都重启，没有这层缓存就没意义）
        if let Some(s) = key.and_then(|_| load()) {
            if cache_matches(&s, self.ttl, key, link) {
                let s = std::sync::Arc::new(s);
                self.snapshot = Some(std::sync::Arc::clone(&s));
                return FetchOutcome {
                    frames_read: s.frames.len(),
                    elapsed: Duration::ZERO,
                    snapshot: s,
                };
            }
        }

        // 3) 真读设备
        let mut out = fetch();
        let snapshot = std::sync::Arc::make_mut(&mut out.snapshot);
        snapshot.cache_key = device_identity(&snapshot.identity)
            .map(|identity| cache_key(link, &identity))
            .filter(|observed| Some(observed.as_str()) == key);
        if snapshot.cache_key.is_some() && !snapshot.frames.is_empty() {
            save(snapshot);
        }
        self.snapshot = Some(std::sync::Arc::clone(&out.snapshot));
        FetchOutcome {
            frames_read: out.frames_read,
            elapsed: out.elapsed,
            snapshot: std::sync::Arc::clone(&out.snapshot),
        }
    }

    /// **任何写操作之后必须调用**：设备侧的值变了，缓存必然过期。
    ///
    /// 同时删掉磁盘缓存，否则下一个进程（或 GUI 重启后）会读到旧值。
    pub fn invalidate(&mut self) {
        self.snapshot = None;
        self.light = None;
        invalidate_disk_cache();
    }

}

pub(crate) fn invalidate_disk_cache() {
    let _ = std::fs::remove_file(crate::paths::snapshot_cache_file());
}

fn load_disk_cache() -> Option<Snapshot> {
    let p = crate::paths::snapshot_cache_file();
    let text = std::fs::read_to_string(&p).ok()?;
    Snapshot::from_cache_text(&text).ok()
}

fn save_disk_cache(s: &Snapshot) {
    let dir = crate::paths::cache_dir();
    if crate::paths::ensure_dir(&dir).is_err() {
        return; // 缓存写不进去不是致命错误，静默降级成纯内存
    }
    let p = crate::paths::snapshot_cache_file();
    let _ = std::fs::write(p, s.to_cache_text());
}

#[cfg(test)]
mod configuration_tests {
    use super::*;

    fn link(path: &str) -> Link {
        Link {
            mode: "Wireless24G".into(), report_id: 3, all_interfaces: Vec::new(),
            iface: crate::hid::HidInterface {
                path: path.into(), vendor_id: 0x0b05, product_id: 0x1ace,
                usage_page: 0xff01, usage: 1, input_len: 64, output_len: 64,
                feature_len: 0, product: "Test receiver".into(), serial: "shared".into(),
            },
        }
    }

    fn complete_snapshot() -> Snapshot {
        let mut snapshot = Snapshot {
            taken: Some(Instant::now()), taken_unix: unix_now(),
            report_id: 3, link_mode: "Wireless24G".into(), ..Default::default()
        };
        for &(key, sub, tail, _) in CONFIGURATION_FRAMES {
            let mut frame = vec![0; 64];
            frame[..3].copy_from_slice(&[3, 0x12, sub]);
            frame[3..5].copy_from_slice(tail);
            match key {
                "12:04:00" => frame[13] = 3,
                "12:04:02" => {
                    for offset in (5..21).step_by(4) { frame[offset] = 15; }
                }
                "12:00:00" => frame[12] = 1,
                "12:07:00" => { frame[5] = 50; frame[6] = 2; frame[7] = 20; frame[12] = 1; }
                "12:05:00" => {
                    for (index, value) in proto::BUTTON_SRC.iter().enumerate() {
                        frame[5 + index * 2..7 + index * 2].copy_from_slice(&value.to_le_bytes());
                    }
                }
                _ => (),
            }
            snapshot.frames.insert(key.into(), frame);
        }
        parse_all(&mut snapshot);
        snapshot.identity.mouse_sn = "synthetic-mouse-a".into();
        snapshot
    }

    fn outcome(snapshot: Snapshot) -> FetchOutcome {
        FetchOutcome { frames_read: snapshot.frames.len(), snapshot: std::sync::Arc::new(snapshot),
            elapsed: Duration::from_millis(1) }
    }

    #[test]
    fn complete_configuration_preserves_every_backup_field() {
        let snapshot = complete_snapshot();
        let configuration = snapshot.configuration().unwrap();
        let text = crate::backup::render(&configuration, &snapshot.link_mode, snapshot.report_id, "test");
        let backup = crate::backup::parse(&text).unwrap();
        assert_eq!(backup.keys_found, crate::backup::BACKUP_KEYS.len());
        assert!(backup.rejected.is_empty());
        assert_eq!(backup.buttons, proto::BUTTON_SRC.map(Some));
        assert_eq!((backup.poweroff_code, backup.low_warn, backup.full_effect), (Some(2), Some(20), Some(1)));
    }

    #[test]
    fn every_configuration_frame_is_required_even_when_display_state_exists() {
        for &(key, _, _, _) in CONFIGURATION_FRAMES {
            let mut snapshot = complete_snapshot();
            snapshot.frames.remove(key);
            parse_all(&mut snapshot);
            assert!(snapshot.configuration().err().unwrap().contains(key));
            if key != "12:07:00" { assert_eq!(snapshot.battery().unwrap().percent, Some(50)); }
            if key == "12:05:00" { assert_eq!(snapshot.state.buttons, [0; proto::BUTTON_SLOTS]); }
        }
    }

    #[test]
    fn malformed_nak_and_wrong_report_id_never_form_a_configuration() {
        for &(key, sub, _, _) in CONFIGURATION_FRAMES {
            for frame in [vec![3, 0x12, sub], vec![3, 0xff, 0xaa], vec![1; 64]] {
                let mut snapshot = complete_snapshot();
                snapshot.frames.insert(key.into(), frame);
                assert!(snapshot.configuration().err().unwrap().contains(key));
            }
            let mut snapshot = complete_snapshot();
            snapshot.failed.insert(key.into(), "read failed".into());
            assert!(snapshot.configuration().is_err());
        }
    }

    #[test]
    fn configuration_reader_always_uses_new_frames_and_rejects_each_missing_frame() {
        let original = complete_snapshot();
        let mut fresh = original.clone();
        fresh.frames.get_mut("12:07:00").unwrap()[7] = 35;
        let mut reads = 0;
        let configuration = read_configuration_with(3, |rid, cmd, sub, tail| {
            assert_eq!((rid, cmd), (3, 0x12));
            reads += 1;
            fresh.frames.get(&format!("12:{sub:02X}:{:02X}", tail[0])).cloned()
        }).unwrap();
        assert_eq!(reads, 8);
        assert_eq!(configuration.state().low_warn, 35);
        assert_eq!(original.state.low_warn, 20);
        for &(missing, _, _, _) in CONFIGURATION_FRAMES {
            let result = read_configuration_with(3, |_, _, sub, tail| {
                let key = format!("12:{sub:02X}:{:02X}", tail[0]);
                if key == missing { None } else { fresh.frames.get(&key).cloned() }
            });
            assert!(result.err().unwrap().contains(missing));
        }
    }

    #[test]
    fn cache_binding_separates_path_device_report_id_and_transport() {
        let selected = link("synthetic-receiver-a");
        let identity = "serial:synthetic-mouse-a";
        let key = cache_key(&selected, identity);
        let mut snapshot = complete_snapshot();
        snapshot.cache_key = Some(key.clone());
        let slot = Some(std::sync::Arc::new(snapshot));
        let ttl = Duration::from_secs(30);
        assert!(cached_hit(&slot, ttl, Some(&key), &selected).is_some());
        assert!(cached_hit(&slot, ttl, None, &selected).is_none());
        assert!(cached_hit(&slot, ttl, Some(&cache_key(&selected, "serial:synthetic-mouse-b")), &selected).is_none());
        let mut other = link("synthetic-receiver-b");
        assert!(cached_hit(&slot, ttl, Some(&cache_key(&other, identity)), &other).is_none());
        other = selected.clone(); other.report_id = 1;
        assert!(cached_hit(&slot, ttl, Some(&key), &other).is_none());
        assert_ne!(cache_key(&other, identity), key);
        other = selected.clone(); other.mode = "Wired".into();
        assert!(cached_hit(&slot, ttl, Some(&key), &other).is_none());
        assert_ne!(cache_key(&other, identity), key);
    }

    #[test]
    fn cache_identity_requires_a_live_serial_or_valid_mouse_mac() {
        let selected = link("synthetic-receiver-a");
        let key = live_cache_key_with(&selected, |rid, cmd, sub, tail| {
            assert_eq!((cmd, sub, tail), (0x12, 0x12, &[2, 0][..]));
            let mut frame = vec![rid, cmd, sub, 2, 0];
            frame.extend_from_slice(b"synthetic-mouse-a\0");
            Some(frame)
        });
        assert_eq!(key, Some(cache_key(&selected, "serial:synthetic-mouse-a")));
        for invalid in [None, Some(vec![3, 0xff, 0xaa]), Some(vec![0; 64]), Some(vec![1; 64])] {
            assert!(live_cache_key_with(&selected, |_, _, _, _| invalid.clone()).is_none());
        }
        for mac in [[0; 6], [255; 6], [2, 3, 4, 5, 6, 7]] {
            let key = live_cache_key_with(&selected, |rid, cmd, sub, _| {
                if cmd == 0x12 { return None; }
                let mut frame = vec![rid, cmd, sub, 0, 0, 0];
                frame.extend_from_slice(&mac);
                Some(frame)
            });
            assert_eq!(key.is_some(), mac == [2, 3, 4, 5, 6, 7]);
        }
    }

    #[test]
    fn disk_cache_roundtrip_keeps_binding_and_rejects_old_or_future_entries() {
        let selected = link("synthetic-receiver-a");
        let key = cache_key(&selected, "serial:synthetic-mouse-a");
        let mut snapshot = complete_snapshot();
        snapshot.cache_key = Some(key.clone());
        let text = snapshot.to_cache_text();
        let restored = Snapshot::from_cache_text(&text).unwrap();
        assert!(cache_matches(&restored, Duration::from_secs(30), Some(&key), &selected));
        let unbound = text.lines().filter(|line| !line.starts_with("cache_key=")).collect::<Vec<_>>().join("\n");
        let restored = Snapshot::from_cache_text(&unbound).unwrap();
        assert!(!cache_matches(&restored, Duration::from_secs(30), Some(&key), &selected));
        for taken in [0, unix_now().saturating_sub(60), unix_now() + 3600] {
            snapshot.taken_unix = taken;
            let restored = Snapshot::from_cache_text(&snapshot.to_cache_text()).unwrap();
            assert!(!cache_matches(&restored, Duration::from_secs(30), Some(&key), &selected));
        }
    }

    #[test]
    fn mismatched_memory_and_disk_cache_are_replaced_by_fresh_device_read() {
        let selected = link("synthetic-receiver-b");
        let key = cache_key(&selected, "serial:synthetic-mouse-a");
        let mut stale = complete_snapshot();
        stale.cache_key = Some(cache_key(&link("synthetic-receiver-a"), "serial:synthetic-mouse-a"));
        let disk = Snapshot::from_cache_text(&stale.to_cache_text()).unwrap();
        let mut cache = SnapshotCache::new(30);
        cache.snapshot = Some(std::sync::Arc::new(stale));
        let mut fetched = false;
        let mut saved = false;
        let result = cache.get_with(&selected, Some(&key), || Some(disk), || {
            fetched = true;
            let mut fresh = complete_snapshot(); fresh.state.low_warn = 35;
            outcome(fresh)
        }, |snapshot| { saved = true; assert_eq!(snapshot.cache_key.as_deref(), Some(key.as_str())); });
        assert!(fetched && saved);
        assert_eq!(result.snapshot.state.low_warn, 35);
    }

    #[test]
    fn a_matching_disk_cache_is_reused_only_with_current_identity_evidence() {
        let selected = link("synthetic-receiver-a");
        let key = cache_key(&selected, "serial:synthetic-mouse-a");
        let mut snapshot = complete_snapshot(); snapshot.cache_key = Some(key.clone());
        let disk = Snapshot::from_cache_text(&snapshot.to_cache_text()).unwrap();
        let mut cache = SnapshotCache::new(30);
        let result = cache.get_with(&selected, Some(&key), || Some(disk),
            || panic!("matching cache should be reused"), |_| panic!("hit must not be saved again"));
        assert_eq!(result.elapsed, Duration::ZERO);
        let result = cache.get_with(&selected, None, || panic!("unverified identity must not load disk"),
            || outcome(complete_snapshot()), |_| panic!("unverified reading must not be cached on disk"));
        assert!(result.snapshot.cache_key.is_none());
    }

    #[test]
    fn identity_change_during_fetch_prevents_cache_publication() {
        let selected = link("synthetic-receiver-a");
        let key = cache_key(&selected, "serial:synthetic-mouse-a");
        let mut cache = SnapshotCache::new(30);
        let result = cache.get_with(&selected, Some(&key), || None, || {
            let mut snapshot = complete_snapshot(); snapshot.identity.mouse_sn = "synthetic-mouse-b".into();
            outcome(snapshot)
        }, |_| panic!("changed device must not be saved"));
        assert!(result.snapshot.cache_key.is_none());
        assert!(cached_hit(&cache.snapshot, cache.ttl, Some(&key), &selected).is_none());
    }
}
