//! Read-only BY battery query for exact VGN / VXE 2.4G keyboard receivers.
//!
//! These identities share a 20-byte report transport. The exact VID/PID pairs,
//! `FF02:0002` collection, report `0x13`, request checksum and reply checksum
//! are all required. `3554:FA09` belongs exclusively to the unified MCHOSE
//! keyboard backend and is never claimed here.

use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct VgnBeiying;

const QUERY_TIMEOUT_MS: u32 = 2_500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Model {
    name: &'static str,
    charging_known: bool,
}

fn model(interface: &HidInterface) -> Option<Model> {
    if interface.usage_page != 0xff02
        || interface.usage != 2
        || interface.input_len != 20
        || interface.output_len != 20
    {
        return None;
    }
    match (interface.vendor_id, interface.product_id) {
        (0x258a, 0x024f) => Some(Model {
            name: "VGN V98 Pro V3",
            charging_known: true,
        }),
        (0x258a, 0x027f) => Some(Model {
            name: "VGN V98 Pro V3",
            charging_known: false,
        }),
        (0x258a, 0x027b) => Some(Model {
            name: "VGN V87 V2",
            charging_known: false,
        }),
        (0x391d, 0x2a01) => Some(Model {
            name: "VGN N75 V2",
            charging_known: false,
        }),
        (0x391d, 0x2a02) => Some(Model {
            name: "VGN V108",
            charging_known: false,
        }),
        _ => None,
    }
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces
        .iter()
        .filter(|interface| {
            model(interface).is_some()
                && interfaces
                    .iter()
                    .filter(|other| {
                        other.vendor_id == interface.vendor_id
                            && other.product_id == interface.product_id
                            && model(other).is_some()
                            && channel::collection_group(interface)
                                == channel::collection_group(other)
                    })
                    .count()
                    == 1
        })
        .collect()
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    model(interface).is_some()
        && descriptor::Descriptor::open(api, interface)
            .is_some_and(|caps| caps.only_input_output_report(interface, 0x13, 20))
}

fn checksum(frame: &[u8]) -> u8 {
    frame
        .iter()
        .fold(0u8, |sum, byte| sum.wrapping_add(*byte))
}

fn request() -> [u8; 20] {
    let mut frame = [0; 20];
    frame[..3].copy_from_slice(&[0x13, 0x4a, 1]);
    frame[19] = checksum(&frame[..19]);
    frame
}

fn identity_request() -> [u8; 20] {
    let mut frame = [0; 20];
    frame[..3].copy_from_slice(&[0x13, 5, 1]);
    frame[19] = checksum(&frame[..19]);
    frame
}

fn battery(reply: &[u8], selected: Model) -> Option<Battery> {
    if reply.len() != 20
        || reply[..4] != [0x13, 0x4a, 1, 0]
        || reply == request()
        || reply == identity_request()
        || reply[19] != checksum(&reply[..19])
        || reply[5] > 100
    {
        return None;
    }
    Some(Battery {
        percent: Some(reply[5]),
        estimated_percent: None,
        charging: selected
            .charging_known
            .then_some(reply[6] & 0x10 != 0),
        voltage_mv: None,
        approximate: false,
        level: None,
    })
}

fn target(interface: &HidInterface) -> Option<Target> {
    let selected = model(interface)?;
    let mut target = Target::new(BackendId::VgnBeiying, interface, 0);
    target.name = selected.name.into();
    target.family = DeviceFamily::Keyboard;
    Some(target)
}

fn read_with(
    target: &Target,
    mut exchange: impl FnMut(&[u8], u32) -> Option<Vec<u8>>,
) -> Result<Snapshot, String> {
    let fail = || crate::i18n::translate("设备无有效状态回报").to_string();
    let selected = model(target.hid()?).ok_or_else(fail)?;
    let reply = exchange(&request(), QUERY_TIMEOUT_MS).ok_or_else(fail)?;
    let battery = battery(&reply, selected).ok_or_else(fail)?;
    Ok(Snapshot {
        target: target.clone(),
        battery: Some(battery),
        battery_at: Some(Instant::now()),
        battery_unavailable: false,
        connected: true,
        dpi: Dpi::default(),
        polling_rate: Default::default(),
    })
}

impl DeviceBackend for VgnBeiying {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces)
            .into_iter()
            .filter(|interface| proven(api, interface))
            .filter_map(target)
            .collect())
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let interface = target.hid()?;
        if target.backend != BackendId::VgnBeiying || !proven(api, interface) {
            return Err(crate::i18n::translate("设备无有效状态回报").into());
        }
        let lock = channel::lock(&channel::collection_group(interface));
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut device = HidDevice::open(interface)?;
        read_with(target, |request, timeout| {
            let started = Instant::now();
            let Some(selected) = model(interface) else {
                return None;
            };
            let response = device.transact(request, timeout, |reply| {
                battery(reply, selected).is_some()
            });
            trace::record(
                interface,
                BackendId::VgnBeiying,
                "query-battery",
                0x13,
                0x4a,
                request.len(),
                response.as_ref().map_or(0, Vec::len),
                started,
                if response.is_some() {
                    trace::Outcome::Ok
                } else {
                    trace::Outcome::NoReply
                },
            );
            response
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(vid: u16, pid: u16, instance: &str) -> HidInterface {
        HidInterface {
            path: format!(
                r"\\?\hid#vid_{vid:04x}&pid_{pid:04x}&mi_01&col02#8&{instance}&0002#{{hid}}"
            ),
            vendor_id: vid,
            product_id: pid,
            usage_page: 0xff02,
            usage: 2,
            input_len: 20,
            output_len: 20,
            feature_len: 0,
            serial: "shared".into(),
            product: "2.4G Wireless Receiver".into(),
        }
    }

    fn reply(level: u8, flags: u8) -> Vec<u8> {
        let mut frame = request();
        frame[4] = 2;
        frame[5] = level;
        frame[6] = flags;
        frame[19] = checksum(&frame[..19]);
        frame.to_vec()
    }

    #[test]
    fn exact_request_and_checksum_match_the_published_by_frame() {
        assert_eq!(
            request(),
            [
                0x13, 0x4a, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x5e
            ]
        );
        assert_eq!(checksum(&request()[..19]), request()[19]);
        assert_eq!(
            request(),
            [0x13, 0x4a, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x5e]
        );
    }

    #[test]
    fn all_exact_pairs_select_the_documented_model_and_only_024f_knows_charging() {
        let pairs = [
            (0x258a, 0x024f, "VGN V98 Pro V3", true),
            (0x258a, 0x027f, "VGN V98 Pro V3", false),
            (0x258a, 0x027b, "VGN V87 V2", false),
            (0x391d, 0x2a01, "VGN N75 V2", false),
            (0x391d, 0x2a02, "VGN V108", false),
        ];
        for (vid, pid, name, charging_known) in pairs {
            let interface = interface(vid, pid, "one");
            let selected = model(&interface).unwrap();
            assert_eq!(
                (selected.name, selected.charging_known),
                (name, charging_known)
            );
            let target = target(&interface).unwrap();
            assert_eq!((target.name.as_str(), target.family), (name, DeviceFamily::Keyboard));
            assert_eq!(battery(&reply(73, 0x10), selected).unwrap().charging,
                charging_known.then_some(true));
            assert_eq!(battery(&reply(73, 0), selected).unwrap().charging,
                charging_known.then_some(false));
        }
    }

    #[test]
    fn checksum_headers_lengths_echoes_and_invalid_percent_are_rejected() {
        let selected = model(&interface(0x258a, 0x024f, "one")).unwrap();
        assert_eq!(battery(&reply(0, 0), selected).unwrap().percent, Some(0));
        assert_eq!(battery(&reply(100, 0x10), selected).unwrap().percent, Some(100));
        for frame in [
            Vec::new(),
            reply(50, 0)[..19].to_vec(),
            [reply(50, 0), vec![0]].concat(),
            request().to_vec(),
            identity_request().to_vec(),
            reply(101, 0),
            reply(255, 0),
        ] {
            assert!(battery(&frame, selected).is_none(), "{frame:?}");
        }
        for offset in [0, 1, 2, 3, 5, 6, 19] {
            let mut frame = reply(50, 0x10);
            frame[offset] ^= 1;
            assert!(battery(&frame, selected).is_none(), "offset {offset}");
        }
    }

    #[test]
    fn only_exact_collection_report_and_identity_are_eligible() {
        let valid = interface(0x258a, 0x024f, "one");
        assert!(model(&valid).is_some());
        assert_eq!(candidates(&[valid.clone()]).len(), 1);
        for (vid, pid, page, usage, input, output) in [
            (0x3554, 0xfa09, 0xff02, 2, 20, 20),
            (0x258a, 0x010c, 0xff02, 2, 20, 20),
            (0x258a, 0x024e, 0xff02, 2, 20, 20),
            (0x258a, 0x024f, 0xff00, 2, 20, 20),
            (0x258a, 0x024f, 0xff02, 1, 20, 20),
            (0x258a, 0x024f, 0xff02, 2, 19, 20),
            (0x258a, 0x024f, 0xff02, 2, 20, 21),
        ] {
            let mut wrong = valid.clone();
            wrong.vendor_id = vid;
            wrong.product_id = pid;
            wrong.usage_page = page;
            wrong.usage = usage;
            wrong.input_len = input;
            wrong.output_len = output;
            assert!(model(&wrong).is_none());
        }
        let mut sibling = valid.clone();
        sibling.path = sibling
            .path
            .replace("col02", "col03")
            .replace("&0002", "&0003");
        assert!(candidates(&[valid.clone(), sibling]).is_empty());
        let mut duplicate = valid.clone();
        duplicate.path = duplicate.path.replace("&0002#", "&0002#");
        assert!(candidates(&[valid, duplicate]).is_empty());
    }

    #[test]
    fn one_slow_query_returns_a_sample_and_failures_never_fabricate_one() {
        let target = target(&interface(0x258a, 0x024f, "one")).unwrap();
        let mut calls = 0;
        let reading = read_with(&target, |frame, timeout| {
            calls += 1;
            assert_eq!(frame, request());
            assert_eq!(timeout, 2_500);
            Some(reply(75, 0x10))
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(
            reading.battery.as_ref().and_then(|battery| battery.percent),
            Some(75)
        );
        assert_eq!(
            reading.battery.as_ref().and_then(|battery| battery.charging),
            Some(true)
        );
        assert!(reading.connected && !reading.battery_unavailable);
        for invalid in [None, Some(request().to_vec()), Some(reply(101, 0))] {
            assert!(read_with(&target, |_, _| invalid.clone()).is_err());
        }
    }
}
