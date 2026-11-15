//! Details of the Chat Completions protocol: the request paths, the format of the model list,
//! how to join a streaming response, and the rules that fix a request after a 400.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::ai::transport::Protocol;

/// The [`Protocol`] implementation for Chat Completions.
/// Callers pass it to `ModelTransport::post`.
pub(crate) struct ChatCompletions;

impl Protocol for ChatCompletions {
    fn read_stream(&self, events: &[String]) -> Result<Value, String> {
        join_stream(events)
    }

    fn heal(&self, body: &mut Value, error: &str) -> bool {
        heal_request(body, error)
    }

    fn is_last(&self, data: &str) -> bool {
        data == "[DONE]"
    }
}

pub(crate) const CHAT_COMPLETIONS_PATH: &str = "/chat/completions";
pub(crate) const MODELS_PATH: &str = "/models";

/// Takes the model IDs out of the response body of [`MODELS_PATH`].
///
/// # Errors
///
/// Returns a parse error when the body has no `data` array,
/// or when an item has no string `id`.
pub(crate) fn model_ids(body: Value) -> Result<Vec<String>, serde_json::Error> {
    #[derive(serde::Deserialize)]
    struct Models {
        data: Vec<Model>,
    }
    #[derive(serde::Deserialize)]
    struct Model {
        id: String,
    }
    let models: Models = serde_json::from_value(body)?;
    Ok(models.data.into_iter().map(|m| m.id).collect())
}

/// Joins the `data` of every event in one streaming response into the non-streaming shape.
///
/// Returns an error message for the user when `[DONE]` did not arrive.
fn join_stream(events: &[String]) -> Result<Value, String> {
    let mut acc = SseAccumulator::default();
    for data in events {
        acc.push_data(data);
    }
    if !acc.done {
        // TODO: This count covers only the text. If the stream breaks while the model
        // is reasoning or writing a tool call, the error still says 0 characters.
        return Err(format!(
            "流式响应未正常结束（已收到 {} 字符，缺 [DONE]）",
            acc.content_chars()
        ));
    }
    Ok(acc.finish())
}

#[derive(Debug, Default)]
struct SseAccumulator {
    content: Option<String>,
    reasoning: Option<String>,
    tool_calls: BTreeMap<u64, ToolCallParts>,
    /// The usage of the whole request.
    /// It comes from the last chunk that has both `prompt_tokens` and `completion_tokens`.
    usage: Option<Value>,
    done: bool,
    /// The number of deltas received.
    /// A non-empty text delta, a non-empty reasoning delta and a tool call delta each count as one.
    /// When no chunk has both `prompt_tokens` and `completion_tokens`,
    /// this number stands in for the output token count.
    delta_count: u32,
}

#[derive(Debug, Default)]
struct ToolCallParts {
    id: Option<String>,
    kind: Option<String>,
    name: Option<String>,
    arguments: Option<String>,
}

impl ToolCallParts {
    /// In Chat Completions, every item of `tool_calls` must have a `type`.
    fn into_json(self) -> Value {
        let mut function = serde_json::Map::new();
        if let Some(name) = self.name {
            function.insert("name".into(), json!(name));
        }
        if let Some(arguments) = self.arguments {
            function.insert("arguments".into(), json!(arguments));
        }
        let mut call = json!({
            "type": self.kind.unwrap_or_else(|| "function".to_string()),
            "function": function,
        });
        if let Some(id) = self.id {
            call["id"] = json!(id);
        }
        call
    }
}

/// Appends `s` to the string in `slot`.
/// When `slot` is `None`, an empty string goes in first.
fn append(slot: &mut Option<String>, s: &str) {
    slot.get_or_insert_with(String::new).push_str(s);
}

impl SseAccumulator {
    fn content_chars(&self) -> usize {
        self.content.as_deref().map_or(0, |s| s.chars().count())
    }

    fn push_data(&mut self, data: &str) {
        if data == "[DONE]" {
            self.done = true;
            return;
        }
        let Ok(chunk) = serde_json::from_str::<Value>(data) else {
            return;
        };
        // The usage in a chunk is the total of the whole request up to that chunk.
        // So the last one is kept, and the values are not added up.
        // OpenAI sends usage only in the last chunk.
        // See <https://platform.openai.com/docs/api-reference/chat/create>.
        // SiliconFlow sends the running total in every chunk.
        let usage = &chunk["usage"];
        // TODO: A chunk with only the input or only the output token count is not recorded.
        // The one count that the service reported is lost.
        if usage["prompt_tokens"].is_u64() && usage["completion_tokens"].is_u64() {
            self.usage = Some(usage.clone());
        }
        let Some(choices) = chunk["choices"].as_array() else {
            return;
        };
        for choice in choices {
            let delta = &choice["delta"];
            if let Some(t) = delta["content"].as_str().filter(|s| !s.is_empty()) {
                append(&mut self.content, t);
                self.delta_count += 1;
            }
            if let Some(t) = delta["reasoning_content"]
                .as_str()
                .filter(|s| !s.is_empty())
            {
                append(&mut self.reasoning, t);
                self.delta_count += 1;
            }
            for call in delta["tool_calls"].as_array().into_iter().flatten() {
                self.push_tool_call(call);
                self.delta_count += 1;
            }
        }
    }

    /// `id` and `name` are appended too, not replaced. `type` is replaced.
    /// The official OpenAI Python SDK joins the parts the same way.
    /// See <https://github.com/openai/openai-python/blob/main/src/openai/lib/streaming/_deltas.py>.
    fn push_tool_call(&mut self, call: &Value) {
        let parts = self
            .tool_calls
            .entry(call["index"].as_u64().unwrap_or(0))
            .or_default();
        if let Some(s) = call["id"].as_str() {
            append(&mut parts.id, s);
        }
        if let Some(s) = call["type"].as_str() {
            parts.kind = Some(s.to_string());
        }
        if let Some(s) = call["function"]["name"].as_str() {
            append(&mut parts.name, s);
        }
        if let Some(s) = call["function"]["arguments"].as_str() {
            append(&mut parts.arguments, s);
        }
    }

    /// SiliconFlow sends `finish_reason: null` in the last chunk of the stream.
    fn finish(self) -> Value {
        let has_calls = !self.tool_calls.is_empty();
        // Without text, `content` is null when there are tool calls,
        // and an empty string when there are none. A non-streaming response does the same.
        let content = match self.content {
            Some(s) => json!(s),
            None if has_calls => Value::Null,
            None => json!(""),
        };
        let mut message = json!({ "role": "assistant", "content": content });
        if let Some(reasoning) = self.reasoning {
            message["reasoning_content"] = json!(reasoning);
        }
        if has_calls {
            let calls: Vec<Value> = self
                .tool_calls
                .into_values()
                .map(ToolCallParts::into_json)
                .collect();
            message["tool_calls"] = json!(calls);
        }
        // TODO: Without usage, this makes up 0 input tokens,
        // and uses the delta count as the output tokens.
        // So these two items of `Usage` are not `None`.
        // The delta count is not a token count either.
        let usage = self.usage.unwrap_or_else(
            || json!({ "prompt_tokens": 0, "completion_tokens": self.delta_count }),
        );
        // TODO: The `finish_reason` from the stream is not kept.
        // So an empty cloud reply cut off by the length limit cannot report `LLM_EMPTY_TRUNCATED`,
        // and a text cut off halfway does not show that it was cut off.
        json!({
            "choices": [{ "message": message, "finish_reason": "stop" }],
            "usage": usage,
        })
    }
}

/// `remove_field` does not remove these fields, and value changes skip them.
/// If these fields are missing or changed, the reply is not what the caller wants,
/// even when the service returns 200.
const PROTECTED_FIELDS: [&str; 4] = ["model", "messages", "tools", "tool_choice"];

/// All reasoning fields that `Dialect::write_reasoning` can write.
/// When the service says the model must think, `heal_request` removes all of these fields.
/// So this list must match the fields that `write_reasoning` writes.
const THINKING_FIELDS: [&str; 4] = [
    "thinking",
    "reasoning",
    "reasoning_effort",
    "chat_template_kwargs",
];

/// Returns the `n`th part of `msg` that is in single quotes, counted from 0.
fn nth_quoted(msg: &str, n: usize) -> Option<&str> {
    msg.split('\'').nth(n * 2 + 1)
}

/// Returns the part of `msg` after the first `start` and before the next `end`.
/// Leading and trailing whitespace is removed.
fn between<'a>(msg: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let rest = &msg[msg.find(start)? + start.len()..];
    Some(rest[..rest.find(end)?].trim())
}

/// Removes `key` from `body`. Returns true when the field was removed.
/// Returns false when `key` is protected or missing.
fn remove_field(body: &mut Value, key: &str) -> bool {
    if PROTECTED_FIELDS.contains(&key) {
        return false;
    }
    body.as_object_mut().and_then(|o| o.remove(key)).is_some()
}

/// 按 400 的错误文字 `err_msg` 就地改 `body`。改了时返回 true。没有规则能改时返回 false。
fn heal_request(body: &mut Value, err_msg: &str) -> bool {
    let lower_msg = err_msg.to_ascii_lowercase();

    // 字段改名：错误文字里第一个用单引号括起来的是旧字段名，第二个是新字段名。
    // 例如 OpenAI 的 `'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead.`
    // TODO: 发给云端的请求里，只剩「测试连接」的 `test_ai_chat` 还发 `max_tokens`，OpenAI 靠这条规则才能测通。
    // `test_ai_chat` 删掉或改发 `max_completion_tokens` 以后，删掉这条规则。
    if lower_msg.contains("is not supported") && lower_msg.contains("instead") {
        if let (Some(old), Some(new)) = (nth_quoted(err_msg, 0), nth_quoted(err_msg, 1)) {
            if let Some(o) = body.as_object_mut() {
                // TODO: 改名不查 `PROTECTED_FIELDS`，受保护的字段也会被改名。
                if let Some(v) = o.remove(old) {
                    o.insert(new.to_string(), v);
                    return true;
                }
            }
        }
    }

    // 值不被支持：错误文字里第一个用单引号括起来的是字段名。
    // `Supported values` 后面列出了值时，字段改成列出的第一个值。没有列出时，删掉这个字段。
    // 例如 `'reasoning_effort' does not support 'minimal' with this model. Supported values are: 'none', 'low', 'medium', 'high'.`
    if lower_msg.contains("does not support") {
        if let Some(param) = nth_quoted(err_msg, 0) {
            if body.get(param).is_some() && !PROTECTED_FIELDS.contains(&param) {
                let fallback = lower_msg
                    .find("supported values")
                    .and_then(|i| nth_quoted(&err_msg[i..], 0).map(str::to_string));
                return match fallback {
                    Some(v) => {
                        body[param] = json!(v);
                        true
                    }
                    None => remove_field(body, param),
                };
            }
        }
    }

    // 组合不被支持：`with` 和 `are not supported` 之间是被点名的字段。
    // 例如 OpenAI 的 `Function tools with reasoning_effort are not supported`。
    // 被点名的是 `reasoning_effort` 时改成 none。请求里没有这个字段时也补上，因为 OpenAI 带工具时默认档不是 none。
    // 被点名的是其他字段时，删掉它。
    if let Some(param) = between(&lower_msg, "with ", " are not supported") {
        if param == "reasoning_effort" {
            if body.get(param).and_then(Value::as_str) != Some("none") {
                body["reasoning_effort"] = json!("none");
                return true;
            }
        } else if remove_field(body, param) {
            return true;
        }
    }

    // 模型必须思考：删掉全部思考字段，用服务的默认档。
    // 例如 Moonshot 的 `only type=enabled is allowed for this model`。
    if lower_msg.contains("only type=enabled")
        || (lower_msg.contains("reasoning") && lower_msg.contains("mandatory"))
        || lower_msg.contains("cannot be disabled")
    {
        let mut hit = false;
        for f in THINKING_FIELDS {
            hit |= remove_field(body, f);
        }
        if hit {
            return true;
        }
    }

    // 未知字段：删掉服务不认的字段，受保护的字段除外。
    // 例如 `Unknown parameter: 'thinking'.`
    if lower_msg.contains("unknown parameter") {
        if let Some(p) = nth_quoted(err_msg, 0) {
            return remove_field(body, p);
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(data: &[&str]) -> Vec<String> {
        data.iter().map(|s| s.to_string()).collect()
    }

    /// [DONE] 是流的最后一个事件，传输层读到它就停。
    #[test]
    fn done_is_the_last_event() {
        assert!(ChatCompletions.is_last("[DONE]"));
        assert!(!ChatCompletions.is_last(r#"{"choices":[]}"#));
    }

    /// 多个块的正文按顺序拼起来。拼出的 `finish_reason` 固定是 stop。
    #[test]
    fn sse_joins_deltas_and_ends_on_done() {
        let resp = join_stream(&events(&[
            r#"{"choices":[{"delta":{"role":"assistant","content":"你"}}]}"#,
            r#"{"choices":[{"delta":{"content":"好"},"finish_reason":null}]}"#,
            "[DONE]",
        ]))
        .unwrap();
        assert_eq!(resp["choices"][0]["message"]["content"], "你好");
        assert_eq!(resp["choices"][0]["finish_reason"], "stop");
    }

    /// DeepSeek 的流里 `content` 和 `reasoning_content` 轮流是 null。正文和思考内容分开拼。
    #[test]
    fn sse_separates_reasoning_from_content_with_explicit_nulls() {
        let resp = join_stream(&events(&[
            r#"{"choices":[{"delta":{"content":null,"reasoning_content":"想"}}]}"#,
            r#"{"choices":[{"delta":{"content":null,"reasoning_content":"一下"}}]}"#,
            r#"{"choices":[{"delta":{"content":"答案","reasoning_content":null}}]}"#,
            "[DONE]",
        ]))
        .unwrap();
        assert_eq!(resp["choices"][0]["message"]["content"], "答案");
        assert_eq!(resp["choices"][0]["message"]["reasoning_content"], "想一下");
    }

    /// SiliconFlow 每块都带用量，而且是累计值。拼出的用量取最后一次，不相加。
    #[test]
    fn sse_usage_takes_last_not_sum() {
        let resp = join_stream(&events(&[
            r#"{"choices":[{"delta":{"content":"a"}}],"usage":{"prompt_tokens":31,"completion_tokens":1}}"#,
            r#"{"choices":[{"delta":{"content":"b"}}],"usage":{"prompt_tokens":31,"completion_tokens":2}}"#,
            "[DONE]",
        ]))
        .unwrap();
        let u = &resp["usage"];
        assert_eq!(u["prompt_tokens"], 31);
        assert_eq!(u["completion_tokens"], 2, "累计值取最后一次而不是相加");
    }

    /// 服务没报用量时，`completion_tokens` 是收到的片段数，`prompt_tokens` 是 0。
    #[test]
    fn sse_falls_back_to_counting_deltas_without_usage() {
        let resp = join_stream(&events(&[
            r#"{"choices":[{"delta":{"content":"a"}}]}"#,
            r#"{"choices":[{"delta":{"content":"b"}}]}"#,
            r#"{"choices":[{"delta":{"content":"c"}}]}"#,
            "[DONE]",
        ]))
        .unwrap();
        let u = &resp["usage"];
        assert_eq!(u["completion_tokens"], 3);
        assert_eq!(u["prompt_tokens"], 0, "没报用量时输入填 0");
    }

    /// 解析不了的块被跳过，后面的块照常拼。
    #[test]
    fn sse_skips_unparsable_data() {
        let resp = join_stream(&events(&[
            "{not json}",
            r#"{"choices":[{"delta":{"content":"ok"}}]}"#,
            "[DONE]",
        ]))
        .unwrap();
        assert_eq!(resp["choices"][0]["message"]["content"], "ok");
    }

    /// 没收到 [DONE] 时返回错误，错误里有已收到的正文字符数。
    #[test]
    fn sse_without_done_is_incomplete() {
        let err =
            join_stream(&events(&[r#"{"choices":[{"delta":{"content":"半截"}}]}"#])).unwrap_err();
        assert!(err.contains("缺 [DONE]"), "{err}");
        assert!(
            err.contains("已收到 2 字符"),
            "已收内容供错误信息报字符数: {err}"
        );
    }

    /// 一个工具调用的参数分多块到达时按顺序接起来。不同 `index` 的调用分开拼，缺 `type` 的补成 function。
    #[test]
    fn sse_joins_tool_call_fragments_by_index() {
        let resp = join_stream(&events(&[
            r#"{"choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"get_timeline","arguments":""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"date_from\":"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"2026-07-01\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_2","function":{"name":"search_text","arguments":"{}"}}]}}]}"#,
            "[DONE]",
        ]))
        .unwrap();
        let msg = &resp["choices"][0]["message"];
        assert_eq!(
            msg["content"],
            Value::Null,
            "只有工具调用时 content 是 null"
        );
        let calls = msg["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["id"], "call_1");
        assert_eq!(calls[0]["type"], "function");
        assert_eq!(calls[0]["function"]["name"], "get_timeline");
        assert_eq!(
            calls[0]["function"]["arguments"],
            r#"{"date_from":"2026-07-01"}"#
        );
        assert_eq!(calls[1]["type"], "function", "缺 type 时补 function");
        assert_eq!(calls[1]["function"]["name"], "search_text");
    }

    /// DeepSeek 思考模式的流里有思考内容和工具调用时，拼出的消息两样都留着。
    #[test]
    fn sse_keeps_reasoning_next_to_tool_call() {
        let resp = join_stream(&events(&[
            r#"{"choices":[{"delta":{"reasoning_content":"先查时间线"}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","type":"function","function":{"name":"get_timeline","arguments":"{}"}}]}}]}"#,
            "[DONE]",
        ]))
        .unwrap();
        let msg = &resp["choices"][0]["message"];
        assert_eq!(msg["reasoning_content"], "先查时间线");
        assert_eq!(msg["tool_calls"][0]["id"], "c");
    }

    /// 错误文案取自真实服务返回的原文。「组合不被支持」的第二条文案是第一条的缩短版，最后一个认不出的错误是虚构的。
    #[test]
    fn heal_request_rules_cover_real_world_400s() {
        let e400 = |s: &str| format!("HTTP 400 Bad Request: {{\"error\":{{\"message\":\"{s}\"}}}}");

        // 字段改名：OpenAI 拒收 `max_tokens`。
        let mut b = json!({"model": "m", "max_tokens": 1024});
        assert!(heal_request(
            &mut b,
            &e400("Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead.")
        ));
        assert_eq!(b["max_completion_tokens"], 1024);
        assert!(b.get("max_tokens").is_none());

        // 未知字段：服务不认 `thinking`。
        let mut b = json!({"model": "m", "thinking": {"type": "disabled"}});
        assert!(heal_request(
            &mut b,
            &e400("Unknown parameter: 'thinking'.")
        ));
        assert!(b.get("thinking").is_none());

        // 值不被支持：改成错误文字里列出的第一个值。
        let mut b = json!({"model": "m", "reasoning_effort": "minimal"});
        assert!(heal_request(
            &mut b,
            &e400("Unsupported value: 'reasoning_effort' does not support 'minimal' with this model. Supported values are: 'none', 'low', 'medium', 'high'.")
        ));
        assert_eq!(b["reasoning_effort"], "none");

        // 组合不被支持：请求里没有 `reasoning_effort` 时也补上 none。
        let mut b = json!({"model": "m", "tools": []});
        assert!(heal_request(
            &mut b,
            &e400("Function tools with reasoning_effort are not supported for gpt-5.6-luna in /v1/chat/completions.")
        ));
        assert_eq!(b["reasoning_effort"], "none");
        // 已经是 none 还报同样的错时，返回 false，不再重发。
        assert!(!heal_request(
            &mut b,
            &e400("Function tools with reasoning_effort are not supported for gpt-5.6-luna.")
        ));

        // 模型必须思考（Moonshot 的 kimi-k2.7-code）：删掉全部思考字段。
        let mut b = json!({"model": "m", "reasoning_effort": "none", "tools": []});
        assert!(heal_request(
            &mut b,
            &e400("invalid thinking: only type=enabled is allowed for this model")
        ));
        assert!(b.get("reasoning_effort").is_none());
        assert!(b.get("tools").is_some(), "撤思考不得波及 tools");

        // 认不出的错误：返回 false，请求体不变。
        let mut b = json!({"model": "m", "max_tokens": 1});
        assert!(!heal_request(&mut b, &e400("内部错误 E5012")));
        assert_eq!(b["max_tokens"], 1);
    }

    /// 服务说不认受保护的字段时，自愈不删它，返回 false。
    #[test]
    fn heal_request_never_sacrifices_core_fields() {
        let e400 = |s: &str| format!("HTTP 400 Bad Request: {{\"error\":{{\"message\":\"{s}\"}}}}");
        for f in PROTECTED_FIELDS {
            let mut b = json!({"model": "m", "messages": [], "tools": [], "tool_choice": "auto"});
            assert!(
                !heal_request(&mut b, &e400(&format!("Unknown parameter: '{f}'."))),
                "{f} 不该为了让请求通过被删"
            );
            assert!(b.get(f).is_some(), "{f} 必须原样保留");
        }
    }

    /// 服务说受保护字段的值不被支持时，自愈不改这个值，返回 false。错误文字是编的。
    #[test]
    fn heal_request_never_changes_core_values() {
        let e400 = |s: &str| format!("HTTP 400 Bad Request: {{\"error\":{{\"message\":\"{s}\"}}}}");
        for f in PROTECTED_FIELDS {
            let original =
                json!({"model": "m", "messages": [], "tools": [], "tool_choice": "auto"});
            let mut b = original.clone();
            let err = format!(
                "Unsupported value: '{f}' does not support this value. Supported values are: 'x'."
            );
            assert!(!heal_request(&mut b, &e400(&err)), "{f} 的值不该被改");
            assert_eq!(b, original, "{f} 必须原样保留");
        }
    }
}
