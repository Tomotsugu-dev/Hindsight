//! AI 总结的 Tauri 命令层（Phase 1B-γ）。
//!
//! 命令体本身只做参数校验 + 错误归类 + emit 顶层 error；真正的编排在
//! [`crate::ai::summary_runner::DaySummaryRunner`]。
//! `SummaryCancel` 是 lib.rs 里 manage 的全局单例，给 `cancel_day_summary` 调。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{Datelike, Duration, NaiveDate};
use tauri::{AppHandle, Emitter, State};

use crate::ai::server::EngineSupervisor;
use crate::ai::summary::{
    precheck_week, AiOverrides, DaySummaryRunner, SummaryProgress, WeekPrecheckResp,
    WeekSummaryRunner, SUMMARY_PROGRESS_EVENT, WEEKLY_SOURCE,
};
use crate::repo::ai_summaries::{self, SegmentSummaryRow};
use crate::repo::reports::device_filter_from_option;
use crate::storage::DbPool;

/// AI 总结流程的取消信号——管 lib.rs `manage` 的全局单例。
///
/// 同一时刻只允许一个 generate_day_summary 在跑（前端 UI 不让重复点）；
/// cancel_day_summary 把内部 AtomicBool 设 true，summary_runner 在每段循环检测到后
/// 就停下来 Ok(()) 退出（不能中断已经在路上的单段 LLM 请求）。
pub struct SummaryCancel(pub Arc<AtomicBool>);

impl Default for SummaryCancel {
    fn default() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
}

/// 全局 AI 生成互斥：daily / weekly / 段重试 / 单图重试同一时刻只允许一个在跑。
///
/// 之前后端零互斥、且所有入口共享一个 cancel 标记并在启动时清 false——正在停止的
/// 旧任务会被新任务"复活"，两个 runner 抢同一个 EngineSupervisor 互相 stop/restart；
/// 周报跑着时在日报页点 Stop 也会连带取消周报。现在：入口 try_lock 失败直接返回
/// `[AI_RUN_BUSY]` 错误码（前端 aiErrors.ts 本地化），cancel 的 reset 只发生在
/// 持锁之后——旧任务必然已退出，清标记不会误伤任何在途任务。
#[derive(Default)]
pub struct RunLock(pub tokio::sync::Mutex<()>);

/// try_lock 失败时的稳定错误码；`[]` 前缀供前端识别本地化，正文是英文兜底。
const AI_RUN_BUSY: &str = "[AI_RUN_BUSY] another AI generation task is already running";

/// 跑某天的全部段总结。
///
/// 命令本体异步等到所有段完成才 resolve（或 cancel 后早 return）；
/// 期间通过 [`SUMMARY_PROGRESS_EVENT`] 流式推进度，前端 listen 边跑边渲染。
///
/// 重复触发：前端 UI 应防重复点；后端不加锁，但启动前会 reset cancel 标记，
/// 所以理论上后到的 generate 不会被前一次的 cancel 干掉。
// Tauri 命令 State 注入 + 前端 payload 字段一起算，参数数 > 7 是常态；
// 拆 struct 反而让命令参数 schema 变嵌套，前端 invoke 调用更冗长
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn generate_day_summary(
    app: AppHandle,
    pool: State<'_, DbPool>,
    supervisor: State<'_, Arc<EngineSupervisor>>,
    cancel: State<'_, SummaryCancel>,
    run_lock: State<'_, RunLock>,
    date: String,
    force_refresh: bool,
    device_id: Option<String>,
    overrides: Option<AiOverrides>,
    // "daily"（DailyTab，默认）/ "debug"（DebugTab）—— PK 级隔离两支数据
    source: Option<String>,
) -> Result<(), String> {
    let Ok(_run) = run_lock.0.try_lock() else {
        return Err(AI_RUN_BUSY.to_string());
    };
    let parsed_date = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
        .map_err(|e| format!("日期格式应为 YYYY-MM-DD：{e}"))?;
    let device = device_filter_from_option(device_id);
    let source = source.unwrap_or_else(|| "daily".to_string());

    cancel.0.store(false, Ordering::Relaxed);
    let runner = DaySummaryRunner::new(
        (*pool).clone(),
        Arc::clone(&supervisor),
        app.clone(),
        Arc::clone(&cancel.0),
    );

    if let Err(e) = runner
        .run(&source, parsed_date, device, force_refresh, overrides)
        .await
    {
        // 顶层失败也 emit 一条 error，前端 UI 能 toast
        let mut p = SummaryProgress::base(source.clone(), date.clone(), "error", 0);
        p.message = Some(e.to_string());
        let _ = app.emit(SUMMARY_PROGRESS_EVENT, &p);
        return Err(e.to_string());
    }
    Ok(())
}

/// 单段重试——只重跑指定一段，不动其它段。复用 supervisor 已经在跑的 server。
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn retry_summary_segment(
    app: AppHandle,
    pool: State<'_, DbPool>,
    supervisor: State<'_, Arc<EngineSupervisor>>,
    cancel: State<'_, SummaryCancel>,
    run_lock: State<'_, RunLock>,
    date: String,
    segment_idx: u32,
    device_id: Option<String>,
    overrides: Option<AiOverrides>,
    source: Option<String>,
) -> Result<(), String> {
    let Ok(_run) = run_lock.0.try_lock() else {
        return Err(AI_RUN_BUSY.to_string());
    };
    let parsed_date = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
        .map_err(|e| format!("日期格式应为 YYYY-MM-DD：{e}"))?;
    let device = device_filter_from_option(device_id);
    let source = source.unwrap_or_else(|| "daily".to_string());

    cancel.0.store(false, Ordering::Relaxed);
    let runner = DaySummaryRunner::new(
        (*pool).clone(),
        Arc::clone(&supervisor),
        app,
        Arc::clone(&cancel.0),
    );
    runner
        .run_one_segment_only(&source, parsed_date, segment_idx, device, overrides)
        .await
        .map_err(String::from)
}

/// 设取消标记——下一段循环开头会感知到然后 Ok(()) 提早返回。
/// 已经在路上的单段 LLM 请求**不会**被中断（一段 30-180s 必须跑完）。
#[tauri::command]
pub async fn cancel_day_summary(cancel: State<'_, SummaryCancel>) -> Result<(), String> {
    cancel.0.store(true, Ordering::Relaxed);
    Ok(())
}

/// 拉某天已经落库的总结。前端进页面时调一次：有就直接渲染，没有就显示
/// "点击生成"按钮。
#[tauri::command]
pub async fn get_day_summary(
    pool: State<'_, DbPool>,
    date: String,
    source: Option<String>,
) -> Result<Vec<SegmentSummaryRow>, String> {
    let src = source.unwrap_or_else(|| "daily".to_string());
    ai_summaries::get_day(&pool, &src, &date)
        .await
        .map_err(String::from)
}

/// 删除某天的全部 AI 产物（段总结 + 历史遗留的逐图描述行）。DailyTab 删除按钮用。
#[tauri::command]
pub async fn clear_day_summary(
    pool: State<'_, DbPool>,
    date: String,
    source: Option<String>,
) -> Result<(), String> {
    let src = source.unwrap_or_else(|| "daily".to_string());
    ai_summaries::clear_day(&pool, &src, &date)
        .await
        .map_err(String::from)
}

/// 只删某天的段总结。调试 tab「段总结结果」Section header 删除按钮用。
#[tauri::command]
pub async fn clear_day_segment_summaries(
    pool: State<'_, DbPool>,
    date: String,
    source: Option<String>,
) -> Result<(), String> {
    let src = source.unwrap_or_else(|| "daily".to_string());
    ai_summaries::clear_day_summaries_only(&pool, &src, &date)
        .await
        .map_err(String::from)
}

// ───────────────────────────── weekly ─────────────────────────────
//
// 周报路径跟 daily 完全独立：单步纯文本 LLM 调用，不过 vision，不切段。
// 命令体只做参数校验 + 周一对齐 + 错误归类，编排在 [`WeekSummaryRunner`]。
// 取消信号跟 daily **共用**全局 `SummaryCancel`——`cancel_day_summary` 会同时取消
// 在跑的 weekly。前端 UI 保证两个 tab 不并发触发，简化心智模型。

/// 把任意"该周内的某天"对齐到当周周一。
/// 前端传 `weekStart` 应当本身就是周一字符串，但兼容性兜底（用户后续可能从月历跳转传任意日）。
fn align_to_monday(date: NaiveDate) -> NaiveDate {
    let dow = date.weekday().num_days_from_monday() as i64;
    date - Duration::days(dow)
}

/// 跑某一周的周报。
///
/// `week_start` 推荐传周一日期 "YYYY-MM-DD"；不是周一时后端自动对齐到当周周一。
/// 命令本体异步等到 LLM 调用完毕（含 DB 写入）才 resolve；期间通过
/// [`SUMMARY_PROGRESS_EVENT`] 流式推 `engine_starting` / `summarizing` /
/// `segment_done` / `all_done` / `error`，前端按 source="weekly" 过滤接收。
///
/// `allow_missing_days`:
/// - false = 老行为：缺日报就 error 早返回。
/// - true = 前端已展示"部分/整周日报缺失"确认弹框且用户选了"继续生成"：
///   - 部分缺：用当日 top apps 顶替进入 prompt
///   - 整周缺但有 activity：仅基于整周 + 每日 top apps 做简化分析
///
/// Tauri 命令参数不支持 doc 注释或 `#[serde(default)]`，前端必须显式传布尔值。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn generate_week_summary(
    app: AppHandle,
    pool: State<'_, DbPool>,
    supervisor: State<'_, Arc<EngineSupervisor>>,
    cancel: State<'_, SummaryCancel>,
    run_lock: State<'_, RunLock>,
    week_start: String,
    force_refresh: bool,
    allow_missing_days: bool,
) -> Result<(), String> {
    let Ok(_run) = run_lock.0.try_lock() else {
        return Err(AI_RUN_BUSY.to_string());
    };
    let parsed_date = NaiveDate::parse_from_str(&week_start, "%Y-%m-%d")
        .map_err(|e| format!("日期格式应为 YYYY-MM-DD：{e}"))?;
    let monday = align_to_monday(parsed_date);
    let monday_str = monday.format("%Y-%m-%d").to_string();

    cancel.0.store(false, Ordering::Relaxed);
    let runner = WeekSummaryRunner::new(
        (*pool).clone(),
        Arc::clone(&supervisor),
        app.clone(),
        Arc::clone(&cancel.0),
    );

    if let Err(e) = runner.run(monday, force_refresh, allow_missing_days).await {
        // 顶层失败也 emit 一条 error 让前端 toast——和 daily 同款 UX
        let mut p =
            SummaryProgress::base(WEEKLY_SOURCE.to_string(), monday_str.clone(), "error", 1);
        p.message = Some(e.to_string());
        let _ = app.emit(SUMMARY_PROGRESS_EVENT, &p);
        return Err(e.to_string());
    }
    Ok(())
}

/// 周报生成前预览：返回该周 7 天每天的"是否有日报 / 是否有活动"。
///
/// 前端"点击生成"时先调这个：
/// - 全 7 天都有日报 → 直接 generate_week_summary
/// - 部分天缺日报 → 弹"部分日期没有日报"确认，用户同意后再 generate_week_summary(allow_missing_days=true)
/// - 整周都没日报 → 弹"本周还没有任何日报"确认（含 days_activity_only 提示），同上
///
/// `week_start` 不是周一时自动对齐到当周周一。
#[tauri::command]
pub async fn precheck_week_summary(
    pool: State<'_, DbPool>,
    week_start: String,
) -> Result<WeekPrecheckResp, String> {
    let parsed_date = NaiveDate::parse_from_str(&week_start, "%Y-%m-%d")
        .map_err(|e| format!("日期格式应为 YYYY-MM-DD：{e}"))?;
    let monday = align_to_monday(parsed_date);
    precheck_week(&pool, monday).await.map_err(String::from)
}

/// 拉某周已落库的周报行（最多一行）。前端进周报 tab 时调一次。
/// `week_start` 不是周一时自动对齐。
#[tauri::command]
pub async fn get_week_summary(
    pool: State<'_, DbPool>,
    week_start: String,
) -> Result<Option<SegmentSummaryRow>, String> {
    let parsed_date = NaiveDate::parse_from_str(&week_start, "%Y-%m-%d")
        .map_err(|e| format!("日期格式应为 YYYY-MM-DD：{e}"))?;
    let monday = align_to_monday(parsed_date);
    let monday_str = monday.format("%Y-%m-%d").to_string();
    let rows = ai_summaries::get_day(&pool, WEEKLY_SOURCE, &monday_str)
        .await
        .map_err(String::from)?;
    Ok(rows.into_iter().next())
}

/// 删除某周已落库的周报行。前端"删除"按钮调。
#[tauri::command]
pub async fn clear_week_summary(pool: State<'_, DbPool>, week_start: String) -> Result<(), String> {
    let parsed_date = NaiveDate::parse_from_str(&week_start, "%Y-%m-%d")
        .map_err(|e| format!("日期格式应为 YYYY-MM-DD：{e}"))?;
    let monday = align_to_monday(parsed_date);
    let monday_str = monday.format("%Y-%m-%d").to_string();
    // 周报只占 ai_summaries 一行（segment_idx=0），用 clear_day_summaries_only
    // 不动 ai_image_descriptions（weekly 本来就没写过那张表）。
    ai_summaries::clear_day_summaries_only(&pool, WEEKLY_SOURCE, &monday_str)
        .await
        .map_err(String::from)
}
