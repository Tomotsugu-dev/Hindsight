//! 空闲回收：所有 guard 都还回来、又闲了一阵，就关掉引擎释放显存。

use std::sync::Arc;
use std::time::Duration;

use super::{wait_child_with_timeout, EngineRuntimeStatus, EngineState, EngineSupervisor};

/// 引擎闲置多久没新请求 → 自动 stop 释放显存。
/// 跑一段 step1+step2 大约 60-120s；阈值取 120s 让用户在跑总结之间稍稍歇会儿
/// 也不会被释放，但跑完真闲下来 2 分钟就回收 GPU。
pub(super) const IDLE_THRESHOLD: Duration = Duration::from_secs(120);
/// idle watcher 检查频率。
const IDLE_TICK: Duration = Duration::from_secs(10);

impl EngineSupervisor {
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
        let s = self.guards.holders.lock().ok()?;
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
    pub(super) async fn maybe_idle_stop(&self) -> bool {
        let child = {
            let mut inner = self.inner.lock().await;
            if inner.state.state != EngineState::Running {
                return false;
            }
            // inflight 锁的临界区极短（usize 比较 + Instant.elapsed），不会因为
            // 持着 inner 锁阻塞而拉长——nested lock 顺序：inner → inflight，
            // 全程不反向，无死锁风险
            let s = match self.guards.holders.lock() {
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
}
