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
try {
    $subs = Invoke-RestMethod -Method Get -Uri $subsUri -Headers $headers
} catch {
    $subs = @()
}
foreach ($s in @($subs)) {
    if ($null -eq $s -or -not $s.id) { continue }
    # 只删草稿。已发布/正在认证的提交绝不能删。
    if ($s.status -ne 'PendingCommit') {
        Write-Host "跳过提交 $($s.id)（状态 $($s.status)，不是草稿）" -ForegroundColor DarkGray
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

# ── 3. 建新草稿 ───────────────────────────────────────────────────────
Write-Host '创建草稿…' -ForegroundColor DarkGray
# 文档明确：这个方法不要请求体。带 {} 会被判 InvalidParameterValue
# "The size of Listings must be 1 or more"。
$sub = Invoke-RestMethod -Method Post -Uri $subsUri -Headers $headers
Write-Host "  id=$($sub.id)  status=$($sub.status)" -ForegroundColor DarkGray

$pkgs = @($sub.applicationPackages)
if ($pkgs.Count -eq 0) { throw '草稿里没有 applicationPackages 槽位，无法挂包' }
Write-Host "  现有包槽：$($pkgs[0].fileName)  fileStatus=$($pkgs[0].fileStatus)" -ForegroundColor DarkGray

# ── 4. 改 fileName 再 PUT（这一步返回 fileUploadUrl）───────────────────
$pkgs[0].fileName = $pkgName
$sub.applicationPackages = $pkgs
$body = $sub | ConvertTo-Json -Depth 32
$sub2 = Invoke-RestMethod -Method Put -Uri "$subsUri/$($sub.id)" -Headers $headers -Body $body -ContentType 'application/json'
$uploadUrl = @($sub2.applicationPackages)[0].fileUploadUrl
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

# ── 6. commit ─────────────────────────────────────────────────────────
if ($DryRun) {
    Write-Host ''
    Write-Host 'DryRun：草稿已建好并挂上包，但没有 commit。' -ForegroundColor Yellow
    Write-Host "去这里核对：https://partner.microsoft.com/dashboard/products/$ProductId/submissions/$($sub.id)" -ForegroundColor Yellow
    return
}

Write-Host '提交认证…' -ForegroundColor DarkGray
Invoke-RestMethod -Method Post -Uri "$subsUri/$($sub.id)/commit" -Headers $headers -Body '{}' -ContentType 'application/json' | Out-Null
Write-Host ''
Write-Host "已提交。状态页：https://partner.microsoft.com/dashboard/products/$ProductId/submissions" -ForegroundColor Green
