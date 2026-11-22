//! 日报、周报、聊天、测试连接要用本机引擎时，都从这里拿一个 [`EngineGuard`]：
//!
//! - 没在跑：按要求启动。
//! - 在跑，模型相同，每槽上下文不小于要求：直接用。
//! - 不满足要求，没人拿着 guard：重启成要求的参数。
//! - 不满足要求，有人拿着 guard：报忙。
//!
//! 一次只处理一个来要引擎的。后来的等前一个起完再判断。

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::OwnedMutexGuard;

use super::{EngineStartOverrides, EngineState, EngineSupervisor, InflightState, DEFAULT_CTX_SIZE};
use crate::error::{Error, Result};

/// 拿着它就占着引擎。丢掉就归还，全部归还后空闲回收才会关引擎。
pub struct EngineGuard {
    port: u16,
    holders: Arc<Mutex<InflightState>>,
}

impl EngineGuard {
    fn new(holders: &Arc<Mutex<InflightState>>) -> Self {
        if let Ok(mut s) = holders.lock() {
            s.count += 1;
            s.last_used_at = Instant::now();
        }
        Self {
            port: 0,
            holders: Arc::clone(holders),
        }
    }

    /// 引擎的端口。
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for EngineGuard {
    fn drop(&mut self) {
        if let Ok(mut s) = self.holders.lock() {
            s.count = s.count.saturating_sub(1);
            s.last_used_at = Instant::now();
        }
    }
}

impl EngineSupervisor {
    /// 按 `wants` 拿一个装着 `model` 的引擎。规则见模块文档。
    ///
    /// # Errors
    ///
    /// 引擎被别人占着、要的又不一样时，返回 [`Error::EngineBusy`]。
    pub async fn acquire(
        self: &Arc<Self>,
        model: PathBuf,
        wants: EngineStartOverrides,
    ) -> Result<EngineGuard> {
        self.guards.acquire(self, model, wants).await
    }

    /// 没人拿着 guard 就关掉引擎。
    pub async fn stop_if_unused(self: &Arc<Self>) -> Result<()> {
        self.guards.stop_if_unused(self).await
    }
}

/// guard 通过它看引擎、启动、停止。测试里换成假的。
pub(super) trait Process: Send + Sync + 'static {
    /// 在跑的话，返回它的端口、模型和参数。
    fn running(&self) -> impl Future<Output = Option<Loaded>> + Send;
    fn start(
        &self,
        model: PathBuf,
        wants: EngineStartOverrides,
    ) -> impl Future<Output = Result<u16>> + Send;
    fn stop(&self) -> impl Future<Output = Result<()>> + Send;
}

/// 在跑的引擎装着什么。
#[derive(Debug, Clone)]
pub(super) struct Loaded {
    pub port: u16,
    pub model: PathBuf,
    pub params: EngineStartOverrides,
}

impl Process for EngineSupervisor {
    async fn running(&self) -> Option<Loaded> {
        let status = self.status().await;
        if status.state != EngineState::Running {
            return None;
        }
        Some(Loaded {
            port: status.port?,
            model: self.loaded_main()?,
            params: self.loaded_overrides(),
        })
    }

    async fn start(&self, model: PathBuf, wants: EngineStartOverrides) -> Result<u16> {
        self.start_with_overrides(model, wants).await
    }

    async fn stop(&self) -> Result<()> {
        EngineSupervisor::stop(self).await
    }
}

/// 发出去的 guard，和让来要引擎的排队的锁。
#[derive(Default)]
pub(super) struct Guards {
    pub(super) holders: Arc<Mutex<InflightState>>,
    turn: Arc<tokio::sync::Mutex<()>>,
}

impl Guards {
    fn count(&self) -> usize {
        self.holders.lock().map(|s| s.count).unwrap_or(0)
    }

    async fn acquire<P: Process>(
        &self,
        process: &Arc<P>,
        model: PathBuf,
        wants: EngineStartOverrides,
    ) -> Result<EngineGuard> {
        let turn = Arc::clone(&self.turn).lock_owned().await;
        // 先占上：判断完到返回之间，空闲回收不会关掉引擎
        let mut guard = EngineGuard::new(&self.holders);
        let others = self.count().saturating_sub(1);
        match process.running().await {
            Some(loaded) if fits(&loaded, &model, &wants) => guard.port = loaded.port,
            Some(_) if others > 0 => return Err(Error::EngineBusy),
            running => {
                let undo = StopIfDropped {
                    process: Arc::clone(process),
                    turn: Some(turn),
                };
                let started = async {
                    if running.is_some() {
                        process.stop().await?;
                    }
                    process.start(model, wants).await
                }
                .await;
                undo.disarm();
                guard.port = started?;
            }
        }
        Ok(guard)
    }

    async fn stop_if_unused<P: Process>(&self, process: &Arc<P>) -> Result<()> {
        // 排在来要引擎的后面：判断完到关掉之间，没人能拿到 guard
        let _turn = self.turn.lock().await;
        if self.count() == 0 {
            process.stop().await?;
        }
        Ok(())
    }
}

/// 模型相同、每槽上下文不小于要求，就能直接用。批大小和并发路数不看。
fn fits(loaded: &Loaded, model: &Path, wants: &EngineStartOverrides) -> bool {
    let ctx = |p: &EngineStartOverrides| p.ctx_size.unwrap_or(DEFAULT_CTX_SIZE);
    loaded.model == model && ctx(&loaded.params) >= ctx(wants)
}

/// 启动途中 acquire 被取消时，关掉这次启动的引擎。关完才放下一个来要引擎的进来。
struct StopIfDropped<P: Process> {
    process: Arc<P>,
    turn: Option<OwnedMutexGuard<()>>,
}

impl<P: Process> StopIfDropped<P> {
    fn disarm(mut self) {
        self.turn = None;
    }
}

impl<P: Process> Drop for StopIfDropped<P> {
    fn drop(&mut self) {
        let Some(turn) = self.turn.take() else {
            return;
        };
        let process = Arc::clone(&self.process);
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                let _ = process.stop().await;
                drop(turn);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
    use std::time::Duration;

    use super::*;
    use crate::ai::engine::idle::IDLE_THRESHOLD;

    /// 假引擎：启动、停止只改内存里的状态，记下次数。
    #[derive(Default)]
    struct FakeProcess {
        running: Mutex<Option<Loaded>>,
        starts: AtomicUsize,
        stops: AtomicUsize,
        fail: AtomicBool,
        /// 测试拿着这把锁时，启动卡在半路。
        gate: tokio::sync::Mutex<()>,
    }

    impl Process for FakeProcess {
        async fn running(&self) -> Option<Loaded> {
            self.running.lock().unwrap().clone()
        }

        async fn start(&self, model: PathBuf, wants: EngineStartOverrides) -> Result<u16> {
            let n = self.starts.fetch_add(1, SeqCst) + 1;
            let _open = self.gate.lock().await;
            if self.fail.load(SeqCst) {
                return Err(Error::EngineStart("假引擎起不来".into()));
            }
            let port = 5000 + n as u16;
            *self.running.lock().unwrap() = Some(Loaded {
                port,
                model,
                params: wants,
            });
            Ok(port)
        }

        async fn stop(&self) -> Result<()> {
            self.stops.fetch_add(1, SeqCst);
            *self.running.lock().unwrap() = None;
            Ok(())
        }
    }

    impl FakeProcess {
        fn starts(&self) -> usize {
            self.starts.load(SeqCst)
        }
        fn stops(&self) -> usize {
            self.stops.load(SeqCst)
        }
        fn loaded_model(&self) -> Option<PathBuf> {
            self.running
                .lock()
                .unwrap()
                .as_ref()
                .map(|l| l.model.clone())
        }
        /// 假装引擎已经装着 `model`、按 `params` 在 4242 端口跑着。
        fn run(&self, model: &str, params: EngineStartOverrides) {
            *self.running.lock().unwrap() = Some(Loaded {
                port: 4242,
                model: PathBuf::from(model),
                params,
            });
        }
    }

    fn setup() -> (Arc<Guards>, Arc<FakeProcess>) {
        (
            Arc::new(Guards::default()),
            Arc::new(FakeProcess::default()),
        )
    }

    fn ctx(n: Option<u32>) -> EngineStartOverrides {
        EngineStartOverrides {
            ctx_size: n,
            ..Default::default()
        }
    }

    fn spawn_acquire(
        guards: &Arc<Guards>,
        fake: &Arc<FakeProcess>,
        model: &str,
    ) -> tokio::task::JoinHandle<Result<EngineGuard>> {
        let (guards, fake, model) = (Arc::clone(guards), Arc::clone(fake), PathBuf::from(model));
        tokio::spawn(async move { guards.acquire(&fake, model, ctx(None)).await })
    }

    /// 等到 `done` 成立，最多 5 秒。
    async fn until(done: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !done() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("等了 5 秒还没到");
    }

    #[tokio::test]
    async fn starts_the_engine_when_none_is_running() {
        let (guards, fake) = setup();
        let g = guards
            .acquire(&fake, "a.gguf".into(), ctx(None))
            .await
            .unwrap();
        assert_eq!(fake.starts(), 1);
        assert_eq!(g.port(), 5001);
    }

    /// 批大小、并发路数不同也直接用；别人拿着也能一起用。
    #[tokio::test]
    async fn reuses_an_engine_with_the_same_model_and_enough_context() {
        let (guards, fake) = setup();
        fake.run(
            "a.gguf",
            EngineStartOverrides {
                ctx_size: Some(16384),
                batch_size: Some(2048),
                parallel_slots: Some(2),
            },
        );
        // 没设上下文 = 8K，小于 16K
        let first = guards
            .acquire(&fake, "a.gguf".into(), ctx(None))
            .await
            .unwrap();
        let second = guards
            .acquire(&fake, "a.gguf".into(), ctx(Some(16384)))
            .await
            .unwrap();
        assert_eq!((first.port(), second.port()), (4242, 4242));
        assert_eq!((fake.starts(), fake.stops()), (0, 0));
    }

    #[tokio::test]
    async fn waits_for_a_start_in_progress_then_reuses_it() {
        let (guards, fake) = setup();
        let hold = fake.gate.lock().await;
        let a = spawn_acquire(&guards, &fake, "a.gguf");
        until(|| fake.starts() == 1).await;
        let b = spawn_acquire(&guards, &fake, "a.gguf");
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(!b.is_finished(), "前一个还没起完，后来的要等");

        drop(hold);
        let (a, b) = (a.await.unwrap().unwrap(), b.await.unwrap().unwrap());
        assert_eq!(a.port(), b.port());
        assert_eq!(fake.starts(), 1);
    }

    #[tokio::test]
    async fn restarts_when_the_engine_does_not_fit_and_nobody_holds_it() {
        let (guards, fake) = setup();
        fake.run("a.gguf", ctx(None));

        // 没设上下文就是 8K，要 8K 直接用
        let g = guards
            .acquire(&fake, "a.gguf".into(), ctx(Some(8192)))
            .await
            .unwrap();
        assert_eq!((fake.stops(), fake.starts()), (0, 0));
        drop(g);

        // 上下文不够
        let g = guards
            .acquire(&fake, "a.gguf".into(), ctx(Some(32768)))
            .await
            .unwrap();
        assert_eq!((fake.stops(), fake.starts()), (1, 1));
        drop(g);

        // 模型不同
        guards
            .acquire(&fake, "b.gguf".into(), ctx(Some(32768)))
            .await
            .unwrap();
        assert_eq!((fake.stops(), fake.starts()), (2, 2));
        assert_eq!(fake.loaded_model(), Some("b.gguf".into()));
    }

    #[tokio::test]
    async fn reports_busy_and_leaves_the_engine_alone_while_someone_holds_it() {
        let (guards, fake) = setup();
        fake.run("a.gguf", ctx(None));
        let held = guards
            .acquire(&fake, "a.gguf".into(), ctx(None))
            .await
            .unwrap();

        let err = guards
            .acquire(&fake, "b.gguf".into(), ctx(None))
            .await
            .err()
            .expect("别人拿着，要报忙");
        assert!(err.to_string().contains("[AI_RUN_BUSY]"), "{err}");
        assert_eq!((fake.stops(), fake.starts()), (0, 0));
        assert_eq!(fake.loaded_model(), Some("a.gguf".into()));

        // 还回来以后就能重启
        drop(held);
        guards
            .acquire(&fake, "b.gguf".into(), ctx(None))
            .await
            .unwrap();
        assert_eq!(fake.loaded_model(), Some("b.gguf".into()));
    }

    #[tokio::test]
    async fn gives_no_guard_when_the_start_fails() {
        let (guards, fake) = setup();
        fake.fail.store(true, SeqCst);
        assert!(guards
            .acquire(&fake, "a.gguf".into(), ctx(None))
            .await
            .is_err());
        assert_eq!(guards.count(), 0);
    }

    #[tokio::test]
    async fn stops_its_own_engine_when_cancelled_while_starting() {
        let (guards, fake) = setup();
        let _hold = fake.gate.lock().await;
        let a = spawn_acquire(&guards, &fake, "a.gguf");
        until(|| fake.starts() == 1).await;

        a.abort();
        let _ = a.await;
        until(|| fake.stops() == 1).await;
        assert_eq!(guards.count(), 0);
    }

    #[tokio::test]
    async fn leaves_the_engine_alone_when_cancelled_while_waiting() {
        let (guards, fake) = setup();
        let hold = fake.gate.lock().await;
        let a = spawn_acquire(&guards, &fake, "a.gguf");
        until(|| fake.starts() == 1).await;
        let b = spawn_acquire(&guards, &fake, "b.gguf");
        tokio::task::yield_now().await;
        b.abort();
        let _ = b.await;

        drop(hold);
        a.await.unwrap().unwrap();
        assert_eq!(fake.stops(), 0);
        assert_eq!(fake.loaded_model(), Some("a.gguf".into()));
    }

    /// 用真的 supervisor：引擎在跑、空闲超时，有 guard 没还时也不关。
    #[tokio::test]
    async fn idle_stop_waits_until_every_guard_is_returned() {
        let sup = Arc::new(EngineSupervisor::new());
        {
            let mut inner = sup.inner.lock().await;
            inner.state.state = EngineState::Running;
            inner.state.port = Some(4242);
        }
        *sup.loaded_main.lock().unwrap() = Some("a.gguf".into());
        let g = sup.acquire("a.gguf".into(), ctx(None)).await.unwrap();
        assert_eq!(g.port(), 4242);

        let idle_too_long = || {
            sup.guards.holders.lock().unwrap().last_used_at =
                Instant::now() - IDLE_THRESHOLD - Duration::from_secs(1);
        };
        idle_too_long();
        sup.maybe_idle_stop().await;
        assert_eq!(sup.status().await.state, EngineState::Running);

        drop(g);
        idle_too_long();
        sup.maybe_idle_stop().await;
        assert_eq!(sup.status().await.state, EngineState::Stopped);
    }

    /// 测试连接测完用它关引擎：别的任务还拿着 guard 时不关。
    #[tokio::test]
    async fn stop_if_unused_keeps_an_engine_someone_holds() {
        let (guards, fake) = setup();
        fake.run("a.gguf", ctx(None));
        let g = guards
            .acquire(&fake, "a.gguf".into(), ctx(None))
            .await
            .unwrap();

        guards.stop_if_unused(&fake).await.unwrap();
        assert_eq!(fake.stops(), 0);

        drop(g);
        guards.stop_if_unused(&fake).await.unwrap();
        assert_eq!(fake.stops(), 1);
    }

    /// 别的任务正在启动引擎时，不会把它关掉。
    #[tokio::test]
    async fn stop_if_unused_waits_for_a_start_in_progress() {
        let (guards, fake) = setup();
        let hold = fake.gate.lock().await;
        let a = spawn_acquire(&guards, &fake, "a.gguf");
        until(|| fake.starts() == 1).await;
        let stopper = {
            let (guards, fake) = (Arc::clone(&guards), Arc::clone(&fake));
            tokio::spawn(async move { guards.stop_if_unused(&fake).await })
        };

        drop(hold);
        let _a = a.await.unwrap().unwrap();
        stopper.await.unwrap().unwrap();
        assert_eq!(fake.stops(), 0);
    }
}
