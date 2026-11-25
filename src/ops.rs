//! CLI 与 GUI 共用的配置事务：新读基准、写入、完整读回确认，再按需提交。
//! 失败时仅在设备身份与配置号仍匹配时恢复 RAM，并报告恢复和持久化状态。

use crate::hid::is_nak;
use crate::hid::HidDevice;
use crate::proto::{self, State, BUTTON_SLOTS, DPI_STAGES, DPI_STEP};

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
            ErrorKind::Asleep => write!(f, "{}（设备未响应，动一下设备再试）", self.message),
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

/// 每次从设备新读完整配置，写入基准和读回验证均不使用展示缓存。
pub trait StateReader {
    fn read(&mut self, dev: &mut HidDevice, rid: u8) -> Result<crate::snapshot::Configuration, OpsError>;
    fn expected_identity(&self) -> Option<&str> { None }
}

pub struct DirectReader;

impl StateReader for DirectReader {
    fn read(&mut self, dev: &mut HidDevice, rid: u8) -> Result<crate::snapshot::Configuration, OpsError> {
        crate::snapshot::read_configuration(dev, rid).map_err(map_read_error)
    }
}

pub struct BoundReader<'a> {
    identity: &'a str,
}

impl<'a> BoundReader<'a> {
    pub fn new(identity: &'a str) -> Self { Self { identity } }
}

impl StateReader for BoundReader<'_> {
    fn read(&mut self, dev: &mut HidDevice, rid: u8) -> Result<crate::snapshot::Configuration, OpsError> {
        DirectReader.read(dev, rid)
    }

    fn expected_identity(&self) -> Option<&str> { Some(self.identity) }
}

/// 把配置读取的字符串错误归类成 [`OpsError`]。
pub fn map_read_error(msg: String) -> OpsError {
    if msg.contains("无应答") || msg.contains("休眠") {
        OpsError::asleep(msg)
    } else {
        OpsError::other(msg)
    }
}

// ==================== 配置事务 ====================

/// 一次 setter 的完整报告。GUI 直接拿去渲染，不用再解析字符串。
#[derive(Debug, Clone, Default)]
pub struct ApplyReport {
    /// 新值是否真的被设备接受（读回与期望一致）
    pub applied: bool,
    /// 原配置已完整读回确认。
    pub restored: bool,
    pub revert_attempted: bool,
    pub committed: bool,

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
    /// 写入、恢复和提交分别报告。
    pub fn summary(&self) -> String {
        format!(
            "写入生效={}  已还原={}  已提交={}",
            if self.applied { "是" } else { "否" },
            if !self.revert_attempted {
                "未执行"
            } else if self.restored {
                "是"
            } else {
                "**否**"
            },
            if self.committed { "是" } else { "否" }
        )
    }

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

/// 一次 setter 的静态参数（与设备当前状态无关的那部分）。
pub struct CycleSpec {
    /// 标题，日志第一行用
    pub title: String,
    /// 保留新值不还原
    pub keep: bool,
    /// 最终配置读回确认后 commit 进 flash
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Setting {
    Poll,
    Dpi(usize),
    Stage,
    Debounce,
    Snap,
    Tuning,
    Lod,
    Light,
    Energy,
    Button(usize),
}

impl Setting {
    fn write(self, dev: &mut HidDevice, rid: u8, state: &State) -> Option<Vec<u8>> {
        match self {
            Self::Poll => proto::write_reg(dev, rid, 0x04, &[state.poll_code]),
            Self::Dpi(stage) => proto::write_dpi(dev, rid, stage, state.dpi(stage), state.rgb[stage]),
            Self::Stage => proto::write_reg(dev, rid, 0x09, &[state.active_stage]),
            Self::Debounce => proto::write_reg(dev, rid, 0x05, &[state.debounce_code]),
            Self::Snap => proto::write_reg(dev, rid, 0x06, &[state.angle_snap]),
            Self::Tuning => proto::write_reg(dev, rid, 0x0B, &state.angle_tuning.to_le_bytes()),
            Self::Lod => proto::write_lod(dev, rid, state.lod),
            Self::Light => proto::write_light(dev, rid, state.light_mode, state.light_bright,
                state.light_rgb, state.light_dir, state.light_random, state.light_speed),
            Self::Energy => proto::write_energy(dev, rid, state.sleep_mode, state.low_warn, state.full_effect),
            Self::Button(slot) => proto::write_button(dev, rid, slot, state.buttons[slot]),
        }
    }
}

trait TransactionDevice {
    fn expected_identity(&self) -> Option<&str> { None }
    fn identity(&mut self) -> Result<String, OpsError>;
    fn read(&mut self) -> Result<crate::snapshot::Configuration, OpsError>;
    fn write(&mut self, setting: Setting, state: &State) -> Option<Vec<u8>>;
    fn commit(&mut self) -> Option<Vec<u8>>;
    fn settle(&mut self);
    fn invalidate(&mut self);
}

struct HidTransaction<'a> {
    dev: &'a mut HidDevice,
    rid: u8,
    reader: &'a mut dyn StateReader,
}

impl TransactionDevice for HidTransaction<'_> {
    fn expected_identity(&self) -> Option<&str> { self.reader.expected_identity() }

    fn identity(&mut self) -> Result<String, OpsError> {
        crate::snapshot::read_device_identity(self.dev, self.rid)
            .ok_or_else(|| OpsError::other("无法确认设备身份"))
    }

    fn read(&mut self) -> Result<crate::snapshot::Configuration, OpsError> {
        self.reader.read(self.dev, self.rid)
    }

    fn write(&mut self, setting: Setting, state: &State) -> Option<Vec<u8>> {
        setting.write(self.dev, self.rid, state)
    }

    fn commit(&mut self) -> Option<Vec<u8>> {
        proto::commit(self.dev, self.rid)
    }

    fn settle(&mut self) { sleep_ms(SETTLE_MS); }
    fn invalidate(&mut self) { crate::snapshot::invalidate_disk_cache(); }
}

struct TransactionTarget {
    identity: String,
    profile: u8,
}

fn read_bound(
    device: &mut impl TransactionDevice,
    target: Option<&TransactionTarget>,
) -> Result<(crate::snapshot::Configuration, TransactionTarget), OpsError> {
    let identity = device.identity()?;
    if device.expected_identity().is_some_and(|expected| expected != identity) {
        return Err(OpsError::other("所选设备身份已变化，请刷新后重试"));
    }
    if target.is_some_and(|target| target.identity != identity) {
        return Err(OpsError::other("设备身份已变化"));
    }
    let configuration = device.read()?;
    if device.identity()? != identity {
        return Err(OpsError::other("读取期间设备身份已变化"));
    }
    let profile = configuration.state().profile;
    if target.is_some_and(|target| target.profile != profile) {
        return Err(OpsError::other("设备配置号已变化"));
    }
    Ok((configuration, TransactionTarget { identity, profile }))
}

fn configuration_mismatch(expected: &State, actual: &State, allow_calibration_reset: bool) -> Option<&'static str> {
    let groups = [
        ("性能", (expected.poll_code, expected.debounce_code, expected.angle_snap, expected.angle_tuning)
            == (actual.poll_code, actual.debounce_code, actual.angle_snap, actual.angle_tuning)),
        ("DPI 数值", expected.enc == actual.enc),
        ("DPI 颜色", expected.rgb == actual.rgb),
        ("配置号/当前档位", (expected.profile, expected.active_stage) == (actual.profile, actual.active_stage)),
        ("能量块", (expected.sleep_mode, expected.low_warn, expected.full_effect)
            == (actual.sleep_mode, actual.low_warn, actual.full_effect)),
        ("LOD", expected.lod == actual.lod),
        ("灯光", (expected.light_mode, expected.light_bright, expected.light_rgb, expected.light_dir, expected.light_random, expected.light_speed)
            == (actual.light_mode, actual.light_bright, actual.light_rgb, actual.light_dir, actual.light_random, actual.light_speed)),
        ("按键", expected.buttons == actual.buttons),
        ("表面标定", allow_calibration_reset || (expected.surface_type, expected.material, expected.surface_extra, expected.calib)
            == (actual.surface_type, actual.material, actual.surface_extra, actual.calib)),
    ];
    groups.into_iter().find_map(|(name, matches)| (!matches).then_some(name))
}

fn check_reply(reply: Option<Vec<u8>>, action: &str) -> Result<(), OpsError> {
    match reply {
        Some(frame) if is_nak(&frame) => Err(OpsError::rejected(format!("{action}被设备 NAK"))),
        Some(_) => Ok(()),
        None => Err(OpsError::asleep(format!("{action}无应答"))),
    }
}

fn recover(
    device: &mut impl TransactionDevice,
    target: &TransactionTarget,
    before: &State,
    attempted: &[Setting],
    log: Logger,
) -> Result<crate::snapshot::Configuration, OpsError> {
    let (current, _) = read_bound(device, Some(target))
        .map_err(|error| OpsError::other(format!("恢复前无法确认原设备及配置，未再次写入：{}", error.message)))?;
    if configuration_mismatch(before, current.state(), false).is_none() {
        log("RESTORE      原配置已读回确认，无需回写");
        return Ok(current);
    }
    for &setting in attempted.iter().rev() {
        if let Err(error) = check_reply(device.write(setting, before), "恢复写入") {
            log(&error.message);
        }
    }
    device.settle();
    let (restored, _) = read_bound(device, Some(target))?;
    if let Some(group) = configuration_mismatch(before, restored.state(), false) {
        return Err(OpsError::mismatch(format!("{group}未恢复到原值")));
    }
    log("VERIFY       restored=YES（完整配置已读回确认）");
    Ok(restored)
}

fn transaction_failure(
    device: &mut impl TransactionDevice,
    target: &TransactionTarget,
    before: &State,
    attempted: &[Setting],
    cause: OpsError,
    commit_attempted: bool,
    log: Logger,
) -> OpsError {
    let recovery = match recover(device, target, before, attempted, log) {
        Ok(_) => "原配置已读回确认".to_string(),
        Err(error) => format!("恢复未确认：{}", error.message),
    };
    let persistence = if commit_attempted {
        "提交未确认，flash 状态未知，未再次提交"
    } else {
        "未发送 commit"
    };
    let calibration = if attempted.contains(&Setting::Lod) {
        "；LOD 写入可能重置表面标定，无法通过 LOD 命令恢复"
    } else { "" };
    let kind = if cause.kind == ErrorKind::Asleep { ErrorKind::Other } else { cause.kind };
    OpsError::new(kind, format!("{}；{recovery}；{persistence}{calibration}", cause.message))
}

fn transact<D, P, F>(
    device: &mut D,
    spec: &CycleSpec,
    probe: P,
    plan: F,
    log: Logger,
) -> Result<ApplyReport, OpsError>
where
    D: TransactionDevice,
    P: Fn(&State) -> String,
    F: FnOnce(&State) -> Result<(State, Vec<Setting>), OpsError>,
{
    log(&format!("== {} ==", spec.title));
    let (configuration, target) = read_bound(device, None)?;
    let before = configuration.state();
    let (expected, mut settings) = plan(before)?;
    let old_value = probe(before);
    let new_value = probe(&expected);
    log(&format!("BEFORE       {old_value}"));
    if configuration_mismatch(before, &expected, false).is_none() {
        settings.clear();
        log("目标配置未变，跳过写入");
    }
    device.invalidate();
    let result = (|| {
        let mut attempted = Vec::new();
        for &setting in &settings {
            attempted.push(setting);
            if let Err(error) = check_reply(device.write(setting, &expected), "写入") {
                return Err(transaction_failure(device, &target, before, &attempted, error, false, log));
            }
        }
        if !attempted.is_empty() { device.settle(); }
        let verified = read_bound(device, Some(&target)).and_then(|(after, _)| {
            if let Some(group) = configuration_mismatch(&expected, after.state(), settings.contains(&Setting::Lod)) {
                Err(OpsError::mismatch(format!("{group}读回与目标配置不一致")))
            } else {
                Ok(after)
            }
        });
        let after = match verified {
            Ok(after) => after,
            Err(error) => return Err(transaction_failure(device, &target, before, &attempted, error, false, log)),
        };
        let readback_value = probe(after.state());
        log(&format!("AFTER WRITE  {readback_value}"));
        log("VERIFY       applied=YES（完整配置已读回确认）");
        let mut report = ApplyReport {
            applied: true, old_value, new_value, readback_value,
            warning: spec.warning.clone(), ..Default::default()
        };
        if !spec.keep && !attempted.is_empty() {
            report.revert_attempted = true;
            let restored = recover(device, &target, before, &attempted, log)
                .map_err(|error| OpsError::other(format!(
                    "试写已确认，但恢复未确认：{}；未发送 commit{}",
                    error.message,
                    if attempted.contains(&Setting::Lod) { "；表面标定无法通过 LOD 命令恢复" } else { "" }
                )))?;
            report.restored = true;
            report.restored_value = probe(restored.state());
        }
        if spec.save {
            if let Err(error) = check_reply(device.commit(), "commit ") {
                return Err(transaction_failure(device, &target, before, &attempted, error, true, log));
            }
            report.committed = true;
            log("COMMIT       已确认");
        }
        Ok(report)
    })();
    device.invalidate();
    result
}

#[allow(clippy::too_many_arguments)]
fn cycle<P, E>(
    dev: &mut HidDevice,
    rid: u8,
    spec: &CycleSpec,
    probe: P,
    mutate: E,
    setting: Setting,
    reader: &mut dyn StateReader,
    log: Logger,
) -> Result<ApplyReport, OpsError>
where
    P: Fn(&State) -> String,
    E: FnOnce(&mut State),
{
    transact(&mut HidTransaction { dev, rid, reader }, spec, probe, |before| {
        let mut expected = before.clone();
        mutate(&mut expected);
        Ok((expected, vec![setting]))
    }, log)
}

pub fn save_configuration(
    dev: &mut HidDevice,
    rid: u8,
    reader: &mut dyn StateReader,
    log: Logger,
) -> Result<ApplyReport, OpsError> {
    transact(&mut HidTransaction { dev, rid, reader },
        &CycleSpec::new("save").keep(true).save(true),
        |state| format!("配置 {}，DPI 档位 {}，回报率 {}", state.profile, state.active_stage, proto::poll_name(state.poll_code)),
        |before| Ok((before.clone(), Vec::new())), log)
}

pub fn restore_backup(
    dev: &mut HidDevice,
    rid: u8,
    backup: &crate::backup::Backup,
    save: bool,
    reader: &mut dyn StateReader,
    log: Logger,
) -> Result<ApplyReport, OpsError> {
    restore_with(&mut HidTransaction { dev, rid, reader }, backup, save, log)
}

pub fn read_bound_configuration(
    dev: &mut HidDevice, rid: u8, reader: &mut dyn StateReader,
) -> Result<crate::snapshot::Configuration, OpsError> {
    read_bound(&mut HidTransaction { dev, rid, reader }, None).map(|(configuration, _)| configuration)
}

fn restore_with(
    device: &mut impl TransactionDevice,
    backup: &crate::backup::Backup,
    save: bool,
    log: Logger,
) -> Result<ApplyReport, OpsError> {
    backup.validate().map_err(OpsError::invalid)?;
    let mut changes_lod = false;
    let mut report = transact(device, &CycleSpec::new("restore").keep(true).save(save),
        |state| format!("配置 {}，DPI 档位 {}，回报率 {}", state.profile, state.active_stage, proto::poll_name(state.poll_code)),
        |before| {
            let expected = backup.merge(before).map_err(OpsError::invalid)?;
            let mut settings = Vec::new();
            if before.poll_code != expected.poll_code { settings.push(Setting::Poll); }
            for stage in 0..DPI_STAGES {
                if (before.enc[stage], before.rgb[stage]) != (expected.enc[stage], expected.rgb[stage]) {
                    if !(proto::DPI_ENC_MIN..=proto::DPI_ENC_MAX).contains(&before.enc[stage]) {
                        return Err(OpsError::other(format!("DPI 档 {} 的原编码无法安全回写，未写入任何设置", stage + 1)));
                    }
                    settings.push(Setting::Dpi(stage));
                }
            }
            if before.active_stage != expected.active_stage { settings.push(Setting::Stage); }
            if before.debounce_code != expected.debounce_code { settings.push(Setting::Debounce); }
            if before.angle_snap != expected.angle_snap { settings.push(Setting::Snap); }
            if before.angle_tuning != expected.angle_tuning { settings.push(Setting::Tuning); }
            if (before.light_mode, before.light_bright, before.light_rgb, before.light_dir, before.light_random, before.light_speed)
                != (expected.light_mode, expected.light_bright, expected.light_rgb, expected.light_dir, expected.light_random, expected.light_speed)
            {
                settings.push(Setting::Light);
            }
            for slot in 0..BUTTON_SLOTS {
                if before.buttons[slot] != expected.buttons[slot] { settings.push(Setting::Button(slot)); }
            }
            if (before.sleep_mode, before.low_warn, before.full_effect)
                != (expected.sleep_mode, expected.low_warn, expected.full_effect)
            {
                settings.push(Setting::Energy);
            }
            changes_lod = before.lod != expected.lod;
            if changes_lod { settings.push(Setting::Lod); }
            Ok((expected, settings))
        }, log)?;
    if changes_lod {
        report.warning = Some("写 LOD 会重置表面标定（厂商确认无恢复命令）。".into());
    }
    Ok(report)
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
        |st: &mut State| st.poll_code = code,
        Setting::Poll,
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
        move |st: &mut State| {
            // 没指定颜色就沿用当前该档的颜色（同一帧里 DPI 和颜色一起写）
            st.enc[idx] = proto::encode_dpi(applied_dpi);
            st.rgb[idx] = rgb.unwrap_or(st.rgb[idx]);
        },
        Setting::Dpi(idx),
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
        |st: &mut State| st.active_stage = stage,
        Setting::Stage,
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
        |st: &mut State| st.debounce_code = code,
        Setting::Debounce,
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
        |st: &mut State| st.angle_snap = snap,
        Setting::Snap,
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

    cycle(
        dev,
        rid,
        &spec,
        |st: &State| st.angle_tuning.to_string(),
        |st: &mut State| st.angle_tuning = value,
        Setting::Tuning,
        reader,
        log,
    )
}

/// LOD 写入会重置表面标定；事务允许此副作用，但不宣称标定可回滚。
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

    let spec = CycleSpec::new(format!(
        "setlod -> {} (code {code})",
        proto::lod_name(code)
    ))
    .keep(keep)
    .save(save)
    .warning(Some(
        "写 LOD 会重置表面标定（厂商确认无恢复命令）。".to_string(),
    ));

    cycle(
        dev,
        rid,
        &spec,
        |st: &State| st.lod.to_string(),
        |st: &mut State| st.lod = code,
        Setting::Lod,
        reader,
        log,
    )
}

/// 未指定的灯光字段沿用完整基准，整块读回确认。
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
        move |st: &mut State| {
            st.light_mode = mode;
            st.light_bright = bright.unwrap_or(st.light_bright);
            st.light_rgb = rgb.unwrap_or(st.light_rgb);
            st.light_dir = dir.unwrap_or(st.light_dir);
            st.light_random = random.unwrap_or(st.light_random);
            st.light_speed = speed.unwrap_or(st.light_speed);
        },
        Setting::Light,
        reader,
        log,
    )
}

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
        |st: &mut State| st.sleep_mode = code,
        Setting::Energy,
        reader,
        log,
    )
}

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
        |st: &mut State| st.low_warn = percent,
        Setting::Energy,
        reader,
        log,
    )
}

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
        |st: &mut State| st.full_effect = on,
        Setting::Energy,
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
        move |st: &mut State| st.buttons[slot] = dst,
        Setting::Button(slot),
        reader,
        log,
    )
}

#[cfg(test)]
mod transaction_tests {
    use super::*;
    use std::collections::VecDeque;

    fn baseline() -> State {
        State {
            poll_code: 3, debounce_code: 1, active_stage: 1,
            enc: [15; DPI_STAGES], rgb: [[10, 20, 30]; DPI_STAGES],
            power: 70, sleep_mode: 2, low_warn: 20, full_effect: 1,
            surface_type: 1, material: 2, calib: [10, 20, 30],
            light_mode: 2, light_bright: 50, light_rgb: [20, 30, 40],
            buttons: proto::BUTTON_SRC, ..Default::default()
        }
    }

    fn configuration(state: &State) -> crate::snapshot::Configuration {
        crate::snapshot::read_configuration_with(3, |rid, cmd, sub, tail| {
            let mut frame = vec![0; 64];
            frame[..5].copy_from_slice(&[rid, cmd, sub, tail[0], tail[1]]);
            match (sub, tail[0]) {
                (0x04, 0) => {
                    frame[13] = state.poll_code;
                    frame[15] = state.debounce_code;
                    frame[17] = state.angle_snap;
                    frame[19..21].copy_from_slice(&state.angle_tuning.to_le_bytes());
                }
                (0x04, 2) => {
                    for (index, encoded) in state.enc.iter().enumerate() {
                        frame[5 + index * 4..7 + index * 4].copy_from_slice(&(*encoded as u16).to_le_bytes());
                    }
                }
                (0x04, 3) => {
                    for (index, color) in state.rgb.iter().enumerate() {
                        frame[5 + index * 3..8 + index * 3].copy_from_slice(color);
                    }
                }
                (0x00, 0) => { frame[11] = state.profile; frame[12] = state.active_stage; }
                (0x07, 0) => {
                    frame[5] = state.power; frame[6] = state.sleep_mode; frame[7] = state.low_warn;
                    frame[8..10].copy_from_slice(&state.voltage_mv.to_le_bytes());
                    frame[10] = state.charging; frame[11] = state.battery_type; frame[12] = state.full_effect;
                }
                (0x06, 0) => {
                    frame[5..9].copy_from_slice(&[state.surface_type, state.material, state.surface_extra, state.lod]);
                    frame[9..12].copy_from_slice(&state.calib);
                }
                (0x03, 0) => {
                    frame[5] = state.light_mode; frame[6] = state.light_bright;
                    frame[7..10].copy_from_slice(&state.light_rgb);
                    frame[11..14].copy_from_slice(&[state.light_dir, state.light_random, state.light_speed]);
                }
                (0x05, 0) => {
                    for (index, button) in state.buttons.iter().enumerate() {
                        frame[5 + index * 2..7 + index * 2].copy_from_slice(&button.to_le_bytes());
                    }
                }
                _ => panic!("unexpected configuration request"),
            }
            Some(frame)
        }).unwrap()
    }

    struct FakeDevice {
        expected_identity: Option<String>,
        reads: VecDeque<Result<State, OpsError>>,
        identities: VecDeque<Result<String, OpsError>>,
        replies: VecDeque<Option<Vec<u8>>>,
        commit_reply: Option<Vec<u8>>,
        events: Vec<&'static str>,
        writes: Vec<(Setting, State)>,
    }

    impl FakeDevice {
        fn new(reads: Vec<Result<State, OpsError>>) -> Self {
            Self {
                expected_identity: None,
                reads: reads.into(), identities: VecDeque::new(), replies: VecDeque::new(),
                commit_reply: Some(vec![0; 64]), events: Vec::new(), writes: Vec::new(),
            }
        }

        fn operations(&self) -> Vec<&'static str> {
            self.events.iter().copied().filter(|event| matches!(*event, "read" | "write" | "commit")).collect()
        }
    }

    impl TransactionDevice for FakeDevice {
        fn expected_identity(&self) -> Option<&str> { self.expected_identity.as_deref() }

        fn identity(&mut self) -> Result<String, OpsError> {
            self.events.push("identity");
            self.identities.pop_front().unwrap_or_else(|| Ok("synthetic-mouse".into()))
        }

        fn read(&mut self) -> Result<crate::snapshot::Configuration, OpsError> {
            self.events.push("read");
            self.reads.pop_front().expect("unexpected configuration read").map(|state| configuration(&state))
        }

        fn write(&mut self, setting: Setting, state: &State) -> Option<Vec<u8>> {
            self.events.push("write");
            self.writes.push((setting, state.clone()));
            self.replies.pop_front().unwrap_or_else(|| Some(vec![3, 0x51, 0x37]))
        }

        fn commit(&mut self) -> Option<Vec<u8>> {
            self.events.push("commit");
            self.commit_reply.clone()
        }

        fn settle(&mut self) { self.events.push("settle"); }
        fn invalidate(&mut self) { self.events.push("invalidate"); }
    }

    fn change_sleep(device: &mut FakeDevice, keep: bool, save: bool) -> Result<ApplyReport, OpsError> {
        transact(device, &CycleSpec::new("sleep").keep(keep).save(save),
            |state| state.sleep_mode.to_string(),
            |before| {
                let mut expected = before.clone();
                expected.sleep_mode = 3;
                Ok((expected, vec![Setting::Energy]))
            },
            &mut |_| {})
    }

    fn changed() -> State {
        State { sleep_mode: 3, ..baseline() }
    }

    #[test]
    fn queued_command_rejects_replacement_before_reading_or_writing_configuration() {
        let mut device = FakeDevice::new(Vec::new());
        device.expected_identity = Some("selected-mouse".into());
        device.identities.push_back(Ok("replacement-mouse".into()));
        assert!(change_sleep(&mut device, true, true).unwrap_err().message.contains("身份已变化"));
        assert!(device.operations().is_empty());
    }

    #[test]
    fn queued_restore_rejects_replacement_before_any_write() {
        let mut device = FakeDevice::new(Vec::new());
        device.expected_identity = Some("selected-mouse".into());
        assert!(restore(&mut device, "low_warn=35", true).unwrap_err().message.contains("身份已变化"));
        assert!(device.operations().is_empty());
    }

    #[test]
    fn queued_command_with_matching_identity_keeps_the_normal_transaction() {
        let mut device = FakeDevice::new(vec![Ok(baseline()), Ok(changed())]);
        device.expected_identity = Some("synthetic-mouse".into());
        assert!(change_sleep(&mut device, true, true).unwrap().committed);
        assert_eq!(device.operations(), ["read", "write", "read", "commit"]);
    }

    fn restore(device: &mut FakeDevice, text: &str, save: bool) -> Result<ApplyReport, OpsError> {
        let backup = crate::backup::parse(text).unwrap();
        restore_with(device, &backup, save, &mut |_| {})
    }

    fn written_settings(device: &FakeDevice) -> Vec<Setting> {
        device.writes.iter().map(|(setting, _)| *setting).collect()
    }

    fn assert_partial_restore(text: &str, expected: State, setting: Setting) {
        let mut device = FakeDevice::new(vec![Ok(baseline()), Ok(expected.clone())]);
        let report = restore(&mut device, text, false).unwrap();
        assert!(report.applied && !report.committed);
        assert_eq!(written_settings(&device), [setting], "{text}");
        assert_eq!(configuration_mismatch(&expected, &device.writes[0].1, false), None, "{text}");
        assert_eq!(device.operations(), ["read", "write", "read"]);
    }

    #[test]
    fn restore_each_partial_energy_field_preserves_its_peers() {
        for (text, expected) in [
            ("poweroff_code=255", State { sleep_mode: 255, ..baseline() }),
            ("low_warn=35", State { low_warn: 35, ..baseline() }),
            ("full_effect=0", State { full_effect: 0, ..baseline() }),
        ] {
            assert_partial_restore(text, expected, Setting::Energy);
        }
    }

    #[test]
    fn restore_each_partial_light_field_preserves_its_peers() {
        for (text, expected) in [
            ("light_mode=1", State { light_mode: 1, ..baseline() }),
            ("light_bright=75", State { light_bright: 75, ..baseline() }),
            ("light_rgb=123456", State { light_rgb: [0x12, 0x34, 0x56], ..baseline() }),
            ("light_dir=1", State { light_dir: 1, ..baseline() }),
            ("light_random=1", State { light_random: 1, ..baseline() }),
            ("light_speed=80", State { light_speed: 80, ..baseline() }),
        ] {
            assert_partial_restore(text, expected, Setting::Light);
        }
    }

    #[test]
    fn restore_dpi_color_only_and_value_only_preserve_the_other_member() {
        for stage in 0..DPI_STAGES {
            let mut expected = baseline();
            expected.rgb[stage] = [0x12, 0x34, 0x56];
            assert_partial_restore(&format!("dpi{}_rgb=123456", stage + 1), expected, Setting::Dpi(stage));
            let mut expected = baseline();
            expected.enc[stage] = 31;
            assert_partial_restore(&format!("dpi{}_enc=31", stage + 1), expected, Setting::Dpi(stage));
        }
    }

    #[test]
    fn restore_preflights_late_invalid_fields_before_any_device_access() {
        for invalid in [
            "dpi4_enc=0", "dpi4_enc=840", "dpi4_enc=-1", "dpi4_enc=2147483647",
            "active_stage=0", "active_stage=5", "debounce_code=8", "angle_snap=2",
            "angle_tuning=-25", "angle_tuning=21", "angle_tuning=3", "lod=2",
            "light_mode=3", "light_mode=5", "light_bright=101", "light_rgb=中文",
            "light_dir=2", "light_random=2", "light_speed=101", "button7=65536",
            "poweroff_code=5", "poweroff_code=254", "low_warn=101", "full_effect=2",
            "low_warn=invalid", "low_warn=20\nlow_warn=invalid", "poll_code=4",
        ] {
            let mut device = FakeDevice::new(Vec::new());
            let error = restore(&mut device, &format!("dpi1_enc=31\n{invalid}"), true).unwrap_err();
            assert_eq!(error.kind, ErrorKind::Invalid, "{invalid}");
            assert!(device.events.is_empty(), "{invalid}");
        }
    }

    #[test]
    fn restore_validates_programmatic_backups_in_shared_core() {
        let backup = crate::backup::Backup { low_warn: Some(101), keys_found: 1, ..Default::default() };
        let mut device = FakeDevice::new(Vec::new());
        assert_eq!(restore_with(&mut device, &backup, true, &mut |_| {}).unwrap_err().kind, ErrorKind::Invalid);
        assert!(device.events.is_empty());
    }

    #[test]
    fn restore_rejects_invalid_inherited_group_members_before_writing() {
        let mut invalid_dpi = baseline();
        invalid_dpi.enc[3] = 0;
        for (text, before) in [
            ("dpi4_rgb=123456", invalid_dpi),
            ("light_bright=70", State { light_mode: 3, ..baseline() }),
            ("light_mode=1", State { light_bright: 101, ..baseline() }),
            ("light_bright=70", State { light_dir: 2, ..baseline() }),
            ("light_bright=70", State { light_random: 2, ..baseline() }),
            ("light_bright=70", State { light_speed: 101, ..baseline() }),
            ("low_warn=30", State { sleep_mode: 5, ..baseline() }),
            ("poweroff_code=3", State { low_warn: 101, ..baseline() }),
            ("low_warn=30", State { full_effect: 2, ..baseline() }),
        ] {
            let mut device = FakeDevice::new(vec![Ok(before)]);
            assert!(restore(&mut device, &format!("poll_code=2\n{text}"), true).is_err(), "{text}");
            assert_eq!(device.operations(), ["read"], "{text}");
        }
    }

    #[test]
    fn restore_refuses_dpi_changes_that_cannot_be_rolled_back_exactly() {
        for encoded in [0, proto::DPI_ENC_MAX + 1] {
            let mut before = baseline();
            before.enc[3] = encoded;
            let mut device = FakeDevice::new(vec![Ok(before)]);
            let error = restore(&mut device, "poll_code=2\ndpi4_enc=31", true).unwrap_err();
            assert!(error.message.contains("原编码无法安全回写"));
            assert_eq!(device.operations(), ["read"]);
        }
    }

    #[test]
    fn restore_requires_a_complete_baseline() {
        let mut device = FakeDevice::new(vec![Err(OpsError::other("缺少配置帧 12:05:00"))]);
        assert!(restore(&mut device, "low_warn=30", true).is_err());
        assert_eq!(device.operations(), ["read"]);
    }

    #[test]
    fn restore_full_backup_verifies_all_groups_before_commit_and_writes_lod_last() {
        let expected = State {
            poll_code: 2, enc: [31; DPI_STAGES], rgb: [[1, 2, 3]; DPI_STAGES],
            active_stage: 4, debounce_code: 2, angle_snap: 1, angle_tuning: -10,
            lod: 1, light_mode: 1, light_bright: 70, light_rgb: [40, 50, 60],
            light_dir: 1, light_random: 1, light_speed: 40,
            buttons: [0; BUTTON_SLOTS], sleep_mode: 255, low_warn: 35, full_effect: 0,
            ..baseline()
        };
        let text = crate::backup::render(&configuration(&expected), "test", 3, "test");
        let after = State { calib: [0; 3], ..expected.clone() };
        let mut device = FakeDevice::new(vec![Ok(baseline()), Ok(after)]);
        let report = restore(&mut device, &text, true).unwrap();
        assert!(report.applied && report.committed);
        assert!(report.warning.unwrap().contains("表面标定"));
        assert_eq!(written_settings(&device), [
            Setting::Poll, Setting::Dpi(0), Setting::Dpi(1), Setting::Dpi(2), Setting::Dpi(3),
            Setting::Stage, Setting::Debounce, Setting::Snap, Setting::Tuning, Setting::Light,
            Setting::Button(0), Setting::Button(1), Setting::Button(2), Setting::Button(3),
            Setting::Button(4), Setting::Button(5), Setting::Button(6), Setting::Button(7),
            Setting::Energy, Setting::Lod,
        ]);
        for (_, written) in &device.writes {
            assert_eq!(configuration_mismatch(&expected, written, false), None);
        }
        let operations = device.operations();
        assert_eq!(operations[0], "read");
        assert_eq!(&operations[operations.len() - 2..], ["read", "commit"]);
    }

    #[test]
    fn restore_unchanged_groups_skip_writes_including_lod() {
        let text = crate::backup::render(&configuration(&baseline()), "test", 3, "test");
        let mut device = FakeDevice::new(vec![Ok(baseline()), Ok(baseline())]);
        let report = restore(&mut device, &text, true).unwrap();
        assert!(report.applied && report.committed && report.warning.is_none());
        assert_eq!(device.operations(), ["read", "read", "commit"]);
    }

    #[test]
    fn restore_nak_or_lost_ack_stops_forward_writes_and_recovers_attempted_groups() {
        for reply in [Some(vec![3, 0xff, 0xaa]), None] {
            let partial = State { poll_code: 2, light_bright: 70, ..baseline() };
            let mut device = FakeDevice::new(vec![Ok(baseline()), Ok(partial), Ok(baseline())]);
            device.replies.extend([Some(vec![3, 0x51, 0x31]), reply]);
            let error = restore(&mut device, "poll_code=2\nlight_bright=70\nlow_warn=35\nlod=1", true).unwrap_err();
            assert!(error.message.contains("原配置已读回确认"));
            assert!(error.message.contains("未发送 commit"));
            assert!(!error.message.contains("未被修改"));
            assert_eq!(written_settings(&device), [Setting::Poll, Setting::Light, Setting::Light, Setting::Poll]);
            for (_, written) in &device.writes[2..] {
                assert_eq!(configuration_mismatch(&baseline(), written, false), None);
            }
            assert!(!device.events.contains(&"commit"));
        }
    }

    #[test]
    fn restore_readback_mismatch_rolls_back_before_any_commit() {
        let wrong = State { poll_code: 2, light_bright: 69, ..baseline() };
        let mut device = FakeDevice::new(vec![Ok(baseline()), Ok(wrong.clone()), Ok(wrong), Ok(baseline())]);
        let error = restore(&mut device, "poll_code=2\nlight_bright=70", true).unwrap_err();
        assert!(error.message.contains("灯光读回"));
        assert!(error.message.contains("原配置已读回确认"));
        assert_eq!(written_settings(&device), [Setting::Poll, Setting::Light, Setting::Light, Setting::Poll]);
        assert!(!device.events.contains(&"commit"));
    }

    #[test]
    fn restore_incomplete_readback_never_commits() {
        let applied = State { poll_code: 2, light_bright: 70, ..baseline() };
        let mut device = FakeDevice::new(vec![
            Ok(baseline()), Err(OpsError::other("缺少配置帧 12:07:00")), Ok(applied), Ok(baseline()),
        ]);
        let error = restore(&mut device, "poll_code=2\nlight_bright=70", true).unwrap_err();
        assert!(error.message.contains("原配置已读回确认"));
        assert!(!device.events.contains(&"commit"));
    }

    #[test]
    fn restore_unconfirmed_recovery_reports_remaining_uncertainty() {
        let partial = State { poll_code: 2, light_bright: 70, ..baseline() };
        let mut device = FakeDevice::new(vec![Ok(baseline()), Ok(partial.clone()), Ok(partial)]);
        device.replies.extend([
            Some(vec![3, 0x51, 0x31]), None, Some(vec![3, 0xff, 0xaa]), None,
        ]);
        let error = restore(&mut device, "poll_code=2\nlight_bright=70", true).unwrap_err();
        assert!(error.message.contains("恢复未确认"));
        assert!(!error.message.contains("原配置已读回确认"));
        assert!(!error.message.contains("未被修改"));
        assert!(!error.message.contains("未写入"));
        assert!(!device.events.contains(&"commit"));
    }

    #[test]
    fn restore_changed_profile_blocks_recovery_writes() {
        let switched = State { profile: 1, poll_code: 2, light_bright: 70, ..baseline() };
        let mut device = FakeDevice::new(vec![Ok(baseline()), Ok(switched.clone()), Ok(switched)]);
        let error = restore(&mut device, "poll_code=2\nlight_bright=70", true).unwrap_err();
        assert!(error.message.contains("配置号已变化"));
        assert!(error.message.contains("恢复未确认"));
        assert_eq!(written_settings(&device), [Setting::Poll, Setting::Light]);
        assert!(!device.events.contains(&"commit"));
    }

    #[test]
    fn restore_commit_nak_or_lost_ack_reports_unknown_flash_and_recovers_ram() {
        for reply in [Some(vec![3, 0xff, 0xaa]), None] {
            let expected = State { poll_code: 2, light_bright: 70, ..baseline() };
            let mut device = FakeDevice::new(vec![Ok(baseline()), Ok(expected.clone()), Ok(expected), Ok(baseline())]);
            device.commit_reply = reply;
            let error = restore(&mut device, "poll_code=2\nlight_bright=70", true).unwrap_err();
            assert!(error.message.contains("flash 状态未知"));
            assert!(error.message.contains("原配置已读回确认"));
            assert_eq!(device.operations(), ["read", "write", "write", "read", "commit", "read", "write", "write", "read"]);
        }
    }

    #[test]
    fn retained_setting_commits_only_after_complete_readback() {
        let mut device = FakeDevice::new(vec![Ok(baseline()), Ok(changed())]);
        let report = change_sleep(&mut device, true, true).unwrap();
        assert!(report.applied && report.committed && !report.revert_attempted);
        assert_eq!(device.operations(), ["read", "write", "read", "commit"]);
        assert_eq!(device.events.first(), Some(&"identity"));
        assert_eq!(device.events.last(), Some(&"invalidate"));
        assert_eq!(device.writes[0].1.low_warn, 20);
        assert_eq!(device.writes[0].1.full_effect, 1);
    }

    #[test]
    fn temporary_setting_is_restored_and_verified_before_saving_original() {
        let mut device = FakeDevice::new(vec![Ok(baseline()), Ok(changed()), Ok(changed()), Ok(baseline())]);
        let report = change_sleep(&mut device, false, true).unwrap();
        assert!(report.applied && report.restored && report.revert_attempted && report.committed);
        assert_eq!(device.operations(), ["read", "write", "read", "read", "write", "read", "commit"]);
        assert_eq!(device.writes[1].1.sleep_mode, 2);
    }

    #[test]
    fn successful_ram_only_write_never_commits() {
        let mut device = FakeDevice::new(vec![Ok(baseline()), Ok(changed())]);
        let report = change_sleep(&mut device, true, false).unwrap();
        assert!(report.applied && !report.committed);
        assert_eq!(device.operations(), ["read", "write", "read"]);
    }

    #[test]
    fn missing_baseline_configuration_sends_no_write_or_commit() {
        let mut device = FakeDevice::new(vec![Err(OpsError::other("缺少配置帧 12:05:00"))]);
        assert!(change_sleep(&mut device, true, true).is_err());
        assert_eq!(device.operations(), ["read"]);
    }

    #[test]
    fn readback_failure_attempts_verified_recovery_without_commit() {
        let mut device = FakeDevice::new(vec![
            Ok(baseline()), Err(OpsError::other("缺少配置帧 12:07:00")),
            Ok(changed()), Ok(baseline()),
        ]);
        let error = change_sleep(&mut device, true, true).unwrap_err();
        assert!(error.message.contains("缺少配置帧"));
        assert!(error.message.contains("原配置已读回确认"));
        assert!(error.message.contains("未发送 commit"));
        assert_eq!(device.operations(), ["read", "write", "read", "read", "write", "read"]);
    }

    #[test]
    fn collateral_energy_change_blocks_commit_even_when_requested_value_matches() {
        let corrupt = State { low_warn: 0, ..changed() };
        let mut device = FakeDevice::new(vec![Ok(baseline()), Ok(corrupt.clone()), Ok(corrupt), Ok(baseline())]);
        let error = change_sleep(&mut device, true, true).unwrap_err();
        assert!(error.message.contains("能量块读回"));
        assert!(error.message.contains("原配置已读回确认"));
        assert_eq!(device.writes[1].1.low_warn, 20);
        assert!(!device.events.contains(&"commit"));
    }

    #[test]
    fn nak_or_lost_ack_with_applied_write_still_recovers_ram() {
        for reply in [Some(vec![3, 0xff, 0xaa]), None] {
            let mut device = FakeDevice::new(vec![Ok(baseline()), Ok(changed()), Ok(baseline())]);
            device.replies.push_back(reply);
            let error = change_sleep(&mut device, true, true).unwrap_err();
            assert!(error.message.contains("原配置已读回确认"));
            assert_eq!(device.operations(), ["read", "write", "read", "write", "read"]);
        }
    }

    #[test]
    fn rejected_write_with_unchanged_state_needs_no_recovery_write() {
        let mut device = FakeDevice::new(vec![Ok(baseline()), Ok(baseline())]);
        device.replies.push_back(Some(vec![3, 0xff, 0xaa]));
        let error = change_sleep(&mut device, true, true).unwrap_err();
        assert!(error.message.contains("NAK"));
        assert!(error.message.contains("原配置已读回确认"));
        assert_eq!(device.operations(), ["read", "write", "read"]);
    }

    #[test]
    fn recovery_failure_is_not_reported_as_restored() {
        let mut device = FakeDevice::new(vec![
            Ok(baseline()), Err(OpsError::asleep("读回无应答")),
            Ok(changed()), Ok(changed()),
        ]);
        device.replies.extend([Some(vec![3, 0x51, 0x37]), Some(vec![3, 0xff, 0xaa])]);
        let error = change_sleep(&mut device, true, true).unwrap_err();
        assert!(error.message.contains("恢复未确认"));
        assert!(!error.message.contains("原配置已读回确认"));
        assert!(!device.events.contains(&"commit"));
    }

    #[test]
    fn unavailable_recovery_preflight_does_not_write_blindly() {
        let mut device = FakeDevice::new(vec![
            Ok(baseline()), Err(OpsError::asleep("读回无应答")), Err(OpsError::asleep("设备无应答")),
        ]);
        let error = change_sleep(&mut device, true, true).unwrap_err();
        assert!(error.message.contains("未再次写入"));
        assert_eq!(device.writes.len(), 1);
        assert!(!device.events.contains(&"commit"));
    }

    #[test]
    fn changed_device_or_profile_blocks_recovery_and_commit() {
        let mut device = FakeDevice::new(vec![Ok(baseline())]);
        device.identities.extend([
            Ok("synthetic-mouse".into()), Ok("synthetic-mouse".into()),
            Ok("other-mouse".into()), Ok("other-mouse".into()),
        ]);
        let error = change_sleep(&mut device, true, true).unwrap_err();
        assert!(error.message.contains("设备身份已变化"));
        assert_eq!(device.operations(), ["read", "write"]);
        let switched = State { profile: 1, ..changed() };
        let mut device = FakeDevice::new(vec![Ok(baseline()), Ok(switched.clone()), Ok(switched)]);
        let error = change_sleep(&mut device, true, true).unwrap_err();
        assert!(error.message.contains("配置号已变化"));
        assert_eq!(device.operations(), ["read", "write", "read", "read"]);
    }

    #[test]
    fn identity_change_while_reading_baseline_prevents_any_write() {
        let mut device = FakeDevice::new(vec![Ok(baseline())]);
        device.identities.extend([Ok("first".into()), Ok("second".into())]);
        assert!(change_sleep(&mut device, true, true).unwrap_err().message.contains("读取期间设备身份已变化"));
        assert_eq!(device.operations(), ["read"]);
    }

    #[test]
    fn failed_commit_reports_unknown_flash_and_does_not_commit_recovery() {
        for reply in [None, Some(vec![3, 0xff, 0xaa])] {
            let mut device = FakeDevice::new(vec![Ok(baseline()), Ok(changed()), Ok(changed()), Ok(baseline())]);
            device.commit_reply = reply;
            let error = change_sleep(&mut device, true, true).unwrap_err();
            assert!(error.message.contains("flash 状态未知"));
            assert!(error.message.contains("原配置已读回确认"));
            assert_eq!(device.operations(), ["read", "write", "read", "commit", "read", "write", "read"]);
        }
    }

    #[test]
    fn battery_telemetry_changes_do_not_fail_configuration_verification() {
        let mut after = changed();
        after.power = 69; after.charging = 1; after.voltage_mv = 4000;
        let mut device = FakeDevice::new(vec![Ok(baseline()), Ok(after)]);
        assert!(change_sleep(&mut device, true, true).unwrap().committed);
    }

    #[test]
    fn unchanged_target_skips_device_writes_but_can_save_verified_state() {
        let mut device = FakeDevice::new(vec![Ok(changed()), Ok(changed())]);
        let report = change_sleep(&mut device, true, true).unwrap();
        assert!(report.applied && report.committed);
        assert_eq!(device.operations(), ["read", "read", "commit"]);
    }

    #[test]
    fn lod_calibration_reset_cannot_be_reported_as_successful_rollback() {
        let mut applied = baseline();
        applied.lod = 1; applied.calib = [0; 3];
        let restored_lod = State { lod: 0, ..applied.clone() };
        let mut device = FakeDevice::new(vec![Ok(baseline()), Ok(applied.clone()), Ok(applied), Ok(restored_lod)]);
        let error = transact(&mut device, &CycleSpec::new("LOD").save(true),
            |state| state.lod.to_string(), |before| {
                let mut expected = before.clone(); expected.lod = 1;
                Ok((expected, vec![Setting::Lod]))
            }, &mut |_| {}).unwrap_err();
        assert!(error.message.contains("表面标定"));
        assert!(error.message.contains("恢复未确认"));
        assert!(!device.events.contains(&"commit"));
    }
}
