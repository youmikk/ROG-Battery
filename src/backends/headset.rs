//! Shared FF90 / 55:65 headset battery protocol, read-only.
//! ATK and MCHOSE transport/capture evidence: Voltide, pinned in PROTOCOLS.md.
use super::*;
use crate::hid::HidDevice;
use std::time::Instant;

pub struct Headset;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Model {
    AtkUltimate,
    AtkUltra,
    MchoseV9,
}

impl Model {
    fn name(self) -> &'static str {
        match self {
            Self::AtkUltimate => "ATK N9 V2 Ultimate",
            Self::AtkUltra => "ATK N9 V2 ULTRA",
            Self::MchoseV9 => "MCHOSE V9 Turbo+",
        }
    }
}

fn model(i: &HidInterface) -> Option<Model> {
    if i.usage_page != 0xff90 || i.usage != 1 || i.output_len != 64 {
        return None;
    }
    match (i.vendor_id, i.product_id) {
        (0x373b, 0x1262) if (22..=64).contains(&i.input_len) => {
            Some(Model::AtkUltimate)
        }
        (0x373b, 0x12d6) if (22..=64).contains(&i.input_len) => {
            Some(Model::AtkUltra)
        }
        (0x3837, 0x600a) if i.input_len == 64 => Some(Model::MchoseV9),
        _ => None,
    }
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    model(i).is_some()
        && descriptor::Descriptor::open(api, i)
            .is_some_and(|caps| caps.only_reports(i, 0x55, i.input_len, 0x55, 64))
}

fn candidates<'a>(api: &HidApi, interfaces: &'a [HidInterface]) -> Vec<&'a HidInterface> {
    interfaces
        .iter()
        .filter(|i| {
            proven(api, i)
                && interfaces
                    .iter()
                    .filter(|other| {
                        proven(api, other)
                            && channel::collection_group(i)
                                == channel::collection_group(other)
                    })
                    .count()
                    == 1
        })
        .collect()
}
fn query() -> [u8; 64] {
    let mut p = [0; 64];
    p[..3].copy_from_slice(&[0x55, 0x65, 1]);
    p
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reading {
    Level { percent: u8, charging: bool },
    ExternalPower,
    Offline,
}

fn reading(model: Model, r: &[u8]) -> Option<Reading> {
    if r[..2] != [0x55, 0x65] || r.len() > 64 {
        return None;
    }
    match model {
        Model::AtkUltimate | Model::AtkUltra => {
            if !(22..=64).contains(&r.len()) || r[2] > 100 {
                return None;
            }
            match r[3] {
                2 | 4 => Some(Reading::Level {
                    percent: r[2],
                    charging: false,
                }),
                3 => Some(Reading::Level {
                    percent: r[2],
                    charging: true,
                }),
                _ => None,
            }
        }
        Model::MchoseV9 => {
            if r.len() != 64 {
                return None;
            }
            match r[3] {
                2 if r[2] <= 100 => Some(Reading::Level {
                    percent: r[2],
                    charging: false,
                }),
                0 | 3 => Some(Reading::ExternalPower),
                0x1a => Some(Reading::Offline),
                _ => None,
            }
        }
    }
}

fn battery(reading: Reading) -> (Option<Battery>, bool) {
    match reading {
        Reading::Level { percent, charging } => (
            Some(Battery {
                estimated_percent: None,
                percent: Some(percent),
                charging: Some(charging),
                voltage_mv: None,
                approximate: false,
                level: None,
            }),
            true,
        ),
        Reading::ExternalPower => (
            Some(Battery {
                estimated_percent: None,
                percent: None,
                charging: Some(true),
                voltage_mv: None,
                approximate: false,
                level: None,
            }),
            true,
        ),
        Reading::Offline => (None, false),
    }
}
impl DeviceBackend for Headset {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(api, interfaces)
            .into_iter()
            .map(|i| {
                let selected = model(i).expect("proven headset model");
                let mut target = Target::new(BackendId::Headset, i, 1);
                target.name = selected.name().into();
                target.family = DeviceFamily::Headset;
                target
            })
            .collect())
    }
    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let selected = model(target.hid()?)
            .ok_or_else(|| crate::i18n::translate("设备无有效状态回报"))?;
        if target.backend != BackendId::Headset || !proven(api, target.hid()?) {
            return Err(crate::i18n::translate("设备无有效状态回报").into());
        }
        let lock = channel::lock(&target.hid()?.path);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(target.hid()?)?;
        let started = Instant::now();
        let response = device.transact(&query(), 1200, |r| reading(selected, r).is_some());
        trace::record(
            target.hid()?,
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
        let parsed = response
            .as_deref()
            .and_then(|r| reading(selected, r))
            .ok_or_else(|| crate::i18n::translate("设备无有效状态回报"))?;
        let (battery, connected) = battery(parsed);
        Ok(Snapshot {
            target: target.clone(),
            battery,
            battery_unavailable: false,
            battery_at: connected.then(Instant::now),
            dpi: Dpi::default(),
            polling_rate: Default::default(),
            connected,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn interface(vid: u16, pid: u16, input_len: u16) -> HidInterface {
        HidInterface {
            path: "headset".into(),
            vendor_id: vid,
            product_id: pid,
            usage_page: 0xff90,
            usage: 1,
            input_len,
            output_len: 64,
            feature_len: 0,
            product: String::new(),
            serial: String::new(),
        }
    }

    fn reply(model: Model, level: u8, state: u8) -> Vec<u8> {
        let mut r = query().to_vec();
        r[2] = level;
        r[3] = state;
        if model == Model::AtkUltimate || model == Model::AtkUltra {
            r.truncate(32);
        }
        r
    }

    #[test]
    fn same_protocol_accepts_only_confirmed_audio_control_collections() {
        let mut i = interface(0x373b, 0x1262, 64);
        assert_eq!(model(&i), Some(Model::AtkUltimate));
        i.product_id = 0x12d6;
        assert_eq!(model(&i), Some(Model::AtkUltra));
        i.vendor_id = 0x3837;
        i.product_id = 0x600a;
        assert_eq!(model(&i), Some(Model::MchoseV9));
        i.product_id = 0x6008;
        assert!(model(&i).is_none());
        i.product_id = 0x600a;
        i.usage_page = 0xff82;
        assert!(model(&i).is_none());
        i.usage_page = 0xff90;
        i.input_len = 63;
        assert!(model(&i).is_none());
        i = interface(0x373b, 0x1262, 22);
        assert!(model(&i).is_some());
        i.input_len = 21;
        assert!(model(&i).is_none());
    }

    #[test]
    fn atk_accepts_descriptor_range_and_three_known_states() {
        for model in [Model::AtkUltimate, Model::AtkUltra] {
            for size in 22..=64 {
                let mut r = reply(model, 50, 2);
                r.resize(size, 0);
                assert_eq!(
                    reading(model, &r),
                    Some(Reading::Level {
                        percent: 50,
                        charging: false
                    })
                );
            }
            assert_eq!(
                reading(model, &reply(model, 45, 3)),
                Some(Reading::Level {
                    percent: 45,
                    charging: true
                })
            );
            assert_eq!(
                reading(model, &reply(model, 99, 4)),
                Some(Reading::Level {
                    percent: 99,
                    charging: false
                })
            );
            for state in [0, 1, 5, 0x1a, 0xff] {
                assert!(reading(model, &reply(model, 50, state)).is_none());
            }
            assert!(reading(model, &reply(model, 101, 2)).is_none());
            assert!(reading(model, &reply(model, 50, 2)[..21]).is_none());
        }
    }

    #[test]
    fn mchose_preserves_external_power_offline_and_invalid_states() {
        assert_eq!(
            reading(Model::MchoseV9, &reply(Model::MchoseV9, 0, 2)),
            Some(Reading::Level {
                percent: 0,
                charging: false
            })
        );
        assert_eq!(
            reading(Model::MchoseV9, &reply(Model::MchoseV9, 80, 2)),
            Some(Reading::Level {
                percent: 80,
                charging: false
            })
        );
        for state in [0, 3] {
            assert_eq!(
                reading(Model::MchoseV9, &reply(Model::MchoseV9, 50, state)),
                Some(Reading::ExternalPower)
            );
        }
        assert_eq!(
            reading(Model::MchoseV9, &reply(Model::MchoseV9, 50, 0x1a)),
            Some(Reading::Offline)
        );
        for state in [1, 4, 0xff] {
            assert!(reading(Model::MchoseV9, &reply(Model::MchoseV9, 50, state)).is_none());
        }
        assert!(reading(Model::MchoseV9, &reply(Model::MchoseV9, 101, 2)).is_none());
        assert!(reading(Model::MchoseV9, &reply(Model::MchoseV9, 50, 2)[..63]).is_none());
    }

    #[test]
    fn echo_headers_and_conversion_remain_strict() {
        let mut r = query();
        r[2] = 50;
        r[3] = 2;
        assert!(reading(Model::MchoseV9, &r).is_some());
        r[1] = 0x0a;
        assert!(reading(Model::MchoseV9, &r).is_none());
        let (parsed_battery, connected) = battery(Reading::ExternalPower);
        assert!(connected);
        assert_eq!(parsed_battery.unwrap().charging, Some(true));
        assert!(!battery(Reading::Offline).1);
    }
}
