$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $repo 'vendor\mingw-import-libs'

$want = @(
    'libgdiplus.a',   # GDI+ self-drawing: rounded corners, gradients, AA text
    'libuxtheme.a',   # visual styles
    'libdwmapi.a',    # window shadow / acrylic (optional)
    'libshlwapi.a',   # path / string helpers
    'libversion.a'    # file version info
)

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
