//! AI 总结相关后端代码。
//!
//! Phase 1A 只放配置类型（[`config`]）；
//! Phase 1B 起会加 `llm`（chat completion 客户端）/ `prompt`（system prompt 构造）/
//! `image`（截图编码）等子模块。
pub mod config;
