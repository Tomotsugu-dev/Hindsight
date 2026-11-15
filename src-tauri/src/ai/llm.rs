//! OpenAI 兼容 chat completions 客户端，专为本地 llama-server 调优（Phase 1B-γ）。
//!
//! - [`ChatClient::new`] 用引擎当前端口构造客户端
//! - [`ChatClient::chat_with_images`] 发送一条 multimodal 请求（system + user text + N 张 image_url）
//!
//! 流式只对**云端**开：非流式请求在生成期间连接零字节流动，大段总结要几分钟，
//! 系统 TCP 栈会先于客户端超时把它掐掉（实测 `os error 60`）。本地 llama-server
//! 走 localhost，没有中间设备，保持非流式 —— 一次性出文更简单可靠。
//! 请求由 [`ModelTransport`] 发，超时和重发在那里；拼流式回复和 400 自愈在 [`ChatCompletions`]。

use std::time::Instant;

use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ai::openai_compat::{budget_key, ChatCompletions, CHAT_COMPLETIONS_PATH};
use crate::ai::transport::{
    bearer_headers, fmt_send_err, ModelTransport, TransportError, TransportPolicy,
};
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

/// llama-server 的 chat 客户端。
#[derive(Clone)]
pub struct ChatClient {
    /// 模型名——llama-server 不强求是真实文件名，可填 "default" / 任意字符串
    /// 都行；这里就拿 active_main 文件名当 ID 方便调试日志区分
    model: String,
    /// 单次响应 max_tokens 上限。caller 按用户配的 ctx_size_per_slot / 2 算
    /// （给 prompt 留另一半）；ctx=8K → 4K，ctx=64K → 32K。
    max_tokens: u32,
    transport: ModelTransport,
}

impl ChatClient {
    /// `port` 来自 [`crate::ai::server::EngineSupervisor::status()`] 返回的端口；
    /// `model` 直接传 `settings.ai.active_main`（含 .gguf 后缀也行）；
    /// `max_tokens` 由 caller 按 effective ctx_size 折半算，让用户的"上下文（每路）64K"
    /// 设置真能反映到单次响应能写多长。
    pub fn new(port: u16, model: impl Into<String>, max_tokens: u32) -> Result<Self> {
        let transport = ModelTransport::new(
            &format!("http://127.0.0.1:{port}/v1"),
            HeaderMap::new(),
            TransportPolicy::local(),
        )?;
        Ok(Self {
            model: model.into(),
            max_tokens: max_tokens.max(512), // 不让 caller 算出过小的 max_tokens 让所有响应都被截断
            transport,
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
        let body = build_chat_body_local(
            &self.model,
            system,
            user_text,
            image_data_uris,
            self.max_tokens,
        );
        send(&self.transport, &body).await
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
    model: String,
    /// 同 [`ChatClient::max_tokens`]——caller 按 effective ctx_size 折半算
    max_tokens: u32,
    transport: ModelTransport,
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
        let transport = ModelTransport::new(
            &base_url,
            bearer_headers(&api_key)?,
            TransportPolicy::cloud(),
        )?;
        Ok(Self {
            model,
            max_tokens: max_tokens.max(512),
            transport,
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
        let body = build_chat_body(
            true,
            &self.model,
            system,
            user_text,
            &[],
            self.max_tokens,
            None,
        );
        send(&self.transport, &body).await
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

/// 发出请求，取回正文和用量。
async fn send(transport: &ModelTransport, body: &serde_json::Value) -> Result<(String, ChatUsage)> {
    let t0 = Instant::now();
    let reply = transport
        .post(CHAT_COMPLETIONS_PATH, body, &ChatCompletions)
        .await
        .map_err(into_llm_error)?;
    let parsed: ChatResp = serde_json::from_value(reply)
        .map_err(|e| Error::LlmResponse(format!("cannot read or parse the response body: {e}")))?;
    parse_response(parsed, t0)
}

/// 日报、周报给用户看的错误文字。
fn into_llm_error(e: TransportError) -> Error {
    let msg = match e {
        TransportError::Send(e) => fmt_send_err(e),
        TransportError::Status { status, body } => {
            let preview: String = body.chars().take(200).collect();
            format!("服务返回 {status}：{preview}")
        }
        TransportError::RateLimited { .. } => {
            "服务持续限流（429 Too Many Requests），已退避重试多次仍失败——多半是账户 RPM 配额太低，稍后再跑或升级配额".to_string()
        }
        TransportError::Body(msg) => msg,
    };
    Error::LlmResponse(msg)
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
    // TODO: 非流式回复的 content 可以是 null，这里会解析失败；改成 Option<String>
    content: String,
    /// 新版 llama-server (>= b4500) 跟 OpenAI 兼容 reasoning 模型 (DeepSeek R1 /
    /// Qwen3 thinking) 都会把思考链放这里，正式回答留在 `content`。
    /// 思考链占满 max_tokens 时 content 为空、reasoning_content 非空——典型征兆。
    #[serde(default)]
    reasoning_content: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::openai_compat::heal_request;

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

    // ---- 以下从网络这一侧锁住日报发请求的行为：重构传输层前后，这几条一行不改 ----

    use crate::ai::transport::fake::{FakeServer, Reply};
    use serde_json::json;

    fn cloud(server: &FakeServer) -> ExternalChatClient {
        ExternalChatClient::new(&server.base_url(), "m".into(), "sk-test".into(), 4096).unwrap()
    }

    /// 一段完整的流：正文一块，末块只带用量，最后 [DONE]。
    fn sse_ok(text: &str) -> Reply {
        Reply::sse(
            &[
                json!({"choices": [{"delta": {"content": text}}]}),
                json!({"choices": [], "usage": {"prompt_tokens": 9, "completion_tokens": 3}}),
            ],
            true,
        )
    }

    /// 云端：请求要流式和用量，带鉴权头；回来的流拼出正文和用量。
    #[tokio::test]
    async fn summary_cloud_reads_sse_reply() {
        let server = FakeServer::start(vec![sse_ok("今天在写测试。")]).await;
        let (text, usage) = cloud(&server).chat_text("sys", "user", &[]).await.unwrap();
        assert_eq!(text, "今天在写测试。");
        assert_eq!(
            (usage.prompt_tokens, usage.completion_tokens),
            (Some(9), Some(3))
        );
        let got = server.received();
        assert_eq!(got[0].body["stream"], true);
        assert_eq!(got[0].body["stream_options"]["include_usage"], true);
        assert!(got[0].head.contains("authorization: bearer sk-test"));
    }

    /// 云端回 400 并写明该换哪个字段名：改名后重发，第二次成功。
    #[tokio::test]
    async fn summary_cloud_heals_400_then_succeeds() {
        let server = FakeServer::start(vec![
            Reply::json(
                400,
                json!({"error": {"message": "Unsupported parameter: 'max_completion_tokens' is not supported with this model. Use 'max_tokens' instead."}}),
            ),
            sse_ok("好"),
        ])
        .await;
        let (text, _) = cloud(&server).chat_text("sys", "user", &[]).await.unwrap();
        assert_eq!(text, "好");
        let got = server.received();
        assert_eq!(got.len(), 2, "只重发一次");
        assert!(got[1].body.get("max_completion_tokens").is_none());
        assert_eq!(got[1].body["max_tokens"], 4096);
    }

    /// 云端限流：等一会儿原样重发，第二次成功。
    #[tokio::test]
    async fn summary_cloud_backs_off_on_429_then_succeeds() {
        let server = FakeServer::start(vec![
            Reply::json(429, json!({"error": {"message": "rate limited"}}))
                .header("Retry-After", "1"),
            sse_ok("好"),
        ])
        .await;
        let (text, _) = cloud(&server).chat_text("sys", "user", &[]).await.unwrap();
        assert_eq!(text, "好");
        let got = server.received();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].body, got[1].body, "限流后原样重发");
    }

    /// 云端的流没收到 [DONE] 就断了：原样重发，第二次成功。
    #[tokio::test]
    async fn summary_cloud_retries_cut_stream_then_succeeds() {
        let server = FakeServer::start(vec![
            Reply::sse(
                &[json!({"choices": [{"delta": {"content": "半截"}}]})],
                false,
            ),
            sse_ok("完整"),
        ])
        .await;
        let (text, _) = cloud(&server).chat_text("sys", "user", &[]).await.unwrap();
        assert_eq!(text, "完整", "半截的流不能当结果");
        assert_eq!(server.received().len(), 2);
    }

    /// 本机 llama-server 回 400：不改请求、不重发，直接报错。
    #[tokio::test]
    async fn summary_local_400_fails_without_retry() {
        let server = FakeServer::start(vec![Reply::json(
            400,
            json!({"error": {"message": "Unknown parameter: 'temperature'."}}),
        )])
        .await;
        let err = ChatClient::new(server.port, "m.gguf", 1024)
            .unwrap()
            .chat_with_images("sys", "user", &[])
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("400"), "{err}");
        assert_eq!(server.received().len(), 1);
    }
}
