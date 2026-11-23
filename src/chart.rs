use crate::history::{HistoryRow, HIST_GAP_BREAK_SEC};
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

/// A measured point. Time stays normalized until the final pixel mapping so
/// dense samples are not rounded onto a fixed-width intermediate canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pt {
    pub fx: f32,
    pub level: u8,
    pub approximate: bool,
}

/// A point on a measured line or coarse battery step, without smoothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurvePt {
    pub fx: f32,
    pub level: f32,
}

/// One uninterrupted interval with a consistent precision and charging state.
#[derive(Clone, Debug)]
pub struct Segment {
    /// Indices of the original samples in `Plot::points`.
    pub samples: Range<usize>,
    pub curve: Vec<CurvePt>,
    /// True only when both the state and its charging flag are known.
    pub charging: bool,
    pub approximate: bool,
}

/// Geometry is rebuilt only when this snapshot changes or the viewport moves.
#[derive(Clone, Debug, Default)]
pub struct Plot {
    pub points: Vec<Pt>,
    pub segments: Vec<Segment>,
    pub first_ts: i64,
    pub last_ts: i64,
    /// Allocators may reuse an old Plot's address after a background refresh.
    /// A revision also invalidates cached geometry in that case.
    pub revision: u64,
}

impl Plot {
    pub fn enough(&self) -> bool {
        self.points.len() >= 2
    }

    pub fn has_approximate(&self) -> bool {
        self.points.iter().any(|point| point.approximate)
    }

    pub fn has_charging_interval(&self) -> bool {
        self.segments.iter().any(|segment| {
            segment.charging && segment.samples.len() >= 2
        })
    }
}

static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

/// Keep measured values unchanged. Explicit history breaks, long absences,
/// precision changes and power/connection transitions each start a new segment.
/// An isolated sample remains visible as a dot; it cannot bridge an interval
/// for which no continuous measurements exist.
pub fn build(rows: &[HistoryRow]) -> Plot {
    let mut ordered = rows.to_vec();
    ordered.sort_by_key(|row| row.ts);

    let mut samples: Vec<HistoryRow> = Vec::with_capacity(ordered.len());
    let mut invalid_gap = false;
    for mut row in ordered {
        if row.level > 100 {
            invalid_gap = true;
            continue;
        }
        row.break_before |= invalid_gap;
        invalid_gap = false;
        if let Some(previous) = samples.last_mut().filter(|previous| previous.ts == row.ts) {
            // Same-second updates have no horizontal duration. Retain the last
            // value and any explicit boundary recorded earlier in that second.
            row.break_before |= previous.break_before;
            *previous = row;
        } else {
            samples.push(row);
        }
    }
    let Some(first) = samples.first() else {
        return Plot::default();
    };
    let first_ts = first.ts;
    let last_ts = samples.last().map_or(first_ts, |row| row.ts);
    let span = last_ts.saturating_sub(first_ts).max(1) as f64;
    let points: Vec<Pt> = samples.iter().map(|row| Pt {
        fx: (row.ts.saturating_sub(first_ts) as f64 / span) as f32,
        level: row.level,
        approximate: row.approximate,
    }).collect();

    let mut segments = Vec::new();
    let mut start = 0;
    for index in 1..samples.len() {
        if starts_segment(&samples[index - 1], &samples[index]) {
            segments.push(build_segment(&points, start..index, &samples[start]));
            start = index;
        }
    }
    segments.push(build_segment(&points, start..samples.len(), &samples[start]));

    Plot {
        points,
        segments,
        first_ts,
        last_ts,
        revision: NEXT_REVISION.fetch_add(1, Ordering::Relaxed),
    }
}

fn starts_segment(previous: &HistoryRow, current: &HistoryRow) -> bool {
    current.break_before
        || current.ts.saturating_sub(previous.ts) > HIST_GAP_BREAK_SEC
        || current.approximate != previous.approximate
        || current.charging_known != previous.charging_known
        || (current.charging_known && current.charging != previous.charging)
        || current.mode != previous.mode
}

fn build_segment(points: &[Pt], samples: Range<usize>, first: &HistoryRow) -> Segment {
    let measured = &points[samples.clone()];
    let mut curve: Vec<CurvePt> = Vec::with_capacity(measured.len() * 2);
    for (index, point) in measured.iter().enumerate() {
        if first.approximate && index > 0 {
            let previous = &measured[index - 1];
            if previous.level != point.level {
                // A level report cannot establish a smooth decline between
                // thresholds. Keep its previous plateau until the next report.
                curve.push(CurvePt { fx: point.fx, level: previous.level as f32 });
            }
        }
        curve.push(CurvePt { fx: point.fx, level: point.level as f32 });
    }
    Segment {
        samples,
        curve,
        charging: first.charging_known && first.charging,
        approximate: first.approximate,
    }
}

pub const NOT_ENOUGH_TEXT: &str = "历史样本不足（至少需要 2 个采样点）";
pub const LOADING_TEXT: &str = "正在读取历史样本 ……";

#[cfg(test)]
mod tests {
    use super::*;

    fn row(ts: i64, level: u8) -> HistoryRow {
        HistoryRow::new(ts, level, false, 2)
    }

    #[test]
    fn sorts_measured_values_without_rounding_or_smoothing_them() {
        let plot = build(&[row(3600, 70), row(0, 90), row(120, 89), row(60, 89)]);
        assert_eq!((plot.first_ts, plot.last_ts), (0, 3600));
        assert_eq!(plot.points.iter().map(|point| point.level).collect::<Vec<_>>(),
            vec![90, 89, 89, 70]);
        assert_eq!(plot.segments.len(), 1);
        let curve = &plot.segments[0].curve;
        assert_eq!(curve.len(), 4);
        assert_eq!(curve.first().unwrap().fx, 0.0);
        assert_eq!(curve.last().unwrap().fx, 1.0);
        assert!((curve[1].fx - 60.0 / 3600.0).abs() < f32::EPSILON);
        assert!(curve.windows(2).all(|pair| pair[1].fx > pair[0].fx));
        assert_eq!(curve.iter().map(|point| point.level).collect::<Vec<_>>(),
            vec![90.0, 89.0, 89.0, 70.0]);
    }

    #[test]
    fn gaps_and_explicit_breaks_leave_disconnected_intervals() {
        let resumed_at = 60 + HIST_GAP_BREAK_SEC + 1;
        let mut explicit = row(resumed_at + 60, 78);
        explicit.break_before = true;
        let plot = build(&[row(0, 90), row(60, 89), row(resumed_at, 80), explicit]);
        assert_eq!(plot.segments.iter().map(|segment| segment.samples.clone())
            .collect::<Vec<_>>(), vec![0..2, 2..3, 3..4]);
        assert_eq!(plot.segments[1].curve.len(), 1);
        assert_eq!(plot.segments[2].curve.len(), 1);
    }

    #[test]
    fn coarse_samples_are_steps_and_never_blend_with_precise_samples() {
        let mut coarse = [row(60, 80), row(120, 60), row(180, 60)];
        for sample in &mut coarse { sample.approximate = true; }
        let plot = build(&[row(0, 92), coarse[0], coarse[1], coarse[2], row(240, 57)]);
        assert!(plot.has_approximate());
        assert_eq!(plot.segments.iter().map(|segment| segment.samples.clone())
            .collect::<Vec<_>>(), vec![0..1, 1..4, 4..5]);
        let segment = &plot.segments[1];
        assert!(segment.approximate);
        assert_eq!(segment.curve.iter().map(|point| (point.fx, point.level))
            .collect::<Vec<_>>(), vec![(0.25, 80.0), (0.5, 80.0), (0.5, 60.0), (0.75, 60.0)]);
    }

    #[test]
    fn only_confirmed_continuous_charging_gets_a_charging_interval() {
        let mut first_unknown = row(0, 50);
        first_unknown.charging = true;
        first_unknown.charging_known = false;
        let mut second_unknown = first_unknown;
        second_unknown.ts = 60;
        let mut charging = row(120, 51);
        charging.charging = true;
        let mut later = charging;
        later.ts = 180;
        later.level = 52;
        let unknown = build(&[first_unknown, second_unknown]);
        assert!(!unknown.has_charging_interval());
        let plot = build(&[first_unknown, second_unknown, charging, later, row(240, 52)]);
        assert_eq!(plot.segments.iter().map(|segment| segment.samples.clone())
            .collect::<Vec<_>>(), vec![0..2, 2..4, 4..5]);
        assert!(plot.has_charging_interval());
        assert!(!plot.segments[0].charging);
        assert!(plot.segments[1].charging);
        assert!(!plot.segments[2].charging);
    }

    #[test]
    fn connection_changes_do_not_join_the_history() {
        let mut wired = row(120, 80);
        wired.mode = 1;
        let plot = build(&[row(0, 80), row(60, 80), wired]);
        assert_eq!(plot.segments.iter().map(|segment| segment.samples.clone())
            .collect::<Vec<_>>(), vec![0..2, 2..3]);
    }

    #[test]
    fn same_second_updates_keep_the_latest_value_and_boundary() {
        let mut boundary = row(60, 80);
        boundary.break_before = true;
        let plot = build(&[row(0, 90), boundary, row(60, 79), row(120, 78)]);
        assert_eq!(plot.points.iter().map(|point| point.level).collect::<Vec<_>>(),
            vec![90, 79, 78]);
        assert_eq!(plot.segments.iter().map(|segment| segment.samples.clone())
            .collect::<Vec<_>>(), vec![0..1, 1..3]);
    }

    #[test]
    fn invalid_samples_cannot_connect_valid_neighbors() {
        let plot = build(&[row(0, 80), row(60, 255), row(120, 78), row(180, 77)]);
        assert_eq!(plot.points.len(), 3);
        assert_eq!(plot.segments.iter().map(|segment| segment.samples.clone())
            .collect::<Vec<_>>(), vec![0..1, 1..3]);
        assert!(build(&[]).points.is_empty());
        assert!(build(&[row(0, 255)]).points.is_empty());
        assert!(!build(&[row(0, 80)]).enough());
    }

    #[test]
    fn rebuilds_invalidate_geometry_even_for_the_same_allocation() {
        let mut plot = build(&[row(0, 80), row(60, 79)]);
        let address = &plot as *const Plot;
        let revision = plot.revision;
        plot = build(&[row(0, 20), row(60, 19)]);
        assert_eq!(&plot as *const Plot, address);
        assert_ne!(plot.revision, revision);
        assert_eq!(plot.clone().revision, plot.revision);
    }
}
