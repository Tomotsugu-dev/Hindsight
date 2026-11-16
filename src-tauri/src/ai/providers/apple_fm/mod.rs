//! macOS 自带的本机模型 Apple Intelligence（Apple Foundation Models）。
//!
//! 在进程内通过 Swift 调系统框架，不走 HTTP。Swift 只包系统调用，判断都在这里。

mod ffi;

use std::time::Instant;

use serde::Deserialize;

use self::ffi::SystemError;
use crate::ai::model::{
    Capabilities, LanguageModelOps, Message, ModelError, Request, Response, UnavailableReason,
    Usage,
};

/// Apple Intelligence 模型。每次生成新建一个 session，不带对话历史。
#[derive(Debug, Clone)]
pub struct AppleFmModel {
    prompt_language: String,
    capabilities: Capabilities,
}

impl AppleFmModel {
    /// `prompt_language` 是设置里 `ai.prompt_language` 的值，生成前用它查语言支不支持。
    pub fn new(prompt_language: &str) -> Self {
        Self {
            prompt_language: prompt_language.to_string(),
            capabilities: Capabilities { on_device: true },
        }
    }
}

impl LanguageModelOps for AppleFmModel {
    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    /// 只做单轮：请求里要正好一条纯文字的用户消息，不带工具。
    /// 思考档位忽略。用量不报，只报耗时。
    async fn generate(&self, req: Request) -> Result<Response, ModelError> {
        let started = Instant::now();
        let prompt = single_prompt(&req)?;
        let context_size = check(&self.prompt_language).map_err(ModelError::Unavailable)?;

        let counted = ffi::count_tokens(&req.system, prompt).wait().await;
        if let Some(e) = &counted.error {
            log::warn!(
                "Apple Intelligence: cannot count tokens, estimating: {} {}",
                e.domain,
                e.code
            );
        }

        let input = counted
            .tokens
            .unwrap_or_else(|| estimate_tokens(&req.system) + estimate_tokens(prompt));
        check_fits(input, req.max_output_tokens, context_size)?;

        let outcome = ffi::generate(&req.system, prompt, req.max_output_tokens, req.temperature)
            .wait()
            .await;
        if let Some(e) = outcome.error {
            return Err(to_model_error(e));
        }
        let text = outcome.text.unwrap_or_default().trim().to_string();
        if text.is_empty() {
            return Err(ModelError::Empty {
                code: "LLM_EMPTY",
                detail: "Apple Intelligence returned no text".to_string(),
            });
        }
        Ok(Response {
            text,
            call: None,
            carry: None,
            usage: Usage {
                elapsed_ms: started.elapsed().as_millis() as u64,
                ..Usage::default()
            },
        })
    }
}

/// 取出请求里唯一的一条用户文字。
///
/// # Errors
///
/// 请求带工具、带图，或者不是正好一条用户消息时，报 [`ModelError::Provider`]。
fn single_prompt(req: &Request) -> Result<&str, ModelError> {
    if !req.tools.is_empty() {
        return Err(ModelError::Provider(
            "Apple Intelligence does not take tools".to_string(),
        ));
    }
    match req.messages.as_slice() {
        [Message::User { text, images }] if images.is_empty() => Ok(text),
        _ => Err(ModelError::Provider(
            "Apple Intelligence takes exactly one text message from the user".to_string(),
        )),
    }
}

/// 检查输入加回答上限放不放得进上下文。
///
/// # Errors
///
/// 放不下时报 [`ModelError::ContextOverflow`]。
fn check_fits(input: u32, max_output: u32, context_size: u32) -> Result<(), ModelError> {
    let needed = input.saturating_add(max_output);
    if needed <= context_size {
        return Ok(());
    }
    Err(ModelError::ContextOverflow(format!(
        "the input ({input} tokens) and the answer limit ({max_output}) need {needed} tokens, \
         the model has {context_size}"
    )))
}

/// 系统数不了 token 时（macOS 26.4 以前）按字符估，取保守的一端：
/// 中日韩字符每个算 1 token，其他字符每 3 个算 1 token。
fn estimate_tokens(text: &str) -> u32 {
    let cjk = text.chars().filter(|&c| is_cjk(c)).count() as u32;
    let other = text.chars().count() as u32 - cjk;
    cjk + other.div_ceil(3)
}

/// 中日韩的文字和标点。
fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{3000}'..='\u{30FF}'     // 标点、平假名、片假名
        | '\u{3400}'..='\u{4DBF}'   // 汉字扩展 A
        | '\u{4E00}'..='\u{9FFF}'   // 常用汉字
        | '\u{AC00}'..='\u{D7AF}'   // 韩文
        | '\u{F900}'..='\u{FAFF}'   // 兼容汉字
        | '\u{FF00}'..='\u{FFEF}'   // 全角字符
        | '\u{20000}'..='\u{2FA1F}' // 汉字扩展 B 以后
    )
}

/// 把系统报的错误认成统一的错误。
///
/// macOS 26 和 27 的错误类型不同，构建机只有 macOS 26 的 SDK，所以按 domain 和 code 认。
/// code 是错误在系统里声明的顺序，见设计稿 2.6。
fn to_model_error(e: SystemError) -> ModelError {
    const OLD: &str = "FoundationModels.LanguageModelSession.GenerationError";
    const NEW: &str = "FoundationModels.LanguageModelError";
    match (e.domain.as_str(), e.code) {
        (OLD | NEW, 0) => ModelError::ContextOverflow(e.message),
        (OLD, 1) => ModelError::Unavailable(UnavailableReason::ModelNotReady),
        (OLD, 2 | 8) | (NEW, 2 | 3) => ModelError::Refused(e.message),
        (OLD, 4) | (NEW, 7) => ModelError::Unavailable(UnavailableReason::LanguageNotSupported),
        (OLD, 6) | (NEW, 1) => ModelError::RateLimited(e.message),
        _ => ModelError::Provider(format!("{} {}: {}", e.domain, e.code, e.message)),
    }
}

/// Swift 交来的状态，见 `hs_afm_status`。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SystemStatus {
    availability: String,
    language_supported: bool,
    context_size: Option<u32>,
}

/// 查这台 Mac 能不能用模型写这种提示词语言。能用时返回上下文长度（token）。
///
/// `prompt_language` 是设置里 `ai.prompt_language` 的值。
pub fn check(prompt_language: &str) -> Result<u32, UnavailableReason> {
    let json = ffi::status(language_tag(prompt_language));
    let status: SystemStatus = serde_json::from_str(&json).map_err(|e| {
        log::warn!("Apple Intelligence: cannot read the status from Swift: {e}");
        UnavailableReason::ModelNotReady
    })?;
    match unavailable_reason(&status) {
        Some(reason) => Err(reason),
        None => status.context_size.ok_or(UnavailableReason::ModelNotReady),
    }
}

/// 给出用不了的原因。能用时返回 `None`。
fn unavailable_reason(status: &SystemStatus) -> Option<UnavailableReason> {
    match status.availability.as_str() {
        "available" if status.language_supported => None,
        "available" => Some(UnavailableReason::LanguageNotSupported),
        "os_too_old" => Some(UnavailableReason::OsTooOld),
        "device_not_eligible" => Some(UnavailableReason::DeviceNotEligible),
        "apple_intelligence_not_enabled" => Some(UnavailableReason::NotEnabled),
        // 包括 `model_not_ready`，以及系统以后新加的原因。
        _ => Some(UnavailableReason::ModelNotReady),
    }
}

/// 提示词语言对应的 BCP 47 语言标签。
fn language_tag(prompt_language: &str) -> &'static str {
    match prompt_language {
        "tw" => "zh-Hant",
        "en" => "en",
        "ja" => "ja",
        "pt" => "pt-BR",
        "es" => "es",
        _ => "zh-Hans",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::model::{Message, Reasoning, ToolSpec};
    use UnavailableReason::*;

    fn status(availability: &str, language_supported: bool) -> SystemStatus {
        SystemStatus {
            availability: availability.to_string(),
            language_supported,
            context_size: Some(8192),
        }
    }

    /// 系统报的每种状态都对应一个原因；能用但语言不支持，报语言不支持。
    /// 系统以后新加的原因按「模型没准备好」处理。
    #[test]
    fn status_maps_each_unavailable_reason() {
        let cases = [
            ("available", true, None),
            ("available", false, Some(LanguageNotSupported)),
            ("os_too_old", false, Some(OsTooOld)),
            ("device_not_eligible", false, Some(DeviceNotEligible)),
            ("apple_intelligence_not_enabled", false, Some(NotEnabled)),
            ("model_not_ready", false, Some(ModelNotReady)),
            ("unknown", false, Some(ModelNotReady)),
        ];
        for (availability, supported, want) in cases {
            assert_eq!(
                unavailable_reason(&status(availability, supported)),
                want,
                "{availability}"
            );
        }
    }

    /// 设置里的六种提示词语言各对应一个语言标签；认不出的值按简体中文，和设置的 sanitize 一致。
    #[test]
    fn prompt_languages_map_to_language_tags() {
        let cases = [
            ("zh", "zh-Hans"),
            ("tw", "zh-Hant"),
            ("en", "en"),
            ("ja", "ja"),
            ("pt", "pt-BR"),
            ("es", "es"),
            ("xx", "zh-Hans"),
        ];
        for (lang, want) in cases {
            assert_eq!(language_tag(lang), want, "{lang}");
        }
    }

    fn request(messages: Vec<Message>, tools: Vec<ToolSpec>) -> Request {
        Request {
            system: "系统提示词".to_string(),
            messages,
            tools,
            reasoning: Reasoning::Default,
            max_output_tokens: 1024,
            temperature: Some(0.4),
        }
    }

    fn user(text: &str) -> Message {
        Message::User {
            text: text.to_string(),
            images: Vec::new(),
        }
    }

    /// 输入加回答上限放不进上下文时，不发请求，直接报超长。正好放得下时照常发。
    #[test]
    fn overflow_is_rejected_before_sending() {
        assert!(check_fits(7168, 1024, 8192).is_ok());
        let err = check_fits(7169, 1024, 8192).unwrap_err();
        assert!(matches!(err, ModelError::ContextOverflow(_)), "{err:?}");
        assert!(
            err.to_string().starts_with("[LLM_CONTEXT_OVERFLOW]"),
            "{err}"
        );
    }

    /// 系统数不了 token 时按字符估：中日韩字符每个 1 token，其他字符每 3 个 1 token，不足 3 个也算 1。
    #[test]
    fn estimate_counts_cjk_as_one_token_each() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("今日は"), 3);
        assert_eq!(estimate_tokens("한국"), 2);
        assert_eq!(estimate_tokens("abcd"), 2);
        assert_eq!(estimate_tokens("写 code"), 1 + 2);
    }

    /// 只收一条纯文字的用户消息。带工具、带图、多条消息都报 Provider，不发请求。
    #[test]
    fn request_must_be_one_text_message() {
        let ok = request(vec![user("材料")], Vec::new());
        assert_eq!(single_prompt(&ok).unwrap(), "材料");

        let tool = ToolSpec {
            name: "t".to_string(),
            description: String::new(),
            parameters: serde_json::json!({}),
        };
        let with_image = Message::User {
            text: "材料".to_string(),
            images: vec!["data:image/png;base64,".to_string()],
        };
        let bad = [
            request(vec![user("材料")], vec![tool]),
            request(vec![with_image], Vec::new()),
            request(vec![user("一"), user("二")], Vec::new()),
            request(Vec::new(), Vec::new()),
        ];
        for req in bad {
            let err = single_prompt(&req).unwrap_err();
            assert!(matches!(err, ModelError::Provider(_)), "{err:?}");
        }
    }

    /// macOS 26 和 27 的错误类型不同，按 domain 和 code 认成同一种统一错误。
    /// 认不出的报 Provider，带上 domain 和 code。
    #[test]
    fn system_errors_map_to_model_errors() {
        const OLD: &str = "FoundationModels.LanguageModelSession.GenerationError";
        const NEW: &str = "FoundationModels.LanguageModelError";
        let error = |domain: &str, code: i64| SystemError {
            domain: domain.to_string(),
            code,
            message: "说明".to_string(),
        };
        let code_of = |domain: &str, code: i64| {
            let err = to_model_error(error(domain, code));
            let text = err.to_string();
            match err {
                ModelError::RateLimited(_) => "RateLimited".to_string(),
                ModelError::Provider(_) => format!("Provider: {text}"),
                _ => text[1..text.find(']').unwrap()].to_string(),
            }
        };
        let cases = [
            (OLD, 0, "LLM_CONTEXT_OVERFLOW"),
            (NEW, 0, "LLM_CONTEXT_OVERFLOW"),
            (OLD, 1, "LLM_UNAVAILABLE_MODEL_NOT_READY"),
            (OLD, 2, "LLM_REFUSED"),
            (NEW, 2, "LLM_REFUSED"),
            (OLD, 8, "LLM_REFUSED"),
            (NEW, 3, "LLM_REFUSED"),
            (OLD, 4, "LLM_UNAVAILABLE_LANGUAGE_NOT_SUPPORTED"),
            (NEW, 7, "LLM_UNAVAILABLE_LANGUAGE_NOT_SUPPORTED"),
            (OLD, 6, "RateLimited"),
            (NEW, 1, "RateLimited"),
        ];
        for (domain, code, want) in cases {
            assert_eq!(code_of(domain, code), want, "{domain} {code}");
        }
        let other = code_of(NEW, 8);
        assert!(other.starts_with("Provider: "), "{other}");
        assert!(other.contains(NEW) && other.contains(" 8"), "{other}");
    }

    /// 真机：本机能用时读到上下文长度。只在支持 Apple Intelligence 的 Mac 上跑：
    /// `cargo test --lib apple_fm::tests::live_ -- --ignored`
    #[test]
    #[ignore]
    fn live_status_reads_context_size() {
        let context = check("zh").expect("这台 Mac 能用 Apple Intelligence");
        assert!(context >= 4096, "{context}");
    }

    /// 真机：生成一段中文，去掉首尾空白，不报用量。
    #[tokio::test]
    #[ignore]
    async fn live_generates_text() {
        let model = AppleFmModel::new("zh");
        let mut req = request(vec![user("用两句话介绍东京。")], Vec::new());
        req.system = "你是一个简洁的助手。".to_string();
        req.max_output_tokens = 256;
        let res = model.generate(req).await.expect("生成成功");
        assert!(!res.text.is_empty());
        assert_eq!(res.text, res.text.trim());
        assert!(res.usage.input_tokens.is_none());
    }

    /// 真机：材料超长时，发之前就报超长；绕过预检直接发，系统报的错也认成超长。
    #[tokio::test]
    #[ignore]
    async fn live_overflow_reports_context_overflow() {
        let long = "今天写了很多代码。".repeat(2000);
        let model = AppleFmModel::new("zh");
        let err = model
            .generate(request(vec![user(&long)], Vec::new()))
            .await
            .unwrap_err();
        assert!(matches!(err, ModelError::ContextOverflow(_)), "{err:?}");

        let outcome = ffi::generate("系统提示词", &long, 256, None).wait().await;
        let err = to_model_error(outcome.error.expect("系统报错"));
        assert!(matches!(err, ModelError::ContextOverflow(_)), "{err:?}");
    }

    /// 真机：取消后马上收到结果，说明 Swift 停了任务，也调了回调。
    #[tokio::test]
    #[ignore]
    async fn live_cancel_stops_generation() {
        let pending = ffi::generate("你是作家。", "写一篇两千字的小说。", 1024, None);
        let started = std::time::Instant::now();
        pending.cancel();
        let outcome = pending.wait().await;
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
        let error = outcome.error.expect("取消时报错");
        assert_eq!(error.domain, "Swift.CancellationError", "{error:?}");
    }
}
