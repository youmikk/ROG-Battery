use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub const MAX_AUTO_ATTEMPTS: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Unsupported,
    Idle,
    Checking,
    Available,
    Current,
    CheckFailed,
    AwaitingPreparation,
    Preparing,
    Installing,
    Installed,
    Restarting,
    RestartFailed,
    Canceled,
    UpdateFailed,
}

impl Phase {
    pub fn busy(self) -> bool {
        matches!(
            self,
            Self::Checking
                | Self::AwaitingPreparation
                | Self::Preparing
                | Self::Installing
                | Self::Installed
                | Self::Restarting
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Windows(i32),
    TimedOut,
    LowBattery,
    WifiRecommended,
    WifiRequired,
    Unexpected,
}

impl Failure {
    pub fn diagnostic(self) -> String {
        match self {
            Self::Windows(code) => format!("HRESULT=0x{:08X}", code as u32),
            _ => format!("{self:?}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub phase: Phase,
    pub available: bool,
    pub home_visible: bool,
    pub failure: Option<Failure>,
    pub last_checked_day: Option<u32>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Record {
    pub schema: u8,
    pub app_version: String,
    pub auto_check_day: Option<u32>,
    pub last_checked_day: Option<u32>,
    pub available: bool,
    pub dismissed_day: Option<u32>,
    pub prompted_day: Option<u32>,
}

impl Record {
    pub fn for_version(self, version: &str) -> Self {
        if self.schema == 1 && self.app_version == version {
            self
        } else {
            Self {
                schema: 1,
                app_version: version.into(),
                ..Default::default()
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckKind {
    Automatic,
    Manual,
    BeforeUpdate,
}

pub struct State {
    pub record: Record,
    pub phase: Phase,
    pub failure: Option<Failure>,
    pub check_kind: CheckKind,
    pub retry_at: Option<Instant>,
    automatic_attempts: u8,
}

impl State {
    pub fn new(record: Record, supported: bool) -> Self {
        let phase = if !supported {
            Phase::Unsupported
        } else if record.available {
            Phase::Available
        } else {
            Phase::Idle
        };
        Self {
            record,
            phase,
            failure: None,
            check_kind: CheckKind::Automatic,
            retry_at: None,
            automatic_attempts: 0,
        }
    }

    pub fn begin_check(&mut self, kind: CheckKind, day: u32) -> bool {
        if self.phase == Phase::Unsupported || self.phase.busy() {
            return false;
        }
        if kind == CheckKind::Automatic {
            if self.automatic_attempts == 0 && self.record.auto_check_day == Some(day) {
                return false;
            }
            if self.automatic_attempts >= MAX_AUTO_ATTEMPTS {
                return false;
            }
            self.automatic_attempts += 1;
        }
        self.record.auto_check_day = Some(day);
        self.check_kind = kind;
        self.retry_at = None;
        self.phase = Phase::Checking;
        self.failure = None;
        true
    }

    pub fn checked(&mut self, available: bool, day: u32) {
        self.record.available = available;
        self.record.last_checked_day = Some(day);
        self.retry_at = None;
        self.failure = None;
        self.phase = if !available {
            Phase::Current
        } else if self.check_kind == CheckKind::BeforeUpdate {
            Phase::AwaitingPreparation
        } else {
            Phase::Available
        };
        if self.check_kind == CheckKind::Manual {
            self.record.dismissed_day = None;
            self.record.prompted_day = None;
        }
    }

    pub fn check_failed(&mut self, failure: Failure, now: Instant) {
        self.phase = Phase::CheckFailed;
        self.failure = Some(failure);
        self.retry_at = if self.check_kind == CheckKind::Automatic
            && self.automatic_attempts < MAX_AUTO_ATTEMPTS
        {
            Some(
                now + Duration::from_secs(if self.automatic_attempts == 1 {
                    30
                } else {
                    120
                }),
            )
        } else {
            None
        };
    }

    pub fn retry_due(&self, now: Instant) -> bool {
        !self.phase.busy() && self.retry_at.is_some_and(|deadline| now >= deadline)
    }

    pub fn dismiss(&mut self, day: u32) {
        self.record.dismissed_day = Some(day);
    }

    pub fn take_prompt(&mut self, home_active: bool, day: u32) -> bool {
        if !home_active || self.phase != Phase::Available || !self.view(day).home_visible
            || self.record.prompted_day == Some(day) {
            return false;
        }
        self.record.prompted_day = Some(day);
        true
    }

    pub fn view(&self, day: u32) -> View {
        let home_visible = (self.record.available || self.phase == Phase::RestartFailed)
            && self.record.dismissed_day != Some(day)
            && self.phase != Phase::Unsupported
            && self.phase != Phase::Installed;
        View {
            phase: self.phase,
            available: self.record.available,
            home_visible,
            failure: self.failure,
            last_checked_day: self.record.last_checked_day,
        }
    }
}

pub fn local_day() -> u32 {
    let mut time = crate::win32::SystemTime::default();
    unsafe { crate::win32::GetLocalTime(&mut time) };
    u32::from(time.year) * 10_000 + u32::from(time.month) * 100 + u32::from(time.day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> Record {
        Record::default().for_version("1.4.8")
    }

    #[test]
    fn automatic_check_runs_once_per_local_day_across_restarts() {
        let mut state = State::new(record(), true);
        assert!(state.begin_check(CheckKind::Automatic, 20261125));
        state.checked(false, 20261125);
        let mut restarted = State::new(state.record.clone(), true);
        assert!(!restarted.begin_check(CheckKind::Automatic, 20261125));
        assert!(restarted.begin_check(CheckKind::Automatic, 20261126));
    }

    #[test]
    fn a_failed_startup_attempt_is_also_remembered() {
        let mut state = State::new(record(), true);
        assert!(state.begin_check(CheckKind::Automatic, 20261125));
        state.check_failed(Failure::TimedOut, Instant::now());
        let mut restarted = State::new(state.record, true);
        assert!(!restarted.begin_check(CheckKind::Automatic, 20261125));
    }

    #[test]
    fn automatic_retry_is_delayed_and_bounded() {
        let mut state = State::new(record(), true);
        let now = Instant::now();
        assert!(state.begin_check(CheckKind::Automatic, 20261125));
        state.check_failed(Failure::TimedOut, now);
        assert!(!state.retry_due(now + Duration::from_secs(29)));
        assert!(state.retry_due(now + Duration::from_secs(30)));
        assert!(state.begin_check(CheckKind::Automatic, 20261125));
        state.check_failed(Failure::TimedOut, now);
        assert!(!state.retry_due(now + Duration::from_secs(119)));
        assert!(state.retry_due(now + Duration::from_secs(120)));
        assert!(state.begin_check(CheckKind::Automatic, 20261125));
        state.check_failed(Failure::TimedOut, now);
        assert!(state.retry_at.is_none());
        assert!(!state.begin_check(CheckKind::Automatic, 20261125));
    }

    #[test]
    fn manual_checks_are_not_limited_by_the_daily_record() {
        let mut state = State::new(record(), true);
        assert!(state.begin_check(CheckKind::Automatic, 20261125));
        state.checked(false, 20261125);
        assert!(state.begin_check(CheckKind::Manual, 20261125));
        state.checked(false, 20261125);
        assert!(state.begin_check(CheckKind::Manual, 20261125));
    }

    #[test]
    fn manual_failure_does_not_schedule_background_retries() {
        let mut state = State::new(record(), true);
        assert!(state.begin_check(CheckKind::Manual, 20261125));
        state.check_failed(Failure::TimedOut, Instant::now());
        assert!(state.retry_at.is_none());
        assert_ne!(state.phase, Phase::Current);
        assert!(state.record.last_checked_day.is_none());
    }

    #[test]
    fn installed_version_changes_invalidate_cached_updates() {
        let mut cached = record();
        cached.available = true;
        cached.auto_check_day = Some(20261125);
        cached.last_checked_day = Some(20261125);
        cached.dismissed_day = Some(20261125);
        let changed = cached.for_version("1.4.9");
        assert!(!changed.available);
        assert!(changed.auto_check_day.is_none());
        assert!(changed.last_checked_day.is_none());
        assert!(changed.dismissed_day.is_none());
    }

    #[test]
    fn unknown_cache_schema_is_reset() {
        let mut cached = record();
        cached.schema = 2;
        cached.available = true;
        assert!(!cached.for_version("1.4.8").available);
    }

    #[test]
    fn package_revision_changes_also_invalidate_cached_updates() {
        let mut cached = Record::default().for_version("1.4.8|revision-1");
        cached.available = true;
        cached.auto_check_day = Some(20261125);
        let updated = cached.for_version("1.4.8|revision-2");
        assert!(!updated.available);
        assert!(updated.auto_check_day.is_none());
    }

    #[test]
    fn postponing_hides_home_card_for_the_current_day_only() {
        let mut state = State::new(record(), true);
        state.record.available = true;
        state.phase = Phase::Available;
        assert!(state.view(20261125).home_visible);
        state.dismiss(20261125);
        assert!(!state.view(20261125).home_visible);
        assert!(state.view(20261126).home_visible);
        assert!(state.view(20261125).available);
    }

    #[test]
    fn portable_and_mock_modes_never_start_checks_or_show_update_cards() {
        let mut cached = record();
        cached.available = true;
        let mut state = State::new(cached, false);
        assert!(!state.begin_check(CheckKind::Automatic, 20261125));
        assert!(!state.begin_check(CheckKind::Manual, 20261125));
        assert!(!state.view(20261125).home_visible);
    }

    #[test]
    fn automatic_discovery_never_starts_a_download() {
        let mut state = State::new(record(), true);
        assert!(state.begin_check(CheckKind::Automatic, 20261125));
        state.checked(true, 20261125);
        assert_eq!(state.phase, Phase::Available);
    }

    #[test]
    fn update_intent_requires_a_fresh_successful_query() {
        let mut state = State::new(record(), true);
        assert!(state.begin_check(CheckKind::BeforeUpdate, 20261125));
        state.checked(true, 20261125);
        assert_eq!(state.phase, Phase::AwaitingPreparation);
    }

    #[test]
    fn no_update_clears_previously_available_results() {
        let mut state = State::new(record(), true);
        state.record.available = true;
        state.phase = Phase::Available;
        assert!(state.begin_check(CheckKind::BeforeUpdate, 20261125));
        state.checked(false, 20261125);
        assert!(!state.view(20261125).home_visible);
        assert_eq!(state.phase, Phase::Current);
    }

    #[test]
    fn simultaneous_queries_are_rejected() {
        let mut state = State::new(record(), true);
        assert!(state.begin_check(CheckKind::Automatic, 20261125));
        assert!(!state.begin_check(CheckKind::Manual, 20261125));
    }

    #[test]
    fn transfer_and_install_phases_are_busy() {
        for phase in [
            Phase::Preparing,
            Phase::Installing,
            Phase::Installed,
            Phase::Restarting,
            Phase::AwaitingPreparation,
        ] {
            let mut state = State::new(record(), true);
            state.phase = phase;
            assert!(!state.begin_check(CheckKind::Manual, 20261125));
        }
    }

    #[test]
    fn cache_roundtrips_without_secrets_or_device_data() {
        let mut cached = record();
        cached.auto_check_day = Some(20261125);
        cached.last_checked_day = Some(20261125);
        cached.available = true;
        let json = serde_json::to_string(&cached).unwrap();
        assert_eq!(serde_json::from_str::<Record>(&json).unwrap(), cached);
        assert!(!json.contains("device"));
        assert!(!json.contains("token"));
    }

    #[test]
    fn prompt_waits_for_home_and_is_consumed_before_reentry_or_restart() {
        let mut state = State::new(record(), true);
        state.checked(true, 20261126);
        assert!(!state.take_prompt(false, 20261126));
        assert!(state.take_prompt(true, 20261126));
        assert!(!state.take_prompt(true, 20261126));
        let mut restarted = State::new(state.record.clone(), true);
        assert!(!restarted.take_prompt(true, 20261126));
        assert!(restarted.take_prompt(true, 20261127));
    }

    #[test]
    fn later_suppresses_automatic_prompts_but_manual_check_can_offer_again() {
        let mut state = State::new(record(), true);
        state.checked(true, 20261126);
        state.dismiss(20261126);
        assert!(!state.take_prompt(true, 20261126));
        assert!(state.begin_check(CheckKind::Manual, 20261126));
        state.checked(true, 20261126);
        assert!(state.take_prompt(true, 20261126));
        for phase in [Phase::Checking, Phase::AwaitingPreparation, Phase::Preparing,
            Phase::Installing, Phase::Installed, Phase::Canceled, Phase::UpdateFailed] {
            state.phase = phase;
            assert!(!state.take_prompt(true, 20261127));
        }
    }
}
