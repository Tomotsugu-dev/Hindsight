//! AI 总结相关后端代码。
//!
//! - [`config`] AI 用户配置（嵌进 Settings.ai）
//! - [`platform`] 当前主机 → llama.cpp release asset 路由
//! - [`binary`] 引擎二进制下载 / 校验 / 安装
//! - [`server`] llama-server 子进程管理（启动 / 停止 / health / idle 收回）
//! - [`models`] GGUF 扫描 / 列表
//! - [`recommended`] 推荐模型表
//! - [`llm`] chat completion 客户端（本地 + 外部）
//! - [`prompt`] system / user prompt 拼装（多语言）
//! - [`image`] 截图缩放 / data URI
//! - [`job_guard`] 子进程组保护（Windows Job Object / Linux+macOS setpgid）
//! - [`summary`] 总结编排 façade（实际实现在 `summary_runner` / `summary_operations` /
//!   `summary_overrides` / `summary_progress`）

pub mod binary;
pub mod config;
pub mod dedup;
pub mod embedding;
pub mod embedding_runtime;
pub mod image;
pub mod job_guard;
pub mod llm;
pub mod models;
pub mod ocr;
pub mod platform;
pub mod prompt;
pub mod recommended;
pub mod server;
pub mod summary;
pub mod summary_operations;
pub mod summary_overrides;
pub mod summary_progress;
pub mod summary_runner;
pub mod weekly_runner;
