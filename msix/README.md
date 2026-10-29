# rog-mouse-ctl MSIX 打包

把 Rust 写的 `rog-mouse-ctl.exe` 打成微软商店可提交的 MSIX 包，同时保留免安装绿色版。
两种形态来自**同一份编译产物**，只有开机自启的实现方式不同。

```
D:\codex\rog-mouse-ctl\
├── Cargo.toml
├── src\                      ← 零依赖 Win32 手写 FFI，与打包无关
├── target\release\
│   └── rog-mouse-ctl.exe     ← 唯一的真·产物
└── msix\                     ← 本目录
    ├── AppxManifest.xml.template   带 @占位符@ 的清单模板
    ├── package.props.psd1          可变参数（身份/版本/工具路径）
    ├── build-msix.ps1              打包 + 可选签名 + 可选解包校验
    ├── Cargo.toml.metadata-snippet.txt  待粘到 Cargo.toml 的 metadata 片段
    ├── Assets\                     11 张 MSIX 图标（从 rog-battery 沿用）
    ├── stage\                      打包中间目录（脚本自动重建，可删）
    ├── AppPackages\                产物：Sonder.RBattery_<版本>_x64.msix
    └── scratch\unpack\             -Verify 的解包目录（可删）
```

---

## 一、绿色版（免安装）

```powershell
cd D:\codex\rog-mouse-ctl
cargo build --release
```

产物：`target\release\rog-mouse-ctl.exe`（当前约 **904 KB / 925,184 字节**，`lto + strip + panic=abort` 之后就这么大；GUI 改自绘 GDI+ 之后比早期的 478 KB 大了近一倍）。

直接把**这一个 exe** 拷走就能用 —— **不需要任何 DLL、不需要运行时、不需要安装程序**。

这一点相对 C# 版是硬优势：C# 版要么要求目标机装 .NET Framework，
要么得 self-contained 发布跟着几百 MB 的运行时目录走。这个 exe 是
Rust 静态链接产物，导入表里只有系统自带的 `KERNEL32.dll` / `hid.dll` /
`SETUPAPI.dll` / `ntdll.dll` / `msvcrt.dll`（`msvcrt.dll` 从 XP 起就是系统组件，
**不是** VC++ 可再发行包；二进制里没有 `vcruntime140` / `msvcp140` 依赖）。

所以验收标准就一句：**拷到任意一台 x64 Win10 1809+ 机器上双击能跑**。
不需要 ASUS 驱动、不需要管理员权限、不需要注册表写入之外的任何前置准备。
配置/缓存/备份全部落在用户目录，见下方差异表。

> PE 头现状：`machine = 0x8664 (x64)`，`subsystem = 2 (WINDOWS_GUI)`。
> 子系统是 GUI，**启动时不会创建控制台窗口**，所以双击、开始菜单、商店启动都没有黑框。
> 命令行能力由 `win32::attach_console()` 在 `main` 开头把父控制台接回来：
> `cmd /c "x info > r.txt"`、`x info | Out-File f.txt`、`Start-Process -RedirectStandardOutput`
> 三种重定向都实测正常。唯一例外是 PowerShell 自己的 `>` 操作符——它给 GUI 子系统
> 程序的句柄写不进去（实测 `out=2500` 有效但产出 0 字节），脚本里请改用上面三种。

> 别名做全局命令：把 exe 所在目录加进 `PATH`，或在 `%USERPROFILE%` 下放一个
> `rog.cmd` 转发。程序本身不需要任何同目录文件 —— 所以随便放哪都行，
> 这正是它能塞进只读的 `C:\Program Files\WindowsApps\` 的前提。

---

## 二、MSIX 版

```powershell
cd D:\codex\rog-mouse-ctl\msix

# 1) 先编译
cargo build --release --manifest-path ..\Cargo.toml

# 2) 打包（默认版本取 package.props.psd1 的 1.1.0.0）
powershell -NoProfile -ExecutionPolicy Bypass -File .\build-msix.ps1

# 3) 打包 + 解包自检（推荐，不碰系统状态）
powershell -NoProfile -ExecutionPolicy Bypass -File .\build-msix.ps1 -Verify

# 4) 换版本号
powershell -NoProfile -ExecutionPolicy Bypass -File .\build-msix.ps1 -Version 1.1.1.0 -Verify

# 5) 本地测试装包前签名（用已有上架证书，脚本不会自建证书）
powershell -NoProfile -ExecutionPolicy Bypass -File .\build-msix.ps1 -Sign -CertPath D:\secret\sonder.pfx -CertPassword '密码'
powershell -NoProfile -ExecutionPolicy Bypass -File .\build-msix.ps1 -Sign -Thumbprint <证书 SHA1 指纹>

# 6) 清干净
powershell -NoProfile -ExecutionPolicy Bypass -File .\build-msix.ps1 -Clean
```

产物：`AppPackages\Sonder.RBattery_<版本>_x64.msix`（1.1.0.0 于 GUI 自绘改造前实测 **410,268 字节** / 当时对应 817,152 字节的 exe。
当前 exe 已到 925,184 字节，**重新打包后包体必然更大，以实测为准** —— 不要拿旧数字对号）。

### 版本号必须严格递增（脚本会拦）

同一个 identity 下，Store **只接受严格大于**已发布版本的号；本地 `Add-AppxPackage`
遇到同版本/降级也会拒绝（`0x80073CF9`）。所以 `build-msix.ps1` 在打包**之前**会拿
`Get-AppxPackage -Name Sonder.RBattery` 的本机已装版本做比对，不满足就直接报错退出：

```
已装版本: 1.0.0.0   本次打包: 1.0.0.0
版本号撞车：本次打包的 1.0.0.0 等于 本机已安装的 1.0.0.0。
```

机器上没装这个包就跳过检查（换机器/新账号打包不该报错）。
确实要打一个不大于已装版本的号（纯看包结构），加 `-SkipInstalledVersionCheck`。

> 当前基线：商店里已上架的是 **C# 版 1.0.0.0**，所以 Rust 版从 **1.1.0.0** 起。
> 定 1.1.0.0 而不是 1.0.1.0 的理由：Rust 重写是功能大改（灯光/按键/性能全参数可写
> + 历史曲线），按语义化版本属于 minor 升级；1.0.1.0 会让人误以为只是修 bug。

### 脚本参数

| 参数 | 作用 |
|---|---|
| `-Version 1.1.1.0` | 覆盖 psd1 里的版本，必须是 4 段数字且首段 ≥ 1，且**必须大于本机已装版本** |
| `-Verify` | 打完后 `makeappx unpack` 解到 `scratch\unpack\`，逐项核对 manifest 与文件 |
| `-Sign` | 签名；**必须**配合 `-CertPath` 或 `-Thumbprint`，或证书库里已有 Subject 匹配的证书 |
| `-CertPath` / `-CertPassword` | `.pfx` 路径与密码 |
| `-Thumbprint` | 从 `Cert:\CurrentUser\My` 按 SHA1 指纹选证书 |
| `-TimestampUrl` | 可选，签名时加 RFC3161 时间戳（需联网） |
| `-SkipInstalledVersionCheck` | 跳过版本撞车检查。**打出来的包不要提交商店** |
| `-Clean` | 删 `stage\`、`AppPackages\`、`scratch\` |

### 签名是可选的，而且默认就该跳过

**本地不需要签名。** 提交 Store 时微软会用自己的证书重新签，所以：

* 日常开发 / 提交商店 → **不要带 `-Sign`**，产出的未签名包直接上传。
* 只有在你想 `Add-AppxPackage` 本地装一下看效果时，才需要签名。

没有可用证书时 `-Sign` 会**直接报错退出**，不会生成"半个签名包"：
`makeappx pack` 出的未签名包已经完整落盘，签名失败不影响它 ——
但脚本会以非零码退出，让你注意到"我本来想签名但没签成"。

脚本**刻意不自动创建自签名证书**（C# 版会自动建）。原因：用户已有正式上架证书，
自动建只会往证书库里塞垃圾，而且自签证书要装进「受信任的根证书颁发机构」
才能装包，装完还得手动清，风险和心智负担都大于收益。

需要签名的两条路：

```powershell
# A. 用你已有的上架证书（.pfx）
-Sign -CertPath D:\secret\sonder.pfx -CertPassword '密码'

# B. 证书已装进 Cert:\CurrentUser\My，按指纹选
-Sign -Thumbprint <SHA1 指纹>
# 或者 Subject 与 manifest Publisher 完全一致时，直接 -Sign 就能匹配上
```

### 提交商店前

1. 跑**不带 `-Sign`** 的命令 —— 商店会用自己的证书重签，未签名包才是要上传的。
2. 版本号必须比商店里已发布的**大**。
3. `Publisher` 必须和 Partner Center「产品标识」页的值逐字一致（现在已对齐）。

---

## 三、双形态差异清单

**只有两处不同**，其余全部一致 —— 这是 MSIX 桌面桥（`runFullTrust`）不虚拟化文件系统的结果。

| 项 | 绿色版 | MSIX |
|---|---|---|
| 配置 | `%APPDATA%\rog-mouse-ctl\settings.ini` | **同**（真实路径，不重定向） |
| 缓存 | `%LOCALAPPDATA%\rog-mouse-ctl\snapshot.cache` | **同** |
| 备份 | `Documents\rog-mouse-ctl\backups\` | **同** |
| exe 位置 | 随便放（含只读目录） | `C:\Program Files\WindowsApps\Sonder.RBattery_<版本>_x64__<hash>\` |
| **开机自启** | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` | **WinRT `startupTask`**（manifest 声明 + `RequestEnableAsync`） |
| **分发方式** | 拷 exe / 解压即用 | 微软商店（或 `.msix` + `Add-AppxPackage`） |
| 更新 | 手动换 exe | 商店自动 / `AppInstaller` 自动 |
| 卸载 | 删文件 | 「应用和功能」/ `Remove-AppxPackage` |
| 代码签名 | 无（SmartScreen 靠口碑积累） | 商店代签，装包不触发 SmartScreen |
| 权限 | 用户身份 | 用户身份（`runFullTrust`，**不是** AppContainer） |
| HID 访问 | 标准 Win32 `CreateFile` on `\\?\hid#vid_0b05...` | **同**，无需额外 capability |
| 单实例/身份 | 无包身份 | 有 PFN，可与其他应用共存不同版本 |

> 注意：绿色版和 MSIX 版**共用同一份 AppData 配置**。同时装两个不会打架，
> 但改了一个的配置另一个也会读到。这是刻意的 —— 迁移时用户设置自动带过去。

---

## 四、踩过的坑（都是真机验证过的）

### 1. `runFullTrust` **不**虚拟化文件系统

`%APPDATA%`、`%LOCALAPPDATA%`、`Documents` 全部落到真实路径，**不会**被重定向到
`%LOCALAPPDATA%\Packages\<PFN>\LocalCache\...`。所以程序**不需要**改任何配置路径，
`src\paths.rs` 里那套 `env_dir("APPDATA")` / `env_dir("LOCALAPPDATA")` 在两种形态下
走的是完全相同的分支。

（会虚拟化的只有 UWP/AppContainer 应用。桌面桥进程跑在用户身份下，看到的文件系统
和 explorer.exe 一样。）

### 2. 注册表虚拟化是**真的**

写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 会**静默成功**，但被重定向进
包私有的 Helium hive：

```
%LOCALAPPDATA%\Packages\Sonder.RBattery_<publisherhash>\SystemAppData\Helium\User.dat
```

现象极具迷惑性：`RegSetValue` 返回 `ERROR_SUCCESS`，读回来也是对的，
但「任务管理器 → 启动应用」和「设置 → 启动」里什么都没有，重启后当然不会自启。

→ **开机自启必须用 `startupTask` 扩展，不能用 Run 键。**

### 3. manifest 的硬性要求

- `<rescap:Capability Name="runFullTrust" />` 必须有。缺了 exe 根本起不来。
- `<Application EntryPoint="Windows.FullTrustApplication">` 必须有。缺了进程被塞进
  AppContainer，`CreateFile` 打不开 HID 设备（`ERROR_ACCESS_DENIED`）。
- `<desktop:Extension Category="windows.startupTask" Executable="...">` 的
  `Executable` **必须**和 `<Application Executable>` 逐字一致，否则 `makeappx` 报
  `The extension executable ... does not match`。本模板用 `@EXECUTABLE@` 占位符
  同时填两处，就是为了防这个。

### 4. `WindowsApps` 目录**只读**

`C:\Program Files\WindowsApps\<包名>\` 是受 ACL 保护的只读目录，**且带包完整性校验**。
程序不能往 exe 同目录写任何东西（日志、配置、缓存、临时文件都不行）。

程序侧已经按这个前提设计：所有可变数据都在 `%APPDATA%` / `%LOCALAPPDATA%` /
`Documents`。以后加新功能时也别往 exe 旁边写文件 —— 绿色版下"能写"会掩盖这个问题，
直到装上 MSIX 才炸。

### 5. `startupTask` 的 `Enabled="false"` 是**规定值**，不是可选

manifest 里必须写 `Enabled="false"`（微软的规定，装机默认关）。用户首次启用要靠程序
主动调：

```rust
// 伪代码，实际是 WinRT: Windows.ApplicationModel.StartupTask
let task = StartupTask::GetAsync("RogBatteryAutoStart").get()?;
match task.State {
    Disabled      => { task.RequestEnableAsync().get()?; }  // 会弹一次用户确认
    EnabledByPolicy => { /* 组策略禁用，没法绕 */ }
    _ => {}
}
```

`RequestEnableAsync` **会弹一个系统确认框**（"是否允许此应用在登录时启动"）。用户点否
之后就一直是 `Disabled`，程序不该反复骚扰 —— 只在他显式打开开关时再调一次。

绿色版则退回 `HKCU\...\Run`。两条路径的开关状态**不互通**（一个在注册表，一个在包
状态里），这是预期行为。

### 6. PowerShell 5.1 的两个坑（脚本已处理）

- **没有 `Join-String`**。用 `-join` 运算符或 `[string]::Join`。
- **`Get-Content -Raw` 会按 ANSI（本机 GBK）解码 UTF-8 文件**，中文全毁。
  读带中文的模板/XML 一律 `[System.IO.File]::ReadAllBytes` + `[System.Text.Encoding]::UTF8.GetString`。
  注意 `GetString` 会把 BOM 解成一个 `U+FEFF` 字符带进 XML，导致
  `Data at the root level is invalid` —— 脚本里 `Read-Utf8NoBom` 会 `TrimStart(0xFEFF)`。
- 脚本自身必须存成 **UTF-8 带 BOM**，否则 5.1 按 GBK 读源码，中文注释变乱码，
  乱码字节可能吞掉后续行，表现为莫名其妙的语法错误。

### 7. `makeappx` 不能单文件拷

它靠**同目录**的 `.manifest` 做免注册 COM 绑定。只拷 `makeappx.exe` 会报
`side-by-side configuration is incorrect`。本机整套在 `D:\codex\_tools\makeappx\`。

---

## 五、验证方式（不碰系统状态）

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\build-msix.ps1 -Verify
```

会 `makeappx unpack` 到 `scratch\unpack\`，逐项核对：

- `Identity/@Name` = `Sonder.RBattery`
- `Identity/@Publisher` = `CN=1307ADA3-...`
- `Identity/@Version` / `@ProcessorArchitecture` = 目标版本 / `x64`
- `Application/@Executable` = `rog-mouse-ctl.exe`，`@EntryPoint` = `Windows.FullTrustApplication`
- `startupTask` 扩展存在、`Executable` 一致、`TaskId` = `RogBatteryAutoStart`、`Enabled` = `false`
- `runFullTrust` capability 存在，且**能力总数 = 1**（多声明会被抓出来）
- exe 存在且字节数和源文件一致
- 六张被 manifest 点名的 Assets 都在
- 包根没有多余文件（防止误把源码/README 打进去）

**为什么不用 `Add-AppxPackage -Register stage\AppxManifest.xml` 验证能装**：
开发模式注册会**占用 identity**，可能顶掉用户已上架的商店版，且留下难以清理的
开发包注册项。解包校验能覆盖 manifest 的全部结构性错误，风险为零。

---

## 六、改东西时的对照表

| 想改 | 改哪 |
|---|---|
| 版本号 | `package.props.psd1` 的 `Version`，或 `-Version` 参数 |
| 身份 / Publisher | `package.props.psd1`（改完必须和证书 Subject 一致） |
| 显示名 / 描述 | `package.props.psd1` 的 `DisplayName` / `ShortName` / `Description` |
| 可执行文件名 | `package.props.psd1` 的 `Executable` **和** `Cargo.toml` 的 `[[bin]] name` |
| 开机自启 TaskId | `package.props.psd1` 的 `StartupTaskId` **和** `src\` 里的常量（两边必须一致） |
| 图标 | 替换 `Assets\*.png`（尺寸/命名别变，manifest 按名字引用） |
| makeappx 路径 | `package.props.psd1` 的 `MakeAppxPath` / `FallbackMakeAppxPath` |

`build-msix.ps1` **只读 `package.props.psd1`**，不读 `Cargo.toml`。所以 `package.props.psd1`
是身份的唯一真源，`Cargo.toml` 里的 `[package.metadata.msix]` 只是给
`cargo metadata` / 未来 build.rs 用的副本，改身份时两处都要动。
---

## 七、上架交付件（2026-10-29 重出）

```powershell
cd D:\codex\rog-mouse-ctl\msix
$env:TEMP="$PWD\scratch\tmp"                       # 本机 makeappx 解包需要可写 TEMP
powershell -NoProfile -ExecutionPolicy Bypass -File .\build-msix.ps1 -Verify
powershell -NoProfile -ExecutionPolicy Bypass -File .\make-upload.ps1
```

产物（均在 `AppPackages\`）：

| 文件 | 大小 | 用途 |
|---|---|---|
| `Sonder.RBattery_1.1.0.0_x64.msix` | 689,121 B | 未签名包；`Add-AppxPackage` 前需自签 |
| `Sonder.RBattery_1.1.0.0_x64.msixupload` | 686,947 B | **提交 Partner Center 就传这个** |

`.msixupload` 由 `make-upload.ps1` 生成，本质是 ZIP，**里面只有包本体** `<包名>.msix`。

> 早前版本还往里塞了 `Assets\StoreLogo.png` 等三张商店图标，被 Partner Center 的包接受验证直接打回：
> `The file StoreLogo.png is not supported. Remove or update this file and then try again.`（三张全报）。
> 按文档那三张图只在「包内没有」时才需要放进 .msixupload；我们的 manifest 已经声明了它们、图就在 msix 里，
> 所以多余的那份必须删掉。万一反过来提示缺图标，用 `make-upload.ps1 -WithStoreLogos` 重出 ——
> 那种情况下三张图要放 ZIP **根目录**，不是 `Assets\` 子目录。

### 商店页素材在 `store\`

包只管「装什么」，列表页管「怎么卖」，两套东西分开：

| 位置 | 内容 |
|---|---|
| `store\listingData.csv` | 直接导入 Partner Center 的列表表（430 行，字段顺序与图片 URL 全部沿用原导出表） |
| `store\screenshots\01..06*.png` | 1920×1080 桌面截图 |
| `store\make-listing.py` | 生成上面的 CSV；改文案改这里 |
| `store\make-screenshots.py` | 由 `scratch\` 的界面渲染图合成截图 |
| `store\README.md` | 上传步骤、文案事实口径、两条踩过的坑 |

### 图标尺寸合规（这次修的坑）

`gen_icon.py` 早前把 256px 的图直接覆盖到了三个槽位，尺寸不合规会被认证打回：

| 文件 | 要求 | 修复前 |
|---|---|---|
| `StoreLogo.png` | 50×50 | 256×256 ✗ |
| `Square44x44Logo.png` | 44×44 | 256×256 ✗ |
| `Square150x150Logo.png` | 150×150 | 256×256 ✗ |

修复脚本 `scratch\fix_msix_assets.py`：从 1024 母图 LANCZOS 缩放（≤48px 加一次 UnsharpMask），非正方形（`Wide310x150`、`SplashScreen 620x300`）按最小边缩放后居中贴透明画布。

### 提交前的两个已知项（不阻塞，但建议处理）

1. ~~exe 仍是 `subsystem=3 (WINDOWS_CUI)`~~ —— **已修**：子系统改为 `windows`（`subsystem=2`），启动不再创建控制台，黑框消失；命令行由 `win32::attach_console()` 接回父控制台。见第一节末尾的说明。
2. **exe 没有 VERSIONINFO 资源** —— `FileVersion` 为空字符串。商店不校验，但「应用和功能」里看不到版本号。MSIX 的版本来自 manifest，所以不影响升级。
3. `Cargo.toml` 的 `version = "1.1.0"` 与 MSIX 的 `1.1.x.0` 不必逐字一致 —— 前者是 crate 版本，后者是商店包版本，只是别差太多让人困惑。

## 八、GitHub Actions 自动发版

两个工作流，各管一件事，互不依赖：

| 文件 | 管什么 | 产物 |
| --- | --- | --- |
| `.github/workflows/msstore.yml` | MSIX 打包（+ 按需提交商店） | run artifacts：`.msix` / `.msixupload` |
| `.github/workflows/release.yml` | 绿色版发版 | GitHub Release：`rog-mouse-ctl.exe` + `.sha256` |

拆开的原因：商店包和绿色版是两条发布线，失败面完全不同（商店那边要账号凭据、要过审，
绿色版只要 tag）。合在一个工作流里，商店凭据出问题会把绿色版发版一起拖住。

### 版本号规则（2026-10-29 统一）

一共三个地方出现版本号，以前各写各的，结果 GitHub 发到 `v1.1.2` 了、软件「关于」里
还写着 `1.1.0`。现在的规则：

| 位置 | 谁决定 | 当前值 |
| --- | --- | --- |
| 软件里显示（「关于」卡 / 侧栏底部 / `rog-mouse-ctl version`） | `build.rs` 烧进 `ROG_APP_VERSION`：平时取 `Cargo.toml` 的 `version`，**CI 发版时用 tag 覆盖** | `1.2.0` |
| GitHub Release / tag | 手动 `git tag v1.2.0` | `v1.2.0` |
| MSIX / 商店（四段号） | `msix/package.props.psd1` 的 `Version`，CI 用「tag + `.0`」覆盖 | `1.2.0.0` |

- `release.yml` 里有一条断言：编完立刻跑 `rog-mouse-ctl.exe version`，和 tag 版本不一致就
  `throw` —— 版本号再也不可能悄悄漂掉。
- 商店侧必须**严格递增**：已提交过 `1.1.9.0`（CI 测试跑上去的），所以下一个包至少
  `1.2.0.0`；这就是这次从 `1.1.x` 直接跳到 `1.2.0` 的原因。
- 发版流程：改 `Cargo.toml` 的 `version` 和 `package.props.psd1` 的 `Version` → 提交 →
  `git tag v1.2.0 && git push origin v1.2.0`。两个工作流会同时起，绿色版进 Release、
  MSIX 进商店。

### 触发方式

| 触发 | `msstore.yml` | `release.yml` |
| --- | --- | --- |
| `git push origin v1.1.2`（tag） | 打 `.msix` + `.msixupload`；**配了产品 ID 才**提交商店 | 编译绿色版 → 建 / 更新 `v1.1.2` 的 Release |
| 手动 `workflow_dispatch` | 同上，可手填 MSIX 版本号 | 填了 `tag` 就发到那个 tag；留空只编译留 artifact |
| push 到 `main` | 不触发 | 不触发 |

### `release.yml` 的两道保险

1. **PE 子系统必须是 `2 (WINDOWS_GUI)`** —— 读 PE 头 `0x18 + 0x44` 校验，不是 2 直接 `throw`。
   防的就是「黑框回归」：谁把 `windows_subsystem` 改回 console，CI 立刻红。
2. **已有的 Release 只覆盖附件** —— 先 `gh release view`，存在就 `gh release upload --clobber`，
   不存在才 `gh release create --generate-notes`。这样手写的更新日志不会被自动生成的内容冲掉。

### 为什么必须用 gnu 工具链

`vendor/mingw-import-libs/` 里是 **GNU 格式**的导入库（`libdwmapi.a`、`libgdiplus.a` 等），
MSVC 目标链不上。所以 runner 上要装 `stable-x86_64-pc-windows-gnu` + MSYS2 的 mingw64 gcc（两个工作流里都已经做好），
产物路径与本地完全一致：`target/release/rog-mouse-ctl.exe`。

### 商店自动发布是「按需开启」的

微软规定**首个版本必须手动上架**，之后才能走 API。所以 `msstore.yml` 用 `vars.MSSTORE_PRODUCT_ID` 门控：
没配这个变量时只打包，完全不碰商店；首个版本过审后配上它，之后推 tag 就会自动提交商店。
绿色版不受这个变量影响，任何时候推 tag 都会发 Release。

### 商店提交走 REST API，不用 `msstore publish`

`msstore publish` 要一个「能被识别的项目目录」，并从项目里读出发布者身份（`Publisher=CN=...`）。
我们没有 `.csproj`；往目录里塞一份 `Package.appxmanifest` 会让它当成 **UWP 项目**去
`MSBuild restore`，在装了 Visual Studio 的 runner 上直接抛：

```
Unhandled exception: MSStore.API.MSStoreException: Failed to restore packages.
   at MSStore.CLI.ProjectConfigurators.UWPProjectConfigurator.RestorePackagesAsync
```

所以改成直接调 **Store 提交 API**，见 `tools/msstore-publish.ps1`：

```
取令牌 → 删掉旧草稿（只删 PendingCommit）→ POST 建草稿 → PUT 改 fileName
→ PUT 传包体到 fileUploadUrl → POST commit
```

三个只有踩过才知道的细节（脚本里都注了）：

1. `POST /submissions` **不能带请求体** —— 带 `{}` 会报
   `InvalidParameterValue: The size of Listings must be 1 or more`。
2. 但 `Content-Type` **必须是** `application/json`，否则报 `Only JSON content is accepted`。
3. 删除旧提交前必须先判 `status -eq 'PendingCommit'`，**绝不能**删已发布或认证中的提交。

本机想手动提交，直接跑：

```powershell
$env:AZURE_AD_TENANT_ID = '<租户 ID>'
$env:AZURE_AD_APPLICATION_CLIENT_ID = '<客户端 ID>'
$env:AZURE_AD_APPLICATION_SECRET = '<客户端密码值>'
powershell -File tools\msstore-publish.ps1 -ProductId 9NRC8HCD4BZ7 -PackagePath msix\AppPackages\<包>.msixupload
```

加 `-DryRun` 只建草稿、不点提交，可以先去 Partner Center 核对。

### 要配的 Secrets

Settings → Secrets and variables → Actions → **Secrets**

> 逐步操作（每一步点哪里、容易拿错的地方、常见报错对照）见 **[docs/store-publishing-setup.md](../docs/store-publishing-setup.md)**。
> 仓库里还有一键写入脚本：`powershell -NoProfile -ExecutionPolicy Bypass -File tools\setup-store-secrets.ps1`（先加 `-DryRun` 看一眼）。

| Name | 取值位置 |
| --- | --- |
| `AZURE_AD_TENANT_ID` | Entra ID → 应用注册 → 概览 → 目录（租户）ID |
| `AZURE_AD_APPLICATION_CLIENT_ID` | 同上 → 应用程序（客户端）ID |
| `AZURE_AD_APPLICATION_SECRET` | 同上 → 证书和密码 → 新建客户端密码（值只显示一次） |
| `SELLER_ID` | Partner Center → 设置 → 帐户设置 → 标识符 → 发布者 ID |

前置条件：Partner Center 关联 Entra ID，并在 Entra ID 里注册应用、授予 Partner Center 的 Manager 角色。

### 要配的 Variables

同一页面 → **Variables**

| Name | 说明 |
| --- | --- |
| `MSSTORE_PRODUCT_ID` | 商店产品 ID（本项目是 `9NRC8HCD4BZ7`）。**不配 = 不自动发布** |
| `MSSTORE_PACKAGE_IDENTITY_NAME` | 可选，覆盖 `package.props.psd1` 的 `IdentityName` |
| `MSSTORE_PUBLISHER` | 可选，覆盖 `Publisher` |
| `MSSTORE_PUBLISHER_DISPLAY_NAME` | 可选，覆盖 `PublisherDisplayName` |

身份三项不配也行 —— `build-msix.ps1` 会退回 `package.props.psd1` 里的值；配了则以 Variables 为准
（脚本用 `[Environment]::GetEnvironmentVariable` 读，本地不设就完全不生效）。

### 版本号从哪来

tag `v1.1.2` → `1.1.2.0`；手动触发可填；两者都没有就用 `Cargo.toml` 的版本补一个 `.0`。
工作流会当场拦「不是四段数字」和「第一段为 0」。

### 产物去哪

- **GitHub Release**：`rog-mouse-ctl.exe`（绿色版单文件）
- **Run artifacts**：`msix-<版本>`，内含 `.msix` / `.msixupload` / exe
- `.msix` 是未签名的，只用于上传商店和本地测试装包，**不放进 Release**

### 本地等价命令（CI 跑的就是这几条）

```powershell
cargo build --release
powershell -File msix\build-msix.ps1 -Version 1.1.2.0 -Verify -SkipInstalledVersionCheck
powershell -File msix\make-upload.ps1 -Version 1.1.2.0
# 提交商店（要三个环境变量，见上一节）
powershell -File tools\msstore-publish.ps1 -ProductId 9NRC8HCD4BZ7 -PackagePath msix\AppPackages\<包>.msixupload
```

> `-Verify` 在**本机**要先把 `$env:TEMP` 指到仓库内 —— 本机沙箱不允许写 `%TEMP%`，
> `makeappx unpack` 会报 `0x80070005`。CI 上不需要这个处理。
