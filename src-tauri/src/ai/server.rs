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
//! 本模块持有两把锁：
//! - `inner`：`tokio::sync::Mutex<Inner>` —— 守 state + child handle
//! - `inflight_state`：`std::sync::Mutex<InflightState>` —— 守 in-flight 计数 + last_used_at
//!
//! **锁顺序：`inner` → `inflight_state`，禁止反向**。`maybe_idle_stop` 在持
//! `inner` 时短暂取 `inflight_state`，符合此顺序；任何 acquire / release inflight
//! 的调用点都必须在 `inner` 锁外完成（或确保不会回头取 `inner`）。
//!
//! `inflight_state` 用 `std::sync::Mutex` 而非 `tokio::sync::Mutex` 是为了让
//! [`InferenceGuard::drop`] 能同步访问（drop 不能 await）；临界区只是 usize +
//! Instant 赋值，几纳秒级别，不会因同步锁阻塞 runtime。
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

/// 引擎闲置多久没新请求 → 自动 stop 释放显存。
/// 跑一段 step1+step2 大约 60-120s；阈值取 120s 让用户在跑总结之间稍稍歇会儿
/// 也不会被释放，但跑完真闲下来 2 分钟就回收 GPU。
const IDLE_THRESHOLD: Duration = Duration::from_secs(120);
/// idle watcher 检查频率。
const IDLE_TICK: Duration = Duration::from_secs(10);

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
    /// `Running` 且无 in-flight 请求时，距离 idle watcher 自动 stop 还剩多少秒。
    /// 有 in-flight 请求时为 `None`（"正忙"）；状态非 Running 也为 `None`。
    pub idle_seconds_remaining: Option<u64>,
}

/// 启动 llama-server 时可选的命令行调参。
///
/// 调试 tab 用：调一次跑一次的 batch / 并发槽位，不进 settings.ai 全局，每次
/// 调试 generate 之前先 stop + start with overrides，跑完再 stop 让下次正常
/// 日报 lazy start 时回到默认值。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineStartOverrides {
    /// 同时设 `--batch-size N --ubatch-size N`。`None` = 用 llama.cpp 默认（512）。
    /// 加大能提升 prompt eval 速度，代价是 KV cache 高水位 ↑（5090 32GB 装得下 4096）。
    pub batch_size: Option<u32>,
    /// `-np N`：llama-server 同时处理的并行槽位数；`None` = 1（单槽）。
    /// 配合 `summary.rs` 的 `buffer_unordered` 才能真正并发——只设 -np 不并发调用没用。
    pub parallel_slots: Option<u32>,
    /// **每个 slot** 的 ctx 上限。`None` = [`DEFAULT_CTX_SIZE`]（8K）。
    /// 实际传给 llama-server 的 `--ctx-size` 是 `ctx_size × parallel_slots`，
    /// 让每个 slot 都拿到 user 选的 budget；llama.cpp 启动时按这个总量
    /// 一次性 mmap KV cache（VRAM / RAM 直接吃掉，不是按需增长）。
    pub ctx_size: Option<u32>,
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
    /// 推理请求计数 + 最后一次使用时间——给 idle watcher 自动 stop 用。
    /// 用 std sync mutex 而不是 tokio mutex：临界区只是 usize+Instant 赋值，几纳秒，
    /// 同时让 [`InferenceGuard::drop`] 能同步访问（drop 不能 await tokio mutex）。
    inflight_state: std::sync::Mutex<InflightState>,
}

const LOGS_RING_SIZE: usize = 500;
/// 启动日志保留行数——头 200 行（cuBLAS init、模型加载、layer offload 等都在这里）。
const STARTUP_LINES: usize = 200;

#[derive(Default)]
struct Inner {
    state: EngineRuntimeStatus,
    child: Option<Child>,
}

/// in-flight 推理请求计数器 + 最后一次活跃时间戳。
struct InflightState {
    /// 当前持有 [`InferenceGuard`] 的请求数；> 0 时 watcher 不 stop 引擎。
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

/// 一次推理请求的 RAII 守护：在 `drop()` 时把 in-flight 计数减 1，并把
/// `last_used_at` 推到当前时刻。配合 idle watcher 实现"跑完任务 N 秒无新请求
/// → 自动 stop 引擎释放显存"。
///
/// 用法（必须在调 `chat.chat_*` **之前** acquire，跨 await 持有到请求返回）：
/// ```ignore
/// let _g = supervisor.acquire_inference();  // in-flight++
/// chat.chat_with_images(...).await?;        // 真正发请求
/// // _g 在此处 drop → in-flight--
/// ```
pub struct InferenceGuard {
    sup: Arc<EngineSupervisor>,
}

impl Drop for InferenceGuard {
    fn drop(&mut self) {
        // std::sync::Mutex 的 lock 在毒化时也能恢复；poison 时仅记录不 panic
        if let Ok(mut s) = self.sup.inflight_state.lock() {
            s.count = s.count.saturating_sub(1);
            s.last_used_at = Instant::now();
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
            startup_logs: Arc::new(Mutex::new(Vec::with_capacity(STARTUP_LINES))),
            logs: Arc::new(Mutex::new(VecDeque::with_capacity(LOGS_RING_SIZE))),
            inflight_state: std::sync::Mutex::new(InflightState::default()),
        }
    }

    /// 注册一次推理请求：in-flight +1 + last_used_at 推到现在。
    /// 调用方应把返回的 guard 持有到请求结束（drop 时自动 -1）。
    ///
    /// `Arc<Self>` 的方法签名让 guard 持有 supervisor 的强引用，
    /// 保证 guard 还在期间 supervisor 不会被释放。
    pub fn acquire_inference(self: &Arc<Self>) -> InferenceGuard {
        if let Ok(mut s) = self.inflight_state.lock() {
            s.count += 1;
            s.last_used_at = Instant::now();
        }
        InferenceGuard {
            sup: Arc::clone(self),
        }
    }

    /// 启动 idle watcher：每 [`IDLE_TICK`] 检查一次，引擎 Running + in-flight==0
    /// + idle > [`IDLE_THRESHOLD`] 时自动 stop 释放显存。
    ///
    /// watcher 持 [`Weak`](std::sync::Weak) 引用——supervisor 被 drop 后 watcher 自然退出，
    /// 不会泄漏 task。lib.rs 里 supervisor 创建后调一次即可，永久后台跑。
    pub fn spawn_idle_watcher(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(IDLE_TICK);
            // 第一次 tick 是立即触发，跳过它——避免刚启动就检查
            interval.tick().await;
            loop {
                interval.tick().await;
                let Some(sup) = weak.upgrade() else { break };
                let _ = sup.maybe_idle_stop().await;
            }
        })
    }

    /// 返回距离下次 idle 触发还有多久（用户可观测）。
    /// 引擎不在 Running / 还有 in-flight 时返回 None。
    pub fn idle_eta(&self) -> Option<Duration> {
        let s = self.inflight_state.lock().ok()?;
        if s.count > 0 {
            return None;
        }
        let elapsed = s.last_used_at.elapsed();
        if elapsed >= IDLE_THRESHOLD {
            Some(Duration::ZERO)
        } else {
            Some(IDLE_THRESHOLD - elapsed)
        }
    }

    /// 检查当前是否满足 idle 释放条件，是则 stop 引擎。
    /// 持 inner mutex 期间检查 in-flight + take child，保证 watcher 与 acquire 不抢同一窗口。
    /// 返回 true 表示真的执行了一次 idle stop。
    async fn maybe_idle_stop(&self) -> bool {
        let child = {
            let mut inner = self.inner.lock().await;
            if inner.state.state != EngineState::Running {
                return false;
            }
            // inflight 锁的临界区极短（usize 比较 + Instant.elapsed），不会因为
            // 持着 inner 锁阻塞而拉长——nested lock 顺序：inner → inflight，
            // 全程不反向，无死锁风险
            let s = match self.inflight_state.lock() {
                Ok(g) => g,
                Err(_) => return false,
            };
            if s.count > 0 || s.last_used_at.elapsed() < IDLE_THRESHOLD {
                return false;
            }
            drop(s);
            inner.state = EngineRuntimeStatus::default();
            inner.child.take()
        };
        if let Some(mut child) = child {
            let _ = child.kill().await;
            wait_child_with_timeout(&mut child).await;
            log::info!(
                "AI 引擎 idle 超过 {}s，已自动释放显存",
                IDLE_THRESHOLD.as_secs()
            );
            true
        } else {
            false
        }
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
        self.start_with_overrides(model_path, mmproj_path, EngineStartOverrides::default())
            .await
    }

    /// 跟 [`start`] 一样，但允许调试 tab 临时覆盖 batch_size / parallel_slots。
    pub async fn start_with_overrides(
        &self,
        model_path: Option<PathBuf>,
        mmproj_path: Option<PathBuf>,
        overrides: EngineStartOverrides,
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
                    return Err(Error::EngineBusy);
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
                    ..Default::default()
                };
                return Err(Error::EngineStart(msg));
            }

            let port = pick_free_port()?;
            // 在 build_command 消费 path 之前抽出文件名给 log 用——验证 step 间确实加载不同模型
            let main_name = model_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "<none>".to_string());
            let mmproj_name = mmproj_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "<none>".to_string());
            let mut cmd = build_command(&bin_path, port, model_path, mmproj_path, &overrides);
            // 装平台特定的 spawn 前钩子（Linux/macOS 设 setpgid + PDEATHSIG）；
            // Windows 是 no-op，post-spawn 才装 Job
            if let Err(e) = crate::ai::job_guard::prepare_command(&mut cmd) {
                log::warn!("job_guard::prepare_command 失败（保护可能降级）: {e}");
            }
            // 调试用：把最终拼出的命令行打到 log，方便排查 -np / --batch-size 是否生效，
            // 同时打 main / mmproj 文件名验证 step 1/2 切换时确实加载了不同模型。
            // 标记 [engine-spawn] 让用户在长 log 里 grep 能直接定位。mmproj=<none> 表示
            // 此次启动**没**带 vision projection（应该是文本模型 step 2 的预期行为）；
            // 文本模型这里仍带着某个 *-mmproj.gguf 说明 effective_summary_mmproj fallback 出 bug。
            log::info!(
                "[engine-spawn] port={port} main={main_name} mmproj={mmproj_name} batch={:?} -np={:?} ctx_per_slot={:?}",
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
                ..Default::default()
            };
            inner.child = Some(child);
            port
        };

        // 第二段：不持锁等 health（持锁等会卡住其它 status 查询）。
        // poll_health 每轮轮询前会检查 inner（state 还是不是 Starting / child 是否退出），
        // 让 stop() 改 state 或子进程自己 OOM 死时能立刻 break，不必等满 90s 超时。
        let healthy = poll_health(port, HEALTH_TIMEOUT, &self.inner).await;

        // 第三段：根据 health 结果定状态
        let mut inner = self.inner.lock().await;
        if healthy {
            inner.state = EngineRuntimeStatus {
                state: EngineState::Running,
                port: Some(port),
                error: None,
                ..Default::default()
            };
            // 新一轮启动：把 inflight 计数清零 + last_used_at 推到现在。
            // 不重置的话上一轮残留的旧 last_used 会让新启动立刻被 watcher 当成 idle 干掉。
            if let Ok(mut s) = self.inflight_state.lock() {
                *s = InflightState::default();
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

    /// 停止当前引擎，再用新 overrides 启动一个——给"两阶段不同参数"的 runner 用。
    /// 内部串行：`stop()` → `start_with_overrides()`。
    /// 失败语义：stop 失败立刻返回错误（不再启动）；start 失败时引擎已是 Stopped 状态。
    pub async fn restart_with_overrides(
        &self,
        model_path: Option<PathBuf>,
        mmproj_path: Option<PathBuf>,
        overrides: EngineStartOverrides,
    ) -> Result<u16> {
        self.stop().await?;
        self.start_with_overrides(model_path, mmproj_path, overrides)
            .await
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
    overrides: &EngineStartOverrides,
) -> Command {
    let mut cmd = Command::new(bin);
    cmd.arg("--host").arg("127.0.0.1");
    cmd.arg("--port").arg(port.to_string());
    if let Some(m) = model_path {
        cmd.arg("-m").arg(m);
    }
    if let Some(p) = mmproj_path {
        cmd.arg("--mmproj").arg(p);
    }
    if let Some(n) = gpu_layers_for(platform::detect()) {
        cmd.arg("-ngl").arg(n.to_string());
    }
    // batch / ubatch：同时设两值保持 logical=physical batch 一致，
    // 用户能预测显存占用；不像 llama.cpp 默认 batch=2048 / ubatch=512 拉开两倍。
    if let Some(b) = overrides.batch_size {
        let b = b.max(32); // llama-server 拒绝过小的 batch；32 是安全下限
        cmd.arg("--batch-size").arg(b.to_string());
        cmd.arg("--ubatch-size").arg(b.to_string());
    }
    // ctx-size + parallel slots 协同：llama-server 把 --ctx-size 平均分给
    // np 个 slot（per-slot = ctx_size / np）。所以这里实际传的 --ctx-size
    // 必须是 (per-slot 上限 × np)，让每个 slot 都拿到用户选的 budget。
    // 否则 -np 4 时 user 看到的 8K base 会缩成每槽 2048，长 prompt 直接 400。
    let np = overrides.parallel_slots.unwrap_or(1).max(1);
    let per_slot_ctx = overrides.ctx_size.unwrap_or(DEFAULT_CTX_SIZE);
    cmd.arg("--ctx-size")
        .arg(per_slot_ctx.saturating_mul(np).to_string());
    // 总是显式传 -np，哪怕是 1。不传时 llama-server auto-fallback 到 4 slot，
    // 让用户的"段总结 single slot"配置失效——summary phase 也起 4 槽，浪费 KV 预算 +
    // 让 checkpoint 池跨 slot 翻 4 倍。
    cmd.arg("-np").arg(np.to_string());
    // 关掉 llama.cpp 新版默认开的 prompt cache：8 GB 上限，纯吃显存（日报场景每段
    // prompt 都不同，命中率几乎为 0）。
    cmd.arg("--cache-ram").arg("0");
    // 关掉 context checkpoints：每个 50 MiB × max 32 个 × N slots 持续累积——大模型 +
    // 多 slot 时能堆出 GB 级开销，导致段总结生成途中 server 崩
    // （"connection closed before message completed"）。我们没复用 prompt prefix 的
    // 场景，关掉无副作用。
    cmd.arg("--ctx-checkpoints").arg("0");
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

/// 轮询 /health 等启动完成。每轮前先检查 inner：
/// - state 不是 Starting（stop() 改成了 Stopped）→ 立刻 break，不再等
/// - child 已退出（OOM / 配置错 fail-fast 等）→ 立刻 break，不死等满 timeout
///
/// 不加这两个 early-exit 时，子进程刚 spawn 就 OOM 死的话还要等 90s 超时；
/// 用户点「停止」也要等 90s 才生效。
async fn poll_health(port: u16, timeout: Duration, inner: &Mutex<Inner>) -> bool {
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
        // early-exit：state 被外部 stop() 改 / child 已退出 → 不再轮询
        {
            let mut guard = inner.lock().await;
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

/// 启动失败时调，给前端可读的错误描述。
/// stderr 已被 [`spawn_drain_task`] 持续消费到 logs ring，这里直接读末尾几行。
async fn resolve_failure(child: &mut Child, logs: &Arc<Mutex<VecDeque<String>>>) -> String {
    let exit_status = child.try_wait();
    let killed = matches!(exit_status, Ok(None));
    if killed {
        // 还活着但 hang——强制 kill
        let _ = child.kill().await;
        wait_child_with_timeout(child).await;
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

/// 读 logs ring 末尾 N 行拼成多行字符串，给错误信息 / 调试 UI 用。
async fn log_tail(logs: &Arc<Mutex<VecDeque<String>>>, n: usize) -> String {
    let g = logs.lock().await;
    let start = g.len().saturating_sub(n);
    g.iter().skip(start).cloned().collect::<Vec<_>>().join("\n")
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
                    // llama-server 的 stderr / stdout 转发到 log 框架。release 时跟
                    // `env_logger` 的 RUST_LOG 过滤一致；dev 模式（debug_assertions）
                    // 再额外 eprintln 一份到 console，确保 cuBLAS init / offloaded
                    // layers 这种启动诊断信息一定能在终端看到。
                    log::info!("[llama-server] {prefixed}");
                    #[cfg(debug_assertions)]
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
                Ok(None) => break, // EOF：子进程关闭了管道
                Err(_) => break,   // 读错误：直接退出 task
            }
        }
    });
}
