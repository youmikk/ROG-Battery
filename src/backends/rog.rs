//! ROG 深度适配沿用现有协议，独立于其他品牌的电量与 DPI 能力。
use super::*;

pub struct Rog;

pub fn fetch(
    link: &crate::hid::Link,
    detail: bool,
) -> Result<crate::snapshot::FetchOutcome, String> {
    let mut device = crate::hid::HidDevice::open(&link.iface)?;
    Ok(if detail {
        crate::snapshot::fetch(
            &mut device,
            &link.all_interfaces,
            link.report_id,
            &link.mode,
        )
    } else {
        crate::snapshot::fetch_light(&mut device, link.report_id, &link.mode)
    })
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
            .find(|link| link.device_key() == target.key)
            .ok_or("ROG 设备已断开")?;
        let outcome = fetch(&link, detail)?;
        let state = &outcome.snapshot.state;
        let connected = outcome.snapshot.frames.contains_key("12:07:00");
        let dpi = if outcome.snapshot.frames.contains_key("12:04:02") {
            Dpi {
                current: Some(
                    state.dpi(state.active_stage.saturating_sub(1).min(3) as usize) as u16,
                ),
                values: (0..4).map(|stage| state.dpi(stage) as u16).collect(),
                writable: true,
            }
        } else {
            Dpi::default()
        };
        let mut target = target.clone();
        let name = outcome.snapshot.identity.display_name();
        if !name.trim().is_empty() {
            target.name = name;
            target.family = crate::device::family_of(&target.name);
        }
        Ok(Snapshot {
            target,
            connected,
            dpi,
            battery: connected.then_some(Battery {
                percent: (state.power <= 100).then_some(state.power),
                charging: Some(state.charging != 0),
                voltage_mv: Some(state.voltage_mv),
                approximate: false,
            }),
        })
    }
}
