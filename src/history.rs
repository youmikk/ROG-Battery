//! 电量历史采样 + 续航/充满时间估算。
//!
//! 逐条照搬 C# 版 `rog-battery.cs` 的 `RecordSampleLocked` / `PruneHistory` /
//! `PredictFromRows`，包括那些用真机数据试出来的阈值和文案。为什么不自作聪明重写：
//! 那些常量（60 秒 / 600 秒 / 7200 秒断档 / 100 小时上限）每一条都对应一次踩坑记录，
//! 注释里都写着当时的现象。换算法就等于把那些坑重新踩一遍。
//!
//! **路径故意叫 `rog-battery` 而不是 `rog-mouse-ctl`**：要复用 C# 版攒下的历史数据，
//! 另起炉灶等于把过去几天的样本扔掉。

use std::path::PathBuf;
use std::sync::Mutex;

use crate::win32;

// ==================== 常量（逐字照抄 C# 版） ====================

/// 保留最近 48 小时（覆盖两晚）。
pub const HIST_KEEP_HOURS: i64 = 48;
/// 文件行数上限，防止无限增长。
pub const HIST_MAX_LINES: usize = 2000;
/// 电量变化时，两条采样至少隔 60 秒。
pub const HIST_MIN_GAP_SEC: i64 = 60;
/// 电量没变时，每 2 分钟记一条心跳。
///
/// 原来是 600 秒（10 分钟）。真机上电量是整 1% 一跳，10 分钟一条心跳要攒够
/// 「3 条 + 跨度 ≥1%」往往得等一两个小时，于是界面长期停在「估算中」。
/// 改成 120 秒后同样的时长能多 5 倍样本点，斜率收敛明显更快；48 小时窗口
/// 内的行数仍远低于 [`HIST_MAX_LINES`]（心跳行 1440 条/48h，加上电量变化
/// 行也还在 2000 以内，超了会自动裁剪最旧的）。
pub const HIST_SAME_GAP_SEC: i64 = 120;
/// 相邻样本间隔超过这个值就认为「中间没在观察」，回归不能用这段跨度。
///
/// 取 2 小时：正常心跳 10 分钟一条，就算把刷新间隔调到 30 分钟也远不到这里；
/// 而实测碰到的那个坑是 6 小时空档（程序退出 / 机器睡眠），必须拦下来。
pub const HIST_GAP_BREAK_SEC: i64 = 7200;

/// 命名互斥体名。**不能改** —— C# 版随时可能回来，改了就互斥失效。
pub const MUTEX_NAME: &str = "Local\\RogBatteryHistory";
/// 等锁上限。拿不到就放弃本次写入，不阻塞调用方。
pub const MUTEX_TIMEOUT_MS: u32 = 3000;

/// 外推上限（小时）。
///
/// 原来取 100 小时，本意是不让界面出现「还能用 137 小时」这种没人信的数字。
/// 但真机实测：这只鼠标 1 小时 48 分才掉 1%（≈0.55 %/h），按 97% 外推就是
/// **176 小时** —— 完全正常的数据被这个上限判成「异常（请忽略）」，用户看到的
/// 就是永远的「估算中」。所以放到 1000 小时（≈41 天），再由 [`COARSE_HOURS`]
/// 用「10 天以上」这种粗粒度表达「很久」，而不是报错。
const MAX_PREDICT_HOURS: f64 = 1000.0;

/// 超过这个小时数就只说「N 天以上」。
///
/// 早期外推误差极大（2% 的跌幅推剩下 97%，真实值可能差三倍），显示到「小时」
/// 是假装精确。粗粒度既诚实又不吓人。
const COARSE_HOURS: f64 = 240.0;

// ==================== 数据行 ====================

/// 一条历史采样：`<unix 秒>\t<电量 0-100>\t<charging 0|1>\t<mode>`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryRow {
    pub ts: i64,
    pub level: u8,
    /// 设备报告的**原始** charging_status，不是归并后的三态。
    pub charging: bool,
    /// 连接方式（1=有线 2=2.4G 3=蓝牙），判定「已充满」要用它。
    pub mode: i32,
}

impl HistoryRow {
    pub fn new(ts: i64, level: u8, charging: bool, mode: i32) -> Self {
        Self {
            ts,
            level,
            charging,
            mode,
        }
    }

    fn to_line(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}",
            self.ts,
            self.level,
            if self.charging { 1 } else { 0 },
            self.mode
        )
    }
}

// ==================== 路径 ====================

/// `%LOCALAPPDATA%\rog-battery\history.tsv`
///
/// 目录名沿用 C# 版的名字，为了读它攒下的数据。
///
/// 这是**首选**路径，纯函数、不做 IO。真正用哪个由 [`history_path`] 决定。
pub fn primary_history_path() -> PathBuf {
    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        let base = PathBuf::from(base);
        if !base.as_os_str().is_empty() {
            return base.join("rog-battery").join("history.tsv");
        }
    }
    crate::paths::cache_dir().join("history.tsv")
}

/// 本次进程实际使用的历史文件路径（首次调用时定下来，之后不变）。
///
/// 为什么要探测：`%LOCALAPPDATA%` 在某些受限环境里对普通进程只读
/// （写入一律 `os error 5 拒绝访问`，连在同一目录新建文件都不行）。
/// 那时如果还死守着它，采样一条也写不进去，续航估算就永远停在
/// 「样本不足」—— 用户看到的是功能坏了，其实是路径不可写。
///
/// 顺序：`%LOCALAPPDATA%\rog-battery\` → 程序所在目录 → 缓存目录。
/// 只有前一个**真的写不进去**才往后掉，正常机器上永远是第一个。
pub fn history_path() -> PathBuf {
    if let Ok(guard) = resolved_cell().lock() {
        if let Some(p) = guard.as_ref() {
            return p.clone();
        }
    }
    let p = match read_custom_history_path() {
        Some(custom) => custom,
        None => resolve_history_path_default(),
    };
    if let Ok(mut guard) = resolved_cell().lock() {
        *guard = Some(p.clone());
    }
    p
}

fn custom_history_pointer() -> PathBuf {
    crate::paths::config_dir().join("custom_history_path.txt")
}

/// 用户在设置里点「更改」时随时改历史文件路径（进程内立即生效）。
pub fn set_custom_history_path(new_path: Option<PathBuf>) -> Result<(), String> {
    let pointer = custom_history_pointer();
    let resolved = match new_path {
        Some(p) => {
            let _ = crate::paths::ensure_dir(&crate::paths::config_dir());
            std::fs::write(&pointer, p.to_string_lossy().as_bytes())
                .map_err(|e| format!("写入历史文件重定向记录失败：{e}"))?;
            p
        }
        None => {
            let _ = std::fs::remove_file(&pointer);
            resolve_history_path_default()
        }
    };
    set_resolved_history_path(resolved)
}

/// 用全局静态变量保存当前实际路径，支持运行时热更新。
fn resolved_cell() -> &'static std::sync::Mutex<Option<PathBuf>> {
    static CELL: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> = std::sync::OnceLock::new();
    CELL.get_or_init(|| std::sync::Mutex::new(None))
}

fn set_resolved_history_path(p: PathBuf) -> Result<(), String> {
    let mut guard = resolved_cell().lock().map_err(|_| "历史路径锁中毒".to_string())?;
    *guard = Some(p);
    Ok(())
}

fn read_custom_history_path() -> Option<PathBuf> {
    if let Ok(text) = std::fs::read_to_string(custom_history_pointer()) {
        let t = text.trim();
        if !t.is_empty() {
            return Some(PathBuf::from(t));
        }
    }
    None
}

/// 当前用到的路径是不是降级路径（不是 `%LOCALAPPDATA%\rog-battery`）。
pub fn history_path_is_fallback() -> bool {
    !history_path_is_custom() && history_path() != primary_history_path()
}

/// 用户是否手动指定了历史文件路径。
pub fn history_path_is_custom() -> bool {
    read_custom_history_path().is_some()
}

fn probe_writable(p: &std::path::Path) -> bool {
    if let Some(dir) = p.parent() {
        let _ = crate::paths::ensure_dir(dir);
    }
    // create+append 能打开就认为可写。打不开会顺手留下一个空文件，
    // 这是有意的：空的 history.tsv 和不存在等价，`read_rows` 读出来就是空表。
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
        .is_ok()
}

fn resolve_history_path_default() -> PathBuf {
    let primary = primary_history_path();
    if probe_writable(&primary) {
        return primary;
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("history.tsv"));
        }
    }
    candidates.push(crate::paths::cache_dir().join("history.tsv"));

    for c in candidates {
        if c == primary {
            continue;
        }
        if probe_writable(&c) {
            // 首选路径里已有的历史不能丢：搬一份过来（探测会先把文件建成空文件，
            // 所以这里用长度判断"原本有没有数据"）。
            let empty = std::fs::metadata(&c).map(|m| m.len() == 0).unwrap_or(true);
            if primary.exists() && empty {
                let _ = std::fs::copy(&primary, &c);
            }
            return c;
        }
    }
    // 都不行：还报首选路径，让错误信息指向用户预期的位置
    primary
}

/// 给诊断输出用：当前解析到的历史文件路径。
pub fn history_path_display() -> String {
    history_path().display().to_string()
}

// ==================== 解析 ====================

/// 解析历史文本。坏行静默跳过（老文件可能被别的工具改过，不值得整体报错）。
///
/// 判据与 C# 版一致：字段数 ≥4、四个字段都能解析成数字、电量在 0..=100。
pub fn parse_rows(text: &str) -> Vec<HistoryRow> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let s = raw.trim();
        if s.is_empty() {
            continue;
        }
        let f: Vec<&str> = s.split('\t').collect();
        if f.len() < 4 {
            continue;
        }
        let (Ok(ts), Ok(level), Ok(chg), Ok(mode)) = (
            f[0].parse::<i64>(),
            f[1].parse::<i32>(),
            f[2].parse::<i32>(),
            f[3].parse::<i32>(),
        ) else {
            continue;
        };
        if !(0..=100).contains(&level) {
            continue;
        }
        out.push(HistoryRow::new(ts, level as u8, chg != 0, mode));
    }
    out
}

/// 读历史文件。文件不存在 / 读不了都返回空（这不是错误，第一次运行就是这样）。
///
/// **读完会按时间戳稳定排序**。文件是多个进程共写的：`record_sample_at` 有
/// `Local\RogBatteryHistory` 互斥体保护，但任何外部工具、脚本、手工编辑都能
/// 把顺序弄乱。下游的 [`predict_from_rows`] 有一堆「从最新往回取」的假设
/// （`prev_t - r.ts > HIST_GAP_BREAK_SEC` 断档检测、`filter_direction` 逐条回看），
/// 一旦输入乱序，断档检测会因为差值是负数而永不触发，`span_minutes` 也会算小，
/// 估算偏乐观。排序成本 2000 行的 O(n log n)，可以忽略。
///
/// 用 `sort_by_key` 而不是 `sort_unstable_by_key`：稳定排序能保住
/// 「同一时刻的两条记录」的原始写入顺序，便于事后排查。
pub fn read_rows() -> Vec<HistoryRow> {
    match std::fs::read_to_string(history_path()) {
        Ok(text) => {
            let mut rows = parse_rows(&text);
            rows.sort_by_key(|r| r.ts);
            rows
        }
        Err(_) => Vec::new(),
    }
}

// ==================== 跨进程互斥 ====================

/// 命名互斥体的 RAII 守卫。`Drop` 负责 `ReleaseMutex` + `CloseHandle`。
struct MutexGuard(win32::Handle);

impl Drop for MutexGuard {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != win32::INVALID_HANDLE_VALUE {
            unsafe {
                win32::ReleaseMutex(self.0);
                win32::CloseHandle(self.0);
            }
        }
    }
}

/// 拿 `Local\RogBatteryHistory`。
///
/// 拿不到（超时）返回 `None`，调用方应当**直接放弃本次写入**而不是阻塞等。
///
/// `WAIT_ABANDONED` 必须当成「拿到」：它表示上一个持有者崩溃了，内核把互斥体
/// 转交给我们。若按失败处理，一次崩溃就会让历史记录功能永久失灵。
fn lock_history() -> Option<MutexGuard> {
    let name = win32::to_wide(MUTEX_NAME);
    unsafe {
        let h = win32::CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr());
        if h.is_null() || h == win32::INVALID_HANDLE_VALUE {
            return None;
        }
        let wait = win32::WaitForSingleObject(h, MUTEX_TIMEOUT_MS);
        // 逐个列举而不是 `if a || b`：把 WAIT_ABANDONED 的处理意图写在匹配臂上，
        // 以后有人想加分支时不会漏掉它。
        match wait {
            // 正常拿到
            win32::WAIT_OBJECT_0 => Some(MutexGuard(h)),
            // 上一个持有者崩了，内核把互斥体转交给我们 —— 这是**成功**，不是失败
            win32::WAIT_ABANDONED => Some(MutexGuard(h)),
            // 超时（或其它意外）：关句柄走人。没拿到锁不能 ReleaseMutex。
            win32::WAIT_TIMEOUT | _ => {
                win32::CloseHandle(h);
                None
            }
        }
    }
}

// ==================== 写入 ====================

/// 判断这条采样要不要写，以及为什么。
///
/// 规则（照抄 C# 版 `RecordSampleLocked`）：
/// - 电量变了或相位变了：至少隔 [`HIST_MIN_GAP_SEC`] 秒
/// - 都没变：每 [`HIST_SAME_GAP_SEC`] 秒记一条心跳
///
/// 为什么要心跳：鼠标在 100% 趴一夜，每分钟一条会把 2000 行全占满同一个值，
/// 真正有用的变化点反而被挤出去；但完全不记又撑不住时间跨度。
pub fn should_record(rows: &[HistoryRow], level: u8, charging: bool, now_sec: i64) -> bool {
    let Some(last) = rows.last() else {
        return true; // 空文件，第一条无条件写
    };
    let gap = now_sec - last.ts;
    let level_changed = last.level != level;
    let phase_changed = last.charging != charging;
    let need = if level_changed || phase_changed {
        HIST_MIN_GAP_SEC
    } else {
        HIST_SAME_GAP_SEC
    };
    gap >= need
}

/// 裁剪结果。
pub struct PruneOutcome {
    pub rows: Vec<HistoryRow>,
    /// 是否真的需要重写文件。单纯追加时为 `false` —— 不要为了「顺便整理」去重写，
    /// 那会在别人正在读的瞬间把文件截断。
    pub rewrite: bool,
    /// 被丢掉的行数。目前只给单测用（`prune_by_count` / `prune_by_time`），
    /// 诊断输出暂时没用到，留着是因为算它是零成本的。
    #[allow(dead_code)]
    pub dropped: usize,
}

/// 按时间窗口和行数上限裁剪。
pub fn prune_rows(rows: &[HistoryRow], now_sec: i64) -> PruneOutcome {
    let cutoff = now_sec - HIST_KEEP_HOURS * 3600;
    let mut kept: Vec<HistoryRow> = Vec::with_capacity(rows.len());
    let mut dropped = 0usize;
    for r in rows {
        if r.ts < cutoff {
            dropped += 1;
            continue;
        }
        kept.push(*r);
    }
    // 没东西可删就别重写文件
    let mut rewrite = dropped > 0;
    if kept.len() > HIST_MAX_LINES {
        let extra = kept.len() - HIST_MAX_LINES;
        kept.drain(0..extra);
        dropped += extra;
        rewrite = true;
    }
    PruneOutcome {
        rows: kept,
        rewrite,
        dropped,
    }
}

/// 写一条采样。返回是否真的写了（被去重规则挡下时返回 `Ok(false)`）。
///
/// `now_sec` 由调用方给，便于单测灌合成时间轴。
pub fn record_sample_at(
    level: u8,
    charging: bool,
    mode: i32,
    now_sec: i64,
) -> Result<bool, String> {
    if level > 100 {
        return Err(format!("电量 {level} 超出 0-100，拒绝写入历史"));
    }

    let _guard = match lock_history() {
        Some(g) => g,
        // 拿不到锁就放弃本次写入。宁可丢一条样本，也不要在托盘进程里卡 3 秒。
        None => return Ok(false),
    };

    let path = history_path();
    if let Some(dir) = path.parent() {
        crate::paths::ensure_dir(dir)?;
    }

    let rows = read_rows();
    if !should_record(&rows, level, charging, now_sec) {
        return Ok(false);
    }

    let new_row = HistoryRow::new(now_sec, level, charging, mode);
    let line = new_row.to_line();
    append_line(&path, &line)?;

    // 先追加再裁剪：万一裁剪失败，至少样本已经落盘了。
    //
    // ⚠️ 裁剪必须拿**含刚追加那条**的列表去算。以前这里传的是 `&rows`（追加前的
    // 快照），于是每次 rewrite 都会把刚写进去的那条覆盖掉；如果 `rows` 已经全部
    // 超出 48 小时窗口，裁剪结果就是空表 —— 文件被改写成 0 字节，样本永远攒不起来，
    // 界面上就永远显示「样本不足」。
    let outcome = prune_after_append(&rows, new_row, now_sec);
    if outcome.rewrite {
        write_all(&path, &outcome.rows)?;
    }
    Ok(true)
}

/// 追加一条之后的裁剪：把新样本并进列表再算，避免「刚写的被裁剪覆盖掉」。
fn prune_after_append(rows: &[HistoryRow], new_row: HistoryRow, now_sec: i64) -> PruneOutcome {
    let mut all = Vec::with_capacity(rows.len() + 1);
    all.extend_from_slice(rows);
    all.push(new_row);
    prune_rows(&all, now_sec)
}

/// 写一条采样，用真实当前时间。
pub fn record_sample(level: u8, charging: bool, mode: i32) -> Result<bool, String> {
    record_sample_at(level, charging, mode, crate::snapshot::now_unix() as i64)
}

fn append_line(path: &std::path::Path, line: &str) -> Result<(), String> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("无法打开历史文件 {}：{e}", path.display()))?;
    f.write_all(line.as_bytes())
        .and_then(|_| f.write_all(b"\n"))
        .map_err(|e| format!("写历史文件失败 {}：{e}", path.display()))
}

fn write_all(path: &std::path::Path, rows: &[HistoryRow]) -> Result<(), String> {
    let mut text = String::with_capacity(rows.len() * 20);
    for r in rows {
        text.push_str(&r.to_line());
        text.push('\n');
    }
    std::fs::write(path, text).map_err(|e| format!("重写历史文件失败 {}：{e}", path.display()))
}

// ==================== 三态 ====================

/// 设备报告的 charging_status 与连接方式归并后的三态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChargeState {
    Discharging,
    Charging,
    Full,
}

/// 归并规则照抄 C# 版 `DecideCharge`：
///
/// 真机日志显示 `charging_status=1` 只出现在 98%、99% 两次，一到 100% 就变 0
/// —— 设备只在「真的在往里充」时报 1，充满后回 0。所以 100% 时必须靠
/// 「有线在供电」判「已充满」，否则用户永远看不到充电相关的字。
pub fn decide_charge(level: u8, charging: bool, mode: i32) -> ChargeState {
    if charging {
        return ChargeState::Charging;
    }
    // 有线模式 = 1（hid::Link::mode 的 "Wired"）
    if mode == 1 && level >= 100 {
        return ChargeState::Full;
    }
    ChargeState::Discharging
}

// ==================== 估算 ====================

/// 估算结果。`known == false` 时 `text` 说明原因，**不要假装有结论**。
#[derive(Clone, Debug, Default)]
pub struct Estimate {
    pub known: bool,
    /// 可直接显示的整句。
    pub text: String,
    /// 正 = 在涨，负 = 在掉（%/小时）。
    pub percent_per_hour: f64,
    pub samples: usize,
    pub span_minutes: f64,
    /// 剩余（放电）或充满（充电）所需小时数；仅 `known == true` 时有意义。
    pub hours: f64,
    /// 这条结论是「沿用」来的：距上一次成功估算过了多少秒。0 = 本次新鲜算出的。
    pub stale_seconds: i64,
}

impl Estimate {
    /// 环下方那行短结论。`text` 是给「明细」用的整句（"预计还能用 3 小时 12 分钟"），
    /// 环下面那格横向只有 ~150px，放整句会挤，所以另出一版紧凑文案。
    pub fn short_text(&self) -> String {
        if !self.known {
            return if self.text == "已充满" {
                "已充满".to_string()
            } else {
                "估算中…".to_string()
            };
        }
        let d = dur_short(self.hours);
        if self.percent_per_hour > 0.0 {
            format!("约 {d}后充满")
        } else {
            format!("约 {d}")
        }
    }
}

/// 「沿用」的上限：30 分钟内有过可信结论才继续显示。
///
/// 再久就不该拿旧斜率骗人了 —— 中间可能插过线、换过档、休眠过。
const STICKY_MAX_AGE_SEC: i64 = 1800;

/// 最近一次成功估算。用于 [`predict_sticky`] 的「沿用」。
static LAST_GOOD: Mutex<Option<(Estimate, i64)>> = Mutex::new(None);

/// 把秒数说成人话（"刚刚" / "12 分钟" / "1 小时"）。
pub fn ago_text(sec: i64) -> String {
    if sec < 60 {
        "刚刚".to_string()
    } else if sec < 3600 {
        format!("{} 分钟", sec / 60)
    } else {
        format!("{} 小时", sec / 3600)
    }
}

/// 带「沿用」的预测：计算本身仍交给 [`predict_from_rows`]，这里只加一层缓存。
///
/// 为什么需要：一次刷新里算不出来（刚跳变 / 刚插拔 / 一瞬间读不到设备）时，
/// 界面会立刻从「约 7 天」掉回「估算中…」，观感上就是"随时都在估算"。
/// 30 分钟内有过结论就继续用那一条，并用 `stale_seconds` 如实标注，不假装是新的。
pub fn predict_sticky(
    rows: &[HistoryRow],
    now_level: u8,
    now_charging: bool,
    now_mode: i32,
    now_sec: i64,
) -> Estimate {
    let fresh = predict_from_rows(rows, now_level, now_charging, now_mode, now_sec);
    let mut slot = LAST_GOOD.lock().unwrap_or_else(|e| e.into_inner());
    if fresh.known {
        *slot = Some((fresh.clone(), now_sec));
        return fresh;
    }
    if let Some((cached, at)) = slot.as_ref() {
        let age = now_sec - *at;
        if (0..=STICKY_MAX_AGE_SEC).contains(&age) {
            let mut e = cached.clone();
            e.stale_seconds = age;
            return e;
        }
    }
    fresh
}

/// 从历史行推算续航/充满时间，**不做文件 IO**。
///
/// `now_sec` 由调用方给：否则「补当前读数」那步会用到真实时间，和合成样本的
/// 时序打架，单测就没法写。
pub fn predict_from_rows(
    rows: &[HistoryRow],
    now_level: u8,
    now_charging: bool,
    now_mode: i32,
    now_sec: i64,
) -> Estimate {
    // 已充满：没什么可预测的
    if decide_charge(now_level, now_charging, now_mode) == ChargeState::Full {
        return Estimate {
            known: false,
            text: "已充满".to_string(),
            ..Default::default()
        };
    }

    // 第一步：取出「连续同相位」的一段（先不管方向）
    let phase = filter_phase(rows, now_charging);

    // 同相位样本太少 → 说明刚切换过充电/放电，历史还没攒够。
    // 这时不要说「样本不足」糊弄过去，要告诉用户真实原因。
    if phase.len() < 3 {
        return Estimate {
            samples: phase.len(),
            text: if rows.len() >= 3 {
                just_switched_text(now_charging)
            } else {
                not_enough_text(now_charging)
            },
            ..Default::default()
        };
    }

    // 第二步：当前读数和这段历史对不上（相差超过 1%）→ 读数跳变，历史作废
    let last_level = phase[phase.len() - 1].level;
    let conflict = if now_charging {
        now_level + 1 < last_level
    } else {
        now_level > last_level + 1
    };
    if conflict {
        return Estimate {
            samples: phase.len(),
            text: jumped_text(now_charging),
            ..Default::default()
        };
    }

    // 第三步：方向过滤。放电段里出现回升（或充电段里出现回落）说明中途插过线
    // 或传感器抖动，只保留最近这段干净的。
    let sel = filter_direction(&phase, now_charging);
    if sel.len() < 3 {
        return Estimate {
            samples: sel.len(),
            text: jumped_text(now_charging),
            ..Default::default()
        };
    }

    // 第四步：把当前读数并进序列末尾。历史最后一条可能是十几分钟前的，
    // 不并进去等于拿旧数据预测现在。
    let mut full = sel;
    if let Some(last) = full.last() {
        if last.ts < now_sec {
            full.push(HistoryRow::new(now_sec, now_level, now_charging, now_mode));
        }
    }

    predict_from_series(&full, now_level, now_charging, now_mode)
}

/// 纯函数核心：给定已含当前读数的升序样本，算斜率。
fn predict_from_series(
    sel: &[HistoryRow],
    now_level: u8,
    now_charging: bool,
    now_mode: i32,
) -> Estimate {
    if decide_charge(now_level, now_charging, now_mode) == ChargeState::Full {
        return Estimate {
            text: "已充满".to_string(),
            ..Default::default()
        };
    }
    if sel.len() < 3 {
        return Estimate {
            samples: sel.len(),
            text: not_enough_text(now_charging),
            ..Default::default()
        };
    }

    let x0 = sel[0].ts as f64;
    let x1 = sel[sel.len() - 1].ts as f64;
    let span_minutes = (x1 - x0) / 60.0;

    // 用极差判断「到底动过没有」。只比首尾会漏掉「先降后升」那种情形：
    // 那种情况下首尾相等，但数据里明明有变化。
    let mut lo = f64::MAX;
    let mut hi = f64::MIN;
    for r in sel {
        let v = r.level as f64;
        if v < lo {
            lo = v;
        }
        if v > hi {
            hi = v;
        }
    }
    if hi - lo < 1.0 {
        return Estimate {
            samples: sel.len(),
            span_minutes,
            text: no_change_text(now_charging),
            ..Default::default()
        };
    }

    // 最小二乘：x 用小时，y 用电量百分点
    let n = sel.len() as f64;
    let (mut sx, mut sy, mut sxy, mut sxx) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for r in sel {
        let xh = (r.ts as f64 - x0) / 3600.0;
        let y = r.level as f64;
        sx += xh;
        sy += y;
        sxy += xh * y;
        sxx += xh * xh;
    }
    let den = n * sxx - sx * sx;
    if den.abs() < 1e-9 {
        return Estimate {
            samples: sel.len(),
            span_minutes,
            text: span_too_short_text(now_charging),
            ..Default::default()
        };
    }
    let slope = (n * sxy - sx * sy) / den; // %/小时

    if slope.abs() < 0.05 {
        return Estimate {
            samples: sel.len(),
            span_minutes,
            text: too_slow_text(now_charging),
            ..Default::default()
        };
    }
    // 方向和当前状态相反，说明刚切过充电/放电，样本混了
    if now_charging && slope <= 0.0 {
        return Estimate {
            samples: sel.len(),
            span_minutes,
            text: "预计充满时间：刚插上电，还在观察".to_string(),
            ..Default::default()
        };
    }
    if !now_charging && slope >= 0.0 {
        return Estimate {
            samples: sel.len(),
            span_minutes,
            text: "预计可用时间：刚拔掉线，还在观察".to_string(),
            ..Default::default()
        };
    }

    let level = now_level as f64;
    let hours = if now_charging {
        (100.0 - level) / slope
    } else {
        level / (-slope)
    };
    if hours < 0.0 || hours > MAX_PREDICT_HOURS {
        return Estimate {
            samples: sel.len(),
            span_minutes,
            text: if now_charging {
                "预计充满时间：异常（请忽略）".to_string()
            } else {
                "预计可用时间：异常（请忽略）".to_string()
            },
            ..Default::default()
        };
    }

    let dur = if hours >= COARSE_HOURS {
        "10 天以上".to_string()
    } else {
        dur_text(hours)
    };
    Estimate {
        known: true,
        text: if now_charging {
            format!("预计 {dur} 后充满")
        } else {
            format!("预计还能用 {dur}")
        },
        percent_per_hour: slope,
        samples: sel.len(),
        span_minutes,
        hours,
        stale_seconds: 0,
    }
}

/// 从最新往回取「连续同一相位」的样本（不做方向判断）。
fn filter_phase(rows: &[HistoryRow], charging: bool) -> Vec<HistoryRow> {
    let mut sel: Vec<HistoryRow> = Vec::new();
    let mut prev_t: Option<i64> = None;
    for r in rows.iter().rev() {
        if r.charging != charging {
            break;
        }
        // 断档检测：两条样本间隔超过 HIST_GAP_BREAK_SEC 说明中间根本没在观察
        // （程序没运行 / 机器睡眠 / 鼠标长时间不响应）。这种跨度不能当连续观测用
        // —— 实测遇到过 6 小时空档，两头差 4%，回归把它当成「6 小时均匀掉 4%」，
        // 于是算出一个好看但完全错的斜率。碰到断档就截断，只用断档之后这一段。
        if let Some(pt) = prev_t {
            if pt - r.ts > HIST_GAP_BREAK_SEC {
                break;
            }
        }
        sel.push(*r);
        prev_t = Some(r.ts);
    }
    sel.reverse();
    sel
}

/// 从最新往回取「方向也一致」的一段：放电样本往回看应单调不降，充电样本相反。
/// 碰到第一次反向就截断。
fn filter_direction(rows: &[HistoryRow], charging: bool) -> Vec<HistoryRow> {
    let mut sel: Vec<HistoryRow> = Vec::new();
    let mut prev: Option<u8> = None;
    for r in rows.iter().rev() {
        if let Some(p) = prev {
            let ok = if charging { r.level <= p } else { r.level >= p };
            if !ok {
                break;
            }
        }
        sel.push(*r);
        prev = Some(r.level);
    }
    sel.reverse();
    sel
}

fn dur_text(hours: f64) -> String {
    let mut total_min = (hours * 60.0).round() as i64;
    if total_min < 1 {
        total_min = 1;
    }
    let days = total_min / (24 * 60);
    let rem = total_min % (24 * 60);
    let h = rem / 60;
    let m = rem % 60;
    if days > 0 {
        if h == 0 {
            format!("{days} 天")
        } else {
            format!("{days} 天 {h} 小时")
        }
    } else if h == 0 {
        format!("{m} 分钟")
    } else if m == 0 {
        format!("{h} 小时")
    } else {
        format!("{h} 小时 {m} 分钟")
    }
}

/// `dur_text` 的紧凑版：环下面那格只有一百多像素，省掉「钟」字就够用。
fn dur_short(hours: f64) -> String {
    let mut total_min = (hours * 60.0).round() as i64;
    if total_min < 1 {
        total_min = 1;
    }
    let days = total_min / (24 * 60);
    let rem = total_min % (24 * 60);
    let h = rem / 60;
    let m = rem % 60;
    if days > 0 {
        if h == 0 {
            format!("{days} 天")
        } else {
            format!("{days} 天 {h} 小时")
        }
    } else if h == 0 {
        format!("{m} 分钟")
    } else if m == 0 {
        format!("{h} 小时")
    } else {
        format!("{h} 小时 {m} 分")
    }
}

// ---- 文案：用户看惯了这些句子，别润色 ----

fn not_enough_text(charging: bool) -> String {
    if charging {
        "预计充满时间：样本不足".to_string()
    } else {
        "预计可用时间：样本不足".to_string()
    }
}

fn just_switched_text(charging: bool) -> String {
    if charging {
        "预计充满时间：刚切换，重新观察中".to_string()
    } else {
        "预计可用时间：刚切换，重新观察中".to_string()
    }
}

fn jumped_text(charging: bool) -> String {
    if charging {
        "预计充满时间：电量刚跳变，重新观察中".to_string()
    } else {
        "预计可用时间：电量刚跳变，重新观察中".to_string()
    }
}

fn no_change_text(charging: bool) -> String {
    if charging {
        "预计充满时间：电量还没变化（需要再多观察一会儿）".to_string()
    } else {
        "预计可用时间：电量还没变化（需要再多观察一会儿）".to_string()
    }
}

fn span_too_short_text(charging: bool) -> String {
    if charging {
        "预计充满时间：数据跨度太短".to_string()
    } else {
        "预计可用时间：数据跨度太短".to_string()
    }
}

fn too_slow_text(charging: bool) -> String {
    if charging {
        "预计充满时间：变化太慢，暂时测不出".to_string()
    } else {
        "预计可用时间：变化太慢，暂时测不出".to_string()
    }
}

// ==================== 单测 ====================

#[cfg(test)]
mod tests {
    use super::*;

    /// 合成时间轴：起点用一个固定时刻，避免测试依赖真实时间。
    const T0: i64 = 1_800_000_000;

    fn row(ts: i64, level: u8, charging: bool) -> HistoryRow {
        HistoryRow::new(ts, level, charging, 2)
    }

    #[test]
    fn parse_real_file_shape() {
        // 就取本机现有那 4 行的真实内容
        let text = "1792568899\t89\t0\t2\n1792569212\t88\t0\t2\n\
                    1792576904\t87\t0\t2\n1792593756\t84\t0\t2\n";
        let rows = parse_rows(text);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0], HistoryRow::new(1792568899, 89, false, 2));
        assert_eq!(rows[3], HistoryRow::new(1792593756, 84, false, 2));
    }

    #[test]
    fn parse_crlf_file() {
        // 真机上这个文件是 C# 版 `File.WriteAllLines` 写的，行尾是 CRLF。
        // 我们追加时写 LF（和 C# 的 `AppendAllText` 一致），于是文件里混着两种行尾。
        // 解析必须都吃得下 —— 靠的是先 `trim()` 再去 split('\t')，
        // 否则最后那个字段会变成 "2\r"、parse::<i32> 失败、整行被当成坏行丢掉。
        let text = "1792568899\t89\t0\t2\r\n1792569212\t88\t1\t2\r\n";
        let rows = parse_rows(text);
        assert_eq!(rows.len(), 2, "CRLF 行尾必须能解析");
        assert_eq!(rows[0], HistoryRow::new(1792568899, 89, false, 2));
        assert_eq!(rows[1], HistoryRow::new(1792569212, 88, true, 2));
    }

    #[test]
    fn appended_line_is_lf_only() {
        // 明确锁定我们自己的写入格式：只写 LF，不做 CRLF 翻译。
        // Rust 的 File 永远以二进制方式打开，所以这里其实是在防「以后有人
        // 换成某个会做文本模式转换的封装」——那会让同一个文件出现三种行尾。
        //
        // 落盘位置用 `target/` 而不是 `%TEMP%`：本机开发环境（受限令牌）下
        // 测试进程在 `%TEMP%` 建目录/写文件都会返「拒绝访问 (os error 5)」，
        // 那是环境限制不是代码问题，但会让这条测试红着，掩盖真正的回归。
        // `target/` 一定可写（cargo 刚往里写过东西），且随 `cargo clean` 清掉。
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("test-scratch");
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join(format!("hist-{}.tsv", std::process::id()));
        let _ = std::fs::remove_file(&path);

        append_line(&path, "100\t50\t0\t2").expect("追加");
        let bytes = std::fs::read(&path).expect("读回");
        assert_eq!(bytes, b"100\t50\t0\t2\n", "应当是 LF 结尾且没有多余字节");

        // 再读回来必须得到一条合法记录
        let text = String::from_utf8(bytes).expect("应当是 ASCII");
        let rows = parse_rows(&text);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], HistoryRow::new(100, 50, false, 2));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parse_skips_bad_lines() {
        let text = "100\t50\t0\t2\n\n乱码\t1\t0\t2\n200\t101\t0\t2\n300\t40\t0\n";
        let rows = parse_rows(text);
        assert_eq!(rows.len(), 1, "只应留下第一条合法行");
        assert_eq!(rows[0].ts, 100);
    }

    #[test]
    fn dedupe_same_level() {
        let rows = vec![row(T0, 84, false)];
        // 电量没变，2 分钟内不写
        assert!(!should_record(&rows, 84, false, T0 + 60));
        // 超过 120 秒（HIST_SAME_GAP_SEC）才记心跳
        assert!(should_record(&rows, 84, false, T0 + 121));
    }

    #[test]
    fn dedupe_level_changed() {
        let rows = vec![row(T0, 84, false)];
        // 电量变了，30 秒还不写
        assert!(!should_record(&rows, 83, false, T0 + 30));
        // 过了 60 秒才写
        assert!(should_record(&rows, 83, false, T0 + 61));
    }

    #[test]
    fn dedupe_phase_changed_counts_as_change() {
        let rows = vec![row(T0, 84, false)];
        assert!(!should_record(&rows, 84, true, T0 + 30));
        assert!(should_record(&rows, 84, true, T0 + 61));
    }

    #[test]
    fn first_sample_always_written() {
        assert!(should_record(&[], 50, false, T0));
    }

    #[test]
    fn prune_by_count() {
        let rows: Vec<HistoryRow> = (0..2500)
            .map(|i| row(T0 + i as i64 * 60, 50, false))
            .collect();
        let now = T0 + 2500 * 60;
        let out = prune_rows(&rows, now);
        assert_eq!(out.rows.len(), HIST_MAX_LINES);
        assert!(out.rewrite);
        // 保留的是最近的
        assert_eq!(out.rows[out.rows.len() - 1].ts, rows[rows.len() - 1].ts);
    }

    #[test]
    fn prune_by_time() {
        let old = row(T0 - 49 * 3600, 60, false);
        let keep = row(T0 - 47 * 3600, 60, false);
        let rows = vec![old, keep];
        let out = prune_rows(&rows, T0);
        assert_eq!(out.rows.len(), 1);
        assert_eq!(out.rows[0].ts, keep.ts);
        assert!(out.rewrite);
    }

    #[test]
    fn prune_noop_does_not_rewrite() {
        let rows: Vec<HistoryRow> = (0..10).map(|i| row(T0 + i * 60, 50, false)).collect();
        let out = prune_rows(&rows, T0 + 600);
        assert!(!out.rewrite, "没什么可删的时候不应该重写文件");
        assert_eq!(out.rows.len(), 10);
    }

    #[test]
    fn prune_after_append_keeps_the_new_sample() {
        // 回归：历史里的行全部超出 48 小时窗口时，裁剪结果里**必须**还有刚追加的
        // 那条。以前传的是追加前的快照，裁剪出来是空表，于是文件被改写成 0 字节，
        // 采样永远攒不下来，界面永远「样本不足」。
        let rows = vec![row(T0 - 50 * 3600, 76, false), row(T0 - 49 * 3600, 76, false)];
        let fresh = row(T0, 98, false);
        let out = prune_after_append(&rows, fresh, T0);
        assert!(out.rewrite);
        assert_eq!(out.rows.len(), 1, "只剩新的那条");
        assert_eq!(out.rows[0].ts, T0);
        assert_eq!(out.rows[0].level, 98);
    }

    #[test]
    fn gap_breaks_phase() {
        // 6 小时空档两头差 4%：整段算斜率会得出「6 小时均匀掉 4%」这种假结论。
        // 断档之后只有 2 条样本，不足 3 条 → 不能说 known。
        let rows = vec![
            row(T0, 90, false),
            row(T0 + 60, 89, false),
            row(T0 + 120, 88, false),
            // 空档 6 小时后
            row(T0 + 6 * 3600, 86, false),
            row(T0 + 6 * 3600 + 60, 86, false),
        ];
        let now_sec = T0 + 6 * 3600 + 60;
        let e = predict_from_rows(&rows, 86, false, 2, now_sec);
        assert!(!e.known);
        assert!(e.samples < 3, "断档后只该剩 2 条，实际 {}", e.samples);
    }

    #[test]
    fn phase_switch_truncates() {
        // 充电段末尾接放电段：放电样本只有 1 条，必须截断
        let rows = vec![
            row(T0, 50, true),
            row(T0 + 600, 52, true),
            row(T0 + 1200, 54, true),
            row(T0 + 1800, 54, false),
        ];
        let e = predict_from_rows(&rows, 54, false, 2, T0 + 1800);
        assert!(!e.known);
        assert!(e.samples < 3, "放电段只该剩 1 条，实际 {}", e.samples);
        // 历史够 3 条但同相位不够 → 是「刚切换」而不是「样本不足」
        assert_eq!(e.text, "预计可用时间：刚切换，重新观察中");
    }

    #[test]
    fn direction_reverse_truncates() {
        // 放电段里出现一次回升 → 只保留回升之后那一段
        let rows = vec![
            row(T0, 60, false),
            row(T0 + 600, 59, false),
            row(T0 + 1200, 58, false),
            row(T0 + 1800, 59, false), // 回升
            row(T0 + 2400, 58, false),
            row(T0 + 3000, 57, false),
        ];
        let e = predict_from_rows(&rows, 57, false, 2, T0 + 3000);
        // 回升之后只剩 3 条（59 → 58 → 57），够算但要确认没用整段
        assert!(e.samples <= 3, "实际 {}", e.samples);
    }

    #[test]
    fn level_jump_invalidates() {
        let rows = vec![
            row(T0, 60, false),
            row(T0 + 600, 59, false),
            row(T0 + 1200, 58, false),
        ];
        // 当前读数比历史末条高 2% → 跳变
        let e = predict_from_rows(&rows, 60, false, 2, T0 + 1200);
        assert!(!e.known);
        assert_eq!(e.text, "预计可用时间：电量刚跳变，重新观察中");
    }

    #[test]
    fn too_few_samples() {
        let rows = vec![row(T0, 60, false), row(T0 + 600, 59, false)];
        let e = predict_from_rows(&rows, 59, false, 2, T0 + 600);
        assert!(!e.known);
        assert_eq!(e.text, "预计可用时间：样本不足");
    }

    #[test]
    fn linear_discharge_gives_sane_estimate() {
        // 每 10 分钟掉 1% → 5 小时掉 30%，从 80% 起算应剩约 13.3 小时
        let mut rows = Vec::new();
        for i in 0..=30i64 {
            rows.push(row(T0 + i * 600, 80 - i as u8, false));
        }
        let now_sec = T0 + 30 * 600;
        let e = predict_from_rows(&rows, 50, false, 2, now_sec);
        assert!(e.known, "应该能算出结论，实际：{}", e.text);
        assert!((e.percent_per_hour + 6.0).abs() < 0.2, "斜率 {}", e.percent_per_hour);
        // 50% / 6 %/h ≈ 8.3 小时
        assert!(e.text.starts_with("预计还能用"), "{}", e.text);
    }

    #[test]
    fn full_charge_reports_full() {
        // 有线 + 100% + 不在充电 → 已充满
        let e = predict_from_rows(&[], 100, false, 1, T0);
        assert!(!e.known);
        assert_eq!(e.text, "已充满");
    }

    #[test]
    fn decide_charge_matches_device_behavior() {
        // 设备只在真的在充时报 1
        assert_eq!(decide_charge(99, true, 2), ChargeState::Charging);
        // 100% 后回 0，靠有线判充满
        assert_eq!(decide_charge(100, false, 1), ChargeState::Full);
        // 无线 100% 不算充满（没插线）
        assert_eq!(decide_charge(100, false, 2), ChargeState::Discharging);
    }

    #[test]
    fn dur_text_formats() {
        assert_eq!(dur_text(0.0), "1 分钟");
        assert_eq!(dur_text(1.0), "1 小时");
        assert_eq!(dur_text(1.5), "1 小时 30 分钟");
        // 天：鼠标这种设备本来就该按天说
        assert_eq!(dur_text(30.0), "1 天 6 小时");
        assert_eq!(dur_text(48.0), "2 天");
        assert_eq!(dur_short(30.0), "1 天 6 小时");
        assert_eq!(dur_short(0.68), "41 分钟");
    }

    /// 真机现场：98% → 97% 用了 1 小时 48 分（≈0.55 %/h），外推 176 小时。
    /// 旧上限 100 小时把它判成「异常（请忽略）」，界面永远「估算中」。
    /// 用户报的就是这个：不能持久计算，随时都是估算中。
    #[test]
    fn slow_drain_still_gives_an_answer() {
        let rows = vec![
            row(T0, 98, false),
            row(T0 + 3600, 98, false),
            row(T0 + 6480, 97, false),
        ];
        let e = predict_from_rows(&rows, 97, false, 2, T0 + 6480);
        assert!(e.known, "慢放电必须能给出结论，实际 text={}", e.text);
        assert!(
            e.hours > 150.0 && e.hours < 220.0,
            "外推应约 176 小时，实际 {}",
            e.hours
        );
        assert!(e.text.contains("天"), "超过一天要按天说：{}", e.text);
        assert!(!e.text.contains("异常"), "不该再说异常：{}", e.text);
        assert!(e.short_text().starts_with("约 "), "{}", e.short_text());
    }

    /// 超过 10 天就只说「10 天以上」——早期外推装精确是骗人。
    #[test]
    fn very_long_estimate_is_coarse() {
        let rows = vec![
            row(T0, 99, false),
            row(T0 + 6000, 99, false),
            row(T0 + 12000, 99, false),
            row(T0 + 18000, 98, false),
        ];
        let e = predict_from_rows(&rows, 98, false, 2, T0 + 18000);
        assert!(e.known, "{}", e.text);
        assert!(e.hours > COARSE_HOURS, "{}", e.hours);
        assert_eq!(e.text, "预计还能用 10 天以上");
    }

    /// 「沿用」：本次算不出来时，30 分钟内的旧结论继续给，但要用 stale_seconds 标注。
    #[test]
    fn sticky_keeps_last_good_estimate() {
        let rows = vec![
            row(T0, 98, false),
            row(T0 + 3600, 98, false),
            row(T0 + 6480, 97, false),
        ];
        let first = predict_sticky(&rows, 97, false, 2, T0 + 6480);
        assert!(first.known && first.stale_seconds == 0, "{:?}", first);

        // 样本全没了（相当于刚清空历史 / 一瞬间读不到）—— 应沿用刚才那条
        let again = predict_sticky(&[], 97, false, 2, T0 + 6480 + 600);
        assert!(again.known, "应沿用上一条，实际 text={}", again.text);
        assert_eq!(again.stale_seconds, 600);
        assert_eq!(again.hours, first.hours);
    }

    #[test]
    fn history_path_uses_rog_battery_dir() {
        // 目录名故意不是 rog-mouse-ctl —— 要复用 C# 版的数据。
        // 用 primary_history_path：history_path 会按可写性降级，单测环境里
        // %LOCALAPPDATA% 可能不可写，拿它断言会误报。
        let p = primary_history_path().display().to_string();
        assert!(p.contains("rog-battery"), "{}", p);
        assert!(p.ends_with("history.tsv"), "{}", p);
    }
}
