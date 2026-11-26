//! DAREU 260D:1074 battery GET over Feature 08 and Input 09.
//! EM901 and CM610 Pro have separate collection layouts. Pair only through a
//! proved USB parent; never send a handshake, configuration or DPI command.
use super::*;
use crate::{hid::HidDevice, win32::*};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use winapi::{
    shared::hidpi::HIDP_VALUE_CAPS,
    um::cfgmgr32::{CM_Get_Device_IDW, CM_Get_Parent, CM_Locate_DevNodeW},
};

pub struct DareuEm901;
const PRODUCT: &str = "2.4G Wireless Receiver";
const MANUFACTURER: &str = "Compx";
const LENGTH: u16 = 17;
const FEATURE: u8 = 8;
const INPUT: u8 = 9;
const COMMAND: u8 = 4;
const MAX_READS: usize = 32;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn identity(i: &HidInterface) -> bool {
    i.vendor_id == 0x260d && i.product_id == 0x1074
}
fn command_collection(i: &HidInterface) -> bool {
    identity(i) && i.usage_page == 0xff02 && i.usage == 2
}
fn receive_collection(i: &HidInterface) -> bool {
    identity(i) && i.usage_page == 0xff01 && i.usage == 0
}

fn has_unresolved_collection(skipped: &[crate::hid::Skipped]) -> bool {
    skipped.iter().any(|entry| {
        let path = entry.path.to_ascii_lowercase();
        path.contains("vid_260d") && path.contains("pid_1074")
    })
}

fn device_instance(i: &HidInterface) -> Option<String> {
    if !identity(i) {
        return None;
    }
    let path = i.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4
        || parts[0] != r"\\?\hid"
        || parts[2].is_empty()
        || parts[3].is_empty()
        || path.contains('\0')
        || parts[1].contains('\\')
        || parts[2].contains('\\')
    {
        return None;
    }
    let fields: Vec<_> = parts[1].split('&').collect();
    for required in ["vid_260d", "pid_1074"] {
        if fields.iter().filter(|field| **field == required).count() != 1 {
            return None;
        }
    }
    if fields
        .iter()
        .filter(|field| field.starts_with("vid_"))
        .count()
        != 1
        || fields
            .iter()
            .filter(|field| field.starts_with("pid_"))
            .count()
            != 1
    {
        return None;
    }
    for prefix in ["mi_", "col"] {
        let indices: Vec<_> = fields
            .iter()
            .filter_map(|field| field.strip_prefix(prefix))
            .collect();
        if indices.len() > 1
            || indices.iter().any(|index| {
                index.len() != 2 || !index.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return None;
        }
    }
    Some(format!("hid\\{}\\{}", parts[1], parts[2]))
}

fn physical_parent_name(value: &str) -> bool {
    let parts: Vec<_> = value.split('\\').collect();
    if parts.len() != 3 || parts[0] != "usb" || parts[2].is_empty() {
        return false;
    }
    let fields: Vec<_> = parts[1].split('&').collect();
    fields.len() == 2 && fields.contains(&"vid_260d") && fields.contains(&"pid_1074")
}

fn usb_parent(i: &HidInterface) -> Option<String> {
    let mut instance = to_wide(&device_instance(i)?);
    let mut node = 0;
    if unsafe { CM_Locate_DevNodeW(&mut node, instance.as_mut_ptr(), 0) } != 0 {
        return None;
    }
    for _ in 0..8 {
        let mut parent = 0;
        if unsafe { CM_Get_Parent(&mut parent, node, 0) } != 0 {
            return None;
        }
        let mut buffer = [0u16; 512];
        if unsafe { CM_Get_Device_IDW(parent, buffer.as_mut_ptr(), buffer.len() as u32, 0) } != 0 {
            return None;
        }
        let length = buffer.iter().position(|c| *c == 0)?;
        let name = String::from_utf16(&buffer[..length])
            .ok()?
            .to_ascii_lowercase();
        if physical_parent_name(&name) {
            return Some(name);
        }
        node = parent;
    }
    None
}

#[derive(Clone, Debug, Default)]
struct Reports {
    input: BTreeMap<u8, u16>,
    feature: BTreeMap<u8, u16>,
}

fn byte_reports(
    fields: &[HIDP_VALUE_CAPS],
    maximum: u16,
    buttons: u16,
) -> Option<BTreeMap<u8, u16>> {
    if buttons != 0 || fields.len() > 32 {
        return None;
    }
    if maximum == 0 {
        return fields.is_empty().then(BTreeMap::new);
    }
    if !(2..=4096).contains(&maximum) || fields.is_empty() {
        return None;
    }
    let mut reports = BTreeMap::new();
    for field in fields {
        let length = field.ReportCount.checked_add(1)?;
        if field.IsAlias != 0
            || field.BitSize != 8
            || field.ReportCount == 0
            || length > maximum
            || reports.insert(field.ReportID, length).is_some()
        {
            return None;
        }
    }
    if reports.contains_key(&0) && reports.len() != 1 {
        return None;
    }
    (reports.values().copied().max() == Some(maximum)).then_some(reports)
}

struct Metadata<'a> {
    api: &'a HidApi,
    handle: Handle,
    preparsed: *mut core::ffi::c_void,
}
impl Drop for Metadata<'_> {
    fn drop(&mut self) {
        unsafe {
            if !self.preparsed.is_null() {
                (self.api.free_preparsed_data)(self.preparsed);
            }
            CloseHandle(self.handle);
        }
    }
}

impl<'a> Metadata<'a> {
    fn open(api: &'a HidApi, i: &HidInterface) -> Option<Self> {
        if device_instance(i).is_none() {
            return None;
        }
        let handle = unsafe {
            CreateFileW(
                to_wide(&i.path).as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                core::ptr::null_mut(),
                OPEN_EXISTING,
                0,
                core::ptr::null_mut(),
            )
        };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut meta = Self {
            api,
            handle,
            preparsed: core::ptr::null_mut(),
        };
        if unsafe { (api.get_preparsed_data)(handle, &mut meta.preparsed) } == 0 {
            return None;
        }
        Some(meta)
    }

    fn read_reports(
        &self,
        kind: u32,
        maximum: u16,
        buttons: u16,
        count: u16,
    ) -> Option<BTreeMap<u8, u16>> {
        if buttons != 0 || count > 32 {
            return None;
        }
        if count == 0 {
            return byte_reports(&[], maximum, buttons);
        }
        let mut fields: Vec<HIDP_VALUE_CAPS> =
            (0..count).map(|_| unsafe { core::mem::zeroed() }).collect();
        let mut returned = count;
        if unsafe {
            (self.api.get_value_caps)(kind, fields.as_mut_ptr(), &mut returned, self.preparsed)
        } < 0
            || returned != count
        {
            return None;
        }
        byte_reports(&fields, maximum, buttons)
    }

    fn reports(&self, i: &HidInterface) -> Option<Reports> {
        let mut caps = HidpCaps::default();
        if unsafe { (self.api.get_caps)(self.preparsed, &mut caps) } < 0
            || caps.usage_page != i.usage_page
            || caps.usage != i.usage
            || caps.input_report_byte_length != i.input_len
            || caps.output_report_byte_length != i.output_len
            || caps.feature_report_byte_length != i.feature_len
        {
            return None;
        }
        Some(Reports {
            input: self.read_reports(
                0,
                caps.input_report_byte_length,
                caps.number_input_button_caps,
                caps.number_input_value_caps,
            )?,
            feature: if command_collection(i) {
                self.read_reports(
                    2,
                    caps.feature_report_byte_length,
                    caps.number_feature_button_caps,
                    caps.number_feature_value_caps,
                )?
            } else {
                BTreeMap::new()
            },
        })
    }

    fn manufacturer(&self) -> Option<String> {
        // HidApi already owns hid.dll. Resolve only the metadata export, without
        // widening the shared enumeration or requesting data-stream access.
        let module = unsafe { GetModuleHandleW(to_wide("hid.dll").as_ptr()) };
        if module.is_null() {
            return None;
        }
        let symbol = unsafe { GetProcAddress(module, b"HidD_GetManufacturerString\0".as_ptr()) };
        if symbol.is_null() {
            return None;
        }
        let get: FnHidDGetProductString = unsafe { core::mem::transmute(symbol) };
        let mut buffer = [0u16; 256];
        if unsafe {
            get(
                self.handle,
                buffer.as_mut_ptr(),
                core::mem::size_of_val(&buffer) as u32,
            )
        } == 0
        {
            return None;
        }
        let length = buffer.iter().position(|c| *c == 0)?;
        String::from_utf16(&buffer[..length]).ok()
    }
}

fn inspect(api: &HidApi, i: &HidInterface) -> Option<Reports> {
    Metadata::open(api, i)?.reports(i)
}
fn manufacturer(api: &HidApi, i: &HidInterface) -> Option<String> {
    Metadata::open(api, i)?.manufacturer()
}

struct Route<'a> {
    tx: &'a HidInterface,
    rx: &'a HidInterface,
    parent: String,
    split: bool,
}

fn select_with<'a>(
    interfaces: &'a [HidInterface],
    mut parent: impl FnMut(&HidInterface) -> Option<String>,
    mut inspect: impl FnMut(&HidInterface) -> Option<Reports>,
    mut manufacturer: impl FnMut(&HidInterface) -> Option<String>,
) -> Vec<Route<'a>> {
    let mut groups: BTreeMap<String, Vec<&HidInterface>> = BTreeMap::new();
    for i in interfaces
        .iter()
        .filter(|i| command_collection(i) || receive_collection(i))
    {
        let Some(key) = parent(i).filter(|p| physical_parent_name(p)) else {
            return Vec::new();
        };
        groups.entry(key).or_default().push(i);
    }
    let mut result = Vec::new();
    for (parent, members) in groups {
        let txs: Vec<_> = members
            .iter()
            .copied()
            .filter(|i| command_collection(i))
            .collect();
        let [tx] = txs.as_slice() else {
            continue;
        };
        if tx.product != PRODUCT || tx.feature_len != LENGTH || device_instance(tx).is_none() {
            continue;
        }
        let Some(tx_reports) = inspect(tx) else {
            continue;
        };
        if tx_reports.feature.get(&FEATURE) != Some(&LENGTH) {
            continue;
        }
        if tx_reports.input.contains_key(&INPUT) {
            // Do not fall back to another collection if a same-collection
            // Input09 exists but has an unsupported length or cannot respond.
            if tx.input_len == LENGTH && tx_reports.input.get(&INPUT) == Some(&LENGTH) {
                result.push(Route {
                    tx,
                    rx: tx,
                    parent,
                    split: false,
                });
            }
            continue;
        }
        let rxs: Vec<_> = members
            .iter()
            .copied()
            .filter(|i| receive_collection(i))
            .collect();
        let [rx] = rxs.as_slice() else {
            continue;
        };
        if rx.product != PRODUCT
            || rx.input_len != LENGTH
            || device_instance(rx).is_none()
            || rx.path.eq_ignore_ascii_case(&tx.path)
        {
            continue;
        }
        if manufacturer(tx).as_deref() != Some(MANUFACTURER)
            || manufacturer(rx).as_deref() != Some(MANUFACTURER)
        {
            continue;
        }
        let Some(rx_reports) = inspect(rx) else {
            continue;
        };
        if rx_reports.input.get(&INPUT) == Some(&LENGTH) {
            result.push(Route {
                tx,
                rx,
                parent,
                split: true,
            });
        }
    }
    result
}

fn target(route: &Route<'_>) -> Target {
    let mut t = Target::new(BackendId::DareuEm901, route.tx, 0);
    t.key = crate::device::stable_key(&format!("dareu-em901:{}", route.parent));
    // PID and generic product name are shared; do not claim one retail model.
    t.name = "DAREU EM901 / CM610 Pro".into();
    t.family = DeviceFamily::Mouse;
    t
}

fn request() -> [u8; LENGTH as usize] {
    let mut packet = [0; LENGTH as usize];
    packet[0] = FEATURE;
    packet[1] = COMMAND;
    packet[16] = 0x49;
    packet
}

fn battery(frame: &[u8], split: bool) -> Option<Battery> {
    if frame.len() != usize::from(LENGTH) || frame[..3] != [INPUT, COMMAND, 0] || frame[6] > 100 {
        return None;
    }
    // Only the split-route capture establishes a reply checksum. The official
    // EM901 getter treats any nonzero charging byte as charging.
    if split && frame.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) != 0x55 {
        return None;
    }
    Some(Battery {
        percent: Some(frame[6]),
        charging: Some(frame[7] != 0),
        estimated_percent: None,
        voltage_mv: None,
        approximate: false,
        level: None,
    })
}

trait Transport {
    fn send(&mut self, packet: &[u8]) -> Result<(), String>;
    fn receive(&mut self, timeout_ms: u32) -> Option<Vec<u8>>;
}
struct NativeTransport {
    input: HidDevice,
    feature: feature::FeatureChannel,
}
impl NativeTransport {
    fn open(route: &Route<'_>) -> Result<Self, String> {
        let input = HidDevice::open_read_only(route.rx)?;
        let feature = feature::FeatureChannel::open(route.tx)?;
        Ok(Self { input, feature })
    }
}
impl Transport for NativeTransport {
    fn send(&mut self, packet: &[u8]) -> Result<(), String> {
        if packet != request() {
            return Err(UNAVAILABLE.into());
        }
        self.feature.send(packet, 250)
    }
    fn receive(&mut self, timeout_ms: u32) -> Option<Vec<u8>> {
        self.input.read_report(timeout_ms)
    }
}

fn read_with(t: &Target, split: bool, transport: &mut impl Transport) -> Result<Snapshot, String> {
    let i = t.hid()?;
    if t.backend != BackendId::DareuEm901
        || !command_collection(i)
        || i.product != PRODUCT
        || i.feature_len != LENGTH
        || device_instance(i).is_none()
    {
        return Err(UNAVAILABLE.into());
    }
    let mut drained = false;
    for _ in 0..8 {
        if transport.receive(0).is_none() {
            drained = true;
            break;
        }
    }
    if !drained {
        return Err(UNAVAILABLE.into());
    }
    let started = Instant::now();
    if let Err(error) = transport.send(&request()) {
        trace::record(
            i,
            BackendId::DareuEm901,
            "query-battery",
            COMMAND.into(),
            FEATURE,
            usize::from(LENGTH),
            0,
            started,
            trace::Outcome::NoReply,
        );
        return Err(error);
    }
    let deadline = Instant::now() + Duration::from_millis(1_000);
    let mut received = 0;
    for _ in 0..MAX_READS {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        let Some(frame) = transport.receive(left.as_millis().max(1) as u32) else {
            break;
        };
        received = frame.len();
        let Some(battery) = battery(&frame, split) else {
            continue;
        };
        trace::record(
            i,
            BackendId::DareuEm901,
            "query-battery",
            COMMAND.into(),
            FEATURE,
            usize::from(LENGTH),
            received,
            started,
            trace::Outcome::Ok,
        );
        let mut target = t.clone();
        target.verified_identity = true;
        return Ok(Snapshot {
            target,
            battery: Some(battery),
            battery_at: Some(Instant::now()),
            battery_unavailable: false,
            connected: true,
            dpi: Dpi::default(),
            polling_rate: Default::default(),
        });
    }
    trace::record(
        i,
        BackendId::DareuEm901,
        "query-battery",
        COMMAND.into(),
        FEATURE,
        usize::from(LENGTH),
        received,
        started,
        if received == 0 {
            trace::Outcome::NoReply
        } else {
            trace::Outcome::InvalidData
        },
    );
    Err(UNAVAILABLE.into())
}

impl DeviceBackend for DareuEm901 {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(select_with(
            interfaces,
            usb_parent,
            |i| inspect(api, i),
            |i| manufacturer(api, i),
        )
        .iter()
        .map(target)
        .collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        if t.backend != BackendId::DareuEm901 {
            return Err(UNAVAILABLE.into());
        }
        let original = t.hid()?;
        if !command_collection(original)
            || original.product != PRODUCT
            || original.feature_len != LENGTH
        {
            return Err(UNAVAILABLE.into());
        }
        let locked_parent = usb_parent(original).ok_or(UNAVAILABLE)?;
        let lock = channel::lock(&locked_parent);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let inventory = crate::hid::enumerate_all_report(api)?;
        if has_unresolved_collection(&inventory.skipped) {
            return Err(UNAVAILABLE.into());
        }
        let routes = select_with(
            &inventory.interfaces,
            usb_parent,
            |i| inspect(api, i),
            |i| manufacturer(api, i),
        );
        let route = routes
            .iter()
            .find(|route| {
                t.hid()
                    .is_ok_and(|i| i.path.eq_ignore_ascii_case(&route.tx.path))
            })
            .ok_or(UNAVAILABLE)?;
        let current = target(route);
        if route.parent != locked_parent || current.key != t.key {
            return Err(UNAVAILABLE.into());
        }
        read_with(&current, route.split, &mut NativeTransport::open(route)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn interface(instance: &str, receive: bool) -> HidInterface {
        let col = if receive { 1 } else { 2 };
        HidInterface {
            path: format!(
                r"\\?\hid#vid_260d&pid_1074&mi_02&col{col:02}#8&{instance}&0&000{col}#{{hid}}"
            ),
            vendor_id: 0x260d,
            product_id: 0x1074,
            usage_page: if receive { 0xff01 } else { 0xff02 },
            usage: if receive { 0 } else { 2 },
            input_len: if receive { LENGTH } else { 0 },
            output_len: 0,
            feature_len: if receive { 0 } else { LENGTH },
            product: PRODUCT.into(),
            serial: "shared".into(),
        }
    }
    fn parent(instance: &str) -> String {
        format!(r"usb\vid_260d&pid_1074\{instance}")
    }
    fn proof(i: &HidInterface) -> Option<Reports> {
        Some(Reports {
            input: if i.input_len > 0 {
                [(INPUT, i.input_len)].into()
            } else {
                BTreeMap::new()
            },
            feature: if command_collection(i) {
                [(FEATURE, LENGTH)].into()
            } else {
                BTreeMap::new()
            },
        })
    }
    fn synthetic(percent: u8, charging: u8) -> Vec<u8> {
        let mut frame = vec![0; usize::from(LENGTH)];
        frame[0] = INPUT;
        frame[1] = COMMAND;
        frame[6] = percent;
        frame[7] = charging;
        frame[16] = 0x55u8.wrapping_sub(
            frame[..16]
                .iter()
                .fold(0u8, |sum, byte| sum.wrapping_add(*byte)),
        );
        frame
    }
    struct Fake {
        reads: VecDeque<Option<Vec<u8>>>,
        sent: Vec<Vec<u8>>,
        fail: bool,
        receives: usize,
    }
    impl Fake {
        fn new(reads: Vec<Option<Vec<u8>>>) -> Self {
            Self {
                reads: reads.into(),
                sent: vec![],
                fail: false,
                receives: 0,
            }
        }
    }
    impl Transport for Fake {
        fn send(&mut self, packet: &[u8]) -> Result<(), String> {
            self.sent.push(packet.to_vec());
            if self.fail {
                Err("feature failed".into())
            } else {
                Ok(())
            }
        }
        fn receive(&mut self, timeout: u32) -> Option<Vec<u8>> {
            assert!(timeout <= 1_000);
            if timeout > 0 {
                self.receives += 1;
            }
            self.reads.pop_front().expect("unexpected read")
        }
    }

    #[test]
    fn battery_only_feature_request_and_route_specific_checksums_are_kept_separate() {
        assert_eq!(
            request(),
            [8, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x49]
        );
        let tx = interface("one", false);
        let rx = interface("one", true);
        let route = Route {
            tx: &tx,
            rx: &rx,
            parent: parent("one"),
            split: true,
        };
        for split in [false, true] {
            let mut transport = Fake::new(vec![None, Some(synthetic(61, 1))]);
            let sample = read_with(&target(&route), split, &mut transport).unwrap();
            assert_eq!(transport.sent, vec![request().to_vec()]);
            assert_eq!(
                (
                    sample.battery.as_ref().unwrap().percent,
                    sample.battery.unwrap().charging
                ),
                (Some(61), Some(true))
            );
            assert!(
                sample.connected && sample.target.verified_identity && sample.battery_at.is_some()
            );
            assert!(
                sample.dpi.stages.is_empty()
                    && !sample.dpi.writable
                    && !sample.polling_rate.writable
            );
        }
        let mut no_checksum = synthetic(61, 1);
        no_checksum[16] ^= 1;
        assert!(battery(&no_checksum, false).is_some());
        assert!(battery(&no_checksum, true).is_none());
    }

    #[test]
    fn complete_descriptor_maps_do_not_infer_lengths_from_other_reports() {
        let field = |id, count| {
            let mut field: HIDP_VALUE_CAPS = unsafe { core::mem::zeroed() };
            field.ReportID = id;
            field.ReportCount = count;
            field.BitSize = 8;
            field
        };
        assert_eq!(
            byte_reports(&[field(9, 16)], 17, 0).unwrap().get(&9),
            Some(&17)
        );
        assert_eq!(
            byte_reports(&[field(8, 16), field(9, 7)], 17, 0)
                .unwrap()
                .get(&9),
            Some(&8)
        );
        assert!(byte_reports(&[field(9, 16)], 64, 0).is_none());
        assert!(byte_reports(&[field(9, 16), field(9, 16)], 17, 0).is_none());
        assert!(byte_reports(&[field(9, 16), field(0, 16)], 17, 0).is_none());
        assert!(byte_reports(&[field(9, 16)], 17, 1).is_none());
        let mut alias = field(9, 16);
        alias.IsAlias = 1;
        assert!(byte_reports(&[alias], 17, 0).is_none());
        let mut bits = field(9, 16);
        bits.BitSize = 1;
        assert!(byte_reports(&[bits], 17, 0).is_none());
        assert!(byte_reports(&[], 0, 0).unwrap().is_empty());
        assert!(byte_reports(&[], 17, 0).is_none());
    }

    #[test]
    fn single_collection_takes_priority_without_borrowing_another_input() {
        let mut tx = interface("one", false);
        tx.input_len = LENGTH;
        let rx = interface("one", true);
        let interfaces = [tx.clone(), rx];
        let routes = select_with(
            &interfaces,
            |_| Some(parent("one")),
            proof,
            |_| panic!("single path has no manufacturer gate"),
        );
        assert_eq!(routes.len(), 1);
        assert!(!routes[0].split);
        assert_eq!(routes[0].tx.path, routes[0].rx.path);
        let mut invalid = interfaces.clone();
        invalid[0].input_len = 64;
        assert!(select_with(
            &invalid,
            |_| Some(parent("one")),
            proof,
            |_| panic!("invalid input09 must not trigger split fallback")
        )
        .is_empty());
        assert!(select_with(
            &[tx],
            |_| Some(parent("one")),
            |_| None,
            |_| panic!("missing input map must not trigger split fallback")
        )
        .is_empty());
    }

    #[test]
    fn split_pair_requires_one_parent_and_exact_strings_on_both_collections() {
        let tx = interface("one", false);
        let rx = interface("one", true);
        let interfaces = [tx.clone(), rx.clone()];
        let routes = select_with(
            &interfaces,
            |_| Some(parent("one")),
            proof,
            |_| Some(MANUFACTURER.into()),
        );
        assert_eq!(routes.len(), 1);
        assert!(routes[0].split);
        assert!(select_with(
            &interfaces,
            |i| Some(parent(if command_collection(i) { "one" } else { "two" })),
            proof,
            |_| Some(MANUFACTURER.into())
        )
        .is_empty());
        for name in [
            None,
            Some("COMPX".into()),
            Some("Compx ".into()),
            Some("".into()),
        ] {
            assert!(select_with(
                &interfaces,
                |_| Some(parent("one")),
                proof,
                |_| name.clone()
            )
            .is_empty());
        }
        for changed in [0, 1] {
            let mut wrong = interfaces.clone();
            wrong[changed].product = "Another Receiver".into();
            assert!(select_with(
                &wrong,
                |_| Some(parent("one")),
                proof,
                |_| Some(MANUFACTURER.into())
            )
            .is_empty());
        }
        assert!(select_with(
            &[tx],
            |_| Some(parent("one")),
            proof,
            |_| Some(MANUFACTURER.into())
        )
        .is_empty());
    }

    #[test]
    fn ambiguity_is_counted_before_product_filter_and_two_receivers_stay_distinct() {
        let tx = interface("one", false);
        let rx = interface("one", true);
        let mut duplicate = tx.clone();
        duplicate.product = "Unknown".into();
        duplicate.path = duplicate.path.replace("mi_02", "mi_03");
        assert!(select_with(
            &[tx.clone(), rx.clone(), duplicate],
            |_| Some(parent("one")),
            |_| panic!("ambiguous sending collection"),
            |_| Some(MANUFACTURER.into())
        )
        .is_empty());
        let mut duplicate = rx.clone();
        duplicate.product = "Unknown".into();
        duplicate.path = duplicate.path.replace("mi_02", "mi_03");
        assert!(select_with(
            &[tx.clone(), rx.clone(), duplicate],
            |_| Some(parent("one")),
            proof,
            |_| Some(MANUFACTURER.into())
        )
        .is_empty());
        assert!(select_with(
            &[tx.clone(), rx.clone()],
            |_| None,
            |_| panic!("unproven parent"),
            |_| None
        )
        .is_empty());
        let interfaces = [tx, rx, interface("two", false), interface("two", true)];
        let routes = select_with(
            &interfaces,
            |i| {
                Some(parent(if i.path.contains("&one&") {
                    "one"
                } else {
                    "two"
                }))
            },
            proof,
            |_| Some(MANUFACTURER.into()),
        );
        assert_eq!(routes.len(), 2);
        assert_ne!(target(&routes[0]).key, target(&routes[1]).key);
        assert!(!target(&routes[0]).same_channel(&target(&routes[1])));
        assert!(has_unresolved_collection(&[crate::hid::Skipped {
            path: interface("hidden", true).path,
            reason: "descriptor inaccessible".into(),
        }]));
        assert!(!has_unresolved_collection(&[crate::hid::Skipped {
            path: interface("other", true)
                .path
                .replace("pid_1074", "pid_1114"),
            reason: "unavailable".into(),
        }]));
    }

    #[test]
    fn reply_prefix_status_length_range_and_nonzero_charge_semantics_are_checked() {
        for percent in 0..=100 {
            for charging in [0, 1, 2, 0xff] {
                let frame = synthetic(percent, charging);
                assert_eq!(battery(&frame, true).unwrap().percent, Some(percent));
                assert_eq!(battery(&frame, true).unwrap().charging, Some(charging != 0));
            }
        }
        let valid = synthetic(61, 0);
        for length in 0..17 {
            assert!(battery(&valid[..length], false).is_none());
        }
        let mut long = valid.clone();
        long.push(0);
        assert!(battery(&long, false).is_none());
        for (offset, value) in [(0, 8), (1, 3), (2, 1), (6, 101)] {
            let mut wrong = valid.clone();
            wrong[offset] = value;
            assert!(battery(&wrong, false).is_none());
        }
        assert!(battery(&request(), false).is_none());
    }

    #[test]
    fn queued_data_silence_and_invalid_reports_never_cause_retries_or_settings_commands() {
        let tx = interface("one", false);
        let rx = interface("one", true);
        let route = Route {
            tx: &tx,
            rx: &rx,
            parent: parent("one"),
            split: true,
        };
        let target = target(&route);
        let stale = synthetic(99, 0);
        let mut normal = Fake::new(vec![
            Some(stale.clone()),
            None,
            Some(vec![0; 17]),
            Some(synthetic(61, 0)),
        ]);
        assert_eq!(
            read_with(&target, true, &mut normal)
                .unwrap()
                .battery
                .unwrap()
                .percent,
            Some(61)
        );
        assert_eq!((normal.sent.len(), normal.receives), (1, 2));
        let mut busy = Fake::new(vec![Some(stale); 8]);
        assert!(read_with(&target, true, &mut busy).is_err());
        assert!(busy.sent.is_empty());
        let mut silent = Fake::new(vec![None, None]);
        assert!(read_with(&target, true, &mut silent).is_err());
        assert_eq!(silent.sent.len(), 1);
        let mut malformed = vec![None];
        malformed.extend(vec![Some(vec![0; 17]); MAX_READS]);
        let mut malformed = Fake::new(malformed);
        assert!(read_with(&target, true, &mut malformed).is_err());
        assert_eq!((malformed.sent.len(), malformed.receives), (1, MAX_READS));
        let mut failed = Fake::new(vec![None]);
        failed.fail = true;
        assert!(read_with(&target, true, &mut failed).is_err());
        assert_eq!((failed.sent.len(), failed.receives), (1, 0));
    }

    #[test]
    fn unrelated_pid_and_malformed_paths_are_rejected_before_io() {
        let mut tx = interface("one", false);
        let rx = interface("one", true);
        for path in [
            "invalid".into(),
            tx.path.replace("pid_1074", "pid_1090"),
            tx.path.replace("vid_260d", "vid_260d&vid_320f"),
            tx.path.replace("mi_02", "mi_002"),
        ] {
            let mut wrong = tx.clone();
            wrong.path = path;
            assert!(device_instance(&wrong).is_none());
        }
        tx.product_id = 0x1090;
        let route = Route {
            tx: &tx,
            rx: &rx,
            parent: parent("one"),
            split: true,
        };
        let mut fake = Fake::new(vec![]);
        assert!(read_with(&target(&route), true, &mut fake).is_err());
        assert!(fake.sent.is_empty());
        assert!(!physical_parent_name(r"usb\vid_260d&pid_1074&mi_02\one"));
        assert!(!physical_parent_name(r"usb\vid_260d&pid_1090\one"));
    }
}
