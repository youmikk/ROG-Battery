# 生成 MSIX 所需的各个尺寸的 logo
#
# 为什么单独用脚本画而不是拿 exe 里的图标：托盘图标是"数字电量"，随电量变，
# 不适合当产品 logo。这里画一个固定的标识：深底圆角方块 + 电池轮廓 + 充电闪电。
#
# 用法: powershell -NoProfile -ExecutionPolicy Bypass -File make-assets.ps1

Add-Type -AssemblyName System.Drawing

$outDir = Join-Path $PSScriptRoot 'Assets'
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

function New-Logo([int]$w, [int]$h, [string]$path, [bool]$wide) {
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.Clear([System.Drawing.Color]::Transparent)

    # 底色：深靛蓝，和系统深色任务栏接近但不完全相同，便于区分
    $bg = [System.Drawing.Color]::FromArgb(255, 28, 30, 44)

    $m = [Math]::Max(1, [int]([Math]::Min($w, $h) * 0.04))
    $rect = New-Object System.Drawing.Rectangle($m, $m, ($w - 2 * $m), ($h - 2 * $m))
    $r = [Math]::Max(2, [int]([Math]::Min($w, $h) * 0.22))
    $gp = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = $r * 2
    $gp.AddArc($rect.X, $rect.Y, $d, $d, 180, 90)
    $gp.AddArc($rect.Right - $d, $rect.Y, $d, $d, 270, 90)
    $gp.AddArc($rect.Right - $d, $rect.Bottom - $d, $d, $d, 0, 90)
    $gp.AddArc($rect.X, $rect.Bottom - $d, $d, $d, 90, 90)
    $gp.CloseFigure()
    $bgBrush = New-Object System.Drawing.SolidBrush($bg)
    $g.FillPath($bgBrush, $gp)

    # 电池主体（横向），宽屏时整体右移一点给文字留位置
    $unit = [Math]::Min($w, $h)
    $bw = [int]($unit * 0.52)      # 电池宽度
    $bh = [int]($unit * 0.26)      # 电池高度
    $pad = [Math]::Max(2, [int]($unit * 0.05))  # 描边粗细
    $cx = [int]($w / 2)
    $cy = [int]($h / 2)
    if ($wide) { $cx = [int]($w * 0.40) }
    $bx = $cx - [int]($bw / 2)
    $by = $cy - [int]($bh / 2)

    $white = New-Object System.Drawing.Pen([System.Drawing.Color]::FromArgb(255, 245, 246, 250), $pad)
    $white.LineJoin = [System.Drawing.Drawing2D.LineJoin]::Round
    $bodyRect = New-Object System.Drawing.Rectangle($bx, $by, $bw, $bh)
    $br = [Math]::Max(1, [int]($bh * 0.18))
    $bp = New-Object System.Drawing.Drawing2D.GraphicsPath
    $bd = $br * 2
    $bp.AddArc($bodyRect.X, $bodyRect.Y, $bd, $bd, 180, 90)
    $bp.AddArc($bodyRect.Right - $bd, $bodyRect.Y, $bd, $bd, 270, 90)
    $bp.AddArc($bodyRect.Right - $bd, $bodyRect.Bottom - $bd, $bd, $bd, 0, 90)
    $bp.AddArc($bodyRect.X, $bodyRect.Bottom - $bd, $bd, $bd, 90, 90)
    $bp.CloseFigure()
    $g.DrawPath($white, $bp)

    # 电池正极凸点
    $tipW = [Math]::Max(2, [int]($bw * 0.07))
    $tipH = [Math]::Max(2, [int]($bh * 0.32))
    $tipRect = New-Object System.Drawing.Rectangle(($bx + $bw + [int]($pad * 0.6)), ($cy - [int]($tipH / 2)), $tipW, $tipH)
    $tipBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 245, 246, 250))
    $g.FillRectangle($tipBrush, $tipRect)

    # 电量填充（绿色，约 70%）
    $fillPad = [int]($pad * 1.8)
    $fillW = [int](($bw - 2 * $fillPad) * 0.70)
    $fillH = $bh - 2 * $fillPad
    if ($fillW -gt 1 -and $fillH -gt 1) {
        $green = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 60, 220, 130))
        $g.FillRectangle($green, ($bx + $fillPad), ($by + $fillPad), $fillW, $fillH)
        $green.Dispose()
    }

    # 闪电：白色，整个装在电池内腔里（不外溢，否则看起来像把边框切断了）
    # 尺寸按内腔算而不是按整图算，这样宽屏/方图两种画幅下都稳。
    $innerW = $bw - 2 * $fillPad
    $innerH = $bh - 2 * $fillPad
    $lw = [int]($innerW * 0.42)
    $lh = [int]($innerH * 0.86)
    $lx = $cx - [int]($lw / 2)
    $ly = $cy - [int]($lh / 2)
    $pts = @(
        (New-Object System.Drawing.PointF(($lx + $lw * 0.62), $ly)),
        (New-Object System.Drawing.PointF($lx, ($ly + $lh * 0.56))),
        (New-Object System.Drawing.PointF(($lx + $lw * 0.42), ($ly + $lh * 0.56))),
        (New-Object System.Drawing.PointF(($lx + $lw * 0.30), ($ly + $lh))),
        (New-Object System.Drawing.PointF(($lx + $lw), ($ly + $lh * 0.40))),
        (New-Object System.Drawing.PointF(($lx + $lw * 0.52), ($ly + $lh * 0.40)))
    )
    $bolt = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 245, 246, 250))
    $g.FillPolygon($bolt, $pts)

    $g.Dispose()
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    $bgBrush.Dispose()
    $white.Dispose()
    $tipBrush.Dispose()
    $bolt.Dispose()
    $gp.Dispose(); $bp.Dispose()
}

New-Logo 50 50 (Join-Path $outDir 'StoreLogo.png') $false
New-Logo 44 44 (Join-Path $outDir 'Square44x44Logo.png') $false
New-Logo 150 150 (Join-Path $outDir 'Square150x150Logo.png') $false
New-Logo 310 310 (Join-Path $outDir 'Square310x310Logo.png') $false
New-Logo 310 150 (Join-Path $outDir 'Wide310x150Logo.png') $true
# SplashScreen：桌面桥应用其实不用，但有些旧校验会找它，给一张没坏处
New-Logo 620 300 (Join-Path $outDir 'SplashScreen.png') $true

# 托盘/任务栏用的小尺寸变体（targetsize 变体让 16/24/32 更清晰）
foreach ($s in 16, 24, 32, 48, 256) {
    New-Logo $s $s (Join-Path $outDir ("Square44x44Logo.targetsize-{0}.png" -f $s)) $false
}

Write-Output "=== generated ==="
Get-ChildItem $outDir -Filter *.png | Sort-Object Name | ForEach-Object {
    Write-Output ("  {0,-42} {1,6} bytes" -f $_.Name, $_.Length)
}
