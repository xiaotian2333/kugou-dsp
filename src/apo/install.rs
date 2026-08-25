//! apo-install / apo-uninstall：文件部署、COM 注册、端点挂载与备份还原

use super::reg;
use super::{APO_DIR, APO_FILES, BACKUP_ROOT, CLSID_KUGOU_APO, FX_PROP_KEY, FX_PROP_VALUE, KUGOU_APO_CLASS};
use std::path::Path;

#[link(name = "kernel32")]
extern "system" {
    fn CopyFileW(src: *const u16, dst: *const u16, fail_if_exists: i32) -> i32;
    fn CreateDirectoryW(path: *const u16, sa: *const std::ffi::c_void) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(hwnd: *const std::ffi::c_void, op: *const u16, file: *const u16,
                     params: *const u16, dir: *const u16, show: i32) -> isize;
}

fn wide(s: &str) -> Vec<u16> { reg::wide(s) }

/// 是否已安装：CLSID 存在 且 至少一个端点 ,2 指向 KuGouAPO
pub fn is_installed(_args: Option<&str>) -> (bool, Option<String>) {
    unsafe {
        let clsid_key = format!(r"SOFTWARE\Classes\CLSID\{CLSID_KUGOU_APO}");
        let clsid_ok = reg::open_key64(&clsid_key, reg::KEY_READ).is_ok();
        let mut mounted = None;
        if let Ok(hk) = reg::open_key64(FX_PROP_KEY, reg::KEY_READ) {
            for dev in reg::enum_subkeys(hk) {
                let fx = format!("{FX_PROP_KEY}\\{dev}\\FxProperties");
                if let Ok(fxhk) = reg::open_key64(&fx, reg::KEY_READ) {
                    if reg::get_string(fxhk, FX_PROP_VALUE).as_deref() == Some(CLSID_KUGOU_APO) {
                        mounted = Some(dev.trim_matches(|c| c == '{' || c == '}').to_string());
                        reg::RegCloseKey(fxhk);
                        break;
                    }
                    reg::RegCloseKey(fxhk);
                }
            }
            reg::RegCloseKey(hk);
        }
        (clsid_ok && mounted.is_some(), mounted)
    }
}

/// 解析 --from / --device 参数；返回 (from, device, 警告列表)
fn parse_args(args: &[String]) -> (Option<String>, Option<String>, Vec<String>) {
    let mut from = None;
    let mut device = None;
    let mut warnings = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--from" if i + 1 < args.len() => { from = Some(args[i + 1].clone()); i += 2; }
            "--device" if i + 1 < args.len() => { device = Some(args[i + 1].clone()); i += 2; }
            other => {
                warnings.push(format!("忽略无法识别的参数: {other}（--device 的值应为设备 ID 本身，如 {{b5287108-...}}）"));
                i += 1;
            }
        }
    }
    (from, device, warnings)
}

/// 定位 KGApo 源目录：优先 --from，其次 exe 同目录 KGApo\
fn locate_source(from: Option<&str>) -> Result<std::path::PathBuf, String> {
    if let Some(d) = from {
        let p = Path::new(d);
        if p.exists() { return Ok(p.to_path_buf()); }
        return Err(format!("--from 目录不存在: {d}"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let p = parent.join("KGApo");
            if p.exists() { return Ok(p); }
        }
    }
    Err("未找到 KGApo 源目录（可传 --from <目录>，或把三件套放到 exe 同目录 KGApo\\ 下）".into())
}

/// UAC 提权自重启；返回 true 表示已成功拉起提权进程（当前进程应退出）
unsafe fn relaunch_elevated(extra: &str) -> bool {
    if reg::is_user_admin() { return false; }
    let exe = std::env::current_exe().map_err(|_| ()).unwrap_or_default();
    let params = format!("\"{}\" {}", exe.display(), extra);
    // SW_SHOWNORMAL=1
    let h = ShellExecuteW(std::ptr::null(), wide("runas").as_ptr(),
                          wide(&exe.display().to_string()).as_ptr(),
                          wide(&params).as_ptr(), std::ptr::null(), 1);
    // >32 表示成功
    h > 32
}

/// apo-install 入口
pub fn install(args: &[String]) -> i32 {
    let (from, device, warnings) = parse_args(args);
    for w in &warnings {
        eprintln!("[警告] {w}");
    }
    if let Some(d) = &device {
        // 粗校验：设备 ID 应为 GUID 形态
        let g = d.trim().trim_start_matches('{').trim_end_matches('}');
        let looks_like_guid = g.len() == 36 && g.matches('-').count() == 4;
        if !looks_like_guid {
            eprintln!(
                "[失败] --device 的值不是合法设备 ID: {d}\n\
                 正确格式示例（注意花括号是 ID 的一部分）:\n  \
                 apo-install --device \"{{b5287108-53c1-41d2-aa81-4955c333e08b}}\"\n\
                 先运行 `kugou-dsp devices` 查看设备清单"
            );
            return 2;
        }
    }
    // 未提权 → 拉起提权副本后退出
    unsafe {
        if !reg::is_user_admin() {
            let mut extra = String::from("apo-install");
            if let Some(f) = &from { extra.push_str(&format!(" --from \"{f}\"")); }
            if let Some(d) = &device { extra.push_str(&format!(" --device {d}")); }
            extra.push_str(" --elevated");
            return if relaunch_elevated(&extra) { 0 } else {
                eprintln!("[失败] 需要管理员权限（UAC 取消或不可用）");
                1
            };
        }
    }
    match install_impl(from.as_deref(), device.as_deref()) {
        Ok(msg) => { println!("[完成] {msg}"); 0 }
        Err(e) => { eprintln!("[失败] {e}"); 1 }
    }
}

fn install_impl(from: Option<&str>, device: Option<&str>) -> Result<String, String> {
    // 1. 部署文件
    let src = locate_source(from)?;
    deploy_files(&src)?;

    // 2. 注册 CLSID（64 位视图）
    register_clsid()?;

    // 3. 确定目标端点
    let endpoint = match device {
        Some(g) => g.trim().trim_start_matches('{').trim_end_matches('}').to_string(),
        None => {
            unsafe { super::device::default_render_device()? }
                .trim_start_matches('{').trim_end_matches('}').to_string()
        }
    };

    // 4. 备份并挂载 FxProperties
    mount_endpoint(&endpoint)?;

    // 5. 初始化配置文件为关闭态（含 Everyone 写权限，便于非管理员 UI 开关）
    super::toggle::write_config_raw(0)?;
    super::toggle::grant_config_write_all()?;

    Ok(format!(
        "安装完成。端点 {{{}}} 已挂载 KuGouAPO。\n用 `kugou-dsp apo-on` 或 UI 开启音效。",
        endpoint
    ))
}

fn deploy_files(src: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    // 创建目标目录链 C:\ProgramData\Kugou\KGApo\v1.0
    let mut cur = std::path::PathBuf::from("C:\\ProgramData");
    for part in ["Kugou", "KGApo", "v1.0"] {
        cur.push(part);
        let w: Vec<u16> = cur.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        unsafe { CreateDirectoryW(w.as_ptr(), std::ptr::null()); }
    }
    for f in APO_FILES {
        let s = src.join(f);
        if !s.exists() {
            return Err(format!("源缺失: {}", s.display()));
        }
        let d = Path::new(APO_DIR).join(f);
        let ok = unsafe {
            CopyFileW(wide(&s.display().to_string()).as_ptr(),
                      wide(&d.display().to_string()).as_ptr(), 0)
        };
        if ok == 0 {
            return Err(format!("复制失败: {} → {}（可能被 audiodg 占用，先关闭音效再试）",
                               s.display(), d.display()));
        }
    }
    Ok(())
}

fn register_clsid() -> Result<(), String> {
    unsafe {
        let key = format!(r"SOFTWARE\Classes\CLSID\{CLSID_KUGOU_APO}");
        let hk = reg::create_key64(&key).map_err(|e| format!("创建 CLSID 键失败: 0x{e:X}"))?;
        reg::set_string(hk, "", KUGOU_APO_CLASS);
        let inproc = format!(r"{key}\InprocServer32");
        let hk2 = reg::create_key64(&inproc).map_err(|e| format!("创建 InprocServer32 失败: 0x{e:X}"))?;
        let dll = format!("{APO_DIR}\\kgapo64.dll");
        reg::set_string(hk2, "", &dll);
        reg::set_string(hk2, "ThreadingModel", "Both");
        reg::RegCloseKey(hk2);
        reg::RegCloseKey(hk);
    }
    Ok(())
}

/// 备份端点当前 ,2 值到 BACKUP_ROOT\<guid>，然后写入 KuGouAPO
fn mount_endpoint(guid: &str) -> Result<(), String> {
    unsafe {
        let fx = format!("{FX_PROP_KEY}\\{{{}}}\\FxProperties", guid);
        // 确保可写（必要时接管属主 + 放权），失败会带手动修复指引
        super::sec::ensure_key_writable(&fx)?;
        let hk_fx = reg::open_key64(&fx, reg::KEY_READ)
            .map_err(|e| format!("端点 FxProperties 打开失败({fx}): 0x{e:X}\n该设备可能是虚拟设备，换 --device 试试"))?;
        let original = reg::get_string(hk_fx, FX_PROP_VALUE);
        reg::RegCloseKey(hk_fx);

        // 写备份键（镜像原值；不存在则记 !VALUE，与原版语义一致）
        let bk = format!("{BACKUP_ROOT}\\{{{}}}", guid);
        let hk_bk = reg::create_key64(&bk).map_err(|e| format!("创建备份键失败: 0x{e:X}"))?;
        match &original {
            Some(v) => { reg::set_string(hk_bk, FX_PROP_VALUE, v); }
            None => { reg::set_string(hk_bk, FX_PROP_VALUE, "!VALUE"); }
        }
        reg::set_string(hk_bk, "Version", "1");
        reg::RegCloseKey(hk_bk);

        // 挂载
        let hk_fx_w = reg::open_key64(&fx, reg::KEY_WRITE)
            .map_err(|e| format!("端点 FxProperties 打开写失败: 0x{e:X}"))?;
        let ok = reg::set_string(hk_fx_w, FX_PROP_VALUE, CLSID_KUGOU_APO);
        reg::RegCloseKey(hk_fx_w);
        if !ok {
            return Err("写入 FxProperties 失败".into());
        }
    }
    Ok(())
}

/// apo-uninstall 入口
pub fn uninstall(args: &[String]) -> i32 {
    let (_, device, warnings) = parse_args(args);
    for w in &warnings {
        eprintln!("[警告] {w}");
    }
    unsafe {
        if !reg::is_user_admin() {
            let mut extra = String::from("apo-uninstall");
            if let Some(d) = &device { extra.push_str(&format!(" --device {d}")); }
            extra.push_str(" --elevated");
            return if relaunch_elevated(&extra) { 0 } else {
                eprintln!("[失败] 需要管理员权限");
                1
            };
        }
    }
    match uninstall_impl(device.as_deref()) {
        Ok(msg) => { println!("[完成] {msg}"); 0 }
        Err(e) => { eprintln!("[失败] {e}"); 1 }
    }
}

fn uninstall_impl(device: Option<&str>) -> Result<String, String> {
    unsafe {
        // 先关音效（若配置文件可写）
        let _ = super::toggle::write_config_raw(0);

        // 找出所有挂载了 KuGouAPO 的端点（限 --device 时只处理它）
        let mut targets: Vec<String> = Vec::new();
        if let Some(g) = device {
            targets.push(g.trim().trim_start_matches('{').trim_end_matches('}').to_string());
        } else if let Ok(hk) = reg::open_key64(FX_PROP_KEY, reg::KEY_READ) {
            for dev in reg::enum_subkeys(hk) {
                let fx = format!("{FX_PROP_KEY}\\{dev}\\FxProperties");
                // 只读检测即可——写权限可能在还原阶段再放权
                if let Ok(fxhk) = reg::open_key64(&fx, reg::KEY_READ) {
                    if reg::get_string(fxhk, FX_PROP_VALUE).as_deref() == Some(CLSID_KUGOU_APO) {
                        targets.push(dev.trim_matches(|c| c == '{' || c == '}').to_string());
                    }
                    reg::RegCloseKey(fxhk);
                }
            }
            reg::RegCloseKey(hk);
        }

        for guid in &targets {
            // 从备份还原 ,2 原值
            let bk = format!("{BACKUP_ROOT}\\{{{}}}", guid);
            if let Ok(hk_bk) = reg::open_key64(&bk, reg::KEY_READ) {
                let orig = reg::get_string(hk_bk, FX_PROP_VALUE);
                reg::RegCloseKey(hk_bk);
                let fx = format!("{FX_PROP_KEY}\\{{{}}}\\FxProperties", guid);
                // 还原路径同样可能被 ACL 拒写，先确保可写
                super::sec::ensure_key_writable(&fx)?;
                if let Ok(hk_fx) = reg::open_key64(&fx, reg::KEY_WRITE) {
                    match orig.as_deref() {
                        Some("!VALUE") | None => { reg::delete_value(hk_fx, FX_PROP_VALUE); }
                        Some(v) => { reg::set_string(hk_fx, FX_PROP_VALUE, v); }
                    }
                    reg::RegCloseKey(hk_fx);
                }
                // 清掉我们自己的备份键树
                let _ = reg::RegDeleteTreeW(
                    reg::HKEY_LOCAL_MACHINE, wide(&bk).as_ptr());
            }
        }

        // 删 CLSID 树
        let clsid_key = format!(r"SOFTWARE\Classes\CLSID\{CLSID_KUGOU_APO}");
        let _ = reg::RegDeleteTreeW(reg::HKEY_LOCAL_MACHINE, wide(&clsid_key).as_ptr());

        if targets.is_empty() {
            return Ok("未发现挂载点（可能已卸载）".into());
        }
        let list = targets.iter()
            .map(|g| format!("{{{g}}}")).collect::<Vec<_>>().join(", ");
        Ok(format!("卸载完成，已还原端点: {list}\n如声音异常请重启 Windows Audio 服务或重启电脑"))
    }
}
