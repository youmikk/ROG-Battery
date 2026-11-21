//! 通信摘要只保留协议标识、长度、耗时和结果，不存报文内容或设备身份。
use super::BackendId;
use std::{
    collections::VecDeque,
    fmt::Write,
    sync::{Mutex, OnceLock},
    time::Instant,
};

#[derive(Clone, Copy, Debug)]
pub enum Outcome {
    Ok,
    NoReply,
    Rejected(u32),
    InvalidData,
}

impl Outcome {
    fn text(self) -> String {
        match self {
            Self::Ok => "ok".into(),
            Self::NoReply => "no-reply".into(),
            Self::Rejected(code) => format!("rejected-{code:#x}"),
            Self::InvalidData => "invalid-data".into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Event {
    pub backend: BackendId,
    pub vendor_id: u16,
    pub product_id: u16,
    pub operation: &'static str,
    pub feature: u16,
    pub function: u8,
    pub request_bytes: usize,
    pub reply_bytes: usize,
    pub elapsed_ms: u128,
    pub outcome: Outcome,
}

const LIMIT: usize = 128;
static EVENTS: OnceLock<Mutex<VecDeque<Event>>> = OnceLock::new();

pub fn record(
    interface: &crate::hid::HidInterface,
    backend: BackendId,
    operation: &'static str,
    feature: u16,
    function: u8,
    request_bytes: usize,
    reply_bytes: usize,
    started: Instant,
    outcome: Outcome,
) {
    let mut events = EVENTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if events.len() >= LIMIT {
        events.pop_front();
    }
    events.push_back(Event {
        backend,
        vendor_id: interface.vendor_id,
        product_id: interface.product_id,
        operation,
        feature,
        function,
        request_bytes,
        reply_bytes,
        elapsed_ms: started.elapsed().as_millis(),
        outcome,
    });
}

pub fn render() -> String {
    let events = EVENTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    render_events(events.iter())
}

fn render_events<'a>(events: impl IntoIterator<Item = &'a Event>) -> String {
    let mut output = String::from("Protocol query summary (no payloads or device identifiers)\n");
    for event in events {
        let request = if event.request_bytes == 0 {
            "-".into()
        } else {
            event.request_bytes.to_string()
        };
        let _ = writeln!(
            output,
            "{} {:04X}:{:04X} {} feature={:04X} function={:02X} bytes={}/{} elapsed={}ms result={}",
            event.backend.name(),
            event.vendor_id,
            event.product_id,
            event.operation,
            event.feature,
            event.function,
            request,
            event.reply_bytes,
            event.elapsed_ms,
            event.outcome.text()
        );
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_summary_contains_only_allowed_fields() {
        let event = Event {
            backend: BackendId::Logitech,
            vendor_id: 0x046d,
            product_id: 0xc52b,
            operation: "query-feature",
            feature: 0x1004,
            function: 1,
            request_bytes: 20,
            reply_bytes: 20,
            elapsed_ms: 12,
            outcome: Outcome::Rejected(2),
        };
        let report = render_events([&event]);
        assert!(report.contains("feature=1004 function=01 bytes=20/20"));
        assert!(!report.contains("serial="));
        assert!(!report.contains("payload="));
    }
}
