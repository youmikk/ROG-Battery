//! 布局常量；响应式断点按窗口扣除侧栏后的可用宽度计算。

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

/// 侧栏展开宽度。
pub const NAV_W_EXPANDED: f32 = 196.0;
/// 侧栏收成图标轨道时的宽度（CSS `--sb-w: 56px`）。
pub const NAV_W_RAIL: f32 = 56.0;

/// 内容区左右内边距（CSS `.page-body { padding: 6px 40px }`）。
pub const PAGE_PAD_X: f32 = 40.0;
/// 内容区顶部内边距（CSS `padding-top: 6px`）。
pub const PAGE_PAD_TOP: f32 = 6.0;
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
/// 导航项高度。
pub const NAV_ITEM_H: f32 = 42.0;

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
    /// 按**内容区**（已扣掉侧栏）的宽度判断。
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

    /// 电量页 KPI 是两列还是一列。
    #[cfg(test)]
    pub fn kv_two_cols(self) -> bool {
        !matches!(self, Breakpoint::Narrow | Breakpoint::Tiny)
    }

    /// 电量环直径（CSS `.ring { width: 118px }`，窄屏 92）。
    #[cfg(test)]
    pub fn ring_diameter(self) -> f32 {
        match self {
            Breakpoint::Wide | Breakpoint::Mid => 118.0,
            _ => 92.0,
        }
    }

    /// 设备信息卡是放在电量卡右边（`hero` 横排）还是下面（竖排）。
    ///
    /// CSS 在 880px 时改成 `flex-direction: column`。
    #[cfg(test)]
    pub fn hero_side_by_side(self) -> bool {
        matches!(self, Breakpoint::Wide | Breakpoint::Mid)
    }
}

/// 一次布局的结果：各处可用的矩形。
///
/// 页面绘制函数收这个结构，而不是自己算「标题下面多少像素」——
/// 五页共用同一套垂直节奏，滚动的位置才对得上。
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// 内容区（已扣掉侧栏和内边距），从页面顶部 y=0 开始（不是客户区坐标）。
    pub content: super::gfx::R,
    /// 内容区实际可用宽度（≤ `PAGE_MAX_W`）。
    pub width: f32,
    #[cfg(test)]
    pub bp: Breakpoint,
}

impl Frame {
    /// 按客户区尺寸和侧栏宽度算布局。
    ///
    /// `w`/`h` 是画布的客户区尺寸，`nav_w` 是侧栏占的宽度。
    pub fn new(w: f32, h: f32, nav_w: f32) -> Frame {
        let avail = (w - nav_w).max(120.0);
        let bp = Breakpoint::of(avail);
        let pad = bp.pad_x();
        let width = (avail - pad * 2.0).min(PAGE_MAX_W).max(60.0);
        // 内容靠左排（不居中）：Windows 11 设置页就是左对齐的，
        // 而且居中后窗口一宽，左边缘就飘，和侧栏脱节。
        Frame {
            content: super::gfx::R::new(pad, PAGE_PAD_TOP, width, h),
            width,
            #[cfg(test)]
            bp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 断点按内容宽度切换() {
        assert_eq!(Breakpoint::of(1200.0), Breakpoint::Wide);
        assert_eq!(Breakpoint::of(900.0), Breakpoint::Wide);
        assert_eq!(Breakpoint::of(899.0), Breakpoint::Mid);
        assert_eq!(Breakpoint::of(700.0), Breakpoint::Mid);
        assert_eq!(Breakpoint::of(699.0), Breakpoint::Narrow);
        assert_eq!(Breakpoint::of(560.0), Breakpoint::Narrow);
        assert_eq!(Breakpoint::of(559.0), Breakpoint::Tiny);
        assert_eq!(Breakpoint::of(0.0), Breakpoint::Tiny);
    }

    #[test]
    fn 内边距随断点收窄() {
        assert_eq!(Breakpoint::Wide.pad_x(), 40.0);
        assert_eq!(Breakpoint::Mid.pad_x(), 26.0);
        assert_eq!(Breakpoint::Narrow.pad_x(), 18.0);
        assert_eq!(Breakpoint::Tiny.pad_x(), 14.0);
        // 必须单调收窄，否则宽屏比窄屏还挤
        assert!(Breakpoint::Wide.pad_x() > Breakpoint::Mid.pad_x());
        assert!(Breakpoint::Mid.pad_x() > Breakpoint::Narrow.pad_x());
        assert!(Breakpoint::Narrow.pad_x() > Breakpoint::Tiny.pad_x());
    }

    #[test]
    fn 窄屏kpi改一列_电环缩小_设备卡移到下面() {
        assert!(Breakpoint::Wide.kv_two_cols());
        assert!(Breakpoint::Mid.kv_two_cols());
        assert!(!Breakpoint::Narrow.kv_two_cols());
        assert!(!Breakpoint::Tiny.kv_two_cols());

        assert_eq!(Breakpoint::Wide.ring_diameter(), 118.0);
        assert_eq!(Breakpoint::Tiny.ring_diameter(), 92.0);

        assert!(Breakpoint::Wide.hero_side_by_side());
        assert!(!Breakpoint::Narrow.hero_side_by_side());
    }

    #[test]
    fn frame_在1100窗口展开侧栏下的宽度() {
        // 1100 宽窗口，侧栏展开 196
        let f = Frame::new(1100.0, 740.0, NAV_W_EXPANDED);
        // 可用宽度 904，内边距 40。
        assert_eq!(f.bp, Breakpoint::Wide);
        assert_eq!(f.content.x, 40.0);
        assert_eq!(f.width, 904.0 - 80.0);
    }

    #[test]
    fn frame_在1100窗口收成轨道下的宽度() {
        // 侧栏收成 56：可用 1044，仍受 PAGE_MAX_W 限制
        let f = Frame::new(1100.0, 740.0, NAV_W_RAIL);
        assert_eq!(f.bp, Breakpoint::Wide);
        assert_eq!(f.width, PAGE_MAX_W, "宽屏时内容不该超过 940");
        assert_eq!(f.content.x, 40.0);
    }

    #[test]
    fn frame_极窄窗口不会算出负宽度() {
        // 窗口被拖到 200px：扣掉侧栏和内边距后必须还有正的宽度，
        // 否则后面所有布局都变成负尺寸（GDI+ 画反向矩形会静默不画）。
        let f = Frame::new(200.0, 400.0, NAV_W_RAIL);
        assert!(f.width > 0.0, "宽度算成了 {}", f.width);
        assert!(f.width >= 60.0);
    }

    #[test]
    fn min_window_size_scales_with_dpi() {
        assert_eq!(min_window_size(1.0), (720, 520));
        assert_eq!(min_window_size(1.5), (1080, 780));
        // 传 0 / 负数时退回 1.0，别让窗口变成 0×0。
        assert_eq!(min_window_size(0.0), (720, 520));
    }
}
