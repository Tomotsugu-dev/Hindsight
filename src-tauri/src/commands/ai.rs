//! AI 子系统的 Tauri 命令 façade —— 重导出 `SummaryCancel` / `fmt_send_err` 给外部使用。
//!
//! 实际命令拆在：
//! - [`crate::commands::ai_endpoint`]：外部 OpenAI 端点测试
//! - [`crate::commands::ai_binary`]：llama-server 二进制下载 / 删除 / 打开目录
//! - [`crate::commands::ai_engine`]：引擎进程的状态、启动、停止、模型切换、日志
//! - [`crate::commands::ai_models`]：本地 GGUF 模型列表 / 删除 / 下载 / 推荐表
//! - [`crate::commands::ai_summary`]：日报生成 / 重试 / 取消 / 读取
//!
//! lib.rs 的 `invoke_handler!` 直接引用上述模块的命令路径；本文件仅为
//! `crate::ai::llm::fmt_send_err` 复用与 `commands::ai::SummaryCancel` 历史
//! 引用保留兼容入口。

pub(crate) use crate::commands::ai_endpoint::fmt_send_err;
pub use crate::commands::ai_summary::SummaryCancel;
