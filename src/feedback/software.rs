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
    network: Option<NetworkFields>,
    launch: Option<String>,
}

struct NetworkFields {
    step: &'static str,
    os_error: u32,
    http_status: u16,
    elapsed_ms: u64,
    attempt: u32,
}

fn network_fields(value: &str) -> Option<NetworkFields> {
    let mut fields = value.split_ascii_whitespace();
    let step = match fields.next()?.strip_prefix("step=")? {
        "open" => "open", "configure" => "configure", "connect" => "connect",
        "request" => "request", "send" => "send", "receive" => "receive",
        "headers" => "headers", "status" => "status", "content_type" => "content_type",
        "deadline" => "deadline", "read" => "read", "response_size" => "response_size",
        _ => return None,
    };
    let os_error = u32::try_from(decimal(fields.next()?.strip_prefix("os_error=")?, 10)?).ok()?;
    let http_status = u16::try_from(decimal(fields.next()?.strip_prefix("http_status=")?, 3)?).ok()?;
    if http_status != 0 && !(100..=599).contains(&http_status) { return None; }
    let elapsed_ms = decimal(fields.next()?.strip_prefix("elapsed_ms=")?, 20)?;
    let attempt = u32::try_from(decimal(fields.next()?.strip_prefix("attempt=")?, 10)?).ok()?;
    if attempt == 0 || fields.next().is_some() { return None; }
    Some(NetworkFields { step, os_error, http_status, elapsed_ms, attempt })
}

fn operation(value: &str) -> &'static str {
    match value.trim() {
        "app_launch" => "app_launch",
        "app_launch_window" => "app_launch_window",
        "settings_save_failed" => "settings_save_failed",
        "settings_shutdown_failed" => "settings_shutdown_failed",
        "device_order_save_failed" => "device_order_save_failed",
        "device_rename_requested" => "device_rename_requested",
        "device_rename_saved" => "device_rename_saved",
        "device_rename_failed" => "device_rename_failed",
        "device_open_failed" => "device_open_failed",
        "device_read_failed" => "device_read_failed",
        "device_detail_failed" => "device_detail_failed",
        "device_discovery_failed" => "device_discovery_failed",
        "device_worker_start_failed" => "device_worker_start_failed",
        "device_identified" => "device_identified",
        "feedback_storage_failed" => "feedback_storage_failed",
        "feedback_network_failed" => "feedback_network_failed",
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

fn launch_fields(operation: &str, value: &str) -> String {
    let fields: &[(&str, &[&str])] = if operation == "app_launch" {
        &[
            ("source", &["Manual", "Autostart", "AfterUpdate"]),
            ("packaged", &["true", "false"]),
            ("instance", &["owner", "already_running", "unavailable"]),
            ("activated", &["true", "false"]),
        ]
    } else {
        &[
            ("start_hidden", &["true", "false"]),
            ("tray_enabled", &["true", "false"]),
            ("taskbar_widget_enabled", &["true", "false"]),
            ("show_main", &["true", "false"]),
        ]
    };
    let mut result = String::new();
    for &(name, allowed) in fields {
        let mut values = value.split_ascii_whitespace().filter_map(|field| field.split_once('='))
            .filter_map(|(key, value)| (key == name).then_some(value));
        if let Some(value) = values.next().filter(|value| allowed.contains(value)) {
            if values.next().is_none() { let _ = write!(result, "; {name}={value}"); }
        }
    }
    result
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
            "audio", "autostart", "backup", "capture", "chart", "cli", "commands", "device", "devices",
            "diagnostics", "dwm", "dynlight", "feedback", "gui", "hid", "history", "i18n",
            "linkstate", "main", "mock", "notifications", "ops", "paths", "proto", "report",
            "settings", "snapshot", "taskbar_widget", "tray_icon", "tray_prefs", "win32", "winrt",
        ],
        "src/backends" => &[
            "ajazz_ak980", "atk_holtek", "atk_nano", "aula", "aula_f108", "aula_f99",
            "aula_hfd", "aula_hfd_mechanical", "aula_v9pro", "bluetooth", "channel", "cherry",
            "compx", "dareu", "dareu_em901", "descriptor", "edifier", "eightbitdo", "enumeration",
            "eweadn", "eweadn_s9pro", "eweadn_x23", "eweadn_x87", "feature", "fifine_ht6",
            "flydigi", "g3m", "gamesir", "gearhub", "headset", "hyperx", "inphic", "inzone", "keyboard20",
            "lingbao", "lofree", "logitech", "logitech_headset", "mchose_glw", "mchose_k87s",
            "mchose_k99", "mchose_mc", "mchose_modern", "mchose_mouse", "mchose_qhw",
            "mchose_r9", "mod", "mojhon", "moondrop", "page_command", "pulsar", "rapoo", "rapoo_config",
            "rapoo_legacy", "rapoo_mt560", "rapoo_office", "razer", "razer_capabilities", "rog",
            "sony", "standard", "status", "steelseries", "trace", "veekos", "vgn", "vgn_beiying",
            "vgn_dragonfly3", "vgn_keyboard", "vgn_ry", "xiberia", "xinput",
        ],
        "src/ui" => &[
            "canvas", "device_name", "dropdown", "feedback", "feedback_icons", "frame", "gfx",
            "layout", "mod", "pages", "palette", "panel_icons", "store_updates", "theme",
            "titlebar", "tray_popup", "viewdata", "widget",
        ],
        "src/ui/feedback" => &["capture", "history", "layout", "state"],
        "src/gui" => &["device_workers", "rog_refresh", "rapoo_devices"],
        "src/feedback" => &["capture", "client", "export", "inventory", "local_files", "software"],
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
        if matches!(self.operation, "app_launch" | "app_launch_window") {
            self.launch = Some(launch_fields(self.operation, value));
            return;
        }
        if self.operation == "feedback_network_failed" {
            self.network = network_fields(value);
            return;
        }
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
        if let Some(launch) = self.launch { line.push_str(&launch); }
        if let Some(network) = self.network {
            let _ = write!(line, "; step={}; os_error={}; http_status={}; elapsed_ms={}; attempt={}",
                network.step, network.os_error, network.http_status, network.elapsed_ms, network.attempt);
        } else if self.operation == "feedback_network_failed" {
            line.push_str("; network_fields=missing_or_invalid");
        }
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
        elapsed: None, codes: Vec::new(), codes_truncated: false, location: None, network: None, launch: None,
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
    network_only: bool,
    events: VecDeque<String>,
    older_events_omitted: usize,
    unparsed_records: usize,
    leading_fragment_omitted: bool,
    trailing_record_omitted: bool,
}

impl Summary {
    fn push_record(&mut self, record: &str) {
        let Some(event) = parse_record(record) else { self.unparsed_records += 1; return; };
        if self.network_only && event.operation != "feedback_network_failed" { return; }
        self.events.push_back(event.render());
        if self.events.len() > MAX_EVENTS {
            self.events.pop_front();
            self.older_events_omitted += 1;
        }
    }
}

#[cfg(test)]
fn summarize(log: &str) -> Summary {
    summarize_filtered(log, false)
}

fn summarize_filtered(log: &str, network_only: bool) -> Summary {
    let mut summary = Summary { network_only, ..Summary::default() };
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

#[cfg(test)]
fn summarize_tail(bytes: &[u8], earlier_bytes_omitted: u64, short_read: bool) -> (String, Vec<String>) {
    summarize_tail_filtered(bytes, earlier_bytes_omitted, short_read, false)
}

fn summarize_tail_filtered(bytes: &[u8], earlier_bytes_omitted: u64, short_read: bool,
    network_only: bool) -> (String, Vec<String>) {
    let log = String::from_utf8_lossy(bytes);
    let summary = summarize_filtered(&log, network_only);
    let status = format!(
        "available; safe summary only; tail_bytes={}; byte_limit={MAX_LOG_BYTES}; earlier_bytes_omitted={earlier_bytes_omitted}; short_read={short_read}; invalid_utf8={}; events_kept={}; event_limit={MAX_EVENTS}; older_events_omitted={}; unparsed_records={}; leading_fragment_omitted={}; trailing_record_omitted={}",
        bytes.len(), matches!(log, std::borrow::Cow::Owned(_)), summary.events.len(),
        summary.older_events_omitted, summary.unparsed_records, summary.leading_fragment_omitted,
        summary.trailing_record_omitted);
    (status, summary.events.into_iter().collect())
}

#[cfg(test)]
fn log_summary(path: &Path) -> (String, Vec<String>) {
    log_summary_filtered(path, false)
}

fn log_summary_filtered(path: &Path, network_only: bool) -> (String, Vec<String>) {
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
        Ok((bytes, offset, short_read)) => summarize_tail_filtered(&bytes, offset, short_read, network_only),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ("not recorded yet".into(), Vec::new()),
        Err(error) => (format!("unavailable; os_error={}",
            error.raw_os_error().map(|code| code.to_string()).unwrap_or_else(|| "missing".into())), Vec::new()),
    }
}

fn stored_summary(path: &Path, fallback: &crate::diagnostics::LogFallback,
    legacy: impl FnOnce() -> Vec<std::path::PathBuf>, network_only: bool) -> (String, Vec<String>) {
    let limit = if network_only { 1 } else { MAX_EVENTS };
    let mut memory = VecDeque::new();
    let mut persisted = Vec::new();
    for record in &fallback.records {
        let Some(event) = parse_record(&record.text) else { continue; };
        if network_only && event.operation != "feedback_network_failed" { continue; }
        let event = event.render();
        if record.persisted { persisted.push(event.clone()); }
        memory.push_back(event);
        if memory.len() > limit { memory.pop_front(); }
    }
    let mut status = vec![format!(
        "memory_records={}; memory_limit_bytes=65536; memory_limit_records=64; memory_records_dropped={}; write_failures={}; last_write_os_error={}",
        fallback.records.len(), fallback.records_dropped, fallback.failures,
        fallback.last_os_error.map(|code| code.to_string()).unwrap_or_else(|| "missing".into()))];
    let mut events = VecDeque::new();
    let mut read = |path: &Path, source: &str| {
        if events.len() + memory.len() >= limit { return limit; }
        let (source_status, source_events) = log_summary_filtered(path, network_only);
        status.push(format!("{source}: {source_status}"));
        for event in source_events.into_iter().rev() {
            if let Some(index) = persisted.iter().rposition(|entry| entry == &event) {
                persisted.remove(index);
                continue;
            }
            events.push_front(event);
            if events.len() + memory.len() >= limit { break; }
        }
        events.len() + memory.len()
    };
    let mut kept = read(path, "current");
    for generation in 1..=crate::diagnostics::LOG_BACKUPS {
        if kept >= limit { break; }
        kept = read(&crate::diagnostics::rotated_path(path, generation), &format!("rotation_{generation}"));
    }
    if kept == 0 {
        for (index, path) in legacy().into_iter().rev().filter(|old| old != path).enumerate() {
            if read(&path, &format!("legacy_{index}")) >= limit { break; }
        }
    }
    events.extend(memory);
    (status.join("\nLog source "), events.into_iter().collect())
}

fn diagnostic_summary(network_only: bool) -> (String, Vec<String>) {
    stored_summary(&crate::diagnostics::log_path(), &crate::diagnostics::log_fallback(),
        crate::diagnostics::legacy_log_paths, network_only)
}

pub fn network_summary() -> String {
    let (status, events) = diagnostic_summary(true);
    let Some(event) = events.last() else {
        return "Latest network failure: not recorded in the bounded log sample.\n".into();
    };
    format!("Latest network failure (safe fields only; sampled disk logs or current-run memory)\n{}\n{event}\n",
        status.lines().next().unwrap_or_default())
}

pub fn collect() -> String {
    let (status, events) = diagnostic_summary(false);
    let distribution = if crate::autostart::is_packaged() { "Microsoft Store / MSIX" } else { "Portable" };
    let mut report = format!("App: X-Battery {}\nSystem: {}\nDistribution: {distribution}\nLog: {status}\nSoftware log summary\nTimeline: legacy logs, disk rotations, then current-run memory; append order within each source; time_unix_s is wall-clock UTC seconds, not monotonic; version is per event.\nLimits: latest complete records within the byte/event caps; omitted counts describe only each sampled tail unless named earlier_bytes_omitted.\nMissing: missing_or_invalid means absent or rejected metadata; missing_or_unapproved source is not exported; absent codes/timings are not proof of success.\nSafety: known operations, numeric codes, fixed network stages and approved relative Rust locations only; no raw messages, panic payloads or backtraces.\n",
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
    fn startup_feedback_preserves_launch_and_visibility_decisions() {
        let log = record(1, "1.5.0", "app_launch",
            "source=Autostart packaged=true pid=123 parent=Some(\"private.exe\") instance=already_running activated=false")
            + &record(2, "1.5.0", "app_launch_window",
                "start_hidden=true tray_enabled=false taskbar_widget_enabled=true show_main=false");
        let summary = rendered(&log);
        assert!(summary.contains("source=Autostart; packaged=true; instance=already_running; activated=false"));
        assert!(summary.contains("start_hidden=true; tray_enabled=false; taskbar_widget_enabled=true; show_main=false"));
        for private in ["pid=", "123", "parent=", "private.exe"] {
            assert!(!summary.contains(private));
        }
    }

    #[test]
    fn startup_feedback_rejects_arbitrary_and_conflicting_field_values() {
        let log = record(1, "1.5.0", "app_launch",
            "source=Manual source=Autostart packaged=true;token=private instance=C:\\private activated=private os_error=5")
            + &record(2, "1.5.0", "app_launch_window",
                "start_hidden=private tray_enabled=true tray_enabled=false taskbar_widget_enabled=private show_main=false;private");
        let summary = rendered(&log);
        for field in ["source=", "packaged=", "instance=", "activated=", "start_hidden=", "tray_enabled=",
            "taskbar_widget_enabled=", "show_main=", "private", "os_error="] {
            assert!(!summary.contains(field), "{field}");
        }
    }

    #[test]
    fn a_complete_new_log_does_not_report_a_leading_fragment() {
        let summary = summarize(&record(1, "1.4.11", "device_identified", "ok"));
        assert_eq!(summary.events.len(), 1);
        assert!(!summary.leading_fragment_omitted);
        assert!(!summary.trailing_record_omitted);
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
            include_str!("../gui/device_workers.rs"), include_str!("../gui/rog_refresh.rs"),
            include_str!("../gui/rapoo_devices.rs"), include_str!("../capture.rs"),
            include_str!("capture.rs"), include_str!("export.rs"),
            include_str!("client.rs"), include_str!("../taskbar_widget.rs"),
            include_str!("../store/api.rs"), include_str!("../store/mod.rs"),
            include_str!("../store/restart.rs"), include_str!("../main.rs")] {
            for call in source.split("diagnostics::event").skip(1) {
                let argument = call.split_once('(').unwrap().1.trim_start().strip_prefix('"').unwrap();
                let name = argument.split_once('"').unwrap().0;
                assert_eq!(operation(name), name);
            }
        }
        for name in ["device_open_failed", "device_read_failed", "device_detail_failed", "device_identified"] {
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
            ("src/capture.rs:1:2", "src/capture.rs:1:2"),
            ("src/feedback/capture.rs:1:2", "src/feedback/capture.rs:1:2"),
            ("src/feedback/export.rs:1:2", "src/feedback/export.rs:1:2"),
            ("src/feedback/local_files.rs:1:2", "src/feedback/local_files.rs:1:2"),
            ("src/backends/pulsar.rs:1:2", "src/backends/pulsar.rs:1:2"),
            ("src/backends/inphic.rs:1:2", "src/backends/inphic.rs:1:2"),
            ("src/backends/mojhon.rs:1:2", "src/backends/mojhon.rs:1:2"),
            ("src/ui/feedback/history.rs:1:2", "src/ui/feedback/history.rs:1:2"),
            ("src/ui/feedback/capture.rs:1:2", "src/ui/feedback/capture.rs:1:2"),
            ("src/gui/device_workers.rs:1:2", "src/gui/device_workers.rs:1:2"),
            ("src/gui/rog_refresh.rs:1:2", "src/gui/rog_refresh.rs:1:2"),
            ("src/gui/rapoo_devices.rs:1:2", "src/gui/rapoo_devices.rs:1:2"),
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

    fn network_record(stamp: u64, attempt: u32) -> String {
        record(stamp, "1.4.11", "feedback_network_failed",
            &format!("step=send os_error=12002 http_status=0 elapsed_ms=15000 attempt={attempt}"))
    }

    #[test]
    fn network_events_preserve_only_fixed_stages_and_numeric_fields() {
        for step in ["open", "configure", "connect", "request", "send", "receive", "headers",
            "status", "content_type", "deadline", "read", "response_size"] {
            let log = network_record(1, 2).replace("step=send", &format!("step={step}"));
            let summary = rendered(&log);
            assert!(summary.contains(&format!("step={step}; os_error=12002; http_status=0; elapsed_ms=15000; attempt=2")));
        }
        for status in [100, 200, 403, 429, 503, 599] {
            let log = network_record(1, 1).replace("http_status=0", &format!("http_status={status}"));
            assert!(rendered(&log).contains(&format!("http_status={status};")));
        }
    }

    #[test]
    fn malformed_network_fields_cannot_export_credentials_urls_or_partial_numbers() {
        for (original, replacement) in [
            ("step=send", "step=https://private/send"), ("step=send", "step=private"),
            ("os_error=12002", "os_error=4294967296"), ("os_error=12002", "os_error=12002private"),
            ("http_status=0", "http_status=99"), ("http_status=0", "http_status=600"),
            ("http_status=0", "http_status=403/private"), ("elapsed_ms=15000", "elapsed_ms=-1"),
            ("elapsed_ms=15000", "elapsed_ms=18446744073709551616"),
            ("attempt=1", "attempt=0"), ("attempt=1", "attempt=１"),
            ("attempt=1", "attempt=1 attempt=2"), ("attempt=1", "attempt=1 bearer=private"),
            ("step=send", "body=private step=send"), ("os_error=12002", "token=os_error=12002"),
        ] {
            let summary = rendered(&network_record(1, 1).replace(original, replacement));
            assert!(summary.ends_with("network_fields=missing_or_invalid"), "{replacement}: {summary}");
            for excluded in ["private", "step=", "os_error=", "http_status=", "elapsed_ms=", "attempt="] {
                assert!(!summary.contains(excluded), "{replacement}: {summary}");
            }
        }
        let summary = rendered(&(network_record(1, 1) + "body=private bearer=private https://private/\n"));
        assert!(summary.ends_with("attempt=1"));
        assert!(!summary.contains("private"));
    }

    #[test]
    fn newest_network_event_is_selected_before_the_general_event_limit() {
        let scratch = crate::diagnostics::tests::Scratch::new();
        let path = scratch.0.join("diagnostics.log");
        let mut log = network_record(20, 1) + &network_record(10, 2);
        for stamp in 30..100 { log.push_str(&record(stamp, "1.4.11", "device_identified", "private")); }
        std::fs::write(&path, log).unwrap();
        let (_, events) = stored_summary(&path, &Default::default(), || panic!("legacy must be lazy"), true);
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("time_unix_s=10"));
        assert!(events[0].ends_with("attempt=2"));
    }

    #[test]
    fn memory_network_failure_survives_disk_failure_and_a_later_success_wins() {
        let scratch = crate::diagnostics::tests::Scratch::new();
        let path = scratch.0.join("diagnostics.log");
        std::fs::create_dir(&path).unwrap();
        let mut fallback = crate::diagnostics::LogFallback {
            failures: 1, last_os_error: Some(112), ..Default::default()
        };
        fallback.records.push_back(crate::diagnostics::MemoryRecord { text: network_record(20, 1), persisted: false });
        let (status, events) = stored_summary(&path, &fallback, || panic!("memory is sufficient"), true);
        assert_eq!(events.len(), 1);
        assert!(events[0].ends_with("attempt=1"));
        assert!(status.contains("write_failures=1; last_write_os_error=112"));
        fallback.records.push_back(crate::diagnostics::MemoryRecord { text: network_record(10, 2), persisted: true });
        let (_, events) = stored_summary(&path, &fallback, Vec::new, true);
        assert_eq!(events.len(), 1);
        assert!(events[0].ends_with("attempt=2"));
    }

    #[test]
    fn summaries_read_rotations_and_legacy_on_demand_without_modifying_them() {
        let scratch = crate::diagnostics::tests::Scratch::new();
        let path = scratch.0.join("diagnostics.log");
        let legacy = scratch.0.join("legacy-private-location.log");
        let old_log = network_record(1, 1);
        std::fs::write(&legacy, &old_log).unwrap();
        let (status, events) = stored_summary(&path, &Default::default(), || vec![legacy.clone()], true);
        assert!(status.contains("legacy_0: available"));
        assert!(!status.contains("private"));
        assert!(events[0].ends_with("attempt=1"));
        assert_eq!(std::fs::read_to_string(&legacy).unwrap(), old_log);
        assert!(!path.exists());
        std::fs::write(crate::diagnostics::rotated_path(&path, 1), network_record(2, 2)).unwrap();
        let (status, events) = stored_summary(&path, &Default::default(), || panic!("rotation is sufficient"), true);
        assert!(status.contains("rotation_1: available"));
        assert!(events[0].ends_with("attempt=2"));
        let (_, events) = stored_summary(&path, &Default::default(), || panic!("no legacy reads after migration"), false);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn summaries_deduplicate_memory_copies_and_never_export_private_device_facts() {
        let scratch = crate::diagnostics::tests::Scratch::new();
        let path = scratch.0.join("diagnostics.log");
        let facts = crate::diagnostics::Facts {
            version: "1.4.11", operation: "device_read_failed", status: "private-status os_error=5",
            family: "private-family", frames: 1, elapsed_ms: 100, history_rows: 1,
            device: Some(&crate::diagnostics::DeviceInfo {
                key: "private-key".into(), model: "private-alias".into(), receiver_name: "private-name".into(),
                receiver_serial: "private-serial".into(), receiver_mac: "private-mac".into(),
                receiver_firmware: "private-firmware".into(), ..Default::default()
            }),
        };
        let fact_record = format!("--- 2 ---\n{}", crate::diagnostics::render(&facts));
        let earlier = record(1, "1.4.11", "device_identified", "private");
        std::fs::write(&path, earlier + &fact_record).unwrap();
        let mut fallback = crate::diagnostics::LogFallback::default();
        fallback.records.push_back(crate::diagnostics::MemoryRecord { text: fact_record, persisted: true });
        fallback.records.push_back(crate::diagnostics::MemoryRecord { text: network_record(3, 1), persisted: false });
        let (status, events) = stored_summary(&path, &fallback, Vec::new, false);
        assert_eq!(events.len(), 3);
        assert!(events[0].contains("device_identified"));
        assert!(events[1].contains("device_read_failed"));
        assert!(events[2].contains("feedback_network_failed"));
        assert!(!events.join("\n").contains("private"));
        assert!(!status.contains("private"));
    }
}
