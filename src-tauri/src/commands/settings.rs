use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

use crate::capture::CaptureService;
use crate::commands::screen_memory::MemoryState;
use crate::memory::resident::ResidentOcr;
use crate::repo::settings::{self, Settings, SettingsPatch};
use crate::storage::DbPool;

/// 拉当前 Settings 全集——前端「设置」页面进去时调一次。
#[tauri::command]
pub async fn get_settings(pool: State<'_, DbPool>) -> Result<Settings, String> {
    settings::load(&pool).await.map_err(Into::into)
}

/// 应用 patch 更新部分 settings 字段。
///
/// 副作用：把 capture 相关字段同步给 `CaptureService`（间隔 / 工作时段 / 隐私关键词
/// / 挂机阈值 / 截图配置），把 minimize_to_tray 同步给 close handler 静态变量，
/// 把 auto_start 切到操作系统的开机自启。所有变更立刻生效，不需要重启。
#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    pool: State<'_, DbPool>,
    svc: State<'_, Arc<CaptureService>>,
    resident: State<'_, Arc<ResidentOcr>>,
    mem: State<'_, MemoryState>,
    patch: SettingsPatch,
) -> Result<Settings, String> {
    let current = settings::load(&pool).await.map_err(String::from)?;

    let prev_enabled = current.capture_enabled;
    let prev_interval = current.capture_interval_seconds;
    let prev_autostart = current.auto_start;
    let prev_resident = current.memory_ocr_resident;

    let next = settings::apply_patch(current, patch);
    settings::save(&pool, &next).await.map_err(String::from)?;

    // 关闭按钮行为切换：同步给 close handler 读的 static，下次点 X 立即生效，
    // 不需要重启
    crate::MINIMIZE_TO_TRAY.store(next.minimize_to_tray, std::sync::atomic::Ordering::Relaxed);

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
        next.screenshot_enabled,
        next.screenshot_path.clone(),
        // 存档规格与 bootstrap 保持一致(screen-memory.md L2 定案):≤2880/q85
        2880,
        2880,
        85,
    )
    .await;
    svc.set_privacy_keywords(
        next.privacy_url_keywords.clone(),
        next.privacy_app_keywords.clone(),
    )
    .await;
    svc.set_idle_threshold(next.idle_threshold_seconds).await;

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

    // OCR 常驻开关:启停立即生效,不需要重启
    if next.memory_ocr_resident != prev_resident {
        resident.sync(next.memory_ocr_resident, mem.0.clone()).await;
    }

    Ok(next)
}
