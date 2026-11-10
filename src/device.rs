//! 设备族。
//!
//! 现在的界面文案一律说「鼠标」，但同一套 HID 协议下还有耳机、键盘、手柄。
//! 提示语按设备族说话，才不至于对着耳机说「鼠标电量低」。判据用型号名里的
//! 关键字 —— 这是唯一在所有设备上都拿得到的稳定信息（PID 每个型号都不同）。

/// 设备族。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DeviceFamily {
    Mouse,
    Headset,
    Keyboard,
    Gamepad,
    /// 没认出来，或者根本没读到身份。
    #[default]
    Unknown,
}

/// 从型号名判设备族。
///
/// 大小写不敏感：设备回报的型号既有全大写（`ROG HARPE ACE MINI`）也有混合。
pub fn family_of(model_name: &str) -> DeviceFamily {
    let m = model_name.to_ascii_uppercase();
    const HEADSET: [&str; 6] = ["HEADSET", "DELTA", "HATSU", "耳机", "降临", "棱镜"];
    const KEYBOARD: [&str; 8] = [
        "KEYBOARD",
        "AZOTH",
        "FLARE",
        "STRIX SCOPE",
        "FALCHION",
        "键盘",
        "龙骑士",
        "夜魔",
    ];
    const GAMEPAD: [&str; 4] = ["GAMEPAD", "RAIKIRI", "手柄", "雷切"];
    const MOUSE: [&str; 8] = [
        "MOUSE", "HARPE", "GLADIUS", "KERIS", "鼠标", "龙鳞", "战刃", "月刃",
    ];
    if HEADSET.iter().any(|k| m.contains(k)) {
        return DeviceFamily::Headset;
    }
    if KEYBOARD.iter().any(|k| m.contains(k)) {
        return DeviceFamily::Keyboard;
    }
    if GAMEPAD.iter().any(|k| m.contains(k)) {
        return DeviceFamily::Gamepad;
    }
    if MOUSE.iter().any(|k| m.contains(k)) {
        return DeviceFamily::Mouse;
    }
    DeviceFamily::Unknown
}

impl DeviceFamily {
    /// 提示语里的名词。
    pub fn noun(self) -> &'static str {
        match self {
            DeviceFamily::Mouse => "鼠标",
            DeviceFamily::Headset => "耳机",
            DeviceFamily::Keyboard => "键盘",
            DeviceFamily::Gamepad => "手柄",
            DeviceFamily::Unknown => "设备",
        }
    }
}

/// 一台设备的读数。字段只描述「看到的事实」，不含怎么显示。
///
/// 首页、详情和托盘共享设备标识。
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DeviceReading {
    pub key: String,
    pub name: String,
    pub family: DeviceFamily,
    pub level: u8,
    pub charging: bool,
    pub connected: bool,
    pub simulated: bool,
}

/// 设置只保存不透明标识，避免把设备序列号或接口路径写入配置。
pub fn stable_key(source: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for b in source.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// 一行读数文案，例如「鼠标 84%（充电中）」。
#[cfg(test)]
pub fn reading_line(r: &DeviceReading) -> String {
    let mut s = format!("{} {}%", r.family.noun(), r.level);
    if r.charging {
        s.push_str("（充电中）");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_model_names_are_recognised() {
        assert_eq!(family_of("ROG HARPE ACE MINI"), DeviceFamily::Mouse);
        assert_eq!(family_of("ROG Gladius III"), DeviceFamily::Mouse);
        assert_eq!(family_of("ROG STRIX FLARE II"), DeviceFamily::Keyboard);
        assert_eq!(family_of("ROG DELTA HEADSET"), DeviceFamily::Headset);
        // 设备回报的中文型号名（真机实测 `ROG 龙鳞 Ace Mini`）也必须认出来。
        assert_eq!(family_of("ROG 龙鳞 Ace Mini"), DeviceFamily::Mouse);
    }

    #[test]
    fn empty_or_unknown_falls_back() {
        assert_eq!(family_of(""), DeviceFamily::Unknown);
        assert_eq!(family_of("USB Composite Device"), DeviceFamily::Unknown);
        assert_eq!(family_of("").noun(), "设备");
    }

    fn reading(family: DeviceFamily, level: u8, charging: bool) -> DeviceReading {
        DeviceReading {
            family,
            level,
            charging,
            ..Default::default()
        }
    }

    #[test]
    fn reading_line_names_the_device_family() {
        assert_eq!(reading_line(&reading(DeviceFamily::Mouse, 84, false)), "鼠标 84%");
        assert_eq!(
            reading_line(&reading(DeviceFamily::Headset, 40, true)),
            "耳机 40%（充电中）"
        );
    }

    #[test]
    fn noun_is_never_the_wrong_device() {
        assert_eq!(family_of("ROG HARPE ACE MINI").noun(), "鼠标");
        assert_ne!(family_of("ROG DELTA HEADSET").noun(), "鼠标");
    }
}
