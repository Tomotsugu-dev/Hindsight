//! AI 总结的 Tauri 命令层（Phase 1B-γ）。
//!
//! 命令体本身只做参数校验 + 错误归类 + emit 顶层 error；真正的编排在
//! [`crate::ai::summary_runner::DaySummaryRunner`]。
//! `SummaryCancel` 是 lib.rs 里 manage 的全局单例，给 `cancel_day_summary` 调。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::NaiveDate;
use tauri::{AppHandle, Emitter, State};

use crate::ai::server::EngineSupervisor;
use crate::ai::summary::{
    AiOverrides, DaySummaryRunner, SummaryProgress, SUMMARY_PROGRESS_EVENT,
};
use crate::repo::ai_summaries::{self, ImageDescriptionRow, SegmentSummaryRow};
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

/// 跑某天的全部段总结。
///
/// 命令本体异步等到所有段完成才 resolve（或 cancel 后早 return）；
/// 期间通过 [`SUMMARY_PROGRESS_EVENT`] 流式推进度，前端 listen 边跑边渲染。
///
/// 重复触发：前端 UI 应防重复点；后端不加锁，但启动前会 reset cancel 标记，
/// 所以理论上后到的 generate 不会被前一次的 cancel 干掉。
#[tauri::command]
pub async fn generate_day_summary(
    app: AppHandle,
    pool: State<'_, DbPool>,
    supervisor: State<'_, Arc<EngineSupervisor>>,
    cancel: State<'_, SummaryCancel>,
    date: String,
    force_refresh: bool,
    device_id: Option<String>,
    overrides: Option<AiOverrides>,
    // "daily"（DailyTab，默认）/ "debug"（DebugTab）—— PK 级隔离两支数据
    source: Option<String>,
    // step1_only=true：跳过 step 2 段总结。「仅生成图片描述」按钮触发。
    // step2_only=true：跳过 step 1，从 DB 读已存图描述跑 step 2。「仅生成段总结」按钮触发。
    // 互斥；默认都 false，daily 路径走完整 step1+step2。
    step1_only: Option<bool>,
    step2_only: Option<bool>,
) -> Result<(), String> {
    let parsed_date = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
        .map_err(|e| format!("日期格式应为 YYYY-MM-DD：{e}"))?;
    let device = device_filter_from_option(device_id);
    let source = source.unwrap_or_else(|| "daily".to_string());
    let step1_only = step1_only.unwrap_or(false);
    let step2_only = step2_only.unwrap_or(false);

    cancel.0.store(false, Ordering::Relaxed);
    let runner = DaySummaryRunner::new(
        (*pool).clone(),
        Arc::clone(&supervisor),
        app.clone(),
        Arc::clone(&cancel.0),
    );

    if let Err(e) = runner
        .run(&source, parsed_date, device, force_refresh, overrides, step1_only, step2_only)
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
#[tauri::command]
pub async fn retry_summary_segment(
    app: AppHandle,
    pool: State<'_, DbPool>,
    supervisor: State<'_, Arc<EngineSupervisor>>,
    cancel: State<'_, SummaryCancel>,
    date: String,
    segment_idx: u32,
    device_id: Option<String>,
    overrides: Option<AiOverrides>,
    source: Option<String>,
) -> Result<(), String> {
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

/// 重跑单张图的描述——调试 tab 的"重跑"按钮调这个。
///
/// 不动段总结、其它图描述；只覆盖 ai_image_descriptions 一行。
/// 期间走全局 SUMMARY_PROGRESS_EVENT 流的 `image_described` phase 推一条事件。
#[tauri::command]
pub async fn retry_single_image_description(
    app: AppHandle,
    pool: State<'_, DbPool>,
    supervisor: State<'_, Arc<EngineSupervisor>>,
    cancel: State<'_, SummaryCancel>,
    date: String,
    segment_idx: u32,
    image_index: u32,
    overrides: Option<AiOverrides>,
    source: Option<String>,
) -> Result<(), String> {
    let parsed_date = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
        .map_err(|e| format!("日期格式应为 YYYY-MM-DD：{e}"))?;
    let source = source.unwrap_or_else(|| "daily".to_string());

    let runner = DaySummaryRunner::new(
        (*pool).clone(),
        Arc::clone(&supervisor),
        app,
        Arc::clone(&cancel.0),
    );
    runner
        .retry_one_image_description(&source, parsed_date, segment_idx, image_index, overrides)
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

/// 删除某天的全部 AI 产物——同时清 `ai_summaries` 段总结 + `ai_image_descriptions`
/// 逐图描述。DailyTab 删除按钮用。
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

/// 只删某天的逐图描述，**不**动段总结。
/// 调试 tab「逐图描述」Section header 删除按钮用。
#[tauri::command]
pub async fn clear_day_image_descriptions(
    pool: State<'_, DbPool>,
    date: String,
    source: Option<String>,
) -> Result<(), String> {
    let src = source.unwrap_or_else(|| "daily".to_string());
    ai_summaries::clear_day_image_descriptions_only(&pool, &src, &date)
        .await
        .map_err(String::from)
}

/// 只删某天的段总结，**不**动逐图描述。
/// 调试 tab「段总结结果」Section header 删除按钮用。
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

/// 拉某段所有"逐图描述"——调试 tab 渲染列表用。两步生成 step 1 的产物。
#[tauri::command]
pub async fn get_segment_image_descriptions(
    pool: State<'_, DbPool>,
    date: String,
    segment_idx: u32,
    source: Option<String>,
) -> Result<Vec<ImageDescriptionRow>, String> {
    let src = source.unwrap_or_else(|| "daily".to_string());
    ai_summaries::get_segment_image_descriptions(&pool, &src, &date, segment_idx)
        .await
        .map_err(String::from)
}

/// 拉某天所有段的"逐图描述"——调试 tab 一次性渲染整日时用。
#[tauri::command]
pub async fn get_day_image_descriptions(
    pool: State<'_, DbPool>,
    date: String,
    source: Option<String>,
) -> Result<Vec<ImageDescriptionRow>, String> {
    let src = source.unwrap_or_else(|| "daily".to_string());
    ai_summaries::get_day_image_descriptions(&pool, &src, &date)
        .await
        .map_err(String::from)
}
