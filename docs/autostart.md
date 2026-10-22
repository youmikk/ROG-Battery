# 开机自启：两条路径的完整实测（2026-10-22）

本文记录 `rog-mouse-ctl` 开机自启的实现依据。**两条路径不能混用**，判据是「有没有包身份」，
不是「WinRT 是否可用」。

---

## 一、为什么必须分流

MSIX 应用写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 会被**注册表虚拟化**
重定向进包私有 hive（`%LOCALAPPDATA%\Packages\<Identity>\SystemAppData\Helium\User.dat`），
系统的启动项列表里**看不到**条目 → 自启静默失效。

反过来，绿色版没有包身份，调 WinRT `StartupTask` 会返回 `0x80070490`
（`ERROR_ELEMENT_NOT_FOUND`）。

所以：

| 形态 | 路径 | 判据 |
|---|---|---|
| 绿色 exe | `HKCU\...\Run` 写值 | `GetCurrentPackageFullName` 返回 15700 |
| MSIX | WinRT `StartupTask` | 返回 122（或有包全名） |

---

## 二、判据 API

```rust
#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentPackageFullName(len: *mut u32, name: *mut u16) -> i32;
}
```

实测（本机绿色版）：

```
GetCurrentPackageFullName(len=null) = 15700  (0x3D54)
  → APPMODEL_ERROR_NO_PACKAGE
```

- 传 `null` 且**有**包身份 → 返回 `122`（`ERROR_INSUFFICIENT_BUFFER`），
  `len` 被填成需要的字节数（含结尾 NUL，单位是 **wchar**，即 `len/2` 个 u16）。
- **直接链 kernel32 即可**，不需要 `libkernelbase.a`（rustlib 里没有这个 .a，
  但 kernel32 的导入库把它当转发导出，`ld` 能解）。

---

## 三、绿色版：注册表

`libadvapi32.a`（694,266 B）在 rustlib 的 `self-contained` 里，可直接 `#[link(name = "advapi32")]`。

```
HKCU\Software\Microsoft\Windows\CurrentVersion\Run
  值名: RogMouseCtl
  类型: REG_SZ
  数据: "C:\path\to\rog-mouse-ctl.exe" --autostart
```

**实测全通**：

```
RegCreateKeyExW = 0  disp=1 (1=新建 2=已存在)
RegSetValueExW  = 0  (78 B)
RegQueryValueExW= 0  ty=1 (REG_SZ)  len=78 B
读回 = C:\probe\rog-mouse-ctl.exe --autostart     ← 逐字符一致
RegDeleteValueW = 0
删除后再查     = 2 (ERROR_FILE_NOT_FOUND)          ← 删对了
```

**写入要点**：

- `RegSetValueExW` 的 `cbData` 参数是**字节数**，且 **REG_SZ 必须含结尾 NUL**
  → `(s.encode_utf16().count() + 1) * 2`
- 路径含空格时**必须加引号**（`"C:\Program Files\..."`），否则系统按空格切分参数
- 用 `HKEY_CURRENT_USER`（`0x8000_0001`），不要 HKLM（需要管理员）

**读回判断「已启用」**：`RegQueryValueExW` 返回 `0` 且值与预期一致。
不要只判断键存在 —— 用户可能改过路径。

---

## 四、MSIX：WinRT `StartupTask`

### 4.1 两个前置条件（缺一必失败）

1. **必须 `RoInitialize(1)`**（`RO_INIT_MULTITHREADED`）。
   不调的话 `RoGetActivationFactory` 返回 `0x800401F0`（`CO_E_NOTINITIALIZED`）。
   > 我一开始误以为 `0x800401F0` 是「没有包身份」，实测证明**完全是没初始化**：
   > 包外调了 `RoInitialize` 之后工厂照样拿到（`S_OK`）。
2. `RoInitialize` 返回 `0x80010106`（`RPC_E_CHANGED_MODE`）**不是致命错误**，
   说明该线程已用别的套间模型初始化，可以继续。

### 4.2 combase 必须动态加载

```
ld: cannot find -lkernelbase: No such file or directory
ld: cannot find -lcombase: No such file or directory
```

rustlib 的 `self-contained` 里**没有** `libkernelbase.a` / `libcombase.a`。
所以走 `LoadLibraryW("combase.dll")` + `GetProcAddress`，与工程里 `hid.dll` 的做法一致。

需要的导出：

```
WindowsCreateString / WindowsDeleteString / WindowsGetStringRawBuffer
RoGetActivationFactory / RoInitialize / RoUninitialize
```

### 4.3 IID：**不要自己算**

网上流传的「SHA1(接口名大写) 前 16 字节 + 改第 6/8 字节」算法，我实测**是错的**：

```
用 Windows.Foundation.IActivationFactory 对照：
  算法算出:  00000043-00fe-5d9d-af27-bb3862cf7460
  真实值:    af86e2e0-b12d-4c6a-9c5a-d7aa65101e90    ❌ 完全不符
```

**正确来源是系统 WinRT 元数据**：

```powershell
[Type]::GetType("Windows.ApplicationModel.IStartupTaskStatics, Windows.ApplicationModel, ContentType=WindowsRuntime").GUID
  → ee5b60bd-a148-41a7-b26e-e8b88a1e62f8
[Type]::GetType("Windows.ApplicationModel.IStartupTask, Windows.ApplicationModel, ContentType=WindowsRuntime").GUID
  → f75c23c8-b5f2-4f6c-88dd-36cb1d599d17
```

元数据文件在 `C:\Windows\System32\WinMetadata\Windows.ApplicationModel.winmd`。

亦可运行时自证：对工厂调 `GetIids`（vtable `[3]`），返回的第二个 IID
就是 `IStartupTaskStatics`：

```
[factory] GetIids = 0  count=2
    00000035-0000-0000-c000-000000000046     ← IActivationFactory
    ee5b60bd-a148-41a7-b26e-e8b88a1e62f8     ← IStartupTaskStatics ✅
```

`IActivationFactory` 的 IID 是固定的 `AF86E2E0-B12D-4C6A-9C5A-D7AA65101E90`。

### 4.4 vtable 槽位：**WinRT 继承 IInspectable，不是 IUnknown**

这是最容易踩死的地方。WinRT 接口比 COM 多 3 个槽：

| 槽位 | 方法 | 说明 |
|---|---|---|
| 0 / 1 / 2 | `QueryInterface` / `AddRef` / `Release` | IUnknown |
| **3 / 4 / 5** | **`GetIids` / `GetRuntimeClassName` / `GetTrustLevel`** | **IInspectable 追加** |
| 6 起 | 接口自己的方法 | 每个接口各自从 6 数起 |

**我把 `[4]` 当 `get_Status` 调，进程直接 `0xC0000005` 崩溃。**
原因：`[4]` 是 `GetRuntimeClassName`，它写的是 `HSTRING`（8 字节指针），
我传的是 `&mut i32`（4 字节栈槽）→ **写越界 4 字节，栈损坏**。

#### 权威槽位表（来源：`[Type]::GetType(...).GetMethods()` 读系统元数据）

不要手数，也不要相信网上流传的「SHA1(接口名大写) 取前 16 字节」那套算法
——我验算过 `IActivationFactory`，算出来是
`00000043-00fe-5d9d-af27-bb3862cf7460`，真值是
`af86e2e0-b12d-4c6a-9c5a-d7aa65101e90`，**完全不符**。正确来源是
`C:\Windows\System32\WinMetadata\Windows.ApplicationModel.winmd`：

```powershell
[Type]::GetType("Windows.ApplicationModel.IStartupTaskStatics, Windows.ApplicationModel, ContentType=WindowsRuntime").GetMethods()
[Type]::GetType("Windows.ApplicationModel.IStartupTask, Windows.ApplicationModel, ContentType=WindowsRuntime").GetMethods()
[Type]::GetType("Windows.Foundation.IAsyncInfo, Windows.Foundation, ContentType=WindowsRuntime").GetMethods()
```

**`IStartupTaskStatics`**（2 个方法）

| 槽 | 方法 |
|---|---|
| 6 | `GetForCurrentPackageAsync()` ← **不是** `GetAsync` |
| 7 | `GetAsync(String taskId)` |

**`IStartupTask`**（4 个方法）

| 槽 | 方法 |
|---|---|
| 6 | `RequestEnableAsync()` → `IAsyncOperation<StartupTaskState>` |
| 7 | `Disable()` → Void |
| 8 | `get_State()` → `StartupTaskState` |
| 9 | `get_TaskId()` → String |

**`IAsyncInfo`**（独立 vtable，5 个方法）
IID = `00000036-0000-0000-c000-000000000046`

| 槽 | 方法 |
|---|---|
| 6 | `get_Id` |
| 7 | `get_Status` |
| 8 | `get_ErrorCode` |
| 9 | `Cancel` |
| 10 | `Close` |

**`IAsyncOperation<T>`**（继承 `IAsyncInfo`）
COM 规则是**派生类方法在前**，所以它是 `IAsyncOperation` 自己的 3 个方法
先占 6/7/8，**然后**才是 `IAsyncInfo` 那五个：

| 槽 | 方法 |
|---|---|
| 6 | `put_Completed` |
| 7 | `get_Completed` |
| 8 | `GetResults` ← **不是 `[11]`** |
| 9 / 10 / 11 / 12 / 13 | `get_Id` / `get_Status` / `get_ErrorCode` / `Cancel` / `Close` |

**这就是继承最坑的地方**：同一份代码里，「`IAsyncInfo` 的 `get_Status` 在
`[7]`」和「`IAsyncOperation` 的 `get_Status` 在 `[10]`」都对。数错一格不会
报错，只会静默调到别的方法上。

**实践建议：拿到 `IAsyncOperation` 后先 `QueryInterface(IID_IAsyncInfo)`
拿独立的 vtable，用 `[7]/[8]` 轮询状态，再回原指针走 `[8] GetResults`。**
本项目 `src\autostart.rs` 就是这么做的——`activate()` 和 `request_enable()`
都先 QI 到 `IAsyncInfo`，彻底消掉继承歧义。

#### 我犯过的、比崩溃更危险的错

槽位错一格**不一定崩**，可能从只读变成写入：

| 方法 | 我原来写的槽 | 实际是什么 | 后果 |
|---|---|---|---|
| `get_state` | `task[6]` | **`RequestEnableAsync`** | 「查状态」会**弹系统框并真的启用开机自启** |
| `request_enable` | `task[8]` | `get_State` | 想启用却只是读了一下 |
| `disable` | `task[9]` | `get_TaskId` | 想关却读了个 ID |
| `activate` 的 `GetAsync` | `statics[6]` | `GetForCurrentPackageAsync` | taskId 被当 out 指针写坏，`rc=S_OK` 但 `op=0x0` |
| `get_Status` | `op[7]` | `get_Completed` | 往 4 字节栈槽写 8 字节 null handler，`status` **恒为 0**，看着像「永远 Started」 |

**教训：vtable 槽位错一格，语义可能从只读变成写入。**
所以 `src\autostart.rs` 里把全部槽位提成命名常量，并加了两条测试
（`vtable_槽位常量固定` / `vtable_槽位不重复`）锁死它们。后者尤其有用：
`IStartupTask` 的三个槽位一旦出现重复，就说明方法数数错了。

### 4.5 完整调用链

```
RoInitialize(1)
  → LoadLibraryW("combase.dll") + 6 个 GetProcAddress
  → WindowsCreateString("Windows.ApplicationModel.StartupTask")
  → RoGetActivationFactory(hs, IID_IActivationFactory)           → factory
  → factory[0] QueryInterface(IID_IStartupTaskStatics)          → statics
  → statics[7] GetAsync(hs_taskId)                              → IAsyncOperation
  → QI(IID_IAsyncInfo) 拿独立 vtable
  → 轮询 asyncInfo[7] get_Status 直到 != 0（0=Started），间隔 20ms，最多 5000ms
  → 若 status == 3 → asyncInfo[8] get_ErrorCode 取错误
  → 若 status == 1 → op[8] GetResults                           → IStartupTask
  → task[8] get_State → StartupTaskState
  → 逐层 Release（[2]）
```

**`StartupTaskState` 取值**：

| 值 | 含义 |
|---|---|
| 0 | Disabled（用户没启用） |
| 1 | DisabledByUser（用户明确拒绝过，**再请求会静默失败**） |
| 2 | Enabled |
| 3 | DisabledByPolicy（组策略禁用） |
| 4 | EnabledByPolicy |

**启用/禁用**：

- `task[6] RequestEnableAsync` → 会**弹系统确认框**，用户拒绝则返回 `DisabledByUser`
- `task[7] Disable` → 无需确认

### 4.6 实测：包外必然 `0x80070490`

即使 WinRT 初始化完全正确、工厂和 statics 都拿到了：

```
GetAsync([6]) = 0x80070490  op=0x0
  → ERROR_ELEMENT_NOT_FOUND
```

**原因**：`TaskId` 只在「有包身份 **且** `AppxManifest.xml` 里声明了
`<desktop:StartupTask TaskId="...">`」的环境中存在。绿色版两者都没有。

→ 所以分流判据只能是**包身份**。WinRT 可用性不能作为判据。

---

## 五、manifest 声明（MSIX 侧前提）

`D:\codex\rog-mouse-ctl\msix\AppxManifest.xml.template` 里已有：

```xml
<desktop:Extension Category="windows.startupTask"
                   Executable="rog-mouse-ctl.exe"
                   EntryPoint="Windows.FullTrustApplication">
  <desktop:StartupTask TaskId="RogBatteryAutoStart"
                       Enabled="false"
                       DisplayName="R Battery" />
</desktop:Extension>
```

- `TaskId` 必须和代码里 `GetAsync` 的参数**一字不差**（`RogBatteryAutoStart`）
- `Enabled="false"` 表示**默认不启用**，用户需在「设置 → 应用 → 启动」或程序内勾选开启
- 这是 `desktop`（`http://schemas.microsoft.com/appx/manifest/desktop/windows10`）扩展，
  要求 `runFullTrust` 能力（manifest 已有）

---

## 六、本机现状（2026-10-22 00:12 实测）

```
GetCurrentPackageFullName = 15700 → 包外

HKCU\...\Run 查询（7 个值，全与本工具无关）:
  Steam / GameViewer / AMDNoiseSuppression / Listary
  QQNT / QuarkCloudDriveUpdaterTaskUser1.0.0.11
  MicrosoftEdgeAutoLaunch_98769996E24836F99EC8617644423B4C

  RogBattery      → rc=2 (不存在)     ← 之前移除的，确实干净
  rog-mouse-ctl   → rc=2
  RogMouseCtl     → rc=2
  R Battery       → rc=2
```

→ 当前**没有任何自启条目**，两条路径都处于「未启用」的干净状态。

---

## 七、已实现并验证

`src/autostart.rs`（38,178 B / 950 行）已落盘并接进 `main.rs`。

### 公开 API

```rust
pub enum AutoStartState {
    Disabled, DisabledByUser, Enabled, DisabledByPolicy, EnabledByPolicy, Unknown,
}
pub enum AutoStartPath { WinRt, Registry, Unknown }

pub struct EnableOutcome { pub state: AutoStartState, pub enabled: bool, pub path: AutoStartPath }

pub fn is_packaged() -> bool;
pub fn package_full_name() -> Option<String>;
pub fn detect_path() -> AutoStartPath;
pub fn state() -> (AutoStartState, AutoStartPath);   // 永不 Err，读不到给 Unknown
pub fn enable()  -> Result<EnableOutcome, String>;
pub fn disable() -> Result<EnableOutcome, String>;
```

**为什么 `state()` 不返回 `Err`**：查状态失败不该阻止 GUI 渲染，
界面上显示「状态未知」即可。

**为什么不用 `bool`**：`DisabledByUser` 和 `DisabledByPolicy` 都是「没启用」，
但用户点「开启」时行为完全不同 —— 前者 `RequestEnableAsync` 会**静默失败**
（系统记着用户拒绝过，只能引导用户去「设置 → 应用 → 启动」手动开），
后者是组策略禁止、程序无解。合并成 `bool` 就没法给用户正确提示。

### 一个容易忽略的设计：不能只看键存在

`registry_state()` **会把 Run 键里记录的路径和当前进程的 exe 路径做比对**。
只判断「键存在」会出现这种坑：用户把 exe 从 A 挪到 B，注册表里还是 A，
界面显示「已启用」但开机什么也没发生。路径不一致时按 `Disabled` 处理，
`enable()` 会覆盖掉旧值。

### 真机往返验证（00:12:07 实测通过）

```
改动前状态：Disabled
enable  → state=Enabled enabled=true path=Registry
Run 键实际内容："D:\codex\rog-mouse-ctl\target\release\deps\rog_mouse_ctl-c37f8bdfdf86f8c0.exe" --autostart
disable → state=Disabled enabled=false
往返测试通过，已恢复原状
test autostart::tests::autostart_真机往返 ... ok   (1 passed; 48 filtered out)
```

测试断言的不只是 `enable()` 的返回值，还直接读回注册表核对了三件事：
① 命令行里有 exe 名；② 带 `--autostart` 参数；③ **路径被引号包起来**
（含空格的路径没有引号会被系统按空格切分参数）。

跑完复查：`HKCU\...\Run` 仍是 **7 个值，未增未减**，`RogMouseCtl` 无残留。
测试内部有 `Guard`（`impl Drop`）兜底，即使断言 panic 也会调 `disable()`，
不会把用户机器留在「自启已开」的状态。

### 测试清单（49 个全过）

| 测试 | 覆盖 |
|---|---|
| `state_describe_覆盖全部变体` | 6 个变体都有中文描述 |
| `is_enabled_只认两种` | `Enabled` / `EnabledByPolicy` |
| `winrt_状态码映射` | 0..4 与越界值 |
| `绿色版必然探测到注册表路径` | 测试进程无包身份 |
| `iid_常量与系统元数据一致` | 防手抄错（拼回字符串逐字符比对） |
| `包外查状态不panic` | 只读路径不崩 |
| `autostart_真机往返` | `#[ignore]`，手动 `--ignored` 触发 |

跑法：
```powershell
cargo test --release                                        # 48 个（跳过 ignored）
cargo test --release autostart_真机往返 -- --ignored --nocapture   # 真机往返
```

---

## 八、待实现清单

- [x] `src/autostart.rs`：`is_packaged()` / `state()` / `enable()` / `disable()`
- [x] 绿色版路径（注册表）—— **真机往返已验证**
- [ ] MSIX 路径（WinRT）—— 代码已写完但**未在真实 MSIX 包里验证 `GetAsync` 能成功**
      （包外实测必然返回 `0x80070490`，见 §4.6）
- [ ] GUI 设置页把「开机自动启动（还没实现）」按钮接上
- [x] 返回值区分「已启用 / 用户拒绝 / 策略禁止 / 出错」
- [ ] 接上 GUI 后**删掉 `autostart.rs` 顶部的 `#![allow(dead_code)]`**
      （现在整条链路没人调，编译器会报 50+ 条 never-used；
      接上后删掉这个 allow，让编译器重新盯真实死代码）
- [x] **`--autostart` 参数被 `main()` 识别**（见下一节，曾是个真 bug）

---

## 九、一个差点漏掉的 bug：注册表写对了，但自启仍然是坏的

**这是本项目最值得记的一次「局部正确、整体失效」。**

### 现象

`enable()` 的注册表往返测试全部通过，写入的值逐字符正确：
```
"H:\...\rog-mouse-ctl.exe" --autostart
```
但实测
```
rog-mouse-ctl.exe --autostart  →  41 ms 退出，打印用法帮助，0 个进程存活
```

### 根因

`main()` 的入口分流只有两条：
```rust
if args.is_empty() { FreeConsole(); return run_gui(); }   // 裸跑 → GUI
if wants_help      { print!("{USAGE}"); return OK; }      // --help → 帮助
```
`--autostart` 既不是空参数，也不是 `--help`，于是**掉进了默认的 CLI 分支**：
`pos` 里第一个是 `--autostart`，`cmd = "--autostart"` 匹配不到任何子命令，
打印一遍帮助就退出了。

**用户看到的现象**：勾了「开机自启」、重启电脑、托盘图标不出现、程序也没报错。
因为 41ms 就正常退出了（`exit=0`），没有任何错误可查。

### 为什么这个 bug 特别危险

- **注册表那一层的测试是绿的**。`autostart_真机往返()` 断言了「键存在」「值含 exe 路径」「值含 `--autostart`」「路径带引号」——四条全过，但仍然没用。
- **没有错误路径**。进程 `exit=0`，日志干净，事件查看器也没有记录。
- **只有端到端才暴露**：必须真的去执行「注册表里那条命令行」，才会发现它进不了 GUI。

### 修法

```rust
let wants_autostart = args.len() == 1 && args[0] == autostart::RUN_ARG;

if args.is_empty() || wants_autostart {
    unsafe { win32::FreeConsole(); }   // 开机时无控制台，不脱离会闪黑框
    return run_gui();
}
```

两个细节：
1. **`args.len() == 1`** 是刻意的。`--autostart` 只有在是**唯一参数**时才进 GUI。
   若写成 `args.contains(&RUN_ARG)`，那么 `info --autostart` 也会弹窗口，
   脚本里加个参数就莫名弹出 GUI 会很糟。
2. **`RUN_ARG` 必须是 `pub const` 且只有一份定义**。若在 `main.rs` 里重写一遍
   字面量 `"--autostart"`，两边一旦不同步就退回这个 bug，**且编译期不会报错**。

### 修复后实测（2026-10-22 00:41）

```
Start-Process exe -ArgumentList '--autostart'
  → 进程存活 3 秒以上（不再 41ms 退出）
  → cls=NativeWindowsGuiWindow vis=True title="ROG 龙鳞 Ace Mini"   ✅
  → 控制台窗口数 = 0  ✅
```

### 教训（通用）

**「写配置」和「读配置的执行路径」必须端到端测一遍。**
只测「配置写对了」是不够的 —— 写对了但读的人不认识，等于没写。
本项目里同类风险还有：`settings.ini` 的键名拼错（`settings.rs` 解析失败会
静默用默认值）、`manifest` 的 `TaskId` 与代码里的常量不一致
（`RogBatteryAutoStart` 出现两处，必须一致）。

---

## 十、踩坑清单

1. `libkernelbase.a` / `libcombase.a` 在 rustlib 里**不存在** → combase 必须动态加载
2. `RoInitialize` 不调 → `0x800401F0`，且这个错误码**不代表没有包身份**
3. 网上流传的 WinRT IID 计算算法**是错的** → 用系统元数据或运行时 `GetIids`
4. WinRT 接口继承 **IInspectable**，vtable 多 3 个槽 → 数错一格就 `0xC0000005`
5. `CoTaskMemFree` 在 `ole32` 不在 `kernel32`
6. `RegSetValueExW` 的 `cbData` 是字节数，REG_SZ 要含结尾 NUL
7. `rustc` 直接编译单文件要显式 `--edition 2021`，否则 `core::ffi::c_void` 找不到
