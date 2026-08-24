//! 酷狗音效预设定义表
//!
//! 逆向依据（见 README「全部音效预设总表」）：
//! - 预设经 dsp.dll 导出 `?LoadPreset@AudioPostProcessWrapper@@QAEXH@Z` 以整数 ID 加载
//! - ID == -1 表示关闭音效（重置为默认参数）
//! - 合法域 0~112（dsp.dll 内部 SetProgram 参数表共 113 组），
//!   酷狗 UI 层 (kugou.dll) 仅映射其中 10 个预设 + 默认

/// 单个预设的元信息（desc 为内嵌文档，供维护者参考）
#[allow(dead_code)]
pub struct PresetInfo {
    /// LoadPreset 整数 ID
    pub id: i32,
    /// 酷狗 UI 原始名称
    pub name: &'static str,
    /// 简要说明
    pub desc: &'static str,
}

/// 全部已知音效预设（与 kugou.dll 名称表 fileoff 0xE72D48 一致）
pub const PRESETS: &[PresetInfo] = &[
    PresetInfo { id: -1, name: "默认",       desc: "关闭音效，重置引擎默认参数" },
    PresetInfo { id: 1,  name: "3D丽音",     desc: "HRTF 声场强化，轻空间感突出清晰度" },
    PresetInfo { id: 2,  name: "超重低音",   desc: "低频增强" },
    PresetInfo { id: 3,  name: "纯净人声",   desc: "收敛混响拖尾，人声凸显" },
    PresetInfo { id: 4,  name: "HIFI现场",   desc: "干声直通+短早期反射，临场感" },
    PresetInfo { id: 5,  name: "3D旋转",     desc: "声场旋转" },
    PresetInfo { id: 6,  name: "5.1全景",    desc: "多声道全景渲染" },
    PresetInfo { id: 7,  name: "黑胶唱片",   desc: "黑胶模拟" },
    PresetInfo { id: 8,  name: "虚拟环境",   desc: "虚拟环境声" },
    PresetInfo { id: 9,  name: "耳机音效",   desc: "耳机优化" },
    PresetInfo { id: 10, name: "声乐古风",   desc: "古风人声" },
];

/// CLI 允许使用的预设 ID 白名单：3D丽音 / 超重低音 / 纯净人声 / HIFI现场
pub const CLI_ALLOWED: &[i32] = &[1, 2, 3, 4];

/// 是否允许在 CLI 上使用该预设
pub fn is_cli_allowed(id: i32) -> bool {
    CLI_ALLOWED.contains(&id)
}

/// 按 ID 查询预设名称；未知 ID 返回 None
pub fn name_of(id: i32) -> Option<&'static str> {
    PRESETS.iter().find(|p| p.id == id).map(|p| p.name)
}

/// 拼接白名单的展示文本（用于 usage 与错误提示）
pub fn allowed_list_text() -> String {
    CLI_ALLOWED
        .iter()
        .filter_map(|id| name_of(*id).map(|n| format!("{id}={n}")))
        .collect::<Vec<_>>()
        .join("  ")
}
