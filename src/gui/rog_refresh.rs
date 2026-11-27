use super::*;
use crate::backends::BackendId;
use crate::device::{DeviceFamily, DeviceReading, ReadingQuality};
use std::collections::BTreeSet;

pub(super) fn run(
    shared: &Arc<Mutex<Shared>>,
    full: bool,
    generation: u64,
    notice: nwg::NoticeSender,
) -> Option<Quick> {
    let mut physical_groups = BTreeMap::new();
    let scan = crate::win32::HidApi::load().and_then(|api| {
        hid::scan_links_with_discovery(&api, |report, groups| {
            physical_groups.clone_from(groups);
            {
                let mut state = lock_shared(shared);
                state.discovery_pending.remove(&BackendId::Rog);
                if !outcome_is_stale(generation, state.write_gen) {
                    update_discoveries(&mut state, report, groups);
                    state.rebuild_readings();
                }
            }
            notice.notice();
        })
    });
    refresh_parallel_with(shared, scan, &physical_groups, full, generation,
        crate::backends::rog::fetch, || notice.notice())
}

type EndpointRank = (bool, bool, String, u8);

struct RefreshCycle {
    links: Vec<hid::Link>,
    preferred: BTreeSet<String>,
    known_endpoints: Vec<(proto::Identity, String)>,
    selected: Mutex<Vec<(proto::Identity, EndpointRank)>>,
    published: Mutex<BTreeSet<String>>,
    completed: Mutex<BTreeSet<String>>,
    completion: Condvar,
}

impl RefreshCycle {
    fn new(state: &mut Shared, scan: Option<&hid::LinkScan>, generation: u64) -> Self {
        let available = scan.into_iter().flat_map(|scan| &scan.links).map(hid::Link::device_key).collect::<BTreeSet<_>>();
        let known_endpoints = state.devices.values().filter(|record| record.basic.is_none()
            && record.reading.connected && record.ident.device_id().is_some())
            .filter_map(|record| record.link.as_ref().map(|link| (record.ident.clone(), link.device_key())))
            .filter(|(_, key)| available.contains(key)).collect::<Vec<_>>();
        let preferred = known_endpoints.iter().map(|(_, key)| key.clone()).collect();
        state.discovery_pending.remove(&BackendId::Rog);
        if let Some(scan) = scan.filter(|_| !outcome_is_stale(generation, state.write_gen)) {
            let lifetime = rog_sample_lifetime(&state.cfg);
            for record in state.devices.values_mut().filter(|record| record.basic.is_none()
                && record.link.as_ref().is_none_or(|link| !available.contains(&link.device_key()))) {
                mark_rog_unread(record, Some(&scan.present_channels), std::time::Instant::now(), lifetime);
            }
            sync_unread_primary(state);
            state.rebuild_readings();
        }
        Self { links: scan.map(|scan| scan.links.clone()).unwrap_or_default(), preferred, known_endpoints,
            selected: Mutex::new(Vec::new()), published: Mutex::new(BTreeSet::new()),
            completed: Mutex::new(BTreeSet::new()), completion: Condvar::new() }
    }

    fn rank(&self, link: &hid::Link) -> EndpointRank {
        (!self.preferred.contains(&link.device_key()), link.mode != "Wired",
            link.iface.path.to_ascii_lowercase(), link.report_id)
    }

    fn accepts(&self, link: &hid::Link, identity: &proto::Identity, detail: bool, measured: bool) -> bool {
        if identity.device_id().is_none() { return true; }
        let rank = self.rank(link);
        let mut selected = self.selected.lock().unwrap_or_else(|error| error.into_inner());
        if let Some((known, previous)) = selected.iter_mut().find(|(known, _)| known.same_device(identity)) {
            if rank > *previous || (rank == *previous && !detail) { return false; }
            if measured {
                *known = identity.clone();
                *previous = rank;
            }
        } else if measured {
            selected.push((identity.clone(), rank));
        }
        true
    }

    fn await_preferred(&self, link: &hid::Link, identity: &proto::Identity) {
        if self.preferred.contains(&link.device_key()) { return; }
        let Some((_, preferred)) = self.known_endpoints.iter().find(|(known, preferred)|
            known.same_device(identity) && preferred != &link.device_key()) else { return; };
        let mut completed = self.completed.lock().unwrap_or_else(|error| error.into_inner());
        while !completed.contains(preferred) {
            completed = self.completion.wait(completed).unwrap_or_else(|error| error.into_inner());
        }
    }

    fn complete(&self, links: impl IntoIterator<Item = String>) {
        self.completed.lock().unwrap_or_else(|error| error.into_inner()).extend(links);
        self.completion.notify_all();
    }
}

struct GroupCompletion<'a> {
    cycle: &'a RefreshCycle,
    links: Vec<String>,
}

impl std::ops::Drop for GroupCompletion<'_> {
    fn drop(&mut self) { self.cycle.complete(self.links.drain(..)); }
}

fn communication_group(interface: &hid::HidInterface) -> String {
    crate::backends::communication_group(&crate::backends::Target::new(BackendId::Rog, interface, 0))
}

fn grouped_links(links: &[hid::Link], physical: &BTreeMap<String, String>) -> Vec<Vec<hid::Link>> {
    let mut groups: Vec<(BTreeSet<String>, Vec<hid::Link>)> = Vec::new();
    for link in links {
        let mut channels = std::iter::once(&link.iface).chain(&link.all_interfaces).map(|interface| {
            physical.get(&interface.path.to_ascii_lowercase())
                .map(|group| format!("physical:{group}"))
                .unwrap_or_else(|| communication_group(interface))
        }).collect::<BTreeSet<_>>();
        let mut members = vec![link.clone()];
        let mut index = 0;
        while index < groups.len() {
            if !groups[index].0.is_disjoint(&channels) {
                let (related, links) = groups.remove(index);
                channels.extend(related);
                members.extend(links);
                index = 0;
            } else { index += 1; }
        }
        groups.push((channels, members));
    }
    groups.into_iter().map(|(_, members)| members).collect()
}

fn refresh_parallel_with(
    shared: &Arc<Mutex<Shared>>,
    scan: Result<hid::LinkScan, String>,
    physical: &BTreeMap<String, String>,
    full: bool,
    generation: u64,
    fetch: impl Fn(&hid::Link, bool) -> Result<FetchOutcome, String> + Sync,
    notify: impl Fn() + Sync,
) -> Option<Quick> {
    let links = scan.as_ref().map(|scan| scan.links.clone()).unwrap_or_default();
    let presence = scan.as_ref().map(|scan| scan.present_channels.clone()).unwrap_or_default();
    let cycle = RefreshCycle::new(&mut lock_shared(shared), scan.as_ref().ok(), generation);
    if let Err(error) = scan {
        return refresh_group_with(shared, Err(error), full, generation, &cycle, fetch, notify);
    }
    notify();
    let mut groups = grouped_links(&links, physical);
    for group in &mut groups { group.sort_by_key(|link| cycle.rank(link)); }
    std::thread::scope(|scope| {
        for links in groups {
            let cycle = &cycle;
            let fetch = &fetch;
            let notify = &notify;
            let present_channels = presence.clone();
            scope.spawn(move || {
                refresh_group_with(shared, Ok(hid::LinkScan { links, present_channels }),
                    full, generation, cycle, fetch, notify);
            });
        }
    });
    let state = lock_shared(shared);
    if outcome_is_stale(generation, state.write_gen) { return None; }
    let published = cycle.published.lock().unwrap_or_else(|error| error.into_inner());
    state.primary_key.as_ref().filter(|key| published.contains(*key)).and(state.quick)
}

fn discovery_key(interface: &hid::HidInterface) -> String {
    crate::device::stable_key(&format!("rog-discovery:{}", interface.path.to_ascii_lowercase()))
}

fn update_discoveries(
    state: &mut Shared,
    report: &hid::EnumerateReport,
    groups: &BTreeMap<String, String>,
) {
    let mut candidates = report.interfaces.iter().filter(|interface| {
        interface.vendor_id == hid::VID_ASUS && interface.is_command_channel()
            && (crate::devices::find_dongle(interface.product_id).is_some()
                || crate::devices::find(interface.product_id).is_some())
    }).cloned().collect::<Vec<_>>();
    candidates.extend(state.rog_discoveries.iter().filter(|previous| {
        report.skipped.iter().any(|skipped| skipped.path.eq_ignore_ascii_case(&previous.path))
    }).cloned());
    candidates.sort_by_key(|interface| interface.path.to_ascii_lowercase());
    let mut representatives = BTreeMap::new();
    for interface in candidates {
        let group = groups.get(&interface.path.to_ascii_lowercase()).cloned()
            .unwrap_or_else(|| communication_group(&interface));
        representatives.entry((interface.vendor_id, interface.product_id, group)).or_insert(interface);
    }
    state.rog_discoveries = representatives.into_values().collect();
}

fn link_contains(link: &hid::Link, interface: &hid::HidInterface) -> bool {
    std::iter::once(&link.iface).chain(&link.all_interfaces)
        .any(|member| member.path.eq_ignore_ascii_case(&interface.path))
}

fn single_endpoint(links: &[hid::Link], link: &hid::Link) -> bool {
    links.iter().filter(|candidate| std::iter::once(&link.iface).chain(&link.all_interfaces)
        .any(|interface| link_contains(candidate, interface))).count() == 1
}

pub(super) fn pending_readings(state: &Shared) -> Vec<DeviceReading> {
    state.rog_discoveries.iter().filter(|interface| {
        !state.devices.values().any(|record| record.link.as_ref()
            .is_some_and(|link| link_contains(link, interface)))
            && !state.key_aliases.get(&discovery_key(interface)).is_some_and(|key| state.devices.contains_key(key))
    }).map(|interface| {
        let model = crate::devices::find(interface.product_id);
        let family = model.map(|model| match model.kind {
            crate::devices::Kind::Mouse => DeviceFamily::Mouse,
            crate::devices::Kind::Keyboard => DeviceFamily::Keyboard,
            crate::devices::Kind::Gamepad => DeviceFamily::Gamepad,
        }).unwrap_or(DeviceFamily::Unknown);
        DeviceReading {
            key: discovery_key(interface),
            name: crate::devices::display_name(&interface.product, interface.product_id, 0),
            family, battery: None, connected: false, quality: ReadingQuality::Discovering,
        }
    }).collect()
}

fn promote(state: &mut Shared, old: &str, new: &str) {
    if state.detail_key.as_deref() == Some(old) { state.detail_key = Some(new.into()); }
    if state.promote_device_key(old, new) { state.device_keys_dirty = true; }
}

fn sync_unread_primary(state: &mut Shared) {
    let Some(primary) = state.primary_key.as_ref().and_then(|key| state.devices.get(key)) else { return; };
    if primary.basic.is_some() || (primary.reading.connected && primary.reading.quality == ReadingQuality::Live) {
        return;
    }
    state.full = None;
    state.quick = None;
    state.status = if primary.reading.connected { Status::Asleep }
        else { Status::Error(crate::tr!("所选设备已断开，请重新连接").into()) };
}

fn publish_outcome(
    state: &mut Shared,
    link: &hid::Link,
    outcome: &FetchOutcome,
    generation: u64,
    single_endpoint: bool,
) -> Option<String> {
    if outcome_is_stale(generation, state.write_gen) { return None; }
    let link_key = link.device_key();
    let previous = state.devices.values().filter(|record| record.link.as_ref()
        .is_some_and(|previous| previous.device_key() == link_key))
        .max_by_key(|record| (record.reading.connected, record.battery_at)).cloned();
    let previous = reconcile_rog_identity(&mut state.devices, link, &outcome.snapshot.identity).or(previous);
    let mut reusable = previous.clone();
    if let Some(previous) = &mut reusable {
        let endpoint_changed = previous.link.as_ref().is_none_or(|old| old.device_key() != link_key);
        let profile_changed = outcome.snapshot.parsed_frame("12:00:00")
            && previous.state.as_ref().is_some_and(|old| old.profile != outcome.snapshot.state.profile);
        if endpoint_changed || profile_changed {
            previous.state = None;
            if previous.ident.same_device(&outcome.snapshot.identity) {
                if let Some(stored) = state.devices.get_mut(&previous.reading.key) { stored.state = None; }
                if state.primary_key.as_ref() == Some(&previous.reading.key) { state.full = None; }
            }
        }
    }
    let record = rog_record(link, outcome, reusable.as_ref());
    let Some(record) = record else {
        sync_unread_primary(state);
        state.rebuild_readings();
        return None;
    };
    let key = record.reading.key.clone();
    if previous.is_none() && key != link_key && !state.key_aliases.contains_key(&link_key) {
        promote(state, &link_key, &key);
    }
    if let Some(old) = previous.as_ref().filter(|old| old.reading.key != key
        && (old.ident.device_id().is_none() || old.ident.same_device(&record.ident))) {
        state.devices.remove(&old.reading.key);
        promote(state, &old.reading.key, &key);
    }
    if single_endpoint {
        let pending = state.rog_discoveries.iter().filter(|interface| link_contains(link, interface))
            .map(discovery_key).collect::<Vec<_>>();
        for old in pending {
            if !state.key_aliases.contains_key(&old) { promote(state, &old, &key); }
        }
    }
    if outcome.snapshot.configuration().is_ok() && !record.ident.model_name.trim().is_empty() {
        let device = diagnostic_device(&record);
        crate::diagnostics::record_device_once(&diagnostic_facts("device_identified", "设备已识别",
            record.reading.family.noun(), outcome.frames_read, outcome.elapsed.as_millis(), Some(&device)));
    }
    if state.primary_key.as_ref().is_none_or(|primary| primary == &key) {
        apply_rog_outcome(outcome, &record, state, generation);
    }
    state.devices.insert(key.clone(), record);
    sync_unread_primary(state);
    state.rebuild_readings();
    Some(key)
}

fn log_read_error(link: &hid::Link, error: &str, detail: bool) {
    let device = diagnostic_link(link);
    crate::diagnostics::record_device_once(&diagnostic_facts(
        if detail { "device_detail_failed" } else { "device_open_failed" }, error,
        DeviceFamily::Unknown.noun(), 0, 0, Some(&device)));
}

fn publish_selected(
    state: &mut Shared, link: &hid::Link, outcome: &FetchOutcome,
    generation: u64, detail: bool, cycle: &RefreshCycle,
) -> Option<String> {
    if outcome_is_stale(generation, state.write_gen) { return None; }
    if !cycle.accepts(link, &outcome.snapshot.identity, detail, outcome.snapshot.battery().is_some()) {
        if let Some(record) = reconcile_rog_identity(&mut state.devices, link, &outcome.snapshot.identity) {
            if single_endpoint(&cycle.links, link) {
                let pending = state.rog_discoveries.iter().filter(|interface| link_contains(link, interface))
                    .map(discovery_key).collect::<Vec<_>>();
                for old in pending {
                    if !state.key_aliases.contains_key(&old) { promote(state, &old, &record.reading.key); }
                }
            }
        }
        sync_unread_primary(state);
        state.rebuild_readings();
        return None;
    }
    let key = publish_outcome(state, link, outcome, generation, single_endpoint(&cycle.links, link))?;
    cycle.published.lock().unwrap_or_else(|error| error.into_inner()).insert(key.clone());
    Some(key)
}

fn refresh_group_with(
    shared: &Arc<Mutex<Shared>>,
    scan: Result<hid::LinkScan, String>,
    full: bool,
    generation: u64,
    cycle: &RefreshCycle,
    mut fetch: impl FnMut(&hid::Link, bool) -> Result<FetchOutcome, String>,
    mut notify: impl FnMut(),
) -> Option<Quick> {
    {
        let mut state = lock_shared(shared);
        state.discovery_pending.remove(&BackendId::Rog);
    }
    notify();
    let scan = match scan {
        Ok(scan) => scan,
        Err(error) => {
            crate::diagnostics::event_once("link_enumeration_failed", &error);
            {
                let mut state = lock_shared(shared);
                if outcome_is_stale(generation, state.write_gen) { return None; }
                let lifetime = rog_sample_lifetime(&state.cfg);
                for record in state.devices.values_mut().filter(|record| record.basic.is_none()) {
                    mark_rog_unread(record, None, std::time::Instant::now(), lifetime);
                }
                state.quick = None;
                state.full = None;
                state.status = Status::Error(error);
                state.rebuild_readings();
            }
            notify();
            return None;
        }
    };
    let _completion = GroupCompletion { cycle, links: scan.links.iter().map(hid::Link::device_key).collect() };
    let mut links = scan.links.clone();
    links.sort_by_key(|link| cycle.rank(link));
    let mut seen = BTreeSet::new();
    for link in &links {
        if outcome_is_stale(generation, lock_shared(shared).write_gen) { return None; }
        let outcome = fetch(link, false);
        if let Ok(outcome) = &outcome { cycle.await_preferred(link, &outcome.snapshot.identity); }
        {
            let mut state = lock_shared(shared);
            if outcome_is_stale(generation, state.write_gen) { return None; }
            match outcome {
                Ok(outcome) => {
                    if let Some(key) = publish_selected(&mut state, link, &outcome, generation, false, cycle) {
                        seen.insert(key);
                    }
                }
                Err(error) => log_read_error(link, &error, false),
            }
        }
        cycle.complete(std::iter::once(link.device_key()));
        notify();
    }
    let mut details = {
        let mut state = lock_shared(shared);
        if outcome_is_stale(generation, state.write_gen) { return None; }
        let lifetime = rog_sample_lifetime(&state.cfg);
        for record in state.devices.values_mut().filter(|record|
            record.basic.is_none() && !seen.contains(&record.reading.key)
                && record.link.as_ref().is_some_and(|link|
                    scan.links.iter().any(|owned| owned.device_key() == link.device_key()))) {
            mark_rog_unread(record, Some(&scan.present_channels), std::time::Instant::now(), lifetime);
        }
        sync_unread_primary(&mut state);
        state.rebuild_readings();
        seen.iter().filter_map(|key| {
            let record = state.devices.get(key)?;
            let link = record.link.as_ref()?;
            (scan.links.iter().any(|owned| owned.device_key() == link.device_key())
                && record.reading.connected && !link.read_only
                && (record.state.is_none() || (full && record.full_at.elapsed() >= Duration::from_secs(state.cfg.cache_ttl_sec))))
                .then(|| (key.clone(), link.clone()))
        }).collect::<Vec<_>>()
    };
    notify();
    while !details.is_empty() {
        let (key, link) = {
            let state = lock_shared(shared);
            if outcome_is_stale(generation, state.write_gen) { return None; }
            let selected = state.detail_key.as_ref().or(state.active_key.as_ref());
            let position = details.iter().position(|(key, _)| selected == Some(key)).unwrap_or(0);
            details.remove(position)
        };
        if lock_shared(shared).devices.get(&key).and_then(|record| record.link.as_ref())
            .is_none_or(|current| current.device_key() != link.device_key()) { continue; }
        let outcome = fetch(&link, true);
        {
            let mut state = lock_shared(shared);
            if outcome_is_stale(generation, state.write_gen) { return None; }
            match outcome {
                Ok(outcome) => {
                    if let Some(updated) = publish_selected(&mut state, &link, &outcome, generation, true, cycle) {
                        seen.remove(&key);
                        seen.insert(updated);
                    }
                }
                Err(error) => log_read_error(&link, &error, true),
            }
        }
        notify();
    }
    let state = lock_shared(shared);
    if outcome_is_stale(generation, state.write_gen) { return None; }
    state.primary_key.as_ref().filter(|key| seen.contains(*key)).and(state.quick)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refresh_with(
        shared: &Arc<Mutex<Shared>>, scan: Result<hid::LinkScan, String>, full: bool, generation: u64,
        fetch: impl FnMut(&hid::Link, bool) -> Result<FetchOutcome, String>, notify: impl FnMut(),
    ) -> Option<Quick> {
        let cycle = RefreshCycle::new(&mut lock_shared(shared), scan.as_ref().ok(), generation);
        refresh_group_with(shared, scan, full, generation, &cycle, fetch, notify)
    }

    fn interface(path: &str) -> hid::HidInterface {
        hid::HidInterface {
            path: path.into(), vendor_id: hid::VID_ASUS, product_id: hid::PID_MOUSE_WIRED,
            usage_page: hid::USAGE_PAGE_MOUSE_CTRL, usage: 1, input_len: 64, output_len: 64,
            feature_len: 0, product: "ROG HARPE ACE MINI".into(), serial: "same-descriptor".into(),
        }
    }

    fn link(path: &str) -> hid::Link {
        let iface = interface(path);
        hid::Link { mode: "Wired".into(), report_id: 1, read_only: false,
            iface: iface.clone(), all_interfaces: vec![iface] }
    }

    fn scan(links: Vec<hid::Link>) -> Result<hid::LinkScan, String> {
        Ok(hid::LinkScan { present_channels: links.iter().flat_map(|link|
            std::iter::once(&link.iface).chain(&link.all_interfaces))
            .map(|interface| interface.path.to_ascii_lowercase()).collect(), links })
    }

    fn outcome(link: &hid::Link, identity: &str, percent: u8, detail: bool) -> FetchOutcome {
        let mut outcome = snapshot::fetch_light_with(link.report_id, &link.mode, |report, command, sub, _| {
            let mut frame = vec![0; 64];
            frame[..3].copy_from_slice(&[report, command, sub]);
            match sub {
                7 => { frame[5] = percent; frame[7] = 20; }
                4 => { frame[13] = 3; frame[15] = 2; }
                0 => { frame[11] = 1; frame[12] = 2; }
                _ => return None,
            }
            Some(frame)
        });
        let snapshot = Arc::make_mut(&mut outcome.snapshot);
        snapshot.identity.mouse_sn = identity.into();
        snapshot.identity.wired_pid = hid::PID_MOUSE_WIRED;
        if detail {
            for (key, sub) in [("12:04:02", 4), ("12:04:03", 4),
                ("12:06:00", 6), ("12:03:00", 3), ("12:05:00", 5)] {
                let mut frame = vec![0; 64];
                frame[..3].copy_from_slice(&[link.report_id, 0x12, sub]);
                frame[5] = 7;
                snapshot.frames.insert(key.into(), frame);
            }
            snapshot.state = snapshot.configuration().unwrap().state().clone();
        }
        outcome.frames_read = snapshot.frames.len();
        outcome
    }

    fn shared() -> Arc<Mutex<Shared>> {
        Arc::new(Mutex::new(Shared::new(Settings::default())))
    }

    fn seed(shared: &Arc<Mutex<Shared>>, link: &hid::Link, identity: &str) -> String {
        let mut state = lock_shared(shared);
        publish_outcome(&mut state, link, &outcome(link, identity, 47, true), 0, true).unwrap()
    }

    #[test]
    fn discovered_receivers_are_one_placeholder_per_hardware_without_samples() {
        let mut state = Shared::new(Settings::default());
        let mut interfaces = Vec::new();
        let mut groups = BTreeMap::new();
        for receiver in ["first", "second"] {
            for slot in 0..6 {
                let path = format!("{receiver}-{slot}");
                let mut iface = interface(&path);
                iface.product_id = hid::PID_DONGLE;
                groups.insert(path, receiver.into());
                interfaces.push(iface);
            }
        }
        let mut unsupported = interface("unsupported");
        unsupported.product_id = 0xFFFF;
        interfaces.push(unsupported);
        update_discoveries(&mut state, &hid::EnumerateReport { interfaces, skipped: vec![] }, &groups);
        let pending = pending_readings(&state);
        assert_eq!(pending.len(), 2);
        assert_ne!(pending[0].key, pending[1].key);
        for reading in pending {
            assert_eq!(reading.family, DeviceFamily::Unknown);
            assert!(reading.is_discovering());
            assert!(!reading.connected);
            assert!(reading.battery.is_none());
            assert_eq!(reading.battery_text(true), "—");
            assert!(reading.name.contains("接收器"));
        }
        state.rebuild_readings();
        assert!(state.devices.is_empty());
        assert!(state.history_tracker.take_pending().0.is_empty());
    }

    #[test]
    fn discovered_placeholder_hides_on_any_link_member_and_survives_probe_failure() {
        let shared = shared();
        let mut receiver = link("mouse");
        receiver.all_interfaces.push(interface("control"));
        {
            let mut state = lock_shared(&shared);
            state.rog_discoveries = vec![interface("CONTROL"), interface("independent")];
        }
        refresh_with(&shared, scan(vec![receiver.clone()]), false, 0,
            |_, _| Err("unresponsive".into()), || {});
        assert_eq!(pending_readings(&lock_shared(&shared)).len(), 2);
        seed(&shared, &receiver, "verified");
        let state = lock_shared(&shared);
        assert_eq!(pending_readings(&state).len(), 1);
        assert_eq!(pending_readings(&state)[0].key, discovery_key(&interface("independent")));
    }

    #[test]
    fn discovery_refresh_preserves_blocked_interfaces_but_removes_absent_ones() {
        let mut state = Shared::new(Settings::default());
        state.rog_discoveries = vec![interface("blocked"), interface("removed")];
        update_discoveries(&mut state, &hid::EnumerateReport { interfaces: vec![],
            skipped: vec![hid::Skipped { path: "BLOCKED".into(), reason: "busy".into() }] }, &BTreeMap::new());
        assert_eq!(state.rog_discoveries.len(), 1);
        assert_eq!(state.rog_discoveries[0].path, "blocked");
        update_discoveries(&mut state, &hid::EnumerateReport { interfaces: vec![], skipped: vec![] }, &BTreeMap::new());
        assert!(pending_readings(&state).is_empty());
    }

    #[test]
    fn all_catalogued_receiver_pids_are_discovered_without_claiming_online_peripherals() {
        let mut state = Shared::new(Settings::default());
        let interfaces = crate::devices::DONGLES.iter().map(|dongle| {
            let mut interface = interface(&format!("receiver-{:04x}", dongle.pid));
            interface.product_id = dongle.pid;
            interface
        }).collect();
        update_discoveries(&mut state, &hid::EnumerateReport { interfaces, skipped: vec![] }, &BTreeMap::new());
        let pending = pending_readings(&state);
        assert_eq!(pending.len(), crate::devices::DONGLES.len());
        assert!(pending.iter().all(|reading| reading.family == DeviceFamily::Unknown
            && !reading.connected && reading.battery.is_none() && reading.is_discovering()));
    }

    #[test]
    fn every_light_publishes_before_next_read_and_details_follow_selected_device() {
        let shared = shared();
        let first = link("first");
        let second = link("second");
        let first_key = crate::backends::rog::identity_key(&first, &outcome(&first, "first", 47, false).snapshot.identity);
        let second_key = crate::backends::rog::identity_key(&second, &outcome(&second, "second", 47, false).snapshot.identity);
        let calls = RefCell::new(Vec::new());
        let notices = Cell::new(0);
        {
            let mut state = lock_shared(&shared);
            state.rog_discoveries = vec![first.iface.clone(), second.iface.clone()];
            state.active_key = Some(discovery_key(&second.iface));
            state.detail_key = state.active_key.clone();
        }
        refresh_with(&shared, scan(vec![first, second]), false, 0, |link, detail| {
            let state = lock_shared(&shared);
            assert!(!state.discovery_pending.contains(&BackendId::Rog));
            if link.iface.path == "second" && !detail {
                assert_eq!(state.devices[&first_key].reading.percent(), Some(41));
                assert!(state.devices[&first_key].state.is_none());
                assert!(notices.get() >= 2);
            }
            if detail {
                assert_eq!(state.devices.len(), 2);
                assert!(state.devices.values().all(|record| record.reading.percent() == Some(41)));
                assert_eq!(state.detail_key.as_ref(), Some(&second_key));
            }
            calls.borrow_mut().push((link.iface.path.clone(), detail));
            Ok(outcome(link, &link.iface.path, 41, detail))
        }, || notices.set(notices.get() + 1));
        assert_eq!(*calls.borrow(), vec![("first".into(), false), ("second".into(), false),
            ("second".into(), true), ("first".into(), true)]);
        let state = lock_shared(&shared);
        assert!(state.devices.values().all(|record| record.state.is_some()));
        assert!(pending_readings(&state).is_empty());
    }

    #[test]
    fn light_and_failed_detail_preserve_configuration_and_measurement_time() {
        for missing_energy in [false, true] {
            let shared = shared();
            let link = link("mouse");
            let key = seed(&shared, &link, "verified");
            let original_full_at = lock_shared(&shared).devices[&key].full_at;
            let measured = std::time::Instant::now() - Duration::from_secs(2);
            lock_shared(&shared).cfg.cache_ttl_sec = 0;
            refresh_with(&shared, scan(vec![link]), true, 0, |link, detail| {
                if detail {
                    let mut state = lock_shared(&shared);
                    assert_eq!(state.devices[&key].reading.percent(), Some(42));
                    assert_eq!(state.devices[&key].state.as_ref().unwrap().enc[0], 7);
                    state.history_tracker.take_pending();
                    if !missing_energy { return Err("detail failed".into()); }
                    let mut failed = outcome(link, "verified", 0, false);
                    Arc::make_mut(&mut failed.snapshot).frames.remove("12:07:00");
                    return Ok(failed);
                }
                let mut result = outcome(link, "verified", 42, false);
                Arc::make_mut(&mut result.snapshot).taken = Some(measured);
                Ok(result)
            }, || {});
            let mut state = lock_shared(&shared);
            let record = &state.devices[&key];
            assert_eq!(record.reading.percent(), Some(42));
            assert_eq!(record.reading.quality, ReadingQuality::Live);
            assert_eq!(record.battery_at, Some(measured));
            assert_eq!(record.full_at, original_full_at);
            assert_eq!(record.state.as_ref().unwrap().enc[0], 7);
            assert_eq!(state.full.as_ref().unwrap().enc[0], 7);
            assert!(state.history_tracker.take_pending().0.is_empty());
        }
    }

    #[test]
    fn failed_light_only_disables_its_own_configuration_and_keeps_presence() {
        let shared = shared();
        let first = link("first");
        let second = link("second");
        let first_key = seed(&shared, &first, "first");
        let second_key = seed(&shared, &second, "second");
        let prior_sample = lock_shared(&shared).devices[&first_key].battery_at;
        refresh_with(&shared, scan(vec![first, second]), false, 0, |link, detail| {
            assert!(!detail);
            if link.iface.path == "first" { Err("sleep".into()) }
            else { Ok(outcome(link, "second", 88, false)) }
        }, || {});
        let state = lock_shared(&shared);
        assert_eq!(state.devices[&first_key].battery_at, prior_sample);
        assert!(state.devices[&first_key].reading.connected);
        assert_eq!(state.devices[&first_key].reading.quality, ReadingQuality::Cached);
        assert!(state.devices[&first_key].state.is_none());
        assert_eq!(state.devices[&second_key].reading.percent(), Some(88));
        assert!(state.devices[&second_key].state.is_some());
    }

    #[test]
    fn absence_disconnects_and_scan_error_ends_loading_without_claiming_removal() {
        for failed in [false, true] {
            let shared = shared();
            let link = link("mouse");
            let key = seed(&shared, &link, "verified");
            let scan = if failed { Err("enumeration failed".into()) } else { scan(vec![]) };
            refresh_with(&shared, scan, false, 0, |_, _| panic!("no links"), || {});
            let state = lock_shared(&shared);
            assert!(!state.discovery_pending.contains(&BackendId::Rog));
            assert_eq!(state.devices[&key].reading.connected, failed);
            assert!(state.devices[&key].state.is_none());
            assert!(state.quick.is_none());
        }
    }

    #[test]
    fn writes_during_light_or_detail_discard_inflight_results_and_stop_more_reads() {
        for write_during_detail in [false, true] {
            let shared = shared();
            let calls = Cell::new(0);
            let first = link("first");
            let second = link("second");
            let result = refresh_with(&shared, scan(vec![first, second]), false, 0, |link, detail| {
                calls.set(calls.get() + 1);
                if detail == write_during_detail { lock_shared(&shared).write_gen += 1; }
                Ok(outcome(link, &link.iface.path, if detail { 99 } else { 41 }, detail))
            }, || {});
            assert!(result.is_none());
            let state = lock_shared(&shared);
            assert_eq!(calls.get(), if write_during_detail { 3 } else { 1 });
            assert_eq!(state.devices.len(), if write_during_detail { 2 } else { 0 });
            assert!(state.devices.values().all(|record| record.reading.percent() == Some(41) && record.state.is_none()));
        }
    }

    #[test]
    fn verified_identity_cannot_downgrade_and_replacement_retires_old_controls() {
        for replacement in ["", "new-device"] {
            let shared = shared();
            let link = link("mouse");
            let key = seed(&shared, &link, "old-device");
            let prior_sample = lock_shared(&shared).devices[&key].battery_at;
            refresh_with(&shared, scan(vec![link]), false, 0, |link, detail| {
                assert!(!detail);
                let mut result = outcome(link, replacement, 99, false);
                if !replacement.is_empty() { Arc::make_mut(&mut result.snapshot).frames.remove("12:07:00"); }
                Ok(result)
            }, || {});
            let state = lock_shared(&shared);
            assert_eq!(state.devices.len(), 1);
            assert_eq!(state.devices[&key].battery_at, prior_sample);
            assert_eq!(state.devices[&key].ident.mouse_sn, "old-device");
            assert_eq!(state.devices[&key].reading.connected, replacement.is_empty());
            assert!(state.devices[&key].state.is_none());
            assert!(state.full.is_none());
            assert!(state.quick.is_none());
        }
    }

    #[test]
    fn profile_change_invalidates_old_configuration_before_detail_read() {
        let shared = shared();
        let link = link("mouse");
        let key = seed(&shared, &link, "verified");
        let calls = Cell::new(0);
        refresh_with(&shared, scan(vec![link]), false, 0, |link, detail| {
            calls.set(calls.get() + 1);
            if detail {
                let state = lock_shared(&shared);
                assert_eq!(state.devices[&key].reading.percent(), Some(40));
                assert!(state.devices[&key].state.is_none());
                assert!(state.full.is_none());
                return Err("optional configuration timed out".into());
            }
            let mut result = outcome(link, "verified", 40, false);
            let snapshot = Arc::make_mut(&mut result.snapshot);
            snapshot.frames.get_mut("12:00:00").unwrap()[11] = 2;
            snapshot.state.profile = 2;
            Ok(result)
        }, || {});
        assert_eq!(calls.get(), 2);
        let state = lock_shared(&shared);
        assert_eq!(state.devices[&key].reading.percent(), Some(40));
        assert!(state.devices[&key].state.is_none());
    }

    #[test]
    fn readonly_links_skip_detail_while_unverified_links_can_complete_identification() {
        for read_only in [false, true] {
            let shared = shared();
            let mut link = link("mouse");
            link.read_only = read_only;
            let calls = Cell::new(0);
            refresh_with(&shared, scan(vec![link]), true, 0, |link, detail| {
                if read_only { assert!(!detail); }
                calls.set(calls.get() + 1);
                Ok(outcome(link, if read_only || detail { "verified" } else { "" }, 0, detail))
            }, || {});
            assert_eq!(calls.get(), if read_only { 1 } else { 2 });
            let state = lock_shared(&shared);
            let record = state.devices.values().next().unwrap();
            assert_eq!(record.reading.percent(), Some(0));
            assert_eq!(record.state.is_none(), read_only);
        }
    }

    #[test]
    fn multichannel_receiver_never_migrates_receiver_selection_to_arbitrary_slot() {
        let shared = shared();
        let mut first = link("receiver");
        first.iface.product_id = hid::PID_DONGLE;
        let mut second = first.clone();
        second.report_id = 3;
        let placeholder = discovery_key(&first.iface);
        {
            let mut state = lock_shared(&shared);
            state.rog_discoveries = vec![first.iface.clone()];
            state.active_key = Some(placeholder.clone());
        }
        refresh_with(&shared, scan(vec![first, second]), false, 0,
            |link, detail| Ok(outcome(link, &format!("device-{}", link.report_id), 42, detail)), || {});
        let state = lock_shared(&shared);
        assert_eq!(state.devices.len(), 2);
        assert!(pending_readings(&state).is_empty());
        assert!(!state.key_aliases.contains_key(&placeholder));
        assert_eq!(state.active_key.as_ref(), Some(&placeholder));
    }

    #[test]
    fn independent_receivers_publish_light_and_detail_while_another_light_is_blocked() {
        let shared = shared();
        let slow = link("slow");
        let fast = link("fast");
        let fast_key = crate::backends::rog::identity_key(&fast, &outcome(&fast, "fast", 61, false).snapshot.identity);
        let (finished, received) = std::sync::mpsc::channel();
        let received = Mutex::new(received);
        refresh_parallel_with(&shared, scan(vec![slow, fast]), &BTreeMap::new(), false, 0, |link, detail| {
            if link.iface.path == "slow" && !detail {
                received.lock().unwrap().recv_timeout(Duration::from_secs(3)).unwrap();
                let state = lock_shared(&shared);
                assert_eq!(state.devices[&fast_key].reading.percent(), Some(61));
            }
            Ok(outcome(link, &link.iface.path, 61, detail))
        }, || {
            if lock_shared(&shared).devices.get(&fast_key).is_some_and(|record| record.state.is_some()) {
                let _ = finished.send(());
            }
        });
        assert_eq!(lock_shared(&shared).devices.len(), 2);
    }

    #[test]
    fn physical_receiver_groups_serialize_slots_and_run_lights_before_details() {
        let shared = shared();
        let first = link("receiver-mi01");
        let second = link("receiver-mi02");
        let physical = BTreeMap::from([("receiver-mi01".into(), "receiver".into()),
            ("receiver-mi02".into(), "receiver".into())]);
        assert_eq!(grouped_links(&[first.clone(), second.clone()], &physical).len(), 1);
        let calls = Mutex::new(Vec::new());
        let active = AtomicBool::new(false);
        refresh_parallel_with(&shared, scan(vec![first, second]), &physical, false, 0, |link, detail| {
            assert!(!active.swap(true, Ordering::SeqCst));
            calls.lock().unwrap().push((link.iface.path.clone(), detail));
            let result = outcome(link, &link.iface.path, 40, detail);
            active.store(false, Ordering::SeqCst);
            Ok(result)
        }, || {});
        let calls = calls.into_inner().unwrap();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls.iter().map(|(_, detail)| *detail).collect::<Vec<_>>(), vec![false, false, true, true]);
    }

    #[test]
    fn known_dual_transport_keeps_its_endpoint_even_when_the_other_replies_first() {
        let shared = shared();
        let wired = link("wired");
        let mut wireless = link("wireless");
        wireless.mode = "Wireless24G".into();
        let key = seed(&shared, &wireless, "same-device");
        let full_at = lock_shared(&shared).devices[&key].full_at;
        let (replied, receive) = std::sync::mpsc::channel();
        let receive = Mutex::new(receive);
        refresh_parallel_with(&shared, scan(vec![wired, wireless]), &BTreeMap::new(), false, 0, |link, detail| {
            assert!(!detail);
            if link.iface.path == "wired" {
                replied.send(()).unwrap();
                Ok(outcome(link, "same-device", 99, false))
            } else {
                receive.lock().unwrap().recv_timeout(Duration::from_secs(3)).unwrap();
                Ok(outcome(link, "same-device", 40, false))
            }
        }, || {
            let state = lock_shared(&shared);
            let record = &state.devices[&key];
            assert_eq!(record.link.as_ref().unwrap().iface.path, "wireless");
            assert_eq!(record.state.as_ref().unwrap().enc[0], 7);
            assert_eq!(record.full_at, full_at);
        });
        let state = lock_shared(&shared);
        assert_eq!(state.devices.len(), 1);
        assert_eq!(state.devices[&key].reading.percent(), Some(40));
    }

    #[test]
    fn failed_preferred_endpoint_can_fall_back_without_losing_verified_identity() {
        let shared = shared();
        let wired = link("wired");
        let mut wireless = link("wireless");
        wireless.mode = "Wireless24G".into();
        let key = seed(&shared, &wireless, "same-device");
        refresh_parallel_with(&shared, scan(vec![wired, wireless]), &BTreeMap::new(), false, 0, |link, detail| {
            if link.iface.path == "wireless" { return Err("channel asleep".into()); }
            Ok(outcome(link, "same-device", 40, detail))
        }, || {});
        let state = lock_shared(&shared);
        assert_eq!(state.devices.len(), 1);
        assert_eq!(state.devices[&key].reading.percent(), Some(40));
        assert_eq!(state.devices[&key].link.as_ref().unwrap().iface.path, "wired");
        assert!(state.devices[&key].state.is_some());
    }

    #[test]
    fn first_dual_transport_discovery_chooses_wired_and_reads_its_own_configuration() {
        let shared = shared();
        let wired = link("wired");
        let mut wireless = link("wireless");
        wireless.mode = "Wireless24G".into();
        let key = crate::backends::rog::identity_key(&wired, &outcome(&wired, "same-device", 40, false).snapshot.identity);
        let (finished, receive) = std::sync::mpsc::channel();
        let receive = Mutex::new(receive);
        {
            lock_shared(&shared).rog_discoveries = vec![wired.iface.clone(), wireless.iface.clone()];
        }
        refresh_parallel_with(&shared, scan(vec![wireless, wired]), &BTreeMap::new(), false, 0, |link, detail| {
            if link.iface.path == "wired" && !detail {
                receive.lock().unwrap().recv_timeout(Duration::from_secs(3)).unwrap();
            }
            if link.iface.path == "wired" && detail {
                let state = lock_shared(&shared);
                assert!(state.devices[&key].state.is_none());
                assert!(state.full.is_none());
            }
            Ok(outcome(link, "same-device", 40, detail))
        }, || {
            let state = lock_shared(&shared);
            if state.devices.get(&key).is_some_and(|record| record.state.is_some()) {
                let _ = finished.send(());
            }
        });
        let state = lock_shared(&shared);
        assert_eq!(state.devices.len(), 1);
        assert_eq!(state.devices[&key].link.as_ref().unwrap().iface.path, "wired");
        assert_eq!(state.devices[&key].state.as_ref().unwrap().enc[0], 7);
        assert!(pending_readings(&state).is_empty());
    }

    #[test]
    fn crossed_device_identities_cannot_deadlock_preferred_endpoint_waits() {
        let shared = shared();
        let first = link("first");
        let second = link("second");
        seed(&shared, &first, "first-device");
        seed(&shared, &second, "second-device");
        refresh_parallel_with(&shared, scan(vec![first, second]), &BTreeMap::new(), false, 0, |link, detail| {
            Ok(outcome(link, if link.iface.path == "first" { "second-device" } else { "first-device" }, 42, detail))
        }, || {});
        let state = lock_shared(&shared);
        assert_eq!(state.devices.len(), 2);
        assert!(state.devices.values().all(|record| record.reading.percent() == Some(42)));
    }
}
