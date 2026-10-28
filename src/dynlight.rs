//! Windows「动态光效」(Dynamic Lighting) / HID LampArray 集成。
//!
//! # 为什么是这条路
//!
//! 2026-10-28 实测（`HidP_GetCaps` 遍历所有顶层集合）发现：本机测试设备
//! （ROG HARPE ACE MINI，USB 0B05:1ACE）
//! 除了私有命令通道（0xFF00/0xFF01/0xFF02、64 字节 in/out）之外，**还暴露了一个
//! 标准的 HID LampArray 集合**：
//!
//! ```text
//! VID_0B05 PID_1ACE  UP=0x59 U=0x01  in=0 out=0 feat=51   (MI_05)
//! ```
//!
//! 读它的属性报表（Feature Report ID 1）得到：
//!
//! | 字段 | 值 |
//! |---|---|
//! | LampCount | 1 |
//! | BoundingBox | 63.6 × 127.1 × 39.6 mm（与机身尺寸吻合）|
//! | LampArrayKind | 2 = Mouse |
//!
//! `Usage Page 0x59 / Usage 0x01` 正是 Windows 动态光效的设备选择器
//! （`System.DeviceInterface.Hid.UsagePage:=89 AND UsageId:=1`）唯一认的协议。
//! 也就是说：**灯根本不走私有通道**，之前往 0xFF01 写灯效报文没反应，是打错了集合。
//!
//! # 方案 A：交给系统
//!
//! 动态光效是 Windows 自带的（设置 → 个性化 → 动态光效）。它自己会找到这个
//! LampArray 集合并按用户选的「纯色 / 呼吸 / 渐变 / 波浪 / 闪烁」驱动灯。
//! 我们**不抢写** —— LampArray 是「最后写入者赢」，我们写一帧、系统下一帧就覆盖，
//! 两边打架只会闪。所以这里只做三件事：
//!
//! 1. 探测设备在不在（给界面显示，证明系统能看见它）；
//! 2. 一键打开系统的动态光效设置页；
//! 3. 提示用户 Armoury Crate 的灯效要先关掉（否则它会抢设备）。
//!
//! 厂商私有协议的 `ops::set_light` / CLI `setlight` 仍然保留（本机实测可写，
//! 只是会被系统覆盖），但 UI 里不再提供，避免「点了没反应」的错觉。

use std::sync::Mutex;

/// HID Usage Page：Lighting And Illumination / LampArray。
const USAGE_PAGE_LAMPARRAY: u16 = 0x0059;

/// LampArray 属性报表（Feature Report ID）。
const REPORT_ATTRIBUTES: u8 = 1;

/// 探测到的 LampArray 设备摘要。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LampInfo {
    /// 可寻址灯珠数量。单区设备是 1。
    pub lamp_count: u16,
    /// `LampArrayKind`：2 = 鼠标。
    pub kind: u16,
}

impl LampInfo {
    /// `LampArrayKind` 的显示名。规范只定义了几个值，其余按「其他」处理。
    pub fn kind_name(self) -> &'static str {
        match self.kind {
            0 => "未定义",
            1 => "键盘",
            2 => "鼠标",
            3 => "游戏手柄",
            4 => "麦克风",
            5 => "鼠标垫",
            6 => "机箱",
            7 => "耳机",
            8 => "音箱",
            9 => "显示器",
            10 => "机箱内灯带",
            _ => "其他",
        }
    }
}

/// 缓存：`None` = 还没探测过；`Some(None)` = 探测过但没找到。
///
/// 缓存的目的不是省几十毫秒，而是保证「绘制时绝不碰 HID」——
/// 界面每帧都会调 [`info`]，而枚举 HID 要打开十几个句柄。
static CACHE: Mutex<Option<Option<LampInfo>>> = Mutex::new(None);

/// 取探测结果（只读缓存；第一次调用才会真正探测）。
pub fn info() -> Option<LampInfo> {
    if let Ok(g) = CACHE.lock() {
        if let Some(v) = *g {
            return v;
        }
    }
    let v = probe();
    if let Ok(mut g) = CACHE.lock() {
        *g = Some(v);
    }
    v
}

/// 重新探测（「重新检测」按钮 / 设备热插拔后）。
pub fn refresh() -> Option<LampInfo> {
    if let Ok(mut g) = CACHE.lock() {
        *g = None;
    }
    info()
}

/// 启动时调一次，把探测挪到消息循环之前（别落在第一次 WM_PAINT 里）。
pub fn init() {
    let _ = refresh();
}

/// 真正去枚举 HID、找 0x59 集合、读属性报表。
fn probe() -> Option<LampInfo> {
    let api = crate::win32::HidApi::load().ok()?;
    let ifaces = crate::hid::enumerate(&api).ok()?;
    let iface = ifaces.iter().find(|i| i.usage_page == USAGE_PAGE_LAMPARRAY)?;
    let (lamp_count, kind) = read_attributes(&iface.path)?;
    if lamp_count == 0 {
        return None;
    }
    Some(LampInfo { lamp_count, kind })
}

/// 用 `ShellExecuteW` 打开任意文件 / 文件夹 / URI。
///
/// 不用 `Command::new("explorer.exe")`：后者要额外起子进程，且子进程的 shell
/// 激活在某些受限环境里会被挡掉，表现为「点了没反应、也没有任何报错」。
/// `ShellExecuteW` 是 shell 激活的标准做法，返回值 > 32 才算成功。
pub fn shell_open(target: &str) -> Result<(), String> {
    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            hwnd: crate::win32::Handle,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show_cmd: i32,
        ) -> crate::win32::Handle;
    }

    let op = crate::win32::to_wide("open");
    let file = crate::win32::to_wide(target);
    let r = unsafe {
        ShellExecuteW(
            core::ptr::null_mut(),
            op.as_ptr(),
            file.as_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            1, // SW_SHOWNORMAL
        )
    };
    let code = r as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(format!("系统拒绝打开（返回 {code}）：{target}"))
    }
}

/// 打开 `ms-settings:personalization-lighting`。
pub fn open_settings() -> Result<(), String> {
    shell_open("ms-settings:personalization-lighting").map_err(|e| {
        format!("打不开系统设置页（{e}），可手动打开 设置 → 个性化 → 动态光效")
    })
}

// ───────────────────────── 底层：HidD_GetFeature ─────────────────────────

// `hid.dll` 在本机这套工具链里**没有** `libhid.a`（`win32.rs` 顶部注释也写了这点），
// 所以不能用 `#[link(name = "hid")]`，只能 `LoadLibraryW` + `GetProcAddress` 动态取。
type FnHidDGetFeature = unsafe extern "system" fn(
    hid_device_object: crate::win32::Handle,
    report_buffer: *mut u8,
    report_buffer_length: u32,
) -> i32;

fn hid_get_feature() -> Option<FnHidDGetFeature> {
    let module = unsafe { crate::win32::LoadLibraryW(crate::win32::to_wide("hid.dll").as_ptr()) };
    if module.is_null() || module as isize == -1 {
        return None;
    }
    let sym = b"HidD_GetFeature\0";
    let ptr = unsafe { crate::win32::GetProcAddress(module, sym.as_ptr()) };
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { core::mem::transmute::<*mut core::ffi::c_void, FnHidDGetFeature>(ptr) })
}

/// 读 LampArray 属性报表，返回 `(LampCount, LampArrayKind)`。
///
/// 报表布局（按实测字节 + HID LampArray 1.0 规范）：
///
/// ```text
/// byte 0      Report ID (= 1)
/// byte 1..3   LampCount            u16 LE
/// byte 3..7   BoundingBoxWidth     u32 LE（微米）
/// byte 7..11  BoundingBoxHeight    u32 LE
/// byte 11..15 BoundingBoxDepth     u32 LE
/// byte 15..17 LampArrayKind        u16 LE
/// byte 17..21 MinUpdateInterval    u32 LE（微秒）
/// ```
fn read_attributes(path: &str) -> Option<(u16, u16)> {
    let get_feature = hid_get_feature()?;
    let wide = crate::win32::to_wide(path);
    let h = unsafe {
        crate::win32::CreateFileW(
            wide.as_ptr(),
            crate::win32::GENERIC_READ,
            crate::win32::FILE_SHARE_READ | 0x0000_0002, // FILE_SHARE_WRITE
            core::ptr::null_mut(),
            crate::win32::OPEN_EXISTING,
            0,
            core::ptr::null_mut(),
        )
    };
    if h.is_null() || h as isize == -1 {
        return None;
    }

    // 51 字节的 feature report，取 64 兜底。
    let mut buf = [0u8; 64];
    buf[0] = REPORT_ATTRIBUTES;
    let ok = unsafe { get_feature(h, buf.as_mut_ptr(), buf.len() as u32) };
    unsafe { crate::win32::CloseHandle(h) };
    if ok == 0 {
        return None;
    }

    let lamp_count = u16::from_le_bytes([buf[1], buf[2]]);
    let kind = u16::from_le_bytes([buf[15], buf[16]]);
    Some((lamp_count, kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 属性报表解析_实测字节() {
        // 2026-10-28 从 MI_05 真实读回的 24 字节：
        // 01 | 01 00 | 70 F8 00 00 | 9A F0 01 00 | CE 9A 00 00 | 02 00 | ...
        let buf: [u8; 24] = [
            0x01, 0x01, 0x00, 0x70, 0xF8, 0x00, 0x00, 0x9A, 0xF0, 0x01, 0x00, 0xCE, 0x9A, 0x00,
            0x00, 0x02, 0x00, 0x00, 0x00, 0xE8, 0x80, 0x00, 0x00, 0x00,
        ];
        let lamp_count = u16::from_le_bytes([buf[1], buf[2]]);
        let kind = u16::from_le_bytes([buf[15], buf[16]]);
        assert_eq!(lamp_count, 1);
        assert_eq!(kind, 2);
        assert_eq!(LampInfo { lamp_count, kind }.kind_name(), "鼠标");
    }

    #[test]
    fn 未知_kind_不猜() {
        let l = LampInfo {
            lamp_count: 4,
            kind: 0x7F,
        };
        assert_eq!(l.kind_name(), "其他");
    }
}
