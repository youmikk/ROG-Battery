//! Read-only CompX page/command Feature Report family (distinct from EEPROM16).
//! Lamzu/WLMouse protocol evidence and exceptions are listed in PROTOCOLS.md.
use super::*;
use crate::device::{DpiStage, PollingRate};
use std::time::{Duration, Instant};

pub struct PageCommand;

fn control(i: &HidInterface) -> bool {
    matches!(i.vendor_id, 0x373e | 0x37b0 | 0x36a7)
        && i.usage_page >= 0xff00
        && i.feature_len == 65
        && !(i.vendor_id == 0x36a7 && i.product_id == 0xa887)
        && !(i.vendor_id == 0x37b0 && matches!(i.product_id, 2 | 4 | 8 | 10))
        && !(i.vendor_id == 0x373e && i.product_id & 0xf000 == 0xb000)
}
fn target_address(i: &HidInterface) -> u8 {
    if i.vendor_id == 0x373e && matches!(i.product_id, 0x006a | 0x0032 | 0x0054) {
        0
    } else {
        2
    }
}
fn standard_layout(i: &HidInterface, paired: Option<(u16, u16)>) -> Option<u8> {
    // LunaFury uses a different DPI storage layout, despite sharing transport.
    let (vid, pid) = paired.unwrap_or((i.vendor_id, i.product_id));
    match (vid, pid) {
        (0x37b0, 7 | 13 | 14 | 9 | 15 | 16) => Some(6),
        (
            0x373e,
            0x001c..=0x001e
            | 0x006a
            | 0x006b
            | 0x0046
            | 0x0047
            | 0x0021
            | 0x0022
            | 0x003a
            | 0x003b
            | 0x0098
            | 0x0099,
        ) => Some(6),
        (0x373e, 0x0066 | 0x0067) => Some(5),
        (0x36a7, 0xa887) => None,
        (0x36a7, _) => Some(6),
        _ => None,
    }
}
fn candidate(i: &HidInterface) -> Target {
    let mut t = Target::new(BackendId::PageCommand, i, target_address(i));
    t.family = DeviceFamily::Mouse;
    if t.name.is_empty() {
        t.name = match i.vendor_id {
            0x36a7 => "WLMouse",
            0x37b0 => "Lamzu",
            _ => "CompX Mouse",
        }
        .into();
    }
    t
}
fn packet(target: u8, page: u8, command: u8, length: u8, args: &[u8]) -> [u8; 65] {
    let mut p = [0; 65];
    p[3] = target;
    p[4] = length;
    p[5] = page;
    p[6] = command;
    p[7..7 + args.len()].copy_from_slice(args);
    p
}
enum Reply {
    Pending,
    Unsupported,
    Data(Vec<u8>),
}
fn parse(r: &[u8], p: &[u8], minimum: usize) -> Option<Reply> {
    if r.len() != 65 || r[0] != 0 || r[5..7] != p[5..7] || r[4] > 58 {
        return None;
    }
    // Some firmware leaves target at zero in replies. A nonzero, different
    // target is invalid; requests are serialized on the exact control interface.
    if r[3] != 0 && r[3] != p[3] {
        return None;
    }
    match r[1] {
        0 | 0xa0 | 0xa3 => Some(Reply::Pending),
        0xa2 => Some(Reply::Unsupported),
        0xa1 if usize::from(r[4]) >= minimum => {
            Some(Reply::Data(r[7..7 + usize::from(r[4])].to_vec()))
        }
        _ => None,
    }
}
trait Channel {
    fn send(&mut self, p: &[u8]) -> bool;
    fn receive(&mut self, timeout: u32) -> Option<Vec<u8>>;
    fn pause(&self) {}
}
impl Channel for feature::FeatureChannel {
    fn send(&mut self, p: &[u8]) -> bool {
        self.send(p, 150).is_ok()
    }
    fn receive(&mut self, timeout: u32) -> Option<Vec<u8>> {
        self.receive(0, timeout).ok()
    }
    fn pause(&self) {
        std::thread::sleep(Duration::from_millis(30));
    }
}
struct Session<C> {
    channel: C,
    interface: HidInterface,
    address: u8,
}
impl<C: Channel> Session<C> {
    fn query(
        &mut self,
        target: u8,
        page: u8,
        command: u8,
        length: u8,
        args: &[u8],
        minimum: usize,
        profile: Option<u8>,
    ) -> Option<Vec<u8>> {
        let started = Instant::now();
        let p = packet(target, page, command, length, args);
        let mut outcome = trace::Outcome::NoReply;
        let mut result = None;
        if self.channel.send(&p) {
            for _ in 0..8 {
                self.channel.pause();
                let Some(left) = Duration::from_millis(800).checked_sub(started.elapsed()) else {
                    break;
                };
                let Some(r) = self
                    .channel
                    .receive(left.as_millis().max(1).min(100) as u32)
                else {
                    continue;
                };
                match parse(&r, &p, minimum) {
                    Some(Reply::Data(data))
                        if profile.is_none_or(|profile| data.first() == Some(&profile)) =>
                    {
                        outcome = trace::Outcome::Ok;
                        result = Some(data);
                        break;
                    }
                    Some(Reply::Unsupported) => {
                        outcome = trace::Outcome::Rejected(0xa2);
                        break;
                    }
                    _ => {}
                }
            }
        }
        trace::record(
            &self.interface,
            BackendId::PageCommand,
            "query",
            u16::from(page),
            command,
            65,
            if result.is_some() { 65 } else { 0 },
            started,
            outcome,
        );
        result
    }
    fn profile(&mut self) -> Option<u8> {
        self.query(self.address, 0, 0x85, 1, &[], 1, None)?
            .first()
            .copied()
            .filter(|p| (1..=5).contains(p))
    }
    fn pairing(&mut self) -> Option<(u16, u16)> {
        let r = self.query(1, 0, 0x8b, 6, &[2], 6, None)?;
        let (vid, pid) = (
            u16::from_be_bytes([r[2], r[3]]),
            u16::from_be_bytes([r[4], r[5]]),
        );
        (matches!(vid, 0x373e | 0x37b0 | 0x36a7) && pid != 0 && pid != 65535).then_some((vid, pid))
    }
    fn read(&mut self, target: &Target) -> Option<Snapshot> {
        let battery = self
            .query(self.address, 0, 0x83, 2, &[], 2, None)
            .and_then(|r| parse_battery(&r));
        let battery_at = battery.as_ref().map(|_| Instant::now());
        let shared =
            self.interface.vendor_id == 0x37b0 && matches!(self.interface.product_id, 1 | 3);
        let paired = if shared { self.pairing() } else { None };
        let layout = if shared && paired.is_none() {
            None
        } else {
            standard_layout(&self.interface, paired)
        };
        let mut dpi = Dpi::default();
        let mut rate = PollingRate::default();
        if let Some(max) = layout {
            if let Some(profile) = self.profile() {
                let stages = self
                    .query(self.address, 1, 0x81, 10, &[profile, max], 2, Some(profile))
                    .and_then(|r| parse_stages(&r, profile, max));
                let active = self
                    .query(self.address, 1, 0x82, 2, &[profile], 2, Some(profile))
                    .map(|r| r[1]);
                let current = self
                    .query(self.address, 1, 0x80, 2, &[profile], 2, Some(profile))
                    .and_then(|r| decode_rate(r[1]));
                // A profile switch during polling invalidates the collected settings,
                // while independent battery data remains valid.
                if self.profile() == Some(profile) {
                    rate.current = current;
                    if let (Some((stages, ys)), Some(active)) = (stages, active) {
                        if (1..=stages.len() as u8).contains(&active) {
                            let index = active - 1;
                            dpi.current = Some(stages[usize::from(index)].value);
                            dpi.current_y = ys.get(&index).copied();
                            dpi.stages = stages;
                            dpi.stage_y = ys;
                            dpi.active_stage = Some(index);
                            dpi.profile = Some(u16::from(profile));
                        }
                    }
                }
            }
        }
        if battery.is_none() && dpi.current.is_none() && rate.current.is_none() {
            return None;
        }
        let mut target = target.clone();
        if let Some((vid, pid)) = paired {
            target.key = crate::device::stable_key(&format!(
                "{}:{vid:04x}:{pid:04x}",
                candidate(&self.interface).key
            ));
            target.verified_identity = true;
            // Confirms the paired model only, not a unique unit; this adapter has no writes.
        }
        Some(Snapshot {
            target,
            battery,
            battery_at,
            dpi,
            polling_rate: rate,
            connected: true,
        })
    }
}
fn parse_battery(r: &[u8]) -> Option<Battery> {
    if r.len() < 2 || r[1] > 100 {
        return None;
    }
    Some(Battery {
        estimated_percent: None,
        percent: Some(r[1]),
        charging: match r[0] {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        },
        voltage_mv: None,
        approximate: false,
        level: None,
    })
}
fn parse_stages(
    r: &[u8],
    profile: u8,
    max: u8,
) -> Option<(Vec<DpiStage>, std::collections::BTreeMap<u8, u16>)> {
    if r.len() < 2
        || r[0] != profile
        || !(1..=max).contains(&r[1])
        || r.len() < 2 + usize::from(r[1]) * 4
    {
        return None;
    }
    let mut stages = Vec::new();
    let mut ys = std::collections::BTreeMap::new();
    for (index, row) in r[2..2 + usize::from(r[1]) * 4].chunks_exact(4).enumerate() {
        let x = u16::from_be_bytes([row[0], row[1]]);
        let y = u16::from_be_bytes([row[2], row[3]]);
        if x == 0 || y == 0 || x == 65535 || y == 65535 {
            return None;
        }
        stages.push(DpiStage {
            index: index as u8,
            value: x,
        });
        ys.insert(index as u8, y);
    }
    Some((stages, ys))
}
fn decode_rate(raw: u8) -> Option<u16> {
    match raw {
        8 => Some(125),
        4 => Some(250),
        2 => Some(500),
        1 | 16 => Some(1000),
        32 => Some(2000),
        64 => Some(4000),
        128 => Some(8000),
        _ => None,
    }
}
impl DeviceBackend for PageCommand {
    fn discover(&self, _: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(interfaces
            .iter()
            .filter(|i| control(i))
            .map(candidate)
            .collect())
    }
    fn read(&self, _: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        if !control(&target.interface) {
            return Err(crate::tr!("设备功能报告长度无效").into());
        }
        let lock = channel::lock(&target.interface.path);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut s = Session {
            channel: feature::FeatureChannel::open(&target.interface)?,
            interface: target.interface.clone(),
            address: target_address(&target.interface),
        };
        s.read(target)
            .ok_or_else(|| crate::i18n::translate("设备无有效状态回报").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn interface() -> HidInterface {
        HidInterface {
            path: "unit-one".into(),
            vendor_id: 0x37b0,
            product_id: 9,
            usage_page: 0xff00,
            usage: 1,
            input_len: 0,
            output_len: 0,
            feature_len: 65,
            product: "Lamzu".into(),
            serial: String::new(),
        }
    }
    fn response(p: &[u8], data: &[u8]) -> Vec<u8> {
        let mut r = vec![0; 65];
        r[1] = 0xa1;
        r[4] = data.len() as u8;
        r[5..7].copy_from_slice(&p[5..7]);
        r[7..7 + data.len()].copy_from_slice(data);
        r
    }
    #[test]
    fn strict_frames_reject_echoes_other_pages_targets_and_truncation() {
        let p = packet(2, 0, 0x83, 2, &[]);
        assert!(matches!(parse(&p, &p, 2), Some(Reply::Pending)));
        let good = response(&p, &[0, 0]);
        assert!(matches!(parse(&good, &p, 2), Some(Reply::Data(_))));
        for (offset, value) in [(0, 1), (3, 1), (4, 59), (5, 1), (6, 0x84)] {
            let mut bad = good.clone();
            bad[offset] = value;
            assert!(parse(&bad, &p, 2).is_none());
        }
        assert!(parse(&good[..64], &p, 2).is_none());
        assert!(parse(&response(&p, &[0]), &p, 2).is_none());
    }
    #[test]
    fn firmware_update_and_report_four_devices_are_excluded() {
        let mut i = interface();
        assert!(control(&i));
        for pid in [2, 4, 8, 10] {
            i.product_id = pid;
            assert!(!control(&i));
        }
        i.vendor_id = 0x36a7;
        i.product_id = 0xa887;
        assert!(!control(&i));
        i.product_id = 0xa863;
        assert!(control(&i));
        i.feature_len = 64;
        assert!(!control(&i));
    }
    #[test]
    fn stage_records_are_complete_and_profile_bound_with_no_clamping() {
        let r = [2, 2, 3, 32, 6, 64, 3, 32, 3, 32];
        let (stages, ys) = parse_stages(&r, 2, 6).unwrap();
        assert_eq!(stages[0].value, 800);
        assert_eq!(stages[1].value, 800);
        assert_eq!(ys[&0], 1600);
        assert!(parse_stages(&r, 1, 6).is_none());
        assert!(parse_stages(&r[..9], 2, 6).is_none());
        assert!(parse_stages(&r, 2, 1).is_none());
        assert_eq!(decode_rate(16), Some(1000));
        assert_eq!(decode_rate(3), None);
        assert_eq!(parse_battery(&[9, 40]).unwrap().charging, None);
        assert!(parse_battery(&[0, 255]).is_none());
    }
    struct Mouse {
        pending: Vec<Vec<u8>>,
        profile_reads: u8,
        change_profile: bool,
        wrong_profile: bool,
        queries: usize,
    }
    impl Channel for Mouse {
        fn send(&mut self, p: &[u8]) -> bool {
            self.queries += 1;
            let mut profile = 2;
            let data = match (p[5], p[6]) {
                (0, 0x83) => vec![0, 75],
                (0, 0x85) => {
                    self.profile_reads += 1;
                    if self.change_profile && self.profile_reads > 1 {
                        profile = 3;
                    }
                    vec![profile]
                }
                (1, 0x81) => vec![
                    if self.wrong_profile { 1 } else { 2 },
                    2,
                    3,
                    32,
                    6,
                    64,
                    6,
                    64,
                    12,
                    128,
                ],
                (1, 0x82) => vec![2, 2],
                (1, 0x80) => vec![2, 64],
                _ => panic!("unexpected query"),
            };
            let mut unrelated = response(p, &[0, 100]);
            unrelated[6] ^= 1;
            self.pending = vec![response(p, &data), p.to_vec(), unrelated];
            true
        }
        fn receive(&mut self, _: u32) -> Option<Vec<u8>> {
            self.pending.pop()
        }
    }
    fn session() -> Session<Mouse> {
        Session {
            channel: Mouse {
                pending: Vec::new(),
                profile_reads: 0,
                change_profile: false,
                wrong_profile: false,
                queries: 0,
            },
            interface: interface(),
            address: 2,
        }
    }
    #[test]
    fn reader_uses_actual_profile_and_keeps_independent_axes_read_only() {
        let mut s = session();
        let t = candidate(&s.interface);
        let r = s.read(&t).unwrap();
        assert_eq!(r.battery.unwrap().percent, Some(75));
        assert_eq!(r.dpi.current, Some(1600));
        assert_eq!(r.dpi.current_y, Some(3200));
        assert_eq!(r.dpi.active_stage, Some(1));
        assert_eq!(r.dpi.profile, Some(2));
        assert_eq!(r.polling_rate.current, Some(4000));
        assert!(!r.dpi.stage_writable && !r.polling_rate.writable);
        assert!(r.polling_rate.values.is_empty());
    }
    #[test]
    fn profile_change_and_incomplete_config_do_not_destroy_independent_battery() {
        let mut s = session();
        s.channel.change_profile = true;
        let t = candidate(&s.interface);
        let r = s.read(&t).unwrap();
        assert!(r.dpi.current.is_none());
        assert!(r.polling_rate.current.is_none());
        assert!(r.battery.is_some());
        let mut s = session();
        s.channel.wrong_profile = true;
        let r = s.read(&t).unwrap();
        assert!(r.dpi.stages.is_empty());
        assert_eq!(r.polling_rate.current, Some(4000));
        assert!(r.battery.is_some());
    }
    #[test]
    fn unknown_identity_and_different_storage_layout_are_battery_only() {
        let mut s = session();
        s.interface.product_id = 0xffff;
        let t = candidate(&s.interface);
        let r = s.read(&t).unwrap();
        assert!(r.battery.is_some());
        assert!(r.dpi.stages.is_empty());
        assert_eq!(s.channel.queries, 1);
        s.interface.vendor_id = 0x373e;
        s.interface.product_id = 0x0032;
        assert_eq!(target_address(&s.interface), 0);
        assert!(standard_layout(&s.interface, None).is_none());
    }
}
