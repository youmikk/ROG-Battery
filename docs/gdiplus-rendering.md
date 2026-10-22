# GDI+ 自绘可行性实测（2026-10-22）

**结论先说**：nwg（native-windows-gui）配合系统自带的 `gdiplus.dll`，能画出圆角、径向渐变、抗锯齿中文、深色主题的现代界面。因此「想变好看」**不需要换 GUI 框架**。

这不是理论推断：下面的图是用 `D:\codex\rog-mouse-ctl\scratch\gdiplus-probe\gdiplus6.rs`（230 行，零第三方 crate，纯手写 FFI）真跑出来、从内存位图直接读像素导出的。

## 实物证据

- 渲染结果：`scratch\gdiplus-probe\gdiplus6.png`（544x281）与 2 倍放大 `gdiplus6-2x.png`
- 画面内容：深色底 `#202226`；上方圆角 16px 的径向渐变卡片（边缘 `R31 G59 B97` → 中心 `R62 G121 B184` → 边缘 `R31 G60 B97`）；下方圆角 12px 的纯色卡片 `#2B2F36`；两行抗锯齿中文，字体 Microsoft YaHei UI 20px
- 像素采样 7/7 全部非背景色，渐变在 x 方向单调变化后又回落 —— 证明不是「整块平涂」

## 对本项目的成本

| 项目 | 数值 |
|---|---|
| 编译时间 | **1 秒**（`rustc -O`） |
| 新增代码 | 约 230 行 FFI 声明 + 绘制代码 |
| 运行时新增依赖 | 只有 `gdiplus.dll`（`C:\Windows\System32\gdiplus.dll`，1,933,312 B，**Windows XP 起系统自带**） |
| 分发形态影响 | **零**。绿色版仍是单文件 exe，不需要附带任何 DLL |
| 产物体积 | strip + panic=abort 后 **880,128 B**（探针含完整窗口代码，非增量） |
| 链接期依赖 | `libgdiplus.a`（156,950 B），从 LLVM-MinGW 单独拷出，**不进安装包** |

## 如何链接（本机这套残缺 MinGW 的关键）

`rustlib\...\self-contained` 里有 43 个 import library，**没有 `libgdiplus.a`**。所以：

1. 从 LLVM-MinGW 单独拷出 `libgdiplus.a` 到一个**只含它**的目录：
   ```
   %LOCALAPPDATA%\AppData\Local\Microsoft\WinGet\Packages\
     MartinStorsjo.LLVM-MinGW.MSVCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\
     llvm-mingw-20261006-msvcrt-x86_64\x86_64-w64-mingw32\lib\libgdiplus.a
   ```
2. 编译时 `-L <那个目录>`，并 `#[link(name = "gdiplus")]`
3. **绝不能把整个 llvm-mingw lib 目录加进 `-L`**：它的 `libmsvcrt.a`/`libmingwex.a` 会盖过 rustlib 同名库，CRT 起步就崩（这条是子代理 `ff2f6e8d` 踩出来的）

同一来源还缺、且后续可能要用到的：`libuxtheme.a`(19,100 B) `libdwmapi.a`(11,010 B) `libshlwapi.a`(94,730 B) `libversion.a`(5,320 B)。rustlib 自带而无需外找的：`libmsimg32.a`(5,298 B) `libcomdlg32.a`(21,154 B) `libwinmm.a`(159,706 B)。

## 六个坑（全部是「编译期零警告、运行时不报错、结果错」）

这一节的共同规律：**GDI+ 的返回值是 `Status` 枚举，0 = Ok；把非 0 当成功、或干脆不看返回值，就会得到一张纯黑图而没有任何错误提示。**

### 1. `GdiplusStartup` 的 `suppress_background_thread` 设成 1 会让 startup 直接失败

```rust
let input = GdiplusStartupInput {
    gdiplus_version: 1,
    debug_event_callback: std::ptr::null_mut(),
    suppress_background_thread: 0,   // 必须是 0
    suppress_external_codecs: 0,
};
```
设 `suppress_background_thread: 1` 且 `output` 传 `null` 时，`GdiplusStartup` 返回 **2（InvalidParameter）**，接着 `GdipCreateFromHDC` 返回 **18（GdiplusNotInitialized）**。两个错误码都不是 0，但如果不打印返回值，就只会看到「什么都没画」。

有两种正确写法：设 0；或者设 1 时传一个非空的 `GdiplusStartupOutput`。

### 2. `GdipDrawString` 是 **7 个参数**，不是 6 个

```c
Status GdipDrawString(GpGraphics*, const WCHAR*, INT length,
                      const GpFont*, const RectF*, const GpStringFormat*, const GpBrush*);
```
漏掉 `stringFormat` 时，编译器不会报错（因为是自己声明的 FFI），但**运行时会在 `gdiplus.dll` 内部崩溃**：`Exception code: 0xc0000005`、`Faulting module name: gdiplus.dll`、`Fault offset: 0x3bd82`。本次实测就是这样崩的，事件日志里能看到。

`stringFormat` 可以传 `null`（用默认格式），但**位置必须占着**。

### 3. Rust 的 `#[repr(C)]` 会把 14 字节的 `BITMAPFILEHEADER` 填充成 16 字节

```rust
#[repr(C)]
struct BitmapFileHeader { bf_type: u16, bf_size: u32, bf_reserved1: u16,
                          bf_reserved2: u16, bf_off_bits: u32 }
// size_of = 16（u32 要 4 字节对齐），但 BMP 磁盘格式要求严格 14 字节
```
用这个结构体 `extend_from_slice` 写文件，整个 BMP 头会错位 2 字节。后果：`bfSize` 读成 1418776320、`bfOffBits` 读成 3670016、`biBitCount` 读成 1，`System.Drawing.Bitmap::FromFile` 报「**内存不足**」（这个错误信息完全误导，实际是文件头坏了）。

**修法：手工拼字节**，不要用结构体。`BITMAPINFOHEADER` 虽然字段本身都 4 字节对齐、`repr(C)` 无填充（`size_of` = 40 正确），但为了统一，也一并手工拼。

### 4. `GetDIBits` 会把画好的内容读成全黑

`GetDIBits(mem_dc, bmp, 0, h, buf, &mut info, 0)` 全部返回成功（`got == h`），但 `buf` 里全是 0。加了 `GdiFlush()` 也没用。

**修法：根本不用 `GetDIBits`。** 改用 `CreateDIBSection`，它直接给出 `bits` 指针：

```rust
fn CreateDIBSection(hdc: Hdc, info: *const BitmapInfo, usage: u32,
                    bits: *mut *mut c_void, section: Handle, offset: u32) -> Hbmp;
```
拿到 `bits` 后自己按 BGRA 顺序读写像素，既可靠又快。本次最终方案就是这条路。

### 5. `BitmapInfo` 的 `colors` 数组长度不能省

`BITMAPINFO` 是「`BITMAPINFOHEADER` + 一个变长 `RGBQUAD` 数组」。32bpp 时用不到调色板，但**结构体里必须留出位置**，否则 `CreateDIBSection` 按 `bmiColors` 计算布局时会读到结构体之外。本次实测至少给 3 项（`colors: [u32; 3]`，`size_of::<BitmapInfo>()` = 52）才稳定。

### 6. 文字画到 32bpp DIB 上，alpha 字节要自己拉成 0xFF

和托盘图标那个坑（见 `docs\nwg-gotchas.md` 第 17 节）是同一类：`DrawTextW` 会往 alpha 字节写 0。GDI+ 的 `GdipDrawString` 在 32bpp DIB 上表现不同（实测不破坏 alpha），但如果混用 GDI 的 `TextOutW`/`DrawTextW`，就必须事后遍历 `*px.add(i+3) = 0xFF`。

## 对「换不换 iced」的影响

| 维度 | nwg + GDI+ | iced 0.14 |
|---|---|---|
| 观感上限 | **实测能做出圆角渐变深色界面**（见上图） | 原生现代风格，开箱即用 |
| 托盘 | nwg 有原生 `TrayNotification`，`Shell_NotifyIconGetRect` 实测 `hr=0` | **不带托盘**，需自己写 `Shell_NotifyIconW` + 隐藏窗口 + 消息泵，子代理正在验证共存 |
| 现有代码 | 五页 + 31 项设备功能已验证，改的是「画法」 | `gui.rs` 2,720 行 / 18 种 nwg 控件（Label 92 处、Button 24 处、ComboBox 18 处、CheckBox 16 处）全要重写 |
| 可复用 | — | `ops.rs` `proto.rs` `hid.rs` `snapshot.rs` `history.rs` `chart.rs` `autostart.rs` `cli.rs` `tray_icon.rs` `win32.rs` `settings.rs` `backup.rs` `paths.rs` **全部零改动复用**（均不含 `nwg::`） |
| 编译 | 1 秒级，零新依赖 | 225 个 crate，冷编 39.6 s |
| 产物 | 816 KB（当前） | 2,566 KB（tiny-skia 版） |
| 已知风险 | 无 | eframe 在本机启动 2.5~3 秒后必崩（链接布局敏感）；iced 未崩但托盘待验 |

**所以真正的取舍不是「好看 vs 难看」，而是「重写 2,720 行 GUI 值不值得换来开箱的现代风格」。**
