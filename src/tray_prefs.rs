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
}

impl Default for DeviceTray {
    fn default() -> Self {
        Self {
            enabled: true,
            kind: TrayKind::Auto,
            content: TrayContent::Combined,
        }
    }
}

impl DeviceTray {
    pub fn parse(value: &str) -> Option<Self> {
        let mut parts = value.split(',');
        let enabled = match parts.next()? {
            "1" => true,
            "0" => false,
            _ => return None,
        };
        let kind = *TrayKind::ALL.get(parts.next()?.parse::<usize>().ok()?)?;
        let content = *TrayContent::ALL.get(parts.next()?.parse::<usize>().ok()?)?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            enabled,
            kind,
            content,
        })
    }

    pub fn text(self) -> String {
        format!(
            "{},{},{}",
            u8::from(self.enabled),
            self.kind as u8,
            self.content as u8
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
        for value in ["1,9,0", "1,0,9", "yes,0,0", "1,0", "1,0,0,1"] {
            assert!(DeviceTray::parse(value).is_none());
        }
    }
}
