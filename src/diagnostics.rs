
use std::fmt::Write as _;

/// 写进诊断文本的全部事实（都是只读快照，不碰设备）。
pub struct Facts<'a> {
    pub version: &'a str,
    pub status: &'a str,
    pub link_mode: &'a str,
    pub family: &'a str,
    pub frames: usize,
    pub elapsed_ms: u128,
    pub history_rows: usize,
}

/// 渲染成纯文本。字段顺序固定，方便比对两次导出的差异。
pub fn render(f: &Facts<'_>) -> String {
    let mut out = String::with_capacity(512);
    let _ = writeln!(out, "R Battery 诊断信息");
    let _ = writeln!(out, "版本: {}", f.version);
    let _ = writeln!(out, "状态: {}", f.status);
    let _ = writeln!(out, "链路: {}", if f.link_mode.is_empty() { "未知" } else { f.link_mode });
    let _ = writeln!(out, "设备族: {}", f.family);
    let _ = writeln!(out, "本轮帧数: {}", f.frames);
    let _ = writeln!(out, "本轮耗时: {} ms", f.elapsed_ms);
    let _ = writeln!(out, "历史样本: {} 条", f.history_rows);
    out
}

/// 追加一段到日志文件。父目录不存在就建。
///
/// 追加而不是覆盖：同一台机器上「读不到设备」可能发生很多次，覆盖只会留下最后
/// 一次，而用户往往是过一会儿才把文件发过来。
pub fn append(path: &std::path::Path, f: &Facts<'_>) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(dir);
        }
    }
    use std::io::Write as _;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("打开日志失败：{e}"))?;
    file.write_all(format!("--- {stamp} ---\n").as_bytes())
        .and_then(|_| file.write_all(render(f).as_bytes()))
        .map_err(|e| format!("写日志失败：{e}"))
}
