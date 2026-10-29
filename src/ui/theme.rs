//! 界面配色令牌。
//!
//! # 为什么颜色要集中在这里
//!
//! 原来的 `gui.rs` 用 nwg 的原生控件，颜色由系统主题决定，我一行都不用管。
//! 现在整页是 GDI+ 自绘，**每一个像素的颜色都是我决定的** ——
//! 如果这些 ARGB 常量散落在五个页面的绘制代码里，改一次配色要翻五百行。
//!
//! 所以这里是一个纯数据的 `Theme`，浅色/深色各一份，页面绘制函数只认
//! `&Theme`，不知道自己是浅色还是深色。
//!
//! # 值从哪来的
//!
//! 直接照搬网页原型 `docs\mockups\ui-prototype.html` 的 CSS 变量
//! （`--bg` / `--card` / `--accent` …）。原型是用户看过并定稿的，
//! 改这里的数字就等于改设计稿，**不要凭手感调**。
//!
//! # 半透明是怎么实现的
//!
//! CSS 里的 `rgba(0, 0, 0, 0.065)` 这类边框色，在这里写成
//! `0x11_00_00_00`（alpha = 0.065 × 255 ≈ 17）。
//!
//! GDI+ 画到 32bpp 的 DIB 上时会真的做 alpha 混合 —— 前提是
//! **画笔/画刷本身就是 ARGB**，且目标表面是 32bpp。
//! `gfx::Canvas` 用的是 `CreateDIBSection` + `biBitCount = 32`，
//! 所以这一步是成立的（探针 `scratch\gdiplus-probe\windowdc.rs` 里
//! 用 alpha = 0x66 的渐变填充验证过）。

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

/// 深色（网页原型 `:root` 那一套）。
pub const DARK: Theme = Theme {
    bg: 0xFF20_2020,
    bg_nav: 0xFF27_2727,
    bg_sunken: 0xFF1B_1B1B,
    card: 0xFF2C_2C2C,
    card_hover: 0xFF32_3232,
    // rgba(255,255,255,0.075) → alpha 19
    border: 0x13FF_FFFF,
    // rgba(255,255,255,0.14) → alpha 36
    border_strong: 0x24FF_FFFF,
    text: 0xFFFF_FFFF,
    // rgba(255,255,255,0.62) → alpha 158
    text2: 0x9EFF_FFFF,
    // rgba(255,255,255,0.38) → alpha 97
    text3: 0x61FF_FFFF,
    accent: 0xFF4C_C2FF,
    accent_fg: 0xFF00_293A,
    // rgba(76,194,255,0.14) → alpha 36
    accent_soft: 0x244C_C2FF,
    green: 0xFF6C_CB5F,
    // rgba(108,203,95,0.15) → alpha 38
    green_soft: 0x266C_CB5F,
    warn: 0xFFFF_D24C,
    // rgba(255,210,76,0.13) → alpha 33
    warn_soft: 0x21FF_D24C,
    red: 0xFFFF_6B6B,
    red_soft: 0x21FF_6B6B,
    // rgba(255,255,255,0.22) → alpha 56
    track: 0x38FF_FFFF,
};

/// 浅色（**默认**，用户定的是默认浅色）。
pub const LIGHT: Theme = Theme {
    bg: 0xFFF3_F3F3,
    bg_nav: 0xFFEA_EAEA,
    bg_sunken: 0xFFE2_E2E2,
    card: 0xFFFF_FFFF,
    card_hover: 0xFFFA_FAFA,
    // rgba(0,0,0,0.065) → alpha 17
    border: 0x1100_0000,
    // rgba(0,0,0,0.12) → alpha 31
    border_strong: 0x1F00_0000,
    text: 0xFF1B_1B1B,
    // rgba(0,0,0,0.60) → alpha 153
    text2: 0x9900_0000,
    // rgba(0,0,0,0.38) → alpha 97
    text3: 0x6100_0000,
    accent: 0xFF00_67C0,
    accent_fg: 0xFFFF_FFFF,
    // rgba(0,103,192,0.10) → alpha 26
    accent_soft: 0x1A00_67C0,
    green: 0xFF0F_7B0F,
    // rgba(15,123,15,0.10) → alpha 26
    green_soft: 0x1A0F_7B0F,
    warn: 0xFF9D_5D00,
    // rgba(157,93,0,0.09) → alpha 23
    warn_soft: 0x179D_5D00,
    red: 0xFFC4_2B1C,
    red_soft: 0x17C4_2B1C,
    // rgba(0,0,0,0.28) → alpha 71
    track: 0x4700_0000,
};

/// 界面用哪一套配色。
///
/// 用 `bool` 而不是枚举：只有两套，而且将来「跟随系统」也只是一个
/// 求值结果，不需要第三个变体。
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
    pub fn flipped(self) -> Mode {
        match self {
            Mode::Light => Mode::Dark,
            Mode::Dark => Mode::Light,
        }
    }

    /// 存进 `settings.ini` 的字符串。
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
    #[allow(dead_code)]
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
    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(key: usize, sub: *const u16, opts: u32, sam: u32, out: *mut usize) -> i32;
        fn RegQueryValueExW(
            key: usize,
            name: *const u16,
            reserved: *mut u32,
            ty: *mut u32,
            data: *mut u8,
            len: *mut u32,
        ) -> i32;
        fn RegCloseKey(key: usize) -> i32;
    }
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
