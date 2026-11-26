//! Fixed MX 2.0S WL Pro and PIXIU75 battery queries, without configuration writes.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct Cherry;

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x046a && matches!(i.product_id, 0x01e0 | 0x01de | 0x01e4 | 0x01e2)
        && i.usage_page == 0xff1c && i.usage == 0x0092
}

fn pixiu(i: &HidInterface) -> bool { matches!(i.product_id, 0x01e4 | 0x01e2) }

fn pixiu_group(i: &HidInterface) -> Option<String> {
    if !management(i) || !pixiu(i) { return None; }
    let path = i.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid" || parts[2].is_empty()
        || parts[3].is_empty() || path.contains('\0') || parts[2].contains('\\') { return None; }
    let fields: Vec<_> = parts[1].split('&').collect();
    let pid = format!("{:04x}", i.product_id);
    for (prefix, expected) in [("vid_", "046a"), ("pid_", pid.as_str()), ("mi_", "01")] {
        let values: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix(prefix)).collect();
        if values.as_slice() != [expected] { return None; }
    }
    let collections: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix("col")).collect();
    let [collection] = collections.as_slice() else { return None; };
    if collection.len() != 2 || !collection.bytes().all(|byte| byte.is_ascii_hexdigit()) { return None; }
    let (instance, suffix) = parts[2].rsplit_once('&')?;
    if instance.split('&').count() < 2 || suffix.len() != 4 || !suffix.starts_with("00")
        || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) { return None; }
    Some(format!("{}#vid_046a&pid_{pid}&mi_01#{instance}#{}", parts[0], parts[3]))
}

fn control(i: &HidInterface) -> bool {
    management(i) && i.input_len == 64 && i.output_len == 64
        && match i.product_id {
            0x01e0 => i.product == "MX 2.0S Pro Dongle",
            0x01de => i.product == "MX 2.0S WL Pro",
            0x01e4 => i.product == "PIXIU75 Dongle",
            0x01e2 => i.product == "PIXIU 75", _ => false,
        }
        && i.path.to_ascii_lowercase().split(['#', '&']).any(|p| p == "mi_01")
        && (!pixiu(i) || (i.feature_len == 0 && pixiu_group(i).is_some()
            && i.path.to_ascii_lowercase().split(['#', '&']).any(|part| part == "col04")))
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|i| control(i) && interfaces.iter().filter(|other|
        management(other) && if pixiu(i) { pixiu_group(i) == pixiu_group(other) }
        else { channel::collection_group(i) == channel::collection_group(other) }).count() == 1).collect()
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    control(interface) && descriptor::Descriptor::open(api, interface)
        .is_some_and(|caps| if pixiu(interface) {
            caps.single_input_output_byte_report_ids(interface, 64, 64) == Some((4, 4))
        } else { caps.only_input_output_report(interface, 4, 64) })
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

fn pixiu_request() -> [u8; 64] {
    let mut frame = [0; 64];
    frame[..8].copy_from_slice(&[4, 0, 0, 0x1a, 4, 0, 0, 0]);
    frame
}

fn pixiu_reply(reply: &[u8]) -> bool {
    reply.len() == 64 && reply[..8] == [4, 0, 0, 0x1a, 4, 0, 0, 0]
}

fn pixiu_battery(reply: &[u8]) -> Option<Battery> {
    if !pixiu_reply(reply) || !(1..=100).contains(&reply[8]) { return None; }
    // Zero can echo the query. Reserved charging bits have no captured meaning.
    Some(Battery { percent: Some(reply[8]), estimated_percent: None,
        charging: match reply[9] { 0 => Some(false), 1 => Some(true), _ => None },
        voltage_mv: None, approximate: false, level: None })
}

fn pixiu_read_with(target: &Target,
    exchange: impl FnOnce(&[u8], &mut dyn FnMut(&[u8]) -> bool) -> Option<Vec<u8>>)
    -> Result<Snapshot, String> {
    let interface = target.hid()?;
    if target.backend != BackendId::Cherry || !pixiu(interface) || !control(interface) {
        return Err(crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试").into());
    }
    let started = Instant::now();
    let mut remaining = 32;
    let response = exchange(&pixiu_request(), &mut |frame| {
        remaining -= 1;
        // Stop on an unusable matching reply; a later old sample cannot replace it.
        pixiu_reply(frame) || remaining == 0
    });
    let battery = response.as_deref().and_then(pixiu_battery);
    trace::record(interface, BackendId::Cherry, "query-battery", 0, 0x1a, 64,
        response.as_ref().map_or(0, Vec::len), started, match (&response, &battery) {
            (_, Some(_)) => trace::Outcome::Ok,
            (Some(_), None) => trace::Outcome::InvalidData,
            (None, None) => trace::Outcome::NoReply,
        });
    let battery = battery.ok_or_else(|| crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试"))?;
    Ok(Snapshot { target: target.clone(), battery: Some(battery), battery_at: Some(Instant::now()),
        battery_unavailable: false, connected: true, dpi: Dpi::default(), polling_rate: Default::default() })
}

fn read_pixiu(api: &HidApi, target: &Target) -> Result<Snapshot, String> {
    let interfaces = enumeration::scan(api)?.interfaces;
    let interface = candidates(&interfaces).into_iter().find(|interface|
        target.hid().is_ok_and(|original| interface.path.eq_ignore_ascii_case(&original.path)))
        .filter(|interface| proven(api, interface))
        .ok_or_else(|| crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试"))?;
    let lock = channel::lock(&pixiu_group(interface)
        .ok_or_else(|| crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试"))?);
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut device = HidDevice::open(interface)?;
    // Existing interrupt OUT transport drains queued notifications before querying.
    pixiu_read_with(target, |request, matches| device.transact(request, 1700, matches))
}

impl DeviceBackend for Cherry {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i)).map(|i| {
            let mut target = Target::new(BackendId::Cherry, i, 0);
            target.name = if pixiu(i) { "CHERRY PIXIU75" } else { "CHERRY MX 2.0S WL Pro" }.into();
            target.family = DeviceFamily::Keyboard;
            target
        }).collect())
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let interface = target.hid()?;
        if pixiu(interface) { return read_pixiu(api, target); }
        if !proven(api, interface) { return Err(crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试").into()); }
        let lock = channel::lock(&channel::collection_group(interface));
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(interface)?;
        let started = Instant::now();
        let response = device.transact(&request(), 800, |r| battery(r).is_some());
        trace::record(interface, BackendId::Cherry, "query-battery", 0x20, 0x1a, 64,
            response.as_ref().map_or(0, Vec::len), started,
            if response.is_some() { trace::Outcome::Ok } else { trace::Outcome::NoReply });
        let battery = response.as_deref().and_then(battery)
            .ok_or_else(|| crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试"))?;
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

    fn pixiu_interface(pid: u16, instance: &str) -> HidInterface {
        let mut result = interface(pid);
        result.path = result.path.replace("unit-a", instance).replace("&0004", "&0003");
        result.product = match pid { 0x01e4 => "PIXIU75 Dongle", _ => "PIXIU 75" }.into();
        result
    }

    fn pixiu_frame(percent: u8, charging: u8) -> Vec<u8> {
        let mut frame = pixiu_request().to_vec();
        frame[8] = percent; frame[9] = charging; frame
    }

    fn pixiu_sample(target: &Target, frames: Vec<Vec<u8>>) -> Result<Snapshot, String> {
        pixiu_read_with(target, |packet, matches| {
            assert_eq!(packet, pixiu_request());
            frames.into_iter().find(|frame| matches(frame))
        })
    }

    #[test]
    fn pixiu_published_payloads_and_synthetic_full_charge_preserve_reported_state() {
        // VigilWatt c97adc3 / #35: public prefixes padded for tests; 100% is synthetic.
        for pid in [0x01e4, 0x01e2] {
            let target = Target::new(BackendId::Cherry, &pixiu_interface(pid, "one"), 0);
            for (percent, charging) in [(48, 0), (49, 1), (50, 1), (51, 1), (55, 1), (100, 1)] {
                let sample = pixiu_sample(&target, vec![pixiu_frame(percent, charging)]).unwrap();
                let battery = sample.battery.unwrap();
                assert_eq!((battery.percent, battery.charging), (Some(percent), Some(charging == 1)));
                assert_eq!(sample.target.key, target.key);
                assert!(sample.connected && sample.battery_at.is_some() && !sample.battery_unavailable);
                assert!(!sample.dpi.writable && !sample.polling_rate.writable && !battery.approximate);
                assert!(battery.estimated_percent.is_none() && battery.voltage_mv.is_none());
            }
        }
        assert_eq!(&pixiu_request()[..8], &[4, 0, 0, 0x1a, 4, 0, 0, 0]);
        assert!(pixiu_request()[8..].iter().all(|byte| *byte == 0));
        assert_eq!(&request()[..8], &[4, 0x20, 0, 0x1a, 6, 0, 0, 0]);
    }

    #[test]
    fn pixiu_requires_complete_header_and_does_not_mix_mx_pro_replies() {
        let valid = pixiu_frame(48, 0);
        for length in 0..64 { assert!(pixiu_battery(&valid[..length]).is_none()); }
        let mut long = valid.clone(); long.push(0); assert!(pixiu_battery(&long).is_none());
        for (offset, value) in [(0, 0), (1, 0x20), (2, 1), (3, 0x1d), (4, 0), (4, 6),
            (5, 1), (6, 1), (7, 1), (8, 0), (8, 101), (8, 255)] {
            let mut wrong = valid.clone(); wrong[offset] = value;
            assert!(pixiu_battery(&wrong).is_none());
        }
        assert!(battery(&valid).is_none());
        let mut pro = request(); pro[8] = 48;
        assert!(battery(&pro).is_some() && pixiu_battery(&pro).is_none());
        for percent in 1..=100 { assert_eq!(pixiu_battery(&pixiu_frame(percent, 0)).unwrap().percent, Some(percent)); }
        for flag in [0, 1, 2, 0xff] { assert!(pixiu_battery(&pixiu_frame(0, flag)).is_none()); }
        for flag in [2, 3, 0x80, 0xff] {
            let battery = pixiu_battery(&pixiu_frame(48, flag)).unwrap();
            assert_eq!((battery.percent, battery.charging), (Some(48), None));
        }
    }

    #[test]
    fn pixiu_ignores_other_blocks_but_matching_zero_ends_query_without_refreshing_history() {
        let target = Target::new(BackendId::Cherry, &pixiu_interface(0x01e4, "one"), 0);
        let mut unrelated = pixiu_frame(88, 0); unrelated[3] = 0x1d;
        let mut pro = request().to_vec(); pro[8] = 90;
        let good = pixiu_sample(&target, vec![unrelated, pro, pixiu_frame(48, 0)]).unwrap();
        assert_eq!(good.battery.as_ref().unwrap().percent, Some(48));
        let mut consumed = 0;
        let missing = pixiu_read_with(&target, |_, matches| {
            [pixiu_request().to_vec(), pixiu_frame(99, 0)].into_iter().find(|frame| {
                consumed += 1; matches(frame)
            })
        });
        assert!(missing.is_err());
        assert_eq!(consumed, 1);
        assert!(pixiu_read_with(&target, |_, _| None).is_err());
        let measured = good.battery_at.unwrap();
        let lifetime = std::time::Duration::from_secs(120);
        let mut history = status::Status::default();
        history.observe_sample(Some(true), good.battery.as_ref(), Some(measured), measured, lifetime);
        history.observe_sample(None, None, None, measured + std::time::Duration::from_secs(10), lifetime);
        assert_eq!(history.measured_at, Some(measured));
        assert_eq!(history.battery.as_ref().unwrap().percent, Some(48));
        assert!(history.connected && !history.connection_known);
        assert!(history.expire(measured + lifetime, lifetime));
    }

    #[test]
    fn pixiu_unrelated_input_flood_has_a_fixed_frame_budget() {
        let target = Target::new(BackendId::Cherry, &pixiu_interface(0x01e4, "one"), 0);
        let mut reads = 0;
        let mut wrong = pixiu_frame(48, 0); wrong[3] = 0x1d;
        let result = pixiu_read_with(&target, |_, matches| {
            (0..100).find_map(|_| {
                reads += 1;
                matches(&wrong).then(|| wrong.clone())
            })
        });
        assert!(result.is_err());
        assert_eq!(reads, 32);
    }

    #[test]
    fn pixiu_requires_exact_product_control_collection_and_windows_id_slot() {
        for pid in [0x01e4, 0x01e2] {
            let good = pixiu_interface(pid, "one");
            assert!(control(&good));
            for (vid, product, page, usage, input, output, feature) in [
                (0x046a, 0x01ac, 0xff1c, 0x92, 64, 64, 0),
                (0x0687, pid, 0xff1c, 0x92, 64, 64, 0),
                (0x046a, pid, 1, 6, 64, 64, 0),
                (0x046a, pid, 0xffef, 0x92, 64, 64, 0),
                (0x046a, pid, 0xff1c, 0x93, 64, 64, 0),
                (0x046a, pid, 0xff1c, 0x92, 63, 64, 0),
                (0x046a, pid, 0xff1c, 0x92, 64, 65, 0),
                (0x046a, pid, 0xff1c, 0x92, 64, 64, 64),
            ] {
                let mut wrong = good.clone(); wrong.vendor_id = vid; wrong.product_id = product;
                wrong.usage_page = page; wrong.usage = usage;
                wrong.input_len = input; wrong.output_len = output; wrong.feature_len = feature;
                assert!(!control(&wrong));
            }
            for product in ["PIXIU75", "PIXIU75 Dongle ", "MX 2.0S Pro Dongle", ""] {
                let mut wrong = good.clone(); wrong.product = product.into(); assert!(!control(&wrong));
            }
            for path in ["invalid".into(), good.path.replace("mi_01", "mi_02"),
                good.path.replace("mi_01", "mi_01&mi_02"), good.path.replace("col04", "col05"),
                good.path.replace("col04", "col04&col05"), good.path.replace("vid_046a", "vid_046a&vid_0687"),
                good.path.replace("&0003#", "&zzzz#"), format!("{}\0", good.path)] {
                let mut wrong = good.clone(); wrong.path = path;
                assert!(!control(&wrong));
                let target = Target::new(BackendId::Cherry, &wrong, 0);
                assert!(pixiu_read_with(&target, |_, _| panic!("invalid interface must not query")).is_err());
            }
        }
    }

    #[test]
    fn pixiu_double_receivers_keep_distinct_keys_and_one_failure_does_not_hide_the_other() {
        for serial in ["", "shared"] {
            let mut first = pixiu_interface(0x01e4, "one"); first.serial = serial.into();
            let mut second = pixiu_interface(0x01e4, "two"); second.serial = serial.into();
            let first_target = Target::new(BackendId::Cherry, &first, 0);
            let second_target = Target::new(BackendId::Cherry, &second, 0);
            assert_ne!(first_target.key, second_target.key);
            assert!(!first_target.same_channel(&second_target));
            let first_lock = channel::lock(&pixiu_group(&first).unwrap());
            let second_lock = channel::lock(&pixiu_group(&second).unwrap());
            assert!(!std::sync::Arc::ptr_eq(&first_lock, &second_lock));
            assert_eq!(candidates(&[first.clone(), second.clone()]).len(), 2);
            let results = super::super::read_targets(vec![first_target.clone(), second_target.clone()], |target| {
                pixiu_sample(target, if target.key == first_target.key { Vec::new() }
                    else { vec![pixiu_frame(51, 1)] })
            });
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].target.key, second_target.key);
            assert_eq!(results[0].battery.as_ref().unwrap().percent, Some(51));
            let mut sibling = first.clone();
            sibling.path = sibling.path.replace("col04", "col05").replace("&0003", "&0004");
            sibling.product = "Unknown".into(); sibling.output_len = 65;
            assert_eq!(pixiu_group(&first), pixiu_group(&sibling));
            let interfaces = [first, sibling, second];
            let selected = candidates(&interfaces);
            assert_eq!(selected.len(), 1);
            assert_eq!(selected[0].path, interfaces[2].path);
        }
    }
}
