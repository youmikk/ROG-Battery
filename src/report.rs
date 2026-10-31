//! 状态 / 身份 / 原始帧的打印层。
//!
//! 协议层（[`crate::proto`]）只负责字节与语义，不该关心「怎么排版给终端看」。
//! 这里全是纯排版函数：输入是解析好的结构体，输出是 stdout。

use crate::proto::{
    debounce_name, lod_name, light_mode_name, poll_name, sleep_mode_name, Identity, State,
    DPI_STAGES,
};

pub fn hex_dump(bytes: &[u8]) {
    for (row, chunk) in bytes.chunks(16).enumerate() {
        let off = row * 16;
        print!("  {off:04X}  ");
        for i in 0..16 {
            match chunk.get(i) {
                Some(b) => print!("{b:02X} "),
                None => print!("   "),
            }
        }
        print!(" |");
        for &b in chunk {
            // 不可打印字符用 '.'，避免终端被控制字符打乱
            print!("{}", if (0x20..0x7F).contains(&b) { b as char } else { '.' });
        }
        println!("|");
    }
}

/// 打印完整状态。字段顺序刻意与 rw-hid.cs 一致，方便和既有实测记录对照。
pub fn print_state(st: &State) {
    println!(
        "  回报率      {} (code {})",
        poll_name(st.poll_code),
        st.poll_code
    );
    for i in 0..DPI_STAGES {
        println!(
            "  DPI 档 {}    {} (enc {})  颜色 {}",
            i + 1,
            st.dpi(i),
            st.enc[i],
            st.rgb_hex(i)
        );
    }
    println!(
        "  当前档位    {} (1-based，对应 {} DPI)",
        st.active_stage,
        if st.active_stage >= 1 && st.active_stage as usize <= DPI_STAGES {
            st.dpi(st.active_stage as usize - 1).to_string()
        } else {
            "?".into()
        }
    );
    println!(
        "  去抖        {} (code {})   角度吸附 {}   角度微调 {}",
        debounce_name(st.debounce_code),
        st.debounce_code,
        st.angle_snap,
        st.angle_tuning
    );
    println!("  抬起距离    {} (code {})", lod_name(st.lod), st.lod);
    println!(
        "  表面标定    类型 {} 材质 {} 附加 {:02X}  k值 {:02X} {:02X} {:02X}",
        st.surface_type, st.material, st.surface_extra, st.calib[0], st.calib[1], st.calib[2]
    );
    println!(
        "  灯光        {} (code {})  亮度 {}  颜色 {}  方向 {} 随机 {} 速度 {}",
        light_mode_name(st.light_mode),
        st.light_mode,
        st.light_bright,
        st.light_rgb_hex(),
        st.light_dir,
        st.light_random,
        st.light_speed
    );
    let btns = st
        .buttons
        .iter()
        .map(|b| format!("{b:04X}"))
        .collect::<Vec<_>>()
        .join(" ");
    println!("  按键绑定    {btns}");
    println!(
        "  能量块      休眠 {} 低电量阈值 {}% 充满效果 {}",
        sleep_mode_name(st.sleep_mode),
        st.low_warn,
        st.full_effect
    );
}

/// 打印电量与人话解释。
pub fn print_power(st: &State) {
    println!(
        "  电量        {}%   电压 {} mV   {}",
        st.power,
        st.voltage_mv,
        if st.charging == 1 { "充电中" } else { "使用中" }
    );
    println!(
        "  休眠        {} (code {})   低电量阈值 {}%   电池类型 {}   充满效果 {}",
        sleep_mode_name(st.sleep_mode),
        st.sleep_mode,
        st.low_warn,
        st.battery_type,
        st.full_effect
    );
}

pub fn print_identity(id: &Identity, mode: &str, rid: u8, product: &str, serial: &str) {
    println!("  链路        {mode} (report id 0x{rid:02X})");
    if !product.is_empty() {
        println!("  HID 产品名  {product}");
    }
    if !serial.is_empty() {
        // 到这里 `serial` 已经过 `hid.rs::read_hid_string` 清洗（只保留开头的
        // 字母数字串）。清洗原因是本机 OMNI 接收器的 HID 字符串描述符固件 bug，
        // 会在真实序列号后面带 2 个未初始化内存码点 —— 详见那个函数的注释。
        println!("  HID 序列号  {serial}");
    }
    if !id.model_name.is_empty() {
        println!("  型号名      {}", id.display_name());
        // 对照表里有统一译名时，把英文型号也印出来，方便对照厂商页面。
        if let Some(d) = [id.wired_pid, id.wireless_pid]
            .iter()
            .filter(|p| **p != 0)
            .find_map(|p| crate::devices::find(*p))
        {
            println!("  英文型号    {}", d.name_en);
            println!("  设备类型    {}", d.kind.as_str());
        }
        println!("  固件自报    {}", id.model_name);
    }
    if id.wired_pid != 0 {
        println!("  有线 PID    0x{:04X}", id.wired_pid);
    }
    if id.wireless_pid != 0 {
        println!("  无线 PID    0x{:04X}", id.wireless_pid);
    }
    if !id.mouse_sn.is_empty() {
        println!("  鼠标 SN     {}", id.mouse_sn);
    }
    if !id.dongle_sn.is_empty() {
        println!("  接收器 SN   {}", id.dongle_sn);
    }
    if !id.dongle_firmware.is_empty() {
        println!("  接收器固件  {}", id.dongle_firmware);
    }
    if !id.mouse_mac.is_empty() {
        println!("  鼠标端 MAC  {}", id.mouse_mac);
    }
    if !id.dongle_mac.is_empty() {
        println!("  接收器 MAC  {}", id.dongle_mac);
    }
    println!(
        "  高回报率    {}",
        if id.booster {
            "支持 2000Hz 以上"
        } else {
            "不支持（仅 125/250/500/1000）"
        }
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 打印层最容易出的问题是「空字段 / 全零状态」时 panic 或除零。
    /// 这几个函数是命令行的唯一出口，崩了等于工具不可用。
    #[test]
    fn printers_survive_empty_state() {
        let st = State::default();
        print_state(&st);
        print_power(&st);
        print_identity(&Identity::default(), "Wired", 0, "", "");
    }

    #[test]
    fn hex_dump_handles_empty_and_short_input() {
        hex_dump(&[]);
        hex_dump(&[0x00]);
        hex_dump(&[0x12, 0x07, 0x7F, 0x20, 0xFF]); // 可打印、边界、不可打印混在一起
    }
}
