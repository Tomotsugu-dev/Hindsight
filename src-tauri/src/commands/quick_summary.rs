//! 快速模板总结的 Tauri 命令层。
//!
//! 跟 [`ai_summary`](super::ai_summary) 互补：没有"开始/取消/进度事件"——纯查询，瞬时返回。
//! 真实计算在 [`crate::quick_summary`]。

use tauri::State;

use crate::quick_summary::{
    self, QuickDaySummary, QuickMonthSummary, QuickWeekSummary,
};
use crate::repo::reports::device_filter_from_option;
use crate::storage::DbPool;

/// 拉某天的快速模板总结。`day_offset = 0` 今天，-1 昨天。
#[tauri::command]
pub async fn get_quick_day_summary(
    pool: State<'_, DbPool>,
    day_offset: i32,
    device_id: Option<String>,
) -> Result<QuickDaySummary, String> {
    quick_summary::compute_day(&pool, day_offset, device_filter_from_option(device_id))
        .await
        .map_err(Into::into)
}

/// 拉某周的快速模板总结。`week_offset = 0` 本周，-1 上周。
#[tauri::command]
pub async fn get_quick_week_summary(
    pool: State<'_, DbPool>,
    week_offset: i32,
    device_id: Option<String>,
) -> Result<QuickWeekSummary, String> {
    quick_summary::compute_week(&pool, week_offset, device_filter_from_option(device_id))
        .await
        .map_err(Into::into)
}

/// 拉某月的快速模板总结。`month_offset = 0` 本月，-1 上月。
#[tauri::command]
pub async fn get_quick_month_summary(
    pool: State<'_, DbPool>,
    month_offset: i32,
    device_id: Option<String>,
) -> Result<QuickMonthSummary, String> {
    quick_summary::compute_month(&pool, month_offset, device_filter_from_option(device_id))
        .await
        .map_err(Into::into)
}
