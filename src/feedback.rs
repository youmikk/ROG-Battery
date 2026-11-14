//! 适配报告使用白名单字段重新生成，不复制用户日志、路径或配置。
use std::fmt::Write;

fn clean(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(96)
        .collect()
}

pub fn report(interfaces: &[crate::hid::HidInterface]) -> String {
    let mut report = format!(
        "R Battery device compatibility report\nApp version: {}\n\n",
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

pub fn issue_url() -> String {
    // 开发分支的模板尚未合入默认分支，直接预填正文也能提交。
    "https://github.com/youmikk/ROG-Battery/issues/new?title=%5BDevice%5D&body=Brand%20%2F%20Model%3A%0AConnection%20%28USB%2C%202.4GHz%2C%20Bluetooth%29%3A%0AApp%20version%3A%0ABattery%20%2F%20DPI%20behavior%3A%0A%0AAttach%20the%20device%20report%20exported%20from%20Settings%20%E2%86%92%20About.%0A%E8%AF%B7%E5%A1%AB%E5%86%99%E5%93%81%E7%89%8C%E3%80%81%E5%9E%8B%E5%8F%B7%E3%80%81%E8%BF%9E%E6%8E%A5%E6%96%B9%E5%BC%8F%E5%92%8C%E9%97%AE%E9%A2%98%E8%A1%A8%E7%8E%B0%E3%80%82".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
