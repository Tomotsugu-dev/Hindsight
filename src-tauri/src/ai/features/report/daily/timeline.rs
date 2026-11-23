//! 日报一段的材料：从活动记录合成逐小时的时间线。

use crate::error::Result;
use crate::repo::reports::DeviceFilter;
use crate::repo::sql::{from_stats_category_sql, host_rule_with_sql};
use crate::storage::{DbPool, SqliteResultExt};

/// 从 `activities` 表合成段内「按小时」的活动时间线，形状 `(time_label, desc)`，
/// 直接喂给 [`summarize_segment`](super::segment::summarize_segment)。这是日报的唯一材料源（不再有截图描述）。
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
    use crate::ai::policy::{AiDataPolicy, AiModule, ProfileChoice};
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
}
