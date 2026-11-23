//! 日报和周报。外面只用这里导出的东西。

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
