//! AI 设置数据结构 + 净化逻辑。
//!
//! - [`AiConfig`] 嵌进 `Settings::ai`，跟着 settings_store JSON 一起持久化
//! - [`AiSegment`] 是一天里被划分出的一个时段，AI 按段汇总
//! - [`sanitize`] 在 settings 写入路径上调用，把非法值钳到合法范围

use serde::{Deserialize, Serialize};

/// `summary_main` 的特殊值——"用云端 API 跑 step 2"。
///
/// 历史：早期 `external_enabled` 单一开关同时表示「云端配好可用」+「step 2 就用云端」。
/// 用户反馈：希望两件事分开 ——「云端 API tab 的启用」只表示配好可用，
/// 是否真的把云端选为 step 2 backend 应该是 Models tab 里独立的一次点击。
/// 实现上避免引入第二个布尔字段（容易跟 `external_enabled` 状态打架），
/// 复用 `summary_main` 一个槽位：本来它存 GGUF 文件名（或空 fallback 到 active_main），
/// 多塞这一个 sentinel 表示「目标不是文件，是云端」。
///
/// 路由判定走 [`AiConfig::summary_use_cloud`]——它把"标记为 cloud" + "external 配好"
/// 两条件合在一起，避免漏判其一造成静默退化。
pub const SUMMARY_CLOUD_SENTINEL: &str = "__cloud__";

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
    /// 外部云端 API 的 OpenAI 兼容 base URL（不含 `/chat/completions` 路径）。
    /// 仅在 `external_enabled = true` 时生效；step 1 单图描述永远走本地，不用这个。
    /// 默认空——前端要求用户主动选 provider 才填。
    pub endpoint: String,
    /// 外部 API 的模型 ID（如 `gpt-4o-mini` / `deepseek-chat`）；
    /// 仅在 `external_enabled = true` 时生效。
    pub model: String,
    /// 外部 API 跑 step 1 图片描述时用的 vision 模型 ID（如 `gpt-4o-mini` / `qwen-vl-plus`）。
    /// 空 = 复用 [`Self::model`]。仅在 [`Self::describe_use_cloud`] 时生效。
    /// 通过 [`Self::cloud_vision_model`] 读取，不要直接读字段。
    #[serde(default)]
    pub vision_model: String,
    /// 外部 API 跑 step 1 时用的独立 base URL；空 = 复用 [`Self::endpoint`]。
    /// 让「文本 API 没有多模态模型（如 DeepSeek）+ Vision 走另一家（如 Kimi）」的组合成立。
    /// 通过 [`Self::cloud_vision_endpoint`] 读取，不要直接读字段。
    #[serde(default)]
    pub vision_endpoint: String,
    /// Vision 独立 API 的 Bearer token；空 = 复用 [`Self::api_key`]。
    /// 通过 [`Self::cloud_vision_api_key`] 读取，不要直接读字段。
    #[serde(default)]
    pub vision_api_key: String,
    /// Vision API 的 provider 预设标识；空 = 前端视为「复用文本 API」。
    /// 后端不消费；UI 用它决定 Vision 子块的 Base URL / placeholder。
    #[serde(default)]
    pub vision_provider: String,
    /// 外部 API 的 Bearer token；明文落 settings JSON。
    pub api_key: String,
    /// 是否启用外部云端 API。
    /// false = 全程本地（默认）。true 时具体哪一步走云由各 step 槽位的
    /// `__cloud__` sentinel 决定（见 [`Self::summary_use_cloud`] /
    /// [`Self::describe_use_cloud`]）。step 1 选云端意味着**截图本身会上传**，
    /// 前端在用户选择时弹隐私确认。
    pub external_enabled: bool,
    /// Provider 预设标识（"openai" / "deepseek" / "openrouter" / "together" / "groq" / "custom"）。
    /// 后端只用来 sanitize；UI 用它决定 Base URL / Model 的 placeholder。
    pub external_provider: String,
    /// 用户对自己的简短描述，AI 总结时拼进 system prompt
    pub user_brief: String,
    /// 一天的时段划分；UI 上是连续条
    pub segments: Vec<AiSegment>,
    /// 排除分析的 category id 列表
    pub excluded_categories: Vec<String>,
    /// 单段送 AI 的截图上限
    pub max_images_per_segment: u32,
    /// 截图相似度去重阈值（余弦），段内贪心去重；step 1 vision LLM 之前砍冗余画面。
    /// 范围 0.70..=0.99；默认 0.95（POC 验证：~70% 去重率，肉眼无误删）。
    /// 越严越保守（接近 1）；越松越激进，可能误删。
    pub dedup_threshold: f32,
    /// 模型（GGUF 文件）保存路径。
    ///
    /// 空字符串 = 走 [`crate::ai::models::default_root_dir`]（`<data_root>/ai/models/`）；
    /// `repo::settings::load` 会在启动时把空值填成实际默认路径。
    /// 用户在 设置 → 数据 里能改成大硬盘上的目录。
    pub models_path: String,
    /// 当前选中的主权重 GGUF 文件名（在 `models_path` 目录下）。
    /// 空字符串 = 还没选模型；`start_engine` 会拒绝启动，让用户先去选。
    ///
    /// 历史遗留字段，扮演 step 1/2 各自字段的 fallback：当 `describe_main` /
    /// `summary_main` 为空时降级用本字段。读取时统一走
    /// [`Self::effective_describe_main`] / [`Self::effective_summary_main`]，
    /// 不要直接读。
    pub active_main: String,
    /// 当前选中的 mmproj GGUF 文件名（vision 模型必带）。
    /// 空字符串 = 没有 mmproj（纯文本模型）。
    /// fallback 语义同 [`Self::active_main`]——读取走 effective 方法。
    pub active_mmproj: String,
    /// step 1 图描述用的主权重 GGUF；空 = 降级到 [`Self::active_main`]。
    /// 用 [`Self::effective_describe_main`] 读取。
    #[serde(default)]
    pub describe_main: String,
    /// step 1 图描述用的 mmproj GGUF；空 = 降级到 [`Self::active_mmproj`]。
    #[serde(default)]
    pub describe_mmproj: String,
    /// step 2 段总结用的主权重 GGUF；空 = 降级到 [`Self::active_main`]。
    /// 段总结一般是纯文本任务，可挑更小或纯文本模型节省 VRAM。
    #[serde(default)]
    pub summary_main: String,
    /// step 2 段总结用的 mmproj GGUF；空 = 降级到 [`Self::active_mmproj`]。
    /// 一般纯文本模型这个留空即可。
    #[serde(default)]
    pub summary_mmproj: String,
    /// AI 总结使用的提示词语言（决定模型出哪种语言的总结 + 默认提示词模板用哪套）。
    /// 取值 "zh" / "tw" / "en" / "ja" / "pt"；非法值 sanitize 时回退到 "zh"。
    pub prompt_language: String,
    /// 用户对内置 system prompt（step 2 段总结）的覆盖；按语言分别存。
    /// 某语言对应字段为空 = 用内置默认；非空 = 走覆盖。
    pub prompt_overrides: PromptOverrides,
    /// 用户对内置 image describe prompt（step 1 单图描述）的覆盖；按语言分别存。
    /// 跟 [`prompt_overrides`] 同结构，互不干扰。
    pub image_describe_overrides: PromptOverrides,
    /// 引擎启动级参数：`--batch-size` / `--ubatch-size`（取一致值）。
    /// `None` = 不传，走 llama.cpp 默认 512。
    /// 改值会让下次引擎启动用新参数；引擎已在跑时不会自动重启，需用户主动 stop。
    ///
    /// 双套参数语义：这三个旧字段（`batch_size` / `parallel_slots` / `ctx_size`）
    /// 现在是 fallback——当对应的 `describe_*` / `summary_*` 字段为 `None` 时降级使用。
    /// 通过 [`Self::describe_batch_size_effective`] 等 getter 取值，调用方不要直接读字段。
    pub batch_size: Option<u32>,
    /// 引擎启动级参数：`-np N` 并行槽位数 + 后端 step 1 image describe 并发数。
    /// `None` = 1（串行）。详见 [`Self::batch_size`] 关于 fallback 语义。
    pub parallel_slots: Option<u32>,
    /// 引擎启动级参数：每 slot 的 ctx 上限（token）。
    /// 实际 `--ctx-size = ctx_size × parallel_slots`，让每槽都拿到这个 budget。
    /// `None` = 8K 默认。详见 [`Self::batch_size`] 关于 fallback 语义。
    pub ctx_size: Option<u32>,

    /// 图描述阶段（step 1，多图并行）的 batch 参数；`None` = fallback 到 [`Self::batch_size`]。
    pub describe_batch_size: Option<u32>,
    /// 图描述阶段的 `-np` 并行槽数；`None` = fallback 到 [`Self::parallel_slots`]。
    /// 这是双套参数的关键差异点——describe 默认推荐高 slots（多图并行）。
    pub describe_parallel_slots: Option<u32>,
    /// 图描述阶段的每槽 ctx；`None` = fallback 到 [`Self::ctx_size`]。
    pub describe_ctx_size: Option<u32>,

    /// 段总结阶段（step 2，单段串行）的 batch 参数；`None` = fallback 到 [`Self::batch_size`]。
    pub summary_batch_size: Option<u32>,
    /// 段总结阶段的 `-np`；`None` = fallback 到 [`Self::parallel_slots`]。
    /// 段总结无并行需求，推荐恒为 1，给 ctx 让出预算。
    pub summary_parallel_slots: Option<u32>,
    /// 段总结阶段的每槽 ctx；`None` = fallback 到 [`Self::ctx_size`]。
    /// 这是双套参数的关键差异点——summary 默认推荐高 ctx（容纳多图描述聚合）。
    pub summary_ctx_size: Option<u32>,
}

impl AiConfig {
    /// 取图描述阶段的 batch；新字段优先，未设则 fallback 到全局 `batch_size`。
    pub fn describe_batch_size_effective(&self) -> Option<u32> {
        self.describe_batch_size.or(self.batch_size)
    }
    /// 取图描述阶段的 slots；同上 fallback。
    pub fn describe_parallel_slots_effective(&self) -> Option<u32> {
        self.describe_parallel_slots.or(self.parallel_slots)
    }
    /// 取图描述阶段的 ctx；同上 fallback。
    pub fn describe_ctx_size_effective(&self) -> Option<u32> {
        self.describe_ctx_size.or(self.ctx_size)
    }
    /// 取段总结阶段的 batch；同上 fallback。
    pub fn summary_batch_size_effective(&self) -> Option<u32> {
        self.summary_batch_size.or(self.batch_size)
    }
    /// 取段总结阶段的 slots；同上 fallback。
    pub fn summary_parallel_slots_effective(&self) -> Option<u32> {
        self.summary_parallel_slots.or(self.parallel_slots)
    }
    /// 取段总结阶段的 ctx；同上 fallback。
    pub fn summary_ctx_size_effective(&self) -> Option<u32> {
        self.summary_ctx_size.or(self.ctx_size)
    }

    /// step 1 主权重文件名（去前后空白后非空）；空 → fallback 到 `active_main`。
    /// `describe_main == SUMMARY_CLOUD_SENTINEL` 也走 fallback——那是"用云端"标记，
    /// 不是真实文件名；走本地路径的代码应当看到 active_main（同 step 2 的语义）。
    pub fn effective_describe_main(&self) -> &str {
        let s = self.describe_main.trim();
        if s.is_empty() || s == SUMMARY_CLOUD_SENTINEL {
            self.active_main.as_str()
        } else {
            self.describe_main.as_str()
        }
    }
    /// step 1 mmproj 文件名。
    ///
    /// **mmproj 跟 main 配套 fallback**：`describe_main` 显式设了，mmproj 就只看
    /// `describe_mmproj`（空 = 该模型不需要 mmproj，纯文本模型）；只有 `describe_main`
    /// 也 fallback 到 `active_main` 时，mmproj 才 fallback 到 `active_mmproj`。
    ///
    /// 旧实现 mmproj 单独 fallback——会让"文本模型 step + 历史 active 是 vision"的组合
    /// 错把 vision mmproj 强加到文本模型上，llama-server 加载后 token embedding 错位，
    /// 推理首 token 即 EOS（"模型返回为空"）。
    pub fn effective_describe_mmproj(&self) -> &str {
        let s = self.describe_main.trim();
        if s.is_empty() || s == SUMMARY_CLOUD_SENTINEL {
            self.active_mmproj.as_str()
        } else {
            self.describe_mmproj.as_str()
        }
    }
    /// step 2 主权重文件名；空 → fallback 到 `active_main`。
    /// `summary_main == SUMMARY_CLOUD_SENTINEL` 也走 fallback —— 那是"用云端"标记，
    /// 不是真实文件名；走本地路径的代码（VRAM 估算 / fallback chain）应当看到 active_main。
    pub fn effective_summary_main(&self) -> &str {
        let s = self.summary_main.trim();
        if s.is_empty() || s == SUMMARY_CLOUD_SENTINEL {
            self.active_main.as_str()
        } else {
            self.summary_main.as_str()
        }
    }
    /// step 2 mmproj 文件名。配套 fallback 规则同 [`Self::effective_describe_mmproj`]。
    /// `summary_main` 为 sentinel 时同走 fallback（mmproj 跟主权重的来源绑定）。
    pub fn effective_summary_mmproj(&self) -> &str {
        let s = self.summary_main.trim();
        if s.is_empty() || s == SUMMARY_CLOUD_SENTINEL {
            self.active_mmproj.as_str()
        } else {
            self.summary_mmproj.as_str()
        }
    }

    /// step 2 是否实际路由到云端：要 `summary_main` 是"用云端"标记 **且** `external_enabled=true`。
    /// `external_enabled=false` 时 sentinel 退化为 fallback（按 `active_main` 跑本地），
    /// 不会因为 sentinel 残留就硬卡住没法跑总结。
    pub fn summary_use_cloud(&self) -> bool {
        self.external_enabled && self.summary_main.trim() == SUMMARY_CLOUD_SENTINEL
    }

    /// step 1 是否实际路由到云端：`describe_main` 是"用云端"标记 **且** `external_enabled=true`。
    /// 语义与 [`Self::summary_use_cloud`] 完全对齐：external 关掉时 sentinel 退化为
    /// fallback（按 `active_main` 跑本地），不会硬卡住。
    /// 注意：step 1 走云端 = **截图图片本体会上传**到所配置的第三方 API。
    pub fn describe_use_cloud(&self) -> bool {
        self.external_enabled && self.describe_main.trim() == SUMMARY_CLOUD_SENTINEL
    }

    /// 云端 step 1 用的模型 ID：`vision_model` 非空用它，空则复用 [`Self::model`]。
    pub fn cloud_vision_model(&self) -> &str {
        let v = self.vision_model.trim();
        if v.is_empty() {
            self.model.as_str()
        } else {
            v
        }
    }

    /// 云端 step 1 用的 base URL：`vision_endpoint` 非空用它，空则复用 [`Self::endpoint`]。
    pub fn cloud_vision_endpoint(&self) -> &str {
        let v = self.vision_endpoint.trim();
        if v.is_empty() {
            self.endpoint.as_str()
        } else {
            v
        }
    }

    /// 云端 step 1 用的 API key：`vision_api_key` 非空用它，空则复用 [`Self::api_key`]。
    pub fn cloud_vision_api_key(&self) -> &str {
        let v = self.vision_api_key.trim();
        if v.is_empty() {
            self.api_key.as_str()
        } else {
            v
        }
    }

    /// step 1 单次响应 max_tokens：让用户配的 ctx_size 能反映到响应能写多长。
    /// 取 effective ctx 的一半（给 prompt 留另一半）；不够 1024 也给 1024 兜底
    /// 避免短输出被截。
    pub fn describe_max_tokens(&self) -> u32 {
        let ctx = self.describe_ctx_size_effective().unwrap_or(8192);
        (ctx / 2).max(1024)
    }

    /// step 2 单次响应 max_tokens。同 [`Self::describe_max_tokens`] 策略，
    /// 但下界 2048（给 reasoning 模型思考链最低保障）。
    pub fn summary_max_tokens(&self) -> u32 {
        let ctx = self.summary_ctx_size_effective().unwrap_or(8192);
        (ctx / 2).max(2048)
    }
}

/// 用户编辑过的 system prompt 覆盖文本，按语言分别独立存。
///
/// 切换 `prompt_language` 不会丢覆盖：用户先在中文写过的覆盖，切到英文再切回中文还在。
/// 若想恢复内置默认，把对应字段清空（"重置"按钮做的就是这件事）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PromptOverrides {
    /// 中文 system prompt 覆盖；空 = 用内置默认
    pub system_zh: String,
    /// 英文 system prompt 覆盖
    pub system_en: String,
    /// 日文 system prompt 覆盖
    pub system_ja: String,
    /// 葡萄牙语（巴西）system prompt 覆盖
    pub system_pt: String,
    /// 繁体中文（台湾）system prompt 覆盖
    #[serde(default)]
    pub system_tw: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        let lang = detect_default_lang();
        Self {
            endpoint: String::new(),
            model: String::new(),
            vision_model: String::new(),
            vision_endpoint: String::new(),
            vision_api_key: String::new(),
            vision_provider: String::new(),
            api_key: String::new(),
            external_enabled: false,
            external_provider: "openai".to_string(),
            user_brief: String::new(),
            segments: default_segments_for(lang),
            excluded_categories: vec!["other".to_string()],
            max_images_per_segment: 1024,
            dedup_threshold: 0.95,
            models_path: String::new(),
            active_main: String::new(),
            active_mmproj: String::new(),
            describe_main: String::new(),
            describe_mmproj: String::new(),
            summary_main: String::new(),
            summary_mmproj: String::new(),
            prompt_language: lang.to_string(),
            prompt_overrides: PromptOverrides::default(),
            image_describe_overrides: PromptOverrides::default(),
            batch_size: None,
            parallel_slots: None,
            ctx_size: None,
            describe_batch_size: None,
            describe_parallel_slots: None,
            describe_ctx_size: None,
            summary_batch_size: None,
            summary_parallel_slots: None,
            summary_ctx_size: None,
        }
    }
}

/// 默认 5 段，覆盖整 24 小时（00-06 / 06-09 / 09-12 / 12-18 / 18-24）；
/// 标签按用户语言取一套。新装首启时通过 [`detect_default_lang`] 拿系统 locale。
pub fn default_segments_for(lang: &str) -> Vec<AiSegment> {
    let labels: [&str; 5] = match lang {
        "en" => [
            "Late Night",
            "Early Morning",
            "Morning",
            "Afternoon",
            "Evening",
        ],
        "ja" => ["深夜", "早朝", "午前", "午後", "夜"],
        "pt" => ["Madrugada", "Manhã cedo", "Manhã", "Tarde", "Noite"],
        _ => ["深夜", "早上", "上午", "下午", "晚上"],
    };
    let ranges: [(u8, u8); 5] = [(0, 6), (6, 9), (9, 12), (12, 18), (18, 24)];
    labels
        .into_iter()
        .zip(ranges)
        .map(|(label, (start_hour, end_hour))| AiSegment {
            label: label.to_string(),
            start_hour,
            end_hour,
            color: String::new(),
        })
        .collect()
}

/// 从系统 locale 推默认 prompt 语言：繁体圈 → "tw"、其余 `zh-*` → "zh"、`ja-*` → "ja"、其它 → "en"。
/// 仅在首次安装 `AiConfig::default()` 时调一次；用户后续在 UI 改了再不动。
pub fn detect_default_lang() -> &'static str {
    match sys_locale::get_locale() {
        Some(loc) => {
            let l = loc.to_ascii_lowercase();
            if l.starts_with("zh") {
                // 繁体圈（台湾 / 香港 / 澳门 / Hant 脚本）→ 繁体提示词
                let hant = [
                    "zh-tw", "zh_tw", "zh-hk", "zh_hk", "zh-mo", "zh_mo", "zh-hant", "zh_hant",
                ];
                if hant.iter().any(|p| l.starts_with(p)) {
                    "tw"
                } else {
                    "zh"
                }
            } else if l.starts_with("ja") {
                "ja"
            } else if l.starts_with("pt") {
                "pt"
            } else {
                "en"
            }
        }
        None => "en",
    }
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

    // external_provider：只接受预设值，非法回退到 "openai"
    next.external_provider = match next.external_provider.trim() {
        "openai" | "deepseek" | "openrouter" | "together" | "groq" | "custom" => {
            next.external_provider.trim().to_string()
        }
        _ => "openai".to_string(),
    };

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

    // 上限抬到 10w 是给「无限制」档留路——真正撑爆 ctx 时 LLM 会返 400，
    // 段标 status='error'，用户看到再调小就好；不在这层 silent 截断
    next.max_images_per_segment = next.max_images_per_segment.clamp(1, 100_000);
    next.dedup_threshold = next.dedup_threshold.clamp(0.70, 0.99);

    // 引擎启动级参数 clamp（跟 AiOverrides 一致）：
    // batch ≥ 32 是 llama-server 不接受过小值的安全下限
    // ctx 上限给 256K（极限场景，超出基本任何卡都装不下，没必要再大）
    // parallel_slots ≥ 1，给 32 上限避免误填出格的值
    next.batch_size = next.batch_size.map(|b| b.clamp(32, 32_768));
    next.parallel_slots = next.parallel_slots.map(|n| n.clamp(1, 32));
    next.ctx_size = next.ctx_size.map(|c| c.clamp(512, 262_144));
    next.describe_batch_size = next.describe_batch_size.map(|b| b.clamp(32, 32_768));
    next.describe_parallel_slots = next.describe_parallel_slots.map(|n| n.clamp(1, 32));
    next.describe_ctx_size = next.describe_ctx_size.map(|c| c.clamp(512, 262_144));
    next.summary_batch_size = next.summary_batch_size.map(|b| b.clamp(32, 32_768));
    next.summary_parallel_slots = next.summary_parallel_slots.map(|n| n.clamp(1, 32));
    next.summary_ctx_size = next.summary_ctx_size.map(|c| c.clamp(512, 262_144));

    next.models_path = next.models_path.trim().to_string();
    next.active_main = next.active_main.trim().to_string();
    next.active_mmproj = next.active_mmproj.trim().to_string();
    next.describe_main = next.describe_main.trim().to_string();
    next.describe_mmproj = next.describe_mmproj.trim().to_string();
    next.summary_main = next.summary_main.trim().to_string();
    next.summary_mmproj = next.summary_mmproj.trim().to_string();

    // prompt_language 限制取值；非法回退到 zh
    next.prompt_language = match next.prompt_language.trim() {
        "tw" => "tw".to_string(),
        "en" => "en".to_string(),
        "ja" => "ja".to_string(),
        "pt" => "pt".to_string(),
        _ => "zh".to_string(),
    };
    // 覆盖文本不 trim 中间空白（用户可能想保留缩进），仅去前后整体空白
    next.prompt_overrides.system_zh = next.prompt_overrides.system_zh.trim().to_string();
    next.prompt_overrides.system_en = next.prompt_overrides.system_en.trim().to_string();
    next.prompt_overrides.system_ja = next.prompt_overrides.system_ja.trim().to_string();
    next.prompt_overrides.system_pt = next.prompt_overrides.system_pt.trim().to_string();
    next.prompt_overrides.system_tw = next.prompt_overrides.system_tw.trim().to_string();
    next.image_describe_overrides.system_zh =
        next.image_describe_overrides.system_zh.trim().to_string();
    next.image_describe_overrides.system_en =
        next.image_describe_overrides.system_en.trim().to_string();
    next.image_describe_overrides.system_ja =
        next.image_describe_overrides.system_ja.trim().to_string();
    next.image_describe_overrides.system_pt =
        next.image_describe_overrides.system_pt.trim().to_string();
    next.image_describe_overrides.system_tw =
        next.image_describe_overrides.system_tw.trim().to_string();

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
