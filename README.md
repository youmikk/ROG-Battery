# R Battery

**简体中文** | [English](README.en.md)

轻量的 Windows 外设电量与鼠标设置工具，保留 ROG 深度适配，并逐步支持 Logitech、雷柏、VGN、VXE、ATK、Razer、Lamzu、WLMouse、MCHOSE、狼蛛、黑爵、达尔优、漫步者、八位堂、HECATE、Sony、HyperX、Cherry 和 Arctis Nova 的部分设备，以及 XInput 与标准 HID 电量查询。通过 Windows HID 和系统接口读取设备状态；单文件运行，无后台服务，日常电量读取与设备设置离线完成。仅在用户主动提交反馈、补充说明或查看回复时联网，无后台遥测。ROG 设备无需安装 Armoury Crate。

[![Release](https://img.shields.io/github/v/release/youmikk/ROG-Battery)](https://github.com/youmikk/ROG-Battery/releases)
[![Windows](https://img.shields.io/badge/Windows-10%20%2F%2011%20x64-0078D4)](#系统要求)
[![Microsoft Store](https://img.shields.io/badge/Microsoft%20Store-R%20Battery-0067B8)](https://apps.microsoft.com/detail/9nrc8hcd4bz7)

[下载绿色版](https://github.com/youmikk/ROG-Battery/releases/latest) · [Microsoft Store](https://apps.microsoft.com/detail/9nrc8hcd4bz7?hl=zh-CN) · [更新日志](CHANGELOG.md)

各品牌只支持已接入协议的部分设备，并非全系兼容。具体型号、功能范围及协议测试与实机验收状态见[兼容性说明](COMPATIBILITY.md)。

## 最新版本：1.4.7

- 新增并完善国内常见品牌的鼠标、键盘、耳机及手柄电量查询，保留 ROG 深度适配；部分其他品牌支持读取和切换实际 DPI 档位。
- 任务栏电量条支持多设备紧凑显示、类型图标和亮绿色充电标识，点击打开快速查看面板；改善点击、排序和后台运行。
- 重新整理关于与设备反馈页面，支持可选诊断预览和截图。应用内提交及回复需配置反馈服务；当前发布版通过浏览器提交 GitHub，截图需手动添加。

本次将原 1.4.7～1.4.12 的更新整合为 1.4.7。已安装这些绿色版的用户可退出程序后替换可执行文件。Microsoft Store 发布单独进行。各型号的支持范围与实机验证状态见[设备兼容性](COMPATIBILITY.md)，历史记录见[更新日志](CHANGELOG.md)，协议依据见[协议研究](PROTOCOLS.md)。

## 使用

从 Microsoft Store 安装，或下载 `rog-mouse-ctl.exe` 后直接运行，无需管理员权限。连接支持的设备后，程序会自动读取状态。

### 设备与托盘

首页显示检测到的设备，点击卡片进入详情。多设备时可返回设备首页切换；单设备直接打开详情。

系统托盘保留一个电量数字入口。单击打开以托盘图标为中心的悬浮面板，可查看设备电量、刷新或进入设置；双击打开主窗口，右键打开菜单。

通知区域左侧还会显示紧凑的任务栏电量条。单设备和多设备均按当前设备顺序显示类型图标和百分比，使用系统字体并跟随任务栏缩放。单击电量条打开快速查看面板，再点一次关闭；设置中可以单独关闭，任务栏自动隐藏或所在屏幕被全屏应用覆盖时同步隐藏。

关闭窗口选择最小化时，标准托盘图标或电量条可见即可继续后台运行；两者均不可用时，程序保留普通最小化窗口，方便重新打开。

详情页的设备偏好依次提供「标准托盘图标」「设备名称」「设备类型」。选择的设备用于 Windows 标准托盘图标；任务栏电量条会同时显示全部有电量数据的设备。名称可以编辑或恢复。类型默认自动识别，也可指定鼠标、键盘、耳机、手柄或其他，选择结果会同步到设备首页、详情和两处电量显示。修改类型不会增加设备本身不支持的功能。

托盘设备列表中，鼠标可直接切换到已读取的 DPI 档位；耳机可进入音频控制页，调整扬声器和耳机音量、静音麦克风，或打开 Windows 声音设置。系统默认播放设备由 Windows 管理，应用不会调用未公开接口强制切换。

### 语言

点击右上角齿轮，进入 **设置 → 常规 → 语言**，可选「跟随系统」「简体中文」「English」，切换立即生效。

“跟随系统”使用 Windows **显示语言**。仅把 English 移到 Windows“首选语言”列表首位，不一定会改变显示语言；这种情况下可直接在应用中选择 English。

### 功能

| 功能 | 说明 |
| --- | --- |
| 电量 | 电量、充电状态、设备信息、最近 48 小时历史和续航估算。 |
| 鼠标性能 | ROG 支持的型号保留 DPI、回报率、去抖等深度设置；其他鼠标按协议能力提供 DPI 数值调节、实际档位切换、X/Y 档位编辑和回报率。雷柏可读取当前配置的实际档位并切换，仍待实机验收。 |
| 按键 | 支持的型号可重映射按键、导出配置和恢复备份。 |
| 灯光 | 提供 Windows 动态光效入口，灯效由系统管理。 |
| 设置 | 明暗主题、开机自启、关闭行为、刷新间隔、存储位置、诊断日志与设备反馈。 |

功能取决于设备型号、固件和连接方式，不支持的设置会在界面中说明。修改抬起距离会重置表面标定，操作前请留意提示。

## 系统要求

- Windows 10 1809（17763）及以上，x64。
- Windows 动态光效需要受支持的 Windows 11 版本和兼容设备。
- 支持的 USB HID 设备或无线接收器连接；蓝牙是否可用取决于设备提供的协议。

Armoury Crate 等软件占用设备时可能造成读写失败，建议先退出相关软件。设备休眠时，唤醒后再刷新。

## 支持的型号

已验证 ROG HARPE ACE MINI 的 USB 直连和 ROG OMNI 接收器连接。内置型号表收录以下产品；收录表示可以识别型号，不代表每项功能都已在该硬件上验证。耳机等设备能否读取电量取决于其 HID 协议，不能仅凭设备类型判断兼容性。

### 鼠标（15 款）

| 型号 | 英文型号 | PID |
| --- | --- | --- |
| ROG 龙鳞 Ace Aim Lab 合作版 | ROG HARPE ACE AIM LAB EDITION | `0x1A92` / `0x1A94` |
| ROG 月刃2 ACE | ROG KERIS II ACE | `0x1B16` / `0x1B18` |
| **ROG 龙鳞 Ace Mini**（已验证） | ROG HARPE ACE MINI | `0x1B63` / `0x1B65` |
| ROG 龙鳞 Ace Extreme | ROG HARPE ACE EXTREME | `0x1B67` / `0x1B69` |
| ROG 月刃2 RGB | ROG KERIS II ORIGIN | `0x1C0C` / `0x1C0E` |
| ROG 龙鳞 2 Ace | ROG HARPE II ACE | `0x1C69` / `0x1C6B` |
| PROART MOUSE MD301 | PROART MOUSE MD301 | `0x1CD3` / `0x1CD5` |
| ROG 月刃 2 RGB - KJP | ROG KERIS II ORIGIN-KJP | `0x1D4C` / `0x1D4E` |
| ROG 龙鳞 2 Extreme 20 周年版 | ROG HARPE II EXTREME EDITION 20 | `0x1D7A` / `0x1D7B` / `0x1D7C` |
| ROG 战刃 4 ACE | ROG GLADIUS IV ACE | `0x1DB0` / `0x1DBF` |
| ROG 斯巴达 X 65K | ROG SPATHA X 65K | `0x1DDE` / `0x1DDF` / `0x1DE0` / `0x1DE1` |
| ROG 龙鳞 2 Ace Mini | ROG HARPE II ACE MINI | `0x1E2F` / `0x1E32` |
| ROG 龙鳞 2 ACE - PBZ | ROG HARPE II ACE-PBZ | `0x1E4A` / `0x1E4B` / `0x1E4D` |
| ROG 战刃 4 ACE MAX | ROG GLADIUS IV ACE MAX | `0x1E4E` / `0x1E4F` |
| ROG 龙鳞 2 Ace Mini Demon1 联名版 | ROG HARPE II ACE MINI DEMON1 EDITION | `0x1E51` / `0x1E52` |

### 键盘（13 款）

键盘与鼠标共用同一套电量协议（`12 01`），可读取电量与基本信息；DPI、回报率等鼠标专属项不适用于键盘。

| 型号 | 英文型号 | PID |
| --- | --- | --- |
| ROG 夜魔 | ROG AZOTH | `0x1A83` / `0x1A85` |
| ROG 游侠 2 98 无线版 | ROG STRIX SCOPE II 96 WIRELESS | `0x1AAE` / `0x1AAF` / `0x1AB0` |
| ROG 夜魔 EXTREME | ROG AZOTH EXTREME | `0x1B3F` / `0x1B40` / `0x1B42` |
| ROG 夜魔 98 HE / LITE | ROG AZOTH 96 HE / LITE | `0x1C10` / `0x1C11` / `0x1C12` |
| ROG 夜魔 X | ROG AZOTH X | `0x1C24` / `0x1C25` / `0x1C26` |
| ROG 影魔 | ROG FALCATA | `0x1C2F` / `0x1C30` / `0x1C31` |
| ROG 术士 98 | ROG STRIX MORPH 96 WIRELESS | `0x1CE8` / `0x1CE9` / `0x1CEA` |
| ROG 夜魔 EXTREME SPECIAL EDITION | ROG AZOTH EXTREME SPECIAL EDITION | `0x1CEF` / `0x1CF0` / `0x1CF1` |
| PROART KEYBOARD KD300 | PROART KEYBOARD KD300 | `0x1D45` / `0x1D46` / `0x1D47` |
| 华硕天选 TX75 | ASUS TX75 ANALOG GAMING KEYBOARD | `0x1D68` / `0x1D6A` |
| ROG 夜魔 Extreme 20 周年版 | ROG Azoth Extreme Edition 20 | `0x1DA2` / `0x1DA3` / `0x1DA4` |
| ROG 术士 98 X | ROG STRIX MORPH 96 X WIRELESS | `0x1E0A` / `0x1E0B` / `0x1E0C` |
| ROG 术士 98 - PBZ | ROG STRIX MORPH 96 WIRELESS-PBZ | `0x1E5B` / `0x1E5C` / `0x1E5D` |

### 手柄（2 款）

| 型号 | 英文型号 | PID |
| --- | --- | --- |
| ROG 雷切 II PRO PC | ROG RAIKIRI II PRO PC | `0x1C64` / `0x1C65` |
| ROG 雷切 II XBOX 无线手柄 | ROG RAIKIRI II XBOX WIRELESS | `0x1C91` / `0x1C92` |

### 无线接收器（5 种）

接收器自身没有电池，只作为转发通道。同一型号常有两种形态，协议相同。

| 接收器 | PID |
| --- | --- |
| ROG OMNI 无线接收器 | `0x1ACE` / `0x1ACF` |
| ROG SpeedNova 8K 无线接收器 | `0x1AD0` / `0x1AD1` |
| 华硕无线接收器 | `0x1D54` |

## 数据与隐私

日常电量读取和设备设置不联网，无后台遥测或自动崩溃上报。只有主动提交反馈、补充说明或查看回复时才连接反馈服务。设置、设备别名、电量历史、缓存、备份和诊断日志均保存在本机。

反馈中的可选诊断摘要只包含应用和系统版本、设备原始型号、VID/PID、类型、连接、电量、适配后端及接口与查询摘要，不复制设备序列号、路径、用户别名、个人配置或日志原文。提交时会包含应用版本和所选设备的基本信息；问题描述、选择附带的诊断和截图会在公开 GitHub 议题展示。截图由反馈服务从私有 R2 存储桶提供公开图片链接，原图不重新编码，请先检查画面和图片元数据中的个人内容。

通过应用提交的本地反馈历史及回复密钥使用 Windows DPAPI 加密保存在设置目录。密钥不会写入公开议题；删除本地历史不会删除 GitHub 上的公开内容，也会失去对应的应用内回复凭证。

| 数据 | 默认位置 |
| --- | --- |
| 设置 | `%APPDATA%/rog-mouse-ctl/settings.ini` |
| 电量历史 | `%LOCALAPPDATA%/rog-battery/history.tsv` |
| 快照缓存 | `%LOCALAPPDATA%/rog-mouse-ctl/snapshot.cache` |
| 默认备份目录 | `%USERPROFILE%/Documents/rog-mouse-ctl/backups/` |

设置和历史文件的位置可在应用内调整。商店版的应用数据由 Windows 管理；绿色版数据及自定义位置中的文件需手动清理。完整说明见 [隐私政策](PRIVACY.md)。

## 命令行

```powershell
rog-mouse-ctl.exe info
rog-mouse-ctl.exe quick
rog-mouse-ctl.exe setdpi 2 1600
rog-mouse-ctl.exe backup
rog-mouse-ctl.exe estimate
rog-mouse-ctl.exe --help
```

`setdpi` 等写入命令会修改设备配置，具体参数见帮助信息。

## 从源码构建

安装 Rust GNU 工具链和 MinGW-w64 GCC，并确保 GCC 可在 PATH 中找到。

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu
cargo +stable-x86_64-pc-windows-gnu build --release --locked
```

产物为 `target/release/rog-mouse-ctl.exe`。仓库已包含所需 Windows 导入库和图标资源，无需额外安装运行时。更新图标时可使用 `windres -O coff -I assets -i assets/app.rc -o assets/app.o` 重新生成资源。

本地发布验证会运行回归测试、构建绿色版，检查版本和 PE 子系统，并生成 SHA256 校验文件：

```powershell
powershell -File tools/build-release.ps1
```

绿色版产物集中在 `dist/版本号/`。推送源码和版本标签后，GitHub Actions 会构建并发布绿色版。MSIX 打包与 Microsoft Store 提交由独立的手动工作流处理。

开发分支可手动运行 GitHub Actions 生成测试包：绿色版工作流的 tag 留空；MSIX 工作流默认关闭「提交 Microsoft Store」。构建产物在对应运行的 Artifacts 中下载。完成稳定性验收后，选择要发布的源码版本，并开启商店提交选项。

需要手动打 MSIX 时，安装 Windows SDK 后可运行：

```powershell
powershell -File msix/build-msix.ps1 -Verify
powershell -File msix/make-upload.ps1
```

版本号由 `Cargo.toml` 管理，商店四段版本在 `msix/package.props.psd1` 中同步。发布前确认工作区干净，推送源码后创建对应的 `vX.Y.Z` 标签。标签、源码版本、程序自报版本和 MSIX 版本必须一致，否则流程会停止。标签仅触发绿色版发布；手动提交 Microsoft Store 还需配置商店凭据与产品 ID。

## 反馈与许可

各品牌独立读取，保留 ROG 深度设置。鼠标、键盘和耳机的支持范围分别核对，不能由品牌名称推断兼容性。罗技支持的板载模式保留实际档位，包括第五档；常用数值按钮不代表 G HUB 配置的档位。雷柏 nRF54L 的实际档位切换要求配置与新状态回报一致，支持范围和实机验证状态见[设备兼容性](COMPATIBILITY.md)。

命令行 `devices` 可只读查看当前设备，`device-report` 输出设备描述符和当前运行的查询摘要。`device-report --probe` 先进行只读查询，再导出同次运行的电量、充电状态、当前 DPI、已读取的板载档位及通信结果，方便用户远程协助适配。报告不导出报文内容或设备唯一标识，也不自动上传。

在设置中打开「反馈设备」，选择设备和问题类型，填写说明；可预览并关闭「附带诊断信息」，或添加最多三张截图。配置了反馈服务的版本可直接提交，并在「我的反馈」中查看处理进度、维护者回复和补充说明。只有用户主动执行这些操作时才联网。

未配置反馈服务时，使用「复制内容并打开 GitHub」，在浏览器中检查并提交 [GitHub issue](https://github.com/youmikk/ROG-Battery/issues)；截图需手动添加，浏览器提交不会加入「我的反馈」。诊断日志入口继续保留；独立的设备报告窗口已移除，以上命令行报告功能不受影响。主动分享日志或备份前，请检查并移除序列号、个人路径等内容。

仓库暂未附 LICENSE 文件；补充之前版权归作者所有。如需用于其他项目，请先开 issue 说明。
