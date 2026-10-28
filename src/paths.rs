//! 标准目录解析。
//!
//! 用户明确要求「需要安装的方式」、配置放系统标准位置，因此这里**不**把任何东西写到 exe 同目录。
//!
//! - 配置：`%APPDATA%\rog-mouse-ctl\`（漫游，跟随用户）
//! - 缓存：`%LOCALAPPDATA%\rog-mouse-ctl\`（本机，可随时删）
//! - 备份：默认 `%USERPROFILE%\Documents\rog-mouse-ctl\backups\`（用户数据，跟文档走）
//!
//! MSIX 打包后进程仍能看到这三个环境变量，因此同一套逻辑在安装版和直接运行版都成立。

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

/// 读取用户自定义的配置文件路径（若已配置且非空）。
pub fn custom_settings_file() -> Option<PathBuf> {
    if let Ok(text) = std::fs::read_to_string(custom_settings_pointer()) {
        let t = text.trim();
        if !t.is_empty() {
            return Some(PathBuf::from(t));
        }
    }
    None
}

/// 设定或重置自定义配置文件路径。
pub fn set_custom_settings_file(new_path: Option<PathBuf>) -> Result<(), String> {
    let pointer = custom_settings_pointer();
    match new_path {
        Some(p) => {
            let _ = ensure_dir(&config_dir());
            std::fs::write(&pointer, p.to_string_lossy().as_bytes())
                .map_err(|e| format!("写入配置文件重定向记录失败：{e}"))
        }
        None => {
            let _ = std::fs::remove_file(&pointer);
            Ok(())
        }
    }
}

/// 快照缓存文件（跨进程复用，省掉 690ms 的全量读取）
pub fn snapshot_cache_file() -> PathBuf {
    cache_dir().join("snapshot.cache")
}

/// 确保目录存在；失败时把错误说清楚（含目标路径）。
pub fn ensure_dir(p: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(p).map_err(|e| format!("无法创建目录 {}：{e}", p.display()))
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
