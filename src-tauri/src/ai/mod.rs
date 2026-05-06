//! AI 总结相关后端代码。
//!
//! - [`config`] AI 用户配置（嵌进 Settings.ai）
//! - [`platform`] 当前主机 → llama.cpp release asset 路由（Phase 1B-α）
//!
//! Phase 1B-α 之后还会加：`binary`（下载 / 校验 / 安装）/ `server`（子进程管理）；
//! Phase 1B-β：`models`（GGUF 扫描 / 下载）+ `recommended`（推荐表）；
//! Phase 1B-γ：`llm`（chat completion）/ `prompt` / `image`。
pub mod binary;
pub mod config;
pub mod image;
pub mod job_guard;
pub mod llm;
pub mod models;
pub mod platform;
pub mod prompt;
pub mod recommended;
pub mod server;
pub mod summary;
