//! 完整导出包含 31 个 key=value 设置；恢复允许只提供部分字段。
//! 同一写入组中缺失的字段沿用新读的完整配置，整份预检通过后才写设备。

use crate::proto::{self, State, BUTTON_SLOTS, DPI_STAGES};

/// 备份文件的 31 个键，顺序即写文件的顺序。
pub const BACKUP_KEYS: [&str; 31] = [
    "poll_code",
    "dpi1_enc",
    "dpi1_rgb",
    "dpi2_enc",
    "dpi2_rgb",
    "dpi3_enc",
    "dpi3_rgb",
    "dpi4_enc",
    "dpi4_rgb",
    "active_stage",
    "debounce_code",
    "angle_snap",
    "angle_tuning",
    "lod",
    "light_mode",
    "light_bright",
    "light_rgb",
    "light_dir",
    "light_random",
    "light_speed",
    "button0",
    "button1",
    "button2",
    "button3",
    "button4",
    "button5",
    "button6",
    "button7",
    "poweroff_code",
    "low_warn",
    "full_effect",
];

fn rgb_hex(c: [u8; 3]) -> String {
    format!("{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// 解析 `RRGGBB` 或 `#RRGGBB`。
pub fn parse_rgb(s: &str) -> Result<[u8; 3], String> {
    let t = s.trim().trim_start_matches('#');
    if t.len() != 6 || !t.is_ascii() {
        return Err(format!("颜色必须是 6 位十六进制 RRGGBB，收到 {s:?}"));
    }
    let mut out = [0u8; 3];
    for i in 0..3 {
        out[i] = u8::from_str_radix(&t[i * 2..i * 2 + 2], 16)
            .map_err(|_| format!("颜色里的 {:?} 不是十六进制", &t[i * 2..i * 2 + 2]))?;
    }
    Ok(out)
}

/// 把状态渲染成备份文本。
pub fn render(configuration: &crate::snapshot::Configuration, link_mode: &str, report_id: u8, timestamp: &str) -> String {
    let st = configuration.state();
    let mut s = String::new();
    s.push_str("# X-Battery 设置备份 —— ROG 鼠标\r\n");
    s.push_str(&format!("# 由 X-Battery 生成于 {timestamp}\r\n"));
    s.push_str(&format!(
        "# 链路={link_mode} report_id=0x{report_id:02X}\r\n"
    ));
    s.push_str("#\r\n");
    s.push_str("# 注意：写完任何设置都必须 commit（0x50 0x03）才会进 flash，否则重连即丢。\r\n");
    s.push_str("# 注意：DPI 只能取 50 的整数倍，其他值会被设备静默截断。\r\n");
    s.push_str("#\r\n");
    // 表面标定值只作参考记录，不参与还原：写 LOD 会把它重置掉，写回去也没有对应命令。
    s.push_str(&format!(
        "# 表面标定参考（不参与还原）：surface_type={} material={} extra={:02x} k={:02x}{:02x}{:02x}\r\n",
        st.surface_type,
        st.material,
        st.surface_extra,
        st.calib[0],
        st.calib[1],
        st.calib[2]
    ));
    s.push_str("\r\n");

    s.push_str(&format!("poll_code={}\r\n", st.poll_code));
    for i in 0..DPI_STAGES {
        s.push_str(&format!("dpi{}_enc={}\r\n", i + 1, st.enc[i]));
        s.push_str(&format!("dpi{}_rgb={}\r\n", i + 1, rgb_hex(st.rgb[i])));
    }
    s.push_str(&format!("active_stage={}\r\n", st.active_stage));
    s.push_str(&format!("debounce_code={}\r\n", st.debounce_code));
    s.push_str(&format!("angle_snap={}\r\n", st.angle_snap));
    s.push_str(&format!("angle_tuning={}\r\n", st.angle_tuning));
    s.push_str(&format!("lod={}\r\n", st.lod));
    s.push_str(&format!("light_mode={}\r\n", st.light_mode));
    s.push_str(&format!("light_bright={}\r\n", st.light_bright));
    s.push_str(&format!("light_rgb={}\r\n", rgb_hex(st.light_rgb)));
    s.push_str(&format!("light_dir={}\r\n", st.light_dir));
    s.push_str(&format!("light_random={}\r\n", st.light_random));
    s.push_str(&format!("light_speed={}\r\n", st.light_speed));
    for i in 0..BUTTON_SLOTS {
        s.push_str(&format!("button{}={}\r\n", i, st.buttons[i]));
    }
    s.push_str(&format!("poweroff_code={}\r\n", st.sleep_mode));
    s.push_str(&format!("low_warn={}\r\n", st.low_warn));
    s.push_str(&format!("full_effect={}\r\n", st.full_effect));
    s
}

/// 缺失字段用 `None` 表示，恢复时保留对应基准值。
#[derive(Default, Debug, Clone)]
pub struct Backup {
    pub poll_code: Option<u8>,
    pub dpi_enc: [Option<i32>; DPI_STAGES],
    pub dpi_rgb: [Option<[u8; 3]>; DPI_STAGES],
    pub active_stage: Option<u8>,
    pub debounce_code: Option<u8>,
    pub angle_snap: Option<u8>,
    pub angle_tuning: Option<i16>,
    pub lod: Option<u8>,
    pub light_mode: Option<u8>,
    pub light_bright: Option<u8>,
    pub light_rgb: Option<[u8; 3]>,
    pub light_dir: Option<u8>,
    pub light_random: Option<u8>,
    pub light_speed: Option<u8>,
    pub buttons: [Option<u16>; BUTTON_SLOTS],
    pub poweroff_code: Option<u8>,
    pub low_warn: Option<u8>,
    pub full_effect: Option<u8>,
    /// 文件里实际出现的不同已知键数量。
    pub keys_found: usize,
    /// 解析失败、类型越界或重复的字段；恢复时整份拒绝。
    pub rejected: Vec<String>,
}

impl Backup {
    pub fn validate(&self) -> Result<(), String> {
        if self.keys_found == 0 {
            return Err("备份中没有任何已知设置".into());
        }
        if !self.rejected.is_empty() {
            return Err(format!("备份含不可用字段：{}", self.rejected.join("；")));
        }
        let mut invalid = Vec::new();
        let mut check = |key: &str, valid: bool| {
            if !valid { invalid.push(key.to_string()); }
        };
        check("poll_code", self.poll_code.is_none_or(|value| value <= 3));
        for (stage, encoded) in self.dpi_enc.iter().enumerate() {
            check(&format!("dpi{}_enc", stage + 1), encoded.is_none_or(|value|
                (proto::DPI_ENC_MIN..=proto::DPI_ENC_MAX).contains(&value)));
        }
        check("active_stage", self.active_stage.is_none_or(|value| (1..=DPI_STAGES as u8).contains(&value)));
        check("debounce_code", self.debounce_code.is_none_or(|value| proto::debounce_ms(value).is_some()));
        check("angle_snap", self.angle_snap.is_none_or(|value| value <= 1));
        check("angle_tuning", self.angle_tuning.is_none_or(|value| (-20..=20).contains(&value) && value % 5 == 0));
        check("lod", self.lod.is_none_or(|value| value <= 1));
        check("light_mode", self.light_mode.is_none_or(proto::light_mode_supported));
        check("light_bright", self.light_bright.is_none_or(|value| value <= 100));
        check("light_dir", self.light_dir.is_none_or(|value| value <= 1));
        check("light_random", self.light_random.is_none_or(|value| value <= 1));
        check("light_speed", self.light_speed.is_none_or(|value| value <= 100));
        check("poweroff_code", self.poweroff_code.is_none_or(proto::sleep_code_valid));
        check("low_warn", self.low_warn.is_none_or(|value| value <= 100));
        check("full_effect", self.full_effect.is_none_or(|value| value <= 1));
        if invalid.is_empty() { Ok(()) } else {
            Err(format!("备份字段超出支持范围：{}", invalid.join("、")))
        }
    }

    pub fn merge(&self, before: &State) -> Result<State, String> {
        let mut complete = self.clone();
        for stage in 0..DPI_STAGES {
            if complete.dpi_enc[stage].is_some() || complete.dpi_rgb[stage].is_some() {
                complete.dpi_enc[stage].get_or_insert(before.enc[stage]);
                complete.dpi_rgb[stage].get_or_insert(before.rgb[stage]);
            }
        }
        if self.light_mode.is_some() || self.light_bright.is_some() || self.light_rgb.is_some()
            || self.light_dir.is_some() || self.light_random.is_some() || self.light_speed.is_some()
        {
            complete.light_mode.get_or_insert(before.light_mode);
            complete.light_bright.get_or_insert(before.light_bright);
            complete.light_rgb.get_or_insert(before.light_rgb);
            complete.light_dir.get_or_insert(before.light_dir);
            complete.light_random.get_or_insert(before.light_random);
            complete.light_speed.get_or_insert(before.light_speed);
        }
        if self.poweroff_code.is_some() || self.low_warn.is_some() || self.full_effect.is_some() {
            complete.poweroff_code.get_or_insert(before.sleep_mode);
            complete.low_warn.get_or_insert(before.low_warn);
            complete.full_effect.get_or_insert(before.full_effect);
        }
        complete.validate()?;
        let mut state = before.clone();
        for stage in 0..DPI_STAGES {
            if let Some(value) = complete.dpi_enc[stage] { state.enc[stage] = value; }
            if let Some(value) = complete.dpi_rgb[stage] { state.rgb[stage] = value; }
        }
        if let Some(value) = complete.poll_code { state.poll_code = value; }
        if let Some(value) = complete.active_stage { state.active_stage = value; }
        if let Some(value) = complete.debounce_code { state.debounce_code = value; }
        if let Some(value) = complete.angle_snap { state.angle_snap = value; }
        if let Some(value) = complete.angle_tuning { state.angle_tuning = value; }
        if let Some(value) = complete.lod { state.lod = value; }
        if let Some(value) = complete.light_mode { state.light_mode = value; }
        if let Some(value) = complete.light_bright { state.light_bright = value; }
        if let Some(value) = complete.light_rgb { state.light_rgb = value; }
        if let Some(value) = complete.light_dir { state.light_dir = value; }
        if let Some(value) = complete.light_random { state.light_random = value; }
        if let Some(value) = complete.light_speed { state.light_speed = value; }
        for slot in 0..BUTTON_SLOTS {
            if let Some(value) = complete.buttons[slot] { state.buttons[slot] = value; }
        }
        if let Some(value) = complete.poweroff_code { state.sleep_mode = value; }
        if let Some(value) = complete.low_warn { state.low_warn = value; }
        if let Some(value) = complete.full_effect { state.full_effect = value; }
        Ok(state)
    }
}

struct Reader<'a> {
    map: &'a [(String, String)],
    rejected: Vec<String>,
}

impl<'a> Reader<'a> {
    fn new(map: &'a [(String, String)]) -> Self {
        Self {
            map,
            rejected: Vec::new(),
        }
    }

    fn raw(&self, key: &str) -> Option<&'a str> {
        self.map
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn reject(&mut self, key: &str, why: String) {
        self.rejected.push(format!("{key}：{why}"));
    }

    fn i32(&mut self, key: &str) -> Option<i32> {
        let raw = self.raw(key)?;
        match raw.trim().parse::<i32>() {
            Ok(v) => Some(v),
            Err(_) => {
                self.reject(key, format!("{raw:?} 不是整数"));
                None
            }
        }
    }

    fn u32(&mut self, key: &str) -> Option<u32> {
        let raw = self.raw(key)?;
        match raw.trim().parse::<u32>() {
            Ok(v) => Some(v),
            Err(_) => {
                self.reject(key, format!("{raw:?} 不是非负整数"));
                None
            }
        }
    }

    fn u8(&mut self, key: &str) -> Option<u8> {
        let v = self.u32(key)?;
        match u8::try_from(v) {
            Ok(v) => Some(v),
            Err(_) => {
                self.reject(key, format!("{v} 超出 0..=255"));
                None
            }
        }
    }

    fn u16(&mut self, key: &str) -> Option<u16> {
        let v = self.u32(key)?;
        match u16::try_from(v) {
            Ok(v) => Some(v),
            Err(_) => {
                self.reject(key, format!("{v} 超出 0..=65535"));
                None
            }
        }
    }

    /// 保留符号，支持范围由恢复预检统一校验。
    fn i16(&mut self, key: &str) -> Option<i16> {
        let v = self.i32(key)?;
        match i16::try_from(v) {
            Ok(v) => Some(v),
            Err(_) => {
                self.reject(key, format!("{v} 超出 -32768..=32767"));
                None
            }
        }
    }

    fn rgb(&mut self, key: &str) -> Option<[u8; 3]> {
        let raw = self.raw(key)?;
        match parse_rgb(raw) {
            Ok(c) => Some(c),
            Err(e) => {
                self.reject(key, e);
                None
            }
        }
    }
}

/// 解析备份文本。
pub fn parse(text: &str) -> Result<Backup, String> {
    let mut map: Vec<(String, String)> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(eq) = line.find('=') else {
            continue;
        };
        let key = line[..eq].trim().to_string();
        let val = line[eq + 1..].trim().to_string();
        if key.is_empty() {
            continue;
        }
        map.push((key, val));
    }

    let mut b = Backup::default();
    b.keys_found = BACKUP_KEYS
        .iter()
        .filter(|k| map.iter().any(|(kk, _)| kk == **k))
        .count();

    let mut r = Reader::new(&map);
    b.poll_code = r.u8("poll_code");
    for i in 0..DPI_STAGES {
        let ek = format!("dpi{}_enc", i + 1);
        let ck = format!("dpi{}_rgb", i + 1);
        b.dpi_enc[i] = r.i32(&ek);
        b.dpi_rgb[i] = r.rgb(&ck);
    }
    b.active_stage = r.u8("active_stage");
    b.debounce_code = r.u8("debounce_code");
    b.angle_snap = r.u8("angle_snap");
    b.angle_tuning = r.i16("angle_tuning");
    b.lod = r.u8("lod");
    b.light_mode = r.u8("light_mode");
    b.light_bright = r.u8("light_bright");
    b.light_rgb = r.rgb("light_rgb");
    b.light_dir = r.u8("light_dir");
    b.light_random = r.u8("light_random");
    b.light_speed = r.u8("light_speed");
    for i in 0..BUTTON_SLOTS {
        let key = format!("button{i}");
        b.buttons[i] = r.u16(&key);
    }
    b.poweroff_code = r.u8("poweroff_code");
    b.low_warn = r.u8("low_warn");
    b.full_effect = r.u8("full_effect");
    b.rejected = r.rejected;
    for key in BACKUP_KEYS {
        if map.iter().filter(|(found, _)| found == key).count() > 1 {
            b.rejected.push(format!("{key}：重复字段"));
        }
    }

    if b.keys_found == 0 {
        return Err("备份文件里没有任何已知的键，可能不是本程序生成的文件".into());
    }
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_accepts_supported_boundary_values() {
        for text in [
            "poll_code=0\ndpi1_enc=1\nactive_stage=1\ndebounce_code=0\nangle_snap=0\nangle_tuning=-20\nlod=0\nlight_mode=0\nlight_bright=0\nlight_dir=0\nlight_random=0\nlight_speed=0\nbutton0=0\npoweroff_code=0\nlow_warn=0\nfull_effect=0",
            "poll_code=3\ndpi4_enc=839\nactive_stage=4\ndebounce_code=7\nangle_snap=1\nangle_tuning=20\nlod=1\nlight_mode=240\nlight_bright=100\nlight_dir=1\nlight_random=1\nlight_speed=100\nbutton7=65535\npoweroff_code=255\nlow_warn=100\nfull_effect=1",
        ] {
            parse(text).unwrap().validate().unwrap();
        }
        for mode in [0, 1, 2, 4, 6, 240] {
            parse(&format!("light_mode={mode}")).unwrap().validate().unwrap();
        }
        for code in [0, 1, 2, 3, 4, 255] {
            parse(&format!("poweroff_code={code}")).unwrap().validate().unwrap();
        }
    }

    #[test]
    fn validation_reports_all_invalid_fields_together() {
        let backup = parse("poll_code=4\ndpi4_enc=840\nactive_stage=0\nlight_mode=3\npoweroff_code=5").unwrap();
        let error = backup.validate().unwrap_err();
        for key in ["poll_code", "dpi4_enc", "active_stage", "light_mode", "poweroff_code"] {
            assert!(error.contains(key), "{error}");
        }
    }

    #[test]
    fn duplicate_known_fields_reject_the_whole_backup() {
        for text in [
            "low_warn=20\nlow_warn=101",
            "light_rgb=abcdef\nlight_rgb=invalid",
            "poll_code=3\npoll_code=3",
        ] {
            let backup = parse(text).unwrap();
            assert_eq!(backup.keys_found, 1);
            assert!(backup.validate().unwrap_err().contains("重复字段"));
        }
    }

    #[test]
    fn validation_rejects_empty_programmatic_backups() {
        assert!(Backup::default().validate().is_err());
    }

    #[test]
    fn unrelated_baseline_groups_are_preserved_without_revalidation() {
        let before = State { light_mode: 3, sleep_mode: 5, ..Default::default() };
        let merged = parse("poll_code=2").unwrap().merge(&before).unwrap();
        assert_eq!(merged.poll_code, 2);
        assert_eq!(merged.light_mode, 3);
        assert_eq!(merged.sleep_mode, 5);
    }

    #[test]
    fn rgb_accepts_hex_colors_with_optional_prefix_and_whitespace() {
        for (input, expected) in [
            ("000000", [0, 0, 0]),
            ("ffffff", [255, 255, 255]),
            ("ABCDEF", [0xab, 0xcd, 0xef]),
            ("aBcD12", [0xab, 0xcd, 0x12]),
            ("#123456", [0x12, 0x34, 0x56]),
            (" \t#123456\r\n", [0x12, 0x34, 0x56]),
        ] {
            assert_eq!(parse_rgb(input).unwrap(), expected, "{input:?}");
        }
    }

    #[test]
    fn rgb_rejects_six_byte_unicode_without_panicking() {
        for input in ["中文", "a中bc", "12é34", "ab🦀", "a\u{0301}bcd"] {
            assert_eq!(input.len(), 6);
            assert!(parse_rgb(input).is_err(), "{input:?}");
            assert!(parse_rgb(&format!("#{input}")).is_err(), "{input:?}");
        }
    }

    #[test]
    fn rgb_rejects_invalid_lengths_and_non_hex_ascii() {
        for input in ["", "#", "12345", "1234567", "gg0000", "12 456", "12\0abc"] {
            assert!(parse_rgb(input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn backup_records_unicode_colors_as_rejected_fields() {
        let parsed = parse("dpi1_rgb=中文\ndpi2_rgb=a中bc\nlight_rgb=ab🦀\npoll_code=3").unwrap();
        assert_eq!(parsed.poll_code, Some(3));
        assert_eq!(parsed.dpi_rgb, [None; DPI_STAGES]);
        assert_eq!(parsed.light_rgb, None);
        assert_eq!(parsed.rejected.len(), 3);
        for key in ["dpi1_rgb", "dpi2_rgb", "light_rgb"] {
            assert!(parsed.rejected.iter().any(|message| message.starts_with(key)));
        }
    }
}
