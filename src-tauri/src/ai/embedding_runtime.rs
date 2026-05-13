//! ONNX Runtime 动态库的下载、安装、状态管理。
//!
//! `embedding.rs` 用 ort + load-dynamic 在运行期 dlopen `libonnxruntime.{dylib,dll,so}`，
//! Hindsight 不静态链接、也不打进 release 包（dylib ~30MB 跟 llama.cpp binary 体积接近，
//! 让用户**用到**才下，跟 [`super::binary`] 同款 lazy-fetch 模式）。
//!
//! 安装目录：`<data_root>/ai/runtime/libonnxruntime.{dylib,dll,so}`
//! 跟 `<data_root>/ai/bin/` 同级 ——`ai/` 下"binary（llama.cpp）" + "runtime（ort）"分两个子目录。
//!
//! 不做：断点续传、镜像 fallback、并发分片下载——都是 v2 优化项（跟 binary.rs 一致）。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::ai::binary::DownloadPhase;
use crate::ai::platform::{self, Platform};
use crate::error::{Error, Result};

const RUNTIME_SUBDIR: &str = "ai/runtime";

/// 当前 PIN 的 onnxruntime 版本——升级时改这一处 + 同步 sha256 表（如启用校验）。
pub const PINNED_VERSION: &str = "1.22.0";

/// 进度节流，跟 binary.rs 对齐。
const EMIT_INTERVAL: Duration = Duration::from_millis(120);

/// onnxruntime 包 ~30MB，给 10 分钟超时（国内慢网兜底）。
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// 给前端展示推理库安装状态。跟 [`super::binary::EngineBinaryStatus`] 平行。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    /// dylib 文件是否已落到磁盘
    pub installed: bool,
    /// 已安装版本（来自版本标记文件）；不存在 = `None`
    pub installed_version: Option<String>,
    /// Hindsight 当前 PIN 的 onnxruntime 版本
    pub current_pin: String,
    /// 估算下载体积（字节）
    pub estimated_bytes: u64,
}

/// 拉当前推理库安装状态。不联网。
pub fn status() -> Result<RuntimeStatus> {
    Ok(RuntimeStatus {
        installed: is_installed()?,
        installed_version: read_installed_version().ok(),
        current_pin: PINNED_VERSION.to_string(),
        estimated_bytes: estimated_bytes(),
    })
}

/// dylib 完整路径（不保证存在）。embedding.rs 启动时调来设 `ORT_DYLIB_PATH`。
pub fn dylib_path() -> Result<PathBuf> {
    Ok(install_root()?.join(default_dylib_name()))
}

/// dylib 文件是否已落盘。
pub fn is_installed() -> Result<bool> {
    Ok(dylib_path()?.exists())
}

/// 下载 → 解压 → 提取 dylib → 标版本。失败时清理临时文件。
///
/// `progress` 回调阶段语义跟 [`super::binary::download`] 一致：
/// - `Downloading`: 已下字节累计 + content_length（节流 ~120ms）
/// - `Verifying` / `Extracting` / `Done`: 单点信号
pub async fn download<F>(mut progress: F) -> Result<()>
where
    F: FnMut(DownloadPhase, u64, Option<u64>) + Send,
{
    let p = platform::detect();
    let asset = asset_name(p)?;
    let url = release_url(asset);

    let dir = install_root()?;
    std::fs::create_dir_all(&dir).map_err(|e| Error::EngineBinary {
        stage: "mkdir",
        details: format!("path={} err={e}", dir.display()),
    })?;

    let temp_archive = dir.join(format!("{asset}.partial"));

    // ── HEAD 拿大小（前端进度条用）───────────────
    let client = reqwest::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .build()?;
    let total: Option<u64> = match client.head(&url).send().await {
        Ok(resp) => resp.content_length(),
        Err(_) => None,
    };
    progress(DownloadPhase::Downloading, 0, total);

    // ── 下载 ─────────────────────────────────────
    let res = stream_download_with(&client, &url, &temp_archive, &mut progress).await;
    if let Err(e) = res {
        let _ = std::fs::remove_file(&temp_archive);
        return Err(e);
    }
    let downloaded_bytes = std::fs::metadata(&temp_archive)
        .map(|m| m.len())
        .unwrap_or(0);
    progress(DownloadPhase::Downloading, downloaded_bytes, total);

    // ── 校验（可选，v1 跟 binary.rs 一致跳过）───
    progress(DownloadPhase::Verifying, downloaded_bytes, total);
    log::info!("onnxruntime sha256 未录入（version={PINNED_VERSION} asset={asset}），跳过校验");

    // ── 解压 + 提取 dylib ────────────────────────
    progress(DownloadPhase::Extracting, downloaded_bytes, total);
    let archive_clone = temp_archive.clone();
    let dir_clone = dir.clone();
    let asset_clone = asset.to_string();
    let res = tokio::task::spawn_blocking(move || {
        extract_dylib(&archive_clone, &asset_clone, &dir_clone)
    })
    .await
    .map_err(|e| Error::EngineBinary {
        stage: "extract",
        details: format!("join: {e}"),
    })?;
    if let Err(e) = res {
        let _ = std::fs::remove_file(&temp_archive);
        return Err(e);
    }
    let _ = std::fs::remove_file(&temp_archive);

    write_installed_version(PINNED_VERSION)?;
    progress(DownloadPhase::Done, 0, None);
    Ok(())
}

/// 删除推理库安装目录。`commands::ai_binary::delete_binary` 同步调一下，
/// 让"删 AI 引擎"按钮真的把 binary + runtime 一起清掉。
pub async fn delete() -> Result<()> {
    let dir = install_root()?;
    if dir.exists() {
        // 简单 remove_dir_all——dylib 不会被任何子进程锁住（ort 在主进程
        // 用过就 dlopen 了，但 macOS / Linux unlink 已 mmap 文件无碍；
        // Windows 上若主进程已 dlopen 会拒绝 unlink，那时让用户重启 app
        // 后再删——这是 v1 简化策略）。
        std::fs::remove_dir_all(&dir).map_err(|e| Error::EngineBinary {
            stage: "cleanup",
            details: format!("remove_dir_all 失败: path={} err={e}", dir.display()),
        })?;
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────
//  内部辅助
// ─────────────────────────────────────────────────────────

/// 平台 → onnxruntime release asset 文件名。GitHub release URL 用这个拼。
fn asset_name(p: Platform) -> Result<&'static str> {
    Ok(match p {
        Platform::MacOSArm64Metal => "onnxruntime-osx-arm64-1.22.0.tgz",
        Platform::MacOSX64 => "onnxruntime-osx-x86_64-1.22.0.tgz",
        Platform::WindowsX64Cpu
        | Platform::WindowsX64Cuda12
        | Platform::WindowsX64Cuda13
        | Platform::WindowsX64Vulkan => "onnxruntime-win-x64-1.22.0.zip",
        Platform::LinuxX64Cpu => "onnxruntime-linux-x64-1.22.0.tgz",
    })
}

/// ort load-dynamic 默认搜索的 dylib 文件名——把解压出来的版本化文件
/// （`libonnxruntime.1.22.0.dylib` 等）重命名成这个，避免还要再设 ORT_DYLIB_PATH。
fn default_dylib_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    }
}

/// "约 30 MB" 的展示——给 UI 显示下载体积量级。
fn estimated_bytes() -> u64 {
    30 * 1024 * 1024
}

fn release_url(asset: &str) -> String {
    format!("https://github.com/microsoft/onnxruntime/releases/download/v{PINNED_VERSION}/{asset}")
}

fn install_root() -> Result<PathBuf> {
    Ok(crate::storage::db_path_dir()?.join(RUNTIME_SUBDIR))
}

fn version_file() -> Result<PathBuf> {
    Ok(install_root()?.join(".onnxruntime-version"))
}

fn read_installed_version() -> Result<String> {
    let f = version_file()?;
    let s = std::fs::read_to_string(&f).map_err(Error::from)?;
    Ok(s.trim().to_string())
}

fn write_installed_version(version: &str) -> Result<()> {
    let f = version_file()?;
    std::fs::write(&f, version).map_err(Error::from)?;
    Ok(())
}

async fn stream_download_with<F>(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    progress: &mut F,
) -> Result<()>
where
    F: FnMut(DownloadPhase, u64, Option<u64>) + Send,
{
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(Error::EngineBinary {
            stage: "download",
            details: format!("HTTP {} {}", resp.status(), url),
        });
    }
    let total = resp.content_length();
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| Error::EngineBinary {
            stage: "download",
            details: format!(
                "File::create 失败: path={} err={e}（kind={:?}）",
                dest.display(),
                e.kind()
            ),
        })?;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_emit = std::time::Instant::now();
    progress(DownloadPhase::Downloading, 0, total);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await.map_err(Error::from)?;
        downloaded += chunk.len() as u64;
        let now = std::time::Instant::now();
        let is_complete = total.map(|t| downloaded >= t).unwrap_or(false);
        if is_complete || now.duration_since(last_emit) >= EMIT_INTERVAL {
            progress(DownloadPhase::Downloading, downloaded, total);
            last_emit = now;
        }
    }
    file.flush().await.map_err(Error::from)?;
    Ok(())
}

/// 从 archive 里**只**提取 dylib 文件，写到 `dest_dir/<default_name>`。
///
/// onnxruntime tarball 长这样：
///   onnxruntime-osx-arm64-1.22.0/
///     lib/libonnxruntime.1.22.0.dylib   ← 我们要的
///     lib/libonnxruntime.dylib          ← 软链接（可能存在）
///     include/...
///     LICENSE
/// 我们只挑那个 versioned 文件名（dlopen 真正打开的实体文件），重命名为
/// `default_dylib_name()`。其它内容（headers、licenses）不留——节省 ~10MB 磁盘。
fn extract_dylib(archive: &Path, asset_name: &str, dest_dir: &Path) -> Result<()> {
    let target = dest_dir.join(default_dylib_name());
    if target.exists() {
        let _ = std::fs::remove_file(&target);
    }
    if asset_name.ends_with(".zip") {
        extract_dylib_from_zip(archive, &target)
    } else if asset_name.ends_with(".tgz") || asset_name.ends_with(".tar.gz") {
        extract_dylib_from_tar_gz(archive, &target)
    } else {
        Err(Error::EngineBinary {
            stage: "extract",
            details: format!("不识别的压缩格式：{asset_name}"),
        })
    }
}

fn extract_dylib_from_zip(archive: &Path, target: &Path) -> Result<()> {
    let file = std::fs::File::open(archive).map_err(Error::from)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| Error::EngineBinary {
        stage: "extract",
        details: format!("zip open: {e}"),
    })?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| Error::EngineBinary {
            stage: "extract",
            details: format!("zip entry {i}: {e}"),
        })?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if dylib_entry_matches(&name) {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(Error::from)?;
            }
            let mut outfile = std::fs::File::create(target).map_err(Error::from)?;
            std::io::copy(&mut entry, &mut outfile).map_err(Error::from)?;
            return Ok(());
        }
    }
    Err(Error::EngineBinary {
        stage: "extract",
        details: format!(
            "zip 里没找到 onnxruntime dylib（target={}）",
            target.display()
        ),
    })
}

fn extract_dylib_from_tar_gz(archive: &Path, target: &Path) -> Result<()> {
    let file = std::fs::File::open(archive).map_err(Error::from)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    for entry in tar.entries().map_err(Error::from)? {
        let mut entry = entry.map_err(Error::from)?;
        // tar 包里软链接也算一种 entry——对应 macOS / Linux tarball 里
        // `libonnxruntime.dylib` → `libonnxruntime.1.22.0.dylib`。
        // 只挑真实文件（version 化的那个），跳过 symlinks，避免拷出来还是断链。
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path_buf = entry.path().map_err(Error::from)?.to_path_buf();
        let name = path_buf.to_string_lossy().to_string();
        if dylib_entry_matches(&name) {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(Error::from)?;
            }
            let mut outfile = std::fs::File::create(target).map_err(Error::from)?;
            std::io::copy(&mut entry, &mut outfile).map_err(Error::from)?;
            return Ok(());
        }
    }
    Err(Error::EngineBinary {
        stage: "extract",
        details: format!(
            "tar.gz 里没找到 onnxruntime dylib（target={}）",
            target.display()
        ),
    })
}

/// archive 内部条目相对路径是不是真正的 onnxruntime dylib。
/// 只挑 lib/ 下的 versioned 文件（带 1.22.0 那个），跳过 symlink target。
fn dylib_entry_matches(entry_name: &str) -> bool {
    let lower = entry_name.to_lowercase();
    if cfg!(target_os = "windows") {
        // Windows ZIP: lib/onnxruntime.dll
        lower.ends_with("/lib/onnxruntime.dll") || lower.ends_with("\\lib\\onnxruntime.dll")
    } else if cfg!(target_os = "macos") {
        // macOS tarball: lib/libonnxruntime.1.22.0.dylib
        lower.contains("/lib/libonnxruntime.")
            && lower.ends_with(".dylib")
            && !lower.ends_with("/libonnxruntime.dylib")
    } else {
        // Linux tarball: lib/libonnxruntime.so.1.22.0
        lower.contains("/lib/libonnxruntime.so.") && !lower.ends_with("/libonnxruntime.so")
    }
}
