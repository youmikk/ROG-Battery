//! 下拉框与动作目标的数据层：候选表、当前值文案、以及「下标 → 设备值」。
//!
//! 这一层原来也长在 `pages.rs` 里，导致 `gui.rs` 提交命令时要反向依赖
//! 绘制模块才能把「用户点了第几项」翻成「协议要写什么值」。

use super::pages::Data;
use super::widget::Drop;

/// 下拉框第 `idx` 项对应的**语义值**。
///
/// 语义值不等于下标：灯光模式的表是 `[0, 1, 2, 4, 6, 0xF0]`（第 3 项的值是 4），
/// 刷新间隔的表是 `[15, 30, 60, 120, 300]` 秒。谁要按下标去推语义值，
/// 就一定会在「灯光选第 3 项」或「刷新间隔选第 4 项」上错位。
///
/// 越界返 `None`。
pub fn option_value(drop: Drop, idx: usize) -> Option<usize> {
    raw_options(drop).get(idx).map(|(_, v)| *v)
}

/// 候选项本身。
///
/// 抽成独立函数是因为 [`option_value`] 也要用同一张表 —— 表一旦分成两份，
/// 改了一处忘了另一处，界面显示的和实际写进设备的值就会错位。
pub fn raw_options(drop: Drop) -> &'static [(String, usize)] {
    RAW[slot_of(drop)].get_or_init(|| build_options(drop)).as_slice()
}

/// `Drop` → 缓存槽位下标。只用于选表，语义必须与 `build_options` 的分支一一对应。
fn slot_of(drop: Drop) -> usize {
    match drop {
        Drop::Sleep => 0,
        Drop::Poll => 1,
        Drop::Stage => 2,
        Drop::Debounce => 3,
        Drop::Lod => 4,
        Drop::LightMode => 5,
        Drop::Refresh => 6,
        Drop::CacheTtl => 7,
        Drop::ButtonTarget(_) => 8,
        Drop::Palette(_) => 9,
        Drop::CloseMode => 10,
    }
}

const DROP_SLOTS: usize = 11;

/// 每种下拉的候选表一辈子都不变，却原来每帧重建一次（`format!` 逐个分配
/// `String`）。设置页四五个下拉打开时，一帧就是几十次堆分配。
///
/// 这里按 `Drop` 分槽缓存。`ButtonTarget(_)` 八个槽位共用一张表
/// （`button_target_name` 只按码查名字），`Palette(_)` 故意给空表。
static RAW: [std::sync::OnceLock<Vec<(String, usize)>>; DROP_SLOTS] =
    [const { std::sync::OnceLock::new() }; DROP_SLOTS];

fn build_options(drop: Drop) -> Vec<(String, usize)> {
    // 每个分支写全 `Vec<(String, usize)>`：不加这个标注，
    // arms 各自是不同的 iterator 类型，`collect()` 推不出目标类型。
    let items: Vec<(String, usize)> = match drop {
        Drop::Sleep => (0u8..=4)
            .map(|c| (crate::proto::sleep_mode_name(c), c as usize))
            .chain(std::iter::once((
                crate::proto::sleep_mode_name(0xFF),
                0xFFusize,
            )))
            .collect(),
        // 用 `u16` 不是 `u8`：1000 > 255，写 `u8` 编译期就报 literal out of range。
        // 这里的值只是显示用的 Hz 数，和 `poll_code`（0..3）是两回事。
        Drop::Poll => [125u16, 250, 500, 1000]
            .iter()
            .enumerate()
            .map(|(i, hz)| (format!("{hz} Hz"), i))
            .collect(),
        // value 必须是**协议里的 1 基档位**，不是 0 基下标。
        //
        // 原来写的是 `(0..4).map(|i| (..., i))`，于是：
        //   点「档位 1」→ 传 stage=0 → `set_stage` 报「档位必须是 1-4」
        //   点「档位 2」→ 传 stage=1 → 设备真的切到 **档位 1**
        //   点「档位 3」→ 传 stage=2 → 设备真的切到 **档位 2**
        //   点「档位 4」→ 传 stage=3 → 设备真的切到 **档位 3**
        // 用户原话「切换 dpi 没有反应，写的是切换了实际没有切换档位」——
        // 真机点「档位 3」时设备正好当前是 2，切到 2 等于没变，
        // 界面高亮也跟着留在 2，看起来完全没反应。
        Drop::Stage => (0..4)
            .map(|i| (format!("档位 {}", i + 1), i + 1))
            .collect(),
        Drop::Debounce => (0u8..=7)
            .map(|c| (crate::proto::debounce_name(c), c as usize))
            .collect(),
        Drop::Lod => (0u8..=1)
            .map(|c| (crate::proto::lod_name(c), c as usize))
            .collect(),
        // **只列设备认的模式**：Rainbow(3) 与 Comet(5) 写进去会被静默回落到
        // ColorCycle，1.1.0 的 CLI 干脆不提供这两个值。列出来只会让用户以为能选。
        Drop::LightMode => [0x00u8, 0x01, 0x02, 0x04, 0x06, 0xF0]
            .iter()
            .map(|&c| (crate::proto::light_mode_name(c), c as usize))
            .collect(),
        Drop::Refresh => [15u64, 30, 60, 120, 300]
            .iter()
            .map(|s| (format!("{s} 秒"), *s as usize))
            .collect(),
        Drop::CacheTtl => [0u64, 15, 30, 60, 300]
            .iter()
            .map(|s| {
                if *s == 0 {
                    ("每次都重读".to_string(), 0)
                } else {
                    (format!("{s} 秒"), *s as usize)
                }
            })
            .collect(),
        // value 是**目标码本身**（如 `0x01F0`），不是下标 —— 这样
        // [`option_value`] 的返回值可以直接写进设备，中间不用再查一次表。
        // 8 个槽位共用同一张候选表，让 `Drop` 保持无参数
        // （`ButtonTarget(slot)` 只区分槽位）。
        Drop::ButtonTarget(_) => BUTTON_TARGETS
            .iter()
            .map(|&c| (button_target_name(c), c as usize))
            .collect(),
        // 色盘不是文字列表，`draw_palette` 走单独一支。这里给空表而不是
        // `palette_colors()`：万一有人误走文字下拉，空表会显示成「没有选项」
        // （一眼看出走错了），而颜色列表会被画成 32 行没法读的文字。
        Drop::Palette(_) => Vec::new(),
        Drop::CloseMode => vec![
            ("每次询问".to_string(), 0),
            ("最小化到系统托盘".to_string(), 1),
            ("直接退出程序".to_string(), 2),
        ],
    };
    items
}

/// 某个下拉框当前该显示什么文字。
pub fn drop_current(drop: Drop, d: &Data) -> String {
    // 读不到设备（或只有占位值）时不要编一个「1000Hz」出来，直接显示「—」。
    if d.state.map(super::pages::placeholder_state).unwrap_or(true) {
        return "—".to_string();
    }
    let st = d.state;
    match drop {
        Drop::Sleep => st
            .map(|s| crate::proto::sleep_mode_name(s.sleep_mode))
            .unwrap_or_else(|| "—".to_string()),
        Drop::Poll => st
            .map(|s| crate::proto::poll_name(s.poll_code))
            .unwrap_or_else(|| "—".to_string()),
        Drop::Stage => st
            .map(|s| format!("档位 {}", s.active_stage.max(1)))
            .unwrap_or_else(|| "—".to_string()),
        Drop::Debounce => st
            .map(|s| crate::proto::debounce_name(s.debounce_code))
            .unwrap_or_else(|| "—".to_string()),
        Drop::Lod => st
            .map(|s| crate::proto::lod_name(s.lod))
            .unwrap_or_else(|| "—".to_string()),
        Drop::LightMode => st
            .map(|s| crate::proto::light_mode_name(s.light_mode))
            .unwrap_or_else(|| "—".to_string()),
        Drop::Refresh => format!("{} 秒", d.cfg.refresh_interval_sec),
        Drop::CacheTtl => {
            if d.cfg.cache_ttl_sec == 0 {
                "每次都重读".to_string()
            } else {
                format!("{} 秒", d.cfg.cache_ttl_sec)
            }
        }
        Drop::ButtonTarget(slot) => d
            .state
            .map(|s| button_target_name(s.buttons[(slot as usize) & 7]))
            .unwrap_or_else(|| "—".to_string()),
        // 色盘自己不显示文字（色块本身就是显示）。走到这里是走错分支了，
        // 给「—」而不是编一个色号。
        Drop::Palette(_) => "—".to_string(),
        Drop::CloseMode => match d.cfg.close_mode.as_str() {
            "minimize" => "最小化到系统托盘".to_string(),
            "quit" => "直接退出程序".to_string(),
            _ => "每次询问".to_string(),
        },
    }
}

/// 当前选中的下标。
///
/// 一律「拿语义值反查下标」，不按下标正查 —— 后者要求「表格顺序恰好等于设备
/// 编码顺序」，而灯光模式的表是 `[0, 1, 2, 4, 6, 0xF0]`（中间缺 3 和 5），
/// 一旦按下标正查，选 Rainbow 会显示成 React。
pub fn drop_selected(drop: Drop, d: &Data) -> usize {
    let opts = options(drop, d);
    let st = d.state;
    let cur: usize = match drop {
        Drop::Sleep => st.map(|s| s.sleep_mode as usize).unwrap_or(0),
        Drop::Poll => st.map(|s| s.poll_code as usize).unwrap_or(0),
        // `active_stage` 是 1 基的（0 表示设备没报），候选值也是 1..=4
        Drop::Stage => st.map(|s| s.active_stage.max(1) as usize).unwrap_or(1),
        Drop::Debounce => st.map(|s| s.debounce_code as usize).unwrap_or(0),
        Drop::Lod => st.map(|s| s.lod as usize).unwrap_or(0),
        Drop::LightMode => st.map(|s| s.light_mode as usize).unwrap_or(0),
        Drop::Refresh => d.cfg.refresh_interval_sec as usize,
        Drop::CacheTtl => d.cfg.cache_ttl_sec as usize,
        Drop::ButtonTarget(slot) => st
            .map(|s| s.buttons[(slot as usize) & 7] as usize)
            .unwrap_or(0),
        Drop::Palette(_) => 0,
        Drop::CloseMode => match d.cfg.close_mode.as_str() {
            "minimize" => 1,
            "quit" => 2,
            _ => 0,
        },
    };
    opts.iter()
        .position(|(_, v)| *v == cur)
        // 反查不到就落到第一项。宁可让用户重选，也不伪造一个「看起来对」的名字 ——
        // 比如备份里还原进来一个未知按键码，显示成「左键点击」就是骗人。
        .unwrap_or(0)
}

/// 按键目标码 → 显示名。
///
/// `proto::BUTTON_SRC` 只有 8 个**源**键的码，没有目标码表（`cli.rs` 里也没有），
/// 所以这张表在这里定义。值来自抓包确认的目标码；不认识的码原样显示成十六进制，
/// 不猜 —— 猜错会让用户把键绑到别处去。
pub fn button_target_name(code: u16) -> String {
    match code {
        0x0000 => "无功能".to_string(),
        0x01F0 => "左键点击".to_string(),
        0x01F1 => "右键点击".to_string(),
        0x01F2 => "中键点击".to_string(),
        0x01E4 => "浏览器后退".to_string(),
        0x01E5 => "浏览器前进".to_string(),
        0x01E6 => "切换 DPI 档位".to_string(),
        0x01E8 => "滚轮上滚".to_string(),
        0x01E9 => "滚轮下滚".to_string(),
        other => format!("0x{other:04X}"),
    }
}

/// 按键页下拉框的候选目标码。
///
/// **只列抓包确认过的码**，外加 `0x0000`（无功能）。`button_target_name` 对未知码
/// 会原样显示成十六进制 —— 从备份还原进来的未知码因此仍能正确显示，
/// 但它不在候选表里，下拉打开时选不中任何一项（`unwrap_or(0)` 落到「无功能」）。
/// 这是有意的：宁可显示成「无功能」让用户重选，也不要伪造一个名字。
pub const BUTTON_TARGETS: [u16; 9] = [
    0x0000, 0x01F0, 0x01F1, 0x01F2, 0x01E4, 0x01E5, 0x01E6, 0x01E8, 0x01E9,
];

/// 某个下拉框的候选项（文字, 值）。
pub fn options(drop: Drop, d: &Data) -> &'static [(String, usize)] {
    // 候选文字目前不依赖 `d`，保留参数是为了将来加「当前值」标记时不用改所有调用点。
    let _ = d;
    raw_options(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 候选表按 `Drop` 缓存：同一个下拉两次取到的必须是同一份内存，
    /// 不同下拉不能串表（槽位下标写错就会串）。
    #[test]
    fn raw_options_is_cached_not_rebuilt() {
        let a = raw_options(Drop::Poll);
        let b = raw_options(Drop::Poll);
        assert!(std::ptr::eq(a, b), "候选表应当复用同一份，不该每次重建");
        assert_eq!(a.len(), 4);
        assert!(
            !std::ptr::eq(raw_options(Drop::Sleep), raw_options(Drop::Poll)),
            "不同下拉必须落到不同槽位"
        );
        assert!(raw_options(Drop::Palette(0)).is_empty(), "色盘走单独一支");
    }

    /// 每个下拉的「第 i 项」都要能翻成设备值，且项数与候选表一致；
    /// `None` 只允许出现在越界下标上。
    #[test]
    fn option_value_agrees_with_option_count() {
        for drop in [
            Drop::Sleep,
            Drop::Poll,
            Drop::Stage,
            Drop::Debounce,
            Drop::Lod,
            Drop::LightMode,
            Drop::CloseMode,
        ] {
            let n = raw_options(drop).len();
            assert!(n > 0, "{drop:?} 候选表为空");
            for i in 0..n {
                assert!(option_value(drop, i).is_some(), "{drop:?} 第 {i} 项无值");
            }
            assert!(option_value(drop, n).is_none(), "{drop:?} 越界下标应当无值");
        }
    }

    /// 按键目标码都要有中文名字，且不能是空串或「未知」。
    #[test]
    fn every_button_target_has_a_name() {
        for code in BUTTON_TARGETS {
            let name = button_target_name(code);
            assert!(!name.trim().is_empty(), "{code:#06x} 没有名字");
            assert!(!name.contains("未知"), "{code:#06x} 落到未知分支");
        }
    }
}

