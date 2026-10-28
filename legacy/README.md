# 旧版（C# / .NET Framework）

这里的文件是第一版实现，**已经不再维护**，留着是为了查资料：

- `rog-battery.cs`、`settings.cs`、`tray.cs`、`devices.cs`、`build.cmd` —— 2026-10 的 C# 版，
  当时只有托盘电量图标 + 右键菜单，不能改 DPI、不能看历史。
- `screenshots/` —— 那个版本的界面截图（已过时，别拿去做宣传图）。
- `make-assets.ps1`、`store-listing.md` —— 当时的 MSIX 素材生成脚本与商店文案。

现在的程序是 Rust 版（仓库根目录），界面和功能都已重做：

| | 旧版 C# | 现在的 Rust 版 |
| --- | --- | --- |
| 体积 | 需要 .NET Framework | 单个约 1 MB 的 exe，零依赖 |
| 界面 | 托盘图标 + 右键菜单 | 五页自绘界面（电量 / 性能 / 灯光 / 按键 / 设置） |
| 能改什么 | 只能读 | DPI、回报率、去抖、LOD、按键映射、灯效跳转 |
| 历史 | 无 | 48 小时曲线 + 预计可用时间 |
| 主题 | 固定 | 跟随系统 / 浅色 / 深色 |

`devices.cs` 里的 PID 型号表已经整理成 [`docs/device-support.md`](../docs/device-support.md)，
那是这份旧代码里唯一还很有用的东西（102 个白名单 PID 里 75 个有电池的型号名）。
