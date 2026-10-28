# R Battery · rog-mouse-ctl

**ROG 鼠标参数控制工具** —— 无需安装 Armoury Crate 或厂商驱动，即可读取和修改鼠标的
DPI、回报率、按键去抖、按键映射与电量状态。单文件运行，全程不联网。

[![绿色版下载](https://img.shields.io/badge/%E7%BB%BF%E8%89%B2%E7%89%88-%E4%B8%8B%E8%BD%BD%E6%9C%80%E6%96%B0%E7%89%88-2EA44F?style=for-the-badge&logo=github&logoColor=white)](https://github.com/youmikk/ROG-Battery/releases)
[![Microsoft Store](https://img.shields.io/badge/Microsoft%20Store-R%20Battery-0067B8?style=for-the-badge&logo=windows&logoColor=white)](https://apps.microsoft.com/detail/9nrc8hcd4bz7?hl=zh-CN&gl=CN)

[![Release](https://img.shields.io/github/v/release/youmikk/ROG-Battery?style=flat-square&label=release)](https://github.com/youmikk/ROG-Battery/releases)
[![Downloads](https://img.shields.io/github/downloads/youmikk/ROG-Battery/total?style=flat-square&label=downloads)](https://github.com/youmikk/ROG-Battery/releases)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11%20x64-0078D4?style=flat-square&logo=windows&logoColor=white)](#系统要求)
[![Size](https://img.shields.io/badge/size-1.0%20MB-4B5563?style=flat-square)](#安装)
[![Dependencies](https://img.shields.io/badge/dependencies-none-2EA44F?style=flat-square)](#从源码构建)

- 单个 exe，约 **1.0 MB**，无运行时依赖，无需管理员权限，不联网
- 提供图形界面（五页）与命令行两种形态
- 直接通过 Windows 自带的 HID 接口通信，不注册后台服务

> **适用范围**：本项目实现的是 ROG 鼠标共用的私有 HID 参数协议，并非某一型号的专用工具。
> 已验证设备为 **ROG HARPE ACE MINI**（`0B05:1ACE`，USB 直连或经 ROG OMNI 接收器）。
> 内置 PID 对照表覆盖 15 款鼠标、13 款键盘、2 款手柄与 5 种接收器，详见
> [支持的型号](#支持的型号)。其他 ROG 鼠标使用同一命令族，但各型号支持的能力存在差异；
> 不支持的项会在界面上标注「本型号不支持」，不会伪装为执行成功。

## 支持的型号

程序内置一张 PID 对照表（75 个 PID、5 种无线接收器），型号名按 PID 查表得出，不依赖固件自报，
所以插上就能显示正确的中文名。完整对照表见 [docs/device-support.md](docs/device-support.md)。

> 表里收录代表「已知型号名，且使用同一套电量 / 参数协议」。具体某一款能改哪些参数仍取决于该型号，
> 不支持的项会在界面上标注「本型号不支持」。

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

## 截图

| 电量 | 性能 |
| --- | --- |
| ![电量页](msix/store/screenshots/01-battery.png) | ![性能页](msix/store/screenshots/02-performance.png) |
| 环形电量、环下预计可用时间、最近 48 小时曲线 | 四档 DPI（含每档指示灯颜色）、回报率、去抖、抬起距离 |

| 按键 | 灯光 |
| --- | --- |
| ![按键页](msix/store/screenshots/03-buttons.png) | ![灯光页](msix/store/screenshots/04-lighting.png) |
| 八个物理按键重映射，支持配置导出与备份恢复 | 交由 Windows 动态光效管理，不占用设备 |

| 设置 | 深色主题 |
| --- | --- |
| ![设置页](msix/store/screenshots/05-settings.png) | ![深色主题](msix/store/screenshots/06-dark-theme.png) |
| 配色、开机自启、关闭行为、文件保存位置 | 跟随系统或手动切换，任务栏数字同步适配明暗 |

---

## 功能

### 电量

- 环形电量显示，配合电压、当前档位 DPI 与变化速率
- 环下方给出预计可用时间，充电时给出预计充满时间，数据来自本机采样
- 最近 48 小时电量曲线，采样点带时间轴
- 设备信息：型号、序列号、接收器固件、连接方式（USB / 2.4G）

### 性能

| 项 | 说明 |
| --- | --- |
| DPI 四档 | 100–42000，步进 50；每档可单独配置指示灯颜色；「切换」用于换档，「应用」用于写入数值 |
| 回报率 | 125 / 250 / 500 / 1000 Hz，越高响应越快，功耗也越高 |
| 按键去抖 | 关闭，或 8–32 ms 共七档；数值越大，双击越不易被识别为两次单击 |
| 角度吸附 | 斜向移动时自动贴合水平或垂直 |
| 角度微调 | 可调范围 −20 至 +20 |
| 抬起距离（LOD） | Low 省电，High 更易追踪。**修改此项会重置鼠标的表面标定**，界面有明确提示 |

### 灯光

灯效交由 **Windows 11 动态光效**统一管理：纯色、呼吸、渐变、波浪、闪烁均由系统提供。
本程序仅负责识别设备并提供跳转至系统设置页的入口，**不占用设备**。设备是否受支持会在页面上标注。

### 按键

八个物理按键可分别映射到不同功能码，支持导出配置与从备份恢复；绑定为「禁用」前需二次确认。

### 设置

- 配色：跟随系统 / 浅色 / 深色（默认跟随系统）
- 开机自启：优先写入注册表 Run 键，写入失败时自动降级到启动文件夹
- 关闭按钮行为：每次询问 / 最小化到托盘 / 直接退出，可记住选择
- 托盘图标开关、刷新间隔、休眠档位、低电量阈值、充满效果
- 历史文件与设置文件的**保存位置可修改**

### 托盘

图标直接绘制当前电量数字，并按任务栏明暗自动换色（≤20% 红色、≤50% 橙色，其余为白或深灰）。
悬停显示四行：型号、连接方式、电量、当前 DPI 与档位。左键或右键均可打开菜单
（显示主窗口、立即刷新、退出）。

### 其他

单实例运行，重复启动会将已有窗口置于前台；启动首帧不闪黑；关闭前可确认。

---

## 安装

**方式一：Microsoft Store** —— [在商店打开](https://apps.microsoft.com/detail/9nrc8hcd4bz7?hl=zh-CN&gl=CN)（`9NRC8HCD4BZ7`），支持自动更新。

**方式二：绿色版** —— 从 [Releases](https://github.com/youmikk/ROG-Battery/releases) 下载 `rog-mouse-ctl.exe`，双击即可运行。
无安装程序，除开机自启外不写注册表。

两种方式的数据位置：

| 数据 | 默认位置 |
| --- | --- |
| 设置 | `%APPDATA%\rog-mouse-ctl\settings.ini` |
| 电量历史 | `%LOCALAPPDATA%\rog-battery\history.tsv` |

通过 MSIX 安装时，上述路径会被系统重定向到 `%LOCALAPPDATA%\Packages\<包族名>\LocalCache\`，卸载时一并清理。
两个文件的位置均可在「设置 → 存储路径」中修改。

## 系统要求

- Windows 10 1809（17763）及以上，x64
- 灯效页的「打开动态光效设置」按钮需要 Windows 11 22H2 及以上
- 支持 USB HID 的 ROG 鼠标、键盘与手柄；型号清单见 [支持的型号](#支持的型号)（已验证：ROG HARPE ACE MINI）

## 命令行

```powershell
rog-mouse-ctl info            # 只读，读取全部参数（带缓存）
rog-mouse-ctl quick           # 只读，仅读取电量，速度最快
rog-mouse-ctl setdpi 2 1600   # 写入第 2 档 DPI
rog-mouse-ctl backup          # 备份为文本
rog-mouse-ctl estimate        # 查看电量历史与续航估算
```

全部子命令、帧格式与返回码见 **[技术文档](docs/technical.md)**。

## 从源码构建

```powershell
.\tools\vendor-import-libs.ps1     # 仅首次需要：生成 import library
cargo build --release              # 产物为 target\release\rog-mouse-ctl.exe
cargo test                         # 148 项测试
```

打包 MSIX 与商店上传件：见 [msix/README.md](msix/README.md)。

## 文档

| 文档 | 内容 |
| --- | --- |
| [docs/technical.md](docs/technical.md) | **技术总文档**：私有 HID 协议、编译链、CLI 全量参考、真机实测记录、安全设计、踩坑记录 |
| [docs/device-support.md](docs/device-support.md) | 75 个 ROG 外设 PID 的型号对照表（鼠标 / 键盘 / 手柄 + 5 种接收器） |
| [docs/autostart.md](docs/autostart.md) | 开机自启的两条路径及其在 MSIX 下的差异 |
| [docs/gdiplus-rendering.md](docs/gdiplus-rendering.md) | 自绘界面的 GDI+ 渲染细节 |
| [docs/gdiplus-window-dc.md](docs/gdiplus-window-dc.md) | 窗口 DC 与双缓冲 |
| [docs/nwg-gotchas.md](docs/nwg-gotchas.md) | GUI 框架选型与踩坑记录 |
| [docs/startup-modes.md](docs/startup-modes.md) | 启动形态（GUI / CLI / 自启） |
| [docs/tray-icon.md](docs/tray-icon.md) | 托盘图标的绘制与自适应配色 |
| [msix/store/submission-notes.md](msix/store/submission-notes.md) | 商店提审所需的能力说明与认证备注 |

## 已知限制

- **灯效仅通过系统设置**：程序自身也能发送灯效命令，但会与 Windows 动态光效争用设备并导致闪烁，因此默认交由系统管理。
- **修改 LOD 会重置表面标定**：厂商未提供仅修改 LOD 而不改动标定的命令，界面已给出提示。
- **不能与 Armoury Crate 同时使用**：它常驻时会占用设备，导致读写失败，请先关闭其灯效或退出程序。
- **鼠标休眠时所有命令均无响应**：移动鼠标后重试即可，不属于故障。
- 加减速曲线、部分型号的独立灯区等能力，本机型号不支持，界面中对应项为禁用状态。

## 隐私

不联网、不收集任何数据、不上报崩溃信息。所有数据仅写入上述两个本地文件。详见 [PRIVACY.md](PRIVACY.md)。

## 许可

仓库暂未附 LICENSE 文件；补充之前版权归作者所有。如需用于其他项目，请先开 issue 说明。
