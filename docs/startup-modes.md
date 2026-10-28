# 双形态启动方案（GUI 无黑框 + CLI 可重定向）

**结论：子系统用 `windows`（PE `subsystem=2`），命令行由 `win32::attach_console()` 在 `main` 开头把父控制台接回来 —— 但只在没有继承句柄时才接。**

本文在 2026-10-29 被推翻重写过一次。早前的结论是「保持 `subsystem=console` + GUI 路径 `FreeConsole()`」，那是错的：
`FreeConsole` 只是事后把控制台赶走，窗口已经画出来了，从 Microsoft Store 启动时用户照样看得见黑框。
真正难的那一半是重定向，而它可以用「先看有没有继承句柄」这一个分支解决。

---

## 问题

`rog-mouse-ctl.exe` 是同一个二进制要干两件事：

1. 命令行工具：`rog-mouse-ctl info` 要往控制台打印，要能被 `> report.txt`、`| Select-String` 捕获
2. 图形界面：双击 exe 要弹窗口，**不能闪一个黑框**

---

## 三条路的实测对比

### 方案 A（旧版，失败）：`windows_subsystem = "windows"` + 无条件 `CONOUT$`

探针 `scratch/consoletest.rs` 实测（`rustc -O` 编译）：

```
AttachConsole = 1                                    ← 挂上了
GetConsoleWindow = 1509564                           ← 拿到窗口
CreateFileW(CONOUT$) = 260                           ← 句柄有效
SetStdHandle(out) = 1                                ← "重定向成功"
WriteFile = 1, written = 43                          ← 直写成功
```

看起来全成功，但：

```
$ cmd /c "consoletest.exe > redirect.txt 2>&1"
redirect.txt 长度: 0        ← 空文件！
```

**当年记下的原因是错的。** 当时写的是「重定向时没有控制台可以 attach，`CONOUT$` 拿不到用户的管道」，
真正的原因更简单：**父进程已经把目标句柄（文件或管道）继承给子进程了，探针却用 `CONOUT$` 把
`STD_OUTPUT_HANDLE` 覆盖掉了** —— 于是输出进了控制台，用户的文件当然收不到字节。
换句话说，方案 A 不是走不通，是少了「有继承句柄就别动它」这一步。

### 方案 B（过渡，有黑框）：console subsystem + `FreeConsole()`

| 场景 | 结果 |
|---|---|
| CLI（带参数）`cmd /c "freeconsole.exe --version > fc-cli.txt"` | ✅ 39 字节正常捕获 |
| GUI（无参数）`GetConsoleWindow` 前 | 984974（有黑框） |
| GUI（无参数）调 `FreeConsole()` 后 | 0（窗口消失） |

CLI 完全正常，但黑框**先被创建再被赶走**。本机进程快，多数时候看不见；Store 版启动路径更长，
用户就能看见一下。这是本次改回方案 C 的直接原因。

### 方案 C（现在采用）：`windows_subsystem = "windows"` + 条件接控制台

```rust
// src/main.rs 顶部
#![cfg_attr(not(test), windows_subsystem = "windows")]
```

`cfg(not(test))` 不能省：`cargo test` 的测试进程如果也是 GUI 子系统，测试输出会被直接丢掉。

```rust
// src/main.rs
fn main() -> ExitCode {
    win32::attach_console();          // 第一件事，早于任何 println!
    ...
}
```

`attach_console` 的顺序是关键：

1. `GetStdHandle(STD_OUTPUT_HANDLE)` 有效 → **直接返回，什么也不做**。
   `cmd /c "x info > r.txt"`、`x info | Out-File f.txt`、`Start-Process -RedirectStandardOutput`
   都走这条路，句柄是父进程继承来的文件/管道。
2. 没有继承句柄 → `AttachConsole(ATTACH_PARENT_PROCESS)`。成功则把 `CONOUT$` / `CONIN$`
   设成标准句柄，`println!` 落到父控制台窗口里。
3. 两步都失败（双击 / 开始菜单 / 商店 / 计划任务）→ 返回 `false`，进程没有控制台，也就没有黑框。

---

## 实测（2026-10-29，本机）

PE 头：`machine = 0x8664`，`subsystem = 2 (WINDOWS_GUI)`，exe 1,032,704 B。

| 启动 / 输出方式 | 结果 |
|---|---|
| 双击、开始菜单、MSIX 商店入口 | 无控制台窗口，无黑框 |
| `cmd /c "rog-mouse-ctl.exe --help > f.txt"` | ✅ 3,033 字节 |
| `rog-mouse-ctl.exe --help \| Out-File f.txt` | ✅ 3,085 字节 |
| `Start-Process -RedirectStandardOutput` | ✅ 2,273 字符 |
| `rog-mouse-ctl.exe --help > f.txt`（**PowerShell 的 `>`**） | ❌ 0 字节 |
| `cargo test` | 152 passed（输出可见，证明 `cfg(not(test))` 生效） |

**PowerShell `>` 那条是已知限制，不是本程序的 bug**：实测进程拿到的 `out=2500` 是有效句柄，
但写进去的字节进不了文件（PowerShell 5.1 给 GUI 子系统程序的这个句柄没有接收者）。
脚本里请用上面三种能用的写法。

---

## 判据为什么是「无参数」

| 启动方式 | `args` | 期望 |
|---|---|---|
| 资源管理器双击 | `[]` | GUI |
| 开始菜单 / MSIX 商店入口 | `[]` | GUI |
| `rog-mouse-ctl info` | `["info"]` | CLI |
| `rog-mouse-ctl --help` | `["--help"]` | CLI |
| 任务栏固定 + 启动 | `[]` | GUI |

**不要加 `--gui` 开关当主判据**：用户双击时不会传参，如果裸跑进 CLI，会打印一段帮助然后退出 —— 用户看到的是「闪一下就没了」，比黑框更糟。

**`--help` 是唯一例外**：用户在终端敲 `rog-mouse-ctl --help` 时期望在终端看到帮助，不是弹窗。

`--autostart`（开机自启专用参数，见 `src/autostart.rs` 的 `RUN_ARG`）必须和裸跑走同一条 GUI 路径：
开机时没有控制台，落到 CLI 分支只会打印用法然后退出，用户看到的是「勾了自启但什么也没发生」。

---

## 验证清单（改动后必须全过）

```powershell
# ① 子系统必须是 GUI
# 读 PE 头 Optional Header 的 Subsystem 字段：0x18 + 0x44（PE32+）
# 期望 2 (WINDOWS_GUI)

# ② 裸跑 -> GUI，进程存活，有窗口，无控制台
# MainWindowHandle != 0 且 GetConsoleWindow == 0

# ③ --help 仍走 CLI
.\rog-mouse-ctl.exe --help        # exit 0，输出 42 行

# ④ 三种重定向都必须非空
cmd /c ".\rog-mouse-ctl.exe --help > a.txt 2>&1"    # 3,033 字节
.\rog-mouse-ctl.exe --help | Out-File b.txt          # 3,085 字节
# Start-Process -RedirectStandardOutput               # 2,273 字符

# ⑤ cargo test 的输出必须看得见
# 看不见就是漏了 cfg(not(test))

# ⑥ GUI 分支不许 println!
# 所有日志写 %LOCALAPPDATA%\rog-mouse-ctl\gui.log（UTF-8 无 BOM）
```

---

## 附带结论：`gui.log` 是合法 UTF-8

排查时我一度以为日志乱码，实际是读取端的问题：

```
原始字节: 47 55 49 20 | E5 B0 9A E6 9C AA E5 AE 9E E7 8E B0
                       尚        未        实        现
```

无 BOM 的标准 UTF-8。用 `Get-Content`（PS 5.1 默认按系统 ANSI/GBK 解码）读才会显示成 `GUI 灏氭湭瀹炵幇`。

**规则：凡是含中文的文件，一律用 `[System.IO.File]::ReadAllText($p, [System.Text.Encoding]::UTF8)`，不要用 `Get-Content`。**

Rust 侧写日志用 `std::fs::File` + `write_all(s.as_bytes())` 即可，不需要加 BOM。
