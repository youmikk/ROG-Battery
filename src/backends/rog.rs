//! ROG 深度适配沿用现有协议，独立于其他品牌的电量与 DPI 能力。
use super::*;

pub struct Rog;

pub(crate) fn identity_key(link: &crate::hid::Link, identity: &crate::proto::Identity) -> String {
    identity.device_id().map(|id| crate::device::stable_key(&format!("device:{id}")))
        .unwrap_or_else(|| link.device_key())
}

pub fn fetch(
    link: &crate::hid::Link,
    detail: bool,
) -> Result<crate::snapshot::FetchOutcome, String> {
    let started = std::time::Instant::now();
    let mut device = crate::hid::HidDevice::open(&link.iface)?;
    let outcome = if detail {
        crate::snapshot::fetch(
            &mut device,
            &link.all_interfaces,
            link.report_id,
            &link.mode,
        )
    } else {
        crate::snapshot::fetch_light(&mut device, link.report_id, &link.mode)
    };
    trace::record(
        &link.iface,
        BackendId::Rog,
        "read-state",
        0x12,
        u8::from(detail),
        0,
        outcome.snapshot.frames.values().map(Vec::len).sum(),
        started,
        if outcome.snapshot.battery().is_some() {
            trace::Outcome::Ok
        } else if outcome.snapshot.frames.contains_key("12:07:00") {
            trace::Outcome::InvalidData
        } else {
            trace::Outcome::NoReply
        },
    );
    Ok(outcome)
}

impl DeviceBackend for Rog {
    fn discover(&self, api: &HidApi, _: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(crate::hid::open_links(api)?
            .iter()
            .map(|link| {
                let mut target = Target::new(BackendId::Rog, &link.iface, link.report_id);
                target.key = link.device_key();
                target
            })
            .collect())
    }

    fn read(&self, api: &HidApi, target: &Target, detail: bool) -> Result<Snapshot, String> {
        let link = crate::hid::open_links(api)?
            .into_iter()
            .find(|link| {
                target.hid().is_ok_and(|i| link.iface.path == i.path) && link.report_id == target.address
            })
            .ok_or("ROG 设备已断开")?;
        let outcome = fetch(&link, detail)?;
        let state = &outcome.snapshot.state;
        let connected = outcome.snapshot.battery().is_some();
        let mut target = target.clone();
        let name = outcome.snapshot.identity.display_name();
        if !name.trim().is_empty() {
            target.name = name;
            target.family = crate::device::family_of(&target.name);
        }
        target.key = identity_key(&link, &outcome.snapshot.identity);
        target.verified_identity = outcome.snapshot.identity.device_id().is_some();
        let dpi = if target.family == DeviceFamily::Mouse
            && outcome.snapshot.parsed_frame("12:04:02")
            && (1..=4).contains(&state.active_stage)
        {
            Dpi {
                current: Some(
                    state.dpi(state.active_stage.saturating_sub(1).min(3) as usize) as u16,
                ),
                values: (crate::proto::DPI_MIN..=crate::proto::DPI_MAX)
                    .step_by(crate::proto::DPI_STEP as usize)
                    .map(|dpi| dpi as u16)
                    .collect(),
                writable: !link.read_only && (target.verified_identity || link.mode == "Wired"),
                ..Default::default()
            }
        } else {
            Dpi::default()
        };
        Ok(Snapshot {
            target,
            connected,
            battery_unavailable: false, battery_at: Some(std::time::Instant::now()), polling_rate: Default::default(),
            dpi,
            battery: outcome.snapshot.battery(),
        })
    }

    fn set_dpi(&self, api: &HidApi, target: &Target, dpi: u16) -> Result<(), String> {
        let value = i32::from(dpi);
        if !(crate::proto::DPI_MIN..=crate::proto::DPI_MAX).contains(&value)
            || value % crate::proto::DPI_STEP != 0
        {
            return Err("DPI 不在设备报告的可用范围内".into());
        }
        let link = crate::hid::open_links(api)?
            .into_iter()
            .find(|link| {
                target.hid().is_ok_and(|i| link.iface.path == i.path) && link.report_id == target.address
            })
            .ok_or("ROG 设备已断开")?;
        if link.read_only { return Err("接收器配对信息未确认，当前链路只允许读取".into()); }
        let current = fetch(&link, true)?;
        if target.verified_identity && identity_key(&link, &current.snapshot.identity) != target.key
        {
            return Err("无法确认所选设备，请刷新后再试".into());
        }
        if link.mode != "Wired" && !target.verified_identity {
            return Err("未确认接收器中设备的唯一标识，不能写入 DPI".into());
        }
        if !current.snapshot.parsed_frame("12:04:02")
            || !(1..=4).contains(&current.snapshot.state.active_stage)
        {
            return Err("无法读取当前 DPI".into());
        }
        let stage = usize::from(current.snapshot.state.active_stage);
        if current.snapshot.state.dpi(stage - 1) == value {
            return Ok(());
        }
        let identity = crate::snapshot::device_identity(&current.snapshot.identity)
            .ok_or("无法确认所选设备身份，请刷新后重试")?;
        let mut device = crate::hid::HidDevice::open(&link.iface)?;
        let mut reader = crate::ops::BoundReader::new(&identity);
        let mut log = |_: &str| {};
        crate::ops::set_dpi(
            &mut device,
            link.report_id,
            stage,
            value,
            None,
            crate::commands::KEEP_AFTER_WRITE,
            crate::commands::SAVE_TO_FLASH,
            &mut reader,
            &mut log,
        )
        .map_err(|error| error.to_string())?
        .into_result()
        .map(|_| ())
        .map_err(|error| error.to_string())
    }
}
