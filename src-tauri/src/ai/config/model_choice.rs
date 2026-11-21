//! 每个任务实际用哪个模型、什么思考档位。
//!
//! 模型字段之间有回退链，调用方走这里的方法取值，不直接读字段。

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use super::AiConfig;
use crate::ai::model::Reasoning;

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

/// 每个任务用哪个模型。现在只有周报。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Picks {
    #[serde(deserialize_with = "or_default")]
    pub weekly: Pick,
}

/// 一个任务选的模型。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Pick {
    /// 和日报用同一个模型。
    #[default]
    Inherit,
    /// 本机模型。`file` 是模型目录里的文件名。
    Local { file: String },
    /// 云端 API。
    Cloud,
}

/// 一个模型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelRef {
    /// 本机模型。`file` 是模型目录里的文件名。
    Local { file: String },
    /// 云端 API。
    Cloud,
}

/// 用 AI 的任务。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Task {
    Daily,
    Weekly,
    Chat,
}

/// 用户给「一个任务 + 一个模型」设的参数。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Advanced {
    pub task: Task,
    pub model: ModelRef,
    /// 思考档位。`None` 是自动。
    #[serde(default)]
    pub reasoning: Option<ReasoningLevel>,
}

/// 用户能选的思考档位。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningLevel {
    Off,
    Low,
    Medium,
    High,
    Max,
}

impl From<ReasoningLevel> for Reasoning {
    fn from(level: ReasoningLevel) -> Self {
        match level {
            ReasoningLevel::Off => Self::Off,
            ReasoningLevel::Low => Self::Low,
            ReasoningLevel::Medium => Self::Medium,
            ReasoningLevel::High => Self::High,
            ReasoningLevel::Max => Self::Max,
        }
    }
}

/// 读不懂的值当默认值，免得新版本写的值让旧版本读不出整份设置。
fn or_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// 读不懂的项跳过，原因同 [`or_default`]。
pub(super) fn known_items<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = serde_json::Value::deserialize(d)?;
    let items = match value {
        serde_json::Value::Array(items) => items,
        _ => Vec::new(),
    };
    Ok(items
        .into_iter()
        .filter_map(|item| serde_json::from_value(item).ok())
        .collect())
}

impl AiConfig {
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

    /// 返回这个任务实际用的模型。
    pub fn task_model(&self, task: Task) -> ModelRef {
        let summary = if self.summary_use_cloud() {
            ModelRef::Cloud
        } else {
            ModelRef::Local {
                file: self.effective_summary_main().to_string(),
            }
        };
        match task {
            Task::Daily => summary,
            Task::Weekly => match &self.picks.weekly {
                Pick::Inherit => summary,
                Pick::Local { file } => ModelRef::Local { file: file.clone() },
                Pick::Cloud if self.external_enabled => ModelRef::Cloud,
                // 和日报一样：云端没开就用 active_main
                Pick::Cloud => ModelRef::Local {
                    file: self.active_main.clone(),
                },
            },
            Task::Chat if self.chat_use_cloud() => ModelRef::Cloud,
            Task::Chat => ModelRef::Local {
                file: self.effective_chat_main().to_string(),
            },
        }
    }

    /// 返回日报或周报的思考档位：`advanced` 里这个任务、这个模型的那一项。没有就是
    /// [`Reasoning::Default`]。
    pub fn report_reasoning(&self, task: Task) -> Reasoning {
        let model = self.task_model(task);
        self.advanced
            .iter()
            .find(|a| a.task == task && a.model == model)
            .and_then(|a| a.reasoning)
            .map_or(Reasoning::Default, Reasoning::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::config::tests::base;

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

    /// 旧设置里没有 picks、advanced：周报跟日报用同一个模型，思考是自动。
    #[test]
    fn report_settings_default_when_missing() {
        let c: AiConfig = serde_json::from_str(r#"{"summaryMain":"sum.gguf"}"#).unwrap();
        assert_eq!(c.picks.weekly, Pick::Inherit);
        assert!(c.advanced.is_empty());
        assert_eq!(c.task_model(Task::Weekly), c.task_model(Task::Daily));
        assert_eq!(c.report_reasoning(Task::Weekly), Reasoning::Default);
    }

    /// 读不懂的 kind、档位按默认读，同一份设置里别的字段照常读出来。
    #[test]
    fn unknown_report_settings_read_as_default() {
        let c: AiConfig = serde_json::from_str(
            r#"{
                "userBrief": "写代码",
                "picks": {"weekly": {"kind": "apple_fm"}},
                "advanced": [
                    {"task": "daily", "model": {"kind": "local", "file": "a.gguf"}, "reasoning": "ultra"},
                    {"task": "screen", "model": {"kind": "cloud"}, "reasoning": "high"},
                    {"task": "weekly", "model": {"kind": "cloud"}, "reasoning": "low"}
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(c.user_brief, "写代码");
        assert_eq!(c.picks.weekly, Pick::Inherit);
        assert_eq!(
            c.advanced,
            vec![Advanced {
                task: Task::Weekly,
                model: ModelRef::Cloud,
                reasoning: Some(ReasoningLevel::Low),
            }]
        );
    }

    /// 周报的模型按 picks.weekly 选；选云端但云端没开时，和日报一样用 active_main。
    #[test]
    fn weekly_model_follows_the_pick() {
        let local = |f: &str| ModelRef::Local { file: f.into() };
        let mut c = AiConfig {
            summary_main: "sum.gguf".into(),
            active_main: "active.gguf".into(),
            ..AiConfig::default()
        };
        assert_eq!(c.task_model(Task::Weekly), local("sum.gguf"), "inherit");

        c.summary_main = SUMMARY_CLOUD_SENTINEL.into();
        c.external_enabled = true;
        assert_eq!(c.task_model(Task::Weekly), ModelRef::Cloud, "inherit 云端");

        c.picks.weekly = Pick::Local {
            file: "week.gguf".into(),
        };
        assert_eq!(c.task_model(Task::Weekly), local("week.gguf"));
        assert_eq!(c.task_model(Task::Daily), ModelRef::Cloud, "日报不受影响");

        c.picks.weekly = Pick::Cloud;
        assert_eq!(c.task_model(Task::Weekly), ModelRef::Cloud);
        c.external_enabled = false;
        assert_eq!(c.task_model(Task::Weekly), local("active.gguf"));
    }

    /// 思考档位按「任务 + 实际用的模型」找；周报 inherit 时用日报的模型找。
    #[test]
    fn report_reasoning_is_found_by_task_and_model() {
        let mut c = AiConfig {
            summary_main: "sum.gguf".into(),
            advanced: vec![
                Advanced {
                    task: Task::Weekly,
                    model: ModelRef::Local {
                        file: "sum.gguf".into(),
                    },
                    reasoning: Some(ReasoningLevel::High),
                },
                Advanced {
                    task: Task::Daily,
                    model: ModelRef::Local {
                        file: "sum.gguf".into(),
                    },
                    reasoning: Some(ReasoningLevel::Off),
                },
            ],
            ..AiConfig::default()
        };
        assert_eq!(c.report_reasoning(Task::Weekly), Reasoning::High);
        assert_eq!(c.report_reasoning(Task::Daily), Reasoning::Off);

        // 换了模型，这个模型没设过：自动
        c.picks.weekly = Pick::Local {
            file: "week.gguf".into(),
        };
        assert_eq!(c.report_reasoning(Task::Weekly), Reasoning::Default);
    }
}
