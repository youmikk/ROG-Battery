// settings.cs —— rog-battery 的少量用户设置（目前只有刷新间隔）
//
// 为什么要有这个文件：
//   以前刷新间隔只能靠命令行 --tray 60 指定，双击启动永远是写死的 30 秒，
//   而 30 秒对一块鼠标电池来说太频繁了 —— 电量在一个刷新周期内根本不会
//   变化一格，读得再勤也只是白白唤醒接收器、多耗一点接收器自己的电。
//   改成 60 秒起步，再给用户一个菜单随时调。
//
// 存哪里（按优先级）：
//   1. 绿色版：exe 同目录的 settings.ini —— 拷走整个文件夹，设置跟着走。
//   2. 兜底：%LOCALAPPDATA%\rog-battery\settings.ini
//      打包成 MSIX 后 exe 在 C:\Program Files\WindowsApps\ 下只读，第 1 项
//      必然失败；而包内写 %LOCALAPPDATA% 会被文件虚拟化透明重定向到
//      %LOCALAPPDATA%\Packages\<包族名>\LocalCache\Local\ 下，读回来也自动
//      命中，所以同一个路径在包内外都成立。这套判断跟 rog-battery.cs 里
//      HistoryPath() 用的是同一招（试写一次才算数，只看目录存在不够）。
//
// 格式：故意用最简单的 key=value 纯文本，不用 XML/JSON。
//   出问题时用户能直接拿记事本改；写坏了也不会让程序启动不了
//   （解析失败就当没这项，回落到默认值）。

using System;
using System.Globalization;

static class Settings
{
    /// <summary>默认刷新间隔（秒）。1 分钟。</summary>
    public const int DEFAULT_INTERVAL_SEC = 60;

    /// <summary>菜单里提供的档位，单位秒。顺序即菜单顺序。</summary>
    public static readonly int[] INTERVAL_CHOICES = { 30, 60, 180, 300 };

    const string FileName = "settings.ini";
    const string KeyInterval = "refresh_interval_sec";

    static string _path;
    static bool _loaded;
    static int _intervalSec = DEFAULT_INTERVAL_SEC;

    /// <summary>settings.ini 的完整路径；全都写不进去时返回 null（此时不持久化，但功能照常）。</summary>
    public static string Path
    {
        get
        {
            if (_path == null) _path = ResolvePath();
            return _path;
        }
    }

    static string ResolvePath()
    {
        // 1) 绿色版：exe 同目录。真的试写一次，成功才用它。
        try
        {
            string exe = System.Reflection.Assembly.GetEntryAssembly().Location;
            if (!string.IsNullOrEmpty(exe))
            {
                string dir = System.IO.Path.GetDirectoryName(exe);
                if (!string.IsNullOrEmpty(dir) && System.IO.Directory.Exists(dir))
                {
                    string same = System.IO.Path.Combine(dir, FileName);
                    // 只碰这个 key：文件已存在时不要清空用户其他设置，
                    // 不存在时创建空文件。AppendAllText("") 两种情况下都安全。
                    System.IO.File.AppendAllText(same, "", System.Text.Encoding.ASCII);
                    return same;
                }
            }
        }
        catch { }

        // 2) 兜底：AppData（MSIX 下由系统重定向）
        try
        {
            string dir = System.IO.Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                "rog-battery");
            System.IO.Directory.CreateDirectory(dir);
            string p = System.IO.Path.Combine(dir, FileName);
            System.IO.File.AppendAllText(p, "", System.Text.Encoding.ASCII);
            return p;
        }
        catch { return null; }
    }

    static void EnsureLoaded()
    {
        if (_loaded) return;
        _loaded = true;
        bool foundKey = false;
        try
        {
            string p = Path;
            if (p == null || !System.IO.File.Exists(p)) return;
            foreach (string raw in System.IO.File.ReadAllLines(p))
            {
                string line = raw.Trim();
                if (line.Length == 0 || line[0] == '#' || line[0] == ';') continue;
                int eq = line.IndexOf('=');
                if (eq <= 0) continue;
                string k = line.Substring(0, eq).Trim();
                string v = line.Substring(eq + 1).Trim();
                if (!string.Equals(k, KeyInterval, StringComparison.OrdinalIgnoreCase)) continue;
                foundKey = true;
                int n;
                if (int.TryParse(v, NumberStyles.Integer, CultureInfo.InvariantCulture, out n))
                    _intervalSec = NormalizeInterval(n);
            }
        }
        catch { /* 文件读坏了就静默用默认值，绝不因此启动失败 */ }

        // 首次运行（文件刚被创建、或用户手工删掉了这个 key）时把默认值写进去。
        // 不写的话用户打开 settings.ini 看到的是个空文件，根本不知道该改什么。
        if (!foundKey) Save();
    }

    /// <summary>
    /// 刷新间隔（秒）。没设置过就是 DEFAULT_INTERVAL_SEC。
    /// 传进来的值先夹到合理范围：太小会让 HID 命令互相挤，太大则托盘的
    /// 电量看起来像卡住了。
    /// </summary>
    public static int IntervalSec
    {
        get { EnsureLoaded(); return _intervalSec; }
        set
        {
            EnsureLoaded();
            int n = NormalizeInterval(value);
            if (n == _intervalSec) return;
            _intervalSec = n;
            Save();
        }
    }

    static int NormalizeInterval(int sec)
    {
        if (sec < 10) return 10;        // 10 秒以下没意义，一次 HID 往返就要 100~200ms
        if (sec > 3600) return 3600;    // 超过 1 小时干脆别刷新了
        return sec;
    }

    static void Save()
    {
        string p = Path;
        if (p == null) return;
        try
        {
            // 先写临时文件再替换：中途崩掉不至于留下半个文件。
            // 同目录才能保证 Move-Item 是原子 rename（跨卷会退化成复制）。
            string tmp = p + ".tmp";
            string text =
                "# rog-battery 设置。可以直接用记事本改，改完重启程序生效。\r\n" +
                "# refresh_interval_sec: 刷新间隔（秒）。托盘右键菜单里也能改。\r\n" +
                KeyInterval + "=" + _intervalSec.ToString(CultureInfo.InvariantCulture) + "\r\n";
            System.IO.File.WriteAllText(tmp, text, new System.Text.UTF8Encoding(false));
            if (System.IO.File.Exists(p)) System.IO.File.Delete(p);
            System.IO.File.Move(tmp, p);
        }
        catch { /* 写不进去就算了，本次运行仍然用内存里的值 */ }
    }

    /// <summary>把秒数格式化成人看的样子：60 → "1 分钟"，30 → "30 秒"。</summary>
    public static string Describe(int sec)
    {
        if (sec % 60 == 0 && sec >= 60)
        {
            int m = sec / 60;
            return m + " 分钟";
        }
        return sec + " 秒";
    }
}
