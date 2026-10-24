//! Tauri commands for the report pages: Daily, Weekly, Monthly and All time.
//!
//! Each command is a thin wrapper that adapts arguments and converts errors; the SQL lives in
//! [`crate::repo::reports`]. `device_id = None` adds up all devices; a string filters to that
//! device.

use chrono::{Local, NaiveDate};
use serde::Deserialize;
use tauri::State;

use crate::repo::reports::{
    self, AppDetail, AppTitles, AppUsage, BucketBy, DaySummary, DetailBucket, DeviceFilter,
    HourSlot,
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

/// A report page's range: day, week or month.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DetailScope {
    Day,
    Week,
    Month,
}

impl DetailScope {
    /// Returns the range's first day, its last day, and whether the time bars are by hour or by
    /// day.
    fn range(self, today: NaiveDate, offset: i32) -> (NaiveDate, NaiveDate, BucketBy) {
        match self {
            DetailScope::Day => {
                let day = reports::day_date(today, offset);
                (day, day, BucketBy::Hour)
            }
            DetailScope::Week => {
                let (from, to) = reports::week_range(today, offset);
                (from, to, BucketBy::Day)
            }
            DetailScope::Month => {
                let (from, to) = reports::month_range(today, offset);
                (from, to, BucketBy::Day)
            }
        }
    }
}

/// Returns one app's time in the given range.
///
/// By hour on the Daily page, by day on the Weekly and Monthly pages.
#[tauri::command]
pub async fn get_app_buckets(
    pool: State<'_, DbPool>,
    scope: DetailScope,
    offset: i32,
    group_id: String,
    device_id: Option<String>,
) -> Result<Vec<DetailBucket>, String> {
    let (from, to, bucket_by) = scope.range(Local::now().date_naive(), offset);
    reports::app_buckets(
        &pool,
        from,
        to,
        group_id,
        DeviceFilter::from_option(device_id),
        bucket_by,
    )
    .await
    .map_err(Into::into)
}

/// Returns one app's time per window title in the given range, and whether it is a browser.
#[tauri::command]
pub async fn get_app_titles(
    pool: State<'_, DbPool>,
    scope: DetailScope,
    offset: i32,
    group_id: String,
    device_id: Option<String>,
) -> Result<AppTitles, String> {
    let (from, to, _) = scope.range(Local::now().date_naive(), offset);
    reports::app_titles(
        &pool,
        from,
        to,
        group_id,
        DeviceFilter::from_option(device_id),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 前端传小写的 "day" / "week" / "month"；换算出的日期和 reports 里的一样，日统计按小时，
    /// 周、月按天。
    #[test]
    fn detail_scope_reads_the_frontend_names_and_maps_to_dates() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 24).unwrap();
        let scope =
            |name: &str| serde_json::from_str::<DetailScope>(&format!("\"{name}\"")).unwrap();

        let (from, to, by) = scope("day").range(today, -1);
        let yesterday = reports::day_date(today, -1);
        assert_eq!((from, to), (yesterday, yesterday));
        assert!(matches!(by, BucketBy::Hour));

        let (from, to, by) = scope("week").range(today, 0);
        assert_eq!((from, to), reports::week_range(today, 0));
        assert!(matches!(by, BucketBy::Day));

        let (from, to, by) = scope("month").range(today, -2);
        assert_eq!((from, to), reports::month_range(today, -2));
        assert!(matches!(by, BucketBy::Day));
    }
}
