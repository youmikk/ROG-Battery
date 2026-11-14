//! 界面要展示的数据（视图模型）。
//!
//! 只有数据、没有任何绘制或事件代码 —— 由 `gui.rs` 组装好后交给
//! `ui::pages` 画。放在这里而不是 `pages.rs` 里，是为了让事件层
//! （`gui.rs`）依赖「数据定义」而不是反过来依赖「画图模块」。

#[derive(Clone)]
pub struct Data<'a> {
    pub selected: Option<&'a crate::device::DeviceReading>,
    /// 设备状态。`None` = 还没读到（首帧 / 后台线程正在读）。
    pub state: Option<&'a crate::proto::State>,
    /// 设备身份（型号、SN、固件）。
    pub ident: Option<&'a crate::proto::Identity>,
    pub link_mode: &'a str,
    /// 状态行文字（「已连接 · 84%」「鼠标休眠中，移动一下即可唤醒」…）。
    pub status: String,
    /// 曲线数据。`None` = 还没读完第一遍（该画「正在读取」而不是「样本不足」）。
    pub plot: Option<&'a crate::chart::Plot>,
    /// 续航估算。
    pub estimate: &'a crate::history::Estimate,
    /// 历史样本条数（`plot` 里可能包含断档前的旧点，这里给的是有效条数）。
    pub rows: usize,
    /// 本地设置（托盘开关、刷新间隔…）。
    pub cfg: &'a crate::settings::Settings,
    /// 开机自启的当前状态文字。
    pub autostart_text: &'a str,
    /// 开机自启是否打开（开关状态，不能靠文案反推）。
    pub autostart_on: bool,
    /// 当前是否为 MSIX 版，是否可显示 Windows 原生通知设置。
    pub native_notifications_supported: bool,
    /// 版本号（侧栏底部）。
    pub version: &'a str,
    /// 首页设备卡片墙使用的读数列表。
    pub readings: &'a [crate::device::DeviceReading],
}
