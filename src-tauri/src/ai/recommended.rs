//! Hindsight 自带的推荐 vision LLM 表（Phase 1B-β β.1）。
//!
//! 用户进 AI 设置页时拿这张表渲染推荐卡片。每条记录至少有 main 文件；
//! vision 模型还会有 mmproj 文件——`Recommended::mmproj` 为 `Some` 表示
//! 必须连带下载。
//!
//! 升级 / 增减项的流程：
//! 1. 先在 HuggingFace 找到对应 GGUF repo（推荐 ggml-org 官方维护的）
//! 2. 验证 llama-server 能加载（startup log 没 unsupported / failed）
//! 3. 用浏览器访问 HF 文件页获取 size_bytes（HF 在 file metadata 显示）
//! 4. 把 size 填进来后跑一次端到端冒烟，确保下载 + 加载 + 推理 OK

use serde::Serialize;

/// 推荐模型。`main_file` 是聊天主权重；`mmproj` 是 vision 投影
/// （仅 vision 模型有）。两个文件都从 `<repo>` 取直链。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommended {
    /// 给用户看的简短名，如 "Qwen2.5-VL 3B (Q4_K)"
    pub display_name: &'static str,
    /// HuggingFace 仓库 ID，形如 `ggml-org/Qwen2.5-VL-3B-Instruct-GGUF`
    pub repo: &'static str,
    /// 主权重 GGUF 文件名（不带路径）
    pub main_file: &'static str,
    /// 主权重字节数，前端展示 "约 1.9 GB" 用
    pub main_bytes: u64,
    /// vision 投影文件；纯文本模型为 `None`
    pub mmproj_file: Option<&'static str>,
    /// vision 投影字节数；`mmproj_file = None` 时无意义，填 0 即可
    pub mmproj_bytes: u64,
}

/// Hindsight 推荐的 vision LLM 列表。
///
/// 收录原则：
/// 1. llama.cpp 主线（master）支持，不需要 fork
/// 2. HuggingFace 上有官方维护的 GGUF（ggml-org / Qwen / unsloth 优先）
/// 3. 文件名 + 字节数已经手工核对过，下载链接稳定
///
/// 排序按"轻 → 重"，第一项最适合首次试用（CPU 也能跑顺）。
/// 字节数 = HF 文件元数据，可能有 ±1% 偏差，下载层用容差比对判已存在。
///
/// 升级 / 增减项流程：
/// 1. HF 仓库上手动验证文件名和大小（点 Files and versions）
/// 2. 验证 llama.cpp `llama-server` 实际能加载 + 推理
/// 3. 改这张表 + 重新 build
pub const RECOMMENDED: &[Recommended] = &[
    // 由轻到重；用户首次试用建议从第一个开始
    Recommended {
        display_name: "Qwen3-VL 4B (Q4_K_M)",
        repo: "Qwen/Qwen3-VL-4B-Instruct-GGUF",
        main_file: "Qwen3VL-4B-Instruct-Q4_K_M.gguf",
        main_bytes: 2_500_000_000,
        mmproj_file: Some("mmproj-Qwen3VL-4B-Instruct-Q8_0.gguf"),
        mmproj_bytes: 454_000_000,
    },
    Recommended {
        display_name: "Gemma 4 E2B-it (Q8_0)",
        repo: "ggml-org/gemma-4-E2B-it-GGUF",
        main_file: "gemma-4-E2B-it-Q8_0.gguf",
        main_bytes: 4_970_000_000,
        mmproj_file: Some("mmproj-gemma-4-E2B-it-Q8_0.gguf"),
        mmproj_bytes: 557_000_000,
    },
    Recommended {
        display_name: "Gemma 4 E4B-it (Q4_K_M)",
        repo: "ggml-org/gemma-4-E4B-it-GGUF",
        main_file: "gemma-4-E4B-it-Q4_K_M.gguf",
        main_bytes: 5_340_000_000,
        mmproj_file: Some("mmproj-gemma-4-E4B-it-Q8_0.gguf"),
        mmproj_bytes: 560_000_000,
    },
    Recommended {
        // 用户原本要 Qwen3.5-9B，但 Qwen 3.5 没出 VL 版本——
        // 替换成 Qwen3-VL-8B，是当前能看图的最大 Qwen，体量接近
        display_name: "Qwen3-VL 8B (Q4_K_M)",
        repo: "Qwen/Qwen3-VL-8B-Instruct-GGUF",
        main_file: "Qwen3VL-8B-Instruct-Q4_K_M.gguf",
        main_bytes: 5_030_000_000,
        mmproj_file: Some("mmproj-Qwen3VL-8B-Instruct-Q8_0.gguf"),
        mmproj_bytes: 752_000_000,
    },
];
