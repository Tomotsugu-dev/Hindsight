use std::sync::Arc;
use std::time::Instant;

use chrono::{Duration, Local, Timelike};
use serde::Serialize;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::capture::{browser_url, privacy, screenshot, window};
use crate::error::Result;
use crate::repo::settings::TimeRange;
use crate::repo::{activities, app_groups, process_paths};
use crate::storage::DbPool;

const POLL_INTERVAL_SECS: u64 = 5;

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
    /// 仅浏览器场景填值；非浏览器为 None。
    /// title 不参与 focus 比较，避免光标位置 / 未保存标记等带来的频繁抖动。
    url: Option<String>,
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
    /// 浏览器 URL 关键词；地址栏 URL 命中其中一条即跳过截图
    privacy_url_keywords: Mutex<Vec<String>>,
    /// 应用 / 标题关键词；app_name 或 window title 命中其中一条即跳过截图
    privacy_app_keywords: Mutex<Vec<String>>,
    /// 用户多久不动鼠键就算"挂机"，超过这个秒数 tick 就 seal 当前会话不再延续。
    /// 0 = 关闭挂机检测（永远算在用，回到 idle 检测之前的行为）。
    idle_threshold_secs: Mutex<u32>,
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
                privacy_url_keywords: Mutex::new(Vec::new()),
                privacy_app_keywords: Mutex::new(Vec::new()),
                // 真实值由 lib.rs 启动时根据 settings 写入；此处给个安全默认（180s = 3min），
                // 万一 set 没调到，行为仍然合理。与 settings::Settings::default 保持一致。
                idle_threshold_secs: Mutex::new(180),
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

    /// 更新挂机阈值（秒）。0 = 关闭检测。设置页改完由命令层调一次。
    pub async fn set_idle_threshold(&self, secs: u32) {
        *self.inner.idle_threshold_secs.lock().await = secs.min(3600);
    }

    /// 更新隐私关键词。设置页改完后由命令层调一次。
    pub async fn set_privacy_keywords(&self, url_keywords: Vec<String>, app_keywords: Vec<String>) {
        *self.inner.privacy_url_keywords.lock().await = url_keywords;
        *self.inner.privacy_app_keywords.lock().await = app_keywords;
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

    // 挂机检测：用户多久没动鼠键。超过阈值就 seal 当前会话并 return，让"挂机时段"
    // 不计入使用时长。ended_at 用"用户最后一次活动时间" = now - idle，duration 才准。
    // 用户回来动鼠键 → 下次 tick 看到 current=None → 自然开新会话。
    {
        let threshold = *inner.idle_threshold_secs.lock().await;
        if threshold > 0 {
            let idle = crate::platform::idle_secs();
            if idle as u32 >= threshold {
                let mut cur_lock = inner.current.lock().await;
                if let Some(prev) = cur_lock.take() {
                    drop(cur_lock);
                    let real_end = Local::now() - Duration::seconds(idle as i64);
                    if let Err(e) =
                        activities::seal_session(&inner.pool, prev.id, real_end).await
                    {
                        log::warn!(
                            "seal_session 失败 (用户挂机 {idle}s, id={}): {e}",
                            prev.id
                        );
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

    // 浏览器场景：每次 tick 都抓 URL，因为 URL 是 focus 的一部分（切 URL 要立即触发新会话）。
    // 平台调用阻塞 + 偶尔卡几百 ms，扔到 spawn_blocking 不堵 async runtime。
    // 非浏览器跳过这步，省 50–300ms。
    let url = if browser_url::is_browser_app(&info.app_name) {
        let app_name_for_url = info.app_name.clone();
        tokio::task::spawn_blocking(move || {
            browser_url::try_get_foreground_browser_url(&app_name_for_url)
        })
        .await
        .ok()
        .flatten()
    } else {
        None
    };

    let new_focus = FocusState {
        app_name: info.app_name.clone(),
        url: url.clone(),
    };

    let mut current_lock = inner.current.lock().await;
    let interval_secs = *inner.interval_secs.lock().await;
    let need_new = match current_lock.as_ref() {
        None => true,
        // 触发新会话的两种情况：
        //   1) 焦点切换（不同 app / 不同 url）—— 立即截图
        //   2) 同一焦点停留满 interval_secs —— 周期性补一张图，让长时间停留也有时间序列
        Some(cur) => {
            cur.focus != new_focus
                || cur.last_extend_at.elapsed().as_secs() as u32 >= interval_secs
        }
    };

    if need_new {
        // 焦点切换 / 间隔到点：先把旧会话钉死（推 outbox），再开新会话
        if let Some(prev) = current_lock.take() {
            if let Err(e) = activities::seal_session(&inner.pool, prev.id, now).await {
                log::warn!("seal_session 失败 (开新会话, id={}): {e}", prev.id);
            }
        }
        // 隐私过滤：标题或 URL（如果有）命中关键词 → 不截图，但活动行照常落库。
        let skip = should_skip_for_privacy(inner, &info, url.as_deref()).await;
        if skip {
            log::info!(
                "隐私过滤命中，跳过截图 app={} title={:?} url={:?}",
                info.app_name,
                info.title,
                url
            );
        }
        let shot = if skip {
            None
        } else {
            take_screenshot(inner).await
        };
        let id = activities::insert_new(&inner.pool, &info, now, shot).await?;
        // 保证这个 process_name 有对应的 app_group / member（首次见到的应用建单成员组）
        if let Err(e) = app_groups::ensure_group(&inner.pool, &info.app_name).await {
            log::warn!("ensure_group 失败 ({}): {e}", info.app_name);
        }
        *current_lock = Some(CurrentSession {
            id,
            focus: new_focus,
            last_extend_at: Instant::now(),
        });
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

/// 隐私关键词是否命中本次焦点。
/// `url` 是浏览器地址栏 URL（暂未接入抓取，永远 None；接入后由调用方传入）。
async fn should_skip_for_privacy(
    inner: &Inner,
    info: &window::WindowInfo,
    url: Option<&str>,
) -> bool {
    let url_kw = inner.privacy_url_keywords.lock().await.clone();
    let app_kw = inner.privacy_app_keywords.lock().await.clone();
    if url_kw.is_empty() && app_kw.is_empty() {
        return false;
    }
    privacy::should_skip_screenshot(&info.app_name, &info.title, url, &url_kw, &app_kw)
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
