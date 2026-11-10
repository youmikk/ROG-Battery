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

function Remove-BuildDirectory([string] $Path) {
    $base = [IO.Path]::GetFullPath($root).TrimEnd('\') + '\'
    $resolved = [IO.Path]::GetFullPath($Path)
    if (-not $resolved.StartsWith($base, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Build directory is outside the packaging workspace."
    }
    if (Test-Path -LiteralPath $resolved) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}

function Write-Step([string] $text) {
    Write-Host ''
    Write-Host "=== $text ===" -ForegroundColor Cyan
}

function Read-Utf8NoBom([string] $path) {
    $text = [System.Text.Encoding]::UTF8.GetString([System.IO.File]::ReadAllBytes($path))
    return $text.TrimStart([char] 0xFEFF)
}

if ($Clean) {
    foreach ($d in @($stage, $pkgDir, (Join-Path $root 'scratch'))) {
        Remove-BuildDirectory $d
    }
    Write-Host 'Clean done.'
    return
}

$propsPath = Join-Path $root 'package.props.psd1'
if (-not (Test-Path $propsPath)) {
    throw "找不到参数文件 $propsPath —— msix 目录不完整，重新检查打包资产。"
}
$props = Import-PowerShellDataFile -Path $propsPath

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

if ($props.Version -notmatch '^\d+\.\d+\.\d+\.\d+$') {
    throw "Version 必须是 4 段数字，例如 1.0.0.0（现在是 '$($props.Version)'）。用 -Version 1.0.1.0 指定。"
}
if ($props.Version -match '^0\.') {
    throw "Version 第一段不能为 0（Store 要求 >= 1.0.0.0，现在是 $($props.Version)）"
}

if (-not $SkipInstalledVersionCheck) {
    $installed = $null
    try {
        $installed = @(Get-AppxPackage -Name $props.IdentityName -ErrorAction Stop)
    } catch {
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

function Find-Tool {
    param([string] $Name, [string] $Preferred, [string] $Fallback)
    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    if ($Preferred -and (Test-Path $Preferred)) { return $Preferred }
    if ($Fallback -and (Test-Path $Fallback)) { return $Fallback }
    $sdk = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $tool = Get-ChildItem $sdk -Recurse -Filter $Name -ErrorAction SilentlyContinue |
        Where-Object { $_.Directory.Name -eq 'x64' } |
        Sort-Object FullName -Descending | Select-Object -First 1
    if ($tool) { return $tool.FullName }
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

Write-Step 'staging'
Remove-BuildDirectory $stage
New-Item -ItemType Directory -Path $stage | Out-Null
New-Item -ItemType Directory -Path (Join-Path $stage 'Assets') | Out-Null

Copy-Item $exe (Join-Path $stage $exeName)
$stagedExeSize = (Get-Item (Join-Path $stage $exeName)).Length
if ($stagedExeSize -ne $exeSize) {
    throw "拷贝 exe 到 stage 后大小不一致（源 $exeSize / 目标 $stagedExeSize），磁盘可能有问题。"
}

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
    $tpl = $tpl.Replace($k, [string] $map[$k])
}

$leftover = [regex]::Matches($tpl, '@[A-Z_]+@')
if ($leftover.Count -gt 0) {
    $names = (@($leftover | ForEach-Object { $_.Value } | Sort-Object -Unique)) -join ', '
    throw "模板里还有没替换的占位符: $names（检查 package.props.psd1 是否缺键）"
}

$manifestPath = Join-Path $stage 'AppxManifest.xml'
[System.IO.File]::WriteAllText($manifestPath, $tpl, (New-Object System.Text.UTF8Encoding($true)))

try {
    [xml] $null = Read-Utf8NoBom $manifestPath
} catch {
    throw "生成的 AppxManifest.xml 不是合法 XML: $($_.Exception.Message)"
}
Write-Host "manifest: $manifestPath"

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
            $signArgs += @('/p', $CertPassword)
        }
    }
    else {
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

if ($Verify) {
    Write-Step 'verify (unpack)'

    $unpackDir = Join-Path (Join-Path $root 'scratch') 'unpack'
    Remove-BuildDirectory $unpackDir
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

    $allCaps = @($doc.SelectNodes('/f:Package/f:Capabilities/*', $ns))
    $capNames = ($allCaps | ForEach-Object { $_.GetAttribute('Name') }) -join ', '
    Add-Check '能力只有 1 个（无多余声明）' ($allCaps.Count -eq 1) $capNames

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
