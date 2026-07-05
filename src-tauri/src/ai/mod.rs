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
//! - [`job_guard`] 子进程组保护（Windows Job Object / Linux+macOS setpgid）
//! - [`summary`] 总结编排 façade（实际实现在 `summary_runner` / `summary_operations` /
//!   `summary_overrides` / `summary_progress`）

pub mod binary;
pub mod config;
pub mod embedding_runtime;
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

/// 统一的 ONNX 会话起点:线程帽 + 关 memory pattern + GPU 执行后端。
///
/// Windows 上注册 DirectML EP(任何 DX12 GPU 含核显都能用,系统自带无需装
/// CUDA);运行时 dll 不带 DML 或设备不支持时,ort 在 session 创建期自动
/// 回退 CPU,只留一条日志——OCR 引擎因此天然双路。
pub(crate) fn onnx_session_builder(
    intra_threads: usize,
) -> ort::Result<ort::session::builder::SessionBuilder> {
    let builder = ort::session::Session::builder()?
        .with_intra_threads(intra_threads)?
        // 动态形状下 memory pattern 只会放大内存池滞留,关掉
        .with_memory_pattern(false)?;
    #[cfg(target_os = "windows")]
    let builder = builder.with_execution_providers([
        ort::execution_providers::DirectMLExecutionProvider::default().build(),
    ])?;
    Ok(builder)
}
