
use std::path::PathBuf;

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// 配置目录：`%APPDATA%\rog-mouse-ctl\`
pub fn config_dir() -> PathBuf {
    if let Some(base) = env_dir("APPDATA") {
        return base.join("rog-mouse-ctl");
    }
    // 兜底：正常 Windows 上不会走到这里
    env_dir("USERPROFILE")
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".rog-mouse-ctl")
}

/// 缓存目录：`%LOCALAPPDATA%\rog-mouse-ctl\`
pub fn cache_dir() -> PathBuf {
    if let Some(base) = env_dir("LOCALAPPDATA") {
        return base.join("rog-mouse-ctl");
    }
    config_dir()
}

/// 备份目录：`%USERPROFILE%\Documents\rog-mouse-ctl\backups\`
pub fn backup_dir() -> PathBuf {
    if let Some(home) = env_dir("USERPROFILE") {
        return home
            .join("Documents")
            .join("rog-mouse-ctl")
            .join("backups");
    }
    config_dir().join("backups")
}

/// 配置文件路径（窗口位置、托盘开关、缓存有效期等）
pub fn settings_file() -> PathBuf {
    if let Some(custom) = custom_settings_file() {
        return custom;
    }
    config_dir().join("settings.ini")
}

fn custom_settings_pointer() -> PathBuf {
    config_dir().join("custom_settings_path.txt")
}

/// 缓存配置重定向，避免绘制时反复读盘。
pub fn custom_settings_file() -> Option<PathBuf> {
    let mut guard = custom_settings_cell().lock().unwrap_or_else(|error| error.into_inner());
    guard.get_or_insert_with(read_custom_settings_pointer).clone()
}

fn custom_settings_cell() -> &'static std::sync::Mutex<Option<Option<PathBuf>>> {
    static CELL: std::sync::OnceLock<std::sync::Mutex<Option<Option<PathBuf>>>> =
        std::sync::OnceLock::new();
    CELL.get_or_init(|| std::sync::Mutex::new(None))
}

fn read_custom_settings_pointer() -> Option<PathBuf> {
    if let Ok(text) = std::fs::read_to_string(custom_settings_pointer()) {
        let t = text.trim();
        if !t.is_empty() {
            return Some(PathBuf::from(t));
        }
    }
    None
}

pub fn with_settings_file<T>(action: impl FnOnce(&std::path::Path) -> Result<T, String>) -> Result<T, String> {
    let mut guard = custom_settings_cell().lock().map_err(|_| "配置路径锁不可用".to_string())?;
    let path = guard.get_or_insert_with(read_custom_settings_pointer).clone()
        .unwrap_or_else(|| config_dir().join("settings.ini"));
    action(&path)
}

pub fn set_custom_settings_file(new_path: Option<PathBuf>, contents: &[u8]) -> Result<(), String> {
    let mut guard = custom_settings_cell().lock().map_err(|_| "配置路径锁不可用".to_string())?;
    relocate_settings(&custom_settings_pointer(), &config_dir().join("settings.ini"), new_path, contents, &mut guard)
}

fn relocate_settings(
    pointer: &std::path::Path,
    default: &std::path::Path,
    new_path: Option<PathBuf>,
    contents: &[u8],
    cached: &mut Option<Option<PathBuf>>,
) -> Result<(), String> {
    let destination = new_path.as_deref().unwrap_or(default);
    let text = destination.to_str().ok_or("配置保存路径必须是有效文本")?;
    if destination == pointer || text.trim().is_empty() {
        return Err("配置保存位置无效".into());
    }
    write_atomic(destination, contents)?;
    if new_path.is_some() {
        write_atomic(pointer, text.as_bytes())?;
    } else {
        match std::fs::remove_file(pointer) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(format!("清除配置文件重定向记录失败：{error}")),
        }
    }
    *cached = Some(new_path);
    Ok(())
}

/// 快照缓存文件（跨进程复用，省掉 690ms 的全量读取）
pub fn snapshot_cache_file() -> PathBuf {
    cache_dir().join("snapshot.cache")
}

/// 确保目录存在；失败时把错误说清楚（含目标路径）。
pub fn ensure_dir(p: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(p).map_err(|e| format!("无法创建目录 {}：{e}", p.display()))
}

pub fn directory_for_shell(p: &std::path::Path) -> Result<PathBuf, String> {
    let directory = if p.as_os_str().is_empty() { std::path::Path::new(".") } else { p };
    ensure_dir(directory)?;
    let physical = directory.canonicalize()
        .map_err(|error| format!("无法解析目录 {}：{error}", directory.display()))?;
    Ok(shell_path(&physical))
}

fn shell_path(p: &std::path::Path) -> PathBuf {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let prefix = p.components().next();
    let wide: Vec<u16> = p.as_os_str().encode_wide().collect();
    match prefix {
        Some(std::path::Component::Prefix(prefix)) => match prefix.kind() {
            std::path::Prefix::VerbatimDisk(_) => {
                PathBuf::from(std::ffi::OsString::from_wide(&wide[4..]))
            }
            std::path::Prefix::VerbatimUNC(_, _) => {
                let mut normalized = vec![b'\\' as u16, b'\\' as u16];
                normalized.extend_from_slice(&wide[8..]);
                PathBuf::from(std::ffi::OsString::from_wide(&normalized))
            }
            _ => p.to_path_buf(),
        },
        _ => p.to_path_buf(),
    }
}

pub fn replace_file(source: &std::path::Path, target: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use winapi::um::winbase::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
    if source.parent() != target.parent() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput,
            "Atomic replacement requires a temporary file in the destination directory"));
    }
    let physical = source.canonicalize()?;
    let parent = physical.parent().ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let name = target.file_name().ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let destination = parent.join(name);
    let source: Vec<u16> = physical.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = destination.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(),
        MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// 写入同目录临时文件后原子替换，失败时保留目标。
pub fn write_atomic(p: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            ensure_dir(parent)?;
        }
    }
    let mut tmp = p.to_path_buf().into_os_string();
    tmp.push(format!(".tmp{}", std::process::id()));
    let tmp = PathBuf::from(tmp);

    if let Err(e) = std::fs::write(&tmp, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("写临时文件 {} 失败：{e}", tmp.display()));
    }
    if let Err(e) = replace_file(&tmp, p) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("替换 {} 失败：{e}", p.display()));
    }
    Ok(())
}

/// 把用户给的路径解析成绝对路径。
///
/// - 空 / 只是文件名 → 落到备份目录
/// - 带目录 → 原样使用（相对路径按当前目录解析）
pub fn resolve_backup_path(arg: Option<&str>, default_name: &str) -> PathBuf {
    match arg {
        None => backup_dir().join(default_name),
        Some(s) => {
            let p = PathBuf::from(s);
            if p.parent().map(|x| x.as_os_str().is_empty()).unwrap_or(true) {
                // 只给了文件名
                backup_dir().join(p)
            } else {
                p
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scratch").join("path-tests");
            std::fs::create_dir_all(&root).unwrap();
            let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let path = root.join(format!("{}-{unique}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir(&path).unwrap();
            assert!(path.canonicalize().unwrap().starts_with(root.canonicalize().unwrap()));
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scratch").join("path-tests");
            if let (Ok(path), Ok(root)) = (self.0.canonicalize(), root.canonicalize()) {
                if path != root && path.starts_with(&root) { let _ = std::fs::remove_dir_all(path); }
            }
        }
    }

    #[test]
    fn shell_paths_preserve_unicode_and_unc_shares_without_verbatim_prefixes() {
        assert_eq!(shell_path(std::path::Path::new(r"\\?\D:\用户目录\日志")), PathBuf::from(r"D:\用户目录\日志"));
        assert_eq!(shell_path(std::path::Path::new(r"\\?\UNC\server\share\日志")), PathBuf::from(r"\\server\share\日志"));
        assert_eq!(shell_path(std::path::Path::new(r"D:\日志")), PathBuf::from(r"D:\日志"));
    }

    #[test]
    fn opening_a_missing_log_directory_creates_it_and_returns_an_explorer_path() {
        let scratch = Scratch::new();
        let directory = scratch.0.join("用户目录").join("logs");
        assert!(!directory.exists());
        let physical = directory_for_shell(&directory).unwrap();
        assert!(physical.is_absolute());
        assert!(physical.is_dir());
        assert!(!physical.as_os_str().to_string_lossy().starts_with(r"\\?\"));
        assert_eq!(physical.canonicalize().unwrap(), directory.canonicalize().unwrap());
    }

    #[test]
    fn a_file_at_the_requested_directory_is_reported_before_opening_explorer() {
        let scratch = Scratch::new();
        let file = scratch.0.join("not-a-directory");
        std::fs::write(&file, b"original").unwrap();
        assert!(directory_for_shell(&file).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"original");
    }

    #[test]
    fn atomic_replacement_uses_the_temporary_files_resolved_directory() {
        let scratch = Scratch::new();
        let directory = scratch.0.join("用户目录");
        std::fs::create_dir(&directory).unwrap();
        let source = directory.join("draft.tmp");
        let target = directory.join("feedback-history.dat");
        std::fs::write(&source, b"new").unwrap();
        std::fs::write(&target, b"old").unwrap();
        replace_file(&source, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert!(!source.exists());
    }

    #[test]
    fn cross_directory_replacement_is_rejected_without_touching_either_file() {
        let scratch = Scratch::new();
        let directory = scratch.0.join("other");
        std::fs::create_dir(&directory).unwrap();
        let source = scratch.0.join("draft.tmp");
        let target = directory.join("settings.ini");
        std::fs::write(&source, b"new").unwrap();
        std::fs::write(&target, b"old").unwrap();
        assert_eq!(replace_file(&source, &target).unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(std::fs::read(&source).unwrap(), b"new");
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
    }

    #[test]
    fn failed_settings_destination_preserves_the_active_pointer_and_cache() {
        let scratch = Scratch::new();
        let pointer = scratch.0.join("pointer");
        let original = scratch.0.join("old.ini");
        let destination = scratch.0.join("directory");
        std::fs::write(&original, b"old settings").unwrap();
        std::fs::write(&pointer, original.to_str().unwrap()).unwrap();
        std::fs::create_dir(&destination).unwrap();
        let mut cached = Some(Some(original.clone()));
        assert!(relocate_settings(&pointer, &scratch.0.join("default.ini"), Some(destination), b"new settings", &mut cached).is_err());
        assert_eq!(cached, Some(Some(original.clone())));
        assert_eq!(std::fs::read_to_string(&pointer).unwrap(), original.to_str().unwrap());
        assert_eq!(std::fs::read(original).unwrap(), b"old settings");
    }

    #[test]
    fn failed_pointer_reset_does_not_change_the_cached_settings_path() {
        use std::os::windows::fs::OpenOptionsExt;
        let scratch = Scratch::new();
        let pointer = scratch.0.join("pointer");
        let original = scratch.0.join("old.ini");
        std::fs::write(&pointer, original.to_str().unwrap()).unwrap();
        let held = std::fs::OpenOptions::new().read(true).share_mode(winapi::um::winnt::FILE_SHARE_READ).open(&pointer).unwrap();
        let mut cached = Some(Some(original.clone()));
        assert!(relocate_settings(&pointer, &scratch.0.join("default.ini"), None, b"settings", &mut cached).is_err());
        assert_eq!(cached, Some(Some(original.clone())));
        assert_eq!(std::fs::read_to_string(&pointer).unwrap(), original.to_str().unwrap());
        drop(held);
    }

    #[test]
    fn successful_relocation_and_reset_preserve_current_settings() {
        let scratch = Scratch::new();
        let pointer = scratch.0.join("pointer");
        let default = scratch.0.join("default.ini");
        let custom = scratch.0.join("custom").join("settings.ini");
        let mut cached = Some(None);
        relocate_settings(&pointer, &default, Some(custom.clone()), b"first", &mut cached).unwrap();
        assert_eq!(cached, Some(Some(custom.clone())));
        assert_eq!(std::fs::read_to_string(&pointer).unwrap(), custom.to_str().unwrap());
        assert_eq!(std::fs::read(&custom).unwrap(), b"first");
        relocate_settings(&pointer, &default, None, b"updated", &mut cached).unwrap();
        assert_eq!(cached, Some(None));
        assert!(!pointer.exists());
        assert_eq!(std::fs::read(default).unwrap(), b"updated");
    }

}
