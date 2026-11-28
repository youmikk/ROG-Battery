#![cfg_attr(not(test), windows_subsystem = "windows")]

use std::io;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

#[path = "autostart/args.rs"]
mod args;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn startup_command(launcher: &Path) -> io::Result<Command> {
    if !launcher.is_absolute() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Launcher path must be absolute"));
    }
    let mut command = Command::new(launcher.with_file_name("x-battery.exe"));
    command.arg(args::RUN_ARG).creation_flags(CREATE_NO_WINDOW);
    Ok(command)
}

fn main() -> io::Result<()> {
    startup_command(&std::env::current_exe()?)?.spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn command_uses_absolute_sibling_and_only_autostart_argument() {
        for directory in [r"C:\Program Files\WindowsApps\示例", r"\\server\share\X Battery"] {
            let directory = Path::new(directory);
            let command = startup_command(&directory.join("x-battery-startup.exe")).unwrap();
            let target = Path::new(command.get_program());
            assert!(target.is_absolute());
            assert_eq!(target, directory.join("x-battery.exe"));
            assert_eq!(command.get_args().collect::<Vec<_>>(), [OsStr::new(args::RUN_ARG)]);
        }
    }

    #[test]
    fn relative_paths_cannot_use_the_working_directory() {
        for launcher in ["x-battery-startup.exe", r".\x-battery-startup.exe", r"C:x-battery-startup.exe", r"\x-battery-startup.exe"] {
            assert_eq!(startup_command(Path::new(launcher)).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        }
    }
}
