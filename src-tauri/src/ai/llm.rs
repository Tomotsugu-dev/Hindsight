//! OpenAI 兼容 chat completions 客户端，专为本地 llama-server 调优（Phase 1B-γ）。
//!
//! - [`ChatClient::new`] 用引擎当前端口构造客户端
//! - [`ChatClient::chat_with_images`] 发送一条 multimodal 请求（system + user text + N 张 image_url）
//!
//! 错误格式化复用 [`crate::commands::ai_endpoint::fmt_send_err`]，统一错误链给用户看。
//!
//! 流式只对**云端**开：非流式请求在生成期间连接零字节流动，大段总结要几分钟，
//! 系统 TCP 栈会先于客户端超时把它掐掉（实测 `os error 60`）。本地 llama-server
//! 走 localhost，没有中间设备，保持非流式 —— 一次性出文更简单可靠。
//! 两条路共用 [`send_and_classify`]，按响应的 content-type 分流。

use std::time::{Duration, Instant};

use reqwest::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ai::openai_compat::{budget_key, heal_request, MAX_HEAL_ROUNDS};
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

// 外部 API（OpenAI / DeepSeek / OpenRouter…）的超时口径经历过两轮：
//   1. 整请求超时 90s → 深夜段这类大内容段生成本就超过 90s，被自己掐断；
//   2. 整请求超时 300s → 仍失败，且错误链末尾是 `os error 60`(ETIMEDOUT)：
//      掐连接的是系统 TCP 栈而不是本端计时器，客户端调多大都没用。
// 病根是非流式请求在生成期间连接**零字节流动**。改流式后连接一直有数据，
// 于是整请求超时这个概念本身就不适用了 —— 换成下面这对「建连 + 块间空闲」。

/// 云端建连上限。连不上是秒级可判的事，不需要等满读超时。
const EXTERNAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// 云端流式的**块间**空闲上限（不是整个请求的上限）。
///
/// 流式下服务端持续吐 token，正常间隔在毫秒到数秒之间；连续 60s 一个字节
/// 都没有，基本可判定连接已死。取值要比"模型思考到首个 token 的时间"宽——
/// 推理模型（DeepSeek R1 系）首 token 前会先想一会儿，实测在十几秒量级。
const EXTERNAL_READ_TIMEOUT: Duration = Duration::from_secs(60);

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
    /// `image_data_uris` 每项是 `data:image/jpeg;base64,...` 格式。
    /// 0 张图也合法——纯文本对话（当前所有调用方都只走纯文本）。
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
/// 唯一入口 [`Self::chat_text`]：纯文本调用，**拒绝**任何带图调用——
/// 本设计里截图永远不上云。
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
        // 流式下不能再用 `timeout`（整个请求的上限）：长段生成本来就要几分钟，
        // 那是正常工作而不是卡死。改用 read_timeout —— 它管"两块数据之间最多
        // 等多久"，正好对上病根：只要模型还在吐 token 连接就不算死，真断了
        // 才在一分钟内判失败。connect_timeout 单独兜住"连都连不上"。
        let http = Client::builder()
            .connect_timeout(EXTERNAL_CONNECT_TIMEOUT)
            .read_timeout(EXTERNAL_READ_TIMEOUT)
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

    /// 发一条纯文本 chat 请求。
    /// 拒绝任何 `image_data_uris` 非空的调用——本设计里截图永远不上云。
    pub async fn chat_text(
        &self,
        system: &str,
        user_text: &str,
        image_data_uris: &[String],
    ) -> Result<(String, ChatUsage)> {
        // 防御：本设计里外部 API 只跑纯文本；任何带图调用都是路由 bug。
        if !image_data_uris.is_empty() {
            return Err(Error::InvalidInput("云端 API 不接受图片"));
        }
        let url = format!("{}/chat/completions", self.base_url);
        let body = build_chat_body(
            true,
            &self.model,
            system,
            user_text,
            &[],
            self.max_tokens,
            None,
        );
        self.post_with_retry(&url, &body).await
    }

    /// POST + 429 自动退避重试。云端 API 有 RPM 限制（如 Moonshot 低档 20 RPM）：
    /// 优先按服务端 Retry-After 等待，没给则指数退避（2/4/8/16/32s），最多 5 次；
    /// 仍失败才把 429 抛给调用方。本地 llama-server 无限流，不走这里。
    async fn post_with_retry(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<(String, ChatUsage)> {
        // 自愈会就地改请求体,克隆一份免得影响调用方
        let mut body = body.clone();
        let mut heal_rounds = 0u32;
        const RETRY_MAX: u32 = 5;
        /// 传输层瞬断的重试上限。真格式不兼容也会走到这（分不开）,
        /// 两次白试的代价可接受;真瞬断两次内基本恢复（实测 4 秒后即成功）。
        const TRANSIENT_MAX: u32 = 2;
        let mut attempt = 0u32;
        let mut transient = 0u32;
        loop {
            let mut req = self.http.post(url).json(&body);
            if !self.api_key.trim().is_empty() {
                req = req.bearer_auth(self.api_key.trim());
            }
            match send_and_classify(req, Instant::now()).await {
                SendOutcome::Done(r) => return r,
                SendOutcome::BadRequest(e) => {
                    // 按错误信息改请求体重发。改不动 / 轮数用尽就把 400 抛出去,
                    // 不做无意义的原样重试。
                    let Error::LlmResponse(msg) = &e else {
                        return Err(e);
                    };
                    if heal_rounds >= MAX_HEAL_ROUNDS || !heal_request(&mut body, msg) {
                        return Err(e);
                    }
                    heal_rounds += 1;
                    log::warn!("云端 API 400,按错误信息自愈后重试(第 {heal_rounds} 轮)");
                }
                SendOutcome::Transient(e) => {
                    transient += 1;
                    if transient > TRANSIENT_MAX {
                        return Err(e);
                    }
                    let wait = Duration::from_secs(2 * u64::from(transient));
                    log::warn!(
                        "云端 API 传输层瞬断（第 {transient}/{TRANSIENT_MAX} 次）：{e}；{}s 后原样重试",
                        wait.as_secs()
                    );
                    tokio::time::sleep(wait).await;
                }
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

/// 段总结的 chat 路由。本地走 [`ChatClient`]，外部走 [`ExternalChatClient`]。
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
    let mut body = build_chat_body(
        false,
        model,
        system,
        user_text,
        image_data_uris,
        max_tokens,
        Some(0.4),
    );
    body["chat_template_kwargs"] = json!({ "enable_thinking": false });
    body
}

fn build_chat_body(
    is_cloud: bool,
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

    // 云端走流式：非流式请求在生成期间连接零字节流动，大段总结要几分钟，
    // 系统 TCP 栈会先于客户端超时掐断（os error 60）。流式让 token 边生成边回。
    // 本地 llama-server 走 localhost，没有中间设备，保持非流式不动。
    //
    // include_usage 三家实测都支持：DeepSeek / 智谱在末块回一次，SiliconFlow
    // 每块都回（累计值）。不支持的服务商忽略该字段，累加器有自数的兜底。
    let mut body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user",   "content": user_content },
        ],
        "stream": is_cloud,
    });
    if is_cloud {
        body["stream_options"] = json!({ "include_usage": true });
    }
    // 输出预算跟用户配的 ctx_size 联动（caller 按 ctx_size/2 算，给 prompt 留另一半）：
    // - ctx=8K → 4K（普通 instruct 模型也用得完只是不会真生成那么多）
    // - ctx=64K → 32K（reasoning 模型思考链 + 答案都有空间）
    // 写死小值（768 / 4096）让 reasoning 模型一律 length 截断 content 空。
    //
    // 字段名分云端/本地：OpenAI 自 gpt-5.6 起对 max_tokens 直接 400
    // 「not supported with this model」。与 chat 共用 budget_key,两边同一份口径。
    body[budget_key(is_cloud)] = json!(max_tokens);
    // 本地 llama 固定 0.4 偏稳定（避免空话 / 重复）；云端传 None 不发该字段——
    // 各家约束不同（kimi-k2.5 只收 1，发 0.4 直接 400），厂商默认值最安全。
    if let Some(t) = temperature {
        body["temperature"] = json!(t);
    }
    body
}

/// 流式响应里的一块（OpenAI 兼容 `chat.completion.chunk`）。
///
/// 字段全 `Option` 且 `default`：三家实测的形状差异不小 —— DeepSeek 会把
/// `content` / `reasoning_content` 显式写成 `null` 交替出现，智谱不发
/// `system_fingerprint` / `logprobs`。未知字段一律忽略，缺失与显式 null 等价。
#[derive(Debug, Default, Deserialize)]
struct ChatChunk {
    #[serde(default)]
    choices: Vec<ChunkChoice>,
    #[serde(default)]
    usage: Option<ChatUsageRaw>,
}

#[derive(Debug, Default, Deserialize)]
struct ChunkChoice {
    #[serde(default)]
    delta: ChunkDelta,
}

#[derive(Debug, Default, Deserialize)]
struct ChunkDelta {
    #[serde(default)]
    content: Option<String>,
    /// 推理模型的思考链。DeepSeek 在思考阶段发它、`content` 为 null，
    /// 正式作答时反过来 —— 分开累计才能沿用非流式那套空回复归因。
    #[serde(default)]
    reasoning_content: Option<String>,
}

/// SSE 流的累加器：喂进每一行，攒出与非流式等价的结果。
///
/// 独立于 IO，因此可以直接单测（真实网络流的形状差异见 `docs/internal/流式传输规划.md`）。
#[derive(Debug, Default)]
struct SseAccumulator {
    content: String,
    reasoning: String,
    usage: Option<ChatUsageRaw>,
    /// 收到 `data: [DONE]` 即为正常收尾；没收到就断流 = 不完整
    done: bool,
    /// 自数的 delta 块数：服务商不回 usage 时用它近似 completion_tokens
    delta_count: u32,
}

impl SseAccumulator {
    /// 吃一行。返回 `Err` 表示这行是致命的协议错误；`Ok(())` 表示已处理或可忽略。
    ///
    /// 忽略而非报错的情形：空行（SSE 的块分隔）、`event:` / `id:` 等其它字段、
    /// 解析不出的 data 块。**单块解析失败不该让整次请求失败** —— 厂商偶尔
    /// 插入自有格式的块，丢掉一块比丢掉整个回答划算。
    fn push_line(&mut self, line: &str) {
        let line = line.trim_end_matches('\r');
        let Some(payload) = line.strip_prefix("data:") else {
            return; // 空行 / event: / id: / 注释行
        };
        let payload = payload.trim();
        if payload == "[DONE]" {
            self.done = true;
            return;
        }
        let Ok(chunk) = serde_json::from_str::<ChatChunk>(payload) else {
            return;
        };
        // usage 可能出现多次（SiliconFlow 每块都带，且是累计值）——取最后一次，
        // 不是累加。DeepSeek / 智谱只在末块给一次，两种形状都被这行覆盖。
        if chunk.usage.is_some() {
            self.usage = chunk.usage;
        }
        for c in chunk.choices {
            if let Some(t) = c.delta.content.filter(|s| !s.is_empty()) {
                self.content.push_str(&t);
                self.delta_count += 1;
            }
            if let Some(t) = c.delta.reasoning_content.filter(|s| !s.is_empty()) {
                self.reasoning.push_str(&t);
                self.delta_count += 1;
            }
        }
    }

    /// 收尾成与非流式同构的 [`ChatResp`]，好让 [`parse_response`] 原样复用
    /// （空回复归因、finish_reason 判定全都不用再写一遍）。
    ///
    /// `finish_reason` 取 "stop" 而不是转发服务端的值：SiliconFlow 末块回
    /// `null`（实测），转发会让下游误判。流走完 = 正常结束，这是唯一可靠的信号。
    fn finish(self) -> ChatResp {
        // 没拿到 usage 的服务商：用自数的块数近似 completion_tokens。
        // 近似值只用于展示，不参与任何计费或截断判断。
        let usage = self.usage.or(Some(ChatUsageRaw {
            prompt_tokens: 0,
            completion_tokens: self.delta_count,
        }));
        ChatResp {
            choices: vec![ChatChoice {
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content: self.content,
                    reasoning_content: if self.reasoning.is_empty() {
                        None
                    } else {
                        Some(self.reasoning)
                    },
                },
                finish_reason: Some("stop".to_string()),
            }],
            usage,
        }
    }
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
        // 本地路径重建不了请求（RequestBuilder 一次性），瞬断当普通错误返回；
        // 本地 llama-server 也基本不存在"连接中途被掐"这回事
        SendOutcome::Transient(e) => Err(e),
        // 本地 llama-server 的 400 是我们自己发错了参数,自愈没有意义
        SendOutcome::BadRequest(e) => Err(e),
    }
}

/// [`send_and_parse`] 的分类版：把 429 单独拎出来（附 Retry-After 等待时长），
/// 让 [`ExternalChatClient::post_with_retry`] 能做限流退避；其余情况走 Done。
enum SendOutcome {
    Done(Result<(String, ChatUsage)>),
    /// HTTP 400 = 参数不合这家/这个模型的口味,而错误信息本身写明了怎么改。
    /// 云端路径按 [`heal_request`] 自愈后重发;本地路径当普通错误。
    BadRequest(Error),
    /// 服务端 429：附 Retry-After 头解析出的等待时长（没给则 None）
    RateLimited(Option<Duration>),
    /// 传输层瞬时故障（连接建立失败 / 响应体中途被掐 / 响应体读取超时）：
    /// 同一请求原样重试大概率成功——实测 deepseek 晚高峰段总结失败后,
    /// 4 秒后的下一次调用即成功。响应体阶段的超时也算瞬断:本客户端只跑
    /// 后台批任务,多等一轮比让当天的段落空更值;发送阶段的超时仍不重试。
    Transient(Error),
}

/// reqwest 错误的 Display 只给最外层（如 "error decoding response body"），
/// 真实原因（连接被重置 / JSON 语法错在哪）全藏在 source 链里——排障靠它。
pub(crate) fn error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        s.push_str(" ← ");
        s.push_str(&c.to_string());
        cur = c.source();
    }
    s
}

async fn send_and_classify(req: RequestBuilder, t0: Instant) -> SendOutcome {
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            // 连接建立失败是秒级失败，重试便宜；超时已经等满时限，不重试。
            let transient = e.is_connect() && !e.is_timeout();
            let err = Error::LlmResponse(crate::commands::ai_endpoint::fmt_send_err(e));
            return if transient {
                SendOutcome::Transient(err)
            } else {
                SendOutcome::Done(Err(err))
            };
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
    if !status.is_success() {
        let preview: String = resp
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect();
        let err = Error::LlmResponse(format!("服务返回 {status}：{preview}"));
        // 400 单独拎出来:参数问题可按对方给的说法改一改重发(见 post_with_retry)。
        // 其余(鉴权/网关/找不到模型)重发也没用,直接判失败。
        if status.as_u16() == 400 {
            return SendOutcome::BadRequest(err);
        }
        return SendOutcome::Done(Err(err));
    }

    // 2xx 的响应体读取 + 解析。这一步失败的主要形态是**传输层瞬断**
    // （连接被服务端/中间层中途掐掉，reqwest 报 "error decoding response body"，
    // 真正的 JSON 不兼容反而罕见）——实测 deepseek 晚高峰连续失败后紧接着
    // 同模型成功。归入 Transient 让上层原样重试；超时除外（已等满时限）。
    // 错误文本带完整原因链：外层 Display 只说"解码失败"，掐连接还是格式错
    // 全靠 source 链区分。
    let is_sse = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/event-stream"));
    let parsed: ChatResp = if is_sse {
        match read_sse_stream(resp).await {
            Ok(p) => p,
            Err(e) => return SendOutcome::Transient(e),
        }
    } else {
        match resp.json().await {
            Ok(p) => p,
            Err(e) => {
                return SendOutcome::Transient(Error::LlmResponse(format!(
                    "响应体读取/解析失败：{}",
                    error_chain(&e)
                )));
            }
        }
    };
    SendOutcome::Done(parse_response(parsed, t0))
}

/// 消费 SSE 流，攒成与非流式等价的 [`ChatResp`]。
///
/// 为什么值得这么做：非流式请求发出后连接会静默到整段生成完，大时段的段总结
/// 要几分钟，期间一个字节都不流动 —— 系统 TCP 栈会先于客户端超时把它掐掉
/// （实测 `Operation timed out (os error 60)`，客户端 300s 根本来不及生效）。
/// 流式让 token 边生成边回，连接上持续有数据，静默这个前提就不存在了。
///
/// 断流（没收到 `data: [DONE]` 就 EOF）判失败而不是留半截：调用方会把返回值
/// 整段存进 ai_summaries / 聊天记录，半截答案看起来像正常内容，比报错更难发现。
/// 错误里带上已收字符数，便于分辨"一个字没来"和"快写完了才断"。
async fn read_sse_stream(resp: reqwest::Response) -> Result<ChatResp> {
    use futures_util::StreamExt;

    let mut acc = SseAccumulator::default();
    let mut buf = String::new();
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| {
            Error::LlmResponse(format!(
                "流式响应中断（已收到 {} 字符）：{}",
                acc.content.chars().count(),
                error_chain(&e)
            ))
        })?;
        // 一个网络块可能切在多字节字符中间，也可能带半行 —— 先按 UTF-8 宽松解码
        // 再按整行切，剩下的半行留在 buf 里等下一块补齐。
        buf.push_str(&String::from_utf8_lossy(&bytes));
        while let Some(idx) = buf.find('\n') {
            let line: String = buf.drain(..=idx).collect();
            acc.push_line(line.trim_end_matches('\n'));
        }
        if acc.done {
            break;
        }
    }
    // 收尾：流已 EOF 时 buf 里可能还剩最后一行（服务端没发末尾换行）
    if !acc.done && !buf.is_empty() {
        let last = std::mem::take(&mut buf);
        acc.push_line(&last);
    }

    if !acc.done {
        return Err(Error::LlmResponse(format!(
            "流式响应未正常结束（已收到 {} 字符，缺 [DONE]）",
            acc.content.chars().count()
        )));
    }
    Ok(acc.finish())
}

/// 2xx 且响应体已解析后的常规收尾：取首个 choice、算 usage、空内容分类报错。
fn parse_response(parsed: ChatResp, t0: Instant) -> Result<(String, ChatUsage)> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 把若干行喂进累加器（模拟真实流的逐行到达）。
    fn feed(lines: &[&str]) -> SseAccumulator {
        let mut acc = SseAccumulator::default();
        for l in lines {
            acc.push_line(l);
        }
        acc
    }

    /// 基本形态:多块拼接 + [DONE] 收尾。
    #[test]
    fn sse_joins_deltas_and_ends_on_done() {
        let acc = feed(&[
            r#"data: {"choices":[{"delta":{"role":"assistant","content":"你"}}]}"#,
            "",
            r#"data: {"choices":[{"delta":{"content":"好"}}]}"#,
            "data: [DONE]",
        ]);
        assert!(acc.done);
        let resp = acc.finish();
        assert_eq!(resp.choices[0].message.content, "你好");
        // finish_reason 恒为 stop:SiliconFlow 末块实测回 null,转发会让下游误判
        assert_eq!(resp.choices[0].finish_reason.as_deref(), Some("stop"));
    }

    /// DeepSeek 形态:content 与 reasoning_content 显式 null 交替出现。
    /// 分开累计才能沿用非流式那套空回复归因（思考烧完没写答案 = LLM_EMPTY_REASONING）。
    #[test]
    fn sse_separates_reasoning_from_content_with_explicit_nulls() {
        let acc = feed(&[
            r#"data: {"choices":[{"delta":{"content":null,"reasoning_content":"想"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":null,"reasoning_content":"一下"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"答案","reasoning_content":null}}]}"#,
            "data: [DONE]",
        ]);
        let resp = acc.finish();
        assert_eq!(resp.choices[0].message.content, "答案");
        assert_eq!(
            resp.choices[0].message.reasoning_content.as_deref(),
            Some("想一下")
        );
    }

    /// SiliconFlow 形态:usage 每块都带且是**累计值** —— 取最后一次，不是累加。
    #[test]
    fn sse_usage_takes_last_not_sum() {
        let acc = feed(&[
            r#"data: {"choices":[{"delta":{"content":"a"}}],"usage":{"prompt_tokens":31,"completion_tokens":1}}"#,
            r#"data: {"choices":[{"delta":{"content":"b"}}],"usage":{"prompt_tokens":31,"completion_tokens":2}}"#,
            "data: [DONE]",
        ]);
        let u = acc.finish().usage.expect("应有 usage");
        assert_eq!(u.prompt_tokens, 31);
        assert_eq!(u.completion_tokens, 2, "累计值取最后一次而不是相加");
    }

    /// 服务商不回 usage 时用自数的块数近似 completion_tokens（只用于展示）。
    #[test]
    fn sse_falls_back_to_counting_deltas_without_usage() {
        let acc = feed(&[
            r#"data: {"choices":[{"delta":{"content":"a"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"b"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"c"}}]}"#,
            "data: [DONE]",
        ]);
        let u = acc.finish().usage.expect("应有兜底 usage");
        assert_eq!(u.completion_tokens, 3);
        assert_eq!(u.prompt_tokens, 0, "自数拿不到 prompt 侧，如实填 0");
    }

    /// 非 data 行与解析不了的块都跳过:空行是 SSE 的块分隔，
    /// 厂商偶尔插自有格式的块 —— 丢一块比丢整个回答划算。
    #[test]
    fn sse_skips_blank_and_unparsable_lines() {
        let acc = feed(&[
            "",
            ": keep-alive comment",
            "event: message",
            "data: {not json}",
            r#"data: {"choices":[{"delta":{"content":"ok"}}]}"#,
            "data: [DONE]",
        ]);
        assert_eq!(acc.finish().choices[0].message.content, "ok");
    }

    /// 带 \r\n 行尾（部分服务端如此）也要正常吃掉。
    #[test]
    fn sse_tolerates_crlf() {
        let acc = feed(&[
            "data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\r",
            "data: [DONE]\r",
        ]);
        assert!(acc.done);
        assert_eq!(acc.finish().choices[0].message.content, "x");
    }

    /// 没收到 [DONE] 就是断流。read_sse_stream 据此判失败而不是留半截 ——
    /// 半截答案会被原样存进 ai_summaries / 聊天记录，看着像正常内容。
    #[test]
    fn sse_without_done_is_incomplete() {
        let acc = feed(&[r#"data: {"choices":[{"delta":{"content":"半截"}}]}"#]);
        assert!(!acc.done, "缺 [DONE] 必须判为未完成");
        assert_eq!(acc.content, "半截", "已收内容仍在，供错误信息报字符数");
    }

    /// 云端请求必须带 stream + include_usage；本地保持非流式。
    #[test]
    fn cloud_body_is_streaming_local_is_not() {
        let cloud = build_chat_body(true, "m", "sys", "user", &[], 4096, None);
        assert_eq!(cloud["stream"], serde_json::json!(true));
        assert_eq!(
            cloud["stream_options"]["include_usage"],
            serde_json::json!(true)
        );

        let local = build_chat_body(false, "m", "sys", "user", &[], 4096, None);
        assert_eq!(local["stream"], serde_json::json!(false));
        assert!(
            local.get("stream_options").is_none(),
            "本地 llama-server 不发 stream_options"
        );
    }

    /// 真机端到端:走完整的 ExternalChatClient 打真实端点，确认流式链路通。
    /// 单测只覆盖累加器的纯逻辑，证明不了真流能跑 —— 这条补上那一段。
    /// 跑法:
    ///   HINDSIGHT_E2E_BASE_URL=https://api.deepseek.com/v1 \
    ///   HINDSIGHT_E2E_MODEL=deepseek-v4-flash \
    ///   HINDSIGHT_E2E_KEY=sk-... \
    ///   cargo test --lib ai::llm::tests::live_ -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_cloud_streaming_roundtrip() {
        let (Ok(base), Ok(model), Ok(key)) = (
            std::env::var("HINDSIGHT_E2E_BASE_URL"),
            std::env::var("HINDSIGHT_E2E_MODEL"),
            std::env::var("HINDSIGHT_E2E_KEY"),
        ) else {
            eprintln!("跳过:未设 HINDSIGHT_E2E_* 三个环境变量");
            return;
        };
        let c = ExternalChatClient::new(&base, model, key, 512).expect("客户端构造");
        let t = std::time::Instant::now();
        let (text, usage) = c
            .chat_text("你是一个简洁的助手。", "用一句话说明什么是 TCP 超时。", &[])
            .await
            .expect("流式请求应成功");
        eprintln!(
            "耗时 {}ms | 内容 {} 字符 | prompt={:?} completion={:?}\n---\n{text}\n---",
            t.elapsed().as_millis(),
            text.chars().count(),
            usage.prompt_tokens,
            usage.completion_tokens,
        );
        assert!(!text.trim().is_empty(), "流式应拼出非空内容");
        assert!(
            usage.completion_tokens.unwrap_or(0) > 0,
            "应拿到 completion_tokens(真值或自数兜底)"
        );
    }

    /// 云端与本地的输出预算字段名必须分开:OpenAI 自 gpt-5.6 起对
    /// `max_tokens` 直接 400,而本地 llama-server 只认 `max_tokens`。
    /// 这里曾经硬编码 `max_tokens`,导致同一端点下聊天可用、日报每段都 400。
    #[test]
    fn cloud_body_uses_max_completion_tokens_local_keeps_max_tokens() {
        let cloud = build_chat_body(true, "gpt-5.6", "sys", "user", &[], 4096, None);
        assert_eq!(cloud["max_completion_tokens"], 4096);
        assert!(
            cloud.get("max_tokens").is_none(),
            "云端不能再发旧字段名: {cloud}"
        );

        let local = build_chat_body_local("qwen", "sys", "user", &[], 768);
        assert_eq!(local["max_tokens"], 768);
        assert!(
            local.get("max_completion_tokens").is_none(),
            "本地 llama-server 只认 max_tokens: {local}"
        );

        // 与 chat 侧共用同一份口径——两边分歧正是这条 bug 的根因
        assert_eq!(budget_key(true), "max_completion_tokens");
        assert_eq!(budget_key(false), "max_tokens");
    }

    /// 摘要侧接上自愈后,老网关拒收新字段名也能自救:
    /// R2(按对方给的新名字改名)与 R1(不认识的字段直接删)都要对摘要请求体生效。
    #[test]
    fn summary_body_is_healable_by_shared_rules() {
        let e400 = |m: &str| format!("HTTP 400 Bad Request: {{\"error\":{{\"message\":\"{m}\"}}}}");

        // 反向场景:端点只认旧名字,让它把新名字改回去
        let mut b = build_chat_body(true, "m", "sys", "user", &[], 4096, None);
        assert!(heal_request(
            &mut b,
            &e400("Unsupported parameter: 'max_completion_tokens' is not supported with this model. Use 'max_tokens' instead.")
        ));
        assert_eq!(b["max_tokens"], 4096);
        assert!(b.get("max_completion_tokens").is_none());

        // 老兼容网关完全不认识这个字段 → 删掉重发(退化成不限输出长度,可接受)。
        // 文案取 R1 认的真机格式 `Unknown parameter: '<字段>'`
        let mut b = build_chat_body(true, "m", "sys", "user", &[], 4096, None);
        assert!(heal_request(
            &mut b,
            &e400("Unknown parameter: 'max_completion_tokens'.")
        ));
        assert!(b.get("max_completion_tokens").is_none());
        // 核心字段一个都不能因为自愈丢掉
        assert!(
            b.get("model").is_some() && b.get("messages").is_some(),
            "{b}"
        );
    }

    /// error_chain 必须逐层展开 source:reqwest 的 "error decoding response body"
    /// 外层信息为零,真实原因(连接被掐 / serde 语法错)全在链的深处——
    /// 8 月初 deepseek 晚高峰的间歇失败正是因为看不到链才拖了一周没定位。
    #[test]
    fn error_chain_walks_sources() {
        #[derive(Debug)]
        struct Outer(std::io::Error);
        impl std::fmt::Display for Outer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "error decoding response body")
            }
        }
        impl std::error::Error for Outer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        let e = Outer(std::io::Error::other("connection reset by peer"));
        let s = error_chain(&e);
        assert!(s.contains("error decoding response body"), "{s}");
        assert!(s.contains(" ← "), "缺链接符:{s}");
        assert!(s.contains("connection reset by peer"), "缺底层原因:{s}");
    }
}
