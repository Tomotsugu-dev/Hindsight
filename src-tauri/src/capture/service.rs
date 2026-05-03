use std::sync::Arc;
use std::time::Instant;

use chrono::{Local, Timelike};
use serde::Serialize;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::capture::{screenshot, window};
use crate::error::Result;
use crate::repo::settings::TimeRange;
use crate::repo::{activities, process_paths};
use crate::storage::DbPool;

const POLL_INTERVAL_SECS: u64 = 1;

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

#[derive(Clone, PartialEq, Eq)]
struct FocusState {
    app_name: String,
    title: String,
}

struct CurrentSession {
    id: i64,
    focus: FocusState,
    last_extend_at: Instant,
}

struct Inner {
    pool: DbPool,
    interval_secs: Mutex<u32>,
    handle: Mutex<Option<JoinHandle<()>>>,
    last_capture_at: Mutex<Option<String>>,
    last_error: Mutex<Option<String>>,
    work_hours: Mutex<WorkHoursState>,
    screenshot: Mutex<ScreenshotConfig>,
    current: Mutex<Option<CurrentSession>>,
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
                current: Mutex::new(None),
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
                tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS)).await;
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
        // seal 当前会话（如果有），让它进入 outbox 在下次同步推送
        let mut cur_lock = self.inner.current.lock().await;
        if let Some(prev) = cur_lock.take() {
            drop(cur_lock);
            if let Err(e) =
                activities::seal_session(&self.inner.pool, prev.id, chrono::Local::now()).await
            {
                log::warn!("stop: seal_session 失败 (id={}): {e}", prev.id);
            }
        }
    }

    /// 清空当前会话指针；用于在外部清空 activities 表后避免下一次 tick 去 UPDATE 已被删除的行。
    pub async fn reset_session(&self) {
        *self.inner.current.lock().await = None;
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
                // 离开工作时段：seal 当前会话（推到云端） + 清空 current
                let mut cur_lock = inner.current.lock().await;
                if let Some(prev) = cur_lock.take() {
                    drop(cur_lock);
                    if let Err(e) =
                        activities::seal_session(&inner.pool, prev.id, Local::now()).await
                    {
                        log::warn!("seal_session 失败 (离开工作时段, id={}): {e}", prev.id);
                    }
                }
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

    if info.app_name.is_empty() || info.app_name == "Unknown" {
        return Ok(());
    }

    let now = Local::now();
    let new_focus = FocusState {
        app_name: info.app_name.clone(),
        title: info.title.clone(),
    };

    let mut current_lock = inner.current.lock().await;
    let need_new = match current_lock.as_ref() {
        None => true,
        Some(cur) => cur.focus != new_focus,
    };

    if need_new {
        // 焦点切换：先把旧会话钉死（推 outbox），再开新会话
        if let Some(prev) = current_lock.take() {
            if let Err(e) = activities::seal_session(&inner.pool, prev.id, now).await {
                log::warn!("seal_session 失败 (焦点切换, id={}): {e}", prev.id);
            }
        }
        let shot = take_screenshot(inner).await;
        let id = activities::insert_new(&inner.pool, &info, now, shot).await?;
        *current_lock = Some(CurrentSession {
            id,
            focus: new_focus,
            last_extend_at: Instant::now(),
        });
    } else {
        let interval_secs = *inner.interval_secs.lock().await;
        let cur = current_lock.as_mut().unwrap();
        if cur.last_extend_at.elapsed().as_secs() as u32 >= interval_secs {
            activities::extend(&inner.pool, cur.id, now).await?;
            cur.last_extend_at = Instant::now();
        }
    }
    drop(current_lock);

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
