//! 设备操作层 —— CLI 与 GUI 共用的唯一一份写实现。
//!
//! # 为什么要有这一层
//!
//! 六步安全循环（读原值 → 写新值 → 读回 → 自动写回原值 → 再读回断言 → 只有显式
//! `save` 才 commit）必须**只有一份实现**。两套必然漂移，而漂移的那次就是用户设备
//! 被改坏的那次。所以 GUI 和 CLI 都不能自己写循环，只能调这里。
//!
//! # 约定
//!
//! * **本模块不打印任何东西**。日志通过 [`Logger`] 回调交给调用方，
//!   由它决定打到 stdout、日志文件还是 GUI 的文本框。
//! * 错误用 [`OpsError`] 分类，调用方能区分「设备休眠」（等一下或动一下鼠标）
//!   和「参数非法」（让用户改输入）。这两种在 GUI 上要给完全不同的提示。
//! * 所有参数校验都在**碰设备之前**做完，参数错了不该付 27ms 的帧代价。
//! * 每一步的「新旧值」都从**同一次读**里取：`cycle` 只读一次原值，
//!   然后把这份 [`State`] 交给构建闭包去生成「写新值」和「写回原值」两个动作。
//!   这样既不会读两遍，也不会出现「写回的值来自另一次读」的错位。

use crate::hid::is_nak;
use crate::hid::HidDevice;
use crate::proto::{self, State, BUTTON_SLOTS, DPI_STAGES, DPI_STEP};

/// 改完到读回之间的等待。实测 80ms 足够让设备把值落到 RAM 里的寄存器文件。
pub const SETTLE_MS: u64 = 80;

// ==================== 错误 ====================

/// 错误分类。GUI 靠这个决定怎么提示用户。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// 参数非法 —— 还没碰设备就知道不合法。GUI 该显示红字，让用户改输入。
    Invalid,
    /// 设备休眠 / 无应答 —— 鼠标 10 分钟没动就会这样。GUI 该灰掉控件并提示
    /// 「鼠标休眠中」，而不是报错。**不要重试**：一次 MISS 要等满超时（约 2.4 秒），
    /// 而正常往返只要 27ms。
    Asleep,
    /// 设备明确拒绝（NAK `03 FF AA`）。
    Rejected,
    /// 写进去了，但读回与期望不一致。
    Mismatch,
    /// 其它（IO、解析、文件等）。
    Other,
}

/// 操作层错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpsError {
    pub kind: ErrorKind,
    pub message: String,
}

impl OpsError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        OpsError {
            kind,
            message: message.into(),
        }
    }

    /// 参数非法（不是设备的问题）。
    pub fn invalid(msg: impl Into<String>) -> Self {
        OpsError::new(ErrorKind::Invalid, msg)
    }

    /// 设备休眠或无应答。
    pub fn asleep(msg: impl Into<String>) -> Self {
        OpsError::new(ErrorKind::Asleep, msg)
    }

    /// 设备 NAK。
    pub fn rejected(msg: impl Into<String>) -> Self {
        OpsError::new(ErrorKind::Rejected, msg)
    }

    /// 读回对不上。
    pub fn mismatch(msg: impl Into<String>) -> Self {
        OpsError::new(ErrorKind::Mismatch, msg)
    }

    /// 其它。
    pub fn other(msg: impl Into<String>) -> Self {
        OpsError::new(ErrorKind::Other, msg)
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// 是不是「鼠标睡着了」。GUI 可以直接拿这个判断要不要把控件灰掉。
    pub fn is_asleep(&self) -> bool {
        self.kind == ErrorKind::Asleep
    }

    /// 是不是用户输入的问题（不是设备的问题）。
    pub fn is_invalid(&self) -> bool {
        self.kind == ErrorKind::Invalid
    }
}

impl std::fmt::Display for OpsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 休眠这类「不是错误的错误」前面加统一后缀，日志里一眼能认出来
        match self.kind {
            ErrorKind::Asleep => write!(f, "{}（鼠标休眠中？先移动一下鼠标）", self.message),
            _ => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for OpsError {}

impl From<String> for OpsError {
    fn from(s: String) -> Self {
        OpsError::other(s)
    }
}

impl From<&str> for OpsError {
    fn from(s: &str) -> Self {
        OpsError::other(s)
    }
}

// ==================== 日志 ====================

/// 日志回调。`ops` 自己永不打印，全部经由它。
///
/// CLI 传 `&mut |line| println!("{line}")`；
/// GUI 传一个往通道里塞消息的闭包，由 GUI 线程取出来追加到文本框。
pub type Logger<'a> = &'a mut dyn FnMut(&str);

// ==================== 状态读取 ====================

/// `cycle` 读取设备状态的来源。
///
/// `fresh = false` 允许吃缓存（「读原值」这一步用）；
/// `fresh = true` 必须真的去读设备（「读回断言」用，因为 RAM 已经变了）。
///
/// `dev` 作为参数传入而不是存在实现里，是为了避开借用冲突：
/// `cycle` 同时需要 `&mut HidDevice` 和 `&mut reader`。
pub trait StateReader {
    fn read(&mut self, dev: &mut HidDevice, rid: u8, fresh: bool) -> Result<State, OpsError>;
}

/// 总是直读设备、不吃缓存的实现。GUI 的后台线程和 `restore` 用这个。
///
/// CLI 用 `Ctx`（它实现了 [`StateReader`] 并带缓存），所以这个类型在
/// 纯 CLI 构建里只出现在测试中 —— GUI 接进来后就是主路径。
#[allow(dead_code)]
pub struct DirectReader;

impl StateReader for DirectReader {
    fn read(&mut self, dev: &mut HidDevice, rid: u8, _fresh: bool) -> Result<State, OpsError> {
        proto::read_state(dev, rid).map_err(map_read_error)
    }
}

/// 把 `proto::read_state` 的字符串错误归类成 [`OpsError`]。
///
/// `read_state` 在设备无应答时会连续 MISS 若干帧，报错文本里含「无应答」。
pub fn map_read_error(msg: String) -> OpsError {
    if msg.contains("无应答") || msg.contains("休眠") {
        OpsError::asleep(msg)
    } else {
        OpsError::other(msg)
    }
}

// ==================== 六步循环 ====================

/// 一次 setter 的完整报告。GUI 直接拿去渲染，不用再解析字符串。
#[derive(Debug, Clone, Default)]
pub struct ApplyReport {
    /// 新值是否真的被设备接受（读回与期望一致）
    pub applied: bool,
    /// 原值是否被成功写回并读回确认。`keep = true` 时恒为 `false`。
    pub restored: bool,
    /// 还原流程是否跑过。`keep = true` 时为 `false`（没尝试还原，
    /// 不是因为还原失败 —— 用这个字段区分两种情况）
    pub revert_attempted: bool,

    /// 写之前的读数（人类可读，如 `1000Hz (code 3)`）
    pub old_value: String,
    /// 期望写进去的值（人类可读）
    pub new_value: String,
    /// 写完读回来的实际值 —— `applied = false` 时看这个知道设备到底接受了什么
    pub readback_value: String,
    /// 还原后读回来的值 —— `restored = false` 时看这个知道现在是啥状态
    pub restored_value: String,

    /// 需要注意的事（DPI 被截断、LOD 会重置标定…）。GUI 用弹窗或黄字显示。
    pub warning: Option<String>,
}

impl ApplyReport {
    /// 一句话结论，给日志尾巴和 GUI 状态栏用。
    ///
    /// [`crate::cli::print_report`] 里的「已还原」文案也走这里，
    /// 免得终端和界面两套说法各写一遍、早晚漂移。
    pub fn summary(&self) -> String {
        format!(
            "写入生效={}  已还原={}",
            if self.applied { "是" } else { "否" },
            if !self.revert_attempted {
                "没有（--keep）"
            } else if self.restored {
                "是"
            } else {
                "**否**"
            }
        )
    }

    /// 把「读回不对」升级成错误。
    ///
    /// [`cycle`] 本身把「写了但读回不一致」当作**结果**而不是错误返回，
    /// 因为 GUI 需要拿到报告去渲染。命令行/自动化则通常希望非零退出码，
    /// 所以由调用方决定要不要调这个方法。
    ///
    /// 注意 `--keep` 时 `restored = false` 是**预期行为**，不会报错
    /// —— 靠 `revert_attempted` 区分「没尝试还原」和「还原失败」。
    pub fn into_result(self) -> Result<ApplyReport, OpsError> {
        if !self.applied {
            return Err(OpsError::mismatch(format!(
                "写入未生效：期望 {}，读回 {}",
                self.new_value, self.readback_value
            )));
        }
        if self.revert_attempted && !self.restored {
            return Err(OpsError::mismatch(format!(
                "还原失败：原值 {}，现在 {}。请用 backup 文件 restore",
                self.old_value, self.restored_value
            )));
        }
        Ok(self)
    }
}

/// 写动作：给设备句柄和 report id，返回应答帧（`None` = 无应答）。
pub type WriteFn = Box<dyn Fn(&mut HidDevice, u8) -> Option<Vec<u8>>>;

/// 一次 setter 的静态参数（与设备当前状态无关的那部分）。
pub struct CycleSpec {
    /// 标题，日志第一行用
    pub title: String,
    /// 保留新值不还原
    pub keep: bool,
    /// 写入后 commit 进 flash
    pub save: bool,
    /// 调用方预先知道要注意的事（会原样进 [`ApplyReport::warning`]）
    pub warning: Option<String>,
}

impl CycleSpec {
    pub fn new(title: impl Into<String>) -> Self {
        CycleSpec {
            title: title.into(),
            keep: false,
            save: false,
            warning: None,
        }
    }

    pub fn keep(mut self, v: bool) -> Self {
        self.keep = v;
        self
    }

    pub fn save(mut self, v: bool) -> Self {
        self.save = v;
        self
    }

    pub fn warning(mut self, w: Option<String>) -> Self {
        self.warning = w;
        self
    }
}

/// 六步安全循环，对应 `rw-hid.cs` 的 `Cycle()`。
///
/// 1. 读原值（缓存优先，**整个流程只读这一次**）
/// 2. 写新值（`save` 时顺带 commit）
/// 3. 读回并断言 `applied`
/// 4. 自动写回原值（`keep` 时跳过）
/// 5. 再读回并断言 `restored`
/// 6. 只有 `save` 才真正落 flash —— 默认跑完设备状态与跑之前一模一样
///
/// `expect` 和 `build` 都拿着**同一次**读到的原值 [`State`]：
/// `expect` 算出期望的读回字符串，`build` 造出「写新值」和「写回原值」两个动作。
///
/// 返回 `Ok(report)` 表示流程走完了，**`applied` / `restored` 要看字段**：
/// 读了但不一致不是致命错误，是个结果。只有无应答、NAK、读失败才返回 `Err`。
#[allow(clippy::too_many_arguments)]
pub fn cycle<P, E, B>(
    dev: &mut HidDevice,
    rid: u8,
    spec: &CycleSpec,
    probe: P,
    expect: E,
    build: B,
    reader: &mut dyn StateReader,
    log: Logger,
) -> Result<ApplyReport, OpsError>
where
    P: Fn(&State) -> String,
    E: FnOnce(&State) -> String,
    B: FnOnce(&State) -> (WriteFn, WriteFn),
{
    log(&format!("== {} ==", spec.title));

    // ---- 步骤 1：读原值（唯一的一次）----
    let before = reader.read(dev, rid, false)?;
    let old_value = probe(&before);
    log(&format!("BEFORE       {old_value}"));

    let expected = expect(&before);
    let (apply, revert) = build(&before);

    if expected == old_value {
        log("提示：目标值与当前值相同，本次写入实际是空操作。");
    }

    // ---- 步骤 2：写新值 ----
    let reply = apply(dev, rid).ok_or_else(|| OpsError::asleep("写入无应答"))?;
    if is_nak(&reply) {
        return Err(OpsError::rejected(format!(
            "设备拒绝写入（NAK 03 FF AA）：{}",
            spec.title
        )));
    }
    if spec.save {
        commit_or_fail(dev, rid)?;
    }
    sleep_ms(SETTLE_MS);

    // ---- 步骤 3：读回断言 ----
    let after = reader.read(dev, rid, true)?;
    let readback_value = probe(&after);
    log(&format!("AFTER WRITE  {readback_value}"));
    let applied = readback_value == expected;
    log(&format!(
        "VERIFY       applied={}",
        if applied { "YES" } else { "NO" }
    ));
    if !applied {
        log(&format!("             期望 {expected}"));
    }

    let mut report = ApplyReport {
        applied,
        restored: false,
        revert_attempted: false,
        old_value,
        new_value: expected,
        readback_value,
        restored_value: String::new(),
        warning: spec.warning.clone(),
    };

    if spec.keep {
        log("（--keep：保留新值不还原。未 commit 的值重连后会丢失）");
        return Ok(report);
    }

    // ---- 步骤 4：自动写回原值 ----
    log("RESTORE      写回原值…");
    let reply = revert(dev, rid).ok_or_else(|| OpsError::asleep("写回原值时无应答"))?;
    if is_nak(&reply) {
        return Err(OpsError::rejected("设备拒绝写回原值（NAK）"));
    }
    if spec.save {
        commit_or_fail(dev, rid)?;
    }
    sleep_ms(SETTLE_MS);

    // ---- 步骤 5：再读回断言 ----
    let back = reader.read(dev, rid, true)?;
    let restored_value = probe(&back);
    log(&format!("AFTER RESTORE {restored_value}"));
    report.restored = restored_value == report.old_value;
    report.restored_value = restored_value;
    report.revert_attempted = true;
    log(&format!(
        "VERIFY       restored={}",
        if report.restored { "YES" } else { "NO" }
    ));
    if !report.restored {
        log(&format!("             期望回到 {}", report.old_value));
        log("             还原失败：请用 backup 文件 restore");
    }

    Ok(report)
}

/// commit，并把「无应答」归类成 `Asleep`、NAK 归类成 `Rejected`。
///
/// 提交命令回**全零是正常的**（不是 NAK），不要误判。
fn commit_or_fail(dev: &mut HidDevice, rid: u8) -> Result<(), OpsError> {
    match proto::commit(dev, rid) {
        Some(m) if is_nak(&m) => Err(OpsError::rejected("commit 被设备 NAK")),
        Some(_) => Ok(()),
        None => Err(OpsError::asleep("commit 无应答")),
    }
}

pub(crate) fn sleep_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

// ==================== setter：前四条（用户最常改的） ====================

/// `setpoll` —— 回报率。寄存器 `0x04`。
pub fn set_poll(
    dev: &mut HidDevice,
    rid: u8,
    hz: i32,
    keep: bool,
    save: bool,
    reader: &mut dyn StateReader,
    log: Logger,
) -> Result<ApplyReport, OpsError> {
    let code = proto::poll_code_of(hz)
        .ok_or_else(|| OpsError::invalid(format!("回报率只支持 125/250/500/1000，收到 {hz}")))?;

    let spec = CycleSpec::new(format!("setpoll -> {hz}Hz (code {code})"))
        .keep(keep)
        .save(save);

    cycle(
        dev,
        rid,
        &spec,
        |st: &State| {
            format!(
                "{} (code {})",
                proto::poll_name(st.poll_code),
                st.poll_code
            )
        },
        |_| format!("{} (code {code})", proto::poll_name(code)),
        move |before: &State| {
            let orig = before.poll_code;
            (
                Box::new(move |d: &mut HidDevice, r: u8| proto::write_reg(d, r, 0x04, &[code])),
                Box::new(move |d: &mut HidDevice, r: u8| proto::write_reg(d, r, 0x04, &[orig])),
            )
        },
        reader,
        log,
    )
}

/// `setdpi` —— 某档 DPI 与该档颜色（同一帧）。寄存器 `0x00 + (stage-1)`。
///
/// `rgb = None` 时沿用当前该档的颜色。
#[allow(clippy::too_many_arguments)]
pub fn set_dpi(
    dev: &mut HidDevice,
    rid: u8,
    stage: usize,
    dpi: i32,
    rgb: Option<[u8; 3]>,
    keep: bool,
    save: bool,
    reader: &mut dyn StateReader,
    log: Logger,
) -> Result<ApplyReport, OpsError> {
    if !(1..=DPI_STAGES).contains(&stage) {
        return Err(OpsError::invalid(format!(
            "DPI 档位必须是 1-{DPI_STAGES}，收到 {stage}"
        )));
    }
    if !(proto::DPI_MIN..=proto::DPI_MAX).contains(&dpi) {
        return Err(OpsError::invalid(format!(
            "DPI 范围 {}-{}，收到 {dpi}",
            proto::DPI_MIN,
            proto::DPI_MAX
        )));
    }
    let idx = stage - 1;

    // 50 步进提醒：设备会静默截断，提前告诉用户
    let enc = proto::encode_dpi(dpi);
    let applied_dpi = proto::decode_dpi(enc);
    let warning = if applied_dpi != dpi {
        Some(format!(
            "DPI 步进为 {DPI_STEP}，{dpi} 会被设备截断成 {applied_dpi}（enc {enc}）。\
             读回比较针对 {applied_dpi}，不是你输入的 {dpi}。"
        ))
    } else {
        None
    };

    let spec = CycleSpec::new(format!("setdpi 档 {stage} -> {applied_dpi}"))
        .keep(keep)
        .save(save)
        .warning(warning);

    cycle(
        dev,
        rid,
        &spec,
        move |st: &State| {
            format!(
                "{} #{:02x}{:02x}{:02x}",
                st.dpi(idx),
                st.rgb[idx][0],
                st.rgb[idx][1],
                st.rgb[idx][2]
            )
        },
        move |before: &State| {
            // 没指定颜色就沿用当前该档的颜色（同一帧里 DPI 和颜色一起写）
            let c = rgb.unwrap_or(before.rgb[idx]);
            format!("{applied_dpi} #{:02x}{:02x}{:02x}", c[0], c[1], c[2])
        },
        move |before: &State| {
            let color = rgb.unwrap_or(before.rgb[idx]);
            let orig_dpi = before.dpi(idx);
            let orig_color = before.rgb[idx];
            (
                Box::new(move |d: &mut HidDevice, r: u8| {
                    proto::write_dpi(d, r, idx, applied_dpi, color)
                }),
                Box::new(move |d: &mut HidDevice, r: u8| {
                    proto::write_dpi(d, r, idx, orig_dpi, orig_color)
                }),
            )
        },
        reader,
        log,
    )
}

/// `setstage` —— 当前 DPI 档位。寄存器 `0x09`，**1-based**。
pub fn set_stage(
    dev: &mut HidDevice,
    rid: u8,
    stage: u8,
    keep: bool,
    save: bool,
    reader: &mut dyn StateReader,
    log: Logger,
) -> Result<ApplyReport, OpsError> {
    if !(1..=4).contains(&stage) {
        return Err(OpsError::invalid(format!("档位必须是 1-4，收到 {stage}")));
    }

    let spec = CycleSpec::new(format!("setstage -> {stage}"))
        .keep(keep)
        .save(save);

    cycle(
        dev,
        rid,
        &spec,
        |st: &State| st.active_stage.to_string(),
        |_| stage.to_string(),
        move |before: &State| {
            let orig = before.active_stage;
            (
                Box::new(move |d: &mut HidDevice, r: u8| proto::write_reg(d, r, 0x09, &[stage])),
                Box::new(move |d: &mut HidDevice, r: u8| proto::write_reg(d, r, 0x09, &[orig])),
            )
        },
        reader,
        log,
    )
}

/// `setdebounce` —— 去抖。寄存器 `0x05`，code 0-7。
pub fn set_debounce(
    dev: &mut HidDevice,
    rid: u8,
    code: u8,
    keep: bool,
    save: bool,
    reader: &mut dyn StateReader,
    log: Logger,
) -> Result<ApplyReport, OpsError> {
    if proto::debounce_ms(code).is_none() {
        return Err(OpsError::invalid(format!(
            "去抖代码必须是 0-7，收到 {code}"
        )));
    }

    let spec = CycleSpec::new(format!(
        "setdebounce -> {} (code {code})",
        proto::debounce_name(code)
    ))
    .keep(keep)
    .save(save);

    cycle(
        dev,
        rid,
        &spec,
        |st: &State| st.debounce_code.to_string(),
        |_| code.to_string(),
        move |before: &State| {
            let orig = before.debounce_code;
            (
                Box::new(move |d: &mut HidDevice, r: u8| proto::write_reg(d, r, 0x05, &[code])),
                Box::new(move |d: &mut HidDevice, r: u8| proto::write_reg(d, r, 0x05, &[orig])),
            )
        },
        reader,
        log,
    )
}

// ==================== setter：其余五条 ====================

/// `setsnap` —— 角度吸附。寄存器 `0x06`。
pub fn set_snap(
    dev: &mut HidDevice,
    rid: u8,
    snap: u8,
    keep: bool,
    save: bool,
    reader: &mut dyn StateReader,
    log: Logger,
) -> Result<ApplyReport, OpsError> {
    if snap > 1 {
        return Err(OpsError::invalid(format!(
            "角度吸附只接受 0 或 1，收到 {snap}"
        )));
    }

    let spec = CycleSpec::new(format!("setsnap -> {snap}"))
        .keep(keep)
        .save(save);

    cycle(
        dev,
        rid,
        &spec,
        |st: &State| st.angle_snap.to_string(),
        |_| snap.to_string(),
        move |before: &State| {
            let orig = before.angle_snap;
            (
                Box::new(move |d: &mut HidDevice, r: u8| proto::write_reg(d, r, 0x06, &[snap])),
                Box::new(move |d: &mut HidDevice, r: u8| proto::write_reg(d, r, 0x06, &[orig])),
            )
        },
        reader,
        log,
    )
}

/// `settuning` —— 角度微调。寄存器 `0x0B`，signed int16 LE，步进 5。
pub fn set_tuning(
    dev: &mut HidDevice,
    rid: u8,
    value: i16,
    keep: bool,
    save: bool,
    reader: &mut dyn StateReader,
    log: Logger,
) -> Result<ApplyReport, OpsError> {
    if !(-20..=20).contains(&value) {
        return Err(OpsError::invalid(format!(
            "角度微调范围 -20..20，收到 {value}"
        )));
    }
    if value % 5 != 0 {
        return Err(OpsError::invalid(format!(
            "角度微调步进为 5，{value} 不是 5 的整数倍"
        )));
    }

    let spec = CycleSpec::new(format!("settuning -> {value}"))
        .keep(keep)
        .save(save);

    let bytes = |v: i16| vec![(v & 0xFF) as u8, ((v >> 8) & 0xFF) as u8];

    cycle(
        dev,
        rid,
        &spec,
        |st: &State| st.angle_tuning.to_string(),
        |_| value.to_string(),
        move |before: &State| {
            let orig = before.angle_tuning;
            (
                Box::new(move |d: &mut HidDevice, r: u8| {
                    let b = bytes(value);
                    proto::write_reg(d, r, 0x0B, &b)
                }),
                Box::new(move |d: &mut HidDevice, r: u8| {
                    let b = bytes(orig);
                    proto::write_reg(d, r, 0x0B, &b)
                }),
            )
        },
        reader,
        log,
    )
}

/// `setlod` —— 抬起距离。独立命令族 `0x51 0x35`。
///
/// **写它会把表面标定重置为默认值**，厂商源码承认没有只设 LOD 的命令
/// （`_ghelper\AsusMouse.cs:1626`）。所以这里强制带 `warning`，
/// 并且写前写后各读一次标定字节，把变化打进日志。
pub fn set_lod(
    dev: &mut HidDevice,
    rid: u8,
    code: u8,
    keep: bool,
    save: bool,
    reader: &mut dyn StateReader,
    log: Logger,
) -> Result<ApplyReport, OpsError> {
    if code > 1 {
        return Err(OpsError::invalid(format!(
            "LOD 只接受 0(Low) 或 1(High)，收到 {code}"
        )));
    }

    // 写之前的标定值，事后对比给用户看
    let before_lod = proto::read_lod_only(dev, rid).map_err(map_read_error)?;
    log("⚠️  警告：写 LOD 会把**表面标定**重置为默认值（厂商源码承认，没有只设 LOD 的命令）。");
    log(&format!(
        "   写之前的标定：k = {:02X} {:02X} {:02X}",
        before_lod.calib[0], before_lod.calib[1], before_lod.calib[2]
    ));

    let spec = CycleSpec::new(format!(
        "setlod -> {} (code {code})",
        proto::lod_name(code)
    ))
    .keep(keep)
    .save(save)
    .warning(Some(
        "写 LOD 会重置表面标定（厂商确认无恢复命令）。".to_string(),
    ));

    let result = cycle(
        dev,
        rid,
        &spec,
        |st: &State| st.lod.to_string(),
        |_| code.to_string(),
        move |before: &State| {
            let orig = before.lod;
            (
                Box::new(move |d: &mut HidDevice, r: u8| proto::write_lod(d, r, code)),
                Box::new(move |d: &mut HidDevice, r: u8| proto::write_lod(d, r, orig)),
            )
        },
        reader,
        log,
    );

    if let Ok(after_lod) = proto::read_lod_only(dev, rid) {
        log(&format!(
            "   写之后的标定：k = {:02X} {:02X} {:02X}{}",
            after_lod.calib[0],
            after_lod.calib[1],
            after_lod.calib[2],
            if after_lod.calib == before_lod.calib {
                "（未变）"
            } else {
                "（已被重置，无法恢复）"
            }
        ));
    }
    result
}

/// `setlight` —— 灯光。独立命令族 `0x51 0x28`，本型号 zone = 0。
///
/// mode 3（Rainbow）与 5（Comet）本型号**不支持**：设备会静默回落到 ColorCycle。
/// 这里在碰设备之前就拒绝，避免用户以为设置成功了。
///
/// `dir` / `random` / `speed` 是 `Option`：**不传就沿用设备当前值**，不是写 0。
/// 早先这里恒写 0，有三个后果：① 界面上这三个控件改了等于没改；
/// ② 设备如果本来就存着非 0 值（GearLink 或别的工具设过），我们的写入会
/// 把它悄悄清零；③ `expect` 里写死的 `0 0 0` 与 `probe` 读到的真实值永远
/// 不相等 → `applied=NO`，明明写成功却报失败。
#[allow(clippy::too_many_arguments)]
pub fn set_light(
    dev: &mut HidDevice,
    rid: u8,
    mode: u8,
    bright: Option<u8>,
    rgb: Option<[u8; 3]>,
    dir: Option<u8>,
    random: Option<u8>,
    speed: Option<u8>,
    keep: bool,
    save: bool,
    reader: &mut dyn StateReader,
    log: Logger,
) -> Result<ApplyReport, OpsError> {
    if !proto::light_mode_supported(mode) {
        return Err(OpsError::invalid(format!(
            "灯光模式 {mode}（{}）本型号不支持：设备会静默回落到 ColorCycle。\n\
             可用模式：0=Static 1=Breathing 2=ColorCycle 4=React 6=BatteryState 240=Off",
            proto::light_mode_name(mode)
        )));
    }
    if let Some(b) = bright {
        if b > 100 {
            return Err(OpsError::invalid(format!("亮度范围 0-100，收到 {b}")));
        }
    }
    if let Some(s) = speed {
        if s > 100 {
            return Err(OpsError::invalid(format!("速度范围 0-100，收到 {s}")));
        }
    }
    if let Some(d) = dir {
        if d > 1 {
            return Err(OpsError::invalid(format!(
                "灯光方向只有 0=正向 1=反向，收到 {d}"
            )));
        }
    }
    if let Some(r) = random {
        if r > 1 {
            return Err(OpsError::invalid(format!(
                "随机颜色只有 0=关 1=开，收到 {r}"
            )));
        }
    }

    let spec = CycleSpec::new(format!(
        "setlight -> {} (code {mode})",
        proto::light_mode_name(mode)
    ))
    .keep(keep)
    .save(save);

    cycle(
        dev,
        rid,
        &spec,
        |st: &State| {
            format!(
                "{} #{} {} {} {} {}",
                st.light_mode,
                format!(
                    "{:02x}{:02x}{:02x}",
                    st.light_rgb[0], st.light_rgb[1], st.light_rgb[2]
                ),
                st.light_bright,
                st.light_dir,
                st.light_random,
                st.light_speed
            )
        },
        // 期望值必须用和 `probe` **完全相同**的格式，并且把"没传就沿用原值"
        // 那三个字段按原值填进去。早先这里写死 `0 0 0`，设备上只要有非 0
        // 值，写成功了也会被判成 applied=NO。
        move |before: &State| {
            let b = bright.unwrap_or(before.light_bright);
            let c = rgb.unwrap_or(before.light_rgb);
            let d = dir.unwrap_or(before.light_dir);
            let r = random.unwrap_or(before.light_random);
            let s = speed.unwrap_or(before.light_speed);
            format!("{mode} #{:02x}{:02x}{:02x} {b} {d} {r} {s}", c[0], c[1], c[2])
        },
        move |before: &State| {
            let b = bright.unwrap_or(before.light_bright);
            let c = rgb.unwrap_or(before.light_rgb);
            let d = dir.unwrap_or(before.light_dir);
            let r = random.unwrap_or(before.light_random);
            let s = speed.unwrap_or(before.light_speed);
            let orig = (
                before.light_mode,
                before.light_bright,
                before.light_rgb,
                before.light_dir,
                before.light_random,
                before.light_speed,
            );
            (
                // 形参不能叫 `d` —— 上面 `let d = dir.unwrap_or(...)` 是方向值，
                // 同名会被闭包形参遮蔽，`write_light(dev, rid, mode, b, c, d, ...)`
                // 就会把 `&mut HidDevice` 当成方向传进去（编译期报 E0308 是好事，
                // 若两者类型恰好相同就会静默写错值）。
                Box::new(move |dev: &mut HidDevice, rid: u8| {
                    proto::write_light(dev, rid, mode, b, c, d, r, s)
                }),
                Box::new(move |dev: &mut HidDevice, rid: u8| {
                    proto::write_light(dev, rid, orig.0, orig.1, orig.2, orig.3, orig.4, orig.5)
                }),
            )
        },
        reader,
        log,
    )
}

/// `setsleep` —— 休眠（自动关机）时间。命令族 `0x51 0x37`。
///
/// `b6` 是**枚举值不是分钟数**：0=1分 1=2分 2=3分 3=5分 4=10分 0xFF=永不
/// （`_ghelper\AsusMouse.cs:9-17` 的 `PowerOffSetting`）。
///
/// 真机实测：设备**不做范围校验**，写 5..254 这些无定义值也会原样存下来
/// （见 README 未解之谜）。所以有效值必须由我们这边把关 —— 让一个无定义值
/// 落进设备，后果不可预知且没有恢复命令。
#[allow(clippy::too_many_arguments)]
pub fn set_sleep(
    dev: &mut HidDevice,
    rid: u8,
    code: u8,
    keep: bool,
    save: bool,
    reader: &mut dyn StateReader,
    log: Logger,
) -> Result<ApplyReport, OpsError> {
    if !proto::sleep_code_valid(code) {
        return Err(OpsError::invalid(format!(
            "休眠档位只接受 0=1分钟 1=2分钟 2=3分钟 3=5分钟 4=10分钟 255=永不，收到 {code}"
        )));
    }

    let mut warning = None;
    if code == proto::SLEEP_NEVER {
        // 实测数据：活跃 8.76 %/h vs 静置 0.148 %/h，差 59 倍。
        // 不写清楚的话，用户选了「永不」以为无所谓，第二天发现没电。
        warning = Some(format!(
            "「永不」休眠会显著加快耗电：鼠标闲置时不会自动关机，\
             实测活跃耗电 8.76 %/h 而静置仅 0.148 %/h（差 59 倍）。\
             底下的灯也会一直亮着。{}",
            ""
        ));
    }

    let spec = CycleSpec::new(format!(
        "setsleep -> {} (code {code})",
        proto::sleep_mode_name(code)
    ))
    .keep(keep)
    .save(save);
    let spec = match warning {
        Some(w) => spec.warning(Some(w)),
        None => spec,
    };

    cycle(
        dev,
        rid,
        &spec,
        |st: &State| {
            format!(
                "{} (code {})",
                proto::sleep_mode_name(st.sleep_mode),
                st.sleep_mode
            )
        },
        // 期望值必须和 probe 用**完全相同**的格式，否则断言永远不相等
        |_| format!("{} (code {code})", proto::sleep_mode_name(code)),
        move |before: &State| {
            // 能量块是一整帧写的：只改休眠档，其余两个字段必须用原值补上，
            // 否则会被这一帧覆盖成 0（正是文档第 5 节事故的成因）。
            let orig = (before.sleep_mode, before.low_warn, before.full_effect);
            (
                Box::new(move |d: &mut HidDevice, r: u8| {
                    proto::write_energy(d, r, code, orig.1, orig.2)
                }),
                Box::new(move |d: &mut HidDevice, r: u8| {
                    proto::write_energy(d, r, orig.0, orig.1, orig.2)
                }),
            )
        },
        reader,
        log,
    )
}

/// `setlowbatt` —— 低电量阈值（百分比）。命令族 `0x51 0x37`，`b7`。
///
/// 真机实测：设备接受 0..255 的任意字节，**不校验范围**（写 255 也原样存下）。
/// 所以这里按语义钳到 0..=100，否则会出现「电量 84%，低电量阈值 255%」
/// 这种永远处于低电量告警的状态。
#[allow(clippy::too_many_arguments)]
pub fn set_low_batt(
    dev: &mut HidDevice,
    rid: u8,
    percent: u8,
    keep: bool,
    save: bool,
    reader: &mut dyn StateReader,
    log: Logger,
) -> Result<ApplyReport, OpsError> {
    if percent > 100 {
        return Err(OpsError::invalid(format!(
            "低电量阈值范围 0-100（百分比），收到 {percent}"
        )));
    }
    if percent == 0 {
        log("提示：阈值 0 等于关掉低电量告警 —— 电量耗尽前不会有任何提醒。");
    }

    let spec = CycleSpec::new(format!("setlowbatt -> {percent}%"))
        .keep(keep)
        .save(save);

    cycle(
        dev,
        rid,
        &spec,
        |st: &State| format!("{}%", st.low_warn),
        // 期望值和 probe 格式必须一致
        |_| format!("{percent}%"),
        move |before: &State| {
            let orig = (before.sleep_mode, before.low_warn, before.full_effect);
            (
                Box::new(move |d: &mut HidDevice, r: u8| {
                    proto::write_energy(d, r, orig.0, percent, orig.2)
                }),
                Box::new(move |d: &mut HidDevice, r: u8| {
                    proto::write_energy(d, r, orig.0, orig.1, orig.2)
                }),
            )
        },
        reader,
        log,
    )
}

/// `setfulleffect` —— 充满电时的灯效开关。命令族 `0x51 0x37`，`b12`。
///
/// 真机实测：设备只认 0 和 1，**写 ≥2 一律变成 1**（不是拒绝，是归一到「开」）。
/// 所以这里只放行 0/1，避免用户以为设成了「2 档」。
#[allow(clippy::too_many_arguments)]
pub fn set_full_effect(
    dev: &mut HidDevice,
    rid: u8,
    on: u8,
    keep: bool,
    save: bool,
    reader: &mut dyn StateReader,
    log: Logger,
) -> Result<ApplyReport, OpsError> {
    if on > 1 {
        return Err(OpsError::invalid(format!(
            "充满效果只接受 0（关）或 1（开），收到 {on}。\
             设备会把任何 ≥2 的值归一成 1，所以这里直接拒绝而不是静默改写。"
        )));
    }

    let spec = CycleSpec::new(format!(
        "setfulleffect -> {} (code {on})",
        if on == 1 { "开" } else { "关" }
    ))
    .keep(keep)
    .save(save);

    cycle(
        dev,
        rid,
        &spec,
        |st: &State| format!("{}", st.full_effect),
        |_| format!("{on}"),
        move |before: &State| {
            let orig = (before.sleep_mode, before.low_warn, before.full_effect);
            (
                Box::new(move |d: &mut HidDevice, r: u8| {
                    proto::write_energy(d, r, orig.0, orig.1, on)
                }),
                Box::new(move |d: &mut HidDevice, r: u8| {
                    proto::write_energy(d, r, orig.0, orig.1, orig.2)
                }),
            )
        },
        reader,
        log,
    )
}

/// `setbutton` —— 按键绑定。独立命令族 `0x51 0x21`。
///
/// `dst = 0x0000` 等于**禁用该键**，只能靠备份还原。
/// `confirmed = false` 时返回 [`ErrorKind::Invalid`]，GUI 该弹二次确认框。
#[allow(clippy::too_many_arguments)]
pub fn set_button(
    dev: &mut HidDevice,
    rid: u8,
    slot: usize,
    dst: u16,
    keep: bool,
    save: bool,
    confirmed: bool,
    reader: &mut dyn StateReader,
    log: Logger,
) -> Result<ApplyReport, OpsError> {
    if slot >= BUTTON_SLOTS {
        return Err(OpsError::invalid(format!(
            "按键槽位必须是 0-{}，收到 {slot}",
            BUTTON_SLOTS - 1
        )));
    }
    if dst == 0x0000 && !confirmed {
        return Err(OpsError::invalid(format!(
            "dst = 0x0000 表示**禁用**「{}」这个键，禁用后它完全没反应，\
             恢复只能靠备份文件。确认要禁用请二次确认。",
            proto::BUTTON_NAME[slot]
        )));
    }

    let alt = if dst == 0 { "（禁用）" } else { "" };
    let spec = CycleSpec::new(format!(
        "setbutton 槽 {slot} ({}) -> 0x{dst:04X}{alt}",
        proto::BUTTON_NAME[slot]
    ))
    .keep(keep)
    .save(save)
    .warning(if dst == 0 {
        Some(format!(
            "「{}」已被禁用（dst = 0x0000），这个键现在完全没反应。\
             恢复只能靠备份文件或重新绑定。",
            proto::BUTTON_NAME[slot]
        ))
    } else {
        None
    });

    cycle(
        dev,
        rid,
        &spec,
        move |st: &State| format!("{:04X}", st.buttons[slot]),
        move |_| format!("{dst:04X}"),
        move |before: &State| {
            let orig = before.buttons[slot];
            (
                Box::new(move |d: &mut HidDevice, r: u8| {
                    proto::write_button(d, r, slot, dst)
                }),
                Box::new(move |d: &mut HidDevice, r: u8| {
                    proto::write_button(d, r, slot, orig)
                }),
            )
        },
        reader,
        log,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_kinds_are_distinguishable() {
        // 这是给 GUI 用的核心契约：休眠和参数非法必须能分开
        let a = OpsError::asleep("写入无应答");
        let b = OpsError::invalid("DPI 范围 100-42000");
        assert!(a.is_asleep());
        assert!(!a.is_invalid());
        assert!(b.is_invalid());
        assert!(!b.is_asleep());
        assert_ne!(a.kind(), b.kind());
    }

    #[test]
    fn asleep_display_has_hint() {
        let e = OpsError::asleep("写入无应答");
        assert!(e.to_string().contains("移动一下鼠标"));
        // 参数非法不该带休眠提示
        let e = OpsError::invalid("亮度范围 0-100");
        assert!(!e.to_string().contains("移动一下鼠标"));
    }

    #[test]
    fn read_error_mapping() {
        assert_eq!(
            map_read_error("读 12 04 00 无应答".to_string()).kind(),
            ErrorKind::Asleep
        );
        assert_eq!(
            map_read_error("帧长度不足".to_string()).kind(),
            ErrorKind::Other
        );
    }

    #[test]
    fn report_summary_shapes() {
        let mut r = ApplyReport {
            applied: true,
            restored: true,
            revert_attempted: true,
            ..Default::default()
        };
        assert_eq!(r.summary(), "写入生效=是  已还原=是");

        r.applied = false;
        assert_eq!(r.summary(), "写入生效=否  已还原=是");

        r.restored = false;
        assert_eq!(r.summary(), "写入生效=否  已还原=**否**");

        let mut k = ApplyReport {
            applied: true,
            restored: false,
            revert_attempted: false,
            ..Default::default()
        };
        assert_eq!(k.summary(), "写入生效=是  已还原=没有（--keep）");
        k.applied = false;
        assert_eq!(k.summary(), "写入生效=否  已还原=没有（--keep）");
    }

    #[test]
    fn parameter_bounds() {
        // 这些边界必须和 proto 的编解码一致，写错就是设备被截断
        assert!(!(1..=DPI_STAGES).contains(&0));
        assert!(!(1..=DPI_STAGES).contains(&(DPI_STAGES + 1)));
        assert!((100..=42000).contains(&100));
        assert!(!(100..=42000).contains(&99));
        assert!(!(100..=42000).contains(&42001));
        assert!(proto::poll_code_of(2000).is_none());
        assert_eq!(proto::poll_code_of(1000), Some(3));
        // DPI 50 步进：437 会被截断成 400
        assert_eq!(proto::decode_dpi(proto::encode_dpi(437)), 400);
    }
}
