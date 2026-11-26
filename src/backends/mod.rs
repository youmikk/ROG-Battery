//! 品牌适配层。未知能力保留为空，不能从设备类型推断写入协议。
pub mod logitech;
pub mod logitech_headset;
pub mod lofree;
pub mod mchose_qhw;
pub mod mchose_mouse;
pub mod mchose_modern;
pub mod mchose_glw;
pub mod mchose_k99;
pub mod mchose_k87s;
pub mod mchose_mc;
pub mod mchose_r9;
pub mod moondrop;
mod channel;
pub mod cherry;
mod descriptor;
pub mod edifier;
pub mod eweadn;
pub mod eweadn_s9pro;
pub mod eweadn_x23;
pub mod eweadn_x87;
pub mod dareu;
pub mod dareu_em901;
pub mod eightbitdo;
pub mod flydigi;
pub mod gamesir;
pub mod gearhub;
pub mod g3m;
pub mod ajazz_ak980;
pub mod aula;
pub mod aula_hfd;
pub mod aula_hfd_mechanical;
pub mod aula_v9pro;
pub mod aula_f99;
pub mod aula_f108;
pub mod atk_nano;
pub mod atk_holtek;
pub mod bluetooth;
pub mod lingbao;
mod enumeration;
pub mod compx;
mod feature;
pub mod headset;
pub mod fifine_ht6;
pub mod hyperx;
pub mod inzone;
pub mod keyboard20;
pub mod page_command;
pub mod rapoo;
pub mod rapoo_legacy;
pub mod rapoo_office;
pub mod rapoo_mt560;
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
pub mod vgn_beiying;
pub mod vgn_dragonfly3;
pub mod vgn_ry;
pub mod vgn_keyboard;
pub mod veekos;
pub mod xinput;
pub mod xiberia;
pub mod thunderobot;
pub mod akg;
pub mod langtu;

pub use crate::device::{Battery, BatteryLevel, Dpi};
use crate::{device::DeviceFamily, hid::HidInterface, win32::HidApi};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BackendId {
    Rog,
    Logitech,
    LogitechHeadset,
    Rapoo,
    RapooLegacy,
    RapooOffice,
    RapooMt560,
    Vgn,
    VgnBeiying,
    VgnDragonfly3,
    VgnRy,
    VgnKeyboard,
    CompX,
    Razer,
    PageCommand,
    Headset,
    Keyboard20,
    MchoseQhw,
    MchoseMouse,
    MchoseModern,
    MchoseGlw,
    MchoseK99,
    MchoseK87s,
    MchoseMc,
    MchoseR9,
    Moondrop,
    Edifier,
    EweadnS9,
    EweadnS9Pro,
    EweadnX23,
    EweadnX87,
    Dareu,
    DareuEm901,
    EightBitDo,
    Flydigi,
    GameSir,
    GearHub,
    G3m,
    AjazzAk980,
    VeekosShine60,
    Aula,
    AulaHfd,
    AulaHfdMechanical,
    AulaV9Pro,
    AulaF99,
    AulaF108,
    AtkNano,
    AtkHoltek,
    Lingbao,
    Cherry,
    Lofree,
    Nova,
    Inzone,
    HyperX,
    DualSense,
    XInput,
    Bluetooth,
    Xiberia,
    FifineHt6,
    StandardHid,
    Thunderobot,
    Akg,
    LangtuT98,
}

pub const ADDITIONAL_BACKENDS: [BackendId; 62] = [
    BackendId::Logitech, BackendId::LogitechHeadset, BackendId::Rapoo, BackendId::Vgn, BackendId::VgnBeiying, BackendId::VgnDragonfly3, BackendId::VgnKeyboard, BackendId::CompX, BackendId::Razer,
    BackendId::PageCommand, BackendId::Headset, BackendId::Keyboard20, BackendId::MchoseQhw, BackendId::MchoseMouse, BackendId::MchoseGlw, BackendId::Edifier, BackendId::EweadnS9, BackendId::EweadnS9Pro, BackendId::Dareu, BackendId::GearHub, BackendId::G3m, BackendId::Cherry, BackendId::Lofree, BackendId::Nova,
    BackendId::EightBitDo, BackendId::Flydigi, BackendId::GameSir, BackendId::Aula, BackendId::Inzone, BackendId::HyperX, BackendId::DualSense, BackendId::XInput, BackendId::Bluetooth,
    BackendId::VgnRy, BackendId::MchoseModern, BackendId::MchoseK99, BackendId::Moondrop, BackendId::RapooLegacy,
    BackendId::RapooOffice, BackendId::AulaHfd, BackendId::MchoseMc,
    BackendId::AulaF99, BackendId::AtkNano, BackendId::AjazzAk980, BackendId::VeekosShine60, BackendId::EweadnX23, BackendId::EweadnX87, BackendId::Xiberia,
    BackendId::AtkHoltek, BackendId::FifineHt6, BackendId::MchoseK87s,
    BackendId::Lingbao, BackendId::AulaF108,
    BackendId::RapooMt560, BackendId::MchoseR9, BackendId::AulaV9Pro,
    BackendId::DareuEm901, BackendId::AulaHfdMechanical, BackendId::Thunderobot, BackendId::Akg, BackendId::LangtuT98, BackendId::StandardHid,
];

impl BackendId {
    pub fn name(self) -> &'static str {
        match self {
            Self::Rog => "ROG",
            Self::Logitech => "Logitech HID++",
            Self::LogitechHeadset => "Logitech G PRO X 2",
            Self::Rapoo => "Rapoo nRF54L",
            Self::RapooLegacy => "Rapoo VT legacy",
            Self::RapooOffice => "Rapoo office mouse",
            Self::RapooMt560 => "Rapoo MT560",
            Self::Vgn => "VGN V98 Pro V3",
            Self::VgnBeiying => "VGN / VXE keyboard",
            Self::VgnDragonfly3 => "VGN Dragonfly3 SE / Pro",
            Self::VgnRy => "VGN ryMs mouse",
            Self::VgnKeyboard => "Weisheng keyboard",
            Self::CompX => "CompX mouse",
            Self::Razer => "Razer Feature",
            Self::PageCommand => "CompX page-command",
            Self::Headset => "FF90 headset",
            Self::Keyboard20 => "MCHOSE keyboard",
            Self::MchoseQhw => "MCHOSE QHW keyboard",
            Self::MchoseMouse => "MCHOSE Feature mouse",
            Self::MchoseModern => "MCHOSE 4D mouse",
            Self::MchoseGlw => "MCHOSE G87 V2",
            Self::MchoseK99 => "MCHOSE K99 V3",
            Self::MchoseK87s => "MCHOSE K87S",
            Self::MchoseMc => "MCHOSE MC headset",
            Self::MchoseR9 => "MCHOSE R9",
            Self::Moondrop => "MOONDROP GM01 Pro",
            Self::Edifier => "EDIFIER Halo Space",
            Self::EweadnS9 => "EWEADN S9",
            Self::EweadnS9Pro => "EWEADN S9 PRO 8K",
            Self::EweadnX23 => "EWEADN X23 Pro",
            Self::EweadnX87 => "EWEADN X87 keyboard",
            Self::Dareu => "DAREU Jm",
            Self::DareuEm901 => "DAREU EM901 / CM610 Pro",
            Self::EightBitDo => "8BitDo Pro 3",
            Self::Flydigi => "Flydigi APEX5",
            Self::GameSir => "GameSir controller",
            Self::GearHub => "GearHub Feature mouse",
            Self::G3m => "HECATE G3M PRO",
            Self::AjazzAk980 => "AJAZZ AK980 PRO",
            Self::VeekosShine60 => "Veekos Shine 60",
            Self::Aula => "AULA SC380 PRO",
            Self::AulaHfd => "AULA HFD keyboard",
            Self::AulaHfdMechanical => "AULA HFD mechanical keyboard",
            Self::AulaV9Pro => "AULA V9Pro+",
            Self::AulaF99 => "AULA F99 MAX",
            Self::AulaF108 => "AULA F108 Pro",
            Self::AtkNano => "ATK / VXE NANO",
            Self::AtkHoltek => "ATK 8K Nano",
            Self::Lingbao => "Lingbao M1 Pro",
            Self::Cherry => "CHERRY keyboard",
            Self::Lofree => "Lofree HYZEN",
            Self::Nova => "SteelSeries Arctis",
            Self::Inzone => "Sony INZONE",
            Self::HyperX => "HyperX headset",
            Self::DualSense => "Sony DualSense",
            Self::XInput => "XInput",
            Self::Bluetooth => "Bluetooth",
            Self::Xiberia => "XIBERIA K06S",
            Self::FifineHt6 => "FIFINE HT6",
            Self::StandardHid => "Standard HID",
            Self::Thunderobot => "THUNDEROBOT ML903 NearLink Se",
            Self::Akg => "AKG N9 Hybrid",
            Self::LangtuT98 => "LANGTU T98",
        }
    }

    pub fn precedes_standard(self) -> bool {
        if matches!(self, Self::Thunderobot | Self::Akg | Self::LangtuT98) { return true; }
        if matches!(self, Self::MchoseK99 | Self::Moondrop | Self::RapooLegacy
            | Self::RapooOffice | Self::AulaHfd | Self::MchoseMc | Self::AulaF99
            | Self::AtkNano | Self::EweadnX23 | Self::EweadnX87 | Self::Xiberia
            | Self::AtkHoltek | Self::FifineHt6 | Self::MchoseK87s
            | Self::Lingbao | Self::AulaF108 | Self::RapooMt560
            | Self::MchoseR9 | Self::AulaV9Pro | Self::DareuEm901 | Self::AjazzAk980
            | Self::VeekosShine60
            | Self::AulaHfdMechanical) { return true; }
        matches!(self, Self::Logitech | Self::LogitechHeadset | Self::Rapoo | Self::Vgn | Self::VgnBeiying | Self::VgnDragonfly3 | Self::VgnKeyboard | Self::CompX | Self::Razer
            | Self::PageCommand | Self::Headset | Self::Keyboard20 | Self::MchoseQhw | Self::MchoseMouse | Self::MchoseModern | Self::VgnRy | Self::MchoseGlw | Self::Edifier | Self::EweadnS9 | Self::EweadnS9Pro | Self::Dareu | Self::EightBitDo | Self::Flydigi | Self::GameSir | Self::GearHub | Self::G3m | Self::AjazzAk980 | Self::Aula | Self::Cherry | Self::Lofree | Self::Nova | Self::Inzone | Self::HyperX | Self::DualSense)
    }

    pub fn sample_lifetime(self) -> std::time::Duration {
        std::time::Duration::from_secs(match self {
            Self::Thunderobot | Self::LangtuT98 => 120,
            Self::Akg => 180,
            Self::MchoseK99 | Self::Moondrop | Self::RapooLegacy
                | Self::RapooOffice | Self::AulaHfd | Self::MchoseMc | Self::AulaF99
                | Self::AtkNano | Self::EweadnX23 | Self::EweadnX87 | Self::Xiberia
                | Self::AtkHoltek | Self::FifineHt6 | Self::MchoseK87s
                | Self::Lingbao | Self::AulaF108 | Self::RapooMt560
                | Self::MchoseR9 | Self::AulaV9Pro | Self::DareuEm901 | Self::AjazzAk980
                | Self::VeekosShine60
                | Self::AulaHfdMechanical => 120,
            Self::Rapoo | Self::DualSense | Self::XInput => 8,
            Self::Bluetooth => 120,
            Self::Logitech | Self::LogitechHeadset => 180,
            Self::CompX => 45,
            Self::Razer | Self::PageCommand | Self::Headset | Self::Keyboard20 | Self::Cherry | Self::Lofree | Self::Nova | Self::Inzone | Self::HyperX => 120,
            Self::Vgn | Self::VgnBeiying | Self::VgnDragonfly3 | Self::VgnRy | Self::VgnKeyboard | Self::MchoseQhw | Self::MchoseMouse | Self::MchoseModern | Self::MchoseGlw | Self::Edifier | Self::EweadnS9 | Self::EweadnS9Pro | Self::Dareu | Self::EightBitDo | Self::Flydigi | Self::GameSir | Self::GearHub | Self::G3m | Self::Aula | Self::StandardHid => 120,
            Self::Rog => 0,
        })
    }

    /// 被动状态采样需要及时发布到界面，不能沿用主动查询的长间隔。
    pub fn publish_interval_limit_ms(self) -> Option<u64> {
        match self {
            Self::Thunderobot => Some(10_000),
            Self::Akg => Some(60_000),
            Self::MchoseK99 | Self::Moondrop | Self::RapooLegacy
                | Self::RapooOffice | Self::AulaHfd | Self::MchoseMc | Self::AulaF99
                | Self::AtkNano | Self::EweadnX87 | Self::Xiberia
                | Self::AtkHoltek | Self::FifineHt6 | Self::MchoseK87s
                | Self::Lingbao | Self::MchoseR9 | Self::AulaV9Pro | Self::AjazzAk980
                | Self::VeekosShine60
                | Self::DareuEm901 => Some(10_000),
            Self::EweadnX23 => Some(5_000),
            Self::AulaF108 | Self::LangtuT98 => Some(15_000),
            Self::AulaHfdMechanical => Some(30_000),
            Self::Rapoo | Self::RapooMt560 | Self::DualSense | Self::XInput => Some(2_000),
            Self::Bluetooth => Some(30_000),
            Self::LogitechHeadset => Some(120_000),
            Self::VgnBeiying | Self::VgnDragonfly3 | Self::VgnRy | Self::VgnKeyboard | Self::MchoseQhw | Self::MchoseMouse | Self::MchoseModern | Self::MchoseGlw | Self::Edifier | Self::EweadnS9 | Self::EweadnS9Pro | Self::Dareu | Self::EightBitDo | Self::Flydigi | Self::GameSir | Self::GearHub | Self::G3m | Self::Aula | Self::CompX | Self::Razer | Self::PageCommand | Self::Headset | Self::Keyboard20 | Self::Cherry | Self::Lofree | Self::Nova | Self::Inzone | Self::HyperX => Some(10_000),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Endpoint {
    Hid(HidInterface),
    XInput(u8),
}

#[derive(Clone, Debug)]
pub struct Target {
    pub key: String,
    pub name: String,
    pub family: DeviceFamily,
    pub backend: BackendId,
    pub endpoint: Endpoint,
    /// 多设备接收器内的地址，与 report ID 分开。
    pub address: u8,
    pub verified_identity: bool,
}

impl Target {
    pub fn hid(&self) -> Result<&HidInterface, String> {
        match &self.endpoint {
            Endpoint::Hid(interface) => Ok(interface),
            Endpoint::XInput(_) => Err("此设备没有 HID 控制接口".into()),
        }
    }

    #[cfg(test)]
    pub fn hid_mut(&mut self) -> &mut HidInterface {
        match &mut self.endpoint {
            Endpoint::Hid(interface) => interface,
            Endpoint::XInput(_) => panic!("expected HID test target"),
        }
    }

    pub fn channel_key(&self) -> String {
        match &self.endpoint {
            Endpoint::Hid(interface) => interface.path.clone(),
            Endpoint::XInput(slot) => format!("xinput:{slot}"),
        }
    }

    pub fn unverified_key(&self) -> String {
        self.hid().map_or_else(|_| self.key.clone(), |i| Self::new(self.backend, i, self.address).key)
    }

    pub fn same_hid_path(&self, other: &Self) -> bool {
        match (self.hid(), other.hid()) {
            (Ok(a), Ok(b)) => a.path.eq_ignore_ascii_case(&b.path),
            _ => false,
        }
    }

    /// A device address on one control channel, independent of serial promotion.
    pub fn same_channel(&self, other: &Self) -> bool {
        if self.backend != other.backend || self.address != other.address { return false; }
        match (&self.endpoint, &other.endpoint) {
            (Endpoint::Hid(a), Endpoint::Hid(b)) if self.backend == BackendId::Razer =>
                razer::collection_group(a) == razer::collection_group(b),
            (Endpoint::Hid(a), Endpoint::Hid(b)) => a.path.eq_ignore_ascii_case(&b.path),
            (Endpoint::XInput(a), Endpoint::XInput(b)) => a == b,
            _ => false,
        }
    }

    pub fn new(backend: BackendId, interface: &HidInterface, address: u8) -> Self {
        let identity = match backend {
            BackendId::Razer => razer::collection_group(interface),
            BackendId::StandardHid | BackendId::Bluetooth | BackendId::Rog
                if !interface.serial.is_empty() => format!(
                "{:04x}:{:04x}:{}",
                interface.vendor_id, interface.product_id, interface.serial
            ),
            _ => interface.path.to_ascii_lowercase(),
        };
        // Backend display names may expand; existing device preferences keep their key.
        let key_prefix = match backend {
            BackendId::VgnDragonfly3 => "VGN Dragonfly3 SE",
            BackendId::Nova => "Arctis Nova",
            BackendId::HyperX => "HyperX Alpha 2",
            _ => backend.name(),
        };
        Self {
            key: crate::device::stable_key(&format!("{key_prefix}:{identity}:{address}")),
            name: interface.product.clone(),
            family: match (interface.usage_page, interface.usage) {
                (1, 2) => DeviceFamily::Mouse,
                (1, 6) => DeviceFamily::Keyboard,
                (1, 4 | 5) => DeviceFamily::Gamepad,
                _ => crate::device::family_of(&interface.product),
            },
            backend,
            endpoint: Endpoint::Hid(interface.clone()),
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
    /// Explicit no-battery evidence, distinct from a silent or failed query.
    pub battery_unavailable: bool,
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
    pub present_channels: Option<std::collections::BTreeSet<String>>,
    pub legacy_key_migrations: std::collections::BTreeMap<String, String>,
}

impl Poll {
    #[cfg(test)]
    pub fn from_snapshots(snapshots: Vec<Snapshot>) -> Self {
        let present_channels = Some(snapshots.iter().map(|s| s.target.channel_key()).collect());
        let targets: Vec<_> = snapshots.iter().map(|snapshot| snapshot.target.clone()).collect();
        let legacy_key_migrations = legacy_key_migrations(&targets, &[]);
        Self { snapshots, present_channels, legacy_key_migrations }
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
    let mut snapshots: Vec<Snapshot> = Vec::new();
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
            if backend == BackendId::Bluetooth
                && snapshots.iter().any(|other| {
                    other.target.backend != BackendId::Bluetooth
                        && bluetooth::same_physical_device(&other.target, &snapshot.target)
                })
            {
                continue;
            }
            if backend != BackendId::Bluetooth {
                snapshots.retain(|other| {
                    other.target.backend != BackendId::Bluetooth
                        || !bluetooth::same_physical_device(&other.target, &snapshot.target)
                });
            }
            if backend == BackendId::StandardHid
                && claimed.contains(&snapshot.target.channel_key())
            {
                continue;
            }
            if backend.precedes_standard() {
                claimed.insert(snapshot.target.channel_key());
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
    if backend == BackendId::XInput { return xinput::poll(); }
    if backend == BackendId::Bluetooth { return bluetooth::poll(); }
    let api = HidApi::load()?;
    let enumeration = enumeration::scan(&api)?;
    let present_channels = enumeration.interfaces.iter().map(|i|i.path.clone())
        .chain(enumeration.skipped.iter().map(|i|i.path.clone())).collect();
    let interfaces = enumeration.interfaces;
    let provider: &dyn DeviceBackend = match backend {
        BackendId::Rog => &rog::Rog,
        BackendId::Logitech => &logitech::Logitech,
        BackendId::LogitechHeadset => &logitech_headset::LogitechHeadset,
        BackendId::Rapoo => &rapoo::Rapoo,
        BackendId::RapooLegacy => &rapoo_legacy::RapooLegacy,
        BackendId::RapooOffice => &rapoo_office::RapooOffice,
        BackendId::RapooMt560 => &rapoo_mt560::RapooMt560,
        BackendId::Vgn => &vgn::Vgn,
        BackendId::VgnBeiying => &vgn_beiying::VgnBeiying,
        BackendId::VgnDragonfly3 => &vgn_dragonfly3::VgnDragonfly3,
        BackendId::VgnRy => &vgn_ry::VgnRy,
        BackendId::VgnKeyboard => &vgn_keyboard::VgnKeyboard,
        BackendId::CompX => &compx::CompX,
        BackendId::Razer => &razer::Razer,
        BackendId::PageCommand => &page_command::PageCommand,
        BackendId::Headset => &headset::Headset,
        BackendId::Keyboard20 => &keyboard20::Keyboard20,
        BackendId::MchoseQhw => &mchose_qhw::MchoseQhw,
        BackendId::MchoseMouse => &mchose_mouse::MchoseMouse,
        BackendId::MchoseModern => &mchose_modern::MchoseModern,
        BackendId::MchoseGlw => &mchose_glw::MchoseGlw,
        BackendId::MchoseK99 => &mchose_k99::MchoseK99,
        BackendId::MchoseK87s => &mchose_k87s::MchoseK87s,
        BackendId::MchoseMc => &mchose_mc::MchoseMc,
        BackendId::MchoseR9 => &mchose_r9::MchoseR9,
        BackendId::Moondrop => &moondrop::Moondrop,
        BackendId::Edifier => &edifier::Edifier,
        BackendId::EweadnS9 => &eweadn::EweadnS9,
        BackendId::EweadnS9Pro => &eweadn_s9pro::EweadnS9Pro,
        BackendId::EweadnX23 => &eweadn_x23::EweadnX23,
        BackendId::EweadnX87 => &eweadn_x87::EweadnX87,
        BackendId::Dareu => &dareu::Dareu,
        BackendId::DareuEm901 => &dareu_em901::DareuEm901,
        BackendId::EightBitDo => &eightbitdo::EightBitDo,
        BackendId::Flydigi => &flydigi::Flydigi,
        BackendId::GameSir => &gamesir::GameSir,
        BackendId::GearHub => &gearhub::GearHub,
        BackendId::G3m => &g3m::G3m,
        BackendId::AjazzAk980 => &ajazz_ak980::AjazzAk980,
        BackendId::VeekosShine60 => &veekos::VeekosShine60,
        BackendId::Aula => &aula::Aula,
        BackendId::AulaHfd => &aula_hfd::AulaHfd,
        BackendId::AulaHfdMechanical => &aula_hfd_mechanical::AulaHfdMechanical,
        BackendId::AulaV9Pro => &aula_v9pro::AulaV9Pro,
        BackendId::AulaF99 => &aula_f99::AulaF99,
        BackendId::AulaF108 => &aula_f108::AulaF108,
        BackendId::AtkNano => &atk_nano::AtkNano,
        BackendId::AtkHoltek => &atk_holtek::AtkHoltek,
        BackendId::Lingbao => &lingbao::Lingbao,
        BackendId::Cherry => &cherry::Cherry,
        BackendId::Lofree => &lofree::Lofree,
        BackendId::Nova => &steelseries::Nova,
        BackendId::Inzone => &inzone::Inzone,
        BackendId::HyperX => &hyperx::HyperX,
        BackendId::LangtuT98 => &langtu::LangtuT98,
        BackendId::DualSense => &sony::DualSense,
        BackendId::XInput => unreachable!("XInput does not enumerate HID interfaces"),
        BackendId::Bluetooth => unreachable!("Bluetooth does not enumerate HID interfaces"),
        BackendId::StandardHid => &standard::StandardHid,
        BackendId::Xiberia => &xiberia::Xiberia,
        BackendId::FifineHt6 => &fifine_ht6::FifineHt6,
        BackendId::Thunderobot => &thunderobot::Thunderobot,
        BackendId::Akg => &akg::Akg,
    };
    let mut poll = Poll { present_channels: Some(present_channels),
        ..Default::default() };
    let targets = provider.discover(&api, &interfaces)?;
    poll.legacy_key_migrations = legacy_key_migrations(&targets, &enumeration.skipped);
    poll.snapshots = read_targets(targets, |target| provider.read(&api, target, detail));
    Ok(poll)
}

fn legacy_key_migrations(targets: &[Target], skipped: &[crate::hid::Skipped])
    -> std::collections::BTreeMap<String, String> {
    let mut migrations = std::collections::BTreeMap::new();
    for target in targets.iter().filter(|target| target.backend == BackendId::Rapoo) {
        let Ok(interface) = target.hid() else { continue; };
        if interface.serial.is_empty() { continue; }
        if skipped.iter().any(|entry| {
            let path = entry.path.to_ascii_lowercase();
            let field = |prefix| path.split(['#', '&']).find_map(|part|
                part.strip_prefix(prefix).filter(|value| value.len() == 4)
                    .and_then(|value| u16::from_str_radix(value, 16).ok()));
            match (field("vid_"), field("pid_")) {
                (Some(vendor), Some(product)) => vendor == interface.vendor_id && product == interface.product_id,
                _ => true,
            }
        }) { continue; }
        let matches: std::collections::BTreeSet<_> = targets.iter().filter(|other|
            other.backend == target.backend && other.address == target.address
                && other.hid().is_ok_and(|other| other.vendor_id == interface.vendor_id
                    && other.product_id == interface.product_id && other.serial == interface.serial))
            .map(|target| target.channel_key().to_ascii_lowercase()).collect();
        if matches.len() != 1 { continue; }
        let legacy = crate::device::stable_key(&format!("{}:{:04x}:{:04x}:{}:{}",
            target.backend.name(), interface.vendor_id, interface.product_id, interface.serial, target.address));
        migrations.insert(legacy, target.key.clone());
    }
    migrations
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
        BackendId::Dareu => dareu::Dareu.set_dpi_stage(&api, target, stage, context),
        BackendId::GearHub => gearhub::GearHub.set_dpi_stage(&api, target, stage, context),
        BackendId::Aula => aula::Aula.set_dpi_stage(&api, target, stage, context),
        BackendId::MchoseMouse => mchose_mouse::MchoseMouse.set_dpi_stage(&api, target, stage, context),
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
            battery_unavailable: false, battery_at: Some(std::time::Instant::now()), polling_rate: Default::default(),
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
    fn richer_reading_replaces_only_identity_verified_bluetooth_duplicate() {
        let address = ["AA", "BB", "01", "02", "03", "04"].concat();
        let bluetooth_interface = HidInterface {
            path: format!("bluetooth:{address}"),
            serial: address.clone(),
            product: "Example Headphones".into(),
            vendor_id: 0,
            product_id: 0,
            usage_page: 0,
            usage: 0,
            input_len: 0,
            output_len: 0,
            feature_len: 0,
        };
        let standard_interface = HidInterface {
            path: format!("\\\\?\\HID#BTHENUM#7&{address}&0&0000#{{hid}}"),
            serial: String::new(),
            product: "Example Headphones".into(),
            vendor_id: 0x1234,
            product_id: 0x5678,
            usage_page: 1,
            usage: 2,
            input_len: 8,
            output_len: 0,
            feature_len: 8,
        };
        let mut bluetooth = reading(BackendId::Bluetooth);
        bluetooth.target = Target::new(BackendId::Bluetooth, &bluetooth_interface, 0);
        let mut standard = reading(BackendId::StandardHid);
        standard.target = Target::new(BackendId::StandardHid, &standard_interface, 0);

        let result = merge_additional_results([
            (BackendId::StandardHid, Ok(vec![standard.clone()])),
            (BackendId::Bluetooth, Ok(vec![bluetooth])),
        ])
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].target.backend, BackendId::StandardHid);

        standard.target.name = "Other Headphones".into();
        let result = merge_additional_results([
            (BackendId::Bluetooth, Ok(vec![reading(BackendId::Bluetooth)])),
            (BackendId::StandardHid, Ok(vec![standard])),
        ])
        .unwrap();
        assert_eq!(result.len(), 2);
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
    fn unverified_receiver_serials_cannot_merge_independent_devices() {
        for backend in [BackendId::Logitech, BackendId::Vgn] {
            let mut first = reading(backend);
            first.target.hid_mut().serial = "shared-descriptor-serial".into();
            first.target.hid_mut().path = "receiver-a".into();
            first.target = Target::new(backend, first.target.hid().unwrap(), 1);
            let mut second = first.clone();
            second.target.hid_mut().path = "receiver-b".into();
            second.target = Target::new(backend, second.target.hid().unwrap(), 1);
            second.battery.as_mut().unwrap().percent = Some(25);
            assert_ne!(first.target.key, second.target.key);
            let snapshots = read_targets(vec![first.target.clone(), second.target.clone()], |target| {
                Ok(if target.same_channel(&first.target) { first.clone() } else { second.clone() })
            });
            assert_eq!(snapshots.len(), 2);
            assert_eq!(snapshots[0].battery.as_ref().unwrap().percent, Some(75));
            assert_eq!(snapshots[1].battery.as_ref().unwrap().percent, Some(25));
            let fallback = first.target.key.clone();
            first.target.key = "protocol-verified-device".into();
            first.target.verified_identity = true;
            assert_eq!(first.target.unverified_key(), fallback);
            let other_slot = Target::new(backend, first.target.hid().unwrap(), 2);
            assert_ne!(fallback, other_slot.key);
            first.target.hid_mut().path = "RECEIVER-A".into();
            assert_eq!(first.target.unverified_key(), fallback);
        }
    }

    #[test]
    fn bluetooth_and_standard_serial_identity_keys_remain_compatible() {
        for backend in [BackendId::Bluetooth, BackendId::StandardHid, BackendId::Rog] {
            let mut target = reading(backend).target;
            target.hid_mut().serial = "serial-with-punctuation".into();
            let interface = target.hid().unwrap();
            let expected = crate::device::stable_key(&format!("{}:{:04x}:{:04x}:{}:{}",
                backend.name(), interface.vendor_id, interface.product_id, interface.serial, target.address));
            assert_eq!(target.unverified_key(), expected);
        }
    }

    #[test]
    fn expanded_provider_names_preserve_device_preferences_and_history_keys() {
        for (backend, prefix) in [
            (BackendId::Nova, "Arctis Nova"),
            (BackendId::VgnDragonfly3, "VGN Dragonfly3 SE"),
            (BackendId::HyperX, "HyperX Alpha 2"),
        ] {
            let mut target = reading(backend).target;
            target.hid_mut().path = "HID#Existing-Receiver".into();
            target.hid_mut().serial = "shared-receiver-serial".into();
            let expected = crate::device::stable_key(&format!(
                "{prefix}:hid#existing-receiver:{}", target.address));
            assert_eq!(target.unverified_key(), expected);
            assert_eq!(Target::new(backend, target.hid().unwrap(), target.address).key, expected);
            target.hid_mut().path = "HID#Another-Receiver".into();
            assert_ne!(target.unverified_key(), expected);
        }
    }

    #[test]
    fn legacy_preferences_require_all_discovered_receivers_to_be_unambiguous() {
        let mut first = reading(BackendId::Rapoo).target;
        first.hid_mut().serial = "duplicate".into();
        first.hid_mut().path = "receiver-a".into();
        first.key = "first-path-key".into();
        let mut second = first.clone();
        second.hid_mut().path = "receiver-b".into();
        second.key = "second-path-key".into();
        let unique = legacy_key_migrations(&[first.clone()], &[]);
        assert_eq!(unique.len(), 1);
        assert_eq!(unique.values().next().map(String::as_str), Some("first-path-key"));
        assert!(legacy_key_migrations(&[first.clone(), second.clone()], &[]).is_empty());
        second.hid_mut().serial = "different".into();
        assert_eq!(legacy_key_migrations(&[first.clone(), second], &[]).len(), 2);
        let interface = first.hid().unwrap();
        for path in [format!("hid#vid_{:04x}&pid_{:04x}#occupied#hid", interface.vendor_id, interface.product_id),
            "unresolved-interface".into()] {
            let skipped = crate::hid::Skipped { path, reason: "unavailable descriptor".into() };
            assert!(legacy_key_migrations(&[first.clone()], &[skipped]).is_empty());
        }
    }

    #[test]
    fn successful_collection_stops_sibling_reads_after_identity_promotion() {
        let mut first = reading(BackendId::Razer);
        first.target.hid_mut().path = "hid#vid_1532&pid_00c1&mi_00&col01#7&aaaa&0&0001#{hid}".into();
        first.target = Target::new(BackendId::Razer, first.target.hid().unwrap(), 0);
        let mut sibling = first.target.clone();
        sibling.hid_mut().path = sibling.hid().unwrap().path.replace("col01", "col02").replace("0001#", "0002#");
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
        first.target.hid_mut().path = "hid#vid_1532&pid_00c1&mi_00&col01#7&aaaa&0&0001#{hid}".into();
        first.target = Target::new(BackendId::Razer, first.target.hid().unwrap(), 0);
        let mut sibling = first.target.clone();
        sibling.hid_mut().path = sibling.hid().unwrap().path.replace("col01", "col02").replace("0001#", "0002#");
        let mut second = sibling.clone();
        second.hid_mut().path = second.hid().unwrap().path.replace("aaaa", "bbbb");
        let mut calls = 0;
        let snapshots = read_targets(vec![first.target.clone(), sibling, second], |target| {
            calls += 1;
            if calls == 1 { return Err("silent collection".into()); }
            let mut result = first.clone();
            result.target = Target::new(BackendId::Razer, target.hid().unwrap(), target.address);
            Ok(result)
        });
        assert_eq!(calls, 3);
        assert_eq!(snapshots.len(), 2);
        assert_ne!(snapshots[0].target.key, snapshots[1].target.key);
        assert!(!snapshots[0].target.same_channel(&snapshots[1].target));
    }
}
