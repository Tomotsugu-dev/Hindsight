//! 同步引擎：登录后台跑两件事
//!   - push：每 30 秒把 sync_outbox 翻成"哪些文件脏了"，对每个脏文件全量重写到 Drive appDataFolder
//!   - pull：每 60 秒列 Drive 上其他设备的文件，按 modifiedTime 增量下载并 LWW merge 到本地
//!
//! 失败走指数退避（最多 1 小时），attempts > 10 留在 outbox 作为 dead-letter，UI 可以看见。

mod io;
mod pull;
mod push;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::error::{Error, Result};
use crate::storage::DbPool;
use crate::sync::auth::{self, TokenInfo};

/// `last_error` 前缀：分类后端写进 status 的同步错误，让前端能稳定判别
/// "需要重新登录" vs "暂时失败"，不再依赖中文本地化字符串匹配。
pub(super) const ERR_PREFIX_CRED_EXPIRED: &str = "[CRED_EXPIRED] ";
pub(super) const ERR_PREFIX_TRANSIENT: &str = "[TRANSIENT] ";

/// 把同步过程产生的 Error 归类成"需要用户介入" vs "等下个 tick 自动重试就行"，
/// 然后加上稳定前缀给前端识别。原文 e.to_string() 拼在前缀后面，UI 显示时去前缀。
pub(super) fn format_sync_error(e: &Error) -> String {
    let prefix = match e {
        // refresh_token 真的失效（用户在 myaccount.google.com 撤销 / token 过期 6 个月）
        Error::OAuthHttp {
            endpoint: "refresh",
            status,
            ..
        } if *status == 400 || *status == 401 => ERR_PREFIX_CRED_EXPIRED,
        // AES 解不开：本地密钥 / 密文已损坏，重新登录是唯一出路
        Error::Crypto(_) => ERR_PREFIX_CRED_EXPIRED,
        // scope 不足：当前 token 没 drive.appdata 权限，必须重新走同意页
        Error::DriveScopeInsufficient => ERR_PREFIX_CRED_EXPIRED,
        // 其它：keyring 临时读失败、网络超时、Drive 5xx、refresh 端点 5xx 等。
        // 后台 30s tick 会自动重试，UI 不必催用户重新登录。
        _ => ERR_PREFIX_TRANSIENT,
    };
    format!("{prefix}{e}")
}

/// 包一次 Drive 调用：如果返回 401，强制刷新 access_token 后重试一次。
///
/// 原因：`auth::ensure_valid_token` 只看本地 `expires_at`，但 Google 端可能
/// 因机器睡眠醒来 / 时钟漂移 / 服务端轮换在到期前就拒收 access_token。
/// 此时单纯刷一次 token 就能恢复，不应让用户重新登录整个 OAuth 流程。
pub(super) async fn with_token_retry<F, Fut, T>(
    pool: &DbPool,
    token: &mut TokenInfo,
    mut op: F,
) -> Result<T>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    match op(token.access_token.clone()).await {
        Err(Error::DriveHttp {
            status: 401,
            stage,
            body,
        }) => {
            log::info!("drive {stage} 返回 401，强制刷新 access_token 后重试");
            log::debug!("drive 401 body: {body}");
            *token = auth::force_refresh(pool).await?;
            op(token.access_token.clone()).await
        }
        other => other,
    }
}

const PUSH_INTERVAL_SECS: u64 = 30;
const PULL_INTERVAL_SECS: i64 = 60;

/// 同步引擎当前状态的对外快照（前端「设备」页面读）。
#[derive(Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    /// 后台 push/pull 循环是否在跑
    pub running: bool,
    /// 最近一次 push 成功的 RFC3339 时间
    pub last_pushed_at: Option<String>,
    /// 最近一次 pull 成功的 RFC3339 时间
    pub last_pulled_at: Option<String>,
    /// 最近一次失败原因（成功后清空）；token 失效会落到这里
    pub last_error: Option<String>,
    /// outbox 待推送行数（含 dead_letter）
    pub pending: u64,
    /// attempts > 10 的死信行数；UI 单独红色提示
    pub dead_letter: u64,
}

/// 内部共享状态：池、后台任务句柄、状态。push / pull 模块都用 `&Arc<Inner>` 访问。
pub(super) struct Inner {
    pub(super) pool: DbPool,
    pub(super) handle: Mutex<Option<JoinHandle<()>>>,
    pub(super) status: RwLock<SyncStatus>,
}

/// 同步引擎对外句柄。一个进程一份；`app.manage(Arc::new(SyncEngine::new))` 注册。
pub struct SyncEngine {
    inner: Arc<Inner>,
}

impl SyncEngine {
    /// 创建（不自动 start）。需要外层调一次 [`SyncEngine::start`] 才会跑后台循环。
    pub fn new(pool: DbPool) -> Self {
        Self {
            inner: Arc::new(Inner {
                pool,
                handle: Mutex::new(None),
                status: RwLock::new(SyncStatus::default()),
            }),
        }
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
            // SyncIncomplete 表示 push 内部已经把分类好的字符串写进 status.last_error 了；
            // 这里如果用 format_sync_error 再覆盖一次，会拿不到 inner cause，全归 [TRANSIENT]。
            if !matches!(e, Error::SyncIncomplete(_)) {
                inner.status.write().await.last_error = Some(format_sync_error(&e));
            }
        }

        let now = Utc::now();
        let should_pull = match last_pull {
            None => true,
            Some(t) => (now - t).num_seconds() >= PULL_INTERVAL_SECS,
        };
        if should_pull {
            if let Err(e) = pull::flush_pull(&inner).await {
                log::warn!("sync pull 失败: {e}");
                if !matches!(e, Error::SyncIncomplete(_)) {
                    inner.status.write().await.last_error = Some(format_sync_error(&e));
                }
            }
            last_pull = Some(now);
        }

        tokio::time::sleep(Duration::from_secs(PUSH_INTERVAL_SECS)).await;
    }
}
