# -*- coding: utf-8 -*-
r"""把 Partner Center 导出的 listingData CSV 更新成 Rust 版（rog-mouse-ctl）的真实描述。

为什么要改：导出的那份是 **C# 旧版** 的文案，和现在的程序已经对不上了 ——
  · 说「一个 50 KB 的文件，用 .NET Framework」→ 现在是约 1 MB 的 Rust 静态链接 exe，零依赖
  · 说「DPI 只能读取，不能修改」→ 现在 DPI/回报率/去抖/LOD/按键都能写
  · 说「低于 30% 自动变红」→ 实际 ≤50% 橙、≤20% 红
  · 说「右键菜单勾选开机自启，写入启动项」→ 商店版走 startupTask（任务管理器启动列表）
  · 完全没提新增的电量页/性能页/按键页/灯光页、预计可用时间、48 小时曲线

做法：读原始 CSV（utf-8-sig），按 Field 名覆盖 zh-cn / en-us 两列，其余列与
所有图片 URL 原样保留，用 csv 模块重写（自动处理多行引号），输出到 msix\store\listingData.csv。

用法：python msix\store\make-listing.py [源CSV]
"""
import csv
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_SRC = r"E:\Downloads\listingData-9NRC8HCD4BZ7-1152921505702254904.csv"
OUT = os.path.join(HERE, "listingData.csv")

ZH_DESC = """R Battery 是一款轻量的 ROG 鼠标参数工具：任务栏上直接显示剩余电量与预计续航，界面里能改 DPI、回报率、去抖和按键，灯效则交给 Windows 动态光效统一管理。

■ 任务栏上直接看电量
托盘图标就是电量数字本身，字色自动跟随任务栏深浅（浅色任务栏深炭灰、深色任务栏纯白），大小与系统时钟、输入法一致。≤50% 转橙、≤20% 转红，一眼就知道该不该充电。鼠标移上去还能看到型号、连接方式（2.4G / 有线）、电量与充电状态、当前 DPI 档位。

■ 电量页：不只看百分比
环内是实时电量，环下是「预计还能用多久 / 约多久充满」—— 按最近 48 小时的真实采样做线性外推；样本不够时宁可显示「估算中」，也不给一个瞎猜的数字。再下面是 48 小时电量曲线，掉电快慢一目了然。

■ 性能页：DPI 四档、回报率、去抖、LOD、角度吸附
每档 DPI 都能单独设数值与指示灯颜色（100–42000，步进 50）。数值填好后点「切换」才真正切档，和「应用」分开，避免误触。回报率 125 / 250 / 500 / 1000 Hz 可选；按键去抖 0–32 ms；抬起距离 Low / High；角度吸附开关。

■ 按键页：改键与厂商默认对照
列出每个按键当前的映射，可以从下拉里改成别的功能；绑成「无功能」会二次确认，避免把键彻底废掉。支持导出当前配置、从备份恢复。

■ 灯光页：交给 Windows 动态光效
本机鼠标是标准 HID LampArray 设备，纯色 / 呼吸 / 渐变 / 波浪 / 闪烁都由 Windows 11「设置 → 个性化 → 动态光效」提供。本程序不再写厂商私有报文，避免和系统抢设备导致闪烁，只提供一键跳转与灯效设备检测。

■ 不打扰，也不折腾鼠标
电量默认每 60 秒读一次（可在设置里改）；DPI 只在打开页面或悬停时读取，其余时间不碰设备。

■ 一个约 1 MB 的程序，零依赖
Rust 静态链接，不需要 .NET、不需要 Python、不需要额外运行库，也不需要管理员权限。配置写在 %APPDATA%（商店版由系统重定向到应用数据目录），卸载即清理。

■ 开机自启
在设置页一键开关。绿色版写入当前用户的启动项（不需要管理员权限）；商店版走系统「启动应用」列表，可随时在任务管理器里关掉。

【支持的设备】
本工具面向 ROG 鼠标这一套私有 HID 参数协议，不是某个型号的专属工具。通过 USB HID 直接与鼠标通信，目前完整验证过的设备：
· ROG HARPE ACE MINI（2.4G 无线 / 有线两种接法）
使用 ROG OMNI RECEIVER 的其它型号可能可以读取，但帧格式未经验证，不保证数值准确；读取失败时界面显示「—」，不会给出错误数字。

【系统要求】
Windows 10 版本 1809（build 17763）或更高。灯效跳转与动态光效需要 Windows 11。

【隐私】
不联网、不收集任何数据。所有信息仅保存在本机。

本应用为第三方工具，与 ASUSTeK Computer Inc. 无隶属关系。ASUS、ROG 是其注册商标，此处仅用于说明兼容性。"""

EN_DESC = """R Battery is a portable settings tool for ROG mice: it puts your remaining battery and estimated runtime right on the Windows taskbar, lets you change DPI, polling rate, debounce and button mappings, and hands lighting over to Windows Dynamic Lighting.

■ Battery percentage, right in the taskbar
The tray icon is the number itself, drawn in charcoal on a light taskbar and white on a dark one, matching the size of the system clock. It turns orange at 50% and red at 20%, so you know at a glance whether to plug in. Hover it for the model, connection type (2.4 GHz / wired), charge level and charging state, and the current DPI stage.

■ Battery page: more than a percentage
The ring shows the live level; right below it you get "about N days left" or "about N minutes to full", extrapolated from up to 48 hours of real samples. When there is not enough data it says "estimating" instead of inventing a number. Below that is a 48-hour chart so you can see how fast the battery is actually draining.

■ Performance page: four DPI stages, polling rate, debounce, LOD, angle snapping
Each DPI stage has its own value and indicator colour (100–42000, step 50). You fill in the value and press "Apply", then press "Switch" to actually change stage — the two are separate so you cannot change stage by accident. Polling rate 125 / 250 / 500 / 1000 Hz, button debounce 0–32 ms, lift-off distance Low / High, angle snapping on/off.

■ Buttons page: remap, and compare with vendor defaults
Every button is listed with its current mapping and can be reassigned from a dropdown. Binding a button to "no function" asks for confirmation first, so you cannot disable a button by accident. You can export the current configuration and restore from a backup.

■ Lighting page: delegated to Windows Dynamic Lighting
This mouse is a standard HID LampArray device, so solid / breathing / gradient / wave / blink patterns all come from Windows 11 Settings → Personalisation → Dynamic Lighting. The app deliberately does not write vendor-private lighting packets, which would fight the system over the device and cause flicker. It offers a one-click shortcut and device detection.

■ Quiet by default
Battery is read once every 60 seconds (configurable in Settings). DPI is only read when you open the page or hover the tray icon; the rest of the time the device is left alone.

■ One ~1 MB program, no dependencies
Statically linked Rust. No .NET, no Python, no runtime to install, and no administrator rights required. Settings live in %APPDATA% (redirected to the app data folder for the Store build), so uninstalling cleans up after itself.

■ Start with Windows
A single switch in Settings. The portable build writes a per-user startup entry (no admin rights); the Store build uses the system Startup Apps list, which you can turn off in Task Manager at any time.

[Supported devices]
This app targets the private HID parameter protocol shared by ROG mice; it is not tied to one model. It talks to the mouse directly over USB HID. Fully verified with:
· ROG HARPE ACE MINI (2.4 GHz wireless and wired)
Other models on the ROG OMNI RECEIVER may work, but their frame formats are unverified, so accuracy is not guaranteed. When a reading fails the UI shows a dash rather than a wrong number.

[System requirements]
Windows 10 version 1809 (build 17763) or later. The Dynamic Lighting shortcut and lighting control require Windows 11.

[Privacy]
No network access, no data collection. Everything stays on your machine.

This is a third-party tool and is not affiliated with ASUSTeK Computer Inc. ASUS and ROG are their trademarks, referenced here only to describe compatibility."""

ZH_SHORT = "轻量的 ROG 鼠标工具：任务栏直接显示电量与预计续航，还能改 DPI、回报率、去抖和按键；灯效交给 Windows 动态光效。"
EN_SHORT = "Portable ROG mouse tool: battery and runtime estimate in the taskbar, plus DPI, polling rate, debounce and button remapping. Lighting is delegated to Windows Dynamic Lighting."

ZH_COPY = "© 2026 Sonder。与 ASUSTeK Computer Inc. 无隶属关系。ASUS、ROG 为其注册商标，仅用于说明兼容性。"
EN_COPY = "© 2026 Sonder. Not affiliated with ASUSTeK Computer Inc. ASUS and ROG are trademarks of their respective owners, referenced only to describe compatibility."

FEATURES_ZH = [
    "任务栏图标直接显示电量百分比，自动适配深浅任务栏；≤50% 转橙、≤20% 转红",
    "电量页：环内实时电量 + 环下预计可用 / 预计充满时间 + 48 小时历史曲线",
    "DPI 四档可改（100–42000，步进 50），每档可设指示灯颜色，「切换」与「应用」分离防误触",
    "回报率 125/250/500/1000 Hz、按键去抖 0–32 ms、抬起距离 Low/High、角度吸附",
    "按键映射可改，可导出配置、从备份恢复；绑成「无功能」前二次确认",
    "灯效交给 Windows 11 动态光效，本程序不抢设备、不写私有报文",
    "约 1 MB、零依赖、不需要管理员权限（Rust 静态链接）",
    "不联网、不收集数据；电量默认每 60 秒刷新，DPI 只在需要时读取",
]
FEATURES_EN = [
    "Battery percentage drawn directly in the taskbar, adapting to light and dark taskbars; orange at 50%, red at 20%",
    "Battery page: live level in the ring, estimated runtime or time to full below it, and a 48-hour history chart",
    "Four editable DPI stages (100–42000, step 50) each with its own indicator colour; Switch and Apply are separate",
    "Polling rate 125/250/500/1000 Hz, debounce 0–32 ms, lift-off distance Low/High, angle snapping",
    "Button remapping with export and restore-from-backup; binding to \"no function\" asks for confirmation",
    "Lighting delegated to Windows 11 Dynamic Lighting — the app never fights the system for the device",
    "About 1 MB, no dependencies, no administrator rights (statically linked Rust)",
    "No network access, no telemetry; battery refreshed every 60 s, DPI read only when needed",
]

CAPTIONS_ZH = [
    "电量页：实时电量、预计可用时间与 48 小时曲线",
    "性能页：DPI 四档、回报率与按键去抖",
    "按键页：按键映射、导出与备份恢复",
    "灯光页：交给 Windows 动态光效",
    "设置页：跟随系统配色、开机自启与存储路径",
    "深色主题：随系统自动切换",
]
CAPTIONS_EN = [
    "Battery page: live level, estimated runtime and a 48-hour chart",
    "Performance page: four DPI stages, polling rate and debounce",
    "Buttons page: remapping with export and backup restore",
    "Lighting page: delegated to Windows Dynamic Lighting",
    "Settings page: system theme, start with Windows, storage location",
    "Dark theme that follows the system automatically",
]

TERMS_ZH = ["鼠标电量", "ROG", "鼠标 DPI", "任务栏电量", "外设工具", "轻量工具"]
TERMS_EN = ["mouse battery", "ROG", "mouse DPI", "taskbar battery", "peripheral tool", "portable"]

MINREQ_ZH = [
    "操作系统：Windows 10 版本 1809（build 17763）或更高",
    "USB 端口，或 ROG OMNI RECEIVER 无线接收器",
    "支持 USB HID 的 ROG 鼠标（已验证 Harpe Ace Mini）",
]
MINREQ_EN = [
    "OS: Windows 10 version 1809 (build 17763) or later",
    "A USB port, or the ROG OMNI RECEIVER wireless dongle",
    "A ROG mouse exposing USB HID (verified: Harpe Ace Mini)",
]


def build_updates():
    u = {
        "Description": (ZH_DESC, EN_DESC),
        "ShortDescription": (ZH_SHORT, EN_SHORT),
        "CopyrightTrademarkInformation": (ZH_COPY, EN_COPY),
        "DevStudio": ("Sonder", "Sonder"),
    }
    for i, (zh, en) in enumerate(zip(FEATURES_ZH, FEATURES_EN), start=1):
        u["Feature%d" % i] = (zh, en)
    for i, (zh, en) in enumerate(zip(CAPTIONS_ZH, CAPTIONS_EN), start=1):
        u["DesktopScreenshotCaption%d" % i] = (zh, en)
    for i, (zh, en) in enumerate(zip(TERMS_ZH, TERMS_EN), start=1):
        u["SearchTerm%d" % i] = (zh, en)
    for i, (zh, en) in enumerate(zip(MINREQ_ZH, MINREQ_EN), start=1):
        u["MinimumHardwareReq%d" % i] = (zh, en)
    return u


def main():
    src = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_SRC
    with open(src, "r", encoding="utf-8-sig", newline="") as f:
        rows = list(csv.reader(f))

    head = rows[0]
    try:
        i_field = head.index("Field")
        i_zh = head.index("zh-cn")
        i_en = head.index("en-us")
    except ValueError:
        raise SystemExit("表头不认识：%r" % (head,))

    updates = build_updates()
    seen = set()
    out = [head]
    for row in rows[1:]:
        if not row or not row[i_field]:
            continue  # 丢掉文件末尾的空行
        name = row[i_field]
        if name in updates:
            zh, en = updates[name]
            while len(row) <= max(i_zh, i_en):
                row.append("")
            row[i_zh] = zh
            row[i_en] = en
            seen.add(name)
        out.append(row)

    missing = sorted(set(updates) - seen)
    os.makedirs(HERE, exist_ok=True)
    with open(OUT, "w", encoding="utf-8-sig", newline="") as f:
        csv.writer(f).writerows(out)

    print("写入 %s（%d 行，%d 字节）" % (OUT, len(out), os.path.getsize(OUT)))
    print("已更新 %d 个字段" % len(seen))
    if missing:
        print("警告：这些字段在源文件里没找到 -> %s" % ", ".join(missing))
    for key in ("Description", "ShortDescription", "CopyrightTrademarkInformation"):
        zh, en = updates[key]
        print("  %-32s zh %4d 字 / en %4d 字符" % (key, len(zh), len(en)))


if __name__ == "__main__":
    main()
