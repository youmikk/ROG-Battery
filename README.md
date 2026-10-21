# rog-battery —— ROG 外设电量 / DPI 托盘工具

双击就常驻托盘，图标上直接显示电量数字，鼠标移上去看详情（含当前 DPI）。
单个 **70 KB** 的 exe，**没有控制台黑框**，不依赖 Python、不用装任何运行时。

内置 **75 个 PID 的型号表**，覆盖 36 个 ROG 鼠标、47 个 ROG 键盘、4 个手柄
（来源：ASUS Gear Link 的 hid-whitelist + 每个型号的 manifest）。
型号表里没有的设备也能识别——会读设备固件自报的产品名。

---

## 用法

**双击 `rog-battery.exe`** —— 托盘出现电量的**纯数字**（和 Windows 时钟一样的观感）：

- 数字就是电量百分比，浅色任务栏黑字、深色任务栏白字
- 低于 30% 变红色
- **充电时**右下角出现绿色标记：实心点 = 充电中，空心圈 = 已充满
- **鼠标移上去**：
  ```
  ROG 龙鳞 Ace Mini
  连接 有线 · 已充满
  剩余电量 100%
  预计 1 小时 10 分钟后充满
  电压 4176 mV
  DPI 800（第 2/4 档）
  ```
  （`预计…` 那行要攒够电量变化样本才出现，见下文）
- **左键单击**：立即刷新
- **右键**：菜单 → 立即刷新 / 刷新间隔 / 开机自动启动 / 退出
- 默认每 **1 分钟**自动刷新电量（右键菜单可改 30 秒 / 1 分钟 / 3 分钟 / 5 分钟）
- 按鼠标上的 DPI 键切档后，**把鼠标移到图标上**就能看到新数值
  （DPI 只在悬停/右键时读，平时不碰设备，见下文）

命令行参数：

```
rog-battery.exe                托盘常驻（双击就是这个），默认 1 分钟刷新
rog-battery.exe --tray 10      托盘常驻，10 秒刷新（临时覆盖，不改设置）
rog-battery.exe --once         读一次打印后退出
rog-battery.exe --watch 5      控制台持续刷新
rog-battery.exe --mode         打印当前连接方式（有线 / 2.4G / 蓝牙）和命令通道
rog-battery.exe --list-devices 列出型号表收录的设备与本机检测到的设备
rog-battery.exe --charge-selftest    检查充电三态判定的真值表（不需要设备真在充电）
rog-battery.exe --dpi          打印四档 DPI 和当前档位
rog-battery.exe --estimate     看剩余/充满时间的历史样本和当前估算
rog-battery.exe --estimate-selftest  估算器自检（合成数据，不用等真的掉电）
rog-battery.exe --tip-selftest 悬浮提示排版自检（含 127 字符上限）
rog-battery.exe --dump-icons D 把各电量档位的图标导出 PNG（调外观用）
rog-battery.exe --help         帮助
```

> 注意：因为是 GUI 程序（所以双击不弹黑框），在 PowerShell 里直接调用拿不到输出——
> 这是 PowerShell 对 GUI 子系统程序的行为。要看输出请用 cmd 运行，或
> `cmd /c "rog-battery.exe --once"`。

### 支持的设备

`--list-devices` 会打印型号表收录了多少个 PID，以及这台机器上当前插着的 ROG 设备。

- **鼠标**：走 `0xFF01` 命令通道，电量命令 `12 07`，能读到电量、电压、
  低电量阈值、电池类型、充满效果、睡眠模式
- **键盘**：走 `0xFF00` 命令通道（经接收器时是 `0xFF02` 的 rid 2），
  电量命令 `12 01`，只能读到电量、省电模式、充电状态、低电量阈值
  —— **键盘不回报电压**，所以提示里不会出现电压行
- **接收器**：一个 dongle 可能同时带键盘和鼠标，程序优先选鼠标槽位
  （鼠标信息更全），没有鼠标才退到键盘

型号表里没有的设备，会读设备固件自报的产品名顶上。实测 0x19AF 读出来是
`ROG AURA LED Controller`（主板的灯控），这类没电池的设备不会被当成外设。

---

## 常见问题

**托盘里找不到图标？**
Windows 11 默认会把新图标塞进 `^` 溢出区。点任务栏那个 `^`，把图标拖到常显区即可。
（本工具已实测直接出现在常显区，但系统设置不同可能表现不同。）

**图标显示 `?`**
完全读不到：接收器没插、或鼠标关机了。**鼠标闲置约 1 分钟后会进睡眠**，此时读不到也
不会马上变成 `?`——程序会保留上一次的有效读数，只有连续 4 次失败（约 2 分钟）才显示 `?`。
动一下鼠标唤醒它，或点左键立即刷新即可恢复。

**想开机自启？**
右键菜单 → **✓ 开机自动启动** 打勾即可（写 `HKCU\...\CurrentVersion\Run`，不需要管理员）。
再点一次取消。移动了 exe 位置后需要重新勾一次。

**觉得刷新太频繁 / 太慢？**
右键菜单 → **刷新间隔** → 30 秒 / 1 分钟 / 3 分钟 / 5 分钟，选完立刻生效，下次启动沿用。
默认 1 分钟。也可以直接编辑 exe 同目录的 `settings.ini`：

```ini
refresh_interval_sec=60
```

改完重启程序生效。绿色版把设置存在 exe 同目录（拷走整个文件夹设置跟着走）；
MSIX 安装版存在 `%LOCALAPPDATA%\rog-battery\settings.ini`（系统自动重定向）。
命令行 `--tray 30` 只是**临时**覆盖本次运行，不会写回设置。

**是不是要先装 GearLink / Armoury Crate？**
不用。电量、DPI、连接方式全部自己发 HID 命令读，跟 ASUS 任何软件无关
（实测杀光 GearLink 三个进程后照常工作）。

**为什么悬浮提示里没有"还能用多久"？**
需要先攒电量变化样本。百分比掉一格常常要几十分钟，刚装上时样本不够，
程序会明确写"样本不足"而不是猜一个数。用 `--estimate` 能看到攒到什么程度了。

**只有一个能运行**
已做单实例保护，重复双击不会再叠出第二个图标。

---

## 为什么不需要 Python

Windows 10/11 自带 .NET Framework 4.x，其中有 C# 编译器：

```
C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe
```

所以这台机器本来就能编译出原生 exe，不用装 .NET SDK、Visual Studio、Rust 或 PyInstaller。

界面用纯 Win32 `Shell_NotifyIcon` + GDI+ 画图标，没有用 WinForms——
WinForms 的 `NotifyIcon` 常驻要 28.8 MB，纯 Win32 在 28 MB 上下。

| 方案 | 体积 | 常驻内存 |
|---|---|---|
| 本项目 | **70 KB** | **30 MB** |
| WinForms 版 | ~30 KB | 28.8 MB |
| Python + hidapi | 需装解释器 | ~25 MB |
| PyInstaller 打包 | 8–15 MB | ~30 MB |

### 图标为什么比别的托盘图标好看
踩过的三个坑，记录一下免得下次重走：

1. 直接往 16×16 上 `DrawString` 开不开抗锯齿都糊，单色渲染像数字闹钟。
2. `MeasureString` 反推字号不准——它返回含上下留白的行高，数字墨迹只占七成，
   算出来必然偏小；极小字号下还低估宽度，导致右侧被裁（曾经只画出 `10`）。
3. 逐级试字号同样因测量不准而不可靠。

现在改为：**在 128px 上画出来 → 扫像素求真实墨迹包围盒 → 按包围盒精确缩放**。
`InkBounds()` 用 `LockBits` 读 alpha（`>24`）求边界，`InkSize()` 复用它测量。
字体选 **Bahnschrift Light Condensed**（Windows 自带），两个条件同时满足：

- **够窄**：三位数 `100` 的天然宽高比只有 1.42，而 Segoe UI 是 2.04——后者在 16px
  里塞不下，必须横向压到 0.75 倍才勉强，字会明显变扁。
- **够细**：Bahnschrift 的 Regular 在 16px 下笔画糊成一块、发笨。Light Condensed
  窄度不变、笔画细一圈。

候选按"细到粗"排队自动探测：`Bahnschrift Light Condensed` →
`Bahnschrift SemiLight Condensed` → `Bahnschrift Condensed` → `Segoe UI`。
本机实测 16px 平均 alpha（越低越细）：Light Condensed 135–155、
SemiLight 145–162、Condensed 156–174。**Segoe UI Light 只有 78–103，已细到发灰、
16px 下几乎看不清，没采用。**

字高按 **`S * 0.75`（16px 图标 = 12px）**，这是量了真实任务栏时钟 `22:44` 的墨迹
高度（约 12px）后定的。撑满画布会又高又瘦，反而更难看。所有位数用同一个字高，
不会出现"两位数比三位数大一号"。

笔画粗细最终实测：图标 1.5px、任务栏时钟 1.71px——比时钟略细。

### 细笔画为什么不会发虚
128px 画完再双三次缩到 16px，细笔画两侧会被插值摊成半透明灰，肉眼就是"糊"。
最后加一道 `SharpenAlpha()`：把 alpha ≤40 的清成完全透明、≥150 的提成不透明，
中间线性过渡。笔画宽度不变，边缘变干净。实测 40/150 最好——试过 70/180，
会把细笔画本身啃掉一块。

---

## 有线 / 2.4G 两种接法

鼠标插上线和走接收器时，走的是**两个不同的 USB 设备**，命令通道也跟着换：

| 接法 | PnP 设备 | 命令通道 |
|---|---|---|
| 2.4G | `USB\VID_0B05&PID_1ACE`（ROG OMNI RECEIVER） | 接收器的 `0xFF01`，In/Out 64 |
| 有线 | `USB\VID_0B05&PID_1B63`（ROG HARPE ACE MINI） | 鼠标本体的 `0xFF01`，In/Out 65 |

两者**可能同时在线**（线插着、接收器也插在 USB 上），所以判据不是"接收器在不在"，
而是**本体 `1B63` 的命令通道在不在**：在就是有线，不在才去问接收器。

`OpenLink()` 就是这个逻辑：

1. 先枚举所有 `0B05` 设备，筛命令通道：`UsagePage` 为 `0xFF01`/`0xFF02`
   **且** `Usage == 1` **且** `In >= 64`（不筛 `Usage`/`In` 会抓到 6 个无用的
   In=2/12/21 接口）。
2. 有非 `1ACE` 的 `0xFF01` 通道 → **有线**，`reportId = 3`。
3. 否则走接收器：向 `0xFF02` 发 `cmd=160` 扫描配对设备，从响应里取鼠标的命令
   `reportId`，再向 `0xFF01` 发命令 → **2.4G**。

协议本身两边**完全一样**，只是设备路径、包长（65 vs 64）和 `reportId` 不同。
所以 `--once` / `--dpi` / 托盘在两种接法下都能用。

插线时接收器那条通道就**不再转发命令**（实测 `cmd=160` 完全无响应），
因此有线优先级必须高于无线，否则插线后会读不到。

### 排查技巧

```
rog-battery.exe --mode
```
打印所有 `0B05` 命令通道、判定出的接法、以及最终用的通道和 `reportId`。
认错接法时先跑这个。

---

## 充电状态为什么分三种

设备本身只报一个 `charging_status` 字段，但它在 **100% 时会变回 0**。

这不是猜的——GearLink 自己留下的历史日志（`%LOCALAPPDATA%\ASUS\Gear Link\MS\LOG\MS_API\`
下 155 条采样）里记录得很清楚：

```
2026-10-19 21:20:34  power= 98 charging=1     ← 在充
2026-10-19 21:22:34  power= 99 charging=1     ← 在充
2026-10-19 21:23:34  power=100 charging=0     ← 满了，字段回 0
2026-10-19 21:24:34  power=100 charging=0
... 之后 150 条全是 charging=0
```

所以只看这个字段的话，鼠标一旦充满就再也显示不出任何和充电有关的信息——
这正是"充电状态一直没实现"的观感来源。现在按三态归并：

| 显示 | 条件 | 右下角标记 |
|---|---|---|
| 充电中 | `charging_status == 1`（设备说在充，最可信） | 绿色实心点 |
| 已充满 | `charging_status == 0` **且** 有线 **且** 电量 100% | 绿色空心圈 |
| 使用中 | 其余情况（含无线满电） | 无 |

注意"插着线但没满"仍算**使用中**：那种情况下鼠标是边用边充，没有"充满"可言。

托盘图标右下角的标记只用了**实心点 / 空心圈**两种形状，因为实测 16px 下画闪电
会糊成加号、画对勾会糊成笑脸。圆点靠"中间有没有洞"区分，5px 也能看出来。

想验证归并逻辑而不必等鼠标真的跑到那个状态：

```
rog-battery.exe --charge-selftest
```
跑 9 条真值表（9 种 charging/level/mode 组合），全过才算对。

---

## 还能用多久 / 多久充满

悬浮提示里会多出一行：

```
连接 2.4G 无线 · 使用中
剩余电量 87%
预计还能用 4 小时 20 分钟
电压 4185 mV
DPI 800（第 2/4 档）
```

充电时那一行变成 `预计 1 小时 10 分钟后充满`，充满后直接写 `已充满`。

### 为什么要"攒历史"

鼠标只报整数百分比，没有电流、没有速率、没有剩余时间字段（这些在协议里全没有，
已扫过 `0xFF01` 的 `0x12` 各 sub）。所以速度只能自己在 PC 侧按电量随时间的变化算。

麻烦在于**电量变化很慢**：GearLink 留下的 155 条历史采样里 151 条都是 100%。
百分比掉一格常常要几十分钟，因此**刚装上时看不到预测，是正常的**——
攒够 3 个以上"电量真的动过"的样本才开始出结论。

采样存在 `history.tsv`（**exe 同目录**，跟着工具一起走）：

```
unix秒   电量   charging_status   连接方式
1792479868	100	0	2
```

写入策略是刻意的：

- 电量变了 → 立刻记（但两条至少隔 60 秒）
- 电量没变 → 每 10 分钟记一条"心跳"，撑住时间跨度
- 只保留最近 48 小时 / 最多 2000 行

不这样的话，鼠标在 100% 趴一夜能把整个文件刷满同一个值，真正有用的变化点反被挤掉。

### 算法

对同一段样本做最小二乘拟合，斜率就是 `%/小时`：

| 场景 | 公式 |
|---|---|
| 充电 | `(100 - 当前电量) / 斜率` |
| 放电 | `当前电量 / (-斜率)` |

### 故意不给你数字的情况

这比算错更值得写清楚——以下情况一律**只显示原因、不给时间**：

- 样本不足 3 条
- 电量根本没动过（极差 < 1%）—— 分母是零
- 斜率方向与当前状态相反（刚插上/刚拔掉线）
- 当前读数和历史末尾差超过 1%（读数跳变）
- **中间断档超过 2 小时**——那段时间程序没在跑（睡眠/退出/鼠标一直不应答），
  不能当成"这 2 小时没耗电"
- 算出来超过 100 小时（闲置功耗和实际使用差太远，外推没意义）

宁可说"还在观察中"，也不给一个看着精确的错数。

#### 断档为什么要单独拦

历史里两条样本之间隔了 6 小时，并不代表这 6 小时"电量没变"——很可能只是
程序没在运行。但最小二乘看不出这个区别，它会把"6 小时 0 变化"当成真实观测，
把斜率稀释掉一大截。

实测踩过这个坑：一段 13.5 小时的历史里电量只从 94% 掉到 93%（中间程序退过），
回归算出 `-0.0525 %/h`，外推得 **1772 小时（74 天）**。用户看到的是
"预计可用时间：异常（请忽略）"，而真实原因只是数据里有个大洞。

现在的做法是：从最新往回遍历时，**一旦发现相邻样本间隔超过
`HIST_GAP_BREAK_SEC`（7200 秒）就截断**，只用断档之后那一段。
宁可样本少一点，也不用一段来历不明的跨度。

### 验证

不用等几小时，两条自检直接灌合成数据：

```
rog-battery.exe --estimate-selftest    # 11 条：斜率、文案、各种拒绝路径、断档
rog-battery.exe --tip-selftest         # 5 条：悬浮提示排版和 127 字符上限
rog-battery.exe --estimate             # 看真实历史：样本数、变化记录、当前估算
```

`--estimate` 会把每条"电量变化点"列出来，估算不出来时看一眼就知道卡在哪。

> `szTip` 上限 128 字符（含结尾 `\0`），最长组合实测 89 字符，还有余量。

---

## 打包成 MSIX（上架 Store 用）

除了双击运行的绿色版，`msix\` 目录里有一套完整的打包脚本，能产出可直接提交
Microsoft Store 的 `.msix`。

```
cd msix
powershell -NoProfile -ExecutionPolicy Bypass -File .\build-msix.ps1          # 未签名，提交 Store 用这个
powershell -NoProfile -ExecutionPolicy Bypass -File .\build-msix.ps1 -Sign     # 自签，本机测试安装用
```

产物在 `msix\AppPackages\`。脚本会依次做：编译 → 生成图标 → 渲染 manifest
模板 → `makeappx pack` →（可选）`signtool sign`。

### 打包版和绿色版的四处行为差异

装成 MSIX 后 exe 会被放进 `C:\Program Files\WindowsApps\...`（**只读**），
进程也带上了包身份，于是有三件事必须走另一套实现：

| | 绿色版 | 包内 |
|---|---|---|
| 开机自启 | 写 `HKCU\...\Run` | manifest 声明 `windows.startupTask` + `StartupTask` API |
| `history.tsv` | exe 同目录 | `%LOCALAPPDATA%\rog-battery\` |
| `rog-battery.log` | exe 同目录 | `%LOCALAPPDATA%\rog-battery\` |
| 卸载 | 删目录 | 设置 → 应用 |

代码里靠 `GetCurrentPackageFullName` 的返回值（15700 = 没打包）自动分派，
**同一份 exe 两种形态都能跑**，不需要编译两个版本。

#### 开机自启为什么不能沿用注册表

打包应用的注册表写入会被 MSIX **虚拟化**重定向到包的私有位置。写
`HKCU\...\Run` 看上去 `SetValue` 成功、注册表编辑器里也确实能看到，
但那是个影子键——真实的系统启动项里没有这一条，重启后不会自启。

正确做法是在 manifest 里声明：

```xml
<desktop:Extension Category="windows.startupTask" Executable="rog-battery.exe"
                    EntryPoint="Windows.FullTrustApplication">
  <desktop:StartupTask TaskId="RogBatteryAutoStart" Enabled="false" DisplayName="R Battery" />
</desktop:Extension>
```

`TaskId` 必须和 `tray.cs` 里 `StartupTaskId` 常量一字不差。然后代码里
`StartupTask.GetAsync(taskId)` → `RequestEnableAsync()` 请求启用，系统会把
状态记在
`HKCU\...\AppModel\SystemAppData\<包族名>\RogBatteryAutoStart\State`
（`0=Disabled`、`1=DisabledByUser`、`2=Enabled`、`3=DisabledByPolicy`、
`4=EnabledByPolicy`），用户能在**任务管理器 → 启动应用**里看到并手动开关。

`RequestEnableAsync` 返回值是新的状态。如果返回 `1`（用户之前在任务管理器里
手动关过），系统不再允许程序自己打开，程序只能提示用户去手动开——这是
Windows 的设计，不是 bug。

> **技术细节**：WinRT 的 `IAsyncOperation<T>` 在 .NET Framework 里是
> `System.__ComObject`（RCW），`GetType().GetMethod("GetResults")` **拿不到**，
> 实测报错就是「GetResults 拿不到（op 类型 System.__ComObject）」。
> 必须走 `System.Runtime.WindowsRuntime` 的投影：`AsTask<T>()` → `Task<T>.Result`。
> `build.cmd` 坚持不加额外 `/r:` 引用，所以这里用
> `Assembly.Load("System.Runtime.WindowsRuntime, ...")` 晚绑定，
> 拿不到就降级（菜单提示），不影响绿色版。

#### 为什么绿色版和包内版不要同时开自启

两套都启用的话，开机会各拉一个进程。单实例 Mutex `Local\RogBatteryTray`
会让第二个直接退出，所以不会真跑两个，但会有一个进程白启一次。
只用其中一种即可。

### 签名与 SmartScreen

`.msix` **必须签名**才能 `Add-AppxPackage` 安装，否则报
`0x800B0100: 应用包必须经过数字签名才能进行签名验证`。

- **本机测试**：`-Sign` 用自签证书签，然后把证书导进
  「受信任的根证书颁发机构」才能装：
  ```powershell
  Export-Certificate -Cert Cert:\CurrentUser\My\<指纹> -FilePath $env:TEMP\rb-test.cer
  Import-Certificate -FilePath $env:TEMP\rb-test.cer -CertStoreLocation Cert:\LocalMachine\Root
  ```
  测完记得删掉那张根证书（`Remove-Item Cert:\LocalMachine\Root\<指纹>`），
  别留在机器上。自签证书的 Subject 就是 `Publisher` 那个值
  （`CN=1307ADA3-FCC0-4C75-81A2-9CC947003BD7`），有效期 3 年——
  跟正式身份一致的好处是本地装的包不会因为 Subject 和 manifest 对不上被拒。
- **提交 Store**：提**未签名**的那个（`build-msix.ps1` 不带 `-Sign` 的产物）。
  Store 会用自己的证书重新签，装到用户机器上**永远不会弹 SmartScreen**——
  这是目前唯一免费消除「Windows 已保护你的电脑」的路。
  自签的那个**不要**提交。

### 身份字段（已填好正式值）

`package.props.psd1` 集中了所有可变项。下面三项直接对应 Partner Center
「产品标识」页，**不要自己改写格式**：

| 项 | 值 |
|---|---|
| `IdentityName` | `Sonder.RBattery` |
| `Publisher` | `CN=1307ADA3-FCC0-4C75-81A2-9CC947003BD7` |
| `PublisherDisplayName` | `Sonder` |

包族名（PFN）随之变成 `Sonder.RBattery_wm3rvpzgxb7kt`，
Store ID `9NRC8HCD4BZ7`。`Publisher` 必须和签名证书 Subject **逐字一致**
（区分大小写、逗号后空格也要一样），否则提交必被拒。

> `DisplayName` / `ShortName` / `Description` 是用户在开始菜单和任务管理器里
> 看到的名字，跟身份无关，可以随时改（改完重打包，Store 允许更新）。
> 身份三项提交后**锁死**，改不了。

### `runFullTrust` 不等于放宽权限

manifest 里声明了 `<rescap:Capability Name="runFullTrust" />`。它是桌面桥应用的
准入门槛（少了 exe 起不来），但**不额外授予文件系统或设备权限**——进程仍然跑在
用户身份下，读 `%LOCALAPPDATA%\ASUS\...`、打开 HID 设备的权限跟普通 exe 完全一样。

所以**不需要** `broadFileSystemAccess`（那个是给真 UWP 用的，加它反而要写评审理由）。

### makeappx 从哪来

`package.props.psd1` 里默认指向：

```
D:\codex\_tools\makeappx\makeappx.exe
D:\codex\_tools\makeappx\signtool.exe
```

来自 NuGet 包 `microsoft.windows.sdk.buildtools`（版本 10.0.28000.2705），
解压在 `D:\codex\_tools\sdk-buildtools\`。**整个目录要留着**——`makeappx`
靠同目录的 `.manifest` 做免注册 COM 绑定，只拷 exe 会报
`side-by-side configuration is incorrect`。

换机器时改 `package.props.psd1` 里那两个路径即可，或者装了 Windows SDK
的话直接指向 `bin\<version>\x64\` 下的同名文件。

---

## 重新构建

```
build.cmd
```

无其它依赖。改代码后重跑这一行。

---

## 协议（逆向自 GearLink 网页版 WebHID，已在真机验证）

接收器 `VID 0x0B05 / PID 0x1ACE`，暴露三个 64 字节 HID 接口：

| Usage Page | report ID | 用途 |
|---|---|---|
| `0xFF02` | 1 | 接收器命令通道（扫描槽位、读接收器序列号） |
| `0xFF00` | 2 | 键盘命令通道 |
| `0xFF01` | 3 | 鼠标命令通道 |

插线直连时不经过接收器：鼠标本体走 `0xFF01`，键盘本体走 `0xFF00`。

### 1. 扫描配对设备

往 `0xFF02` 发（首字节 report ID，之后 4 字节帧头 + 补零到 64）：

```
01 | a0 00 00 00 | 00 ... 00
     ^cmd ^sub ^idx(LE16)
```

响应：

```
01 | a0 00 <countLo> <countHi> | PID(LE16) reportId notifyReportId | ...（每台 4 字节）
```

例：`01 a0 00 01 00 65 1b 03 04` → 1 台，PID `0x1B65`，命令用 report ID `03`。
（第 3 字节 `03` 是 controlReportId：`2` = 键盘槽位，`3` = 鼠标槽位。
程序按这个字节挑设备，所以同一个接收器上插键盘也能认出来。）

### 2. 读电量（鼠标）

往 `0xFF01` 发，**report ID 用上一步扫描到的那个**：

```
03 | 12 07 00 00 | 00 ... 00
     ^cmd=18 ^sub=7
```

响应：

```
03 | 12 07 00 00 | 电量% | 睡眠 | 低电量阈值 | 电压(LE16) | 充电 | 电池类型 | 充满效果
```

实测：电量 `100`、低电量阈值 `20`、电压 `4176` mV、充满效果 `1`、睡眠 `0`。
电压随负载抖动（几秒内 4171→4184 mV 属正常）。

`电量 == 0 && 充电 == 0` 表示**设备休眠、暂时没有数据**，不是真的 0%——
程序会把它当成读不到，保留上一次的值。

### 2b. 读电量（键盘）

键盘是**另一条命令**，字段偏移也不同：

```
02 | 12 01 00 00 | 00 ... 00
     ^cmd ^sub=1        （键盘槽位用 report ID 2；经接收器是 0xFF02 的 rid 2）
```

响应（需 ≥10 字节）：

```
02 | 12 01 00 00 | 0 | 电量% | idle枚举 | 省电模式 | 充电 | 低电量阈值 | ...
                       ^d[5]
```

- **电量取 `d[5]`，不是鼠标的 `d[4]`**（`d[4]` 恒 0），取到 >100 就当未知
- `d[6]` idle：0=1分钟、1=2、2=3、3=5、4=10、255=从不
- `d[7]` 省电：0 关闭、1 关灯、2 变暗
- `d[8]` 充电：1 = 充电中
- **键盘不回报电压**，所以提示里没有电压行
- 注意 `12 07` 在键盘上是「查询 Windows 灯效支持」，不是电量

### 帧格式

`[cmd, sub, indexLo, indexHi, ...payload]`，USB 补到 64 字节、无线补到 20 字节；
响应按前 4 字节（cmd/sub/index）回显匹配，默认超时 500 ms。

### 3. 读 DPI（请求-响应，普通查询命令）

DPI 是**可以直接问的**，不需要监听什么异步事件：

```
发: 03 | 12 00 | 00 00 | 00 ...          ← cmd=0x12 sub=0x00 index=0
收: 12 00 00 00 | 07 00 07 00 05 07 00 02 ff ff 15 00 01 ...
                  ^^          ^^
                  |           └─ 偏移 7：当前档位（1 基，2 = 第 2 档）
                  └─ 偏移 3..4：DPI 步长信息
```

四档的具体数值用另一条：

```
发: 03 | 12 04 | 02 00 | 00 ...          ← sub=0x04 index=2
收: 12 04 02 00 | 07 00 07 00  0f 00 0f 00  1f 00 1f 00  3f 00 3f 00
                 ^^^^^^^^^^^  ^^^^^^^^^^^  ^^^^^^^^^^^  ^^^^^^^^^^^
                 第1档         第2档        第3档        第4档
                 (X,Y)         (X,Y)        (X,Y)        (X,Y)
```

- 每档占 **4 字节**：X 轴 2 字节 + Y 轴 2 字节（都是小端），通常 X = Y
- 真实 DPI = `(原始值 + 1) × 50`
- 上例：`07 00` → `(7+1)×50` = **400**，`0f 00` → `(15+1)×50` = **800**，
  `1f 00` → **1600**，`3f 00` → **3200**

颜色也能读（`12 04 index=3`，每档 3 字节 RGB）：

```
收: 12 04 03 00 | ff 00 00  ff 00 ff  00 ff 00  00 ff 00
                  #ff0000   #ff00ff   #00ff00   #00ff00
```

> **踩过的坑：DPI 不是事件推送。**
> 早期版本以为鼠标会在按 DPI 键时主动推一帧 `03 12 00 09 00 <idx>`，
> 于是写成"后台监听事件"。真相是：那一帧根本不是 `event id=9`，
> 而是 `cmd=0x12, sub=0x00, index=0x0009` 的**普通应答**，
> 是 GearLink / Armoury Crate 服务在后台不停轮询同一个查询命令，
> 才让人误以为设备在主动推。GearLink 一卸载，帧就再也没了。
>
> 所以只能主动问。但"问多勤"是可以选的：早期每秒问一次，后来 30 秒一次，
> 现在默认 **1 分钟一次**（托盘菜单可调 30 秒 ~ 5 分钟），DPI 则**只在鼠标
> 悬停到图标上时才问**——毕竟只有那一刻那行数字才会被看见。

### 电量与 DPI 同时读

两条命令都在 `0xFF01` 上，但**不能两个线程同时发**——应答没有序号，
互相抢会把对方的响应读走。所以 DPI 查询固定在主线程（悬停探测到光标
落在图标上时现读），电量刷新走 `RefreshCore`，两者靠消息串行化，不会撞车。

早期用独立 `DpiLoop` 线程轮询，就得靠"各自独立开句柄"来回避这个问题；
现在改成按需读取，反而更简单了。

## 已知限制

- **型号表里的设备不用接上就能查**：`--list-devices` 会列出收录的 75 个 PID，
  但只有本机真的插着的设备才能读到电量。没有对应设备的用户只能看型号表。
- **只测过龙鳞 Ace Mini 一台实物**。其余 35 个鼠标 PID、47 个键盘 PID 的型号名
  来自 ASUS 官方 manifest，协议按 Gear Link / libratbag 的公开实现编写 ——
  理论上通用（都是 `12 07` / `12 01` 那套），但没在实机上逐个验证。
  遇到读不出的设备，用 `--list-devices` 与 `--mode` 看走到哪一步。
- **键盘只能读电量和充电状态**，没有电压、电池类型、充满效果
  —— 键盘的电量响应里就没有这些字段。
- **只能读 DPI，不能改**。改 DPI 的命令字节没有逆出来（GearLink 日志只记 JSON，
  不记原始 HID 帧），而且不确定 `ROGMS.sys` 过滤驱动是否参与。目前要改 DPI
  请用鼠标上的实体键，或用 GearLink。
- **DPI 只对鼠标有效**。键盘没有 DPI 概念，悬停时不会出现那一行。
- 启动气泡（`NIF_INFO`）在 Windows 11 上不显示——Win11 已用 toast 通知取代 balloon，
  这不影响使用，图标本身正常。
- 控制台中文依赖代码页 936（GBK）。终端若是 UTF-8，设 `ROG_BATTERY_UTF8=1`。

## 排错

托盘不出现时，用调试日志看卡在哪：

```
set ROG_BATTERY_DEBUG=1
rog-battery.exe
type %TEMP%\rog-battery.log
```

正常会看到 `RegisterClassEx OK` / `CreateWindowEx OK` / `NIM_ADD OK`。
默认不写日志（避免长期常驻反复写盘）。

### explorer 重启后图标会自己回来

以前不会：托盘图标是登记在 explorer 里的，explorer 一崩溃/更新/手动重启，
图标就永久消失，只能杀进程重开。现在注册了 `TaskbarCreated` 广播消息
（`RegisterWindowMessage("TaskbarCreated")`），收到后自动 `NIM_ADD` 补回，
失败时还会重试 10 次（每次间隔 500ms）。

### DPI 什么时候读

**只在你把鼠标移到托盘图标上（或右键打开菜单）的那一刻读一次**，其余时间一条命令都不发。

DPI 没有推送通道——不是"鼠标主动告诉你它换挡了"，只能主动去问。所以早期版本
试过两种轮询：

| 方案 | 每天查询 | 每天 HID 命令 | 问题 |
|---|---|---|---|
| 每秒轮询 | 86,400 | 345,600 | 鼠标 2.4G 收发器被反复叫醒，进不了深度休眠 |
| 跟电量同频（30 秒） | 2,880 | 11,520 | 好一些，但仍是"没人看的时候也在读" |
| 跟电量同频（60 秒） | 1,440 | 5,760 | **现在的默认值**，一半的通信量 |
| **悬停时现读** | 取决于实际悬停，通常 < 20 | < 80 | 只有真要看的时刻才通信 |

**悬停是怎么发现的：自己算，不用 `NIN_POPUPOPEN`。**

试过 `NOTIFYICON_VERSION_4` + `NIN_POPUPOPEN`，但 v4 的语义是
"标准悬浮提示不再自动显示，应用自己画"。我们没实现自绘，
结果就是鼠标移上去**什么都不显示**。为了提前一点点知道用户要悬停，
接管整个提示的绘制不划算，于是退回 v3，改成每 200ms 用
`GetCursorPos` + `Shell_NotifyIconGetRect` 自己判断光标是否落在图标上
（纯本地坐标比较，不碰设备）。

判断到悬停后**立刻现读一次** DPI。此时 Shell 还没弹提示，来得及把新值
填进 `szTip`，所以看到的是当前值而不是缓存。

**悬停期间每 2 秒复读一次**（`HOVER_REFRESH_MS`）。原因：提示一旦弹出，
Shell 不会主动再来问我们要内容——悬停着按 DPI 键切档，提示里会一直显示
旧档位。实测就是这么发现的。但不能每 200ms 读一次（5 次/秒，比最早的
每秒轮询还费），所以单独限流到 2 秒。

另有 1 秒的硬限流（`DPI_MIN_GAP_MS`）：鼠标在图标上来回蹭会连着触发，
且一次左键单击实测能引出 6 个 `NIN_SELECT`。限流后连点只读一次。

> 想立刻看当前 DPI，除了悬停，也可以用 `rog-battery.exe --dpi`。

---

## 支持的型号（75 个 PID / 30 个型号）

下表来自 ASUS Gear Link 的 `hid-whitelist.json` 与各型号 `manifest.json` 的
`caps.deviceNameZH`，只收录 `hasPowerInfo = true` 的设备。
同一型号通常有 2 个 PID（有线 / 2.4G 各一个），少数型号有 3–4 个。

| 型号 | 英文名 | PID |
|---|---|---|
| ROG 龙鳞 Ace Mini | ROG HARPE ACE MINI | `0x1B63` `0x1B65` |
| ROG 龙鳞 Ace Extreme | ROG HARPE ACE EXTREME | `0x1B67` `0x1B69` |
| ROG 龙鳞 Ace Aim Lab 合作版 | ROG HARPE ACE AIM LAB EDITION | `0x1A92` `0x1A94` |
| ROG 龙鳞 2 Ace | ROG HARPE II ACE | `0x1C69` `0x1C6B` |
| ROG 龙鳞 2 Ace Mini | ROG HARPE II ACE MINI | `0x1E2F` `0x1E32` |
| ROG 龙鳞 2 Ace Mini Demon1联名版 | ROG HARPE II ACE MINI DEMON1 EDITION | `0x1E51` `0x1E52` |
| ROG 龙鳞 2 Extreme 20周年版 | ROG HARPE II EXTREME EDITION 20 | `0x1D7A` `0x1D7B` `0x1D7C` |
| ROG 龙鳞2 ACE - PBZ | ROG HARPE II ACE-PBZ | `0x1E4A` `0x1E4B` `0x1E4D` |
| ROG 月刃2 ACE | ROG KERIS II ACE | `0x1B16` `0x1B18` |
| ROG 月刃2 RGB | ROG KERIS II ORIGIN | `0x1C0C` `0x1C0E` |
| ROG 月刃 2 RGB - KJP | ROG KERIS II ORIGIN-KJP | `0x1D4C` `0x1D4E` |
| ROG 战刃 4 ACE | ROG GLADIUS IV ACE | `0x1DB0` `0x1DBF` |
| ROG 战刃 4 ACE MAX | ROG GLADIUS IV ACE MAX | `0x1E4E` `0x1E4F` |
| ROG 斯巴达 X 65K | ROG SPATHA X 65K | `0x1DDE` `0x1DDF` `0x1DE0` `0x1DE1` |
| PROART MOUSE MD301 | PROART MOUSE MD301 | `0x1CD3` `0x1CD5` |
| ROG 夜魔 | ROG AZOTH | `0x1A83` `0x1A85` |
| ROG 夜魔 X | ROG AZOTH X | `0x1C24` `0x1C25` `0x1C26` |
| ROG 夜魔 EXTREME | ROG AZOTH EXTREME | `0x1B3F` `0x1B40` `0x1B42` |
| ROG 夜魔 EXTREME SPECIAL EDITION | ROG AZOTH EXTREME SPECIAL EDITION | `0x1CEF` `0x1CF0` `0x1CF1` |
| ROG 夜魔 Extreme 20 周年版 | ROG Azoth Extreme Edition 20 | `0x1DA2` `0x1DA3` `0x1DA4` |
| ROG 夜魔 98 HE / LITE | ROG AZOTH 96 HE / LITE | `0x1C10` `0x1C11` `0x1C12` |
| ROG 游侠2 98无线版 | ROG STRIX SCOPE II 96 WIRELESS | `0x1AAE` `0x1AAF` `0x1AB0` |
| ROG 术士 98 | ROG STRIX MORPH 96 WIRELESS | `0x1CE8` `0x1CE9` `0x1CEA` |
| ROG 术士 98 X | ROG STRIX MORPH 96 X WIRELESS | `0x1E0A` `0x1E0B` `0x1E0C` |
| ROG 术士 98 - PBZ | ROG STRIX MORPH 96 WIRELESS-PBZ | `0x1E5B` `0x1E5C` `0x1E5D` |
| ROG 影魔 | ROG FALCATA | `0x1C2F` `0x1C30` `0x1C31` |
| 华硕天选TX75 | ASUS TX75 ANALOG GAMING KEYBOARD | `0x1D68` `0x1D6A` |
| PROART KEYBOARD KD300 | PROART KEYBOARD KD300 | `0x1D45` `0x1D46` `0x1D47` |
| ROG 雷切 II PRO PC | ROG RAIKIRI II PRO PC | `0x1C64` `0x1C65` |
| ROG 雷切 II XBOX无线手柄 | ROG RAIKIRI II XBOX WIRELESS | `0x1C91` `0x1C92` |

接收器（不读电量，只用来转发命令）：

| 接收器 | PID |
|---|---|
| ROG OMNI 无线接收器 | `0x1ACE` `0x1ACF` |
| ROG SpeedNova 8K 无线接收器 | `0x1AD0` `0x1AD1` |
| 华硕无线接收器 | `0x1D54` |

> 你的设备不在表里也能用——程序会读设备固件自报的产品名当显示名。
> 想让某个型号进表，把 `--list-devices` 的输出贴到 issue 里即可。

---

## 文件

| 文件 | 说明 |
|---|---|
| `rog-battery.cs` | 主程序：HID 协议、Win32 枚举、命令行、电量历史与时间预测 |
| `devices.cs` | 型号表：75 个 PID → 型号名（鼠标 / 键盘 / 手柄）+ 接收器名字表 |
| `tray.cs` | 托盘：消息窗口、右键菜单、定时刷新、图标绘制 |
| `settings.cs` | 设置持久化：`settings.ini` 读写，目前只有刷新间隔 |
| `build.cmd` | 一行编译脚本 |
| `rog-battery.exe` | 构建产物，约 70 KB |
| `settings.ini` | 用户设置（自动生成，不入库）。包内版在 `%LOCALAPPDATA%\rog-battery\` |
| `history.tsv` | 电量采样历史（自动生成，删掉即重新开始攒）。包内版在 `%LOCALAPPDATA%\rog-battery\` |
| `msix/` | MSIX 打包脚本、manifest 模板、图标生成、发布参数 |
| `msix/publish-store.ps1` | Store Submission API 自动发布（`-WhatIf` / `-DryRun` 可空跑） |
| `docs/screenshots/` | 上架用的真实截图素材 |