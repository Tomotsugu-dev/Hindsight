//! 外部 OpenAI 兼容端点的连通性测试命令。
//!
//! 用户在「AI 设置 → 外部模型」里填 endpoint / api_key 之后点"测试连接"，
//! 触发 `test_ai_endpoint` 这条命令；返回成功时附可用模型 ID(上限 500)。
//! 请求经 [`ModelTransport`] 发。

use std::time::Duration;

use reqwest::StatusCode;
use serde::Serialize;

use crate::ai::providers::chat_completions::{
    model_ids, ChatCompletions, CHAT_COMPLETIONS_PATH, MODELS_PATH,
};
use crate::ai::transport::{
    bearer_headers, fmt_send_err, ModelTransport, TransportError, TransportPolicy,
};

/// `test_ai_endpoint` 的返回。
///
/// 任何失败（网络不通、状态码非 2xx、JSON 解析失败、…）都用 `ok=false + message`
/// 表达，**不抛 Err**——这样前端只需要检查一个布尔字段，不用同时处理 invoke
/// 自身的拒绝路径和返回值的成败两套逻辑。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestAiEndpointResp {
    pub ok: bool,
    /// 成功时取响应里的 `data[].id`，最多前 10 个；失败时为空
    pub models: Vec<String>,
    /// 失败时填给用户看的错误描述；成功时为空
    pub message: String,
}

/// 测试 OpenAI 兼容端点的连通性：`GET {endpoint}/models`。
///
/// `api_key` 非空时走 Bearer auth；Ollama 一般不用填。
#[tauri::command]
pub async fn test_ai_endpoint(
    endpoint: String,
    api_key: Option<String>,
) -> Result<TestAiEndpointResp, String> {
    if endpoint.trim().trim_end_matches('/').is_empty() {
        return Ok(fail("服务地址为空"));
    }
    let transport = match probe_transport(&endpoint, api_key.as_deref(), Duration::from_secs(8)) {
        Ok(t) => t,
        Err(msg) => return Ok(fail(&msg)),
    };
    let body = match transport.get(MODELS_PATH).await {
        Ok(body) => body,
        Err(e) => return Ok(fail(&describe(e, 120))),
    };
    match model_ids(body) {
        // 500:够住 OpenRouter 这类聚合站的全目录;"拉取模型"下拉与测试
        // toast 共用这份返回,展示层各自截断
        Ok(ids) => Ok(TestAiEndpointResp {
            ok: true,
            models: ids.into_iter().take(500).collect(),
            message: String::new(),
        }),
        Err(e) => Ok(fail(&format!(
            "cannot read or parse the response body: {e}"
        ))),
    }
}

fn probe_transport(
    endpoint: &str,
    api_key: Option<&str>,
    total_timeout: Duration,
) -> Result<ModelTransport, String> {
    let headers = bearer_headers(api_key.unwrap_or("")).map_err(|e| e.to_string())?;
    ModelTransport::new(endpoint, headers, TransportPolicy::probe(total_timeout))
        .map_err(|e| e.to_string())
}

/// 测试失败时 toast 上的一句话。响应体最多留 `preview` 个字，不把整个 HTML 错误页贴上去。
fn describe(e: TransportError, preview: usize) -> String {
    let cut = |body: String| body.chars().take(preview).collect::<String>();
    match e {
        TransportError::Send(e) => fmt_send_err(e),
        TransportError::Status { status, body } => format!("服务返回 {status}：{}", cut(body)),
        TransportError::RateLimited { body, .. } => {
            format!("服务返回 {}：{}", StatusCode::TOO_MANY_REQUESTS, cut(body))
        }
        TransportError::Body(msg) => msg,
    }
}

pub(crate) fn fail(msg: &str) -> TestAiEndpointResp {
    TestAiEndpointResp {
        ok: false,
        models: Vec::new(),
        message: msg.to_string(),
    }
}

/// 1×1 透明 PNG 的 data URL——`test_ai_chat(with_image=true)` 用它验证模型
/// 真的接受图片输入（纯文本模型会 4xx），比只看 /models 列表可靠。
const TINY_PNG_DATA_URL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

/// 测试模型是否真实可用：`POST {endpoint}/chat/completions` 发一次最小请求
/// （max_tokens=1）。模型 ID 拼错（如 deepseek-v4-flash 写成 ...flash1）会被
/// 服务端 4xx 当场报出来；`with_image=true` 时消息附一张 1×1 PNG，同时验证
/// 多模态能力。返回复用 [`TestAiEndpointResp`]（models 恒空）。
#[tauri::command]
pub async fn test_ai_chat(
    endpoint: String,
    api_key: Option<String>,
    model: String,
    with_image: bool,
) -> Result<TestAiEndpointResp, String> {
    if endpoint.trim().trim_end_matches('/').is_empty() || model.trim().is_empty() {
        return Ok(fail("服务地址或模型 ID 为空"));
    }
    let content = if with_image {
        serde_json::json!([
            { "type": "text", "text": "hi" },
            { "type": "image_url", "image_url": { "url": TINY_PNG_DATA_URL } }
        ])
    } else {
        serde_json::json!("hi")
    };
    let body = serde_json::json!({
        "model": model.trim(),
        "messages": [{ "role": "user", "content": content }],
        "max_tokens": 1,
    });

    // chat 比 /models 慢得多（要真跑一次前向），超时放宽到 30s
    let transport = match probe_transport(&endpoint, api_key.as_deref(), Duration::from_secs(30)) {
        Ok(t) => t,
        Err(msg) => return Ok(fail(&msg)),
    };
    match transport
        .post(CHAT_COMPLETIONS_PATH, &body, &ChatCompletions)
        .await
    {
        Ok(_) => Ok(TestAiEndpointResp {
            ok: true,
            models: Vec::new(),
            message: String::new(),
        }),
        Err(e) => Ok(fail(&describe(e, 200))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::transport::fake::{FakeServer, Reply};
    use serde_json::json;

    /// 测试连接：GET /models，带鉴权头，返回模型 ID。
    #[tokio::test]
    async fn test_ai_endpoint_lists_model_ids() {
        let server = FakeServer::start(vec![Reply::json(
            200,
            json!({"data": [{"id": "a"}, {"id": "b"}]}),
        )])
        .await;
        let r = test_ai_endpoint(server.base_url(), Some("sk-x".into()))
            .await
            .unwrap();
        assert!(r.ok, "{}", r.message);
        assert_eq!(r.models, ["a", "b"]);
        let got = server.received();
        assert!(got[0].head.starts_with("get /v1/models"), "{}", got[0].head);
        assert!(got[0].head.contains("authorization: bearer sk-x"));
    }

    /// 测试模型：非 2xx 时 ok=false，消息里有状态码和服务端的原话。
    #[tokio::test]
    async fn test_ai_chat_reports_status_and_body() {
        let server = FakeServer::start(vec![Reply::json(
            401,
            json!({"error": {"message": "Invalid API key"}}),
        )])
        .await;
        let r = test_ai_chat(server.base_url(), Some("bad".into()), "m".into(), false)
            .await
            .unwrap();
        assert!(!r.ok);
        assert!(
            r.message.contains("401") && r.message.contains("Invalid API key"),
            "{}",
            r.message
        );
        assert_eq!(server.received().len(), 1);
    }

    /// 测试模型：200 就算通过。没填 Key 时不发鉴权头。
    #[tokio::test]
    async fn test_ai_chat_ok_on_200() {
        let server = FakeServer::start(vec![Reply::json(
            200,
            json!({"choices": [{"message": {"role": "assistant", "content": "h"}, "finish_reason": "length"}]}),
        )])
        .await;
        let r = test_ai_chat(server.base_url(), None, "m".into(), false)
            .await
            .unwrap();
        assert!(r.ok, "{}", r.message);
        let got = server.received();
        assert_eq!(got[0].body["messages"][0]["content"], "hi");
        assert!(
            !got[0].head.contains("authorization"),
            "没填 Key 不发鉴权头"
        );
    }

    // ---- 以下锁住改用传输层后的新行为 ----

    /// 测试模型：服务拒收 max_tokens 时跟真跑一样改名再发，算通过。
    #[tokio::test]
    async fn test_ai_chat_heals_400_like_real_runs() {
        let server = FakeServer::start(vec![
            Reply::json(
                400,
                json!({"error": {"message": "Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead."}}),
            ),
            Reply::json(
                200,
                json!({"choices": [{"message": {"role": "assistant", "content": "h"}, "finish_reason": "length"}]}),
            ),
        ])
        .await;
        let r = test_ai_chat(server.base_url(), None, "m".into(), false)
            .await
            .unwrap();
        assert!(r.ok, "{}", r.message);
        let got = server.received();
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].body["max_completion_tokens"], 1);
    }

    /// 测试连接：被限流不等也不重发，照实报 429。
    #[tokio::test]
    async fn test_ai_endpoint_reports_429_without_retry() {
        let server = FakeServer::start(vec![Reply::json(
            429,
            json!({"error": {"message": "slow down"}}),
        )])
        .await;
        let r = test_ai_endpoint(server.base_url(), None).await.unwrap();
        assert!(!r.ok);
        assert!(
            r.message.contains("429") && r.message.contains("slow down"),
            "{}",
            r.message
        );
        assert_eq!(server.received().len(), 1);
    }
}
