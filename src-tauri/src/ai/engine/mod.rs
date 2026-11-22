//! `llama-server` 子进程管理 (Phase 1B-α)。
//!
//! [`EngineSupervisor`] 在 `Arc<EngineSupervisor>` 管理之下作为 app 单例：
//! - lazy spawn：只有用户点"启动引擎"或第一次跑 AI 总结时才拉起
//! - app 退出钩子里 [`stop()`](EngineSupervisor::stop) 会被调，等子进程收尸再让 Tauri 退
//!
//! 进程状态机（[`Slot`]）：
//! ```text
//!   Stopped → Starting → Running
//!                ↓
//!              Failed
//! ```
//! 哪一步 stop 都回到 Stopped；Running 闲久了也回到 Stopped。
//!
//! ## 锁顺序约定
//!
//! 本模块持有三把锁：
//! - `guards.turn`：`tokio::sync::Mutex<()>` —— 来要引擎的排队
//! - `inner`：`tokio::sync::Mutex<Inner>` —— 守 slot + 世代
//! - `guards.holders`：`std::sync::Mutex<InflightState>` —— 守 guard 个数 + last_used_at
//!
//! **锁顺序：`turn` → `inner` → `holders`，禁止反向**。`maybe_idle_stop` 在持
//! `inner` 时短暂取 `holders`，符合此顺序；发放、归还 guard 都在 `inner` 锁外完成。
//!
//! `holders` 用 `std::sync::Mutex` 而非 `tokio::sync::Mutex` 是为了让
//! [`EngineGuard`] 的 drop 能同步访问（drop 不能 await）；临界区只是 usize +
//! Instant 赋值，几纳秒级别，不会因同步锁阻塞 runtime。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::Mutex;

use crate::ai::binary;
use crate::error::{Error, Result};

mod command;
mod guard;
mod idle;
mod logs;
mod server;
pub use command::{EngineStartOverrides, DEFAULT_CTX_SIZE};
pub use guard::EngineGuard;

use logs::Logs;
use server::LlamaServer;

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
}

#[derive(Default)]
struct Inner {
    slot: Slot,
    /// 启动世代计数：每次 start spawn / stop 都 +1。start 的收尾阶段（health 判定后）
    /// 用它识别"自己等待期间引擎已被 stop()/新的 start 接管"——此时槽里的已
    /// 不属于本次调用，不能 kill 或改状态，否则会误杀新启动的健康引擎 / 把 Stopped
    /// 冲成 Failed。
    generation: u64,
}

/// 引擎现在什么样。起着、跑着时带着进程。
#[derive(Default)]
enum Slot {
    #[default]
    Stopped,
    Starting(LlamaServer),
    Running(LlamaServer),
    /// 上一次没起来，带着给前端看的原因。
    Failed(String),
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
        }
    }

    /// 当前状态快照（前端用）。Running 时附带 idle 倒计时，让前端能展示
    /// "X 秒后自动释放显存"。
    pub async fn status(&self) -> EngineRuntimeStatus {
        let inner = self.inner.lock().await;
        match &inner.slot {
            Slot::Stopped => EngineRuntimeStatus::default(),
            Slot::Starting(s) => EngineRuntimeStatus {
                state: EngineState::Starting,
                port: Some(s.loaded.port),
                ..Default::default()
            },
            Slot::Running(s) => EngineRuntimeStatus {
                state: EngineState::Running,
                port: Some(s.loaded.port),
                idle_seconds_remaining: self.idle_eta().map(|d| d.as_secs()),
                ..Default::default()
            },
            Slot::Failed(msg) => EngineRuntimeStatus {
                state: EngineState::Error,
                error: Some(msg.clone()),
                ..Default::default()
            },
        }
    }

    /// 停止子进程。kill + wait 收尸，状态切回 Stopped。
    /// 已经 Stopped 时是 no-op。
    ///
    /// 两段式：先持锁取走进程 + 置 stopped，立刻释放锁；再不持锁 kill/wait。
    /// 子进程慢退出（数秒）时其它 status() 调用不会被 hold，避免与
    /// `lib.rs` 的 `RunEvent::Exit` 钩子里 `block_on(stop())` 配合时死等。
    pub async fn stop(&self) -> Result<()> {
        let slot = {
            let mut inner = self.inner.lock().await;
            // 推进世代：让还在等 health 的旧 start 立刻退出，且不把 Stopped 冲成 Failed
            inner.generation = inner.generation.wrapping_add(1);
            std::mem::take(&mut inner.slot)
        };
        if let Slot::Starting(server) | Slot::Running(server) = slot {
            server.kill().await;
        }
        Ok(())
    }

    /// 按 `params` 拉起装着 `model` 的引擎，等它能用了返回端口。
    ///
    /// # Errors
    ///
    /// 已经有引擎在起、在跑时返回 [`Error::EngineBusy`]。没装引擎、没等到它能用时返回
    /// [`Error::EngineStart`]，拉不起进程时返回 [`Error::Io`]；这几种的原因也写进状态给前端看。
    async fn start(&self, model: PathBuf, params: EngineStartOverrides) -> Result<u16> {
        // 第一段：占锁、预检、spawn、置 Starting、释放锁
        let (port, my_gen) = {
            let mut inner = self.inner.lock().await;
            if matches!(inner.slot, Slot::Starting(_) | Slot::Running(_)) {
                return Err(Error::EngineBusy);
            }
            let bin = binary::binary_path()?;
            if !bin.exists() {
                let msg = "AI 引擎 binary 未安装，先去下载".to_string();
                inner.slot = Slot::Failed(msg.clone());
                return Err(Error::EngineStart(msg));
            }
            let server = match LlamaServer::spawn(&bin, model, params, &self.logs).await {
                Ok(s) => s,
                Err(e) => {
                    inner.slot = Slot::Failed(format!("spawn 失败：{e}"));
                    return Err(Error::Io(e));
                }
            };
            let port = server.loaded.port;
            inner.slot = Slot::Starting(server);
            inner.generation = inner.generation.wrapping_add(1);
            (port, inner.generation)
        };

        // 第二段：不持锁等 health（持锁等会卡住其它 status 查询）
        let healthy = self.wait_until_healthy(port, my_gen).await;

        // 第三段：根据 health 结果定状态
        let mut inner = self.inner.lock().await;
        let server = match std::mem::take(&mut inner.slot) {
            Slot::Starting(s) if inner.generation == my_gen => s,
            other => {
                // 等待期间引擎被 stop() / 新的 start 接管：槽里的属于接管方，
                // 本次调用不能动它（否则误杀新引擎 / 冲掉新状态），只报自己失败
                inner.slot = other;
                return Err(Error::EngineStart(
                    "启动等待期间引擎被 stop/新的 start 接管".to_string(),
                ));
            }
        };
        if healthy {
            inner.slot = Slot::Running(server);
            // 空闲计时从启动完成算起。guard 的个数不动：来要引擎的那一方已经拿着一个
            if let Ok(mut s) = self.guards.holders.lock() {
                s.last_used_at = Instant::now();
            }
            Ok(port)
        } else {
            // /health 没等到 200——可能子进程已退出（缺模型 / 配置错），
            // 也可能仍活着但 hang 住。两种情况都从 logs ring 拿末尾几行作错误描述。
            let msg = server.failure_reason(&self.logs).await;
            inner.slot = Slot::Failed(msg.clone());
            Err(Error::EngineStart(msg))
        }
    }

    /// 轮询 /health 等启动完成。每轮前先看这次起的还在不在起：
    /// - 世代变了、槽不是 Starting 了（stop() / 新的 start 接管）→ 不再等
    /// - 子进程已退出（OOM / 配置错 fail-fast 等）→ 不再等，不死等满 timeout
    ///
    /// 不加这些 early-exit 时，子进程刚 spawn 就 OOM 死的话还要等 90s 超时；
    /// 用户点「停止」也要等 90s 才生效；stop 后紧接着新 start 时，旧 start 会
    /// 盯着新 start 的 Starting 状态轮询自己已死的端口。
    async fn wait_until_healthy(&self, port: u16, my_gen: u64) -> bool {
        let deadline = Instant::now() + HEALTH_TIMEOUT;
        let url = format!("http://127.0.0.1:{port}/health");
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
        {
            Ok(c) => c,
            Err(_) => return false,
        };
        while Instant::now() < deadline {
            if !self.still_starting(my_gen).await {
                return false;
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

    /// 槽里还是第 `my_gen` 次起的那个，进程也还活着。
    async fn still_starting(&self, my_gen: u64) -> bool {
        let mut inner = self.inner.lock().await;
        if inner.generation != my_gen {
            return false;
        }
        match &mut inner.slot {
            Slot::Starting(s) => !s.exited(),
            _ => false,
        }
    }
}
