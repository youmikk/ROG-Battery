# R Battery — 隐私政策 / Privacy Policy

**最后更新：2026 年 10 月 21 日**

## 中文

### 概述

R Battery（以下简称"本应用"）是一个在 Windows 任务栏显示无线鼠标电量的本地工具。
本应用**不收集、不上传、不共享任何个人信息**。

### 我们不收集什么

- 不收集姓名、邮箱、账号等任何身份信息
- 不收集设备标识符、硬件序列号
- 不收集使用行为、点击日志、崩溃报告
- 不收集位置信息

### 网络访问

本应用**不进行任何网络通信**。它没有联网能力，也不会尝试连接任何服务器。

### 本机存储的数据

本应用会在您电脑上写入以下文件，**全部仅存于本机**：

1. `%LOCALAPPDATA%\rog-battery\history.tsv`
   鼠标电量与充电状态的历史采样，用于估算剩余可用时间和充满时间。
   保留最近 48 小时，超出自动清理。

2. `%LOCALAPPDATA%\rog-battery\rog-battery.log`
   仅在您手动开启调试模式（设置环境变量 `ROG_BATTERY_DEBUG=1`）时生成，
   用于排查问题。默认不创建。

以上文件您可以随时直接删除，删除后本应用会重新开始累积数据，不影响正常使用。

### 注册表

若您在右键菜单中选择"开机自动启动"，本应用会写入当前用户的启动项：

- 绿色版：`HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`
- 商店版：通过 Windows 的 StartupTask 机制注册

该启动项可在应用内随时取消勾选移除，不需要管理员权限。

### USB 设备访问

本应用通过 Windows 提供的 HID 接口与您的鼠标通信，用于读取电量和 DPI。
它只读取，不修改设备配置，也不会访问其它 USB 设备的数据。

### 儿童隐私

本应用不面向儿童，也不收集任何年龄相关信息。

### 政策变更

若本政策发生变更，我们会更新本页面顶部的日期。由于本应用不收集任何数据，
变更的可能性很低。

### 联系方式

如有疑问，请通过 Microsoft Store 产品页面的"支持"链接联系我们。

---

## English

### Overview

R Battery ("the app") is a local utility that displays your wireless mouse's battery
level on the Windows taskbar. The app **does not collect, transmit, or share any
personal information**.

### What we do not collect

- No names, email addresses, or account information
- No device identifiers or hardware serial numbers
- No usage behavior, click logs, or crash reports
- No location information

### Network access

The app **makes no network connections of any kind**. It has no networking capability
and does not attempt to contact any server.

### Data stored on your device

The app writes the following files, **all stored locally on your machine**:

1. `%LOCALAPPDATA%\rog-battery\history.tsv`
   Historical samples of mouse battery level and charging state, used to estimate
   remaining time and time to full. Keeps the most recent 48 hours; older entries
   are pruned automatically.

2. `%LOCALAPPDATA%\rog-battery\rog-battery.log`
   Created only when you manually enable debug mode (environment variable
   `ROG_BATTERY_DEBUG=1`), for troubleshooting. Not created by default.

You may delete these files at any time. The app will simply start collecting fresh
data; nothing breaks.

### Registry

If you enable "Start with Windows" from the right-click menu, the app writes a
per-user startup entry:

- Portable build: `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`
- Store build: registered through the Windows StartupTask mechanism

You can remove it at any time by unchecking the same menu item. No administrator
rights are required.

### USB device access

The app communicates with your mouse through the HID interface provided by Windows
in order to read battery level and DPI. It only reads; it does not modify device
configuration and does not access data from other USB devices.

### Children's privacy

The app is not directed at children and collects no age-related information.

### Changes to this policy

If this policy changes, we will update the date at the top of this page. Because the
app collects no data, changes are unlikely.

### Contact

For questions, please reach out through the "Support" link on the Microsoft Store
product page.
