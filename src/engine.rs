//! 高层封装：dsp.dll 符号解析与音效引擎会话（一键预设路线）
//!
//! 会话流程:
//!   DspEngine::new(dir)            加载 dsp.dll 并解析符号
//!   apply_preset(id)               创建 AudioPostProcessWrapper 并加载预设
//!                                  返回 (wrapper, IExternalDSPSet*)
//!   vtable_slot(es, off)           取处理接口虚槽位 (0x10 flush / 0x14 process)

use crate::ffi::{self, names, CxxObj, DspLib};
use std::os::raw::c_void;

/// 全部导出函数的解析结果
pub struct Symbols {
    pub apo_ctor: ffi::FnApoWrapperCtor,
    pub apo_load_preset: ffi::FnLoadPreset,
    pub apo_get_external_set: ffi::FnGetExternalSet,
}

impl Symbols {
    pub unsafe fn resolve(lib: &DspLib) -> Result<Symbols, String> {
        Ok(Symbols {
            apo_ctor: lib.sym(names::APO_WRAPPER_CTOR)?,
            apo_load_preset: lib.sym(names::APO_LOAD_PRESET)?,
            apo_get_external_set: lib.sym(names::APO_GET_EXTERNAL_SET)?,
        })
    }
}

/// 音效引擎会话
///
/// 内存策略：所有 C++ 对象一次性分配、随进程退出回收，
/// 规避跨 CRT 释放与 DLL 卸载顺序问题（本工具为单次运行型）。
pub struct DspEngine {
    _lib: DspLib,
    pub syms: Symbols,
}

unsafe impl Send for DspEngine {}

/// AudioPostProcessWrapper 仅含一个指针成员，取富余
const WRAPPER_STORAGE: usize = 0x40;

impl DspEngine {
    /// 加载 dsp.dll 并解析符号
    ///
    /// `dll_dir`: dsp.dll 所在目录；静态依赖 (infra.dll/logging.dll/CRT)
    /// 经 LOAD_WITH_ALTERED_SEARCH_PATH 自动从同目录解析
    pub unsafe fn new(dll_dir: &std::path::Path) -> Result<DspEngine, String> {
        let dll_path = dll_dir.join("dsp.dll");
        if !dll_path.exists() {
            return Err(format!("找不到 {}", dll_path.display()));
        }
        let lib = DspLib::load(&dll_path)?;
        let syms = Symbols::resolve(&lib)?;
        Ok(DspEngine { _lib: lib, syms })
    }

    /// 创建 AudioPostProcessWrapper 并应用一键预设
    ///
    /// 返回 (wrapper, IExternalDSPSet*)；后者即 COM 风格处理接口，
    /// 虚槽位 +0x10 为 flush、+0x14 为 process（见 offline.rs）。
    pub unsafe fn apply_preset(&self, preset: i32) -> Result<(*mut CxxObj, *mut CxxObj), String> {
        let storage = vec![0u8; WRAPPER_STORAGE].into_boxed_slice();
        let wrapper = Box::into_raw(storage) as *mut CxxObj;
        (self.syms.apo_ctor)(wrapper);
        (self.syms.apo_load_preset)(wrapper, preset);

        // 验证内部状态：取出 IExternalDSPSet 确认预设确实生成了处理链
        let mut es: *mut CxxObj = std::ptr::null_mut();
        let ok = (self.syms.apo_get_external_set)(wrapper, &mut es);
        if !ok || es.is_null() {
            return Err(format!("LoadPreset({preset}) 后 GetExternalDSPSet 无效"));
        }
        Ok((wrapper, es))
    }
}

/// 取对象 vtable 中第 byte_offset/4 个虚函数地址
pub unsafe fn vtable_slot(obj: *mut CxxObj, byte_offset: usize) -> *const c_void {
    let vt = *(obj as *mut *const c_void);
    let entry = (vt as *const *const c_void).add(byte_offset / std::mem::size_of::<*const c_void>());
    core::ptr::read_unaligned(entry)
}
