mod api;
mod state;

use api::{Api, Completion, Pending};
use state::{local_day, CheckKind, Record, State};
pub use state::{Failure, Phase, View};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::time::Instant;
use windows::Services::Store::StorePackageUpdate;

pub const PRODUCT_URL: &str = "ms-windows-store://pdp/?ProductId=9NRC8HCD4BZ7";
pub const REVIEW_URL: &str = "ms-windows-store://review/?ProductId=9NRC8HCD4BZ7";
pub const UPDATES_URL: &str = "ms-windows-store://downloadsandupdates";

struct Inner {
    state: State,
    pending: Option<Pending>,
    updates: Vec<StorePackageUpdate>,
}

pub struct Controller {
    hwnd: crate::win32::Handle,
    path: PathBuf,
    inner: RefCell<Inner>,
    api: RefCell<Option<Api>>,
    in_api: Cell<bool>,
    started: Cell<bool>,
}

fn load_record(path: &std::path::Path, version: &str) -> Record {
    let record = if std::fs::metadata(path).is_ok_and(|metadata| metadata.len() <= 4096) {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    } else {
        Record::default()
    };
    record.for_version(version)
}

impl Controller {
    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self {
            hwnd: std::ptr::null_mut(),
            path: PathBuf::new(),
            inner: RefCell::new(Inner {
                state: State::new(
                    Record::default().for_version(crate::gui::APP_VERSION),
                    false,
                ),
                pending: None,
                updates: Vec::new(),
            }),
            api: RefCell::new(None),
            in_api: Cell::new(false),
            started: Cell::new(false),
        }
    }

    pub fn new(hwnd: crate::win32::Handle) -> Self {
        let supported = crate::autostart::is_packaged() && !crate::mock::enabled();
        let path = crate::paths::cache_dir().join("store-updates.json");
        let version = if supported {
            api::installed_version()
                .map(|version| format!("{}|{version}", crate::gui::APP_VERSION))
                .unwrap_or_else(|_| crate::gui::APP_VERSION.into())
        } else {
            crate::gui::APP_VERSION.into()
        };
        let record = load_record(&path, &version);
        Self {
            hwnd,
            path,
            inner: RefCell::new(Inner {
                state: State::new(record, supported),
                pending: None,
                updates: Vec::new(),
            }),
            api: RefCell::new(None),
            in_api: Cell::new(false),
            started: Cell::new(false),
        }
    }

    pub fn view(&self) -> View {
        self.inner.borrow().state.view(local_day())
    }

    fn save(&self) {
        let record = self.inner.borrow().state.record.clone();
        let result = serde_json::to_vec(&record)
            .map_err(|error| error.to_string())
            .and_then(|bytes| crate::paths::write_atomic(&self.path, &bytes));
        if let Err(error) = result {
            crate::diagnostics::event_once("store_update_cache_failed", &error);
        }
    }

    fn with_api(
        &self,
        action: impl FnOnce(&Api) -> Result<Pending, Failure>,
    ) -> Result<Pending, Failure> {
        self.in_api.set(true);
        let cached = self.api.borrow_mut().take();
        let api = match cached.map(Ok).unwrap_or_else(|| Api::new(self.hwnd)) {
            Ok(api) => api,
            Err(error) => {
                self.in_api.set(false);
                return Err(error);
            }
        };
        let result = action(&api);
        self.api.replace(Some(api));
        self.in_api.set(false);
        result
    }

    fn start_check(&self, kind: CheckKind) {
        if self.in_api.get() || !self.inner.borrow_mut().state.begin_check(kind, local_day()) {
            return;
        }
        self.save();
        match self.with_api(Api::check) {
            Ok(pending) => self.inner.borrow_mut().pending = Some(pending),
            Err(error) => self
                .inner
                .borrow_mut()
                .state
                .check_failed(error, Instant::now()),
        }
    }

    pub fn startup(&self) {
        if !self.started.replace(true) {
            self.start_check(CheckKind::Automatic);
        }
    }

    pub fn check(&self) {
        self.start_check(CheckKind::Manual);
    }

    pub fn download(&self) {
        self.start_check(CheckKind::BeforeDownload);
    }

    fn start_transfer(&self, install: bool) {
        let updates = {
            let mut inner = self.inner.borrow_mut();
            inner.state.phase = if install {
                Phase::Installing
            } else {
                Phase::Downloading
            };
            inner.state.failure = None;
            inner.updates.clone()
        };
        match self.with_api(|api| api.transfer(&updates, install)) {
            Ok(pending) => self.inner.borrow_mut().pending = Some(pending),
            Err(error) => {
                let mut inner = self.inner.borrow_mut();
                inner.state.phase = Phase::UpdateFailed;
                inner.state.failure = Some(error);
                drop(inner);
                if install {
                    api::unregister_restart();
                }
            }
        }
    }

    pub fn begin_install(&self) -> bool {
        if self.in_api.get() {
            return false;
        }
        let mut inner = self.inner.borrow_mut();
        if inner.state.phase != Phase::ReadyToInstall || inner.updates.is_empty() {
            return false;
        }
        inner.state.phase = Phase::Preparing;
        true
    }

    pub fn install(&self) {
        if self.inner.borrow().state.phase != Phase::Preparing {
            return;
        }
        api::register_restart();
        self.start_transfer(true);
    }

    pub fn preparation_failed(&self) {
        self.inner.borrow_mut().state.phase = Phase::ReadyToInstall;
    }

    pub fn installing(&self) -> bool {
        matches!(
            self.inner.borrow().state.phase,
            Phase::Preparing | Phase::Installing
        )
    }

    pub fn dismiss(&self) {
        self.inner.borrow_mut().state.dismiss(local_day());
        self.save();
    }

    pub fn cancel(&self) {
        if self.in_api.get() || self.installing() || self.view().phase == Phase::Unsupported {
            return;
        }
        let pending = self.inner.borrow_mut().pending.take();
        if let Some(pending) = pending {
            pending.cancel();
        }
        let mut inner = self.inner.borrow_mut();
        inner.state.phase = Phase::Canceled;
        inner.state.retry_at = None;
        inner.state.failure = None;
    }

    pub fn tick(&self, main_active: bool) -> bool {
        if self.in_api.get() {
            return false;
        }
        let before = self.view();
        let pending = self.inner.borrow_mut().pending.take();
        if let Some(pending) = pending {
            self.in_api.set(true);
            let result = pending.poll();
            self.in_api.set(false);
            match result {
                Ok(None) => self.inner.borrow_mut().pending = Some(pending),
                Ok(Some(Completion::Checked(updates))) => {
                    let mut inner = self.inner.borrow_mut();
                    inner.state.checked(!updates.is_empty(), local_day());
                    inner.updates = updates;
                    drop(inner);
                    self.save();
                }
                Ok(Some(Completion::Downloaded)) => {
                    self.inner.borrow_mut().state.phase = Phase::ReadyToInstall
                }
                Ok(Some(Completion::Installed)) => {
                    let mut inner = self.inner.borrow_mut();
                    inner.state.phase = Phase::Installed;
                    inner.state.record.available = false;
                    drop(inner);
                    self.save();
                }
                Ok(Some(Completion::Canceled)) => {
                    let installing = self.installing();
                    let mut inner = self.inner.borrow_mut();
                    inner.state.phase = Phase::Canceled;
                    inner.state.retry_at = None;
                    inner.state.failure = None;
                    drop(inner);
                    if installing {
                        api::unregister_restart();
                    }
                }
                Err(error) => {
                    let mut inner = self.inner.borrow_mut();
                    let installing = inner.state.phase == Phase::Installing;
                    if inner.state.phase == Phase::Checking {
                        inner.state.check_failed(error, Instant::now());
                    } else {
                        inner.state.phase = Phase::UpdateFailed;
                        inner.state.failure = Some(error);
                    }
                    drop(inner);
                    if installing {
                        api::unregister_restart();
                    }
                }
            }
        }
        if self.inner.borrow().state.retry_due(Instant::now()) {
            self.start_check(CheckKind::Automatic);
        }
        if main_active && self.inner.borrow().state.phase == Phase::WaitingToDownload {
            self.start_transfer(false);
        }
        before != self.view()
    }

    pub fn shutdown(&self) {
        let pending = self.inner.borrow_mut().pending.take();
        if let Some(pending) = pending {
            if !self.installing() {
                pending.cancel();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_invalid_cache_has_no_cached_update() {
        let missing = crate::paths::cache_dir().join("missing-store-update-cache-test.json");
        let loaded = load_record(&missing, "1.4.8");
        assert_eq!(loaded.schema, 1);
        assert!(!loaded.available);
        assert!(loaded.auto_check_day.is_none());
    }

    #[test]
    fn all_store_links_use_official_protocol_and_the_existing_product() {
        assert_eq!(
            PRODUCT_URL,
            "ms-windows-store://pdp/?ProductId=9NRC8HCD4BZ7"
        );
        assert_eq!(
            REVIEW_URL,
            "ms-windows-store://review/?ProductId=9NRC8HCD4BZ7"
        );
        assert_eq!(UPDATES_URL, "ms-windows-store://downloadsandupdates");
    }
}
