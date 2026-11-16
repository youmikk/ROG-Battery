//! Serialize a complete protocol operation on one physical HID channel.
use std::{collections::BTreeMap, sync::{Arc, Mutex, OnceLock}};

pub fn lock(key: &str) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<BTreeMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner());
    locks.entry(key.to_ascii_lowercase()).or_default().clone()
}
