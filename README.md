# R Battery

**简体中文** | [English](README.en.md)

轻量的 Windows 外设电量与鼠标设置工具，保留 ROG 深度适配，并逐步支持 Logitech、雷柏、VGN、VXE、ATK、Razer、Lamzu、WLMouse、MCHOSE、狼蛛、黑爵、达尔优、漫步者、八位堂、HECATE、Sony、HyperX、Cherry 和 Arctis Nova 的部分设备，以及 XInput 与标准 HID 电量查询。通过 Windows HID 和系统接口读取设备状态；单文件运行，无后台服务，不联网，ROG 设备无需安装 Armoury Crate。

[![Release](https://img.shields.io/github/v/release/youmikk/ROG-Battery)](https://github.com/youmikk/ROG-Battery/releases)
[![Windows](https://img.shields.io/badge/Windows-10%20%2F%2011%20x64-0078D4)](#系统要求)
[![Microsoft Store](https://img.shields.io/badge/Microsoft%20Store-R%20Battery-0067B8)](https://apps.microsoft.com/detail/9nrc8hcd4bz7)

[下载绿色版](https://github.com/youmikk/ROG-Battery/releases/latest) · [Microsoft Store](https://apps.microsoft.com/detail/9nrc8hcd4bz7?hl=zh-CN) · [更新日志](CHANGELOG.md)

各品牌只支持已接入协议的部分设备，并非全系兼容。新增适配已通过协议模拟测试，仍待逐型号实机验证；具体实现范围见[兼容性说明](COMPATIBILITY.md)。

## 未发布改进

任务栏电量条改为系统字体与设备图标，按实际字形宽度排列；单设备和多设备统一显示「设备图标 + 百分比」，充电标识统一使用亮绿色。同时修复设备拖拽排序、仅启用电量条时的后台运行及部分耳机异常报文处理。

电量适配新增 VGN Dragonfly3 Pro 和六组 ryMs 接收器、迈从 K7 V2 / A7 V3 所用的部分 4D 接收器、迈从 K99 V3 键盘、水月雨 GM01 Pro 耳机及雷柏旧 VT 接收器。修正 Windows 无编号 HID 报告的接收长度，避免有效回复因缓冲长度不符而读不到。这批新增路径只读取电量，不开放 DPI 设置；对应型号、连接方式与实机验证状态见[兼容性说明](COMPATIBILITY.md)。以下使用说明对应当前源码，正式下载仍为 1.4.12。

后续接入雷柏 MT760、ATK / VXE NANO、狼蛛 F87ProV2 / F99 MAX、前行者 X87、迈从 V9 PRO 2 / X9 和西伯利亚 K06S 的限定电量协议。设备只回报充电但不提供百分比时，显示充电状态并清除旧数字；读取失败仍按原采样时间过期。各路径的充电支持不同，新增型号仍待本项目实机验收。

## 1.4.12

本版先提供绿色版。新增迈从 V9 PRO 耳机支持，先通过同一接收器的 `FF22` Feature `AA/64` 描述符读取固件版本；低于 `0008` 时停止查询，版本未知时保守回退。新增适配仍待对应型号实机验证。

## 1.4.11

本版先提供绿色版。完善 FF90 耳机电量状态：新增 ATK N9 V2 ULTRA，区分 ATK 放电 / 充电 / 满电状态；迈从 V9 Turbo+ 的“外部供电但电量未知”和“明确离线”会分别处理，不再把静默或供电状态误显示为 0% / 100%。

## 1.4.10

本版先提供绿色版。新增前行者 S9 PRO 8K、S9 M156、迈从 G87 V2、飞智 APEX5、盖世小鸡 Nova / Cyclone 2 的只读电量适配；用户已在 S9 PRO 8K 上确认 23%～98% 与充电读数。所有路径仍只发送已核对查询，不发送配置、震动、灯光或模式写入。

## 1.4.9

本版先提供绿色版。任务栏电量条改为与系统托盘一致的紧凑尺寸：单设备只显示数字，多设备逐台显示设备类型和百分比，并按任务栏实际可用空间完整排列。VGN / VXE 新增五组 2.4G 键盘接收器的严格只读电量查询；`258A:024F` 使用已核对的充电位，其余型号保持充电未知。

## 1.4.8

本版先提供绿色版。新增通知区域左侧任务栏电量条，并继续完善国内常见品牌：雷柏、VGN / VXE / ATK、迈从、狼蛛、黑爵、达尔优、漫步者、八位堂和 HECATE。新增设备协议通过本地模拟回归，仍待逐型号实机验收，具体接收器和型号范围见[兼容性说明](COMPATIBILITY.md)。

迈从鼠标新增 A7 V2 与部分 L7 的 Feature 协议电量、型号及实际档位读取，A7 V2 Ultra+ / L7 Pro+ 开放已有档位切换。旧 A7 Ultra 接收器存在不返回电量的记录，不保证所有同名设备可用；A7 V3 和旧 A5 属于其他协议，不在 1.4.8 范围内。

黑爵 AJ179 PRO 新增 2.4G 电量，以及 USB / 2.4G 实际 X/Y 档位读取和切换。按鼠标自身编号识别型号，读取当前配置后再切档，保留所有档位数值和灯色；充电与有线电量保持未知，蓝牙暂未接入，待实机验收。

狼蛛 SC380 PRO 新增 2.4G 电量、实际 X/Y DPI 档位读取与切换。型号签名确认后读取原配置，切档保留其他设置并完整读回；充电状态保持未知，有线和其他狼蛛型号暂未接入，待实机验收。

狼蛛 F75、F87 PRO 的 2.4G 电量与充电查询也已接入。两者与部分迈从键盘共用 `3554:FA09` 接收器，程序会按完整型号码选择协议，并在读取前后确认设备身份；F87 的官方无线集合存在冲突，暂不启用。

八位堂 Pro 3 在 D-input 模式下增加只读电量和充电显示。程序按标准手柄集合与唯一 Input Report `04` 限定接口，只接收现有状态报文，不发送模式、震动或灯光命令；XInput 模式继续使用系统电量接口。

VGN Dragonfly3 SE 接收器新增电量查询。仅匹配 `391D:1A01 / FF01:0010` 和 65 字节无编号输入输出；程序只发送固定 `55 30` 查询，不读取配置或切换档位，有线回复会清除旧电量，充电状态保持未知。

HECATE G3M PRO 新增 2.4G 接收器百分比读取。程序仅匹配 `320F:706E / FF1C:0092` 和 64 字节 Report `04`，不读取 DPI、灯光或其他配置；充电状态保持未知。

## 1.4.7

新增 Sony DualSense / DualSense Edge 的 USB、蓝牙电量读取，区间值显示为带 ≈ 的数字。协议回归已通过，实机验收待完成，范围见[兼容性说明](COMPATIBILITY.md)。

同时接入 Windows XInput 手柄电量，兼容性取决于当前驱动是否返回有效电池等级。系统槽位分别显示，四级估值带 ≈，有线或未知电池类型不显示虚构的电量和充电状态；对应硬件验收待完成。

新增 Sony INZONE H5、H9 II 接收器电量查询，分别匹配控制接口，保留设备返回的百分比与充电信息；不包含耳机设置写入，实机验收待完成。

同时接入 HyperX Cloud Alpha 2 Wireless 基站的电量、充电和电压查询，读取前后确认耳机连接，过滤其他基站消息；实机验收待完成。

键盘增加 CHERRY MX 2.0S WL Pro 的 USB / 2.4G 电量与充电查询，限定已核对的型号和管理接口；蓝牙与其他 Cherry 型号不在本次范围，实机验收待完成。

标准 HID 按描述符范围换算的电量也统一显示 ≈，精确百分比继续使用普通数字。1.4.7 先提供绿色版，微软商城提交待稳定性验收后单独进行。

## 1.4.6

支持数值 DPI 写入的设备可在性能页设置 1～4 档软件预设，自动按 DPI 升序排列，并从主窗口或托盘切换。减少启用档数会收起最高档并保留数值，增加时恢复。预设不改变设备真实档位列表；会修改鼠标保存 DPI 的连接会在页面提示。此改动已通过本地回归及界面渲染检查，逐型号实机验收待完成。

此外已接入雷柏当前配置的实际档位读取与切换、部分迈从键盘和 Arctis Nova 耳机的电量协议；只返回等级的设备显示带 ≈ 的估算电量，精确读数保持原有数字样式。这些新增能力仍待对应设备实机验收，范围见[兼容性说明](COMPATIBILITY.md)。

1.4.6 先提供绿色版；微软商城提交在完成稳定性验收后单独进行。

## 1.4.5

- 新增部分 VGN / VXE / ATK 鼠标的电量、实际 X/Y DPI 档位和回报率功能。
- 新增 Razer 电量与设置适配；已确认连接能力的型号可切换、编辑 DPI 档位和设置回报率。
- 新增部分 Lamzu / WLMouse 设备的电量、档位和回报率只读查询。
- 新增 ATK N9 V2 Ultimate、MCHOSE V9 Turbo+ 耳机电量查询。
- 完善 Logitech 扩展 DPI 和回报率功能，写入后读取确认。
- 修复雷柏持续回报时电量间歇显示未知的问题；雷柏 DPI 仍为只读。
- 改善短暂读取失败、缓存过期和设备身份变化的处理，保留 ROG 现有深度设置。

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
| 设置 | 明暗主题、开机自启、关闭行为、刷新间隔、存储位置及诊断日志。 |

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

应用不联网，不上传设备信息或使用数据。设置、设备别名、电量历史、缓存、备份和诊断日志均保存在本机。

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

问题或兼容性反馈请提交 [GitHub issue](https://github.com/youmikk/ROG-Battery/issues)。提交日志前请检查并移除序列号、个人路径等内容。

仓库暂未附 LICENSE 文件；补充之前版权归作者所有。如需用于其他项目，请先开 issue 说明。
