//! MCHOSE 4D mouse receiver status, independent of Feature mice and QHW keyboards.
use super::*;
use crate::{hid::HidDevice, win32::to_wide};
use std::{collections::BTreeMap, time::Instant};
use winapi::um::cfgmgr32::{CM_Get_Device_IDW, CM_Get_Parent, CM_Locate_DevNodeW};

pub struct MchoseModern;
const REPORT: u8 = 0x4d;
const LENGTH: u16 = 64;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";
const V7_PID: u16 = 0x101d;
const V7_NAME: &str = "MCHOSE V7 Pro";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x3837 && matches!(i.product_id, 0x1014 | 0x1016 | 0x1018 | V7_PID)
        && i.usage_page == 0xff01 && i.usage == 1
}

fn control(i: &HidInterface) -> bool {
    management(i) && i.input_len == LENGTH && i.output_len == LENGTH
        && (i.product_id != V7_PID || i.product == V7_NAME && v7_instance(i).is_some())
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|i| i.product_id != V7_PID && control(i) && interfaces.iter().filter(|other|
        management(other) && channel::collection_group(i) == channel::collection_group(other)
    ).count() == 1).collect()
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    control(i) && descriptor::Descriptor::open(api, i)
        .is_some_and(|d| d.input_output_byte_reports(i, REPORT, LENGTH, LENGTH)
            || windows_array_variant(i) && d.initialized_input_output_report(i, REPORT, LENGTH))
}

fn windows_array_variant(i: &HidInterface) -> bool {
    control(i) && matches!((i.product_id, i.product.trim()),
            (0x1014 | 0x1018, "MCHOSE A7 V3 Ultra+") | (0x1018, "MCHOSE A7 V3 Pro"))
        && i.path.to_ascii_lowercase().split('#').nth(1)
            .is_some_and(|hardware| hardware.split('&').any(|field| field == "mi_02"))
}

fn v7_instance(interface: &HidInterface) -> Option<String> {
    if interface.vendor_id != 0x3837 || interface.product_id != V7_PID { return None; }
    let path = interface.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid" || parts[2].is_empty()
        || parts[3].is_empty() || path.contains('\0')
        || parts[1].contains('\\') || parts[2].contains('\\') { return None; }
    let fields: Vec<_> = parts[1].split('&').collect();
    for (prefix, expected) in [("vid_", "vid_3837"), ("pid_", "pid_101d")] {
        if fields.iter().filter(|field| field.starts_with(prefix)).count() != 1
            || !fields.contains(&expected) { return None; }
    }
    for prefix in ["mi_", "col"] {
        let indices: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix(prefix)).collect();
        if indices.len() > 1 || indices.iter().any(|index|
            index.len() != 2 || !index.bytes().all(|byte| byte.is_ascii_hexdigit())) { return None; }
    }
    Some(format!("hid\\{}\\{}", parts[1], parts[2]))
}

fn v7_parent_name(name: &str) -> bool {
    let parts: Vec<_> = name.split('\\').collect();
    if parts.len() != 3 || parts[0] != "usb" || parts[2].is_empty() || name.contains('\0') {
        return false;
    }
    let fields: Vec<_> = parts[1].split('&').collect();
    fields.len() == 2 && fields.contains(&"vid_3837") && fields.contains(&"pid_101d")
}

fn v7_parent(interface: &HidInterface) -> Option<String> {
    let mut instance = to_wide(&v7_instance(interface)?);
    let mut node = 0;
    if unsafe { CM_Locate_DevNodeW(&mut node, instance.as_mut_ptr(), 0) } != 0 { return None; }
    for _ in 0..8 {
        let mut parent = 0;
        if unsafe { CM_Get_Parent(&mut parent, node, 0) } != 0 { return None; }
        let mut buffer = [0u16; 512];
        if unsafe { CM_Get_Device_IDW(parent, buffer.as_mut_ptr(), buffer.len() as u32, 0) } != 0 {
            return None;
        }
        let length = buffer.iter().position(|unit| *unit == 0)?;
        let name = String::from_utf16(&buffer[..length]).ok()?.to_ascii_lowercase();
        if v7_parent_name(&name) { return Some(name); }
        node = parent;
    }
    None
}

fn unreadable_v7(skipped: &[crate::hid::Skipped]) -> bool {
    skipped.iter().any(|entry| entry.path.to_ascii_lowercase().split('#').nth(1)
        .is_some_and(|hardware| {
            let fields: Vec<_> = hardware.split('&').collect();
            fields.contains(&"vid_3837") && fields.contains(&"pid_101d")
        }))
}

fn select_v7_with<'a>(interfaces: &'a [HidInterface],
    mut parent: impl FnMut(&HidInterface) -> Option<String>,
    mut proof: impl FnMut(&HidInterface) -> bool) -> Vec<(&'a HidInterface, String)> {
    let mut groups: BTreeMap<String, Vec<&HidInterface>> = BTreeMap::new();
    for interface in interfaces.iter().filter(|interface|
        interface.product_id == V7_PID && management(interface)) {
        if v7_instance(interface).is_none() { return Vec::new(); }
        let Some(parent) = parent(interface).map(|name| name.to_ascii_lowercase())
            .filter(|name| v7_parent_name(name)) else { return Vec::new(); };
        groups.entry(parent).or_default().push(interface);
    }
    groups.into_iter().filter_map(|(parent, members)| {
        let [interface] = members.as_slice() else { return None; };
        (control(interface) && proof(interface)).then_some((*interface, parent))
    }).collect()
}

fn append_v7_targets_with(targets: &mut Vec<Target>, report: Result<crate::hid::EnumerateReport, String>,
    parent: impl FnMut(&HidInterface) -> Option<String>, proof: impl FnMut(&HidInterface) -> bool) {
    let Ok(report) = report else { return; };
    if unreadable_v7(&report.skipped) { return; }
    targets.extend(select_v7_with(&report.interfaces, parent, proof)
        .into_iter().map(|(interface, _)| target(interface, &report.interfaces)));
}

fn target(i: &HidInterface, interfaces: &[HidInterface]) -> Target {
    let mut t = Target::new(BackendId::MchoseModern, i, 0);
    let name = std::iter::once(i).chain(interfaces.iter().filter(|other|
        channel::collection_group(i) == channel::collection_group(other)))
        .map(|i| i.product.trim())
        .find(|name| name.to_ascii_uppercase().starts_with("MCHOSE "));
    // Shared receiver and reply PIDs cannot identify the paired retail model.
    t.name = name.unwrap_or("MCHOSE Mouse").into();
    t.family = DeviceFamily::Mouse;
    t
}

fn request() -> [u8; LENGTH as usize] {
    let mut p = [0; LENGTH as usize];
    p[..9].copy_from_slice(&[REPORT, 1, 1, 0, 0, 9, 0, 0, 8]);
    p
}

#[derive(Debug, PartialEq, Eq)]
enum Reply { Battery { percent: u8, charging: Option<bool> }, Offline }

fn response_data(r: &[u8]) -> Option<&[u8]> {
    if r.len() != usize::from(LENGTH) || r[..2] != [REPORT, 1] || r[2] > 1
        || r[4..8] != [0, 9, 0, 0] { return None; }
    let length = usize::from(r[3]);
    let end = 8 + length;
    if length == 0 || end + usize::from(r[2] == 1) > r.len() { return None; }
    if r[2] == 1 && r[end] != r[2..end].iter().fold(0, |xor, b| xor ^ b) { return None; }
    Some(&r[8..end])
}

fn reply(r: &[u8]) -> Option<Reply> {
    let data = response_data(r)?;
    if data.len() < 14 || data[..2] != [0x37, 0x38] || data[12] > 100 { return None; }
    match data[10] {
        0 => Some(Reply::Offline),
        1 | 2 => Some(Reply::Battery { percent: data[12], charging: match data[11] {
            0 => Some(false), 1 => Some(true), _ => None,
        }}),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failure { Invalid, Busy, Unavailable }

impl Failure {
    fn message(self) -> &'static str {
        match self {
            Self::Invalid => "设备状态回复无效",
            Self::Busy => "设备忙，请稍后重试",
            Self::Unavailable => UNAVAILABLE,
        }
    }
    fn outcome(self) -> trace::Outcome {
        match self {
            Self::Invalid => trace::Outcome::InvalidData,
            Self::Busy => trace::Outcome::Rejected(0xff),
            Self::Unavailable => trace::Outcome::Rejected(0),
        }
    }
}

fn receiver_reply(interface: &HidInterface, frame: &[u8]) -> Result<Reply, Failure> {
    if response_data(frame) == Some(&[0xff][..]) { return Err(Failure::Busy); }
    let state = reply(frame).ok_or(Failure::Invalid)?;
    if interface.product_id == V7_PID {
        match state {
            Reply::Battery { charging: None, .. } => return Err(Failure::Invalid),
            Reply::Battery { percent: 0, .. } => return Err(Failure::Unavailable),
            _ => (),
        }
    }
    Ok(state)
}

fn read_with(t: &Target, exchange: impl FnOnce(&[u8]) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    if t.backend != BackendId::MchoseModern || !control(t.hid()?) { return Err(UNAVAILABLE.into()); }
    let response = exchange(&request()).ok_or(UNAVAILABLE)?;
    let state = receiver_reply(t.hid()?, &response).map_err(Failure::message)?;
    let battery = match state {
        Reply::Battery { percent, charging } => Some(Battery { percent: Some(percent), charging,
            estimated_percent: None, voltage_mv: None, approximate: false, level: None }),
        Reply::Offline => None,
    };
    let mut target = t.clone();
    target.verified_identity = true;
    Ok(Snapshot { target, battery_at: battery.as_ref().map(|_| Instant::now()), battery,
        battery_unavailable: state == Reply::Offline, connected: state != Reply::Offline,
        dpi: Dpi::default(), polling_rate: Default::default() })
}

fn query_with(interface: &HidInterface, mut exchange: impl FnMut(&[u8], u32,
    &mut dyn FnMut(&[u8]) -> bool) -> Option<Vec<u8>>) -> Result<Vec<u8>, String> {
    let started = Instant::now();
    let mut remaining = 32usize;
    let mut last_failure = None;
    let mut received = 0;
    let response = exchange(&request(), 1500, &mut |frame| {
        remaining = remaining.saturating_sub(1);
        received = frame.len();
        match receiver_reply(interface, frame) {
            Ok(_) => true,
            Err(failure) => {
                last_failure = Some(failure);
                interface.product_id == V7_PID && failure != Failure::Invalid || remaining == 0
            }
        }
    });
    let result = response.as_deref().map(|frame| receiver_reply(interface, frame));
    let failure = match result { Some(Err(failure)) => Some(failure), _ => last_failure };
    let outcome = match result {
        Some(Ok(Reply::Offline)) => trace::Outcome::Offline,
        Some(Ok(_)) => trace::Outcome::Ok,
        _ => failure.map_or(trace::Outcome::NoReply, Failure::outcome),
    };
    trace::record(interface, BackendId::MchoseModern, "query-battery", 0x0900, REPORT,
        usize::from(LENGTH), received, started, outcome);
    if matches!(result, Some(Ok(_))) {
        response.ok_or_else(|| UNAVAILABLE.into())
    } else {
        Err(failure.map_or(UNAVAILABLE, Failure::message).into())
    }
}

fn read_channel(interface: &HidInterface, target: &Target) -> Result<Snapshot, String> {
    let mut device = HidDevice::open(interface)?;
    let response = query_with(interface, |packet, timeout, accept|
        device.transact(packet, timeout, accept))?;
    read_with(target, |_| Some(response))
}

impl DeviceBackend for MchoseModern {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        let mut targets: Vec<_> = candidates(interfaces).into_iter().filter(|i| proven(api, i))
            .map(|i| target(i, interfaces)).collect();
        if interfaces.iter().any(|interface| interface.product_id == V7_PID && management(interface)) {
            append_v7_targets_with(&mut targets, crate::hid::enumerate_all_report(api),
                v7_parent, |interface| proven(api, interface));
        }
        Ok(targets)
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        let i = t.hid()?;
        if t.backend != BackendId::MchoseModern || !control(i) { return Err(UNAVAILABLE.into()); }
        if i.product_id == V7_PID {
            let parent = v7_parent(i).ok_or(UNAVAILABLE)?;
            let lock = channel::lock(&parent);
            let _guard = lock.lock().unwrap_or_else(|poison| poison.into_inner());
            let report = crate::hid::enumerate_all_report(api)?;
            if unreadable_v7(&report.skipped) { return Err(UNAVAILABLE.into()); }
            let selected = select_v7_with(&report.interfaces, v7_parent, |interface| proven(api, interface));
            let (current, _) = selected.into_iter().find(|(interface, current_parent)|
                current_parent == &parent && interface.path.eq_ignore_ascii_case(&i.path))
                .ok_or(UNAVAILABLE)?;
            let current_target = target(current, &report.interfaces);
            if current_target.key != t.key { return Err(UNAVAILABLE.into()); }
            return read_channel(current, &current_target);
        }
        if !proven(api, i) { return Err(UNAVAILABLE.into()); }
        let lock = channel::lock(&channel::collection_group(i));
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        read_channel(i, t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(pid: u16, instance: &str) -> HidInterface {
        HidInterface { path: format!(r"\\?\hid#vid_3837&pid_{pid:04x}&mi_02&col01#8&{instance}&0001#{{hid}}"),
            vendor_id: 0x3837, product_id: pid, usage_page: 0xff01, usage: 1,
            input_len: LENGTH, output_len: LENGTH, feature_len: 0,
            product: "USB Receiver".into(), serial: "shared".into() }
    }

    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut r = request().to_vec();
        r[3] = payload.len() as u8;
        r[8..8 + payload.len()].copy_from_slice(payload);
        checksum(&mut r);
        r
    }

    fn checksum(r: &mut [u8]) {
        let end = 8 + usize::from(r[3]);
        r[end] = r[2..end].iter().fold(0, |xor, b| xor ^ b);
    }

    fn captured_payload() -> Vec<u8> {
        // Public Ultra+ payload; framing and zero padding are reconstructed.
        vec![0x37, 0x38, 0x26, 0x40, 4, 0, 0, 0, 0, 0x10, 2, 1, 85, 0, 8, 0xe4]
    }

    #[test]
    fn device_info_payload_preserves_percentage_charge_and_does_not_infer_model_from_pid() {
        let r = frame(&captured_payload());
        assert_eq!(r[24], 0xdf);
        assert_eq!(reply(&r), Some(Reply::Battery { percent: 85, charging: Some(true) }));
        let mut data = captured_payload(); data[11] = 0; data[12] = 42;
        let r = frame(&data);
        assert_eq!(r[24], 0xa1);
        assert_eq!(reply(&r), Some(Reply::Battery { percent: 42, charging: Some(false) }));
        let mut i = interface(0x1014, "one");
        i.product = "MCHOSE K7 V2 Ultra+".into();
        let sample = read_with(&target(&i, &[]), |_| Some(r)).unwrap();
        assert_eq!(sample.target.name, i.product);
        assert!(sample.dpi.stages.is_empty() && !sample.dpi.writable);
        assert_eq!(target(&interface(0x1014, "one"), &[]).name, "MCHOSE Mouse");
    }

    #[test]
    fn receiver_generations_and_keyboard_protocols_are_isolated() {
        for pid in [0x1014, 0x1016, 0x1018] {
            let i = interface(pid, "one"); assert!(control(&i));
            let other = interface(pid, "two");
            assert_ne!(target(&i, &[]).key, target(&other, &[]).key);
            assert_eq!(candidates(&[i.clone(), other]).len(), 2);
            let mut sibling = i.clone();
            sibling.path = sibling.path.replace("col01", "col02").replace("&0001#", "&0002#");
            assert!(candidates(&[i, sibling]).is_empty());
        }
        for pid in [0x100b, 0x1020, 0x1015, 0x3033, 0x600a, 0x4026] {
            assert!(!control(&interface(pid, "one")));
        }
        for (page, usage, input, output) in [(0xff0b, 0xc4, 64, 64), (0xff01, 1, 41, 41),
            (0xff01, 1, 65, 65), (0xff80, 1, 64, 64), (1, 2, 64, 64)] {
            let mut i = interface(0x1018, "one");
            i.usage_page = page; i.usage = usage; i.input_len = input; i.output_len = output;
            assert!(!control(&i));
        }
    }

    #[test]
    fn windows_array_exception_is_limited_to_documented_pro_and_ultra_receiver() {
        let mut i = interface(0x1018, "one");
        for name in ["MCHOSE A7 V3 Ultra+", "MCHOSE A7 V3 Pro"] {
            i.product = name.into(); assert!(windows_array_variant(&i));
        }
        i.product = "MCHOSE K7 V2 Ultra+".into(); assert!(!windows_array_variant(&i));
        i.product = "MCHOSE A7 V3 Ultra+".into();
        i.product_id = 0x1014; assert!(windows_array_variant(&i));
        i.product = "MCHOSE A7 V3 Pro".into(); assert!(!windows_array_variant(&i));
        i.product = "MCHOSE A7 V3 Ultra+".into();
        i.product_id = 0x1018;
        i.path = i.path.replace("mi_02", "mi_01"); assert!(!windows_array_variant(&i));
        i.path = i.path.replace("mi_01", "mi_02"); i.input_len = 41;
        assert!(!windows_array_variant(&i));
    }

    #[test]
    fn malformed_busy_echo_and_other_commands_cannot_publish_battery() {
        let valid = frame(&captured_payload());
        assert!(reply(&request()).is_none());
        assert!(reply(&frame(&[0xff])).is_none());
        for length in 0..64 { assert!(reply(&valid[..length]).is_none()); }
        for (offset, value) in [(0, 0x11), (1, 2), (2, 3), (3, 13), (3, 56),
            (4, 4), (5, 0), (6, 1), (7, 1), (8, 0x36), (18, 3), (20, 101), (24, 0)] {
            let mut r = valid.clone(); r[offset] = value;
            assert!(reply(&r).is_none(), "offset {offset}");
        }
        for (offset, value) in [(8, 0x36), (18, 0xff), (20, 101)] {
            let mut r = valid.clone(); r[offset] = value; checksum(&mut r);
            assert!(reply(&r).is_none());
        }
        let mut no_checksum = valid.clone(); no_checksum[2] = 0;
        assert!(reply(&no_checksum).is_some());
        for level in 0..=100 {
            let mut r = valid.clone(); r[20] = level; checksum(&mut r);
            assert_eq!(reply(&r), Some(Reply::Battery { percent: level, charging: Some(true) }));
        }
        let mut unknown_charge = valid; unknown_charge[19] = 0xff; checksum(&mut unknown_charge);
        assert_eq!(reply(&unknown_charge), Some(Reply::Battery { percent: 85, charging: None }));
    }

    #[test]
    fn offline_clears_frozen_level_and_background_wake_recovers_new_sample() {
        let t = target(&interface(0x1018, "one"), &[]);
        let mut data = captured_payload(); data[10] = 0;
        let offline = read_with(&t, |_| Some(frame(&data))).unwrap();
        assert!(!offline.connected && offline.battery_unavailable);
        assert!(offline.battery.is_none() && offline.battery_at.is_none());
        assert!(read_with(&t, |_| None).is_err());
        data[10] = 2; data[12] = 84;
        let awake = read_with(&t, |p| { assert_eq!(p, request()); Some(frame(&data)) }).unwrap();
        assert!(awake.connected && !awake.battery_unavailable && awake.battery_at.is_some());
        assert_eq!(awake.battery.unwrap().percent, Some(84));
        let mut wrong = t; wrong.backend = BackendId::MchoseMouse;
        assert!(read_with(&wrong, |_| panic!("legacy protocol must not send")).is_err());
    }

    fn v7(instance: &str) -> HidInterface {
        let mut receiver = interface(V7_PID, instance);
        receiver.product = V7_NAME.into();
        receiver
    }

    fn v7_frame(percent: u8, charging: u8, connection: u8) -> Vec<u8> {
        let mut data = captured_payload();
        data[2..4].copy_from_slice(&0x403eu16.to_le_bytes());
        data[10] = connection;
        data[11] = charging;
        data[12] = percent;
        frame(&data)
    }

    fn parent(instance: &str) -> String {
        format!(r"usb\vid_3837&pid_101d\{instance}")
    }

    #[test]
    fn v7_requires_exact_name_interface_capacity_and_matching_path_identity() {
        let receiver = v7("exact");
        assert!(control(&receiver));
        assert!(candidates(&[receiver.clone()]).is_empty());
        assert!(!windows_array_variant(&receiver));
        for change in [
            |device: &mut HidInterface| device.product = "MCHOSE V7".into(),
            |device: &mut HidInterface| device.product = "MCHOSE V7 Pro ".into(),
            |device: &mut HidInterface| device.vendor_id = 0x5253,
            |device: &mut HidInterface| device.product_id = 0x403e,
            |device: &mut HidInterface| device.usage_page = 0xff0b,
            |device: &mut HidInterface| device.usage = 2,
            |device: &mut HidInterface| device.input_len = 65,
            |device: &mut HidInterface| device.output_len = 63,
            |device: &mut HidInterface| device.path = device.path.replace("pid_101d", "pid_1018"),
            |device: &mut HidInterface| device.path = device.path.replace("mi_02", "mi_02&mi_03"),
            |device: &mut HidInterface| device.path = device.path.replace("col01", "colGG"),
            |device: &mut HidInterface| device.path.push('\0'),
        ] {
            let mut wrong = receiver.clone();
            change(&mut wrong);
            assert!(!control(&wrong), "{}", wrong.path);
            assert!(read_with(&target(&wrong, &[]), |_| panic!("unproven target must not send")).is_err());
        }
        let mut uppercase = receiver;
        uppercase.path.make_ascii_uppercase();
        assert!(control(&uppercase));
    }

    #[test]
    fn v7_discovery_joins_cross_interface_siblings_by_physical_parent() {
        let first = v7("first");
        let mut duplicate = first.clone();
        duplicate.path = duplicate.path.replace("mi_02", "mi_04").replace("&first&", "&alias&");
        duplicate.product = "unknown receiver".into();
        duplicate.input_len = 41;
        let second = v7("second");
        for interfaces in [vec![first.clone(), duplicate.clone(), second.clone()],
            vec![second.clone(), duplicate.clone(), first.clone()]] {
            let selected = select_v7_with(&interfaces, |device| Some(
                if device.path == second.path { parent("second") }
                else { parent("first").to_ascii_uppercase() }), |_| true);
            assert_eq!(selected.len(), 1);
            assert_eq!(selected[0].0.path, second.path);
            assert_eq!(selected[0].1, parent("second"));
        }
        let mut input_sibling = first.clone();
        input_sibling.usage_page = 1;
        let interfaces = [first.clone(), input_sibling, second.clone()];
        let selected = select_v7_with(&interfaces, |device| Some(
            parent(if device.path == first.path { "first" } else { "second" })), |_| true);
        assert_eq!(selected.len(), 2);
        assert_ne!(target(selected[0].0, &interfaces).key, target(selected[1].0, &interfaces).key);
        assert_eq!(first.serial, second.serial);
    }

    #[test]
    fn v7_unknown_topology_stops_before_descriptor_checks_and_bad_proof_is_isolated() {
        let first = v7("first");
        let second = v7("second");
        let interfaces = [first.clone(), second.clone()];
        for invalid in [None, Some("usb\\vid_3837&pid_1018\\wrong"),
            Some("usb\\vid_3837&pid_101d&mi_02\\wrong"),
            Some("usb\\vid_3837&pid_101d\\"), Some("usb\\vid_3837&pid_101d\\bad\\extra")] {
            assert!(select_v7_with(&interfaces, |device| {
                if device.path == first.path { Some(parent("first")) }
                else { invalid.map(str::to_string) }
            }, |_| panic!("unknown physical topology must not reach report proof")).is_empty());
        }
        let selected = select_v7_with(&interfaces, |device| Some(parent(
            if device.path == first.path { "first" } else { "second" })),
            |device| device.path == second.path);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].0.path, second.path);
        assert!(unreadable_v7(&[crate::hid::Skipped {
            path: first.path.to_ascii_uppercase(), reason: "cannot read caps".into(),
        }]));
        assert!(!unreadable_v7(&[crate::hid::Skipped {
            path: first.path.replace("pid_101d", "pid_1018"), reason: "cannot read caps".into(),
        }]));
    }

    #[test]
    fn v7_rescan_failure_preserves_existing_a7_targets_without_probing() {
        let existing = interface(0x1018, "existing");
        let receiver = v7("new");
        for report in [Err("enumeration failed".into()), Ok(crate::hid::EnumerateReport {
            interfaces: vec![receiver.clone()],
            skipped: vec![crate::hid::Skipped { path: receiver.path.clone(), reason: "caps unavailable".into() }],
        })] {
            let mut targets = vec![target(&existing, &[])];
            let original_key = targets[0].key.clone();
            append_v7_targets_with(&mut targets, report,
                |_| panic!("failed scan cannot establish physical topology"),
                |_| panic!("failed scan cannot prove reports"));
            assert_eq!(targets.len(), 1);
            assert_eq!(targets[0].key, original_key);
            assert_eq!(targets[0].hid().unwrap().path, existing.path);
        }
        let mut targets = vec![target(&existing, &[])];
        append_v7_targets_with(&mut targets, Ok(crate::hid::EnumerateReport {
            interfaces: vec![receiver], skipped: vec![],
        }), |_| Some(parent("new")), |_| true);
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].hid().unwrap().path, existing.path);
        assert_eq!(targets[1].name, V7_NAME);
    }

    #[test]
    fn v7_uses_numbered_64_byte_battery_report_and_never_exposes_settings() {
        let receiver = v7("status");
        let device = target(&receiver, &[]);
        assert_eq!(request().len(), 64);
        assert_eq!(&request()[..9], &[0x4d, 1, 1, 0, 0, 9, 0, 0, 8]);
        for percent in 1..=100 {
            for charging in 0..=1 {
                let sample = read_with(&device, |packet| {
                    assert_eq!(packet, request());
                    Some(v7_frame(percent, charging, 2))
                }).unwrap();
                assert_eq!(sample.target.name, V7_NAME);
                assert!(sample.target.verified_identity && sample.connected && sample.battery_at.is_some());
                let battery = sample.battery.unwrap();
                assert_eq!(battery.percent, Some(percent));
                assert_eq!(battery.charging, Some(charging == 1));
                assert!(sample.dpi.current.is_none() && sample.dpi.stages.is_empty());
                assert!(!sample.dpi.writable && !sample.dpi.stage_writable && !sample.polling_rate.writable);
            }
        }
        let valid = v7_frame(85, 1, 2);
        for length in 0..=65 {
            let mut malformed = valid.clone();
            malformed.resize(length, 0);
            assert_eq!(receiver_reply(&receiver, &malformed).is_ok(), length == 64);
        }
        let mut zero_slot = vec![0];
        zero_slot.extend_from_slice(&valid);
        assert_eq!(receiver_reply(&receiver, &zero_slot), Err(Failure::Invalid));
    }

    #[test]
    fn v7_placeholders_and_unknown_charge_fail_without_changing_existing_protocol_semantics() {
        let receiver = v7("placeholders");
        let other = interface(0x1018, "legacy");
        for charging in 0..=1 {
            let zero = v7_frame(0, charging, 2);
            assert_eq!(receiver_reply(&receiver, &zero), Err(Failure::Unavailable));
            assert_eq!(receiver_reply(&other, &zero), Ok(Reply::Battery {
                percent: 0, charging: Some(charging == 1),
            }));
        }
        for charging in 2..=255 {
            let unknown = v7_frame(37, charging, 1);
            assert_eq!(receiver_reply(&receiver, &unknown), Err(Failure::Invalid));
            assert_eq!(receiver_reply(&other, &unknown), Ok(Reply::Battery { percent: 37, charging: None }));
        }
        for percent in 101..=255 {
            assert_eq!(receiver_reply(&receiver, &v7_frame(percent, 0, 1)), Err(Failure::Invalid));
        }
        for connection in 3..=255 {
            assert_eq!(receiver_reply(&receiver, &v7_frame(37, 0, connection)), Err(Failure::Invalid));
        }
        let mut wrong_vid = v7_frame(37, 0, 1);
        wrong_vid[8] ^= 1;
        checksum(&mut wrong_vid);
        assert_eq!(receiver_reply(&receiver, &wrong_vid), Err(Failure::Invalid));
    }

    fn query_frames(interface: &HidInterface, frames: Vec<Vec<u8>>) -> (Result<Vec<u8>, String>, usize, usize) {
        let mut sends = 0;
        let mut reads = 0;
        let result = query_with(interface, |packet, timeout, accept| {
            sends += 1;
            assert_eq!(packet, request());
            assert_eq!(timeout, 1500);
            for frame in &frames {
                reads += 1;
                if accept(frame) { return Some(frame.clone()); }
            }
            None
        });
        (result, sends, reads)
    }

    #[test]
    fn busy_invalid_and_silent_replies_are_reported_without_retransmission() {
        let receiver = v7("errors");
        let busy = frame(&[0xff]);
        assert_eq!(&busy[..10], &[0x4d, 1, 1, 1, 0, 9, 0, 0, 0xff, 0xf6]);
        for (frames, error, expected_reads) in [
            (vec![], UNAVAILABLE, 0),
            (vec![busy.clone()], Failure::Busy.message(), 1),
            (vec![request().to_vec()], Failure::Invalid.message(), 1),
            (vec![v7_frame(0, 0, 2)], UNAVAILABLE, 1),
            (vec![v7_frame(37, 2, 2)], Failure::Invalid.message(), 1),
            (vec![request().to_vec(); 40], Failure::Invalid.message(), 32),
        ] {
            let (result, sends, reads) = query_frames(&receiver, frames);
            assert_eq!(result, Err(error.into()));
            assert_eq!(sends, 1);
            assert_eq!(reads, expected_reads);
        }
        let mut malformed_busy = busy;
        malformed_busy[9] ^= 1;
        assert_eq!(query_frames(&receiver, vec![malformed_busy]).0, Err(Failure::Invalid.message().into()));
        let valid = v7_frame(37, 1, 2);
        let (result, sends, reads) = query_frames(&receiver, vec![request().to_vec(), valid.clone()]);
        assert_eq!(result.unwrap(), valid);
        assert_eq!((sends, reads), (1, 2));
        let previous_receiver = interface(0x1018, "previous");
        let (result, sends, reads) = query_frames(&previous_receiver, vec![frame(&[0xff]), valid.clone()]);
        assert_eq!(result.unwrap(), valid);
        assert_eq!((sends, reads), (1, 2));
    }

    #[test]
    fn v7_failure_and_recovery_keep_two_same_model_devices_independent() {
        let first = target(&v7("first"), &[]);
        let second = target(&v7("second"), &[]);
        assert_ne!(first.key, second.key);
        let sample = read_with(&first, |_| Some(v7_frame(85, 1, 2))).unwrap();
        let measured = sample.battery_at;
        assert!(read_with(&first, |_| None).is_err());
        assert!(read_with(&first, |_| Some(v7_frame(0, 0, 2))).is_err());
        let other = read_with(&second, |_| Some(v7_frame(37, 0, 1))).unwrap();
        assert_eq!(sample.battery_at, measured);
        assert_eq!(sample.battery.unwrap().percent, Some(85));
        assert_eq!(other.battery.unwrap().percent, Some(37));
        let offline = read_with(&first, |_| Some(v7_frame(85, 1, 0))).unwrap();
        assert!(!offline.connected && offline.battery_unavailable);
        assert!(offline.battery.is_none() && offline.battery_at.is_none());
        let recovered = read_with(&first, |_| Some(v7_frame(84, 0, 2))).unwrap();
        assert!(recovered.connected && !recovered.battery_unavailable);
        assert_eq!(recovered.battery.unwrap().percent, Some(84));
        assert!(recovered.battery_at >= measured);
    }

    #[test]
    fn ultra_1014_windows_variant_stays_exact_and_preserves_public_reply() {
        let mut receiver = interface(0x1014, "ultra");
        receiver.product = "MCHOSE A7 V3 Ultra+".into();
        assert!(windows_array_variant(&receiver));
        let mut second = receiver.clone();
        second.path = second.path.replace("&ultra&", "&second&");
        let interfaces = [receiver.clone(), second.clone()];
        assert_eq!(candidates(&interfaces).len(), 2);
        assert_ne!(target(&receiver, &interfaces).key, target(&second, &interfaces).key);
        let mut captured = vec![0x4d, 1, 1, 0x10, 0, 9, 0, 0, 0x37, 0x38, 0x26, 0x40,
            4, 0, 0, 0, 0, 0x10, 2, 0, 0x2a, 0, 8, 0xe4, 0xa1];
        captured.resize(64, 0);
        let sample = read_with(&target(&receiver, &interfaces), |_| Some(captured.clone())).unwrap();
        assert_eq!(sample.battery.unwrap().percent, Some(42));
        captured[24] ^= 1;
        assert!(read_with(&target(&receiver, &interfaces), |_| Some(captured)).is_err());
        for change in [
            |device: &mut HidInterface| device.product_id = 0x1016,
            |device: &mut HidInterface| device.product_id = V7_PID,
            |device: &mut HidInterface| device.product = "MCHOSE K7 V2 Ultra+".into(),
            |device: &mut HidInterface| device.usage_page = 0xff0b,
            |device: &mut HidInterface| device.path = device.path.replace("mi_02", "mi_01"),
            |device: &mut HidInterface| device.output_len = 41,
            |device: &mut HidInterface| device.input_len = 127,
        ] {
            let mut wrong = receiver.clone();
            change(&mut wrong);
            assert!(!windows_array_variant(&wrong));
        }
    }
}
