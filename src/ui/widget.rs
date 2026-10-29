//! 控件模型：命中区域、交互状态、动作。
//!
//! # 为什么用「即时模式」而不是控件树
//!
//! nwg 那套是保留模式：建控件 → 保存句柄 → 收到事件时反查是哪个控件。
//! 换成自绘后这条路走不通了 —— 没有系统控件，也就没有「控件句柄」可查。
//!
//! 这里用即时模式（immediate mode）：
//!
//! 1. 每帧绘制时，画到哪个可交互元素就把它的矩形 + 动作登记进
//!    [`Ui::hots`]；
//! 2. 鼠标事件到来时，从后往前扫这个列表（后画的在上层，比如下拉弹层）
//!    做命中测试；
//! 3. 命中的动作用 [`Act`] 表达，交给上层处理。
//!
//! 这样**画出来的东西和能点的东西永远是同一份代码算的**。保留模式最容易
//! 出的 bug 是「控件挪了位置但命中区没挪」，即时模式天然没有这个问题。
//!
//! # 为什么动作是数据不是闭包
//!
//! `Act` 是 `Copy` 的纯数据。好处：可以在没有 Win32 的环境里单测
//! 「点 (x,y) 应该产生哪个 Act」，而不用起窗口。这是这一层最大的价值 ——
//! 布局算错、命中区重叠这类 bug 全部能在 `cargo test` 里抓到。

#![allow(dead_code)]

use super::gfx::R;

/// 五个页面。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Battery,
    Perf,
    Light,
    Keys,
    Settings,
}

impl Page {
    pub const ALL: [Page; 5] = [
        Page::Battery,
        Page::Perf,
        Page::Light,
        Page::Keys,
        Page::Settings,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Page::Battery => "电量",
            Page::Perf => "性能",
            Page::Light => "灯光",
            Page::Keys => "按键",
            Page::Settings => "设置",
        }
    }

    /// 侧栏图标用了 1.5px 线宽的几何图形，这里给一个代号。
    pub fn icon(self) -> Icon {
        match self {
            Page::Battery => Icon::Battery,
            Page::Perf => Icon::Perf,
            Page::Light => Icon::Light,
            Page::Keys => Icon::Keys,
            Page::Settings => Icon::Settings,
        }
    }
}

/// 侧栏图标。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Battery,
    Perf,
    Light,
    Keys,
    Settings,
}

/// 下拉框的种类。
///
/// 每个种类对应一个「当前值 + 候选列表」，页面的 `pick` 逻辑按种类分派。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Drop {
    /// 电量页：自动休眠档位（`setsleep`）。
    Sleep,
    /// 电量页 / 性能页共用的「充满效果」其实是开关，不是下拉，故不在此。
    /// 性能页：回报率（`setpoll`）。
    Poll,
    /// 性能页：当前 DPI 档位（`setstage`）。
    Stage,
    /// 性能页：去抖（`setdebounce`）。
    Debounce,
    /// 性能页：抬起距离（`setlod`）。
    Lod,
    /// 灯光页：灯效模式（`setlight`）。
    LightMode,
    /// 设置页：后台刷新间隔（本地，不写设备）。
    Refresh,
    /// 设置页：快照缓存有效期（本地）。
    CacheTtl,
    /// 按键页：第 N 槽的绑定目标（`0`–`7`）。
    ///
    /// 单独开一个变体而不是塞进 `LightMode` 当假值，是因为按键页有 8 个下拉框，
    /// 每个的当前值和候选集都不同 —— `Drop` 是**无参数**枚举，一个变体对不了
    /// 8 个槽位。（灯光页的「方向」借用 `LightMode` + 100 是另一回事：
    /// 那里只有一个假值，用哨兵够用，但那种写法不该扩散。）
    ButtonTarget(u8),
    /// 颜色**色盘**。`u8` 是槽位：`0..=3` = 对应 DPI 档的指示灯颜色，
    /// `LIGHT_COLOR_SLOT(255)` = 灯光颜色。
    ///
    /// 为什么不用「点色块直接轮换下一个颜色」：用户明确说要**色盘**
    ///（能看见所有候选色再挑），轮换式他得点十几次才能回到想要的那个。
    ///
    /// 弹层由 [`super::pages`] 的 `draw_palette` 画成网格，不是普通的
    /// 一列文字项 —— 色块摆成网格才看得出色相分布。
    Palette(u8),
    /// 设置页：窗口关闭行为（每次询问 / 最小化到托盘 / 直接退出）。
    CloseMode,
}

/// [`Drop::Palette`] 里代表「灯光颜色」的槽位值。
///
/// 0..=3 被 DPI 四档占用，所以灯光用 255 而不是 4 —— 两者在语义上完全无关，
/// 将来 DPI 档数变多时 255 也不会撞上。
pub const LIGHT_COLOR_SLOT: u8 = 255;

/// 文本输入框的种类。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edit {
    /// 性能页：第 N 档 DPI 的数值（0–3）。
    Dpi(u8),
    /// 性能页：第 N 档 DPI 的颜色（0–3），6 位十六进制。
    DpiColor(u8),
    /// 电量页：低电量阈值（0–100）。
    LowWarn,
    /// 灯光页：亮度（0–100）。
    LightBright,
    /// 灯光页：颜色（6 位十六进制）。
    LightColor,
    /// 灯光页：速度（0–100）。
    LightSpeed,
    /// 按键页：第 N 槽的绑定码（4 位十六进制）。
    Button(u8),
}

impl Edit {
    /// 这个输入框允许哪些字符。
    pub fn allows(self, ch: char) -> bool {
        match self {
            Edit::DpiColor(_) | Edit::LightColor => ch.is_ascii_hexdigit(),
            Edit::Button(_) => ch.is_ascii_hexdigit(),
            // DPI 最大值 42000，5 位；阈值/亮度/速度都是 3 位以内
            _ => ch.is_ascii_digit(),
        }
    }

    /// 最多几位。
    pub fn max_len(self) -> usize {
        match self {
            Edit::Dpi(_) => 5,
            Edit::DpiColor(_) | Edit::LightColor => 6,
            Edit::Button(_) => 4,
            Edit::LowWarn => 3,
            Edit::LightBright | Edit::LightSpeed => 3,
        }
    }
}

/// 滑块。
///
/// `#[repr(usize)]` + 显式判别值：`live` 数组用它当下标，靠「声明顺序恰好
/// 是 0,1,2」是**隐含约定**，以后有人插一个新滑块进来就会静默错位（表现是
/// 拖 A 滑块 B 跟着动）。写出来之后，位置由这里的数字决定。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
pub enum Slide {
    /// 灯光亮度（0–100）。
    LightBright = 0,
    /// 角度微调（−20..20）。
    Tune = 1,
    /// 灯光速度（0–100）。
    LightSpeed = 2,
}

impl Slide {
    /// `live` 数组里的下标。
    pub fn idx(self) -> usize {
        self as usize
    }
}

/// 本地偏好（不写设备，写 `settings.ini`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pref {
    TrayEnabled,
    CloseToTray,
    AutoStart,
    /// 电量页：充满效果开关（这个**是**写设备的）。
    FullEffect,
    /// 性能页：角度吸附开关。
    AngleSnap,
    /// 灯光页：随机颜色开关。
    LightRandom,
}

/// 一个可以点的东西要做的事。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Act {
    /// 切页。
    Nav(Page),
    /// 侧栏展开 / 收成图标轨道。
    ToggleRail,
    /// 浅色 / 深色切换。
    ToggleTheme,
    /// 电量页的「立即刷新」。
    Refresh,
    /// 打开一个下拉弹层。
    OpenDrop(Drop),
    /// 在下拉弹层里选第 `index` 项。
    PickDrop(Drop, usize),
    /// 关闭下拉弹层（点别处）。
    CloseDrop,
    /// 把焦点给某个输入框。
    Focus(Edit),
    /// 应用某个输入框的值（输入框旁边的「应用」，或回车）。
    ApplyEdit(Edit),
    /// 点一下开关。
    Toggle(Pref),
    /// 拖动滑块到某个比例（0–1），拖拽过程中连续触发。
    SetSlide(Slide, f32),
    /// 按下滑块。真正的新值由随后连续的 [`Act::SetSlide`] 给出，
    /// 这个动作只负责让 `Ui` 记住「现在拖的是哪一条」。
    SlidePress(Slide),
    /// 松手，滑块拖动结束。页面据此决定要不要把值写进设备
    ///（拖动过程中每一帧都写设备会打爆 HID 带宽）。
    CommitSlide(Slide),
    /// 按键页：把某一槽恢复成默认值。
    ResetButton(u8),
    /// 折叠面板开合。0 = 性能页「高级」，1 = 灯光页「高级」。
    ToggleAdv(u8),
    /// 指定配色偏好。设置页的「外观」分段控件用：
    /// 0 = 跟随系统，1 = 浅色，2 = 深色。
    SetTheme(u8),
    /// 设置页：备份到文件。
    Backup,
    /// 设置页：从文件还原。
    Restore,
    /// 设置页：打开历史记录所在文件夹。
    OpenHistoryFolder,
    /// 设置页：更改历史文件保存位置。
    ChangeHistoryPath,
    /// 设置页：恢复历史文件默认位置。
    ResetHistoryPath,
    /// 设置页：更改设置文件保存位置。
    ChangeSettingsPath,
    /// 设置页：恢复设置文件默认位置。
    ResetSettingsPath,
    /// 设置页：打开设置文件所在文件夹。
    OpenSettingsFolder,
    /// 设置页：清空历史记录。
    ClearHistory,
    /// 设置页「关于」：打开开源仓库。
    OpenRepo,
    /// 设置页「关于」：打开 Microsoft Store 产品页。
    OpenStore,
    /// 灯光页：打开 Windows「动态光效」设置页（`ms-settings:personalization-lighting`）。
    OpenLightingSettings,
    /// 灯光页：重新探测 HID LampArray 设备。
    RefreshLighting,
    /// 切换色盘模式（0 = 常用预设，1 = 自定义色环）。
    SetPaletteTab(u8),
    /// 点击色环上的色相角度。
    SetCustomHue(Drop, f32),
    /// 切换关闭确认对话框的选项（最小化/退出）。
    SelectCloseChoice(CloseChoice),
    /// 切换关闭确认对话框「记住我的选择」复选框。
    ToggleCloseRemember,
    /// 确认关闭对话框。
    ConfirmCloseDialog,
    /// 取消关闭对话框。
    CancelCloseDialog,
    /// 设置窗口关闭模式：0=每次询问，1=最小化到托盘，2=直接退出。
    SetCloseMode(u8),
    /// 键盘焦点在输入框里按了回车 —— 和 `ApplyEdit` 等价，
    /// 但要用当前编辑中的文本，不是已提交的值。
    None,
}

/// 关闭窗口对话框里的单选项。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CloseChoice {
    /// 最小化到系统托盘
    Minimize,
    /// 直接退出程序
    Quit,
}

/// 一个可交互区域。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Hot {
    pub rect: R,
    pub act: Act,
}

/// 鼠标 / 键盘的即时状态。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Pointer {
    /// 逻辑像素、客户区坐标（**内容坐标要自己加滚动量**）。
    pub x: f32,
    pub y: f32,
    pub down: bool,
    /// 拖拽中的滑块。`Some` 时鼠标移动会持续产生 `SetSlide`。
    pub dragging: Option<Slide>,
}

/// 鼠标指针形状。canvas 按它去 `LoadCursorW`。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cursor {
    Arrow,
    Hand,
    IBeam,
}

/// 界面的瞬时状态（不进 `settings.ini`，程序关了就没）。
pub struct Ui {
    pub page: Page,
    /// 侧栏是否收成 56px 图标轨道。用户定的是**默认收起**。
    pub rail: bool,
    /// 当前配色。`settings.ini` 的 `theme` 键存它的字符串形式。
    pub mode: super::theme::Mode,
    /// 配色偏好：0 = 跟随系统，1 = 浅色，2 = 深色。设置页分段控件用。
    pub theme_pref: u8,
    /// 折叠面板是否展开。0 = 性能页「高级」，1 = 灯光页「高级」。
    ///
    /// 收起时它的内容完全不参与 y 坐标推进，所以开合会改变整页高度 ——
    /// 这正是想要的效果。
    pub adv: [bool; 2],
    /// 每个页面各自的滚动位置。切页时保留，切回来还是原位。
    scroll: [f32; 5],
    /// 每个页面的内容总高度（绘制时写入，滚轮要靠它夹住）。
    content_h: [f32; 5],
    /// 当前打开的下拉弹层。
    open_drop: Option<Drop>,
    /// 下拉弹层展开后的矩形（绘制时写入，命中测试要用）。
    drop_rect: Option<R>,
    /// 触发弹层的那个下拉框本身的矩形（内容坐标）。
    ///
    /// 弹层默认贴在它下面，但**下面放不下时要翻到上面**（去抖有 8 项、
    /// 高 248px，行一靠底就会有一半选项落到视口外面，表现为「高的能选、
    /// 低的点不到」）。翻上翻下都得知道按钮在哪。
    drop_btn: Option<R>,
    /// 正在编辑的输入框。
    pub focus: Option<Edit>,
    /// 各输入框的编辑中文本。
    ///
    /// **用列表不用单个 `String`**：DPI 一行里有两个输入框（数值 + 颜色），
    /// 一个「应用」按钮要同时读两个。单个缓冲区表达不了「两个字段各有草稿」。
    ///
    /// 用 `Vec` 而不是 `HashMap`：`Edit` 没实现 `Hash`，而这里最多十来个元素，
    /// 线性查找比哈希更快也更好读。
    pub edits: Vec<(Edit, String)>,
    /// 「草稿还是设备原值的副本、用户一个字符都没改过」的那个输入框。
    ///
    /// 有它才能做到**第一次敲键就替换掉预填值**：DPI 框点进去时草稿是
    /// `400`（设备当前值），用户想改成 `1600`，直接敲 `1` 得到的是
    /// `4001` 而不是 `1` —— 这是最常被吐槽的一处。看到 `Some(e)` 就把
    /// 草稿清空再写第一个字符，语义等于「全选后输入」。
    ///
    /// 退格/删字之后必须清掉：那时草稿已经是用户改过的内容了。
    pub edit_fresh: Option<Edit>,
    /// 本帧登记的内容区可交互区域（**内容坐标**）。
    pub hots: Vec<Hot>,
    /// 本帧登记的侧栏可交互区域（**窗口坐标**，不随滚动移动）。
    ///
    /// 两套坐标不能混在一个列表里：侧栏钉在窗口上，内容区跟着 `scroll` 走。
    pub chrome: Vec<Hot>,
    pub dialog_hots: Vec<Hot>,
    /// 鼠标位置 / 按下状态。
    pub pointer: Pointer,
    /// 鼠标悬停的 Act（用于高亮）。
    pub hover: Option<Act>,
    /// 鼠标按下的 Act（松开时若还在同一个上才算点击）。
    pub pressed: Option<Act>,
    /// 本次按下命中的滑块。存下来是因为 `pressed` 里那个 `SetSlide(s, frac)`
    /// 的分量只在按下那一瞬间有意义，后面拖动时要的是「哪一条滑块」。
    pub pressed_slide: Option<Slide>,
    /// 滑块拖拽中的临时值（0.0–1.0），下标是 [`Slide`] 的 `as usize`。
    ///
    /// # 为什么需要这一层
    ///
    /// 页面画滑块时读的是**设备状态**（`d.state`）。拖拽期间鼠标已经移开了，
    /// 写设备却要几十毫秒、读回还得再来一轮 —— 中间那几帧滑块会卡在原地不动，
    /// 松手才「跳」到新位置。`None` = 这一条没有正在拖，页面回落到设备值。
    pub live: [Option<f32>; 3],
    /// 状态栏文字（右下角/底部浮层）。
    pub status: String,
    /// 状态栏提示的触发时间（用于自动在 ~2s 后消失）。
    pub status_time: Option<std::time::Instant>,
    /// 色盘模式：0 = 常用预设，1 = 自定义色环。
    pub palette_tab: u8,
    /// 自定义色环当前选中的色相（0.0 .. 360.0）。
    pub custom_hue: f32,
    /// 自绘标题栏上正悬停的按钮。`None` = 没悬停在任何按钮上。
    ///
    /// 由 `canvas.rs` 的 `WM_MOUSEMOVE` 写（那里鼠标进 canvas 子窗口之后
    /// 就收不到「标题栏区域」的移动了 —— 标题栏在 canvas 之上），
    /// 由 `draw_frame` 读。
    ///
    /// 命中测试和按钮点击走的是主窗口的 `WM_NCHITTEST`（`frame.rs`），
    /// 这里只管「画哪一条高亮」。
    pub hover_btn: Option<crate::ui::titlebar::Btn>,
    /// 是否弹出「关闭窗口」确认对话框。
    pub show_close_dialog: bool,
    /// 关闭对话框里的单选项。
    pub close_choice: CloseChoice,
    /// 关闭对话框里的「记住我的选择」复选框。
    pub close_remember: bool,
}

impl Default for Ui {
    fn default() -> Self {
        Ui {
            page: Page::Battery,
            // 用户 m09924：「默认收缩模式，切换收缩的功能放在左侧功能栏最下方」
            rail: true,
            // 用户 m09701：「默认浅色页面吧」
            mode: super::theme::Mode::Light,
            // 默认「跟随系统」
            theme_pref: 0,
            // 性能页的「高级」默认收起（原型 `<details>` 没写 open）；
            // 灯光页的默认展开（原型写了 `open`）。
            adv: [false, true],
            scroll: [0.0; 5],
            content_h: [0.0; 5],
            open_drop: None,
            drop_rect: None,
            drop_btn: None,
            focus: None,
            edits: Vec::new(),
            edit_fresh: None,
            hots: Vec::new(),
            chrome: Vec::new(),
            dialog_hots: Vec::new(),
            pointer: Pointer::default(),
            hover: None,
            pressed: None,
            pressed_slide: None,
            live: [None; 3],
            status: String::new(),
            status_time: None,
            palette_tab: 0,
            custom_hue: 0.0,
            hover_btn: None,
            show_close_dialog: false,
            close_choice: CloseChoice::Minimize,
            close_remember: false,
        }
    }
}

impl Ui {
    fn page_index(p: Page) -> usize {
        match p {
            Page::Battery => 0,
            Page::Perf => 1,
            Page::Light => 2,
            Page::Keys => 3,
            Page::Settings => 4,
        }
    }

    pub fn scroll_y(&self, p: Page) -> f32 {
        self.scroll[Self::page_index(p)]
    }

    pub fn scroll_mut(&mut self, p: Page) -> &mut f32 {
        &mut self.scroll[Self::page_index(p)]
    }

    /// 设置提示信息并记录触发时间戳，供自动淡出消退。
    pub fn set_status(&mut self, text: impl Into<String>) {
        let s = text.into();
        self.status_time = if s.trim().is_empty() {
            None
        } else {
            Some(std::time::Instant::now())
        };
        self.status = s;
    }

    /// 立即清空提示信息。
    pub fn clear_status(&mut self) {
        self.status.clear();
        self.status_time = None;
    }

    /// 绘制结束时报上这一页的内容总高度，滚轮要用它夹住。
    pub fn set_content_h(&mut self, p: Page, h: f32) {
        self.content_h[Self::page_index(p)] = h;
    }

    pub fn content_h(&self, p: Page) -> f32 {
        self.content_h[Self::page_index(p)]
    }

    /// 开始新一帧：清空登记的命中区。
    pub fn begin_frame(&mut self) {
        self.hots.clear();
        self.chrome.clear();
        self.dialog_hots.clear();
        self.drop_rect = None;
        self.drop_btn = None;
    }

    /// 登记一个内容区可交互区域（**内容坐标**，随滚动移动）。
    pub fn hot(&mut self, rect: R, act: Act) {
        self.hots.push(Hot { rect, act });
    }

    /// 登记一个侧栏可交互区域（**窗口坐标**，不随滚动移动）。
    pub fn chrome_hot(&mut self, rect: R, act: Act) {
        self.chrome.push(Hot { rect, act });
    }

    /// 登记一个模态弹窗可交互区域（**窗口坐标**）。
    pub fn dialog_hot(&mut self, rect: R, act: Act) {
        self.dialog_hots.push(Hot { rect, act });
    }

    /// 登记一个下拉弹层的整体矩形。绘制弹层时调用。
    pub fn set_drop_rect(&mut self, r: R) {
        self.drop_rect = Some(r);
    }

    pub fn drop_rect(&self) -> Option<R> {
        self.drop_rect
    }

    /// 登记触发弹层的下拉框矩形（`draw_drop` 用它决定往上还是往下开）。
    pub fn set_drop_btn(&mut self, r: R) {
        self.drop_btn = Some(r);
    }

    pub fn drop_btn(&self) -> Option<R> {
        self.drop_btn
    }

    pub fn open_drop(&self) -> Option<Drop> {
        self.open_drop
    }

    pub fn set_open_drop(&mut self, d: Option<Drop>) {
        self.open_drop = d;
    }

    // ---- 输入框草稿 ----

    /// 取某个输入框的编辑中文本。没有草稿时返回 `None`（调用方该显示设备当前值）。
    pub fn edit_text(&self, e: Edit) -> Option<&str> {
        self.edits
            .iter()
            .find(|(k, _)| *k == e)
            .map(|(_, v)| v.as_str())
    }

    /// 开始编辑：把设备当前值抄进草稿。
    ///
    /// **已经有草稿时绝不覆盖** —— 用户可能敲了一半、点了别处、又点回来，
    /// 这时用设备值覆盖就把他的字吃掉了一半。
    ///
    /// 注意这里**不是**「`focus == Some(e)` 就返回」：`on_press` 命中
    /// `Act::Focus(e)` 时已经把 `focus` 设好了，用那个条件会让草稿永远建不出来，
    /// 表现是「点进输入框后敲字没反应」。
    pub fn begin_edit(&mut self, e: Edit, initial: String) {
        self.focus = Some(e);
        if self.edits.iter().any(|(k, _)| *k == e) {
            return;
        }
        self.edits.push((e, initial));
        // 新建的草稿 = 设备原值的副本，还没被用户碰过 → 第一次敲键要替换。
        self.edit_fresh = Some(e);
    }

    /// 敲一个字符。超长或不合法时忽略并返回 `false`。
    pub fn type_char(&mut self, ch: char) -> bool {
        let Some(e) = self.focus else {
            return false;
        };
        if !e.allows(ch) {
            return false;
        }
        let Some(slot) = self.edits.iter_mut().find(|(k, _)| *k == e) else {
            return false;
        };
        // 草稿还是原值副本 → 第一个字符直接替换整段（等价于「全选后输入」）。
        // 少了这一步，点进 `400` 想改成 `1600` 会得到 `4001`。
        if self.edit_fresh == Some(e) {
            slot.1.clear();
            self.edit_fresh = None;
        }
        // 只按字符数限长：DPI 颜色是 6 位 hex、按键是 4 位 hex，都是 ASCII，
        // 字符数 = 字节数，不会出现截断半个多字节字符的问题。
        if slot.1.chars().count() >= e.max_len() {
            return false;
        }
        slot.1.push(ch);
        true
    }

    /// 退格。返回是否真的删掉了东西。
    pub fn backspace(&mut self) -> bool {
        let Some(e) = self.focus else {
            return false;
        };
        let Some(slot) = self.edits.iter_mut().find(|(k, _)| *k == e) else {
            return false;
        };
        // 退格是在改原值了，不能再算「还没碰过」。
        if self.edit_fresh == Some(e) {
            self.edit_fresh = None;
        }
        slot.1.pop().is_some()
    }

    /// 丢掉某个输入框的草稿（提交成功 / 取消编辑后调用）。
    pub fn drop_edit(&mut self, e: Edit) {
        self.edits.retain(|(k, _)| *k != e);
        if self.edit_fresh == Some(e) {
            self.edit_fresh = None;
        }
        if self.focus == Some(e) {
            self.focus = None;
        }
    }

    /// 命中测试：返回最上层（最后登记）的那个区域。
    ///
    /// `scroll` 是当前页的滚动量。**所有登记区域都是内容坐标**
    ///（`hots` 和 `drop_rect` 都一样），所以比较时统一先 `y + scroll`
    /// 换成客户区坐标，再和鼠标位置比。
    ///
    /// 命中优先级：**弹层自己的选项 → 弹层的空白 → 内容区 → 侧栏**。
    ///
    /// 「弹层的空白」这一档不能省：弹层盖在页面上，如果它下面某一行的矩形
    /// 也包含这个点，少了这一档就会穿透去点到那行 —— 用户看到的是
    /// 「在下拉框里点了一下空白，结果改了个 DPI」。
    pub fn hit(&self, x: f32, y: f32, scroll: f32) -> Option<Act> {
        // 关闭确认模态对话框优先独占所有交互
        if self.show_close_dialog {
            for h in self.dialog_hots.iter().rev() {
                if h.rect.contains(x, y) {
                    return Some(h.act);
                }
            }
            return Some(Act::None);
        }

        let cy = y + scroll;
        let in_drop = self.drop_rect.map(|r| r.contains(x, cy)).unwrap_or(false);

        if in_drop {
            let dr = self.drop_rect.unwrap();
            // 只认**完全落在弹层矩形里**的登记项 —— 弹层的选项一定整个在里面，
            // 页面上的行/卡片一定伸到弹层外面，这样不用额外的标记位就能区分。
            for h in self.hots.iter().rev() {
                if !h.rect.contains(x, cy) {
                    continue;
                }
                let inside = h.rect.x >= dr.x
                    && h.rect.y >= dr.y
                    && h.rect.right() <= dr.right()
                    && h.rect.bottom() <= dr.bottom();
                if inside {
                    return Some(h.act);
                }
            }
            // 落在弹层范围内但没命中选项 = 点空白，吃掉它并当作关掉弹层
            return Some(Act::CloseDrop);
        }

        for h in self.hots.iter().rev() {
            if h.rect.contains(x, cy) {
                return Some(h.act);
            }
        }
        for h in self.chrome.iter().rev() {
            // 侧栏钉在窗口上，不随滚动移动 —— 这里用原始 `y`
            if h.rect.contains(x, y) {
                return Some(h.act);
            }
        }
        None
    }

    /// 鼠标滚轮。
    ///
    /// 返回是否需要重绘。滚动量夹在 `[0, content_h - view_h]` ——
    /// 不夹的话能滚出一片空白，看起来像程序没画完。
    pub fn wheel(&mut self, delta_ticks: f32, view_h: f32) -> bool {
        let p = self.page;
        let max = (self.content_h(p) - view_h).max(0.0);
        let s = self.scroll_mut(p);
        let before = *s;
        // 一格 53 逻辑像素（Win11 的默认滚动量），delta 为负表示向下
        *s = (*s - delta_ticks * 53.0).clamp(0.0, max);
        (*s - before).abs() > 0.01
    }

    /// 让某个区域滚进可视区（点输入框时保证光标可见）。
    pub fn ensure_visible(&mut self, rect: R, view_h: f32) -> bool {
        let p = self.page;
        let max = (self.content_h(p) - view_h).max(0.0);
        let s = self.scroll_mut(p);
        let before = *s;
        if rect.y < *s {
            *s = rect.y.clamp(0.0, max);
        } else if rect.bottom() > *s + view_h {
            *s = (rect.bottom() - view_h).clamp(0.0, max);
        }
        (*s - before).abs() > 0.01
    }

    // ---- 事件处理（canvas 的 WNDPROC 调这里） ----

    /// 侧栏当前宽度（逻辑像素）。
    pub fn nav_width(&self) -> f32 {
        if self.rail {
            super::layout::NAV_W_RAIL
        } else {
            super::layout::NAV_W_EXPANDED
        }
    }

    /// 鼠标左键按下。
    ///
    /// `(x, y)` 是客户区逻辑坐标，`scroll` 是当前页滚动量。命中到的东西写进
    /// `pressed`；如果是滑块，额外记进 `pressed_slide` 好让后续拖动有目标。
    pub fn on_press(&mut self, x: f32, y: f32, scroll: f32) {
        // 弹层开着时点别处 = 关掉弹层，且**不**让它穿透去点底下的控件。
        // 这是 Windows 下拉框的惯例：第一下只负责关。
        //
        // ⚠️ `drop_rect` 是**内容坐标**，这里必须和 `hit()` 一样先 `y + scroll`。
        // 原来直接拿客户区 `y` 去比：页面一滚动，弹层矩形在内容坐标里整体
        // 下移了 `scroll`，于是弹层**上半截**的点被判定成「点在外面」→
        // 只关弹层、吞掉这次点击。去抖的选项顺序是「关闭 / 8ms / … / 32ms」，
        // 上半截正好全是低值，表现就是「高的能选，选了高的之后低的不响应」。
        if self.open_drop.is_some() {
            let cy = y + scroll;
            let in_drop = self.drop_rect.map(|r| r.contains(x, cy)).unwrap_or(false);
            if !in_drop {
                self.open_drop = None;
                self.pressed = None;
                self.pressed_slide = None;
                return;
            }
        }
        let hit = self.hit(x, y, scroll);
        self.pressed = hit;
        self.pressed_slide = match hit {
            Some(Act::SetSlide(s, _)) => Some(s),
            _ => None,
        };
        // 输入框：按下即进编辑态，草稿初值由页面在绘制时用 `begin_edit` 给
        if let Some(Act::Focus(e)) = hit {
            self.focus = Some(e);
        }
    }

    /// 鼠标左键松开。返回「这次算一次点击」的动作。
    ///
    /// 只有按下和松开落在**同一项**上才算点击 —— 按下后拖走再松手不该触发，
    /// 这是 Windows 的惯例（否则会误触）。
    pub fn on_release(&mut self, x: f32, y: f32, scroll: f32) -> Option<Act> {
        let down = self.pressed.take()?;
        let up = self.hit(x, y, scroll);
        if up == Some(down) {
            Some(down)
        } else {
            None
        }
    }

    /// 鼠标移动后刷新悬停态。返回是否变了（变了才要重画）。
    pub fn refresh_hover(&mut self) -> bool {
        let scroll = self.scroll_y(self.page);
        let want = self.hit(self.pointer.x, self.pointer.y, scroll);
        // §下拉弹层开着时，命中优先级变了：鼠标在弹层里应该高亮弹层项
        if want != self.hover {
            self.hover = want;
            true
        } else {
            false
        }
    }

    /// 滑块某一点对应的比例（0–1）。
    ///
    /// 用**登记下来的命中区**算，不重新推一遍布局 —— 重新推一遍就会和画出来的
    /// 位置漂移，表现是「滑块跟手但差几个像素」，很难查。
    pub fn fraction_at(&self, s: Slide, x: f32) -> Option<f32> {
        let r = self
            .hots
            .iter()
            .rev()
            .find(|h| matches!(h.act, Act::SetSlide(k, _) if k == s))?
            .rect;
        // 轨道两端各留半个滑块直径（18/2=9），否则拖到最左/最右时
        // 滑块圆会有一半画到控件外面去。
        let pad = 9.0;
        let usable = (r.w - pad * 2.0).max(1.0);
        Some(((x - r.x - pad) / usable).clamp(0.0, 1.0))
    }

    /// 当前该显示什么光标。
    pub fn cursor_hint(&self) -> Cursor {
        match self.hover {
            Some(Act::Focus(_)) => Cursor::IBeam,
            Some(Act::None) => Cursor::Arrow,
            Some(_) => Cursor::Hand,
            None => Cursor::Arrow,
        }
    }

    /// 回车：提交当前输入框。返回要提交的那个输入框（草稿还在才算）。
    ///
    /// 返回 `Edit` 而不是 `bool`：回车应当**等于点「应用」**。
    /// 原来只清掉焦点、草稿留着不动，敲完字按回车毫无反应 ——
    /// 用户只能再去够右上角那个「应用」按钮。
    pub fn commit_focus(&mut self) -> Option<Edit> {
        let e = self.focus?;
        // 草稿还在 = 有东西要提交
        let has = self.edit_text(e).is_some();
        self.focus = None;
        if has {
            Some(e)
        } else {
            None
        }
    }

    /// Esc：放弃当前输入框的草稿。
    pub fn cancel_edit(&mut self) -> bool {
        let Some(e) = self.focus else {
            return false;
        };
        self.drop_edit(e);
        true
    }

    /// 焦点离开窗口：把输入框当失焦提交（草稿留着，`focus` 清掉）。
    ///
    /// **不清草稿**：用户点了别的程序再点回来，输入框里敲了一半的字应该还在。
    /// 但如果直接丢弃 `focus` 而不清草稿，下一帧绘制会以为「还在编辑」而
    /// 显示插入光标 —— 所以这里只清 `focus`。
    pub fn commit_edit_on_blur(&mut self) -> bool {
        let had = self.focus.take().is_some();
        had
    }

    /// 滑块拖拽中的临时值；`None` = 没在拖，调用方该用设备值。
    pub fn live_frac(&self, s: Slide) -> Option<f32> {
        self.live[s.idx()]
    }

    /// 记录滑块拖拽中的值。只在 `pointer.dragging` 是这个滑块时才有意义。
    pub fn set_live_frac(&mut self, s: Slide, frac: f32) {
        self.live[s.idx()] = Some(frac.clamp(0.0, 1.0));
    }

    /// 清掉某条滑块的临时值（松手提交后调用，回落到设备值）。
    pub fn clear_live_frac(&mut self, s: Slide) {
        self.live[s.idx()] = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> Ui {
        Ui::default()
    }

    #[test]
    fn 默认收起侧栏_默认在电量页() {
        let u = ui();
        assert!(u.rail, "用户要求默认收起");
        assert_eq!(u.page, Page::Battery);
    }

    #[test]
    fn 命中测试取最后登记的那个() {
        let mut u = ui();
        u.begin_frame();
        // 两个重叠区域，后登记的在上面
        u.hot(R::new(0.0, 0.0, 100.0, 100.0), Act::Refresh);
        u.hot(R::new(50.0, 50.0, 100.0, 100.0), Act::ToggleRail);

        assert_eq!(u.hit(10.0, 10.0, 0.0), Some(Act::Refresh));
        // 重叠区取上层
        assert_eq!(u.hit(60.0, 60.0, 0.0), Some(Act::ToggleRail));
        assert_eq!(u.hit(200.0, 10.0, 0.0), None);
    }

    #[test]
    fn 命中测试算上滚动量() {
        let mut u = ui();
        u.begin_frame();
        // 内容坐标 y=500 处有个按钮
        u.hot(R::new(10.0, 500.0, 100.0, 30.0), Act::Refresh);
        // 没滚动时它在窗口 y=500（可视区 600 内可见）
        assert_eq!(u.hit(50.0, 510.0, 0.0), Some(Act::Refresh));
        // 滚到 400 之后，它显示在窗口 y=100
        assert_eq!(u.hit(50.0, 110.0, 400.0), Some(Act::Refresh));
        // 而窗口 y=510 现在对应内容 y=910，不该命中
        assert_eq!(u.hit(50.0, 510.0, 400.0), None);
    }

    #[test]
    fn 侧栏命中不随滚动移动() {
        let mut u = ui();
        u.begin_frame();
        // 侧栏的「收起 / 展开」按钮钉在窗口左下
        u.chrome_hot(R::new(8.0, 700.0, 40.0, 32.0), Act::ToggleRail);
        // 无论滚到哪都在同一个窗口位置命中
        assert_eq!(u.hit(20.0, 710.0, 0.0), Some(Act::ToggleRail));
        assert_eq!(u.hit(20.0, 710.0, 1200.0), Some(Act::ToggleRail));
    }

    #[test]
    fn 弹层空白处吃掉点击不穿透到下层() {
        let mut u = ui();
        u.begin_frame();
        u.hot(R::new(0.0, 0.0, 400.0, 400.0), Act::Refresh);
        // 弹层放在中间，但只登记了两个选项
        u.hot(
            R::new(100.0, 100.0, 120.0, 30.0),
            Act::PickDrop(Drop::Poll, 0),
        );
        u.hot(
            R::new(100.0, 130.0, 120.0, 30.0),
            Act::PickDrop(Drop::Poll, 1),
        );
        u.set_drop_rect(R::new(100.0, 100.0, 120.0, 120.0));

        assert_eq!(
            u.hit(150.0, 115.0, 0.0),
            Some(Act::PickDrop(Drop::Poll, 0))
        );
        // 弹层内的空白（选项下方）：必须是 CloseDrop，不能是 Refresh
        assert_eq!(u.hit(150.0, 200.0, 0.0), Some(Act::CloseDrop));
        // 弹层外仍然正常
        assert_eq!(u.hit(20.0, 20.0, 0.0), Some(Act::Refresh));
    }

    #[test]
    fn 页面滚动时弹层选项正确接收点击而不被误判为点外部关闭() {
        let mut u = ui();
        u.begin_frame();
        u.set_open_drop(Some(Drop::Debounce));
        // 假设去抖下拉框在内容 y=600，展开高 248（内容 y=634..882）
        let drop_r = R::new(300.0, 634.0, 150.0, 248.0);
        u.set_drop_rect(drop_r);
        // 低选项（例如 0=关闭 / 1=8ms）在弹层上半部分：内容 y=638..668
        let opt0 = R::new(304.0, 638.0, 142.0, 30.0);
        u.hot(opt0, Act::PickDrop(Drop::Debounce, 0));
        let opt7 = R::new(304.0, 848.0, 142.0, 30.0);
        u.hot(opt7, Act::PickDrop(Drop::Debounce, 7));

        // 页面已向下滚动 300 逻辑像素
        let scroll = 300.0;
        // 那么 opt0 显示在客户区窗口坐标 y = 638 - 300 = 338
        // 窗口坐标下的 (350, 345) 应该命中 opt0
        u.on_press(350.0, 345.0, scroll);
        assert_eq!(
            u.on_release(350.0, 345.0, scroll),
            Some(Act::PickDrop(Drop::Debounce, 0)),
            "页面滚动时点低选项必须成功选中，不能被判为点外部关闭"
        );
        assert_eq!(u.open_drop(), Some(Drop::Debounce));
    }

    #[test]
    fn 滚动夹在内容范围内() {
        let mut u = ui();
        u.set_content_h(Page::Battery, 2000.0);
        // 视口 600，可滚 1400
        assert!(u.wheel(-100.0, 600.0)); // 向下滚到很大
        assert_eq!(u.scroll_y(Page::Battery), 1400.0, "不能超过 max");

        assert!(u.wheel(100.0, 600.0));
        assert_eq!(u.scroll_y(Page::Battery), 0.0, "不能滚成负数");

        // 内容比视口矮时不产生滚动
        u.set_content_h(Page::Perf, 100.0);
        u.page = Page::Perf;
        assert!(!u.wheel(-10.0, 600.0));
        assert_eq!(u.scroll_y(Page::Perf), 0.0);
    }

    #[test]
    fn 每页滚动位置互不干扰() {
        let mut u = ui();
        u.set_content_h(Page::Battery, 5000.0);
        u.set_content_h(Page::Perf, 5000.0);
        u.page = Page::Battery;
        u.wheel(-10.0, 600.0);
        let bat = u.scroll_y(Page::Battery);
        assert!(bat > 0.0);
        assert_eq!(u.scroll_y(Page::Perf), 0.0, "另一页不该跟着动");
    }

    #[test]
    fn 让区域可见() {
        let mut u = ui();
        u.set_content_h(Page::Battery, 2000.0);
        u.page = Page::Battery;
        // 区域在可视区下方 → 滚下去
        assert!(u.ensure_visible(R::new(0.0, 900.0, 100.0, 30.0), 600.0));
        assert_eq!(u.scroll_y(Page::Battery), 330.0);
        // 再调一次不该再动
        assert!(!u.ensure_visible(R::new(0.0, 900.0, 100.0, 30.0), 600.0));
        // 区域在可视区上方 → 滚上来
        assert!(u.ensure_visible(R::new(0.0, 100.0, 100.0, 30.0), 600.0));
        assert_eq!(u.scroll_y(Page::Battery), 100.0);
    }

    #[test]
    fn 输入框字符限制() {
        // DPI 颜色只收十六进制
        assert!(Edit::DpiColor(0).allows('f'));
        assert!(Edit::DpiColor(0).allows('A'));
        assert!(!Edit::DpiColor(0).allows('g'));
        assert!(!Edit::DpiColor(0).allows('-'));
        // DPI 数值只收数字（负号会被 Edit::allows 拒掉）
        assert!(Edit::Dpi(0).allows('4'));
        assert!(!Edit::Dpi(0).allows('x'));
        // 长度上限
        assert_eq!(Edit::Dpi(0).max_len(), 5); // 42000
        assert_eq!(Edit::LowWarn.max_len(), 3); // 100
        assert_eq!(Edit::DpiColor(0).max_len(), 6); // RRGGBB
        assert_eq!(Edit::Button(0).max_len(), 4); // 01F0
    }

    #[test]
    fn 页面枚举齐全且标题不为空() {
        assert_eq!(Page::ALL.len(), 5);
        for p in Page::ALL {
            assert!(!p.title().is_empty());
        }
        // 标题不能重复（侧栏会看起来有两个同名项）
        let mut titles: Vec<_> = Page::ALL.iter().map(|p| p.title()).collect();
        titles.sort_unstable();
        titles.dedup();
        assert_eq!(titles.len(), 5);
    }

    #[test]
    fn 输入框草稿_开始_录入_退格_提交() {
        let mut u = ui();
        // 没开始编辑时不该有草稿 —— 调用方要显示设备当前值
        assert_eq!(u.edit_text(Edit::LowWarn), None);

        u.begin_edit(Edit::LowWarn, "20".into());
        assert_eq!(u.focus, Some(Edit::LowWarn));
        assert_eq!(u.edit_text(Edit::LowWarn), Some("20"));

        // 再点一次同一个框不该冲掉草稿（用户可能刚改了一半）
        u.begin_edit(Edit::LowWarn, "99".into());
        assert_eq!(u.edit_text(Edit::LowWarn), Some("20"));

        assert!(u.backspace());
        assert_eq!(u.edit_text(Edit::LowWarn), Some("2"));

        // 非数字被拒
        assert!(!u.type_char('x'));
        assert!(u.type_char('5'));
        assert_eq!(u.edit_text(Edit::LowWarn), Some("25"));

        // 超长被拒（阈值最多 3 位）
        assert!(u.type_char('0'));
        assert!(!u.type_char('0'), "第 4 位该被拒");
        assert_eq!(u.edit_text(Edit::LowWarn), Some("250"));

        u.drop_edit(Edit::LowWarn);
        assert_eq!(u.edit_text(Edit::LowWarn), None);
        assert_eq!(u.focus, None);
    }

    #[test]
    fn 同一行两个输入框各有草稿() {
        // DPI 一行里有「数值」和「颜色」两个框，一个「应用」按钮要同时读它们。
        // 单个缓冲区表达不了这个场景 —— 这正是改成列表的原因。
        let mut u = ui();
        u.begin_edit(Edit::Dpi(0), "800".into());
        u.begin_edit(Edit::DpiColor(0), "ff00ff".into());
        assert_eq!(u.edit_text(Edit::Dpi(0)), Some("800"));
        assert_eq!(u.edit_text(Edit::DpiColor(0)), Some("ff00ff"));

        // 焦点刚进框时草稿是「全选」状态：第一个字符直接替换预填值。
        // 没有这条的时候，点开写着 400 的框敲 1600 会得到 4001 —— 用户
        // 只能先按五次退格，看起来就像「不能自己输入」。
        assert!(u.type_char('a'), "第一个字符应替换预填值，而不是被限长拒掉");
        assert_eq!(u.edit_text(Edit::DpiColor(0)), Some("a"));
        assert_eq!(u.edit_text(Edit::Dpi(0)), Some("800"), "另一个框不该被动");
        // 之后恢复普通追加
        assert!(u.type_char('b'));
        assert_eq!(u.edit_text(Edit::DpiColor(0)), Some("ab"));
        // 长度上限照旧生效（颜色就是 6 位）
        for c in ['c', 'd', 'e', 'f'] {
            assert!(u.type_char(c));
        }
        assert_eq!(u.edit_text(Edit::DpiColor(0)), Some("abcdef"));
        assert!(!u.type_char('0'), "6 位已满，第 7 位该被拒");
    }

    #[test]
    fn 回车等于点应用() {
        let mut u = ui();
        assert_eq!(u.commit_focus(), None, "没有焦点时无事可做");

        u.begin_edit(Edit::Dpi(0), "1600".into());
        // 回车必须把「要提交哪个框」告诉调用方：只收焦点不发动作的话，
        // 用户敲完字按回车毫无反应，只能再去够右上角的「应用」。
        assert_eq!(u.commit_focus(), Some(Edit::Dpi(0)));
        assert_eq!(u.focus, None, "焦点该收掉");
        assert_eq!(u.edit_text(Edit::Dpi(0)), Some("1600"), "草稿留给页面校验");
        assert_eq!(u.commit_focus(), None, "焦点已收，第二次回车不重复提交");
    }
}
