[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string] $ProductId,
    [Parameter(Mandatory = $true)][string] $PackagePath,
    [string] $TenantId,
    [string] $ClientId,
    [string] $ClientSecret,
    [switch] $DryRun
)

$ErrorActionPreference = 'Stop'
$api = 'https://manage.devcenter.microsoft.com/v1.0/my'

function Get-EnvOrParam($value, $name) {
    if ($value) { return $value }
    $v = [Environment]::GetEnvironmentVariable($name)
    if (-not $v) { throw "缺少 $name" }
    return $v
}

$TenantId     = Get-EnvOrParam $TenantId     'AZURE_AD_TENANT_ID'
$ClientId     = Get-EnvOrParam $ClientId     'AZURE_AD_APPLICATION_CLIENT_ID'
$ClientSecret = Get-EnvOrParam $ClientSecret 'AZURE_AD_APPLICATION_SECRET'

if (-not (Test-Path $PackagePath)) { throw "找不到包：$PackagePath" }
$pkg = (Resolve-Path $PackagePath).Path
$pkgName = Split-Path -Leaf $pkg
Write-Host "包：$pkgName  ($((Get-Item $pkg).Length) B)" -ForegroundColor Cyan

Write-Host '获取访问令牌…' -ForegroundColor DarkGray
$tokenBody = @{
    grant_type    = 'client_credentials'
    client_id     = $ClientId
    client_secret = $ClientSecret
    scope         = 'https://manage.devcenter.microsoft.com/.default'
}
$tok = Invoke-RestMethod -Method Post -ContentType 'application/x-www-form-urlencoded' `
    -Body $tokenBody -Uri "https://login.microsoftonline.com/$TenantId/oauth2/v2.0/token"
$headers = @{ Authorization = "Bearer $($tok.access_token)" }

function Get-HttpErrorBody {
    param($err)
    if ($err.ErrorDetails -and $err.ErrorDetails.Message) {
        $m = [string]$err.ErrorDetails.Message
        if (-not [string]::IsNullOrWhiteSpace($m)) { return $m.Trim() }
    }
    try {
        $r = $err.Exception.Response
        if ($r -and $r.Content) {
            $t = $r.Content.ReadAsStringAsync().GetAwaiter().GetResult()
            if (-not [string]::IsNullOrWhiteSpace($t)) { return $t.Trim() }
        }
    } catch { }
    try {
        $r2 = $err.Exception.Response
        if ($r2) {
            $s = $r2.GetResponseStream()
            if ($s) {
                $rd = New-Object System.IO.StreamReader($s)
                $t2 = $rd.ReadToEnd()
                $rd.Close()
                if (-not [string]::IsNullOrWhiteSpace($t2)) { return $t2.Trim() }
            }
        }
    } catch { }
    return $null
}
Write-Host '  OK' -ForegroundColor DarkGray

$subsUri = "$api/applications/$ProductId/submissions"

$app = Invoke-RestMethod -Method Get -Uri "$api/applications/$ProductId" -Headers $headers
if ($app.pendingApplicationSubmission -and $app.pendingApplicationSubmission.id) {
    throw "An existing Store submission needs attention: $($app.pendingApplicationSubmission.id). It has been left unchanged."
}
Write-Host '创建草稿…'
$sub = Invoke-RestMethod -Method Post -Uri $subsUri -Headers $headers -ContentType 'application/json'
if (-not $sub) { throw '创建草稿失败（没有返回提交对象）' }
Write-Host "  id=$($sub.id)  status=$($sub.status)" -ForegroundColor DarkGray

$pkgs = @($sub.applicationPackages)
if ($pkgs.Count -eq 0) { throw '草稿里没有 applicationPackages 槽位，无法挂包' }
Write-Host "  现有包槽：$($pkgs.Count) 个，第一个 $($pkgs[0].fileName)  fileStatus=$($pkgs[0].fileStatus)" -ForegroundColor DarkGray
Write-Host "  槽字段：$(($pkgs[0].PSObject.Properties | ForEach-Object { $_.Name }) -join ', ')" -ForegroundColor DarkGray

$verFromName = if ($pkgName -match '_(\d+\.\d+\.\d+\.\d+)_') { $matches[1] } else { $null }
$innerName = ([System.IO.Path]::GetFileNameWithoutExtension($pkgName)) + '.msix'
Write-Host "  目标包：$innerName  version=$verFromName（上传档案：$pkgName）" -ForegroundColor DarkGray

$cur = Invoke-RestMethod -Method Get -Uri "$subsUri/$($sub.id)" -Headers $headers

$shapes = @(
    @{ name = '旧槽 PendingDelete + 新槽 PendingUpload'
       pkg  = {
           param($base, $n, $ver)
           $old = $base | Select-Object *
           $old.fileStatus = 'PendingDelete'
           $new = $base | Select-Object *
           foreach ($f in 'id', 'version') { if ($new.PSObject.Properties[$f]) { $new.PSObject.Properties.Remove($f) } }
           $new.fileName = $n
           $new.fileStatus = 'PendingUpload'
           @($old, $new)
       } },
    @{ name = '旧槽 PendingDelete + 新槽 PendingUpload + version'
       pkg  = {
           param($base, $n, $ver)
           $old = $base | Select-Object *
           $old.fileStatus = 'PendingDelete'
           $new = $base | Select-Object *
           if ($new.PSObject.Properties['id']) { $new.PSObject.Properties.Remove('id') }
           $new.fileName = $n
           $new.fileStatus = 'PendingUpload'
           if ($ver) { $new.version = $ver }
           @($old, $new)
       } },
    @{ name = '完整槽 + fileName + PendingUpload'
       pkg  = {
           param($base, $n, $ver)
           $o = $base | Select-Object *
           $o.fileName = $n
           $o.fileStatus = 'PendingUpload'
           $o
       } },
    @{ name = '完整槽 + 仅 fileName（保持 Uploaded，兜底）'
       pkg  = {
           param($base, $n, $ver)
           $o = $base | Select-Object *
           $o.fileName = $n
           $o
       } }
)

$sub2 = $null
$acceptedShape = $null
foreach ($s in $shapes) {
    $cur = Invoke-RestMethod -Method Get -Uri "$subsUri/$($sub.id)" -Headers $headers
    $baseSlot = @($cur.applicationPackages)[0]
    $cur.applicationPackages = @((& $s.pkg $baseSlot $innerName $verFromName))
    $notesRoot = Join-Path (Split-Path -Parent $PSScriptRoot) 'msix/store'
    foreach ($listing in $cur.listings.PSObject.Properties) {
        $language = if ($listing.Name -like 'zh-*') { 'zh-CN' } elseif ($listing.Name -like 'en-*') { 'en-US' } else { $null }
        if ($language -and $listing.Value.baseListing) {
            $notes = Get-Content (Join-Path $notesRoot "release-notes.$language.txt") -Raw -Encoding utf8
            $listing.Value.baseListing | Add-Member -NotePropertyName releaseNotes -NotePropertyValue $notes.Trim() -Force
        }
    }

    $body = $cur | ConvertTo-Json -Depth 32
    try {
        $try = Invoke-RestMethod -Method Put -Uri "$subsUri/$($sub.id)" -Headers $headers -Body $body -ContentType 'application/json'
    } catch {
        $detail = Get-HttpErrorBody $_
        if (-not $detail) { $detail = $_.Exception.Message }
        Write-Host "  [$($s.name)] PUT 被拒：$detail" -ForegroundColor Yellow
        Write-Host "    发送的包对象：$(($cur.applicationPackages | ConvertTo-Json -Depth 8 -Compress))" -ForegroundColor DarkGray
        continue
    }
    $pkgs = @($try.applicationPackages)
    foreach ($g in $pkgs) {
        Write-Host "  [$($s.name)] -> $($g.fileName)  fileStatus=$($g.fileStatus)  version=$($g.version)" -ForegroundColor DarkGray
    }
    $mine = @($pkgs | Where-Object { $_.fileName -eq $innerName })
    if ($mine.Count -gt 0) { $sub2 = $try; $acceptedShape = $s.name; break }
    Write-Host '    → 商店没接受这个包名，换下一种形状' -ForegroundColor DarkGray
}

if (-not $sub2) {
    throw "试了 $($shapes.Count) 种形状都没能让 API 接受 $innerName。去 Partner Center 手动换包后重跑。"
}
Write-Host "  已接受形状：$acceptedShape" -ForegroundColor DarkGray

$slot2 = @($sub2.applicationPackages)[0]

$cands = @()
if ($sub.fileUploadUrl) { $cands += @{ n = 'POST 的 fileUploadUrl'; u = $sub.fileUploadUrl } }
if ($sub2.fileUploadUrl -and $sub2.fileUploadUrl -ne $sub.fileUploadUrl) {
    $cands += @{ n = 'PUT 的 fileUploadUrl'; u = $sub2.fileUploadUrl }
}
$mineSlot = @(@($sub2.applicationPackages) | Where-Object { $_.fileName -eq $innerName })
if ($mineSlot.Count -gt 0 -and $mineSlot[0].fileUploadUrl) {
    $u3 = $mineSlot[0].fileUploadUrl
    if (-not ($cands | Where-Object { $_.u -eq $u3 })) { $cands += @{ n = '新包条目的 fileUploadUrl'; u = $u3 } }
}
$extra = @()
foreach ($c in $cands) {
    if ($c.u -match '\?') {
        $base, $q = $c.u -split '\?', 2
        if ($base.EndsWith('/')) {
            $extra += @{ n = ($c.n + ' + 拼包名'); u = ($base + $innerName + '?' + $q) }
        }
    }
}
$cands = $cands + $extra
if ($cands.Count -eq 0) { throw '拿不到任何 fileUploadUrl' }

Write-Host "  候选上传地址 $($cands.Count) 个：" -ForegroundColor DarkGray

$bytes = [System.IO.File]::ReadAllBytes($pkg)
$upHeaders = @{
    'x-ms-blob-type' = 'BlockBlob'
    'x-ms-version'   = '2020-04-08'
    'Content-Type'   = 'application/octet-stream'
}

$uploaded = $false
$slotUp = $null
foreach ($c in $cands) {
    Write-Host "上传包体（$($c.n)）…" -ForegroundColor DarkGray
    try {
        Invoke-WebRequest -Method Put -Uri $c.u -Headers $upHeaders -Body $bytes -UseBasicParsing | Out-Null
    } catch {
        $d = Get-HttpErrorBody $_

        Write-Host "  上传失败，尝试下一个上传地址。" -ForegroundColor Yellow
        continue
    }
    Write-Host "  已上传 $([math]::Round($bytes.Length / 1MB, 2)) MB" -ForegroundColor DarkGray

    for ($w = 1; $w -le 4; $w++) {
        Start-Sleep -Seconds 8
        $afterUp = Invoke-RestMethod -Method Get -Uri "$subsUri/$($sub.id)" -Headers $headers
        $pkgAfter = @($afterUp.applicationPackages)
        $slotUp = @($pkgAfter | Where-Object { $_.fileName -eq $innerName })
        if ($slotUp.Count -eq 0) { $slotUp = @($pkgAfter)[0] } else { $slotUp = $slotUp[0] }
        Write-Host "  上传后核对（$($w*8)s）：$($slotUp.fileName)  version=$($slotUp.version)  fileStatus=$($slotUp.fileStatus)" -ForegroundColor DarkGray
        if ($slotUp.fileName -ne $innerName) { continue }
        if (-not $verFromName -or $slotUp.version -eq $verFromName -or [string]::IsNullOrEmpty($slotUp.version)) {
            $uploaded = $true
            break
        }
    }
    if ($uploaded) { break }
    Write-Host '    → 商店还没认到这个包，换下一个候选地址' -ForegroundColor DarkGray
}

if (-not $uploaded) {
    throw "包体传上去了但商店没认到（当前记的是 $($slotUp.fileName) / version=$($slotUp.version)）。停止提交，避免把旧包发出去。"
}

if ($DryRun) {
    Write-Host ''
    Write-Host 'DryRun：草稿已建好并挂上包，但没有 commit。' -ForegroundColor Yellow
    Write-Host "去这里核对：https://partner.microsoft.com/dashboard/products/$ProductId/submissions/$($sub.id)" -ForegroundColor Yellow
    return
}

Write-Host '提交认证…' -ForegroundColor DarkGray
Invoke-RestMethod -Method Post -Uri "$subsUri/$($sub.id)/commit" -Headers $headers -Body '{}' -ContentType 'application/json' | Out-Null

$finalOk = $false
for ($w = 1; $w -le 30; $w++) {
    Start-Sleep -Seconds 10
    try {
        $check = Invoke-RestMethod -Method Get -Uri "$subsUri/$($sub.id)" -Headers $headers
    } catch {
        continue
    }
    $pkgs = @($check.applicationPackages)
    $minePkg = @($pkgs | Where-Object { $_.fileName -eq $innerName })
    $gotPkg = if ($minePkg.Count -gt 0) { $minePkg[0] } else { $pkgs[0] }
    Write-Host "  提交后核对（$($w*10)s）：$($gotPkg.fileName)  version=$($gotPkg.version)  status=$($check.status)" -ForegroundColor DarkGray
    if ($check.status -in @('CommitFailed', 'PreProcessingFailed', 'CertificationFailed', 'PublishFailed')) {
        throw "Store submission failed: $($check.status)"
    }
    if ($gotPkg.fileName -eq $innerName -and $gotPkg.version -eq $verFromName) {
        $finalOk = $true
        break
    }
    if ($check.status -in @('CommitFailed', 'PreProcessingFailed', 'CertificationFailed', 'PublishFailed')) { break }
}

if ($finalOk) {
    Write-Host "  ✔ 商店已接收 $innerName（version=$verFromName）" -ForegroundColor Green
} else {
    throw "Store package version has not been verified: $($gotPkg.version); expected $verFromName"
}

Write-Host ''
Write-Host "已提交。状态页：https://partner.microsoft.com/dashboard/products/$ProductId/submissions" -ForegroundColor Green
