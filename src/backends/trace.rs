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
    Offline,
}

impl Outcome {
    fn text(self) -> String {
        match self {
            Self::Ok => "ok".into(),
            Self::NoReply => "no-reply".into(),
            Self::Rejected(code) => format!("rejected-{code:#x}"),
            Self::InvalidData => "invalid-data".into(),
            Self::Offline => "offline".into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Event {
    pub channel: u64,
    pub address: Option<u8>,
    pub recorded_ms: u128,
    pub sequence: u64,
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
    pub sample_age_ms: Option<u128>,
}

const LIMIT: usize = 128;
static EVENTS: OnceLock<Mutex<VecDeque<Event>>> = OnceLock::new();
static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
    record_sample(interface, backend, operation, feature, function, request_bytes,
        reply_bytes, started, outcome, None);
}

pub fn record_sample(
    interface: &crate::hid::HidInterface, backend: BackendId, operation: &'static str,
    feature: u16, function: u8, request_bytes: usize, reply_bytes: usize,
    started: Instant, outcome: Outcome, measured: Option<Instant>,
) {
    record_scoped(interface, backend, None, operation, feature, function, request_bytes,
        reply_bytes, started, outcome, measured);
}

pub fn record_address(
    interface: &crate::hid::HidInterface, backend: BackendId, address: u8,
    operation: &'static str, feature: u16, function: u8, request_bytes: usize,
    reply_bytes: usize, started: Instant, outcome: Outcome,
) {
    record_scoped(interface, backend, Some(address), operation, feature, function,
        request_bytes, reply_bytes, started, outcome, None);
}

fn record_scoped(
    interface: &crate::hid::HidInterface, backend: BackendId, address: Option<u8>,
    operation: &'static str, feature: u16, function: u8, request_bytes: usize,
    reply_bytes: usize, started: Instant, outcome: Outcome, measured: Option<Instant>,
) {
    let mut events = EVENTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if events.len() >= LIMIT {
        events.pop_front();
    }
    events.push_back(Event {
        channel: crate::diagnostics::anonymous_id(&interface.path.to_ascii_lowercase()),
        address,
        recorded_ms: crate::diagnostics::unix_millis(),
        sequence: SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1,
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
        sample_age_ms: measured.map(|measured| measured.elapsed().as_millis()),
    });
}

pub fn render() -> String {
    let events = EVENTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    render_events(events.iter())
}

pub fn render_selected(backend: BackendId, interface: &crate::hid::HidInterface,
    address: Option<u8>) -> String {
    let events = EVENTS.get_or_init(Default::default).lock().unwrap_or_else(|error| error.into_inner());
    let channel = crate::diagnostics::anonymous_id(&interface.path.to_ascii_lowercase());
    render_channel(&events, backend, channel, address)
}

fn render_channel(events: &VecDeque<Event>, backend: BackendId, channel: u64,
    address: Option<u8>) -> String {
    let selection = match address {
        Some(address) => format!("Selected scope: channel={channel:016x} address=0x{address:02x}; includes interface-scoped events\n"),
        None => format!("Selected scope: channel={channel:016x} interface-only; excludes all address-scoped events\n"),
    };
    let mut selected: Vec<_> = events.iter().rev().filter(|event|
        event.backend == backend && event.channel == channel
            && (event.address.is_none() || event.address == address)).take(24).collect();
    selected.reverse();
    while render_events(selected.iter().copied()).len() + selection.len() > 4096 {
        selected.remove(0);
    }
    let mut output = render_events(selected);
    output.push_str(&selection);
    output
}

fn render_events<'a>(events: impl IntoIterator<Item = &'a Event>) -> String {
    let mut output = String::from("Protocol query summary (bounded recent events; channel IDs apply only to this run; no payloads)\n");
    for event in events {
        let request = if event.request_bytes == 0 {
            "-".into()
        } else {
            event.request_bytes.to_string()
        };
        let scope = match event.address {
            Some(address) => format!("scope=address address=0x{address:02x}"),
            None => "scope=interface".into(),
        };
        let _ = writeln!(
            output,
            "at_ms={} seq={} channel={:016x} {} {} {:04X}:{:04X} {} feature={:04X} function={:02X} bytes={}/{} elapsed={}ms result={}{}",
            event.recorded_ms,
            event.sequence,
            event.channel,
            scope,
            event.backend.name(),
            event.vendor_id,
            event.product_id,
            event.operation,
            event.feature,
            event.function,
            request,
            event.reply_bytes,
            event.elapsed_ms,
            event.outcome.text(),
            event.sample_age_ms.map(|age| format!(" sample_age_ms={age}")).unwrap_or_default()
        );
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(sequence: u64, address: Option<u8>) -> Event {
        Event {
            channel: 123,
            address,
            recorded_ms: 1_700_000_000_000,
            sequence,
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
            sample_age_ms: Some(3000),
        }
    }

    #[test]
    fn protocol_summary_contains_only_allowed_fields() {
        let event = event(1, None);
        let report = render_events([&event]);
        assert!(report.contains("feature=1004 function=01 bytes=20/20"));
        assert!(!report.contains("serial="));
        assert!(!report.contains("payload="));
        assert!(report.contains("at_ms=1700000000000 seq=1 channel=000000000000007b"));
        assert!(report.contains("sample_age_ms=3000"));
        assert!(report.contains("scope=interface"));
        let mut other = event.clone();
        other.channel = 456;
        other.sequence = 2;
        let filtered = render_channel(&[event, other].into(), BackendId::Logitech, 123, None);
        assert!(!filtered.contains("seq=2"));
    }

    #[test]
    fn selected_address_includes_interface_events_but_excludes_other_devices() {
        let mut other_channel = event(5, Some(1));
        other_channel.channel = 456;
        let mut other_backend = event(6, Some(1));
        other_backend.backend = BackendId::Rapoo;
        let events = [event(1, Some(1)), event(2, Some(2)), event(3, Some(0xff)),
            event(4, None), other_channel, other_backend].into();
        let report = render_channel(&events, BackendId::Logitech, 123, Some(1));
        for sequence in [1, 4] {
            assert!(report.contains(&format!("seq={sequence} ")));
        }
        for sequence in [2, 3, 5, 6] {
            assert!(!report.contains(&format!("seq={sequence} ")));
        }
        assert!(report.contains("scope=address address=0x01"));
        assert!(report.contains("scope=interface"));
    }

    #[test]
    fn interface_selection_is_not_an_address_wildcard() {
        let events = [event(1, Some(0)), event(2, Some(1)), event(3, Some(0xff)),
            event(4, None)].into();
        let report = render_channel(&events, BackendId::Logitech, 123, None);
        assert!(report.contains("seq=4 "));
        assert!(!report.contains("scope=address"));
        assert!(report.contains("interface-only; excludes all address-scoped events"));
    }

    #[test]
    fn receiver_register_queries_keep_the_actual_receiver_address() {
        let mut receiver = event(1, Some(0xff));
        receiver.operation = "read-register";
        receiver.feature = 0x2b5;
        let mut device = event(2, Some(2));
        device.operation = "read-register";
        device.feature = 0x0d;
        let events = [receiver, device, event(3, None)].into();
        let receiver_report = render_channel(&events, BackendId::Logitech, 123, Some(0xff));
        assert!(receiver_report.contains("seq=1 "));
        assert!(receiver_report.contains("scope=address address=0xff"));
        assert!(receiver_report.contains("read-register feature=02B5"));
        assert!(!receiver_report.contains("seq=2 "));
        assert!(receiver_report.contains("seq=3 "));
        let device_report = render_channel(&events, BackendId::Logitech, 123, Some(2));
        assert!(!device_report.contains("seq=1 "));
        assert!(device_report.contains("seq=2 "));
        assert!(device_report.contains("seq=3 "));
    }

    #[test]
    fn selected_report_keeps_newest_matching_events_within_byte_budget() {
        let events = (1..=100).map(|sequence|
            event(sequence, Some(if sequence % 2 == 0 { 1 } else { 2 }))).collect();
        let report = render_channel(&events, BackendId::Logitech, 123, Some(1));
        assert!(report.len() <= 4096);
        assert!(report.contains("seq=100 "));
        assert!(!report.contains("seq=99 "));
        let rows: Vec<_> = report.lines().filter(|line| line.starts_with("at_ms=")).collect();
        assert!(!rows.is_empty() && rows.len() <= 24);
        assert!(rows.iter().all(|line| line.contains("scope=address address=0x01")));
    }
}
