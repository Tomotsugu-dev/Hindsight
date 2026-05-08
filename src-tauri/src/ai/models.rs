//! AI 模型（GGUF）的目录路由 + 扫描 + 删除 + HF 下载（含暂停 / 续传）。
//!
//! - [`default_root_dir`] / [`root_dir`]：路径解析，跟用户在 设置→数据 里
//!   配的 `ai.models_path` 联动
//! - [`list_local`]：扫描目录拿到 `.gguf` 文件清单（main + mmproj 平等列）
//! - [`list_partials`]：扫描 `<file>.partial` 半成品 + 已下字节数，给 UI 渲染"继续"
//! - [`delete`]：删一个文件
//! - [`download_from_hf`]：流式下载，断点续传，可被外部 cancel 优雅暂停（保留 .partial）
//! - [`set_cancel`] / [`is_cancelled`] / [`clear_cancel`]：给 [`download_from_hf`] 配套
//!   的 cancel signal，前端 invoke `cancel_model_download` 时翻 flag

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::ai::config::AiConfig;
use crate::error::{Error, Result};

/// 全局 cancel signal map：文件名 → AtomicBool。
///
/// 前端 invoke `cancel_model_download(file)` 时翻这个 flag；
/// [`download_from_hf`] 每写一个 chunk 后检查，true 时优雅退出（保留 .partial）。
/// 下载结束（成功 / 失败 / 取消）后调 [`clear_cancel`] 移除条目。
fn cancel_map() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static MAP: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 注册某文件的 cancel signal。重复注册时返回旧的 flag（让旧任务也能走 cancel 路径）。
fn register_cancel(file: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    cancel_map()
        .lock()
        .expect("cancel_map mutex poisoned")
        .insert(file.to_string(), Arc::clone(&flag));
    flag
}

/// 给指定文件翻 cancel flag。文件没在下载时静默返回（前端可能在文件下完后才点取消）。
pub fn set_cancel(file: &str) {
    if let Some(flag) = cancel_map()
        .lock()
        .expect("cancel_map mutex poisoned")
        .get(file)
    {
        flag.store(true, Ordering::Relaxed);
    }
}

/// 下载结束（成功 / 失败 / 取消）后清掉 map 里的条目。
fn clear_cancel(file: &str) {
    cancel_map()
        .lock()
        .expect("cancel_map mutex poisoned")
        .remove(file);
}

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

/// 半成品下载条目——给前端渲染"继续"按钮 + 已下进度。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialEntry {
    /// 目标文件名（不含 `.partial` 后缀）
    pub filename: String,
    /// 已下字节数（即 `<file>.partial` 当前大小）
    pub downloaded_bytes: u64,
}

/// 扫描模型目录，返回所有 `<file>.gguf.partial` 半成品的 (target_filename, size)。
/// 用 `<file>.partial` 后缀识别——[`download_from_hf`] 落盘前一直在写这个名字。
pub async fn list_partials(cfg: &AiConfig) -> Result<Vec<PartialEntry>> {
    let dir = root_dir(cfg);
    if !tokio::fs::try_exists(&dir).await.map_err(Error::Io)? {
        return Ok(Vec::new());
    }
    let mut out: Vec<PartialEntry> = Vec::new();
    let mut read = tokio::fs::read_dir(&dir).await.map_err(Error::Io)?;
    while let Some(item) = read.next_entry().await.map_err(Error::Io)? {
        let path = item.path();
        if !path.is_file() {
            continue;
        }
        let Some(filename) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(target) = filename.strip_suffix(".partial") else {
            continue;
        };
        let downloaded_bytes = item.metadata().await.map(|m| m.len()).unwrap_or(0);
        out.push(PartialEntry {
            filename: target.to_string(),
            downloaded_bytes,
        });
    }
    out.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(out)
}

/// 从 HuggingFace 流式下载一个文件到 [`root_dir`] 下面。
///
/// - HF URL 用 `repo` + `hf_file` 构造（直链 `/resolve/main/`）
/// - **落盘文件名**用 `save_as`（不指定时回落到 `hf_file`）——多个推荐 rec 的 mmproj
///   在 HF 上常常同名（unsloth 系列都是 `mmproj-F16.gguf`），落盘必须给唯一名，
///   不然不同 rec 的 mmproj 会互相覆盖
/// - 已存在 + 大小匹配 `expected_bytes`（容差 1% 或 1KB）→ 视作"已下"，直接返回
/// - `<save_as>.partial` 已存在 → 发 `Range: bytes=N-` 续传
/// - 中途被 [`set_cancel`] 翻 flag → 保留 `.partial`，返回 `Error::DownloadCancelled`
/// - 真失败（网络中断等） → **保留** `.partial` 让用户下次续传
/// - 完成后 atomic rename 到 `<save_as>`
///
/// cancel signal map 用 `save_as` 作 key（前端按相同名字调 cancel_model_download）。
/// progress 回调里 emit 的 file 字段也应该用 `save_as`，让前端能按落盘名索引。
pub async fn download_from_hf<F>(
    cfg: &AiConfig,
    repo: &str,
    hf_file: &str,
    save_as: Option<&str>,
    expected_bytes: u64,
    mut progress: F,
) -> Result<PathBuf>
where
    F: FnMut(u64, Option<u64>) + Send,
{
    if hf_file.contains('/') || hf_file.contains('\\') || hf_file.contains("..") {
        return Err(Error::InvalidInput("HF 文件名不能包含路径分隔符或 .."));
    }
    let local_name = save_as.unwrap_or(hf_file);
    if local_name.contains('/') || local_name.contains('\\') || local_name.contains("..") {
        return Err(Error::InvalidInput("save_as 文件名不能包含路径分隔符或 .."));
    }

    let dir = root_dir(cfg);
    tokio::fs::create_dir_all(&dir).await.map_err(Error::Io)?;
    let dest = dir.join(local_name);
    let temp = dir.join(format!("{local_name}.partial"));

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

    // 检测半成品续传：partial 存在 + 已下字节数 > 0 → Range request 续
    let resume_from = match tokio::fs::metadata(&temp).await {
        Ok(m) if m.len() > 0 => Some(m.len()),
        _ => None,
    };

    // 注册 cancel signal——下面循环每写一个 chunk 检查一次。key 用 save_as，
    // 因为前端 cancel 命令传的是落盘名（progress event 也按落盘名 emit）
    let cancel = register_cancel(local_name);

    let url = hf_url(repo, hf_file);
    let result = stream_to_file(&url, &temp, resume_from, &cancel, &mut progress).await;
    clear_cancel(local_name);

    match result {
        Ok(()) => {
            // 落盘成功 → atomic rename 到最终路径
            // Windows 上 rename 不支持覆盖，先删 dest 如果在
            let _ = tokio::fs::remove_file(&dest).await;
            tokio::fs::rename(&temp, &dest).await.map_err(Error::Io)?;
            Ok(dest)
        }
        Err(Error::DownloadCancelled(_)) => {
            Err(Error::DownloadCancelled(local_name.to_string()))
        }
        Err(e) => Err(e),
    }
}

/// 流式下载到 dest 文件，支持续传 + cancel。
///
/// - `resume_from = Some(N)` 时发 `Range: bytes=N-`，文件以 append 模式打开；
///   服务器不接受 Range（返 200）则当作从头下，文件改成 truncate 重写
/// - `cancel` flag 在每个 chunk 写完后检查，true 时早 return [`Error::DownloadCancelled`]
async fn stream_to_file<F>(
    url: &str,
    dest: &Path,
    resume_from: Option<u64>,
    cancel: &Arc<AtomicBool>,
    progress: &mut F,
) -> Result<()>
where
    F: FnMut(u64, Option<u64>) + Send,
{
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    let client = reqwest::Client::builder()
        .timeout(HF_DOWNLOAD_TIMEOUT)
        .build()?;
    let mut req = client.get(url);
    if let Some(n) = resume_from {
        req = req.header("Range", format!("bytes={n}-"));
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        return Err(Error::Other(format!(
            "下载失败：HTTP {} {url}",
            resp.status()
        )));
    }

    // 服务器是否真的接受了 Range：206 Partial Content + Content-Range header
    let server_resumed = resp.status().as_u16() == 206;
    let mut downloaded: u64 = if server_resumed {
        resume_from.unwrap_or(0)
    } else {
        // 服务器返 200，意味着不支持 Range（或我们没发 Range）→ 从头写
        0
    };
    // total 算上断点之前的字节：206 时 content_length 是剩余，得加上 resume_from
    let total = resp
        .content_length()
        .map(|cl| if server_resumed { cl + downloaded } else { cl });

    let mut file = if server_resumed {
        // append 模式
        tokio::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(dest)
            .await
            .map_err(Error::Io)?
    } else {
        // truncate 重写——上次的 partial 作废
        tokio::fs::File::create(dest).await.map_err(Error::Io)?
    };
    let mut stream = resp.bytes_stream();
    let mut last_emit = Instant::now();
    progress(downloaded, total);

    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::Relaxed) {
            file.flush().await.map_err(Error::Io)?;
            return Err(Error::DownloadCancelled(String::new()));
        }
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
