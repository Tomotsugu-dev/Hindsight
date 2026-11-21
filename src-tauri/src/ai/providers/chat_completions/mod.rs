//! Calls models through the Chat Completions protocol.
//! Cloud services and the local llama-server all use this protocol.

mod dialect;
mod protocol;
mod reply;
mod request;

use std::time::Instant;

use reqwest::header::HeaderMap;
use reqwest::StatusCode;

use self::dialect::Dialect;
use crate::ai::model::{Capabilities, LanguageModelOps, Message, ModelError, Request, Response};
use crate::ai::transport::{
    bearer_headers, fmt_send_err, ModelTransport, TransportError, TransportPolicy,
};
use crate::error::{Error, Result};

pub(crate) use protocol::{model_ids, ChatCompletions, CHAT_COMPLETIONS_PATH, MODELS_PATH};

/// A model that uses the Chat Completions protocol.
#[derive(Clone)]
pub struct ChatCompletionsModel {
    transport: ModelTransport,
    model: String,
    dialect: Dialect,
    capabilities: Capabilities,
}

impl ChatCompletionsModel {
    /// Creates a cloud model.
    ///
    /// `endpoint` is the part of the URL before `/chat/completions`.
    /// When `api_key` is empty, no auth header is sent.
    /// `provider` is the dialect key from [`CloudConnection::dialect`](crate::ai::config::CloudConnection::dialect).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] when `endpoint` or `model` is empty.
    pub fn cloud(endpoint: &str, model: &str, api_key: &str, provider: &str) -> Result<Self> {
        if endpoint.trim().trim_end_matches('/').is_empty() || model.trim().is_empty() {
            return Err(Error::InvalidInput("云端 API 地址或模型 ID 为空"));
        }
        Ok(Self {
            transport: ModelTransport::new(
                endpoint,
                bearer_headers(api_key)?,
                TransportPolicy::cloud(),
            )?,
            model: model.to_string(),
            dialect: Dialect::for_provider(provider),
            capabilities: Capabilities { on_device: false },
        })
    }

    /// Creates a model on the local llama-server.
    pub fn local(port: u16, model: &str) -> Result<Self> {
        Ok(Self {
            transport: ModelTransport::new(
                &format!("http://127.0.0.1:{port}/v1"),
                HeaderMap::new(),
                TransportPolicy::local(),
            )?,
            model: model.to_string(),
            dialect: Dialect::LlamaServer,
            capabilities: Capabilities { on_device: true },
        })
    }
}

impl LanguageModelOps for ChatCompletionsModel {
    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    async fn generate(&self, req: Request) -> std::result::Result<Response, ModelError> {
        let tools = (!req.tools.is_empty()).then_some(self.dialect.tool_protocol());
        let earlier_calls = req
            .messages
            .iter()
            .filter(|m| matches!(m, Message::Assistant { call: Some(_), .. }))
            .count();
        let body = request::body(&self.model, self.dialect, tools, &req);
        let started = Instant::now();
        let reply = self
            .transport
            .post(CHAT_COMPLETIONS_PATH, &body, &ChatCompletions)
            .await
            .map_err(into_model_error)?;
        reply::response(&reply, tools, earlier_calls + 1, started)
    }
}

/// Turns a transport failure into a [`ModelError`].
/// The error text keeps at most 300 characters of the response body.
fn into_model_error(e: TransportError) -> ModelError {
    let preview = |body: &str| body.chars().take(300).collect::<String>();
    match e {
        TransportError::Send(e) => ModelError::Transport(fmt_send_err(e)),
        TransportError::Status { status, body } => {
            let msg = format!("HTTP {status}: {}", preview(&body));
            if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                ModelError::Auth(msg)
            } else {
                ModelError::Provider(msg)
            }
        }
        TransportError::RateLimited { body, .. } => ModelError::RateLimited(format!(
            "HTTP {}: {}",
            StatusCode::TOO_MANY_REQUESTS,
            preview(&body)
        )),
        TransportError::Body(msg) => ModelError::Transport(msg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::model::{Reasoning, ToolCall, ToolSpec};
    use crate::ai::transport::fake::{FakeServer, Reply};
    use serde_json::json;

    fn ask(text: &str) -> Request {
        Request {
            system: "s".into(),
            messages: vec![Message::User {
                text: text.into(),
                images: Vec::new(),
            }],
            tools: Vec::new(),
            reasoning: Reasoning::Default,
            max_output_tokens: 1024,
            temperature: None,
        }
    }

    fn cloud(server: &FakeServer) -> ChatCompletionsModel {
        ChatCompletionsModel::cloud(&format!("{}/", server.base_url()), "m", "sk-test", "custom")
            .unwrap()
    }

    #[test]
    fn cloud_rejects_empty_endpoint_or_model() {
        assert!(ChatCompletionsModel::cloud(" / ", "m", "", "custom").is_err());
        assert!(ChatCompletionsModel::cloud("http://x/v1", " ", "", "custom").is_err());
    }

    /// 云端请求去掉地址末尾的 `/`，带鉴权头。流式回复拼成正文。
    #[tokio::test]
    async fn cloud_reads_a_streamed_reply() {
        let server = FakeServer::start(vec![Reply::sse(
            &[
                json!({"choices": [{"delta": {"content": "好"}}]}),
                json!({"choices": [], "usage": {"prompt_tokens": 9, "completion_tokens": 3}}),
            ],
            true,
        )])
        .await;
        let r = cloud(&server).generate(ask("q")).await.unwrap();
        assert_eq!(r.text, "好");
        assert_eq!(r.usage.input_tokens, Some(9));
        let got = server.received();
        assert!(
            got[0].head.starts_with("post /v1/chat/completions"),
            "{}",
            got[0].head
        );
        assert!(got[0].head.contains("authorization: bearer sk-test"));
    }

    /// 服务没给 id 时，编出的 id 序号是对话里已有的调用数加一。
    #[tokio::test]
    async fn call_ids_count_earlier_calls() {
        let call = json!({"index": 0, "function": {"name": "get_timeline", "arguments": "{}"}});
        let server = FakeServer::start(vec![Reply::sse(
            &[json!({"choices": [{"delta": {"tool_calls": [call]}}]})],
            true,
        )])
        .await;
        let mut req = ask("q");
        req.tools = vec![ToolSpec {
            name: "get_timeline".into(),
            description: String::new(),
            parameters: json!({"type": "object", "properties": {}}),
        }];
        req.messages.push(Message::Assistant {
            text: String::new(),
            call: Some(ToolCall {
                id: "call_1".into(),
                name: "get_timeline".into(),
                arguments: json!({}),
            }),
            carry: None,
        });
        req.messages.push(Message::ToolResult {
            call_id: "call_1".into(),
            content: "结果".into(),
        });
        let r = cloud(&server).generate(req).await.unwrap();
        assert_eq!(r.call.unwrap().id, "call_2");
    }

    /// 401 是鉴权错误。错误文字里有状态码和服务返回的原文。
    #[tokio::test]
    async fn auth_failure_carries_status_and_message() {
        let server = FakeServer::start(vec![Reply::json(
            401,
            json!({"error": {"message": "Invalid API key provided"}}),
        )])
        .await;
        let err = cloud(&server).generate(ask("q")).await.unwrap_err();
        assert!(matches!(err, ModelError::Auth(_)), "{err:?}");
        let text = err.to_string();
        assert!(text.contains("401") && text.contains("Invalid API key provided"));
    }

    /// 重试用完还是 429 时报限流，错误文字里有服务返回的原文。
    #[tokio::test]
    async fn rate_limit_after_retries() {
        let limited = || Reply::json(429, json!({"error": {"message": "rate limited"}}));
        let server = FakeServer::start((0..6).map(|_| limited()).collect()).await;
        let err = cloud(&server).generate(ask("q")).await.unwrap_err();
        assert!(matches!(err, ModelError::RateLimited(_)), "{err:?}");
        assert!(err.to_string().contains("429") && err.to_string().contains("rate limited"));
    }

    /// 长错误体按字符截到 300 个字，切在汉字上也不出错。
    #[tokio::test]
    async fn long_error_body_is_cut_at_300_chars() {
        let body = format!("{}错误详情继续", "x".repeat(299));
        let server = FakeServer::start(vec![Reply::html(500, &body)]).await;
        let err = cloud(&server).generate(ask("q")).await.unwrap_err();
        assert!(matches!(err, ModelError::Provider(_)), "{err:?}");
        let text = err.to_string();
        assert!(text.contains("500") && text.contains("错"), "{text}");
        assert!(!text.contains("误"), "{text}");
    }

    /// 连不上服务时，错误文字写出连接失败。
    #[tokio::test]
    async fn connection_refused_is_a_transport_error() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let model =
            ChatCompletionsModel::cloud(&format!("http://127.0.0.1:{port}/v1"), "m", "", "custom")
                .unwrap();
        let err = model.generate(ask("q")).await.unwrap_err();
        assert!(
            matches!(&err, ModelError::Transport(m) if m.starts_with("connection failed")),
            "{err:?}"
        );
    }

    /// 本机的请求不流式。遇到 400 不改请求，也不重发。
    #[tokio::test]
    async fn local_sends_once_without_stream() {
        let server = FakeServer::start(vec![Reply::json(
            400,
            json!({"error": {"message": "Unknown parameter: 'temperature'."}}),
        )])
        .await;
        let model = ChatCompletionsModel::local(server.port, "m.gguf").unwrap();
        let err = model.generate(ask("q")).await.unwrap_err();
        assert!(err.to_string().contains("400"), "{err}");
        let got = server.received();
        assert_eq!(got.len(), 1);
        assert!(got[0].body.get("stream").is_none());
    }
}
