# 双形态启动方案（GUI 无黑框 + CLI 可重定向）

**结论：保持 `subsystem=console` 不变，GUI 路径开头调 `FreeConsole()`。**

这篇记录为什么不是另一种做法，以及实测数据。改动只涉及 `main.rs` 十几行和 `win32.rs` 一个 FFI 声明。

---

## 问题

`rog-mouse-ctl.exe` 是同一个二进制要干两件事：

1. 命令行工具：`rog-mouse-ctl info` 要往控制台打印，要能被 `> report.txt`、`| Select-String` 捕获
2. 图形界面：双击 exe 要弹窗口，**不能闪一个黑框**

console subsystem 的程序双击运行时，Windows 会自动配一个控制台窗口 —— 这就是黑框的来源。

---

## 两条路的实测对比

### 方案 A：`#![windows_subsystem = "windows"]` + `AttachConsole(ATTACH_PARENT_PROCESS)`

思路是反过来：编成 Windows 子系统（双击无黑框），需要输出时主动挂回父进程的控制台。

探针 `scratch/consoletest.rs` 实测（`rustc -O` 编译，5,089,401 B）：

```
AttachConsole = 1                                    ← 挂上了
GetConsoleWindow = 1509564                           ← 拿到窗口
CreateFileW(CONOUT$) = 260                           ← 句柄有效
SetStdHandle(out) = 1                                ← 重定向成功
SetStdHandle(err) = 1
WriteFile = 1, written = 43                          ← 直写成功
```

**看起来全成功，但：**

```
$ cmd /c "consoletest.exe > redirect.txt 2>&1"
redirect.txt 长度: 0        ← 空文件！
```

`PowerShell ... | Out-File out.txt` 同样是 **0 字节**。

**原因**：重定向的时候根本没有控制台可以 attach。`CONOUT$` 是「当前控制台」的符号名，进程没有控制台时 `CreateFileW("CONOUT$")` 要么失败、要么拿到一个和用户管道无关的句柄。`WriteFile` 返回成功（写进了一个没有接收者的地方），用户的文件收不到任何字节。

**这条路会废掉所有脚本化用法**：`info > report.txt`、`| Select-String`、CI 里抓输出，全部变成空。

### 方案 B：保持 console subsystem + GUI 路径调 `FreeConsole()`

探针 `scratch/freeconsole.rs` 实测：

| 场景 | 结果 |
|---|---|
| CLI（带参数）`cmd /c "freeconsole.exe --version > fc-cli.txt"` | ✅ 39 字节正常捕获 |
| GUI（无参数）`GetConsoleWindow` 前 | 984974（有黑框） |
| GUI（无参数）调 `FreeConsole()` 后 | **0（窗口消失）** |

```rust
unsafe {
    FreeConsole();                    // 1 = 成功脱离
}
```

两条都满足，没有取舍。

---

## 实现

### `main.rs` 入口

```rust
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // 特例：帮助必须留在 CLI，否则命令行敲 --help 会弹窗
    let wants_help = matches!(args.first().map(|s| s.as_str()), Some("-h" | "--help" | "help"));

    // 双击 exe 时没有任何参数 -> GUI
    if args.is_empty() {
        // SAFETY: FreeConsole 只影响本进程的控制台关联，无指针参数、无失败副作用。
        // 必须在创建任何窗口之前调用，否则控制台窗口会先画出来再消失（可见闪烁）。
        unsafe { win32::FreeConsole(); }
        return run_gui();
    }

    if wants_help {
        print_usage();
        return ExitCode::SUCCESS;
    }

    run_cli(&args)
}
```

### `win32.rs` 声明

```rust
#[link(name = "kernel32")]
extern "system" {
    /// 把本进程从控制台脱离。GUI 模式下调用，让 console subsystem 的
    /// 黑框窗口消失。
    ///
    /// 为什么不用 `#![windows_subsystem = "windows"]` + `AttachConsole`
    /// 见 `docs/startup-modes.md`：那条路会让 `> file` 重定向产出 0 字节。
    pub fn FreeConsole() -> i32;
}
```

`libkernel32.a` 在 rustlib 自带的 `self-contained` 目录里，不需要 dlltool，`x86_64-pc-windows-gnu` 直接能链。

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

---

## 验证清单（改动后必须全过）

```powershell
# ① 裸跑 -> GUI，进程存活，有窗口，无控制台
Start-Process .\rog-mouse-ctl.exe
# 进程存活、MainWindowHandle != 0、GetConsoleWindow == 0

# ② --help 仍走 CLI
.\rog-mouse-ctl.exe --help        # exit 0，输出 42 行

# ③ 重定向必须非空
.\rog-mouse-ctl.exe info | Out-File r.txt    # r.txt > 0 字节

# ④ GUI 分支不许 println!
# 所有日志写 %LOCALAPPDATA%\rog-mouse-ctl\gui.log（UTF-8 无 BOM）
```

实测结果（2026-10-21 23:21）：① 进程存活 `MainWindowHandle=1049404`；② exit=0；③ **1407 字节**（不是 0）。

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
