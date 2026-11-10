//! 备份文件的读写与还原。
//!
//! 备份格式是 `key=value` 的 ini 风格纯文本，**恰好 31 个键**：
//!
//! ```text
//! poll_code, dpi{1..4}_enc, dpi{1..4}_rgb, active_stage,
//! debounce_code, angle_snap, angle_tuning, lod,
//! light_mode, light_bright, light_rgb, light_dir, light_random, light_speed,
//! button0..button7, poweroff_code, low_warn, full_effect
//! ```
//!
//! 其中**能量块（poweroff_code / low_warn / full_effect）必须存在** ——
//! 文档第 5 节的事故就是因为最初的备份漏了这三项，commit 之后休眠时间字节被写坏，
//! 鼠标变成 1 分钟就休眠，看起来像坏了。

use crate::proto::{State, BUTTON_SLOTS, DPI_STAGES};

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
    if t.len() != 6 {
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
pub fn render(st: &State, link_mode: &str, report_id: u8, timestamp: &str) -> String {
    let mut s = String::new();
    s.push_str("# R Battery (rog-mouse-ctl) 设置备份 —— ROG 鼠标\r\n");
    s.push_str(&format!("# 由 rog-mouse-ctl 生成于 {timestamp}\r\n"));
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
    // 能量块必须最后写，也必须存在 —— 见文件头注释。
    s.push_str(&format!("poweroff_code={}\r\n", st.sleep_mode));
    s.push_str(&format!("low_warn={}\r\n", st.low_warn));
    s.push_str(&format!("full_effect={}\r\n", st.full_effect));
    s
}

/// 解析后的备份。缺键用 `None` 表示（还原时跳过）。
#[derive(Default, Debug)]
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
    /// 文件里实际出现的已知键数量（用于校验「31 个键」）
    pub keys_found: usize,
    /// 值存在但解析失败/超范围的键。调用方应把它们提示给用户 ——
    /// 静默丢弃会让「还原成功」变成谎话。
    pub rejected: Vec<String>,
}

/// 备份字段的类型化读取。
///
/// 原来的 `get_u8`/`get_u16` 是 `get_u32(...) as u8` —— 静默截断：文件里
/// `poll_code=256` 会变成 0，`angle_tuning=-90` 因为 `parse::<u32>` 失败直接
/// 变成 `None`，**负角度整个丢失**（而设备确实会写出负值）。这里改成范围
/// 检查：超范围或解析失败都记进 `rejected`，由调用方决定怎么提示，不再悄悄
/// 改写数值。
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

    /// 角度微调是**有符号**的：设备会返回 -90..90。
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

    if b.keys_found == 0 {
        return Err("备份文件里没有任何已知的键，可能不是本程序生成的文件".into());
    }
    Ok(b)
}

/// 备份缺少能量块时给出明确警告 —— 这正是文档第 5 节事故的根因。
pub fn energy_missing(b: &Backup) -> bool {
    b.poweroff_code.is_none() && b.low_warn.is_none() && b.full_effect.is_none()
}
