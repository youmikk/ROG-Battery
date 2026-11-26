use windows::core::{HSTRING, Interface};
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

pub fn supported() -> bool {
    crate::autostart::is_packaged()
}

fn escape_xml(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            _ => output.push(character),
        }
    }
    output
}

fn toast_xml(device_name: &str, level: u8, threshold: u8) -> String {
    let name = escape_xml(device_name);
    if crate::i18n::english() {
        format!(
            "<toast><visual><binding template=\"ToastGeneric\"><text>Low battery</text><text>{name} is at {level}%, below the {threshold}% threshold. Please charge it soon.</text></binding></visual><audio silent=\"false\"/></toast>"
        )
    } else {
        format!(
            "<toast><visual><binding template=\"ToastGeneric\"><text>低电量提醒</text><text>{name} 电量仅剩 {level}%，已低于 {threshold}%，请及时充电。</text></binding></visual><audio silent=\"false\"/></toast>"
        )
    }
}

fn notification(device_key: &str, device_name: &str, level: u8, threshold: u8) -> windows::core::Result<ToastNotification> {
    let document: XmlDocument = unsafe {
        windows::Win32::System::WinRT::RoActivateInstance(&HSTRING::from("Windows.Data.Xml.Dom.XmlDocument"))
    }?.cast()?;
    document.LoadXml(&HSTRING::from(toast_xml(device_name, level, threshold)))?;
    let toast = ToastNotification::CreateToastNotification(&document)?;
    let tag: String = device_key.chars().take(16).collect();
    toast.SetTag(&HSTRING::from(tag))?;
    toast.SetGroup(&HSTRING::from("battery-low"))?;
    Ok(toast)
}

pub fn show_low_battery(device_key: &str, device_name: &str, level: u8, threshold: u8) -> Result<(), String> {
    if !supported() {
        return Err("当前非微软商店版，暂不支持触发 Windows 低电量系统通知".into());
    }
    let _apartment = crate::winrt::Apartment::open()?;
    let result = (|| -> windows::core::Result<()> {
        let toast = notification(device_key, device_name, level, threshold)?;
        ToastNotificationManager::CreateToastNotifier()?.Show(&toast)
    })();
    result.map_err(|error| format!("显示系统通知失败：0x{:08X}", error.code().0 as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_document_escapes_device_names_and_retains_notification_identity() {
        std::thread::spawn(|| {
            let _apartment = crate::winrt::Apartment::open().unwrap();
            let toast = notification("0123456789abcdefghijkl", "A<&\"'测试", 9, 20).unwrap();
            assert_eq!(toast.Tag().unwrap().to_string_lossy(), "0123456789abcdef");
            assert_eq!(toast.Group().unwrap().to_string_lossy(), "battery-low");
            let xml = toast.Content().unwrap().GetXml().unwrap().to_string_lossy();
            assert!(xml.contains("&lt;"));
            assert!(xml.contains("&amp;"));
            assert!(xml.contains("9"));
            assert!(xml.contains("20"));
        }).join().unwrap();
    }

    #[test]
    fn document_failure_releases_the_apartment_before_the_next_notification() {
        std::thread::spawn(|| {
            {
                let _apartment = crate::winrt::Apartment::open().unwrap();
                assert!(notification("key", "invalid\u{0}name", 9, 20).is_err());
            }
            let _apartment = crate::winrt::Apartment::open().unwrap();
            assert!(notification("key", "valid", 9, 20).is_ok());
        }).join().unwrap();
    }
}
