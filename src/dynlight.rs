
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
            0 => crate::tr!("未定义"),
            1 => crate::tr!("键盘"),
            2 => crate::tr!("鼠标"),
            3 => crate::tr!("游戏手柄"),
            4 => crate::tr!("麦克风"),
            5 => crate::tr!("鼠标垫"),
            6 => crate::tr!("机箱"),
            7 => crate::tr!("耳机"),
            8 => crate::tr!("音箱"),
            9 => crate::tr!("显示器"),
            10 => crate::tr!("机箱内灯带"),
            _ => crate::tr!("其他"),
        }
    }
}

/// 缓存：`None` = 还没探测过；`Some(None)` = 探测过但没找到。
///
/// 缓存的目的不是省几十毫秒，而是保证「绘制时绝不碰 HID」——
/// 界面每帧都会调 [`info`]，而枚举 HID 要打开十几个句柄。
static CACHE: Mutex<Option<Option<LampInfo>>> = Mutex::new(None);

/// 取缓存锁。中毒了就取回内部值继续用：这里没有「破了就不能用」的不变量，
/// 而 `if let Ok(g) = CACHE.lock()` 的旧写法一旦中毒就会**每帧**重跑全量枚举。
fn cache() -> std::sync::MutexGuard<'static, Option<Option<LampInfo>>> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

/// 取探测结果（只读缓存；第一次调用才会真正探测）。
///
/// 探测放在**持锁**区间内：枚举要开十几个句柄、耗时几十毫秒，并发首调各跑一遍
/// 既浪费又会在界面首帧叠出明显卡顿。持锁期间不做任何回调，不会自锁。
pub fn info() -> Option<LampInfo> {
    let mut g = cache();
    if let Some(v) = *g {
        return v;
    }
    let v = probe();
    *g = Some(v);
    v
}

/// 重新探测（「重新检测」按钮 / 设备热插拔后）。
pub fn refresh() -> Option<LampInfo> {
    *cache() = None;
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
    let (lamp_count, kind) = read_attributes(&api, &iface.path)?;
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
        Err(crate::trf!(
            "系统拒绝打开（返回 {code}）：{target}",
            code = code,
            target = target
        ))
    }
}

/// 打开 `ms-settings:personalization-lighting`。
pub fn open_settings() -> Result<(), String> {
    shell_open("ms-settings:personalization-lighting").map_err(|e| {
        crate::trf!(
            "打不开系统设置页（{e}），可手动打开 设置 → 个性化 → 动态光效",
            e = e
        )
    })
}

// ───────────────────────── 底层：HidD_GetFeature ─────────────────────────

fn read_attributes(api: &crate::win32::HidApi, path: &str) -> Option<(u16, u16)> {
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
    let ok = unsafe { (api.get_feature)(h, buf.as_mut_ptr(), buf.len() as u32) };
    unsafe { crate::win32::CloseHandle(h) };
    if ok == 0 {
        return None;
    }

    let lamp_count = u16::from_le_bytes([buf[1], buf[2]]);
    let kind = u16::from_le_bytes([buf[15], buf[16]]);
    Some((lamp_count, kind))
}
