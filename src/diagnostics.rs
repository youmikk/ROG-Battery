//! 出错时落一份可复制的诊断文本。
//!
//! 用户报「读不到鼠标」时，能问的只有「什么型号、什么系统、读了几帧」。与其
//! 让人来回截图，不如程序自己把当时的事实写成一个纯文本文件，让用户直接贴。

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

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts<'static> {
        Facts {
            version: "1.3.3",
            status: "未检测到设备（没有任何帧有应答）",
            link_mode: "Wireless24G",
            family: "鼠标",
            frames: 0,
            elapsed_ms: 0,
            history_rows: 37,
        }
    }

    #[test]
    fn render_contains_every_fact() {
        let t = render(&facts());
        for needle in ["1.3.3", "未检测到设备", "Wireless24G", "鼠标", "37 条"] {
            assert!(t.contains(needle), "缺少 {needle}：\n{t}");
        }
    }

    #[test]
    fn empty_link_is_written_as_unknown() {
        let mut f = facts();
        f.link_mode = "";
        assert!(render(&f).contains("链路: 未知"));
    }

    #[test]
    fn append_keeps_earlier_blocks() {
        // 沙箱里子进程只能写仓库内路径，`%TEMP%` 会 Access denied。
        let p = std::path::PathBuf::from("target/rog-diag-test.txt");
        let _ = std::fs::remove_file(&p);
        append(&p, &facts()).expect("写诊断文件");
        append(&p, &facts()).expect("再写一次");
        let got = std::fs::read_to_string(&p).expect("读回");
        assert_eq!(got.matches("R Battery 诊断信息").count(), 2);
        let _ = std::fs::remove_file(&p);
    }
}
