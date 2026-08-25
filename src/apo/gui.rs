//! 简单 Win32 原生控制窗口（零依赖手写 FFI）
//!
//! 布局：
//!   目标设备: [下拉框                ]
//!   [ 安装全局音效 ]  [ 卸载还原     ]
//!   [   开启音效   ]  [   关闭音效   ]
//!   状态文本……
#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms, clippy::too_many_arguments)]

use super::device;
use super::install;
use super::reg;
use super::toggle;
use std::os::raw::c_void;

type HWND = *mut c_void;
type UINT = u32;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;

const CS_HREDRAW: u32 = 0x0002;
const CS_VREDRAW: u32 = 0x0001;
const COLOR_BTNFACE: i32 = 15;
const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_TABSTOP: u32 = 0x0001_0000;
const WS_VSCROLL: u32 = 0x0020_0000;
const CBS_DROPDOWNLIST: u32 = 3;
const SW_SHOW: i32 = 5;
const WM_CLOSE: UINT = 0x0010;
const WM_DESTROY: UINT = 0x0002;
const WM_COMMAND: UINT = 0x0111;
const WM_APP_REFRESH: UINT = 0x8001;
const CBN_SELCHANGE: i32 = 1;

// 控件 ID
const IDC_COMBO_DEVICE: isize = 2001;
const IDC_BTN_INSTALL: isize = 1001;
const IDC_BTN_UNINSTALL: isize = 1002;
const IDC_BTN_ON: isize = 1003;
const IDC_BTN_OFF: isize = 1004;
// 高字/低字提取
const HIWORD_SHIFT: u32 = 16;

// CB 消息
const CB_ADDSTRING: UINT = 0x0143;
const CB_SETCURSEL: UINT = 0x014E;
const CB_GETCURSEL: UINT = 0x0147;
const CB_RESETCONTENT: UINT = 0x014B;

#[link(name = "user32")]
extern "system" {
    fn RegisterClassExW(wc: *const WndClassExW) -> u16;
    fn CreateWindowExW(
        ex: u32, class: *const u16, title: *const u16, style: u32,
        x: i32, y: i32, w: i32, h: i32, parent: HWND, menu: *mut c_void,
        inst: *mut c_void, param: *mut c_void,
    ) -> HWND;
    fn DefWindowProcW(h: HWND, msg: UINT, wp: WPARAM, lp: LPARAM) -> LRESULT;
    fn ShowWindow(h: HWND, cmd: i32) -> i32;
    fn GetMessageW(msg: *mut Msg, h: HWND, min: u32, max: u32) -> i32;
    fn TranslateMessage(msg: *const Msg) -> i32;
    fn DispatchMessageW(msg: *const MSG_ALIAS) -> LRESULT;
    fn PostQuitMessage(code: i32);
    fn PostMessageW(h: HWND, msg: UINT, wp: WPARAM, lp: LPARAM) -> i32;
    fn SendMessageW(h: HWND, msg: UINT, wp: WPARAM, lp: LPARAM) -> LPARAM;
    fn SetWindowTextW(h: HWND, text: *const u16) -> i32;
    fn EnableWindow(h: HWND, enable: i32) -> i32;
    fn DestroyWindow(h: HWND) -> i32;
}

// 消息结构体在 GetMessageW / DispatchMessageW 间共用，用同一别名避免类型不匹配
#[allow(non_camel_case_types)]
type MSG_ALIAS = Msg;

#[repr(C)]
struct WndClassExW {
    cb_size: u32,
    style: u32,
    wnd_proc: unsafe extern "system" fn(HWND, UINT, WPARAM, LPARAM) -> LRESULT,
    cls_extra: i32,
    win_extra: i32,
    instance: *mut c_void,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: *mut c_void,
    menu_name: *const u16,
    icon_sm: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Point { x: i32, y: i32 }

#[repr(C)]
struct Msg {
    hwnd: HWND,
    message: UINT,
    w_param: WPARAM,
    l_param: LPARAM,
    time: u32,
    pt: Point,
}

#[link(name = "gdi32")]
extern "system" {
    fn GetStockObject(obj: i32) -> *mut c_void;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn WaitForSingleObject(h: *mut c_void, ms: u32) -> u32;
    fn CloseHandle(h: *mut c_void) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteExW(info: *mut ShellExecuteInfoW) -> i32;
}

#[repr(C)]
struct ShellExecuteInfoW {
    cb_size: u32,
    f_mask: u32,
    hwnd: HWND,
    verb: *const u16,
    file: *const u16,
    params: *const u16,
    dir: *const u16,
    show: i32,
    inst_app: *mut c_void,
    id_list: *mut c_void,
    class_: *const u16,
    hot_key: u32,
    icon_or_monitor: *mut c_void,
    process: *mut c_void,
}

const SEE_MASK_NOCLOSEPROCESS: u32 = 0x40;

/// UI 全局上下文（单窗口应用，静态存储即可）
static mut UI: Option<Ui> = None;

struct Ui {
    hwnd: HWND,
    combo: HWND,
    status: HWND,
    btn_install: HWND,
    btn_uninstall: HWND,
    btn_on: HWND,
    btn_off: HWND,
    devices: Vec<device::RenderDevice>,
    selected_guid: Option<String>,
    busy: bool,
}

unsafe fn ui() -> &'static mut Ui {
    match &mut *std::ptr::addr_of_mut!(UI) {
        Some(u) => u,
        None => std::hint::unreachable_unchecked(),
    }
}

fn w(s: &str) -> Vec<u16> { reg::wide(s) }

unsafe fn set_status(text: &str) {
    SetWindowTextW(ui().status, w(text).as_ptr());
}

/// 刷新状态区文本与按钮可用性
unsafe fn refresh_status() {
    let u = ui();
    let (installed, endpoint) = install::is_installed(None);
    let enabled = toggle::read_config() == Some(1);
    let mut s = String::new();
    s.push_str(&format!("安装状态: {}\n", if installed { "已安装" } else { "未安装" }));
    if let Some(ep) = &endpoint {
        s.push_str(&format!("挂载端点: {{{ep}}}\n"));
    }
    s.push_str(&format!(
        "开关状态: {}\n",
        if installed {
            if enabled { "开启中" } else { "关闭" }
        } else { "-" }
    ));
    if !u.busy {
        s.push_str("效果本体: 酷狗 kgapo64.dll（原版）· 安装后开机自动生效");
        set_status(&s);
        EnableWindow(u.btn_install, (!installed && !u.busy) as i32);
        EnableWindow(u.btn_uninstall, (installed && !u.busy) as i32);
        EnableWindow(u.btn_on, (installed && !enabled && !u.busy) as i32);
        EnableWindow(u.btn_off, (installed && enabled && !u.busy) as i32);
        EnableWindow(u.combo, (!u.busy) as i32);
    }
}

/// 填充设备下拉框，默认选中系统默认设备
unsafe fn populate_devices() {
    let u = ui();
    SendMessageW(u.combo, CB_RESETCONTENT, 0, 0);
    u.devices.clear();
    match device::list_render_devices() {
        Ok(list) => {
            let mut def_idx = None;
            for (i, d) in list.iter().enumerate() {
                let label = format!("{}{}", if d.is_default { "[默认] " } else { "" }, d.name);
                SendMessageW(u.combo, CB_ADDSTRING, 0, w(&label).as_ptr() as LPARAM);
                if d.is_default {
                    def_idx = Some(i);
                    u.selected_guid = Some(d.guid.clone());
                }
            }
            u.devices = list;
            SendMessageW(u.combo, CB_SETCURSEL, def_idx.unwrap_or(0), 0);
        }
        Err(e) => set_status(&format!("[枚举设备失败] {e}")),
    }
}

/// 以提权方式运行自身子命令，完成后向主窗口投递刷新消息
unsafe fn run_elevated(params: String) {
    let u = ui();
    u.busy = true;
    set_status("正在执行（请在 UAC 弹窗中确认）……");
    EnableWindow(u.btn_install, 0);
    EnableWindow(u.btn_uninstall, 0);
    EnableWindow(u.btn_on, 0);
    EnableWindow(u.btn_off, 0);
    EnableWindow(u.combo, 0);

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
    // 宽字符串缓冲区必须活到 ShellExecuteExW 调用结束
    let verb_buf = w("runas");
    let file_buf = w(&exe.display().to_string());
    let params_buf = w(&params);
    let mut info = ShellExecuteInfoW {
        cb_size: std::mem::size_of::<ShellExecuteInfoW>() as u32,
        f_mask: SEE_MASK_NOCLOSEPROCESS,
        hwnd: u.hwnd,
        verb: verb_buf.as_ptr(),
        file: file_buf.as_ptr(),
        params: params_buf.as_ptr(),
        dir: std::ptr::null(),
        show: 1, // SW_SHOWNORMAL
        inst_app: std::ptr::null_mut(),
        id_list: std::ptr::null_mut(),
        class_: std::ptr::null(),
        hot_key: 0,
        icon_or_monitor: std::ptr::null_mut(),
        process: std::ptr::null_mut(),
    };

    if ShellExecuteExW(&mut info) != 0 && !info.process.is_null() {
        // 后台线程等待提权子进程结束后刷新 UI（usize 跨线程，无借用）
        wait_then_refresh(info.process as usize, u.hwnd as usize);
    } else {
        // 用户取消 UAC 或失败
        u.busy = false;
        refresh_status();
    }
}

/// 等待句柄结束并投递刷新消息
fn wait_then_refresh(process: usize, hwnd: usize) {
    std::thread::spawn(move || unsafe {
        WaitForSingleObject(process as *mut c_void, 120_000);
        CloseHandle(process as *mut c_void);
        PostMessageW(hwnd as *mut c_void, WM_APP_REFRESH, 0, 0);
    });
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: UINT, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let id = (wp as u32 & 0xFFFF) as isize; // LOWORD = 控件 ID
            let code = ((wp as u32) >> HIWORD_SHIFT) as i32; // HIWORD = 通知码
            match (id, code) {
                (IDC_BTN_INSTALL, _) => {
                    let dev = ui().selected_guid.clone();
                    let mut p = String::from("apo-install");
                    if let Some(d) = dev {
                        p.push_str(&format!(" --device \"{d}\""));
                    }
                    p.push_str(" --elevated");
                    run_elevated(p);
                }
                (IDC_BTN_UNINSTALL, _) => {
                    run_elevated("apo-uninstall --elevated".into());
                }
                (IDC_BTN_ON, _) => { toggle::set(true); refresh_status(); }
                (IDC_BTN_OFF, _) => { toggle::set(false); refresh_status(); }
                (IDC_COMBO_DEVICE, CBN_SELCHANGE) => {
                    let sel = SendMessageW(ui().combo, CB_GETCURSEL, 0, 0);
                    if sel >= 0 {
                        ui().selected_guid =
                            ui().devices.get(sel as usize).map(|d| d.guid.clone());
                    }
                }
                _ => {}
            }
            0
        }
        WM_APP_REFRESH => {
            let u = ui();
            u.busy = false;
            refresh_status();
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn create_control(
    class: &str, text: &str, style: u32, x: i32, y: i32, wd: i32, ht: i32,
    parent: HWND, id: isize,
) -> HWND {
    let inst = GetModuleHandleW(std::ptr::null());
    CreateWindowExW(
        0, w(class).as_ptr(), w(text).as_ptr(), WS_CHILD | WS_VISIBLE | WS_TABSTOP | style,
        x, y, wd, ht, parent, id as *mut c_void, inst, std::ptr::null_mut(),
    )
}

/// gui 子命令入口：创建窗口并进入消息循环
pub fn run_gui() -> i32 {
    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let class_name = "KGDSPPaoWnd";
        let wc = WndClassExW {
            cb_size: std::mem::size_of::<WndClassExW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            wnd_proc,
            cls_extra: 0,
            win_extra: 0,
            instance: hinst,
            icon: std::ptr::null_mut(),
            cursor: std::ptr::null_mut(), // 简化：使用系统默认箭头需 LoadCursor；置空由系统兜底
            background: GetStockObject(COLOR_BTNFACE),
            menu_name: std::ptr::null(),
            icon_sm: std::ptr::null_mut(),
        };
        RegisterClassExW(&wc);

        // 客户区目标尺寸 470x230 → 用固定窗口大小近似（含边框）
        const W: i32 = 486;
        const H: i32 = 278;
        let hwnd = CreateWindowExW(
            0, w(class_name).as_ptr(), w("酷狗全局音效控制器 (kugou-dsp)").as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            100, 100, W, H,
            std::ptr::null_mut(), std::ptr::null_mut(), hinst, std::ptr::null_mut(),
        );

        // 控件布局（客户区坐标）
        create_control("STATIC", "目标设备:", 0, 12, 14, 68, 20, hwnd, 0);
        let combo = create_control(
            "COMBOBOX", "", CBS_DROPDOWNLIST | WS_VSCROLL,
            84, 10, 386, 300, hwnd, IDC_COMBO_DEVICE,
        );
        let btn_install = create_control("BUTTON", "安装全局音效", 0, 12, 50, 224, 34, hwnd, IDC_BTN_INSTALL);
        let btn_uninstall = create_control("BUTTON", "卸载还原", 0, 246, 50, 224, 34, hwnd, IDC_BTN_UNINSTALL);
        let btn_on = create_control("BUTTON", "开启音效", 0, 12, 94, 224, 40, hwnd, IDC_BTN_ON);
        let btn_off = create_control("BUTTON", "关闭音效", 0, 246, 94, 224, 40, hwnd, IDC_BTN_OFF);
        let status = create_control("STATIC", "初始化中…", 0, 12, 148, 458, 90, hwnd, 0);

        ShowWindow(hwnd, SW_SHOW);

        UI = Some(Ui {
            hwnd,
            combo,
            status,
            btn_install,
            btn_uninstall,
            btn_on,
            btn_off,
            devices: Vec::new(),
            selected_guid: None,
            busy: false,
        });
        populate_devices();
        refresh_status();

        let mut msg = Msg {
            hwnd: std::ptr::null_mut(),
            message: 0,
            w_param: 0,
            l_param: 0,
            time: 0,
            pt: Point { x: 0, y: 0 },
        };
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    0
}
