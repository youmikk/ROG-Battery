//! 界面要展示的数据（视图模型）。
//!
//! 只有数据、没有任何绘制或事件代码 —— 由 `gui.rs` 组装好后交给
//! `ui::pages` 画。放在这里而不是 `pages.rs` 里，是为了让事件层
//! （`gui.rs`）依赖「数据定义」而不是反过来依赖「画图模块」。

#[derive(Clone)]
pub struct Data<'a> {
    /// 设备状态。`None` = 还没读到（首帧 / 后台线程正在读）。
    pub state: Option<&'a crate::proto::State>,
    /// 设备身份（型号、SN、固件）。
    pub ident: Option<&'a crate::proto::Identity>,
    /// 实测链路模式（`"Wired"` / `"Wireless24G"`，空串 = 还没读到）。
    ///
    /// 不能用 `ident.dongle_sn` 是否为空来判有线无线 —— 2.4G 下那个字段
    /// 也可能读成空串，会把无线误显示成有线。
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
    /// 版本号（侧栏底部）。
    pub version: &'a str,
    /// 帧耗时（侧栏底部「19 帧 · 588 ms」）。
    pub frames: usize,
    pub elapsed_ms: u128,
}

#[cfg(test)]
mod tests {
    /// `ui::pages::Data` 必须仍然是同一个类型（只是再导出）。否则 gui.rs 传进去
    /// 的结构和 pages 读的就不是一个东西，而这个错误只会在改名时静默出现。
    #[test]
    fn pages_re_exports_the_same_data_type() {
        fn same(d: crate::ui::pages::Data<'_>) -> crate::ui::viewdata::Data<'_> {
            d
        }
        let _ = same as fn(crate::ui::pages::Data<'_>) -> crate::ui::viewdata::Data<'_>;
    }
}
