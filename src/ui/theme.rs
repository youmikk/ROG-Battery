//! 浅色和深色界面的 ARGB 配色。

/// 一套配色。
///
/// 所有字段都是 GDI+ 的 `ARGB` 格式：`0xAARRGGBB`。
#[derive(Clone, Copy)]
pub struct Theme {
    /// 内容区底色。
    pub bg: u32,
    /// 左侧导航栏底色（比 `bg` 略深或略浅，靠一层区分）。
    pub bg_nav: u32,
    /// 输入框这类「凹陷」控件的底色。
    pub bg_sunken: u32,
    /// 卡片底色。
    pub card: u32,
    /// 卡片悬停时的底色（列表行用）。
    pub card_hover: u32,
    /// 细描边（卡片边缘、行分隔线）。
    pub border: u32,
    /// 强描边（输入框、按钮边缘）。
    pub border_strong: u32,
    /// 主文字。
    pub text: u32,
    /// 次要文字（说明、单位）。
    pub text2: u32,
    /// 更弱的文字（字段名、坐标轴刻度）。
    pub text3: u32,
    /// 强调色（选中项、主按钮、曲线）。
    pub accent: u32,
    /// 强调色上面的文字色（主按钮里的字）。
    pub accent_fg: u32,
    /// 强调色的淡底（选中行背景）。
    pub accent_soft: u32,
    /// 正向色（使用中、下降斜率）。
    pub green: u32,
    /// 正向色的淡底（胶囊标签）。
    pub green_soft: u32,
    /// 警告色。
    pub warn: u32,
    /// 警告色的淡底（警告条）。
    pub warn_soft: u32,
    /// 错误色。
    pub red: u32,
    /// 错误色的淡底。
    pub red_soft: u32,
    /// 滑块未填充的轨道色。
    pub track: u32,
}

/// 深色。
pub const DARK: Theme = Theme {
    bg: 0xFF1B201D,
    bg_nav: 0xFF181D1A,
    bg_sunken: 0xFF1B201D,
    card: 0xFF232A25,
    card_hover: 0xFF29322A,
    border: 0xFF323C34,
    border_strong: 0xFF455448,
    text: 0xFFE1E8DF,
    text2: 0xFFA0ACA2,
    text3: 0xFF7E8B81,
    accent: 0xFFA1C486,
    accent_fg: 0xFF1B201D,
    accent_soft: 0xFF303F2A,
    green: 0xFFA1C486,
    green_soft: 0xFF303F2A,
    warn: 0xFFFF_D24C,
    // rgba(255,210,76,0.13) → alpha 33
    warn_soft: 0x21FF_D24C,
    red: 0xFFFF_6B6B,
    red_soft: 0x21FF_6B6B,
    track: 0xFF333E35,
};

/// 浅色。
pub const LIGHT: Theme = Theme {
    bg: 0xFFF7F8F7,
    bg_nav: 0xFFF0F2EF,
    bg_sunken: 0xFFF7F8F7,
    card: 0xFFFFFFFF,
    card_hover: 0xFFF8FAF5,
    border: 0xFFE6EAE5,
    border_strong: 0xFFD5DDD2,
    text: 0xFF25302B,
    text2: 0xFF707972,
    text3: 0xFF89938B,
    accent: 0xFF507B3B,
    accent_fg: 0xFFFFFFFF,
    accent_soft: 0xFFE3ECD9,
    green: 0xFF6B9B54,
    green_soft: 0xFFE3ECD9,
    warn: 0xFF9D_5D00,
    // rgba(157,93,0,0.09) → alpha 23
    warn_soft: 0x179D_5D00,
    red: 0xFFC4_2B1C,
    red_soft: 0x17C4_2B1C,
    track: 0xFFEDF1EC,
};

/// 界面配色。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Light,
    Dark,
}

impl Mode {
    /// 两套配色里取一套。
    pub fn theme(self) -> &'static Theme {
        match self {
            Mode::Light => &LIGHT,
            Mode::Dark => &DARK,
        }
    }

    /// 反色。给标题栏的主题切换按钮用。
    #[cfg(test)]
    pub fn flipped(self) -> Mode {
        match self {
            Mode::Light => Mode::Dark,
            Mode::Dark => Mode::Light,
        }
    }

    /// 存进 `settings.ini` 的字符串。
    #[cfg(test)]
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Light => "light",
            Mode::Dark => "dark",
        }
    }

    /// 从 `settings.ini` 读回来。认不出的一律当浅色（默认）。
    ///
    /// 注意：`"system"` 在这里**不**解析成具体配色 —— 那需要一个注册表
    /// 查询，属于运行期行为。用 [`resolve`] 得到实际配色。
    ///
    /// 现在运行期走 [`resolve`]，这个只剩单测在用，留着当解析规则的说明书。
    #[cfg(test)]
    pub fn parse(s: &str) -> Mode {
        match s.trim().to_ascii_lowercase().as_str() {
            "dark" => Mode::Dark,
            _ => Mode::Light,
        }
    }
}

/// 配色偏好（`settings.ini` 的 `theme` 键）。
///
/// 三种值：`system` / `light` / `dark`。默认 `system`。
pub const PREF_SYSTEM: &str = "system";
pub const PREF_LIGHT: &str = "light";
pub const PREF_DARK: &str = "dark";

/// 把偏好归一化成合法的三段值。认不出的一律当「跟随系统」。
pub fn normalize_pref(s: &str) -> &'static str {
    match s.trim().to_ascii_lowercase().as_str() {
        "light" => PREF_LIGHT,
        "dark" => PREF_DARK,
        _ => PREF_SYSTEM,
    }
}

/// 读系统「应用」深浅色（`AppsUseLightTheme`，缺省回落到 `SystemUsesLightTheme`）。
///
/// 值为 1 → 浅色。读不到默认深色（Windows 默认是深色任务栏/深色应用）。
pub fn system_is_light() -> bool {
    use crate::win32::{RegCloseKey, RegOpenKeyExW, RegQueryValueExW};
    const HKCU: usize = 0xFFFF_FFFF_8000_0001;
    const KEY_QUERY_VALUE: u32 = 0x0001;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    fn read_u32(value_name: &str) -> Option<u32> {
        let sub = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        let mut hk: usize = 0;
        unsafe {
            if RegOpenKeyExW(HKCU, sub.as_ptr(), 0, KEY_QUERY_VALUE, &mut hk) != 0 {
                return None;
            }
            let name = wide(value_name);
            let mut ty: u32 = 0;
            let mut val: u32 = 0;
            let mut blen: u32 = std::mem::size_of::<u32>() as u32;
            let rc = RegQueryValueExW(
                hk,
                name.as_ptr(),
                std::ptr::null_mut(),
                &mut ty,
                &mut val as *mut u32 as *mut u8,
                &mut blen,
            );
            RegCloseKey(hk);
            if rc == 0 {
                Some(val)
            } else {
                None
            }
        }
    }

    match read_u32("AppsUseLightTheme").or_else(|| read_u32("SystemUsesLightTheme")) {
        Some(v) => v == 1,
        None => false,
    }
}

/// 偏好 → 实际配色。
pub fn resolve(pref: &str) -> Mode {
    match normalize_pref(pref) {
        PREF_LIGHT => Mode::Light,
        PREF_DARK => Mode::Dark,
        _ => {
            if system_is_light() {
                Mode::Light
            } else {
                Mode::Dark
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 两套配色的强调色不一样() {
        // 浅色和深色如果强调色相同，说明抄错了 —— 这两个值在原型的
        // CSS 里就是不同的（#0067C0 vs #4CC2FF）。
        assert_ne!(LIGHT.accent, DARK.accent);
        assert_ne!(LIGHT.bg, DARK.bg);
        assert_ne!(LIGHT.text, DARK.text);
    }

    #[test]
    fn 所有颜色都是不透明的_除了刻意半透明的() {
        // 只有边框、次要文字、淡底这几类允许带 alpha。
        // 底色、卡片、文字主色必须完全不透明 —— 否则 BitBlt 到屏幕上
        // 会露出底下没画过的东西。
        for (name, c) in [
            ("bg", LIGHT.bg),
            ("card", LIGHT.card),
            ("text", LIGHT.text),
            ("accent", LIGHT.accent),
        ] {
            assert_eq!(c >> 24, 0xFF, "{name} 必须不透明");
        }
        for (name, c) in [
            ("bg", DARK.bg),
            ("card", DARK.card),
            ("text", DARK.text),
            ("accent", DARK.accent),
        ] {
            assert_eq!(c >> 24, 0xFF, "{name} 必须不透明");
        }
    }

    #[test]
    fn mode_解析与反色() {
        assert_eq!(Mode::parse("dark"), Mode::Dark);
        assert_eq!(Mode::parse("DARK"), Mode::Dark);
        assert_eq!(Mode::parse(" dark "), Mode::Dark);
        assert_eq!(Mode::parse("light"), Mode::Light);
        assert_eq!(Mode::parse(""), Mode::Light);
        assert_eq!(Mode::parse("随便什么"), Mode::Light);
        assert_eq!(Mode::Dark.flipped(), Mode::Light);
        assert_eq!(Mode::Light.flipped(), Mode::Dark);
        assert_eq!(Mode::Light.as_str(), "light");
        assert_eq!(Mode::Dark.as_str(), "dark");
    }
}
