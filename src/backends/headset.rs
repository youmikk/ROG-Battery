//! Shared FF90 / 55:65 headset battery protocol, read-only.
//! ATK and MCHOSE transport/capture evidence: Voltide, pinned in PROTOCOLS.md.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct Headset;
fn model(i: &HidInterface) -> Option<&'static str> {
    if i.usage_page != 0xff90 || i.input_len != 64 || i.output_len != 64 {
        return None;
    }
    match (i.vendor_id, i.product_id) {
        (0x373b, 0x1262) => Some("ATK N9 V2 Ultimate"),
        (0x3837, 0x600a) => Some("MCHOSE V9 Turbo+"),
        _ => None,
    }
}
fn query() -> [u8; 64] {
    let mut p = [0; 64];
    p[..3].copy_from_slice(&[0x55, 0x65, 1]);
    p
}
fn battery(r: &[u8]) -> Option<Battery> {
    if r.len() != 64 || r[..2] != [0x55, 0x65] || r[2] > 100 {
        return None;
    }
    // State 0 is an echoed query, not a 1% measurement. State 0x1a is unknown.
    // Do not force 100% from the full flag: retain the returned percentage.
    let charging = match r[3] {
        2 | 4 => false,
        3 => true,
        _ => return None,
    };
    Some(Battery {
        estimated_percent: None,
        percent: Some(r[2]),
        charging: Some(charging),
        voltage_mv: None,
        approximate: false,
        level: None,
    })
}
impl DeviceBackend for Headset {
    fn discover(&self, _: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(interfaces
            .iter()
            .filter_map(|i| {
                let name = model(i)?;
                let mut target = Target::new(BackendId::Headset, i, 1);
                target.name = name.into();
                target.family = DeviceFamily::Headset;
                Some(target)
            })
            .collect())
    }
    fn read(&self, _: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        model(&target.interface).ok_or_else(|| crate::i18n::translate("设备无有效状态回报"))?;
        let lock = channel::lock(&target.interface.path);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(&target.interface)?;
        let started = Instant::now();
        let response = device.transact(&query(), 1200, |r| battery(r).is_some());
        trace::record(
            &target.interface,
            BackendId::Headset,
            "query-battery",
            0x55,
            0x65,
            64,
            response.as_ref().map_or(0, Vec::len),
            started,
            if response.is_some() {
                trace::Outcome::Ok
            } else {
                trace::Outcome::NoReply
            },
        );
        let battery = response
            .as_deref()
            .and_then(battery)
            .ok_or_else(|| crate::i18n::translate("设备无有效状态回报"))?;
        Ok(Snapshot {
            target: target.clone(),
            battery: Some(battery),
            battery_at: Some(Instant::now()),
            dpi: Dpi::default(),
            polling_rate: Default::default(),
            connected: true,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_protocol_accepts_only_confirmed_audio_control_collections() {
        let mut i = HidInterface {
            path: "headset".into(),
            vendor_id: 0x373b,
            product_id: 0x1262,
            usage_page: 0xff90,
            usage: 1,
            input_len: 64,
            output_len: 64,
            feature_len: 0,
            product: String::new(),
            serial: String::new(),
        };
        assert!(model(&i).is_some());
        i.vendor_id = 0x3837;
        i.product_id = 0x600a;
        assert!(model(&i).is_some());
        i.product_id = 0x6008;
        assert!(model(&i).is_none());
        i.product_id = 0x600a;
        i.usage_page = 0xff82;
        assert!(model(&i).is_none());
        i.usage_page = 0xff90;
        i.input_len = 65;
        assert!(model(&i).is_none());
    }
    #[test]
    fn valid_zero_full_and_charging_are_distinct_from_echo_and_invalid_status() {
        let mut r = query();
        assert!(battery(&r).is_none());
        r[2] = 0;
        r[3] = 2;
        assert_eq!(battery(&r).unwrap().percent, Some(0));
        r[2] = 45;
        r[3] = 3;
        assert_eq!(battery(&r).unwrap().charging, Some(true));
        r[2] = 99;
        r[3] = 4;
        assert_eq!(battery(&r).unwrap().percent, Some(99));
        r[3] = 0x1a;
        assert!(battery(&r).is_none());
        r[3] = 2;
        r[2] = 255;
        assert!(battery(&r).is_none());
        r[2] = 100;
        assert!(battery(&r[..63]).is_none());
        r[1] = 0x0a;
        assert!(battery(&r).is_none());
    }
}
