# 把 ..\target\release\rog-mouse-ctl.exe + AppxManifest 打成一个 .msix
#
# 用法（在本目录下）:
#   powershell -NoProfile -ExecutionPolicy Bypass -File build-msix.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File build-msix.ps1 -Version 1.0.1.0
#   powershell -NoProfile -ExecutionPolicy Bypass -File build-msix.ps1 -Verify
#   powershell -NoProfile -ExecutionPolicy Bypass -File build-msix.ps1 -Sign -CertPath D:\secret\sonder.pfx -CertPassword '***'
#   powershell -NoProfile -ExecutionPolicy Bypass -File build-msix.ps1 -Sign -Thumbprint <SHA1>
#   powershell -NoProfile -ExecutionPolicy Bypass -File build-msix.ps1 -Clean
#
# 【这个脚本做什么】
#   0. 检查版本号是否大于本机已安装的同 identity 版本（撞车当场拦住）
#   1. 检查 ..\target\release\rog-mouse-ctl.exe 存在（不存在就让你先 cargo build --release）
#   2. 把 AppxManifest.xml.template 里的 @占位符@ 换成 package.props.psd1 里的值
#   3. 建一个干净的暂存目录 stage\：exe + manifest + Assets\*.png
#   4. 调 makeappx.exe pack -> AppPackages\Sonder.RBattery_<版本>_x64.msix
#   5. 可选 -Verify：用 makeappx unpack 把产物解回 scratch\unpack，逐项核对
#   6. 可选 -Sign：用你已有的上架证书签名（**不会**自动生成证书）
#
# 【为什么不自动生成自签名证书】
#   用户已有可上架的正式证书；商店提交时微软会用自己的证书重签，本地签名只是
#   为了 Add-AppxPackage 本地测试。自动建证书只会往证书库里塞垃圾，还要手动
#   删根证书，风险大于收益。所以要签名必须显式给证书。
#
# 【为什么暂存目录而不是直接打仓库根目录】
#   MSIX 会把目录里**所有**文件都塞进包。仓库根目录有 .git、Cargo.toml、src\、
#   target\ —— 打进去既臃肿又可能泄隐私（源码/编译缓存）。所以必须显式挑文件。
#
# 【编码注意（踩过坑）】
#   本文件必须以 UTF-8 **带 BOM** 保存。PowerShell 5.1 读「UTF-8 无 BOM」的 .ps1
#   时按系统 ANSI 代码页（本机 GBK）解码，中文注释会被解成乱码，乱码里的字节可能
#   吞掉后续行，表现为「明明代码没写错却语法错误」「变量莫名是空」。
#   同理，读带中文的 XML/模板一律走 [System.IO.File]::ReadAllBytes + UTF8.GetString，
#   不要用 Get-Content -Raw（无 -Encoding 时按 GBK 解，中文必毁）。

[CmdletBinding()]
param(
    [string] $Version,
    [switch] $Sign,
    [string] $CertPath,
    [string] $CertPassword,
    [string] $Thumbprint,
    [string] $TimestampUrl,
    [switch] $SkipInstalledVersionCheck,
    [switch] $Verify,
    [switch] $Clean
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$root   = $PSScriptRoot
$repo   = Split-Path $root -Parent
$stage  = Join-Path $root 'stage'
$pkgDir = Join-Path $root 'AppPackages'

function Write-Step([string] $text) {
    Write-Host ''
    Write-Host "=== $text ===" -ForegroundColor Cyan
}

# 读 UTF-8 文本并剥掉 BOM（GetString 会把 BOM 解成 U+FEFF，带进 XML 会
# 「Data at the root level is invalid」）
function Read-Utf8NoBom([string] $path) {
    $text = [System.Text.Encoding]::UTF8.GetString([System.IO.File]::ReadAllBytes($path))
    return $text.TrimStart([char] 0xFEFF)
}

# ---------------- 清理 ----------------
if ($Clean) {
    foreach ($d in @($stage, $pkgDir, (Join-Path $root 'scratch'))) {
        if (Test-Path $d) { Remove-Item $d -Recurse -Force; Write-Host "  removed $d" }
    }
    Write-Host 'Clean done.'
    return
}

# ---------------- 读配置 ----------------
$propsPath = Join-Path $root 'package.props.psd1'
if (-not (Test-Path $propsPath)) {
    throw "找不到参数文件 $propsPath —— msix 目录不完整，重新检查打包资产。"
}
$props = Import-PowerShellDataFile -Path $propsPath

# CI（GitHub Actions）用 Variables 覆盖身份字段；本地不设这些环境变量就完全不生效。
# 为什么允许覆盖：身份字段跟着账号走，不该写死在一个仓库文件里，但本地又要能一键打包。
foreach ($pair in @(
        @{ Env = 'MSSTORE_PACKAGE_IDENTITY_NAME'; Key = 'IdentityName' }
        @{ Env = 'MSSTORE_PUBLISHER'; Key = 'Publisher' }
        @{ Env = 'MSSTORE_PUBLISHER_DISPLAY_NAME'; Key = 'PublisherDisplayName' }
    )) {
    $override = [Environment]::GetEnvironmentVariable($pair.Env)
    if ($override) {
        $props[$pair.Key] = $override
        Write-Host ("  {0} <- {1}" -f $pair.Key, $pair.Env)
    }
}

if ($Version) { $props.Version = $Version }

# 版本号必须是 4 段数字，且第一段不能是 0（Store 要求 >= 1.0.0.0）
if ($props.Version -notmatch '^\d+\.\d+\.\d+\.\d+$') {
    throw "Version 必须是 4 段数字，例如 1.0.0.0（现在是 '$($props.Version)'）。用 -Version 1.0.1.0 指定。"
}
if ($props.Version -match '^0\.') {
    throw "Version 第一段不能为 0（Store 要求 >= 1.0.0.0，现在是 $($props.Version)）"
}

# ---------------- 版本号撞车检查 ----------------
# 为什么必须有：同一个 identity 下，Store **只接受严格大于**已发布版本的版本号，
# 本地 Add-AppxPackage 遇到同版本或降级也会直接拒绝（0x80073CF9）。
# 忘了升版本是最容易犯的错，等商店审核失败才发现太晚，所以在这里当场拦住。
#
# 机器上没装这个包（Get-AppxPackage 返回空）就跳过 —— 换机器/新账号打包不该报错。
if (-not $SkipInstalledVersionCheck) {
    $installed = $null
    try {
        $installed = @(Get-AppxPackage -Name $props.IdentityName -ErrorAction Stop)
    } catch {
        # Get-AppxPackage 在某些受限上下文里不可用，跳过而不是报错
        Write-Host "（跳过已装版本检查：Get-AppxPackage 不可用 —— $($_.Exception.Message)）"
        $installed = @()
    }

    if ($installed.Count -gt 0) {
        $installedVer = ($installed | ForEach-Object { $_.Version } |
                         Sort-Object -Descending | Select-Object -First 1)
        $newVer = [version] $props.Version

        Write-Host ("已装版本: {0}   本次打包: {1}" -f $installedVer, $newVer)

        if ($newVer -le [version] $installedVer) {
            $cmp = if ($newVer -eq [version] $installedVer) { '等于' } else { '小于' }
            throw @"
版本号撞车：本次打包的 $newVer $cmp 本机已安装的 $installedVer。

同一个 identity（$($props.IdentityName)）下，Store 只接受**严格大于**已发布版本的号，
本地 Add-AppxPackage 遇到同版本/降级也会拒绝（0x80073CF9），所以这个包发不出去也装不上。

改法（二选一）：
  1. 用参数指定更高版本：
       -Version $(([version] $installedVer).Major).$(([version] $installedVer).Minor + 1).0.0
  2. 改 package.props.psd1 里的 Version。

确实需要打这个号（例如只是想看看包结构）？
  -Verify 只是解包自检，仍会被这条拦住；要绕过就加 -SkipInstalledVersionCheck，
  但**带这个参数的包不要提交商店、也不要装到装了老版本的机器上**。
"@
        }
    } else {
        Write-Host "（本机未安装 $($props.IdentityName)，跳过版本撞车检查）"
    }
} else {
    Write-Warning "已跳过版本撞车检查（-SkipInstalledVersionCheck）—— 产物可能无法安装或提交。"
}

$exeName = $props.Executable
if (-not $exeName) {
    throw "package.props.psd1 里缺少 Executable 键（应为 'rog-mouse-ctl.exe'）"
}

# ---------------- 找工具 ----------------
# 优先用 PATH，其次用 package.props.psd1 里记的绝对路径（本机把 SDK BuildTools
# 解在 D:\codex\_tools 下，没装 Windows SDK），最后用备选路径。
function Find-Tool {
    param([string] $Name, [string] $Preferred, [string] $Fallback)
    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    if ($Preferred -and (Test-Path $Preferred)) { return $Preferred }
    if ($Fallback -and (Test-Path $Fallback)) { return $Fallback }
    return $null
}

$makeappx = Find-Tool -Name 'makeappx.exe' `
    -Preferred $props.MakeAppxPath `
    -Fallback $props.FallbackMakeAppxPath

if (-not $makeappx) {
    throw @"
找不到 makeappx.exe。三条路：
  1. 装 Windows SDK（含 MakeAppx），或
  2. 在 package.props.psd1 里把 MakeAppxPath 指向 makeappx.exe 的绝对路径，
  3. 或把 FallbackMakeAppxPath 指向 SDK BuildTools 解出来的 makeappx.exe。
注意：不能只拷 makeappx.exe 单个文件 —— 它靠同目录的 .manifest 做免注册 COM 绑定，
只拷 exe 会报「side-by-side configuration is incorrect」。
"@
}
Write-Host "makeappx: $makeappx"

# ---------------- 检查 exe ----------------
$exe = Join-Path (Join-Path $repo 'target\release') $exeName
if (-not (Test-Path $exe)) {
    throw @"
找不到 $exe

先编译 release 版：
    cd $repo
    cargo build --release

产物应在 $exe
"@
}
$exeSize = (Get-Item $exe).Length
if ($exeSize -eq 0) {
    throw "$exe 是 0 字节 —— 编译产物损坏，重新跑 cargo build --release。"
}
Write-Host ("exe: {0} ({1:N0} bytes)" -f $exe, $exeSize)

# ---------------- 搭暂存目录 ----------------
Write-Step 'staging'
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Path $stage | Out-Null
New-Item -ItemType Directory -Path (Join-Path $stage 'Assets') | Out-Null

Copy-Item $exe (Join-Path $stage $exeName)
$stagedExeSize = (Get-Item (Join-Path $stage $exeName)).Length
if ($stagedExeSize -ne $exeSize) {
    throw "拷贝 exe 到 stage 后大小不一致（源 $exeSize / 目标 $stagedExeSize），磁盘可能有问题。"
}

# Assets：manifest 点名的 + 全部 targetsize 变体
# （targetsize-<n>.png 是给不同缩放比例用的，manifest 只点名一个，但 Windows
#  会按命名自动挑更合适的，所以全带上更好）
$assetSrc = Join-Path $root 'Assets'
if (-not (Test-Path $assetSrc)) {
    throw "找不到 $assetSrc —— Assets 目录缺失，重新拷贝一份 MSIX 素材。"
}
$pngs = @(Get-ChildItem $assetSrc -Filter *.png)
if ($pngs.Count -eq 0) {
    throw "$assetSrc 里没有 .png —— 素材缺失，manifest 会引用到不存在的 logo。"
}
foreach ($p in $pngs) {
    Copy-Item $p.FullName (Join-Path (Join-Path $stage 'Assets') $p.Name)
}
$assetCount = @(Get-ChildItem (Join-Path $stage 'Assets')).Count
Write-Host "assets: $assetCount png"

# manifest 必须点名的若干张，缺一张 makeappx 会报难懂的校验错，
# 这里先自己查一遍，报错才好懂。
$requiredAssets = @(
    'StoreLogo.png',
    'Square44x44Logo.png',
    'Square150x150Logo.png',
    'Square310x310Logo.png',
    'Wide310x150Logo.png',
    'SplashScreen.png'
)
$missing = @($requiredAssets | Where-Object {
    -not (Test-Path (Join-Path (Join-Path $stage 'Assets') $_))
})
if ($missing.Count -gt 0) {
    throw ("Assets 里缺 manifest 点名的文件: " + ($missing -join ', '))
}

# ---------------- 生成 manifest ----------------
Write-Step 'manifest'

$tpl = Read-Utf8NoBom (Join-Path $root 'AppxManifest.xml.template')

$map = @{
    '@IDENTITY_NAME@'          = $props.IdentityName
    '@PUBLISHER@'              = $props.Publisher
    '@PUBLISHER_DISPLAY_NAME@' = $props.PublisherDisplayName
    '@DISPLAY_NAME@'           = $props.DisplayName
    '@SHORT_NAME@'             = $props.ShortName
    '@DESCRIPTION@'            = $props.Description
    '@VERSION@'                = $props.Version
    '@STARTUP_TASK_ID@'        = $props.StartupTaskId
    '@EXECUTABLE@'             = $exeName
}
foreach ($k in $map.Keys) {
    # 用普通字符串替换（不是 -replace），避免值里的 $ 被当正则替换引用
    $tpl = $tpl.Replace($k, [string] $map[$k])
}

# 检查还有没有漏替换的占位符 —— 漏一个 makeappx 就会报难懂的 XML 校验错
$leftover = [regex]::Matches($tpl, '@[A-Z_]+@')
if ($leftover.Count -gt 0) {
    $names = (@($leftover | ForEach-Object { $_.Value } | Sort-Object -Unique)) -join ', '
    throw "模板里还有没替换的占位符: $names（检查 package.props.psd1 是否缺键）"
}

# 带 BOM 写出：Windows 的 manifest 解析器对无 BOM 的 UTF-8 也能吃，
# 但带 BOM 可以避免中文 Description 在某些工具里被当 ANSI 读出乱码。
$manifestPath = Join-Path $stage 'AppxManifest.xml'
[System.IO.File]::WriteAllText($manifestPath, $tpl, (New-Object System.Text.UTF8Encoding($true)))

# 提前用 XML 解析器验一遍，比 makeappx 报错好懂
try {
    [xml] $null = Read-Utf8NoBom $manifestPath
} catch {
    throw "生成的 AppxManifest.xml 不是合法 XML: $($_.Exception.Message)"
}
Write-Host "manifest: $manifestPath"

# ---------------- pack ----------------
Write-Step 'packing'
if (-not (Test-Path $pkgDir)) { New-Item -ItemType Directory -Path $pkgDir | Out-Null }

$msixName = '{0}_{1}_x64.msix' -f $props.IdentityName, $props.Version
$msixPath = Join-Path $pkgDir $msixName
if (Test-Path $msixPath) { Remove-Item $msixPath -Force }

& $makeappx pack /d $stage /p $msixPath /o
if ($LASTEXITCODE -ne 0) {
    throw "makeappx pack 失败，退出码 $LASTEXITCODE。常见原因：exe 名和 manifest 不一致、Assets 缺文件、startupTask 扩展的 Executable 与 Application 不匹配。"
}
if (-not (Test-Path $msixPath)) {
    throw "makeappx 返回 0 但 $msixPath 不存在 —— 检查 /p 参数是否被 shell 改写。"
}
$msixSize = (Get-Item $msixPath).Length
Write-Host ''
Write-Host ("packed: {0} ({1:N0} bytes)" -f $msixPath, $msixSize)

# ---------------- 可选：签名 ----------------
# 商店提交时微软会重新签名，本地签名只是为了 Add-AppxPackage 测试安装。
# **本脚本不会自动生成证书** —— 没有可用证书就明确报错，让你自己决定。
if ($Sign) {
    Write-Step 'signing'

    $signtool = Find-Tool -Name 'signtool.exe' -Preferred $props.SignToolPath -Fallback $null
    if (-not $signtool) {
        throw @"
找不到 signtool.exe，但指定了 -Sign。
把 package.props.psd1 里的 SignToolPath 指向 signtool.exe 的绝对路径，
或者去掉 -Sign 只打未签名包（提交 Store 用的就是这个）。
"@
    }

    $signArgs = @('sign', '/fd', 'SHA256')

    if ($CertPath) {
        if (-not (Test-Path $CertPath)) {
            throw "找不到证书文件 $CertPath（-CertPath 给的是 .pfx 的完整路径）"
        }
        Write-Host "cert file: $CertPath"
        $signArgs += @('/f', $CertPath)
        if ($CertPassword) {
            # 注意：signtool 的 /p 只收明文密码，会在命令行里留下痕迹。
            # 本机测试可接受；共享机器上建议把证书装进证书库，改用 -Thumbprint。
            $signArgs += @('/p', $CertPassword)
        }
    }
    else {
        # 从证书库按 Subject（== manifest Publisher）或显式 -Thumbprint 找。
        # 刻意不 New-SelfSignedCertificate：用户已有上架证书。
        $cert = $null
        if ($Thumbprint) {
            $cert = Get-ChildItem Cert:\CurrentUser\My |
                    Where-Object { $_.Thumbprint -eq $Thumbprint } |
                    Select-Object -First 1
            if (-not $cert) {
                throw "证书库里找不到指纹为 $Thumbprint 的证书（查 Cert:\CurrentUser\My）。"
            }
        }
        else {
            $cert = Get-ChildItem Cert:\CurrentUser\My |
                    Where-Object { $_.Subject -eq $props.Publisher -and $_.HasPrivateKey } |
                    Select-Object -First 1
            if (-not $cert) {
                throw @"
指定了 -Sign，但证书库里没有 Subject 等于 manifest Publisher 的证书：
    Subject = $($props.Publisher)

三条路（脚本刻意不自动建自签名证书）：
  1. 用你已有的上架证书：
       -Sign -CertPath D:\path\to\your.pfx -CertPassword '密码'
  2. 把上架证书装进 Cert:\CurrentUser\My 后直接 -Sign，
     或用 -Thumbprint <SHA1 指纹> 精确指定；
  3. 本地只是想过一遍结构校验：去掉 -Sign，打未签名包再跑 -Verify。
"@
            }
        }
        Write-Host ("cert: {0}  thumbprint {1}" -f $cert.Subject, $cert.Thumbprint)
        $signArgs += @('/sha1', $cert.Thumbprint)
    }

    $signArgs += @('/td', 'SHA256')
    if ($TimestampUrl) { $signArgs += @('/tr', $TimestampUrl) }
    $signArgs += $msixPath

    & $signtool @signArgs
    if ($LASTEXITCODE -ne 0) {
        throw "signtool 签名失败，退出码 $LASTEXITCODE。检查证书是否过期、Publisher 是否与证书 Subject 逐字一致。"
    }

    Write-Host ''
    Write-Host '包已签名。信任链没装的话 Add-AppxPackage 会报 0x800B0109：'
    Write-Host '  需要把签这张证书的根证书装进「受信任的根证书颁发机构」才能本地安装。'
    Write-Host '  测完记得删：Remove-Item Cert:\LocalMachine\Root\<指纹>'
    Write-Host ''
    Write-Host '**签名包只用于本地测试，不要提交 Store。** 提交前重跑一次不带 -Sign 的命令。'
}

# ---------------- 可选：解包校验 ----------------
# 为什么必须有这一步：包能"打成功"不代表内容对。解回来逐项核对 Identity /
# Executable / startupTask / runFullTrust 以及文件完整性 —— 这是唯一不碰系统
# 状态的验证手段（不注册、不安装，绝不会顶掉已上架的商店版）。
if ($Verify) {
    Write-Step 'verify (unpack)'

    $unpackDir = Join-Path (Join-Path $root 'scratch') 'unpack'
    if (Test-Path $unpackDir) { Remove-Item $unpackDir -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $unpackDir | Out-Null

    & $makeappx unpack /p $msixPath /d $unpackDir /o
    if ($LASTEXITCODE -ne 0) {
        throw "makeappx unpack 失败，退出码 $LASTEXITCODE —— 包本身可能是坏的。"
    }

    $checks = New-Object System.Collections.ArrayList
    $failed = 0

    function Add-Check {
        param([string] $name, [bool] $ok, [string] $detail)
        if ($ok) {
            Write-Host ("  [OK]   {0,-44} {1}" -f $name, $detail)
        } else {
            Write-Host ("  [FAIL] {0,-44} {1}" -f $name, $detail) -ForegroundColor Red
            $script:failed++
        }
    }

    # --- manifest ---
    Write-Host ''
    Write-Host ' -- manifest --'
    $unpackedManifest = Join-Path $unpackDir 'AppxManifest.xml'
    Add-Check 'manifest 存在' (Test-Path $unpackedManifest) $unpackedManifest
    if (-not (Test-Path $unpackedManifest)) {
        throw "解包里没有 AppxManifest.xml —— 产物不是有效 MSIX。"
    }

    $doc = New-Object System.Xml.XmlDocument
    $doc.LoadXml((Read-Utf8NoBom $unpackedManifest))

    $ns = New-Object System.Xml.XmlNamespaceManager($doc.NameTable)
    $ns.AddNamespace('f',       'http://schemas.microsoft.com/appx/manifest/foundation/windows10')
    $ns.AddNamespace('uap',     'http://schemas.microsoft.com/appx/manifest/uap/windows10')
    $ns.AddNamespace('rescap',  'http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities')
    $ns.AddNamespace('desktop', 'http://schemas.microsoft.com/appx/manifest/desktop/windows10')

    $ident = $doc.SelectSingleNode('/f:Package/f:Identity', $ns)
    Add-Check 'Identity/@Name' `
        ($ident.GetAttribute('Name') -eq $props.IdentityName) `
        $ident.GetAttribute('Name')
    Add-Check 'Identity/@Publisher' `
        ($ident.GetAttribute('Publisher') -eq $props.Publisher) `
        $ident.GetAttribute('Publisher')
    Add-Check 'Identity/@Version' `
        ($ident.GetAttribute('Version') -eq $props.Version) `
        $ident.GetAttribute('Version')
    Add-Check 'Identity/@ProcessorArchitecture' `
        ($ident.GetAttribute('ProcessorArchitecture') -eq 'x64') `
        $ident.GetAttribute('ProcessorArchitecture')

    $app = $doc.SelectSingleNode('/f:Package/f:Applications/f:Application', $ns)
    Add-Check 'Application/@Executable' `
        ($app.GetAttribute('Executable') -eq $exeName) `
        $app.GetAttribute('Executable')
    Add-Check 'Application/@EntryPoint' `
        ($app.GetAttribute('EntryPoint') -eq 'Windows.FullTrustApplication') `
        $app.GetAttribute('EntryPoint')

    $startupExt = $doc.SelectSingleNode(
        '/f:Package/f:Applications/f:Application/f:Extensions/desktop:Extension[@Category="windows.startupTask"]',
        $ns)
    Add-Check 'startupTask 扩展存在' ($null -ne $startupExt) 'desktop:Extension[@Category="windows.startupTask"]'
    if ($startupExt) {
        Add-Check 'startupTask Extension/@Executable' `
            ($startupExt.GetAttribute('Executable') -eq $exeName) `
            $startupExt.GetAttribute('Executable')
        $task = $startupExt.SelectSingleNode('desktop:StartupTask', $ns)
        Add-Check 'StartupTask/@TaskId' `
            ($task.GetAttribute('TaskId') -eq $props.StartupTaskId) `
            $task.GetAttribute('TaskId')
        Add-Check 'StartupTask/@Enabled=false（默认关）' `
            ($task.GetAttribute('Enabled') -eq 'false') `
            $task.GetAttribute('Enabled')
    }

    $cap = $doc.SelectSingleNode('/f:Package/f:Capabilities/rescap:Capability[@Name="runFullTrust"]', $ns)
    Add-Check 'runFullTrust capability' ($null -ne $cap) 'rescap:Capability Name="runFullTrust"'

    # 只允许 runFullTrust：HID 不需要额外能力，多声明一个都该被发现
    $allCaps = @($doc.SelectNodes('/f:Package/f:Capabilities/*', $ns))
    $capNames = ($allCaps | ForEach-Object { $_.GetAttribute('Name') }) -join ', '
    Add-Check '能力只有 1 个（无多余声明）' ($allCaps.Count -eq 1) $capNames

    # --- 文件 ---
    Write-Host ''
    Write-Host ' -- files --'
    $unpackedExe = Join-Path $unpackDir $exeName
    if (Test-Path $unpackedExe) {
        $uexeSize = (Get-Item $unpackedExe).Length
        Add-Check "exe（$exeName）" ($uexeSize -eq $exeSize) ("{0:N0} bytes / 源 {1:N0}" -f $uexeSize, $exeSize)
    } else {
        Add-Check "exe（$exeName）" $false 'MISSING'
    }

    foreach ($a in $requiredAssets) {
        $p = Join-Path (Join-Path $unpackDir 'Assets') $a
        if (Test-Path $p) {
            Add-Check "Assets\$a" $true ("{0:N0} bytes" -f (Get-Item $p).Length)
        } else {
            Add-Check "Assets\$a" $false 'MISSING'
        }
    }

    # 包根不允许夹带源码/配置之类
    $allowed = @('AppxManifest.xml', $exeName, 'AppxBlockMap.xml', 'AppxSignature.p7x', '[Content_Types].xml')
    $extra = @(Get-ChildItem $unpackDir -File | Where-Object { $allowed -notcontains $_.Name })
    Add-Check '包根无多余文件' ($extra.Count -eq 0) (($extra | ForEach-Object { $_.Name }) -join ', ')

    Write-Host ''
    Write-Host ("校验完成：失败 {0} 项；解包目录 {1}（可删）" -f $failed, $unpackDir)
    if ($failed -gt 0) {
        throw "MSIX 校验失败 $failed 项，见上面 [FAIL] 行。包不可发布。"
    }
    Write-Host '全部通过。'
}

# ---------------- 收尾 ----------------
Write-Step 'done'
Write-Host ("  {0}" -f $msixPath)
Write-Host ("  {0:N0} bytes" -f $msixSize)
Write-Host ''
if ($Sign) {
    Write-Host '这个包是给本地测试装的。提交 Store 前重跑一次不带 -Sign 的命令，'
    Write-Host '产出的未签名包才是要上传的（Store 会用自己的证书重签）。'
} else {
    Write-Host '提交 Store：把上面这个**未签名**的 .msix 传上去，Store 会用自己的证书重签，'
    Write-Host '装到用户机器上永不触发 SmartScreen。'
    Write-Host ''
    Write-Host 'manifest 里的身份已对齐 Partner Center：'
    Write-Host ("  Name      = {0}" -f $props.IdentityName)
    Write-Host ("  Publisher = {0}" -f $props.Publisher)
    Write-Host '换产品时改 package.props.psd1，不要在这里改。'
}
