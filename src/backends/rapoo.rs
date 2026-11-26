//! nRF54L 鼠标持续状态报告；配置访问使用独立的兄弟集合。
use super::*;
use crate::hid::HidDevice;
use std::{
    collections::BTreeMap,
    sync::{atomic::{AtomicBool, Ordering}, Arc, Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};

pub struct Rapoo;
const VID: u16 = 0x24ae;
const STATUS_REPORT: u8 = 7;
// VT1 实机活动时约每三秒上报一次；静置停手时会暂停推送，需与其它被动协议一致保留 120 秒有效期。
const FIRST_REPORT_WAIT: Duration = Duration::from_secs(4);
const SAMPLE_LIFETIME: Duration = Duration::from_secs(120);
const SILENT_REOPEN: Duration = Duration::from_secs(30);
const DRAIN_BATCH_LIMIT: usize = 32;

#[derive(Default)]
struct StatusState {
    sample: Option<(Instant, Option<(Battery, Dpi)>, usize)>,
    failed: bool,
}

impl StatusState {
    fn receive(&mut self, frame: &[u8], now: Instant) {
        if frame.first() == Some(&STATUS_REPORT) {
            if let Some(parsed) = parse_status(frame) {
                self.sample = Some((now, Some(parsed), frame.len()));
            } else if frame.len() >= 9 && frame[7] == 0 && frame[2] <= 6 {
                // 完整的离线状态保留为状态证据；损坏报文不覆盖最后有效电量。
                self.sample = Some((now, None, frame.len()));
            }
        }
    }

    fn recent(&self, now: Instant) -> Option<&(Instant, Option<(Battery, Dpi)>, usize)> {
        (!self.failed).then(|| self.sample.as_ref())
            .flatten().filter(|sample| now.saturating_duration_since(sample.0) < SAMPLE_LIFETIME)
    }

    fn confirmed_after(&self,now:Instant,after:Option<Instant>)->Option<&(Instant,Option<(Battery,Dpi)>,usize)> {
        self.recent(now).filter(|s|after.is_none_or(|after|s.0>after))
    }
}

#[derive(Default)]
struct StatusReader {
    state: Mutex<StatusState>,
    changed: Condvar,
    stop: AtomicBool,
}

static READERS: OnceLock<Mutex<BTreeMap<String, Arc<StatusReader>>>> = OnceLock::new();

fn readers() -> &'static Mutex<BTreeMap<String, Arc<StatusReader>>> {
    READERS.get_or_init(Default::default)
}

fn record_status_frame(
    interface: &HidInterface,
    state: &mut StatusState,
    read_started: Instant,
    frame: &[u8],
) {
    if frame.first() == Some(&STATUS_REPORT) {
        trace::record(
            interface,
            BackendId::Rapoo,
            "receive-status",
            0,
            STATUS_REPORT,
            0,
            frame.len(),
            read_started,
            if parse_status(frame).is_some() {
                trace::Outcome::Ok
            } else if frame.len() >= 9 && frame[7] == 0 && frame[2] <= 6 {
                trace::Outcome::Offline
            } else {
                trace::Outcome::InvalidData
            },
        );
    }
    state.receive(frame, Instant::now());
}

fn status_reader(interface: &HidInterface) -> Arc<StatusReader> {
    let mut sessions = readers().lock().unwrap_or_else(|error| error.into_inner());
    if let Some(session) = sessions.get(&interface.path) {
        if !session.state.lock().unwrap_or_else(|error| error.into_inner()).failed {
            return Arc::clone(session);
        }
        session.stop.store(true, Ordering::Release);
    }
    let session = Arc::new(StatusReader::default());
    sessions.insert(interface.path.clone(), Arc::clone(&session));
    let worker = Arc::clone(&session);
    let interface = interface.clone();
    let spawned = std::thread::Builder::new().name("rapoo-status".into()).spawn(move || {
        // HID 句柄只在拥有它的线程中以只读模式打开；界面只读取经校验的采样。
        let started = Instant::now();
        let mut device = match HidDevice::open_read_only(&interface) {
            Ok(device) => device,
            Err(_) => {
                worker.state.lock().unwrap_or_else(|error| error.into_inner()).failed = true;
                worker.changed.notify_all();
                trace::record(&interface, BackendId::Rapoo, "open-status", 0,
                    STATUS_REPORT, 0, 0, started, trace::Outcome::NoReply);
                return;
            }
        };
        trace::record(&interface, BackendId::Rapoo, "listener-open", 0,
            STATUS_REPORT, 0, 0, started, trace::Outcome::Ok);
        let mut last_input = started;
        while !worker.stop.load(Ordering::Acquire) {
            // 无回帧时继续等下一帧，超时不重置仍在有效期内的采样；静默超过阈值时无闪烁重开只读句柄。
            let read_started = Instant::now();
            if let Some(frame) = device.read_report(400) {
                last_input = Instant::now();
                let mut frames = vec![frame];
                while frames.len() < DRAIN_BATCH_LIMIT && !worker.stop.load(Ordering::Acquire) {
                    let Some(extra) = device.read_report(0) else { break; };
                    last_input = Instant::now();
                    frames.push(extra);
                }
                let mut state = worker.state.lock().unwrap_or_else(|error| error.into_inner());
                for frame in &frames {
                    record_status_frame(&interface, &mut state, read_started, frame);
                }
                worker.changed.notify_all();
            } else if read_started.elapsed() < Duration::from_millis(100) {
                trace::record(&interface, BackendId::Rapoo, "listener-immediate-failure", 0,
                    STATUS_REPORT, 0, 0, read_started, trace::Outcome::NoReply);
                // 拔出设备等立即失败不能形成忙循环；下一轮枚举会重建监听。
                worker.state.lock().unwrap_or_else(|error| error.into_inner()).failed = true;
                worker.changed.notify_all();
                break;
            } else if last_input.elapsed() >= SILENT_REOPEN {
                drop(device);
                let reopen_started = Instant::now();
                match HidDevice::open_read_only(&interface) {
                    Ok(reopened) => {
                        device = reopened;
                        last_input = Instant::now();
                        trace::record(&interface, BackendId::Rapoo, "listener-reopen", 0,
                            STATUS_REPORT, 0, 0, reopen_started, trace::Outcome::Ok);
                    }
                    Err(_) => {
                        trace::record(&interface, BackendId::Rapoo, "listener-reopen", 0,
                            STATUS_REPORT, 0, 0, reopen_started, trace::Outcome::NoReply);
                        worker.state.lock().unwrap_or_else(|error| error.into_inner()).failed = true;
                        worker.changed.notify_all();
                        break;
                    }
                }
            }
        }
        if worker.stop.load(Ordering::Acquire) {
            trace::record(&interface, BackendId::Rapoo, "listener-stopped", 0,
                STATUS_REPORT, 0, 0, Instant::now(), trace::Outcome::Ok);
        }
    });
    if spawned.is_err() {
        session.state.lock().unwrap_or_else(|error| error.into_inner()).failed = true;
        session.changed.notify_all();
    }
    session
}

/// 雷柏 nRF54L 设备的通道角色与归并指纹。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LinkRole {
    Receiver,
    Wired,
}

/// 用于识别同一实体雷柏鼠标的有线 (`0x4xxx`) 与 2.4G 接收器 (`< 0x4000`) 双通道签名。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct DeviceSignature {
    pub model: &'static str,
    pub serial: String,
    pub role: LinkRole,
}

pub(crate) fn device_signature(target: &Target) -> Option<DeviceSignature> {
    if target.backend != BackendId::Rapoo {
        return None;
    }
    let interface = target.hid().ok()?;
    let model = model(interface.product_id)?;
    let role = if interface.product_id >= 0x4000 {
        LinkRole::Wired
    } else {
        LinkRole::Receiver
    };
    Some(DeviceSignature {
        model,
        serial: interface.serial.clone(),
        role,
    })
}

// 型号与协议归属：Rapoo AHUB 官方 VT_nrf54L 清单。
fn model(pid: u16) -> Option<&'static str> {
    match pid {
        0x464d => Some("Rapoo ESM612ACE"),
        0x4681 => Some("Rapoo ESM612MAX"),
        0x4631 => Some("Rapoo ESM612PRO"),
        0x4637 => Some("Rapoo ESM622MAX"),
        0x4632 => Some("Rapoo ESM622PRO"),
        0x4633 => Some("Rapoo ESM652PRO"),
        0x4640 => Some("Rapoo ESM672MAX"),
        0x4635 => Some("Rapoo ESM672PRO"),
        0x4610 | 0x1410 => Some("Rapoo VT0"),
        0x46a1 | 0x14a1 => Some("Rapoo VT0 Air MAX"),
        0x4620 | 0x1420 => Some("Rapoo VT0 MAX"),
        0x464a | 0x144a => Some("Rapoo VT0 MAX MASTER"),
        0x4615 | 0x1415 => Some("Rapoo VT1"),
        0x4644 | 0x1444 => Some("Rapoo VT1 MAX"),
        0x4b0a | 0x1b0a => Some("Rapoo VT1S MAX V2"),
        0x466a | 0x146a => Some("Rapoo VT1S V2"),
        0x4622 | 0x1422 => Some("Rapoo VT2"),
        0x4b12 | 0x1b12 => Some("Rapoo VT2 Air MAX V2"),
        0x4623 | 0x1423 => Some("Rapoo VT2 MAX"),
        0x4675 | 0x1475 => Some("Rapoo VT2 MAX MASTER V2"),
        0x4b09 | 0x1b09 => Some("Rapoo VT2 MAX V2"),
        0x4668 | 0x1468 => Some("Rapoo VT2 V2"),
        0x46a4 | 0x14a4 => Some("Rapoo VT2K MAX"),
        0x4b13 | 0x1b13 => Some("Rapoo VT2S Air MAX V2"),
        0x4676 | 0x1476 => Some("Rapoo VT2S MAX MASTER V2"),
        0x4b04 | 0x1b04 => Some("Rapoo VT2S MAX V2"),
        0x465d | 0x145d => Some("Rapoo VT2S V2"),
        0x4612 | 0x1412 => Some("Rapoo VT3"),
        0x4b0c | 0x1b0c => Some("Rapoo VT3 Air MAX V2"),
        0x4617 | 0x1417 => Some("Rapoo VT3 MAX"),
        0x466d | 0x146d => Some("Rapoo VT3 MAX MASTER V2"),
        0x4b06 | 0x1b06 => Some("Rapoo VT3 MAX V2"),
        0x46a5 | 0x14a5 => Some("Rapoo VT3 PRO V2"),
        0x4662 | 0x1462 => Some("Rapoo VT3 V2"),
        0x4611 | 0x1411 => Some("Rapoo VT3S"),
        0x46a2 | 0x14a2 => Some("Rapoo VT3S Air Max"),
        0x4b0d | 0x1b0d => Some("Rapoo VT3S Air MAX V2"),
        0x4621 | 0x1421 => Some("Rapoo VT3S MAX"),
        0x464b | 0x144b => Some("Rapoo VT3S MAX MASTER"),
        0x466e | 0x146e => Some("Rapoo VT3S MAX MASTER V2"),
        0x4b07 | 0x1b07 => Some("Rapoo VT3S MAX V2"),
        0x46a6 | 0x14a6 => Some("Rapoo VT3S PRO V2"),
        0x4664 | 0x1464 => Some("Rapoo VT3S V2"),
        0x4b14 | 0x1b14 => Some("Rapoo VT4 Air MAX V2"),
        0x4677 | 0x1477 => Some("Rapoo VT4 MAX MASTER V2"),
        0x4b03 | 0x1b03 => Some("Rapoo VT4 MAX V2"),
        0x465b | 0x145b => Some("Rapoo VT4 V2"),
        0x4651 | 0x1451 => Some("Rapoo VT6"),
        0x4652 | 0x1452 => Some("Rapoo VT6 MAX"),
        0x4613 | 0x1413 => Some("Rapoo VT7"),
        0x4b0e | 0x1b0e => Some("Rapoo VT7 Air MAX V2"),
        0x4616 | 0x1416 => Some("Rapoo VT7 MAX"),
        0x4670 | 0x1470 => Some("Rapoo VT7 MAX MASTER V2"),
        0x4b05 | 0x1b05 => Some("Rapoo VT7 MAX V2"),
        0x46a7 | 0x14a7 => Some("Rapoo VT7 PRO V2"),
        0x4660 | 0x1460 => Some("Rapoo VT7 V2"),
        0x4b0f | 0x1b0f => Some("Rapoo VT7S Air MAX V2"),
        0x4671 | 0x1471 => Some("Rapoo VT7S MAX MASTER V2"),
        0x4b02 | 0x1b02 => Some("Rapoo VT7S MAX V2"),
        0x46a8 | 0x14a8 => Some("Rapoo VT7S PRO V2"),
        0x4659 | 0x1459 => Some("Rapoo VT7S V2"),
        0x4b15 | 0x1b15 => Some("Rapoo VT8 Air MAX V2"),
        0x4678 | 0x1478 => Some("Rapoo VT8 MAX MASTER V2"),
        0x4b0b | 0x1b0b => Some("Rapoo VT8 MAX V2"),
        0x466c | 0x146c => Some("Rapoo VT8 V2"),
        0x4614 | 0x1414 => Some("Rapoo VT9"),
        0x4b10 | 0x1b10 => Some("Rapoo VT9 Air MAX V2"),
        0x4618 | 0x1418 => Some("Rapoo VT9 MAX"),
        0x4673 | 0x1473 => Some("Rapoo VT9 MAX MASTER V2"),
        0x4b08 | 0x1b08 => Some("Rapoo VT9 MAX V2"),
        0x46a9 | 0x14a9 => Some("Rapoo VT9 PRO V2"),
        0x4666 | 0x1466 => Some("Rapoo VT9 V2"),
        0x4b11 | 0x1b11 => Some("Rapoo VT9S Air MAX V2"),
        0x4674 | 0x1474 => Some("Rapoo VT9S MAX MASTER V2"),
        0x4b01 | 0x1b01 => Some("Rapoo VT9S MAX V2"),
        0x46aa | 0x14aa => Some("Rapoo VT9S PRO V2"),
        0x4653 | 0x1453 => Some("Rapoo VT9S V2"),
        _ => None,
    }
}

fn is_control(interface: &HidInterface) -> bool {
    interface.vendor_id == VID
        && model(interface.product_id).is_some()
        && interface.usage_page == 0xff00
        && interface.usage == 2
        && (9..=65).contains(&interface.input_len)
}

fn parse_status(frame: &[u8]) -> Option<(Battery, Dpi)> {
    // 电量和 X/Y DPI 需要完整的前八个数据字节；配置索引属于可选扩展。
    if frame.len() < 9 || frame[0] != STATUS_REPORT {
        return None;
    }
    let data = &frame[1..];
    let charging = match data[6] {
        1 => false,
        2 | 3 => true,
        _ => return None,
    };
    if data[7] > 100 || data[1] > 6 || data.get(11).is_some_and(|index| *index > 7) {
        return None;
    }
    let x = u16::from_le_bytes([data[2], data[3]]);
    let y = u16::from_le_bytes([data[4], data[5]]);
    let valid = (50..u16::MAX).contains(&x) && (50..u16::MAX).contains(&y);
    Some((
        Battery {
            estimated_percent: None,
            percent: Some(data[7]),
            charging: Some(charging),
            voltage_mv: None,
            approximate: false,
            level: None,
        },
        Dpi {
            current: valid.then_some(x),
            current_y: valid.then_some(y),
            active_stage: valid.then_some(data[1]),
            profile: data.get(11).copied().map(u16::from),
            values: Vec::new(),
            writable: false,
            ..Default::default()
        },
    ))
}

fn offline_snapshot(target: &Target) -> Snapshot {
    Snapshot {
        target: target.clone(), battery: None, dpi: Dpi::default(), connected: false,
        battery_unavailable: true, battery_at: None, polling_rate: Default::default(),
    }
}

fn read_with(target: &Target,
    mut read_status: impl FnMut(Duration) -> Result<Option<(Instant, Battery, Dpi)>, String>,
    read_config: impl FnOnce(&Dpi) -> Result<Dpi, String>) -> Result<Snapshot, String> {
    let Some((mut taken, mut battery, status)) = read_status(FIRST_REPORT_WAIT)? else {
        return Ok(offline_snapshot(target));
    };
    let mut dpi = read_config(&status).unwrap_or(status);
    match read_status(Duration::ZERO) {
        Ok(Some((new_taken, new_battery, new_status))) => {
            taken = new_taken;
            battery = new_battery;
            if dpi.profile != new_status.profile || dpi.active_stage != new_status.active_stage
                || dpi.current != new_status.current || dpi.current_y != new_status.current_y {
                dpi = new_status;
            }
        },
        Ok(None) => return Ok(offline_snapshot(target)),
        Err(_) => dpi.disable_writes(),
    }
    Ok(Snapshot {
        target: target.clone(), battery: Some(battery), dpi, connected: true,
        battery_unavailable: false, battery_at: Some(taken), polling_rate: Default::default(),
    })
}

impl DeviceBackend for Rapoo {
    fn discover(&self, _: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        // 用完整接口路径隔离设备；拔出后停止旧监听，重连不沿用旧采样。
        readers().lock().unwrap_or_else(|error| error.into_inner()).retain(|path, session| {
            let present = interfaces.iter().any(|interface| is_control(interface) && &interface.path == path);
            if !present { session.stop.store(true, Ordering::Release); }
            present
        });
        Ok(interfaces
            .iter()
            .filter(|interface| is_control(interface))
            .map(|interface| {
                let mut target = Target::new(BackendId::Rapoo, interface, 0);
                target.name = model(interface.product_id).unwrap().into();
                target.family = DeviceFamily::Mouse;
                target
            })
            .collect())
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        if !is_control(target.hid()?) {
            return Err(crate::tr!("此雷柏型号尚未适配").into());
        }
        // Configuration failure never clears independently received battery data.
        read_with(target, |wait| latest_sample(target, None, wait),
            |status| super::rapoo_config::read(api, target, status))
    }

    fn set_dpi_stage(&self,api:&HidApi,target:&Target,stage:crate::device::DpiStage,context:crate::device::StageContext)->Result<(),String> {
        if !is_control(target.hid()?) {return Err(crate::tr!("此雷柏型号尚未适配").into());}
        super::rapoo_config::switch(api,target,stage,context)
    }
}

pub(super) fn latest_status(target:&Target,after:Option<Instant>,wait:Duration)->Result<(Instant,Battery,Dpi),String> {
    latest_sample(target, after, wait)?
        .ok_or_else(|| crate::tr!("暂未收到设备状态更新，请轻动鼠标唤醒后再试").into())
}

fn latest_sample(target: &Target, after: Option<Instant>, wait: Duration)
    -> Result<Option<(Instant, Battery, Dpi)>, String> {
    let session=status_reader(target.hid()?);
    status_from_session(target, &session, after, wait)
}

fn status_from_session(target: &Target, session: &StatusReader, after: Option<Instant>,
    wait: Duration) -> Result<Option<(Instant, Battery, Dpi)>, String> {
    let started=Instant::now();
    let state=session.state.lock().unwrap_or_else(|e|e.into_inner());
    let (state,_)=session.changed.wait_timeout_while(state,wait,|s| !s.failed &&
        s.confirmed_after(Instant::now(),after).is_none())
        .unwrap_or_else(|e|e.into_inner());
    let sample=state.confirmed_after(Instant::now(),after);
    let result=sample.map(|s|s.1.clone().map(|(battery,dpi)|(s.0,battery,dpi)));
    let outcome = match &result {
        Some(Some(_)) => trace::Outcome::Ok,
        Some(None) => trace::Outcome::Offline,
        None => trace::Outcome::NoReply,
    };
    trace::record_sample(target.hid()?, BackendId::Rapoo, "read-cached-status", 0, STATUS_REPORT, 0,
        sample.map(|sample| sample.2).unwrap_or(0), started, outcome,
        state.sample.as_ref().map(|sample| sample.0));
    result.ok_or_else(||crate::tr!("暂未收到设备状态更新，请轻动鼠标唤醒后再试").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_target() -> Target {
        Target::new(BackendId::Rapoo, &HidInterface {
            vendor_id: VID, product_id: 0x4b01, usage_page: 0xff00, usage: 2,
            input_len: 19, output_len: 0, feature_len: 0, serial: String::new(),
            product: "Rapoo fixture".into(), path: "rapoo-isolated-fixture".into(),
        }, 0)
    }

    #[test]
    fn offline_during_configuration_clears_the_published_snapshot() {
        for configuration_succeeds in [false, true] {
            let target = test_target();
            let session = StatusReader::default();
            session.state.lock().unwrap().receive(&report(), Instant::now());
            let sample = read_with(&target,
                |wait| status_from_session(&target, &session, None, wait),
                |status| {
                    let mut offline = report();
                    offline[7] = 0;
                    session.state.lock().unwrap().receive(&offline, Instant::now());
                    if configuration_succeeds { Ok(status.clone()) }
                    else { Err("configuration unavailable".into()) }
                }).unwrap();
            assert!(!sample.connected);
            assert!(sample.battery.is_none());
            assert!(sample.battery_at.is_none());
            assert!(sample.battery_unavailable);
            assert!(!sample.dpi.writable);
        }
    }

    #[test]
    fn initial_offline_status_skips_configuration_and_does_not_affect_another_receiver() {
        let target = test_target();
        let offline = StatusReader::default();
        let independent = StatusReader::default();
        let mut frame = report();
        frame[7] = 0;
        offline.state.lock().unwrap().receive(&frame, Instant::now());
        frame[7] = 1;
        frame[8] = 42;
        let measured = Instant::now();
        independent.state.lock().unwrap().receive(&frame, measured);
        let sample = read_with(&target,
            |wait| status_from_session(&target, &offline, None, wait),
            |_| panic!("offline receiver must not query configuration")).unwrap();
        assert!(!sample.connected);
        assert!(sample.battery_unavailable);
        let mut other_interface = target.hid().unwrap().clone();
        other_interface.path = "independent-rapoo-fixture".into();
        let other = Target::new(BackendId::Rapoo, &other_interface, 0);
        let sample = read_with(&other,
            |wait| status_from_session(&other, &independent, None, wait),
            |_| Err("configuration unavailable".into())).unwrap();
        assert_ne!(target.key, sample.target.key);
        assert!(sample.connected);
        assert_eq!(sample.battery.unwrap().percent, Some(42));
        assert_eq!(sample.battery_at, Some(measured));
    }

    #[test]
    fn configuration_failure_and_silence_preserve_valid_sample_without_renewing_it() {
        let target = test_target();
        let session = StatusReader::default();
        let measured = Instant::now() - Duration::from_secs(1);
        session.state.lock().unwrap().receive(&report(), measured);
        let sample = read_with(&target,
            |wait| status_from_session(&target, &session, None, wait),
            |_| Err("configuration unavailable".into())).unwrap();
        assert!(sample.connected);
        assert!(!sample.battery_unavailable);
        assert_eq!(sample.battery.unwrap().percent, Some(75));
        assert_eq!(sample.battery_at, Some(measured));
    }

    #[test]
    fn failed_status_recheck_disables_writes_without_inventing_an_offline_frame() {
        let target = test_target();
        let session = StatusReader::default();
        let measured = Instant::now();
        session.state.lock().unwrap().receive(&report(), measured);
        let sample = read_with(&target,
            |wait| status_from_session(&target, &session, None, wait),
            |status| {
                session.state.lock().unwrap().failed = true;
                let mut configured = status.clone();
                configured.writable = true;
                Ok(configured)
            }).unwrap();
        assert!(sample.connected);
        assert_eq!(sample.battery.unwrap().percent, Some(75));
        assert_eq!(sample.battery_at, Some(measured));
        assert!(!sample.dpi.writable);
    }

    #[test]
    fn newer_valid_status_during_configuration_replaces_battery_and_dpi() {
        let target = test_target();
        let session = StatusReader::default();
        let updated_at = Instant::now();
        session.state.lock().unwrap().receive(&report(), updated_at - Duration::from_secs(1));
        let sample = read_with(&target,
            |wait| status_from_session(&target, &session, None, wait),
            |status| {
                let mut updated = report();
                updated[8] = 0;
                updated[7] = 2;
                updated[3..5].copy_from_slice(&1200u16.to_le_bytes());
                session.state.lock().unwrap().receive(&updated, updated_at);
                let mut configured = status.clone();
                configured.writable = true;
                Ok(configured)
            }).unwrap();
        assert!(sample.connected);
        let battery = sample.battery.unwrap();
        assert_eq!(battery.percent, Some(0));
        assert_eq!(battery.charging, Some(true));
        assert_eq!(sample.battery_at, Some(updated_at));
        assert_eq!(sample.dpi.current, Some(1200));
        assert!(!sample.dpi.writable);
    }

    #[test]
    fn expanded_catalog_requires_the_nrf54l_status_interface() {
        let mut interface = HidInterface {
            vendor_id: VID,
            product_id: 0x4b01,
            usage_page: 0xff00,
            usage: 2,
            input_len: 19,
            output_len: 0,
            feature_len: 0,
            path: String::new(),
            serial: String::new(),
            product: String::new(),
        };
        assert_eq!(model(interface.product_id), Some("Rapoo VT9S MAX V2"));
        assert!(is_control(&interface));
        interface.product_id = 0x1b01;
        assert!(is_control(&interface));
        interface.usage = 14;
        assert!(!is_control(&interface));
        interface.usage = 2;
        interface.product_id = 0x0001;
        assert!(!is_control(&interface));
        interface.product_id = 0x4b01;
        interface.input_len = 8;
        assert!(!is_control(&interface));
    }
    fn report() -> Vec<u8> {
        vec![7, 0, 1, 0x20, 3, 0x20, 3, 1, 75, 0, 0, 0, 0]
    }
    #[test]
    fn status_has_real_percentage_and_read_only_dpi() {
        let (battery, dpi) = parse_status(&report()).unwrap();
        assert_eq!(battery.percent, Some(75));
        assert_eq!(battery.charging, Some(false));
        assert_eq!(dpi.current, Some(800));
        assert!(!dpi.writable);
        assert_eq!(parse_status(&report()[..9]).unwrap().1.current, Some(800));
        let mut frame = report();
        frame[7] = 3;
        frame[8] = 0;
        assert_eq!(parse_status(&frame).unwrap().0.percent, Some(0));
        assert_eq!(parse_status(&frame).unwrap().0.charging, Some(true));
    }
    #[test]
    fn offline_truncated_and_invalid_reports_are_rejected() {
        let mut frame = report();
        assert!(parse_status(&frame[..8]).is_none());
        frame[7] = 0;
        assert!(parse_status(&frame).is_none());
        frame[7] = 1;
        frame[8] = 101;
        assert!(parse_status(&frame).is_none());
        frame[8] = 75;
        frame[12] = 8;
        assert!(parse_status(&frame).is_none());
    }
    #[test]
    fn independent_axes_retain_both_values_without_authorizing_scalar_writes() {
        let mut frame = report();
        frame[5] = 0x40;
        let dpi=parse_status(&frame).unwrap().1;
        assert_eq!(dpi.current,Some(800));
        assert_eq!(dpi.current_y,Some(832));
        assert!(!dpi.writable);
    }

    #[test]
    fn telemetry_remains_available_between_three_second_reports_then_expires() {
        let started = Instant::now();
        let mut state = StatusState::default();
        state.receive(&report(), started);
        assert_eq!(state.recent(started + Duration::from_secs(3)).unwrap().1.as_ref().unwrap().0.percent, Some(75));
        state.receive(&report(), started + Duration::from_secs(3));
        assert!(state.recent(started + Duration::from_secs(10)).is_some());
        assert!(state.recent(started + Duration::from_secs(11)).is_none());
    }

    #[test]
    fn offline_status_clears_the_previous_battery_and_next_report_recovers() {
        let now = Instant::now();
        let mut state = StatusState::default();
        let mut frame = report();
        state.receive(&frame, now);
        frame[7] = 0;
        state.receive(&frame, now);
        assert!(state.recent(now).unwrap().1.is_none());
        frame[7] = 1;
        frame[8] = 70;
        state.receive(&frame, now);
        assert_eq!(state.recent(now).unwrap().1.as_ref().unwrap().0.percent, Some(70));
        state.failed = true;
        assert!(state.recent(now).is_none());
    }

    #[test]
    fn unrelated_reports_cannot_clear_or_refresh_the_battery_sample() {
        let now = Instant::now();
        let mut state = StatusState::default();
        state.receive(&report(), now);
        state.receive(&[6, 1, 2, 3], now + Duration::from_secs(7));
        assert!(state.recent(now + SAMPLE_LIFETIME).is_none());
    }

    #[test]
    fn invalid_or_truncated_status_cannot_replace_or_extend_valid_battery() {
        let now = Instant::now();
        let mut state = StatusState::default();
        state.receive(&report(), now);
        let mut invalid = report(); invalid[8] = 255;
        state.receive(&invalid, now + Duration::from_secs(3));
        state.receive(&[7, 0, 0], now + Duration::from_secs(6));
        let sample = state.recent(now + Duration::from_secs(7)).unwrap();
        assert_eq!(sample.0, now);
        assert_eq!(sample.1.as_ref().unwrap().0.percent, Some(75));
        assert!(state.recent(now + SAMPLE_LIFETIME).is_none());
    }
    #[test]
    fn stage_confirmation_requires_a_new_valid_report_after_the_write() {
        let now=Instant::now();let mut state=StatusState::default();
        state.receive(&report(),now);
        assert!(state.confirmed_after(now,Some(now)).is_none());
        state.receive(&[8,1,0,0],now+Duration::from_millis(1));
        assert!(state.confirmed_after(now+Duration::from_millis(1),Some(now)).is_none());
        let mut frame=report();frame[2]=6;frame[3..5].copy_from_slice(&25600u16.to_le_bytes());
        frame[5..7].copy_from_slice(&30000u16.to_le_bytes());
        state.receive(&frame,now+Duration::from_secs(3));
        let dpi=&state.confirmed_after(now+Duration::from_secs(3),Some(now)).unwrap().1.as_ref().unwrap().1;
        assert_eq!(dpi.active_stage,Some(6));assert_eq!(dpi.current_y,Some(30000));
        assert!(state.confirmed_after(now+Duration::from_secs(11),Some(now)).is_some());
        assert!(state.confirmed_after(now+Duration::from_secs(3)+SAMPLE_LIFETIME,Some(now)).is_none());
    }

    #[test]
    fn sample_survives_brief_idle_pause_beyond_eight_seconds() {
        let now = Instant::now();
        let mut state = StatusState::default();
        state.receive(&report(), now);
        assert!(state.recent(now + Duration::from_secs(25)).is_some());
        assert!(state.recent(now + SAMPLE_LIFETIME - Duration::from_secs(1)).is_some());
        assert!(state.recent(now + SAMPLE_LIFETIME).is_none());
    }

    #[test]
    fn device_signature_distinguishes_wired_and_receiver_roles() {
        let mk = |pid: u16, serial: &str, path: &str| Target::new(BackendId::Rapoo, &HidInterface {
            vendor_id: VID, product_id: pid, usage_page: 0xff00, usage: 2,
            input_len: 19, output_len: 0, feature_len: 0, serial: serial.into(),
            product: "Rapoo VT1".into(), path: path.into(),
        }, 0);
        let rx = device_signature(&mk(0x1415, "2507-54L", "rx-a")).unwrap();
        let wired = device_signature(&mk(0x4615, "2507-54L", "wired-a")).unwrap();
        assert_eq!(rx.model, "Rapoo VT1");
        assert_eq!(wired.model, "Rapoo VT1");
        assert_eq!(rx.role, LinkRole::Receiver);
        assert_eq!(wired.role, LinkRole::Wired);
    }
}
