//! 音频端点设备枚举（COM/IMMDeviceEnumerator 手写绑定）
//!
//! 用于定位默认渲染设备对应的 MMDevices 注册表键，以及列出全部活动设备。
#![allow(non_camel_case_types, clippy::upper_case_acronyms, clippy::too_many_arguments)]

use super::reg::from_wide;

use std::os::raw::c_void;

// CLSID_MMDeviceEnumerator / IID_IMMDeviceEnumerator
const CLSID_MMDEVICE_ENUMERATOR_BYTES: [u8; 16] = [
    0x95, 0x03, 0xDE, 0xBC, 0x2F, 0xE5, 0x7C, 0x46, 0x8E, 0x3D, 0xC4, 0x57, 0x92, 0x91, 0x69, 0x2E,
];
const IID_IMMDEVICE_ENUMERATOR_BYTES: [u8; 16] = [
    0xD2, 0x64, 0x56, 0xA9, 0x14, 0x96, 0x35, 0x4F, 0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6,
];

type HRESULT = i32;
#[repr(C)]
struct Guid([u8; 16]);

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, model: u32) -> HRESULT;
    fn CoCreateInstance(
        clsid: *const Guid, outer: *mut c_void, ctx: u32, iid: *const Guid, out: *mut *mut c_void,
    ) -> HRESULT;
}

const CLSCTX_ALL: u32 = 23;
/// COINIT_APARTMENTTHREADED
const COINIT_APARTMENTTHREADED: u32 = 2;

/// 渲染设备信息
pub struct RenderDevice {
    /// MMDevices 注册表键名（纯 GUID 大括号形式）
    pub guid: String,
    /// 设备友好名
    pub name: String,
    /// 是否系统默认渲染设备
    pub is_default: bool,
}

unsafe fn com_device_id(dev: *mut c_void) -> Option<String> {
    // IMMDevice vtable: +0x14 GetId(this, LPWSTR*)
    let vt = *(dev as *mut *const c_void);
    let get_id: extern "system" fn(*mut c_void, *mut *const u16) -> HRESULT =
        std::mem::transmute(*((vt as *const usize).add(5)));
    let mut pw: *const u16 = std::ptr::null();
    if get_id(dev, &mut pw) != 0 || pw.is_null() {
        return None;
    }
    let mut len = 0usize;
    while *pw.add(len) != 0 { len += 1; }
    let id = from_wide(std::slice::from_raw_parts(pw, len));
    // 释放任务交给进程退出（单次运行型策略），避免跨堆释放问题
    Some(id)
}

/// 从 IMMDevice ID "{0.0.0.00000000}.{GUID}" 提取注册表键用的 GUID 部分
fn endpoint_guid(id: &str) -> Option<String> {
    id.rsplit('.').next().map(|s| s.to_string())
}

/// 枚举活动渲染设备；`with_default` 时同时标注默认设备。
pub unsafe fn list_render_devices() -> Result<Vec<RenderDevice>, String> {
    CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED);
    let mut enumr: *mut c_void = std::ptr::null_mut();
    let hr = CoCreateInstance(
        &Guid(CLSID_MMDEVICE_ENUMERATOR_BYTES), std::ptr::null_mut(), CLSCTX_ALL,
        &Guid(IID_IMMDEVICE_ENUMERATOR_BYTES), &mut enumr,
    );
    if hr != 0 {
        return Err(format!("CoCreateInstance(MMDeviceEnumerator) 失败: 0x{hr:08X}"));
    }

    let vt = *(enumr as *mut *const c_void);
    type FnEnum = extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void) -> HRESULT;
    type FnDefault = extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void) -> HRESULT;
    let enum_ep: FnEnum = std::mem::transmute(*((vt as *const usize).add(3)));
    let get_def: FnDefault = std::mem::transmute(*((vt as *const usize).add(4)));

    // eRender=0, DEVICE_STATE_ACTIVE=1
    let mut coll: *mut c_void = std::ptr::null_mut();
    if enum_ep(enumr, 0, 1, &mut coll) != 0 || coll.is_null() {
        return Err("EnumAudioEndpoints 失败".into());
    }
    let cvt = *(coll as *mut *const c_void);
    type FnCount = extern "system" fn(*mut c_void, *mut u32) -> HRESULT;
    type FnItem = extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT;
    let get_count: FnCount = std::mem::transmute(*((cvt as *const usize).add(3)));
    let get_item: FnItem = std::mem::transmute(*((cvt as *const usize).add(4)));
    let mut n: u32 = 0;
    get_count(coll, &mut n);

    let mut default_guid = String::new();
    let mut def_dev: *mut c_void = std::ptr::null_mut();
    // eRender=0, eConsole=0
    if get_def(enumr, 0, 0, &mut def_dev) == 0 && !def_dev.is_null() {
        if let Some(id) = com_device_id(def_dev) {
            if let Some(g) = endpoint_guid(&id) {
                default_guid = g.to_lowercase();
            }
        }
    }

    let mut out = Vec::new();
    for i in 0..n {
        let mut dev: *mut c_void = std::ptr::null_mut();
        if get_item(coll, i, &mut dev) != 0 || dev.is_null() {
            continue;
        }
        if let Some(id) = com_device_id(dev) {
            if let Some(g) = endpoint_guid(&id) {
                let name = read_friendly_name(&g);
                out.push(RenderDevice {
                    is_default: g.to_lowercase() == default_guid,
                    name,
                    guid: format!("{{{g}}}"),
                });
            }
        }
    }
    Ok(out)
}

/// 从注册表读设备名：优先 FriendlyName(,14)，回退 DeviceDesc(,2)
fn read_friendly_name(guid: &str) -> String {
    let sub = format!(
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Render\{guid}\Properties"
    );
    unsafe {
        match super::reg::open_key64(&sub, super::reg::KEY_READ) {
            Ok(hk) => {
                let name = super::reg::get_string(
                    hk,
                    "{a45c254e-df1c-4efd-8020-67d146a850e0},14",
                )
                .or_else(|| {
                    super::reg::get_string(hk, "{a45c254e-df1c-4efd-8020-67d146a850e0},2")
                })
                .unwrap_or_else(|| "(未知设备)".into());
                super::reg::RegCloseKey(hk);
                name
            }
            Err(_) => "(未知设备)".into(),
        }
    }
}

/// 返回默认渲染设备的 GUID（大括号形式）
pub unsafe fn default_render_device() -> Result<String, String> {
    let list = list_render_devices()?;
    list.into_iter()
        .find(|d| d.is_default)
        .map(|d| d.guid)
        .ok_or_else(|| "未找到默认渲染设备".into())
}

/// 供 CLI 调用：打印设备清单
pub fn cmd_list_devices() -> i32 {
    unsafe {
        match list_render_devices() {
            Ok(list) => {
                println!("活动渲染设备:");
                for d in list {
                    println!(
                        "  {} {}  {}",
                        if d.is_default { "*" } else { " " },
                        d.name,
                        d.guid
                    );
                }
                println!("(* 为默认设备)")
            }
            Err(e) => {
                eprintln!("[失败] {e}");
                return 1;
            }
        }
    }
    0
}
