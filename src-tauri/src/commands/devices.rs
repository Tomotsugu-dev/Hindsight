use tauri::State;

use crate::repo::devices::{self, DeviceRow};
use crate::storage::DbPool;

#[tauri::command]
pub async fn list_devices(pool: State<'_, DbPool>) -> Result<Vec<DeviceRow>, String> {
    devices::list_all(&pool).await.map_err(String::from)
}

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
