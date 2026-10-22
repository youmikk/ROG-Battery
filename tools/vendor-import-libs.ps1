#!/usr/bin/env pwsh
# Copy the import libraries that rustlib's self-contained MinGW is missing.
#
# Why this exists: rustlib's x86_64-pc-windows-gnu self-contained directory
# ships only 43 Windows import libraries. It has NO libgdiplus.a / libuxtheme.a
# / libdwmapi.a / libshlwapi.a / libversion.a. To link against those system
# DLLs, the .a files must come from somewhere else.
#
# Why copy only the needed ones instead of adding the whole llvm-mingw lib dir
# to -L: llvm-mingw's libmsvcrt.a / libmingwex.a would shadow rustlib's same
# named libs and the CRT dies at startup.
#
# IMPORTANT: this file must stay pure ASCII. Windows PowerShell 5.1 decodes .ps1
# as ANSI (GBK on this machine), so non-ASCII comments get mangled into invalid
# tokens and the script fails to parse.
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools/vendor-import-libs.ps1

$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $repo 'vendor\mingw-import-libs'

# Import libs to borrow from LLVM-MinGW. Add more here as needed.
$want = @(
    'libgdiplus.a',   # GDI+ self-drawing: rounded corners, gradients, AA text
    'libuxtheme.a',   # visual styles
    'libdwmapi.a',    # window shadow / acrylic (optional)
    'libshlwapi.a',   # path / string helpers
    'libversion.a'    # file version info
)

# Search common install locations for LLVM-MinGW's x86_64-w64-mingw32\lib
$candidates = @(
    (Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Packages'),
    (Join-Path $env:ProgramFiles 'llvm-mingw'),
    (Join-Path ${env:ProgramFiles(x86)} 'llvm-mingw')
)

$libDir = $null
foreach ($c in $candidates) {
    if (-not (Test-Path $c)) { continue }
    $hit = Get-ChildItem $c -Directory -Recurse -Filter 'x86_64-w64-mingw32' -ErrorAction SilentlyContinue |
        Where-Object { Test-Path (Join-Path $_.FullName 'lib\libgdiplus.a') } |
        Select-Object -First 1
    if ($hit) { $libDir = Join-Path $hit.FullName 'lib'; break }
}

if (-not $libDir) {
    Write-Host 'LLVM-MinGW x86_64-w64-mingw32\lib not found.' -ForegroundColor Red
    Write-Host 'Install it first: winget install MartinStorsjo.LLVM-MinGW.MSVCRT' -ForegroundColor Yellow
    Write-Host 'WARNING: that installer appends its bin dir to the USER PATH and can' -ForegroundColor Yellow
    Write-Host 'hijack the Rust linker. After installing, check:' -ForegroundColor Yellow
    Write-Host '  [Environment]::GetEnvironmentVariable("PATH","User")' -ForegroundColor Yellow
    Write-Host 'and remove the LLVM-MinGW entry. This script does NOT need it in PATH;' -ForegroundColor Yellow
    Write-Host 'it only reads the .a files.' -ForegroundColor Yellow
    exit 1
}

Write-Host "LLVM-MinGW lib dir: $libDir"
New-Item -ItemType Directory -Force -Path $dest | Out-Null

$copied = 0
$missing = @()
foreach ($name in $want) {
    $src = Join-Path $libDir $name
    if (Test-Path $src) {
        Copy-Item $src $dest -Force
        $sz = (Get-Item (Join-Path $dest $name)).Length
        Write-Host ("  [OK]   {0,-16} {1,8} B" -f $name, $sz) -ForegroundColor Green
        $copied++
    }
    else {
        $missing += $name
        Write-Host ("  [MISS] {0}" -f $name) -ForegroundColor DarkYellow
    }
}

Write-Host ''
Write-Host "Copied $copied into $dest"
if ($missing.Count -gt 0) {
    Write-Host "Missing: $($missing -join ', ')"
    Write-Host '(may live in a different LLVM-MinGW package variant; does not block the rest)' -ForegroundColor DarkYellow
}

Write-Host ''
Write-Host 'How to use when compiling:'
Write-Host "  cargo rustc --release -- -L `"$dest`""
Write-Host ''
Write-Host 'Or in .cargo/config.toml:'
Write-Host '  [build]'
Write-Host "  rustflags = [`"-L`", `"$($dest -replace '\\','\\')`"]"
