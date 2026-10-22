//! 「一次性读完 + 缓存」的读模型。
//!
//! 用户明确要求：**一次把所有参数读全，缓存起来，之后从缓存读**，
//! 不要每次按需只读一帧。
//!
//! 为什么这么做——文档实测的帧成本：
//!
//! ```text
//! 进程启动 + OpenLink   ~190-230 ms  （只付一次）
//! 每多一帧的边际成本     ~27 ms
//! 17 帧全读             ~690 ms
//! 单帧 MISS（无应答）    = 超时值（DEFAULT_TIMEOUT_MS = 1200 ms）
//! ```
//!
//! 也就是说「多读几帧」远比「读不到」便宜。把 12 类信息一次读全（17 帧 + 电量搭车），
//! 之后所有读取都走内存/磁盘缓存，总代价约 690ms 而不是每次 200-400ms。
//!
//! ## 缓存策略
//!
//! - 内存缓存：一个 `Snapshot`，带 `Instant` 时间戳
//! - 磁盘缓存：`%LOCALAPPDATA%\rog-mouse-ctl\snapshot.cache`，带 wall-clock 时间戳，
//!   跨进程复用（CLI 每次启动都是新进程，没有磁盘缓存的话「缓存」就毫无意义）
//! - 有效期：默认 **30 秒**（可在 `settings.ini` 里改 `cache_ttl_sec`）
//! - 主动失效：**任何写操作之后**调用 [`SnapshotCache::invalidate`]，
//!   因为设备侧的值已经变了，缓存必然过期
//! - 电量档位永远比其它项更容易变，所以 `battery` 那几帧额外有一个更短的有效期

use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::hid::HidDevice;
use crate::proto::{self, Identity, State};

/// 默认缓存有效期（秒）。可在 `settings.ini` 里用 `cache_ttl_sec` 覆盖。
pub const DEFAULT_TTL_SEC: u64 = 30;

/// 设备的完整快照：17 帧原始数据 + 解析结果 + 时间戳。
///
/// **缓存原始帧而不是解析后的值**，是为了让「从设备读」和「从缓存读」
/// 走完全相同的解析代码路径（`proto::parse_*`），不可能出现两条逻辑不一致的情况。

/// 设备的完整快照：17 帧原始数据 + 解析结果 + 时间戳。
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
    /// 解析失败的帧（key -> 原因）。不让一帧读失败就整体失败。
    pub failed: BTreeMap<String, String>,
}

impl Snapshot {
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

// ==================== 抓取 ====================

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

/// 轻量读取只碰的 3 帧 —— 给 GUI 后台线程周期刷新用。
///
/// 顺序不是随便排的：**电量放第一帧**。它是这三项里唯一会变、也是用户最关心的；
/// 如果鼠标休眠，第一帧就会 MISS，这时候立刻收手比读完剩下两帧再一起报错省
/// 两个 2400ms 的超时（实测 MISS 一次要等满超时，是全协议最贵的操作）。
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
    pub snapshot: Snapshot,
    pub elapsed: Duration,
    pub frames_read: usize,
}

/// **轻量读取：只读 3 帧**，约 100-200ms。给 GUI 后台周期刷新用。
///
/// 与 [`fetch`] 的两点区别：
///
/// 1. **第一帧就 MISS 立即返回**，不再读剩下两帧。鼠标休眠时全量读会白等三次超时
///    （单帧 MISS = 一次 `DEFAULT_TIMEOUT_MS` = 1200ms，实测含设备打开约
///    1100-1150ms；不提前返回就要付 3 倍），而「电量读不到」等于「鼠标睡了」
///    这个结论第一帧就够了。
/// 2. 不读身份帧 —— 身份不会因为用户改设置而变，缓存里的旧值仍然是对的。
///
/// 返回的 `snapshot.frames` 里只有实际读到的帧；调用方要么看 `frames_read`，
/// 要么用 `snapshot.state` 里自己关心的那几个字段（未读的字段是 `Default`，
/// 不代表设备真的读出来是 0）。判据：`snapshot.frames.contains_key(...)`。
pub fn fetch_light(dev: &mut HidDevice, report_id: u8, link_mode: &str) -> FetchOutcome {
    fetch_light_with(report_id, link_mode, |rid, cmd, sub, tail| {
        dev.xfer(rid, cmd, sub, tail, crate::hid::DEFAULT_TIMEOUT_MS)
    })
}

/// [`fetch_light`] 的内核，把「发一帧」抽成参数。
///
/// 抽出这一层是为了让「第一帧 MISS 就提前返回」这条**时序行为能被单测覆盖** ——
/// 靠真机等鼠标休眠来验证这件事不可行（用户一直在动鼠标，等不到 10 分钟空闲），
/// 而这是全程序唯一一条以「省掉两次超时」为目的的逻辑，不能只靠读代码保证。
///
/// 实测成本对照（2026-10-21，`raw 12 FF` 连跑 3 次）：单帧 MISS
/// **1098 / 1099 / 1151 ms**（协议超时 900ms + 约 200ms 设备打开）。
/// 这里用 1200ms 超时，所以休眠时提前返回约 1.4s，不提前返回约 3.8s。
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
                        snapshot: snap,
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
        snapshot: snap,
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
        snapshot: snap,
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

    try_parse("12:04:00", &proto::parse_perf);
    try_parse("12:04:02", &proto::parse_dpi);
    try_parse("12:04:03", &proto::parse_colors);
    try_parse("12:00:00", &proto::parse_profile);
    try_parse("12:07:00", &proto::parse_energy);
    try_parse("12:06:00", &proto::parse_lod);
    try_parse("12:03:00", &proto::parse_light);
    try_parse("12:05:00", &proto::parse_buttons);

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
        let mut s = String::new();
        s.push_str(CACHE_MAGIC);
        s.push('\n');
        s.push_str(&format!("taken_unix={}\n", self.taken_unix));
        s.push_str(&format!("link_mode={}\n", self.link_mode));
        s.push_str(&format!("report_id={}\n", self.report_id));
        for (k, v) in &self.frames {
            let hex: String = v.iter().map(|b| format!("{b:02x}")).collect();
            s.push_str(&format!("frame {k} {hex}\n"));
        }
        s
    }

    /// 从缓存文本恢复。时间戳无法恢复成 `Instant`，所以 `taken` 留 `None`，
    /// 由调用方用 `taken_unix` 的年龄做判断。
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
            } else if let Some(rest) = line.strip_prefix("frame ") {
                let mut it = rest.splitn(2, ' ');
                let key = it.next().unwrap_or("").to_string();
                let hex = it.next().unwrap_or("");
                if key.is_empty() || hex.len() % 2 != 0 {
                    continue;
                }
                let mut bytes = Vec::with_capacity(hex.len() / 2);
                let mut ok = true;
                for i in (0..hex.len()).step_by(2) {
                    match u8::from_str_radix(&hex[i..i + 2], 16) {
                        Ok(b) => bytes.push(b),
                        Err(_) => {
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
    pub snapshot: Option<Snapshot>,
    pub ttl: Duration,
    /// 轻量读取的独立槽位（只含 3 帧）。
    ///
    /// **刻意与 `snapshot` 分开、也刻意不写磁盘**：3 帧的快照不是完整快照，
    /// 写进磁盘缓存会被下一个进程误当成 17 帧全量读出来用，
    /// 导致 DPI 档位、灯光、按键全显示默认值。
    pub light: Option<Snapshot>,
}

impl SnapshotCache {
    pub fn new(ttl_sec: u64) -> Self {
        SnapshotCache {
            snapshot: None,
            ttl: Duration::from_secs(ttl_sec.max(1)),
            light: None,
        }
    }

    /// 轻量读取：3 帧，约 100-200ms。给 GUI 后台周期刷新用。
    ///
    /// 三层策略：
    /// 1. 内存里有**全量**快照且还新鲜 → 直接用它（它一定含这 3 项）
    /// 2. 内存里有新鲜的轻量快照 → 用它
    /// 3. 真读 3 帧（第一帧 MISS 立刻返回，不白等两次超时）
    ///
    /// 返回 `None` 表示连第一帧都没读到 —— 即**鼠标休眠**，不是错误。
    pub fn get_light(&mut self, dev: &mut HidDevice, report_id: u8, link_mode: &str) -> Option<FetchOutcome> {
        // 1) 全量快照新鲜：它含这三项，没必要再读
        if let Some(s) = &self.snapshot {
            if s.is_fresh(self.ttl) {
                return Some(FetchOutcome {
                    frames_read: s.frames.len(),
                    elapsed: Duration::ZERO,
                    snapshot: s.clone(),
                });
            }
        }

        // 2) 轻量快照新鲜
        if let Some(s) = &self.light {
            if s.is_fresh(self.ttl) {
                return Some(FetchOutcome {
                    frames_read: s.frames.len(),
                    elapsed: Duration::ZERO,
                    snapshot: s.clone(),
                });
            }
        }

        // 3) 真读
        let out = fetch_light(dev, report_id, link_mode);
        if out.frames_read == 0 {
            // 一帧都没读到：鼠标睡了。不缓存「空」，否则界面会卡在休眠态
            self.light = None;
            return None;
        }
        self.light = Some(out.snapshot.clone());
        Some(out)
    }

    /// 取一份可用的快照：内存命中 → 磁盘命中 → 真读设备。
    ///
    /// `allow_stale` 为真时允许返回过期缓存（GUI 在后台刷新时用，
    /// 这样界面先显示旧数据、随后原地更新，不会白屏）。
    pub fn get(
        &mut self,
        dev: &mut HidDevice,
        all_interfaces: &[crate::hid::HidInterface],
        report_id: u8,
        link_mode: &str,
        allow_stale: bool,
    ) -> FetchOutcome {
        // 1) 内存
        if let Some(s) = &self.snapshot {
            if allow_stale || s.is_fresh(self.ttl) {
                return FetchOutcome {
                    frames_read: s.frames.len(),
                    elapsed: Duration::ZERO,
                    snapshot: s.clone(),
                };
            }
        }

        // 2) 磁盘（跨进程复用，CLI 每次都重启，没有这层缓存就没意义）
        if let Some(s) = load_disk_cache() {
            let fresh = s.wall_age_sec() < self.ttl.as_secs();
            if allow_stale || fresh {
                self.snapshot = Some(s.clone());
                return FetchOutcome {
                    frames_read: s.frames.len(),
                    elapsed: Duration::ZERO,
                    snapshot: s,
                };
            }
        }

        // 3) 真读设备
        let out = fetch(dev, all_interfaces, report_id, link_mode);
        if !out.snapshot.frames.is_empty() {
            save_disk_cache(&out.snapshot);
        }
        self.snapshot = Some(out.snapshot.clone());
        FetchOutcome {
            frames_read: out.frames_read,
            elapsed: out.elapsed,
            snapshot: out.snapshot,
        }
    }

    /// **任何写操作之后必须调用**：设备侧的值变了，缓存必然过期。
    ///
    /// 同时删掉磁盘缓存，否则下一个进程（或 GUI 重启后）会读到旧值。
    pub fn invalidate(&mut self) {
        self.snapshot = None;
        self.light = None;
        let p = crate::paths::snapshot_cache_file();
        let _ = std::fs::remove_file(p);
    }

    /// 只失效内存、保留磁盘 —— 给「刚写完但还没读回」的中间态用。
    #[allow(dead_code)]
    pub fn invalidate_memory_only(&mut self) {
        self.snapshot = None;
    }
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
mod tests {
    use super::*;

    /// 轻量读取的帧清单必须只含 3 项，且电量排第一。
    ///
    /// 顺序是功能性的：鼠标休眠时第一帧 MISS 就 `return`，靠的就是电量在首位。
    /// 谁把电量挪到后面，休眠时就要多等 2 次超时（实测每次 2400ms）。
    #[test]
    fn light_frames_are_three_and_battery_is_first() {
        assert_eq!(LIGHT_FRAMES.len(), 3, "轻量读取应该正好 3 帧");
        let first = LIGHT_FRAMES[0];
        assert_eq!(first.0, "12:07:00", "第一帧必须是电量帧");
        assert_eq!((first.1, first.2), (0x12, 0x07));
        // 另外两帧是性能与 profile
        let keys: Vec<&str> = LIGHT_FRAMES.iter().map(|f| f.0).collect();
        assert!(keys.contains(&"12:04:00"));
        assert!(keys.contains(&"12:00:00"));
    }

    /// 轻量帧必须是全量帧的子集 —— 否则同一个字段会有两套读法。
    #[test]
    fn light_frames_are_subset_of_full_frames() {
        let full: Vec<&str> = MOUSE_FRAMES.iter().map(|f| f.0).collect();
        for (key, ..) in LIGHT_FRAMES {
            assert!(
                full.contains(key),
                "轻量帧 {key} 不在全量帧清单里，说明两处定义已经漂移"
            );
        }
    }

    /// 3 帧快照不能把 `frames.len()` 冒充成完整快照。
    ///
    /// 磁盘缓存格式不记录「这是全量还是轻量」，所以轻量快照**根本不该写盘**。
    /// 这条测试锁住的是 `SnapshotCache::light` 与 `snapshot` 是两个独立槽位。
    #[test]
    fn light_and_full_slots_are_separate() {
        let c = SnapshotCache::new(30);
        assert!(c.snapshot.is_none());
        assert!(c.light.is_none());
    }

    /// 造一个看起来像 `12 07` 应答的帧。
    fn fake_energy(level: u8) -> Vec<u8> {
        let mut m = vec![0u8; 64];
        m[0] = 0x03;
        m[1] = 0x12;
        m[2] = 0x07;
        m[5] = level; // 电量
        m[6] = 4; // 休眠 10 分钟
        m[7] = 20; // 低电量阈值
        m[8] = 0x8F; // 电压低字节
        m[9] = 0x0F; // 电压高字节 → 3983 mV
        m[12] = 1; // 充满效果
        m
    }

    /// **休眠路径的核心断言**：第一帧 MISS 就收手，绝不问第二帧。
    ///
    /// 这条只能用假探针验 —— 真机上是「用户 10 分钟不碰鼠标」才触发，
    /// 而这台机器上用户一直在用。但它的后果是实打实的：多问两帧
    /// 就多付 2 × 1200ms（实测单帧 MISS 约 1.1-1.15s），GUI 后台线程会明显卡。
    #[test]
    fn light_stops_after_first_miss() {
        let mut asked: Vec<(u8, u8)> = Vec::new();
        let out = fetch_light_with(0x03, "Wireless24G", |_rid, cmd, sub, _tail| {
            asked.push((cmd, sub));
            None // 全部无应答 = 鼠标休眠
        });

        assert_eq!(
            asked,
            vec![(0x12, 0x07)],
            "第一帧 MISS 之后必须立刻返回，实际问了 {} 帧",
            asked.len()
        );
        assert_eq!(out.frames_read, 0);
        assert_eq!(out.snapshot.frames.len(), 0);
        assert!(
            out.snapshot.failed.contains_key("12:07:00"),
            "失败的帧要记进 failed，调用方靠它区分「读不到」和「读出来是 0」"
        );
        assert_eq!(out.snapshot.failed.len(), 1, "只该有第一帧的失败记录");
    }

    /// 醒着的时候三帧都要读到，不能因为「有 failed 就早退」而漏读。
    #[test]
    fn light_reads_all_three_when_awake() {
        let mut n = 0;
        let out = fetch_light_with(0x03, "Wireless24G", |_rid, cmd, sub, _tail| {
            n += 1;
            match (cmd, sub) {
                (0x12, 0x07) => Some(fake_energy(84)),
                (0x12, 0x04) => {
                    let mut m = vec![0u8; 64];
                    m[0] = 0x03;
                    m[1] = 0x12;
                    m[2] = 0x04;
                    m[13] = 3; // 回报率 1000Hz
                    m[15] = 2; // 去抖 12ms
                    Some(m)
                }
                (0x12, 0x00) => {
                    let mut m = vec![0u8; 64];
                    m[0] = 0x03;
                    m[1] = 0x12;
                    m[2] = 0x00;
                    m[12] = 2; // 当前档位
                    Some(m)
                }
                _ => None,
            }
        });

        assert_eq!(n, 3, "醒着时必须读完 3 帧");
        assert_eq!(out.frames_read, 3);
        assert!(out.snapshot.failed.is_empty(), "没有失败帧");
        assert_eq!(out.snapshot.state.power, 84);
        assert_eq!(out.snapshot.state.poll_code, 3);
        assert_eq!(out.snapshot.state.active_stage, 2);
    }

    /// **第二帧才 MISS 不该早退** —— 早退条件只针对第一帧。
    ///
    /// 理由：第一帧读到电量说明鼠标是醒的，此时第二帧丢失是偶发丢包，
    /// 应该继续把第三帧读完，而不是因为一次丢包就放弃整次刷新。
    #[test]
    fn light_keeps_going_when_second_frame_misses() {
        let mut asked = 0;
        let out = fetch_light_with(0x03, "Wireless24G", |_rid, cmd, sub, _tail| {
            asked += 1;
            match (cmd, sub) {
                (0x12, 0x07) => Some(fake_energy(84)),
                (0x12, 0x04) => None, // 第二帧偶发丢失
                (0x12, 0x00) => {
                    let mut m = vec![0u8; 64];
                    m[0] = 0x03;
                    m[1] = 0x12;
                    m[2] = 0x00;
                    m[12] = 2;
                    Some(m)
                }
                _ => None,
            }
        });

        assert_eq!(asked, 3, "第二帧丢失后仍要读第三帧");
        assert_eq!(out.frames_read, 2);
        assert_eq!(out.snapshot.failed.len(), 1);
        assert_eq!(out.snapshot.state.power, 84, "读到的电量照常保留");
        assert_eq!(out.snapshot.state.active_stage, 2);
    }

    /// NAK 不算 MISS，不该触发早退 —— 设备明确回了话，说明它是醒的。
    #[test]
    fn light_nak_is_not_a_miss() {
        let mut asked = 0;
        let out = fetch_light_with(0x03, "Wireless24G", |_rid, _cmd, _sub, _tail| {
            asked += 1;
            Some(vec![0x03, 0xFF, 0xAA])
        });

        assert_eq!(asked, 3, "NAK 是三帧都给否定应答，应该问完三帧");
        assert_eq!(out.frames_read, 0);
        assert_eq!(out.snapshot.failed.len(), 3);
    }
}
