//! 连接状态三态。
//!
//! 以前只有「读到帧 / 没读到帧」两种，于是两种完全不同的故障被压成同一句话：
//! 接收器插着、鼠标睡着了（动一下就能唤醒），和接收器根本没插（要用户去翻包）。
//! 用户看到「鼠标休眠中？」但鼠标就在手边时会以为软件坏了。

/// 设备连接状态。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinkState {
    /// 读到了设备的帧：链路通，设备在线。
    Connected,
    /// 接收器在（能读到接收器序列号），但设备不应答 —— 通常是设备休眠或关机。
    ReceiverOnly,
    /// 接收器都不在：没插、插在别的口但没枚举出来，或者设备完全离线。
    Absent,
}

pub fn classify(frames_read: usize, dongle_sn: &str) -> LinkState {
    if frames_read > 0 {
        return LinkState::Connected;
    }
    if dongle_sn.trim().is_empty() {
        LinkState::Absent
    } else {
        LinkState::ReceiverOnly
    }
}

/// 给界面/提示用的一句话。
pub fn describe(state: LinkState) -> &'static str {
    match state {
        LinkState::Connected => "已连接",
        LinkState::ReceiverOnly => "接收器在位，设备未响应",
        LinkState::Absent => "未检测到设备",
    }
}
