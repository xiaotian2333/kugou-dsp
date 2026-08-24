//! 离线管线：WAV -> dsp.dll 音效预设 -> WAV
//!
//! 数据通路（逆向确认，对应 AudioPostProcessWrapper::GetExternalDSPSet 返回对象）:
//!   LoadPreset(id) -> GetExternalDSPSet -> vtable+0x10 flush -> vtable+0x14 Process
//!
//! 关键结论（dsp.dll RVA 反汇编）:
//! - flush  (vt+0x10, ret 0x14): 仅加锁复位流状态，3 个 32 位栈参
//! - Process(vt+0x14, ret 0x30): 16 位整型交错 PCM，签名如下
//! ```c
//! bool Process(this, int16_t* buf,   // 原位处理
//!              int   byteLen,        // 总字节数; frames = byteLen/(channels*2)
//!              int   unused3,        // 本路径未用
//!              uint* written,        // 回写输出字节 = frames*channels*2
//!              int   unused5,        // 本路径未用
//!              uint  sampleRate,     // >=22050 才会被引擎采纳
//!              uint  channels,       // 引擎仅接受 2 (立体声)
//!              uint  bitsMust16,     // 必须 0x10
//!              uint  flagMust0,      // 必须 0
//!              uint p10, uint p11)   // 补齐 ret 0x30
//! ```

use crate::engine::{vtable_slot, DspEngine};
use std::os::raw::c_int;

pub struct WavData {
    pub sample_rate: u32,
    pub channels: u16,
    /// 交错 f32 样本
    pub samples: Vec<f32>,
}

pub fn read_wav(path: &str) -> Result<WavData, String> {
    let mut reader = hound::WavReader::open(path).map_err(|e| format!("打开 {path}: {e}"))?;
    let spec = reader.spec();
    let channels = spec.channels;
    let sample_rate = spec.sample_rate;

    let mut samples = Vec::new();
    for s in reader.samples::<i32>() {
        let v = s.map_err(|e| format!("读取样本: {e}"))?;
        let f = match spec.bits_per_sample {
            8 => ((v & 0xFF) as u8 as i16 - 128) as f32 / 128.0,
            16 => (v as i16) as f32 / 32768.0,
            24 => {
                let x = v & 0xFFFFFF;
                let signed = if x & 0x800000 != 0 { x | !0xFFFFFF } else { x };
                signed as f32 / 8388608.0
            }
            32 => f32::from_bits(v as u32),
            b => return Err(format!("不支持的位深 {b}")),
        };
        samples.push(f);
    }
    Ok(WavData {
        sample_rate,
        channels,
        samples,
    })
}

pub fn write_wav(path: &str, data: &WavData) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: data.channels,
        sample_rate: data.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer =
        hound::WavWriter::create(path, spec).map_err(|e| format!("创建 {path}: {e}"))?;
    for s in &data.samples {
        writer
            .write_sample(*s)
            .map_err(|e| format!("写入样本: {e}"))?;
    }
    Ok(())
}

/// IExternalDSPSet(wrapper 版)::Process —— COM stdcall, this + 11 参 (ret 0x30)
type FnProcess = unsafe extern "system" fn(
    this_: *mut std::ffi::c_void,
    buf: usize,        // int16 交错缓冲 (原位)
    byte_len: usize,   // 总字节数
    unused3: usize,
    written: usize,
    unused5: usize,
    sample_rate: usize,
    channels: usize,
    bits_must_0x10: usize,
    flag_must_0: usize,
    p10: usize,
    p11: usize,
) -> c_int;

/// f32 [-1,1] -> int16（软限幅后量化）
fn f32_to_i16(x: f32) -> i16 {
    let c = x.tanh().clamp(-1.0, 1.0);
    (c * 32767.0) as i16
}

/// 执行离线处理：对输入施加 `preset_id` 对应的酷狗音效预设
pub unsafe fn process_wav(
    engine: &DspEngine,
    input: &WavData,
    preset_id: i32,
) -> Result<WavData, String> {
    let chans = input.channels.max(1) as usize;
    if chans != 2 {
        return Err(format!(
            "预设引擎仅支持立体声输入，当前 {chans} 声道（可先将文件转为立体声）"
        ));
    }

    // 1. 一键建链：LoadPreset + 取处理接口
    let (_wrapper, es) = engine.apply_preset(preset_id)?;

    // 2. flush 槽位 [0x10]: ret 0x14 => this + 4 个 32 位栈参 (pts:i64 + rate:f64)
    type FnFlushM = unsafe extern "system" fn(*mut std::ffi::c_void, i64, f64);
    let flush_m: FnFlushM = std::mem::transmute(vtable_slot(es, 0x10));
    flush_m(es as *mut std::ffi::c_void, 0i64, input.sample_rate as f64);

    // 3. process 槽位 [0x14]: 16 位整型原位处理
    let process: FnProcess = std::mem::transmute(vtable_slot(es, 0x14));

    let chunk_frames = 512usize; // 引擎内部缓冲按小帧设计
    let total_frames = input.samples.len() / chans;
    let mut out = Vec::with_capacity(input.samples.len());
    let mut offset_frame = 0usize;

    while offset_frame < total_frames {
        let frames = chunk_frames.min(total_frames - offset_frame);
        let n_samples = frames * chans;

        // f32 -> i16 交错
        let mut work_i16: Vec<i16> = Vec::with_capacity(n_samples);
        for x in &input.samples[offset_frame * chans..offset_frame * chans + n_samples] {
            work_i16.push(f32_to_i16(*x));
        }

        let mut written: u32 = 0;
        let r = process(
            es as *mut std::ffi::c_void,
            work_i16.as_mut_ptr() as usize,
            work_i16.len() * 2, // 字节数
            0,
            &mut written as *mut u32 as usize,
            0,
            input.sample_rate as usize,
            chans,
            0x10, // 位深标志必须 16
            0,
            0,
            0,
        );
        if r == 0 {
            return Err(format!("Process 在帧 {offset_frame} 处失败"));
        }

        // written 口径与输入一致: 字节数(i16)
        let mut usable_samples = (written as usize / 2).min(n_samples);
        // 对齐到整帧
        usable_samples -= usable_samples % chans;
        for &v in &work_i16[..usable_samples] {
            out.push(v as f32 / 32768.0);
        }
        // 引擎 FIFO 存在约 3 块预热延迟，期间 written=0；
        // 以静音补齐保持时间轴连续
        while out.len() < (offset_frame + frames) * chans {
            out.push(0.0);
        }

        offset_frame += frames;
    }

    // 尾部排空: 继续喂静音块，挤出 FIFO 内残余数据（线性淡出防爆音）
    let silence_i16 = vec![0i16; chunk_frames * chans];
    let tail_blocks = 16usize;
    for b in 0..tail_blocks {
        let mut written: u32 = 0;
        let r = process(
            es as *mut std::ffi::c_void,
            silence_i16.as_ptr() as usize,
            silence_i16.len() * 2,
            0,
            &mut written as *mut u32 as usize,
            0,
            input.sample_rate as usize,
            chans,
            0x10,
            0,
            0,
            0,
        );
        if r == 0 {
            break;
        }
        let usable = ((written as usize / 2).min(silence_i16.len())) / chans * chans;
        for (i, &v) in silence_i16[..usable].iter().enumerate() {
            let pos = b * chunk_frames + i / chans;
            let fade = 1.0 - pos as f32 / (tail_blocks * chunk_frames) as f32;
            out.push((v as f32 / 32768.0) * fade.clamp(0.0, 1.0));
        }
    }

    Ok(WavData {
        sample_rate: input.sample_rate,
        channels: input.channels,
        samples: out,
    })
}
