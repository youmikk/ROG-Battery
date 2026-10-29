<#
.SYNOPSIS
  用 Microsoft Store 提交 API 直接把 .msixupload 提交到商店（不依赖 msstore CLI）。

.DESCRIPTION
  为什么不用 msstore publish：
  它要求一个「项目目录」并从项目里推断发布者身份。我们没有 .csproj；
  放一份 Package.appxmanifest 会让它当成 UWP 项目去 MSBuild restore，
  在装了 Visual Studio 的 runner 上直接抛 Failed to restore packages。
  所以这里直接走 REST：
    取令牌 -> 删旧草稿 -> POST 建草稿 -> PUT 改 fileName -> PUT 传包 -> POST commit

.PARAMETER PackagePath
  要提交的 .msixupload 或 .msix 文件。

.PARAMETER DryRun
  只建草稿、不 commit（用于核对）。

.EXAMPLE
  ./tools/msstore-publish.ps1 -ProductId 9NRC8HCD4BZ7 -PackagePath dist/x.msixupload -DryRun
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string] $ProductId,
    [Parameter(Mandatory = $true)][string] $PackagePath,
    [string] $TenantId,
    [string] $ClientId,
    [string] $ClientSecret,
    [switch] $DryRun
)

$ErrorActionPreference = 'Stop'
$api = 'https://manage.devcenter.microsoft.com/v1.0/my'

function Get-EnvOrParam($value, $name) {
    if ($value) { return $value }
    $v = [Environment]::GetEnvironmentVariable($name)
    if (-not $v) { throw "缺少 $name" }
    return $v
}

$TenantId     = Get-EnvOrParam $TenantId     'AZURE_AD_TENANT_ID'
$ClientId     = Get-EnvOrParam $ClientId     'AZURE_AD_APPLICATION_CLIENT_ID'
$ClientSecret = Get-EnvOrParam $ClientSecret 'AZURE_AD_APPLICATION_SECRET'

if (-not (Test-Path $PackagePath)) { throw "找不到包：$PackagePath" }
$pkg = (Resolve-Path $PackagePath).Path
$pkgName = Split-Path -Leaf $pkg
Write-Host "包：$pkgName  ($((Get-Item $pkg).Length) B)" -ForegroundColor Cyan

# ── 1. 取 access token（client credentials）────────────────────────────
Write-Host '获取访问令牌…' -ForegroundColor DarkGray
$tokenBody = @{
    grant_type    = 'client_credentials'
    client_id     = $ClientId
    client_secret = $ClientSecret
    scope         = 'https://manage.devcenter.microsoft.com/.default'
}
$tok = Invoke-RestMethod -Method Post -ContentType 'application/x-www-form-urlencoded' `
    -Body $tokenBody -Uri "https://login.microsoftonline.com/$TenantId/oauth2/v2.0/token"
$headers = @{ Authorization = "Bearer $($tok.access_token)" }
Write-Host '  OK' -ForegroundColor DarkGray

# ── 2. 清掉已有草稿（API 只能删 API 建的草稿）──────────────────────────
$subsUri = "$api/applications/$ProductId/submissions"

# GET /submissions 的形状文档没写清楚：实测可能是裸数组，也可能包一层 { value: [...] }。
# 之前只按裸数组解析，遇到包装形状就一个都拿不到 → 旧草稿删不掉 → 建新草稿 409。
function Get-Submissions {
    $resp = Invoke-RestMethod -Method Get -Uri $subsUri -Headers $headers
    if ($resp -is [array]) { return @($resp) }
    if ($resp.PSObject.Properties['value']) { return @($resp.value) }
    return @($resp)
}

try {
    $subs = Get-Submissions
} catch {
    $subs = @()
}
Write-Host "  已列到 $(@($subs).Count) 个提交：$((@($subs) | ForEach-Object { "$($_.id)/$($_.status)" }) -join '  ')" -ForegroundColor DarkGray
# 只有草稿/失败态可以删；正在认证或发布的提交删不掉，也不能并存第二个提交。
$deletable = @('PendingCommit', 'CommitFailed')
$busyStatuses = @('CommitStarted', 'Certification', 'Publishing', 'PreProcessing', 'PendingDelete')
$busy = $null
foreach ($s in @($subs)) {
    if ($null -eq $s -or -not $s.id) { continue }
    if ($s.status -in $busyStatuses) {
        $busy = $s
        continue
    }
    if ($s.status -notin $deletable) {
        Write-Host "跳过提交 $($s.id)（状态 $($s.status)）" -ForegroundColor DarkGray
        continue
    }
    Write-Host "删除旧草稿 $($s.id)（状态 $($s.status)）…" -ForegroundColor DarkGray
    try {
        Invoke-RestMethod -Method Delete -Uri "$subsUri/$($s.id)" -Headers $headers | Out-Null
        Write-Host '  已删除' -ForegroundColor DarkGray
    } catch {
        Write-Host "  删除失败：$($_.Exception.Message)" -ForegroundColor Yellow
        Write-Host '  若提示 Ingestion API can only ... created through the API，' -ForegroundColor Yellow
        Write-Host '  说明该草稿是网页上手工建的，请去 Partner Center 删掉后再跑。' -ForegroundColor Yellow
        throw
    }
}

# 商店同一时间只允许一个提交。上一个提交还在认证/发布时，这里只能等，
# 不该把整条 CI 判红 —— MSIX 已经作为 artifact 上传了，认证完成后重跑一次即可。
if ($busy) {
    Write-Host ''
    Write-Host "商店已有提交正在进行：id=$($busy.id) status=$($busy.status)" -ForegroundColor Yellow
    Write-Host "等它结束后重跑本步骤即可（或去这里手动提交）：" -ForegroundColor Yellow
    Write-Host "  https://partner.microsoft.com/dashboard/products/$ProductId/submissions" -ForegroundColor Yellow
    Write-Host "::warning::商店已有提交正在进行（$($busy.status)），本次跳过自动提交；MSIX 包已上传为 artifact"
    exit 0
}

# ── 3. 建新草稿 ───────────────────────────────────────────────────────
Write-Host '创建草稿…' -ForegroundColor DarkGray
# 文档明确：这个方法不要请求体 —— 带 {} 会被判
#   InvalidParameterValue: The size of Listings must be 1 or more
# 但 Content-Type 必须是 application/json，否则报 Only JSON content is accepted。
try {
    $sub = Invoke-RestMethod -Method Post -Uri $subsUri -Headers $headers -ContentType 'application/json'
} catch {
    $msg = $_.Exception.Message
    if ($msg -match 'Conflict' -or $msg -match 'InvalidState') {
        # 上一次跑挂在半路，留下一个 PendingCommit 草稿，但上面的列表没删掉它。
        # 这里再清一遍再试一次，别把整条 CI 判红。
        Write-Host "  提交位被占用（$msg），再清一次草稿后重试…" -ForegroundColor Yellow
        try {
            foreach ($x in @(Get-Submissions)) {
                if ($x.id -and $x.status -in $deletable) {
                    Write-Host "    删除 $($x.id)（$($x.status)）…" -ForegroundColor DarkGray
                    try {
                        Invoke-RestMethod -Method Delete -Uri "$subsUri/$($x.id)" -Headers $headers | Out-Null
                        Write-Host '      已删除' -ForegroundColor DarkGray
                    } catch {
                        Write-Host "      删除失败：$($_.Exception.Message)" -ForegroundColor Yellow
                    }
                }
            }
        } catch { }
        Start-Sleep -Seconds 5
        try {
            $sub = Invoke-RestMethod -Method Post -Uri $subsUri -Headers $headers -ContentType 'application/json'
        } catch {
            $msg2 = $_.Exception.Message
            Write-Host ''
            Write-Host "创建草稿仍被拒：$msg2" -ForegroundColor Yellow
            Write-Host "::warning::商店提交位被占用，本次跳过自动提交；MSIX 包已上传为 artifact"
            exit 0
        }
    } else {
        throw
    }
}
if (-not $sub) { throw '创建草稿失败（没有返回提交对象）' }
Write-Host "  id=$($sub.id)  status=$($sub.status)" -ForegroundColor DarkGray

$pkgs = @($sub.applicationPackages)
if ($pkgs.Count -eq 0) { throw '草稿里没有 applicationPackages 槽位，无法挂包' }
Write-Host "  现有包槽：$($pkgs.Count) 个，第一个 $($pkgs[0].fileName)  fileStatus=$($pkgs[0].fileStatus)" -ForegroundColor DarkGray
Write-Host "  槽字段：$(($pkgs[0].PSObject.Properties | ForEach-Object { $_.Name }) -join ', ')" -ForegroundColor DarkGray

# ── 4. 换包 ───────────────────────────────────────────────────────────
# 坑（2026-10-29 实测两次）：POST /submissions 会把上一次提交的 applicationPackages
# 整份克隆过来。如果克隆到的槽 fileStatus 已经是 Uploaded，**只改 fileName 会被 API
# 静默忽略**，commit 时仍然发布旧包 —— 现象是「CI 全绿、商店版本没变」。
# 但直接清 fileUploadUrl 又会报 InvalidParameterValue: Please keep all file entries。
# 所以这里依次试几种改法，取第一个能让 API 真的把 fileName 换成新包名的。
$variants = @(
    @{ name = 'fileName + fileStatus=PendingUpload'
       apply = { param($s) $s.applicationPackages[0].fileName = $pkgName
                          $s.applicationPackages[0].fileStatus = 'PendingUpload' } },
    @{ name = '仅改 fileName'
       apply = { param($s) $s.applicationPackages[0].fileName = $pkgName } },
    @{ name = 'fileName + fileStatus=PendingUpload + 清 fileUploadUrl'
       apply = { param($s) $s.applicationPackages[0].fileName = $pkgName
                          $s.applicationPackages[0].fileStatus = 'PendingUpload'
                          if ($s.applicationPackages[0].PSObject.Properties['fileUploadUrl']) {
                              $s.applicationPackages[0].fileUploadUrl = $null } } }
)

$sub2 = $null
foreach ($v in $variants) {
    $cur = Invoke-RestMethod -Method Get -Uri "$subsUri/$($sub.id)" -Headers $headers
    $cur.applicationPackages = @($cur.applicationPackages)
    & $v.apply $cur
    $body = $cur | ConvertTo-Json -Depth 32
    try {
        $try = Invoke-RestMethod -Method Put -Uri "$subsUri/$($sub.id)" -Headers $headers -Body $body -ContentType 'application/json'
    } catch {
        Write-Host "  [$($v.name)] PUT 被拒：$($_.Exception.Message)" -ForegroundColor Yellow
        continue
    }
    $got = @($try.applicationPackages)[0]
    Write-Host "  [$($v.name)] -> $($got.fileName)  fileStatus=$($got.fileStatus)" -ForegroundColor DarkGray
    if ($got.fileName -eq $pkgName) { $sub2 = $try; break }
}

if (-not $sub2) {
    throw "试了 $($variants.Count) 种改法都没能让 API 接受新包名（目标 $pkgName）。去 Partner Center 手动删掉这个提交后重跑。"
}

$slot2 = @($sub2.applicationPackages)[0]
$uploadUrl = $slot2.fileUploadUrl
if (-not $uploadUrl) { $uploadUrl = $sub2.fileUploadUrl }
if (-not $uploadUrl) { throw 'PUT 之后没拿到 fileUploadUrl' }
Write-Host '  已登记文件名，拿到上传地址' -ForegroundColor DarkGray

# ── 5. 传包（BlockBlob PUT）───────────────────────────────────────────
Write-Host '上传包体…' -ForegroundColor DarkGray
$bytes = [System.IO.File]::ReadAllBytes($pkg)
$upHeaders = @{
    'x-ms-blob-type' = 'BlockBlob'
    'x-ms-version'   = '2020-04-08'
    'Content-Type'   = 'application/octet-stream'
}
Invoke-WebRequest -Method Put -Uri $uploadUrl -Headers $upHeaders -Body $bytes -UseBasicParsing | Out-Null
Write-Host "  已上传 $([math]::Round($bytes.Length / 1MB, 2)) MB" -ForegroundColor DarkGray

# 上传后再核对一次：确认商店记下的就是这个包，且已经标成 Uploaded。
$afterUp = Invoke-RestMethod -Method Get -Uri "$subsUri/$($sub.id)" -Headers $headers
$slotUp = @($afterUp.applicationPackages)[0]
Write-Host "  上传后核对：$($slotUp.fileName)  fileStatus=$($slotUp.fileStatus)  version=$($slotUp.version)" -ForegroundColor DarkGray
if ($slotUp.fileName -ne $pkgName) {
    throw "上传后商店记的包名是 $($slotUp.fileName)，不是 $pkgName —— 停止提交，避免把旧包发出去。"
}
if ($slotUp.fileStatus -ne 'Uploaded') {
    Write-Host "::warning::包状态是 $($slotUp.fileStatus) 而不是 Uploaded，提交可能失败"
}

# ── 6. commit ─────────────────────────────────────────────────────────
if ($DryRun) {
    Write-Host ''
    Write-Host 'DryRun：草稿已建好并挂上包，但没有 commit。' -ForegroundColor Yellow
    Write-Host "去这里核对：https://partner.microsoft.com/dashboard/products/$ProductId/submissions/$($sub.id)" -ForegroundColor Yellow
    return
}

Write-Host '提交认证…' -ForegroundColor DarkGray
Invoke-RestMethod -Method Post -Uri "$subsUri/$($sub.id)/commit" -Headers $headers -Body '{}' -ContentType 'application/json' | Out-Null

# 提交后再核对一次，防止「跑绿了但发出去的是旧包」这种静默错误。
try {
    $check = Invoke-RestMethod -Method Get -Uri "$subsUri/$($sub.id)" -Headers $headers
    $gotPkg = @($check.applicationPackages)[0]
    Write-Host "  已提交的包：$($gotPkg.fileName)  version=$($gotPkg.version)  status=$($check.status)" -ForegroundColor DarkGray
    if ($gotPkg.fileName -and $gotPkg.fileName -ne $pkgName) {
        Write-Host "::warning::提交里挂的包名是 $($gotPkg.fileName)，不是 $pkgName —— 去 Partner Center 核对一下"
    }
} catch {
    Write-Host "  （提交后核对失败，不影响提交本身：$($_.Exception.Message)）" -ForegroundColor DarkGray
}

Write-Host ''
Write-Host "已提交。状态页：https://partner.microsoft.com/dashboard/products/$ProductId/submissions" -ForegroundColor Green
