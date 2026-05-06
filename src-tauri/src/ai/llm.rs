//! OpenAI 兼容 chat completions 客户端，专为本地 llama-server 调优（Phase 1B-γ）。
//!
//! - [`ChatClient::new`] 用引擎当前端口构造客户端
//! - [`ChatClient::chat_with_images`] 发送一条 multimodal 请求（system + user text + N 张 image_url）
//!
//! 错误格式化复用 [`crate::commands::ai::fmt_send_err`]，统一错误链给用户看。
//!
//! 不做流式：γ 阶段每段一次性出文，简单可靠。流式留给后续优化。

use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{Error, Result};

/// 单段推理超时。GPU（CUDA / Metal）上一段几秒就完，但 CPU fallback 上
/// 一段 30 张图能要 60-120s，统一给到 180s 留宽裕；比 supervisor 健康检查
/// (90s) 长，避免引擎刚 ready 就被 chat 超时打回。
const CHAT_TIMEOUT: Duration = Duration::from_secs(180);

/// llama-server 的 chat 客户端。
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

    /// 发一条 multimodal chat 请求。
    ///
    /// `image_data_uris` 每项是 `data:image/jpeg;base64,...` 格式，由
    /// [`crate::ai::image::to_data_uri`] 生成。0 张图也合法——纯文本对话。
    ///
    /// 返回模型 `choices[0].message.content` 字符串；服务端格式不对、内容为空
    /// 都会返 Err，让上层把该段标 status='error'。
    pub async fn chat_with_images(
        &self,
        system: &str,
        user_text: &str,
        image_data_uris: &[String],
    ) -> Result<String> {
        // user content 是数组：先一项 text，再 N 项 image_url
        let mut user_content: Vec<serde_json::Value> = Vec::with_capacity(image_data_uris.len() + 1);
        user_content.push(json!({ "type": "text", "text": user_text }));
        for uri in image_data_uris {
            user_content.push(json!({
                "type": "image_url",
                "image_url": { "url": uri }
            }));
        }

        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user",   "content": user_content },
            ],
            // 不开流式；llama-server 会一次性返完整 response
            "stream": false,
            // 让模型有充足空间写段落；ctx_size 是 4096，留 ~2k 给输入图文
            "max_tokens": 768,
            // 0.4 偏稳定，避免空话 / 重复
            "temperature": 0.4,
        });

        let url = format!("{}/chat/completions", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&body)
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
            return Err(Error::Other(format!(
                "服务返回 {status}：{preview}"
            )));
        }

        let parsed: ChatResp = resp
            .json()
            .await
            .map_err(|e| Error::Other(format!("响应不是 OpenAI 兼容格式：{e}")))?;

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
        Ok(content)
    }
}

/// 只取 OpenAI 响应里 `choices[0].message.content`。其它字段（usage / finish_reason）
/// 不关心，serde 自动忽略。
#[derive(Debug, Deserialize)]
struct ChatResp {
    choices: Vec<ChatChoice>,
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
