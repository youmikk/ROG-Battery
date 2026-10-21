# 把仓库里的 rog-battery.exe + AppxManifest 打成一个 .msix
#
# 用法（在本目录下）:
#   powershell -NoProfile -ExecutionPolicy Bypass -File build-msix.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File build-msix.ps1 -Version 1.0.1.0
#   powershell -NoProfile -ExecutionPolicy Bypass -File build-msix.ps1 -Sign      # 本地测试用自签名
#   powershell -NoProfile -ExecutionPolicy Bypass -File build-msix.ps1 -Clean
#
# 【这个脚本做什么】
#   1. 把 AppxManifest.xml.template 里的 @占位符@ 换成 package.props.psd1 里的值
#   2. 建一个干净的暂存目录：exe + manifest + Assets\*.png
#   3. 调 makeappx.exe pack
#   4. 可选：用自签名证书签一下，好在本地 Add-AppxPackage 装起来测
#
# 【为什么暂存目录而不是直接打仓库根目录】
#   MSIX 会把目录里**所有**文件都塞进包。仓库根目录有 .git、README.md、
#   rog-battery.cs 源码、history.tsv（用户自己的历史数据）—— 打进去既臃肿又
#   泄隐私。所以必须显式挑文件。
#
# 【编码注意（踩过坑）】
#   本文件必须以 UTF-8 **带 BOM** 保存。PowerShell 5.1 读「UTF-8 无 BOM」的 .ps1
#   时按系统 ANSI 代码页（本机 GBK）解码，中文注释会被解成乱码，乱码里的字节可能
#   吞掉后续行，表现为「明明代码没写错却语法错误」「变量莫名是空」。

[CmdletBinding()]
param(
    [string] $Version,
    [switch] $Sign,
    [switch] $Clean
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$root    = $PSScriptRoot
$repo    = Split-Path $root -Parent
$stage   = Join-Path $root 'stage'
$pkgDir  = Join-Path $root 'AppPackages'
$exe     = Join-Path $repo 'rog-battery.exe'

# ---------------- 清理 ----------------
if ($Clean) {
    foreach ($d in @($stage, $pkgDir)) {
        if (Test-Path $d) { Remove-Item $d -Recurse -Force; Write-Host "  removed $d" }
    }
    Write-Host 'Clean done.'
    return
}

# ---------------- 读配置 ----------------
$propsPath = Join-Path $root 'package.props.psd1'
if (-not (Test-Path $propsPath)) { throw "找不到 $propsPath" }
$props = Import-PowerShellDataFile -Path $propsPath

if ($Version) { $props.Version = $Version }

# 版本号必须是 4 段数字，且第一段不能是 0（Store 要求 >= 1.0.0.0）
if ($props.Version -notmatch '^\d+\.\d+\.\d+\.\d+$') {
    throw "Version 必须是 4 段数字，例如 1.0.0.0（现在是 '$($props.Version)'）"
}
if ($props.Version -match '^0\.') {
    throw "Version 第一段不能为 0（Store 要求 >= 1.0.0.0，现在是 $($props.Version)）"
}

# ---------------- 找 makeappx ----------------
# 优先用 PATH，其次用 package.props.psd1 里记的绝对路径（本机把 SDK BuildTools
# 解在 D:\codex\_tools 下，没装 Windows SDK）。
function Find-MakeAppx {
    $cmd = Get-Command makeappx.exe -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    if ($props.MakeAppxPath -and (Test-Path $props.MakeAppxPath)) { return $props.MakeAppxPath }
    return $null
}

$makeappx = Find-MakeAppx
if (-not $makeappx) {
    throw @"
找不到 makeappx.exe。两条路：
  1. 装 Windows SDK（含 MakeAppx），或
  2. 在 package.props.psd1 里把 MakeAppxPath 指向 makeappx.exe 的绝对路径。
"@
}
Write-Host "makeappx: $makeappx"

# ---------------- 检查 exe ----------------
if (-not (Test-Path $exe)) {
    throw "找不到 $exe —— 先跑一次 build.cmd 编译。"
}
$exeSize = (Get-Item $exe).Length
Write-Host ("exe: {0} ({1:N0} bytes)" -f $exe, $exeSize)

# ---------------- 搭暂存目录 ----------------
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Path $stage | Out-Null
New-Item -ItemType Directory -Path (Join-Path $stage 'Assets') | Out-Null

# exe
Copy-Item $exe (Join-Path $stage 'rog-battery.exe')

# Assets：只拷 manifest 里点名的那些，顺手把 make-assets 生成的全部带上
# （targetsize 变体是给不同缩放比例用的，manifest 只点名一个，但 Windows
#  会按 targetsize-<n>.png 的命名自动挑更合适的，所以全带上更好）
$assetSrc = Join-Path $root 'Assets'
if (-not (Test-Path $assetSrc)) {
    throw "找不到 $assetSrc —— 先跑 make-assets.ps1 生成 logo。"
}
Get-ChildItem $assetSrc -Filter *.png | ForEach-Object {
    Copy-Item $_.FullName (Join-Path (Join-Path $stage 'Assets') $_.Name)
}
$assetCount = (Get-ChildItem (Join-Path $stage 'Assets')).Count
Write-Host "assets: $assetCount png"

# ---------------- 生成 manifest ----------------
$tpl = Get-Content (Join-Path $root 'AppxManifest.xml.template') -Raw -Encoding UTF8
$map = @{
    '@IDENTITY_NAME@'          = $props.IdentityName
    '@PUBLISHER@'              = $props.Publisher
    '@PUBLISHER_DISPLAY_NAME@' = $props.PublisherDisplayName
    '@DISPLAY_NAME@'           = $props.DisplayName
    '@SHORT_NAME@'             = $props.ShortName
    '@DESCRIPTION@'            = $props.Description
    '@VERSION@'                = $props.Version
    '@STARTUP_TASK_ID@'        = $props.StartupTaskId
}
foreach ($k in $map.Keys) {
    # -replace 用正则，$ 在替换串里有特殊含义，所以走普通字符串替换
    $tpl = $tpl.Replace($k, [string] $map[$k])
}

# 检查还有没有漏替换的占位符 —— 漏一个 makeappx 就会报难懂的 XML 校验错
$leftover = [regex]::Matches($tpl, '@[A-Z_]+@')
if ($leftover.Count -gt 0) {
    $names = ($leftover | ForEach-Object { $_.Value } | Sort-Object -Unique) -join ', '
    throw "模板里还有没替换的占位符: $names（检查 package.props.psd1 是否缺键）"
}

# 带 BOM 写出：Windows 的 manifest 解析器对无 BOM 的 UTF-8 也能吃，
# 但带 BOM 可以避免中文 Description 在某些工具里被当 ANSI 读出乱码。
$manifestPath = Join-Path $stage 'AppxManifest.xml'
[System.IO.File]::WriteAllText($manifestPath, $tpl, (New-Object System.Text.UTF8Encoding($true)))
Write-Host "manifest: $manifestPath"

# 提前用 XML 解析器验一遍，比 makeappx 报错好懂
try {
    [xml] $null = Get-Content $manifestPath -Raw -Encoding UTF8
} catch {
    throw "生成的 AppxManifest.xml 不是合法 XML: $($_.Exception.Message)"
}

# ---------------- pack ----------------
if (-not (Test-Path $pkgDir)) { New-Item -ItemType Directory -Path $pkgDir | Out-Null }

$msixName = '{0}_{1}_x64.msix' -f $props.IdentityName, $props.Version
$msixPath = Join-Path $pkgDir $msixName
if (Test-Path $msixPath) { Remove-Item $msixPath -Force }

Write-Host ''
Write-Host '=== packing ==='
& $makeappx pack /d $stage /p $msixPath /o
if ($LASTEXITCODE -ne 0) { throw "makeappx 失败，退出码 $LASTEXITCODE" }

if (-not (Test-Path $msixPath)) { throw "makeappx 说成功了但 $msixPath 不存在" }
$msixSize = (Get-Item $msixPath).Length
Write-Host ''
Write-Host ("packed: {0} ({1:N0} bytes)" -f $msixPath, $msixSize)

# ---------------- 可选：自签名 ----------------
# 仅供本地测试安装。这个证书没有任何公共信任，别人装会一直弹警告，
# 也绕不过 SmartScreen —— 它的唯一用途是让你自己 Add-AppxPackage 起来看效果。
if ($Sign) {
    $signtool = $null
    $cmd = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($cmd) { $signtool = $cmd.Source }
    elseif ($props.SignToolPath -and (Test-Path $props.SignToolPath)) { $signtool = $props.SignToolPath }

    if (-not $signtool) {
        Write-Warning '找不到 signtool.exe，跳过签名。本地测试可以用 -Sign 前先装 SDK。'
    } else {
        $certSubject = $props.Publisher

        # 找现成的（按 Subject 匹配），没有再建
        $cert = Get-ChildItem Cert:\CurrentUser\My |
                Where-Object { $_.Subject -eq $certSubject -and $_.HasPrivateKey } |
                Select-Object -First 1

        if (-not $cert) {
            Write-Host "creating self-signed cert: $certSubject"
            # KeyUsage DigitalSignature + Type CodeSigning 是签名证书的最小要求
            $cert = New-SelfSignedCertificate `
                -Subject $certSubject `
                -Type CodeSigningCert `
                -KeyUsage DigitalSignature `
                -CertStoreLocation 'Cert:\CurrentUser\My' `
                -NotAfter (Get-Date).AddYears(3)
        }
        Write-Host ("cert: {0}  thumbprint {1}" -f $cert.Subject, $cert.Thumbprint)

        # signtool 找证书要的是 SHA1 指纹
        & $signtool sign /fd SHA256 /sha1 $cert.Thumbprint /td SHA256 $msixPath
        if ($LASTEXITCODE -ne 0) { throw "signtool 失败，退出码 $LASTEXITCODE" }

        Write-Host ''
        Write-Host '注意：自签名证书要装进「受信任的根证书颁发机构」才能 Add-AppxPackage。'
        Write-Host '  以管理员 PowerShell 跑：'
        Write-Host ("    Export-Certificate -Cert Cert:\CurrentUser\My\{0} -FilePath `$env:TEMP\rb-test.cer" -f $cert.Thumbprint)
        Write-Host '    Import-Certificate -FilePath $env:TEMP\rb-test.cer -CertStoreLocation Cert:\LocalMachine\Root'
        Write-Host ''
        Write-Host '  测完务必清干净（留着根证书 = 任何用这张私钥签的东西都能在本机静默安装）：'
        Write-Host ("    Remove-Item Cert:\LocalMachine\Root\{0}" -f $cert.Thumbprint)
        Write-Host ("    Remove-Item Cert:\CurrentUser\My\{0}" -f $cert.Thumbprint)
        Write-Host '  再删掉测试包：'
        Write-Host '    Get-AppxPackage -Name <包名> | Remove-AppxPackage'
        Write-Host ("  安装：Add-AppxPackage '{0}'" -f $msixPath)
    }
}

Write-Host ''
Write-Host '=== done ==='
Write-Host ("  {0}" -f $msixPath)
Write-Host ''
Write-Host '提交 Store 时：'
Write-Host '  - 提这个**未签名**的 .msix（Store 会用自己的证书重新签），'
Write-Host '  - 但 Publisher 必须改成 Partner Center 产品标识页给出的原值，'
Write-Host '  - 用 -Sign 打出来的包**不要**提交，那是本地测试用的。'
