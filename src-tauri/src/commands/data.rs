use tauri::State;

use crate::repo::reports::{self, AppUsage, HourSlot};
use crate::storage::DbPool;

#[tauri::command]
pub async fn get_day_hours(
    pool: State<'_, DbPool>,
    day_offset: i32,
) -> Result<Vec<HourSlot>, String> {
    reports::day_hours(&pool, day_offset)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_day_apps(
    pool: State<'_, DbPool>,
    day_offset: i32,
    limit: Option<u32>,
) -> Result<Vec<AppUsage>, String> {
    reports::day_apps(&pool, day_offset, limit.unwrap_or(10))
        .await
        .map_err(Into::into)
}
