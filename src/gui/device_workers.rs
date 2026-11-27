use super::*;
use crate::backends::{BackendId, BackendScan, Snapshot, Target};
use std::collections::BTreeSet;
use std::time::Instant;

const CONFIGURATION_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct Request {
    targets: Vec<Target>,
    sequence: u64,
}

#[derive(Default)]
struct Mailbox {
    request: Mutex<Option<Request>>,
    changed: Condvar,
    stopped: AtomicBool,
}

impl Mailbox {
    fn send(&self, request: Request) {
        *self.request.lock().unwrap_or_else(|error| error.into_inner()) = Some(request);
        self.changed.notify_one();
    }

    fn stop(&self) {
        let _guard = self.request.lock().unwrap_or_else(|error| error.into_inner());
        self.stopped.store(true, Ordering::Release);
        self.changed.notify_one();
    }

    fn next(&self) -> Option<Request> {
        let request = self.request.lock().unwrap_or_else(|error| error.into_inner());
        let mut request = self.changed.wait_while(request,
            |request| request.is_none() && !self.stopped.load(Ordering::Acquire))
            .unwrap_or_else(|error| error.into_inner());
        if self.stopped.load(Ordering::Acquire) { None } else { request.take() }
    }
}

struct Worker {
    mailbox: Arc<Mailbox>,
    thread: std::thread::JoinHandle<()>,
}

#[derive(Default)]
pub(super) struct Workers {
    active: BTreeMap<String, Worker>,
    retired: Vec<(String, std::thread::JoinHandle<()>)>,
}

fn groups(targets: &[Target]) -> BTreeMap<String, Vec<Target>> {
    let mut groups = BTreeMap::<String, Vec<Target>>::new();
    for target in targets {
        let targets = groups.entry(crate::backends::communication_group(target)).or_default();
        if !targets.iter().any(|other| other.address == target.address
            && other.channel_key().eq_ignore_ascii_case(&target.channel_key())) { targets.push(target.clone()); }
    }
    groups
}

fn same_descriptor(first: &Target, second: &Target) -> bool {
    match (first.hid(), second.hid()) {
        (Ok(first), Ok(second)) => first.vendor_id == second.vendor_id && first.product_id == second.product_id
            && first.serial == second.serial,
        _ => first.same_channel(second),
    }
}

impl Workers {
    pub(super) fn update(&mut self, backend: BackendId, scan: &BackendScan, sequence: u64,
        shared: &Arc<Mutex<Shared>>, signal: &Arc<Signal>, notice: nwg::NoticeSender) {
        let groups = groups(&scan.targets);
        let absent: Vec<_> = self.active.keys().filter(|key| !groups.contains_key(*key)).cloned().collect();
        for key in absent {
            if let Some(worker) = self.active.remove(&key) {
                worker.mailbox.stop();
                self.retired.push((key, worker.thread));
            }
        }
        self.retired.retain(|(_, thread)| !thread.is_finished());
        for (group, targets) in groups {
            if self.retired.iter().any(|(retired, _)| retired == &group) { continue; }
            if !self.active.contains_key(&group) {
                let mailbox = Arc::new(Mailbox::default());
                let worker_mailbox = Arc::clone(&mailbox);
                let worker_shared = Arc::clone(shared);
                let worker_signal = Arc::clone(signal);
                let worker_group = group.clone();
                let thread = std::thread::Builder::new().name("device-reader".into()).spawn(move || {
                    run(backend, &worker_group, &worker_mailbox, &worker_shared, &worker_signal, notice);
                });
                match thread {
                    Ok(thread) => { self.active.insert(group.clone(), Worker { mailbox, thread }); }
                    Err(error) => {
                        crate::diagnostics::event_once("device_worker_start_failed", &error.to_string());
                        continue;
                    }
                }
            }
            self.active[&group].mailbox.send(Request { targets, sequence });
        }
    }
}

impl std::ops::Drop for Workers {
    fn drop(&mut self) {
        for worker in self.active.values() { worker.mailbox.stop(); }
        for (_, worker) in std::mem::take(&mut self.active) { let _ = worker.thread.join(); }
        for (_, thread) in self.retired.drain(..) { let _ = thread.join(); }
    }
}

pub(super) fn cancel_missing_commands(state: &mut Shared, backend: BackendId, targets: &[Target]) {
    if let Some(queue) = state.source_pending.get_mut(&backend) {
        let mut retained = VecDeque::new();
        while let Some(command) = queue.pop_front() {
            if command.basic_target().is_some_and(|target| targets.iter().any(|current| current.same_channel(target))) {
                retained.push_back(command);
            } else {
                state.results.push_back(CmdResult { label: command.label(), ok: None,
                    err: Some(crate::ui::feedback::text("设备已断开，请重新连接", "Device disconnected. Reconnect it.").into()), hint: "" });
            }
        }
        *queue = retained;
    }
    if state.source_pending.get(&backend).is_none_or(VecDeque::is_empty)
        && state.source_running.get(&backend).copied().unwrap_or(0) == 0 {
        state.source_busy.remove(&backend);
    }
}

fn commands(backend: BackendId, group: &str, shared: &Arc<Mutex<Shared>>, mailbox: &Mailbox, signal: &Signal,
    execute: &mut impl FnMut(&Cmd) -> CmdResult, notify: &impl Fn()) -> bool {
    let mut wrote = false;
    for _ in 0..MAX_PENDING * 2 {
        if mailbox.stopped.load(Ordering::Acquire) || signal.shutdown.load(Ordering::Relaxed) { break; }
        let command = {
            let mut state = lock_shared(shared);
            let command = state.source_pending.get_mut(&backend).and_then(|queue| {
                let index = queue.iter().position(|command| command.basic_target()
                    .is_some_and(|target| crate::backends::communication_group(target) == group))?;
                queue.remove(index)
            });
            if command.is_some() { *state.source_running.entry(backend).or_default() += 1; }
            command
        };
        let Some(command) = command else { break; };
        let result = execute(&command);
        let mut state = lock_shared(shared);
        state.results.push_back(result);
        *state.source_running.entry(backend).or_default() -= 1;
        if state.source_pending.get(&backend).is_none_or(VecDeque::is_empty)
            && state.source_running.get(&backend).copied().unwrap_or(0) == 0 {
            state.source_busy.remove(&backend);
        }
        drop(state);
        notify();
        wrote = true;
    }
    wrote
}

fn run(backend: BackendId, group: &str, mailbox: &Mailbox, shared: &Arc<Mutex<Shared>>,
    signal: &Signal, notice: nwg::NoticeSender) {
    run_with(backend, group, mailbox, shared, signal, crate::backends::read_target,
        |command| run_cmd(command, None), || notice.notice());
}

fn run_with(backend: BackendId, group: &str, mailbox: &Mailbox, shared: &Arc<Mutex<Shared>>,
    signal: &Signal, mut read: impl FnMut(&Target, bool) -> Result<Snapshot, String>,
    mut execute: impl FnMut(&Cmd) -> CmdResult, notify: impl Fn()) {
    let mut configured = BTreeMap::<String, (Instant, u64, bool)>::new();
    let mut responding = BTreeSet::new();
    while let Some(mut request) = mailbox.next() {
        if signal.shutdown.load(Ordering::Relaxed) { break; }
        request.targets.sort_by_key(|target| !responding.contains(&(target.channel_key(), target.address)));
        let mut live = Vec::new();
        let mut sampled = Vec::<Target>::new();
        for target in &request.targets {
            if mailbox.stopped.load(Ordering::Acquire) || signal.shutdown.load(Ordering::Relaxed) { return; }
            if sampled.iter().any(|previous| previous.same_channel(target)) { continue; }
            if commands(backend, group, shared, mailbox, signal, &mut execute, &notify) { configured.clear(); }
            if mailbox.stopped.load(Ordering::Acquire) || signal.shutdown.load(Ordering::Relaxed) { return; }
            let generation = *lock_shared(shared).source_channel_gen.entry((backend, group.to_string())).or_default();
            let result = read(target, false);
            if result.is_ok() { sampled.push(target.clone()); }
            if result.as_ref().is_ok_and(|snapshot| lock_shared(shared).devices.values()
                .filter_map(|device| device.basic.as_ref()).any(|stored|
                    stored.target.same_channel(target) && (dpi_context_changed(&snapshot.dpi, &stored.dpi)
                        || !same_descriptor(&snapshot.target, &stored.target)
                        || snapshot.target.verified_identity != stored.target.verified_identity
                        || (snapshot.target.verified_identity && snapshot.target.key != stored.target.key)))) {
                configured.remove(&target.unverified_key());
            }
            if result.as_ref().is_ok_and(|snapshot| snapshot.connected) {
                responding.insert((target.channel_key(), target.address));
                live.retain(|candidate: &Target| !candidate.same_channel(target));
                live.push(target.clone());
            } else {
                if responding.remove(&(target.channel_key(), target.address)) { configured.remove(&target.unverified_key()); }
                if result.is_err() && crate::backends::has_configuration(backend)
                    && !matches!(backend, BackendId::Rapoo | BackendId::Logitech) {
                    live.push(target.clone());
                }
            }
            publish(shared, mailbox, target, result, generation, false);
            notify();
        }
        if mailbox.stopped.load(Ordering::Acquire) || signal.shutdown.load(Ordering::Relaxed) { return; }
        {
            let mut state = lock_shared(shared);
            state.source_completed.insert((backend, group.to_string()), request.sequence);
            let completed = state.source_discoveries.get(&backend).is_some_and(|scan|
                groups(&scan.targets).keys().all(|group|
                    state.source_completed.get(&(backend, group.clone())).copied().unwrap_or(0) >= request.sequence));
            if completed { state.complete_refresh(backend, request.sequence); }
        }
        if !crate::backends::has_configuration(backend) { continue; }
        let selected = lock_shared(shared).detail_key.clone();
        live.sort_by_key(|target| !selected.as_ref().is_some_and(|key|
            lock_shared(shared).devices.get(key).and_then(|device| device.basic.as_ref())
                .is_some_and(|snapshot| snapshot.target.same_channel(target))));
        let mut attempts = BTreeMap::new();
        let mut detailed = Vec::<Target>::new();
        for target in live {
            if mailbox.stopped.load(Ordering::Acquire) || signal.shutdown.load(Ordering::Relaxed) { return; }
            if detailed.iter().any(|previous| previous.same_channel(&target)) { continue; }
            if commands(backend, group, shared, mailbox, signal, &mut execute, &notify) {
                configured.clear();
                attempts.clear();
                detailed.clear();
            }
            if mailbox.stopped.load(Ordering::Acquire) || signal.shutdown.load(Ordering::Relaxed) { return; }
            let preferred = {
                let state = lock_shared(shared);
                state.detail_key.as_ref().and_then(|key| state.devices.get(key))
                    .and_then(|device| device.basic.as_ref())
                    .is_some_and(|snapshot| snapshot.target.same_channel(&target))
            };
            if !preferred && mailbox.request.lock().unwrap_or_else(|error| error.into_inner()).is_some() { break; }
            if configured.get(&target.unverified_key()).is_some_and(|(taken, sequence, successful)|
                taken.elapsed() < CONFIGURATION_INTERVAL
                    && (!preferred || !successful || *sequence == request.sequence)) { continue; }
            let generation = *lock_shared(shared).source_channel_gen.entry((backend, group.to_string())).or_default();
            let result = read(&target, true);
            let successful = result.is_ok();
            if publish(shared, mailbox, &target, result, generation, true) {
                attempts.insert(target.unverified_key(), (Instant::now(), request.sequence, successful));
                if successful { detailed.push(target); }
            }
            notify();
        }
        configured.extend(attempts);
    }
}

fn publish(shared: &Arc<Mutex<Shared>>, mailbox: &Mailbox, target: &Target,
    result: Result<Snapshot, String>, generation: u64, detail: bool) -> bool {
    let mut state = lock_shared(shared);
    if mailbox.stopped.load(Ordering::Acquire)
        || generation != *state.source_channel_gen.get(&(target.backend,
            crate::backends::communication_group(target))).unwrap_or(&0) { return false; }
    if state.source_pending.get(&target.backend).is_some_and(|queue| queue.iter().any(|command|
        command.basic_target().is_some_and(|pending| crate::backends::communication_group(pending)
            == crate::backends::communication_group(target)))) { return false; }
    let Some(scan) = state.source_discoveries.get(&target.backend) else { return false; };
    if !scan.targets.iter().any(|current| current.same_channel(target) && same_descriptor(current, target)) { return false; }
    let mut poll = scan.presence.clone();
    match result {
        Ok(snapshot) => {
            let result_key = snapshot.target.key.clone();
            poll.snapshots.push(snapshot);
            merge_additional_update(&mut state, target.backend, poll, Instant::now(),
                if detail { Publication::Detail } else { Publication::Battery });
            let result_key = state.rapoo_devices.card_key(target).unwrap_or(&result_key);
            if let Some(key) = state.devices.get(result_key).filter(|device| device.basic.as_ref()
                .is_some_and(|snapshot| snapshot.target.same_channel(target))).map(|device| device.reading.key.clone()) {
                let pending = discovery_key(target);
                if !state.key_aliases.contains_key(&pending) {
                    if state.promote_device_key(&pending, &key) { state.device_keys_dirty = true; }
                }
            }
        }
        Err(_) => {
            let now = Instant::now();
            for device in state.devices.values_mut().filter(|device| device.basic.as_ref()
                .is_some_and(|snapshot| snapshot.target.same_channel(target))) {
                if let Some(snapshot) = &mut device.basic {
                    let configuration_only = snapshot.connected && device.battery_at.is_none()
                        && snapshot.battery.is_none() && now.saturating_duration_since(device.full_at) < CONFIGURATION_INTERVAL;
                    if detail || !configuration_only { snapshot.disable_writes(); }
                }
                if !detail {
                    if let Some(status) = &mut device.basic_status {
                        status.observe(None, None, now, target.backend.sample_lifetime());
                        device.reading.quality = status.quality;
                    }
                }
            }
            state.rebuild_readings();
        }
    }
    true
}

fn discovery_key(target: &Target) -> String {
    crate::device::stable_key(&format!("discovery:{:?}:{}", target.backend,
        crate::backends::communication_group(target)))
}

pub(super) fn pending_readings(state: &Shared) -> Vec<crate::device::DeviceReading> {
    let mut pending = Vec::new();
    for scan in state.source_discoveries.values() {
        for (_, targets) in groups(&scan.targets) {
            if targets.iter().any(|target| state.rapoo_devices.card_key(target)
                .is_some_and(|key| state.devices.contains_key(key))) { continue; }
            if state.devices.values().any(|device| device.basic.as_ref().is_some_and(|snapshot|
                targets.iter().any(|target| snapshot.target.same_channel(target)
                    || (target.backend == BackendId::StandardHid && snapshot.target.backend.precedes_standard()
                        && snapshot.connected && snapshot.target.same_hid_path(target))))) { continue; }
            let target = &targets[0];
            let receiver = targets.len() > 1 || target.hid().is_ok_and(|interface| {
                let name = interface.product.to_ascii_lowercase();
                name.contains("receiver") || name.contains("dongle") || name.contains("接收器")
            }) || crate::backends::rapoo::device_signature(target)
                .is_some_and(|signature| signature.role == crate::backends::rapoo::LinkRole::Receiver);
            let name = if receiver {
                format!("{} · {}", if target.name.is_empty() { target.backend.name() } else { &target.name },
                    crate::ui::feedback::text("接收器", "Receiver"))
            } else if target.name.is_empty() { target.backend.name().to_string() } else { target.name.clone() };
            pending.push(crate::device::DeviceReading {
                key: discovery_key(target),
                name, family: if receiver { crate::device::DeviceFamily::Unknown } else { target.family },
                connected: false, battery: None, quality: crate::device::ReadingQuality::Discovering,
            });
        }
    }
    pending
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::Poll;
    use crate::device::{Battery, DeviceFamily, Dpi, PollingRate, ReadingQuality};

    fn target(path: &str, address: u8) -> Target {
        let interface = hid::HidInterface { path: path.into(), serial: String::new(),
            vendor_id: 0x046d, product_id: 0xc548, usage_page: 0xff00, usage: 1,
            input_len: 20, output_len: 20, feature_len: 0, product: "Mouse receiver".into() };
        let mut target = Target::new(BackendId::Logitech, &interface, address);
        target.name = "Mouse".into();
        target.family = DeviceFamily::Mouse;
        target
    }

    fn sample(target: &Target, percent: u8) -> Snapshot {
        Snapshot { target: target.clone(), battery: Some(Battery { percent: Some(percent),
            estimated_percent: None, charging: Some(false), voltage_mv: None, approximate: false, level: None }),
            dpi: Dpi { current: Some(800), profile: Some(1), values: vec![400, 800],
                writable: true, ..Default::default() },
            polling_rate: PollingRate { current: Some(1000), values: vec![500, 1000], writable: true,
                ..Default::default() },
            battery_at: Some(Instant::now()), battery_unavailable: false, connected: true }
    }

    fn state(targets: Vec<Target>) -> Shared {
        let mut state = Shared::new(Settings::default());
        let backend = targets.first().map_or(BackendId::Logitech, |target| target.backend);
        state.source_discoveries.insert(backend, BackendScan {
            presence: Poll { present_channels: Some(targets.iter().map(Target::channel_key).collect()),
                ..Default::default() }, targets,
        });
        state.rebuild_readings();
        state
    }

    #[test]
    fn discovery_shows_one_receiver_without_inventing_six_online_mice() {
        let targets: Vec<_> = (1..=6).map(|address| target("receiver", address)).collect();
        let state = state(targets);
        assert_eq!(state.readings.len(), 1);
        let pending = &state.readings[0];
        assert_eq!(pending.quality, ReadingQuality::Discovering);
        assert!(!pending.connected);
        assert!(pending.display_percent().is_none());
        assert!(state.devices.is_empty());
    }

    #[test]
    fn early_battery_replaces_discovery_and_does_not_clear_other_device_settings() {
        let first = target("first", 1);
        let second = target("second", 1);
        let mut state = state(vec![first.clone(), second.clone()]);
        state.active_key = Some(discovery_key(&first));
        merge_additional(&mut state, BackendId::Logitech, vec![sample(&second, 66)].into());
        let before = state.devices[&second.key].clone();
        let state = Arc::new(Mutex::new(state));
        let mut reading = sample(&first, 75);
        reading.dpi = Default::default();
        reading.polling_rate = Default::default();
        publish(&state, &Mailbox::default(), &first, Ok(reading), 0, false);
        let state = lock_shared(&state);
        assert_eq!(state.readings.len(), 2);
        assert_eq!(state.devices[&first.key].reading.percent(), Some(75));
        assert_eq!(state.active_key.as_deref(), Some(first.key.as_str()));
        assert_eq!(state.devices[&second.key].reading, before.reading);
        assert_eq!(state.devices[&second.key].basic.as_ref().unwrap().dpi, before.basic.unwrap().dpi);
        assert!(state.devices[&second.key].basic.as_ref().unwrap().polling_rate.writable);
    }

    #[test]
    fn battery_updates_keep_same_configuration_without_extending_its_age() {
        let target = target("first", 1);
        let mut state = state(vec![target.clone()]);
        merge_additional(&mut state, BackendId::Logitech, vec![sample(&target, 66)].into());
        let full_at = state.devices[&target.key].full_at;
        let state = Arc::new(Mutex::new(state));
        let mut reading = sample(&target, 65);
        reading.dpi = Default::default();
        reading.polling_rate = Default::default();
        publish(&state, &Mailbox::default(), &target, Ok(reading), 0, false);
        let state = lock_shared(&state);
        let device = &state.devices[&target.key];
        assert_eq!(device.reading.percent(), Some(65));
        assert_eq!(device.full_at, full_at);
        assert_eq!(device.basic.as_ref().unwrap().dpi.values, vec![400, 800]);
        assert!(device.basic.as_ref().unwrap().dpi.writable);
        assert_eq!(device.basic.as_ref().unwrap().polling_rate.current, Some(1000));
    }

    #[test]
    fn changed_profile_does_not_inherit_configuration_or_writes() {
        let target = target("first", 1);
        let mut state = state(vec![target.clone()]);
        merge_additional(&mut state, BackendId::Logitech, vec![sample(&target, 66)].into());
        let state = Arc::new(Mutex::new(state));
        let mut reading = sample(&target, 65);
        reading.dpi = Dpi { current: Some(1600), profile: Some(2), ..Default::default() };
        reading.polling_rate = Default::default();
        publish(&state, &Mailbox::default(), &target, Ok(reading), 0, false);
        let state = lock_shared(&state);
        let snapshot = state.devices[&target.key].basic.as_ref().unwrap();
        assert!(snapshot.dpi.values.is_empty());
        assert!(!snapshot.dpi.writable && !snapshot.polling_rate.writable);
        assert_eq!(snapshot.dpi.current, Some(1600));
    }

    #[test]
    fn stopped_or_pre_write_results_cannot_replace_current_readings() {
        let target = target("first", 1);
        let state = Arc::new(Mutex::new(state(vec![target.clone()])));
        let mailbox = Mailbox::default();
        mailbox.stop();
        publish(&state, &mailbox, &target, Ok(sample(&target, 90)), 0, false);
        assert!(lock_shared(&state).devices.is_empty());
        lock_shared(&state).source_channel_gen.insert((BackendId::Logitech, "first".into()), 1);
        publish(&state, &Mailbox::default(), &target, Ok(sample(&target, 80)), 0, true);
        assert!(lock_shared(&state).devices.is_empty());
        publish(&state, &Mailbox::default(), &target, Ok(sample(&target, 70)), 1, false);
        assert_eq!(lock_shared(&state).devices[&target.key].reading.percent(), Some(70));
    }

    #[test]
    fn presence_scan_does_not_disable_live_configuration() {
        let target = target("first", 1);
        let mut state = state(vec![target.clone()]);
        merge_additional(&mut state, BackendId::Logitech, vec![sample(&target, 66)].into());
        let before = state.devices[&target.key].reading.clone();
        let presence = state.source_discoveries[&BackendId::Logitech].presence.clone();
        merge_additional_update(&mut state, BackendId::Logitech, presence, Instant::now(), Publication::Presence);
        assert_eq!(state.devices[&target.key].reading, before);
        assert!(state.devices[&target.key].basic.as_ref().unwrap().dpi.writable);
        merge_additional_update(&mut state, BackendId::Logitech,
            Poll { present_channels: Some(Default::default()), ..Default::default() }, Instant::now(), Publication::Presence);
        assert!(!state.devices[&target.key].reading.connected);
        assert!(!state.devices[&target.key].basic.as_ref().unwrap().dpi.writable);
    }

    #[test]
    fn receiver_reads_all_batteries_before_any_configuration() {
        let first = target("receiver", 1);
        let second = target("receiver", 2);
        let state = Arc::new(Mutex::new(state(vec![first.clone(), second.clone()])));
        let mailbox = Mailbox::default();
        mailbox.send(Request { targets: vec![first.clone(), second.clone()], sequence: 1 });
        let mut calls = Vec::new();
        run_with(BackendId::Logitech, "receiver", &mailbox, &state, &Signal::new(60),
            |target, detail| {
                calls.push((target.address, detail));
                if detail && target.address == 2 { mailbox.stop(); }
                let mut snapshot = sample(target, 75);
                if !detail { snapshot.dpi = Default::default(); snapshot.polling_rate = Default::default(); }
                Ok(snapshot)
            }, |_| panic!("no writes requested"), || ());
        assert_eq!(calls, vec![(1, false), (2, false), (1, true), (2, true)]);
        assert_eq!(lock_shared(&state).devices.len(), 2);
    }

    #[test]
    fn mailbox_coalesces_refresh_requests_instead_of_accumulating_them() {
        let mailbox = Mailbox::default();
        for sequence in 1..=100 { mailbox.send(Request { targets: Vec::new(), sequence }); }
        assert_eq!(mailbox.next().unwrap().sequence, 100);
        mailbox.stop();
        assert!(mailbox.next().is_none());
    }

    #[test]
    fn slow_configuration_on_one_channel_does_not_hold_another_channel_battery() {
        let first = target("slow", 1);
        let second = target("fast", 1);
        let state = Arc::new(Mutex::new(state(vec![first.clone(), second.clone()])));
        let slow_mailbox = Arc::new(Mailbox::default());
        let fast_mailbox = Arc::new(Mailbox::default());
        slow_mailbox.send(Request { targets: vec![first.clone()], sequence: 1 });
        fast_mailbox.send(Request { targets: vec![second.clone()], sequence: 1 });
        let (entered, waiting) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let (notified, notification) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let slow_state = Arc::clone(&state);
            let slow_box = Arc::clone(&slow_mailbox);
            let slow = scope.spawn(move || run_with(BackendId::Logitech, "slow", &slow_box,
                &slow_state, &Signal::new(60), |target, detail| {
                    if detail {
                        entered.send(()).unwrap();
                        released.recv_timeout(Duration::from_secs(5)).unwrap();
                    }
                    Ok(sample(target, 70))
                }, |_| panic!("no writes"), || ()));
            waiting.recv_timeout(Duration::from_secs(2)).unwrap();
            let fast_state = Arc::clone(&state);
            let fast_box = Arc::clone(&fast_mailbox);
            let fast = scope.spawn(move || run_with(BackendId::Logitech, "fast", &fast_box,
                &fast_state, &Signal::new(60), |target, _| Ok(sample(target, 80)),
                |_| panic!("no writes"), || { let _ = notified.send(()); }));
            let observed = notification.recv_timeout(Duration::from_secs(2));
            slow_mailbox.stop();
            fast_mailbox.stop();
            release.send(()).unwrap();
            slow.join().unwrap();
            fast.join().unwrap();
            assert!(observed.is_ok());
        });
        let state = lock_shared(&state);
        assert_eq!(state.devices[&first.key].reading.percent(), Some(70));
        assert_eq!(state.devices[&second.key].reading.percent(), Some(80));
    }

    #[test]
    fn queued_write_runs_before_reads_on_its_channel_and_leaves_other_commands_queued() {
        let first = target("first", 1);
        let second = target("second", 1);
        let mut state = state(vec![first.clone(), second.clone()]);
        state.source_pending.insert(BackendId::Logitech, [
            Cmd::BasicDpi { target: second, dpi: 400 },
            Cmd::BasicDpi { target: first.clone(), dpi: 1600 },
        ].into());
        state.source_busy.insert(BackendId::Logitech);
        let state = Arc::new(Mutex::new(state));
        let mailbox = Mailbox::default();
        mailbox.send(Request { targets: vec![first], sequence: 1 });
        let wrote = Cell::new(false);
        run_with(BackendId::Logitech, "first", &mailbox, &state, &Signal::new(60),
            |target, _| {
                assert!(wrote.get());
                mailbox.stop();
                Ok(sample(target, 70))
            }, |command| {
                assert_eq!(command.basic_target().unwrap().channel_key(), "first");
                wrote.set(true);
                CmdResult { label: command.label(), ok: Some("done".into()), err: None, hint: "" }
            }, || ());
        let state = lock_shared(&state);
        assert_eq!(state.source_pending[&BackendId::Logitech].len(), 1);
        assert!(state.source_busy.contains(&BackendId::Logitech));
        assert_eq!(state.source_running[&BackendId::Logitech], 0);
    }

    #[test]
    fn pending_write_rejects_a_read_even_if_it_started_after_queueing() {
        let target = target("first", 1);
        let mut state = state(vec![target.clone()]);
        state.source_channel_gen.insert((BackendId::Logitech, "first".into()), 1);
        state.source_pending.insert(BackendId::Logitech,
            [Cmd::BasicDpi { target: target.clone(), dpi: 1600 }].into());
        let state = Arc::new(Mutex::new(state));
        assert!(!publish(&state, &Mailbox::default(), &target, Ok(sample(&target, 70)), 1, true));
        assert!(lock_shared(&state).devices.is_empty());
    }

    fn razer_siblings() -> Vec<Target> {
        (1..=2).map(|collection| {
            let mut interface = target("unused", 0).hid().unwrap().clone();
            interface.vendor_id = 0x1532;
            interface.product_id = 0x00c1;
            interface.path = format!("hid#vid_1532&pid_00c1&mi_00&col0{collection}#unit&000{collection}#{{hid}}");
            Target::new(BackendId::Razer, &interface, 0)
        }).collect()
    }

    #[test]
    fn silent_razer_collection_does_not_hide_responding_sibling() {
        let targets = razer_siblings();
        let grouped = groups(&targets);
        assert_eq!(grouped.len(), 1);
        let (group, targets) = grouped.into_iter().next().unwrap();
        assert_eq!(targets.len(), 2);
        let state = Arc::new(Mutex::new(state(targets.clone())));
        let mailbox = Mailbox::default();
        mailbox.send(Request { targets: targets.clone(), sequence: 1 });
        let mut calls = Vec::new();
        run_with(BackendId::Razer, &group, &mailbox, &state, &Signal::new(60), |target, detail| {
            calls.push((target.channel_key(), detail));
            if target.channel_key() == targets[0].channel_key() { return Err("silent".into()); }
            if detail { mailbox.stop(); }
            Ok(sample(target, 70))
        }, |_| panic!("no writes"), || ());
        assert_eq!(calls, vec![(targets[0].channel_key(), false),
            (targets[1].channel_key(), false), (targets[1].channel_key(), true)]);
        assert_eq!(lock_shared(&state).devices[&targets[1].key].reading.percent(), Some(70));
    }

    #[test]
    fn wired_configuration_fallback_tries_sibling_after_battery_failures() {
        let targets = razer_siblings();
        let state = Arc::new(Mutex::new(state(targets.clone())));
        let mailbox = Mailbox::default();
        mailbox.send(Request { targets: targets.clone(), sequence: 1 });
        let mut calls = Vec::new();
        run_with(BackendId::Razer, &crate::backends::communication_group(&targets[0]),
            &mailbox, &state, &Signal::new(60), |target, detail| {
                calls.push((target.channel_key(), detail));
                if !detail || target.channel_key() == targets[0].channel_key() { return Err("no battery".into()); }
                let mut snapshot = sample(target, 70);
                snapshot.battery = None;
                snapshot.battery_at = None;
                Ok(snapshot)
            }, |_| panic!("no writes"), || {
                if lock_shared(&state).devices.values().any(|device|
                    device.basic.as_ref().is_some_and(|snapshot| snapshot.dpi.writable)) { mailbox.stop(); }
            });
        assert_eq!(calls, vec![(targets[0].channel_key(), false), (targets[1].channel_key(), false),
            (targets[0].channel_key(), true), (targets[1].channel_key(), true)]);
        let state = lock_shared(&state);
        assert!(state.devices[&targets[1].key].reading.connected);
        assert_eq!(state.devices[&targets[1].key].reading.percent(), None);
        assert!(state.devices[&targets[1].key].basic.as_ref().unwrap().dpi.writable);
    }

    #[test]
    fn failed_configuration_preserves_new_battery_and_other_device_controls() {
        let first = target("first", 1);
        let second = target("second", 1);
        let mut state = state(vec![first.clone(), second.clone()]);
        merge_additional(&mut state, BackendId::Logitech,
            vec![sample(&first, 66), sample(&second, 77)].into());
        let state = Arc::new(Mutex::new(state));
        assert!(publish(&state, &Mailbox::default(), &first, Err("configuration timeout".into()), 0, true));
        let state = lock_shared(&state);
        assert_eq!(state.devices[&first.key].reading.percent(), Some(66));
        assert_eq!(state.devices[&first.key].reading.quality, ReadingQuality::Live);
        assert!(!state.devices[&first.key].basic.as_ref().unwrap().dpi.writable);
        assert!(state.devices[&second.key].basic.as_ref().unwrap().dpi.writable);
    }

    #[test]
    fn stopping_a_channel_during_a_write_prevents_the_next_command_and_read() {
        let target = target("first", 1);
        let mut state = state(vec![target.clone()]);
        state.source_pending.insert(BackendId::Logitech, [
            Cmd::BasicDpi { target: target.clone(), dpi: 400 },
            Cmd::BasicDpi { target: target.clone(), dpi: 800 },
        ].into());
        let state = Arc::new(Mutex::new(state));
        let mailbox = Mailbox::default();
        mailbox.send(Request { targets: vec![target], sequence: 1 });
        let mut calls = 0;
        run_with(BackendId::Logitech, "first", &mailbox, &state, &Signal::new(60),
            |_, _| panic!("stopped channel cannot read"), |command| {
                calls += 1;
                mailbox.stop();
                CmdResult { label: command.label(), ok: Some("done".into()), err: None, hint: "" }
            }, || ());
        assert_eq!(calls, 1);
        assert_eq!(lock_shared(&state).source_pending[&BackendId::Logitech].len(), 1);
    }

    #[test]
    fn writing_another_device_does_not_discard_this_channels_battery() {
        let first = target("first", 1);
        let second = target("second", 1);
        let state = Arc::new(Mutex::new(state(vec![first.clone(), second.clone()])));
        assert!(submit(&state, &Signal::new(60), Cmd::BasicDpi { target: second, dpi: 800 }));
        assert_eq!(lock_shared(&state).source_write_gen[&BackendId::Logitech], 1);
        assert!(publish(&state, &Mailbox::default(), &first, Ok(sample(&first, 70)), 0, false));
        assert_eq!(lock_shared(&state).devices[&first.key].reading.percent(), Some(70));
    }

    #[test]
    fn changed_descriptor_on_reused_path_rejects_old_device_result() {
        let old = target("first", 1);
        let mut new = old.clone();
        new.hid_mut().serial = "replacement".into();
        let state = Arc::new(Mutex::new(state(vec![new.clone()])));
        assert!(!publish(&state, &Mailbox::default(), &old, Ok(sample(&old, 70)), 0, false));
        assert!(lock_shared(&state).devices.is_empty());
        assert!(publish(&state, &Mailbox::default(), &new, Ok(sample(&new, 80)), 0, false));
        assert_eq!(lock_shared(&state).devices[&new.key].reading.percent(), Some(80));
    }

    #[test]
    fn discovery_selection_promotes_to_current_identity_instead_of_retired_key() {
        let endpoint = target("first", 1);
        let mut old = sample(&endpoint, 60);
        old.target.key = "aaa-retired".into();
        old.target.verified_identity = true;
        let mut state = state(vec![endpoint.clone()]);
        merge_additional(&mut state, BackendId::Logitech, vec![old].into());
        state.active_key = Some(discovery_key(&endpoint));
        let state = Arc::new(Mutex::new(state));
        let mut next = sample(&endpoint, 80);
        next.target.key = "zzz-current".into();
        next.target.verified_identity = true;
        assert!(publish(&state, &Mailbox::default(), &endpoint, Ok(next), 0, false));
        let state = lock_shared(&state);
        assert_eq!(state.active_key.as_deref(), Some("zzz-current"));
        assert!(!state.devices["aaa-retired"].reading.connected);
        assert_eq!(state.devices["zzz-current"].reading.percent(), Some(80));
    }

    #[test]
    fn selected_configuration_failure_backs_off_while_battery_keeps_refreshing() {
        let target = target("first", 1);
        let mut state = state(vec![target.clone()]);
        state.detail_key = Some(target.key.clone());
        let state = Arc::new(Mutex::new(state));
        let mailbox = Mailbox::default();
        mailbox.send(Request { targets: vec![target.clone()], sequence: 1 });
        let mut light_reads = 0;
        let mut detail_reads = 0;
        run_with(BackendId::Logitech, "first", &mailbox, &state, &Signal::new(60), |target, detail| {
            if detail {
                detail_reads += 1;
                mailbox.send(Request { targets: vec![target.clone()], sequence: 2 });
                return Err("configuration timeout".into());
            }
            light_reads += 1;
            if light_reads == 2 { mailbox.send(Request { targets: vec![target.clone()], sequence: 3 }); }
            if light_reads == 3 { mailbox.stop(); }
            let mut snapshot = sample(target, 70 + light_reads);
            snapshot.dpi = Default::default();
            snapshot.polling_rate = Default::default();
            Ok(snapshot)
        }, |_| panic!("no writes"), || ());
        assert_eq!(light_reads, 3);
        assert_eq!(detail_reads, 1);
        assert_eq!(lock_shared(&state).devices[&target.key].reading.percent(), Some(72));
    }

    #[test]
    fn configuration_only_device_keeps_settings_between_light_battery_failures() {
        let mut target = target("wired", 0);
        target.backend = BackendId::Razer;
        let state = Arc::new(Mutex::new(state(vec![target.clone()])));
        let mut configuration = sample(&target, 70);
        configuration.battery = None;
        configuration.battery_at = None;
        assert!(publish(&state, &Mailbox::default(), &target, Ok(configuration), 0, true));
        assert!(publish(&state, &Mailbox::default(), &target, Err("no battery".into()), 0, false));
        {
            let mut state = lock_shared(&state);
            let device = state.devices.get_mut(&target.key).unwrap();
            assert!(device.basic.as_ref().unwrap().dpi.writable);
            assert_eq!(device.reading.percent(), None);
            device.full_at = Instant::now() - CONFIGURATION_INTERVAL;
        }
        assert!(publish(&state, &Mailbox::default(), &target, Err("no battery".into()), 0, false));
        assert!(!lock_shared(&state).devices[&target.key].basic.as_ref().unwrap().dpi.writable);
    }

    #[test]
    fn responding_receiver_slot_is_read_before_silent_slots_on_following_refresh() {
        let targets: Vec<_> = (1..=3).map(|address| target("receiver", address)).collect();
        let state = Arc::new(Mutex::new(state(targets.clone())));
        let mailbox = Mailbox::default();
        mailbox.send(Request { targets: targets.clone(), sequence: 1 });
        let mut calls = Vec::new();
        run_with(BackendId::Logitech, "receiver", &mailbox, &state, &Signal::new(60), |target, detail| {
            assert!(!detail);
            calls.push(target.address);
            if calls.len() == 4 { mailbox.stop(); }
            if target.address != 3 { return Err("empty slot".into()); }
            if calls.len() == 3 { mailbox.send(Request { targets: targets.clone(), sequence: 2 }); }
            Ok(sample(target, 70))
        }, |_| panic!("no writes"), || ());
        assert_eq!(calls, vec![1, 2, 3, 3]);
    }
}
