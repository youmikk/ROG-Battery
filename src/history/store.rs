//! 历史采样的存储层：路径、跨进程互斥、原子写入与旧格式兼容。
//! 多设备记录和旧四列记录共用history.tsv及既有命名互斥体。
//!
//! 路径故意叫 `rog-battery` 而不是 `rog-mouse-ctl`：要复用 C# 版攒下的历史数据。
//!
//! 上层：[`super::predict`]（纯计算）、[`super::format`]（纯文案）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::win32;
use super::devices::{self, DeviceHistoryFile};

// ==================== 常量（逐字照抄 C# 版） ====================

/// 保留最近 48 小时（覆盖两晚）。
pub const HIST_KEEP_HOURS: i64 = 48;
/// 时间戳的合理下界（2020-01-01Z）。历史文件是纯文本、可被任何工具改，
/// 越界的行在下游减法里会溢出，读入时就该扔掉。
pub const TS_MIN_PLAUSIBLE: i64 = 1_577_836_800;
/// 时间戳的合理上界（2100-01-01Z）。
pub const TS_MAX_PLAUSIBLE: i64 = 4_102_444_800;
/// 每台设备的行数上限；未归属旧记录也独立使用这个上限。
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

static CACHED_ROW_COUNT: AtomicUsize = AtomicUsize::new(0);

/// 最近一次成功读取或写入后的有效历史条数，包括未归属的旧记录。
/// 诊断输出只取这个内存值，不在UI线程访问文件或等待历史互斥体。
pub fn cached_row_count() -> usize {
    CACHED_ROW_COUNT.load(Ordering::Relaxed)
}

fn remember_row_count(history: &DeviceHistoryFile) {
    let count = history.devices.values().fold(history.legacy.len(), |total, rows| total.saturating_add(rows.len()));
    CACHED_ROW_COUNT.store(count, Ordering::Relaxed);
}

// ==================== 数据行 ====================

/// 一条历史采样。旧格式没有设备键和flags，新增采样由v2格式保存。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryRow {
    pub ts: i64,
    pub level: u8,
    /// 设备报告的**原始** charging_status，不是归并后的三态。
    pub charging: bool,
    /// 连接方式（1=有线 2=2.4G 3=蓝牙），判定「已充满」要用它。
    pub mode: i32,
    /// 该百分比由设备电量档位近似换算。
    pub approximate: bool,
    /// false表示设备没有提供可靠的充电状态。
    pub charging_known: bool,
    /// 当前采样之前存在断档，不应跨过此点连线或推算。
    pub break_before: bool,
}

impl HistoryRow {
    pub fn new(ts: i64, level: u8, charging: bool, mode: i32) -> Self {
        Self {
            ts,
            level,
            charging,
            mode,
            approximate: false,
            charging_known: true,
            break_before: false,
        }
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

/// 本次进程实际使用的历史文件路径；设置中更改位置后立即更新。
///
/// 为什么要探测：`%LOCALAPPDATA%` 在某些受限环境里对普通进程只读
/// （写入一律 `os error 5 拒绝访问`，连在同一目录新建文件都不行）。
/// 那时如果还死守着它，采样一条也写不进去，续航估算就永远停在
/// 「样本不足」—— 用户看到的是功能坏了，其实是路径不可写。
///
/// 顺序：`%LOCALAPPDATA%\rog-battery\` → 程序所在目录 → 缓存目录。
/// 只有前一个**真的写不进去**才往后掉，正常机器上永远是第一个。
pub fn history_path() -> PathBuf {
    let mut guard = resolved_cell().lock().unwrap_or_else(|error| error.into_inner());
    if let Some(r) = guard.as_ref() {
        return r.path.clone();
    }
    let (path, custom) = match read_custom_history_path() {
        Some(custom) => (custom, true),
        None => (resolve_history_path_default(), false),
    };
    *guard = Some(Resolved { path: path.clone(), custom });
    path
}

fn custom_history_pointer() -> PathBuf {
    crate::paths::config_dir().join("custom_history_path.txt")
}

/// 用户在设置里点「更改」时随时改历史文件路径（进程内立即生效）。
pub fn set_custom_history_path(new_path: Option<PathBuf>) -> Result<(), String> {
    let _guard = lock_history().ok_or("历史文件正在使用，暂时无法更改保存位置")?;
    let source = active_history_path_locked()?;
    let pointer = custom_history_pointer();
    let (resolved, custom) = match new_path {
        Some(p) => {
            if p.as_os_str().is_empty() || p.to_str().is_none()
                || p.to_string_lossy().trim() != p.to_string_lossy()
                || p.to_string_lossy().contains(['\n', '\r', '\0']) {
                return Err("历史文件保存路径无效".into());
            }
            (p, true)
        }
        None => (resolve_history_path_default(), false),
    };
    let now_sec = crate::snapshot::now_unix() as i64;
    // 目标可能也有历史，必须按设备合并；先完成写盘再切换指针和进程缓存。
    let migrated = migrate_history_at(&source, &resolved, now_sec)?;
    let mut cache = resolved_cell().lock().map_err(|_| "历史路径锁中毒".to_string())?;
    if custom {
        let text = resolved.to_str().ok_or("历史文件保存路径无效")?.as_bytes();
        match std::fs::read(&pointer) {
            Ok(current) if current == text => {}
            Ok(_) => atomic_write(&pointer, text)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => atomic_write(&pointer, text)?,
            Err(error) => return Err(format!("读取历史文件重定向记录失败：{error}")),
        }
    } else {
        match std::fs::remove_file(&pointer) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("清除历史文件重定向记录失败：{error}")),
        }
    }
    *cache = Some(Resolved { path: resolved, custom });
    remember_row_count(&migrated);
    Ok(())
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

fn read_custom_history_path() -> Option<PathBuf> {
    if let Ok(text) = std::fs::read_to_string(custom_history_pointer()) {
        let t = text.trim();
        if !t.is_empty() {
            return Some(PathBuf::from(t));
        }
    }
    None
}

/// 文件操作已持有历史互斥体。重新检查指针，使其它实例切换路径后，
/// 当前实例的后续采样也会采用合并完成的新文件；UI查询仍使用内存缓存。
fn active_history_path_locked() -> Result<PathBuf, String> {
    let custom = match std::fs::read_to_string(custom_history_pointer()) {
        Ok(text) => {
            let value = text.trim();
            if value.is_empty() { None } else { Some(PathBuf::from(value)) }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("读取历史文件重定向记录失败：{error}")),
    };
    let mut cache = resolved_cell().lock().map_err(|_| "历史路径锁中毒".to_string())?;
    if let Some(current) = cache.as_ref() {
        let unchanged = match &custom {
            Some(path) => current.custom && current.path == *path,
            None => !current.custom,
        };
        if unchanged { return Ok(current.path.clone()); }
    }
    let resolved = match custom {
        Some(path) => Resolved { path, custom: true },
        None => Resolved { path: resolve_history_path_default(), custom: false },
    };
    let path = resolved.path.clone();
    *cache = Some(resolved);
    Ok(path)
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
    // 这是有意的：空的 history.tsv 和不存在等价，读取后都是空表。
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

fn read_history_file(path: &Path) -> Result<DeviceHistoryFile, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(devices::parse_device_history(&text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(DeviceHistoryFile::default()),
        Err(error) => Err(format!("读取历史文件 {} 失败：{error}", path.display())),
    }
}

/// 读取指定历史文件并裁剪内存副本，不修改磁盘。
pub fn load_device_history_at(path: &Path, now_sec: i64) -> Result<DeviceHistoryFile, String> {
    let mut history = read_history_file(path)?;
    devices::normalize(&mut history, now_sec);
    Ok(history)
}

pub fn load_device_history() -> Result<DeviceHistoryFile, String> {
    let _guard = lock_history().ok_or("历史文件正在使用，暂时无法读取")?;
    let history = load_device_history_at(&active_history_path_locked()?, crate::snapshot::now_unix() as i64)?;
    remember_row_count(&history);
    Ok(history)
}

fn write_history_file(path: &Path, history: &DeviceHistoryFile) -> Result<(), String> {
    atomic_write(path, devices::render(history).as_bytes())
}

fn update_device_history_at(path: &Path, samples: &[(String, HistoryRow)],
    aliases: &[(String, String)], legacy_owner: Option<&str>, now_sec: i64)
    -> Result<DeviceHistoryFile, String> {
    devices::validate_update(samples, aliases, legacy_owner)?;
    let previous = read_history_file(path)?;
    let mut updated = previous.clone();
    devices::apply_update(&mut updated, samples, aliases, legacy_owner, now_sec);
    if updated != previous { write_history_file(path, &updated)?; }
    Ok(updated)
}

/// 当前路径与文件读改写都在既有跨进程互斥体内完成。
pub fn update_device_history(samples: &[(String, HistoryRow)], aliases: &[(String, String)],
    legacy_owner: Option<&str>) -> Result<DeviceHistoryFile, String> {
    let _guard = lock_history().ok_or("历史文件正在使用，暂时无法写入")?;
    let history = update_device_history_at(&active_history_path_locked()?, samples, aliases, legacy_owner,
        crate::snapshot::now_unix() as i64)?;
    remember_row_count(&history);
    Ok(history)
}

fn clear_device_history_at(path: &Path) -> Result<(), String> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() && meta.len() == 0 => Ok(()),
        Ok(_) => atomic_write(path, &[]),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("读取历史文件 {} 失败：{error}", path.display())),
    }
}

/// 清空所有设备和未归属旧记录，保留已选择的路径。
pub fn clear_device_history() -> Result<(), String> {
    let _guard = lock_history().ok_or("历史文件正在使用，暂时无法清空")?;
    clear_device_history_at(&active_history_path_locked()?)?;
    CACHED_ROW_COUNT.store(0, Ordering::Relaxed);
    Ok(())
}

fn migrate_history_at(source: &Path, target: &Path, now_sec: i64) -> Result<DeviceHistoryFile, String> {
    let source_history = read_history_file(source)?;
    let mut merged = if source != target {
        let target_history = read_history_file(target)?;
        let mut merged = target_history.clone();
        devices::merge_into(&mut merged, source_history, now_sec);
        if merged != target_history { write_history_file(target, &merged)?; }
        merged
    } else { source_history };
    devices::normalize(&mut merged, now_sec);
    // 即使没有样本，也在切换前验证目标可作为文件写入。不会覆盖已有内容。
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        crate::paths::ensure_dir(parent)?;
    }
    std::fs::OpenOptions::new().append(true).create(true).open(target)
        .map_err(|error| format!("无法使用历史文件 {}：{error}", target.display()))?;
    Ok(merged)
}

/// 同目录临时文件写完并刷盘后替换目标，读者只会看到替换前或替换后的完整文件。
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    crate::paths::write_atomic(path, bytes).map_err(|error| format!("保存历史记录失败：{error}"))
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

#[cfg(test)]
mod tests {
    use super::*;
    const NOW: i64 = 1_800_000_000;

    struct TestFile(PathBuf);
    impl TestFile {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            Self(std::env::temp_dir().join(format!("r-battery-history-{label}-{}-{nonce}-{}.tsv",
                std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))))
        }
    }
    impl Drop for TestFile {
        fn drop(&mut self) {
            if let Ok(metadata) = std::fs::metadata(&self.0) {
                if metadata.is_dir() { let _ = std::fs::remove_dir(&self.0); }
                else { let _ = std::fs::remove_file(&self.0); }
            }
        }
    }

    #[test]
    fn load_missing_and_clear_preserve_file_location_and_remove_all_formats() {
        let file = TestFile::new("clear");
        assert_eq!(load_device_history_at(&file.0, NOW).unwrap(), DeviceHistoryFile::default());
        let mut history = DeviceHistoryFile::default();
        history.legacy.push(HistoryRow::new(NOW - 60, 70, false, 2));
        history.devices.insert("one".into(), vec![HistoryRow::new(NOW, 40, false, 0)]);
        history.devices.insert("two".into(), vec![HistoryRow::new(NOW, 90, true, 1)]);
        write_history_file(&file.0, &history).unwrap();
        assert_eq!(load_device_history_at(&file.0, NOW).unwrap(), history);
        clear_device_history_at(&file.0).unwrap();
        assert!(file.0.is_file());
        assert_eq!(std::fs::metadata(&file.0).unwrap().len(), 0);
        assert_eq!(load_device_history_at(&file.0, NOW).unwrap(), DeviceHistoryFile::default());
        clear_device_history_at(&file.0).unwrap();
    }

    #[test]
    fn unchanged_update_does_not_rewrite_and_mixed_formats_survive_device_updates() {
        let file = TestFile::new("unchanged");
        // 保留不影响样本的注释/空行：若无需修改，文件字节和修改时间都应不变。
        let text = format!("# keep until actual change\n\n{}\t60\t0\t2\nv2\tdevice\t{}\t40\t0\t0\t7\n", NOW - 120, NOW - 120);
        std::fs::write(&file.0, &text).unwrap();
        let before = std::fs::metadata(&file.0).unwrap().modified().unwrap();
        let original = load_device_history_at(&file.0, NOW).unwrap();
        update_device_history_at(&file.0, &[], &[], None, NOW).unwrap();
        assert_eq!(std::fs::read_to_string(&file.0).unwrap(), text);
        assert_eq!(std::fs::metadata(&file.0).unwrap().modified().unwrap(), before);
        let sample = [("another-device".into(), HistoryRow::new(NOW, 59, false, 2))];
        let updated = update_device_history_at(&file.0, &sample, &[], None, NOW).unwrap();
        assert_eq!(updated.devices["device"], original.devices["device"]);
        assert_eq!(updated.legacy, original.legacy);
        assert_eq!(updated.devices["another-device"], [sample[0].1]);
        assert_eq!(load_device_history_at(&file.0, NOW).unwrap(), updated);
        let saved = std::fs::read(&file.0).unwrap();
        let throttled = [("another-device".into(), HistoryRow::new(NOW + 1, 59, false, 2))];
        assert_eq!(update_device_history_at(&file.0, &throttled, &[], None, NOW + 1).unwrap(), updated);
        assert_eq!(std::fs::read(&file.0).unwrap(), saved);
    }

    #[test]
    fn cached_diagnostic_count_includes_all_devices_and_unclaimed_rows() {
        let mut history = DeviceHistoryFile { legacy: vec![HistoryRow::new(NOW, 70, false, 2)], ..Default::default() };
        history.devices.insert("one".into(), vec![HistoryRow::new(NOW, 40, false, 0)]);
        history.devices.insert("two".into(), vec![HistoryRow::new(NOW - 120, 90, true, 1), HistoryRow::new(NOW, 91, true, 1)]);
        remember_row_count(&history);
        assert_eq!(cached_row_count(), 4);
        remember_row_count(&DeviceHistoryFile::default());
        assert_eq!(cached_row_count(), 0);
    }

    #[test]
    fn disk_alias_and_legacy_migration_are_consumed_and_flags_survive_reload() {
        let file = TestFile::new("migration");
        let mut old = HistoryRow::new(NOW - 120, 90, false, 2);
        old.approximate = true; old.charging_known = false; old.break_before = true;
        let mut initial = DeviceHistoryFile { legacy: vec![HistoryRow::new(NOW - 60, 20, false, 2)], ..Default::default() };
        initial.devices.insert("old-key".into(), vec![old]);
        initial.devices.insert("current-key".into(), vec![old]);
        write_history_file(&file.0, &initial).unwrap();
        let aliases = [("old-key".into(), "current-key".into())];
        let migrated = update_device_history_at(&file.0, &[], &aliases, Some("current-key"), NOW).unwrap();
        assert!(migrated.legacy.is_empty());
        assert_eq!(migrated.devices.len(), 1);
        assert_eq!(migrated.devices["current-key"].len(), 2);
        assert_eq!(migrated.devices["current-key"][0], old);
        assert_eq!(load_device_history_at(&file.0, NOW).unwrap(), migrated);
        let saved = std::fs::read(&file.0).unwrap();
        update_device_history_at(&file.0, &[], &aliases, Some("another-rog"), NOW).unwrap();
        assert_eq!(std::fs::read(&file.0).unwrap(), saved);
    }

    #[test]
    fn changing_locations_merges_all_devices_and_retains_source_on_error() {
        let source = TestFile::new("source");
        let target = TestFile::new("target");
        let bad_target = TestFile::new("bad-target");
        let mut shared = HistoryRow::new(NOW - 120, 80, false, 2);
        let mut from = DeviceHistoryFile { legacy: vec![HistoryRow::new(NOW - 60, 20, false, 2)], ..Default::default() };
        from.devices.insert("shared".into(), vec![shared]);
        from.devices.insert("source-only".into(), vec![HistoryRow::new(NOW, 60, false, 0)]);
        let mut to = DeviceHistoryFile { legacy: from.legacy.clone(), ..Default::default() };
        shared.break_before = true;
        to.devices.insert("shared".into(), vec![shared]);
        to.devices.insert("target-only".into(), vec![HistoryRow::new(NOW, 40, false, 0)]);
        write_history_file(&source.0, &from).unwrap();
        write_history_file(&target.0, &to).unwrap();
        let original = std::fs::read(&source.0).unwrap();
        migrate_history_at(&source.0, &target.0, NOW).unwrap();
        let merged = load_device_history_at(&target.0, NOW).unwrap();
        assert_eq!(merged.devices.len(), 3);
        assert_eq!(merged.devices["shared"], [shared]);
        assert_eq!(merged.legacy, from.legacy);
        assert_eq!(std::fs::read(&source.0).unwrap(), original);
        let once = std::fs::read(&target.0).unwrap();
        migrate_history_at(&source.0, &target.0, NOW).unwrap();
        assert_eq!(std::fs::read(&target.0).unwrap(), once);
        std::fs::create_dir(&bad_target.0).unwrap();
        assert!(migrate_history_at(&source.0, &bad_target.0, NOW).is_err());
        assert_eq!(std::fs::read(&source.0).unwrap(), original);
        assert!(bad_target.0.is_dir());
    }

    #[test]
    fn invalid_updates_and_atomic_replacement_failure_preserve_previous_bytes() {
        let file = TestFile::new("invalid");
        let text = format!("v2\tdevice\t{NOW}\t75\t0\t2\t2\n");
        std::fs::write(&file.0, &text).unwrap();
        let bad = [("bad\nkey".into(), HistoryRow::new(NOW, 50, false, 2))];
        assert!(update_device_history_at(&file.0, &bad, &[], None, NOW).is_err());
        assert_eq!(std::fs::read_to_string(&file.0).unwrap(), text);
        use std::os::windows::fs::OpenOptionsExt;
        let handle = std::fs::OpenOptions::new().read(true).share_mode(1).open(&file.0).unwrap();
        assert!(atomic_write(&file.0, b"replacement").is_err());
        assert_eq!(std::fs::read_to_string(&file.0).unwrap(), text);
        drop(handle);
        assert_eq!(load_device_history_at(&file.0, NOW).unwrap().devices["device"].len(), 1);
    }
}
