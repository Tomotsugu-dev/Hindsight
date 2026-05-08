use tauri::State;

use crate::repo::devices::{self, DeviceRow};
use crate::storage::DbPool;

/// 拉所有已知设备（本机 + 同步看到的远端设备）的清单。
/// 前端「设备」页面渲染设备卡片用，每张卡片显示 display_name / color / icon / last_seen。
#[tauri::command]
pub async fn list_devices(pool: State<'_, DbPool>) -> Result<Vec<DeviceRow>, String> {
    devices::list_all(&pool).await.map_err(String::from)
}

/// 更新本机设备的展示名 / 配色 / 图标。
/// 同时写 DB（让其它设备同步到）+ 写 device.json（让下次冷启动也用新值）。
#[tauri::command]
pub async fn update_self_device(
    pool: State<'_, DbPool>,
    name: Option<String>,
    color: Option<String>,
    icon: Option<String>,
) -> Result<DeviceRow, String> {
    let id = crate::device::self_id().map_err(String::from)?.to_string();
    let row = devices::update_self_meta(&pool, id, name, color, icon)
        .await
        .map_err(String::from)?;

    // 同时把 device.json 也写一份（保证下次冷启动 `device::ensure_loaded` 拿到最新值）
    if let Err(e) = crate::device::update_self(
        Some(row.display_name.clone()),
        Some(row.color.clone()),
        Some(row.icon.clone()),
    ) {
        log::warn!("device.json 更新失败（DB 已更新）: {e}");
    }

    Ok(row)
}
