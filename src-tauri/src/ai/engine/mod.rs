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
//! ## 锁顺序约定
//!
//! 本模块持有三把锁：
//! - `guards.turn`：`tokio::sync::Mutex<()>` —— 来要引擎的排队
//! - `inner`：`tokio::sync::Mutex<Inner>` —— 守 state + child handle
//! - `guards.holders`：`std::sync::Mutex<InflightState>` —— 守 guard 个数 + last_used_at
//!
//! **锁顺序：`turn` → `inner` → `holders`，禁止反向**。`maybe_idle_stop` 在持
//! `inner` 时短暂取 `holders`，符合此顺序；发放、归还 guard 都在 `inner` 锁外完成。
//!
//! `holders` 用 `std::sync::Mutex` 而非 `tokio::sync::Mutex` 是为了让
//! [`EngineGuard`] 的 drop 能同步访问（drop 不能 await）；临界区只是 usize +
//! Instant 赋值，几纳秒级别，不会因同步锁阻塞 runtime。

use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::process::Child;
use tokio::sync::Mutex;

use crate::ai::binary;
use crate::error::{Error, Result};

mod command;
mod guard;
mod idle;
mod logs;
pub use command::{EngineStartOverrides, DEFAULT_CTX_SIZE};
pub use guard::EngineGuard;

use command::build_command;
use logs::Logs;

/// 启动后等 `/health` 返回 200 的最大时长。
/// vision LLM 加载耗时随模型大小，9B 量级模型 CPU 上加载 30-60s 是常态。
const HEALTH_TIMEOUT: Duration = Duration::from_secs(90);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(500);

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
    /// `Running` 且无 in-flight 请求时，距离 idle watcher 自动 stop 还剩多少秒。
    /// 有 in-flight 请求时为 `None`（"正忙"）；状态非 Running 也为 `None`。
    pub idle_seconds_remaining: Option<u64>,
}

/// llama-server 子进程的单例守护者。
pub struct EngineSupervisor {
    inner: Mutex<Inner>,
    logs: Logs,
    /// 发出去的 guard。空闲回收看它。
    guards: guard::Guards,
    /// 当前（或最近一次启动时）加载的 main GGUF 路径。调用方在"复用 Running 引擎"
    /// 之前用 [`Self::loaded_main`] 校验装的是不是自己需要的模型——否则周报可能
    /// 被 2 分钟前日报留下的 vision 模型+小上下文生成（截断/低质/报错）。
    loaded_main: std::sync::Mutex<Option<PathBuf>>,
    /// 当前（或最近一次启动时）的 EngineStartOverrides。与 [`Self::loaded_main`] 配套：
    /// 复用 Running 引擎前还要校验 ctx / slots / batch 是否匹配——模型相同但用默认
    /// 8K ctx 起的引擎（如手动 start_engine）拿去跑配了 64K ctx 的 step 会请求超限。
    loaded_overrides: std::sync::Mutex<EngineStartOverrides>,
}

#[derive(Default)]
struct Inner {
    state: EngineRuntimeStatus,
    child: Option<Child>,
    /// 启动世代计数：每次 start spawn / stop 都 +1。start 的收尾阶段（health 判定后）
    /// 用它识别"自己等待期间引擎已被 stop()/新的 start 接管"——此时 child/state 已
    /// 不属于本次调用，不能 kill 或改状态，否则会误杀新启动的健康引擎 / 把 Stopped
    /// 冲成 Error。
    generation: u64,
}

/// 发出去的 guard 个数 + 最后一次活跃时间戳。
struct InflightState {
    /// 还没归还的 [`EngineGuard`] 个数；> 0 时 watcher 不 stop 引擎。
    count: usize,
    /// 最后一次 acquire / release 的时刻。watcher 用 `elapsed()` 判断是否 idle 超阈。
    last_used_at: Instant,
}

impl Default for InflightState {
    fn default() -> Self {
        Self {
            count: 0,
            last_used_at: Instant::now(),
        }
    }
}

impl Default for EngineSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineSupervisor {
    /// 创建一个 Stopped 状态的 supervisor。lib.rs 里 `Arc::new` 后通过 `app.manage` 注册。
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            logs: Logs::new(),
            guards: guard::Guards::default(),
            loaded_main: std::sync::Mutex::new(None),
            loaded_overrides: std::sync::Mutex::new(EngineStartOverrides::default()),
        }
    }

    /// 当前（或最近一次启动加载）的 main GGUF 路径；从未启动过为 None。
    pub fn loaded_main(&self) -> Option<PathBuf> {
        self.loaded_main.lock().ok().and_then(|g| g.clone())
    }

    /// 当前（或最近一次启动时）的 EngineStartOverrides。复用 Running 引擎前
    /// 与 [`Self::loaded_main`] 一起校验。
    pub fn loaded_overrides(&self) -> EngineStartOverrides {
        self.loaded_overrides
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// 当前状态快照（前端用）。Running 时附带 idle 倒计时，让前端能展示
    /// "X 秒后自动释放显存"。
    pub async fn status(&self) -> EngineRuntimeStatus {
        let mut s = self.inner.lock().await.state.clone();
        if s.state == EngineState::Running {
            s.idle_seconds_remaining = self.idle_eta().map(|d| d.as_secs());
        }
        s
    }

    /// Starts llama-server with explicit engine parameter overrides.
    ///
    /// Returns the running port without respawning if already `Running`, or errors if `Starting`.
    async fn start_with_overrides(
        &self,
        model_path: PathBuf,
        overrides: EngineStartOverrides,
    ) -> Result<u16> {
        // 记录本次要加载的 main 路径（Running 早退分支不覆盖——那时装的还是旧的）。
        // 供 loaded_main() 查询："复用已在跑的引擎"前调用方能校验装的是不是自己要的模型。
        let requested_main = Some(model_path.clone());
        // 第一段：占锁、预检、置 starting、spawn、释放锁
        let (port, my_gen) = {
            let mut inner = self.inner.lock().await;
            match inner.state.state {
                EngineState::Running => {
                    if let Some(p) = inner.state.port {
                        return Ok(p);
                    }
                }
                EngineState::Starting => {
                    return Err(Error::EngineBusy);
                }
                EngineState::Stopped | EngineState::Error => {}
            }
            *self.loaded_main.lock().expect("loaded_main 锁不该毒化") = requested_main;
            *self
                .loaded_overrides
                .lock()
                .expect("loaded_overrides 锁不该毒化") = overrides.clone();

            let bin_path = binary::binary_path()?;
            if !bin_path.exists() {
                let msg = "AI 引擎 binary 未安装，先去下载".to_string();
                inner.state = EngineRuntimeStatus {
                    state: EngineState::Error,
                    port: None,
                    error: Some(msg.clone()),
                    ..Default::default()
                };
                return Err(Error::EngineStart(msg));
            }

            let port = pick_free_port()?;
            let main_name = model_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "<none>".to_string());
            let mut cmd = build_command(&bin_path, port, &model_path, &overrides);
            // 装平台特定的 spawn 前钩子（Linux/macOS 设 setpgid + PDEATHSIG）；
            // Windows 是 no-op，post-spawn 才装 Job
            if let Err(e) = crate::ai::job_guard::prepare_command(&mut cmd) {
                log::warn!("job_guard::prepare_command 失败（保护可能降级）: {e}");
            }
            // 调试用：把最终拼出的命令行打到 log，方便排查 -np / --batch-size 是否生效，
            // 同时打模型文件名，确认日报、周报、聊天之间切换时加载的是哪个模型。
            // 标记 [engine-spawn] 让用户在长 log 里 grep 能直接定位。
            log::info!(
                "[engine-spawn] port={port} main={main_name} batch={:?} -np={:?} ctx_per_slot={:?}",
                overrides.batch_size,
                overrides.parallel_slots,
                overrides.ctx_size,
            );

            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    let msg = format!("spawn 失败：{e}");
                    inner.state = EngineRuntimeStatus {
                        state: EngineState::Error,
                        port: None,
                        error: Some(msg.clone()),
                        ..Default::default()
                    };
                    return Err(Error::Io(e));
                }
            };

            // 把 child 加到全局 Job Object —— Hindsight 进程死时 OS 内核会同步杀光
            // 所有 Job 成员，无视父进程怎么死（panic / Ctrl+C / taskkill）。
            // assign 失败不阻塞启动，只 log 警告退化到原 Exit hook 路径。
            // Linux / macOS 上 assign_child_pid 是 no-op。
            if let Some(pid) = child.id() {
                if let Err(e) = crate::ai::job_guard::assign_child_pid(pid) {
                    log::warn!("AssignProcessToJobObject pid={pid} 失败: {e}");
                }
            }

            // 启动前清空日志缓冲，让本次启动从空白开始
            self.logs.clear().await;
            if let Some(stderr) = child.stderr.take() {
                self.logs.collect(stderr, "stderr");
            }
            if let Some(stdout) = child.stdout.take() {
                self.logs.collect(stdout, "stdout");
            }

            inner.state = EngineRuntimeStatus {
                state: EngineState::Starting,
                port: Some(port),
                error: None,
                ..Default::default()
            };
            inner.child = Some(child);
            inner.generation = inner.generation.wrapping_add(1);
            (port, inner.generation)
        };

        // 第二段：不持锁等 health（持锁等会卡住其它 status 查询）。
        // poll_health 每轮轮询前会检查 inner（state 还是不是 Starting / child 是否退出 /
        // generation 是否被 stop()/新 start 推进），让外部变化能立刻 break，不必等满 90s。
        let healthy = poll_health(port, HEALTH_TIMEOUT, &self.inner, my_gen).await;

        // 第三段：根据 health 结果定状态
        let mut inner = self.inner.lock().await;
        if inner.generation != my_gen {
            // 等待期间引擎被 stop() / 新的 start 接管：当前 child/state 属于接管方，
            // 本次调用不能动它们（否则误杀新引擎 / 冲掉新状态），只报自己失败
            return Err(Error::EngineStart(
                "启动等待期间引擎被 stop/新的 start 接管".to_string(),
            ));
        }
        if healthy {
            inner.state = EngineRuntimeStatus {
                state: EngineState::Running,
                port: Some(port),
                error: None,
                ..Default::default()
            };
            // 空闲计时从启动完成算起。guard 的个数不动：来要引擎的那一方已经拿着一个
            if let Ok(mut s) = self.guards.holders.lock() {
                s.last_used_at = Instant::now();
            }
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
                ..Default::default()
            };
            inner.child = None;
            Err(Error::EngineStart(err_msg))
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
            // 推进世代：让还在等 health 的旧 start 立刻退出，且不把 Stopped 冲成 Error
            inner.generation = inner.generation.wrapping_add(1);
            inner.child.take()
        };
        if let Some(mut child) = child {
            let _ = child.kill().await;
            wait_child_with_timeout(&mut child).await;
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

/// 轮询 /health 等启动完成。每轮前先检查 inner：
/// - generation 变了（stop() / 新的 start 接管）→ 立刻 break，不再等
/// - state 不是 Starting（stop() 改成了 Stopped）→ 立刻 break，不再等
/// - child 已退出（OOM / 配置错 fail-fast 等）→ 立刻 break，不死等满 timeout
///
/// 不加这些 early-exit 时，子进程刚 spawn 就 OOM 死的话还要等 90s 超时；
/// 用户点「停止」也要等 90s 才生效；stop 后紧接着新 start 时，旧 start 会
/// 盯着新 start 的 Starting 状态轮询自己已死的端口。
async fn poll_health(port: u16, timeout: Duration, inner: &Mutex<Inner>, my_gen: u64) -> bool {
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
        // early-exit：generation 被推进 / state 被外部 stop() 改 / child 已退出 → 不再轮询
        {
            let mut guard = inner.lock().await;
            if guard.generation != my_gen {
                return false;
            }
            if guard.state.state != EngineState::Starting {
                return false;
            }
            if let Some(child) = guard.child.as_mut() {
                if let Ok(Some(_status)) = child.try_wait() {
                    return false; // child 已死，没必要继续等 health
                }
            } else {
                return false; // child 句柄被 take 走了（stop() 抢走了）
            }
        }

        if let Ok(r) = client.get(&url).send().await {
            if r.status().is_success() {
                return true;
            }
        }
        tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
    }
    false
}

/// 启动失败时调，给前端可读的错误描述：读 `logs` 的末尾几行。
async fn resolve_failure(child: &mut Child, logs: &Logs) -> String {
    let exit_status = child.try_wait();
    let killed = matches!(exit_status, Ok(None));
    if killed {
        // 还活着但 hang——强制 kill
        let _ = child.kill().await;
        wait_child_with_timeout(child).await;
        // 等 50ms 让 drain task 把残余 stderr 行写完
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let tail = logs.tail(12).await;
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
                format!(
                    "启动 {}s 未响应 /health，已强制 kill",
                    HEALTH_TIMEOUT.as_secs()
                )
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

/// 限时等子进程 wait()，避免子进程卡住把 idle watcher / stop() 调用栈吊死。
///
/// kill 信号已发送到内核层；child.wait() 主要等内核回收 PID，正常 ms 级返回。
/// 超过 [`SHUTDOWN_WAIT_TIMEOUT`] 还没回 = 进程被 D 状态卡死或者僵尸表满了，
/// 此时再等下去也意义不大，放弃等待让上层流程往下走。
const SHUTDOWN_WAIT_TIMEOUT: Duration = Duration::from_secs(5);

async fn wait_child_with_timeout(child: &mut Child) {
    if tokio::time::timeout(SHUTDOWN_WAIT_TIMEOUT, child.wait())
        .await
        .is_err()
    {
        log::warn!(
            "llama-server wait 超过 {}s 仍未退出，放弃等待（OS 应已收到 kill 信号自行清理）",
            SHUTDOWN_WAIT_TIMEOUT.as_secs()
        );
    }
}
