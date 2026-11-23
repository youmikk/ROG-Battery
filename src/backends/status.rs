//! 连接证据和电量采样分别保存；查询超时不等于设备断开。
use crate::device::{Battery, ReadingQuality};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub battery: Option<Battery>,
    pub measured_at: Option<Instant>,
    pub queried_at: Option<Instant>,
    pub connection_known: bool,
    pub connected: bool,
    pub quality: ReadingQuality,
}

impl Status {
    pub fn clear_battery(&mut self) {
        self.battery = None;
        self.measured_at = None;
        self.quality = ReadingQuality::Unknown;
    }

    pub fn expire(&mut self, now: Instant, lifetime: Duration) -> bool {
        if self.measured_at.is_some_and(|measured| now.saturating_duration_since(measured) >= lifetime)
            && self.quality != ReadingQuality::Stale {
            self.quality = ReadingQuality::Stale;
            return true;
        }
        false
    }

    pub fn observe(&mut self, connected: Option<bool>, battery: Option<&Battery>, now: Instant,
        lifetime: Duration) {
        self.observe_sample(connected, battery, Some(now), now, lifetime);
    }

    pub fn observe_sample(&mut self, connected: Option<bool>, battery: Option<&Battery>,
        measured: Option<Instant>, now: Instant, lifetime: Duration) {
        self.queried_at = Some(now);
        self.connection_known = connected.is_some();
        if let Some(connected) = connected {
            self.connected = connected;
        }
        let valid = battery.filter(|battery| {
            battery.percent.is_none_or(|percent| percent <= 100)
                && battery.estimated_percent.is_none_or(|percent| percent <= 100)
                && (battery.percent.is_some() || battery.level.is_some()
                    || battery.estimated_percent.is_some() || battery.charging.is_some() || battery.voltage_mv.is_some())
        });
        if connected == Some(true) {
            if let Some(battery) = valid {
                // A fresh charge-only report replaces the whole sample. Its
                // missing percentage must clear the old number, not renew it.
                if battery.percent.is_some() || battery.level.is_some()
                    || battery.estimated_percent.is_some() || battery.charging.is_some() {
                    let measured = measured.unwrap_or(now).min(now);
                    if self.measured_at.is_none_or(|previous| measured > previous) {
                        self.measured_at = Some(measured);
                        self.battery = Some(battery.clone());
                    }
                    // 持续监听器重复发布仍然新鲜的同一采样，不延长测量时间，也不切成“上次电量”。
                    if self.measured_at == Some(measured) && now.saturating_duration_since(measured) < lifetime {
                        self.quality = ReadingQuality::Live;
                        return;
                    }
                }
            }
        }
        self.quality = match self.measured_at {
            Some(measured) if self.connected && now.saturating_duration_since(measured) < lifetime => ReadingQuality::Cached,
            Some(_) => ReadingQuality::Stale,
            None => ReadingQuality::Unknown,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn battery(percent: u8) -> Battery {
        Battery { estimated_percent: None, percent: Some(percent), charging: Some(false), voltage_mv: None,
            approximate: false, level: None }
    }

    #[test]
    fn silence_retains_connection_and_valid_sample_without_extending_its_age() {
        let now = Instant::now();
        let mut state = Status::default();
        state.observe(Some(true), Some(&battery(75)), now, Duration::from_secs(10));
        state.observe(None, None, now + Duration::from_secs(3), Duration::from_secs(10));
        assert!(state.connected);
        assert!(!state.connection_known);
        assert_eq!(state.quality, ReadingQuality::Cached);
        assert_eq!(state.measured_at, Some(now));
        state.observe(None, None, now + Duration::from_secs(10), Duration::from_secs(10));
        assert_eq!(state.quality, ReadingQuality::Stale);
        assert_eq!(state.battery.unwrap().percent, Some(75));
    }

    #[test]
    fn invalid_percent_does_not_replace_a_valid_sample() {
        let now = Instant::now();
        let mut state = Status::default();
        state.observe(Some(true), Some(&battery(75)), now, Duration::from_secs(10));
        state.observe(Some(true), Some(&battery(255)), now + Duration::from_secs(1), Duration::from_secs(10));
        assert_eq!(state.battery.unwrap().percent, Some(75));
        assert_eq!(state.measured_at, Some(now));
        assert_eq!(state.quality, ReadingQuality::Cached);
    }

    #[test]
    fn explicit_disconnect_preserves_history_and_wake_replaces_it() {
        let now = Instant::now();
        let mut state = Status::default();
        state.observe(Some(true), Some(&battery(75)), now, Duration::from_secs(10));
        state.observe(Some(false), None, now, Duration::from_secs(10));
        assert!(!state.connected);
        assert!(state.connection_known);
        assert_eq!(state.quality, ReadingQuality::Stale);
        state.observe(Some(true), Some(&battery(74)), now + Duration::from_secs(1), Duration::from_secs(10));
        assert!(state.connected);
        assert_eq!(state.quality, ReadingQuality::Live);
        assert_eq!(state.battery.unwrap().percent, Some(74));
    }

    #[test]
    fn repeatedly_querying_a_passive_sample_does_not_extend_its_measurement_time() {
        let now = Instant::now();
        let mut state = Status::default();
        state.observe_sample(Some(true), Some(&battery(75)), Some(now), now, Duration::from_secs(8));
        state.observe_sample(Some(true), Some(&battery(75)), Some(now), now + Duration::from_secs(5), Duration::from_secs(8));
        assert_eq!(state.quality, ReadingQuality::Live);
        state.observe_sample(Some(true), Some(&battery(75)), Some(now), now + Duration::from_secs(8), Duration::from_secs(8));
        assert_eq!(state.measured_at, Some(now));
        assert_eq!(state.quality, ReadingQuality::Stale);
    }

    #[test]
    fn sample_expires_without_a_new_poll_or_a_false_connection_change() {
        let now = Instant::now();
        let mut state = Status::default();
        state.observe(Some(true), Some(&battery(75)), now, Duration::from_secs(8));
        assert!(state.expire(now + Duration::from_secs(8), Duration::from_secs(8)));
        assert_eq!(state.quality, ReadingQuality::Stale);
        assert_eq!(state.queried_at, Some(now));
        assert!(state.connected);
    }

    #[test]
    fn charging_without_percentage_replaces_the_old_number_and_expires_normally() {
        let now = Instant::now();
        let lifetime = Duration::from_secs(120);
        let charge_only = Battery { percent: None, estimated_percent: None,
            charging: Some(true), voltage_mv: None, approximate: false, level: None };
        for prior in [false, true] {
            let mut state = Status::default();
            if prior { state.observe(Some(true), Some(&battery(47)), now, lifetime); }
            let measured = now + Duration::from_secs(10);
            state.observe_sample(Some(true), Some(&charge_only), Some(measured), measured, lifetime);
            assert_eq!(state.quality, ReadingQuality::Live);
            assert_eq!(state.battery.as_ref().unwrap().percent, None);
            assert_eq!(state.battery.as_ref().unwrap().charging, Some(true));
            state.observe_sample(None, None, None, measured + lifetime, lifetime);
            assert_eq!(state.measured_at, Some(measured));
            assert_eq!(state.quality, ReadingQuality::Stale);
            state.observe_sample(Some(true), Some(&battery(48)), Some(measured + lifetime), measured + lifetime, lifetime);
            assert_eq!(state.quality, ReadingQuality::Live);
            assert_eq!(state.battery.as_ref().unwrap().percent, Some(48));
        }
    }
}
