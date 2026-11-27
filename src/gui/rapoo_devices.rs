use std::collections::{BTreeMap, BTreeSet};

use crate::backends::{Poll, Snapshot, Target};
use crate::backends::rapoo::{DeviceSignature, LinkRole, device_signature};

type Identity = (&'static str, String);

struct Endpoint {
    target: Target,
    signature: DeviceSignature,
    own_key: String,
    card_key: String,
    current: bool,
    present: bool,
}

#[derive(Default)]
pub(super) struct Devices {
    endpoints: Vec<Endpoint>,
    ambiguous: BTreeSet<Identity>,
    used_keys: BTreeSet<String>,
    key_generation: u64,
}

#[derive(Default)]
pub(super) struct Changes {
    pub migrations: Vec<(String, String)>,
    pub retired_keys: BTreeSet<String>,
    pub interrupted_keys: BTreeSet<String>,
}

impl Devices {
    pub(super) fn is_present(&self, key: &str) -> Option<bool> {
        self.endpoints.iter().any(|endpoint| endpoint.card_key == key).then(||
            self.endpoints.iter().any(|endpoint| endpoint.card_key == key && endpoint.present))
    }

    pub(super) fn card_key(&self, target: &Target) -> Option<&str> {
        let signature = device_signature(target)?;
        self.endpoints.iter().find(|endpoint|
            endpoint.target.same_channel(target) && endpoint.signature == signature)
            .map(|endpoint| endpoint.card_key.as_str())
    }

    fn fresh_key(&mut self, previous: &str) -> String {
        loop {
            self.key_generation += 1;
            let key = crate::device::stable_key(&format!("rapoo:independent:{previous}:{}", self.key_generation));
            if self.used_keys.insert(key.clone()) { return key; }
        }
    }

    fn observe(&mut self, target: &Target) {
        let Some(signature) = device_signature(target) else { return; };
        let previous = self.endpoints.iter().position(|endpoint|
            endpoint.target.same_channel(target) && endpoint.signature == signature);
        for endpoint in &mut self.endpoints {
            if endpoint.target.same_channel(target) {
                endpoint.current = false;
                endpoint.present = false;
            }
        }
        if let Some(index) = previous {
            let endpoint = &mut self.endpoints[index];
            endpoint.target = target.clone();
            endpoint.current = true;
            endpoint.present = true;
        } else {
            let mut card_key = target.key.clone();
            if !self.used_keys.insert(card_key.clone()) { card_key = self.fresh_key(&card_key); }
            self.endpoints.push(Endpoint {
                target: target.clone(), signature, own_key: card_key.clone(), card_key,
                current: true, present: true,
            });
        }
    }

    fn join(&mut self, old: &str, new: &str, migrations: &mut Vec<(String, String)>) {
        if old == new { return; }
        for endpoint in &mut self.endpoints {
            if endpoint.card_key == old { endpoint.card_key = new.into(); }
        }
        migrations.push((old.into(), new.into()));
    }

    fn separate(&mut self, identity: &Identity, changes: &mut Changes) {
        for index in 0..self.endpoints.len() {
            let endpoint = &self.endpoints[index];
            if endpoint.signature.model != identity.0 || endpoint.signature.serial != identity.1
                || endpoint.card_key == endpoint.own_key { continue; }
            let retired = endpoint.own_key.clone();
            changes.interrupted_keys.insert(endpoint.card_key.clone());
            let independent = self.fresh_key(&retired);
            changes.retired_keys.insert(retired);
            changes.interrupted_keys.insert(independent.clone());
            let endpoint = &mut self.endpoints[index];
            // 新采样不复用旧别名的源键，已经在途的历史迁移也不能重定向它。
            endpoint.own_key = independent.clone();
            endpoint.card_key = independent;
        }
    }

    fn find_unplugged_replacement(&self, key: &str, identity: &Identity) -> Option<String> {
        let cards: BTreeSet<_> = self.endpoints.iter().filter(|endpoint|
            endpoint.card_key != key
                && endpoint.signature.model == identity.0 && endpoint.signature.serial == identity.1)
            .map(|endpoint| endpoint.card_key.as_str()).collect();
        let mut candidates = cards.into_iter();
        let candidate = candidates.next()?;
        if candidates.next().is_some() || self.is_present(candidate) != Some(false) { return None; }
        Some(candidate.into())
    }

    fn uniquely_discovered(endpoint: &Endpoint, poll: &Poll) -> bool {
        let Ok(interface) = endpoint.target.hid() else { return false; };
        let legacy = crate::device::stable_key(&format!("{}:{:04x}:{:04x}:{}:{}",
            endpoint.target.backend.name(), interface.vendor_id, interface.product_id,
            interface.serial, endpoint.target.address));
        poll.legacy_key_migrations.get(&legacy) == Some(&endpoint.target.key)
    }

    pub(super) fn coalesce(&mut self, poll: &mut Poll, existing: &BTreeSet<String>) -> Changes {
        if let Some(paths) = &poll.present_channels {
            for endpoint in &mut self.endpoints {
                endpoint.present = endpoint.current
                    && paths.iter().any(|path| path.eq_ignore_ascii_case(&endpoint.target.channel_key()));
            }
        }
        for snapshot in &poll.snapshots { self.observe(&snapshot.target); }

        let mut groups: BTreeMap<Identity, Vec<usize>> = BTreeMap::new();
        for (index, endpoint) in self.endpoints.iter().enumerate() {
            if endpoint.present && !endpoint.signature.serial.trim().is_empty() {
                groups.entry((endpoint.signature.model, endpoint.signature.serial.clone()))
                    .or_default().push(index);
            }
        }
        let mut changes = Changes::default();
        for (identity, indices) in groups {
            let receivers: Vec<_> = indices.iter().copied().filter(|&index|
                self.endpoints[index].signature.role == LinkRole::Receiver).collect();
            let wired: Vec<_> = indices.iter().copied().filter(|&index|
                self.endpoints[index].signature.role == LinkRole::Wired).collect();
            // 发现阶段的唯一性凭据也涵盖本轮读取失败的接收器。
            let unique = indices.iter().all(|&index| Self::uniquely_discovered(&self.endpoints[index], poll));
            if poll.present_channels.is_some() && (receivers.len() > 1 || wired.len() > 1) {
                self.ambiguous.insert(identity.clone());
            }
            if self.ambiguous.contains(&identity) || (poll.present_channels.is_some() && !unique) {
                self.separate(&identity, &mut changes);
                continue;
            }
            if let ([receiver], [wire]) = (receivers.as_slice(), wired.as_slice()) {
                let receiver_key = self.endpoints[*receiver].card_key.clone();
                let wired_key = self.endpoints[*wire].card_key.clone();
                if receiver_key != wired_key && !unique { continue; }
                let (old, new) = if existing.contains(&wired_key) && !existing.contains(&receiver_key) {
                    (receiver_key, wired_key)
                } else { (wired_key, receiver_key) };
                self.join(&old, &new, &mut changes.migrations);
            }
            let key = self.endpoints[indices[0]].card_key.clone();
            if unique && poll.present_channels.is_some() && !existing.contains(&key) {
                if let Some(old) = self.find_unplugged_replacement(&key, &identity) {
                    self.join(&key, &old, &mut changes.migrations);
                }
            }
        }

        let mut merged: BTreeMap<String, Snapshot> = BTreeMap::new();
        let mut reported = BTreeSet::new();
        for mut snapshot in std::mem::take(&mut poll.snapshots) {
            if let Some(endpoint) = self.endpoints.iter().find(|endpoint|
                endpoint.current && endpoint.target.same_channel(&snapshot.target)) {
                snapshot.target.key = endpoint.card_key.clone();
                reported.insert((endpoint.target.channel_key().to_ascii_lowercase(), endpoint.target.address));
            }
            if !snapshot.connected { snapshot.disable_writes(); }
            let key = snapshot.target.key.clone();
            let preferred = merged.get(&key).is_none_or(|previous| rank(&snapshot) > rank(previous));
            if preferred { merged.insert(key, snapshot); }
        }
        poll.snapshots = merged.into_values().filter(|snapshot| {
            // 单个接收器离线报告不能否定仍存在但本轮读取失败的有线端点。
            snapshot.connected || !self.endpoints.iter().any(|endpoint|
                endpoint.card_key == snapshot.target.key && endpoint.present
                    && !reported.contains(&(endpoint.target.channel_key().to_ascii_lowercase(), endpoint.target.address)))
        }).collect();
        changes
    }
}

fn rank(snapshot: &Snapshot) -> (bool, bool, bool, Option<std::time::Instant>, bool) {
    (snapshot.connected,
        snapshot.dpi.writable || snapshot.dpi.stage_writable || snapshot.dpi.stage_editable
            || snapshot.polling_rate.writable,
        snapshot.battery.is_some(), snapshot.battery_at,
        device_signature(&snapshot.target).is_some_and(|signature| signature.role == LinkRole::Wired))
}
