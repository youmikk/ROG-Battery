//! ATK 8K Nano 373B:1178 receiver: Holtek GetConfigData on FF10:0001.
//! Feature report zero requests eight bytes; the reply arrives on Input zero.
use super::*;
use crate::{hid::HidDevice, win32::to_wide};
use std::{collections::BTreeMap, time::{Duration, Instant}};
use winapi::um::cfgmgr32::{CM_Get_Device_IDW, CM_Get_Parent, CM_Locate_DevNodeW};

pub struct AtkHoltek;
const INPUT_LENGTH: u16 = 65;
const FEATURE_LENGTH: u16 = 9;
const COMMAND: u8 = 0x82;
const MAX_READS: usize = 32;
const UNAVAILABLE: &str = "设备暂未响应，请唤醒或检查连接后重试";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x373b && i.product_id == 0x1178
        && i.usage_page == 0xff10 && i.usage == 1
}

fn capacities(i: &HidInterface) -> bool {
    management(i) && i.input_len == INPUT_LENGTH && i.output_len == INPUT_LENGTH
        && i.feature_len == FEATURE_LENGTH
}

fn device_instance(i: &HidInterface) -> Option<String> {
    if !management(i) { return None; }
    let path = i.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid" || parts[2].is_empty()
        || parts[3].is_empty() || path.contains('\0')
        || parts[1].contains('\\') || parts[2].contains('\\') { return None; }
    let fields: Vec<_> = parts[1].split('&').collect();
    for required in ["vid_373b", "pid_1178"] {
        if fields.iter().filter(|field| **field == required).count() != 1 { return None; }
    }
    if fields.iter().filter(|field| field.starts_with("vid_")).count() != 1
        || fields.iter().filter(|field| field.starts_with("pid_")).count() != 1 { return None; }
    for prefix in ["mi_", "col"] {
        let indices: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix(prefix)).collect();
        if indices.len() > 1 || indices.iter().any(|index|
            index.len() != 2 || !index.bytes().all(|b| b.is_ascii_hexdigit())) { return None; }
    }
    Some(format!("hid\\{}\\{}", parts[1], parts[2]))
}

fn physical_parent_name(name: &str) -> bool {
    let parts: Vec<_> = name.split('\\').collect();
    if parts.len() != 3 || parts[0] != "usb" || parts[2].is_empty() { return false; }
    let fields: Vec<_> = parts[1].split('&').collect();
    fields.len() == 2 && fields.contains(&"vid_373b") && fields.contains(&"pid_1178")
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
    if !capacities(i) || device_instance(i).is_none() { return false; }
    descriptor::Descriptor::open(api, i).is_some_and(|d|
        d.single_input_output_byte_report_ids(i, INPUT_LENGTH, INPUT_LENGTH) == Some((0, 0))
        && d.feature_reports(i, FEATURE_LENGTH, &[0], &[0])
        && d.feature_byte_report(i, 0, FEATURE_LENGTH))
}

fn select_with<'a>(interfaces: &'a [HidInterface],
    mut parent: impl FnMut(&HidInterface) -> Option<String>,
    mut proof: impl FnMut(&HidInterface) -> bool) -> Vec<(&'a HidInterface, String)> {
    let mut groups: BTreeMap<String, Vec<&HidInterface>> = BTreeMap::new();
    for i in interfaces.iter().filter(|i| management(i)) {
        let Some(key) = parent(i).filter(|key| physical_parent_name(key)) else { return Vec::new(); };
        groups.entry(key).or_default().push(i);
    }
    let mut result = Vec::new();
    for (parent, members) in groups {
        let [i] = members.as_slice() else { continue; };
        if capacities(i) && device_instance(i).is_some() && proof(i) { result.push((*i, parent)); }
    }
    result
}

fn target(i: &HidInterface, parent: &str) -> Target {
    let mut t = Target::new(BackendId::AtkHoltek, i, 0);
    t.key = crate::device::stable_key(&format!("atk-holtek:{parent}"));
    // This receiver is shared; it does not prove the paired mouse's model.
    t.name = "ATK 8K Nano".into();
    t.family = DeviceFamily::Mouse;
    t
}

fn request() -> [u8; FEATURE_LENGTH as usize] { [0, COMMAND, 0, 0, 0, 0, 0, 0, 0] }

#[derive(Debug, PartialEq, Eq)]
enum Reply { Battery { percent: u8, charging: bool }, NoLink }

fn reply(r: &[u8]) -> Option<Reply> {
    // Native ReadFile retains the zero ID slot. All offsets below include it;
    // the official WebHID codec uses payload offsets 8 (battery) and 9 (link).
    if r.len() != usize::from(INPUT_LENGTH) || r[..2] != [0, COMMAND] { return None; }
    if r[10] == 0 { return Some(Reply::NoLink); }
    let percent = r[9] & 0x7f;
    if !(1..=100).contains(&percent) { return None; }
    Some(Reply::Battery { percent, charging: r[9] & 0x80 != 0 })
}

trait Transport {
    fn send(&mut self, request: &[u8]) -> Result<(), String>;
    fn receive(&mut self, timeout_ms: u32) -> Option<Vec<u8>>;
}

struct NativeTransport { input: HidDevice, feature: feature::FeatureChannel }

impl NativeTransport {
    fn open(i: &HidInterface) -> Result<Self, String> {
        // Open Input before the query so an immediate interrupt reply is queued.
        let input = HidDevice::open_read_only(i)?;
        let feature = feature::FeatureChannel::open(i)?;
        Ok(Self { input, feature })
    }
}

impl Transport for NativeTransport {
    fn send(&mut self, packet: &[u8]) -> Result<(), String> {
        if packet != request() { return Err(UNAVAILABLE.into()); }
        self.feature.send(packet, 250)
    }

    fn receive(&mut self, timeout_ms: u32) -> Option<Vec<u8>> { self.input.read_report(timeout_ms) }
}

fn read_with(t: &Target, transport: &mut impl Transport) -> Result<Snapshot, String> {
    let i = t.hid()?;
    if t.backend != BackendId::AtkHoltek || !capacities(i) || device_instance(i).is_none() {
        return Err(UNAVAILABLE.into());
    }
    // A bounded pre-query drain keeps already queued status/config replies from
    // becoming this query's result. If the queue stays busy, send nothing.
    let mut drained = false;
    for _ in 0..8 {
        if transport.receive(0).is_none() { drained = true; break; }
    }
    if !drained { return Err(UNAVAILABLE.into()); }
    let started = Instant::now();
    if let Err(error) = transport.send(&request()) {
        trace::record(i, BackendId::AtkHoltek, "query-battery", COMMAND.into(), 0,
            usize::from(FEATURE_LENGTH), 0, started, trace::Outcome::NoReply);
        return Err(error);
    }
    let deadline = Instant::now() + Duration::from_millis(900);
    let mut last_length = 0;
    for _ in 0..MAX_READS {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else { break; };
        let Some(frame) = transport.receive(left.as_millis().max(1) as u32) else { break; };
        last_length = frame.len();
        let Some(state) = reply(&frame) else { continue; };
        let battery = match state {
            Reply::Battery { percent, charging } => Some(Battery { percent: Some(percent),
                charging: Some(charging), estimated_percent: None, voltage_mv: None,
                approximate: false, level: None }),
            Reply::NoLink => None,
        };
        trace::record(i, BackendId::AtkHoltek, "query-battery", COMMAND.into(), 0,
            usize::from(FEATURE_LENGTH), last_length, started, trace::Outcome::Ok);
        let mut target = t.clone();
        target.verified_identity = true;
        return Ok(Snapshot { target, battery_at: battery.as_ref().map(|_| Instant::now()),
            battery, battery_unavailable: state == Reply::NoLink,
            connected: state != Reply::NoLink, dpi: Dpi::default(), polling_rate: Default::default() });
    }
    trace::record(i, BackendId::AtkHoltek, "query-battery", COMMAND.into(), 0,
        usize::from(FEATURE_LENGTH), last_length, started,
        if last_length == 0 { trace::Outcome::NoReply } else { trace::Outcome::InvalidData });
    Err(UNAVAILABLE.into())
}

impl DeviceBackend for AtkHoltek {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(select_with(interfaces, usb_parent, |i| proven(api, i)).into_iter()
            .map(|(i, parent)| target(i, &parent)).collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        if t.backend != BackendId::AtkHoltek { return Err(UNAVAILABLE.into()); }
        let interfaces = crate::hid::enumerate_all_report(api)?.interfaces;
        let candidates = select_with(&interfaces, usb_parent, |i| proven(api, i));
        let (i, parent) = candidates.into_iter()
            .find(|(i, _)| t.hid().is_ok_and(|original| i.path.eq_ignore_ascii_case(&original.path)))
            .ok_or(UNAVAILABLE)?;
        let lock = channel::lock(&parent);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let current = target(i, &parent);
        if current.key != t.key { return Err(UNAVAILABLE.into()); }
        read_with(&current, &mut NativeTransport::open(i)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn interface(instance: &str) -> HidInterface {
        // Synthetic path only; the model's MI number has not been captured here.
        HidInterface { path: format!(r"\\?\hid#vid_373b&pid_1178&mi_01&col01#8&{instance}&0&0000#{{hid}}"),
            vendor_id: 0x373b, product_id: 0x1178, usage_page: 0xff10, usage: 1,
            input_len: INPUT_LENGTH, output_len: INPUT_LENGTH, feature_len: FEATURE_LENGTH,
            product: "ATK 8K Nano Dongle".into(), serial: "shared".into() }
    }

    fn parent(instance: &str) -> String { format!(r"usb\vid_373b&pid_1178\{instance}") }

    fn synthetic_frame(percent: u8, charging: bool, link: u8) -> Vec<u8> {
        // Source-derived synthetic fixture, not a captured hardware report.
        let mut r = vec![0; usize::from(INPUT_LENGTH)];
        r[1] = COMMAND;
        r[9] = percent | if charging { 0x80 } else { 0 };
        r[10] = link;
        r
    }

    struct Fake { reads: VecDeque<Option<Vec<u8>>>, sent: Vec<Vec<u8>>, fail_send: bool, queries: usize }

    impl Fake {
        fn new(reads: Vec<Option<Vec<u8>>>) -> Self {
            Self { reads: reads.into(), sent: vec![], fail_send: false, queries: 0 }
        }
    }

    impl Transport for Fake {
        fn send(&mut self, request: &[u8]) -> Result<(), String> {
            self.sent.push(request.to_vec());
            if self.fail_send { Err("feature failed".into()) } else { Ok(()) }
        }
        fn receive(&mut self, timeout: u32) -> Option<Vec<u8>> {
            assert!(timeout <= 900);
            if timeout > 0 { self.queries += 1; }
            self.reads.pop_front().expect("unexpected input read")
        }
    }

    #[test]
    fn holtek_feature_query_is_separate_from_bitmouse_and_all_configuration_writes() {
        assert_eq!(request(), [0, 0x82, 0, 0, 0, 0, 0, 0, 0]);
        let t = target(&interface("one"), &parent("one"));
        let mut fake = Fake::new(vec![None, Some(synthetic_frame(79, true, 1))]);
        let sample = read_with(&t, &mut fake).unwrap();
        assert_eq!(fake.sent, vec![request().to_vec()]);
        assert_eq!((sample.battery.as_ref().unwrap().percent, sample.battery.unwrap().charging),
            (Some(79), Some(true)));
        assert!(sample.connected && sample.battery_at.is_some() && !sample.battery_unavailable);
        assert!(sample.dpi.stages.is_empty() && !sample.dpi.writable && !sample.polling_rate.writable);
    }

    #[test]
    fn full_windows_input_and_documented_offsets_are_required() {
        for percent in 1..=100 {
            for charging in [false, true] {
                assert_eq!(reply(&synthetic_frame(percent, charging, 1)),
                    Some(Reply::Battery { percent, charging }));
            }
        }
        let valid = synthetic_frame(79, true, 1);
        for length in 0..65 { assert!(reply(&valid[..length]).is_none()); }
        let mut long = valid.clone(); long.push(0); assert!(reply(&long).is_none());
        for (offset, value) in [(0, 1), (1, 0x12), (1, 0xa5), (9, 0), (9, 101), (9, 255)] {
            let mut wrong = valid.clone(); wrong[offset] = value; assert!(reply(&wrong).is_none());
        }
        assert!(reply(&request()).is_none());
        let mut unknown_link = valid; unknown_link[10] = 2;
        // The vendor treats any nonzero linkStatus as online, not as a bitfield.
        assert!(matches!(reply(&unknown_link), Some(Reply::Battery { .. })));
    }

    #[test]
    fn zero_link_clears_current_sample_without_reusing_receiver_percentage() {
        assert_eq!(reply(&synthetic_frame(127, true, 0)), Some(Reply::NoLink));
        let t = target(&interface("one"), &parent("one"));
        let mut sleeping = Fake::new(vec![None, Some(synthetic_frame(79, true, 0))]);
        let sample = read_with(&t, &mut sleeping).unwrap();
        assert!(!sample.connected && sample.battery_unavailable);
        assert!(sample.battery.is_none() && sample.battery_at.is_none());
        let mut awake = Fake::new(vec![None, Some(synthetic_frame(78, false, 1))]);
        assert_eq!(read_with(&t, &mut awake).unwrap().battery.unwrap().percent, Some(78));
    }

    #[test]
    fn pending_frames_are_drained_and_failed_queries_are_never_resent() {
        let t = target(&interface("one"), &parent("one"));
        let stale = synthetic_frame(98, true, 1);
        let mut fake = Fake::new(vec![Some(stale.clone()), None,
            Some(vec![0; 65]), Some(synthetic_frame(78, false, 1))]);
        assert_eq!(read_with(&t, &mut fake).unwrap().battery.unwrap().percent, Some(78));
        assert_eq!((fake.sent.len(), fake.queries), (1, 2));
        let mut congested = Fake::new(vec![Some(stale); 8]);
        assert!(read_with(&t, &mut congested).is_err());
        assert!(congested.sent.is_empty());
        let mut silent = Fake::new(vec![None, None]);
        assert!(read_with(&t, &mut silent).is_err());
        assert_eq!(silent.sent.len(), 1);
        let mut invalid = vec![None]; invalid.extend(vec![Some(vec![0; 65]); MAX_READS]);
        let mut invalid = Fake::new(invalid);
        assert!(read_with(&t, &mut invalid).is_err());
        assert_eq!((invalid.sent.len(), invalid.queries), (1, MAX_READS));
        let mut failed = Fake::new(vec![None]); failed.fail_send = true;
        assert!(read_with(&t, &mut failed).is_err());
        assert_eq!((failed.sent.len(), failed.queries), (1, 0));
    }

    #[test]
    fn exact_receiver_reports_and_usb_parent_prevent_protocol_fallbacks() {
        let i = interface("one");
        let choose_parent = |_: &HidInterface| Some(parent("one"));
        assert_eq!(select_with(&[i.clone()], choose_parent, |_| true).len(), 1);
        for (pid, page, usage, input, output, feature) in [
            (0x1216, 0xff05, 1, 64, 64, 0), (0x10f0, 0xff10, 1, 65, 65, 9),
            (0x1178, 0xff02, 2, 17, 17, 0), (0x1178, 0xffb1, 1, 65, 65, 9),
            (0x1178, 0xffb3, 1, 65, 65, 9), (0x1178, 0xff10, 2, 65, 65, 9),
            (0x1178, 0xff10, 1, 64, 65, 9), (0x1178, 0xff10, 1, 65, 64, 9),
            (0x1178, 0xff10, 1, 65, 65, 8), (0x1178, 0xff10, 1, 65, 65, 65),
        ] {
            let mut wrong = i.clone();
            wrong.product_id = pid; wrong.usage_page = page; wrong.usage = usage;
            wrong.input_len = input; wrong.output_len = output; wrong.feature_len = feature;
            assert!(!capacities(&wrong));
            let mut fake = Fake::new(vec![]);
            assert!(read_with(&target(&wrong, &parent("one")), &mut fake).is_err());
            assert!(fake.sent.is_empty());
        }
        assert!(select_with(&[i.clone()], |_| None, |_| panic!("unproven parent")).is_empty());
        assert!(select_with(&[i.clone()], choose_parent, |_| false).is_empty());
        let mut sibling = i.clone();
        sibling.path = sibling.path.replace("mi_01", "mi_02"); sibling.input_len = 64;
        assert!(select_with(&[i.clone(), sibling], choose_parent,
            |_| panic!("ambiguous parent must be rejected before report proof")).is_empty());
        for path in ["unstructured", r"\\?\hid#vid_373b&pid_1216#one#{hid}",
            r"\\?\hid#vid_373b&pid_1178&pid_1216#one#{hid}"] {
            let mut wrong = i.clone(); wrong.path = path.into(); assert!(device_instance(&wrong).is_none());
        }
        assert!(!physical_parent_name(r"usb\vid_373b&pid_1178&mi_01\one"));
        assert_ne!(target(&i, &parent("one")).key, target(&interface("two"), &parent("two")).key);
        let mut wrong_backend = target(&i, &parent("one")); wrong_backend.backend = BackendId::AtkNano;
        assert!(read_with(&wrong_backend, &mut Fake::new(vec![])).is_err());
    }
}
