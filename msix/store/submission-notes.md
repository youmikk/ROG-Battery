# 提交认证时要填的话（复制粘贴用）

Partner Center 每次**新建一次提交**都会问一遍下面这些，但都是表单字段，不用改代码、
不用改 manifest。文本存这里，下次开提交直接复制，几十秒的事。

---

## 1. runFullTrust 用途说明

`AppxManifest.xml` 里唯一的能力声明就是 `<rescap:Capability Name="runFullTrust" />`，
提交时「能力声明 / Capability declaration」那一步会要求写用途。

**中文（推荐直接用这句）**

```
桌面桥应用（Windows.FullTrustApplication）。程序需要以当前用户身份打开 USB HID
设备，读取鼠标电量与参数（DPI、回报率、去抖、按键映射），并写入用户修改后的设置。
不联网、不收集任何数据、不需要管理员权限。
```

**English**

```
Desktop Bridge app (Windows.FullTrustApplication). The app must open the USB HID
device as the signed-in user to read mouse battery level and settings (DPI, polling
rate, debounce, button mapping) and to write back the values the user changes.
It does not access the network, does not collect any data, and does not require
administrator rights.
```

> 一句话版本也够：`桌面桥应用，需要按用户身份打开 HID 设备读写鼠标参数。`
> 但把「不联网 / 不收集数据 / 不要管理员」一起写上，能省掉一轮追问。

---

## 2. 认证说明（Notes for certification）

这一步比能力声明更重要 —— 审核员手里没有 ROG 鼠标，**不写清楚会以「无法验证功能」打回**。

**中文**

```
本应用是 ROG 鼠标（ASUS）的配套工具，需要连接受支持的实体鼠标才能读到数据。

已验证设备：ASUS ROG Harpe Ace Mini（USB 有线 / ROG OMNI RECEIVER 2.4G 无线）。
未连接设备时的表现：界面正常启动，数值位置显示「—」，不会崩溃、不会报错弹窗，
设置页、按键页、灯光页都可以正常浏览。请在没有设备的情况下按此预期验收界面。

功能说明：
- 电量页：从 HID 设备读取电量百分比与电压，并按本机历史采样估算可用时间。
- 性能页：读取/写入 DPI 四档、回报率、按键去抖、抬起距离、角度吸附。
- 按键页：读取/写入 8 个按键的映射，支持导出与从备份恢复。
- 灯光页：本机鼠标是标准 HID LampArray 设备，灯效由 Windows 11「动态光效」提供，
  应用只提供跳转按钮与设备检测，不写厂商私有报文。
- 设置页：开机自启、托盘显示、刷新间隔、配色（跟随系统/浅色/深色）、存储路径。

权限说明：只声明 runFullTrust，用于以当前用户身份访问 USB HID 设备。
不访问网络，不收集或上传任何用户数据，所有配置与历史都只保存在本机。
```

**English**

```
This is a companion utility for ASUS ROG mice. It reads data from a supported
physical mouse, so full functionality cannot be exercised without the hardware.

Verified device: ASUS ROG Harpe Ace Mini (USB wired / ROG OMNI RECEIVER 2.4 GHz).
Behaviour with no device attached: the app starts normally, numeric fields show a
dash ("—"), nothing crashes and no error dialog appears; the Settings, Buttons and
Lighting pages remain fully browsable. Please verify the UI against this expectation.

What the app does:
- Battery page: reads level and voltage over HID; estimates remaining runtime from
  locally stored samples.
- Performance page: reads and writes four DPI stages, polling rate, button debounce,
  lift-off distance and angle snapping.
- Buttons page: reads and writes the mapping of eight buttons; export and restore.
- Lighting page: the device is a standard HID LampArray; lighting is provided by
  Windows 11 Dynamic Lighting. The app only offers a shortcut and device detection
  and does not write any vendor-private report.
- Settings page: start with Windows, tray behaviour, refresh interval, colour scheme
  (follow system / light / dark), storage location.

Permissions: only runFullTrust is declared, used to access the USB HID device as the
signed-in user. No network access, no data collection or upload; settings and history
stay on the local machine.
```

---

## 3. 隐私

程序不联网、不收集数据，所以隐私政策只需要一页说明。若列表页强制要 URL，用仓库里
的 `PRIVACY.md`（或 GitHub Pages 指向它）即可，正文三句话：

```
本应用不收集、不存储、不上传任何个人信息。
电量历史与设置仅保存在本机（%APPDATA% 与 %LOCALAPPDATA%，商店版由系统重定向到应用数据目录）。
应用不包含任何网络请求代码。
```

---

## 4. 第一次 vs 之后

**第一次（只有这次要手动填）**：在 Partner Center 建产品 → 填商店列表 → 传包 →
填能力声明与认证说明 → 提交认证。这一轮的东西会**随提交一起存下来**。

**之后每次更新**：新建提交时，Partner Center 会把上一次的列表、定价、可用性、
能力声明都克隆过来，你只需要：

1. 传新包（`*.msixupload`）；
2. 如果界面变了，换截图；
3. 点「提交认证」。

所以「要写的东西」只有第一次。审核本身永远要过一遍，这个不叫自动化。

## 5. 想连点击都省掉：一条命令发版

`msix\publish.ps1` 把「编译 → 打 msix → 校验 → 打 msixupload → 提交」串起来，
走的是微软官方的 **Microsoft Store Developer CLI**（不是野路子爬接口）。

一次性准备（只做一次）：

```powershell
winget install --id Microsoft.StoreDeveloperCLI -e
# 重开终端，确认：msstore --version
```

再在 Entra ID 建一个应用注册，给它 Partner Center 的 Manager 角色，拿到
tenantId / clientId / clientSecret；sellerId 在 Partner Center「帐户设置 → 组织标识」。
凭据二选一：环境变量 `MSTORE_TENANT_ID` / `MSTORE_SELLER_ID` / `MSTORE_CLIENT_ID` /
`MSTORE_CLIENT_SECRET`，或者交互式 `msstore reconfigure`。

之后发版就是：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File msix\publish.ps1
# 只想先建提交、不点提交认证：  -DryRun
# 已经打好包只想重传：          -SkipBuild -Version 1.1.1.0
```

脚本会先跑 `build-msix.ps1 -Verify`，校验不过直接停 —— 别把会被打回的包传上去。
CLI 的开关名以 `msstore publish --help` 为准（版本间略有差异）。
