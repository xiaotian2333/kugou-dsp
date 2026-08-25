//! Win32 注册表 FFI（手写绑定，零依赖）
//!
//! 关键点：本工具是 32 位进程，而 audiodg 是 64 位，
//! 写 Classes\CLSID 与 MMDevices 必须显式 KEY_WOW64_64KEY。
#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::os::raw::{c_int, c_long, c_ulong, c_void};

pub type HKEY = *mut c_void;
pub type DWORD = c_ulong;
pub type LONG = c_long;

pub const HKEY_LOCAL_MACHINE: HKEY = 0x8000_0002usize as HKEY;

pub const KEY_READ: DWORD = 0x0002_0019;
pub const KEY_WRITE: DWORD = 0x0002_0006;
pub const KEY_ALL_ACCESS: DWORD = 0x000F_003F;
/// 64 位视图访问标志（32 位进程访问 x64 注册表必带）
pub const KEY_WOW64_64KEY: DWORD = 0x0100;
pub const REG_OPTION_NON_VOLATILE: DWORD = 0;
pub const REG_SZ: DWORD = 1;
#[allow(dead_code)]
pub const REG_DWORD: DWORD = 4;

#[link(name = "advapi32")]
extern "system" {
    pub fn RegOpenKeyExW(hKey: HKEY, sub: *const u16, opt: DWORD, sam: DWORD, res: *mut HKEY) -> LONG;
    pub fn RegCreateKeyExW(
        hKey: HKEY, sub: *const u16, reserved: DWORD, class: *const u16, opts: DWORD,
        sam: DWORD, sec: *const c_void, res: *mut HKEY, disp: *mut DWORD,
    ) -> LONG;
    pub fn RegCloseKey(hKey: HKEY) -> LONG;
    pub fn RegQueryValueExW(
        hKey: HKEY, name: *const u16, reserved: *const DWORD, typ: *mut DWORD,
        data: *mut u8, cb: *mut DWORD,
    ) -> LONG;
    pub fn RegSetValueExW(
        hKey: HKEY, name: *const u16, reserved: DWORD, typ: DWORD,
        data: *const u8, cb: DWORD,
    ) -> LONG;
    pub fn RegDeleteValueW(hKey: HKEY, name: *const u16) -> LONG;
    pub fn RegDeleteTreeW(hKey: HKEY, sub: *const u16) -> LONG;
    pub fn RegEnumKeyExW(
        hKey: HKEY, idx: DWORD, name: *mut u16, nlen: *mut DWORD, res: *const DWORD,
        class: *mut u16, clen: *mut DWORD, time: *mut c_void,
    ) -> LONG;
}

/// UTF-8 → 以 NUL 结尾的 UTF-16
pub fn wide(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// UTF-16 → Rust String（到首个 NUL）
pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// 打开 64 位视图下的注册表键
pub unsafe fn open_key64(sub: &str, sam: DWORD) -> Result<HKEY, LONG> {
    let mut hk: HKEY = std::ptr::null_mut();
    let r = RegOpenKeyExW(HKEY_LOCAL_MACHINE, wide(sub).as_ptr(), 0, sam | KEY_WOW64_64KEY, &mut hk);
    if r == 0 { Ok(hk) } else { Err(r) }
}

/// 创建（或打开）64 位视图下的注册表键
pub unsafe fn create_key64(sub: &str) -> Result<HKEY, LONG> {
    let mut hk: HKEY = std::ptr::null_mut();
    let mut disp: DWORD = 0;
    let r = RegCreateKeyExW(
        HKEY_LOCAL_MACHINE, wide(sub).as_ptr(), 0, std::ptr::null(),
        REG_OPTION_NON_VOLATILE, KEY_ALL_ACCESS | KEY_WOW64_64KEY,
        std::ptr::null(), &mut hk, &mut disp,
    );
    if r == 0 { Ok(hk) } else { Err(r) }
}

/// 读字符串值（REG_SZ）
pub unsafe fn get_string(hk: HKEY, name: &str) -> Option<String> {
    let mut ty: DWORD = 0;
    let mut cb: DWORD = 0;
    if RegQueryValueExW(hk, wide(name).as_ptr(), std::ptr::null(), &mut ty,
                        std::ptr::null_mut(), &mut cb) != 0 || cb == 0 {
        return None;
    }
    let mut buf = vec![0u8; cb as usize];
    if RegQueryValueExW(hk, wide(name).as_ptr(), std::ptr::null(), &mut ty,
                        buf.as_mut_ptr(), &mut cb) != 0 {
        return None;
    }
    let w: Vec<u16> = buf.chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some(from_wide(&w))
}

/// 写字符串值（REG_SZ）
pub unsafe fn set_string(hk: HKEY, name: &str, val: &str) -> bool {
    let v = wide(val);
    RegSetValueExW(hk, wide(name).as_ptr(), 0, REG_SZ,
                   v.as_ptr() as *const u8, (v.len() * 2) as DWORD) == 0
}

/// 删除值
pub unsafe fn delete_value(hk: HKEY, name: &str) -> bool {
    RegDeleteValueW(hk, wide(name).as_ptr()) == 0
}

/// 枚举子键名
pub unsafe fn enum_subkeys(hk: HKEY) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0..4096u32 {
        let mut buf = [0u16; 256];
        let mut len: DWORD = 256;
        let r = RegEnumKeyExW(hk, i, buf.as_mut_ptr(), &mut len, std::ptr::null(),
                              std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
        if r != 0 { break; }
        out.push(from_wide(&buf[..len as usize]));
    }
    out
}

/// IsUserAnAdmin（shell32），判断当前是否已提权
pub unsafe fn is_user_admin() -> bool {
    #[link(name = "shell32")]
    extern "system" { fn IsUserAnAdmin() -> c_int; }
    IsUserAnAdmin() != 0
}
