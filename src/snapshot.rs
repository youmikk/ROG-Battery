
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
        if wall > 0 {
            let age = std::time::Duration::from_secs(unix_now().saturating_sub(wall));
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
) -> Option<std::sync::Arc<Snapshot>> {
    let s = slot.as_ref()?;
    if s.is_fresh(ttl) {
        Some(std::sync::Arc::clone(s))
    } else {
        None
    }
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
        if let Some(s) = cached_hit(&self.snapshot, self.ttl) {
            return Some(FetchOutcome {
                frames_read: s.frames.len(),
                elapsed: Duration::ZERO,
                snapshot: s,
            });
        }

        // 2) 轻量快照新鲜
        if let Some(s) = cached_hit(&self.light, self.ttl) {
            return Some(FetchOutcome {
                frames_read: s.frames.len(),
                elapsed: Duration::ZERO,
                snapshot: s,
            });
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
    pub fn get(
        &mut self,
        dev: &mut HidDevice,
        all_interfaces: &[crate::hid::HidInterface],
        report_id: u8,
        link_mode: &str,
    ) -> FetchOutcome {
        // 1) 内存
        if let Some(s) = cached_hit(&self.snapshot, self.ttl) {
            return FetchOutcome {
                frames_read: s.frames.len(),
                elapsed: Duration::ZERO,
                snapshot: s,
            };
        }

        // 2) 磁盘（跨进程复用，CLI 每次都重启，没有这层缓存就没意义）
        if let Some(s) = load_disk_cache() {
            let fresh = s.wall_age_sec() < self.ttl.as_secs();
            if fresh {
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
        let out = fetch(dev, all_interfaces, report_id, link_mode);
        if !out.snapshot.frames.is_empty() {
            save_disk_cache(&out.snapshot);
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
        let p = crate::paths::snapshot_cache_file();
        let _ = std::fs::remove_file(p);
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
