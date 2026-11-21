//! VGN Dragonfly3 SE receiver battery only. No online/configuration probes.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct VgnDragonfly3;
const LENGTH: u16 = 65;
const UNAVAILABLE: &str = "设备无有效状态回报";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x391d && i.product_id == 0x1a01 && i.usage_page == 0xff01 && i.usage == 0x10
}

fn control(i: &HidInterface) -> bool {
    management(i) && i.input_len == LENGTH && i.output_len == LENGTH
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    control(i)
        && descriptor::Descriptor::open(api, i)
            .is_some_and(|d| d.only_reports(i, 0, LENGTH, 0, LENGTH))
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces
        .iter()
        .filter(|i| {
            control(i)
                && interfaces
                    .iter()
                    .filter(|other| {
                        management(other)
                            && channel::collection_group(i) == channel::collection_group(other)
                    })
                    .count()
                    == 1
        })
        .collect()
}

fn candidate(i: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::VgnDragonfly3, i, 0);
    target.name = "VGN Dragonfly3 SE".into();
    target.family = DeviceFamily::Mouse;
    target
}

fn request() -> [u8; 65] {
    let mut packet = [0; 65];
    packet[..3].copy_from_slice(&[0, 0x55, 0x30]);
    packet
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reply {
    Battery(u8),
    Wired,
}

fn reply(frame: &[u8]) -> Option<Reply> {
    // Unnumbered Windows input omits the leading report-ID slot.
    if frame.len() != 64 || frame[1] != 0x30 || frame == &request()[1..] {
        return None;
    }
    match frame[9] {
        0 if frame[8] <= 100 => Some(Reply::Battery(frame[8])),
        1 => Some(Reply::Wired),
        _ => None,
    }
}

impl DeviceBackend for VgnDragonfly3 {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces)
            .into_iter()
            .filter(|i| proven(api, i))
            .map(candidate)
            .collect())
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let interface = target.hid()?;
        if target.backend != BackendId::VgnDragonfly3 || !proven(api, interface) {
            return Err(UNAVAILABLE.into());
        }
        let lock = channel::lock(&channel::collection_group(interface));
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(interface)?;
        let started = Instant::now();
        let response = device
            .transact(&request(), 1000, |r| reply(r).is_some())
            .and_then(|r| reply(&r));
        trace::record(
            interface,
            BackendId::VgnDragonfly3,
            "query-battery",
            0,
            0x30,
            usize::from(LENGTH),
            if response.is_some() { 64 } else { 0 },
            started,
            if response.is_some() {
                trace::Outcome::Ok
            } else {
                trace::Outcome::NoReply
            },
        );
        let response = response.ok_or(UNAVAILABLE)?;
        let mut target = candidate(interface);
        target.verified_identity = true;
        let (battery, battery_at, unavailable) = match response {
            Reply::Battery(percent) => (
                Some(Battery {
                    percent: Some(percent),
                    estimated_percent: None,
                    charging: None,
                    voltage_mv: None,
                    approximate: false,
                    level: None,
                }),
                Some(Instant::now()),
                false,
            ),
            Reply::Wired => (None, None, true),
        };
        Ok(Snapshot {
            target,
            battery,
            battery_at,
            battery_unavailable: unavailable,
            connected: true,
            dpi: Dpi::default(),
            polling_rate: Default::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(instance: &str) -> HidInterface {
        HidInterface {
            path: format!(r"\\?\hid#vid_391d&pid_1a01&mi_02&col04#8&{instance}&0004#{{hid}}"),
            vendor_id: 0x391d,
            product_id: 0x1a01,
            usage_page: 0xff01,
            usage: 0x10,
            input_len: 65,
            output_len: 65,
            feature_len: 0,
            product: "VGN Dragonfly3 SE".into(),
            serial: "shared".into(),
        }
    }

    fn frame(percent: u8, wired: u8) -> Vec<u8> {
        let mut frame = vec![0xa5; 64];
        frame[1] = 0x30;
        frame[8] = percent;
        frame[9] = wired;
        frame
    }

    #[test]
    fn exact_receiver_collection_and_unnumbered_report_lengths_are_required() {
        let i = interface("one");
        assert!(management(&i) && control(&i));
        for (vendor, product, page, usage, input, output) in [
            (0x391d, 0x1a03, 0xff01, 0x10, 65, 65),
            (0x391d, 0x1001, 0xff01, 0x10, 65, 65),
            (0x391d, 0x1a01, 0xff05, 0x02, 65, 65),
            (0x391d, 0x1a01, 0xff01, 0x10, 64, 65),
            (0x391d, 0x1a01, 0xff01, 0x10, 65, 66),
        ] {
            let mut wrong = i.clone();
            wrong.vendor_id = vendor;
            wrong.product_id = product;
            wrong.usage_page = page;
            wrong.usage = usage;
            wrong.input_len = input;
            wrong.output_len = output;
            assert!(!control(&wrong));
        }
        assert_eq!(candidate(&i).family, DeviceFamily::Mouse);
        assert!(!candidate(&i).verified_identity);
    }

    #[test]
    fn request_uses_one_zero_id_slot_and_two_command_bytes() {
        let packet = request();
        assert_eq!(packet.len(), 65);
        assert_eq!(&packet[..3], &[0, 0x55, 0x30]);
        assert!(packet[3..].iter().all(|b| *b == 0));
    }

    #[test]
    fn public_reply_shape_distinguishes_percentage_wired_echo_and_invalid_values() {
        assert_eq!(reply(&frame(63, 0)), Some(Reply::Battery(63)));
        assert_eq!(reply(&frame(0, 0)), Some(Reply::Battery(0)));
        assert_eq!(reply(&frame(100, 0)), Some(Reply::Battery(100)));
        assert_eq!(reply(&frame(99, 1)), Some(Reply::Wired));
        assert!(reply(&request()[1..]).is_none());
        for (percent, wired) in [(101, 0), (255, 0), (50, 2), (50, 255)] {
            assert!(reply(&frame(percent, wired)).is_none());
        }
        let mut wrong = frame(50, 0);
        wrong[1] = 0x31;
        assert!(reply(&wrong).is_none());
        for length in 0..64 {
            assert!(reply(&frame(50, 0)[..length]).is_none());
        }
    }

    #[test]
    fn charging_is_unknown_and_wired_mode_explicitly_clears_battery() {
        let battery = match reply(&frame(63, 0)) {
            Some(Reply::Battery(percent)) => Battery {
                percent: Some(percent),
                estimated_percent: None,
                charging: None,
                voltage_mv: None,
                approximate: false,
                level: None,
            },
            _ => panic!("expected battery"),
        };
        assert_eq!(battery.charging, None);
        assert_eq!(reply(&frame(63, 1)), Some(Reply::Wired));
    }

    #[test]
    fn identical_receivers_keep_separate_interface_instances() {
        let first = candidate(&interface("one"));
        let second = candidate(&interface("two"));
        assert_ne!(first.key, second.key);
        assert!(!first.same_channel(&second));
    }
}
