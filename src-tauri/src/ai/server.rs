//! `llama-server` 子进程管理 (Phase 1B-α)。
//!
//! [`EngineSupervisor`] 在 `Arc<EngineSupervisor>` 管理之下作为 app 单例：
//! - lazy spawn：只有用户点"启动引擎"或第一次跑 AI 总结时才拉起
//! - app 退出钩子里 [`stop()`](EngineSupervisor::stop) 会被调，等子进程收尸再让 Tauri 退
//!
//! 进程状态机：
//! ```text
//!   Stopped → Starting → Running { port }
//!                ↓               ↓
//!              Error    ←───── (kill / stop)
//! ```
//!
//! Phase 1B-α 阶段没有模型，[`start`](EngineSupervisor::start) 接 `None` 调用时
//! llama-server 会 fail-fast（缺 `-m` 参数），supervisor 把 stderr 包成可读错误
//! 给前端展示。Phase 1B-β 加模型选择后，会真传入 model / mmproj 路径。

use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::ai::binary;
use crate::ai::platform::{self, Platform};
use crate::error::{Error, Result};

/// 启动后等 `/health` 返回 200 的最大时长。
/// vision LLM 加载耗时随模型大小，9B 量级模型 CPU 上加载 30-60s 是常态。
const HEALTH_TIMEOUT: Duration = Duration::from_secs(90);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// llama-server 上下文窗口大小。
///
/// Vision 模型每张图被分成多个 patch，token 数比纯文本对话大很多：
/// 768 px max_dim 下一张图 ~256 tokens × 12 张 = ~3000 tokens，加 system / user
/// prompt 和输出空间 (max_tokens=768)，4K 容易超。8K 给足余量；GPU 上 KV cache
/// 多用一倍 RAM 不痛——5090/Apple Silicon 都装得下，CPU fallback 也可接受。
const DEFAULT_CTX_SIZE: u32 = 8192;

/// 给前端展示的运行时状态。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineRuntimeStatus {
    /// "stopped" | "starting" | "running" | "error"
    pub state: &'static str,
    /// `Running` 时的监听端口；其它状态 `None`
    pub port: Option<u16>,
    /// `Error` 时的可读错误（stderr 截短）；其它状态 `None`
    pub error: Option<String>,
}

impl EngineRuntimeStatus {
    fn stopped() -> Self {
        Self {
            state: "stopped",
            ..Default::default()
        }
    }
}

/// llama-server 子进程的单例守护者。
pub struct EngineSupervisor {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    state: EngineRuntimeStatus,
    child: Option<Child>,
}

impl Default for EngineSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineSupervisor {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                state: EngineRuntimeStatus::stopped(),
                child: None,
            }),
        }
    }

    /// 当前状态快照（前端用）。
    pub async fn status(&self) -> EngineRuntimeStatus {
        self.inner.lock().await.state.clone()
    }

    /// 启动 llama-server。
    ///
    /// `model_path` / `mmproj_path` 可为 None——α.4 还没模型选择，留待 β。传 None
    /// 时 llama-server 会因为缺 `-m` 参数自己退出，supervisor 拿 stderr 包错误。
    ///
    /// 已经在 Running 状态时返回当前端口，不重复 spawn；已经在 Starting 状态时
    /// 返回错误（避免并发抢同一进程）。
    pub async fn start(
        &self,
        model_path: Option<PathBuf>,
        mmproj_path: Option<PathBuf>,
    ) -> Result<u16> {
        // 第一段：占锁、预检、置 starting、spawn、释放锁
        let port = {
            let mut inner = self.inner.lock().await;
            match inner.state.state {
                "running" => {
                    if let Some(p) = inner.state.port {
                        return Ok(p);
                    }
                }
                "starting" => {
                    return Err(Error::Other("引擎已经在启动中".to_string()));
                }
                _ => {}
            }

            let bin_path = binary::binary_path()?;
            if !bin_path.exists() {
                let msg = "AI 引擎 binary 未安装，先去下载".to_string();
                inner.state = EngineRuntimeStatus {
                    state: "error",
                    port: None,
                    error: Some(msg.clone()),
                };
                return Err(Error::Other(msg));
            }

            let port = pick_free_port()?;
            let mut cmd = build_command(&bin_path, port, model_path, mmproj_path);

            let child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    let msg = format!("spawn 失败：{e}");
                    inner.state = EngineRuntimeStatus {
                        state: "error",
                        port: None,
                        error: Some(msg.clone()),
                    };
                    return Err(Error::Io(e));
                }
            };

            inner.state = EngineRuntimeStatus {
                state: "starting",
                port: Some(port),
                error: None,
            };
            inner.child = Some(child);
            port
        };

        // 第二段：不持锁等 health（持锁等会卡住其它 status 查询）
        let healthy = poll_health(port, HEALTH_TIMEOUT).await;

        // 第三段：根据 health 结果定状态
        let mut inner = self.inner.lock().await;
        if healthy {
            inner.state = EngineRuntimeStatus {
                state: "running",
                port: Some(port),
                error: None,
            };
            Ok(port)
        } else {
            // /health 没等到 200——可能子进程已退出（缺模型 / 配置错），
            // 也可能仍活着但 hang 住。两种情况都拿 stderr 包错误并清掉 child。
            let err_msg = if let Some(child) = inner.child.as_mut() {
                resolve_failure(child).await
            } else {
                "child handle 丢失".to_string()
            };
            inner.state = EngineRuntimeStatus {
                state: "error",
                port: None,
                error: Some(err_msg.clone()),
            };
            inner.child = None;
            Err(Error::Other(err_msg))
        }
    }

    /// 停止子进程。kill + wait 收尸，状态切回 Stopped。
    /// 已经 Stopped 时是 no-op。
    pub async fn stop(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        if let Some(mut child) = inner.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        inner.state = EngineRuntimeStatus::stopped();
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────
//  内部辅助
// ─────────────────────────────────────────────────────────

/// 让 OS 挑一个未占用的端口然后立刻释放——子进程 spawn 用这个端口。
///
/// drop 与 spawn 之间有 race window（~ms 级），其它进程可能抢到这个端口，
/// 但概率极低。如果真撞上，llama-server 启动会 bind 失败，被我们的错误捕获处理。
fn pick_free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(Error::Io)?;
    let port = listener.local_addr().map_err(Error::Io)?.port();
    drop(listener);
    Ok(port)
}

/// 根据 platform 决定 `-ngl`（GPU layer 数）：CUDA / Metal binary 全 offload，
/// CPU binary 不传。
fn gpu_layers_for(p: Platform) -> Option<u32> {
    match p {
        Platform::WindowsX64Cuda12 | Platform::WindowsX64Cuda13 => Some(99),
        Platform::MacOSArm64Metal => Some(99),
        // CPU binary 不支持 -ngl
        Platform::WindowsX64Cpu | Platform::MacOSX64 | Platform::LinuxX64Cpu => None,
    }
}

fn build_command(
    bin: &std::path::Path,
    port: u16,
    model_path: Option<PathBuf>,
    mmproj_path: Option<PathBuf>,
) -> Command {
    let mut cmd = Command::new(bin);
    cmd.arg("--host").arg("127.0.0.1");
    cmd.arg("--port").arg(port.to_string());
    cmd.arg("--ctx-size").arg(DEFAULT_CTX_SIZE.to_string());
    if let Some(m) = model_path {
        cmd.arg("-m").arg(m);
    }
    if let Some(p) = mmproj_path {
        cmd.arg("--mmproj").arg(p);
    }
    if let Some(n) = gpu_layers_for(platform::detect()) {
        cmd.arg("-ngl").arg(n.to_string());
    }
    // 截 stderr/stdout 用于失败时回放给用户；不让它们污染 Hindsight 自己的终端
    cmd.stderr(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());

    // Windows 下抑制控制台窗口闪一下
    // tokio::process::Command 自带 creation_flags 方法（inherent on Windows），
    // 不用 import std::os::windows::process::CommandExt
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    cmd
}

async fn poll_health(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let url = format!("http://127.0.0.1:{port}/health");
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    while Instant::now() < deadline {
        if let Ok(r) = client.get(&url).send().await {
            if r.status().is_success() {
                return true;
            }
        }
        tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
    }
    false
}

/// 启动失败时调，给前端可读的错误描述。
///
/// - 子进程已退出：截 stderr 末尾 ~300 字符返回（一般包含具体错因）
/// - 子进程仍活着但 /health 不响应：先 kill 再返回 timeout 描述
async fn resolve_failure(child: &mut Child) -> String {
    match child.try_wait() {
        Ok(Some(_status)) => drain_stderr(child).await,
        Ok(None) => {
            // 还活着但 hang——强制 kill
            let _ = child.kill().await;
            let _ = child.wait().await;
            // 退出后 stderr 可能有信息
            let stderr_tail = drain_stderr(child).await;
            if stderr_tail.trim().is_empty() {
                format!("启动 {}s 仍未响应 /health，已强制 kill", HEALTH_TIMEOUT.as_secs())
            } else {
                format!(
                    "启动 {}s 未响应 /health，强制 kill；stderr 末尾：{}",
                    HEALTH_TIMEOUT.as_secs(),
                    stderr_tail
                )
            }
        }
        Err(e) => format!("无法读取子进程状态：{e}"),
    }
}

async fn drain_stderr(child: &mut Child) -> String {
    use tokio::io::AsyncReadExt;
    let Some(mut stderr) = child.stderr.take() else {
        return String::new();
    };
    let mut buf = Vec::with_capacity(4096);
    let _ = stderr.read_to_end(&mut buf).await;
    let s = String::from_utf8_lossy(&buf);
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // 只取末尾 300 字符——失败信息通常在末尾，开头是大量启动 log
    let start = trimmed.len().saturating_sub(300);
    trimmed[start..].to_string()
}
