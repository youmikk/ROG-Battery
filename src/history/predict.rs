//! 续航 / 充满时间估算：最小二乘斜率 + 相位过滤 + 「沿用」缓存。
//!
//! 纯计算，不碰文件（读盘在 `super`）。`now_sec` 一律由调用方传入，
//! 否则「补当前读数」那步会用到真实时间，和合成样本的时序打架、单测没法写。

use super::format::{dur_short, dur_text, reason_text, Why};
use super::{decide_charge, ChargeState, HistoryRow, HIST_GAP_BREAK_SEC};
use std::sync::Mutex;

/// 外推上限（小时）。
///
/// 原来取 100 小时，本意是不让界面出现「还能用 137 小时」这种没人信的数字。
/// 但真机实测：这只鼠标 1 小时 48 分才掉 1%（≈0.55 %/h），按 97% 外推就是
/// **176 小时** —— 完全正常的数据被这个上限判成「异常（请忽略）」，用户看到的
/// 就是永远的「估算中」。所以放到 1000 小时（≈41 天），再由 [`COARSE_HOURS`]
/// 用「10 天以上」这种粗粒度表达「很久」，而不是报错。
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
}

impl Estimate {
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
        let d = dur_short(self.hours);
        if self.percent_per_hour > 0.0 {
            format!("约 {d}后充满")
        } else {
            format!("约 {d}")
        }
    }
}

/// 「沿用」的上限：30 分钟内有过可信结论才继续显示。
///
/// 再久就不该拿旧斜率骗人了 —— 中间可能插过线、换过档、休眠过。
const STICKY_MAX_AGE_SEC: i64 = 1800;

/// 最近一次成功估算。用于 [`predict_sticky`] 的「沿用」。
static LAST_GOOD: Mutex<Option<(Estimate, i64)>> = Mutex::new(None);

/// 带「沿用」的预测：计算本身仍交给 [`predict_from_rows`]，这里只加一层缓存。
///
/// 为什么需要：一次刷新里算不出来（刚跳变 / 刚插拔 / 一瞬间读不到设备）时，
/// 界面会立刻从「约 7 天」掉回「估算中…」，观感上就是"随时都在估算"。
/// 30 分钟内有过结论就继续用那一条，并用 `stale_seconds` 如实标注，不假装是新的。
pub fn predict_sticky(
    rows: &[HistoryRow],
    now_level: u8,
    now_charging: bool,
    now_mode: i32,
    now_sec: i64,
) -> Estimate {
    let fresh = predict_from_rows(rows, now_level, now_charging, now_mode, now_sec);
    let mut slot = LAST_GOOD.lock().unwrap_or_else(|e| e.into_inner());
    if fresh.known {
        *slot = Some((fresh.clone(), now_sec));
        return fresh;
    }
    if let Some((cached, at)) = slot.as_ref() {
        let age = now_sec - *at;
        if (0..=STICKY_MAX_AGE_SEC).contains(&age) {
            let mut e = cached.clone();
            e.stale_seconds = age;
            return e;
        }
    }
    fresh
}

/// 从历史行推算续航/充满时间，**不做文件 IO**。
///
/// `now_sec` 由调用方给：否则「补当前读数」那步会用到真实时间，和合成样本的
/// 时序打架，单测就没法写。
pub fn predict_from_rows(
    rows: &[HistoryRow],
    now_level: u8,
    now_charging: bool,
    now_mode: i32,
    now_sec: i64,
) -> Estimate {
    // 已充满：没什么可预测的
    if decide_charge(now_level, now_charging, now_mode) == ChargeState::Full {
        return Estimate {
            known: false,
            text: "已充满".to_string(),
            ..Default::default()
        };
    }

    // 第一步：取出「连续同相位」的一段（先不管方向）。
    // 只算起始下标、不复制 —— 这条路径每个刷新周期都会跑，2000 行拷两份
    // 纯属垃圾（而且它俩都是「从最新往回扫到第一个不满足就停」的连续尾段，
    // 天然就是一个切片）。
    let phase = &rows[phase_start(rows, now_charging)..];

    // 同相位样本太少 → 说明刚切换过充电/放电，历史还没攒够。
    // 这时不要说「样本不足」糊弄过去，要告诉用户真实原因。
    if phase.len() < 3 {
        return Estimate {
            samples: phase.len(),
            text: if rows.len() >= 3 {
                reason_text(Why::JustSwitched, now_charging)
            } else {
                reason_text(Why::NotEnough, now_charging)
            },
            ..Default::default()
        };
    }

    // 第二步：当前读数和这段历史对不上（相差超过 1%）→ 读数跳变，历史作废
    let last_level = phase[phase.len() - 1].level;
    let conflict = if now_charging {
        now_level + 1 < last_level
    } else {
        now_level > last_level + 1
    };
    if conflict {
        return Estimate {
            samples: phase.len(),
            text: reason_text(Why::Jumped, now_charging),
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

    // 第四步：把当前读数并进序列末尾。历史最后一条可能是十几分钟前的，
    // 不并进去等于拿旧数据预测现在。
    // 只有这里必须复制一次：要往末尾追加当前读数，不能借用历史行切片。
    // 容量一次给够，避免 push 触发扩容。
    let mut full: Vec<HistoryRow> = Vec::with_capacity(sel.len() + 1);
    full.extend_from_slice(sel);
    if let Some(last) = full.last() {
        if last.ts < now_sec {
            full.push(HistoryRow::new(now_sec, now_level, now_charging, now_mode));
        }
    }

    predict_from_series(&full, now_level, now_charging, now_mode)
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
    }
}

/// 「连续同一相位」那一段在 `rows` 里的起始下标（终点恒为 `rows.len()`）。
///
/// 从最新往回扫，扫到相位不同或断档就停 —— 结果必然是一个连续尾段，
/// 所以只返回下标，让调用方用切片，不用复制一份数据。
pub(super) fn phase_start(rows: &[HistoryRow], charging: bool) -> usize {
    let mut start = rows.len();
    let mut prev_t: Option<i64> = None;
    for r in rows.iter().rev() {
        if r.charging != charging {
            break;
        }
        // 断档检测：两条样本间隔超过 HIST_GAP_BREAK_SEC 说明中间根本没在观察
        // （程序没运行 / 机器睡眠 / 鼠标长时间不响应）。这种跨度不能当连续观测用
        // —— 实测遇到过 6 小时空档，两头差 4%，回归把它当成「6 小时均匀掉 4%」，
        // 于是算出一个好看但完全错的斜率。碰到断档就截断，只用断档之后这一段。
        if let Some(pt) = prev_t {
            if pt - r.ts > HIST_GAP_BREAK_SEC {
                break;
            }
        }
        start -= 1;
        prev_t = Some(r.ts);
    }
    start
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
