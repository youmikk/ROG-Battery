# 商店自动发布：凭据配置手册

目标：让 `.github/workflows/msstore.yml` 能替你把 `.msixupload` 提交到 Microsoft Store。

需要 **4 个值**，全部来自两个地方：Entra 管理中心（3 个）+ 合作伙伴中心（1 个）。

| GitHub 名字 | 是什么 | 从哪拿 | 容易拿错的地方 |
| --- | --- | --- | --- |
| `AZURE_AD_TENANT_ID` | 目录（租户）ID | Entra 管理中心 → 标识 → 概述 → **租户 ID** | 别拿成「对象 ID」或订阅 ID |
| `AZURE_AD_APPLICATION_CLIENT_ID` | 应用程序（客户端）ID | Entra → 标识 → 应用程序 → 应用注册 → 选中应用 → 概览 → **应用程序(客户端) ID** | 别拿成「对象 ID」 |
| `AZURE_AD_APPLICATION_SECRET` | 客户端密码**值** | 同一应用 → 证书和密码 → 新建客户端密码 → 复制 **值** | 值是密码正文，不是「密码 ID」；**离开页面就再也看不到** |
| `SELLER_ID` | 卖家 / 发布者 ID | 合作伙伴中心 → 设置 → 帐户设置 → **标识符** → 发布者 ID | 本项目这个值是 `1307ADA3-FCC0-4C75-81A2-9CC947003BD7`（就是 `AppxManifest` 里 `Publisher="CN=..."` 的那串） |

> 官方口径见 [使用 GitHub Actions 将应用更新发布到 Microsoft Store](https://learn.microsoft.com/zh-cn/windows/apps/publish/msstore-dev-cli/github-actions) 的「先决条件」一节，以及 [Microsoft Store 开发者 CLI 命令](https://learn.microsoft.com/zh-cn/windows/apps/publish/msstore-dev-cli/commands)。

---

## 链接一览（全部可直接点）

| 用途 | 链接 |
| --- | --- |
| 合作伙伴中心（总入口） | <https://partner.microsoft.com/dashboard> |
| 合作伙伴中心 → 帐户设置 → 标识符（拿卖家 ID） | <https://partner.microsoft.com/dashboard/account/v3/accountsettings/identifiers> |
| 合作伙伴中心 → 用户管理 → Microsoft Entra 应用程序（授 Manager 角色） | <https://partner.microsoft.com/dashboard/account/v3/usermanagement/entraapps> |
| 合作伙伴中心 → 本产品概览 | <https://partner.microsoft.com/dashboard/products/9NRC8HCD4BZ7/overview> |
| Entra 管理中心（总入口） | <https://entra.microsoft.com> |
| Entra → 应用注册列表 | <https://entra.microsoft.com/#view/Microsoft_AAD_IAM/ActiveDirectoryMenuBlade/~/RegisteredApps> |
| Azure 门户 → 应用注册（同一个东西的后备入口） | <https://portal.azure.com/#view/Microsoft_AAD_IAM/ActiveDirectoryMenuBlade/~/RegisteredApps> |
| GitHub 仓库 Secrets | <https://github.com/youmikk/ROG-Battery/settings/secrets/actions> |
| GitHub 仓库 Actions 运行记录 | <https://github.com/youmikk/ROG-Battery/actions> |
| 商店商品页（用来确认版本上线） | <https://apps.microsoft.com/detail/9NRC8HCD4BZ7> |

> 微软后台改版比较勤，带 `#view=...` 或 `/v3/...` 的直达链接打不开时，就从上面的「总入口」按菜单路径点进去，路径写在每一步里。

官方文档：

- [使用 GitHub Actions 将应用更新发布到 Microsoft Store](https://learn.microsoft.com/zh-cn/windows/apps/publish/msstore-dev-cli/github-actions)
- [Microsoft Store 开发者 CLI 命令（MSIX）](https://learn.microsoft.com/zh-cn/windows/apps/publish/msstore-dev-cli/commands)
- [在 Microsoft Entra ID 中注册应用程序](https://learn.microsoft.com/zh-cn/entra/identity-platform/quickstart-register-app)
- [在合作伙伴中心添加 Microsoft Entra 应用程序（服务主体）](https://learn.microsoft.com/zh-cn/partner-center/service-principal)
- [关联现有 Microsoft Entra ID 租户](https://learn.microsoft.com/zh-cn/windows/apps/publish/partner-center/associate-existing-azure-ad-tenant-with-partner-center-account)
- [创建新的 Microsoft Entra ID 租户](https://learn.microsoft.com/zh-cn/windows/apps/publish/partner-center/create-new-azure-ad-tenant)
- [为应用添加凭据（客户端密码）](https://learn.microsoft.com/zh-cn/entra/identity-platform/how-to-add-credentials)


---

## 第 0 步：确认商城账号已经关联 Entra 租户

微软要求「合作伙伴中心帐户必须有关联的租户」，否则第 3 步找不到入口。

1. 打开 <https://partner.microsoft.com/dashboard>，右上角齿轮 → **帐户设置**。
2. 左侧找 **组织 → Microsoft Entra ID**（旧版叫「租户」/「Azure AD」）。
3. 如果显示「已关联」并给出一个域名（如 `xxx.onmicrosoft.com`）→ 跳过本步。
4. 如果没关联 → 点「关联」，按 [关联现有 Microsoft Entra ID](https://learn.microsoft.com/zh-cn/windows/apps/publish/partner-center/associate-existing-azure-ad-tenant-with-partner-center-account) 或 [创建新的 Entra ID 租户](https://learn.microsoft.com/zh-cn/windows/apps/publish/partner-center/create-new-azure-ad-tenant) 操作。个人开发者账号用默认目录就行。

---

## 第 1 步：在 Entra 注册一个应用

> 这是「给 CI 用的机器人账号」，不是给你登录用的。

1. 打开 <https://entra.microsoft.com>（或 <https://portal.azure.com> → Microsoft Entra ID）。
2. **标识 → 应用程序 → 应用注册 → 新注册**。
3. 名称随便，建议 `github-msstore-publish`。
4. 受支持的帐户类型：**仅此组织目录中的帐户（单租户）**。
5. 重定向 URI **留空**（CI 走客户端密码，不需要交互登录）。
6. 注册后停在「概述」页，抄两个值：
   - **应用程序(客户端) ID** → `AZURE_AD_APPLICATION_CLIENT_ID`
   - **目录(租户) ID** → `AZURE_AD_TENANT_ID`

## 第 2 步：建客户端密码

1. 同一应用的左侧 **证书和密码** → **客户端密码** → **新客户端密码**。
2. 说明随便（`github-actions`），有效期按需（最长 24 个月；到期要重建并更新 secret）。
3. 点「添加」后，**立刻复制「值」那一列**（不是「密码 ID」）。
   → `AZURE_AD_APPLICATION_SECRET`

> 忘记复制只能删掉重建。仓库里永远不要写这个值。

## 第 3 步：把这个应用加进合作伙伴中心，并授「管理者」角色

不授角色的话，CLI 能登录但提交会 401/403。

1. 合作伙伴中心 → 齿轮 **设置** → **用户管理** → 选项卡 **Microsoft Entra 应用程序**。
2. **添加 Microsoft Entra 应用程序** → 选「现有」→ 选中第 1 步注册的 `github-msstore-publish`。
3. 角色选 **管理者（Manager）**（与旧版 CPP 的 Owner 等价；「开发者」权限不够提交）。
4. 保存。

官方口径：[在合作伙伴中心添加 Microsoft Entra 应用程序（服务主体）](https://learn.microsoft.com/zh-cn/partner-center/service-principal)。

## 第 4 步：取卖家 ID

1. 合作伙伴中心 → **设置** → **帐户设置** → **标识符**。
2. 抄 **发布者 ID / 卖家 ID**。本项目是 `1307ADA3-FCC0-4C75-81A2-9CC947003BD7`
   （与 `msix/package.props.psd1` 的 `Publisher` 去掉 `CN=` 前缀完全一致）。

---

## 第 5 步：写进 GitHub

### 命令行（推荐）

```powershell
gh secret set AZURE_AD_TENANT_ID              --body "<租户 ID>"
gh secret set AZURE_AD_APPLICATION_CLIENT_ID  --body "<客户端 ID>"
gh secret set AZURE_AD_APPLICATION_SECRET     --body "<密码值>"
gh secret set SELLER_ID                       --body "1307ADA3-FCC0-4C75-81A2-9CC947003BD7"

# 首个版本手动过审之后，再打开自动提交：
gh variable set MSSTORE_PRODUCT_ID --body "9NRC8HCD4BZ7"

gh secret list
gh variable list
```

仓库里带了一个交互脚本，等价于上面几条：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\setup-store-secrets.ps1 -DryRun   # 先看会执行什么
powershell -NoProfile -ExecutionPolicy Bypass -File tools\setup-store-secrets.ps1
```

### 网页

仓库 → **Settings** → **Secrets and variables** → **Actions** → 分别用 **Secrets** / **Variables** 两个标签页新建。

### 别忘了 Variables

| 名字 | 值 | 作用 |
| --- | --- | --- |
| `MSSTORE_PRODUCT_ID` | `9NRC8HCD4BZ7` | **不配 = 只打包不提交**（首个版本过审前就该是这个状态） |
| `MSSTORE_PACKAGE_IDENTITY_NAME` | 可选，覆盖 `package.props.psd1` 的 `IdentityName` |
| `MSSTORE_PUBLISHER` | 可选，覆盖 `Publisher` |
| `MSSTORE_PUBLISHER_DISPLAY_NAME` | 可选，覆盖 `PublisherDisplayName` |

---

## 第 6 步：本地先验一遍（不用等 CI）

```powershell
winget install "Microsoft Store Developer CLI"      # 需要 .NET 9 桌面运行时
msstore reconfigure --tenantId <租户> --sellerId <卖家> --clientId <客户端> --clientSecret <密码>
msstore info                                        # 能看到配置就说明凭据语法没错
msstore apps list                                   # 能列出 9NRC8HCD4BZ7 说明角色授对了
```

最后一条是关键：`apps list` 返回空/403 → 第 3 步的角色没生效（等几分钟再试，Entra 有传播延迟）。

想验证整条上传链路又不真的发布：

```powershell
msstore publish msix\AppPackages\Sonder.RBattery_x.y.z.0_x64.msixupload -id 9NRC8HCD4BZ7 --noCommit
msstore submission status 9NRC8HCD4BZ7     # 应为草稿
msstore submission delete 9NRC8HCD4BZ7 --no-confirm
```

> `--noCommit` 会**上传**包并改草稿，不是纯演练；验完记得删草稿，否则下次 `publish` 会覆盖它（官方文档明确说 `publish` 会丢弃待处理草稿里的元数据改动）。

---

## 第 7 步：首个版本手动上架（只做一次）

微软规定**第一个版本必须人工在合作伙伴中心提交**，API 只能用于后续更新。所以顺序是：

1. 打开 <https://partner.microsoft.com/dashboard/products/9NRC8HCD4BZ7/overview>。
2. 左侧 **提交 → 程序包**，上传 `msix\AppPackages\Sonder.RBattery_x.y.z.0_x64.msixupload`（本项目用 `msix\make-upload.ps1` 生成）。
3. **商店列表**：把 `msix\store\listingData.csv` 的内容填进各语言页面，截图用 `msix\store\screenshots\01..06.png`。
4. **属性 / 年龄分级 / 定价**：工具类、免费。
5. **提交以供认证**，等审核通过、商品页上线。
6. 上线之后，再回到 GitHub 配 `MSSTORE_PRODUCT_ID` 变量 —— 从这一刻起推 tag 才会自动提交更新。

> 没有这一步就配 `MSSTORE_PRODUCT_ID`，CI 会在 `msstore publish` 处报错（产品还没上架，无法创建提交）。

---

## 常见报错对照

| 报错 | 原因 | 处理 |
| --- | --- | --- |
| `AADSTS700016: Application with identifier ... was not found in the directory` | 客户端 ID 和租户 ID 不属于同一个目录 | 确认应用注册在**关联到伙伴中心的那个**租户里 |
| `AADSTS7000215: Invalid client secret provided` | 填了「密码 ID」或密码已过期 | 重建客户端密码，复制**值** |
| `AADSTS65001: The user or administrator has not consented` | 应用没被授角色 / 没同意 | 回到第 3 步授「管理者」 |
| `401 Unauthorized` / `403 Forbidden`（提交时） | 角色是「开发者」或尚未传播 | 改 Manager，等 5–15 分钟 |
| `The submission ... package version must be greater` | 版本号没递增 | `-Version` 传比商店现有更高的四段号 |
| `Package acceptance validation error: The file StoreLogo.png is not supported` | `.msixupload` 里多塞了商店图标 | 本项目已修：`make-upload.ps1` 默认只打 `.msix` |

---

## 安全

- 客户端密码等同发布权限，只放 GitHub Secrets，不进仓库、不进日志。
- 有效期到了会突然全部失败；到期前重建密码 → `gh secret set` 覆盖。
- 想彻底不用密码：`msstore reconfigure` 支持证书方式（`-ct/--certificateThumbprint`、`-cfp/--certificateFilePath`），适合自托管 runner，公共 runner 上不推荐。
- 建议只给这个应用 Partner Center 的「管理者」，不要用你自己日常账号的凭据。

---

## 自动提交的两个前置条件（踩过的坑）

自动提交（GitHub Actions 或本机 publish.ps1）走的是 **Ingestion API**，比网页多两条硬限制。

### 1. 必须有一个「项目」用来读发布者身份

msstore publish 的第一个位置参数 <pathOrUrl> 是**项目目录**，它从里面读 Publisher（CN=1307ADA3-...）。不传就默认当前目录，于是报：

\We could not find a project publisher for the project at '<目录>'
\
我们没有 \.csproj\，解决办法是把**包内真正的 \AppxManifest.xml\** 取出来，另存为同一目录下的 \Package.appxmanifest\，并把该目录作为第一个参数传进去：

\\powershell
msstore publish <放包的目录> -i <放包的目录> -id 9NRC8HCD4BZ7
\
> **这条路已废弃（2026-10-29）**：塞 `Package.appxmanifest` 会让 `msstore publish` 把目录
> 当成 UWP 项目去 `MSBuild restore`，在装了 Visual Studio 的 runner 上直接崩。
> 现在 CI 与本机都改为直接调 Store 提交 API，见 `tools/msstore-publish.ps1` 与
> `msix/README.md` 第八节「商店提交走 REST API」。

### 2. 商店里不能留着「网页上手工创建」的待提交草稿

API 只能改、删、提交**由 API 创建**的草稿。若 Partner Center 里存在一个手工创建的 \PendingCommit\ 草稿，会报：

\Ingestion API can only update, delete, and commit submissions that are
created through the API. Please delete the current in-progress submission
and create one using the API
\
处理办法（一次性）：打开
<https://partner.microsoft.com/dashboard/products/9NRC8HCD4BZ7/submissions>
→ 删除那个待提交草稿 → 重跑工作流。之后草稿都由 API 创建，不会再出现。

> 已上线版本不受影响：删掉的只是草稿。

> 附带一条：\msstore submission delete\ 默认要交互确认，非交互环境要加 \--no-confirm\。

---

## 附：实际跑通的方式（2026-10-29）

`msstore publish` 走不通（见上），最终改用 Store 提交 API：

```
tools/msstore-publish.ps1
  取令牌 → 删旧草稿（只删 PendingCommit）→ POST 建草稿 → PUT 改 fileName
  → PUT 传包体 → POST commit
```

CI 里就是一步：`.github/workflows/msstore.yml` 的「上传并提交到商店」，
凭据从 Secrets 读，产品 ID 从 `vars.MSSTORE_PRODUCT_ID` 读。

首次跑通记录：run `40854169786`（版本 1.1.9.0），提交 id `1152921505702265885`，
状态 `CommitStarted`，包 `Sonder.RBattery_1.1.9.0_x64.msixupload`。

## 附二：换包为什么一直不生效（2026-10-30 定位）

现象：CI 全绿、日志里 `已提交的包：Sonder.RBattery_1.2.0.0_x64.msixupload version=1.2.0.0`，
但商店里版本一直停在 `1.1.2.0`。

根因有三条，叠在一起：

1. **`applicationPackages` 的对象形状不对。**
   官方[管理应用提交](https://learn.microsoft.com/zh-cn/windows/uwp/monetize/manage-app-submissions)
   写明：调用「更新应用提交」时，这个对象**只需**提供
   `fileName`、`fileStatus`、`minimumDirectXVersion`、`minimumSystemRam`，
   其余字段（`id`、`version`、`architecture`、`languages`、`capabilities`、
   `targetDeviceFamilies`）由合作伙伴中心填充。之前把克隆来的整份槽原样 PUT 回去，
   带 `fileStatus=PendingUpload` 就被 400 拒。
2. **`fileName` 指的是上传 ZIP 里的 `.msix` 名，不是 `.msixupload` 名。**
   `.msixupload` 本身就是「内含 `.msix` 的 ZIP」，所以把它整份 PUT 到 SAS 地址正好，
   但 `fileName` 必须写 `Sonder.RBattery_X.Y.Z.0_x64.msix`。
3. **`minimumDirectXVersion` / `minimumSystemRam` 的枚举值不能写 `None`。**
   它们分别是 `DirectXVersion`（`NotSet`/`DirectX9`…`DirectX12`）与
   `MemoryRAM`（`NotSet`/`RAM_300MB`…），要写 `NotSet`。

之前的「只改 fileName」之所以看着成功：PUT 返回 200，但商店把 `fileStatus=Uploaded`
当成已定稿，`commit` 之后又把槽换回旧包 —— 于是又一次把 1.1.2.0 发出去。

现在的做法（`tools/msstore-publish.ps1`）：按文档形状从完整到精简试四种
（`fileName + PendingUpload + NotSet×2` → `+ PendingUpload` → `+ NotSet×2` → 仅 `fileName`），
取第一个商店接受 `fileName` 的；上传后再核对 `fileName` 与 `version` 都等于目标值，
否则**直接失败并拒绝 commit**，不再静默把旧包发出去。

另外：商店同一时间只允许一个在途提交。上一次提交还在
`CommitStarted`/`Certification`/`Release` 时，脚本会打 warning 并跳过（`exit 0`），
MSIX 包照常作为 artifact 上传，等前一个走完再重跑即可。

## 附三：换包的最终可用做法（2026-10-30 跑通）

按 400 正文逐条试出来的结论，pplicationPackages 必须是**两条**：

| 条目 | id | version | fileName | fileStatus |
| --- | --- | --- | --- | --- |
| 旧包 | 原样保留 | 原样保留 | 原样保留 | PendingDelete |
| 新包 | **删掉** | 留空（提交后由 ingestion 回填） | Sonder.RBattery_X.Y.Z.0_x64.msix | PendingUpload |

其他要点：

1. 已存在的包槽**不能**改回 PendingUpload（会报
   Please keep all file entries for existing packages ... missing in your update），
   只能标 PendingDelete 再新增一条。
2. minimumDirectXVersion / minimumSystemRam 写 NotSet 会被拒
   （'NotSet' is not a valid 'DirectXVersion' value），正确值是 None，最好原样沿用。
3. ileName 是上传 ZIP 里的 .msix 名；.msixupload 本身就是内含该 .msix 的 ZIP，
   整份 PUT 到 ileUploadUrl（blob 级 SAS）即可。
4. **ingestion 解析包体发生在 commit 之后**：上传完成后 ersion 仍是空的、
   ileStatus 仍是 PendingUpload，这是正常的；commit 后 1～2 分钟才会回填
   ersion 并转 Uploaded。所以脚本先按包名放行，commit 后轮询 5 分钟核对版本。
5. 提交后状态依次是 CommitStarted → Certification → Published
   （TargetPublishMode=Immediate 时认证通过即自动上线）。

跑通记录：run 41074616586，提交 id 1152921505702278422，
包 Sonder.RBattery_1.2.0.0_x64.msix / Version 1.2.0.0，随后进入 Certification。
