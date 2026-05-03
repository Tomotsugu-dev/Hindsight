use std::sync::Arc;

use chrono::Local;
use serde::Serialize;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::capture::window;
use crate::error::Result;
use crate::repo::{activities, process_paths};
use crate::storage::DbPool;

const MERGE_GAP_SECS: i64 = 600;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    pub running: bool,
    pub today_count: u32,
    pub last_capture_at: Option<String>,
    pub last_error: Option<String>,
}

struct Inner {
    pool: DbPool,
    interval_secs: Mutex<u32>,
    handle: Mutex<Option<JoinHandle<()>>>,
    last_capture_at: Mutex<Option<String>>,
    last_error: Mutex<Option<String>>,
}

pub struct CaptureService {
    inner: Arc<Inner>,
}

impl CaptureService {
    pub fn new(pool: DbPool, interval_secs: u32) -> Self {
        Self {
            inner: Arc::new(Inner {
                pool,
                interval_secs: Mutex::new(interval_secs),
                handle: Mutex::new(None),
                last_capture_at: Mutex::new(None),
                last_error: Mutex::new(None),
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
            loop {
                let secs = *inner.interval_secs.lock().await;
                tokio::time::sleep(std::time::Duration::from_secs(secs as u64)).await;
                if let Err(e) = tick(&inner).await {
                    log::warn!("采集 tick 失败: {e}");
                    let mut le = inner.last_error.lock().await;
                    *le = Some(e.to_string());
                }
            }
        }));
    }

    pub async fn stop(&self) {
        let mut h = self.inner.handle.lock().await;
        if let Some(handle) = h.take() {
            handle.abort();
        }
    }

    pub async fn is_running(&self) -> bool {
        self.inner.handle.lock().await.is_some()
    }

    pub async fn status(&self) -> CaptureStatus {
        let today_count = activities::today_count(&self.inner.pool)
            .await
            .unwrap_or(0);
        CaptureStatus {
            running: self.is_running().await,
            today_count,
            last_capture_at: self.inner.last_capture_at.lock().await.clone(),
            last_error: self.inner.last_error.lock().await.clone(),
        }
    }
}

async fn tick(inner: &Inner) -> Result<()> {
    let info = match window::current_window() {
        Ok(i) => i,
        Err(e) => {
            log::debug!("跳过本次采集：{e}");
            return Ok(());
        }
    };

    let now = Local::now();
    let latest = activities::latest_for(&inner.pool, &info.app_name).await?;

    let should_merge = match latest.as_ref() {
        Some(l) => {
            let gap = (now - l.ended_at).num_seconds();
            let same_title = l.window_title.as_deref().unwrap_or("") == info.title;
            info.app_name != "Unknown" && gap <= MERGE_GAP_SECS && same_title
        }
        None => false,
    };

    if should_merge {
        let id = latest.unwrap().id;
        activities::extend(&inner.pool, id, now).await?;
    } else {
        activities::insert_new(&inner.pool, &info, now).await?;
    }

    if let Some(path) = info.app_path.as_ref() {
        if !path.is_empty() {
            let _ = process_paths::upsert(&inner.pool, &info.app_name, path).await;
        }
    }

    let mut last_at = inner.last_capture_at.lock().await;
    *last_at = Some(now.to_rfc3339());
    let mut last_err = inner.last_error.lock().await;
    *last_err = None;

    Ok(())
}
