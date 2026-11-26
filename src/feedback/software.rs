use super::client::Device;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const MAX_LOG_BYTES: u64 = 64 * 1024;
const MAX_EVENTS: usize = 20;
const MAX_ERROR_CODES: usize = 8;
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum ErrorCode {
    Win32(u32),
    Hresult(u32),
}

impl ErrorCode {
    fn failed(self) -> bool {
        match self {
            Self::Win32(code) => code != 0,
            Self::Hresult(code) => code & 0x8000_0000 != 0,
        }
    }
}

struct Event<'a> {
    timestamp: Option<u64>,
    version: Option<&'a str>,
    operation: &'static str,
    failed: bool,
    elapsed: Option<u64>,
    codes: Vec<ErrorCode>,
    codes_truncated: bool,
    location: Option<String>,
}

fn operation(value: &str) -> &'static str {
    match value.trim() {
        "settings_save_failed" => "settings_save_failed",
        "settings_shutdown_failed" => "settings_shutdown_failed",
        "device_order_save_failed" => "device_order_save_failed",
        "device_rename_requested" => "device_rename_requested",
        "device_rename_saved" => "device_rename_saved",
        "device_rename_failed" => "device_rename_failed",
        "device_open_failed" => "device_open_failed",
        "device_read_failed" => "device_read_failed",
        "device_identified" => "device_identified",
        "feedback_storage_failed" => "feedback_storage_failed",
        "history_update_failed" => "history_update_failed",
        "history_shutdown_failed" => "history_shutdown_failed",
        "link_enumeration_failed" => "link_enumeration_failed",
        "device_enumeration_failed" => "device_enumeration_failed",
        "store_update_prepare_failed" => "store_update_prepare_failed",
        "store_update_save_failed" => "store_update_save_failed",
        "store_update_cache_failed" => "store_update_cache_failed",
        "store_restart_registration_failed" => "store_restart_registration_failed",
        "store_update_started" => "store_update_started",
        "store_update_installed" => "store_update_installed",
        "store_update_canceled" => "store_update_canceled",
        "store_update_failed" => "store_update_failed",
        "store_restart_failed" => "store_restart_failed",
        "store_restart_activated" => "store_restart_activated",
        "store_restart_helper_failed" => "store_restart_helper_failed",
        "low_battery_notification_failed" => "low_battery_notification_failed",
        "taskbar_widget_unavailable" => "taskbar_widget_unavailable",
        "taskbar_widget_position_failed" => "taskbar_widget_position_failed",
        "taskbar_widget_register_failed" => "taskbar_widget_register_failed",
        "taskbar_widget_create_failed" => "taskbar_widget_create_failed",
        "taskbar_widget_paint_failed" => "taskbar_widget_paint_failed",
        "读取" | "读取设备" => "device_read",
        "恢复" => "restore",
        _ => "unknown_operation",
    }
}

fn decimal(value: &str, max_digits: usize) -> Option<u64> {
    if value.is_empty() || value.len() > max_digits || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn valid_version(value: &str) -> Option<&str> {
    let mut count = 0;
    for part in value.split('.') {
        if decimal(part, 5)? > u64::from(u16::MAX) || (part.len() > 1 && part.starts_with('0')) {
            return None;
        }
        count += 1;
    }
    (3..=4).contains(&count).then_some(value)
}

fn known_source(path: &str) -> bool {
    let Some((directory, file)) = path.rsplit_once('/') else { return false; };
    let Some(stem) = file.strip_suffix(".rs") else { return false; };
    let names: &[&str] = match directory {
        "src" => &[
            "audio", "autostart", "backup", "chart", "cli", "commands", "device", "devices",
            "diagnostics", "dwm", "dynlight", "feedback", "gui", "hid", "history", "i18n",
            "linkstate", "main", "mock", "notifications", "ops", "paths", "proto", "report",
            "settings", "snapshot", "taskbar_widget", "tray_icon", "tray_prefs", "win32", "winrt",
        ],
        "src/backends" => &[
            "ajazz_ak980", "atk_holtek", "atk_nano", "aula", "aula_f108", "aula_f99",
            "aula_hfd", "aula_hfd_mechanical", "aula_v9pro", "bluetooth", "channel", "cherry",
            "compx", "dareu", "dareu_em901", "descriptor", "edifier", "eightbitdo", "enumeration",
            "eweadn", "eweadn_s9pro", "eweadn_x23", "eweadn_x87", "feature", "fifine_ht6",
            "flydigi", "g3m", "gamesir", "gearhub", "headset", "hyperx", "inzone", "keyboard20",
            "lingbao", "lofree", "logitech", "logitech_headset", "mchose_glw", "mchose_k87s",
            "mchose_k99", "mchose_mc", "mchose_modern", "mchose_mouse", "mchose_qhw",
            "mchose_r9", "mod", "moondrop", "page_command", "rapoo", "rapoo_config",
            "rapoo_legacy", "rapoo_mt560", "rapoo_office", "razer", "razer_capabilities", "rog",
            "sony", "standard", "status", "steelseries", "trace", "veekos", "vgn", "vgn_beiying",
            "vgn_dragonfly3", "vgn_keyboard", "vgn_ry", "xiberia", "xinput",
        ],
        "src/ui" => &[
            "canvas", "device_name", "dropdown", "feedback", "feedback_icons", "frame", "gfx",
            "layout", "mod", "pages", "palette", "panel_icons", "store_updates", "theme",
            "titlebar", "tray_popup", "viewdata", "widget",
        ],
        "src/ui/feedback" => &["layout", "state"],
        "src/feedback" => &["client", "inventory", "software"],
        "src/diagnostics" => &["readings"],
        "src/history" => &["devices", "format", "predict", "store", "tracker"],
        "src/store" => &["api", "mod", "restart", "state"],
        "src/autostart" => &["startup"],
        "src/cli" => &["quick"],
        "src/i18n" => &["catalog"],
        "src/taskbar_widget" => &["render"],
        _ => &[],
    };
    names.contains(&stem)
}

fn source_location(value: &str) -> Option<String> {
    if value.len() > 128 { return None; }
    let (path_line, column) = value.rsplit_once(':')?;
    let (path, line) = path_line.rsplit_once(':')?;
    let line = u32::try_from(decimal(line, 10)?).ok()?;
    let column = u32::try_from(decimal(column, 10)?).ok()?;
    let path = path.replace('\\', "/");
    (line > 0 && column > 0 && known_source(&path)).then(|| format!("{path}:{line}:{column}"))
}

fn numeric_code(value: &str, hresult: bool) -> Option<u32> {
    if let Some(hex) = value.strip_prefix("0x").or_else(|| value.strip_prefix("0X")) {
        if hex.is_empty() || hex.len() > 8 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(hex, 16).ok()
    } else if hresult && value.starts_with('-') {
        decimal(&value[1..], 10)?;
        value.parse::<i32>().ok().map(|code| code as u32)
    } else {
        u32::try_from(decimal(value, 10)?).ok()
    }
}

fn code_boundary(character: char) -> bool {
    matches!(character, ' ' | '\t' | ',' | ';' | ')' | ']' | '}' | '，' | '；' | '）' | '。')
}

fn code_after(value: &str, hresult: bool, parenthesized: bool) -> Option<ErrorCode> {
    let (value, parenthesized) = match value.strip_prefix("Some(") {
        Some(inner) if !hresult && !parenthesized => (inner, true),
        _ => (value, parenthesized),
    };
    let count = value.bytes().take_while(|byte|
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'+' | b'_')).count();
    let code = numeric_code(&value[..count], hresult)?;
    let mut remaining = &value[count..];
    if parenthesized { remaining = remaining.strip_prefix(')')?; }
    if !remaining.chars().next().is_none_or(code_boundary) { return None; }
    Some(if hresult { ErrorCode::Hresult(code) } else { ErrorCode::Win32(code) })
}

impl Event<'_> {
    fn add_code(&mut self, code: ErrorCode) {
        self.failed |= code.failed();
        if self.codes.contains(&code) { return; }
        if self.codes.len() == MAX_ERROR_CODES {
            self.codes_truncated = true;
        } else {
            self.codes.push(code);
        }
    }

    fn read_message(&mut self, value: &str) {
        for (marker, hresult, parenthesized) in [
            ("os error ", false, false), ("os_error=", false, false),
            ("GetLastError=", false, false), ("GetLastError()=", false, false),
            ("Win32 错误码 ", false, false), ("HRESULT=", true, false),
            ("HRESULT: ", true, false), ("HRESULT(", true, true), ("hresult=", true, false),
        ] {
            for (offset, _) in value.match_indices(marker) {
                if !value[..offset].chars().next_back().is_none_or(|character|
                    matches!(character, ' ' | '\t' | '(' | '[' | ';' | '（' | '：' | ':')) {
                    continue;
                }
                if let Some(code) = code_after(&value[offset + marker.len()..], hresult, parenthesized) {
                    self.add_code(code);
                }
            }
        }
        let fixed_hresult = match self.operation {
            "store_restart_registration_failed" => Some(value),
            "low_battery_notification_failed" => value.strip_prefix("显示系统通知失败：")
                .or_else(|| value.strip_prefix("RoInitialize 失败：")),
            _ => None,
        };
        if let Some(value) = fixed_hresult.filter(|value| value.len() == 10 && value.starts_with("0x")) {
            if let Some(code) = numeric_code(value, true) { self.add_code(ErrorCode::Hresult(code)); }
        }
        let lower = value.to_ascii_lowercase();
        self.failed |= ["failed", "失败", "异常", "拒绝"].iter().any(|marker| lower.contains(marker));
    }

    fn render(self) -> String {
        let result = if self.failed || self.operation.ends_with("_failed") { "failed" }
            else if self.operation.ends_with("_unavailable") { "unavailable" } else { "recorded" };
        let mut line = format!("operation={}; result={result}; time_unix_s={}; version={}",
            self.operation, self.timestamp.map(|stamp| stamp.to_string()).as_deref().unwrap_or("missing_or_invalid"),
            self.version.unwrap_or("missing_or_invalid"));
        if let Some(elapsed) = self.elapsed { let _ = write!(line, "; elapsed_ms={elapsed}"); }
        for code in self.codes {
            match code {
                ErrorCode::Win32(code) => { let _ = write!(line, "; os_error={code}"); }
                ErrorCode::Hresult(code) => { let _ = write!(line, "; hresult=0x{code:08X}"); }
            }
        }
        if self.codes_truncated { line.push_str("; error_codes_truncated=true"); }
        if self.operation == "panic" {
            let _ = write!(line, "; source={}", self.location.as_deref().unwrap_or("missing_or_unapproved"));
        }
        line
    }
}

fn parse_record(record: &str) -> Option<Event<'_>> {
    let mut lines = record.lines();
    let header = lines.next()?;
    let timestamp = header.strip_prefix("--- ").and_then(|value| value.strip_suffix(" ---"))
        .and_then(|value| decimal(value, 12)).filter(|stamp| (1..=253_402_300_799).contains(stamp));
    let panic = match lines.next()? {
        "X-Battery 崩溃信息" => true,
        "X-Battery 诊断信息" => false,
        _ => return None,
    };
    let mut event = Event {
        timestamp, version: None, operation: "panic", failed: panic,
        elapsed: None, codes: Vec::new(), codes_truncated: false, location: None,
    };
    let mut field = lines.next().unwrap_or_default();
    if let Some(value) = field.strip_prefix("版本: ") {
        event.version = valid_version(value);
        field = lines.next().unwrap_or_default();
    }
    if panic {
        event.location = field.strip_prefix("位置: ").and_then(source_location);
    } else {
        event.operation = operation(field.strip_prefix("操作: ")?);
        field = lines.next().unwrap_or_default();
        if let Some(value) = field.strip_prefix("消息: ") {
            event.read_message(value);
        } else if let Some(value) = field.strip_prefix("状态: ") {
            event.read_message(value);
            if lines.next().is_some_and(|line| line.starts_with("设备族: "))
                && lines.next().is_some_and(|line| line.starts_with("本轮帧数: ")) {
                event.elapsed = lines.next().and_then(|line| line.strip_prefix("本轮耗时: "))
                    .and_then(|value| value.strip_suffix(" ms")).and_then(|value| decimal(value, 20));
            }
        }
    }
    Some(event)
}

#[derive(Default)]
struct Summary {
    events: VecDeque<String>,
    older_events_omitted: usize,
    unparsed_records: usize,
    leading_fragment_omitted: bool,
    trailing_record_omitted: bool,
}

impl Summary {
    fn push_record(&mut self, record: &str) {
        let Some(event) = parse_record(record) else { self.unparsed_records += 1; return; };
        self.events.push_back(event.render());
        if self.events.len() > MAX_EVENTS {
            self.events.pop_front();
            self.older_events_omitted += 1;
        }
    }
}

fn summarize(log: &str) -> Summary {
    let mut summary = Summary::default();
    let mut start = None;
    let mut offset = 0;
    for line in log.split_inclusive('\n') {
        if line.starts_with("--- ") {
            if let Some(previous) = start { summary.push_record(&log[previous..offset]); }
            else { summary.leading_fragment_omitted = offset != 0; }
            start = Some(offset);
        }
        offset += line.len();
    }
    if let Some(start) = start {
        if log.ends_with('\n') { summary.push_record(&log[start..]); }
        else { summary.trailing_record_omitted = true; }
    } else {
        summary.leading_fragment_omitted = !log.is_empty();
    }
    summary
}

fn summarize_tail(bytes: &[u8], earlier_bytes_omitted: u64, short_read: bool) -> (String, Vec<String>) {
    let log = String::from_utf8_lossy(bytes);
    let summary = summarize(&log);
    let status = format!(
        "available; safe summary only; tail_bytes={}; byte_limit={MAX_LOG_BYTES}; earlier_bytes_omitted={earlier_bytes_omitted}; short_read={short_read}; invalid_utf8={}; events_kept={}; event_limit={MAX_EVENTS}; older_events_omitted={}; unparsed_records={}; leading_fragment_omitted={}; trailing_record_omitted={}",
        bytes.len(), matches!(log, std::borrow::Cow::Owned(_)), summary.events.len(),
        summary.older_events_omitted, summary.unparsed_records, summary.leading_fragment_omitted,
        summary.trailing_record_omitted);
    (status, summary.events.into_iter().collect())
}

fn log_summary(path: &Path) -> (String, Vec<String>) {
    let read = (|| -> std::io::Result<_> {
        let mut file = std::fs::File::open(path)?;
        let length = file.metadata()?.len();
        let offset = length.saturating_sub(MAX_LOG_BYTES);
        let expected = length - offset;
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        file.take(expected).read_to_end(&mut bytes)?;
        let short_read = bytes.len() as u64 != expected;
        Ok((bytes, offset, short_read))
    })();
    match read {
        Ok((bytes, offset, short_read)) => summarize_tail(&bytes, offset, short_read),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ("not recorded yet".into(), Vec::new()),
        Err(error) => (format!("unavailable; os_error={}",
            error.raw_os_error().map(|code| code.to_string()).unwrap_or_else(|| "missing".into())), Vec::new()),
    }
}

pub fn collect() -> String {
    let (status, events) = log_summary(&crate::diagnostics::log_path());
    let distribution = if crate::autostart::is_packaged() { "Microsoft Store / MSIX" } else { "Portable" };
    let mut report = format!("App: X-Battery {}\nSystem: {}\nDistribution: {distribution}\nLog: {status}\nSoftware log summary\nTimeline: append order; time_unix_s is wall-clock UTC seconds, not monotonic; version is per event.\nLimits: latest complete records within the byte/event caps; omitted counts describe only the sampled tail unless named earlier_bytes_omitted.\nMissing: missing_or_invalid means absent or rejected metadata; missing_or_unapproved source is not exported; absent codes/timings are not proof of success.\nSafety: known operations, numeric codes and approved relative Rust locations only; no raw messages, panic payloads or backtraces.\n",
        valid_version(crate::gui::APP_VERSION).unwrap_or("missing_or_invalid"), super::system_version());
    for event in events {
        let _ = writeln!(report, "{event}");
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(timestamp: u64, version: &str, operation: &str, message: &str) -> String {
        format!("--- {timestamp} ---\nX-Battery 诊断信息\n版本: {version}\n操作: {operation}\n消息: {message}\n")
    }

    fn rendered(log: &str) -> String {
        summarize(log).events.into_iter().collect::<Vec<_>>().join("\n")
    }

    fn draft(diagnostics: String) -> super::super::client::Draft {
        super::super::client::Draft {
            category: "other".into(), device: device(),
            description: "Settings could not be saved.".into(), diagnostics: Some(diagnostics), images: Vec::new(), capture: None,
        }
    }

    #[test]
    fn software_reports_use_the_existing_feedback_schema_without_a_device() {
        let draft = draft(collect());
        assert!(is_device(&draft.device));
        assert!(super::super::client::validate_details(&draft).is_ok());
    }

    #[test]
    fn log_summaries_keep_error_codes_but_never_copy_messages_or_private_fields() {
        let log = record(1, "1.4.8", "settings_save_failed",
            "替换 C:\\Users\\Alice\\settings.ini 失败 (os error 17); token=private-token\n设备型号: private-alias\n接收器序列号: private-serial\n回溯:\nprivate-account")
            + &record(2, "1.4.9", "feedback_storage_failed", "step=Replace local history; os_error=32");
        let summary = rendered(&log);
        assert!(summary.contains("settings_save_failed"));
        assert!(summary.contains("os_error=17"));
        assert!(summary.contains("os_error=32"));
        for private in ["Alice", "Users", "settings.ini", "private-", "token=", "serial", "step="] {
            assert!(!summary.contains(private), "{private}");
        }
    }

    #[test]
    fn unknown_operations_and_panic_payloads_cannot_export_arbitrary_log_text() {
        let log = record(1, "1.4.9", "private-username", "private-password")
            + "--- 2 ---\nX-Battery 崩溃信息\n版本: 1.4.8\n位置: D:\\private\\source.rs\n消息: private-payload (os error 5)\n位置: src/gui.rs:1:2\n回溯:\n操作: settings_save_failed\n版本: 9.9.9\n";
        let summary = rendered(&log);
        assert!(summary.contains("operation=unknown_operation"));
        assert!(summary.contains("operation=panic; result=failed; time_unix_s=2; version=1.4.8; source=missing_or_unapproved"));
        assert!(!summary.contains("os_error="));
        assert!(!summary.contains("source=src/"));
        assert!(!summary.contains("9.9.9"));
        assert!(!summary.contains("private"));
    }

    #[test]
    fn only_the_latest_twenty_log_events_are_attached() {
        let log: String = (0..128).map(|index| record(index + 1, "1.4.9", "settings_save_failed", &format!("os_error={index}"))).collect();
        let summary = summarize(&log);
        assert_eq!(summary.events.len(), MAX_EVENTS);
        assert_eq!(summary.older_events_omitted, 108);
        assert!(summary.events.front().unwrap().contains("os_error=108"));
        assert!(summary.events.back().unwrap().contains("os_error=127"));
        assert!(rendered(&log).len() < 20 * 1024);
    }

    #[test]
    fn storage_events_preserve_codes_in_current_and_legacy_formats() {
        let event = super::super::client::storage_event("Replace local history", &std::io::Error::from_raw_os_error(32));
        let log = record(1, "1.4.8", "feedback_storage_failed", &event)
            + &record(2, "1.4.9", "device_order_save_failed", "os_error=Some(17)");
        let summary = summarize(&log);
        assert!(summary.events[0].starts_with("operation=feedback_storage_failed; result=failed;"));
        assert!(summary.events[0].contains("os_error=32"));
        assert!(summary.events[1].starts_with("operation=device_order_save_failed; result=failed;"));
        assert!(summary.events[1].contains("os_error=17"));
        let summary = rendered(&log);
        assert!(!summary.contains("Replace local history"));
    }

    #[test]
    fn timeline_retains_event_versions_and_append_order_across_clock_changes() {
        let log = record(1_795_630_002, "1.4.8", "device_rename_requested", "key=private")
            + &record(1_795_630_001, "1.4.9.0", "device_rename_saved", "key=private")
            + &record(1_795_630_001, "1.4.9", "taskbar_widget_position_failed", "GetLastError=5");
        let summary = summarize(&log.replace('\n', "\r\n"));
        assert!(summary.events[0].contains("time_unix_s=1795630002; version=1.4.8"));
        assert!(summary.events[1].contains("time_unix_s=1795630001; version=1.4.9.0"));
        assert!(summary.events[2].contains("time_unix_s=1795630001; version=1.4.9"));
        assert!(summary.events[2].ends_with("os_error=5"));
    }

    #[test]
    fn metadata_cannot_smuggle_paths_tokens_or_unbounded_numbers() {
        for version in ["C:\\Users\\private", "1.4.9;token=private", "1.4.9+private",
            "1.4.9-beta.private", "１.４.９", "1.4", "01.4.9", "65536.4.9", "1.2.3.4.5"] {
            let summary = rendered(&record(1, version, "device_identified", "ok"));
            assert!(summary.ends_with("version=missing_or_invalid"), "{version}");
            assert!(!summary.contains(version), "{version}");
        }
        for timestamp in ["private", "0", "-1", "1;token=private", "253402300800", "18446744073709551615"] {
            let log = record(1, "1.4.9", "device_identified", "ok").replacen("--- 1 ---", &format!("--- {timestamp} ---"), 1);
            assert!(rendered(&log).contains("time_unix_s=missing_or_invalid; version=1.4.9"));
        }
        let summary = rendered("--- 1 ---\nX-Battery 诊断信息\n操作: device_identified\n消息: ok\n版本: 9.9.9\n");
        assert!(summary.ends_with("version=missing_or_invalid"));
    }

    #[test]
    fn every_current_event_call_has_an_explicit_operation() {
        for source in [include_str!("../gui.rs"), include_str!("../settings.rs"),
            include_str!("client.rs"), include_str!("../taskbar_widget.rs"),
            include_str!("../store/api.rs"), include_str!("../store/mod.rs"),
            include_str!("../store/restart.rs"), include_str!("../main.rs")] {
            for call in source.split("diagnostics::event").skip(1) {
                let argument = call.split_once('(').unwrap().1.trim_start().strip_prefix('"').unwrap();
                let name = argument.split_once('"').unwrap().0;
                assert_eq!(operation(name), name);
            }
        }
        for name in ["device_open_failed", "device_read_failed", "device_identified"] {
            assert_eq!(operation(name), name);
        }
        assert_eq!(operation("taskbar_widget_private"), "unknown_operation");
    }

    #[test]
    fn hresult_and_win32_formats_are_normalized_without_copying_error_text() {
        let log = record(1, "1.4.9", "taskbar_widget_create_failed",
            "GetLastError=5; HRESULT=0x80070005; HRESULT: -2147024891; HRESULT(0X80004005); Win32 错误码 32; GetLastError()=87; os_error=Some(17); os error 0x00000006; hresult=2147942487")
            + &record(2, "1.4.9", "store_restart_registration_failed", "0x80070005");
        let summary = summarize(&log);
        for code in ["os_error=5", "hresult=0x80070005", "hresult=0x80004005", "os_error=32",
            "os_error=87", "os_error=17", "os_error=6", "hresult=0x80070057"] {
            assert!(summary.events[0].contains(code), "{code}");
        }
        assert_eq!(summary.events[0].matches("hresult=0x80070005").count(), 1);
        assert!(!summary.events[0].contains("error_codes_truncated"));
        assert!(summary.events[1].ends_with("hresult=0x80070005"));
        for message in ["显示系统通知失败：0x80070005", "RoInitialize 失败：0x80004005"] {
            let notification = rendered(&record(3, "1.4.9", "low_battery_notification_failed", message));
            assert!(notification.contains("hresult=0x800"));
            let unrelated = rendered(&record(3, "1.4.9", "device_identified", message));
            assert!(!unrelated.contains("hresult="));
        }
        let successful = rendered(&record(3, "1.4.9", "device_identified", "GetLastError=0; HRESULT=1"));
        assert!(successful.contains("result=recorded"));
    }

    #[test]
    fn malformed_codes_and_numeric_credentials_are_not_truncated_into_valid_codes() {
        let messages = ["os_error=123private", "os_error=4294967296", "os_error=-1", "os_error=+5",
            "os_error=５", "os_error=5.5", "os_error=5/private", "os_error=5\\private",
            "os_error=5@private", "os_error=5:private", "os_error=Some(5", "os_error=Some(5)private",
            "token=os_error=12345", "private_os_error=5", "https://private/GetLastError=5",
            "HRESULT=0x80070005private", "HRESULT=0x100000000", "HRESULT=-2147483649",
            "HRESULT(0x80070005", "HRESULT=0x", "token=0x80070005", "0x80070005"];
        for message in messages {
            let summary = rendered(&record(1, "1.4.9", "device_identified", message));
            assert!(!summary.contains("os_error="), "{message}");
            assert!(!summary.contains("hresult="), "{message}");
            assert!(!summary.contains("private"));
        }
    }

    #[test]
    fn code_capacity_is_shared_bounded_and_explicit() {
        let repeated = "os_error=5; ".repeat(1000);
        let summary = rendered(&record(1, "1.4.9", "device_read_failed", &repeated));
        assert_eq!(summary.matches("os_error=").count(), 1);
        assert!(!summary.contains("error_codes_truncated"));
        let many: String = (1..=100).map(|code| format!("os_error={code}; ")).collect();
        let log: String = (1..=MAX_EVENTS as u64).map(|stamp|
            record(stamp, "65535.65535.65535.65535", "taskbar_widget_register_failed", &many)).collect();
        let summary = rendered(&log);
        assert_eq!(summary.matches("os_error=").count(), MAX_ERROR_CODES * MAX_EVENTS);
        assert_eq!(summary.matches("error_codes_truncated=true").count(), MAX_EVENTS);
        assert!(summary.len() < 16 * 1024);
        assert!(super::super::client::validate_details(&draft(summary)).is_ok());
    }

    #[test]
    fn panic_locations_require_known_repo_sources_and_valid_coordinates() {
        for (location, expected) in [("src/gui.rs:4274:9", "src/gui.rs:4274:9"),
            ("src\\feedback\\software.rs:10:2", "src/feedback/software.rs:10:2"),
            ("src/backends/rapoo.rs:150:1", "src/backends/rapoo.rs:150:1")] {
            let log = format!("--- 1 ---\nX-Battery 崩溃信息\n版本: 1.4.8\n位置: {location}\n消息: private-payload\n线程: private-thread\n回溯:\nC:\\private\\file.rs\n");
            let summary = rendered(&log);
            assert!(summary.ends_with(&format!("source={expected}")));
            assert!(super::super::client::validate_details(&draft(summary.clone())).is_ok());
            assert!(!summary.contains("private"));
        }
        for location in ["C:\\repo\\src\\gui.rs:1:2", "/repo/src/gui.rs:1:2",
            "\\\\server\\src\\gui.rs:1:2", "src/../src/gui.rs:1:2", "src//gui.rs:1:2",
            "src/private.rs:1:2", "src/private/gui.rs:1:2", "src/GUI.rs:1:2",
            "src/gui.rs:0:1", "src/gui.rs:1:0", "src/gui.rs:-1:2", "src/gui.rs:1:4294967296",
            "src/gui.rs:1:2;token=private", "src/gui.rs:1:2\rprivate", "src/gui.rs:１:2",
            "C:src/gui.rs:1:2", "src/gui.rs:1", "../src/gui.rs:1:2"] {
            assert!(source_location(location).is_none(), "{location}");
        }
    }

    #[test]
    fn real_diagnostic_facts_keep_duration_but_ignore_device_fields() {
        let facts = crate::diagnostics::Facts {
            version: "1.4.8", operation: "device_read_failed", status: "无法打开设备（GetLastError=5）",
            family: "mouse", frames: 0, elapsed_ms: 4012, history_rows: 1, device: None,
        };
        let log = format!("--- 1 ---\n{}设备型号: private-alias\n版本: 9.9.9\n本轮耗时: 123 ms\n", crate::diagnostics::render(&facts));
        let summary = rendered(&log);
        assert!(summary.contains("version=1.4.8; elapsed_ms=4012; os_error=5"));
        assert!(!summary.contains("private"));
        assert!(!summary.contains("123"));
        let message = rendered(&record(1, "1.4.9", "device_rename_saved", "key=private\n本轮耗时: 123 ms\n版本: 9.9.9"));
        assert!(!message.contains("elapsed_ms="));
        assert!(!message.contains("9.9.9"));
    }

    #[test]
    fn tail_summary_marks_partial_records_encoding_errors_and_omissions() {
        let mut bytes = b"\xffprivate-fragment\n".to_vec();
        for stamp in 1..=22 { bytes.extend(record(stamp, "1.4.9", "device_identified", "ok").as_bytes()); }
        bytes.extend(b"--- 23 ---\nnot a diagnostic record\n--- 24 ---\nX-Battery ");
        let (status, events) = summarize_tail(&bytes, 4096, true);
        for field in ["earlier_bytes_omitted=4096", "short_read=true", "invalid_utf8=true",
            "events_kept=20", "older_events_omitted=2", "unparsed_records=1",
            "leading_fragment_omitted=true", "trailing_record_omitted=true"] {
            assert!(status.contains(field), "{field}");
        }
        assert!(events[0].contains("time_unix_s=3;"));
        assert!(events[19].contains("time_unix_s=22;"));
        assert!(!events.join("\n").contains("private"));
        let (empty, events) = summarize_tail(&[], 0, false);
        assert!(empty.contains("available;"));
        assert!(empty.contains("events_kept=0"));
        assert!(events.is_empty());
        assert!(summarize("版本: 1.4.9\n操作: settings_save_failed\n消息: os_error=5\n").events.is_empty());
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
