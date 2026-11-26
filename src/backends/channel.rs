//! Serialize a complete protocol operation on one physical HID channel.
use std::{collections::BTreeMap, sync::{Arc, Mutex, OnceLock}};

pub fn lock(key: &str) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<BTreeMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner());
    locks.entry(key.to_ascii_lowercase()).or_default().clone()
}

/// Preserve the USB interface and Windows instance while joining sibling HID collections.
pub fn collection_group(interface: &crate::hid::HidInterface) -> String {
    let path = interface.path.to_ascii_lowercase();
    let mut parts: Vec<String> = path.split('#').map(str::to_string).collect();
    if parts.len() < 4 { return path; }
    let collection = parts[1].split('&').find_map(|p| p.strip_prefix("col"))
        .filter(|s| s.len() == 2 && s.bytes().all(|b| b.is_ascii_hexdigit()));
    if let Some(collection) = collection {
        // Windows HID PDO 实例后缀从 &0000 起编址（即 col09 -> &0008），同时兼容测试用例的同号后缀。
        let one_based = format!("&00{collection}");
        let zero_based = u8::from_str_radix(collection, 16)
            .ok()
            .filter(|&index| index > 0)
            .map(|index| format!("&{:04x}", index - 1));
        let suffix_len = if parts[2].ends_with(&one_based) {
            Some(one_based.len())
        } else {
            zero_based
                .as_ref()
                .filter(|suffix| parts[2].ends_with(suffix.as_str()))
                .map(String::len)
        };
        if let Some(suffix_len) = suffix_len {
            let length = parts[2].len() - suffix_len;
            parts[2].truncate(length);
        }
        parts[1] = parts[1].split('&').filter(|p| !p.starts_with("col")).collect::<Vec<_>>().join("&");
    }
    parts.join("#")
}

#[cfg(test)]
mod tests {
    use super::collection_group;
    use crate::hid::HidInterface;

    fn iface(path: &str) -> HidInterface {
        HidInterface {
            path: path.into(), serial: String::new(), product: String::new(),
            vendor_id: 0x24ae, product_id: 0x1415, usage_page: 0xff00, usage: 2,
            input_len: 19, output_len: 0, feature_len: 0,
        }
    }

    #[test]
    fn groups_both_zero_based_windows_and_one_based_collection_suffixes() {
        let win_a = iface(r"\\?\hid#vid_24ae&pid_1415&mi_01&col05#a&135a9884&0&0004#{4d1e55b2-f16f-11cf-88cb-001111000030}");
        let win_b = iface(r"\\?\hid#vid_24ae&pid_1415&mi_01&col09#a&135a9884&0&0008#{4d1e55b2-f16f-11cf-88cb-001111000030}");
        let win_other = iface(r"\\?\hid#vid_24ae&pid_1415&mi_01&col09#a&54477e4&0&0008#{4d1e55b2-f16f-11cf-88cb-001111000030}");
        assert_eq!(collection_group(&win_a), collection_group(&win_b));
        assert_ne!(collection_group(&win_a), collection_group(&win_other));

        let syn_a = iface("hid#vid_1532&pid_007b&mi_00&col01#7&aaaa&0&0001#{hid}");
        let syn_b = iface("hid#vid_1532&pid_007b&mi_00&col02#7&aaaa&0&0002#{hid}");
        assert_eq!(collection_group(&syn_a), collection_group(&syn_b));
    }
}
