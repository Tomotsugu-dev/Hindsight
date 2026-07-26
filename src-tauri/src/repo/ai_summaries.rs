//! AI 总结结果缓存（Phase 1B-γ）。
//!
//! 表结构见 [storage::migrations] 的 v18 (`AI_SUMMARIES_TABLE_SQL`)。
//! 主键 `(local_date, segment_idx)` —— 每天每段一行，重跑同段直接 UPSERT 覆盖。
//!
//! 三种 status：
//! - `ok`：模型正常出文，content 是 markdown 段落
//! - `skipped_no_screenshots`：段内无截图（用户该时段没用电脑），content 空
//! - `error`：LLM 报错或超时，content 空、error 字段填可读描述
//!
//! 不进 sync_outbox：本地产物 + 模型差异大，跨设备同步无意义。

use rusqlite::types::ToSql;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::repo::reports::DeviceFilter;
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};

/// 单段总结的一行（DB <-> 前端共用）。
///
/// `segment_idx` 是该段在 `settings.ai.segments` 数组里的下标。
/// `label` / `start_hour` / `end_hour` 冗余存了一份——用户事后改段配置后，
/// 旧总结仍能正确显示当时的标签和时段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentSummaryRow {
    /// "daily"（日报页写入读取）/ "debug"（调试 tab 写入读取）。
    /// PK 含 source 让两支独立，互不覆盖、互不擦除。
    pub source: String,
    pub local_date: String,
    pub segment_idx: u32,
    pub label: String,
    pub start_hour: u8,
    pub end_hour: u8,
    pub content: String,
    pub model: String,
    /// "ok" / "skipped_no_screenshots" / "error"
    pub status: String,
    pub error: Option<String>,
    pub generated_at: String,
}

/// 拿某天某 source 下所有段的总结，按 segment_idx 升序。
/// `source` = "daily" / "debug" — 区分日报正式产物与调试沙盒产物。
pub async fn get_day(
    pool: &DbPool,
    source: &str,
    local_date: &str,
) -> Result<Vec<SegmentSummaryRow>> {
    let src = source.to_string();
    let date = local_date.to_string();
    let rows = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT source, local_date, segment_idx, label, start_hour, end_hour,
                            content, model, status, error, generated_at
                       FROM ai_summaries
                      WHERE source = ?1 AND local_date = ?2
                      ORDER BY segment_idx ASC",
                )
                .db()?;
            let rows = stmt
                .query_map(rusqlite::params![src, date], |r| {
                    Ok(SegmentSummaryRow {
                        source: r.get(0)?,
                        local_date: r.get(1)?,
                        segment_idx: r.get::<_, i64>(2)? as u32,
                        label: r.get(3)?,
                        start_hour: r.get::<_, i64>(4)? as u8,
                        end_hour: r.get::<_, i64>(5)? as u8,
                        content: r.get(6)?,
                        model: r.get(7)?,
                        status: r.get(8)?,
                        error: r.get(9)?,
                        generated_at: r.get(10)?,
                    })
                })
                .db()?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.db()?);
            }
            Ok(out)
        })
        .await?;
    Ok(rows)
}

/// 拿某 source 在 [start_date, end_date] 闭区间内、按 (local_date, segment_idx) 升序的所有段。
///
/// 周报路径用：`get_range(pool, "daily", monday, sunday)` 拿到一周内所有日报段；
/// 调用方按 local_date group + 拼成日维度文本送给 LLM。日期字符串格式 "YYYY-MM-DD"
/// 跟 [`SegmentSummaryRow::local_date`] 一致，SQLite 文本比较即可正确排序。
pub async fn get_range(
    pool: &DbPool,
    source: &str,
    start_date: &str,
    end_date: &str,
) -> Result<Vec<SegmentSummaryRow>> {
    let src = source.to_string();
    let start = start_date.to_string();
    let end = end_date.to_string();
    let rows = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT source, local_date, segment_idx, label, start_hour, end_hour,
                            content, model, status, error, generated_at
                       FROM ai_summaries
                      WHERE source = ?1 AND local_date >= ?2 AND local_date <= ?3
                      ORDER BY local_date ASC, segment_idx ASC",
                )
                .db()?;
            let rows = stmt
                .query_map(rusqlite::params![src, start, end], |r| {
                    Ok(SegmentSummaryRow {
                        source: r.get(0)?,
                        local_date: r.get(1)?,
                        segment_idx: r.get::<_, i64>(2)? as u32,
                        label: r.get(3)?,
                        start_hour: r.get::<_, i64>(4)? as u8,
                        end_hour: r.get::<_, i64>(5)? as u8,
                        content: r.get(6)?,
                        model: r.get(7)?,
                        status: r.get(8)?,
                        error: r.get(9)?,
                        generated_at: r.get(10)?,
                    })
                })
                .db()?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.db()?);
            }
            Ok(out)
        })
        .await?;
    Ok(rows)
}

/// 拿某段已落库的 status；没行返回 None。给 Phase 2 step2_only 看到空 stored 时
/// 区分"真空截图"（Phase 1 已写 skipped）跟"step 1 全失败"（Phase 1 已写 error）用。
pub async fn get_segment_status(
    pool: &DbPool,
    source: &str,
    local_date: &str,
    segment_idx: u32,
) -> Result<Option<String>> {
    let src = source.to_string();
    let date = local_date.to_string();
    let row = pool
        .0
        .call(move |conn| {
            let row = conn
                .query_row(
                    "SELECT status FROM ai_summaries
                       WHERE source = ?1 AND local_date = ?2 AND segment_idx = ?3
                       LIMIT 1",
                    rusqlite::params![src, date, segment_idx as i64],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .db()?;
            Ok(row)
        })
        .await?;
    Ok(row)
}

/// 写入或覆盖一段。`generated_at` 自动用当前 UTC 时间填，调用方不用管。
/// PK = (source, local_date, segment_idx)，所以 daily / debug 互不冲突。
pub async fn upsert_segment(pool: &DbPool, row: &SegmentSummaryRow) -> Result<()> {
    let mut row = row.clone();
    if row.generated_at.is_empty() {
        row.generated_at = utc_now_rfc3339();
    }
    pool.0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO ai_summaries(
                     source, local_date, segment_idx, label, start_hour, end_hour,
                     content, model, status, error, generated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(source, local_date, segment_idx) DO UPDATE SET
                     label        = excluded.label,
                     start_hour   = excluded.start_hour,
                     end_hour     = excluded.end_hour,
                     content      = excluded.content,
                     model        = excluded.model,
                     status       = excluded.status,
                     error        = excluded.error,
                     generated_at = excluded.generated_at",
                rusqlite::params![
                    row.source,
                    row.local_date,
                    row.segment_idx as i64,
                    row.label,
                    row.start_hour as i64,
                    row.end_hour as i64,
                    row.content,
                    row.model,
                    row.status,
                    row.error,
                    row.generated_at,
                ],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// 清空某 source 下某天所有段总结 + 同日逐图描述。`force_refresh` 时调。
pub async fn clear_day(pool: &DbPool, source: &str, local_date: &str) -> Result<()> {
    let src = source.to_string();
    let date = local_date.to_string();
    pool.0
        .call(move |conn| {
            conn.execute(
                "DELETE FROM ai_summaries WHERE source = ?1 AND local_date = ?2",
                rusqlite::params![src, date],
            )
            .db()?;
            conn.execute(
                "DELETE FROM ai_image_descriptions WHERE source = ?1 AND local_date = ?2",
                rusqlite::params![src, date],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// 只清当天段总结（不动历史遗留的逐图描述行）。
pub async fn clear_day_summaries_only(pool: &DbPool, source: &str, local_date: &str) -> Result<()> {
    let src = source.to_string();
    let date = local_date.to_string();
    pool.0
        .call(move |conn| {
            conn.execute(
                "DELETE FROM ai_summaries WHERE source = ?1 AND local_date = ?2",
                rusqlite::params![src, date],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// 拉某天某段使用最多的应用（display_name, minutes, category_id），按 minutes 降序。
///
/// 用来给 LLM 一个 "用户在干什么" 的弱信号，防止它对着十几张截图猜半天。
/// limit 默认调用方传 8 即可。
pub async fn list_segment_top_apps(
    pool: &DbPool,
    local_date: &str,
    start_hour: u8,
    end_hour: u8,
    excluded_categories: &[String],
    device: DeviceFilter,
    limit: u32,
) -> Result<Vec<(String, u32, String)>> {
    let date = local_date.to_string();
    let excluded: Vec<String> = excluded_categories.to_vec();
    let dev = device.clone();
    let rows = pool
        .0
        .call(move |conn| {
            let placeholders = if excluded.is_empty() {
                String::new()
            } else {
                let marks = vec!["?"; excluded.len()].join(",");
                format!(" AND COALESCE(c.id, 'other') NOT IN ({})", marks)
            };
            // 硬编码排除 hidden 分类（不在 excluded_categories 配置范畴内）
            let sql = format!(
                "SELECT COALESCE(g.display_name, a.process_name) AS name,
                        SUM(a.duration_secs) AS secs,
                        COALESCE(c.id, 'other') AS cat
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
                    AND g.category_id IS NOT 'hidden'
                    {}
                    {}
                  GROUP BY name, cat
                  ORDER BY secs DESC
                  LIMIT ?",
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
            let lim = limit as i64;
            params.push(&lim);
            let mut stmt = conn.prepare(&sql).db()?;
            let it = stmt
                .query_map(params.as_slice(), |r| {
                    let name: String = r.get(0)?;
                    let secs: i64 = r.get(1)?;
                    let cat: String = r.get(2)?;
                    Ok((name, secs, cat))
                })
                .db()?;
            let mut out = Vec::new();
            for row in it {
                let (name, secs, cat) = row.db()?;
                let minutes = (secs / 60).max(0) as u32;
                out.push((name, minutes, cat));
            }
            Ok(out)
        })
        .await?;
    Ok(rows)
}

/// 拉某段日期范围内（含两端）使用最多的应用（display_name, minutes, category_id），按 minutes 降序。
///
/// 跟 [`list_segment_top_apps`] 的区别仅在 WHERE：按日期范围而非"某天某小时窗口"；
/// 周报 step2 用这个拼 user prompt，给 LLM 一份整周 top apps 切片。
pub async fn list_range_top_apps(
    pool: &DbPool,
    start_date: &str,
    end_date: &str,
    excluded_categories: &[String],
    device: DeviceFilter,
    limit: u32,
) -> Result<Vec<(String, u32, String)>> {
    let from = start_date.to_string();
    let to = end_date.to_string();
    let excluded: Vec<String> = excluded_categories.to_vec();
    let dev = device.clone();
    let rows = pool
        .0
        .call(move |conn| {
            let placeholders = if excluded.is_empty() {
                String::new()
            } else {
                let marks = vec!["?"; excluded.len()].join(",");
                format!(" AND COALESCE(c.id, 'other') NOT IN ({})", marks)
            };
            // 硬编码排除 hidden 分类（不在 excluded_categories 配置范畴内）
            let sql = format!(
                "SELECT COALESCE(g.display_name, a.process_name) AS name,
                        SUM(a.duration_secs) AS secs,
                        COALESCE(c.id, 'other') AS cat
                   FROM activities a
              LEFT JOIN app_group_members gm
                     ON gm.process_name = a.process_name AND gm.deleted_at IS NULL
              LEFT JOIN app_groups g
                     ON g.id = gm.group_id AND g.deleted_at IS NULL
              LEFT JOIN categories c
                     ON c.id = g.category_id AND c.deleted_at IS NULL
                  WHERE a.local_date >= ?
                    AND a.local_date <= ?
                    AND g.category_id IS NOT 'hidden'
                    {}
                    {}
                  GROUP BY name, cat
                  ORDER BY secs DESC
                  LIMIT ?",
                placeholders,
                dev.sql_clause(),
            );
            let mut params: Vec<&dyn ToSql> = Vec::new();
            params.push(&from);
            params.push(&to);
            for cat in &excluded {
                params.push(cat);
            }
            if let Some(extra) = dev.extra_param() {
                params.push(extra);
            }
            let lim = limit as i64;
            params.push(&lim);
            let mut stmt = conn.prepare(&sql).db()?;
            let it = stmt
                .query_map(params.as_slice(), |r| {
                    let name: String = r.get(0)?;
                    let secs: i64 = r.get(1)?;
                    let cat: String = r.get(2)?;
                    Ok((name, secs, cat))
                })
                .db()?;
            let mut out = Vec::new();
            for row in it {
                let (name, secs, cat) = row.db()?;
                let minutes = (secs / 60).max(0) as u32;
                out.push((name, minutes, cat));
            }
            Ok(out)
        })
        .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};

    /// 造一行合法的 ok 段总结；content 里编入 (source, date, idx) 指纹，
    /// 方便断言"读到的确实是这一支写进去的行"而不是撞了别的 source。
    fn seg(source: &str, date: &str, idx: u32) -> SegmentSummaryRow {
        SegmentSummaryRow {
            source: source.to_string(),
            local_date: date.to_string(),
            segment_idx: idx,
            label: format!("段{idx}"),
            start_hour: 9,
            end_hour: 12,
            content: format!("{source}/{date}/{idx} 的总结正文"),
            model: "test-model.gguf".to_string(),
            status: "ok".to_string(),
            error: None,
            generated_at: "2026-07-20T01:02:03+00:00".to_string(),
        }
    }

    async fn insert_activity(
        pool: &DbPool,
        device_id: &str,
        local_date: &str,
        local_hour: u8,
        process_name: &str,
        duration_secs: i64,
    ) {
        let device_id = device_id.to_string();
        let local_date = local_date.to_string();
        let process_name = process_name.to_string();
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activities(
                        started_at, ended_at, duration_secs, local_date, local_hour,
                        process_name, window_title, category_id, device_id, updated_at, origin
                     ) VALUES(
                        ?1 || 'T10:00:00Z', ?1 || 'T10:00:30Z', ?2, ?1, ?3,
                        ?4, '', 'other', ?5, ?1 || 'T10:00:30Z', 'local'
                     )",
                    rusqlite::params![
                        local_date,
                        duration_secs,
                        local_hour as i64,
                        process_name,
                        device_id
                    ],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// 1:1 组：组 id = process_name，挂指定分类（迁移已 seed code/browse/hidden 等）。
    async fn seed_solo_group(pool: &DbPool, name: &str, category_id: &str) {
        let name = name.to_string();
        let category_id = category_id.to_string();
        pool.0
            .call(move |conn| {
                let now = "2026-07-20T00:00:00Z";
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

    async fn insert_image_desc(pool: &DbPool, source: &str, local_date: &str) {
        let source = source.to_string();
        let local_date = local_date.to_string();
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO ai_image_descriptions(
                        source, local_date, segment_idx, image_index,
                        screenshot_path, description, model, generated_at
                     ) VALUES(?1, ?2, 0, 0, '/tmp/x.webp', 'desc', 'm', '2026-07-20T00:00:00Z')",
                    rusqlite::params![source, local_date],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn count_image_descs(pool: &DbPool, source: &str, local_date: &str) -> i64 {
        let source = source.to_string();
        let local_date = local_date.to_string();
        pool.0
            .call(move |conn| {
                let n = conn
                    .query_row(
                        "SELECT COUNT(*) FROM ai_image_descriptions
                          WHERE source = ?1 AND local_date = ?2",
                        rusqlite::params![source, local_date],
                        |r| r.get::<_, i64>(0),
                    )
                    .db()?;
                Ok(n)
            })
            .await
            .unwrap()
    }

    /// 前端渲染依赖 get_day 按 segment_idx 升序——段是按时间排的时间轴。
    /// 真实翻车场景：并发/重试导致段乱序落库，若靠插入顺序读取，UI 会把
    /// "晚上"段画到"上午"前面。这里故意乱序 upsert 再验证读出顺序。
    #[tokio::test]
    async fn get_day_sorts_by_segment_idx_and_roundtrips_fields() {
        let pool = fresh_test_pool().await;
        for idx in [2u32, 0, 1] {
            let mut r = seg("daily", "2026-07-20", idx);
            r.start_hour = 6 + (idx * 4) as u8;
            r.end_hour = r.start_hour + 4;
            upsert_segment(&pool, &r).await.unwrap();
        }

        let rows = get_day(&pool, "daily", "2026-07-20").await.unwrap();
        let idxs: Vec<u32> = rows.iter().map(|r| r.segment_idx).collect();
        assert_eq!(
            idxs,
            vec![0, 1, 2],
            "必须按 segment_idx 升序，与写入顺序无关"
        );

        // 字段完整往返：label/hours/content/model 任一列错位都意味着 SELECT
        // 列序和 struct 字段对不上（新增列时最容易犯）。
        let r1 = &rows[1];
        assert_eq!(r1.label, "段1");
        assert_eq!((r1.start_hour, r1.end_hour), (10, 14));
        assert_eq!(r1.content, "daily/2026-07-20/1 的总结正文");
        assert_eq!(r1.model, "test-model.gguf");
        assert_eq!(r1.status, "ok");
        assert_eq!(r1.error, None);
    }

    /// daily / debug / weekly 共用一张表，只靠 PK 里的 source 区分。
    /// 真实翻车场景：调试 tab 重跑总结把日报页当天数据覆盖掉（v21 迁移就是
    /// 为修这个）。同 (date, idx) 三支各写一行，互相不覆盖、互相读不到。
    #[tokio::test]
    async fn sources_are_fully_isolated() {
        let pool = fresh_test_pool().await;
        let date = "2026-07-20";
        for src in ["daily", "debug", "weekly"] {
            upsert_segment(&pool, &seg(src, date, 0)).await.unwrap();
        }
        // debug 支再覆盖一次自己——不能波及 daily/weekly
        let mut dbg2 = seg("debug", date, 0);
        dbg2.content = "debug 第二版".to_string();
        upsert_segment(&pool, &dbg2).await.unwrap();

        for src in ["daily", "weekly"] {
            let rows = get_day(&pool, src, date).await.unwrap();
            assert_eq!(rows.len(), 1, "{src} 支应只有自己那一行");
            assert_eq!(
                rows[0].content,
                format!("{src}/{date}/0 的总结正文"),
                "{src} 支内容被别的 source 覆盖了"
            );
        }
        let dbg = get_day(&pool, "debug", date).await.unwrap();
        assert_eq!(dbg.len(), 1);
        assert_eq!(dbg[0].content, "debug 第二版");

        // get_segment_status 同样按 source 隔离：查不存在的 source 必须是 None，
        // 否则 Phase 2 会把 debug 的状态误判成 daily 已跑过而跳段。
        assert_eq!(
            get_segment_status(&pool, "daily", date, 0).await.unwrap(),
            Some("ok".to_string())
        );
        assert_eq!(
            get_segment_status(&pool, "nonexistent", date, 0)
                .await
                .unwrap(),
            None
        );
    }

    /// 同 (source, date, segment) 重跑必须整行覆盖。真实翻车场景：某段先失败
    /// 落了 error 行，用户重跑成功后若 error 字段没被 excluded.error 刷掉，
    /// UI 会在正常总结旁边还挂着红色错误 badge。
    #[tokio::test]
    async fn upsert_same_key_overwrites_entire_row() {
        let pool = fresh_test_pool().await;
        let date = "2026-07-20";

        let mut bad = seg("daily", date, 0);
        bad.status = "error".to_string();
        bad.content = String::new();
        bad.error = Some("LLM 超时".to_string());
        upsert_segment(&pool, &bad).await.unwrap();

        // 重跑成功：换了 label/hours（用户中途改过段配置）+ 换了模型
        let mut ok = seg("daily", date, 0);
        ok.label = "重跑后的段".to_string();
        ok.start_hour = 7;
        ok.end_hour = 11;
        ok.model = "new-model.gguf".to_string();
        upsert_segment(&pool, &ok).await.unwrap();

        let rows = get_day(&pool, "daily", date).await.unwrap();
        assert_eq!(rows.len(), 1, "UPSERT 应覆盖旧行而不是新增一行");
        let r = &rows[0];
        assert_eq!(r.status, "ok");
        assert_eq!(r.error, None, "旧 error 文本必须被刷掉");
        assert_eq!(r.label, "重跑后的段");
        assert_eq!((r.start_hour, r.end_hour), (7, 11));
        assert_eq!(r.model, "new-model.gguf");
        assert_eq!(r.content, "daily/2026-07-20/0 的总结正文");
    }

    /// skipped / error 状态行的落库与读取。真实翻车场景：error 字段是 Option，
    /// SELECT 时如果按 NOT NULL 读会直接 panic；skipped 行 content 为空串，
    /// 前端靠 status 而不是 content 判断展示分支。
    #[tokio::test]
    async fn skipped_and_error_rows_roundtrip() {
        let pool = fresh_test_pool().await;
        let date = "2026-07-20";

        let mut skipped = seg("daily", date, 0);
        skipped.status = "skipped_no_screenshots".to_string();
        skipped.content = String::new();
        upsert_segment(&pool, &skipped).await.unwrap();

        let mut err = seg("daily", date, 1);
        err.status = "error".to_string();
        err.content = String::new();
        err.error = Some("connection refused (port 8080)".to_string());
        upsert_segment(&pool, &err).await.unwrap();

        let rows = get_day(&pool, "daily", date).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].status, "skipped_no_screenshots");
        assert_eq!(rows[0].content, "");
        assert_eq!(rows[0].error, None);
        assert_eq!(rows[1].status, "error");
        assert_eq!(rows[1].content, "");
        assert_eq!(
            rows[1].error.as_deref(),
            Some("connection refused (port 8080)")
        );

        // get_segment_status 是 Phase 2 区分「真无截图」vs「step1 全失败」的依据
        assert_eq!(
            get_segment_status(&pool, "daily", date, 0).await.unwrap(),
            Some("skipped_no_screenshots".to_string())
        );
        assert_eq!(
            get_segment_status(&pool, "daily", date, 1).await.unwrap(),
            Some("error".to_string())
        );
        assert_eq!(
            get_segment_status(&pool, "daily", date, 2).await.unwrap(),
            None,
            "没跑过的段必须返回 None，返回空串会让调用方误判已跑"
        );
    }

    /// generated_at 空串时自动补当前 UTC；调用方显式给了就原样存。
    /// 真实翻车场景：weekly_runner 自己填了 generated_at，若被 upsert 覆盖成
    /// now()，前端"生成于"时间戳会随每次无关写入漂移。
    #[tokio::test]
    async fn generated_at_autofill_and_preserve() {
        let pool = fresh_test_pool().await;

        let mut auto = seg("daily", "2026-07-20", 0);
        auto.generated_at = String::new();
        upsert_segment(&pool, &auto).await.unwrap();

        let mut explicit = seg("daily", "2026-07-20", 1);
        explicit.generated_at = "2025-12-31T23:59:59+00:00".to_string();
        upsert_segment(&pool, &explicit).await.unwrap();

        let rows = get_day(&pool, "daily", "2026-07-20").await.unwrap();
        assert!(
            chrono::DateTime::parse_from_rfc3339(&rows[0].generated_at).is_ok(),
            "自动补的 generated_at 应是合法 RFC3339，实际: {:?}",
            rows[0].generated_at
        );
        assert_eq!(rows[1].generated_at, "2025-12-31T23:59:59+00:00");
    }

    /// get_range 是周报唯一的日报取数入口：闭区间 + (date, idx) 双键升序 +
    /// source 过滤。真实翻车场景：边界日期用了 > 而不是 >= 会让周一或周日的
    /// 日报默默从周报材料里消失。
    #[tokio::test]
    async fn get_range_is_inclusive_sorted_and_source_scoped() {
        let pool = fresh_test_pool().await;
        // 乱序写入：区间内 4 行 + 区间外 2 行 + 区间内的 debug 干扰行
        for (date, idx) in [
            ("2026-07-22", 0u32),
            ("2026-07-20", 1),
            ("2026-07-19", 0), // 起点前一天：不该出现
            ("2026-07-21", 0),
            ("2026-07-23", 0), // 终点后一天：不该出现
            ("2026-07-20", 0),
        ] {
            upsert_segment(&pool, &seg("daily", date, idx))
                .await
                .unwrap();
        }
        upsert_segment(&pool, &seg("debug", "2026-07-21", 0))
            .await
            .unwrap();

        let rows = get_range(&pool, "daily", "2026-07-20", "2026-07-22")
            .await
            .unwrap();
        let keys: Vec<(String, u32)> = rows
            .iter()
            .map(|r| (r.local_date.clone(), r.segment_idx))
            .collect();
        assert_eq!(
            keys,
            vec![
                ("2026-07-20".to_string(), 0),
                ("2026-07-20".to_string(), 1),
                ("2026-07-21".to_string(), 0),
                ("2026-07-22".to_string(), 0),
            ],
            "闭区间两端都要含，且按 (local_date, segment_idx) 升序"
        );
        assert!(
            rows.iter().all(|r| r.source == "daily"),
            "debug 行混进了周报材料"
        );
    }

    /// upsert_skipped_no_activity 是"整段无活动"的兜底落库路径。
    /// 真实翻车场景：兜底行 status 拼错或 content 不为空，前端会把空白段
    /// 当成正常总结渲染出一张空卡片；覆盖旧 error 行失败则重跑后错误提示不消失。
    #[tokio::test]
    async fn upsert_skipped_no_activity_writes_placeholder_and_overwrites() {
        let pool = fresh_test_pool().await;
        let date = "2026-07-20";

        // 先落一行 error（模拟前一轮 LLM 失败），兜底必须能覆盖它
        let mut bad = seg("daily", date, 3);
        bad.status = "error".to_string();
        bad.error = Some("boom".to_string());
        upsert_segment(&pool, &bad).await.unwrap();

        crate::ai::summary_operations::upsert_skipped_no_activity(
            &pool,
            "daily",
            date,
            3,
            "下午",
            13,
            18,
            "test-model.gguf".to_string(),
        )
        .await
        .unwrap();

        let rows = get_day(&pool, "daily", date).await.unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.status, "skipped_no_activity");
        assert_eq!(r.content, "", "兜底行不该有正文");
        assert_eq!(r.error, None, "兜底行必须清掉旧 error");
        assert_eq!(r.label, "下午");
        assert_eq!((r.start_hour, r.end_hour), (13, 18));
        assert!(
            chrono::DateTime::parse_from_rfc3339(&r.generated_at).is_ok(),
            "generated_at 应自动填合法时间戳"
        );
    }

    /// clear_day 是 force_refresh 的实现：同 source 同日的段总结 + 逐图描述
    /// 一起清，但绝不能误删别的 source / 别的日期。真实翻车场景：调试 tab 的
    /// 删除按钮把日报页当天数据也清了。
    #[tokio::test]
    async fn clear_day_scopes_to_source_and_date() {
        let pool = fresh_test_pool().await;
        upsert_segment(&pool, &seg("daily", "2026-07-20", 0))
            .await
            .unwrap();
        upsert_segment(&pool, &seg("daily", "2026-07-21", 0))
            .await
            .unwrap();
        upsert_segment(&pool, &seg("debug", "2026-07-20", 0))
            .await
            .unwrap();
        insert_image_desc(&pool, "daily", "2026-07-20").await;
        insert_image_desc(&pool, "debug", "2026-07-20").await;

        clear_day(&pool, "daily", "2026-07-20").await.unwrap();

        assert!(
            get_day(&pool, "daily", "2026-07-20")
                .await
                .unwrap()
                .is_empty(),
            "目标日应被清空"
        );
        assert_eq!(count_image_descs(&pool, "daily", "2026-07-20").await, 0);
        // 旁支全部幸存
        assert_eq!(
            get_day(&pool, "daily", "2026-07-21").await.unwrap().len(),
            1
        );
        assert_eq!(
            get_day(&pool, "debug", "2026-07-20").await.unwrap().len(),
            1
        );
        assert_eq!(count_image_descs(&pool, "debug", "2026-07-20").await, 1);
    }

    /// clear_day_summaries_only 只清段总结、留逐图描述——step2 重跑复用 step1
    /// 产物省 token 就靠这个差别。真实翻车场景：这里误连带删了描述，重跑就得
    /// 重新过一遍 vision 推理（CPU 上一段两三分钟）。
    #[tokio::test]
    async fn clear_day_summaries_only_keeps_image_descriptions() {
        let pool = fresh_test_pool().await;
        upsert_segment(&pool, &seg("daily", "2026-07-20", 0))
            .await
            .unwrap();
        insert_image_desc(&pool, "daily", "2026-07-20").await;

        clear_day_summaries_only(&pool, "daily", "2026-07-20")
            .await
            .unwrap();

        assert!(get_day(&pool, "daily", "2026-07-20")
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            count_image_descs(&pool, "daily", "2026-07-20").await,
            1,
            "逐图描述必须保留"
        );
    }

    /// list_segment_top_apps 的小时窗口是 [start, end) 半开区间，秒数换算分钟
    /// 向下取整，hidden 分类硬排除、未分组进程按 'other' 放行。真实翻车场景：
    /// end_hour 写成 <= 会把下一段开头的应用算进本段，LLM 拿到串场材料。
    #[tokio::test]
    async fn list_segment_top_apps_window_order_and_filters() {
        let pool = fresh_test_pool().await;
        let date = "2026-07-20";
        seed_solo_group(&pool, "Code", "code").await;
        seed_solo_group(&pool, "Chrome", "browse").await;
        seed_solo_group(&pool, "Secret", "hidden").await;

        // 窗口 [9, 12)：Code 300+330=630s → 10 分钟；Chrome 480s → 8 分钟
        insert_activity(&pool, TEST_SELF_ID, date, 9, "Code", 300).await;
        insert_activity(&pool, TEST_SELF_ID, date, 10, "Code", 330).await;
        insert_activity(&pool, TEST_SELF_ID, date, 9, "Chrome", 480).await;
        // 未分组进程：COALESCE 兜底成 'other'，120s → 2 分钟
        insert_activity(&pool, TEST_SELF_ID, date, 11, "Mystery", 120).await;
        // 窗口外：8 点（起点前）和 12 点（半开区间终点）都不能算进来
        insert_activity(&pool, TEST_SELF_ID, date, 8, "Code", 3600).await;
        insert_activity(&pool, TEST_SELF_ID, date, 12, "Code", 3600).await;
        // hidden 分类：时长再大也不能出现
        insert_activity(&pool, TEST_SELF_ID, date, 9, "Secret", 6000).await;
        // 别的设备的 Chrome：Only(self) 时不能算
        insert_activity(&pool, "device-win", date, 9, "Chrome", 6000).await;

        let rows = list_segment_top_apps(
            &pool,
            date,
            9,
            12,
            &[],
            DeviceFilter::Only(TEST_SELF_ID.to_string()),
            8,
        )
        .await
        .unwrap();
        assert_eq!(
            rows,
            vec![
                ("Code".to_string(), 10, "code".to_string()),
                ("Chrome".to_string(), 8, "browse".to_string()),
                ("Mystery".to_string(), 2, "other".to_string()),
            ],
            "分钟数按窗口内秒数向下取整，降序排列，hidden/窗口外/他机行为均排除"
        );

        // All 视角：两台设备的 Chrome 合并 (480+6000)/60 = 108 分钟，跃居第一
        let all = list_segment_top_apps(&pool, date, 9, 12, &[], DeviceFilter::All, 8)
            .await
            .unwrap();
        assert_eq!(all[0], ("Chrome".to_string(), 108, "browse".to_string()));

        // excluded_categories 把 browse 整类踢掉
        let no_browse = list_segment_top_apps(
            &pool,
            date,
            9,
            12,
            &["browse".to_string()],
            DeviceFilter::Only(TEST_SELF_ID.to_string()),
            8,
        )
        .await
        .unwrap();
        // 精确比对整个结果：只踢 browse 一类，Code / Mystery(other) 必须原样保留——
        // 只断言"没有 browse"会漏掉「误把 other 一起排掉」这类过度过滤 bug
        assert_eq!(
            no_browse,
            vec![
                ("Code".to_string(), 10, "code".to_string()),
                ("Mystery".to_string(), 2, "other".to_string()),
            ],
            "excluded_categories 应只踢掉 browse 整类，其余行原样保留"
        );

        // limit 截断：只留时长第一名
        let top1 = list_segment_top_apps(
            &pool,
            date,
            9,
            12,
            &[],
            DeviceFilter::Only(TEST_SELF_ID.to_string()),
            1,
        )
        .await
        .unwrap();
        assert_eq!(top1.len(), 1);
        assert_eq!(top1[0].0, "Code");
    }

    /// list_range_top_apps 按日期闭区间聚合，跨天同 app 时长要合并成一行。
    /// 真实翻车场景：边界日用开区间会把周一/周日的使用时长丢掉，周报 top apps
    /// 跟日报对不上数。
    #[tokio::test]
    async fn list_range_top_apps_inclusive_range_and_cross_day_sum() {
        let pool = fresh_test_pool().await;
        seed_solo_group(&pool, "Code", "code").await;
        seed_solo_group(&pool, "Chrome", "browse").await;

        // Code 恰好压在区间两端：20 号 300s + 22 号 300s → 合并 10 分钟
        insert_activity(&pool, TEST_SELF_ID, "2026-07-20", 9, "Code", 300).await;
        insert_activity(&pool, TEST_SELF_ID, "2026-07-22", 9, "Code", 300).await;
        insert_activity(&pool, TEST_SELF_ID, "2026-07-21", 9, "Chrome", 60).await;
        // 区间外前后各一天：不能混进来
        insert_activity(&pool, TEST_SELF_ID, "2026-07-19", 9, "Chrome", 60000).await;
        insert_activity(&pool, TEST_SELF_ID, "2026-07-23", 9, "Chrome", 60000).await;

        let rows =
            list_range_top_apps(&pool, "2026-07-20", "2026-07-22", &[], DeviceFilter::All, 8)
                .await
                .unwrap();
        assert_eq!(
            rows,
            vec![
                ("Code".to_string(), 10, "code".to_string()),
                ("Chrome".to_string(), 1, "browse".to_string()),
            ],
            "两端日期必须计入，区间外大时长行必须排除"
        );
    }
}
