//! Per-device measurement tracking. Disk IO is performed by the history worker.
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::device::{DeviceReading, ReadingQuality};
use super::{HistoryRow, Estimate};

#[derive(Clone, Default)]
pub struct HistoryView {
    pub plot: Option<Arc<crate::chart::Plot>>,
    pub estimate: Estimate,
    pub rows: usize,
}

#[derive(Clone, Default)]
struct Stream {
    measured: Option<Instant>,
    current: Option<HistoryRow>,
    queued: Option<HistoryRow>,
    interrupted: bool,
    available: bool,
    segment_start: Option<i64>,
}

#[derive(Clone, Default)]
pub struct Tracker {
    streams: BTreeMap<String, Stream>,
    pending: Vec<(String, HistoryRow)>,
    aliases: Vec<(String, String)>,
    reset_at: Option<Instant>,
}

impl Tracker {
    pub fn interrupt(&mut self, key: &str) {
        let stream = self.streams.entry(key.into()).or_default();
        stream.interrupted = true;
        stream.available = false;
    }

    pub fn observe(&mut self, reading: &DeviceReading, measured: Option<Instant>, mode: i32,
        now: Instant, unix: i64, lifetime: Duration) {
        if measured.is_some_and(|at| self.reset_at.is_some_and(|reset| at <= reset)) { return; }
        let stream = self.streams.entry(reading.key.clone()).or_default();
        let valid = reading.connected && matches!(reading.quality, ReadingQuality::Live | ReadingQuality::Cached)
            && measured.is_some_and(|at| at <= now && now.duration_since(at) < lifetime)
            && reading.display_percent().is_some();
        stream.available = valid;
        if !valid {
            stream.interrupted = true;
            return;
        }
        // A cached publication, including a passive sample still labelled Live,
        // must not move the measurement timestamp or add a point to the curve.
        if reading.quality != ReadingQuality::Live { return; }
        let at = measured.unwrap();
        if stream.measured.is_some_and(|previous| at <= previous) { return; }
        let percent = reading.display_percent().unwrap();
        let battery = reading.battery.as_ref().unwrap();
        let mut row = HistoryRow::new(unix - now.duration_since(at).as_secs() as i64,
            percent.value, battery.charging == Some(true), mode);
        row.approximate = percent.approximate;
        row.charging_known = battery.charging.is_some();
        row.break_before = stream.interrupted;
        if row.break_before || stream.current.is_some_and(|old| old.mode != row.mode
            || old.charging != row.charging || old.charging_known != row.charging_known
            || old.approximate != row.approximate) {
            stream.segment_start = Some(row.ts);
        }
        stream.interrupted = false;
        stream.measured = Some(at);
        stream.current = Some(row);
        let due = super::should_record_device(stream.queued.as_slice(), row);
        if due {
            stream.queued = Some(row);
            self.pending.push((reading.key.clone(), row));
        }
    }

    pub fn promote(&mut self, old: &str, new: &str) {
        if old == new { return; }
        if let Some(stream) = self.streams.remove(old) {
            let target = self.streams.entry(new.into()).or_default();
            if target.measured < stream.measured { *target = stream; }
        }
        for (key, _) in &mut self.pending { if key == old { *key = new.into(); } }
        for (_, target) in &mut self.aliases { if target == old { *target = new.into(); } }
        self.aliases.push((old.into(), new.into()));
    }

    pub fn take_pending(&mut self) -> (Vec<(String, HistoryRow)>, Vec<(String, String)>) {
        (std::mem::take(&mut self.pending), std::mem::take(&mut self.aliases))
    }

    pub fn retry(&mut self, mut samples: Vec<(String, HistoryRow)>, mut aliases: Vec<(String, String)>) {
        samples.append(&mut self.pending);
        aliases.append(&mut self.aliases);
        aliases.sort();
        aliases.dedup();
        let cutoff = crate::snapshot::now_unix() as i64 - super::HIST_KEEP_HOURS * 3600;
        samples.retain(|(_, row)| row.ts >= cutoff);
        self.pending = samples;
        self.aliases = aliases;
    }

    pub fn reset(&mut self) {
        self.reset_at = Some(Instant::now());
        self.pending.clear();
        self.aliases.clear();
        for stream in self.streams.values_mut() {
            // Retain the measurement identity so cached data cannot repopulate
            // a history the user has just cleared.
            stream.current = None;
            stream.queued = None;
            stream.interrupted = true;
            stream.available = false;
            stream.segment_start = None;
        }
    }

    pub fn estimate(&self, key: &str, rows: &[HistoryRow], now: i64) -> Estimate {
        let Some(stream) = self.streams.get(key).filter(|stream| stream.available && !stream.interrupted) else {
            return Estimate { text: "等待设备更新电量".into(), ..Default::default() };
        };
        let Some(current) = stream.current else { return Estimate::default(); };
        let start = stream.segment_start.map_or(0, |start| rows.partition_point(|row| row.ts < start));
        let mut estimate = super::predict_device(&rows[start..], current);
        if estimate.known {
            let age = (now - current.ts).max(0);
            estimate.stale_seconds = if age >= 120 { age } else { 0 };
        }
        estimate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::Battery;
    const TS: i64 = 1_800_000_000;
    fn reading(key: &str, level: u8) -> DeviceReading {
        DeviceReading { key: key.into(), connected: true, battery: Some(Battery {
            percent: Some(level), estimated_percent: None, charging: Some(false),
            voltage_mv: None, approximate: false, level: None }), ..Default::default() }
    }
    fn observe(tracker: &mut Tracker, reading: &DeviceReading, at: Instant, seconds: u64, measured: Instant) {
        tracker.observe(reading, Some(measured), 0, at + Duration::from_secs(seconds),
            TS + seconds as i64, Duration::from_secs(180));
    }
    #[test]
    fn devices_sample_independently_and_republished_live_data_is_not_a_new_measurement() {
        let at = Instant::now();
        let mut tracker = Tracker::default();
        for key in ["mouse", "keyboard", "headset", "gamepad"] {
            observe(&mut tracker, &reading(key, 80), at, 0, at);
        }
        assert_eq!(tracker.take_pending().0.len(), 4);
        observe(&mut tracker, &reading("mouse", 80), at, 120, at);
        assert!(tracker.take_pending().0.is_empty());
        observe(&mut tracker, &reading("mouse", 79), at, 121, at + Duration::from_secs(121));
        assert_eq!(tracker.take_pending().0.len(), 1);
    }
    #[test]
    fn cached_charge_only_and_offline_reports_do_not_create_samples_and_reconnect_breaks() {
        let at = Instant::now(); let mut tracker = Tracker::default();
        let mut r = reading("a", 60);
        observe(&mut tracker, &r, at, 0, at); tracker.take_pending();
        r.quality = ReadingQuality::Cached;
        observe(&mut tracker, &r, at, 121, at + Duration::from_secs(121));
        assert!(tracker.take_pending().0.is_empty());
        r.quality = ReadingQuality::Live; r.battery.as_mut().unwrap().percent = None;
        observe(&mut tracker, &r, at, 122, at + Duration::from_secs(122));
        assert!(tracker.take_pending().0.is_empty());
        assert!(!tracker.estimate("a", &[], TS + 122).known);
        r = reading("a", 59);
        observe(&mut tracker, &r, at, 123, at + Duration::from_secs(123));
        assert!(tracker.take_pending().0[0].1.break_before);
    }
    #[test]
    fn clear_keeps_seen_measurements_and_identity_promotion_moves_pending_samples() {
        let at = Instant::now(); let mut tracker = Tracker::default();
        observe(&mut tracker, &reading("temp", 80), at, 0, at);
        tracker.promote("temp", "verified");
        let (samples, aliases) = tracker.take_pending();
        assert_eq!(samples[0].0, "verified");
        assert_eq!(aliases, vec![("temp".into(), "verified".into())]);
        tracker.reset();
        observe(&mut tracker, &reading("verified", 80), at, 120, at);
        assert!(tracker.take_pending().0.is_empty());
        observe(&mut tracker, &reading("verified", 79), at, 121, at + Duration::from_secs(121));
        assert!(tracker.take_pending().0[0].1.break_before);
    }

    #[test]
    fn clear_rejects_measurements_taken_before_reset_but_delivered_later() {
        let at = Instant::now();
        let mut tracker = Tracker::default();
        observe(&mut tracker, &reading("a", 80), at, 0, at);
        tracker.reset();
        let reset = at + Duration::from_secs(60);
        tracker.reset_at = Some(reset);
        observe(&mut tracker, &reading("a", 79), at, 61, at + Duration::from_secs(59));
        observe(&mut tracker, &reading("new-device", 60), at, 61, reset);
        assert!(tracker.take_pending().0.is_empty());
        observe(&mut tracker, &reading("a", 78), at, 62, at + Duration::from_secs(62));
        assert_eq!(tracker.take_pending().0.len(), 1);
    }

    #[test]
    fn interruption_is_not_lost_when_fresh_samples_arrive_before_disk_flush() {
        let at = Instant::now(); let mut tracker = Tracker::default();
        let rows = [HistoryRow::new(TS, 90, false, 0), HistoryRow::new(TS+1800, 85, false, 0),
            HistoryRow::new(TS+3600, 80, false, 0)];
        observe(&mut tracker, &reading("a", 80), at, 3600, at + Duration::from_secs(3600));
        assert!(tracker.estimate("a", &rows, TS+3600).known);
        tracker.interrupt("a");
        observe(&mut tracker, &reading("a", 79), at, 3601, at + Duration::from_secs(3601));
        observe(&mut tracker, &reading("a", 79), at, 3602, at + Duration::from_secs(3602));
        assert!(!tracker.estimate("a", &rows, TS+3602).known);
    }

    #[test]
    fn repeated_disk_failures_do_not_grow_duplicate_aliases() {
        let mut tracker = Tracker::default();
        for _ in 0..100 {
            tracker.retry(Vec::new(), vec![("a".into(), "b".into())]);
        }
        assert_eq!(tracker.take_pending().1, vec![("a".into(), "b".into())]);
    }
}
