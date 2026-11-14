//! 品牌适配层。未知能力保留为空，不能从设备类型推断写入协议。
pub mod logitech;
pub mod rog;
pub mod standard;

use crate::{device::DeviceFamily, hid::HidInterface, win32::HidApi};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendId {
    Rog,
    Logitech,
    StandardHid,
}

impl BackendId {
    pub fn name(self) -> &'static str {
        match self {
            Self::Rog => "ROG",
            Self::Logitech => "Logitech HID++",
            Self::StandardHid => "Standard HID",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Battery {
    pub percent: Option<u8>,
    pub charging: Option<bool>,
    pub voltage_mv: Option<u16>,
    pub approximate: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dpi {
    pub current: Option<u16>,
    pub values: Vec<u16>,
    pub writable: bool,
}

impl Dpi {
    /// 快捷值来自设备能力，当前值始终保留。
    pub fn quick_values(&self) -> Vec<u16> {
        let mut values = Vec::new();
        if let Some(current) = self.current {
            values.push(current);
        }
        for value in [400, 800, 1600, 3200] {
            if self.values.contains(&value) && !values.contains(&value) {
                values.push(value);
            }
        }
        values.truncate(4);
        values.sort_unstable();
        values
    }
}

#[derive(Clone, Debug)]
pub struct Target {
    pub key: String,
    pub name: String,
    pub family: DeviceFamily,
    pub backend: BackendId,
    pub interface: HidInterface,
    /// 多设备接收器内的地址，与 report ID 分开。
    pub address: u8,
    pub verified_identity: bool,
}

impl Target {
    pub fn new(backend: BackendId, interface: &HidInterface, address: u8) -> Self {
        let identity = if interface.serial.is_empty() {
            interface.path.to_ascii_lowercase()
        } else {
            format!(
                "{:04x}:{:04x}:{}",
                interface.vendor_id, interface.product_id, interface.serial
            )
        };
        Self {
            key: crate::device::stable_key(&format!("{}:{identity}:{address}", backend.name())),
            name: interface.product.clone(),
            family: match (interface.usage_page, interface.usage) {
                (1, 2) => DeviceFamily::Mouse,
                (1, 6) => DeviceFamily::Keyboard,
                (1, 4 | 5) => DeviceFamily::Gamepad,
                _ => crate::device::family_of(&interface.product),
            },
            backend,
            interface: interface.clone(),
            address,
            verified_identity: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub target: Target,
    pub battery: Option<Battery>,
    pub dpi: Dpi,
    pub connected: bool,
}

pub trait DeviceBackend {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String>;
    fn read(&self, api: &HidApi, target: &Target, detail: bool) -> Result<Snapshot, String>;
    fn set_dpi(&self, _api: &HidApi, _target: &Target, _dpi: u16) -> Result<(), String> {
        Err("设备不支持 DPI 写入".into())
    }
}

/// 枚举/读取失败仅影响当前适配器，不能阻断其他品牌。
pub fn read_additional(detail: bool) -> Result<Vec<Snapshot>, String> {
    let api = HidApi::load()?;
    let interfaces = crate::hid::enumerate_all_report(&api)?.interfaces;
    let providers: [&dyn DeviceBackend; 2] = [&logitech::Logitech, &standard::StandardHid];
    let mut snapshots = Vec::new();
    let mut claimed = std::collections::BTreeSet::new();
    for provider in providers {
        let Ok(targets) = provider.discover(&api, &interfaces) else {
            continue;
        };
        for target in targets {
            if target.backend != BackendId::Logitech && claimed.contains(&target.interface.path) {
                continue;
            }
            match provider.read(&api, &target, detail) {
                Ok(snapshot) => {
                    if snapshot
                        .battery
                        .as_ref()
                        .is_some_and(|b| b.percent.is_some())
                    {
                        if snapshot.target.backend == BackendId::Logitech {
                            claimed.insert(snapshot.target.interface.path.clone());
                        }
                        snapshots.push(snapshot);
                    }
                }
                Err(_) => continue,
            }
        }
    }
    Ok(snapshots)
}

pub fn set_dpi(target: &Target, dpi: u16) -> Result<(), String> {
    let api = HidApi::load()?;
    match target.backend {
        BackendId::Logitech => logitech::Logitech.set_dpi(&api, target, dpi),
        _ => Err("此协议不提供通用 DPI 写入".into()),
    }
}

pub fn read_all(detail: bool) -> Result<Vec<Snapshot>, String> {
    let api = HidApi::load()?;
    let mut snapshots = Vec::new();
    if let Ok(targets) = rog::Rog.discover(&api, &[]) {
        for target in targets {
            if let Ok(snapshot) = rog::Rog.read(&api, &target, detail) {
                snapshots.push(snapshot);
            }
        }
    }
    snapshots.extend(read_additional(detail)?);
    Ok(snapshots)
}
