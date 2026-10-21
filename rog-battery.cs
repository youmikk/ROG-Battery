// rog-battery.cs —— 独立读取 ROG 龙鳞 Ace Mini（经 ROG OMNI RECEIVER 0B05:1ACE）电量
//
// 零依赖、零安装：直接用 Windows 自带的 .NET Framework 编译器编译
//   C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe
//
// 协议来自 GearLink 网页版 JS（WebHID）逆向，已在真机验证。
//
// 两种连接方式，命令协议完全一致，只是"命令通道"在哪里不同：
//
//   有线（鼠标插线，本体直接枚举）
//       本体 PID 0x1B63 的 0xFF01 通道就是命令通道，不用扫描接收器
//       写包 65 字节（In/Out 都是 65），report ID 用 3
//       响应首字节是 00（不是 report ID），cmd 在第 2 字节
//
//   2.4G（无线接收器 0B05:1ACE）
//       接收器控制通道 0xFF02 / report ID 1 → 扫描配对设备 cmd=160 sub=0 idx=0
//           响应: 01 | a0 00 <countLo> <countHi> | PID(LE16) reportId notifyReportId ...
//       鼠标控制通道 0xFF01 / report ID = 扫描得到的 reportId
//           写包 64 字节，响应首字节就是 report ID
//
//   两种模式下响应帧都满足：cmd、sub、index(LE16) 连在一起，
//   所以判断应答时不要依赖 report ID，而是找 cmd/sub 对齐的位置。
//
//   电量 cmd=18 sub=7 idx=0
//       响应: <4 字节头> | 电量% 睡眠 低电量阈值 电压(LE16) 充电 电池类型 充满效果
//   DPI  cmd=18 sub=4 idx=2 / idx=3，当前档位 idx=0 的数据段第 7 字节
//
// 用法:
//   rog-battery.exe            读一次
//   rog-battery.exe --watch    持续刷新（默认 30 秒）
//   rog-battery.exe --watch 10 每 10 秒刷新

using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;

static class RogBattery
{
    const ushort VID = 0x0B05;
    const ushort PID_DONGLE = 0x1ACE;      // ROG OMNI RECEIVER（2.4G 接收器，本体无型号）
    const ushort PID_MOUSE = 0x1B65;       // 龙鳞 Ace Mini（接收器扫描响应里报的 PID）
    const ushort PID_WIRED = 0x1B63;       // 龙鳞 Ace Mini 插线直连时本体自己的 PID
    const ushort P_DONGLE_CTRL = 0xFF02;   // 接收器命令通道
    const ushort P_MOUSE_CTRL = 0xFF01;    // 鼠标命令通道（有线/无线都是这个 UsagePage）

    const uint GENERIC_READ = 0x80000000;
    const uint GENERIC_WRITE = 0x40000000;
    const uint FILE_SHARE_READ = 1;
    const uint FILE_SHARE_WRITE = 2;
    const uint OPEN_EXISTING = 3;
    const uint FILE_FLAG_OVERLAPPED = 0x40000000;
    const uint DIGCF_PRESENT = 0x02;
    const uint DIGCF_DEVICEINTERFACE = 0x10;
    const uint WAIT_OBJECT_0 = 0;
    const uint WAIT_TIMEOUT = 258;
    const int ERROR_IO_PENDING = 997;

    static readonly Guid HidGuid =
        new Guid("4D1E55B2-F16F-11CF-88CB-001111000030");

    // ---------------- Win32 ----------------
    [StructLayout(LayoutKind.Sequential)]
    struct SP_DEVICE_INTERFACE_DATA
    {
        public int cbSize;
        public Guid InterfaceClassGuid;
        public int Flags;
        public IntPtr Reserved;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct SP_DEVICE_INTERFACE_DETAIL_DATA
    {
        public int cbSize;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 512)]
        public string DevicePath;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct HIDD_ATTRIBUTES
    {
        public int Size;
        public ushort VendorID;
        public ushort ProductID;
        public ushort VersionNumber;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct HIDP_CAPS
    {
        public ushort Usage;
        public ushort UsagePage;
        public ushort InputReportByteLength;
        public ushort OutputReportByteLength;
        public ushort FeatureReportByteLength;
        [MarshalAs(UnmanagedType.ByValArray, SizeConst = 17)]
        public ushort[] Reserved;
        public ushort NumberLinkCollectionNodes;
        public ushort NumberInputButtonCaps;
        public ushort NumberInputValueCaps;
        public ushort NumberInputDataIndices;
        public ushort NumberOutputButtonCaps;
        public ushort NumberOutputValueCaps;
        public ushort NumberOutputDataIndices;
        public ushort NumberFeatureButtonCaps;
        public ushort NumberFeatureValueCaps;
        public ushort NumberFeatureDataIndices;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct OVERLAPPED
    {
        public IntPtr Internal;
        public IntPtr InternalHigh;
        public int Offset;
        public int OffsetHigh;
        public IntPtr hEvent;
    }

    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr SetupDiGetClassDevs(ref Guid ClassGuid, IntPtr Enumerator,
        IntPtr hwndParent, uint Flags);

    [DllImport("setupapi.dll", SetLastError = true)]
    static extern bool SetupDiEnumDeviceInterfaces(IntPtr hDevInfo, IntPtr devInfo,
        ref Guid interfaceClassGuid, int memberIndex, ref SP_DEVICE_INTERFACE_DATA deviceInterfaceData);

    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool SetupDiGetDeviceInterfaceDetail(IntPtr hDevInfo,
        ref SP_DEVICE_INTERFACE_DATA deviceInterfaceData,
        ref SP_DEVICE_INTERFACE_DETAIL_DATA deviceInterfaceDetailData,
        int deviceInterfaceDetailDataSize, out int requiredSize, IntPtr deviceInfoData);

    [DllImport("setupapi.dll", SetLastError = true)]
    static extern bool SetupDiDestroyDeviceInfoList(IntPtr hDevInfo);

    [DllImport("hid.dll", SetLastError = true)]
    static extern bool HidD_GetAttributes(IntPtr h, ref HIDD_ATTRIBUTES attr);

    [DllImport("hid.dll", SetLastError = true)]
    static extern bool HidD_GetPreparsedData(IntPtr h, out IntPtr pp);

    [DllImport("hid.dll", SetLastError = true)]
    static extern bool HidD_FreePreparsedData(IntPtr pp);

    [DllImport("hid.dll", SetLastError = true)]
    static extern int HidP_GetCaps(IntPtr pp, ref HIDP_CAPS caps);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateFileW(string name, uint access, uint share,
        IntPtr sec, uint disp, uint flags, IntPtr templ);

    // 读缓冲区用 IntPtr（非托管），不能用 byte[]：overlapped I/O 在 ReadFile
    // 返回后才由驱动写入，托管数组那时可能已被 GC 移动，堆会被写坏。
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool ReadFile(IntPtr h, IntPtr buf, int toRead, out int read, ref OVERLAPPED ov);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool WriteFile(IntPtr h, byte[] buf, int toWrite, out int written, IntPtr ov);

    [DllImport("kernel32.dll")]
    static extern IntPtr LocalAlloc(uint flags, IntPtr bytes);

    [DllImport("kernel32.dll")]
    static extern IntPtr LocalFree(IntPtr p);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetOverlappedResult(IntPtr h, ref OVERLAPPED ov, out int transferred, bool wait);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CancelIoEx(IntPtr h, ref OVERLAPPED ov);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr CreateEventW(IntPtr sec, bool manual, bool init, IntPtr name);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern uint WaitForSingleObject(IntPtr h, uint ms);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CloseHandle(IntPtr h);

    // winexe 没有自带控制台。分三种情况：
    //   1) 被管道/重定向捕获 —— STD_OUTPUT_HANDLE 本身就是有效的，直接用它
    //   2) 从 cmd/PowerShell 窗口启动 —— 挂到父控制台，往 CONOUT$ 写
    //   3) 双击启动 —— 父进程没有控制台，什么都不做（也就不会弹黑框）
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool AttachConsole(int processId);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr GetStdHandle(int nStdHandle);

    const int STD_OUTPUT_HANDLE = -11;
    const int STD_ERROR_HANDLE = -12;

    static bool _consoleReady;

    static void EnsureConsole()
    {
        if (_consoleReady) return;
        _consoleReady = true;

        // 情况 1：句柄已经有效（重定向到文件/管道时由父进程提供）
        if (TryBind(GetStdHandle(STD_OUTPUT_HANDLE), GetStdHandle(STD_ERROR_HANDLE)))
            return;

        // 情况 2：挂到父控制台，然后重新打开 CONOUT$
        if (!AttachConsole(-1)) return;      // -1 = ATTACH_PARENT_PROCESS

        IntPtr o = IntPtr.Zero, e = IntPtr.Zero;
        try
        {
            var fs = new System.IO.FileStream("CONOUT$", System.IO.FileMode.Open,
                System.IO.FileAccess.Write, System.IO.FileShare.Write);
            var w = new System.IO.StreamWriter(fs);
            w.AutoFlush = true;
            Console.SetOut(w);
        }
        catch { }
        try
        {
            var fs = new System.IO.FileStream("CONOUT$", System.IO.FileMode.Open,
                System.IO.FileAccess.Write, System.IO.FileShare.Write);
            var w = new System.IO.StreamWriter(fs);
            w.AutoFlush = true;
            Console.SetError(w);
        }
        catch { }
    }

    static bool TryBind(IntPtr outH, IntPtr errH)
    {
        bool ok = false;
        try
        {
            if (outH != IntPtr.Zero && outH != new IntPtr(-1))
            {
                var sfh = new Microsoft.Win32.SafeHandles.SafeFileHandle(outH, false);
                var fs = new System.IO.FileStream(sfh, System.IO.FileAccess.Write);
                var w = new System.IO.StreamWriter(fs);
                w.AutoFlush = true;
                Console.SetOut(w);
                ok = true;
            }
        }
        catch { }
        try
        {
            if (errH != IntPtr.Zero && errH != new IntPtr(-1))
            {
                var sfh = new Microsoft.Win32.SafeHandles.SafeFileHandle(errH, false);
                var fs = new System.IO.FileStream(sfh, System.IO.FileAccess.Write);
                var w = new System.IO.StreamWriter(fs);
                w.AutoFlush = true;
                Console.SetError(w);
            }
        }
        catch { }
        return ok;
    }

    static readonly IntPtr INVALID = new IntPtr(-1);

    // ---------------- HID 设备 ----------------
    class HidIface
    {
        public string Path;
        public ushort ProductId;
        public ushort UsagePage;
        public ushort Usage;
        public ushort InLen;
        public ushort OutLen;

        // 命令通道 = vendor 集合(0xFF01/0xFF02) + Usage 1 + 报告长度 >= 64。
        // 只看 UsagePage 会误抓到同一 PID 下那些 In=2/12/21 的输入集合。
        public bool IsCommandChannel
        {
            get
            {
                return (UsagePage == P_MOUSE_CTRL || UsagePage == P_DONGLE_CTRL)
                    && Usage == 1 && InLen >= 64;
            }
        }
    }

    static List<HidIface> Enumerate(ushort wantPid)
    {
        var list = new List<HidIface>();
        Guid g = HidGuid;
        IntPtr devInfo = SetupDiGetClassDevs(ref g, IntPtr.Zero, IntPtr.Zero,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
        if (devInfo == INVALID) return list;

        try
        {
            var did = new SP_DEVICE_INTERFACE_DATA();
            did.cbSize = Marshal.SizeOf(typeof(SP_DEVICE_INTERFACE_DATA));

            for (int i = 0; ; i++)
            {
                if (!SetupDiEnumDeviceInterfaces(devInfo, IntPtr.Zero, ref g, i, ref did))
                    break;

                var detail = new SP_DEVICE_INTERFACE_DETAIL_DATA();
                detail.cbSize = IntPtr.Size == 8 ? 8 : 6;
                int need;
                if (!SetupDiGetDeviceInterfaceDetail(devInfo, ref did, ref detail,
                        Marshal.SizeOf(typeof(SP_DEVICE_INTERFACE_DETAIL_DATA)), out need, IntPtr.Zero))
                    continue;

                IntPtr h = CreateFileW(detail.DevicePath, 0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE, IntPtr.Zero, OPEN_EXISTING, 0, IntPtr.Zero);
                if (h == INVALID) continue;

                try
                {
                    var attr = new HIDD_ATTRIBUTES();
                    attr.Size = Marshal.SizeOf(typeof(HIDD_ATTRIBUTES));
                    if (!HidD_GetAttributes(h, ref attr)) continue;
                    if (attr.VendorID != VID) continue;
                    if (wantPid != 0 && attr.ProductID != wantPid) continue;

                    IntPtr pp;
                    if (!HidD_GetPreparsedData(h, out pp)) continue;
                    try
                    {
                        var caps = new HIDP_CAPS();
                        caps.Reserved = new ushort[17];
                        if (HidP_GetCaps(pp, ref caps) != 0x00110000) continue;
                        list.Add(new HidIface
                        {
                            Path = detail.DevicePath,
                            ProductId = attr.ProductID,
                            UsagePage = caps.UsagePage,
                            Usage = caps.Usage,
                            InLen = caps.InputReportByteLength,
                            OutLen = caps.OutputReportByteLength,
                        });
                    }
                    finally { HidD_FreePreparsedData(pp); }
                }
                finally { CloseHandle(h); }
            }
        }
        finally { SetupDiDestroyDeviceInfoList(devInfo); }

        return list;
    }

    // ---------------- 打开的句柄 ----------------
    // 关键：overlapped 读的缓冲区必须是非托管的。
    // ReadFile 传托管 byte[] 时 CLR 只在调用期间钉住数组，而 FILE_FLAG_OVERLAPPED
    // 的 I/O 在函数返回后才由驱动写内存 —— 超时一多数组被 GC 搬走，堆就被写坏，
    // 表现为进程无声无息地退出（无线模式命令少，很久才踩一次；有线模式一扫就炸）。
    class Dev : IDisposable
    {
        const int EBUF = 4096;

        IntPtr h, rd, ev, rbuf;
        bool pending;
        OVERLAPPED pov;
        readonly string path;
        public ushort InLen;
        public ushort OutLen;
        public bool WriteOk;

        public Dev(string path, ushort inLen, ushort outLen)
        {
            this.path = path;
            InLen = inLen;
            OutLen = outLen;
            // 写句柄必须同步打开：WriteFile 传 NULL 的 lpOverlapped 时
            // 不允许用 FILE_FLAG_OVERLAPPED 打开的句柄，否则恒报 error 87。
            h = CreateFileW(path, GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE, IntPtr.Zero, OPEN_EXISTING, 0, IntPtr.Zero);
            if (h != INVALID) { WriteOk = true; }
            else
            {
                h = CreateFileW(path, 0, FILE_SHARE_READ | FILE_SHARE_WRITE,
                    IntPtr.Zero, OPEN_EXISTING, 0, IntPtr.Zero);
                WriteOk = false;
            }
            if (h == INVALID) throw new Exception("CreateFileW 失败 " + Marshal.GetLastWin32Error());
        }

        bool EnsureReader()
        {
            if (rd != IntPtr.Zero) return true;
            rd = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE,
                IntPtr.Zero, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, IntPtr.Zero);
            if (rd == INVALID) { rd = IntPtr.Zero; return false; }
            // LMEM_FIXED：地址固定，GC 不会移动它
            rbuf = LocalAlloc(0x0040, new IntPtr(EBUF));
            // 自动复位事件（manual=false）：ReadFile 完成时系统自动置位，
            // 我们等到后直接消费，下一次 ReadFile 前无需手动 ResetEvent。
            ev = CreateEventW(IntPtr.Zero, false, false, IntPtr.Zero);
            return ev != IntPtr.Zero && rbuf != IntPtr.Zero;
        }

        public void Write(byte[] data)
        {
            if (!WriteOk) throw new Exception("句柄无写权限");
            int written;
            if (!WriteFile(h, data, data.Length, out written, IntPtr.Zero))
                throw new Exception("WriteFile 失败 " + Marshal.GetLastWin32Error());
        }

        // 撤回上一笔还没落地的读，并等它真正结束。
        // 只 CancelIoEx 就走人的话，驱动稍后仍会往缓冲区写，那时缓冲区可能已释放。
        void Settle()
        {
            if (!pending) return;
            CancelIoEx(rd, ref pov);
            int done;
            GetOverlappedResult(rd, ref pov, out done, true);
            pending = false;
        }

        public byte[] Read(int timeoutMs)
        {
            if (InLen == 0 || !EnsureReader()) return null;
            Settle();
            pov = new OVERLAPPED();
            pov.hEvent = ev;
            int got;
            if (ReadFile(rd, rbuf, EBUF, out got, ref pov))
            {
                if (got <= 0) return null;
                var q = new byte[got];
                Marshal.Copy(rbuf, q, 0, got);
                return q;
            }
            if (Marshal.GetLastWin32Error() != ERROR_IO_PENDING) return null;
            pending = true;

            uint w = WaitForSingleObject(ev, (uint)timeoutMs);
            if (w == WAIT_TIMEOUT) { Settle(); return null; }
            if (!GetOverlappedResult(rd, ref pov, out got, false)) { pending = false; return null; }
            pending = false;
            if (got <= 0) return null;
            var res = new byte[got];
            Marshal.Copy(rbuf, res, 0, got);
            return res;
        }

        public void Drain()
        {
            for (int i = 0; i < 32; i++)
                if (Read(8) == null) return;
        }

        public void Dispose()
        {
            Settle();
            if (rd != IntPtr.Zero) { CloseHandle(rd); rd = IntPtr.Zero; }
            if (ev != IntPtr.Zero) { CloseHandle(ev); ev = IntPtr.Zero; }
            if (rbuf != IntPtr.Zero) { LocalFree(rbuf); rbuf = IntPtr.Zero; }
            if (h != IntPtr.Zero) { CloseHandle(h); h = IntPtr.Zero; }
        }
    }

    // ---------------- 连接方式 ----------------
    public enum LinkMode { None = 0, Wired = 1, Wireless24G = 2 }

    public static string ModeText(LinkMode m)
    {
        if (m == LinkMode.Wired) return "有线";
        if (m == LinkMode.Wireless24G) return "2.4G 无线";
        return "未连接";
    }

    class Link
    {
        public LinkMode Mode;
        public HidIface Ctrl;      // 鼠标命令通道
        public byte Rid;           // 写帧首字节
    }

    // ---------- 连接缓存 ----------
    // OpenLink 一次要枚举全部 HID 设备、开句柄、再向接收器发 cmd=160 扫描命令，
    // 实测 60~100 ms。而托盘每次刷新都要知道"该往哪个句柄写"，
    // DPI 以前还每秒问一次，等于每秒把设备重新认识一遍。
    //
    // 所以这里缓存上次成功的 Link：设备没变就直接复用，只有读失败才作废重找。
    // 换接法（插线/拔线）时旧通道的命令会失败 → 自动作废 → 下次重新枚举，
    // 所以不需要额外轮询探测。
    static Link _cachedLink;

    /// <summary>让缓存失效。读失败时调用。</summary>
    public static void InvalidateLinkCache() { _cachedLink = null; }

    /// <summary>带缓存的 OpenLink：命中缓存时不枚举、不发扫描命令。</summary>
    static Link OpenLinkCached(out string err)
    {
        err = null;
        var c = _cachedLink;
        if (c != null) return c;

        var lk = OpenLink(out err);
        if (lk != null) _cachedLink = lk;
        return lk;
    }

    // 找到当前的鼠标命令通道。
    // 有线时鼠标本体自己暴露 0xFF01；插线后接收器那条通道就不再转发命令了
    // （实测 cmd=160 扫描完全无响应），所以必须先看本体在不在，不能只看接收器。
    // 注意：调用方一般应该用 OpenLinkCached，只有缓存失效/需要重新探测时才直接调这个。
    static Link OpenLink(out string err)
    {
        err = null;
        var all = Enumerate(0);

        HidIface wired = null;
        var cands = new System.Collections.Generic.List<HidIface>();
        foreach (var x in all)
        {
            if (x.UsagePage != P_MOUSE_CTRL || !x.IsCommandChannel) continue;
            if (x.ProductId == PID_DONGLE) continue;      // 接收器侧的不算本体
            cands.Add(x);
        }
        if (cands.Count == 1)
        {
            // 只有一台候选，不用试探——省掉每秒一次的额外写操作
            wired = cands[0];
        }
        else if (cands.Count > 1)
        {
            // 别的 ROG 设备（键盘等）也可能有 0xFF01 命令通道，
            // 所以不能见到就用——发一条电量查询，能应答的才真是鼠标。
            foreach (var x in cands)
                if (AnswersAsMouse(x)) { wired = x; break; }
        }
        if (wired != null)
            return new Link { Mode = LinkMode.Wired, Ctrl = wired, Rid = 3 };

        HidIface dc = null, mc = null;
        foreach (var x in all)
        {
            if (x.ProductId != PID_DONGLE || !x.IsCommandChannel) continue;
            if (dc == null && x.UsagePage == P_DONGLE_CTRL) dc = x;
            if (mc == null && x.UsagePage == P_MOUSE_CTRL) mc = x;
        }
        if (dc == null) { err = "没找到接收器命令通道 (0xFF02)"; return null; }
        if (mc == null) { err = "没找到鼠标命令通道 (0xFF01)"; return null; }

        var d = new Dev(dc.Path, dc.InLen, dc.OutLen);
        try
        {
            if (!d.WriteOk) { err = "接收器句柄无写权限"; return null; }

            byte[] scan = null;
            for (int i = 0; i < 3 && scan == null; i++)
                scan = Ask(d, 1, 160, 0, null, 2000);
            if (scan == null) { err = "接收器扫描配对设备无响应"; return null; }

            int count = scan[3] << 8 | scan[2];
            if (count < 1) { err = "接收器没有配对的设备"; return null; }

            // 每台配对设备 4 字节：PID(LE16) reportId notifyReportId
            // 只接一台鼠标时直接用第 0 台；接了多台就优先挑 0x1B65（龙鳞 Ace Mini）
            int pick = 0;
            for (int i = 0; i < count; i++)
            {
                int c = 4 + i * 4;
                if (scan.Length < c + 4) break;
                if ((scan[c + 1] << 8 | scan[c]) == PID_MOUSE) { pick = i; break; }
            }
            int b = 4 + pick * 4;
            if (scan.Length < b + 4) { err = "扫描响应长度不足"; return null; }

            byte rid = scan[b + 2];
            if (rid == 0) rid = 3;
            return new Link { Mode = LinkMode.Wireless24G, Ctrl = mc, Rid = rid };
        }
        finally { d.Dispose(); }
    }

    // 试探某个命令通道是不是真鼠标：发电量查询，看有没有正常长度的应答。
    // 只用于有线判定，所以 reportId 固定 3（实测本体就是 3）。
    static bool AnswersAsMouse(HidIface x)
    {
        var d = new Dev(x.Path, x.InLen, x.OutLen);
        try
        {
            if (!d.WriteOk) return false;
            var p = Ask(d, 3, 18, 7, 0, 600);
            return p != null && p.Length >= 12;
        }
        catch { return false; }
        finally { d.Dispose(); }
    }

    // ---------------- 协议 ----------------
    // 发一帧 <rid> cmd sub idxLo idxHi 到命令通道，等与之对应的应答。
    // 应答帧两种布局都可能有，所以不靠 report ID 定位，而是找 cmd/sub 对齐处：
    //   00 12 07 00 00 ...   ← 有线，首字节是状态
    //   03 12 07 00 00 ...   ← 2.4G，首字节是 report ID
    static byte[] Ask(Dev dev, byte rid, byte cmd, byte sub, ushort? index, int timeoutMs)
    {
        dev.Drain();

        int total = dev.OutLen > 0 ? dev.OutLen : 64;
        var pkt = new byte[total];
        pkt[0] = rid;
        if (total >= 5)
        {
            pkt[1] = cmd;
            pkt[2] = sub;
            ushort ix = index ?? 0;
            pkt[3] = (byte)(ix & 0xFF);
            pkt[4] = (byte)(ix >> 8);
        }
        if (!dev.WriteOk) return null;
        dev.Write(pkt);

        var sw = Stopwatch.StartNew();
        while (sw.ElapsedMilliseconds < timeoutMs)
        {
            var r = dev.Read(250);
            if (r == null || r.Length < 3) continue;

            for (int off = 1; off >= 0; off--)
            {
                if (r.Length - off < 2) continue;
                if (r[off] != cmd || r[off + 1] != sub) continue;

                if (index.HasValue && r.Length - off >= 4)
                {
                    int got = r[off + 2] | r[off + 3] << 8;
                    // index 只表示"查哪一项"，有些命令回帧里恒为 0，不能因此丢帧
                    if (got != 0 && got != index.Value) continue;
                }

                var outb = new byte[r.Length - off];
                Array.Copy(r, off, outb, 0, outb.Length);
                return outb;
            }
        }
        return null;
    }

    class PowerInfo
    {
        public int Level, Voltage, LowPower, Sleep, BatteryType, FullEffect;
        public bool Charging;
    }

    // 充电状态三态。
    // 依据 GearLink 自己留下的历史日志（155 条采样）：
    //   charging_status=1 只出现在 98%、99% 两次（21:20 / 21:22），
    //   一到 100% 就变 0（21:23 起一直是 0）。
    // 也就是说设备只在"真的在往里充"时报 1，充满后回 0。
    // 所以 100% 时必须靠"有线在供电"来判"已充满"，否则用户永远看不到充电相关的字。
    public enum ChargeState
    {
        Discharging = 0,   // 在用电池
        Charging = 1,      // 正在充
        Full = 2,          // 已充满（100% 且插着线）
    }

    /// <summary>托盘用的公开快照：null 表示读不到。</summary>
    public class State
    {
        public int Level, Voltage;
        public bool Charging;      // 设备报告的 charging_status
        public ChargeState Charge; // 归并后的三态
        public LinkMode Mode;
    }

    /// <summary>把一次 PowerInfo 读数包装成 State（含三态归并）。
    /// 只能用 private：PowerInfo 是私有嵌套类，公开出去会触发 CS0051。</summary>
    static State FromDevice(PowerInfo p, LinkMode mode)
    {
        if (p == null) return null;
        return new State
        {
            Level = p.Level,
            Voltage = p.Voltage,
            Charging = p.Charging,
            Charge = DecideCharge(p, mode),
            Mode = mode,
        };
    }

    /// <summary>读一次电量；失败返回 null（不抛异常）。</summary>
    public static State Snapshot()
    {
        string err;
        LinkMode mode;
        var p = ReadPower(out err, out mode);
        return FromDevice(p, mode);
    }

    /// <summary>把"设备报的 charging_status"和"接线方式"归并成三态。</summary>
    static ChargeState DecideCharge(PowerInfo p, LinkMode mode)
    {
        // 设备自己说在充，最可信
        if (p.Charging) return ChargeState.Charging;

        // 设备说没在充，但有线接着：要么真充满了，要么只是插着线在用。
        // 满电判据用 100%——日志里 98/99 时仍报 charging=1，说明 100% 才是充满点。
        if (mode == LinkMode.Wired && p.Level >= 100) return ChargeState.Full;

        return ChargeState.Discharging;
    }

    /// <summary>当前连接方式（有线 / 2.4G / 未连接）。</summary>
    public static LinkMode CurrentMode()
    {
        string err;
        var lk = OpenLinkCached(out err);
        return lk == null ? LinkMode.None : lk.Mode;
    }

    static PowerInfo ReadPower(out string err, out LinkMode mode)
    {
        err = null;
        mode = LinkMode.None;

        var lk = OpenLinkCached(out err);
        if (lk == null) return null;
        mode = lk.Mode;

        var dev = new Dev(lk.Ctrl.Path, lk.Ctrl.InLen, lk.Ctrl.OutLen);
        try
        {
            if (!dev.WriteOk)
            {
                // 句柄开不出写权限，多半是设备已经拔了/换了接法：缓存作废，下次重新枚举
                InvalidateLinkCache();
                err = "鼠标命令通道无写权限";
                return null;
            }

            byte[] p = null;
            for (int i = 0; i < 3 && p == null; i++)
                p = Ask(dev, lk.Rid, 18, 7, 0, 1500);
            if (p == null)
            {
                // 连发 3 次都无应答：可能是鼠标休眠，也可能是换了接法导致这条通道失效。
                // 作废缓存，下一轮重新枚举——比一直往死通道写更可靠。
                InvalidateLinkCache();
                err = "鼠标电量命令无响应";
                return null;
            }
            if (p.Length < 12) { err = "电量响应过短"; return null; }

            // 电量必须是 0..100。以前不校验，坏帧会让托盘直接画出 "255%"，
            // 而且 MakeIcon 只看 <30 才变红，255 反而显示成正常颜色。
            if (p[4] > 100) { err = "电量越界 " + p[4]; return null; }

            return new PowerInfo
            {
                Level = p[4],
                Sleep = p[5],
                LowPower = p[6],
                Voltage = p[7] | p[8] << 8,
                Charging = p[9] == 1,
                BatteryType = p[10],
                FullEffect = p[11],
            };
        }
        catch (Exception e) { err = e.Message; return null; }
        finally { dev.Dispose(); }
    }

    // ---------------- DPI ----------------
    // 全部靠向鼠标查询得到，不依赖 GearLink（已卸载也能用）：
    //
    //   12 00 <idx=0>  → 四档配置的摘要，返回 payload 第 7 字节 = 当前档位（1 基）
    //                    e.g. 07 00 07 00 05 07 00 02 ff ff 15 00 01 ...
    //                                                  ^^ = 2 表示第 2 档
    //   12 04 <idx=2>  → 四档 DPI 值表，每档 2 字节小端，真实 DPI = (值+1)*50
    //                    e.g. 07 00 07 00 0f 00 0f 00 1f 00 1f 00 3f 00 3f 00
    //                         → 400 / 400 / 800 / 800 / 1600 / 1600 / 3200 / 3200
    //                         （X/Y 成对，通常相同）
    //   12 04 <idx=3>  → 四档 LED 颜色，每档 3 字节 RGB
    //
    // 注意：DPI 没有"鼠标主动推给你"的事件。之前以为的异步事件其实是
    // Armoury Crate 服务在轮询同一个字段，它一卸载就再也没有帧了。
    public class DpiInfo
    {
        public int[] Values = new int[0];   // 四档 DPI 数值
        public string[] Colors = new string[0];
        public int Tier = -1;               // 当前档位（1 基，1..4）；-1 = 未知
        public string Source;               // "device" / "cache" / null

        public int Selected { get { return Tier - 1; } }   // 0 基，便于取值

        public int Current
        {
            get
            {
                int i = Tier - 1;
                if (i >= 0 && i < Values.Length) return Values[i];
                return -1;
            }
        }
    }

    static string CacheDir()
    {
        return System.IO.Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            @"ASUS\Gear Link\MS\HTML");
    }

    /// <summary>读 GearLink 留下的档位副本。仅当设备读不到时作为兜底。</summary>
    static DpiInfo ReadDpiCache()
    {
        var info = new DpiInfo();
        try
        {
            var dir = CacheDir();
            if (!System.IO.Directory.Exists(dir)) return null;
            var files = System.IO.Directory.GetFiles(dir, "*-dpi-*.json",
                System.IO.SearchOption.AllDirectories);
            if (files.Length == 0) return null;
            Array.Sort(files, delegate (string a, string b)
            {
                return System.IO.File.GetLastWriteTimeUtc(b)
                    .CompareTo(System.IO.File.GetLastWriteTimeUtc(a));
            });

            var vals = new System.Collections.Generic.List<int>();
            var cols = new System.Collections.Generic.List<string>();
            int sel = -1;

            foreach (var f in files)
            {
                string json;
                try
                {
                    var b64 = System.IO.File.ReadAllText(f).Trim();
                    json = System.Text.Encoding.UTF8.GetString(Convert.FromBase64String(b64));
                }
                catch { continue; }
                if (json.IndexOf("\"data\"") < 0) continue;

                vals.Clear(); cols.Clear(); sel = -1;
                int p = json.IndexOf("\"data\"");
                while (true)
                {
                    int v = json.IndexOf("\"value\":", p);
                    if (v < 0) break;
                    int vs = v + 8, ve = vs;
                    while (ve < json.Length && (char.IsDigit(json[ve]) || json[ve] == '-')) ve++;
                    int value;
                    if (!int.TryParse(json.Substring(vs, ve - vs), out value)) break;
                    vals.Add(value);

                    int ce = json.IndexOf("\"color\":\"", v);
                    string col = "";
                    if (ce > 0)
                    {
                        ce += 9;
                        int cx = json.IndexOf('"', ce);
                        if (cx > ce) col = json.Substring(ce, cx - ce);
                    }
                    cols.Add(col);

                    int se = json.IndexOf("\"isSelected\":", v);
                    if (se > 0 && se < json.IndexOf('}', v)
                        && json.IndexOf("true", se) == se + 13)
                        sel = vals.Count - 1;

                    p = (se > 0) ? se + 13 : ve;
                }
                if (vals.Count > 0) break;
            }

            if (vals.Count == 0) return null;
            info.Values = vals.ToArray();
            info.Colors = cols.ToArray();
            info.Tier = sel + 1;          // 转成 1 基；sel=-1 时为 0，表示未知
            if (info.Tier == 0) info.Tier = -1;
            info.Source = "cache";
            return info;
        }
        catch { return null; }
    }

    /// <summary>发一帧查询并等应答；失败返回 null。</summary>
    static byte[] Query(Dev m, byte rid, byte cmd, byte sub, ushort index, int timeoutMs)
    {
        return Ask(m, rid, cmd, sub, index, timeoutMs);
    }

    /// <summary>
    /// 从鼠标直接读 DPI：档位 + 四档数值 + 颜色。
    /// 读不到档位数值时退回 GearLink 的缓存副本；再不行返回 null。
    /// </summary>
    public static DpiInfo ReadDpiFromDevice()
    {
        string err;
        var lk = OpenLinkCached(out err);
        if (lk == null) return null;

        var m = new Dev(lk.Ctrl.Path, lk.Ctrl.InLen, lk.Ctrl.OutLen);
        try
        {
            if (!m.WriteOk) { InvalidateLinkCache(); return null; }

            var info = new DpiInfo();

            // 应答帧（Query 已剥掉开头的 reportId/状态字节，所以 cfg[0] 就是 cmd）：
            //   有线 12 00 00 00 | 00 15 00 01 00 05 07 00 <tier> FF FF
            //   2.4G 12 00 00 00 | 07 00 07 00 05 07 00 <tier> FF FF
            // 配置段前缀长度两种接法不同（有线 8、2.4G 7），但尾部
            // <05 07 00> <tier> <FF FF> 完全一致，所以按这个特征找，别硬算下标。
            const int pay = 4;   // 表与颜色帧的 payload 起点，两种接法相同

            // 1) 当前档位：12 00 idx=0
            var cfg = Query(m, lk.Rid, 0x12, 0x00, 0, 700);
            if (cfg != null)
            {
                for (int i = pay; i + 5 < cfg.Length; i++)
                {
                    if (cfg[i] != 0x05 || cfg[i + 1] != 0x07 || cfg[i + 2] != 0x00) continue;
                    if (cfg[i + 4] != 0xFF || cfg[i + 5] != 0xFF) continue;
                    int tier = cfg[i + 3];
                    if (tier >= 1 && tier <= 4) { info.Tier = tier; break; }
                }
            }

            // 2) 四档数值：12 04 idx=2，每档 X/Y 各 2 字节，真实 DPI = (值+1)*50
            var tbl = Query(m, lk.Rid, 0x12, 0x04, 2, 700);
            if (tbl != null && tbl.Length > pay + 15)
            {
                var vals = new System.Collections.Generic.List<int>();
                // 每档占 4 字节（X 低 2 字节 + Y 高 2 字节），X 等于 Y，只取前一半
                for (int g = 0; g < 4; g++)
                {
                    int off = pay + g * 4;
                    int raw = tbl[off] | tbl[off + 1] << 8;
                    vals.Add((raw + 1) * 50);
                }
                if (vals.Count > 0) { info.Values = vals.ToArray(); info.Source = "device"; }
            }

            // 3) 四档颜色：12 04 idx=3，每档 3 字节 RGB
            var col = Query(m, lk.Rid, 0x12, 0x04, 3, 500);
            if (col != null && col.Length > pay + 11)
            {
                var cols = new System.Collections.Generic.List<string>();
                for (int g = 0; g < 4; g++)
                {
                    int off = pay + g * 3;
                    cols.Add("#" + col[off].ToString("x2")
                        + col[off + 1].ToString("x2") + col[off + 2].ToString("x2"));
                }
                if (cols.Count > 0) info.Colors = cols.ToArray();
            }

            if (info.Values.Length > 0 || info.Tier > 0) return info;
            return null;
        }
        catch { return null; }
        finally { m.Dispose(); }
    }

    /// <summary>托盘/命令行统一入口：优先问鼠标，失败才用缓存。</summary>
    public static DpiInfo SnapshotDpi()
    {
        var dev = ReadDpiFromDevice();
        if (dev != null && dev.Values.Length > 0 && dev.Tier > 0) return dev;

        var cache = ReadDpiCache();
        if (cache == null) return dev;

        // 设备只读到一半就补齐
        if (dev != null)
        {
            if (cache.Values.Length > 0 && dev.Values.Length == 0) dev.Values = cache.Values;
            if (cache.Colors.Length > 0 && dev.Colors.Length == 0) dev.Colors = cache.Colors;
            if (dev.Tier <= 0 && cache.Tier > 0) dev.Tier = cache.Tier;
            return dev;
        }
        return cache;
    }

    // 用法: rog-battery.exe --mode
    // 打印当前连接方式和命令通道细节，排查"有线/2.4G 认错"时用。
    static int ShowMode()
    {
        string err = null;
        var all = Enumerate(0);

        Console.WriteLine("0B05 的命令通道：");
        bool any = false;
        foreach (var x in all)
        {
            if (!x.IsCommandChannel) continue;
            any = true;
            Console.WriteLine("  PID=0x" + x.ProductId.ToString("X4")
                + "  UsagePage=0x" + x.UsagePage.ToString("X4")
                + "  In=" + x.InLen + " Out=" + x.OutLen);
        }
        if (!any) Console.WriteLine("  (没有)");

        var lk = OpenLink(out err);
        Console.WriteLine();
        if (lk == null)
        {
            Console.WriteLine("连接方式: 未连接 / 读不到  (" + err + ")");
            return 1;
        }
        Console.WriteLine("连接方式: " + ModeText(lk.Mode));
        Console.WriteLine("命令通道: PID=0x" + lk.Ctrl.ProductId.ToString("X4")
            + "  UsagePage=0x" + lk.Ctrl.UsagePage.ToString("X4")
            + "  In=" + lk.Ctrl.InLen + " Out=" + lk.Ctrl.OutLen
            + "  reportId=" + lk.Rid);
        return 0;
    }

    static string DumpDpi(DpiInfo d)
    {
        if (d == null) return "(未取到)";
        if (d.Values.Length == 0) return "档位表未知，当前索引 " + d.Selected;
        var sb = new System.Text.StringBuilder();
        for (int i = 0; i < d.Values.Length; i++)
        {
            if (i > 0) sb.Append(" / ");
            if (i == d.Selected) sb.Append("[" + d.Values[i] + "]");
            else sb.Append(d.Values[i]);
        }
        return sb.ToString() + "  (来源 " + d.Source + ")";
    }

    // ---------------- 剩余时间估算 ----------------
    // 电量只有整数百分比，鼠标又常常在某个值上停很久（GearLink 日志里 155 条采样
    // 有 151 条是 100%），所以单次读数绝推不出速度 —— 必须攒一段"电量真的变了"
    // 的历史。做法是把每次读数记到 %LOCALAPPDATA%\rog-battery\history.tsv，
    // 需要时对同相位样本做线性回归，得到 %/小时。
    public class Estimate
    {
        public bool Known;
        public string Text = "";             // 可直接显示的整句
        public double PercentPerHour;        // 正=在涨，负=在掉
        public int Samples;
        public double SpanMinutes;
    }

    const int HIST_KEEP_HOURS = 48;           // 保留最近 48 小时（覆盖两晚）
    const int HIST_MAX_LINES = 2000;          // 文件行数上限，防止无限增长
    const int HIST_MIN_GAP_SEC = 60;          // 电量变化时，两条采样至少隔 60 秒
    const int HIST_SAME_GAP_SEC = 600;        // 电量没变时，每 10 分钟记一条心跳
    // 相邻样本间隔超过这个值就认为"中间没在观察"，回归不能用这段跨度。
    // 取 2 小时：正常心跳 10 分钟一条，就算把刷新间隔调到 30 分钟也远不到这里；
    // 而实测碰到的那个坑是 6 小时空档（程序退出/机器睡眠），必须拦下来。
    const int HIST_GAP_BREAK_SEC = 7200;

    /// <summary>
    /// 当前进程是不是跑在 MSIX/Appx 包里。
    /// 为什么要问这一句：装了包之后 exe 会被放在
    /// C:\Program Files\WindowsApps\... 下，那个目录**对普通写操作是只读的**。
    /// 而下面 HistoryPath 原来只判 Directory.Exists 就往那儿写，
    /// 结果是每次都失败、又永远退回不到 AppData（因为路径分支已经"返回"了）。
    /// </summary>
    static bool IsPackaged()
    {
        try
        {
            int len = 0;
            int rc = GetCurrentPackageFullName(ref len, null);
            // 15700 = APPMODEL_ERROR_NO_PACKAGE，即没打包
            return rc != 15700;
        }
        catch { return false; }
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    static extern int GetCurrentPackageFullName(ref int packageFullNameLength, System.Text.StringBuilder packageFullName);

    static string HistoryPath()
    {
        // 打包运行时**必须**先排除 exe 同目录。
        // 装成 MSIX 后 exe 在 C:\Program Files\WindowsApps\<包名>\ 里，
        // 那里对非管理员写入一律拒绝；而原来的判断只看目录存不存在，
        // 于是返回一个写不进去的路径、并且直接 return（不会走到 AppData 退路）。
        // 实测：包内跑起来后 history.tsv 完全写不出来，也不报错，估算功能静默失效。
        bool packaged = IsPackaged();

        // 绿色版：优先放 exe 同目录 —— 但要**真的试写一次**才知道能不能写。
        // 只看 Directory.Exists 是不够的：D:\codex 曾经带 Low 完整性标签、
        // Program Files 下的普通用户目录、只读挂载的盘，都是"目录存在但写不进"。
        // LogPath 一直用「追加一次空串」来探测，这里照抄同一招。
        if (!packaged)
        {
            try
            {
                string exe = System.Reflection.Assembly.GetEntryAssembly().Location;
                if (!string.IsNullOrEmpty(exe))
                {
                    string dir = System.IO.Path.GetDirectoryName(exe);
                    if (!string.IsNullOrEmpty(dir) && System.IO.Directory.Exists(dir))
                    {
                        string same = System.IO.Path.Combine(dir, "history.tsv");
                        System.IO.File.AppendAllText(same, "", System.Text.Encoding.ASCII);
                        return same;    // 探测成功就不用往下走了
                    }
                }
            }
            catch { }
        }

        // 退路：AppData。
        // 打包运行时 SpecialFolder.LocalApplicationData 仍然返回
        // %LOCALAPPDATA%，但写入会被系统重定向到
        // %LOCALAPPDATA%\Packages\<包族名>\LocalCache\Local\ 下 —— 这是 MSIX 的
        // 文件虚拟化，对程序透明：同样路径读回来自动命中重定向后的真实文件。
        try
        {
            string dir = System.IO.Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                "rog-battery");
            System.IO.Directory.CreateDirectory(dir);
            string appData = System.IO.Path.Combine(dir, "history.tsv");
            System.IO.File.AppendAllText(appData, "", System.Text.Encoding.ASCII);
            return appData;
        }
        catch { return null; }
    }

    /// <summary>
    /// 记一条采样。留点智能：电量没变就不必每分钟都写，否则鼠标在 100%
    /// 趴一夜，600 行全被同一个值占满，真正有用的变化点反而被挤出去。
    /// 规则：电量变了立刻写；没变则最多每 HIST_SAME_GAP_SEC 写一条"心跳"，
    /// 既能撑住时间跨度，又不会淹没变化点。
    /// </summary>
    public static void RecordSample(State st)
    {
        if (st == null || st.Level < 0 || st.Level > 100) return;
        string path = HistoryPath();
        if (path == null) return;

        // 进程间互斥：托盘在跑的时候，用户完全可能再开一个 --watch 或者
        // --estimate（后者也会记一条）。两边同时"读全文件→判断→追加"，
        // 轻则写出两行同一秒，重则 PruneHistory 用旧快照覆盖掉对方刚追加的行。
        // 用命名 Mutex（不是 file lock）是因为跨进程、且进程崩溃时内核自动释放。
        bool gotLock = false;
        try
        {
            try
            {
                gotLock = _histMutex.WaitOne(3000);
            }
            catch (AbandonedMutexException)
            {
                // 上一个持有者崩了，互斥量已归我们，继续
                gotLock = true;
            }
            if (!gotLock) return;

            RecordSampleLocked(st, path);
        }
        finally
        {
            if (gotLock) { try { _histMutex.ReleaseMutex(); } catch { } }
        }
    }

    static readonly System.Threading.Mutex _histMutex =
        new System.Threading.Mutex(false, @"Local\RogBatteryHistory");

    static void RecordSampleLocked(State st, string path)
    {
        long now = DateTimeOffset.UtcNow.ToUnixTimeSeconds();
        var rows = ReadHistory();

        if (rows.Count > 0)
        {
            var last = rows[rows.Count - 1];
            long gap = now - (long)last[0];
            bool levelChanged = (int)last[1] != st.Level;
            bool phaseChanged = (last[2] >= 0.5) != st.Charging;
            long need = (levelChanged || phaseChanged) ? HIST_MIN_GAP_SEC : HIST_SAME_GAP_SEC;
            if (gap < need) return;
        }

        // charging 用原始 charging_status，不用归并后的三态：
        // "已充满"那段时间电量不变，混进充电样本会把斜率拉平。
        string line = now + "\t" + st.Level + "\t" + (st.Charging ? "1" : "0")
                    + "\t" + (int)st.Mode + "\n";
        try
        {
            System.IO.File.AppendAllText(path, line, System.Text.Encoding.ASCII);
            PruneHistory(path, rows);
        }
        catch (Exception e) { LastError = "写历史失败: " + e.Message; }
    }

    /// <summary>最近一次写历史/读历史的错误，供 --estimate 之类的诊断输出用。</summary>
    public static string LastError;

    static void PruneHistory(string path, List<double[]> rows)
    {
        try
        {
            long cutoff = DateTimeOffset.UtcNow.ToUnixTimeSeconds() - HIST_KEEP_HOURS * 3600L;
            var keep = new List<string>();
            int skip = 0;
            foreach (var r in rows)
            {
                if ((long)r[0] < cutoff) { skip++; continue; }
                keep.Add(((long)r[0]) + "\t" + (int)r[1] + "\t" + ((int)r[2])
                       + "\t" + (int)r[3]);
            }
            // 没东西可删就别重写文件
            if (skip == 0 && keep.Count < HIST_MAX_LINES) return;
            if (keep.Count > HIST_MAX_LINES)
                keep.RemoveRange(0, keep.Count - HIST_MAX_LINES);
            System.IO.File.WriteAllLines(path, keep.ToArray(), System.Text.Encoding.ASCII);
        }
        catch { }
    }

    /// <summary>读历史，每行 [unix秒, 电量, charging(0/1), mode]。</summary>
    static List<double[]> ReadHistory()
    {
        var list = new List<double[]>();
        string path = HistoryPath();
        if (path == null || !System.IO.File.Exists(path)) return list;
        try
        {
            foreach (string raw in System.IO.File.ReadAllLines(path, System.Text.Encoding.ASCII))
            {
                string s = raw.Trim();
                if (s.Length == 0) continue;
                string[] f = s.Split('\t');
                if (f.Length < 4) continue;
                long t; int lv, ch, md;
                if (!long.TryParse(f[0], out t)) continue;
                if (!int.TryParse(f[1], out lv)) continue;
                if (!int.TryParse(f[2], out ch)) continue;
                if (!int.TryParse(f[3], out md)) continue;
                if (lv < 0 || lv > 100) continue;
                list.Add(new double[] { t, lv, ch, md });
            }
        }
        catch { }
        return list;
    }

    /// <summary>
    /// 预测还要多久充满 / 还能用多久。
    /// 样本不足时 Known=false，Text 说明原因，不要假装有结论。
    /// </summary>
    public static Estimate Predict(State now)
    {
        if (now == null) return new Estimate();
        var rows = ReadHistory();
        long nowSec = DateTimeOffset.UtcNow.ToUnixTimeSeconds();
        return PredictFromRows(rows, now, nowSec);
    }

    /// <summary>
    /// 从历史行推算，不做文件 IO。nowSec 由调用方给，这样自检能灌合成时间轴
    /// （否则"补当前读数"那步会用到真实时间，和合成样本的时序打架）。
    /// </summary>
    static Estimate PredictFromRows(List<double[]> rows, State now, long nowSec)
    {
        if (now == null) return new Estimate();
        if (now.Charge == ChargeState.Full)
        {
            var full = new Estimate();
            full.Text = "已充满";
            return full;
        }
        bool charging = now.Charge == ChargeState.Charging;

        // 第一步：取出"连续同相位"的一段（先不管方向）。
        var phase = FilterPhase(rows, charging);

        // 同相位样本太少 → 说明刚切换过充电/放电，历史还没攒够。
        // 这时不要说"样本不足"糊弄过去，要告诉用户真实原因。
        if (phase.Count < 3)
        {
            var few = new Estimate();
            few.Samples = phase.Count;
            if (rows.Count >= 3)
                few.Text = charging
                    ? "预计充满时间：刚切换，重新观察中"
                    : "预计可用时间：刚切换，重新观察中";
            else
                few.Text = charging ? "预计充满时间：样本不足" : "预计可用时间：样本不足";
            return few;
        }

        // 第二步：当前读数和这段历史对不上（相差超过 1%）→ 读数跳变，历史作废。
        double lastLv = phase[phase.Count - 1][1];
        bool conflict = charging ? (now.Level < lastLv - 1) : (now.Level > lastLv + 1);
        if (conflict)
        {
            var bad = new Estimate();
            bad.Samples = phase.Count;
            bad.Text = charging
                ? "预计充满时间：电量刚跳变，重新观察中"
                : "预计可用时间：电量刚跳变，重新观察中";
            return bad;
        }

        // 第三步：方向过滤。放电段里出现回升（或充电段里出现回落）说明
        // 中途插过线/传感器抖动，只保留最近这段干净的。
        var sel = FilterDirection(phase, charging);
        if (sel.Count < 3)
        {
            var jump = new Estimate();
            jump.Samples = sel.Count;
            jump.Text = charging
                ? "预计充满时间：电量刚跳变，重新观察中"
                : "预计可用时间：电量刚跳变，重新观察中";
            return jump;
        }

        // 第四步：把当前读数并进序列末尾。历史最后一条可能是十几分钟前的，
        // 不并进去等于拿旧数据预测现在。
        var full2 = new List<double[]>(sel);
        if ((long)full2[full2.Count - 1][0] < nowSec)
            full2.Add(new double[] { nowSec, now.Level, charging ? 1 : 0, (int)now.Mode });

        return PredictFrom(full2, now);
    }

    /// <summary>从最新往回取"连续同一相位"的样本（不做方向判断）。</summary>
    static List<double[]> FilterPhase(List<double[]> rows, bool charging)
    {
        var sel = new List<double[]>();
        double prevT = double.NaN;
        for (int i = rows.Count - 1; i >= 0; i--)
        {
            bool isChg = rows[i][2] >= 0.5;
            if (isChg != charging) break;

            // 断档检测：两条样本间隔超过 HIST_GAP_BREAK_SEC 说明中间根本没在观察
            // （程序没运行 / 机器睡眠 / 鼠标长时间不响应）。这种跨度不能当连续
            // 观测用——实测遇到过 6 小时空档，两头差 4%，回归把它当成"6 小时
            // 均匀掉 4%"，于是算出一个好看但完全错的斜率。
            // 碰到断档就截断，只用断档之后这一段。
            if (!double.IsNaN(prevT) && (prevT - rows[i][0]) > HIST_GAP_BREAK_SEC) break;

            sel.Insert(0, rows[i]);
            prevT = rows[i][0];
        }
        return sel;
    }

    /// <summary>
    /// 从最新往回取"方向也一致"的一段：放电样本往回看应单调不降，
    /// 充电样本相反。碰到第一次反向就截断。
    /// </summary>
    static List<double[]> FilterDirection(List<double[]> rows, bool charging)
    {
        var sel = new List<double[]>();
        double prev = double.NaN;
        for (int i = rows.Count - 1; i >= 0; i--)
        {
            double lv = rows[i][1];
            if (!double.IsNaN(prev))
            {
                bool ok = charging ? (lv <= prev) : (lv >= prev);
                if (!ok) break;
            }
            sel.Insert(0, rows[i]);
            prev = lv;
        }
        return sel;
    }

    /// <summary>
    /// 纯函数核心：给定按时间升序的样本 [unix秒, 电量, ...] 和当前状态，算斜率。
    /// 抽出来是为了能用 `--estimate-selftest` 灌合成数据验证，不用真等几小时。
    /// 注意：调用方负责保证 sel 已包含"当前读数"这条（Predict 会补），
    /// 这里不再自己加——自检用的是合成时间轴，混进真实 now 会破坏时序。
    /// </summary>
    static Estimate PredictFrom(List<double[]> sel, State now)
    {
        var e = new Estimate();
        if (now == null) return e;

        if (now.Charge == ChargeState.Full) { e.Text = "已充满"; return e; }
        bool charging = now.Charge == ChargeState.Charging;
        e.Samples = sel.Count;

        if (sel.Count < 3)
        {
            e.Text = charging ? "预计充满时间：样本不足" : "预计可用时间：样本不足";
            return e;
        }

        double x0 = sel[0][0];
        double x1 = sel[sel.Count - 1][0];
        double spanMin = (x1 - x0) / 60.0;
        e.SpanMinutes = spanMin;

        // 用极差判断"到底动过没有"。只比首尾会漏掉"先降后升"这种情形：
        // 那种情况下首尾相等，但数据里明明有变化。
        double lo = double.MaxValue, hi = double.MinValue;
        foreach (var r in sel) { if (r[1] < lo) lo = r[1]; if (r[1] > hi) hi = r[1]; }

        if (hi - lo < 1.0)
        {
            e.Text = charging
                ? "预计充满时间：电量还没变化（需要再多观察一会儿）"
                : "预计可用时间：电量还没变化（需要再多观察一会儿）";
            return e;
        }

        double n = sel.Count, sx = 0, sy = 0, sxy = 0, sxx = 0;
        foreach (var r in sel)
        {
            double xh = (r[0] - x0) / 3600.0;      // 小时
            sx += xh; sy += r[1]; sxy += xh * r[1]; sxx += xh * xh;
        }
        double den = n * sxx - sx * sx;
        if (Math.Abs(den) < 1e-9)
        {
            e.Text = charging ? "预计充满时间：数据跨度太短" : "预计可用时间：数据跨度太短";
            return e;
        }
        double slope = (n * sxy - sx * sy) / den;   // %/小时

        if (Math.Abs(slope) < 0.05)
        {
            e.Text = charging
                ? "预计充满时间：变化太慢，暂时测不出"
                : "预计可用时间：变化太慢，暂时测不出";
            return e;
        }

        // 方向和当前状态相反，说明刚切过充电/放电，样本混了
        if (charging && slope <= 0)
        {
            e.Text = "预计充满时间：刚插上电，还在观察";
            return e;
        }
        if (!charging && slope >= 0)
        {
            e.Text = "预计可用时间：刚拔掉线，还在观察";
            return e;
        }

        double hours = charging ? (100.0 - now.Level) / slope : now.Level / (-slope);
        // 上限从 30 天收紧到 100 小时。原来的阈值形同虚设：鼠标闲置时斜率能小到
        // 0.1 %/h，外推出来就是几百小时，30 天（720h）都拦不住，于是托盘上
        // 会出现"预计还能用 137 小时"这种没人信的数字。超过 100 小时不是
        // "能用 4 天"，而是"这么算根本没意义"——闲置功耗和实际使用差太远。
        if (hours < 0 || hours > 100)
        {
            e.Text = charging ? "预计充满时间：异常（请忽略）" : "预计可用时间：异常（请忽略）";
            return e;
        }

        e.Known = true;
        e.PercentPerHour = slope;
        string dur = DurText(hours);
        e.Text = charging ? ("预计 " + dur + " 后充满") : ("预计还能用 " + dur);
        return e;
    }

    static string DurText(double hours)
    {
        int totalMin = (int)Math.Round(hours * 60.0);
        if (totalMin < 1) totalMin = 1;
        int h = totalMin / 60, m = totalMin % 60;
        if (h == 0) return m + " 分钟";
        if (m == 0) return h + " 小时";
        return h + " 小时 " + m + " 分钟";
    }

    // ---------------- 输出 ----------------
    static string Colorize(int level)
    {
        if (level >= 60) return "绿";
        if (level >= 30) return "黄";
        return "红";
    }

    /// <summary>
    /// 估算器自检：灌合成样本，验证斜率/文案。`--estimate-selftest`
    /// 不用等真实电量变化，也不用碰鼠标。
    /// —— 返回失败项数
    /// </summary>
    static int _estFail;
    static void CheckEstimate(string name, List<double[]> sel, State now, int wantMin)
    {
        // 走完整链路（含相位/方向过滤 + 补当前读数），
        // 不然自检验的是另一段代码，等于白测。
        const long t0 = 1800000000;
        // nowSec 就取最后一个样本的时刻，这样不会再多补一条"当前读数"，
        // 斜率才能和期望值精确对上。
        long nowSec = sel.Count > 0 ? (long)sel[sel.Count - 1][0] : t0;
        var est = PredictFromRows(sel, now, nowSec);
        bool ok;
        string detail;
        if (wantMin < 0)
        {
            ok = !est.Known;
            detail = est.Known ? "应无结论却有：" + est.Text : "无结论（对）";
        }
        else if (!est.Known)
        {
            ok = false;
            detail = "应得结论，实际：" + est.Text;
        }
        else
        {
            double hours = now.Charge == ChargeState.Charging
                ? (100.0 - now.Level) / est.PercentPerHour
                : now.Level / (-est.PercentPerHour);
            int gotMin = (int)Math.Round(hours * 60.0);
            ok = Math.Abs(gotMin - wantMin) <= 2;
            detail = "得到 " + gotMin + " 分钟，期望 " + wantMin
                + "（斜率 " + est.PercentPerHour.ToString("F2") + " %/h）";
        }
        if (!ok) _estFail++;
        Console.WriteLine((ok ? "  OK   " : "  FAIL ") + name + "  " + detail);
    }

    static int EstimateSelfTest()
    {
        const long t0 = 1800000000;
        _estFail = 0;

        // 1) 放电：20%/小时，当前 60% → 3 小时
        //    样本必须和当前读数衔接（末尾就是 60%），否则会触发"电量刚跳变"保护。
        var a = new List<double[]>();
        a.Add(new double[] { t0 + 0 * 900, 80, 0, 0 });   // 每 15 分钟掉 5% = 20%/h
        a.Add(new double[] { t0 + 1 * 900, 75, 0, 0 });
        a.Add(new double[] { t0 + 2 * 900, 70, 0, 0 });
        a.Add(new double[] { t0 + 3 * 900, 65, 0, 0 });
        a.Add(new double[] { t0 + 4 * 900, 60, 0, 0 });
        CheckEstimate("放电 60% @ 20%/h", a,
            new State { Level = 60, Charge = ChargeState.Discharging }, 180);

        // 2) 充电：25%/小时，当前 50% → 2 小时
        var b = new List<double[]>();
        b.Add(new double[] { t0 + 0 * 900, 25, 1, 0 });   // 每 15 分钟涨 6.25% = 25%/h
        b.Add(new double[] { t0 + 1 * 900, 31.25, 1, 0 });
        b.Add(new double[] { t0 + 2 * 900, 37.5, 1, 0 });
        b.Add(new double[] { t0 + 3 * 900, 43.75, 1, 0 });
        b.Add(new double[] { t0 + 4 * 900, 50, 1, 0 });
        CheckEstimate("充电 50% @ 25%/h", b,
            new State { Level = 50, Charge = ChargeState.Charging }, 120);

        // 3) 电量一动不动 → 必须不给结论，不能除以零编出一个数
        var c = new List<double[]>();
        for (int i = 0; i <= 20; i++)
            c.Add(new double[] { t0 + i * 600, 100, 0, 0 });
        CheckEstimate("电量恒为 100%", c,
            new State { Level = 100, Charge = ChargeState.Discharging }, -1);

        // 4) 样本太少（2 条）
        var d = new List<double[]>();
        d.Add(new double[] { t0, 90, 0, 0 });
        d.Add(new double[] { t0 + 600, 89, 0, 0 });
        CheckEstimate("只有 2 个样本", d,
            new State { Level = 89, Charge = ChargeState.Discharging }, -1);

        // 5) 已充满 → 直接说已充满，不算时间
        var e5 = new List<double[]>();
        e5.Add(new double[] { t0, 100, 0, 0 });
        var estFull = PredictFromRows(e5,
            new State { Level = 100, Charge = ChargeState.Full }, t0 + 60);
        bool okFull = !estFull.Known && estFull.Text == "已充满";
        if (!okFull) _estFail++;
        Console.WriteLine((okFull ? "  OK   " : "  FAIL ") + "已充满不估算  " + estFull.Text);

        // 6) 放电却在涨（刚拔线，样本相位混了）→ 不给结论
        var f = new List<double[]>();
        for (int i = 0; i <= 5; i++)
            f.Add(new double[] { t0 + i * 600, 50 + i * 2, 0, 0 });
        CheckEstimate("放电相位却在涨", f,
            new State { Level = 60, Charge = ChargeState.Discharging }, -1);

        // 7) 充电斜率却是负的（刚插上电）→ 不给结论
        var g = new List<double[]>();
        for (int i = 0; i <= 5; i++)
            g.Add(new double[] { t0 + i * 600, 60 - i * 2, 1, 0 });
        CheckEstimate("充电相位却在掉", g,
            new State { Level = 50, Charge = ChargeState.Charging }, -1);

        // 8) 时间跨度为零（同一秒重复写）→ 不给结论
        var h = new List<double[]>();
        for (int i = 0; i <= 4; i++)
            h.Add(new double[] { t0, 50 + i, 0, 0 });
        CheckEstimate("所有样本同一时刻", h,
            new State { Level = 54, Charge = ChargeState.Discharging }, -1);

        // 9) 先降后升（中途插了一下线）→ 不能被末尾的跳升带偏
        var k = new List<double[]>();
        for (int i = 0; i <= 5; i++)
            k.Add(new double[] { t0 + i * 1800, 100 - i, 0, 0 });      // 100→95
        k.Add(new double[] { t0 + 6 * 1800, 100, 0, 0 });              // 跳回 100
        CheckEstimate("放电中途跳回 100%", k,
            new State { Level = 100, Charge = ChargeState.Discharging }, -1);

        // 10) 真实可算的一组：放电 4 小时从 100% 到 92%（2%/小时），
        //     当前 92% → 46 小时
        var m = new List<double[]>();
        for (int i = 0; i <= 8; i++)
            m.Add(new double[] { t0 + i * 1800, 100 - i, 0, 0 });      // 每 30 分掉 1%
        CheckEstimate("放电 2%/h 慢速", m,
            new State { Level = 92, Charge = ChargeState.Discharging }, 46 * 60);

        // 11) 断档：晚上用了 4 小时掉 4%，然后程序退了 8 小时，再开又用 1 小时掉 1%。
        //     老实回归会把 8 小时算成"什么都没发生"，斜率被稀释到 0.5%/h → 190 小时。
        //     正确做法是丢掉断档前的段，只看最近这 1 小时：1%/h → 95% 剩 95 小时。
        var gap = new List<double[]>();
        for (int i = 0; i < 5; i++)
            gap.Add(new double[] { t0 + i * 1800, 100 - i, 0, 0 });           // 21:00 起，100→96
        long resume = t0 + 4 * 1800 + 8 * 3600;                               // 断 8 小时
        for (int i = 0; i <= 4; i++)
            gap.Add(new double[] { resume + i * 900, 96 - i * 0.25, 0, 0 });  // 1 小时掉 1%
        CheckEstimate("断档后只用最近一段", gap,
            new State { Level = 95, Charge = ChargeState.Discharging }, 95 * 60);

        Console.WriteLine();
        Console.WriteLine(_estFail == 0 ? "全部通过 (11 项)" : _estFail + " 项失败");
        return _estFail == 0 ? 0 : 1;
    }

    /// <summary>三态判定真值表自检。用 `--charge-selftest` 跑，不依赖鼠标真在充电。</summary>
    static int ChargeSelfTest()
    {
        // (charging_status, level, mode, 期望)
        var cases = new[]
        {
            // 设备说在充 → 一律充电中，不管电量和接法
            new object[] { true,  50,  LinkMode.Wireless24G, ChargeState.Charging },
            new object[] { true,  99,  LinkMode.Wireless24G, ChargeState.Charging },
            new object[] { true,  100, LinkMode.Wired,       ChargeState.Charging },
            new object[] { true,  100, LinkMode.Wireless24G, ChargeState.Charging },
            // 设备说没充、但插着线且满电 → 已充满
            new object[] { false, 100, LinkMode.Wired,       ChargeState.Full },
            // 插着线但没满 → 还在用电池/边用边充，仍算使用中
            new object[] { false, 99,  LinkMode.Wired,       ChargeState.Discharging },
            new object[] { false, 50,  LinkMode.Wired,       ChargeState.Discharging },
            // 无线满电 → 使用中（没有在供电）
            new object[] { false, 100, LinkMode.Wireless24G, ChargeState.Discharging },
            new object[] { false, 50,  LinkMode.Wireless24G, ChargeState.Discharging },
        };

        int fail = 0;
        foreach (var c in cases)
        {
            var p = new PowerInfo { Charging = (bool)c[0], Level = (int)c[1] };
            var got = DecideCharge(p, (LinkMode)c[2]);
            var want = (ChargeState)c[3];
            bool ok = got == want;
            if (!ok) fail++;
            Console.WriteLine((ok ? "  OK   " : "  FAIL ")
                + "charging=" + ((bool)c[0] ? "1" : "0")
                + " level=" + c[1]
                + " mode=" + ModeText((LinkMode)c[2])
                + " → " + got + " (期望 " + want + ")");
        }
        Console.WriteLine();
        Console.WriteLine(fail == 0 ? "全部通过 (" + cases.Length + " 项)" : fail + " 项失败");
        return fail == 0 ? 0 : 1;
    }

    /// <summary>`--estimate`：把历史样本和当前估算摊开看，方便判断为什么"还没结果"。</summary>
    static int ShowEstimate()
    {
        var st = Snapshot();
        if (st == null)
        {
            Console.WriteLine("读取失败：鼠标没连上（休眠或未配对）。");
            return 1;
        }
        Console.WriteLine("当前: " + st.Level + "%  " + st.Charge
            + "  " + ModeText(st.Mode) + "  " + st.Voltage + " mV");

        string path = HistoryPath();
        Console.WriteLine("历史文件: " + (path ?? "(不可用)"));

        var rows = ReadHistory();
        Console.WriteLine("样本总数: " + rows.Count);
        RecordSample(st);   // --estimate 也顺手记一条，否则永远攒不起来
        rows = ReadHistory();

        if (rows.Count > 0)
        {
            long t0 = (long)rows[0][0], t1 = (long)rows[rows.Count - 1][0];
            var a = DateTimeOffset.FromUnixTimeSeconds(t0).ToLocalTime();
            var b = DateTimeOffset.FromUnixTimeSeconds(t1).ToLocalTime();
            Console.WriteLine("时间跨度: " + a.ToString("MM-dd HH:mm") + " → "
                + b.ToString("MM-dd HH:mm") + "（" + (int)((t1 - t0) / 60) + " 分钟）");

            // 逐段列出电量真正变化的地方，一眼能看出有没有可用斜率
            Console.WriteLine();
            Console.WriteLine("电量变化记录:");
            int prev = -1; int printed = 0;
            for (int i = 0; i < rows.Count; i++)
            {
                int lv = (int)rows[i][1];
                if (i == 0 || lv != prev)
                {
                    var t = DateTimeOffset.FromUnixTimeSeconds((long)rows[i][0]).ToLocalTime();
                    Console.WriteLine("  " + t.ToString("MM-dd HH:mm:ss") + "  " + lv + "%"
                        + "  " + (rows[i][2] >= 0.5 ? "充电" : "放电"));
                    printed++;
                    if (printed > 40) { Console.WriteLine("  ...（太多，已截断）"); break; }
                }
                prev = lv;
            }
        }

        Console.WriteLine();
        var est = Predict(st);
        Console.WriteLine("估算: " + est.Text);
        if (est.Known)
            Console.WriteLine("      斜率 " + est.PercentPerHour.ToString("F2") + " %/小时，"
                + est.Samples + " 个样本，跨度 " + (int)Math.Round(est.SpanMinutes) + " 分钟");
        else
            Console.WriteLine("      样本 " + est.Samples + " 个，跨度 "
                + (int)Math.Round(est.SpanMinutes) + " 分钟");
        return 0;
    }

    static int Main(string[] args)
    {
        bool tray = args.Length == 0;       // 无参数（双击）= 托盘常驻
        bool watch = false;
        bool help = false;
        int interval = 30;

        for (int i = 0; i < args.Length; i++)
        {
            string a = args[i];
            if (a == "--tray" || a == "-t")
            {
                tray = true;
                int v;
                if (i + 1 < args.Length && int.TryParse(args[i + 1], out v)) interval = v;
            }
            else if (a == "--watch" || a == "-w")
            {
                watch = true; tray = false;
                int v;
                if (i + 1 < args.Length && int.TryParse(args[i + 1], out v)) interval = v;
            }
            else if (a == "--once")
            {
                tray = false; watch = false;
            }
            else if (a == "--dump-icons") { Tray.DumpIcons(args.Length > i + 1 ? args[i + 1] : "icons"); return 0; }
            else if (a == "--mode")
            {
                EnsureConsole();
                return ShowMode();
            }
            else if (a == "--charge-selftest")
            {
                EnsureConsole();
                try { Console.OutputEncoding = System.Text.Encoding.UTF8; } catch { }
                return ChargeSelfTest();
            }
            else if (a == "--tip-selftest")
            {
                EnsureConsole();
                try { Console.OutputEncoding = System.Text.Encoding.UTF8; } catch { }
                return Tray.TipSelfTest();
            }
            else if (a == "--estimate-selftest")
            {
                EnsureConsole();
                try { Console.OutputEncoding = System.Text.Encoding.UTF8; } catch { }
                return EstimateSelfTest();
            }
            else if (a == "--estimate")
            {
                EnsureConsole();
                try { Console.OutputEncoding = System.Text.Encoding.UTF8; } catch { }
                return ShowEstimate();
            }
            else if (a == "--dpi")
            {
                EnsureConsole();
                var info = SnapshotDpi();
                Console.WriteLine("DPI 档位: " + DumpDpi(info));
                if (info != null && info.Current > 0)
                    Console.WriteLine("当前 DPI: " + info.Current);
                return 0;
            }
            else if (a == "--help" || a == "-h" || a == "-?")
            {
                help = true;
            }
        }

        // 托盘模式不碰控制台：双击时连窗口都不该出现。
        if (tray)
        {
            // 只允许一个实例，否则反复双击会叠加好几个图标
            bool isNew;
            using (var mutex = new Mutex(true, @"Local\RogBatteryTray", out isNew))
            {
                if (!isNew) return 0;
                return Tray.Run(interval);
            }
        }

        EnsureConsole();

        // 输出编码：中文 Windows 控制台默认 936(GBK)，中文本来就能显示，
        // 只有显式要求时才切成 UTF-8。
        try
        {
            if (Environment.GetEnvironmentVariable("ROG_BATTERY_UTF8") == "1")
                Console.OutputEncoding = System.Text.Encoding.UTF8;
        }
        catch { }

        if (help)
        {
            Console.WriteLine("用法: rog-battery.exe [选项]");
            Console.WriteLine("  不带参数        托盘常驻，图标上显示电量（双击就是这个）");
            Console.WriteLine("  --tray [秒]     同上，可指定刷新间隔（默认 30）");
            Console.WriteLine("  --once          只读一次然后退出");
            Console.WriteLine("  --watch [秒]    在控制台持续刷新（默认 30）");
            Console.WriteLine("  --mode          打印当前连接方式（有线 / 2.4G）和命令通道");
            Console.WriteLine("  --dpi           打印四档 DPI 和当前档位");
            Console.WriteLine("  --estimate      看剩余/充满时间的历史样本和当前估算");
            Console.WriteLine("  --charge-selftest    充电三态判定真值表自检");
            Console.WriteLine("  --estimate-selftest  估算器自检（合成数据，不用等真掉电）");
            Console.WriteLine("  --tip-selftest       悬浮提示排版自检（含 127 字符上限）");
            Console.WriteLine("  --dump-icons <目录>  把各电量档位的图标导出成 PNG");
            return 0;
        }

        do
        {
            string err;
            LinkMode mode;
            var info = ReadPower(out err, out mode);
            string stamp = DateTime.Now.ToString("HH:mm:ss");
            if (info == null)
            {
                Console.WriteLine("[" + stamp + "] 读取失败: " + err);
            }
            else
            {
                var cs = DecideCharge(info, mode);
                string chg = mode == LinkMode.Wired ? "  🔌有线" : "  📶2.4G";
                chg += cs == ChargeState.Charging ? " ⚡充电中"
                     : cs == ChargeState.Full ? " ✅已充满"
                     : " 🔋使用中";
                Console.WriteLine("[" + stamp + "] ROG 龙鳞 Ace Mini   电量 " + info.Level + "% ("
                    + Colorize(info.Level) + ")   电压 " + info.Voltage + " mV" + chg);
                Console.WriteLine("            低电量阈值 " + info.LowPower
                    + "%   电池类型 " + info.BatteryType
                    + "   充满效果 " + info.FullEffect + "   睡眠模式 " + info.Sleep);

                var st = FromDevice(info, mode);
                RecordSample(st);
                var est = Predict(st);
                Console.WriteLine("            " + est.Text
                    + (est.Known ? "（" + est.Samples + " 个样本，跨度 "
                        + (int)Math.Round(est.SpanMinutes) + " 分钟）" : ""));
            }
            if (!watch) break;
            Thread.Sleep(Math.Max(1, interval) * 1000);
        } while (true);

        return 0;
    }
}
