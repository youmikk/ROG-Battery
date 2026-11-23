
use super::format::{dur_short, dur_text, reason_text, Why};
use super::{decide_charge, ChargeState, HistoryRow, HIST_GAP_BREAK_SEC};

const MAX_PREDICT_HOURS: f64 = 1000.0;

/// 超过这个小时数就只说「N 天以上」。
///
/// 早期外推误差极大（2% 的跌幅推剩下 97%，真实值可能差三倍），显示到「小时」
/// 是假装精确。粗粒度既诚实又不吓人。
pub(super) const COARSE_HOURS: f64 = 240.0;

/// 估算结果。`known == false` 时 `text` 说明原因，**不要假装有结论**。
#[derive(Clone, Debug, Default)]
pub struct Estimate {
    pub known: bool,
    /// 可直接显示的整句。
    pub text: String,
    /// 正 = 在涨，负 = 在掉（%/小时）。
    pub percent_per_hour: f64,
    pub samples: usize,
    pub span_minutes: f64,
    /// 剩余（放电）或充满（充电）所需小时数；仅 `known == true` 时有意义。
    pub hours: f64,
    /// 这条结论是「沿用」来的：距上一次成功估算过了多少秒。0 = 本次新鲜算出的。
    pub stale_seconds: i64,
    /// Coarse battery steps support only rounded, approximate durations.
    pub approximate: bool,
}

impl Estimate {
    pub fn localized_short_text(&self) -> String {
        if !crate::i18n::english() {
            return self.short_text();
        }
        if !self.known {
            return crate::i18n::translate(if self.text == "已充满" {
                "已充满"
            } else {
                "估算中…"
            })
            .into();
        }
        if self.approximate { return self.coarse_duration(); }
        let d = crate::i18n::duration(self.hours);
        if self.percent_per_hour > 0.0 {
            crate::trf!("约 {d}后充满", d = d)
        } else {
            crate::trf!("约 {d}", d = d)
        }
    }

    pub fn localized_text(&self) -> String {
        if self.known && self.approximate {
            let duration = self.coarse_duration();
            return if crate::i18n::english() {
                if self.percent_per_hour > 0.0 { format!("Time to full: {duration}") }
                else { format!("Remaining battery life: {duration}") }
            } else if self.percent_per_hour > 0.0 { format!("预计充满时间：{duration}") }
            else { format!("预计可用时间：{duration}") };
        }
        if !crate::i18n::english() {
            return self.text.clone();
        }
        if self.known {
            let dur = if self.hours >= COARSE_HOURS {
                crate::tr!("10 天以上").to_string()
            } else {
                crate::i18n::duration(self.hours)
            };
            return if self.percent_per_hour > 0.0 {
                crate::trf!("预计 {dur} 后充满", dur = dur)
            } else {
                crate::trf!("预计还能用 {dur}", dur = dur)
            };
        }
        let translated = crate::i18n::translate(&self.text);
        if translated != self.text {
            return translated.into();
        }
        if let Some((head, reason)) = self.text.split_once('：') {
            return format!(
                "{}: {}",
                crate::i18n::translate(head),
                crate::i18n::translate(reason)
            );
        }
        translated.into()
    }

    /// 环下方那行短结论。`text` 是给「明细」用的整句（"预计还能用 3 小时 12 分钟"），
    /// 环下面那格横向只有 ~150px，放整句会挤，所以另出一版紧凑文案。
    pub fn short_text(&self) -> String {
        if !self.known {
            return if self.text == "已充满" {
                "已充满".to_string()
            } else {
                "估算中…".to_string()
            };
        }
        if self.approximate { return self.coarse_duration(); }
        let d = dur_short(self.hours);
        if self.percent_per_hour > 0.0 {
            format!("约 {d}后充满")
        } else {
            format!("约 {d}")
        }
    }

    fn coarse_duration(&self) -> String {
        let hours = self.hours.ceil().max(1.0) as u64;
        if crate::i18n::english() {
            if hours >= 240 { "≈ 10 days+".into() }
            else if hours >= 48 { format!("≈ {} days", hours.div_ceil(24)) }
            else { format!("≈ {hours} h") }
        } else if hours >= 240 { "≈ 10 天以上".into() }
        else if hours >= 48 { format!("≈ {} 天", hours.div_ceil(24)) }
        else { format!("≈ {hours} 小时") }
    }
}

/// `current` is an actual measurement, never a cached value timestamped "now".
/// Every caller supplies only this device's rows; there is no global ETA cache.
pub fn predict_device(rows: &[HistoryRow], current: HistoryRow) -> Estimate {
    let mut observed: Vec<HistoryRow> = rows.iter().copied().filter(|r| r.ts <= current.ts).collect();
    if let Some(last) = observed.last_mut().filter(|last| last.ts == current.ts) {
        let boundary = last.break_before;
        *last = current;
        last.break_before |= boundary;
    } else {
        observed.push(current);
    }
    let now_level = current.level;
    let now_charging = current.charging;
    let now_mode = current.mode;
    // 已充满：没什么可预测的
    if decide_charge(now_level, now_charging, now_mode) == ChargeState::Full {
        return Estimate {
            known: false,
            text: "已充满".to_string(),
            ..Default::default()
        };
    }

    // Use only the current continuous charge/precision interval.
    let phase = &observed[phase_start(&observed, now_charging)..];

    // 同相位样本太少 → 说明刚切换过充电/放电，历史还没攒够。
    // 这时不要说「样本不足」糊弄过去，要告诉用户真实原因。
    if phase.len() < 3 {
        return Estimate {
            samples: phase.len(),
            text: if observed.len() >= 3 {
                reason_text(Why::JustSwitched, now_charging)
            } else {
                reason_text(Why::NotEnough, now_charging)
            },
            ..Default::default()
        };
    }

    // 第三步：方向过滤。放电段里出现回升（或充电段里出现回落）说明中途插过线
    // 或传感器抖动，只保留最近这段干净的。
    let sel = &phase[direction_start(phase, now_charging)..];
    if sel.len() < 3 {
        return Estimate {
            samples: sel.len(),
            text: reason_text(Why::Jumped, now_charging),
            ..Default::default()
        };
    }

    let span = (current.ts - sel[0].ts) as f64 / 60.0;
    let changes = sel.windows(2).filter(|w| w[0].level != w[1].level).count();
    if !current.charging_known && (now_charging || changes < 2 || span < 30.0) {
        return Estimate { text: "充电状态未知，等待稳定放电趋势".into(),
            samples: sel.len(), span_minutes: span, ..Default::default() };
    }
    if current.approximate && (changes < 2 || span < 60.0) {
        return Estimate { text: "仅有粗略电量，等待更多档位变化".into(),
            samples: sel.len(), span_minutes: span, ..Default::default() };
    }
    let mut estimate = predict_from_series(sel, now_level, now_charging, now_mode);
    estimate.approximate = current.approximate;
    if estimate.known && estimate.approximate {
        let duration = estimate.coarse_duration();
        estimate.text = if now_charging { format!("预计充满时间：{duration}") }
            else { format!("预计可用时间：{duration}") };
    }
    estimate
}

/// 纯函数核心：给定已含当前读数的升序样本，算斜率。
fn predict_from_series(
    sel: &[HistoryRow],
    now_level: u8,
    now_charging: bool,
    now_mode: i32,
) -> Estimate {
    if decide_charge(now_level, now_charging, now_mode) == ChargeState::Full {
        return Estimate {
            text: "已充满".to_string(),
            ..Default::default()
        };
    }
    if sel.len() < 3 {
        return Estimate {
            samples: sel.len(),
            text: reason_text(Why::NotEnough, now_charging),
            ..Default::default()
        };
    }

    let x0 = sel[0].ts as f64;
    let x1 = sel[sel.len() - 1].ts as f64;
    let span_minutes = (x1 - x0) / 60.0;
    if span_minutes < 10.0 {
        return Estimate { samples: sel.len(), span_minutes,
            text: reason_text(Why::SpanTooShort, now_charging), ..Default::default() };
    }

    // 用极差判断「到底动过没有」。只比首尾会漏掉「先降后升」那种情形：
    // 那种情况下首尾相等，但数据里明明有变化。
    let mut lo = f64::MAX;
    let mut hi = f64::MIN;
    for r in sel {
        let v = r.level as f64;
        if v < lo {
            lo = v;
        }
        if v > hi {
            hi = v;
        }
    }
    if hi - lo < 1.0 {
        return Estimate {
            samples: sel.len(),
            span_minutes,
            text: reason_text(Why::NoChange, now_charging),
            ..Default::default()
        };
    }

    // 最小二乘：x 用小时，y 用电量百分点
    let n = sel.len() as f64;
    let (mut sx, mut sy, mut sxy, mut sxx) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for r in sel {
        let xh = (r.ts as f64 - x0) / 3600.0;
        let y = r.level as f64;
        sx += xh;
        sy += y;
        sxy += xh * y;
        sxx += xh * xh;
    }
    let den = n * sxx - sx * sx;
    if den.abs() < 1e-9 {
        return Estimate {
            samples: sel.len(),
            span_minutes,
            text: reason_text(Why::SpanTooShort, now_charging),
            ..Default::default()
        };
    }
    let slope = (n * sxy - sx * sy) / den; // %/小时

    if slope.abs() < 0.05 {
        return Estimate {
            samples: sel.len(),
            span_minutes,
            text: reason_text(Why::TooSlow, now_charging),
            ..Default::default()
        };
    }
    // 方向和当前状态相反，说明刚切过充电/放电，样本混了
    if now_charging && slope <= 0.0 {
        return Estimate {
            samples: sel.len(),
            span_minutes,
            text: "预计充满时间：刚插上电，还在观察".to_string(),
            ..Default::default()
        };
    }
    if !now_charging && slope >= 0.0 {
        return Estimate {
            samples: sel.len(),
            span_minutes,
            text: "预计可用时间：刚拔掉线，还在观察".to_string(),
            ..Default::default()
        };
    }

    let level = now_level as f64;
    let hours = if now_charging {
        (100.0 - level) / slope
    } else {
        level / (-slope)
    };
    if hours < 0.0 || hours > MAX_PREDICT_HOURS {
        return Estimate {
            samples: sel.len(),
            span_minutes,
            text: if now_charging {
                "预计充满时间：异常（请忽略）".to_string()
            } else {
                "预计可用时间：异常（请忽略）".to_string()
            },
            ..Default::default()
        };
    }

    let dur = if hours >= COARSE_HOURS {
        "10 天以上".to_string()
    } else {
        dur_text(hours)
    };
    Estimate {
        known: true,
        text: if now_charging {
            format!("预计 {dur} 后充满")
        } else {
            format!("预计还能用 {dur}")
        },
        percent_per_hour: slope,
        samples: sel.len(),
        span_minutes,
        hours,
        stale_seconds: 0,
        approximate: false,
    }
}

/// 「连续同一相位」那一段在 `rows` 里的起始下标（终点恒为 `rows.len()`）。
///
/// 从最新往回扫，扫到相位不同或断档就停 —— 结果必然是一个连续尾段，
/// 所以只返回下标，让调用方用切片，不用复制一份数据。
pub(super) fn phase_start(rows: &[HistoryRow], charging: bool) -> usize {
    let mut start = rows.len();
    let mut prev_t: Option<i64> = None;
    let Some(latest) = rows.last() else { return 0; };
    for r in rows.iter().rev() {
        if r.charging != charging || r.charging_known != latest.charging_known
            || r.approximate != latest.approximate || r.mode != latest.mode {
            break;
        }
        if let Some(pt) = prev_t {
            if pt - r.ts > HIST_GAP_BREAK_SEC {
                break;
            }
        }
        start -= 1;
        prev_t = Some(r.ts);
        if r.break_before { break; }
    }
    start
}

#[cfg(test)]
mod tests {
    use super::*;
    const TS: i64 = 1_800_000_000;
    fn row(seconds: i64, level: u8, charging: bool) -> HistoryRow {
        HistoryRow::new(TS + seconds, level, charging, 0)
    }
    #[test]
    fn forecasts_are_independent_for_devices_and_charge_phases() {
        let discharge = [row(0, 90, false), row(1800, 85, false), row(3600, 80, false)];
        let charge = [row(0, 60, true), row(1800, 70, true), row(3600, 80, true)];
        let a = predict_device(&discharge, discharge[2]);
        let b = predict_device(&charge, charge[2]);
        assert!(a.known && (a.hours - 8.0).abs() < 0.01);
        assert!(b.known && (b.hours - 1.0).abs() < 0.01);
        assert!(!predict_device(&discharge, row(3601, 80, true)).known);
        assert!(!predict_device(&[], row(0, 80, false)).known);
    }
    #[test]
    fn gaps_boundaries_and_precision_changes_discard_old_forecasts() {
        let rows = [row(0, 90, false), row(1800, 85, false), row(3600, 80, false)];
        assert!(!predict_device(&rows, row(12000, 79, false)).known);
        let mut current = row(3601, 80, false); current.break_before = true;
        assert!(!predict_device(&rows, current).known);
        current.break_before = false; current.approximate = true;
        assert!(!predict_device(&rows, current).known);
        current.approximate = false; current.charging_known = false;
        assert!(!predict_device(&rows, current).known);
    }
    #[test]
    fn unknown_charge_requires_an_observed_discharge_trend() {
        let mut rows = [row(0, 90, false), row(1800, 85, false), row(3600, 80, false)];
        for r in &mut rows { r.charging_known = false; }
        assert!(predict_device(&rows, rows[2]).known);
        for (i, r) in rows.iter_mut().enumerate() { r.level = 80 + i as u8 * 5; }
        assert!(!predict_device(&rows, rows[2]).known);
    }
    #[test]
    fn coarse_battery_waits_for_multiple_steps_and_never_claims_minute_precision() {
        let mut rows = [row(0, 75, false), row(1800, 50, false), row(3600, 25, false)];
        for r in &mut rows { r.approximate = true; }
        assert!(!predict_device(&rows[..2], rows[1]).known);
        let estimate = predict_device(&rows, rows[2]);
        assert!(estimate.known && estimate.approximate);
        assert!(estimate.short_text().contains('≈'));
        assert!(!estimate.short_text().contains("分钟"));
        let short = [row(0, 90, false), row(60, 89, false), row(120, 88, false)];
        assert!(!predict_device(&short, short[2]).known);
    }
}

/// 「方向也一致」那一段在 `rows` 里的起始下标（终点恒为 `rows.len()`）。
///
/// 放电样本往回看应单调不降，充电样本相反；碰到第一次反向就截断。
/// 同样是连续尾段，返回下标即可。
pub(super) fn direction_start(rows: &[HistoryRow], charging: bool) -> usize {
    let mut start = rows.len();
    let mut prev: Option<u8> = None;
    for r in rows.iter().rev() {
        if let Some(p) = prev {
            let ok = if charging { r.level <= p } else { r.level >= p };
            if !ok {
                break;
            }
        }
        start -= 1;
        prev = Some(r.level);
    }
    start
}
