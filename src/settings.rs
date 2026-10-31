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
    /// 窗口关闭时的行为："ask"（每次询问）、"minimize"（最小化到托盘）、"quit"（直接退出）。
    pub close_mode: String,
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
            close_mode: "ask".to_string(),
            theme: "system".to_string(),
        }
    }
}

impl Settings {
    fn parse_collect(text: &str) -> (Settings, Vec<String>) {
        let mut s = Settings::default();
        // 手改文件打错值不该让程序起不来，但也不能一声不响地当没看见：
        // 逐条记下来，由 load() 打到 stderr。
        let mut warnings: Vec<String> = Vec::new();
        // 旧版本同时写 close_to_tray 和 close_mode，两者表达同一件事。
        // 现在只认 close_mode；close_to_tray 仅在文件里没出现 close_mode 时兜底换算。
        let mut legacy_close: Option<bool> = None;
        let mut saw_close_mode = false;
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
                "cache_ttl_sec" => match v.parse::<u64>() {
                    Ok(n) => s.cache_ttl_sec = n,
                    Err(_) => warnings.push(format!("cache_ttl_sec={v} 不是非负整数，已忽略")),
                },
                "refresh_interval_sec" => match v.parse::<u64>() {
                    // 0 会让后台线程空转，钳一个下限出来
                    Ok(n) => s.refresh_interval_sec = n.max(5),
                    Err(_) => warnings.push(format!("refresh_interval_sec={v} 不是非负整数，已忽略")),
                },
                "tray_enabled" => match parse_bool(v) {
                    Some(b) => s.tray_enabled = b,
                    None => warnings.push(format!("tray_enabled={v} 不是布尔值（true/false），已忽略")),
                },
                "close_to_tray" => match parse_bool(v) {
                    Some(b) => legacy_close = Some(b),
                    None => warnings.push(format!("close_to_tray={v} 不是布尔值，已忽略")),
                },
                "close_mode" => {
                    let want = v.trim().to_ascii_lowercase();
                    if want == "ask" || want == "minimize" || want == "quit" {
                        s.close_mode = want;
                        saw_close_mode = true;
                    } else {
                        warnings.push(format!("close_mode={v} 只认 ask/minimize/quit，已忽略"));
                    }
                }
                "theme" => {
                    // 认 system/light/dark；写成 Light/Dark/System 也认，省得用户猜大小写。
                    // 打错的值不致命，但要留痕，否则「我明明设了深色」查不出原因。
                    let want = v.trim().to_ascii_lowercase();
                    if want == "light" || want == "dark" || want == "system" {
                        s.theme = want;
                    } else {
                        warnings.push(format!("theme={v} 只认 system/light/dark，已忽略"));
                    }
                }
                _ => {} // 未知键忽略，向后兼容（旧文件只有 cache_ttl_sec 一行也能读）
            }
        }
        if !saw_close_mode {
            if let Some(b) = legacy_close {
                s.close_mode = if b { "minimize" } else { "quit" }.to_string();
            }
        }
        (s, warnings)
    }

    /// 只取结果、丢弃告警。仅供单测使用（生产一律走 [`load`]）。
    #[cfg(test)]
    #[cfg(test)]
    fn parse(text: &str) -> Settings {
        Self::parse_collect(text).0
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
        s.push_str("# 关闭窗口动作：ask=每次询问，minimize=最小化到系统托盘，quit=直接退出。\n");
        s.push_str(&format!("close_mode={}\n\n", self.close_mode));
        s.push_str("# 界面配色：system（默认，跟随系统深浅色）/ light（浅色）/ dark（深色）。\n");
        s.push_str("# 程序里也能切换，切换后这里会被回写。\n");
        s.push_str(&format!("theme={}\n", self.theme));
        s
    }

    /// 把设置渲染成 ini 文本（供「更改设置文件位置」时把当前配置写到新位置）。
    pub fn render(s: &Settings) -> String {
        s.to_text()
    }

    /// 写出默认配置文件。GUI 首次启动 / `cache` 子命令会用到。
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

/// 读设置：文件不存在算正常（返回默认值）；读不动则**返回错误**，不再假装成功。
///
/// 以前是 `Err(_) => Settings::default()`，于是「权限被拒 / 文件被独占 /
/// 路径指向一个目录」全部变成「一切正常，用默认值」——用户改的设置凭空消失，
/// 界面上还看不出任何异常。现在把原因带出来。
pub fn try_load() -> Result<(Settings, Vec<String>), String> {
    let p = paths::settings_file();
    match std::fs::read_to_string(&p) {
        Ok(t) => Ok(Settings::parse_collect(&t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((Settings::default(), Vec::new())),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

/// 读设置，读不到就告警 + 用默认值（GUI / CLI 的通用入口）。
///
/// 告警走 stderr：GUI 下用户看不到，但日志与命令行调用能看到；
/// 静默失败比打一行日志危险得多。
pub fn load() -> Settings {
    match try_load() {
        Ok((s, warnings)) => {
            for w in warnings {
                eprintln!("⚠ 设置文件：{w}");
            }
            s
        }
        Err(e) => {
            eprintln!("⚠ 读取设置失败（{e}），本次用默认值。检查文件是否存在/可读。");
            Settings::default()
        }
    }
}

/// 写设置。失败时返回错误（调用方决定是警告还是致命）。
///
/// 走原子写：中途崩溃留下半份 settings.ini，下次启动会把整份配置读成默认值。
pub fn save(s: &Settings) -> Result<(), String> {
    let p = paths::settings_file();
    paths::write_atomic(&p, s.to_text().as_bytes())
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
    fn parse_collect_reports_bad_values() {
        // 手改文件打错值：不致命，但要留痕，否则「我明明设了」查不出原因。
        let (s, w) = Settings::parse_collect(
            "tray_enabled=maybe\ntheme=blue\ncache_ttl_sec=x\nclose_mode=hide\nunknown_key=1\n",
        );
        assert!(s.tray_enabled, "坏值不该改掉默认的 true");
        assert_eq!(s.theme, "system");
        assert_eq!(s.cache_ttl_sec, crate::snapshot::DEFAULT_TTL_SEC);
        assert_eq!(s.close_mode, "ask");
        assert_eq!(w.len(), 4, "四个坏值各留一条痕，未知键保持沉默：{w:?}");
    }


    #[test]
    fn parse_settings() {
        let s = Settings::parse("cache_ttl_sec=5\nunknown=1\n");
        assert_eq!(s.cache_ttl_sec, 5);
        // 缺失的键用默认值
        assert_eq!(s.refresh_interval_sec, 60);
        assert!(s.tray_enabled);
        assert_eq!(s.close_mode, "ask");
    }

    #[test]
    fn legacy_close_to_tray_maps_to_close_mode() {
        // 旧文件里的 close_to_tray 必须还能换算成 close_mode
        assert_eq!(Settings::parse("close_to_tray=true").close_mode, "minimize");
        assert_eq!(Settings::parse("close_to_tray=false").close_mode, "quit");
        // 两者同时出现时以 close_mode 为准（它更具体，能表达 ask）
        assert_eq!(
            Settings::parse("close_to_tray=true\nclose_mode=ask").close_mode,
            "ask"
        );
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
            close_mode: "quit".to_string(),
            theme: "dark".to_string(),
        };
        let back = Settings::parse(&s.to_text());
        assert_eq!(back.cache_ttl_sec, 45);
        assert_eq!(back.refresh_interval_sec, 20);
        assert!(!back.tray_enabled);
        assert_eq!(back.close_mode, "quit");
        assert_eq!(back.theme, "dark", "配色没有往返成功");
    }

    #[test]
    fn theme_accepts_case_and_rejects_junk() {
        // 手改文件写成 Dark 要认
        assert_eq!(Settings::parse("theme=Dark").theme, "dark");
        assert_eq!(Settings::parse("theme=DARK").theme, "dark");
        assert_eq!(Settings::parse("theme= light ").theme, "light");
        // 打错字不能把程序搞崩，退回默认「跟随系统」
        assert_eq!(Settings::parse("theme=blue").theme, "system");
        assert_eq!(Settings::parse("theme=").theme, "system");
        // 旧文件没有这个键也是默认「跟随系统」
        assert_eq!(Settings::parse("cache_ttl_sec=30").theme, "system");
        // system 也要能往返
        assert_eq!(Settings::parse("theme=System").theme, "system");
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
