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
    let mut identity_frames = 0;
    let mut outcome = fetch_bound_with(&mut device,
        |device| read_live_identity_with(link.report_id, |rid, cmd, sub, tail| {
            let frame = device.xfer(rid, cmd, sub, tail, crate::hid::DEFAULT_TIMEOUT_MS);
            if frame.as_ref().is_some_and(|frame| !crate::hid::is_nak(frame)) { identity_frames += 1; }
            frame
        }),
        |device| if detail {
            crate::snapshot::fetch(device, &link.all_interfaces, link.report_id, &link.mode)
        } else {
            crate::snapshot::fetch_light(device, link.report_id, &link.mode)
        },
    )?;
    outcome.elapsed = started.elapsed();
    outcome.frames_read += identity_frames;
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

fn read_live_identity_with(
    report_id: u8,
    mut ask: impl FnMut(u8, u8, u8, &[u8]) -> Option<Vec<u8>>,
) -> crate::proto::Identity {
    let mut identity = crate::proto::Identity::default();
    if let Some(frame) = ask(report_id, 0x12, 0x12, &[2, 0]) {
        if frame.starts_with(&[report_id, 0x12, 0x12]) {
            identity.mouse_sn = crate::proto::parse_sn(&frame);
        }
    }
    if let Some(frame) = ask(report_id, 0xFB, 0x02, &[0, 0]) {
        if frame.starts_with(&[report_id, 0xFB, 0x02]) {
            let mac = crate::proto::Identity {
                mouse_mac: crate::proto::parse_mac(&frame), ..Default::default()
            };
            identity.mouse_mac = mac.device_id().unwrap_or_default().to_string();
        }
    }
    identity
}

fn fetch_bound_with<Channel>(
    channel: &mut Channel,
    mut read_identity: impl FnMut(&mut Channel) -> crate::proto::Identity,
    read_sample: impl FnOnce(&mut Channel) -> crate::snapshot::FetchOutcome,
) -> Result<crate::snapshot::FetchOutcome, String> {
    let before = read_identity(channel);
    let mut outcome = read_sample(channel);
    let after = read_identity(channel);
    let middle = &outcome.snapshot.identity;
    if before.conflicts_with(&after) || before.conflicts_with(middle)
        || middle.conflicts_with(&after) {
        return Err("读取过程中设备连接状态发生变化，请稍候刷新".into());
    }
    let mut verified = crate::proto::Identity::default();
    if !before.mouse_sn.trim().is_empty() && before.mouse_sn.trim() == after.mouse_sn.trim() {
        verified.mouse_sn = before.mouse_sn.trim().to_string();
    }
    if let (Some(before), Some(after)) = (valid_mac(&before), valid_mac(&after)) {
        if before.eq_ignore_ascii_case(after) { verified.mouse_mac = before.to_ascii_uppercase(); }
    }
    if verified.device_id().is_none() && (before.device_id().is_some()
        || after.device_id().is_some() || middle.device_id().is_some()) {
        return Err("无法确认读取前后是否为同一台设备，已放弃本次更新".into());
    }
    if verified.device_id().is_some() && !before.same_device(&after) {
        return Err("前后两次识别到的设备信息不一致，请刷新重试".into());
    }
    let snapshot = std::sync::Arc::make_mut(&mut outcome.snapshot);
    snapshot.identity.mouse_sn = verified.mouse_sn;
    snapshot.identity.mouse_mac = verified.mouse_mac;
    snapshot.cache_key = None;
    if snapshot.identity.mouse_sn.is_empty() { snapshot.frames.remove("12:12:02"); }
    if snapshot.identity.mouse_mac.is_empty() { snapshot.frames.remove("FB:02:00"); }
    Ok(outcome)
}

fn valid_mac(identity: &crate::proto::Identity) -> Option<&str> {
    let mac = crate::proto::Identity { mouse_mac: identity.mouse_mac.clone(), ..Default::default() };
    mac.device_id().map(|_| identity.mouse_mac.trim())
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
                writable: !link.read_only && target.verified_identity,
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
        if !target.verified_identity { return Err("未确认所选设备的唯一标识，不能写入 DPI".into()); }
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
        if identity_key(&link, &current.snapshot.identity) != target.key {
            return Err("无法确认所选设备，请刷新后再试".into());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{proto::Identity, snapshot::FetchOutcome};
    use std::collections::VecDeque;

    fn identity(serial: &str, mac: &str) -> Identity {
        Identity { mouse_sn: serial.into(), mouse_mac: mac.into(), ..Default::default() }
    }

    fn sample() -> FetchOutcome {
        crate::snapshot::fetch_light_with(3, "Wireless24G", |rid, cmd, sub, _| {
            if (cmd, sub) != (0x12, 0x07) { return None; }
            let mut frame = vec![0; 64];
            frame[..3].copy_from_slice(&[rid, cmd, sub]);
            frame[5] = 75;
            Some(frame)
        })
    }

    fn fetch_sample(before: Identity, middle: Identity, after: Identity) -> Result<FetchOutcome, String> {
        let mut channel = (VecDeque::from([before, after]), Vec::new());
        let result = fetch_bound_with(&mut channel,
            |channel| {
                channel.1.push("identity");
                channel.0.pop_front().unwrap()
            },
            |channel| {
                channel.1.push("sample");
                let mut outcome = sample();
                std::sync::Arc::make_mut(&mut outcome.snapshot).identity = middle;
                outcome
            },
        );
        assert_eq!(channel.1, ["identity", "sample", "identity"]);
        assert!(channel.0.is_empty());
        result
    }

    #[test]
    fn light_snapshot_publishes_the_identity_confirmed_around_its_read() {
        let known = identity("mouse-a", "12:34:56:78:9A:BC");
        let outcome = fetch_sample(known.clone(), Identity::default(), known).unwrap();
        assert_eq!(outcome.snapshot.identity.mouse_sn, "mouse-a");
        assert_eq!(outcome.snapshot.identity.mouse_mac, "12:34:56:78:9A:BC");
        assert!(outcome.snapshot.battery().is_some());
    }

    #[test]
    fn replacement_during_light_read_discards_the_entire_sample() {
        assert!(fetch_sample(identity("mouse-a", ""), Identity::default(), identity("mouse-b", "")).is_err());
    }

    #[test]
    fn identity_lost_or_gained_during_the_read_is_not_bound_to_either_device() {
        let known = identity("mouse-a", "");
        assert!(fetch_sample(known.clone(), Identity::default(), Identity::default()).is_err());
        assert!(fetch_sample(Identity::default(), Identity::default(), known).is_err());
    }

    #[test]
    fn devices_without_identity_remain_readable_as_unknown() {
        let outcome = fetch_sample(Identity::default(), Identity::default(), Identity::default()).unwrap();
        assert!(outcome.snapshot.identity.device_id().is_none());
        assert!(outcome.snapshot.battery().is_some());
        assert!(outcome.snapshot.cache_key.is_none());
    }

    #[test]
    fn serial_availability_changes_publish_only_the_mac_proven_on_both_sides() {
        let mac = "12:34:56:78:9A:BC";
        for (before, after) in [(identity("mouse-a", mac), identity("", mac)),
            (identity("", mac), identity("mouse-a", mac))] {
            let outcome = fetch_sample(before, identity("mouse-a", mac), after).unwrap();
            assert!(outcome.snapshot.identity.mouse_sn.is_empty());
            assert_eq!(outcome.snapshot.identity.mouse_mac, mac);
        }
    }

    #[test]
    fn incomparable_identity_types_do_not_authorize_the_sample() {
        assert!(fetch_sample(identity("mouse-a", ""), Identity::default(),
            identity("", "12:34:56:78:9A:BC")).is_err());
    }

    #[test]
    fn contradictory_middle_identity_is_rejected_even_if_the_outer_reads_match() {
        let known = identity("mouse-a", "");
        assert!(fetch_sample(known.clone(), identity("mouse-b", ""), known).is_err());
        assert!(fetch_sample(Identity::default(), identity("mouse-b", ""), Identity::default()).is_err());
    }

    #[test]
    fn a_changed_mac_is_not_hidden_by_a_matching_serial() {
        assert!(fetch_sample(identity("mouse-a", "12:34:56:78:9A:BC"), Identity::default(),
            identity("mouse-a", "12:34:56:78:9A:BD")).is_err());
    }

    #[test]
    fn mac_case_changes_and_missing_mac_keep_only_revalidated_fields() {
        let outcome = fetch_sample(identity("", "12:34:56:78:9a:bc"), Identity::default(),
            identity("", "12:34:56:78:9A:BC")).unwrap();
        assert_eq!(outcome.snapshot.identity.mouse_mac, "12:34:56:78:9A:BC");
        let outcome = fetch_sample(identity("mouse-a", "12:34:56:78:9A:BC"),
            identity("mouse-a", "12:34:56:78:9A:BC"), identity("mouse-a", "")).unwrap();
        assert_eq!(outcome.snapshot.identity.mouse_sn, "mouse-a");
        assert!(outcome.snapshot.identity.mouse_mac.is_empty());
    }

    #[test]
    fn live_identity_reads_both_types_on_the_same_channel() {
        let mut requests = Vec::new();
        let identity = read_live_identity_with(7, |rid, cmd, sub, tail| {
            requests.push((rid, cmd, sub, tail.to_vec()));
            let mut frame = vec![0; 64];
            frame[..3].copy_from_slice(&[rid, cmd, sub]);
            if cmd == 0x12 { frame[5..12].copy_from_slice(b"mouse-a"); }
            else { frame[6..12].copy_from_slice(&[0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC]); }
            Some(frame)
        });
        assert_eq!(requests, [(7, 0x12, 0x12, vec![2, 0]), (7, 0xFB, 0x02, vec![0, 0])]);
        assert_eq!(identity.mouse_sn, "mouse-a");
        assert_eq!(identity.mouse_mac, "12:34:56:78:9A:BC");
    }

    #[test]
    fn live_identity_parser_rejects_wrong_headers_and_placeholder_macs() {
        for report_id in [2, 3] {
            for bytes in [[0; 6], [0xFF; 6]] {
                let identity = read_live_identity_with(3, |_, cmd, sub, _| {
                    let mut frame = vec![0; 64];
                    frame[..3].copy_from_slice(&[report_id, cmd, sub]);
                    if cmd == 0xFB { frame[6..12].copy_from_slice(&bytes); }
                    Some(frame)
                });
                assert!(identity.device_id().is_none());
            }
        }
        let identity = read_live_identity_with(3, |_, _, _, _| Some(vec![3, 0xFF, 0xAA]));
        assert!(identity.device_id().is_none());
    }
}
