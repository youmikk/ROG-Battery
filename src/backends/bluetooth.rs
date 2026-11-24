//! Windows-connected Bluetooth devices whose cached battery is exposed as a PnP property.
use super::*;
use crate::win32::{FreeLibrary, GetProcAddress, Handle};
use std::{
    collections::{BTreeSet, HashMap},
    ffi::c_void,
    sync::{Mutex, OnceLock},
    time::Instant,
};
use winapi::shared::guiddef::GUID;

const ERROR_NO_MORE_ITEMS: u32 = 259;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const DIGCF_PRESENT: u32 = 0x0000_0002;
const DIGCF_ALLCLASSES: u32 = 0x0000_0004;
const DIGCF_DEVICEINTERFACE: u32 = 0x0000_0010;
const DEVPROP_TYPE_BYTE: u32 = 3;
const DEVPROP_TYPE_UINT32: u32 = 7;
const DEVPROP_TYPE_GUID: u32 = 13;
const DEVPROP_TYPE_STRING: u32 = 18;
const BLUETOOTH_MAX_NAME_SIZE: usize = 248;
const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0000_0800;
const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;

#[repr(C)]
#[derive(Clone, Copy)]
struct DevPropKey {
    fmtid: GUID,
    pid: u32,
}

const BATTERY: DevPropKey = DevPropKey {
    fmtid: GUID {
        Data1: 0x104e_a319,
        Data2: 0x6ee2,
        Data3: 0x4701,
        Data4: [0xbd, 0x47, 0x8d, 0xdb, 0xf4, 0x25, 0xbb, 0xe5],
    },
    pid: 2,
};

const ADDRESS: DevPropKey = DevPropKey {
    fmtid: GUID {
        Data1: 0x2bd6_7d8b,
        Data2: 0x8beb,
        Data3: 0x48d5,
        Data4: [0x87, 0xe0, 0x6c, 0xda, 0x34, 0x28, 0x04, 0x0a],
    },
    pid: 1,
};

const CONTAINER: DevPropKey = DevPropKey {
    fmtid: GUID {
        Data1: 0x8c7e_d206,
        Data2: 0x3f8a,
        Data3: 0x4827,
        Data4: [0xb3, 0xab, 0xae, 0x9e, 0x1f, 0xae, 0xfc, 0x6c],
    },
    pid: 2,
};

const HID_DEVICE_INTERFACE: crate::win32::Guid = crate::win32::Guid {
    data1: 0x4d1e_55b2,
    data2: 0xf16f,
    data3: 0x11cf,
    data4: [0x88, 0xcb, 0x00, 0x11, 0x11, 0x00, 0x00, 0x30],
};

#[repr(C)]
struct SpDevInfoData {
    cb_size: u32,
    class_guid: GUID,
    dev_inst: u32,
    reserved: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BluetoothSearchParams {
    dw_size: u32,
    return_authenticated: i32,
    return_remembered: i32,
    return_unknown: i32,
    return_connected: i32,
    issue_inquiry: i32,
    timeout_multiplier: u8,
    radio: Handle,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SystemTime {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    milliseconds: u16,
}

#[repr(C)]
struct BluetoothDeviceInfo {
    dw_size: u32,
    address: u64,
    class_of_device: u32,
    connected: i32,
    remembered: i32,
    authenticated: i32,
    last_seen: SystemTime,
    last_used: SystemTime,
    name: [u16; BLUETOOTH_MAX_NAME_SIZE],
}

type FindFirst =
    unsafe extern "system" fn(*const BluetoothSearchParams, *mut BluetoothDeviceInfo) -> Handle;
type FindNext = unsafe extern "system" fn(Handle, *mut BluetoothDeviceInfo) -> i32;
type FindClose = unsafe extern "system" fn(Handle) -> i32;

struct BluetoothApi {
    first: FindFirst,
    next: FindNext,
    close: FindClose,
}

impl BluetoothApi {
    fn load() -> Result<&'static Self, String> {
        static API: OnceLock<Result<BluetoothApi, String>> = OnceLock::new();
        API.get_or_init(|| unsafe {
            for library in ["bthprops.cpl", "irprops.cpl"] {
                let module = LoadLibraryExW(
                    crate::win32::to_wide(library).as_ptr(),
                    core::ptr::null_mut(),
                    LOAD_LIBRARY_SEARCH_SYSTEM32,
                );
                if module.is_null() {
                    continue;
                }
                let first = GetProcAddress(module, b"BluetoothFindFirstDevice\0".as_ptr());
                let next = GetProcAddress(module, b"BluetoothFindNextDevice\0".as_ptr());
                let close = GetProcAddress(module, b"BluetoothFindDeviceClose\0".as_ptr());
                if first.is_null() || next.is_null() || close.is_null() {
                    FreeLibrary(module);
                    continue;
                }
                return Ok(BluetoothApi {
                    first: core::mem::transmute::<*mut c_void, FindFirst>(first),
                    next: core::mem::transmute::<*mut c_void, FindNext>(next),
                    close: core::mem::transmute::<*mut c_void, FindClose>(close),
                });
            }
            Err("Windows Bluetooth API is unavailable".into())
        })
        .as_ref()
        .map_err(Clone::clone)
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(name: *const u16, file: Handle, flags: u32) -> Handle;
}

#[link(name = "setupapi")]
extern "system" {
    fn SetupDiEnumDeviceInfo(device_set: Handle, index: u32, device: *mut SpDevInfoData) -> i32;
    fn SetupDiGetDeviceInstanceIdW(
        device_set: Handle,
        device: *const SpDevInfoData,
        instance: *mut u16,
        size: u32,
        required: *mut u32,
    ) -> i32;
    fn SetupDiGetDevicePropertyW(
        device_set: Handle,
        device: *const SpDevInfoData,
        key: *const DevPropKey,
        kind: *mut u32,
        buffer: *mut u8,
        size: u32,
        required: *mut u32,
        flags: u32,
    ) -> i32;
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Candidate {
    address: String,
    name: String,
    class_of_device: u32,
}

fn normalize_address(value: &str) -> Option<String> {
    let address: String = value.chars().filter(char::is_ascii_hexdigit).collect();
    (address.len() == 12).then(|| address.to_ascii_uppercase())
}

fn first_hex_run(value: &str, minimum: usize) -> Option<String> {
    let mut run = String::new();
    for character in value.chars().chain(std::iter::once('\0')) {
        if character.is_ascii_hexdigit() {
            run.push(character);
        } else {
            if run.len() >= minimum {
                return Some(run[..minimum].to_ascii_uppercase());
            }
            run.clear();
        }
    }
    None
}

fn address_from_id(id: &str) -> Option<String> {
    let upper = id.to_ascii_uppercase();
    if let Some((_, suffix)) = upper.rsplit_once("DEV_") {
        if let Some(address) = first_hex_run(suffix, 12) {
            return Some(address);
        }
    }
    let tail = upper.rsplit(['-', '#', '\\']).next().unwrap_or(&upper);
    normalize_address(tail).or_else(|| first_hex_run(tail, 12))
}

fn connected_devices() -> Result<Vec<Candidate>, String> {
    let api = BluetoothApi::load()?;
    let search = BluetoothSearchParams {
        dw_size: core::mem::size_of::<BluetoothSearchParams>() as u32,
        return_authenticated: 1,
        return_remembered: 1,
        return_connected: 1,
        timeout_multiplier: 1,
        ..Default::default()
    };
    let mut info: BluetoothDeviceInfo = unsafe { core::mem::zeroed() };
    info.dw_size = core::mem::size_of::<BluetoothDeviceInfo>() as u32;
    let find = unsafe { (api.first)(&search, &mut info) };
    if find.is_null() || find == INVALID_HANDLE_VALUE {
        let error = crate::win32::last_error();
        if error == ERROR_NO_MORE_ITEMS {
            return Ok(Vec::new());
        }
        return Err(format!("BluetoothFindFirstDevice failed: {error}"));
    }
    let mut out = Vec::new();
    let mut enumeration_error = None;
    loop {
        if info.connected != 0 {
            let address = format!("{:012X}", info.address & 0x0000_ffff_ffff_ffff);
            let end = info
                .name
                .iter()
                .position(|value| *value == 0)
                .unwrap_or(info.name.len());
            let name = String::from_utf16_lossy(&info.name[..end])
                .trim()
                .to_string();
            if !name.is_empty()
                && !out
                    .iter()
                    .any(|device: &Candidate| device.address == address)
            {
                out.push(Candidate {
                    address,
                    name,
                    class_of_device: info.class_of_device,
                });
            }
        }
        info = unsafe { core::mem::zeroed() };
        info.dw_size = core::mem::size_of::<BluetoothDeviceInfo>() as u32;
        if unsafe { (api.next)(find, &mut info) } == 0 {
            let error = crate::win32::last_error();
            if error != ERROR_NO_MORE_ITEMS {
                enumeration_error = Some(format!("BluetoothFindNextDevice failed: {error}"));
            }
            break;
        }
    }
    unsafe {
        let _ = (api.close)(find);
    }
    if let Some(error) = enumeration_error {
        return Err(error);
    }
    out.sort_by(|a, b| a.address.cmp(&b.address));
    Ok(out)
}

struct DeviceSet(Handle);

impl Drop for DeviceSet {
    fn drop(&mut self) {
        unsafe {
            let _ = crate::win32::SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

fn property(set: Handle, device: &SpDevInfoData, key: &DevPropKey) -> Option<(u32, Vec<u8>)> {
    let mut kind = 0u32;
    let mut required = 0u32;
    unsafe {
        let _ = SetupDiGetDevicePropertyW(
            set,
            device,
            key,
            &mut kind,
            core::ptr::null_mut(),
            0,
            &mut required,
            0,
        );
    }
    if crate::win32::last_error() != ERROR_INSUFFICIENT_BUFFER || required == 0 {
        return None;
    }
    let mut bytes = vec![0u8; required as usize];
    let ok = unsafe {
        SetupDiGetDevicePropertyW(
            set,
            device,
            key,
            &mut kind,
            bytes.as_mut_ptr(),
            required,
            &mut required,
            0,
        )
    };
    if ok == 0 {
        return None;
    }
    bytes.truncate(required as usize);
    Some((kind, bytes))
}

fn guid_text(bytes: &[u8]) -> Option<String> {
    let bytes: [u8; 16] = bytes.try_into().ok()?;
    Some(format!(
        "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        u32::from_ne_bytes(bytes[0..4].try_into().ok()?),
        u16::from_ne_bytes(bytes[4..6].try_into().ok()?),
        u16::from_ne_bytes(bytes[6..8].try_into().ok()?),
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    ))
}

fn container_property(set: Handle, device: &SpDevInfoData) -> Option<String> {
    let (kind, bytes) = property(set, device, &CONTAINER)?;
    (kind == DEVPROP_TYPE_GUID).then_some(())?;
    guid_text(&bytes)
}

fn instance_id(set: Handle, device: &SpDevInfoData) -> Option<String> {
    let mut required = 0u32;
    unsafe {
        let _ = SetupDiGetDeviceInstanceIdW(set, device, core::ptr::null_mut(), 0, &mut required);
    }
    if crate::win32::last_error() != ERROR_INSUFFICIENT_BUFFER || required == 0 {
        return None;
    }
    let mut value = vec![0u16; required as usize];
    let ok = unsafe {
        SetupDiGetDeviceInstanceIdW(set, device, value.as_mut_ptr(), required, &mut required)
    };
    if ok == 0 {
        return None;
    }
    while value.last() == Some(&0) {
        value.pop();
    }
    Some(String::from_utf16_lossy(&value))
}

fn bluetooth_container(address: &str) -> Option<String> {
    let set = unsafe {
        crate::win32::SetupDiGetClassDevsW(
            core::ptr::null(),
            core::ptr::null(),
            core::ptr::null_mut(),
            DIGCF_PRESENT | DIGCF_ALLCLASSES,
        )
    };
    if set.is_null() || set == INVALID_HANDLE_VALUE {
        return None;
    }
    let set = DeviceSet(set);
    let mut containers = BTreeSet::new();
    for index in 0..u32::MAX {
        let mut device: SpDevInfoData = unsafe { core::mem::zeroed() };
        device.cb_size = core::mem::size_of::<SpDevInfoData>() as u32;
        if unsafe { SetupDiEnumDeviceInfo(set.0, index, &mut device) } == 0 {
            break;
        }
        let Some(id) = instance_id(set.0, &device) else {
            continue;
        };
        let id = id.to_ascii_uppercase();
        if !id.starts_with("BTHENUM\\") && !id.starts_with("BTHLE\\") {
            continue;
        }
        let Some(candidate) =
            property_address(set.0, &device).or_else(|| address_from_id(&id))
        else {
            continue;
        };
        if candidate != address {
            continue;
        }
        if let Some(container) = container_property(set.0, &device) {
            containers.insert(container);
        }
    }
    (containers.len() == 1).then(|| containers.into_iter().next().unwrap())
}

fn hid_interface_container(path: &str) -> Option<String> {
    let set = unsafe {
        crate::win32::SetupDiGetClassDevsW(
            &HID_DEVICE_INTERFACE,
            core::ptr::null(),
            core::ptr::null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    };
    if set.is_null() || set == INVALID_HANDLE_VALUE {
        return None;
    }
    let set = DeviceSet(set);
    for index in 0..u32::MAX {
        let mut interface = crate::win32::SpDeviceInterfaceData::default();
        interface.cb_size = core::mem::size_of::<crate::win32::SpDeviceInterfaceData>() as u32;
        if unsafe {
            crate::win32::SetupDiEnumDeviceInterfaces(
                set.0,
                core::ptr::null_mut(),
                &HID_DEVICE_INTERFACE,
                index,
                &mut interface,
            )
        } == 0
        {
            break;
        }
        let mut required = 0u32;
        unsafe {
            let _ = crate::win32::SetupDiGetDeviceInterfaceDetailW(
                set.0,
                &mut interface,
                core::ptr::null_mut(),
                0,
                &mut required,
                core::ptr::null_mut(),
            );
        }
        if crate::win32::last_error() != ERROR_INSUFFICIENT_BUFFER || required == 0 {
            continue;
        }
        let cb_size = if core::mem::size_of::<usize>() == 8 { 8 } else { 6 };
        let mut buffer = vec![0u8; required as usize];
        unsafe {
            core::ptr::write_unaligned(buffer.as_mut_ptr() as *mut u32, cb_size);
        }
        let mut info: SpDevInfoData = unsafe { core::mem::zeroed() };
        info.cb_size = core::mem::size_of::<SpDevInfoData>() as u32;
        if unsafe {
            crate::win32::SetupDiGetDeviceInterfaceDetailW(
                set.0,
                &mut interface,
                buffer.as_mut_ptr() as *mut c_void,
                required,
                core::ptr::null_mut(),
                &mut info as *mut SpDevInfoData as *mut c_void,
            )
        } == 0
        {
            continue;
        }
        let wide: Vec<u16> = buffer[4..]
            .chunks_exact(2)
            .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
            .collect();
        let end = wide.iter().position(|value| *value == 0).unwrap_or(wide.len());
        let candidate = String::from_utf16_lossy(&wide[..end]);
        if candidate.eq_ignore_ascii_case(path) {
            return container_property(set.0, &info);
        }
    }
    None
}

fn property_address(set: Handle, device: &SpDevInfoData) -> Option<String> {
    let (kind, bytes) = property(set, device, &ADDRESS)?;
    if kind != DEVPROP_TYPE_STRING || bytes.len() < 2 || bytes.len() % 2 != 0 {
        return None;
    }
    let mut value: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
        .collect();
    while value.last() == Some(&0) {
        value.pop();
    }
    normalize_address(&String::from_utf16_lossy(&value))
}

fn property_battery(set: Handle, device: &SpDevInfoData) -> Option<u8> {
    let (kind, bytes) = property(set, device, &BATTERY)?;
    decode_battery(kind, &bytes)
}

fn decode_battery(kind: u32, bytes: &[u8]) -> Option<u8> {
    let level = match (kind, bytes) {
        (kind, [value]) if kind == DEVPROP_TYPE_BYTE => *value,
        (kind, bytes) if kind == DEVPROP_TYPE_UINT32 && bytes.len() == 4 => {
            let value = u32::from_ne_bytes(bytes.try_into().ok()?);
            u8::try_from(value).ok()?
        }
        _ => return None,
    };
    (level <= 100).then_some(level)
}

#[derive(Clone, Copy)]
struct TrackedBattery {
    level: u8,
    measured_at: Instant,
}

fn update_battery_times(
    tracked: &mut HashMap<String, TrackedBattery>,
    connected: &BTreeSet<String>,
    batteries: &HashMap<String, u8>,
    now: Instant,
) -> HashMap<String, Instant> {
    tracked.retain(|address, _| connected.contains(address));
    let mut times = HashMap::new();
    for (address, level) in batteries {
        let entry = tracked.entry(address.clone()).or_insert(TrackedBattery {
            level: *level,
            measured_at: now,
        });
        if entry.level != *level {
            entry.level = *level;
            entry.measured_at = now;
        }
        times.insert(address.clone(), entry.measured_at);
    }
    times
}

fn cached_battery_times(
    connected: &BTreeSet<String>,
    batteries: &HashMap<String, u8>,
    now: Instant,
) -> HashMap<String, Instant> {
    static TRACKED: OnceLock<Mutex<HashMap<String, TrackedBattery>>> = OnceLock::new();
    let mut tracked = TRACKED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    update_battery_times(&mut tracked, connected, batteries, now)
}

fn unambiguous_batteries(
    levels: HashMap<String, BTreeSet<u8>>,
) -> HashMap<String, u8> {
    levels
        .into_iter()
        .filter_map(|(address, levels)| {
            (levels.len() == 1).then(|| (address, *levels.first().unwrap()))
        })
        .collect()
}

fn bluetooth_address(target: &Target) -> Option<String> {
    if target.backend != BackendId::Bluetooth {
        return None;
    }
    let interface = target.hid().ok()?;
    normalize_address(&interface.serial).or_else(|| {
        interface
            .path
            .strip_prefix("bluetooth:")
            .and_then(normalize_address)
    })
}

fn compact_hex(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_hexdigit)
        .map(|character| character.to_ascii_uppercase())
        .collect()
}

fn standard_hid_matches_address(interface: &HidInterface, address: &str) -> bool {
    normalize_address(&interface.serial).as_deref() == Some(address)
        || compact_hex(&interface.path).contains(address)
}

/// Windows can expose one Bluetooth peripheral through several HID and system
/// battery routes. Prefer the PnP container ID; retain the address/name check
/// as a conservative fallback for older drivers that do not expose it.
pub(crate) fn same_physical_device(first: &Target, second: &Target) -> bool {
    let (bluetooth, standard) = match (first.backend, second.backend) {
        (BackendId::Bluetooth, BackendId::StandardHid) => (first, second),
        (BackendId::StandardHid, BackendId::Bluetooth) => (second, first),
        _ => return false,
    };
    let same_name = bluetooth
        .name
        .trim()
        .eq_ignore_ascii_case(standard.name.trim());
    let Some(address) = bluetooth_address(bluetooth) else {
        return false;
    };
    if let (Some(bluetooth_container), Ok(interface)) =
        (bluetooth_container(&address), standard.hid())
    {
        if hid_interface_container(&interface.path)
            .is_some_and(|container| container == bluetooth_container)
        {
            return true;
        }
    }
    if !same_name || standard.backend != BackendId::StandardHid {
        return false;
    }
    standard
        .hid()
        .is_ok_and(|interface| standard_hid_matches_address(interface, &address))
}

fn cached_batteries() -> Result<HashMap<String, u8>, String> {
    let set = unsafe {
        crate::win32::SetupDiGetClassDevsW(
            core::ptr::null(),
            core::ptr::null(),
            core::ptr::null_mut(),
            DIGCF_PRESENT | DIGCF_ALLCLASSES,
        )
    };
    if set.is_null() || set == INVALID_HANDLE_VALUE {
        return Err(format!(
            "SetupDiGetClassDevsW failed: {}",
            crate::win32::last_error()
        ));
    }
    let set = DeviceSet(set);
    let mut batteries: HashMap<String, BTreeSet<u8>> = HashMap::new();
    for index in 0..u32::MAX {
        let mut device: SpDevInfoData = unsafe { core::mem::zeroed() };
        device.cb_size = core::mem::size_of::<SpDevInfoData>() as u32;
        if unsafe { SetupDiEnumDeviceInfo(set.0, index, &mut device) } == 0 {
            let error = crate::win32::last_error();
            if error != ERROR_NO_MORE_ITEMS {
                return Err(format!("SetupDiEnumDeviceInfo failed: {error}"));
            }
            break;
        }
        let Some(id) = instance_id(set.0, &device) else {
            continue;
        };
        let id = id.to_ascii_uppercase();
        if !id.starts_with("BTHENUM\\") && !id.starts_with("BTHLE\\") {
            continue;
        }
        let Some(address) = property_address(set.0, &device).or_else(|| address_from_id(&id))
        else {
            continue;
        };
        if let Some(level) = property_battery(set.0, &device) {
            batteries.entry(address).or_default().insert(level);
        }
    }
    Ok(unambiguous_batteries(batteries))
}

fn family_of(name: &str, class_of_device: u32) -> crate::device::DeviceFamily {
    use crate::device::DeviceFamily;
    let major = ((class_of_device >> 8) & 0x1f) as u8;
    let minor = ((class_of_device >> 2) & 0x3f) as u8;
    match major {
        4 => DeviceFamily::Headset,
        5 if minor & 0x10 != 0 => DeviceFamily::Keyboard,
        5 if minor & 0x20 != 0 => DeviceFamily::Mouse,
        5 if minor == 0x01 || minor == 0x02 => DeviceFamily::Gamepad,
        _ => crate::device::family_of(name),
    }
}

fn target(candidate: &Candidate) -> Target {
    let interface = HidInterface {
        path: format!("bluetooth:{}", candidate.address),
        serial: candidate.address.clone(),
        product: candidate.name.clone(),
        vendor_id: 0,
        product_id: 0,
        usage_page: 0,
        usage: 0,
        input_len: 0,
        output_len: 0,
        feature_len: 0,
    };
    let mut target = Target::new(BackendId::Bluetooth, &interface, 0);
    target.family = family_of(&candidate.name, candidate.class_of_device);
    target.verified_identity = true;
    target
}

pub fn poll() -> Result<Poll, String> {
    let connected = connected_devices()?;
    let batteries = cached_batteries()?;
    let now = Instant::now();
    let connected_addresses: BTreeSet<String> =
        connected.iter().map(|device| device.address.clone()).collect();
    let battery_times = cached_battery_times(&connected_addresses, &batteries, now);
    let mut present = BTreeSet::new();
    let mut snapshots = Vec::new();
    for device in connected {
        let target = target(&device);
        present.insert(target.channel_key());
        let Some(level) = batteries.get(&device.address).copied() else {
            continue;
        };
        snapshots.push(Snapshot {
            target,
            battery: Some(Battery {
                percent: Some(level),
                estimated_percent: None,
                charging: None,
                voltage_mv: None,
                approximate: false,
                level: None,
            }),
            dpi: Dpi::default(),
            polling_rate: Default::default(),
            connected: true,
            battery_at: battery_times.get(&device.address).copied(),
            battery_unavailable: false,
        });
    }
    Ok(Poll {
        snapshots,
        present_channels: Some(present),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bluetooth_addresses_accept_separators_and_instance_suffixes() {
        assert_eq!(
            normalize_address("AABB-0102-0304"),
            Some("AABB01020304".into())
        );
        assert_eq!(
            address_from_id("Bluetooth#7-AABB01020304"),
            Some("AABB01020304".into())
        );
        assert_eq!(
            address_from_id("BTHENUM\\DEV_AABB01020304&0000"),
            Some("AABB01020304".into())
        );
        assert_eq!(normalize_address("AABB0102030"), None);
    }

    #[test]
    fn targets_are_stable_connected_bluetooth_devices_without_write_capabilities() {
        let candidate = Candidate {
            address: ["AA", "BB", "01", "02", "03", "04"].concat(),
            name: "Example Headphones".into(),
            class_of_device: 0,
        };
        let first = target(&candidate);
        let second = target(&candidate);
        assert_eq!(first.key, second.key);
        assert_eq!(first.backend, BackendId::Bluetooth);
        assert_eq!(first.family, DeviceFamily::Headset);
        assert_eq!(
            first.channel_key(),
            format!("bluetooth:{}", candidate.address)
        );
        assert!(first.verified_identity);
    }

    #[test]
    fn cached_bluetooth_battery_decoder_accepts_only_bounded_windows_types() {
        assert_eq!(decode_battery(DEVPROP_TYPE_BYTE, &[0]), Some(0));
        assert_eq!(decode_battery(DEVPROP_TYPE_BYTE, &[100]), Some(100));
        assert_eq!(
            decode_battery(DEVPROP_TYPE_UINT32, &75u32.to_ne_bytes()),
            Some(75)
        );

        assert_eq!(decode_battery(DEVPROP_TYPE_BYTE, &[101]), None);
        assert_eq!(decode_battery(DEVPROP_TYPE_BYTE, &[50, 0]), None);
        assert_eq!(
            decode_battery(DEVPROP_TYPE_UINT32, &101u32.to_ne_bytes()),
            None
        );
        assert_eq!(decode_battery(DEVPROP_TYPE_UINT32, &[50, 0]), None);
        assert_eq!(decode_battery(DEVPROP_TYPE_STRING, &[50]), None);
    }

    #[test]
    fn unchanged_cached_reading_keeps_its_original_measurement_time() {
        let address = ["AA", "BB", "01", "02", "03", "04"].concat();
        let connected = BTreeSet::from([address.clone()]);
        let mut batteries = HashMap::from([(address.clone(), 50)]);
        let mut tracked = HashMap::new();
        let start = Instant::now();
        assert_eq!(
            update_battery_times(&mut tracked, &connected, &batteries, start).get(&address),
            Some(&start)
        );
        let later = start + std::time::Duration::from_secs(30);
        assert_eq!(
            update_battery_times(&mut tracked, &connected, &batteries, later).get(&address),
            Some(&start)
        );

        batteries.insert(address.clone(), 49);
        let changed = later + std::time::Duration::from_secs(30);
        assert_eq!(
            update_battery_times(&mut tracked, &connected, &batteries, changed).get(&address),
            Some(&changed)
        );

        batteries.clear();
        assert!(update_battery_times(&mut tracked, &connected, &batteries, changed).is_empty());
        assert!(tracked.contains_key(&address));
        update_battery_times(&mut tracked, &BTreeSet::new(), &batteries, changed);
        assert!(!tracked.contains_key(&address));
    }

    #[test]
    fn conflicting_cached_levels_for_one_address_remain_unknown() {
        let address = ["AA", "BB", "01", "02", "03", "04"].concat();
        let consistent = unambiguous_batteries(HashMap::from([(
            address.clone(),
            BTreeSet::from([40]),
        )]));
        assert_eq!(consistent.get(&address), Some(&40));

        let conflicting = unambiguous_batteries(HashMap::from([(
            address.clone(),
            BTreeSet::from([40, 41]),
        )]));
        assert!(!conflicting.contains_key(&address));
    }

    #[test]
    fn windows_container_guid_uses_native_field_endianness() {
        let bytes = [
            0x06, 0xd2, 0x7e, 0x8c, 0x8a, 0x3f, 0x27, 0x48, 0xb3, 0xab, 0xae,
            0x9e, 0x1f, 0xae, 0xfc, 0x6c,
        ];
        assert_eq!(
            guid_text(&bytes).as_deref(),
            Some("8C7ED206-3F8A-4827-B3AB-AE9E1FAEFC6C")
        );
        assert_eq!(guid_text(&bytes[..15]), None);
    }

    #[test]
    fn standard_hid_duplicate_requires_matching_name_and_bluetooth_address() {
        let address = ["AA", "BB", "01", "02", "03", "04"].concat();
        let bluetooth_target = target(&Candidate {
            address: address.clone(),
            name: "Example Headphones".into(),
            class_of_device: 0,
        });
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
        let standard_target = Target::new(BackendId::StandardHid, &standard_interface, 0);
        assert!(same_physical_device(&bluetooth_target, &standard_target));

        let mut different_name = standard_target.clone();
        different_name.name = "Other Headphones".into();
        assert!(!same_physical_device(&bluetooth_target, &different_name));

        let unrelated_interface = HidInterface {
            path: "\\\\?\\HID#VID_1234&PID_5678#7&FFFFFFFFFFFF&0&0000#{hid}".into(),
            ..standard_interface
        };
        let unrelated = Target::new(BackendId::StandardHid, &unrelated_interface, 0);
        assert!(!same_physical_device(&bluetooth_target, &unrelated));
    }

    #[test]
    fn bluetooth_class_selects_device_type_before_name_fallback() {
        assert_eq!(
            family_of("MX Master 3S", 0x0005_80),
            crate::device::DeviceFamily::Mouse
        );
        assert_eq!(
            family_of("Generic keys", 0x0005_40),
            crate::device::DeviceFamily::Keyboard
        );
        assert_eq!(
            family_of("Pro Controller", 0x0005_08),
            crate::device::DeviceFamily::Gamepad
        );
        assert_eq!(
            family_of("Generic audio", 0x0004_00),
            crate::device::DeviceFamily::Headset
        );
        assert_eq!(
            family_of("Example Headphones", 0),
            crate::device::DeviceFamily::Headset
        );
    }
}
