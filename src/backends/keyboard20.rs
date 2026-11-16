//! Read-only Report 0x13 keyboard status. Transport evidence is in PROTOCOLS.md.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct Keyboard20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Model {
    SharedReceiver,
    Z75sCandidate,
}

fn model(i: &HidInterface) -> Option<Model> {
    if i.usage_page != 0xff02 || i.usage != 2 || i.input_len != 20 || i.output_len != 20 {
        return None;
    }
    match (i.vendor_id, i.product_id) {
        (0x3554, 0xfa09) => Some(Model::SharedReceiver),
        (0x258a, 0x01bb) => Some(Model::Z75sCandidate),
        _ => None,
    }
}

fn name(i: &HidInterface) -> &'static str {
    // FA09 is shared by several keyboards. Only a specific product string
    // justifies a model name; a generic receiver label does not identify one.
    let product = i.product.to_ascii_uppercase();
    for (token, name) in [
        ("G75 PRO", "MCHOSE G75 Pro"), ("K99 V2", "MCHOSE K99 V2"),
        ("K99V2", "MCHOSE K99 V2"), ("GX87", "MCHOSE GX87"),
    ] {
        if product.contains(token) { return name; }
    }
    "MCHOSE Keyboard"
}

fn checksum(frame: &[u8]) -> u8 {
    frame.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte))
}

pub(super) fn query(command: u8) -> [u8; 20] {
    let mut frame = [0; 20];
    frame[..3].copy_from_slice(&[0x13, command, 1]);
    frame[19] = checksum(&frame[..19]);
    frame
}

fn reply(frame: &[u8], command: u8, length: u8) -> bool {
    frame.len() == 20 && frame[..5] == [0x13, command, 1, 0, length]
        && frame[19] == checksum(&frame[..19])
}

pub(super) fn battery(frame: &[u8]) -> Option<Battery> {
    if !reply(frame, 0x4a, 2) || frame[5] > 100 { return None; }
    Some(Battery {
        estimated_percent: None,
        percent: Some(frame[5]),
        charging: match frame[6] >> 4 { 0 => Some(false), 1 => Some(true), _ => None },
        voltage_mv: None,
        approximate: false,
        level: None,
    })
}

fn read_with(target: &Target, mut exchange: impl FnMut(u8) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    let fail = || crate::i18n::translate("设备无有效状态回报").to_string();
    let selected = model(&target.interface).ok_or_else(fail)?;
    let mut target = target.clone();
    if selected == Model::Z75sCandidate {
        let identity = exchange(5).ok_or_else(fail)?;
        if !reply(&identity, 5, 6) || identity[5..11] != [0x0a, 0, 0, 0, 0, 9] {
            return Err(fail());
        }
        target.name = "MCHOSE Z75S".into();
        target.verified_identity = true;
    }
    let response = exchange(0x4a).ok_or_else(fail)?;
    let battery = battery(&response).ok_or_else(fail)?;
    Ok(Snapshot {
        target, battery: Some(battery), battery_at: Some(Instant::now()),
        dpi: Dpi::default(), polling_rate: Default::default(), connected: true,
    })
}

impl DeviceBackend for Keyboard20 {
    fn discover(&self, _: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(interfaces.iter().filter(|i| model(i).is_some()).map(|i| {
            let mut target = Target::new(BackendId::Keyboard20, i, 0);
            target.name = name(i).into();
            target.family = DeviceFamily::Keyboard;
            target
        }).collect())
    }

    fn read(&self, _: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let selected = model(&target.interface)
            .ok_or_else(|| crate::i18n::translate("设备无有效状态回报"))?;
        let lock = channel::lock(&target.interface.path);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(&target.interface)?;
        read_with(target, |command| {
            let started = Instant::now();
            let matches = |r: &[u8]| if command == 5 { reply(r, 5, 6) } else { battery(r).is_some() };
            // FA09 captures use SET_REPORT; Z75S's verified Windows handshake
            // uses interrupt OUT. Never probe the ordinary key input collection.
            let response = match selected {
                Model::SharedReceiver => device.transact_control_output(&query(command), 800, matches),
                Model::Z75sCandidate => device.transact(&query(command), 800, matches),
            };
            trace::record(&target.interface, BackendId::Keyboard20, "query-keyboard",
                0x13, command, 20, response.as_ref().map_or(0, Vec::len), started,
                if response.is_some() { trace::Outcome::Ok } else { trace::Outcome::NoReply });
            response
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface() -> HidInterface {
        HidInterface { path: "keyboard-a".into(), vendor_id: 0x3554, product_id: 0xfa09,
            usage_page: 0xff02, usage: 2, input_len: 20, output_len: 20, feature_len: 0,
            product: "2.4G Wireless Receiver".into(), serial: "shared".into() }
    }
    fn response(command: u8, payload: &[u8]) -> Vec<u8> {
        let mut frame = query(command);
        frame[4] = payload.len() as u8;
        frame[5..5 + payload.len()].copy_from_slice(payload);
        frame[19] = checksum(&frame[..19]);
        frame.to_vec()
    }

    #[test]
    fn captured_reply_distinguishes_full_charging_zero_and_unknown_flags() {
        // Voltide's M HUB capture: 13 4A 01 00 02 64 01 ... C5.
        let full = response(0x4a, &[100, 1]);
        assert_eq!(full[19], 0xc5);
        let b = battery(&full).unwrap();
        assert_eq!((b.percent, b.charging), (Some(100), Some(false)));
        let charging = battery(&response(0x4a, &[0, 0x10])).unwrap();
        assert_eq!((charging.percent, charging.charging), (Some(0), Some(true)));
        let unknown = battery(&response(0x4a, &[99, 0x21])).unwrap();
        assert_eq!((unknown.percent, unknown.charging), (Some(99), None));
        assert!(battery(&response(0x4a, &[101, 0])).is_none());
    }

    #[test]
    fn echo_corruption_wrong_command_address_and_truncation_are_not_measurements() {
        assert!(battery(&query(0x4a)).is_none());
        let valid = response(0x4a, &[64, 0]);
        for end in 0..20 { assert!(battery(&valid[..end]).is_none()); }
        for index in [0, 1, 2, 3, 4, 19] {
            let mut changed = valid.clone();
            changed[index] ^= 1;
            if index != 19 { changed[19] = checksum(&changed[..19]); }
            assert!(battery(&changed).is_none(), "field {index}");
        }
    }

    #[test]
    fn shared_pid_requires_identity_before_battery_and_never_accepts_other_model() {
        let mut i = interface();
        i.vendor_id = 0x258a;
        i.product_id = 0x01bb;
        let target = Target::new(BackendId::Keyboard20, &i, 0);
        let mut commands = Vec::new();
        let reading = read_with(&target, |command| {
            commands.push(command);
            Some(if command == 5 { response(5, &[10, 0, 0, 0, 0, 9]) }
                else { response(0x4a, &[57, 0]) })
        }).unwrap();
        assert_eq!(commands, [5, 0x4a]);
        assert_eq!(reading.target.name, "MCHOSE Z75S");
        assert!(reading.target.verified_identity);
        commands.clear();
        assert!(read_with(&target, |command| {
            commands.push(command);
            Some(response(5, &[10, 0, 0, 0, 0, 8]))
        }).is_err());
        assert_eq!(commands, [5]);
        assert!(read_with(&target, |_| None).is_err());
    }

    #[test]
    fn catalog_restricts_control_endpoint_and_keeps_shared_receiver_name_neutral() {
        let i = interface();
        assert_eq!(model(&i), Some(Model::SharedReceiver));
        assert_eq!(name(&i), "MCHOSE Keyboard");
        let mut other = i.clone();
        other.product = "MCHOSE GX87".into();
        assert_eq!(name(&other), "MCHOSE GX87");
        other.usage = 1;
        assert!(model(&other).is_none());
        other = i.clone(); other.input_len = 17;
        assert!(model(&other).is_none());
        other = i.clone(); other.product_id = 0xfa08;
        assert!(model(&other).is_none());
        other = i.clone(); other.path = "keyboard-b".into();
        let first = Target::new(BackendId::Keyboard20, &i, 0);
        let second = Target::new(BackendId::Keyboard20, &other, 0);
        assert_ne!(first.key, second.key);
        assert!(!first.same_channel(&second));
    }
}
