# Store 商店列表文案（R Battery）

Partner Center → Store listing → 中文（中国）/ English (United States) 两页分别粘贴。
所有内容已按微软政策核对：不使用 ROG/ASUS 作为产品名，仅作兼容性说明。

---

## 1. Product name 产品名称

**两页都保持 `R Battery`，不要选 `R 电池`。**

原因：产品名在整条 Store 链路里应当唯一——应用内自称、安装包身份
（`Sonder.RBattery`）、Store 链接、搜索结果都指向同一个字符串。分语言起不同
名字会让「已购买/已安装」显示不一致，也不利于搜索权重。中文用户搜不到
`R Battery` 的问题靠描述里的关键词解决，不靠改标题。

---

## 2. Description 描述（中文）

R Battery 把无线鼠标的剩余电量直接显示在 Windows 任务栏上——不用打开任何窗口，
不用装 Armoury Crate，扫一眼任务栏就知道还能撑多久。

■ 任务栏上直接看电量
托盘图标本身就是电量数字，浅色任务栏显示黑字、深色显示白字，和系统时钟一个观感。
低于 30% 自动变红，不用一直盯着看。

■ 鼠标移上去看详情
悬停即显示连接方式、充电状态、剩余电量、电压、当前 DPI 档位，以及预计还能用多久、
多久能充满。

■ 充电状态分三种，不是简单的「插着线就是在充」
区分「充电中」「已充满」「未充电」，避免把插着线但电量已满误判成还在充电。

■ 平时不打扰，也不折腾鼠标
电量每 30 秒刷新一次；DPI 只在你悬停或打开菜单时才去读，平时完全不碰设备，
不影响鼠标续航。

■ 一个 50 KB 的文件，没有任何依赖
不需要 Python、不需要 .NET SDK、不需要安装运行库。用 Windows 10/11 自带的
.NET Framework 即可，双击就能跑。

■ 开机自启
右键菜单一键勾选，写入当前用户的启动项，不需要管理员权限。

【支持的设备】
本应用通过 USB HID 直接与鼠标通信，目前验证过的型号：
· ASUS ROG Harpe Ace Mini（2.4G 无线 / 有线两种接法）

使用 ROG OMNI RECEIVER 接收器的其它型号可能可以读取，但帧格式未经验证，
不保证数值准确。读取失败时图标显示「?」，不会给出错误数字。

【关于 DPI】
只能读取，不能通过本应用修改。改 DPI 请用鼠标上的实体按键。

【隐私】
不联网、不收集任何数据。所有信息仅保存在本机。

本应用为第三方工具，与 ASUSTeK Computer Inc. 无隶属关系。ASUS、ROG 是其注册商标，
此处仅用于说明兼容性。

---

## 3. Description 描述（英文）

R Battery puts your wireless mouse's remaining battery right on the Windows taskbar —
no window to open, no Armoury Crate required. Just glance at the taskbar.

■ Battery percentage at a glance
The tray icon is the battery number itself, drawn in black on a light taskbar and
white on a dark one, matching the look of the system clock. It turns red below 30%.

■ Hover for details
Hover the icon to see connection type, charging state, remaining charge, voltage,
current DPI stage, and estimated time remaining or time to full.

■ Three charging states, not just "the cable is plugged in"
Distinguishes Charging / Fully charged / Not charging, so a connected cable that is
no longer charging is not reported as charging.

■ Quiet by default
Battery is refreshed every 30 seconds. DPI is read only while you hover or open the
menu — the device is left alone the rest of the time, so the app costs you no mouse
battery life.

■ One 50 KB file, no dependencies
No Python, no .NET SDK, no runtime to install. It uses the .NET Framework already
present on Windows 10/11. Double-click and it runs.

■ Start with Windows
One checkbox in the right-click menu. Writes a per-user startup entry; no admin
rights required.

[Supported devices]
This app talks to the mouse directly over USB HID. Verified with:
· ASUS ROG Harpe Ace Mini (2.4 GHz wireless and wired)

Other models on the ROG OMNI RECEIVER may work, but their frame formats are
unverified, so accuracy is not guaranteed. If a reading fails the icon shows "?"
rather than a wrong number.

[About DPI]
Read-only. This app cannot change DPI — use the physical button on the mouse.

[Privacy]
No network access, no data collection. Everything stays on your machine.

This is a third-party tool and is not affiliated with ASUSTeK Computer Inc.
ASUS and ROG are their trademarks, referenced here only to describe compatibility.

---

## 4. What's new in this version 此版本中的新功能

**留空。** 页面右侧已写明「Leave blank if this is the first submission for this
product」——首次提交必须留空，填了反而可能被审核问「上一版在哪」。

---

## 5. Product features 产品功能（中文，15 条）

任务栏图标直接显示电量百分比
浅色和深色任务栏自动适配字色
电量低于 30% 自动变红
悬停查看连接方式、电压和当前 DPI 档位
显示预计可用时间和预计充满时间
区分充电中、已充满、未充电三种状态
DPI 仅在需要时读取，尽量少消耗鼠标电量
电量每 30 秒自动刷新
左键单击立即刷新
右键菜单可设置开机自启
单个 50 KB 可执行文件，无需安装运行库
不依赖 Armoury Crate 或 GearLink
不联网，不收集任何用户数据
单实例保护，重复启动不会出现多个图标
资源管理器重启后图标自动恢复

---

## 6. Product features 产品功能（英文，15 条）

Battery percentage shown directly in the taskbar
Adapts text color to light and dark taskbars
Turns red automatically below 30%
Hover to see connection type, voltage, and current DPI stage
Estimated time remaining and estimated time to full
Distinguishes charging, fully charged, and not charging
Reads DPI only when needed, to minimize impact on mouse battery
Automatic battery refresh every 30 seconds
Click to refresh immediately
Start with Windows from the right-click menu
Single 50 KB executable with no runtime to install
No dependency on Armoury Crate or GearLink
No network access and no user data collected
Single-instance protection prevents duplicate tray icons
Icon recovers automatically after Explorer restarts

---

## 7. 还没填的必填项（提交前要准备）

| 字段 | 状态 | 说明 |
|---|---|---|
| Screenshot 截图 | **缺** | 至少 1 张，1366×768 或更大，PNG |
| Store logo | 已有 | `msix/Assets/` 里那套，但 Store listing 页可能要单独上传 |
| Copyright and trademark info | 缺 | 填 `Copyright © 2026 Sonder. Not affiliated with ASUSTeK Computer Inc.` |
| Support contact info | 缺 | 填一个能收信的邮箱；Store 会公开展示 |
| Privacy policy URL | **缺** | Win32/桌面应用强制要求，必须是一个可访问的 https 页面 |
| Website | 可选 | 可留空，填 GitHub 仓库地址更好 |
| Additional system requirements | 可选 | 留空 |

### 隐私政策的建议写法

必须放在**公网可访问的 https 地址**（GitHub 仓库里的 `.md` 文件页或 GitHub Pages
都可以）。内容一页足够：

1. 本应用不收集任何个人信息
2. 不联网、不上报任何数据
3. 唯一写入的数据是 `%LOCALAPPDATA%\rog-battery\` 下的电量历史（用于估算剩余时间），
   仅存本机，用户可随时删除
4. 开机自启项写入当前用户注册表，可在应用内关闭
5. 联系方式邮箱

---

## 8. 审核时可能被问的点（提前想好回答）

- **为什么需要 runFullTrust？** 因为是 Win32 托盘程序，需要访问 USB HID 设备读取
  鼠标电量，没有别的实现方式。
- **为什么标题不含 ROG 却描述 ROG？** 描述里是兼容性说明（nominative use），
  产品名刻意避开商标。
- **DPI 只读是不是功能不完整？** 描述里已明说，不构成误导。
