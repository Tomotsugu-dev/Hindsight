//! AI 总结子模块的 façade —— 把 [`DaySummaryRunner`] / [`AiOverrides`] /
//! [`SummaryProgress`] / [`SUMMARY_PROGRESS_EVENT`] 重导出到一处方便外部 import。
//!
//! 实际实现拆在：
//! - [`crate::ai::summary_overrides`]：调试覆盖结构体 `AiOverrides`
//! - [`crate::ai::summary_progress`]：进度事件 payload `SummaryProgress` + 事件名
//! - [`crate::ai::summary_operations`]：活动时间线合成 + 段总结业务函数
//! - [`crate::ai::summary_runner`]：编排核心 `DaySummaryRunner`

pub use crate::ai::summary_overrides::AiOverrides;
pub use crate::ai::summary_progress::{SummaryProgress, SUMMARY_PROGRESS_EVENT};
pub use crate::ai::summary_runner::DaySummaryRunner;
pub use crate::ai::weekly_runner::{
    precheck_week, WeekPrecheckResp, WeekSummaryRunner, WEEKLY_SOURCE,
};
// `WeekPrecheckDay` 不在 façade re-export 里——它只通过 `WeekPrecheckResp.days` 字段
// 间接出现在 JSON payload 中，前端 TS 自己 declare 对应类型；后端 crate 内部如有需要
// 直接 `use crate::ai::weekly_runner::WeekPrecheckDay`。
