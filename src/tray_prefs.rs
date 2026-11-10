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
            Self::Auto => crate::tr!("自动识别"),
            Self::Mouse => crate::tr!("鼠标"),
            Self::Keyboard => crate::tr!("键盘"),
            Self::Headset => crate::tr!("耳机"),
            Self::Gamepad => crate::tr!("手柄"),
            Self::Generic => crate::tr!("其他"),
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

    pub fn order(self) -> u8 {
        match self {
            Self::Mouse => 0,
            Self::Keyboard => 1,
            Self::Headset => 2,
            Self::Gamepad => 3,
            Self::Generic | Self::Auto => 4,
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
