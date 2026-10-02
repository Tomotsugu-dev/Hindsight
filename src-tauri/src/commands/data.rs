//! 报表查询 Tauri 命令——给前端「日 / 周 / 月」页面用。
//!
//! 全部命令薄壳：参数适配 + 错误转换；真实 SQL 在 [`crate::repo::reports`]。
//! `device_id = None` 表示"所有设备聚合"，传字符串则按 device 过滤。

use chrono::Local;
use tauri::State;

use crate::repo::reports::{
    self, device_filter_from_option, AppDetail, AppUsage, BucketBy, DaySummary, HourSlot,
};
use crate::storage::DbPool;

/// Time per category for each of the 24 hours of a day, for the bars at the top of the Daily
/// page. `day_offset = 0` is today, -1 is yesterday, and so on.
#[tauri::command]
pub async fn get_day_hours(
    pool: State<'_, DbPool>,
    day_offset: i32,
    device_id: Option<String>,
) -> Result<Vec<HourSlot>, String> {
    let day = reports::day_date(Local::now().date_naive(), day_offset);
    reports::day_hours(&pool, day, device_filter_from_option(device_id))
        .await
        .map_err(Into::into)
}

/// Top apps for one day, by total time, most first. `day_offset = 0` is today; `limit` defaults
/// to 10.
#[tauri::command]
pub async fn get_day_apps(
    pool: State<'_, DbPool>,
    day_offset: i32,
    limit: Option<u32>,
    device_id: Option<String>,
) -> Result<Vec<AppUsage>, String> {
    let day = reports::day_date(Local::now().date_naive(), day_offset);
    reports::top_apps(
        &pool,
        day,
        day,
        limit.unwrap_or(10),
        device_filter_from_option(device_id),
    )
    .await
    .map_err(Into::into)
}

/// Top apps within one hour of a day, for clicking an hour bar on the Daily page. A record that
/// crosses the hour counts only its part inside this hour. `hour` is 0–23; `limit` defaults to 10.
#[tauri::command]
pub async fn get_hour_apps(
    pool: State<'_, DbPool>,
    day_offset: i32,
    hour: i32,
    limit: Option<u32>,
    device_id: Option<String>,
) -> Result<Vec<AppUsage>, String> {
    let day = reports::day_date(Local::now().date_naive(), day_offset);
    reports::day_hour_apps(
        &pool,
        day,
        hour,
        limit.unwrap_or(10),
        device_filter_from_option(device_id),
    )
    .await
    .map_err(Into::into)
}

/// The details drawer opened by clicking an app: time bars and time per window title. The Daily
/// page gets 24 bars, one per hour. `icon_process` is the `iconProcess` of the clicked ranking row;
/// any member process name of the group works.
#[tauri::command]
pub async fn get_app_day_detail(
    pool: State<'_, DbPool>,
    day_offset: i32,
    icon_process: String,
    device_id: Option<String>,
) -> Result<AppDetail, String> {
    let day = reports::day_date(Local::now().date_naive(), day_offset);
    reports::app_range_detail(
        &pool,
        day,
        day,
        icon_process,
        device_filter_from_option(device_id),
        BucketBy::Hour,
    )
    .await
    .map_err(Into::into)
}

/// Same as [`get_app_day_detail`], for the Weekly page: one bar per day, Monday to Sunday.
#[tauri::command]
pub async fn get_app_week_detail(
    pool: State<'_, DbPool>,
    week_offset: i32,
    icon_process: String,
    device_id: Option<String>,
) -> Result<AppDetail, String> {
    let (from, to) = reports::week_range(Local::now().date_naive(), week_offset);
    reports::app_range_detail(
        &pool,
        from,
        to,
        icon_process,
        device_filter_from_option(device_id),
        BucketBy::Day,
    )
    .await
    .map_err(Into::into)
}

/// Same as [`get_app_day_detail`], for the Monthly page: one bar per day of the month.
#[tauri::command]
pub async fn get_app_month_detail(
    pool: State<'_, DbPool>,
    month_offset: i32,
    icon_process: String,
    device_id: Option<String>,
) -> Result<AppDetail, String> {
    let (from, to) = reports::month_range(Local::now().date_naive(), month_offset);
    reports::app_range_detail(
        &pool,
        from,
        to,
        icon_process,
        device_filter_from_option(device_id),
        BucketBy::Day,
    )
    .await
    .map_err(Into::into)
}

/// Each category's time per day of a week, one entry per day from Monday to Sunday.
/// `week_offset = 0` is this week.
#[tauri::command]
pub async fn get_week_days(
    pool: State<'_, DbPool>,
    week_offset: i32,
    device_id: Option<String>,
) -> Result<Vec<DaySummary>, String> {
    let (from, to) = reports::week_range(Local::now().date_naive(), week_offset);
    reports::day_category_time(&pool, from, to, device_filter_from_option(device_id))
        .await
        .map_err(Into::into)
}

/// Top apps for a week, totaled over the 7 days, most first. `limit` defaults to 10.
#[tauri::command]
pub async fn get_week_apps(
    pool: State<'_, DbPool>,
    week_offset: i32,
    limit: Option<u32>,
    device_id: Option<String>,
) -> Result<Vec<AppUsage>, String> {
    let (from, to) = reports::week_range(Local::now().date_naive(), week_offset);
    reports::top_apps(
        &pool,
        from,
        to,
        limit.unwrap_or(10),
        device_filter_from_option(device_id),
    )
    .await
    .map_err(Into::into)
}

/// Each category's time per day of a month, one entry per day. `month_offset = 0` is this month.
#[tauri::command]
pub async fn get_month_days(
    pool: State<'_, DbPool>,
    month_offset: i32,
    device_id: Option<String>,
) -> Result<Vec<DaySummary>, String> {
    let (from, to) = reports::month_range(Local::now().date_naive(), month_offset);
    reports::day_category_time(&pool, from, to, device_filter_from_option(device_id))
        .await
        .map_err(Into::into)
}

/// Top apps for a month, totaled over the month, most first. `limit` defaults to 10.
#[tauri::command]
pub async fn get_month_apps(
    pool: State<'_, DbPool>,
    month_offset: i32,
    limit: Option<u32>,
    device_id: Option<String>,
) -> Result<Vec<AppUsage>, String> {
    let (from, to) = reports::month_range(Local::now().date_naive(), month_offset);
    reports::top_apps(
        &pool,
        from,
        to,
        limit.unwrap_or(10),
        device_filter_from_option(device_id),
    )
    .await
    .map_err(Into::into)
}
