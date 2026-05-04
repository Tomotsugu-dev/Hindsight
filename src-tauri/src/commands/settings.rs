use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

use crate::capture::CaptureService;
use crate::repo::settings::{self, Settings, SettingsPatch};
use crate::storage::DbPool;

#[tauri::command]
pub async fn get_settings(pool: State<'_, DbPool>) -> Result<Settings, String> {
    settings::load(&pool).await.map_err(Into::into)
}

#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    pool: State<'_, DbPool>,
    svc: State<'_, Arc<CaptureService>>,
    patch: SettingsPatch,
) -> Result<Settings, String> {
    let current = settings::load(&pool).await.map_err(|e| e.to_string())?;

    let prev_enabled = current.capture_enabled;
    let prev_interval = current.capture_interval_seconds;
    let prev_autostart = current.auto_start;

    let next = settings::apply_patch(current, patch);
    settings::save(&pool, &next).await.map_err(|e| e.to_string())?;

    if next.capture_enabled != prev_enabled {
        if next.capture_enabled {
            svc.start().await;
        } else {
            svc.stop().await;
        }
    }
    if next.capture_interval_seconds != prev_interval {
        svc.set_interval(next.capture_interval_seconds).await;
    }
    svc.set_work_hours(next.work_hours_enabled, next.work_ranges.clone())
        .await;
    svc.set_screenshot_config(
        next.capture_enabled,
        next.screenshot_path.clone(),
        1280,
        720,
        80,
    )
    .await;
    svc.set_privacy_keywords(
        next.privacy_url_keywords.clone(),
        next.privacy_app_keywords.clone(),
    )
    .await;

    if next.auto_start != prev_autostart {
        let mgr = app.autolaunch();
        let res = if next.auto_start {
            mgr.enable()
        } else {
            mgr.disable()
        };
        if let Err(e) = res {
            log::warn!("切换开机自启失败: {e}");
        }
    }

    Ok(next)
}
