use super::state::Failure;
use std::time::{Duration, Instant};
use windows::core::{Interface, PCWSTR};
use windows::ApplicationModel::Package;
use windows::Services::Store::{StoreContext, StorePackageUpdate, StorePackageUpdateState};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Recovery::{
    RegisterApplicationRestart, UnregisterApplicationRestart, RESTART_NO_CRASH, RESTART_NO_HANG,
    RESTART_NO_REBOOT,
};
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_SINGLETHREADED};
use windows::Win32::UI::Shell::IInitializeWithWindow;
use windows_collections::IIterable;
use windows_future::AsyncStatus;

pub enum Completion {
    Checked(Vec<StorePackageUpdate>),
    Installed,
    Canceled,
}

pub struct Pending {
    poll: Box<dyn Fn() -> Result<Option<Completion>, Failure>>,
    cancel: Box<dyn Fn()>,
    deadline: Option<Instant>,
}

impl Pending {
    pub fn poll(&self) -> Result<Option<Completion>, Failure> {
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.cancel();
            return Err(Failure::TimedOut);
        }
        (self.poll)()
    }

    pub fn cancel(&self) {
        (self.cancel)();
    }
}

struct Apartment;

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

pub struct Api {
    context: StoreContext,
    _apartment: Apartment,
}

fn failure(error: windows::core::Error) -> Failure {
    Failure::Windows(error.code().0)
}

fn transfer_result(state: StorePackageUpdateState) -> Result<Completion, Failure> {
    match state {
        StorePackageUpdateState::Completed => Ok(Completion::Installed),
        StorePackageUpdateState::Canceled => Ok(Completion::Canceled),
        StorePackageUpdateState::ErrorLowBattery => Err(Failure::LowBattery),
        StorePackageUpdateState::ErrorWiFiRecommended => Err(Failure::WifiRecommended),
        StorePackageUpdateState::ErrorWiFiRequired => Err(Failure::WifiRequired),
        _ => Err(Failure::Unexpected),
    }
}

impl Api {
    pub fn new(hwnd: crate::win32::Handle) -> Result<Self, Failure> {
        unsafe { RoInitialize(RO_INIT_SINGLETHREADED) }.map_err(failure)?;
        let apartment = Apartment;
        let context = StoreContext::GetDefault().map_err(failure)?;
        let owner: IInitializeWithWindow = context.cast().map_err(failure)?;
        unsafe { owner.Initialize(HWND(hwnd)) }.map_err(failure)?;
        Ok(Self {
            context,
            _apartment: apartment,
        })
    }

    pub fn check(&self) -> Result<Pending, Failure> {
        let operation = self
            .context
            .GetAppAndOptionalStorePackageUpdatesAsync()
            .map_err(failure)?;
        let cancellation = operation.clone();
        Ok(Pending {
            poll: Box::new(move || {
                match operation.Status().map_err(failure)? {
                    AsyncStatus::Started => return Ok(None),
                    AsyncStatus::Canceled => return Ok(Some(Completion::Canceled)),
                    _ => {}
                }
                let results = operation.GetResults().map_err(failure)?;
                let count = results.Size().map_err(failure)?;
                let mut updates = Vec::new();
                for index in 0..count {
                    updates.push(results.GetAt(index).map_err(failure)?);
                }
                Ok(Some(Completion::Checked(updates)))
            }),
            cancel: Box::new(move || {
                let _ = cancellation.Cancel();
            }),
            deadline: Some(Instant::now() + Duration::from_secs(45)),
        })
    }

    pub fn update(
        &self,
        updates: &[StorePackageUpdate],
    ) -> Result<Pending, Failure> {
        let packages: IIterable<StorePackageUpdate> =
            updates.iter().cloned().map(Some).collect::<Vec<_>>().into();
        let silent = self.context.CanSilentlyDownloadStorePackageUpdates().unwrap_or(false);
        crate::diagnostics::event("store_update_started", if silent { "silent" } else { "interactive" });
        let operation = if silent {
            self.context
                .TrySilentDownloadAndInstallStorePackageUpdatesAsync(&packages)
        } else {
            self.context
                .RequestDownloadAndInstallStorePackageUpdatesAsync(&packages)
        }
        .map_err(failure)?;
        let cancellation = operation.clone();
        Ok(Pending {
            poll: Box::new(move || {
                match operation.Status().map_err(failure)? {
                    AsyncStatus::Started => return Ok(None),
                    AsyncStatus::Canceled => return Ok(Some(Completion::Canceled)),
                    _ => {}
                }
                let result = operation.GetResults().map_err(failure)?;
                transfer_result(result.OverallState().map_err(failure)?).map(Some)
            }),
            cancel: Box::new(move || {
                let _ = cancellation.Cancel();
            }),
            deadline: None,
        })
    }
}

pub fn register_restart() -> Result<(), Failure> {
    let arguments = crate::win32::to_wide(super::RESTART_ARG);
    unsafe {
        RegisterApplicationRestart(
            PCWSTR(arguments.as_ptr()),
            RESTART_NO_CRASH | RESTART_NO_HANG | RESTART_NO_REBOOT,
        )
    }.map_err(failure)
}

pub fn installed_version() -> Result<String, Failure> {
    unsafe { RoInitialize(RO_INIT_SINGLETHREADED) }.map_err(failure)?;
    let _apartment = Apartment;
    let version = Package::Current()
        .and_then(|package| package.Id())
        .and_then(|identity| identity.Version())
        .map_err(failure)?;
    Ok(format!(
        "{}.{}.{}.{}",
        version.Major, version.Minor, version.Build, version.Revision
    ))
}

pub fn unregister_restart() {
    let _ = unsafe { UnregisterApplicationRestart() };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_update_completion_means_installed() {
        assert!(matches!(
            transfer_result(StorePackageUpdateState::Completed),
            Ok(Completion::Installed)
        ));
    }

    #[test]
    fn store_cancellation_is_not_reported_as_success() {
        assert!(matches!(
            transfer_result(StorePackageUpdateState::Canceled),
            Ok(Completion::Canceled)
        ));
    }

    #[test]
    fn transfer_errors_are_not_reported_as_installed() {
        for state in [
            StorePackageUpdateState::OtherError,
            StorePackageUpdateState::ErrorLowBattery,
            StorePackageUpdateState::ErrorWiFiRecommended,
            StorePackageUpdateState::ErrorWiFiRequired,
            StorePackageUpdateState::Pending,
            StorePackageUpdateState::Deploying,
        ] {
            assert!(transfer_result(state).is_err());
        }
    }
}
