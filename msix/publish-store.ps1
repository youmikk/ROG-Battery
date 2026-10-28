# 打包 → 上传 Microsoft Store（MSIX / Microsoft Store submission API）
#
# 用法（在本目录下）:
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\publish-store.ps1 -WhatIf
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\publish-store.ps1 -DryRun
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\publish-store.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\publish-store.ps1 -Version 1.0.1.0
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\publish-store.ps1 -Package .\AppPackages\Sonder.RBattery_1.0.0.0_x64.msix
#
# 【三种模式的区别，别搞混】
#   -WhatIf  只打印「将要做什么」，不编译、不校验、不联网。用于确认参数和流程。
#   -DryRun  真的走完本地全流程（编译 + 解开 msix 逐项断言 + 读凭据），
#            然后把 5~10 步的 HTTP 请求计划原样打印出来，一个字节都不发。
#            这是「能跑通到上传前的最后一步」的验证方式。
#   都不带   真跑：编译 → 校验 → 换 token → 建提交 → 回写数据 → 传包 → commit → 轮询。
#
# 【为什么这个脚本不能「从零建 app」】
#   Store submission API 只能给**已存在**的 app 建提交，而且要求该 app 在
#   Partner Center 里**已经有过一次完整提交**（含 age ratings 问卷）。所以第一次
#   上架必须在网页上手工做完，之后这个脚本才有意义。脚本结尾会把这个前提再打一遍。
#
# 【凭据从哪来】
#   优先环境变量（CI 用），其次同目录 store-credentials.psd1（本机用，已进 .gitignore）。
#   键名：TenantId / ClientId / ClientSecret / SellerId / ProductId / AppId
#   一个都不能硬编码进脚本 —— 这是能提交到公开仓库的前提。

[CmdletBinding()]
param(
    # 4 段版本号，透传给 build-msix.ps1 -Version。省略则用 package.props.psd1 里的值。
    [string] $Version,

    # 跳过编译，直接用现成的 .msix。仍然会做全部产物断言。
    [string] $Package,

    # 只打印计划，什么都不做。
    [switch] $WhatIf,

    # 本地全部做完，网络请求只打印不发送。
    [switch] $DryRun,

    # 传到草稿为止，不 commit。第一次真跑建议先用它，去 Partner Center 看过再手动提交。
    [switch] $NoCommit,

    # 轮询 commit 状态的超时（分钟）。
    [int] $PollTimeoutMinutes = 15,

    # 轮询间隔（秒）。
    [int] $PollIntervalSeconds = 10,

    # 显式指定凭据文件，默认同目录 store-credentials.psd1。
    [string] $CredentialsPath
)

$ErrorActionPreference = 'Stop'

# ---------------------------------------------------------------------------
# 常量：官方端点。改这里之前先看 publish-store.md 里的链接。
# ---------------------------------------------------------------------------
$ApiBase          = 'https://manage.devcenter.microsoft.com/v1.0/my'
$TokenHost        = 'https://login.microsoftonline.com'
$ApiScope         = 'https://manage.devcenter.microsoft.com/.default'
$ApiResource      = 'https://manage.devcenter.microsoft.com'
$DefaultProductId = '9NRC8HCD4BZ7'

$root        = $PSScriptRoot
$repo        = Split-Path $root -Parent
$pkgDir      = Join-Path $root 'AppPackages'
$uploadDir   = Join-Path $pkgDir '.upload'
$propsPath   = Join-Path $root 'package.props.psd1'
$exePath     = Join-Path $repo 'rog-battery.exe'
$buildScript = Join-Path $root 'build-msix.ps1'

$script:ExitCode   = 1
$script:Submission = $null
$script:IntentionalStop = $false
# 这三个在 finally 里要用，先初始化。$msixLeaf 必须在 DryRun 打印计划之前就有值，
# 否则 [8/10] 那段计划会打出 "AppPackages\.zip" 这种缺文件名的空壳路径。
$msixPath = $null
$msixLeaf = $null
$zipPath  = $null

# ---------------------------------------------------------------------------
# 输出小工具
# ---------------------------------------------------------------------------
function Write-Step {
    param([string] $Number, [string] $Text)
    Write-Host ''
    Write-Host ("=== [{0}] {1}" -f $Number, $Text) -ForegroundColor Cyan
}

function Write-Ok {
    param([string] $Text)
    Write-Host ("    OK   {0}" -f $Text) -ForegroundColor Green
}

function Write-Note {
    param([string] $Text)
    Write-Host ("         {0}" -f $Text) -ForegroundColor Gray
}

function Write-Plan {
    param([string] $Text)
    Write-Host ("    ->   {0}" -f $Text) -ForegroundColor Yellow
}

function Write-Bad {
    param([string] $Text)
    Write-Host ("    !!   {0}" -f $Text) -ForegroundColor Red
}

function Write-WarnLine {
    param([string] $Text)
    Write-Host ("    ??   {0}" -f $Text) -ForegroundColor DarkYellow
}

# 把 URL 里的签名/查询串抹掉再打印，避免把 SAS token 写进日志或 CI 输出。
function Hide-Secret {
    param([string] $Uri)
    if (-not $Uri) { return $Uri }
    $q = $Uri.IndexOf('?')
    if ($q -ge 0) { return $Uri.Substring(0, $q) + '?<redacted>' }
    return $Uri
}

function Stop-WithCode {
    param([int] $Code, [string] $Message)
    $script:ExitCode = $Code
    # 标记「这是脚本主动停的」，catch 里就不会为预期内的停止刷一屏调用栈。
    # 真正的意外异常不会走这里，堆栈仍然照打。
    $script:IntentionalStop = $true
    throw $Message
}

# ---------------------------------------------------------------------------
# HTTP 错误码 → 中文可执行建议
# 官方文档只给 400/404/409 三行，实际踩坑远不止这三种，这里补全。
# ---------------------------------------------------------------------------
function Show-HttpHint {
    param([int] $StatusCode, [string] $Body)

    switch ($StatusCode) {
        400 {
            Write-Bad '400 Bad Request：请求体/参数不合法。'
            if ($Body -match 'InvalidPackageIdentityName') {
                Write-Note 'InvalidPackageIdentityName = 包的 Identity/Name 和 Partner Center 里这个产品的保留名不一致。'
                Write-Note '去 Partner Center → 产品 → 产品标识（Product identity）页对一遍 Name 和 Publisher，'
                Write-Note '然后改 package.props.psd1（不要改脚本）。'
            }
            elseif ($Body -match 'InvalidPackageFamilyName') {
                Write-Note 'InvalidPackageFamilyName = Identity/Name 或 Publisher 至少一个不对，导致算出来的 PFN 不是已保留的那个。'
            }
            elseif ($Body -match 'InvalidPackagePublisherDisplayName') {
                Write-Note 'PublisherDisplayName 必须和 Partner Center 账户里的发布者显示名完全一致。'
            }
            elseif ($Body -match 'InvalidArchive|InvalidZip') {
                Write-Note '上传的 ZIP 结构不对：ZIP 根下必须直接是包文件，文件名要和提交数据里 fileName 一致。'
            }
        }
        401 {
            Write-Bad '401 Unauthorized：token 无效或过期（AccessToken 只有 60 分钟寿命）。'
            Write-Note '检查：client_secret 是否已过期（Partner Center 里 key 有效期最长 2 年）、'
            Write-Note '      系统时间是否准确、tenant 是否是该 ClientId 所属的租户。'
        }
        403 {
            Write-Bad '403 Forbidden：认证过了但没权限。'
            Write-Note '最常见原因：这个 Entra 应用在 Partner Center 里没有得到 Manager 角色。'
            Write-Note 'Partner Center → 账户设置 → 用户管理 → Microsoft Entra 应用 → 选中该应用 → 分配 Manager。'
        }
        404 {
            Write-Bad '404 Not Found：applicationId（Store ID）写错，或这个产品在该账户下不存在。'
            Write-Note "本项目 Store ID 应为 $DefaultProductId；去 Partner Center 产品页的 URL 里核对。"
        }
        409 {
            Write-Bad '409 Conflict：当前状态不允许这个操作。'
            Write-Note '典型：这个产品用了 mandatory app updates 或 Store 管理的 consumable 附加项，'
            Write-Note '      或者产品在 Pricing Version 2 上 —— 这两种情况 API 明确不支持，只能回网页操作。'
            Write-Note '也可能：已有一个进行中的草稿提交，先 DELETE 掉再新建。'
        }
        429 {
            Write-Bad '429 Too Many Requests：被限流。等几分钟再来（看响应头 Retry-After）。'
        }
        500 { Write-Bad '500 服务端错误。官方建议：重试。' }
        503 { Write-Bad '503 服务不可用。稍后重试。' }
        default {
            Write-Bad ("HTTP {0}：未覆盖的错误码，看上面的响应体。" -f $StatusCode)
        }
    }
}

# ---------------------------------------------------------------------------
# 通用：带完整错误输出的 REST 调用
# 绝不静默吞异常 —— 每一层都打印真实消息、HTTP 状态码和服务端响应体。
# ---------------------------------------------------------------------------
function Invoke-StoreApi {
    param(
        [Parameter(Mandatory = $true)][string] $Method,
        [Parameter(Mandatory = $true)][string] $Uri,
        [string] $AccessToken,
        $Body,
        [int] $TimeoutSec = 120
    )

    $headers = @{ Accept = 'application/json' }
    if ($AccessToken) { $headers['Authorization'] = "Bearer $AccessToken" }

    $payload = $null
    if ($null -ne $Body) {
        if ($Body -is [string]) { $payload = $Body }
        else {
            # -Depth 必须给大值：官方 troubleshooting 明确说默认 2 层会把提交数据截断成 {}。
            $payload = $Body | ConvertTo-Json -Depth 100 -Compress
        }
        $headers['Content-Type'] = 'application/json; charset=utf-8'
    }

    Write-Host ("         {0} {1}" -f $Method, (Hide-Secret $Uri)) -ForegroundColor DarkGray

    $params = @{
        Method         = $Method
        Uri            = $Uri
        Headers        = $headers
        UseBasicParsing = $true
        TimeoutSec     = $TimeoutSec
    }
    if ($null -ne $payload) {
        # 按 UTF-8 字节发，避免中文在 5.1 里被当成 Latin-1 发出去。
        $params['Body'] = [System.Text.Encoding]::UTF8.GetBytes($payload)
    }

    try {
        $resp = Invoke-WebRequest @params
    }
    catch {
        $ex = $_.Exception
        Write-Bad ("{0} {1} 失败" -f $Method, (Hide-Secret $Uri))
        Write-Bad ("异常: {0}" -f $ex.Message)

        $status = 0
        $errBody = $null
        if ($ex.PSObject.Properties['Response'] -and $ex.Response) {
            try { $status = [int] $ex.Response.StatusCode } catch { $status = 0 }
            try { Write-Bad ("HTTP {0} {1}" -f $status, $ex.Response.StatusDescription) } catch { }
            try {
                $stream = $ex.Response.GetResponseStream()
                if ($stream) {
                    $reader = New-Object System.IO.StreamReader($stream)
                    try { $errBody = $reader.ReadToEnd() } finally { $reader.Dispose() }
                }
            } catch { }
        }
        # 5.1 里服务端响应体常常只出现在 ErrorDetails 上
        if (-not $errBody -and $_.ErrorDetails -and $_.ErrorDetails.Message) {
            $errBody = $_.ErrorDetails.Message
        }
        if ($errBody) {
            Write-Bad '服务端响应体:'
            Write-Host $errBody -ForegroundColor DarkRed
        }
        if ($status -gt 0) { Show-HttpHint -StatusCode $status -Body ([string] $errBody) }
        throw
    }

    $content = $resp.Content
    if ($null -eq $content -or $content -eq '') { return $null }
    if ($content -is [byte[]]) { $content = [System.Text.Encoding]::UTF8.GetString($content) }
    try {
        return $content | ConvertFrom-Json
    }
    catch {
        Write-WarnLine '响应不是 JSON，原样返回文本。'
        return $content
    }
}

# ---------------------------------------------------------------------------
# MSIX 产物检查
# 提交 Store 的三条硬要求，缺一条都会被 Store 打回：
#   1) 必须**未签名**（AppxSignature.p7x 存在就是签名了，Store 会拒绝）
#   2) AppxManifest.xml 必须在包根
#   3) Identity 的 Name / Publisher / Version 必须和 Partner Center 完全一致
# ---------------------------------------------------------------------------
function Get-MsixInspection {
    param([Parameter(Mandatory = $true)][string] $Path)

    $problems = New-Object System.Collections.Generic.List[string]
    $result = [ordered]@{
        Path        = $Path
        Exists      = $false
        SizeBytes   = 0
        EntryCount  = 0
        EntryNames  = @()
        HasSignature = $false
        IdentityName = $null
        IdentityPublisher = $null
        IdentityVersion = $null
        Problems    = $problems
    }

    if (-not (Test-Path -LiteralPath $Path)) {
        $problems.Add("文件不存在: $Path")
        return [pscustomobject] $result
    }
    $result.Exists = $true
    $result.SizeBytes = (Get-Item -LiteralPath $Path).Length
    if ($result.SizeBytes -eq 0) { $problems.Add('文件大小为 0') }

    $zip = $null
    try {
        Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction Stop
        $zip = [System.IO.Compression.ZipFile]::OpenRead($Path)

        $names = New-Object System.Collections.Generic.List[string]
        foreach ($entry in $zip.Entries) {
            $names.Add($entry.FullName)
            if ($entry.FullName -match '(^|/)AppxSignature\.p7x$') {
                $result.HasSignature = $true
            }
        }
        $result.EntryNames = $names.ToArray()
        $result.EntryCount = $names.Count

        foreach ($required in @('AppxManifest.xml', 'AppxBlockMap.xml', '[Content_Types].xml')) {
            if ($names -notcontains $required) {
                $problems.Add("缺少 $required —— 不是 makeappx 产出的合法 MSIX")
            }
        }

        $manifestEntry = $zip.GetEntry('AppxManifest.xml')
        if ($manifestEntry) {
            $reader = New-Object System.IO.StreamReader($manifestEntry.Open(), [System.Text.Encoding]::UTF8)
            try { $manifestText = $reader.ReadToEnd() } finally { $reader.Dispose() }

            $doc = New-Object System.Xml.XmlDocument
            $doc.LoadXml($manifestText)
            $identityNode = $doc.SelectSingleNode("/*[local-name()='Package']/*[local-name()='Identity']")
            if (-not $identityNode) {
                $problems.Add("AppxManifest.xml 里找不到 <Identity> 节点")
            }
            else {
                $result.IdentityName      = $identityNode.GetAttribute('Name')
                $result.IdentityPublisher = $identityNode.GetAttribute('Publisher')
                $result.IdentityVersion   = $identityNode.GetAttribute('Version')
            }
        }
    }
    catch {
        $problems.Add("解压/解析 MSIX 失败: $($_.Exception.Message)")
    }
    finally {
        if ($zip) { $zip.Dispose() }
    }

    return [pscustomobject] $result
}

# ---------------------------------------------------------------------------
# 版本号校验
# MSIX 规范：四段、每段 0..65535。Store 额外要求第一段 >= 1（0.x 是给预发布留的，
# 上架版本不能是 0 开头，build-msix.ps1 也拦了这条）。
# ---------------------------------------------------------------------------
function Test-VersionString {
    param([string] $Value)

    if ([string]::IsNullOrWhiteSpace($Value)) { return '版本号为空' }
    if ($Value -notmatch '^\d+\.\d+\.\d+\.\d+$') { return "必须是 4 段数字（例如 1.0.1.0），现在是 '$Value'" }
    foreach ($part in $Value.Split('.')) {
        if ([int] $part -gt 65535) { return "每段必须在 0..65535 之间，'$part' 超了（MSIX 规范限制）" }
    }
    if ([int] $Value.Split('.')[0] -lt 1) { return "第一段必须 >= 1（Microsoft Store 要求版本号 >= 1.0.0.0），现在是 '$Value'" }
    return $null
}

# ---------------------------------------------------------------------------
# 凭据：环境变量优先，其次 store-credentials.psd1
# ---------------------------------------------------------------------------
function Get-StoreCredentials {
    param([string] $Path)

    $envMap = [ordered]@{
        TenantId     = 'STORE_TENANT_ID'
        ClientId     = 'STORE_CLIENT_ID'
        ClientSecret = 'STORE_CLIENT_SECRET'
        SellerId     = 'STORE_SELLER_ID'
        ProductId    = 'STORE_PRODUCT_ID'
        AppId        = 'STORE_APP_ID'
    }

    $values = @{}
    $sources = New-Object System.Collections.Generic.List[string]

    foreach ($key in $envMap.Keys) {
        $envValue = [Environment]::GetEnvironmentVariable($envMap[$key])
        if (-not [string]::IsNullOrWhiteSpace($envValue)) {
            $values[$key] = $envValue.Trim()
        }
    }
    if ($values.Count -gt 0) { $sources.Add('环境变量 ' + (($values.Keys | Sort-Object) -join ', ')) }

    $filePath = $Path
    if (-not $filePath) { $filePath = Join-Path $root 'store-credentials.psd1' }

    $fileExists = Test-Path -LiteralPath $filePath
    if ($fileExists) {
        try {
            $data = Import-PowerShellDataFile -Path $filePath
            foreach ($key in $envMap.Keys) {
                if (-not $values.ContainsKey($key)) {
                    if ($data.ContainsKey($key) -and -not [string]::IsNullOrWhiteSpace([string] $data[$key])) {
                        $values[$key] = ([string] $data[$key]).Trim()
                    }
                }
            }
            $sources.Add("文件 $filePath")
        }
        catch {
            Write-Bad "读凭据文件失败: $filePath"
            Write-Bad $_.Exception.Message
            throw
        }
    }

    if (-not $values.ContainsKey('ProductId') -or [string]::IsNullOrWhiteSpace($values['ProductId'])) {
        $values['ProductId'] = $DefaultProductId
    }
    # MSIX 应用在经典 submission API 里，路径上的 applicationId 就是 Store ID，
    # 所以 AppId 默认跟随 ProductId；单独留一个键是为了将来换 API 变体时不用改代码。
    if (-not $values.ContainsKey('AppId') -or [string]::IsNullOrWhiteSpace($values['AppId'])) {
        $values['AppId'] = $values['ProductId']
    }

    $missing = New-Object System.Collections.Generic.List[string]
    foreach ($required in @('TenantId', 'ClientId', 'ClientSecret')) {
        if (-not $values.ContainsKey($required) -or [string]::IsNullOrWhiteSpace($values[$required])) {
            $missing.Add($required)
        }
    }

    return [pscustomobject]@{
        Values       = $values
        Missing      = $missing.ToArray()
        Sources      = $sources.ToArray()
        FilePath     = $filePath
        FileExists   = $fileExists
        EnvVarNames  = $envMap
    }
}

function Show-CredentialGuide {
    param([string] $TriedPath, [string[]] $Missing)

    Write-Host ''
    Write-Bad '没有可用的 Microsoft Store 凭据，真实上传不会开始。'
    Write-Host ''
    Write-Host '  缺的键：' -ForegroundColor Yellow
    foreach ($m in $Missing) { Write-Host ("    - {0}" -f $m) -ForegroundColor Yellow }
    Write-Host ''
    Write-Host ("  找过的地方：(1) 环境变量 STORE_*  (2) 文件 {0}（存在: {1}）" -f $TriedPath, (Test-Path -LiteralPath $TriedPath)) -ForegroundColor Yellow
    Write-Host ''
    Write-Host '  ---------- 怎么拿到这四样东西（一次性，之后复用）----------' -ForegroundColor Cyan
    Write-Host ''
    Write-Host '  第 1 步：把 Partner Center 账号关联到 Entra ID 租户' -ForegroundColor White
    Write-Host '    https://partner.microsoft.com/dashboard → 齿轮「账户设置」→「组织」/「租户」'
    Write-Host '    没有租户可以在这里免费新建一个（「创建新的 Microsoft Entra ID 租户」，不收费）。'
    Write-Host '    文档：https://learn.microsoft.com/en-us/windows/apps/publish/partner-center/create-new-azure-ad-tenant'
    Write-Host ''
    Write-Host '  第 2 步：在 Partner Center 里登记一个 Entra 应用' -ForegroundColor White
    Write-Host '    账户设置 →「用户管理」→「Microsoft Entra 应用程序」→ 添加/新建应用'
    Write-Host '    ★ 必须给它分配「Manager」角色 —— 少了这一步，后面所有调用都会 403。'
    Write-Host '    文档：https://learn.microsoft.com/en-us/windows/apps/publish/partner-center/manage-azure-ad-applications-in-partner-center'
    Write-Host ''
    Write-Host '  第 3 步：抄下 Tenant ID 和 Client ID' -ForegroundColor White
    Write-Host '    点开刚建的应用，页面上就有这两个值。'
    Write-Host ''
    Write-Host '  第 4 步：点「Add new key」生成 Client Secret' -ForegroundColor White
    Write-Host '    ★ 生成后立刻抄走 —— 离开这个页面就再也看不到，只能重新生成。'
    Write-Host ''
    Write-Host '  第 5 步：写成本机凭据文件（该文件已在 .gitignore 里，不会进仓库）' -ForegroundColor White
    Write-Host '    文件路径：msix\store-credentials.psd1'
    Write-Host ''
    Write-Host '      @{' -ForegroundColor DarkGray
    Write-Host "          TenantId     = '<目录(租户) ID>'" -ForegroundColor DarkGray
    Write-Host "          ClientId     = '<应用程序(客户端) ID>'" -ForegroundColor DarkGray
    Write-Host "          ClientSecret = '<第 4 步的密钥值>'" -ForegroundColor DarkGray
    Write-Host "          SellerId     = '<Partner Center 的 Seller ID，网页 URL 里能看到>'" -ForegroundColor DarkGray
    Write-Host "          ProductId    = '$DefaultProductId'" -ForegroundColor DarkGray
    Write-Host "          AppId        = '$DefaultProductId'" -ForegroundColor DarkGray
    Write-Host '      }' -ForegroundColor DarkGray
    Write-Host ''
    Write-Host '    或者在 CI 里用环境变量（推荐，密钥不进文件）：' -ForegroundColor White
    Write-Host '      STORE_TENANT_ID / STORE_CLIENT_ID / STORE_CLIENT_SECRET / STORE_SELLER_ID / STORE_PRODUCT_ID / STORE_APP_ID'
    Write-Host ''
    Write-Host '  拿到之后先跑 -DryRun 确认凭据能被读到，再跑真上传。' -ForegroundColor White
    Write-Host ''
}

# ---------------------------------------------------------------------------
# AAD token
# v2.0 端点 + scope 是现在推荐写法；经典 Store 文档写的是 v1.0 端点 + resource。
# 两者对 AAD 应用都有效，所以 v2 失败时自动回退 v1 —— 但两次都失败就把两份错误都打出来。
# ---------------------------------------------------------------------------
function Get-AadToken {
    param(
        [Parameter(Mandatory = $true)][string] $TenantId,
        [Parameter(Mandatory = $true)][string] $ClientId,
        [Parameter(Mandatory = $true)][string] $ClientSecret
    )

    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    }
    catch {
        Write-WarnLine "设置 TLS 1.2 失败（继续）: $($_.Exception.Message)"
    }

    $v2Uri = '{0}/{1}/oauth2/v2.0/token' -f $TokenHost, $TenantId
    $v1Uri = '{0}/{1}/oauth2/token' -f $TokenHost, $TenantId

    $v2Body = @{
        grant_type    = 'client_credentials'
        client_id     = $ClientId
        client_secret = $ClientSecret
        scope         = $ApiScope
    }
    $v1Body = @{
        grant_type    = 'client_credentials'
        client_id     = $ClientId
        client_secret = $ClientSecret
        resource      = $ApiResource
    }

    try {
        Write-Note "POST $v2Uri (scope=$ApiScope)"
        $resp = Invoke-RestMethod -Method Post -Uri $v2Uri -Body $v2Body -ContentType 'application/x-www-form-urlencoded' -UseBasicParsing -TimeoutSec 60
        Write-Ok  'v2.0 端点取到 access_token'
        return $resp.access_token
    }
    catch {
        Write-WarnLine "v2.0 端点失败: $($_.Exception.Message)"
        if ($_.ErrorDetails -and $_.ErrorDetails.Message) {
            Write-WarnLine ("服务端: " + $_.ErrorDetails.Message)
        }
        Write-Note '回退到 v1.0 端点（resource=）重试一次……'
    }

    try {
        Write-Note "POST $v1Uri (resource=$ApiResource)"
        $resp = Invoke-RestMethod -Method Post -Uri $v1Uri -Body $v1Body -ContentType 'application/x-www-form-urlencoded' -UseBasicParsing -TimeoutSec 60
        Write-Ok  'v1.0 端点取到 access_token'
        return $resp.access_token
    }
    catch {
        Write-Bad 'v1.0 端点也失败了，认证没通过。'
        Write-Bad ("异常: {0}" -f $_.Exception.Message)
        if ($_.ErrorDetails -and $_.ErrorDetails.Message) {
            $detail = $_.ErrorDetails.Message
            Write-Bad ('服务端: ' + $detail)
            if ($detail -match 'AADSTS700016') {
                Write-Note 'AADSTS700016 = 这个 ClientId 在该租户里不存在。检查 ClientId 是否抄错，或应用是不是建在别的租户里。'
            }
            elseif ($detail -match 'AADSTS7000215') {
                Write-Note 'AADSTS7000215 = ClientSecret 不对（或已过期/已删除）。回 Partner Center 重新 Add new key。'
            }
            elseif ($detail -match 'AADSTS90002|AADSTS900023') {
                Write-Note 'AADSTS90002 = 找不到这个租户。TenantId 抄错，或者租户还没和 Partner Center 关联。'
            }
            elseif ($detail -match 'AADSTS65001|AADSTS70011') {
                Write-Note 'AADSTS65001/70011 = 权限/scope 不被该应用接受。确认应用是通过 Partner Center 建的那一个。'
            }
        }
        $script:ExitCode = 2
        throw 'AAD 认证失败'
    }
}

# ---------------------------------------------------------------------------
# 打包
# ---------------------------------------------------------------------------
function Invoke-Packaging {
    param([string] $VersionOverride)

    if (-not (Test-Path -LiteralPath $buildScript)) {
        Stop-WithCode 3 "找不到 $buildScript"
    }

    # 注意别叫 $args —— 那是 PowerShell 的自动变量。
    $cliArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $buildScript)
    if ($VersionOverride) { $cliArgs += @('-Version', $VersionOverride) }

    Write-Note ("powershell " + ($cliArgs -join ' '))
    & powershell @cliArgs
    if ($LASTEXITCODE -ne 0) {
        Stop-WithCode 3 "build-msix.ps1 失败，退出码 $LASTEXITCODE"
    }
}

# ---------------------------------------------------------------------------
# 主流程
# ---------------------------------------------------------------------------
try {
    Write-Host ''
    Write-Host '============================================================' -ForegroundColor White
    Write-Host '   R Battery  →  Microsoft Store 发布流水线' -ForegroundColor White
    $modeText = '真实运行'
    if ($WhatIf) { $modeText = '-WhatIf 只打印计划' }
    elseif ($DryRun) { $modeText = '-DryRun 本地跑完、不发网络请求' }
    Write-Host ("   模式: {0}" -f $modeText) -ForegroundColor White
    Write-Host '============================================================' -ForegroundColor White

    # =====================================================================
    # [1] 前置校验
    # =====================================================================
    Write-Step '1/10' '前置校验'

    if (-not (Test-Path -LiteralPath $propsPath)) { Stop-WithCode 3 "找不到 $propsPath" }
    $props = Import-PowerShellDataFile -Path $propsPath
    Write-Ok "package.props.psd1 已读取（IdentityName=$($props.IdentityName)）"

    $effectiveVersion = $props.Version
    if ($Version) { $effectiveVersion = $Version }
    $versionError = Test-VersionString -Value $effectiveVersion
    if ($versionError) { Stop-WithCode 3 "版本号不合法：$versionError" }
    Write-Ok "版本号 $effectiveVersion（4 段合法，第一段 >= 1）"

    if ($Package) {
        Write-Note "已指定 -Package，跳过编译。版本号仅用于校验比对：$effectiveVersion"
    }
    else {
        if (-not (Test-Path -LiteralPath $exePath)) {
            Stop-WithCode 3 "找不到 $exePath —— 先跑一次 build.cmd 编译出 exe。"
        }
        $exeSize = (Get-Item -LiteralPath $exePath).Length
        Write-Ok ("exe 存在: {0}（{1:N0} 字节）" -f $exePath, $exeSize)
    }

    $makeappx = $null
    $cmd = Get-Command makeappx.exe -ErrorAction SilentlyContinue
    if ($cmd) { $makeappx = $cmd.Source }
    elseif ($props.MakeAppxPath -and (Test-Path -LiteralPath $props.MakeAppxPath)) { $makeappx = $props.MakeAppxPath }
    if (-not $makeappx) {
        Write-Bad '找不到 makeappx.exe'
        Write-Note '两条路：装 Windows SDK（含 MakeAppx），或在 package.props.psd1 里把 MakeAppxPath 指对。'
        Write-Note '注意：makeappx 依赖同目录的 .manifest 做免注册 COM 绑定，只拷 exe 会报 side-by-side configuration is incorrect。'
        Stop-WithCode 3 'makeappx 不可用'
    }
    Write-Ok "makeappx: $makeappx"

    $expectedMsix = Join-Path $pkgDir ('{0}_{1}_x64.msix' -f $props.IdentityName, $effectiveVersion)
    Write-Note "期望产物: $expectedMsix"

    if ($WhatIf) {
        Write-Host ''
        Write-Host '  -WhatIf：以下是真实运行会执行的步骤，本次一个都没做。' -ForegroundColor Yellow
        foreach ($line in @(
            "[2/10] 打包      powershell -File .\build-msix.ps1 -Version $effectiveVersion",
            "[3/10] 校验     解压 $expectedMsix，断言无 AppxSignature.p7x、Identity 与 psd1 一致",
            '[4/10] 凭据     读环境变量 STORE_* 或 msix\store-credentials.psd1',
            "[5/10] 认证     POST $TokenHost/<TenantId>/oauth2/v2.0/token  scope=$ApiScope",
            "[6/10] 建提交   POST $ApiBase/applications/<AppId>/submissions",
            "[7/10] 回写     PUT  $ApiBase/applications/<AppId>/submissions/<submissionId>",
            '[8/10] 上传     PUT  <fileUploadUrl>（ZIP 内含一个 .msix，header x-ms-blob-type: BlockBlob）',
            '[9/10] 提交     POST .../submissions/<submissionId>/commit',
            '[10/10] 轮询    GET  .../submissions/<submissionId>/status'
        )) { Write-Host ("    {0}" -f $line) -ForegroundColor Yellow }
        Write-Host ''
        Write-Host '  换成 -DryRun 可以真的编译 + 校验，只是不联网。' -ForegroundColor Yellow
        $script:ExitCode = 0
        return
    }

    # =====================================================================
    # [2] 打包
    # =====================================================================
    Write-Step '2/10' '打包 MSIX'

    if ($Package) {
        $msixPath = (Resolve-Path -LiteralPath $Package).Path
        Write-Ok "使用现成包: $msixPath"
    }
    else {
        Invoke-Packaging -VersionOverride $(if ($Version) { $Version } else { $null })
        $msixPath = $expectedMsix
        if (-not (Test-Path -LiteralPath $msixPath)) {
            # build-msix.ps1 的产物名是 Name_Version_x64.msix，理论上一定一致；不一致说明
            # props 里的版本被 -Version 覆盖后没落到文件名上，属于必须人工看的异常。
            $candidates = @(Get-ChildItem -LiteralPath $pkgDir -Filter '*.msix' -ErrorAction SilentlyContinue |
                            Sort-Object LastWriteTime -Descending)
            if ($candidates.Count -gt 0) {
                $msixPath = $candidates[0].FullName
                Write-WarnLine "期望的产物名不存在，改用最新的 .msix: $msixPath"
            }
            else {
                Stop-WithCode 3 "打包结束但 $pkgDir 下没有 .msix"
            }
        }
        Write-Ok "产物: $msixPath"
    }

    # 包名 / ZIP 路径在这里定下来：第 7、8 步要用，DryRun 打印计划时也要用。
    # ZIP 不能放在 $uploadDir 里 —— CreateFromDirectory 会枚举源目录，
    # 目标文件在源目录里会把自身也打进去（边写边读，包会坏）。
    $msixLeaf = Split-Path $msixPath -Leaf
    $zipPath  = Join-Path $pkgDir ($msixLeaf + '.zip')

    # =====================================================================
    # [3] 校验产物
    # =====================================================================
    Write-Step '3/10' '校验产物（未签名 + Identity 三项对齐）'

    $inspect = Get-MsixInspection -Path $msixPath

    foreach ($p in $inspect.Problems) { Write-Bad $p }
    if ($inspect.Problems.Count -gt 0) { Stop-WithCode 3 'MSIX 结构校验未通过' }

    Write-Ok ("包可读：{0:N0} 字节 / {1} 个条目" -f $inspect.SizeBytes, $inspect.EntryCount)

    if ($inspect.HasSignature) {
        Write-Bad '包里含 AppxSignature.p7x —— 这是签过名的包，Store 会拒收。'
        Write-Note '重跑不带 -Sign 的 build-msix.ps1（本脚本调用时本来就不带 -Sign）。'
        Stop-WithCode 3 '提交 Store 的包必须未签名'
    }
    Write-Ok '无 AppxSignature.p7x —— 未签名，符合提交要求'

    $identityChecks = @(
        @{ Key = 'Name';      Expected = $props.IdentityName;       Actual = $inspect.IdentityName }
        @{ Key = 'Publisher'; Expected = $props.Publisher;          Actual = $inspect.IdentityPublisher }
        @{ Key = 'Version';   Expected = $effectiveVersion;         Actual = $inspect.IdentityVersion }
    )
    $identityFailures = New-Object System.Collections.Generic.List[string]
    foreach ($check in $identityChecks) {
        if ([string] $check.Expected -cne [string] $check.Actual) {
            $identityFailures.Add(("{0}: 期望 '{1}'，实际 '{2}'" -f $check.Key, $check.Expected, $check.Actual))
        }
        else {
            Write-Ok ("Identity/{0} = {1}" -f $check.Key, $check.Actual)
        }
    }
    if ($identityFailures.Count -gt 0) {
        foreach ($f in $identityFailures) { Write-Bad $f }
        Write-Note 'Identity 必须和 Partner Center「产品标识」页逐字一致（区分大小写），否则提交会报 InvalidPackageIdentityName。'
        Stop-WithCode 3 'AppxManifest.xml 的 Identity 与 package.props.psd1 不一致'
    }

    # =====================================================================
    # [4] 凭据
    # =====================================================================
    Write-Step '4/10' '读取凭据'

    $creds = Get-StoreCredentials -Path $CredentialsPath
    if ($creds.Sources.Count -gt 0) {
        Write-Ok ('来源: ' + ($creds.Sources -join ' + '))
    }
    else {
        Write-WarnLine '环境变量和凭据文件都没有给出任何值'
    }
    Write-Note ("凭据文件: {0}（存在: {1}）" -f $creds.FilePath, $creds.FileExists)

    foreach ($key in @('TenantId', 'ClientId', 'SellerId', 'ProductId', 'AppId')) {
        if ($creds.Values.ContainsKey($key) -and -not [string]::IsNullOrWhiteSpace($creds.Values[$key])) {
            Write-Note ("{0,-13}= {1}" -f $key, $creds.Values[$key])
        }
    }
    if ($creds.Values.ContainsKey('ClientSecret') -and -not [string]::IsNullOrWhiteSpace($creds.Values['ClientSecret'])) {
        Write-Note ("{0,-13}= {1}（长度 {2}，不显示）" -f 'ClientSecret', '***', $creds.Values['ClientSecret'].Length)
    }

    $hasCreds = ($creds.Missing.Count -eq 0)

    if (-not $hasCreds) {
        if ($DryRun) {
            Write-WarnLine '缺少凭据 —— 本地部分到此为止是正常的；真实运行会在这一步停下。'
        }
        else {
            Show-CredentialGuide -TriedPath $creds.FilePath -Missing $creds.Missing
            # 用 throw 而不是 exit：exit 在 try/catch 里会被下面的 catch 当成异常捕获，
            # 走 Stop-WithCode 能保证退出码是 2 且错误信息如实打印。
            Stop-WithCode 2 ('缺少 Microsoft Store 凭据: ' + ($creds.Missing -join ', '))
        }
    }

    # 缺凭据时给占位符，让 DryRun 的计划照样能看清结构（而不是打出一串空白）。
    $tenantId  = $creds.Values['TenantId']
    $clientId  = $creds.Values['ClientId']
    $appId     = $creds.Values['AppId']
    $productId = $creds.Values['ProductId']
    if ([string]::IsNullOrWhiteSpace($tenantId)) { $tenantId = '<TenantId>' }
    if ([string]::IsNullOrWhiteSpace($clientId)) { $clientId = '<ClientId>' }
    if ([string]::IsNullOrWhiteSpace($appId))    { $appId    = $DefaultProductId }

    $submissionsUri = '{0}/applications/{1}/submissions' -f $ApiBase, $appId

    if ($DryRun) {
        # -------------------------------------------------------------
        # DryRun：把真实运行会发的请求逐条打印出来，然后退出
        # -------------------------------------------------------------
        Write-Step '5/10' '（DryRun）API 请求计划'
        Write-Host ''
        Write-Host '  下面这些请求在真实运行时会依次发出；本次一个都没发。' -ForegroundColor Yellow
        Write-Host ''
        Write-Plan ("[5/10] POST {0}/{1}/oauth2/v2.0/token" -f $TokenHost, $TenantId)
        Write-Note 'Content-Type: application/x-www-form-urlencoded'
        Write-Note ("body: grant_type=client_credentials&client_id={0}&client_secret=***&scope={1}" -f $clientId, $ApiScope)
        Write-Note '响应: { "token_type": "Bearer", "expires_in": 3599, "access_token": "***" }  —— 有效期 60 分钟'
        Write-Host ''
        Write-Plan ("[6/10] POST {0}" -f $submissionsUri)
        Write-Note 'Authorization: Bearer <token>'
        Write-Note '无请求体（官方："Do not provide a request body for this method."）'
        Write-Note '响应: { "id": "<submissionId>", "fileUploadUrl": "https://productingestionbin1.blob.core.windows.net/...", ... }'
        Write-Note '  ★ 这个 POST 会把「上一次已发布提交」整份复制成新的草稿；没有历史提交时会失败，'
        Write-Note '    所以第一次上架必须先在 Partner Center 手工做完一次提交。'
        Write-Host ''
        Write-Plan ("[7/10] PUT  {0}/applications/{1}/submissions/<submissionId>" -f $ApiBase, $appId)
        Write-Note 'body: 第 6 步响应原样回传，只把 applicationPackages 换成一项：'
        Write-Note ("        {{ ""fileName"": ""{0}"", ""fileStatus"": ""PendingUpload""," -f (Split-Path $msixPath -Leaf))
        Write-Note  '           "minimumDirectXVersion": "None", "minimumSystemRam": "None" }'
        Write-Note '注意 ConvertTo-Json 必须 -Depth 100，默认 2 层会把整份提交数据压成 {}。'
        Write-Host ''
        Write-Plan '[8/10] PUT  <第 6 步返回的 fileUploadUrl>'
        Write-Note 'Content-Type: application/octet-stream'
        Write-Note 'x-ms-blob-type: BlockBlob'
        Write-Note ("body: {0}（约 {1:N0} 字节）" -f (Join-Path $pkgDir ($msixLeaf + '.zip')), ($inspect.SizeBytes + 512))
        Write-Note '  ZIP 根下直接放 .msix，文件名与第 7 步 fileName 完全一致。'
        Write-Host ''
        Write-Plan ("[9/10] POST {0}/applications/{1}/submissions/<submissionId>/commit" -f $ApiBase, $appId)
        Write-Note '无请求体。响应: { "status": "CommitStarted" }'
        Write-Host ''
        Write-Plan ("[10/10] GET  {0}/applications/{1}/submissions/<submissionId>/status" -f $ApiBase, $appId)
        Write-Note '轮询直到状态离开 CommitStarted：成功 → PreProcessing，失败 → CommitFailed（细节在 statusDetails.errors）'
        Write-Note ("超时 {0} 分钟，每 {1} 秒一次" -f $PollTimeoutMinutes, $PollIntervalSeconds)
        Write-Host ''
        Write-Ok 'DryRun 完成：本地编译 + 产物断言全部通过，网络请求 0 次。'
        $script:ExitCode = 0
        return
    }

    # =====================================================================
    # [5] 认证
    # =====================================================================
    Write-Step '5/10' '换取 Azure AD access token'
    $token = Get-AadToken -TenantId $tenantId -ClientId $clientId -ClientSecret $creds.Values['ClientSecret']
    if (-not $token) { Stop-WithCode 2 '拿到的 access_token 为空' }
    Write-Ok ("access_token 长度 {0}，60 分钟内有效" -f $token.Length)

    # =====================================================================
    # [6] 创建提交
    # =====================================================================
    Write-Step '6/10' '创建提交（复制上一次已发布提交为草稿）'
    $submission = Invoke-StoreApi -Method 'POST' -Uri $submissionsUri -AccessToken $token
    if (-not $submission) { Stop-WithCode 4 '创建提交返回空响应' }
    if (-not $submission.PSObject.Properties['id'] -or -not $submission.id) {
        Stop-WithCode 4 '创建提交的响应里没有 id'
    }
    $submissionId = [string] $submission.id
    Write-Ok "submissionId = $submissionId"
    Write-Note ("status = {0}" -f $submission.status)

    $uploadUrl = $null
    if ($submission.PSObject.Properties['fileUploadUrl']) { $uploadUrl = [string] $submission.fileUploadUrl }
    if ([string]::IsNullOrWhiteSpace($uploadUrl)) {
        Stop-WithCode 4 '响应里没有 fileUploadUrl，无法上传包'
    }
    Write-Ok ("fileUploadUrl 已拿到: {0}" -f (Hide-Secret $uploadUrl))

    # =====================================================================
    # [7] 回写提交数据（声明新包）
    # =====================================================================
    Write-Step '7/10' '回写提交数据：applicationPackages 指向新包'

    $newPackage = [pscustomobject]@{
        fileName              = $msixLeaf
        fileStatus            = 'PendingUpload'
        minimumDirectXVersion = 'None'
        minimumSystemRam      = 'None'
    }

    # 只保留这一个包：上架 x64 单包，旧包不能跟着复制过来，否则 Store 会认为你同时提交了多份。
    if ($submission.PSObject.Properties['applicationPackages']) {
        $submission.applicationPackages = @($newPackage)
    }
    else {
        $submission | Add-Member -NotePropertyName 'applicationPackages' -NotePropertyValue @($newPackage)
    }
    Write-Ok "applicationPackages = [ $msixLeaf ]（fileStatus=PendingUpload）"

    $updated = Invoke-StoreApi -Method 'PUT' -Uri ("{0}/{1}" -f $submissionsUri, $submissionId) -AccessToken $token -Body $submission
    if ($updated -and $updated.PSObject.Properties['status']) {
        Write-Ok ("提交数据已接受，status = {0}" -f $updated.status)
    }
    else {
        Write-Ok '提交数据已接受'
    }

    # =====================================================================
    # [8] 上传 ZIP
    # =====================================================================
    Write-Step '8/10' '上传包（ZIP → SAS URL）'

    if (Test-Path -LiteralPath $uploadDir) { Remove-Item -LiteralPath $uploadDir -Recurse -Force }
    New-Item -ItemType Directory -Path $uploadDir -Force | Out-Null
    Copy-Item -LiteralPath $msixPath -Destination (Join-Path $uploadDir $msixLeaf) -Force

    Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction Stop
    if (Test-Path -LiteralPath $zipPath) { Remove-Item -LiteralPath $zipPath -Force }
    [System.IO.Compression.ZipFile]::CreateFromDirectory($uploadDir, $zipPath)
    $zipSize = (Get-Item -LiteralPath $zipPath).Length
    Write-Ok ("ZIP 已生成: {0}（{1:N0} 字节）" -f $zipPath, $zipSize)

    try {
        # 注意：SAS URL 带查询串，别把它写进日志／issue。
        Write-Note ("PUT {0}" -f (Hide-Secret $uploadUrl))
        $uploadResp = Invoke-WebRequest -Method Put -Uri $uploadUrl -InFile $zipPath `
            -Headers @{ 'x-ms-blob-type' = 'BlockBlob' } `
            -ContentType 'application/octet-stream' -UseBasicParsing -TimeoutSec 600
        Write-Ok ("上传完成，HTTP {0}" -f [int] $uploadResp.StatusCode)
    }
    catch {
        Write-Bad '上传 ZIP 到 Azure Blob 失败'
        Write-Bad ("异常: {0}" -f $_.Exception.Message)
        if ($_.ErrorDetails -and $_.ErrorDetails.Message) {
            Write-Bad ('服务端: ' + $_.ErrorDetails.Message)
        }
        if ($_.Exception.PSObject.Properties['Response'] -and $_.Exception.Response) {
            try {
                Write-Bad ("HTTP {0} {1}" -f [int] $_.Exception.Response.StatusCode, $_.Exception.Response.StatusDescription)
                Write-Note 'SAS URL 可能已过期（有效期通常几小时）或已被用过一次。'
                Write-Note '重跑本脚本会拿到新的 fileUploadUrl；旧草稿会被替换，不是问题。'
            }
            catch { }
        }
        $script:ExitCode = 4
        throw '上传失败'
    }

    if ($NoCommit) {
        Write-Step '9/10' '（-NoCommit）跳过 commit'
        Write-WarnLine '包已上传为草稿，但没有提交认证。'
        Write-Note "submissionId = $submissionId"
        Write-Note '去 Partner Center 检查这一稿，然后在那里点「提交认证」，或者重跑本脚本去掉 -NoCommit。'
        $script:ExitCode = 0
        return
    }

    # =====================================================================
    # [9] commit
    # =====================================================================
    Write-Step '9/10' '提交（commit）'
    $commitResp = Invoke-StoreApi -Method 'POST' -Uri ("{0}/{1}/commit" -f $submissionsUri, $submissionId) -AccessToken $token
    $commitStatus = '<unknown>'
    if ($commitResp -and $commitResp.PSObject.Properties['status']) { $commitStatus = [string] $commitResp.status }
    Write-Ok "commit 已受理，status = $commitStatus"

    # =====================================================================
    # [10] 轮询
    # =====================================================================
    Write-Step '10/10' ("轮询状态（最长 {0} 分钟）" -f $PollTimeoutMinutes)

    $statusUri   = "{0}/{1}/status" -f $submissionsUri, $submissionId
    $deadline    = (Get-Date).AddMinutes($PollTimeoutMinutes)
    $lastStatus  = $null
    $finalStatus = $null
    $failureStates = @('CommitFailed', 'PreProcessingFailed', 'CertificationFailed', 'PublishFailed', 'ReleaseFailed', 'Canceled')
    $pendingStates = @('CommitStarted', 'PendingCommit', 'None')

    while ($true) {
        Start-Sleep -Seconds $PollIntervalSeconds

        $st = Invoke-StoreApi -Method 'GET' -Uri $statusUri -AccessToken $token
        $current = '<unknown>'
        if ($st -and $st.PSObject.Properties['status']) { $current = [string] $st.status }

        if ($current -ne $lastStatus) {
            Write-Host ("         {0:HH:mm:ss}  {1}" -f (Get-Date), $current) -ForegroundColor White
            $lastStatus = $current
        }

        # 有错误就立刻摊开，别等超时
        if ($st -and $st.PSObject.Properties['statusDetails'] -and $st.statusDetails) {
            $details = $st.statusDetails
            if ($details.PSObject.Properties['errors'] -and $details.errors -and @($details.errors).Count -gt 0) {
                Write-Bad '状态里有错误：'
                foreach ($e in @($details.errors)) {
                    $code = if ($e.PSObject.Properties['code']) { $e.code } else { '?' }
                    $msg  = if ($e.PSObject.Properties['details']) { $e.details } else { '' }
                    Write-Bad ("[{0}] {1}" -f $code, $msg)
                }
            }
            if ($details.PSObject.Properties['warnings'] -and $details.warnings -and @($details.warnings).Count -gt 0) {
                foreach ($w in @($details.warnings)) {
                    $code = if ($w.PSObject.Properties['code']) { $w.code } else { '?' }
                    $msg  = if ($w.PSObject.Properties['details']) { $w.details } else { '' }
                    Write-WarnLine ("警告 [{0}] {1}" -f $code, $msg)
                }
            }
        }

        if ($failureStates -contains $current) {
            $finalStatus = $current
            break
        }
        # 离开 CommitStarted 就说明 ingestion 已经接手，不必再等认证结果（可能好几天）
        if ($pendingStates -notcontains $current) {
            $finalStatus = $current
            break
        }
        if ((Get-Date) -gt $deadline) {
            Write-WarnLine ("轮询超时（{0} 分钟），最后状态: {1}" -f $PollTimeoutMinutes, $current)
            $finalStatus = $current
            break
        }
    }

    Write-Host ''
    if ($failureStates -contains $finalStatus) {
        Write-Bad ("提交失败，最终状态: {0}" -f $finalStatus)
        Write-Note '上面的 [code] 就是官方 statusDetails.errors[].code。常见含义见 publish-store.md 的排查表。'
        Write-Note "submissionId = $submissionId （可用 GET $statusUri 继续查，或在 Partner Center 看这一稿）"
        $script:ExitCode = 4
        throw "提交失败: $finalStatus"
    }

    Write-Ok ("commit 成功，当前状态: {0}" -f $finalStatus)
    Write-Note "submissionId = $submissionId"

    # =====================================================================
    # 收尾：剩下必须人工做的事
    # =====================================================================
    Write-Host ''
    Write-Host '============================================================' -ForegroundColor White
    Write-Host '  自动化到此为止。剩下必须人工做的事：' -ForegroundColor White
    Write-Host '============================================================' -ForegroundColor White
    Write-Host ''
    Write-Host '  1. 等认证。' -ForegroundColor White
    Write-Host '     自动化只覆盖到「提交被接受」（状态进入 PreProcessing）。之后是微软的'
    Write-Host '     预检 → 认证（通常几小时到几天）→ 发布。认证结论只发邮件/Partner Center。'
    Write-Host ''
    Write-Host '  2. 处理认证反馈（如果被打回）。' -ForegroundColor White
    Write-Host '     认证报告：Partner Center → 该产品 → 提交 → 认证报告。'
    Write-Host '     改完代码重跑：.\publish-store.ps1 -Version <更高版本号>'
    Write-Host '     ★ 版本号必须比已发布的大，否则 Store 直接拒收。'
    Write-Host ''
    Write-Host '  3. 发布（仅当 targetPublishMode = Manual）。' -ForegroundColor White
    Write-Host '     这个值是从上一次提交复制来的。如果是 Manual，认证通过后要有人在'
    Write-Host '     Partner Center 点「Publish now」，包才会真的上线。想全自动就在提交数据里'
    Write-Host '     把 targetPublishMode 改成 Immediate（本脚本没改，避免误上线）。'
    Write-Host ''
    Write-Host '  4. 商店页面素材（首次上架才需要）。' -ForegroundColor White
    Write-Host '     API 只复制上一稿的 listings。截图（至少 1 张）、Store logo、分类、'
    Write-Host '     隐私政策 URL、age ratings 问卷，都得在 Partner Center 网页上填。'
    Write-Host '     参考同目录 store-listing.md（文案）和 privacy-policy.html（隐私政策）。'
    Write-Host ''
    Write-Host '  5. 首次上架的特殊性。' -ForegroundColor White
    Write-Host '     如果这个产品还从没在 Partner Center 手工提交过，本脚本第 6 步就会失败 ——'
    Write-Host '     那是设计如此：API 不能在网页之外创建 app，也不能跳过 age ratings 问卷。'
    Write-Host '     先去网页把第一稿提交掉，之后每一次更新都能用这个脚本。'
    Write-Host ''
    Write-Host '  6. 保管理凭据。' -ForegroundColor White
    Write-Host '     store-credentials.psd1 已进 .gitignore；ClientSecret 有效期最长 2 年，'
    Write-Host '     到期前 401 会让你想起来。在 Partner Center 点 Add new key 换新即可。'
    Write-Host ''

    $script:ExitCode = 0
}
catch {
    Write-Host ''
    if ($script:IntentionalStop) {
        # 预期内的停止（缺凭据、校验不过、提交失败……）：原因已经打印过了，只补一行。
        Write-Host ("  结束（退出码 {0}）：{1}" -f $script:ExitCode, $_.Exception.Message) -ForegroundColor Yellow
    }
    else {
        Write-Bad '脚本因意外错误中止'
        Write-Host ("  错误: {0}" -f $_.Exception.Message) -ForegroundColor Red
        if ($_.InvocationInfo -and $_.InvocationInfo.PositionMessage) {
            Write-Host ($_.InvocationInfo.PositionMessage) -ForegroundColor DarkRed
        }
        if ($_.ScriptStackTrace) {
            Write-Host '  调用栈:' -ForegroundColor DarkRed
            Write-Host ($_.ScriptStackTrace) -ForegroundColor DarkRed
        }
        if ($script:ExitCode -eq 1) { $script:ExitCode = 3 }
    }
}
finally {
    # 上传用的临时 ZIP 和暂存目录不留在仓库里（AppPackages/ 已 gitignore，这里再清一次更干净）
    if (-not $WhatIf) {
        if (Test-Path -LiteralPath $uploadDir) {
            try { Remove-Item -LiteralPath $uploadDir -Recurse -Force -ErrorAction Stop }
            catch { Write-WarnLine ("清理临时目录失败（不影响结果）: {0}" -f $_.Exception.Message) }
        }
        if ($zipPath -and (Test-Path -LiteralPath $zipPath)) {
            try { Remove-Item -LiteralPath $zipPath -Force -ErrorAction Stop }
            catch { Write-WarnLine ("清理临时 ZIP 失败（不影响结果）: {0}" -f $_.Exception.Message) }
        }
    }
}

exit $script:ExitCode
