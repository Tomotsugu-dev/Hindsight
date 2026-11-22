//! Model selection and per-(task, model) parameter resolution.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use super::{AiConfig, CloudConnection};
use crate::ai::model::Reasoning;

/// Minimum per-slot context size for chat when a user sets an explicit override.
const CHAT_MIN_CTX_SIZE: u32 = 8192;

/// 任务没选模型时的错误。方括号里是前端认的错误码，后面的英文给日志看。
pub const NO_MODEL_ERROR: &str = "[AI_NO_MODEL] no language model is configured";
/// 任务选的云端连接不存在或没填完时的错误。
pub const CLOUD_UNAVAILABLE_ERROR: &str =
    "[AI_CLOUD_UNAVAILABLE] the selected cloud connection is incomplete or unavailable";

/// AI tasks that pick a language model.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Task {
    Daily,
    Weekly,
    Chat,
}

/// A concrete language model target.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelRef {
    /// A local GGUF model file inside `models_path`.
    Local { file: String },
    /// A cloud connection referenced by [`CloudConnection::id`].
    Cloud { connection: String },
}

/// Model choice for the weekly report task.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Pick {
    /// Use the same model as the daily report.
    #[default]
    Inherit,
    /// A local GGUF model file inside `models_path`.
    Local { file: String },
    /// A cloud connection referenced by [`CloudConnection::id`].
    Cloud { connection: String },
}

/// Model selection per AI task.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct Picks {
    #[serde(deserialize_with = "or_default")]
    pub daily: Option<ModelRef>,
    #[serde(deserialize_with = "or_default")]
    pub weekly: Pick,
    #[serde(deserialize_with = "or_default")]
    pub chat: Option<ModelRef>,
}

/// User-configured reasoning level.
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

/// Per-(task, model) parameter overrides.
///
/// Missing entries or `None` fields use automatic defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Advanced {
    pub task: Task,
    pub model: ModelRef,
    /// Reasoning level override (`None` means automatic).
    #[serde(default)]
    pub reasoning: Option<ReasoningLevel>,
    /// Local engine per-slot context size override (`None` means engine default).
    #[serde(default)]
    pub ctx_size: Option<u32>,
    /// Local engine parallel slot count `-np` override (`None` means 1).
    #[serde(default)]
    pub parallel_slots: Option<u32>,
    /// Local engine `--batch-size` / `--ubatch-size` override (`None` means llama.cpp default).
    #[serde(default)]
    pub batch_size: Option<u32>,
}

impl Advanced {
    /// Returns `true` if every override field is `None`.
    pub(super) fn is_empty(&self) -> bool {
        self.reasoning.is_none()
            && self.ctx_size.is_none()
            && self.parallel_slots.is_none()
            && self.batch_size.is_none()
    }
}

/// Falls back to `Default` for unknown JSON values so newer settings stay readable by older versions.
fn or_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// Skips unrecognized items in a JSON array for forward compatibility.
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

fn normalize_model_ref(model: &ModelRef) -> Option<ModelRef> {
    match model {
        ModelRef::Local { file } => {
            let f = file.trim();
            (!f.is_empty()).then(|| ModelRef::Local {
                file: f.to_string(),
            })
        }
        ModelRef::Cloud { connection } => {
            let id = connection.trim();
            (!id.is_empty()).then(|| ModelRef::Cloud {
                connection: id.to_string(),
            })
        }
    }
}

impl AiConfig {
    /// Looks up a saved cloud connection by its `id`.
    pub fn cloud_connection(&self, id: &str) -> Option<&CloudConnection> {
        let id = id.trim();
        self.cloud.iter().find(|c| c.id == id)
    }

    /// Resolves the concrete model configured for `task`, if any.
    ///
    /// [`Pick::Inherit`] on [`Task::Weekly`] resolves to the daily report model.
    pub fn model_for(&self, task: Task) -> Option<ModelRef> {
        match task {
            Task::Daily => self.picks.daily.as_ref().and_then(normalize_model_ref),
            Task::Weekly => match &self.picks.weekly {
                Pick::Inherit => self.model_for(Task::Daily),
                Pick::Local { file } => {
                    normalize_model_ref(&ModelRef::Local { file: file.clone() })
                }
                Pick::Cloud { connection } => normalize_model_ref(&ModelRef::Cloud {
                    connection: connection.clone(),
                }),
            },
            Task::Chat => self.picks.chat.as_ref().and_then(normalize_model_ref),
        }
    }

    /// Returns whether `task` has a usable model configured (a local filename or a ready cloud
    /// connection).
    pub fn is_ready_for(&self, task: Task) -> bool {
        match self.model_for(task) {
            Some(ModelRef::Local { .. }) => true,
            Some(ModelRef::Cloud { connection }) => self
                .cloud_connection(&connection)
                .is_some_and(CloudConnection::is_ready),
            None => false,
        }
    }

    /// 检查 `task` 有没有能用的模型。
    ///
    /// 没选模型时返回 [`NO_MODEL_ERROR`]，选的云端连接不能用时返回 [`CLOUD_UNAVAILABLE_ERROR`]。
    pub fn check_ready(&self, task: Task) -> Result<(), &'static str> {
        match self.model_for(task) {
            None => Err(NO_MODEL_ERROR),
            Some(_) if !self.is_ready_for(task) => Err(CLOUD_UNAVAILABLE_ERROR),
            Some(_) => Ok(()),
        }
    }

    /// Returns the [`Advanced`] override entry matching `task` and its resolved model.
    pub fn advanced_for(&self, task: Task) -> Option<&Advanced> {
        let model = self.model_for(task)?;
        self.advanced
            .iter()
            .find(|a| a.task == task && a.model == model)
    }

    /// Returns the effective reasoning level for `task` (`Reasoning::Default` when unset).
    pub fn reasoning_for(&self, task: Task) -> Reasoning {
        self.advanced_for(task)
            .and_then(|a| a.reasoning)
            .map_or(Reasoning::Default, Reasoning::from)
    }

    /// Returns the explicit batch size override for `task`, if any.
    pub fn batch_size_for(&self, task: Task) -> Option<u32> {
        self.advanced_for(task).and_then(|a| a.batch_size)
    }

    /// Returns the explicit parallel slot override for `task`, if any.
    pub fn parallel_slots_for(&self, task: Task) -> Option<u32> {
        self.advanced_for(task).and_then(|a| a.parallel_slots)
    }

    /// Returns the explicit per-slot context size override for `task`, if any.
    pub fn ctx_size_for(&self, task: Task) -> Option<u32> {
        self.advanced_for(task).and_then(|a| a.ctx_size)
    }

    /// Returns the per-slot context size override for chat, floored to 8192 tokens so tool
    /// results fit. `None` keeps the engine default ([`crate::ai::server::DEFAULT_CTX_SIZE`]).
    pub fn chat_ctx_size(&self) -> Option<u32> {
        self.ctx_size_for(Task::Chat)
            .map(|c| c.max(CHAT_MIN_CTX_SIZE))
    }

    /// Returns `max_tokens` for one summary response: half of the effective context size
    /// (default 8192), with a floor of 2048 tokens for reasoning models.
    pub fn summary_max_tokens(&self, task: Task) -> u32 {
        let ctx = self.ctx_size_for(task).unwrap_or(8192);
        (ctx / 2).max(2048)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::config::tests::base;

    fn cloud_conn(id: &str, endpoint: &str, model: &str) -> CloudConnection {
        CloudConnection {
            id: id.into(),
            name: id.into(),
            dialect: "openai".into(),
            endpoint: endpoint.into(),
            model: model.into(),
            api_key: String::new(),
            context_length: None,
        }
    }

    fn adv(task: Task, model: ModelRef, reasoning: Option<ReasoningLevel>) -> Advanced {
        Advanced {
            task,
            model,
            reasoning,
            ctx_size: None,
            parallel_slots: None,
            batch_size: None,
        }
    }

    #[test]
    fn model_for_and_weekly_inheritance() {
        let local = |f: &str| ModelRef::Local { file: f.into() };
        let cloud = |id: &str| ModelRef::Cloud {
            connection: id.into(),
        };
        let mut c = base();
        assert_eq!(c.model_for(Task::Daily), None);
        assert_eq!(c.model_for(Task::Weekly), None);
        assert_eq!(c.model_for(Task::Chat), None);

        c.picks.daily = Some(local("daily.gguf"));
        assert_eq!(c.model_for(Task::Daily), Some(local("daily.gguf")));
        assert_eq!(c.model_for(Task::Weekly), Some(local("daily.gguf")));

        c.picks.weekly = Pick::Local {
            file: "week.gguf".into(),
        };
        assert_eq!(c.model_for(Task::Weekly), Some(local("week.gguf")));
        assert_eq!(c.model_for(Task::Daily), Some(local("daily.gguf")));

        c.picks.weekly = Pick::Cloud {
            connection: "c1".into(),
        };
        assert_eq!(c.model_for(Task::Weekly), Some(cloud("c1")));

        c.picks.chat = Some(cloud("c2"));
        assert_eq!(c.model_for(Task::Chat), Some(cloud("c2")));
    }

    #[test]
    fn is_ready_for_checks_cloud_connection_readiness() {
        let mut c = base();
        c.cloud.push(cloud_conn("c1", "https://api.x/v1", "m1"));
        c.cloud.push(cloud_conn("c2", "https://api.x/v1", ""));
        c.picks.daily = Some(ModelRef::Cloud {
            connection: "c1".into(),
        });
        c.picks.chat = Some(ModelRef::Cloud {
            connection: "c2".into(),
        });
        c.picks.weekly = Pick::Cloud {
            connection: "missing".into(),
        };
        assert!(c.is_ready_for(Task::Daily));
        assert!(!c.is_ready_for(Task::Chat), "missing model means not ready");
        assert!(!c.is_ready_for(Task::Weekly), "unknown id means not ready");
    }

    /// 没选模型和云端连接没填完是两种错，前端按错误码给出不同的引导。
    #[test]
    fn check_ready_tells_missing_model_from_unusable_cloud() {
        let mut c = base();
        assert_eq!(c.check_ready(Task::Daily), Err(NO_MODEL_ERROR));

        c.cloud.push(cloud_conn("c1", "https://api.x/v1", ""));
        c.picks.daily = Some(ModelRef::Cloud {
            connection: "c1".into(),
        });
        assert_eq!(c.check_ready(Task::Daily), Err(CLOUD_UNAVAILABLE_ERROR));

        c.cloud[0].model = "m1".into();
        assert_eq!(c.check_ready(Task::Daily), Ok(()));
        assert_eq!(c.check_ready(Task::Weekly), Ok(()), "周报跟日报");
        assert_eq!(c.check_ready(Task::Chat), Err(NO_MODEL_ERROR));

        assert!(NO_MODEL_ERROR.starts_with("[AI_NO_MODEL]"));
        assert!(CLOUD_UNAVAILABLE_ERROR.starts_with("[AI_CLOUD_UNAVAILABLE]"));
    }

    #[test]
    fn unknown_pick_and_advanced_variants_fall_back_safely() {
        let c: AiConfig = serde_json::from_str(
            r#"{
                "userBrief": "coding",
                "picks": {
                    "daily": {"kind": "apple_fm"},
                    "weekly": {"kind": "apple_fm"},
                    "chat": {"kind": "local", "file": "c.gguf"}
                },
                "advanced": [
                    {"task": "daily", "model": {"kind": "local", "file": "a.gguf"}, "reasoning": "ultra"},
                    {"task": "screen", "model": {"kind": "cloud", "connection": "c1"}, "reasoning": "high"},
                    {"task": "weekly", "model": {"kind": "cloud", "connection": "c1"}, "reasoning": "low"}
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(c.user_brief, "coding");
        assert_eq!(c.picks.daily, None);
        assert_eq!(c.picks.weekly, Pick::Inherit);
        assert_eq!(
            c.picks.chat,
            Some(ModelRef::Local {
                file: "c.gguf".into()
            })
        );
        assert_eq!(
            c.advanced,
            vec![adv(
                Task::Weekly,
                ModelRef::Cloud {
                    connection: "c1".into()
                },
                Some(ReasoningLevel::Low),
            )]
        );
    }

    #[test]
    fn reasoning_for_looks_up_by_task_and_resolved_model() {
        let m = ModelRef::Local {
            file: "sum.gguf".into(),
        };
        let mut c = base();
        c.picks.daily = Some(m.clone());
        c.advanced = vec![
            adv(Task::Weekly, m.clone(), Some(ReasoningLevel::High)),
            adv(Task::Daily, m.clone(), Some(ReasoningLevel::Off)),
        ];
        assert_eq!(c.reasoning_for(Task::Daily), Reasoning::Off);
        assert_eq!(c.reasoning_for(Task::Weekly), Reasoning::High);

        c.picks.weekly = Pick::Local {
            file: "other.gguf".into(),
        };
        assert_eq!(c.reasoning_for(Task::Weekly), Reasoning::Default);
    }

    #[test]
    fn chat_ctx_size_applies_8k_floor() {
        let m = ModelRef::Local {
            file: "chat.gguf".into(),
        };
        let mut c = base();
        c.picks.chat = Some(m.clone());
        assert_eq!(c.chat_ctx_size(), None);

        c.advanced.push(Advanced {
            task: Task::Chat,
            model: m.clone(),
            reasoning: None,
            ctx_size: Some(2048),
            parallel_slots: None,
            batch_size: None,
        });
        assert_eq!(c.chat_ctx_size(), Some(8192));

        c.advanced[0].ctx_size = Some(16_384);
        assert_eq!(c.chat_ctx_size(), Some(16_384));
    }
}
