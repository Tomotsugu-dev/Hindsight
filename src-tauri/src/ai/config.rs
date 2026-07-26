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
    /// 用户对自己的简短描述，AI 总结时拼进 system prompt
    pub user_brief: String,
    /// 一天的时段划分；UI 上是连续条
    pub segments: Vec<AiSegment>,
    /// 排除分析的 category id 列表
    pub excluded_categories: Vec<String>,
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

    /// 段总结主权重文件名；空 → fallback 到 `active_main`。
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
    /// 段总结 mmproj 文件名。**mmproj 跟 main 配套 fallback**：`summary_main`
    /// 显式设了，mmproj 就只看 `summary_mmproj`（空 = 纯文本模型）；
    /// `summary_main` 为空或 sentinel 时才 fallback 到 `active_mmproj`——
    /// 否则会把 vision mmproj 强加到文本模型上，token embedding 错位首 token 即 EOS。
    pub fn effective_summary_mmproj(&self) -> &str {
        let s = self.summary_main.trim();
        if s.is_empty() || s == SUMMARY_CLOUD_SENTINEL {
            self.active_mmproj.as_str()
        } else {
            self.summary_mmproj.as_str()
        }
    }

    /// 段总结是否实际路由到云端：要 `summary_main` 是"用云端"标记 **且** `external_enabled=true`。
    /// `external_enabled=false` 时 sentinel 退化为 fallback（按 `active_main` 跑本地），
    /// 不会因为 sentinel 残留就硬卡住没法跑总结。
    pub fn summary_use_cloud(&self) -> bool {
        self.external_enabled && self.summary_main.trim() == SUMMARY_CLOUD_SENTINEL
    }

    /// 云端三元组是否配齐(Chat 路由用:enabled + endpoint + model 缺一不可)。
    pub fn chat_cloud_ready(&self) -> bool {
        self.external_enabled && !self.endpoint.trim().is_empty() && !self.model.trim().is_empty()
    }

    /// Chat 是否路由到云端:`chat_main` 显式 sentinel 或未设置(自动)时,
    /// 云端三元组配齐即走云端;显式本地文件名则永远本地。
    /// sentinel 但云端没配齐 → 退化为本地 fallback,不硬卡(同 step 1/2 语义)。
    pub fn chat_use_cloud(&self) -> bool {
        let c = self.chat_main.trim();
        if !c.is_empty() && c != SUMMARY_CLOUD_SENTINEL {
            return false;
        }
        self.chat_cloud_ready()
    }

    /// Chat 本地路径实际加载的主权重:显式文件名优先,否则同 step 2 的 fallback 链。
    pub fn effective_chat_main(&self) -> &str {
        let c = self.chat_main.trim();
        if c.is_empty() || c == SUMMARY_CLOUD_SENTINEL {
            self.effective_summary_main()
        } else {
            self.chat_main.as_str()
        }
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
}

impl Default for AiConfig {
    fn default() -> Self {
        let lang = detect_default_lang();
        Self {
            endpoint: String::new(),
            model: String::new(),
            api_key: String::new(),
            external_enabled: false,
            external_provider: "openai".to_string(),
            user_brief: String::new(),
            segments: default_segments_for(lang),
            excluded_categories: vec!["other".to_string()],
            models_path: String::new(),
            active_main: String::new(),
            active_mmproj: String::new(),
            summary_main: String::new(),
            summary_mmproj: String::new(),
            chat_main: String::new(),
            auto_summary: false,
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

    // 引擎启动级参数 clamp（跟 AiOverrides 一致）：
    // batch ≥ 32 是 llama-server 不接受过小值的安全下限
    // ctx 上限给 256K（极限场景，超出基本任何卡都装不下，没必要再大）
    // parallel_slots ≥ 1，给 32 上限避免误填出格的值
    next.batch_size = next.batch_size.map(|b| b.clamp(32, 32_768));
    next.parallel_slots = next.parallel_slots.map(|n| n.clamp(1, 32));
    next.ctx_size = next.ctx_size.map(|c| c.clamp(512, 262_144));
    next.summary_batch_size = next.summary_batch_size.map(|b| b.clamp(32, 32_768));
    next.summary_parallel_slots = next.summary_parallel_slots.map(|n| n.clamp(1, 32));
    next.summary_ctx_size = next.summary_ctx_size.map(|c| c.clamp(512, 262_144));

    next.models_path = next.models_path.trim().to_string();
    next.active_main = next.active_main.trim().to_string();
    next.active_mmproj = next.active_mmproj.trim().to_string();
    next.summary_main = next.summary_main.trim().to_string();
    next.summary_mmproj = next.summary_mmproj.trim().to_string();
    next.chat_main = next.chat_main.trim().to_string();

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

#[cfg(test)]
mod tests {
    use super::*;

    /// 合法 prompt 语言全集。测试里独立列一份，不引用 sanitize 内部的 match——
    /// 若产品代码误删某语言，这里会红。
    const VALID_LANGS: [&str; 5] = ["zh", "tw", "en", "ja", "pt"];

    /// 造一个"干净"的基准配置。基于 Default，但把 prompt_language 固定成 "zh"，
    /// 避免 Default 里 detect_default_lang 随宿主 locale 变化导致断言不稳定。
    fn base() -> AiConfig {
        AiConfig {
            prompt_language: "zh".to_string(),
            ..AiConfig::default()
        }
    }

    fn seg(label: &str, start: u8, end: u8, color: &str) -> AiSegment {
        AiSegment {
            label: label.to_string(),
            start_hour: start,
            end_hour: end,
            color: color.to_string(),
        }
    }

    // ---------- sanitize ----------

    #[test]
    fn sanitize_prompt_language_whitelist_and_fallback() {
        // 白名单内的值（含带空白）原样保留；名单外一律回退 zh——
        // prompt 模板按语言取套，落一个未知 tag 会让取模板处 panic 或拿错语言。
        for lang in VALID_LANGS {
            let mut next = base();
            next.prompt_language = format!("  {lang}  ");
            let out = sanitize(next, &base());
            assert_eq!(out.prompt_language, lang, "合法语言应 trim 后保留");
        }
        for bad in ["fr", "zh-CN", "EN", "", "  ", "日本語"] {
            let mut next = base();
            next.prompt_language = bad.to_string();
            let out = sanitize(next, &base());
            assert_eq!(out.prompt_language, "zh", "非法语言 {bad:?} 应回退 zh");
        }
    }

    #[test]
    fn sanitize_external_provider_whitelist_and_fallback() {
        // provider 只影响 UI placeholder，但落库前仍要归一，避免前端拿到未知值渲染错乱
        for p in [
            "openai",
            "deepseek",
            "openrouter",
            "together",
            "groq",
            "custom",
        ] {
            let mut next = base();
            next.external_provider = format!(" {p} ");
            let out = sanitize(next, &base());
            assert_eq!(out.external_provider, p);
        }
        for bad in ["azure", "OpenAI", "", "gpt"] {
            let mut next = base();
            next.external_provider = bad.to_string();
            let out = sanitize(next, &base());
            assert_eq!(
                out.external_provider, "openai",
                "非法 provider {bad:?} 应回退 openai"
            );
        }
    }

    #[test]
    fn sanitize_trims_all_string_fields() {
        // 这些字段最终会拼 URL / 文件路径 / header，首尾空白会造成难查的 404 或找不到文件
        let mut next = base();
        next.endpoint = "  https://api.example.com/v1  ".to_string();
        next.model = "\tgpt-4o-mini\n".to_string();
        next.api_key = " sk-abc ".to_string();
        next.user_brief = "  程序员  ".to_string();
        next.models_path = " /data/models ".to_string();
        next.active_main = " main.gguf ".to_string();
        next.active_mmproj = " mmproj.gguf ".to_string();
        next.summary_main = " sum.gguf ".to_string();
        next.summary_mmproj = " sum-mm.gguf ".to_string();
        next.chat_main = " chat.gguf ".to_string();
        let out = sanitize(next, &base());
        assert_eq!(out.endpoint, "https://api.example.com/v1");
        assert_eq!(out.model, "gpt-4o-mini");
        assert_eq!(out.api_key, "sk-abc");
        assert_eq!(out.user_brief, "程序员");
        assert_eq!(out.models_path, "/data/models");
        assert_eq!(out.active_main, "main.gguf");
        assert_eq!(out.active_mmproj, "mmproj.gguf");
        assert_eq!(out.summary_main, "sum.gguf");
        assert_eq!(out.summary_mmproj, "sum-mm.gguf");
        assert_eq!(out.chat_main, "chat.gguf");
    }

    #[test]
    fn sanitize_prompt_overrides_trim_edges_keep_inner_indent() {
        // 用户自定义 prompt 里的内部缩进/换行是有意排版，只能去首尾整体空白；
        // 纯空白视为"没写覆盖"归空，否则会以空白 prompt 顶掉内置默认。
        let text = "\n  第一行\n    缩进第二行\n";
        let mut next = base();
        next.prompt_overrides.system_zh = text.to_string();
        next.prompt_overrides.system_en = format!("  {text}  ");
        next.prompt_overrides.system_ja = "   \n\t  ".to_string(); // 纯空白 → 空
        next.prompt_overrides.system_pt = "single".to_string();
        next.prompt_overrides.system_tw = "\t保留\t中间\ttab\t".to_string();
        let out = sanitize(next, &base());
        assert_eq!(out.prompt_overrides.system_zh, "第一行\n    缩进第二行");
        assert_eq!(out.prompt_overrides.system_en, "第一行\n    缩进第二行");
        assert_eq!(out.prompt_overrides.system_ja, "");
        assert_eq!(out.prompt_overrides.system_pt, "single");
        assert_eq!(out.prompt_overrides.system_tw, "保留\t中间\ttab");
    }

    #[test]
    fn sanitize_segments_filters_invalid_trims_label_normalizes_color() {
        // 非法时段（start>=end、end>24）静默丢弃而非整体拒绝——
        // 让用户拖拽编辑中间态提交时不至于全盘失败
        let mut next = base();
        next.segments = vec![
            seg("  早上  ", 6, 9, " #ABC "), // 合法：label 去空白、颜色归一
            seg("倒置", 9, 6, ""),           // start > end → 丢
            seg("零长", 12, 12, ""),         // start == end → 丢
            seg("越界", 20, 25, ""),         // end > 24 → 丢
            seg("晚上", 18, 24, "#ff8800"),  // 合法：end==24 是边界允许值
        ];
        let out = sanitize(next, &base());
        assert_eq!(out.segments.len(), 2, "只留两条合法段");
        assert_eq!(out.segments[0].label, "早上");
        assert_eq!(out.segments[0].color, "#aabbcc");
        assert_eq!(out.segments[1].label, "晚上");
        assert_eq!(out.segments[1].color, "#ff8800");
    }

    #[test]
    fn sanitize_segments_all_invalid_falls_back_to_old() {
        // 全部段都非法时若接受空组，AI 总结将无段可跑；兜底回退旧值
        let mut old = base();
        old.segments = vec![seg("旧段", 0, 24, "")];
        let mut next = base();
        next.segments = vec![seg("坏", 10, 5, ""), seg("坏2", 3, 30, "")];
        let out = sanitize(next, &old);
        assert_eq!(out.segments.len(), 1);
        assert_eq!(out.segments[0].label, "旧段");
        assert_eq!(
            (out.segments[0].start_hour, out.segments[0].end_hour),
            (0, 24)
        );
    }

    #[test]
    fn sanitize_excluded_categories_trim_and_drop_empty() {
        // 空串 category id 匹配不到任何分类，留着只会污染 UI 列表
        let mut next = base();
        next.excluded_categories = vec![
            "  work  ".to_string(),
            "".to_string(),
            "   ".to_string(),
            "games".to_string(),
        ];
        let out = sanitize(next, &base());
        assert_eq!(
            out.excluded_categories,
            vec!["work".to_string(), "games".to_string()]
        );
    }

    #[test]
    fn sanitize_engine_params_clamped_to_documented_ranges() {
        // 期望范围独立取自文档约定：batch 32~32768、slots 1~32、ctx 512~262144。
        // 越界值钳到边界而不是拒绝——保证引擎启动参数永远可用。
        let mut next = base();
        next.batch_size = Some(1);
        next.parallel_slots = Some(0);
        next.ctx_size = Some(100);
        next.summary_batch_size = Some(1_000_000);
        next.summary_parallel_slots = Some(999);
        next.summary_ctx_size = Some(u32::MAX);
        let out = sanitize(next, &base());
        assert_eq!(out.batch_size, Some(32), "batch 下界 32");
        assert_eq!(out.parallel_slots, Some(1), "slots 下界 1");
        assert_eq!(out.ctx_size, Some(512), "ctx 下界 512");
        assert_eq!(out.summary_batch_size, Some(32_768), "batch 上界 32768");
        assert_eq!(out.summary_parallel_slots, Some(32), "slots 上界 32");
        assert_eq!(out.summary_ctx_size, Some(262_144), "ctx 上界 262144");
    }

    #[test]
    fn sanitize_engine_params_none_stays_none_and_valid_untouched() {
        // None 语义是"走 llama.cpp 默认"，clamp 不得把它实体化成数字
        let none_case = sanitize(base(), &base());
        assert_eq!(none_case.batch_size, None);
        assert_eq!(none_case.parallel_slots, None);
        assert_eq!(none_case.ctx_size, None);
        assert_eq!(none_case.summary_batch_size, None);
        assert_eq!(none_case.summary_parallel_slots, None);
        assert_eq!(none_case.summary_ctx_size, None);

        // 合法值（含正好在边界上的）不应被改动
        let mut next = base();
        next.batch_size = Some(512);
        next.parallel_slots = Some(4);
        next.ctx_size = Some(8192);
        next.summary_batch_size = Some(32); // 恰在下界
        next.summary_parallel_slots = Some(32); // 恰在上界
        next.summary_ctx_size = Some(262_144); // 恰在上界
        let out = sanitize(next, &base());
        assert_eq!(out.batch_size, Some(512));
        assert_eq!(out.parallel_slots, Some(4));
        assert_eq!(out.ctx_size, Some(8192));
        assert_eq!(out.summary_batch_size, Some(32));
        assert_eq!(out.summary_parallel_slots, Some(32));
        assert_eq!(out.summary_ctx_size, Some(262_144));
    }

    // ---------- sanitize_hex_color（经 segments 间接进 sanitize，这里直测私有函数） ----------

    #[test]
    fn hex_color_valid_forms_normalized() {
        // 统一输出小写 #rrggbb，UI 端比较颜色时不用再做大小写/短格式兼容
        assert_eq!(sanitize_hex_color("#abc"), "#aabbcc", "#rgb 短格式按位展开");
        assert_eq!(sanitize_hex_color("#AbC"), "#aabbcc", "短格式大写归小写");
        assert_eq!(sanitize_hex_color("#FF8800"), "#ff8800", "长格式大写归小写");
        assert_eq!(sanitize_hex_color("  #1a2b3c  "), "#1a2b3c", "trim 后合法");
        assert_eq!(sanitize_hex_color(""), "", "空串保持空（= UI 自动渐变）");
    }

    #[test]
    fn hex_color_invalid_forms_cleared() {
        // 非法颜色置空而不是保留原文——空有明确定义（自动渐变），脏值会让 CSS 静默失效
        assert_eq!(sanitize_hex_color("abc123"), "", "缺 # 前缀");
        assert_eq!(sanitize_hex_color("#abcd"), "", "长度 4 非法");
        assert_eq!(sanitize_hex_color("#ab"), "", "长度 2 非法");
        assert_eq!(sanitize_hex_color("#abcdefa"), "", "长度 7 非法");
        assert_eq!(sanitize_hex_color("#ggg"), "", "非 hex 字符");
        assert_eq!(sanitize_hex_color("#12345z"), "", "长格式混入非 hex");
        assert_eq!(sanitize_hex_color("   "), "", "纯空白 trim 后为空");
    }

    // ---------- default_segments_for ----------

    #[test]
    fn default_segments_cover_full_day_seamlessly_for_every_lang() {
        // 时段是"全天覆盖、无缝衔接"的约定：首段起点 0、末段终点 24、相邻段首尾相接。
        // 未知语言与空串也必须给出可用组——首启 locale 探测失败不能让默认配置残缺。
        for lang in ["zh", "tw", "en", "ja", "pt", "ko", ""] {
            let segs = default_segments_for(lang);
            assert_eq!(segs.len(), 5, "lang={lang:?} 应为 5 段");
            assert_eq!(segs[0].start_hour, 0, "lang={lang:?} 首段从 0 点起");
            assert_eq!(segs[4].end_hour, 24, "lang={lang:?} 末段到 24 点止");
            for w in segs.windows(2) {
                assert!(
                    w[0].start_hour < w[0].end_hour,
                    "lang={lang:?} 段内 start<end"
                );
                assert_eq!(w[0].end_hour, w[1].start_hour, "lang={lang:?} 相邻段无缝");
            }
            // 边界值是产品约定的固定五段划分
            let bounds: Vec<(u8, u8)> = segs.iter().map(|s| (s.start_hour, s.end_hour)).collect();
            assert_eq!(bounds, vec![(0, 6), (6, 9), (9, 12), (12, 18), (18, 24)]);
            // 默认色留空 = UI 自动渐变
            assert!(
                segs.iter().all(|s| s.color.is_empty()),
                "lang={lang:?} 默认不带色"
            );
        }
    }

    #[test]
    fn default_segments_labels_localized_with_zh_fallback() {
        let zh: Vec<String> = default_segments_for("zh")
            .iter()
            .map(|s| s.label.clone())
            .collect();
        let en: Vec<String> = default_segments_for("en")
            .iter()
            .map(|s| s.label.clone())
            .collect();
        let ja: Vec<String> = default_segments_for("ja")
            .iter()
            .map(|s| s.label.clone())
            .collect();
        let pt: Vec<String> = default_segments_for("pt")
            .iter()
            .map(|s| s.label.clone())
            .collect();
        // 各语言组彼此不同（真的做了本地化，而不是同一套复制）
        assert_ne!(zh, en);
        assert_ne!(zh, ja);
        assert_ne!(en, pt);
        // 抽查产品文案锚点：英文组是英文、中文组是中文
        assert_eq!(en[0], "Late Night");
        assert_eq!(zh[4], "晚上");
        assert!(en.iter().all(|l| l.is_ascii()), "英文标签应为纯 ASCII");
        // tw 尚无独立文案、未知语言亦然——都回退中文组，保证标签永不为空
        let tw: Vec<String> = default_segments_for("tw")
            .iter()
            .map(|s| s.label.clone())
            .collect();
        let ko: Vec<String> = default_segments_for("ko")
            .iter()
            .map(|s| s.label.clone())
            .collect();
        assert_eq!(tw, zh, "tw 回退中文组");
        assert_eq!(ko, zh, "未知语言回退中文组");
        assert!(zh.iter().all(|l| !l.is_empty()));
    }

    // ---------- detect_default_lang ----------

    #[test]
    fn detect_default_lang_returns_valid_tag_stable_under_sanitize() {
        // locale 来源是 OS 全局、无注入点，分支覆盖不可行；
        // 这里只锁输出契约：返回值必须落在合法语言集内，
        // 且作为 prompt_language 回灌 sanitize 不会被改写（否则首启配置会被静默篡改）。
        let lang = detect_default_lang();
        assert!(
            VALID_LANGS.contains(&lang),
            "detect_default_lang 返回了名单外的值: {lang:?}"
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
        // "other" 分类默认排除——杂项噪声不进 AI 分析
        assert_eq!(d.excluded_categories, vec!["other".to_string()]);
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
        assert_eq!(parsed.excluded_categories, d.excluded_categories);
        assert_eq!(parsed.segments.len(), d.segments.len());
        assert_eq!(parsed.external_enabled, d.external_enabled);
        assert_eq!(parsed.batch_size, None);
        assert_eq!(parsed.summary_ctx_size, None);
        assert!(parsed.summary_main.is_empty() && parsed.chat_main.is_empty());
    }

    #[test]
    fn serde_uses_camel_case_keys_both_ways() {
        // 前端 TS 侧按 camelCase 读写；键名变化会静默丢字段（serde default 兜底掩盖问题）
        let json = serde_json::to_string(&AiConfig::default()).expect("序列化不应失败");
        for key in [
            "\"externalProvider\"",
            "\"promptLanguage\"",
            "\"excludedCategories\"",
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
        assert_eq!(parsed.summary_batch_size, None);
    }

    // ---------- effective_* / 云端路由 ----------

    #[test]
    fn effective_summary_main_fallback_chain() {
        // 空 / sentinel → active_main；显式文件名 → 用它。
        // sentinel 走 fallback 是刻意的：本地路径代码（VRAM 估算等）不能把 "__cloud__" 当文件名
        let mut c = base();
        c.active_main = "active.gguf".to_string();
        c.summary_main = String::new();
        assert_eq!(c.effective_summary_main(), "active.gguf", "空 → fallback");
        c.summary_main = SUMMARY_CLOUD_SENTINEL.to_string();
        assert_eq!(
            c.effective_summary_main(),
            "active.gguf",
            "sentinel → fallback"
        );
        c.summary_main = "  __cloud__  ".to_string();
        assert_eq!(
            c.effective_summary_main(),
            "active.gguf",
            "带空白 sentinel 也识别"
        );
        c.summary_main = "small.gguf".to_string();
        assert_eq!(c.effective_summary_main(), "small.gguf", "显式文件名优先");
    }

    #[test]
    fn effective_summary_mmproj_pairs_with_main() {
        // mmproj 跟 main 配套：summary_main 显式设置时绝不 fallback 到 active_mmproj——
        // 把 vision mmproj 强加给文本模型会导致 embedding 错位（首 token 即 EOS）
        let mut c = base();
        c.active_mmproj = "vision-mm.gguf".to_string();
        c.summary_main = String::new();
        assert_eq!(
            c.effective_summary_mmproj(),
            "vision-mm.gguf",
            "main 空 → 跟随 active 配套"
        );
        c.summary_main = SUMMARY_CLOUD_SENTINEL.to_string();
        assert_eq!(
            c.effective_summary_mmproj(),
            "vision-mm.gguf",
            "sentinel 同 fallback 组"
        );
        c.summary_main = "text-model.gguf".to_string();
        c.summary_mmproj = String::new();
        assert_eq!(
            c.effective_summary_mmproj(),
            "",
            "main 显式且 mmproj 空 = 纯文本，不得借用 active"
        );
        c.summary_mmproj = "sum-mm.gguf".to_string();
        assert_eq!(
            c.effective_summary_mmproj(),
            "sum-mm.gguf",
            "main 显式 → 只看 summary_mmproj"
        );
    }

    #[test]
    fn summary_use_cloud_requires_sentinel_and_enabled() {
        // 双条件缺一不可：只 sentinel 不 enabled → 本地 fallback（不硬卡）；
        // 只 enabled 不 sentinel → 用户没选云端，别偷偷上云
        let mut c = base();
        c.summary_main = SUMMARY_CLOUD_SENTINEL.to_string();
        c.external_enabled = false;
        assert!(
            !c.summary_use_cloud(),
            "external 未启用时 sentinel 退化为本地"
        );
        c.external_enabled = true;
        assert!(c.summary_use_cloud(), "双条件齐 → 云端");
        c.summary_main = "local.gguf".to_string();
        assert!(!c.summary_use_cloud(), "非 sentinel 永不走云");
        c.summary_main = " __cloud__ ".to_string();
        assert!(
            c.summary_use_cloud(),
            "sentinel 带空白也应识别（未 sanitize 的读取路径）"
        );
    }

    #[test]
    fn chat_cloud_ready_needs_all_three() {
        // enabled + endpoint + model 三元组缺一不可；api_key 不在必要集里
        //（部分自建兼容端点无鉴权）
        let ready = |enabled: bool, ep: &str, model: &str| {
            let mut c = base();
            c.external_enabled = enabled;
            c.endpoint = ep.to_string();
            c.model = model.to_string();
            c.chat_cloud_ready()
        };
        assert!(ready(true, "https://api.x.com", "m1"));
        assert!(!ready(false, "https://api.x.com", "m1"), "未启用");
        assert!(!ready(true, "", "m1"), "缺 endpoint");
        assert!(!ready(true, "https://api.x.com", ""), "缺 model");
        assert!(!ready(true, "   ", "m1"), "纯空白 endpoint 等同缺失");
    }

    #[test]
    fn chat_use_cloud_three_states() {
        let mut ready = base();
        ready.external_enabled = true;
        ready.endpoint = "https://api.x.com".to_string();
        ready.model = "m1".to_string();

        // 态 1：显式本地文件名 → 无论云端是否 ready，永远本地
        let mut c = ready.clone();
        c.chat_main = "local.gguf".to_string();
        assert!(!c.chat_use_cloud(), "显式本地永不上云");
        // 态 2：显式 sentinel → ready 才上云，不 ready 退化本地不硬卡
        let mut c = ready.clone();
        c.chat_main = SUMMARY_CLOUD_SENTINEL.to_string();
        assert!(c.chat_use_cloud(), "sentinel + ready → 云端");
        c.model = String::new();
        assert!(!c.chat_use_cloud(), "sentinel 但三元组不齐 → 本地 fallback");
        // 态 3：空 = 自动 → 跟随 ready 状态
        let mut c = ready.clone();
        c.chat_main = String::new();
        assert!(c.chat_use_cloud(), "自动 + ready → 云端");
        c.external_enabled = false;
        assert!(!c.chat_use_cloud(), "自动 + 未配好 → 本地");
    }

    #[test]
    fn effective_chat_main_passthrough_chain() {
        // 显式文件名 → 用它；空/sentinel → 穿透到 step 2 的链（summary_main → active_main）
        let mut c = base();
        c.active_main = "active.gguf".to_string();
        c.summary_main = "sum.gguf".to_string();
        c.chat_main = "chat.gguf".to_string();
        assert_eq!(c.effective_chat_main(), "chat.gguf", "显式优先");
        c.chat_main = String::new();
        assert_eq!(
            c.effective_chat_main(),
            "sum.gguf",
            "空 → 穿透到 summary_main"
        );
        c.chat_main = SUMMARY_CLOUD_SENTINEL.to_string();
        assert_eq!(c.effective_chat_main(), "sum.gguf", "sentinel → 同空处理");
        c.summary_main = String::new();
        assert_eq!(
            c.effective_chat_main(),
            "active.gguf",
            "两级皆空 → 落到 active_main"
        );
    }

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
