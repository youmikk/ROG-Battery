# rog-mouse-ctl

ROG **龙鳞 Ace Mini**（英文型号 **ROG HARPE ACE MINI**）的 HID 参数控制工具，
**完全不依赖 ASUS 驱动 / Armoury Crate / GearLink**：自己用 setupapi 枚举 HID 接口、
自己拼 64 字节 feature report、自己解析应答。

- 零外部依赖（不引 `windows-sys` / `winapi`），Win32 调用全部手写 `extern "system"`
- 一个 exe，约 **904 KB**
- 读模型：**一次读全 19 帧 + 三层缓存**（内存 → 磁盘 → 真读设备），见第二节
- 协议来源：`D:\codex\rog-battery\_research\hid-write.md`（写）、`hid-features.md`（读），
  以及已真机验证的 C# 参考实现 `rw-hid.cs`

---

## 一、编译

Rust **不在系统 PATH 里**，必须先设三个环境变量（工具链是 MinGW，不是 MSVC）：

```powershell
$env:RUSTUP_HOME = 'D:\dsh\tools\rustup'
$env:CARGO_HOME  = 'D:\dsh\tools\cargo'
$env:PATH = "D:\dsh\tools\cargo\bin;$env:PATH"

cd D:\codex\rog-mouse-ctl
cargo build --release
```

产物：`D:\codex\rog-mouse-ctl\target\release\rog-mouse-ctl.exe`（925,184 字节 / 904 KB）

实测：干净编译 **约 8 秒**，**零 warning**。`cargo build` **完全不需要访问 crates.io**
（`Cargo.lock` 只有 8 个包，nwg 及其依赖已在本地 registry 缓存）。

### 第一次编译前必须先跑一次 `tools\vendor-import-libs.ps1`

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\vendor-import-libs.ps1
```

它从本机 LLVM-MinGW 里拷出 **5 个 import library** 到 `vendor\mingw-import-libs\`
（`libgdiplus.a` 156,950 B / `libshlwapi.a` 94,730 / `libuxtheme.a` 19,100 /
`libdwmapi.a` 11,010 / `libversion.a` 5,320），`build.rs` 会把该目录加进链接搜索路径。

为什么需要：本机 rustlib 的 `self-contained` 目录里有 43 个 import library，
**恰好缺这 5 个**，而 GDI+ 自绘要用 `gdiplus.dll`，缺了就是链接期
`ld: cannot find -lgdiplus`。

> **绝不能**把整个 LLVM-MinGW `lib` 目录加进 `-L`：它的 `libmsvcrt.a` / `libmingwex.a`
> 会盖过 rustlib 的同名库，CRT 起步直接崩。只拷这 5 个到独立目录。

`gdiplus.dll` 自 Windows XP 起是系统组件，**不进安装包、不额外分发**。

> 如果以后要引入**走 `raw-dylib` 的 crate**（`windows` crate、`parking_lot_core` 等），
> 那还需要能跑 `dlltool`，而它需要 GNU 汇编器 `as.exe` —— 本机没有。
> 绕法是做一个转发 exe 转给 LLVM-MinGW 的 `llvm-dlltool.exe`（实测可行，见
> `docs\nwg-gotchas.md` 附录）。**但 iced / eframe 那条路的结论见附录，
> 不是「编不过」，是「不值得换」。**

### 关于链接方式（这段是踩坑记录，改代码前先看）

目标平台是 `stable-x86_64-pc-windows-gnu`，这台机器没有 VS 工具链、没有 Windows SDK。
在这套 rustlib 里：

| DLL | 做法 | 原因 |
|---|---|---|
| `kernel32` / `setupapi` | `#[link(name = "setupapi")]` 静态导入库 | rustlib 里有 `libsetupapi.a`、`libkernel32.a` |
| `hid.dll` | `LoadLibraryW` + `GetProcAddress` 运行时取函数指针 | rustlib 里**没有** `libhid.a` |
| `gdiplus.dll` | `#[link(name = "gdiplus")]` + `build.rs` 加 `vendor\mingw-import-libs` 到搜索路径 | rustlib 里**没有** `libgdiplus.a`，本机也没有 `as.exe` 所以不能自己造；从 LLVM-MinGW 拷现成的 |

不能用 `#[link(name = "hid", kind = "raw-dylib")]`：那会让 rustc 调 `dlltool.exe`，
而 dlltool 只存在于 `...\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained\dlltool.exe`，不在 PATH 里，
报错是 `error: error calling dlltool 'dlltool.exe': program not found`。

`HidD_*` 系列返回的是 **`BOOLEAN`（1 字节）**，不是 `BOOL`（4 字节），FFI 里必须用 `u8` 承接。

---

## 二、子命令

### 通用开关

| 开关 | 作用 |
|---|---|
| （默认） | **自还原**：改完读回验证，然后自动写回原值再验证。跑完设备状态和跑之前一样。 |
| `--keep` | 保留新值，不自动还原 |
| `--save` | 显式提交进 flash（`0x50 0x03`）。不加则只改 RAM，重连即丢 |
| `--yes` | 确认危险操作（目前只有 `setbutton … 0` 需要） |
| `--fresh` | 绕过缓存，强制重新读设备（`info` 用） |

### 0. 读模型：一次读全 + 缓存

**一次 `info` 读 19 帧**（17 帧鼠标侧 + 2 帧接收器侧），实测 **591 ms**。
读到的**原始帧字节**会缓存起来，第二次读只花 **203 ms**（省掉约 590ms）。

缓存是三层：

1. **内存**（同一次进程内）；
2. **磁盘** `%LOCALAPPDATA%\rog-mouse-ctl\snapshot.cache`（跨进程，默认 30 秒有效）；
3. **真读设备**。

有效期可在 `%APPDATA%\rog-mouse-ctl\settings.ini` 里改：

```ini
cache_ttl_sec=30
```

**为什么缓存原始帧而不是解析后的值**：这样「从设备读」和「从缓存读」走**完全同一套**
`proto::parse_*` 代码，不可能出现两条逻辑不一致。

**任何写操作之后缓存自动失效**（写前写后都会 `invalidate`），不会读到脏数据。

单帧失败不会让整个快照失败：失败的帧记进 `snapshot.failed`，其余照常解析，
`info` 会打印「读取异常项」。

```powershell
rog-mouse-ctl.exe cache          # 看缓存文件位置、帧数、年龄、有效期、配置/备份目录
rog-mouse-ctl.exe cache clear    # 删缓存
```

### 1. `info [--fresh] [--light]` / `quick` —— 只读，零风险

```powershell
rog-mouse-ctl.exe info
rog-mouse-ctl.exe info --fresh    # 绕过缓存，强制重读
rog-mouse-ctl.exe info --light    # 只读 3 帧，约 100ms
rog-mouse-ctl.exe quick           # 同上（info --light 的别名）
```

**全量模式**（默认）输出设备身份（型号名、有线/无线 PID、鼠标 SN、接收器 SN、接收器固件、
双端 MAC）、电量（电量 / 电压 / 充电状态 / 休眠 / 低电量阈值 / 电池类型 / 充满效果）、
性能（回报率 / 4 档 DPI 与颜色 / 当前档位 / 去抖 / 角度吸附 / 角度微调）、
LOD 与表面标定、灯光、8 个按键绑定、能量块。**19 帧 / 约 590ms**。

**轻量模式**（`--light` / `quick`）只读 3 帧 —— 电量（`12 07`）、性能（`12 04 00`）、
profile 与当前档位（`12 00 00`），**约 95ms**，比全量快 6 倍。输出是 `KEY=VALUE`，
方便脚本解析：

```
state=ok
level=84
voltage_mv=3985
charging=0
poll=1000Hz
active_stage=2
frames=3
cached=no
elapsed_ms=96
```

两点设计约束（改代码前先看）：

1. **电量帧排第一**。鼠标休眠时第一帧 MISS 就立即 `return`，不再读剩下两帧 ——
   实测单帧 MISS 约 **1.1 秒**（`raw 12 FF` 连跑 3 次：1098/1099/1151ms），
   白等两次就多付 2.2 秒。这是刻意写进
   `snapshot::LIGHT_FRAMES` 顺序里的功能性约束，有单测锁住。
2. **3 帧快照不写磁盘缓存**。它不是完整快照，写进 `snapshot.cache` 会被下一个进程
   当成 19 帧全量读出来用，导致 DPI 档值、灯光、按键全显示默认值。
   所以 `SnapshotCache` 用独立的 `light` 槽位，且只在内存里。

休眠时输出：

```
state=asleep
hint=鼠标休眠中，移动一下即可唤醒
```

**退出码仍然是 0** —— 休眠是设备的正常状态，不是错误。

### 2. `backup [文件]` / `restore <文件> [--save]`

```powershell
rog-mouse-ctl.exe backup                            # 默认落到 %USERPROFILE%\Documents\rog-mouse-ctl\backups\
rog-mouse-ctl.exe backup backups\stock.txt          # 只给文件名也落到上面的备份目录
rog-mouse-ctl.exe backup D:\somewhere\stock.txt     # 给完整路径则按给的路径
rog-mouse-ctl.exe restore stock-20261021-223130.txt
rog-mouse-ctl.exe restore stock-20261021-223130.txt --save    # 真正确认写入 flash
```

备份默认文件名是 `stock-<本地时间>.txt`，例如 `stock-20261021-223130.txt`。
**用的是本地时间**（`GetLocalTime`），不是 UTC —— 早期版本用 UTC 算，
22:26 的备份会写成 `142627`，容易被误判成旧备份。

备份是 `key=value` 纯文本，**恰好 31 个键**：

```
poll_code
dpi{1..4}_enc, dpi{1..4}_rgb
active_stage, debounce_code, angle_snap, angle_tuning, lod
light_mode, light_bright, light_rgb, light_dir, light_random, light_speed
button0 .. button7
poweroff_code, low_warn, full_effect
```

**能量块（`poweroff_code` / `low_warn` / `full_effect`）必须存在**，见第五节事故记录。

`restore` 的写入顺序刻意模仿 `rw-hid.cs` 的 `ModeRestore`：4 档 DPI → 回报率 → 当前档位 →
去抖 → 角度吸附 → 角度微调 → LOD → 灯光 → 8 个按键 → **能量块最后单独重写并读回断言**。
能量块放最后是因为它是唯一一个「写错会让设备看起来坏了」的项，放在最后才能确保它不被后续操作影响。

### 3. `setdpi <1-4> <dpi> [RRGGBB]`

```powershell
rog-mouse-ctl.exe setdpi 1 500              # 只改 DPI，颜色沿用当前该档的
rog-mouse-ctl.exe setdpi 1 500 ff0000       # DPI 与该档颜色同一帧写下去
```

DPI 与该档颜色在**同一帧**里写（值 `{encLo, encHi, R, G, B}`），这是设备的设计。
范围 100–42000，步进 **50**。

### 4. `setpoll <125|250|500|1000>`

```powershell
rog-mouse-ctl.exe setpoll 500
```

本型号只支持这四档（`7D 20` 的 booster 位为 0，没有 2000Hz+）。

### 5. `setstage <1-4>`

```powershell
rog-mouse-ctl.exe setstage 3
```

当前 DPI 档位。**寄存器是 1-based**（1 就是第 1 档）。

### 6. `setdebounce <0-7>` / `setsnap <0|1>` / `settuning <-20..20>`

```powershell
rog-mouse-ctl.exe setdebounce 4       # 20ms；0=关 1=8ms 2=12 3=16 4=20 5=24 6=28 7=32
rog-mouse-ctl.exe setsnap 1           # 角度吸附开
rog-mouse-ctl.exe settuning 10        # 角度微调 +10（范围 -20..20，步进 5）
```

### 7. `setlod <low|high>`

```powershell
rog-mouse-ctl.exe setlod high
```

⚠️ **写 LOD 会把表面标定重置为默认值**，这是设备行为（G-Helper 源码注释也承认），
没有「只设 LOD」的单独命令。程序会打印写前/写后的标定 k 值供对照。

### 8. `setlight <mode> [亮度] [RRGGBB]`

```powershell
rog-mouse-ctl.exe setlight 0 70 ff0000     # Static 红色 70%
rog-mouse-ctl.exe setlight 240             # 关灯（Off）
```

可用模式：

| mode | 含义 |
|---|---|
| 0 | Static 常亮 |
| 1 | Breathing 呼吸 |
| 2 | ColorCycle 彩色循环 |
| 4 | React 按键反应 |
| 6 | BatteryState 电量指示 |
| 240 | Off 关灯 |

**mode 3（Rainbow）与 5（Comet）本型号不支持** —— 设备不会报错，而是**静默回落到 ColorCycle**。
所以程序在参数校验阶段就直接拒绝，避免用户以为设置成功了。

`dir` / `random` / `speed` 只对 Rainbow/Comet 有意义，而这两个模式不支持，因此本型号永远写 0。
灯区（zone）本型号恒为 0（唯一的灯在滚轮上）。

### 9. `setbutton <0-7> <dstHex>`

```powershell
rog-mouse-ctl.exe setbutton 1 01F0        # 右键改成左键
rog-mouse-ctl.exe setbutton 3 0000 --yes  # 禁用侧键后退（危险，需 --yes）
```

槽位的物理键源码是**固定**的，改的是目标动作码（`dst`）：

| 槽位 | 物理键 | src |
|---|---|---|
| 0 | 左键 | 0x01F0 |
| 1 | 右键 | 0x01F1 |
| 2 | 滚轮按下 | 0x01F2 |
| 3 | 侧键后退 | 0x01E4 |
| 4 | 侧键前进 | 0x01E5 |
| 5 | DPI 键 | 0x01E6 |
| 6 | 滚轮上 | 0x01E8 |
| 7 | 滚轮下 | 0x01E9 |

⚠️ **`dst = 0x0000` 等于禁用该键**，禁掉之后这个键完全没反应，只能靠备份还原。
程序会先警告并要求加 `--yes`，否则以退出码 2 退出。

### 10. `save` —— 提交进 flash

```powershell
rog-mouse-ctl.exe save
```

发 `0x50 0x03`。**回全零是正常的，不是 NAK**（这是文档特别强调的坑）。
不加 `--save` 的 setter 只改 RAM，鼠标重连 / 接收器断电后丢失。

### 11. `raw <cmdHex> <subHex> [tailHex...] [--dongle]` —— 原始帧调试口

```powershell
rog-mouse-ctl.exe raw 12 07 00              # 读电量帧，打完整 64 字节 + 下标
rog-mouse-ctl.exe raw A1 01 --dongle        # 走接收器通道（0xFF02，report id 0x01）
```

**只用于只读核对**（协议逆向、偏移验证）。它会打印：

- 请求的三个字节；
- 完整 64 字节应答的十六进制转储（16 字节一行，带 ASCII 列）；
- 前 24 字节的**下标对照行**，偏移核对时最有用。

写命令不要走这里：写有六步安全循环（自动还原），绕过去就没有保护了。

### 12. `setsleep <code>` / `setlowbatt <0-100>` / `setfulleffect <0|1>` —— 能量块三项

```powershell
rog-mouse-ctl.exe setsleep 4          # 10 分钟无操作后休眠（出厂值）
rog-mouse-ctl.exe setsleep 0xFF       # 永不休眠（⚠️ 见下）
rog-mouse-ctl.exe setlowbatt 15       # 低电量告警阈值 15%
rog-mouse-ctl.exe setfulleffect 0     # 关掉充满电时的灯效
```

**`setsleep` 的 code 是枚举值，不是分钟数**：

| code | 语义 | 证据 |
|---|---|---|
| `0` | 1 分钟 | 实测（事故中观察到） |
| `1` | 2 分钟 | 双源确认（见下），未逐档掐表 |
| `2` | 3 分钟 | 同上 |
| `3` | 5 分钟 | 同上 |
| `4` | 10 分钟 | 实测（空闲 600 秒正好入睡） |
| `255` | 永不 | 实测（事故中观察到永不休眠） |

映射由两个互不相关的第三方逆向实现逐项一致确认：
G-Helper `_ghelper\AsusMouse.cs:9-17` 的 `enum PowerOffSetting`，
以及 RigStats `_research\_rigstats\Mouse_MouseProtocol.cs:71-80` 的 `PowerOffMinutes` 字典。
但**「2/3/5 分钟」是从枚举名推出来的，没有人真的掐表等过** —— 这个区别只对文档有意义，UI 正常显示即可。

`0xFF` 会额外打一条警告：「永不」休眠会显著加快耗电 —— 实测活跃耗电 8.76 %/h
而静置仅 0.148 %/h（差 59 倍），底下的灯也会一直亮着。

#### 设备对这三个字节完全不校验（真机实测，2026-10-21）

这是本节最重要的一条。逐值写入 + 读回，共 15 次：

```
写 0x00 -> 读回 0x00      写 0x05 -> 读回 0x05      写 0x10 (16)  -> 读回 0x10
写 0x01 -> 读回 0x01      写 0x06 -> 读回 0x06      写 0x64 (100) -> 读回 0x64
写 0x02 -> 读回 0x02      写 0x07 -> 读回 0x07      写 0xFE (254) -> 读回 0xFE
写 0x03 -> 读回 0x03      写 0x08 -> 读回 0x08      写 0xFF (255) -> 读回 0xFF
写 0x04 -> 读回 0x04      写 0x09 -> 读回 0x09      写 0x0F (15)  -> 读回 0x0F
```

**`5..254` 这些无定义值全部原样存进 flash。** 同样：

- `b7` 低电量阈值：`0/1/4/5/10/15/20/21/25/30/50/100/255` 全部原样接受
- `b12` 充满效果：`0→0`、`1→1`、**`2→1`、`3→1`、`255→1`**（≥2 一律归一成「开」，不是拒绝）

所以程序里的范围校验**不是防御性编程，而是唯一的安全网**。一旦让无定义值落进设备，
没有任何命令能读回它的语义，也没有恢复命令。三个 setter 都按这个原则把关：

| 命令 | 拒绝的值 | 退出码 |
|---|---|---|
| `setsleep` | 5..254（非 `0..=4` 且非 `255`） | 1，**一帧都不发** |
| `setlowbatt` | > 100 | 1，一帧都不发 |
| `setfulleffect` | > 1 | 1，一帧都不发 |

`setsleep` 和 `setlowbatt` 都接受 `0xFF` 形式的十六进制写法。

### 13. `estimate` —— 电量历史与续航估算

```powershell
rog-mouse-ctl.exe estimate
```

读 `%LOCALAPPDATA%\rog-battery\history.tsv`（**故意用 `rog-battery` 目录名**，
与已停用的 C# 版共用历史数据），打印当前估算，并**顺手追加一条采样**。

```
== estimate（电量历史与估算）==
历史文件    %LOCALAPPDATA%\AppData\Local\rog-battery\history.tsv
已记录      6 条，最早 10-21 15:48，最近 10-21 23:30
当前读数    84%  使用中  无线
归并状态    使用中

估算        预计可用时间：电量还没变化（需要再多观察一会儿）
  参考       用了 4 个样本
  未追加     距上一条太近（去重规则：变化 60s / 心跳 600s）
```

采样去重规则：电量或充放电相位变了至少隔 60 秒记一条，没变则每 10 分钟记一条心跳。
间隔超过 2 小时视为断档，估算不会跨越断档。

写文件用 Windows 命名互斥体 `Local\RogBatteryHistory` 保护，因为 C# 版可能还在跑，
而且 GUI 和命令行也可能同时写。拿不到锁（3 秒超时）就放弃本次写入 —— 丢一条采样远好过写坏文件。

### 目录约定（安装形态）

配置和数据**不放在 exe 同目录**，走系统标准位置：

| 用途 | 位置 |
|---|---|
| 设置 | `%APPDATA%\rog-mouse-ctl\settings.ini` |
| 快照缓存 | `%LOCALAPPDATA%\rog-mouse-ctl\snapshot.cache` |
| 备份 | `%USERPROFILE%\Documents\rog-mouse-ctl\backups\` |

这是为了将来能做成安装包（MSIX 里 exe 不能写自己的安装目录）。

---

## 三、安全设计

每个 setter 都走 `rw-hid.cs` `Cycle()` 的六步：

1. 读原值，打印 `BEFORE`
2. 写新值（只有 `--save` 才顺带 commit）
3. 等 80ms 读回，打印 `AFTER WRITE`，断言 `VERIFY applied=YES/NO`
4. **自动写回原值**
5. 再读回，打印 `AFTER RESTORE`，断言 `VERIFY restored=YES/NO`
6. 只有显式 `--save` 才真正落 flash

也就是**默认跑完是自还原、不留痕迹的**。实测一次完整循环 **约 1.4 秒**（8 帧状态读 + 2 次写）。

还原失败时程序会以非零码退出并提示用备份文件 restore，不会假装成功。

### 六步循环的实现细节（改代码前必看）

**唯一实现**：`ops::cycle` 是这套逻辑的**唯一一份代码**，CLI 和 GUI 都调它。
早期 CLI 里有一份独立实现，重构时删掉了 —— 两套必然漂移，而漂移的那次就是用户设备被改坏的那次。

**`probe` 与 `expect` 必须产出完全同格式的字符串**。这是个真踩过的坑：

```rust
// ❌ 错：probe 返回 "5 分钟 (code 3)"，expect 返回 "3"，永远不相等
cycle(..., |st| format!("{} (code {})", name(st.sleep_mode), st.sleep_mode),
           |_| code.to_string(), ...)

// ✅ 对：expect 复用同一个格式化函数
cycle(..., |st| format!("{} (code {})", name(st.sleep_mode), st.sleep_mode),
           |_| format!("{} (code {code})", name(code)), ...)
```

写错的后果不是「写不进去」，而是**设备明明改对了却报 `applied=NO`**，
很容易被误判成设备故障而去乱试其他命令 —— 对一个能改 flash 的程序来说这很危险。

**只读一次原值**：`cycle` 内部读一次 `State`，然后同一份数据同时交给
`expect`（算期望字符串）和 `build`（造写新值/写回原值两个闭包）。
这样不会出现「写回的值来自另一次读」的错位。

**错误分类**：`OpsError` 带 `ErrorKind`，GUI 靠它区分三种完全不同的用户动作：

| kind | 含义 | 用户该做什么 |
|---|---|---|
| `Invalid` | 参数不合法，**一帧都没发** | 改输入 |
| `Asleep` | 无应答，鼠标休眠中 | 动一下鼠标 |
| `Rejected` | 设备明确 NAK | 看日志/换参数 |
| `Mismatch` | 写进去了但读回不一致 | 用备份核对 |
| `Other` | 其它 | — |

「参数非法」和「设备无应答」混成一句「失败」会让用户瞎试，
而对这个程序来说瞎试意味着往 flash 里写东西。

---

## 四、真机实测记录

测试时间 2026-10-21 21:50–22:35，设备 ROG HARPE ACE MINI（无线 2.4G，经 OMNI 接收器），
链路 `Wireless24G`、report id `0x03`、命令通道 `in=64 out=64`。

### `info --fresh`（19 帧 / 591ms）

```
== 设备身份 ==
  链路        Wireless24G (report id 0x03)
  HID 产品名  ROG OMNI RECEIVER
  HID 序列号  TCMPKRD0000L+
  型号名      ROG HARPE ACE MINI
  有线 PID    0x1B63
  无线 PID    0x1B65
  鼠标 SN     B00000000000
  接收器固件  7.00.07
  鼠标端 MAC  {{MAC_fake02}}
  接收器 MAC  {{MAC_fake01}}
  高回报率    不支持（仅 125/250/500/1000）
  命令通道    in=64 out=64 字节，report id 0x03
  本型号唯一灯区 = zone 0（滚轮）

== 电量 ==
  电量        84%   电压 3995 mV   使用中
  休眠        10 分钟 (code 4)   低电量阈值 20%   电池类型 0   充满效果 1

== 性能与设置 ==
  回报率      1000Hz (code 3)
  DPI 档 1    400 (enc 7)  颜色 #ff1929
  DPI 档 2    800 (enc 15)  颜色 #ff00ff
  DPI 档 3    1600 (enc 31)  颜色 #0000ff
  DPI 档 4    3200 (enc 63)  颜色 #00ff00
  当前档位    2 (1-based，对应 800 DPI)
  去抖        12ms (code 2)   角度吸附 0   角度微调 0
  抬起距离    Low (code 0)
  表面标定    类型 0 材质 255 附加 FF  k值 00 00 00
  灯光        ColorCycle 彩色循环 (code 2)  亮度 50  颜色 #000000  方向 0 随机 0 速度 0
  按键绑定    01F0 01F1 01F2 01E4 01E5 01E6 01E8 01E9
  能量块      休眠 10 分钟 低电量阈值 20% 充满效果 1

== 快照 ==
  帧数        19 帧，耗时 591 ms
```

第二次 `info`（命中缓存）：

```
== 快照 ==
  帧数        19 帧（来自缓存）
  缓存年龄    4 秒前
```

进程总耗时从 **786ms → 203ms**。

**与 `rog-battery.exe --once` 对照**：电量 84%、电压 3999 mV、睡眠模式 4、充满效果 1、
低电量阈值 20%、PID 0x1B65 —— 全部一致。

### `backup`（31 个键）

`%USERPROFILE%\Documents\rog-mouse-ctl\backups\stock-20261021-223130.txt`，31 个键全齐：

```
poll_code=3
dpi1_enc=7   dpi1_rgb=ff1929
dpi2_enc=15  dpi2_rgb=ff00ff
dpi3_enc=31  dpi3_rgb=0000ff
dpi4_enc=63  dpi4_rgb=00ff00
active_stage=2
debounce_code=2
angle_snap=0
angle_tuning=0
lod=0
light_mode=2  light_bright=50  light_rgb=000000  light_dir=0  light_random=0  light_speed=0
button0=496 button1=497 button2=498 button3=484
button4=485 button5=486 button6=488 button7=489
poweroff_code=4
low_warn=20
full_effect=1
```

文件头：

```
# ROG 龙鳞 Ace Mini (Harpe Ace Mini) 设置备份
# 由 rog-mouse-ctl 生成于 2026-10-21 22:31:30
# 链路=Wireless24G report_id=0x03
#
# 注意：写完任何设置都必须 commit（0x50 0x03）才会进 flash，否则重连即丢。
# 注意：DPI 只能取 50 的整数倍，其他值会被设备静默截断。
#
# 表面标定参考（不参与还原）：surface_type=0 material=255 extra=ff k=000000
```

### 各 setter（每条都验证了 applied + restored）

| 命令 | BEFORE → AFTER WRITE | 还原后 | 结果 |
|---|---|---|---|
| `setpoll 500` | 1000Hz → 500Hz | 1000Hz | applied=YES restored=YES |
| `setpoll 250` | 1000Hz → 250Hz | 1000Hz | applied=YES restored=YES |
| `setdpi 1 500` | 400 → 500 | 400 | applied=YES restored=YES |
| `setlight 0 70 ff0000` | ColorCycle/#000000/50 → Static/#ff0000/70 | 原样 | applied=YES restored=YES |
| `setstage 3` | 2 → 3 | 2 | applied=YES restored=YES |
| `setdebounce 4` | 2(12ms) → 4(20ms) | 2 | applied=YES restored=YES |
| `setsnap 1` | 0 → 1 | 0 | applied=YES restored=YES |
| `settuning 15` | 0 → 15 | 0 | applied=YES restored=YES |
| `setlod high` | Low → High | Low | applied=YES restored=YES |
| `setbutton 1 01F0` | 01F1 → 01F0 | 01F1 | applied=YES restored=YES |

拒绝规则（全部实测）：

| 命令 | 退出码 | 提示 |
|---|---|---|
| `setlight 3`（Rainbow） | 1 | 本型号不支持，设备会静默回落 ColorCycle |
| `setlight 5`（Comet） | 1 | 同上 |
| `setdpi 1 99999` | 1 | DPI 范围 100-42000 |
| `settuning 7` | 1 | 步进为 5 |
| `setpoll 2000` | 1 | 只支持 125/250/500/1000 |
| `setbutton 0 0000`（无 `--yes`） | 2 | 警告禁用左键，要求 `--yes` |
| `setbutton 99 01F0` | 1 | 槽位必须是 0-7 |
| `setsleep 5` | 1 | 休眠档位只接受 0..4 或 255（一帧都没发） |
| `setsleep 100` | 1 | 同上 |
| `setlowbatt 101` | 1 | 低电量阈值范围 0-100 |
| `setfulleffect 2` | 1 | 只接受 0 或 1，设备会把 ≥2 归一成 1 |
| `setfulleffect 255` | 1 | 同上 |

### 能量块三个 setter（2026-10-21 23:27 实测）

| 命令 | BEFORE | AFTER WRITE | VERIFY | AFTER RESTORE |
|---|---|---|---|---|
| `setsleep 3` | 10 分钟 (code 4) | 5 分钟 (code 3) | applied=YES | 10 分钟 (code 4) |
| `setsleep 255` | 10 分钟 (code 4) | 永不 (code 255) | applied=YES | 10 分钟 (code 4) |
| `setlowbatt 15` | 20% | 15% | applied=YES | 20% |
| `setfulleffect 0` | 1 | 0 | applied=YES | 1 |

跑完后 `info --fresh` 确认 `能量块  休眠 10 分钟 低电量阈值 20% 充满效果 1` 未变。

### 轻量读取（`quick` / `info --light`，2026-10-21 23:29 实测）

```
state=ok
level=84            voltage_mv=3985      charging=0
sleep_mode=4        low_warn=20          full_effect=1
poll=1000Hz         poll_code=3          debounce_code=2
angle_snap=0        angle_tuning=0
active_stage=2      profile=0
frames=3            cached=no            elapsed_ms=96
```

| 模式 | 帧数 | 帧耗时 | 进程总耗时 |
|---|---|---|---|
| `quick` / `info --light` | 3 | **93–97ms** | 273–327ms |
| `info`（全量） | 19 | 590ms | 786ms |

连跑 3 次全部 `frames=3 elapsed_ms≈95`，并与全量 `info` 的字段逐项吻合
（84% / 3985mV / 1000Hz / 档位 2 / 去抖 2 / snap 0 / tuning 0）。

**已验证 3 帧快照不污染磁盘缓存**：`cache clear` → `quick` → 检查
`%LOCALAPPDATA%\rog-mouse-ctl\snapshot.cache` **文件根本没被创建**。

#### 休眠路径怎么验的（这段是方法论，值得看）

「第一帧 MISS 就返回」这条逻辑**在真机上验不了** —— 要触发它得让鼠标空闲满 10 分钟，
而测试期间用户一直在用鼠标（23:31 和 23:43 两次尝试都发现它醒着，电量还从 84% 掉到 83%）。
等下去只会反复失败，而且**故意让鼠标睡 10 分钟**这件事本身就干扰用户。

所以改成两步：

**第一步，量清单帧 MISS 的真实成本**（`raw 12 FF`，这条 cmd 设备不实现，必然 MISS）：

```
第 1 次: wall=1151ms
第 2 次: wall=1099ms
第 3 次: wall=1098ms
进程启动基线（--help）: 10ms
```

即 **单帧 MISS ≈ 1.1 秒**（协议超时 900ms + 约 200ms 设备打开）。
`fetch_light` 用 1200ms 超时，所以：休眠时提前返回约 **1.4s**，
不提前返回要付 3 倍约 **3.8s** —— 对每 60 秒刷一次的 GUI 后台线程，这是「流畅 vs 明显卡顿」的区别。

**第二步，把发包抽成可注入的闭包，用假探针验时序**：

```rust
pub fn fetch_light_with<F>(report_id: u8, link_mode: &str, ask: F) -> FetchOutcome
where F: FnMut(u8, u8, u8, &[u8]) -> Option<Vec<u8>>
```

四个测试锁死行为：

| 测试 | 断言 | 为什么要这条 |
|---|---|---|
| `light_stops_after_first_miss` | `asked == vec![(0x12, 0x07)]` | 第一帧 MISS 后**一次都不多问** |
| `light_reads_all_three_when_awake` | `asked == 3`，三帧值都正确 | 醒着时不能因为「有 failed 就早退」而漏读 |
| `light_keeps_going_when_second_frame_misses` | `asked == 3` | 第一帧读到就说明鼠标醒着，第二帧偶发丢失该继续读完 |
| `light_nak_is_not_a_miss` | `asked == 3` | NAK 是设备**明确回话**，说明它醒着，不该早退 |

**教训**：不要把「无法在真机上复现的状态」当成「不需要验证」。
把它拆成「量清成本」+「抽出可注入的时序逻辑」，两件都能离线验，而且比真机手测更严密
（真机只能验一次，单测验每一种分支）。

### 持久化

```
setpoll 500 --keep --save   → 500Hz，独立进程 info --fresh 复核仍为 500Hz（确认进了 flash）
setpoll 1000 --keep --save  → 改回 1000Hz，独立进程复核 1000Hz
restore stock-20261021-223130.txt → 全部回出厂，能量块读回 YES
```

### `raw A1 01 --dongle`（连跑 3 次，逐字节相同）

```
== raw ==
请求        report id 0x01  cmd 0xA1  sub 0x01  tail []
应答        64 字节
  0000  01 A1 01 00 00 07 00 07 00 00 00 00 00 00 00 00  |................|
  0010  00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00  |................|
  0020  00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00  |................|
  0030  00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00  |................|
```

`b5.b6.b7` = `07.00.07` 即接收器固件版本。

### 最终的 `rog-battery.exe --once`（原始输出）

```
[22:30:20] ROG 龙鳞 Ace Mini   电量 84% (绿)   电压 3999 mV  📶2.4G 🔋使用中
            PID 0x1B65   鼠标   低电量阈值 20%   电池类型 0   充满效果 1   睡眠模式 4
            预计还能用 86 小时 30 分钟（52 个样本，跨度 567 分钟）
```

**设备已完全恢复出厂状态**（回报率 1000Hz、4 档 DPI 400/800/1600/3200、当前档位 2、
去抖 12ms、LOD Low、灯光 ColorCycle、8 个按键原样、能量块 10 分钟/20%/充满效果 1）。

### 编译

```
Finished `release` profile [optimized] target(s) in 8.28s
D:\codex\rog-mouse-ctl\target\release\rog-mouse-ctl.exe   925,184 字节（904.0 KB）
```

**零 warning**。导入表只有 `KERNEL32.dll` `hid.dll` `SETUPAPI.dll` `ntdll.dll` `msvcrt.dll`
`user32.dll` `comctl32.dll` `ole32.dll` `gdi32.dll` `advapi32.dll` `shell32.dll`
（GDI+ 自绘改造后会多一个 `gdiplus.dll`），**没有 `vcruntime140` / `msvcp140`** ——
绿色版单文件可直接分发，不需要 VC++ 可再发行包。

---

## 五、注意事项

### 1. 改完必须 commit，否则丢了别来找

设备的写命令只改 **RAM 里的寄存器文件**，不发 `0x50 0x03` 就不会进 flash。
鼠标重连、接收器断电、甚至深度休眠后都会回到 flash 里的旧值。
本程序所有 setter 默认**不** commit（因为默认还要还原），要留下改动必须显式加 `--save`。

### 2. DPI 只能取 50 的整数倍

编码是 `enc = (dpi - 50) / 50`，步进 **50**。写 437 会被设备**静默截断**成 400，
程序会提前提示「437 会被截断成 400」，读回也会是 `applied=NO`（如果只改 DPI 的话）。
上限 42000（enc=839）。

### 3. 和 ASUS 软件互斥

Armoury Crate / GearLink 开着的时候可能：

- 独占 HID 句柄 → 本程序打不开（会明确报错）；
- 同时改同一项 → 两边互相覆盖，谁后 commit 谁算。

本程序的 `CreateFile` 用了 `FILE_SHARE_READ | FILE_SHARE_WRITE`，所以**通常**能和它们共存，
但「同时改同一个值」这种竞争无法避免。改完记得读 `.exe info` 复核。

### 4. 写 LOD 会重置表面标定

厂商源码（`_ghelper\AsusMouse.cs:1626`）明确承认这一点：

```csharp
//This also resets the "calibration" to default. There is no seperate command to only set the lift off distance
```

且**整个写命令空间里没有任何命令能把标定写回去**。标定值记在 `12 06` 的 k9..k11，
**本机的标定已经被重置过（现状全 0）**，恢复不了。

本程序会把它写进备份文件的注释行（但不参与还原，因为写不回去），
`setlod` 也会打印写前/写后的 k 值供对照。
**如果你很在意抬升手感，改 LOD 前先想清楚 —— 这是不可逆的。**

### 5. 按键 `0x0000` 是禁用，不是「无功能」

写 `dst = 0x0000` 之后这个键**完全没反应**，没有任何 UI 恢复手段，只能靠备份文件还原。
程序会警告并要求 `--yes`，但这只是防手滑，不是保险。

### 6. 鼠标休眠时所有命令都无响应 —— 这不是错误

鼠标 **10 分钟无操作**（`sleep_mode = 4`）会休眠，此时**所有**命令都超时无回包。
程序会报「鼠标无应答（休眠中？先移动一下鼠标）」，**移动一下鼠标**再跑就行。

程序刻意**不做重试**：一次 MISS 要等满 1200ms 超时（实测含设备打开约 1.1 秒，
`raw 12 FF` 三次：1098/1099/1151ms），而正常往返只要 27ms。
重试 3 次 = 3.3 秒白等，没有任何意义 —— 而且休眠是设备的正常状态，不是故障。

### 7. 备份必须包含能量块（血泪教训）

文档第 5 节记录过一次真实事故：手写能量包把「充满效果」从 1 改成 0，commit 之后
**休眠时间字节从 0x00(1 分钟) 变成 0xFF(永不)**，然后用被污染的基线「恢复」成 1 分钟，
而真实原值是 4（10 分钟）—— 结果鼠标 1 分钟就休眠，看起来像是坏了。

根因是**读写偏移不一样**：

```
读 12 07：aligned[6]=休眠  aligned[7]=低电量  aligned[12]=充满效果
写 51 37：tail[2]=休眠     tail[4]=低电量     tail[6]=充满效果     （tail 从 pkt[3] 起）
```

即 `tail[n]` 对应 `aligned[n+4]`。正确帧：`51 37 00 00 04 00 14 00 01` = 10 分钟 / 20% / 充满效果开。

本程序的 `restore` 会把能量块**放在最后单独重写并读回断言**，
备份缺这三个键时会打印明确警告并跳过。

### 8. 只碰 `0x51` 命令族

写命令只用 `cmd 0x51`（`sub 0x31` 寄存器文件、`sub 0x35` LOD、`sub 0x28` 灯光、`sub 0x21` 按键）
和 `cmd 0x50 sub 0x03` 提交。文档明确说 `0x50`–`0x5F` 之外的写命令都别发。
读只用已验证会回包的 `0x12` / `0x7D` / `0xFB` 三族。

**刻意不读 MotionSync（`12 04 04`）**：本型号不响应，一次 MISS 要等满超时（约 1.1 秒），
为永远不来的值付这个代价没有意义。这也是早期 `read` 从 451ms 涨到 1679ms 的原因。

---

## 六、双形态启动与托盘图标

这两块各自有一个「看起来能用、实际会坏」的陷阱，单独成文：

- **[`docs/startup-modes.md`](docs/startup-modes.md)** —— GUI 无黑框 + CLI 可重定向
  为什么不用 `#![windows_subsystem = "windows"]`：那条路会让 `> file` 重定向产出 **0 字节空文件**（实测），废掉所有脚本化用法。
  正确做法是保持 console subsystem，GUI 路径开头调 `FreeConsole()`（实测 `GetConsoleWindow` 984974 → 0）。

- **[`docs/tray-icon.md`](docs/tray-icon.md)** —— 托盘图标
  `TrayNotification` 的 icon 为 `None` 会 **panic**，而 `OemIcon` 没有「应用图标」变体。
  用 `assets/tray.ico`（16/24/32/48/256）+ `include_bytes!` + `Icon::builder().source_bin()`。
  **千万不要加 `.size()`** —— 会走 WIC 路径，对 PNG 压缩的 ICO 条目报
  `ResourceCreationError("Failed to create a image factory")`（实测五个尺寸全挂）。

两个可复现的探针源码留在 `scratch/`：`consoletest.rs`（方案 A 失败证据）、`freeconsole.rs`（方案 B 成功证据）、`icotest/`（ico 加载验证）。

---

## 六之二、图形界面（本机真机验证，2026-10-22）

不带任何参数启动就是 GUI（`--autostart` 也进 GUI，供开机自启用）。五个标签页：

| 页 | 内容 |
|---|---|
| **电量** | 电量百分比 + 进度条、电压、充放电状态、48 小时历史曲线、休眠时间 / 低电量阈值 / 充满效果 |
| **性能** | 回报率、4 档 DPI + 各自的灯光颜色、当前档位、去抖、角度吸附、角度微调、抬起距离 |
| **灯光** | 模式、亮度、颜色、方向 / 随机 / 速度 |
| **按键** | 8 个键的映射（6 键改键 + 滚轮上下） |
| **设置** | 托盘开关、关闭到托盘、开机自动启动、后台刷新间隔、快照缓存有效期 |

写操作全部走 `ops::*`，与 CLI 是**同一份六步循环实现**——不是两套。GUI 只是
「读控件 → 调 `ops::set_*` → 把 `ApplyReport` 渲染成文字」。

### HID 绝不在 GUI 线程

休眠时一次查询失败要 1.3 秒（`Ask` 超时 400ms × 3 次重试）。放在 GUI 线程上就是
「点一下卡一秒半」。实际做法：

- 后台线程跑 `Signal { poke: Mutex<u64>, cv: Condvar, interval_ms: AtomicU64 }`，
  用 `cv.wait_timeout` 睡到下一个周期或被 `request_refresh()` 叫醒
- 拿到数据后 `nwg::Notice` 的 `sender.notice()` 唤醒 GUI 线程
- 界面读的是 `Arc<Mutex<UiState>>` 里的**已算好结果**，不碰设备

### 历史曲线：不能在 `WM_PAINT` 里读文件

`history::read_rows()` 要抢 `Local\RogBatteryHistory` 互斥量，**超时 3000ms**。
而 `WM_PAINT` 会被反复重发（窗口移动、遮挡恢复、`InvalidateRect`），
在里面读盘意味着界面每次重绘都可能卡 3 秒。

正确结构（`src/chart.rs`）：

```
后台线程（每 30 秒） → read_rows() → 算成点集 → Arc<Mutex<Vec<Point>>> 里替换
WM_PAINT            → lock 后 clone 点集 → 只画，不做任何 I/O
```

曲线用 `WM_PAINT` 子类化自绘：`bind_raw_event_handler` 绑在页面句柄上，
**事件 id 必须大于 `0xFFFF`**（nwg 对低 id 会 panic，实测用 `0x00C4_0001`）。

### 托盘图标画实时电量数字

`src/tray_icon.rs` 用 GDI 动态生成：`CreateDIBSection` → `DrawTextW` → `CreateIconIndirect`。
`data = None` 时回落到静态 `assets/tray.ico`。

三个非显而易见的坑（详见 [`docs/nwg-gotchas.md`](docs/nwg-gotchas.md) 第 16–18 节）：

1. `ICONINFO` 必须写满**五个**字段。少写 `xHotspot`/`yHotspot` 时
   `CreateIconIndirect` 返回 NULL 且 `GetLastError() = 203`（`ERROR_ENVVAR_NOT_FOUND`）——
   错误码与图标毫无关系，会把人引去查 PATH
2. `DrawTextW` 在 32bpp DIB 上**把 alpha 写成 0**（文字全透明）。必须事后遍历整张
   位图把 alpha 拉成 `0xFF`；另外抗锯齿要用 `ANTIALIASED_QUALITY(4)`，
   `CLEARTYPE(5)` 会留下彩色描边
3. `CreateIconIndirect` 之前必须先把 DIB 从 DC 里 `SelectObject` 换出去

尺寸按 `SM_CXSMICON` 运行时取（本机是 **16**，不是 32）。硬画 32 会被系统双线性
缩到 16，笔画糊成一团。

GDI 对象泄漏有测试守着：`repeated_renders_do_not_leak_gdi_objects` 画 200 次，
断言 `GetGuiResources(GR_GDIOBJECTS)` 增长不超过 10。

### 开机自启：两条完全不同的路径

判据只有一个 —— **有没有 MSIX 包身份**（`GetCurrentPackageFullName` 返回 `15700`
= 无包）。不能拿「WinRT 能不能用」当判据：无包身份时 `RoGetActivationFactory`
照样返回 `S_OK`，失败在更后面。

| 形态 | 写入位置 | 说明 |
|---|---|---|
| 绿色版 | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\RogMouseCtl` | 值 = `"<exe>" --autostart`（路径必带引号） |
| MSIX | WinRT `StartupTask`（`TaskId=RogBatteryAutoStart`） | 写 Run 键会被**注册表虚拟化**重定向进包私有 hive，系统启动项里看不到 |

`state()` 返回五种状态而不是 `bool`：`DisabledByUser`（用户拒绝过，再请求会静默失败）
与 `DisabledByPolicy`（组策略禁止）都是「没启用」，但用户点「开启」后的行为完全不同。

详见 [`docs/autostart.md`](docs/autostart.md)。

### 已知未做

- 历史曲线只有折线，没有悬停提示（需要额外处理 `WM_MOUSEMOVE` 与坐标反算）
- 设置页的「后台刷新间隔」只影响 GUI 自己的定时器，不影响 `--autostart` 起的进程
## 七、未解之谜（留给后来人）

1. **表面标定当前是全 0**：文档记录的基线是 `1C 2D 0F`（28/45/15），
   但实测 `12 06` 原始帧是 `03 12 06 00 00 00 FF FF 00 00 00 00` ——
   标定 k 值全 0、`material` 从 `01` 变成 `FF`。
   推测是逆向阶段反复写 LOD 把它重置掉了（这正是文档警告的副作用）。
   厂商源码（`_ghelper\AsusMouse.cs:1626`）确认「没有单独设置 LOD 的命令」，
   且整个写命令空间里**没有任何命令能把标定写回去**（`0x51 0x35` 的参数位是
   `FF 00 FF <0/1>` 掩码语义，没留标定字段）。
   本程序不擅自恢复它 —— **恢复不了，这是永久性的**。
   备份文件的注释行会记录当前值，方便以后换机器时对照。

2. **`12 06` 的 b7 语义未确认**：实测恒为 `0xFF`，程序原样记录为「附加 FF」。

3. **接收器 `A1 01` 的固件偏移**（已实测确认）：
   文档写的是 `b6.b5.b4`，但这个**不是字节序反了，是文档的原始帧漏了一个字节**。

   实测 3 次完全一致的完整 64 字节帧：

   ```
   01 A1 01 00 | 00 07 00 07 | 00 00 ... (后 56 字节全 0)
   b0 b1 b2 b3   b4 b5 b6 b7
   ```

   - `b0` report id、`b1` cmd `A1`、`b2` sub `01`、`b3` idx `00`
   - `b4` 填充，恒 `00`
   - `b5`/`b6`/`b7` = `07`/`00`/`07` → 按 `b5.b6.b7` 解出 `7.00.07`

   文档记的 `01 A1 01 00 07 00 07 00` 少了 `b4` 那个 `00`，所以后面的值整体右移一位。
   程序按 **`b5.b6.b7`** 实现（`proto::parse_dongle_fw`）。

   注意 **`A1 01` 只有 8 字节有效载荷**，不是 64 字节 —— 这一点和鼠标侧帧不同。

4. **~~接收器 SN 末尾多一个 `+`~~ → 已查明：是固件 bug，不是「相邻字段」**

   现象：`HidD_GetSerialNumberString` 返回的序列号后面跟 2 个垃圾码点，
   而协议字段 `A1 00` 的前 12 字符永远是干净的 `TCMPKRD0000L`。

   **根因（2026-10-21 实测确认）**：OMNI 接收器的 HID 字符串描述符 `bLength`
   多声明了 2 个字符，Windows 照 `bLength` 读就越界拿到固件 RAM 的残留数据。

   证据一 —— **同一接口连跑 `info` 10 次，垃圾值在循环**：
   ```text
   第 1 次  U+0004 U+002B
   第 2 次  U+0044 U+002B
   第 3 次  U+0084 U+002B
   第 4 次  U+00C4 U+002B
   第 5 次  U+0004 U+002B      ← 回到起点
   ...
   ```
   第一字符按 **0x40（64 字节）步长循环** = 某个分配缓冲区的地址低字节；
   第二字符恒为 `0x2B`（`+`）= 相邻的固定字节。
   **之前记录的「末尾多一个 `+`」就是这个 `0x2B`，它不是数据的一部分。**

   证据二 —— **枚举全部 12 个 HID 接口，不同接口给不同垃圾值**：
   ```text
   8 个接口:  ... 004C 000C 0028   →  "TCMPKRD0000L" + U+000C + U+0028
   4 个接口:  ... 004C 0027 002A   →  "TCMPKRD0000L" + U+0027 + U+002A
   ```

   **修法**：`hid.rs::read_hid_string` 只对序列号取**开头的连续 ASCII 字母数字串**。
   判据是 ASUS 序列号固定 12 位字母数字（协议字段、鼠标 SN `B00000000000`、
   ASUS 自己的 `UnifyDGDevice.xml` 三重印证）。
   修后 `info` 连跑 10 次全部得到干净的 `TCMPKRD0000L`。

   试过但**不可行**的方案（免得后人重走）：`trim_end_matches(|c| !c.is_ascii_graphic())`
   —— 垃圾第二字符可能是 `+`/`*`，都是可打印 ASCII，从尾部 trim 一个都去不掉。

   **权威值始终是协议字段 `A1 00`**，HID 字符串只用于显示。

5. **`12 04 00` 回报率解析的两个分支**：文档没写清楚 b13 的高 4 位和低 3 位分别是什么。
   程序的做法是：先取 `high = (raw >> 4) & 0x0F`，`high > 0` 就用它，否则用 `raw & 0x07`。
   本机实测 b13 = `0x03`，两种算法都得 1000Hz，所以**这个分支还没被真正验证过**。
   如果以后遇到 b13 高位非 0 的机器，需要重新确认。

6. **休眠档位 `b6` 的映射双源确认，但触发时长未逐档掐表**

   两个互不相关的第三方逆向实现逐项一致：

   | code | 分钟 | G-Helper `AsusMouse.cs:9-17` | rigstats `Mouse_MouseProtocol.cs:71-80` |
   |---|---|---|---|
   | `0` | 1 | `Minutes1 = 0` | `[0] = 1` |
   | `1` | 2 | `Minutes2 = 1` | `[1] = 2` |
   | `2` | 3 | `Minutes3 = 2` | `[2] = 3` |
   | `3` | 5 | `Minutes5 = 3` | `[3] = 5` |
   | `4` | 10 | `Minutes10 = 4` | `[4] = 10` |
   | `255` | 永不 | `Never = 0xFF` | `[0xFF] = null` |

   `4 → 10 分钟` 另有物理实测吻合：从用户最后一动鼠标起 **600 秒整**设备进入休眠，之后查询无应答。

   **没验证的是触发时长**：`1/2/3` 档的「2/3/5 分钟」来自协议枚举名，没有人真的掐表等过。
   映射本身可信，具体几秒入睡没逐档测。

   **设备不校验这个字节**：真机扫 `0..9 / 15 / 16 / 100 / 254 / 255` 共 15 个值，
   **全部原样存进 flash**，包括 `5..254` 这些无定义值。所以 `ops::set_sleep` 的范围校验
   不是防御性编程，而是唯一的安全网 —— 让一个无定义值落进设备，**没有恢复命令**。

7. **低电量阈值 `b7` 和充满效果 `b12` 同样不校验**

   - `b7`（百分比）：写 `0/1/4/5/10/15/20/21/25/30/50/100/255` 全部原样接受。
     写 `255` 会造成「电量 84%，低电量阈值 255%」永远告警 → setter 必须钳到 `0..=100`。
   - `b12`（充满效果）：写 `0→0`、`1→1`，而 **`2/3/255` 一律归一成 `1`**
     （静默改写，不是拒绝）→ 只放行 `0/1`。

8. **`restore` 曾是绕过所有参数校验的后门（已修，2026-10-21）**

   背景：所有 setter 走 `ops::set_*` 的六步安全循环，参数范围在那里校验。
   但 `restore` 刻意**不走 ops** —— 它是「按备份原样铺回去」，不该有
   「写-读回-自动还原」那套循环（否则每项都被临时改两次）。
   **代价是：绕过 ops 也就绕过了校验。**

   实测出的两个真 bug（都是「备份文件残缺时静默把设备改坏」，和 §5 那次
   能量块事故同一类 —— **缺失的键不该被当成零值写进设备**）：

   | Bug | 原行为 | 现在 |
   |---|---|---|
   | 缺 `dpiN_rgb` 键 | `unwrap_or([0,0,0])` → 颜色被**刷成黑色** | 读设备当前值沿用 |
   | `dpiN_enc` 无范围校验 | `dpi1_enc=999` → **50000 DPI** 真的写进设备 | 超范围中止整份还原 |

   边界行为（实测）：`enc=0`（50 DPI）拒绝、`enc=1`（100 DPI）接受、
   `enc=839`（42000 DPI）接受、`enc=840` 拒绝。
   错误文案统一为「合法 `1..=839`，即 `100..=42000` DPI」。
   **下限是 1 不是 0**：`decode_dpi(0) == 50`，低于统一的对外下限 100。

   权威常量现在在 `src/proto.rs`：`DPI_MIN=100` / `DPI_MAX=42000` /
   `DPI_ENC_MIN=(100-50)/50=1` / `DPI_ENC_MAX=(42000-50)/50=839`。
   `ops::set_dpi` 与 `cmd_restore` 都引用它们，不再各自硬编码。

   **同一类的最后一处也已修完（2026-10-22）**：`cli.rs` 的灯光段原来是
   `light_bright.unwrap_or(50)` / `light_rgb.unwrap_or([0,0,0])` /
   方向·随机·速度 `unwrap_or(0)`。备份缺这些键时会静默改成 50 亮度 /
   黑色 / 方向 0 —— 用户看到的就是「还原备份后灯变黑了」。
   改法照 DPI 段：缺键就沿用设备当前值，且**只在确实缺字段时才读设备**
   （完整的备份不该多付一帧的代价）。

   决定性验证（关键是先把设备设成**既不是 50 也不是 `#000000`**，
   否则新旧行为无法区分）：

   | 步骤 | 结果 |
   |---|---|
   | `setlight 2 80 ff8800 --keep --save` | 设备变成 亮度 80 / `#ff8800` |
   | `restore partial-light.ini`（只有 `light_mode=1`） | `灯光 -> Breathing 亮度 80 颜色 #ff8800` |
   | 读回断言 | `Breathing (code 1) 亮度 80 颜色 #ff8800` ✅ |

   旧代码在这一步会输出 `亮度 50 颜色 #000000`。

   ⚠️ **做这个实验有个坑**：`restore` **不走 ops 的六步循环**，写下去就
   永久生效（不像 setter 会自动还原）。我第一次测试时设备是
   `ColorCycle(2)`，跑完变成 `Breathing(1)`，必须手工改回去。
   **拿 `restore` 做实验前先记下设备当前值，跑完逐项核对。**

9. **`read_hid_string` 的 `take_while(is_ascii_alphanumeric)` 对带 `-` 的型号会误截**

   现在的判据是「取开头的连续 ASCII 字母数字」，用来丢掉 §4 那个固件垃圾尾巴。
   代价：若将来遇到序列号含 `-` 或 `.` 的型号（如 `TCMP-KRD0052`），
   会被截成 **`TCMP`** —— 不是截断一部分，是只剩第一段，且**静默给出错误答案**。

   已知的改法是「允许 `[A-Za-z0-9._-]`」，**但现在没有设备可验**，
   改了也是猜。而且固件垃圾实测可能就是可打印字符（`+` 0x2B / `*` 0x2A），
   说明「用字符白名单猜边界」这条路本身脆弱。

   当前选择：**保守截断（宁可少留不多留）+ 明确规则「协议字段才是权威值」**。
   等真遇到带 `-` 的型号再改，那时有设备可验。

10. **哪些 GDI/nwg 的失败是「静默给错答案」而不是报错**（2026-10-22 补齐）

    做托盘图标时连撞三个：`CreateIconIndirect` 少写两个字段返回 NULL 且错误码是
    `203 ERROR_ENVVAR_NOT_FOUND`（与图标无关）；`DrawTextW` 在 32bpp DIB 上把 alpha
    写成 0（文字存在但全透明，看起来像「没画」）；`CreateIconIndirect` 前忘了
    `SelectObject` 换出位图。三个都**编译期零警告、运行时不抛异常**，只能靠肉眼
    或像素采样发现。

    → 写进 [`docs/nwg-gotchas.md`](docs/nwg-gotchas.md) 第 16–18 节。共同规律：
    **Win32 的返回值 NULL/FALSE 只说明「失败了」，`GetLastError` 未必指向真实原因**；
    涉及 GDI 对象时优先怀疑「句柄还在 DC 里」「位图格式不对」「结构体大小不对」。
