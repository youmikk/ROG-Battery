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

/// 根据「读到几帧」「链路模式」「接收器序列号」判三态。
///
/// 判据只用实测值：`frames_read` 来自这一轮真正收到的报文数，`dongle_sn`
/// 来自设备自报的接收器序列号。**不能**用 `link_mode` 判有无设备 —— 2.4G 下
/// 那个字段也可能读成空串，会把无线误判成有线。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_mean_connected() {
        assert_eq!(classify(17, ""), LinkState::Connected);
        assert_eq!(classify(3, "C00000000001"), LinkState::Connected);
    }

    #[test]
    fn receiver_without_device_is_not_absent() {
        assert_eq!(classify(0, "C00000000001"), LinkState::ReceiverOnly);
        assert_eq!(classify(0, "  "), LinkState::Absent);
        assert_eq!(classify(0, ""), LinkState::Absent);
    }

    #[test]
    fn descriptions_differ_per_state() {
        assert_ne!(describe(LinkState::Connected), describe(LinkState::Absent));
        assert_ne!(
            describe(LinkState::ReceiverOnly),
            describe(LinkState::Absent)
        );
    }
}
