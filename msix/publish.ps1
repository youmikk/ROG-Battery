# 一条命令发版到 Microsoft Store（第二次及以后用这个）
#
# 第一次必须手动：在 Partner Center 里建好产品、填完列表、传一次包、过认证。
# 之后每次更新都可以用这个脚本走「Microsoft Store Developer CLI」自动上传：
#   重新编译 → 重新打 msix → 重新打 msixupload → msstore publish
#
# 前置（只做一次）：
#   1. 装 CLI：
#        winget install --id Microsoft.StoreDeveloperCLI -e
#      （装完重开一个终端，`msstore --version` 能出版本号）
#   2. 在 Azure AD（Entra ID）里建一个应用注册，给它 Partner Center 的 Manager 角色，
#      拿到 tenantId / clientId / clientSecret，sellerId 在 Partner Center
#      「帐户设置 → 组织标识 → 卖方 ID」。
#   3. 配一次凭据（两种任选）：
#        a) 环境变量：MSTORE_TENANT_ID / MSTORE_SELLER_ID / MSTORE_CLIENT_ID / MSTORE_CLIENT_SECRET
#        b) 交互式：msstore reconfigure
#
# 用法：
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\publish.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\publish.ps1 -DryRun
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\publish.ps1 -SkipBuild -Version 1.1.1.0
#
# 注意：CLI 的开关名以 `msstore publish --help` 为准（不同版本略有差异）。

[CmdletBinding()]
param(
    [string]$ProductId = '9NRC8HCD4BZ7',
    [string]$Version = '',
    [switch]$SkipBuild,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path

# ── 1. 版本号 ────────────────────────────────────────────────
if (-not $Version) {
    $props = Import-PowerShellDataFile (Join-Path $here 'package.props.psd1')
    $Version = $props.Version
}
$pkgName = "Sonder.RBattery_${Version}_x64"
$upload = Join-Path $here "AppPackages\$pkgName.msixupload"
Write-Host "版本：$Version   产品：$ProductId" -ForegroundColor Cyan

# ── 2. 编译 + 打包 ───────────────────────────────────────────
if (-not $SkipBuild) {
    Write-Host ''
    Write-Host '[1/3] 编译 release …' -ForegroundColor Cyan
    $cargoHome = 'D:\dsh\tools\cargo'
    $env:RUSTUP_HOME = 'D:\dsh\tools\rustup'
    $env:CARGO_HOME = $cargoHome
    Get-Process rog-mouse-ctl -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 300
    & (Join-Path $cargoHome 'bin\cargo.exe') build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build 失败' }

    Write-Host ''
    Write-Host '[2/3] 打 msix（含校验）…' -ForegroundColor Cyan
    $env:TEMP = Join-Path $here 'scratch\tmp'
    $env:TMP = $env:TEMP
    New-Item -ItemType Directory -Force -Path $env:TEMP | Out-Null
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $here 'build-msix.ps1') -Verify -Version $Version
    if ($LASTEXITCODE -ne 0) { throw 'build-msix.ps1 失败（校验没过就不要提交）' }

    Write-Host ''
    Write-Host '[3/3] 打 msixupload …' -ForegroundColor Cyan
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $here 'make-upload.ps1') -Version $Version
    if ($LASTEXITCODE -ne 0) { throw 'make-upload.ps1 失败' }
}

if (-not (Test-Path $upload)) {
    throw "找不到 $upload —— 先去掉 -SkipBuild 跑一次完整打包"
}
Write-Host ''
Write-Host ("待提交：{0}  ({1:N0} bytes)" -f $upload, (Get-Item $upload).Length) -ForegroundColor Green

# ── 3. 提交 ─────────────────────────────────────────────────
$cli = (Get-Command msstore -ErrorAction SilentlyContinue).Source
if (-not $cli) {
    Write-Host ''
    Write-Host '没装 Microsoft Store Developer CLI，无法自动提交。' -ForegroundColor Yellow
    Write-Host '装一次就行（之后每次发版都不用再管）：' -ForegroundColor Yellow
    Write-Host '    winget install --id Microsoft.StoreDeveloperCLI -e' -ForegroundColor Yellow
    Write-Host ''
    Write-Host "装好后重跑本脚本；或者手动把这个文件拖进 Partner Center：$upload" -ForegroundColor Yellow
    exit 2
}

$cliArgs = @('publish', '-id', $ProductId, '-i', $upload)
if ($DryRun) { $cliArgs += '--noCommit' }   # 只建提交、不点提交认证，方便先核对

Write-Host ''
Write-Host ('执行：msstore ' + ($cliArgs -join ' ')) -ForegroundColor Cyan
& $cli @cliArgs
$rc = $LASTEXITCODE
Write-Host ''
if ($rc -eq 0) {
    Write-Host '已提交。去 Partner Center 看「提交」页状态；认证通常几小时到一天。' -ForegroundColor Green
    if ($DryRun) { Write-Host '（-DryRun：这次只建了提交草稿，没点「提交认证」）' -ForegroundColor Yellow }
}
else {
    Write-Host "msstore 返回 $rc。常见原因：凭据没配、产品 ID 不对、版本号没递增。" -ForegroundColor Red
    Write-Host '排查：msstore reconfigure  /  msstore apps list' -ForegroundColor Red
}
exit $rc
