//! MCHOSE R9 receiver battery using M HUB's MC report protocol.
//! The HID interface number is resolved from the live USB tree, not audio IDs.
use super::*;
use crate::{hid::HidDevice, win32::to_wide};
use std::{collections::BTreeMap, time::Instant};
use winapi::um::cfgmgr32::{CM_Get_Device_IDW, CM_Get_Parent, CM_Locate_DevNodeW};

pub struct MchoseR9;

const PRODUCT: &str = "MCHOSE R9";
const REPORT: u8 = 0x55;
const LENGTH: u16 = 64;
const COMMAND: u8 = 0x0b;
const TIMEOUT_MS: u32 = 1_000;
const MAX_READS: usize = 32;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x3837 && i.product_id == 0x6013
        && i.usage_page == 0xff07 && i.usage == 0x0222
}

fn device_instance(i: &HidInterface) -> Option<String> {
    if !management(i) { return None; }
    let path = i.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid" || parts[2].is_empty()
        || parts[3].is_empty() || path.contains('\0')
        || parts[1].contains('\\') || parts[2].contains('\\') {
        return None;
    }
    let fields: Vec<_> = parts[1].split('&').collect();
    for (prefix, expected) in [("vid_", "vid_3837"), ("pid_", "pid_6013")] {
        if fields.iter().filter(|field| field.starts_with(prefix)).count() != 1
            || !fields.contains(&expected) { return None; }
    }
    for prefix in ["mi_", "col"] {
        let indices: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix(prefix)).collect();
        if indices.len() > 1 || indices.iter().any(|index|
            index.len() != 2 || !index.bytes().all(|byte| byte.is_ascii_hexdigit())) {
            return None;
        }
    }
    Some(format!("hid\\{}\\{}", parts[1], parts[2]))
}

fn control(i: &HidInterface) -> bool {
    management(i) && i.product.trim().eq_ignore_ascii_case(PRODUCT)
        && i.input_len == LENGTH && i.output_len == LENGTH && device_instance(i).is_some()
}

fn physical_parent_name(name: &str) -> bool {
    let parts: Vec<_> = name.split('\\').collect();
    if parts.len() != 3 || parts[0] != "usb" || parts[2].is_empty() || name.contains('\0') {
        return false;
    }
    let fields: Vec<_> = parts[1].split('&').collect();
    fields.len() == 2 && fields.contains(&"vid_3837") && fields.contains(&"pid_6013")
}

fn usb_parent(i: &HidInterface) -> Option<String> {
    let mut instance = to_wide(&device_instance(i)?);
    let mut node = 0;
    if unsafe { CM_Locate_DevNodeW(&mut node, instance.as_mut_ptr(), 0) } != 0 { return None; }
    for _ in 0..8 {
        let mut parent = 0;
        if unsafe { CM_Get_Parent(&mut parent, node, 0) } != 0 { return None; }
        let mut buffer = [0u16; 512];
        if unsafe { CM_Get_Device_IDW(parent, buffer.as_mut_ptr(), buffer.len() as u32, 0) } != 0 {
            return None;
        }
        let length = buffer.iter().position(|c| *c == 0)?;
        let name = String::from_utf16(&buffer[..length]).ok()?.to_ascii_lowercase();
        if physical_parent_name(&name) { return Some(name); }
        node = parent;
    }
    None
}

fn proven(api: &HidApi, i: &HidInterface) -> bool {
    control(i) && descriptor::Descriptor::open(api, i)
        .is_some_and(|d| d.input_output_byte_reports(i, REPORT, LENGTH, LENGTH))
}

fn select_with<'a>(interfaces: &'a [HidInterface],
    mut parent: impl FnMut(&HidInterface) -> Option<String>,
    mut proof: impl FnMut(&HidInterface) -> bool) -> Vec<(&'a HidInterface, String)> {
    let mut groups: BTreeMap<String, Vec<&HidInterface>> = BTreeMap::new();
    for i in interfaces.iter().filter(|i| management(i)) {
        // An unresolved sibling could belong to any receiver. It cannot be
        // discarded merely because its product string or report sizes differ.
        let Some(key) = parent(i).filter(|key| physical_parent_name(key)) else { return Vec::new(); };
        groups.entry(key).or_default().push(i);
    }
    let mut selected = Vec::new();
    for (parent, members) in groups {
        let [i] = members.as_slice() else { continue; };
        if control(i) && proof(i) { selected.push((*i, parent)); }
    }
    selected
}

fn unreadable_receiver(skipped: &[crate::hid::Skipped]) -> bool {
    skipped.iter().any(|entry| {
        let path = entry.path.to_ascii_lowercase();
        // A descriptor that could not be read may conceal another management
        // collection. Be conservative even if the rest of its path is invalid.
        path.contains("vid_3837") && path.contains("pid_6013")
    })
}

fn target(i: &HidInterface, parent: &str) -> Target {
    let mut target = Target::new(BackendId::MchoseR9, i, 0);
    target.key = crate::device::stable_key(&format!("mchose-r9:{parent}"));
    target.name = PRODUCT.into();
    target.family = DeviceFamily::Headset;
    target
}

fn request() -> [u8; LENGTH as usize] {
    let mut frame = [0; LENGTH as usize];
    // MC GET type 0001; length one contains only subcommand 0B, with no body.
    frame[..9].copy_from_slice(&[REPORT, 0xaa, 1, 0, 1, 0, 1, COMMAND, 0]);
    frame
}

#[derive(Debug, PartialEq, Eq)]
enum Reply {
    Battery { percent: u8, charging: bool },
    Sleeping,
}

fn reply(frame: &[u8]) -> Option<Reply> {
    if frame.len() != usize::from(LENGTH) || frame[..3] != [REPORT, 0xaa, 1]
        || frame[3] > 1 || frame[4] < 3 { return None; }
    // The declared length includes the subcommand and body. Accept the GET
    // response and the separately identified MC battery notification only.
    let checksum_at = 7 + usize::from(frame[4]);
    if checksum_at >= frame.len()
        || !matches!(&frame[5..8], [0, 1, COMMAND] | [0, 4, 0x22]) { return None; }
    let checksum = if frame[3] == 1 {
        frame[3..checksum_at].iter().fold(0, |value, byte| value ^ byte)
    } else { 0 };
    if frame[checksum_at] != checksum { return None; }
    if frame[8] == 0 { return Some(Reply::Sleeping); }
    let percent = frame[9] & 0x7f;
    (percent <= 100).then_some(Reply::Battery { percent, charging: frame[9] & 0x80 != 0 })
}

trait Transport {
    fn pending(&mut self) -> Option<Vec<u8>>;
    fn query(&mut self, request: &[u8], timeout_ms: u32,
        accept: &mut dyn FnMut(&[u8]) -> bool) -> Option<Vec<u8>>;
}

impl Transport for HidDevice {
    fn pending(&mut self) -> Option<Vec<u8>> { self.read_report(0) }

    fn query(&mut self, packet: &[u8], timeout_ms: u32,
        accept: &mut dyn FnMut(&[u8]) -> bool) -> Option<Vec<u8>> {
        if packet != request() || timeout_ms != TIMEOUT_MS { return None; }
        self.transact_with_input_report(packet, REPORT, timeout_ms, accept)
    }
}

fn read_with(t: &Target, transport: &mut impl Transport) -> Result<Snapshot, String> {
    let i = t.hid()?;
    if t.backend != BackendId::MchoseR9 || !control(i) { return Err(UNAVAILABLE.into()); }
    // Do not turn a queued notification from before this query into a fresh
    // sample. A continuously busy queue is left alone rather than retried.
    let mut drained = false;
    for _ in 0..8 {
        if transport.pending().is_none() { drained = true; break; }
    }
    if !drained { return Err(UNAVAILABLE.into()); }

    let started = Instant::now();
    let mut remaining = MAX_READS;
    let response = transport.query(&request(), TIMEOUT_MS, &mut |frame| {
        remaining = remaining.saturating_sub(1);
        reply(frame).is_some() || remaining == 0
    });
    let state = response.as_deref().and_then(reply);
    trace::record(i, BackendId::MchoseR9, "query-battery", COMMAND.into(), REPORT,
        usize::from(LENGTH), response.as_ref().map_or(0, Vec::len), started,
        if state.is_some() { trace::Outcome::Ok }
        else if response.is_some() { trace::Outcome::InvalidData }
        else { trace::Outcome::NoReply });
    let state = state.ok_or(UNAVAILABLE)?;
    let sleeping = state == Reply::Sleeping;
    let battery = match state {
        Reply::Battery { percent, charging } => Some(Battery {
            percent: Some(percent), charging: Some(charging), estimated_percent: None,
            voltage_mv: None, approximate: false, level: None,
        }),
        Reply::Sleeping => None,
    };
    let mut target = t.clone();
    target.verified_identity = true;
    Ok(Snapshot { target, battery_at: battery.as_ref().map(|_| Instant::now()), battery,
        battery_unavailable: sleeping, connected: !sleeping, dpi: Dpi::default(),
        polling_rate: Default::default() })
}

impl DeviceBackend for MchoseR9 {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(select_with(interfaces, usb_parent, |i| proven(api, i)).into_iter()
            .map(|(i, parent)| target(i, &parent)).collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        let original = t.hid()?;
        if t.backend != BackendId::MchoseR9 || !control(original) { return Err(UNAVAILABLE.into()); }
        let original_parent = usb_parent(original).ok_or(UNAVAILABLE)?;
        let lock = channel::lock(&original_parent);
        let _guard = lock.lock().unwrap_or_else(|poison| poison.into_inner());

        // Re-enumerate while holding the physical receiver lock. Neither an
        // old descriptor nor a missing sibling is enough to authorize a query.
        let enumeration = crate::hid::enumerate_all_report(api)?;
        if unreadable_receiver(&enumeration.skipped) { return Err(UNAVAILABLE.into()); }
        let selected = select_with(&enumeration.interfaces, usb_parent, |i| proven(api, i));
        let (i, parent) = selected.into_iter().find(|(i, parent)|
            parent == &original_parent && i.path.eq_ignore_ascii_case(&original.path))
            .ok_or(UNAVAILABLE)?;
        let current = target(i, &parent);
        if current.key != t.key { return Err(UNAVAILABLE.into()); }
        read_with(&current, &mut HidDevice::open(i)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn interface(instance: &str, mi: &str) -> HidInterface {
        // Synthetic Windows paths exercise topology; no MI number is claimed
        // to be a captured R9 hardware interface.
        HidInterface {
            path: format!(r"\\?\hid#vid_3837&pid_6013&mi_{mi}&col04#8&{instance}&0&0003#{{hid}}"),
            vendor_id: 0x3837, product_id: 0x6013, usage_page: 0xff07, usage: 0x0222,
            input_len: LENGTH, output_len: LENGTH, feature_len: 0,
            product: PRODUCT.into(), serial: "shared".into(),
        }
    }

    fn parent(instance: &str) -> String { format!(r"usb\vid_3837&pid_6013\{instance}") }

    fn frame(percent: u8, charging: bool, awake: bool, checked: bool, notify: bool) -> Vec<u8> {
        // Protocol-derived fixtures, not R9 captures or hardware validation.
        let mut bytes = vec![0; usize::from(LENGTH)];
        bytes[..10].copy_from_slice(&[REPORT, 0xaa, 1, u8::from(checked), 3, 0,
            if notify { 4 } else { 1 }, if notify { 0x22 } else { COMMAND },
            u8::from(awake), percent | if charging { 0x80 } else { 0 }]);
        if checked { bytes[10] = bytes[3..10].iter().fold(0, |xor, value| xor ^ value); }
        bytes
    }

    #[derive(Default)]
    struct Fake {
        pending: VecDeque<Vec<u8>>,
        replies: VecDeque<Vec<u8>>,
        sent: Vec<Vec<u8>>,
        received: usize,
    }

    impl Fake {
        fn response(reply: Vec<u8>) -> Self {
            Self { replies: VecDeque::from([reply]), ..Default::default() }
        }
    }

    impl Transport for Fake {
        fn pending(&mut self) -> Option<Vec<u8>> { self.pending.pop_front() }

        fn query(&mut self, packet: &[u8], timeout_ms: u32,
            accept: &mut dyn FnMut(&[u8]) -> bool) -> Option<Vec<u8>> {
            assert_eq!(timeout_ms, TIMEOUT_MS);
            self.sent.push(packet.to_vec());
            while let Some(reply) = self.replies.pop_front() {
                self.received += 1;
                if accept(&reply) { return Some(reply); }
            }
            None
        }
    }

    #[test]
    fn exact_read_request_preserves_precise_battery_and_exposes_no_settings() {
        assert_eq!(&request()[..9], &[0x55, 0xaa, 1, 0, 1, 0, 1, 0x0b, 0]);
        assert!(request()[9..].iter().all(|byte| *byte == 0));
        let t = target(&interface("one", "04"), &parent("one"));
        let mut transport = Fake::response(frame(62, true, true, true, false));
        let sample = read_with(&t, &mut transport).unwrap();
        assert_eq!(transport.sent, vec![request().to_vec()]);
        let battery = sample.battery.as_ref().unwrap();
        assert_eq!((battery.percent, battery.charging), (Some(62), Some(true)));
        assert!(!battery.approximate && battery.voltage_mv.is_none());
        assert!(sample.connected && sample.battery_at.is_some() && !sample.battery_unavailable);
        assert!(!sample.dpi.writable && sample.dpi.stages.is_empty() && !sample.polling_rate.writable);
        assert_eq!(sample.target.key, t.key);
    }

    #[test]
    fn get_and_notification_validate_lengths_types_checksums_and_percentages() {
        for notify in [false, true] {
            for checked in [false, true] {
                for charging in [false, true] {
                    for percent in 0..=100 {
                        assert_eq!(reply(&frame(percent, charging, true, checked, notify)),
                            Some(Reply::Battery { percent, charging }));
                    }
                    for percent in 101..=127 {
                        assert!(reply(&frame(percent, charging, true, checked, notify)).is_none());
                    }
                }
            }
        }
        assert!(reply(&request()).is_none());
        let valid = frame(40, false, true, true, false);
        assert_eq!(&valid[..11], &[0x55, 0xaa, 1, 1, 3, 0, 1, 0x0b, 1, 40, 0x21]);
        for length in 0..64 { assert!(reply(&valid[..length]).is_none()); }
        let mut long = valid.clone(); long.push(0); assert!(reply(&long).is_none());
        for (offset, value) in [(0, 0), (1, 0xab), (2, 2), (3, 2), (4, 2), (4, 57),
            (5, 1), (6, 2), (6, 4), (7, 0x22), (7, 0x1a), (10, 0)] {
            let mut wrong = valid.clone(); wrong[offset] = value;
            assert!(reply(&wrong).is_none());
        }
        let mut unchecked = frame(40, false, true, false, false);
        unchecked[10] = 1;
        assert!(reply(&unchecked).is_none());
        let mut wrong_command = frame(0, false, false, true, false);
        wrong_command[7] = 0x0c;
        wrong_command[10] = wrong_command[3..10].iter().fold(0, |xor, byte| xor ^ byte);
        assert!(reply(&wrong_command).is_none());
    }

    #[test]
    fn sleep_clears_the_sample_and_later_status_restores_it_without_settings() {
        let t = target(&interface("one", "02"), &parent("one"));
        let mut sleep = Fake::response(frame(127, true, false, true, true));
        let sample = read_with(&t, &mut sleep).unwrap();
        assert!(!sample.connected && sample.battery_unavailable);
        assert!(sample.battery.is_none() && sample.battery_at.is_none());
        let mut awake = Fake::response(frame(39, false, true, false, false));
        let sample = read_with(&t, &mut awake).unwrap();
        assert!(sample.connected && !sample.battery_unavailable && sample.battery_at.is_some());
        assert_eq!(sample.battery.unwrap().percent, Some(39));
        assert!(read_with(&t, &mut Fake::default()).is_err());
    }

    #[test]
    fn queries_drain_old_status_skip_unrelated_reports_and_never_retry() {
        let t = target(&interface("one", "00"), &parent("one"));
        let mut fresh = Fake::response(frame(42, false, true, true, false));
        fresh.pending.push_back(frame(98, true, true, false, true));
        fresh.replies.push_front(request().to_vec());
        fresh.replies.push_front(vec![0; 64]);
        let sample = read_with(&t, &mut fresh).unwrap();
        assert_eq!(sample.battery.unwrap().percent, Some(42));
        assert_eq!((fresh.sent.len(), fresh.received), (1, 3));

        let mut busy = Fake { pending: VecDeque::from(vec![vec![0; 64]; 8]), ..Default::default() };
        assert!(read_with(&t, &mut busy).is_err());
        assert!(busy.sent.is_empty());
        let mut invalid = Fake { replies: VecDeque::from(vec![vec![0; 64]; 40]), ..Default::default() };
        assert!(read_with(&t, &mut invalid).is_err());
        assert_eq!((invalid.sent.len(), invalid.received), (1, MAX_READS));
        let mut silent = Fake::default();
        assert!(read_with(&t, &mut silent).is_err());
        assert_eq!(silent.sent.len(), 1);
    }

    #[test]
    fn identity_requires_exact_r9_name_usage_capacity_and_well_formed_hid_path() {
        let i = interface("one", "04");
        assert!(control(&i));
        let mut normalized = i.clone(); normalized.product = "  mchose r9  ".into();
        assert!(control(&normalized));
        for (pid, name, page, usage, input, output) in [
            (0x6011, "MCHOSE V9 Pro 2", 0xff07, 0x0222, 64, 64),
            (0x6013, "MCHOSE R9 Pro", 0xff07, 0x0222, 64, 64),
            (0x6013, "USB Receiver", 0xff07, 0x0222, 64, 64),
            (0x6013, PRODUCT, 0xff90, 1, 64, 64),
            (0x6013, PRODUCT, 0xff07, 1, 64, 64),
            (0x6013, PRODUCT, 0xff07, 0x0222, 63, 64),
            (0x6013, PRODUCT, 0xff07, 0x0222, 64, 65),
        ] {
            let mut wrong = i.clone();
            wrong.product_id = pid; wrong.product = name.into(); wrong.usage_page = page;
            wrong.usage = usage; wrong.input_len = input; wrong.output_len = output;
            assert!(!control(&wrong));
            let mut transport = Fake::default();
            assert!(read_with(&target(&wrong, &parent("one")), &mut transport).is_err());
            assert!(transport.sent.is_empty());
        }
        for path in ["unstructured", r"\\?\hid#vid_3837&pid_6011&mi_00#one#{hid}",
            r"\\?\hid#vid_3837&vid_373b&pid_6013#one#{hid}",
            r"\\?\hid#vid_3837&pid_6013&pid_6011#one#{hid}",
            r"\\?\hid#vid_3837&pid_6013&mi_00&mi_02#one#{hid}",
            r"\\?\hid#vid_3837&pid_6013&col04&col05#one#{hid}",
            r"\\?\hid#vid_3837&pid_6013&mi_xx#one#{hid}",
            r"\\?\hid#vid_3837&pid_6013&col1#one#{hid}",
            r"\\?\hid#vid_3837&pid_6013##{hid}",
            r"\\?\hid#vid_3837&pid_6013#one\other#{hid}"] {
            let mut wrong = i.clone(); wrong.path = path.into();
            assert!(device_instance(&wrong).is_none());
        }
        let mut wrong_backend = target(&i, &parent("one"));
        wrong_backend.backend = BackendId::MchoseMc;
        assert!(read_with(&wrong_backend, &mut Fake::default()).is_err());
    }

    #[test]
    fn physical_parent_proof_rejects_hidden_siblings_and_keeps_receivers_separate() {
        let one = interface("one", "00");
        let two = interface("two", "04");
        assert!(device_instance(&one).is_some() && device_instance(&two).is_some());
        let first_parent = |_: &HidInterface| Some(parent("one"));
        assert_eq!(select_with(&[one.clone()], first_parent, |_| true).len(), 1);
        assert!(select_with(&[one.clone()], first_parent, |_| false).is_empty());
        assert!(select_with(&[one.clone()], |_| None, |_| panic!("unresolved topology")).is_empty());
        for invalid in [r"usb\vid_3837&pid_6013&mi_00\one", r"hid\vid_3837&pid_6013\one",
            r"usb\vid_3837&pid_6011\one", r"usb\vid_3837&pid_6013\"] {
            assert!(!physical_parent_name(invalid));
        }

        let mut sibling = one.clone();
        sibling.path = sibling.path.replace("mi_00", "mi_02");
        sibling.product = "Unknown".into(); sibling.input_len = 63;
        assert!(select_with(&[one.clone(), sibling.clone()], first_parent,
            |_| panic!("ambiguous collection must not be opened")).is_empty());
        assert!(select_with(&[one.clone(), sibling], |i| {
            if i.product == PRODUCT { Some(parent("one")) } else { None }
        }, |_| panic!("unresolved sibling must not be ignored")).is_empty());

        let selected = [one.clone(), two.clone()];
        let selected = select_with(&selected, |i| Some(parent(
            if i.path.contains("&one&") { "one" } else { "two" })), |_| true);
        assert_eq!(selected.len(), 2);
        assert_ne!(target(&one, &parent("one")).key, target(&two, &parent("two")).key);
        assert_eq!(target(&one, &parent("one")).key,
            target(&interface("one", "04"), &parent("one")).key);
    }

    #[test]
    fn unreadable_r9_descriptors_block_queries_without_blocking_other_models() {
        let skipped = |path: &str| crate::hid::Skipped { path: path.into(), reason: "unavailable".into() };
        assert!(unreadable_receiver(&[skipped(r"\\?\HID#VID_3837&PID_6013&MI_04#one#{hid}")]));
        assert!(!unreadable_receiver(&[skipped(r"\\?\hid#vid_3837&pid_6011&mi_03#one#{hid}")]));
        assert!(!unreadable_receiver(&[]));
    }
}
