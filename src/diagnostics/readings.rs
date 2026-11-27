use crate::device::{Battery, DeviceReading, ReadingQuality};
use std::{collections::VecDeque, fmt::Write, sync::{Mutex, OnceLock}, time::{Duration, Instant}};

const EVENT_LIMIT: usize = 256;
const STATE_LIMIT: usize = 128;
const REPORT_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface { Published, Expired, Main, Taskbar }

impl Surface {
    fn name(self) -> &'static str {
        match self {
            Self::Published => "published", Self::Expired => "sample-expired",
            Self::Main => "main-input", Self::Taskbar => "taskbar-input",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct State {
    channel: Option<u64>,
    measured: Option<Instant>,
    lifetime: Option<Duration>,
    connected: bool,
    quality: ReadingQuality,
    battery: Option<Battery>,
    displayed: bool,
}

struct Event {
    device: u64,
    surface: Surface,
    state: State,
    at_ms: u128,
    at: Instant,
    sequence: u64,
}

#[derive(Default)]
struct Timeline {
    latest: VecDeque<(u64, Surface, State)>,
    events: VecDeque<Event>,
    sequence: u64,
}

impl Timeline {
    fn observe(&mut self, device: u64, surface: Surface, state: State, at: Instant, at_ms: u128) {
        if let Some(index) = self.latest.iter().position(|entry| entry.0 == device && entry.1 == surface) {
            if self.latest[index].2 == state { return; }
            self.latest.remove(index);
        }
        self.latest.push_back((device, surface, state.clone()));
        if self.latest.len() > STATE_LIMIT { self.latest.pop_front(); }
        self.sequence += 1;
        self.events.push_back(Event { device, surface, state, at_ms, at, sequence: self.sequence });
        if self.events.len() > EVENT_LIMIT { self.events.pop_front(); }
    }

    fn render(&self, device: u64) -> String {
        let mut lines = VecDeque::new();
        let mut bytes = 0;
        let mut omitted = 0;
        for event in self.events.iter().filter(|event| event.device == device) {
            let state = &event.state;
            let channel = state.channel.map(|value| format!("{value:016x}")).unwrap_or_else(|| "unknown".into());
            let sample = state.measured.map(|value| format!("{:016x}", super::anonymous_id(&format!("{value:?}"))))
                .unwrap_or_else(|| "unknown".into());
            let age = state.measured.map(|value| event.at.saturating_duration_since(value).as_millis().to_string())
                .unwrap_or_else(|| "unknown".into());
            let lifetime = state.lifetime.map(|value| value.as_millis().to_string()).unwrap_or_else(|| "unknown".into());
            let battery = state.battery.as_ref().and_then(Battery::display_percent)
                .map(|value| value.label(false)).unwrap_or_else(|| "unknown".into());
            let charging = charging(state.battery.as_ref());
            let line = format!("at_ms={} seq={} device={device:016x} channel={channel} {} sample={sample} age_ms={age} lifetime_ms={lifetime} connected={} quality={:?} last_percent={battery} charging={charging} displayed={}\n",
                event.at_ms, event.sequence, event.surface.name(), state.connected, state.quality, state.displayed);
            bytes += line.len();
            lines.push_back(line);
            while bytes > REPORT_BYTES - 256 {
                if let Some(line) = lines.pop_front() { bytes -= line.len(); omitted += 1; }
            }
        }
        let mut report = format!("Reading timeline (state changes; current run; UI entries describe input data, not successful drawing; newest {EVENT_LIMIT} events retained globally; omitted_from_report={omitted})\n");
        for line in lines { report.push_str(&line); }
        report
    }
}

static TIMELINE: OnceLock<Mutex<Timeline>> = OnceLock::new();

fn charging(battery: Option<&Battery>) -> &'static str {
    match battery.and_then(|battery| battery.charging) {
        Some(true) => "true", Some(false) => "false", None => "unknown",
    }
}

pub fn observe(surface: Surface, reading: &DeviceReading, measured: Option<Instant>,
    channel: Option<&str>, lifetime: Option<Duration>, displayed: bool, now: Instant) {
    let state = State {
        channel: channel.map(|value| super::anonymous_id(&value.to_ascii_lowercase())),
        measured, lifetime, connected: reading.connected, quality: reading.quality,
        battery: reading.battery.clone(), displayed,
    };
    TIMELINE.get_or_init(Default::default).lock().unwrap_or_else(|error| error.into_inner())
        .observe(super::anonymous_id(&reading.key), surface, state, now, super::unix_millis());
}

pub fn report(reading: &DeviceReading, measured: Option<Instant>, lifetime: Duration) -> String {
    let age = measured.map(|value| value.elapsed().as_millis().to_string()).unwrap_or_else(|| "unknown".into());
    let device = super::anonymous_id(&reading.key);
    let mut report = format!("Captured at (Unix ms): {}\nTemporary device: {device:016x}\nSample age (ms): {age}\nSample lifetime (ms): {}\n",
        super::unix_millis(), lifetime.as_millis());
    let _ = writeln!(report, "Last measured battery: {}", reading.battery.as_ref()
        .and_then(Battery::display_percent).map(|value| value.label(true)).unwrap_or_else(|| "unknown".into()));
    let _ = writeln!(report, "Last measured charging: {}", charging(reading.battery.as_ref()));
    report.push_str(&TIMELINE.get_or_init(Default::default).lock().unwrap_or_else(|error| error.into_inner()).render(device));
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(at: Instant) -> State {
        State { channel: Some(9), measured: Some(at), lifetime: Some(Duration::from_secs(8)),
            connected: true, quality: ReadingQuality::Live, battery: Some(Battery {
                percent: Some(75), estimated_percent: None, charging: Some(false), voltage_mv: None,
                approximate: false, level: None,
            }), displayed: true }
    }

    #[test]
    fn repeated_repaints_do_not_consume_the_timeline() {
        let now = Instant::now();
        let mut timeline = Timeline::default();
        for tick in 0..1000 {
            timeline.observe(1, Surface::Main, state(now), now + Duration::from_millis(tick), 1000 + u128::from(tick));
        }
        assert_eq!(timeline.events.len(), 1);
    }

    #[test]
    fn expiration_and_recovery_keep_measurement_age_and_surface_evidence() {
        let now = Instant::now();
        let mut timeline = Timeline::default();
        let sample = state(now);
        timeline.observe(1, Surface::Published, sample.clone(), now, 1000);
        timeline.observe(1, Surface::Taskbar, sample.clone(), now, 1001);
        let mut expired = sample;
        expired.quality = ReadingQuality::Stale;
        expired.displayed = false;
        timeline.observe(1, Surface::Expired, expired.clone(), now + Duration::from_secs(8), 9000);
        timeline.observe(1, Surface::Main, expired, now + Duration::from_secs(9), 10000);
        let recovered = state(now + Duration::from_secs(20));
        timeline.observe(1, Surface::Published, recovered.clone(), now + Duration::from_secs(20), 21000);
        timeline.observe(2, Surface::Published, recovered, now + Duration::from_secs(20), 21000);
        let report = timeline.render(1);
        assert!(report.contains("sample-expired"));
        assert!(report.contains("age_ms=8000 lifetime_ms=8000 connected=true quality=Stale last_percent=75 charging=false displayed=false"));
        assert!(report.contains("main-input"));
        assert!(report.contains("taskbar-input"));
        assert!(report.contains("at_ms=21000"));
        assert!(!report.contains("device=0000000000000002"));
    }

    #[test]
    fn charging_transitions_are_visible_even_when_the_percentage_does_not_change() {
        let now = Instant::now();
        let mut timeline = Timeline::default();
        let mut sample = state(now);
        for charging in [Some(false), Some(true), None] {
            sample.battery.as_mut().unwrap().charging = charging;
            timeline.observe(1, Surface::Published, sample.clone(), now, 1000);
        }
        sample.battery = None;
        timeline.observe(1, Surface::Published, sample, now, 1000);
        assert_eq!(timeline.events.len(), 4);
        let report = timeline.render(1);
        assert!(report.contains("last_percent=75 charging=false"));
        assert!(report.contains("last_percent=75 charging=true"));
        assert!(report.contains("last_percent=75 charging=unknown"));
        assert!(report.contains("last_percent=unknown charging=unknown"));
    }

    #[test]
    fn storage_and_report_are_bounded_without_exporting_device_keys() {
        let now = Instant::now();
        let mut timeline = Timeline::default();
        for index in 0..1000 {
            timeline.observe(index, Surface::Published, state(now), now, 1000);
        }
        assert_eq!(timeline.events.len(), EVENT_LIMIT);
        assert_eq!(timeline.latest.len(), STATE_LIMIT);
        for index in 0..1000 {
            timeline.observe(1, Surface::Published, state(now + Duration::from_millis(index)), now, 1000);
        }
        let report = timeline.render(1);
        assert!(report.len() <= REPORT_BYTES);
        assert!(!report.contains("omitted_from_report=0)"));
    }
}
