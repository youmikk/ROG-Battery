[CmdletBinding()]
param(
    [string] $Tag = '',
    [string] $Executable = '',
    [switch] $SourceOnly
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$cargo = Get-Content -LiteralPath (Join-Path $repo 'Cargo.toml') -Raw
$match = [regex]::Match($cargo, '(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"\s*$')
if (-not $match.Success) { throw 'Cargo.toml has no stable package version.' }
$version = $match.Groups[1].Value
$storeVersion = "$version.0"
$props = Import-PowerShellDataFile -LiteralPath (Join-Path $repo 'msix/package.props.psd1')
if ($props.Version -ne $storeVersion) {
    throw "Store version $($props.Version) does not match Cargo version $version."
}
if ($Tag -and $Tag -cne "v$version") {
    throw "Tag $Tag does not match Cargo version v$version."
}
$result = [ordered]@{ Version = $version; StoreVersion = $storeVersion; Tag = $Tag }
if ($SourceOnly) { return [pscustomobject]$result }

if (-not $Executable) { $Executable = Join-Path $repo 'target/release/rog-mouse-ctl.exe' }
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$bytes = [IO.File]::ReadAllBytes($Executable)
if ($bytes.Length -lt 256 -or $bytes[0] -ne 0x4D -or $bytes[1] -ne 0x5A) {
    throw 'Executable has no valid DOS header.'
}
$pe = [BitConverter]::ToInt32($bytes, 0x3C)
if ($pe -lt 64 -or $pe + 94 -gt $bytes.Length -or
    [BitConverter]::ToUInt32($bytes, $pe) -ne 0x00004550) {
    throw 'Executable has no valid PE header.'
}
if ([BitConverter]::ToUInt16($bytes, $pe + 4) -ne 0x8664) {
    throw 'Release executable must be x64.'
}
if ([BitConverter]::ToUInt16($bytes, $pe + 0x18 + 0x44) -ne 2) {
    throw 'Release executable must use the Windows GUI subsystem.'
}

$scratch = Join-Path $repo 'scratch/release-validation'
New-Item -ItemType Directory -Force -Path $scratch | Out-Null
$id = [guid]::NewGuid().ToString('N')
$stdout = Join-Path $scratch "version-$id.txt"
$stderr = Join-Path $scratch "version-$id.err.txt"
$process = Start-Process -FilePath $Executable -ArgumentList 'version' -WindowStyle Hidden `
    -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
if (-not $process.WaitForExit(10000)) {
    $process.Kill()
    throw 'Executable did not return its version within 10 seconds.'
}
$process.WaitForExit()
$reported = (Get-Content -LiteralPath $stdout -Raw).Trim()
if ($process.ExitCode -ne 0 -or $reported -cne $version) {
    throw "Executable reports '$reported' (exit $($process.ExitCode)); expected '$version'."
}
$result.Sha256 = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash.ToLowerInvariant()
$result.Bytes = $bytes.Length
return [pscustomobject]$result
