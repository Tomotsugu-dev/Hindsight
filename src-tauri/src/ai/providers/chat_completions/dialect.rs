//! What differs between the services that use Chat Completions.

use serde_json::{json, Value};

use super::request::ToolProtocol;
use crate::ai::model::Reasoning;

const CLOUD_REASONING_FLOOR: u32 = 12_288;
const LOCAL_REASONING_FLOOR: u32 = 4_096;

/// How one service uses Chat Completions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Dialect {
    OpenAi,
    Kimi,
    DeepSeek,
    OpenRouter,
    /// Other cloud services and self-hosted endpoints.
    Generic,
    LlamaServer,
}

impl Dialect {
    /// Picks the dialect from the provider preset in the settings (`ai.external_provider`).
    pub(super) fn for_provider(provider: &str) -> Self {
        match provider {
            "openai" => Self::OpenAi,
            "kimi" | "kimi-cn" => Self::Kimi,
            "deepseek" => Self::DeepSeek,
            "openrouter" => Self::OpenRouter,
            _ => Self::Generic,
        }
    }

    pub(super) fn is_cloud(self) -> bool {
        self != Self::LlamaServer
    }

    /// Returns how tools are given to the model.
    pub(super) fn tool_protocol(self) -> ToolProtocol {
        if self.is_cloud() {
            ToolProtocol::Native
        } else {
            ToolProtocol::Grammar
        }
    }

    /// Returns the name of the output limit field.
    pub(super) fn budget_key(self) -> &'static str {
        // For some models, OpenAI returns 400 for `max_tokens` and asks for `max_completion_tokens`.
        // TODO: llama-server also accepts `max_completion_tokens` since b8144.
        // `PINNED_TAG` is newer than b8144.
        // If local requests use this name too, cloud and local can share one field name.
        if self.is_cloud() {
            "max_completion_tokens"
        } else {
            "max_tokens"
        }
    }

    /// Returns the reasoning level that is actually sent.
    pub(super) fn effective_reasoning(self, asked: Reasoning, with_tools: bool) -> Reasoning {
        match self {
            // OpenAI Chat Completions does not allow function tools with a `reasoning_effort`
            // other than none. OpenAI also returns 400 when the request has no such field:
            // `Function tools with reasoning_effort are not supported`.
            Self::OpenAi if with_tools => Reasoning::Off,
            // The chat templates of some local models turn on thinking by default, for example
            // Qwen3.5-4B. See <https://huggingface.co/Qwen/Qwen3.5-4B>.
            // Thinking can use up the output limit.
            Self::LlamaServer if asked == Reasoning::Default => Reasoning::Off,
            _ => asked,
        }
    }

    /// Returns the output limit that is sent.
    ///
    /// Reasoning tokens count as output.
    /// Unless the level is `Off`, the result is at least the reasoning floor.
    pub(super) fn budget(self, answer: u32, reasoning: Reasoning) -> u32 {
        if reasoning == Reasoning::Off {
            return answer;
        }
        let floor = if self.is_cloud() {
            CLOUD_REASONING_FLOOR
        } else {
            LOCAL_REASONING_FLOOR
        };
        answer.max(floor)
    }

    /// Writes the level into the request body. `Default` writes no field.
    pub(super) fn write_reasoning(self, body: &mut Value, reasoning: Reasoning) {
        if reasoning == Reasoning::Default {
            return;
        }
        let off = reasoning == Reasoning::Off;
        match self {
            // Moonshot's kimi-k2.7-code must think. It returns 400 for none:
            // `only type=enabled is allowed for this model`.
            // Fixing the request after that 400 removes the reasoning fields.
            Self::OpenAi | Self::Kimi => {
                body["reasoning_effort"] = json!(low_medium_high(reasoning).unwrap_or("none"));
            }
            Self::DeepSeek => {
                body["thinking"] = json!({"type": if off { "disabled" } else { "enabled" }});
                // DeepSeek has only three levels: low, high and max.
                // See <https://api-docs.deepseek.com/guides/thinking_mode>.
                let effort = match reasoning {
                    Reasoning::Low => Some("low"),
                    Reasoning::Medium | Reasoning::High => Some("high"),
                    Reasoning::Max => Some("max"),
                    Reasoning::Default | Reasoning::Off => None,
                };
                if let Some(e) = effort {
                    body["reasoning_effort"] = json!(e);
                }
            }
            Self::OpenRouter => {
                body["reasoning"] = match low_medium_high(reasoning) {
                    Some(e) => json!({ "effort": e }),
                    None => json!({ "enabled": false }),
                };
            }
            // `chat_template_kwargs` passes variables to the chat template.
            // The Qwen3.5 template turns thinking on and off with `enable_thinking`.
            // See <https://huggingface.co/Qwen/Qwen3.5-4B>.
            // TODO: Low to Max all send `enable_thinking: true`. The four levels have the same effect.
            // llama-server accepts `reasoning_budget_tokens` in a request since b9982.
            // After `PINNED_TAG` moves to b9982 or later, each level can get its own budget.
            Self::Generic | Self::LlamaServer => {
                body["chat_template_kwargs"] = json!({ "enable_thinking": !off });
            }
        }
    }
}

/// Returns the matching value among low, medium and high.
/// `Max` becomes high. `Default` and `Off` return `None`.
fn low_medium_high(level: Reasoning) -> Option<&'static str> {
    match level {
        Reasoning::Low => Some("low"),
        Reasoning::Medium => Some("medium"),
        Reasoning::High | Reasoning::Max => Some("high"),
        Reasoning::Default | Reasoning::Off => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Reasoning::*;

    fn body_with_reasoning(dialect: Dialect, asked: Reasoning, with_tools: bool) -> Value {
        let mut body = json!({"model": "m", "messages": []});
        dialect.write_reasoning(&mut body, dialect.effective_reasoning(asked, with_tools));
        body
    }

    fn base() -> Value {
        json!({"model": "m", "messages": []})
    }

    /// 云端方言在 Default 档不往请求体加任何字段。
    #[test]
    fn default_adds_nothing_in_the_cloud() {
        for d in [
            Dialect::DeepSeek,
            Dialect::OpenRouter,
            Dialect::OpenAi,
            Dialect::Kimi,
            Dialect::Generic,
        ] {
            assert_eq!(body_with_reasoning(d, Default, false), base(), "{d:?}");
        }
    }

    /// DeepSeek 用 `thinking.type` 开关思考，用 `reasoning_effort` 给强度。Medium 发成 high，Off 不发强度。
    #[test]
    fn deepseek_switch_and_effort() {
        for (asked, want) in [(Low, "low"), (Medium, "high"), (High, "high"), (Max, "max")] {
            let b = body_with_reasoning(Dialect::DeepSeek, asked, false);
            assert_eq!(b["thinking"]["type"], "enabled");
            assert_eq!(b["reasoning_effort"], want);
        }
        let b = body_with_reasoning(Dialect::DeepSeek, Off, false);
        assert_eq!(b["thinking"]["type"], "disabled");
        assert!(b.get("reasoning_effort").is_none(), "关闭不发强度");
    }

    /// OpenRouter 的强度写进 `reasoning.effort`，Max 发成 high。Off 发 `reasoning.enabled: false`。
    /// 两种都不发顶层的 `reasoning_effort`。
    #[test]
    fn openrouter_nests_reasoning() {
        for (asked, want) in [
            (Low, "low"),
            (Medium, "medium"),
            (High, "high"),
            (Max, "high"),
        ] {
            let b = body_with_reasoning(Dialect::OpenRouter, asked, false);
            assert_eq!(b["reasoning"]["effort"], want);
            assert!(b.get("reasoning_effort").is_none());
        }
        let b = body_with_reasoning(Dialect::OpenRouter, Off, false);
        assert_eq!(b["reasoning"]["enabled"], false);
        assert!(b.get("reasoning_effort").is_none());
    }

    /// OpenAI 按档位发 `reasoning_effort`。Max 发成 high，Off 发 none。
    #[test]
    fn openai_sends_effort_per_level() {
        for (asked, want) in [
            (Low, "low"),
            (Medium, "medium"),
            (High, "high"),
            (Max, "high"),
            (Off, "none"),
        ] {
            assert_eq!(
                body_with_reasoning(Dialect::OpenAi, asked, false)["reasoning_effort"],
                want
            );
        }
    }

    /// OpenAI 带工具时每一档都发 none。Kimi 带工具时照常按档位发。
    #[test]
    fn openai_with_tools_always_turns_reasoning_off() {
        for asked in [Default, Off, Low, High, Max] {
            assert_eq!(
                body_with_reasoning(Dialect::OpenAi, asked, true)["reasoning_effort"],
                "none"
            );
        }
        assert_eq!(
            body_with_reasoning(Dialect::Kimi, Medium, true)["reasoning_effort"],
            "medium"
        );
        assert_eq!(
            body_with_reasoning(Dialect::Kimi, Off, true)["reasoning_effort"],
            "none"
        );
    }

    /// Generic 和 LlamaServer 只发思考开关。LlamaServer 在 Default 档也发关。
    #[test]
    fn switch_only_dialects() {
        let b = body_with_reasoning(Dialect::Generic, Off, false);
        assert_eq!(b["chat_template_kwargs"]["enable_thinking"], false);
        let b = body_with_reasoning(Dialect::Generic, Medium, false);
        assert_eq!(b["chat_template_kwargs"]["enable_thinking"], true);
        for (asked, want) in [
            (Default, false),
            (Off, false),
            (Low, true),
            (Medium, true),
            (High, true),
            (Max, true),
        ] {
            let b = body_with_reasoning(Dialect::LlamaServer, asked, true);
            assert_eq!(b["chat_template_kwargs"]["enable_thinking"], want);
        }
    }

    /// 档位不是 Off 时，上限不低于思考底线。档位是 Off 时，上限就是回答上限。
    #[test]
    fn budget_has_a_floor_unless_off() {
        assert_eq!(Dialect::DeepSeek.budget(1024, Default), 12_288);
        assert_eq!(Dialect::DeepSeek.budget(16_384, High), 16_384);
        assert_eq!(Dialect::DeepSeek.budget(1024, Off), 1024);
        assert_eq!(Dialect::LlamaServer.budget(1024, High), 4096);
        assert_eq!(Dialect::LlamaServer.budget(1024, Off), 1024);
    }

    /// 云端方言用 `max_completion_tokens`，LlamaServer 用 `max_tokens`。
    #[test]
    fn budget_key_differs_between_cloud_and_local() {
        assert_eq!(Dialect::Generic.budget_key(), "max_completion_tokens");
        assert_eq!(Dialect::LlamaServer.budget_key(), "max_tokens");
    }

    /// 云端发原生工具，本机用 Grammar。
    #[test]
    fn local_uses_grammar_for_tools() {
        assert_eq!(Dialect::DeepSeek.tool_protocol(), ToolProtocol::Native);
        assert_eq!(Dialect::LlamaServer.tool_protocol(), ToolProtocol::Grammar);
    }

    #[test]
    fn provider_presets_pick_a_dialect() {
        assert_eq!(Dialect::for_provider("kimi-cn"), Dialect::Kimi);
        assert_eq!(Dialect::for_provider("together"), Dialect::Generic);
        assert_eq!(Dialect::for_provider("custom"), Dialect::Generic);
    }
}
