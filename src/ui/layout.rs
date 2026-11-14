//! 布局常量；内容居中，页面标题紧贴窗口标题栏。

/// 窗口能缩到的最小逻辑尺寸。
pub const MIN_WIN_W: f32 = 720.0;
pub const MIN_WIN_H: f32 = 520.0;

/// 最小窗口尺寸（物理像素）。
pub fn min_window_size(scale: f32) -> (i32, i32) {
    let s = if scale > 0.1 { scale } else { 1.0 };
    (
        (MIN_WIN_W * s).round() as i32,
        (MIN_WIN_H * s).round() as i32,
    )
}

pub const CONTENT_TOP: f32 = super::titlebar::TITLE_H;

/// 内容区左右内边距（CSS `.page-body { padding: 6px 40px }`）。
pub const PAGE_PAD_X: f32 = 40.0;
/// 内容区顶部内边距（CSS `padding-top: 6px`）。
pub const PAGE_PAD_TOP: f32 = 6.0;
/// 设置页标题和分类固定在内容上方。
pub const SETTINGS_BODY_TOP: f32 = CONTENT_TOP + PAGE_PAD_TOP + 108.0;
/// 内容区底部额外留白（CSS `padding-bottom: 240px`）。
///
/// 网页里这个值是为了「滚到底时最后一个区块能到顶部」。桌面窗口不需要
/// 那个效果（没有滚动跟随高亮了），所以这里只留一点点，
/// 否则用户滚到底会看到一大片空白以为还有内容。
pub const PAGE_PAD_BOTTOM: f32 = 24.0;
/// 内容区最大宽度（CSS `max-width: 940px`）。
///
/// 窗口很宽时内容不该跟着拉到 2000px —— 那样一行字长得没法读。
pub const PAGE_MAX_W: f32 = 940.0;

/// 页面标题 `h1` 的占位高度（字号见 [`font::H1`]）。
pub const H1_H: f32 = 40.0;
/// 区块标题 `h2` 的占位高度（字号见 [`font::H2`]）。
pub const H2_H: f32 = 32.0;

/// 卡片圆角。
pub const R_CARD: f32 = 13.0;
/// 控件圆角（CSS `--radius-ctl: 4px`）。
pub const R_CTL: f32 = 4.0;

/// 设置行的最小高度（CSS `.row { min-height: 62px }`）。
pub const ROW_H: f32 = 62.0;

/// 字号表。集中在一处，「说明文字」到底 11 还是 11.5 不用每个页面猜。
pub mod font {
    /// 页面标题。
    pub const H1: f32 = 26.0;
    /// 区块标题。
    pub const H2: f32 = 14.0;
    /// 正文 / 设置项标题。
    pub const BODY: f32 = 13.5;
    /// 说明文字。
    pub const HINT: f32 = 11.5;
    /// 胶囊标签、单位。
    pub const TINY: f32 = 11.0;
}

/// 当前窗口的响应式档位。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Breakpoint {
    /// 宽屏：内容区够 940px，全部按原稿排。
    Wide,
    /// 中屏：内容区约 700–940，左右内边距收到 26。
    Mid,
    /// 窄屏：内容区 <700，KPI 两列改一列，电环缩小。
    Narrow,
    /// 极窄：内容区 <560，卡片内边距也收。
    Tiny,
}

impl Breakpoint {
    /// 按窗口宽度判断。
    pub fn of(content_w: f32) -> Breakpoint {
        if content_w >= 900.0 {
            Breakpoint::Wide
        } else if content_w >= 700.0 {
            Breakpoint::Mid
        } else if content_w >= 560.0 {
            Breakpoint::Narrow
        } else {
            Breakpoint::Tiny
        }
    }

    /// 内容区左右内边距。
    pub fn pad_x(self) -> f32 {
        match self {
            Breakpoint::Wide => PAGE_PAD_X,
            Breakpoint::Mid => 26.0,
            Breakpoint::Narrow => 18.0,
            Breakpoint::Tiny => 14.0,
        }
    }

}

/// 一次布局的结果：各处可用的矩形。
///
/// 页面绘制函数收这个结构，而不是自己算「标题下面多少像素」——
/// 五页共用同一套垂直节奏，滚动的位置才对得上。
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// 内容区，使用窗口坐标。
    pub content: super::gfx::R,
    pub viewport: super::gfx::R,
    /// 内容区实际可用宽度（≤ `PAGE_MAX_W`）。
    pub width: f32,
}

impl Frame {
    pub fn new(w: f32, h: f32) -> Frame {
        let avail = w.max(120.0);
        let bp = Breakpoint::of(avail);
        let pad = bp.pad_x();
        let width = (avail - pad * 2.0).min(PAGE_MAX_W).max(60.0);
        let top = CONTENT_TOP + PAGE_PAD_TOP;
        Frame {
            content: super::gfx::R::new((avail - width) / 2.0, top, width, (h - top).max(0.0)),
            viewport: super::gfx::R::new(0.0, CONTENT_TOP, avail, (h - CONTENT_TOP).max(0.0)),
            width,
        }
    }
}
