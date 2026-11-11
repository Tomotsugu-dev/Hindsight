//! Sync engine: once signed in, runs push and pull in the background on a
//! timer. The backend sets the intervals.
//!
//! - push: rewrites the files this device changed in the cloud, each one whole.
//! - pull: merges the files other devices wrote to the cloud into the local
//!   database.
//!
//! Push and pull both ask [`enabled_datasets`] which datasets sync this round.

mod io;
mod pull;
mod push;

#[cfg(test)]
mod e2e_tests;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::error::{Error, Result};
use crate::repo::settings::Settings;
use crate::storage::DbPool;
use crate::sync::cloud::{CloudBackend, FailureKind};
use crate::sync::file_name::Dataset;

/// Prefix of `last_error` when the user has to sign in again. The Devices page
/// matches these prefixes as written.
pub(super) const ERR_PREFIX_CRED_EXPIRED: &str = "[CRED_EXPIRED] ";
/// Prefix of `last_error` when the cloud is out of space.
pub(super) const ERR_PREFIX_OUT_OF_SPACE: &str = "[OUT_OF_SPACE] ";
/// Prefix of `last_error` when the cloud account has expired.
pub(super) const ERR_PREFIX_ACCOUNT_EXPIRED: &str = "[ACCOUNT_EXPIRED] ";
/// Prefix of `last_error` when the cloud is rate-limiting or briefly
/// unavailable. Nutstore answers 503 when it rate-limits.
pub(super) const ERR_PREFIX_SERVER_BUSY: &str = "[SERVER_BUSY] ";
/// Prefix of `last_error` when the next round will retry on its own.
pub(super) const ERR_PREFIX_TRANSIENT: &str = "[TRANSIENT] ";

/// The prefix the Devices page matches for each kind of failure. The backend
/// decides the kind; see [`CloudBackend::failure_kind`].
fn sync_error_prefix(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::CredentialInvalid => ERR_PREFIX_CRED_EXPIRED,
        FailureKind::OutOfSpace => ERR_PREFIX_OUT_OF_SPACE,
        FailureKind::AccountExpired => ERR_PREFIX_ACCOUNT_EXPIRED,
        FailureKind::ServerBusy => ERR_PREFIX_SERVER_BUSY,
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
    /// Outbox rows waiting to be pushed. Rows counted in `dead_letter` are not
    /// included.
    pub pending: u64,
    /// Outbox rows that have failed `FAILED_AFTER_ATTEMPTS` times or more. Push
    /// keeps retrying them; the Devices page counts them as failed.
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
    /// The cloud backend sync uses: Google Drive or WebDAV. Only
    /// [`Paused::replace_cloud`] replaces it, and a push or pull round holds
    /// `flush_gate` from start to end, so a round uses one backend throughout.
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

/// Sets `sync_in_flight` while it lives and clears it when dropped, early
/// returns included.
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

/// The sync engine's public handle. One instance per process, registered with
/// `app.manage(Arc::new(SyncEngine::new))`.
pub struct SyncEngine {
    inner: Arc<Inner>,
}

impl SyncEngine {
    /// Used at app startup: the backend is built from `auth_state`.
    pub async fn new(pool: DbPool, mem: Option<crate::memory::MemoryDb>) -> Self {
        let self_id = crate::device::self_id().unwrap_or("").to_string();
        let cloud = CloudBackend::from_auth_state(pool.clone(), self_id.clone()).await;
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

    /// Pauses sync: waits for the running push or pull to finish, and holds back
    /// new ones until the returned guard is dropped. A command that changes the
    /// tables or replaces the backend holds it from start to end; see
    /// `Inner::flush_gate` for why.
    pub async fn pause_flushes(&self) -> Paused<'_> {
        let gate = self.inner.flush_gate.lock().await;
        Paused {
            inner: &self.inner,
            cloud: self.inner.cloud(),
            _gate: gate,
        }
    }

    /// For tests: returns the backend without pausing sync, for example to read
    /// a cursor name.
    #[cfg(test)]
    pub(crate) fn cloud(&self) -> Arc<CloudBackend> {
        self.inner.cloud()
    }

    /// Returns this device's `device_id`.
    pub fn self_id(&self) -> &str {
        &self.inner.self_id
    }

    /// Starts the background push/pull loop.
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

    /// Stops the background push/pull loop.
    #[allow(dead_code)]
    pub async fn stop(&self) {
        let mut h = self.inner.handle.lock().await;
        if let Some(handle) = h.take() {
            handle.abort();
            log::info!("sync engine 已停止");
        }
    }

    /// Returns whether the background loop has started. Without a sign-in it
    /// has started too: the loop keeps running and skips every round.
    pub async fn is_running(&self) -> bool {
        self.inner.handle.lock().await.is_some()
    }

    /// Clears the last error the Devices page shows. Call it after a new sign-in:
    /// otherwise it stays until a push uploads something, and all that time the
    /// Devices page asks to sign in again.
    pub async fn clear_last_error(&self) {
        self.inner.status.write().await.last_error = None;
    }

    /// Returns the sync status the Devices page shows.
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

    /// Runs a round of push and pull now, without waiting for the background loop.
    ///
    /// # Errors
    ///
    /// A round is already running: [`Error::InvalidInput`]
    pub async fn sync_now(&self) -> Result<()> {
        // A round is already running: refuse, do not queue. That round is already
        // syncing, so another one right after it is no use. The Devices page
        // disables the button during a sync; this only stops two starts at the
        // same moment.
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

/// A pause in sync, which ends when this is dropped. Only this hands out and
/// replaces the backend, so a command that holds it uses the backend sync uses,
/// and the backend does not change halfway through.
///
/// Push, pull and `Paused` take turns holding the same lock:
///
/// ```text
/// push    hold → upload → release
/// Paused                          hold → replace backend → release
/// ```
///
/// While holding it, do not sync or pause again: the lock is not reentrant, so
/// it would wait forever.
pub struct Paused<'a> {
    inner: &'a Inner,
    /// The backend when sync was paused. It is kept here so that
    /// [`Self::cloud`] can lend a reference, which cannot be used after `Paused`
    /// is dropped.
    cloud: Arc<CloudBackend>,
    _gate: tokio::sync::MutexGuard<'a, ()>,
}

impl Paused<'_> {
    /// Returns the backend sync uses.
    pub fn cloud(&self) -> &CloudBackend {
        &self.cloud
    }

    /// Replaces the backend sync uses. Call it after the backend switch commits.
    pub fn replace_cloud(&mut self, cloud: CloudBackend) {
        let cloud = Arc::new(cloud);
        *self.inner.cloud.lock().unwrap() = Arc::clone(&cloud);
        self.cloud = cloud;
    }
}

async fn run_loop(inner: Arc<Inner>) {
    let mut last_pull: Option<DateTime<Utc>> = None;
    loop {
        let _in_flight = InFlightGuard::set(&inner.sync_in_flight);
        // TODO: a push or pull that succeeds does not clear `last_error`. A
        // `[TRANSIENT]` error stays on the Devices page until a push uploads
        // something.
        if let Err(e) = push::flush_push(&inner).await {
            record_round_failure(&inner, "push", &e).await;
        }

        // Check the interval on every round: the backend may have changed between rounds.
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

        // Waiting for the next round is not syncing. Without this, the Devices
        // page would always show "Syncing…", and "Sync now" would always refuse.
        drop(_in_flight);
        tokio::time::sleep(inner.cloud().push_interval()).await;
    }
}

/// Returns the datasets that sync this round, for push and pull alike. Core,
/// website rules and super-categories always do; the optional ones follow their
/// switches, and chat and screen memory also need the memory database.
fn enabled_datasets(cfg: &Settings, has_mem: bool) -> Vec<Dataset> {
    Dataset::ALL
        .into_iter()
        .filter(|dataset| match dataset {
            Dataset::Core | Dataset::SiteRules | Dataset::SuperCategories => true,
            Dataset::AiSummaries => cfg.sync_ai_summaries,
            Dataset::Chat => cfg.sync_chat_history && has_mem,
            Dataset::Memory => cfg.sync_screen_memory && has_mem,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 开关和记忆库决定哪些数据集同步：核心三类总在；聊天和屏幕记忆还要有记忆库。
    #[test]
    fn enabled_datasets_follow_the_switches() {
        use Dataset::{AiSummaries, Chat, Core, Memory, SiteRules, SuperCategories};
        let mut cfg = Settings::default();
        assert_eq!(
            enabled_datasets(&cfg, true),
            vec![Core, SiteRules, SuperCategories]
        );

        cfg.sync_ai_summaries = true;
        cfg.sync_chat_history = true;
        cfg.sync_screen_memory = true;
        assert_eq!(
            enabled_datasets(&cfg, true),
            vec![Core, SiteRules, SuperCategories, AiSummaries, Chat, Memory]
        );
        // 没有记忆库：聊天和屏幕记忆开着也不同步
        assert_eq!(
            enabled_datasets(&cfg, false),
            vec![Core, SiteRules, SuperCategories, AiSummaries]
        );
    }

    /// 暂停期间换了后端，再从同一个 `Paused` 取，拿到的是新后端。
    #[tokio::test]
    async fn paused_cloud_is_the_replaced_backend() {
        use crate::sync::cloud::drive::{DriveClient, InMemoryDriveStore};
        use crate::sync::cloud::webdav::{FakeDav, WebDavClient};
        let pool = DbPool::open_in_memory().await.unwrap();
        let drive = DriveClient::with_fake_drive(Arc::new(InMemoryDriveStore::new()), pool.clone());
        let engine = SyncEngine::with_backend(
            pool.clone(),
            None,
            CloudBackend::from(drive),
            "device-a".into(),
        );
        let webdav = || {
            let dav = Arc::new(FakeDav::new());
            CloudBackend::from(WebDavClient::with_fake_server(
                dav,
                pool.clone(),
                "device-a".into(),
            ))
        };
        let webdav_cursor = webdav().pull_cursor_name(Dataset::Core);

        let mut paused = engine.pause_flushes().await;
        assert_ne!(
            paused.cloud().pull_cursor_name(Dataset::Core),
            webdav_cursor
        );
        paused.replace_cloud(webdav());
        assert_eq!(
            paused.cloud().pull_cursor_name(Dataset::Core),
            webdav_cursor
        );
    }

    /// 「云同步」页按前缀选提示：限流要有自己的前缀，不能和「下一轮自动重试」混在一起。
    #[test]
    fn server_busy_has_its_own_prefix() {
        let e = Error::WebDavHttp {
            stage: "propfind",
            status: 503,
            body: "BlockedTemporarily".to_string(),
        };
        assert!(format_sync_error(FailureKind::ServerBusy, &e).starts_with("[SERVER_BUSY] "));
        assert!(format_sync_error(FailureKind::Transient, &e).starts_with("[TRANSIENT] "));
    }
}
