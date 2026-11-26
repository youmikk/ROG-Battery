use super::{text, Action, Controls, Model, R, ResultMessage, Style, Task};
use base64::{engine::general_purpose::STANDARD, Engine};

pub(super) const TIMER: usize = 0x43415054;

impl Controls {
    pub(super) fn capture_form_layout(&self, state: &Model, mut y: f32) -> f32 {
        let available = state.selected.and_then(|index| state.devices.get(index))
            .is_some_and(|device| device.capture_interface.is_some());
        let attached = state.capture.is_some();
        let status = crate::capture::status();
        self.surface(R::new(25.0, y, 610.0, if attached { 105.0 } else { 75.0 }), true);
        self.label(text("设备通信采集", "Device traffic capture"), 39.0, y + 10.0, 410.0, 22.0, 12.0, true, false);
        self.label(if attached { text("已采集，提交反馈时可一并上传", "Captured; ready to upload with feedback") }
            else if status.started && status.data_frames == 0 { text("本次未采集到通信数据，可重新采集", "No traffic was captured; try again") }
            else if status.started { text("采集记录尚未附加，请查看结果", "Capture not attached; review the result") }
            else if available { text("记录充电、开关机等操作，帮助排查问题", "Record charging and power changes to help diagnose issues") }
            else { text("当前设备暂无可采集接口", "No capture interface is available for this device") },
            39.0, y + 37.0, 420.0, 28.0, 11.0, false, true);
        self.button(Action::Capture, if attached { text("查看记录", "View capture") }
            else if status.started { text("查看结果", "View result") } else { text("开始采集", "Capture") },
            "", R::new(480.0, y + 15.0, 138.0, 34.0), Style::Card, false);
        if attached {
            self.label(text("附带采集记录 · raw.jsonl", "Include capture · raw.jsonl"),
                39.0, y + 72.0, 510.0, 23.0, 11.0, false, false);
            self.button(Action::ToggleCapture, text("附带采集记录", "Include capture"), "",
                R::new(589.0, y + 74.0, 33.0, 19.0), Style::Toggle(state.attach_capture), false);
        }
        y += if attached { 117.0 } else { 87.0 };
        y
    }

    pub(super) fn finish_capture(&self) {
        self.stop_capture_timer();
        let path = {
            let state = self.state.borrow();
            state.selected.and_then(|index| state.devices.get(index))
                .and_then(|device| device.capture_interface.as_ref().map(|interface| interface.path.clone()))
        };
        let Some(raw) = path.and_then(|path| crate::capture::feedback_data(&path)) else { return; };
        match crate::feedback::client::capture(raw.as_bytes()) {
            Ok(capture) => {
                let mut state = self.state.borrow_mut();
                if state.capture.as_ref() != Some(&capture) {
                    state.capture = Some(capture);
                    state.edit_draft();
                }
            }
            Err(error) => self.state.borrow_mut().error = error,
        }
    }

    pub(super) fn capture_layout(&self) -> f32 {
        let status = crate::capture::status();
        let restored = if status.started { None } else { self.state.borrow().capture.clone() };
        let mut y = 0.0;
        self.label(text("记录软件读取设备时的通信，帮助排查电量问题。同一接收器下其他外设的通信也可能被记录。",
            "Record traffic while the app reads the device to diagnose battery issues. Other peripherals on the same receiver may also be recorded."),
            25.0, y, 610.0, 52.0, 12.0, false, true);
        y += 60.0;
        self.label(text("有数据时默认随反馈上传到公开的 GitHub 议题，无需登录。文件可能包含设备标识或输入信息，可在提交前查看并关闭附带。",
            "Captured data is included with feedback on a public GitHub issue, without signing in. It may contain device identifiers or input information; review or exclude it before submitting."),
            25.0, y, 610.0, 68.0, 12.0, false, false);
        y += 78.0;
        if !status.started && restored.is_none() {
            self.button(Action::CaptureStart, text("开始本地采集", "Start local capture"), "",
                R::new(25.0, y, 285.0, 36.0), Style::Card, false);
            y += 50.0;
        } else {
            self.label(if restored.is_some() { text("已恢复采集附件 · raw.jsonl", "Restored capture attachment · raw.jsonl").into() }
                else { format!("{} · {} {} · {}s / 300s",
                if status.running { text("采集中", "Capturing") } else { text("已停止", "Stopped") },
                status.frames, text("次传输", "transfers"), status.elapsed_seconds) },
                25.0, y, 610.0, 24.0, 12.0, true, false);
            y += 34.0;
            if restored.is_none() && status.data_frames == 0 {
                self.label(if status.running { text("等待设备通信。请保持设备唤醒，等采集到报文后再停止。",
                    "Waiting for device traffic. Keep the device awake until data is captured.") }
                    else { text("本次未采集到通信数据，暂无可上传附件。可重新采集或直接提交设备反馈。",
                        "No traffic was captured, so there is no attachment to upload. Try again or submit device feedback.") },
                    25.0, y, 610.0, 44.0, 11.0, false, true);
                y += 52.0;
            }
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
                if restored.is_none() && status.data_frames == 0 {
                    self.button(Action::CaptureStart, text("重新采集", "Try capture again"), "",
                        R::new(25.0, y, 610.0, 36.0), Style::Card, false);
                    y += 50.0;
                }
                let saving = self.state.borrow().task == Some(Task::CaptureExport);
                self.button(Action::CaptureSave, if saving { text("正在保存…", "Saving…") }
                    else { text("另存到本机", "Save a local copy") }, "",
                    R::new(25.0, y, 300.0, 36.0), Style::Card, false);
                self.button(Action::CaptureClear, text("清除采集记录", "Clear capture"), "",
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
            let preview = restored.as_ref().and_then(|capture| STANDARD.decode(&capture.data_base64).ok())
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .map(|raw| raw.lines().rev().take(6).collect::<Vec<_>>().into_iter().rev()
                    .map(|line| line.chars().take(180).collect::<String>()).collect::<Vec<_>>())
                .unwrap_or(status.preview);
            for line in preview {
                self.label(line, 25.0, y, 610.0, 65.0, 10.0, false, true);
                y += 73.0;
            }
            let summary = status.summary.lines().take(8).collect::<Vec<_>>().join("\n");
            self.label(summary, 25.0, y, 610.0, 200.0, 10.0, false, true);
            y += 210.0;
        }
        self.label(text("最多 5 分钟、1024 条或 512 KiB 记录。关闭窗口或返回会停止。未采集到记录时，可先提交设备反馈。",
            "Up to 5 minutes, 1024 records or 512 KiB. Closing or going back stops capture. If no data is captured, submit device feedback first."),
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
                    *self.capture_folder.borrow_mut() = None;
                    let mut state = self.state.borrow_mut();
                    state.capture = None;
                    state.attach_capture = true;
                    state.edit_draft();
                    drop(state);
                    let timer = unsafe { crate::win32::SetTimer(self.window.handle.hwnd().unwrap() as _, TIMER, 1000, 0) };
                    if timer == 0 {
                        crate::capture::stop();
                        self.error(text("无法启动采集计时器。", "Could not start capture timer."));
                        return;
                    }
                    (self.request_capture_refresh)();
                }
            }
            Action::CaptureMark(step) => crate::capture::mark(step),
            Action::CaptureStop => self.finish_capture(),
            Action::CaptureClear => {
                self.stop_capture_timer();
                crate::capture::clear();
                *self.capture_folder.borrow_mut() = None;
                let mut state = self.state.borrow_mut();
                state.capture = None;
                state.edit_draft();
            }
            Action::CaptureSave => {
                self.finish_capture();
                let export = crate::capture::export().or_else(|| self.state.borrow().capture.as_ref()
                    .and_then(|capture| STANDARD.decode(&capture.data_base64).ok())
                    .and_then(|bytes| String::from_utf8(bytes).ok()).and_then(crate::capture::Export::from_raw));
                if let Some(export) = export {
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
