use crate::device::DeviceFamily;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrayKind {
    #[default]
    Auto,
    Mouse,
    Keyboard,
    Headset,
    Gamepad,
    Generic,
}

impl TrayKind {
    pub const ALL: [Self; 6] = [
        Self::Auto,
        Self::Mouse,
        Self::Keyboard,
        Self::Headset,
        Self::Gamepad,
        Self::Generic,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "自动识别",
            Self::Mouse => "鼠标",
            Self::Keyboard => "键盘",
            Self::Headset => "耳机",
            Self::Gamepad => "手柄",
            Self::Generic => "通用",
        }
    }

    pub fn resolve(self, family: DeviceFamily) -> Self {
        if self != Self::Auto {
            return self;
        }
        match family {
            DeviceFamily::Mouse => Self::Mouse,
            DeviceFamily::Keyboard => Self::Keyboard,
            DeviceFamily::Headset => Self::Headset,
            DeviceFamily::Gamepad => Self::Gamepad,
            DeviceFamily::Unknown => Self::Generic,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrayContent {
    #[default]
    Combined,
    Number,
    Icon,
}

impl TrayContent {
    pub const ALL: [Self; 3] = [Self::Combined, Self::Number, Self::Icon];
    pub fn label(self) -> &'static str {
        match self {
            Self::Combined => "图标＋电量",
            Self::Number => "仅电量数字",
            Self::Icon => "仅设备图标",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceTray {
    pub enabled: bool,
    pub kind: TrayKind,
    pub content: TrayContent,
    pub content_auto: bool,
}

impl Default for DeviceTray {
    fn default() -> Self {
        Self {
            enabled: true,
            kind: TrayKind::Auto,
            content: TrayContent::Combined,
            content_auto: true,
        }
    }
}

impl DeviceTray {
    pub fn effective_content(self, device_count: usize) -> TrayContent {
        if !self.content_auto {
            self.content
        } else if device_count == 1 {
            TrayContent::Number
        } else {
            TrayContent::Combined
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let mut parts = value.split(',');
        let enabled = match parts.next()? {
            "1" => true,
            "0" => false,
            _ => return None,
        };
        let kind = *TrayKind::ALL.get(parts.next()?.parse::<usize>().ok()?)?;
        let content = *TrayContent::ALL.get(parts.next()?.parse::<usize>().ok()?)?;
        // 旧配置的三段值视为手动选择，保留已有显示方式。
        let content_auto = match parts.next() {
            None | Some("0") => false,
            Some("1") => true,
            _ => return None,
        };
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            enabled,
            kind,
            content,
            content_auto,
        })
    }

    pub fn text(self) -> String {
        format!(
            "{},{},{}{}",
            u8::from(self.enabled),
            self.kind as u8,
            self.content as u8,
            if self.content_auto { ",1" } else { "" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manual_kind_survives_detection_changes() {
        assert_eq!(
            TrayKind::Headset.resolve(DeviceFamily::Mouse),
            TrayKind::Headset
        );
        assert_eq!(
            TrayKind::Auto.resolve(DeviceFamily::Keyboard),
            TrayKind::Keyboard
        );
    }
    #[test]
    fn invalid_preferences_are_rejected() {
        for value in ["1,9,0", "1,0,9", "yes,0,0", "1,0", "1,0,0,9", "1,0,0,1,0"] {
            assert!(DeviceTray::parse(value).is_none());
        }
    }

    #[test]
    fn automatic_content_follows_device_count_and_survives_other_preferences() {
        let pref = DeviceTray {
            enabled: false,
            kind: TrayKind::Mouse,
            ..Default::default()
        };
        let loaded = DeviceTray::parse(&pref.text()).unwrap();
        assert_eq!(loaded, pref);
        assert_eq!(loaded.effective_content(1), TrayContent::Number);
        assert_eq!(loaded.effective_content(2), TrayContent::Combined);
        assert_eq!(loaded.effective_content(1), TrayContent::Number);
    }

    #[test]
    fn explicit_and_legacy_content_never_changes_with_device_count() {
        for content in TrayContent::ALL {
            let pref = DeviceTray {
                content,
                content_auto: false,
                ..Default::default()
            };
            let loaded = DeviceTray::parse(&pref.text()).unwrap();
            assert_eq!(loaded, pref);
            for count in [1, 2, 4] {
                assert_eq!(loaded.effective_content(count), content);
            }
        }
        assert_eq!(
            DeviceTray::parse("1,0,0").unwrap().effective_content(1),
            TrayContent::Combined
        );
    }
}
