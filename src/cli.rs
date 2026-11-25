//! 命令行子命令实现。
//!
//! 设置操作转调 [`crate::ops`] 的验证与提交事务；这里处理参数、文件和终端输出。
//!
//! 展示读取可使用 [`SnapshotCache`]；备份、写入基准和读回验证均新读完整配置。

use crate::backup;
use crate::hid::{is_nak, HidDevice, HidInterface, Link};
use crate::ops::{self, ApplyReport, ErrorKind, OpsError, StateReader};
use crate::proto::LIGHT_ZONE;
use crate::report;
use crate::snapshot::{FetchOutcome, SnapshotCache};

mod quick;

// ==================== 上下文 ====================

/// 一次命令执行的上下文：链路信息 + 快照缓存。
///
/// **刻意不持有设备句柄**：`ops::cycle` 同时需要 `&mut HidDevice` 和
/// `&mut dyn StateReader`，把设备句柄留在 `Ctx` 里会自己借自己。
pub struct Ctx<'a> {
    pub link: &'a Link,
    pub cache: SnapshotCache,
}

impl<'a> Ctx<'a> {
    pub fn new(link: &'a Link, ttl_sec: u64) -> Self {
        Ctx {
            link,
            cache: SnapshotCache::new(ttl_sec),
        }
    }

    pub fn rid(&self) -> u8 {
        self.link.report_id
    }

    /// 取快照：内存 → 磁盘 → 真读设备。
    fn snapshot(&mut self, dev: &mut HidDevice) -> FetchOutcome {
        self.cache.get(dev, self.link)
    }

    fn state(&mut self, dev: &mut HidDevice) -> Result<crate::snapshot::Configuration, String> {
        self.invalidate();
        crate::snapshot::read_configuration(dev, self.rid())
    }

    /// 操作结束：整体失效，下一次命令会重新读设备。
    fn invalidate(&mut self) {
        self.cache.invalidate();
    }
}

impl StateReader for Ctx<'_> {
    fn read(&mut self, dev: &mut HidDevice, rid: u8) -> Result<crate::snapshot::Configuration, OpsError> {
        if rid != self.rid() {
            return Err(OpsError::other("配置读取的 report ID 与所选设备不匹配"));
        }
        self.state(dev).map_err(ops::map_read_error)
    }
}

// ==================== 打印 ====================

/// 把 [`ApplyReport`] 打到终端。GUI 不用这个，直接读字段自己渲染。
///
/// 只打**结论块**：`BEFORE` / `AFTER WRITE` / `AFTER RESTORE` 已经由
/// [`ops::cycle`] 的日志回调逐行打过了，这里再打一遍只是噪声。
pub fn print_report(r: &ApplyReport) {
    println!();
    // 结论文字走 ApplyReport::summary()，终端和 GUI 共用一套说法
    println!("结论         {}", r.summary());
    // 读回不一致时把值并排列出来，用户一眼能看出设备到底接受了什么
    if !r.applied || (r.revert_attempted && !r.restored) {
        println!("  原值       {}", r.old_value);
        println!("  期望       {}", r.new_value);
        println!("  实际       {}", r.readback_value);
        if r.revert_attempted {
            println!("  还原后     {}", r.restored_value);
        }
    }
    if let Some(w) = &r.warning {
        println!("注意         {w}");
    }
}

/// 把 [`OpsError`] 的类别翻成人话。
pub fn error_hint(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Invalid => "参数不合法，本次没有碰设备。",
        ErrorKind::Asleep => "鼠标休眠中。移动一下鼠标即可唤醒，唤醒后重试。",
        ErrorKind::Rejected => "设备明确拒绝了这个命令（NAK）。",
        ErrorKind::Mismatch => "写进去了但读回不一致，请用 backup 文件核对。",
        ErrorKind::Other => "操作失败。",
    }
}

// ==================== 只读命令 ====================

/// `info` —— 只读，零风险。
///
/// 完整展示快照可复用缓存；命中前仍需查询设备身份。
pub fn cmd_info(ctx: &mut Ctx, dev: &mut HidDevice, force_fresh: bool) -> Result<(), String> {
    if force_fresh {
        ctx.cache.invalidate();
    }
    let out = ctx.snapshot(dev);
    let snap = &out.snapshot;

    if snap.frames.is_empty() {
        return Err("一帧都没读到。鼠标可能休眠中（移动一下即可唤醒），或接收器未连接。".into());
    }

    println!("== 设备身份 ==");
    report::print_identity(
        &snap.identity,
        &ctx.link.mode,
        ctx.link.report_id,
        &dev.product,
        &dev.serial,
    );
    println!(
        "  命令通道    in={} out={} 字节，report id 0x{:02X}",
        dev.input_len(),
        dev.output_len(),
        ctx.link.report_id
    );
    println!("  {}", light_zone_hint());

    println!();
    println!("== 电量 ==");
    if snap.failed.contains_key("12:07:00") {
        println!("  读取失败：{}", snap.failed["12:07:00"]);
    } else {
        report::print_power(&snap.state);
    }

    println!();
    println!("== 性能与设置 ==");
    report::print_state(&snap.state);

    // 缓存与耗时：让人一眼看出这次是「真读了」还是「吃缓存」
    let from_cache = out.elapsed.is_zero();
    println!();
    println!(
        "== 快照 ==\n  帧数        {} 帧{}",
        snap.frames.len(),
        if from_cache {
            "（来自缓存）".to_string()
        } else {
            format!("，耗时 {} ms", out.elapsed.as_millis())
        }
    );
    if from_cache {
        println!("  缓存年龄    {} 秒前", snap.wall_age_sec());
    }
    if !snap.failed.is_empty() {
        println!("  读取异常项  {} 帧", snap.failed.len());
        for w in snap.warnings() {
            println!("    - {w}");
        }
    }
    Ok(())
}

pub fn cmd_quick(ctx: &mut Ctx, dev: &mut HidDevice) -> Result<(), String> {
    let out = match ctx.cache.get_light(dev, ctx.link) {
        Some(o) => o,
        None => {
            // 休眠不是错误：给一个明确的、脚本能识别的状态行，退出码仍然是 0
            println!("state=asleep");
            println!("hint=鼠标休眠中，移动一下即可唤醒");
            return Ok(());
        }
    };
    print!("{}", quick::render(&out, out.snapshot.wall_age_sec()));
    Ok(())
}

/// `backup <file>` —— 只读，生成 31 键的完整备份。
///
/// 快照里已经包含 `12 06`（LOD 与表面标定），不需要再单独读一次。
pub fn cmd_backup(ctx: &mut Ctx, dev: &mut HidDevice, path: &str) -> Result<(), String> {
    let configuration = ctx.state(dev)?;
    let st = configuration.state();

    // 人读的时间戳，不要放 unix 秒（对照时还得自己换算）
    let timestamp = crate::settings::timestamp_for_humans();

    let text = backup::render(&configuration, &ctx.link.mode, ctx.link.report_id, &timestamp);

    // 备份目录不存在就先建（配置/数据放系统标准位置，不是 exe 同目录）；
    // 走原子写 —— 备份是 set_lod 重置标定之后唯一的退路，不能留半份。
    crate::paths::write_atomic(std::path::Path::new(path), text.as_bytes())
        .map_err(|e| format!("写文件 {path} 失败：{e}"))?;

    println!("== 备份 ==");
    report::print_state(st);
    println!();
    println!("已写入 {path}（{} 个键）", backup::BACKUP_KEYS.len());
    println!(
        "能量块：poweroff_code={} low_warn={} full_effect={}",
        st.sleep_mode, st.low_warn, st.full_effect
    );
    Ok(())
}

// ==================== 还原 ====================

/// `restore <file>`：整份预检、合并完整基准，读回确认后按需提交。
pub fn cmd_restore(ctx: &mut Ctx, dev: &mut HidDevice, path: &str, save: bool) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("读文件 {path} 失败：{e}"))?;
    let backup = backup::parse(&text)?;

    println!("== 还原 {path} ==");
    println!("识别到 {} / {} 个键", backup.keys_found, backup::BACKUP_KEYS.len());
    let result = ops::restore_backup(dev, ctx.rid(), &backup, save, &mut |line| println!("  {line}"));
    ctx.invalidate();
    let report = result.map_err(|error| error.message)?;
    print_report(&report);
    Ok(())
}

// ==================== setter ====================
//
// 每个 setter 只做三件事：解析参数 → 调 ops::* → 打报告。
// 设置事务与错误分类由 ops 统一处理。

/// setter 的统一收尾：打报告，再把「读回不一致」升级成非零退出码。
///
/// GUI 直接看 [`ApplyReport`] 的字段；CLI 没有报告面板，只能靠退出码，
/// 所以这条差异只在 CLI 这一侧落地一次。
fn finish(r: Result<ApplyReport, OpsError>) -> Result<ApplyReport, OpsError> {
    let r = r?;
    print_report(&r);
    r.into_result()
}

/// `setdpi <1-4> <dpi> [RRGGBB]`
pub fn cmd_setdpi(
    ctx: &mut Ctx,
    dev: &mut HidDevice,
    stage: usize,
    dpi: i32,
    rgb: Option<[u8; 3]>,
    keep: bool,
    save: bool,
) -> Result<ApplyReport, OpsError> {
    let mut log = |line: &str| println!("{line}");
    let rid = ctx.rid();
    finish(ops::set_dpi(dev, rid, stage, dpi, rgb, keep, save, ctx, &mut log))
}

/// `setpoll <125|250|500|1000>`
pub fn cmd_setpoll(
    ctx: &mut Ctx,
    dev: &mut HidDevice,
    hz: i32,
    keep: bool,
    save: bool,
) -> Result<ApplyReport, OpsError> {
    let mut log = |line: &str| println!("{line}");
    let rid = ctx.rid();
    finish(ops::set_poll(dev, rid, hz, keep, save, ctx, &mut log))
}

/// `setstage <1-4>` —— 当前 DPI 档位（寄存器 0x09，1-based）
pub fn cmd_setstage(
    ctx: &mut Ctx,
    dev: &mut HidDevice,
    stage: u8,
    keep: bool,
    save: bool,
) -> Result<ApplyReport, OpsError> {
    let mut log = |line: &str| println!("{line}");
    let rid = ctx.rid();
    finish(ops::set_stage(dev, rid, stage, keep, save, ctx, &mut log))
}

/// `setdebounce <0-7>`
pub fn cmd_setdebounce(
    ctx: &mut Ctx,
    dev: &mut HidDevice,
    code: u8,
    keep: bool,
    save: bool,
) -> Result<ApplyReport, OpsError> {
    let mut log = |line: &str| println!("{line}");
    let rid = ctx.rid();
    finish(ops::set_debounce(dev, rid, code, keep, save, ctx, &mut log))
}

/// `setsnap <0|1>`
pub fn cmd_setsnap(
    ctx: &mut Ctx,
    dev: &mut HidDevice,
    snap: u8,
    keep: bool,
    save: bool,
) -> Result<ApplyReport, OpsError> {
    let mut log = |line: &str| println!("{line}");
    let rid = ctx.rid();
    finish(ops::set_snap(dev, rid, snap, keep, save, ctx, &mut log))
}

/// `settuning <-20..20>` —— 角度微调，signed int16 LE，步进 5
pub fn cmd_settuning(
    ctx: &mut Ctx,
    dev: &mut HidDevice,
    value: i16,
    keep: bool,
    save: bool,
) -> Result<ApplyReport, OpsError> {
    let mut log = |line: &str| println!("{line}");
    let rid = ctx.rid();
    finish(ops::set_tuning(dev, rid, value, keep, save, ctx, &mut log))
}

/// `setlod <low|high>` —— ⚠️ 会重置表面标定（厂商确认无恢复命令）
pub fn cmd_setlod(
    ctx: &mut Ctx,
    dev: &mut HidDevice,
    code: u8,
    keep: bool,
    save: bool,
) -> Result<ApplyReport, OpsError> {
    let mut log = |line: &str| println!("{line}");
    let rid = ctx.rid();
    finish(ops::set_lod(dev, rid, code, keep, save, ctx, &mut log))
}

/// `setlight <mode> [brightness] [RRGGBB]`
///
/// mode 3（Rainbow）与 5（Comet）本型号**不支持**，设备会静默回落到 ColorCycle，
/// `ops::set_light` 会在碰设备之前拒绝。
pub fn cmd_setlight(
    ctx: &mut Ctx,
    dev: &mut HidDevice,
    mode: u8,
    bright: Option<u8>,
    rgb: Option<[u8; 3]>,
    keep: bool,
    save: bool,
) -> Result<ApplyReport, OpsError> {
    let mut log = |line: &str| println!("{line}");
    let rid = ctx.rid();
    // CLI 不暴露方向/随机/速度：本型号只有 Rainbow(3)/Comet(5) 用得上这三个字段，
    // 而这两个模式写进去会被设备静默回落到 ColorCycle。传 `None` = 沿用设备当前值。
    finish(ops::set_light(
        dev, rid, mode, bright, rgb, None, None, None, keep, save, ctx, &mut log,
    ))
}

/// `setbutton <slot 0-7> <dstHex>`
///
/// `dst = 0x0000` 等于**禁用该键**，只能靠备份还原 —— 必须二次确认。
pub fn cmd_setbutton(
    ctx: &mut Ctx,
    dev: &mut HidDevice,
    slot: usize,
    dst: u16,
    keep: bool,
    save: bool,
    confirmed: bool,
) -> Result<ApplyReport, OpsError> {
    let mut log = |line: &str| println!("{line}");
    let rid = ctx.rid();
    finish(ops::set_button(dev, rid, slot, dst, keep, save, confirmed, ctx, &mut log))
}

/// `setsleep <code>` —— 休眠（自动关机）时间。
///
/// code：0=1分 1=2分 2=3分 3=5分 4=10分 255=永不。**是枚举不是分钟数**，
/// 设备不校验范围（写 5..254 也原样存），所以这里在碰设备之前拒绝无定义值。
pub fn cmd_setsleep(
    ctx: &mut Ctx,
    dev: &mut HidDevice,
    code: u8,
    keep: bool,
    save: bool,
) -> Result<ApplyReport, OpsError> {
    let mut log = |line: &str| println!("{line}");
    let rid = ctx.rid();
    finish(ops::set_sleep(dev, rid, code, keep, save, ctx, &mut log))
}

/// `setlowbatt <percent>` —— 低电量告警阈值。
pub fn cmd_setlowbatt(
    ctx: &mut Ctx,
    dev: &mut HidDevice,
    percent: u8,
    keep: bool,
    save: bool,
) -> Result<ApplyReport, OpsError> {
    let mut log = |line: &str| println!("{line}");
    let rid = ctx.rid();
    finish(ops::set_low_batt(dev, rid, percent, keep, save, ctx, &mut log))
}

/// `setfulleffect <0|1>` —— 充满电时的灯效开关。
pub fn cmd_setfulleffect(
    ctx: &mut Ctx,
    dev: &mut HidDevice,
    on: u8,
    keep: bool,
    save: bool,
) -> Result<ApplyReport, OpsError> {
    let mut log = |line: &str| println!("{line}");
    let rid = ctx.rid();
    finish(ops::set_full_effect(dev, rid, on, keep, save, ctx, &mut log))
}

/// `save` —— 单独提交当前 RAM 里的设置进 flash。
pub fn cmd_save(ctx: &mut Ctx, dev: &mut HidDevice) -> Result<(), String> {
    let rid = ctx.rid();
    let result = ops::save_configuration(dev, rid, ctx, &mut |line| println!("{line}"));
    ctx.invalidate();
    let report = result.map_err(|error| error.to_string())?;
    print_report(&report);
    Ok(())
}

/// `estimate` —— 读当前电量 + 历史，打印续航/充满估算。
///
/// 读取一次新电量，写到与界面相同的设备历史。
pub fn cmd_estimate(ctx: &mut Ctx, dev: &mut HidDevice) -> Result<(), String> {
    println!("== estimate（电量历史与估算）==");
    let out = crate::snapshot::fetch(dev, &ctx.link.all_interfaces, ctx.link.report_id, &ctx.link.mode);
    let battery = out.snapshot.battery().ok_or("未读取到新电量，请唤醒设备后重试")?;
    let st = &out.snapshot.state;
    let identity = &out.snapshot.identity;
    let key = if !identity.mouse_sn.trim().is_empty() {
        crate::device::stable_key(&format!("device:{}", identity.mouse_sn.trim()))
    } else if !identity.mouse_mac.trim().is_empty() {
        crate::device::stable_key(&format!("device:{}", identity.mouse_mac.trim()))
    } else { ctx.link.device_key() };
    let mode = link_mode_code(ctx.link);
    let now_sec = out.snapshot.taken_unix as i64;
    let sample = crate::history::HistoryRow::new(now_sec, battery.percent.unwrap(), battery.charging == Some(true), mode);
    let aliases = [(ctx.link.device_key(), key.clone())];
    // The CLI does not enumerate all peers here, so it cannot safely claim an
    // unlabelled legacy file on behalf of the selected device.
    let file = crate::history::update_device_history(&[(key.clone(), sample)], &aliases, None)?;
    let rows = file.devices.get(&key).map(Vec::as_slice).unwrap_or(&[]);

    println!("历史文件    {}", crate::history::history_path_display());
    if crate::history::history_path_is_fallback() {
        println!("            （首选位置 %LOCALAPPDATA%\\rog-battery 不可写，已自动换到上面这个可写位置）");
    }
    println!(
        "已记录      {} 条{}{}",
        rows.len(),
        rows.first()
            .map(|r| format!("，最早 {}", fmt_ts(r.ts)))
            .unwrap_or_default(),
        rows.last()
            .map(|r| format!("，最近 {}", fmt_ts(r.ts)))
            .unwrap_or_default(),
    );
    println!(
        "当前读数    {}%  {}  {}",
        st.power,
        if st.charging == 1 { "充电中" } else { "使用中" },
        if mode == 1 { "有线" } else { "无线" }
    );

    // 合并三态后判断是否已充满，这一步和估算器内部的判定保持一致
    let charge = crate::history::decide_charge(st.power, st.charging == 1, mode);
    println!(
        "归并状态    {}",
        match charge {
            crate::history::ChargeState::Charging => "充电中",
            crate::history::ChargeState::Discharging => "使用中",
            crate::history::ChargeState::Full => "已充满",
        }
    );

    let est = crate::history::predict_device(rows, sample);
    println!();
    println!("估算        {}", est.text);
    if est.known {
        println!(
            "  斜率       {:+.2} %/小时（{} 个样本，跨度 {:.0} 分钟）",
            est.percent_per_hour, est.samples, est.span_minutes
        );
    } else if est.samples > 0 {
        println!("  参考       用了 {} 个样本", est.samples);
    }

    println!("  历史       已更新此设备的采样（变化 60s / 心跳 120s）");
    Ok(())
}

/// 连接模式转成历史文件里的数字：1=有线 2=2.4G（与 C# 版 `LinkMode` 一致）。
fn link_mode_code(link: &Link) -> i32 {
    match link.mode.as_str() { "Wired" => 1, "Wireless24G" => 2, "Bluetooth" => 3, _ => 0 }
}

/// 把 unix 秒格式化成 `MM-DD HH:MM`（本地时间，给历史记录列表看）。
fn fmt_ts(unix: i64) -> String {
    match crate::settings::format_unix_local(unix) {
        Some(s) => s,
        None => format!("unix {unix}"),
    }
}

/// `raw` —— 原始帧调试口：发一帧命令，把完整应答字节打出来。
///
/// 只用于**只读**核对（协议逆向、偏移验证）。写命令不要走这里：
/// 设置命令通过共享事务验证和恢复。
pub fn cmd_raw(ctx: &mut Ctx, dev: &mut HidDevice, cmd: u8, sub: u8, tail: &[u8], timeout_ms: u32) -> Result<(), String> {
    let rid = ctx.rid();
    cmd_raw_on(dev, rid, cmd, sub, tail, timeout_ms)
}

/// 同上，但直接在给定句柄上发（`--dongle` 走另一条通道时用）。
pub fn cmd_raw_on(
    dev: &mut HidDevice,
    rid: u8,
    cmd: u8,
    sub: u8,
    tail: &[u8],
    timeout_ms: u32,
) -> Result<(), String> {
    println!("== raw ==");
    println!("请求        report id 0x{rid:02X}  cmd 0x{cmd:02X}  sub 0x{sub:02X}  tail {tail:02X?}");
    match dev.xfer(rid, cmd, sub, tail, timeout_ms) {
        Some(m) => {
            if is_nak(&m) {
                println!("应答        NAK（03 FF AA）");
                return Err("设备拒绝该命令（NAK）".to_string());
            }
            println!("应答        {} 字节", m.len());
            report::hex_dump(&m);
            // 标出关键下标，偏移核对时最有用
            let n = m.len().min(24);
            println!();
            print!("下标        ");
            for i in 0..n {
                print!("{i:>3}");
            }
            println!();
            print!("值          ");
            for b in m.iter().take(n) {
                // 固定两位十六进制，列宽与上面的下标对齐
                print!("{b:>3X}");
            }
            println!();
            if m.len() > n {
                println!("（只标出前 {n} 字节的下标，完整数据看上面）");
            }
            Ok(())
        }
        None => Err(format!(
            "{timeout_ms} ms 内无应答（鼠标休眠中？先移动一下鼠标）"
        )),
    }
}

/// 灯区常量在 CLI 里只用于提示，避免 dead_code
pub fn light_zone_hint() -> String {
    format!("本型号唯一灯区 = zone {LIGHT_ZONE}（滚轮）")
}

/// 打开只读的接收器控制通道（`--dongle` 用）。
pub fn open_dongle(all: &[HidInterface]) -> Result<HidDevice, String> {
    HidDevice::open_matching(all, crate::hid::USAGE_PAGE_DONGLE_CTRL)
}
