//! Shared FF90 / 55:65 headset battery protocol, read-only.
//! ATK and MCHOSE transport/capture evidence: Voltide, pinned in PROTOCOLS.md.
use super::*;
use crate::hid::HidDevice;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

pub struct Headset;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Model {
    AtkUltimate,
    AtkUltra,
    MchoseV9,
    MchoseV9Pro,
}

impl Model {
    fn name(self) -> &'static str {
        match self {
            Self::AtkUltimate => "ATK N9 V2 Ultimate",
            Self::AtkUltra => "ATK N9 V2 ULTRA",
            Self::MchoseV9 => "MCHOSE V9 Turbo+",
            Self::MchoseV9Pro => "MCHOSE V9 PRO",
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
        (0x291d, 0x385d) if i.input_len == 64 => Some(Model::MchoseV9Pro),
        _ => None,
    }
}

#[derive(Default)]
struct ProCache {
    firmware: BTreeMap<String, HidInterface>,
    versions: BTreeMap<String, Option<u16>>,
}

fn pro_cache() -> &'static Mutex<ProCache> {
    static CACHE: OnceLock<Mutex<ProCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(ProCache::default()))
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    model(i).is_some()
        && descriptor::Descriptor::open(api, i)
            .is_some_and(|caps| caps.only_reports(i, 0x55, i.input_len, 0x55, 64))
}

fn candidates<'a>(api: &HidApi, interfaces: &'a [HidInterface]) -> Vec<&'a HidInterface> {
    // Descriptor checks open Windows handles. Prove each collection once per scan.
    let proven: Vec<_> = interfaces.iter().filter(|i| proven(api, i)).collect();
    let mut groups = BTreeMap::new();
    for interface in &proven {
        *groups.entry(channel::collection_group(interface)).or_insert(0usize) += 1;
    }
    proven.into_iter().filter(|i| groups[&channel::collection_group(i)] == 1).collect()
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
    if !(22..=64).contains(&r.len()) || !r.starts_with(&[0x55, 0x65]) || r == query() {
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
        Model::MchoseV9 | Model::MchoseV9Pro => {
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

fn firmware_request() -> [u8; 64] {
    let mut request = [0; 64];
    request[..3].copy_from_slice(&[0xaa, 1, 1]);
    request
}

fn receiver_version(reply: &[u8]) -> Option<u16> {
    if reply.len() != 64
        || reply[..2] != [0xaa, 1]
        || reply == firmware_request()
    {
        return None;
    }
    let decimal = reply[2..6]
        .iter()
        .map(u8::to_string)
        .collect::<String>();
    decimal.parse().ok()
}

fn firmware_candidate(
    api: &HidApi,
    interfaces: &[HidInterface],
    group: &str,
) -> Option<HidInterface> {
    let candidates = interfaces
        .iter()
        .filter(|i| {
            i.vendor_id == 0x291d
                && i.product_id == 0x385d
                && i.usage_page == 0xff22
                && i.usage == 1
                && channel::collection_group(i) == group
                && descriptor::Descriptor::open(api, i)
                    .is_some_and(|caps| caps.feature_byte_report(i, 0xaa, 64))
        })
        .collect::<Vec<_>>();
    (candidates.len() == 1).then(|| candidates[0].clone())
}

fn cache_firmware_candidates(
    api: &HidApi,
    interfaces: &[HidInterface],
    controls: &[&HidInterface],
) {
    let groups = controls
        .iter()
        .filter(|i| model(i) == Some(Model::MchoseV9Pro))
        .map(|i| channel::collection_group(i))
        .collect::<BTreeSet<_>>();
    let mut cache = pro_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.firmware.clear();
    cache.versions.retain(|group, _| groups.contains(group));
    for group in groups {
        if let Some(interface) = firmware_candidate(api, interfaces, &group) {
            cache.firmware.insert(group, interface);
        }
    }
}

fn pro_version(interface: &HidInterface) -> Option<u16> {
    let group = channel::collection_group(interface);
    let firmware = {
        let mut cache = pro_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(version) = cache.versions.get(&group) {
            return *version;
        }
        let firmware = cache.firmware.get(&group).cloned();
        if firmware.is_none() {
            cache.versions.insert(group.clone(), None);
            return None;
        }
        firmware
    }?;
    let started = Instant::now();
    let result = (|| {
        let mut channel =
            feature::FeatureChannel::open_with_length(&firmware, 64)?;
        channel.send(&firmware_request(), 600)?;
        std::thread::sleep(Duration::from_millis(300));
        channel.receive(0xaa, 600)
    })();
    let version = result
        .as_ref()
        .ok()
        .and_then(|reply| receiver_version(reply));
    trace::record(
        &firmware,
        BackendId::Headset,
        "query-receiver-version",
        0xaa,
        1,
        64,
        result.as_ref().map_or(0, Vec::len),
        started,
        if result.is_ok() && version.is_some() {
            trace::Outcome::Ok
        } else {
            trace::Outcome::InvalidData
        },
    );
    pro_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .versions
        .insert(group, version);
    version
}

impl DeviceBackend for Headset {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        let controls = candidates(api, interfaces);
        cache_firmware_candidates(api, interfaces, &controls);
        Ok(controls
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
        if selected == Model::MchoseV9Pro
            && pro_version(target.hid()?).is_some_and(|version| version < 8)
        {
            return Err(crate::i18n::translate("接收器固件低于 0008").into());
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
        i.vendor_id = 0x291d;
        i.product_id = 0x385d;
        assert_eq!(model(&i), Some(Model::MchoseV9Pro));
        i.vendor_id = 0x3837;
        i.product_id = 0x600a;
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

    #[test]
    fn short_battery_reports_are_rejected_before_reading_fields() {
        for model in [Model::AtkUltimate, Model::AtkUltra, Model::MchoseV9, Model::MchoseV9Pro] {
            let frame = reply(model, 45, 2);
            let minimum = if matches!(model, Model::AtkUltimate | Model::AtkUltra) { 22 } else { 64 };
            for length in 0..minimum {
                assert_eq!(reading(model, &frame[..length]), None, "{model:?}, length {length}");
            }
            let mut oversized = frame;
            oversized.resize(65, 0);
            assert_eq!(reading(model, &oversized), None);
        }
    }

    #[test]
    fn battery_request_echo_cannot_become_external_power() {
        for model in [Model::AtkUltimate, Model::AtkUltra, Model::MchoseV9, Model::MchoseV9Pro] {
            assert_eq!(reading(model, &query()), None, "{model:?}");
            assert_eq!(reading(model, &reply(model, 1, 2)),
                Some(Reading::Level { percent: 1, charging: false }));
        }
        for model in [Model::MchoseV9, Model::MchoseV9Pro] {
            assert_eq!(reading(model, &reply(model, 0, 0)), Some(Reading::ExternalPower));
            assert_eq!(reading(model, &reply(model, 1, 3)), Some(Reading::ExternalPower));
        }
    }

    #[test]
    fn receiver_version_uses_decimal_concatenation_and_rejects_echoes() {
        let mut reply = firmware_request();
        reply[2..6].copy_from_slice(&[0, 0, 0, 8]);
        assert_eq!(receiver_version(&reply), Some(8));
        reply[2..6].copy_from_slice(&[0, 1, 2, 3]);
        assert_eq!(receiver_version(&reply), Some(123));
        let mut echo = firmware_request();
        echo[2] = 0xff;
        assert!(receiver_version(&echo).is_none());
        assert!(receiver_version(&firmware_request()).is_none());
        let mut short = firmware_request().to_vec();
        short.truncate(63);
        assert!(receiver_version(&short).is_none());
        reply[1] = 2;
        assert!(receiver_version(&reply).is_none());
    }
}
