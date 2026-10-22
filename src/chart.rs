//! 电量历史曲线的**数据层**：把 `history.tsv` 的行映射成像素点。
//!
//! 绘制不在这里 —— 界面已经全部改成 GDI+ 自绘（`src/ui/pages.rs` 的
//! `draw_chart`），本模块只负责「行数据 → 坐标」这一件纯计算的事。
//!
//! 早先这里还有一套 GDI 渲染（`paint` / `fill` / `draw_text` + 一堆
//! `COLORREF` 常量和 `DT_*` 位），是 nwg `Tab` 子类化时代的产物。
//! 自绘界面接管后它们一行都没人调了，2026-10-22 直接删掉 ——
//! 用 `#[allow(dead_code)]` 压住只会让下一个读代码的人以为它还有用。
//!
//! 「行数据 → 像素坐标」全部收在 [`build`] 里，是**纯函数**（不碰 Win32、不碰文件），
//! 所以能在单测里喂任意数据点，不用去污染用户真实的 `history.tsv`。

use crate::history::HistoryRow;
use crate::win32::Rect;

// ==================== 版面 ====================

/// 绘图区在电量页客户坐标里的位置与大小。
///
/// 注意：自绘界面里页面按**逻辑像素**排版，这套常量现在只作为
/// `build` 的坐标系基准（`pages.rs` 把 `pt.x` 从 `AREA_X..AREA_X+AREA_W`
/// 线性搬到卡片里的实际宽度）。数值本身没有物理意义了。
pub const AREA_X: i32 = 24;
pub const AREA_Y: i32 = 186;
pub const AREA_W: i32 = 660;
pub const AREA_H: i32 = 200;

/// 左侧留给 Y 轴刻度、底部留给 X 轴时间、顶部留给说明行。
const PAD_LEFT: i32 = 40;
const PAD_RIGHT: i32 = 12;
const PAD_TOP: i32 = 22;
const PAD_BOTTOM: i32 = 22;

/// 折线真正落笔的矩形（不含刻度和文字）。
pub fn plot_rect() -> Rect {
    Rect::new(
        AREA_X + PAD_LEFT,
        AREA_Y + PAD_TOP,
        AREA_X + AREA_W - PAD_RIGHT,
        AREA_Y + AREA_H - PAD_BOTTOM,
    )
}

/// 电量 → 纵坐标。100% 落在 `rc.top`，0% 落在 `rc.bottom - 1`。
///
/// 单独抽成函数是为了让**刻度线和数据点用同一套算术** ——
/// 各写一遍的话，25% 的刻度线永远和数据里 level=25 的点差一个像素。
pub fn level_to_y(rc: Rect, level: u8) -> i32 {
    let h = rc.height().max(1);
    let clamped = level.min(100) as i32;
    rc.bottom - 1 - (clamped * (h - 1) + 50) / 100
}

// ==================== 行数据 → 像素坐标 ====================

/// 一个采样点落到屏幕上的位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pt {
    pub x: i32,
    pub y: i32,
    /// 原始电量，画点时不直接用（`y` 已经算好了），诊断和单测要看它。
    pub level: u8,
    /// 这个采样点当时是否在充电 —— 用于给折线段换颜色。
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

/// 把历史行映射成像素点。
///
/// X 轴范围取**数据自己的时间跨度**，不是写死 48 小时：历史文件保留 48 小时，
/// 但手上只有 8.75 小时数据时，按 48 小时铺满会让 7 个点全挤在左边五分之一里，
/// 看起来像「点挤在一起」的 bug。跨度写在顶部说明行里，用户仍然知道这是多久的数据。
///
/// `first_ts` / `last_ts` 用 `min` / `max` 而不是首尾元素：`read_rows()` 虽然会排序，
/// 但这个函数不该依赖调用方的排序 —— 用极值后即使输入乱序，X 也不会反向。
pub fn build(rows: &[HistoryRow], plot: Rect) -> Plot {
    if rows.is_empty() {
        return Plot::default();
    }

    let first_ts = rows.iter().map(|r| r.ts).min().unwrap_or(0);
    let last_ts = rows.iter().map(|r| r.ts).max().unwrap_or(0);
    // 所有采样点在同一秒时跨度为 0，除法会炸 —— 钳到 1 秒，
    // 结果是所有点落在同一条竖线上（诚实反映「数据只覆盖一瞬」）。
    let span = (last_ts - first_ts).max(1);

    let w = plot.width().max(1);
    let mut points = Vec::with_capacity(rows.len());
    for r in rows {
        let fx = (r.ts - first_ts) as f64 / span as f64;
        let x = plot.left + (fx * (w - 1) as f64).round() as i32;
        points.push(Pt {
            x,
            y: level_to_y(plot, r.level),
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
        let prc = plot_rect();
        assert!(!build(&[], prc).enough());
        assert!(!build(&real_rows()[..1], prc).enough());
        // 1 个点也能算出坐标（不能 panic），但不够画折线
        let one = build(&real_rows()[..1], prc);
        assert_eq!(one.points.len(), 1);
        assert_eq!(one.points[0].x, prc.left);
    }

    /// 真实数据必须画成「左上 → 右下」的缓降折线，7 个点，X 单调递增。
    /// 这条测试就是「别把 X 轴映射写错」的钉子：写错时点会挤成一堆或变成平线。
    #[test]
    fn real_seven_points_form_a_falling_curve() {
        let prc = plot_rect();
        let plot = build(&real_rows(), prc);
        assert!(plot.enough());
        assert_eq!(plot.points.len(), 7);

        // 起点贴左边界，终点贴右边界（右边界是 `right - 1`，因为闭区间像素）
        assert_eq!(plot.points[0].x, prc.left);
        assert_eq!(plot.points[6].x, prc.right - 1);

        // X 单调递增（时间往前走）
        for w in plot.points.windows(2) {
            assert!(w[1].x > w[0].x, "X 必须严格递增：{:?}", w);
        }

        // 电量 89% → 82%，屏幕上是往下走（y 变大）
        assert_eq!(plot.points[0].level, 89);
        assert_eq!(plot.points[6].level, 82);
        assert!(
            plot.points[6].y > plot.points[0].y,
            "电量下降对应 y 增大：{} → {}",
            plot.points[0].y,
            plot.points[6].y
        );

        // 不能画成平线：y 的极差至少要有几个像素（7 个点跨 7 个百分点）
        let ys: Vec<i32> = plot.points.iter().map(|p| p.y).collect();
        let span = ys.iter().max().unwrap() - ys.iter().min().unwrap();
        assert!(span >= 5, "曲线太扁（y 极差 {span}px），像是 X/Y 映射写错了");
    }

    /// 所有点必须落在绘图矩形内，越界就是裁剪或被别的控件挡住。
    #[test]
    fn all_points_stay_inside_plot_rect() {
        let prc = plot_rect();
        let mut rows = real_rows();
        // 再塞几个极端值：0% 与 100%
        rows.push(HistoryRow::new(1792601000, 0, true, 2));
        rows.push(HistoryRow::new(1792602000, 100, false, 2));
        let plot = build(&rows, prc);
        for p in &plot.points {
            assert!(
                p.x >= prc.left && p.x < prc.right,
                "x={} 越界 [{}, {})",
                p.x,
                prc.left,
                prc.right
            );
            assert!(
                p.y >= prc.top && p.y <= prc.bottom - 1,
                "y={} 越界 [{}, {}]",
                p.y,
                prc.top,
                prc.bottom - 1
            );
        }
    }

    /// 0% / 100% 必须正好落在绘图区的下/上边界，25% 等刻度线与数据点共用同一套算术。
    #[test]
    fn level_maps_to_axis_edges() {
        let prc = plot_rect();
        assert_eq!(level_to_y(prc, 100), prc.top);
        assert_eq!(level_to_y(prc, 0), prc.bottom - 1);
        // 越界的电量被钳住而不是画到框外
        assert_eq!(level_to_y(prc, 200), prc.top);
        // 单调：电量越高，y 越小
        let mut prev = i32::MAX;
        for lv in [0u8, 25, 50, 75, 100] {
            let y = level_to_y(prc, lv);
            assert!(y < prev, "level {lv} 的 y 没有随电量升高而减小");
            prev = y;
        }
    }

    /// 所有点在同一秒时跨度为 0，不能除零 panic。
    #[test]
    fn identical_timestamps_do_not_panic() {
        let prc = plot_rect();
        let rows: Vec<HistoryRow> = (0..4)
            .map(|i| HistoryRow::new(1_700_000_000, 50 + i, false, 2))
            .collect();
        let plot = build(&rows, prc);
        assert_eq!(plot.points.len(), 4);
        assert!(plot.points.iter().all(|p| p.x == prc.left));
    }

    /// 输入乱序也能画：X 轴范围用 min/max，不用首尾元素。
    #[test]
    fn unsorted_input_keeps_x_monotonic_by_timestamp() {
        let prc = plot_rect();
        let mut rows = real_rows();
        rows.reverse();
        let plot = build(&rows, prc);
        let mut by_ts: Vec<(i64, i32)> = Vec::new();
        for (r, p) in rows.iter().zip(plot.points.iter()) {
            by_ts.push((r.ts, p.x));
        }
        by_ts.sort_by_key(|(ts, _)| *ts);
        for w in by_ts.windows(2) {
            assert!(w[1].1 >= w[0].1, "时间越大 X 不能更小：{:?}", w);
        }
    }

    /// 顶部那行「近 48 小时 · N 个采样点」由 `pages.rs` 直接拼（它还要决定
    /// 放不放下、什么时候换成「正在读取」），所以这里只钉住两件数据层的事：
    /// 点数要对、起止时间要来自极值而不是首尾元素。
    #[test]
    fn span_comes_from_extremes_not_endpoints() {
        let plot = build(&real_rows(), plot_rect());
        assert_eq!(plot.points.len(), 7);
        assert_eq!(plot.first_ts, 1792568899);
        assert_eq!(plot.last_ts, 1792600403);

        // 倒序输入：`first_ts` / `last_ts` 必须仍然是 min / max，
        // 否则 X 轴会反向（曲线左右翻转）。
        let mut rows = real_rows();
        rows.reverse();
        let rev = build(&rows, plot_rect());
        assert_eq!(rev.first_ts, plot.first_ts);
        assert_eq!(rev.last_ts, plot.last_ts);
    }

    #[test]
    fn not_enough_text_is_the_documented_sentence() {
        assert_eq!(NOT_ENOUGH_TEXT, "历史样本不足（至少需要 2 个采样点）");
    }
}
