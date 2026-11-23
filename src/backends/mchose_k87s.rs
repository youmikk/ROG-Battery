//! K87S GLW receiver: read the selected profile, without configuration repair.
//! Its exact M HUB catalog entry selects this channel and config-byte layout.
use super::*;
use crate::hid::HidDevice;
use std::time::{Duration, Instant};

pub struct MchoseK87s;

const PRODUCT: &str = "MCHOSE K87S 2.4G";
const REPORT_BYTES: u16 = 65;
const PAYLOAD_BYTES: usize = 64;
const UNAVAILABLE: &str = "设备无有效状态回报";

fn management(i: &HidInterface) -> bool {
    i.vendor_id == 0x3837 && i.product_id == 0x3033 && i.usage_page == 1 && i.usage == 0
}

fn receiver_group(i: &HidInterface) -> Option<String> {
    if !management(i) { return None; }
    let path = i.path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    if parts.len() != 4 || parts[0] != r"\\?\hid" || parts[2].is_empty()
        || parts[3].is_empty() || path.contains('\0') { return None; }
    let fields: Vec<_> = parts[1].split('&').collect();
    for (prefix, expected) in [("vid_", i.vendor_id), ("pid_", i.product_id)] {
        let values: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix(prefix)).collect();
        let [value] = values.as_slice() else { return None; };
        if value.len() != 4 || u16::from_str_radix(value, 16).ok() != Some(expected) { return None; }
    }
    let interfaces: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix("mi_")).collect();
    let [interface] = interfaces.as_slice() else { return None; };
    if interface.len() != 2 || !interface.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
    let collections: Vec<_> = fields.iter().filter_map(|field| field.strip_prefix("col")).collect();
    if collections.is_empty() { return Some(path); }
    let [collection] = collections.as_slice() else { return None; };
    if collection.len() != 2 || !collection.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
    let (instance, index) = parts[2].rsplit_once('&')?;
    if instance.split('&').count() < 2 || index.len() != 4 || !index.starts_with("00")
        || !index.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
    let hardware = fields.into_iter().filter(|field| !field.starts_with("col"))
        .collect::<Vec<_>>().join("&");
    Some(format!("{}#{hardware}#{instance}#{}", parts[0], parts[3]))
}

fn control(i: &HidInterface) -> bool {
    management(i) && i.product == PRODUCT && i.input_len == REPORT_BYTES
        && i.output_len == REPORT_BYTES && receiver_group(i).is_some()
}

fn candidates(interfaces: &[HidInterface]) -> Vec<&HidInterface> {
    interfaces.iter().filter(|i| control(i) && interfaces.iter().filter(|other|
        management(other) && receiver_group(i) == receiver_group(other)
    ).count() == 1).collect()
}

fn input_id(api: &HidApi, i: &HidInterface) -> Option<u8> {
    if !control(i) { return None; }
    let (input, output) = descriptor::Descriptor::open(api, i)?
        .single_input_output_report_ids(i, REPORT_BYTES, REPORT_BYTES)?;
    (output == 0).then_some(input)
}

fn target(i: &HidInterface) -> Target {
    let mut t = Target::new(BackendId::MchoseK87s, i, 0);
    t.name = "MCHOSE K87S".into();
    t.family = DeviceFamily::Keyboard;
    t
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page { Base, ProfileHead(u8), ProfileTail(u8) }

impl Page {
    fn fields(self) -> Option<(u8, u16, u8)> {
        match self {
            Self::Base => Some((4, 0, 56)),
            Self::ProfileHead(slot) if slot < 4 => Some((5, u16::from(slot) * 64, 56)),
            Self::ProfileTail(slot) if slot < 4 => Some((5, u16::from(slot) * 64 + 56, 8)),
            _ => None,
        }
    }

    fn request(self) -> Option<[u8; REPORT_BYTES as usize]> {
        let (command, offset, length) = self.fields()?;
        let mut frame = [0; REPORT_BYTES as usize];
        frame[1] = 0x55;
        frame[2] = command;
        frame[5] = length;
        frame[6..8].copy_from_slice(&offset.to_le_bytes());
        frame[4] = frame[5..].iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        Some(frame)
    }
}

fn payload(frame: &[u8], input_id: u8) -> Option<&[u8]> {
    if input_id == 0 {
        (frame.len() == PAYLOAD_BYTES).then_some(frame)
    } else if frame.len() == usize::from(REPORT_BYTES) && frame.first() == Some(&input_id) {
        Some(&frame[1..])
    } else { None }
}

fn response(frame: &[u8], input_id: u8, page: Page) -> Option<&[u8]> {
    let data = payload(frame, input_id)?;
    let (command, offset, length) = page.fields()?;
    if data[..2] != [0xaa, command] || data[4] != length
        || data[5..7] != offset.to_le_bytes() { return None; }
    // The SDK does not establish a reply checksum or the semantics of the
    // other header bytes. Require the entire requested range instead.
    Some(&data[8..8 + usize::from(length)])
}

fn active_profile(base: &[u8]) -> Option<u8> {
    if base.len() != 56 || base[0] >= 4 || base[1] > 4 { return None; }
    let index = base[0];
    let mut sorted = [0; 4];
    sorted.copy_from_slice(&base[2..6]);
    sorted.sort_unstable();
    if sorted == [0, 1, 2, 3] {
        (base[1] > 0 && index < base[1]).then_some(base[2 + usize::from(index)])
    } else if base[2..6] == [0, 0, 0, 0] {
        // Legacy firmware has no order table. Only its absent (zero) table is
        // accepted; a malformed partial permutation is not silently repaired.
        (base[1] == 0 || index < base[1]).then_some(index)
    } else { None }
}

fn battery(config: &[u8]) -> Option<Battery> {
    if config.len() != 64 { return None; }
    let raw = config[32] & 0x7f;
    if !(1..=100).contains(&raw) { return None; }
    // K87S does not advertise M HUB's 1% precision capability. Match the
    // official ten-percent display bands rather than claim exact precision.
    let estimated = (((u16::from(raw) + 5) / 10).max(1) * 10).min(100) as u8;
    Some(Battery { percent: None, estimated_percent: Some(estimated), charging: None,
        voltage_mv: None, approximate: true, level: None })
}

fn read_with(t: &Target, input_id: u8,
    mut exchange: impl FnMut(Page, &[u8]) -> Option<Vec<u8>>) -> Result<Snapshot, String> {
    let i = t.hid()?;
    if t.backend != BackendId::MchoseK87s || !control(i) { return Err(UNAVAILABLE.into()); }
    let first = exchange(Page::Base, &Page::Base.request().unwrap()).ok_or(UNAVAILABLE)?;
    let base = response(&first, input_id, Page::Base).ok_or(UNAVAILABLE)?;
    let slot = active_profile(base).ok_or(UNAVAILABLE)?;
    let head = Page::ProfileHead(slot);
    let tail = Page::ProfileTail(slot);
    let first_config = exchange(head, &head.request().unwrap()).ok_or(UNAVAILABLE)?;
    let mut config = response(&first_config, input_id, head).ok_or(UNAVAILABLE)?.to_vec();
    let measured = Instant::now();
    let last_config = exchange(tail, &tail.request().unwrap()).ok_or(UNAVAILABLE)?;
    config.extend_from_slice(response(&last_config, input_id, tail).ok_or(UNAVAILABLE)?);
    let battery = battery(&config).ok_or(UNAVAILABLE)?;
    let last = exchange(Page::Base, &Page::Base.request().unwrap()).ok_or(UNAVAILABLE)?;
    let final_base = response(&last, input_id, Page::Base).ok_or(UNAVAILABLE)?;
    if active_profile(final_base) != Some(slot) || final_base[..6] != base[..6] {
        return Err(UNAVAILABLE.into());
    }
    let mut target = target(i);
    target.verified_identity = true;
    Ok(Snapshot { target, battery: Some(battery), battery_at: Some(measured),
        battery_unavailable: false, connected: true, dpi: Dpi::default(),
        polling_rate: Default::default() })
}

impl DeviceBackend for MchoseK87s {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        Ok(candidates(interfaces).into_iter().filter(|i| input_id(api, i).is_some()).map(target).collect())
    }

    fn read(&self, api: &HidApi, t: &Target, _: bool) -> Result<Snapshot, String> {
        if t.backend != BackendId::MchoseK87s { return Err(UNAVAILABLE.into()); }
        let i = t.hid()?;
        let input_id = input_id(api, i).ok_or(UNAVAILABLE)?;
        let lock = channel::lock(&receiver_group(i).ok_or(UNAVAILABLE)?);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut device = HidDevice::open(i)?;
        let deadline = Instant::now() + Duration::from_secs(3);
        read_with(t, input_id, |page, packet| {
            let remaining = deadline.checked_duration_since(Instant::now())?;
            let timeout = remaining.as_millis().min(1_000) as u32;
            if timeout == 0 { return None; }
            let started = Instant::now();
            let (command, offset, _) = page.fields()?;
            let mut frames = 32;
            let result = device.transact_with_input_report(packet, input_id, timeout, |frame| {
                frames -= 1;
                response(frame, input_id, page).is_some() || frames == 0
            });
            trace::record(i, BackendId::MchoseK87s,
                if page == Page::Base { "query-base" } else { "query-profile" },
                offset, command, packet.len(), result.as_ref().map_or(0, Vec::len), started,
                match result.as_deref() {
                    Some(frame) if response(frame, input_id, page).is_some() => trace::Outcome::Ok,
                    Some(_) => trace::Outcome::InvalidData,
                    None => trace::Outcome::NoReply,
                });
            result
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(instance: &str) -> HidInterface {
        HidInterface { path: format!(r"\\?\hid#vid_3837&pid_{:04x}&mi_01&col01#8&{instance}&0&0000#{{hid}}", 0x3033),
            vendor_id: 0x3837, product_id: 0x3033, usage_page: 1, usage: 0,
            input_len: REPORT_BYTES, output_len: REPORT_BYTES, feature_len: 0,
            product: PRODUCT.into(), serial: "shared".into() }
    }

    fn base() -> Vec<u8> {
        let mut result = vec![0; 56];
        result[..6].copy_from_slice(&[1, 3, 2, 0, 3, 1]);
        result
    }

    fn frame(id: u8, page: Page, data: &[u8]) -> Vec<u8> {
        // Synthetic frames from the independently traced M HUB protocol.
        let (command, offset, length) = page.fields().unwrap();
        assert_eq!(data.len(), usize::from(length));
        let mut payload = vec![0; PAYLOAD_BYTES];
        payload[..2].copy_from_slice(&[0xaa, command]);
        payload[4] = length; payload[5..7].copy_from_slice(&offset.to_le_bytes());
        payload[8..8 + data.len()].copy_from_slice(data);
        if id == 0 { payload } else { let mut r = vec![id]; r.extend(payload); r }
    }

    fn frames(id: u8, raw: u8) -> Vec<Vec<u8>> {
        let b = base();
        let slot = active_profile(&b).unwrap();
        let mut config = vec![0; 64]; config[32] = raw;
        vec![frame(id, Page::Base, &b), frame(id, Page::ProfileHead(slot), &config[..56]),
            frame(id, Page::ProfileTail(slot), &config[56..]), frame(id, Page::Base, &b)]
    }

    #[test]
    fn exact_k87s_model_and_descriptor_capacities_are_required_before_io() {
        let i = interface("one"); assert!(control(&i));
        for product in ["MCHOSE G87 V2 2.4G", "MCHOSE K99 V3 2.4G", "MCHOSE K87S",
            "MCHOSE K87S 2.4G ", "", "USB Receiver"] {
            let mut other = i.clone(); other.product = product.into(); assert!(!control(&other));
        }
        for (vid, pid, page, usage, input, output) in [
            (0x3837, 0x200e, 1, 0, 65, 65), (0x3837, 0x2010, 1, 0, 65, 65),
            (0x3837, 0x3033, 0xff31, 0x74, 65, 65), (0x3837, 0x3033, 1, 6, 65, 65),
            (0x3837, 0x3033, 1, 0, 64, 65), (0x3837, 0x3033, 1, 0, 65, 64),
            (0x3554, 0xfa09, 1, 0, 65, 65)] {
            let mut other = i.clone(); other.vendor_id=vid; other.product_id=pid;
            other.usage_page=page; other.usage=usage; other.input_len=input; other.output_len=output;
            assert!(!control(&other));
        }
        for path in [i.path.replace(&format!("pid_{:04x}", i.product_id), "pid_2010"), i.path.replace("mi_01", "mi_010"),
            i.path.replace("col01", "col01&col02"), i.path.replace("vid_3837", "vid_3837&vid_3837"),
            "invalid".into(), String::new()] {
            let mut other = i.clone(); other.path = path; assert!(!control(&other));
        }
        let mut wrong = target(&i); wrong.backend=BackendId::MchoseGlw;
        assert!(read_with(&wrong, 0, |_, _| panic!("wrong backend must not send")).is_err());
    }

    #[test]
    fn siblings_are_rejected_even_when_the_collection_suffix_is_zero_based() {
        let one = interface("one"); let two = interface("two");
        assert_eq!(candidates(&[one.clone(), two.clone()]).len(), 2);
        assert_ne!(target(&one).key, target(&two).key);
        let mut sibling=one.clone();
        sibling.path=sibling.path.replace("col01", "col02").replace("&0000#", "&0001#");
        sibling.product="MCHOSE G87 V2 2.4G".into(); sibling.input_len=64;
        assert!(candidates(&[one, sibling]).is_empty());
    }

    #[test]
    fn ordered_and_legacy_profiles_are_validated_without_repair() {
        assert_eq!(active_profile(&base()), Some(0));
        let mut ordered=base(); ordered[0]=2; assert_eq!(active_profile(&ordered), Some(3));
        let mut legacy=vec![0;56]; legacy[0]=2; assert_eq!(active_profile(&legacy), Some(2));
        for (index, count, order) in [(4,4,[0,1,2,3]), (3,3,[0,1,2,3]),
            (0,0,[0,1,2,3]), (0,5,[0,1,2,3]), (1,4,[0,1,1,3]),
            (1,4,[0,1,2,255]), (0,4,[255,255,255,255])] {
            let mut wrong=vec![0;56]; wrong[..6].copy_from_slice(&[index,count,order[0],order[1],order[2],order[3]]);
            assert!(active_profile(&wrong).is_none());
        }
        assert!(active_profile(&[0;55]).is_none());
        assert!(Page::ProfileHead(4).request().is_none());
        assert!(Page::ProfileTail(255).request().is_none());
    }

    #[test]
    fn one_transaction_reads_the_selected_profile_and_rechecks_base_for_both_report_layouts() {
        let t=target(&interface("one"));
        for id in [0,7] {
            let mut replies=frames(id,64).into_iter(); let mut seen=vec![];
            let sample=read_with(&t,id,|page,packet| {
                seen.push(page); assert_eq!(packet.len(),65); assert_eq!(packet[0],0);
                assert_eq!(packet[1],0x55); assert!([4,5].contains(&packet[2]));
                assert_eq!(packet[4],packet[5..].iter().fold(0u8,|sum,byte|sum.wrapping_add(*byte)));
                assert!(packet[8..].iter().all(|byte|*byte==0)); replies.next()
            }).unwrap();
            assert_eq!(seen,[Page::Base,Page::ProfileHead(0),Page::ProfileTail(0),Page::Base]);
            assert!(sample.connected && sample.target.verified_identity && sample.battery_at.is_some());
            assert!(!sample.dpi.writable && !sample.polling_rate.writable);
            let b=sample.battery.unwrap();
            assert_eq!((b.percent,b.estimated_percent,b.charging),(None,Some(60),None));
            assert_eq!(b.display_percent().unwrap().label(true),"≈60%");
        }
        for (raw,expected) in [(1,10),(14,10),(15,20),(24,20),(55,60),(64,60),(65,70),(94,90),(95,100),(100,100)] {
            for charge in [0,0x80] { let mut config=vec![0;64]; config[32]=raw|charge;
                let b=battery(&config).unwrap(); assert_eq!(b.estimated_percent,Some(expected)); assert_eq!(b.charging,None); }
        }
        for raw in [0,101,127,128,229,255] { let mut config=vec![0;64];config[32]=raw;assert!(battery(&config).is_none()); }
    }

    #[test]
    fn short_echo_wrong_offsets_and_invalid_report_ids_are_rejected() {
        for id in [0,7] {
            let good=frame(id,Page::Base,&base());
            for length in 0..good.len() { assert!(response(&good[..length],id,Page::Base).is_none()); }
            let mut long=good.clone();long.push(0);assert!(response(&long,id,Page::Base).is_none());
            assert!(response(&good,id^1,Page::Base).is_none());
            let shift=usize::from(id!=0);
            for (offset,value) in [(0,0x55),(1,5),(4,55),(4,57),(5,1),(6,1)] {
                let mut wrong=good.clone();wrong[shift+offset]=value;assert!(response(&wrong,id,Page::Base).is_none());
            }
        }
        assert!(response(&Page::Base.request().unwrap()[1..],0,Page::Base).is_none());
    }

    #[test]
    fn invalid_base_partial_reads_and_profile_switch_publish_no_new_sample() {
        let t=target(&interface("one"));
        for missing in 0..4 {
            let mut replies=frames(0,64).into_iter(); let mut calls=0;
            assert!(read_with(&t,0,|_,_| { let index=calls;calls+=1;
                if index==missing {None} else {replies.next()} }).is_err());
            assert_eq!(calls,missing+1);
        }
        let invalid=frame(0,Page::Base,&[255;56]); let mut calls=0;
        assert!(read_with(&t,0,|_,_|{calls+=1;Some(invalid.clone())}).is_err());assert_eq!(calls,1);
        let mut replies=frames(0,64);let mut switched=base();switched[0]=2;
        replies[3]=frame(0,Page::Base,&switched);let mut replies=replies.into_iter();
        assert!(read_with(&t,0,|_,_|replies.next()).is_err());
        let mut changed_order=base();changed_order[2..6].copy_from_slice(&[3,0,2,1]);
        assert_eq!(active_profile(&changed_order),Some(0));
        let mut replies=frames(0,64);replies[3]=frame(0,Page::Base,&changed_order);let mut replies=replies.into_iter();
        assert!(read_with(&t,0,|_,_|replies.next()).is_err());
    }

    #[test]
    fn failures_preserve_sample_age_and_valid_recovery_replaces_it() {
        let t=target(&interface("one"));let mut replies=frames(0,64).into_iter();
        let initial=read_with(&t,0,|_,_|replies.next()).unwrap();
        let now=initial.battery_at.unwrap();let lifetime=Duration::from_secs(120);
        let mut state=status::Status::default();
        state.observe_sample(Some(true),initial.battery.as_ref(),initial.battery_at,now,lifetime);
        for seconds in [10,119,120] {
            assert!(read_with(&t,0,|_,_|None).is_err());
            state.observe_sample(None,None,None,now+Duration::from_secs(seconds),lifetime);
            assert_eq!(state.measured_at,Some(now));
        }
        assert_eq!(state.quality,crate::device::ReadingQuality::Stale);
        let mut replies=frames(0,54).into_iter();let next=read_with(&t,0,|_,_|replies.next()).unwrap();
        let later=now+Duration::from_secs(130);
        state.observe_sample(Some(true),next.battery.as_ref(),Some(later),later,lifetime);
        assert_eq!(state.quality,crate::device::ReadingQuality::Live);
        assert_eq!(state.battery.unwrap().estimated_percent,Some(50));
    }
}
