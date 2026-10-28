# 商店页素材（Partner Center）

上架时要填/要传的东西都在这个目录里。**别手改 `listingData.csv`**——它由
`make-listing.py` 生成，改了下次重跑就被覆盖。

## 交付件

| 文件 | 传到哪里 | 说明 |
| --- | --- | --- |
| `listingData.csv` | Partner Center → 产品 → 商店列表 → **导入/导出** | 从原导出表改出来的，430 行字段顺序、图片 URL 全部原样保留，只换了文案 |
| `screenshots\01..06*.png` | 商店列表 → 桌面截图（至少 1 张） | 1920×1080，程序原生 1100×740 合成到画布上（渐变底 + 圆角 + 投影） |
| submission-notes.md | 能力声明 / 认证说明 / 隐私 | runFullTrust 用途说明、Notes for certification、每次提交的固定动作，复制粘贴用 |
| ..\AppPackages\*.msixupload | 包 → 上传新包 | 只含 msix 本体；三张商店图标已在包内（manifest 已声明），别再重复塞进去，会被包接受验证打回 |

## 改文案

1. 编辑 `make-listing.py` 里的 `ZH_DESC` / `ZH_SHORT` / `FEATURES_ZH` / `CAPTIONS_ZH` 等常量；
2. `python msix\store\make-listing.py` 重新生成 `listingData.csv`；
3. 上传前跑一遍校验（会打印每个字段的中英文长度）：

```powershell
python msix\store\make-listing.py
```

## 重出截图

截图源是 `scratch\` 里的界面渲染图（`cargo test 渲染前端ui效果图` 产出 BMP，转 PNG 后同目录）。
改界面后重新跑那个测试再执行：

```powershell
python msix\store\make-screenshots.py
```

`make-screenshots.py` 顶部的 `SHOTS` 列表决定用哪几张、配明底还是暗底。
两条踩过的坑写在注释里：

- 电量页的测试 fixture 电压是 `0 mV`、历史 `0 个采样点`，看着像坏界面 —— **不能**拿来上架；
- 按键页 fixture 必须给 `buttons: BUTTON_SRC`，否则八行全显示「无功能」，像把鼠标按键废了。

## 文案里的事实口径

写文案前先对一遍程序里的真实默认值，别照抄旧版（旧的 C# 版说「50 KB / .NET」「DPI 只能读」，
全都不成立了）：

- 电量刷新间隔默认 **60 秒**（`src/settings.rs` 的 `refresh_interval_sec`）；读数缓存 `cache_ttl_sec` 默认 30。
- 托盘字色阈值（`src/tray_icon.rs` 的 `glyph_color`）：≤20% 红、21–50% 橙、其余跟任务栏深浅取白/深炭灰。
- DPI 四档 100–42000、步进 50；「切换」与「应用」是两个动作。
- 回报率 125 / 250 / 500 / 1000 Hz；去抖 0–32 ms；LOD Low/High；角度吸附开关。
- 灯效交给 Windows 动态光效（本机是标准 HID LampArray，`UsagePage 0x59`）。
- 开机自启：绿色版写 `HKCU\...\Run`（失败自动降级到「启动」文件夹的 vbs）；商店版走 WinRT StartupTask。
- 配置位置：`%APPDATA%\rog-mouse-ctl\settings.ini`、`%LOCALAPPDATA%\rog-battery\history.tsv`；
  商店版由 MSIX 重定向到 `%LOCALAPPDATA%\Packages\Sonder.RBattery_*\LocalCache\...`。

## 已知待办

- exe 仍是 `subsystem=3 (WINDOWS_CUI)`，从开始菜单启动可能闪一下黑框；要彻底干净得改
  `/SUBSYSTEM:WINDOWS` + `AttachConsole(ATTACH_PARENT_PROCESS)`。
- exe 没有 VERSIONINFO（属性页里文件版本为空），商店不强制，但显得糙。
- `Cargo.toml` 的 `version = "0.1.0"` 与 MSIX 的 `1.1.0.0` 没对齐。
