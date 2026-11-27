# X-Battery

**简体中文** | [English](README.en.md)

X-Battery 原名 R Battery，源码已完成更名；已发布安装包仍可能显示旧名称。

**Windows 多品牌外设电量监控与鼠标快捷设置工具。** 在一个窗口中查看鼠标、键盘、耳机和手柄的电量，通过任务栏电量条与托盘面板快速访问设备。按设备能力提供 DPI 切换，耳机页面集成 Windows 音量和麦克风控制，ROG 鼠标保留深度设置。

单文件运行，无需管理员权限，无独立后台服务。日常电量读取与设备设置离线完成，无后台遥测；ROG 设备无需安装 Armoury Crate。

[![Release](https://img.shields.io/github/v/release/youmikk/X-Battery)](https://github.com/youmikk/X-Battery/releases)
[![Windows](https://img.shields.io/badge/Windows-10%20%2F%2011%20x64-0078D4)](#系统要求)
[![Microsoft Store](https://img.shields.io/badge/Microsoft%20Store-X--Battery-0067B8)](https://apps.microsoft.com/detail/9nrc8hcd4bz7)

[下载绿色版](https://github.com/youmikk/X-Battery/releases/latest) · [Microsoft Store](https://apps.microsoft.com/detail/9nrc8hcd4bz7?hl=zh-CN) · [设备兼容性](COMPATIBILITY.md) · [更新日志](CHANGELOG.md)

## 主要功能

| 功能 | 说明 |
| --- | --- |
| 多设备电量 | 集中查看不同品牌的设备电量及已确认的充电状态。等级换算的电量显示 `≈`，无法读取时显示未知。 |
| 任务栏电量条 | 在通知区域左侧紧凑显示设备类型与百分比，支持多设备、系统缩放和亮绿色充电标识；单击打开快速查看面板。 |
| 托盘快捷操作 | 单击数字托盘查看设备，双击打开主窗口；支持的鼠标可快捷切换 DPI，耳机可展开音频控制。 |
| 鼠标 DPI | 按型号提供数值调节、实际档位切换、X/Y 档位编辑或回报率设置。支持数值写入的连接还可保存 1～4 档软件预设，自动按数值升序排列。 |
| 音频控制 | 通过 Windows 音频接口调整扬声器、耳机音量与麦克风静音；默认输出设备的切换通过 Windows 声音设置完成。 |
| 设备管理 | 自定义设备名称与类型，拖动调整顺序，选择标准托盘显示的设备；设置同步到主窗口与任务栏。 |
| 电量历史与续航 | 各品牌已适配设备独立记录最近 48 小时电量曲线，根据连续有效采样估算剩余使用时间或充满时间；粗略电量保留 `≈` 标识。 |
| ROG 深度设置 | 已适配鼠标保留 DPI、回报率、去抖、按键映射、休眠等设置及配置备份。 |
| 低电量通知 | 商店版支持 Windows 系统通知；绿色版不提供。通知仅使用有效的精确电量，不使用估算值或过期缓存。 |
| 商店更新与评价 | 商店版每日首次启动静默检查更新，只在首页提醒；下载与安装分别确认。关于页提供微软商店官方评价入口。 |
| 外观与语言 | 支持明暗主题、跟随系统、简体中文与 English，另有开机自启、关闭行为及刷新间隔设置。 |

鼠标可用功能由实际型号、固件和连接方式决定。设备自身的 DPI 档位与软件预设分别展示，软件预设不会把只读设备变成可设置设备。支持的 Logitech 板载配置保留实际档位，包括第五档；软件预设的 1～4 档限制不影响这些硬件档位。

电量历史与预测适用于能够返回电量的鼠标、键盘、耳机和手柄。缓存和重复上报不会生成新采样；断连、长时间缺口或充放电状态变化后重新积累连续样本。预测反映近期耗电趋势，数据不足时显示原因；只上报粗略档位的设备需观察更多档位变化，预测按整小时或天显示约数。

<a id="支持的型号"></a>

## 品牌与设备范围

下表按设备类型列出已接入适配的品牌与协议。**仅覆盖部分型号和连接方式，不代表品牌全系兼容，也不代表所有型号均已实机验证。** 具体型号、充电语义、DPI 能力及验证状态以[设备兼容性说明](COMPATIBILITY.md)为准。

| 设备类型 | 已接入的品牌与协议范围 |
| --- | --- |
| 鼠标 | ASUS / ROG、Logitech 罗技、Rapoo 雷柏、VGN、VXE、ATK、Razer 雷蛇、MCHOSE 迈从、AULA 狼蛛、AJAZZ 黑爵、Dareu 达尔优、EWEADN 前行者、DELUX 多彩、灵豹、HECATE、Lamzu、WLMouse |
| 键盘 | ASUS / ROG、Logitech HID++、VGN / VXE、MCHOSE 迈从、AULA 狼蛛、AJAZZ 黑爵、EWEADN 前行者、Veekos、CHERRY、Lofree 洛斐 |
| 耳机 | ATK、MCHOSE 迈从、EDIFIER 漫步者、MOONDROP 水月雨、XIBERIA 西伯利亚、FIFINE、Logitech 罗技、Sony INZONE、HyperX、SteelSeries Arctis |
| 手柄 | ASUS / ROG、Sony DualSense / DualSense Edge、8BitDo 八位堂、Flydigi 飞智、GameSir 盖世小鸡，以及能通过 Windows XInput 返回电池信息的手柄 |
| 标准 HID | 描述符提供标准 Battery Strength 电量能力的设备；标准电量协议不提供通用 DPI 设置 |
| 蓝牙（通用） | Windows 设置中已显示电量的经典蓝牙 / 双模设备；读取系统缓存，不向设备发送命令，也不提供通用 DPI 设置 |

例如，雷柏 VT 系列与 MT 系列、VGN 鼠标与键盘、狼蛛不同代际键盘会使用不同协议；同名系列或共用接收器编号不能直接推断兼容。通用蓝牙路径只显示 Windows 已经提供的电量，蓝牙缓存不提供充电状态时显示为未知；仅支持 BLE / GATT 的设备可能需要后续单独接入。新增适配中的不少型号已完成协议回归，仍需对应设备实机验收。

完整资料：[多品牌兼容性](COMPATIBILITY.md) · [ASUS / ROG 型号目录](ROG_MODELS.md) · [雷柏 nRF54L 型号表](RAPOO_MODELS.md) · [协议研究](PROTOCOLS.md)

## 最新版本：1.4.11

- 商店版在首页提示更新，点击“立即更新”后下载、安装并重新打开软件。
- 新增高斯、狼途、CHERRY、HyperX、AKG、雷神、迈从和赛睿的部分型号电量适配，补齐四款雷蛇接收器的电量读取。
- 设备反馈增加通信采集，支持操作标记、预览、保存和免登录上传采集附件。
- 修复采集结果与上传开关显示，精简更新和反馈页面文案。

绿色版用户可退出程序后替换可执行文件。README 对应当前 GitHub 版本；Microsoft Store 更新需完成商店审核后提供。完整内容见[更新日志](CHANGELOG.md)。

## 快速开始

1. 从 [GitHub Releases](https://github.com/youmikk/X-Battery/releases/latest) 下载 `x-battery.exe` 并运行，或通过 [Microsoft Store](https://apps.microsoft.com/detail/9nrc8hcd4bz7?hl=zh-CN) 安装。
2. 按兼容性说明中的连接方式接入设备。程序自动读取状态；单设备直接打开详情，多设备显示设备列表。
3. 单击数字托盘或任务栏电量条打开快速查看面板。点击支持的鼠标或耳机可展开快捷控制，再次点击收起。
4. 进入 **设置 → 电量与托盘** 调整显示选项；在设备详情中修改名称、类型与标准托盘选择。

启动时先展示已发现的设备或接收器，电量读取完成后逐台更新。DPI、回报率等配置随后在后台加载；同一接收器上的通信按顺序执行。尚未收到设备回包时显示“正在确认连接与电量”。

### 任务栏与设备偏好

标准 Windows 托盘图标显示一个所选设备，任务栏电量条按保存顺序展示有有效电量的设备。电量条空间不足时仅展示可完整容纳的设备，其余设备仍可在快速查看面板中查看。

任务栏自动隐藏或所在屏幕被全屏应用覆盖时，电量条同步隐藏。关闭主窗口选择最小化后，只要标准托盘或电量条可用，程序就会继续后台运行；两者均不可用时保留普通最小化窗口。

设备类型默认自动识别，也可手动指定鼠标、键盘、耳机、手柄或其他。手动选择优先于自动识别，显示名称与类型在各页面同步；更改类型不会增加硬件能力。

### 语言

点击右上角齿轮，进入 **设置 → 常规 → 语言**，可选「跟随系统」「简体中文」「English」，切换立即生效。

默认「跟随系统」使用 Windows **显示语言**，可能与“首选语言”列表的排列不同。需要英文界面时可直接选择 English。

### 商店更新与评价

微软商店安装版在本地日历的每天第一次启动时静默查询官方商店，并保存当日检查记录；启动时网络或商店服务暂不可用会进行有限的延迟重试。绿色版与模拟设备模式不会自动检查更新。

可用更新仅在 **首页设备列表** 内提示，不在隐藏窗口、托盘快速查看或设备详情页弹窗。选择「稍后提醒」后当日不再显示首页提示；**设置 → 关于 → 检查更新** 可随时手动查询，不受每日限制。检查失败显示失败原因，不会误报为最新版本。

下载与安装均需用户确认。下载完成后不会自动安装；安装前等待已有设备写入完成、暂时阻止新的写入，并保存设置和电量历史，再调用微软官方安装接口。保存失败或取消后恢复正常操作，开机自启偏好不变。Windows 可能关闭并重启软件；如未自动重启，请更新后重新打开。应用也保留「退出并在商店更新」入口。

更新说明通过微软商店产品页查看，应用不自行获取商店接口未公开的更新日志，也不接入 GitHub 或第三方自动更新源。**设置 → 关于 → 评价 X-Battery** 提供官方商店评价入口，欢迎分享真实的使用体验；评价完全自愿，不影响任何功能。

### 设备没有显示电量

先确认[兼容性说明](COMPATIBILITY.md)中的具体型号和连接方式，唤醒设备后等待后台读取或点击刷新。厂商驱动占用控制接口时可能影响读取，可退出相关软件后重试。接收器已连接并不一定表示设备正在回报有效电量。

仍无法读取时，可在 **设置 → 关于 → 反馈设备** 中提供型号、连接方式、问题描述，以及可选的诊断摘要。

## 系统要求

- Windows 10 1809（17763）及以上，x64。
- 使用对应型号已适配的 USB、有线或 2.4 GHz 接收器连接；蓝牙支持按协议逐项确认。
- Windows 动态光效入口需要受支持的 Windows 11 版本和兼容硬件。

## 反馈与隐私

在 **设置 → 关于 → 反馈设备** 中选择设备与问题类型，填写说明；可预览或关闭「附带设备诊断」，并选择最多三张截图。

设备列表显示 VID/PID；“列表中没有我的设备”下方列出系统枚举到的其他设备及可能名称。选择后补充品牌和完整型号，勾选诊断时会附带所选设备的接口信息和全部 HID 接口清单。

**官方发布版已接入反馈服务，无需 GitHub 账户即可在应用内提交。** 支持附带截图、查看维护者回复并补充说明，反馈及所选附件会公开展示。未配置反馈服务的自行构建版本提供「复制内容并打开 GitHub」入口，截图需手动添加，这类提交不会加入应用内「我的反馈」。部署方式见[反馈服务说明](services/feedback-relay/README.md)。

日常电量读取与设备设置不联网，无后台遥测或自动崩溃上报。设置、设备别名、历史、缓存、备份和诊断日志保存在本机。商店版每日首次启动会通过微软官方服务检查更新；下载、安装、评价和反馈联网功能由用户主动操作。应用不会向更新服务附加设备诊断、设置或电量历史。

可选诊断摘要保留应用和系统版本、设备原始型号、VID/PID、类型、连接、电量及查询结果，不复制序列号、文件路径、用户别名、个人配置或日志原文。描述、所选诊断和截图会公开展示，请在提交前检查内容。应用内反馈历史与回复密钥使用 Windows DPAPI 加密；删除本地历史不会删除 GitHub 公开内容。完整说明见[隐私政策](PRIVACY.md)。

| 本地数据 | 默认位置 |
| --- | --- |
| 设置 | `%APPDATA%/rog-mouse-ctl/settings.ini` |
| 电量历史 | `%LOCALAPPDATA%/rog-battery/history.tsv` |
| 快照缓存 | `%LOCALAPPDATA%/rog-mouse-ctl/snapshot.cache` |
| 商店更新检查记录 | `%LOCALAPPDATA%/rog-mouse-ctl/store-updates.json` |
| 配置备份 | `%USERPROFILE%/Documents/rog-mouse-ctl/backups/` |

设置和历史位置可在应用内调整。可执行文件已更名为 `x-battery.exe`，数据目录仍沿用 `rog-mouse-ctl`，以兼容已有安装与设置。商店版数据由 Windows 管理，绿色版数据及自定义位置中的文件需手动清理。

## 命令行

多品牌只读查询与诊断：

```powershell
x-battery.exe devices
x-battery.exe device-report
x-battery.exe device-report --probe
x-battery.exe --help
```

`devices` 查看当前设备与读数；`device-report` 输出设备与查询摘要；`--probe` 先执行只读查询，再生成同一次运行的报告。报告不会自动上传，也不包含原始报文或设备唯一标识。

ROG 专用查询与设置示例：

```powershell
x-battery.exe info
x-battery.exe quick
x-battery.exe setdpi 2 1600
x-battery.exe backup
x-battery.exe estimate
```

这些命令沿用 ROG 协议，并非多品牌通用设置命令。`setdpi` 等操作会修改设备配置，保留与保存参数见 `--help`；其他品牌的快捷 DPI 使用应用内对应设备的控制项。

`quick` 输出 `key=value`：`state=ok` 表示三组查询均解析成功；`state=partial` 表示有缺失、无效帧或电量超范围，仅输出可用字段；`state=asleep` 表示未收到有效回帧。脚本应检查状态和所需字段，缺失电量不等于 0%。缓存命中仍会查询当前设备身份。

## 从源码构建

安装 Rust GNU 工具链和 MinGW-w64 GCC，并确保 GCC 可在 PATH 中找到。

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu
cargo +stable-x86_64-pc-windows-gnu build --release --locked
```

产物为 `target/release/x-battery.exe`。仓库包含所需 Windows 导入库、图标和版本资源，无需额外安装运行时。更新图标或文件信息时使用 `windres -O coff -I assets -i assets/app.rc -o assets/app.o` 重新生成资源。

发布前可运行本地测试、构建及版本校验：

```powershell
powershell -File tools/build-release.ps1
```

原生界面回归会切换进程级语言等状态；本地构建脚本和 CI 均以 `--test-threads=1` 执行测试。

产物集中在 `dist/版本号/`。日常开发使用 `main`；推送源码和对应版本标签后，GitHub Actions 构建并发布绿色版。仅验证构建时，可手动运行绿色版工作流并将 tag 留空。

MSIX 打包与 Microsoft Store 提交由独立手动工作流处理，默认不提交商店。`Cargo.toml`、`msix/package.props.psd1`、发布标签和程序自报版本必须一致。稳定性验收后，再按需开启商店提交并配置所需凭据与产品 ID。

## 许可

仓库暂未附 LICENSE 文件；补充之前版权归作者所有。如需用于其他项目，请先开 [issue](https://github.com/youmikk/X-Battery/issues) 说明。
