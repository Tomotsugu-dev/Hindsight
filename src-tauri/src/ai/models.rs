//! AI 模型（GGUF）的目录路由 + 扫描 + 删除（Phase 1B-β β.1）。
//!
//! - [`default_root_dir`] / [`root_dir`]：路径解析，跟用户在 设置→数据 里
//!   配的 `ai.models_path` 联动
//! - [`list_local`]：扫描目录拿到 `.gguf` 文件清单（main + mmproj 平等列）
//! - [`delete`]：删一个文件
//!
//! HF 下载在 β.2 加；模型选中 + 跟 supervisor 联动在 β.3。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::ai::config::AiConfig;
use crate::error::{Error, Result};

/// HF 模型下载超时：1h（Qwen2.5-VL-7B 体积较大，慢网下载需要时间）
const HF_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// 进度回调最小间隔，避免一秒上千次事件
const HF_PROGRESS_INTERVAL: Duration = Duration::from_millis(120);

/// HF 文件直链 URL（main 分支）
fn hf_url(repo: &str, file: &str) -> String {
    format!("https://huggingface.co/{repo}/resolve/main/{file}")
}

/// 默认模型目录：`<data_root>/ai/models/`。
///
/// 跟 [`crate::ai::binary`] 的 `<data_root>/ai/bin/` 同根，方便用户一眼
/// 看出"AI 引擎数据都在这"。`db_path_dir` 失败时回退相对路径——理论上
/// 只在测试或非常异常的环境出现。
pub fn default_root_dir() -> PathBuf {
    crate::storage::db_path_dir()
        .map(|p| p.join("ai").join("models"))
        .unwrap_or_else(|_| PathBuf::from("ai").join("models"))
}

/// 实际生效的模型目录：用户在 settings 里配了就用配的，没配走默认。
///
/// settings 加载时如果 `ai.models_path` 为空会自动填充成默认路径
/// （[`crate::repo::settings::load`] 里处理），所以正常情况下到这里
/// `cfg.models_path` 都非空，这里的兜底只是防御性。
pub fn root_dir(cfg: &AiConfig) -> PathBuf {
    let trimmed = cfg.models_path.trim();
    if trimmed.is_empty() {
        default_root_dir()
    } else {
        PathBuf::from(trimmed)
    }
}

/// 一个本地 GGUF 文件——不区分"模型本体"和"mmproj"，UI 自己按
/// `is_mmproj` 字段区分展示。
///
/// vision 模型部署时是 main + mmproj 一对，但磁盘上就是两个独立的 .gguf；
/// 这里直观地按文件列，不强加 bundle 概念，免得用户手动放进来的
/// 文件被 bundle 逻辑过滤掉。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelEntry {
    /// 文件名（含 `.gguf` 后缀）
    pub filename: String,
    /// 绝对路径，前端可以直接拿来传给后端的 select / delete
    pub path: String,
    /// 字节数；前端用来格式化 "1.93 GB" 之类
    pub size_bytes: u64,
    /// 文件名包含 `mmproj` 标记 → vision 投影文件，不是主模型
    pub is_mmproj: bool,
}

/// 扫描模型目录，返回所有 `.gguf` 文件。目录不存在时返回空 `Vec`，
/// 不当错误（用户首次进入还没下载是正常状态）。
pub async fn list_local(cfg: &AiConfig) -> Result<Vec<ModelEntry>> {
    let dir = root_dir(cfg);
    if !tokio::fs::try_exists(&dir).await.map_err(Error::Io)? {
        return Ok(Vec::new());
    }

    let mut entries: Vec<ModelEntry> = Vec::new();
    let mut read = tokio::fs::read_dir(&dir).await.map_err(Error::Io)?;
    while let Some(item) = read.next_entry().await.map_err(Error::Io)? {
        let path = item.path();
        if !path.is_file() {
            continue;
        }
        let Some(filename) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // 大小写无关匹配 .gguf
        if !filename.to_ascii_lowercase().ends_with(".gguf") {
            continue;
        }
        let size_bytes = item.metadata().await.map(|m| m.len()).unwrap_or(0);
        entries.push(ModelEntry {
            filename: filename.to_string(),
            path: path.to_string_lossy().into_owned(),
            size_bytes,
            // 命名约定：`*-mmproj-*.gguf` 或 `mmproj.gguf` 之类，名字里
            // 必含 mmproj。HF 上 ggml-org / Mungert 等几个主流维护者都用这个
            is_mmproj: filename.to_ascii_lowercase().contains("mmproj"),
        });
    }

    // 按文件名排序，输出稳定，UI 显示也整齐
    entries.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(entries)
}

/// 删除单个 GGUF 文件。文件名必须是 [`list_local`] 返回的那种 basename，
/// 不接受路径分隔符，避免传 `..` 跳出目录。
pub async fn delete(cfg: &AiConfig, filename: &str) -> Result<()> {
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err(Error::InvalidInput("文件名不能包含路径分隔符或 .."));
    }
    let path = root_dir(cfg).join(filename);
    if !tokio::fs::try_exists(&path).await.map_err(Error::Io)? {
        return Ok(());
    }
    tokio::fs::remove_file(&path).await.map_err(Error::Io)?;
    Ok(())
}

/// 从 HuggingFace 流式下载一个文件到 [`root_dir`] 下面。
///
/// - 文件名必须是 basename，禁路径分隔符（防穿目录）
/// - 已存在 + 大小匹配 `expected_bytes`（容差 1% 或 1KB）→ 视作"已下"，直接返回
/// - 写到 `<file>.partial`，下载完 rename 到 `<file>`，避免半成品被当真
/// - 失败时清理 `.partial`
///
/// `progress` 在下载阶段被节流（~120ms 一帧）回调，参数是
/// `(downloaded_bytes, content_length_or_None)`。
pub async fn download_from_hf<F>(
    cfg: &AiConfig,
    repo: &str,
    file: &str,
    expected_bytes: u64,
    mut progress: F,
) -> Result<PathBuf>
where
    F: FnMut(u64, Option<u64>) + Send,
{
    if file.contains('/') || file.contains('\\') || file.contains("..") {
        return Err(Error::InvalidInput("文件名不能包含路径分隔符或 .."));
    }

    let dir = root_dir(cfg);
    tokio::fs::create_dir_all(&dir).await.map_err(Error::Io)?;
    let dest = dir.join(file);
    let temp = dir.join(format!("{file}.partial"));

    // 已下：大小匹配则直接复用
    if expected_bytes > 0 {
        if let Ok(meta) = tokio::fs::metadata(&dest).await {
            let actual = meta.len();
            // 容差 1% 或 1KB（取大者）：HF size metadata 偶尔有几字节差
            let tol = (expected_bytes / 100).max(1024);
            if actual.abs_diff(expected_bytes) <= tol {
                progress(actual, Some(actual));
                return Ok(dest);
            }
        }
    }

    // 清掉旧的 .partial（上次失败留下的）
    let _ = tokio::fs::remove_file(&temp).await;

    let url = hf_url(repo, file);
    if let Err(e) = stream_to_file(&url, &temp, &mut progress).await {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(e);
    }

    // 落盘成功 → atomic rename 到最终路径
    // Windows 上 rename 不支持覆盖，先删 dest 如果在
    let _ = tokio::fs::remove_file(&dest).await;
    tokio::fs::rename(&temp, &dest).await.map_err(Error::Io)?;

    Ok(dest)
}

async fn stream_to_file<F>(url: &str, dest: &Path, progress: &mut F) -> Result<()>
where
    F: FnMut(u64, Option<u64>) + Send,
{
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    let client = reqwest::Client::builder()
        .timeout(HF_DOWNLOAD_TIMEOUT)
        .build()?;
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(Error::Other(format!(
            "下载失败：HTTP {} {url}",
            resp.status()
        )));
    }
    let total = resp.content_length();
    let mut file = tokio::fs::File::create(dest).await.map_err(Error::Io)?;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_emit = Instant::now();
    progress(0, total);

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await.map_err(Error::Io)?;
        downloaded += chunk.len() as u64;
        let now = Instant::now();
        let is_complete = total.map(|t| downloaded >= t).unwrap_or(false);
        if is_complete || now.duration_since(last_emit) >= HF_PROGRESS_INTERVAL {
            progress(downloaded, total);
            last_emit = now;
        }
    }
    file.flush().await.map_err(Error::Io)?;
    Ok(())
}
