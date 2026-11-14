//! HID Usage Tables: Generic Device Controls / Battery Strength (06:20).
//! 只读取描述符公开的电量字段；不把毫伏或剩余容量当百分比。
use super::*;
use crate::win32::*;
use winapi::shared::hidpi::HIDP_VALUE_CAPS;

pub struct StandardHid;

struct Descriptor {
    handle: Handle,
    preparsed: *mut core::ffi::c_void,
    api: HidApi,
}

impl Descriptor {
    fn open(api: &HidApi, interface: &HidInterface) -> Result<Self, String> {
        let path = to_wide(&interface.path);
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                core::ptr::null_mut(),
                OPEN_EXISTING,
                0,
                core::ptr::null_mut(),
            )
        };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err("HID 描述符不可读取".into());
        }
        let mut preparsed = core::ptr::null_mut();
        if unsafe { (api.get_preparsed_data)(handle, &mut preparsed) } == 0 {
            unsafe {
                CloseHandle(handle);
            }
            return Err("HID 描述符不可解析".into());
        }
        Ok(Self {
            handle,
            preparsed,
            api: *api,
        })
    }

    fn battery_caps(&self, report_type: u32) -> Vec<HIDP_VALUE_CAPS> {
        let mut caps = HidpCaps::default();
        if unsafe { (self.api.get_caps)(self.preparsed, &mut caps) } < 0 {
            return Vec::new();
        }
        let mut length = if report_type == 2 {
            caps.number_feature_value_caps
        } else {
            caps.number_input_value_caps
        };
        if length == 0 || length > 512 {
            return Vec::new();
        }
        let mut values: Vec<HIDP_VALUE_CAPS> = (0..length)
            .map(|_| unsafe { core::mem::zeroed() })
            .collect();
        if unsafe {
            (self.api.get_value_caps)(
                report_type,
                values.as_mut_ptr(),
                &mut length,
                self.preparsed,
            )
        } < 0
        {
            return Vec::new();
        }
        values.truncate(usize::from(length));
        values.retain(|value| {
            let contains = unsafe {
                if value.IsRange != 0 {
                    let range = value.u.Range();
                    range.UsageMin <= 0x20 && range.UsageMax >= 0x20
                } else {
                    value.u.NotRange().Usage == 0x20
                }
            };
            value.UsagePage == 6
                && contains
                && value.ReportCount == 1
                && value.LogicalMin >= 0
                && value.LogicalMax > value.LogicalMin
        });
        values
    }

    fn percent(&self, report_type: u32, cap: &HIDP_VALUE_CAPS, report: &mut [u8]) -> Option<u8> {
        if report.first().copied() != Some(cap.ReportID) {
            return None;
        }
        let mut value = 0;
        let status = unsafe {
            (self.api.get_usage_value)(
                report_type,
                6,
                cap.LinkCollection,
                0x20,
                &mut value,
                self.preparsed,
                report.as_mut_ptr(),
                report.len() as u32,
            )
        };
        if status < 0 {
            return None;
        }
        normalize_percent(value, cap.LogicalMin, cap.LogicalMax)
    }
}

impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            (self.api.free_preparsed_data)(self.preparsed);
            CloseHandle(self.handle);
        }
    }
}

fn normalize_percent(value: u32, minimum: i32, maximum: i32) -> Option<u8> {
    if minimum < 0 || maximum <= minimum || value < minimum as u32 || value > maximum as u32 {
        return None;
    }
    let span = (maximum - minimum) as u64;
    Some(((u64::from(value - minimum as u32) * 100 + span / 2) / span) as u8)
}

impl DeviceBackend for StandardHid {
    fn discover(&self, api: &HidApi, interfaces: &[HidInterface]) -> Result<Vec<Target>, String> {
        let mut targets = Vec::new();
        for interface in interfaces {
            if interface.vendor_id == crate::hid::VID_ASUS {
                continue;
            }
            let Ok(descriptor) = Descriptor::open(api, interface) else {
                continue;
            };
            if !descriptor.battery_caps(2).is_empty() || !descriptor.battery_caps(0).is_empty() {
                targets.push(Target::new(BackendId::StandardHid, interface, 0));
            }
        }
        Ok(targets)
    }

    fn read(&self, api: &HidApi, target: &Target, _: bool) -> Result<Snapshot, String> {
        let descriptor = Descriptor::open(api, &target.interface)?;
        let mut percent = None;
        for cap in descriptor.battery_caps(2) {
            let started = std::time::Instant::now();
            if let Some(mut report) =
                crate::hid::query_report(&target.interface, cap.ReportID, true, 160)
            {
                percent = descriptor.percent(2, &cap, &mut report);
                trace::record(
                    &target.interface,
                    BackendId::StandardHid,
                    "feature-report",
                    0x0620,
                    cap.ReportID,
                    target.interface.feature_len as usize,
                    report.len(),
                    started,
                    if percent.is_some() {
                        trace::Outcome::Ok
                    } else {
                        trace::Outcome::InvalidData
                    },
                );
                if percent.is_some() {
                    break;
                }
            } else {
                trace::record(
                    &target.interface,
                    BackendId::StandardHid,
                    "feature-report",
                    0x0620,
                    cap.ReportID,
                    target.interface.feature_len as usize,
                    0,
                    started,
                    trace::Outcome::NoReply,
                );
            }
        }
        if percent.is_none() {
            let caps = descriptor.battery_caps(0);
            for cap in &caps {
                let started = std::time::Instant::now();
                if let Some(mut report) =
                    crate::hid::query_report(&target.interface, cap.ReportID, false, 160)
                {
                    percent = descriptor.percent(0, cap, &mut report);
                    trace::record(
                        &target.interface,
                        BackendId::StandardHid,
                        "input-report",
                        0x0620,
                        cap.ReportID,
                        target.interface.input_len as usize,
                        report.len(),
                        started,
                        if percent.is_some() {
                            trace::Outcome::Ok
                        } else {
                            trace::Outcome::InvalidData
                        },
                    );
                    if percent.is_some() {
                        break;
                    }
                } else {
                    trace::record(
                        &target.interface,
                        BackendId::StandardHid,
                        "input-report",
                        0x0620,
                        cap.ReportID,
                        target.interface.input_len as usize,
                        0,
                        started,
                        trace::Outcome::NoReply,
                    );
                }
            }
            if percent.is_none() && !caps.is_empty() {
                if let Ok(mut device) = crate::hid::HidDevice::open(&target.interface) {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(80);
                    while let Some(left) =
                        deadline.checked_duration_since(std::time::Instant::now())
                    {
                        let Some(mut report) = device.read_report(left.as_millis().max(1) as u32)
                        else {
                            break;
                        };
                        percent = caps
                            .iter()
                            .find_map(|cap| descriptor.percent(0, cap, &mut report));
                        if percent.is_some() {
                            break;
                        }
                    }
                }
            }
        }
        // Input 电量报告可能只在变化时发送。接口仍在时保留最近有效读数，
        // 避免每次短读取超时都把设备误显示为断开；过期后等待新报告。
        static CACHE: std::sync::OnceLock<
            std::sync::Mutex<std::collections::BTreeMap<String, (u8, std::time::Instant)>>,
        > = std::sync::OnceLock::new();
        let mut cache = CACHE
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(percent) = percent {
            cache.insert(target.key.clone(), (percent, std::time::Instant::now()));
        } else {
            percent = cache
                .get(&target.key)
                .filter(|(_, taken)| taken.elapsed().as_secs() <= 300)
                .map(|(percent, _)| *percent);
        }
        cache.retain(|_, (_, taken)| taken.elapsed().as_secs() <= 300);
        drop(cache);
        Ok(Snapshot {
            target: target.clone(),
            connected: true,
            dpi: Dpi::default(),
            battery: Some(Battery {
                percent,
                charging: None,
                voltage_mv: None,
                approximate: false,
                level: None,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descriptor_range_and_null_values_are_respected() {
        assert_eq!(normalize_percent(127, 0, 255), Some(50));
        assert_eq!(normalize_percent(0, 0, 100), Some(0));
        assert_eq!(normalize_percent(100, 0, 100), Some(100));
        assert_eq!(normalize_percent(255, 0, 100), None);
        assert_eq!(normalize_percent(2, 5, 100), None);
        assert_eq!(normalize_percent(5, 5, 5), None);
    }
}
