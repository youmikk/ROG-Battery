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

#[derive(Clone, Debug, Default)]
pub struct Endpoint {
    pub id: String,
    pub name: String,
    pub volume: u8,
    pub muted: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub speaker: Option<Endpoint>,
    pub headset: Option<Endpoint>,
    pub microphone: Option<Endpoint>,
    pub error: Option<String>,
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
