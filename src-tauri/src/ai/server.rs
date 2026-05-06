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

use std::collections::VecDeque;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
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

/// 引擎进程的离散状态。
///
/// 序列化结果保持 `"stopped"` / `"starting"` / `"running"` / `"error"`，
/// 与之前 `&'static str` 一致——前端 `EngineRuntimeStatus` 字面量联合类型不动。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineState {
    #[default]
    Stopped,
    Starting,
    Running,
    Error,
}

/// 给前端展示的运行时状态。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineRuntimeStatus {
    pub state: EngineState,
    /// `Running` 时的监听端口；其它状态 `None`
    pub port: Option<u16>,
    /// `Error` 时的可读错误（stderr 截短）；其它状态 `None`
    pub error: Option<String>,
}

/// llama-server 子进程的单例守护者。
pub struct EngineSupervisor {
    inner: Mutex<Inner>,
    /// 启动期间（前 STARTUP_LINES 行）保留下来的日志——cuBLAS init / offloaded
    /// XX/YY layers to GPU 这种关键诊断信息在这里，不被后续 chat 日志冲掉。
    startup_logs: Arc<Mutex<Vec<String>>>,
    /// 持续滚动的 ring，最近 LOGS_RING_SIZE 行 stderr / stdout（含启动日志的副本）。
    /// Arc 让 spawn 出来的 reader task 能持有它而不锁 inner。
    logs: Arc<Mutex<VecDeque<String>>>,
}

const LOGS_RING_SIZE: usize = 500;
/// 启动日志保留行数——头 200 行（cuBLAS init、模型加载、layer offload 等都在这里）。
const STARTUP_LINES: usize = 200;

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
            inner: Mutex::new(Inner::default()),
            startup_logs: Arc::new(Mutex::new(Vec::with_capacity(STARTUP_LINES))),
            logs: Arc::new(Mutex::new(VecDeque::with_capacity(LOGS_RING_SIZE))),
        }
    }

    /// 当前状态快照（前端用）。
    pub async fn status(&self) -> EngineRuntimeStatus {
        self.inner.lock().await.state.clone()
    }

    /// 拿日志：启动头 N 行（保留区） + 最近 ring buffer，拼一起。
    /// 前端调试 tab 用来看 llama-server 启动时的 GPU 加载日志。
    pub async fn recent_logs(&self) -> Vec<String> {
        let startup = self.startup_logs.lock().await.clone();
        let ring = self.logs.lock().await;
        let mut out = Vec::with_capacity(startup.len() + ring.len() + 1);
        if !startup.is_empty() {
            out.extend(startup);
            out.push(format!(
                "──── 上面是启动日志（保留前 {} 行），下面是最近滚动日志 ────",
                STARTUP_LINES
            ));
        }
        out.extend(ring.iter().cloned());
        out
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
                EngineState::Running => {
                    if let Some(p) = inner.state.port {
                        return Ok(p);
                    }
                }
                EngineState::Starting => {
                    return Err(Error::Other("引擎已经在启动中".to_string()));
                }
                EngineState::Stopped | EngineState::Error => {}
            }

            let bin_path = binary::binary_path()?;
            if !bin_path.exists() {
                let msg = "AI 引擎 binary 未安装，先去下载".to_string();
                inner.state = EngineRuntimeStatus {
                    state: EngineState::Error,
                    port: None,
                    error: Some(msg.clone()),
                };
                return Err(Error::Other(msg));
            }

            let port = pick_free_port()?;
            let mut cmd = build_command(&bin_path, port, model_path, mmproj_path);

            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    let msg = format!("spawn 失败：{e}");
                    inner.state = EngineRuntimeStatus {
                        state: EngineState::Error,
                        port: None,
                        error: Some(msg.clone()),
                    };
                    return Err(Error::Io(e));
                }
            };

            // 启动前清空两份日志缓冲，让本次启动从空白开始
            self.logs.lock().await.clear();
            self.startup_logs.lock().await.clear();

            // spawn 异步 task 消费 stderr / stdout：写一份到 ring buffer（持续滚动），
            // 同时前 STARTUP_LINES 行额外写一份到 startup_logs（永久保留，不被冲掉）。
            // 不消费的话 buffer 满会阻塞 llama-server。
            if let Some(stderr) = child.stderr.take() {
                spawn_drain_task(
                    stderr,
                    "stderr",
                    Arc::clone(&self.logs),
                    Arc::clone(&self.startup_logs),
                );
            }
            if let Some(stdout) = child.stdout.take() {
                spawn_drain_task(
                    stdout,
                    "stdout",
                    Arc::clone(&self.logs),
                    Arc::clone(&self.startup_logs),
                );
            }

            inner.state = EngineRuntimeStatus {
                state: EngineState::Starting,
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
                state: EngineState::Running,
                port: Some(port),
                error: None,
            };
            Ok(port)
        } else {
            // /health 没等到 200——可能子进程已退出（缺模型 / 配置错），
            // 也可能仍活着但 hang 住。两种情况都从 logs ring 拿末尾几行作错误描述。
            let err_msg = if let Some(child) = inner.child.as_mut() {
                resolve_failure(child, &self.logs).await
            } else {
                "child handle 丢失".to_string()
            };
            inner.state = EngineRuntimeStatus {
                state: EngineState::Error,
                port: None,
                error: Some(err_msg.clone()),
            };
            inner.child = None;
            Err(Error::Other(err_msg))
        }
    }

    /// 停止子进程。kill + wait 收尸，状态切回 Stopped。
    /// 已经 Stopped 时是 no-op。
    ///
    /// 两段式：先持锁 take child + 置 stopped，立刻释放锁；再不持锁 kill/wait。
    /// 子进程慢退出（数秒）时其它 status() 调用不会被 hold，避免与
    /// `lib.rs` 的 `RunEvent::Exit` 钩子里 `block_on(stop())` 配合时死等。
    pub async fn stop(&self) -> Result<()> {
        let child = {
            let mut inner = self.inner.lock().await;
            inner.state = EngineRuntimeStatus::default();
            inner.child.take()
        };
        if let Some(mut child) = child {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
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
/// stderr 已被 [`spawn_drain_task`] 持续消费到 logs ring，这里直接读末尾几行。
async fn resolve_failure(
    child: &mut Child,
    logs: &Arc<Mutex<VecDeque<String>>>,
) -> String {
    let exit_status = child.try_wait();
    let killed = matches!(exit_status, Ok(None));
    if killed {
        // 还活着但 hang——强制 kill
        let _ = child.kill().await;
        let _ = child.wait().await;
        // 等 50ms 让 drain task 把残余 stderr 行写完
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let tail = log_tail(logs, 12).await;
    match exit_status {
        Ok(Some(_)) => {
            if tail.is_empty() {
                "子进程已退出但无日志输出".to_string()
            } else {
                format!("子进程退出；日志末尾：\n{tail}")
            }
        }
        Ok(None) => {
            if tail.is_empty() {
                format!("启动 {}s 未响应 /health，已强制 kill", HEALTH_TIMEOUT.as_secs())
            } else {
                format!(
                    "启动 {}s 未响应 /health，已强制 kill；日志末尾：\n{tail}",
                    HEALTH_TIMEOUT.as_secs()
                )
            }
        }
        Err(e) => format!("无法读取子进程状态：{e}"),
    }
}

/// 读 logs ring 末尾 N 行拼成多行字符串，给错误信息 / 调试 UI 用。
async fn log_tail(logs: &Arc<Mutex<VecDeque<String>>>, n: usize) -> String {
    let g = logs.lock().await;
    let start = g.len().saturating_sub(n);
    g.iter()
        .skip(start)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

/// spawn 一个 tokio task，按行 drain stderr / stdout：
/// - 每行都进 ring buffer（最近 LOGS_RING_SIZE 行，老的被淘汰）
/// - 启动期间（startup_logs 还没满）的同一行也额外写一份进 startup_logs，永久保留
///
/// `tag` = "stderr" / "stdout"，给行前面带个前缀方便区分。
fn spawn_drain_task<R>(
    reader: R,
    tag: &'static str,
    ring: Arc<Mutex<VecDeque<String>>>,
    startup: Arc<Mutex<Vec<String>>>,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    let prefixed = format!("[{tag}] {line}");
                    // 直接 eprintln 走 stderr，不依赖 log 框架——这样 dev console 一定能
                    // 看到 llama-server 的所有输出（cuBLAS init / offloaded layers 等关键
                    // 诊断信息）。要屏蔽就用 grep -v "[stderr]" 之类管道过滤。
                    eprintln!("[llama-server] {prefixed}");
                    // 启动日志保留区（前 STARTUP_LINES 行；满了不再写）
                    {
                        let mut s = startup.lock().await;
                        if s.len() < STARTUP_LINES {
                            s.push(prefixed.clone());
                        }
                    }
                    // ring buffer
                    let mut g = ring.lock().await;
                    if g.len() >= LOGS_RING_SIZE {
                        g.pop_front();
                    }
                    g.push_back(prefixed);
                }
                Ok(None) => break,    // EOF：子进程关闭了管道
                Err(_) => break,      // 读错误：直接退出 task
            }
        }
    });
}
