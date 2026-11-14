
use crate::paths;

/// 全部可配置项。加新项时记得同时改 [`Settings::default`] 和 [`Settings::parse`]。
#[derive(Clone, Debug)]
pub struct Settings {
    pub device_tray: std::collections::BTreeMap<String, crate::tray_prefs::DeviceTray>,
    pub device_names: std::collections::BTreeMap<String, String>,
    /// 用户拖拽后的设备顺序。未知或已移除的设备会被忽略。
    pub device_order: Vec<String>,
    pub tray_device: Option<String>,
    pub tray_device_name: String,
    /// 快照缓存有效期（秒）。0 表示每次都重读设备。
    pub cache_ttl_sec: u64,
    /// 界面后台主动刷新的间隔（秒）。
    ///
    /// **和 `cache_ttl_sec` 不是一回事**：`cache_ttl_sec` 是「多快允许复用缓存」，
    /// 这个 `refresh_interval_sec` 是「多久主动去读一次设备」。
    pub refresh_interval_sec: u64,
    /// 是否启用托盘图标。
    pub tray_enabled: bool,
    /// MSIX 版是否启用 Windows 低电量通知。绿色版不显示也不调用。
    pub notify_low_battery: bool,
    /// 窗口关闭时的行为："ask"（每次询问）、"minimize"（最小化到托盘）、"quit"（直接退出）。
    pub close_mode: String,
    /// 界面配色：`"light"` 或 `"dark"`。
    ///
    /// 存成 `String` 而不是 [`crate::ui::theme::Mode`] 有两个原因：
    /// ① 这个文件是给人手改的，写 `theme=dark` 比 `theme=Dark` 自然；
    /// ② `settings.rs` 不该依赖 UI 模块 —— CLI 子命令也要能读设置，
    /// 而 UI 模块会链 gdiplus，把 CLI 拖进多余依赖。
    pub theme: String,
    pub language: crate::i18n::Language,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            device_tray: Default::default(),
            device_names: Default::default(),
            device_order: Vec::new(),
            tray_device: None,
            tray_device_name: String::new(),
            cache_ttl_sec: crate::snapshot::DEFAULT_TTL_SEC,
            // 全量 19 帧约 591ms，只读 3 帧约 150ms；60 秒一次的额外耗电是
            // 0.07 %/天量级，可忽略。取 15 秒唯一的代价是鼠标休眠时更频繁地
            // 撞上超时（一次 MISS 要等满 1200ms），所以默认 60 秒。
            refresh_interval_sec: 60,
            tray_enabled: true,
            notify_low_battery: true,
            close_mode: "ask".to_string(),
            theme: "system".to_string(),
            language: crate::i18n::Language::System,
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
                "tray_device" => {
                    if v.len() == 16 && v.bytes().all(|b| b.is_ascii_hexdigit()) {
                        s.tray_device = Some(v.to_ascii_lowercase());
                    }
                }
                "tray_device_name" => s.tray_device_name = v.to_string(),
                "device_order" => {
                    let mut seen = std::collections::BTreeSet::new();
                    s.device_order = v
                        .split(',')
                        .map(str::trim)
                        .filter(|id| {
                            id.len() == 16
                                && id.bytes().all(|b| b.is_ascii_hexdigit())
                                && seen.insert(id.to_ascii_lowercase())
                        })
                        .map(str::to_ascii_lowercase)
                        .collect();
                }
                "language" => match crate::i18n::Language::parse(v) {
                    Some(language) => s.language = language,
                    None => warnings.push(format!("language={v} 只认 system/zh-CN/en，已忽略")),
                },
                "cache_ttl_sec" => match v.parse::<u64>() {
                    Ok(n) => s.cache_ttl_sec = n,
                    Err(_) => warnings.push(format!("cache_ttl_sec={v} 不是非负整数，已忽略")),
                },
                "refresh_interval_sec" => match v.parse::<u64>() {
                    // 0 会让后台线程空转，钳一个下限出来
                    Ok(n) => s.refresh_interval_sec = n.max(5),
                    Err(_) => {
                        warnings.push(format!("refresh_interval_sec={v} 不是非负整数，已忽略"))
                    }
                },
                "tray_enabled" => match parse_bool(v) {
                    Some(b) => s.tray_enabled = b,
                    None => {
                        warnings.push(format!("tray_enabled={v} 不是布尔值（true/false），已忽略"))
                    }
                },
                "notify_low_battery" => match parse_bool(v) {
                    Some(b) => s.notify_low_battery = b,
                    None => warnings.push(format!(
                        "notify_low_battery={v} 不是布尔值（true/false），已忽略"
                    )),
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
                key if key.starts_with("device_name.") => {
                    let id = &key[12..];
                    if id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
                        if s.set_device_name(&id.to_ascii_lowercase(), v).is_err() {
                            warnings.push("设备名称无效，已忽略".to_string());
                        }
                    }
                }
                key if key.starts_with("device_tray.") => {
                    let id = &key[12..];
                    if id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
                        if let Some(pref) = crate::tray_prefs::DeviceTray::parse(v) {
                            s.device_tray.insert(id.to_ascii_lowercase(), pref);
                        } else {
                            warnings.push("设备托盘配置无效，已忽略".to_string());
                        }
                    }
                }
                _ => {}
            }
        }
        if !saw_close_mode {
            if let Some(b) = legacy_close {
                s.close_mode = if b { "minimize" } else { "quit" }.to_string();
            }
        }
        (s, warnings)
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
        s.push_str("# Microsoft Store 版本是否启用 Windows 低电量通知（true/false）。\n");
        s.push_str(&format!(
            "notify_low_battery={}\n\n",
            self.notify_low_battery
        ));
        s.push_str("# 关闭窗口动作：ask=每次询问，minimize=最小化到系统托盘，quit=直接退出。\n");
        s.push_str(&format!("close_mode={}\n\n", self.close_mode));
        s.push_str("# 界面配色：system（默认，跟随系统深浅色）/ light（浅色）/ dark（深色）。\n");
        s.push_str("# 程序里也能切换，切换后这里会被回写。\n");
        s.push_str(&format!("theme={}\n", self.theme));
        s.push_str("# Language: system / zh-CN / en.\n");
        s.push_str(&format!("language={}\n", self.language.code()));
        if let Some(key) = &self.tray_device {
            s.push_str(&format!("tray_device={key}\n"));
            s.push_str(&format!(
                "tray_device_name={}\n",
                self.tray_device_name.replace(['\r', '\n'], " ")
            ));
        }
        for (key, pref) in &self.device_tray {
            s.push_str(&format!("device_tray.{key}={}\n", pref.text()));
        }
        for (key, name) in &self.device_names {
            s.push_str(&format!("device_name.{key}={name}\n"));
        }
        if !self.device_order.is_empty() {
            s.push_str(&format!("device_order={}\n", self.device_order.join(",")));
        }
        s
    }

    pub fn device_name<'a>(&'a self, device: &'a crate::device::DeviceReading) -> &'a str {
        self.device_names
            .get(&device.key)
            .map(String::as_str)
            .unwrap_or(&device.name)
    }

    pub fn device_order_index(&self, key: &str) -> Option<usize> {
        self.device_order.iter().position(|candidate| candidate == key)
    }

    pub fn set_device_order(&mut self, keys: impl IntoIterator<Item = String>) {
        let mut seen = std::collections::BTreeSet::new();
        self.device_order = keys
            .into_iter()
            .map(|key| key.to_ascii_lowercase())
            .filter(|key| {
                key.len() == 16
                    && key.bytes().all(|b| b.is_ascii_hexdigit())
                    && seen.insert(key.clone())
            })
            .collect();
    }

    pub fn tray_name(&self) -> &str {
        self.tray_device
            .as_ref()
            .and_then(|key| self.device_names.get(key))
            .map(String::as_str)
            .unwrap_or(&self.tray_device_name)
    }

    pub fn set_device_name(&mut self, key: &str, name: &str) -> Result<(), &'static str> {
        if name.chars().any(char::is_control) || name.trim().chars().count() > 64 {
            return Err(crate::tr!("名称最多 64 个字符，不能包含换行或控制字符"));
        }
        let name = name.trim();
        if name.is_empty() {
            self.device_names.remove(key);
        } else {
            self.device_names.insert(key.to_string(), name.to_string());
        }
        Ok(())
    }

    pub fn tray_for(&self, key: &str) -> crate::tray_prefs::DeviceTray {
        self.device_tray.get(key).copied().unwrap_or_default()
    }

    /// 所有界面统一使用手动类型；只有自动识别才读取检测结果。
    pub fn device_kind(
        &self,
        device: &crate::device::DeviceReading,
    ) -> crate::tray_prefs::TrayKind {
        self.tray_for(&device.key).kind.resolve(device.family)
    }

    /// 首次选定后固定身份；设备断开或顺序变化时不自动换成另一台。
    pub fn ensure_tray_device(&mut self, readings: &[crate::device::DeviceReading]) -> bool {
        let chosen = if let Some(key) = &self.tray_device {
            readings.iter().find(|r| &r.key == key)
        } else {
            readings.iter().min_by_key(|r| {
                (
                    !self.tray_for(&r.key).enabled,
                    self.device_kind(r).order(),
                    r.key.clone(),
                )
            })
        };
        let Some(chosen) = chosen else { return false };
        if self.tray_device.as_ref() == Some(&chosen.key) && self.tray_device_name == chosen.name {
            return false;
        }
        self.tray_device = Some(chosen.key.clone());
        self.tray_device_name = chosen.name.clone();
        true
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
