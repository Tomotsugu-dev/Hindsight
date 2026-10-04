//! Tauri commands for the report pages: Daily, Weekly, Monthly and All time.
//!
//! Each command is a thin wrapper that adapts arguments and converts errors; the SQL lives in
//! [`crate::repo::reports`]. `device_id = None` adds up all devices; a string filters to that
//! device.

use chrono::{Local, NaiveDate};
use tauri::State;

use crate::repo::reports::{
    self, AppDetail, AppUsage, BucketBy, DaySummary, DeviceFilter, HourSlot,
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
    reports::day_hours(&pool, day, DeviceFilter::from_option(device_id))
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
        DeviceFilter::from_option(device_id),
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
        DeviceFilter::from_option(device_id),
    )
    .await
    .map_err(Into::into)
}

/// The details drawer opened by clicking an app: time bars and time per window title. The Daily
/// page gets 24 bars, one per hour. `group_id` is the `groupId` of the clicked ranking row.
#[tauri::command]
pub async fn get_app_day_detail(
    pool: State<'_, DbPool>,
    day_offset: i32,
    group_id: String,
    device_id: Option<String>,
) -> Result<AppDetail, String> {
    let day = reports::day_date(Local::now().date_naive(), day_offset);
    reports::app_range_detail(
        &pool,
        day,
        day,
        group_id,
        DeviceFilter::from_option(device_id),
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
    group_id: String,
    device_id: Option<String>,
) -> Result<AppDetail, String> {
    let (from, to) = reports::week_range(Local::now().date_naive(), week_offset);
    reports::app_range_detail(
        &pool,
        from,
        to,
        group_id,
        DeviceFilter::from_option(device_id),
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
    group_id: String,
    device_id: Option<String>,
) -> Result<AppDetail, String> {
    let (from, to) = reports::month_range(Local::now().date_naive(), month_offset);
    reports::app_range_detail(
        &pool,
        from,
        to,
        group_id,
        DeviceFilter::from_option(device_id),
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
    reports::day_category_time(&pool, from, to, DeviceFilter::from_option(device_id))
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
        DeviceFilter::from_option(device_id),
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
    reports::day_category_time(&pool, from, to, DeviceFilter::from_option(device_id))
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
        DeviceFilter::from_option(device_id),
    )
    .await
    .map_err(Into::into)
}

/// The All time page: time spent in each category for every day from `from` to `to`.
#[tauri::command]
pub async fn get_range_category_time(
    pool: State<'_, DbPool>,
    from: NaiveDate,
    to: NaiveDate,
    device_id: Option<String>,
) -> Result<Vec<DaySummary>, String> {
    reports::day_category_time(&pool, from, to, DeviceFilter::from_option(device_id))
        .await
        .map_err(Into::into)
}

/// The All time page: time spent in each app from `from` to `to`.
#[tauri::command]
pub async fn get_range_apps(
    pool: State<'_, DbPool>,
    from: NaiveDate,
    to: NaiveDate,
    device_id: Option<String>,
) -> Result<Vec<AppUsage>, String> {
    reports::top_apps(
        &pool,
        from,
        to,
        u32::MAX,
        DeviceFilter::from_option(device_id),
    )
    .await
    .map_err(Into::into)
}

/// The All time page: an app's time and window titles for each day from `from` to `to`.
/// `group_id` is the `groupId` from that app's row in the app ranking.
#[tauri::command]
pub async fn get_app_range_detail(
    pool: State<'_, DbPool>,
    from: NaiveDate,
    to: NaiveDate,
    group_id: String,
    device_id: Option<String>,
) -> Result<AppDetail, String> {
    reports::app_range_detail(
        &pool,
        from,
        to,
        group_id,
        DeviceFilter::from_option(device_id),
        BucketBy::Day,
    )
    .await
    .map_err(Into::into)
}
