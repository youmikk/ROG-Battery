//! XIBERIA K06S receiver battery on the documented FF90 command collection.
//! Only the read-only 55:65 battery request is sent; FF82 is never queried.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct Xiberia;

const PRODUCT: &str = "XIBERIA K06S Wireless";
const REPORT: u8 = 0x55;
const LENGTH: u16 = 64;
const TIMEOUT_MS: u32 = 800;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x36f9 && i.product_id == 0xf002
        && i.usage_page == 0xff90 && i.usage == 1
}

fn receiver_group(i: &HidInterface) -> Option<String> {
    if !management(i) { return None; }
    let path = i.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid" || parts[2].is_empty()
        || parts[3].is_empty() || path.contains('\0') { return None; }
    let fields: Vec<_> = parts[1].split('&').collect();
    for required in ["vid_36f9", "pid_f002"] {
        if fields.iter().filter(|field| **field == required).count() != 1 { return None; }
    }
    let interfaces: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix("mi_")).collect();
    let [interface] = interfaces.as_slice() else { return None; };
    if interface.len() != 2 || !interface.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
    let collections: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix("col")).collect();
    let [collection] = collections.as_slice() else { return None; };
    if collection.len() != 2 || !collection.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
    let (instance, index) = parts[2].rsplit_once('&')?;
    if instance.split('&').count() < 2 || index.len() != 4
        || !index.starts_with("00") || !index.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
    let hardware = fields.into_iter().filter(|field| !field.starts_with("col"))
        .collect::<Vec<_>>().join("&");
    Some(format!("{}#{hardware}#{instance}#{}", parts[0], parts[3]))
}

fn control(i: &HidInterface) -> bool {
    management(i) && i.product.trim().eq_ignore_ascii_case(PRODUCT)
        && i.input_len == LENGTH && i.output_len == LENGTH && receiver_group(i).is_some()
        && i.path.to_ascii_lowercase().split('#').nth(1)
            .is_some_and(|hardware| hardware.split('&').any(|field| field == "col03"))
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
    let mut t = Target::new(BackendId::Xiberia, i, 0);
    t.name = PRODUCT.into();
    t.family = DeviceFamily::Headset;
    t
}

fn request() -> [u8; LENGTH as usize] {
    let mut r = [0; LENGTH as usize];
    r[..4].copy_from_slice(&[REPORT, 0x65, 0, 0]);
    r
}

fn percent(r: &[u8]) -> Option<u8> {
    // A zero-percent layout is identical to the request. No protocol field
    // distinguishes a genuine zero from an echo, so it cannot be published.
    (r.len() == usize::from(LENGTH) && r[..3] == [REPORT, 0x65, 0]
        && (1..=100).contains(&r[3])).then(|| r[3])
}

fn read_with(t: &Target, exchange: impl FnOnce(&[u8]) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    let i = t.hid()?;
    if t.backend != BackendId::Xiberia || !control(i) { return Err(UNAVAILABLE.into()); }
    let r = exchange(&request()).ok_or(UNAVAILABLE)?;
    let percent = percent(&r).ok_or(UNAVAILABLE)?;
    let mut target = target(i);
    target.verified_identity = true;
    Ok(Snapshot { target,
        battery: Some(Battery { percent: Some(percent), charging: None,
            estimated_percent: None, voltage_mv: None, approximate: false, level: None }),
        battery_at: Some(Instant::now()), battery_unavailable: false, connected: true,
        dpi: Dpi::default(), polling_rate: Default::default() })
}

impl DeviceBackend for Xiberia {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| proven(api, i)).map(target).collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        let i = t.hid()?;
        if t.backend != BackendId::Xiberia || !proven(api, i) { return Err(UNAVAILABLE.into()); }
        let lock = channel::lock(&receiver_group(i).ok_or(UNAVAILABLE)?);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(i)?;
        read_with(t, |request| {
            let started = Instant::now();
            let mut remaining = 32;
            let response = device.transact(request, TIMEOUT_MS, |r| {
                remaining -= 1;
                percent(r).is_some() || remaining == 0
            });
            trace::record(i, BackendId::Xiberia, "query-battery", 0x65, REPORT,
                request.len(), response.as_ref().map_or(0, Vec::len), started,
                match response.as_deref() {
                    Some(r) if percent(r).is_some() => trace::Outcome::Ok,
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
        HidInterface { path: format!(r"\\?\hid#vid_36f9&pid_f002&mi_00&col03#8&{instance}&0&0002#{{hid}}"),
            vendor_id: 0x36f9, product_id: 0xf002, usage_page: 0xff90, usage: 1,
            input_len: LENGTH, output_len: LENGTH, feature_len: 0,
            product: PRODUCT.into(), serial: "shared".into() }
    }

    fn frame(value: u8) -> Vec<u8> {
        let mut r = request().to_vec(); r[3] = value; r
    }

    #[test]
    fn public_k06s_full_battery_prefix_decodes_without_inventing_charge() {
        // #846 supplies the 55 65 00 64 prefix and 64-byte reply length;
        // trailing zero padding is reconstructed, not a full hardware capture.
        let sample = read_with(&target(&interface("one")), |p| {
            assert_eq!(p.len(), 64);
            assert_eq!(&p[..4], &[0x55, 0x65, 0, 0]);
            assert!(p[4..].iter().all(|b| *b == 0));
            Some(frame(100))
        }).unwrap();
        assert_eq!(sample.target.family, DeviceFamily::Headset);
        assert_eq!((sample.battery.as_ref().unwrap().percent, sample.battery.unwrap().charging),
            (Some(100), None));
        assert!(sample.connected && sample.battery_at.is_some() && !sample.battery_unavailable);
        assert!(!sample.dpi.writable && sample.dpi.stages.is_empty());
    }

    #[test]
    fn zero_echo_bad_headers_and_out_of_range_values_are_not_battery() {
        assert!(percent(&request()).is_none());
        for value in 1..=100 { assert_eq!(percent(&frame(value)), Some(value)); }
        for value in 101..=255 { assert!(percent(&frame(value)).is_none()); }
        let valid = frame(100);
        for length in 0..64 { assert!(percent(&valid[..length]).is_none()); }
        let mut long = valid.clone(); long.push(0); assert!(percent(&long).is_none());
        for (offset, value) in [(0, 0x54), (1, 0x10), (2, 1), (3, 0)] {
            let mut wrong = valid.clone(); wrong[offset] = value;
            assert!(percent(&wrong).is_none());
        }
    }

    #[test]
    fn silence_cannot_create_a_fresh_battery_and_next_valid_poll_recovers() {
        let t = target(&interface("one"));
        assert!(read_with(&t, |_| None).is_err());
        assert!(read_with(&t, |_| Some(request().to_vec())).is_err());
        let sample = read_with(&t, |_| Some(frame(99))).unwrap();
        assert!(sample.battery_at.is_some() && sample.connected);
        assert_eq!(sample.battery.unwrap().percent, Some(99));
    }

    #[test]
    fn exact_k06s_collection_excludes_other_xiberia_firmware_and_failed_ff82_channel() {
        let i = interface("one"); assert!(control(&i));
        for (vid, pid, page, usage, name, input, output) in [
            (0x36f9, 0xf008, 0xff90, 1, "XIBERIA T7", 64, 64),
            (0x36f9, 0xf001, 0xff90, 1, "XIBERIA T10 Wireless", 64, 64),
            (0x36f9, 0xf016, 0xff90, 1, "XIBERIA K03S", 64, 64),
            (0x10d6, 0x4801, 0xff90, 1, "K02BS", 64, 64),
            (0x36f9, 0xf002, 0xff82, 1, PRODUCT, 64, 64),
            (0x36f9, 0xf002, 0xff90, 2, PRODUCT, 64, 64),
            (0x36f9, 0xf002, 0xff90, 1, "XIBERIA K06S", 64, 64),
            (0x36f9, 0xf002, 0xff90, 1, PRODUCT, 63, 64),
            (0x36f9, 0xf002, 0xff90, 1, PRODUCT, 64, 65),
        ] {
            let mut wrong = i.clone();
            wrong.vendor_id = vid; wrong.product_id = pid; wrong.usage_page = page;
            wrong.usage = usage; wrong.product = name.into(); wrong.input_len = input; wrong.output_len = output;
            assert!(!control(&wrong));
            assert!(read_with(&target(&wrong), |_| panic!("ineligible collection must not send")).is_err());
        }
        let mut alternate = i.clone(); alternate.path = alternate.path.replace("col03", "col04");
        assert!(!control(&alternate));
        let mut wrong_backend = target(&i); wrong_backend.backend = BackendId::Headset;
        assert!(read_with(&wrong_backend, |_| panic!("wrong protocol must not send")).is_err());
    }

    #[test]
    fn multiple_receivers_are_distinct_and_duplicate_management_collections_are_rejected() {
        let one = interface("one"); let two = interface("two");
        assert_eq!(candidates(&[one.clone(), two.clone()]).len(), 2);
        assert_ne!(target(&one).key, target(&two).key);
        let mut second = one.clone();
        second.path = second.path.replace("col03", "col04").replace("0002", "0003");
        second.input_len = 63;
        assert!(candidates(&[one.clone(), second]).is_empty());
        let mut ff82 = one.clone(); ff82.usage_page = 0xff82;
        ff82.path = ff82.path.replace("col03", "col04").replace("0002", "0003");
        assert_eq!(candidates(&[one, ff82]).len(), 1);
    }
}
