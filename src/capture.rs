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
    data_frames: usize,
    steps: Vec<(u128, usize)>,
    stopped: Option<Stop>,
}

struct Stop {
    reason: &'static str,
    elapsed: Duration,
}

#[derive(Default)]
pub struct Status {
    pub started: bool,
    pub running: bool,
    pub frames: usize,
    pub data_frames: usize,
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
                privacy: "Raw data may contain device identifiers or input information. Included captures are shared publicly with feedback.",
            },
            records: Vec::new(), bytes: 0, frames: 0, data_frames: 0, steps: Vec::new(), stopped: None,
        }
    }

    fn expire(&mut self, now: Instant) {
        if self.stopped.is_none() && now.duration_since(self.started) >= Duration::from_secs(MAX_SECONDS) {
            self.stop("time-limit", now);
        }
    }

    fn stop(&mut self, reason: &'static str, now: Instant) {
        if self.stopped.is_some() { return; }
        let elapsed = now.duration_since(self.started);
        let limit = Duration::from_secs(MAX_SECONDS);
        self.stopped = Some(if elapsed >= limit {
            Stop { reason: "time-limit", elapsed: limit }
        } else {
            Stop { reason, elapsed }
        });
    }

    fn append(&mut self, value: serde_json::Value, now: Instant) -> bool {
        self.expire(now);
        if self.stopped.is_some() { return false; }
        let line = value.to_string();
        if self.records.len() >= MAX_RECORDS || self.bytes + line.len() + 1 > MAX_BYTES {
            self.stop("size-limit", now);
            return false;
        }
        self.bytes += line.len() + 1;
        self.records.push(line);
        if self.records.len() == MAX_RECORDS || self.bytes == MAX_BYTES { self.stop("size-limit", now); }
        true
    }

    fn frame(&mut self, path: &str, api: &str, data: &[u8], success: bool, now: Instant) {
        if !self.path.eq_ignore_ascii_case(path) || data.len() > 4096 { return; }
        let mut hex = String::with_capacity(data.len() * 2);
        for byte in data { let _ = write!(hex, "{byte:02X}"); }
        if self.append(serde_json::json!({"kind":"transfer", "elapsed_ms":now.duration_since(self.started).as_millis(),
            "api":api, "success":success, "hex":hex}), now) {
            self.frames += 1;
            if !data.is_empty() { self.data_frames += 1; }
        }
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
            self.metadata.usage, self.frames, self.stopped.as_ref().map_or("running", |stop| stop.reason));
        for (elapsed, step) in &self.steps {
            let _ = writeln!(result, "{elapsed} ms: {}", STEPS[*step].1);
        }
        result
    }

    fn status(&mut self, now: Instant) -> Status {
        self.expire(now);
        let elapsed = self.stopped.as_ref().map_or_else(|| now.duration_since(self.started), |stop| stop.elapsed);
        Status { started: true, running: self.stopped.is_none(), frames: self.frames, data_frames: self.data_frames,
            elapsed_seconds: elapsed.as_secs(),
            preview: self.records.iter().rev().take(6).rev().map(|line| {
                let mut preview: String = line.chars().take(180).collect();
                if line.len() > preview.len() { preview.push_str("…"); }
                preview
            }).collect(), summary: self.summary() }
    }
}

fn start_session(current: &mut Option<Session>, interface: &HidInterface, now: Instant) -> Result<(), &'static str> {
    if !eligible(interface) { return Err("此接口不支持原始采集 / Capture unavailable for this interface"); }
    if current.as_ref().is_some_and(|session| session.stopped.is_none() || session.data_frames != 0
        || !session.path.eq_ignore_ascii_case(&interface.path)) {
        return Err("请先保存或清除上次采集 / Save or clear the previous capture first");
    }
    let mut session = Session::new(interface, now);
    session.mark(0, now);
    *current = Some(session);
    Ok(())
}

pub fn start(interface: &HidInterface) -> Result<(), &'static str> {
    let mut guard = cell().lock().unwrap_or_else(|error| error.into_inner());
    start_session(&mut guard, interface, Instant::now())?;
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
    if let Some(session) = guard.as_mut() { session.stop("stopped", Instant::now()); }
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
    let status = session.status(Instant::now());
    if !status.running { ACTIVE.store(false, Ordering::Release); }
    status
}

pub struct Export { raw: String, summary: String, timestamp: u128 }

pub fn export() -> Option<Export> {
    stop();
    let guard = cell().lock().unwrap_or_else(|error| error.into_inner());
    let session = guard.as_ref()?;
    export_session(session)
}

pub fn feedback_data(path: &str) -> Option<String> {
    let guard = cell().lock().unwrap_or_else(|error| error.into_inner());
    let session = guard.as_ref()?;
    if session.stopped.is_none() || session.data_frames == 0 || !session.path.eq_ignore_ascii_case(path) {
        return None;
    }
    Some(export_session(session)?.raw)
}

fn export_session(session: &Session) -> Option<Export> {
    let mut raw = serde_json::to_string(&session.metadata).ok()?;
    raw.push('\n');
    for record in &session.records { raw.push_str(record); raw.push('\n'); }
    raw.push_str(&serde_json::json!({"kind":"end", "reason":session.stopped.as_ref().map(|stop| stop.reason),
        "transfers":session.frames}).to_string());
    raw.push('\n');
    Some(Export { raw, summary: session.summary(), timestamp: session.metadata.started_unix_ms })
}

impl Export {
    pub fn from_raw(raw: String) -> Option<Self> {
        let metadata: serde_json::Value = serde_json::from_str(raw.lines().next()?).ok()?;
        let timestamp = u128::from(metadata.get("started_unix_ms")?.as_u64()?);
        let vendor = metadata.get("vendor_id")?.as_u64()?;
        let product = metadata.get("product_id")?.as_u64()?;
        let summary = format!("Feedback capture: {vendor:04X}:{product:04X}\nFile: raw.jsonl\n");
        Some(Self { raw, summary, timestamp })
    }

    pub fn save(self) -> Result<PathBuf, String> {
        self.save_in(&crate::paths::cache_dir().join("captures"))
    }

    fn save_in(self, root: &Path) -> Result<PathBuf, String> {
        self.save_with(root, |file, bytes| file.write_all(bytes).and_then(|_| file.sync_all()))
    }

    fn save_with(self, root: &Path,
        mut write: impl FnMut(&mut std::fs::File, &[u8]) -> std::io::Result<()>) -> Result<PathBuf, String> {
        std::fs::create_dir_all(root).map_err(|error| error.to_string())?;
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = root.join(format!("capture-{}-{}-{}", self.timestamp, std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).map_err(|error| error.to_string())?;
        let mut created = Vec::new();
        let result = (|| -> std::io::Result<()> {
            for (name, contents) in [("raw.jsonl", self.raw), ("summary.txt", self.summary)] {
                let target = path.join(name);
                let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&target)?;
                created.push(target);
                write(&mut file, contents.as_bytes())?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            for file in created { let _ = std::fs::remove_file(file); }
            return if std::fs::remove_dir(&path).is_ok() {
                Err(format!("保存失败，已清除未完成文件 / Export failed; partial files removed: {error}"))
            } else {
                Err(format!("保存失败，可能残留原始数据，请检查 / Export failed; check remaining raw data in {}: {error}", path.display()))
            };
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
    fn feedback_requires_data_a_stopped_session_and_the_same_interface() {
        clear();
        let source = interface("capture-feedback-a");
        start(&source).unwrap();
        record(&source.path, "input-timeout", &[], false);
        stop();
        assert!(feedback_data(&source.path).is_none());
        assert_eq!(status().data_frames, 0);
        start(&source).unwrap();
        record(&source.path, "input", &[0, 0xab, 0xff], true);
        assert_eq!(status().data_frames, 1);
        assert!(feedback_data(&source.path).is_none());
        stop();
        assert!(feedback_data("capture-feedback-b").is_none());
        let raw = feedback_data("CAPTURE-FEEDBACK-A").unwrap();
        assert!(raw.contains("00ABFF"));
        assert!(crate::feedback::client::capture(raw.as_bytes()).is_ok());
        assert_eq!(Export::from_raw(raw.clone()).unwrap().raw, raw);
        clear();
        assert!(feedback_data(&source.path).is_none());
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
    fn stopped_elapsed_is_frozen_and_repeated_stop_preserves_the_first_stop() {
        let now = Instant::now();
        let mut session = Session::new(&interface("one"), now);
        session.frame("one", "input-timeout", &[], false, now + Duration::from_secs(1));
        session.frame("one", "input", &[1], true, now + Duration::from_secs(2));
        let running = session.status(now + Duration::from_secs(3));
        assert!(running.running);
        assert_eq!(running.elapsed_seconds, 3);
        assert_eq!(running.frames, 2);
        assert_eq!(running.data_frames, 1);
        let elapsed = Duration::from_millis(7500);
        session.stop("stopped", now + elapsed);
        let raw = export_session(&session).unwrap().raw;
        let end: serde_json::Value = serde_json::from_str(raw.lines().last().unwrap()).unwrap();
        assert_eq!(end, serde_json::json!({"kind":"end", "reason":"stopped", "transfers":2}));
        for seconds in [8, MAX_SECONDS, MAX_SECONDS + 30] {
            let later = now + Duration::from_secs(seconds);
            session.stop("stopped", later);
            session.mark(1, later);
            session.frame("one", "input", &[2], true, later);
            let status = session.status(later);
            assert!(status.started);
            assert!(!status.running);
            assert_eq!(status.elapsed_seconds, 7);
            assert_eq!(status.frames, 2);
            assert_eq!(status.data_frames, 1);
            let stopped = session.stopped.as_ref().unwrap();
            assert_eq!(stopped.reason, "stopped");
            assert_eq!(stopped.elapsed, elapsed);
            assert_eq!(export_session(&session).unwrap().raw, raw);
        }
    }

    #[test]
    fn time_limit_freezes_at_the_deadline_even_when_observed_late() {
        let now = Instant::now();
        let limit = Duration::from_secs(MAX_SECONDS);
        for delay in [Duration::ZERO, Duration::from_secs(37)] {
            let mut session = Session::new(&interface("one"), now);
            let before = session.status(now + limit - Duration::from_millis(1));
            assert!(before.running);
            assert_eq!(before.elapsed_seconds, MAX_SECONDS - 1);
            let stopped = session.status(now + limit + delay);
            assert!(!stopped.running);
            assert_eq!(stopped.elapsed_seconds, MAX_SECONDS);
            session.stop("stopped", now + limit + Duration::from_secs(60));
            let stop = session.stopped.as_ref().unwrap();
            assert_eq!(stop.reason, "time-limit");
            assert_eq!(stop.elapsed, limit);
            assert_eq!(session.status(now + limit + Duration::from_secs(90)).elapsed_seconds, MAX_SECONDS);
        }
    }

    #[test]
    fn explicit_stop_at_or_after_the_deadline_uses_the_time_limit() {
        let now = Instant::now();
        for seconds in [MAX_SECONDS, MAX_SECONDS + 37] {
            let mut session = Session::new(&interface("one"), now);
            session.stop("stopped", now + Duration::from_secs(seconds));
            let stop = session.stopped.as_ref().unwrap();
            assert_eq!(stop.reason, "time-limit");
            assert_eq!(stop.elapsed, Duration::from_secs(MAX_SECONDS));
            assert_eq!(session.status(now + Duration::from_secs(seconds + 1)).elapsed_seconds, MAX_SECONDS);
        }
    }

    #[test]
    fn stopped_empty_and_timeout_only_sessions_can_restart_on_the_same_path() {
        let now = Instant::now();
        for timeout_count in [0, 2] {
            let mut current = None;
            start_session(&mut current, &interface("receiver-a"), now).unwrap();
            let session = current.as_mut().unwrap();
            for _iteration in 0..timeout_count {
                session.frame("receiver-a", "input-timeout", &[], false, now + Duration::from_secs(1));
            }
            session.mark(1, now + Duration::from_secs(2));
            session.stop("stopped", now + Duration::from_secs(3));
            let stopped = session.status(now + Duration::from_secs(4));
            assert!(!stopped.running);
            assert_eq!(stopped.frames, timeout_count);
            assert_eq!(stopped.data_frames, 0);
            let restarted = now + Duration::from_secs(10);
            start_session(&mut current, &interface("RECEIVER-A"), restarted).unwrap();
            let session = current.as_mut().unwrap();
            let status = session.status(restarted);
            assert!(status.started);
            assert!(status.running);
            assert_eq!(status.frames, 0);
            assert_eq!(status.data_frames, 0);
            assert_eq!(status.elapsed_seconds, 0);
            assert_eq!(session.started, restarted);
            assert_eq!(session.path, "receiver-a");
            assert_eq!(session.steps, vec![(0, 0)]);
            assert_eq!(session.records.len(), 1);
            assert_eq!(session.bytes, session.records[0].len() + 1);
        }
    }

    #[test]
    fn running_empty_and_timeout_only_sessions_reject_restart_without_losing_records() {
        let now = Instant::now();
        for timeout_count in [0, 2] {
            let mut current = None;
            start_session(&mut current, &interface("receiver-a"), now).unwrap();
            let session = current.as_mut().unwrap();
            for _iteration in 0..timeout_count {
                session.frame("receiver-a", "input-timeout", &[], false, now + Duration::from_secs(1));
            }
            let raw = export_session(session).unwrap().raw;
            for path in ["RECEIVER-A", "receiver-b"] {
                assert!(start_session(&mut current, &interface(path), now + Duration::from_secs(2)).is_err());
                let session = current.as_ref().unwrap();
                assert_eq!(session.path, "receiver-a");
                assert_eq!(session.started, now);
                assert!(session.stopped.is_none());
                assert_eq!(session.frames, timeout_count);
                assert_eq!(export_session(session).unwrap().raw, raw);
            }
        }
    }

    #[test]
    fn stopped_nonempty_sessions_reject_restart_without_losing_records() {
        let now = Instant::now();
        for success in [false, true] {
            let mut current = None;
            start_session(&mut current, &interface("receiver-a"), now).unwrap();
            let session = current.as_mut().unwrap();
            session.frame("receiver-a", "input", &[0, 0xab, 0xff], success, now + Duration::from_secs(1));
            session.stop("stopped", now + Duration::from_secs(2));
            let raw = export_session(session).unwrap().raw;
            for path in ["RECEIVER-A", "receiver-b"] {
                assert!(start_session(&mut current, &interface(path), now + Duration::from_secs(3)).is_err());
                let session = current.as_mut().unwrap();
                assert_eq!(session.path, "receiver-a");
                assert_eq!(session.started, now);
                let status = session.status(now + Duration::from_secs(4));
                assert!(!status.running);
                assert_eq!(status.frames, 1);
                assert_eq!(status.data_frames, 1);
                assert_eq!(status.elapsed_seconds, 2);
                assert_eq!(export_session(session).unwrap().raw, raw);
            }
        }
    }

    #[test]
    fn stopped_empty_and_timeout_only_sessions_cannot_be_replaced_by_another_device() {
        let now = Instant::now();
        for timeout_count in [0, 2] {
            let mut current = None;
            start_session(&mut current, &interface("receiver-a"), now).unwrap();
            let session = current.as_mut().unwrap();
            for _iteration in 0..timeout_count {
                session.frame("receiver-a", "input-timeout", &[], false, now + Duration::from_secs(1));
            }
            session.stop("stopped", now + Duration::from_secs(2));
            let raw = export_session(session).unwrap().raw;
            assert!(start_session(&mut current, &interface("receiver-b"), now + Duration::from_secs(3)).is_err());
            let session = current.as_mut().unwrap();
            assert_eq!(session.path, "receiver-a");
            assert_eq!(session.started, now);
            let status = session.status(now + Duration::from_secs(4));
            assert!(!status.running);
            assert_eq!(status.frames, timeout_count);
            assert_eq!(status.data_frames, 0);
            assert_eq!(status.elapsed_seconds, 2);
            assert_eq!(export_session(session).unwrap().raw, raw);
        }
    }

    #[test]
    fn bounded_capture_expires_and_does_not_refresh_on_more_frames() {
        let now = Instant::now();
        let mut session = Session::new(&interface("one"), now);
        session.mark(2, now + Duration::from_secs(1));
        session.frame("one", "input", &[1], true, now + Duration::from_secs(MAX_SECONDS));
        assert_eq!(session.stopped.as_ref().unwrap().reason, "time-limit");
        assert_eq!(session.frames, 0);
        assert_eq!(session.steps, vec![(1000, 2)]);
        session.mark(0, now + Duration::from_secs(MAX_SECONDS + 1));
        assert_eq!(session.steps.len(), 1);
        assert_eq!(session.status(now + Duration::from_secs(MAX_SECONDS + 2)).elapsed_seconds, MAX_SECONDS);
    }

    #[test]
    fn byte_budget_accounts_for_hex_and_json_overhead() {
        let now = Instant::now();
        let mut session = Session::new(&interface("one"), now);
        for _iteration in 0..1000 { session.frame("one", "input", &[255;4096], true, now + Duration::from_secs(2)); }
        session.stop("stopped", now + Duration::from_secs(MAX_SECONDS + 1));
        assert_eq!(session.stopped.as_ref().unwrap().reason, "size-limit");
        assert_eq!(session.status(now + Duration::from_secs(MAX_SECONDS + 2)).elapsed_seconds, 2);
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
    fn failed_export_cleans_only_its_new_bundle_and_keeps_existing_files() {
        let root = std::env::temp_dir().join(format!("x-battery-capture-failure-{}-{}",
            std::process::id(), crate::diagnostics::unix_millis()));
        std::fs::create_dir(&root).unwrap();
        let existing = root.join("keep.txt");
        std::fs::write(&existing, b"original").unwrap();
        let mut writes = 0;
        let result = Export { raw: "sensitive raw bytes".into(), summary: "summary".into(), timestamp: 2 }
            .save_with(&root, |file, bytes| {
                file.write_all(bytes)?;
                writes += 1;
                if writes == 2 { Err(std::io::Error::other("simulated disk failure")) } else { Ok(()) }
            });
        assert!(result.unwrap_err().contains("partial files removed"));
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        assert_eq!(std::fs::read(&existing).unwrap(), b"original");
        std::fs::remove_file(existing).unwrap();
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
        assert_eq!(session.stopped.as_ref().unwrap().reason, "size-limit");
    }
}
