# Build a multi-size .ico for the nwg tray icon from MSIX PNG assets.
#
# Why this is needed:
#   nwg::OemIcon only has 6 system variants (no Application), and
#   TrayNotification::build panics when icon is None:
#     src\controls\tray_notification.rs:370
#     panic!("Tray notification requires an Icon at creation")
#   So we must supply a real .ico file.
#
# ASCII-only on purpose: PowerShell 5.1 reads .ps1 as system ANSI (GBK here),
# and mojibake in a comment can produce stray quote/backtick chars that break parsing.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$src = 'D:\codex\rog-mouse-ctl\msix\Assets'
$outDir = 'D:\codex\rog-mouse-ctl\assets'
New-Item -ItemType Directory -Path $outDir -Force | Out-Null
$outIco = Join-Path $outDir 'tray.ico'

# Tray itself uses 16/20/24/32, but Windows also asks for bigger sizes in
# Alt+Tab and Task Manager, so ship them all to avoid blurry upscaling.
# Array of pairs, NOT [ordered]@{}: OrderedDictionary does not type-coerce keys,
# so an Int32 key lookup can silently return $null and yield a bare directory path.
$sources = @(
    @{ Name = 'Square44x44Logo.targetsize-16.png';  Size = 16  },
    @{ Name = 'Square44x44Logo.targetsize-24.png';  Size = 24  },
    @{ Name = 'Square44x44Logo.targetsize-32.png';  Size = 32  },
    @{ Name = 'Square44x44Logo.targetsize-48.png';  Size = 48  },
    @{ Name = 'Square44x44Logo.targetsize-256.png'; Size = 256 }
)

# Embed the PNG bytes as-is; Vista+ ICO supports PNG-compressed entries.
$images = @()
foreach ($s in $sources) {
    $p = Join-Path $src $s.Name
    if (-not (Test-Path $p)) { throw "missing source: $p" }
    $bytes = [System.IO.File]::ReadAllBytes($p)
    # Trust pixels, not filenames.
    $img = [System.Drawing.Image]::FromFile($p)
    $w = $img.Width; $h = $img.Height; $img.Dispose()
    if ($w -ne $h) { throw "not square: $p ($w x $h)" }
    if ($w -gt 256) { throw "too big: $p ($w)" }
    if ($w -ne $s.Size) { throw "size mismatch: $p is $w, expected $($s.Size)" }
    $images += [pscustomobject]@{ Size = $w; Bytes = $bytes }
    Write-Host ("  src {0,-38} {1}x{1}  {2,6} B" -f $s.Name, $w, $bytes.Length)
}

$count = $images.Count
$ms = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter($ms)

# ICONDIR: reserved=0, type=1 (icon), count
$bw.Write([uint16]0)
$bw.Write([uint16]1)
$bw.Write([uint16]$count)

# ICONDIRENTRY is 16 bytes each; write offsets as 0 first, backfill later.
$dirStart = $ms.Position
foreach ($im in $images) {
    $bw.Write([byte]($im.Size -band 0xFF))   # width  (256 is encoded as 0)
    $bw.Write([byte]($im.Size -band 0xFF))   # height
    $bw.Write([byte]0)      # colorCount: 0 = truecolor
    $bw.Write([byte]0)      # reserved
    $bw.Write([uint16]1)    # planes
    $bw.Write([uint16]32)   # bitCount
    $bw.Write([uint32]$im.Bytes.Length)
    $bw.Write([uint32]0)    # imageOffset placeholder
}

$offsets = @()
foreach ($im in $images) {
    $offsets += [uint32]$ms.Position
    $bw.Write($im.Bytes)
}
$bw.Flush()

# Backfill imageOffset (last 4 bytes of each 16-byte entry).
for ($i = 0; $i -lt $count; $i++) {
    $ms.Position = $dirStart + ($i * 16) + 12
    $bw.Write($offsets[$i])
}
$bw.Flush()

[System.IO.File]::WriteAllBytes($outIco, $ms.ToArray())
$bw.Dispose(); $ms.Dispose()

Write-Host ""
Write-Host "wrote: $outIco  $((Get-Item $outIco).Length) B"

# Verify GDI+ can parse what we just wrote.
$ico = New-Object System.Drawing.Icon($outIco, 16, 16)
Write-Host "  reload ok: $($ico.Width)x$($ico.Height)  Handle=$($ico.Handle)"
$ico.Dispose()

foreach ($s in 16, 24, 32, 48, 256) {
    try {
        $i2 = New-Object System.Drawing.Icon($outIco, $s, $s)
        Write-Host ("  ask {0,3}px -> got {1}x{2}" -f $s, $i2.Width, $i2.Height)
        $i2.Dispose()
    } catch {
        Write-Host ("  ask {0,3}px -> FAILED: {1}" -f $s, $_.Exception.Message)
    }
}
