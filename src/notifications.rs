use core::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

const IID_XML_DOCUMENT_IO: Guid = Guid {
    data1: 0x6CD0_E74E,
    data2: 0xEE65,
    data3: 0x4489,
    data4: [0x9E, 0xBF, 0xCA, 0x43, 0xE8, 0x7B, 0xA6, 0x37],
};
const IID_TOAST_FACTORY: Guid = Guid {
    data1: 0x0412_4B20,
    data2: 0x82C6,
    data3: 0x4229,
    data4: [0xB1, 0x09, 0xFD, 0x9E, 0xD4, 0x66, 0x2B, 0x53],
};
const IID_TOAST_MANAGER_STATICS: Guid = Guid {
    data1: 0x50AC_103F,
    data2: 0xD235,
    data3: 0x4598,
    data4: [0xBB, 0xEF, 0x98, 0xFE, 0x4D, 0x1A, 0x3A, 0xD4],
};
const IID_TOAST_NOTIFICATION2: Guid = Guid {
    data1: 0x9DFB_9FD1,
    data2: 0x143A,
    data3: 0x490E,
    data4: [0x90, 0xBF, 0xB9, 0xFB, 0xA7, 0x13, 0x2D, 0xE7],
};

const CO_E_NOTINITIALIZED: u32 = 0x8004_01F0;
const RPC_E_CHANGED_MODE: u32 = 0x8001_0106;
const RO_INIT_MULTITHREADED: i32 = 1;

type HResult = i32;
type FnRoInitialize = unsafe extern "system" fn(i32) -> HResult;
type FnRoUninitialize = unsafe extern "system" fn();
type FnWindowsCreateString =
    unsafe extern "system" fn(*const u16, u32, *mut *mut c_void) -> HResult;
type FnWindowsDeleteString = unsafe extern "system" fn(*mut c_void) -> HResult;
type FnRoGetActivationFactory =
    unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> HResult;
type FnRoActivateInstance = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HResult;
type FnQueryInterface =
    unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> HResult;
type FnRelease = unsafe extern "system" fn(*mut c_void) -> u32;

struct RoApi {
    _module: crate::win32::Handle,
    initialize: FnRoInitialize,
    uninitialize: FnRoUninitialize,
    create_string: FnWindowsCreateString,
    delete_string: FnWindowsDeleteString,
    get_factory: FnRoGetActivationFactory,
    activate_instance: FnRoActivateInstance,
}

impl RoApi {
    fn load() -> Result<Self, String> {
        let name = crate::win32::to_wide("combase.dll");
        let module = unsafe { crate::win32::LoadLibraryW(name.as_ptr()) };
        if module.is_null() || module == crate::win32::INVALID_HANDLE_VALUE {
            return Err(format!(
                "无法加载 combase.dll（GetLastError={}）",
                crate::win32::last_error()
            ));
        }

        fn symbol(module: crate::win32::Handle, name: &str) -> *mut c_void {
            let wide = crate::win32::to_wide(name);
            let ansi: Vec<u8> = wide
                .iter()
                .map(|&unit| if unit < 128 { unit as u8 } else { b'?' })
                .collect();
            unsafe { crate::win32::GetProcAddress(module, ansi.as_ptr()) }
        }

        macro_rules! load {
            ($name:literal, $ty:ty) => {{
                let pointer = symbol(module, $name);
                if pointer.is_null() {
                    unsafe { crate::win32::FreeLibrary(module) };
                    return Err(format!("combase.dll 缺少导出函数 {}", $name));
                }
                unsafe { core::mem::transmute::<*mut c_void, $ty>(pointer) }
            }};
        }

        Ok(Self {
            _module: module,
            initialize: load!("RoInitialize", FnRoInitialize),
            uninitialize: load!("RoUninitialize", FnRoUninitialize),
            create_string: load!("WindowsCreateString", FnWindowsCreateString),
            delete_string: load!("WindowsDeleteString", FnWindowsDeleteString),
            get_factory: load!("RoGetActivationFactory", FnRoGetActivationFactory),
            activate_instance: load!("RoActivateInstance", FnRoActivateInstance),
        })
    }
}

struct RoSession {
    api: RoApi,
    needs_uninitialize: bool,
}

impl RoSession {
    fn open() -> Result<Self, String> {
        let api = RoApi::load()?;
        let result = unsafe { (api.initialize)(RO_INIT_MULTITHREADED) } as u32;
        if result == 0 {
            return Ok(Self {
                api,
                needs_uninitialize: true,
            });
        }
        if result == RPC_E_CHANGED_MODE {
            return Ok(Self {
                api,
                needs_uninitialize: false,
            });
        }
        if result == CO_E_NOTINITIALIZED {
            return Err("RoInitialize 返回 CO_E_NOTINITIALIZED".to_string());
        }
        Err(format!("RoInitialize 失败：0x{result:08X}"))
    }

    fn hstring(&self, text: &str) -> Result<*mut c_void, String> {
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut value: *mut c_void = core::ptr::null_mut();
        let result = unsafe {
            (self.api.create_string)(units.as_ptr(), units.len() as u32, &mut value)
        };
        if result < 0 {
            return Err(format!(
                "WindowsCreateString 失败：0x{:08X}",
                result as u32
            ));
        }
        Ok(value)
    }

    fn free_hstring(&self, value: *mut c_void) {
        if !value.is_null() {
            unsafe {
                (self.api.delete_string)(value);
            }
        }
    }

    fn activate(&self, class_name: &str) -> Result<*mut c_void, String> {
        let class = self.hstring(class_name)?;
        let mut instance: *mut c_void = core::ptr::null_mut();
        let result = unsafe { (self.api.activate_instance)(class, &mut instance) };
        self.free_hstring(class);
        if result < 0 || instance.is_null() {
            return Err(format!(
                "RoActivateInstance({class_name}) 失败：0x{:08X}",
                result as u32
            ));
        }
        Ok(instance)
    }

    fn factory(&self, class_name: &str, iid: &Guid) -> Result<*mut c_void, String> {
        let class = self.hstring(class_name)?;
        let mut factory: *mut c_void = core::ptr::null_mut();
        let result = unsafe { (self.api.get_factory)(class, iid, &mut factory) };
        self.free_hstring(class);
        if result < 0 || factory.is_null() {
            return Err(format!(
                "RoGetActivationFactory({class_name}) 失败：0x{:08X}",
                result as u32
            ));
        }
        Ok(factory)
    }
}

impl Drop for RoSession {
    fn drop(&mut self) {
        if self.needs_uninitialize {
            unsafe { (self.api.uninitialize)() };
        }
    }
}

unsafe fn vtable_call<T>(object: *mut c_void, index: usize) -> T {
    let table = *(object as *mut *const usize);
    core::mem::transmute_copy::<usize, T>(&*table.add(index))
}

unsafe fn release(object: *mut c_void) {
    if !object.is_null() {
        let call: FnRelease = vtable_call(object, 2);
        call(object);
    }
}

unsafe fn query_interface(object: *mut c_void, iid: &Guid) -> Result<*mut c_void, String> {
    let call: FnQueryInterface = vtable_call(object, 0);
    let mut output: *mut c_void = core::ptr::null_mut();
    let result = call(object, iid, &mut output);
    if result < 0 || output.is_null() {
        return Err(format!(
            "WinRT QueryInterface 失败：0x{:08X}",
            result as u32
        ));
    }
    Ok(output)
}

pub fn supported() -> bool {
    crate::autostart::is_packaged()
}

fn escape_xml(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            _ => output.push(character),
        }
    }
    output
}

fn toast_xml(device_name: &str, level: u8, threshold: u8) -> String {
    let name = escape_xml(device_name);
    if crate::i18n::english() {
        format!(
            "<toast><visual><binding template=\"ToastGeneric\"><text>Low battery</text><text>{name} is at {level}%, below the {threshold}% threshold. Please charge it soon.</text></binding></visual><audio silent=\"false\"/></toast>"
        )
    } else {
        format!(
            "<toast><visual><binding template=\"ToastGeneric\"><text>低电量提醒</text><text>{name} 电量仅剩 {level}%，已低于 {threshold}%，请及时充电。</text></binding></visual><audio silent=\"false\"/></toast>"
        )
    }
}

pub fn show_low_battery(
    device_key: &str,
    device_name: &str,
    level: u8,
    threshold: u8,
) -> Result<(), String> {
    if !supported() {
        return Err("仅 Microsoft Store 版本支持系统通知".to_string());
    }

    let session = RoSession::open()?;
    let document = session.activate("Windows.Data.Xml.Dom.XmlDocument")?;
    let result = (|| {
        let xml_io = unsafe { query_interface(document, &IID_XML_DOCUMENT_IO) }?;
        let xml = session.hstring(&toast_xml(device_name, level, threshold))?;
        type FnLoadXml =
            unsafe extern "system" fn(*mut c_void, *mut c_void) -> HResult;
        let load: FnLoadXml = unsafe { vtable_call(xml_io, 6) };
        let load_result = unsafe { load(xml_io, xml) };
        session.free_hstring(xml);
        unsafe { release(xml_io) };
        if load_result < 0 {
            return Err(format!("Toast XML 加载失败：0x{:08X}", load_result as u32));
        }

        let toast_factory =
            session.factory("Windows.UI.Notifications.ToastNotification", &IID_TOAST_FACTORY)?;
        type FnCreateToast =
            unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HResult;
        let create: FnCreateToast = unsafe { vtable_call(toast_factory, 6) };
        let mut toast: *mut c_void = core::ptr::null_mut();
        let create_result = unsafe { create(toast_factory, document, &mut toast) };
        unsafe { release(toast_factory) };
        if create_result < 0 || toast.is_null() {
            return Err(format!(
                "ToastNotification 创建失败：0x{:08X}",
                create_result as u32
            ));
        }

        if let Ok(tag_object) =
            unsafe { query_interface(toast, &IID_TOAST_NOTIFICATION2) }
        {
            let tag: String = device_key.chars().take(16).collect();
            let tag_value = session.hstring(&tag)?;
            type FnSetString = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HResult;
            let set_tag: FnSetString = unsafe { vtable_call(tag_object, 6) };
            let _ = unsafe { set_tag(tag_object, tag_value) };
            session.free_hstring(tag_value);
            let group = session.hstring("battery-low")?;
            let set_group: FnSetString = unsafe { vtable_call(tag_object, 8) };
            let _ = unsafe { set_group(tag_object, group) };
            session.free_hstring(group);
            unsafe { release(tag_object) };
        }

        let manager = session.factory(
            "Windows.UI.Notifications.ToastNotificationManager",
            &IID_TOAST_MANAGER_STATICS,
        )?;
        type FnCreateNotifier =
            unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HResult;
        let create_notifier: FnCreateNotifier = unsafe { vtable_call(manager, 6) };
        let mut notifier: *mut c_void = core::ptr::null_mut();
        let notifier_result = unsafe { create_notifier(manager, &mut notifier) };
        unsafe { release(manager) };
        if notifier_result < 0 || notifier.is_null() {
            unsafe { release(toast) };
            return Err(format!(
                "ToastNotifier 创建失败：0x{:08X}",
                notifier_result as u32
            ));
        }

        type FnShow = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HResult;
        let show: FnShow = unsafe { vtable_call(notifier, 6) };
        let show_result = unsafe { show(notifier, toast) };
        unsafe {
            release(notifier);
            release(toast);
        }
        if show_result < 0 {
            return Err(format!("显示系统通知失败：0x{:08X}", show_result as u32));
        }
        Ok(())
    })();
    unsafe { release(document) };
    result
}
