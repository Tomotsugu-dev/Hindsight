//! 同步引擎：登录后台跑两件事
//!   - push：每 30 秒把 sync_outbox 翻成"哪些文件脏了"，对每个脏文件全量重写到 Drive appDataFolder
//!   - pull：每 60 秒列 Drive 上其他设备的文件，按 modifiedTime 增量下载并 LWW merge 到本地
//!
//! 失败走指数退避（最多 1 小时），attempts > 10 留在 outbox 作为 dead-letter，UI 可以看见。

mod io;
mod pull;
mod push;

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::error::Result;
use crate::storage::DbPool;

const PUSH_INTERVAL_SECS: u64 = 30;
const PULL_INTERVAL_SECS: i64 = 60;

#[derive(Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub running: bool,
    pub last_pushed_at: Option<String>,
    pub last_pulled_at: Option<String>,
    pub last_error: Option<String>,
    pub pending: u64,
    pub dead_letter: u64,
}

/// 内部共享状态：池、后台任务句柄、状态。push / pull 模块都用 `&Arc<Inner>` 访问。
pub(super) struct Inner {
    pub(super) pool: DbPool,
    pub(super) handle: Mutex<Option<JoinHandle<()>>>,
    pub(super) status: RwLock<SyncStatus>,
}

pub struct SyncEngine {
    inner: Arc<Inner>,
}

impl SyncEngine {
    pub fn new(pool: DbPool) -> Self {
        Self {
            inner: Arc::new(Inner {
                pool,
                handle: Mutex::new(None),
                status: RwLock::new(SyncStatus::default()),
            }),
        }
    }

    pub async fn start(&self) {
        let mut h = self.inner.handle.lock().await;
        if h.is_some() {
            return;
        }
        let inner = Arc::clone(&self.inner);
        *h = Some(tokio::spawn(async move {
            run_loop(inner).await;
        }));
        log::info!("sync engine 已启动");
    }

    /// 停止后台 push/pull 循环。当前没有 UI 入口；保留给将来"sign_out 后停 engine"的场景。
    #[allow(dead_code)]
    pub async fn stop(&self) {
        let mut h = self.inner.handle.lock().await;
        if let Some(handle) = h.take() {
            handle.abort();
            log::info!("sync engine 已停止");
        }
    }

    pub async fn is_running(&self) -> bool {
        self.inner.handle.lock().await.is_some()
    }

    /// 清掉缓存的 last_error。重新登录成功后调用，避免 UI 还显示旧的
    /// "登录凭证失效"错误，导致"退出"按钮一直停留在"重新登录"形态。
    pub async fn clear_last_error(&self) {
        self.inner.status.write().await.last_error = None;
    }

    pub async fn status(&self) -> SyncStatus {
        let mut s = self.inner.status.read().await.clone();
        s.running = self.is_running().await;
        s.pending = io::count_outbox(&self.inner.pool).await.unwrap_or(0);
        s.dead_letter = io::count_dead_letter(&self.inner.pool).await.unwrap_or(0);
        s
    }

    /// UI "立即同步" 按钮：跑一次 push + pull，不等下个 30s tick。
    pub async fn sync_now(&self) -> Result<()> {
        // 清掉上次的错误，否则即使这次成功，UI 也会留着旧 last_error
        self.inner.status.write().await.last_error = None;
        push::flush_push(&self.inner).await?;
        pull::flush_pull(&self.inner).await?;
        // push/pull 内部如果 token 拿不到会写 last_error 但 return Ok；这里统一暴露给 UI
        let last_err = self.inner.status.read().await.last_error.clone();
        if let Some(e) = last_err {
            return Err(crate::error::Error::SyncIncomplete(e));
        }
        Ok(())
    }
}

async fn run_loop(inner: Arc<Inner>) {
    let mut last_pull: Option<DateTime<Utc>> = None;
    loop {
        if let Err(e) = push::flush_push(&inner).await {
            log::warn!("sync push 失败: {e}");
            inner.status.write().await.last_error = Some(e.to_string());
        }

        let now = Utc::now();
        let should_pull = match last_pull {
            None => true,
            Some(t) => (now - t).num_seconds() >= PULL_INTERVAL_SECS,
        };
        if should_pull {
            if let Err(e) = pull::flush_pull(&inner).await {
                log::warn!("sync pull 失败: {e}");
                inner.status.write().await.last_error = Some(e.to_string());
            }
            last_pull = Some(now);
        }

        tokio::time::sleep(Duration::from_secs(PUSH_INTERVAL_SECS)).await;
    }
}
