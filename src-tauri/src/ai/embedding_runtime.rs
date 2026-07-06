//! ONNX Runtime 动态库的下载、安装、状态管理。
//!
//! OCR（`ai/ocr.rs`）用 ort + load-dynamic 在运行期 dlopen `libonnxruntime.{dylib,dll,so}`，
//! Hindsight 不静态链接、也不打进 release 包（dylib ~30MB 跟 llama.cpp binary 体积接近，
//! 让用户**用到**才下，跟 [`super::binary`] 同款 lazy-fetch 模式）。
//!
//! 安装目录：`<data_root>/ai/runtime/libonnxruntime.{dylib,dll,so}`
//! 跟 `<data_root>/ai/bin/` 同级 ——`ai/` 下"binary（llama.cpp）" + "runtime（ort）"分两个子目录。
//!
//! 下载源分平台：macOS/Linux 用 GitHub 官方 CPU 构建；**Windows 用 NuGet 的
//! DirectML 构建**（onnxruntime.dll + DirectML.dll 双件落同目录），让
//! `ai/mod.rs` 注册的 DML EP 真正生效——OCR 推理跑 GPU（任意 DX12 显卡含核显），
//! 不再烧 CPU。见 [`artifacts`]。
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

/// Windows 用 DirectML 构建（NuGet 发布），这是与 ORT 1.22.0 配套的
/// Microsoft.AI.DirectML 版本（来源：该 nupkg 的 nuspec 依赖声明）。
/// 升级 PINNED_VERSION 时必须重查 nuspec 同步此值。
const DIRECTML_VERSION: &str = "1.15.4";

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

/// dylib 完整路径（不保证存在）。[`init_dylib_path`] 启动时调来设 `ORT_DYLIB_PATH`。
pub fn dylib_path() -> Result<PathBuf> {
    Ok(install_root()?.join(default_dylib_name()))
}

/// 启动时调用：把 lazy-download 落盘的 onnxruntime dylib 塞进 `ORT_DYLIB_PATH`，
/// 让 ort 的 load-dynamic feature 在 dlopen 时定位到正确路径（OCR 引擎依赖）。
///
/// 文件不存在 → 不设环境变量、log info 提示；OCR 首次调用会返回
/// [`Error::EmbeddingRuntimeMissing`]，前端弹框引导下载。
pub fn init_dylib_path() {
    let path = match dylib_path() {
        Ok(p) => p,
        Err(e) => {
            log::warn!("init_dylib_path: 算 dylib 路径失败: {e}");
            return;
        }
    };
    if path.exists() {
        std::env::set_var("ORT_DYLIB_PATH", &path);
        log::info!("ORT_DYLIB_PATH = {}", path.display());
    } else {
        log::info!(
            "onnxruntime 未安装（{}）；首次使用 OCR 时会引导下载",
            path.display()
        );
    }
}

/// dylib 文件是否已落盘。Windows 上还要求 DirectML.dll 同目录就位——
/// 旧版 CPU 构建的安装（无 DirectML.dll）因此自然判定为未安装，
/// 引导用户重下一次（~40MB）即完成到 GPU 构建的迁移。
pub fn is_installed() -> Result<bool> {
    let ok = dylib_path()?.exists()
        && (!cfg!(target_os = "windows") || install_root()?.join("DirectML.dll").exists());
    Ok(ok)
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
    let arts = artifacts(platform::detect())?;

    let dir = install_root()?;
    std::fs::create_dir_all(&dir).map_err(|e| Error::EngineBinary {
        stage: "mkdir",
        details: format!("path={} err={e}", dir.display()),
    })?;

    // ── HEAD 各工件拿总大小（前端进度条用）───────
    let client = reqwest::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .build()?;
    let mut sizes: Vec<Option<u64>> = Vec::with_capacity(arts.len());
    for a in &arts {
        let size = match client.head(&a.url).send().await {
            Ok(resp) => resp.content_length(),
            Err(_) => None,
        };
        sizes.push(size);
    }
    // 任一工件 HEAD 失败就报未知总量（进度条转不确定态），不影响下载
    let grand_total: Option<u64> = sizes
        .iter()
        .copied()
        .collect::<Option<Vec<u64>>>()
        .map(|v| v.iter().sum());
    progress(DownloadPhase::Downloading, 0, grand_total);

    // ── 逐工件:下载 → 提取 → 清临时 ─────────────
    let mut done_bytes: u64 = 0;
    for a in &arts {
        let temp_archive = dir.join(format!("{}.partial", a.dest));

        let base = done_bytes;
        let mut sub = |phase: DownloadPhase, bytes: u64, _total: Option<u64>| {
            if matches!(phase, DownloadPhase::Downloading) {
                progress(DownloadPhase::Downloading, base + bytes, grand_total);
            }
        };
        if let Err(e) = stream_download_with(&client, &a.url, &temp_archive, &mut sub).await {
            let _ = std::fs::remove_file(&temp_archive);
            return Err(e);
        }
        done_bytes += std::fs::metadata(&temp_archive)
            .map(|m| m.len())
            .unwrap_or(0);

        // ── 校验（可选，v1 跟 binary.rs 一致跳过）───
        progress(DownloadPhase::Verifying, done_bytes, grand_total);
        log::info!(
            "onnxruntime 组件 sha256 未录入（version={PINNED_VERSION} dest={}），跳过校验",
            a.dest
        );

        // ── 解压 + 提取 ──────────────────────────
        progress(DownloadPhase::Extracting, done_bytes, grand_total);
        let archive_clone = temp_archive.clone();
        let target = dir.join(a.dest);
        let spec = a.spec;
        let res =
            tokio::task::spawn_blocking(move || extract_one(&archive_clone, spec, &target))
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
    }

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
/// Windows 分支仅为穷举保留——实际下载走 [`artifacts`] 的 NuGet DML 路径。
fn asset_name(p: Platform) -> Result<&'static str> {
    Ok(match p {
        Platform::MacOSArm64Metal => "onnxruntime-osx-arm64-1.22.0.tgz",
        Platform::MacOSX64 => "onnxruntime-osx-x86_64-1.22.0.tgz",
        Platform::WindowsX64Cpu | Platform::WindowsX64Cuda12 | Platform::WindowsX64Cuda13 => {
            "onnxruntime-win-x64-1.22.0.zip"
        }
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

/// 展示用下载体积量级——Windows 双包(ORT-DML ~18MB + DirectML ~18MB)约 40MB,
/// 其它平台单包约 30MB。
fn estimated_bytes() -> u64 {
    if cfg!(target_os = "windows") {
        40 * 1024 * 1024
    } else {
        30 * 1024 * 1024
    }
}

fn release_url(asset: &str) -> String {
    format!("https://github.com/microsoft/onnxruntime/releases/download/v{PINNED_VERSION}/{asset}")
}

/// 一次运行时安装要落盘的单个工件。
struct Artifact {
    url: String,
    /// 归档内怎么找目标文件
    spec: ExtractSpec,
    /// install_root 下的落地文件名
    dest: &'static str,
}

/// 归档内条目匹配规则。
#[derive(Clone, Copy)]
enum ExtractSpec {
    /// onnxruntime 官方 tarball 里的版本化 dylib(模糊匹配,见 [`dylib_entry_matches`])
    OnnxDylib,
    /// NuGet 包(zip)内的精确相对路径(小写比较)
    Exact(&'static str),
}

/// 平台 → 工件清单。
///
/// - macOS / Linux:GitHub 官方 CPU 构建单件(macOS 的 OCR 默认走 Vision,
///   此运行时只服务 Paddle 回退与其它 ONNX 用途);
/// - Windows:NuGet 的 **DirectML 构建** + 配套 DirectML.dll 双件——
///   `ai/mod.rs` 注册的 DML EP 只有在 dll 带 DML 时才真正生效,否则静默回退 CPU。
///   两个 DLL 落同一目录:libloading 以 LOAD_WITH_ALTERED_SEARCH_PATH 打开
///   onnxruntime.dll,其延迟加载的 DirectML.dll 优先在同目录解析。
///   目前只发 x64 安装包,故硬编码 win-x64;将来出 arm64 包时此处按架构分支。
fn artifacts(p: Platform) -> Result<Vec<Artifact>> {
    if cfg!(target_os = "windows") {
        return Ok(vec![
            Artifact {
                url: format!(
                    "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime.directml/{PINNED_VERSION}/microsoft.ml.onnxruntime.directml.{PINNED_VERSION}.nupkg"
                ),
                spec: ExtractSpec::Exact("runtimes/win-x64/native/onnxruntime.dll"),
                dest: "onnxruntime.dll",
            },
            Artifact {
                url: format!(
                    "https://api.nuget.org/v3-flatcontainer/microsoft.ai.directml/{DIRECTML_VERSION}/microsoft.ai.directml.{DIRECTML_VERSION}.nupkg"
                ),
                // 注意精确到 x64-win:包里还有 xbox 变体和 Debug 版
                spec: ExtractSpec::Exact("bin/x64-win/directml.dll"),
                dest: "DirectML.dll",
            },
        ]);
    }
    Ok(vec![Artifact {
        url: release_url(asset_name(p)?),
        spec: ExtractSpec::OnnxDylib,
        dest: default_dylib_name(),
    }])
}

/// 按 spec 从归档提取目标文件。NuGet 包一定是 zip;官方发布在
/// macOS/Linux 上是 tar.gz(Windows 官方 zip 路径已不再使用)。
fn extract_one(archive: &Path, spec: ExtractSpec, target: &Path) -> Result<()> {
    match spec {
        ExtractSpec::Exact(entry) => extract_from_zip(archive, target, &|name: &str| {
            let lower = name.to_lowercase();
            lower == entry || lower.ends_with(&format!("/{entry}"))
        }),
        ExtractSpec::OnnxDylib => extract_from_tar_gz(archive, target, &dylib_entry_matches),
    }
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
fn extract_from_zip(
    archive: &Path,
    target: &Path,
    matches: &dyn Fn(&str) -> bool,
) -> Result<()> {
    if target.exists() {
        let _ = std::fs::remove_file(target);
    }
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
        if matches(&name) {
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

fn extract_from_tar_gz(
    archive: &Path,
    target: &Path,
    matches: &dyn Fn(&str) -> bool,
) -> Result<()> {
    if target.exists() {
        let _ = std::fs::remove_file(target);
    }
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
        if matches(&name) {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// nupkg 提取冒烟:用真实下载的 NuGet 包验证 Exact 匹配路径与提取逻辑。
    /// 跑法:
    /// `ORT_DML_NUPKG=<路径> DIRECTML_NUPKG=<路径> cargo test --lib nupkg_extract -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn nupkg_extract_smoke() {
        let tmp = std::env::temp_dir().join("hindsight-nupkg-extract-test");
        let _ = std::fs::create_dir_all(&tmp);
        let cases = [
            (
                "ORT_DML_NUPKG",
                "runtimes/win-x64/native/onnxruntime.dll",
                "onnxruntime.dll",
            ),
            ("DIRECTML_NUPKG", "bin/x64-win/directml.dll", "DirectML.dll"),
        ];
        for (env, entry, dest) in cases {
            let Some(p) = std::env::var_os(env) else {
                eprintln!("未设 {env},跳过");
                continue;
            };
            let target = tmp.join(dest);
            extract_one(Path::new(&p), ExtractSpec::Exact(entry), &target)
                .expect("提取失败");
            let len = std::fs::metadata(&target).expect("目标不存在").len();
            eprintln!("{dest}: {len} bytes");
            assert!(len > 1_000_000, "{dest} 太小,匹配到了错误条目?");
        }
    }
}
