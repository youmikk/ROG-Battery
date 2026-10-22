//! 设置文件：`%APPDATA%\rog-mouse-ctl\settings.ini`
//!
//! 用户要求「需要安装的方式」+「配置放系统标准位置」，所以设置不能放 exe 同目录。
//!
//! 格式是极简的 `key=value`，不引第三方依赖也能解析：
//!
//! ```ini
//! cache_ttl_sec=30
//! cache_enabled=1
//! ```
//!
//! 文件不存在时用默认值（**不会**因为配置缺失而报错）。

use crate::paths;

/// 全部可配置项。加新项时记得同时改 [`Settings::default`] 和 [`Settings::parse`]。
#[derive(Clone, Debug)]
pub struct Settings {
    /// 快照缓存有效期（秒）。0 表示每次都重读设备。
    pub cache_ttl_sec: u64,
    /// 界面后台主动刷新的间隔（秒）。
    ///
    /// **和 `cache_ttl_sec` 不是一回事**：`cache_ttl_sec` 是「多快允许复用缓存」，
    /// 这个 `refresh_interval_sec` 是「多久主动去读一次设备」。
    pub refresh_interval_sec: u64,
    /// 是否启用托盘图标。
    pub tray_enabled: bool,
    /// 点关闭按钮时最小化到托盘（true）还是直接退出（false）。
    pub close_to_tray: bool,
    /// 界面配色：`"light"` 或 `"dark"`。
    ///
    /// 存成 `String` 而不是 [`crate::ui::theme::Mode`] 有两个原因：
    /// ① 这个文件是给人手改的，写 `theme=dark` 比 `theme=Dark` 自然；
    /// ② `settings.rs` 不该依赖 UI 模块 —— CLI 子命令也要能读设置，
    /// 而 UI 模块会链 gdiplus，把 CLI 拖进多余依赖。
    pub theme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            cache_ttl_sec: crate::snapshot::DEFAULT_TTL_SEC,
            // 全量 19 帧约 591ms，只读 3 帧约 150ms；60 秒一次的额外耗电是
            // 0.07 %/天量级，可忽略。取 15 秒唯一的代价是鼠标休眠时更频繁地
            // 撞上超时（一次 MISS 要等满 1200ms），所以默认 60 秒。
            refresh_interval_sec: 60,
            tray_enabled: true,
            close_to_tray: true,
            theme: "light".to_string(),
        }
    }
}

impl Settings {
    fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim();
            match k.trim() {
                "cache_ttl_sec" => {
                    if let Ok(n) = v.parse::<u64>() {
                        s.cache_ttl_sec = n;
                    }
                }
                "refresh_interval_sec" => {
                    if let Ok(n) = v.parse::<u64>() {
                        // 0 会让后台线程空转，钳一个下限出来
                        s.refresh_interval_sec = n.max(5);
                    }
                }
                "tray_enabled" => {
                    if let Some(b) = parse_bool(v) {
                        s.tray_enabled = b;
                    }
                }
                "close_to_tray" => {
                    if let Some(b) = parse_bool(v) {
                        s.close_to_tray = b;
                    }
                }
                "theme" => {
                    // 只认 light/dark，其它一律当默认（不报错 —— 手改文件打错字
                    // 不该让程序起不来）。写成 Light/Dark 也认，省得用户猜大小写。
                    let want = v.trim().to_ascii_lowercase();
                    if want == "light" || want == "dark" {
                        s.theme = want;
                    }
                }
                _ => {} // 未知键忽略，向后兼容（旧文件只有 cache_ttl_sec 一行也能读）
            }
        }
        s
    }

    pub fn to_text(&self) -> String {
        let mut s = String::new();
        s.push_str("# rog-mouse-ctl 设置\n");
        s.push_str("# 位置：%APPDATA%\\rog-mouse-ctl\\settings.ini\n");
        s.push_str("# 删除本文件即恢复默认值。改完重启程序生效。\n\n");
        s.push_str("# 快照缓存有效期（秒）。一次读全 19 帧约 591ms，缓存命中则零成本。\n");
        s.push_str("# 设为 0 表示每次都重新读设备。\n");
        s.push_str(&format!("cache_ttl_sec={}\n\n", self.cache_ttl_sec));
        s.push_str("# 界面后台主动刷新的间隔（秒），最小 5。\n");
        s.push_str("# 注意这和上面的缓存有效期不是一回事：缓存管「多快能复用旧数据」，\n");
        s.push_str("# 这里管「多久真去读一次设备」。鼠标休眠时会读不到，\n");
        s.push_str("# 程序应显示「休眠中」而不是报错 —— 动一下鼠标就能唤醒。\n");
        s.push_str(&format!("refresh_interval_sec={}\n\n", self.refresh_interval_sec));
        s.push_str("# 是否启用托盘图标（true/false）。\n");
        s.push_str(&format!("tray_enabled={}\n\n", self.tray_enabled));
        s.push_str("# 点窗口关闭按钮时：true=最小化到托盘，false=直接退出。\n");
        s.push_str("# 只在 tray_enabled=true 时才有意义。\n");
        s.push_str(&format!("close_to_tray={}\n\n", self.close_to_tray));
        s.push_str("# 界面配色：light（默认，浅色）或 dark（深色）。\n");
        s.push_str("# 程序里也能切换，切换后这里会被回写。\n");
        s.push_str(&format!("theme={}\n", self.theme));
        s
    }

    /// 写出默认配置文件。GUI 首次启动 / `cache` 子命令会用到。
    #[allow(dead_code)]
    pub fn write_default_if_missing() {
        if paths::settings_file().exists() {
            return;
        }
        let _ = save(&Settings::default());
    }
}

/// 容忍 `true`/`false`/`1`/`0`/`yes`/`no`（大小写不敏感）。
/// 手写配置文件的人不会记得程序到底认哪种写法。
fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// 读设置。文件不存在或读不动都用默认值，**不报错**。
pub fn load() -> Settings {
    let p = paths::settings_file();
    match std::fs::read_to_string(&p) {
        Ok(t) => Settings::parse(&t),
        Err(_) => Settings::default(),
    }
}

/// 写设置。失败时返回错误（调用方决定是警告还是致命）。
#[allow(dead_code)]
pub fn save(s: &Settings) -> Result<(), String> {
    let dir = paths::config_dir();
    paths::ensure_dir(&dir)?;
    let p = paths::settings_file();
    std::fs::write(&p, s.to_text()).map_err(|e| format!("写 {} 失败：{e}", p.display()))
}

/// 用于备份文件名的**本地**时间戳 `YYYYMMDD-HHMMSS`。
///
/// 走 kernel32 的 `GetLocalTime` 而不是自己算 UTC：
/// 之前用 `UNIX_EPOCH` 换算，22:26 的备份文件名会写成 `142627`，
/// 和用户在资源管理器里看到的时间差 8 小时，非常容易误判成「备份是旧的」。
pub fn timestamp_for_filename() -> String {
    let st = local_time();
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        st.year, st.month, st.day, st.hour, st.minute, st.second
    )
}

/// 给人看的本地时间 `YYYY-MM-DD HH:MM:SS`，写进备份文件头方便事后对照。
pub fn timestamp_for_humans() -> String {
    let st = local_time();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        st.year, st.month, st.day, st.hour, st.minute, st.second
    )
}

fn local_time() -> crate::win32::SystemTime {
    let mut st = crate::win32::SystemTime::default();
    // SAFETY: `GetLocalTime` 只写 `*mut SYSTEMTIME`，指针来自栈上具名变量。
    unsafe { crate::win32::GetLocalTime(&mut st) };
    st
}

/// 把 unix 秒转成本地时间 `MM-DD HH:MM`。
///
/// 用 `FileTimeToLocalFileTime` + `FileTimeToSystemTime` 走系统换算，
/// 不自己实现时区规则 —— 那样夏令时/时区改动都会算错。
/// 失败返回 `None`，调用方自己退化成显示 unix 秒。
pub fn format_unix_local(unix: i64) -> Option<String> {
    // unix 秒 → FILETIME（1601-01-01 起的 100ns 计数）
    const EPOCH_DIFF_SEC: i64 = 11_644_473_600; // 1601-01-01 到 1970-01-01
    let secs = unix.checked_add(EPOCH_DIFF_SEC)?;
    if secs < 0 {
        return None;
    }
    let ft_ticks = (secs as u64).checked_mul(10_000_000)?;

    let mut local_ft = crate::win32::FileTime::default();
    local_ft.low = (ft_ticks & 0xFFFF_FFFF) as u32;
    local_ft.high = (ft_ticks >> 32) as u32;

    let mut local = crate::win32::FileTime::default();
    let mut st = crate::win32::SystemTime::default();
    // SAFETY: 三个指针都指向栈上具名变量，两个 API 只写输出参数。
    unsafe {
        if crate::win32::FileTimeToLocalFileTime(&local_ft, &mut local) == 0 {
            return None;
        }
        if crate::win32::FileTimeToSystemTime(&local, &mut st) == 0 {
            return None;
        }
    }
    Some(format!(
        "{:02}-{:02} {:02}:{:02}",
        st.month, st.day, st.hour, st.minute
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_settings() {
        let s = Settings::parse("cache_ttl_sec=5\nunknown=1\n");
        assert_eq!(s.cache_ttl_sec, 5);
        // 缺失的键用默认值
        assert_eq!(s.refresh_interval_sec, 60);
        assert!(s.tray_enabled);
        assert!(s.close_to_tray);
    }

    #[test]
    fn parse_empty_uses_default() {
        let s = Settings::parse("");
        assert_eq!(s.cache_ttl_sec, crate::snapshot::DEFAULT_TTL_SEC);
    }

    #[test]
    fn old_file_still_parses() {
        // 旧版只有一行 cache_ttl_sec，加了新键以后必须还能读
        let s = Settings::parse("# rog-mouse-ctl 设置\ncache_ttl_sec=30\n");
        assert_eq!(s.cache_ttl_sec, 30);
        assert_eq!(s.refresh_interval_sec, 60);
    }

    #[test]
    fn bool_forms_accepted() {
        for (text, want) in [
            ("tray_enabled=true", true),
            ("tray_enabled=TRUE", true),
            ("tray_enabled=1", true),
            ("tray_enabled=yes", true),
            ("tray_enabled=on", true),
            ("tray_enabled=false", false),
            ("tray_enabled=0", false),
            ("tray_enabled=no", false),
            ("tray_enabled=off", false),
        ] {
            let s = Settings::parse(text);
            assert_eq!(s.tray_enabled, want, "解析 {text:?} 得到错误结果");
        }
    }

    #[test]
    fn refresh_interval_has_floor() {
        let s = Settings::parse("refresh_interval_sec=0");
        assert_eq!(s.refresh_interval_sec, 5, "0 会让后台线程空转，必须钳住");
        let s = Settings::parse("refresh_interval_sec=1");
        assert_eq!(s.refresh_interval_sec, 5);
    }

    #[test]
    fn roundtrip_through_text() {
        let s = Settings {
            cache_ttl_sec: 45,
            refresh_interval_sec: 20,
            tray_enabled: false,
            close_to_tray: false,
            theme: "dark".to_string(),
        };
        let back = Settings::parse(&s.to_text());
        assert_eq!(back.cache_ttl_sec, 45);
        assert_eq!(back.refresh_interval_sec, 20);
        assert!(!back.tray_enabled);
        assert!(!back.close_to_tray);
        assert_eq!(back.theme, "dark", "配色没有往返成功");
    }

    #[test]
    fn theme_accepts_case_and_rejects_junk() {
        // 手改文件写成 Dark 要认
        assert_eq!(Settings::parse("theme=Dark").theme, "dark");
        assert_eq!(Settings::parse("theme=DARK").theme, "dark");
        assert_eq!(Settings::parse("theme= light ").theme, "light");
        // 打错字不能把程序搞崩，退回默认浅色
        assert_eq!(Settings::parse("theme=blue").theme, "light");
        assert_eq!(Settings::parse("theme=").theme, "light");
        // 旧文件没有这个键也是浅色
        assert_eq!(Settings::parse("cache_ttl_sec=30").theme, "light");
    }

    #[test]
    fn timestamp_shape() {
        let t = timestamp_for_filename();
        // 形如 20261021-223000
        assert_eq!(t.len(), 15, "时间戳应是 8 位日期 + '-' + 6 位时间：{t}");
        assert_eq!(&t[8..9], "-");
        assert!(t[..8].chars().all(|c| c.is_ascii_digit()));
        assert!(t[9..].chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn unix_epoch_formats() {
        // 1970-01-01 00:00 UTC = 北京时间 08:00（这台机器在 UTC+8）
        // 不断言具体时刻，只断言格式和「能算出来」
        let s = format_unix_local(0).expect("1970 应该能换算");
        assert_eq!(s.len(), 11, "格式应是 MM-DD HH:MM：{s}");
        assert_eq!(&s[2..3], "-");
        assert_eq!(&s[5..6], " ");
        assert_eq!(&s[8..9], ":");
    }
}
