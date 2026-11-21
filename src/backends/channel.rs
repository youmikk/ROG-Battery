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
        let suffix = format!("&00{collection}");
        if parts[2].ends_with(&suffix) {
            let length=parts[2].len() - suffix.len();
            parts[2].truncate(length);
        }
        parts[1] = parts[1].split('&').filter(|p| !p.starts_with("col")).collect::<Vec<_>>().join("&");
    }
    parts.join("#")
}
