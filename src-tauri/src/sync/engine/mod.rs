//! 同步引擎：登录后台跑两件事，间隔由后端定（Drive：push 每 30 秒、pull 每 60 秒）
//!   - push：把 sync_outbox 翻成"哪些文件脏了"，对每个脏文件全量重写到云端
//!   - pull：列云端上其他设备的文件，按修改时间增量下载并 LWW merge 到本地
//!
//! 失败走指数退避（最多 1 小时），attempts > 10 留在 outbox 作为 dead-letter，UI 可以看见。

mod datasets;
mod io;
mod pull;
mod push;

#[cfg(test)]
mod e2e_tests;

pub(crate) use pull::rewind_cursor;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::error::{Error, Result};
use crate::storage::DbPool;
use crate::sync::cloud::{CloudBackend, FailureKind};

/// Prefix of `last_error` when the user has to sign in again. The Devices page
/// matches these prefixes as written.
pub(super) const ERR_PREFIX_CRED_EXPIRED: &str = "[CRED_EXPIRED] ";
/// Prefix of `last_error` when the cloud is out of space.
pub(super) const ERR_PREFIX_OUT_OF_SPACE: &str = "[OUT_OF_SPACE] ";
/// Prefix of `last_error` when the cloud account has expired.
pub(super) const ERR_PREFIX_ACCOUNT_EXPIRED: &str = "[ACCOUNT_EXPIRED] ";
/// Prefix of `last_error` when the next round will retry on its own.
pub(super) const ERR_PREFIX_TRANSIENT: &str = "[TRANSIENT] ";

/// The prefix the Devices page matches for each kind of failure. The backend
/// decides the kind; see [`CloudBackend::failure_kind`].
fn sync_error_prefix(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::CredentialInvalid => ERR_PREFIX_CRED_EXPIRED,
        FailureKind::OutOfSpace => ERR_PREFIX_OUT_OF_SPACE,
        FailureKind::AccountExpired => ERR_PREFIX_ACCOUNT_EXPIRED,
        FailureKind::Transient => ERR_PREFIX_TRANSIENT,
    }
}

/// The prefix followed by the full error text, for `status.last_error`. The
/// frontend reads the prefix to decide whether to offer signing in again.
fn format_sync_error(kind: FailureKind, e: &Error) -> String {
    format!("{}{e}", sync_error_prefix(kind))
}

/// Records a failed push or pull round for the Devices page. The log gets the
/// error's class at warn and its text only at debug: only info and above is
/// printed by default, and the text can carry a Google response. Start with
/// RUST_LOG=hindsight=debug to see it.
async fn record_round_failure(inner: &Inner, round: &str, e: &Error) {
    let kind = inner.cloud().failure_kind(e);
    log::warn!(
        "sync {round} failed {}(see status)",
        sync_error_prefix(kind)
    );
    log::debug!("sync {round}: {e}");
    inner.status.write().await.last_error = Some(format_sync_error(kind, e));
}

/// What the Devices page shows about sync. Returned by `sync_status`.
#[derive(Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    /// Whether the background sync loop is running.
    pub running: bool,
    /// Whether a sync round is running right now. The Devices page restores its
    /// "Syncing…" button state from this after a page switch, since component
    /// state is lost on unmount.
    pub sync_in_flight: bool,
    /// When a push last uploaded something, RFC3339 UTC.
    pub last_pushed_at: Option<String>,
    /// The latest failure, prefixed as `ERR_PREFIX_CRED_EXPIRED` and
    /// `ERR_PREFIX_TRANSIENT` describe. Cleared once a push uploads again.
    pub last_error: Option<String>,
    /// Outbox rows waiting to be pushed, dead letters included.
    pub pending: u64,
    /// Outbox rows that failed `MAX_ATTEMPTS` times. Push no longer retries them;
    /// the Devices page counts them as failed.
    pub dead_letter: u64,
}

/// The sync engine's state: the two databases, the cloud backend, this device's
/// `device_id`, the background loop's task, the status the Devices page reads,
/// and the lock and flag that keep two sync rounds from overlapping.
///
/// The background loop runs in its own `tokio::spawn`ed task and keeps its own
/// handle to this state; commands reach the same state through `SyncEngine`.
/// Hence the `Arc`.
pub(super) struct Inner {
    /// The main database: push reads the outbox and the tables it builds files
    /// from; pull merges downloaded rows back into it.
    pub(super) pool: DbPool,
    /// The memory database, which holds the two optional datasets: chat history
    /// and screen text. `None` when it failed to open at startup; those two
    /// datasets then never sync, whatever the settings say.
    pub(super) mem: Option<crate::memory::MemoryDb>,
    /// The cloud backend sync uses: Google Drive or WebDAV. Replacing it takes
    /// `flush_gate`, which a push or pull round holds from start to end, so a
    /// round uses one backend throughout.
    pub(super) cloud: std::sync::Mutex<Arc<CloudBackend>>,
    /// This device's `device_id`, a UUID. Every file this device uploads is named
    /// `device.<self_id>.…`, which is how pull tells other devices' files apart.
    pub(super) self_id: String,
    /// The background sync loop while it runs; `None` when it is stopped.
    pub(super) handle: Mutex<Option<JoinHandle<()>>>,
    /// What the Devices page shows: last push and pull times, the latest error,
    /// rows waiting to be pushed, and dead letters.
    pub(super) status: RwLock<SyncStatus>,
    /// Lets only one push or pull pass, or one cloud cleanup (which takes it through
    /// [`SyncEngine::pause_flushes`]), run at a time. Running them together loses
    /// data: of two concurrent pushes, the slower overwrites the cloud with older
    /// content after the faster has deleted the outbox rows for the newer changes,
    /// so those changes are never uploaded; a cleanup landing between a push's
    /// outbox read and its table read makes the push upload empty content.
    pub(super) flush_gate: Mutex<()>,
    /// Whether a sync round (push and pull) is running right now. The Devices page
    /// shows "Syncing…" from it, and "Sync now" refuses to start while it is set.
    /// It is only a flag; `flush_gate` does the mutual exclusion.
    pub(super) sync_in_flight: std::sync::atomic::AtomicBool,
    /// Set once this run has deleted, or found absent, this device's cloud copies
    /// of the files it no longer publishes. Push checks it once per launch.
    /// TODO(ADR-0003, ADR-0004): remove together with the cleanup in push.
    pub(super) legacy_cloud_files_checked: std::sync::atomic::AtomicBool,
}

impl Inner {
    pub(super) fn cloud(&self) -> Arc<CloudBackend> {
        Arc::clone(&self.cloud.lock().unwrap())
    }
}

/// RAII:作用域内置位 sync_in_flight,离开(含错误提前返回)自动清零。
struct InFlightGuard<'a>(&'a std::sync::atomic::AtomicBool);

impl<'a> InFlightGuard<'a> {
    fn set(flag: &'a std::sync::atomic::AtomicBool) -> Self {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        Self(flag)
    }
}

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// 同步引擎对外句柄。一个进程一份；`app.manage(Arc::new(SyncEngine::new))` 注册。
pub struct SyncEngine {
    inner: Arc<Inner>,
}

impl SyncEngine {
    /// Used at app startup: the backend is built from `auth_state`.
    pub async fn new(pool: DbPool, mem: Option<crate::memory::MemoryDb>) -> Self {
        let self_id = crate::device::self_id().unwrap_or("").to_string();
        let cloud = match CloudBackend::from_auth_state(pool.clone(), self_id.clone()).await {
            Ok(cloud) => cloud,
            Err(e) => {
                log::warn!("sync: the saved backend cannot be built, using Drive: {e}");
                CloudBackend::drive(pool.clone())
            }
        };
        Self::with_backend(pool, mem, cloud, self_id)
    }

    /// Test entry: takes the backend and the device id, so one process can run
    /// several independent devices against the in-memory cloud.
    pub fn with_backend(
        pool: DbPool,
        mem: Option<crate::memory::MemoryDb>,
        cloud: CloudBackend,
        self_id: String,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                pool,
                mem,
                cloud: std::sync::Mutex::new(Arc::new(cloud)),
                self_id,
                handle: Mutex::new(None),
                status: RwLock::new(SyncStatus::default()),
                sync_in_flight: std::sync::atomic::AtomicBool::new(false),
                flush_gate: Mutex::new(()),
                legacy_cloud_files_checked: std::sync::atomic::AtomicBool::new(false),
            }),
        }
    }

    /// 暂停 push/pull：等在途 flush 结束并挡住新的，直到返回的 guard 被 drop。
    /// purge_local_data / purge_cloud_data 这类"动表"的命令在整个清理期间持有它。
    pub async fn pause_flushes(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.inner.flush_gate.lock().await
    }

    /// 同步现在用的后端，给清云端的命令用。要在暂停同步以后取：先取再暂停的话，中间
    /// 换了后端，命令会删到旧后端上。
    pub fn cloud(&self) -> Arc<CloudBackend> {
        self.inner.cloud()
    }

    /// 换掉同步用的后端。换后端的事务提交后、恢复同步之前调用，这样下一轮就用新后端，
    /// 不用重启。
    pub fn replace_cloud(&self, cloud: CloudBackend) {
        *self.inner.cloud.lock().unwrap() = Arc::new(cloud);
    }

    /// 借出当前设备身份。给 command-layer 入口（`purge_cloud_data` 等）走，
    /// 测试场景 self_id 不等于 `device::self_id()`，必须从 engine 拿。
    pub fn self_id(&self) -> &str {
        &self.inner.self_id
    }

    /// 启动后台 push/pull 循环。已在跑时 no-op。未登录时循环内每次都 silently 跳过。
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

    /// 后台循环是否在跑（仅看 task handle，未登录时也算 running）。
    pub async fn is_running(&self) -> bool {
        self.inner.handle.lock().await.is_some()
    }

    /// 清掉缓存的 last_error。重新登录成功后调用，避免 UI 还显示旧的
    /// "登录凭证失效"错误，导致"退出"按钮一直停留在"重新登录"形态。
    pub async fn clear_last_error(&self) {
        self.inner.status.write().await.last_error = None;
    }

    /// 拉一份快照（含 outbox 行数实时查询）给前端 sync_status 命令用。
    pub async fn status(&self) -> SyncStatus {
        let mut s = self.inner.status.read().await.clone();
        s.running = self.is_running().await;
        s.sync_in_flight = self
            .inner
            .sync_in_flight
            .load(std::sync::atomic::Ordering::SeqCst);
        s.pending = io::count_outbox(&self.inner.pool).await.unwrap_or(0);
        s.dead_letter = io::count_dead_letter(&self.inner.pool).await.unwrap_or(0);
        s
    }

    /// UI "立即同步" 按钮：跑一次 push + pull，不等下个 tick。
    pub async fn sync_now(&self) -> Result<()> {
        // 已有一次在跑(手动或后台 tick)→ 明确拒绝而不是排队再跑一遍。
        // 前端按 syncInFlight 禁用按钮,正常点不到这里;真racing到了给人话。
        if self
            .inner
            .sync_in_flight
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(crate::error::Error::InvalidInput("同步已在进行中"));
        }
        let _in_flight = InFlightGuard::set(&self.inner.sync_in_flight);
        // Clear the previous error, or the UI keeps showing it after a success.
        self.inner.status.write().await.last_error = None;
        if let Err(e) = push::flush_push(&self.inner).await {
            record_round_failure(&self.inner, "push", &e).await;
            return Err(e);
        }
        if let Err(e) = pull::flush_pull(&self.inner).await {
            record_round_failure(&self.inner, "pull", &e).await;
            return Err(e);
        }
        Ok(())
    }
}

async fn run_loop(inner: Arc<Inner>) {
    let mut last_pull: Option<DateTime<Utc>> = None;
    loop {
        let _in_flight = InFlightGuard::set(&inner.sync_in_flight);
        if let Err(e) = push::flush_push(&inner).await {
            record_round_failure(&inner, "push", &e).await;
        }

        // 间隔每轮都问：两轮之间可能换了后端。
        let pull_every = chrono::Duration::from_std(inner.cloud().pull_interval())
            .expect("a backend's pull interval is minutes, not centuries");
        let now = Utc::now();
        let should_pull = match last_pull {
            None => true,
            Some(t) => now - t >= pull_every,
        };
        if should_pull {
            if let Err(e) = pull::flush_pull(&inner).await {
                record_round_failure(&inner, "pull", &e).await;
            }
            last_pull = Some(now);
        }
        drop(_in_flight); // sleep 期间不算"同步中"

        tokio::time::sleep(inner.cloud().push_interval()).await;
    }
}
