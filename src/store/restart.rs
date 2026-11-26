use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Storage::Packaging::Appx::GetCurrentApplicationUserModelId;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED};
use windows::Win32::UI::Shell::{ApplicationActivationManager, IApplicationActivationManager, AO_NONE};

pub const HELPER_ARG: &str = "--complete-update-restart";
const READY: u8 = 1;

fn com_failure(error: windows::core::Error) -> String {
    format!("HRESULT=0x{:08X}", error.code().0 as u32)
}

pub struct Job {
    child: Child,
    ready: Receiver<Result<(), String>>,
    started: Instant,
    armed: bool,
}

impl Job {
    pub fn start() -> Result<Self, String> {
        let application = application_id()?;
        let mut child = Command::new(std::env::current_exe().map_err(|error| error.to_string())?)
            .args([HELPER_ARG, &std::process::id().to_string(), &application])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
            .creation_flags(0x0800_0000)
            .spawn().map_err(|error| error.to_string())?;
        let mut output = child.stdout.take().ok_or("Missing restart channel")?;
        let (sender, ready) = mpsc::channel();
        std::thread::spawn(move || {
            let mut byte = [0];
            let result = output.read_exact(&mut byte).map_err(|error| error.to_string())
                .and_then(|_| if byte == [READY] { Ok(()) } else { Err("Invalid restart acknowledgement".into()) });
            let _ = sender.send(result);
        });
        Ok(Self { child, ready, started: Instant::now(), armed: false })
    }

    pub fn poll(&mut self) -> Result<bool, String> {
        match self.ready.try_recv() {
            Ok(result) => {
                result?;
                let mut input = self.child.stdin.take().ok_or("Missing restart channel")?;
                input.write_all(&[READY]).and_then(|_| input.flush()).map_err(|error| error.to_string())?;
                self.armed = true;
                Ok(true)
            }
            Err(TryRecvError::Empty) if self.started.elapsed() < Duration::from_secs(10) => Ok(false),
            Err(_) => Err("Restart helper did not become ready".into()),
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if !self.armed { let _ = self.child.kill(); }
    }
}

fn application_id() -> Result<String, String> {
    let mut length = 0;
    let result = unsafe { GetCurrentApplicationUserModelId(&mut length, None) };
    if result.0 != 122 || !(2..=1024).contains(&length) {
        return Err(format!("Application identity unavailable: {}", result.0));
    }
    let mut buffer = vec![0; length as usize];
    let result = unsafe { GetCurrentApplicationUserModelId(&mut length, Some(PWSTR(buffer.as_mut_ptr()))) };
    if result.0 != 0 { return Err(format!("Application identity unavailable: {}", result.0)); }
    let length = buffer.iter().position(|value| *value == 0).ok_or("Invalid application identity")?;
    String::from_utf16(&buffer[..length]).map_err(|error| error.to_string())
}

struct Parent(crate::win32::Handle);
impl Drop for Parent {
    fn drop(&mut self) { unsafe { crate::win32::CloseHandle(self.0); } }
}

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) { unsafe { CoUninitialize(); } }
}

fn await_exit(acknowledge: impl FnOnce() -> Result<(), String>,
    authorize: impl FnOnce() -> Result<(), String>,
    wait: impl FnOnce() -> bool,
    activate: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    acknowledge()?;
    authorize()?;
    if !wait() { return Err("Previous application did not exit".into()); }
    activate()
}

fn activate_with_retry(mut activate: impl FnMut() -> Result<u32, String>,
    mut pause: impl FnMut(Duration)) -> Result<u32, String> {
    for attempt in 0..3 {
        match activate() {
            Ok(pid) => return Ok(pid),
            Err(error) if attempt == 2 => return Err(error),
            Err(_) => pause(Duration::from_secs(attempt + 1)),
        }
    }
    unreachable!()
}

fn activation_manager() -> Result<IApplicationActivationManager, String> {
    unsafe { CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER) }.map_err(com_failure)
}

pub fn helper(arguments: &[String]) -> Result<(), String> {
    if arguments.len() != 3 || arguments[0] != HELPER_ARG { return Err("Invalid restart arguments".into()); }
    let parent_id = arguments[1].parse::<u32>().map_err(|error| error.to_string())?;
    if parent_id == 0 || parent_id == std::process::id() || arguments[2] != application_id()? {
        return Err("Invalid restart identity".into());
    }
    let handle = unsafe { crate::win32::OpenProcess(0x0010_0000, 0, parent_id) };
    if handle.is_null() { return Err(std::io::Error::last_os_error().to_string()); }
    let parent = Parent(handle);
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }.map_err(com_failure)?;
    let _apartment = Apartment;
    let _manager = activation_manager()?;
    let application = crate::win32::to_wide(&arguments[2]);
    let restart = crate::win32::to_wide(super::RESTART_ARG);
    await_exit(
        || std::io::stdout().write_all(&[READY]).and_then(|_| std::io::stdout().flush()).map_err(|error| error.to_string()),
        || {
            let mut byte = [0];
            std::io::stdin().read_exact(&mut byte).map_err(|error| error.to_string())?;
            if byte != [READY] { return Err("Restart canceled".into()); }
            Ok(())
        },
        || unsafe { crate::win32::WaitForSingleObject(parent.0, 60_000) == 0 },
        || loop {
            let result = activate_with_retry(|| {
                let manager = activation_manager()?;
                unsafe { manager.ActivateApplication(PCWSTR(application.as_ptr()), PCWSTR(restart.as_ptr()), AO_NONE) }
                    .map_err(com_failure)
            }, std::thread::sleep);
            match result {
                Ok(pid) => {
                    crate::diagnostics::event("store_restart_activated", &format!("pid={pid}"));
                    return Ok(());
                }
                Err(error) => {
                    crate::diagnostics::event("store_restart_failed", &error);
                    let message = crate::win32::to_wide(&format!("{}\n{error}", crate::ui::feedback::text(
                        "更新已安装，重新打开失败。", "Update installed. Unable to reopen the app.")));
                    let title = crate::win32::to_wide("X-Battery");
                    let choice = unsafe { crate::win32::MessageBoxW(std::ptr::null_mut(), message.as_ptr(), title.as_ptr(), 0x15) };
                    if choice != 4 { return Err(error); }
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn activation_waits_for_readiness_authorization_and_parent_exit() {
        let events = RefCell::new(Vec::new());
        await_exit(|| { events.borrow_mut().push("ready"); Ok(()) },
            || { events.borrow_mut().push("authorized"); Ok(()) },
            || { events.borrow_mut().push("exited"); true },
            || { events.borrow_mut().push("activated"); Ok(()) }).unwrap();
        assert_eq!(*events.borrow(), ["ready", "authorized", "exited", "activated"]);
    }

    #[test]
    fn failed_preparation_canceled_parent_and_exit_timeout_never_activate() {
        assert!(await_exit(|| Err("failed".into()), || panic!(), || panic!(), || panic!()).is_err());
        assert!(await_exit(|| Ok(()), || Err("canceled".into()), || panic!(), || panic!()).is_err());
        assert!(await_exit(|| Ok(()), || Ok(()), || false, || panic!()).is_err());
        assert_eq!(await_exit(|| Ok(()), || Ok(()), || true, || Err("activation failed".into())), Err("activation failed".into()));
    }

    #[test]
    fn activation_retries_are_bounded_and_stop_after_success() {
        let mut attempts = 0;
        let mut delays = Vec::new();
        let result = activate_with_retry(|| {
            attempts += 1;
            if attempts == 2 { Ok(42) } else { Err("busy".into()) }
        }, |delay| delays.push(delay));
        assert_eq!(result, Ok(42));
        assert_eq!(attempts, 2);
        assert_eq!(delays, [Duration::from_secs(1)]);
        attempts = 0;
        delays.clear();
        assert!(activate_with_retry(|| { attempts += 1; Err("failed".into()) }, |delay| delays.push(delay)).is_err());
        assert_eq!(attempts, 3);
        assert_eq!(delays, [Duration::from_secs(1), Duration::from_secs(2)]);
    }
}
