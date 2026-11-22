//! AI 总结的具体业务操作：
//!
//! - [`build_activity_timeline`]：从 activities 合成段内逐小时活动时间线（唯一材料源）
//! - [`summarize_segment`]：段总结（纯文本调用，写库 + 返回行）
//! - [`build_step2`]：按设置打开日报和周报用的模型（本地 / 云端）
//!
//! 这些函数从 `DaySummaryRunner` 拎出来便于单测与代码审查；调用方传 owned
//! 数据 + Arc 的 cancel / pool，避免持引用跨 await。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::ai::config::{AiConfig, ModelRef, Task, CLOUD_UNAVAILABLE_ERROR, NO_MODEL_ERROR};
use crate::ai::llm::Step2Chat;
use crate::ai::prompt::{build_system_prompt, build_user_prompt, SegmentContext};
use crate::error::{Error, Result};
use crate::repo::ai_summaries::{self, SegmentSummaryRow};
use crate::repo::reports::DeviceFilter;
use crate::repo::sql::{from_stats_category_sql, host_rule_with_sql};
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

    let chat_result = cancellable(cancel, step2.chat(&system, &user_text)).await;
    if matches!(chat_result, Err(Error::SummaryCancelled)) {
        return Err(Error::SummaryCancelled);
    }
    let (row, status_str): (SegmentSummaryRow, &'static str) = match chat_result {
        // 落库的 model 用 step2_model——本地是 GGUF 文件名，
        // 外部是用户填的云端模型 ID（如 deepseek-chat）
        Ok(content) => (
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

/// Opens the model configured for a daily or weekly report with its reasoning level.
///
/// When using a local model, `local_port` is the port of the running llama-server instance.
pub(crate) fn build_step2(ai: &AiConfig, task: Task, local_port: u16) -> Result<Step2Chat> {
    let max_tokens = ai.summary_max_tokens(task);
    let reasoning = ai.reasoning_for(task);
    match ai.model_for(task) {
        Some(ModelRef::Cloud { connection }) => {
            let conn = ai
                .cloud_connection(&connection)
                .ok_or(Error::InvalidInput(CLOUD_UNAVAILABLE_ERROR))?;
            Step2Chat::cloud(
                &conn.endpoint,
                &conn.model,
                &conn.api_key,
                &conn.dialect,
                max_tokens,
                reasoning,
            )
        }
        Some(ModelRef::Local { file }) => {
            Step2Chat::local(local_port, &file, max_tokens, reasoning)
        }
        None => Err(Error::InvalidInput(NO_MODEL_ERROR)),
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
/// - `visible` 是 AI 能看到哪些活动，见
///   [`AiDataPolicy::main_filter`](crate::ai::policy::AiDataPolicy::main_filter)；
///   再按 [`DeviceFilter`] 过滤
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
    visible: &str,
    device: &DeviceFilter,
) -> Result<Vec<(String, String)>> {
    use rusqlite::ToSql;

    let date = date_str.to_string();
    let visible = visible.to_string();
    let dev = device.clone();
    let rows: Vec<(u8, String, Option<String>, i64)> = pool
        .0
        .call(move |conn| {
            let sql = format!(
                "{with}
                 SELECT a.local_hour,
                        COALESCE(g.display_name, a.process_name) AS app_display,
                        a.window_title,
                        a.duration_secs
                   {from}
                  WHERE a.local_date = ?
                    AND a.local_hour >= ?
                    AND a.local_hour < ?
                    AND a.duration_secs > 0
                    AND {visible}
                    {device}
                  ORDER BY a.local_hour ASC, a.duration_secs DESC",
                with = host_rule_with_sql("activities"),
                from = from_stats_category_sql("activities"),
                device = dev.sql_clause(),
            );
            // 网站规则的 WITH 在最前，它的两个 ? 是日期范围的两头
            let mut params: Vec<&dyn ToSql> = vec![&date, &date, &date];
            let sh = start_hour as i64;
            let eh = end_hour as i64;
            params.push(&sh);
            params.push(&eh);
            if let Some(extra) = dev.sql_param() {
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

    Ok(format_timeline_hours(rows))
}

/// 把 SQL 行（hour, app, title, dur）按小时 / 应用聚合成 `(time_label, desc)` 列表。
/// 抽函数让单测可以纯粹喂结构化数据，不依赖 SQLite。
fn format_timeline_hours(rows: Vec<(u8, String, Option<String>, i64)>) -> Vec<(String, String)> {
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
                app_bucket.1.push(trimmed.to_string());
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
    use crate::ai::model::Reasoning;
    use crate::ai::policy::{AiDataPolicy, AiModule, ProfileChoice};
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
        ai.cloud = vec![crate::ai::config::CloudConnection {
            id: "e2e".to_string(),
            name: "e2e".to_string(),
            dialect: "openai".to_string(),
            endpoint,
            model,
            api_key,
            context_length: None,
        }];
        ai.picks.daily = Some(ModelRef::Cloud {
            connection: "e2e".to_string(),
        });

        ai_summaries::clear_day(&pool, "daily", &date)
            .await
            .unwrap();

        let cancel = Arc::new(AtomicBool::new(false));
        let step2 = build_step2(&ai, Task::Daily, 0).unwrap();
        let visible = AiDataPolicy::load(&pool.0, AiModule::Daily, &ProfileChoice::Auto)
            .await
            .unwrap()
            .main_filter();

        for (idx, seg) in ai.segments.iter().enumerate() {
            if seg.end_hour <= seg.start_hour {
                continue;
            }
            let timeline = build_activity_timeline(
                &pool,
                &date,
                seg.start_hour,
                seg.end_hour,
                &visible,
                &DeviceFilter::All,
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
                &visible,
                DeviceFilter::All,
                8,
            )
            .await
            .unwrap_or_default();
            let (row, status) = summarize_segment(
                &pool,
                &step2,
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

    /// 按主库设置，取 2026-05-15 从 `start` 点到 `end` 点、所有设备的时间线。
    async fn timeline(pool: &DbPool, start: u8, end: u8) -> Vec<(String, String)> {
        let visible = AiDataPolicy::load(&pool.0, AiModule::Daily, &ProfileChoice::Main)
            .await
            .unwrap()
            .main_filter();
        build_activity_timeline(pool, "2026-05-15", start, end, &visible, &DeviceFilter::All)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn the_daily_timeline_uses_the_policy() {
        let pool = fresh_test_pool().await;
        seed_solo_group(&pool, "Secret", "hidden").await;
        seed_solo_group(&pool, "Signal", "private").await;
        insert_act(&pool, "2026-05-15", 9, "VSCode", "main.rs", 300).await;
        insert_act(&pool, "2026-05-15", 9, "Secret", "s", 300).await;
        insert_act(&pool, "2026-05-15", 9, "Signal", "c", 300).await;

        let out = timeline(&pool, 9, 10).await;
        let desc = &out[0].1;
        // 按主库设置，「不计入统计」和「隐私」分类都看不到
        assert!(desc.contains("VSCode"), "{desc}");
        assert!(
            !desc.contains("Secret") && !desc.contains("Signal"),
            "{desc}"
        );
    }

    #[tokio::test]
    async fn timeline_empty_activities_returns_empty() {
        let pool = fresh_test_pool().await;
        let out = timeline(&pool, 9, 10).await;
        assert!(out.is_empty(), "无 activities 应返回空: {out:?}");
    }

    #[tokio::test]
    async fn timeline_groups_by_hour_and_sorts_by_duration() {
        let pool = fresh_test_pool().await;
        // 同小时 (9点) 3 个 app：VSCode > Chrome > Slack（全部 >= 60s 才不会折叠到「其它」）
        insert_act(&pool, "2026-05-15", 9, "VSCode", "main.rs", 300).await;
        insert_act(&pool, "2026-05-15", 9, "Chrome", "GitHub", 180).await;
        insert_act(&pool, "2026-05-15", 9, "Slack", "#hindsight", 90).await;

        let out = timeline(&pool, 9, 10).await;
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
    async fn timeline_drops_activities_with_privacy_keywords() {
        let pool = fresh_test_pool().await;
        insert_act(&pool, "2026-05-15", 9, "Chrome", "GitHub PR #142", 300).await;
        insert_act(&pool, "2026-05-15", 9, "VSCode", "main.rs", 300).await;
        let mut s = crate::repo::settings::load(&pool).await.unwrap();
        s.privacy_app_keywords = vec!["github".to_string()];
        crate::repo::settings::save(&pool, &s).await.unwrap();

        let out = timeline(&pool, 9, 10).await;
        let desc = &out[0].1;
        // 标题命中关键词，整条活动都不给：应用名和时长也不算
        assert!(!desc.contains("Chrome"), "{desc}");
        assert!(desc.contains("VSCode 5 分钟"), "{desc}");
    }

    #[tokio::test]
    async fn timeline_applies_the_visible_condition() {
        let pool = fresh_test_pool().await;
        // Slack 挂到 'browse' 分类（旧版本用 'fun'，v31 软删后改成另一个 active 默认分类）
        seed_solo_group(&pool, "Slack", "browse").await;
        seed_solo_group(&pool, "VSCode", "code").await;
        insert_act(&pool, "2026-05-15", 9, "Slack", "amusing", 300).await;
        insert_act(&pool, "2026-05-15", 9, "VSCode", "lib.rs", 300).await;

        let out = build_activity_timeline(
            &pool,
            "2026-05-15",
            9,
            10,
            "COALESCE(c.id, 'other') <> 'browse'",
            &DeviceFilter::All,
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

        let out = timeline(&pool, 9, 12).await;
        assert_eq!(out.len(), 2, "只 9 + 11 两点有活动: {out:?}");
        assert_eq!(out[0].0, "09:00-10:00");
        assert_eq!(out[1].0, "11:00-12:00");
    }

    // ───────── 以下为补充测试：写路径边界 / 路由分支 / 取消语义 ─────────

    /// 同 [`insert_act`] 但可指定 device_id——测多设备行不误入本机日报用。
    #[allow(clippy::too_many_arguments)]
    async fn insert_act_dev(
        pool: &DbPool,
        local_date: &str,
        local_hour: u8,
        process_name: &str,
        window_title: &str,
        duration_secs: i64,
        device_id: &str,
    ) {
        let local_date = local_date.to_string();
        let process_name = process_name.to_string();
        let window_title = window_title.to_string();
        let device_id = device_id.to_string();
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

    /// 拿一个刚刚释放的本地端口——连接必然被拒，用来模拟"引擎没起来/端口配错"。
    fn free_local_port() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        port
    }

    /// 起一个只回一发 canned OpenAI 兼容响应的本地假服务，返回端口。
    /// 读完整个请求（headers + Content-Length body）再回包，避免半途关闭触发 RST。
    async fn spawn_canned_openai_server(content: &str) -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = serde_json::json!({
            "choices": [{
                "message": { "role": "assistant", "content": content },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 42, "completion_tokens": 17 }
        })
        .to_string();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf: Vec<u8> = Vec::new();
            let mut tmp = [0u8; 4096];
            loop {
                let n = sock.read(&mut tmp).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                    let cl = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if buf.len() >= pos + 4 + cl {
                        break;
                    }
                }
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            let _ = sock.shutdown().await;
        });
        port
    }

    /// 停止按钮按下的同一瞬间 LLM 恰好返回：已完成的结果（包括真实错误）不能被
    /// 吞成 SummaryCancelled——否则用户点了停止就永远看不到真正的失败原因。
    #[tokio::test]
    async fn cancellable_ready_result_beats_cancel_flag() {
        let cancel = Arc::new(AtomicBool::new(true));
        let ok = cancellable(&cancel, async { Ok::<_, Error>(42u32) }).await;
        assert_eq!(ok.unwrap(), 42, "已就绪的 Ok 结果应原样返回");

        let err = cancellable(&cancel, async {
            Err::<u32, _>(Error::InvalidInput("boom"))
        })
        .await;
        assert!(
            matches!(err, Err(Error::InvalidInput("boom"))),
            "已就绪的 Err 应透传原错误而非 SummaryCancelled: {err:?}"
        );
    }

    /// 引擎挂死不回包时按停止：250ms 轮询必须能打断永远 pending 的 future。
    /// 这是停止按钮的核心保证——没有它用户只能干等 600s 超时。
    #[tokio::test]
    async fn cancellable_pending_future_returns_cancelled() {
        let cancel = Arc::new(AtomicBool::new(true));
        let r: Result<u32> = cancellable(&cancel, std::future::pending::<Result<u32>>()).await;
        assert!(
            matches!(r, Err(Error::SummaryCancelled)),
            "挂死 future + cancel 置位应返回 SummaryCancelled: {r:?}"
        );
    }

    /// 60s 阈值 off-by-one 会把 59s 显示成"0 分钟"或把 60s 显示成"60s"，
    /// 模型看到"0 分钟"会写出荒谬的时长描述。
    #[test]
    fn secs_human_minute_threshold() {
        assert_eq!(format_secs_human(59), "59s");
        assert_eq!(format_secs_human(60), "1 分钟");
        assert_eq!(format_secs_human(3599), "59 分钟");
    }

    /// 标题排序必须按字符数而非字节数——中文 4 字 12 字节，按字节排会让 CJK
    /// 标题系统性挤掉更长的英文标题；重复标题（同窗口反复聚焦）只应出现一次。
    #[test]
    fn pick_titles_dedup_char_count_top5() {
        let titles: Vec<String> = ["abcde", "中文标题", "abcde", "xx"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(pick_titles(&titles), "abcde、中文标题、xx");

        // 6 个不同长度的标题只保留最长 5 个——prompt 体积失控的保险丝
        let many: Vec<String> = ["aaaaaa", "bbbbb", "cccc", "ddd", "ee", "f"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let out = pick_titles(&many);
        assert_eq!(out, "aaaaaa、bbbbb、cccc、ddd、ee");
    }

    /// 碎片应用（<60s 的窗口切换噪音）必须折叠成「其它」而不是逐个罗列，
    /// 否则一小时几十次 alt-tab 会把 prompt 塞满垃圾条目；同 app 多行要聚合时长。
    #[test]
    fn timeline_hours_folds_sub_minute_apps() {
        let rows: Vec<(u8, String, Option<String>, i64)> = vec![
            (9, "VSCode".to_string(), Some("a.rs".to_string()), 200),
            (9, "VSCode".to_string(), Some("a.rs".to_string()), 100),
            (9, "Finder".to_string(), None, 45),
            (9, "Preview".to_string(), None, 10),
            (10, "Spotlight".to_string(), None, 30),
        ];
        let out = format_timeline_hours(rows);
        assert_eq!(out.len(), 2, "{out:?}");

        // 同 app 两行 200+100 聚合成 5 分钟；重复标题去重只出现一次
        assert_eq!(out[0].0, "09:00-10:00");
        assert!(
            out[0].1.contains("VSCode 5 分钟（a.rs）"),
            "同 app 时长应聚合、标题应去重: {}",
            out[0].1
        );
        // 45s + 10s 两个碎片折叠：不出现 app 名，只留统计
        assert!(
            out[0].1.contains("其它（2 项 · 55s）"),
            "碎片应折叠成「其它」: {}",
            out[0].1
        );
        assert!(
            !out[0].1.contains("Finder") && !out[0].1.contains("Preview"),
            "碎片 app 名不应罗列: {}",
            out[0].1
        );
        // 只有碎片活动的小时也要出条目——否则该小时被静默吞掉，日报出现空洞
        assert_eq!(out[1].0, "10:00-11:00");
        assert_eq!(out[1].1, "其它（1 项 · 30s）");
    }

    /// 纯空白窗口标题（部分 app 全屏时上报空串）不能渲染成「（）」污染 prompt。
    #[test]
    fn timeline_hours_skips_blank_titles() {
        let rows: Vec<(u8, String, Option<String>, i64)> =
            vec![(9, "Term".to_string(), Some("   ".to_string()), 120)];
        let out = format_timeline_hours(rows);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1, "Term 2 分钟", "空白标题不应产生括号");
    }

    /// 多设备同步后别的设备的 activities 混进本机日报是真实翻车场景。
    /// 同时带可见条件 + DeviceFilter::Only 覆盖 SQL 参数顺序：
    /// 网站规则的日期参数与 device 参数一旦颠倒会静默查错列、结果全错。
    #[tokio::test]
    async fn timeline_device_only_filter_with_the_visible_condition() {
        let pool = fresh_test_pool().await;
        seed_solo_group(&pool, "Slack", "browse").await;
        seed_solo_group(&pool, "VSCode", "code").await;
        insert_act(&pool, "2026-05-15", 9, "VSCode", "main.rs", 300).await;
        insert_act(&pool, "2026-05-15", 9, "Slack", "chat", 300).await;
        // 别的设备上 VSCode 用了 100 分钟——若泄漏进来时长会被显著放大
        insert_act_dev(
            &pool,
            "2026-05-15",
            9,
            "VSCode",
            "other.rs",
            6000,
            "other-device",
        )
        .await;

        let no_browse = "COALESCE(c.id, 'other') <> 'browse'";
        let only = DeviceFilter::Only(TEST_SELF_ID.to_string());
        let out = build_activity_timeline(&pool, "2026-05-15", 9, 10, no_browse, &only)
            .await
            .unwrap();
        assert_eq!(out.len(), 1, "{out:?}");
        let desc = &out[0].1;
        assert!(
            desc.contains("VSCode 5 分钟"),
            "只应统计本机 300s（若混入他机则是 105 分钟）: {desc}"
        );
        assert!(!desc.contains("other.rs"), "他机标题不应出现: {desc}");
        assert!(!desc.contains("Slack"), "browse 分类应同时被排除: {desc}");

        // 对照组：All 应把两台设备聚合（300 + 6000 = 105 分钟）
        let all =
            build_activity_timeline(&pool, "2026-05-15", 9, 10, no_browse, &DeviceFilter::All)
                .await
                .unwrap();
        assert!(
            all[0].1.contains("VSCode 105 分钟"),
            "All 过滤应聚合全部设备: {}",
            all[0].1
        );
    }

    /// unsealed 心跳行（duration_secs=0）必须被 SQL 层排除——若漏进来会以
    /// 「其它」碎片形式出现，让模型看到幽灵活动。
    #[tokio::test]
    async fn timeline_excludes_unsealed_zero_duration_rows() {
        let pool = fresh_test_pool().await;
        insert_act(&pool, "2026-05-15", 9, "Ghost", "beat", 0).await;
        insert_act(&pool, "2026-05-15", 9, "Real", "work.rs", 120).await;

        let out = timeline(&pool, 9, 10).await;
        assert_eq!(out.len(), 1, "{out:?}");
        let desc = &out[0].1;
        assert!(desc.contains("Real"), "已 seal 行应保留: {desc}");
        assert!(!desc.contains("Ghost"), "dur=0 行不应出现: {desc}");
        // 关键：0s 行应被 SQL 排除，而不是折叠成「其它（1 项 · 0s）」
        assert!(!desc.contains("其它"), "0s 行不应折叠进「其它」: {desc}");
    }

    /// 段范围是半开区间 [start, end)——写成 <= 会把下一段的第一个小时重复
    /// 计入两个段，同一小时的活动在日报里出现两次。
    #[tokio::test]
    async fn timeline_hour_range_is_half_open() {
        let pool = fresh_test_pool().await;
        insert_act(&pool, "2026-05-15", 8, "Early", "e", 300).await;
        insert_act(&pool, "2026-05-15", 9, "Mid", "m", 300).await;
        insert_act(&pool, "2026-05-15", 10, "Late", "l", 300).await;

        let out = timeline(&pool, 9, 10).await;
        assert_eq!(out.len(), 1, "仅 9 点应命中: {out:?}");
        assert_eq!(out[0].0, "09:00-10:00");
        assert!(out[0].1.contains("Mid"));
        assert!(
            !out[0].1.contains("Early") && !out[0].1.contains("Late"),
            "边界小时不应混入: {}",
            out[0].1
        );
    }

    #[test]
    fn step2_local_and_missing_routes() {
        let mut ai = AiConfig::default();
        assert!(matches!(
            build_step2(&ai, Task::Daily, 8080),
            Err(Error::InvalidInput(_))
        ));

        ai.picks.daily = Some(ModelRef::Local {
            file: "qwen.gguf".to_string(),
        });
        let s = build_step2(&ai, Task::Daily, 8080).unwrap();
        assert!(s.is_local());
        assert_eq!(s.model_label(), "qwen.gguf");
    }

    #[tokio::test]
    async fn reports_send_their_reasoning() {
        use crate::ai::config::{Advanced, ReasoningLevel};
        use crate::ai::transport::fake::{FakeServer, Reply};
        let server = FakeServer::start(vec![Reply::json(
            200,
            serde_json::json!({"choices": [{"message": {"role": "assistant", "content": "好"}}]}),
        )])
        .await;
        let mut ai = AiConfig::default();
        ai.picks.daily = Some(ModelRef::Local {
            file: "m.gguf".into(),
        });
        ai.advanced = vec![Advanced {
            task: Task::Daily,
            model: ModelRef::Local {
                file: "m.gguf".into(),
            },
            reasoning: Some(ReasoningLevel::High),
            ctx_size: None,
            parallel_slots: None,
            batch_size: None,
        }];
        build_step2(&ai, Task::Daily, server.port)
            .unwrap()
            .chat("sys", "user")
            .await
            .unwrap();
        let body = &server.received()[0].body;
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], true);
    }

    /// 云端路由时落库 model 必须是云端连接的模型名——取成本地 GGUF 文件名
    /// 会让 DailyTab / 导出 Markdown 显示错误的生成来源。
    #[test]
    fn step2_cloud_route_uses_cloud_model_label() {
        let mut ai = AiConfig::default();
        ai.cloud = vec![crate::ai::config::CloudConnection {
            id: "c1".into(),
            name: "DeepSeek".into(),
            dialect: "deepseek".into(),
            endpoint: "https://api.example.com/v1/".into(),
            model: "deepseek-chat".into(),
            api_key: String::new(),
            context_length: None,
        }];
        ai.picks.daily = Some(ModelRef::Cloud {
            connection: "c1".into(),
        });
        let s = build_step2(&ai, Task::Daily, 8080).unwrap();
        assert!(!s.is_local());
        assert_eq!(s.model_label(), "deepseek-chat");
    }

    /// 选了云端但连接没填全或已删除：必须构造期就报 InvalidInput。顶层错误条按这个类型
    /// 显示「去补配置」的引导；悄悄构造一个打空地址的 client，到运行期才失败，
    /// 用户只会看到一条没法照着做的通用报错。
    #[test]
    fn step2_cloud_missing_config_errors() {
        let mut ai = AiConfig::default();
        ai.cloud = vec![crate::ai::config::CloudConnection {
            id: "c1".into(),
            name: "bad".into(),
            dialect: "openai".into(),
            endpoint: "   ".into(),
            model: "deepseek-chat".into(),
            api_key: String::new(),
            context_length: None,
        }];
        ai.picks.daily = Some(ModelRef::Cloud {
            connection: "c1".into(),
        });
        assert!(
            matches!(
                build_step2(&ai, Task::Daily, 0),
                Err(Error::InvalidInput(_))
            ),
            "blank endpoint should return InvalidInput"
        );

        ai.cloud[0].endpoint = "https://api.example.com/v1".into();
        ai.cloud[0].model = "  ".into();
        assert!(
            matches!(
                build_step2(&ai, Task::Daily, 0),
                Err(Error::InvalidInput(_))
            ),
            "blank model should return InvalidInput"
        );

        ai.picks.daily = Some(ModelRef::Cloud {
            connection: "missing".into(),
        });
        assert!(
            matches!(
                build_step2(&ai, Task::Daily, 0),
                Err(Error::InvalidInput(_))
            ),
            "dangling connection id should return InvalidInput"
        );
    }

    /// 重跑同段必须覆盖不叠加——PK (source, date, idx) 语义回归：
    /// 若重复插行，前端日报同一段会渲染两次。
    #[tokio::test]
    async fn skipped_row_upsert_overwrites_not_duplicates() {
        let pool = fresh_test_pool().await;
        upsert_skipped_no_activity(&pool, "daily", "2026-05-15", 0, "上午", 9, 12, "m1".into())
            .await
            .unwrap();
        // 用户改了段配置后重跑：label / 时段 / 模型都变，应整行覆盖
        upsert_skipped_no_activity(&pool, "daily", "2026-05-15", 0, "早晨", 8, 12, "m2".into())
            .await
            .unwrap();

        let rows = ai_summaries::get_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "重跑应覆盖不叠加: {rows:?}");
        let r = &rows[0];
        assert_eq!(r.status, "skipped_no_activity");
        assert_eq!(r.label, "早晨", "第二次的字段应覆盖第一次");
        assert_eq!(r.start_hour, 8);
        assert_eq!(r.model, "m2");
        assert!(r.content.is_empty() && r.error.is_none());
    }

    /// 用户对从没生成过的日期点"强制刷新"：空库删除必须静默成功，不能报错。
    #[tokio::test]
    async fn clear_day_on_empty_db_is_ok() {
        let pool = fresh_test_pool().await;
        ai_summaries::clear_day(&pool, "daily", "2026-05-15")
            .await
            .expect("空库 clear_day 不应报错");
        assert!(ai_summaries::get_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap()
            .is_empty());
    }

    /// force_refresh 清某天日报时误伤别的 source 或相邻日期会让用户丢历史报告——
    /// DELETE 少写一个 WHERE 条件就是这个后果。
    #[tokio::test]
    async fn clear_day_spares_other_source_and_date() {
        let pool = fresh_test_pool().await;
        upsert_skipped_no_activity(&pool, "daily", "2026-05-15", 0, "上午", 9, 12, "m".into())
            .await
            .unwrap();
        upsert_skipped_no_activity(&pool, "daily", "2026-05-16", 0, "上午", 9, 12, "m".into())
            .await
            .unwrap();
        upsert_skipped_no_activity(&pool, "weekly", "2026-05-15", 0, "上午", 9, 12, "m".into())
            .await
            .unwrap();

        ai_summaries::clear_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap();

        assert!(
            ai_summaries::get_day(&pool, "daily", "2026-05-15")
                .await
                .unwrap()
                .is_empty(),
            "目标 source+日期应被清空"
        );
        assert_eq!(
            ai_summaries::get_day(&pool, "daily", "2026-05-16")
                .await
                .unwrap()
                .len(),
            1,
            "相邻日期不应误伤"
        );
        assert_eq!(
            ai_summaries::get_day(&pool, "weekly", "2026-05-15")
                .await
                .unwrap()
                .len(),
            1,
            "别的 source 不应误伤"
        );
    }

    /// 引擎没起来/端口配错时 chat 必然失败：该段必须落 status='error' 行并继续
    ///（前端红色 badge 的数据源），而不是抛 Err 让整轮 daily 中断。
    #[tokio::test]
    async fn summarize_segment_chat_failure_writes_error_row() {
        let pool = fresh_test_pool().await;
        let port = free_local_port();
        let step2 = Step2Chat::local(port, "dead.gguf", 1024, Reasoning::Default).unwrap();
        let ai = AiConfig::default();
        let cancel = Arc::new(AtomicBool::new(false));
        let timeline = vec![("09:00-10:00".to_string(), "VSCode 30 分钟".to_string())];

        let (row, status) = summarize_segment(
            &pool,
            &step2,
            &ai,
            "daily",
            "2026-05-15",
            "上午",
            9,
            12,
            0,
            &timeline,
            &[],
            "dead.gguf".to_string(),
            &cancel,
        )
        .await
        .expect("chat 失败不应让 summarize_segment 抛 Err");

        assert_eq!(status, "error");
        assert_eq!(row.status, "error");
        assert!(row.content.is_empty(), "失败段不应有正文");
        assert!(
            row.error.as_deref().is_some_and(|e| !e.is_empty()),
            "error 字段应带可读描述: {:?}",
            row.error
        );

        let rows = ai_summaries::get_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "error 行必须已落库: {rows:?}");
        assert_eq!(rows[0].status, "error");
        assert_eq!(rows[0].model, "dead.gguf", "model 应取传入的 step2_model");
    }

    /// 幂等重跑：昨天生成时该段无活动落了 skipped，今天重跑（此处走失败路径）
    /// 必须覆盖同一行——重复跑多少次都只有一行，且状态是最后一次的。
    #[tokio::test]
    async fn summarize_segment_rerun_overwrites_previous_row() {
        let pool = fresh_test_pool().await;
        upsert_skipped_no_activity(&pool, "daily", "2026-05-15", 0, "上午", 9, 12, "m".into())
            .await
            .unwrap();

        let port = free_local_port();
        let step2 = Step2Chat::local(port, "dead.gguf", 1024, Reasoning::Default).unwrap();
        let ai = AiConfig::default();
        let cancel = Arc::new(AtomicBool::new(false));
        let timeline = vec![("09:00-10:00".to_string(), "VSCode 30 分钟".to_string())];

        for _ in 0..2 {
            summarize_segment(
                &pool,
                &step2,
                &ai,
                "daily",
                "2026-05-15",
                "上午",
                9,
                12,
                0,
                &timeline,
                &[],
                "dead.gguf".to_string(),
                &cancel,
            )
            .await
            .unwrap();
        }

        let rows = ai_summaries::get_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "skipped + 两次重跑仍应只有一行: {rows:?}");
        assert_eq!(rows[0].status, "error", "最后一次运行的状态应覆盖 skipped");
    }

    /// 停止按钮语义：取消**不落行**——该段下次生成自然重跑。若取消也写 error 行，
    /// 用户按一次停止就会看到一排假错误段。
    #[tokio::test]
    async fn summarize_segment_cancelled_writes_no_row() {
        let pool = fresh_test_pool().await;
        // 挂起的假服务：backlog 完成握手但永不回包，让请求停在途中等 cancel 打断
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let step2 = Step2Chat::local(port, "m.gguf", 1024, Reasoning::Default).unwrap();
        let ai = AiConfig::default();
        let cancel = Arc::new(AtomicBool::new(true)); // 预先按下停止
        let timeline = vec![("09:00-10:00".to_string(), "VSCode 30 分钟".to_string())];

        let r = summarize_segment(
            &pool,
            &step2,
            &ai,
            "daily",
            "2026-05-15",
            "上午",
            9,
            12,
            0,
            &timeline,
            &[],
            "m.gguf".to_string(),
            &cancel,
        )
        .await;
        assert!(
            matches!(r, Err(Error::SummaryCancelled)),
            "取消应抛 SummaryCancelled: {r:?}"
        );
        assert!(
            ai_summaries::get_day(&pool, "daily", "2026-05-15")
                .await
                .unwrap()
                .is_empty(),
            "取消不应留下任何行"
        );
        drop(listener);
    }

    /// chat 成功的主干路径：status='ok' 行落库、正文取 choices[0] 并 trim、
    /// model 字段用传入的 step2_model——这是日报正常生成的全部落库契约。
    #[tokio::test]
    async fn summarize_segment_ok_writes_ok_row_and_trims_content() {
        let pool = fresh_test_pool().await;
        let port = spawn_canned_openai_server("  上午主要在写 Hindsight 的单元测试。  ").await;
        let step2 = Step2Chat::local(port, "qwen.gguf", 1024, Reasoning::Default).unwrap();
        let ai = AiConfig::default();
        let cancel = Arc::new(AtomicBool::new(false));
        let timeline = vec![("09:00-10:00".to_string(), "VSCode 30 分钟".to_string())];
        let top_apps = vec![("VSCode".to_string(), 30u32, "code".to_string())];

        let (row, status) = summarize_segment(
            &pool,
            &step2,
            &ai,
            "daily",
            "2026-05-15",
            "上午",
            9,
            12,
            2,
            &timeline,
            &top_apps,
            "qwen.gguf".to_string(),
            &cancel,
        )
        .await
        .unwrap();

        assert_eq!(status, "ok");
        assert_eq!(row.status, "ok");
        assert_eq!(
            row.content, "上午主要在写 Hindsight 的单元测试。",
            "正文应取自响应且去掉首尾空白"
        );
        assert!(row.error.is_none());

        let rows = ai_summaries::get_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "ok 行必须已落库: {rows:?}");
        assert_eq!(rows[0].segment_idx, 2);
        assert_eq!(rows[0].status, "ok");
        assert_eq!(rows[0].content, row.content);
        assert_eq!(rows[0].model, "qwen.gguf");
    }
}
