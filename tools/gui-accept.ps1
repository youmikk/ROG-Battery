<#
gui-accept.ps1 -- acceptance harness for the ROG mouse GUI.

Pure test tool: it never modifies the project. It launches the exe, inspects the
window tree, screenshots the main window, pokes the self-drawn surface, checks
tray-icon registration, and finally makes sure the process is gone.

Checks (one PASS / FAIL / SKIP line each):
  1 process alive            launched exe still alive after the settle window
  2 no console window        no ConsoleWindowClass / PseudoConsoleWindow for the pid
  3 main window              title match, client area >= MinWidth x MinHeight
  4 surface child            child of class RogMouseCtlSurface, size ~= client area
  5 window not blank         PrintWindow(PW_RENDERFULLCONTENT) capture, > MinColors colors
  6 tray icon registered     Shell_NotifyIconGetRect on the message-only hwnd, hr == 0
  7 five pages switch        click the rail nav items, 5 screenshots must all differ
  8 clean exit               WM_CLOSE, else force kill; process must be gone

Usage:
  powershell -ExecutionPolicy Bypass -File tools\gui-accept.ps1
  powershell -ExecutionPolicy Bypass -File tools\gui-accept.ps1 -Verbose
  powershell -ExecutionPolicy Bypass -File tools\gui-accept.ps1 -SkipLaunch -KeepAlive
  powershell -ExecutionPolicy Bypass -File tools\gui-accept.ps1 -Exe .\other.exe -OutDir .\scratch\shots

Exit code: 0 = no FAIL, 1 = at least one FAIL.

This file is deliberately pure ASCII (no CJK) so Windows PowerShell 5.1 can not
mis-decode it as GBK; see the encoding note at the bottom of the file.
#>

[CmdletBinding()]
param(
    [string] $Exe = 'D:\codex\rog-mouse-ctl\target\release\rog-mouse-ctl.exe',
    [string] $OutDir = 'D:\codex\rog-mouse-ctl\scratch\gui-shots',
    [switch] $KeepAlive,
    [switch] $SkipLaunch,
    [Alias('Pid')] [int] $TargetPid = 0,
    [int] $SettleMs = 5000,
    [int] $ClickSettleMs = 300,
    [int] $MinWidth = 1000,
    [int] $MinHeight = 640,
    [int] $MinColors = 20,
    [int] $SizeTolerance = 8
)

$ErrorActionPreference = 'Stop'

# ---------------------------------------------------------------- constants --
# The window title is 'R Battery' (plain ASCII, matches the Store display name).
$WindowTitle    = 'R Battery'
$SurfaceClass   = 'RogMouseCtlSurface'
$MsgWindowClass = 'NativeWindowsGuiWindow'

# Rail-mode left navigation, logical pixels (source: src\ui\layout.rs, src\ui\pages.rs):
#   nav width 56, item height 38, 1px gap, first item at y=14, item box is 38 wide
#   centred in 56 => x = 9..47, centre x = 28.
$NavCenterX   = 28.0
$NavFirstY    = 14.0
$NavItemH     = 38.0
$NavItemGap   = 1.0

$PageNames = @('battery', 'perf', 'light', 'keys', 'settings')

Write-Host '== gui-accept ==' ((Get-Date).ToString('yyyy-MM-dd HH:mm:ss'))
Write-Verbose ('exe    = ' + $Exe)
Write-Verbose ('outdir = ' + $OutDir)

# ------------------------------------------------------------- Win32 bridge --
# One Add-Type block; PowerShell 5.1 can not call [DllImport] directly.
if (-not ('RogAccept' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public static class RogAccept
{
    public const uint WM_CLOSE       = 0x0010;
    public const uint WM_LBUTTONDOWN = 0x0201;
    public const uint WM_LBUTTONUP   = 0x0202;
    public const int  SW_RESTORE     = 9;
    public const uint SRCCOPY        = 0x00CC0020;
    public const uint RDW_FLAGS      = 0x0001 | 0x0100 | 0x0080; // INVALIDATE|UPDATENOW|ALLCHILDREN

    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }

    // NOTIFYICONIDENTIFIER on x64:
    //   cbSize(4) + pad(4) + hWnd(8) + uID(4) + pad(4) + guidItem(16) = 40 bytes
    [StructLayout(LayoutKind.Sequential, Pack = 8)]
    public struct NOTIFYICONIDENTIFIER
    {
        public uint cbSize;
        public IntPtr hWnd;
        public uint uID;
        public Guid guidItem;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct STARTUPINFO
    {
        public int cb;
        public string lpReserved;
        public string lpDesktop;
        public string lpTitle;
        public int dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
        public short wShowWindow;
        public short cbReserved2;
        public IntPtr lpReserved2;
        public IntPtr hStdInput;
        public IntPtr hStdOutput;
        public IntPtr hStdError;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct PROCESS_INFORMATION
    {
        public IntPtr hProcess;
        public IntPtr hThread;
        public int dwProcessId;
        public int dwThreadId;
    }

    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr FindWindowW(string lpClassName, string lpWindowName);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr FindWindowExW(IntPtr hWndParent, IntPtr hWndChildAfter, string lpszClass, string lpszWindow);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetClassNameW(IntPtr hWnd, StringBuilder lpClassName, int nMaxCount);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetWindowTextW(IntPtr hWnd, StringBuilder lpString, int nMaxCount);

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);

    [DllImport("user32.dll")]
    public static extern bool GetClientRect(IntPtr hWnd, out RECT lpRect);

    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr SendMessageW(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern bool PostMessageW(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdcBlt, uint nFlags);

    [DllImport("user32.dll")]
    public static extern IntPtr GetDC(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern int ReleaseDC(IntPtr hWnd, IntPtr hDC);

    [DllImport("gdi32.dll")]
    public static extern IntPtr CreateCompatibleDC(IntPtr hdc);

    [DllImport("gdi32.dll")]
    public static extern IntPtr CreateCompatibleBitmap(IntPtr hdc, int nWidth, int nHeight);

    [DllImport("gdi32.dll")]
    public static extern IntPtr SelectObject(IntPtr hdc, IntPtr hgdiobj);

    [DllImport("gdi32.dll")]
    public static extern bool DeleteObject(IntPtr hObject);

    [DllImport("gdi32.dll")]
    public static extern bool DeleteDC(IntPtr hdc);

    [DllImport("gdi32.dll")]
    public static extern bool BitBlt(IntPtr hdcDest, int x, int y, int w, int h, IntPtr hdcSrc, int x1, int y1, uint rop);

    [DllImport("shell32.dll")]
    public static extern int Shell_NotifyIconGetRect(ref NOTIFYICONIDENTIFIER identifier, out RECT iconLocation);

    [DllImport("user32.dll")]
    public static extern bool IsWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool RedrawWindow(IntPtr hWnd, IntPtr lprcUpdate, IntPtr hrgnUpdate, uint flags);

    [DllImport("user32.dll")]
    public static extern uint GetDpiForSystem();

    [DllImport("user32.dll")]
    public static extern uint GetDpiForWindow(IntPtr hWnd);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool CreateProcessW(string lpApplicationName, StringBuilder lpCommandLine,
        IntPtr lpProcessAttributes, IntPtr lpThreadAttributes, bool bInheritHandles, uint dwCreationFlags,
        IntPtr lpEnvironment, string lpCurrentDirectory, ref STARTUPINFO lpStartupInfo,
        out PROCESS_INFORMATION lpProcessInformation);

    [DllImport("kernel32.dll")]
    public static extern bool CloseHandle(IntPtr hObject);

    // ------------------------------------------------------------- helpers --
    public static string ClassOf(IntPtr h)
    {
        StringBuilder sb = new StringBuilder(256);
        if (GetClassNameW(h, sb, sb.Capacity) <= 0) return "";
        return sb.ToString();
    }

    public static string TitleOf(IntPtr h)
    {
        StringBuilder sb = new StringBuilder(512);
        if (GetWindowTextW(h, sb, sb.Capacity) <= 0) return "";
        return sb.ToString();
    }

    public static uint PidOf(IntPtr h)
    {
        uint p;
        GetWindowThreadProcessId(h, out p);
        return p;
    }

    public static bool ClientSize(IntPtr h, out int w, out int hh)
    {
        RECT r;
        w = 0; hh = 0;
        if (!GetClientRect(h, out r)) return false;
        w = r.Right - r.Left;
        hh = r.Bottom - r.Top;
        return true;
    }

    public static bool WindowSize(IntPtr h, out int w, out int hh)
    {
        RECT r;
        w = 0; hh = 0;
        if (!GetWindowRect(h, out r)) return false;
        w = r.Right - r.Left;
        hh = r.Bottom - r.Top;
        return true;
    }

    public static string RectText(IntPtr h)
    {
        RECT r;
        if (!GetWindowRect(h, out r)) return "?";
        return string.Format("{0},{1} {2}x{3}", r.Left, r.Top, r.Right - r.Left, r.Bottom - r.Top);
    }

    // All top-level windows owned by pid (as hwnd strings).
    public static string[] TopWindows(uint pid)
    {
        List<string> res = new List<string>();
        EnumWindows(delegate(IntPtr h, IntPtr l)
        {
            if (PidOf(h) == pid) res.Add(h.ToInt64().ToString());
            return true;
        }, IntPtr.Zero);
        return res.ToArray();
    }

    public static string[] AllTopWindows()
    {
        List<string> res = new List<string>();
        EnumWindows(delegate(IntPtr h, IntPtr l)
        {
            res.Add(h.ToInt64().ToString());
            return true;
        }, IntPtr.Zero);
        return res.ToArray();
    }

    // Console window classes seen in the wild:
    //   ConsoleWindowClass          classic conhost
    //   PseudoConsoleWindow         conhost under ConPTY (Windows 11 default here)
    //   CASCADIA_HOSTING_WINDOW_CLASS  Windows Terminal
    public static bool IsConsoleClass(string c)
    {
        return c == "ConsoleWindowClass" || c == "PseudoConsoleWindow" || c == "CASCADIA_HOSTING_WINDOW_CLASS";
    }

    public static string[] ConsoleWindows(uint pid)
    {
        List<string> res = new List<string>();
        EnumWindows(delegate(IntPtr h, IntPtr l)
        {
            if (PidOf(h) != pid) return true;
            string c = ClassOf(h);
            if (IsConsoleClass(c)) res.Add(c);
            return true;
        }, IntPtr.Zero);
        return res.ToArray();
    }

    // Sanity probe: how many console windows exist system-wide right now. If this
    // is 0 the detector itself may be blind on this machine.
    public static int SystemConsoleCount()
    {
        int n = 0;
        EnumWindows(delegate(IntPtr h, IntPtr l)
        {
            if (IsConsoleClass(ClassOf(h))) n++;
            return true;
        }, IntPtr.Zero);
        return n;
    }

    // Direct children of a window, as hwnd strings.
    public static string[] ChildrenOf(IntPtr parent)
    {
        List<string> res = new List<string>();
        IntPtr prev = IntPtr.Zero;
        for (int i = 0; i < 1000; i++)
        {
            IntPtr h = FindWindowExW(parent, prev, null, null);
            if (h == IntPtr.Zero) break;
            res.Add(h.ToInt64().ToString());
            prev = h;
        }
        return res.ToArray();
    }

    public static IntPtr FirstChild(IntPtr parent, string cls)
    {
        return FindWindowExW(parent, IntPtr.Zero, cls, null);
    }

    // Message-only windows (parent HWND_MESSAGE = -3) are NOT reachable with
    // EnumWindows. Enumerating with a null class filter and filtering by pid
    // ourselves is the reliable form: a bare FindWindowExW(-3, 0, cls, null) can
    // return 0 and can also hand back another process' window.
    public static string[] MessageWindows(uint pid, string cls)
    {
        List<string> res = new List<string>();
        IntPtr prev = IntPtr.Zero;
        for (int i = 0; i < 6000; i++)
        {
            IntPtr h = FindWindowExW(new IntPtr(-3), prev, null, null);
            if (h == IntPtr.Zero) break;
            prev = h;
            if (PidOf(h) != pid) continue;
            if (cls != null && cls.Length > 0 && ClassOf(h) != cls) continue;
            res.Add(h.ToInt64().ToString());
        }
        return res.ToArray();
    }

    public static int NidSize()
    {
        return Marshal.SizeOf(typeof(NOTIFYICONIDENTIFIER));
    }

    public static void TrayRect(IntPtr hwnd, out int hr, out int left, out int top, out int right, out int bottom)
    {
        NOTIFYICONIDENTIFIER nid = new NOTIFYICONIDENTIFIER();
        nid.cbSize = (uint)Marshal.SizeOf(typeof(NOTIFYICONIDENTIFIER));
        nid.hWnd = hwnd;
        nid.uID = 0;
        nid.guidItem = Guid.Empty;
        RECT rc = new RECT();
        hr = Shell_NotifyIconGetRect(ref nid, out rc);
        left = rc.Left; top = rc.Top; right = rc.Right; bottom = rc.Bottom;
    }

    public static int Launch(string exe, string workDir, out string error)
    {
        error = "";
        STARTUPINFO si = new STARTUPINFO();
        si.cb = Marshal.SizeOf(typeof(STARTUPINFO));
        PROCESS_INFORMATION pi = new PROCESS_INFORMATION();
        StringBuilder cmd = new StringBuilder();
        cmd.Append('"').Append(exe).Append('"');
        uint flags = 0x00000010;   // CREATE_NEW_CONSOLE: same as a desktop double-click
        bool ok = CreateProcessW(null, cmd, IntPtr.Zero, IntPtr.Zero, false, flags,
            IntPtr.Zero, workDir, ref si, out pi);
        if (!ok)
        {
            error = "CreateProcessW failed, win32=" + Marshal.GetLastWin32Error().ToString();
            return -1;
        }
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
        return pi.dwProcessId;
    }

    public static void FocusWindow(IntPtr h)
    {
        ShowWindow(h, SW_RESTORE);
        SetForegroundWindow(h);
    }

    public static void RedrawNow(IntPtr h)
    {
        RedrawWindow(h, IntPtr.Zero, IntPtr.Zero, RDW_FLAGS);
    }

    // Left click at client coordinates of h. Down and up are sent separately so
    // the child WNDPROC runs its normal press/release path.
    public static void ClickAt(IntPtr h, int x, int y)
    {
        IntPtr lp = (IntPtr)((y << 16) | (x & 0xFFFF));
        SendMessageW(h, WM_LBUTTONDOWN, (IntPtr)1, lp);   // MK_LBUTTON
        SendMessageW(h, WM_LBUTTONUP, IntPtr.Zero, lp);
    }

    public static void CloseWindow(IntPtr h)
    {
        PostMessageW(h, WM_CLOSE, IntPtr.Zero, IntPtr.Zero);
    }
}
'@
}

Add-Type -AssemblyName System.Drawing

# ------------------------------------------------------------------ helpers --
$script:Results = New-Object System.Collections.ArrayList

function Add-Result {
    param(
        [Parameter(Mandatory = $true)][ValidateSet('PASS', 'FAIL', 'SKIP')][string] $Status,
        [Parameter(Mandatory = $true)][string] $Name,
        [string] $Detail = ''
    )
    $line = '[{0}] {1,-26} {2}' -f $Status, $Name, $Detail
    Write-Host $line
    [void]$script:Results.Add((New-Object psobject -Property @{
        Status = $Status
        Name   = $Name
        Detail = $Detail
    }))
}

function Test-ProcessAlive {
    param([int] $ProcId)
    if ($ProcId -le 0) { return $false }
    return ($null -ne (Get-Process -Id $ProcId -ErrorAction SilentlyContinue))
}

function Format-Hwnd {
    param([IntPtr] $Hwnd)
    if ($Hwnd -eq [IntPtr]::Zero) { return '0x0' }
    return ('0x{0:X}' -f $Hwnd.ToInt64())
}

function Find-MainWindow {
    <#
        Locate the top-level GUI window. Prefers a window owned by ProcId whose
        title matches exactly; falls back to a title-only lookup (useful with
        -SkipLaunch when the pid is not known yet).
    #>
    param(
        [int] $ProcId,
        [string] $Title
    )
    $byPid = [IntPtr]::Zero
    if ($ProcId -gt 0) {
        foreach ($s in [RogAccept]::TopWindows([uint32]$ProcId)) {
            $h = [IntPtr][int64]$s
            if ([RogAccept]::TitleOf($h) -eq $Title) {
                if ([RogAccept]::IsWindowVisible($h)) { return $h }
                if ($byPid -eq [IntPtr]::Zero) { $byPid = $h }
            }
        }
        if ($byPid -ne [IntPtr]::Zero) { return $byPid }
    }
    $h2 = [RogAccept]::FindWindowW($null, $Title)
    if ($h2 -ne [IntPtr]::Zero) { return $h2 }
    return [IntPtr]::Zero
}

function Get-PixelStats {
    <#
        Sample the bitmap on a grid and return distinct-colour count plus an FNV-1a
        hash of the sampled pixels. LockBits + Marshal.Copy keeps this fast enough
        for 1100x740 frames.
    #>
    param(
        [Parameter(Mandatory = $true)][System.Drawing.Bitmap] $Bitmap,
        [int] $TargetSamples = 12000
    )
    $w = $Bitmap.Width
    $h = $Bitmap.Height
    $rect = New-Object System.Drawing.Rectangle -ArgumentList 0, 0, $w, $h
    $mode = [System.Drawing.Imaging.ImageLockMode]::ReadOnly
    $fmt = [System.Drawing.Imaging.PixelFormat]::Format32bppArgb
    $data = $Bitmap.LockBits($rect, $mode, $fmt)
    # Normalise: a bottom-up DIB reports a negative stride, we always read rows
    # from the top of the buffer.
    $stride = [Math]::Abs($data.Stride)
    $bytes = [byte[]]::new($stride * $h)
    try {
        [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $bytes, 0, $bytes.Length)
    }
    finally {
        $Bitmap.UnlockBits($data)
    }

    $step = [int][Math]::Max(1, [Math]::Floor([Math]::Sqrt(($w * $h) / [double]$TargetSamples)))
    $colors = New-Object 'System.Collections.Generic.HashSet[int]'
    $hash = [uint64]2166136261
    $mask = [uint64]0xFFFFFFFF

    for ($y = 0; $y -lt $h; $y += $step) {
        $row = $y * $stride
        for ($x = 0; $x -lt $w; $x += $step) {
            $i = $row + ($x * 4)
            $b = [int]$bytes[$i]
            $g = [int]$bytes[$i + 1]
            $r = [int]$bytes[$i + 2]
            $key = ($r -shl 16) -bor ($g -shl 8) -bor $b
            [void]$colors.Add($key)
            $hash = ((($hash -bxor [uint64]$key) * 16777619) -band $mask)
        }
    }

    return (New-Object psobject -Property @{
        Distinct = $colors.Count
        Hash     = ('{0:X8}' -f $hash)
        Step     = $step
    })
}

function Save-WindowPng {
    <#
        Capture a window to PNG. Primary path is PrintWindow with
        PW_RENDERFULLCONTENT (2) which reads the DWM redirection surface, so it
        sees child windows and is not polluted by whatever is on top.
        Never uses CopyFromScreen. The GDI path (window DC -> memory DC -> bitmap)
        is only a last-resort fallback and is flagged as occlusion-prone.
    #>
    param(
        [Parameter(Mandatory = $true)][IntPtr] $Hwnd,
        [Parameter(Mandatory = $true)][string] $Path,
        [switch] $ClientArea
    )

    if (-not [RogAccept]::IsWindow($Hwnd)) { throw 'not a window: ' + (Format-Hwnd $Hwnd) }

    $w = 0; $h = 0
    if ($ClientArea) {
        if (-not [RogAccept]::ClientSize($Hwnd, [ref]$w, [ref]$h)) { throw 'GetClientRect failed' }
    }
    else {
        if (-not [RogAccept]::WindowSize($Hwnd, [ref]$w, [ref]$h)) { throw 'GetWindowRect failed' }
    }
    if ($w -le 0 -or $h -le 0) { throw ('bad window size {0}x{1}' -f $w, $h) }

    $pf = [System.Drawing.Imaging.PixelFormat]::Format32bppArgb
    $bmp = New-Object System.Drawing.Bitmap -ArgumentList $w, $h, $pf
    $method = ''

    try {
        $ok = $false
        foreach ($flags in @([uint32]2, [uint32]0)) {
            $g = [System.Drawing.Graphics]::FromImage($bmp)
            $hdc = $g.GetHdc()
            try {
                $ok = [RogAccept]::PrintWindow($Hwnd, $hdc, $flags)
            }
            finally {
                $g.ReleaseHdc($hdc)
                $g.Dispose()
            }
            $method = 'PrintWindow(flags=' + $flags + ')'
            if ($ok) { break }
        }

        if (-not $ok) {
            # Fallback: window DC -> memory DC -> bitmap. Occlusion-prone, only here
            # so a broken PrintWindow still yields something to look at.
            $srcDc = [RogAccept]::GetDC($Hwnd)
            if ($srcDc -eq [IntPtr]::Zero) { throw 'GetDC failed' }
            $memDc = [RogAccept]::CreateCompatibleDC($srcDc)
            $memBmp = [RogAccept]::CreateCompatibleBitmap($srcDc, $w, $h)
            $old = [RogAccept]::SelectObject($memDc, $memBmp)
            $ok = [RogAccept]::BitBlt($memDc, 0, 0, $w, $h, $srcDc, 0, 0, [RogAccept]::SRCCOPY)
            $g2 = [System.Drawing.Graphics]::FromImage($bmp)
            $bdc = $g2.GetHdc()
            try {
                [void][RogAccept]::BitBlt($bdc, 0, 0, $w, $h, $memDc, 0, 0, [RogAccept]::SRCCOPY)
            }
            finally {
                $g2.ReleaseHdc($bdc)
                $g2.Dispose()
            }
            [void][RogAccept]::SelectObject($memDc, $old)
            [void][RogAccept]::DeleteObject($memBmp)
            [void][RogAccept]::DeleteDC($memDc)
            [void][RogAccept]::ReleaseDC($Hwnd, $srcDc)
            $method = 'BitBlt(window DC, occlusion-prone)'
        }

        $stats = Get-PixelStats -Bitmap $bmp
        $dir = Split-Path -Parent $Path
        if ($dir -and -not (Test-Path -LiteralPath $dir)) {
            [void](New-Item -ItemType Directory -Force -Path $dir)
        }
        $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    }
    finally {
        $bmp.Dispose()
    }

    return (New-Object psobject -Property @{
        Path     = $Path
        Width    = $w
        Height   = $h
        Method   = $method
        Distinct = $stats.Distinct
        Hash     = $stats.Hash
        Step     = $stats.Step
    })
}

function Get-SettingValue {
    param([string] $Name)
    $cfg = Join-Path $env:APPDATA 'rog-mouse-ctl\settings.ini'
    if (-not (Test-Path -LiteralPath $cfg)) { return $null }
    foreach ($line in (Get-Content -LiteralPath $cfg -Encoding UTF8)) {
        $t = $line.Trim()
        if ($t.StartsWith('#')) { continue }
        $eq = $t.IndexOf('=')
        if ($eq -lt 0) { continue }
        if ($t.Substring(0, $eq).Trim() -eq $Name) {
            return $t.Substring($eq + 1).Trim()
        }
    }
    return $null
}

# ---------------------------------------------------------------- preflight --
$ProcId      = 0
$MainHwnd    = [IntPtr]::Zero
$SurfaceHwnd = [IntPtr]::Zero

try {
    $dpi = 96
    try { $dpi = [int][RogAccept]::GetDpiForSystem() } catch { $dpi = 96 }
    if ($dpi -le 0) { $dpi = 96 }
    $scale = $dpi / 96.0
    Write-Verbose ('dpi={0} scale={1}' -f $dpi, $scale)
    Write-Verbose ('NOTIFYICONIDENTIFIER size = {0} bytes (expected 40 on x64)' -f [RogAccept]::NidSize())

    if (-not (Test-Path -LiteralPath $OutDir)) {
        [void](New-Item -ItemType Directory -Force -Path $OutDir)
    }
    Write-Verbose ('screenshots -> ' + $OutDir)

    # ---------------------------------------------------------------- launch --
    if ($SkipLaunch) {
        if ($TargetPid -gt 0) {
            $ProcId = $TargetPid
            Write-Verbose ('skip launch, using pid ' + $ProcId)
        }
        else {
            $h = Find-MainWindow -ProcId 0 -Title $WindowTitle
            if ($h -eq [IntPtr]::Zero) {
                throw ('no running window titled ''' + $WindowTitle + '''; use -Exe to launch or -Pid')
            }
            $ProcId = [int][RogAccept]::PidOf($h)
            Write-Verbose ('skip launch, discovered pid ' + $ProcId)
        }
    }
    else {
        if (-not (Test-Path -LiteralPath $Exe -PathType Leaf)) {
            throw ('exe not found: ' + $Exe)
        }
        $exeFull = (Resolve-Path -LiteralPath $Exe).Path
        $workDir = Split-Path -Parent $exeFull
        $launchErr = ''
        $ProcId = [RogAccept]::Launch($exeFull, $workDir, [ref]$launchErr)
        if ($ProcId -le 0) { throw ('launch failed: ' + $launchErr) }
        Write-Verbose ('launched pid ' + $ProcId + ' from ' + $exeFull)
    }

    # ---- check 1 + 2 + locate the main window (one polling session) ----------
    $consoleSeen = @{}
    $deadline = (Get-Date).AddMilliseconds($SettleMs)
    while ((Get-Date) -lt $deadline) {
        if (-not (Test-ProcessAlive -ProcId $ProcId)) { break }
        foreach ($c in [RogAccept]::ConsoleWindows([uint32]$ProcId)) { $consoleSeen[$c] = $true }
        if ($MainHwnd -eq [IntPtr]::Zero) {
            $found = Find-MainWindow -ProcId $ProcId -Title $WindowTitle
            if ($found -ne [IntPtr]::Zero) {
                $MainHwnd = $found
                Write-Verbose ('main window appeared: ' + (Format-Hwnd $MainHwnd))
            }
        }
        Start-Sleep -Milliseconds 100
    }

    $alive = Test-ProcessAlive -ProcId $ProcId
    try {
        if ($alive) {
            Add-Result -Status 'PASS' -Name '1 process alive' -Detail ('pid={0}, alive after {1} ms' -f $ProcId, $SettleMs)
        }
        else {
            Add-Result -Status 'FAIL' -Name '1 process alive' -Detail ('pid={0} exited within {1} ms' -f $ProcId, $SettleMs)
        }
    }
    catch { Add-Result -Status 'FAIL' -Name '1 process alive' -Detail ('probe error: ' + $_.Exception.Message) }

    try {
        $finalConsole = @([RogAccept]::ConsoleWindows([uint32]$ProcId))
        $sanity = 0
        try { $sanity = [int][RogAccept]::SystemConsoleCount() } catch { }
        $transient = ''
        if ($consoleSeen.Count -gt 0) {
            $transient = '; transient during startup: ' + (($consoleSeen.Keys | Sort-Object) -join ',')
        }
        if ($finalConsole.Count -eq 0) {
            Add-Result -Status 'PASS' -Name '2 no console window' -Detail (
                'none now (checked ConsoleWindowClass/PseudoConsoleWindow/Cascadia){0}; system-wide console windows={1}' -f $transient, $sanity)
        }
        else {
            Add-Result -Status 'FAIL' -Name '2 no console window' -Detail (
                ('found {0}: {1}' -f $finalConsole.Count, (($finalConsole | Sort-Object -Unique) -join ',')) + $transient)
        }
    }
    catch { Add-Result -Status 'FAIL' -Name '2 no console window' -Detail ('probe error: ' + $_.Exception.Message) }

    # ----------------------------------------------------------- check 3 ------
    try {
        if ($MainHwnd -eq [IntPtr]::Zero -and $ProcId -gt 0) {
            $MainHwnd = Find-MainWindow -ProcId $ProcId -Title $WindowTitle
        }
        if ($MainHwnd -eq [IntPtr]::Zero) {
            Add-Result -Status 'FAIL' -Name '3 main window' -Detail ('no top-level window titled ''' + $WindowTitle + '''')
        }
        else {
            [RogAccept]::FocusWindow($MainHwnd) | Out-Null
            Start-Sleep -Milliseconds 200
            $w = 0; $h = 0
            [void][RogAccept]::ClientSize($MainHwnd, [ref]$w, [ref]$h)
            $detail = ('hwnd={0} title-matches, client={1}x{2} (need >={3}x{4}), window={5}' -f `
                (Format-Hwnd $MainHwnd), $w, $h, $MinWidth, $MinHeight, [RogAccept]::RectText($MainHwnd))
            if ($w -ge $MinWidth -and $h -ge $MinHeight) {
                Add-Result -Status 'PASS' -Name '3 main window' -Detail $detail
            }
            else {
                Add-Result -Status 'FAIL' -Name '3 main window' -Detail $detail
            }
        }
    }
    catch { Add-Result -Status 'FAIL' -Name '3 main window' -Detail ('probe error: ' + $_.Exception.Message) }

    # ----------------------------------------------------------- check 4 ------
    try {
        if ($MainHwnd -eq [IntPtr]::Zero) {
            Add-Result -Status 'FAIL' -Name '4 surface child' -Detail 'no main window to search under'
        }
        else {
            $SurfaceHwnd = [RogAccept]::FirstChild($MainHwnd, $SurfaceClass)
            if ($SurfaceHwnd -eq [IntPtr]::Zero) {
                $kids = @()
                foreach ($s in [RogAccept]::ChildrenOf($MainHwnd)) {
                    $kh = [IntPtr][int64]$s
                    $kids += ('{0}:{1}' -f [RogAccept]::ClassOf($kh), [RogAccept]::RectText($kh))
                }
                $kidText = if ($kids.Count -eq 0) { 'no children' } else { 'children=' + ($kids -join ' ') }
                Add-Result -Status 'FAIL' -Name '4 surface child' -Detail ("no child of class '$SurfaceClass' under " + (Format-Hwnd $MainHwnd) + '; ' + $kidText)
            }
            else {
                $mw = 0; $mh = 0
                [void][RogAccept]::ClientSize($MainHwnd, [ref]$mw, [ref]$mh)
                $sw = 0; $sh = 0
                [void][RogAccept]::ClientSize($SurfaceHwnd, [ref]$sw, [ref]$sh)
                $dw = [Math]::Abs($sw - $mw)
                $dh = [Math]::Abs($sh - $mh)
                $detail = ('hwnd={0} size={1}x{2} vs client {3}x{4} (d={5},{6}, tol={7})' -f `
                    (Format-Hwnd $SurfaceHwnd), $sw, $sh, $mw, $mh, $dw, $dh, $SizeTolerance)
                if ($dw -le $SizeTolerance -and $dh -le $SizeTolerance -and $sw -gt 0 -and $sh -gt 0) {
                    Add-Result -Status 'PASS' -Name '4 surface child' -Detail $detail
                }
                else {
                    Add-Result -Status 'FAIL' -Name '4 surface child' -Detail $detail
                }
            }
        }
    }
    catch { Add-Result -Status 'FAIL' -Name '4 surface child' -Detail ('probe error: ' + $_.Exception.Message) }

    # ----------------------------------------------------------- check 5 ------
    $mainPng = Join-Path $OutDir 'main.png'
    try {
        if ($MainHwnd -eq [IntPtr]::Zero) {
            Add-Result -Status 'FAIL' -Name '5 window not blank' -Detail 'no main window to capture'
        }
        else {
            $cap = Save-WindowPng -Hwnd $MainHwnd -Path $mainPng
            $detail = ('{0}x{1} via {2}, distinct colors={3} (need >{4}), hash={5}, file={6}' -f `
                $cap.Width, $cap.Height, $cap.Method, $cap.Distinct, $MinColors, $cap.Hash, $cap.Path)
            if ($cap.Distinct -gt $MinColors) {
                Add-Result -Status 'PASS' -Name '5 window not blank' -Detail $detail
            }
            else {
                Add-Result -Status 'FAIL' -Name '5 window not blank' -Detail $detail
            }
        }
    }
    catch { Add-Result -Status 'FAIL' -Name '5 window not blank' -Detail ('probe error: ' + $_.Exception.Message) }

    # ----------------------------------------------------------- check 6 ------
    try {
        $trayEnabled = Get-SettingValue -Name 'tray_enabled'
        if ($null -ne $trayEnabled -and $trayEnabled -match '^(false|0|no|off)$') {
            Add-Result -Status 'SKIP' -Name '6 tray icon' -Detail ('tray_enabled=' + $trayEnabled + ' in settings.ini, tray icon expected to be absent')
        }
        else {
            $msgWins = @([RogAccept]::MessageWindows([uint32]$ProcId, $MsgWindowClass))
            if ($msgWins.Count -eq 0) {
                $anyMsg = @([RogAccept]::MessageWindows([uint32]$ProcId, $null))
                Add-Result -Status 'FAIL' -Name '6 tray icon' -Detail (
                    "no message-only '$MsgWindowClass' window for pid $ProcId (message-only windows seen: $($anyMsg.Count))")
            }
            else {
                $best = $null
                $lastHr = 0
                foreach ($s in $msgWins) {
                    $mh = [IntPtr][int64]$s
                    $hr = 0; $l = 0; $t = 0; $r = 0; $b = 0
                    [RogAccept]::TrayRect($mh, [ref]$hr, [ref]$l, [ref]$t, [ref]$r, [ref]$b)
                    if ($best -eq $null -or ($hr -eq 0 -and $best.Hr -ne 0)) {
                        $best = New-Object psobject -Property @{
                            Hwnd = $mh; Hr = $hr; L = $l; T = $t; R = $r; B = $b
                        }
                    }
                    $lastHr = $hr
                }
                $detail = ('msg-hwnd={0} Shell_NotifyIconGetRect hr={1} (nid cbSize={2}), icon rect={3},{4} {5}x{6}' -f `
                    (Format-Hwnd $best.Hwnd), $best.Hr, [RogAccept]::NidSize(), $best.L, $best.T, ($best.R - $best.L), ($best.B - $best.T))
                if ($best.Hr -eq 0) {
                    Add-Result -Status 'PASS' -Name '6 tray icon' -Detail $detail
                }
                else {
                    Add-Result -Status 'FAIL' -Name '6 tray icon' -Detail ($detail + (' [last hr={0}; hresult 0x{1:X8}]' -f $lastHr, [uint32]$lastHr))
                }
            }
        }
    }
    catch { Add-Result -Status 'FAIL' -Name '6 tray icon' -Detail ('probe error: ' + $_.Exception.Message) }

    # ----------------------------------------------------------- check 7 ------
    try {
        if ($SurfaceHwnd -eq [IntPtr]::Zero -and $MainHwnd -ne [IntPtr]::Zero) {
            $SurfaceHwnd = [RogAccept]::FirstChild($MainHwnd, $SurfaceClass)
        }
        if ($SurfaceHwnd -eq [IntPtr]::Zero) {
            Add-Result -Status 'FAIL' -Name '7 five pages switch' -Detail ("no '$SurfaceClass' child; cannot click the rail nav")
        }
        elseif ($MainHwnd -eq [IntPtr]::Zero) {
            Add-Result -Status 'FAIL' -Name '7 five pages switch' -Detail 'no main window to capture'
        }
        else {
            $hashes = @()
            $shots = @()
            $clickLog = @()
            for ($i = 0; $i -lt 5; $i++) {
                $logicalX = $NavCenterX
                $logicalY = $NavFirstY + ($i * ($NavItemH + $NavItemGap)) + ($NavItemH / 2.0)
                $px = [int][Math]::Round($logicalX * $scale)
                $py = [int][Math]::Round($logicalY * $scale)
                [RogAccept]::ClickAt($SurfaceHwnd, $px, $py)
                $clickLog += ('p{0}@({1},{2})' -f ($i + 1), $px, $py)
                Start-Sleep -Milliseconds $ClickSettleMs
                [RogAccept]::RedrawNow($MainHwnd) | Out-Null
                Start-Sleep -Milliseconds 80
                $png = Join-Path $OutDir ('page{0}-{1}.png' -f ($i + 1), $PageNames[$i])
                $cap = Save-WindowPng -Hwnd $MainHwnd -Path $png
                $hashes += $cap.Hash
                $shots += $cap
                Write-Verbose ('page {0}: click ({1},{2}) -> {3} colors={4} hash={5}' -f `
                    ($i + 1), $px, $py, $cap.Path, $cap.Distinct, $cap.Hash)
            }
            $uniq = @($hashes | Sort-Object -Unique)
            $blankCount = @($shots | Where-Object { $_.Distinct -le $MinColors }).Count
            $detail = ('{0} shots, {1} distinct hashes (need 5), blank={2}; clicks={3}' -f `
                $shots.Count, $uniq.Count, $blankCount, ($clickLog -join ' '))
            if ($uniq.Count -eq 5) {
                Add-Result -Status 'PASS' -Name '7 five pages switch' -Detail $detail
            }
            else {
                $dup = ($hashes | Group-Object | Where-Object Count -gt 1 | ForEach-Object { $_.Name }) -join ','
                Add-Result -Status 'FAIL' -Name '7 five pages switch' -Detail ($detail + ('; duplicate hashes: ' + $dup))
            }
        }
    }
    catch { Add-Result -Status 'FAIL' -Name '7 five pages switch' -Detail ('probe error: ' + $_.Exception.Message) }

    # ----------------------------------------------------------- check 8 ------
    if ($KeepAlive) {
        Add-Result -Status 'SKIP' -Name '8 clean exit' -Detail ('-KeepAlive given; pid {0} left running for manual inspection' -f $ProcId)
    }
    else {
        try {
            $graceful = $false
            if ($MainHwnd -ne [IntPtr]::Zero -and [RogAccept]::IsWindow($MainHwnd)) {
                [RogAccept]::CloseWindow($MainHwnd)
                $closeDeadline = (Get-Date).AddMilliseconds(3000)
                while ((Get-Date) -lt $closeDeadline) {
                    if (-not (Test-ProcessAlive -ProcId $ProcId)) { $graceful = $true; break }
                    Start-Sleep -Milliseconds 100
                }
            }
            $hiddenToTray = ''
            if (-not $graceful -and (Test-ProcessAlive -ProcId $ProcId) -and $MainHwnd -ne [IntPtr]::Zero) {
                if (-not [RogAccept]::IsWindowVisible($MainHwnd)) { $hiddenToTray = '; window hid to tray (close_to_tray)' }
            }
            if (-not $graceful -and (Test-ProcessAlive -ProcId $ProcId)) {
                Stop-Process -Id $ProcId -Force -ErrorAction SilentlyContinue
                $killDeadline = (Get-Date).AddMilliseconds(2000)
                while ((Get-Date) -lt $killDeadline) {
                    if (-not (Test-ProcessAlive -ProcId $ProcId)) { break }
                    Start-Sleep -Milliseconds 50
                }
            }
            $gone = -not (Test-ProcessAlive -ProcId $ProcId)
            if ($gone) {
                if ($graceful) {
                    Add-Result -Status 'PASS' -Name '8 clean exit' -Detail ('WM_CLOSE terminated pid ' + $ProcId + ' within 3 s')
                }
                else {
                    Add-Result -Status 'PASS' -Name '8 clean exit' -Detail ('pid ' + $ProcId + ' gone, but only after Stop-Process -Force' + $hiddenToTray)
                }
            }
            else {
                Add-Result -Status 'FAIL' -Name '8 clean exit' -Detail ('pid ' + $ProcId + ' still alive after WM_CLOSE and Stop-Process -Force')
            }
        }
        catch { Add-Result -Status 'FAIL' -Name '8 clean exit' -Detail ('probe error: ' + $_.Exception.Message) }
    }
}
catch {
    # Anything that escapes an individual check still has to produce a verdict line.
    Add-Result -Status 'FAIL' -Name '0 harness' -Detail ($_.Exception.Message)
    Write-Verbose $_.ScriptStackTrace
}
finally {
    if (-not $KeepAlive) {
        if ($ProcId -gt 0 -and (Test-ProcessAlive -ProcId $ProcId)) {
            Write-Verbose ('cleanup: force killing pid ' + $ProcId)
            Stop-Process -Id $ProcId -Force -ErrorAction SilentlyContinue
        }
    }
}

# ---------------------------------------------------------------- summary ----
$passed = @($script:Results | Where-Object { $_.Status -eq 'PASS' }).Count
$failed = @($script:Results | Where-Object { $_.Status -eq 'FAIL' }).Count
$skipped = @($script:Results | Where-Object { $_.Status -eq 'SKIP' }).Count

Write-Host ''
if ($failed -gt 0) {
    Write-Host 'failed checks:'
    foreach ($r in $script:Results) {
        if ($r.Status -eq 'FAIL') { Write-Host ('  - ' + $r.Name + ': ' + $r.Detail) }
    }
}
if ($OutDir -and (Test-Path -LiteralPath $OutDir)) {
    Write-Host ('screenshots: ' + $OutDir)
}
Write-Host ('{0} passed, {1} failed, {2} skipped' -f $passed, $failed, $skipped)

if ($failed -gt 0) { exit 1 } else { exit 0 }

# Encoding note: this file is ASCII-only and saved as UTF-8 with BOM. Windows
# PowerShell 5.1 decodes a BOM-less .ps1 as GBK on this machine, which turns CJK
# comments into invalid tokens; pure ASCII sidesteps that entirely.
