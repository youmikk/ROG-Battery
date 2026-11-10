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
    [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
        $zip, $msix, "$pkgName.msix", [System.IO.Compression.CompressionLevel]::Optimal)

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
