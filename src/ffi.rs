//! dsp.dll 运行时 FFI 绑定
//!
//! dsp.dll 无导入库(.lib)，所有符号经 LoadLibraryExW + GetProcAddress 动态解析。
//! 符号名为 MSVC 32 位 name-mangled 形式，与导出表一一对应。
//! 仅可在 i686 (32位) 进程中加载。
//!
//! 本项目仅使用「一键预设」路线（AudioPostProcessWrapper）：
//!   ctor -> LoadPreset(id) -> GetExternalDSPSet -> vtable flush/process
#![allow(non_snake_case)]

use std::ffi::CString;
use std::os::raw::{c_int, c_void};

/// 不透明 C++ 对象
#[repr(C)]
pub struct CxxObj {
    _private: [u8; 0],
}

pub const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x8;

// ---------------------------------------------------------------------------
// Win32
// ---------------------------------------------------------------------------

extern "system" {
    fn LoadLibraryExW(name: *const u16, file: usize, flags: u32) -> isize;
    fn FreeLibrary(module: isize) -> i32;
    fn GetProcAddress(module: isize, name: *const u8) -> *const c_void;
    fn GetLastError() -> u32;
}

/// 格式化 Windows 错误码为可读文本
pub fn last_error_msg(code: u32) -> String {
    format!("Win32 错误码 {:#x}", code)
}

// ---------------------------------------------------------------------------
// 模块句柄与符号解析
// ---------------------------------------------------------------------------

pub struct DspLib {
    pub module: isize,
}

impl DspLib {
    /// 以 LOAD_WITH_ALTERED_SEARCH_PATH 加载 dsp.dll，
    /// 其静态依赖 (infra.dll / logging.dll / CRT) 会自动从同目录解析
    pub unsafe fn load(path: &std::path::Path) -> Result<DspLib, String> {
        let canon = path
            .canonicalize()
            .map_err(|e| format!("路径无效 {}: {}", path.display(), e))?;
        use std::os::windows::ffi::OsStrExt;
        let mut w: Vec<u16> = canon.as_os_str().encode_wide().collect();
        w.push(0);
        let module = LoadLibraryExW(w.as_ptr(), 0, LOAD_WITH_ALTERED_SEARCH_PATH);
        if module == 0 {
            return Err(format!(
                "LoadLibraryExW 失败: {} ({})",
                last_error_msg(GetLastError()),
                path.display()
            ));
        }
        Ok(DspLib { module })
    }

    /// 按 mangled 名解析符号并转成给定类型的函数指针
    pub unsafe fn sym<T>(&self, mangled: &str) -> Result<T, String> {
        // Mangled 名是纯 ASCII，直接按字节传给 GetProcAddress
        let c = CString::new(mangled).map_err(|_| "符号名含 \\0")?;
        let p = GetProcAddress(self.module, c.as_ptr() as *const u8);
        if p.is_null() {
            return Err(format!("符号未找到: {}", mangled));
        }
        Ok(std::mem::transmute_copy::<*const c_void, T>(&p))
    }
}

impl Drop for DspLib {
    fn drop(&mut self) {
        if self.module != 0 {
            unsafe {
                FreeLibrary(self.module);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// AudioPostProcessWrapper 一键预设接口（thiscall）
// ---------------------------------------------------------------------------

/// AudioPostProcessWrapper::AudioPostProcessWrapper(void)
pub type FnApoWrapperCtor = unsafe extern "thiscall" fn(this_: *mut CxxObj);
/// void AudioPostProcessWrapper::LoadPreset(int preset)
///
/// 反汇编依据 dsp.dll VA 0x10005D4E(导出) -> 0x100058E0(核心):
///   preset==-1 关闭声场组件；>=0 启用并对内部两个效果器 SetProgram(preset)
pub type FnLoadPreset = unsafe extern "thiscall" fn(this_: *mut CxxObj, preset: c_int);
/// bool AudioPostProcessWrapper::GetExternalDSPSet(IExternalDSPSet** out)
pub type FnGetExternalSet =
    unsafe extern "thiscall" fn(this_: *mut CxxObj, out: *mut *mut CxxObj) -> bool;

// ---------------------------------------------------------------------------
// 导出符号名常量（MSVC mangled，与 dsp.dll 导出表一一对应）
// ---------------------------------------------------------------------------

pub mod names {
    pub const APO_WRAPPER_CTOR: &str = "??0AudioPostProcessWrapper@@QAE@XZ";
    pub const APO_LOAD_PRESET: &str = "?LoadPreset@AudioPostProcessWrapper@@QAEXH@Z";
    pub const APO_GET_EXTERNAL_SET: &str =
        "?GetExternalDSPSet@AudioPostProcessWrapper@@QAE_NPAPAUIExternalDSPSet@@@Z";
}
