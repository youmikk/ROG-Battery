//! EWEADN X23 Pro receiver: read-only sensor collection Input Report pushes.
use super::*;
use crate::{hid::HidDevice, win32::*};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use winapi::um::cfgmgr32::{CM_Get_Device_IDW, CM_Get_Parent, CM_Locate_DevNodeW};

pub struct EweadnX23;

const VENDOR: u16 = 0x04f3;
const PRODUCT: u16 = 0x026f;
const NAME: &str = "X23 Pro Receiver";
const WINDOWS_REPORT_BYTES: u16 = 32;
const FIRST_WAIT_MS: u32 = 1_200;
const UNAVAILABLE: &str = "X23 Pro 暂无新的接收器状态帧";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == VENDOR && i.product_id == PRODUCT && i.product == NAME
        && i.usage_page == 0x000a && i.usage == 0 && i.input_len == WINDOWS_REPORT_BYTES
        && i.path.to_ascii_lowercase().contains("&mi_01&col04#")
}

fn device_instance(i: &HidInterface) -> Option<String> {
    if !management(i) { return None; }
    let path = i.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid" || parts[2].is_empty()
        || parts[3].is_empty() || path.contains('\0')
        || parts[1].contains('\\') || parts[2].contains('\\') { return None; }
    let fields: Vec<_> = parts[1].split('&').collect();
    for required in ["vid_04f3", "pid_026f", "mi_01", "col04"] {
        if fields.iter().filter(|field| **field == required).count() != 1 { return None; }
    }
    if fields.iter().filter(|field| field.starts_with("vid_")).count() != 1
        || fields.iter().filter(|field| field.starts_with("pid_")).count() != 1
        || fields.iter().filter(|field| field.starts_with("mi_")).count() != 1
        || fields.iter().filter(|field| field.starts_with("col")).count() != 1 { return None; }
    Some(format!("hid\\{}\\{}", parts[1], parts[2]))
}

fn physical_parent(name: &str) -> bool {
    let parts: Vec<_> = name.split('\\').collect();
    if parts.len() != 3 || parts[0] != "usb" || parts[2].is_empty() { return false; }
    let fields: Vec<_> = parts[1].split('&').collect();
    fields.len() == 2 && fields.contains(&"vid_04f3") && fields.contains(&"pid_026f")
}

fn usb_parent(i: &HidInterface) -> Option<String> {
    let mut instance = to_wide(&device_instance(i)?);
    let mut node = 0;
    if unsafe { CM_Locate_DevNodeW(&mut node, instance.as_mut_ptr(), 0) } != 0 { return None; }
    for _ in 0..8 {
        let mut parent = 0;
        if unsafe { CM_Get_Parent(&mut parent, node, 0) } != 0 { return None; }
        let mut buffer = [0u16; 512];
        if unsafe { CM_Get_Device_IDW(parent, buffer.as_mut_ptr(), buffer.len() as u32, 0) } != 0 {
            return None;
        }
        let length = buffer.iter().position(|c| *c == 0)?;
        let name = String::from_utf16(&buffer[..length]).ok()?.to_ascii_lowercase();
        if physical_parent(&name) { return Some(name); }
        node = parent;
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    report_id: u8,
    windows_bytes: u16,
}

fn prove_input(api: &HidApi, i: &HidInterface) -> Option<Layout> {
    if !management(i) || device_instance(i).is_none() { return None; }
    let report_id = descriptor::Descriptor::open(api, i)?
        .single_input_byte_report_id(i, WINDOWS_REPORT_BYTES)?;
    [0, 4].contains(&report_id).then_some(Layout {
        report_id,
        windows_bytes: WINDOWS_REPORT_BYTES,
    })
}

fn select_with<'a>(
    interfaces: &'a [HidInterface],
    mut parent: impl FnMut(&HidInterface) -> Option<String>,
    mut proof: impl FnMut(&HidInterface) -> Option<Layout>,
) -> Vec<(&'a HidInterface, String, Layout)> {
    let mut groups: BTreeMap<String, Vec<&HidInterface>> = BTreeMap::new();
    for i in interfaces.iter().filter(|i| management(i)) {
        let Some(key) = parent(i).filter(|key| physical_parent(key)) else {
            return Vec::new();
        };
        groups.entry(key).or_default().push(i);
    }
    let mut out = Vec::new();
    for (parent, members) in groups {
        let [i] = members.as_slice() else { continue; };
        if let Some(layout) = proof(i) { out.push((*i, parent, layout)); }
    }
    out
}

fn target(i: &HidInterface, parent: &str) -> Target {
    let mut target = Target::new(BackendId::EweadnX23, i, 0);
    target.key = crate::device::stable_key(&format!("eweadn-x23:{parent}"));
    target.name = "EWEADN X23 Pro".into();
    target.family = DeviceFamily::Mouse;
    target.verified_identity = true;
    target
}

fn percent(frame: &[u8], layout: Layout) -> Option<u8> {
    if frame.len() != usize::from(layout.windows_bytes) || frame[0] != layout.report_id
        || frame.iter().all(|byte| *byte == 0) || frame[4] > 100 { return None; }
    Some(frame[4])
}

fn snapshot(target: &Target, level: u8, measured_at: Instant) -> Snapshot {
    Snapshot {
        target: target.clone(),
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
        battery_at: Some(measured_at),
        battery_unavailable: false,
    }
}

fn current(
    api: &HidApi,
    original: &Target,
) -> Result<(HidInterface, String, Layout), String> {
    let interfaces = crate::hid::enumerate_all_report(api)?.interfaces;
    let candidates = select_with(&interfaces, usb_parent, |i| prove_input(api, i));
    candidates
        .into_iter()
        .find(|(i, _, _)| original.hid().is_ok_and(|target|
            i.path.eq_ignore_ascii_case(&target.path)))
        .map(|(i, parent, layout)| (i.clone(), parent, layout))
        .ok_or_else(|| UNAVAILABLE.into())
}

impl DeviceBackend for EweadnX23 {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(select_with(interfaces, usb_parent, |i| prove_input(api, i))
            .into_iter()
            .map(|(i, parent, _)| target(i, &parent))
            .collect())
    }

    fn read(&self, api: &HidApi, original: &Target, _: bool) -> Result<Snapshot, String> {
        if original.backend != BackendId::EweadnX23 { return Err(UNAVAILABLE.into()); }
        let (interface, parent, layout) = current(api, original)?;
        let current_target = target(&interface, &parent);
        if current_target.key != original.key { return Err(UNAVAILABLE.into()); }
        let mut input = HidDevice::open_read_only(&interface)
            .map_err(|_| "X23 Pro 接收器不可读取".to_string())?;
        let deadline = Instant::now() + Duration::from_millis(u64::from(FIRST_WAIT_MS));
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let Some(frame) = input.read_report(left.as_millis().max(1) as u32) else {
                break;
            };
            if let Some(level) = percent(&frame, layout) {
                return Ok(snapshot(&current_target, level, Instant::now()));
            }
        }
        Err(UNAVAILABLE.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface() -> HidInterface {
        HidInterface {
            path: r"\\?\hid#vid_04f3&pid_026f&mi_01&col04#7&fixture&0&0000#{hid}".into(),
            serial: String::new(),
            product: NAME.into(),
            vendor_id: VENDOR,
            product_id: PRODUCT,
            usage_page: 0x000a,
            usage: 0,
            input_len: WINDOWS_REPORT_BYTES,
            output_len: WINDOWS_REPORT_BYTES,
            feature_len: 0,
        }
    }
    fn parent() -> String { r"usb\vid_04f3&pid_026f\fixture".into() }
    fn frame(report_id: u8, level: u8) -> Vec<u8> {
        let mut frame = vec![0; usize::from(WINDOWS_REPORT_BYTES)];
        frame[0] = report_id;
        frame[1..=3].copy_from_slice(&[1, 2, 3]);
        frame[4] = level;
        frame
    }

    #[test]
    fn only_the_exact_x23_receiver_collection_is_selected() {
        let exact = interface();
        assert!(management(&exact));
        for mutate in [
            |i: &mut HidInterface| i.product = "X23 Receiver".into(),
            |i: &mut HidInterface| i.usage_page = 1,
            |i: &mut HidInterface| i.path = i.path.replace("col04", "col03"),
            |i: &mut HidInterface| i.path = i.path.replace("mi_01", "mi_02"),
            |i: &mut HidInterface| i.input_len = 31,
        ] {
            let mut wrong = exact.clone();
            mutate(&mut wrong);
            assert!(!management(&wrong));
        }
    }

    #[test]
    fn numbered_and_unnumbered_windows_reports_use_the_same_battery_offset() {
        for id in [0, 4] {
            let layout = Layout { report_id: id, windows_bytes: WINDOWS_REPORT_BYTES };
            for level in [0, 1, 50, 100] {
                let mut raw = frame(id, level);
                if level == 0 { raw[1] = 1; }
                assert_eq!(percent(&raw, layout), Some(level));
            }
            let mut wrong_id = frame(id, 50);
            wrong_id[0] = if id == 0 { 4 } else { 0 };
            assert!(percent(&wrong_id, layout).is_none());
            assert!(percent(&frame(id, 101), layout).is_none());
            assert!(percent(&vec![0; usize::from(WINDOWS_REPORT_BYTES)], layout).is_none());
            assert!(percent(&frame(id, 50)[..31], layout).is_none());
        }
    }

    #[test]
    fn duplicate_or_parentless_candidates_never_open_an_input_stream() {
        let first = interface();
        let mut second = first.clone();
        second.path = second.path.replace("fixture", "other");
        let layout = Layout { report_id: 4, windows_bytes: WINDOWS_REPORT_BYTES };
        assert_eq!(
            select_with(&[first.clone()], |_| Some(parent()), |_| Some(layout)).len(),
            1
        );
        assert!(select_with(&[first.clone()], |_| None, |_| Some(layout)).is_empty());
        assert!(select_with(
            &[first.clone(), second],
            |_| Some(parent()),
            |_| Some(layout)
        )
        .is_empty());
    }
}
