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

use tauri::{AppHandle, Emitter};

use crate::ai::config::AiConfig;
use crate::ai::image::to_data_uri;
use crate::ai::llm::{ChatClient, ExternalChatClient, Step2Chat};
use crate::ai::prompt::{
    build_image_describe_user_prompt, build_system_prompt, build_user_prompt, SegmentContext,
};
use crate::ai::server::EngineSupervisor;
use crate::ai::summary_progress::{SummaryProgress, SUMMARY_PROGRESS_EVENT};
use crate::capture::privacy;
use crate::error::Result;
use crate::repo::ai_summaries::{self, ImageDescriptionRow, ScreenshotMeta, SegmentSummaryRow};
use crate::repo::reports::DeviceFilter;
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};

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
            generated_at: utc_now_rfc3339(),
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
                    generated_at: utc_now_rfc3339(),
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
                    generated_at: utc_now_rfc3339(),
                },
                "error",
            ),
        };

    // upsert 失败不让整轮 daily 抛飞——磁盘满 / DB lock 时 row 写不进去也得让上层
    // emit segment_done 把当前 row 推给前端（至少能看到红色 error badge + 错误描述）。
    // 老逻辑 .await? 会 propagate，让后续段连 emit 都发不出，前端整页空白。
    if let Err(e) = ai_summaries::upsert_segment(pool, &row).await {
        log::error!(
            "ai_summaries upsert 失败（段 {} status={}）：{e}",
            row.segment_idx,
            row.status,
        );
    }
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

/// 根据 [`AiConfig::summary_use_cloud`] 构造 step 2 的 chat 路由。
///
/// - false：[`Step2Chat::Local`]——本地端口；`local_model_label` 是当前引擎实际加载的
///   GGUF 文件名（即 `effective_summary_main`，跟 step 1 可能不同），用作
///   `model_label()` 落库 + chat completions 请求的 model 字段
/// - true：[`Step2Chat::External`] 包一个新建的 [`ExternalChatClient`]，
///   走用户填的 endpoint / model / api_key
///
/// `summary_use_cloud()` 同时检查 `summary_main == SUMMARY_CLOUD_SENTINEL` 且
/// `external_enabled = true` —— 用户在 Models tab 的云端卡点了 Text + 云端 API tab
/// 启用 toggle 开着，两个条件都满足才路由到 External。
///
/// 外部 client 构造失败（endpoint 空、model 空）会向上抛——这种情况说明用户
/// 选了 cloud 但配置不全，让顶层错误条直接显示让他去填。
pub(crate) fn build_step2(
    ai: &AiConfig,
    local_port: u16,
    local_model_label: &str,
) -> Result<Step2Chat> {
    let max_tokens = ai.summary_max_tokens();
    if ai.summary_use_cloud() {
        let ext = ExternalChatClient::new(
            &ai.endpoint,
            ai.model.clone(),
            ai.api_key.clone(),
            max_tokens,
        )?;
        Ok(Step2Chat::External(ext))
    } else {
        Ok(Step2Chat::Local(ChatClient::new(
            local_port,
            local_model_label,
            max_tokens,
        )?))
    }
}

/// 让某段直接落 `skipped_no_activity` 行 —— 既无截图也无 activities 时的真兜底。
///
/// "无截图但 activities 有数据" 的旧 `"skipped_no_screenshots"` 状态值仍由
/// 数据库里的历史行使用；新一代生成路径会先尝试合成描述再走 step 2，剩下的
/// 真空段才落本状态。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn upsert_skipped_no_activity(
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
            status: "skipped_no_activity".to_string(),
            error: None,
            generated_at: utc_now_rfc3339(),
        },
    )
    .await
}

// ───────── activities 兜底：截图为空时从 activities 表合成段描述 ─────────

/// 当一段时间没有截图时，从 `activities` 表合成「按小时」的活动描述，
/// 形状与 step 1 的 `(time_label, description)` 一致，可直接喂给 [`summarize_segment`]。
///
/// 用于 step 1 路径 [`crate::ai::summary_runner::DaySummaryRunner::run_one_segment`]
/// 的 `metas.is_empty()` 分支兜底——只要 activities 还有行（用户关了截图 / 截图过期
/// 被清 / OS 权限抖动），就把窗口标题 + 时长汇总成一段文字喂给 step 2，避免段卡片
/// 在 UI 上彻底空白。
///
/// SQL 语义：
/// - `local_date` + `local_hour` 在 `[start_hour, end_hour)` 范围内
/// - 仅取 `duration_secs > 0` 的已 seal 行（unsealed 心跳行 dur=0 排除）
/// - 复用 [`crate::repo::ai_summaries::list_segment_screenshots`] 的
///   `excluded_categories` 与 [`DeviceFilter`] 过滤模式
///
/// 隐私行为：window_title 命中 `privacy_app_keywords`（子串忽略大小写）→ 替换成
/// `[私密]`，app 名 + 时长照常贡献。URL 关键词不参与（activities 表无 URL 字段）。
///
/// 返回的 `Vec` 元素形状：`(time_label, hour_summary_text)`，如：
///   `("09:00-10:00", "VSCode 45 分钟（DataTab.tsx、ModelsSection.tsx）· Chrome 10 分钟…")`
///
/// 空小时（该小时无任何活动）不产生条目；整段无活动 → 返回 `vec![]`，
/// 调用方应据此回退到 `skipped_no_activity`。
pub(crate) async fn build_synthetic_descriptions_from_activities(
    pool: &DbPool,
    date_str: &str,
    start_hour: u8,
    end_hour: u8,
    excluded_categories: &[String],
    device: &DeviceFilter,
    privacy_app_keywords: &[String],
) -> Result<Vec<(String, String)>> {
    use rusqlite::ToSql;

    let date = date_str.to_string();
    let excluded: Vec<String> = excluded_categories.to_vec();
    let dev = device.clone();
    let rows: Vec<(u8, String, Option<String>, i64)> = pool
        .0
        .call(move |conn| {
            let placeholders = if excluded.is_empty() {
                String::new()
            } else {
                let marks = vec!["?"; excluded.len()].join(",");
                format!(" AND COALESCE(c.id, 'other') NOT IN ({})", marks)
            };
            let sql = format!(
                "SELECT a.local_hour,
                        COALESCE(g.display_name, a.process_name) AS app_display,
                        a.window_title,
                        a.duration_secs
                   FROM activities a
              LEFT JOIN app_group_members gm
                     ON gm.process_name = a.process_name AND gm.deleted_at IS NULL
              LEFT JOIN app_groups g
                     ON g.id = gm.group_id AND g.deleted_at IS NULL
              LEFT JOIN categories c
                     ON c.id = g.category_id AND c.deleted_at IS NULL
                  WHERE a.local_date = ?
                    AND a.local_hour >= ?
                    AND a.local_hour < ?
                    AND a.duration_secs > 0
                    {}
                    {}
                  ORDER BY a.local_hour ASC, a.duration_secs DESC",
                placeholders,
                dev.sql_clause(),
            );
            let mut params: Vec<&dyn ToSql> = Vec::new();
            params.push(&date);
            let sh = start_hour as i64;
            let eh = end_hour as i64;
            params.push(&sh);
            params.push(&eh);
            for cat in &excluded {
                params.push(cat);
            }
            if let Some(extra) = dev.extra_param() {
                params.push(extra);
            }
            let mut stmt = conn.prepare(&sql).db()?;
            let it = stmt
                .query_map(params.as_slice(), |r| {
                    let hour: i64 = r.get(0)?;
                    let app: String = r.get(1)?;
                    let title: Option<String> = r.get(2)?;
                    let dur: i64 = r.get(3)?;
                    Ok((hour as u8, app, title, dur))
                })
                .db()?;
            let mut out = Vec::new();
            for row in it {
                out.push(row.db()?);
            }
            Ok(out)
        })
        .await?;

    Ok(format_synthetic_hours(rows, privacy_app_keywords))
}

/// 把 SQL 行（hour, app, title, dur）按小时 / 应用聚合成 `(time_label, desc)` 列表。
/// 抽函数让单测可以纯粹喂结构化数据，不依赖 SQLite。
fn format_synthetic_hours(
    rows: Vec<(u8, String, Option<String>, i64)>,
    privacy_app_keywords: &[String],
) -> Vec<(String, String)> {
    use std::collections::BTreeMap;

    // hour → app → (total_secs, Vec<title>)
    // BTreeMap 让 hour 升序、app 名稳定；app 内时长聚合后再排序。
    let mut by_hour: BTreeMap<u8, BTreeMap<String, (i64, Vec<String>)>> = BTreeMap::new();
    for (hour, app, title, dur) in rows {
        let app_bucket = by_hour
            .entry(hour)
            .or_default()
            .entry(app)
            .or_insert((0i64, Vec::new()));
        app_bucket.0 += dur;
        if let Some(t) = title {
            let trimmed = t.trim();
            if !trimmed.is_empty() {
                let display = if privacy::matches_any(trimmed, privacy_app_keywords) {
                    "[私密]".to_string()
                } else {
                    trimmed.to_string()
                };
                app_bucket.1.push(display);
            }
        }
    }

    let mut result = Vec::new();
    for (hour, apps_map) in by_hour {
        let mut apps: Vec<(String, i64, Vec<String>)> = apps_map
            .into_iter()
            .map(|(app, (secs, titles))| (app, secs, titles))
            .collect();
        // 按总时长降序，时长相同时按 app 名稳定
        apps.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

        let mut major_parts: Vec<String> = Vec::new();
        let mut minor_count: u32 = 0;
        let mut minor_secs: i64 = 0;
        for (app, secs, titles) in apps {
            if secs < 60 {
                minor_count += 1;
                minor_secs += secs;
                continue;
            }
            let dur_str = format_secs_human(secs);
            let titles_str = pick_titles(&titles);
            let part = if titles_str.is_empty() {
                format!("{app} {dur_str}")
            } else {
                format!("{app} {dur_str}（{titles_str}）")
            };
            major_parts.push(part);
        }
        if minor_count > 0 {
            major_parts.push(format!("其它（{minor_count} 项 · {minor_secs}s）"));
        }
        if major_parts.is_empty() {
            continue;
        }
        let label = format!("{hour:02}:00-{:02}:00", hour.saturating_add(1));
        let desc = major_parts.join(" · ");
        result.push((label, desc));
    }
    result
}

fn format_secs_human(secs: i64) -> String {
    let minutes = secs / 60;
    if minutes >= 1 {
        format!("{minutes} 分钟")
    } else {
        format!("{secs}s")
    }
}

/// 去重保序后按字符数降序取前 3 个，"、" 分隔。
fn pick_titles(titles: &[String]) -> String {
    use std::collections::HashSet;
    let mut seen: HashSet<&str> = HashSet::new();
    let mut unique: Vec<&str> = Vec::new();
    for t in titles {
        if seen.insert(t.as_str()) {
            unique.push(t.as_str());
        }
    }
    unique.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));
    unique.into_iter().take(3).collect::<Vec<_>>().join("、")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};

    /// 插一行 activities 行用于测试，控制 local_hour / app / title / dur / category。
    /// category_id 为 None 时不挂 app_group（COALESCE 落到 'other'）。
    async fn insert_act(
        pool: &DbPool,
        local_date: &str,
        local_hour: u8,
        process_name: &str,
        window_title: &str,
        duration_secs: i64,
    ) {
        let local_date = local_date.to_string();
        let process_name = process_name.to_string();
        let window_title = window_title.to_string();
        let device_id = TEST_SELF_ID.to_string();
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activities(
                        started_at, ended_at, duration_secs, local_date, local_hour,
                        process_name, window_title, category_id, device_id, updated_at, origin
                     ) VALUES(
                        ?1 || 'T' || printf('%02d', ?2) || ':00:00Z',
                        ?1 || 'T' || printf('%02d', ?2) || ':00:30Z',
                        ?3, ?1, ?2,
                        ?4, ?5, 'other', ?6,
                        ?1 || 'T' || printf('%02d', ?2) || ':00:30Z',
                        'local'
                     )",
                    rusqlite::params![
                        local_date,
                        local_hour as i64,
                        duration_secs,
                        process_name,
                        window_title,
                        device_id,
                    ],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn seed_solo_group(pool: &DbPool, name: &str, category_id: &str) {
        let name = name.to_string();
        let category_id = category_id.to_string();
        pool.0
            .call(move |conn| {
                let now = "2026-05-15T10:00:00Z";
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES(?1, ?1, ?2, ?3, NULL)",
                    rusqlite::params![name, category_id, now],
                )
                .db()?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES(?1, ?1, ?2, NULL)",
                    rusqlite::params![name, now],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn synth_empty_activities_returns_empty() {
        let pool = fresh_test_pool().await;
        let out = build_synthetic_descriptions_from_activities(
            &pool,
            "2026-05-15",
            9,
            10,
            &[],
            &DeviceFilter::All,
            &[],
        )
        .await
        .unwrap();
        assert!(out.is_empty(), "无 activities 应返回空: {out:?}");
    }

    #[tokio::test]
    async fn synth_groups_by_hour_and_sorts_by_duration() {
        let pool = fresh_test_pool().await;
        // 同小时 (9点) 3 个 app：VSCode > Chrome > Slack（全部 >= 60s 才不会折叠到「其它」）
        insert_act(&pool, "2026-05-15", 9, "VSCode", "main.rs", 300).await;
        insert_act(&pool, "2026-05-15", 9, "Chrome", "GitHub", 180).await;
        insert_act(&pool, "2026-05-15", 9, "Slack", "#hindsight", 90).await;

        let out = build_synthetic_descriptions_from_activities(
            &pool,
            "2026-05-15",
            9,
            10,
            &[],
            &DeviceFilter::All,
            &[],
        )
        .await
        .unwrap();
        assert_eq!(out.len(), 1, "只一小时应只返回一项: {out:?}");
        assert_eq!(out[0].0, "09:00-10:00");
        let desc = &out[0].1;
        let p_vscode = desc.find("VSCode").expect("缺 VSCode");
        let p_chrome = desc.find("Chrome").expect("缺 Chrome");
        let p_slack = desc.find("Slack").expect("缺 Slack");
        assert!(p_vscode < p_chrome, "VSCode 应排在 Chrome 前: {desc}");
        assert!(p_chrome < p_slack, "Chrome 应排在 Slack 前: {desc}");
    }

    #[tokio::test]
    async fn synth_privacy_keyword_replaces_window_title() {
        let pool = fresh_test_pool().await;
        insert_act(&pool, "2026-05-15", 9, "Chrome", "GitHub PR #142", 300).await;

        let keywords = vec!["github".to_string()];
        let out = build_synthetic_descriptions_from_activities(
            &pool,
            "2026-05-15",
            9,
            10,
            &[],
            &DeviceFilter::All,
            &keywords,
        )
        .await
        .unwrap();
        assert_eq!(out.len(), 1);
        let desc = &out[0].1;
        assert!(
            desc.contains("[私密]"),
            "命中 keyword 应替换成 [私密]: {desc}"
        );
        assert!(!desc.contains("GitHub PR #142"), "原标题不应再出现: {desc}");
        assert!(desc.contains("Chrome"), "app 名仍应贡献: {desc}");
        assert!(desc.contains("5 分钟"), "时长仍应贡献: {desc}");
    }

    #[tokio::test]
    async fn synth_excludes_categories() {
        let pool = fresh_test_pool().await;
        // Slack 挂到 'browse' 分类（旧版本用 'fun'，v31 软删后改成另一个 active 默认分类）
        seed_solo_group(&pool, "Slack", "browse").await;
        seed_solo_group(&pool, "VSCode", "code").await;
        insert_act(&pool, "2026-05-15", 9, "Slack", "amusing", 300).await;
        insert_act(&pool, "2026-05-15", 9, "VSCode", "lib.rs", 300).await;

        let excluded = vec!["browse".to_string()];
        let out = build_synthetic_descriptions_from_activities(
            &pool,
            "2026-05-15",
            9,
            10,
            &excluded,
            &DeviceFilter::All,
            &[],
        )
        .await
        .unwrap();
        assert_eq!(out.len(), 1);
        let desc = &out[0].1;
        assert!(desc.contains("VSCode"), "code 类应保留: {desc}");
        assert!(!desc.contains("Slack"), "browse 类应被排除: {desc}");
    }

    #[tokio::test]
    async fn synth_skips_empty_hours() {
        let pool = fresh_test_pool().await;
        // 9 点 + 11 点有活动，10 点空
        insert_act(&pool, "2026-05-15", 9, "VSCode", "main.rs", 300).await;
        insert_act(&pool, "2026-05-15", 11, "VSCode", "lib.rs", 300).await;

        let out = build_synthetic_descriptions_from_activities(
            &pool,
            "2026-05-15",
            9,
            12,
            &[],
            &DeviceFilter::All,
            &[],
        )
        .await
        .unwrap();
        assert_eq!(out.len(), 2, "只 9 + 11 两点有活动: {out:?}");
        assert_eq!(out[0].0, "09:00-10:00");
        assert_eq!(out[1].0, "11:00-12:00");
    }
}
