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
