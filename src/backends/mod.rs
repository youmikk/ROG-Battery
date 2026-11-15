//! 品牌适配层。未知能力保留为空，不能从设备类型推断写入协议。
pub mod logitech;
mod feature;
pub mod rapoo;
pub mod rog;
pub mod standard;
pub mod trace;
pub mod vgn;

pub use crate::device::{Battery, BatteryLevel, Dpi};
use crate::{device::DeviceFamily, hid::HidInterface, win32::HidApi};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BackendId {
    Rog,
    Logitech,
    Rapoo,
    Vgn,
    StandardHid,
}

pub const ADDITIONAL_BACKENDS: [BackendId; 4] = [
    BackendId::Logitech, BackendId::Rapoo, BackendId::Vgn, BackendId::StandardHid,
];

impl BackendId {
    pub fn name(self) -> &'static str {
        match self {
            Self::Rog => "ROG",
            Self::Logitech => "Logitech HID++",
            Self::Rapoo => "Rapoo nRF54L",
            Self::Vgn => "VGN V98 Pro V3",
            Self::StandardHid => "Standard HID",
        }
    }

    pub fn precedes_standard(self) -> bool {
        matches!(self, Self::Logitech | Self::Rapoo | Self::Vgn)
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
    merge_additional_results(
        ADDITIONAL_BACKENDS
            .into_iter()
            .map(|backend| (backend, read_backend(backend, detail))),
    )
}

fn merge_additional_results(
    results: impl IntoIterator<Item = (BackendId, Result<Vec<Snapshot>, String>)>,
) -> Result<Vec<Snapshot>, String> {
    let mut snapshots = Vec::new();
    let mut claimed = std::collections::BTreeSet::new();
    let mut failures = Vec::new();
    let mut successful_backends = 0;
    for (backend, result) in results {
        let devices = match result {
            Ok(devices) => {
                successful_backends += 1;
                devices
            }
            Err(error) => {
                failures.push(format!("{}: {error}", backend.name()));
                continue;
            }
        };
        for snapshot in devices {
            if backend == BackendId::StandardHid
                && claimed.contains(&snapshot.target.interface.path)
            {
                continue;
            }
            if backend.precedes_standard() {
                claimed.insert(snapshot.target.interface.path.clone());
            }
            snapshots.push(snapshot);
        }
    }
    if successful_backends == 0 && !failures.is_empty() {
        return Err(failures.join("; "));
    }
    Ok(snapshots)
}

pub fn read_backend(backend: BackendId, detail: bool) -> Result<Vec<Snapshot>, String> {
    let api = HidApi::load()?;
    let interfaces = crate::hid::enumerate_all_report(&api)?.interfaces;
    let provider: &dyn DeviceBackend = match backend {
        BackendId::Rog => &rog::Rog,
        BackendId::Logitech => &logitech::Logitech,
        BackendId::Rapoo => &rapoo::Rapoo,
        BackendId::Vgn => &vgn::Vgn,
        BackendId::StandardHid => &standard::StandardHid,
    };
    let mut snapshots = Vec::new();
    for target in provider.discover(&api, &interfaces)? {
        if let Ok(snapshot) = provider.read(&api, &target, detail) {
            if snapshot.battery.is_some() || snapshot.dpi.current.is_some() {
                snapshots.push(snapshot);
            }
        }
    }
    Ok(snapshots)
}

pub fn set_dpi(target: &Target, dpi: u16) -> Result<(), String> {
    let api = HidApi::load()?;
    match target.backend {
        BackendId::Rog => rog::Rog.set_dpi(&api, target, dpi),
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
    match read_additional(detail) {
        Ok(additional) => snapshots.extend(additional),
        Err(error) if snapshots.is_empty() => return Err(error),
        Err(_) => (),
    }
    Ok(snapshots)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(backend: BackendId) -> Snapshot {
        let interface = HidInterface {
            path: "shared-interface".into(),
            serial: String::new(),
            product: "Test mouse".into(),
            vendor_id: 0x046d,
            product_id: 0xc08b,
            usage_page: 1,
            usage: 2,
            input_len: 20,
            output_len: 20,
            feature_len: 0,
        };
        Snapshot {
            target: Target::new(backend, &interface, 1),
            connected: true,
            dpi: Dpi::default(),
            battery: Some(Battery {
                percent: Some(75),
                charging: None,
                voltage_mv: None,
                approximate: false,
                level: None,
            }),
        }
    }

    #[test]
    fn one_backend_failure_does_not_discard_another_backend_reading() {
        let result = merge_additional_results([
            (BackendId::Logitech, Err("unavailable".into())),
            (
                BackendId::StandardHid,
                Ok(vec![reading(BackendId::StandardHid)]),
            ),
        ])
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].target.backend, BackendId::StandardHid);
        assert_eq!(result[0].battery.as_ref().unwrap().percent, Some(75));
    }

    #[test]
    fn vendor_reading_has_priority_over_standard_reading_on_same_interface() {
        for backend in [BackendId::Logitech, BackendId::Rapoo, BackendId::Vgn] {
            let result = merge_additional_results([
                (backend, Ok(vec![reading(backend)])),
                (
                    BackendId::StandardHid,
                    Ok(vec![reading(BackendId::StandardHid)]),
                ),
            ])
            .unwrap();
            assert_eq!(result.len(), 1);
            assert_eq!(result[0].target.backend, backend);
        }
    }

    #[test]
    fn all_backend_failures_remain_an_error() {
        assert!(merge_additional_results([
            (BackendId::Logitech, Err("unavailable".into())),
            (BackendId::StandardHid, Err("unavailable".into()))
        ])
        .is_err());
    }
}
