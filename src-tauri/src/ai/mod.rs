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
pub mod ocr_patch;
#[cfg(target_os = "macos")]
pub mod ocr_vision;
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

/// 基础 ONNX 会话 builder:线程帽 + 关 memory pattern,**不带 GPU EP**。
pub(crate) fn onnx_session_builder(
    intra_threads: usize,
) -> ort::Result<ort::session::builder::SessionBuilder> {
    ort::session::Session::builder()?
        .with_intra_threads(intra_threads)?
        // 动态形状下 memory pattern 只会放大内存池滞留,关掉
        .with_memory_pattern(false)
}

/// 从文件建会话,Windows 上优先 DirectML GPU、失败回退 CPU——**显式两段式**。
///
/// 之前的写法把 DML 注册挂在 builder 上静默回退:失败没有任何可观测痕迹,
/// 用户以为在用 GPU 实际烧 CPU(旧 CPU 构建 dll 被进程 dlopen 后尤其如此)。
/// 现在第一段用 `error_on_failure` 强制注册,成败都打**明确日志**;
/// 失败时第二段用纯 CPU builder 重建,行为不变、真相可见。
pub(crate) fn onnx_session_from_file(
    intra_threads: usize,
    path: &std::path::Path,
) -> ort::Result<(ort::session::Session, bool)> {
    #[cfg(target_os = "windows")]
    // HINDSIGHT_OCR_CPU=1 强制 CPU:性能 A/B 与故障排查用(同 macOS 的
    // HINDSIGHT_OCR_PADDLE 惯例),生产路径不设。
    if std::env::var_os("HINDSIGHT_OCR_CPU").is_none() {
        let dml = onnx_session_builder(intra_threads)?
            .with_execution_providers([
                ort::execution_providers::DirectMLExecutionProvider::default()
                    .build()
                    .error_on_failure(),
            ])
            .and_then(|b| b.commit_from_file(path));
        match dml {
            Ok(sess) => {
                log::info!(
                    "ONNX 会话加载:DirectML GPU({})",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
                return Ok((sess, true));
            }
            Err(e) => {
                log::warn!(
                    "ONNX 会话:DirectML 不可用,回退 CPU({}): {e}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
            }
        }
    }
    let sess = onnx_session_builder(intra_threads)?.commit_from_file(path)?;
    log::info!(
        "ONNX 会话加载:CPU({})",
        path.file_name().unwrap_or_default().to_string_lossy()
    );
    Ok((sess, false))
}

/// [`onnx_session_from_file`] 的内存字节变体——运行时改图后的模型
/// (见 [`ocr_patch`])不落盘直接建会话。DML/CPU 两段式与文件版一致。
pub(crate) fn onnx_session_from_memory(
    intra_threads: usize,
    bytes: &[u8],
    label: &str,
) -> ort::Result<(ort::session::Session, bool)> {
    #[cfg(target_os = "windows")]
    if std::env::var_os("HINDSIGHT_OCR_CPU").is_none() {
        let dml = onnx_session_builder(intra_threads)?
            .with_execution_providers([
                ort::execution_providers::DirectMLExecutionProvider::default()
                    .build()
                    .error_on_failure(),
            ])
            .and_then(|b| b.commit_from_memory(bytes));
        match dml {
            Ok(sess) => {
                log::info!("ONNX 会话加载:DirectML GPU({label})");
                return Ok((sess, true));
            }
            Err(e) => {
                log::warn!("ONNX 会话:DirectML 不可用,回退 CPU({label}): {e}");
            }
        }
    }
    let sess = onnx_session_builder(intra_threads)?.commit_from_memory(bytes)?;
    log::info!("ONNX 会话加载:CPU({label})");
    Ok((sess, false))
}
