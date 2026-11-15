//! Writes a [`Request`] as a Chat Completions request body.

use serde_json::{json, Map, Value};

use super::dialect::Dialect;
use crate::ai::model::{Message, Request, ToolSpec};

/// How tools are given to the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ToolProtocol {
    /// The request sends `tools`, and the model returns calls in `tool_calls`.
    Native,
    /// The conversation is written as one text, and the model can only output a decision object.
    Grammar,
}

/// Returns the request body. When `tools` is `None`, the body has no tools.
pub(super) fn body(
    model: &str,
    dialect: Dialect,
    tools: Option<ToolProtocol>,
    req: &Request,
) -> Value {
    let messages = match tools {
        Some(ToolProtocol::Grammar) => transcript(req),
        _ => native_messages(req),
    };
    let mut body = json!({ "model": model, "messages": messages });
    match tools {
        Some(ToolProtocol::Native) => {
            let specs: Vec<Value> = req.tools.iter().map(tool_json).collect();
            body["tools"] = json!(specs);
            body["tool_choice"] = json!("auto");
        }
        // `json_schema` is a llama-server field. llama-server limits sampling to this schema.
        // See <https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md>.
        Some(ToolProtocol::Grammar) => body["json_schema"] = decision_schema(&req.tools),
        None => {}
    }
    let reasoning = dialect.effective_reasoning(req.reasoning, tools.is_some());
    body[dialect.budget_key()] = json!(dialect.budget(req.max_output_tokens, reasoning));
    dialect.write_reasoning(&mut body, reasoning);
    if dialect.is_cloud() {
        // Without streaming, no data arrives until the answer is complete.
        // If that takes longer than the read timeout of `TransportPolicy::cloud`,
        // the connection breaks.
        body["stream"] = json!(true);
        body["stream_options"] = json!({ "include_usage": true });
    }
    // Some cloud models accept only a fixed temperature, and return an error for other values.
    // For example, Moonshot's kimi-k2.6 accepts only 1 with thinking, and only 0.6 without it.
    // See <https://platform.kimi.ai/docs/api/models-overview>.
    if let Some(t) = req.temperature.filter(|_| !dialect.is_cloud()) {
        body["temperature"] = json!(t);
    }
    body
}

/// Returns the `messages` of `req` for native tool calling.
/// The first message is the system message.
/// Each later message matches one item of `req.messages`.
fn native_messages(req: &Request) -> Vec<Value> {
    let mut out = vec![json!({ "role": "system", "content": req.system })];
    for m in &req.messages {
        out.push(match m {
            Message::User { text, images } => {
                json!({ "role": "user", "content": user_content(text, images) })
            }
            Message::Assistant {
                carry: Some(carry), ..
            } => carry.0.clone(),
            Message::Assistant {
                text,
                call: Some(call),
                carry: None,
            } => {
                let mut msg = json!({
                    "role": "assistant",
                    "tool_calls": [{
                        "id": call.id,
                        "type": "function",
                        "function": { "name": call.name, "arguments": call.arguments.to_string() },
                    }],
                });
                if !text.is_empty() {
                    msg["content"] = json!(text);
                }
                msg
            }
            Message::Assistant {
                text,
                call: None,
                carry: None,
            } => json!({ "role": "assistant", "content": text }),
            Message::ToolResult { call_id, content } => {
                json!({ "role": "tool", "tool_call_id": call_id, "content": content })
            }
        });
    }
    out
}

/// Writes the conversation as one text.
/// Returns the system message and one user message that holds this text.
fn transcript(req: &Request) -> Vec<Value> {
    let mut text = String::new();
    let mut images = Vec::new();
    for m in &req.messages {
        match m {
            Message::User { text: t, images: i } => {
                text.push_str(&format!("用户: {t}\n"));
                images.extend(i.iter().cloned());
            }
            Message::Assistant {
                call: Some(call), ..
            } => text.push_str(&format!(
                "助手(调用工具): {} {}\n",
                call.name, call.arguments
            )),
            Message::Assistant { text: t, .. } => text.push_str(&format!("助手: {t}\n")),
            Message::ToolResult { content, .. } => {
                text.push_str(&format!("工具结果:\n{content}\n"))
            }
        }
    }
    text.push_str("请输出下一步决策(JSON):");
    vec![
        json!({ "role": "system", "content": req.system }),
        json!({ "role": "user", "content": user_content(&text, &images) }),
    ]
}

/// Returns the `content` of a user message.
/// Without images, it is a string. With images, it is an array of text and image parts.
fn user_content(text: &str, images: &[String]) -> Value {
    if images.is_empty() {
        return json!(text);
    }
    let mut parts = vec![json!({ "type": "text", "text": text })];
    for uri in images {
        parts.push(json!({ "type": "image_url", "image_url": { "url": uri } }));
    }
    json!(parts)
}

/// Returns `spec` as one item of the `tools` array in the request.
fn tool_json(spec: &ToolSpec) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": spec.name,
            "description": spec.description,
            "parameters": spec.parameters,
        },
    })
}

// TODO: Grammar mode does not send the tool `description` to the model.
// The decision schema also drops the `required` list of each tool.
// So required arguments are not enforced.
/// Returns the JSON Schema of the decision object in Grammar mode.
///
/// The decision object is flat. `action` is a tool name or `answer`.
/// The other fields are the arguments of all tools.
/// For example, `{"action": "query_stats", "date_from": "2026-11-03", "date_to": "2026-11-09"}`,
/// or `{"action": "answer", "answer": "..."}`.
/// When two tools have an argument with the same name, the first tool's definition is used.
fn decision_schema(tools: &[ToolSpec]) -> Value {
    let mut props = Map::new();
    let mut actions: Vec<&str> = Vec::new();
    for t in tools {
        actions.push(&t.name);
        for (k, v) in t.parameters["properties"].as_object().into_iter().flatten() {
            props.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    actions.push("answer");
    props.insert(
        "action".into(),
        json!({ "type": "string", "enum": actions }),
    );
    props.insert("answer".into(), json!({ "type": "string" }));
    json!({ "type": "object", "properties": props, "required": ["action"] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::model::{Carry, Reasoning, ToolCall};

    fn req(messages: Vec<Message>, tools: Vec<ToolSpec>) -> Request {
        Request {
            system: "系统".into(),
            messages,
            tools,
            reasoning: Reasoning::Default,
            max_output_tokens: 1024,
            temperature: Some(0.4),
        }
    }

    fn user(text: &str) -> Message {
        Message::User {
            text: text.into(),
            images: Vec::new(),
        }
    }

    fn spec(name: &str, props: Value) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: format!("{name} 的说明"),
            parameters: json!({ "type": "object", "properties": props }),
        }
    }

    fn call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: json!({ "keywords": ["报销"] }),
        }
    }

    /// 云端的请求流式、要用量，不发温度。
    #[test]
    fn cloud_streams_and_never_sends_temperature() {
        let b = body("m", Dialect::Generic, None, &req(vec![user("问")], vec![]));
        assert_eq!(b["stream"], true);
        assert_eq!(b["stream_options"]["include_usage"], true);
        assert!(b.get("temperature").is_none(), "{b}");
    }

    /// 本机的请求不流式，发温度。Default 档在本机发关思考。
    #[test]
    fn local_sends_temperature_without_stream() {
        let b = body(
            "m",
            Dialect::LlamaServer,
            None,
            &req(vec![user("问")], vec![]),
        );
        assert!(b.get("stream").is_none());
        assert_eq!(b["temperature"], 0.4);
        assert_eq!(b["max_tokens"], 1024);
        assert_eq!(b["chat_template_kwargs"]["enable_thinking"], false);
    }

    /// 纯文本消息是字符串，带图时是数组。
    #[test]
    fn user_content_is_a_string_without_images() {
        let with_image = Message::User {
            text: "看图".into(),
            images: vec!["data:image/png;base64,AA".into()],
        };
        let b = body(
            "m",
            Dialect::LlamaServer,
            None,
            &req(vec![user("问"), with_image], vec![]),
        );
        let msgs = b["messages"].as_array().unwrap();
        assert_eq!(msgs[1]["content"], "问");
        assert_eq!(msgs[2]["content"][0]["text"], "看图");
        assert_eq!(
            msgs[2]["content"][1]["image_url"]["url"],
            "data:image/png;base64,AA"
        );
    }

    /// 原生工具方式下 system 在首位。带 carry 的助手消息原样带回，没有 carry 的按调用重建。
    #[test]
    fn native_messages_replay_carry_and_rebuild_calls() {
        let carried = json!({
            "role": "assistant",
            "reasoning_content": "思考过程",
            "tool_calls": [{"id": "cR", "type": "function",
                "function": {"name": "get_timeline", "arguments": "{}"}}]
        });
        let messages = vec![
            user("昨天下午在干嘛"),
            Message::Assistant {
                text: String::new(),
                call: Some(call("cR", "get_timeline")),
                carry: Some(Carry(carried.clone())),
            },
            Message::ToolResult {
                call_id: "cR".into(),
                content: "时间线结果".into(),
            },
            Message::Assistant {
                text: "初步答案".into(),
                call: None,
                carry: None,
            },
            Message::Assistant {
                text: String::new(),
                call: Some(call("c9", "search_text")),
                carry: None,
            },
        ];
        let tools = vec![spec("search_text", json!({}))];
        let b = body(
            "m",
            Dialect::Generic,
            Some(ToolProtocol::Native),
            &req(messages, tools),
        );
        assert_eq!(b["tool_choice"], "auto");
        assert_eq!(b["tools"][0]["function"]["name"], "search_text");
        let msgs = b["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[2], carried, "带 carry 的消息逐字带回");
        assert_eq!(msgs[3]["role"], "tool");
        assert_eq!(msgs[3]["tool_call_id"], "cR");
        assert_eq!(msgs[4]["content"], "初步答案");
        let rebuilt = &msgs[5];
        assert!(rebuilt.get("content").is_none(), "{rebuilt}");
        assert_eq!(rebuilt["tool_calls"][0]["id"], "c9");
        assert_eq!(
            rebuilt["tool_calls"][0]["function"]["arguments"],
            r#"{"keywords":["报销"]}"#
        );
    }

    /// Grammar 方式下，对话写进一条 user 消息。请求带决策 schema，不带 `tools`。
    #[test]
    fn grammar_writes_a_transcript_and_a_schema() {
        let messages = vec![
            user("搜一下"),
            Message::Assistant {
                text: String::new(),
                call: Some(call("call_1", "search_text")),
                carry: None,
            },
            Message::ToolResult {
                call_id: "call_1".into(),
                content: "三条结果".into(),
            },
            Message::Assistant {
                text: "好".into(),
                call: None,
                carry: None,
            },
        ];
        let tools = vec![spec("search_text", json!({}))];
        let b = body(
            "m",
            Dialect::LlamaServer,
            Some(ToolProtocol::Grammar),
            &req(messages, tools),
        );
        assert!(b.get("tools").is_none());
        let msgs = b["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(
            msgs[1]["content"],
            "用户: 搜一下\n助手(调用工具): search_text {\"keywords\":[\"报销\"]}\n\
             工具结果:\n三条结果\n助手: 好\n请输出下一步决策(JSON):"
        );
        assert_eq!(b["json_schema"]["required"], json!(["action"]));
    }

    /// 决策 schema 的 `action` 是工具名加 answer。各工具的参数合在一起，同名时取第一个工具的定义。
    #[test]
    fn decision_schema_merges_tool_parameters() {
        let tools = vec![
            spec(
                "search_text",
                json!({ "keywords": {"type": "array"}, "date_from": {"type": "string", "maxLength": 10} }),
            ),
            spec("get_timeline", json!({ "date_from": {"type": "integer"} })),
        ];
        let s = decision_schema(&tools);
        assert_eq!(
            s["properties"]["action"]["enum"],
            json!(["search_text", "get_timeline", "answer"])
        );
        assert_eq!(s["properties"]["keywords"]["type"], "array");
        assert_eq!(s["properties"]["date_from"]["maxLength"], 10);
        assert_eq!(s["properties"]["answer"]["type"], "string");
    }
}
