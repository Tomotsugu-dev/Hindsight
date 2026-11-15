//! OpenAI 兼容协议的共用件：请求路径、输出预算字段名、流式回复、400 请求自愈。

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::ai::transport::Protocol;

/// Chat Completions 协议。发请求时交给传输层。
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

/// 输出预算的字段名。云端用 OpenAI 现行的 `max_completion_tokens`:实测
/// (2026-08)gpt-5.6 起对 `max_tokens` 直接 400「not supported with this
/// model」,而 DeepSeek/Moonshot 两个名字都认——统一发新名字是唯一通吃解。
/// 本地 llama-server 沿用 `max_tokens`。
pub(crate) fn budget_key(is_cloud: bool) -> &'static str {
    if is_cloud {
        "max_completion_tokens"
    } else {
        "max_tokens"
    }
}

// ── 流式回复 ────────────────────────────────────────────────────
//
// 把 Chat Completions 的流式回复拼成和非流式一样形状的 JSON。
//
// 调用方不用分流式和非流式：两种回复拿到手是同一个形状，
// `choices[0].message` 里有 `content`、`reasoning_content`、`tool_calls`，外加 `usage`。

/// 把一个流式回复里每个事件的 `data` 拼起来。
///
/// # Errors
///
/// 没收到 `[DONE]`。错误里写着已经收到多少字，用来分辨「一个字没来」和「快写完了才断」。
fn join_stream(events: &[String]) -> Result<Value, String> {
    let mut acc = SseAccumulator::default();
    for data in events {
        acc.push_data(data);
    }
    if !acc.done {
        return Err(format!(
            "流式响应未正常结束（已收到 {} 字符，缺 [DONE]）",
            acc.content_chars()
        ));
    }
    Ok(acc.finish())
}

/// 一行一行吃进流，攒出整条回复。
#[derive(Debug, Default)]
struct SseAccumulator {
    content: Option<String>,
    reasoning: Option<String>,
    /// 按 `index` 排好的工具调用
    tool_calls: BTreeMap<u64, ToolCallParts>,
    /// 最后一次出现的用量
    usage: Option<Value>,
    /// 收到 `data: [DONE]` 才算完整
    done: bool,
    /// 收到的片段数。服务商不回用量时，拿它当输出的 token 数
    delta_count: u32,
}

/// 一个工具调用的各个片段，拼到最后才完整。
#[derive(Debug, Default)]
struct ToolCallParts {
    id: Option<String>,
    kind: Option<String>,
    name: Option<String>,
    arguments: Option<String>,
}

impl ToolCallParts {
    /// 没收到的字段不写，和非流式回复一致。`type` 没收到时写 `function`。
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

fn append(slot: &mut Option<String>, s: &str) {
    slot.get_or_insert_with(String::new).push_str(s);
}

impl SseAccumulator {
    fn content_chars(&self) -> usize {
        self.content.as_deref().map_or(0, |s| s.chars().count())
    }

    /// 吃一个事件的 `data`。解析不了的跳过。
    fn push_data(&mut self, data: &str) {
        if data == "[DONE]" {
            self.done = true;
            return;
        }
        let Ok(chunk) = serde_json::from_str::<Value>(data) else {
            return;
        };
        // 有的服务商每块都带用量，而且是累计值：取最后一次，不相加
        let usage = &chunk["usage"];
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

    /// 拼一个工具调用的片段。字符串字段接在后面，`type` 直接覆盖，和 OpenAI 官方 SDK 的拼法一样。
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

    /// 拼成非流式回复的形状。
    ///
    /// `finish_reason` 固定写 `"stop"`：有的服务商在流里给 `null`，流走完就是正常结束。
    /// 没有用量时，输出的 token 数用片段数代替，输入记 0。
    fn finish(self) -> Value {
        let has_calls = !self.tool_calls.is_empty();
        // 和非流式回复一致：没有正文时，有工具调用写 null，没有工具调用写空字符串
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
        let usage = self.usage.unwrap_or_else(
            || json!({ "prompt_tokens": 0, "completion_tokens": self.delta_count }),
        );
        json!({
            "choices": [{ "message": message, "finish_reason": "stop" }],
            "usage": usage,
        })
    }
}

// ── 请求自愈 ────────────────────────────────────────────────────
//
// 云端 400 绝大多数是"参数不合这家/这个模型的口味",而**错误信息本身就写明
// 了该怎么改**。与其为每个 provider 硬编码知识(追不上 API 演进,更盖不住
// custom 端点的无穷组合),不如按对方的说法改一改重发。OpenAI 兼容生态的
// 错误措辞高度同构,同一套规则对没见过的端点一样有效。
//
// 规则与安全边界经 `scripts/llm/selfheal_probe.py` 验证:mock 侧复现真机
// 抓到的 400 原文(8 场景全自愈)、边界侧 4 场景全部按预期干净放弃、
// 真端点 DeepSeek×2 / OpenAI / Moonshot 端到端通过。

/// 自愈重试上限。够用:实测最长的链是"改名 → 补 none"两轮。
pub(crate) const MAX_HEAL_ROUNDS: u32 = 3;

/// 绝不可为了让请求通过而删掉的字段。删了确实能 200,但功能已经废了——
/// 例如删 `tools` 后 Chat 失去查库能力却装作一切正常,静默失效比报错更坏。
const PROTECTED_FIELDS: [&str; 4] = ["model", "messages", "tools", "tool_choice"];

/// 思考控制字段全集(各家口径不同,撤销时一并清掉)。
const THINKING_FIELDS: [&str; 4] = [
    "thinking",
    "reasoning",
    "reasoning_effort",
    "chat_template_kwargs",
];

/// 取错误信息里第 `n` 个单引号包裹的片段(0 起)。
fn quoted(msg: &str, n: usize) -> Option<&str> {
    msg.split('\'').nth(n * 2 + 1)
}

/// 取 `start` 与 `end` 之间的片段。
fn between<'a>(msg: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let rest = &msg[msg.find(start)? + start.len()..];
    Some(rest[..rest.find(end)?].trim())
}

fn remove_field(body: &mut Value, key: &str) -> bool {
    if PROTECTED_FIELDS.contains(&key) {
        return false;
    }
    body.as_object_mut().and_then(|o| o.remove(key)).is_some()
}

/// 按 400 的错误信息就地修正请求体。返回 false = 没有规则能修,该放弃。
pub(crate) fn heal_request(body: &mut Value, err_msg: &str) -> bool {
    let low = err_msg.to_ascii_lowercase();

    // R2 参数改名:对方直接给了新名字(`'max_tokens' ... use 'max_completion_tokens' instead`)
    if low.contains("is not supported") && low.contains("instead") {
        if let (Some(old), Some(new)) = (quoted(err_msg, 0), quoted(err_msg, 1)) {
            if let Some(o) = body.as_object_mut() {
                if let Some(v) = o.remove(old) {
                    o.insert(new.to_string(), v);
                    return true;
                }
            }
        }
    }

    // R3 值不被支持:降到错误信息里列出的第一个合法值,拿不到就删字段
    if low.contains("does not support") {
        if let Some(param) = quoted(err_msg, 0) {
            if body.get(param).is_some() {
                // `Supported values are: 'none', 'low', ...` —— 取第一个
                let fallback = low
                    .find("supported values")
                    .and_then(|i| quoted(&err_msg[i..], 0).map(str::to_string));
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

    // R4 组合不被支持(`Function tools with reasoning_effort are not supported`):
    // 把被点名的参数压到安全值。注意"根本没发这个参数"也会被拒——OpenAI 带
    // tools 时默认档就非 none,所以要**补**上而不只是改。
    if let Some(param) = between(&low, "with ", " are not supported") {
        if param == "reasoning_effort" {
            if body.get(param).and_then(Value::as_str) != Some("none") {
                body["reasoning_effort"] = json!("none");
                return true;
            }
        } else if remove_field(body, param) {
            return true;
        }
    }

    // R5 模型强制思考(Moonshot `only type=enabled is allowed for this model`,
    // OpenRouter `Reasoning is mandatory ...`):撤掉全部思考控制,按默认行为跑
    if low.contains("only type=enabled")
        || (low.contains("reasoning") && low.contains("mandatory"))
        || low.contains("cannot be disabled")
    {
        let mut hit = false;
        for f in THINKING_FIELDS {
            hit |= remove_field(body, f);
        }
        if hit {
            return true;
        }
    }

    // R1 未知参数:最通用的一条——对方不认识的字段直接删。受保护字段除外
    // (端点若连 tools 都不认,应当干净失败,让用户知道它不能用于对话)
    if low.contains("unknown parameter") {
        if let Some(p) = quoted(err_msg, 0) {
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

    /// 基本形态:多块拼接 + [DONE] 收尾。
    #[test]
    fn sse_joins_deltas_and_ends_on_done() {
        let resp = join_stream(&events(&[
            r#"{"choices":[{"delta":{"role":"assistant","content":"你"}}]}"#,
            r#"{"choices":[{"delta":{"content":"好"}}]}"#,
            "[DONE]",
        ]))
        .unwrap();
        assert_eq!(resp["choices"][0]["message"]["content"], "你好");
        // finish_reason 恒为 stop:SiliconFlow 末块实测回 null,转发会让下游误判
        assert_eq!(resp["choices"][0]["finish_reason"], "stop");
    }

    /// DeepSeek 形态:content 与 reasoning_content 显式 null 交替出现。
    /// 分开累计才能沿用非流式那套空回复归因（思考烧完没写答案 = LLM_EMPTY_REASONING）。
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

    /// SiliconFlow 形态:usage 每块都带且是**累计值** —— 取最后一次，不是累加。
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

    /// 服务商不回 usage 时用自数的块数近似 completion_tokens（只用于展示）。
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
        assert_eq!(u["prompt_tokens"], 0, "自数拿不到 prompt 侧，如实填 0");
    }

    /// 解析不了的块跳过:厂商偶尔插自有格式的块 —— 丢一块比丢整个回答划算。
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

    /// 没收到 [DONE] 就是断流。join_stream 据此判失败而不是留半截 ——
    /// 半截答案会被原样存进 ai_summaries / 聊天记录，看着像正常内容。
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

    /// 工具调用分多块到达:id 和名字在第一块,参数一段一段接上;按 index 分开。
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

    /// 思考加工具调用(DeepSeek 思考模式):两样都要留着,回放时要原样带回。
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

    /// 自愈规则表。用例的错误文案全部取自真机实测原文
    /// (scripts/llm/thinking_probe.py + selfheal 原型),不是编的。
    #[test]
    fn heal_request_rules_cover_real_world_400s() {
        let e400 = |s: &str| format!("HTTP 400 Bad Request: {{\"error\":{{\"message\":\"{s}\"}}}}");

        // R2 参数改名:OpenAI gpt-5.6 起拒收 max_tokens
        let mut b = json!({"model": "m", "max_tokens": 1024});
        assert!(heal_request(
            &mut b,
            &e400("Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead.")
        ));
        assert_eq!(b["max_completion_tokens"], 1024);
        assert!(b.get("max_tokens").is_none());

        // R1 未知参数:切服务商后残留的别家字段
        let mut b = json!({"model": "m", "thinking": {"type": "disabled"}});
        assert!(heal_request(
            &mut b,
            &e400("Unknown parameter: 'thinking'.")
        ));
        assert!(b.get("thinking").is_none());

        // R3 值不被支持:降到错误信息里列出的第一个合法值
        let mut b = json!({"model": "m", "reasoning_effort": "minimal"});
        assert!(heal_request(
            &mut b,
            &e400("Unsupported value: 'reasoning_effort' does not support 'minimal' with this model. Supported values are: 'none', 'low', 'medium', 'high'.")
        ));
        assert_eq!(b["reasoning_effort"], "none");

        // R4 组合不被支持:参数**不在报文里**也要能补上(OpenAI 默认档非 none)
        let mut b = json!({"model": "m", "tools": []});
        assert!(heal_request(
            &mut b,
            &e400("Function tools with reasoning_effort are not supported for gpt-5.6-luna in /v1/chat/completions.")
        ));
        assert_eq!(b["reasoning_effort"], "none");
        // 已经是 none 还报同样的错 → 无计可施,不能空转
        assert!(!heal_request(
            &mut b,
            &e400("Function tools with reasoning_effort are not supported for gpt-5.6-luna.")
        ));

        // R5 模型强制思考(Moonshot kimi-k2.7-code):撤掉全部思考控制
        let mut b = json!({"model": "m", "reasoning_effort": "none", "tools": []});
        assert!(heal_request(
            &mut b,
            &e400("invalid thinking: only type=enabled is allowed for this model")
        ));
        assert!(b.get("reasoning_effort").is_none());
        assert!(b.get("tools").is_some(), "撤思考不得波及 tools");

        // 无法识别的错误:干净放弃,不瞎改
        let mut b = json!({"model": "m", "max_tokens": 1});
        assert!(!heal_request(&mut b, &e400("内部错误 E5012")));
        assert_eq!(b["max_tokens"], 1);
    }

    /// 安全边界:能"修好"反而是事故的场景。
    /// 删掉 tools 请求确实会 200,但 Chat 从此查不了库还装作正常——
    /// 静默失效比报错更坏,这里必须拒绝修复。
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
}
