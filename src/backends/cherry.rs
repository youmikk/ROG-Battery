//! Fixed MX 2.0S WL Pro battery query, without keyboard configuration writes.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct Cherry;

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x046a && matches!(i.product_id, 0x01e0 | 0x01de)
        && i.usage_page == 0xff1c && i.usage == 0x0092
}

fn control(i: &HidInterface) -> bool {
    management(i) && i.input_len == 64 && i.output_len == 64
        && match i.product_id {
            0x01e0 => i.product == "MX 2.0S Pro Dongle",
            0x01de => i.product == "MX 2.0S WL Pro", _ => false,
        }
        && i.path.to_ascii_lowercase().split(['#', '&']).any(|p| p == "mi_01")
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|i| control(i) && interfaces.iter().filter(|other|
        management(other) && channel::collection_group(i) == channel::collection_group(other)).count() == 1).collect()
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    control(interface) && descriptor::Descriptor::open(api, interface)
        .is_some_and(|caps| caps.only_input_output_report(interface, 4, 64))
}

fn request() -> [u8; 64] {
    let mut frame = [0; 64];
    frame[..5].copy_from_slice(&[4, 0x20, 0, 0x1a, 6]);
    frame
}

fn battery(reply: &[u8]) -> Option<Battery> {
    if reply.len() != 64 || reply[..5] != [4, 0x20, 0, 0x1a, 6]
        || reply[7] != 0 || !(1..=100).contains(&reply[8]) || reply[9] > 1 { return None; }
    // A zero reply is indistinguishable from an echoed query. Its battery
    // meaning is unverified, so it cannot replace or renew a valid observation.
    Some(Battery { percent: Some(reply[8]), estimated_percent: None,
        charging: Some(reply[9] == 1), voltage_mv: None, approximate: false, level: None })
}

impl DeviceBackend for Cherry {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i)).map(|i| {
            let mut target = Target::new(BackendId::Cherry, i, 0);
            target.name = "CHERRY MX 2.0S WL Pro".into();
            target.family = DeviceFamily::Keyboard;
            target
        }).collect())
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let interface = target.hid()?;
        if !proven(api, interface) { return Err(crate::i18n::translate("设备无有效状态回报").into()); }
        let lock = channel::lock(&channel::collection_group(interface));
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(interface)?;
        let started = Instant::now();
        let response = device.transact(&request(), 800, |r| battery(r).is_some());
        trace::record(interface, BackendId::Cherry, "query-battery", 0x20, 0x1a, 64,
            response.as_ref().map_or(0, Vec::len), started,
            if response.is_some() { trace::Outcome::Ok } else { trace::Outcome::NoReply });
        let battery = response.as_deref().and_then(battery)
            .ok_or_else(|| crate::i18n::translate("设备无有效状态回报"))?;
        Ok(Snapshot { target: target.clone(), battery: Some(battery), battery_at: Some(Instant::now()),
            battery_unavailable: false, connected: true, dpi: Dpi::default(), polling_rate: Default::default() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(pid: u16) -> HidInterface {
        HidInterface { path: format!(r"\\?\hid#vid_046a&pid_{pid:04x}&mi_01&col04#8&unit-a&0004#{{hid}}"),
            vendor_id: 0x046a, product_id: pid, usage_page: 0xff1c, usage: 0x92,
            input_len: 64, output_len: 64, feature_len: 0, serial: "shared".into(),
            product: if pid == 0x01e0 { "MX 2.0S Pro Dongle" } else { "MX 2.0S WL Pro" }.into() }
    }

    #[test]
    fn community_reply_prefixes_preserve_reported_charge_and_do_not_adjust_to_vendor_ui() {
        // Published 16-byte prefixes from issue #482; the rest is test padding.
        for (percent, charging) in [(55, 0), (58, 1), (63, 1), (61, 0)] {
            let mut reply = [0; 64];
            reply[..10].copy_from_slice(&[4, 0x20, 0, 0x1a, 6, 0, 0, 0, percent, charging]);
            let b = battery(&reply).unwrap();
            assert_eq!((b.percent, b.charging), (Some(percent), Some(charging == 1)));
            assert!(!b.approximate);
            assert_eq!(b.display_percent().unwrap().label(true), format!("{percent}%"));
        }
        assert_eq!(&request()[..5], &[4, 0x20, 0, 0x1a, 6]);
        assert!(request()[5..].iter().all(|b| *b == 0));
    }

    #[test]
    fn echoes_truncated_frames_other_commands_and_unknown_flags_are_not_battery() {
        assert!(battery(&request()).is_none());
        let mut valid = request(); valid[8] = 50;
        assert!(battery(&valid).is_some());
        for length in 0..64 { assert!(battery(&valid[..length]).is_none()); }
        let mut long = valid.to_vec(); long.push(0); assert!(battery(&long).is_none());
        for (offset, value) in [(0, 1), (1, 0), (2, 1), (3, 0x19), (4, 5), (7, 1), (8, 0), (8, 101), (9, 2), (9, 0xff)] {
            let mut wrong = valid; wrong[offset] = value;
            assert!(battery(&wrong).is_none());
        }
        for percent in 1..=100 {
            let mut r = valid; r[8] = percent;
            assert_eq!(battery(&r).unwrap().percent, Some(percent));
        }
    }

    #[test]
    fn exact_models_interface_and_product_are_required_without_probing_other_cherry_devices() {
        for pid in [0x01e0, 0x01de] {
            let i = interface(pid);
            assert!(control(&i));
            let mut other = i.clone(); other.path = other.path.replace("mi_01", "mi_00");
            assert!(!control(&other));
            other = i.clone(); other.product.push_str(" Other");
            assert!(!control(&other));
            other = i.clone(); other.usage_page = 1; other.usage = 6;
            assert!(!control(&other));
            other = i.clone(); other.input_len = 65;
            assert!(!control(&other));
        }
        let mut ble = interface(0x01de); ble.vendor_id = 0x0687;
        assert!(!control(&ble));
        assert!(!control(&interface(0x01ac)));
    }

    #[test]
    fn ambiguous_siblings_are_rejected_before_product_filtering_and_instances_stay_separate() {
        let i = interface(0x01e0);
        let mut other = i.clone(); other.path = other.path.replace("unit-a", "unit-b");
        assert_eq!(candidates(&[i.clone(), other.clone()]).len(), 2);
        assert_ne!(Target::new(BackendId::Cherry, &i, 0).key, Target::new(BackendId::Cherry, &other, 0).key);
        let mut sibling = i.clone();
        sibling.path = sibling.path.replace("col04", "col05").replace("&0004", "&0005");
        sibling.product = "Unknown".into();
        assert_eq!(candidates(&[i.clone(), sibling.clone(), other]).len(), 1);
        assert!(candidates(&[i, sibling]).is_empty());
    }
}
