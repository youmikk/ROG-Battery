//! 结果文案层：把「小时数」「秒数」「算不出来的原因」翻成人话。
//!
//! 抽出来的原因：这些句子是**用户唯一能看到的东西**（环下面那行、页面底部的
//! 明细），改一个措辞就会改掉产品观感；但它们的逻辑（分支怎么选）和协议、
//! 磁盘 IO 毫无关系。混在 `history.rs` 里既不好读，也不好单独钉文案。
//!
//! 六种「算不出来」原来各写一个函数、每份都是「与充电相位对应的两句话」的
//! 复制粘贴 —— 现在收成一张表，加一种原因只加一行。

/// 「算不出来」的原因。选哪一个是 `predict_from_rows` 的事，这里只管怎么说。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// 同相位的样本少于 3 条
    NotEnough,
    /// 刚插上电 / 刚拔掉线，方向还没稳定
    JustSwitched,
    /// 当前读数和最后一条采样差得太多
    Jumped,
    /// 区间内电量没变化（极差 < 1%）
    NoChange,
    /// 数据跨度太短，外推没有意义
    SpanTooShort,
    /// 变化速率接近 0，外推会得到天文数字
    TooSlow,
}

impl Why {
    /// 与充电相位无关的那半句。
    fn sentence(self) -> &'static str {
        match self {
            Why::NotEnough => "样本不足",
            Why::JustSwitched => "刚切换，重新观察中",
            Why::Jumped => "电量刚跳变，重新观察中",
            Why::NoChange => "电量还没变化（需要再多观察一会儿）",
            Why::SpanTooShort => "数据跨度太短",
            Why::TooSlow => "变化太慢，暂时测不出",
        }
    }
}

/// 拼成完整句子：「预计可用时间：xxx」/「预计充满时间：xxx」。
pub fn reason_text(why: Why, charging: bool) -> String {
    let head = if charging {
        "预计充满时间"
    } else {
        "预计可用时间"
    };
    format!("{head}：{}", why.sentence())
}

/// 「还差多久」的长版本：`1 天 6 小时` / `3 小时 12 分钟`。
pub fn dur_text(hours: f64) -> String {
    dur(hours, "分钟")
}

/// `dur_text` 的紧凑版：环下面那格只有一百多像素，省掉一个「钟」字就够用。
pub fn dur_short(hours: f64) -> String {
    dur(hours, "分")
}

fn dur(hours: f64, minute_unit: &str) -> String {
    // 不足 1 分钟也按 1 分钟说 —— 「0 分钟」读起来像坏了
    let total_min = ((hours * 60.0).round() as i64).max(1);
    let days = total_min / (24 * 60);
    let rem = total_min % (24 * 60);
    let h = rem / 60;
    let m = rem % 60;

    if days > 0 {
        return if h == 0 {
            format!("{days} 天")
        } else {
            format!("{days} 天 {h} 小时")
        };
    }
    if h == 0 {
        return format!("{m} 分钟");
    }
    if m == 0 {
        return format!("{h} 小时");
    }
    format!("{h} 小时 {m} {minute_unit}")
}

/// 把秒数说成人话（"刚刚" / "12 分钟" / "1 小时"）。
pub fn ago_text(sec: i64) -> String {
    if sec < 60 {
        "刚刚".to_string()
    } else if sec < 3600 {
        format!("{} 分钟", sec / 60)
    } else {
        format!("{} 小时", sec / 3600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_units_and_carry() {
        assert_eq!(dur_text(0.0), "1 分钟");
        assert_eq!(dur_text(1.0), "1 小时");
        assert_eq!(dur_text(1.5), "1 小时 30 分钟");
        assert_eq!(dur_text(30.0), "1 天 6 小时");
        assert_eq!(dur_text(48.0), "2 天");
        assert_eq!(dur_text(24.0), "1 天");
    }

    #[test]
    fn short_and_long_differ_only_in_the_minute_unit() {
        assert_eq!(dur_short(30.0), "1 天 6 小时");
        // 纯分钟数两种说法一致：环下面那格也只有「41 分钟」这一种读法
        assert_eq!(dur_short(0.68), "41 分钟");
        assert_eq!(dur_short(1.5), "1 小时 30 分");
        assert_eq!(dur_text(0.68), "41 分钟");
    }

    #[test]
    fn ago_text_buckets() {
        assert_eq!(ago_text(0), "刚刚");
        assert_eq!(ago_text(59), "刚刚");
        assert_eq!(ago_text(60), "1 分钟");
        assert_eq!(ago_text(3599), "59 分钟");
        assert_eq!(ago_text(3600), "1 小时");
    }

    #[test]
    fn every_reason_reads_naturally_in_both_phases() {
        for (why, tail) in [
            (Why::NotEnough, "样本不足"),
            (Why::JustSwitched, "刚切换，重新观察中"),
            (Why::Jumped, "电量刚跳变，重新观察中"),
            (Why::NoChange, "电量还没变化（需要再多观察一会儿）"),
            (Why::SpanTooShort, "数据跨度太短"),
            (Why::TooSlow, "变化太慢，暂时测不出"),
        ] {
            assert_eq!(reason_text(why, false), format!("预计可用时间：{tail}"));
            assert_eq!(reason_text(why, true), format!("预计充满时间：{tail}"));
        }
    }
}
