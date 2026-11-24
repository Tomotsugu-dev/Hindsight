//! 日报、周报与后台定时调度。
//!
//! 外部模块只使用这里导出的入口。

mod daily;
mod model;
mod progress;
mod schedule;
mod weekly;

#[cfg(test)]
mod test_util;

pub use daily::{DaySummaryRunner, DAILY_SOURCE};
pub use progress::{ProgressSink, SummaryProgress, SUMMARY_PROGRESS_EVENT};
pub use schedule::spawn as spawn_schedule;
pub use weekly::{precheck_week, WeekPrecheckResp, WeekSummaryRunner, WEEKLY_SOURCE};
