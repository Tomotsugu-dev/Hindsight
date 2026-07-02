//! 外部 OpenAI 兼容端点的连通性测试命令。
//!
//! 用户在「AI 设置 → 外部模型」里填 endpoint / api_key 之后点"测试连接"，
//! 触发 `test_ai_endpoint` 这条命令；返回成功时附最多 10 个可用模型 ID。
//!
//! 该模块同时提供 [`fmt_send_err`] —— 把 reqwest::Error 的整条 source chain
//! 拼成对用户可读的中文错误，被 `crate::ai::llm` 的本地 chat 路径复用。

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
    data: Vec<OpenAIModelEntry>,
}

/// OpenAI `/v1/models` 响应里的一项；只为本地解析用，跟
/// 本地磁盘上的 [`crate::ai::models::ModelEntry`] 是两个概念
#[derive(Debug, Deserialize)]
struct OpenAIModelEntry {
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

    let client = match Client::builder().timeout(Duration::from_secs(8)).build() {
        Ok(c) => c,
        Err(e) => return Ok(fail(&format!("HTTP 客户端构造失败：{e}"))),
    };

    let mut req = client.get(&url);
    if let Some(k) = api_key.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        req = req.bearer_auth(k);
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return Ok(fail(&fmt_send_err(e))),
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

    let models: Vec<String> = parsed.data.into_iter().map(|m| m.id).take(10).collect();

    Ok(TestAiEndpointResp {
        ok: true,
        models,
        message: String::new(),
    })
}

/// 把 reqwest 发送错误格式化成对用户可读的字符串。
///
/// reqwest::Error 的 Display 只给最外层（"error sending request for url ..."），
/// 真正告诉用户"为啥连不上"的是 source chain 最深处的 io::Error
/// （`Connection refused`、`os error 10061` 等）。这里把整条链拼起来，
/// 并按 reqwest 提供的分类给一句开场白。
pub(crate) fn fmt_send_err(e: reqwest::Error) -> String {
    let head = if e.is_timeout() {
        "请求超时"
    } else if e.is_connect() {
        "连接失败（确认服务是否启动、端口是否正确）"
    } else if e.is_request() {
        "请求构造失败"
    } else {
        "网络错误"
    };

    // 跳过 reqwest::Error 自己（它的 to_string 跟 URL 重复了），从 source 起拼
    let mut details: Vec<String> = Vec::new();
    let mut cur: Option<&dyn std::error::Error> = std::error::Error::source(&e);
    while let Some(s) = cur {
        details.push(s.to_string());
        cur = s.source();
    }

    if details.is_empty() {
        head.to_string()
    } else {
        format!("{head}：{}", details.join(" → "))
    }
}

fn fail(msg: &str) -> TestAiEndpointResp {
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
    let trimmed = endpoint.trim().trim_end_matches('/');
    if trimmed.is_empty() || model.trim().is_empty() {
        return Ok(fail("服务地址或模型 ID 为空"));
    }
    let url = format!("{}/chat/completions", trimmed);

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
    let client = match Client::builder().timeout(Duration::from_secs(30)).build() {
        Ok(c) => c,
        Err(e) => return Ok(fail(&format!("HTTP 客户端构造失败：{e}"))),
    };
    let mut req = client.post(&url).json(&body);
    if let Some(k) = api_key.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        req = req.bearer_auth(k);
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return Ok(fail(&fmt_send_err(e))),
    };
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let preview: String = body.chars().take(200).collect();
        return Ok(fail(&format!("服务返回 {status}：{preview}")));
    }
    Ok(TestAiEndpointResp {
        ok: true,
        models: Vec::new(),
        message: String::new(),
    })
}
