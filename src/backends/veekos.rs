//! Veekos Shine 60 2.4G receiver status page; read-only Feature access.
use super::*;
use super::descriptor::FeatureReportProof;
use crate::win32::to_wide;
use std::collections::BTreeMap;
use winapi::um::cfgmgr32::{CM_Get_Device_IDW, CM_Get_Parent, CM_Locate_DevNodeW};

pub struct VeekosShine60;

const VENDOR: u16 = 0x3151;
const PRODUCT: u16 = 0x5026;
const NAME: &str = "Veekos 2.4G Wireless Keyboard";
const REPORT_BYTES: u16 = 65;

fn management(i: &HidInterface) -> bool {
    i.vendor_id == VENDOR && i.product_id == PRODUCT && i.product == NAME
        && i.usage_page == 0xffff && i.usage == 2 && i.input_len == 0
        && i.output_len == 0 && i.feature_len == REPORT_BYTES
        && i.path.to_ascii_lowercase().contains("&mi_02")
}

fn report_proof(api: &HidApi, interface: &HidInterface) -> FeatureReportProof {
    if !management(interface) { return FeatureReportProof::Unknown; }
    let Some(descriptor) = descriptor::Descriptor::open(api, interface)
        else { return FeatureReportProof::Unknown; };
    descriptor.single_feature_report(interface, REPORT_BYTES, &[0x83, 0])
}

fn device_instance(interface: &HidInterface) -> Option<String> {
    if !management(interface) { return None; }
    let path = interface.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid" || parts[2].is_empty()
        || parts[3].is_empty() || path.contains('\0')
        || parts[1].contains('\\') || parts[2].contains('\\') { return None; }
    let fields: Vec<_> = parts[1].split('&').collect();
    for required in ["vid_3151", "pid_5026", "mi_02"] {
        if fields.iter().filter(|field| **field == required).count() != 1 { return None; }
    }
    for prefix in ["vid_", "pid_", "mi_"] {
        if fields.iter().filter(|field| field.starts_with(prefix)).count() != 1 { return None; }
    }
    let collections: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix("col")).collect();
    if collections.len() > 1 || collections.iter().any(|index|
        index.len() != 2 || !index.bytes().all(|byte| byte.is_ascii_hexdigit())) { return None; }
    Some(format!("hid\\{}\\{}", parts[1], parts[2]))
}

fn physical_parent(name: &str) -> bool {
    let parts: Vec<_> = name.split('\\').collect();
    if parts.len() != 3 || parts[0] != "usb" || parts[2].is_empty() || name.contains('\0') {
        return false;
    }
    let fields: Vec<_> = parts[1].split('&').collect();
    fields.len() == 2 && fields.contains(&"vid_3151") && fields.contains(&"pid_5026")
}

fn usb_parent(interface: &HidInterface) -> Option<String> {
    let mut instance = to_wide(&device_instance(interface)?);
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
        if physical_parent(&name) { return Some(name); }
        node = parent;
    }
    None
}

fn select_with<'a>(interfaces: &'a [HidInterface],
    mut parent: impl FnMut(&HidInterface) -> Option<String>,
    mut proof: impl FnMut(&HidInterface) -> FeatureReportProof) -> Vec<(&'a HidInterface, u8)> {
    let mut groups: BTreeMap<String, Vec<&HidInterface>> = BTreeMap::new();
    for interface in interfaces.iter().filter(|interface| management(interface)) {
        if device_instance(interface).is_none() { return Vec::new(); }
        let Some(key) = parent(interface).map(|key| key.to_ascii_lowercase())
            .filter(|key| physical_parent(key)) else { return Vec::new(); };
        groups.entry(key).or_default().push(interface);
    }
    let mut selected = Vec::new();
    'groups: for members in groups.into_values() {
        let mut candidate = None;
        for interface in members {
            match proof(interface) {
                FeatureReportProof::Match(report_id) => {
                    if candidate.is_some() { continue 'groups; }
                    candidate = Some((interface, report_id));
                }
                FeatureReportProof::Mismatch => {}
                FeatureReportProof::Unknown => continue 'groups,
            }
        }
        if let Some(candidate) = candidate { selected.push(candidate); }
    }
    selected
}

fn target(interface: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::VeekosShine60, interface, 0);
    target.name = "Veekos Shine 60".into();
    target.family = DeviceFamily::Keyboard;
    target.key = crate::device::stable_key(&format!(
        "veekos-shine60:{}:{}",
        interface.serial.trim(),
        interface.path
    ));
    target.verified_identity = true;
    target
}

fn candidates(api: &HidApi, interfaces: &[HidInterface]) -> Vec<(Target, u8)> {
    select_with(interfaces, usb_parent, |interface| report_proof(api, interface))
        .into_iter().map(|(interface, report_id)| (target(interface), report_id)).collect()
}

fn decode(frame: &[u8], id: u8) -> Option<u8> {
    if frame.len() != usize::from(REPORT_BYTES) { return None; }
    (frame[0] == id
        && frame[1] == 0
        && frame[2] <= 100
        && frame[3] == 100)
        .then_some(frame[2])
}

fn snapshot(target: &Target, level: u8) -> Snapshot {
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
        battery_at: Some(std::time::Instant::now()),
        battery_unavailable: false,
    }
}

fn read_value(api: &HidApi, original: &Target) -> Result<Snapshot, String> {
    let interfaces = crate::hid::enumerate_all_report(api)?.interfaces;
    let (target, id) = candidates(api, &interfaces)
        .into_iter()
        .find(|(target, _)|
            original.hid().is_ok_and(|source| target.hid().is_ok_and(|candidate|
                source.path.eq_ignore_ascii_case(&candidate.path))))
        .ok_or_else(|| "Shine 60 状态通道未确认".to_string())?;
    if target.key != original.key { return Err("Shine 60 身份已变化".into()); }
    let interface = target.hid().map_err(|_| "Shine 60 状态通道无效".to_string())?;
    let frame = crate::hid::query_report(interface, id, true, 800)
        .ok_or_else(|| "Shine 60 无有效 Feature 回复".to_string())?;
    let level = decode(&frame, id).ok_or_else(|| "Shine 60 状态回复无效".to_string())?;
    Ok(snapshot(&target, level))
}

impl DeviceBackend for VeekosShine60 {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(api, interfaces).into_iter().map(|(target, _)| target).collect())
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        if target.backend != BackendId::VeekosShine60 { return Err("Shine 60 目标无效".into()); }
        read_value(api, target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface() -> HidInterface {
        HidInterface {
            path: r"\\?\hid#vid_3151&pid_5026&mi_02&col01#7&fixture&0&0000#{hid}".into(),
            serial: String::new(),
            product: NAME.into(),
            vendor_id: VENDOR,
            product_id: PRODUCT,
            usage_page: 0xffff,
            usage: 2,
            input_len: 0,
            output_len: 0,
            feature_len: REPORT_BYTES,
        }
    }

    fn frame(id: u8, level: u8) -> Vec<u8> {
        let mut frame = vec![0; usize::from(REPORT_BYTES)];
        frame[..4].copy_from_slice(&[id, 0, level, 100]);
        frame
    }

    fn feature_proof(interface: &HidInterface, buttons: &[u8], values: Option<&[u8]>)
        -> FeatureReportProof {
        let caps = crate::win32::HidpCaps {
            usage: interface.usage,
            usage_page: interface.usage_page,
            input_report_byte_length: interface.input_len,
            output_report_byte_length: interface.output_len,
            feature_report_byte_length: interface.feature_len,
            number_feature_button_caps: buttons.len() as u16,
            number_feature_value_caps: 1,
            ..Default::default()
        };
        descriptor::feature_report_proof_with(&caps, interface, REPORT_BYTES, &[0x83, 0],
            |is_button, _| if is_button { Some(buttons.to_vec()) } else { values.map(<[u8]>::to_vec) })
    }

    #[test]
    fn complete_mismatch_sibling_does_not_hide_the_only_matching_collection() {
        let first = interface();
        let mut sibling = first.clone();
        sibling.path = sibling.path.replace("col01", "col02");
        for report_id in [0, 0x83] {
            assert_eq!(feature_proof(&first, &[report_id, report_id], Some(&[report_id])),
                FeatureReportProof::Match(report_id));
            assert_eq!(feature_proof(&sibling, &[0x84], Some(&[0x84])),
                FeatureReportProof::Mismatch);
            for interfaces in [vec![first.clone(), sibling.clone()], vec![sibling.clone(), first.clone()]] {
                let selected = select_with(&interfaces,
                    |_| Some(r"usb\vid_3151&pid_5026\receiver".into()),
                    |interface| {
                        let id = if interface.path == first.path { report_id } else { 0x84 };
                        feature_proof(interface, &[id, id], Some(&[id]))
                    });
                assert_eq!(selected.len(), 1);
                assert_eq!(selected[0].0.path, first.path);
                assert_eq!(selected[0].1, report_id);
            }
        }
    }

    #[test]
    fn unknown_sibling_blocks_only_its_physical_receiver_in_either_order() {
        let first = interface();
        let mut sibling = first.clone();
        sibling.path = sibling.path.replace("col01", "col02");
        let mut independent = first.clone();
        independent.path = independent.path.replace("&fixture&", "&independent&");
        for incomplete in [None, Some(vec![]), Some(vec![0x84, 0x84])] {
            assert_eq!(feature_proof(&sibling, &[0x84], incomplete.as_deref()), FeatureReportProof::Unknown);
            for interfaces in [vec![first.clone(), sibling.clone(), independent.clone()],
                vec![independent.clone(), sibling.clone(), first.clone()]] {
                let selected = select_with(&interfaces, |interface| {
                    let instance = if interface.path == independent.path { "independent" } else { "receiver" };
                    Some(format!(r"usb\vid_3151&pid_5026\{instance}"))
                }, |interface| {
                    if interface.path == sibling.path { feature_proof(interface, &[0x84], incomplete.as_deref()) }
                    else { feature_proof(interface, &[0x83], Some(&[0x83])) }
                });
                assert_eq!(selected.len(), 1);
                assert_eq!(selected[0].0.path, independent.path);
            }
        }
    }

    #[test]
    fn two_matching_collections_are_rejected_even_with_different_report_ids() {
        let first = interface();
        let mut sibling = first.clone();
        sibling.path = sibling.path.replace("col01", "col02");
        for sibling_id in [0, 0x83] {
            for interfaces in [vec![first.clone(), sibling.clone()], vec![sibling.clone(), first.clone()]] {
                assert!(select_with(&interfaces, |_| Some(r"usb\vid_3151&pid_5026\receiver".into()),
                    |interface| {
                        let id = if interface.path == first.path { 0 } else { sibling_id };
                        feature_proof(interface, &[id], Some(&[id]))
                    }).is_empty());
            }
        }
    }

    #[test]
    fn complete_mismatches_never_create_a_target() {
        let first = interface();
        let mut sibling = first.clone();
        sibling.path = sibling.path.replace("col01", "col02");
        for interfaces in [vec![first.clone()], vec![first, sibling]] {
            assert!(select_with(&interfaces, |_| Some(r"usb\vid_3151&pid_5026\receiver".into()),
                |interface| feature_proof(interface, &[0x84], Some(&[0x84]))).is_empty());
        }
    }

    #[test]
    fn independent_receivers_are_selected_in_either_enumeration_order() {
        let first = interface();
        let mut second = first.clone();
        second.path = second.path.replace("&fixture&", "&second&");
        for interfaces in [vec![first.clone(), second.clone()], vec![second.clone(), first.clone()]] {
            let selected = select_with(&interfaces, |interface| {
                let instance = if interface.path == first.path { "first" } else { "second" };
                Some(format!(r"usb\vid_3151&pid_5026\{instance}"))
            }, |interface| feature_proof(interface, &[0x83], Some(&[0x83])));
            assert_eq!(selected.len(), 2);
            assert_ne!(target(selected[0].0).key, target(selected[1].0).key);
        }
    }

    #[test]
    fn ambiguous_physical_receiver_does_not_suppress_an_independent_receiver() {
        let first = interface();
        let mut sibling = first.clone();
        sibling.path = sibling.path.replace("col01", "col02").replace("&fixture&", "&alias&");
        let mut independent = first.clone();
        independent.path = independent.path.replace("&fixture&", "&independent&");
        let interfaces = [first.clone(), sibling, independent.clone()];
        let selected = select_with(&interfaces, |interface| {
            Some(if interface.path == independent.path {
                r"usb\vid_3151&pid_5026\independent"
            } else if interface.path == first.path {
                r"USB\VID_3151&PID_5026\AMBIGUOUS"
            } else {
                r"usb\vid_3151&pid_5026\ambiguous"
            }.into())
        }, |interface| feature_proof(interface, &[0], Some(&[0])));
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].0.path, independent.path);
        assert_eq!(selected[0].1, 0);
    }

    #[test]
    fn failed_report_proof_is_isolated_after_physical_grouping() {
        let first = interface();
        let mut second = first.clone();
        second.path = second.path.replace("&fixture&", "&second&");
        let interfaces = [first.clone(), second.clone()];
        let selected = select_with(&interfaces, |interface| Some(format!(
            r"usb\vid_3151&pid_5026\{}", if interface.path == first.path { "first" } else { "second" }
        )), |interface| feature_proof(interface, &[0x83],
            if interface.path == second.path { Some(&[0x83]) } else { None }));
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].0.path, second.path);
    }

    #[test]
    fn unresolved_or_nonphysical_parent_never_assumes_collection_independence() {
        let first = interface();
        let mut second = first.clone();
        second.path = second.path.replace("&fixture&", "&second&");
        for invalid in [None, Some(r"usb\vid_3151&pid_5026&mi_02\second"),
            Some(r"usb\vid_3151&pid_5029\second"), Some(r"usb\vid_3151&pid_5026\")] {
            assert!(select_with(&[first.clone(), second.clone()], |interface| {
                if interface.path == first.path { Some(r"usb\vid_3151&pid_5026\first".into()) }
                else { invalid.map(str::to_string) }
            }, |_| panic!("unresolved topology must not reach report proof")).is_empty());
        }
    }

    #[test]
    fn device_instance_requires_the_exact_reviewed_usb_interface() {
        let original = interface();
        assert!(device_instance(&original).is_some());
        for path in [original.path.replace("mi_02", "mi_020"),
            original.path.replace("pid_5026", "pid_5029"),
            original.path.replace("col01", "colGG"),
            original.path.replace("mi_02", "mi_02&mi_03"),
            original.path.replace("&fixture&", "&fixture\\other&")] {
            let mut wrong = original.clone();
            wrong.path = path;
            assert!(device_instance(&wrong).is_none());
        }
    }

    #[test]
    fn only_the_exact_shine60_feature_collection_is_accepted() {
        let exact = interface();
        assert!(management(&exact));
        for mutate in [
            |i: &mut HidInterface| i.product_id = 0x5029,
            |i: &mut HidInterface| i.product = "Other keyboard".into(),
            |i: &mut HidInterface| i.usage = 1,
            |i: &mut HidInterface| i.path = i.path.replace("mi_02", "mi_01"),
            |i: &mut HidInterface| i.input_len = 65,
            |i: &mut HidInterface| i.feature_len = 64,
        ] {
            let mut wrong = exact.clone();
            mutate(&mut wrong);
            assert!(!management(&wrong));
        }
    }

    #[test]
    fn status_page_requires_id_zero_zero_scale_and_valid_percentage() {
        for id in [0, 0x83] {
            for level in [0, 1, 48, 61, 100] {
                assert_eq!(decode(&frame(id, level), id), Some(level));
            }
            assert!(decode(&frame(id ^ 1, 48), id).is_none());
            let mut wrong = frame(id, 48);
            wrong[1] = 1;
            assert!(decode(&wrong, id).is_none());
            wrong = frame(id, 48);
            wrong[2] = 101;
            assert!(decode(&wrong, id).is_none());
            wrong = frame(id, 48);
            wrong[3] = 99;
            assert!(decode(&wrong, id).is_none());
            assert!(decode(&frame(id, 48)[..64], id).is_none());
        }
    }

    #[test]
    fn every_truncated_or_oversized_status_page_is_rejected_without_panicking() {
        for report_id in [0, 0x83] {
            for length in 0..=usize::from(REPORT_BYTES) + 1 {
                let mut reply = frame(report_id, 48);
                reply.resize(length, 0);
                assert_eq!(decode(&reply, report_id),
                    (length == usize::from(REPORT_BYTES)).then_some(48));
            }
        }
    }

    #[test]
    fn snapshot_never_infers_charging_or_write_capabilities() {
        let mut target = Target::new(BackendId::VeekosShine60, &interface(), 0);
        target.name = "Veekos Shine 60".into();
        target.family = DeviceFamily::Keyboard;
        let sample = snapshot(&target, 48);
        assert_eq!(sample.battery.as_ref().unwrap().charging, None);
        assert!(sample.dpi.current.is_none() && !sample.dpi.writable);
        assert!(!sample.polling_rate.writable);
    }
}
