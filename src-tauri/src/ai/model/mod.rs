//! The interface for calling language models.

use std::future::Future;

use serde_json::Value;

use crate::ai::providers::chat_completions::ChatCompletionsModel;
use crate::error::Error;

/// What one call asks the model to do.
#[derive(Debug, Clone)]
pub struct Request {
    pub system: String,
    pub messages: Vec<Message>,
    /// The tools that the model can call this time.
    /// When the list is empty, the request has no tools.
    pub tools: Vec<ToolSpec>,
    pub reasoning: Reasoning,
    /// The token limit for the answer.
    ///
    /// When reasoning is not off, the adapter sends the larger of this number
    /// and the reasoning floor.
    /// Reasoning tokens also count toward the limit that is sent.
    pub max_output_tokens: u32,
    /// The sampling temperature that the task suggests.
    /// The adapter may leave it out.
    pub temperature: Option<f64>,
}

/// One message in the conversation history.
#[derive(Debug, Clone)]
pub enum Message {
    /// Each item in `images` is the data URI of one image.
    User { text: String, images: Vec<String> },
    /// `carry` holds the `carry` of the [`Response`] that produced this message.
    /// It must not be changed.
    Assistant {
        text: String,
        call: Option<ToolCall>,
        carry: Option<Carry>,
    },
    /// `call_id` is the `id` of the matching [`ToolCall`].
    ToolResult { call_id: String, content: String },
}

/// The reasoning level.
///
/// `Default` sets no level. The adapter picks the level for each service.
/// When the service does not have the asked level, the adapter uses the nearest higher level.
/// If there is no higher level, the adapter uses the highest one.
/// When the service does not allow reasoning with tools, the adapter turns reasoning off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reasoning {
    Default,
    Off,
    Low,
    Medium,
    High,
    Max,
}

/// One reply from the model.
#[derive(Debug, Clone)]
pub struct Response {
    /// The text has no leading or trailing whitespace.
    pub text: String,
    /// The tool call that the model asks for.
    /// When the service returns several calls, this holds only the first one.
    pub call: Option<ToolCall>,
    pub carry: Option<Carry>,
    pub usage: Usage,
}

/// The total usage of one call.
///
/// An item that the service did not report is `None`.
/// In a streamed reply, if no chunk has both the input and the output token counts,
/// the input is 0 and the output is the number of deltas received.
#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    /// The output token count includes reasoning.
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    /// Milliseconds from sending the request to reading the whole reply.
    /// This includes retries and the waits before them.
    pub elapsed_ms: u64,
}

/// The definition of one tool that the model can call.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    /// The description of the tool.
    /// Native tool calling sends it to the model. Grammar mode does not send it.
    pub description: String,
    /// The JSON Schema of the arguments.
    pub parameters: Value,
}

/// One call from the model.
#[derive(Debug, Clone)]
pub struct ToolCall {
    /// The identifier of this call.
    /// The tool result points back to this call with the `call_id` of [`Message::ToolResult`].
    pub id: String,
    /// The tool to call. It matches the `name` of a [`ToolSpec`].
    pub name: String,
    /// The arguments of the call. They are not checked against the schema yet.
    /// When the arguments of a native call are not JSON, this is an empty object.
    pub arguments: Value,
}

/// Content that goes back to the model unchanged in the next step.
///
/// Only the adapter that produced it reads it.
/// In DeepSeek thinking mode, when a request has tools, later requests must send back
/// `reasoning_content`. Otherwise DeepSeek returns 400.
/// See <https://api-docs.deepseek.com/guides/thinking_mode>.
#[derive(Debug, Clone)]
pub struct Carry(pub(crate) Value);

/// Properties that are set when the model is created, such as whether it runs on this device.
#[derive(Debug, Clone)]
pub struct Capabilities {
    pub on_device: bool,
}

/// 模型在这台电脑上用不了的原因。
///
/// 传给前端时写成 snake_case，比如 `device_not_eligible`。
// Windows 上目前没有模型会报这些原因。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    /// 不是 macOS。
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    NotMacos,
    /// macOS 低于 26，系统里没有这个框架。
    OsTooOld,
    /// 系统说这台设备不支持。可能是 Intel 芯片，也可能是在中国大陆，系统不区分。
    DeviceNotEligible,
    /// 系统说没开 Apple Intelligence。
    NotEnabled,
    /// 系统说模型还没准备好，比如还在下载。
    ModelNotReady,
    /// 设置里的提示词语言不受支持。
    LanguageNotSupported,
}

/// Why a model call failed.
///
/// The transport layer has already retried a cloud request on 429, on a connection failure
/// other than a timeout, and when the response body broke off or could not be read.
/// The error is from the last attempt.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    /// The service returned 401 or 403.
    #[error("{0}")]
    Auth(String),
    /// The retries ran out and the service is still rate limiting.
    #[error("{0}")]
    RateLimited(String),
    /// The model wrote no text.
    /// `code` is an error code that the frontend knows.
    /// It is `LLM_EMPTY` or starts with `LLM_EMPTY_`.
    #[error("[{code}] {detail}")]
    Empty { code: &'static str, detail: String },
    /// The request got no response (connection failure or timeout),
    /// the stream did not end normally, or the response body could not be read.
    #[error("{0}")]
    Transport(String),
    /// The service returned an error status other than 401, 403 or 429,
    /// or the model output is not in the agreed format.
    #[error("{0}")]
    Provider(String),
}

impl From<ModelError> for Error {
    fn from(e: ModelError) -> Self {
        Error::LlmResponse(e.to_string())
    }
}

/// The operations of a language model: reading its capabilities and generating a reply.
pub trait LanguageModelOps {
    fn capabilities(&self) -> &Capabilities;

    /// Sends `req` to the model and returns the model's reply.
    ///
    /// Dropping the returned future cancels the call.
    /// The HTTP implementation closes the connection.
    fn generate(&self, req: Request) -> impl Future<Output = Result<Response, ModelError>> + Send;
}

/// A model that is ready to use.
#[derive(Clone)]
pub enum LanguageModel {
    /// Cloud services and the local llama-server all use this kind.
    ChatCompletions(ChatCompletionsModel),
}

impl LanguageModelOps for LanguageModel {
    fn capabilities(&self) -> &Capabilities {
        match self {
            Self::ChatCompletions(m) => m.capabilities(),
        }
    }

    async fn generate(&self, req: Request) -> Result<Response, ModelError> {
        match self {
            Self::ChatCompletions(m) => m.generate(req).await,
        }
    }
}
