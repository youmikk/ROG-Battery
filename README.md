# R Battery

**简体中文** | [English](README.en.md)

轻量的 ROG 外设电量与鼠标设置工具。无需安装 Armoury Crate，通过 Windows HID 接口读取设备状态；单文件运行，无后台服务，不联网。

[![Release](https://img.shields.io/github/v/release/youmikk/ROG-Battery)](https://github.com/youmikk/ROG-Battery/releases)
[![Windows](https://img.shields.io/badge/Windows-10%20%2F%2011%20x64-0078D4)](#系统要求)
[![Microsoft Store](https://img.shields.io/badge/Microsoft%20Store-R%20Battery-0067B8)](https://apps.microsoft.com/detail/9nrc8hcd4bz7)

[下载绿色版](https://github.com/youmikk/ROG-Battery/releases/latest) · [Microsoft Store](https://apps.microsoft.com/detail/9nrc8hcd4bz7?hl=zh-CN) · [更新日志](CHANGELOG.md)

## 1.4.1

- 修复修改设备名称并保存后程序可能闪退的问题。
- 诊断日志新增设备型号、VID/PID、接收器信息、操作结果和崩溃位置。
- 中文和英文即时切换，默认跟随 Windows 显示语言。
- 设备卡片首页；只有一台设备时直接进入详情页。

## 使用

从 Microsoft Store 安装，或下载 `rog-mouse-ctl.exe` 后直接运行，无需管理员权限。连接支持的设备后，程序会自动读取状态。

### 设备与托盘

首页显示检测到的设备，点击卡片进入详情。多设备时可返回设备首页切换；单设备直接打开详情。

系统托盘保留一个电量数字入口。单击打开以托盘图标为中心的悬浮面板，可查看设备电量、刷新或进入设置；双击打开主窗口，右键打开菜单。

详情页的设备偏好依次提供「在托盘显示」「设备名称」「设备类型」。选择的设备用于托盘数字；名称可以编辑或恢复。类型默认自动识别，也可指定鼠标、键盘、耳机、手柄或其他，选择结果会同步到设备首页、详情和托盘。修改类型不会增加设备本身不支持的功能。

### 语言

点击右上角齿轮，进入 **设置 → 语言**，可选「跟随系统」「简体中文」「English」，切换立即生效。

“跟随系统”使用 Windows **显示语言**。仅把 English 移到 Windows“首选语言”列表首位，不一定会改变显示语言；这种情况下可直接在应用中选择 English。

### 功能

| 功能 | 说明 |
| --- | --- |
| 电量 | 电量、充电状态、设备信息、最近 48 小时历史和续航估算。 |
| 鼠标性能 | 支持的型号可调整四档 DPI、回报率、去抖、角度吸附、角度微调和抬起距离。 |
| 按键 | 支持的型号可重映射按键、导出配置和恢复备份。 |
| 灯光 | 提供 Windows 动态光效入口，灯效由系统管理。 |
| 设置 | 明暗主题、开机自启、关闭行为、刷新间隔、存储位置及诊断日志。 |

功能取决于设备型号、固件和连接方式，不支持的设置会在界面中说明。修改抬起距离会重置表面标定，操作前请留意提示。

## 系统要求

- Windows 10 1809（17763）及以上，x64。
- Windows 动态光效需要受支持的 Windows 11 版本和兼容设备。
- ROG USB HID 设备或兼容的无线接收器连接；不保证蓝牙模式可用。

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

MSIX 打包需要 Windows SDK 中的 MakeAppx：

```powershell
powershell -File msix/build-msix.ps1 -Verify
powershell -File msix/make-upload.ps1
```

版本号由 `Cargo.toml` 管理，商店四段版本在 `msix/package.props.psd1` 中同步。`v*` 标签会触发绿色版发布和 MSIX 打包；配置商店凭据与产品 ID 后会提交 Microsoft Store。

## 反馈与许可

问题或兼容性反馈请提交 [GitHub issue](https://github.com/youmikk/ROG-Battery/issues)。提交日志前请检查并移除序列号、个人路径等内容。

仓库暂未附 LICENSE 文件；补充之前版权归作者所有。如需用于其他项目，请先开 issue 说明。
