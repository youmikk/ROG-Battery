//! 命令执行层：把界面上的一次操作变成一次设备/文件动作。
//!
//! 这里**不重复任何校验或写入逻辑**，全部转调 `crate::ops` —— 界面一旦自己
//! 写一遍写入代码，它就必然和 CLI 那份漂移。
//!
//! 从 `gui.rs` 搬出来的原因：这层只依赖 `ops` / `hid` / `backup` / `settings`，
//! 不碰任何窗口、绘制或消息循环，却原来寄生在 2800 行的 GUI 文件里。

use crate::gui::{autostart_hint, Cmd, CmdResult};
use crate::hid::{self, HidDevice};
use crate::{settings, snapshot};

/// 散在 12 个调用点各写一次字面量 `false`，看不出来哪处错了；
/// 起名 `KEEP_AFTER_WRITE` 之后，读代码的人不用回去翻 `ops.rs` 就能判断。
///
/// - [`KEEP_AFTER_WRITE`] = `true`：保留新值（这是 GUI 唯一正确的取值）
/// - [`SAVE_TO_FLASH`] = `true`：提交到 flash，重启后仍在
pub(crate) const KEEP_AFTER_WRITE: bool = true;
pub(crate) const SAVE_TO_FLASH: bool = true;

// 在函数体里用短名字，避免每个调用点都排成两行。
pub(crate) const KEEP: bool = KEEP_AFTER_WRITE;
pub(crate) const SAVE: bool = SAVE_TO_FLASH;

/// 后台线程执行一条命令。
///
/// 这里**不重复任何校验或写入逻辑** —— 全部转调 [`crate::ops`]。
/// GUI 一旦自己写一遍写入代码，它就必然和 CLI 那份漂移，
/// 而漂移的那次就是用户设备被改坏的那次。
///
/// `--keep` 恒为 `true`：GUI 的写是「用户要这个值」，不是「试一下再还原」。
/// `--save` 恒为 `true`：界面上改完点应用，用户预期是重启后还记得。
///
/// **这两个参数的语义见 `ops::cycle` 六步**，别望文生义：
/// - `keep = false` → 步骤 4 会**把旧值写回去**（CLI 的默认行为，
///   用来试一个值、看完读回断言就撤销，跑完不留痕迹）；
/// - `keep = true`  → 立刻 return，保留新值。
///
/// 这里原来写的是 `false, true`，注释还写着「不是试一下再还原」——
/// 恰好把结论和取值写反了。后果是**界面上每一个设置都被程序自己撤销**：
/// 用户拖完亮度，滑块弹回原位、灯效不变，而底部浮层还诚实地报
/// 「已还原=是」。这个 bug 在 CLI 上永远看不出来（CLI 本来就要 `--keep`），
/// 只有真机点一遍才会暴露。
pub(crate) fn run_cmd(cmd: &Cmd) -> CmdResult {
    let label = cmd.label();

    // 自启这条路**完全不碰 HID**，所以必须在 `open_link` 之前分叉 ——
    // 否则用户只是想勾一个注册表项，却要先等设备往返，休眠时还要等满超时。
    if let Cmd::AutoStart(on) = cmd {
        return run_autostart(*on);
    }

    // 备份/还原要读写文件，但同样先开链路。
    let mut log = |_line: &str| {};

    let result = (|| -> Result<crate::ops::ApplyReport, crate::ops::OpsError> {
        let api = crate::win32::HidApi::load().map_err(crate::ops::OpsError::other)?;
        let link = hid::open_link(&api).map_err(crate::ops::OpsError::other)?;
        let mut dev = HidDevice::open(&link.iface).map_err(crate::ops::OpsError::other)?;
        let rid = link.report_id;
        let mut reader = crate::ops::DirectReader;

        match cmd {
            Cmd::Poll(hz) => {
                crate::ops::set_poll(&mut dev, rid, *hz, KEEP, SAVE, &mut reader, &mut log)
            }
            Cmd::Dpi { stage, dpi, rgb } => crate::ops::set_dpi(
                &mut dev,
                rid,
                *stage,
                *dpi,
                *rgb,
                KEEP,
                SAVE,
                &mut reader,
                &mut log,
            ),
            Cmd::Stage(s) => {
                crate::ops::set_stage(&mut dev, rid, *s, KEEP, SAVE, &mut reader, &mut log)
            }
            Cmd::Debounce(c) => {
                crate::ops::set_debounce(&mut dev, rid, *c, KEEP, SAVE, &mut reader, &mut log)
            }
            Cmd::Snap(v) => {
                crate::ops::set_snap(&mut dev, rid, *v, KEEP, SAVE, &mut reader, &mut log)
            }
            Cmd::Tuning(v) => {
                crate::ops::set_tuning(&mut dev, rid, *v, KEEP, SAVE, &mut reader, &mut log)
            }
            Cmd::Lod(v) => {
                crate::ops::set_lod(&mut dev, rid, *v, KEEP, SAVE, &mut reader, &mut log)
            }
            Cmd::Light {
                mode,
                bright,
                rgb,
                dir,
                random,
                speed,
            } => crate::ops::set_light(
                &mut dev,
                rid,
                *mode,
                *bright,
                *rgb,
                *dir,
                *random,
                *speed,
                KEEP,
                SAVE,
                &mut reader,
                &mut log,
            ),
            Cmd::Button {
                slot,
                dst,
                confirmed,
            } => crate::ops::set_button(
                &mut dev,
                rid,
                *slot,
                *dst,
                KEEP,
                SAVE,
                *confirmed,
                &mut reader,
                &mut log,
            ),
            Cmd::Sleep(c) => {
                crate::ops::set_sleep(&mut dev, rid, *c, KEEP, SAVE, &mut reader, &mut log)
            }
            Cmd::LowBatt(p) => {
                crate::ops::set_low_batt(&mut dev, rid, *p, KEEP, SAVE, &mut reader, &mut log)
            }
            Cmd::FullEffect(v) => {
                crate::ops::set_full_effect(&mut dev, rid, *v, KEEP, SAVE, &mut reader, &mut log)
            }
            // 上面已提前返回 / 由下面的文件分支处理。写这两支是为了让 match
            // 保持穷尽 —— 将来加新 `Cmd` 变体时编译器会在这里提醒。
            Cmd::AutoStart(_) => unreachable!("AutoStart 在 open_link 之前已分叉"),
            Cmd::Backup(_) | Cmd::Restore(_) => unreachable!("文件命令单独处理"),
        }
    })();

    match result {
        Ok(report) => {
            // `into_result` 会把「写进去了但读回不一致」也变成 Err ——
            // 那种情况比 NAK 更该让人看见，不能当成成功。
            //
            // 先留一份 warning（`into_result` 会消费 report），
            // 因为「不一致」时 warning 往往是解释原因的那句话。
            let warning = report.warning.clone();
            let summary = report.summary();
            match report.into_result() {
                Ok(_) => CmdResult {
                    label,
                    ok: Some(match warning {
                        Some(w) => format!("{summary}；{w}"),
                        None => summary,
                    }),
                    err: None,
                    hint: "",
                },
                Err(e) => {
                    // 先取 hint 再动 `e.message`：`OpsError` 是结构体，
                    // 移走 `message` 之后 `e.kind()` 就借不到了（E0382）。
                    let hint = crate::cli::error_hint(e.kind());
                    CmdResult {
                        label,
                        ok: None,
                        err: Some(match warning {
                            Some(w) => format!("{}（{w}）", e.message),
                            None => e.message,
                        }),
                        hint,
                    }
                }
            }
        }
        Err(e) => {
            let hint = crate::cli::error_hint(e.kind());
            CmdResult {
                label,
                ok: None,
                err: Some(e.message),
                hint,
            }
        }
    }
}

/// 备份 / 还原。需要文件系统，也需要设备。
pub(crate) fn run_file_cmd(cmd: &Cmd) -> CmdResult {
    let label = cmd.label();
    let (path, save) = match cmd {
        Cmd::Backup(p) => (p, true),
        Cmd::Restore(p) => (p, false),
        _ => unreachable!(),
    };

    let fail = |msg: String, hint: &'static str| CmdResult {
        label: label.clone(),
        ok: None,
        err: Some(msg),
        hint,
    };

    let api = match crate::win32::HidApi::load() {
        Ok(a) => a,
        Err(e) => return fail(e, "请确认接收器已插好"),
    };
    let link = match hid::open_link(&api) {
        Ok(l) => l,
        Err(e) => return fail(e, "请确认接收器已插好"),
    };
    let mut dev = match HidDevice::open(&link.iface) {
        Ok(d) => d,
        Err(e) => return fail(e, "设备被别的程序占用了？"),
    };

    if save {
        let out = snapshot::fetch(&mut dev, &link.all_interfaces, link.report_id, &link.mode);
        if out.snapshot.frames.is_empty() {
            return fail("读不到设备状态，无法备份".into(), "");
        }
        let text = crate::backup::render(
            &out.snapshot.state,
            &link.mode,
            link.report_id,
            &settings::timestamp_for_humans(),
        );
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(e) = crate::paths::ensure_dir(parent) {
                    return fail(e, "");
                }
            }
        }
        match std::fs::write(path, text.as_bytes()) {
            Ok(_) => CmdResult {
                label,
                ok: Some(format!(
                    "已写入 {}（{} 个键）",
                    path.display(),
                    crate::backup::BACKUP_KEYS.len()
                )),
                err: None,
                hint: "",
            },
            Err(e) => fail(format!("写文件失败：{e}"), ""),
        }
    } else {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => return fail(format!("读不了 {}：{e}", path.display()), ""),
        };
        let b = match crate::backup::parse(&text) {
            Ok(b) => b,
            Err(e) => return fail(format!("备份文件解析失败：{e}"), ""),
        };
        if b.keys_found == 0 {
            return fail("备份文件里没有任何已知键".into(), "");
        }
        // 值解析失败/超范围的键必须让用户看到：静默跳过会让「已还原」变成谎话。
        if !b.rejected.is_empty() {
            return fail(
                format!(
                    "备份里有 {} 项值不可用（{}），未还原任何设置",
                    b.rejected.len(),
                    b.rejected.join("；")
                ),
                "",
            );
        }
        let mut lines: Vec<String> = Vec::new();
        match crate::cli::apply_backup(&mut dev, link.report_id, &b, true, &mut |l| {
            lines.push(l.to_string())
        }) {
            Ok(()) => CmdResult {
                label,
                ok: Some(format!("已还原 {} 个键", b.keys_found)),
                err: None,
                hint: "",
            },
            Err(e) => fail(e, ""),
        }
    }
}

/// 执行自启开关。
///
/// 三种「没成功」要区分开，不能都说成失败：
/// - `DisabledByUser`：系统记着用户拒绝过，`RequestEnableAsync` **静默失败**
/// - `DisabledByPolicy`：组策略禁止，程序无解
/// - 其它（`Unknown` / `Disabled`）：读不到或系统没接受
fn run_autostart(on: bool) -> CmdResult {
    let label = format!("开机自启 {}", if on { "开启" } else { "关闭" });
    let outcome = if on {
        crate::autostart::enable()
    } else {
        crate::autostart::disable()
    };
    match outcome {
        Ok(out) => {
            let hint = autostart_hint(on, out.state, out.path);
            // `enabled` 是回读后的真实状态，不是「写成功就代表生效」。
            if out.enabled == on {
                CmdResult {
                    label,
                    ok: Some(hint),
                    err: None,
                    hint: "",
                }
            } else {
                CmdResult {
                    label,
                    ok: None,
                    err: Some(hint),
                    hint: "",
                }
            }
        }
        Err(e) => CmdResult {
            label,
            ok: None,
            err: Some(e),
            hint: "",
        },
    }
}
