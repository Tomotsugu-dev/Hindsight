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
///
/// CUDA 平台需要的不只是 `llama-server.exe` + `ggml-cuda.dll`，还要 NVIDIA 的
/// runtime DLL（`cudart64_*.dll` / `cublas64_*.dll`）。runtime 缺失会导致
/// `ggml-cuda.dll` 加载静默失败，模型退回 CPU 跑。所以 CUDA 平台额外要求
/// 目录里存在任意版本的 `cudart64_*.dll`。
pub fn is_installed(p: Platform) -> bool {
    let dir = match platform_dir(p) {
        Ok(d) => d,
        Err(_) => return false,
    };
    if !dir.join(platform::binary_relative_path(p)).exists() {
        return false;
    }
    if matches!(
        p,
        Platform::WindowsX64Cuda12 | Platform::WindowsX64Cuda13
    ) && !has_cudart_runtime(&dir)
    {
        return false;
    }
    true
}

/// 目录里是否能找到 NVIDIA cudart runtime DLL。
///
/// llama.cpp 主 zip 里有 `ggml-cuda.dll`，但 `cudart64_*.dll` 单独打成
/// `cudart-llama-bin-win-cuda-X.Y-x64.zip` 由 [`download`] 解压进同一目录。
/// 检测的是 `cudart64_` 前缀（版本号字段会变，比如 `cudart64_12.dll` /
/// `cudart64_13.dll`），不锁死后缀版本。
fn has_cudart_runtime(dir: &Path) -> bool {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str() {
            let lower = name.to_lowercase();
            if lower.starts_with("cudart64_") && lower.ends_with(".dll") {
                return true;
            }
        }
    }
    false
}

/// 下载 → 校验 → 解压 → 标版本。失败时清理临时文件。
///
/// 对 CUDA 平台会按顺序下两个 zip（主 binary + NVIDIA cudart runtime），
/// 解压到同一目录；进度按合计字节累计上报，前端只看到一条连续的进度条。
///
/// `progress` 在多个阶段被调用：
/// - `Downloading`: `(已下字节累计, 合计 content_length)`，节流到每 ~120ms 一次
/// - `Verifying` / `Extracting` / `Done`: `(0, None)` 单点信号
///
/// 调用方拿到 `Done` 即知安装完成。
pub async fn download<F>(mut progress: F) -> Result<()>
where
    F: FnMut(DownloadPhase, u64, Option<u64>) + Send,
{
    let p = platform::detect();
    let tag = platform::PINNED_TAG;
    let main_asset = platform::release_asset_name(p, tag);
    let main_url = release_url(tag, &main_asset);

    // 准备要下的 asset 列表：主 zip 必下；CUDA 平台再加一个 cudart runtime zip。
    let mut assets: Vec<(String, String)> = vec![(main_asset.clone(), main_url)];
    if let Some(cudart) = platform::cuda_runtime_asset_name(p) {
        let url = release_url(tag, cudart);
        assets.push((cudart.to_string(), url));
    }

    // 清掉旧版本目录，避免新解压跟旧文件混在一起。
    // Windows 上 stop() 返回到 OS 真正释放 .exe / .dll 文件锁有 ~几百 ms 的窗口期
    // （内核 unmap image + Defender 实时扫描），所以 remove 用带退避的重试。
    let dir = platform_dir(p)?;
    if dir.exists() {
        remove_dir_all_retry(&dir).await?;
    }
    std::fs::create_dir_all(&dir).map_err(|e| {
        Error::Other(format!(
            "create_dir_all 失败: path={} err={e}",
            dir.display()
        ))
    })?;

    // 提前 HEAD 拿合计大小，前端进度条总数才能正确累计。
    let client = reqwest::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .build()?;
    let mut sizes: Vec<Option<u64>> = Vec::with_capacity(assets.len());
    let mut combined_total: Option<u64> = Some(0);
    for (_, url) in &assets {
        let len = match client.head(url).send().await {
            Ok(resp) => resp.content_length(),
            Err(_) => None,
        };
        if let Some(l) = len {
            if let Some(t) = combined_total.as_mut() {
                *t += l;
            }
        } else {
            combined_total = None;
        }
        sizes.push(len);
    }

    let mut total_downloaded: u64 = 0;
    progress(DownloadPhase::Downloading, 0, combined_total);

    for ((asset_name, url), size_hint) in assets.iter().zip(sizes.iter()) {
        let temp_path = dir.join(format!("{asset_name}.partial"));

        // ── 下载 ─────────────────────────────────────────
        let base = total_downloaded;
        let total_for_progress = combined_total;
        let res = {
            let mut wrapped = |_phase: DownloadPhase, d: u64, _t: Option<u64>| {
                progress(DownloadPhase::Downloading, base + d, total_for_progress);
            };
            stream_download_with(&client, url, &temp_path, &mut wrapped).await
        };
        if let Err(e) = res {
            let _ = std::fs::remove_file(&temp_path);
            return Err(e);
        }
        total_downloaded = match size_hint {
            Some(s) => base + s,
            None => match std::fs::metadata(&temp_path) {
                Ok(m) => base + m.len(),
                Err(_) => base,
            },
        };
        progress(DownloadPhase::Downloading, total_downloaded, combined_total);

        // ── 校验（可选）─────────────────────────────────
        progress(DownloadPhase::Verifying, total_downloaded, combined_total);
        if let Some(expected) = platform::sha256(p, tag) {
            let temp_clone = temp_path.clone();
            let expected = expected.to_string();
            let asset_clone = asset_name.clone();
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
                "ai engine sha256 未录入（platform={p:?} tag={tag} asset={asset_name}），跳过校验"
            );
        }

        // ── 解压 ─────────────────────────────────────────
        progress(DownloadPhase::Extracting, total_downloaded, combined_total);
        let temp_clone = temp_path.clone();
        let dir_clone = dir.clone();
        let asset_clone = asset_name.clone();
        let res = tokio::task::spawn_blocking(move || {
            extract_archive(&temp_clone, &asset_clone, &dir_clone)
        })
        .await
        .map_err(|e| Error::Other(format!("extract join: {e}")))?;
        if let Err(e) = res {
            let _ = std::fs::remove_file(&temp_path);
            return Err(e);
        }
        let _ = std::fs::remove_file(&temp_path);
    }

    write_installed_version(p, tag)?;
    progress(DownloadPhase::Done, 0, None);
    Ok(())
}

fn release_url(tag: &str, asset: &str) -> String {
    format!("https://github.com/ggml-org/llama.cpp/releases/download/{tag}/{asset}")
}

/// 带退避重试的 `remove_dir_all`。
///
/// 触发场景：刚 `kill` llama-server 后立刻删目录，Windows 内核还没 unmap .exe / .dll
/// 文件镜像，或者 Defender 仍在扫描，`remove_dir_all` 会回 PermissionDenied (os 5)。
/// 实测 ~几百 ms 即可释放，所以退避序列 0.2s/0.5s/1s/2s/3s 共 ~7s。
///
/// 重试到底还失败就把最后一次错误带路径抛上去。
async fn remove_dir_all_retry(dir: &Path) -> Result<()> {
    const BACKOFFS_MS: [u64; 5] = [200, 500, 1000, 2000, 3000];
    let mut last_err: Option<std::io::Error> = None;
    for (attempt, delay_ms) in std::iter::once(0)
        .chain(BACKOFFS_MS.iter().copied())
        .enumerate()
    {
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        match std::fs::remove_dir_all(dir) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                log::warn!(
                    "remove_dir_all 第 {} 次失败: path={} kind={:?} err={e}",
                    attempt + 1,
                    dir.display(),
                    e.kind()
                );
                last_err = Some(e);
            }
        }
    }
    let e = last_err.expect("循环里必须至少出错一次才会到这里");
    Err(Error::Other(format!(
        "remove_dir_all 失败（重试 {} 次仍被拒）: path={} err={e}（kind={:?}）",
        BACKOFFS_MS.len() + 1,
        dir.display(),
        e.kind()
    )))
}

/// 删除当前平台的安装目录（NSIS 卸载向导可调；UI 卸载按钮也走这里）。
pub async fn delete() -> Result<()> {
    let p = platform::detect();
    let dir = platform_dir(p)?;
    if dir.exists() {
        remove_dir_all_retry(&dir).await?;
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────
//  内部辅助
// ─────────────────────────────────────────────────────────

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
        return Err(Error::Other(format!(
            "下载失败：HTTP {} {}",
            resp.status(),
            url
        )));
    }
    let total = resp.content_length();

    let mut file = tokio::fs::File::create(dest).await.map_err(|e| {
        Error::Other(format!(
            "File::create 失败: path={} err={e}（kind={:?}）",
            dest.display(),
            e.kind()
        ))
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
    // llama.cpp 的 macOS / Linux tar.gz 内部包了一层 `llama-<tag>/` 目录，
    // 但 binary_relative_path 假设解压后是扁平布局（直接 dest/llama-server）。
    // 先解到临时子目录，再判断是否需要剥掉单层 wrapper，把真正的内容平铺到 dest。
    let temp = dest.join("__tar_extract_tmp");
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).map_err(Error::from)?;

    let file = std::fs::File::open(archive).map_err(Error::from)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    // tar::Archive::unpack 自带防 .. 逃逸（默认 follow_symlinks(false)）
    tar.unpack(&temp).map_err(Error::from)?;

    // 收集 temp 顶层条目；只有一项且是目录 → 剥掉，把它的内容移到 dest
    let mut top: Vec<std::fs::DirEntry> = std::fs::read_dir(&temp)
        .map_err(Error::from)?
        .filter_map(|e| e.ok())
        .collect();
    let strip_root = top.len() == 1
        && top[0]
            .file_type()
            .map(|t| t.is_dir())
            .unwrap_or(false);
    let move_from = if strip_root {
        let inner = top.remove(0).path();
        std::fs::read_dir(&inner)
            .map_err(Error::from)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect::<Vec<_>>()
    } else {
        top.into_iter().map(|e| e.path()).collect()
    };
    for src in move_from {
        let name = match src.file_name() {
            Some(n) => n.to_owned(),
            None => continue,
        };
        let dst = dest.join(&name);
        // 用户已经装过一次 → 同名文件存在，先删后 rename。rename 跨目录在同卷上是 O(1)。
        if dst.exists() {
            if dst.is_dir() {
                let _ = std::fs::remove_dir_all(&dst);
            } else {
                let _ = std::fs::remove_file(&dst);
            }
        }
        std::fs::rename(&src, &dst).map_err(Error::from)?;
    }
    let _ = std::fs::remove_dir_all(&temp);
    Ok(())
}
