use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub mod readings;

const LOG_BYTES: u64 = 512 * 1024;
pub(crate) const LOG_BACKUPS: usize = 2;
const RECORD_BYTES: usize = 16 * 1024;
const MEMORY_BYTES: usize = 64 * 1024;
const MEMORY_RECORDS: usize = 64;

#[derive(Clone, Default)]
pub(crate) struct LogFallback {
    pub records: VecDeque<MemoryRecord>,
    pub failures: u64,
    pub records_dropped: u64,
    pub last_os_error: Option<i32>,
}

#[derive(Clone)]
pub(crate) struct MemoryRecord {
    pub text: String,
    pub persisted: bool,
}

#[derive(Default)]
struct LogWriter {
    fallback: LogFallback,
    memory_bytes: usize,
}

impl LogWriter {
    fn append_with(&mut self, text: &str, stamp: u64,
        write: impl FnOnce(&str) -> std::io::Result<()>) -> Result<(), String> {
        let mut record = format!("--- {stamp} ---\n");
        let text = text.trim_end();
        let limit = RECORD_BYTES - record.len() - 32;
        let mut end = text.len().min(limit);
        while !text.is_char_boundary(end) { end -= 1; }
        record.push_str(&text[..end]);
        if end < text.len() { record.push_str("[record truncated]"); }
        record.push('\n');
        let result = write(&record);
        self.memory_bytes += record.len();
        self.fallback.records.push_back(MemoryRecord { text: record, persisted: result.is_ok() });
        while self.memory_bytes > MEMORY_BYTES || self.fallback.records.len() > MEMORY_RECORDS {
            if let Some(record) = self.fallback.records.pop_front() {
                self.memory_bytes -= record.text.len();
                self.fallback.records_dropped = self.fallback.records_dropped.saturating_add(1);
            }
        }
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                self.fallback.failures = self.fallback.failures.saturating_add(1);
                self.fallback.last_os_error = error.raw_os_error();
                Err(format!("诊断日志写入失败，已保留有界内存副本；os_error={}",
                    error.raw_os_error().map(|code| code.to_string()).unwrap_or_else(|| "missing".into())))
            }
        }
    }
}

fn writer() -> &'static Mutex<LogWriter> {
    static WRITER: OnceLock<Mutex<LogWriter>> = OnceLock::new();
    WRITER.get_or_init(Default::default)
}

pub(crate) fn log_fallback() -> LogFallback {
    writer().lock().unwrap_or_else(|error| error.into_inner()).fallback.clone()
}

pub fn unix_millis() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis()).unwrap_or(0)
}

pub fn anonymous_id(value: &str) -> u64 {
    use std::hash::BuildHasher;
    static HASHER: OnceLock<std::collections::hash_map::RandomState> = OnceLock::new();
    HASHER.get_or_init(Default::default).hash_one(value)
}

#[derive(Clone, Debug, Default)]
pub struct DeviceInfo {
    pub model: String,
    pub key: String,
    pub link_mode: String,
    pub wired_pid: u16,
    pub wireless_pid: u16,
    pub receiver_name: String,
    pub receiver_vendor_id: u16,
    pub receiver_product_id: u16,
    pub receiver_serial: String,
    pub receiver_mac: String,
    pub receiver_firmware: String,
}

/// 一次诊断事件的上下文。设备字段可以为空。
pub struct Facts<'a> {
    pub version: &'a str,
    pub operation: &'a str,
    pub status: &'a str,
    pub family: &'a str,
    pub frames: usize,
    pub elapsed_ms: u128,
    pub history_rows: usize,
    pub device: Option<&'a DeviceInfo>,
}

pub fn log_path() -> PathBuf {
    crate::paths::cache_dir().join("diagnostics").join("diagnostics.log")
}

pub(crate) fn rotated_path(path: &Path, generation: usize) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{generation}"));
    PathBuf::from(name)
}

pub(crate) fn legacy_log_paths() -> Vec<PathBuf> {
    let mut paths = vec![crate::paths::config_dir().join("diagnostics.log")];
    if let Some(parent) = crate::paths::settings_file().parent() {
        let custom = parent.join("diagnostics.log");
        if !paths.contains(&custom) { paths.push(custom); }
    }
    paths
}

fn masked(value: &str, keep: usize) -> String {
    let value = value.trim();
    if value.is_empty() {
        return "-".to_string();
    }
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= keep {
        return "*".repeat(chars.len());
    }
    format!(
        "{}{}",
        "*".repeat((chars.len() - keep).min(8)),
        chars[chars.len() - keep..].iter().collect::<String>()
    )
}

fn hex_pid(pid: u16) -> String {
    if pid == 0 {
        "未知".to_string()
    } else {
        format!("0x{pid:04X}")
    }
}

/// 渲染成纯文本。字段顺序固定，方便比对两次导出的差异。
pub fn render(f: &Facts<'_>) -> String {
    let mut out = String::with_capacity(768);
    let _ = writeln!(out, "X-Battery 诊断信息");
    let _ = writeln!(out, "版本: {}", f.version);
    let _ = writeln!(out, "操作: {}", f.operation);
    let _ = writeln!(out, "状态: {}", f.status);
    let _ = writeln!(out, "设备族: {}", f.family);
    let _ = writeln!(out, "本轮帧数: {}", f.frames);
    let _ = writeln!(out, "本轮耗时: {} ms", f.elapsed_ms);
    let _ = writeln!(out, "历史样本: {} 条", f.history_rows);
    if let Some(device) = f.device {
        let _ = writeln!(out, "设备型号: {}", value_or_unknown(&device.model));
        let _ = writeln!(out, "设备标识: {}", value_or_unknown(&device.key));
        let _ = writeln!(out, "链路: {}", value_or_unknown(&device.link_mode));
        let _ = writeln!(out, "有线 PID: {}", hex_pid(device.wired_pid));
        let _ = writeln!(out, "无线 PID: {}", hex_pid(device.wireless_pid));
        let _ = writeln!(
            out,
            "接收器名称: {}",
            value_or_unknown(&device.receiver_name)
        );
        if device.receiver_vendor_id != 0 || device.receiver_product_id != 0 {
            let _ = writeln!(
                out,
                "接收器 VID/PID: {:04X}:{:04X}",
                device.receiver_vendor_id, device.receiver_product_id
            );
        }
        let _ = writeln!(out, "接收器序列号: {}", masked(&device.receiver_serial, 4));
        let _ = writeln!(out, "接收器 MAC: {}", masked(&device.receiver_mac, 5));
        let _ = writeln!(
            out,
            "接收器固件: {}",
            value_or_unknown(&device.receiver_firmware)
        );
    }
    out
}

fn value_or_unknown(value: &str) -> &str {
    let value = value.trim();
    if value.is_empty() {
        "未知"
    } else {
        value
    }
}

/// 追加一段到日志文件。父目录不存在就建。
pub fn append(path: &Path, f: &Facts<'_>) -> Result<(), String> {
    append_text(path, &render(f))
}

pub fn append_text(path: &Path, text: &str) -> Result<(), String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs()).unwrap_or(0);
    writer().lock().unwrap_or_else(|error| error.into_inner())
        .append_with(text, stamp, |record| write_record(path, record))
}

fn write_record(path: &Path, record: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let length = match std::fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error),
    };
    if length.saturating_add(record.len() as u64 + 1) > LOG_BYTES {
        for generation in (1..=LOG_BACKUPS).rev() {
            let destination = rotated_path(path, generation);
            match std::fs::remove_file(&destination) {
                Ok(()) => {},
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                Err(error) => return Err(error),
            }
            let source = if generation == 1 { path.to_path_buf() } else { rotated_path(path, generation - 1) };
            match std::fs::rename(source, destination) {
                Ok(()) => {},
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                Err(error) => return Err(error),
            }
        }
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    if file.metadata()?.len() > 0 { file.write_all(b"\n")?; }
    file.write_all(record.as_bytes())
}

/// 在设备或 UI 状态变化时追加一条不依赖设备快照的事件。
pub fn event(operation: &str, message: &str) {
    let _ = append_text(
        &log_path(),
        &format!(
            "X-Battery 诊断信息\n版本: {}\n操作: {operation}\n消息: {message}",
            crate::gui::APP_VERSION
        ),
    );
}

/// 同一个事件只在本次运行中写一次。
pub fn event_once(operation: &str, message: &str) {
    static SEEN: OnceLock<Mutex<std::collections::BTreeSet<String>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(std::collections::BTreeSet::new()));
    let Ok(mut seen) = seen.lock() else {
        return;
    };
    if seen.insert(format!("{operation}|{message}")) {
        drop(seen);
        event(operation, message);
    }
}

/// 每种设备只记录一次，避免后台刷新把日志写满。
pub fn record_device_once(f: &Facts<'_>) {
    let Some(device) = f.device else {
        return;
    };
    let fingerprint = format!(
        "{}|{}|{}|{:04X}:{:04X}|{}|{}",
        device.key,
        device.model,
        device.link_mode,
        device.receiver_vendor_id,
        device.receiver_product_id,
        device.receiver_serial,
        device.receiver_mac
    );
    static SEEN: OnceLock<Mutex<std::collections::BTreeSet<String>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(std::collections::BTreeSet::new()));
    let Ok(mut seen) = seen.lock() else {
        return;
    };
    if seen.insert(fingerprint) {
        let _ = append(&log_path(), f);
    }
}

pub fn install_panic_hook(version: &'static str) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|v| format!("{}:{}:{}", v.file(), v.line(), v.column()))
            .unwrap_or_else(|| "未知位置".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("未知 panic");
        let text = format!(
            "X-Battery 崩溃信息\n版本: {version}\n位置: {location}\n消息: {payload}\n线程: {}\n回溯:\n{}",
            std::thread::current().name().unwrap_or("未命名"),
            std::backtrace::Backtrace::force_capture()
        );
        let _ = append_text(&log_path(), &text);
        default_hook(info);
    }));
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) struct Scratch(pub PathBuf);

    impl Scratch {
        pub(crate) fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scratch").join("diagnostics-tests");
            std::fs::create_dir_all(&root).unwrap();
            let path = root.join(format!("{}-{}-{}", std::process::id(), unix_millis(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scratch").join("diagnostics-tests");
            if let (Ok(path), Ok(root)) = (self.0.canonicalize(), root.canonicalize()) {
                if path.starts_with(&root) && path != root { let _ = std::fs::remove_dir_all(path); }
            }
        }
    }

    fn message(number: usize) -> String {
        format!("X-Battery 诊断信息\n版本: 1.4.11\n操作: device_read_failed\n消息: os_error={number}")
    }

    #[test]
    fn log_destination_is_the_cache_diagnostics_directory() {
        assert_eq!(log_path(), crate::paths::cache_dir().join("diagnostics").join("diagnostics.log"));
    }

    #[test]
    fn rotations_bound_disk_usage_and_keep_whole_recent_records() {
        let scratch = Scratch::new();
        let path = scratch.0.join("diagnostics.log");
        let mut writer = LogWriter::default();
        for number in 0..250 {
            let text = message(number) + &"界".repeat(RECORD_BYTES);
            writer.append_with(&text, number as u64, |record| write_record(&path, record)).unwrap();
        }
        let paths = [path.clone(), rotated_path(&path, 1), rotated_path(&path, 2)];
        for path in paths {
            let log = std::fs::read_to_string(path).unwrap();
            assert!(log.len() as u64 <= LOG_BYTES);
            assert!(log.starts_with("--- "));
            assert!(log.ends_with("[record truncated]\n"));
        }
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), LOG_BACKUPS + 1);
        let current = std::fs::read_to_string(path).unwrap();
        assert!(current.contains("os_error=249"));
        assert!(!current.contains("os_error=0界"));
        assert!(writer.memory_bytes <= MEMORY_BYTES);
    }

    #[test]
    fn a_new_log_starts_with_its_record_header() {
        let scratch = Scratch::new();
        let path = scratch.0.join("diagnostics.log");
        let mut writer = LogWriter::default();
        writer.append_with(&message(5), 1, |record| write_record(&path, record)).unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), writer.fallback.records[0].text);
    }

    #[test]
    fn unavailable_directory_keeps_records_in_memory_and_recovers() {
        let scratch = Scratch::new();
        let parent = scratch.0.join("blocked");
        std::fs::write(&parent, b"file instead of directory").unwrap();
        let path = parent.join("diagnostics.log");
        let mut writer = LogWriter::default();
        let error = writer.append_with(&message(5), 1, |record| write_record(&path, record)).unwrap_err();
        assert!(!error.contains(&scratch.0.to_string_lossy().to_string()));
        assert_eq!(writer.fallback.failures, 1);
        assert!(!writer.fallback.records[0].persisted);
        assert!(writer.fallback.records[0].text.contains("os_error=5"));
        std::fs::remove_file(parent).unwrap();
        writer.append_with(&message(6), 2, |record| write_record(&path, record)).unwrap();
        assert!(writer.fallback.records[1].persisted);
        assert_eq!(writer.fallback.failures, 1);
        assert!(std::fs::read_to_string(path).unwrap().contains("os_error=6"));
    }

    #[test]
    fn failed_rotation_does_not_append_past_the_disk_cap() {
        let scratch = Scratch::new();
        let path = scratch.0.join("diagnostics.log");
        std::fs::write(&path, vec![b'x'; LOG_BYTES as usize]).unwrap();
        let blocker = rotated_path(&path, LOG_BACKUPS);
        std::fs::create_dir(&blocker).unwrap();
        let mut writer = LogWriter::default();
        assert!(writer.append_with(&message(5), 1, |record| write_record(&path, record)).is_err());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), LOG_BYTES);
        assert_eq!(writer.fallback.records.len(), 1);
        std::fs::remove_dir(blocker).unwrap();
        writer.append_with(&message(6), 2, |record| write_record(&path, record)).unwrap();
        assert_eq!(std::fs::metadata(rotated_path(&path, 1)).unwrap().len(), LOG_BYTES);
        assert!(std::fs::read_to_string(path).unwrap().contains("os_error=6"));
    }

    #[test]
    fn partial_write_failure_keeps_a_complete_copy_and_next_record_boundary() {
        let scratch = Scratch::new();
        let path = scratch.0.join("diagnostics.log");
        let mut writer = LogWriter::default();
        assert!(writer.append_with(&message(5), 1, |record| {
            std::fs::write(&path, &record.as_bytes()[..12])?;
            Err(std::io::Error::from_raw_os_error(112))
        }).is_err());
        writer.append_with(&message(6), 2, |record| write_record(&path, record)).unwrap();
        assert_eq!(writer.fallback.last_os_error, Some(112));
        assert!(writer.fallback.records[0].text.ends_with("os_error=5\n"));
        assert!(std::fs::read_to_string(path).unwrap().contains("\n--- 2 ---\n"));
    }

    #[test]
    fn repeated_write_failures_have_bounded_record_count_and_bytes() {
        let mut writer = LogWriter::default();
        for number in 0..1000 {
            assert!(writer.append_with(&message(number), 1,
                |_| Err(std::io::Error::from_raw_os_error(5))).is_err());
        }
        assert_eq!(writer.fallback.records.len(), MEMORY_RECORDS);
        assert_eq!(writer.fallback.failures, 1000);
        assert_eq!(writer.fallback.records_dropped, 1000 - MEMORY_RECORDS as u64);
        for number in 0..100 {
            assert!(writer.append_with(&(message(number) + &"界".repeat(RECORD_BYTES)), 1,
                |_| Err(std::io::Error::from_raw_os_error(5))).is_err());
        }
        assert!(writer.memory_bytes <= MEMORY_BYTES);
        assert_eq!(writer.memory_bytes, writer.fallback.records.iter().map(|record| record.text.len()).sum::<usize>());
        assert!(writer.fallback.records.iter().all(|record| record.text.len() <= RECORD_BYTES));
    }
}
