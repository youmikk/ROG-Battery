# 托盘图标：为什么必须 embed，以及 `.size()` 为什么不能用

`TrayNotification` 在 `icon` 为 `None` 时**直接 panic**：

```rust
// native-windows-gui-1.0.13/src/controls/tray_notification.rs:370
panic!("Tray notification requires an Icon at creation")
```

而 `OemIcon` 只有 6 个系统变体，没有「应用图标」：

```rust
// src/resources/system_images.rs:82
Sample = 32512, Ques = 32514, WinLogo = 32517,
Warning = 32515, Error = 32513, Information = 32516
```

所以必须给一个真实的 `.ico`。

---

## 文件

`assets/tray.ico` —— 6,059 字节，含 16 / 24 / 32 / 48 / 256 五个尺寸。

由 `tools/make-tray-icon.ps1` 从 MSIX 的 PNG 资产生成，可复现：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\make-tray-icon.ps1
```

脚本读取 `msix/Assets/Square44x44Logo.targetsize-*.png`，把 PNG 字节原样嵌进 ICO 目录（Vista+ 的 ICO 支持 PNG 压缩条目），并逐个校验像素尺寸而不是相信文件名。

> 脚本必须纯 ASCII。PowerShell 5.1 按系统 ANSI（GBK）解码 `.ps1`，注释里的中文可能变成带引号/反引号的乱码字节，导致解析失败。

---

## 用法：`include_bytes!` + `source_bin`

```rust
static TRAY_ICON: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/tray.ico"));

let mut icon = nwg::Icon::default();
nwg::Icon::builder()
    .source_bin(Some(TRAY_ICON))
    .build(&mut icon)?;
// 不要加 .size()

nwg::TrayNotification::builder()
    .parent(&message_window)          // 必须是 MessageWindow，不能是 Window
    .icon(Some(&icon))                // 必须给，None 会 panic
    .tip(Some("R Battery"))
    .build(&mut tray)?;
```

用 `concat!(env!("CARGO_MANIFEST_DIR"), ...)` 而不是相对路径，避免 `include_bytes!` 的路径按**源文件所在目录**解析（`src/main.rs` 里写 `../assets/`，`scratch/x/src/main.rs` 里要写 `../../../assets/`，很容易错）。

---

## 三个实测结论

### ① `from_bin` / `source_bin` 可用

用 `scratch/icotest/` 探针实测（nwg 1.0.13，`image-decoder` feature）：

```
ico 字节数 = 6059
from_bin OK, handle = 65585
Icon::builder().source_bin(Some(TRAY_ICO)).build(&mut icon) -> Ok(handle=65585)
```

### ② `.size()` 一定不要加

```rust
Icon::builder().source_bin(Some(TRAY_ICO)).size(Some((16, 16))).strict(true).build(&mut icon)
  -> Err(ResourceCreationError("Failed to create a image factory"))
```

五个尺寸（16/24/32/48/256）**全部**报同样的错。

原因：加了 `.size()` 后 nwg 走 WIC（Windows Imaging Component）的 image factory 去缩放，而 ICO 里 256 那条是 **PNG 压缩条目**，WIC 的 ico 解码器不处理它。不加 `.size()` 时 nwg 直接用 `CreateIconFromResourceEx`，这个 API 原生支持 PNG 条目。

托盘实际只需要 16/20/24/32，Windows 会自己从多尺寸 ICO 里挑最合适的那条，不需要手动指定。

### ③ `Icon::handle` 是 `*mut c_void`，不能 `{}` 打印

```rust
// 编译错误 error[E0277]: `*mut winapi::ctypes::c_void` doesn't implement `std::fmt::Display`
format!("handle = {}", icon.handle)

// 正确
format!("handle = {}", icon.handle as usize)
format!("handle = {:?}", icon.handle)
```

调试日志里打图标句柄会撞这个。

---

## 验证 ico 本身（不依赖 Rust）

生成脚本自带校验：用 `System.Drawing.Icon` 逐尺寸加载，`16/24/32/48` 全部正确返回对应尺寸。`256` 那条 `System.Drawing.Icon` 的构造函数返回 48×48（GDI+ 不解析 PNG 条目，回落到当时缓存的最小有效尺寸），这是 **GDI+ 的限制，不是 ico 的问题** —— `CreateIconFromResourceEx`（Windows Shell 走的路径）能正常读。

生成 `rog-mouse-ctl.exe` 后用资源管理器双击 `assets/tray.ico` 能正常预览，就说明 Shell 认可这个文件。
