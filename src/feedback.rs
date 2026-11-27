//! 适配报告使用白名单字段重新生成，不复制用户日志、路径或配置。
use std::fmt::Write;
pub mod client;
pub mod software;
pub mod inventory;

pub fn with_inventory(summary: &str, inventory: &str) -> String {
    const LIMIT: usize = 20 * 1024;
    const TRUNCATED: &str = "\nDiagnostic summary truncated to fit the report.\n";
    let available = LIMIT.saturating_sub(inventory.len() + TRUNCATED.len() + 1);
    let mut output = String::new();
    for line in summary.split_inclusive('\n') {
        if output.len() + line.len() > available {
            output.push_str(TRUNCATED);
            break;
        }
        output.push_str(line);
    }
    output.push('\n');
    output.push_str(inventory);
    output
}

#[derive(Clone)]
pub struct Candidate {
    pub interface: Option<crate::hid::HidInterface>,
    pub capture_interface: Option<crate::hid::HidInterface>,
    /// Display-only name; the report uses the original device name below.
    pub label: String,
    pub device: client::Device,
    pub battery: String,
    pub diagnostics: String,
}

pub fn candidate(label: String, reading: &crate::device::DeviceReading,
    interface: Option<&crate::hid::HidInterface>, backend: Option<crate::backends::BackendId>,
    address: Option<u8>, connection: &str, family: crate::device::DeviceFamily,
    measured: Option<std::time::Instant>, lifetime: std::time::Duration) -> Candidate {
    let family = family_code(family);
    let name = clean(&reading.name);
    let mut diagnostics = format!("App: X-Battery {}\nSystem: {}\nModel: {}\nType: {}\nConnection: {}\nBattery: {}\nReading: {:?}\nConnected: {}\n",
        crate::gui::APP_VERSION, system_version(), name, family, clean(connection),
        reading.battery_text(true), reading.quality, reading.connected);
    if let Some(interface) = interface {
        let _ = writeln!(diagnostics, "VID/PID: {:04X}:{:04X}\nUsage: {:04X}:{:04X}\nReport bytes (input/output/feature): {}/{}/{}",
            interface.vendor_id, interface.product_id, interface.usage_page, interface.usage,
            interface.input_len, interface.output_len, interface.feature_len);
        if let Some(backend) = backend {
            let _ = writeln!(diagnostics, "Provider: {}", backend.name());
            diagnostics.push_str(&crate::backends::trace::render_selected(backend, interface, address));
        }
    }
    diagnostics.push_str(&crate::diagnostics::readings::report(reading, measured, lifetime));
    Candidate { label, battery: reading.battery_text(true), diagnostics,
        interface: interface.cloned(),
        capture_interface: interface.filter(|interface| crate::capture::eligible(interface)).cloned(),
        device: client::Device { name, family: family.into(), connection: connection.into(), brand: None, model: None } }
}

fn family_code(family: crate::device::DeviceFamily) -> &'static str {
    use crate::device::DeviceFamily::*;
    match family { Mouse => "mouse", Keyboard => "keyboard", Headset => "headset", Gamepad => "gamepad", Unknown => "other" }
}

/// Only the OS version is read; no computer, account or installation identifier.
pub fn system_version() -> String {
    #[repr(C)]
    struct Version { size: u32, major: u32, minor: u32, build: u32, platform: u32, service: [u16; 128] }
    unsafe {
        let module = crate::win32::GetModuleHandleW(crate::win32::to_wide("ntdll.dll").as_ptr());
        let address = crate::win32::GetProcAddress(module, b"RtlGetVersion\0".as_ptr() as _);
        if address.is_null() { return "Windows".into(); }
        let get: unsafe extern "system" fn(*mut Version) -> i32 = std::mem::transmute(address);
        let mut version: Version = std::mem::zeroed();
        version.size = std::mem::size_of::<Version>() as u32;
        if get(&mut version) != 0 { return "Windows".into(); }
        format!("Windows {}.{} build {}", version.major, version.minor, version.build)
    }
}

fn clean(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(96)
        .collect()
}

pub fn report(interfaces: &[crate::hid::HidInterface]) -> String {
    let mut report = format!(
        "X-Battery device compatibility report\nApp version: {}\n\n",
        crate::gui::APP_VERSION
    );
    let mut seen = std::collections::BTreeSet::new();
    for interface in interfaces {
        let identity = (
            interface.vendor_id,
            interface.product_id,
            interface.usage_page,
            interface.usage,
            interface.input_len,
            interface.output_len,
            interface.feature_len,
            interface.product.clone(),
        );
        if !seen.insert(identity) {
            continue;
        }
        let _ = writeln!(report, "Model: {}", clean(&interface.product));
        let _ = writeln!(
            report,
            "VID/PID: {:04X}:{:04X}",
            interface.vendor_id, interface.product_id
        );
        let _ = writeln!(
            report,
            "Usage page/usage: {:04X}:{:04X}",
            interface.usage_page, interface.usage
        );
        let _ = writeln!(
            report,
            "Input/output/feature bytes: {}/{}/{}\n",
            interface.input_len, interface.output_len, interface.feature_len
        );
        if seen.len() >= 64 {
            break;
        }
    }
    report
}

pub fn collect() -> Result<String, String> {
    let api = crate::win32::HidApi::load()?;
    let mut output = report(&crate::hid::enumerate_all_report(&api)?.interfaces);
    output.push_str(&super::backends::trace::render());
    Ok(output)
}

/// 独立诊断进程先查询再导出，摘要与读数来自同一次运行。
pub fn collect_probe() -> Result<String, String> {
    let api = crate::win32::HidApi::load()?;
    let interfaces = crate::hid::enumerate_all_report(&api)?.interfaces;
    let mut output = report(&interfaces);
    output.push_str("Read-only capability probe (no settings are changed)\n");
    for backend in std::iter::once(super::backends::BackendId::Rog)
        .chain(super::backends::ADDITIONAL_BACKENDS) {
        if backend == super::backends::BackendId::XInput {
            match super::backends::xinput::poll() {
                Ok(poll) if poll.snapshots.is_empty() => {
                    let outcome = if poll.present_channels.is_some_and(|channels| channels.is_empty()) {
                        "no-connected-slot"
                    } else { "query-failed" };
                    let _ = writeln!(output, "XInput: {outcome}");
                }
                Ok(poll) => for snapshot in poll.snapshots {
                    append_probe_result(&mut output, &snapshot.target, Some(&snapshot));
                },
                Err(_) => { let _ = writeln!(output, "XInput: query-failed"); }
            }
            continue;
        }
        if backend == super::backends::BackendId::Bluetooth {
            match super::backends::bluetooth::poll() {
                Ok(poll) if poll.snapshots.is_empty() => {
                    let outcome = if poll.present_channels.is_some_and(|channels| channels.is_empty()) {
                        "no-connected-device"
                    } else { "no-cached-battery" };
                    let _ = writeln!(output, "Bluetooth: {outcome}");
                }
                Ok(poll) => for snapshot in poll.snapshots {
                    append_probe_result(&mut output, &snapshot.target, Some(&snapshot));
                },
                Err(_) => { let _ = writeln!(output, "Bluetooth: query-failed"); }
            }
            continue;
        }
        let provider: &dyn super::backends::DeviceBackend = match backend {
            super::backends::BackendId::Logitech => &super::backends::logitech::Logitech,
            super::backends::BackendId::LogitechHeadset => {
                &super::backends::logitech_headset::LogitechHeadset
            }
            super::backends::BackendId::Rapoo => &super::backends::rapoo::Rapoo,
            super::backends::BackendId::RapooLegacy => &super::backends::rapoo_legacy::RapooLegacy,
            super::backends::BackendId::RapooOffice => &super::backends::rapoo_office::RapooOffice,
            super::backends::BackendId::RapooMt560 => &super::backends::rapoo_mt560::RapooMt560,
            super::backends::BackendId::Vgn => &super::backends::vgn::Vgn,
            super::backends::BackendId::VgnBeiying => &super::backends::vgn_beiying::VgnBeiying,
            super::backends::BackendId::VgnDragonfly3 => &super::backends::vgn_dragonfly3::VgnDragonfly3,
            super::backends::BackendId::VgnRy => &super::backends::vgn_ry::VgnRy,
            super::backends::BackendId::VgnKeyboard => &super::backends::vgn_keyboard::VgnKeyboard,
            super::backends::BackendId::CompX => &super::backends::compx::CompX,
            super::backends::BackendId::Razer => &super::backends::razer::Razer,
            super::backends::BackendId::PageCommand => &super::backends::page_command::PageCommand,
            super::backends::BackendId::Headset => &super::backends::headset::Headset,
            super::backends::BackendId::Keyboard20 => &super::backends::keyboard20::Keyboard20,
            super::backends::BackendId::MchoseQhw => &super::backends::mchose_qhw::MchoseQhw,
            super::backends::BackendId::MchoseMouse => &super::backends::mchose_mouse::MchoseMouse,
            super::backends::BackendId::MchoseModern => &super::backends::mchose_modern::MchoseModern,
            super::backends::BackendId::MchoseGlw => &super::backends::mchose_glw::MchoseGlw,
            super::backends::BackendId::MchoseK99 => &super::backends::mchose_k99::MchoseK99,
            super::backends::BackendId::MchoseK87s => &super::backends::mchose_k87s::MchoseK87s,
            super::backends::BackendId::MchoseMc => &super::backends::mchose_mc::MchoseMc,
            super::backends::BackendId::MchoseR9 => &super::backends::mchose_r9::MchoseR9,
            super::backends::BackendId::Moondrop => &super::backends::moondrop::Moondrop,
            super::backends::BackendId::Edifier => &super::backends::edifier::Edifier,
            super::backends::BackendId::EweadnS9 => &super::backends::eweadn::EweadnS9,
            super::backends::BackendId::EweadnS9Pro => &super::backends::eweadn_s9pro::EweadnS9Pro,
            super::backends::BackendId::EweadnX23 => &super::backends::eweadn_x23::EweadnX23,
            super::backends::BackendId::EweadnX87 => &super::backends::eweadn_x87::EweadnX87,
            super::backends::BackendId::Dareu => &super::backends::dareu::Dareu,
            super::backends::BackendId::DareuEm901 => &super::backends::dareu_em901::DareuEm901,
            super::backends::BackendId::EightBitDo => &super::backends::eightbitdo::EightBitDo,
            super::backends::BackendId::Flydigi => &super::backends::flydigi::Flydigi,
            super::backends::BackendId::GameSir => &super::backends::gamesir::GameSir,
            super::backends::BackendId::GearHub => &super::backends::gearhub::GearHub,
            super::backends::BackendId::G3m => &super::backends::g3m::G3m,
            super::backends::BackendId::AjazzAk980 => &super::backends::ajazz_ak980::AjazzAk980,
            super::backends::BackendId::VeekosShine60 => &super::backends::veekos::VeekosShine60,
            super::backends::BackendId::Aula => &super::backends::aula::Aula,
            super::backends::BackendId::AulaHfd => &super::backends::aula_hfd::AulaHfd,
            super::backends::BackendId::AulaHfdMechanical => &super::backends::aula_hfd_mechanical::AulaHfdMechanical,
            super::backends::BackendId::AulaV9Pro => &super::backends::aula_v9pro::AulaV9Pro,
            super::backends::BackendId::AulaF99 => &super::backends::aula_f99::AulaF99,
            super::backends::BackendId::AulaF108 => &super::backends::aula_f108::AulaF108,
            super::backends::BackendId::AtkNano => &super::backends::atk_nano::AtkNano,
            super::backends::BackendId::AtkHoltek => &super::backends::atk_holtek::AtkHoltek,
            super::backends::BackendId::Lingbao => &super::backends::lingbao::Lingbao,
            super::backends::BackendId::Xiberia => &super::backends::xiberia::Xiberia,
            super::backends::BackendId::FifineHt6 => &super::backends::fifine_ht6::FifineHt6,
            super::backends::BackendId::Thunderobot => &super::backends::thunderobot::Thunderobot,
            super::backends::BackendId::Akg => &super::backends::akg::Akg,
            super::backends::BackendId::LangtuT98 => &super::backends::langtu::LangtuT98,
            super::backends::BackendId::Cherry => &super::backends::cherry::Cherry,
            super::backends::BackendId::Lofree => &super::backends::lofree::Lofree,
            super::backends::BackendId::Nova => &super::backends::steelseries::Nova,
            super::backends::BackendId::Inzone => &super::backends::inzone::Inzone,
            super::backends::BackendId::HyperX => &super::backends::hyperx::HyperX,
            super::backends::BackendId::DualSense => &super::backends::sony::DualSense,
            super::backends::BackendId::XInput => unreachable!("XInput is queried without HID"),
            super::backends::BackendId::Bluetooth => unreachable!("Bluetooth is queried without HID"),
            super::backends::BackendId::StandardHid => &super::backends::standard::StandardHid,
            super::backends::BackendId::Rog => &super::backends::rog::Rog,
        };
        let Ok(targets) = provider.discover(&api, &interfaces) else {
            let _ = writeln!(output, "{}: discovery-failed", backend.name());
            continue;
        };
        if targets.is_empty() {
            let _ = writeln!(output, "{}: no-supported-interface", backend.name());
        }
        for target in targets.into_iter().take(64) {
            let reading = provider.read(&api, &target, true);
            append_probe_result(&mut output, &target, reading.as_ref().ok());
        }
    }
    output.push('\n');
    output.push_str(&super::backends::trace::render());
    Ok(output)
}

fn append_probe_result(output: &mut String, target: &super::backends::Target,
    reading: Option<&super::backends::Snapshot>) {
    let source = match &target.endpoint {
        super::backends::Endpoint::Hid(i) => format!("{:04X}:{:04X}", i.vendor_id, i.product_id),
        super::backends::Endpoint::XInput(slot) => format!("slot={}", slot + 1),
    };
    let _ = writeln!(output, "{} {} result={}", target.backend.name(), source,
        match reading {
            Some(reading) if reading.connected && (reading.battery.is_some() || reading.dpi.current.is_some()
                || reading.polling_rate.current.is_some()) => "ok",
            Some(_) => "no-valid-reading",
            None => "read-failed",
        });
    if let Some(reading) = reading {
        let percent = reading.battery.as_ref().and_then(|battery| battery.display_percent())
            .map(|percent| percent.label(true)).unwrap_or_else(|| "unknown".into());
        let charging = match reading.battery.as_ref().and_then(|battery| battery.charging) {
            Some(true) => "yes", Some(false) => "no", None => "unknown",
        };
        let dpi = reading.dpi.current.map(|dpi| dpi.to_string()).unwrap_or_else(|| "unknown".into());
        let _ = writeln!(output, "Battery: {percent}; charging: {charging}; current DPI: {dpi}; DPI writable: {}",
            reading.dpi.writable);
        if let Some(battery) = &reading.battery {
            if let Some(level) = battery.level {
                let _ = writeln!(output, "Battery level: {level:?}; approximate: {}", battery.approximate);
            }
        }
        if let Some(y) = reading.dpi.current_y { let _ = writeln!(output, "Current DPI Y: {y}"); }
        let rate = reading.polling_rate.current.map(|hz| hz.to_string()).unwrap_or_else(|| "unknown".into());
        let _ = writeln!(output, "Polling rate: {rate} Hz; choices: {:?}; writable: {}",
            reading.polling_rate.values, reading.polling_rate.writable);
        if !reading.dpi.stages.is_empty() {
            let stages = reading.dpi.stages.iter().map(|stage| match reading.dpi.stage_y.get(&stage.index) {
                Some(y) => format!("{}:X{}/Y{y}",stage.index + 1,stage.value),
                None => format!("{}:{}",stage.index + 1,stage.value),
            })
                .collect::<Vec<_>>().join(", ");
            let active = reading.dpi.active_stage.map(|index| (index + 1).to_string()).unwrap_or_else(|| "unknown".into());
            let _ = writeln!(output, "Onboard DPI stages: {stages}; active stage: {active}; stage switching: {}",
                reading.dpi.stage_writable);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_keep_source_interfaces_independently_of_capture_eligibility() {
        let mut interface = crate::hid::HidInterface {
            path: "private-source-path".into(), serial: "private-serial".into(),
            vendor_id: 0x1234, product_id: 0x5678, usage_page: 1, usage: 2,
            input_len: 8, output_len: 0, feature_len: 8, product: "Factory mouse".into(),
        };
        let reading = crate::device::DeviceReading { name: "Factory mouse".into(), ..Default::default() };
        let make = |interface: Option<&crate::hid::HidInterface>| candidate("private-alias".into(), &reading,
            interface, None, None, "unknown", crate::device::DeviceFamily::Mouse, None,
            std::time::Duration::from_secs(120));
        let standard = make(Some(&interface));
        assert_eq!(standard.interface.as_ref().unwrap().path, interface.path);
        assert!(standard.capture_interface.is_none());
        for private in [&interface.path, &interface.serial, &standard.label] {
            assert!(!standard.diagnostics.contains(private));
        }
        interface.usage_page = 0xff00;
        let vendor = make(Some(&interface));
        assert_eq!(vendor.capture_interface.as_ref().unwrap().path, interface.path);
        assert_eq!(vendor.interface.as_ref().unwrap().path, interface.path);
        let non_hid = make(None);
        assert!(non_hid.interface.is_none());
        assert!(non_hid.capture_interface.is_none());
    }

    #[test]
    fn report_budget_preserves_inventory_and_marks_truncated_history() {
        let inventory = format!("HID interface inventory\n{}", "VID/PID=24AE:1415\n".repeat(500));
        let summary = "battery state change\n".repeat(1500);
        let report = with_inventory(&summary, &inventory);
        assert!(report.len() <= 20 * 1024);
        assert!(report.contains("Diagnostic summary truncated"));
        assert!(report.ends_with(&inventory));
    }
    #[test]
    fn xinput_report_uses_slot_without_fabricated_usb_identity() {
        let target = crate::backends::xinput::target(1);
        let reading = crate::backends::Snapshot { target: target.clone(), battery: None,
            battery_at: None, battery_unavailable: true, connected: true,
            dpi: Default::default(), polling_rate: Default::default() };
        let mut output = String::new();
        append_probe_result(&mut output, &target, Some(&reading));
        assert!(output.contains("XInput slot=2 result=no-valid-reading"));
        assert!(output.contains("Battery: unknown; charging: unknown"));
        assert!(!output.contains("0000:0000"));
        assert!(!output.contains(&target.key));
    }

    #[test]
    fn exported_report_never_contains_paths_or_serial_numbers() {
        let interface = crate::hid::HidInterface {
            path: "sensitive-interface-path".into(),
            serial: "private-device-serial".into(),
            vendor_id: 0x046d,
            product_id: 0xc52b,
            usage_page: 0xff00,
            usage: 1,
            input_len: 20,
            output_len: 20,
            feature_len: 0,
            product: "Test mouse\n\r".into(),
        };
        let result = report(&[interface]);
        assert!(!result.contains("sensitive-interface-path"));
        assert!(!result.contains("private-device-serial"));
        assert!(result.contains("046D:C52B"));
        assert!(result.contains("Model: Test mouse\n"));
    }

    #[test]
    fn probe_results_export_only_readings_and_protocol_identifiers() {
        let interface = crate::hid::HidInterface { path: "private-path".into(),
            serial: "private-serial".into(), product: "Factory model".into(),
            vendor_id: 0x258a, product_id: 0x024e, usage_page: 0xff02, usage: 1,
            input_len: 0, output_len: 0, feature_len: 520 };
        let mut target = super::super::backends::Target::new(super::super::backends::BackendId::Vgn, &interface, 0);
        target.name = "private-custom-name".into();
        let snapshot = super::super::backends::Snapshot { target: target.clone(),
            battery: Some(crate::device::Battery { estimated_percent: None, percent: Some(75), charging: None,
                voltage_mv: None, approximate: false, level: None }),
            dpi: crate::device::Dpi { current: Some(800),
                stages: vec![crate::device::DpiStage { index: 1, value: 800 },
                    crate::device::DpiStage { index: 4, value: 3200 }],
                active_stage: Some(1), profile: Some(1), stage_writable: true,
                ..Default::default() }, polling_rate: Default::default(), connected: true, battery_unavailable: false, battery_at: Some(std::time::Instant::now()) };
        let mut output = String::new();
        append_probe_result(&mut output, &target, Some(&snapshot));
        assert!(output.contains("Battery: 75%; charging: unknown"));
        assert!(output.contains("2:800, 5:3200"));
        for private in ["private-path", "private-serial", "private-custom-name", &target.key] {
            assert!(!output.contains(private));
        }
        let coarse = super::super::backends::Snapshot {
            battery: Some(crate::device::Battery { estimated_percent: Some(50), percent: None, charging: Some(false),
                voltage_mv: None, approximate: true, level: Some(crate::device::BatteryLevel::Medium) }),
            ..snapshot.clone()
        };
        let mut output = String::new();
        append_probe_result(&mut output, &target, Some(&coarse));
        assert!(output.contains("Battery: ≈50%; charging: no"));
        assert!(output.contains("Battery level: Medium; approximate: true"));
        assert!(!output.contains("Battery: 50%"));
        let missing = super::super::backends::Snapshot { connected: false, battery: None, ..snapshot };
        let mut output = String::new();
        append_probe_result(&mut output, &target, Some(&missing));
        assert!(output.contains("result=no-valid-reading"));
    }
}
