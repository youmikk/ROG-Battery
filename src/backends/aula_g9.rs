//! AULA G9 passive status86; evidence and limits: docs/PROTOCOLS.md.
use super::*;
use crate::hid::HidDevice;
use std::{sync::TryLockError, time::{Duration, Instant}};

pub struct AulaG9;
pub(super) const SAMPLE_LIFETIME: Duration = Duration::from_secs(10);
const LENGTH: u16 = 64;
const REPORT_ID: u8 = 0xab;
const READ_WINDOW: Duration = Duration::from_millis(1500);
const MAX_FRAMES: usize = 32;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(interface: &HidInterface) -> bool {
    // Upstream establishes FFAB but no usage ID or MI/Col index.
    interface.vendor_id == 0x38a6 && interface.product_id == 0x0030 && interface.usage_page == 0xffab
}

fn control(interface: &HidInterface) -> bool {
    management(interface) && interface.input_len == LENGTH && interface.output_len == LENGTH
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|interface| control(interface)
        && interfaces.iter().filter(|other| management(other)
            && channel::collection_group(interface) == channel::collection_group(other)).count() == 1).collect()
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    control(interface) && descriptor::Descriptor::open(api, interface)
        .is_some_and(|descriptor| descriptor.input_report_ids(interface, LENGTH, &[REPORT_ID]))
}

fn target(interface: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::AulaG9, interface, 0);
    target.name = "AULA G9".into();
    target.family = DeviceFamily::Headset;
    target
}

fn valid_target(original: &Target) -> bool {
    original.backend == BackendId::AulaG9 && original.address == 0
        && original.hid().is_ok_and(|interface| control(interface) && original.key == target(interface).key)
}

fn discover_with(interfaces: &[HidInterface], mut prove: impl FnMut(&HidInterface) -> bool) -> Vec<Target> {
    candidates(interfaces).into_iter().filter(|interface| prove(interface)).map(target).collect()
}

fn battery(frame: &[u8]) -> Option<Battery> {
    if frame.len() != usize::from(LENGTH) || frame[..4] != [REPORT_ID, 0x47, 0x4d, 0x86]
        || frame[6] > 100 { return None; }
    let unavailable = frame[6] == 0 && frame[7] == 0;
    Some(Battery { percent: (!unavailable).then_some(frame[6]), estimated_percent: None,
        approximate: false, charging: (frame[7] == 2).then_some(true), voltage_mv: None, level: None })
}

fn receive_with(mut remaining: impl FnMut() -> Option<Duration>,
    mut read: impl FnMut(u32) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    let mut last_invalid = None;
    for _ in 0..MAX_FRAMES {
        let Some(left) = remaining().filter(|left| !left.is_zero()) else { break; };
        let timeout = left.as_millis().min(READ_WINDOW.as_millis()).max(1) as u32;
        let Some(frame) = read(timeout) else { break; };
        if battery(&frame).is_some() { return Some(frame); }
        last_invalid = Some(frame);
    }
    last_invalid
}

struct Channel { device: HidDevice }

impl Channel {
    fn open(interface: &HidInterface) -> Result<Self, String> {
        let device = HidDevice::open_read_only(interface)?;
        if device.input_len() != LENGTH || device.writable { return Err(UNAVAILABLE.into()); }
        Ok(Self { device })
    }

    fn receive(&mut self) -> Option<Vec<u8>> {
        let deadline = Instant::now() + READ_WINDOW;
        receive_with(|| deadline.checked_duration_since(Instant::now()),
            |timeout| self.device.read_report(timeout))
    }
}

fn read_with(original: &Target, interfaces: &[HidInterface], mut now: impl FnMut() -> Instant,
    mut prove: impl FnMut(&HidInterface) -> bool,
    receive: impl FnOnce(&HidInterface) -> Result<Option<Vec<u8>>, String>) -> Result<Snapshot, String> {
    if !valid_target(original) { return Err(UNAVAILABLE.into()); }
    let original_interface = original.hid()?;
    let interface = candidates(interfaces).into_iter().find(|interface|
        interface.path.eq_ignore_ascii_case(&original_interface.path)
            && interface.serial == original_interface.serial).ok_or(UNAVAILABLE)?;
    let lock = channel::lock(&channel::collection_group(interface));
    let _guard = match lock.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return Err(UNAVAILABLE.into()),
    };
    if !prove(interface) { return Err(UNAVAILABLE.into()); }
    let started = now();
    let response = receive(interface);
    let measured = now();
    let frame = response.as_ref().ok().and_then(|frame| frame.as_deref());
    let sample = frame.and_then(battery).filter(|_| measured.saturating_duration_since(started) < READ_WINDOW);
    trace::record_sample(interface, BackendId::AulaG9, "passive-battery", u16::from(REPORT_ID), 0x86,
        0, frame.map_or(0, <[u8]>::len), started,
        if sample.is_some() { trace::Outcome::Ok } else if frame.is_some() { trace::Outcome::InvalidData }
        else { trace::Outcome::NoReply }, sample.as_ref().map(|_| measured));
    let _ = response?;
    let battery = sample.ok_or(UNAVAILABLE)?;
    if !prove(interface) || now().saturating_duration_since(measured) >= SAMPLE_LIFETIME { return Err(UNAVAILABLE.into()); }
    Ok(Snapshot { target: target(interface), battery_unavailable: battery.percent.is_none() && battery.charging.is_none(),
        battery: Some(battery), battery_at: Some(measured), connected: true,
        dpi: Dpi::default(), polling_rate: Default::default() })
}

impl DeviceBackend for AulaG9 {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(discover_with(interfaces, |interface| proven(api, interface)))
    }

    fn read(&self, api: &HidApi, original: &Target, _: bool) -> Result<Snapshot, String> {
        if !valid_target(original) { return Err(UNAVAILABLE.into()); }
        let interfaces = enumeration::scan(api)?.interfaces;
        read_with(original, &interfaces, Instant::now, |interface| proven(api, interface),
            |interface| Ok(Channel::open(interface)?.receive()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, sync::Arc};

    fn interface(instance: &str) -> HidInterface {
        // Synthetic path/usage: neither MI/Col nor usage ID is established upstream.
        HidInterface { path: format!(r"\\?\hid#vid_38a6&pid_0030&mi_03&col01#7&{instance}&0&0000#{{hid}}"),
            vendor_id: 0x38a6, product_id: 0x0030, usage_page: 0xffab, usage: 1,
            input_len: LENGTH, output_len: LENGTH, feature_len: 0,
            product: "Wireless Headset".into(), serial: "shared".into() }
    }

    // Reconstructed prefix/fields with zero padding; not a complete raw capture.
    fn frame(percent: u8, flag: u8) -> Vec<u8> {
        let mut bytes = vec![0; 64];
        bytes[..4].copy_from_slice(&[0xab, 0x47, 0x4d, 0x86]);
        bytes[6] = percent;
        bytes[7] = flag;
        bytes
    }

    fn sample(interface: &HidInterface, bytes: Vec<u8>) -> Snapshot {
        let now = Instant::now();
        read_with(&target(interface), std::slice::from_ref(interface), || now, |_| true,
            |_| Ok(Some(bytes))).unwrap()
    }

    #[test]
    fn reconstructed_charging_sample_and_unverified_flags_are_separate() {
        let snapshot = sample(&interface("charge"), frame(76, 2));
        let battery = snapshot.battery.unwrap();
        assert_eq!((battery.percent, battery.charging), (Some(76), Some(true)));
        assert!(!battery.approximate);
        assert_eq!((battery.estimated_percent, battery.level, battery.voltage_mv), (None, None, None));
        assert_eq!(snapshot.target.family, DeviceFamily::Headset);
        assert!(!snapshot.target.verified_identity);
        assert!(snapshot.connected && snapshot.battery_at.is_some() && !snapshot.battery_unavailable);
        assert!(!snapshot.dpi.writable && !snapshot.polling_rate.writable);
        for flag in (0..=255).filter(|flag| *flag != 2) {
            assert_eq!(super::battery(&frame(76, flag)).unwrap().charging, None);
        }
    }

    #[test]
    fn firmware_placeholder_clears_battery_without_claiming_zero_or_offline() {
        let snapshot = sample(&interface("placeholder"), frame(0, 0));
        assert!(snapshot.connected && snapshot.battery_unavailable && snapshot.battery_at.is_some());
        let battery = snapshot.battery.unwrap();
        assert_eq!((battery.percent, battery.charging), (None, None));
        let empty_charging = super::battery(&frame(0, 2)).unwrap();
        assert_eq!((empty_charging.percent, empty_charging.charging), (Some(0), Some(true)));
    }

    #[test]
    fn replacement_battery_can_jump_in_either_direction() {
        let interface = interface("replacement");
        for percent in [1, 100, 12, 95, 0, 76] {
            let snapshot = sample(&interface, frame(percent, 2));
            assert_eq!(snapshot.battery.unwrap().percent, Some(percent));
        }
    }

    #[test]
    fn exact_length_header_and_percentage_are_required() {
        for length in 0..64 { assert!(battery(&frame(76, 2)[..length]).is_none()); }
        let mut oversized = frame(76, 2);
        oversized.push(0);
        assert!(battery(&oversized).is_none());
        for index in 0..4 {
            let mut wrong = frame(76, 2);
            wrong[index] ^= 1;
            assert!(battery(&wrong).is_none());
        }
        for percent in 101..=255 { assert!(battery(&frame(percent, 2)).is_none()); }
    }

    #[test]
    fn discovery_needs_descriptor_proof_and_exact_device_capacities() {
        let first = interface("discovery-a");
        let second = interface("discovery-b");
        let targets = discover_with(&[first, second], |_| true);
        assert_eq!(targets.len(), 2);
        assert_ne!(targets[0].key, targets[1].key);
        assert!(discover_with(&[interface("no-proof")], |_| false).is_empty());
        assert!(!super::super::has_configuration(BackendId::AulaG9));
        for change in [
            |i: &mut HidInterface| i.vendor_id = 0x0c45,
            |i: &mut HidInterface| i.product_id = 0x28fd,
            |i: &mut HidInterface| i.usage_page = 0xff00,
            |i: &mut HidInterface| i.input_len = 9,
            |i: &mut HidInterface| i.input_len = 65,
            |i: &mut HidInterface| i.output_len = 0,
        ] {
            let mut wrong = interface("wrong");
            change(&mut wrong);
            assert!(discover_with(&[wrong], |_| panic!("outside scope")).is_empty());
        }
    }

    #[test]
    fn ambiguous_vendor_siblings_are_excluded_and_consumer_siblings_are_ignored() {
        let first = interface("ambiguous");
        let mut sibling = first.clone();
        sibling.path = sibling.path.replace("col01", "col02").replace("&0000#", "&0001#");
        sibling.usage = 2;
        assert!(discover_with(&[first.clone(), sibling.clone()], |_| true).is_empty());
        assert!(discover_with(&[first.clone(), first.clone()], |_| true).is_empty());
        sibling.usage_page = 0x0c;
        sibling.input_len = 3;
        sibling.output_len = 0;
        assert_eq!(discover_with(&[first, sibling], |_| true).len(), 1);
    }

    #[test]
    fn malformed_frames_are_skipped_and_retained_only_for_diagnostics() {
        let mut frames = [Some(vec![0; 64]), Some(frame(76, 2))].into_iter();
        let response = receive_with(|| Some(READ_WINDOW), |timeout| {
            assert_eq!(timeout, 1500);
            frames.next().unwrap()
        }).unwrap();
        assert_eq!(battery(&response).unwrap().percent, Some(76));
        let mut frames = [Some(vec![0; 64]), None].into_iter();
        let response = receive_with(|| Some(READ_WINDOW), |_| frames.next().unwrap()).unwrap();
        assert!(battery(&response).is_none());
        assert!(receive_with(|| None, |_| panic!("expired")).is_none());
    }

    #[test]
    fn passive_read_has_frame_and_time_limits() {
        let mut count = 0;
        assert!(receive_with(|| Some(READ_WINDOW), |_| { count += 1; Some(vec![0; 64]) })
            .as_deref().and_then(battery).is_none());
        assert_eq!(count, MAX_FRAMES);
        let checks = Cell::new(0);
        let mut reads = 0;
        receive_with(|| {
            checks.set(checks.get() + 1);
            (checks.get() == 1).then_some(Duration::from_millis(1))
        }, |timeout| { assert_eq!(timeout, 1); reads += 1; Some(vec![0; 64]) });
        assert_eq!(reads, 1);
        let i = interface("late");
        let now = Instant::now();
        let clock = Cell::new(now);
        assert!(read_with(&target(&i), std::slice::from_ref(&i), || clock.get(), |_| true, |_| {
            clock.set(now + READ_WINDOW);
            Ok(Some(frame(76, 2)))
        }).is_err());
    }

    #[test]
    fn silence_and_errors_cannot_republish_an_old_sample() {
        let i = interface("silence");
        let now = Instant::now();
        for response in [Ok(None), Ok(Some(vec![0; 64])), Err("read failed".into())] {
            assert!(read_with(&target(&i), std::slice::from_ref(&i), || now, |_| true, |_| response).is_err());
        }
        assert_eq!(sample(&i, frame(12, 2)).battery.unwrap().percent, Some(12));
    }

    #[test]
    fn replaced_removed_and_invalid_targets_never_fall_back() {
        let first = interface("removed");
        let second = interface("remaining");
        let now = Instant::now();
        assert!(read_with(&target(&first), std::slice::from_ref(&second), || now, |_| true,
            |_| panic!("wrong receiver")).is_err());
        let mut replacement = first.clone();
        replacement.serial = "new receiver".into();
        assert!(read_with(&target(&first), &[replacement], || now, |_| true,
            |_| panic!("replaced receiver")).is_err());
        for change in [
            |t: &mut Target| t.backend = BackendId::Aula,
            |t: &mut Target| t.address = 1,
            |t: &mut Target| t.key = "wrong-key".into(),
        ] {
            let mut original = target(&first);
            change(&mut original);
            assert!(read_with(&original, std::slice::from_ref(&first), || now, |_| true,
                |_| panic!("invalid target")).is_err());
        }
    }

    #[test]
    fn descriptor_loss_prevents_receiving_or_publishing() {
        let i = interface("descriptor");
        let now = Instant::now();
        assert!(read_with(&target(&i), std::slice::from_ref(&i), || now, |_| false,
            |_| panic!("unproven")).is_err());
        let proofs = Cell::new(0);
        assert!(read_with(&target(&i), std::slice::from_ref(&i), || now, |_| {
            proofs.set(proofs.get() + 1);
            proofs.get() == 1
        }, |_| Ok(Some(frame(76, 2)))).is_err());
        assert_eq!(proofs.get(), 2);
    }

    #[test]
    fn identical_receivers_have_independent_keys_locks_and_samples() {
        let first = interface("isolated-a");
        let second = interface("isolated-b");
        let now = Instant::now();
        let first_lock = channel::lock(&channel::collection_group(&first));
        let second_lock = channel::lock(&channel::collection_group(&second));
        assert!(!Arc::ptr_eq(&first_lock, &second_lock));
        let _guard = first_lock.lock().unwrap();
        let interfaces = [first.clone(), second.clone()];
        assert!(read_with(&target(&first), &interfaces, || now, |_| panic!("busy"), |_| panic!("busy")).is_err());
        let snapshot = read_with(&target(&second), &interfaces, || now, |_| true,
            |_| Ok(Some(frame(80, 2)))).unwrap();
        assert_eq!(snapshot.target.key, target(&second).key);
        assert_ne!(snapshot.target.key, target(&first).key);
        assert_eq!(snapshot.battery.unwrap().percent, Some(80));
    }
}
