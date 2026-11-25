use super::AutoStartState;
use std::marker::PhantomData;
use std::rc::Rc;
use std::time::{Duration, Instant};
use windows::core::{Error, RuntimeType, HSTRING};
use windows::ApplicationModel::StartupTask;
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};
use windows_future::{AsyncStatus, IAsyncOperation};

const STARTUP_TASK_ID: &str = "RogBatteryAutoStart";
const RPC_E_CHANGED_MODE: i32 = 0x80010106u32 as i32;
const WAIT_TIMEOUT: Duration = Duration::from_secs(5);

struct Apartment {
    initialized: bool,
    _thread: PhantomData<Rc<()>>,
}

impl Apartment {
    fn open() -> Result<Self, String> {
        let initialized = match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
            Ok(()) => true,
            Err(error) if error.code().0 == RPC_E_CHANGED_MODE => false,
            Err(error) => return Err(describe_error("RoInitialize", error)),
        };
        Ok(Self { initialized, _thread: PhantomData })
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { RoUninitialize() };
        }
    }
}

fn describe_error(operation: &str, error: Error) -> String {
    format!("{operation} 失败：0x{:08X}", error.code().0 as u32)
}

fn wait_operation<T: RuntimeType + 'static>(
    operation: IAsyncOperation<T>,
    name: &str,
    timeout: Duration,
) -> Result<T, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let status = match operation.Status() {
            Ok(status) => status,
            Err(error) => {
                let _ = operation.Cancel();
                return Err(describe_error(name, error));
            }
        };
        if status != AsyncStatus::Started {
            return operation.GetResults().map_err(|error| describe_error(name, error));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let _ = operation.Cancel();
            return Err(format!("{name} 超时"));
        }
        std::thread::sleep(remaining.min(Duration::from_millis(20)));
    }
}

fn with_task(
    action: impl FnOnce(&StartupTask) -> Result<AutoStartState, String>,
) -> Result<AutoStartState, String> {
    let _apartment = Apartment::open()?;
    let operation = StartupTask::GetAsync(&HSTRING::from(STARTUP_TASK_ID))
        .map_err(|error| describe_error("StartupTask.GetAsync", error))?;
    let task = wait_operation(operation, "StartupTask.GetAsync", WAIT_TIMEOUT)?;
    action(&task)
}

pub(super) fn state() -> Result<AutoStartState, String> {
    with_task(|task| {
        task.State()
            .map(|state| AutoStartState::from_winrt(state.0))
            .map_err(|error| describe_error("StartupTask.State", error))
    })
}

pub(super) fn enable() -> Result<AutoStartState, String> {
    with_task(|task| {
        let operation = task.RequestEnableAsync()
            .map_err(|error| describe_error("StartupTask.RequestEnableAsync", error))?;
        wait_operation(operation, "StartupTask.RequestEnableAsync", WAIT_TIMEOUT)
            .map(|state| AutoStartState::from_winrt(state.0))
    })
}

pub(super) fn disable() -> Result<AutoStartState, String> {
    with_task(|task| {
        task.Disable().map_err(|error| describe_error("StartupTask.Disable", error))?;
        task.State()
            .map(|state| AutoStartState::from_winrt(state.0))
            .map_err(|error| describe_error("StartupTask.State", error))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::HRESULT;
    use windows::Win32::System::WinRT::RO_INIT_SINGLETHREADED;

    #[test]
    fn repeated_initialization_on_the_same_thread_succeeds() {
        std::thread::spawn(|| {
            let first = Apartment::open().unwrap();
            let second = Apartment::open().unwrap();
            assert!(first.initialized);
            assert!(second.initialized);
            drop(second);
            drop(first);
            let fresh = unsafe { RoInitialize(RO_INIT_SINGLETHREADED) };
            assert!(fresh.is_ok(), "initialization references must be balanced: {fresh:?}");
            unsafe { RoUninitialize() };
        }).join().unwrap();
    }

    #[test]
    fn existing_single_threaded_apartment_is_not_uninitialized() {
        std::thread::spawn(|| {
            unsafe { RoInitialize(RO_INIT_SINGLETHREADED) }.unwrap();
            let borrowed = Apartment::open().unwrap();
            assert!(!borrowed.initialized);
            drop(borrowed);
            let changed = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
            assert_eq!(changed.unwrap_err().code().0, RPC_E_CHANGED_MODE);
            unsafe { RoUninitialize() };
        }).join().unwrap();
    }

    #[test]
    fn completed_and_failed_operations_preserve_their_results() {
        let success = IAsyncOperation::ready(Ok(42u32));
        assert_eq!(wait_operation(success, "test", Duration::ZERO).unwrap(), 42);
        let failure = IAsyncOperation::<u32>::ready(Err(Error::from_hresult(HRESULT(0x80070005u32 as i32))));
        assert!(wait_operation(failure, "test", WAIT_TIMEOUT).unwrap_err().contains("80070005"));
    }

    #[test]
    fn pending_operation_returns_at_the_deadline() {
        let (sender, receiver) = std::sync::mpsc::channel::<()>();
        let operation = IAsyncOperation::spawn(move || {
            let _ = receiver.recv();
            Ok(42u32)
        });
        assert!(wait_operation(operation, "test", Duration::ZERO).unwrap_err().contains("超时"));
        drop(sender);
    }

    #[test]
    fn manifest_task_identity_matches_the_api_request() {
        assert!(include_str!("../../msix/package.props.psd1")
            .contains(&format!("StartupTaskId = '{STARTUP_TASK_ID}'")));
        assert!(include_str!("../../msix/AppxManifest.xml.template")
            .contains("TaskId=\"@STARTUP_TASK_ID@\""));
    }
}
