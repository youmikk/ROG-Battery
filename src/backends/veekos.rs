//! Veekos Shine 60 2.4G receiver status page; read-only Feature access.
use super::*;

pub struct VeekosShine60;

const VENDOR: u16 = 0x3151;
const PRODUCT: u16 = 0x5026;
const NAME: &str = "Veekos 2.4G Wireless Keyboard";
const REPORT_BYTES: u16 = 65;

fn management(i: &HidInterface) -> bool {
    i.vendor_id == VENDOR && i.product_id == PRODUCT && i.product == NAME
        && i.usage_page == 0xffff && i.usage == 2 && i.input_len == 0
        && i.output_len == 0 && i.feature_len == REPORT_BYTES
        && i.path.to_ascii_lowercase().contains("&mi_02")
}

fn report_id(api: &HidApi, i: &HidInterface) -> Option<u8> {
    if !management(i) { return None; }
    let descriptor = descriptor::Descriptor::open(api, i)?;
    [0x83, 0].into_iter().find(|id|
        descriptor.feature_reports(i, REPORT_BYTES, &[*id], &[*id]))
}

fn candidates(api: &HidApi, interfaces: &[HidInterface]) -> Vec<(Target, u8)> {
    let matches: Vec<_> = interfaces
        .iter()
        .filter_map(|interface| report_id(api, interface).map(|id| (interface, id)))
        .collect();
    let [(interface, id)] = matches.as_slice() else {
        return Vec::new();
    };
    let mut target = Target::new(BackendId::VeekosShine60, interface, 0);
    target.name = "Veekos Shine 60".into();
    target.family = DeviceFamily::Keyboard;
    target.key = crate::device::stable_key(&format!(
        "veekos-shine60:{}:{}",
        interface.serial.trim(),
        interface.path
    ));
    target.verified_identity = true;
    vec![(target, *id)]
}

fn decode(frame: &[u8], id: u8) -> Option<u8> {
    if frame.len() != usize::from(REPORT_BYTES) { return None; }
    (frame[0] == id
        && frame[1] == 0
        && frame[2] <= 100
        && frame[3] == 100)
        .then_some(frame[2])
}

fn snapshot(target: &Target, level: u8) -> Snapshot {
    Snapshot {
        target: target.clone(),
        battery: Some(Battery {
            percent: Some(level),
            estimated_percent: None,
            charging: None,
            voltage_mv: None,
            approximate: false,
            level: None,
        }),
        dpi: Dpi::default(),
        polling_rate: Default::default(),
        connected: true,
        battery_at: Some(std::time::Instant::now()),
        battery_unavailable: false,
    }
}

fn read_value(api: &HidApi, original: &Target) -> Result<Snapshot, String> {
    let interfaces = crate::hid::enumerate_all_report(api)?.interfaces;
    let (target, id) = candidates(api, &interfaces)
        .into_iter()
        .find(|(target, _)|
            original.hid().is_ok_and(|source| target.hid().is_ok_and(|candidate|
                source.path.eq_ignore_ascii_case(&candidate.path))))
        .ok_or_else(|| "Shine 60 状态通道未确认".to_string())?;
    if target.key != original.key { return Err("Shine 60 身份已变化".into()); }
    let interface = target.hid().map_err(|_| "Shine 60 状态通道无效".to_string())?;
    let frame = crate::hid::query_report(interface, id, true, 800)
        .ok_or_else(|| "Shine 60 无有效 Feature 回复".to_string())?;
    let level = decode(&frame, id).ok_or_else(|| "Shine 60 状态回复无效".to_string())?;
    Ok(snapshot(&target, level))
}

impl DeviceBackend for VeekosShine60 {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(api, interfaces).into_iter().map(|(target, _)| target).collect())
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        if target.backend != BackendId::VeekosShine60 { return Err("Shine 60 目标无效".into()); }
        read_value(api, target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface() -> HidInterface {
        HidInterface {
            path: r"\\?\hid#vid_3151&pid_5026&mi_02&col01#7&fixture&0&0000#{hid}".into(),
            serial: String::new(),
            product: NAME.into(),
            vendor_id: VENDOR,
            product_id: PRODUCT,
            usage_page: 0xffff,
            usage: 2,
            input_len: 0,
            output_len: 0,
            feature_len: REPORT_BYTES,
        }
    }

    fn frame(id: u8, level: u8) -> Vec<u8> {
        let mut frame = vec![0; usize::from(REPORT_BYTES)];
        frame[..4].copy_from_slice(&[id, 0, level, 100]);
        frame
    }

    #[test]
    fn only_the_exact_shine60_feature_collection_is_accepted() {
        let exact = interface();
        assert!(management(&exact));
        for mutate in [
            |i: &mut HidInterface| i.product_id = 0x5029,
            |i: &mut HidInterface| i.product = "Other keyboard".into(),
            |i: &mut HidInterface| i.usage = 1,
            |i: &mut HidInterface| i.path = i.path.replace("mi_02", "mi_01"),
            |i: &mut HidInterface| i.input_len = 65,
            |i: &mut HidInterface| i.feature_len = 64,
        ] {
            let mut wrong = exact.clone();
            mutate(&mut wrong);
            assert!(!management(&wrong));
        }
    }

    #[test]
    fn status_page_requires_id_zero_zero_scale_and_valid_percentage() {
        for id in [0, 0x83] {
            for level in [0, 1, 48, 61, 100] {
                assert_eq!(decode(&frame(id, level), id), Some(level));
            }
            assert!(decode(&frame(id ^ 1, 48), id).is_none());
            let mut wrong = frame(id, 48);
            wrong[1] = 1;
            assert!(decode(&wrong, id).is_none());
            wrong = frame(id, 48);
            wrong[2] = 101;
            assert!(decode(&wrong, id).is_none());
            wrong = frame(id, 48);
            wrong[3] = 99;
            assert!(decode(&wrong, id).is_none());
            assert!(decode(&frame(id, 48)[..64], id).is_none());
        }
    }

    #[test]
    fn every_truncated_or_oversized_status_page_is_rejected_without_panicking() {
        for report_id in [0, 0x83] {
            for length in 0..=usize::from(REPORT_BYTES) + 1 {
                let mut reply = frame(report_id, 48);
                reply.resize(length, 0);
                assert_eq!(decode(&reply, report_id),
                    (length == usize::from(REPORT_BYTES)).then_some(48));
            }
        }
    }

    #[test]
    fn snapshot_never_infers_charging_or_write_capabilities() {
        let mut target = Target::new(BackendId::VeekosShine60, &interface(), 0);
        target.name = "Veekos Shine 60".into();
        target.family = DeviceFamily::Keyboard;
        let sample = snapshot(&target, 48);
        assert_eq!(sample.battery.as_ref().unwrap().charging, None);
        assert!(sample.dpi.current.is_none() && !sample.dpi.writable);
        assert!(!sample.polling_rate.writable);
    }
}
