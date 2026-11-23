//! FIFINE HT6 status query on the receiver's documented FF90 collection.
//! SoundLink captures distinguish battery, charging, power-off and link loss.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct FifineHt6;

const REPORT: u8 = 0x55;
const LENGTH: u16 = 64;
const UNAVAILABLE: &str = "设备无有效状态回报";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x3142 && i.product_id == 0x00c3
        && i.usage_page == 0xff90 && i.usage == 1
}

fn receiver_group(i: &HidInterface) -> Option<String> {
    if !management(i) { return None; }
    let path = i.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid" || parts[2].is_empty()
        || parts[3].is_empty() || path.contains('\0')
        || parts[1].contains('\\') || parts[2].contains('\\') { return None; }
    let fields: Vec<_> = parts[1].split('&').collect();
    for required in ["vid_3142", "pid_00c3", "mi_00"] {
        if fields.iter().filter(|field| **field == required).count() != 1 { return None; }
    }
    for prefix in ["vid_", "pid_", "mi_"] {
        if fields.iter().filter(|field| field.starts_with(prefix)).count() != 1 { return None; }
    }
    let collections: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix("col")).collect();
    let instance = match collections.as_slice() {
        [] => parts[2],
        [collection] => {
            if collection.len() != 2 || !collection.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
            let (instance, index) = parts[2].rsplit_once('&')?;
            if instance.split('&').count() < 2 || index.len() != 4 || !index.starts_with("00")
                || !index.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
            instance
        }
        _ => return None,
    };
    let hardware = fields.into_iter().filter(|field| !field.starts_with("col"))
        .collect::<Vec<_>>().join("&");
    // The MI is documented; preserve it and the full Windows instance instead
    // of grouping receivers by their often shared product or serial strings.
    Some(format!("{}#{hardware}#{instance}#{}", parts[0], parts[3]))
}

fn control(i: &HidInterface) -> bool {
    management(i) && i.input_len == LENGTH && i.output_len == LENGTH
        && receiver_group(i).is_some()
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|i| control(i) && interfaces.iter().filter(|other|
        management(other) && receiver_group(i) == receiver_group(other)
    ).count() == 1).collect()
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    control(i) && descriptor::Descriptor::open(api, i)
        .is_some_and(|d| d.only_input_output_report(i, REPORT, LENGTH))
}

fn target(i: &HidInterface) -> Target {
    let mut t = Target::new(BackendId::FifineHt6, i, 0);
    t.name = "FIFINE HT6 Wireless".into();
    t.family = DeviceFamily::Headset;
    t
}

fn request() -> [u8; LENGTH as usize] {
    let mut r = [0; LENGTH as usize];
    r[..4].copy_from_slice(&[REPORT, 0x65, 1, 0]);
    r
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reading { Battery { percent: u8, charging: bool }, Offline }

fn reading(r: &[u8]) -> Option<Reading> {
    if r.len() != usize::from(LENGTH) || r[0] != REPORT { return None; }
    // 0A is an HT6 power-off notification in the command byte. 1A is a
    // disconnected link in a state reply; RGB replies use other commands.
    if r[1] == 0x0a { return Some(Reading::Offline); }
    if r[1] != 0x65 { return None; }
    match r[3] {
        0x1a => Some(Reading::Offline),
        2 | 3 if r[2] <= 100 => Some(Reading::Battery { percent: r[2], charging: r[3] == 3 }),
        _ => None,
    }
}

fn read_with(t: &Target, exchange: impl FnOnce(&[u8]) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    let i = t.hid()?;
    if t.backend != BackendId::FifineHt6 || !control(i) { return Err(UNAVAILABLE.into()); }
    let r = exchange(&request()).ok_or(UNAVAILABLE)?;
    let reading = reading(&r).ok_or(UNAVAILABLE)?;
    let battery = match reading {
        Reading::Battery { percent, charging } => Some(Battery {
            percent: Some(percent), charging: Some(charging), estimated_percent: None,
            voltage_mv: None, approximate: false, level: None,
        }),
        Reading::Offline => None,
    };
    let mut target = target(i);
    target.verified_identity = true;
    Ok(Snapshot { target, battery_at: battery.as_ref().map(|_| Instant::now()), battery,
        battery_unavailable: reading == Reading::Offline, connected: reading != Reading::Offline,
        dpi: Dpi::default(), polling_rate: Default::default() })
}

impl DeviceBackend for FifineHt6 {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i)).map(target).collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        let original = t.hid()?;
        if t.backend != BackendId::FifineHt6 || !control(original) { return Err(UNAVAILABLE.into()); }
        let interfaces = crate::hid::enumerate_all_report(api)?.interfaces;
        let candidates = candidates(&interfaces);
        let i = candidates.into_iter().find(|i| i.path.eq_ignore_ascii_case(&original.path))
            .filter(|i| proven(api, i)).ok_or(UNAVAILABLE)?;
        let lock = channel::lock(&receiver_group(i).ok_or(UNAVAILABLE)?);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(i)?;
        read_with(&target(i), |request| {
            let started = Instant::now();
            let mut remaining = 32;
            let response = device.transact(request, 1_000, |r| {
                remaining -= 1;
                reading(r).is_some() || remaining == 0
            });
            trace::record(i, BackendId::FifineHt6, "query-battery", 0x65, REPORT,
                request.len(), response.as_ref().map_or(0, Vec::len), started,
                match response.as_deref() {
                    Some(r) if reading(r).is_some() => trace::Outcome::Ok,
                    Some(_) => trace::Outcome::InvalidData,
                    None => trace::Outcome::NoReply,
                });
            response
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(instance: &str) -> HidInterface {
        HidInterface { path: format!(r"\\?\hid#vid_3142&pid_00c3&mi_00&col03#8&{instance}&0&0002#{{hid}}"),
            vendor_id: 0x3142, product_id: 0x00c3, usage_page: 0xff90, usage: 1,
            input_len: LENGTH, output_len: LENGTH, feature_len: 0,
            product: "FIFINE HT6 Wireless".into(), serial: "shared".into() }
    }

    fn frame(percent: u8, status: u8) -> Vec<u8> {
        let mut r = vec![0; usize::from(LENGTH)];
        r[..4].copy_from_slice(&[REPORT, 0x65, percent, status]);
        r
    }

    #[test]
    fn captured_ht6_prefix_decodes_eighty_percent_and_sends_only_the_state_query() {
        // Published SoundLink capture prefix, padded as the upstream replay is.
        let prefix = [0x55, 0x65, 0x50, 0x02, 0x02, 0, 0, 0xff,
            0, 0, 0, 0, 1, 8, 0, 0, 4, 2, 1, 0, 1];
        let mut capture = vec![0; usize::from(LENGTH)];
        capture[..prefix.len()].copy_from_slice(&prefix);
        let mut sent = 0;
        let sample = read_with(&target(&interface("one")), |p| {
            sent += 1;
            assert_eq!(&p[..4], &[0x55, 0x65, 1, 0]);
            assert_eq!(p.len(), 64);
            assert!(p[4..].iter().all(|b| *b == 0));
            Some(capture)
        }).unwrap();
        assert_eq!(sent, 1);
        let battery = sample.battery.unwrap();
        assert_eq!((battery.percent, battery.charging), (Some(80), Some(false)));
        assert_eq!(sample.target.family, DeviceFamily::Headset);
        assert!(sample.connected && sample.battery_at.is_some() && !sample.battery_unavailable);
        assert!(!sample.dpi.writable && sample.dpi.stages.is_empty());
    }

    #[test]
    fn synthetic_boundary_and_charging_replies_require_known_state_and_valid_percent() {
        for status in 2..=3 {
            for percent in 0..=100 {
                assert_eq!(reading(&frame(percent, status)),
                    Some(Reading::Battery { percent, charging: status == 3 }));
            }
            for percent in 101..=255 { assert!(reading(&frame(percent, status)).is_none()); }
        }
        for status in [0, 1, 4, 5, 0x0a, 0xff] {
            assert!(reading(&frame(80, status)).is_none());
        }
        assert!(reading(&request()).is_none());
    }

    #[test]
    fn power_off_and_lost_link_clear_battery_but_silence_waits_for_the_next_poll() {
        let t = target(&interface("one"));
        let mut powered_off = frame(255, 2); powered_off[1] = 0x0a;
        for response in [powered_off, frame(255, 0x1a)] {
            let sample = read_with(&t, |_| Some(response)).unwrap();
            assert!(!sample.connected && sample.battery_unavailable);
            assert!(sample.battery.is_none() && sample.battery_at.is_none());
        }
        assert!(read_with(&t, |_| None).is_err());
        let wake = read_with(&t, |_| Some(frame(79, 2))).unwrap();
        assert!(wake.connected && !wake.battery_unavailable && wake.battery_at.is_some());
        assert_eq!(wake.battery.unwrap().percent, Some(79));
    }

    #[test]
    fn other_commands_and_incomplete_reports_cannot_publish_state() {
        let valid = frame(80, 2);
        for length in 0..64 { assert!(reading(&valid[..length]).is_none()); }
        let mut long = valid.clone(); long.push(0); assert!(reading(&long).is_none());
        for (offset, value) in [(0, 0), (0, 0x54), (1, 6), (1, 0x23)] {
            let mut wrong = valid.clone(); wrong[offset] = value;
            assert!(reading(&wrong).is_none());
        }
        let mut led = frame(80, 0x1a); led[1] = 6;
        assert!(reading(&led).is_none());
    }

    #[test]
    fn endpoint_is_limited_to_ht6_mi00_and_exact_report_capacities() {
        let i = interface("one"); assert!(control(&i));
        for (vid, pid, page, usage, input, output) in [
            (0x3141, 0x00c3, 0xff90, 1, 64, 64),
            (0x3142, 0x00c2, 0xff90, 1, 64, 64),
            (0x3142, 0x00c3, 0xff82, 1, 64, 64),
            (0x3142, 0x00c3, 0xff90, 2, 64, 64),
            (0x3142, 0x00c3, 0xff90, 1, 63, 64),
            (0x3142, 0x00c3, 0xff90, 1, 64, 65),
        ] {
            let mut wrong = i.clone();
            wrong.vendor_id = vid; wrong.product_id = pid; wrong.usage_page = page;
            wrong.usage = usage; wrong.input_len = input; wrong.output_len = output;
            assert!(!control(&wrong));
            assert!(read_with(&target(&wrong), |_| panic!("ineligible endpoint must not send")).is_err());
        }
        for path in ["invalid", r"\\?\hid#vid_3142&pid_00c3&mi_01#instance#{hid}",
            r"\\?\hid#vid_3142&pid_00c3&mi_00&mi_01#instance#{hid}",
            r"\\?\hid#vid_3142&pid_00c3&pid_00c2&mi_00#instance#{hid}"] {
            let mut wrong = i.clone(); wrong.path = path.into(); assert!(!control(&wrong));
        }
        let mut backend = target(&i); backend.backend = BackendId::Headset;
        assert!(read_with(&backend, |_| panic!("wrong backend must not send")).is_err());
    }

    #[test]
    fn duplicate_collections_are_rejected_and_two_receivers_remain_separate() {
        let one = interface("one"); let two = interface("two");
        assert_eq!(candidates(&[one.clone(), two.clone()]).len(), 2);
        assert_ne!(target(&one).key, target(&two).key);
        let mut sibling = one.clone();
        sibling.path = sibling.path.replace("col03", "col04").replace("0002", "0003");
        sibling.input_len = 63;
        assert!(candidates(&[one.clone(), sibling]).is_empty());
        let mut audio = one.clone(); audio.usage_page = 0x0b;
        assert_eq!(candidates(&[one, audio]).len(), 1);
    }
}
