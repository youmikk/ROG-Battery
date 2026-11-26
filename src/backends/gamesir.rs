//! Read-only GameSir Nova and Cyclone 2 receiver battery queries.
//!
//! Nova uses the unnumbered `FF7A` device-info exchange. Cyclone 2 uses
//! `FFF0:0040` Reports `0F/12`. Both are exact vendor collections and are
//! queried with fixed read-only commands only.

use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct GameSir;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Model {
    Nova,
    Cyclone2,
}

const READ_TIMEOUT_MS: u32 = 1_000;

fn model(interface: &HidInterface) -> Option<Model> {
    if interface.vendor_id != 0x3537 {
        return None;
    }
    match interface.product_id {
        0x1098 | 0x100f | 0x1040
            if interface.usage_page == 0xff7a
                && interface.input_len >= 19
                && interface.output_len >= 3 =>
        {
            Some(Model::Nova)
        }
        0x100b
            if interface.usage_page == 0xfff0
                && interface.usage == 0x0040
                && interface.input_len == 64
                && interface.output_len == 64 =>
        {
            Some(Model::Cyclone2)
        }
        _ => None,
    }
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    let Some(selected) = model(interface) else {
        return false;
    };
    descriptor::Descriptor::open(api, interface).is_some_and(|caps| match selected {
        Model::Nova => caps.single_input_output_report_ids(
            interface,
            interface.input_len,
            interface.output_len,
        ) == Some((0, 0)),
        Model::Cyclone2 => caps.only_reports(interface, 0x12, 64, 0x0f, 64),
    })
}

fn candidates<'a>(api: &HidApi, interfaces: &'a [HidInterface]) -> Vec<&'a HidInterface> {
    interfaces
        .iter()
        .filter(|interface| {
            proven(api, interface)
                && interfaces
                    .iter()
                    .filter(|other| {
                        proven(api, other)
                            && channel::collection_group(interface)
                                == channel::collection_group(other)
                    })
                    .count()
                    == 1
        })
        .collect()
}

fn nova_request() -> [u8; 3] {
    [0, 1, 1]
}

fn cyclone_request() -> [u8; 2] {
    [0x0f, 0x03]
}

fn nova_battery(reply: &[u8]) -> Option<Battery> {
    if reply.len() < 19 || reply[0] != 1 {
        return None;
    }
    match reply[18] {
        0..=100 => Some(Battery {
            percent: Some(reply[18]),
            estimated_percent: None,
            charging: Some(false),
            voltage_mv: None,
            approximate: false,
            level: None,
        }),
        0xff => Some(Battery {
            percent: None,
            estimated_percent: None,
            charging: Some(true),
            voltage_mv: None,
            approximate: false,
            level: None,
        }),
        _ => None,
    }
}

fn cyclone_battery(reply: &[u8]) -> Option<Battery> {
    if reply.len() != 64 || reply[0] != 0x12 || reply[35] > 1 || reply[36] > 100 {
        return None;
    }
    Some(Battery {
        percent: Some(reply[36]),
        estimated_percent: None,
        charging: Some(reply[35] == 1),
        voltage_mv: None,
        approximate: false,
        level: None,
    })
}

fn target(interface: &HidInterface) -> Target {
    let selected = model(interface).expect("proven GameSir interface");
    let mut target = Target::new(BackendId::GameSir, interface, 0);
    target.name = match selected {
        Model::Nova if !interface.product.trim().is_empty() => interface.product.clone(),
        Model::Nova => "GameSir Nova".into(),
        Model::Cyclone2 => "GameSir Cyclone 2".into(),
    };
    target.family = DeviceFamily::Gamepad;
    target.verified_identity = true;
    target
}

fn read_with(
    target: &Target,
    mut exchange: impl FnMut(Model, &[u8], u8, u32) -> Option<Vec<u8>>,
) -> Result<Snapshot, String> {
    let selected = model(target.hid()?).ok_or_else(|| {
        crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试").to_string()
    })?;
    let (request, input_id): (&[u8], u8) = match selected {
        Model::Nova => (&nova_request(), 0),
        Model::Cyclone2 => (&cyclone_request(), 0x12),
    };
    let reply = exchange(selected, request, input_id, READ_TIMEOUT_MS)
        .ok_or_else(|| crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试").to_string())?;
    let battery = match selected {
        Model::Nova => nova_battery(&reply),
        Model::Cyclone2 => cyclone_battery(&reply),
    }
    .ok_or_else(|| crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试").to_string())?;
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

impl DeviceBackend for GameSir {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(api, interfaces)
            .into_iter()
            .map(target)
            .collect())
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let interface = target.hid()?;
        if target.backend != BackendId::GameSir || !proven(api, interface) {
            return Err(crate::i18n::translate("设备暂未响应，请唤醒或检查连接后重试").into());
        }
        let lock = channel::lock(&channel::collection_group(interface));
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut device = HidDevice::open(interface)?;
        read_with(target, |selected, request, input_id, timeout| {
            let started = Instant::now();
            let response = device.transact_with_input_report(
                request,
                input_id,
                timeout,
                |reply| match selected {
                    Model::Nova => nova_battery(reply).is_some(),
                    Model::Cyclone2 => cyclone_battery(reply).is_some(),
                },
            );
            trace::record(
                interface,
                BackendId::GameSir,
                "query-battery",
                u16::from(input_id),
                request[0],
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

    fn interface(model: Model) -> HidInterface {
        match model {
            Model::Nova => HidInterface {
                path: r"\\?\hid#vid_3537&pid_1098&mi_01&col03#8&unit-a&0003#{hid}".into(),
                vendor_id: 0x3537,
                product_id: 0x1098,
                usage_page: 0xff7a,
                usage: 0,
                input_len: 64,
                output_len: 64,
                feature_len: 0,
                serial: "shared".into(),
                product: "GameSir Nova 2 Lite".into(),
            },
            Model::Cyclone2 => HidInterface {
                path: r"\\?\hid#vid_3537&pid_100b&mi_01&col04#8&unit-b&0004#{hid}".into(),
                vendor_id: 0x3537,
                product_id: 0x100b,
                usage_page: 0xfff0,
                usage: 0x0040,
                input_len: 64,
                output_len: 64,
                feature_len: 0,
                serial: "shared".into(),
                product: "GameSir Cyclone 2".into(),
            },
        }
    }

    fn nova_reply(battery: u8) -> Vec<u8> {
        let mut frame = vec![0; 64];
        frame[0] = 1;
        frame[18] = battery;
        frame
    }

    fn cyclone_reply(level: u8, charging: u8) -> Vec<u8> {
        let mut frame = vec![0; 64];
        frame[0] = 0x12;
        frame[35] = charging;
        frame[36] = level;
        frame
    }

    #[test]
    fn exact_fixed_requests_and_battery_fields() {
        assert_eq!(nova_request(), [0, 1, 1]);
        assert_eq!(cyclone_request(), [0x0f, 0x03]);
        for level in 0..=100 {
            assert_eq!(
                nova_battery(&nova_reply(level)).unwrap().percent,
                Some(level)
            );
            assert_eq!(
                cyclone_battery(&cyclone_reply(level, 0)).unwrap().percent,
                Some(level)
            );
        }
        let charging = nova_battery(&nova_reply(0xff)).unwrap();
        assert_eq!(charging.percent, None);
        assert_eq!(charging.charging, Some(true));
        assert_eq!(
            cyclone_battery(&cyclone_reply(50, 1)).unwrap().charging,
            Some(true)
        );
    }

    #[test]
    fn malformed_nova_and_cyclone_packets_are_rejected() {
        for battery in [101, 0xfe] {
            assert!(nova_battery(&nova_reply(battery)).is_none());
        }
        let mut nova = nova_reply(50);
        nova[0] = 0xf4;
        assert!(nova_battery(&nova).is_none());
        assert!(nova_battery(&nova[..18]).is_none());
        for (offset, values) in [
            (0, vec![0, 0x10, 0xff]),
            (35, vec![2, 255]),
            (36, vec![101, 255]),
        ] {
            for value in values {
                let mut frame = cyclone_reply(50, 0);
                frame[offset] = value;
                assert!(cyclone_battery(&frame).is_none());
            }
        }
        assert!(cyclone_battery(&cyclone_reply(50, 0)[..63]).is_none());
    }

    #[test]
    fn only_exact_pid_collection_and_capacities_are_eligible() {
        let nova = interface(Model::Nova);
        let cyclone = interface(Model::Cyclone2);
        assert_eq!(model(&nova), Some(Model::Nova));
        assert_eq!(model(&cyclone), Some(Model::Cyclone2));
        for (vid, pid, page, usage, input, output) in [
            (0x1234, 0x1098, 0xff7a, 0, 64, 64),
            (0x3537, 0x1099, 0xff7a, 0, 64, 64),
            (0x3537, 0x1098, 0xff7b, 0, 64, 64),
            (0x3537, 0x1098, 0xff7a, 0, 18, 64),
            (0x3537, 0x1098, 0xff7a, 0, 64, 2),
            (0x3537, 0x100b, 0xfff0, 0x40, 63, 64),
            (0x3537, 0x100b, 0xfff0, 0x40, 64, 63),
        ] {
            let mut wrong = nova.clone();
            wrong.vendor_id = vid;
            wrong.product_id = pid;
            wrong.usage_page = page;
            wrong.usage = usage;
            wrong.input_len = input;
            wrong.output_len = output;
            assert!(model(&wrong).is_none());
        }
        assert_eq!(target(&nova).family, DeviceFamily::Gamepad);
        assert_eq!(target(&nova).name, "GameSir Nova 2 Lite");
        assert_eq!(target(&cyclone).name, "GameSir Cyclone 2");
    }

    #[test]
    fn one_exchange_returns_nova_or_cyclone_snapshot() {
        for (selected, reply) in [
            (Model::Nova, nova_reply(73)),
            (Model::Cyclone2, cyclone_reply(64, 1)),
        ] {
            let target = target(&interface(selected));
            let expected_request = match selected {
                Model::Nova => nova_request().to_vec(),
                Model::Cyclone2 => cyclone_request().to_vec(),
            };
            let mut calls = 0;
            let reading = read_with(&target, |model_seen, request, input_id, timeout| {
                calls += 1;
                assert_eq!(model_seen, selected);
                assert_eq!(request, expected_request);
                assert_eq!(
                    input_id,
                    match selected {
                        Model::Nova => 0,
                        Model::Cyclone2 => 0x12,
                    }
                );
                assert_eq!(timeout, READ_TIMEOUT_MS);
                Some(reply.clone())
            })
            .unwrap();
            assert_eq!(calls, 1);
            assert_eq!(
                reading.battery.as_ref().and_then(|battery| battery.percent),
                Some(if selected == Model::Nova { 73 } else { 64 })
            );
        }
    }
}
