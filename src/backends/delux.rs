//! DELUX M900 Pro passive report 03; evidence: VigilWatt c97adc389b60b0e945a62cddacbaaace39887723.
use super::*;
use crate::hid::HidDevice;
use std::{collections::BTreeMap, sync::{Mutex, OnceLock, TryLockError}, time::{Duration, Instant}};

pub struct Delux;

pub(super) const SAMPLE_LIFETIME: Duration = Duration::from_secs(90);
const LENGTH: u16 = 5;
const REPORT_ID: u8 = 3;
const TIMEOUT_MS: u32 = 5_000;
const MAX_FRAMES: usize = 32;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(interface: &HidInterface) -> bool {
    interface.vendor_id == 0x1d57 && interface.product_id == 0xfa65
        && interface.usage_page == 0x000a && interface.usage == 0
}

fn receiver_group(interface: &HidInterface) -> Option<String> {
    if !management(interface) { return None; }
    let path = interface.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid"
        || parts[2].is_empty() || parts[2].contains(['\\', '/'])
        || parts[2].split('&').any(str::is_empty)
        || parts[3] != "{4d1e55b2-f16f-11cf-88cb-001111000030}"
        || path.contains('\0') { return None; }
    let fields: Vec<_> = parts[1].split('&').collect();
    if !(2..=4).contains(&fields.len()) || fields[..2] != ["vid_1d57", "pid_fa65"] {
        return None;
    }
    let hex_pair = |value: &str| value.len() == 2
        && value.bytes().all(|byte| byte.is_ascii_hexdigit());
    let mut next = 2;
    if let Some(number) = fields.get(next).and_then(|field| field.strip_prefix("mi_")) {
        if !hex_pair(number) { return None; }
        next += 1;
    }
    if next == fields.len() { return Some(path); }
    let collection = fields[next].strip_prefix("col")?;
    if next + 1 != fields.len() || !hex_pair(collection) || collection == "00" {
        return None;
    }
    let (instance, suffix) = parts[2].rsplit_once('&')?;
    if instance.split('&').count() < 2 || suffix.len() != 4 || !suffix.starts_with("00")
        || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) { return None; }
    Some(format!("{}#{}#{instance}#{}", parts[0], fields[..next].join("&"), parts[3]))
}

fn control(interface: &HidInterface) -> bool {
    management(interface) && interface.input_len == LENGTH && interface.output_len == 0
        && interface.feature_len == 0 && receiver_group(interface).is_some()
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|interface| control(interface)
        && interfaces.iter().filter(|other| management(other)
            && receiver_group(other) == receiver_group(interface)).count() == 1).collect()
}

fn descriptor_gate(interface: &HidInterface,
    proof: impl FnOnce(u16, &[u8]) -> bool) -> bool {
    control(interface) && proof(LENGTH, &[REPORT_ID])
}

fn proven(api: &HidApi, interface: &HidInterface) -> bool {
    descriptor_gate(interface, |length, ids| descriptor::Descriptor::open(api, interface)
        .is_some_and(|descriptor| descriptor.input_report_ids(interface, length, ids)))
}

fn target(interface: &HidInterface) -> Target {
    let mut target = Target::new(BackendId::Delux, interface, 0);
    target.name = "DELUX M900 Pro".into();
    target.family = DeviceFamily::Mouse;
    target
}

fn valid_target(original: &Target) -> bool {
    original.backend == BackendId::Delux && original.address == 0
        && original.hid().is_ok_and(|interface|
            control(interface) && original.key == target(interface).key)
}

fn discover_with(interfaces: &[HidInterface], mut prove: impl FnMut(&HidInterface) -> bool)
    -> Vec<Target> {
    candidates(interfaces).into_iter().filter(|interface| prove(interface)).map(target).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status { Discharging(u8), Charging, UnconfirmedFull }

fn status(frame: &[u8]) -> Option<Status> {
    if frame.len() != usize::from(LENGTH) || frame[..3] != [REPORT_ID, 0x30, 0x40]
        || frame[4] > 100 { return None; }
    match frame[3] {
        1 => Some(Status::Discharging(frame[4])),
        3 => Some(Status::Charging),
        2 => Some(Status::UnconfirmedFull),
        _ => None,
    }
}

#[derive(Clone, Default)]
struct ChargeFilter { uncertain: bool, settle_until: Option<Instant>, last_seen: Option<Instant> }

impl ChargeFilter {
    fn apply(&mut self, status: Status, now: Instant) -> Battery {
        // A long gap may conceal an entire dock cycle. Require a new settling window.
        if self.last_seen.is_none_or(|last| now.saturating_duration_since(last) >= SAMPLE_LIFETIME) {
            self.settle_until = Some(now + Duration::from_secs(8));
        }
        self.last_seen = Some(now);
        let (percent, charging) = match status {
            Status::Charging => { self.uncertain = true; (None, Some(true)) }
            Status::UnconfirmedFull => { self.uncertain = true; (None, None) }
            Status::Discharging(value) => {
                if self.uncertain {
                    self.uncertain = false;
                    self.settle_until = Some(now + Duration::from_secs(8));
                }
                let settled = self.settle_until.is_none_or(|until| now >= until);
                (settled.then_some(value), Some(false))
            }
        };
        Battery { percent, charging, estimated_percent: None, approximate: false,
            voltage_mv: None, level: None }
    }
}

#[derive(Default)]
struct Filters(BTreeMap<String, (String, ChargeFilter)>);

impl Filters {
    fn for_interface(&mut self, interface: &HidInterface) -> &mut ChargeFilter {
        let entry = self.0.entry(interface.path.to_ascii_lowercase())
            .or_insert_with(|| (interface.serial.clone(), Default::default()));
        if entry.0 != interface.serial { *entry = (interface.serial.clone(), Default::default()); }
        &mut entry.1
    }
    fn retain_present(&mut self, interfaces: &[HidInterface]) {
        self.0.retain(|path, (serial, _)| candidates(interfaces).iter().any(|interface|
            interface.path.eq_ignore_ascii_case(path) && &interface.serial == serial));
    }
}

fn filters() -> &'static Mutex<Filters> {
    static FILTERS: OnceLock<Mutex<Filters>> = OnceLock::new();
    FILTERS.get_or_init(Default::default)
}

fn receive_with(mut remaining: impl FnMut() -> Option<Duration>,
    mut read: impl FnMut(u32) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    let mut last_invalid = None;
    for _ in 0..MAX_FRAMES {
        let Some(left) = remaining().filter(|left| !left.is_zero()) else { break; };
        let timeout = left.as_millis().min(u128::from(TIMEOUT_MS)).max(1) as u32;
        let Some(frame) = read(timeout) else { break; };
        if remaining().is_none_or(|left| left.is_zero()) { return None; }
        if status(&frame).is_some() { return Some(frame); }
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
        let deadline = Instant::now() + Duration::from_millis(u64::from(TIMEOUT_MS));
        receive_with(|| deadline.checked_duration_since(Instant::now()),
            |timeout| self.device.read_report(timeout))
    }
}

fn read_with(original: &Target, interfaces: &[HidInterface], filter: &mut ChargeFilter,
    mut prove: impl FnMut(&HidInterface) -> bool,
    receive: impl FnOnce(&HidInterface) -> Result<Option<Vec<u8>>, String>,
    mut now: impl FnMut() -> Instant) -> Result<Snapshot, String> {
    if !valid_target(original) { return Err(UNAVAILABLE.into()); }
    let original_interface = original.hid()?;
    let interface = candidates(interfaces).into_iter().find(|candidate|
        candidate.path.eq_ignore_ascii_case(&original_interface.path)
            && candidate.serial == original_interface.serial).ok_or(UNAVAILABLE)?;
    let lock = channel::lock(&receiver_group(interface).ok_or(UNAVAILABLE)?);
    let _guard = match lock.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return Err(UNAVAILABLE.into()),
    };
    if !prove(interface) { return Err(UNAVAILABLE.into()); }
    let started = now();
    let response = receive(interface);
    let received = now();
    let frame = response.as_ref().ok().and_then(|frame| frame.as_deref());
    let sample = frame.and_then(status);
    trace::record_sample(interface, BackendId::Delux, "passive-battery", u16::from(REPORT_ID), 0,
        0, frame.map_or(0, <[u8]>::len), started,
        match (frame, sample) {
            (_, Some(_)) => trace::Outcome::Ok,
            (Some(_), None) => trace::Outcome::InvalidData,
            (None, None) => trace::Outcome::NoReply,
        }, sample.map(|_| received));
    let _ = response?;
    let sample = sample.ok_or(UNAVAILABLE)?;
    if !prove(interface) { return Err(UNAVAILABLE.into()); }
    let battery = filter.apply(sample, received);
    Ok(Snapshot { target: target(interface), battery_at: Some(received), connected: true,
        battery_unavailable: battery.percent.is_none() && battery.charging.is_none(),
        battery: Some(battery), dpi: Dpi::default(), polling_rate: Default::default() })
}

impl DeviceBackend for Delux {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        filters().lock().unwrap_or_else(|error| error.into_inner()).retain_present(interfaces);
        Ok(discover_with(interfaces, |interface| proven(api, interface)))
    }
    fn read(&self, api: &HidApi, original: &Target, _: bool) -> Result<Snapshot, String> {
        if !valid_target(original) { return Err(UNAVAILABLE.into()); }
        let interfaces = enumeration::scan(api)?.interfaces;
        let mut states = filters().lock().unwrap_or_else(|error| error.into_inner());
        states.retain_present(&interfaces);
        let filter = states.for_interface(original.hid()?);
        let result = read_with(original, &interfaces, filter, |interface| proven(api, interface),
            |interface| Ok(Channel::open(interface)?.receive()), Instant::now);
        // Never reuse a dock transition across a failed read or replaced interface.
        if result.is_err() { *filter = ChargeFilter::default(); }
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const DISCHARGING: [u8; 5] = [3, 0x30, 0x40, 1, 89];
    const CHARGING: [u8; 5] = [3, 0x30, 0x40, 3, 89];

    // Only the two status vectors above are recorded upstream; paths are synthetic.
    fn interface(instance: &str) -> HidInterface {
        HidInterface {
            path: format!(r"\\?\hid#vid_1d57&pid_fa65&mi_01&col03#7&{instance}&0&0002#{{4d1e55b2-f16f-11cf-88cb-001111000030}}"),
            vendor_id: 0x1d57, product_id: 0xfa65, usage_page: 0x000a, usage: 0,
            input_len: LENGTH, output_len: 0, feature_len: 0,
            product: "2.4G Wireless Receiver".into(), serial: "shared serial".into(),
        }
    }

    fn read_frame(interface: &HidInterface, filter: &mut ChargeFilter, now: Instant,
        frame: &[u8]) -> Snapshot {
        read_with(&target(interface), std::slice::from_ref(interface), filter, |_| true,
            |_| Ok(Some(frame.to_vec())), || now).unwrap()
    }

    #[test]
    fn recorded_vectors_identify_discharging_and_charging_without_trusting_dock_percentage() {
        assert_eq!(status(&DISCHARGING), Some(Status::Discharging(89)));
        assert_eq!(status(&CHARGING), Some(Status::Charging));
        let now = Instant::now();
        let mut filter = ChargeFilter::default();
        let first = read_frame(&interface("recorded"), &mut filter, now, &DISCHARGING);
        assert_eq!(first.battery.unwrap().percent, None);
        let snapshot = read_frame(&interface("recorded"), &mut filter, now + Duration::from_secs(8), &DISCHARGING);
        let battery = snapshot.battery.unwrap();
        assert_eq!((battery.percent, battery.charging), (Some(89), Some(false)));
        assert_eq!((battery.estimated_percent, battery.voltage_mv, battery.level), (None, None, None));
        assert!(!battery.approximate && snapshot.connected && !snapshot.battery_unavailable);
        assert!(!snapshot.target.verified_identity && !snapshot.dpi.writable && !snapshot.polling_rate.writable);
        let charging = read_frame(&interface("recorded"), &mut filter, now + Duration::from_secs(10), &CHARGING);
        assert_eq!((charging.battery.as_ref().unwrap().percent, charging.battery.unwrap().charging), (None, Some(true)));
        assert_eq!(charging.battery_at, Some(now + Duration::from_secs(10)));
    }

    #[test]
    fn exact_report_boundaries_and_other_brand_headers_are_rejected() {
        for n in 0..DISCHARGING.len() { assert_eq!(status(&DISCHARGING[..n]), None); }
        let mut extra = DISCHARGING.to_vec(); extra.push(0);
        assert_eq!(status(&extra), None);
        for offset in 0..3 { let mut frame = DISCHARGING; frame[offset] ^= 1; assert_eq!(status(&frame), None); }
        assert_eq!(status(&[3, 0x76, 0x41, 1, 85]), None);
        for value in 101..=255 { assert_eq!(status(&[3, 0x30, 0x40, 1, value]), None); }
        for flag in 0..=255 { if !matches!(flag, 1..=3) { assert_eq!(status(&[3, 0x30, 0x40, flag, 89]), None); } }
        for value in [0, 1, 99, 100] { assert_eq!(status(&[3, 0x30, 0x40, 1, value]), Some(Status::Discharging(value))); }
    }

    #[test]
    fn charging_from_start_and_unrecorded_full_flag_never_create_a_percentage() {
        let now = Instant::now();
        for value in [0, 89, 100] {
            let mut filter = ChargeFilter::default();
            let sample = filter.apply(status(&[3, 0x30, 0x40, 3, value]).unwrap(), now);
            assert_eq!((sample.percent, sample.charging), (None, Some(true)));
        }
        let mut filter = ChargeFilter::default();
        let snapshot = read_frame(&interface("full"), &mut filter, now, &[3, 0x30, 0x40, 2, 100]);
        assert!(snapshot.battery_unavailable);
        assert_eq!((snapshot.battery.as_ref().unwrap().percent, snapshot.battery.unwrap().charging), (None, None));
    }

    #[test]
    fn dock_exit_requires_eight_seconds_and_a_new_frame_without_reusing_old_numbers() {
        let now = Instant::now();
        let mut filter = ChargeFilter::default();
        filter.apply(Status::Discharging(89), now);
        assert_eq!(filter.apply(Status::Discharging(89), now + Duration::from_secs(8)).percent, Some(89));
        assert_eq!(filter.apply(Status::Charging, now + Duration::from_secs(10)).percent, None);
        for t in [12, 15, 19] {
            assert_eq!(filter.apply(Status::Discharging(100), now + Duration::from_secs(t)).percent, None);
        }
        assert_eq!(filter.apply(Status::Discharging(88), now + Duration::from_secs(20)).percent, Some(88));
        filter.apply(Status::UnconfirmedFull, now + Duration::from_secs(22));
        assert_eq!(filter.apply(Status::Discharging(100), now + Duration::from_secs(23)).percent, None);
        assert_eq!(filter.apply(Status::Discharging(87), now + Duration::from_secs(31)).percent, Some(87));
    }

    #[test]
    fn stale_state_requires_settling_again() {
        let now = Instant::now(); let mut filter = ChargeFilter::default();
        filter.apply(Status::Discharging(89), now);
        filter.apply(Status::Discharging(89), now + Duration::from_secs(8));
        assert_eq!(filter.apply(Status::Discharging(100), now + Duration::from_secs(98)).percent, None);
        assert_eq!(filter.apply(Status::Discharging(87), now + Duration::from_secs(106)).percent, Some(87));
    }

    #[test]
    fn discovery_checks_usage_lengths_descriptor_and_receiver_ambiguity() {
        let first = interface("first"); let second = interface("second");
        assert!(control(&first));
        assert_eq!(discover_with(&[first.clone(), second.clone()], |_| true).len(), 2);
        assert!(discover_with(std::slice::from_ref(&first), |_| false).is_empty());
        let mut sibling = first.clone();
        sibling.path = sibling.path.replace("col03", "col04").replace("&0002#", "&0003#");
        assert_eq!(receiver_group(&first), receiver_group(&sibling));
        assert!(discover_with(&[first.clone(), sibling], |_| true).is_empty());
        for field in 0..7 {
            let mut wrong = first.clone();
            match field {
                0 => wrong.vendor_id ^= 1, 1 => wrong.product_id ^= 1,
                2 => wrong.usage_page = 0xff00, 3 => wrong.usage = 1,
                4 => wrong.input_len = 6, 5 => wrong.output_len = 5,
                _ => wrong.feature_len = 5,
            }
            let proof_called = Cell::new(false);
            assert!(!descriptor_gate(&wrong, |_, _| { proof_called.set(true); true }));
            assert!(!proof_called.get());
        }
        assert!(descriptor_gate(&first, |length, ids| length == 5 && ids == [3]));
        let mut malformed = first.clone(); malformed.path = "not a HID path".into();
        assert!(!control(&malformed));
    }

    #[test]
    fn inphic_collection_is_not_claimed_and_does_not_block_delux() {
        let delux = interface("shared"); let mut inphic = delux.clone();
        inphic.usage_page = 0xff00; inphic.usage = 1;
        inphic.path = inphic.path.replace("col03", "col04").replace("&0002#", "&0003#");
        let found = discover_with(&[delux.clone(), inphic], |_| true);
        assert_eq!(found.len(), 1); assert_eq!(found[0].key, target(&delux).key);
    }

    #[test]
    fn receiver_filters_are_isolated_and_reset_on_unplug_or_serial_change() {
        let now = Instant::now(); let first = interface("first"); let second = interface("second");
        let mut filters = Filters::default();
        filters.for_interface(&first).apply(Status::Charging, now);
        filters.for_interface(&second).apply(Status::Discharging(65), now);
        assert_eq!(filters.for_interface(&second).apply(Status::Discharging(65), now + Duration::from_secs(8)).percent, Some(65));
        assert_eq!(filters.for_interface(&first).apply(Status::Discharging(100), now + Duration::from_secs(8)).percent, None);
        let mut upper = second.clone(); upper.path = upper.path.to_ascii_uppercase();
        assert_eq!(filters.for_interface(&upper).apply(Status::Discharging(64), now + Duration::from_secs(9)).percent, Some(64));
        filters.retain_present(std::slice::from_ref(&first)); assert_eq!(filters.0.len(), 1);
        assert_eq!(filters.for_interface(&second).apply(Status::Discharging(64), now + Duration::from_secs(10)).percent, None);
        let mut replaced = first.clone(); replaced.serial = "replacement".into();
        assert!(filters.for_interface(&replaced).last_seen.is_none());
        filters.retain_present(&[]); assert!(filters.0.is_empty());
    }

    #[test]
    fn silence_invalid_frames_and_transport_errors_do_not_refresh_or_change_filter() {
        let interface = interface("errors"); let now = Instant::now();
        let mut filter = ChargeFilter::default(); filter.apply(Status::Charging, now);
        for response in [Ok(None), Ok(Some(vec![3, 0x76, 0x41, 1, 85])), Err("disconnected".into())] {
            assert!(read_with(&target(&interface), std::slice::from_ref(&interface), &mut filter,
                |_| true, |_| response, || now + Duration::from_secs(1)).is_err());
            assert_eq!(filter.last_seen, Some(now)); assert!(filter.uncertain);
        }
    }

    #[test]
    fn invalid_target_disconnect_or_changed_descriptor_cannot_read_or_publish() {
        let interface = interface("guard"); let now = Instant::now(); let mut filter = ChargeFilter::default();
        let mut wrong = target(&interface); wrong.backend = BackendId::Inphic;
        assert!(read_with(&wrong, std::slice::from_ref(&interface), &mut filter, |_| panic!("proof"), |_| panic!("read"), || now).is_err());
        assert!(read_with(&target(&interface), &[], &mut filter, |_| panic!("proof"), |_| panic!("read"), || now).is_err());
        let mut changed = interface.clone(); changed.serial = "new".into();
        assert!(read_with(&target(&interface), &[changed], &mut filter, |_| panic!("proof"), |_| panic!("read"), || now).is_err());
        assert!(read_with(&target(&interface), std::slice::from_ref(&interface), &mut filter, |_| false, |_| panic!("read"), || now).is_err());
        let calls = Cell::new(0);
        assert!(read_with(&target(&interface), std::slice::from_ref(&interface), &mut filter,
            |_| { calls.set(calls.get()+1); calls.get()==1 }, |_| Ok(Some(DISCHARGING.to_vec())), || now).is_err());
        assert!(filter.last_seen.is_none());
    }

    #[test]
    fn bounded_passive_reads_ignore_other_frames_and_do_not_accept_late_reports() {
        let reads = Cell::new(0);
        let frame = receive_with(|| Some(Duration::from_millis(30)), |timeout| {
            assert!(timeout <= 30); reads.set(reads.get()+1);
            Some(if reads.get()==1 { vec![3, 0x76, 0x41, 1, 85] } else { DISCHARGING.to_vec() })
        });
        assert_eq!(frame, Some(DISCHARGING.to_vec())); assert_eq!(reads.get(), 2);
        let reads = Cell::new(0);
        receive_with(|| Some(Duration::from_secs(5)), |_| { reads.set(reads.get()+1); Some(vec![0; 5]) });
        assert_eq!(reads.get(), MAX_FRAMES);
        let calls = Cell::new(0);
        assert!(receive_with(|| {calls.set(calls.get()+1); (calls.get()==1).then_some(Duration::from_millis(1))}, |_| Some(DISCHARGING.to_vec())).is_none());
        assert!(receive_with(|| None, |_| panic!("read after deadline")).is_none());
        assert!(receive_with(|| Some(Duration::from_secs(1)), |_| None).is_none());
    }
}