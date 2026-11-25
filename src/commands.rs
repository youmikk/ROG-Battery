//! GUI 命令入口；ROG 设置通过共享核心验证、恢复并按需提交。

use crate::gui::{autostart_hint, Cmd, CmdResult};
use crate::hid::{self, HidDevice};
use crate::{settings, snapshot};

/// GUI 保留验证通过的新值并提交；失败仍由核心尝试恢复原配置。
pub(crate) const KEEP_AFTER_WRITE: bool = true;
pub(crate) const SAVE_TO_FLASH: bool = true;

// 在函数体里用短名字，避免每个调用点都排成两行。
pub(crate) const KEEP: bool = KEEP_AFTER_WRITE;
pub(crate) const SAVE: bool = SAVE_TO_FLASH;

/// 后台执行设备命令，成功结果必须通过核心的完整读回确认。
pub(crate) fn run_cmd(cmd: &Cmd, target: Option<&hid::Link>) -> CmdResult {
    let label = cmd.label();
    let basic_result = match cmd {
        Cmd::BasicDpi { target, dpi } => Some(crate::backends::set_dpi(target, *dpi)),
        Cmd::BasicDpiStage { target, stage, context } => Some(crate::backends::set_dpi_stage(target, *stage, *context)),
        Cmd::BasicDpiEdit { target, stage, context, expected_y, x, y } => Some(crate::backends::edit_dpi_stage(target, *stage, *context, *expected_y, *x, *y)),
        Cmd::BasicDpiPreset { target, stage, context, expected_y, value } => Some(crate::backends::compx::set_preset(target, *stage, *context, *expected_y, *value)),
        Cmd::BasicPollingRate { target, hz } => Some(crate::backends::set_polling_rate(target, *hz)),
        _ => None,
    };
    if let Some(result) = basic_result {
        return match result {
            Ok(()) => CmdResult { label, ok: Some(crate::tr!("已生效").to_string()), err: None, hint: "" },
            Err(error) => CmdResult { label, ok: None, err: Some(crate::i18n::translate(&error).to_string()), hint: "" },
        };
    }

    // 自启这条路**完全不碰 HID**，所以必须在 `open_link` 之前分叉 ——
    // 否则用户只是想勾一个注册表项，却要先等设备往返，休眠时还要等满超时。
    if let Cmd::AutoStart(on) = cmd {
        return run_autostart(*on);
    }

    // 备份/还原要读写文件，但同样先开链路。
    let mut log = |_line: &str| {};

    let result = (|| -> Result<crate::ops::ApplyReport, crate::ops::OpsError> {
        let link = target.ok_or_else(|| {
            crate::ops::OpsError::other(crate::tr!("所选设备已断开，请重新连接").to_string())
        })?;
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
            Cmd::BasicDpi { .. } | Cmd::BasicDpiStage { .. } | Cmd::BasicDpiEdit { .. } | Cmd::BasicDpiPreset { .. }
                | Cmd::BasicPollingRate { .. } => unreachable!("设备设置由品牌适配器处理"),
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
            let warning = report.warning.as_ref().map(|warning| {
                if !crate::i18n::english() { return warning.clone(); }
                match cmd {
                    Cmd::Lod(_) => crate::tr!("写 LOD 会重置表面标定（厂商确认无恢复命令）。").to_string(),
                    Cmd::Sleep(_) => crate::tr!("「永不」休眠会显著增加耗电，设备闲置时不会自动休眠。").to_string(),
                    Cmd::Button { slot, dst: 0, .. } => crate::trf!("「{}」已被禁用，恢复需要重新绑定或还原备份。", crate::i18n::translate(crate::proto::BUTTON_NAME[*slot & 7])),
                    _ => crate::i18n::translate(warning).to_string(),
                }
            });
            let summary = crate::tr!("已生效").to_string();
            match report.into_result() {
                Ok(_) => CmdResult {
                    label,
                    ok: Some(match warning {
                        Some(w) => format!("{summary}: {}", crate::i18n::translate(&w)),
                        None => summary,
                    }),
                    err: None,
                    hint: "",
                },
                Err(e) => {
                    // 先取 hint 再动 `e.message`：`OpsError` 是结构体，
                    // 移走 `message` 之后 `e.kind()` 就借不到了（E0382）。
                    let hint = crate::i18n::operation_hint(e.kind());
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
            let hint = crate::i18n::operation_hint(e.kind());
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
pub(crate) fn run_file_cmd(cmd: &Cmd, target: Option<&hid::Link>) -> CmdResult {
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

    let Some(link) = target else {
        return fail(
            crate::tr!("所选设备已断开").to_string(),
            crate::tr!("请重新连接设备"),
        );
    };
    let mut dev = match HidDevice::open(&link.iface) {
        Ok(d) => d,
        Err(e) => return fail(e, crate::tr!("设备被别的程序占用了？")),
    };

    if save {
        let configuration = match snapshot::read_configuration(&mut dev, link.report_id) {
            Ok(configuration) => configuration,
            Err(error) => return fail(error, ""),
        };
        let text = crate::backup::render(
            &configuration,
            &link.mode,
            link.report_id,
            &settings::timestamp_for_humans(),
        );
        match crate::paths::write_atomic(path, text.as_bytes()) {
            Ok(_) => CmdResult {
                label,
                ok: Some(crate::trf!(
                    "已写入 {}（{} 个键）",
                    path.display(),
                    crate::backup::BACKUP_KEYS.len()
                )),
                err: None,
                hint: "",
            },
            Err(e) => fail(crate::trf!("写文件失败：{e}", e = e), ""),
        }
    } else {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => return fail(crate::trf!("读不了 {}：{e}", path.display(), e = e), ""),
        };
        let b = match crate::backup::parse(&text) {
            Ok(b) => b,
            Err(e) => return fail(crate::trf!("备份文件解析失败：{e}", e = e), ""),
        };
        match crate::ops::restore_backup(&mut dev, link.report_id, &b, true, &mut |_| {}) {
            Ok(report) => CmdResult {
                label,
                ok: Some(match report.warning {
                    Some(warning) => format!("{}：{}", crate::trf!("已还原 {} 个键", b.keys_found), crate::i18n::translate(&warning)),
                    None => crate::trf!("已还原 {} 个键", b.keys_found),
                }),
                err: None,
                hint: "",
            },
            Err(error) => fail(error.message, ""),
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
    let label = crate::trf!(
        "开机自启 {}",
        if on {
            crate::tr!("开启")
        } else {
            crate::tr!("关闭")
        }
    );
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
