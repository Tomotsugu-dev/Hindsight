//! Reads a Chat Completions reply into a [`Response`].

use std::time::Instant;

use serde::Deserialize;
use serde_json::{json, Value};

use super::request::ToolProtocol;
use crate::ai::model::{Carry, ModelError, Response, ToolCall, Usage};

/// Reads one reply. `reply` has the shape of a non-streamed reply.
///
/// `next_call` is the position of this reply's tool call in the conversation, counted from 1.
/// A Grammar call and a native call without an id from the service both get the id
/// `call_{next_call}`.
///
/// # Errors
///
/// Returns [`ModelError::Empty`] when the text is empty and there is no native call,
/// or when the Grammar `answer` is empty.
/// Returns [`ModelError::Provider`] when the Grammar output is not a decision object.
pub(super) fn response(
    reply: &Value,
    tools: Option<ToolProtocol>,
    next_call: usize,
    started: Instant,
) -> Result<Response, ModelError> {
    let usage = Usage {
        input_tokens: reply["usage"]["prompt_tokens"].as_u64(),
        output_tokens: reply["usage"]["completion_tokens"].as_u64(),
        reasoning_tokens: reply["usage"]["completion_tokens_details"]["reasoning_tokens"].as_u64(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    let choice = &reply["choices"][0];
    let message = &choice["message"];
    let text = message["content"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_string();
    let finish_reason = choice["finish_reason"].as_str().unwrap_or("<none>");
    let reasoning_chars = message["reasoning_content"]
        .as_str()
        .map_or(0, |s| s.chars().count());
    log::info!(
        "[chat-result] latency={}ms prompt_tokens={:?} completion_tokens={:?} finish_reason={} content_chars={} reasoning_chars={}",
        usage.elapsed_ms,
        usage.input_tokens,
        usage.output_tokens,
        finish_reason,
        text.chars().count(),
        reasoning_chars,
    );

    if tools == Some(ToolProtocol::Native) {
        if let Some((call, carry)) = native_call(message, next_call) {
            return Ok(Response {
                text,
                call: Some(call),
                carry: Some(carry),
                usage,
            });
        }
    }
    if text.is_empty() {
        return Err(empty_error(finish_reason, &usage, reasoning_chars));
    }
    if tools == Some(ToolProtocol::Grammar) {
        return grammar_response(&text, next_call, usage);
    }
    Ok(Response {
        text,
        call: None,
        carry: None,
        usage,
    })
}

/// Takes the first native tool call from `message`.
/// Also returns the whole message, which goes back to the model in the next step.
/// Returns `None` when there is no call.
///
/// When the service gives no call id, the id is `call_{next_call}`.
fn native_call(message: &Value, next_call: usize) -> Option<(ToolCall, Carry)> {
    let first = message["tool_calls"].get(0)?;
    let id = first["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map_or_else(|| format!("call_{next_call}"), str::to_string);
    let name = first["function"]["name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    // TODO: Arguments that are not JSON become an empty object.
    // The caller cannot tell broken arguments from missing arguments.
    let arguments = first["function"]["arguments"]
        .as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| json!({}));
    let mut kept = first.clone();
    kept["id"] = json!(id);
    let mut carried = message.clone();
    // The caller runs only the first call. The carried message keeps only this call.
    // OpenAI returns 400 when a call in the carried message has no matching tool message after it.
    carried["tool_calls"] = json!([kept]);
    Some((
        ToolCall {
            id,
            name,
            arguments,
        },
        Carry(carried),
    ))
}

/// Reads the Grammar output into a reply.
///
/// `decision` is the JSON decision object from the model.
/// The caller has already checked that it is not empty.
/// When `action` is `answer`, the reply text is the `answer` field.
/// When `action` is a tool name, the reply is a call.
/// The other fields of the decision object are the arguments of the call.
/// The call id is `call_{next_call}`.
/// `usage` goes into the reply as it is.
fn grammar_response(
    decision: &str,
    next_call: usize,
    usage: Usage,
) -> Result<Response, ModelError> {
    #[derive(Deserialize)]
    struct Decision {
        action: String,
        answer: Option<String>,
        #[serde(flatten)]
        arguments: Value,
    }
    let d: Decision = serde_json::from_str(decision)
        .map_err(|e| ModelError::Provider(format!("the decision is not valid JSON: {e}")))?;
    if d.action == "answer" {
        let answer = d.answer.unwrap_or_default().trim().to_string();
        if answer.is_empty() {
            return Err(ModelError::Empty {
                code: "LLM_EMPTY",
                detail: "the decision has an empty answer".into(),
            });
        }
        return Ok(Response {
            text: answer,
            call: None,
            carry: None,
            usage,
        });
    }
    Ok(Response {
        text: String::new(),
        call: Some(ToolCall {
            id: format!("call_{next_call}"),
            name: d.action,
            arguments: d.arguments,
        }),
        carry: None,
        usage,
    })
}

/// Returns the error for an empty reply text.
///
/// The error code names the cause.
/// The cause is picked from `finish_reason`, the output token count,
/// and the number of characters in the reasoning text (`reasoning_chars`).
fn empty_error(finish_reason: &str, usage: &Usage, reasoning_chars: usize) -> ModelError {
    let code = if reasoning_chars > 0 {
        "LLM_EMPTY_REASONING"
    } else if finish_reason == "stop" && usage.output_tokens == Some(0) {
        "LLM_EMPTY_EOS"
    } else if finish_reason == "length" && usage.output_tokens.is_some_and(|n| n > 0) {
        "LLM_EMPTY_TRUNCATED"
    } else {
        "LLM_EMPTY"
    };
    ModelError::Empty {
        code,
        detail: format!(
            "empty content (finish_reason={finish_reason}, prompt_tokens={:?}, completion_tokens={:?}, reasoning_chars={reasoning_chars})",
            usage.input_tokens, usage.output_tokens,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response_now(reply: Value, tools: Option<ToolProtocol>) -> Result<Response, ModelError> {
        response(&reply, tools, 3, Instant::now())
    }

    fn content(text: &str, finish: &str, usage: Value) -> Value {
        json!({
            "choices": [{
                "message": {"role": "assistant", "content": text},
                "finish_reason": finish
            }],
            "usage": usage
        })
    }

    fn empty_code(reply: Value) -> &'static str {
        match response_now(reply, None) {
            Err(ModelError::Empty { code, .. }) => code,
            other => panic!("应是 Empty: {other:?}"),
        }
    }

    /// 正文去掉首尾空白。用量逐项读出，思考用量在 `completion_tokens_details` 里。
    #[test]
    fn text_is_trimmed_and_usage_is_read() {
        let r = response_now(
            json!({
                "choices": [{"message": {"content": "  今天在写测试。 ", "tool_calls": []}}],
                "usage": {"prompt_tokens": 42, "completion_tokens": 17,
                    "completion_tokens_details": {"reasoning_tokens": 5}}
            }),
            Some(ToolProtocol::Native),
        )
        .unwrap();
        assert_eq!(r.text, "今天在写测试。");
        assert!(r.call.is_none(), "空的 tool_calls 不是调用");
        assert_eq!(
            (
                r.usage.input_tokens,
                r.usage.output_tokens,
                r.usage.reasoning_tokens
            ),
            (Some(42), Some(17), Some(5))
        );
    }

    /// 回复里有多个调用时只取第一个。带回的消息只留这一个调用，思考内容照常带回。
    #[test]
    fn native_keeps_only_the_first_call() {
        let r = response_now(
            json!({"choices": [{"message": {
            "role": "assistant", "content": null, "reasoning_content": "想",
            "tool_calls": [
                {"id": "call_a", "type": "function", "function": {
                    "name": "query_stats", "arguments": "{\"date_from\":\"2026-07-01\"}"}},
                {"id": "call_b", "type": "function", "function": {
                    "name": "get_timeline", "arguments": "{}"}}
            ]}}]}),
            Some(ToolProtocol::Native),
        )
        .unwrap();
        let call = r.call.unwrap();
        assert_eq!(call.id, "call_a");
        assert_eq!(call.name, "query_stats");
        assert_eq!(call.arguments["date_from"], "2026-07-01");
        let carried = r.carry.unwrap().0;
        assert_eq!(carried["tool_calls"].as_array().unwrap().len(), 1);
        assert_eq!(carried["reasoning_content"], "想", "思考内容要带回");
    }

    /// 服务没给 id 时按序号编一个，带回的消息里也写上同一个 id。
    #[test]
    fn native_call_without_id_gets_a_numbered_id() {
        let r = response_now(
            json!({"choices": [{"message": {"tool_calls": [
                {"function": {"name": "query_day", "arguments": "{}"}}]}}]}),
            Some(ToolProtocol::Native),
        )
        .unwrap();
        assert_eq!(r.call.unwrap().id, "call_3");
        assert_eq!(r.carry.unwrap().0["tool_calls"][0]["id"], "call_3");
    }

    /// 参数不是 JSON 时给空对象，调用照常返回。
    #[test]
    fn native_bad_arguments_become_an_empty_object() {
        let r = response_now(
            json!({"choices": [{"message": {"tool_calls": [
                {"id": "c1", "function": {"name": "search_text", "arguments": "{keywords: 不是JSON"}}]}}]}),
            Some(ToolProtocol::Native),
        )
        .unwrap();
        assert_eq!(r.call.unwrap().arguments, json!({}));
        assert_eq!(r.usage.input_tokens, None, "没报用量就是 None");
    }

    /// 空正文按原因给错误码。有思考内容时，错误码总是 LLM_EMPTY_REASONING。
    #[test]
    fn empty_text_gets_a_code_by_cause() {
        let mut reasoning = content("", "length", json!({"completion_tokens": 512}));
        reasoning["choices"][0]["message"]["reasoning_content"] = json!("长思考");
        assert_eq!(empty_code(reasoning), "LLM_EMPTY_REASONING");
        assert_eq!(
            empty_code(content("", "stop", json!({"completion_tokens": 0}))),
            "LLM_EMPTY_EOS"
        );
        assert_eq!(
            empty_code(content("", "length", json!({"completion_tokens": 300}))),
            "LLM_EMPTY_TRUNCATED"
        );
        assert_eq!(
            empty_code(content("  ", "stop", Value::Null)),
            "LLM_EMPTY",
            "没有用量时分不出原因"
        );
        assert_eq!(empty_code(json!({})), "LLM_EMPTY", "没有 choices");
    }

    /// 错误文字以错误码开头，前端按它显示原因。
    #[test]
    fn empty_error_text_starts_with_the_code() {
        let err = response_now(
            content(
                "",
                "stop",
                json!({"prompt_tokens": 800, "completion_tokens": 0}),
            ),
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(err.starts_with("[LLM_EMPTY_EOS] empty content"), "{err}");
    }

    /// Grammar 方式下，`action` 是 answer 时是回答，否则是调用，调用的 id 按序号编。
    #[test]
    fn grammar_reads_answer_or_call() {
        let answer = content(
            r#"{"action":"answer","answer":" 好 "}"#,
            "stop",
            Value::Null,
        );
        let r = response_now(answer, Some(ToolProtocol::Grammar)).unwrap();
        assert_eq!(r.text, "好");
        assert!(r.call.is_none());

        let call = content(
            r#"{"action":"search_text","keywords":["报销"]}"#,
            "stop",
            Value::Null,
        );
        let r = response_now(call, Some(ToolProtocol::Grammar)).unwrap();
        let call = r.call.unwrap();
        assert_eq!(call.id, "call_3");
        assert_eq!(call.name, "search_text");
        assert_eq!(call.arguments, json!({"keywords": ["报销"]}));
    }

    /// Grammar 方式下，输出不是 JSON 时是 Provider 错误，`answer` 为空时是 Empty 错误。
    #[test]
    fn grammar_rejects_bad_json_and_empty_answer() {
        let bad = content("不是 JSON", "stop", Value::Null);
        assert!(matches!(
            response_now(bad, Some(ToolProtocol::Grammar)),
            Err(ModelError::Provider(_))
        ));
        let blank = content(r#"{"action":"answer","answer":"  "}"#, "stop", Value::Null);
        assert!(matches!(
            response_now(blank, Some(ToolProtocol::Grammar)),
            Err(ModelError::Empty { .. })
        ));
    }
}
