# native-windows-gui 1.0.13 实测踩坑记录

本文件只记录**我亲自复现过**的问题，每条都带源码位置和验证方法。
不是从文档抄的，是从编译错误、真机截图、像素采样、源码阅读里得来的。

工具链背景：`x86_64-pc-windows-gnu`（Rust 自带残缺 MinGW，**有 import library
但没有 `as.exe`**）→ 任何走 `raw-dylib` 的 crate 必死。nwg 只 `#[link]` 系统
已有 DLL（`user32` / `comctl32` / `kernel32` / `ntdll` / `msvcrt` / `ole32`），
所以它是这套环境上唯一能用的 GUI 框架（`eframe`/`egui` 会经 `parking_lot_core`
触发 `dlltool` 生成 `.def`，而 `dlltool` 需要 `as.exe` → 实测两次失败）。

---

## 1. `Label` 在 `Tab` 里默认会画灰底方块（`background_color` 必须显式给）

**症状**：Tab 页里每个 `Label` 都渲染成一块灰底矩形（`#F0F0F0`），
而页面空白处是白色（`#FFFFFF`）。看着像控件没对齐、或者占位框没删。

**关键**：`PrintWindow` 和真实 `CopyFromScreen` 两种抓法结果**一致**，
所以不是截图伪影，是真实渲染。

**根因**（`src/controls/label.rs`）：`Label` 的 `background_color` 默认是
`None`，而**只有 `bg.is_some()` 时才会绑定 `WM_CTLCOLORSTATIC` 处理器**：

```rust
// label.rs:234 —— 注意这个 if
if bg.is_some() {
    let handler0 = bind_raw_event_handler_inner(&parent_handle, handle as UINT_PTR, move |_hwnd, msg, _w, l| {
        match msg {
            WM_CTLCOLORSTATIC => {
                let child = l as HWND;
                if child == handle {
                    return Some(brush as LRESULT);
                }
            },
            _ => {}
        }
        None
    });
    ...
}
```

不设 `background_color` → 不绑 handler → `Static` 用系统默认的
`COLOR_BTNFACE`（`#F0F0F0`）铺满自己的矩形 → 灰块。

**修法**：给页面上的每个 `Label` 显式指定和页面一致的背景色。

```rust
const PAGE_BG: [u8; 3] = [255, 255, 255];

nwg::Label::builder()
    .text("电量")
    .parent(page)
    .background_color(Some(PAGE_BG))   // 必需，不是可选美化
    .build(&mut lbl)?;
```

**实测验证**（`PrintWindow(h, hdc, 2)` 后逐像素采样）：

| 采样点 | 不设 background_color | 设成 `[255,255,255]` |
|---|---|---|
| 状态行 Label 内部 | `#F0F0F0` | `#FFFFFF` |
| Label 右侧空白 | `#FFFFFF` | `#FFFFFF` |
| 页深处空白 | `#FFFFFF` | `#FFFFFF` |

> **一个我犯过的错，写在这里免得后人重踩**
>
> 我曾据「`label.rs` 把 handler 绑在 `parent_handle` 上」推断这是 nwg 的
> bug，理由是「`Tab` 不转发 `WM_CTLCOLORSTATIC`」。**这个推断是错的**：
> `WM_CTLCOLORSTATIC` 的语义本来就是「子控件发消息给父窗口」，绑在
> parent 上正是正确做法。
>
> 更根本的错误是**验证次序**：我量的二进制是 `PAGE_BG` 写进源码**之前**
> 的版本，却拿它的现象去质疑**之后**的源码。凡是「代码看着有 bug」的
> 结论，先确认量的是不是那版代码编出来的产物。


---

## 2. `TrayNotification` 必须给 icon，否则 panic（不是返回 `Err`）

```rust
// src/controls/tray_notification.rs:370
panic!("Tray notification requires an Icon at creation")
```

`OemIcon` 只有 6 个系统变体，**没有「应用图标」**：

```rust
// src/resources/system_images.rs:82
Sample = 32512, Ques = 32514, WinLogo = 32517,
Warning = 32515, Error = 32513, Information = 32516
```

所以必须 embed 一个真实 `.ico`：见 [tray-icon.md](tray-icon.md)。

`panic = "abort"` 下这个 panic 会直接 abort 进程（不 unwind），
所以没法用 `catch_unwind` 兜 —— 只能保证「永远给 icon」。

## 3. `TrayNotification` 的 parent 必须是 `MessageWindow`

不能是普通 `Window`。而 `MessageWindow` 的父句柄是 `HWND_MESSAGE(-3)`，
所以**用 `EnumWindows` 枚举不到它**，验证托盘是否注册成功必须：

```powershell
$mh = [Win32]::FindWindowEx([IntPtr](-3), [IntPtr]::Zero, "NativeWindowsGuiWindow", $null)
$nid = @{ cbSize=24; hWnd=$mh; uID=0; guidItem=[Guid]::Empty }
$hr = [Win32]::Shell_NotifyIconGetRect([ref]$nid, [ref]$rect)   # hr=0 = 注册成功
```

对照：C# 版用的 `RogBatteryTrayWnd` 是**顶层窗口**（`FindWindowEx(0, ...)` 能找到），
`Shell_NotifyIconGetRect` 返 `hr=-2147467259`(E_FAIL) —— 说明 message-only
方案才是正解。

## 4. 分页控件叫 `TabsContainer` + `Tab`，不是 `Tabs`

```rust
nwg::TabsContainer::builder()
    .parent(&window)
    .position((12, 12))
    .size((740, 520))       // 高度必须给够！
    .build(&mut tabs)?;

for name in ["电量", "性能", "灯光", "按键", "设置"] {
    let mut t = nwg::Tab::default();
    nwg::Tab::builder().text(name).parent(&tabs).build(&mut t)?;
}
```

**高度给 60 会导致标签条完全不渲染。** 520 实测正常。

## 5. 页面控件必须 parent 到 `Tab`，不能 parent 到 `Window`

`Tab` 是真正的子窗口（类名 `NWG_TAB`，`forced_flags = WS_CHILD | WS_CLIPCHILDREN`），
`Tab::init` 会把它 resize 成和 tab 视图一样大并把所有 tab 隐藏。
挂到 `Window` 上的控件会被 Tab 容器**遮挡**（实测第一版截图里只看到
ProgressBar 和 CheckBox 露在外面）。

页面坐标是**相对 Tab 客户区**，左上角约 `(4, 26)` 起（标签条下方）。

## 6. `TCM_SETCURSEL` 不会触发 `TCN_SELCHANGE`

`0x130C` 只改选中项，**不触发通知**，所以程序化切页时页面不真的切。
要程序化切页得自己发 `WM_NOTIFY + TCN_SELCHANGE`，或者只让用户点。

（实测：往 `SysTabControl32` 发 `WM_KEYDOWN VK_RIGHT(0x27)` 能正常切页，
且页面跟着变。）

## 7. `Window` 只有 `minimize()`，没有 `set_minimized(bool)`

从托盘恢复用 `set_visible(true)`。

## 8. `Timer` 已废弃，用 `AnimationTimer`

```rust
nwg::AnimationTimer::builder()
    .parent(&window)
    .interval(std::time::Duration::from_secs(15))   // 不是 .interval(15000)
    .active(true)                                   // 不是 .stopped(false)
    .build(&mut timer)?;
```

## 9. `TrayNotification::set_tip` 不接受 `Option`

- builder 的 `.tip()` 收 `Option<&str>`
- setter `set_tip(&self, tip: &str)` 只收 `&str`

## 10. `Menu` 没有 `append()`

每个 `MenuItem` 都要显式 `.parent(&menu)`：

```rust
nwg::Menu::builder().popup(true).parent(&msg_window).build(&mut menu)?;
nwg::MenuItem::builder().text("显示主窗口").parent(&menu).build(&mut item)?;
```

托盘右键定位用 `nwg::GlobalCursor::position()`：

```rust
nwg::Event::OnContextMenu if handle == tray.handle => {
    let (x, y) = nwg::GlobalCursor::position();
    menu.popup(x, y);
}
```

## 11. 闭包借用冲突 `E0505`

`full_bind_event_handler(&w.handle, move |..| { ...使用 w... })` 会在闭包里
再次借 `w` 而报 `E0505`。先 `let wh = w.handle.clone();` 再用 `&wh`。

## 12. `default-features = false` 是必须的

nwg 的 `Cargo.toml` 里 `default = ["all"]`，会连带拉
`plotters` / `plotters-backend` / `stretch` / `newline-converter` /
`raw-window-handle`。编译时间从秒级变成分钟级。

```toml
native-windows-gui = { version = "1.0.13", default-features = false, features = [
    "tabs", "tray-notification", "message-window", "cursor", "menu",
    "progress-bar", "trackbar", "combobox", "animation-timer",
    "notice", "image-decoder", "file-dialog",
] }
```

实测这 12 个 feature 的组合编译 **2.37 秒**，所有对应类型都能实例化 + 运行。

## 13. 探针程序必须进消息循环

`main()` 里建完窗口直接 `return` → 进程 `exit=0` **立刻退出**，
看起来像「nwg 坏了」。必须：

```rust
nwg::dispatch_thread_events();
```

我第一版探针就踩了这个，误判为 nwg 有问题。

## 14. `Icon::builder().size(...)` 会让 `.ico` 加载失败

```rust
// ✅ 走 CreateIconFromResourceEx，原生支持 PNG 压缩条目
nwg::Icon::from_bin(TRAY_ICO)
nwg::Icon::builder().source_bin(Some(TRAY_ICO)).build(&mut icon)

// ❌ Err("Failed to create a image factory")
//    走 WIC image factory，而 WIC 的 ico 解码器不认 PNG 压缩条目
nwg::Icon::builder().source_bin(..).size(Some((16,16))).strict(true)
```

详见 [tray-icon.md](tray-icon.md)。

## 15. `Icon::handle` 不能 `{}` 打印

类型是 `*mut winapi::ctypes::c_void` → `error[E0277]`。
要 `as usize` 或 `{:?}`。

---

## 16. `ICONINFO` 必须写满五个字段，否则 `CreateIconIndirect` 恒返回 NULL

想用代码动态生成托盘图标（比如在图标上画当前电量数字），要用
`CreateIconIndirect(&ICONINFO)`。结构看着只有三个「有用」字段，但**少写一个就失败**：

```c
typedef struct {
    BOOL    fIcon;      // 必须 TRUE
    DWORD   xHotspot;   // 光标专用，图标填 0
    DWORD   yHotspot;   // 同上
    HBITMAP hbmMask;    // 图标：既作 AND 掩码
    HBITMAP hbmColor;   // 图标：又作彩色位图
} ICONINFO;
```

漏掉 `xHotspot` / `yHotspot` 时：

```
CreateIconIndirect → NULL
GetLastError()     → 203  (ERROR_ENVVAR_NOT_FOUND)
```

`203` 是 `"找不到环境变量"`，跟图标毫无关系。**这个错误码会把人引到完全错误的方向**
（我一开始去查 PATH 和 `HICON` 生命周期，全部白费）。补齐两个 `DWORD` 立刻返回有效句柄。

Rust 侧对应（`src\tray_icon.rs`）：

```rust
#[repr(C)]
struct IconInfo {
    f_icon: i32,        // 1
    x_hotspot: u32,     // 0
    y_hotspot: u32,     // 0
    hbm_mask: isize,    // CreateBitmap(w, h, 1, 1, null) 单色掩码
    hbm_color: isize,   // 32bpp DIB 段
}
```

记得两件事：
- 图标类型 `fIcon = 1`（`0` 是光标）
- 用完要把两个位图 `DeleteObject`，但**不能**在 `CreateIconIndirect` 之前删

## 17. `DrawTextW` 在 32bpp DIB 上会把 alpha 写成 0

在 32bpp（`BI_RGB` + 高分字节未用）的 DIB 段上画文字，GDI 只填了 RGB，
**alpha 字节被留成 0**。结果是：屏幕上看到的图标「没有字」，其实字在那儿但全透明。

实测现象很有迷惑性：托盘图标位置显示一块深灰纯色方块，没有数字；
放大 8 倍逐像素看，`82` 那两个字的像素 RGB 是对的（`#FFFFFF`），alpha 是 `0x00`。

修法：`DrawTextW` 之后**遍历整个位图，把 alpha 字节统一拉成 `0xFF`**：

```rust
let px = bits as *mut u8;
for i in (0..(w * h) as usize).map(|i| i * 4) {
    *px.add(i + 3) = 0xFF;
}
```

另外两条：
- 字号必须配 `ANTIALIASED_QUALITY`（`4`），**不要用 `CLEARTYPE_QUALITY`（`5`）** ——
  次像素抗锯齿在透明背景上会留下彩色边缘（红/青描边），看起来像图坏了
- `DrawTextW` 之前必须 `SelectObject(dc, font)`，用完 `SelectObject(dc, old)` 还原

## 18. 想 `CreateIconIndirect`，必须先把 DIB 从 DC 里 `SelectObject` 出来

用 `CreateDIBSection` 拿到 `HBITMAP` 后，常规做法是 `SelectObject(mem_dc, hbm)` 再画。
画完之后**必须再 `SelectObject(mem_dc, old_bmp)` 把它换出去**，否则
`CreateIconIndirect` 拿到的是一个「仍被 DC 选中」的位图 —— 返回 NULL 或生成残缺图标。

顺序固定为：

```
CreateCompatibleDC
  → CreateDIBSection      (拿 hbm + bits 指针)
  → SelectObject(mem_dc, hbm)     ← 画之前换进来
  → 各种 GDI 绘制
  → SelectObject(mem_dc, old)     ← 画完换出去，这一步不能省
  → CreateIconIndirect
  → DeleteDC / DeleteObject(hbm) / DeleteObject(mask)
```

GDI 对象是全局共享的，这条规则对所有涉及位图的 API 都成立，不只是图标。
## 附：为什么当初选了 nwg，以及 egui / iced 后来查出什么（2026-10-22 更正）

### 当初的判断（部分已被推翻，保留原文以便看清错在哪）

- `eframe`/`egui` 会经 `parking_lot_core` 触发 `dlltool` 生成 `.def`
- `dlltool.exe` 在 rustlib 的 `self-contained` 目录里**有**
  （`D:\dsh\tools\rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained\dlltool.exe`，1,328,640 字节）
- 但它需要 GNU 汇编器 `as.exe` 来汇编 `.def`，**这台机器上完全没有**
  （`as` / `gcc` / `clang` 全无；Git 的 `usr\bin` 和 `mingw64\bin` 里也没有）
- 实测两次失败：
  - `error: error calling dlltool 'dlltool.exe': program not found`
  - 把 dlltool 加进 PATH 后 → `dlltool.exe: CreateProcess` 失败
- `self-contained` 是个**残缺 MinGW**：有 43 个 Windows import library
  （`libkernel32.a` / `libuser32.a` / `libgdi32.a` / `libcomctl32.a` /
  `libshell32.a` / `libsetupapi.a` / `libole32.a` / `libuuid.a` /
  `libopengl32.a` 等）+ 两个可执行文件 `dlltool.exe`、`ld.exe`，
  **有 import library，没有汇编器**

→ 当时的结论是「任何走 `raw-dylib` 的 crate 在这套 toolchain 上必死」。
纯手写 Win32 FFI（`#[link(name = "user32")] extern "system" {...}`）和
nwg 都可行，前者 0.2 秒编译、后者 2.4 秒。

### 更正：上面的「必死」是错的

后来实测发现 **`dlltool` 这条路可以走通**，只是不能靠 rustlib 自带的那个：

```
# 装 LLVM-MinGW（winget: MartinStorsjo.LLVM-MinGW.MSVCRT）
# 用它的 dlltool；真正的实现在 llvm-dlltool，不需要 as.exe
dlltool -d kernel32.dll_imports.def -D kernel32.dll \
        -l kernel32.dll_imports.lib -m i386:x86-64 ...
# → 生成 2098 字节的 .lib，成功
```

证据：`D:\codex\_gui-verify\evidence\dlltool_invocations.log`。
它记录了一次 shim 调用：CWD 在 `deps\rustcZvai0h`，ARGV 是 rustc 传给
`dlltool` 的完整参数，OUT 是 `kernel32.dll_imports.lib`，
`exists-after = true` / `size = 2098` / `real status = ExitStatus(0)`。

配合这个 shim 之后：

| 框架 | 编译 | 运行 |
|---|---|---|
| **`iced 0.14`**（`tiny-skia` + `thread-pool`） | ✅ 冷编 39.58 s / 热编 20.91 s | ✅ **窗口正常**，776x579，中文渲染完美 |
| `eframe`/`egui 0.29` | ✅ 26–28 s | ❌ **`0xC0000005` 崩溃** |

`iced_probe.exe` 实测：窗口类 `Window Class`、标题 `ROG 鼠标控制 - iced probe`、
左侧导航「设备 / 按键映射 / 灯光 / 性能 / 关于」+ 右侧进度条 87%，全部正常。
截图见 `scratch/iced-probe-window.png`。

`eframe` 一侧三个变体（`eframe_bare` / `eframe_patched` / `eframe_trace`）
**全部** `0xC0000005`；`eframe_exit42` / `eframe_ctor` 是主动 `exit(42)` 的
定位探针，能跑到退出点 —— 说明崩溃发生在更靠后的地方（进 `run()` 之后）。

### 所以现在的真实结论

1. **「gnu toolchain 编译不了 GUI 框架」这个前提不成立** —— 加 shim 之后 iced 和 egui 都能编。
2. **但 `egui`/`eframe 0.29` 在本机跑不起来**（`0xC0000005`），不论是否 patch。
   这是它被排除的**真实原因**，不再是 `dlltool` 那个判断。
3. **`iced 0.14` 能编也能跑**，是目前唯一验证通过的现代 UI 框架。
4. **nwg 仍是当前实现的选择**，因为：
   - 托盘：nwg 有原生 `TrayNotification`（实测 `hr=0`）；iced 没自带托盘，
     要自己 `Shell_NotifyIconW` 挂，与 winit 事件循环能否共存**尚未验证**
   - 现有 `src/gui.rs`（110 KB，五页 + 后台 HID 线程 + `Notice`）已验收通过
   - 换框架 = 全部重写 + 31 项设备功能重新验一遍
5. **将来若要换**，候选是 `iced 0.14`（不是 egui），且必须先过「托盘共存」这道门槛。

### 写在最后

这条附录原本写得很确定（「必死」），实际上是**结论下得太早**：
只试了 rustlib 自带的那一个 `dlltool`，失败后就推断「这条路不存在」，
没去查「换一个 dlltool 实现是否可行」。

教训：**「我用这个方法失败了」不等于「这件事做不到」**
（同类的还有另一条：我一度从 nwg 源码推断「`Label` 灰底是 nwg 的 bug」，
实际是自己把 `WM_CTLCOLORSTATIC` 的消息方向记反了，见第 1 节）。
