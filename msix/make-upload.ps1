# 生成 .msixupload（Partner Center 提交用的打包件）
#
# .msixupload 本质就是一个 ZIP，里面只放**包本体**：
#   <包名>.msix
#
# 为什么不再塞 Assets\StoreLogo.png 那三张图：
#   1) 我们的 AppxManifest 已经声明了 Assets\StoreLogo.png / Square150x150Logo.png /
#      Square44x44Logo.png，图就在 msix 内部，Partner Center 能自己取到；
#   2) 额外塞一份反而会被包接受验证打回 ——
#      "The file StoreLogo.png is not supported. Remove or update this file and then try again."
#      （2026-10-29 实测，三张图全部报这个错）
#   按文档，那三张图只在「包内没有这些图」时才需要放进 .msixupload。
#
# 万一 Partner Center 反过来提示缺商店图标，用 -WithStoreLogos 重出：此时三张图
# 放在 ZIP **根目录**（不是 Assets\ 子目录），这是它认的位置。
#
# 用法：
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\make-upload.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\make-upload.ps1 -Version 1.1.1.0
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\make-upload.ps1 -WithStoreLogos

[CmdletBinding()]
param(
    [string]$Version = '',
    [switch]$WithStoreLogos
)

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

if (-not $Version) {
    $props = Import-PowerShellDataFile (Join-Path $here 'package.props.psd1')
    $Version = $props.Version
}

$pkgName = "Sonder.RBattery_${Version}_x64"
$msix = Join-Path $here "AppPackages\$pkgName.msix"
if (-not (Test-Path $msix)) {
    throw "找不到 $msix —— 先跑 build-msix.ps1 -Version $Version"
}

$upload = Join-Path $here "AppPackages\$pkgName.msixupload"
if (Test-Path $upload) { Remove-Item -Force $upload }

$zip = [System.IO.Compression.ZipFile]::Open($upload, 'Create')
try {
    # 1) 包本体（唯一必需项）
    [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
        $zip, $msix, "$pkgName.msix", [System.IO.Compression.CompressionLevel]::Optimal)

    # 2) 可选：仅当包内没有商店图标时才需要，且必须放根目录
    if ($WithStoreLogos) {
        foreach ($n in @('StoreLogo.png', 'Square150x150Logo.png', 'Square44x44Logo.png')) {
            $src = Join-Path $here "Assets\$n"
            if (-not (Test-Path $src)) { throw "缺图标 Assets\$n" }
            [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                $zip, $src, $n, [System.IO.Compression.CompressionLevel]::Optimal)
        }
    }
}
finally {
    $zip.Dispose()
}

Write-Host ''
Write-Host "upload: $upload" -ForegroundColor Green
Write-Host ("        {0:N0} bytes" -f (Get-Item $upload).Length)
Write-Host ''
Write-Host '内容清单：'
$z = [System.IO.Compression.ZipFile]::OpenRead($upload)
try {
    foreach ($e in $z.Entries) {
        Write-Host ("        {0}  ({1:N0} bytes)" -f $e.FullName, $e.Length)
    }
}
finally {
    $z.Dispose()
}
Write-Host ''
if ($WithStoreLogos) {
    Write-Host '注意：已把三张商店图标放在 ZIP 根目录（兜底模式）。' -ForegroundColor Yellow
}
else {
    Write-Host '三张商店图标在 msix 内部（manifest 已声明），不再重复放进 .msixupload。' -ForegroundColor DarkGray
}
