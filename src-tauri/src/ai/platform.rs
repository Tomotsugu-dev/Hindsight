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
//! Windows backend 路由（按 [`BackendChoice`] 偏好决定，缺省 `Auto` 走自动检测）：
//! - **Auto**：CUDA → Vulkan → CPU 三档优先级，挑系统能跑的最强
//! - **Cuda**：强制 CUDA；系统没装 N 卡驱动则退回 CPU
//! - **Vulkan**：强制 Vulkan（A 卡 / Intel / 老 N 卡通用 fallback）
//! - **Cpu**：强制 CPU
//!
//! CUDA 由用户驱动决定（不是我们装 CUDA，是检测他装的）：
//! - `nvidia-smi` 默认输出包含 `CUDA Version: X.Y`，那是当前驱动支持的最高 CUDA
//! - >= 13.1 → 选 `cuda-13.1` binary
//! - >= 12.4 但 < 13.1 → 选 `cuda-12.4` binary
//! - 没装驱动或太老 → 没 CUDA
//!
//! Vulkan 由用户显卡驱动决定：检测注册表 `HKLM\SOFTWARE\Khronos\Vulkan\Drivers`
//! 是否有 ICD 注册——A/N/I 卡装了驱动都会有；命中 = Vulkan 能跑。
//!
//! macOS：Apple Silicon 内建 Metal，Intel Mac 用 x64 binary（也支持 Metal，
//! Metal 是 OS framework）；不需要 GPU 检测，[`BackendChoice`] 偏好被忽略。
//!
//! Linux：v1 不主动支持 Linux——只留 CPU 占位变体让 enum 完整。
//! Linux 上 llama.cpp 官方未发 CUDA binary，要 GPU 加速用户得自编。
//! 等 v2 真要支持 Linux 时再决定走 Vulkan / 自编 CUDA / 别的。

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use serde::Serialize;

/// Hindsight 当前 PIN 的 llama.cpp 版本。
///
/// 升级流程：换这里的常量 + 同步 [`sha256`] 表 + 跑端到端冒烟测试。
/// 不要在前端 / 用户 settings 里暴露这个值——它是 Hindsight 自身发布物的一部分，
/// 不是用户偏好。
pub const PINNED_TAG: &str = "b9025";

/// 当前机器对应哪个 llama-server binary 变体。
///
/// 注意 CUDA / Vulkan 都由用户系统提供，Hindsight 不安装、不打包对应 runtime。
/// 我们做的事仅限于"检测用户驱动 → 挑匹配的 llama.cpp binary"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    WindowsX64Cpu,
    /// driver 支持的最高 CUDA ∈ [12.4, 13.1)
    WindowsX64Cuda12,
    /// driver 支持的最高 CUDA >= 13.1
    WindowsX64Cuda13,
    /// Vulkan backend——A 卡 / Intel / 没装 CUDA 的 N 卡通用 fallback
    WindowsX64Vulkan,
    /// Apple Silicon——内建 Metal
    MacOSArm64Metal,
    /// Intel Mac——binary 自带 Metal 支持（如硬件允许），无需检测
    MacOSX64,
    /// 占位：v1 不主动支持 Linux；只为 enum 完整保留这个变体
    LinuxX64Cpu,
}

/// 用户在 AI 设置 → 引擎页选择的 backend 偏好。
///
/// `Auto` = 让 Hindsight 自动按"CUDA → Vulkan → CPU"挑最强可用；其它三档强制使用对应 backend。
/// 偏好选了 `Cuda` 但系统没装 N 卡驱动，会安全回退到 CPU（不会让下载 / 启动死循环失败）。
/// Vulkan 偏好不做回退——前端 UI 会把"系统不支持 Vulkan"的选项灰掉，理论上用户碰不到。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum BackendChoice {
    Auto = 0,
    Cuda = 1,
    Vulkan = 2,
    Cpu = 3,
}

impl BackendChoice {
    /// 从 settings 里的 `"auto" / "cuda" / "vulkan" / "cpu"` 字符串映射。
    /// 任何未知值统一回退到 `Auto`——前端按下拉枚举只会送回这四个之一。
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "cuda" => Self::Cuda,
            "vulkan" => Self::Vulkan,
            "cpu" => Self::Cpu,
            _ => Self::Auto,
        }
    }

    /// 反过来给 settings sanitize / 前端展示用的小写字符串。
    /// `commands::ai_engine::set_backend_choice` 走 `from_str(...).as_str()` 钳到合法枚举值再落库，
    /// 避免前端送了 "garbage" 之类非法字符串直接进 DB。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cuda => "cuda",
            Self::Vulkan => "vulkan",
            Self::Cpu => "cpu",
        }
    }
}

/// 用户偏好的全局缓存——bootstrap 启动期 + `commands::ai_engine::set_backend_choice`
/// 触发更新。`detect()` 每次都直接 load 这个 atomic，让用户切了下拉框立刻生效（不像
/// `auto_detect_cached()` 那样在 OnceLock 里固化）。
///
/// 默认 0 = Auto；用 AtomicU8 + repr(u8) enum 走 store/load 是 lock-free 的最简方式。
static USER_PREFERENCE: AtomicU8 = AtomicU8::new(0);

/// bootstrap / commands 调用——把用户在 settings 里选的偏好同步到全局原子状态。
/// 调完之后，下一次任何代码读 [`detect()`] 都会反映新偏好。
pub fn set_user_preference(choice: BackendChoice) {
    USER_PREFERENCE.store(choice as u8, Ordering::Relaxed);
}

/// 反查当前用户偏好（前端 EngineTab 拿来回显下拉框选中态）。
pub fn user_preference() -> BackendChoice {
    match USER_PREFERENCE.load(Ordering::Relaxed) {
        1 => BackendChoice::Cuda,
        2 => BackendChoice::Vulkan,
        3 => BackendChoice::Cpu,
        _ => BackendChoice::Auto,
    }
}

/// 系统三档 backend 的可用性快照——给前端 EngineTab 判断下拉框里哪些项要灰掉。
///
/// macOS / Linux 上这三个字段意义不大（那里只有一个 backend 选项），前端不渲染下拉。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendCapabilities {
    /// 检测到 N 卡驱动且 CUDA Version >= 12.4
    pub cuda: bool,
    /// 检测到任意 Vulkan ICD 注册（A/N/I 卡装了驱动都会有）
    pub vulkan: bool,
    /// CPU 永远可用；保留字段让前端结构对称，不用做特判
    pub cpu: bool,
}

/// 检测当前主机三档 backend 各自能不能跑。
///
/// 整体走的是已经被 [`detect_cuda_version`] / [`detect_vulkan_available`] 各自 OnceLock
/// 缓存的子探测函数，重复调几乎零开销；macOS / Linux 直接返回全 false（CUDA + Vulkan
/// 在 v1 路由里只对 Windows 生效）。
pub fn detect_backend_capabilities() -> BackendCapabilities {
    if std::env::consts::OS != "windows" {
        return BackendCapabilities {
            cuda: false,
            vulkan: false,
            cpu: true,
        };
    }
    let cuda = matches!(detect_cuda_version_cached(), Some(v) if v >= (12, 4));
    BackendCapabilities {
        cuda,
        vulkan: detect_vulkan_available_cached(),
        cpu: true,
    }
}

/// 决定当前该用哪个 binary 变体——先看用户偏好，再走自动检测。
///
/// 行为：
/// - macOS / Linux：偏好被忽略（那里只有一个 backend 可选），直接按 OS / arch 路由
/// - Windows + `BackendChoice::Auto`：CUDA → Vulkan → CPU 三档优先级
/// - Windows + `Cuda`：跟 Vulkan 路径对称——直接选 CUDA binary（按检测到的版本挑 12 / 13；
///   探不到时假定较新的 Cuda13 让用户得到明确的启动失败信号）。**不静默 fallback CPU**：
///   用户在 UI 上明确选了 CUDA（哪怕带 danger 警告也点了"继续"），就应该真去下 CUDA binary；
///   硬件不支持时让 llama-server 启动报错给前端展示，比"为什么显存没占用"友好
/// - Windows + `Vulkan`：直接选 [`Platform::WindowsX64Vulkan`]（同上，不 fallback）
/// - Windows + `Cpu`：直接选 [`Platform::WindowsX64Cpu`]
///
/// 不缓存最终结果：偏好可以热切，每次都读一次 atomic + 走子探测的 OnceLock 缓存。
pub fn detect() -> Platform {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    match (os, arch) {
        ("windows", "x86_64") => detect_windows_x64(user_preference()),
        ("macos", "aarch64") => Platform::MacOSArm64Metal,
        ("macos", _) => Platform::MacOSX64,
        ("linux", _) => Platform::LinuxX64Cpu,
        // 兜底：未支持组合（Windows ARM / 32-bit / 其它 OS）退化到最近 CPU 变体
        ("windows", _) => Platform::WindowsX64Cpu,
        _ => Platform::LinuxX64Cpu,
    }
}

fn detect_windows_x64(prefer: BackendChoice) -> Platform {
    match prefer {
        BackendChoice::Cuda => pick_cuda_forced(),
        BackendChoice::Vulkan => Platform::WindowsX64Vulkan,
        BackendChoice::Cpu => Platform::WindowsX64Cpu,
        BackendChoice::Auto => {
            // Auto 模式：先尝试 CUDA，再 Vulkan，最后 CPU 兜底
            if let Some(p) = pick_cuda_auto() {
                return p;
            }
            if detect_vulkan_available_cached() {
                return Platform::WindowsX64Vulkan;
            }
            Platform::WindowsX64Cpu
        }
    }
}

/// `BackendChoice::Cuda` 偏好路径：用户硬选了 CUDA，按检测到的版本挑 binary；
/// 探不到 / 版本太老时仍下 Cuda13 binary，让 llama-server 启动报错给用户清晰信号——
/// **不静默 fallback 到 CPU**，跟 Vulkan 偏好路径对称（前端 UI 已经 ConfirmDialog
/// 用 danger 警告过"硬件未检测到，可能启动失败"了）。
fn pick_cuda_forced() -> Platform {
    match detect_cuda_version_cached() {
        Some(v) if v >= (13, 1) => Platform::WindowsX64Cuda13,
        Some(v) if v >= (12, 4) => Platform::WindowsX64Cuda12,
        _ => Platform::WindowsX64Cuda13,
    }
}

/// `BackendChoice::Auto` 路径下的 CUDA 优先尝试。
/// 只有明确检测到 ≥ 12.4 的 CUDA 才返回 Some；否则交给上层走 Vulkan → CPU。
fn pick_cuda_auto() -> Option<Platform> {
    match detect_cuda_version_cached() {
        Some(v) if v >= (13, 1) => Some(Platform::WindowsX64Cuda13),
        Some(v) if v >= (12, 4) => Some(Platform::WindowsX64Cuda12),
        _ => None,
    }
}

/// CUDA 版本探测的 OnceLock 缓存——首次调跑 `nvidia-smi`（~100ms），之后命中缓存零开销。
/// 偏好 Cuda / Auto 都共享这一份缓存。
fn detect_cuda_version_cached() -> Option<(u32, u32)> {
    static CACHE: OnceLock<Option<(u32, u32)>> = OnceLock::new();
    *CACHE.get_or_init(detect_cuda_version)
}

/// Vulkan 可用性探测的 OnceLock 缓存——首次调跑 `reg query`（~50ms），之后命中缓存。
fn detect_vulkan_available_cached() -> bool {
    static CACHE: OnceLock<bool> = OnceLock::new();
    *CACHE.get_or_init(detect_vulkan_available)
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

/// 探测系统是否注册了任何 Vulkan ICD（Installable Client Driver）。
///
/// Windows 上 A/N/I 卡的图形驱动安装时都会在 `HKLM\SOFTWARE\Khronos\Vulkan\Drivers`
/// 注册一个指向 ICD JSON 文件的值。这条键存在并有子值 = 用户机器装了至少一个能跑
/// Vulkan 的显卡驱动。
///
/// 用子进程 `reg query` 而非引入 winreg crate——跟现有 [`detect_cuda_version`] 走
/// `nvidia-smi` 子进程是一致的风格，且不增加依赖。子进程开销约 50ms，调用方应通过
/// [`detect_vulkan_available_cached`] 拿。
///
/// `reg query` 输出无值时 status 仍 success，但 stdout 里只有空行 + END。所以判断
/// 标准是：status.success() **且** stdout 中含至少一个 REG_DWORD 值（值名是 ICD JSON 路径，
/// 数据 = 0 表示 enabled）；REG_SZ 罕见但有些驱动也会用，一并 contains 检查。
#[cfg(target_os = "windows")]
fn detect_vulkan_available() -> bool {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    let output = Command::new("reg")
        .args(["query", r"HKLM\SOFTWARE\Khronos\Vulkan\Drivers"])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return false,
    };
    // REG_SZ 是 ICD JSON 路径的值类型；含这串 = 至少注册了一个驱动
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.contains("REG_SZ") || stdout.contains("REG_DWORD")
}

#[cfg(not(target_os = "windows"))]
fn detect_vulkan_available() -> bool {
    // v1 只在 Windows 上路由 Vulkan binary（macOS 走 Metal，Linux 不主动支持）
    false
}

/// 从任意文本里解出 `CUDA Version: X.Y`。
///
/// 拆出来当独立函数方便单测——`detect_cuda_version` 拿 nvidia-smi 跑出来的
/// 字符串塞进来。
///
/// `#[allow(dead_code)]`：macOS 的 `detect_cuda_version` 直接返回 `None`，
/// lib 编译里没人调；只有 `#[cfg(test)]` 里的单测会用。Linux / Windows 上
/// `detect_cuda_version` 真的会调它，allow 在那两个平台无影响。
#[allow(dead_code)]
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
        Platform::WindowsX64Vulkan => format!("llama-{tag}-bin-win-vulkan-x64.zip"),
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
        | Platform::WindowsX64Cuda13
        | Platform::WindowsX64Vulkan => "llama-server.exe",
        Platform::MacOSArm64Metal | Platform::MacOSX64 | Platform::LinuxX64Cpu => "llama-server",
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
        // Vulkan：单包 ~32MB，不像 CUDA 还要 cudart runtime
        Platform::WindowsX64Vulkan => 32 * MB,
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
    // 当前阶段所有平台返回 None = 跳过 sha256 校验。
    // 待办（owner: 引擎子系统）：PIN tag 稳定后跑一次实际下载抓 sha256 填进来；
    // 此前每次升级 [`PINNED_TAG`] 都要同步更新这里，否则用户拿到的 binary 完全无完整性保证。
    let _ = (p, tag);
    None
}

// ───────────── 系统 VRAM 探测 ─────────────

/// 系统总显存信息。前端用来跟 `estimateVramGB` 对比给用户 OOM 风险红绿灯。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VramInfo {
    /// 系统总量（GB）——按字面值，不打折。
    /// - discrete: nvidia-smi 报的 memory.total
    /// - unified: 整台机器的物理 RAM
    ///
    /// 前端展示这个原始数字（用户看到的就是机器实际配置）；做 OOM 判断 / 推荐参数时
    /// 由前端 helper `effectiveVramGB(vi)` 按 source 折算（unified 留 30% 给系统 + 其它进程）。
    pub total_gb: f64,
    /// "discrete" = NVIDIA 独立显存；"unified" = Apple Silicon 统一内存。
    pub source: &'static str,
}

/// 全局缓存：第一次调跑命令（约 100-500ms 首次开销），之后直接返。
/// 用 `OnceLock<Option<VramInfo>>`——CPU-only 机器探测返 `None`
/// 时也会被缓存进 OnceLock 的初始化值，避免反复尝试 spawn `nvidia-smi`。
static VRAM_CACHE: OnceLock<Option<VramInfo>> = OnceLock::new();

/// 拉系统 VRAM 信息。CPU-only 机器或探测失败 → 返 `None`，前端按"未检测到独立显存"处理。
///
/// 缓存哲学：换显卡需重启 app 才能拿新值——可接受 trade-off，避免每次轮询都 spawn `nvidia-smi`。
pub fn detect_total_vram_gb() -> Option<VramInfo> {
    VRAM_CACHE.get_or_init(detect_vram_uncached).clone()
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn detect_vram_uncached() -> Option<VramInfo> {
    use std::process::Command;
    let mut cmd = Command::new("nvidia-smi");
    cmd.args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"]);

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
    // 输出形如 "24576\n"——取第一个非空行 trim 解 u64（单位 MB）
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mb: u64 = stdout.lines().next()?.trim().parse().ok()?;
    Some(VramInfo {
        total_gb: mb as f64 / 1024.0,
        source: "discrete",
    })
}

#[cfg(target_os = "macos")]
fn detect_vram_uncached() -> Option<VramInfo> {
    use std::process::Command;
    let output = Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let bytes: u64 = stdout.trim().parse().ok()?;
    let total_ram_gb = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    // 报字面值——用户看到的就是机器实际配置（24GB 就是 24GB）。
    // OOM 判断 / 推荐参数那边由前端 `effectiveVramGB(vi)` helper 按 source 折算
    // （unified 留 30% 给系统 + 其它进程）。
    Some(VramInfo {
        total_gb: total_ram_gb,
        source: "unified",
    })
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn detect_vram_uncached() -> Option<VramInfo> {
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
        Platform::WindowsX64Vulkan,
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
        assert!(binary_relative_path(Platform::WindowsX64Vulkan).ends_with(".exe"));
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

    #[test]
    fn backend_choice_roundtrip() {
        for c in [
            BackendChoice::Auto,
            BackendChoice::Cuda,
            BackendChoice::Vulkan,
            BackendChoice::Cpu,
        ] {
            assert_eq!(BackendChoice::from_str(c.as_str()), c);
        }
        // 未知字符串回退到 Auto
        assert_eq!(BackendChoice::from_str(""), BackendChoice::Auto);
        assert_eq!(BackendChoice::from_str("opencl"), BackendChoice::Auto);
        // 大小写不敏感
        assert_eq!(BackendChoice::from_str("CUDA"), BackendChoice::Cuda);
        assert_eq!(BackendChoice::from_str("Vulkan"), BackendChoice::Vulkan);
    }

    #[test]
    fn vulkan_asset_naming() {
        let name = release_asset_name(Platform::WindowsX64Vulkan, "b9025");
        assert_eq!(name, "llama-b9025-bin-win-vulkan-x64.zip");
    }

    #[test]
    fn vulkan_no_cuda_runtime() {
        // Vulkan 单包，跟 CPU / Metal 一样不需要额外 runtime zip
        assert_eq!(cuda_runtime_asset_name(Platform::WindowsX64Vulkan), None);
    }
}
