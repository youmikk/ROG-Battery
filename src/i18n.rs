//! Application language, independent of the device protocol and stored values.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    System,
    Chinese,
    English,
}

impl Language {
    pub const ALL: [Self; 3] = [Self::System, Self::Chinese, Self::English];

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "system" => Some(Self::System),
            "zh" | "zh-cn" | "zh-hans" => Some(Self::Chinese),
            "en" | "en-us" => Some(Self::English),
            _ => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Chinese => "zh-CN",
            Self::English => "en",
        }
    }

    pub fn resolve(self, system_lang_id: u16) -> Self {
        match self {
            Self::System if system_lang_id & 0x03ff == 0x04 => Self::Chinese,
            Self::System => Self::English,
            explicit => explicit,
        }
    }
}

static ACTIVE: AtomicU8 = AtomicU8::new(Language::Chinese as u8);

pub fn apply(preference: Language) -> bool {
    let language = preference.resolve(unsafe { crate::win32::GetUserDefaultUILanguage() });
    ACTIVE.swap(language as u8, Ordering::Relaxed) != language as u8
}

pub fn current() -> Language {

    match ACTIVE.load(Ordering::Relaxed) {
        value if value == Language::English as u8 => Language::English,
        _ => Language::Chinese,
    }
}

pub fn english() -> bool {
    current() == Language::English
}

/// Exact message lookup, used for protocol labels and cached explanations.
pub fn translate(text: &str) -> &str {
    catalog::lookup(current(), text)
}

pub fn duration(hours: f64) -> String {
    let minutes = ((hours * 60.0).round() as i64).max(1);
    let (days, h, m) = (minutes / 1440, minutes % 1440 / 60, minutes % 60);
    match (days, h, m) {
        (d, 0, _) if d > 0 => format!("{d} d"),
        (d, _, _) if d > 0 => format!("{d} d {h} h"),
        (_, 0, _) => format!("{m} min"),
        (_, _, 0) => format!("{h} h"),
        _ => format!("{h} h {m} min"),
    }
}

pub fn age(seconds: i64) -> String {
    if !english() {
        return crate::history::ago_text(seconds);
    }
    if seconds < 60 {
        "just now".into()
    } else if seconds < 3600 {
        format!("{} min", seconds / 60)
    } else {
        format!("{} h", seconds / 3600)
    }
}

pub fn operation_hint(kind: crate::ops::ErrorKind) -> &'static str {
    match kind {
        crate::ops::ErrorKind::Invalid => crate::tr!("输入参数超出设备支持范围，当前设置未改动。"),
        crate::ops::ErrorKind::Asleep => {
            crate::tr!("鼠标休眠中。移动一下鼠标即可唤醒，唤醒后重试。")
        }
        crate::ops::ErrorKind::Rejected => crate::tr!("设备未接受该项设置，请确认设备处于可修改状态后重试。"),
        crate::ops::ErrorKind::Mismatch => {
            crate::tr!("设置已发送但核对结果不一致，建议刷新设备或结合备份文件确认。")
        }
        crate::ops::ErrorKind::Other => crate::tr!("操作失败。"),
    }
}

mod catalog;

pub fn battery_level(level: crate::device::BatteryLevel) -> &'static str {
    match level {
        crate::device::BatteryLevel::Critical => crate::tr!("电量极低"),
        crate::device::BatteryLevel::Low => crate::tr!("电量偏低"),
        crate::device::BatteryLevel::Medium => crate::tr!("电量中等"),
        crate::device::BatteryLevel::Good => crate::tr!("电量良好"),
        crate::device::BatteryLevel::Full => crate::tr!("电量充足"),
    }
}
