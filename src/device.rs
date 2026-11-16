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
    pub battery: Option<Battery>,
    pub connected: bool,
    pub quality: ReadingQuality,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReadingQuality {
    #[default]
    Live,
    Cached,
    Stale,
    Unknown,
}

impl DeviceReading {
    pub fn percent(&self) -> Option<u8> {
        (self.connected && matches!(self.quality, ReadingQuality::Live | ReadingQuality::Cached))
            .then(|| self.battery.as_ref()?.percent)
            .flatten()
            .filter(|percent| *percent <= 100)
    }

    pub fn is_charging(&self) -> bool {
        self.connected && self.quality == ReadingQuality::Live
            && self.battery.as_ref().and_then(|battery| battery.charging) == Some(true)
    }

    pub fn is_low(&self) -> bool {
        self.connected
            && self.quality == ReadingQuality::Live
            && !self.is_charging()
            && match self.percent() {
                Some(percent) => percent <= 20,
                None => self
                    .battery
                    .as_ref()
                    .and_then(|battery| battery.level)
                    .is_some_and(|level| {
                        matches!(level, BatteryLevel::Critical | BatteryLevel::Low)
                    }),
            }
    }

    pub fn coarse_level(&self) -> Option<BatteryLevel> {
        (self.connected && matches!(self.quality, ReadingQuality::Live | ReadingQuality::Cached))
            .then(|| self.battery.as_ref()?.level)
            .flatten()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatteryLevel {
    Critical,
    Low,
    Good,
    Full,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Battery {
    pub percent: Option<u8>,
    pub charging: Option<bool>,
    pub voltage_mv: Option<u16>,
    pub approximate: bool,
    pub level: Option<BatteryLevel>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dpi {
    pub current: Option<u16>,
    pub current_y: Option<u16>,
    pub values: Vec<u16>,
    pub writable: bool,
    pub stages: Vec<DpiStage>,
    pub active_stage: Option<u8>,
    pub profile: Option<u16>,
    /// Fingerprint of the complete configuration when the protocol has no profile ID.
    pub configuration_token: Option<u64>,
    pub stage_writable: bool,
    pub stage_editable: bool,
    pub stage_y: std::collections::BTreeMap<u8, u16>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PollingRate {
    pub current: Option<u16>,
    pub values: Vec<u16>,
    pub writable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DpiStage {
    /// 协议中的原始档位编号；禁用档位不会导致后续编号重排。
    pub index: u8,
    pub value: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageContext {
    Profile(u16),
    Configuration(u64),
}

impl StageContext {
    pub fn profile(self) -> Result<u16, String> {
        match self {
            Self::Profile(profile) => Ok(profile),
            _ => Err("设备配置已变化，请刷新后重试".into()),
        }
    }
}

impl Dpi {
    pub fn stage_context(&self) -> Option<StageContext> {
        self.configuration_token.map(StageContext::Configuration)
            .or_else(|| self.profile.map(StageContext::Profile))
    }
    pub fn disable_writes(&mut self) {
        self.writable = false;
        self.stage_writable = false;
        self.stage_editable = false;
    }
    pub fn quick_values(&self) -> Vec<u16> {
        let mut values = Vec::new();
        if let Some(current) = self.current {
            values.push(current);
        }
        for value in [400, 800, 1600, 3200] {
            if self.values.contains(&value) && !values.contains(&value) {
                values.push(value);
            }
        }
        values.truncate(4);
        values.sort_unstable();
        values
    }
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
