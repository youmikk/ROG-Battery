param([int]$ProcessId)
$ErrorActionPreference = 'Stop'
for ($attempt = 0; $attempt -lt 100; $attempt++) {
    $instance = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if (!$instance) { exit 1 }
    if ($instance.MainWindowHandle -ne [IntPtr]::Zero) { exit 0 }
    Start-Sleep -Milliseconds 100
}
exit 2
