//! 电量历史采样 + 续航/充满时间估算。
//!
//! 三层，各自只干一件事：
//! - [`store`]：路径解析、跨进程互斥、解析与写入（碰磁盘）
//! - [`predict`]：最小二乘斜率 + 连续采样和相位过滤（纯计算）
//! - [`format`]：把小时数/秒数/算不出来的原因翻成人话（纯文案）
//!
//! 对外仍然是同一个 `crate::history::*` 门面，调用方不用改。

mod format;
mod predict;
mod store;
mod devices;
mod tracker;

pub use format::ago_text;
pub use predict::{predict_device, Estimate};
pub use store::*;
pub use devices::{DeviceHistoryFile, DeviceRows, should_record_device};
pub use tracker::{HistoryView, Tracker};
