use tauri::State;

use crate::repo::reports::{
    self, device_filter_from_option, AppUsage, DaySummary, HourSlot,
};
use crate::storage::DbPool;

#[tauri::command]
pub async fn get_day_hours(
    pool: State<'_, DbPool>,
    day_offset: i32,
    device_id: Option<String>,
) -> Result<Vec<HourSlot>, String> {
    reports::day_hours(&pool, day_offset, device_filter_from_option(device_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_day_apps(
    pool: State<'_, DbPool>,
    day_offset: i32,
    limit: Option<u32>,
    device_id: Option<String>,
) -> Result<Vec<AppUsage>, String> {
    reports::day_apps(
        &pool,
        day_offset,
        limit.unwrap_or(10),
        device_filter_from_option(device_id),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_week_days(
    pool: State<'_, DbPool>,
    week_offset: i32,
    device_id: Option<String>,
) -> Result<Vec<DaySummary>, String> {
    reports::week_days(&pool, week_offset, device_filter_from_option(device_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_week_apps(
    pool: State<'_, DbPool>,
    week_offset: i32,
    limit: Option<u32>,
    device_id: Option<String>,
) -> Result<Vec<AppUsage>, String> {
    reports::week_apps(
        &pool,
        week_offset,
        limit.unwrap_or(10),
        device_filter_from_option(device_id),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_month_days(
    pool: State<'_, DbPool>,
    month_offset: i32,
    device_id: Option<String>,
) -> Result<Vec<DaySummary>, String> {
    reports::month_days(&pool, month_offset, device_filter_from_option(device_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_month_apps(
    pool: State<'_, DbPool>,
    month_offset: i32,
    limit: Option<u32>,
    device_id: Option<String>,
) -> Result<Vec<AppUsage>, String> {
    reports::month_apps(
        &pool,
        month_offset,
        limit.unwrap_or(10),
        device_filter_from_option(device_id),
    )
    .await
    .map_err(Into::into)
}
