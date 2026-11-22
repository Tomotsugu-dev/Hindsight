//! Lease acquisition, reuse, and concurrency coordination for the local engine.
//!
//! Callers borrow the local engine through [`EngineSupervisor::acquire`]:
//! - Not running: starts the engine with the requested parameters.
//! - Running and compatible (same model and per-slot context size >= requested):
//!   reuses it and shares the port.
//! - Running, incompatible, and unused: stops it and restarts with the requested parameters.
//! - Running, incompatible, and held by another caller: returns [`Error::EngineBusy`].
//!
//! Acquisition decisions are serialized so later callers wait for an in-progress
//! start or restart before deciding.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::OwnedMutexGuard;

use super::server::LoadedEngine;
use super::{EngineSlot, EngineStartOverrides, EngineSupervisor, HolderState, DEFAULT_CTX_SIZE};
use crate::error::{Error, Result};

/// RAII lease on the local engine.
///
/// While at least one guard is held, idle shutdown leaves the engine running.
/// Dropping the guard decrements the active lease count and refreshes the last-used timestamp.
pub struct EngineGuard {
    port: u16,
    holders: Arc<Mutex<HolderState>>,
}

impl EngineGuard {
    fn new(holders: &Arc<Mutex<HolderState>>) -> Self {
        if let Ok(mut state) = holders.lock() {
            state.active_count += 1;
            state.last_used_at = Instant::now();
        }
        Self {
            port: 0,
            holders: Arc::clone(holders),
        }
    }

    /// Returns the local TCP port the running `llama-server` listens on.
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for EngineGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = self.holders.lock() {
            state.active_count = state.active_count.saturating_sub(1);
            state.last_used_at = Instant::now();
        }
    }
}

impl EngineSupervisor {
    /// Acquires a lease on a local engine serving `model` with at least the requested `overrides`.
    ///
    /// Reuse, restart, and queueing rules are documented at the module level.
    ///
    /// # Errors
    ///
    /// - Returns [`Error::EngineBusy`] if another caller holds a lease on an incompatible engine.
    /// - Propagates startup errors from starting or restarting the engine process.
    pub async fn acquire(
        self: &Arc<Self>,
        model: PathBuf,
        overrides: EngineStartOverrides,
    ) -> Result<EngineGuard> {
        self.guards.acquire(self, model, overrides).await
    }

    /// Stops the local engine if no [`EngineGuard`] is currently held.
    pub async fn stop_if_unused(self: &Arc<Self>) -> Result<()> {
        self.guards.stop_if_unused(self).await
    }
}

// Engine process operations needed by lease coordination; mocked in unit tests.
pub(super) trait EngineProcess: Send + Sync + 'static {
    fn running(&self) -> impl Future<Output = Option<LoadedEngine>> + Send;
    fn start(
        &self,
        model: PathBuf,
        overrides: EngineStartOverrides,
    ) -> impl Future<Output = Result<u16>> + Send;
    fn stop(&self) -> impl Future<Output = Result<()>> + Send;
}

impl EngineProcess for EngineSupervisor {
    async fn running(&self) -> Option<LoadedEngine> {
        match &self.state.lock().await.slot {
            EngineSlot::Running(server) => Some(server.loaded.clone()),
            _ => None,
        }
    }

    async fn start(&self, model: PathBuf, overrides: EngineStartOverrides) -> Result<u16> {
        EngineSupervisor::start(self, model, overrides).await
    }

    async fn stop(&self) -> Result<()> {
        EngineSupervisor::stop(self).await
    }
}

#[derive(Default)]
pub(super) struct Guards {
    pub(super) holders: Arc<Mutex<HolderState>>,
    // Serializes `acquire` and `stop_if_unused` decisions.
    acquire_lock: Arc<tokio::sync::Mutex<()>>,
}

impl Guards {
    fn active_count(&self) -> usize {
        self.holders
            .lock()
            .map(|state| state.active_count)
            .unwrap_or(0)
    }

    async fn acquire<P: EngineProcess>(
        &self,
        process: &Arc<P>,
        model: PathBuf,
        overrides: EngineStartOverrides,
    ) -> Result<EngineGuard> {
        let acquire_guard = Arc::clone(&self.acquire_lock).lock_owned().await;
        // Reserve a lease up front so idle shutdown cannot stop the engine between check and return.
        let mut guard = EngineGuard::new(&self.holders);
        let other_holders = self.active_count().saturating_sub(1);
        match process.running().await {
            Some(loaded) if can_reuse(&loaded, &model, &overrides) => guard.port = loaded.port,
            Some(_) if other_holders > 0 => return Err(Error::EngineBusy),
            running => {
                // Hand `acquire_guard` to the cancel guard: if this future is dropped mid-start
                // before `disarm()`, `Drop` stops the half-started engine first and only releases
                // `acquire_lock` afterward; normal completion releases it immediately on `disarm()`.
                let cancel_guard = StartupCancelGuard {
                    process: Arc::clone(process),
                    acquire_guard: Some(acquire_guard),
                };
                let started = async {
                    if running.is_some() {
                        process.stop().await?;
                    }
                    process.start(model, overrides).await
                }
                .await;
                cancel_guard.disarm();
                guard.port = started?;
            }
        }
        Ok(guard)
    }

    async fn stop_if_unused<P: EngineProcess>(&self, process: &Arc<P>) -> Result<()> {
        // Queue behind `acquire` so a new caller cannot acquire a lease after the zero check.
        let _acquire_guard = self.acquire_lock.lock().await;
        if self.active_count() == 0 {
            process.stop().await?;
        }
        Ok(())
    }
}

// Ignore batch size and parallel slots so concurrent tasks with the same model and sufficient
// per-slot context can share the running engine instead of failing with `EngineBusy`.
fn can_reuse(loaded: &LoadedEngine, model: &Path, requested: &EngineStartOverrides) -> bool {
    let effective_ctx =
        |overrides: &EngineStartOverrides| overrides.ctx_size.unwrap_or(DEFAULT_CTX_SIZE);
    loaded.model == model && effective_ctx(&loaded.params) >= effective_ctx(requested)
}

struct StartupCancelGuard<P: EngineProcess> {
    process: Arc<P>,
    acquire_guard: Option<OwnedMutexGuard<()>>,
}

impl<P: EngineProcess> StartupCancelGuard<P> {
    fn disarm(mut self) {
        self.acquire_guard = None;
    }
}

impl<P: EngineProcess> Drop for StartupCancelGuard<P> {
    fn drop(&mut self) {
        let Some(acquire_guard) = self.acquire_guard.take() else {
            return;
        };
        let process = Arc::clone(&self.process);
        // `Drop` is synchronous and cannot block the runtime, so stop asynchronously and release
        // `acquire_lock` only after the half-started process has exited.
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = process.stop().await;
                drop(acquire_guard);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
    use std::time::Duration;

    use super::*;

    // In-memory fake engine that tracks start/stop counts and pauses startup via `start_gate`.
    #[derive(Default)]
    struct FakeProcess {
        running: Mutex<Option<LoadedEngine>>,
        starts: AtomicUsize,
        stops: AtomicUsize,
        fail_start: AtomicBool,
        start_gate: tokio::sync::Mutex<()>,
    }

    impl EngineProcess for FakeProcess {
        async fn running(&self) -> Option<LoadedEngine> {
            self.running.lock().unwrap().clone()
        }

        async fn start(&self, model: PathBuf, overrides: EngineStartOverrides) -> Result<u16> {
            let seq = self.starts.fetch_add(1, SeqCst) + 1;
            let _gate = self.start_gate.lock().await;
            if self.fail_start.load(SeqCst) {
                return Err(Error::EngineStart("fake engine failed to start".into()));
            }
            let port = 5000 + seq as u16;
            *self.running.lock().unwrap() = Some(LoadedEngine {
                port,
                model,
                params: overrides,
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
                .map(|loaded| loaded.model.clone())
        }

        fn set_running(&self, model: &str, params: EngineStartOverrides) {
            *self.running.lock().unwrap() = Some(LoadedEngine {
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

    fn with_ctx(ctx_size: Option<u32>) -> EngineStartOverrides {
        EngineStartOverrides {
            ctx_size,
            ..Default::default()
        }
    }

    fn spawn_acquire(
        guards: &Arc<Guards>,
        fake: &Arc<FakeProcess>,
        model: &str,
    ) -> tokio::task::JoinHandle<Result<EngineGuard>> {
        let (guards, fake, model) = (Arc::clone(guards), Arc::clone(fake), PathBuf::from(model));
        tokio::spawn(async move { guards.acquire(&fake, model, with_ctx(None)).await })
    }

    async fn wait_until(done: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !done() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("timed out after 5s waiting for condition");
    }

    #[tokio::test]
    async fn starts_the_engine_when_none_is_running() {
        let (guards, fake) = setup();
        let guard = guards
            .acquire(&fake, "a.gguf".into(), with_ctx(None))
            .await
            .unwrap();
        assert_eq!(fake.starts(), 1);
        assert_eq!(guard.port(), 5001);
    }

    // Differences in batch size and parallel slots are ignored, and multiple guards can share.
    #[tokio::test]
    async fn reuses_an_engine_with_the_same_model_and_enough_context() {
        let (guards, fake) = setup();
        fake.set_running(
            "a.gguf",
            EngineStartOverrides {
                ctx_size: Some(16384),
                batch_size: Some(2048),
                parallel_slots: Some(2),
            },
        );
        // Unspecified ctx defaults to 8K, which fits within the loaded 16K.
        let first = guards
            .acquire(&fake, "a.gguf".into(), with_ctx(None))
            .await
            .unwrap();
        let second = guards
            .acquire(&fake, "a.gguf".into(), with_ctx(Some(16384)))
            .await
            .unwrap();
        assert_eq!((first.port(), second.port()), (4242, 4242));
        assert_eq!((fake.starts(), fake.stops()), (0, 0));
    }

    #[tokio::test]
    async fn waits_for_a_start_in_progress_then_reuses_it() {
        let (guards, fake) = setup();
        let gate = fake.start_gate.lock().await;
        let first = spawn_acquire(&guards, &fake, "a.gguf");
        wait_until(|| fake.starts() == 1).await;
        let second = spawn_acquire(&guards, &fake, "a.gguf");
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(
            !second.is_finished(),
            "second caller must wait while the first start is still in progress"
        );

        drop(gate);
        let (first, second) = (
            first.await.unwrap().unwrap(),
            second.await.unwrap().unwrap(),
        );
        assert_eq!(first.port(), second.port());
        assert_eq!(fake.starts(), 1);
    }

    #[tokio::test]
    async fn restarts_when_the_engine_does_not_fit_and_nobody_holds_it() {
        let (guards, fake) = setup();
        fake.set_running("a.gguf", with_ctx(None));

        // Unspecified ctx equals 8K, so requesting 8K reuses directly.
        let guard = guards
            .acquire(&fake, "a.gguf".into(), with_ctx(Some(8192)))
            .await
            .unwrap();
        assert_eq!((fake.stops(), fake.starts()), (0, 0));
        drop(guard);

        // Insufficient context restarts once nobody holds a guard.
        let guard = guards
            .acquire(&fake, "a.gguf".into(), with_ctx(Some(32768)))
            .await
            .unwrap();
        assert_eq!((fake.stops(), fake.starts()), (1, 1));
        drop(guard);

        // Different model restarts once nobody holds a guard.
        guards
            .acquire(&fake, "b.gguf".into(), with_ctx(Some(32768)))
            .await
            .unwrap();
        assert_eq!((fake.stops(), fake.starts()), (2, 2));
        assert_eq!(fake.loaded_model(), Some("b.gguf".into()));
    }

    #[tokio::test]
    async fn reports_busy_and_leaves_the_engine_alone_while_someone_holds_it() {
        let (guards, fake) = setup();
        fake.set_running("a.gguf", with_ctx(None));
        let held = guards
            .acquire(&fake, "a.gguf".into(), with_ctx(None))
            .await
            .unwrap();

        let err = guards
            .acquire(&fake, "b.gguf".into(), with_ctx(None))
            .await
            .err()
            .expect("should report busy while another caller holds the engine");
        assert!(err.to_string().contains("[AI_RUN_BUSY]"), "{err}");
        assert_eq!((fake.stops(), fake.starts()), (0, 0));
        assert_eq!(fake.loaded_model(), Some("a.gguf".into()));

        // Restart succeeds once the existing guard is dropped.
        drop(held);
        guards
            .acquire(&fake, "b.gguf".into(), with_ctx(None))
            .await
            .unwrap();
        assert_eq!(fake.loaded_model(), Some("b.gguf".into()));
    }

    #[tokio::test]
    async fn gives_no_guard_when_the_start_fails() {
        let (guards, fake) = setup();
        fake.fail_start.store(true, SeqCst);
        assert!(guards
            .acquire(&fake, "a.gguf".into(), with_ctx(None))
            .await
            .is_err());
        assert_eq!(guards.active_count(), 0);
    }

    #[tokio::test]
    async fn stops_its_own_engine_when_cancelled_while_starting() {
        let (guards, fake) = setup();
        let _gate = fake.start_gate.lock().await;
        let first = spawn_acquire(&guards, &fake, "a.gguf");
        wait_until(|| fake.starts() == 1).await;

        first.abort();
        let _ = first.await;
        wait_until(|| fake.stops() == 1).await;
        assert_eq!(guards.active_count(), 0);
    }

    #[tokio::test]
    async fn leaves_the_engine_alone_when_cancelled_while_waiting() {
        let (guards, fake) = setup();
        let gate = fake.start_gate.lock().await;
        let first = spawn_acquire(&guards, &fake, "a.gguf");
        wait_until(|| fake.starts() == 1).await;
        let second = spawn_acquire(&guards, &fake, "b.gguf");
        tokio::task::yield_now().await;
        second.abort();
        let _ = second.await;

        drop(gate);
        first.await.unwrap().unwrap();
        assert_eq!(fake.stops(), 0);
        assert_eq!(fake.loaded_model(), Some("a.gguf".into()));
    }

    #[tokio::test]
    async fn stop_if_unused_keeps_an_engine_someone_holds() {
        let (guards, fake) = setup();
        fake.set_running("a.gguf", with_ctx(None));
        let guard = guards
            .acquire(&fake, "a.gguf".into(), with_ctx(None))
            .await
            .unwrap();

        guards.stop_if_unused(&fake).await.unwrap();
        assert_eq!(fake.stops(), 0);

        drop(guard);
        guards.stop_if_unused(&fake).await.unwrap();
        assert_eq!(fake.stops(), 1);
    }

    // `stop_if_unused` queues behind an in-progress start and does not kill its new lease.
    #[tokio::test]
    async fn stop_if_unused_waits_for_a_start_in_progress() {
        let (guards, fake) = setup();
        let gate = fake.start_gate.lock().await;
        let first = spawn_acquire(&guards, &fake, "a.gguf");
        wait_until(|| fake.starts() == 1).await;
        let stopper = {
            let (guards, fake) = (Arc::clone(&guards), Arc::clone(&fake));
            tokio::spawn(async move { guards.stop_if_unused(&fake).await })
        };

        drop(gate);
        let _guard = first.await.unwrap().unwrap();
        stopper.await.unwrap().unwrap();
        assert_eq!(fake.stops(), 0);
    }
}
