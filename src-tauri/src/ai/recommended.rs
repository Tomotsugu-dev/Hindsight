//! Hindsight 自带的推荐 vision LLM 表。
//!
//! ## 数据 vs 代码
//!
//! 列表本身放在 [`src-tauri/resources/recommended-models.json`]（编译时 `include_str!`
//! 嵌入二进制，发布产物零外部文件依赖）。本文件只负责：
//!   - 定义跟前端共用的 [`Recommended`] 结构（`#[serde(rename_all = "camelCase")]` 跟 JSON 对齐）
//!   - 启动时解析一次 JSON 缓存进 `OnceLock`，后续 [`recommended`] 调用直接返回切片
//!
//! 解析失败不致命：log 一条 error + 返回空切片。前端会看到「推荐区为空」，
//! 用户可以走「自定义 HF 仓库」表单或推荐卡之外的本地导入路径，不影响核心采集。
//!
//! ## 增 / 减项流程
//!
//! 改 [`recommended-models.json`]（不动 .rs），重新打包发布即可。

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// 推荐模型。`mainFile` 是聊天主权重；`mmprojFile` 是 vision 投影
/// （仅 vision 模型有，纯文本模型为 None）。两个文件都从 [`Recommended::repo`] 取直链。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommended {
    /// 给用户看的简短名，如 "Qwen2.5-VL 3B (Q4_K)"
    pub display_name: String,
    /// HuggingFace 仓库 ID，形如 `ggml-org/Qwen2.5-VL-3B-Instruct-GGUF`
    pub repo: String,
    /// 主权重 GGUF 文件名（不带路径）
    pub main_file: String,
    /// 主权重字节数，前端展示 "约 1.9 GB" 用
    pub main_bytes: u64,
    /// vision 投影文件；纯文本模型为 `None`
    #[serde(default)]
    pub mmproj_file: Option<String>,
    /// vision 投影字节数；`mmproj_file = None` 时无意义，填 0 即可
    #[serde(default)]
    pub mmproj_bytes: u64,
    /// 模型品牌 logo URL（HF org 头像直链）；`None` 时前端 fallback 到首字母占位。
    /// 跟 `repo` 不同——`repo` 可能是镜像维护者（unsloth），logo 反映的是原作者品牌（Qwen / Google / zai-org）。
    #[serde(default)]
    pub logo_url: Option<String>,
    /// 是否支持 vision 输入（图描述 / step 1 任务）。
    ///
    /// 跟"有没有 mmproj 文件"**不是同一回事**——某些镜像仓库附带 mmproj 文件但模型架构本身
    /// 不支持图像，加载会失败。本字段由维护者按"原始模型卡 + llama.cpp 实测能否处理图片"
    /// 显式标定，不要靠文件名 / mmproj 存在性自动推断。
    /// `false` = 纯文本，前端会禁用 step 1 toggle；`true` = 兼容图描述 + 段总结。
    #[serde(default)]
    pub vision: bool,
    /// 品牌标识——前端按这个值分组筛选，避免靠 displayName / logoUrl 字符串硬匹配。
    /// 可选值：Qwen / Google / DeepSeek / OpenAI / Z.AI（由 JSON 维护者显式填）。
    /// 空串 = 不参与品牌筛选（旧 JSON 兼容）。
    #[serde(default)]
    pub brand: String,
    /// 能力 / 定位标签，前端按 pastel 色 chip 渲染在模型名右侧。
    ///
    /// 约定大写英文缩写，便于跨语言一致 + 紧凑显示：
    /// - **能力类**：`TEXT`、`VISION`、`CODE`
    /// - **定位类**：`FAST`（< 1GB 轻量）、`BALANCED`（中量）、`REASONING`（深度推理）、`R1`（DeepSeek R1 系）
    /// - **标记类**：`DEFAULT`（仅一条作为"首推"）
    ///
    /// 由 JSON 维护者按模型实测特性填；前端拿到不认识的 type 会按 default fallback 色显示。
    #[serde(default)]
    pub caps: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RecommendedRoot {
    models: Vec<Recommended>,
}

/// 数据来源：编译时嵌入的 JSON，运行时不需要外部文件。
const RECOMMENDED_JSON: &str = include_str!("../../resources/recommended-models.json");

/// 拿当前推荐模型列表（启动后固定不变；首次调用时解析 JSON 缓存）。
/// 解析失败 log 一条 error 后返回空切片，让前端推荐区空着——核心功能不受影响。
pub fn recommended() -> &'static [Recommended] {
    static CACHE: OnceLock<Vec<Recommended>> = OnceLock::new();
    CACHE
        .get_or_init(|| match serde_json::from_str::<RecommendedRoot>(RECOMMENDED_JSON) {
            Ok(root) => root.models,
            Err(e) => {
                log::error!(
                    "recommended-models.json 解析失败：{e}（推荐区将为空，用户仍可走自定义 HF / 本地导入）"
                );
                Vec::new()
            }
        })
        .as_slice()
}
