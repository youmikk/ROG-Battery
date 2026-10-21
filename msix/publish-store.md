# 打包 → 上传 Microsoft Store：三条途径与自动化脚本说明

对象：`R Battery`（MSIX / x64 / 仅 `Windows.Desktop`）
配套脚本：`msix\publish-store.ps1`

---

## 0. 先看结论

| 问题 | 答案 |
| --- | --- |
| 有没有官方的「脚本自动提交」途径？ | 有，但**只对 MSIX/APPX 走经典 Submission API**（`manage.devcenter.microsoft.com`） |
| 个人开发者账号能用吗？ | **能**。官方文档没有按账号类型设限，限制的是「有没有 Entra 租户 + 应用有没有 Manager 角色」 |
| 自动化能覆盖到哪一步？ | 从「改完代码」到「提交被 Store 接受」（状态进 `PreProcessing`）全自动 |
| 还差什么人工？ | 等认证结果；首次上架必须在网页先做一稿；商店素材/age ratings 得在网页填；`Manual` 发布模式下要点一下 Publish |
| 现在能跑吗？ | 本地能跑通到「缺凭据就停」，退出码 2。拿到 Entra 凭据后才能继续 |

---

## 1. 三条途径对比

| | **Submission API**（本脚本用的） | **msstore CLI** | **Partner Center 网页** |
| --- | --- | --- | --- |
| 官方名称 | Microsoft Store submission API | Microsoft Store Developer CLI | Partner Center 手动提交 |
| 适用于 | **MSIX / APPX**（本项目的格式） | MSIX/APPX，也覆盖 MSI/EXE 的部分流程 | 全部 |
| 能全自动吗 | 能，脚本直接发 HTTP | 能，CLI 封装了同一套 API | 不能，纯人工点 |
| 首次上架 | **不能**（API 不能创建 app） | **不能**（官方明说必须先网页发一次） | 能，且只能在这里做 |
| 认证方式 | Entra ID 应用 + client credentials | Entra ID 凭据（**不能用 MSA**） | 浏览器登录 |
| 前置成本 | 关联租户 + 注册 Entra 应用 + 分配 Manager 角色 | 同上（它也是走这套凭据），外加装 .NET 9 运行时 | 0 |
| 额外限制 | 不支持 mandatory app updates、Store 管理的 consumable 附加项、Pricing Version 2 | 更新操作**目前只支持免费产品** | 无 |
| 适合谁 | 想把发布塞进 CI 的 | 同上但不想自己写 HTTP | 首次上架、改商店页面 |

### 为什么本项目选 Submission API 而不是 msstore CLI

1. **CLI 的更新操作目前只支持免费产品**。R Battery 现在是免费的，但一旦以后想做付费或有内购，CLI 就卡住了，API 不会。
2. **CLI 有两处反直觉的坑**：`msstore publish` 在已有已发布提交时会**先删掉现有草稿**再重建，所以必须先 publish 再改 metadata；`--noCommit` 只是不提交，**不是 dry run** —— 它照样会真的创建草稿并上传包。
3. **CLI 在本机装不上/没验证过**：`winget search "Microsoft Store Developer CLI"` 能搜到（源 `msstore`，ID `9P53PC5S0PHJ`），但没有实际安装运行。脚本走 HTTP 是零依赖的，只要 PowerShell 就能跑。

> **关于 `winget install Microsoft.msstore`**：这个包名是错的，搜不到。正确写法是先装运行时再装 CLI：
> ```
> winget install Microsoft.DotNet.DesktopRuntime.9
> winget install "Microsoft Store Developer CLI"
> ```
> 或从 Microsoft Store 装：<https://www.microsoft.com/store/apps/9P53PC5S0PHJ>
> 文档：<https://learn.microsoft.com/en-us/windows/apps/publish/msstore-dev-cli/overview>

### 两套 API 别搞混

| | 经典 Submission API | 新版 Store submission API |
| --- | --- | --- |
| base URL | `https://manage.devcenter.microsoft.com/v1.0/my/` | `https://api.store.microsoft.com` |
| 适用包型 | **MSIX / APPX** ← 本项目 | **MSI / EXE** 安装包 |
| 认证 scope | `https://manage.devcenter.microsoft.com/.default` | `https://api.store.microsoft.com/.default` |
| 额外 header | 无 | `X-Seller-Account-Id: <SellerId>` |

本项目是 MSIX，**必须用左边那套**。右边那套对 MSIX 无效。

---

## 2. 个人开发者账号到底能不能用 Submission API

**结论：能。** 官方文档里没有任何「个人账号不可用」的条款。

### 官方原文

前置条件页（<https://learn.microsoft.com/en-us/windows/uwp/monetize/create-and-manage-submissions-using-windows-store-services>）：

> To use this API, you must meet the following prerequisites:
> - You must have a Partner Center account and your account must be associated with a Microsoft Entra ID tenant.
> - You must be the **Global administrator** of the Microsoft Entra ID tenant.
> - You must add a Microsoft Entra ID application to your Partner Center account and assign it the **Manager** role.

注意它只说了四件事：有 Partner Center 账号、有租户、你是租户的 Global admin、应用有 Manager 角色。**一个字都没提账号类型（Individual / Company）。**

租户可以免费建（<https://learn.microsoft.com/en-us/windows/apps/publish/partner-center/create-new-azure-ad-tenant>）：

> If you don't have an Microsoft Entra ID tenant, you can create a new Microsoft Entra ID tenant in Partner Center for no additional charge.

关联租户也不要求公司身份（<https://learn.microsoft.com/en-us/windows/apps/publish/partner-center/associate-existing-azure-ad-tenant-with-partner-center-account>）：

> Any user who has the **Manager** role for a Partner Center account can associate Microsoft Entra ID tenants with the account.

### 账号类型本身

`open-a-developer-account`（<https://learn.microsoft.com/en-us/windows/apps/publish/partner-center/open-a-developer-account>）里只有两种：**Individual** 和 **Company**，且新版 onboarding 两种都不收注册费：

> there are no registration fees for either account type

Individual 定位是「独立开发者、小规模创作者、业余爱好、学校、个人项目」——正好是本项目这种情况。
入口只有 <https://storedeveloper.microsoft.com>，走其他路径会掉进 legacy 流程。

⚠️ Individual → Company **不支持转换**。如果哪天要变成公司主体，只能新开一个 Company 账号，已发布的应用要重新走一遍。

### 真正的门槛（跟账号类型无关）

1. **必须在 Partner Center 手工发过一稿**。同一份文档的 prerequisites 明确要求 app 已经至少有过一次完整提交，且 age ratings 问卷已答完。API 不能创建 app，也不能代答问卷。
2. **必须在模板里给 Entra 应用分配 Manager 角色**。漏了这步，所有调用都是 403 —— 这是最常见的坑。
3. **用 API 建过提交之后就别在网页改**：官方明说在 Partner Center 编辑过的提交，API 再也 commit 不了，可能卡死到只能删掉重建。

---

## 3. 获取 Azure AD / Entra 凭据（一次性，分步）

### 第 1 步：确认你有一个 Entra ID 租户

打开 <https://partner.microsoft.com/dashboard> → 右上齿轮「Account settings」→「Organization profile」→「Tenants」。

- 已有租户 → 记下 **Tenant ID**（形如 `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`）
- 没有租户 → 同一页面点「Create new Microsoft Entra ID tenant」，**免费**
  （文档：<https://learn.microsoft.com/en-us/windows/apps/publish/partner-center/create-new-azure-ad-tenant>）

> 如果你有自己的公司租户，也可以「Associate」过来，但需要你是那个租户的 Global administrator。

### 第 2 步：在 Partner Center 注册 Entra 应用

「Account settings」→「User management」→「Microsoft Entra applications」→「Add Microsoft Entra application」

- 选「Create new Microsoft Entra application」
- 名字随便（例如 `rog-battery-publisher`）
- **★ 角色必须选「Manager」** ← 少了这一步，后面全是 403
- 提交后页面会显示 **Tenant ID** 和 **Client ID**（也叫 Application ID）

文档：<https://learn.microsoft.com/en-us/windows/apps/publish/partner-center/manage-azure-ad-applications-in-partner-center>

### 第 3 步：生成 Client Secret

点开刚建的应用 →「Add new key」→ 立刻抄下密钥值。

**★ 离开这个页面就再也看不到它了**，只能重新生成。有效期最长 2 年。

> 想省掉「抄密钥」这一步，可以在 Azure 门户（<https://portal.azure.com> → Microsoft Entra ID → App registrations → 你的应用 → Certificates & secrets）里上传证书，然后用证书代替 secret。本脚本只实现了 client secret，证书方式需要改 `Get-AadToken`，`msstore CLI` 原生支持（`--certificateThumbprint` / `--certificateFilePath`）。

### 第 4 步：找 Seller ID

Partner Center 右上角账号名 →「Account settings」→「Organization profile」→「Legal info」，或者直接看浏览器地址栏里 `accountid=` 后面那串。

### 第 5 步：写凭据文件

`D:\codex\rog-battery\msix\store-credentials.psd1`（**已在 .gitignore 里，不会进仓库**）：

```powershell
@{
    TenantId     = '你的租户 ID'
    ClientId     = '你的应用程序(客户端) ID'
    ClientSecret = '第 3 步抄下来的密钥'
    SellerId     = '你的 Seller ID'
    ProductId    = '9NRC8HCD4BZ7'
    AppId        = '9NRC8HCD4BZ7'
}
```

`ProductId` / `AppId` 都是 Store ID（本项目 `9NRC8HCD4BZ7`）。经典 Submission API 的 URL 里 `applications/{applicationId}` 用的就是它。

CI 里改用环境变量（密钥不进文件）：

| 环境变量 | 对应 |
| --- | --- |
| `STORE_TENANT_ID` | TenantId |
| `STORE_CLIENT_ID` | ClientId |
| `STORE_CLIENT_SECRET` | ClientSecret |
| `STORE_SELLER_ID` | SellerId |
| `STORE_PRODUCT_ID` | ProductId |
| `STORE_APP_ID` | AppId |

环境变量优先于文件，可以混用（文件补环境变量没给的那几个）。

### 第 6 步：验证凭据能用

```powershell
cd D:\codex\rog-battery\msix
powershell -NoProfile -ExecutionPolicy Bypass -File .\publish-store.ps1 -DryRun
```

看到 `ClientSecret = *** （长度 N，不显示）` 且没有「缺少凭据」告警，就说明读到了。

---

## 4. 脚本用法

```powershell
cd D:\codex\rog-battery\msix
```

### 参数

| 参数 | 作用 | 例子 |
| --- | --- | --- |
| 无参数 | 编译 + 校验 + 真上传 + commit + 轮询 | `.\publish-store.ps1` |
| `-Version <4段>` | 透传给 `build-msix.ps1 -Version`，同时也是 Identity 校验的期望值 | `-Version 1.0.1.0` |
| `-Package <路径>` | 跳过编译，直接用现成 .msix。**仍会做全部产物断言** | `-Package .\AppPackages\Sonder.RBattery_1.0.0.0_x64.msix` |
| `-WhatIf` | 只打印将要做的事，**不编译、不校验、不联网** | `.\publish-store.ps1 -WhatIf` |
| `-DryRun` | 真的编译 + 真的校验 + 读凭据，然后打印全部 HTTP 请求计划，**一个请求都不发** | `.\publish-store.ps1 -DryRun` |
| `-NoCommit` | 上传到草稿为止，不 commit。第一次真跑建议先用它，去网页看过再提交 | `.\publish-store.ps1 -NoCommit` |
| `-PollTimeoutMinutes <n>` | 轮询 commit 状态的超时，默认 15 分钟 | `-PollTimeoutMinutes 30` |
| `-PollIntervalSeconds <n>` | 轮询间隔，默认 10 秒 | `-PollIntervalSeconds 5` |
| `-CredentialsPath <路径>` | 指定凭据文件，默认同目录 `store-credentials.psd1` | `-CredentialsPath D:\secrets\store.psd1` |

### 日常发版（最常见）

```powershell
# 1. 改代码
.\..\build.cmd

# 2. 版本号 +1，打包并上传（这里会真的提交到 Store）
powershell -NoProfile -ExecutionPolicy Bypass -File .\publish-store.ps1 -Version 1.0.1.0
```

### 先看会做什么

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\publish-store.ps1 -WhatIf
```

### 本地全流程验证（不联网，推荐每次改脚本后跑）

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\publish-store.ps1 -DryRun
```

### 只传到草稿，人工检查后再提交

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\publish-store.ps1 -Version 1.0.1.0 -NoCommit
# 去 Partner Center 看过这一稿没问题，再手动点提交，或者去掉 -NoCommit 重跑
```

### 用现成包（不重新编译）

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\publish-store.ps1 `
  -Package .\AppPackages\Sonder.RBattery_1.0.0.0_x64.msix -DryRun
```

### 退出码

| 码 | 含义 |
| --- | --- |
| `0` | 成功（或 WhatIf/DryRun/NoCommit 正常结束） |
| `2` | 缺凭据 / AAD 认证失败 |
| `3` | 本地校验失败（exe 不存在、版本号非法、包签名了、Identity 不匹配……） |
| `4` | API 侧失败（建提交失败、上传失败、commit 失败、状态失败） |

CI 里可以据此判断是「配置问题」还是「代码/包问题」。

---

## 5. 脚本内部做了什么

| 步骤 | 动作 | 关键校验 |
| --- | --- | --- |
| 1 | 前置校验 | psd1 可读、版本号 4 段且首段 ≥1、exe 存在、makeappx 可用 |
| 2 | 调 `build-msix.ps1` | 透传 `-Version`，失败就带退出码停 |
| 3 | 校验产物 | `ZipFile::OpenRead` 列条目；**断言无 `AppxSignature.p7x`**；解析 `AppxManifest.xml` 断言 Identity 的 Name / Publisher / Version 与 psd1 **逐字相同（区分大小写）** |
| 4 | 读凭据 | 环境变量优先，其次 psd1；密钥只打长度 |
| 5 | 认证 | `POST {tokenHost}/{tenant}/oauth2/v2.0/token`，`grant_type=client_credentials`，`scope=https://manage.devcenter.microsoft.com/.default`；失败自动回退 v1 端点（`resource=`）并解析 `AADSTS*` 错误码 |
| 6 | 建提交 | `POST {api}/applications/{appId}/submissions`，**无请求体**，响应里取 `id` 和 `fileUploadUrl`（SAS） |
| 7 | 回写数据 | 把第 6 步响应原样 `PUT` 回去，只把 `applicationPackages` 换成一项 `{fileName, fileStatus=PendingUpload, minimumDirectXVersion=None, minimumSystemRam=None}`。**`ConvertTo-Json -Depth 100`**（官方明确警告默认 2 层会把提交数据截断成 `{}`） |
| 8 | 上传 | 把 .msix 打进 ZIP（ZIP 根下直接放，文件名与第 7 步 `fileName` 一致），`PUT` 到 SAS URL，header `x-ms-blob-type: BlockBlob` |
| 9 | 提交 | `POST .../submissions/{id}/commit`，**无请求体**，响应 `{"status":"CommitStarted"}` |
| 10 | 轮询 | `GET .../submissions/{id}/status`，离开 `CommitStarted` 就停；遇到失败态立刻摊开 `statusDetails.errors` |

### 两个容易踩的实现细节

**1. ZIP 不能放在源目录里。** `[System.IO.Compression.ZipFile]::CreateFromDirectory` 会枚举源目录，如果目标 ZIP 就在被枚举的目录里，它会把自己也打进去，而且边写边读，包会坏。脚本把 ZIP 放在 `AppPackages\` 下、暂存目录用 `.upload\`，两者分开。

**2. SAS URL 不能进日志。** `fileUploadUrl` 带 `?sv=...&sig=...`，等于一个限时可用的写权限令牌。脚本所有打印都走 `Hide-Secret`，把查询串换成 `?<redacted>`。

---

## 6. 常见失败排查表

### 本地阶段（退出码 3）

| 现象 | 原因 | 处理 |
| --- | --- | --- |
| `找不到 rog-battery.exe` | 没编译 | `cd D:\codex\rog-battery && build.cmd` |
| `版本号不合法：必须是 4 段数字` | 少写了段 | 写满四段，如 `1.0.1.0` |
| `第一段必须 >= 1` | 用了 `0.x.x.x` | Store 上架版本不能以 0 开头 |
| `每段必须在 0..65535 之间` | 段值超了 | MSIX 的 Identity/Version 是四段 UInt16 |
| `包里含 AppxSignature.p7x` | 用了 `build-msix.ps1 -Sign` 的产物 | 重新打包，**不带 `-Sign`**。提交 Store 的包必须未签名，Store 会用自己的证书重签 |
| `Identity/Name: 期望 X，实际 Y` | `package.props.psd1` 和 Partner Center 保留名不一致 | 改 psd1，别改脚本 |
| `找不到 makeappx.exe` | 路径变了 | `package.props.psd1` 的 `MakeAppxPath` 指对。**注意 `D:\codex\_tools\makeappx` 整个目录要保留**（靠同目录 `.manifest` 做免注册 COM 绑定，单拷 exe 会报 `side-by-side configuration is incorrect`） |

### 认证阶段（退出码 2）

| 现象 | 含义 | 处理 |
| --- | --- | --- |
| `AADSTS700016` | ClientId 在该租户里不存在 | 检查 ClientId 是否抄错，或应用建在别的租户下 |
| `AADSTS7000215` | ClientSecret 不对 / 已过期 / 已删除 | Partner Center 里重新 Add new key |
| `AADSTS90002` / `AADSTS900023` | 找不到租户 | TenantId 抄错，或租户还没和 Partner Center 关联 |
| `AADSTS65001` / `AADSTS70011` | 权限/scope 不被该应用接受 | 确认应用是通过 **Partner Center** 建的那一个，而不是在 Azure 门户自建的 |
| `AADSTS50034` | 用户/主体不存在 | 通常是用了 MSA 账号去建应用；这里要的是 Entra ID 应用 |

### API 阶段（退出码 4）

| HTTP | 含义 | 处理 |
| --- | --- | --- |
| **401** | token 无效/过期 | AccessToken 只有 **60 分钟**寿命；检查系统时间、secret 是否过期 |
| **403** | 认证过了但没权限 | **最常见：Entra 应用没拿到 Manager 角色**。Partner Center → 用户管理 → Microsoft Entra 应用 → 分配 Manager |
| **400 `InvalidPackageIdentityName`** | 包的 Identity/Name 与 Store 保留名不一致 | 对 Partner Center「产品标识」页逐字核对（区分大小写），改 `package.props.psd1` |
| **400 `InvalidPackageFamilyName`** | Name 或 Publisher 至少一个错，导致 PFN 不是已保留的那个 | 同上；本项目的 PFN 应为 `Sonder.RBattery_wm3rvpzgxb7kt` |
| **400 `InvalidPackagePublisherDisplayName`** | 发布者显示名不匹配 | 必须和 Partner Center 账户里的 PublisherDisplayName 完全一致 |
| **400 `InvalidArchive`** | ZIP 结构不对 | ZIP 根下必须直接是包文件，文件名与提交数据里 `fileName` 一致 |
| **404** | Store ID 错，或产品不在此账户下 | 本项目应为 `9NRC8HCD4BZ7`；去 Partner Center 产品页 URL 里核对 |
| **409** | 当前状态不允许该操作 | 产品用了 **mandatory app updates** 或 **Store 管理的 consumable 附加项**，这两类 API 明确不支持；也可能已有进行中的草稿，先 `DELETE` 再建 |
| **429** | 被限流 | 等几分钟，看响应头 `Retry-After` |
| **500 / 503** | 服务端问题 | 重试 |

### 提交状态失败（`statusDetails.errors[].code`）

| code | 含义 | 处理 |
| --- | --- | --- |
| `InvalidArchive` | 上传的 ZIP 不认识 | 检查 ZIP 里是不是只有一个完好的 .msix |
| `MissingFiles` | 提交数据引用的文件不在 ZIP 里 | 检查 `fileName` 和 ZIP 内文件名是否完全一致 |
| `PackageValidationFailed` | 包本身不合规 | 本地用 `makeappx unpack` + Windows App Cert Kit 先过一遍 |
| `InvalidParameterValue` | 某个字段值非法 | 看 `details` 里的具体字段 |
| `InvalidOperation` / `InvalidState` | 状态机不允许 | 常见于「网页改过之后又用 API commit」 |
| `ResourceNotFound` | 引用的资源不存在 | 可能是 listings 里的图片路径失效 |
| `ServiceError` | 服务端错误 | 重试 |
| `ListingOptOutWarning` / `ListingOptInWarning` | 市场上下架状态有变 | 警告级，确认是否符合预期 |
| `UpdateOnlyWarning` | 只是更新，不影响已发布版本 | 警告级 |
| `PackageValidationWarning` | 包校验有警告 | 想尽快上架要先消掉 |
| `Other` | 兜底 | 看 `details` |

### 状态机参考

```
None → PendingCommit → CommitStarted → PreProcessing → Certification → Release → Published
                            ↓                ↓              ↓            ↓
                      CommitFailed   PreProcessingFailed  CertificationFailed  ReleaseFailed
```

`Canceled` 表示被取消。脚本会在任何终态立刻停止轮询并打印原因，不等满超时。

---

## 7. 自动化到此为止 —— 剩下必须人工做的事

1. **等认证。** 脚本只覆盖到「提交被接受」。之后是微软的预检 → 认证（通常几小时到几天）→ 发布。结论只发邮件 / 显示在 Partner Center。
2. **处理认证反馈。** 认证报告在 Partner Center → 该产品 → 提交 → 认证报告。改完重跑 `-Version <更高版本号>`。**版本号必须比已发布的大**，否则 Store 直接拒收。
3. **发布（仅当 `targetPublishMode = Manual`）。** 这个值从上一稿复制而来。Manual 模式下认证通过后要有人点「Publish now」才真的上线。脚本没改这个值（避免误上线），想全自动就自己把提交数据里 `targetPublishMode` 改成 `Immediate`。
4. **商店页面素材（首次上架才需要）。** API 只复制上稿的 listings。截图（至少 1 张）、Store logo、分类、隐私政策 URL、age ratings 问卷都得在网页填。文案见 `store-listing.md`，隐私政策见 `privacy-policy.html`。
5. **首次上架的特殊性。** 如果这个产品从没在 Partner Center 手工提交过，脚本第 6 步就会失败 —— 那是设计如此：API 不能在网页之外创建 app，也不能跳过 age ratings 问卷。先把第一稿在网页发掉，之后每次更新都能用脚本。
6. **保管凭据。** `store-credentials.psd1` 已进 `.gitignore`。ClientSecret 最长 2 年，到期会有 401 提醒你。

---

## 8. 参考链接

- Submission API 总览（MSI/EXE 新版）：<https://learn.microsoft.com/en-us/windows/apps/publish/store-submission-api>
- 经典 submission API 前置条件与流程：<https://learn.microsoft.com/en-us/windows/uwp/monetize/create-and-manage-submissions-using-windows-store-services>
- 创建提交：<https://learn.microsoft.com/en-us/windows/uwp/monetize/create-an-app-submission>
- 管理提交（PUT 数据）：<https://learn.microsoft.com/en-us/windows/uwp/monetize/manage-app-submissions>
- commit：<https://learn.microsoft.com/en-us/windows/uwp/monetize/commit-an-app-submission>
- 创建 Entra 租户：<https://learn.microsoft.com/en-us/windows/apps/publish/partner-center/create-new-azure-ad-tenant>
- 关联已有租户：<https://learn.microsoft.com/en-us/windows/apps/publish/partner-center/associate-existing-azure-ad-tenant-with-partner-center-account>
- 管理 Entra 应用：<https://learn.microsoft.com/en-us/windows/apps/publish/partner-center/manage-azure-ad-applications-in-partner-center>
- 打开开发者账号：<https://learn.microsoft.com/en-us/windows/apps/publish/partner-center/open-a-developer-account>
- Store Developer CLI：<https://learn.microsoft.com/en-us/windows/apps/publish/msstore-dev-cli/overview>
- Store Developer CLI 命令：<https://learn.microsoft.com/en-us/windows/apps/publish/msstore-dev-cli/commands>
- 首次 MSIX 提交：<https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/create-app-submission>
- 开源替代实现 StoreBroker：<https://github.com/Microsoft/StoreBroker>（微软自己一线应用在用，功能比本脚本全，代价是要装模块并学它的配置文件格式）
