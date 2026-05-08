//! AI 总结的具体业务操作：
//!
//! - [`describe_images`]：step 1 逐图描述（支持并发）
//! - [`summarize_segment`]：step 2 段总结（纯文本调用，写库 + 返回行）
//! - [`build_step2`]：根据 settings 构造 step 2 chat 路由（本地 / 外部）
//! - [`extract_time_label`]：从截图文件名解析 `HH:MM` 标签
//!
//! 这些函数从 `DaySummaryRunner` 拎出来便于单测与代码审查；调用方传 owned
//! 数据 + Arc 的 supervisor / cancel / pool / app，避免持引用跨 await。

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::Utc;
use tauri::{AppHandle, Emitter};

use crate::ai::config::AiConfig;
use crate::ai::image::to_data_uri;
use crate::ai::llm::{ChatClient, ExternalChatClient, Step2Chat};
use crate::ai::prompt::{
    build_image_describe_user_prompt, build_system_prompt, build_user_prompt, SegmentContext,
};
use crate::ai::server::EngineSupervisor;
use crate::ai::summary_progress::{SummaryProgress, SUMMARY_PROGRESS_EVENT};
use crate::error::Result;
use crate::repo::ai_summaries::{self, ImageDescriptionRow, ScreenshotMeta, SegmentSummaryRow};
use crate::storage::DbPool;

/// 总结 LLM 输出时给段加的图片缩放上限——长边 768 px 是 vision LLM 的常见甜点：
/// 文字仍可读，token 数比原图少一半以上。
pub const SUMMARY_IMAGE_MAX_DIM: u32 = 768;

// ───────────────────────────── step 1: describe_images ─────────────────────────────

/// 单张图描述任务的 owned 上下文。
///
/// 把所有 per-image 参数打包成一个结构体，避免在 `buffer_unordered` 闭包里
/// 逐字段 clone 时遗漏新加字段；新增 image-level 参数时只改本结构体定义即可。
pub(crate) struct ImageWorkItem {
    pub index: u32,
    pub segment_idx: u32,
    pub total_segments: u32,
    pub source: String,
    pub date_str: String,
    pub prompt_lang: String,
    pub describe_system: String,
    pub model: String,
    pub img_path: String,
    pub app_display: String,
    pub category_name: Option<String>,
}

/// step 1 的图描述编排：可串行可并发。
///
/// `parallel` 决定同时跑多少张图——为了真正并发，llama-server 那边
/// `-np N` 也得开（`AiOverrides.parallel_slots` 一并传）；只开这边并发
/// 不传 `-np` 的话 llama.cpp 内部还是排队，速度不变。
///
/// 错误分两类：
/// - DB 写入失败（任意一张图）→ `?` 抛上去，整段失败
/// - data_uri / chat 失败 → log + 跳过该张，不阻塞其它
#[allow(clippy::too_many_arguments)]
pub(crate) async fn describe_images(
    pool: DbPool,
    app: AppHandle,
    supervisor: Arc<EngineSupervisor>,
    cancel: Arc<AtomicBool>,
    step1: ChatClient,
    picked: &[ScreenshotMeta],
    prompt_lang: String,
    describe_system: String,
    model: String,
    date_str: String,
    source: String,
    segment_idx: u32,
    total_segments: u32,
    parallel: usize,
) -> Result<Vec<(String, String)>> {
    let tasks = build_image_tasks(
        picked,
        &prompt_lang,
        &describe_system,
        &model,
        &date_str,
        &source,
        segment_idx,
        total_segments,
    );
    let raw = execute_parallel(tasks, step1, pool, app, supervisor, cancel, parallel).await;
    collect_descriptions(raw)
}

/// 把 `picked` 转成 `ImageWorkItem` 列表（owned 数据，丢给并发执行器消费）。
#[allow(clippy::too_many_arguments)]
fn build_image_tasks(
    picked: &[ScreenshotMeta],
    prompt_lang: &str,
    describe_system: &str,
    model: &str,
    date_str: &str,
    source: &str,
    segment_idx: u32,
    total_segments: u32,
) -> Vec<ImageWorkItem> {
    picked
        .iter()
        .enumerate()
        .map(|(i, meta)| ImageWorkItem {
            index: i as u32,
            segment_idx,
            total_segments,
            source: source.to_string(),
            date_str: date_str.to_string(),
            prompt_lang: prompt_lang.to_string(),
            describe_system: describe_system.to_string(),
            model: model.to_string(),
            img_path: meta.path.clone(),
            app_display: meta.app_display.clone(),
            category_name: meta.category_name.clone(),
        })
        .collect()
}

/// 并发执行所有 image task，收集每个的结果（None = 跳过、Some = 描述成功、Err = DB 写入失败）。
///
/// 用 `buffer_unordered` 顺序不固定；调用 [`collect_descriptions`] 排序后才能给 step 2 用。
async fn execute_parallel(
    tasks: Vec<ImageWorkItem>,
    chat: ChatClient,
    pool: DbPool,
    app: AppHandle,
    supervisor: Arc<EngineSupervisor>,
    cancel: Arc<AtomicBool>,
    parallel: usize,
) -> Vec<Result<Option<(u32, String, String)>>> {
    use futures_util::StreamExt;
    let stream = futures_util::stream::iter(tasks.into_iter().map(|item| {
        let chat = chat.clone();
        let pool = pool.clone();
        let app = app.clone();
        let supervisor = Arc::clone(&supervisor);
        let cancel = Arc::clone(&cancel);
        async move { process_one_image(item, chat, pool, app, supervisor, cancel).await }
    }));
    stream.buffer_unordered(parallel.max(1)).collect().await
}

/// 单张图：to_data_uri → chat → 写 DB → emit `image_described`。
///
/// 返回 `Ok(Some((index, time_label, description)))` 表示成功；
/// `Ok(None)` 表示该张被跳过（坏文件 / chat 失败 / 取消信号）；
/// `Err` 仅在 DB 写入失败时抛——会让整段失败。
async fn process_one_image(
    item: ImageWorkItem,
    chat: ChatClient,
    pool: DbPool,
    app: AppHandle,
    supervisor: Arc<EngineSupervisor>,
    cancel: Arc<AtomicBool>,
) -> Result<Option<(u32, String, String)>> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }

    let time_label = extract_time_label(&item.img_path);

    let data_uri = match to_data_uri(Path::new(&item.img_path), SUMMARY_IMAGE_MAX_DIM).await {
        Ok(u) => u,
        Err(e) => {
            log::warn!("跳过坏截图 {}: {e}", item.img_path);
            return Ok(None);
        }
    };

    // per-image 现拼 user prompt：每张图带上自己的应用名（+ 分类）
    let describe_user = build_image_describe_user_prompt(
        &item.prompt_lang,
        &item.app_display,
        item.category_name.as_deref(),
    );

    let single = std::slice::from_ref(&data_uri);
    let _inflight = supervisor.acquire_inference();
    let (desc, usage) = match chat
        .chat_with_images(&item.describe_system, &describe_user, single)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            log::warn!("图描述失败 {}: {e}", item.img_path);
            return Ok(None);
        }
    };

    ai_summaries::upsert_image_description(
        &pool,
        &ImageDescriptionRow {
            source: item.source.clone(),
            local_date: item.date_str.clone(),
            segment_idx: item.segment_idx,
            image_index: item.index,
            screenshot_path: item.img_path.clone(),
            description: desc.clone(),
            model: item.model.clone(),
            generated_at: Utc::now().to_rfc3339(),
            latency_ms: Some(usage.latency_ms),
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
        },
    )
    .await?;

    let mut p_img = SummaryProgress::base(
        item.source.clone(),
        item.date_str.clone(),
        "image_described",
        item.total_segments,
    );
    p_img.segment_idx = Some(item.segment_idx);
    p_img.image_index = Some(item.index);
    p_img.image_path = Some(item.img_path.clone());
    p_img.image_description = Some(desc.clone());
    p_img.latency_ms = Some(usage.latency_ms);
    p_img.prompt_tokens = usage.prompt_tokens;
    p_img.completion_tokens = usage.completion_tokens;
    if let Err(e) = app.emit(SUMMARY_PROGRESS_EVENT, &p_img) {
        log::warn!("emit {SUMMARY_PROGRESS_EVENT} 失败: {e}");
    }

    Ok(Some((item.index, time_label, desc)))
}

/// 把 [`execute_parallel`] 的乱序结果按 `image_index` 排序，
/// 丢掉 None / 抛出 Err，最终返回给 step 2 用的 `(time_label, description)` 列表。
fn collect_descriptions(
    raw: Vec<Result<Option<(u32, String, String)>>>,
) -> Result<Vec<(String, String)>> {
    let mut triples: Vec<(u32, String, String)> = Vec::with_capacity(raw.len());
    for r in raw {
        if let Some(triple) = r? {
            triples.push(triple);
        }
    }
    // buffer_unordered 完成顺序不可预期；按 image_index 排序后才能给 step 2 用
    triples.sort_by_key(|(i, _, _)| *i);
    Ok(triples.into_iter().map(|(_, t, d)| (t, d)).collect())
}

// ───────────────────────────── step 2: summarize_segment ─────────────────────────────

/// step 2 段总结：拿 step1 的描述 + top_apps 拼 prompt → 调 LLM → 落库。
///
/// 落库语义：
/// - chat 成功 → status = "ok"
/// - chat 失败 → status = "error"，error 字段塞错误描述（不抛 Err，让上层继续走）
/// - DB 写入失败 → 抛 Err，整段失败
///
/// 返回 `(已落库的行, status_str)`，让调用方拼 `segment_done` 事件 payload 用。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn summarize_segment(
    pool: &DbPool,
    step2: &Step2Chat,
    supervisor: &Arc<EngineSupervisor>,
    ai: &AiConfig,
    source: &str,
    date_str: &str,
    label: &str,
    start_hour: u8,
    end_hour: u8,
    segment_idx: u32,
    descriptions: &[(String, String)],
    top_apps: &[(String, u32, String)],
    step2_model: String,
) -> Result<(SegmentSummaryRow, &'static str)> {
    let ctx = SegmentContext {
        label,
        start_hour,
        end_hour,
        top_apps,
        image_descriptions: descriptions,
    };
    let system = build_system_prompt(ai);
    let user_text = build_user_prompt(ai, &ctx);

    // 本地 step2 走自家引擎，需要 acquire 防止 watcher 在请求中途 stop；
    // 云端 step2 (External) 不动 supervisor，不 acquire。
    let _inflight = step2.is_local().then(|| supervisor.acquire_inference());
    let (row, status_str): (SegmentSummaryRow, &'static str) =
        match step2.chat(&system, &user_text, &[]).await {
            // step 2 是纯文本调用，本表只关心 content；usage 暂不落 ai_summaries
            // （需要时未来加列）。落库的 model 用 step2_model——本地是 GGUF 文件名，
            // 外部是用户填的云端模型 ID（如 gpt-4o-mini）
            Ok((content, _usage)) => (
                SegmentSummaryRow {
                    source: source.to_string(),
                    local_date: date_str.to_string(),
                    segment_idx,
                    label: label.to_string(),
                    start_hour,
                    end_hour,
                    content,
                    model: step2_model,
                    status: "ok".to_string(),
                    error: None,
                    generated_at: Utc::now().to_rfc3339(),
                },
                "ok",
            ),
            Err(e) => (
                SegmentSummaryRow {
                    source: source.to_string(),
                    local_date: date_str.to_string(),
                    segment_idx,
                    label: label.to_string(),
                    start_hour,
                    end_hour,
                    content: String::new(),
                    model: step2_model,
                    status: "error".to_string(),
                    error: Some(e.to_string()),
                    generated_at: Utc::now().to_rfc3339(),
                },
                "error",
            ),
        };

    ai_summaries::upsert_segment(pool, &row).await?;
    Ok((row, status_str))
}

// ───────────────────────────── 公共小工具 ─────────────────────────────

/// 从截图绝对路径里解析本地时间标签 `HH:MM`。
///
/// 文件名约定（capture/screenshot.rs:48 写入）：`HHMMSS_NNN.jpg`，按本机时区。
/// 解析失败时回退到 "??:??"——不能让缺时间戳阻塞段总结。
pub(crate) fn extract_time_label(screenshot_path: &str) -> String {
    let stem = std::path::Path::new(screenshot_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    // 取下划线前部分 "HHMMSS"
    let head = stem.split('_').next().unwrap_or("");
    if head.len() == 6 && head.chars().all(|c| c.is_ascii_digit()) {
        let hh = &head[0..2];
        let mm = &head[2..4];
        return format!("{hh}:{mm}");
    }
    "??:??".to_string()
}

/// 根据 settings.ai.external_enabled 构造 step 2 的 chat 路由。
///
/// - false：[`Step2Chat::Local`]——本地端口；`local_model_label` 是当前引擎实际加载的
///   GGUF 文件名（即 `effective_summary_main`，跟 step 1 可能不同），用作
///   `model_label()` 落库 + chat completions 请求的 model 字段
/// - true：[`Step2Chat::External`] 包一个新建的 [`ExternalChatClient`]，
///   走用户填的 endpoint / model / api_key
///
/// 外部 client 构造失败（endpoint 空、model 空）会向上抛——这种情况说明用户
/// 开了 toggle 但没填配置，让顶层错误条直接显示让他去填。
pub(crate) fn build_step2(
    ai: &AiConfig,
    local_port: u16,
    local_model_label: &str,
) -> Result<Step2Chat> {
    if ai.external_enabled {
        let ext = ExternalChatClient::new(&ai.endpoint, ai.model.clone(), ai.api_key.clone())?;
        Ok(Step2Chat::External(ext))
    } else {
        Ok(Step2Chat::Local(ChatClient::new(
            local_port,
            local_model_label,
        )?))
    }
}

/// 让某段直接落 `skipped_no_screenshots` 行 —— 没截图时 step 1/2 都不跑，直接补档。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn upsert_skipped_segment(
    pool: &DbPool,
    source: &str,
    date_str: &str,
    segment_idx: u32,
    label: &str,
    start_hour: u8,
    end_hour: u8,
    model: String,
) -> Result<()> {
    ai_summaries::upsert_segment(
        pool,
        &SegmentSummaryRow {
            source: source.to_string(),
            local_date: date_str.to_string(),
            segment_idx,
            label: label.to_string(),
            start_hour,
            end_hour,
            content: String::new(),
            model,
            status: "skipped_no_screenshots".to_string(),
            error: None,
            generated_at: Utc::now().to_rfc3339(),
        },
    )
    .await
}
