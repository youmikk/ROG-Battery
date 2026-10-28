<#
.SYNOPSIS
    把商店自动发布所需的凭据写进 GitHub 仓库的 Secrets / Variables。

.DESCRIPTION
    对应 docs/store-publishing-setup.md 的「第 5 步」。
    4 个 Secret（租户 / 客户端 ID / 客户端密码 / 卖家 ID）+ 1 个 Variable（产品 ID）。

    不带参数运行会逐个交互询问；带 -DryRun 只打印将要执行的 gh 命令，不写任何东西。

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File tools\setup-store-secrets.ps1 -DryRun

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File tools\setup-store-secrets.ps1 `
        -TenantId 00000000-1111-2222-3333-444444444444 `
        -ClientId 55555555-6666-7777-8888-999999999999 `
        -SellerId 1307ADA3-FCC0-4C75-81A2-9CC947003BD7 `
        -ProductId 9NRC8HCD4BZ7
#>
[CmdletBinding()]
param(
    [string]$TenantId,
    [string]$ClientId,
    [string]$ClientSecret,
    [string]$SellerId,
    [string]$ProductId,
    [string]$Repo,
    [switch]$SkipProductId,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'

# gh 靠当前目录认仓库；从仓库外调用脚本时先挪到脚本自己所在目录（tools\ 的上一层）
if ($PSScriptRoot) { Push-Location -LiteralPath (Split-Path -Parent $PSScriptRoot) }

function Write-Step([string]$Text) {
    Write-Host ""
    Write-Host "== $Text" -ForegroundColor Cyan
}

function Write-Ok([string]$Text) {
    Write-Host "   [OK] $Text" -ForegroundColor Green
}

function Write-Skip([string]$Text) {
    Write-Host "   [--] $Text" -ForegroundColor DarkGray
}

# ---------------------------------------------------------------- gh 可用性

$gh = Get-Command gh -ErrorAction SilentlyContinue
if (-not $gh) {
    $candidates = @(
        'C:\Program Files\GitHub CLI\gh.exe',
        "$env:LOCALAPPDATA\Programs\GitHub CLI\gh.exe"
    )
    foreach ($c in $candidates) {
        if (Test-Path -LiteralPath $c) { $gh = Get-Item -LiteralPath $c; break }
    }
}
if (-not $gh) {
    throw "找不到 gh.exe。先装 GitHub CLI：winget install --id GitHub.cli -e"
}

Write-Step "环境"
Write-Ok "gh = $($gh.Source)"

if (-not $Repo) {
    try {
        $Repo = (& $gh.Source repo view --json nameWithOwner --jq '.nameWithOwner' 2>$null)
    } catch { $Repo = $null }
    if (-not $Repo) {
        throw "取不到当前仓库名（不在 git 仓库里，或 gh 未登录）。用 -Repo owner/name 显式指定。"
    }
}
Write-Ok "仓库 = $Repo"

if ($DryRun) {
    Write-Host "   (DryRun：下面只打印命令，不会真的写入)" -ForegroundColor Yellow
}

# ---------------------------------------------------------------- 收集参数

function Ask-Value([string]$Name, [string]$Hint, [string]$Current, [string]$Sample, [switch]$Secret) {
    if ($Current) { return $Current }
    if ($DryRun) { return $Sample }
    Write-Host ""
    Write-Host "   $Name" -ForegroundColor White
    Write-Host "     从哪拿：$Hint" -ForegroundColor DarkGray
    if ($Secret) {
        $sec = Read-Host "     粘贴值（输入不回显）" -AsSecureString
        $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($sec)
        try {
            return [Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr)
        } finally {
            [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
        }
    }
    return (Read-Host "     粘贴值")
}

Write-Step "凭据"

$TenantId = Ask-Value 'AZURE_AD_TENANT_ID' `
    'Entra 管理中心 → 标识 → 概述 → 租户 ID' $TenantId `
    -Sample '11111111-1111-1111-1111-111111111111'

$ClientId = Ask-Value 'AZURE_AD_APPLICATION_CLIENT_ID' `
    'Entra → 应用注册 → 你的应用 → 概览 → 应用程序(客户端) ID' $ClientId `
    -Sample '22222222-2222-2222-2222-222222222222'

$ClientSecret = Ask-Value 'AZURE_AD_APPLICATION_SECRET' `
    '同一应用 → 证书和密码 → 客户端密码 → 复制「值」（不是密码 ID）' $ClientSecret `
    -Sample 'dummy~client-secret-value-0000000000' -Secret

$SellerId = Ask-Value 'SELLER_ID' `
    '合作伙伴中心 → 设置 → 帐户设置 → 标识符 → 发布者 ID' $SellerId `
    -Sample '1307ADA3-FCC0-4C75-81A2-9CC947003BD7'

if (-not $SkipProductId -and -not $ProductId) {
    if ($DryRun) {
        $ProductId = '9NRC8HCD4BZ7'
    } else {
        Write-Host ""
        Write-Host "   MSSTORE_PRODUCT_ID" -ForegroundColor White
        Write-Host "     首个版本还没手动过审就留空（回车），之后填 9NRC8HCD4BZ7" -ForegroundColor DarkGray
        $ProductId = Read-Host "     粘贴值（可留空）"
    }
}

# ---------------------------------------------------------------- 写入

$secrets = [ordered]@{
    AZURE_AD_TENANT_ID             = $TenantId
    AZURE_AD_APPLICATION_CLIENT_ID = $ClientId
    AZURE_AD_APPLICATION_SECRET    = $ClientSecret
    SELLER_ID                      = $SellerId
}

Write-Step "Secrets"
foreach ($name in $secrets.Keys) {
    $value = $secrets[$name]
    if (-not $value) {
        Write-Skip "$name 为空，跳过"
        continue
    }
    if ($DryRun) {
        Write-Host "   gh secret set $name --repo $Repo --body <$($value.Length) 字符>" -ForegroundColor DarkYellow
        continue
    }
    & $gh.Source secret set $name --repo $Repo --body $value
    if ($LASTEXITCODE -ne 0) { throw "gh secret set $name 失败（退出码 $LASTEXITCODE）" }
    Write-Ok "$name 已写入（$($value.Length) 字符）"
}

Write-Step "Variables"
if ($ProductId) {
    if ($DryRun) {
        Write-Host "   gh variable set MSSTORE_PRODUCT_ID --repo $Repo --body $ProductId" -ForegroundColor DarkYellow
    } else {
        & $gh.Source variable set MSSTORE_PRODUCT_ID --repo $Repo --body $ProductId
        if ($LASTEXITCODE -ne 0) { throw "gh variable set MSSTORE_PRODUCT_ID 失败（退出码 $LASTEXITCODE）" }
        Write-Ok "MSSTORE_PRODUCT_ID = $ProductId（商店自动提交已开启）"
    }
} else {
    Write-Skip "MSSTORE_PRODUCT_ID 未设置 —— 工作流只打包，不提交商店"
}

if ($DryRun) {
    Write-Host ""
    Write-Host "DryRun 结束，什么都没写。" -ForegroundColor Yellow
    exit 0
}

# ---------------------------------------------------------------- 自检

Write-Step "自检"
try {
    $listed = (& $gh.Source secret list --repo $Repo) -join "`n"
    foreach ($name in $secrets.Keys) {
        if ($listed -match [regex]::Escape($name)) { Write-Ok "secret 存在：$name" }
        else { Write-Host "   [!!] secret 缺失：$name" -ForegroundColor Red }
    }
} catch {
    Write-Host "   [!!] 列 secrets 失败：$($_.Exception.Message)" -ForegroundColor Red
}

Write-Host ""
Write-Host "下一步：" -ForegroundColor Cyan
Write-Host "  1. 本地先验一遍凭据（不用等 CI）："
Write-Host "       msstore reconfigure --tenantId `$env:TENANT --sellerId `$env:SELLER --clientId `$env:CLIENT --clientSecret `$env:SECRET"
Write-Host "       msstore apps list        # 能列出产品说明角色授对了"
Write-Host "  2. 确认首个版本已经手动过审，再让 CI 接管："
Write-Host "       git tag v1.1.3; git push origin v1.1.3"
Write-Host ""
