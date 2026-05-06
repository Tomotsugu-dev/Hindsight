//! OpenAI 兼容 chat completions 客户端，专为本地 llama-server 调优（Phase 1B-γ）。
//!
//! - [`ChatClient::new`] 用引擎当前端口构造客户端
//! - [`ChatClient::chat_with_images`] 发送一条 multimodal 请求（system + user text + N 张 image_url）
//!
//! 错误格式化复用 [`crate::commands::ai::fmt_send_err`]，统一错误链给用户看。
//!
//! 不做流式：γ 阶段每段一次性出文，简单可靠。流式留给后续优化。

use std::time::{Duration, Instant};

use reqwest::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{Error, Result};

/// 单次 chat 调用的性能数据，给调试 tab 显示用。
///
/// `latency_ms` 始终有值；`prompt_tokens` / `completion_tokens` 可能为 None
/// （部分 llama-server 配置 / 模型不返 usage 字段）。
#[derive(Debug, Clone, Default)]
pub struct ChatUsage {
    pub latency_ms: u64,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
}

/// 本地 llama-server 单段推理超时。GPU（CUDA / Metal）上一段几秒就完，但 CPU
/// fallback 上一段 30 张图能要 60-120s，统一给到 180s 留宽裕；比 supervisor
/// 健康检查 (90s) 长，避免引擎刚 ready 就被 chat 超时打回。
const CHAT_TIMEOUT: Duration = Duration::from_secs(180);

/// 外部 API（OpenAI / DeepSeek / OpenRouter…）超时。
/// 云端文本聊天一般 5-30s 完，给 90s 容忍偶发慢响应 + 网络抖动。
const EXTERNAL_CHAT_TIMEOUT: Duration = Duration::from_secs(90);

/// llama-server 的 chat 客户端。
#[derive(Clone)]
pub struct ChatClient {
    base_url: String,
    /// 模型名——llama-server 不强求是真实文件名，可填 "default" / 任意字符串
    /// 都行；这里就拿 active_main 文件名当 ID 方便调试日志区分
    model: String,
    http: Client,
}

impl ChatClient {
    /// `port` 来自 [`crate::ai::server::EngineSupervisor::status()`] 返回的端口；
    /// `model` 直接传 `settings.ai.active_main`（含 .gguf 后缀也行）。
    pub fn new(port: u16, model: impl Into<String>) -> Result<Self> {
        let http = Client::builder()
            .timeout(CHAT_TIMEOUT)
            .build()
            .map_err(|e| Error::Other(format!("HTTP 客户端构造失败：{e}")))?;
        Ok(Self {
            base_url: format!("http://127.0.0.1:{}/v1", port),
            model: model.into(),
            http,
        })
    }

    /// 用于 step2 路由日志 / `Step2Chat::model_label`；本地客户端的 model 是
    /// GGUF 文件名（如 `qwen2.5-vl-7b-instruct-q4_k_m.gguf`）。
    pub fn model(&self) -> &str {
        &self.model
    }

    /// 发一条 multimodal chat 请求。
    ///
    /// `image_data_uris` 每项是 `data:image/jpeg;base64,...` 格式，由
    /// [`crate::ai::image::to_data_uri`] 生成。0 张图也合法——纯文本对话。
    ///
    /// 返回模型 `choices[0].message.content` 字符串 + `ChatUsage` 性能数据。
    /// 服务端格式不对、内容为空都会返 Err，让上层把该段标 status='error'。
    pub async fn chat_with_images(
        &self,
        system: &str,
        user_text: &str,
        image_data_uris: &[String],
    ) -> Result<(String, ChatUsage)> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = build_chat_body(&self.model, system, user_text, image_data_uris);
        post_chat_completions(self.http.post(&url), body).await
    }
}

/// 外部云端 API 的 OpenAI 兼容 chat 客户端（仅用于 step 2 段总结，纯文本）。
///
/// 跟 [`ChatClient`] 走同样的 `/chat/completions` 协议，区别只在：
/// - base URL 是用户填的（`https://api.openai.com/v1` 等）
/// - 带 `Authorization: Bearer <api_key>` 头
/// - 拒绝任何 image_data_uris 非空的调用——本设计里截图永远不上云
#[derive(Clone)]
pub struct ExternalChatClient {
    base_url: String,
    model: String,
    api_key: String,
    http: Client,
}

impl ExternalChatClient {
    /// `endpoint` 是用户填的 base URL（如 `https://api.openai.com/v1`），
    /// 末尾的 `/` 会被去掉；`model` 是模型 ID（如 `gpt-4o-mini`）；
    /// `api_key` 空字符串视为无鉴权（custom endpoint 可能不需要 key）。
    pub fn new(endpoint: &str, model: String, api_key: String) -> Result<Self> {
        let base_url = endpoint.trim().trim_end_matches('/').to_string();
        if base_url.is_empty() {
            return Err(Error::Other("云端 API 地址为空".into()));
        }
        if model.trim().is_empty() {
            return Err(Error::Other("云端模型 ID 为空".into()));
        }
        let http = Client::builder()
            .timeout(EXTERNAL_CHAT_TIMEOUT)
            .build()
            .map_err(|e| Error::Other(format!("HTTP 客户端构造失败：{e}")))?;
        Ok(Self {
            base_url,
            model,
            api_key,
            http,
        })
    }

    pub async fn chat_text(
        &self,
        system: &str,
        user_text: &str,
        image_data_uris: &[String],
    ) -> Result<(String, ChatUsage)> {
        // 防御：本设计里外部 API 只跑 step 2 纯文本；任何带图调用都是路由 bug。
        if !image_data_uris.is_empty() {
            return Err(Error::Other(
                "云端 API 不接受图片：step 1 必须走本地 vision 模型".into(),
            ));
        }
        let url = format!("{}/chat/completions", self.base_url);
        let body = build_chat_body(&self.model, system, user_text, &[]);
        let mut req = self.http.post(&url).json(&body);
        // custom endpoint 可能不要 key；空串视为无鉴权
        if !self.api_key.trim().is_empty() {
            req = req.bearer_auth(self.api_key.trim());
        }
        send_and_parse(req, Instant::now()).await
    }
}

/// step 2 段总结的 chat 路由。本地走 [`ChatClient`]（复用 step 1 引擎），
/// 外部走 [`ExternalChatClient`]。
///
/// 用 enum 而不是 `Box<dyn Trait>` 是为了避免引入 `async-trait` 依赖；
/// `summary.rs` 在构造期判一次 `external_enabled` 就拿到具体变体。
#[derive(Clone)]
pub enum Step2Chat {
    Local(ChatClient),
    External(ExternalChatClient),
}

impl Step2Chat {
    /// step 2 永远是纯文本调用（`image_data_uris` 应该是空数组）；
    /// 这里保留 `image_data_uris` 参数只是为了跟 step 1 调用签名对齐，
    /// 调用方不会真的传图。External 变体会拒绝带图调用。
    pub async fn chat(
        &self,
        system: &str,
        user_text: &str,
        image_data_uris: &[String],
    ) -> Result<(String, ChatUsage)> {
        match self {
            Step2Chat::Local(c) => c.chat_with_images(system, user_text, image_data_uris).await,
            Step2Chat::External(c) => c.chat_text(system, user_text, image_data_uris).await,
        }
    }

    /// 当前 step2 是否走本地引擎（用于 idle watcher：只有本地调用才 acquire 推理 guard）。
    pub fn is_local(&self) -> bool {
        matches!(self, Step2Chat::Local(_))
    }

    /// step 2 实际写入 `ai_summaries.model` 的标识——本地用 GGUF 文件名，
    /// 外部用 provider 上的模型 ID（让导出的 Markdown / DailyTab UI 都能区分）。
    pub fn model_label(&self) -> &str {
        match self {
            Step2Chat::Local(c) => c.model(),
            Step2Chat::External(c) => &c.model,
        }
    }
}

/// 构造 OpenAI 兼容 `/chat/completions` 请求体。
///
/// `image_data_uris` 非空时 user content 走数组形式（text + image_url），
/// 空时走纯字符串——兼容部分 provider（如 DeepSeek）对纯文本只接受字符串。
fn build_chat_body(
    model: &str,
    system: &str,
    user_text: &str,
    image_data_uris: &[String],
) -> serde_json::Value {
    let user_content = if image_data_uris.is_empty() {
        json!(user_text)
    } else {
        let mut arr: Vec<serde_json::Value> = Vec::with_capacity(image_data_uris.len() + 1);
        arr.push(json!({ "type": "text", "text": user_text }));
        for uri in image_data_uris {
            arr.push(json!({
                "type": "image_url",
                "image_url": { "url": uri }
            }));
        }
        json!(arr)
    };

    json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user",   "content": user_content },
        ],
        "stream": false,
        // 让模型有充足空间写段落；本地 ctx 4096 留 ~2k 给输入图文，
        // 云端模型 ctx 都很大不卡这个上限
        "max_tokens": 768,
        // 0.4 偏稳定，避免空话 / 重复
        "temperature": 0.4,
    })
}

/// 已经带好 body / 鉴权头的 RequestBuilder → 发出去 → 解析 ChatResp。
///
/// `t0` 在调用方记录，用来算 latency。失败统一映射为 `Error::Other` + 人类可读字符串。
async fn post_chat_completions(req: RequestBuilder, body: serde_json::Value) -> Result<(String, ChatUsage)> {
    let t0 = Instant::now();
    send_and_parse(req.json(&body), t0).await
}

async fn send_and_parse(req: RequestBuilder, t0: Instant) -> Result<(String, ChatUsage)> {
    let resp = req
        .send()
        .await
        .map_err(|e| Error::Other(crate::commands::ai::fmt_send_err(e)))?;

    let status = resp.status();
    if !status.is_success() {
        let preview: String = resp
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect();
        return Err(Error::Other(format!("服务返回 {status}：{preview}")));
    }

    let parsed: ChatResp = resp
        .json()
        .await
        .map_err(|e| Error::Other(format!("响应不是 OpenAI 兼容格式：{e}")))?;

    let usage = ChatUsage {
        latency_ms: t0.elapsed().as_millis() as u64,
        prompt_tokens: parsed.usage.as_ref().map(|u| u.prompt_tokens),
        completion_tokens: parsed.usage.as_ref().map(|u| u.completion_tokens),
    };

    let content = parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .unwrap_or_default()
        .trim()
        .to_string();

    if content.is_empty() {
        return Err(Error::Other("模型返回为空".to_string()));
    }
    Ok((content, usage))
}

/// 取 OpenAI 响应里 `choices[0].message.content` 和 `usage`。其它字段（finish_reason）
/// 不关心，serde 自动忽略。
#[derive(Debug, Deserialize)]
struct ChatResp {
    choices: Vec<ChatChoice>,
    /// llama-server 一般会返；个别版本不返时这里是 None
    usage: Option<ChatUsageRaw>,
}

#[derive(Debug, Deserialize)]
struct ChatUsageRaw {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize, Serialize)]
struct ChatMessage {
    #[allow(dead_code)]
    role: String,
    content: String,
}
