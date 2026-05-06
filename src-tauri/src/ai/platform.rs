//! 当前主机 → llama.cpp release asset 路由。
//!
//! llama.cpp 滚动发布，tag 形如 `b9025`，asset 名形如：
//!   - `llama-b9025-bin-win-cpu-x64.zip`
//!   - `llama-b9025-bin-win-cuda-12.4-x64.zip`
//!   - `llama-b9025-bin-win-cuda-13.1-x64.zip`
//!   - `llama-b9025-bin-macos-arm64.tar.gz`
//!   - `llama-b9025-bin-macos-x64.tar.gz`
//!   - `llama-b9025-bin-ubuntu-x64.tar.gz`
//!
//! Windows CUDA 变体由用户驱动决定（不是我们装 CUDA，是检测他装的）：
//! - `nvidia-smi` 默认输出包含 `CUDA Version: X.Y`，那是当前驱动支持的最高 CUDA
//! - >= 13.1 → 选 `cuda-13.1` binary
//! - >= 12.4 但 < 13.1 → 选 `cuda-12.4` binary
//! - 没装驱动或太老 → CPU
//!
//! macOS：Apple Silicon 内建 Metal，Intel Mac 用 x64 binary（也支持 Metal，
//! Metal 是 OS framework）；不需要 GPU 检测。
//!
//! Linux：v1 不主动支持 Linux——只留 CPU 占位变体让 enum 完整。
//! Linux 上 llama.cpp 官方未发 CUDA binary，要 GPU 加速用户得自编。
//! 等 v2 真要支持 Linux 时再决定走 Vulkan / 自编 CUDA / 别的。

use std::sync::OnceLock;

/// Hindsight 当前 PIN 的 llama.cpp 版本。
///
/// 升级流程：换这里的常量 + 同步 [`sha256`] 表 + 跑端到端冒烟测试。
/// 不要在前端 / 用户 settings 里暴露这个值——它是 Hindsight 自身发布物的一部分，
/// 不是用户偏好。
pub const PINNED_TAG: &str = "b9025";

/// 当前机器对应哪个 llama-server binary 变体。
///
/// 注意 CUDA 由用户系统提供，Hindsight 不安装、不打包 CUDA runtime。
/// 我们做的事仅限于"检测用户驱动 → 挑匹配的 llama.cpp CUDA binary"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    WindowsX64Cpu,
    /// driver 支持的最高 CUDA ∈ [12.4, 13.1)
    WindowsX64Cuda12,
    /// driver 支持的最高 CUDA >= 13.1
    WindowsX64Cuda13,
    /// Apple Silicon——内建 Metal
    MacOSArm64Metal,
    /// Intel Mac——binary 自带 Metal 支持（如硬件允许），无需检测
    MacOSX64,
    /// 占位：v1 不主动支持 Linux；只为 enum 完整保留这个变体
    LinuxX64Cpu,
}

/// 检测当前主机最合适的变体。
///
/// 结果做缓存——首次调用会跑一次 `nvidia-smi` 探测 CUDA 版本，之后直接读缓存。
/// 用户即便在运行时插拔 GPU / 装驱动也不会重新探测，但这种情况极罕见，重启 app 即可。
pub fn detect() -> Platform {
    static DETECTED: OnceLock<Platform> = OnceLock::new();
    *DETECTED.get_or_init(detect_uncached)
}

fn detect_uncached() -> Platform {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    match (os, arch) {
        ("windows", "x86_64") => detect_windows_x64(),
        ("macos", "aarch64") => Platform::MacOSArm64Metal,
        ("macos", _) => Platform::MacOSX64,
        ("linux", _) => Platform::LinuxX64Cpu,
        // 兜底：未支持组合（Windows ARM / 32-bit / 其它 OS）退化到最近 CPU 变体
        ("windows", _) => Platform::WindowsX64Cpu,
        _ => Platform::LinuxX64Cpu,
    }
}

fn detect_windows_x64() -> Platform {
    match detect_cuda_version() {
        Some(v) if v >= (13, 1) => Platform::WindowsX64Cuda13,
        Some(v) if v >= (12, 4) => Platform::WindowsX64Cuda12,
        // 检测到 CUDA 但版本太老（< 12.4）→ 继续用 CPU；llama.cpp 没发更老的 CUDA binary
        _ => Platform::WindowsX64Cpu,
    }
}

/// 探测 driver 支持的最高 CUDA 版本。
///
/// 解析 `nvidia-smi` 默认输出的 header 行：
/// > NVIDIA-SMI 545.84  Driver Version: 545.84  CUDA Version: 12.3
///
/// 这里的"CUDA Version"是当前驱动**支持的最高** CUDA，不是用户装的 toolkit 版本。
/// 我们要的就是这个数字——决定能跑哪个 llama.cpp CUDA binary。
///
/// 命令不存在 / 没 NVIDIA / 解析失败：返回 None，调用方按 CPU 路径处理。
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn detect_cuda_version() -> Option<(u32, u32)> {
    use std::process::Command;
    let mut cmd = Command::new("nvidia-smi");

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW，避免 detect 时弹一个黑控制台窗
        cmd.creation_flags(0x0800_0000);
    }

    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_cuda_version(&stdout)
}

#[cfg(target_os = "macos")]
fn detect_cuda_version() -> Option<(u32, u32)> {
    // macOS 不存在 NVIDIA + CUDA 路径；不必探测
    None
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn detect_cuda_version() -> Option<(u32, u32)> {
    None
}

/// 从任意文本里解出 `CUDA Version: X.Y`。
///
/// 拆出来当独立函数方便单测——`detect_cuda_version` 拿 nvidia-smi 跑出来的
/// 字符串塞进来。
fn parse_cuda_version(text: &str) -> Option<(u32, u32)> {
    let key = "CUDA Version:";
    let idx = text.find(key)?;
    let rest = text[idx + key.len()..].trim_start();

    let mut chars = rest.chars().peekable();
    let major: String = std::iter::from_fn(|| chars.next_if(|c| c.is_ascii_digit())).collect();
    if major.is_empty() {
        return None;
    }
    if chars.next()? != '.' {
        return None;
    }
    let minor: String = std::iter::from_fn(|| chars.next_if(|c| c.is_ascii_digit())).collect();
    if minor.is_empty() {
        return None;
    }
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// 平台变体对应的 GitHub release asset 文件名。
///
/// 命名约定来自 llama.cpp 实际发布物（已对照 b9025 验证）。升级 tag 时
/// 顺手 spot-check 一下命名是否变了。
pub fn release_asset_name(p: Platform, tag: &str) -> String {
    match p {
        Platform::WindowsX64Cpu => format!("llama-{tag}-bin-win-cpu-x64.zip"),
        Platform::WindowsX64Cuda12 => format!("llama-{tag}-bin-win-cuda-12.4-x64.zip"),
        Platform::WindowsX64Cuda13 => format!("llama-{tag}-bin-win-cuda-13.1-x64.zip"),
        Platform::MacOSArm64Metal => format!("llama-{tag}-bin-macos-arm64.tar.gz"),
        Platform::MacOSX64 => format!("llama-{tag}-bin-macos-x64.tar.gz"),
        Platform::LinuxX64Cpu => format!("llama-{tag}-bin-ubuntu-x64.tar.gz"),
    }
}

/// CUDA 平台额外需要的 NVIDIA runtime zip 名（cudart / cublas / cublasLt 等）。
///
/// llama.cpp 把 CUDA runtime DLL 单独打到独立 zip，**不在主 binary zip 里**。
/// 缺这些 DLL → `ggml-cuda.dll` 加载会静默失败 → 模型退回 CPU 跑。
///
/// 文件名里**不含 tag**（NVIDIA 提供的 runtime 跟 llama.cpp 版本无关），
/// 但 GitHub release URL 仍然按 tag 路由，所以 URL 拼接时还要 tag。
///
/// 非 CUDA 平台返回 `None`：CPU / Metal / Linux 都没有这个需求。
pub fn cuda_runtime_asset_name(p: Platform) -> Option<&'static str> {
    match p {
        Platform::WindowsX64Cuda12 => Some("cudart-llama-bin-win-cuda-12.4-x64.zip"),
        Platform::WindowsX64Cuda13 => Some("cudart-llama-bin-win-cuda-13.1-x64.zip"),
        _ => None,
    }
}

/// 解压后 `llama-server` 可执行文件的相对路径。
///
/// llama.cpp release 包内布局是**扁平的**——所有可执行文件 + 动态库
/// 都在同一目录下，没有 `build/bin/` 之类的嵌套（已对照 b9025 实测）。
/// 所以 path = 直接文件名。可执行文件依赖同目录的 `.dll` / `.so` / `.dylib`。
pub fn binary_relative_path(p: Platform) -> &'static str {
    match p {
        Platform::WindowsX64Cpu
        | Platform::WindowsX64Cuda12
        | Platform::WindowsX64Cuda13 => "llama-server.exe",
        Platform::MacOSArm64Metal | Platform::MacOSX64 | Platform::LinuxX64Cpu => {
            "llama-server"
        }
    }
}

/// 估算下载体积（字节）。前端拿来给用户提示"约 NN MB"。
///
/// 数字按 b9025 实测取整，每次升级 PIN tag 时如果包大小有较大变动顺手更新。
/// 不需要精确——UI 只显示"约 150 MB"这种粗粒度，用户看到的是数量级。
pub fn estimated_bytes(p: Platform) -> u64 {
    const MB: u64 = 1024 * 1024;
    match p {
        // CPU 包：~16MB（GGML + 几个动态库）
        Platform::WindowsX64Cpu => 16 * MB,
        // CUDA 12.4：主 binary ~214MB + cudart runtime ~391MB ≈ 605MB
        Platform::WindowsX64Cuda12 => 605 * MB,
        // CUDA 13.1：主 binary ~135MB + cudart runtime ~384MB ≈ 520MB
        Platform::WindowsX64Cuda13 => 520 * MB,
        // macOS / Linux：CPU 体积，~30MB
        Platform::MacOSArm64Metal => 30 * MB,
        Platform::MacOSX64 => 30 * MB,
        Platform::LinuxX64Cpu => 30 * MB,
    }
}

/// 下载校验用 SHA256，按 `(platform, tag)` 查。
///
/// `None` = 该组合的 SHA256 暂未录入；下载层应记 warning 但不 abort（v1 简化策略）。
///
/// 录入步骤：升级 [`PINNED_TAG`] 时跑一次实际下载，
/// `Get-FileHash <file> -Algorithm SHA256`，把所有变体填进来。
pub fn sha256(p: Platform, tag: &str) -> Option<&'static str> {
    // TODO(α.3 之前)：PIN tag 选定后填入实际 sha256。
    let _ = (p, tag);
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const ALL: &[Platform] = &[
        Platform::WindowsX64Cpu,
        Platform::WindowsX64Cuda12,
        Platform::WindowsX64Cuda13,
        Platform::MacOSArm64Metal,
        Platform::MacOSX64,
        Platform::LinuxX64Cpu,
    ];

    #[test]
    fn release_asset_names_distinct() {
        let names: HashSet<_> = ALL
            .iter()
            .map(|&p| release_asset_name(p, PINNED_TAG))
            .collect();
        assert_eq!(names.len(), ALL.len(), "每个变体的 asset 名必须互不相同");
    }

    #[test]
    fn release_asset_names_contain_tag() {
        for &p in ALL {
            let name = release_asset_name(p, "b1234");
            assert!(name.contains("b1234"), "{name} 应该含 tag");
            assert!(name.starts_with("llama-"), "{name} 应该以 llama- 开头");
        }
    }

    #[test]
    fn windows_binaries_have_exe() {
        assert!(binary_relative_path(Platform::WindowsX64Cpu).ends_with(".exe"));
        assert!(binary_relative_path(Platform::WindowsX64Cuda12).ends_with(".exe"));
        assert!(binary_relative_path(Platform::WindowsX64Cuda13).ends_with(".exe"));
    }

    #[test]
    fn unix_binaries_no_extension() {
        assert!(!binary_relative_path(Platform::MacOSArm64Metal).ends_with(".exe"));
        assert!(!binary_relative_path(Platform::MacOSX64).ends_with(".exe"));
        assert!(!binary_relative_path(Platform::LinuxX64Cpu).ends_with(".exe"));
    }

    #[test]
    fn cuda_platforms_have_runtime_asset() {
        assert_eq!(
            cuda_runtime_asset_name(Platform::WindowsX64Cuda12),
            Some("cudart-llama-bin-win-cuda-12.4-x64.zip")
        );
        assert_eq!(
            cuda_runtime_asset_name(Platform::WindowsX64Cuda13),
            Some("cudart-llama-bin-win-cuda-13.1-x64.zip")
        );
    }

    #[test]
    fn non_cuda_platforms_have_no_runtime_asset() {
        for &p in ALL {
            if matches!(p, Platform::WindowsX64Cuda12 | Platform::WindowsX64Cuda13) {
                continue;
            }
            assert_eq!(cuda_runtime_asset_name(p), None);
        }
    }

    #[test]
    fn detect_returns_supported_variant() {
        // 主机平台无法控制；只验证返回值是已知的变体之一
        let p = detect();
        assert!(ALL.contains(&p));
    }

    #[test]
    fn parse_cuda_version_typical() {
        let s = "NVIDIA-SMI 545.84  Driver Version: 545.84  CUDA Version: 12.3 ";
        assert_eq!(parse_cuda_version(s), Some((12, 3)));
    }

    #[test]
    fn parse_cuda_version_two_digits() {
        assert_eq!(parse_cuda_version("CUDA Version: 13.10"), Some((13, 10)));
    }

    #[test]
    fn parse_cuda_version_missing() {
        assert_eq!(parse_cuda_version("nothing relevant"), None);
    }

    #[test]
    fn parse_cuda_version_malformed() {
        // 没有点 → None
        assert_eq!(parse_cuda_version("CUDA Version: 12 "), None);
        // 只有 major 后跟非点 → None
        assert_eq!(parse_cuda_version("CUDA Version: 12_3"), None);
        // 没数字 → None
        assert_eq!(parse_cuda_version("CUDA Version: foo"), None);
    }

    /// 验证版本比较逻辑（detect_windows_x64 用的阈值）符合预期。
    /// 不实际跑 detect_windows_x64（依赖主机），只测算法走向。
    #[test]
    fn version_thresholds() {
        let pick = |v: Option<(u32, u32)>| -> Platform {
            match v {
                Some(v) if v >= (13, 1) => Platform::WindowsX64Cuda13,
                Some(v) if v >= (12, 4) => Platform::WindowsX64Cuda12,
                _ => Platform::WindowsX64Cpu,
            }
        };
        assert_eq!(pick(Some((13, 1))), Platform::WindowsX64Cuda13);
        assert_eq!(pick(Some((14, 0))), Platform::WindowsX64Cuda13);
        assert_eq!(pick(Some((13, 0))), Platform::WindowsX64Cuda12); // 13.0 < 13.1，回退到 12
        assert_eq!(pick(Some((12, 5))), Platform::WindowsX64Cuda12);
        assert_eq!(pick(Some((12, 4))), Platform::WindowsX64Cuda12);
        assert_eq!(pick(Some((12, 3))), Platform::WindowsX64Cpu);
        assert_eq!(pick(Some((11, 8))), Platform::WindowsX64Cpu);
        assert_eq!(pick(None), Platform::WindowsX64Cpu);
    }
}
