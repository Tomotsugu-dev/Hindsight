//! AI 总结的具体业务操作：
//!
//! - [`build_activity_timeline`]：从 activities 合成段内逐小时活动时间线（唯一材料源）
//! - [`summarize_segment`]：段总结（纯文本调用，写库 + 返回行）
//! - [`build_step2`]：根据 settings 构造段总结 chat 路由（本地 / 外部）
//!
//! 这些函数从 `DaySummaryRunner` 拎出来便于单测与代码审查；调用方传 owned
//! 数据 + Arc 的 supervisor / cancel / pool，避免持引用跨 await。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::ai::config::AiConfig;
use crate::ai::llm::{ChatClient, ExternalChatClient, Step2Chat};
use crate::ai::prompt::{build_system_prompt, build_user_prompt, SegmentContext};
use crate::ai::server::EngineSupervisor;
use crate::capture::privacy;
use crate::error::{Error, Result};
use crate::repo::ai_summaries::{self, SegmentSummaryRow};
use crate::repo::reports::DeviceFilter;
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};

/// 把一个 future 变成"可被停止按钮中断的"：每 250ms 轮询一次 cancel 标志，
/// 置位则**丢弃 future**（reqwest 请求随之断开连接）并返回 [`Error::SummaryCancelled`]。
///
/// 正在路上的 LLM 请求（本地超时 600s）和引擎加载（最长 90s）都能被中断；
/// llama-server 检测到客户端断开会停掉该 slot 的生成，云端 API 同理，中断是安全的。
pub(crate) async fn cancellable<T>(
    cancel: &Arc<AtomicBool>,
    fut: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::pin!(fut);
    loop {
        tokio::select! {
            r = &mut fut => return r,
            _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Error::SummaryCancelled);
                }
            }
        }
    }
}

// ───────────────────────────── 段总结 ─────────────────────────────

/// 段总结：拿活动时间线 + top_apps 拼 prompt → 调 LLM → 落库。
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
    timeline: &[(String, String)],
    top_apps: &[(String, u32, String)],
    step2_model: String,
    // 停止按钮的取消标志：置位时中断在途请求，向上抛 SummaryCancelled
    //（**不写行**——该段下次生成自然重跑），由 runner 统一 emit cancelled 收尾
    cancel: &Arc<AtomicBool>,
) -> Result<(SegmentSummaryRow, &'static str)> {
    let ctx = SegmentContext {
        label,
        start_hour,
        end_hour,
        top_apps,
        timeline,
    };
    let system = build_system_prompt(ai);
    let user_text = build_user_prompt(ai, &ctx);

    // 本地走自家引擎，需要 acquire 防止 watcher 在请求中途 stop；
    // 云端 (External) 不动 supervisor，不 acquire。
    let _inflight = step2.is_local().then(|| supervisor.acquire_inference());
    let chat_result = cancellable(cancel, step2.chat(&system, &user_text, &[])).await;
    if matches!(chat_result, Err(Error::SummaryCancelled)) {
        return Err(Error::SummaryCancelled);
    }
    let (row, status_str): (SegmentSummaryRow, &'static str) = match chat_result {
        // 落库的 model 用 step2_model——本地是 GGUF 文件名，
        // 外部是用户填的云端模型 ID（如 deepseek-chat）
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
    if let Err(e) = ai_summaries::upsert_segment(pool, &row).await {
        log::error!(
            "ai_summaries upsert 失败（段 {} status={}）：{e}",
            row.segment_idx,
            row.status,
        );
    }
    Ok((row, status_str))
}

/// 根据 [`AiConfig::summary_use_cloud`] 构造段总结的 chat 路由。
///
/// - false：[`Step2Chat::Local`]——本地端口；`local_model_label` 是当前引擎实际加载的
///   GGUF 文件名（即 `effective_summary_main`），用作 `model_label()` 落库 +
///   chat completions 请求的 model 字段
/// - true：[`Step2Chat::External`] 包一个新建的 [`ExternalChatClient`]，
///   走用户填的 endpoint / model / api_key
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

/// 让某段直接落 `skipped_no_activity` 行 —— 该段完全没有活动记录时的兜底。
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

// ───────── 活动时间线：从 activities 表合成段材料（唯一材料源） ─────────

/// 从 `activities` 表合成段内「按小时」的活动时间线，形状 `(time_label, desc)`，
/// 直接喂给 [`summarize_segment`]。这是日报的唯一材料源（不再有截图描述）。
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
pub(crate) async fn build_activity_timeline(
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

    Ok(format_timeline_hours(rows, privacy_app_keywords))
}

/// 把 SQL 行（hour, app, title, dur）按小时 / 应用聚合成 `(time_label, desc)` 列表。
/// 抽函数让单测可以纯粹喂结构化数据，不依赖 SQLite。
fn format_timeline_hours(
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

/// 去重保序后按字符数降序取前 5 个，"、" 分隔。
/// 窗口标题是总结的主线索（文件名 / 网页标题 / 视频标题），多带一点
/// 让模型有素材可写；5 条 × 每小时几个应用的 prompt 开销可忽略。
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
    unique.into_iter().take(5).collect::<Vec<_>>().join("、")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};

    /// 端到端:真实主库 → 新单步管线(活动时间线 + 云端文本模型)→ 写回 daily 日报。
    /// 跑法:
    ///   `DAILY_E2E_DATE=2026-07-05 CHAT_E2E_ENDPOINT=... CHAT_E2E_MODEL=... CHAT_E2E_KEY=... \
    ///    cargo test --lib summary_operations::tests::e2e -- --ignored --nocapture`
    /// 写的是真实 ai_summaries(source='daily'),会先清掉当天旧行。
    #[tokio::test]
    #[ignore]
    async fn e2e_regenerate_daily_report() {
        let date = std::env::var("DAILY_E2E_DATE").expect("设 DAILY_E2E_DATE=YYYY-MM-DD");
        let endpoint = std::env::var("CHAT_E2E_ENDPOINT").expect("设 CHAT_E2E_ENDPOINT");
        let model = std::env::var("CHAT_E2E_MODEL").expect("设 CHAT_E2E_MODEL");
        let api_key = std::env::var("CHAT_E2E_KEY").unwrap_or_default();

        let pool = DbPool::open(&crate::storage::db_path().unwrap())
            .await
            .unwrap();
        let cfg = crate::repo::settings::load(&pool).await.unwrap();
        // 强制云端文本路由(用传入凭据),其余(段划分/排除分类/语言/简介)沿用用户设置
        let mut ai = cfg.ai.clone();
        ai.external_enabled = true;
        ai.summary_main = crate::ai::config::SUMMARY_CLOUD_SENTINEL.to_string();
        ai.endpoint = endpoint;
        ai.model = model;
        ai.api_key = api_key;

        ai_summaries::clear_day(&pool, "daily", &date)
            .await
            .unwrap();

        let supervisor = Arc::new(EngineSupervisor::new());
        let cancel = Arc::new(AtomicBool::new(false));
        let step2 = build_step2(&ai, 0, "").unwrap();

        for (idx, seg) in ai.segments.iter().enumerate() {
            if seg.end_hour <= seg.start_hour {
                continue;
            }
            let timeline = build_activity_timeline(
                &pool,
                &date,
                seg.start_hour,
                seg.end_hour,
                &ai.excluded_categories,
                &DeviceFilter::All,
                &cfg.privacy_app_keywords,
            )
            .await
            .unwrap();
            println!(
                "\n===== 段 {idx} {}({:02}:00-{:02}:00) 时间线 {} 行",
                seg.label,
                seg.start_hour,
                seg.end_hour,
                timeline.len()
            );
            if timeline.is_empty() {
                upsert_skipped_no_activity(
                    &pool,
                    "daily",
                    &date,
                    idx as u32,
                    &seg.label,
                    seg.start_hour,
                    seg.end_hour,
                    step2.model_label().to_string(),
                )
                .await
                .unwrap();
                println!("(无活动,skipped)");
                continue;
            }
            let top_apps = crate::repo::ai_summaries::list_segment_top_apps(
                &pool,
                &date,
                seg.start_hour,
                seg.end_hour,
                &ai.excluded_categories,
                DeviceFilter::All,
                8,
            )
            .await
            .unwrap_or_default();
            let (row, status) = summarize_segment(
                &pool,
                &step2,
                &supervisor,
                &ai,
                "daily",
                &date,
                &seg.label,
                seg.start_hour,
                seg.end_hour,
                idx as u32,
                &timeline,
                &top_apps,
                step2.model_label().to_string(),
                &cancel,
            )
            .await
            .unwrap();
            println!("[{status}]\n{}", row.content);
            if status == "error" {
                println!("error: {:?}", row.error);
            }
        }
    }

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
    async fn timeline_empty_activities_returns_empty() {
        let pool = fresh_test_pool().await;
        let out = build_activity_timeline(&pool, "2026-05-15", 9, 10, &[], &DeviceFilter::All, &[])
            .await
            .unwrap();
        assert!(out.is_empty(), "无 activities 应返回空: {out:?}");
    }

    #[tokio::test]
    async fn timeline_groups_by_hour_and_sorts_by_duration() {
        let pool = fresh_test_pool().await;
        // 同小时 (9点) 3 个 app：VSCode > Chrome > Slack（全部 >= 60s 才不会折叠到「其它」）
        insert_act(&pool, "2026-05-15", 9, "VSCode", "main.rs", 300).await;
        insert_act(&pool, "2026-05-15", 9, "Chrome", "GitHub", 180).await;
        insert_act(&pool, "2026-05-15", 9, "Slack", "#hindsight", 90).await;

        let out = build_activity_timeline(&pool, "2026-05-15", 9, 10, &[], &DeviceFilter::All, &[])
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
    async fn timeline_privacy_keyword_replaces_window_title() {
        let pool = fresh_test_pool().await;
        insert_act(&pool, "2026-05-15", 9, "Chrome", "GitHub PR #142", 300).await;

        let keywords = vec!["github".to_string()];
        let out = build_activity_timeline(
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
    async fn timeline_excludes_categories() {
        let pool = fresh_test_pool().await;
        // Slack 挂到 'browse' 分类（旧版本用 'fun'，v31 软删后改成另一个 active 默认分类）
        seed_solo_group(&pool, "Slack", "browse").await;
        seed_solo_group(&pool, "VSCode", "code").await;
        insert_act(&pool, "2026-05-15", 9, "Slack", "amusing", 300).await;
        insert_act(&pool, "2026-05-15", 9, "VSCode", "lib.rs", 300).await;

        let excluded = vec!["browse".to_string()];
        let out = build_activity_timeline(
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
    async fn timeline_skips_empty_hours() {
        let pool = fresh_test_pool().await;
        // 9 点 + 11 点有活动，10 点空
        insert_act(&pool, "2026-05-15", 9, "VSCode", "main.rs", 300).await;
        insert_act(&pool, "2026-05-15", 11, "VSCode", "lib.rs", 300).await;

        let out = build_activity_timeline(&pool, "2026-05-15", 9, 12, &[], &DeviceFilter::All, &[])
            .await
            .unwrap();
        assert_eq!(out.len(), 2, "只 9 + 11 两点有活动: {out:?}");
        assert_eq!(out[0].0, "09:00-10:00");
        assert_eq!(out[1].0, "11:00-12:00");
    }
}
