//! 电量历史曲线的**数据层**：把 `history.tsv` 的行映射成横向坐标。
//!
//! 绘制在 `src/ui/pages.rs` 的 `draw_chart` 里。本模块只负责一件纯计算的事：
//! 「行数据 → 采样点」，不碰 Win32、不碰文件，所以能在单测里喂任意数据点，
//! 不用去污染用户真实的 `history.tsv`。
//!
//! # 为什么这里只管 X
//!
//! 纵向映射归页面：界面按**数据自己的极值**自适应 Y 轴（手上只有 82~89% 时
//! 要把这段铺满整个绘图区，否则曲线贴成一条直线），而这个窗口只能由绘制方
//! 决定 —— 它还画刻度线、充电底带、面积渐变，必须和曲线用同一套算术。
//!
//! 早先这里也存过 `Pt.y`（按 0~100% 固定映射到 `AREA_H`），但自绘界面接管后
//! `pages.rs` 一律用 `pt.level` 现算，`Pt.y` 一次都没被读过 —— 两套纵向算术
//! 并存的结果是改一个忘一个。2026-11-01 直接删掉，只留 X。
//!
//! 早先这里还有一套 GDI 渲染（`paint` / `fill` / `draw_text` + 一堆 `COLORREF`
//! 常量和 `DT_*` 位），是 nwg `Tab` 子类化时代的产物，2026-10-22 已删。

use crate::history::HistoryRow;

// ==================== 版面 ====================

/// X 轴的坐标系基准：`build` 把时间线性映射到 `AREA_X .. AREA_X + AREA_W`。
///
/// 自绘界面按**逻辑像素**排版，这两个数没有物理意义，只作为比例基准 ——
/// `pages.rs` 用 `(pt.x - AREA_X) / AREA_W` 把点搬进卡片的实际宽度。
pub const AREA_X: i32 = 24;
pub const AREA_W: i32 = 660;

// ==================== 行数据 → 采样点 ====================

/// 一个采样点。纵向位置由绘制方按自己的 Y 轴窗口算。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pt {
    /// 已映射好的横向像素坐标（`AREA_X .. AREA_X + AREA_W`）。
    pub x: i32,
    /// 原始电量，绘制方据此算 Y。
    pub level: u8,
    /// 这个采样点当时是否在充电 —— 用于给折线段换颜色、画充电底带。
    pub charging: bool,
}

/// 一次完整的折线数据。
#[derive(Clone, Debug, Default)]
pub struct Plot {
    pub points: Vec<Pt>,
    pub first_ts: i64,
    pub last_ts: i64,
}

impl Plot {
    /// 少于 2 个点画不出折线 —— 调用方要显示「历史样本不足」而不是留白。
    pub fn enough(&self) -> bool {
        self.points.len() >= 2
    }
}

/// 把历史行映射成采样点。
///
/// X 轴范围取**数据自己的时间跨度**，不是写死 48 小时：历史文件保留 48 小时，
/// 但手上只有 8.75 小时数据时，按 48 小时铺满会让 7 个点全挤在左边五分之一里，
/// 看起来像「点挤在一起」的 bug。跨度写在顶部说明行里，用户仍然知道这是多久的数据。
///
/// `first_ts` / `last_ts` 用 `min` / `max` 而不是首尾元素：`read_rows()` 虽然会排序，
/// 但这个函数不该依赖调用方的排序 —— 用极值后即使输入乱序，X 也不会反向。
pub fn build(rows: &[HistoryRow]) -> Plot {
    if rows.is_empty() {
        return Plot::default();
    }

    let first_ts = rows.iter().map(|r| r.ts).min().unwrap_or(0);
    let last_ts = rows.iter().map(|r| r.ts).max().unwrap_or(0);
    // 所有采样点在同一秒时跨度为 0，除法会炸 —— 钳到 1 秒，
    // 结果是所有点落在同一条竖线上（诚实反映「数据只覆盖一瞬」）。
    let span = (last_ts - first_ts).max(1);

    let mut points = Vec::with_capacity(rows.len());
    for r in rows {
        let fx = (r.ts - first_ts) as f64 / span as f64;
        let x = AREA_X + (fx * (AREA_W - 1) as f64).round() as i32;
        points.push(Pt {
            x,
            level: r.level,
            charging: r.charging,
        });
    }

    Plot {
        points,
        first_ts,
        last_ts,
    }
}

/// 数据不够时显示的文字。抽出来是为了让单测钉住这句文案 ——
/// 它是用户唯一能看到的解释，不能被悄悄改掉。
pub const NOT_ENOUGH_TEXT: &str = "历史样本不足（至少需要 2 个采样点）";

/// 后台线程还没读完第一遍时显示的文字。
///
/// 必须和 [`NOT_ENOUGH_TEXT`] 分开：读盘冷启动 712 ms，这段时间里说
/// 「样本不足」是**假话**（文件里明明有 7 条），用户会以为功能坏了。
pub const LOADING_TEXT: &str = "正在读取历史样本 ……";

// ==================== 单测 ====================

#[cfg(test)]
mod tests {
    use super::*;

    /// 用户机器上真实的 7 条采样（`%LOCALAPPDATA%\rog-battery\history.tsv`）。
    ///
    /// **在测试里构造行数据，不碰真实文件** —— 往 `history.tsv` 塞伪造数据会污染
    /// 用户的续航估算（`predict_from_rows` 读的是同一个文件）。
    fn real_rows() -> Vec<HistoryRow> {
        [
            (1792568899i64, 89u8),
            (1792569212, 88),
            (1792576904, 87),
            (1792593756, 84),
            (1792595845, 84),
            (1792596612, 84),
            (1792600403, 82),
        ]
        .iter()
        .map(|&(ts, level)| HistoryRow::new(ts, level, false, 2))
        .collect()
    }

    #[test]
    fn empty_and_single_are_not_enough() {
        assert!(!build(&[]).enough());
        assert!(!build(&real_rows()[..1]).enough());
        // 1 个点也能算出坐标（不能 panic），但不够画折线
        let one = build(&real_rows()[..1]);
        assert_eq!(one.points.len(), 1);
        assert_eq!(one.points[0].x, AREA_X);
    }

    /// 真实数据：7 个点、X 严格递增、首尾贴满整个横向区间。
    /// 这条测试就是「别把 X 轴映射写错」的钉子：写错时点会挤成一堆或变成平线。
    #[test]
    fn real_seven_points_span_the_whole_band() {
        let plot = build(&real_rows());
        assert!(plot.enough());
        assert_eq!(plot.points.len(), 7);

        assert_eq!(plot.points[0].x, AREA_X);
        assert_eq!(plot.points[6].x, AREA_X + AREA_W - 1);

        for w in plot.points.windows(2) {
            assert!(w[1].x > w[0].x, "X 必须严格递增：{w:?}");
        }

        // 电平原样带过去，绘制方靠它算 Y
        assert_eq!(plot.points[0].level, 89);
        assert_eq!(plot.points[6].level, 82);
    }

    /// 所有点必须落在横向区间内，越界就是被裁剪或被别的控件挡住。
    #[test]
    fn all_points_stay_inside_the_band() {
        let mut rows = real_rows();
        rows.push(HistoryRow::new(1792601000, 0, true, 2));
        rows.push(HistoryRow::new(1792602000, 100, false, 2));
        let plot = build(&rows);
        for p in &plot.points {
            assert!(
                p.x >= AREA_X && p.x < AREA_X + AREA_W,
                "x={} 越界 [{}, {})",
                p.x,
                AREA_X,
                AREA_X + AREA_W
            );
        }
    }

    /// 所有点在同一秒时跨度为 0，不能除零 panic。
    #[test]
    fn identical_timestamps_do_not_panic() {
        let rows: Vec<HistoryRow> = (0..4)
            .map(|i| HistoryRow::new(1_700_000_000, 50 + i, false, 2))
            .collect();
        let plot = build(&rows);
        assert_eq!(plot.points.len(), 4);
        assert!(plot.points.iter().all(|p| p.x == AREA_X));
    }

    /// 输入乱序也能画：X 轴范围用 min/max，不用首尾元素。
    #[test]
    fn unsorted_input_keeps_x_monotonic_by_timestamp() {
        let mut rows = real_rows();
        rows.reverse();
        let plot = build(&rows);
        let mut by_ts: Vec<(i64, i32)> = rows.iter().zip(plot.points.iter()).map(|(r, p)| (r.ts, p.x)).collect();
        by_ts.sort_by_key(|(ts, _)| *ts);
        for w in by_ts.windows(2) {
            assert!(w[1].1 >= w[0].1, "时间越大 X 不能更小：{w:?}");
        }
    }

    /// 顶部那行「近 48 小时 · N 个采样点」由 `pages.rs` 直接拼（它还要决定
    /// 放不放下、什么时候换成「正在读取」），所以这里只钉住两件数据层的事：
    /// 点数要对、起止时间要来自极值而不是首尾元素。
    #[test]
    fn span_comes_from_extremes_not_endpoints() {
        let plot = build(&real_rows());
        assert_eq!(plot.points.len(), 7);
        assert_eq!(plot.first_ts, 1792568899);
        assert_eq!(plot.last_ts, 1792600403);

        // 倒序输入：`first_ts` / `last_ts` 必须仍然是 min / max，
        // 否则 X 轴会反向（曲线左右翻转）。
        let mut rows = real_rows();
        rows.reverse();
        let rev = build(&rows);
        assert_eq!(rev.first_ts, plot.first_ts);
        assert_eq!(rev.last_ts, plot.last_ts);
    }

    #[test]
    fn not_enough_text_is_the_documented_sentence() {
        assert_eq!(NOT_ENOUGH_TEXT, "历史样本不足（至少需要 2 个采样点）");
    }
}
