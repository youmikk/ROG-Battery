//! VGN ryMs receiver status. F7 is the only transmitted command.
use super::*;
use std::time::{Duration, Instant};

pub struct VgnRy;
const LENGTH: u16 = 65;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn model(pid: u16) -> Option<&'static str> {
    Some(match pid {
        0x1a04 => "VGN Dragonfly3 Pro Max",
        0x1a08 => "VGN Dragonfly Y2 V2 Pro Max",
        0x1a13 => "VGN Dragonfly F1 V2 Pro Max",
        0x1a25 => "VGN Dragonfly3 Ultra",
        0x1a2b => "VGN Dragonfly F1 V5 Ultra",
        0x1a41 => "VGN Dragonfly F2 V2 Ultra",
        _ => return None,
    })
}

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x391d && model(i.product_id).is_some()
        && i.usage_page == 0xffff && i.usage == 2
}

fn control(i: &HidInterface) -> bool {
    if !management(i) || i.feature_len != LENGTH { return false; }
    // All six receiver entries in VGN HUB bind this exact USB interface/collection.
    let path = i.path.to_ascii_lowercase();
    let Some(hardware) = path.split('#').nth(1) else { return false; };
    let fields: Vec<_> = hardware.split('&').collect();
    fields.contains(&"mi_01") && fields.contains(&"col01")
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|i| control(i) && interfaces.iter().filter(|other|
        management(other) && channel::collection_group(i) == channel::collection_group(other)
    ).count() == 1).collect()
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    control(i) && descriptor::Descriptor::open(api, i).is_some_and(|d|
        d.feature_reports(i, LENGTH, &[0], &[0]) && d.feature_byte_report(i, 0, LENGTH))
}

fn target(i: &HidInterface) -> Target {
    let mut t = Target::new(BackendId::VgnRy, i, 0);
    t.name = model(i.product_id).unwrap_or("VGN Mouse").into();
    t.family = DeviceFamily::Mouse;
    t
}

fn request() -> [u8; LENGTH as usize] {
    let mut r = [0; LENGTH as usize];
    r[1] = 0xf7;
    r[8] = 0x08; // 0xff minus the sum of the seven command bytes.
    r
}

#[derive(Debug, PartialEq, Eq)]
enum Reply { Battery(u8), Sleeping }

fn reply(r: &[u8]) -> Option<Reply> {
    if r.len() != usize::from(LENGTH) || r[..2] != [0, 0]
        || r[1..].iter().all(|b| *b == 0) || r[3] > 100 { return None; }
    match r[5] { 0 => Some(Reply::Battery(r[3])), 1 => Some(Reply::Sleeping), _ => None }
}

fn read_with(t: &Target, exchange: impl FnOnce(&[u8]) -> Result<Vec<u8>, String>) -> Result<Snapshot, String> {
    let i = t.hid()?;
    if t.backend != BackendId::VgnRy || !control(i) { return Err(UNAVAILABLE.into()); }
    let r = exchange(&request())?;
    let state = reply(&r).ok_or(UNAVAILABLE)?;
    let mut target = target(i);
    target.verified_identity = true;
    let battery = match state {
        Reply::Battery(percent) => Some(Battery { percent: Some(percent), charging: None,
            estimated_percent: None, voltage_mv: None, approximate: false, level: None }),
        Reply::Sleeping => None,
    };
    Ok(Snapshot { target, battery_at: battery.as_ref().map(|_| Instant::now()), battery,
        battery_unavailable: state == Reply::Sleeping, connected: state != Reply::Sleeping,
        dpi: Dpi::default(), polling_rate: Default::default() })
}

impl DeviceBackend for VgnRy {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i)).map(target).collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        let i = t.hid()?;
        if t.backend != BackendId::VgnRy || !proven(api, i) { return Err(UNAVAILABLE.into()); }
        let lock = channel::lock(&channel::collection_group(i));
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        read_with(t, |request| {
            let started = Instant::now();
            let response = (|| {
                let mut channel = feature::FeatureChannel::open(i)?;
                channel.send(request, 500)?;
                std::thread::sleep(Duration::from_millis(60));
                channel.receive(0, 500)
            })();
            trace::record(i, BackendId::VgnRy, "query-battery", 0xf7, 0, request.len(),
                response.as_ref().map_or(0, Vec::len), started, match response.as_deref() {
                    Ok(r) if reply(r).is_some() => trace::Outcome::Ok,
                    Ok(_) => trace::Outcome::InvalidData,
                    Err(_) => trace::Outcome::NoReply,
                });
            response
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(pid: u16, instance: &str) -> HidInterface {
        HidInterface { path: format!(r"\\?\hid#vid_391d&pid_{pid:04x}&mi_01&col01#8&{instance}&0001#{{hid}}"),
            vendor_id: 0x391d, product_id: pid, usage_page: 0xffff, usage: 2,
            input_len: 0, output_len: 0, feature_len: LENGTH,
            product: "VGN USB Receiver".into(), serial: "shared".into() }
    }

    fn frame(percent: u8, sleep: u8) -> Vec<u8> {
        let mut r = vec![0; usize::from(LENGTH)];
        r[2] = 1; r[3] = percent; r[5] = sleep;
        r
    }

    #[test]
    fn receiver_catalog_excludes_wired_yjx_and_ambiguous_collections() {
        for pid in [0x1a04, 0x1a08, 0x1a13, 0x1a25, 0x1a2b, 0x1a41] {
            let i = interface(pid, "one");
            assert!(control(&i));
            assert_eq!(candidates(&[i.clone()]).len(), 1);
            for wrong_pid in [pid - 0xa00, 0x1a01, 0x1a03, 0x1aff] {
                let mut wrong = i.clone(); wrong.product_id = wrong_pid;
                assert!(!control(&wrong));
            }
            let mut sibling = i.clone();
            sibling.path = sibling.path.replace("col01", "col02").replace("&0001#", "&0002#");
            assert!(candidates(&[i.clone(), sibling]).is_empty());
            let another = interface(pid, "two");
            assert_eq!(candidates(&[i.clone(), another.clone()]).len(), 2);
            assert_ne!(target(&i).key, target(&another).key);
        }
        let mut i = interface(0x1a04, "one");
        i.path = i.path.replace("mi_01", "mi_02"); assert!(!control(&i));
        i = interface(0x1a04, "one"); i.feature_len = 64; assert!(!control(&i));
        i = interface(0x1a04, "one"); i.vendor_id = 0x3837; assert!(!control(&i));
        i = interface(0x1a04, "one"); i.usage_page = 0xff01; assert!(!control(&i));
    }

    #[test]
    fn status_rejects_empty_echo_short_and_invalid_frames_without_guessing_charge() {
        assert_eq!(&request()[..9], &[0, 0xf7, 0, 0, 0, 0, 0, 0, 8]);
        assert!(request()[9..].iter().all(|b| *b == 0));
        assert!(reply(&request()).is_none());
        assert!(reply(&[0; 65]).is_none());
        for percent in 0..=100 { assert_eq!(reply(&frame(percent, 0)), Some(Reply::Battery(percent))); }
        for percent in 101..=255 { assert!(reply(&frame(percent, 0)).is_none()); }
        for sleep in 2..=255 { assert!(reply(&frame(92, sleep)).is_none()); }
        for size in 0..65 { assert!(reply(&frame(92, 0)[..size]).is_none()); }
        let mut wrong = frame(92, 0); wrong.push(0); assert!(reply(&wrong).is_none());
        for position in [0, 1] { let mut wrong = frame(92, 0); wrong[position] = 0xff; assert!(reply(&wrong).is_none()); }
    }

    #[test]
    fn sleep_clears_battery_and_next_poll_restores_it_without_manual_refresh() {
        let t = target(&interface(0x1a04, "one"));
        let read = |r| read_with(&t, |request_bytes| { assert_eq!(request_bytes, request()); Ok(r) });
        let first = read(frame(92, 0)).unwrap();
        assert!(first.connected && first.battery_at.is_some());
        assert_eq!(first.battery.unwrap().charging, None);
        let sleep = read(frame(92, 1)).unwrap();
        assert!(!sleep.connected && sleep.battery_unavailable && sleep.battery.is_none() && sleep.battery_at.is_none());
        assert!(read_with(&t, |_| Err("timeout".into())).is_err());
        let awake = read(frame(91, 0)).unwrap();
        assert!(awake.connected && !awake.battery_unavailable && awake.battery_at.is_some());
        assert_eq!(awake.battery.unwrap().percent, Some(91));
        assert!(awake.dpi.stages.is_empty() && !awake.dpi.writable);
        let mut wrong = t; wrong.backend = BackendId::VgnDragonfly3;
        assert!(read_with(&wrong, |_| panic!("wrong protocol must not send")).is_err());
    }
}
