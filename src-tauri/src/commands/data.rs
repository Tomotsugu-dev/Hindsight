//! 报表查询 Tauri 命令——给前端「日 / 周 / 月」页面用。
//!
//! 全部命令薄壳：参数适配 + 错误转换；真实 SQL 在 [`crate::repo::reports`]。
//! `device_id = None` 表示"所有设备聚合"，传字符串则按 device 过滤。

use tauri::State;

use crate::repo::reports::{
    self, device_filter_from_option, AppDetail, AppUsage, DaySummary, HourSlot,
};
use crate::storage::DbPool;

/// 拉某天 24 小时的使用时长分布（每小时一条），给「日」页面顶部柱状图用。
/// `day_offset = 0` 是今天，-1 是昨天，依此类推。
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

/// 拉某天的 top 应用列表（按使用时长降序），`limit` 默认 10。
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

/// 拉某天某小时（local_hour = ?）的 top 应用列表，给「日」页面"点小时柱子→筛选"用。
/// `hour` 在 0..=23；其它行为同 [`get_day_apps`]。
#[tauri::command]
pub async fn get_hour_apps(
    pool: State<'_, DbPool>,
    day_offset: i32,
    hour: i32,
    limit: Option<u32>,
    device_id: Option<String>,
) -> Result<Vec<AppUsage>, String> {
    reports::day_hour_apps(
        &pool,
        day_offset,
        hour,
        limit.unwrap_or(10),
        device_filter_from_option(device_id),
    )
    .await
    .map_err(Into::into)
}

/// 「点应用 → 详情抽屉」聚合数据：时间柱 + 窗口标题用时。`icon_process` 传排行行里的
/// 稳定代表 process_name（合并组里任一成员名都行）。日报按小时聚合。
#[tauri::command]
pub async fn get_app_day_detail(
    pool: State<'_, DbPool>,
    day_offset: i32,
    icon_process: String,
    device_id: Option<String>,
) -> Result<AppDetail, String> {
    reports::app_day_detail(
        &pool,
        day_offset,
        icon_process,
        device_filter_from_option(device_id),
    )
    .await
    .map_err(Into::into)
}

/// 周报版：本周(周一~周日)按天聚合。
#[tauri::command]
pub async fn get_app_week_detail(
    pool: State<'_, DbPool>,
    week_offset: i32,
    icon_process: String,
    device_id: Option<String>,
) -> Result<AppDetail, String> {
    reports::app_week_detail(
        &pool,
        week_offset,
        icon_process,
        device_filter_from_option(device_id),
    )
    .await
    .map_err(Into::into)
}

/// 月报版：当月按天聚合。
#[tauri::command]
pub async fn get_app_month_detail(
    pool: State<'_, DbPool>,
    month_offset: i32,
    icon_process: String,
    device_id: Option<String>,
) -> Result<AppDetail, String> {
    reports::app_month_detail(
        &pool,
        month_offset,
        icon_process,
        device_filter_from_option(device_id),
    )
    .await
    .map_err(Into::into)
}

/// 拉某周 7 天的逐日汇总（每天一条）。`week_offset = 0` 本周。
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

/// 拉某周的 top 应用聚合，跨 7 天汇总。
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

/// 拉某月每天的汇总（30 / 31 条），给「月」页面热力图用。
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

/// 拉某月的 top 应用聚合，跨整月汇总。
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
