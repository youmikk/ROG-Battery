//! rog-mouse-ctl —— ROG 龙鳞 Ace Mini（Harpe Ace Mini）HID 参数控制工具。
//!
//! 完全不依赖 ASUS 驱动 / Armoury Crate / GearLink：自己枚举 HID 接口、
//! 自己拼 64 字节 feature report、自己解析应答。
//!
//! 协议细节见 `_research\hid-write.md`（写）与 `_research\hid-features.md`（读）。
//!
//! 读模型：**一次读全 17 帧 + 缓存**（见 `snapshot.rs`）。

mod autostart;
mod backup;
mod chart;
mod cli;
mod dwm;
mod gui;
mod hid;
mod history;
mod ops;
mod paths;
mod proto;
mod settings;
mod snapshot;
mod tray_icon;
mod ui;
mod win32;

use std::process::ExitCode;

const USAGE: &str = r#"rog-mouse-ctl —— ROG 龙鳞 Ace Mini HID 参数控制（不依赖 ASUS 驱动）

用法：
  rog-mouse-ctl info [--fresh]                读全部信息（一次读全，默认吃 30 秒缓存）
  rog-mouse-ctl backup [文件]                 备份全部 31 项设置（默认 %USERPROFILE%\Documents\rog-mouse-ctl\backups\）
  rog-mouse-ctl restore <文件> [--save]       从备份还原（能量块最后单独重写并断言）
  rog-mouse-ctl save                          把当前 RAM 设置提交进 flash（0x50 0x03）

  rog-mouse-ctl setdpi <1-4> <dpi> [RRGGBB]   设置某档 DPI 与该档颜色（同一帧）
  rog-mouse-ctl setpoll <125|250|500|1000>    回报率
  rog-mouse-ctl setstage <1-4>                当前 DPI 档位
  rog-mouse-ctl setdebounce <0-7>             去抖：0=关 1=8ms … 7=32ms
  rog-mouse-ctl setsnap <0|1>                 角度吸附
  rog-mouse-ctl settuning <-20..20>           角度微调（步进 5）
  rog-mouse-ctl setlod <low|high>             抬起距离（⚠️ 会重置表面标定）
  rog-mouse-ctl setlight <mode> [亮] [RRGGBB] 灯光 mode: 0=Static 1=Breathing 2=ColorCycle
                                              4=React 6=BatteryState 240=Off
  rog-mouse-ctl setbutton <0-7> <dstHex>      按键绑定（dst=0x0000 即禁用，需 --yes）
  rog-mouse-ctl setsleep <code>               休眠：0=1分 1=2分 2=3分 3=5分 4=10分 255=永不
  rog-mouse-ctl setlowbatt <0-100>            低电量告警阈值（%）
  rog-mouse-ctl setfulleffect <0|1>           充满电时的灯效开关

  rog-mouse-ctl cache                         查看快照缓存状态
  rog-mouse-ctl cache clear                   删除快照缓存

  rog-mouse-ctl estimate                      电量历史 + 续航/充满时间估算（顺手记一条采样）

  rog-mouse-ctl quick                          轻量读取：只读 3 帧，输出 KEY=VALUE
  rog-mouse-ctl info --light                   同 quick（只读 3 帧）

  rog-mouse-ctl raw <cmdHex> <subHex> [tailHex...] [--dongle]
                                              原始帧调试口（**只读**核对用）

通用开关：
  --fresh    强制绕过缓存，重新读设备（info 用）
  --light    info 只读 3 帧（电量/回报率/当前档位），约 100-200ms，休眠时立即返回
  --keep     保留新值不还原（默认会**自动写回原值**，跑完不留痕迹）
  --save     显式提交进 flash（默认只改 RAM，不落盘）
  --yes      确认危险操作（目前只有 setbutton … 0 需要）

示例：
  rog-mouse-ctl info
  rog-mouse-ctl info --fresh
  rog-mouse-ctl backup
  rog-mouse-ctl setpoll 500                 # 改成 500 再自动改回 1000，仅演示
  rog-mouse-ctl setpoll 500 --keep --save   # 真正留在 500 并写入 flash
  rog-mouse-ctl setdpi 1 500                # 1 档改 500 DPI，跑完自动还原
  rog-mouse-ctl setlight 0 70 ff0000        # Static 红色 70% 亮度，跑完自动还原
  rog-mouse-ctl restore backup.txt --save
"#;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // ---- 入口分流 ----
    //
    // 判据严格用「有没有参数」：用户双击 exe 时没有参数，必须直接进 GUI。
    // 裸跑如果进 CLI 只会打印帮助然后退出，双击的人什么都看不到。
    //
    // 唯一例外是 `--help` / `-h` / `help`：这几个仍然走 CLI 打印帮助，
    // 否则在命令行里敲 `rog-mouse-ctl --help` 会弹出一个窗口，很奇怪。
    //
    // `--autostart` 是**开机自启专用参数**（见 `src/autostart.rs` 的 `RUN_ARG`）。
    // 它必须和裸跑走同一条路进 GUI —— 开机时没有控制台，不 FreeConsole 会闪黑框；
    // 而且它**绝不能落到下面的 CLI 分支**，那只会打印用法然后立刻退出，
    // 用户看到的就是「开机自启勾了但什么也没发生」。
    let wants_help = matches!(args.first().map(|s| s.as_str()), Some("-h" | "--help" | "help"));
    let wants_autostart = args.len() == 1 && args[0] == autostart::RUN_ARG;

    if args.is_empty() || wants_autostart {
        // 必须在**创建任何窗口之前**脱离控制台，否则黑框可能已经画出来了。
        // SAFETY: `FreeConsole` 只影响本进程的控制台关联，无指针参数。
        unsafe {
            win32::FreeConsole();
        }
        return run_gui();
    }
    let _ = wants_help; // 参数非空时由下面的 CLI 分支统一处理

    if wants_help {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    // 先解析通用开关，剩下的才是位置参数
    let fresh = args.iter().any(|a| a == "--fresh");
    let keep = args.iter().any(|a| a == "--keep");
    let save = args.iter().any(|a| a == "--save");
    let yes = args.iter().any(|a| a == "--yes");
    let use_dongle = args.iter().any(|a| a == "--dongle");
    let light = args.iter().any(|a| a == "--light");
    let pos: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(|s| s.as_str())
        .collect();

    if pos.is_empty() {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let cmd = pos[0];

    // 不需要设备的命令先处理掉（cache 相关）
    if cmd == "cache" {
        return match cmd_cache(&pos) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("错误：{e}");
                ExitCode::FAILURE
            }
        };
    }

    match run(cmd, &pos, keep, save, yes, fresh, use_dongle, light) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("错误：{e}");
            ExitCode::FAILURE
        }
    }
}

/// GUI 入口。控制台已经在上面的 [`main`] 里脱离干净，这里**不能**再 `println!`
/// —— 没有地方会接住它。需要日志就写 [`paths::cache_dir`] 下的文件。
fn run_gui() -> ExitCode {
    // DPI 感知必须在**任何窗口创建之前**声明，否则 Windows 会先按 96 DPI 建窗口，
    // 再把整窗当位图拉伸到实际缩放比 —— 那就是「字体发虚」的根因（GDI+ 画出来的
    // 抗锯齿文字被整体插值放大，边缘糊掉）。
    //
    // 只影响 GUI 分支：CLI 分支不建窗口，设不设都无所谓，但设了会让控制台的
    // 字符尺寸按物理像素算 —— 保持不设更安全。
    //
    // 失败（返回 false）不致命：老系统没有 `SetProcessDpiAwarenessContext` 时会走
    // 降级链，全失败也只是退回模糊状态，不该因此拒绝启动。
    let _ = dwm::enable_dpi_awareness();

    match gui::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // GUI 起不来时控制台已经没了，唯一的告知途径是消息框 + 日志文件。
            // 静默退出会让用户以为程序坏了。
            let dir = paths::cache_dir();
            let _ = paths::ensure_dir(&dir);
            let _ = std::fs::write(
                dir.join("gui.log"),
                format!("{} GUI 启动失败：{e}\n", settings::timestamp_for_humans()),
            );
            let text = win32::to_wide(&format!("图形界面启动失败：\n\n{e}"));
            let caption = win32::to_wide("rog-mouse-ctl");
            // MB_OK(0) | MB_ICONERROR(0x10)
            unsafe {
                win32::MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), 0x10);
            }
            ExitCode::FAILURE
        }
    }
}


fn run(
    cmd: &str,
    pos: &[&str],
    keep: bool,
    save: bool,
    yes: bool,
    fresh: bool,
    use_dongle: bool,
    light: bool,
) -> Result<(), String> {
    // 参数校验先做，避免为了一个拼错的字去动设备
    validate(cmd, pos)?;

    let api = win32::HidApi::load()?;
    let link = hid::open_link(&api)?;
    let mut dev = hid::HidDevice::open(&link.iface)?;

    // `raw` 也要写 feature report 才能发命令，所以同样需要写句柄
    if !dev.writable && !matches!(cmd, "info" | "backup" | "quick" | "estimate") {
        return Err("设备以只读方式打开（写句柄不可用），无法执行写命令。\
             可能被 Armoury Crate / GearLink 独占，先关掉它们。"
            .into());
    }

    let ttl = settings::load().cache_ttl_sec;
    let mut ctx = cli::Ctx::new(&link, ttl);

    match cmd {
        "info" => {
            if light {
                cli::cmd_quick(&mut ctx, &mut dev)
            } else {
                cli::cmd_info(&mut ctx, &mut dev, fresh)
            }
        }

        "quick" => cli::cmd_quick(&mut ctx, &mut dev),

        "backup" => {
            let name = format!("stock-{}.txt", settings::timestamp_for_filename());
            let path = paths::resolve_backup_path(pos.get(1).copied(), &name);
            let p = path.display().to_string();
            cli::cmd_backup(&mut ctx, &mut dev, &p)
        }

        "restore" => {
            let path = paths::resolve_backup_path(Some(pos[1]), "");
            cli::cmd_restore(&mut ctx, &mut dev, &path.display().to_string(), save)
        }

        "save" => cli::cmd_save(&mut ctx, &mut dev),

        "estimate" => cli::cmd_estimate(&mut ctx, &mut dev),

        "setdpi" => {
            let stage: usize = pos[1]
                .parse()
                .map_err(|_| format!("档位必须是 1-4，收到 {:?}", pos[1]))?;
            let dpi: i32 = pos[2]
                .parse()
                .map_err(|_| format!("DPI 必须是整数，收到 {:?}", pos[2]))?;
            let rgb = match pos.get(3) {
                Some(s) => Some(backup::parse_rgb(s)?),
                None => None,
            };
            cli::cmd_setdpi(&mut ctx, &mut dev, stage, dpi, rgb, keep, save).map(|_| ()).map_err(fail)
        }

        "setpoll" => {
            let hz: i32 = pos[1]
                .parse()
                .map_err(|_| format!("回报率必须是整数，收到 {:?}", pos[1]))?;
            cli::cmd_setpoll(&mut ctx, &mut dev, hz, keep, save).map(|_| ()).map_err(fail)
        }

        "setstage" => {
            let stage: u8 = pos[1]
                .parse()
                .map_err(|_| format!("档位必须是 1-4，收到 {:?}", pos[1]))?;
            cli::cmd_setstage(&mut ctx, &mut dev, stage, keep, save).map(|_| ()).map_err(fail)
        }

        "setdebounce" => {
            let code: u8 = pos[1]
                .parse()
                .map_err(|_| format!("去抖代码必须是 0-7，收到 {:?}", pos[1]))?;
            cli::cmd_setdebounce(&mut ctx, &mut dev, code, keep, save).map(|_| ()).map_err(fail)
        }

        "setsnap" => {
            let snap: u8 = pos[1]
                .parse()
                .map_err(|_| format!("角度吸附必须是 0 或 1，收到 {:?}", pos[1]))?;
            cli::cmd_setsnap(&mut ctx, &mut dev, snap, keep, save).map(|_| ()).map_err(fail)
        }

        "settuning" => {
            let v: i16 = pos[1]
                .parse()
                .map_err(|_| format!("角度微调必须是整数，收到 {:?}", pos[1]))?;
            cli::cmd_settuning(&mut ctx, &mut dev, v, keep, save).map(|_| ()).map_err(fail)
        }

        "setlod" => {
            let code = match pos[1].to_ascii_lowercase().as_str() {
                "low" => 0u8,
                "high" => 1u8,
                other => return Err(format!("只接受 low / high，收到 {other:?}")),
            };
            cli::cmd_setlod(&mut ctx, &mut dev, code, keep, save).map(|_| ()).map_err(fail)
        }

        "setlight" => {
            let mode_s = pos[1];
            let mode = if let Some(hex) = mode_s
                .strip_prefix("0x")
                .or_else(|| mode_s.strip_prefix("0X"))
            {
                u8::from_str_radix(hex, 16)
                    .map_err(|_| format!("灯光模式 {mode_s:?} 不是合法十六进制"))?
            } else {
                mode_s
                    .parse::<u8>()
                    .map_err(|_| format!("灯光模式 {mode_s:?} 不是合法数字"))?
            };
            let bright = match pos.get(2) {
                Some(s) => Some(
                    s.parse::<u8>()
                        .map_err(|_| format!("亮度必须是 0-100，收到 {s:?}"))?,
                ),
                None => None,
            };
            let rgb = match pos.get(3) {
                Some(s) => Some(backup::parse_rgb(s)?),
                None => None,
            };
            cli::cmd_setlight(&mut ctx, &mut dev, mode, bright, rgb, keep, save).map(|_| ()).map_err(fail)
        }

        "setsleep" => {
            let code = parse_u8_arg(pos[1], "休眠档位")?;
            cli::cmd_setsleep(&mut ctx, &mut dev, code, keep, save)
                .map(|_| ())
                .map_err(fail)
        }

        "setlowbatt" => {
            let pct = parse_u8_arg(pos[1], "低电量阈值")?;
            cli::cmd_setlowbatt(&mut ctx, &mut dev, pct, keep, save)
                .map(|_| ())
                .map_err(fail)
        }

        "setfulleffect" => {
            let on = parse_u8_arg(pos[1], "充满效果开关")?;
            cli::cmd_setfulleffect(&mut ctx, &mut dev, on, keep, save)
                .map(|_| ())
                .map_err(fail)
        }

        "setbutton" => {
            let slot: usize = pos[1]
                .parse()
                .map_err(|_| format!("槽位必须是 0-7，收到 {:?}", pos[1]))?;
            let dst = pos[2];
            let dst = if let Some(hex) = dst.strip_prefix("0x").or_else(|| dst.strip_prefix("0X")) {
                u16::from_str_radix(hex, 16)
            } else {
                u16::from_str_radix(dst, 16)
            }
            .map_err(|_| format!("动作码 {dst:?} 不是合法十六进制"))?;
            cli::cmd_setbutton(&mut ctx, &mut dev, slot, dst, keep, save, yes).map(|_| ()).map_err(fail)
        }

        "raw" => {
            let cmd = parse_hex_u8(pos[1], "cmd")?;
            let sub = parse_hex_u8(pos[2], "sub")?;
            // 尾部字节：`raw 12 04 00` 里 pos[3..] 都是 tail
            let mut tail: Vec<u8> = Vec::new();
            for s in &pos[3..] {
                tail.push(parse_hex_u8(s, "tail")?);
            }
            // `--dongle` 走接收器通道（0xFF02，report id 0x01）
            //
            // 超时 900ms，比 DEFAULT_TIMEOUT_MS(1200) 短。**这不是「raw 想快」，
            // 而是两条路径对 MISS 的解释不同**：
            //
            //   raw  —— 人手敲的调试口，会故意发不存在的 cmd 去试探协议边界
            //           （`raw 12 FF`）。MISS 是**预期结果**，早点告诉人比多等好。
            //   light/ops —— 高频自动路径，MISS **本身就是要判定的信号**（休眠 /
            //           链路断）。必须诚实等满一整个超时，不能因为设备刚好慢一点
            //           就提前判成「无应答」。
            //
            // 所以两个值保留差异是对的。改这里之前先想清楚是哪一类用法。
            const RAW_TIMEOUT_MS: u32 = 900;
            if use_dongle {
                let mut d = cli::open_dongle(&link.all_interfaces)?;
                cli::cmd_raw_on(&mut d, 1, cmd, sub, &tail, RAW_TIMEOUT_MS)
            } else {
                cli::cmd_raw(&mut ctx, &mut dev, cmd, sub, &tail, RAW_TIMEOUT_MS)
            }
        }

        other => Err(format!("未知子命令 {other:?}\n\n{USAGE}")),
    }
}

/// 解析一个十进制字节，但也吃 `0xFF` 这种十六进制写法。
///
/// 能量块三个 setter 用：用户既可能按 G-Helper 的说法写 `setsleep 4`，
/// 也可能照 `51 37` 帧直接写 `setsleep 0xFF`。
fn parse_u8_arg(s: &str, what: &str) -> Result<u8, String> {
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return u8::from_str_radix(h, 16)
            .map_err(|_| format!("{what} {s:?} 不是合法十六进制（00-FF）"));
    }
    s.parse::<u8>()
        .map_err(|_| format!("{what} 必须是 0-255 的整数（也可写 0xFF），收到 {s:?}"))
}

/// 解析一个十六进制字节，允许 `0x` 前缀。`raw` 子命令专用。
fn parse_hex_u8(s: &str, what: &str) -> Result<u8, String> {
    let h = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    u8::from_str_radix(h, 16).map_err(|_| format!("{what} 字节 {s:?} 不是合法十六进制（00-FF）"))
}

/// 把 [`ops::OpsError`] 转成给终端看的字符串，并附上类别提示。
///
/// 提示按类别分开：参数非法 = 改输入（且**设备根本没被碰过**）；
/// 休眠 = 动一下鼠标。两种都笼统报「失败」只会让用户瞎试。
fn fail(e: ops::OpsError) -> String {
    if e.is_invalid() {
        return format!(
            "{e}\n提示：{}（本次未向设备发送任何帧）",
            cli::error_hint(e.kind())
        );
    }
    if e.is_asleep() {
        // 休眠不是错误，不该让人以为程序坏了
        return format!("{e}\n提示：{}", cli::error_hint(e.kind()));
    }
    format!("{e}\n提示：{}", cli::error_hint(e.kind()))
}

/// `cache` —— 查看 / 清除快照缓存。不需要打开设备。
fn cmd_cache(pos: &[&str]) -> Result<(), String> {
    let file = paths::snapshot_cache_file();
    match pos.get(1).copied() {
        Some("clear") => {
            match std::fs::remove_file(&file) {
                Ok(()) => println!("已删除缓存 {}", file.display()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    println!("缓存本来就不存在（{}）", file.display());
                }
                Err(e) => return Err(format!("删除 {} 失败：{e}", file.display())),
            }
            Ok(())
        }
        None => {
            println!("缓存文件    {}", file.display());
            match std::fs::read_to_string(&file) {
                Ok(text) => {
                    // 只解析头部，不重新解析全部帧
                    let mut wall = 0u64;
                    let mut frames = 0usize;
                    for line in text.lines() {
                        if let Some(v) = line.strip_prefix("taken_unix=") {
                            wall = v.trim().parse().unwrap_or(0);
                        } else if line.starts_with("frame ") {
                            frames += 1;
                        }
                    }
                    let age = crate::snapshot::now_unix().saturating_sub(wall);
                    println!("帧数        {frames} 帧");
                    println!("年龄        {age} 秒前");
                    let ttl = settings::load().cache_ttl_sec;
                    println!(
                        "有效期      {ttl} 秒 → {}",
                        if age < ttl { "仍然有效" } else { "已过期" }
                    );
                }
                Err(_) => println!("（还没有缓存，下次 info 会生成）"),
            }
            println!("配置目录    {}", paths::config_dir().display());
            println!("备份目录    {}", paths::backup_dir().display());
            Ok(())
        }
        Some(other) => Err(format!(
            "cache 只接受 clear，收到 {other:?}\n\n用法：rog-mouse-ctl cache [clear]"
        )),
    }
}

/// 参数校验。放在打开设备之前，拼错的命令不会碰到硬件。
fn validate(cmd: &str, pos: &[&str]) -> Result<(), String> {
    let need = |n: usize| -> Result<(), String> {
        if pos.len() < n {
            return Err(format!(
                "{cmd} 需要 {n} 个参数，收到 {}\n\n{USAGE}",
                pos.len()
            ));
        }
        Ok(())
    };

    match cmd {
        "info" | "save" | "estimate" | "quick" => Ok(()),
        "backup" => Ok(()),                    // 文件可选，默认落到备份目录
        "restore" => need(2),
        "raw" => {
            need(3)?;
            parse_hex_u8(pos[1], "cmd")?;
            parse_hex_u8(pos[2], "sub")?;
            // 第 3 个字节可以没有（很多命令 tail 为空）
            for s in &pos[3..] {
                parse_hex_u8(s, "tail")?;
            }
            Ok(())
        }
        "setdpi" => {
            need(3)?;
            let stage: usize = pos[1]
                .parse()
                .map_err(|_| format!("档位必须是 1-4，收到 {:?}", pos[1]))?;
            if !(1..=4).contains(&stage) {
                return Err(format!("档位必须是 1-4，收到 {stage}"));
            }
            let dpi: i32 = pos[2]
                .parse()
                .map_err(|_| format!("DPI 必须是整数，收到 {:?}", pos[2]))?;
            // 型号规格：100..42000，步进 50
            if !(100..=42000).contains(&dpi) {
                return Err(format!("DPI 范围 100-42000，收到 {dpi}"));
            }
            Ok(())
        }
        "setpoll" => {
            need(2)?;
            let hz: i32 = pos[1]
                .parse()
                .map_err(|_| format!("回报率必须是整数，收到 {:?}", pos[1]))?;
            if proto::poll_code_of(hz).is_none() {
                return Err(format!("回报率只支持 125/250/500/1000，收到 {hz}"));
            }
            Ok(())
        }
        "setstage" => {
            need(2)?;
            let stage: u8 = pos[1]
                .parse()
                .map_err(|_| format!("档位必须是 1-4，收到 {:?}", pos[1]))?;
            if !(1..=4).contains(&stage) {
                return Err(format!("档位必须是 1-4，收到 {stage}"));
            }
            Ok(())
        }
        "setdebounce" => {
            need(2)?;
            let code: u8 = pos[1]
                .parse()
                .map_err(|_| format!("去抖代码必须是 0-7，收到 {:?}", pos[1]))?;
            if proto::debounce_ms(code).is_none() {
                return Err(format!("去抖代码必须是 0-7，收到 {code}"));
            }
            Ok(())
        }
        "setsnap" => {
            need(2)?;
            let v: u8 = pos[1]
                .parse()
                .map_err(|_| format!("角度吸附必须是 0 或 1，收到 {:?}", pos[1]))?;
            if v > 1 {
                return Err(format!("角度吸附必须是 0 或 1，收到 {v}"));
            }
            Ok(())
        }
        "settuning" => {
            need(2)?;
            let v: i16 = pos[1]
                .parse()
                .map_err(|_| format!("角度微调必须是整数，收到 {:?}", pos[1]))?;
            if !(-20..=20).contains(&v) {
                return Err(format!("角度微调范围 -20..20，收到 {v}"));
            }
            if v % 5 != 0 {
                return Err(format!("角度微调步进为 5，{v} 不是 5 的整数倍"));
            }
            Ok(())
        }
        "setlod" => {
            need(2)?;
            match pos[1].to_ascii_lowercase().as_str() {
                "low" | "high" => Ok(()),
                other => Err(format!("只接受 low / high，收到 {other:?}")),
            }
        }
        "setlight" => {
            need(2)?;
            Ok(())
        }
        "setbutton" => {
            need(3)?;
            let slot: usize = pos[1]
                .parse()
                .map_err(|_| format!("槽位必须是 0-7，收到 {:?}", pos[1]))?;
            if slot >= proto::BUTTON_SLOTS {
                return Err(format!(
                    "槽位必须是 0-{0}，收到 {slot}\n槽位含义：0 左键 / 1 右键 / 2 滚轮按下 / 3 后退 / 4 前进 / 5 DPI键 / 6 滚轮上 / 7 滚轮下",
                    proto::BUTTON_SLOTS - 1
                ));
            }
            Ok(())
        }
        "setsleep" => {
            need(2)?;
            let code = parse_u8_arg(pos[1], "休眠档位")?;
            if !proto::sleep_code_valid(code) {
                return Err(format!(
                    "休眠档位只接受 0=1分钟 1=2分钟 2=3分钟 3=5分钟 4=10分钟 255=永不，收到 {code}\n\
                     注意这是枚举值不是分钟数：设备对 5..254 不做校验会原样存下，但没有定义。"
                ));
            }
            Ok(())
        }
        "setlowbatt" => {
            need(2)?;
            let pct = parse_u8_arg(pos[1], "低电量阈值")?;
            if pct > 100 {
                return Err(format!("低电量阈值范围 0-100（百分比），收到 {pct}"));
            }
            Ok(())
        }
        "setfulleffect" => {
            need(2)?;
            let on = parse_u8_arg(pos[1], "充满效果开关")?;
            if on > 1 {
                return Err(format!(
                    "充满效果只接受 0（关）或 1（开），收到 {on}\n\
                     设备会把任何 ≥2 的值归一成 1，所以这里直接拒绝。"
                ));
            }
            Ok(())
        }
        other => Err(format!("未知子命令 {other:?}\n\n{USAGE}")),
    }
}
