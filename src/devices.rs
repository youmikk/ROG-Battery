//! ROG 外设 PID → 型号名对照表（VID 0x0B05）。
//!
//! 数据来源：ASUS Gear Link 的 `hid-whitelist.json` 与各 PID 的 `manifest.json`
//! （`caps.deviceName` / `caps.deviceNameZH`），抓取日期 2026-10-21。
//! 完整表格见 `docs/device-support.md`。
//!
//! 为什么需要这张表：设备固件自报的产品名不可控（可能夹厂商缩写、全英文、甚至空串），
//! 而接收器的扫描应答只给 PID。表里有的 PID 一律以表为准，表里没有才退回固件自报名。

/// 设备大类。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Mouse,
    Keyboard,
    Gamepad,
}

impl Kind {
    /// 中文名，用于界面显示。
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Mouse => "鼠标",
            Kind::Keyboard => "键盘",
            Kind::Gamepad => "手柄",
        }
    }
}

/// 一个带电池的外设型号。同一型号通常占两个相邻 PID（有线 / 2.4G），
/// 少数型号有三个，因此表里按 PID 逐条列出。
pub struct Device {
    pub pid: u16,
    pub name_en: &'static str,
    pub name_zh: &'static str,
    pub kind: Kind,
}

/// 无线接收器：自身没有电池，只是转发通道。
pub struct Dongle {
    pub pid: u16,
    pub name_zh: &'static str,
}

/// 收录的接收器（5 个）。
pub const DONGLES: &[Dongle] = &[
    Dongle { pid: 0x1ACE, name_zh: "ROG OMNI 无线接收器" },
    Dongle { pid: 0x1ACF, name_zh: "ROG OMNI 无线接收器" },
    Dongle { pid: 0x1AD0, name_zh: "ROG SpeedNova 8K 无线接收器" },
    Dongle { pid: 0x1AD1, name_zh: "ROG SpeedNova 8K 无线接收器" },
    Dongle { pid: 0x1D54, name_zh: "华硕无线接收器" },
];

/// 收录的带电池设备（75 个 PID）。
pub const DEVICES: &[Device] = &[
    Device { pid: 0x1A92, name_en: "ROG HARPE ACE AIM LAB EDITION", name_zh: "ROG 龙鳞 Ace Aim Lab 合作版", kind: Kind::Mouse },
    Device { pid: 0x1A94, name_en: "ROG HARPE ACE AIM LAB EDITION", name_zh: "ROG 龙鳞 Ace Aim Lab 合作版", kind: Kind::Mouse },
    Device { pid: 0x1B16, name_en: "ROG KERIS II ACE", name_zh: "ROG 月刃2 ACE", kind: Kind::Mouse },
    Device { pid: 0x1B18, name_en: "ROG KERIS II ACE", name_zh: "ROG 月刃2 ACE", kind: Kind::Mouse },
    Device { pid: 0x1B63, name_en: "ROG HARPE ACE MINI", name_zh: "ROG 龙鳞 Ace Mini", kind: Kind::Mouse },
    Device { pid: 0x1B65, name_en: "ROG HARPE ACE MINI", name_zh: "ROG 龙鳞 Ace Mini", kind: Kind::Mouse },
    Device { pid: 0x1B67, name_en: "ROG HARPE ACE EXTREME", name_zh: "ROG 龙鳞 Ace Extreme", kind: Kind::Mouse },
    Device { pid: 0x1B69, name_en: "ROG HARPE ACE EXTREME", name_zh: "ROG 龙鳞 Ace Extreme", kind: Kind::Mouse },
    Device { pid: 0x1C0C, name_en: "ROG KERIS II ORIGIN", name_zh: "ROG 月刃2 RGB", kind: Kind::Mouse },
    Device { pid: 0x1C0E, name_en: "ROG KERIS II ORIGIN", name_zh: "ROG 月刃2 RGB", kind: Kind::Mouse },
    Device { pid: 0x1C69, name_en: "ROG HARPE II ACE", name_zh: "ROG 龙鳞 2 Ace", kind: Kind::Mouse },
    Device { pid: 0x1C6B, name_en: "ROG HARPE II ACE", name_zh: "ROG 龙鳞 2 Ace", kind: Kind::Mouse },
    Device { pid: 0x1CD3, name_en: "PROART MOUSE MD301", name_zh: "PROART MOUSE MD301", kind: Kind::Mouse },
    Device { pid: 0x1CD5, name_en: "PROART MOUSE MD301", name_zh: "PROART MOUSE MD301", kind: Kind::Mouse },
    Device { pid: 0x1D4C, name_en: "ROG KERIS II ORIGIN-KJP", name_zh: "ROG 月刃 2 RGB - KJP", kind: Kind::Mouse },
    Device { pid: 0x1D4E, name_en: "ROG KERIS II ORIGIN-KJP", name_zh: "ROG 月刃 2 RGB - KJP", kind: Kind::Mouse },
    Device { pid: 0x1D7A, name_en: "ROG HARPE II EXTREME EDITION 20", name_zh: "ROG 龙鳞 2 Extreme 20周年版", kind: Kind::Mouse },
    Device { pid: 0x1D7B, name_en: "ROG HARPE II EXTREME EDITION 20", name_zh: "ROG 龙鳞 2 Extreme 20周年版", kind: Kind::Mouse },
    Device { pid: 0x1D7C, name_en: "ROG HARPE II EXTREME EDITION 20", name_zh: "ROG 龙鳞 2 Extreme 20周年版", kind: Kind::Mouse },
    Device { pid: 0x1DB0, name_en: "ROG GLADIUS IV ACE", name_zh: "ROG 战刃 4 ACE", kind: Kind::Mouse },
    Device { pid: 0x1DBF, name_en: "ROG GLADIUS IV ACE", name_zh: "ROG 战刃 4 ACE", kind: Kind::Mouse },
    Device { pid: 0x1DDE, name_en: "ROG SPATHA X 65K", name_zh: "ROG 斯巴达 X 65K", kind: Kind::Mouse },
    Device { pid: 0x1DDF, name_en: "ROG SPATHA X 65K", name_zh: "ROG 斯巴达 X 65K", kind: Kind::Mouse },
    Device { pid: 0x1DE0, name_en: "ROG SPATHA X 65K", name_zh: "ROG 斯巴达 X 65K", kind: Kind::Mouse },
    Device { pid: 0x1DE1, name_en: "ROG SPATHA X 65K", name_zh: "ROG 斯巴达 X 65K", kind: Kind::Mouse },
    Device { pid: 0x1E2F, name_en: "ROG HARPE II ACE MINI", name_zh: "ROG 龙鳞 2 Ace Mini", kind: Kind::Mouse },
    Device { pid: 0x1E32, name_en: "ROG HARPE II ACE MINI", name_zh: "ROG 龙鳞 2 Ace Mini", kind: Kind::Mouse },
    Device { pid: 0x1E4A, name_en: "ROG HARPE II ACE-PBZ", name_zh: "ROG 龙鳞2 ACE - PBZ", kind: Kind::Mouse },
    Device { pid: 0x1E4B, name_en: "ROG HARPE II ACE-PBZ", name_zh: "ROG 龙鳞2 ACE - PBZ", kind: Kind::Mouse },
    Device { pid: 0x1E4D, name_en: "ROG HARPE II ACE-PBZ", name_zh: "ROG 龙鳞2 ACE - PBZ", kind: Kind::Mouse },
    Device { pid: 0x1E4E, name_en: "ROG GLADIUS IV ACE MAX", name_zh: "ROG 战刃 4 ACE MAX", kind: Kind::Mouse },
    Device { pid: 0x1E4F, name_en: "ROG GLADIUS IV ACE MAX", name_zh: "ROG 战刃 4 ACE MAX", kind: Kind::Mouse },
    Device { pid: 0x1E51, name_en: "ROG HARPE II ACE MINI DEMON1 EDITION", name_zh: "ROG 龙鳞 2 Ace Mini Demon1联名版", kind: Kind::Mouse },
    Device { pid: 0x1E52, name_en: "ROG HARPE II ACE MINI DEMON1 EDITION", name_zh: "ROG 龙鳞 2 Ace Mini Demon1联名版", kind: Kind::Mouse },
    Device { pid: 0x1A83, name_en: "ROG AZOTH", name_zh: "ROG 夜魔", kind: Kind::Keyboard },
    Device { pid: 0x1A85, name_en: "ROG AZOTH", name_zh: "ROG 夜魔", kind: Kind::Keyboard },
    Device { pid: 0x1AAE, name_en: "ROG STRIX SCOPE II 96 WIRELESS", name_zh: "ROG 游侠2 98无线版", kind: Kind::Keyboard },
    Device { pid: 0x1AAF, name_en: "ROG STRIX SCOPE II 96 WIRELESS", name_zh: "ROG 游侠2 98无线版", kind: Kind::Keyboard },
    Device { pid: 0x1AB0, name_en: "ROG STRIX SCOPE II 96 WIRELESS", name_zh: "ROG 游侠2 98无线版", kind: Kind::Keyboard },
    Device { pid: 0x1B3F, name_en: "ROG AZOTH EXTREME", name_zh: "ROG 夜魔 EXTREME", kind: Kind::Keyboard },
    Device { pid: 0x1B40, name_en: "ROG AZOTH EXTREME", name_zh: "ROG 夜魔 EXTREME", kind: Kind::Keyboard },
    Device { pid: 0x1B42, name_en: "ROG AZOTH EXTREME", name_zh: "ROG 夜魔 EXTREME", kind: Kind::Keyboard },
    Device { pid: 0x1C10, name_en: "ROG AZOTH 96 HE / LITE", name_zh: "ROG 夜魔 98 HE / LITE", kind: Kind::Keyboard },
    Device { pid: 0x1C11, name_en: "ROG AZOTH 96 HE / LITE", name_zh: "ROG 夜魔 98 HE / LITE", kind: Kind::Keyboard },
    Device { pid: 0x1C12, name_en: "ROG AZOTH 96 HE / LITE", name_zh: "ROG 夜魔 98 HE / LITE", kind: Kind::Keyboard },
    Device { pid: 0x1C24, name_en: "ROG AZOTH X", name_zh: "ROG 夜魔 X", kind: Kind::Keyboard },
    Device { pid: 0x1C25, name_en: "ROG AZOTH X", name_zh: "ROG 夜魔 X", kind: Kind::Keyboard },
    Device { pid: 0x1C26, name_en: "ROG AZOTH X", name_zh: "ROG 夜魔 X", kind: Kind::Keyboard },
    Device { pid: 0x1C2F, name_en: "ROG FALCATA", name_zh: "ROG 影魔", kind: Kind::Keyboard },
    Device { pid: 0x1C30, name_en: "ROG FALCATA", name_zh: "ROG 影魔", kind: Kind::Keyboard },
    Device { pid: 0x1C31, name_en: "ROG FALCATA", name_zh: "ROG 影魔", kind: Kind::Keyboard },
    Device { pid: 0x1CE8, name_en: "ROG STRIX MORPH 96 WIRELESS", name_zh: "ROG 术士 98", kind: Kind::Keyboard },
    Device { pid: 0x1CE9, name_en: "ROG STRIX MORPH 96 WIRELESS", name_zh: "ROG 术士 98", kind: Kind::Keyboard },
    Device { pid: 0x1CEA, name_en: "ROG STRIX MORPH 96 WIRELESS", name_zh: "ROG 术士 98", kind: Kind::Keyboard },
    Device { pid: 0x1CEF, name_en: "ROG AZOTH EXTREME SPECIAL EDITION", name_zh: "ROG 夜魔 EXTREME SPECIAL EDITION", kind: Kind::Keyboard },
    Device { pid: 0x1CF0, name_en: "ROG AZOTH EXTREME SPECIAL EDITION", name_zh: "ROG 夜魔 EXTREME SPECIAL EDITION", kind: Kind::Keyboard },
    Device { pid: 0x1CF1, name_en: "ROG AZOTH EXTREME SPECIAL EDITION", name_zh: "ROG 夜魔 EXTREME SPECIAL EDITION", kind: Kind::Keyboard },
    Device { pid: 0x1D45, name_en: "PROART KEYBOARD KD300", name_zh: "PROART KEYBOARD KD300", kind: Kind::Keyboard },
    Device { pid: 0x1D46, name_en: "PROART KEYBOARD KD300", name_zh: "PROART KEYBOARD KD300", kind: Kind::Keyboard },
    Device { pid: 0x1D47, name_en: "PROART KEYBOARD KD300", name_zh: "PROART KEYBOARD KD300", kind: Kind::Keyboard },
    Device { pid: 0x1D68, name_en: "ASUS TX75 ANALOG GAMING KEYBOARD", name_zh: "华硕天选TX75", kind: Kind::Keyboard },
    Device { pid: 0x1D6A, name_en: "ASUS TX75 ANALOG GAMING KEYBOARD", name_zh: "华硕天选TX75", kind: Kind::Keyboard },
    Device { pid: 0x1DA2, name_en: "ROG Azoth Extreme Edition 20", name_zh: "ROG 夜魔 Extreme 20 周年版", kind: Kind::Keyboard },
    Device { pid: 0x1DA3, name_en: "ROG Azoth Extreme Edition 20", name_zh: "ROG 夜魔 Extreme 20 周年版", kind: Kind::Keyboard },
    Device { pid: 0x1DA4, name_en: "ROG Azoth Extreme Edition 20", name_zh: "ROG 夜魔 Extreme 20 周年版", kind: Kind::Keyboard },
    Device { pid: 0x1E0A, name_en: "ROG STRIX MORPH 96 X WIRELESS", name_zh: "ROG 术士 98 X", kind: Kind::Keyboard },
    Device { pid: 0x1E0B, name_en: "ROG STRIX MORPH 96 X WIRELESS", name_zh: "ROG 术士 98 X", kind: Kind::Keyboard },
    Device { pid: 0x1E0C, name_en: "ROG STRIX MORPH 96 X WIRELESS", name_zh: "ROG 术士 98 X", kind: Kind::Keyboard },
    Device { pid: 0x1E5B, name_en: "ROG STRIX MORPH 96 WIRELESS-PBZ", name_zh: "ROG 术士 98 - PBZ", kind: Kind::Keyboard },
    Device { pid: 0x1E5C, name_en: "ROG STRIX MORPH 96 WIRELESS-PBZ", name_zh: "ROG 术士 98 - PBZ", kind: Kind::Keyboard },
    Device { pid: 0x1E5D, name_en: "ROG STRIX MORPH 96 WIRELESS-PBZ", name_zh: "ROG 术士 98 - PBZ", kind: Kind::Keyboard },
    Device { pid: 0x1C64, name_en: "ROG RAIKIRI II PRO PC", name_zh: "ROG 雷切 II PRO PC", kind: Kind::Gamepad },
    Device { pid: 0x1C65, name_en: "ROG RAIKIRI II PRO PC", name_zh: "ROG 雷切 II PRO PC", kind: Kind::Gamepad },
    Device { pid: 0x1C91, name_en: "ROG RAIKIRI II XBOX WIRELESS", name_zh: "ROG 雷切 II XBOX无线手柄", kind: Kind::Gamepad },
    Device { pid: 0x1C92, name_en: "ROG RAIKIRI II XBOX WIRELESS", name_zh: "ROG 雷切 II XBOX无线手柄", kind: Kind::Gamepad },
];

/// 按 PID 查型号。
pub fn find(pid: u16) -> Option<&'static Device> {
    DEVICES.iter().find(|d| d.pid == pid)
}

/// 按 PID 查接收器。
pub fn find_dongle(pid: u16) -> Option<&'static Dongle> {
    DONGLES.iter().find(|d| d.pid == pid)
}

/// 型号名的解析顺序：有线 PID → 无线 PID → 固件自报名。
///
/// 表里的名字是人工校对过的统一译名，比固件自报的可靠；但表里没有的新型号
/// 仍然要靠固件自报，所以保留最后一级兜底。
pub fn display_name(model_name: &str, wired_pid: u16, wireless_pid: u16) -> String {
    for pid in [wired_pid, wireless_pid] {
        if pid == 0 {
            continue;
        }
        if let Some(d) = find(pid) {
            return d.name_zh.to_string();
        }
        if let Some(d) = find_dongle(pid) {
            return d.name_zh.to_string();
        }
    }
    model_name.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 型号表按_pid_能查到名字() {
        assert_eq!(find(0x1B65).unwrap().name_zh, "ROG 龙鳞 Ace Mini");
        assert_eq!(find(0x1B65).unwrap().kind, Kind::Mouse);
        assert!(find(0x1ACE).is_none(), "0x1ACE 是接收器，不在设备表里");
        assert_eq!(find_dongle(0x1ACE).unwrap().name_zh, "ROG OMNI 无线接收器");
    }

    #[test]
    fn 型号表优先于固件自报名() {
        // 固件自报的是英文缩写，表里有统一译名 → 用表里的
        assert_eq!(display_name("HARPE ACE MINI", 0x1B65, 0), "ROG 龙鳞 Ace Mini");
        // 有线 PID 为 0、只有无线 PID 时也能命中
        assert_eq!(display_name("", 0, 0x1B63), "ROG 龙鳞 Ace Mini");
    }

    #[test]
    fn 表里没有的型号退回固件自报名() {
        assert_eq!(display_name("ROG 某新型号", 0xFFFF, 0), "ROG 某新型号");
        assert_eq!(display_name("  ", 0xFFFF, 0), "");
    }

    #[test]
    fn 表里没有重复_pid() {
        let mut pids: Vec<u16> = DEVICES.iter().map(|d| d.pid).collect();
        pids.sort_unstable();
        let before = pids.len();
        pids.dedup();
        assert_eq!(before, pids.len(), "型号表里有重复 PID");
    }
}
