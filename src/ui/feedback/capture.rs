use super::{text, Action, Controls, R, ResultMessage, Style, Task};

pub(super) const TIMER: usize = 0x43415054;

impl Controls {
    pub(super) fn capture_layout(&self) -> f32 {
        let status = crate::capture::status();
        let mut y = 0.0;
        self.label(text("记录所选接口的现有通信；如果接收器连接多个外设，会包含该接口上所有槽位的数据。不额外发送命令，一般键鼠输入接口不能采集。",
            "Records existing traffic on the selected interface, including all receiver slots if it serves multiple peripherals. No extra commands. General keyboard/mouse input is excluded."),
            25.0, y, 610.0, 72.0, 12.0, false, true);
        y += 80.0;
        self.label(text("原始数据可能包含设备标识或输入信息，不保证匿名。只保存在本机，不会随反馈上传；分享前请检查 raw.jsonl。",
            "Raw bytes may contain identifiers or input data and are not anonymous. Saved locally, never attached to feedback. Review raw.jsonl before sharing."),
            25.0, y, 610.0, 68.0, 12.0, false, false);
        y += 78.0;
        if !status.started {
            self.button(Action::CaptureStart, text("开始本地采集", "Start local capture"), "",
                R::new(25.0, y, 285.0, 36.0), Style::Card, false);
            y += 50.0;
        } else {
            self.label(format!("{} · {} {} · {}s / 300s",
                if status.running { text("采集中", "Capturing") } else { text("已停止", "Stopped") },
                status.frames, text("次传输", "transfers"), status.elapsed_seconds),
                25.0, y, 610.0, 24.0, 12.0, true, false);
            y += 34.0;
            if status.running {
                self.label(text("每完成一步点击对应按钮，可跳过或重复。等待一次电量刷新后再做下一步。",
                    "Click after each action. Skip or repeat as needed; wait for a battery refresh before the next action."),
                    25.0, y, 610.0, 44.0, 11.0, false, true);
                y += 52.0;
                for (step, (zh, en)) in crate::capture::STEPS.iter().enumerate() {
                    self.button(Action::CaptureMark(step), format!("{}. {}", step + 1, text(zh, en)), "",
                        R::new(25.0, y, 610.0, 34.0), Style::Card, false);
                    y += 42.0;
                }
                self.button(Action::CaptureStop, text("停止并预览", "Stop and review"), "",
                    R::new(25.0, y, 285.0, 36.0), Style::Card, false);
            } else {
                let saving = self.state.borrow().task == Some(Task::CaptureExport);
                self.button(Action::CaptureSave, if saving { text("正在保存…", "Saving…") }
                    else { text("保存原始包和摘要", "Save raw data and summary") }, "",
                    R::new(25.0, y, 300.0, 36.0), Style::Card, false);
                self.button(Action::CaptureClear, text("清除内存记录", "Clear memory"), "",
                    R::new(340.0, y, 295.0, 36.0), Style::Card, false);
            }
            y += 50.0;
            if self.capture_folder.borrow().is_some() {
                self.button(Action::CaptureFolder, text("打开保存文件夹", "Open saved folder"), "",
                    R::new(25.0, y, 300.0, 36.0), Style::Card, false);
                y += 48.0;
            }
            self.label(text("最近 6 条预览（长报文已省略，完整内容在保存文件中）", "Last 6 records (long frames shortened; full data in saved file)"),
                25.0, y, 610.0, 38.0, 11.0, true, false);
            y += 46.0;
            for line in status.preview {
                self.label(line, 25.0, y, 610.0, 65.0, 10.0, false, true);
                y += 73.0;
            }
            let summary = status.summary.lines().take(8).collect::<Vec<_>>().join("\n");
            self.label(summary, 25.0, y, 610.0, 200.0, 10.0, false, true);
            y += 210.0;
        }
        self.label(text("最多 5 分钟、1024 条或 512 KiB 记录。关闭窗口或返回会停止。没有报文并不代表不支持；未识别设备请先提交普通反馈。",
            "Limited to 5 minutes, 1024 records or 512 KiB. Closing or going back stops capture. No frames does not mean unsupported; submit regular feedback for unrecognized devices."),
            25.0, y, 610.0, 68.0, 11.0, false, true);
        y + 80.0
    }

    pub(super) fn stop_capture_timer(&self) {
        crate::capture::stop();
        unsafe { crate::win32::KillTimer(self.window.handle.hwnd().unwrap() as _, TIMER); }
    }

    pub(super) fn capture_action(&self, action: Action) {
        match action {
            Action::CaptureStart => {
                let interface = {
                    let state = self.state.borrow();
                    state.selected.and_then(|index| state.devices.get(index))
                        .and_then(|device| device.capture_interface.clone())
                };
                if let Some(interface) = interface {
                    if let Err(error) = crate::capture::start(&interface) { self.error(error); return; }
                    let timer = unsafe { crate::win32::SetTimer(self.window.handle.hwnd().unwrap() as _, TIMER, 1000, 0) };
                    if timer == 0 {
                        crate::capture::stop();
                        self.error(text("无法启动采集计时器。", "Could not start capture timer."));
                        return;
                    }
                }
            }
            Action::CaptureMark(step) => crate::capture::mark(step),
            Action::CaptureStop => self.stop_capture_timer(),
            Action::CaptureClear => {
                self.stop_capture_timer();
                crate::capture::clear();
                *self.capture_folder.borrow_mut() = None;
            }
            Action::CaptureSave => {
                self.stop_capture_timer();
                if let Some(export) = crate::capture::export() {
                    self.work(Task::CaptureExport, move || ResultMessage::CaptureSaved(export.save()));
                    return;
                }
            }
            Action::CaptureFolder => {
                let folder = self.capture_folder.borrow().clone();
                if let Some(folder) = folder {
                    if let Err(error) = crate::dynlight::shell_open(&folder.to_string_lossy()) { self.error(error); return; }
                }
            }
            _ => return,
        }
        self.rebuild();
    }
}
