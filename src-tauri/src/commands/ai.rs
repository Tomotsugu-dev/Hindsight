//! AI 子系统的 Tauri 命令薄壳。
//!
//! 业务逻辑（如真正的 LLM 调用）会在 `crate::ai` 模块里实现，
//! 这里只做参数校验 / 错误归类 / 返回对前端友好的形状。

use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Deserialize)]
struct ModelsResp {
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
}

/// 测试 OpenAI 兼容端点的连通性：`GET {endpoint}/models`。
///
/// `endpoint` 末尾的 `/` 会被吃掉再拼路径，避免出现 `//models`。
/// `api_key` 非空时走 Bearer auth；Ollama 一般不用填。
#[tauri::command]
pub async fn test_ai_endpoint(
    endpoint: String,
    api_key: Option<String>,
) -> Result<TestAiEndpointResp, String> {
    let trimmed = endpoint.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Ok(fail("服务地址为空"));
    }
    let url = format!("{}/models", trimmed);

    let client = match Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(e) => return Ok(fail(&format!("HTTP 客户端构造失败：{e}"))),
    };

    let mut req = client.get(&url);
    if let Some(k) = api_key.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        req = req.bearer_auth(k);
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return Ok(fail(&format!("连接失败：{e}"))),
    };

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        // 截短服务返回，避免把整个 HTML 错误页粘进 toast
        let preview: String = body.chars().take(120).collect();
        return Ok(fail(&format!("服务返回 {status}：{preview}")));
    }

    let parsed: ModelsResp = match resp.json().await {
        Ok(p) => p,
        Err(e) => return Ok(fail(&format!("响应不是 OpenAI 兼容格式：{e}"))),
    };

    let models: Vec<String> = parsed
        .data
        .into_iter()
        .map(|m| m.id)
        .take(10)
        .collect();

    Ok(TestAiEndpointResp {
        ok: true,
        models,
        message: String::new(),
    })
}

fn fail(msg: &str) -> TestAiEndpointResp {
    TestAiEndpointResp {
        ok: false,
        models: Vec::new(),
        message: msg.to_string(),
    }
}
