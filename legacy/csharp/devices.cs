// devices.cs —— ROG 外设型号表（VID 0x0B05）
//
// 数据来源：ASUS Gear Link 的 hid-whitelist.json + 每个 PID 的 manifest.json
//   https://gearlink.asus.com/external-configs/hid-whitelist.json
//   https://gearlink.asus.com/view/<PID十进制>/manifest.json   → caps.deviceName / caps.deviceNameZH
// 抓取日期 2026-10-21，白名单 102 个 PID 全部取得 manifest。
//
// 为什么用这张表而不是硬编码一个 PID：
//   接收器扫描响应只给出 PID + controlReportId + notifyReportId，
//   型号名必须自己映射。原代码写死 0x1B65（龙鳞 Ace Mini），
//   换任何别的 ROG 鼠标都会挑不中。
//
// 命名约定：同一型号通常占两个相邻 PID（有线直连 / 2.4G 经接收器），
//   Spatha X 65K 与 Harpe II Ace-PBZ 有三个，都归到同一个型号名下。
//
// 注意：不要加 namespace RogBattery —— rog-battery.cs 里有个同名的 static class RogBattery，
//   二者会撞成 CS0101（命名空间与类型同名）。整个项目都在全局命名空间里。

using System.Collections.Generic;

static class Devices
{
    public const ushort VID = 0x0B05;

    // 接收器（dongle）PID —— 这些自身没有电量，只是转发通道。
    // 0x1ACF / 0x1AD1 是 OMNI / SpeedNova 的另一形态（Gear Link hidPattern=3），
    // 协议相同，所以一并当作可用接收器。
    public static readonly HashSet<ushort> DonglePids = new HashSet<ushort>
    {
        0x1ACE,   // ROG OMNI RECEIVER
        0x1ACF,   // ROG OMNI RECEIVER（另一形态）
        0x1AD0,   // ROG SPEEDNOVA 8K RECEIVER
        0x1AD1,   // ROG SPEEDNOVA 8K RECEIVER（另一形态）
        0x1D54,   // Asus Dongle
    };

    // 接收器自己的名字。它们不出现在 manifest 的 hasPowerInfo 列表里
    // （本身没电池），所以不在 Names 表内，单独放一份。
    static readonly Dictionary<ushort, string> DongleNames = new Dictionary<ushort, string>
    {
        { 0x1ACE, "ROG OMNI 无线接收器" },
        { 0x1ACF, "ROG OMNI 无线接收器" },
        { 0x1AD0, "ROG SpeedNova 8K 无线接收器" },
        { 0x1AD1, "ROG SpeedNova 8K 无线接收器" },
        { 0x1D54, "华硕无线接收器" },
    };

    // PID → 型号名。键是 PID，值是 (英文名, 中文名)。
    // 只收「有电池」的设备：manifest 里 hasPowerInfo=true 的鼠标、键盘、手柄。
    static readonly Dictionary<ushort, string[]> Names = new Dictionary<ushort, string[]>
    {
        // ---------------- 鼠标 ----------------
        { 0x1A92, new[] { "ROG HARPE ACE AIM LAB EDITION", "ROG 龙鳞 Ace Aim Lab 合作版" } },
        { 0x1A94, new[] { "ROG HARPE ACE AIM LAB EDITION", "ROG 龙鳞 Ace Aim Lab 合作版" } },
        { 0x1B16, new[] { "ROG KERIS II ACE", "ROG 月刃2 ACE" } },
        { 0x1B18, new[] { "ROG KERIS II ACE", "ROG 月刃2 ACE" } },
        { 0x1B63, new[] { "ROG HARPE ACE MINI", "ROG 龙鳞 Ace Mini" } },
        { 0x1B65, new[] { "ROG HARPE ACE MINI", "ROG 龙鳞 Ace Mini" } },
        { 0x1B67, new[] { "ROG HARPE ACE EXTREME", "ROG 龙鳞 Ace Extreme" } },
        { 0x1B69, new[] { "ROG HARPE ACE EXTREME", "ROG 龙鳞 Ace Extreme" } },
        { 0x1C0C, new[] { "ROG KERIS II ORIGIN", "ROG 月刃2 RGB" } },
        { 0x1C0E, new[] { "ROG KERIS II ORIGIN", "ROG 月刃2 RGB" } },
        { 0x1C69, new[] { "ROG HARPE II ACE", "ROG 龙鳞 2 Ace" } },
        { 0x1C6B, new[] { "ROG HARPE II ACE", "ROG 龙鳞 2 Ace" } },
        { 0x1CD3, new[] { "PROART MOUSE MD301", "PROART MOUSE MD301" } },
        { 0x1CD5, new[] { "PROART MOUSE MD301", "PROART MOUSE MD301" } },
        { 0x1D4C, new[] { "ROG KERIS II ORIGIN-KJP", "ROG 月刃 2 RGB - KJP" } },
        { 0x1D4E, new[] { "ROG KERIS II ORIGIN-KJP", "ROG 月刃 2 RGB - KJP" } },
        { 0x1D7A, new[] { "ROG HARPE II EXTREME EDITION 20", "ROG 龙鳞 2 Extreme 20周年版" } },
        { 0x1D7B, new[] { "ROG HARPE II EXTREME EDITION 20", "ROG 龙鳞 2 Extreme 20周年版" } },
        { 0x1D7C, new[] { "ROG HARPE II EXTREME EDITION 20", "ROG 龙鳞 2 Extreme 20周年版" } },
        { 0x1DB0, new[] { "ROG GLADIUS IV ACE", "ROG 战刃 4 ACE" } },
        { 0x1DBF, new[] { "ROG GLADIUS IV ACE", "ROG 战刃 4 ACE" } },
        { 0x1DDE, new[] { "ROG SPATHA X 65K", "ROG 斯巴达 X 65K" } },
        { 0x1DDF, new[] { "ROG SPATHA X 65K", "ROG 斯巴达 X 65K" } },
        { 0x1DE0, new[] { "ROG SPATHA X 65K", "ROG 斯巴达 X 65K" } },
        { 0x1DE1, new[] { "ROG SPATHA X 65K", "ROG 斯巴达 X 65K" } },
        { 0x1E2F, new[] { "ROG HARPE II ACE MINI", "ROG 龙鳞 2 Ace Mini" } },
        { 0x1E32, new[] { "ROG HARPE II ACE MINI", "ROG 龙鳞 2 Ace Mini" } },
        { 0x1E4A, new[] { "ROG HARPE II ACE-PBZ", "ROG 龙鳞2 ACE - PBZ" } },
        { 0x1E4B, new[] { "ROG HARPE II ACE-PBZ", "ROG 龙鳞2 ACE - PBZ" } },
        { 0x1E4D, new[] { "ROG HARPE II ACE-PBZ", "ROG 龙鳞2 ACE - PBZ" } },
        { 0x1E4E, new[] { "ROG GLADIUS IV ACE MAX", "ROG 战刃 4 ACE MAX" } },
        { 0x1E4F, new[] { "ROG GLADIUS IV ACE MAX", "ROG 战刃 4 ACE MAX" } },
        { 0x1E51, new[] { "ROG HARPE II ACE MINI DEMON1 EDITION", "ROG 龙鳞 2 Ace Mini Demon1联名版" } },
        { 0x1E52, new[] { "ROG HARPE II ACE MINI DEMON1 EDITION", "ROG 龙鳞 2 Ace Mini Demon1联名版" } },

        // ---------------- 键盘（同属 12 01 电量协议）----------------
        { 0x1A83, new[] { "ROG AZOTH", "ROG 夜魔" } },
        { 0x1A85, new[] { "ROG AZOTH", "ROG 夜魔" } },
        { 0x1AAE, new[] { "ROG STRIX SCOPE II 96 WIRELESS", "ROG 游侠2 98无线版" } },
        { 0x1AAF, new[] { "ROG STRIX SCOPE II 96 WIRELESS", "ROG 游侠2 98无线版" } },
        { 0x1AB0, new[] { "ROG STRIX SCOPE II 96 WIRELESS", "ROG 游侠2 98无线版" } },
        { 0x1B3F, new[] { "ROG AZOTH EXTREME", "ROG 夜魔 EXTREME" } },
        { 0x1B40, new[] { "ROG AZOTH EXTREME", "ROG 夜魔 EXTREME" } },
        { 0x1B42, new[] { "ROG AZOTH EXTREME", "ROG 夜魔 EXTREME" } },
        { 0x1C10, new[] { "ROG AZOTH 96 HE / LITE", "ROG 夜魔 98 HE / LITE" } },
        { 0x1C11, new[] { "ROG AZOTH 96 HE / LITE", "ROG 夜魔 98 HE / LITE" } },
        { 0x1C12, new[] { "ROG AZOTH 96 HE / LITE", "ROG 夜魔 98 HE / LITE" } },
        { 0x1C24, new[] { "ROG AZOTH X", "ROG 夜魔 X" } },
        { 0x1C25, new[] { "ROG AZOTH X", "ROG 夜魔 X" } },
        { 0x1C26, new[] { "ROG AZOTH X", "ROG 夜魔 X" } },
        { 0x1C2F, new[] { "ROG FALCATA", "ROG 影魔" } },
        { 0x1C30, new[] { "ROG FALCATA", "ROG 影魔" } },
        { 0x1C31, new[] { "ROG FALCATA", "ROG 影魔" } },
        { 0x1CE8, new[] { "ROG STRIX MORPH 96 WIRELESS", "ROG 术士 98" } },
        { 0x1CE9, new[] { "ROG STRIX MORPH 96 WIRELESS", "ROG 术士 98" } },
        { 0x1CEA, new[] { "ROG STRIX MORPH 96 WIRELESS", "ROG 术士 98" } },
        { 0x1CEF, new[] { "ROG AZOTH EXTREME SPECIAL EDITION", "ROG 夜魔 EXTREME SPECIAL EDITION" } },
        { 0x1CF0, new[] { "ROG AZOTH EXTREME SPECIAL EDITION", "ROG 夜魔 EXTREME SPECIAL EDITION" } },
        { 0x1CF1, new[] { "ROG AZOTH EXTREME SPECIAL EDITION", "ROG 夜魔 EXTREME SPECIAL EDITION" } },
        { 0x1D45, new[] { "PROART KEYBOARD KD300", "PROART KEYBOARD KD300" } },
        { 0x1D46, new[] { "PROART KEYBOARD KD300", "PROART KEYBOARD KD300" } },
        { 0x1D47, new[] { "PROART KEYBOARD KD300", "PROART KEYBOARD KD300" } },
        { 0x1D68, new[] { "ASUS TX75 ANALOG GAMING KEYBOARD", "华硕天选TX75" } },
        { 0x1D6A, new[] { "ASUS TX75 ANALOG GAMING KEYBOARD", "华硕天选TX75" } },
        { 0x1DA2, new[] { "ROG Azoth Extreme Edition 20", "ROG 夜魔 Extreme 20 周年版" } },
        { 0x1DA3, new[] { "ROG Azoth Extreme Edition 20", "ROG 夜魔 Extreme 20 周年版" } },
        { 0x1DA4, new[] { "ROG Azoth Extreme Edition 20", "ROG 夜魔 Extreme 20 周年版" } },
        { 0x1E0A, new[] { "ROG STRIX MORPH 96 X WIRELESS", "ROG 术士 98 X" } },
        { 0x1E0B, new[] { "ROG STRIX MORPH 96 X WIRELESS", "ROG 术士 98 X" } },
        { 0x1E0C, new[] { "ROG STRIX MORPH 96 X WIRELESS", "ROG 术士 98 X" } },
        { 0x1E5B, new[] { "ROG STRIX MORPH 96 WIRELESS-PBZ", "ROG 术士 98 - PBZ" } },
        { 0x1E5C, new[] { "ROG STRIX MORPH 96 WIRELESS-PBZ", "ROG 术士 98 - PBZ" } },
        { 0x1E5D, new[] { "ROG STRIX MORPH 96 WIRELESS-PBZ", "ROG 术士 98 - PBZ" } },

        // ---------------- 手柄 ----------------
        { 0x1C64, new[] { "ROG RAIKIRI II PRO PC", "ROG 雷切 II PRO PC" } },
        { 0x1C65, new[] { "ROG RAIKIRI II PRO PC", "ROG 雷切 II PRO PC" } },
        { 0x1C91, new[] { "ROG RAIKIRI II XBOX WIRELESS", "ROG 雷切 II XBOX无线手柄" } },
        { 0x1C92, new[] { "ROG RAIKIRI II XBOX WIRELESS", "ROG 雷切 II XBOX无线手柄" } },
    };

    // 接收器扫描响应里的 controlReportId 含义（Gear Link / libratbag 一致）：
    //   2 = 键盘槽位，3 = 鼠标槽位。
    // 槽位还能报出表里没有的新 PID，所以不能只靠表判断种类。
    public static bool IsKnown(ushort pid) { return Names.ContainsKey(pid); }

    /// <summary>PID 对应的中文型号名；表里没有则返回 null。</summary>
    public static string NameZh(ushort pid)
    {
        string[] v;
        return Names.TryGetValue(pid, out v) ? v[1] : null;
    }

    /// <summary>PID 对应的英文型号名；表里没有则返回 null。</summary>
    public static string NameEn(ushort pid)
    {
        string[] v;
        return Names.TryGetValue(pid, out v) ? v[0] : null;
    }

    /// <summary>型号名兜底：表里没有就用 PID 拼一个可读的名字。</summary>
    public static string NameZhOrPid(ushort pid)
    {
        return NameZh(pid) ?? ("ROG 外设 (0x" + pid.ToString("X4") + ")");
    }

    /// <summary>
    /// 三层取名：型号表 → 接收器表 → 设备固件自报的产品名 → PID 兜底。
    /// 固件自报名不可控（可能夹厂商缩写、英文、甚至空串），所以只当补充，
    /// 表里有的 PID 永远以表为准 —— 表里的名字是人工校对过的统一译名。
    /// </summary>
    public static string Display(ushort pid, string productName)
    {
        string zh = NameZh(pid);
        if (zh != null) return zh;

        string dg;
        if (DongleNames.TryGetValue(pid, out dg)) return dg;

        if (!string.IsNullOrEmpty(productName)) return "ROG " + productName;
        return "ROG 外设 (0x" + pid.ToString("X4") + ")";
    }

    /// <summary>表里收录的型号数量（--list-devices 与自检用）。</summary>
    public static int Count { get { return Names.Count; } }
}
