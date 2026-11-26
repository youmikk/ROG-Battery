//! Read-only Weisheng keyboard receiver protocol used by VGN / VXE.
//! The shared USB ID does not identify a brand or a particular keyboard model.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct VgnKeyboard;
const QUERY_TIMEOUT_MS: u32 = 2_500;

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x320f && i.product_id == 0x5088
        && i.usage_page == 0xff1c && i.usage == 0x0092
}

fn control(i: &HidInterface) -> bool {
    management(i) && i.input_len == 64 && i.output_len == 64
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|i| control(i) && interfaces.iter().filter(|other|
        management(other) && channel::collection_group(i) == channel::collection_group(other)).count() == 1).collect()
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    control(interface) && descriptor::Descriptor::open(api, interface)
        .is_some_and(|caps| caps.only_input_output_report(interface, 4, 64))
}

fn target(i: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::VgnKeyboard, i, 0);
    target.family = DeviceFamily::Keyboard;
    if target.name.trim().is_empty() {
        target.name = format!("2.4G {}", crate::tr!("键盘"));
    }
    target
}

fn request() -> [u8; 64] {
    let mut frame = [0; 64];
    frame[..5].copy_from_slice(&[4, 0, 0, 0x1a, 6]);
    frame
}

fn asleep(reply: &[u8]) -> bool {
    reply.len() == 64 && reply[..5] == [4, 0, 0, 0xff, 6]
}

fn battery(reply: &[u8]) -> Option<Battery> {
    if reply.len() != 64 || reply[..5] != [4, 0, 0, 0x1a, 6]
        || reply[8] > 100 || reply == request() { return None; }
    // No established charging field or response checksum. An all-zero payload
    // is a query echo, not evidence of an empty battery.
    Some(Battery { percent: Some(reply[8]), estimated_percent: None, charging: None,
        voltage_mv: None, approximate: false, level: None })
}

fn read_with(target: &Target, mut exchange: impl FnMut(&[u8], u32) -> Option<Vec<u8>>)
    -> Result<Snapshot, String> {
    let fail = || crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试").to_string();
    if !control(target.hid()?) { return Err(fail()); }
    let reply = exchange(&request(), QUERY_TIMEOUT_MS).ok_or_else(fail)?;
    let battery = battery(&reply).ok_or_else(fail)?;
    Ok(Snapshot { target: target.clone(), battery: Some(battery), battery_at: Some(Instant::now()),
        battery_unavailable: false, connected: true, dpi: Dpi::default(), polling_rate: Default::default() })
}

impl DeviceBackend for VgnKeyboard {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i)).map(target).collect())
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let interface = target.hid()?;
        if !proven(api, interface) { return Err(crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试").into()); }
        let lock = channel::lock(&channel::collection_group(interface));
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(interface)?;
        read_with(target, |request, timeout| {
            let started = Instant::now();
            let response = device.transact(request, timeout, |r| battery(r).is_some() || asleep(r));
            let sleep = response.as_deref().is_some_and(asleep);
            trace::record(interface, BackendId::VgnKeyboard,
                if sleep { "query-battery-asleep" } else { "query-battery" }, 4, 0x1a, request.len(),
                response.as_ref().map_or(0, Vec::len), started,
                if response.is_some() && !sleep { trace::Outcome::Ok } else { trace::Outcome::NoReply });
            response
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface() -> HidInterface {
        HidInterface { path: r"\\?\hid#vid_320f&pid_5088&mi_01&col04#8&unit-a&0004#{hid}".into(),
            vendor_id: 0x320f, product_id: 0x5088, usage_page: 0xff1c, usage: 0x92,
            input_len: 64, output_len: 64, feature_len: 0, serial: "shared".into(),
            product: "Wireless Gaming Keyboard".into() }
    }

    #[test]
    fn captured_battery_prefix_keeps_device_percent_and_unknown_charging() {
        // Published nine-byte prefix, with synthetic trailing padding.
        let mut reply = request(); reply[8] = 0x5d;
        let b = battery(&reply).unwrap();
        assert_eq!((b.percent, b.charging, b.voltage_mv), (Some(93), None, None));
        assert_eq!(b.display_percent().unwrap().label(true), "93%");
        assert_eq!(&request()[..5], &[4, 0, 0, 0x1a, 6]);
        assert!(request()[5..].iter().all(|b| *b == 0));
        for percent in 1..=100 {
            reply[8] = percent;
            assert_eq!(battery(&reply).unwrap().percent, Some(percent));
        }
    }

    #[test]
    fn sleep_echo_other_protocols_and_invalid_lengths_cannot_renew_a_sample() {
        assert!(battery(&request()).is_none());
        let mut sleeping = request(); sleeping[3] = 0xff;
        assert!(asleep(&sleeping));
        assert!(battery(&sleeping).is_none());
        let mut valid = request(); valid[8] = 93;
        for len in 0..64 { assert!(battery(&valid[..len]).is_none()); assert!(!asleep(&sleeping[..len])); }
        let mut long = valid.to_vec(); long.push(0); assert!(battery(&long).is_none());
        for (offset, value) in [(0, 1), (1, 0x20), (2, 1), (3, 0xff), (4, 5), (8, 101), (8, 255)] {
            let mut wrong = valid; wrong[offset] = value;
            assert!(battery(&wrong).is_none());
        }
    }

    #[test]
    fn a_slow_receiver_gets_one_read_only_query_and_sleep_produces_no_snapshot() {
        let t = target(&interface());
        let mut calls = 0;
        let mut valid = request(); valid[8] = 93;
        let reading = read_with(&t, |frame, timeout| {
            calls += 1;
            assert_eq!(frame, request());
            assert_eq!(timeout, 2500); // The published response took about 1.03 seconds.
            Some(valid.to_vec())
        }).unwrap();
        assert_eq!(calls, 1);
        assert!(reading.connected && !reading.battery_unavailable);
        assert!(reading.battery_at.is_some());
        assert!(reading.dpi.stages.is_empty());
        let mut sleeping = request(); sleeping[3] = 0xff;
        for response in [None, Some(request().to_vec()), Some(sleeping.to_vec())] {
            assert!(read_with(&t, |_, _| response.clone()).is_err());
        }
    }

    #[test]
    fn shared_identity_does_not_invent_a_model_or_merge_identical_receivers() {
        let i = interface();
        assert_eq!(target(&i).name, "Wireless Gaming Keyboard");
        assert!(!target(&i).verified_identity);
        let mut named = i.clone(); named.product = "Rainy75".into();
        assert_eq!(target(&named).name, "Rainy75");
        let mut other = i.clone(); other.path = other.path.replace("unit-a", "unit-b");
        assert_eq!(candidates(&[i.clone(), other.clone()]).len(), 2);
        assert_ne!(target(&i).key, target(&other).key);
        let mut sibling = i.clone();
        sibling.path = sibling.path.replace("col04", "col05").replace("&0004", "&0005");
        sibling.input_len = 65;
        assert!(candidates(&[i.clone(), sibling]).is_empty());
        for (vid, pid, page, usage, input, output) in [
            (0x320f, 0x5055, 0xff1c, 0x92, 64, 64),
            (0x046a, 0x5088, 0xff1c, 0x92, 64, 64),
            (0x320f, 0x5088, 1, 6, 64, 64),
            (0x320f, 0x5088, 0xff1c, 0x92, 32, 64),
            (0x320f, 0x5088, 0xff1c, 0x92, 64, 65),
        ] {
            let mut wrong = i.clone();
            wrong.vendor_id = vid; wrong.product_id = pid; wrong.usage_page = page; wrong.usage = usage;
            wrong.input_len = input; wrong.output_len = output;
            assert!(!control(&wrong));
            assert!(read_with(&target(&wrong), |_, _| panic!("must not send")).is_err());
        }
    }
}
