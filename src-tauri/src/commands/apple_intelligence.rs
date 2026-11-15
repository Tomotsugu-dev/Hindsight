//! Apple Intelligence（macOS 自带的本机模型）的状态命令。

use serde::Serialize;
use tauri::State;

use crate::ai::model::UnavailableReason;
use crate::repo::settings;
use crate::storage::DbPool;

/// [`apple_intelligence_status`] 的返回值。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppleIntelligenceStatus {
    /// 用不了的原因。`None` 表示能用。
    pub unavailable: Option<UnavailableReason>,
    /// 这台 Mac 的上下文长度（token）。用不了时是 `None`。
    pub context_size: Option<u32>,
}

/// 查这台电脑能不能用 Apple Intelligence 写日报。按设置里的提示词语言查。
#[tauri::command]
pub async fn apple_intelligence_status(
    pool: State<'_, DbPool>,
) -> Result<AppleIntelligenceStatus, String> {
    let cfg = settings::load(&pool).await.map_err(String::from)?;
    Ok(match check(&cfg.ai.prompt_language) {
        Ok(context_size) => AppleIntelligenceStatus {
            unavailable: None,
            context_size: Some(context_size),
        },
        Err(reason) => AppleIntelligenceStatus {
            unavailable: Some(reason),
            context_size: None,
        },
    })
}

#[cfg(target_os = "macos")]
fn check(prompt_language: &str) -> Result<u32, UnavailableReason> {
    crate::ai::providers::apple_fm::check(prompt_language)
}

#[cfg(not(target_os = "macos"))]
fn check(_prompt_language: &str) -> Result<u32, UnavailableReason> {
    Err(UnavailableReason::NotMacos)
}
