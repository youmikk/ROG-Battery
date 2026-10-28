//! 命令行子命令实现。
//!
//! 所有**写**操作都转调 [`crate::ops`]：六步安全循环只有那一份实现，
//! 这里只负责「把结构化报告打到终端」和「文件/参数这类外壳工作」。
//!
//! 所有读都经过 [`Ctx`]：它包着「一次性读完 + 缓存」的 [`SnapshotCache`]。
//! 只读命令（`info` / `backup`）直接吃缓存；写命令的「读原值」也吃缓存，
//! 但写后的读回断言必须直读设备（RAM 已经变了）。

use crate::backup;
use crate::hid::{is_nak, HidDevice, HidInterface, Link};
use crate::ops::{self, ApplyReport, ErrorKind, OpsError, StateReader};
use crate::proto::{self, State, BUTTON_NAME, BUTTON_SLOTS, DPI_STAGES, LIGHT_ZONE};
use crate::snapshot::{FetchOutcome, SnapshotCache};

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
    fn snapshot(&mut self, dev: &mut HidDevice, allow_stale: bool) -> FetchOutcome {
        self.cache.get(
            dev,
            &self.link.all_interfaces,
            self.link.report_id,
            &self.link.mode,
            allow_stale,
        )
    }

    /// 当前完整状态（缓存优先）。
    fn state(&mut self, dev: &mut HidDevice) -> Result<State, String> {
        let out = self.snapshot(dev, false);
        if out.snapshot.state.poll_code == 0
            && out.snapshot.state.enc == [0; DPI_STAGES]
            && out.snapshot.failed.contains_key("12:04:00")
        {
            return Err("鼠标无应答（休眠中？先移动一下鼠标）".to_string());
        }
        Ok(out.snapshot.state.clone())
    }

    /// 操作结束：整体失效，下一次命令会重新读设备。
    fn invalidate(&mut self) {
        self.cache.invalidate();
    }
}

/// 给 [`ops::cycle`] 用的状态读取器。
///
/// * `fresh = false`：走缓存（「读原值」这一步）。
/// * `fresh = true`：**直读 8 个状态帧并让缓存失效**。
///   刻意只读 8 帧而不是全部 17 帧：写操作的断言只关心状态，
///   身份信息不会因为写而改变，读它们纯属浪费 27ms × 9。
impl StateReader for Ctx<'_> {
    fn read(&mut self, dev: &mut HidDevice, rid: u8, fresh: bool) -> Result<State, OpsError> {
        if !fresh {
            return self.state(dev).map_err(ops::map_read_error);
        }
        let r = proto::read_state(dev, rid);
        // 不管读成没成，RAM 已经变了，缓存都不能再信
        self.cache.invalidate();
        r.map_err(ops::map_read_error)
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
/// **一次读全 17 帧**（约 690ms，超出缓存有效期时才付），之后从缓存出。
pub fn cmd_info(ctx: &mut Ctx, dev: &mut HidDevice, force_fresh: bool) -> Result<(), String> {
    if force_fresh {
        ctx.cache.invalidate();
    }
    let out = ctx.snapshot(dev, false);
    let snap = &out.snapshot;

    if snap.frames.is_empty() {
        return Err("一帧都没读到。鼠标可能休眠中（移动一下即可唤醒），或接收器未连接。".into());
    }

    println!("== 设备身份 ==");
    proto::print_identity(
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
        proto::print_power(&snap.state);
    }

    println!();
    println!("== 性能与设置 ==");
    proto::print_state(&snap.state);

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

/// `info --light` —— **只读 3 帧**（实测 94-97ms），给脚本/后台周期轮询用。
///
/// 与全量 `info` 的区别：
/// - 只读电量帧、性能帧、profile 帧，**不读** DPI 档值/颜色/灯光/按键/身份
/// - 鼠标休眠时**第一帧就返回**，不会白等后面两帧的超时
///   （单帧 MISS = 一次 `DEFAULT_TIMEOUT_MS` = 1200ms；不提前返回就要付 3600ms）
///
/// 输出刻意做成一行机器可读的 `KEY=VALUE` 形式，方便 `.bat`/`.ps1` 直接解析。
pub fn cmd_quick(ctx: &mut Ctx, dev: &mut HidDevice) -> Result<(), String> {
    let rid = ctx.rid();
    let link_mode = ctx.link.mode.clone();
    let out = match ctx.cache.get_light(dev, rid, &link_mode) {
        Some(o) => o,
        None => {
            // 休眠不是错误：给一个明确的、脚本能识别的状态行，退出码仍然是 0
            println!("state=asleep");
            println!("hint=鼠标休眠中，移动一下即可唤醒");
            return Ok(());
        }
    };
    let st = &out.snapshot.state;
    let snap = &out.snapshot;

    println!("state=ok");
    if snap.frames.contains_key("12:07:00") {
        println!("level={}", st.power);
        println!("voltage_mv={}", st.voltage_mv);
        println!("charging={}", st.charging);
        println!("sleep_mode={}", st.sleep_mode);
        println!("low_warn={}", st.low_warn);
        println!("full_effect={}", st.full_effect);
    }
    if snap.frames.contains_key("12:04:00") {
        // raw 值与解析值并排给出：b13 的解析分支本机未被真正验证过，
        // 把原始字节一起吐出来，将来换个设备遇到异常值能立刻定位
        println!("poll={}", proto::poll_name(st.poll_code));
        println!("poll_code={}", st.poll_code);
        println!("debounce_code={}", st.debounce_code);
        println!("angle_snap={}", st.angle_snap);
        println!("angle_tuning={}", st.angle_tuning);
    }
    if snap.frames.contains_key("12:00:00") {
        println!("active_stage={}", st.active_stage);
        println!("profile={}", st.profile);
    }
    println!("frames={}", out.frames_read);
    if out.elapsed.is_zero() {
        println!("cached=yes");
        println!("cache_age_sec={}", snap.wall_age_sec());
    } else {
        println!("cached=no");
        println!("elapsed_ms={}", out.elapsed.as_millis());
    }
    Ok(())
}

/// `backup <file>` —— 只读，生成 31 键的完整备份。
///
/// 快照里已经包含 `12 06`（LOD 与表面标定），不需要再单独读一次。
pub fn cmd_backup(ctx: &mut Ctx, dev: &mut HidDevice, path: &str) -> Result<(), String> {
    let out = ctx.snapshot(dev, false);
    let st = out.snapshot.state.clone();
    if out.snapshot.frames.is_empty() {
        return Err("读不到设备状态，无法备份".into());
    }

    // 人读的时间戳，不要放 unix 秒（对照时还得自己换算）
    let timestamp = crate::settings::timestamp_for_humans();

    let text = backup::render(&st, &ctx.link.mode, ctx.link.report_id, &timestamp);

    // 备份目录不存在就先建（配置/数据放系统标准位置，不是 exe 同目录）
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            crate::paths::ensure_dir(parent)?;
        }
    }
    std::fs::write(path, text.as_bytes()).map_err(|e| format!("写文件 {path} 失败：{e}"))?;

    println!("== 备份 ==");
    proto::print_state(&st);
    println!();
    println!("已写入 {path}（{} 个键）", backup::BACKUP_KEYS.len());
    println!(
        "能量块：poweroff_code={} low_warn={} full_effect={}",
        st.sleep_mode, st.low_warn, st.full_effect
    );
    if !out.snapshot.failed.is_empty() {
        eprintln!();
        eprintln!(
            "警告：有 {} 帧没读到，备份可能不完整：",
            out.snapshot.failed.len()
        );
        for w in out.snapshot.warnings() {
            eprintln!("  - {w}");
        }
    }
    Ok(())
}

// ==================== 还原 ====================

/// 把一份已解析的备份逐项写回设备。
///
/// # 为什么它从 `cmd_restore` 里抽出来
///
/// GUI 也要「从备份还原」，但 GUI 里没有终端 —— `cmd_restore` 满篇 `println!`
/// 和 `eprintln!`，直接调会把文字吐进一个已经被 `FreeConsole` 脱离的进程。
///
/// 抽成这个函数而不是在 gui.rs 里重写一遍，是因为这段代码里**每一条校验都对应
/// 一次真实事故**（DPI 编码范围、缺 `dpiN_rgb` 时兜底成黑色、灯光缺字段时
/// 把亮度打到 50、能量块填错槽位）。两套实现必然漂移，而漂移的那次
/// 就是用户设备被写坏的那次。
///
/// `log` 收的是**已格式化好的一行**（不带换行），调用方决定打到终端还是
/// 塞进界面的结果框。
pub fn apply_backup(
    dev: &mut HidDevice,
    rid: u8,
    b: &backup::Backup,
    save: bool,
    log: &mut dyn FnMut(&str),
) -> Result<(), String> {
    // ---- 4 档 DPI + 颜色 ----
    //
    // 这里刻意**不走 `ops::set_dpi`**：restore 是「按备份原样铺回去」，
    // 不该有「写-读回-自动还原」那套六步循环（否则还原过程中每一项都会被
    // 临时改两次）。但绕过 ops 也就绕过了它的参数校验，所以校验必须在这里补。
    //
    // 两道校验都有真实事故背景（2026-10-21 实测）：
    // ① 备份里少了 `dpiN_rgb` 键时，旧代码 `unwrap_or([0,0,0])` 会把颜色
    //    静默刷成黑色 —— 和文档第 5 节那次「能量块填错槽位」是同一类错误：
    //    **缺失的键不该被当成零值写进设备**。现在缺失就沿用设备当前值。
    // ② `dpiN_enc` 完全没有范围校验，写 `dpi1_enc=999` 会变成 50000 DPI
    //    （`decode_dpi(999) = 999*50+50`）。现在超范围直接拒绝整份还原。
    for i in 0..DPI_STAGES {
        let Some(enc) = b.dpi_enc[i] else { continue };
        if !(proto::DPI_ENC_MIN..=proto::DPI_ENC_MAX).contains(&enc) {
            return Err(format!(
                "DPI 档 {} 的编码 {enc} 超范围（合法 {}..={}，即 {}..={} DPI）。\
                 备份文件可能已损坏，为安全起见整份还原已中止，设备未被修改。",
                i + 1,
                proto::DPI_ENC_MIN,
                proto::DPI_ENC_MAX,
                proto::decode_dpi(proto::DPI_ENC_MIN),
                proto::decode_dpi(proto::DPI_ENC_MAX)
            ));
        }
        // 缺 `dpiN_rgb` 时沿用设备当前颜色，绝不兜底成黑色
        let rgb = match b.dpi_rgb[i] {
            Some(c) => c,
            None => {
                let mut reader = ops::DirectReader;
                let cur = reader.read(dev, rid, true).map_err(|e| e.message)?;
                cur.rgb[i]
            }
        };
        if proto::write_dpi(dev, rid, i, proto::decode_dpi(enc), rgb).is_none() {
            return Err(format!("写 DPI 档 {} 无应答", i + 1));
        }
        log(&format!(
            "DPI 档 {} -> {}  颜色 #{:02x}{:02x}{:02x}",
            i + 1,
            proto::decode_dpi(enc),
            rgb[0],
            rgb[1],
            rgb[2]
        ));
    }

    // ---- 回报率（寄存器 0x04）----
    if let Some(code) = b.poll_code {
        if proto::write_reg(dev, rid, 0x04, &[code]).is_none() {
            return Err("写回报率无应答".into());
        }
        log(&format!("回报率 -> {}", proto::poll_name(code)));
    }

    // ---- 当前档位（寄存器 0x09，1-based）----
    if let Some(stage) = b.active_stage {
        if proto::write_reg(dev, rid, 0x09, &[stage]).is_none() {
            return Err("写当前档位无应答".into());
        }
        log(&format!("当前档位 -> {stage}"));
    }

    // ---- 去抖 / 角度吸附 / 角度微调 ----
    if let Some(code) = b.debounce_code {
        if proto::write_reg(dev, rid, 0x05, &[code]).is_none() {
            return Err("写去抖无应答".into());
        }
        log(&format!("去抖 -> {} (code {code})", proto::debounce_name(code)));
    }
    if let Some(snap) = b.angle_snap {
        if proto::write_reg(dev, rid, 0x06, &[snap]).is_none() {
            return Err("写角度吸附无应答".into());
        }
        log(&format!("角度吸附 -> {snap}"));
    }
    if let Some(t) = b.angle_tuning {
        let v = t as i16;
        if proto::write_reg(dev, rid, 0x0B, &[(v & 0xFF) as u8, ((v >> 8) & 0xFF) as u8]).is_none() {
            return Err("写角度微调无应答".into());
        }
        log(&format!("角度微调 -> {v}"));
    }

    // ---- LOD（独立命令族；会重置表面标定）----
    if let Some(code) = b.lod {
        if proto::write_lod(dev, rid, code).is_none() {
            return Err("写 LOD 无应答".into());
        }
        log(&format!("LOD -> {} (code {code})", proto::lod_name(code)));
        log("注意：写 LOD 会把表面标定重置为默认，这是设备行为，无法避免。");
    }

    // ---- 灯光 ----
    //
    // 和上面 DPI 那段同一个道理：备份里缺的键**不能兜底成零值**。
    // 旧代码用 `unwrap_or(50)` / `unwrap_or([0,0,0])` / `unwrap_or(0)`，
    // 于是一份只记了 `light_mode` 的残缺备份会把亮度打到 50、
    // 颜色刷成黑色、方向/随机/速度清零 —— 用户看到的就是「还原备份后灯变黑了」。
    //
    // 现在缺失的字段一律沿用设备当前值。只有 `light_mode` 存在才进这段，
    // 所以「备份里完全没提灯光」仍然是「不动灯光」，语义不变。
    if let Some(mode) = b.light_mode {
        let need_any = b.light_bright.is_none()
            || b.light_rgb.is_none()
            || b.light_dir.is_none()
            || b.light_random.is_none()
            || b.light_speed.is_none();
        // 只在确实缺字段时才去读设备 —— 完整的备份不该多付一帧的代价
        let cur = if need_any {
            let mut reader = ops::DirectReader;
            Some(reader.read(dev, rid, true).map_err(|e| e.message)?)
        } else {
            None
        };
        let bright = match (b.light_bright, &cur) {
            (Some(v), _) => v,
            (None, Some(c)) => c.light_bright,
            (None, None) => unreachable!("need_any 为 true 时 cur 必然为 Some"),
        };
        let rgb = match (b.light_rgb, &cur) {
            (Some(v), _) => v,
            (None, Some(c)) => c.light_rgb,
            (None, None) => unreachable!("need_any 为 true 时 cur 必然为 Some"),
        };
        let dir = match (b.light_dir, &cur) {
            (Some(v), _) => v,
            (None, Some(c)) => c.light_dir,
            (None, None) => unreachable!("need_any 为 true 时 cur 必然为 Some"),
        };
        let rnd = match (b.light_random, &cur) {
            (Some(v), _) => v,
            (None, Some(c)) => c.light_random,
            (None, None) => unreachable!("need_any 为 true 时 cur 必然为 Some"),
        };
        let spd = match (b.light_speed, &cur) {
            (Some(v), _) => v,
            (None, Some(c)) => c.light_speed,
            (None, None) => unreachable!("need_any 为 true 时 cur 必然为 Some"),
        };
        if proto::write_light(dev, rid, mode, bright, rgb, dir, rnd, spd).is_none() {
            return Err("写灯光无应答".into());
        }
        log(&format!(
            "灯光 -> {} (code {mode}) 亮度 {bright} 颜色 #{:02x}{:02x}{:02x}",
            proto::light_mode_name(mode),
            rgb[0],
            rgb[1],
            rgb[2]
        ));
    }

    // ---- 8 个按键 ----
    for i in 0..BUTTON_SLOTS {
        let Some(dst) = b.buttons[i] else { continue };
        if proto::write_button(dev, rid, i, dst).is_none() {
            return Err(format!("写按键槽 {i} 无应答"));
        }
        log(&format!("按键 {i} ({}) -> 0x{dst:04X}", BUTTON_NAME[i]));
    }

    // ---- 能量块：最后单独重写，并读回确认 ----
    let (Some(sleep), Some(low), Some(full)) = (b.poweroff_code, b.low_warn, b.full_effect) else {
        if save {
            proto::commit(dev, rid);
            log("已 commit（能量块被跳过，见上面的警告）。");
        }
        return Ok(());
    };

    if proto::write_energy(dev, rid, sleep, low, full).is_none() {
        return Err("写能量块无应答".into());
    }
    log(&format!(
        "能量块 -> 休眠 {} (code {sleep}) 低电量 {low}% 充满效果 {full}",
        proto::sleep_mode_name(sleep)
    ));

    if save {
        proto::commit(dev, rid);
        log("已 commit（0x50 0x03），设置已进 flash。");
    } else {
        log("未 commit（默认不写 flash）。确认无误后可再加 --save。");
    }

    // 读回断言（restore 必须做，这是文档第 5 节事故的直接教训）
    ops::sleep_ms(ops::SETTLE_MS);
    let verified = proto::read_energy_only(dev, rid)?;
    let ok = verified.sleep_mode == sleep && verified.low_warn == low && verified.full_effect == full;
    log(&format!(
        "VERIFY       能量块读回：休眠 {} / 低电量 {}% / 充满效果 {} → {}",
        proto::sleep_mode_name(verified.sleep_mode),
        verified.low_warn,
        verified.full_effect,
        if ok { "YES" } else { "NO" }
    ));
    if !ok {
        return Err("能量块读回与备份不一致".into());
    }
    Ok(())
}

/// `restore <file>` —— 按备份逐项写回，**能量块最后单独重写并断言**。
pub fn cmd_restore(ctx: &mut Ctx, dev: &mut HidDevice, path: &str, save: bool) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("读文件 {path} 失败：{e}"))?;
    let b = backup::parse(&text)?;

    println!("== 还原 {path} ==");
    println!("识别到 {} / {} 个键", b.keys_found, backup::BACKUP_KEYS.len());
    if backup::energy_missing(&b) {
        eprintln!("警告：这份备份缺少能量块（poweroff_code / low_warn / full_effect）。");
        eprintln!("      文档第 5 节的事故正是因此而起——commit 后休眠时间会被写坏。");
        eprintln!("      本次还原**跳过**能量块，请手动确认设备休眠设置。");
    }

    let rid = ctx.rid();
    let result = {
        let mut log = |line: &str| println!("  {line}");
        apply_backup(dev, rid, &b, save, &mut log)
    };

    ctx.invalidate();
    result
}

// ==================== setter ====================
//
// 每个 setter 只做三件事：解析参数 → 调 ops::* → 打报告。
// 六步循环、错误分类、读回断言全部在 ops 里，这里不许重复实现。

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
    let r = ops::set_dpi(dev, rid, stage, dpi, rgb, keep, save, ctx, &mut log)?;
    print_report(&r);
    // CLI 要非零退出码：读回不一致在这里升级成错误（GUI 则直接看报告字段）
    r.into_result()
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
    let r = ops::set_poll(dev, rid, hz, keep, save, ctx, &mut log)?;
    print_report(&r);
    // CLI 要非零退出码：读回不一致在这里升级成错误（GUI 则直接看报告字段）
    r.into_result()
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
    let r = ops::set_stage(dev, rid, stage, keep, save, ctx, &mut log)?;
    print_report(&r);
    // CLI 要非零退出码：读回不一致在这里升级成错误（GUI 则直接看报告字段）
    r.into_result()
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
    let r = ops::set_debounce(dev, rid, code, keep, save, ctx, &mut log)?;
    print_report(&r);
    // CLI 要非零退出码：读回不一致在这里升级成错误（GUI 则直接看报告字段）
    r.into_result()
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
    let r = ops::set_snap(dev, rid, snap, keep, save, ctx, &mut log)?;
    print_report(&r);
    // CLI 要非零退出码：读回不一致在这里升级成错误（GUI 则直接看报告字段）
    r.into_result()
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
    let r = ops::set_tuning(dev, rid, value, keep, save, ctx, &mut log)?;
    print_report(&r);
    // CLI 要非零退出码：读回不一致在这里升级成错误（GUI 则直接看报告字段）
    r.into_result()
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
    let r = ops::set_lod(dev, rid, code, keep, save, ctx, &mut log)?;
    print_report(&r);
    // CLI 要非零退出码：读回不一致在这里升级成错误（GUI 则直接看报告字段）
    r.into_result()
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
    let r = ops::set_light(
        dev, rid, mode, bright, rgb, None, None, None, keep, save, ctx, &mut log,
    )?;
    print_report(&r);
    // CLI 要非零退出码：读回不一致在这里升级成错误（GUI 则直接看报告字段）
    r.into_result()
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
    let r = ops::set_button(dev, rid, slot, dst, keep, save, confirmed, ctx, &mut log)?;
    print_report(&r);
    // CLI 要非零退出码：读回不一致在这里升级成错误（GUI 则直接看报告字段）
    r.into_result()
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
    let r = ops::set_sleep(dev, rid, code, keep, save, ctx, &mut log)?;
    print_report(&r);
    // CLI 要非零退出码：读回不一致在这里升级成错误（GUI 则直接看报告字段）
    r.into_result()
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
    let r = ops::set_low_batt(dev, rid, percent, keep, save, ctx, &mut log)?;
    print_report(&r);
    // CLI 要非零退出码：读回不一致在这里升级成错误（GUI 则直接看报告字段）
    r.into_result()
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
    let r = ops::set_full_effect(dev, rid, on, keep, save, ctx, &mut log)?;
    print_report(&r);
    // CLI 要非零退出码：读回不一致在这里升级成错误（GUI 则直接看报告字段）
    r.into_result()
}

/// `save` —— 单独提交当前 RAM 里的设置进 flash。
pub fn cmd_save(ctx: &mut Ctx, dev: &mut HidDevice) -> Result<(), String> {
    println!("== save（提交进 flash）==");
    // 提交前先把当前状态打印出来，让用户知道到底把什么写进去了
    let st = ctx.state(dev)?;
    proto::print_state(&st);

    let rid = ctx.rid();
    let r = match proto::commit(dev, rid) {
        Some(m) => {
            if is_nak(&m) {
                Err("commit 被设备 NAK".to_string())
            } else {
                // 回全零是正常的（不是 NAK）
                println!(
                    "\n已提交（0x50 0x03）。回帧 {} 字节，全零属正常。",
                    m.len()
                );
                println!("VERIFY       commit=OK");
                Ok(())
            }
        }
        None => Err("commit 无应答（鼠标休眠中？）".to_string()),
    };
    ctx.invalidate();
    r
}

/// `estimate` —— 读当前电量 + 历史，打印续航/充满估算。
///
/// 顺手按 C# 版的规则记一条采样（`record_sample` 内部会做去重和裁剪），
/// 这样命令行跑几次就等于在攒数据，不用等 GUI。
pub fn cmd_estimate(ctx: &mut Ctx, dev: &mut HidDevice) -> Result<(), String> {
    println!("== estimate（电量历史与估算）==");
    let st = ctx.state(dev)?;
    let mode = link_mode_code(ctx.link);
    let rows = crate::history::read_rows();

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

    let now_sec = crate::snapshot::now_unix() as i64;
    let est = crate::history::predict_from_rows(&rows, st.power, st.charging == 1, mode, now_sec);
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

    // 记一条采样。失败只提示，不影响估算结果的输出。
    match crate::history::record_sample(st.power, st.charging == 1, mode) {
        Ok(true) => println!("  已追加     1 条采样到历史"),
        Ok(false) => println!("  未追加     距上一条太近（去重规则：变化 60s / 心跳 600s）"),
        Err(e) => println!("  写历史失败 {e}"),
    }
    Ok(())
}

/// 连接模式转成历史文件里的数字：1=有线 2=2.4G（与 C# 版 `LinkMode` 一致）。
fn link_mode_code(link: &Link) -> i32 {
    if link.mode.starts_with("Wired") {
        1
    } else {
        2
    }
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
/// 写有六步安全循环，绕过去就没有自动还原保护了。
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
            proto::hex_dump(&m);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_hints_cover_all_kinds() {
        // 每个类别都要有自己的提示，不能落到空字符串
        for k in [
            ErrorKind::Invalid,
            ErrorKind::Asleep,
            ErrorKind::Rejected,
            ErrorKind::Mismatch,
            ErrorKind::Other,
        ] {
            assert!(!error_hint(k).is_empty(), "{k:?} 没有提示文案");
        }
    }
}
