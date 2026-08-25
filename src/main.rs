//! kugou-dsp：酷狗 dsp.dll 音效引擎调用器（一键预设版）
//!
//! 子命令:
//!   info                 加载自检
//!   wav IN OUT           离线处理 WAV 文件，施加指定音效预设
//!
//! 预设经 dsp.dll `AudioPostProcessWrapper::LoadPreset(id)` 加载，
//! CLI 白名单: 1=3D丽音  2=超重低音  3=纯净人声  4=HIFI现场
//! 全部已知预设见 README「全部音效预设总表」。
//!
//! 通用选项: --dll-dir PATH (默认仅搜索 exe 同目录的 dll\ 子目录)

mod engine;
mod ffi;
mod offline;
mod presets;

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        print_usage();
        std::process::exit(2);
    }

    // 解析 --dll-dir
    let mut dll_dir: Option<PathBuf> = None;
    let mut pos: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--dll-dir" && i + 1 < args.len() {
            dll_dir = Some(PathBuf::from(&args[i + 1]));
            i += 2;
        } else {
            pos.push(args[i].clone());
            i += 1;
        }
    }
    let dll_dir = dll_dir.unwrap_or_else(default_dll_dir);

    let code = run(&pos, &dll_dir);
    std::process::exit(code);
}

fn default_dll_dir() -> PathBuf {
    // 仅搜索 exe 同目录的 dll\ 子目录；找不到也原样返回，
    // 由后续加载阶段报错提示（不再回退 exe 根目录或 A:\KuGou）
    if let Ok(exe) = std::env::current_exe() {
        if let Some(p) = exe.parent() {
            return p.join("dll");
        }
    }
    PathBuf::from("dll")
}

fn print_usage() {
    println!(
        "用法:\n\
         kugou-dsp info [--dll-dir DIR]\n\
         kugou-dsp wav <in.wav> <out.wav> --preset <ID> [--keep-tail] [--dll-dir DIR]\n\
         \n\
         --keep-tail  保留结尾混响拖尾（输出比输入略长；默认与输入等长）\n\
         可用预设 ID: {}",
        presets::allowed_list_text()
    );
}

fn run(pos: &[String], dll_dir: &std::path::Path) -> i32 {
    match pos[0].as_str() {
        "info" => cmd_info(dll_dir),
        "wav" => {
            if pos.len() < 3 {
                eprintln!("wav 需要 <in.wav> <out.wav>");
                return 2;
            }
            cmd_wav(pos, dll_dir)
        }
        other => {
            eprintln!("未知子命令: {other}");
            print_usage();
            2
        }
    }
}

fn cmd_info(dll_dir: &std::path::Path) -> i32 {
    println!("DLL 目录: {}", dll_dir.display());
    unsafe {
        match engine::DspEngine::new(dll_dir) {
            Ok(e) => {
                println!("[OK] dsp.dll 加载成功，符号解析完成");
                // 冒烟测试：用白名单首个预设建链
                let id = presets::CLI_ALLOWED[0];
                match e.apply_preset(id) {
                    Ok((_, es)) => {
                        let name = presets::name_of(id).unwrap_or("?");
                        println!("[OK] LoadPreset({id}) [{name}] 建链成功, IExternalDSPSet = {:p}", es);
                        0
                    }
                    Err(m) => {
                        eprintln!("[失败] 预设建链自检失败: {m}");
                        1
                    }
                }
            }
            Err(msg) => {
                eprintln!("[失败] {msg}");
                1
            }
        }
    }
}

fn cmd_wav(pos: &[String], dll_dir: &std::path::Path) -> i32 {
    // wav IN OUT --preset ID [--keep-tail]
    let mut preset: Option<i32> = None;
    let mut keep_tail = false;
    let mut i = 3;
    while i < pos.len() {
        match pos[i].as_str() {
            "--preset" if i + 1 < pos.len() => {
                match pos[i + 1].parse::<i32>() {
                    Ok(v) => preset = Some(v),
                    Err(_) => {
                        eprintln!("--preset 需要整数 ID，收到 {:?}", pos[i + 1]);
                        return 2;
                    }
                }
                i += 2;
            }
            "--keep-tail" => {
                keep_tail = true;
                i += 1;
            }
            other => {
                eprintln!("未知参数 {other}");
                return 2;
            }
        }
    }

    let preset = match preset {
        Some(p) => p,
        None => {
            eprintln!("缺少 --preset 参数。可用预设: {}", presets::allowed_list_text());
            return 2;
        }
    };
    if !presets::is_cli_allowed(preset) {
        eprintln!(
            "预设 {preset} 不在 CLI 白名单内。可用预设: {}",
            presets::allowed_list_text()
        );
        return 2;
    }
    let pname = presets::name_of(preset).unwrap_or("?");

    println!("读取 {}...", pos[1]);
    let input = match offline::read_wav(&pos[1]) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[失败] {e}");
            return 1;
        }
    };
    println!(
        "  {} Hz x {} 声道 x {} 帧",
        input.sample_rate,
        input.channels,
        input.samples.len() / input.channels.max(1) as usize
    );

    unsafe {
        let engine = match engine::DspEngine::new(dll_dir) {
            Ok(e) => e,
            Err(m) => {
                eprintln!("[失败] {m}");
                return 1;
            }
        };
        println!("音效预设 [{pname}] (id={preset}) 处理中...");
        match offline::process_wav(&engine, &input, preset, keep_tail) {
            Ok(outdata) => match offline::write_wav(&pos[2], &outdata) {
                Ok(_) => {
                    println!("[完成] {}", pos[2]);
                    0
                }
                Err(e) => {
                    eprintln!("[失败] {e}");
                    1
                }
            },
            Err(e) => {
                eprintln!("[失败] {e}");
                1
            }
        }
    }
}
