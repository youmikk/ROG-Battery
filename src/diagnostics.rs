use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

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
    crate::paths::settings_file()
        .parent()
        .map(|dir| dir.join("diagnostics.log"))
        .unwrap_or_else(|| crate::paths::config_dir().join("diagnostics.log"))
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
    let _ = writeln!(out, "R Battery 诊断信息");
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
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(dir);
        }
    }
    use std::io::Write as _;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("打开日志失败：{e}"))?;
    file.write_all(format!("--- {stamp} ---\n").as_bytes())
        .and_then(|_| file.write_all(text.trim_end().as_bytes()))
        .and_then(|_| file.write_all(b"\n"))
        .map_err(|e| format!("写日志失败：{e}"))
}

/// 在设备或 UI 状态变化时追加一条不依赖设备快照的事件。
pub fn event(operation: &str, message: &str) {
    let _ = append_text(
        &log_path(),
        &format!(
            "R Battery 诊断信息\n版本: {}\n操作: {operation}\n消息: {message}",
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
            "R Battery 崩溃信息\n版本: {version}\n位置: {location}\n消息: {payload}\n线程: {}\n回溯:\n{}",
            std::thread::current().name().unwrap_or("未命名"),
            std::backtrace::Backtrace::force_capture()
        );
        let _ = append_text(&log_path(), &text);
        default_hook(info);
    }));
}
