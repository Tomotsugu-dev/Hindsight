//! 聊天调模型的入口。每一步，模型要么调一个工具，要么作答。

use serde_json::Value;

use crate::ai::model::{
    Carry, LanguageModel, LanguageModelOps, Message, Reasoning, Request, ToolCall, Usage,
};
use crate::ai::providers::chat_completions::ChatCompletionsModel;
use crate::ai::tools::Tools;
use crate::error::Result;

/// 一次模型调用的 token 用量。服务没报的项记 0。
#[derive(Debug, Clone, Copy, Default)]
pub struct TokenUsage {
    pub prompt: u64,
    pub completion: u64,
    /// 思考用的 token，已经算在 `completion` 里。
    pub reasoning: u64,
}

impl From<Usage> for TokenUsage {
    fn from(u: Usage) -> Self {
        Self {
            prompt: u.input_tokens.unwrap_or(0),
            completion: u.output_tokens.unwrap_or(0),
            reasoning: u.reasoning_tokens.unwrap_or(0),
        }
    }
}

/// 一步的结果。
#[derive(Debug)]
pub enum StepOut {
    /// `carry` 原样放进下一步的 [`Turn::AssistantCall`]。
    Call {
        name: String,
        args: Value,
        id: String,
        carry: Option<Carry>,
    },
    Final(String),
}

/// 对话里的一条。
#[derive(Debug, Clone)]
pub enum Turn {
    User(String),
    AssistantText(String),
    AssistantCall {
        id: String,
        name: String,
        args: Value,
        carry: Option<Carry>,
    },
    ToolResult {
        id: String,
        content: String,
    },
}

impl Turn {
    fn to_message(&self) -> Message {
        match self {
            Self::User(text) => Message::User {
                text: text.clone(),
                images: Vec::new(),
            },
            Self::AssistantText(text) => Message::Assistant {
                text: text.clone(),
                call: None,
                carry: None,
            },
            Self::AssistantCall {
                id,
                name,
                args,
                carry,
            } => Message::Assistant {
                text: String::new(),
                call: Some(ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: args.clone(),
                }),
                carry: carry.clone(),
            },
            Self::ToolResult { id, content } => Message::ToolResult {
                call_id: id.clone(),
                content: content.clone(),
            },
        }
    }
}

/// 把设置 `ai.chat_thinking` 读成思考档位。认不出的值是 `Default`。
pub fn reasoning_from_setting(s: &str) -> Reasoning {
    match s {
        "on" | "high" => Reasoning::High,
        "max" => Reasoning::Max,
        "medium" => Reasoning::Medium,
        "low" => Reasoning::Low,
        "off" => Reasoning::Off,
        _ => Reasoning::Default,
    }
}

/// 一步作答的 token 上限，见 [`Request::max_output_tokens`]。
const MAX_ANSWER_TOKENS: u32 = 1024;

/// 聊天用的模型。
pub struct ChatLlm {
    model: LanguageModel,
    reasoning: Reasoning,
}

impl ChatLlm {
    /// 打开云端模型。前四个参数同 [`ChatCompletionsModel::cloud`]。
    pub fn cloud(
        endpoint: &str,
        model: &str,
        api_key: &str,
        provider: &str,
        reasoning: Reasoning,
    ) -> Result<Self> {
        Ok(Self {
            model: LanguageModel::ChatCompletions(ChatCompletionsModel::cloud(
                endpoint, model, api_key, provider,
            )?),
            reasoning,
        })
    }

    /// 打开本机 llama-server 上的模型。
    pub fn local(port: u16, model: &str, reasoning: Reasoning) -> Result<Self> {
        Ok(Self {
            model: LanguageModel::ChatCompletions(ChatCompletionsModel::local(port, model)?),
            reasoning,
        })
    }

    /// 不带工具问一次，返回回复的正文。`max_tokens` 是回答的 token 上限，见 [`Request::max_output_tokens`]。
    pub async fn complete(
        &self,
        system: &str,
        user: &str,
        max_tokens: u32,
    ) -> Result<(String, TokenUsage)> {
        let req = Request {
            system: system.to_string(),
            messages: vec![Message::User {
                text: user.to_string(),
                images: Vec::new(),
            }],
            tools: Vec::new(),
            reasoning: self.reasoning,
            max_output_tokens: max_tokens,
            temperature: Some(0.0),
        };
        let r = self.model.generate(req).await?;
        Ok((r.text, r.usage.into()))
    }

    /// 跑一步：模型调一个工具，或者作答。
    pub async fn step(&self, system: &str, turns: &[Turn]) -> Result<(StepOut, TokenUsage)> {
        let req = Request {
            system: system.to_string(),
            messages: turns.iter().map(Turn::to_message).collect(),
            tools: Tools::specs(),
            reasoning: self.reasoning,
            max_output_tokens: MAX_ANSWER_TOKENS,
            temperature: None,
        };
        let r = self.model.generate(req).await?;
        let out = match r.call {
            Some(call) => StepOut::Call {
                name: call.name,
                args: call.arguments,
                id: call.id,
                carry: r.carry,
            },
            None => StepOut::Final(r.text),
        };
        Ok((out, r.usage.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 分档以前的旧值 "on" 当高档；认不出的值回 Default。
    #[test]
    fn reasoning_from_setting_reads_old_and_unknown_values() {
        assert_eq!(reasoning_from_setting("on"), Reasoning::High);
        assert_eq!(reasoning_from_setting("max"), Reasoning::Max);
        assert_eq!(reasoning_from_setting("off"), Reasoning::Off);
        assert_eq!(reasoning_from_setting("怪值"), Reasoning::Default);
    }
}

/// 从网络这一侧锁住聊天发请求、读回复的行为。
#[cfg(test)]
mod http_tests {
    use super::*;
    use crate::ai::transport::fake::{FakeServer, Reply};
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// 拿一个刚释放的本地端口——连接必然被拒,模拟"服务没起来/地址配错"。
    fn free_local_port() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        port
    }

    /// 起一个一次性 HTTP 假服务:读完整个请求(headers + Content-Length body)
    /// 再按给定状态行/响应体回包,并把收到的原始请求全文送回测试侧。
    /// `status_line` 形如 "200 OK" / "401 Unauthorized"。
    async fn spawn_http_once(
        status_line: &str,
        body: String,
    ) -> (u16, tokio::sync::oneshot::Receiver<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let status_line = status_line.to_string();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf: Vec<u8> = Vec::new();
            let mut tmp = [0u8; 4096];
            loop {
                let n = sock.read(&mut tmp).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                    let cl = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if buf.len() >= pos + 4 + cl {
                        break;
                    }
                }
            }
            let resp = format!(
                "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status_line,
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            let _ = sock.shutdown().await;
            let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
        });
        (port, rx)
    }

    fn cloud_client(port: u16) -> ChatLlm {
        ChatLlm::cloud(
            // 带尾斜杠:顺带验证不会拼出 //chat/completions
            &format!("http://127.0.0.1:{port}/v1/"),
            "test-model",
            "sk-test",
            "custom",
            Reasoning::Default,
        )
        .unwrap()
    }

    fn ask() -> Vec<Turn> {
        vec![Turn::User("q".into())]
    }

    /// deepseek + Off 的请求体带上 thinking.type=disabled。
    #[tokio::test]
    async fn step_cloud_sends_thinking_field_for_deepseek() {
        let body = json!({
            "choices": [{"message": {"role": "assistant", "content": "好的"}}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1}
        })
        .to_string();
        let (port, rx) = spawn_http_once("200 OK", body).await;
        let llm = ChatLlm::cloud(
            &format!("http://127.0.0.1:{port}/v1"),
            "test-model",
            "",
            "deepseek",
            Reasoning::Off,
        )
        .unwrap();
        llm.step("s", &[Turn::User("问".into())]).await.unwrap();
        let raw = rx.await.unwrap();
        assert!(
            raw.contains(r#""thinking":{"type":"disabled"}"#),
            "请求体应含 deepseek 官方关闭字段,实际:{raw}"
        );
    }

    /// 作答:content trim、usage 透传;空的 tool_calls 数组不算调用。
    #[tokio::test]
    async fn step_cloud_final_content_trims_and_reads_usage() {
        let body = json!({
            "choices": [{
                "message": {"role": "assistant", "content": "  今天主要在写测试。 ", "tool_calls": []},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 42, "completion_tokens": 17}
        })
        .to_string();
        let (port, _rx) = spawn_http_once("200 OK", body).await;
        let (out, usage) = cloud_client(port)
            .step("系统提示", &[Turn::User("我今天干了啥".into())])
            .await
            .unwrap();
        match out {
            StepOut::Final(text) => assert_eq!(text, "今天主要在写测试。"),
            other => panic!("空 tool_calls 数组应走作答分支,实际: {other:?}"),
        }
        assert_eq!((usage.prompt, usage.completion), (42, 17));
    }

    /// 调用:name/args/id 逐项解析;多个 tool_calls 只执行第一个,带回的消息只留第一个。
    #[tokio::test]
    async fn step_cloud_tool_call_parses_first_and_truncates_carry() {
        let body = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {"id": "call_1", "type": "function", "function": {
                            "name": "query_stats",
                            "arguments": "{\"date_from\":\"2026-07-01\",\"date_to\":\"2026-07-25\"}"
                        }},
                        {"id": "call_2", "type": "function", "function": {
                            "name": "get_timeline", "arguments": "{}"
                        }}
                    ]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 100, "completion_tokens": 30}
        })
        .to_string();
        let (port, _rx) = spawn_http_once("200 OK", body).await;
        let (out, usage) = cloud_client(port)
            .step("s", &[Turn::User("统计一下".into())])
            .await
            .unwrap();
        let StepOut::Call {
            name,
            args,
            id,
            carry,
        } = out
        else {
            panic!("有 tool_calls 应走调用分支");
        };
        assert_eq!(name, "query_stats");
        assert_eq!(args["date_from"], "2026-07-01");
        assert_eq!(args["date_to"], "2026-07-25");
        assert_eq!(id, "call_1");
        let carried = carry.expect("云端必须带回整条消息").0;
        assert_eq!(carried["tool_calls"].as_array().unwrap().len(), 1);
        assert_eq!(carried["tool_calls"][0]["id"], "call_1");
        assert_eq!((usage.prompt, usage.completion), (100, 30));
    }

    /// arguments 不是合法 JSON 时退化为空对象,照常走调用分支;缺 usage 按 0 计。
    #[tokio::test]
    async fn step_cloud_malformed_arguments_fall_back_to_empty_object() {
        let body = json!({
            "choices": [{
                "message": {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "c1", "type": "function", "function": {
                        "name": "search_text", "arguments": "{keywords: 不是JSON"
                    }}
                ]},
                "finish_reason": "tool_calls"
            }]
        })
        .to_string();
        let (port, _rx) = spawn_http_once("200 OK", body).await;
        let (out, usage) = cloud_client(port)
            .step("s", &[Turn::User("搜".into())])
            .await
            .unwrap();
        let StepOut::Call { name, args, .. } = out else {
            panic!("坏 arguments 仍应走调用分支");
        };
        assert_eq!(name, "search_text");
        assert_eq!(args, json!({}));
        assert_eq!((usage.prompt, usage.completion), (0, 0), "缺 usage 按 0 计");
    }

    /// 请求:system 在首位,四种 Turn 各自写成 Chat Completions 的消息,
    /// 带 carry 的调用原样带回,鉴权头和 tools 都在。
    #[tokio::test]
    async fn step_cloud_renders_turns_and_auth_into_request() {
        let body = json!({
            "choices": [{"message": {"role": "assistant", "content": "好"}, "finish_reason": "stop"}]
        })
        .to_string();
        let (port, rx) = spawn_http_once("200 OK", body).await;
        let carried = json!({
            "role": "assistant",
            "reasoning_content": "思考过程",
            "tool_calls": [{"id": "cR", "type": "function",
                "function": {"name": "get_timeline", "arguments": "{}"}}]
        });
        let turns = vec![
            Turn::User("昨天下午在干嘛".into()),
            Turn::AssistantCall {
                id: "cR".into(),
                name: "get_timeline".into(),
                args: json!({}),
                carry: Some(Carry(carried.clone())),
            },
            Turn::ToolResult {
                id: "cR".into(),
                content: "时间线结果".into(),
            },
            Turn::AssistantText("初步答案".into()),
            Turn::AssistantCall {
                id: "c9".into(),
                name: "search_text".into(),
                args: json!({"keywords": ["报销"]}),
                carry: None,
            },
            Turn::ToolResult {
                id: "c9".into(),
                content: "搜索结果".into(),
            },
        ];
        cloud_client(port).step("系统人设", &turns).await.unwrap();

        let req = rx.await.unwrap();
        let head_end = req.find("\r\n\r\n").unwrap();
        let head = req[..head_end].to_lowercase();
        assert!(
            head.starts_with("post /v1/chat/completions"),
            "尾斜杠应被去掉: {head}"
        );
        assert!(head.contains("authorization: bearer sk-test"));
        let body: Value = serde_json::from_str(&req[head_end + 4..]).unwrap();
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["tool_choice"], "auto");
        // Default 也给足思考预算:DeepSeek 默认思考,1024 会被思考链打满
        assert_eq!(body["max_completion_tokens"], 12_288);
        assert!(body.get("max_tokens").is_none());
        assert_eq!(
            body["tools"].as_array().unwrap().len(),
            Tools::specs().len()
        );
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 7, "system + 6 条 turn");
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "系统人设");
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[2], carried, "带 carry 的调用必须逐字带回");
        assert_eq!(msgs[3]["role"], "tool");
        assert_eq!(msgs[3]["tool_call_id"], "cR");
        assert_eq!(msgs[3]["content"], "时间线结果");
        assert_eq!(msgs[4]["role"], "assistant");
        assert_eq!(msgs[4]["content"], "初步答案");
        assert_eq!(msgs[5]["tool_calls"][0]["id"], "c9");
        assert_eq!(msgs[5]["tool_calls"][0]["function"]["name"], "search_text");
        assert_eq!(msgs[6]["tool_call_id"], "c9");
    }

    /// 401:错误里有状态码和服务端原话。
    #[tokio::test]
    async fn step_cloud_401_error_carries_status_and_server_message() {
        let (port, _rx) = spawn_http_once(
            "401 Unauthorized",
            r#"{"error":{"message":"Invalid API key provided"}}"#.into(),
        )
        .await;
        let err = cloud_client(port)
            .step("s", &ask())
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("401"), "缺状态码: {err}");
        assert!(
            err.contains("Invalid API key provided"),
            "缺服务端原话: {err}"
        );
    }

    /// 429 退避重试 5 次仍限流、500 不重试：两种都把状态码和服务端原话报给用户。
    #[tokio::test]
    async fn step_cloud_429_and_500_pass_through_readably() {
        let limited = || Reply::json(429, json!({"error": {"message": "rate limited"}}));
        let server = FakeServer::start((0..6).map(|_| limited()).collect()).await;
        let err = cloud_client(server.port)
            .step("s", &ask())
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("429") && err.contains("rate limited"),
            "429 信息不可读: {err}"
        );
        assert_eq!(server.received().len(), 6, "首发 1 次 + 重试 5 次");

        let (port, _rx) =
            spawn_http_once("500 Internal Server Error", "上游模型服务不可用".into()).await;
        let err = cloud_client(port)
            .step("s", &ask())
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("500") && err.contains("上游模型服务不可用"),
            "500 信息不可读: {err}"
        );
    }

    /// 长错误体按字符截到 300;切在汉字上也不 panic。
    #[tokio::test]
    async fn step_cloud_long_error_body_preview_truncates_on_char_boundary() {
        let body = "a".repeat(400);
        let (port, _rx) = spawn_http_once("500 Internal Server Error", body).await;
        let err = cloud_client(port)
            .step("s", &ask())
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("500"), "应报 500: {err}");
        assert!(
            err.contains(&"a".repeat(300)) && !err.contains(&"a".repeat(301)),
            "预览应恰为前 300 字符: {err}"
        );

        let tricky = format!("{}错误详情继续", "x".repeat(299));
        let (port, _rx) = spawn_http_once("500 Internal Server Error", tricky).await;
        let err = cloud_client(port)
            .step("s", &ask())
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("错"), "第 300 个字符(错)应被保留: {err}");
        assert!(!err.contains("误"), "第 301 个字符起应被截断: {err}");
    }

    /// 2xx 但响应体不是 JSON:重试 2 次后报解析失败。
    #[tokio::test]
    async fn step_cloud_non_json_body_reports_parse_error() {
        let page = || Reply::html(200, "<html>Bad Gateway</html>");
        let server = FakeServer::start(vec![page(), page(), page()]).await;
        let err = cloud_client(server.port)
            .step("s", &ask())
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("cannot read or parse the response body"),
            "畸形体应报解析错误: {err}"
        );
        assert_eq!(server.received().len(), 3, "首发 1 次 + 重试 2 次");
    }

    /// choices 整个缺失、content 只有空白:都报空回复的错误码。
    #[tokio::test]
    async fn step_cloud_missing_choices_or_blank_content_reports_empty() {
        for body in [json!({}).to_string(), json!({
            "choices": [{"message": {"role": "assistant", "content": "   "}, "finish_reason": "stop"}]
        })
        .to_string()]
        {
            let (port, _rx) = spawn_http_once("200 OK", body.clone()).await;
            let err = cloud_client(port)
                .step("s", &ask())
                .await
                .unwrap_err()
                .to_string();
            assert!(err.contains("[LLM_EMPTY]"), "body={body} 应报空回复: {err}");
        }
    }

    /// 拒连:错误写出连接失败。
    #[tokio::test]
    async fn step_cloud_connection_refused_maps_to_connection_failed() {
        let port = free_local_port();
        let err = cloud_client(port)
            .step("s", &ask())
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("connection failed"), "{err}");
    }

    /// 挂死不回包的服务:500ms 内仍在等,不能早退成假成功或假错误。
    #[tokio::test]
    async fn step_cloud_hanging_server_stays_pending() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = cloud_client(port);
        let turns = ask();
        let r = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            client.step("s", &turns),
        )
        .await;
        assert!(r.is_err(), "不回包时 500ms 内不应有任何结果");
        drop(listener);
    }

    /// 云端回 400 并写明该换哪个字段名：改名后重发，第二次成功。
    #[tokio::test]
    async fn chat_cloud_heals_400_then_succeeds() {
        let server = FakeServer::start(vec![
            Reply::json(
                400,
                json!({"error": {"message": "Unsupported parameter: 'max_completion_tokens' is not supported with this model. Use 'max_tokens' instead."}}),
            ),
            Reply::json(
                200,
                json!({"choices": [{"message": {"role": "assistant", "content": "好"}, "finish_reason": "stop"}]}),
            ),
        ])
        .await;
        let (out, _) = cloud_client(server.port).step("s", &ask()).await.unwrap();
        assert!(matches!(out, StepOut::Final(ref t) if t == "好"), "{out:?}");
        let got = server.received();
        assert_eq!(got.len(), 2, "只重发一次");
        assert!(got[1].body.get("max_completion_tokens").is_none());
        assert_eq!(got[1].body["max_tokens"], 12_288);
    }

    /// 停止按钮丢弃生成的 future：连接随之断开，服务端看得到。
    #[tokio::test]
    async fn chat_dropping_step_closes_connection() {
        let server = FakeServer::start(vec![Reply::Hang]).await;
        let llm = cloud_client(server.port);
        let turns = ask();
        let r = tokio::time::timeout(std::time::Duration::from_millis(300), llm.step("s", &turns))
            .await;
        assert!(r.is_err(), "服务端不回包时不该有结果");
        assert!(
            server
                .wait_disconnect(std::time::Duration::from_secs(2))
                .await,
            "丢弃 future 后连接应断开"
        );
    }

    /// 云端请求流式；工具调用的碎片拼回一条，id 和参数都在，用量取最后一块。
    #[tokio::test]
    async fn chat_cloud_streams_and_joins_tool_call() {
        let call = |args: &str, head: bool| {
            let mut c = json!({"index": 0, "function": {"arguments": args}});
            if head {
                c["id"] = json!("call_1");
                c["function"]["name"] = json!("query_day");
            }
            json!({"choices": [{"delta": {"tool_calls": [c]}}]})
        };
        let server = FakeServer::start(vec![Reply::sse(
            &[
                call("{\"date\":", true),
                call("\"2026-01-02\"}", false),
                json!({"choices": [], "usage": {"prompt_tokens": 7, "completion_tokens": 3}}),
            ],
            true,
        )])
        .await;
        let (out, usage) = cloud_client(server.port).step("s", &ask()).await.unwrap();
        let StepOut::Call {
            name,
            args,
            id,
            carry,
        } = out
        else {
            panic!("应是工具调用: {out:?}");
        };
        assert_eq!(name, "query_day");
        assert_eq!(args, json!({"date": "2026-01-02"}));
        assert_eq!(id, "call_1");
        assert_eq!(carry.unwrap().0["tool_calls"][0]["id"], "call_1");
        assert_eq!((usage.prompt, usage.completion), (7, 3));
        let got = server.received();
        assert_eq!(got[0].body["stream"], true);
        assert_eq!(got[0].body["stream_options"]["include_usage"], true);
    }

    /// 流式工具调用没带 id：按序号编一个，带回的消息里也是这个 id。
    #[tokio::test]
    async fn chat_cloud_stream_tool_call_without_id_gets_a_numbered_id() {
        let call = json!({"index": 0, "function": {"name": "query_day", "arguments": "{}"}});
        let server = FakeServer::start(vec![Reply::sse(
            &[json!({"choices": [{"delta": {"tool_calls": [call]}}]})],
            true,
        )])
        .await;
        let (out, _) = cloud_client(server.port).step("s", &ask()).await.unwrap();
        let StepOut::Call { id, carry, .. } = out else {
            panic!("应是工具调用: {out:?}");
        };
        assert_eq!(id, "call_1");
        assert_eq!(carry.unwrap().0["tool_calls"][0]["id"], "call_1");
    }

    /// 网络块切在一个汉字中间：拼出来的正文不能有乱码。
    #[tokio::test]
    async fn chat_cloud_stream_split_inside_a_character() {
        let server = FakeServer::start(vec![Reply::sse(
            &[json!({"choices": [{"delta": {"content": "上午"}}]})],
            true,
        )
        .split_inside("上")])
        .await;
        let (out, _) = cloud_client(server.port).step("s", &ask()).await.unwrap();
        assert!(
            matches!(out, StepOut::Final(ref t) if t == "上午"),
            "{out:?}"
        );
    }

    /// 云端限流时按退避重发，限流解除就正常作答。
    #[tokio::test]
    async fn chat_cloud_backs_off_on_429_then_succeeds() {
        let server = FakeServer::start(vec![
            Reply::json(429, json!({"error": {"message": "slow down"}})),
            Reply::sse(&[json!({"choices": [{"delta": {"content": "好"}}]})], true),
        ])
        .await;
        let (out, _) = cloud_client(server.port).step("s", &ask()).await.unwrap();
        assert!(matches!(out, StepOut::Final(ref t) if t == "好"), "{out:?}");
        assert_eq!(server.received().len(), 2);
    }

    /// 本机不流式、不重发：500 一次就报错；工具走 json_schema。
    #[tokio::test]
    async fn chat_local_sends_once_without_stream() {
        let server = FakeServer::start(vec![Reply::json(500, json!({"error": "boom"}))]).await;
        let err = ChatLlm::local(server.port, "m", Reasoning::Off)
            .unwrap()
            .step("s", &ask())
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("500") && err.contains("boom"), "{err}");
        let got = server.received();
        assert_eq!(got.len(), 1);
        assert!(got[0].body.get("stream").is_none());
        assert!(got[0].body.get("tools").is_none());
        assert!(got[0].body.get("json_schema").is_some());
    }

    /// 本机的决策 JSON:调用的 id 按序号编,作答取 answer。
    #[tokio::test]
    async fn chat_local_reads_grammar_decisions() {
        let decision = |content: &str| {
            Reply::json(
                200,
                json!({"choices": [{"message": {"role": "assistant", "content": content}}]}),
            )
        };
        let server = FakeServer::start(vec![
            decision(r#"{"action":"search_text","keywords":["报销"]}"#),
            decision(r#"{"action":"answer","answer":"找到了"}"#),
        ])
        .await;
        let llm = ChatLlm::local(server.port, "m", Reasoning::Default).unwrap();
        let (out, _) = llm.step("s", &ask()).await.unwrap();
        let StepOut::Call { name, args, id, .. } = out else {
            panic!("应是工具调用: {out:?}");
        };
        assert_eq!((name.as_str(), id.as_str()), ("search_text", "call_1"));
        assert_eq!(args, json!({"keywords": ["报销"]}));
        let (out, _) = llm.step("s", &ask()).await.unwrap();
        assert!(
            matches!(out, StepOut::Final(ref t) if t == "找到了"),
            "{out:?}"
        );
    }
}
