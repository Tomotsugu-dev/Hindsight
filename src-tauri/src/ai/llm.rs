//! OpenAI 兼容 chat completions 客户端，专为本地 llama-server 调优（Phase 1B-γ）。
//!
//! - [`ChatClient::new`] 用引擎当前端口构造客户端
//! - [`ChatClient::chat_with_images`] 发送一条 multimodal 请求（system + user text + N 张 image_url）
//!
//! 错误格式化复用 [`crate::commands::ai_endpoint::fmt_send_err`]，统一错误链给用户看。
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

/// 本地 llama-server 单段推理超时。
/// 段总结路径单段可能拼 26 张图描述聚合（5K-15K input token）+ 几千 token 输出，
/// Apple Silicon Metal 跑 4B Q4 模型实测分钟级别——给到 600s 容忍长 prompt 长输出。
/// 比 supervisor 健康检查 (90s) 长，避免引擎刚 ready 就被 chat 超时打回。
const CHAT_TIMEOUT: Duration = Duration::from_secs(600);

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
    /// 单次响应 max_tokens 上限。caller 按用户配的 ctx_size_per_slot / 2 算
    /// （给 prompt 留另一半）；ctx=8K → 4K，ctx=64K → 32K。
    max_tokens: u32,
    http: Client,
}

impl ChatClient {
    /// `port` 来自 [`crate::ai::server::EngineSupervisor::status()`] 返回的端口；
    /// `model` 直接传 `settings.ai.active_main`（含 .gguf 后缀也行）；
    /// `max_tokens` 由 caller 按 effective ctx_size 折半算，让用户的"上下文（每路）64K"
    /// 设置真能反映到单次响应能写多长。
    pub fn new(port: u16, model: impl Into<String>, max_tokens: u32) -> Result<Self> {
        let http = Client::builder()
            .timeout(CHAT_TIMEOUT)
            .build()
            .map_err(|e| Error::LlmResponse(format!("HTTP 客户端构造失败：{e}")))?;
        Ok(Self {
            base_url: format!("http://127.0.0.1:{}/v1", port),
            model: model.into(),
            max_tokens: max_tokens.max(512), // 不让 caller 算出过小的 max_tokens 让所有响应都被截断
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
        let body = build_chat_body_local(
            &self.model,
            system,
            user_text,
            image_data_uris,
            self.max_tokens,
        );
        post_chat_completions(self.http.post(&url), body).await
    }
}

/// 外部云端 API 的 OpenAI 兼容 chat 客户端。
///
/// 跟 [`ChatClient`] 走同样的 `/chat/completions` 协议，区别只在：
/// - base URL 是用户填的（`https://api.openai.com/v1` 等）
/// - 带 `Authorization: Bearer <api_key>` 头
///
/// 两个入口，语义分开：
/// - [`Self::chat_text`]：step 2 段总结，**拒绝**任何带图调用（防路由 bug）
/// - [`Self::chat_with_images`]：step 1 图片描述——仅当用户在模型卡上**显式**把
///   Vision (Step 1) 指到云端（`describe_use_cloud`，前端有隐私确认弹窗）才会被
///   构造使用，此时截图会以 data URI 形式上传到用户配置的第三方 API
#[derive(Clone)]
pub struct ExternalChatClient {
    base_url: String,
    model: String,
    api_key: String,
    /// 同 [`ChatClient::max_tokens`]——caller 按 effective ctx_size 折半算
    max_tokens: u32,
    http: Client,
}

impl ExternalChatClient {
    /// `endpoint` 是用户填的 base URL（如 `https://api.openai.com/v1`），
    /// 末尾的 `/` 会被去掉；`model` 是模型 ID（如 `gpt-4o-mini`）；
    /// `api_key` 空字符串视为无鉴权（custom endpoint 可能不需要 key）。
    pub fn new(endpoint: &str, model: String, api_key: String, max_tokens: u32) -> Result<Self> {
        let base_url = endpoint.trim().trim_end_matches('/').to_string();
        if base_url.is_empty() {
            return Err(Error::InvalidInput("云端 API 地址为空"));
        }
        if model.trim().is_empty() {
            return Err(Error::InvalidInput("云端模型 ID 为空"));
        }
        let http = Client::builder()
            .timeout(EXTERNAL_CHAT_TIMEOUT)
            .build()
            .map_err(|e| Error::LlmResponse(format!("HTTP 客户端构造失败：{e}")))?;
        Ok(Self {
            base_url,
            model,
            api_key,
            max_tokens: max_tokens.max(512),
            http,
        })
    }

    /// 发一条纯文本 chat 请求（step 2 段总结专用）。
    /// 拒绝任何 `image_data_uris` 非空的调用——本设计里截图永远不上云。
    pub async fn chat_text(
        &self,
        system: &str,
        user_text: &str,
        image_data_uris: &[String],
    ) -> Result<(String, ChatUsage)> {
        // 防御：本设计里外部 API 只跑 step 2 纯文本；任何带图调用都是路由 bug。
        if !image_data_uris.is_empty() {
            return Err(Error::InvalidInput(
                "云端 API 不接受图片：step 1 必须走本地 vision 模型",
            ));
        }
        let url = format!("{}/chat/completions", self.base_url);
        let body = build_chat_body(&self.model, system, user_text, &[], self.max_tokens, None);
        self.post_with_retry(&url, &body).await
    }

    /// 发一条多模态 chat 请求（step 1 云端图片描述专用）。
    /// 只有 `describe_use_cloud` 路由会构造到这里——用户已在前端确认过
    /// "截图将上传"的隐私弹窗。请求体格式与本地 [`ChatClient::chat_with_images`]
    /// 完全一致（OpenAI 兼容 text + image_url data URI）。
    pub async fn chat_with_images(
        &self,
        system: &str,
        user_text: &str,
        image_data_uris: &[String],
    ) -> Result<(String, ChatUsage)> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = build_chat_body(&self.model, system, user_text, image_data_uris, self.max_tokens, None);
        self.post_with_retry(&url, &body).await
    }

    /// POST + 429 自动退避重试。云端 API 有 RPM 限制（如 Moonshot 低档 20 RPM），
    /// step 1 并发跑图很容易撞 429：优先按服务端 Retry-After 等待，没给则指数
    /// 退避（2/4/8/16/32s），最多 5 次；仍失败才把 429 抛给调用方。
    /// 本地 llama-server 无限流，不走这里。
    async fn post_with_retry(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<(String, ChatUsage)> {
        const RETRY_MAX: u32 = 5;
        let mut attempt = 0u32;
        loop {
            let mut req = self.http.post(url).json(body);
            if !self.api_key.trim().is_empty() {
                req = req.bearer_auth(self.api_key.trim());
            }
            match send_and_classify(req, Instant::now()).await {
                SendOutcome::Done(r) => return r,
                SendOutcome::RateLimited(retry_after) => {
                    attempt += 1;
                    if attempt > RETRY_MAX {
                        return Err(Error::LlmResponse(format!(
                            "服务持续限流（429 Too Many Requests），已退避重试 {RETRY_MAX} 次仍失败——多半是账户 RPM 配额太低，稍后再跑或升级配额"
                        )));
                    }
                    // Retry-After 常给 1s（如 Moonshot），但 RPM 是分钟级滑窗，
                    // 1s 重试大概率再撞。取 max(Retry-After, 指数退避) 稳妥拉开。
                    let exp = Duration::from_secs(1u64 << attempt);
                    let wait = retry_after.map_or(exp, |ra| ra.max(exp));
                    log::info!(
                        "云端 API 限流（429），第 {attempt} 次退避 {}s 后重试",
                        wait.as_secs()
                    );
                    tokio::time::sleep(wait).await;
                }
            }
        }
    }
}

/// step 1 图片描述的 chat 路由。本地走 [`ChatClient`]（llama-server vision 模型），
/// 云端走 [`ExternalChatClient::chat_with_images`]（用户显式选择 + 隐私确认后）。
/// 结构与 [`Step2Chat`] 对齐。
#[derive(Clone)]
pub enum Step1Chat {
    Local(ChatClient),
    External(ExternalChatClient),
}

impl Step1Chat {
    pub async fn chat_with_images(
        &self,
        system: &str,
        user_text: &str,
        image_data_uris: &[String],
    ) -> Result<(String, ChatUsage)> {
        match self {
            Step1Chat::Local(c) => c.chat_with_images(system, user_text, image_data_uris).await,
            Step1Chat::External(c) => c.chat_with_images(system, user_text, image_data_uris).await,
        }
    }

    /// 是否走本地引擎（用于 idle watcher：只有本地调用才 acquire 推理 guard）。
    pub fn is_local(&self) -> bool {
        matches!(self, Step1Chat::Local(_))
    }

    /// 写入 `ai_image_descriptions.model` 的标识——本地用 GGUF 文件名，云端用模型 ID。
    pub fn model_label(&self) -> &str {
        match self {
            Step1Chat::Local(c) => c.model(),
            Step1Chat::External(c) => &c.model,
        }
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
///
/// `max_tokens` 由 caller 按用户配的 ctx_size 折半给（详见函数体注释）。
/// 本地 llama-server 版：temperature 固定 0.4（我们自己调优的稳定值），
/// 并默认关闭思考——图描述 / 段总结是结构化改写任务，不需要推理链。
/// 实测（Gemma 4 E2B）：思考型模型会把 max_tokens 烧在思考上导致 content 空
/// （LLM_EMPTY_REASONING）；带 enable_thinking=false 后 281 token 思考 → 17 token
/// 直出正文。Gemma 4 / Qwen3 系模板认这个开关；不认的模板（如 R1-Distill 这类
/// 纯推理模型）安全忽略（实测未知 kwarg 不报错）。
fn build_chat_body_local(
    model: &str,
    system: &str,
    user_text: &str,
    image_data_uris: &[String],
    max_tokens: u32,
) -> serde_json::Value {
    let mut body =
        build_chat_body(model, system, user_text, image_data_uris, max_tokens, Some(0.4));
    body["chat_template_kwargs"] = json!({ "enable_thinking": false });
    body
}

fn build_chat_body(
    model: &str,
    system: &str,
    user_text: &str,
    image_data_uris: &[String],
    max_tokens: u32,
    temperature: Option<f64>,
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

    let mut body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user",   "content": user_content },
        ],
        "stream": false,
        // max_tokens 跟用户配的 ctx_size 联动（caller 按 ctx_size/2 算，给 prompt 留另一半）：
        // - ctx=8K → max_tokens 4K（普通 instruct 模型也用得完只是不会真生成那么多）
        // - ctx=64K → max_tokens 32K（reasoning 模型思考链 + 答案都有空间）
        // 写死小值（768 / 4096）让 reasoning 模型一律 length 截断 content 空。
        "max_tokens": max_tokens,
    });
    // 本地 llama 固定 0.4 偏稳定（避免空话 / 重复）；云端传 None 不发该字段——
    // 各家约束不同（kimi-k2.5 只收 1，发 0.4 直接 400），厂商默认值最安全。
    if let Some(t) = temperature {
        body["temperature"] = json!(t);
    }
    body
}

/// 已经带好 body / 鉴权头的 RequestBuilder → 发出去 → 解析 ChatResp。
///
/// `t0` 在调用方记录，用来算 latency。失败统一映射为 `Error::Other` + 人类可读字符串。
async fn post_chat_completions(
    req: RequestBuilder,
    body: serde_json::Value,
) -> Result<(String, ChatUsage)> {
    let t0 = Instant::now();
    send_and_parse(req.json(&body), t0).await
}

/// 本地 llama-server 路径用：本地无限流，429（理论不出现）当普通错误。
async fn send_and_parse(req: RequestBuilder, t0: Instant) -> Result<(String, ChatUsage)> {
    match send_and_classify(req, t0).await {
        SendOutcome::Done(r) => r,
        SendOutcome::RateLimited(_) => Err(Error::LlmResponse(
            "服务返回 429 Too Many Requests".to_string(),
        )),
    }
}

/// [`send_and_parse`] 的分类版：把 429 单独拎出来（附 Retry-After 等待时长），
/// 让 [`ExternalChatClient::post_with_retry`] 能做限流退避；其余情况走 Done。
enum SendOutcome {
    Done(Result<(String, ChatUsage)>),
    /// 服务端 429：附 Retry-After 头解析出的等待时长（没给则 None）
    RateLimited(Option<Duration>),
}

async fn send_and_classify(req: RequestBuilder, t0: Instant) -> SendOutcome {
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            return SendOutcome::Done(Err(Error::LlmResponse(
                crate::commands::ai_endpoint::fmt_send_err(e),
            )))
        }
    };

    let status = resp.status();
    if status.as_u16() == 429 {
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        return SendOutcome::RateLimited(retry_after);
    }
    SendOutcome::Done(parse_response(resp, status, t0).await)
}

/// 非 429 的常规收尾：非 2xx 报错；2xx 解析 OpenAI 兼容响应。
async fn parse_response(
    resp: reqwest::Response,
    status: reqwest::StatusCode,
    t0: Instant,
) -> Result<(String, ChatUsage)> {
    if !status.is_success() {
        let preview: String = resp
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect();
        return Err(Error::LlmResponse(format!("服务返回 {status}：{preview}")));
    }

    let parsed: ChatResp = resp
        .json()
        .await
        .map_err(|e| Error::LlmResponse(format!("响应不是 OpenAI 兼容格式：{e}")))?;

    let usage = ChatUsage {
        latency_ms: t0.elapsed().as_millis() as u64,
        prompt_tokens: parsed.usage.as_ref().map(|u| u.prompt_tokens),
        completion_tokens: parsed.usage.as_ref().map(|u| u.completion_tokens),
    };

    let first_choice = parsed.choices.into_iter().next();
    let finish_reason = first_choice
        .as_ref()
        .and_then(|c| c.finish_reason.clone())
        .unwrap_or_else(|| "<none>".to_string());
    let reasoning_chars = first_choice
        .as_ref()
        .and_then(|c| c.message.reasoning_content.as_ref())
        .map(|s| s.chars().count())
        .unwrap_or(0);
    let content = first_choice
        .map(|c| c.message.content)
        .unwrap_or_default()
        .trim()
        .to_string();

    // 标记 [chat-result] 让用户能 grep 出每次 chat 的关键指标——
    // 0 token 输出 + finish_reason=stop = prompt 一进模型就吐 EOS（chat template /
    // mmproj 错配 / 模型不兼容典型征兆）；length = 撞 max_tokens；
    // reasoning_chars > 0 = reasoning 模型思考链占了大头
    log::info!(
        "[chat-result] latency={}ms prompt_tokens={:?} completion_tokens={:?} finish_reason={} content_chars={} reasoning_chars={}",
        usage.latency_ms,
        usage.prompt_tokens,
        usage.completion_tokens,
        finish_reason,
        content.chars().count(),
        reasoning_chars,
    );

    if content.is_empty() {
        // 内容为空分四种成因，各发一个**稳定错误码**（前缀 `[LLM_EMPTY_*]`）；前端按码
        // 显示本地化的"为什么 + 怎么办"，不把 token 术语堆给用户。技术细节（token 数 /
        // finish_reason）已经在上面的 [chat-result] log::info 里，调试看日志即可。
        // 码后面保留一小段英文技术摘要，纯给日志 / 不认识码的兜底用，前端会忽略它。
        //   - LLM_EMPTY_REASONING：reasoning 模型思考链占满 max_tokens，正式答案没机会输出
        //   - LLM_EMPTY_EOS：模型 prompt 一进去就 EOS（chat template / mmproj 错配）
        //   - LLM_EMPTY_TRUNCATED：老版 llama-server 思考链塞 content 撞 max_tokens 被截
        //   - LLM_EMPTY：其它未分类的空响应
        let code = if reasoning_chars > 0 {
            "LLM_EMPTY_REASONING"
        } else if finish_reason == "stop" && usage.completion_tokens == Some(0) {
            "LLM_EMPTY_EOS"
        } else if finish_reason == "length" && usage.completion_tokens.is_some_and(|n| n > 0) {
            "LLM_EMPTY_TRUNCATED"
        } else {
            "LLM_EMPTY"
        };
        return Err(Error::LlmResponse(format!(
            "[{}] empty content (finish_reason={}, prompt_tokens={:?}, completion_tokens={:?}, reasoning_chars={})",
            code, finish_reason, usage.prompt_tokens, usage.completion_tokens, reasoning_chars,
        )));
    }
    Ok((content, usage))
}

/// 取 OpenAI 响应里 `choices[0].message.content` 和 `usage`。
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
    /// "stop" / "length" / "tool_calls" 等；模型主动 EOS 是 "stop"，
    /// completion_tokens=0 + finish_reason="stop" 说明 prompt 一进去模型就吐 EOS
    /// （chat template / mmproj 错配 / 模型不兼容长上下文等典型场景）。
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ChatMessage {
    #[allow(dead_code)]
    role: String,
    content: String,
    /// 新版 llama-server (>= b4500) 跟 OpenAI 兼容 reasoning 模型 (DeepSeek R1 /
    /// Qwen3 thinking) 都会把思考链放这里，正式回答留在 `content`。
    /// 思考链占满 max_tokens 时 content 为空、reasoning_content 非空——典型征兆。
    #[serde(default)]
    reasoning_content: Option<String>,
}
