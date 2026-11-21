
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

/// 曲线上的一个点：横向是 0..=1 的比例，纵向是**平滑后**的电量。
///
/// 平滑（移动平均 + Catmull-Rom）本来在绘制方每帧现算，充电时 60fps 下
/// 每秒要算 120 次、每次 2 万个点。数据 30 秒才更新一次，所以挪到这里
/// 由 `build` 算一遍，绘制方只做像素映射。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurvePt {
    /// 横向比例（0 = 区间左端，1 = 右端），按 `AREA_X / AREA_W` 归一。
    pub fx: f32,
    /// 平滑后的电量（可能是小数，纵向映射仍由绘制方按自己的 Y 窗口算）。
    pub level: f32,
}

/// 一次完整的折线数据。
#[derive(Clone, Debug, Default)]
pub struct Plot {
    pub points: Vec<Pt>,
    /// 已平滑、待映射的曲线点。`points.len() < 3` 时就是原始点的拷贝。
    pub curve: Vec<CurvePt>,
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

    let curve = build_curve(&points);

    Plot {
        points,
        curve,
        first_ts,
        last_ts,
    }
}

// ==================== 平滑 ====================

/// 曲线上限：`AREA_W` 是 660 逻辑像素，两点/像素已经远超肉眼能分辨的密度。
/// 2000 个采样 × 10 段插值 = 2 万点，是一条 660px 宽的线负担 30 倍冗余。
const MAX_CURVE_PTS: usize = (AREA_W as usize) * 2;

/// 尾部移动平均 + Catmull-Rom 插值，产出归一化曲线。
fn build_curve(points: &[Pt]) -> Vec<CurvePt> {
    let area_w = AREA_W as f32;
    let raw: Vec<(f32, f32)> = points
        .iter()
        .map(|p| {
            (
                (p.x - AREA_X) as f32 / area_w,
                p.level as f32,
            )
        })
        .collect();
    let soft = soften(&raw, 5);
    // 段数自适应：点多时降到 1，避免总点数随采样数线性膨胀。
    let seg = (MAX_CURVE_PTS / raw.len().max(1)).clamp(1, 10);
    smooth_curve(&soft, seg)
        .into_iter()
        .map(|(fx, level)| CurvePt { fx, level })
        .collect()
}

/// 窗口移动平均：把 1% 量化的台阶磨成缓坡。
///
/// 首尾各留一半窗口不参与（saturating 边界），末点强制保留原值 ——
/// 它要跟「最新值」那个圆点标记重合，被平均掉就错位了。
fn soften(pts: &[(f32, f32)], win: usize) -> Vec<(f32, f32)> {
    if win < 3 || pts.len() < win {
        return pts.to_vec();
    }
    let h = win / 2;
    let n = pts.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let lo = i.saturating_sub(h);
        let hi = (i + h).min(n - 1);
        let mut sum = 0.0f32;
        let mut cnt = 0.0f32;
        for j in lo..=hi {
            sum += pts[j].1;
            cnt += 1.0;
        }
        out.push((pts[i].0, sum / cnt));
    }
    out[n - 1] = pts[n - 1];
    out
}

/// Catmull-Rom：只对 y 插值，x 线性。
///
/// 采样间隔可能极不均匀（休眠两小时才一个点）。x 也插值会让曲线在密集处打结、
/// 稀疏处回折，所以 x 一律线性；y 用样条并夹在数据自身极值内，避免过冲出界。
fn smooth_curve(pts: &[(f32, f32)], seg: usize) -> Vec<(f32, f32)> {
    if pts.len() < 3 || seg < 2 {
        return pts.to_vec();
    }
    let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
    for (_, y) in pts {
        min_y = min_y.min(*y);
        max_y = max_y.max(*y);
    }
    let n = pts.len();
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(n * seg + 1);
    for i in 0..n - 1 {
        let p0 = pts[i.saturating_sub(1)];
        let p1 = pts[i];
        let p2 = pts[i + 1];
        let p3 = pts[(i + 2).min(n - 1)];
        for s in 0..seg {
            let t = s as f32 / seg as f32;
            let t2 = t * t;
            let t3 = t2 * t;
            let y = 0.5
                * (2.0 * p1.1
                    + (p2.1 - p0.1) * t
                    + (2.0 * p0.1 - 5.0 * p1.1 + 4.0 * p2.1 - p3.1) * t2
                    + (3.0 * p1.1 - p0.1 - 3.0 * p2.1 + p3.1) * t3);
            out.push((p1.0 + (p2.0 - p1.0) * t, y.clamp(min_y, max_y)));
        }
    }
    out.push(*pts.last().unwrap());
    out
}

pub const NOT_ENOUGH_TEXT: &str = "历史样本不足（至少需要 2 个采样点）";

/// 后台线程还没读完第一遍时显示的文字。
///
/// 必须和 [`NOT_ENOUGH_TEXT`] 分开：读盘冷启动 712 ms，这段时间里说
/// 「样本不足」是**假话**（文件里明明有 7 条），用户会以为功能坏了。
pub const LOADING_TEXT: &str = "正在读取历史样本 ……";
