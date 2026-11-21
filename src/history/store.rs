//! 历史采样的**存储层**：路径、解析、跨进程互斥、写入与裁剪。
//!
//! 逐条照搬 C# 版 `rog-battery.cs` 的 `RecordSampleLocked` / `PruneHistory`，
//! 包括那些用真机数据试出来的阈值（60 秒去重 / 120 秒心跳 / 7200 秒断档 /
//! 48 小时窗口 / 2000 行上限）。每一条都对应一次踩坑记录，换算法等于把那些坑
//! 重新踩一遍。
//!
//! 路径故意叫 `rog-battery` 而不是 `rog-mouse-ctl`：要复用 C# 版攒下的历史数据。
//!
//! 上层：[`super::predict`]（纯计算）、[`super::format`]（纯文案）。

use std::path::PathBuf;

use crate::win32;

// ==================== 常量（逐字照抄 C# 版） ====================

/// 保留最近 48 小时（覆盖两晚）。
pub const HIST_KEEP_HOURS: i64 = 48;
/// 时间戳的合理下界（2020-01-01Z）。历史文件是纯文本、可被任何工具改，
/// 越界的行在下游减法里会溢出，读入时就该扔掉。
pub const TS_MIN_PLAUSIBLE: i64 = 1_577_836_800;
/// 时间戳的合理上界（2100-01-01Z）。
pub const TS_MAX_PLAUSIBLE: i64 = 4_102_444_800;
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
pub const HIST_GAP_BREAK_SEC: i64 = 7200;

/// 命名互斥体名。**不能改** —— C# 版随时可能回来，改了就互斥失效。
pub const MUTEX_NAME: &str = "Local\\RogBatteryHistory";
/// 等锁上限。拿不到就放弃本次写入，不阻塞调用方。
pub const MUTEX_TIMEOUT_MS: u32 = 3000;

/// 单行历史记录的最大字节数（`ts\tlevel\tcharging\tmode`，留足余量）。
///
/// 只用来估算「文件体积是否接近行数上限」，不需要精确。
const LINE_MAX_BYTES: usize = 24;
/// 每写这么多条就做一次整份裁剪。
///
/// 48 小时窗口按行时间判断，必须全量；但写采样只需要最后一行。折中是攒够一批
/// 再读全量，把「每条一次全量读」降成「每 40 条一次」。
const PRUNE_EVERY_WRITES: u32 = 40;
/// 距上次裁剪又写了几条。
static WRITES_SINCE_PRUNE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

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
        if let Some(r) = guard.as_ref() {
            return r.path.clone();
        }
    }
    let (path, custom) = match read_custom_history_path() {
        Some(custom) => (custom, true),
        None => (resolve_history_path_default(), false),
    };
    if let Ok(mut guard) = resolved_cell().lock() {
        *guard = Some(Resolved {
            path: path.clone(),
            custom,
        });
    }
    path
}

fn custom_history_pointer() -> PathBuf {
    crate::paths::config_dir().join("custom_history_path.txt")
}

/// 用户在设置里点「更改」时随时改历史文件路径（进程内立即生效）。
pub fn set_custom_history_path(new_path: Option<PathBuf>) -> Result<(), String> {
    let pointer = custom_history_pointer();
    let (resolved, custom) = match new_path {
        Some(p) => {
            let _ = crate::paths::ensure_dir(&crate::paths::config_dir());
            std::fs::write(&pointer, p.to_string_lossy().as_bytes())
                .map_err(|e| format!("写入历史文件重定向记录失败：{e}"))?;
            (p, true)
        }
        None => {
            let _ = std::fs::remove_file(&pointer);
            (resolve_history_path_default(), false)
        }
    };
    set_resolved_history_path(resolved, custom)
}

/// 当前实际生效的历史文件路径 + 它是不是用户自定义的。
///
/// 两件事一起缓存：`history_path_is_custom` 会被设置页每帧调用一次，
/// 单独去读指针文件就是每帧一次磁盘 IO（而且在 UI 线程上）。
#[derive(Clone)]
struct Resolved {
    path: PathBuf,
    custom: bool,
}

/// 用全局静态变量保存当前实际路径，支持运行时热更新。
fn resolved_cell() -> &'static std::sync::Mutex<Option<Resolved>> {
    static CELL: std::sync::OnceLock<std::sync::Mutex<Option<Resolved>>> =
        std::sync::OnceLock::new();
    CELL.get_or_init(|| std::sync::Mutex::new(None))
}

fn set_resolved_history_path(p: PathBuf, custom: bool) -> Result<(), String> {
    let mut guard = resolved_cell().lock().map_err(|_| "历史路径锁中毒".to_string())?;
    *guard = Some(Resolved { path: p, custom });
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
    // 先确保缓存已填好（`history_path` 会顺手记下 custom 标志）
    let _ = history_path();
    match resolved_cell().lock() {
        Ok(guard) => guard.as_ref().map(|r| r.custom).unwrap_or(false),
        Err(_) => false,
    }
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
        // 直接按迭代器取前 4 个字段，不 collect —— 原来每行都分配一个
        // `Vec<&str>`，2000 行就是 2000 次堆分配，而这条路径每个刷新周期都跑。
        // 多出来的字段照旧忽略（原实现只检查「至少 4 段」）。
        let mut it = s.split('\t');
        let (Some(f0), Some(f1), Some(f2), Some(f3)) =
            (it.next(), it.next(), it.next(), it.next())
        else {
            continue;
        };
        let (Ok(ts), Ok(level), Ok(chg), Ok(mode)) = (
            f0.parse::<i64>(),
            f1.parse::<i32>(),
            f2.parse::<i32>(),
            f3.parse::<i32>(),
        ) else {
            continue;
        };
        if !(0..=100).contains(&level) {
            continue;
        }
        // 时间戳也要挑：文件是可被外部编辑的纯文本，一行 `i64::MAX` 就能让
        // 下游的 `prev_t - r.ts` / `ts - first_ts` 溢出（debug 直接 panic，
        // release 绕回负数，估算和曲线全变垃圾）。
        if !(TS_MIN_PLAUSIBLE..=TS_MAX_PLAUSIBLE).contains(&ts) {
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
/// （`prev_t - r.ts > HIST_GAP_BREAK_SEC` 断档检测、`direction_start` 逐条回看），
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

    // 只读最后一条：去重只看「上一条」，为了它整份读盘 + 解析 + 排序是每 60 秒一次的
    // 固定浪费（2000 行 ≈ 2000 次 split + collect + O(n log n) 排序）。
    let last = read_last_row();
    let tail: &[HistoryRow] = match last.as_ref() {
        Some(r) => std::slice::from_ref(r),
        None => &[],
    };
    if !should_record(tail, level, charging, now_sec) {
        return Ok(false);
    }

    let new_row = HistoryRow::new(now_sec, level, charging, mode);
    let line = new_row.to_line();
    append_line(&path, &line)?;

    // 先追加再裁剪：万一裁剪失败，至少样本已经落盘了。
    //
    // 裁剪要拿**含刚追加那条**的列表去算。以前这里传的是 `&rows`（追加前的
    // 快照），于是每次 rewrite 都会把刚写进去的那条覆盖掉；如果 `rows` 已经全部
    // 超出 48 小时窗口，裁剪结果就是空表 —— 文件被改写成 0 字节，样本永远攒不起来，
    // 界面上就永远显示「样本不足」。现在直接读追加后的文件，天然包含新样本。
    if prune_due(&path) {
        let all = read_rows();
        let outcome = prune_rows(&all, now_sec);
        if outcome.rewrite {
            write_all(&path, &outcome.rows)?;
        }
    }
    Ok(true)
}

/// 只读文件末尾，取最后一条样本。
///
/// 写采样时唯一需要的历史信息就是「上一条是什么」，为此把整份文件读盘 + 解析 +
/// 排序（2000 行）是纯浪费。这里从文件尾往前找最后一行，最多回退 8KB。
fn read_last_row() -> Option<HistoryRow> {
    read_last_row_at(&history_path())
}

pub(super) fn read_last_row_at(path: &std::path::Path) -> Option<HistoryRow> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    if len == 0 {
        return None;
    }
    let mut back = 256u64;
    loop {
        let start = len.saturating_sub(back);
        f.seek(SeekFrom::Start(start)).ok()?;
        let mut buf = Vec::with_capacity((len - start) as usize);
        f.read_to_end(&mut buf).ok()?;
        let text = String::from_utf8_lossy(&buf);
        // 末尾若没有换行，说明是刚写完的残行，丢掉它再取上一行
        let mut lines: Vec<&str> = text.split('\n').filter(|l| !l.trim().is_empty()).collect();
        if !text.ends_with('\n') {
            lines.pop();
        }
        if let Some(line) = lines.last() {
            let mut rows = parse_rows(line);
            if let Some(r) = rows.pop() {
                return Some(r);
            }
        }
        if start == 0 || back >= 8192 {
            return None;
        }
        back *= 2;
    }
}

/// 是否该做一次整份裁剪。
///
/// 裁剪需要全量行（48 小时窗口按行判断），而写采样只用到「最后一行」。
/// 每写一条就读整份文件是纯浪费，所以按文件体积 + 写入计数节流：
/// 体积接近行数上限，或累计写了 `PRUNE_EVERY_WRITES` 条，才真正读全量。
fn prune_due(path: &std::path::Path) -> bool {
    let writes = WRITES_SINCE_PRUNE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let too_big = std::fs::metadata(path)
        .map(|m| m.len() as usize >= HIST_MAX_LINES * LINE_MAX_BYTES)
        .unwrap_or(false);
    if too_big || writes >= PRUNE_EVERY_WRITES {
        WRITES_SINCE_PRUNE.store(0, std::sync::atomic::Ordering::Relaxed);
        return true;
    }
    false
}

/// 追加一条之后的裁剪：把新样本并进列表再算，避免「刚写的被裁剪覆盖掉」。
#[allow(dead_code)]
pub(super) fn prune_after_append(rows: &[HistoryRow], new_row: HistoryRow, now_sec: i64) -> PruneOutcome {
    let mut all = Vec::with_capacity(rows.len() + 1);
    all.extend_from_slice(rows);
    all.push(new_row);
    prune_rows(&all, now_sec)
}

/// 写一条采样，用真实当前时间。
pub fn record_sample(level: u8, charging: bool, mode: i32) -> Result<bool, String> {
    record_sample_at(level, charging, mode, crate::snapshot::now_unix() as i64)
}

pub(super) fn append_line(path: &std::path::Path, line: &str) -> Result<(), String> {
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
