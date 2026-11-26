use crate::hid::HidInterface;
use serde::Serialize;
use std::{
    fmt::Write as _,
    io::Write as _,
    path::{Path, PathBuf},
    sync::{atomic::{AtomicBool, Ordering}, Mutex, OnceLock},
    time::{Duration, Instant},
};

const MAX_BYTES: usize = 512 * 1024;
const MAX_RECORDS: usize = 1024;
const MAX_SECONDS: u64 = 300;
pub const STEPS: [(&str, &str); 5] = [
    ("正常使用", "Normal use"),
    ("接上充电线", "Connect charging cable"),
    ("拔掉充电线", "Disconnect charging cable"),
    ("关闭外设电源", "Power off peripheral"),
    ("重新开启外设", "Power on peripheral"),
];

static ACTIVE: AtomicBool = AtomicBool::new(false);
static SESSION: OnceLock<Mutex<Option<Session>>> = OnceLock::new();

#[derive(Serialize)]
struct Metadata {
    schema: u8,
    app: &'static str,
    started_unix_ms: u128,
    vendor_id: u16,
    product_id: u16,
    usage_page: u16,
    usage: u16,
    input_bytes: u16,
    output_bytes: u16,
    feature_bytes: u16,
    scope: &'static str,
    privacy: &'static str,
}

struct Session {
    path: String,
    started: Instant,
    metadata: Metadata,
    records: Vec<String>,
    bytes: usize,
    frames: usize,
    steps: Vec<(u128, usize)>,
    stopped: Option<&'static str>,
}

#[derive(Default)]
pub struct Status {
    pub started: bool,
    pub running: bool,
    pub frames: usize,
    pub elapsed_seconds: u64,
    pub preview: Vec<String>,
    pub summary: String,
}

fn cell() -> &'static Mutex<Option<Session>> { SESSION.get_or_init(Default::default) }

pub fn eligible(interface: &HidInterface) -> bool {
    interface.usage_page >= 0xff00
        && [interface.input_len, interface.output_len, interface.feature_len]
            .into_iter().any(|length| (2..=4096).contains(&length))
}

impl Session {
    fn new(interface: &HidInterface, now: Instant) -> Self {
        Self {
            path: interface.path.to_ascii_lowercase(), started: now,
            metadata: Metadata {
                schema: 1, app: crate::gui::APP_VERSION,
                started_unix_ms: crate::diagnostics::unix_millis(),
                vendor_id: interface.vendor_id, product_id: interface.product_id,
                usage_page: interface.usage_page, usage: interface.usage,
                input_bytes: interface.input_len, output_bytes: interface.output_len,
                feature_bytes: interface.feature_len,
                scope: "one interface; all receiver slots; existing application I/O only; timestamps are completion times",
                privacy: "LOCAL RAW DATA: may contain device identifiers or input data. Review before sharing. Not attached to feedback.",
            },
            records: Vec::new(), bytes: 0, frames: 0, steps: Vec::new(), stopped: None,
        }
    }

    fn expire(&mut self, now: Instant) {
        if self.stopped.is_none() && now.duration_since(self.started) >= Duration::from_secs(MAX_SECONDS) {
            self.stopped = Some("time-limit");
        }
    }

    fn append(&mut self, value: serde_json::Value, now: Instant) -> bool {
        self.expire(now);
        if self.stopped.is_some() { return false; }
        let line = value.to_string();
        if self.records.len() >= MAX_RECORDS || self.bytes + line.len() + 1 > MAX_BYTES {
            self.stopped = Some("size-limit");
            return false;
        }
        self.bytes += line.len() + 1;
        self.records.push(line);
        true
    }

    fn frame(&mut self, path: &str, api: &str, data: &[u8], success: bool, now: Instant) {
        if !self.path.eq_ignore_ascii_case(path) || data.len() > 4096 { return; }
        let mut hex = String::with_capacity(data.len() * 2);
        for byte in data { let _ = write!(hex, "{byte:02X}"); }
        if self.append(serde_json::json!({"kind":"transfer", "elapsed_ms":now.duration_since(self.started).as_millis(),
            "api":api, "success":success, "hex":hex}), now) { self.frames += 1; }
    }

    fn mark(&mut self, step: usize, now: Instant) {
        let Some((_, label)) = STEPS.get(step) else { return; };
        let elapsed = now.duration_since(self.started).as_millis();
        if self.append(serde_json::json!({"kind":"step", "elapsed_ms":elapsed,
            "step":step, "label":label}), now) { self.steps.push((elapsed, step)); }
    }

    fn summary(&self) -> String {
        let mut result = format!("Local capture summary: {:04X}:{:04X}, usage={:04X}:{:04X}\nTransfers: {}; result: {}\n",
            self.metadata.vendor_id, self.metadata.product_id, self.metadata.usage_page,
            self.metadata.usage, self.frames, self.stopped.unwrap_or("running"));
        for (elapsed, step) in &self.steps {
            let _ = writeln!(result, "{elapsed} ms: {}", STEPS[*step].1);
        }
        result.push_str("No frames does not prove an unsupported protocol. Existing polling cadence is unchanged.\n");
        result
    }
}

pub fn start(interface: &HidInterface) -> Result<(), &'static str> {
    if !eligible(interface) { return Err("此接口不支持原始采集 / Capture unavailable for this interface"); }
    let mut guard = cell().lock().unwrap_or_else(|error| error.into_inner());
    if guard.is_some() { return Err("请先保存或清除上次采集 / Save or clear the previous capture first"); }
    let now = Instant::now();
    let mut session = Session::new(interface, now);
    session.mark(0, now);
    *guard = Some(session);
    ACTIVE.store(true, Ordering::Release);
    Ok(())
}

pub fn record(path: &str, api: &'static str, data: &[u8], success: bool) {
    if !ACTIVE.load(Ordering::Acquire) { return; }
    let mut guard = cell().lock().unwrap_or_else(|error| error.into_inner());
    if let Some(session) = guard.as_mut() {
        session.frame(path, api, data, success, Instant::now());
        if session.stopped.is_some() { ACTIVE.store(false, Ordering::Release); }
    }
}

pub fn mark(step: usize) {
    let mut guard = cell().lock().unwrap_or_else(|error| error.into_inner());
    if let Some(session) = guard.as_mut() { session.mark(step, Instant::now()); }
}

pub fn stop() {
    let mut guard = cell().lock().unwrap_or_else(|error| error.into_inner());
    if let Some(session) = guard.as_mut() { session.stopped.get_or_insert("stopped"); }
    ACTIVE.store(false, Ordering::Release);
}

pub fn clear() {
    let mut guard = cell().lock().unwrap_or_else(|error| error.into_inner());
    ACTIVE.store(false, Ordering::Release);
    *guard = None;
}

pub fn status() -> Status {
    let mut guard = cell().lock().unwrap_or_else(|error| error.into_inner());
    let Some(session) = guard.as_mut() else { return Status::default(); };
    session.expire(Instant::now());
    if session.stopped.is_some() { ACTIVE.store(false, Ordering::Release); }
    Status { started: true, running: session.stopped.is_none(), frames: session.frames,
        elapsed_seconds: session.started.elapsed().as_secs().min(MAX_SECONDS),
        preview: session.records.iter().rev().take(6).rev().map(|line| {
            let mut preview: String = line.chars().take(180).collect();
            if line.len() > preview.len() { preview.push_str("…"); }
            preview
        }).collect(), summary: session.summary() }
}

pub struct Export { raw: String, summary: String, timestamp: u128 }

pub fn export() -> Option<Export> {
    stop();
    let guard = cell().lock().unwrap_or_else(|error| error.into_inner());
    let session = guard.as_ref()?;
    let mut raw = serde_json::to_string(&session.metadata).ok()?;
    raw.push('\n');
    for record in &session.records { raw.push_str(record); raw.push('\n'); }
    raw.push_str(&serde_json::json!({"kind":"end", "reason":session.stopped,
        "transfers":session.frames}).to_string());
    raw.push('\n');
    Some(Export { raw, summary: session.summary(), timestamp: session.metadata.started_unix_ms })
}

impl Export {
    pub fn save(self) -> Result<PathBuf, String> {
        self.save_in(&crate::paths::cache_dir().join("captures"))
    }

    fn save_in(self, root: &Path) -> Result<PathBuf, String> {
        std::fs::create_dir_all(root).map_err(|error| error.to_string())?;
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = root.join(format!("capture-{}-{}-{}", self.timestamp, std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).map_err(|error| error.to_string())?;
        for (name, contents) in [("raw.jsonl", self.raw), ("summary.txt", self.summary)] {
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path.join(name))
                .map_err(|error| error.to_string())?;
            file.write_all(contents.as_bytes()).and_then(|_| file.sync_all()).map_err(|error| error.to_string())?;
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn interface(path: &str) -> HidInterface {
        HidInterface { path: path.into(), vendor_id: 0x1234, product_id: 0x5678, usage_page: 0xff00,
            usage: 1, input_len: 64, output_len: 64, feature_len: 0,
            product: "private name".into(), serial: "secret serial".into() }
    }

    #[test]
    fn general_input_collections_are_excluded() {
        let mut candidate = interface("one");
        for page in [1, 7, 12, 0x0d, 0xff] { candidate.usage_page = page; assert!(!eligible(&candidate)); }
        candidate.usage_page = 0xff90;
        assert!(eligible(&candidate));
    }

    #[test]
    fn same_model_receivers_are_kept_separate_and_bytes_preserved() {
        let now = Instant::now();
        let mut session = Session::new(&interface("receiver-a"), now);
        session.frame("receiver-b", "input", &[0x00, 0xab, 0xff], true, now);
        assert_eq!(session.frames, 0);
        session.frame("RECEIVER-A", "input", &[0x00, 0xab, 0xff], true, now);
        assert_eq!(session.frames, 1);
        assert!(session.records[0].contains("00ABFF"));
        assert!(!session.summary().contains("00ABFF"));
        let header = serde_json::to_string(&session.metadata).unwrap();
        assert!(!header.contains("receiver-a"));
        assert!(!header.contains("secret serial"));
        assert!(!header.contains("private name"));
    }

    #[test]
    fn bounded_capture_expires_and_does_not_refresh_on_more_frames() {
        let now = Instant::now();
        let mut session = Session::new(&interface("one"), now);
        session.mark(2, now + Duration::from_secs(1));
        session.frame("one", "input", &[1], true, now + Duration::from_secs(MAX_SECONDS));
        assert_eq!(session.stopped, Some("time-limit"));
        assert_eq!(session.frames, 0);
        assert_eq!(session.steps, vec![(1000, 2)]);
        session.mark(0, now + Duration::from_secs(MAX_SECONDS + 1));
        assert_eq!(session.steps.len(), 1);
    }

    #[test]
    fn byte_budget_accounts_for_hex_and_json_overhead() {
        let now = Instant::now();
        let mut session = Session::new(&interface("one"), now);
        for _iteration in 0..1000 { session.frame("one", "input", &[255;4096], true, now); }
        assert_eq!(session.stopped, Some("size-limit"));
        assert!(session.bytes <= MAX_BYTES);
        assert!(session.frames < MAX_RECORDS);
        assert_eq!(session.bytes, session.records.iter().map(|line| line.len() + 1).sum::<usize>());
    }

    #[test]
    fn export_writes_raw_and_safe_summary_separately_without_replacing_files() {
        let root = std::env::temp_dir().join(format!("x-battery-capture-test-{}-{}",
            std::process::id(), crate::diagnostics::unix_millis()));
        let export = || Export { raw: "{\"hex\":\"DEADBEEF\"}\n".into(), summary: "Transfers: 1\n".into(), timestamp: 1 };
        let first = export().save_in(&root).unwrap();
        let second = export().save_in(&root).unwrap();
        assert_ne!(first, second);
        assert!(std::fs::read_to_string(first.join("raw.jsonl")).unwrap().contains("DEADBEEF"));
        assert!(!std::fs::read_to_string(first.join("summary.txt")).unwrap().contains("DEADBEEF"));
        for folder in [first, second] {
            for name in ["raw.jsonl", "summary.txt"] { std::fs::remove_file(folder.join(name)).unwrap(); }
            std::fs::remove_dir(folder).unwrap();
        }
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn record_count_is_bounded_and_invalid_steps_are_ignored() {
        let now = Instant::now();
        let mut session = Session::new(&interface("one"), now);
        session.mark(99, now);
        assert!(session.records.is_empty());
        for _iteration in 0..MAX_RECORDS + 10 { session.mark(0, now); }
        assert_eq!(session.records.len(), MAX_RECORDS);
        assert_eq!(session.steps.len(), MAX_RECORDS);
        assert_eq!(session.stopped, Some("size-limit"));
    }
}
