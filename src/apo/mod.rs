//! 全局音效（APO）模块
//!
//! 复用酷狗原版 kgapo64.dll（EqualizerAPO 改版 + 内嵌 ViPER 引擎）实现
//! 电脑全局音效，开机自启由 APO 注册天然保证。
//!
//! 子命令:
//!   apo-install   [--device <GUID>] [--from <dir>]  安装（UAC 提权）
//!   apo-uninstall [--device <GUID>]                 卸载还原
//!   apo-on / apo-off / apo-status                   开关与状态
//!   gui                                             简单控制窗口

pub mod device;
pub mod gui;
pub mod install;
pub mod reg;
pub mod sec;
pub mod toggle;

/// KuGouAPO 的 COM CLSID（kgapo64.dll）
pub const CLSID_KUGOU_APO: &str = "{B19D744D-4269-41A5-8361-314020726F10}";
/// COM 类友好名（与原版一致）
pub const KUGOU_APO_CLASS: &str = "KuGouAPO Class";
/// 配置文件：固定路径（kgapo64 直接拼接，不走注册表）
pub const CONFIG_FILE: &str = "C:\\ProgramData\\Kugou\\kugou_apo_config_file";
/// 全局事件：配置变更通知
pub const EVENT_CONFIG_CHANGED: &str = "Global\\kugou_apo_config_changed_event";
/// APO 部署目录
pub const APO_DIR: &str = "C:\\ProgramData\\Kugou\\KGApo\\v1.0";
/// 需部署的文件清单（来自原版安装包）
pub const APO_FILES: &[&str] = &["ApoConfigurator.exe", "kgapo.dll", "kgapo64.dll"];
/// 端点 FxProperties 中挂载 KuGouAPO 的属性槽位：
/// PKEY_FX_EndpointEffectClsid {d04e05a6-594b-4fb6-a80d-01af5eed7d1d},2
pub const FX_PROP_KEY: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Render";
pub const FX_PROP_VALUE: &str = "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},2";
/// 原值备份根键（卸载时按此还原）
pub const BACKUP_ROOT: &str = r"SOFTWARE\Kugou\BackupAPOs_kugoudsp";

/// apo 子命令入口。`args` 为子命令后的参数序列。
pub fn run(cmd: &str, args: &[String]) -> i32 {
    match cmd {
        "apo-install" => install::install(args),
        "apo-uninstall" => install::uninstall(args),
        "apo-on" => toggle::set(true),
        "apo-off" => toggle::set(false),
        "apo-status" => status(),
        _ => {
            eprintln!("未知 apo 子命令: {cmd}");
            eprintln!("可用: apo-install / apo-uninstall / apo-on / apo-off / apo-status");
            2
        }
    }
}

fn status() -> i32 {
    let installed = install::is_installed(None);
    let enabled = toggle::read_config() == Some(1);
    println!("KuGouAPO 安装状态: {}", if installed.0 { "已安装" } else { "未安装" });
    if let Some(dev) = installed.1 {
        println!("挂载端点: {{{}}}", dev);
    }
    println!("开关状态: {}", if enabled { "开启" } else { "关闭" });
    0
}
