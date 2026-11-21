//! HYZEN receiver battery transactions, limited to online and battery queries.
use super::*;
use crate::hid::HidDevice;
use std::time::{Duration, Instant};

pub struct Lofree;

fn control(i: &HidInterface) -> bool {
    i.vendor_id == 0x388d && i.product_id == 0x0025
        && i.usage_page == 0xff1c && i.usage == 0x92
        && i.input_len == 32 && i.output_len == 32
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    // The official driver suppresses receiver queries while a wired HYZEN is
    // present. There is no proven receiver/cable pairing ID, so do not guess it.
    if interfaces.iter().any(|i| i.vendor_id == 0x388d && i.product_id == 0x0024) {
        return Vec::new();
    }
    interfaces.iter().filter(|i| control(i)).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase { Start, Data(u8), End }

impl Phase {
    fn command(self) -> u8 { match self { Self::Start => 1, Self::End => 2, Self::Data(cmd) => cmd } }
    fn packet(self) -> [u8; 32] {
        let mut packet = [0; 32]; packet[0] = 4; packet[3] = self.command(); packet
    }
}

fn reply(frame: &[u8], phase: Phase) -> Option<u8> {
    if frame.len() != 32 || frame[0] != 4 { return None; }
    match phase {
        Phase::Start | Phase::End => (frame[3] == phase.command()).then_some(0),
        Phase::Data(command) => {
            // Byte 4 is payload size, 5/6 is the offset and data starts at 8.
            // Require actual payload, not the zero-length request echo. Unlike
            // the reference's loose first-byte read, a malformed size is ignored.
            if !matches!(command, 0xaa | 0x1a) || ![0, command].contains(&frame[3])
                || !(1..=24).contains(&frame[4]) || frame[5..7] != [0, 0] { return None; }
            let value = frame[8];
            match command {
                0xaa if value <= 1 => Some(value),
                0x1a if value <= 100 => Some(value),
                _ => None,
            }
        },
    }
}

trait Channel {
    fn exchange(&mut self, phase: Phase, timeout: u32) -> Option<u8>;
}

struct Session<'a> { device: HidDevice, interface: &'a HidInterface }

impl Channel for Session<'_> {
    fn exchange(&mut self, phase: Phase, timeout: u32) -> Option<u8> {
        let started = Instant::now();
        let response = self.device.transact(&phase.packet(), timeout, |r| reply(r, phase).is_some());
        trace::record(self.interface, BackendId::Lofree, match phase {
            Phase::Start => "start-query", Phase::Data(0xaa) => "query-connection",
            Phase::Data(_) => "query-battery", Phase::End => "end-query",
        }, 4, phase.command(), 32, response.as_ref().map_or(0, Vec::len), started,
            if response.is_some() { trace::Outcome::Ok } else { trace::Outcome::NoReply });
        response.as_deref().and_then(|r| reply(r, phase))
    }
}

fn transaction(channel: &mut impl Channel, command: u8) -> Option<u8> {
    if !matches!(command, 0xaa | 0x1a) { return None; }
    let deadline = Instant::now() + Duration::from_millis(2700);
    let started = channel.exchange(Phase::Start, 900).is_some();
    let value = if started {
        deadline.checked_duration_since(Instant::now()).and_then(|left|
            channel.exchange(Phase::Data(command), left.as_millis().clamp(1, 1800) as u32))
    } else { None };
    // A delivered START may lose its acknowledgement. Always attempt END, even
    // after timeout, but do not publish data unless both data and END succeed.
    let ended = channel.exchange(Phase::End, 300).is_some();
    ended.then_some(value).flatten()
}

struct Sample { connected: bool, percent: Option<u8>, measured: Option<Instant> }

fn sample(channel: &mut impl Channel) -> Option<Sample> {
    let connected = transaction(channel, 0xaa)? == 1;
    let mut sample = Sample { connected, percent: None, measured: None };
    if !connected { return Some(sample); }
    let Some(percent) = transaction(channel, 0x1a) else { return Some(sample); };
    let measured = Instant::now();
    match transaction(channel, 0xaa) {
        Some(1) => { sample.percent = Some(percent); sample.measured = Some(measured); },
        Some(0) => sample.connected = false,
        _ => (),
    }
    Some(sample)
}

impl DeviceBackend for Lofree {
    fn discover(&self, _: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().map(|i| {
            let mut target = Target::new(BackendId::Lofree, i, 0);
            target.name = "Lofree HYZEN".into();
            target.family = DeviceFamily::Keyboard;
            target
        }).collect())
    }

    fn read(&self, _: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let interface = target.hid()?;
        if !control(interface) { return Err(crate::i18n::translate("设备无有效状态回报").into()); }
        let lock = channel::lock(&interface.path);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut channel = Session { device: HidDevice::open(interface)?, interface };
        let sample = sample(&mut channel).ok_or_else(|| crate::i18n::translate("设备无有效状态回报"))?;
        let battery = sample.percent.map(|percent| Battery { percent: Some(percent), estimated_percent: None,
            charging: None, voltage_mv: None, approximate: false, level: None });
        Ok(Snapshot { target: target.clone(), battery, battery_at: sample.measured,
            battery_unavailable: !sample.connected, connected: sample.connected,
            dpi: Dpi::default(), polling_rate: Default::default() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn data(command: u8, value: u8) -> [u8; 32] {
        let mut r = [0; 32]; r[0] = 4; r[3] = command; r[4] = 1; r[8] = value; r
    }

    struct Script { replies: VecDeque<Option<u8>>, sent: Vec<Phase> }
    impl Channel for Script {
        fn exchange(&mut self, phase: Phase, timeout: u32) -> Option<u8> {
            assert!(timeout > 0 && timeout <= 1800);
            self.sent.push(phase); self.replies.pop_front().expect("unexpected query")
        }
    }
    fn script(replies: impl IntoIterator<Item = Option<u8>>) -> Script {
        Script { replies: replies.into_iter().collect(), sent: Vec::new() }
    }

    fn interface() -> HidInterface {
        HidInterface { path: "keyboard-a".into(), vendor_id: 0x388d, product_id: 0x0025,
            usage_page: 0xff1c, usage: 0x92, input_len: 32, output_len: 32, feature_len: 0,
            product: "HYZEN67@Lofree".into(), serial: "shared".into() }
    }

    #[test]
    fn numeric_battery_requires_nonempty_payload_and_zero_offset() {
        for percent in 0..=100 {
            for command in [0, 0x1a] { assert_eq!(reply(&data(command, percent), Phase::Data(0x1a)), Some(percent)); }
        }
        for invalid in 101..=255 { assert!(reply(&data(0x1a, invalid), Phase::Data(0x1a)).is_none()); }
        for online in [0, 1] { assert_eq!(reply(&data(0xaa, online), Phase::Data(0xaa)), Some(online)); }
        assert!(reply(&data(0xaa, 2), Phase::Data(0xaa)).is_none());
        assert!(reply(&Phase::Data(0x1a).packet(), Phase::Data(0x1a)).is_none());
        for (index, value) in [(0, 1), (3, 0xaa), (4, 0), (4, 25), (5, 1), (6, 1)] {
            let mut malformed = data(0x1a, 61); malformed[index] = value;
            assert!(reply(&malformed, Phase::Data(0x1a)).is_none());
        }
        for length in 0..32 { assert!(reply(&data(0x1a, 61)[..length], Phase::Data(0x1a)).is_none()); }
        let mut long = data(0x1a, 61).to_vec(); long.push(0);
        assert!(reply(&long, Phase::Data(0x1a)).is_none());
    }

    #[test]
    fn every_started_transaction_attempts_end_even_when_start_or_data_times_out() {
        for (responses, phases) in [
            (vec![None, Some(0)], vec![Phase::Start, Phase::End]),
            (vec![Some(0), None, Some(0)], vec![Phase::Start, Phase::Data(0x1a), Phase::End]),
            (vec![Some(0), Some(61), None], vec![Phase::Start, Phase::Data(0x1a), Phase::End]),
        ] {
            let mut io = script(responses);
            assert!(transaction(&mut io, 0x1a).is_none());
            assert_eq!(io.sent, phases);
            assert!(io.replies.is_empty());
        }
        let mut io = script([Some(0), Some(61), Some(0)]);
        assert_eq!(transaction(&mut io, 0x1a), Some(61));
        for phase in io.sent {
            assert_eq!(&phase.packet()[..4], &[4, 0, 0, phase.command()]);
            assert!(phase.packet()[4..].iter().all(|b| *b == 0));
        }
        let mut io = script([]);
        assert!(transaction(&mut io, 0x27).is_none());
        assert!(io.sent.is_empty());
    }

    #[test]
    fn phase_matching_never_accepts_end_as_start_or_a_data_frame_as_ack() {
        assert!(reply(&Phase::End.packet(), Phase::Start).is_none());
        assert!(reply(&Phase::Start.packet(), Phase::End).is_none());
        assert!(reply(&data(0x1a, 61), Phase::End).is_none());
        assert!(reply(&Phase::End.packet(), Phase::Data(0x1a)).is_none());
    }

    #[test]
    fn offline_skips_battery_and_lost_confirmation_cannot_publish_a_new_sample() {
        let mut io = script([Some(0), Some(0), Some(0)]);
        let s = sample(&mut io).unwrap();
        assert!(!s.connected && s.percent.is_none() && s.measured.is_none());
        assert_eq!(io.sent, vec![Phase::Start, Phase::Data(0xaa), Phase::End]);
        for final_state in [Some(0), None] {
            let mut io = script([Some(0), Some(1), Some(0), Some(0), Some(61), Some(0), Some(0), final_state, Some(0)]);
            let s = sample(&mut io).unwrap();
            assert!(s.percent.is_none() && s.measured.is_none());
            assert_eq!(s.connected, final_state.is_none());
        }
        let mut io = script([Some(0), Some(1), Some(0), Some(0), Some(61), Some(0), Some(0), Some(1), Some(0)]);
        let s = sample(&mut io).unwrap();
        assert!(s.connected && s.measured.is_some());
        assert_eq!(s.percent, Some(61));
    }

    #[test]
    fn wired_presence_suppresses_queries_without_merging_identical_receivers() {
        let i = interface();
        assert!(control(&i));
        let mut second = i.clone(); second.path = "keyboard-b".into();
        assert_eq!(candidates(&[i.clone(), second.clone()]).len(), 2);
        assert_ne!(Target::new(BackendId::Lofree, &i, 0).key, Target::new(BackendId::Lofree, &second, 0).key);
        let mut cable = i.clone(); cable.product_id = 0x0024;
        assert!(!control(&cable));
        assert!(candidates(&[i.clone(), second, cable]).is_empty());
        let mut other = i.clone(); other.product_id = 0x0029;
        assert!(!control(&other));
        other = i.clone(); other.usage_page = 1; other.usage = 6;
        assert!(!control(&other));
        other = i.clone(); other.output_len = 64;
        assert!(!control(&other));
    }
}
