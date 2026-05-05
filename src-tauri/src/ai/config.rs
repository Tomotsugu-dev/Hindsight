//! AI 设置数据结构 + 净化逻辑。
//!
//! - [`AiConfig`] 嵌进 `Settings::ai`，跟着 settings_store JSON 一起持久化
//! - [`AiSegment`] 是一天里被划分出的一个时段，AI 按段汇总
//! - [`sanitize`] 在 settings 写入路径上调用，把非法值钳到合法范围

use serde::{Deserialize, Serialize};

/// 一天内的一个时段，AI 按段聚合截图 + 活动做总结。
///
/// 取值约束：`start_hour ∈ 0..=23`、`end_hour ∈ 1..=24`、`start_hour < end_hour`。
/// 不支持跨午夜（晚段最大 `[18, 24]`）。约束在 [`sanitize`] 里强制。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AiSegment {
    pub label: String,
    pub start_hour: u8,
    pub end_hour: u8,
    /// 用户自定义底色，hex 格式 `#rrggbb`；空字符串 = 走 UI 自动按时段渐变
    pub color: String,
}

/// AI 子系统的所有用户配置。嵌进 [`crate::repo::settings::Settings::ai`]。
///
/// `#[serde(default)]` 让旧 settings JSON（没有 ai 字段）反序列化时自动填默认值，
/// 不需要 schema migration。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AiConfig {
    /// OpenAI 兼容 base URL（不含 `/chat/completions` 路径片段）。
    /// 默认指向本机 Ollama。
    pub endpoint: String,
    /// 用户填的模型 ID（如 `minicpm-v:8b`）；空字符串 = 还没配置好
    pub model: String,
    /// 可选 Bearer token，Ollama 不需要
    pub api_key: String,
    /// 用户对自己的简短描述，AI 总结时拼进 system prompt
    pub user_brief: String,
    /// 一天的时段划分；UI 上是连续条
    pub segments: Vec<AiSegment>,
    /// 排除分析的 category id 列表
    pub excluded_categories: Vec<String>,
    /// 单段送 AI 的截图上限
    pub max_images_per_segment: u32,
    /// dHash 64bit 汉明距离阈值，用于截图去重
    pub hash_threshold: u32,
    /// 哈希聚类时间窗（分钟）；只在窗内的截图之间比相似度
    pub hash_window_minutes: u32,
    /// 模型（GGUF 文件）保存路径。
    ///
    /// 空字符串 = 走 [`crate::ai::models::default_root_dir`]（`<data_root>/ai/models/`）；
    /// `repo::settings::load` 会在启动时把空值填成实际默认路径。
    /// 用户在 设置 → 数据 里能改成大硬盘上的目录。
    pub models_path: String,
    /// 当前选中的主权重 GGUF 文件名（在 `models_path` 目录下）。
    /// 空字符串 = 还没选模型；`start_engine` 会拒绝启动，让用户先去选。
    pub active_main: String,
    /// 当前选中的 mmproj GGUF 文件名（vision 模型必带）。
    /// 空字符串 = 没有 mmproj（纯文本模型）。
    pub active_mmproj: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:11434/v1".to_string(),
            model: String::new(),
            api_key: String::new(),
            user_brief: String::new(),
            segments: default_segments(),
            excluded_categories: vec!["other".to_string()],
            max_images_per_segment: 30,
            hash_threshold: 5,
            hash_window_minutes: 5,
            models_path: String::new(),
            active_main: String::new(),
            active_mmproj: String::new(),
        }
    }
}

/// 默认 5 段，覆盖整 24 小时：
/// 深夜 00-06 / 早上 06-09 / 上午 09-12 / 下午 12-18 / 晚上 18-24
pub fn default_segments() -> Vec<AiSegment> {
    vec![
        AiSegment {
            label: "深夜".to_string(),
            start_hour: 0,
            end_hour: 6,
            color: String::new(),
        },
        AiSegment {
            label: "早上".to_string(),
            start_hour: 6,
            end_hour: 9,
            color: String::new(),
        },
        AiSegment {
            label: "上午".to_string(),
            start_hour: 9,
            end_hour: 12,
            color: String::new(),
        },
        AiSegment {
            label: "下午".to_string(),
            start_hour: 12,
            end_hour: 18,
            color: String::new(),
        },
        AiSegment {
            label: "晚上".to_string(),
            start_hour: 18,
            end_hour: 24,
            color: String::new(),
        },
    ]
}

/// 把用户提交的 AiConfig 钳到合法范围。
///
/// 注意：segments 过滤后**全空**时回退到 `old.segments`，避免用户误删空了
/// 整组时段（前端 UI 也不应允许空，但兜底一层稳）。
///
/// 字段处理：
/// - 字符串：trim
/// - segments：过滤掉 `start_hour >= end_hour` 或 `end_hour > 24` 的项
/// - 数值字段：clamp 到合理范围
pub fn sanitize(mut next: AiConfig, old: &AiConfig) -> AiConfig {
    next.endpoint = next.endpoint.trim().to_string();
    next.model = next.model.trim().to_string();
    next.api_key = next.api_key.trim().to_string();
    next.user_brief = next.user_brief.trim().to_string();

    let valid_segments: Vec<AiSegment> = next
        .segments
        .into_iter()
        .filter(|s| s.start_hour < s.end_hour && s.end_hour <= 24)
        .map(|mut s| {
            s.label = s.label.trim().to_string();
            s.color = sanitize_hex_color(&s.color);
            s
        })
        .collect();
    next.segments = if valid_segments.is_empty() {
        old.segments.clone()
    } else {
        valid_segments
    };

    next.excluded_categories = next
        .excluded_categories
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    next.max_images_per_segment = next.max_images_per_segment.clamp(1, 200);
    next.hash_threshold = next.hash_threshold.min(32);
    next.hash_window_minutes = next.hash_window_minutes.min(60);

    next.models_path = next.models_path.trim().to_string();
    next.active_main = next.active_main.trim().to_string();
    next.active_mmproj = next.active_mmproj.trim().to_string();

    next
}

/// 校验 hex 颜色：接受 `#rgb` / `#rrggbb`，统一返回小写 `#rrggbb`；非法值置空。
fn sanitize_hex_color(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return String::new();
    }
    let body = match s.strip_prefix('#') {
        Some(b) => b,
        None => return String::new(),
    };
    let valid_len = matches!(body.len(), 3 | 6);
    if !valid_len || !body.chars().all(|c| c.is_ascii_hexdigit()) {
        return String::new();
    }
    if body.len() == 3 {
        let mut out = String::with_capacity(7);
        out.push('#');
        for c in body.chars() {
            let lc = c.to_ascii_lowercase();
            out.push(lc);
            out.push(lc);
        }
        out
    } else {
        let mut out = String::with_capacity(7);
        out.push('#');
        for c in body.chars() {
            out.push(c.to_ascii_lowercase());
        }
        out
    }
}
