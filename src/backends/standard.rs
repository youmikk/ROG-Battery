//! HID Usage Tables: Generic Device Controls / Battery Strength (06:20).
//! 只读取描述符公开的电量字段；不把毫伏或剩余容量当百分比。
use super::*;
use crate::win32::*;
use crate::device::BatteryPercent;
use std::{collections::{BTreeMap, BTreeSet}, sync::{Arc, Mutex, OnceLock}, time::Instant};
use winapi::shared::hidpi::HIDP_VALUE_CAPS;
use winapi::um::cfgmgr32::{CM_Get_Device_IDW, CM_Get_Parent, CR_SUCCESS, MAX_DEVICE_ID_LEN};

pub struct StandardHid;

struct Descriptor {
    handle: Handle,
    preparsed: *mut core::ffi::c_void,
    api: HidApi,
}

impl Descriptor {
    fn open(api: &HidApi, interface: &HidInterface) -> Result<Self, String> {
        let path = to_wide(&interface.path);
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                core::ptr::null_mut(),
                OPEN_EXISTING,
                0,
                core::ptr::null_mut(),
            )
        };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err("HID 描述符不可读取".into());
        }
        let mut preparsed = core::ptr::null_mut();
        if unsafe { (api.get_preparsed_data)(handle, &mut preparsed) } == 0 {
            unsafe {
                CloseHandle(handle);
            }
            return Err("HID 描述符不可解析".into());
        }
        Ok(Self {
            handle,
            preparsed,
            api: *api,
        })
    }

    fn battery_caps(&self, report_type: u32) -> Vec<HIDP_VALUE_CAPS> {
        let mut caps = HidpCaps::default();
        if unsafe { (self.api.get_caps)(self.preparsed, &mut caps) } < 0 {
            return Vec::new();
        }
        let mut length = if report_type == 2 {
            caps.number_feature_value_caps
        } else {
            caps.number_input_value_caps
        };
        if length == 0 || length > 512 {
            return Vec::new();
        }
        let mut values: Vec<HIDP_VALUE_CAPS> = (0..length)
            .map(|_| unsafe { core::mem::zeroed() })
            .collect();
        if unsafe {
            (self.api.get_value_caps)(
                report_type,
                values.as_mut_ptr(),
                &mut length,
                self.preparsed,
            )
        } < 0
        {
            return Vec::new();
        }
        values.truncate(usize::from(length));
        values.retain(|value| {
            let contains = unsafe {
                if value.IsRange != 0 {
                    let range = value.u.Range();
                    range.UsageMin <= 0x20 && range.UsageMax >= 0x20
                } else {
                    value.u.NotRange().Usage == 0x20
                }
            };
            value.UsagePage == 6
                && contains
                && value.ReportCount == 1
                && value.LogicalMin >= 0
                && value.LogicalMax > value.LogicalMin
        });
        values
    }

    fn percent(&self, report_type: u32, cap: &HIDP_VALUE_CAPS, report: &mut [u8]) -> Option<BatteryPercent> {
        if report.first().copied() != Some(cap.ReportID) {
            return None;
        }
        let mut value = 0;
        let status = unsafe {
            (self.api.get_usage_value)(
                report_type,
                6,
                cap.LinkCollection,
                0x20,
                &mut value,
                self.preparsed,
                report.as_mut_ptr(),
                report.len() as u32,
            )
        };
        if status < 0 {
            return None;
        }
        normalize_percent(value, cap.LogicalMin, cap.LogicalMax)
    }
}

impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            (self.api.free_preparsed_data)(self.preparsed);
            CloseHandle(self.handle);
        }
    }
}

fn normalize_percent(value: u32, minimum: i32, maximum: i32) -> Option<BatteryPercent> {
    if minimum < 0 || maximum <= minimum || value < minimum as u32 || value > maximum as u32 {
        return None;
    }
    let span = (maximum - minimum) as u64;
    Some(BatteryPercent { value: ((u64::from(value - minimum as u32) * 100 + span / 2) / span) as u8,
        approximate: minimum != 0 || maximum != 100 })
}

fn battery(value: BatteryPercent) -> Battery {
    // Keep descriptor-scaled strengths separate from native percentages. In
    // particular a 0..4 descriptor supplies five levels, not exact 25% steps.
    Battery { percent: (!value.approximate).then_some(value.value),
        estimated_percent: value.approximate.then_some(value.value),
        charging: None, voltage_mv: None, approximate: value.approximate, level: None }
}

fn reserved_by_rog(interface: &HidInterface) -> bool {
    interface.vendor_id == crate::hid::VID_ASUS
        && (crate::devices::find(interface.product_id).is_some()
            || crate::devices::find_dongle(interface.product_id).is_some())
}

#[link(name = "setupapi")]
extern "system" {
    fn SetupDiCreateDeviceInfoList(class: *const Guid, parent: Handle) -> Handle;
    fn SetupDiOpenDeviceInterfaceW(set: Handle, path: *const u16, flags: u32,
        interface: *mut SpDeviceInterfaceData) -> i32;
}

fn interface_devnode(path: &str) -> Option<u32> {
    if path.is_empty() || path.contains('\0') { return None; }
    let set = unsafe { SetupDiCreateDeviceInfoList(core::ptr::null(), core::ptr::null_mut()) };
    if set.is_null() || set == INVALID_HANDLE_VALUE { return None; }
    let result = (|| {
        let mut interface = SpDeviceInterfaceData::default();
        if unsafe { SetupDiOpenDeviceInterfaceW(set, to_wide(path).as_ptr(), 0, &mut interface) } == 0
            || interface.flags & 1 == 0 { return None; }
        let mut device = SpDevInfoData::default();
        let status = unsafe { SetupDiGetDeviceInterfaceDetailW(set, &mut interface,
            core::ptr::null_mut(), 0, core::ptr::null_mut(),
            (&mut device as *mut SpDevInfoData).cast()) };
        (status == 0 && last_error() == winapi::shared::winerror::ERROR_INSUFFICIENT_BUFFER)
            .then_some(device.dev_inst)
    })();
    unsafe { SetupDiDestroyDeviceInfoList(set); }
    result
}

fn usb_group_with(interface: &HidInterface, mut instance: u32,
    mut device_id: impl FnMut(u32) -> Option<String>,
    mut parent: impl FnMut(u32) -> Option<u32>) -> Option<String> {
    let expected = format!("vid_{:04x}&pid_{:04x}", interface.vendor_id, interface.product_id);
    let mut visited = BTreeSet::new();
    for _ in 0..32 {
        if !visited.insert(instance) { return None; }
        let identity = device_id(instance)?.to_ascii_lowercase();
        let parts: Vec<_> = identity.split('\\').collect();
        if parts.len() != 3 || parts[2].is_empty() || identity.contains('\0') { return None; }
        match parts[0] {
            "usb" if parts[1] == expected => return Some(identity),
            "usb" => {
                let suffix = parts[1].strip_prefix(&format!("{expected}&mi_"))?;
                if suffix.len() != 2 || !suffix.bytes().all(|value| value.is_ascii_hexdigit()) {
                    return None;
                }
            }
            "hid" => (),
            _ => return None,
        }
        instance = parent(instance)?;
    }
    None
}

fn usb_group(interface: &HidInterface) -> Option<String> {
    if interface.serial.is_empty() { return None; }
    usb_group_with(interface, interface_devnode(&interface.path)?, |instance| {
        let mut buffer = [0u16; MAX_DEVICE_ID_LEN];
        if unsafe { CM_Get_Device_IDW(instance, buffer.as_mut_ptr(), buffer.len() as u32, 0) } != CR_SUCCESS {
            return None;
        }
        let end = buffer.iter().position(|&unit| unit == 0)?;
        String::from_utf16(&buffer[..end]).ok()
    }, |instance| {
        let mut parent = 0;
        (unsafe { CM_Get_Parent(&mut parent, instance, 0) } == CR_SUCCESS).then_some(parent)
    })
}

fn target_for(interface: &HidInterface, physical_group: Option<&str>) -> Target {
    let mut target = Target::new(BackendId::StandardHid, interface, 0);
    let identity = physical_group.filter(|_| !interface.serial.is_empty())
        .map_or_else(|| format!("path:{}", interface.path.to_ascii_lowercase()),
            |group| format!("physical:{}:{}:serial:{}:{}", group.len(), group.to_ascii_lowercase(),
                interface.serial.len(), interface.serial));
    target.key = crate::device::stable_key(&format!("Standard HID:{:04x}:{:04x}:{identity}",
        interface.vendor_id, interface.product_id));
    target
}

type Presence = BTreeMap<String, Arc<()>>;
type Sample = (BatteryPercent, Instant);

#[derive(Default)]
struct CachedDevice {
    generation: Arc<()>,
    members: BTreeSet<String>,
    sample: Option<Sample>,
}

#[derive(Default)]
struct Cache {
    presence: Presence,
    devices: BTreeMap<String, CachedDevice>,
}

struct ReadLease {
    key: String,
    path: String,
    presence: Arc<()>,
    generation: Arc<()>,
}

impl Cache {
    fn observe(&mut self, report: &crate::hid::EnumerateReport) -> Presence {
        let paths: BTreeSet<_> = report.interfaces.iter().map(|interface| interface.path.to_ascii_lowercase())
            .chain(report.skipped.iter().map(|entry| entry.path.to_ascii_lowercase())).collect();
        self.presence.retain(|path, _| paths.contains(path));
        for path in paths { self.presence.entry(path).or_default(); }
        self.devices.retain(|_, device| {
            device.members.retain(|path| self.presence.contains_key(path));
            !device.members.is_empty()
        });
        self.presence.clone()
    }

    fn register(&mut self, target: &Target, presence: &Arc<()>) -> bool {
        let path = target.channel_key().to_ascii_lowercase();
        if !self.presence.get(&path).is_some_and(|current| Arc::ptr_eq(current, presence)) { return false; }
        self.devices.retain(|key, device| {
            if *key != target.key { device.members.remove(&path); }
            !device.members.is_empty()
        });
        self.devices.entry(target.key.clone()).or_default().members.insert(path);
        true
    }

    fn lease(&self, target: &Target) -> Result<ReadLease, String> {
        let path = target.channel_key().to_ascii_lowercase();
        let device = self.devices.get(&target.key).filter(|device| device.members.contains(&path))
            .ok_or("HID 设备身份需要重新枚举")?;
        Ok(ReadLease { key: target.key.clone(), path: path.clone(),
            presence: self.presence.get(&path).ok_or("HID 接口已移除")?.clone(),
            generation: device.generation.clone() })
    }

    fn sample(&mut self, lease: &ReadLease, percent: Option<BatteryPercent>, now: Instant)
        -> Result<Option<Sample>, String> {
        if !self.presence.get(&lease.path).is_some_and(|current| Arc::ptr_eq(current, &lease.presence)) {
            return Err("HID 接口在读取期间重新枚举".into());
        }
        let device = self.devices.get_mut(&lease.key).filter(|device|
            Arc::ptr_eq(&device.generation, &lease.generation) && device.members.contains(&lease.path))
            .ok_or("HID 设备身份在读取期间改变")?;
        if let Some(percent) = percent {
            if device.sample.is_none_or(|(_, taken)| now >= taken) { device.sample = Some((percent, now)); }
        }
        device.sample = device.sample.filter(|(_, taken)|
            now.saturating_duration_since(*taken) < BackendId::StandardHid.sample_lifetime());
        Ok(device.sample)
    }
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn observe_presence_with(cache: &Mutex<Cache>,
    scan: impl FnOnce() -> Result<crate::hid::EnumerateReport, String>) -> Result<Presence, String> {
    let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
    Ok(cache.observe(&scan()?))
}

fn complete_read(cache: &mut Cache, lease: &ReadLease, target: &Target,
    percent: Option<BatteryPercent>, now: Instant) -> Result<Snapshot, String> {
    let (percent, measured) = cache.sample(lease, percent, now)?.ok_or("设备无有效电量回报")?;
    Ok(Snapshot {
        target: target.clone(), connected: true, battery_unavailable: false,
        battery_at: Some(measured), polling_rate: Default::default(), dpi: Dpi::default(),
        battery: Some(battery(percent)),
    })
}

impl DeviceBackend for StandardHid {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        let presence = observe_presence_with(cache(), || enumeration::scan(api))?;
        let mut targets = Vec::new();
        for interface in interfaces {
            if reserved_by_rog(interface) {
                continue;
            }
            let Ok(descriptor) = Descriptor::open(api, interface) else {
                continue;
            };
            if !descriptor.battery_caps(2).is_empty() || !descriptor.battery_caps(0).is_empty() {
                let target = target_for(interface, usb_group(interface).as_deref());
                let Some(epoch) = presence.get(&interface.path.to_ascii_lowercase()) else { continue; };
                if cache().lock().unwrap_or_else(|error| error.into_inner()).register(&target, epoch) {
                    targets.push(target);
                }
            }
        }
        Ok(targets)
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let lease = cache().lock().unwrap_or_else(|error| error.into_inner()).lease(target)?;
        let interface = target.hid()?;
        if target_for(interface, usb_group(interface).as_deref()).key != target.key {
            return Err("HID 设备身份需要重新枚举".into());
        }
        let descriptor = Descriptor::open(api, target.hid()?)?;
        let mut percent = None;
        for cap in descriptor.battery_caps(2) {
            let started = std::time::Instant::now();
            if let Some(mut report) =
                crate::hid::query_report(target.hid()?, cap.ReportID, true, 160)
            {
                percent = descriptor.percent(2, &cap, &mut report);
                trace::record(
                    target.hid()?,
                    BackendId::StandardHid,
                    "feature-report",
                    0x0620,
                    cap.ReportID,
                    target.hid()?.feature_len as usize,
                    report.len(),
                    started,
                    if percent.is_some() {
                        trace::Outcome::Ok
                    } else {
                        trace::Outcome::InvalidData
                    },
                );
                if percent.is_some() {
                    break;
                }
            } else {
                trace::record(
                    target.hid()?,
                    BackendId::StandardHid,
                    "feature-report",
                    0x0620,
                    cap.ReportID,
                    target.hid()?.feature_len as usize,
                    0,
                    started,
                    trace::Outcome::NoReply,
                );
            }
        }
        if percent.is_none() {
            let caps = descriptor.battery_caps(0);
            for cap in &caps {
                let started = std::time::Instant::now();
                if let Some(mut report) =
                    crate::hid::query_report(target.hid()?, cap.ReportID, false, 160)
                {
                    percent = descriptor.percent(0, cap, &mut report);
                    trace::record(
                        target.hid()?,
                        BackendId::StandardHid,
                        "input-report",
                        0x0620,
                        cap.ReportID,
                        target.hid()?.input_len as usize,
                        report.len(),
                        started,
                        if percent.is_some() {
                            trace::Outcome::Ok
                        } else {
                            trace::Outcome::InvalidData
                        },
                    );
                    if percent.is_some() {
                        break;
                    }
                } else {
                    trace::record(
                        target.hid()?,
                        BackendId::StandardHid,
                        "input-report",
                        0x0620,
                        cap.ReportID,
                        target.hid()?.input_len as usize,
                        0,
                        started,
                        trace::Outcome::NoReply,
                    );
                }
            }
            if percent.is_none() && !caps.is_empty() {
                if let Ok(mut device) = crate::hid::HidDevice::open_read_only(target.hid()?) {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(80);
                    while let Some(left) =
                        deadline.checked_duration_since(std::time::Instant::now())
                    {
                        let Some(mut report) = device.read_report(left.as_millis().max(1) as u32)
                        else {
                            break;
                        };
                        percent = caps
                            .iter()
                            .find_map(|cap| descriptor.percent(0, cap, &mut report));
                        if percent.is_some() {
                            break;
                        }
                    }
                }
            }
        }
        if target_for(interface, usb_group(interface).as_deref()).key != target.key {
            return Err("HID 设备身份在读取期间改变".into());
        }
        complete_read(&mut cache().lock().unwrap_or_else(|error| error.into_inner()),
            &lease, target, percent, Instant::now())
    }
}

#[cfg(test)]
pub(crate) fn snapshot_for_test(
    interface: &HidInterface,
    physical_group: Option<&str>,
    percent: u8,
    now: Instant,
) -> Snapshot {
    let target = target_for(interface, physical_group);
    let mut cache = Cache::default();
    let presence = cache.observe(&crate::hid::EnumerateReport {
        interfaces: vec![interface.clone()], skipped: Vec::new(),
    });
    assert!(cache.register(&target, &presence[&interface.path.to_ascii_lowercase()]));
    let lease = cache.lease(&target).expect("test target must be registered");
    let percent = normalize_percent(u32::from(percent), 0, 100)
        .expect("test battery percentage must be within 0..=100");
    complete_read(&mut cache, &lease, &target, Some(percent), now)
        .expect("test read must produce a fresh snapshot")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(path: &str) -> HidInterface {
        HidInterface {
            path: path.into(), serial: "DUPLICATE-FULL-SERIAL".into(), product: "HID battery".into(),
            vendor_id: 0x1234, product_id: 0x5678, usage_page: 1, usage: 2,
            input_len: 8, output_len: 0, feature_len: 8,
        }
    }

    #[test]
    fn duplicate_serials_use_distinct_physical_keys_and_share_only_proven_siblings() {
        let first = target_for(&interface("receiver-a-col01"), Some(r"usb\vid_1234&pid_5678\first"));
        let sibling = target_for(&interface("receiver-a-col02"), Some(r"usb\vid_1234&pid_5678\first"));
        let second = target_for(&interface("receiver-b-col01"), Some(r"usb\vid_1234&pid_5678\second"));
        assert_eq!(first.key, sibling.key);
        assert_ne!(first.key, second.key);
    }

    #[test]
    fn logical_slots_with_one_usb_parent_keep_full_serials_or_exact_paths() {
        let parent = Some(r"usb\vid_1234&pid_5678\shared-receiver");
        let mut first = interface("slot-a-col01");
        first.serial = "ABC-1".into();
        let mut sibling = first.clone();
        sibling.path = "slot-a-col02".into();
        assert_eq!(target_for(&first, parent).key, target_for(&sibling, parent).key);
        let mut second = interface("slot-b-col01");
        for serial in ["ABC-2", "abc-1", "ABC1", " ABC-1 "] {
            second.serial = serial.into();
            assert_ne!(target_for(&first, parent).key, target_for(&second, parent).key);
        }
        first.serial.clear();
        second.serial.clear();
        assert_ne!(target_for(&first, parent).key, target_for(&second, parent).key);
        assert_eq!(target_for(&first, parent).key, target_for(&first, None).key);
        first.path = second.path.to_ascii_uppercase();
        assert_eq!(target_for(&first, parent).key, target_for(&second, parent).key);
    }

    #[test]
    fn cache_does_not_share_samples_between_distinct_serials_on_one_receiver() {
        let mut cache = Cache::default();
        cache.observe(&report(&["slot-a", "slot-b"], &[]));
        let mut first = interface("slot-a");
        let mut second = interface("slot-b");
        first.serial = "ABC-1".into();
        second.serial = "ABC-2".into();
        let parent = Some(r"usb\vid_1234&pid_5678\shared-receiver");
        let first = target_for(&first, parent);
        let second = target_for(&second, parent);
        let first_lease = register(&mut cache, &first);
        let second_lease = register(&mut cache, &second);
        let now = Instant::now();
        let first_percent = normalize_percent(18, 0, 100).unwrap();
        cache.sample(&first_lease, Some(first_percent), now).unwrap();
        assert_eq!(cache.sample(&second_lease, None, now).unwrap(), None);
        cache.sample(&second_lease, normalize_percent(88, 0, 100), now).unwrap();
        assert_eq!(cache.sample(&first_lease, None, now).unwrap(), Some((first_percent, now)));
    }

    fn report(paths: &[&str], skipped: &[&str]) -> crate::hid::EnumerateReport {
        crate::hid::EnumerateReport {
            interfaces: paths.iter().map(|path| interface(path)).collect(),
            skipped: skipped.iter().map(|path| crate::hid::Skipped {
                path: (*path).into(), reason: "descriptor unavailable".into(),
            }).collect(),
        }
    }

    fn register(cache: &mut Cache, target: &Target) -> ReadLease {
        let presence = cache.presence[&target.channel_key().to_ascii_lowercase()].clone();
        assert!(cache.register(target, &presence));
        cache.lease(target).unwrap()
    }

    #[test]
    fn actual_usb_ancestry_joins_collections_without_merging_equal_serial_devices() {
        let identities: BTreeMap<u32, &str> = [
            (1, r"HID\VID_1234&PID_5678&MI_00&COL01\first-col01"),
            (2, r"HID\VID_1234&PID_5678&MI_00&COL02\first-col02"),
            (3, r"USB\VID_1234&PID_5678&MI_00\first-interface"),
            (4, r"USB\VID_1234&PID_5678\first"),
            (5, r"HID\VID_1234&PID_5678&COL01\second-col01"),
            (6, r"USB\VID_1234&PID_5678\second"),
        ].into_iter().collect();
        let parents: BTreeMap<u32, u32> = [(1, 3), (2, 3), (3, 4), (5, 6)].into_iter().collect();
        let resolve = |node| usb_group_with(&interface("unused"), node,
            |node| identities.get(&node).map(|identity| (*identity).into()),
            |node| parents.get(&node).copied());
        assert_eq!(resolve(1), resolve(2));
        assert_eq!(resolve(1).as_deref(), Some(r"usb\vid_1234&pid_5678\first"));
        assert_eq!(resolve(5).as_deref(), Some(r"usb\vid_1234&pid_5678\second"));
        assert_ne!(resolve(1), resolve(5));
    }

    #[test]
    fn uncertain_ancestry_never_climbs_to_a_bluetooth_radio_or_unrelated_usb_parent() {
        for identity in [r"USB\VID_1234&PID_9999\wrong-product",
            r"USB\VID_1234&PID_5678&MI_00&extra\malformed",
            r"BTHENUM\DEV_112233445566\remote", r"BTHLEDEVICE\unknown\remote",
            r"USB\VID_1234&PID_5678\", "USB\\VID_1234&PID_5678\\bad\0"] {
            assert!(usb_group_with(&interface("unused"), 1, |_| Some(identity.into()),
                |_| panic!("uncertain ancestors must not be skipped")).is_none());
        }
        assert!(usb_group_with(&interface("unused"), 1, |_| None, |_| Some(2)).is_none());
        assert!(usb_group_with(&interface("unused"), 1,
            |_| Some(r"HID\VID_1234&PID_5678\collection".into()), |_| Some(1)).is_none());
        let mut visits = 0;
        assert!(usb_group_with(&interface("unused"), 1, |_| {
            visits += 1;
            Some(r"HID\VID_1234&PID_5678\collection".into())
        }, |node| Some(node + 1)).is_none());
        assert_eq!(visits, 32);
    }

    #[test]
    fn unresolved_topology_uses_exact_paths_and_never_serial_or_product_names() {
        let first = target_for(&interface("PATH-A-COL01"), None);
        assert_eq!(first.key, target_for(&interface("path-a-col01"), None).key);
        assert_ne!(first.key, target_for(&interface("path-a-col02"), None).key);
        assert_ne!(first.key, target_for(&interface("path-b-col01"), None).key);
        assert_ne!(first.key, Target::new(BackendId::StandardHid, &interface("PATH-A-COL01"), 0).key);
        let mut renamed = interface("PATH-A-COL01");
        renamed.serial = "ANOTHER-SERIAL".into();
        renamed.product = "Renamed device".into();
        assert_eq!(first.key, target_for(&renamed, None).key);
        assert!(!first.verified_identity);
    }

    #[test]
    fn cache_isolates_duplicate_serials_and_reuses_only_proven_sibling_samples() {
        let mut cache = Cache::default();
        cache.observe(&report(&["a1", "a2", "b1"], &[]));
        let first = target_for(&interface("a1"), Some(r"usb\vid_1234&pid_5678\a"));
        let sibling = target_for(&interface("a2"), Some(r"usb\vid_1234&pid_5678\a"));
        let second = target_for(&interface("b1"), Some(r"usb\vid_1234&pid_5678\b"));
        let first_lease = register(&mut cache, &first);
        let sibling_lease = register(&mut cache, &sibling);
        let second_lease = register(&mut cache, &second);
        let now = Instant::now();
        let zero = normalize_percent(0, 0, 100).unwrap();
        cache.sample(&first_lease, Some(zero), now).unwrap();
        assert_eq!(cache.sample(&sibling_lease, None, now).unwrap(), Some((zero, now)));
        assert_eq!(cache.sample(&second_lease, None, now).unwrap(), None);
        let high = normalize_percent(91, 0, 100).unwrap();
        cache.sample(&second_lease, Some(high), now).unwrap();
        assert_eq!(cache.sample(&first_lease, None, now).unwrap(), Some((zero, now)));
        assert_eq!(cache.sample(&second_lease, None, now).unwrap(), Some((high, now)));
        let stable_key = first.key.clone();
        cache.observe(&report(&["a1", "a2"], &[]));
        assert_eq!(target_for(&interface("a1"), Some(r"usb\vid_1234&pid_5678\a")).key, stable_key);
    }

    #[test]
    fn full_presence_including_unreadable_paths_retains_a_device_until_all_members_disappear() {
        let mut cache = Cache::default();
        cache.observe(&report(&["a1", "a2", "b1"], &[]));
        let first = target_for(&interface("a1"), Some(r"usb\vid_1234&pid_5678\a"));
        let sibling = target_for(&interface("a2"), Some(r"usb\vid_1234&pid_5678\a"));
        let second = target_for(&interface("b1"), Some(r"usb\vid_1234&pid_5678\b"));
        let first_lease = register(&mut cache, &first);
        let sibling_lease = register(&mut cache, &sibling);
        let second_lease = register(&mut cache, &second);
        let now = Instant::now();
        let percent = normalize_percent(44, 0, 100).unwrap();
        cache.sample(&first_lease, Some(percent), now).unwrap();
        cache.sample(&second_lease, Some(percent), now).unwrap();
        cache.observe(&report(&["b1"], &["A2"]));
        assert!(cache.sample(&first_lease, None, now).is_err());
        assert_eq!(cache.sample(&sibling_lease, None, now).unwrap(), Some((percent, now)));
        cache.observe(&report(&["b1"], &[]));
        assert!(!cache.devices.contains_key(&first.key));
        assert_eq!(cache.sample(&second_lease, None, now).unwrap(), Some((percent, now)));
    }

    #[test]
    fn same_path_reenumeration_clears_samples_and_rejects_late_reads_and_discovery() {
        let mut cache = Cache::default();
        let old_presence = cache.observe(&report(&["same-path"], &[]));
        let target = target_for(&interface("same-path"), Some(r"usb\vid_1234&pid_5678\same-parent"));
        let old_lease = register(&mut cache, &target);
        let now = Instant::now();
        let old_percent = normalize_percent(93, 0, 100).unwrap();
        cache.sample(&old_lease, Some(old_percent), now).unwrap();
        cache.observe(&report(&[], &[]));
        cache.observe(&report(&["same-path"], &[]));
        assert!(!cache.register(&target, &old_presence["same-path"]));
        let new_lease = register(&mut cache, &target);
        assert!(cache.sample(&old_lease, Some(old_percent), now).is_err());
        assert_eq!(cache.sample(&new_lease, None, now).unwrap(), None);
        let new_percent = normalize_percent(12, 0, 100).unwrap();
        cache.sample(&new_lease, Some(new_percent), now).unwrap();
        assert!(cache.sample(&old_lease, Some(old_percent), now).is_err());
        assert_eq!(cache.sample(&new_lease, None, now).unwrap(), Some((new_percent, now)));
    }

    #[test]
    fn changed_topology_on_a_continuously_present_path_starts_a_new_cache_generation() {
        let mut cache = Cache::default();
        cache.observe(&report(&["same-path"], &[]));
        let fallback = target_for(&interface("same-path"), None);
        let resolved = target_for(&interface("same-path"), Some(r"usb\vid_1234&pid_5678\parent"));
        let old_lease = register(&mut cache, &fallback);
        let now = Instant::now();
        cache.sample(&old_lease, normalize_percent(90, 0, 100), now).unwrap();
        let new_lease = register(&mut cache, &resolved);
        assert!(cache.sample(&old_lease, normalize_percent(90, 0, 100), now).is_err());
        assert_eq!(cache.sample(&new_lease, None, now).unwrap(), None);
    }

    #[test]
    fn failed_complete_scan_does_not_supply_removal_evidence() {
        let cache = Mutex::new(Cache::default());
        observe_presence_with(&cache, || Ok(report(&["present"], &[]))).unwrap();
        let target = target_for(&interface("present"), None);
        let lease = register(&mut cache.lock().unwrap(), &target);
        let now = Instant::now();
        let percent = normalize_percent(55, 0, 100).unwrap();
        cache.lock().unwrap().sample(&lease, Some(percent), now).unwrap();
        assert!(observe_presence_with(&cache, || Err("enumeration interrupted".into())).is_err());
        assert_eq!(cache.lock().unwrap().sample(&lease, None, now).unwrap(), Some((percent, now)));
    }

    #[test]
    fn silent_first_collection_does_not_hide_a_successful_sibling_or_another_device() {
        let mut cache = Cache::default();
        cache.observe(&report(&["a1", "a2", "b1"], &[]));
        let targets = vec![
            target_for(&interface("a1"), Some(r"usb\vid_1234&pid_5678\a")),
            target_for(&interface("a2"), Some(r"usb\vid_1234&pid_5678\a")),
            target_for(&interface("b1"), Some(r"usb\vid_1234&pid_5678\b")),
        ];
        for target in &targets { register(&mut cache, target); }
        let now = Instant::now();
        let mut visited = Vec::new();
        let snapshots = super::super::read_targets(targets, |target| {
            visited.push(target.channel_key());
            let percent = match target.channel_key().as_str() {
                "a1" => None,
                "a2" => normalize_percent(29, 0, 100),
                "b1" => normalize_percent(81, 0, 100),
                _ => unreachable!(),
            };
            let lease = cache.lease(target)?;
            complete_read(&mut cache, &lease, target, percent, now)
        });
        assert_eq!(visited, ["a1", "a2", "b1"]);
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].battery.as_ref().unwrap().percent, Some(29));
        assert_eq!(snapshots[1].battery.as_ref().unwrap().percent, Some(81));
        assert_ne!(snapshots[0].target.key, snapshots[1].target.key);
    }

    #[test]
    fn cached_samples_preserve_precision_measurement_time_and_expiry() {
        let mut cache = Cache::default();
        cache.observe(&report(&["present"], &[]));
        let target = target_for(&interface("present"), None);
        let lease = register(&mut cache, &target);
        let now = Instant::now();
        let later = now + std::time::Duration::from_secs(1);
        let estimate = normalize_percent(1, 0, 4).unwrap();
        complete_read(&mut cache, &lease, &target, Some(estimate), now).unwrap();
        let cached = complete_read(&mut cache, &lease, &target, None, later).unwrap();
        assert_eq!(cached.battery_at, Some(now));
        assert_eq!(cached.battery.as_ref().unwrap().percent, None);
        assert_eq!(cached.battery.as_ref().unwrap().estimated_percent, Some(25));
        assert_eq!(cached.battery.as_ref().unwrap().charging, None);
        assert!(complete_read(&mut cache, &lease, &target, None,
            now + BackendId::StandardHid.sample_lifetime()).is_err());
        assert!(cache.devices[&target.key].sample.is_none());
    }
    #[test]
    fn standard_fallback_skips_known_rog_devices_without_excluding_all_asus_devices() {
        let mut interface = HidInterface {
            path: "test".into(),
            serial: String::new(),
            product: String::new(),
            vendor_id: crate::hid::VID_ASUS,
            product_id: 0x1b65,
            usage_page: 1,
            usage: 2,
            input_len: 20,
            output_len: 20,
            feature_len: 0,
        };
        assert!(reserved_by_rog(&interface));
        interface.product_id = 0x1ace;
        assert!(reserved_by_rog(&interface));
        interface.product_id = 0xf0f0;
        assert!(!reserved_by_rog(&interface));
        interface.product_id = 0x1b65;
        interface.vendor_id = 0x046d;
        assert!(!reserved_by_rog(&interface));
    }
    #[test]
    fn descriptor_range_and_null_values_are_respected() {
        assert_eq!(normalize_percent(127, 0, 255), Some(BatteryPercent { value: 50, approximate: true }));
        assert_eq!(normalize_percent(0, 0, 100), Some(BatteryPercent { value: 0, approximate: false }));
        assert_eq!(normalize_percent(100, 0, 100), Some(BatteryPercent { value: 100, approximate: false }));
        assert_eq!(normalize_percent(255, 0, 100), None);
        assert_eq!(normalize_percent(2, 5, 100), None);
        assert_eq!(normalize_percent(5, 5, 5), None);
        assert_eq!(normalize_percent(0, -1, 100), None);
        assert_eq!(normalize_percent(u32::MAX, 0, i32::MAX), None);
    }

    #[test]
    fn scaled_strengths_remain_estimates_even_at_zero_and_full_but_native_percentages_stay_exact() {
        for raw in 0..=4 {
            let b = battery(normalize_percent(raw, 0, 4).unwrap());
            assert_eq!(b.percent, None);
            assert_eq!(b.estimated_percent, Some(raw as u8 * 25));
            assert_eq!(b.display_percent().unwrap().label(true), format!("≈{}%", raw * 25));
        }
        for raw in 0..=100 {
            let b = battery(normalize_percent(raw, 0, 100).unwrap());
            assert_eq!(b.percent, Some(raw as u8));
            assert_eq!(b.estimated_percent, None);
            assert_eq!(b.display_percent().unwrap().label(true), format!("{raw}%"));
        }
        assert_eq!(normalize_percent(127, 0, 255).unwrap().label(false), "≈50");
        assert_eq!(normalize_percent(15, 10, 20).unwrap().label(true), "≈50%");
        assert_eq!(normalize_percent(i32::MAX as u32, 0, i32::MAX).unwrap().label(true), "≈100%");
    }

    #[test]
    fn scaled_strength_keeps_its_precision_through_cached_and_stale_readings() {
        let now = std::time::Instant::now();
        let mut state = status::Status::default();
        let b = battery(normalize_percent(1, 0, 4).unwrap());
        let lifetime = BackendId::StandardHid.sample_lifetime();
        state.observe_sample(Some(true), Some(&b), Some(now), now, lifetime);
        state.observe_sample(Some(true), Some(&b), Some(now), now + lifetime, lifetime);
        assert_eq!(state.measured_at, Some(now));
        let mut reading = crate::device::DeviceReading { battery: state.battery.clone(),
            connected: true, quality: state.quality, ..Default::default() };
        assert_eq!(reading.battery_text(true), "—");
        reading.quality = crate::device::ReadingQuality::Cached;
        assert_eq!(reading.battery_text(true), "≈25%");
        assert_eq!(reading.percent(), None);
    }
}
