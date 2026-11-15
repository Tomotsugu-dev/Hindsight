//! macOS 自带的本机模型 Apple Intelligence（Apple Foundation Models）。
//!
//! 在进程内通过 Swift 调系统框架，不走 HTTP。Swift 只包系统调用，判断都在这里。

mod ffi;

use serde::Deserialize;

use crate::ai::model::UnavailableReason;

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

    /// 真机：本机能用时读到上下文长度。只在支持 Apple Intelligence 的 Mac 上跑：
    /// `cargo test --lib apple_fm::tests::live_ -- --ignored`
    #[test]
    #[ignore]
    fn live_status_reads_context_size() {
        let context = check("zh").expect("这台 Mac 能用 Apple Intelligence");
        assert!(context >= 4096, "{context}");
    }
}
