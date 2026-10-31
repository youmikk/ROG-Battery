//! 电量历史采样 + 续航/充满时间估算。
//!
//! 三层，各自只干一件事：
//! - [`store`]：路径解析、跨进程互斥、解析与写入（碰磁盘）
//! - [`predict`]：最小二乘斜率 + 相位过滤 + 「沿用」缓存（纯计算）
//! - [`format`]：把小时数/秒数/算不出来的原因翻成人话（纯文案）
//!
//! 对外仍然是同一个 `crate::history::*` 门面，调用方不用改。

mod format;
mod predict;
mod store;

pub use format::ago_text;
pub use predict::{predict_from_rows, predict_sticky, Estimate};
pub use store::*;

// 单测要直接扫存储层的内部细节，这里把私有函数引进来（不对外导出）。
#[cfg(test)]
use predict::{direction_start, phase_start, COARSE_HOURS};
#[cfg(test)]
use store::{append_line, prune_after_append, read_last_row_at};

// ==================== 单测 ====================

#[cfg(test)]
mod tests {
    use super::*;

    /// 合成时间轴：起点用一个固定时刻，避免测试依赖真实时间。
    const T0: i64 = 1_800_000_000;

    fn row(ts: i64, level: u8, charging: bool) -> HistoryRow {
        HistoryRow::new(ts, level, charging, 2)
    }

    #[test]
    fn parse_real_file_shape() {
        // 就取本机现有那 4 行的真实内容
        let text = "1792568899\t89\t0\t2\n1792569212\t88\t0\t2\n\
                    1792576904\t87\t0\t2\n1792593756\t84\t0\t2\n";
        let rows = parse_rows(text);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0], HistoryRow::new(1792568899, 89, false, 2));
        assert_eq!(rows[3], HistoryRow::new(1792593756, 84, false, 2));
    }

    #[test]
    fn parse_rows_drops_implausible_timestamps() {
        // 一行 i64::MAX、一行 1970 年，都不该进入样本集
        let text = "9223372036854775807\t50\t0\t2\n\
                    0\t50\t0\t2\n\
                    1800000000\t50\t0\t2\n";
        let rows = parse_rows(text);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].ts, 1_800_000_000);
    }

    #[test]
    fn parse_crlf_file() {        // 真机上这个文件是 C# 版 `File.WriteAllLines` 写的，行尾是 CRLF。
        // 我们追加时写 LF（和 C# 的 `AppendAllText` 一致），于是文件里混着两种行尾。
        // 解析必须都吃得下 —— 靠的是先 `trim()` 再去 split('\t')，
        // 否则最后那个字段会变成 "2\r"、parse::<i32> 失败、整行被当成坏行丢掉。
        let text = "1792568899\t89\t0\t2\r\n1792569212\t88\t1\t2\r\n";
        let rows = parse_rows(text);
        assert_eq!(rows.len(), 2, "CRLF 行尾必须能解析");
        assert_eq!(rows[0], HistoryRow::new(1792568899, 89, false, 2));
        assert_eq!(rows[1], HistoryRow::new(1792569212, 88, true, 2));
    }

    #[test]
    fn appended_line_is_lf_only() {
        // 明确锁定我们自己的写入格式：只写 LF，不做 CRLF 翻译。
        // Rust 的 File 永远以二进制方式打开，所以这里其实是在防「以后有人
        // 换成某个会做文本模式转换的封装」——那会让同一个文件出现三种行尾。
        //
        // 落盘位置用 `target/` 而不是 `%TEMP%`：本机开发环境（受限令牌）下
        // 测试进程在 `%TEMP%` 建目录/写文件都会返「拒绝访问 (os error 5)」，
        // 那是环境限制不是代码问题，但会让这条测试红着，掩盖真正的回归。
        // `target/` 一定可写（cargo 刚往里写过东西），且随 `cargo clean` 清掉。
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("test-scratch");
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join(format!("hist-{}.tsv", std::process::id()));
        let _ = std::fs::remove_file(&path);

        append_line(&path, "1800000000\t50\t0\t2").expect("追加");
        let bytes = std::fs::read(&path).expect("读回");
        assert_eq!(bytes, b"1800000000\t50\t0\t2\n", "应当是 LF 结尾且没有多余字节");

        // 再读回来必须得到一条合法记录
        let text = String::from_utf8(bytes).expect("应当是 ASCII");
        let rows = parse_rows(&text);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], HistoryRow::new(1_800_000_000, 50, false, 2));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parse_skips_bad_lines() {
        let text = "1800000000\t50\t0\t2\n\n乱码\t1\t0\t2\n1800000060\t101\t0\t2\n1800000120\t40\t0\n";
        let rows = parse_rows(text);
        assert_eq!(rows.len(), 1, "只应留下第一条合法行");
        assert_eq!(rows[0].ts, 1_800_000_000);
    }

    #[test]
    fn dedupe_same_level() {
        let rows = vec![row(T0, 84, false)];
        // 电量没变，2 分钟内不写
        assert!(!should_record(&rows, 84, false, T0 + 60));
        // 超过 120 秒（HIST_SAME_GAP_SEC）才记心跳
        assert!(should_record(&rows, 84, false, T0 + 121));
    }

    #[test]
    fn dedupe_level_changed() {
        let rows = vec![row(T0, 84, false)];
        // 电量变了，30 秒还不写
        assert!(!should_record(&rows, 83, false, T0 + 30));
        // 过了 60 秒才写
        assert!(should_record(&rows, 83, false, T0 + 61));
    }

    #[test]
    fn dedupe_phase_changed_counts_as_change() {
        let rows = vec![row(T0, 84, false)];
        assert!(!should_record(&rows, 84, true, T0 + 30));
        assert!(should_record(&rows, 84, true, T0 + 61));
    }

    #[test]
    fn first_sample_always_written() {
        assert!(should_record(&[], 50, false, T0));
    }

    #[test]
    fn prune_by_count() {
        let rows: Vec<HistoryRow> = (0..2500)
            .map(|i| row(T0 + i as i64 * 60, 50, false))
            .collect();
        let now = T0 + 2500 * 60;
        let out = prune_rows(&rows, now);
        assert_eq!(out.rows.len(), HIST_MAX_LINES);
        assert!(out.rewrite);
        // 保留的是最近的
        assert_eq!(out.rows[out.rows.len() - 1].ts, rows[rows.len() - 1].ts);
    }

    #[test]
    fn prune_by_time() {
        let old = row(T0 - 49 * 3600, 60, false);
        let keep = row(T0 - 47 * 3600, 60, false);
        let rows = vec![old, keep];
        let out = prune_rows(&rows, T0);
        assert_eq!(out.rows.len(), 1);
        assert_eq!(out.rows[0].ts, keep.ts);
        assert!(out.rewrite);
    }

    #[test]
    fn prune_noop_does_not_rewrite() {
        let rows: Vec<HistoryRow> = (0..10).map(|i| row(T0 + i * 60, 50, false)).collect();
        let out = prune_rows(&rows, T0 + 600);
        assert!(!out.rewrite, "没什么可删的时候不应该重写文件");
        assert_eq!(out.rows.len(), 10);
    }

    #[test]
    fn read_last_row_reads_tail_without_full_parse() {
        // 写采样只读最后一行：给一个 500 行的文件，结果必须等于最后一行。
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-last-row.tsv");
        let mut text = String::new();
        for i in 0..500 {
            text.push_str(&format!("{}\t{}\t0\t2\n", T0 - (500 - i) * 60, 80));
        }
        text.push_str(&format!("{}\t{}\t1\t2\n", T0, 97));
        std::fs::write(&path, text).expect("写测试文件");

        let last = read_last_row_at(&path).expect("应读到最后一行");
        assert_eq!(last.ts, T0);
        assert_eq!(last.level, 97);
        assert!(last.charging, "充电标志也要读出来");

        // 末尾是不完整的残行时，退回上一条完整行
        std::fs::write(&path, format!("{}\t90\t0\t2\n1234", T0)).expect("写残行");
        let last = read_last_row_at(&path).expect("残行应被跳过");
        assert_eq!(last.ts, T0);
        assert_eq!(last.level, 90);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_last_row_handles_missing_file() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("does-not-exist-last-row.tsv");
        let _ = std::fs::remove_file(&path);
        assert!(read_last_row_at(&path).is_none());
    }

    #[test]
    fn prune_after_append_keeps_the_new_sample() {
        // 回归：历史里的行全部超出 48 小时窗口时，裁剪结果里**必须**还有刚追加的
        // 那条。以前传的是追加前的快照，裁剪出来是空表，于是文件被改写成 0 字节，
        // 采样永远攒不下来，界面永远「样本不足」。
        let rows = vec![row(T0 - 50 * 3600, 76, false), row(T0 - 49 * 3600, 76, false)];
        let fresh = row(T0, 98, false);
        let out = prune_after_append(&rows, fresh, T0);
        assert!(out.rewrite);
        assert_eq!(out.rows.len(), 1, "只剩新的那条");
        assert_eq!(out.rows[0].ts, T0);
        assert_eq!(out.rows[0].level, 98);
    }

    #[test]
    fn gap_breaks_phase() {
        // 6 小时空档两头差 4%：整段算斜率会得出「6 小时均匀掉 4%」这种假结论。
        // 断档之后只有 2 条样本，不足 3 条 → 不能说 known。
        let rows = vec![
            row(T0, 90, false),
            row(T0 + 60, 89, false),
            row(T0 + 120, 88, false),
            // 空档 6 小时后
            row(T0 + 6 * 3600, 86, false),
            row(T0 + 6 * 3600 + 60, 86, false),
        ];
        let now_sec = T0 + 6 * 3600 + 60;
        let e = predict_from_rows(&rows, 86, false, 2, now_sec);
        assert!(!e.known);
        assert!(e.samples < 3, "断档后只该剩 2 条，实际 {}", e.samples);
    }

    #[test]
    fn phase_switch_truncates() {
        // 充电段末尾接放电段：放电样本只有 1 条，必须截断
        let rows = vec![
            row(T0, 50, true),
            row(T0 + 600, 52, true),
            row(T0 + 1200, 54, true),
            row(T0 + 1800, 54, false),
        ];
        let e = predict_from_rows(&rows, 54, false, 2, T0 + 1800);
        assert!(!e.known);
        assert!(e.samples < 3, "放电段只该剩 1 条，实际 {}", e.samples);
        // 历史够 3 条但同相位不够 → 是「刚切换」而不是「样本不足」
        assert_eq!(e.text, "预计可用时间：刚切换，重新观察中");
    }

    #[test]
    fn direction_reverse_truncates() {
        // 放电段里出现一次回升 → 只保留回升之后那一段
        let rows = vec![
            row(T0, 60, false),
            row(T0 + 600, 59, false),
            row(T0 + 1200, 58, false),
            row(T0 + 1800, 59, false), // 回升
            row(T0 + 2400, 58, false),
            row(T0 + 3000, 57, false),
        ];
        let e = predict_from_rows(&rows, 57, false, 2, T0 + 3000);
        // 回升之后只剩 3 条（59 → 58 → 57），够算但要确认没用整段
        assert!(e.samples <= 3, "实际 {}", e.samples);
    }

    #[test]
    fn level_jump_invalidates() {
        let rows = vec![
            row(T0, 60, false),
            row(T0 + 600, 59, false),
            row(T0 + 1200, 58, false),
        ];
        // 当前读数比历史末条高 2% → 跳变
        let e = predict_from_rows(&rows, 60, false, 2, T0 + 1200);
        assert!(!e.known);
        assert_eq!(e.text, "预计可用时间：电量刚跳变，重新观察中");
    }

    #[test]
    fn too_few_samples() {
        let rows = vec![row(T0, 60, false), row(T0 + 600, 59, false)];
        let e = predict_from_rows(&rows, 59, false, 2, T0 + 600);
        assert!(!e.known);
        assert_eq!(e.text, "预计可用时间：样本不足");
    }

    #[test]
    fn linear_discharge_gives_sane_estimate() {
        // 每 10 分钟掉 1% → 5 小时掉 30%，从 80% 起算应剩约 13.3 小时
        let mut rows = Vec::new();
        for i in 0..=30i64 {
            rows.push(row(T0 + i * 600, 80 - i as u8, false));
        }
        let now_sec = T0 + 30 * 600;
        let e = predict_from_rows(&rows, 50, false, 2, now_sec);
        assert!(e.known, "应该能算出结论，实际：{}", e.text);
        assert!((e.percent_per_hour + 6.0).abs() < 0.2, "斜率 {}", e.percent_per_hour);
        // 50% / 6 %/h ≈ 8.3 小时
        assert!(e.text.starts_with("预计还能用"), "{}", e.text);
    }

    #[test]
    fn full_charge_reports_full() {
        // 有线 + 100% + 不在充电 → 已充满
        let e = predict_from_rows(&[], 100, false, 1, T0);
        assert!(!e.known);
        assert_eq!(e.text, "已充满");
    }

    #[test]
    fn decide_charge_matches_device_behavior() {
        // 设备只在真的在充时报 1
        assert_eq!(decide_charge(99, true, 2), ChargeState::Charging);
        // 100% 后回 0，靠有线判充满
        assert_eq!(decide_charge(100, false, 1), ChargeState::Full);
        // 无线 100% 不算充满（没插线）
        assert_eq!(decide_charge(100, false, 2), ChargeState::Discharging);
    }

        /// 真机现场：98% → 97% 用了 1 小时 48 分（≈0.55 %/h），外推 176 小时。
    /// 旧上限 100 小时把它判成「异常（请忽略）」，界面永远「估算中」。
    /// 用户报的就是这个：不能持久计算，随时都是估算中。
    #[test]
    fn slow_drain_still_gives_an_answer() {
        let rows = vec![
            row(T0, 98, false),
            row(T0 + 3600, 98, false),
            row(T0 + 6480, 97, false),
        ];
        let e = predict_from_rows(&rows, 97, false, 2, T0 + 6480);
        assert!(e.known, "慢放电必须能给出结论，实际 text={}", e.text);
        assert!(
            e.hours > 150.0 && e.hours < 220.0,
            "外推应约 176 小时，实际 {}",
            e.hours
        );
        assert!(e.text.contains("天"), "超过一天要按天说：{}", e.text);
        assert!(!e.text.contains("异常"), "不该再说异常：{}", e.text);
        assert!(e.short_text().starts_with("约 "), "{}", e.short_text());
    }

    /// 超过 10 天就只说「10 天以上」——早期外推装精确是骗人。
    #[test]
    fn very_long_estimate_is_coarse() {
        let rows = vec![
            row(T0, 99, false),
            row(T0 + 6000, 99, false),
            row(T0 + 12000, 99, false),
            row(T0 + 18000, 98, false),
        ];
        let e = predict_from_rows(&rows, 98, false, 2, T0 + 18000);
        assert!(e.known, "{}", e.text);
        assert!(e.hours > COARSE_HOURS, "{}", e.hours);
        assert_eq!(e.text, "预计还能用 10 天以上");
    }

    /// 「沿用」：本次算不出来时，30 分钟内的旧结论继续给，但要用 stale_seconds 标注。
    #[test]
    fn sticky_keeps_last_good_estimate() {
        let rows = vec![
            row(T0, 98, false),
            row(T0 + 3600, 98, false),
            row(T0 + 6480, 97, false),
        ];
        let first = predict_sticky(&rows, 97, false, 2, T0 + 6480);
        assert!(first.known && first.stale_seconds == 0, "{:?}", first);

        // 样本全没了（相当于刚清空历史 / 一瞬间读不到）—— 应沿用刚才那条
        let again = predict_sticky(&[], 97, false, 2, T0 + 6480 + 600);
        assert!(again.known, "应沿用上一条，实际 text={}", again.text);
        assert_eq!(again.stale_seconds, 600);
        assert_eq!(again.hours, first.hours);
    }

    #[test]
    fn history_path_uses_rog_battery_dir() {
        // 目录名故意不是 rog-mouse-ctl —— 要复用 C# 版的数据。
        // 用 primary_history_path：history_path 会按可写性降级，单测环境里
        // %LOCALAPPDATA% 可能不可写，拿它断言会误报。
        let p = primary_history_path().display().to_string();
        assert!(p.contains("rog-battery"), "{}", p);
        assert!(p.ends_with("history.tsv"), "{}", p);
    }

    #[test]
    fn phase_and_direction_pick_contiguous_tail() {
        // 相位不同就截断：只有最后两条是连续放电
        let rows = vec![
            row(1_800_000_000, 90, false),
            row(1_800_000_060, 89, false),
            row(1_800_000_120, 88, false),
            row(1_800_000_180, 87, true), // 相位变了
            row(1_800_000_240, 86, false),
            row(1_800_000_300, 85, false),
        ];
        let p = phase_start(&rows, false);
        assert_eq!(p, 4);
        assert_eq!(rows[p..].len(), 2);
        assert_eq!(direction_start(&rows[p..], false), 0);

        // 中途回升：方向过滤只保留回升之后那段
        let rows2 = vec![
            row(1_800_000_000, 70, false),
            row(1_800_000_060, 72, false), // 回升
            row(1_800_000_120, 71, false),
            row(1_800_000_180, 70, false),
        ];
        let p2 = phase_start(&rows2, false);
        assert_eq!(p2, 0);
        assert_eq!(direction_start(&rows2[p2..], false), 1);
    }

    #[test]
    fn phase_start_breaks_on_long_gap() {
        // 间隔超过 HIST_GAP_BREAK_SEC 就不算连续观测
        let rows = vec![
            row(1_800_000_000, 90, false),
            row(1_800_000_060, 89, false),
            row(1_800_000_060 + HIST_GAP_BREAK_SEC + 60, 60, false),
        ];
        let p = phase_start(&rows, false);
        assert_eq!(p, 2, "断档之前的两条应当被截掉");
        assert_eq!(rows[p..].len(), 1);
    }

    #[test]
    fn parse_rows_tolerates_extra_fields_and_skips_short_lines() {
        // 第 2 行只有 3 段（丢弃）、第 3 行有 6 段（照收）、第 4 行只有 2 段（丢弃）
        let text = "1800000000\t80\t1\t2\n1800000060\t79\t1\n1800000120\t78\t0\t2\t备注\t多出来的字段\n1800000180\t77\n";
        let rows = parse_rows(text);
        assert_eq!(rows.len(), 2, "少字段的行丢弃，多字段的行照收");
        assert_eq!(rows[0].ts, 1_800_000_000);
        assert_eq!(rows[1].ts, 1_800_000_120);
        assert_eq!(rows[1].level, 78);
    }
}
