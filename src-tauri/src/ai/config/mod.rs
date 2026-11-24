//! AI settings types and sanitization.
//!
//! [`AiConfig`] is embedded in `Settings::ai` and persisted as JSON.
//! [`sanitize()`] normalizes and clamps values before settings are saved.

mod legacy;
mod model_choice;
mod sanitize;
mod segments;
mod visibility;

use serde::{Deserialize, Serialize};

use model_choice::known_items;

pub use legacy::migrate_legacy_models;
#[cfg(test)]
pub use model_choice::ReasoningLevel;
pub use model_choice::{
    Advanced, ModelRef, Pick, Picks, Task, CLOUD_UNAVAILABLE_ERROR, NO_MODEL_ERROR,
};
pub use sanitize::sanitize;
pub use segments::{default_segments_for, DaySegment};
pub use visibility::{Visibility, VisibilityProfile};

/// A saved cloud API connection bound to one model.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct CloudConnection {
    /// Stable identifier referenced by [`ModelRef::Cloud`] and [`Pick::Cloud`].
    pub id: String,
    /// Display name shown in the UI.
    pub name: String,
    /// Provider dialect key (for example, `openai`, `deepseek`, `kimi`, `custom`).
    pub dialect: String,
    /// OpenAI-compatible base URL without `/chat/completions`.
    pub endpoint: String,
    /// Cloud model identifier (for example, `gpt-4o-mini` or `deepseek-chat`).
    pub model: String,
    /// Bearer token stored in plaintext in the settings JSON.
    pub api_key: String,
    /// Optional user-supplied context length in tokens.
    pub context_length: Option<u32>,
}

impl CloudConnection {
    /// Returns whether this connection has the minimum fields required to send requests.
    ///
    /// `api_key` is optional because some self-hosted compatible endpoints require no auth.
    pub fn is_ready(&self) -> bool {
        !self.endpoint.trim().is_empty() && !self.model.trim().is_empty()
    }
}

/// User configuration for the AI subsystem, embedded in [`crate::repo::settings::Settings::ai`].
///
/// `#[serde(default)]` fills missing fields with defaults when loading older settings files.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AiConfig {
    /// Saved cloud API connections referenced by [`Picks`] and [`Advanced`].
    pub cloud: Vec<CloudConnection>,
    /// Model selection per AI task.
    pub picks: Picks,
    /// Per-(task, model) parameter overrides.
    #[serde(deserialize_with = "known_items")]
    pub advanced: Vec<Advanced>,
    /// Directory where local GGUF model files are stored.
    ///
    /// An empty string resolves to [`crate::ai::models::default_root_dir`].
    pub models_path: String,
    /// Short self-description of the user included in summary system prompts.
    pub user_brief: String,
    /// Time segments that divide a day for daily report summarization.
    pub segments: Vec<DaySegment>,
    /// Categories visible to the AI for daily reports, weekly reports, and chat.
    pub visibility: Visibility,
    /// Prompt language tag (`zh`, `tw`, `en`, `ja`, `pt`, or `es`).
    ///
    /// Unknown values fall back to `zh` in [`sanitize()`].
    pub prompt_language: String,
    /// Per-language overrides for the built-in summary system prompt.
    pub prompt_overrides: PromptOverrides,
    /// Whether background jobs automatically generate daily and weekly reports.
    pub auto_summary: bool,
    /// Scheduled daily auto-summary times (`HH:MM`, up to 6, preserving insertion order).
    ///
    /// An empty list means ASAP mode for the previous day.
    pub auto_summary_times: Vec<String>,
}

/// Custom summary system prompts stored per language.
///
/// An empty string uses the built-in default for that language.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct PromptOverrides {
    pub system_zh: String,
    pub system_en: String,
    pub system_ja: String,
    pub system_pt: String,
    pub system_tw: String,
    pub system_es: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        let lang = crate::platform::system_language();
        Self {
            cloud: Vec::new(),
            picks: Picks::default(),
            advanced: Vec::new(),
            models_path: String::new(),
            user_brief: String::new(),
            segments: default_segments_for(lang),
            visibility: Visibility::default(),
            prompt_language: lang.to_string(),
            prompt_overrides: PromptOverrides::default(),
            auto_summary: false,
            auto_summary_times: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Complete set of valid prompt languages. Listed independently from sanitize
    /// so deleting a language in production code fails this test.
    pub(super) const VALID_LANGS: [&str; 6] = ["zh", "tw", "en", "ja", "pt", "es"];

    /// Clean baseline config with a deterministic `prompt_language`.
    pub(super) fn base() -> AiConfig {
        AiConfig {
            prompt_language: "zh".to_string(),
            ..AiConfig::default()
        }
    }

    #[test]
    fn system_language_returns_valid_tag_stable_under_sanitize() {
        let lang = crate::platform::system_language();
        assert!(
            VALID_LANGS.contains(&lang),
            "system_language returned unknown tag: {lang:?}"
        );
        let mut next = base();
        next.prompt_language = lang.to_string();
        let out = sanitize(next, &base());
        assert_eq!(out.prompt_language, lang);
    }

    #[test]
    fn default_config_key_values() {
        let d = AiConfig::default();
        assert!(d.cloud.is_empty());
        assert_eq!(d.picks, Picks::default());
        assert!(d.advanced.is_empty());
        assert_eq!(d.segments.len(), 5);
        assert!(VALID_LANGS.contains(&d.prompt_language.as_str()));
        assert!(!d.auto_summary);
        assert!(d.auto_summary_times.is_empty());
        assert!(d.models_path.is_empty());
    }

    #[test]
    fn serde_empty_object_yields_defaults() {
        let parsed: AiConfig = serde_json::from_str("{}").expect("empty object should deserialize");
        let d = AiConfig::default();
        assert!(parsed.cloud.is_empty());
        assert_eq!(parsed.picks, d.picks);
        assert!(parsed.advanced.is_empty());
        assert_eq!(parsed.visibility, d.visibility);
        assert_eq!(parsed.segments.len(), d.segments.len());
        assert_eq!(parsed.auto_summary, d.auto_summary);
    }

    #[test]
    fn legacy_fields_are_ignored_on_deserialize() {
        let parsed: AiConfig = serde_json::from_str(
            r#"{
                "excludedCategories": ["other", "games"],
                "externalEnabled": true,
                "activeMain": "old.gguf",
                "summaryMain": "__cloud__",
                "autoSummaryAt": "03:00"
            }"#,
        )
        .unwrap();
        assert_eq!(parsed.visibility, Visibility::default());
        assert_eq!(parsed.picks, Picks::default());
        let json = serde_json::to_string(&parsed).unwrap();
        for gone in [
            "excludedCategories",
            "externalEnabled",
            "activeMain",
            "summaryMain",
            "autoSummaryAt",
        ] {
            assert!(!json.contains(gone), "legacy key {gone} leaked: {json}");
        }
    }

    /// 开发版按日报、周报、对话分开存配置，没发过版，不迁移：读进来当空的，不报错。
    #[test]
    fn per_feature_profiles_read_as_empty() {
        let parsed: AiConfig = serde_json::from_str(
            r#"{"visibility": {"daily": {
                "profiles": {"p_work": {"name": "工作", "categories": ["code"]}},
                "autoProfile": "p_work"}}}"#,
        )
        .unwrap();
        assert_eq!(parsed.visibility, Visibility::default());
    }

    #[test]
    fn serde_uses_camel_case_keys_both_ways() {
        let mut cfg = base();
        cfg.cloud.push(CloudConnection {
            id: "c1".into(),
            name: "DeepSeek".into(),
            dialect: "deepseek".into(),
            endpoint: "https://api.deepseek.com/v1".into(),
            model: "deepseek-chat".into(),
            api_key: "sk-1".into(),
            context_length: Some(65_536),
        });
        cfg.picks.daily = Some(ModelRef::Cloud {
            connection: "c1".into(),
        });
        cfg.auto_summary_times = vec!["23:00".into()];

        let json = serde_json::to_string(&cfg).expect("serialization should succeed");
        for key in [
            "\"cloud\"",
            "\"apiKey\"",
            "\"contextLength\"",
            "\"picks\"",
            "\"promptLanguage\"",
            "\"autoSummaryTimes\"",
            "\"promptOverrides\"",
            "\"systemZh\"",
        ] {
            assert!(json.contains(key), "missing {key} in {json}");
        }
        assert!(!json.contains("\"api_key\""));
        assert!(!json.contains("\"prompt_language\""));

        let parsed: AiConfig = serde_json::from_str(&json).expect("round-trip should succeed");
        assert_eq!(parsed.cloud, cfg.cloud);
        assert_eq!(parsed.picks, cfg.picks);
        assert_eq!(parsed.auto_summary_times, cfg.auto_summary_times);
    }

    #[test]
    fn summary_max_tokens_half_ctx_with_floor() {
        let mut c = base();
        c.picks.daily = Some(ModelRef::Local {
            file: "m.gguf".into(),
        });
        assert_eq!(c.summary_max_tokens(Task::Daily), 4096);

        c.advanced.push(Advanced {
            task: Task::Daily,
            model: ModelRef::Local {
                file: "m.gguf".into(),
            },
            reasoning: None,
            ctx_size: Some(32_768),
            parallel_slots: None,
            batch_size: None,
        });
        assert_eq!(c.summary_max_tokens(Task::Daily), 16_384);

        c.advanced[0].ctx_size = Some(3_000);
        assert_eq!(c.summary_max_tokens(Task::Daily), 2048);
    }
}
