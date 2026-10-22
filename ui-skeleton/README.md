# GUI 骨架（已实测验证）

给 `rog-mouse-ctl` 的 GUI 部分定的骨架。**全部代码都在真机上编译、运行、截图验证过**，不是理论示例。

- `main.rs.example` —— 完整可编译的 UI 骨架
- `Cargo.toml.example` —— 依赖清单
- `screenshot-tab0.png` / `screenshot-tab1.png` —— 真机截图（电量页 / 性能页）

## 验证环境

| 项 | 值 |
|---|---|
| rustc | 1.99.1 (58cf8f9c3 2026-10-13) |
| cargo | 1.99.1 (5f94df478 2026-08-27) |
| host triple | `x86_64-pc-windows-gnu`（MinGW，**不是** MSVC） |
| native-windows-gui | 1.0.13 |
| 编译耗时 | **2.4 秒**（release） |
| 产物大小 | **314.5 KB** |
| 警告 | **0** |

## 为什么选 native-windows-gui 而不是 egui / iced

**egui / eframe 这条路在这台机器上走不通**，试了两次都是同一个根因：

```
error: error calling dlltool 'dlltool.exe': program not found
error: dlltool could not create import library with ...kernel32.dll_imports.def
```

`dlltool.exe` 是存在的（在 rustup 的 `self-contained` 目录里，1,328,640 字节），
但它需要 GNU 汇编器 `as.exe` 来组装 `.def` 文件，而**这台机器上 `as` / `gcc` / `clang` 一个都没有**。
rustup 的 `self-contained` 是**残缺 MinGW**：有 43 个 Windows import library、有 `ld.exe`、有 `dlltool.exe`，
**没有汇编器**。任何走 `raw-dylib` / 生成 `.def` 的 crate（`parking_lot_core`、`windows` crate）必死。

`native-windows-gui` 完全不碰 dlltool —— 它只 `#[link]` 已存在的 DLL
（`user32` `comctl32` `kernel32` `ntdll` `msvcrt` `ole32`），链接直接成功。

## 已实测通过的验证项

| 验证 | 结果 |
|---|---|
| 编译 | 2.4 秒，0 警告，314.5 KB |
| 窗口 + 中文标题 | ✅ `ROG Harpe Ace Mini` |
| 分页 5 个中文标签 | ✅ 电量 / 性能 / 灯光 / 按键 / 设置 全部正常渲染 |
| 切页 | ✅ 电量页控件 → 性能页 ComboBox + TrackBar 正常切换 |
| 页面内控件 | ✅ Label / ProgressBar / Button / ComboBox / TrackBar / CheckBox |
| 托盘图标注册 | ✅ `Shell_NotifyIconGetRect` 返回 `hr=0`，矩形 `L=2251 T=1392 R=2283 B=1440` |
| 托盘右键菜单 | ✅ `Menu` + `MenuItem`，`OnContextMenu` → `menu.popup(x, y)` |

托盘图标的验证方法（供后续回归用）：

```powershell
# nwg 的 MessageWindow 类名是 NativeWindowsGuiWindow，父句柄是 HWND_MESSAGE(-3)
# 所以 EnumWindows 枚举不到，必须用 FindWindowEx 带父窗口
$mh = [Win32]::FindWindowEx([IntPtr](-3), [IntPtr]::Zero, "NativeWindowsGuiWindow", $null)
# 再用 message-only hwnd + uID=0 查图标矩形
$hr = [Win32]::Shell_NotifyIconGetRect([ref]$nid, [ref]$rect)   # hr=0 表示注册成功
```

## 11 个已踩过的坑（全部在源码注释里标了行号）

1. `TabsContainer` 高度必须给够（≥340），给 60 会导致标签条不渲染
2. 控件类名是 `TabsContainer` + `Tab`，**不是** `Tabs`
3. `TrayNotification` 的 parent 必须是 `MessageWindow`，不能是普通 `Window`
4. `TrayNotification::build` 时 icon 为 `None` 会 **panic**
   （`src\controls\tray_notification.rs:370` → `panic!("Tray notification requires an Icon at creation")`）
5. `OemIcon` 只有 6 个变体：`Sample=32512` `Ques=32514` `WinLogo=32517`
   `Warning=32515` `Error=32513` `Information=32516`。**没有 `Application`**
   （`src\resources\system_images.rs:82`）
6. `Menu` 没有 `append()`，每个 `MenuItem` 都要显式 `.parent(&menu)`
7. `Timer` 已 deprecated，用 `AnimationTimer`：
   `.interval(Duration)` + `.active(true)`，**不是** `.interval(30000)` + `.stopped(false)`
8. `TrayNotification::set_tip(&self, tip: &str)` —— **不接受 `Option`**
   （builder 的 `.tip()` 才收 `Option<&str>`）
9. `Window` 只有 `minimize()`，**没有 `set_minimized(bool)`**；从托盘恢复用 `set_visible(true)`
10. **页面控件必须 parent 到 `Tab`，不能 parent 到 `Window`**
    `Tab` 是真正的子窗口（类名 `NWG_TAB`，`WS_CHILD | WS_CLIPCHILDREN`），
    `Tab::init` 会把它 resize 成和 tab 视图一样大并把所有 tab 隐藏。
    挂到 `Window` 上会被 Tab 容器遮挡。
11. 页面坐标是**相对 Tab 客户区**的，左上角约 `(4, 26)` 起（标签条下方）

## 两个非 nwg 的坑

**PowerShell 5.1 改 Rust 源文件会毁中文**：
`Get-Content -Raw` 在 5.1 里默认按系统 ANSI(GBK) 解码，读 UTF-8 源文件会把中文写成乱码
（实测「开机自动启动」→「寮€鏈鸿嚜鍔ㄥ惎鍔?」）。要改请用编辑器，或显式 `-Encoding UTF8`。

**截图验证用 `PrintWindow`，不要用 `CopyFromScreen`**：
`CopyFromScreen` 会被其它窗口遮挡污染（实测截到一半是别的窗口的黑块）。
正确姿势：
```csharp
[DllImport("user32.dll")] static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint f);
// f = 2 (PW_RENDERFULLCONTENT)
```

## 骨架里没做但真实实现要补的

- **分页显隐**：nwg 的 `Tab` 切页是 Win32 自己管的（`TCM_SETCURSEL` 会触发
  `TCN_SELCHANGE` → nwg 内部显示对应 `NWG_TAB`），但**用 `TCM_SETCURSEL` 消息
  程序化切页不会触发通知**，页不会真的切。要程序化切页得自己发
  `WM_NOTIFY + TCN_SELCHANGE`，或者干脆只让用户点。
- **HID 轮询放后台线程**：`nwg::Notice` 的 `sender.notice()` 从任意线程唤醒 GUI 线程，
  这是把 `snapshot.rs` 的读取结果搬进 UI 的正确方式。**绝不能在 GUI 线程里做 HID 读取**
  （休眠时一次失败要 1.3 秒，UI 会冻结）。
- **托盘开关**：`tray.set_visible(bool)` 可以隐藏/显示图标，对应「设置里打开托盘」的需求。
