//! Battery-only XInput polling; system slots are not HID identities or brands.
use super::*;
use crate::win32::{FreeLibrary, GetProcAddress, Handle};
use std::{collections::BTreeSet, sync::OnceLock, time::Instant};
use winapi::um::xinput::{XINPUT_BATTERY_INFORMATION, XINPUT_STATE};

const NOT_CONNECTED: u32 = 1167;
type GetState = unsafe extern "system" fn(u32, *mut XINPUT_STATE) -> u32;
type GetBattery = unsafe extern "system" fn(u32, u8, *mut XINPUT_BATTERY_INFORMATION) -> u32;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(name: *const u16, file: Handle, flags: u32) -> Handle;
}

struct Api { state: GetState, battery: GetBattery }

impl Api {
    fn load() -> Result<&'static Self, String> {
        static API: OnceLock<Result<Api, String>> = OnceLock::new();
        API.get_or_init(|| unsafe {
            // XInput 1.4 ships with every supported Windows version. Load only
            // the system copy, never a game wrapper from the app/current folder.
            let module = LoadLibraryExW(crate::win32::to_wide("xinput1_4.dll").as_ptr(),
                core::ptr::null_mut(), 0x0000_0800);
            if module.is_null() { return Err("XInput 1.4 is unavailable".into()); }
            let state = GetProcAddress(module, b"XInputGetState\0".as_ptr());
            let battery = GetProcAddress(module, b"XInputGetBatteryInformation\0".as_ptr());
            if state.is_null() || battery.is_null() {
                FreeLibrary(module);
                return Err("XInput battery API is unavailable".into());
            }
            // The cached pointers retain their module for the process lifetime.
            Ok(Api { state: core::mem::transmute::<*mut core::ffi::c_void, GetState>(state),
                battery: core::mem::transmute::<*mut core::ffi::c_void, GetBattery>(battery) })
        }).as_ref().map_err(Clone::clone)
    }

    fn sample(&self, slot: u8) -> Sample {
        let mut state: XINPUT_STATE = unsafe { core::mem::zeroed() };
        let result = unsafe { (self.state)(u32::from(slot), &mut state) };
        match result {
            NOT_CONNECTED => return Sample::Disconnected,
            0 => (),
            _ => return Sample::Failed,
        }
        // Do not retain packet numbers, button or axis data.
        let mut battery: XINPUT_BATTERY_INFORMATION = unsafe { core::mem::zeroed() };
        match unsafe { (self.battery)(u32::from(slot), 0, &mut battery) } {
            0 => battery_sample(battery.BatteryType, battery.BatteryLevel),
            NOT_CONNECTED => Sample::Disconnected,
            _ => Sample::Connected { battery: None, clear: false },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Sample {
    Disconnected,
    Failed,
    Connected { battery: Option<Battery>, clear: bool },
}

fn battery_sample(kind: u8, raw: u8) -> Sample {
    match kind {
        0 => return Sample::Disconnected,
        // The API defines the level only for wireless, known battery types.
        1 | 0xff => return Sample::Connected { battery: None, clear: true },
        2 | 3 => (),
        _ => return Sample::Connected { battery: None, clear: false },
    }
    let (estimated, level) = match raw {
        0 => (0, BatteryLevel::Critical), 1 => (20, BatteryLevel::Low),
        2 => (50, BatteryLevel::Medium), 3 => (100, BatteryLevel::Full),
        _ => return Sample::Connected { battery: None, clear: false },
    };
    Sample::Connected { battery: Some(Battery { percent: None, estimated_percent: Some(estimated),
        charging: None, voltage_mv: None, approximate: true, level: Some(level) }), clear: false }
}

pub(crate) fn target(slot: u8) -> Target {
    Target { key: crate::device::stable_key(&format!("xinput-slot:{slot}")),
        name: format!("XInput {} {}", if crate::i18n::english() { "Controller" } else { "手柄" }, slot + 1),
        family: DeviceFamily::Gamepad,
        backend: BackendId::XInput, endpoint: Endpoint::XInput(slot), address: slot,
        verified_identity: false }
}

fn collect(mut sample: impl FnMut(u8) -> Sample, now: Instant) -> Poll {
    let mut present = BTreeSet::new();
    let mut snapshots = Vec::new();
    for slot in 0..4 {
        let target = target(slot);
        match sample(slot) {
            Sample::Disconnected => (),
            Sample::Failed => { present.insert(target.channel_key()); }
            Sample::Connected { battery, clear } => {
                present.insert(target.channel_key());
                snapshots.push(Snapshot { target, connected: true,
                    battery_at: battery.as_ref().map(|_| now), battery_unavailable: clear, battery,
                    dpi: Dpi::default(), polling_rate: Default::default() });
            }
        }
    }
    Poll { snapshots, present_channels: Some(present), ..Default::default() }
}

pub fn poll() -> Result<Poll, String> {
    let api = Api::load()?;
    Ok(collect(|slot| api.sample(slot), Instant::now()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_levels_remain_estimates_without_inventing_charging() {
        for kind in [2, 3] {
            for (raw, expected) in [0, 20, 50, 100].into_iter().enumerate() {
                let Sample::Connected { battery: Some(b), clear } = battery_sample(kind, raw as u8) else { panic!(); };
                assert!(!clear); assert_eq!(b.percent, None); assert_eq!(b.charging, None);
                assert_eq!(b.display_percent().unwrap().label(true), format!("≈{expected}%"));
            }
            assert_eq!(battery_sample(kind, 4), Sample::Connected { battery: None, clear: false });
        }
        for raw in [0, 3, 255] {
            assert_eq!(battery_sample(0, raw), Sample::Disconnected);
            for kind in [1, 0xff] {
                assert_eq!(battery_sample(kind, raw), Sample::Connected { battery: None, clear: true });
            }
            assert_eq!(battery_sample(4, raw), Sample::Connected { battery: None, clear: false });
        }
    }

    #[test]
    fn disconnected_failed_and_connected_slots_have_distinct_presence_evidence() {
        let now = Instant::now();
        let poll = collect(|slot| match slot {
            0 => battery_sample(3, 2), 1 => Sample::Disconnected,
            2 => Sample::Failed, _ => battery_sample(1, 255),
        }, now);
        assert_eq!(poll.snapshots.len(), 2);
        let present = poll.present_channels.unwrap();
        assert_eq!(present, ["xinput:0", "xinput:2", "xinput:3"].into_iter().map(str::to_string).collect());
        assert_ne!(poll.snapshots[0].target.key, poll.snapshots[1].target.key);
        assert!(poll.snapshots.iter().all(|s| s.target.hid().is_err()));
        assert!(poll.snapshots.iter().all(|s| !s.target.verified_identity));
        assert!(poll.snapshots[1].battery.is_none());
        assert_eq!(poll.snapshots[1].battery_at, None);
        assert!(poll.snapshots[1].battery_unavailable);
        assert!(!poll.snapshots[0].target.same_channel(&poll.snapshots[1].target));
    }

    #[test]
    fn wired_mode_clears_previous_wireless_charge_and_failure_does_not_renew_it() {
        let now = Instant::now();
        let lifetime = BackendId::XInput.sample_lifetime();
        let mut state = super::super::status::Status::default();
        for (second, sample, expected) in [(0, battery_sample(3, 2), Some(50)),
            (1, Sample::Connected { battery: None, clear: false }, Some(50)),
            (2, battery_sample(1, 3), None), (3, battery_sample(3, 1), Some(20)),
            (4, battery_sample(0xff, 3), None)] {
            let at = now + std::time::Duration::from_secs(second);
            let poll = collect(|_| sample.clone(), at);
            let reading = &poll.snapshots[0];
            state.observe_sample(Some(true), reading.battery.as_ref(), reading.battery_at, at, lifetime);
            if reading.battery_unavailable { state.clear_battery(); }
            assert_eq!(state.battery.as_ref().and_then(|b| b.estimated_percent), expected);
            if second == 1 { assert_eq!(state.measured_at, Some(now)); }
        }
    }
}
