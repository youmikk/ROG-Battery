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
    const HEADSET: [&str; 14] = [
        "HEADSET", "HEADPHONE", "HEADPHONES", "EARPHONE", "EARBUDS", "BUDS",
        "AIRPODS", "SPEAKER", "SOUNDBAR", "DELTA", "HATSU", "耳机", "降临", "棱镜",
    ];
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
    pub fn display_percent(&self) -> Option<BatteryPercent> {
        (self.connected && matches!(self.quality, ReadingQuality::Live | ReadingQuality::Cached))
            .then(|| self.battery.as_ref()?.display_percent()).flatten()
    }

    pub fn battery_text(&self, unit: bool) -> String {
        self.display_percent().map(|percent| percent.label(unit)).unwrap_or_else(|| "—".into())
    }

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
    Medium,
    Good,
    Full,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Battery {
    pub percent: Option<u8>,
    /// Protocol-specific representative value for a coarse level, not a measurement.
    pub estimated_percent: Option<u8>,
    pub charging: Option<bool>,
    pub voltage_mv: Option<u16>,
    pub approximate: bool,
    pub level: Option<BatteryLevel>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatteryPercent {
    pub value: u8,
    pub approximate: bool,
}

impl BatteryPercent {
    pub fn label(self, unit: bool) -> String {
        format!("{}{}{}", if self.approximate { "≈" } else { "" }, self.value, if unit { "%" } else { "" })
    }
}

impl Battery {
    pub fn display_percent(&self) -> Option<BatteryPercent> {
        if let Some(value) = self.percent {
            (value <= 100).then_some(BatteryPercent { value, approximate: self.approximate })
        } else {
            self.estimated_percent.filter(|value| *value <= 100)
                .map(|value| BatteryPercent { value, approximate: true })
        }
    }
}

#[cfg(test)]
mod battery_display_tests {
    use super::*;

    #[test]
    fn estimates_keep_the_prefix_without_turning_into_exact_samples() {
        let mut reading = DeviceReading { connected: true, quality: ReadingQuality::Live,
            battery: Some(Battery { percent: None, estimated_percent: Some(50), charging: None,
                voltage_mv: None, approximate: true, level: Some(BatteryLevel::Medium) }),
            ..Default::default() };
        assert_eq!(reading.percent(), None);
        assert_eq!(reading.battery_text(true), "≈50%");
        assert_eq!(reading.battery_text(false), "≈50");
        reading.quality = ReadingQuality::Cached;
        assert_eq!(reading.battery_text(true), "≈50%");
        for quality in [ReadingQuality::Stale, ReadingQuality::Unknown] {
            reading.quality = quality;
            assert_eq!(reading.battery_text(true), "—");
        }
        reading.quality = ReadingQuality::Live;
        reading.connected = false;
        assert_eq!(reading.battery_text(true), "—");
        reading.connected = true;
        let b = reading.battery.as_mut().unwrap();
        b.percent = Some(47); b.approximate = false;
        assert_eq!(reading.battery_text(true), "47%");
        assert_eq!(reading.percent(), Some(47));
    }

    #[test]
    fn zero_full_invalid_and_voltage_only_readings_keep_their_meaning() {
        let mut b = Battery { percent: None, estimated_percent: None, charging: None,
            voltage_mv: Some(4000), approximate: true, level: None };
        assert_eq!(b.display_percent(), None);
        for value in [0, 100] {
            b.estimated_percent = Some(value);
            assert_eq!(b.display_percent().unwrap().label(true), format!("≈{value}%"));
        }
        b.estimated_percent = Some(101);
        assert_eq!(b.display_percent(), None);
        b.estimated_percent = Some(50); b.percent = Some(255);
        assert_eq!(b.display_percent(), None);
        b.percent = Some(0); b.approximate = false;
        assert_eq!(b.display_percent().unwrap().label(true), "0%");
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dpi {
    pub current: Option<u16>,
    pub current_y: Option<u16>,
    pub values: Vec<u16>,
    pub writable: bool,
    /// Effect of a numerical DPI write; absent when that operation is not supported.
    pub write_effect: Option<DpiWriteEffect>,
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
    pub connection: Option<PollingRateConnection>,
    pub onboard_mode: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PollingRateConnection {
    Wired,
    Wireless,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DpiStage {
    /// 协议中的原始档位编号；禁用档位不会导致后续编号重排。
    pub index: u8,
    pub value: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DpiWriteEffect {
    Current,
    Stored,
}

pub const MAX_DPI_PRESETS: usize = 4;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DpiPresets {
    pub enabled: bool,
    pub values: Vec<u16>,
    /// Values hidden by the stage-count selector, retained for restoration.
    pub hidden: Vec<u16>,
}

impl DpiPresets {
    pub fn valid(&self) -> bool {
        let all: Vec<_> = self.values.iter().chain(&self.hidden).copied().collect();
        !self.values.is_empty() && all.len() <= MAX_DPI_PRESETS && all.iter().all(|v| *v != 0)
            && all.iter().collect::<std::collections::BTreeSet<_>>().len() == all.len()
    }

    pub fn edit(&mut self, index: usize, value: u16, dpi: &Dpi) -> bool {
        if dpi.write_effect.is_none() || !dpi.values.contains(&value)
            || index >= self.values.len() || self.values.iter().enumerate().any(|(i,v)| i != index && *v == value) {
            return false;
        }
        self.values[index] = value;
        self.values.sort_unstable();
        self.hidden.retain(|v| *v != value);
        true
    }

    fn add(&mut self, dpi: &Dpi) -> bool {
        if dpi.write_effect.is_none() || self.values.len() >= MAX_DPI_PRESETS { return false; }
        let next = self.hidden.first().copied().or_else(|| {
            dpi.current.into_iter().chain([400,800,1600,3200]).chain(dpi.values.iter().copied())
                .find(|v| *v != 0 && dpi.values.contains(v) && !self.values.contains(v))
        });
        if let Some(value) = next {
            self.hidden.retain(|v| *v != value);
            self.values.push(value);
            self.values.sort_unstable();
            true
        } else { false }
    }

    pub fn set_count(&mut self, count: usize, dpi: &Dpi) -> bool {
        if !(1..=MAX_DPI_PRESETS).contains(&count) { return false; }
        let mut next = self.clone();
        if count < next.values.len() {
            next.hidden.extend(next.values.split_off(count));
            next.hidden.sort_unstable();
        }
        while next.values.len() < count {
            if !next.add(dpi) { return false; }
        }
        if !next.valid() { return false; }
        *self = next;
        true
    }
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
    pub fn shortcut_stages(&self, presets: Option<&DpiPresets>) -> (Vec<DpiStage>, bool) {
        if let Some(presets) = presets.filter(|p| p.enabled) {
            return (presets.values.iter().enumerate().map(|(index,value)| DpiStage {
                index: index as u8, value: *value }).collect(), true);
        }
        if !self.stages.is_empty() { return (self.stages.clone(), false); }
        (self.quick_values().into_iter().enumerate().map(|(index,value)| DpiStage {
            index: index as u8, value }).collect(), true)
    }

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

#[cfg(test)]
mod preset_tests {
    use super::*;
    #[test]
    fn editing_presets_sorts_supported_values_and_limits_to_four_without_writing_device() {
        let dpi = Dpi { current: Some(800), values: vec![400,800,1200,1600,2400,3200,6400],
            writable: true, write_effect: Some(DpiWriteEffect::Current), ..Default::default() };
        let mut p = DpiPresets { enabled: true, values: vec![800,1600], ..Default::default() };
        assert!(!p.edit(0, 801, &dpi));
        assert!(!p.edit(0, 1600, &dpi));
        assert!(p.edit(0, 1200, &dpi));
        assert_eq!(p.values, vec![1200,1600]);
        assert!(p.edit(0, 3200, &dpi));
        assert_eq!(p.values, vec![1600,3200]);
        while p.values.len() < MAX_DPI_PRESETS { assert!(p.add(&dpi)); }
        assert!(!p.add(&dpi));
        assert!(p.valid());
        assert_eq!(p.values, vec![400,800,1600,3200]);
        assert_eq!(dpi.current, Some(800));
        let mut readonly = dpi.clone(); readonly.write_effect = None;
        assert!(!p.edit(0, 400, &readonly));
    }

    #[test]
    fn custom_values_do_not_replace_or_renumber_real_device_stages() {
        let stages = vec![DpiStage { index: 1, value: 800 }, DpiStage { index: 4, value: 3200 }];
        let dpi = Dpi { stages: stages.clone(), ..Default::default() };
        let mut presets = DpiPresets { enabled: true, values: vec![400,1600], ..Default::default() };
        let (choices, scalar) = dpi.shortcut_stages(Some(&presets));
        assert!(scalar);
        assert_eq!(choices.iter().map(|s|s.value).collect::<Vec<_>>(), vec![400,1600]);
        presets.enabled = false;
        assert_eq!(dpi.shortcut_stages(Some(&presets)), (stages.clone(), false));
        assert_eq!(dpi.stages, stages);
    }

    #[test]
    fn stage_count_hides_high_values_and_restores_them_without_device_writes() {
        let dpi = Dpi { current: Some(800), values: vec![400,800,1600,3200,6400],
            write_effect: Some(DpiWriteEffect::Current), ..Default::default() };
        let mut presets = DpiPresets { enabled: true, values: vec![400,800,1600,3200], ..Default::default() };
        assert!(presets.set_count(2, &dpi));
        assert_eq!(presets.values, vec![400,800]);
        assert_eq!(presets.hidden, vec![1600,3200]);
        assert_eq!(dpi.shortcut_stages(Some(&presets)).0.iter().map(|s|s.value).collect::<Vec<_>>(),vec![400,800]);
        assert!(presets.set_count(1, &dpi));
        assert_eq!(presets.hidden, vec![800,1600,3200]);
        assert!(presets.set_count(4, &dpi));
        assert_eq!(presets.values, vec![400,800,1600,3200]);
        assert!(presets.hidden.is_empty());
        assert!(!presets.set_count(0, &dpi));
        assert!(!presets.set_count(5, &dpi));
        assert_eq!(dpi.current, Some(800));
    }

    #[test]
    fn editing_hidden_value_does_not_restore_duplicates_and_failed_expansion_is_atomic() {
        let dpi = Dpi { values: vec![400,800,1600,3200], write_effect: Some(DpiWriteEffect::Current), ..Default::default() };
        let mut presets = DpiPresets {enabled:true,values:vec![400,800],hidden:vec![1600,3200]};
        assert!(presets.edit(0,1600,&dpi));
        assert_eq!(presets.values,vec![800,1600]);
        assert_eq!(presets.hidden,vec![3200]);
        assert!(presets.set_count(4,&dpi));
        assert_eq!(presets.values,vec![400,800,1600,3200]);
        let limited = Dpi {values:vec![400,800], ..dpi};
        let mut presets = DpiPresets {enabled:true,values:vec![400],..Default::default()};
        let before = presets.clone();
        assert!(!presets.set_count(4,&limited));
        assert_eq!(presets,before);
    }
}
