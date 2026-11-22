//! 引擎空闲回收：全部租约归还且持续空闲超过阈值后，自动停止 `llama-server` 释放显存。

use std::sync::Arc;
use std::time::Duration;

use super::{EngineSlot, EngineSupervisor};

/// 无活跃租约且持续空闲超过该时长后，自动停止引擎释放显存。
/// 连续使用报告/对话的间隔通常在几十秒内，取 120s 兼顾避免频繁冷启动与及时释放 GPU 资源。
pub(super) const IDLE_THRESHOLD: Duration = Duration::from_secs(120);
/// 后台空闲检查轮询间隔。
const IDLE_POLL_INTERVAL: Duration = Duration::from_secs(10);

impl EngineSupervisor {
    /// 启动后台空闲回收任务，每 [`IDLE_POLL_INTERVAL`] 检查一次是否达到 [`IDLE_THRESHOLD`]。
    ///
    /// 后台任务仅持有 [`Weak`](std::sync::Weak) 引用，当 `EngineSupervisor` 被销毁时自动退出，
    /// 不会泄漏任务。由 `lib.rs` 在应用启动时调用一次即可。
    pub fn spawn_idle_watcher(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(IDLE_POLL_INTERVAL);
            // `tokio::time::interval` 首次 `tick()` 立即返回，跳过以避免刚启动就检查。
            interval.tick().await;
            loop {
                interval.tick().await;
                let Some(supervisor) = weak.upgrade() else {
                    break;
                };
                let _ = supervisor.maybe_idle_stop().await;
            }
        })
    }

    /// 返回距离达到空闲回收阈值的剩余时间。
    ///
    /// 若当前仍有未归还的 [`EngineGuard`](super::EngineGuard) 返回 `None`。
    /// 注意：本方法仅根据租约状态计算，不校验进程是否处于 `Running`，调用方需自行结合状态使用。
    pub fn idle_eta(&self) -> Option<Duration> {
        let holders = self.guards.holders.lock().ok()?;
        if holders.active_count > 0 {
            return None;
        }
        let elapsed = holders.last_used_at.elapsed();
        if elapsed >= IDLE_THRESHOLD {
            Some(Duration::ZERO)
        } else {
            Some(IDLE_THRESHOLD - elapsed)
        }
    }

    /// 检查当前是否满足空闲回收条件，满足则停止运行中的引擎并返回 `true`。
    pub(super) async fn maybe_idle_stop(&self) -> bool {
        let server = {
            let mut state = self.state.lock().await;
            // 持有 `state` 锁期间再读 `holders`，严格遵守 `state → holders` 的锁顺序。
            if self.idle_eta() != Some(Duration::ZERO) {
                return false;
            }
            match std::mem::take(&mut state.slot) {
                EngineSlot::Running(server) => server,
                other => {
                    state.slot = other;
                    return false;
                }
            }
        };
        server.kill().await;
        log::info!(
            "AI 引擎 idle 超过 {}s，已自动释放显存",
            IDLE_THRESHOLD.as_secs()
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::ai::engine::server::{LlamaServer, LoadedEngine};
    use crate::ai::engine::{EngineStartOverrides, EngineState};

    // 引擎处于 Running 且空闲超时时：只要仍有租约未归还就不停机，全部归还后立即停机。
    #[cfg(unix)] // 使用 sleep 子进程模拟 llama-server
    #[tokio::test]
    async fn idle_stop_waits_until_every_guard_is_returned() {
        let supervisor = Arc::new(EngineSupervisor::new());
        let child = tokio::process::Command::new("sleep")
            .arg("60")
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        supervisor.state.lock().await.slot = EngineSlot::Running(LlamaServer {
            child,
            loaded: LoadedEngine {
                port: 4242,
                model: "a.gguf".into(),
                params: EngineStartOverrides::default(),
            },
        });
        let guard = supervisor
            .acquire("a.gguf".into(), EngineStartOverrides::default())
            .await
            .unwrap();
        assert_eq!(guard.port(), 4242);

        let expire_idle = || {
            supervisor.guards.holders.lock().unwrap().last_used_at =
                Instant::now() - IDLE_THRESHOLD - Duration::from_secs(1);
        };
        expire_idle();
        assert!(!supervisor.maybe_idle_stop().await);
        assert_eq!(supervisor.status().await.state, EngineState::Running);

        drop(guard);
        expire_idle();
        assert!(supervisor.maybe_idle_stop().await);
        assert_eq!(supervisor.status().await.state, EngineState::Stopped);
    }
}
