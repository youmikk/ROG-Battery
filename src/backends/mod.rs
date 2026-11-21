//! 品牌适配层。未知能力保留为空，不能从设备类型推断写入协议。
pub mod logitech;
mod channel;
mod enumeration;
pub mod compx;
mod feature;
pub mod headset;
pub mod keyboard20;
pub mod page_command;
pub mod rapoo;
mod rapoo_config;
pub mod razer;
mod razer_capabilities;
pub mod rog;
pub mod standard;
pub mod sony;
pub mod steelseries;
pub mod status;
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
    CompX,
    Razer,
    PageCommand,
    Headset,
    Keyboard20,
    Nova,
    DualSense,
    StandardHid,
}

pub const ADDITIONAL_BACKENDS: [BackendId; 11] = [
    BackendId::Logitech, BackendId::Rapoo, BackendId::Vgn, BackendId::CompX, BackendId::Razer,
    BackendId::PageCommand, BackendId::Headset, BackendId::Keyboard20, BackendId::Nova,
    BackendId::DualSense, BackendId::StandardHid,
];

impl BackendId {
    pub fn name(self) -> &'static str {
        match self {
            Self::Rog => "ROG",
            Self::Logitech => "Logitech HID++",
            Self::Rapoo => "Rapoo nRF54L",
            Self::Vgn => "VGN V98 Pro V3",
            Self::CompX => "CompX mouse",
            Self::Razer => "Razer Feature",
            Self::PageCommand => "CompX page-command",
            Self::Headset => "FF90 headset",
            Self::Keyboard20 => "MCHOSE keyboard",
            Self::Nova => "Arctis Nova",
            Self::DualSense => "Sony DualSense",
            Self::StandardHid => "Standard HID",
        }
    }

    pub fn precedes_standard(self) -> bool {
        matches!(self, Self::Logitech | Self::Rapoo | Self::Vgn | Self::CompX | Self::Razer
            | Self::PageCommand | Self::Headset | Self::Keyboard20 | Self::Nova | Self::DualSense)
    }

    pub fn sample_lifetime(self) -> std::time::Duration {
        std::time::Duration::from_secs(match self {
            Self::Rapoo | Self::DualSense => 8,
            Self::Logitech => 180,
            Self::CompX => 45,
            Self::Razer | Self::PageCommand | Self::Headset | Self::Keyboard20 | Self::Nova => 120,
            Self::Vgn | Self::StandardHid => 120,
            Self::Rog => 0,
        })
    }

    /// 被动状态采样需要及时发布到界面，不能沿用主动查询的长间隔。
    pub fn publish_interval_limit_ms(self) -> Option<u64> {
        match self {
            Self::Rapoo | Self::DualSense => Some(2_000),
            Self::CompX | Self::Razer | Self::PageCommand | Self::Headset | Self::Keyboard20 | Self::Nova => Some(10_000),
            _ => None,
        }
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
    /// A device address on one control channel, independent of serial promotion.
    pub fn same_channel(&self, other: &Self) -> bool {
        self.backend == other.backend && self.address == other.address
            && if self.backend == BackendId::Razer {
                razer::collection_group(&self.interface) == razer::collection_group(&other.interface)
            } else {
                self.interface.path.eq_ignore_ascii_case(&other.interface.path)
            }
    }

    pub fn new(backend: BackendId, interface: &HidInterface, address: u8) -> Self {
        let identity = if backend == BackendId::Razer {
            razer::collection_group(interface)
        } else if interface.serial.is_empty() || matches!(backend, BackendId::Rapoo | BackendId::CompX
            | BackendId::PageCommand | BackendId::Headset | BackendId::Keyboard20 | BackendId::Nova
            | BackendId::DualSense) {
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
    pub polling_rate: crate::device::PollingRate,
    pub connected: bool,
    pub battery_at: Option<std::time::Instant>,
}

impl Snapshot {
    pub fn disable_writes(&mut self) {
        self.dpi.disable_writes();
        self.polling_rate.writable = false;
    }
}

#[derive(Default)]
pub struct Poll {
    pub snapshots: Vec<Snapshot>,
    /// None 表示枚举失败，不提供拔出设备的证据。
    pub present_paths: Option<std::collections::BTreeSet<String>>,
}

impl Poll {
    #[cfg(test)]
    pub fn from_snapshots(snapshots: Vec<Snapshot>) -> Self {
        let present_paths = Some(snapshots.iter().map(|s| s.target.interface.path.clone()).collect());
        Self { snapshots, present_paths }
    }
}

#[cfg(test)]
impl From<Vec<Snapshot>> for Poll {
    fn from(snapshots: Vec<Snapshot>) -> Self { Self::from_snapshots(snapshots) }
}

pub trait DeviceBackend {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String>;
    fn read(&self, api: &HidApi, target: &Target, detail: bool) -> Result<Snapshot, String>;
    fn set_dpi(&self, _api: &HidApi, _target: &Target, _dpi: u16) -> Result<(), String> {
        Err("设备不支持 DPI 写入".into())
    }
    fn set_dpi_stage(&self, _api: &HidApi, _target: &Target,
        _stage: crate::device::DpiStage, _context: crate::device::StageContext) -> Result<(), String> {
        Err("设备不支持 DPI 档位切换".into())
    }
    fn edit_dpi_stage(&self, _api: &HidApi, _target: &Target,
        _stage: crate::device::DpiStage, _context: crate::device::StageContext, _expected_y: u16, _x: u16, _y: u16) -> Result<(), String> {
        Err("设备不支持 DPI 档位编辑".into())
    }
    fn set_polling_rate(&self, _api: &HidApi, _target: &Target, _hz: u16) -> Result<(), String> {
        Err("设备不支持回报率设置".into())
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
    Ok(poll_backend(backend, detail)?.snapshots)
}

pub fn poll_backend(backend: BackendId, detail: bool) -> Result<Poll, String> {
    let api = HidApi::load()?;
    let enumeration = enumeration::scan(&api)?;
    let present_paths = enumeration.interfaces.iter().map(|i|i.path.clone())
        .chain(enumeration.skipped.iter().map(|i|i.path.clone())).collect();
    let interfaces = enumeration.interfaces;
    let provider: &dyn DeviceBackend = match backend {
        BackendId::Rog => &rog::Rog,
        BackendId::Logitech => &logitech::Logitech,
        BackendId::Rapoo => &rapoo::Rapoo,
        BackendId::Vgn => &vgn::Vgn,
        BackendId::CompX => &compx::CompX,
        BackendId::Razer => &razer::Razer,
        BackendId::PageCommand => &page_command::PageCommand,
        BackendId::Headset => &headset::Headset,
        BackendId::Keyboard20 => &keyboard20::Keyboard20,
        BackendId::Nova => &steelseries::Nova,
        BackendId::DualSense => &sony::DualSense,
        BackendId::StandardHid => &standard::StandardHid,
    };
    let mut poll = Poll { present_paths: Some(present_paths),
        ..Default::default() };
    poll.snapshots = read_targets(provider.discover(&api, &interfaces)?, |target| provider.read(&api, target, detail));
    Ok(poll)
}

fn read_targets(targets: Vec<Target>, mut read: impl FnMut(&Target) -> Result<Snapshot, String>) -> Vec<Snapshot> {
    let mut snapshots: Vec<Snapshot> = Vec::new();
    for target in targets {
        // A successful sibling has already read this device, even if its serial
        // changed the key. Failed collections never prevent trying a sibling.
        if snapshots.iter().any(|s| s.target.same_channel(&target)) { continue; }
        if let Ok(snapshot) = read(&target) {
            if !snapshots.iter().any(|s| s.target.key == snapshot.target.key) {
                snapshots.push(snapshot);
            }
        }
    }
    snapshots
}

pub fn set_dpi(target: &Target, dpi: u16) -> Result<(), String> {
    let api = HidApi::load()?;
    match target.backend {
        BackendId::Rog => rog::Rog.set_dpi(&api, target, dpi),
        BackendId::Logitech => logitech::Logitech.set_dpi(&api, target, dpi),
        BackendId::CompX => compx::CompX.set_dpi(&api, target, dpi),
        BackendId::Razer => razer::Razer.set_dpi(&api, target, dpi),
        _ => Err("此协议不提供通用 DPI 写入".into()),
    }
}

pub fn set_dpi_stage(target: &Target, stage: crate::device::DpiStage, context: crate::device::StageContext) -> Result<(), String> {
    let api = HidApi::load()?;
    match target.backend {
        BackendId::Logitech => logitech::Logitech.set_dpi_stage(&api, target, stage, context),
        BackendId::CompX => compx::CompX.set_dpi_stage(&api, target, stage, context),
        BackendId::Razer => razer::Razer.set_dpi_stage(&api, target, stage, context),
        BackendId::Rapoo => rapoo::Rapoo.set_dpi_stage(&api, target, stage, context),
        _ => Err("设备不支持 DPI 档位切换".into()),
    }
}

pub fn edit_dpi_stage(target: &Target, stage: crate::device::DpiStage, context: crate::device::StageContext,
    expected_y: u16, x: u16, y: u16) -> Result<(), String> {
    let api = HidApi::load()?;
    match target.backend {
        BackendId::CompX => compx::CompX.edit_dpi_stage(&api, target, stage, context, expected_y, x, y),
        BackendId::Razer => razer::Razer.edit_dpi_stage(&api, target, stage, context, expected_y, x, y),
        _ => Err("设备不支持 DPI 档位编辑".into()),
    }
}

pub fn set_polling_rate(target: &Target, hz: u16) -> Result<(), String> {
    let api = HidApi::load()?;
    match target.backend {
        BackendId::CompX => compx::CompX.set_polling_rate(&api, target, hz),
        BackendId::Logitech => logitech::Logitech.set_polling_rate(&api, target, hz),
        BackendId::Razer => razer::Razer.set_polling_rate(&api, target, hz),
        _ => Err("设备不支持回报率设置".into()),
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
            battery_at: Some(std::time::Instant::now()), polling_rate: Default::default(),
            dpi: Dpi::default(),
            battery: Some(Battery {
                estimated_percent: None,
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
        for backend in ADDITIONAL_BACKENDS.into_iter().filter(|id| id.precedes_standard()) {
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

    #[test]
    fn successful_collection_stops_sibling_reads_after_identity_promotion() {
        let mut first = reading(BackendId::Razer);
        first.target.interface.path = "hid#vid_1532&pid_00c1&mi_00&col01#7&aaaa&0&0001#{hid}".into();
        first.target = Target::new(BackendId::Razer, &first.target.interface, 0);
        let mut sibling = first.target.clone();
        sibling.interface.path = sibling.interface.path.replace("col01", "col02").replace("0001#", "0002#");
        let mut calls = 0;
        let snapshots = read_targets(vec![first.target.clone(), sibling], |_| {
            calls += 1;
            let mut result = first.clone();
            result.target.key = "serial-confirmed".into();
            result.target.verified_identity = true;
            Ok(result)
        });
        assert_eq!(calls, 1);
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].target.key, "serial-confirmed");
    }

    #[test]
    fn failed_collection_allows_sibling_and_does_not_merge_identical_receivers() {
        let mut first = reading(BackendId::Razer);
        first.target.interface.path = "hid#vid_1532&pid_00c1&mi_00&col01#7&aaaa&0&0001#{hid}".into();
        first.target = Target::new(BackendId::Razer, &first.target.interface, 0);
        let mut sibling = first.target.clone();
        sibling.interface.path = sibling.interface.path.replace("col01", "col02").replace("0001#", "0002#");
        let mut second = sibling.clone();
        second.interface.path = second.interface.path.replace("aaaa", "bbbb");
        let mut calls = 0;
        let snapshots = read_targets(vec![first.target.clone(), sibling, second], |target| {
            calls += 1;
            if calls == 1 { return Err("silent collection".into()); }
            let mut result = first.clone();
            result.target = Target::new(BackendId::Razer, &target.interface, target.address);
            Ok(result)
        });
        assert_eq!(calls, 3);
        assert_eq!(snapshots.len(), 2);
        assert_ne!(snapshots[0].target.key, snapshots[1].target.key);
        assert!(!snapshots[0].target.same_channel(&snapshots[1].target));
    }
}
