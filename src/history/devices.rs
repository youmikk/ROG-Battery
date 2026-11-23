//! 多设备历史的文本格式、合并和采样规则，不访问文件或设备。

use std::collections::{BTreeMap, BTreeSet};

use super::store::{
    decide_charge, parse_rows, HistoryRow, HIST_KEEP_HOURS, HIST_MAX_LINES,
    HIST_MIN_GAP_SEC, HIST_SAME_GAP_SEC, TS_MAX_PLAUSIBLE, TS_MIN_PLAUSIBLE,
};

pub type DeviceRows = BTreeMap<String, Vec<HistoryRow>>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceHistoryFile {
    pub devices: DeviceRows,
    /// 旧四列格式没有设备标识，确认唯一归属前不能分配给任意设备。
    pub legacy: Vec<HistoryRow>,
}

const FLAG_APPROXIMATE: u8 = 1;
const FLAG_CHARGING_KNOWN: u8 = 2;
const FLAG_BREAK_BEFORE: u8 = 4;
const ALL_FLAGS: u8 = FLAG_APPROXIMATE | FLAG_CHARGING_KNOWN | FLAG_BREAK_BEFORE;

pub(super) fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 512
        && key.trim() == key
        && !key.chars().any(char::is_control)
}

pub(super) fn valid_row(row: &HistoryRow) -> bool {
    row.level <= 100 && (TS_MIN_PLAUSIBLE..=TS_MAX_PLAUSIBLE).contains(&row.ts)
}

/// 仅解析；写入层保留裁剪前的快照，以便没有实际变化时不重写文件。
pub(super) fn parse_device_history(text: &str) -> DeviceHistoryFile {
    let mut history = DeviceHistoryFile { legacy: parse_rows(text), ..Default::default() };
    for raw in text.lines() {
        let mut fields = raw.trim_end_matches('\r').split('\t');
        let (Some("v2"), Some(key), Some(ts), Some(level), Some(charging), Some(mode), Some(flags)) =
            (fields.next(), fields.next(), fields.next(), fields.next(), fields.next(), fields.next(), fields.next())
        else { continue; };
        if fields.next().is_some() || !valid_key(key) { continue; }
        let (Ok(ts), Ok(level), Ok(charging), Ok(mode), Ok(flags)) =
            (ts.parse::<i64>(), level.parse::<u8>(), charging.parse::<u8>(), mode.parse::<i32>(), flags.parse::<u8>())
        else { continue; };
        if charging > 1 || flags & !ALL_FLAGS != 0 { continue; }
        let mut row = HistoryRow::new(ts, level, charging != 0, mode);
        row.approximate = flags & FLAG_APPROXIMATE != 0;
        row.charging_known = flags & FLAG_CHARGING_KNOWN != 0;
        row.break_before = flags & FLAG_BREAK_BEFORE != 0;
        if valid_row(&row) { history.devices.entry(key.into()).or_default().push(row); }
    }
    history
}

/// 解析并按每台设备分别保留最近48小时、最多2000条。不会把旧格式自动归属。
#[cfg(test)]
pub fn parse_device_history_at(text: &str, now_sec: i64) -> DeviceHistoryFile {
    let mut history = parse_device_history(text);
    normalize(&mut history, now_sec);
    history
}

fn flags(row: &HistoryRow) -> u8 {
    u8::from(row.approximate) * FLAG_APPROXIMATE
        | u8::from(row.charging_known) * FLAG_CHARGING_KNOWN
        | u8::from(row.break_before) * FLAG_BREAK_BEFORE
}

pub(super) fn render(history: &DeviceHistoryFile) -> String {
    let mut text = String::new();
    for row in &history.legacy {
        use std::fmt::Write;
        let _ = writeln!(text, "{}\t{}\t{}\t{}", row.ts, row.level, u8::from(row.charging), row.mode);
    }
    for (key, rows) in &history.devices {
        for row in rows {
            use std::fmt::Write;
            let _ = writeln!(text, "v2\t{key}\t{}\t{}\t{}\t{}\t{}",
                row.ts, row.level, u8::from(row.charging), row.mode, flags(row));
        }
    }
    text
}

fn normalize_rows(rows: &mut Vec<HistoryRow>, now_sec: i64) {
    let cutoff = now_sec.saturating_sub(HIST_KEEP_HOURS * 3600);
    rows.retain(|row| valid_row(row) && row.ts >= cutoff && row.ts <= now_sec);
    rows.sort_by_key(|row| row.ts);
    // 路径迁移和身份合并都可能遇到相同样本。断点是样本属性，重复合并时取OR，
    // 否则一个旧副本会把另一份文件中已确认的断点抹掉。
    let mut indices: BTreeMap<(i64, u8, bool, i32, bool, bool), usize> = BTreeMap::new();
    let mut merged: Vec<HistoryRow> = Vec::with_capacity(rows.len());
    for row in rows.drain(..) {
        let key = (row.ts, row.level, row.charging, row.mode, row.approximate, row.charging_known);
        if let Some(&index) = indices.get(&key) {
            merged[index].break_before |= row.break_before;
        } else {
            indices.insert(key, merged.len());
            merged.push(row);
        }
    }
    if merged.len() > HIST_MAX_LINES {
        merged.drain(..merged.len() - HIST_MAX_LINES);
    }
    *rows = merged;
}

pub(super) fn normalize(history: &mut DeviceHistoryFile, now_sec: i64) {
    normalize_rows(&mut history.legacy, now_sec);
    for rows in history.devices.values_mut() { normalize_rows(rows, now_sec); }
    history.devices.retain(|key, rows| valid_key(key) && !rows.is_empty());
}

/// 状态和可靠性变化立即记录；普通电量变化60秒、未变化120秒。
/// break_before表示发生一次断档，即使上一条也有断点，新时间的断点仍要记录。
pub fn should_record_device(rows: &[HistoryRow], row: HistoryRow) -> bool {
    if !valid_row(&row) { return false; }
    let Some(last) = rows.last() else { return true; };
    if row.ts < last.ts || row == *last { return false; }
    if row.break_before || row.break_before != last.break_before
        || row.approximate != last.approximate || row.charging_known != last.charging_known
        || row.mode != last.mode || row.charging != last.charging
        || decide_charge(row.level, row.charging, row.mode) != decide_charge(last.level, last.charging, last.mode)
    { return true; }
    let gap = row.ts.saturating_sub(last.ts);
    gap >= if row.level != last.level { HIST_MIN_GAP_SEC } else { HIST_SAME_GAP_SEC }
}

pub(super) fn validate_update(samples: &[(String, HistoryRow)], aliases: &[(String, String)],
    legacy_owner: Option<&str>) -> Result<(), String> {
    if samples.iter().any(|(key, row)| !valid_key(key) || !valid_row(row)) {
        return Err("历史采样的设备标识、电量或时间戳无效".into());
    }
    if aliases.iter().any(|(old, new)| !valid_key(old) || !valid_key(new))
        || legacy_owner.is_some_and(|key| !valid_key(key)) {
        return Err("历史迁移的设备标识无效".into());
    }
    Ok(())
}

fn alias_routes(aliases: &[(String, String)]) -> BTreeMap<&str, &str> {
    let mut edges: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (old, new) in aliases {
        if old != new { edges.entry(old).or_default().insert(new); }
    }
    let mut routes = BTreeMap::new();
    for &start in edges.keys() {
        let mut cursor = start;
        let mut visited = BTreeSet::new();
        while visited.insert(cursor) {
            match edges.get(cursor) {
                None => { routes.insert(start, cursor); break; }
                Some(next) if next.len() == 1 => cursor = next.iter().next().copied().unwrap(),
                // 一个旧身份被指向两个设备时，不能选择其中一个。循环也保持原数据。
                Some(_) => break,
            }
        }
    }
    routes
}

pub(super) fn apply_update(history: &mut DeviceHistoryFile, samples: &[(String, HistoryRow)],
    aliases: &[(String, String)], legacy_owner: Option<&str>, now_sec: i64) {
    let routes = alias_routes(aliases);
    // 只迁移文件里实际存在的旧键；不为已遗忘/未知设备创建空历史。
    let old_devices = std::mem::take(&mut history.devices);
    for (key, rows) in old_devices {
        let destination = routes.get(key.as_str()).copied().unwrap_or(key.as_str()).to_string();
        history.devices.entry(destination).or_default().extend(rows);
    }
    if let Some(owner) = legacy_owner {
        if !history.legacy.is_empty() {
            let destination = routes.get(owner).copied().unwrap_or(owner);
            history.devices.entry(destination.into()).or_default().append(&mut history.legacy);
        }
    }
    normalize(history, now_sec);
    // 同一批可能含多台设备、多次采样。每台先按时间处理，再独立节流。
    let mut pending: BTreeMap<&str, Vec<HistoryRow>> = BTreeMap::new();
    for (key, row) in samples {
        if row.ts <= now_sec {
            // 写盘重试时样本可能早于身份升级离开队列。沿用同批别名，避免
            // 已迁移的旧键被这个在途样本再次创建。
            let destination = routes.get(key.as_str()).copied().unwrap_or(key.as_str());
            pending.entry(destination).or_default().push(*row);
        }
    }
    for (key, mut rows) in pending {
        rows.sort_by_key(|row| row.ts);
        let stored = history.devices.entry(key.into()).or_default();
        for row in rows {
            // 精确重复只补断点，不因来源文件/队列重复而增加一行。
            if let Some(existing) = stored.iter_mut().find(|old|
                old.ts == row.ts && old.level == row.level && old.charging == row.charging
                && old.mode == row.mode && old.approximate == row.approximate
                && old.charging_known == row.charging_known) {
                existing.break_before |= row.break_before;
            } else if should_record_device(stored, row) {
                stored.push(row);
            }
        }
    }
    normalize(history, now_sec);
}

pub(super) fn merge_into(target: &mut DeviceHistoryFile, source: DeviceHistoryFile, now_sec: i64) {
    target.legacy.extend(source.legacy);
    for (key, rows) in source.devices { target.devices.entry(key).or_default().extend(rows); }
    normalize(target, now_sec);
}

#[cfg(test)]
mod tests {
    use super::*;
    const NOW: i64 = 1_800_000_000;

    #[test]
    fn device_limits_and_time_windows_are_independent() {
        let mut history = DeviceHistoryFile::default();
        for key in ["mouse", "keyboard"] {
            let rows = history.devices.entry(key.into()).or_default();
            rows.push(HistoryRow::new(NOW - HIST_KEEP_HOURS * 3600 - 1, 90, false, 2));
            rows.push(HistoryRow::new(NOW + 1, 90, false, 2));
            for age in (0..2100).rev() { rows.push(HistoryRow::new(NOW - age, 80, false, 2)); }
            rows.reverse();
        }
        normalize(&mut history, NOW);
        for rows in history.devices.values() {
            assert_eq!(rows.len(), 2000);
            assert_eq!(rows.first().unwrap().ts, NOW - 1999);
            assert_eq!(rows.last().unwrap().ts, NOW);
        }
        assert_eq!(history.devices.len(), 2);
    }

    #[test]
    fn v2_flags_roundtrip_and_legacy_parser_never_reads_device_rows() {
        let mut history = DeviceHistoryFile::default();
        history.legacy.push(HistoryRow::new(NOW, 75, false, 2));
        for bits in 0..=ALL_FLAGS {
            let mut row = HistoryRow::new(NOW - i64::from(bits), 60, bits % 2 == 0, 0);
            row.approximate = bits & FLAG_APPROXIMATE != 0;
            row.charging_known = bits & FLAG_CHARGING_KNOWN != 0;
            row.break_before = bits & FLAG_BREAK_BEFORE != 0;
            history.devices.entry("device".into()).or_default().push(row);
        }
        normalize(&mut history, NOW);
        let text = render(&history);
        assert_eq!(parse_device_history_at(&text, NOW), history);
        assert_eq!(parse_rows(&text), history.legacy);
        let defaults = HistoryRow::new(NOW, 50, false, 2);
        assert!(!defaults.approximate && defaults.charging_known && !defaults.break_before);
        let invalid = format!("v2\tx\t{NOW}\t50\t0\t2\t8\nv2\tx\t{NOW}\t101\t0\t2\t2\nv2\t\t{NOW}\t50\t0\t2\t2\n");
        assert!(parse_device_history_at(&invalid, NOW).devices.is_empty());
    }

    #[test]
    fn legacy_is_unclaimed_without_owner_then_consumed_only_once() {
        let old = HistoryRow::new(NOW - 120, 80, false, 2);
        let mut history = DeviceHistoryFile { legacy: vec![old], ..Default::default() };
        let samples = vec![("rog-a".into(), HistoryRow::new(NOW, 78, false, 2)),
            ("rog-b".into(), HistoryRow::new(NOW, 20, false, 2))];
        apply_update(&mut history, &samples, &[], None, NOW);
        assert_eq!(history.legacy, [old]);
        assert_eq!(history.devices["rog-a"].len(), 1);
        assert_eq!(history.devices["rog-b"].len(), 1);
        apply_update(&mut history, &[], &[], Some("rog-a"), NOW);
        assert!(history.legacy.is_empty());
        assert_eq!(history.devices["rog-a"], [old, samples[0].1]);
        let once = history.clone();
        apply_update(&mut history, &[], &[], Some("rog-b"), NOW);
        assert_eq!(history, once);
    }

    #[test]
    fn alias_chains_merge_existing_keys_and_preserve_duplicate_breaks() {
        let row = HistoryRow::new(NOW - 120, 80, false, 2);
        let mut broken = row; broken.break_before = true;
        let mut history = DeviceHistoryFile::default();
        history.devices.insert("old".into(), vec![row]);
        history.devices.insert("current".into(), vec![broken]);
        let aliases = vec![("bridge".into(), "current".into()), ("old".into(), "bridge".into()),
            ("absent".into(), "never-create".into())];
        apply_update(&mut history, &[], &aliases, None, NOW);
        assert_eq!(history.devices.len(), 1);
        assert_eq!(history.devices["current"], [broken]);
        let once = history.clone();
        apply_update(&mut history, &[], &aliases, None, NOW);
        assert_eq!(history, once);
        let ambiguous = vec![("current".into(), "a".into()), ("current".into(), "b".into())];
        apply_update(&mut history, &[], &ambiguous, None, NOW);
        assert_eq!(history, once);
        let cycle = vec![("current".into(), "a".into()), ("a".into(), "current".into())];
        apply_update(&mut history, &[], &cycle, None, NOW);
        assert_eq!(history, once);
    }

    #[test]
    fn in_flight_old_key_samples_follow_final_alias_without_recreating_old_history() {
        let mut history = DeviceHistoryFile::default();
        let old = HistoryRow::new(NOW - 120, 80, false, 2);
        history.devices.insert("temporary".into(), vec![old]);
        let current = HistoryRow::new(NOW, 79, false, 2);
        let aliases = [("intermediate".into(), "verified".into()),
            ("temporary".into(), "intermediate".into())];
        let pending = [("temporary".into(), current), ("verified".into(), current)];
        apply_update(&mut history, &pending, &aliases, None, NOW);
        assert_eq!(history.devices.len(), 1);
        assert_eq!(history.devices["verified"], [old, current]);
        let once = history.clone();
        apply_update(&mut history, &pending, &aliases, None, NOW);
        assert_eq!(history, once);
    }

    #[test]
    fn per_device_throttling_preserves_phase_reliability_and_break_changes() {
        let base = HistoryRow::new(NOW - 120, 80, false, 2);
        let rows = [base];
        assert!(!should_record_device(&rows, base));
        assert!(!should_record_device(&rows, HistoryRow::new(base.ts + 119, 80, false, 2)));
        assert!(should_record_device(&rows, HistoryRow::new(base.ts + 120, 80, false, 2)));
        assert!(!should_record_device(&rows, HistoryRow::new(base.ts + 59, 79, false, 2)));
        assert!(should_record_device(&rows, HistoryRow::new(base.ts + 60, 79, false, 2)));
        for flag in 0..5 {
            let mut changed = base; changed.ts += 1;
            match flag { 0 => changed.charging = true, 1 => changed.mode = 1,
                2 => changed.approximate = true, 3 => changed.charging_known = false,
                _ => changed.break_before = true }
            assert!(should_record_device(&rows, changed));
        }
        let full = [HistoryRow::new(NOW - 1, 99, false, 1)];
        assert!(should_record_device(&full, HistoryRow::new(NOW, 100, false, 1)));
        let mut history = DeviceHistoryFile::default();
        history.devices.insert("a".into(), vec![base]);
        apply_update(&mut history, &[("a".into(), HistoryRow::new(base.ts + 1, 79, false, 2)),
            ("b".into(), HistoryRow::new(base.ts + 1, 40, false, 2))], &[], None, NOW);
        assert_eq!(history.devices["a"], rows);
        assert_eq!(history.devices["b"].len(), 1);
    }
}
