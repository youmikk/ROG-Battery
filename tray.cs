// tray.cs —— rog-battery 的托盘部分（纯 Win32 P/Invoke，不用 WinForms）
//
// 为什么不用 WinForms 的 NotifyIcon：
//   WinForms 版常驻约 28.8 MB，纯 Win32 只要约 17.8 MB —— 差 11 MB，没必要。
//
// 结构：
//   隐藏的 message-only 窗口  ← Shell_NotifyIcon 的回调目标
//   WM_TRAYICON               ← 鼠标点托盘图标时收到
//   定时器 (SetTimer)         ← 每隔 N 秒重新读电量并更新图标提示/绘图

using System;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Text;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

static class Tray
{
    // ---------- 常量 ----------
    const int WM_DESTROY = 0x0002;
    const int WM_CLOSE = 0x0010;
    const int WM_COMMAND = 0x0111;
    const int WM_LBUTTONUP = 0x0202;
    const int WM_LBUTTONDBLCLK = 0x0203;
    const int WM_RBUTTONUP = 0x0205;
    const int WM_TIMER = 0x0113;
    const int WM_CONTEXTMENU = 0x007B;
    const int WM_APP = 0x8000;
    const int WM_TRAYICON = WM_APP + 1;
    // DPI 线程 → 主线程：DPI 档位变了，请刷新托盘提示
    const int WM_DPI_CHANGED = WM_APP + 2;

    // 老版本（v3 语义）发来的鼠标通知，就是普通 WM_LBUTTONUP / WM_RBUTTONUP。
    // 不用 v4 那套 NIN_POPUPOPEN：切到 v4 之后 Shell 不再自动弹悬浮提示，
    // 实测用户把鼠标移上去什么都没有。悬停判断改成自己算（见 HoverTimerProc）。

    const int NIM_ADD = 0, NIM_MODIFY = 1, NIM_DELETE = 2;
    const int NIF_MESSAGE = 1, NIF_ICON = 2, NIF_TIP = 4, NIF_INFO = 0x10;
    const int MF_STRING = 0;
    const int MF_CHECKED = 0x8;
    const int MF_SEPARATOR = 0x800;
    const int TPM_RETURNCMD = 0x0100;
    const int ID_REFRESH = 1001, ID_EXIT = 1002, ID_AUTOSTART = 1003;
    const int TIMER_ID = 1;
    // 悬停探测。200ms 一次 GetCursorPos，开销约等于零（纯本地，不进内核等待）。
    // 之所以要这么勤：Shell 在鼠标停留约 500ms 后才弹提示，我们得赶在那之前
    // 把 DPI 读出来填进 szTip，读一次要 50~100ms，留出的余量刚好够。
    const int TIMER_HOVER = 2;
    const int HOVER_POLL_MS = 200;
    // 悬停期间复读 DPI 的间隔。用户悬停后可能按 DPI 键切档，
    // 而 Shell 不会主动来问新内容——只能自己定期重读。
    // 2 秒足够跟上手动切档的节奏；只在真停在图标上的那几秒里发生，代价可忽略。
    const int HOVER_REFRESH_MS = 2000;

    // explorer 重启（崩溃/更新/手动重启）后会向所有顶级窗口广播这条消息。
    // 托盘图标是登记在 explorer 里的，它一重启就全没了，必须重新 NIM_ADD。
    // 不处理的话图标会永久消失，用户只能杀进程重开。
    // 消息名是注册出来的，所以每次运行都要 RegisterWindowMessage 拿 ID。
    static uint WM_TASKBARCREATED;

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct NOTIFYICONDATA
    {
        public int cbSize;
        public IntPtr hWnd;
        public int uID;
        public int uFlags;
        public int uCallbackMessage;
        public IntPtr hIcon;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)]
        public string szTip;
        public int dwState;
        public int dwStateMask;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)]
        public string szInfo;
        public int uTimeoutOrVersion;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 64)]
        public string szInfoTitle;
        public int dwInfoFlags;
        public Guid guidItem;
        public IntPtr hBalloonIcon;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct WNDCLASSEX
    {
        public int cbSize, style;
        public IntPtr lpfnWndProc;
        public int cbClsExtra, cbWndExtra;
        public IntPtr hInstance, hIcon, hCursor, hbrBackground;
        public string lpszMenuName, lpszClassName;
        public IntPtr hIconSm;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct MSG
    {
        public IntPtr hwnd;
        public int message;
        public IntPtr wParam, lParam;
        public int time, ptX, ptY;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct POINT { public int X, Y; }

    [StructLayout(LayoutKind.Sequential)]
    struct RECT { public int Left, Top, Right, Bottom; }

    // 给 Shell_NotifyIconGetRect 用。Win8 起用 guidItem 或 (hWnd,uID) 都能定位。
    [StructLayout(LayoutKind.Sequential)]
    struct NOTIFYICONIDENTIFIER
    {
        public int cbSize;
        public IntPtr hWnd;
        public int uID;
        public Guid guidItem;
    }

    delegate IntPtr WndProcDelegate(IntPtr hWnd, int msg, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern ushort RegisterClassEx(ref WNDCLASSEX lpwcx);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateWindowEx(int exStyle, string cls, string name, int style,
        int x, int y, int w, int h, IntPtr parent, IntPtr menu, IntPtr inst, IntPtr param);
    [DllImport("user32.dll")]
    static extern IntPtr DefWindowProc(IntPtr hWnd, int msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")]
    static extern bool DestroyWindow(IntPtr hWnd);
    [DllImport("user32.dll")]
    static extern bool PeekMessage(out MSG msg, IntPtr hWnd, int min, int max, int remove);

    [DllImport("user32.dll")]
    static extern bool GetMessage(out MSG msg, IntPtr hWnd, int min, int max);
    [DllImport("user32.dll")]
    static extern bool TranslateMessage(ref MSG msg);
    [DllImport("user32.dll")]
    static extern IntPtr DispatchMessage(ref MSG msg);
    [DllImport("user32.dll")]
    static extern bool PostQuitMessage(int code);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern bool PostMessage(IntPtr hWnd, int msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")]
    static extern UIntPtr SetTimer(IntPtr hWnd, UIntPtr id, uint ms, IntPtr proc);
    [DllImport("user32.dll")]
    static extern bool KillTimer(IntPtr hWnd, UIntPtr id);
    [DllImport("user32.dll")]
    static extern IntPtr CreatePopupMenu();
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern bool AppendMenu(IntPtr menu, int flags, int id, string text);
    [DllImport("user32.dll")]
    static extern bool DestroyMenu(IntPtr menu);
    [DllImport("user32.dll")]
    static extern int TrackPopupMenuEx(IntPtr menu, int flags, int x, int y, IntPtr hWnd, IntPtr tpm);
    [DllImport("user32.dll")]
    static extern bool GetCursorPos(out POINT p);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern uint RegisterWindowMessage(string name);
    [DllImport("user32.dll")]
    static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")]
    static extern IntPtr LoadIcon(IntPtr inst, IntPtr name);
    [DllImport("user32.dll")]
    static extern bool DestroyIcon(IntPtr icon);
    // 查托盘图标在屏幕上的位置（Win7+）。用来判断鼠标是不是停在图标上，
    // 纯本地调用，不碰任何设备，比轮询 HID 便宜得多。
    [DllImport("shell32.dll")]
    static extern int Shell_NotifyIconGetRect(ref NOTIFYICONIDENTIFIER id, out RECT rect);
    [DllImport("kernel32.dll")]
    static extern IntPtr GetModuleHandle(string name);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CloseHandle(IntPtr h);
    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    static extern bool Shell_NotifyIcon(int msg, ref NOTIFYICONDATA data);

    static WndProcDelegate _proc;      // 必须持有引用，否则委托被 GC 回收后回调崩溃
    static IntPtr _hwnd;
    static IntPtr _icon;               // 当前托盘图标（需手动 DestroyIcon，GDI 对象会泄漏）
    static NOTIFYICONDATA _nid;
    static int _intervalSec = 30;

    // winexe 没有控制台，诊断信息只能落盘。
    // 位置：优先 exe 同目录（跟 history.tsv 一样，本机实测 %TEMP% 有时写不进去），
    // 失败再退 %TEMP%。两个位置都试一遍，全失败就静默放弃（不能因为写日志崩掉托盘）。
    static string LogPath
    {
        get
        {
            if (_logPath != null) return _logPath;
            var candidates = new System.Collections.Generic.List<string>();
            try
            {
                string exe = System.Reflection.Assembly.GetEntryAssembly().Location;
                if (!string.IsNullOrEmpty(exe))
                {
                    string dir = System.IO.Path.GetDirectoryName(exe);
                    if (!string.IsNullOrEmpty(dir)) candidates.Add(System.IO.Path.Combine(dir, "rog-battery.log"));
                }
            }
            catch { }
            try { candidates.Add(System.IO.Path.Combine(System.IO.Path.GetTempPath(), "rog-battery.log")); }
            catch { }

            foreach (var c in candidates)
            {
                try
                {
                    System.IO.File.AppendAllText(c, "", System.Text.Encoding.UTF8);
                    _logPath = c;
                    return c;
                }
                catch { }
            }
            _logPath = "";      // 明确标记"写不了"，避免每次 Log 都重新探测
            return "";
        }
    }
    static string _logPath;

    /// <summary>诊断日志默认关闭。设 ROG_BATTERY_DEBUG=1 才写，避免长期常驻时反复写盘。</summary>
    static readonly bool DebugOn =
        Environment.GetEnvironmentVariable("ROG_BATTERY_DEBUG") == "1";

    static void Log(string s)
    {
        if (!DebugOn) return;
        try
        {
            string p = LogPath;
            if (string.IsNullOrEmpty(p)) return;
            // 用 UTF-8 写（带 BOM），这样 PowerShell 的 Get-Content 能自动识别，
            // 否则中文会被按 ANSI 读成乱码。
            System.IO.File.AppendAllText(p,
                DateTime.Now.ToString("HH:mm:ss.fff") + "  " + s + Environment.NewLine,
                new System.Text.UTF8Encoding(true));
        }
        catch { }
    }

    /// <summary>托盘主循环。intervalSec = 刷新间隔秒数。</summary>
    public static int Run(int intervalSec)
    {
        if (DebugOn)
        {
            try
            {
                System.IO.File.WriteAllText(LogPath,
                    "=== rog-battery 托盘启动 " + DateTime.Now + " ===\r\n",
                    new System.Text.UTF8Encoding(true));
            }
            catch { }
        }
        Log("Run(" + intervalSec + ") 进入");
        _intervalSec = Math.Max(2, intervalSec);

        var wc = new WNDCLASSEX();
        wc.cbSize = Marshal.SizeOf(typeof(WNDCLASSEX));
        wc.lpszClassName = "RogBatteryTrayWnd";
        _proc = WndProc;
        wc.lpfnWndProc = Marshal.GetFunctionPointerForDelegate(_proc);
        wc.hInstance = GetModuleHandle(null);
        if (RegisterClassEx(ref wc) == 0)
        {
            Log("RegisterClassEx 失败 err=" + Marshal.GetLastWin32Error());
            return 1;
        }
        Log("RegisterClassEx OK");

        // 注册 "TaskbarCreated"：explorer 重启后会广播，用它补回托盘图标
        WM_TASKBARCREATED = RegisterWindowMessage("TaskbarCreated");
        Log("TaskbarCreated 消息 id=" + WM_TASKBARCREATED);

        _hwnd = CreateWindowEx(0, wc.lpszClassName, "", 0, 0, 0, 0, 0,
            IntPtr.Zero, IntPtr.Zero, wc.hInstance, IntPtr.Zero);
        if (_hwnd == IntPtr.Zero)
        {
            Log("CreateWindowEx 失败 err=" + Marshal.GetLastWin32Error());
            return 1;
        }
        Log("CreateWindowEx OK hwnd=" + _hwnd);

        // 先读一次，用真实电量画首个图标。
        // DPI 不在这里读：启动时读一次要等 100ms 才出图标，而且多数时候没人看。
        // 第一次悬停时 ReadDpiNow 会补上。
        var st = RogBattery.Snapshot();
        Log("首次读数: " + (st == null ? "失败(设备未连接)" : st.Level + "% " + st.Voltage + "mV"));
        ApplyState(st);

        _nid = new NOTIFYICONDATA();
        _nid.cbSize = Marshal.SizeOf(typeof(NOTIFYICONDATA));
        _nid.hWnd = _hwnd;
        _nid.uID = 1;
        _nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        _nid.uCallbackMessage = WM_TRAYICON;
        _nid.hIcon = _icon;
        _nid.szTip = TipText(st, null);

        // 开机自启时程序往往比任务栏（explorer）先起来，此刻 NIM_ADD 必然失败。
        // 所以不能一次失败就退出，要重试最多 ~60 秒；期间照常响应消息。
        bool added = Shell_NotifyIcon(NIM_ADD, ref _nid);
        if (added)
        {
            Log("NIM_ADD OK 图标已注册 szTip=" + _nid.szTip.Replace("\n", " / "));
        }
        else
        {
            Log("NIM_ADD 首次失败 err=" + Marshal.GetLastWin32Error() + "，开始重试");
            AddIconWithRetry(30);
        }

        // 不要切到 NOTIFYICON_VERSION_4！
        // v4 的文档行为是「标准悬浮提示不再自动显示，应用要自己画」。
        // 切过去之后用户把鼠标移到图标上什么也看不到——实测就是这么坏的。
        // 这里必须留在默认版本（v3 语义），托盘提示才由 Shell 照常弹出。
        //
        // 那"悬停时才知道要读 DPI"怎么办？见下面的 hover 定时器：
        // 用 GetCursorPos + 图标矩形做本地判断，零设备通信。

        SetTimer(_hwnd, (UIntPtr)TIMER_ID, (uint)(_intervalSec * 1000), IntPtr.Zero);
        SetTimer(_hwnd, (UIntPtr)TIMER_HOVER, HOVER_POLL_MS, IntPtr.Zero);
        Log("定时器 " + _intervalSec + " 秒；悬停探测 " + HOVER_POLL_MS
            + " ms（本地坐标，不碰设备），进入消息循环");

        MSG msg;
        while (GetMessage(out msg, IntPtr.Zero, 0, 0))
        {
            TranslateMessage(ref msg);
            DispatchMessage(ref msg);
        }

        Log("消息循环结束，清理");
        Cleanup();
        return 0;
    }

    // ---------- DPI 查询 ----------
    // DPI 只在"用户要看到它"时才读：鼠标悬停到托盘图标上、或打开右键菜单。
    // 其余时间一条命令都不发。
    //
    // 演进过程（记下来免得以后又绕回去）：
    //   最早：常驻线程每秒问一次 → 每天 86,400 次 SetupAPI 枚举 + 259,200 条 HID
    //         命令，是电量查询的 90 倍。鼠标的 2.4G 收发器被这些无意义的往返
    //         一直叫醒，进不了深度休眠。
    //   后来：降到跟电量同频 30 秒 → 每天 2,880 次。好多了，但仍然是"没人看
    //         的时候也在读"。
    //   现在：干脆不轮询了。悬停（NIN_POPUPOPEN）时才现读一次。
    //         DPI 值本身只在悬浮提示里出现一行，不悬停就等于没人看。
    //
    // 用 NIN_POPUPOPEN 而不是轮询"鼠标是否在图标上"，是因为后者要么需要
    // 定时取鼠标坐标+图标矩形（又变成轮询），要么得装钩子（太重）。
    // 限流用的时间基准。不能用 Environment.TickCount：它是 int32，开机 49.7 天
    // 后溢出成负数；TickCount64 又好死不死是 .NET 4.5 才有的（本项目编译目标是
    // 4.0 的 csc）。Stopwatch 从进程启动开始计时，单调且够用。
    static readonly System.Diagnostics.Stopwatch _clock =
        System.Diagnostics.Stopwatch.StartNew();
    static long _lastDpiReadAt = -1;
    // 两次现读的最小间隔。悬停本身可能反复触发，一次点击也能引出多个
    // NIN_SELECT（实测左键一发 6 个），限流到 1 秒一次足够跟手了。
    const int DPI_MIN_GAP_MS = 1000;

    /// <summary>档位变了就重画提示（图标仍显示电量，不受影响）。</summary>
    /// <remarks>
    /// 这个函数跑在 DPI 线程上，而 _nid 是主线程的 Refresh 在改的。
    /// NOTIFYICONDATA 是结构体，`var b = _nid` 这种拷贝不是原子的——主线程
    /// 正在逐字段写的时候拷过来，可能拿到"新 hWnd + 旧 szTip"这种半成品，
    /// 再交给 Shell_NotifyIcon 就会把提示刷成乱码或者让图标闪一下。
    /// 所以这里只更新共享的 _lastDpi，真正的 NIM_MODIFY 丢给主线程去做。
    /// </remarks>
    static void OnDpiChanged(RogBattery.DpiInfo dpi)
    {
        // 值没变就别去动托盘提示，省得无谓刷新
        bool same = _lastDpi != null
            && _lastDpi.Tier == dpi.Tier
            && _lastDpi.Current == dpi.Current
            && _lastDpi.Values.Length == dpi.Values.Length;

        _lastDpi = dpi;
        if (same) return;

        Log("DPI → 第 " + dpi.Tier + " 档"
            + (dpi.Current <= 0 ? "" : " = " + dpi.Current));

        // 让主线程去发 NIM_MODIFY（PostMessage 本身是线程安全的）
        if (_hwnd != IntPtr.Zero) PostMessage(_hwnd, WM_DPI_CHANGED, IntPtr.Zero, IntPtr.Zero);
    }

    /// <summary>
    /// 鼠标是否停在托盘图标上。纯本地查询，零设备通信。
    /// </summary>
    /// <remarks>
    /// 为什么不用 NOTIFYICON_VERSION_4 的 NIN_POPUPOPEN：切到 v4 后 Shell 把悬浮
    /// 提示的显示责任交还给应用（标准 tooltip 不再自动弹），实测直接把提示弄没了。
    /// 我们只想"提前一点点知道用户要悬停"，不值得为此接管整个提示的绘制。
    ///
    /// 所以改回 v3 语义（Shell 全权负责弹提示），自己用 GetCursorPos 判断鼠标在不在
    /// 图标矩形里。Shell_NotifyIconGetRect 是 Win7 起的官方接口，拿到的就是图标
    /// 在屏幕上的实际位置，任何 DPI / 缩放 / 任务栏位置都不用自己算。
    /// </remarks>
    static bool CursorOnIcon()
    {
        if (_hwnd == IntPtr.Zero) return false;
        try
        {
            var id = new NOTIFYICONIDENTIFIER();
            id.cbSize = Marshal.SizeOf(typeof(NOTIFYICONIDENTIFIER));
            id.hWnd = _hwnd;
            id.uID = 1;
            RECT r;
            if (Shell_NotifyIconGetRect(ref id, out r) != 0) return false;  // 0 = S_OK
            POINT p;
            if (!GetCursorPos(out p)) return false;
            // 图标被折叠进溢出区时 Explorer 给的矩形可能落在弹层里，
            // 位置依然是对的，直接按矩形判定即可。
            return p.X >= r.Left && p.X < r.Right
                && p.Y >= r.Top  && p.Y < r.Bottom;
        }
        catch { return false; }
    }

    /// <summary>
    /// 悬停探测。200ms 一次本地坐标查询；光标停在图标上期间，按 HOVER_REFRESH_MS
    /// 的节奏持续复读 DPI。
    /// </summary>
    /// <remarks>
    /// 为什么要"持续复读"而不是"进入时读一次"：
    /// 用户悬停后可能按鼠标上的 DPI 键切档，而 Shell 弹出提示后不会主动再问我们
    /// 要内容——提示里的 DPI 就一直停在进入时那个值，必须离开再靠近才更新。
    /// 实测就是这么被发现的：悬停着切了档，看到的还是上一档。
    ///
    /// 但不能每 200ms 读一次（那是 5 次/秒，比最早的每秒轮询还费）。所以用
    /// HOVER_REFRESH_MS 单独限流：悬停期间每 2 秒复读一次，既跟得上手速，
    /// 又只在"用户真的停在图标上"那几秒里发生。
    ///
    /// 离开时把 _nextDpiAt 清零，下次进来立刻读，不用等满 2 秒。
    /// </remarks>
    static void HoverTick()
    {
        bool inside = CursorOnIcon();
        if (!inside)
        {
            _nextDpiAt = 0;
            return;
        }
        // 首次进入立即读；之后按 HOVER_REFRESH_MS 复读
        if (_nextDpiAt == 0)
        {
            _nextDpiAt = _clock.ElapsedMilliseconds + HOVER_REFRESH_MS;
            ReadDpiNow("悬停");
            return;
        }
        if (_clock.ElapsedMilliseconds < _nextDpiAt) return;
        _nextDpiAt = _clock.ElapsedMilliseconds + HOVER_REFRESH_MS;
        ReadDpiNow("悬停续读");
    }

    static long _nextDpiAt;

    /// <summary>
    /// 现读一次 DPI 并更新提示。
    /// 只在"用户要看"时调用：鼠标悬停到图标上、打开右键菜单。
    /// 读失败就静默保持旧值（鼠标休眠时读不到是常态，不该弹错）。
    /// </summary>
    static void ReadDpiNow(string why)
    {
        // 一次查询 50~100ms，这段时间消息循环是停的。悬停和点击都可能连着来
        // （实测一次左键能引出 6 个 NIN_SELECT），必须限流，否则托盘会卡。
        long now = _clock.ElapsedMilliseconds;
        // -1 表示还没读过，别拿它跟进程运行时长比（刚启动时 now 本身就小，
        // 会被误判成"刚读过"而白白吞掉第一次悬停）。
        if (_lastDpiReadAt >= 0 && now - _lastDpiReadAt < DPI_MIN_GAP_MS)
        {
            Log("DPI 限流，跳过 (" + why + ") 距上次 " + (now - _lastDpiReadAt) + " ms");
            return;
        }
        _lastDpiReadAt = now;
        try
        {
            var sw = System.Diagnostics.Stopwatch.StartNew();
            var dpi = RogBattery.SnapshotDpi();
            sw.Stop();
            if (dpi != null && dpi.Tier > 0)
            {
                Log("DPI 现读 (" + why + ") 用时 " + sw.ElapsedMilliseconds + " ms");
                OnDpiChanged(dpi);
            }
            else
            {
                Log("DPI 现读失败 (" + why + ") 用时 " + sw.ElapsedMilliseconds + " ms"
                    + "（鼠标休眠或未配对）");
            }
        }
        catch (Exception e) { Log("DPI 现读异常 (" + why + "): " + e.Message); }
    }

    /// <summary>每隔 2 秒重试注册图标，最多 tries 次（每轮抽出消息泵一下，别假死）。</summary>
    static void AddIconWithRetry(int tries)
    {
        for (int i = 0; i < tries; i++)
        {
            if (Shell_NotifyIcon(NIM_ADD, ref _nid))
            {
                Log("NIM_ADD 重试成功（第 " + (i + 1) + " 次）");
                return;
            }
            // 抽干消息队列，避免等待期间 UI 无响应
            MSG m;
            while (PeekMessage(out m, IntPtr.Zero, 0, 0, 0x0001 /*PM_REMOVE*/))
            {
                TranslateMessage(ref m);
                DispatchMessage(ref m);
            }
            System.Threading.Thread.Sleep(2000);
        }
        Log("NIM_ADD 重试全部失败，放弃（explorer 未运行？）");
    }

    /// <summary>
    /// 托盘区被重建（explorer 重启）后重新登记图标。
    /// explorer 还没完全起来时 NIM_ADD 会失败，所以失败就重试几次。
    /// 这段跑在消息循环里，不能长时间阻塞，因此重试间隔短于 AddIconWithRetry。
    /// </summary>
    static void ReAddIcon()
    {
        // explorer 刚重启时任务栏可能还没准备好，所以失败要等一会儿再试。
        // 每次等待都短（500ms），且循环上限有限，不至于把消息循环堵死。
        for (int i = 0; i < 10; i++)
        {
            var nid = _nid;
            nid.hIcon = _icon;
            if (Shell_NotifyIcon(NIM_ADD, ref nid))
            {
                // 图标刚登记，顺手把工具提示也补上（NIM_ADD 时 explorer 可能还没
                // 接受 szTip，所以再发一次 NIM_MODIFY 更保险）
                var b = _nid;
                b.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
                b.hIcon = _icon;
                b.szTip = TipText(_last, _lastDpi);
                Shell_NotifyIcon(NIM_MODIFY, ref b);
                Log("TaskbarCreated 后 NIM_ADD 成功（第 " + (i + 1) + " 次尝试）");
                return;
            }
            System.Threading.Thread.Sleep(500);
        }

        // 兜底：交给原来那个更长的重试（会自己泵消息，不至于假死）
        Log("TaskbarCreated 后重试 10 次仍失败，转长重试");
        AddIconWithRetry(30);
    }

    static void Cleanup()
    {
        if (_hwnd != IntPtr.Zero)
        {
            KillTimer(_hwnd, (UIntPtr)TIMER_ID);
            if (_nid.hWnd != IntPtr.Zero)
            {
                _nid.uFlags = 0;
                Shell_NotifyIcon(NIM_DELETE, ref _nid);
                _nid.hWnd = IntPtr.Zero;
            }
            DestroyWindow(_hwnd);
            _hwnd = IntPtr.Zero;
        }
        if (_icon != IntPtr.Zero) { DestroyIcon(_icon); _icon = IntPtr.Zero; }
    }

    /// <summary>把图标按各电量档位导出成 PNG（并放大 8 倍），用于肉眼检查渲染效果。</summary>
    internal static void DumpIcons(string dir)
    {
        System.IO.Directory.CreateDirectory(dir);
        int S = Math.Max(16, GetSystemMetrics(SM_CXSMICON));
        int[] levels = { 5, 9, 28, 42, 87, 100 };

        // 每种电量导出三态：使用中 / 充电中 / 已充满。
        // 光看"使用中"无法判断标记位置对不对，所以三态一起导。
        var variants = new[]
        {
            new { Name = "", Charge = RogBattery.ChargeState.Discharging },
            new { Name = "-chg", Charge = RogBattery.ChargeState.Charging },
            new { Name = "-full", Charge = RogBattery.ChargeState.Full },
        };

        foreach (var v in variants)
        foreach (int lv in levels)
        {
            var st = new RogBattery.State();
            st.Level = lv;
            st.Voltage = 4200;
            st.Charge = v.Charge;
            st.Charging = v.Charge == RogBattery.ChargeState.Charging;
            IntPtr h = MakeIcon(st);
            if (h == IntPtr.Zero) continue;
            try
            {
                using (var ic = Icon.FromHandle(h))
                using (var bmp = ic.ToBitmap())
                {
                    string stem = "icon-" + lv + v.Name;
                    bmp.Save(System.IO.Path.Combine(dir, stem + ".png"),
                        System.Drawing.Imaging.ImageFormat.Png);

                    using (var big = new Bitmap(bmp.Width * 8, bmp.Height * 8))
                    {
                        using (var g = Graphics.FromImage(big))
                        {
                            g.InterpolationMode =
                                System.Drawing.Drawing2D.InterpolationMode.NearestNeighbor;
                            g.PixelOffsetMode = System.Drawing.Drawing2D.PixelOffsetMode.Half;
                            g.DrawImage(bmp, 0, 0, big.Width, big.Height);
                        }
                        big.Save(System.IO.Path.Combine(dir, stem + "-x8.png"),
                            System.Drawing.Imaging.ImageFormat.Png);
                    }
                }
            }
            finally { DestroyIcon(h); }
        }
        Console.WriteLine("图标已导出到 " + dir + "（画布 " + S + "x" + S + "）");
    }

    static bool _refreshing;

    static void Refresh()
    {
        // 重入保护：TrackPopupMenuEx 会抽消息队列，菜单开着的时候 WM_TIMER 照样
        // 能进来；左键单击和"立即刷新"也可能和定时器撞在一起。Refresh 是同步的
        // （一次 HID 往返 100~200ms），重入会让两条命令互相抢应答。
        if (_refreshing) { Log("Refresh 重入，跳过本次"); return; }
        _refreshing = true;
        try
        {
            RefreshCore();
        }
        finally { _refreshing = false; }
    }

    static void RefreshCore()
    {
        var st = RogBattery.Snapshot();

        // 接收器偶尔会连续几次超时不响应（实测能持续一分钟），但这并不代表鼠标
        // 断开。瞬时失败时保留上一次的有效读数，避免图标在 100 / ? 之间来回跳。
        // 只有连续多次失败才认定真的读不到。
        //
        // 注意 failed 这个标志：显示可以沿用旧值，但**历史绝对不能写**。
        // 以前这里把 st 换成 _last 之后继续 RecordSample(st)，等于每 30 秒往
        // history.tsv 记一条"旧电量 + 新时间戳"。连续 3 次失败（90 秒）内都算
        // 有效样本，看起来就像"电量稳定保持"，斜率被拉平 → 预计时间越算越长，
        // 最后超过 720 小时被判成"异常（请忽略）"。
        bool failed = false;
        if (st != null)
        {
            _last = st;
            _failStreak = 0;
        }
        else
        {
            _failStreak++;
            failed = true;
            if (_last != null && _failStreak < 4) st = _last;
        }

        // DPI 用上一次现读的结果。这里**不去读设备**——DPI 只在悬停/打开菜单
        // 时才现读（见 ReadDpiNow），定时刷新只管电量。
        var dpi = _lastDpi;

        // 估算只在 refresh 线程里做：RecordSample 写文件、Predict 读文件，
        // 放两个线程会自己和自己抢文件。失败一律吞掉，不能因此影响托盘。
        //
        // failed=true 时既不写历史也不重算预测：那一轮根本没读到真实数据，
        // 写进去的是伪造样本。保留上一次的 _lastEst 即可。
        if (st != null && !failed)
        {
            try
            {
                RogBattery.RecordSample(st);
                _lastEst = RogBattery.Predict(st);
            }
            catch (Exception e) { Log("估算失败: " + e.Message); }
        }

        ApplyState(st);
        _nid.hIcon = _icon;
        _nid.szTip = TipText(st, dpi);
        _nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        bool ok = Shell_NotifyIcon(NIM_MODIFY, ref _nid);
        Log("刷新 " + (st == null ? "失败" : st.Level + "% " + st.Voltage + "mV")
            + " dpi=" + (dpi == null ? "?" : dpi.Current.ToString())
            + (st == null ? "" : " charge=" + st.Charge)
            + " est=" + (_lastEst == null ? "?" : (_lastEst.Known ? "known" : "none"))
            + " 连续失败=" + _failStreak + " NIM_MODIFY=" + ok);
    }

    static RogBattery.State _last;      // 上次成功读数
    static int _failStreak;             // 连续失败次数
    static RogBattery.DpiInfo _lastDpi; // 上次成功读到的 DPI
    static RogBattery.Estimate _lastEst; // 上次估算结果（可能 Known=false）

    /// <summary>生成/替换图标。旧的必须 DestroyIcon，否则长时间刷新会耗尽 GDI 句柄。</summary>
    static void ApplyState(RogBattery.State st)
    {
        IntPtr old = _icon;
        _icon = MakeIcon(st);
        if (old != IntPtr.Zero) DestroyIcon(old);
    }

    static string TipText(RogBattery.State st, RogBattery.DpiInfo dpi)
    {
        string dpiLine = "";
        if (dpi != null && dpi.Current > 0)
        {
            dpiLine = "\nDPI " + dpi.Current;
            if (dpi.Values.Length > 0 && dpi.Selected >= 0)
                dpiLine += "（第 " + (dpi.Selected + 1) + "/" + dpi.Values.Length + " 档）";
        }
        if (st == null)
            return "ROG 龙鳞 Ace Mini\n未连接（鼠标休眠或未配对）" + dpiLine;
        // Shell_NotifyIcon 的 szTip 上限 128 字符（含结尾 \0），且用 \n 换行
        string link = st.Mode == RogBattery.LinkMode.Wired ? "有线"
                    : st.Mode == RogBattery.LinkMode.Wireless24G ? "2.4G 无线"
                    : "未知";
        string charge = st.Charge == RogBattery.ChargeState.Charging ? "充电中"
                      : st.Charge == RogBattery.ChargeState.Full ? "已充满"
                      : "使用中";

        // 估算只在有结论时加一行；没结论（样本不足/电量没动）就不占地方。
        // szTip 只有 128 字符，中文每字算 1，最多再加一行。
        string estLine = "";
        try
        {
            var est = _lastEst;
            if (est != null && est.Known) estLine = "\n" + est.Text;
        }
        catch { }

        return "ROG 龙鳞 Ace Mini"
             + "\n连接 " + link + " · " + charge
             + "\n剩余电量 " + st.Level + "%"
             + estLine
             + "\n电压 " + st.Voltage + " mV"
             + dpiLine;
    }

    /// <summary>
    /// `--tip-selftest`：构造几种状态，检查悬浮提示的内容和长度。
    /// 真机要等电量真变化才能看到"预计…"那行，靠这个能立刻验证排版。
    /// 返回失败项数。
    /// </summary>
    internal static int TipSelfTest()
    {
        int fail = 0;
        var dpi = new RogBattery.DpiInfo();
        dpi.Values = new int[] { 400, 800, 1600, 3200 };
        dpi.Tier = 2;

        var st = new RogBattery.State
        {
            Level = 100, Voltage = 4185,
            Charge = RogBattery.ChargeState.Discharging,
            Mode = RogBattery.LinkMode.Wireless24G,
        };

        Action<string, string> check = delegate (string name, string text)
        {
            bool ok = text.Length <= 127;      // szTip 是 128 字符，留 1 个结尾 \0
            if (!ok) fail++;
            Console.WriteLine((ok ? "  OK   " : "  FAIL ") + name + "（" + text.Length + " 字符）");
            Console.WriteLine("         " + text.Replace("\n", " / "));
        };

        // 1) 有结论：应该多出一行"预计…"
        _lastEst = new RogBattery.Estimate { Known = true, Text = "预计还能用 12 小时 30 分钟" };
        string a = TipText(st, dpi);
        check("有结论", a);
        if (a.IndexOf("预计还能用 12 小时 30 分钟") < 0)
        {
            fail++; Console.WriteLine("  FAIL 结论那行没出现");
        }

        // 2) 无结论：不应该出现"预计…"那行
        _lastEst = new RogBattery.Estimate { Known = false, Text = "预计可用时间：样本不足" };
        string b = TipText(st, dpi);
        check("无结论", b);
        if (b.IndexOf("样本不足") >= 0)
        {
            fail++; Console.WriteLine("  FAIL 无结论时不该显示原因");
        }

        // 3) 充电中：文案应变成"后充满"
        _lastEst = new RogBattery.Estimate { Known = true, Text = "预计 1 小时 20 分钟后充满" };
        st.Charge = RogBattery.ChargeState.Charging;
        check("充电中", TipText(st, dpi));

        // 4) 最长的组合：充电 + 第 4 档 DPI + 结论，检查是否超 127
        dpi.Tier = 4;
        _lastEst = new RogBattery.Estimate { Known = true, Text = "预计 12 小时 30 分钟后充满" };
        check("最长组合", TipText(st, dpi));

        // 5) 未连接
        _lastEst = null;
        check("未连接", TipText(null, dpi));

        Console.WriteLine();
        Console.WriteLine(fail == 0 ? "全部通过 (5 项)" : fail + " 项失败");
        return fail;
    }

    [DllImport("user32.dll")]
    static extern int GetSystemMetrics(int index);
    const int SM_CXSMICON = 49;

    /// <summary>任务栏是否浅色主题。浅色底要黑字，深色底要白字——和 Windows 自己的时钟一致。</summary>
    static bool TaskbarUsesLightTheme()
    {
        try
        {
            using (var k = Microsoft.Win32.Registry.CurrentUser.OpenSubKey(
                @"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"))
            {
                if (k == null) return false;
                object v = k.GetValue("SystemUsesLightTheme");
                if (v is int) return (int)v != 0;
            }
        }
        catch { }
        return false;
    }

    // ---------- 图标绘制 ----------
    // 难点在于"把数字塞满 16x16 又不糊"。踩过的三个坑：
    //   1) 直接往 16x16 写字 → 锯齿（单色渲染像数字闹钟）
    //   2) 用 MeasureString 反推字号 → 它返回的是含上下留白的行高，数字墨迹
    //      只占约七成，算出来的字号必然偏小；极小字号下还会低估宽度，
    //      导致结果过宽被裁掉（曾经只画出 "10"）
    //   3) 逐级试字号 → 同样因测量不准而不可靠
    // 现在的做法：在 128px 上画出来，扫像素求真实墨迹包围盒，再按包围盒
    // 精确缩放到目标尺寸。既保证塞得满，又天然居中，还不依赖字体度量。
    const int REF = 128;          // 参考渲染字号
    const int PAD = 40;           // 参考画布留边，防止超宽时被截

    // 字体选择。两个维度同时要满足：够窄、够细。
    //  · 窄："100" 的天然宽高比 Segoe UI 是 2.04，16px 图标 12px 字高时可用宽
    //    只有 15.8px，Segoe UI 自然宽约 21px 要压到 0.75 倍，字明显变扁。
    //    Bahnschrift（DIN 风格窄体）只有 1.42，几乎不用压。
    //  · 细：Bahnschrift 的 Regular 是中等偏粗，16px 下笔画糊成一块发笨。
    //    Light Condensed 保留同样的窄度，笔画细一圈。
    // 候选按偏好从细到粗排列，逐个探测；都没有才退 Segoe UI。
    // （本机实测 16px 平均 alpha：Light Condensed 135-155，SemiLight 145-162，
    //   Condensed 156-174，Segoe UI Light 78-103 已细到发灰、不可用。）
    static readonly string IconFont = PickIconFont();

    static readonly string[] IconFontCandidates =
    {
        "Bahnschrift Light Condensed",
        "Bahnschrift SemiLight Condensed",
        "Bahnschrift Condensed",
        "Segoe UI",
    };

    static string PickIconFont()
    {
        try
        {
            using (var col = new System.Drawing.Text.InstalledFontCollection())
            {
                foreach (var want in IconFontCandidates)
                    foreach (var fam in col.Families)
                        if (fam.Name.Equals(want, StringComparison.OrdinalIgnoreCase))
                            return fam.Name;
            }
        }
        catch { }
        return "Segoe UI";
    }

    static IntPtr MakeIcon(RogBattery.State st)
    {
        int S = Math.Max(16, GetSystemMetrics(SM_CXSMICON));
        string txt = (st == null) ? "?" : st.Level.ToString();

        Color fill;
        if (st != null && st.Level < 30) fill = Color.FromArgb(232, 17, 35);   // 低电量红
        else if (TaskbarUsesLightTheme()) fill = Color.FromArgb(20, 20, 20);  // 浅色栏黑字
        else fill = Color.White;                                              // 深色栏白字

        // 要额外画充电标记时，右下角得让出一块地方，所以数字缩小并往左上挪。
        // 16px 里三位数本来就顶满宽度，不缩小就只能让闪电压在数字上。
        bool badge = st != null && st.Charge != RogBattery.ChargeState.Discharging;

        // 参照物：实测任务栏时钟 "22:44" 的墨迹高约 12px，笔画也细。所以目标字高
        // 定为 S * 0.75（16px 图标 = 12px），而不是把画布撑满——撑满会又高又瘦。
        // 有标记时压到 0.66（≈10.5px）：再小字就糊了，宁可让标记压在末位数字角上，
        // 和 Windows 自己的"电池+闪电"叠加画法一致。
        float targetH = S * (badge ? 0.66f : 0.75f);

        SizeF inkRef = InkSize("100", fill);     // 用三位数当基准，保证三者等高
        SizeF inkTxt = InkSize(txt, fill);
        if (inkRef.Width < 1f || inkRef.Height < 1f || inkTxt.Width < 1f || inkTxt.Height < 1f)
            return FallbackIcon(S, txt, fill);

        float sc = targetH / inkRef.Height;
        float w = inkTxt.Width * sc;
        float h = inkTxt.Height * sc;

        // 只压宽度，不动高度；留一点点边，别顶到画布边框
        float maxW = S * (badge ? 0.86f : 0.96f);
        if (w > maxW) w = maxW;

        // 有标记时整体往左上挪，把右下角腾给标记
        float cx = badge ? S * 0.45f : S * 0.5f;
        float cy = badge ? S * 0.45f : S * 0.5f;

        // 把 txt 的墨迹按上述尺寸画进 S x S
        using (var refBmp = RenderRef(txt, fill))
        using (var small = new Bitmap(S, S, System.Drawing.Imaging.PixelFormat.Format32bppArgb))
        {
            Rectangle ink = InkBounds(refBmp);
            using (var g2 = Graphics.FromImage(small))
            {
                g2.Clear(Color.Transparent);
                g2.InterpolationMode =
                    System.Drawing.Drawing2D.InterpolationMode.HighQualityBicubic;
                g2.PixelOffsetMode = System.Drawing.Drawing2D.PixelOffsetMode.HighQuality;
                g2.CompositingQuality =
                    System.Drawing.Drawing2D.CompositingQuality.HighQuality;

                g2.DrawImage(refBmp,
                    new RectangleF(cx - w / 2f, cy - h / 2f, w, h),
                    ink, GraphicsUnit.Pixel);
            }
            SharpenAlpha(small);

            if (badge)
                using (var g3 = Graphics.FromImage(small))
                    DrawChargeBadge(g3, S, st.Charge == RogBattery.ChargeState.Full);

            return small.GetHicon();
        }
    }

    /// <summary>
    /// 右下角的充电标记。
    /// 实测 16px 下"闪电"和"对勾"都会糊成无意义的色块——闪电看成加号、对勾看成笑脸，
    /// 所以只用最不容易糊的形状：充电中是实心圆点，已充满是同尺寸的空心圈。
    /// 圆点/圆环靠"中间有没有洞"区分，5px 下还能看出来；再复杂的形状就没了。
    /// </summary>
    static void DrawChargeBadge(Graphics g, int S, bool full)
    {
        float b = S * 0.34f;                 // 标记直径，16px 图标约 5.4px
        float ox = S - b - S * 0.02f, oy = S - b - S * 0.02f;

        g.SmoothingMode = SmoothingMode.AntiAlias;
        g.PixelOffsetMode = System.Drawing.Drawing2D.PixelOffsetMode.HighQuality;
        g.CompositingQuality = System.Drawing.Drawing2D.CompositingQuality.HighQuality;

        // 两种任务栏都能看清的绿：浅色底用深一点的绿，深色底用亮绿
        Color green = TaskbarUsesLightTheme()
            ? Color.FromArgb(0, 150, 60)
            : Color.FromArgb(60, 220, 110);

        if (full)
        {
            // 已充满：空心圈（同样的外径，中间挖空）
            using (var pen = new Pen(green, Math.Max(1f, b * 0.24f)))
                g.DrawEllipse(pen, ox, oy, b, b);
        }
        else
        {
            // 充电中：实心点
            using (var br = new SolidBrush(green))
                g.FillEllipse(br, ox, oy, b, b);
        }
    }

    /// <summary>在参考画布上画出文字（用于测量墨迹/作为缩放源）。</summary>
    static Bitmap RenderRef(string txt, Color fill)
    {
        var bmp = new Bitmap(REF * 3 + PAD * 2, REF * 2 + PAD * 2,
            System.Drawing.Imaging.PixelFormat.Format32bppArgb);
        using (var g = Graphics.FromImage(bmp))
        {
            g.Clear(Color.Transparent);
            g.SmoothingMode = SmoothingMode.AntiAlias;
            g.TextRenderingHint = TextRenderingHint.AntiAlias;
            g.PixelOffsetMode = System.Drawing.Drawing2D.PixelOffsetMode.HighQuality;
            using (var f = new Font(IconFont, REF, FontStyle.Regular, GraphicsUnit.Pixel))
            using (var b = new SolidBrush(fill))
            using (var sf = new StringFormat
            {
                Alignment = StringAlignment.Center,
                LineAlignment = StringAlignment.Center,
                FormatFlags = StringFormatFlags.NoWrap | StringFormatFlags.NoClip,
                Trimming = StringTrimming.None,
            })
                g.DrawString(txt, f, b,
                    new RectangleF(0, 0, bmp.Width, bmp.Height), sf);
        }
        return bmp;
    }

    /// <summary>扫描非透明像素，求墨迹包围盒。</summary>
    static Rectangle InkBounds(Bitmap bmp)
    {
        int minX = bmp.Width, minY = bmp.Height, maxX = -1, maxY = -1;
        var data = bmp.LockBits(new Rectangle(0, 0, bmp.Width, bmp.Height),
            System.Drawing.Imaging.ImageLockMode.ReadOnly,
            System.Drawing.Imaging.PixelFormat.Format32bppArgb);
        try
        {
            int stride = data.Stride;
            var buf = new byte[stride * bmp.Height];
            System.Runtime.InteropServices.Marshal.Copy(data.Scan0, buf, 0, buf.Length);
            for (int y = 0; y < bmp.Height; y++)
            {
                int row = y * stride;
                for (int x = 0; x < bmp.Width; x++)
                {
                    // 32bppArgb 在内存里是 BGRA，alpha 在 +3
                    if (buf[row + x * 4 + 3] > 24)
                    {
                        if (x < minX) minX = x;
                        if (x > maxX) maxX = x;
                        if (y < minY) minY = y;
                        if (y > maxY) maxY = y;
                    }
                }
            }
        }
        finally { bmp.UnlockBits(data); }

        if (maxX < 0) return Rectangle.Empty;
        return Rectangle.FromLTRB(minX, minY, maxX + 1, maxY + 1);
    }

    static SizeF InkSize(string txt, Color fill)
    {
        using (var bmp = RenderRef(txt, fill))
        {
            var r = InkBounds(bmp);
            return r.IsEmpty ? SizeF.Empty : new SizeF(r.Width, r.Height);
        }
    }

    /// <summary>
    /// 拉伸 alpha 对比度。缩小时双三次插值会把细笔画的两侧摊成半透明灰，
    /// 16px 下肉眼就是"发虚"。这里把淡边清掉、正色提实，中间线性过渡，
    /// 笔画宽度不变但边缘变干净。实测 40/150 效果最好（70/180 会啃掉细笔）。
    /// </summary>
    static void SharpenAlpha(Bitmap bmp)
    {
        const int lo = 40, hi = 150;
        var rect = new Rectangle(0, 0, bmp.Width, bmp.Height);
        var data = bmp.LockBits(rect, System.Drawing.Imaging.ImageLockMode.ReadWrite,
            System.Drawing.Imaging.PixelFormat.Format32bppArgb);
        try
        {
            int stride = data.Stride;
            var buf = new byte[stride * bmp.Height];
            System.Runtime.InteropServices.Marshal.Copy(data.Scan0, buf, 0, buf.Length);
            for (int i = 3; i < buf.Length; i += 4)
            {
                int a = buf[i];
                if (a <= lo) a = 0;
                else if (a >= hi) a = 255;
                else a = (a - lo) * 255 / (hi - lo);
                buf[i] = (byte)a;
            }
            System.Runtime.InteropServices.Marshal.Copy(buf, 0, data.Scan0, buf.Length);
        }
        finally { bmp.UnlockBits(data); }
    }

    /// <summary>兜底：测量失败时退回最简单的小字绘制。</summary>
    static IntPtr FallbackIcon(int S, string txt, Color fill)
    {
        using (var bmp = new Bitmap(S, S))
        {
            using (var g = Graphics.FromImage(bmp))
            {
                g.Clear(Color.Transparent);
                g.TextRenderingHint = TextRenderingHint.AntiAlias;
                using (var f = new Font("Segoe UI", S * 0.7f, FontStyle.Regular,
                    GraphicsUnit.Pixel))
                using (var b = new SolidBrush(fill))
                using (var sf = new StringFormat
                {
                    Alignment = StringAlignment.Center,
                    LineAlignment = StringAlignment.Center,
                })
                    g.DrawString(txt, f, b, new RectangleF(0, 0, S, S), sf);
            }
            return bmp.GetHicon();
        }
    }

    // ---------- 消息处理 ----------
    static IntPtr WndProc(IntPtr hWnd, int msg, IntPtr wParam, IntPtr lParam)
    {
        // explorer 重启 → 托盘区被重建 → 我们的图标没了，重新登记。
        // 这条要放在 switch 之前判断：它的消息号是运行时注册的，
        // 不是编译期常量，不能写成 case。
        if (WM_TASKBARCREATED != 0 && (uint)msg == WM_TASKBARCREATED)
        {
            Log("收到 TaskbarCreated，重新登记托盘图标");
            ReAddIcon();
            return IntPtr.Zero;
        }

        switch (msg)
        {
            case WM_TRAYICON:
            {
                int evt = (int)(lParam.ToInt64() & 0xFFFF);

                // 左键：刷新电量，并现读一次 DPI（用户多半就是想看那个数）
                if (evt == WM_LBUTTONUP || evt == WM_LBUTTONDBLCLK)
                {
                    Refresh();      // 立即刷新电量
                    ReadDpiNow("左键");
                    return IntPtr.Zero;
                }

                // 右键：弹菜单（菜单里也有"刷新"，并会带出 DPI）
                if (evt == WM_RBUTTONUP || evt == WM_CONTEXTMENU)
                {
                    ShowMenu();
                    return IntPtr.Zero;
                }
                return IntPtr.Zero;
            }

            case WM_CONTEXTMENU:
                // v4 下右键菜单请求可能直接发给窗口而不是走 uCallbackMessage
                ShowMenu();
                return IntPtr.Zero;

            case WM_TIMER:
                if ((int)wParam.ToInt64() == TIMER_ID) Refresh();
                else if ((int)wParam.ToInt64() == TIMER_HOVER) HoverTick();
                return IntPtr.Zero;

            case WM_DPI_CHANGED:
            {
                // DPI 读完了，在主线程安全地改 _nid
                _nid.uFlags = NIF_TIP;
                _nid.szTip = TipText(_last, _lastDpi);
                Shell_NotifyIcon(NIM_MODIFY, ref _nid);
                return IntPtr.Zero;
            }

            case WM_COMMAND:
            {
                int id = (int)wParam.ToInt64() & 0xFFFF;
                if (id == ID_REFRESH) { Refresh(); return IntPtr.Zero; }
                if (id == ID_AUTOSTART) { ToggleAutostart(); return IntPtr.Zero; }
                if (id == ID_EXIT) { PostQuitMessage(0); return IntPtr.Zero; }
                return IntPtr.Zero;
            }

            case WM_CLOSE:
                PostQuitMessage(0);
                return IntPtr.Zero;

            case WM_DESTROY:
                PostQuitMessage(0);
                return IntPtr.Zero;
        }
        return DefWindowProc(hWnd, msg, wParam, lParam);
    }

    static void ShowMenu()
    {
        // 用户主动打开了菜单（他多半想看当前状态），顺便把 DPI 刷新成当前值。
        ReadDpiNow("菜单");

        IntPtr menu = CreatePopupMenu();
        try
        {
            AppendMenu(menu, MF_STRING, ID_REFRESH, "立即刷新");
            AppendMenu(menu, MF_SEPARATOR, 0, null);
            // 自启项带勾选状态
            AppendMenu(menu, AutostartEnabled() ? (MF_STRING | MF_CHECKED) : MF_STRING,
                ID_AUTOSTART, "开机自动启动");
            AppendMenu(menu, MF_SEPARATOR, 0, null);
            AppendMenu(menu, MF_STRING, ID_EXIT, "退出");
            POINT pt;
            GetCursorPos(out pt);
            // 注意：托盘菜单必须先 SetForegroundWindow，否则点击别处菜单不会消失
            SetForegroundWindow(_hwnd);
            int cmd = TrackPopupMenuEx(menu, TPM_RETURNCMD, pt.X, pt.Y, _hwnd, IntPtr.Zero);
            if (cmd == ID_REFRESH) Refresh();
            else if (cmd == ID_AUTOSTART) ToggleAutostart();
            else if (cmd == ID_EXIT) { PostQuitMessage(0); }
        }
        finally { DestroyMenu(menu); }
    }

    // ---------- 开机自启（HKCU\Software\Microsoft\Windows\CurrentVersion\Run）----------
    // 用注册表而不是启动文件夹快捷方式：不用建 .lnk、路径带空格也不用管引号，
    // 而且 HKCU 不需要管理员权限。
    const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    const string RunValue = "RogBattery";

    static string ExePath()
    {
        try { return System.Reflection.Assembly.GetEntryAssembly().Location; }
        catch { return null; }
    }

    static bool AutostartEnabled()
    {
        try
        {
            using (var k = Microsoft.Win32.Registry.CurrentUser.OpenSubKey(RunKey))
            {
                if (k == null) return false;
                var v = k.GetValue(RunValue) as string;
                if (string.IsNullOrEmpty(v)) return false;
                // 比对路径，避免换了目录后旧条目残留还显示"已启用"
                string exe = ExePath();
                if (exe == null) return true;
                return v.Trim('"').Equals(exe, StringComparison.OrdinalIgnoreCase);
            }
        }
        catch { return false; }
    }

    static void ToggleAutostart()
    {
        try
        {
            using (var k = Microsoft.Win32.Registry.CurrentUser.CreateSubKey(RunKey))
            {
                if (k == null) return;
                if (AutostartEnabled())
                {
                    k.DeleteValue(RunValue, false);
                    Log("已关闭开机自启");
                    Toast("已关闭开机自启");
                }
                else
                {
                    string exe = ExePath();
                    if (string.IsNullOrEmpty(exe)) return;
                    k.SetValue(RunValue, "\"" + exe + "\"");
                    Log("已开启开机自启: " + exe);
                    Toast("已设为开机自动启动");
                }
            }
        }
        catch (Exception e)
        {
            Log("自启设置失败: " + e.Message);
            Toast("设置失败：" + e.Message);
        }
    }

    /// <summary>气泡提示。Win11 上 NIF_INFO 可能不显示，所以同时更新 tooltip 兜底。</summary>
    static void Toast(string text)
    {
        var b = _nid;
        b.uFlags = NIF_INFO;
        b.szInfoTitle = "ROG 电量";
        b.szInfo = text;
        b.dwInfoFlags = 0x1;   // NIIF_INFO
        b.uTimeoutOrVersion = 3000;
        Shell_NotifyIcon(NIM_MODIFY, ref b);
    }
}
