//! `llama-server` binary 的下载、校验、解压、安装。
//!
//! 流程：[`download`] 流式 GET → 写到 `<asset>.partial` → 可选 SHA256 →
//! 解压（zip 或 tar.gz）→ 写版本文件 → 删 .partial。
//!
//! 安装目录：`<data_root>/ai/bin/<platform_id>/build/bin/llama-server[.exe]`
//! 沿用 `bootstrap::data_root` 控制的根；用户改 data_root 后下载位置自然跟着搬。
//!
//! 不做：断点续传、镜像 fallback、并发分片下载——全是 v2 优化项。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::ai::platform::{self, Platform};
use crate::error::{Error, Result};

const ENGINE_SUBDIR: &str = "ai/bin";

/// 进度回调每次最少间隔 120ms，避免一秒钟刷上千次事件搞爆前端。
/// 完成态（done / total 命中）会立即发，不受节流。
const EMIT_INTERVAL: Duration = Duration::from_millis(120);

/// 大文件给 30 分钟超时；llama-server CUDA binary 解压前 ~150MB，
/// 国内网络慢的话十几分钟很正常
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// 给前端展示当前安装状态。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineBinaryStatus {
    /// 当前主机对应的 binary 是否已安装（文件存在）
    pub installed: bool,
    /// 已安装版本（PIN tag 字符串），来自版本标记文件；不存在 = `None`
    pub installed_version: Option<String>,
    /// Hindsight 当前 PIN 的 llama.cpp 版本
    pub current_pin: String,
    /// 当前主机被路由到的变体（"win-cuda-12.4-x64" 等）
    pub platform_id: String,
    /// 该 binary 的 asset 完整文件名（前端展示下载链接 / 调试用）
    pub asset_name: String,
    /// 估算下载体积（字节），UI 给用户显示"约 NN MB"用；
    /// 不精确，仅给量级。
    pub estimated_bytes: u64,
}

/// 下载流程进度阶段。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadPhase {
    Downloading,
    Verifying,
    Extracting,
    Done,
}

pub fn status() -> Result<EngineBinaryStatus> {
    let p = platform::detect();
    let tag = platform::PINNED_TAG;
    Ok(EngineBinaryStatus {
        installed: is_installed(p),
        installed_version: read_installed_version(p).ok(),
        current_pin: tag.to_string(),
        platform_id: platform_id(p).to_string(),
        asset_name: platform::release_asset_name(p, tag),
        estimated_bytes: platform::estimated_bytes(p),
    })
}

/// 当前主机 binary 完整路径（不保证存在；用 [`is_installed`] 判存在性）。
pub fn binary_path() -> Result<PathBuf> {
    let p = platform::detect();
    Ok(platform_dir(p)?.join(platform::binary_relative_path(p)))
}

/// 当前主机的 binary 是否已落到磁盘上。
pub fn is_installed(p: Platform) -> bool {
    let path = match platform_dir(p) {
        Ok(d) => d.join(platform::binary_relative_path(p)),
        Err(_) => return false,
    };
    path.exists()
}

/// 下载 → 校验 → 解压 → 标版本。失败时清理临时文件。
///
/// `progress` 在多个阶段被调用：
/// - `Downloading`: `(downloaded_bytes, content_length)`，节流到每 ~120ms 一次
/// - `Verifying` / `Extracting` / `Done`: `(0, None)` 单点信号
///
/// 调用方拿到 `Done` 即知安装完成。
pub async fn download<F>(mut progress: F) -> Result<()>
where
    F: FnMut(DownloadPhase, u64, Option<u64>) + Send,
{
    let p = platform::detect();
    let tag = platform::PINNED_TAG;
    let asset = platform::release_asset_name(p, tag);
    let url = format!(
        "https://github.com/ggml-org/llama.cpp/releases/download/{tag}/{asset}"
    );

    // 清掉旧版本目录，避免新解压跟旧文件混在一起。
    let dir = platform_dir(p)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(Error::from)?;
    }
    std::fs::create_dir_all(&dir).map_err(Error::from)?;
    let temp_path = dir.join(format!("{asset}.partial"));

    // ── 下载 ─────────────────────────────────────────────
    progress(DownloadPhase::Downloading, 0, None);
    if let Err(e) = stream_download(&url, &temp_path, &mut progress).await {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e);
    }

    // ── 校验（可选）─────────────────────────────────────
    progress(DownloadPhase::Verifying, 0, None);
    if let Some(expected) = platform::sha256(p, tag) {
        let temp_clone = temp_path.clone();
        let expected = expected.to_string();
        let asset_clone = asset.clone();
        let res = tokio::task::spawn_blocking(move || {
            verify_sha256(&temp_clone, &expected, &asset_clone)
        })
        .await
        .map_err(|e| Error::Other(format!("verify join: {e}")))?;
        if let Err(e) = res {
            let _ = std::fs::remove_file(&temp_path);
            return Err(e);
        }
    } else {
        log::warn!(
            "ai engine sha256 未录入（platform={:?} tag={tag}），跳过校验",
            p
        );
    }

    // ── 解压 ─────────────────────────────────────────────
    progress(DownloadPhase::Extracting, 0, None);
    let temp_clone = temp_path.clone();
    let dir_clone = dir.clone();
    let asset_clone = asset.clone();
    let res = tokio::task::spawn_blocking(move || {
        extract_archive(&temp_clone, &asset_clone, &dir_clone)
    })
    .await
    .map_err(|e| Error::Other(format!("extract join: {e}")))?;
    if let Err(e) = res {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e);
    }

    // 清理 temp + 写版本文件
    let _ = std::fs::remove_file(&temp_path);
    write_installed_version(p, tag)?;

    progress(DownloadPhase::Done, 0, None);
    Ok(())
}

/// 删除当前平台的安装目录（NSIS 卸载向导可调；UI 卸载按钮也走这里）。
pub fn delete() -> Result<()> {
    let p = platform::detect();
    let dir = platform_dir(p)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(Error::from)?;
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────
//  内部辅助
// ─────────────────────────────────────────────────────────

async fn stream_download<F>(url: &str, dest: &Path, progress: &mut F) -> Result<()>
where
    F: FnMut(DownloadPhase, u64, Option<u64>) + Send,
{
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    let client = reqwest::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .build()?;
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(Error::Other(format!(
            "下载失败：HTTP {} {}",
            resp.status(),
            url
        )));
    }
    let total = resp.content_length();

    let mut file = tokio::fs::File::create(dest).await.map_err(Error::from)?;
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

fn install_root() -> Result<PathBuf> {
    Ok(crate::storage::db_path_dir()?.join(ENGINE_SUBDIR))
}

fn platform_dir(p: Platform) -> Result<PathBuf> {
    Ok(install_root()?.join(platform_id(p)))
}

/// 平台变体的目录名——跟 llama.cpp release asset 中部命名对齐，
/// 用户翻文件夹也能直观看出"我装的是哪个版本"。
fn platform_id(p: Platform) -> &'static str {
    match p {
        Platform::WindowsX64Cpu => "win-cpu-x64",
        Platform::WindowsX64Cuda12 => "win-cuda-12.4-x64",
        Platform::WindowsX64Cuda13 => "win-cuda-13.1-x64",
        Platform::MacOSArm64Metal => "macos-arm64",
        Platform::MacOSX64 => "macos-x64",
        Platform::LinuxX64Cpu => "ubuntu-x64",
    }
}

fn version_file(p: Platform) -> Result<PathBuf> {
    Ok(platform_dir(p)?.join(".llama-cpp-version"))
}

fn read_installed_version(p: Platform) -> Result<String> {
    let f = version_file(p)?;
    let s = std::fs::read_to_string(&f).map_err(Error::from)?;
    Ok(s.trim().to_string())
}

fn write_installed_version(p: Platform, tag: &str) -> Result<()> {
    let f = version_file(p)?;
    std::fs::write(&f, tag).map_err(Error::from)?;
    Ok(())
}

fn verify_sha256(path: &Path, expected: &str, asset: &str) -> Result<()> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let mut file = std::fs::File::open(path).map_err(Error::from)?;
    std::io::copy(&mut file, &mut hasher).map_err(Error::from)?;
    let actual = format!("{:x}", hasher.finalize());
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "{asset} sha256 不匹配：期望 {expected}，实际 {actual}"
        )))
    }
}

/// 按 `asset_name` 的后缀（**不是 archive 路径的后缀**）决定压缩格式。
///
/// 实际下载文件路径会带 `.partial` 后缀作为"未完成"标记，所以靠路径
/// 判断会把 `.zip.partial` 误判成未识别格式。这里用上层透传的原始
/// asset 名（如 `llama-b9025-bin-win-cuda-13.1-x64.zip`）来做识别。
fn extract_archive(archive: &Path, asset_name: &str, dest: &Path) -> Result<()> {
    if asset_name.ends_with(".zip") {
        extract_zip(archive, dest)
    } else if asset_name.ends_with(".tar.gz") || asset_name.ends_with(".tgz") {
        extract_tar_gz(archive, dest)
    } else {
        Err(Error::Other(format!("不识别的压缩格式：{asset_name}")))
    }
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(archive).map_err(Error::from)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| Error::Other(format!("zip open: {e}")))?;

    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| Error::Other(format!("zip entry {i}: {e}")))?;
        // enclosed_name 防 .. 路径逃逸；不安全条目直接跳过
        let outpath = match entry.enclosed_name() {
            Some(p) => dest.join(p),
            None => {
                log::warn!("跳过不安全的 zip 条目: {}", entry.name());
                continue;
            }
        };

        if entry.is_dir() {
            std::fs::create_dir_all(&outpath).map_err(Error::from)?;
        } else {
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent).map_err(Error::from)?;
            }
            let mut outfile = std::fs::File::create(&outpath).map_err(Error::from)?;
            std::io::copy(&mut entry, &mut outfile).map_err(Error::from)?;
        }

        // Unix 平台保留可执行位
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&outpath, std::fs::Permissions::from_mode(mode))
                .map_err(Error::from)?;
        }
    }
    Ok(())
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(archive).map_err(Error::from)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    // tar::Archive::unpack 自带防 .. 逃逸（默认 follow_symlinks(false)）
    tar.unpack(dest).map_err(Error::from)?;
    Ok(())
}
