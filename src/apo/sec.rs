//! 注册表键「确保可写」：复刻 EqualizerAPO 的 ensuring-writability 流程
//!
//! 某些端点的 FxProperties 键属主为 SYSTEM/TrustedInstaller 且不给
//! Administrators 写权限，直接写会 ACCESS_DENIED(0x5)。处理方式：
//!   启用 SeTakeOwnershipPrivilege → 接管属主为 Administrators
//!   → 在原 DACL 基础上追加 Administrators FullControl ACE → 重试写入。
#![allow(non_snake_case, clippy::upper_case_acronyms)]

use super::reg;
use std::os::raw::c_void;

type HANDLE = *mut c_void;

const TOKEN_ADJUST_PRIVILEGES: u32 = 0x0020;
const TOKEN_QUERY: u32 = 0x0008;
const SE_PRIVILEGE_ENABLED: u32 = 2;
// ERROR_NOT_ALL_ASSIGNED
const ERROR_NOT_ALL_ASSIGNED: i32 = 1300;

#[repr(C)]
#[derive(Clone, Copy)]
struct Luid { low_part: u32, high_part: i32 }

#[repr(C)]
struct LuidAndAttributes { luid: Luid, attributes: u32 }

#[repr(C)]
struct TokenPrivileges {
    privilege_count: u32,
    privileges: [LuidAndAttributes; 1],
}

/// 启用当前进程令牌上的指定特权（如 SeTakeOwnershipPrivilege）
pub unsafe fn enable_privilege(name: &str) -> Result<(), String> {
    #[link(name = "kernel32")]
    extern "system" { fn GetCurrentProcess() -> HANDLE; }
    #[link(name = "advapi32")]
    extern "system" {
        fn OpenProcessToken(proc_handle: HANDLE, access: u32, token: *mut HANDLE) -> i32;
        fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut Luid) -> i32;
        fn AdjustTokenPrivileges(
            token: HANDLE, disable_all: i32, new_state: *const TokenPrivileges,
            buf_len: u32, prev: *mut c_void, ret_len: *mut u32,
        ) -> i32;
        fn GetLastError() -> u32;
        fn CloseHandle(h: HANDLE) -> i32;
    }

    let mut token: HANDLE = std::ptr::null_mut();
    if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token) == 0 {
        return Err("OpenProcessToken 失败".into());
    }
    let mut luid = Luid { low_part: 0, high_part: 0 };
    let ok = LookupPrivilegeValueW(
        std::ptr::null(), reg::wide(name).as_ptr(), &mut luid,
    );
    CloseHandle(token);
    if ok == 0 {
        return Err(format!("LookupPrivilegeValue({name}) 失败"));
    }

    // 重新打开令牌供调整使用
    if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token) == 0 {
        return Err("OpenProcessToken 失败".into());
    }
    let tp = TokenPrivileges {
        privilege_count: 1,
        privileges: [LuidAndAttributes { luid, attributes: SE_PRIVILEGE_ENABLED }],
    };
    let ok = AdjustTokenPrivileges(
        token, 0, &tp, 0, std::ptr::null_mut(), std::ptr::null_mut(),
    );
    let err = GetLastError();
    CloseHandle(token);
    if ok == 0 {
        return Err("AdjustTokenPrivileges 调用失败".into());
    }
    if err == ERROR_NOT_ALL_ASSIGNED as u32 {
        return Err(format!("特权 {name} 未被授予（需要管理员身份运行）"));
    }
    Ok(())
}

/// 确保注册表键可写；必要时接管属主并追加 Administrators 完全控制 ACE。
///
/// `subpath`: 不含根的子路径，如 `SOFTWARE\...\FxProperties`
pub unsafe fn ensure_key_writable(subpath: &str) -> Result<(), String> {
    // ① 直接尝试写权限打开
    if reg::open_key64(subpath, reg::KEY_WRITE).is_ok() {
        return Ok(());
    }

    // ② 启用接管特权
    enable_privilege("SeTakeOwnershipPrivilege")
        .map_err(|e| format!("启用特权失败: {e}"))?;

    // ③ Administrators 组 SID (S-1-5-32-544)
    #[repr(C)]
    struct SidIdentifierAuthority { value: [u8; 6] }
    const SECURITY_NT_AUTHORITY: [u8; 6] = [0, 0, 0, 0, 0, 5];
    const SECURITY_BUILTIN_DOMAIN_RID: u32 = 32;
    const DOMAIN_ALIAS_RID_ADMINS: u32 = 544;

    #[link(name = "advapi32")]
    extern "system" {
        fn AllocateAndInitializeSid(
            authority: *const SidIdentifierAuthority, sub_count: u8,
            sa0: u32, sa1: u32, sa2: u32, sa3: u32, sa4: u32, sa5: u32,
            sa6: u32, sa7: u32, sid: *mut *mut c_void,
        ) -> i32;
        fn FreeSid(sid: *mut c_void) -> *mut c_void;
        fn GetNamedSecurityInfoW(
            name: *const u16, typ: u32, info: u32, owner: *mut *mut c_void,
            group: *mut *mut c_void, dacl: *mut *mut c_void,
            sacl: *mut *mut c_void, sec_desc: *mut *mut c_void,
        ) -> u32;
        fn SetNamedSecurityInfoW(
            name: *const u16, typ: u32, info: u32, owner: *mut c_void,
            group: *mut c_void, dacl: *const c_void, sacl: *const c_void,
        ) -> u32;
        fn SetEntriesInAclW(
            count: u32, list: *const ExplicitAccessW, old: *const c_void,
            new: *mut *mut c_void,
        ) -> u32;
        fn LocalFree(h: *mut c_void) -> *mut c_void;
    }

    #[repr(C)]
    struct TrusteeW {
        multiple_trustee: *mut c_void,
        multiple_trustee_operation: u32,
        trustee_type: u32,
        trustee_form: u32,
        ptstr_name: *mut c_void,
    }
    #[repr(C)]
    struct ExplicitAccessW {
        grantee: TrusteeW,
        access_permissions: u32,
        access_mode: u32,
        inheritance: u32,
    }

    const SE_REGISTRY_KEY: u32 = 2;
    const OWNER_SECURITY_INFORMATION: u32 = 1;
    const DACL_SECURITY_INFORMATION: u32 = 4;
    // TRUSTEE_FORM: TRUSTEE_IS_SID=0；TRUSTEE_TYPE: TRUSTEE_IS_GROUP=2
    const TRUSTEE_IS_SID: u32 = 0;
    const TRUSTEE_IS_GROUP: u32 = 2;
    const NO_MULTIPLE_TRUSTEE: u32 = 0;
    const SET_ACCESS: u32 = 2;
    // 注册表 KEY_ALL_ACCESS
    const KEY_ALL_ACCESS_MASK: u32 = 0x000F_003F;

    let mut admins: *mut c_void = std::ptr::null_mut();
    let authority = SidIdentifierAuthority { value: SECURITY_NT_AUTHORITY };
    if AllocateAndInitializeSid(
        &authority, 2,
        SECURITY_BUILTIN_DOMAIN_RID, DOMAIN_ALIAS_RID_ADMINS,
        0, 0, 0, 0, 0, 0, &mut admins,
    ) == 0 {
        return Err("AllocateAndInitializeSid 失败".into());
    }

    let full_path = format!("MACHINE\\{subpath}");
    let wpath = reg::wide(&full_path);

    // ④ 取当前安全描述符
    let mut old_owner: *mut c_void = std::ptr::null_mut();
    let mut old_dacl: *mut c_void = std::ptr::null_mut();
    let mut sd: *mut c_void = std::ptr::null_mut();
    let r = GetNamedSecurityInfoW(
        wpath.as_ptr(), SE_REGISTRY_KEY,
        OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
        &mut old_owner, std::ptr::null_mut(), &mut old_dacl,
        std::ptr::null_mut(), &mut sd,
    );
    if r != 0 {
        FreeSid(admins);
        return Err(format!("GetNamedSecurityInfo({full_path}) 失败: 0x{r:X}"));
    }

    // ⑤ 属主改为 Administrators
    let mut result = Ok(());
    if !old_owner.is_null() && old_owner != admins {
        let r = SetNamedSecurityInfoW(
            wpath.as_ptr(), SE_REGISTRY_KEY, OWNER_SECURITY_INFORMATION,
            admins, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(),
        );
        if r != 0 {
            result = Err(format!("接管属主失败({full_path}): 0x{r:X}"));
        }
    }

    // ⑥ 追加 Administrators FullControl ACE
    if result.is_ok() {
        let ea = ExplicitAccessW {
            grantee: TrusteeW {
                multiple_trustee: std::ptr::null_mut(),
                multiple_trustee_operation: NO_MULTIPLE_TRUSTEE,
                trustee_type: TRUSTEE_IS_GROUP,
                trustee_form: TRUSTEE_IS_SID,
                ptstr_name: admins,
            },
            access_permissions: KEY_ALL_ACCESS_MASK,
            access_mode: SET_ACCESS,
            inheritance: 0,
        };
        let mut new_dacl: *mut c_void = std::ptr::null_mut();
        let r = SetEntriesInAclW(1, &ea, old_dacl, &mut new_dacl);
        if r != 0 {
            result = Err(format!("SetEntriesInAcl 失败: 0x{r:X}"));
        } else {
            let r = SetNamedSecurityInfoW(
                wpath.as_ptr(), SE_REGISTRY_KEY, DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(), std::ptr::null_mut(), new_dacl, std::ptr::null(),
            );
            if r != 0 {
                result = Err(format!("写入新 DACL 失败({full_path}): 0x{r:X}"));
            }
            if !new_dacl.is_null() { LocalFree(new_dacl); }
        }
    }

    if !sd.is_null() { LocalFree(sd); }
    FreeSid(admins);
    result?;

    // ⑦ 重试写权限打开做最终确认
    if reg::open_key64(subpath, reg::KEY_WRITE).is_err() {
        return Err(format!(
            "放权后仍无法写入 {subpath}\n\
             手动修复: regedit 定位 HKLM\\{subpath} → 权限 → 高级 → \
             所有者改为 Administrators 并勾选完全控制"
        ));
    }
    Ok(())
}
