use core::ptr;

use winapi::ctypes::c_void;
use winapi::shared::minwindef::DWORD;
use winapi::shared::winerror::{RPC_E_CHANGED_MODE, S_FALSE, S_OK};
use winapi::shared::wtypes::VT_LPWSTR;
use winapi::um::combaseapi::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
};
use winapi::um::endpointvolume::IAudioEndpointVolume;
use winapi::um::functiondiscoverykeys_devpkey::PKEY_Device_FriendlyName;
use winapi::um::mmdeviceapi::{
    eCapture, eCommunications, eMultimedia, eRender, CLSID_MMDeviceEnumerator, IMMDevice,
    IMMDeviceCollection, IMMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use winapi::um::objbase::COINIT_APARTMENTTHREADED;
use winapi::um::propidl::{PropVariantClear, PROPVARIANT};
use winapi::um::propsys::IPropertyStore;
use winapi::Interface;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Endpoint {
    pub id: String,
    pub name: String,
    pub volume: u8,
    pub muted: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub speaker: Option<Endpoint>,
    pub headset: Option<Endpoint>,
    pub microphone: Option<Endpoint>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Slot {
    #[default]
    Speaker,
    Headset,
}

impl Snapshot {
    pub fn output(&self, slot: Slot) -> Option<&Endpoint> {
        match slot {
            Slot::Speaker => self.speaker.as_ref(),
            Slot::Headset => self.headset.as_ref(),
        }
    }

    fn output_mut(&mut self, slot: Slot) -> Option<&mut Endpoint> {
        match slot {
            Slot::Speaker => self.speaker.as_mut(),
            Slot::Headset => self.headset.as_mut(),
        }
    }
}

enum Request {
    Refresh,
    Volume(String, u8),
    Mute(String),
}

#[derive(Default)]
struct CachedAudio {
    snapshot: Snapshot,
    revision: u64,
}

/// 两个窗口共用音频状态；COM 读取和写入只在工作线程执行。
#[derive(Clone)]
pub struct Controls {
    cache: std::sync::Arc<std::sync::Mutex<CachedAudio>>,
    active: std::sync::Arc<std::sync::atomic::AtomicU8>,
    sender: std::sync::mpsc::Sender<Request>,
}

impl Controls {
    pub fn new(notify: impl Fn() + Send + 'static) -> Self {
        use std::sync::{atomic::Ordering, mpsc, Arc, Mutex};
        let cache = Arc::new(Mutex::new(CachedAudio::default()));
        let active = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let (sender, receiver) = mpsc::channel();
        let worker_cache = Arc::clone(&cache);
        let worker_active = Arc::clone(&active);
        std::thread::spawn(move || loop {
            let request = match receiver.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(request) => request,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if worker_active.load(Ordering::Relaxed) == 0 {
                        continue;
                    }
                    Request::Refresh
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            let mut requests = vec![request];
            let cache = worker_cache.lock().unwrap_or_else(|e| e.into_inner());
            for request in receiver.try_iter() {
                queue_request(&mut requests, request);
            }
            let revision = cache.revision;
            drop(cache);
            let mut error = None;
            for request in requests {
                let result = match request {
                    Request::Refresh => Ok(()),
                    Request::Volume(id, percent) => set_volume(&id, percent),
                    Request::Mute(id) => toggle_mute(&id).map(|_| ()),
                };
                if let Err(value) = result {
                    error = Some(value);
                }
            }
            let mut fresh = snapshot();
            if error.is_some() {
                fresh.error = error;
            }
            let mut cache = worker_cache.lock().unwrap_or_else(|e| e.into_inner());
            // 拖动期间的新值不能被较早开始的读取覆盖。
            if cache.revision == revision && cache.snapshot != fresh {
                cache.snapshot = fresh;
                drop(cache);
                notify();
            }
        });
        Self { cache, active, sender }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).snapshot.clone()
    }

    pub fn set_active(&self, window: u8, on: bool) {
        use std::sync::atomic::Ordering;
        let previous = if on {
            self.active.fetch_or(window, Ordering::Relaxed)
        } else {
            self.active.fetch_and(!window, Ordering::Relaxed)
        };
        if on && previous & window == 0 {
            self.refresh();
        }
    }

    pub fn refresh(&self) {
        let _ = self.sender.send(Request::Refresh);
    }

    pub fn set_volume(&self, slot: Slot, percent: u8) {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let Some(endpoint) = cache.snapshot.output_mut(slot) else { return };
        let percent = percent.min(100);
        if endpoint.volume == percent {
            return;
        }
        endpoint.volume = percent;
        let id = endpoint.id.clone();
        cache.snapshot.error = None;
        cache.revision = cache.revision.wrapping_add(1);
        let _ = self.sender.send(Request::Volume(id, percent));
    }

    pub fn toggle_microphone(&self) {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let Some(endpoint) = cache.snapshot.microphone.as_mut() else { return };
        endpoint.muted = !endpoint.muted;
        let id = endpoint.id.clone();
        cache.snapshot.error = None;
        cache.revision = cache.revision.wrapping_add(1);
        let _ = self.sender.send(Request::Mute(id));
    }

    pub fn toggle_output(&self, slot: Slot) {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let Some(endpoint) = cache.snapshot.output_mut(slot) else { return };
        endpoint.muted = !endpoint.muted;
        let id = endpoint.id.clone();
        cache.snapshot.error = None;
        cache.revision = cache.revision.wrapping_add(1);
        let _ = self.sender.send(Request::Mute(id));
    }
}

fn queue_request(requests: &mut Vec<Request>, request: Request) {
    // 连续拖动只保留同一端点的最新音量，麦克风切换保持原顺序。
    if let (Some(Request::Volume(previous, _)), Request::Volume(next, _)) =
        (requests.last(), &request)
    {
        if previous == next {
            requests.pop();
        }
    }
    requests.push(request);
}

struct ComGuard {
    uninitialize: bool,
}

impl ComGuard {
    fn new() -> Result<Self, String> {
        let result = unsafe { CoInitializeEx(ptr::null_mut(), COINIT_APARTMENTTHREADED as DWORD) };
        if result == S_OK || result == S_FALSE {
            Ok(Self { uninitialize: true })
        } else if result == RPC_E_CHANGED_MODE {
            Ok(Self {
                uninitialize: false,
            })
        } else {
            Err(format!(
                "初始化 Windows 音频接口失败：0x{:08X}",
                result as u32
            ))
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.uninitialize {
            unsafe { CoUninitialize() };
        }
    }
}

fn wide(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe {
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }
}

fn friendly_name(device: *mut IMMDevice) -> String {
    let mut store: *mut IPropertyStore = ptr::null_mut();
    let result = unsafe { (*device).OpenPropertyStore(0, &mut store) };
    if result < 0 || store.is_null() {
        return String::new();
    }
    let mut value: PROPVARIANT = unsafe { std::mem::zeroed() };
    let result = unsafe { (*store).GetValue(&PKEY_Device_FriendlyName, &mut value) };
    let name = if result >= 0 && u32::from(value.vt) == VT_LPWSTR {
        wide(unsafe { *value.data.pwszVal() })
    } else {
        String::new()
    };
    unsafe {
        PropVariantClear(&mut value);
        (*store).Release();
    }
    name
}

fn endpoint_id(device: *mut IMMDevice) -> String {
    let mut raw: *mut u16 = ptr::null_mut();
    let result = unsafe { (*device).GetId(&mut raw) };
    if result < 0 || raw.is_null() {
        return String::new();
    }
    let id = wide(raw);
    unsafe { CoTaskMemFree(raw.cast()) };
    id
}

fn volume_interface(device: *mut IMMDevice) -> Result<*mut IAudioEndpointVolume, String> {
    let mut raw: *mut c_void = ptr::null_mut();
    let result = unsafe {
        (*device).Activate(
            &IAudioEndpointVolume::uuidof(),
            CLSCTX_ALL,
            ptr::null_mut(),
            &mut raw,
        )
    };
    if result < 0 || raw.is_null() {
        Err(format!("打开音频端点失败：0x{:08X}", result as u32))
    } else {
        Ok(raw.cast())
    }
}

fn endpoint(device: *mut IMMDevice) -> Result<Endpoint, String> {
    let name = friendly_name(device);
    let id = endpoint_id(device);
    if id.is_empty() {
        return Err("音频端点没有可识别 ID".to_string());
    }
    let volume = volume_interface(device)?;
    let mut scalar = 0.0f32;
    let mut muted = 0;
    let level_result = unsafe { (*volume).GetMasterVolumeLevelScalar(&mut scalar) };
    let mute_result = unsafe { (*volume).GetMute(&mut muted) };
    unsafe { (*volume).Release() };
    if level_result < 0 {
        return Err(format!("读取音量失败：0x{:08X}", level_result as u32));
    }
    Ok(Endpoint {
        id,
        name: if name.trim().is_empty() {
            "未命名音频设备".to_string()
        } else {
            name
        },
        volume: (scalar * 100.0).round().clamp(0.0, 100.0) as u8,
        muted: mute_result >= 0 && muted != 0,
    })
}

fn create_enumerator() -> Result<*mut IMMDeviceEnumerator, String> {
    let mut raw: *mut c_void = ptr::null_mut();
    let result = unsafe {
        CoCreateInstance(
            &CLSID_MMDeviceEnumerator,
            ptr::null_mut(),
            CLSCTX_ALL,
            &IMMDeviceEnumerator::uuidof(),
            &mut raw,
        )
    };
    let enumerator: *mut IMMDeviceEnumerator = raw.cast();
    if result < 0 || enumerator.is_null() {
        Err(format!(
            "创建 Windows 音频设备枚举器失败：0x{:08X}",
            result as u32
        ))
    } else {
        Ok(enumerator)
    }
}

fn render_endpoints(enumerator: *mut IMMDeviceEnumerator) -> Result<Vec<Endpoint>, String> {
    let mut collection: *mut IMMDeviceCollection = ptr::null_mut();
    let result =
        unsafe { (*enumerator).EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE, &mut collection) };
    if result < 0 || collection.is_null() {
        return Err(format!("枚举扬声器设备失败：0x{:08X}", result as u32));
    }
    let count = 0u32;
    let result = unsafe { (*collection).GetCount(&count) };
    if result < 0 {
        unsafe { (*collection).Release() };
        return Err(format!("读取扬声器数量失败：0x{:08X}", result as u32));
    }
    let mut endpoints = Vec::new();
    for index in 0..count {
        let mut device: *mut IMMDevice = ptr::null_mut();
        let result = unsafe { (*collection).Item(index, &mut device) };
        if result < 0 || device.is_null() {
            continue;
        }
        if let Ok(value) = endpoint(device) {
            endpoints.push(value);
        }
        unsafe { (*device).Release() };
    }
    unsafe { (*collection).Release() };
    Ok(endpoints)
}

fn default_endpoint(
    enumerator: *mut IMMDeviceEnumerator,
    flow: winapi::um::mmdeviceapi::EDataFlow,
    role: winapi::um::mmdeviceapi::ERole,
) -> Result<Endpoint, String> {
    let mut device: *mut IMMDevice = ptr::null_mut();
    let result = unsafe { (*enumerator).GetDefaultAudioEndpoint(flow, role, &mut device) };
    if result < 0 || device.is_null() {
        return Err(format!(
            "Windows 没有可用的默认音频设备：0x{:08X}",
            result as u32
        ));
    }
    let value = endpoint(device);
    unsafe { (*device).Release() };
    value
}

fn headset_like(name: &str) -> bool {
    let lower = name.to_lowercase();
    [
        "headset",
        "headphone",
        "earphone",
        "earbuds",
        "耳机",
        "耳麦",
        "耳罩",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub fn snapshot() -> Snapshot {
    let result = (|| {
        let _guard = ComGuard::new()?;
        let enumerator = create_enumerator()?;
        let render = render_endpoints(enumerator).unwrap_or_default();
        let microphone = default_endpoint(enumerator, eCapture, eCommunications).ok();
        let default_render = default_endpoint(enumerator, eRender, eMultimedia).ok();

        let mut headset = render
            .iter()
            .find(|endpoint| headset_like(&endpoint.name))
            .cloned();
        let mut speaker = default_render.clone().or_else(|| render.first().cloned());
        if speaker
            .as_ref()
            .is_some_and(|endpoint| headset_like(&endpoint.name))
        {
            headset = speaker.clone();
            speaker = render
                .iter()
                .find(|endpoint| !headset_like(&endpoint.name))
                .cloned();
        }
        unsafe { (*enumerator).Release() };
        Ok(Snapshot {
            speaker,
            headset,
            microphone,
            error: None,
        })
    })();
    result.unwrap_or_else(|error| Snapshot {
        error: Some(error),
        ..Default::default()
    })
}

fn with_volume<T>(
    id: &str,
    action: impl FnOnce(*mut IAudioEndpointVolume) -> Result<T, String>,
) -> Result<T, String> {
    let _guard = ComGuard::new()?;
    let enumerator = create_enumerator()?;
    let mut wide_id: Vec<u16> = id.encode_utf16().collect();
    wide_id.push(0);
    let mut device: *mut IMMDevice = ptr::null_mut();
    let result = unsafe { (*enumerator).GetDevice(wide_id.as_ptr(), &mut device) };
    unsafe { (*enumerator).Release() };
    if result < 0 || device.is_null() {
        return Err(format!("找不到音频设备：0x{:08X}", result as u32));
    }
    let volume = volume_interface(device);
    unsafe { (*device).Release() };
    let volume = volume?;
    let result = action(volume);
    unsafe { (*volume).Release() };
    result
}

pub fn set_volume(id: &str, percent: u8) -> Result<(), String> {
    let value = f32::from(percent.min(100)) / 100.0;
    with_volume(id, |volume| {
        let result = unsafe { (*volume).SetMasterVolumeLevelScalar(value, ptr::null()) };
        if result < 0 {
            Err(format!("设置音量失败：0x{:08X}", result as u32))
        } else {
            Ok(())
        }
    })
}

pub fn toggle_mute(id: &str) -> Result<bool, String> {
    with_volume(id, |volume| {
        let mut current = 0;
        let read = unsafe { (*volume).GetMute(&mut current) };
        if read < 0 {
            return Err(format!("读取静音状态失败：0x{:08X}", read as u32));
        }
        let next = if current == 0 { 1 } else { 0 };
        let result = unsafe { (*volume).SetMute(next, ptr::null()) };
        if result < 0 {
            Err(format!("切换静音失败：0x{:08X}", result as u32))
        } else {
            Ok(next != 0)
        }
    })
}

pub fn open_sound_settings() -> Result<(), String> {
    crate::dynlight::shell_open("ms-settings:sound")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_drag_keeps_latest_value_for_each_endpoint() {
        let mut requests = Vec::new();
        for (id, volume) in [("speaker", 20), ("speaker", 21), ("speaker", 22),
            ("headset", 60), ("headset", 61)] {
            queue_request(&mut requests, Request::Volume(id.to_string(), volume));
        }
        assert_eq!(requests.len(), 2);
        assert!(matches!(&requests[0], Request::Volume(id, 22) if id == "speaker"));
        assert!(matches!(&requests[1], Request::Volume(id, 61) if id == "headset"));
    }

    #[test]
    fn volume_drag_preserves_microphone_toggles_and_order() {
        let mut requests = vec![Request::Volume("speaker".to_string(), 20)];
        queue_request(&mut requests, Request::Mute("mic".to_string()));
        queue_request(&mut requests, Request::Volume("speaker".to_string(), 30));
        queue_request(&mut requests, Request::Volume("speaker".to_string(), 40));
        queue_request(&mut requests, Request::Mute("mic".to_string()));
        assert_eq!(requests.len(), 4);
        assert!(matches!(&requests[0], Request::Volume(_, 20)));
        assert!(matches!(&requests[1], Request::Mute(id) if id == "mic"));
        assert!(matches!(&requests[2], Request::Volume(_, 40)));
        assert!(matches!(&requests[3], Request::Mute(id) if id == "mic"));
    }
}
