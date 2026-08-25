//! apo-on / apo-off：写单字节配置 + 触发全局事件

use super::reg;
use super::CONFIG_FILE;
use std::fs;

#[link(name = "kernel32")]
extern "system" {
    fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> *mut std::ffi::c_void;
    fn CreateEventW(sa: *const std::ffi::c_void, manual: i32, init: i32,
                    name: *const u16) -> *mut std::ffi::c_void;
    fn SetEvent(h: *mut std::ffi::c_void) -> i32;
}

/// EVENT_MODIFY_STATE
const EVENT_MODIFY_STATE: u32 = 0x0002;

/// 读当前配置字节（None=文件不存在）
pub fn read_config() -> Option<u8> {
    fs::read(CONFIG_FILE).ok().and_then(|d| d.first().copied())
}

/// 原始写入单字节（不触发事件）
pub fn write_config_raw(v: u8) -> Result<(), String> {
    if let Some(parent) = std::path::Path::new(CONFIG_FILE).parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(CONFIG_FILE, [v]).map_err(|e| format!("写入 {CONFIG_FILE} 失败: {e}"))
}

/// 触发配置变更事件；事件不存在时先创建（audiodg 未加载时无害）
unsafe fn signal_changed() -> bool {
    let name = reg::wide(super::EVENT_CONFIG_CHANGED);
    let mut h = OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr());
    if h.is_null() {
        h = CreateEventW(std::ptr::null(), 0, 0, name.as_ptr());
    }
    if h.is_null() {
        return false;
    }
    SetEvent(h) != 0
}

/// 开关入口：v=true 开启，false 关闭
pub fn set(on: bool) -> i32 {
    let v = if on { 1u8 } else { 0u8 };
    match write_config_raw(v) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("[失败] {e}\n提示: 若权限不足请以管理员运行一次 apo-install 重新部署");
            return 1;
        }
    }
    let signaled = unsafe { signal_changed() };
    println!(
        "[完成] 全局音效已{}{}",
        if on { "开启" } else { "关闭" },
        if signaled { "" } else { "（未找到变更事件——APO 可能尚未安装或音频服务未重启，音效不会即时生效）" }
    );
    0
}

/// 放宽配置文件 ACL 为 Everyone 可写（安装阶段调用，保证非管理员 UI 能开关）
///
/// 通过 GetNamedSecurityInfoW → SetEntriesInAclW → SetNamedSecurityInfoW 完成。
pub fn grant_config_write_all() -> Result<(), String> {
    #[repr(C)]
    struct TrusteeW {
        // 字段顺序必须与 Windows TRUSTEE_W 一致：
        // (pMultipleTrustee, MultipleTrusteeOperation, TrusteeType, TrusteeForm, ptstrName)
        multiple_trustee: *mut std::ffi::c_void,
        multiple_trustee_operation: u32,
        trustee_type: u32,
        trustee_form: u32,
        ptstr_name: *mut u16,
    }
    #[repr(C)]
    struct ExplicitAccessW {
        grantee: TrusteeW,
        access_permissions: u32,
        access_mode: u32,
        inheritance: u32,
    }

    const SE_FILE_OBJECT: u32 = 1;
    // MULTIPLE_TRUSTEE_OPERATION: NO_MULTIPLE_TRUSTEE=0
    // TRUSTEE_FORM: TRUSTEE_IS_NAME=1
    // TRUSTEE_TYPE: TRUSTEE_IS_WELL_KNOWN_GROUP=5
    const NO_MULTIPLE_TRUSTEE: u32 = 0;
    const TRUSTEE_IS_NAME: u32 = 1;
    const TRUSTEE_IS_WELL_KNOWN_GROUP: u32 = 5;
    // SET_ACCESS=2；标准 FILE_GENERIC_WRITE（含 SYNCHRONIZE/READ_CONTROL）
    const SET_ACCESS: u32 = 2;
    const FILE_GENERIC_WRITE: u32 = 0x0012_0116;
    const SUB_CONTAINERS_AND_OBJECTS_INHERIT: u32 = 2;

    #[link(name = "advapi32")]
    extern "system" {
        fn GetNamedSecurityInfoW(
            name: *const u16, typ: u32, info: u32, owner: *mut *mut std::ffi::c_void,
            group: *mut *mut std::ffi::c_void, dacl: *mut *mut std::ffi::c_void,
            sacl: *mut *mut std::ffi::c_void, sec_desc: *mut *mut std::ffi::c_void,
        ) -> u32;
        fn SetNamedSecurityInfoW(
            name: *const u16, typ: u32, info: u32, owner: *mut std::ffi::c_void,
            group: *mut std::ffi::c_void, dacl: *const std::ffi::c_void,
            sacl: *const std::ffi::c_void,
        ) -> u32;
        fn SetEntriesInAclW(
            count: u32, list: *const ExplicitAccessW, old: *const std::ffi::c_void,
            new: *mut *mut std::ffi::c_void,
        ) -> u32;
        fn LocalFree(h: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }

    unsafe {
        // 手工填充 EXPLICIT_ACCESS（等价于 BuildExplicitAccessWithName）
        let everyone = reg::wide("Everyone");
        let ea = ExplicitAccessW {
            grantee: TrusteeW {
                multiple_trustee: std::ptr::null_mut(),
                multiple_trustee_operation: NO_MULTIPLE_TRUSTEE,
                trustee_type: TRUSTEE_IS_WELL_KNOWN_GROUP,
                trustee_form: TRUSTEE_IS_NAME,
                ptstr_name: everyone.as_ptr() as *mut u16,
            },
            access_permissions: FILE_GENERIC_WRITE,
            access_mode: SET_ACCESS,
            inheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
        };

        let wname = reg::wide(CONFIG_FILE);
        let mut dacl: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut sd: *mut std::ffi::c_void = std::ptr::null_mut();
        let r = GetNamedSecurityInfoW(
            wname.as_ptr(), SE_FILE_OBJECT,
            4 /*DACL_SECURITY_INFORMATION*/,
            std::ptr::null_mut(), std::ptr::null_mut(), &mut dacl,
            std::ptr::null_mut(), &mut sd,
        );
        if r != 0 {
            return Err(format!("GetNamedSecurityInfo 失败: 0x{r:X}"));
        }
        let mut new_dacl: *mut std::ffi::c_void = std::ptr::null_mut();
        let r = SetEntriesInAclW(1, &ea, dacl, &mut new_dacl);
        if r != 0 {
            if !sd.is_null() { LocalFree(sd); }
            return Err(format!("SetEntriesInAcl 失败: 0x{r:X}"));
        }
        let r = SetNamedSecurityInfoW(
            wname.as_ptr(), SE_FILE_OBJECT, 4,
            std::ptr::null_mut(), std::ptr::null_mut(), new_dacl, std::ptr::null(),
        );
        if !new_dacl.is_null() { LocalFree(new_dacl); }
        if !sd.is_null() { LocalFree(sd); }
        if r != 0 {
            return Err(format!("SetNamedSecurityInfo 失败: 0x{r:X}"));
        }
    }
    Ok(())
}
