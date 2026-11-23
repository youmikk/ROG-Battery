//! 假设备：没有硬件也能看界面。
//!
//! 真 HID 无法凭空造出来（要驱动），所以模拟放在「读数」这一层：给一组假读数，
//! 界面、托盘、提示都当真的用。开关是环境变量 `ROG_MOCK_DEVICES=<文件>`，
//! 一行一台设备：`族|型号|电量|充电`（族写 mouse/keyboard/headset/gamepad，
//! 充电写 0/1）。空行和 `#` 开头的行跳过。

use crate::device::{Battery, DeviceFamily, DeviceReading, ReadingQuality};

pub fn enabled() -> bool {
    std::env::var_os("ROG_MOCK_DEVICES").is_some()
}

pub fn window_title() -> &'static str {
    if enabled() { "X-Battery · 模拟设备" } else { crate::gui::WINDOW_TITLE }
}

/// 解析夹具文本。坏行跳过 —— 模拟工具不该因为一行写错就整体失效。
pub fn parse(text: &str) -> Vec<DeviceReading> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split('|').map(str::trim);
        let (Some(fam), Some(name), Some(level), Some(charging)) =
            (it.next(), it.next(), it.next(), it.next())
        else {
            continue;
        };
        let family = match fam.to_ascii_lowercase().as_str() {
            "mouse" | "鼠标" => DeviceFamily::Mouse,
            "keyboard" | "键盘" => DeviceFamily::Keyboard,
            "headset" | "耳机" => DeviceFamily::Headset,
            "gamepad" | "手柄" => DeviceFamily::Gamepad,
            _ => DeviceFamily::Unknown,
        };
        let Ok(level) = level.parse::<u8>() else {
            continue;
        };
        let occurrence = out
            .iter()
            .filter(|d: &&DeviceReading| d.name == name)
            .count();
        out.push(DeviceReading {
            key: crate::device::stable_key(&format!("mock:{name}:{occurrence}")),
            name: name.to_string(),
            family,
            battery: Some(Battery {
                percent: Some(level.min(100)),
                estimated_percent: None,
                charging: Some(
                    charging == "1" || charging.eq_ignore_ascii_case("true"),
                ),
                voltage_mv: None,
                approximate: false,
                level: None,
            }),
            connected: it.next() != Some("offline"),
            quality: ReadingQuality::Live,
        });
    }
    out
}

/// 模拟模式读不到文件时显示空列表，不回退到真实硬件。
pub fn readings() -> Option<Vec<DeviceReading>> {
    let path = std::env::var_os("ROG_MOCK_DEVICES")?;
    let text = std::fs::read_to_string(path).unwrap_or_default();
    Some(parse(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_families_are_parsed_in_order() {
        let text = "mouse|ROG HARPE ACE MINI|84|0\nkeyboard|ROG AZOTH|20|1\nheadset|ROG DELTA|55|0\ngamepad|ROG RAIKIRI|70|0\n";
        let got = parse(text);
        assert_eq!(got.len(), 4);
        assert_eq!(got[0].family, DeviceFamily::Mouse);
        assert_eq!(got[1].family, DeviceFamily::Keyboard);
        assert_eq!(got[2].family, DeviceFamily::Headset);
        assert_eq!(got[3].family, DeviceFamily::Gamepad);
        assert_eq!(got[1].percent(), Some(20));
        assert!(got[1].is_charging());
    }

    #[test]
    fn comments_and_bad_lines_are_skipped() {
        let text = "# 注释\n\n坏行没有分隔符\nmouse|X|abc|0\nmouse|X|50|0\n";
        let got = parse(text);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].percent(), Some(50));
    }

    #[test]
    fn level_is_clamped() {
        assert_eq!(parse("mouse|X|250|0")[0].percent(), Some(100));
    }
}
