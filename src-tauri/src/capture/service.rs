use std::sync::Arc;

use chrono::{Local, Timelike};
use serde::Serialize;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::capture::{screenshot, window};
use crate::error::Result;
use crate::repo::settings::TimeRange;
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

#[derive(Default, Clone)]
struct WorkHoursState {
    enabled: bool,
    ranges: Vec<TimeRange>,
}

#[derive(Clone)]
struct ScreenshotConfig {
    enabled: bool,
    dir: String,
    target_width: u32,
    target_height: u32,
    jpeg_quality: u8,
}

impl Default for ScreenshotConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dir: String::new(),
            target_width: 1280,
            target_height: 720,
            jpeg_quality: 80,
        }
    }
}

struct Inner {
    pool: DbPool,
    interval_secs: Mutex<u32>,
    handle: Mutex<Option<JoinHandle<()>>>,
    last_capture_at: Mutex<Option<String>>,
    last_error: Mutex<Option<String>>,
    work_hours: Mutex<WorkHoursState>,
    screenshot: Mutex<ScreenshotConfig>,
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
                work_hours: Mutex::new(WorkHoursState::default()),
                screenshot: Mutex::new(ScreenshotConfig::default()),
            }),
        }
    }

    pub async fn set_screenshot_config(
        &self,
        enabled: bool,
        dir: String,
        target_width: u32,
        target_height: u32,
        jpeg_quality: u8,
    ) {
        let mut cfg = self.inner.screenshot.lock().await;
        *cfg = ScreenshotConfig {
            enabled,
            dir,
            target_width,
            target_height,
            jpeg_quality,
        };
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

    pub async fn set_interval(&self, secs: u32) {
        let secs = secs.clamp(1, 600);
        *self.inner.interval_secs.lock().await = secs;
    }

    pub async fn set_work_hours(&self, enabled: bool, ranges: Vec<TimeRange>) {
        let mut state = self.inner.work_hours.lock().await;
        *state = WorkHoursState { enabled, ranges };
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
    {
        let wh = inner.work_hours.lock().await;
        if wh.enabled && !wh.ranges.is_empty() {
            let now = Local::now();
            let now_minutes = now.hour() as i32 * 60 + now.minute() as i32;
            let in_range = wh.ranges.iter().any(|r| {
                let start = parse_hm(&r.start);
                let end = parse_hm(&r.end);
                if start <= end {
                    now_minutes >= start && now_minutes < end
                } else {
                    now_minutes >= start || now_minutes < end
                }
            });
            if !in_range {
                log::debug!("跳过本次采集：当前不在工作时段");
                return Ok(());
            }
        }
    }

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
        let shot = take_screenshot(inner).await;
        activities::insert_new(&inner.pool, &info, now, shot).await?;
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

fn parse_hm(s: &str) -> i32 {
    let mut parts = s.split(':');
    let h: i32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let m: i32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    h * 60 + m
}

async fn take_screenshot(inner: &Inner) -> Option<String> {
    let cfg = inner.screenshot.lock().await.clone();
    if !cfg.enabled || cfg.dir.trim().is_empty() {
        return None;
    }
    let path = std::path::PathBuf::from(&cfg.dir);
    match screenshot::capture_active_window(
        path,
        cfg.target_width,
        cfg.target_height,
        cfg.jpeg_quality,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            log::warn!("截图失败: {e}");
            None
        }
    }
}
