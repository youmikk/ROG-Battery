//! MT560 24AE:1872: passive Telink8273 Input BB, never a BA command.
//! The official protocol establishes fields, not report cadence or RF freshness.
use super::*;
use crate::{hid::HidDevice, win32::*};
use std::{
    collections::BTreeMap,
    sync::{atomic::{AtomicBool, Ordering}, Arc, Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};
use winapi::{
    shared::hidpi::HIDP_VALUE_CAPS,
    um::cfgmgr32::{CM_Get_Device_IDW, CM_Get_Parent, CM_Locate_DevNodeW},
};

pub struct RapooMt560;
const REPORT: u8 = 0xbb;
const MAX_BATCH: usize = 32;
const SAMPLE_LIFETIME: Duration = Duration::from_secs(120);
const FIRST_WAIT: Duration = Duration::from_secs(2);
const SILENT_REOPEN: Duration = Duration::from_secs(30);
const REOPEN_BACKOFF: Duration = Duration::from_secs(2);
const UNAVAILABLE: &str = "设备无有效状态回报";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x24ae && i.product_id == 0x1872 && i.usage_page == 0xff00 && i.usage == 0x0e
}

fn device_instance(i: &HidInterface) -> Option<String> {
    if !management(i) { return None; }
    let path = i.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid" || parts[2].is_empty()
        || parts[3].is_empty() || path.contains('\0')
        || parts[1].contains('\\') || parts[2].contains('\\') { return None; }
    let fields: Vec<_> = parts[1].split('&').collect();
    for required in ["vid_24ae", "pid_1872"] {
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

fn physical_parent(name: &str) -> bool {
    let parts: Vec<_> = name.split('\\').collect();
    if parts.len() != 3 || parts[0] != "usb" || parts[2].is_empty() { return false; }
    let fields: Vec<_> = parts[1].split('&').collect();
    fields.len() == 2 && fields.contains(&"vid_24ae") && fields.contains(&"pid_1872")
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
        if physical_parent(&name) { return Some(name); }
        node = parent;
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InputLayout { capacity: u16, status_bytes: u16 }

fn input_layout(fields: &[HIDP_VALUE_CAPS], capacity: u16) -> Option<InputLayout> {
    if !(7..=4096).contains(&capacity) || fields.is_empty() || fields.len() > 32 { return None; }
    let mut lengths = BTreeMap::new();
    for field in fields {
        let length = field.ReportCount.checked_add(1)?;
        if field.IsAlias != 0 || field.BitSize != 8 || field.ReportCount == 0
            || ![0xba, REPORT].contains(&field.ReportID) || length > capacity
            || lengths.insert(field.ReportID, length).is_some() { return None; }
    }
    let status_bytes = *lengths.get(&REPORT)?;
    (status_bytes >= 7 && lengths.values().copied().max() == Some(capacity))
        .then_some(InputLayout { capacity, status_bytes })
}

// Access zero reads metadata only, never an input/output stream. The BB length
// must be proven separately because the BA control report may be longer.
struct InputDescriptor { api: HidApi, handle: Handle, data: *mut core::ffi::c_void }

impl Drop for InputDescriptor {
    fn drop(&mut self) {
        unsafe { (self.api.free_preparsed_data)(self.data); CloseHandle(self.handle); }
    }
}

fn prove_input(api: &HidApi, i: &HidInterface) -> Option<InputLayout> {
    if !management(i) || device_instance(i).is_none() || !(7..=4096).contains(&i.input_len) {
        return None;
    }
    let path = to_wide(&i.path);
    let handle = unsafe { CreateFileW(path.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE,
        core::ptr::null_mut(), OPEN_EXISTING, 0, core::ptr::null_mut()) };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE { return None; }
    let mut data = core::ptr::null_mut();
    if unsafe { (api.get_preparsed_data)(handle, &mut data) } == 0 {
        unsafe { CloseHandle(handle); }
        return None;
    }
    let descriptor = InputDescriptor { api: *api, handle, data };
    let mut caps = HidpCaps::default();
    if unsafe { (api.get_caps)(descriptor.data, &mut caps) } < 0
        || caps.usage_page != i.usage_page || caps.usage != i.usage
        || caps.input_report_byte_length != i.input_len || caps.number_input_button_caps != 0
        || !(1..=32).contains(&caps.number_input_value_caps) { return None; }
    let count = caps.number_input_value_caps;
    let mut fields: Vec<HIDP_VALUE_CAPS> = (0..count)
        .map(|_| unsafe { core::mem::zeroed() }).collect();
    let mut returned = count;
    if unsafe { (api.get_value_caps)(0, fields.as_mut_ptr(), &mut returned, descriptor.data) } < 0
        || returned != count { return None; }
    input_layout(&fields, caps.input_report_byte_length)
}

fn select_with<'a>(interfaces: &'a [HidInterface],
    mut parent: impl FnMut(&HidInterface) -> Option<String>,
    mut proof: impl FnMut(&HidInterface) -> Option<InputLayout>) -> Vec<(&'a HidInterface, String, InputLayout)> {
    let mut groups: BTreeMap<String, Vec<&HidInterface>> = BTreeMap::new();
    for i in interfaces.iter().filter(|i| management(i)) {
        let Some(key) = parent(i).filter(|key| physical_parent(key)) else { return Vec::new(); };
        groups.entry(key).or_default().push(i);
    }
    let mut result = Vec::new();
    for (parent, members) in groups {
        let [i] = members.as_slice() else { continue; };
        if device_instance(i).is_none() { continue; }
        if let Some(layout) = proof(i) {
            if layout.capacity == i.input_len && (7..=layout.capacity).contains(&layout.status_bytes) {
                result.push((*i, parent, layout));
            }
        }
    }
    result
}

fn target(i: &HidInterface, parent: &str) -> Target {
    let mut t = Target::new(BackendId::RapooMt560, i, 0);
    t.key = crate::device::stable_key(&format!("rapoo-mt560:{parent}"));
    t.name = "Rapoo MT560".into();
    t.family = DeviceFamily::Mouse;
    t
}

fn percent(frame: &[u8], layout: InputLayout) -> Option<u8> {
    if !(7..=layout.capacity).contains(&layout.status_bytes)
        || (frame.len() != usize::from(layout.status_bytes)
            && frame.len() != usize::from(layout.capacity))
        || frame[..2] != [REPORT, 0xb0] || frame[5] != 1 || frame[6] > 100 { return None; }
    // ReadFile can pad a shorter BB report to the collection's maximum input
    // capacity. Its descriptor-proven body remains the only status data; BA's
    // longer capacity does not authorize arbitrary partial or oversized frames.
    // Byte 2 is an uninterpreted mode, 3/4 are CPI, and flag 0 means invalid
    // battery. None of these fields proves charging or a disconnected mouse.
    Some(frame[6])
}

#[derive(Clone, Copy, Debug)]
struct Sample { received: Instant, percent: u8, bytes: usize }

#[derive(Default)]
struct ReaderState { sample: Option<Sample>, failed: bool }

impl ReaderState {
    fn recent(&self, now: Instant) -> Option<Sample> {
        if self.failed { return None; }
        self.sample.filter(|sample| now.saturating_duration_since(sample.received) < SAMPLE_LIFETIME)
    }
}

// Keep a saturated batch's newest sample until a later read observes an empty
// queue. It retains its original receipt time across batches and idle waits.
#[derive(Default)]
struct Queue { newest: Option<Sample> }

impl Queue {
    fn observed(&mut self, frame: &[u8], layout: InputLayout, received: Instant) {
        if let Some(percent) = percent(frame, layout) {
            self.newest = Some(Sample { received, percent, bytes: frame.len() });
        }
    }

    fn empty(&mut self) -> Option<Sample> { self.newest.take() }
}

enum Batch { Empty(Option<Sample>), Saturated }

fn drain_batch(first: Vec<u8>, layout: InputLayout, queue: &mut Queue,
    mut next: impl FnMut() -> Option<Vec<u8>>) -> Batch {
    let mut frame = first;
    for index in 0..MAX_BATCH {
        queue.observed(&frame, layout, Instant::now());
        if index + 1 == MAX_BATCH { return Batch::Saturated; }
        match next() {
            Some(following) => frame = following,
            None => return Batch::Empty(queue.empty()),
        }
    }
    unreachable!()
}

fn reopen_due(last_input: Instant, now: Instant, read_elapsed: Duration) -> bool {
    read_elapsed < Duration::from_millis(100)
        || now.saturating_duration_since(last_input) >= SILENT_REOPEN
}

struct Reader {
    parent: String,
    layout: InputLayout,
    state: Mutex<ReaderState>,
    changed: Condvar,
    stop: AtomicBool,
}

impl Reader {
    fn halt(&self) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        self.stop.store(true, Ordering::Release);
        state.failed = true;
        self.changed.notify_all();
    }

    fn publish(&self, sample: Sample) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        // Halt and publication use the same lock. A stale worker cannot pass
        // this check and overwrite a stopped session after topology changes.
        if self.stop.load(Ordering::Acquire) || state.failed { return false; }
        state.sample = Some(sample);
        self.changed.notify_all();
        true
    }

    fn pause(&self, duration: Duration) {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        drop(self.changed.wait_timeout_while(state, duration,
            |_| !self.stop.load(Ordering::Acquire)).unwrap_or_else(|p| p.into_inner()));
    }
}

static READERS: OnceLock<Mutex<BTreeMap<String, Arc<Reader>>>> = OnceLock::new();

fn readers() -> &'static Mutex<BTreeMap<String, Arc<Reader>>> {
    READERS.get_or_init(Default::default)
}

fn retain_proven(candidates: &[(&HidInterface, String, InputLayout)]) {
    readers().lock().unwrap_or_else(|p| p.into_inner()).retain(|path, reader| {
        let keep = candidates.iter().any(|(i, parent, layout)| {
            i.path.eq_ignore_ascii_case(path) && *parent == reader.parent && *layout == reader.layout
        });
        if !keep { reader.halt(); }
        keep
    });
}

fn reader(api: &HidApi, i: &HidInterface, parent: &str, layout: InputLayout) -> (Arc<Reader>, bool) {
    let key = i.path.to_ascii_lowercase();
    let mut sessions = readers().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(existing) = sessions.get(&key) {
        if existing.parent == parent && existing.layout == layout
            && !existing.state.lock().unwrap_or_else(|p| p.into_inner()).failed {
            return (Arc::clone(existing), false);
        }
        existing.halt();
    }
    let session = Arc::new(Reader { parent: parent.into(), layout,
        state: Mutex::new(ReaderState::default()), changed: Condvar::new(), stop: AtomicBool::new(false) });
    sessions.insert(key, Arc::clone(&session));
    let worker = Arc::clone(&session);
    let api = *api;
    let i = i.clone();
    let spawned = std::thread::Builder::new().name("rapoo-mt560-input".into()).spawn(move || {
        let mut attempted = false;
        while !worker.stop.load(Ordering::Acquire) {
            if attempted {
                worker.pause(REOPEN_BACKOFF);
                if worker.stop.load(Ordering::Acquire) { break; }
            }
            attempted = true;
            let started = Instant::now();
            // Every open, including recovery opens, must still refer to the
            // unique, descriptor-proven management collection of this parent.
            let interfaces = match crate::hid::enumerate_all_report(&api) {
                Ok(report) => report.interfaces,
                Err(_) => continue,
            };
            let candidates = select_with(&interfaces, usb_parent, |i| prove_input(&api, i));
            let current = candidates.iter().find(|(candidate, parent, candidate_layout)|
                candidate.path.eq_ignore_ascii_case(&i.path) && *parent == worker.parent
                    && *candidate_layout == layout);
            let Some((current, _, _)) = current else { worker.halt(); break; };
            if worker.stop.load(Ordering::Acquire) { break; }
            let mut input = match HidDevice::open_read_only(current) {
                Ok(input) => input,
                Err(_) => {
                    trace::record(&i, BackendId::RapooMt560, "open-input", 0, REPORT,
                        0, 0, started, trace::Outcome::NoReply);
                    continue;
                }
            };
            let mut last_input = Instant::now();
            let mut queue = Queue::default();
            while !worker.stop.load(Ordering::Acquire) {
                let started = Instant::now();
                let frame = input.read_report(400);
                if worker.stop.load(Ordering::Acquire) { break; }
                let Some(frame) = frame else {
                    // Do not publish a queued candidate on an immediate native
                    // failure. A normal idle wait confirms the queue drained.
                    if started.elapsed() >= Duration::from_millis(100) {
                        if let Some(sample) = queue.empty() { publish(&worker, &i, sample, started); }
                    }
                    if reopen_due(last_input, Instant::now(), started.elapsed()) { break; }
                    continue;
                };
                last_input = Instant::now();
                match drain_batch(frame, layout, &mut queue, || {
                    if worker.stop.load(Ordering::Acquire) { None } else { input.read_report(0) }
                }) {
                    Batch::Empty(Some(sample)) => { publish(&worker, &i, sample, started); }
                    Batch::Saturated => worker.pause(Duration::from_millis(2)),
                    Batch::Empty(None) => (),
                }
            }
            // The input handle is dropped before the recovery delay. Opening
            // another handle does not renew the stored battery timestamp.
        }
    });
    if spawned.is_err() {
        session.state.lock().unwrap_or_else(|p| p.into_inner()).failed = true;
        session.changed.notify_all();
    }
    (session, true)
}

fn publish(worker: &Reader, i: &HidInterface, sample: Sample, started: Instant) {
    if worker.publish(sample) {
        trace::record(i, BackendId::RapooMt560, "receive-battery", 0, REPORT,
            0, sample.bytes, started, trace::Outcome::Ok);
    }
}

fn snapshot(t: &Target, sample: Sample) -> Snapshot {
    let mut target = t.clone();
    target.verified_identity = true;
    Snapshot { target,
        battery: Some(Battery { percent: Some(sample.percent), charging: None,
            estimated_percent: None, voltage_mv: None, approximate: false, level: None }),
        battery_at: Some(sample.received), battery_unavailable: false,
        connected: true, dpi: Dpi::default(), polling_rate: Default::default() }
}

impl DeviceBackend for RapooMt560 {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        let candidates = select_with(interfaces, usb_parent, |i| prove_input(api, i));
        retain_proven(&candidates);
        Ok(candidates.into_iter().map(|(i, parent, _)| target(i, &parent)).collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        if t.backend != BackendId::RapooMt560 { return Err(UNAVAILABLE.into()); }
        let interfaces = crate::hid::enumerate_all_report(api)?.interfaces;
        let candidates = select_with(&interfaces, usb_parent, |i| prove_input(api, i));
        retain_proven(&candidates);
        let (i, parent, layout) = candidates.into_iter().find(|(i, _, _)|
            t.hid().is_ok_and(|original| i.path.eq_ignore_ascii_case(&original.path)))
            .ok_or(UNAVAILABLE)?;
        let current = target(i, &parent);
        if current.key != t.key { return Err(UNAVAILABLE.into()); }
        let (reader, first) = reader(api, i, &parent, layout);
        let state = reader.state.lock().unwrap_or_else(|p| p.into_inner());
        let wait = if first { FIRST_WAIT } else { Duration::ZERO };
        let (state, _) = reader.changed.wait_timeout_while(state, wait,
            |state| !state.failed && state.recent(Instant::now()).is_none())
            .unwrap_or_else(|p| p.into_inner());
        let sample = state.recent(Instant::now()).ok_or(UNAVAILABLE)?;
        Ok(snapshot(&current, sample))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    // All fixtures here are synthetic. No complete MT560 hardware BB capture
    // or Windows descriptor was available when implementing this candidate.
    fn interface(instance: &str) -> HidInterface {
        HidInterface { path: format!(r"\\?\hid#vid_24ae&pid_1872&mi_01&col04#8&{instance}&0&0003#{{hid}}"),
            vendor_id: 0x24ae, product_id: 0x1872, usage_page: 0xff00, usage: 0x0e,
            input_len: 32, output_len: 32, feature_len: 0,
            product: String::new(), serial: "shared".into() }
    }
    fn parent(instance: &str) -> String { format!(r"usb\vid_24ae&pid_1872\{instance}") }
    fn layout() -> InputLayout { InputLayout { capacity: 32, status_bytes: 7 } }
    fn field(id: u8, length: u16) -> HIDP_VALUE_CAPS {
        let mut value: HIDP_VALUE_CAPS = unsafe { core::mem::zeroed() };
        value.ReportID = id; value.BitSize = 8; value.ReportCount = length - 1;
        value
    }
    fn frame(value: u8, length: u16) -> Vec<u8> {
        let mut r = vec![0; usize::from(length)];
        r[..7].copy_from_slice(&[REPORT, 0xb0, 0, 0x20, 3, 1, value]);
        r
    }

    #[test]
    fn descriptor_proves_bb_length_separately_from_the_collection_maximum() {
        assert_eq!(input_layout(&[field(0xba, 32), field(REPORT, 7)], 32), Some(layout()));
        assert_eq!(input_layout(&[field(REPORT, 7)], 7),
            Some(InputLayout { capacity: 7, status_bytes: 7 }));
        for fields in [vec![], vec![field(0xba, 32)],
            vec![field(REPORT, 7)], vec![field(7, 32), field(REPORT, 7)],
            vec![field(0xba, 32), field(REPORT, 6)],
            vec![field(0xba, 32), field(REPORT, 7), field(REPORT, 7)]] {
            assert!(input_layout(&fields, 32).is_none());
        }
        let mut alias = field(REPORT, 32); alias.IsAlias = 1;
        let mut bits = field(REPORT, 32); bits.BitSize = 16;
        let mut empty = field(REPORT, 32); empty.ReportCount = 0;
        for bad in [alias, bits, empty] { assert!(input_layout(&[bad], 32).is_none()); }
        assert!(input_layout(&[field(REPORT, 7)], 4097).is_none());
    }

    #[test]
    fn complete_bb_frames_require_valid_battery_without_inventing_charge_or_dpi() {
        for length in [7, 8, 32, 64, 65] {
            let layout = InputLayout { capacity: length, status_bytes: length };
            for value in 0..=255 {
                assert_eq!(percent(&frame(value, length), layout), (value <= 100).then_some(value));
            }
            let good = frame(60, length);
            for short in 0..good.len() { assert!(percent(&good[..short], layout).is_none()); }
            let mut long = good.clone(); long.push(0); assert!(percent(&long, layout).is_none());
            for (offset, value) in [(0, 7), (0, 0xba), (1, 0), (5, 0), (5, 2), (5, 255)] {
                let mut wrong = good.clone(); wrong[offset] = value;
                assert!(percent(&wrong, layout).is_none());
            }
        }
        let at = Instant::now();
        let s = snapshot(&target(&interface("one"), &parent("one")), Sample { received: at, percent: 0, bytes: 7 });
        assert_eq!(s.battery_at, Some(at));
        assert_eq!(s.battery.unwrap().charging, None);
        assert!(s.dpi.current.is_none() && !s.dpi.writable && !s.polling_rate.writable);
    }

    #[test]
    fn short_status_reports_accept_only_their_own_size_or_windows_collection_padding() {
        let shape = layout();
        let short = frame(37, shape.status_bytes);
        assert_eq!(percent(&short, shape), Some(37));
        let mut padded = short.clone();
        padded.resize(usize::from(shape.capacity), 0);
        assert_eq!(percent(&padded, shape), Some(37));
        for length in 0..=usize::from(shape.capacity) + 1 {
            if length == usize::from(shape.status_bytes) || length == usize::from(shape.capacity) {
                continue;
            }
            let mut invalid = short.clone(); invalid.resize(length, 0);
            assert!(percent(&invalid, shape).is_none());
        }
        for (offset, value) in [(0, 0xba), (1, 0), (5, 0), (5, 2), (6, 101)] {
            let mut invalid = padded.clone(); invalid[offset] = value;
            assert!(percent(&invalid, shape).is_none());
        }
    }

    #[test]
    fn queue_processing_uses_the_latest_valid_report_and_has_a_fixed_read_bound() {
        let mut frames: VecDeque<_> = [frame(71, 7), vec![0; 7], frame(70, 7)].into();
        let mut queue = Queue::default();
        let mut calls = 0;
        let Batch::Empty(Some(sample)) = drain_batch(frame(72, 7), layout(), &mut queue,
            || { calls += 1; frames.pop_front() }) else { panic!("queue should drain"); };
        assert_eq!(sample.percent, 70);
        assert_eq!(calls, 4);
        let mut calls = 0;
        assert!(matches!(drain_batch(frame(72, 7), layout(), &mut queue,
            || { calls += 1; Some(frame(71, 7)) }), Batch::Saturated));
        assert_eq!(calls, MAX_BATCH - 1);
        assert_eq!(queue.empty().unwrap().percent, 71);
        assert!(matches!(drain_batch(vec![0; 7], layout(), &mut queue, || None), Batch::Empty(None)));
    }

    #[test]
    fn exact_batch_multiples_are_published_after_idle_with_original_receipt_time() {
        for count in [31usize, 32, 33, 64, 65, 96] {
            let mut frames: VecDeque<_> = (0..count).map(|n| frame(n as u8, 7)).collect();
            let mut queue = Queue::default();
            let mut sample = None;
            let mut batches = 0;
            while let Some(first) = frames.pop_front() {
                let mut calls = 0;
                let batch = drain_batch(first, layout(), &mut queue, || { calls += 1; frames.pop_front() });
                assert!(calls <= MAX_BATCH - 1);
                batches += 1;
                match batch {
                    Batch::Empty(value) => sample = value,
                    Batch::Saturated => assert!(sample.is_none()),
                }
            }
            assert_eq!(batches, count.div_ceil(MAX_BATCH));
            if count % MAX_BATCH == 0 {
                assert!(sample.is_none());
                let queued_at = queue.newest.unwrap().received;
                sample = queue.empty();
                assert_eq!(sample.unwrap().received, queued_at);
            }
            assert_eq!(sample.unwrap().percent, (count - 1) as u8);
            assert!(queue.empty().is_none());
        }
        let at = Instant::now();
        let mut queue = Queue::default();
        queue.observed(&frame(55, 7), layout(), at);
        queue.observed(&vec![0; 7], layout(), at + Duration::from_secs(1));
        let sample = queue.empty().unwrap();
        assert_eq!(sample.received, at);
        assert_eq!(sample.percent, 55);
    }

    #[test]
    fn continuous_queue_stays_bounded_and_keeps_candidate_across_invalid_batches() {
        let mut queue = Queue::default();
        let at = Instant::now();
        queue.observed(&frame(73, 7), layout(), at);
        for _ in 0..4 {
            let mut reads = 0;
            assert!(matches!(drain_batch(vec![0; 7], layout(), &mut queue, || {
                reads += 1; Some(vec![0; 7])
            }), Batch::Saturated));
            assert_eq!(reads, MAX_BATCH - 1);
            assert_eq!(queue.newest.unwrap().received, at);
        }
        assert_eq!(queue.empty().unwrap().percent, 73);
        assert!(queue.empty().is_none());
    }

    #[test]
    fn silent_or_invalid_input_never_renews_the_original_timestamp() {
        let start = Instant::now();
        let t = target(&interface("one"), &parent("one"));
        let sample = Sample { received: start, percent: 55, bytes: 7 };
        let mut state = ReaderState { sample: Some(sample), failed: false };
        for seconds in [0, 10, 60, 119] {
            let current = state.recent(start + Duration::from_secs(seconds)).unwrap();
            assert_eq!(snapshot(&t, current).battery_at, Some(start));
        }
        assert!(state.recent(start + SAMPLE_LIFETIME).is_none());
        assert!(matches!(drain_batch(vec![0; 7], layout(), &mut Queue::default(), || None), Batch::Empty(None)));
        assert_eq!(state.sample.unwrap().received, start);
        state.failed = true;
        assert!(state.recent(start).is_none());
        state = ReaderState { sample: Some(Sample { received: start + SAMPLE_LIFETIME,
            percent: 54, bytes: 7 }), failed: false };
        assert_eq!(state.recent(start + SAMPLE_LIFETIME).unwrap().percent, 54);
    }

    #[test]
    fn slow_errors_reopen_after_bounded_silence_without_renewing_a_sample() {
        let at = Instant::now();
        let sample = Sample { received: at, percent: 52, bytes: 7 };
        let state = ReaderState { sample: Some(sample), failed: false };
        assert!(reopen_due(at, at, Duration::from_millis(1)));
        assert!(!reopen_due(at, at + SILENT_REOPEN - Duration::from_millis(1), Duration::from_millis(400)));
        for seconds in [30, 60, 90, 120] {
            assert!(reopen_due(at, at + Duration::from_secs(seconds), Duration::from_millis(400)));
            assert_eq!(state.sample.unwrap().received, at);
        }
        assert!(state.recent(at + SAMPLE_LIFETIME).is_none());
        assert!(!reopen_due(at + Duration::from_secs(29), at + Duration::from_secs(30), Duration::from_millis(400)));
        assert!(REOPEN_BACKOFF >= Duration::from_secs(2));
    }

    #[test]
    fn stopped_reader_cannot_publish_a_late_sample_or_affect_its_replacement() {
        let reader = || Reader { parent: parent("one"), layout: layout(),
            state: Mutex::new(ReaderState::default()), changed: Condvar::new(), stop: AtomicBool::new(false) };
        let old = reader();
        let replacement = reader();
        let sample = Sample { received: Instant::now(), percent: 80, bytes: 7 };
        assert!(old.publish(sample));
        old.halt();
        assert!(!old.publish(Sample { percent: 79, ..sample }));
        let state = old.state.lock().unwrap();
        assert!(state.failed && state.recent(Instant::now()).is_none());
        assert_eq!(state.sample.unwrap().percent, 80);
        assert!(replacement.publish(Sample { percent: 78, ..sample }));
        assert_eq!(replacement.state.lock().unwrap().sample.unwrap().percent, 78);
    }

    #[test]
    fn physical_parent_and_unique_management_collection_are_required_before_opening() {
        let one = interface("one"); let two = interface("two");
        let pick_parent = |i: &HidInterface| Some(parent(if i.path.contains("&two&") { "two" } else { "one" }));
        assert_eq!(select_with(&[one.clone(), two.clone()], pick_parent, |_| Some(layout())).len(), 2);
        assert_ne!(target(&one, &parent("one")).key, target(&two, &parent("two")).key);
        let mut sibling = one.clone(); sibling.path = sibling.path.replace("col04", "col05"); sibling.input_len = 6;
        assert!(select_with(&[one.clone(), sibling], pick_parent, |_| panic!("duplicate must not prove")).is_empty());
        assert!(select_with(&[one.clone()], |_| None, |_| panic!("unknown parent")).is_empty());
        assert!(select_with(&[one.clone()], pick_parent, |_| None).is_empty());
        assert_eq!(select_with(&[one.clone()], pick_parent, |_| Some(layout())).len(), 1);
        assert!(select_with(&[one], |_| Some("collection:one".into()), |_| panic!("not a USB parent")).is_empty());
    }

    #[test]
    fn other_rapoo_families_and_ordinary_input_collections_are_excluded() {
        let good = interface("one");
        for (vid, pid, page, usage) in [(0x24ae, 0x4504, 0xff00, 0x0e),
            (0x24ae, 0x1870, 0xff00, 0x0e), (0x24ae, 0x1851, 0xff00, 0x0e),
            (0x24ae, 0x1872, 0xff00, 2), (0x24ae, 0x1872, 1, 2),
            (0x25a7, 0x2410, 0xff29, 1)] {
            let mut wrong = good.clone(); wrong.vendor_id = vid; wrong.product_id = pid;
            wrong.usage_page = page; wrong.usage = usage;
            assert!(!management(&wrong));
            assert!(select_with(&[wrong], |_| panic!("wrong identity"), |_| panic!("wrong identity")).is_empty());
        }
        for path in ["invalid", r"\\?\hid#vid_24ae&pid_1872&pid_1870&mi_01#one#{hid}",
            r"\\?\hid#vid_24ae&pid_1872&mi_01&mi_02#one#{hid}"] {
            let mut wrong = good.clone(); wrong.path = path.into();
            assert!(device_instance(&wrong).is_none());
        }
    }
}
