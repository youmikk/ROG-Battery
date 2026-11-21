[CmdletBinding()]
param(
    [string] $Tag = '',
    [switch] $Offline
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$source = & (Join-Path $PSScriptRoot 'check-release.ps1') -Tag $Tag -SourceOnly
$cargo = (Get-Command cargo.exe -ErrorAction Stop).Source
$arguments = @('--release', '--locked')
if ($Offline) { $arguments += '--offline' }

Push-Location $repo
try {
    & $cargo test @arguments
    if ($LASTEXITCODE -ne 0) { throw 'Release regression tests failed.' }
    & $cargo build @arguments
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    $verified = & (Join-Path $PSScriptRoot 'check-release.ps1') -Tag $Tag

    $dist = Join-Path $repo "dist/$($source.Version)"
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    Copy-Item -LiteralPath (Join-Path $repo 'target/release/rog-mouse-ctl.exe') -Destination $dist
    [IO.File]::WriteAllText((Join-Path $dist 'rog-mouse-ctl.exe.sha256'),
        "$($verified.Sha256)  rog-mouse-ctl.exe`n", [Text.Encoding]::ASCII)
    $files = @('rog-mouse-ctl.exe')
    $hashes = foreach ($file in $files) {
        $hash = (Get-FileHash -LiteralPath (Join-Path $dist $file) -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $file"
    }
    [IO.File]::WriteAllText((Join-Path $dist 'SHA256SUMS.txt'), ($hashes -join "`n") + "`n", [Text.Encoding]::ASCII)
    Write-Host "Release $($source.Version) verified: $dist"
    Get-ChildItem -LiteralPath $dist -File | Select-Object Name, Length
} finally {
    Pop-Location
}
