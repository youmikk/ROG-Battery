//! 协议层：读状态、写寄存器、备份/还原。
//!
//! 所有偏移都遵循文档约定：**下标 0 是 report id**（`hid.rs` 的 `xfer` 已经做过对齐，
//! 无论设备回包是从 report id 还是从 cmd 字节开始）。
//!
//! 写入是**一个寄存器文件**：`cmd 0x51 / sub 0x31 / 寄存器号`，改完用 `cmd 0x50 / sub 0x03` 提交进 flash。
//! 另有三个独立命令族：LOD（`0x35`）、灯光（`0x28`）、按键（`0x21`）。

use crate::hid::{is_nak, HidDevice, DEFAULT_TIMEOUT_MS};

/// DPI 档位数量（本型号固定 4）
pub const DPI_STAGES: usize = 4;
/// DPI 步进。只能写 50 的整数倍，否则被设备静默截断。
pub const DPI_STEP: i32 = 50;
/// DPI 允许范围（含两端）。用户可写范围，`ops::set_dpi` 用它做校验。
pub const DPI_MIN: i32 = 100;
pub const DPI_MAX: i32 = 42000;
/// 由 [`DPI_MIN`] / [`DPI_MAX`] 推出的编码边界。
///
/// `DPI_ENC_MIN = (100-50)/50 = 1`（对应 100 DPI），
/// `DPI_ENC_MAX = (42000-50)/50 = 839`（对应 42000 DPI）。
///
/// **注意下限是 1 不是 0**：`decode_dpi(0) == 50`，低于 [`DPI_MIN`]。
/// 设备本身接受 50 DPI，但我们统一的对外范围是 100 起，所以编码 0 要拒掉。
///
/// 存在的理由：`restore` 路径直接写编码值、绕过了 `ops::set_dpi` 的范围校验，
/// 需要在这里有个权威边界可查。**不要硬编码 1 / 839** —— 万一将来改
/// [`DPI_MIN`] / [`DPI_MAX`] / [`DPI_STEP`]，这两个常量要跟着走。
pub const DPI_ENC_MIN: i32 = (DPI_MIN - DPI_STEP) / DPI_STEP;
pub const DPI_ENC_MAX: i32 = (DPI_MAX - DPI_STEP) / DPI_STEP;
/// 按键槽位数量
pub const BUTTON_SLOTS: usize = 8;
/// 本型号唯一灯区（滚轮）的设备索引
pub const LIGHT_ZONE: u8 = 0;

/// 8 个按键槽位的物理键源码，顺序固定、不可改（改的是 dst）。
pub const BUTTON_SRC: [u16; BUTTON_SLOTS] = [
    0x01F0, // 0 左键
    0x01F1, // 1 右键
    0x01F2, // 2 滚轮按下
    0x01E4, // 3 侧键后退
    0x01E5, // 4 侧键前进
    0x01E6, // 5 DPI 键
    0x01E8, // 6 滚轮上
    0x01E9, // 7 滚轮下
];

pub const BUTTON_NAME: [&str; BUTTON_SLOTS] = [
    "左键",
    "右键",
    "滚轮按下",
    "侧键后退",
    "侧键前进",
    "DPI键",
    "滚轮上",
    "滚轮下",
];

// ==================== 编解码 ====================

/// DPI 编码：`enc = (dpi - 50) / 50`（非整数倍按截断处理），并夹到合法区间。
///
/// 必须夹取。原实现 `dpi < 50 → 0`（解码回来 50 DPI，低于设备下限 100）、
/// `dpi = 50000 → 999`（远超 `DPI_ENC_MAX`）：这两个值写进鼠标后设备行为
/// 未定义，而界面上完全看不出发生过什么。`ops` 层已经做过范围校验，这里是
/// 最后一道防线，防止将来新调用方直接调进来。
pub fn encode_dpi(dpi: i32) -> i32 {
    encode_dpi_checked(dpi).0
}

/// 同 [`encode_dpi`]，另外告诉你「输入被夹取过没有」。
///
/// 返回 `(编码值, 是否被改过)`。调用方可以据此提示用户「你输入的值会被设备
/// 改成 X」，而不是默默写一个不一样的数。
pub fn encode_dpi_checked(dpi: i32) -> (i32, bool) {
    // 截断语义保持不变（`ops` 的「步进提醒」就是按截断算的），只补夹取。
    let raw = (dpi - DPI_STEP) / DPI_STEP;
    let enc = raw.clamp(DPI_ENC_MIN, DPI_ENC_MAX);
    (enc, enc != raw)
}

/// DPI 解码：`dpi = enc * 50 + 50`
pub fn decode_dpi(enc: i32) -> i32 {
    enc * DPI_STEP + DPI_STEP
}

/// 回报率 → 代码：0=125 1=250 2=500 3=1000
pub fn poll_code_of(hz: i32) -> Option<u8> {
    match hz {
        125 => Some(0),
        250 => Some(1),
        500 => Some(2),
        1000 => Some(3),
        _ => None,
    }
}

pub fn poll_name(code: u8) -> String {
    match code {
        0 => "125Hz".into(),
        1 => "250Hz".into(),
        2 => "500Hz".into(),
        3 => "1000Hz".into(),
        // 本型号没有 2000 以上（7D 20 的 booster 位为 0），但保留映射以免读出错值
        4 => "2000Hz".into(),
        5 => "4000Hz".into(),
        6 => "8000Hz".into(),
        other => format!("code{other}"),
    }
}

/// 去抖代码 → 毫秒（0 = 关闭）
pub fn debounce_ms(code: u8) -> Option<u32> {
    match code {
        0 => Some(0),
        1 => Some(8),
        2 => Some(12),
        3 => Some(16),
        4 => Some(20),
        5 => Some(24),
        6 => Some(28),
        7 => Some(32),
        _ => None,
    }
}

pub fn debounce_name(code: u8) -> String {
    match debounce_ms(code) {
        Some(0) => "关闭".into(),
        Some(ms) => format!("{ms}ms"),
        None => format!("code{code}"),
    }
}

/// 休眠设置里表示「永不自动关机」的代码。
///
/// 语义来源是**两个互不相关的第三方逆向实现**，逐项一致：
///
/// - G-Helper `_ghelper\AsusMouse.cs:9-17`：
///   `enum PowerOffSetting { Minutes1=0, Minutes2=1, Minutes3=2, Minutes5=3, Minutes10=4, Never=0xFF }`
/// - RigStats `_research\_rigstats\Mouse_MouseProtocol.cs:71-80`：
///   `PowerOffMinutes = { [0]=1, [1]=2, [2]=3, [3]=5, [4]=10, [0xFF]=null }`
///
/// 另外 `4 → 10 分钟` 有物理实测佐证（鼠标空闲 600 秒整正好入睡）。
///
/// ⚠️ 这是**枚举值不是分钟数** —— 5..254 全是无定义值。
pub const SLEEP_NEVER: u8 = 0xFF;

/// 休眠档位是不是一个有定义的值。
///
/// 真机实测（2026-10-21，逐值写入 + 读回 15 次）：设备**完全不校验**这个字节，
/// 写 5/6/7/8/9/15/16/100/254 全部原样存进 flash。所以：
///
/// **这里的范围校验不是防御性编程，而是唯一的安全网** —— 一旦让 5..254 这种
/// 无定义值落进设备，没有任何命令能读回它的语义，也没有恢复命令。
pub fn sleep_code_valid(code: u8) -> bool {
    matches!(code, 0..=4 | SLEEP_NEVER)
}

/// 休眠时间代码 → 人话。
///
/// 映射本身已由两个独立的第三方实现确认（见 [`SLEEP_NEVER`]），但
/// **1/2/3 档的实际触发时长没有逐档掐表验证过** —— 值是从枚举名推出来的。
/// `0`（事故中观察到「1 分钟」）、`4`（空闲 600 秒入睡）、`0xFF`（事故中
/// 观察到永不休眠）是物理实测的。这个区别对用户没影响，UI 正常显示即可。
pub fn sleep_mode_name(code: u8) -> String {
    match code {
        0 => "1 分钟".into(),
        1 => "2 分钟".into(),
        2 => "3 分钟".into(),
        3 => "5 分钟".into(),
        4 => "10 分钟".into(),
        0xFF => "永不".into(),
        other => format!("code{other}"),
    }
}

pub fn lod_name(code: u8) -> String {
    match code {
        0 => "Low".into(),
        1 => "High".into(),
        other => format!("code{other}"),
    }
}

pub fn light_mode_name(code: u8) -> String {
    match code {
        0x00 => "Static 常亮".into(),
        0x01 => "Breathing 呼吸".into(),
        0x02 => "ColorCycle 彩色循环".into(),
        0x03 => "Rainbow 彩虹（本型号不支持）".into(),
        0x04 => "React 按键反应".into(),
        0x05 => "Comet 彗星（本型号不支持）".into(),
        0x06 => "BatteryState 电量指示".into(),
        0xF0 => "Off 关灯".into(),
        other => format!("code{other}"),
    }
}

/// 本型号支持的灯光模式。Rainbow(3) 与 Comet(5) 会被设备**静默回落**到 ColorCycle，
/// 因此在 CLI 入口就拒绝掉，不让用户以为设置成功了。
pub fn light_mode_supported(code: u8) -> bool {
    matches!(code, 0x00 | 0x01 | 0x02 | 0x04 | 0x06 | 0xF0)
}

// ==================== 状态 ====================

/// 设备的完整可读写状态。备份文件就是它的文本形式。
#[derive(Default, Clone)]
pub struct State {
    // —— 性能块（12 04 00）——
    pub poll_code: u8,
    pub debounce_code: u8,
    pub angle_snap: u8,
    pub angle_tuning: i16,

    // —— DPI 数值与颜色（12 04 02 / 12 04 03）——
    pub enc: [i32; DPI_STAGES],
    pub rgb: [[u8; 3]; DPI_STAGES],

    // —— profile 与当前档位（12 00 00）——
    pub profile: u8,
    pub active_stage: u8,

    // —— 能量块（12 07 00）——
    pub power: u8,
    pub sleep_mode: u8,
    pub low_warn: u8,
    pub voltage_mv: u16,
    pub charging: u8,
    pub battery_type: u8,
    pub full_effect: u8,

    // —— LOD 与表面标定（12 06 00）——
    pub surface_type: u8,
    pub material: u8,
    /// `12 06` 的 b7，语义未确认（实测恒 0xFF），原样保留以便对照
    pub surface_extra: u8,
    pub lod: u8,
    /// 表面标定 k 值（写 LOD 会重置它，备份时必须记下来）
    pub calib: [u8; 3],

    // —— 灯光（12 03 00）——
    pub light_mode: u8,
    pub light_bright: u8,
    pub light_rgb: [u8; 3],
    pub light_dir: u8,
    pub light_random: u8,
    pub light_speed: u8,

    // —— 按键（12 05 00）——
    pub buttons: [u16; BUTTON_SLOTS],
}

impl State {
    /// 取某档 DPI 的解析值。
    ///
    /// `stage` 来自界面/备份文件/CLI 参数，越界时按最后一档处理 —— 这类下标
    /// 越界不该让整个进程 panic。逻辑上的范围校验在 `ops` 层已经做过。
    pub fn dpi(&self, stage: usize) -> i32 {
        decode_dpi(self.enc[stage.min(DPI_STAGES - 1)])
    }

    pub fn rgb_hex(&self, stage: usize) -> String {
        let c = self.rgb[stage.min(DPI_STAGES - 1)];
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    }

    pub fn light_rgb_hex(&self) -> String {
        format!(
            "#{:02x}{:02x}{:02x}",
            self.light_rgb[0], self.light_rgb[1], self.light_rgb[2]
        )
    }
}

// ==================== 读 ====================

/// 解析性能块 `12 04 00`：回报率在 b13、去抖 b15、角度吸附 b17、角度微调 b19..20（signed LE16）。
///
/// 纯函数：只吃字节，不碰设备。缓存里的原始帧也走这里，保证「读设备」和「读缓存」结果一致。
pub fn parse_perf(m: &[u8], st: &mut State) -> Result<(), String> {
    if is_nak(m) {
        return Err("设备 NAK 了 12 04 00".into());
    }
    if m.len() < 21 {
        return Err(format!("12 04 00 回帧过短：{} 字节", m.len()));
    }
    // 本型号 booster 位为 0，高 4 位为 0 时取低 3 位
    let raw = m[13];
    let high = (raw >> 4) & 0x0F;
    st.poll_code = if high > 0 { high } else { raw & 0x07 };
    st.debounce_code = m[15];
    st.angle_snap = m[17];
    st.angle_tuning = (m[19] as i16) | ((m[20] as i16) << 8);
    Ok(())
}

/// 解析 4 档 DPI 数值 `12 04 02`。
pub fn parse_dpi(m: &[u8], st: &mut State) -> Result<(), String> {
    if is_nak(m) || m.len() < 5 + DPI_STAGES * 4 {
        return Err("12 04 02 回帧无效".into());
    }
    for i in 0..DPI_STAGES {
        let at = 5 + i * 4;
        st.enc[i] = m[at] as i32 | ((m[at + 1] as i32) << 8);
    }
    Ok(())
}

/// 解析 4 档颜色 `12 04 03`。
pub fn parse_colors(m: &[u8], st: &mut State) -> Result<(), String> {
    if is_nak(m) || m.len() < 5 + DPI_STAGES * 3 {
        return Err("12 04 03 回帧无效".into());
    }
    for i in 0..DPI_STAGES {
        let at = 5 + i * 3;
        st.rgb[i] = [m[at], m[at + 1], m[at + 2]];
    }
    Ok(())
}

/// 解析 profile 与当前 DPI 档位 `12 00 00`：b11 = profile（0 起），b12 = 当前档位（1 起）。
pub fn parse_profile(m: &[u8], st: &mut State) -> Result<(), String> {
    if is_nak(m) || m.len() < 13 {
        return Err("12 00 00 回帧无效".into());
    }
    st.profile = m[11];
    st.active_stage = m[12];
    Ok(())
}

/// 解析电量与能量块 `12 07 00`：
/// b5 电量 b6 休眠 b7 低电量阈值 b8..9 电压mV b10 充电 b11 电池类型 b12 充满效果。
pub fn parse_energy(m: &[u8], st: &mut State) -> Result<(), String> {
    if is_nak(m) || m.len() < 13 {
        return Err("12 07 00 回帧无效".into());
    }
    st.power = m[5];
    st.sleep_mode = m[6];
    st.low_warn = m[7];
    st.voltage_mv = m[8] as u16 | ((m[9] as u16) << 8);
    st.charging = m[10];
    st.battery_type = m[11];
    st.full_effect = m[12];
    Ok(())
}

/// 解析 LOD 与表面标定 `12 06 00`：b5/b6 表面类型与材质、b7 未用、b8 LOD、b9..b11 标定 k 值。
pub fn parse_lod(m: &[u8], st: &mut State) -> Result<(), String> {
    if is_nak(m) || m.len() < 12 {
        return Err("12 06 00 回帧无效".into());
    }
    st.surface_type = m[5];
    st.material = m[6];
    st.surface_extra = m[7];
    st.lod = m[8];
    st.calib = [m[9], m[10], m[11]];
    Ok(())
}

/// 解析灯光 `12 03 00`：b5 模式 b6 亮度 b7..9 RGB b11 方向 b12 随机 b13 速度。
pub fn parse_light(m: &[u8], st: &mut State) -> Result<(), String> {
    if is_nak(m) || m.len() < 14 {
        return Err("12 03 00 回帧无效".into());
    }
    st.light_mode = m[5];
    st.light_bright = m[6];
    st.light_rgb = [m[7], m[8], m[9]];
    st.light_dir = m[11];
    st.light_random = m[12];
    st.light_speed = m[13];
    Ok(())
}

/// 解析按键绑定 `12 05 00`：槽位 i 的动作码在 `b[5+2i]`（LE16）。
pub fn parse_buttons(m: &[u8], st: &mut State) -> Result<(), String> {
    if is_nak(m) || m.len() < 5 + BUTTON_SLOTS * 2 {
        return Err("12 05 00 回帧无效".into());
    }
    for i in 0..BUTTON_SLOTS {
        let at = 5 + i * 2;
        st.buttons[i] = m[at] as u16 | ((m[at + 1] as u16) << 8);
    }
    Ok(())
}

/// 读一帧并交给解析函数。
fn read_frame(
    dev: &mut HidDevice,
    rid: u8,
    cmd: u8,
    sub: u8,
    tail: &[u8],
    parse: impl Fn(&[u8], &mut State) -> Result<(), String>,
    st: &mut State,
) -> Result<(), String> {
    let m = dev
        .xfer(rid, cmd, sub, tail, DEFAULT_TIMEOUT_MS)
        .ok_or("鼠标无应答（休眠中？先移动一下鼠标）")?;
    parse(&m, st)
}

fn read_perf(dev: &mut HidDevice, rid: u8, st: &mut State) -> Result<(), String> {
    read_frame(dev, rid, 0x12, 0x04, &[0x00, 0x00], parse_perf, st)
}

fn read_dpi(dev: &mut HidDevice, rid: u8, st: &mut State) -> Result<(), String> {
    read_frame(dev, rid, 0x12, 0x04, &[0x02, 0x00], parse_dpi, st)
}

fn read_colors(dev: &mut HidDevice, rid: u8, st: &mut State) -> Result<(), String> {
    read_frame(dev, rid, 0x12, 0x04, &[0x03, 0x00], parse_colors, st)
}

fn read_profile(dev: &mut HidDevice, rid: u8, st: &mut State) -> Result<(), String> {
    read_frame(dev, rid, 0x12, 0x00, &[0x00, 0x00], parse_profile, st)
}

fn read_energy(dev: &mut HidDevice, rid: u8, st: &mut State) -> Result<(), String> {
    read_frame(dev, rid, 0x12, 0x07, &[0x00, 0x00], parse_energy, st)
}

fn read_lod(dev: &mut HidDevice, rid: u8, st: &mut State) -> Result<(), String> {
    read_frame(dev, rid, 0x12, 0x06, &[0x00, 0x00], parse_lod, st)
}

fn read_light(dev: &mut HidDevice, rid: u8, st: &mut State) -> Result<(), String> {
    read_frame(dev, rid, 0x12, 0x03, &[LIGHT_ZONE, 0x00], parse_light, st)
}

fn read_buttons(dev: &mut HidDevice, rid: u8, st: &mut State) -> Result<(), String> {
    read_frame(dev, rid, 0x12, 0x05, &[0x00, 0x00], parse_buttons, st)
}

/// 读完整状态。共 8 帧，实测约 0.25–0.5 秒。
///
/// 刻意**不读 MotionSync**（`12 04 04`）：本型号不响应，一次 MISS 要等满 2400ms，
/// 为永远不来的值付这个代价没有意义。
pub fn read_state(dev: &mut HidDevice, rid: u8) -> Result<State, String> {
    let mut st = State::default();
    read_perf(dev, rid, &mut st)?;
    read_dpi(dev, rid, &mut st)?;
    read_colors(dev, rid, &mut st)?;
    read_profile(dev, rid, &mut st)?;
    read_energy(dev, rid, &mut st)?;
    read_lod(dev, rid, &mut st)?;
    read_light(dev, rid, &mut st)?;
    read_buttons(dev, rid, &mut st)?;
    Ok(st)
}

/// 只读能量块。`restore` 结尾要用它做「能量块是否真的写进去了」的断言。
pub fn read_energy_only(dev: &mut HidDevice, rid: u8) -> Result<State, String> {
    let mut st = State::default();
    read_energy(dev, rid, &mut st)?;
    Ok(st)
}

/// 只读 LOD 块（含标定值）。
pub fn read_lod_only(dev: &mut HidDevice, rid: u8) -> Result<State, String> {
    let mut st = State::default();
    read_lod(dev, rid, &mut st)?;
    Ok(st)
}

// ==================== 身份信息 ====================

/// 身份信息（只读，`info` 用）
#[derive(Default, Clone)]
pub struct Identity {
    pub model_name: String,
    pub wired_pid: u16,
    pub wireless_pid: u16,
    pub mouse_sn: String,
    pub dongle_sn: String,
    pub mouse_mac: String,
    pub dongle_mac: String,
    pub dongle_firmware: String,
    pub booster: bool,
}

impl Identity {
    /// 界面上显示的型号名：PID 对照表优先，其次固件自报名（见 [`crate::devices`]）。
    pub fn display_name(&self) -> String {
        crate::devices::display_name(&self.model_name, self.wired_pid, self.wireless_pid)
    }
}

fn ascii_at(bytes: &[u8], from: usize) -> String {
    if from >= bytes.len() {
        return String::new();
    }
    let end = bytes[from..]
        .iter()
        .position(|&b| b == 0)
        .map(|p| from + p)
        .unwrap_or(bytes.len());
    bytes[from..end]
        .iter()
        .take_while(|&&b| (0x20..0x7F).contains(&b))
        .map(|&b| b as char)
        .collect::<String>()
        .trim()
        .to_string()
}

fn mac_at(bytes: &[u8], from: usize) -> String {
    if from + 6 > bytes.len() {
        return String::new();
    }
    let b = &bytes[from..from + 6];
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        b[0], b[1], b[2], b[3], b[4], b[5]
    )
}

/// 从 `7D 02 01` 解析型号名（ASCII 从下标 5 起）。
pub fn parse_model_name(m: &[u8]) -> String {
    ascii_at(m, 5)
}

/// 从 `7D 02 00` 解析有线模式 PID（LE16 在下标 5）。
pub fn parse_pid_at5(m: &[u8]) -> u16 {
    if m.len() < 7 {
        return 0;
    }
    m[5] as u16 | ((m[6] as u16) << 8)
}

/// 从 `7D 20 02` 解析 booster 标志（b4 == 1 表示支持 2000Hz 以上）。
pub fn parse_booster(m: &[u8]) -> bool {
    m.len() > 4 && m[4] == 1
}

/// 从 `FB 02 00` / `FB 02 01` 解析链路 MAC（b6..b11）。
pub fn parse_mac(m: &[u8]) -> String {
    mac_at(m, 6)
}

/// 从 `12 12 02` 解析鼠标 SN（ASCII 从下标 5 起）。
pub fn parse_sn(m: &[u8]) -> String {
    ascii_at(m, 5)
}

/// 从 `A1 00` 解析接收器 SN：**前 12 字符**（第 13 字节是相邻字段，不是 SN 的一部分）。
pub fn parse_dongle_sn(m: &[u8]) -> String {
    ascii_at(m, 4).chars().take(12).collect()
}

/// 从 `A1 01` 解析接收器固件版本。
///
/// ⚠️ 文档写的是 `b6.b5.b4`，但实测原始帧 `01 A1 01 00 00 07 00 07` 按 `b5.b6.b7`
/// 解出来才是文档给的 `7.00.07`；按 `b6.b5.b4` 会解成 `0.07.00`。
/// 这里按实测实现，结果与文档记录的版本号一致。
pub fn parse_dongle_fw(m: &[u8]) -> String {
    if m.len() < 8 {
        return String::new();
    }
    format!("{}.{:02}.{:02}", m[5], m[6], m[7])
}


// ==================== 写 ====================

/// 写 `0x51 0x31` 寄存器文件：`{rid, 0x51, 0x31, REG, 0x00, VALUE..}`
pub fn write_reg(dev: &mut HidDevice, rid: u8, reg: u8, value: &[u8]) -> Option<Vec<u8>> {
    let mut tail = Vec::with_capacity(2 + value.len());
    tail.push(reg);
    tail.push(0x00);
    tail.extend_from_slice(value);
    dev.xfer(rid, 0x51, 0x31, &tail, DEFAULT_TIMEOUT_MS)
}

/// 提交进 flash：`0x50 0x03`。回全零是**正常**，不是 NAK。
pub fn commit(dev: &mut HidDevice, rid: u8) -> Option<Vec<u8>> {
    dev.xfer(rid, 0x50, 0x03, &[], DEFAULT_TIMEOUT_MS)
}

/// 写 DPI 档位（值 + 该档颜色同一帧）：`encLo, encHi, R, G, B`
pub fn write_dpi(
    dev: &mut HidDevice,
    rid: u8,
    stage: usize,
    dpi: i32,
    rgb: [u8; 3],
) -> Option<Vec<u8>> {
    let enc = encode_dpi(dpi);
    write_reg(
        dev,
        rid,
        stage as u8,
        &[(enc & 0xFF) as u8, ((enc >> 8) & 0xFF) as u8, rgb[0], rgb[1], rgb[2]],
    )
}

/// 写能量块 `51 37`。
///
/// **读写偏移不一样**（文档第 5 节事故记录的根因）：
/// ```text
/// 写 tail[0]=00 tail[1]=00 tail[2]=休眠 tail[3]=00 tail[4]=低电量 tail[5]=00 tail[6]=充满效果
/// 读 aligned[6]=休眠 aligned[7]=低电量 aligned[12]=充满效果
/// ```
/// 即 `tail[n]` 对应 `aligned[n+4]`。正确帧样例：`51 37 00 00 04 00 14 00 01`
/// = 10 分钟 / 20% / 充满效果开。
pub fn write_energy(
    dev: &mut HidDevice,
    rid: u8,
    sleep_mode: u8,
    low_warn: u8,
    full_effect: u8,
) -> Option<Vec<u8>> {
    dev.xfer(
        rid,
        0x51,
        0x37,
        &[0x00, 0x00, sleep_mode, 0x00, low_warn, 0x00, full_effect],
        DEFAULT_TIMEOUT_MS,
    )
}

/// 写 LOD（独立命令族）：`{rid, 0x51, 0x35, 0xFF, 0x00, 0xFF, code}`，code 0=Low 1=High。
///
/// ⚠️ 厂商源码承认：这个命令会**把表面标定重置为默认**，没有「只设 LOD」的单独命令。
pub fn write_lod(dev: &mut HidDevice, rid: u8, code: u8) -> Option<Vec<u8>> {
    dev.xfer(rid, 0x51, 0x35, &[0xFF, 0x00, 0xFF, code], DEFAULT_TIMEOUT_MS)
}

/// 写灯光（独立命令族）：
/// `{rid, 0x51, 0x28, zone, 0x00, mode, brightness, R, G, B, dir, random, speed}`
///
/// 本型号 zone 恒 0；dir/random/speed 只对 Rainbow/Comet 有意义，而这两个模式本型号不支持，
/// 因此永远写 0（与 G-Helper 做法一致）。
pub fn write_light(
    dev: &mut HidDevice,
    rid: u8,
    mode: u8,
    brightness: u8,
    rgb: [u8; 3],
    dir: u8,
    random: u8,
    speed: u8,
) -> Option<Vec<u8>> {
    dev.xfer(
        rid,
        0x51,
        0x28,
        &[
            LIGHT_ZONE,
            0x00,
            mode,
            brightness,
            rgb[0],
            rgb[1],
            rgb[2],
            dir,
            random,
            speed,
        ],
        DEFAULT_TIMEOUT_MS,
    )
}

/// 第 `slot` 个按键的固定源码；越界返回 `None`。
pub fn button_src(slot: usize) -> Option<u16> {
    BUTTON_SRC.get(slot).copied()
}

/// 写按键绑定（独立命令族）：
/// `{rid, 0x51, 0x21, 0x00, 0x00, srcLo, srcHi, dstLo, dstHi}`
///
/// `src` 是该物理键的固定源码（不可改），`dst` 是动作码；**`dst = 0x0000` 等于禁用该键**。
pub fn write_button(dev: &mut HidDevice, rid: u8, slot: usize, dst: u16) -> Option<Vec<u8>> {
    // 最后一道防线：`ops` 层已校验过槽位范围，这里直接返回 None 而不是
    // 下标 panic（越界会让整个 GUI 崩掉，代价远大于写不进去）。
    let src = button_src(slot)?;
    dev.xfer(
        rid,
        0x51,
        0x21,
        &[
            0x00,
            0x00,
            (src & 0xFF) as u8,
            ((src >> 8) & 0xFF) as u8,
            (dst & 0xFF) as u8,
            ((dst >> 8) & 0xFF) as u8,
        ],
        DEFAULT_TIMEOUT_MS,
    )
}

// ==================== 打印 ====================

/// 十六进制转储。`raw` 子命令与诊断用。
///
/// 输出两行：偏移行 + 数据行，每行 16 字节，与常见 HID 抓包工具格式一致，
/// 方便直接和文档里的「原始帧」逐字节对照。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_index_is_clamped() {
        let mut st = State::default();
        st.enc = [7, 15, 23, 31];
        st.rgb = [[1, 2, 3], [4, 5, 6], [7, 8, 9], [10, 11, 12]];

        assert_eq!(st.dpi(0), decode_dpi(7));
        assert_eq!(st.dpi(DPI_STAGES - 1), decode_dpi(31));
        // 越界不该 panic，按最后一档处理
        assert_eq!(st.dpi(99), decode_dpi(31));
        assert_eq!(st.rgb_hex(99), "#0a0b0c");
    }

    #[test]
    fn button_src_is_bounded() {
        assert_eq!(button_src(0), Some(BUTTON_SRC[0]));
        assert_eq!(button_src(BUTTON_SLOTS - 1), Some(BUTTON_SRC[BUTTON_SLOTS - 1]));
        assert_eq!(button_src(BUTTON_SLOTS), None);
        assert_eq!(button_src(999), None);
    }

    #[test]
    fn encode_dpi_clamps_without_changing_truncation() {
        // 正常值：截断语义不变（437 -> enc 7 -> 400），且不算被改过
        assert_eq!(encode_dpi(800), 15);
        assert_eq!(decode_dpi(encode_dpi(437)), 400);
        assert_eq!(encode_dpi_checked(800), (15, false));

        // 低于下限：不能落成 0（解码 50，低于设备下限），夹到 DPI_ENC_MIN
        let (low, changed) = encode_dpi_checked(10);
        assert_eq!(low, DPI_ENC_MIN);
        assert!(changed);
        assert_eq!(decode_dpi(low), 100);

        // 高于上限：夹到 DPI_ENC_MAX（解码 42000）
        let (high, changed) = encode_dpi_checked(50_000);
        assert_eq!(high, DPI_ENC_MAX);
        assert!(changed);
        assert_eq!(decode_dpi(high), 42_000);
    }
}
