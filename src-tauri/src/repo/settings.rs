use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::storage::{db_path_dir, DbPool};

pub fn default_screenshot_path() -> String {
    db_path_dir()
        .map(|p| p.join("screenshots").to_string_lossy().to_string())
        .unwrap_or_else(|_| String::new())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeRange {
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub capture_enabled: bool,
    pub capture_interval_seconds: u32,
    pub screenshot_path: String,
    pub work_hours_enabled: bool,
    pub work_ranges: Vec<TimeRange>,
    pub auto_start: bool,
    pub show_window_on_auto_start: bool,
    pub retention_days: u32,
    /// 用户自带的 Firebase 项目凭证（BYO 架构 —— 数据放进用户自己的 Firebase）
    pub firebase_client_id: String,
    pub firebase_client_secret: String,
    pub firebase_api_key: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            capture_enabled: true,
            capture_interval_seconds: 10,
            screenshot_path: String::new(),
            work_hours_enabled: false,
            work_ranges: Vec::new(),
            auto_start: false,
            show_window_on_auto_start: false,
            retention_days: 7,
            firebase_client_id: String::new(),
            firebase_client_secret: String::new(),
            firebase_api_key: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub capture_enabled: Option<bool>,
    pub capture_interval_seconds: Option<u32>,
    pub screenshot_path: Option<String>,
    pub work_hours_enabled: Option<bool>,
    pub work_ranges: Option<Vec<TimeRange>>,
    pub auto_start: Option<bool>,
    pub show_window_on_auto_start: Option<bool>,
    pub retention_days: Option<u32>,
    pub firebase_client_id: Option<String>,
    pub firebase_client_secret: Option<String>,
    pub firebase_api_key: Option<String>,
}

pub async fn load(pool: &DbPool) -> Result<Settings> {
    let data: String = pool
        .0
        .call(|conn| {
            let row: String = conn
                .query_row("SELECT data FROM settings_store WHERE id = 1", [], |r| {
                    r.get(0)
                })
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(row)
        })
        .await?;

    let mut settings = serde_json::from_str::<Settings>(&data).unwrap_or_default();

    if settings.screenshot_path.trim().is_empty() {
        settings.screenshot_path = default_screenshot_path();
        save(pool, &settings).await?;
    }

    Ok(settings)
}

pub async fn save(pool: &DbPool, settings: &Settings) -> Result<()> {
    let data = serde_json::to_string(settings)?;
    pool.0
        .call(move |conn| {
            conn.execute(
                "UPDATE settings_store SET data = ? WHERE id = 1",
                rusqlite::params![data],
            )
            .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub fn apply_patch(current: Settings, patch: SettingsPatch) -> Settings {
    Settings {
        capture_enabled: patch.capture_enabled.unwrap_or(current.capture_enabled),
        capture_interval_seconds: patch
            .capture_interval_seconds
            .map(|v| v.clamp(1, 600))
            .unwrap_or(current.capture_interval_seconds),
        screenshot_path: patch
            .screenshot_path
            .map(|p| {
                if p.trim().is_empty() {
                    default_screenshot_path()
                } else {
                    p
                }
            })
            .unwrap_or(current.screenshot_path),
        work_hours_enabled: patch
            .work_hours_enabled
            .unwrap_or(current.work_hours_enabled),
        work_ranges: patch.work_ranges.unwrap_or(current.work_ranges),
        auto_start: patch.auto_start.unwrap_or(current.auto_start),
        show_window_on_auto_start: patch
            .show_window_on_auto_start
            .unwrap_or(current.show_window_on_auto_start),
        retention_days: patch
            .retention_days
            .map(|v| v.clamp(1, 365))
            .unwrap_or(current.retention_days),
        firebase_client_id: patch
            .firebase_client_id
            .map(|v| v.trim().to_string())
            .unwrap_or(current.firebase_client_id),
        firebase_client_secret: patch
            .firebase_client_secret
            .map(|v| v.trim().to_string())
            .unwrap_or(current.firebase_client_secret),
        firebase_api_key: patch
            .firebase_api_key
            .map(|v| v.trim().to_string())
            .unwrap_or(current.firebase_api_key),
    }
}
