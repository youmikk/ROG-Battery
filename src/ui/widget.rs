
use super::gfx::R;

/// 五个页面。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Home,
    Battery,
    Perf,
    Light,
    Keys,
    Settings,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
#[repr(usize)]
pub enum SettingsSection {
    #[default]
    General = 0,
    Battery = 1,
    Data = 2,
    About = 3,
}

impl Page {

}

/// 固定导航图标。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Battery,
    Perf,
    Light,
    Keys,
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
    Palette(u8),
    /// 设置页：窗口关闭行为（每次询问 / 最小化到托盘 / 直接退出）。
    CloseMode,
    TrayKind,
    TrayDevice,
    Language,
}

/// [`Drop::Palette`] 里代表「灯光颜色」的槽位值。
///
/// 0..=3 被 DPI 四档占用，所以灯光用 255 而不是 4 —— 两者在语义上完全无关，
/// 将来 DPI 档数变多时 255 也不会撞上。
pub const LIGHT_COLOR_SLOT: u8 = 255;

/// 文本输入框的种类。
impl Edit {
    /// 这个输入框写的是设备上的值（读不到设备时不该进编辑态）。
    pub fn is_device(self) -> bool {
        matches!(
            self,
            Edit::LowWarn | Edit::Dpi(_) | Edit::DpiColor(_) | Edit::LightColor
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edit {
    /// 性能页：第 N 档 DPI 的数值（0–3）。
    Dpi(u8),
    /// 性能页：第 N 档 DPI 的颜色（0–3），6 位十六进制。
    DpiColor(u8),
    /// 电量页：低电量阈值（0–100）。
    LowWarn,
    LightColor,
}

impl Edit {
    /// 这个输入框允许哪些字符。
    pub fn allows(self, ch: char) -> bool {
        match self {
            Edit::DpiColor(_) | Edit::LightColor => ch.is_ascii_hexdigit(),
            // DPI 最大值 42000，5 位；阈值/亮度/速度都是 3 位以内
            _ => ch.is_ascii_digit(),
        }
    }

    /// 最多几位。
    pub fn max_len(self) -> usize {
        match self {
            Edit::Dpi(_) => 5,
            Edit::DpiColor(_) | Edit::LightColor => 6,
            Edit::LowWarn => 3,
        }
    }
}

/// 滑块。
///
/// `#[repr(usize)]` + 显式判别值：`live` 数组用它当下标，靠「声明顺序」是
/// **隐含约定**，以后有人插一个新滑块进来就会静默错位（表现是拖 A 滑块 B
/// 跟着动）。写出来之后，位置由这里的数字决定。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
pub enum Slide {
    /// 角度微调（−20..20）。
    Tune = 0,
    AudioVolume = 1,
}

impl Slide {
    /// `live` 数组里的下标。
    pub fn idx(self) -> usize {
        self as usize
    }
}

/// 两个悬停目标「是不是同一个东西」。
///
/// `Act::SetSlide` / `Act::SetCustomHue` 里带着 f32 分量（滑块比例、色相角度），
/// 它们随鼠标位置连续变化。直接用 `PartialEq` 比较，鼠标在滑块上移动一个像素
/// 就认为「悬停变了」→ 整页重绘；而这种变化对高亮毫无意义。
fn hover_same(a: Option<Act>, b: Option<Act>) -> bool {
    match (a, b) {
        (Some(Act::SetSlide(x, _)), Some(Act::SetSlide(y, _))) => x == y,
        (Some(Act::SetCustomHue(x, _)), Some(Act::SetCustomHue(y, _))) => x == y,
        (x, y) => x == y,
    }
}

/// 本地偏好（不写设备，写 `settings.ini`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pref {
    TrayEnabled,
    LowBatteryNotify,
    AutoStart,
    /// 电量页：充满效果开关（这个**是**写设备的）。
    FullEffect,
    /// 性能页：角度吸附开关。
    AngleSnap,
}

/// 一个可以点的东西要做的事。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Act {
    SelectDevice(usize),
    ReorderDevice(usize, usize),
    RenameDevice,
    ToggleDeviceTray,
    /// 切页。
    Nav(Page),
    SettingsSection(SettingsSection),
    /// 打开下拉菜单。
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
    CommitSlide(Slide),
    SelectAudio(crate::audio::Slot),
    ToggleMicrophone,
    OpenSoundSettings,
    ToggleAdv(u8),
    /// 指定配色偏好。设置页的「外观」分段控件用：
    /// 0 = 跟随系统，1 = 浅色，2 = 深色。
    SetTheme(u8),
    /// 设置页：打开诊断日志所在文件夹。
    OpenLogFolder,
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

impl Act {
    /// 这个动作要读写设备吗？
    ///
    /// 读不到设备时点这些动作只会失败，`gui` 那边统一换成一句提示；
    /// 本地动作（切页、改本地设置）照常执行。
    pub fn needs_device(self) -> bool {
        match self {
            Act::ApplyEdit(Edit::Dpi(_)) | Act::CommitSlide(Slide::Tune) => true,
            Act::Backup | Act::Restore | Act::RefreshLighting => true,
            Act::Toggle(p) => p.needs_device(),
            Act::PickDrop(d, _) => d.needs_device(),
            // 打开色盘本身就是去挑一个要写进设备的颜色；其它下拉只是展开，
            // 选中时才写（那一步由 `PickDrop` 管）。
            Act::OpenDrop(Drop::Palette(_)) => true,
            // 色盘：开面板、切页签、拖色相、点色块，全都要写设备
            Act::SetPaletteTab(_) | Act::SetCustomHue(Drop::Palette(_), _) => true,
            // `Focus` 由 `gui` 里那一支单独处理（它还要把焦点环退回去）
            _ => false,
        }
    }
}

impl Pref {
    /// 这一项是写设备的开关吗（托盘、开机自启是本地设置）。
    pub fn needs_device(self) -> bool {
        matches!(self, Pref::FullEffect | Pref::AngleSnap)
    }
}

impl Drop {
    /// 这个下拉框选完要写设备吗（刷新间隔、缓存有效期、关闭动作都是本地的）。
    pub fn needs_device(self) -> bool {
        matches!(
            self,
            Drop::Sleep
                | Drop::Poll
                | Drop::Stage
                | Drop::Debounce
                | Drop::Lod
                | Drop::LightMode
                | Drop::ButtonTarget(_)
                | Drop::Palette(_)
        )
    }
}

/// 一个可交互区域。
/// 电量历史曲线的像素级几何（`draw_chart` 每帧都要用，但只在数据或
/// 绘图区尺寸变化时才需要重算）。
///
/// 充电动画期间整帧 60fps 重绘，而采样 30 秒才更新一次；把「归一化坐标 →
/// 像素」这一步与三份 Vec（采样点 / 平滑曲线 / 面积多边形）连同充电底带
/// 一起缓存，热路径就只剩三次 GDI+ 调用。
#[derive(Clone, Debug)]
pub struct ChartGeom {
    /// 缓存键：`Plot` 地址 + 绘图区四个边界 + 纵轴上下界。
    pub key: (usize, u32, u32, u32, u32, u32, u32),
    /// 采样点（像素）。
    pub pts: Vec<(f32, f32)>,
    /// 平滑曲线（像素）。
    pub curve: Vec<(f32, f32)>,
    /// 面积多边形（曲线 + 首尾拉到底边）。
    pub area: Vec<(f32, f32)>,
    /// 充电时段底带。
    pub bands: Vec<R>,
}

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
    pub press_x: f32,
    pub press_y: f32,
    pub down: bool,
    /// 拖拽中的滑块。`Some` 时鼠标移动会持续产生 `SetSlide`。
    pub dragging: Option<Slide>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DeviceDrag {
    pub source: usize,
    pub target: usize,
    pub previous_target: usize,
    pub pointer_x: f32,
    pub pointer_y: f32,
    pub grab_dx: f32,
    pub grab_dy: f32,
    pub started_at: std::time::Instant,
    pub target_changed_at: std::time::Instant,
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
    pub settings_section: SettingsSection,
    settings_scroll: [f32; 4],
    settings_content_h: [f32; 4],
    pub selected_device: Option<String>,
    pub tray_device_keys: Vec<String>,
    pub device_keys: Vec<String>,
    pub initial_route_done: bool,
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
    scroll: [f32; 6],
    /// 每个页面的内容总高度（绘制时写入，滚轮要靠它夹住）。
    content_h: [f32; 6],
    /// 当前打开的下拉弹层。
    open_drop: Option<Drop>,
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
    /// 本帧登记的固定导航可交互区域（**窗口坐标**，不随滚动移动）。
    ///
    /// 两套坐标不能混在一个列表里：固定导航钉在窗口上，内容区跟着 `scroll` 走。
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
    pub device_drag: Option<DeviceDrag>,
    /// 滑块拖拽中的临时值（0.0–1.0），下标是 [`Slide`] 的 `as usize`。
    ///
    /// # 为什么需要这一层
    ///
    /// 页面画滑块时读的是**设备状态**（`d.state`）。拖拽期间鼠标已经移开了，
    /// 写设备却要几十毫秒、读回还得再来一轮 —— 中间那几帧滑块会卡在原地不动，
    /// 松手才「跳」到新位置。`None` = 这一条没有正在拖，页面回落到设备值。
    pub live: [Option<f32>; 2],
    pub audio_slot: crate::audio::Slot,
    /// 状态栏文字（右下角/底部浮层）。
    pub status: String,
    /// 状态栏提示的触发时间（用于自动在 ~2s 后消失）。
    pub status_time: Option<std::time::Instant>,
    /// 本帧绘制时是否真的画了需要连续重绘的动效。
    ///
    /// 由页面在绘制过程中置位（例如充电彗尾），[`Ui::begin_frame`] 清零。
    /// canvas 的 16ms 定时器据此判断「还要不要继续跑」：光靠「用户想要动效」
    /// 这个标志会漏掉「动效已经结束」，结果定时器一路空转到下一次刷新。
    pub anim_hint: bool,
    /// 电量历史曲线的几何缓存，见 [`ChartGeom`]。
    pub chart: Option<ChartGeom>,
    /// 色盘模式：0 = 常用预设，1 = 自定义色环。
    pub palette_tab: u8,
    /// 自定义色环当前选中的色相（0.0 .. 360.0）。
    pub custom_hue: f32,
    /// 窗口按钮的悬停态，与客户区悬停态一起更新。
    pub hover_btn: Option<crate::ui::titlebar::Btn>,
    /// 是否弹出「关闭窗口」确认对话框。
    pub show_close_dialog: bool,
    /// 关闭对话框里的单选项。
    pub close_choice: CloseChoice,
    /// 关闭对话框里的「记住我的选择」复选框。
    pub close_remember: bool,
    /// 读不到设备时的整页锁定。
    ///
    /// 由 `pages::page` 每帧按页面决定（设备页为 true）。锁定时 `hot` 不登记
    /// 命中区，点哪儿都不产生动作；画面上另有一层遮罩说明原因。
    pub locked: bool,
}

impl Default for Ui {
    fn default() -> Self {
        Ui {
            page: Page::Home,
            settings_section: SettingsSection::General,
            settings_scroll: [0.0; 4],
            settings_content_h: [0.0; 4],
            selected_device: None,
            tray_device_keys: Vec::new(),
            device_keys: Vec::new(),
            initial_route_done: false,
            // 默认浅色主题。
            mode: super::theme::Mode::Light,
            // 默认「跟随系统」
            theme_pref: 0,
            adv: [false, true],
            scroll: [0.0; 6],
            content_h: [0.0; 6],
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
            device_drag: None,
            live: [None; 2],
            audio_slot: crate::audio::Slot::Speaker,
            status: String::new(),
            status_time: None,
            anim_hint: false,
            chart: None,
            palette_tab: 0,
            custom_hue: 0.0,
            hover_btn: None,
            show_close_dialog: false,
            close_choice: CloseChoice::Minimize,
            close_remember: false,
            locked: false,
        }
    }
}

impl Ui {
    pub fn sync_devices(&mut self, devices: &[crate::device::DeviceReading]) {
        self.device_keys = devices.iter().map(|d| d.key.clone()).collect();
        if self.device_drag.is_some_and(|drag| {
            drag.source >= devices.len() || drag.target >= devices.len()
        }) {
            self.device_drag = None;
        }
        if !self.initial_route_done && !devices.is_empty() {
            self.initial_route_done = true;
            if self.page == Page::Home && devices.len() == 1 {
                self.select_device(&devices[0].key);
            }
        }
        if let Some(key) = &self.selected_device {
            if !devices.iter().any(|d| &d.key == key) {
                self.selected_device = None;
                if !matches!(self.page, Page::Home | Page::Settings) {
                    self.page = Page::Home;
                }
            }
        }
    }

    pub fn select_device(&mut self, key: &str) {
        self.selected_device = Some(key.to_string());
        self.page = Page::Battery;
        self.scroll[1..5].fill(0.0);
        self.open_drop = None;
        self.focus = None;
        self.edits.clear();
        self.chart = None;
        self.live.fill(None);
    }

    pub fn select_settings_section(&mut self, section: SettingsSection) {
        if self.settings_section == section {
            return;
        }
        self.settings_scroll[self.settings_section as usize] = self.scroll_y(Page::Settings);
        self.settings_section = section;
        *self.scroll_mut(Page::Settings) = self.settings_scroll[section as usize];
        self.content_h[Self::page_index(Page::Settings)] = self.settings_content_h[section as usize];
        self.open_drop = None;
        self.cancel_edit();
        self.hover = None;
    }
    fn page_index(p: Page) -> usize {
        match p {
            Page::Home => 0,
            Page::Battery => 1,
            Page::Perf => 2,
            Page::Light => 3,
            Page::Keys => 4,
            Page::Settings => 5,
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
        if p == Page::Settings {
            self.settings_content_h[self.settings_section as usize] = h;
        }
    }

    pub fn content_h(&self, p: Page) -> f32 {
        self.content_h[Self::page_index(p)]
    }

    /// 视口高度变了（缩放 / 最大化 / 还原）之后，把当前页的滚动量夹回合法范围。
    ///
    /// 不做这一步的表现：窗口放大后 `content_h - view_h` 变小，而滚动量还停在
    /// 旧值 → 内容整体被推到视口上方，底部一片空白，用户得再滚一下才恢复。
    pub fn clamp_scroll(&mut self, view_h: f32) -> bool {
        let p = self.page;
        let ch = self.content_h(p);
        if ch <= 0.0 {
            // 这一页还没画过，不知道内容多高 —— 别乱动。
            return false;
        }
        let max = (ch - view_h).max(0.0);
        let s = self.scroll_mut(p);
        let before = *s;
        *s = s.clamp(0.0, max);
        (*s - before).abs() > 0.01
    }

    /// 开始新一帧：清空登记的命中区与动效标记。
    pub fn begin_frame(&mut self) {
        self.hots.clear();
        self.chrome.clear();
        self.dialog_hots.clear();
        self.drop_rect = None;
        self.drop_btn = None;
        self.anim_hint = false;
    }

    /// 登记一个内容区可交互区域（**内容坐标**，随滚动移动）。
    ///
    /// 读不到设备时**照样登记**：控件要有悬停/按压反馈，点了再由 `gui`
    /// 判断该不该提示「读不到设备」—— 静默吞掉点击才是真的坏了。
    pub fn hot(&mut self, rect: R, act: Act) {
        self.hots.push(Hot { rect, act });
    }

    /// 登记一个固定导航可交互区域（**窗口坐标**，不随滚动移动）。
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

        for h in self.chrome.iter().rev() {
            if h.rect.contains(x, y) {
                return Some(h.act);
            }
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

    // ---- 事件处理（canvas 的 WNDPROC 调这里） ----

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
            if let Some(h) = self.chrome.iter().rev().find(|h| h.rect.contains(x, y)) {
                self.pressed = Some(h.act);
                self.pressed_slide = None;
                self.open_drop = None;
                return;
            }
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

    /// 放弃当前按下的手势（鼠标捕获被系统收回时调用）。
    ///
    /// 不清的话：按住滑块拖到窗口外、按 Alt+Tab 切走、或者弹出别的窗口把捕获
    /// 抢走，`pressed`/`pressed_slide` 会一直留着 —— 之后不按键移动鼠标，
    /// 滑块仍然跟着跑，看起来像「鼠标粘住了」。
    pub fn cancel_press(&mut self) {
        self.pressed = None;
        self.pressed_slide = None;
        self.device_drag = None;
        self.pointer.down = false;
        self.pointer.dragging = None;
    }

    pub fn activate_device_drag(&mut self, x: f32, y: f32, scroll: f32) -> bool {
        if self.device_keys.len() < 2 {
            return false;
        }
        let Some(Act::SelectDevice(source)) = self.pressed else {
            return false;
        };
        if self.hit(x, y, scroll) != Some(Act::SelectDevice(source)) {
            return false;
        }
        let Some(card) = self
            .hots
            .iter()
            .rev()
            .find(|hot| hot.act == Act::SelectDevice(source))
            .map(|hot| hot.rect)
        else {
            return false;
        };
        let now = std::time::Instant::now();
        self.device_drag = Some(DeviceDrag {
            source,
            target: source,
            previous_target: source,
            pointer_x: x,
            pointer_y: y + scroll,
            grab_dx: x - card.x,
            grab_dy: y + scroll - card.y,
            started_at: now,
            target_changed_at: now,
        });
        self.pressed = None;
        true
    }

    pub fn update_device_drag(&mut self, x: f32, y: f32, scroll: f32) -> bool {
        let Some(mut drag) = self.device_drag else {
            return false;
        };
        drag.pointer_x = x;
        drag.pointer_y = y + scroll;
        let target = match self.hit(x, y, scroll) {
            Some(Act::SelectDevice(index)) => index,
            _ => drag.target,
        };
        if target != drag.target {
            drag.previous_target = drag.target;
            drag.target = target;
            drag.target_changed_at = std::time::Instant::now();
        }
        self.device_drag = Some(drag);
        true
    }

    pub fn finish_device_drag(&mut self) -> Option<Act> {
        let drag = self.device_drag.take()?;
        (drag.source != drag.target).then_some(Act::ReorderDevice(drag.source, drag.target))
    }

    /// 客户区与非客户区共用悬停更新，避免跨区后留下旧高亮。
    pub fn refresh_hover_at(&mut self, x: f32, y: f32, win_w: f32) -> bool {
        let want_btn = crate::ui::titlebar::hit(x, y, win_w);
        let scroll = self.scroll_y(self.page);
        let want = if want_btn.is_some() {
            None
        } else {
            self.hit(x, y, scroll)
        };
        let changed = self.hover_btn != want_btn || !hover_same(want, self.hover);
        self.hover_btn = want_btn;
        self.hover = want;
        changed
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
