//! 设备族。
//!
//! 现在的界面文案一律说「鼠标」，但同一套 HID 协议下还有耳机、键盘、手柄。
//! 提示语按设备族说话，才不至于对着耳机说「鼠标电量低」。判据用型号名里的
//! 关键字 —— 这是唯一在所有设备上都拿得到的稳定信息（PID 每个型号都不同）。

/// 设备族。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceFamily {
    Mouse,
    Headset,
    Keyboard,
    /// 没认出来，或者根本没读到身份。
    Unknown,
}

/// 从型号名判设备族。
///
/// 大小写不敏感：设备回报的型号既有全大写（`ROG HARPE ACE MINI`）也有混合。
pub fn family_of(model_name: &str) -> DeviceFamily {
    let m = model_name.to_ascii_uppercase();
    const HEADSET: [&str; 4] = ["HEADSET", "HATSU", "FALCHION", "耳机"];
    const KEYBOARD: [&str; 4] = ["KEYBOARD", "AZOTH", "FLARE", "STRIX SCOPE"];
    const MOUSE: [&str; 4] = ["MOUSE", "HARPE", "GLADIUS", "KERIS"];
    if HEADSET.iter().any(|k| m.contains(k)) {
        return DeviceFamily::Headset;
    }
    if KEYBOARD.iter().any(|k| m.contains(k)) {
        return DeviceFamily::Keyboard;
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
            DeviceFamily::Unknown => "设备",
        }
    }
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
    }

    #[test]
    fn empty_or_unknown_falls_back() {
        assert_eq!(family_of(""), DeviceFamily::Unknown);
        assert_eq!(family_of("USB Composite Device"), DeviceFamily::Unknown);
        assert_eq!(family_of("").noun(), "设备");
    }

    #[test]
    fn noun_is_never_the_wrong_device() {
        assert_eq!(family_of("ROG HARPE ACE MINI").noun(), "鼠标");
        assert_ne!(family_of("ROG DELTA HEADSET").noun(), "鼠标");
    }
}
