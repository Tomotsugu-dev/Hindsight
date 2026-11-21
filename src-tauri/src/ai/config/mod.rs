//! AI 设置数据结构 + 净化逻辑。
//!
//! - [`AiConfig`] 嵌进 `Settings::ai`，跟着 settings_store JSON 一起持久化
//! - [`AiSegment`] 是一天里被划分出的一个时段，AI 按段汇总
//! - [`sanitize()`] 在 settings 写入路径上调用，把非法值钳到合法范围
//!
//! 子模块：`model_choice` 每个任务用哪个模型，`visibility` AI 能看到哪些分类，
//! `segments` 一天怎么分段，`sanitize` 保存前洗值。

mod model_choice;
mod sanitize;
mod segments;
mod visibility;

use serde::{Deserialize, Serialize};

use model_choice::known_items;

pub use model_choice::{Advanced, ModelRef, Pick, Picks, Task, SUMMARY_CLOUD_SENTINEL};
// 别的模块只有测试按名字用它
#[cfg(test)]
pub use model_choice::ReasoningLevel;
pub use sanitize::sanitize;
pub use segments::{default_segments_for, AiSegment};
pub use visibility::{ModuleProfiles, Visibility, VisibilityProfile};

/// 云端 API 的一组已保存配置(provider/端点/Key/模型)。
/// 只是"预设收藏":应用某组 = 把四个字段拷进 AiConfig 的激活字段,
/// 聊天/总结消费方仍只读激活字段,不感知本列表。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ExternalProfile {
    /// 显示名(前端生成/用户可读),仅用于列表展示
    pub name: String,
    pub provider: String,
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
}

/// AI 子系统的所有用户配置。嵌进 [`crate::repo::settings::Settings::ai`]。
///
/// `#[serde(default)]` 让旧 settings JSON（没有 ai 字段）反序列化时自动填默认值，
/// 不需要 schema migration。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AiConfig {
    /// 外部云端 API 的 OpenAI 兼容 base URL（不含 `/chat/completions` 路径）。
    /// 仅在 `external_enabled = true` 时生效。
    /// 默认空——前端要求用户主动选 provider 才填。
    pub endpoint: String,
    /// 外部 API 的模型 ID（如 `gpt-4o-mini` / `deepseek-chat`）；
    /// 仅在 `external_enabled = true` 时生效。
    pub model: String,
    /// 外部 API 的 Bearer token；明文落 settings JSON。
    pub api_key: String,
    /// 是否启用外部云端 API。
    /// false = 全程本地（默认）。true 时段总结是否走云由 `summary_main` 的
    /// `__cloud__` sentinel 决定（见 [`Self::summary_use_cloud`]）；
    /// Chat 由 [`Self::chat_use_cloud`] 决定。
    pub external_enabled: bool,
    /// Provider 预设标识（"openai" / "deepseek" / "openrouter" / "together" / "groq" / "custom"）。
    /// 后端只用来 sanitize；UI 用它决定 Base URL / Model 的 placeholder。
    pub external_provider: String,
    /// 已保存的云端 API 配置(上限 10;应用/删除见前端 云端 API 页)。
    #[serde(default)]
    pub external_profiles: Vec<ExternalProfile>,
    /// 用户对自己的简短描述，AI 总结时拼进 system prompt
    pub user_brief: String,
    /// 一天的时段划分；UI 上是连续条
    pub segments: Vec<AiSegment>,
    /// The profiles of the daily report, the weekly report and chat: which categories the
    /// AI can see.
    pub visibility: Visibility,
    /// 每个任务用哪个模型。
    pub picks: Picks,
    /// 用户给「任务 + 模型」设的参数。
    #[serde(deserialize_with = "known_items")]
    pub advanced: Vec<Advanced>,
    /// 模型（GGUF 文件）保存路径。
    ///
    /// 空字符串 = 走 [`crate::ai::models::default_root_dir`]（`<data_root>/ai/models/`）；
    /// `repo::settings::load` 会在启动时把空值填成实际默认路径。
    /// 用户在 设置 → 数据 里能改成大硬盘上的目录。
    pub models_path: String,
    /// 当前选中的主权重 GGUF 文件名（在 `models_path` 目录下）。
    /// 空字符串 = 还没选模型；`start_engine` 会拒绝启动，让用户先去选。
    ///
    /// 历史遗留字段，扮演 `summary_main` 的 fallback：为空时降级用本字段。
    /// 读取时统一走 [`Self::effective_summary_main`]，不要直接读。
    pub active_main: String,
    /// 当前选中的 mmproj GGUF 文件名（vision 模型必带）。
    /// 空字符串 = 没有 mmproj（纯文本模型）。
    /// fallback 语义同 [`Self::active_main`]——读取走 effective 方法。
    pub active_mmproj: String,
    /// 段总结用的主权重 GGUF；空 = 降级到 [`Self::active_main`]。
    /// 段总结一般是纯文本任务，可挑更小或纯文本模型节省 VRAM。
    #[serde(default)]
    pub summary_main: String,
    /// step 2 段总结用的 mmproj GGUF；空 = 降级到 [`Self::active_mmproj`]。
    /// 一般纯文本模型这个留空即可。
    #[serde(default)]
    pub summary_mmproj: String,
    /// 对话(Chat)用的模型槽位,独立于段总结:
    /// 空 = 自动(云端三元组配好走云端,否则同 step 2);
    /// `__cloud__` = 明确用云端 API;文件名 = 明确用该本地 GGUF(纯文本,不带 mmproj)。
    /// 读取走 [`Self::chat_use_cloud`] / [`Self::effective_chat_main`]。
    #[serde(default)]
    pub chat_main: String,
    /// 聊天的思考档位，取值是 `auto`、`off`、`low`、`medium`、`high`、`max`。
    /// 日报和周报不用这个设置。各服务怎么发这些档位，见 `ai::providers::chat_completions::dialect`。
    #[serde(default = "default_chat_thinking")]
    pub chat_thinking: String,
    /// AI 总结使用的提示词语言（决定模型出哪种语言的总结 + 默认提示词模板用哪套）。
    /// 取值 "zh" / "tw" / "en" / "ja" / "pt"；非法值 sanitize 时回退到 "zh"。
    pub prompt_language: String,
    /// 用户对内置 system prompt（段总结）的覆盖；按语言分别存。
    /// 某语言对应字段为空 = 用内置默认；非空 = 走覆盖。
    pub prompt_overrides: PromptOverrides,
    /// 引擎启动级参数：`--batch-size` / `--ubatch-size`（取一致值）。
    /// `None` = 不传，走 llama.cpp 默认 512。
    /// 改值会让下次引擎启动用新参数；引擎已在跑时不会自动重启，需用户主动 stop。
    ///
    /// 这三个旧字段（`batch_size` / `parallel_slots` / `ctx_size`）是 fallback——
    /// 当对应的 `summary_*` 字段为 `None` 时降级使用。
    /// 通过 [`Self::summary_batch_size_effective`] 等 getter 取值，调用方不要直接读字段。
    pub batch_size: Option<u32>,
    /// 引擎启动级参数：`-np N` 并行槽位数。
    /// `None` = 1（串行）。详见 [`Self::batch_size`] 关于 fallback 语义。
    pub parallel_slots: Option<u32>,
    /// 引擎启动级参数：每 slot 的 ctx 上限（token）。
    /// 实际 `--ctx-size = ctx_size × parallel_slots`，让每槽都拿到这个 budget。
    /// `None` = 8K 默认。详见 [`Self::batch_size`] 关于 fallback 语义。
    pub ctx_size: Option<u32>,

    /// 自动总结:日/周结束后由后台任务自动补齐日报与周报,
    /// 无需手动点「开始总结」(月报生成器落地后自动纳入)。默认关。
    #[serde(default)]
    pub auto_summary: bool,

    /// 自动总结定时(旧单时刻字段,仅兼容历史配置读取;新写入走
    /// [`Self::auto_summary_times`],前端写 times 时一并置空本字段)。
    #[serde(default)]
    pub auto_summary_at: Option<String>,

    /// 自动总结的时间点列表("HH:MM",保持添加顺序,上限 6)。语义:
    /// **每个时间点都生成/刷新当天日报**(后点覆盖前点,如 12:00 半天版、
    /// 23:00 全天版);已生成时刻 ≥ 时间点视为该点完成(重启不重复)。
    /// 空 + 旧字段也空 = 尽快模式(只补前一天,旧行为)。
    #[serde(default)]
    pub auto_summary_times: Vec<String>,

    /// 段总结阶段的 batch 参数；`None` = fallback 到 [`Self::batch_size`]。
    pub summary_batch_size: Option<u32>,
    /// 段总结阶段的 `-np`；`None` = fallback 到 [`Self::parallel_slots`]。
    /// 段总结无并行需求，推荐恒为 1，给 ctx 让出预算。
    pub summary_parallel_slots: Option<u32>,
    /// 段总结阶段的每槽 ctx；`None` = fallback 到 [`Self::ctx_size`]。
    pub summary_ctx_size: Option<u32>,
}

impl AiConfig {
    /// 取段总结阶段的 batch；新字段优先，未设则 fallback 到全局 `batch_size`。
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
    /// 自动总结的**有效**时间点:新列表优先,空则回落旧单时刻字段;
    /// 两者都空 = 尽快模式(调用方据 is_empty 判定)。
    pub fn effective_auto_summary_times(&self) -> Vec<String> {
        if !self.auto_summary_times.is_empty() {
            return self.auto_summary_times.clone();
        }
        self.auto_summary_at.clone().into_iter().collect()
    }

    /// chat 引擎的每槽 ctx:全局 `ctx_size` 兜底,但不低于 8192——
    /// 旧配置里的小 ctx(为多槽段总结调的 2048/4096)对单槽聊天会立刻溢出:
    /// 工具结果预算(chat/engine.rs TOOL_RESULT_MAX_CHARS=4000 字符/条)按 ~8K 窗口设计。
    /// `None` = 维持引擎默认([`crate::ai::server::DEFAULT_CTX_SIZE`],同为 8K)。
    pub fn chat_ctx_size_effective(&self) -> Option<u32> {
        self.ctx_size.map(|c| c.max(8192))
    }

    /// 段总结单次响应 max_tokens：取 effective ctx 的一半（给 prompt 留另一半），
    /// 下界 2048（给 reasoning 模型思考链最低保障）。
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
    /// Spanish system prompt override. `#[serde(default)]` so settings written by
    /// versions before Spanish shipped still deserialize (missing field → empty).
    #[serde(default)]
    pub system_es: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        let lang = crate::platform::system_language();
        Self {
            endpoint: String::new(),
            model: String::new(),
            api_key: String::new(),
            external_enabled: false,
            external_provider: "openai".to_string(),
            external_profiles: Vec::new(),
            user_brief: String::new(),
            segments: default_segments_for(lang),
            visibility: Visibility::default(),
            picks: Picks::default(),
            advanced: Vec::new(),
            models_path: String::new(),
            active_main: String::new(),
            active_mmproj: String::new(),
            summary_main: String::new(),
            summary_mmproj: String::new(),
            chat_main: String::new(),
            chat_thinking: default_chat_thinking(),
            auto_summary: false,
            auto_summary_at: None,
            auto_summary_times: Vec::new(),
            prompt_language: lang.to_string(),
            prompt_overrides: PromptOverrides::default(),
            batch_size: None,
            parallel_slots: None,
            ctx_size: None,
            summary_batch_size: None,
            summary_parallel_slots: None,
            summary_ctx_size: None,
        }
    }
}

fn default_chat_thinking() -> String {
    "auto".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 合法 prompt 语言全集。测试里独立列一份，不引用 sanitize 内部的 match——
    /// 若产品代码误删某语言，这里会红。
    pub(super) const VALID_LANGS: [&str; 6] = ["zh", "tw", "en", "ja", "pt", "es"];

    /// 造一个"干净"的基准配置。基于 Default，但把 prompt_language 固定成 "zh"，
    /// 避免 Default 里 system_language 随宿主 locale 变化导致断言不稳定。
    pub(super) fn base() -> AiConfig {
        AiConfig {
            prompt_language: "zh".to_string(),
            ..AiConfig::default()
        }
    }

    /// 序列化字段名是 camelCase 的 autoSummaryAt(前端契约)。
    #[test]
    fn auto_summary_at_serializes_camel_case() {
        let cfg = AiConfig {
            auto_summary_at: Some("03:00".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("\"autoSummaryAt\":\"03:00\""), "{json}");
    }

    // ---------- system_language ----------

    #[test]
    fn system_language_returns_valid_tag_stable_under_sanitize() {
        // locale 来源是 OS 全局、无注入点，分支覆盖不可行；
        // 这里只锁输出契约：返回值必须落在合法语言集内，
        // 且作为 prompt_language 回灌 sanitize 不会被改写（否则首启配置会被静默篡改）。
        let lang = crate::platform::system_language();
        assert!(
            VALID_LANGS.contains(&lang),
            "system_language 返回了名单外的值: {lang:?}"
        );
        let mut next = base();
        next.prompt_language = lang.to_string();
        let out = sanitize(next, &base());
        assert_eq!(out.prompt_language, lang, "探测结果必须能原样通过 sanitize");
    }

    // ---------- AiConfig::default + serde 兼容 ----------

    #[test]
    fn default_config_key_values() {
        let d = AiConfig::default();
        // 隐私底线：默认必须全本地、云端三元组全空
        assert!(!d.external_enabled);
        assert!(d.endpoint.is_empty() && d.model.is_empty() && d.api_key.is_empty());
        assert_eq!(d.external_provider, "openai");
        assert_eq!(d.segments.len(), 5);
        assert!(VALID_LANGS.contains(&d.prompt_language.as_str()));
        assert!(!d.auto_summary);
        // 引擎参数默认全 None = 跟随 llama.cpp 默认
        assert!(d.batch_size.is_none() && d.parallel_slots.is_none() && d.ctx_size.is_none());
        assert!(
            d.summary_batch_size.is_none()
                && d.summary_parallel_slots.is_none()
                && d.summary_ctx_size.is_none()
        );
        // 模型槽位默认全空 = 未选择
        assert!(d.active_main.is_empty() && d.summary_main.is_empty() && d.chat_main.is_empty());
        assert!(d.models_path.is_empty());
    }

    #[test]
    fn serde_empty_object_yields_defaults() {
        // 旧版本 settings JSON 里没有 ai 字段 → 反序列化 {} 必须等价于 Default，
        // 这是"无 schema migration"承诺的根基
        let parsed: AiConfig = serde_json::from_str("{}").expect("{} 应能反序列化");
        let d = AiConfig::default();
        assert_eq!(parsed.external_provider, d.external_provider);
        assert_eq!(parsed.visibility, d.visibility);
        assert_eq!(parsed.segments.len(), d.segments.len());
        assert_eq!(parsed.external_enabled, d.external_enabled);
        assert_eq!(parsed.batch_size, None);
        assert_eq!(parsed.summary_ctx_size, None);
        assert!(parsed.summary_main.is_empty() && parsed.chat_main.is_empty());
    }

    #[test]
    fn old_excluded_categories_is_ignored() {
        // 旧版本写过 excludedCategories：照样读得出来，配置按默认，写回时这个字段就没了
        let parsed: AiConfig =
            serde_json::from_str(r#"{"excludedCategories":["other","games"]}"#).unwrap();
        assert_eq!(parsed.visibility, Visibility::default());
        let json = serde_json::to_string(&parsed).unwrap();
        assert!(!json.contains("excludedCategories"), "{json}");
    }

    #[test]
    fn serde_uses_camel_case_keys_both_ways() {
        // 前端 TS 侧按 camelCase 读写；键名变化会静默丢字段（serde default 兜底掩盖问题）
        let json = serde_json::to_string(&AiConfig::default()).expect("序列化不应失败");
        for key in [
            "\"externalProvider\"",
            "\"promptLanguage\"",
            "\"autoProfile\"",
            "\"activeMain\"",
            "\"promptOverrides\"",
            "\"systemZh\"",
        ] {
            assert!(json.contains(key), "序列化输出应含 {key}，实际: {json}");
        }
        assert!(
            !json.contains("\"external_provider\""),
            "不应出现 snake_case 键"
        );

        let parsed: AiConfig = serde_json::from_str(
            r##"{
                "externalEnabled": true,
                "externalProvider": "deepseek",
                "promptLanguage": "ja",
                "activeMain": "m.gguf",
                "batchSize": 256,
                "segments": [{"label": "x", "startHour": 1, "endHour": 2, "color": "#aabbcc"}],
                "promptOverrides": {"systemJa": "覆盖"}
            }"##,
        )
        .expect("camelCase JSON 应能反序列化");
        assert!(parsed.external_enabled);
        assert_eq!(parsed.external_provider, "deepseek");
        assert_eq!(parsed.prompt_language, "ja");
        assert_eq!(parsed.active_main, "m.gguf");
        assert_eq!(parsed.batch_size, Some(256));
        assert_eq!(parsed.segments.len(), 1);
        assert_eq!(parsed.segments[0].start_hour, 1);
        assert_eq!(parsed.prompt_overrides.system_ja, "覆盖");
    }

    #[test]
    fn serde_partial_json_keeps_defaults_for_missing_fields() {
        // 后加的字段（summaryMain / chatMain / autoSummary 等）在老 JSON 里缺失，
        // 必须落回默认而不是报错——向后兼容的关键路径
        let parsed: AiConfig = serde_json::from_str(r#"{"model": "gpt-4o-mini", "ctxSize": 4096}"#)
            .expect("部分字段 JSON 应能反序列化");
        assert_eq!(parsed.model, "gpt-4o-mini");
        assert_eq!(parsed.ctx_size, Some(4096));
        assert_eq!(parsed.external_provider, "openai", "缺失字段保默认");
        assert!(parsed.summary_main.is_empty());
        assert!(parsed.chat_main.is_empty());
        assert!(!parsed.auto_summary);
        assert!(
            parsed.auto_summary_at.is_none(),
            "旧配置无 autoSummaryAt 字段应回落 None(尽快模式)"
        );
        assert_eq!(parsed.summary_batch_size, None);
    }

    // ---------- 引擎参数 ----------

    #[test]
    fn summary_engine_params_prefer_new_fields_over_legacy() {
        // 新 summary_* 字段优先；None 时降级旧全局字段；双 None 保持 None
        let mut c = base();
        c.batch_size = Some(128);
        c.parallel_slots = Some(2);
        c.ctx_size = Some(4096);
        c.summary_batch_size = Some(64);
        c.summary_parallel_slots = Some(1);
        c.summary_ctx_size = Some(16_384);
        assert_eq!(c.summary_batch_size_effective(), Some(64));
        assert_eq!(c.summary_parallel_slots_effective(), Some(1));
        assert_eq!(c.summary_ctx_size_effective(), Some(16_384));

        c.summary_batch_size = None;
        c.summary_parallel_slots = None;
        c.summary_ctx_size = None;
        assert_eq!(
            c.summary_batch_size_effective(),
            Some(128),
            "fallback 到旧字段"
        );
        assert_eq!(c.summary_parallel_slots_effective(), Some(2));
        assert_eq!(c.summary_ctx_size_effective(), Some(4096));

        let d = base();
        assert_eq!(d.summary_batch_size_effective(), None, "双 None 保持 None");
        assert_eq!(d.summary_parallel_slots_effective(), None);
        assert_eq!(d.summary_ctx_size_effective(), None);
    }

    #[test]
    fn summary_max_tokens_half_ctx_with_floor() {
        // 期望值独立推导：默认 ctx 8192 → 一半 4096；
        // 32768 → 16384；3000 → 1500 低于 2048 下界 → 2048（reasoning 思考链最低保障）
        let mut c = base();
        assert_eq!(c.summary_max_tokens(), 4096, "未设 ctx 按默认 8192 的一半");
        c.summary_ctx_size = Some(32_768);
        assert_eq!(c.summary_max_tokens(), 16_384);
        c.summary_ctx_size = Some(3_000);
        assert_eq!(c.summary_max_tokens(), 2048, "一半不足 2048 时钳到下界");
        // summary_ctx 未设时 fallback 用全局 ctx_size
        c.summary_ctx_size = None;
        c.ctx_size = Some(16_384);
        assert_eq!(c.summary_max_tokens(), 8192, "fallback 全局 ctx 的一半");
    }
}
