# kugou-dsp — 酷狗 dsp.dll 音效引擎 Rust 调用器

通过 FFI 调用酷狗音乐 9.2.71 的 `dsp.dll`（ViPER 音效引擎），对任意 PCM/WAV
数据施加酷狗官方音效预设。**完全离线运行，不依赖酷狗主程序，不需要联网。**

预设加载走 dsp.dll 原生「一键预设」接口
`AudioPostProcessWrapper::LoadPreset(int id)`——与酷狗客户端 UI 点击音效面板
完全同源的处理链。

## 目录结构

```
rust/
├── Cargo.toml           # 目标: i686-pc-windows-msvc (dsp.dll 是 32 位)
├── dll/                 # 依赖模块副本(开发用)
├── dist/                # 独立发行目录
│   ├── kugou-dsp.exe    # 主程序 (32 位)
│   └── dll/             # ★ dsp/infra/logging/MSVC CRT 全部在此子目录
└── src/
    ├── main.rs          # CLI 入口 (info / wav)
    ├── presets.rs       # 预设 ID 总表 + CLI 白名单
    ├── ffi.rs           # dsp.dll 符号动态解析 + 签名定义
    ├── engine.rs        # 引擎会话封装 (LoadPreset 路线)
    └── offline.rs       # WAV -> 预设 -> WAV 离线管线
```

## 使用

```pwsh
cd A:\KuGou\rust

# 加载自检
.\dist\kugou-dsp.exe info

# 施加音效预设 (--preset 仅接受白名单 ID)
.\dist\kugou-dsp.exe wav test_in.wav out_3d.wav   --preset 1
.\dist\kugou-dsp.exe wav test_in.wav out_bass.wav --preset 2
.\dist\kugou-dsp.exe wav test_in.wav out_vocal.wav --preset 3
.\dist\kugou-dsp.exe wav test_in.wav out_hifi.wav --preset 4

# 保留结尾混响拖尾（输出比输入略长）
.\dist\kugou-dsp.exe wav test_in.wav out_tail.wav --preset 3 --keep-tail
```

`--dll-dir` 默认**仅搜索 exe 同目录的 `dll\` 子目录**（不回退其他路径），
找不到 `dsp.dll` 时会直接报错；DLL 放在其他位置时必须显式传参。

## 全部音效预设总表

以下为静态逆向 kugou.dll（UI 层）与 dsp.dll（引擎层）得出的完整映射。
预设名来自 kugou.dll 内置名称表（fileoff `0xE72D48`，UTF-16）；
ID 经延迟导入符号 `?LoadPreset@AudioPostProcessWrapper@@QAEXH@Z` 传入引擎。

| ID | 预设名 | CLI 可用 | UI 图标资源 ID | 说明 |
|---:|---|:-:|---|---|
| -1 | 默认 | — | — | 关闭音效，重置引擎默认参数 |
| 1 | 3D丽音 | ✔ | `0x2694` | HRTF 声场强化，轻空间感突出清晰度 |
| 2 | 超重低音 | ✔ | `0x2695` | 低频增强 |
| 3 | 纯净人声 | ✔ | `0x2696` | 收敛混响拖尾，人声凸显 |
| 4 | HIFI现场 | ✔ | `0x2698` | 干声直通+短早期反射，临场感 |
| 5 | 3D旋转 | — | `0x2284` | 声场旋转 |
| 6 | 5.1全景 | — | `0x25C5` | 多声道全景渲染 |
| 7 | 黑胶唱片 | — | `0x25C6` | 黑胶模拟 |
| 8 | 虚拟环境 | — | `0x25CA` | 虚拟环境声 |
| 9 | 耳机音效 | — | `0x25CB` | 耳机优化 |
| 10 | 声乐古风 | — | `0x25CC` | 古风人声 |

> 标注"CLI 可用"的 4 个为本工具开放的白名单（见 `src/presets.rs`
> 中 `CLI_ALLOWED`），其余预设引擎同样接受对应 ID，但本工具不开放，
> 如需启用修改 `CLI_ALLOWED` 即可。
> ID 与名称的绑定数组位于 kugou.dll `.data 0x111648C0`（运行时构造），
> 「index 即 presetID」为高置信推断；`-1 = 默认/关闭` 已由反汇编确证
> （dsp.dll VA `0x100058E0`: `cmp [ebp+8], -1` 分支关闭声场组件）。

### 引擎层程序号空间

dsp.dll 内部 `SetProgram(pid)`（VA `0x10022A60`）合法域为 **0 ~ 112**
共 113 组内置程序。每组程序为 **27 个 float（108 字节）** 的效果参数包
（干湿比 / 时间常数 / 阻尼 / 宽度等），连续存放于只读参数表：

```
参数表基址 VA 0x101222B8, 每项 0x6C 字节:
  SetProgram(pid): memcpy(obj+0x16C, table + pid*0x6C, 108)
                   -> ApplyParams(0x10026C40) -> Rebuild(0x10026390)
```

酷狗 UI 的 10 个预设即该表的一个子集映射；上表之外的 100 余组程序
（多为不同空间/混响变体）无 UI 名称，未纳入本工具。

## 逆向结论（本程序的技术依据）

### 启动链

KuGou.exe → LoadLibrary("kugou.dll") → GetProcAddress("KugouMain")；
音效引擎在 **dsp.dll**，导出 82 个 MSVC mangled C++ 符号。
kugou.dll 经**延迟导入**调用 dsp.dll（UI 效果应用入口 VA `0x1092867F`，
配置键名 `"effectType"`）。

### 本工具使用的 FFI 签名

| 符号 | 约定 | 说明 |
|---|---|---|
| `??0AudioPostProcessWrapper@@QAE@XZ` | thiscall | 包装器构造（对象仅一个指针成员） |
| `?LoadPreset@AudioPostProcessWrapper@@QAEXH@Z` | thiscall | 一键预设：`id==-1` 关闭，`>=0` 启用声场组件并 SetProgram(id) |
| `?GetExternalDSPSet@AudioPostProcessWrapper@@QAE_NPAPAUIExternalDSPSet@@@Z` | thiscall | 取 COM 风格处理接口 |

### IExternalDSPSet 处理接口（vtable）

```c
// +0x10 flush: 初始化流状态
void Flush(IExternalDSPSet* this, int64_t pts /*0*/, double sampleRate);

// +0x14 process: ret 0x30, this + 11 参数 (COM stdcall)
int Process(
    IExternalDSPSet* this,   // [ebp+8]
    void* dst,               // a1 输出缓冲(可与 src 相同=原位)
    uint  byteLen,           // a2 数据总字节数
    void* src,               // a3 输入缓冲
    uint* written,           // a4 回写实际输出字节
    ...                      // a5 未确认
    uint sampleRate,         // a6
    uint channels,           // a7 声道数(参与整除)
    uint bitsPerSample,      // a8 位深((bits+7)>>3 = 每样本字节)
    uint flag /*0*/,         // a9
    uint p10, p11);          // 时间戳/保留
// 返回 1=成功; *written 回写字节数已实测精确
```

### dsp.dll 关键地址参考（i386 VA, ImageBase 0x10000000）

| 地址 | 含义 |
|---|---|
| `0x10005D4E` | `LoadPreset` 导出函数（薄封装） |
| `0x100058E0` | LoadPreset 核心：开关组件 + 双效果器 SetProgram(preset) |
| `0x10021090` | `SetEffectEnable(effectID, bool)` 组件开关（0x1004=声场主效果） |
| `0x10021455+` | process 音频处理链（按组件开关串联） |
| `0x10022A60` | `SetProgram(pid)` 分发器（查 113 组参数表） |
| `0x101222B8` | 预设参数表（113 × 27 float，每项 0x6C 字节） |
| `0x10026D10` | 声道方位角表（±90°/±150°/±45°/±135°，Atoms Surround 声场渲染配置） |
| RTTI | `ViPER3DRender/HRTF3DImpl/Audio3DSource/Virtualizer/Surround/Reberation/FFTFilter/SmartVolume/Limiter` 及酷狗自研 `Atoms*` 系列 |

### kugou.dll 关键地址参考（fileoff）

| 地址 | 含义 |
|---|---|
| `0xE72D48` | 音效预设名称表（UTF-16，10 预设） |
| `0xE73100` | 源文件路径串 `audio_plugin\ui_dsp_manager.cpp` |
| `0x10217216` | `UIDSPManager::SetEffect(index, enable)` |
| `0x1092867F` | effectType 应用入口（弹「使用蝰蛇音效 成功/失败」提示） |
| `0x111648C0` | 运行时构造的效果描述表（9×0x44，静态为空） |

## 注意事项 / 已知限制

1. **必须 32 位进程**：构建目标锁定 i686-pc-windows-msvc
2. **帧大小敏感**：引擎内部缓冲按小帧设计，管线使用 512 帧/块；
   大块(4096+)会触发内部越界
3. **增益与限幅**：效果链自带内部增益（LoadPreset(-1) 重置值
   4.0/0.25/1.0/3.0）与限幅组件，输入输出均为线性 int16 量化，
   管线不做任何额外软限幅（客户端同源行为）
4. **单次运行型内存策略**：C++ 对象随进程退出回收，不做释放
   （规避跨 CRT 释放问题）；长驻进程集成需自行补 Release/Dtor 调用
5. **声道数**：预设链按立体声设计验证，单声道输入行为未保证
6. **引擎延迟模型**：Process 为「推入-处理-拉回」结构（dsp.dll
   VA 0x10021430），预热期 written=0 且输入在内部 FIFO 积压，
   稳态后以恒定延迟（512 帧/块时约 9 块 ≈100ms@44.1k）流出，
   无数据丢失；默认输出与输入等长，`--keep-tail` 排空 FIFO
   保留完整混响尾

## 构建

```pwsh
rustup target add i686-pc-windows-msvc   # 仅首次
cd A:\KuGou\rust
cargo build --release --target i686-pc-windows-msvc
# dist\ 目录即为独立可分发产物
```
