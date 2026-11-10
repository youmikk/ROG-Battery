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

#[cfg(test)]
thread_local! {
    static TEST_LANGUAGE: std::cell::Cell<Option<Language>> = const { std::cell::Cell::new(None) };
}

pub fn apply(preference: Language) -> bool {
    let language = preference.resolve(unsafe { crate::win32::GetUserDefaultUILanguage() });
    ACTIVE.swap(language as u8, Ordering::Relaxed) != language as u8
}

pub fn current() -> Language {
    #[cfg(test)]
    if let Some(language) = TEST_LANGUAGE.with(|value| value.get()) {
        return language;
    }
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
        crate::ops::ErrorKind::Invalid => crate::tr!("参数不合法，本次没有碰设备。"),
        crate::ops::ErrorKind::Asleep => {
            crate::tr!("鼠标休眠中。移动一下鼠标即可唤醒，唤醒后重试。")
        }
        crate::ops::ErrorKind::Rejected => crate::tr!("设备明确拒绝了这个命令（NAK）。"),
        crate::ops::ErrorKind::Mismatch => {
            crate::tr!("写进去了但读回不一致，请用 backup 文件核对。")
        }
        crate::ops::ErrorKind::Other => crate::tr!("操作失败。"),
    }
}

#[cfg(test)]
pub fn with_language<T>(language: Language, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<Language>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_LANGUAGE.with(|value| value.set(self.0));
        }
    }
    let _restore = Restore(TEST_LANGUAGE.with(|value| value.replace(Some(language))));
    run()
}

mod catalog;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_uses_windows_display_language_with_english_fallback() {
        for id in [0x0804, 0x0404, 0x0c04, 0x1004, 0x1404] {
            assert_eq!(Language::System.resolve(id), Language::Chinese);
        }
        for id in [0x0409, 0x0809, 0x0411, 0x0407, 0] {
            assert_eq!(Language::System.resolve(id), Language::English);
        }
        assert_eq!(Language::Chinese.resolve(0x0409), Language::Chinese);
        assert_eq!(Language::English.resolve(0x0804), Language::English);
    }

    #[test]
    fn language_codes_are_stable_and_invalid_values_are_rejected() {
        for language in Language::ALL {
            assert_eq!(Language::parse(language.code()), Some(language));
        }
        assert_eq!(Language::parse("EN-us"), Some(Language::English));
        assert_eq!(Language::parse("ZH-cn"), Some(Language::Chinese));
        assert_eq!(Language::parse("unknown"), None);
    }

    #[test]
    fn switching_language_translates_known_labels_and_preserves_user_data() {
        with_language(Language::English, || {
            assert_eq!(translate("设置"), "Settings");
            assert_eq!(translate("ROG 龙鳞 ACE"), "ROG 龙鳞 ACE");
            assert_eq!(
                translate(r"D:\设备备份\settings.ini"),
                r"D:\设备备份\settings.ini"
            );
        });
        with_language(Language::Chinese, || assert_eq!(translate("设置"), "设置"));
    }

    #[test]
    fn cached_estimates_translate_when_language_changes() {
        let estimate = crate::history::Estimate {
            text: "预计可用时间：样本不足".into(),
            ..Default::default()
        };
        with_language(Language::English, || {
            assert_eq!(
                estimate.localized_text(),
                "Remaining battery life: Not enough samples"
            );
            assert_eq!(estimate.localized_short_text(), "Estimating…");
            assert_eq!(duration(1.5), "1 h 30 min");
            assert_eq!(duration(30.0), "1 d 6 h");
            assert_eq!(age(90), "1 min");
        });
        with_language(Language::Chinese, || {
            assert_eq!(estimate.localized_text(), estimate.text)
        });
    }
}
