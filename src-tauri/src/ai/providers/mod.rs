//! Adapters for model services.
//! Each adapter implements [`crate::ai::model::LanguageModelOps`].

#[cfg(target_os = "macos")]
pub mod apple_fm;
pub mod chat_completions;
