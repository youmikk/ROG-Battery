use super::client::Device;
use std::fmt::Write as _;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const MAX_LOG_BYTES: u64 = 64 * 1024;
const MAX_EVENTS: usize = 20;
const SOFTWARE_NAME: &str = "X-Battery software";

pub fn device() -> Device {
    Device {
        name: SOFTWARE_NAME.into(),
        family: "other".into(),
        connection: "unknown".into(),
        brand: None,
        model: None,
    }
}

pub fn is_device(device: &Device) -> bool {
    device.name == SOFTWARE_NAME && device.family == "other"
}

#[derive(Default)]
struct Event {
    operation: Option<&'static str>,
    failed: bool,
    elapsed: Option<u64>,
    codes: std::collections::BTreeSet<u32>,
}

fn operation(value: &str) -> &'static str {
    match value.trim() {
        "settings_save_failed" => "settings_save_failed",
        "settings_shutdown_failed" => "settings_shutdown_failed",
        "device_order_save_failed" => "device_order_save_failed",
        "feedback_storage_failed" => "feedback_storage_failed",
        "history_update_failed" => "history_update_failed",
        "history_shutdown_failed" => "history_shutdown_failed",
        "link_enumeration_failed" => "link_enumeration_failed",
        "device_enumeration_failed" => "device_enumeration_failed",
        "store_update_prepare_failed" => "store_update_prepare_failed",
        "store_update_save_failed" => "store_update_save_failed",
        "读取" | "读取设备" => "device_read",
        "恢复" => "restore",
        _ => "other",
    }
}

fn error_codes(value: &str) -> impl Iterator<Item = u32> + '_ {
    ["os error ", "os_error="].into_iter().flat_map(move |marker| {
        value.split(marker).skip(1).filter_map(|remaining| {
            let remaining = remaining.strip_prefix("Some(").unwrap_or(remaining);
            let count = remaining.bytes().take_while(u8::is_ascii_digit).count();
            (count > 0 && count <= 10).then(|| remaining[..count].parse().ok()).flatten()
        })
    })
}

fn finish_event(event: &mut Event, output: &mut std::collections::VecDeque<String>) {
    let Some(operation) = event.operation else { return; };
    let mut line = format!("operation={operation}; result={}",
        if event.failed || operation.ends_with("_failed") { "failed" } else { "recorded" });
    if let Some(elapsed) = event.elapsed {
        let _ = write!(line, "; elapsed_ms={elapsed}");
    }
    for code in event.codes.iter().take(8) {
        let _ = write!(line, "; os_error={code}");
    }
    output.push_back(line);
    if output.len() > MAX_EVENTS {
        output.pop_front();
    }
    *event = Event::default();
}

fn summarize(log: &str) -> Vec<String> {
    let mut output = std::collections::VecDeque::new();
    let mut event = Event::default();
    for line in log.lines() {
        if line.starts_with("--- ") {
            finish_event(&mut event, &mut output);
            event = Event::default();
        } else if line == "X-Battery 崩溃信息" {
            event.operation = Some("panic");
            event.failed = true;
        } else if let Some(value) = line.strip_prefix("操作: ") {
            event.operation = Some(operation(value));
        } else if let Some(value) = line.strip_prefix("本轮耗时: ") {
            event.elapsed = value.strip_suffix(" ms").and_then(|value| value.parse().ok());
        } else if let Some(value) = line.strip_prefix("消息: ").or_else(|| line.strip_prefix("状态: ")) {
            event.codes.extend(error_codes(value).take(8));
            let lower = value.to_ascii_lowercase();
            event.failed |= !event.codes.is_empty()
                || ["failed", "error", "失败", "异常", "拒绝"].iter().any(|marker| lower.contains(marker));
        }
    }
    finish_event(&mut event, &mut output);
    output.into_iter().collect()
}

fn log_summary(path: &Path) -> (String, Vec<String>) {
    let read = (|| -> std::io::Result<Vec<u8>> {
        let mut file = std::fs::File::open(path)?;
        let length = file.metadata()?.len();
        file.seek(SeekFrom::Start(length.saturating_sub(MAX_LOG_BYTES)))?;
        let mut bytes = Vec::new();
        file.take(MAX_LOG_BYTES).read_to_end(&mut bytes)?;
        Ok(bytes)
    })();
    match read {
        Ok(bytes) => ("available; safe summary only".into(), summarize(&String::from_utf8_lossy(&bytes))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ("not recorded yet".into(), Vec::new()),
        Err(error) => (format!("unavailable; os_error={}", error.raw_os_error().unwrap_or(0)), Vec::new()),
    }
}

pub fn collect() -> String {
    let (status, events) = log_summary(&crate::diagnostics::log_path());
    let distribution = if crate::autostart::is_packaged() { "Microsoft Store / MSIX" } else { "Portable" };
    let mut report = format!("App: X-Battery {}\nSystem: {}\nDistribution: {distribution}\nLog: {status}\nSoftware log summary\n",
        crate::gui::APP_VERSION, super::system_version());
    for event in events {
        let _ = writeln!(report, "{event}");
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn software_reports_use_the_existing_feedback_schema_without_a_device() {
        let draft = super::super::client::Draft {
            category: "other".into(), device: device(),
            description: "Settings could not be saved.".into(), diagnostics: Some(collect()), images: Vec::new(),
        };
        assert!(is_device(&draft.device));
        assert!(super::super::client::validate_details(&draft).is_ok());
    }

    #[test]
    fn log_summaries_keep_error_codes_but_never_copy_messages_or_private_fields() {
        let log = "--- 1 ---\nX-Battery 诊断信息\n操作: settings_save_failed\n消息: 替换 C:\\Users\\Alice\\settings.ini 失败 (os error 17); token=private-token\n设备型号: private-alias\n接收器序列号: private-serial\n回溯:\nprivate-account\n--- 2 ---\n操作: feedback_storage_failed\n消息: step=Replace local history; os_error=32\n";
        let summary = summarize(log).join("\n");
        assert!(summary.contains("settings_save_failed"));
        assert!(summary.contains("os_error=17"));
        assert!(summary.contains("os_error=32"));
        for private in ["Alice", "Users", "settings.ini", "private-", "token=", "serial", "step="] {
            assert!(!summary.contains(private), "{private}");
        }
    }

    #[test]
    fn unknown_operations_and_panic_payloads_cannot_export_arbitrary_log_text() {
        let summary = summarize("--- 1 ---\n操作: private-username\n消息: private-password\n--- 2 ---\nX-Battery 崩溃信息\n位置: D:\\private\\source.rs\n消息: private-payload (os error 5)\n").join("\n");
        assert!(summary.contains("operation=other"));
        assert!(summary.contains("operation=panic; result=failed; os_error=5"));
        assert!(!summary.contains("private"));
    }

    #[test]
    fn only_the_latest_twenty_log_events_are_attached() {
        let log: String = (0..128).map(|index| format!("--- {index} ---\n操作: settings_save_failed\n消息: os_error={index}\n")).collect();
        let summary = summarize(&log);
        assert_eq!(summary.len(), MAX_EVENTS);
        assert!(summary.first().unwrap().contains("os_error=108"));
        assert!(summary.last().unwrap().contains("os_error=127"));
        assert!(summary.join("\n").len() < 20 * 1024);
    }

    #[test]
    fn storage_events_preserve_codes_in_current_and_legacy_formats() {
        let event = super::super::client::storage_event("Replace local history", &std::io::Error::from_raw_os_error(32));
        let log = format!("--- 1 ---\n操作: feedback_storage_failed\n消息: {event}\n--- 2 ---\n操作: device_order_save_failed\n消息: os_error=Some(17)\n");
        let summary = summarize(&log).join("\n");
        assert!(summary.contains("operation=feedback_storage_failed; result=failed; os_error=32"));
        assert!(summary.contains("operation=device_order_save_failed; result=failed; os_error=17"));
        assert!(!summary.contains("Replace local history"));
    }

    #[test]
    fn a_missing_log_does_not_prevent_describing_or_submitting_a_software_problem() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("scratch").join(format!("missing-software-log-{}", std::process::id())).join("diagnostics.log");
        let (status, events) = log_summary(&path);
        assert_eq!(status, "not recorded yet");
        assert!(events.is_empty());
    }
}
