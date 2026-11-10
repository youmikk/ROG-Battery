$ErrorActionPreference = 'Stop'
$panelUrl = 'http://127.0.0.1:48763'
$node = (Get-Command node -ErrorAction Stop).Source
$workspace = Split-Path -Parent $PSScriptRoot
$scratch = Join-Path $workspace 'scratch\mock-panel'
[void][IO.Directory]::CreateDirectory($scratch)
function PanelReady {
    try {
        $health = Invoke-RestMethod -Uri ($panelUrl + '/health') -TimeoutSec 1
        return $health.app -eq 'r-battery-mock-panel'
    } catch { return $false }
}
if (!(PanelReady)) {
    $scriptPath = Join-Path $PSScriptRoot 'mock-devices-server.mjs'
    Start-Process -FilePath $node -ArgumentList @('"' + $scriptPath + '"') -WorkingDirectory $workspace -WindowStyle Hidden -RedirectStandardOutput (Join-Path $scratch 'server.log') -RedirectStandardError (Join-Path $scratch 'server-error.log')
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        if (PanelReady) { break }
        Start-Sleep -Milliseconds 150
    }
}
if (!(PanelReady)) { throw 'Local test panel did not start. Check scratch/mock-panel/server-error.log.' }
Start-Process -FilePath $panelUrl -WindowStyle Normal
