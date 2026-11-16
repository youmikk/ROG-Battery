//! Share only short-lived descriptor scans, never device readings or write authorization.
use crate::{hid::{self, EnumerateReport}, win32::HidApi};
use std::{sync::{Mutex, OnceLock}, time::{Duration, Instant}};

const LIFETIME: Duration = Duration::from_millis(500);
#[derive(Default)]
struct Cache { report: Option<(Instant, EnumerateReport)> }
impl Cache {
    fn read(&mut self, now: Instant, scan: impl FnOnce() -> Result<EnumerateReport,String>) -> Result<EnumerateReport,String> {
        if let Some((taken,report)) = &self.report {
            if now.saturating_duration_since(*taken) < LIFETIME { return Ok(report.clone()); }
        }
        // Invalidate before scanning: a failure must not turn an expired scan into
        // fresh presence evidence, or turn an unavailable scan into an empty device list.
        self.report = None;
        let report = scan()?;
        self.report = Some((now,report.clone()));
        Ok(report)
    }
}
pub fn scan(api: &HidApi) -> Result<EnumerateReport,String> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner());
    cache.read(Instant::now(), || hid::enumerate_all_report(api))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn report() -> EnumerateReport { EnumerateReport {interfaces:Vec::new(),skipped:vec![hid::Skipped {
        path:"still-present".into(),reason:"temporarily unavailable".into(),
    }]} }
    #[test]
    fn nearby_scans_share_presence_evidence_without_extending_expiry() {
        let now=Instant::now();let mut cache=Cache::default();
        cache.read(now,||Ok(report())).unwrap();
        let reused=cache.read(now+Duration::from_millis(499),||panic!("scan should be shared")).unwrap();
        assert_eq!(reused.skipped[0].path,"still-present");
        assert!(cache.read(now+LIFETIME,||Err("scan failed".into())).is_err());
        assert!(cache.report.is_none());
    }
    #[test]
    fn failed_scan_remains_an_error_and_next_scan_can_recover() {
        let mut cache=Cache::default();let now=Instant::now();
        assert!(cache.read(now,||Err("unavailable".into())).is_err());
        assert_eq!(cache.read(now,||Ok(report())).unwrap().skipped.len(),1);
    }
}
