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

use chrono::Utc;
use rusqlite::types::ToSql;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::storage::SqliteResultExt;
use crate::error::Result;
use crate::repo::reports::DeviceFilter;
use crate::storage::DbPool;

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

/// 写入或覆盖一段。`generated_at` 自动用当前 UTC 时间填，调用方不用管。
/// PK = (source, local_date, segment_idx)，所以 daily / debug 互不冲突。
pub async fn upsert_segment(pool: &DbPool, row: &SegmentSummaryRow) -> Result<()> {
    let mut row = row.clone();
    if row.generated_at.is_empty() {
        row.generated_at = Utc::now().to_rfc3339();
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

/// 只清当天段总结，**不**动逐图描述（step 1 产物）。
/// step2-only 路径调：用户想用已有 image descriptions 重跑段总结，必须保留 step 1 数据。
pub async fn clear_day_summaries_only(
    pool: &DbPool,
    source: &str,
    local_date: &str,
) -> Result<()> {
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

/// 只清当天逐图描述，**不**动段总结。
/// 调试 tab 的「逐图描述」Section 删除按钮用——用户想清掉现有描述重跑 step 1，但不想动段总结。
pub async fn clear_day_image_descriptions_only(
    pool: &DbPool,
    source: &str,
    local_date: &str,
) -> Result<()> {
    let src = source.to_string();
    let date = local_date.to_string();
    pool.0
        .call(move |conn| {
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

/// 清掉某 source 某段所有逐图描述（段重跑时 step 1 开始前调，避免新旧 image_index 错位）。
pub async fn clear_segment_descriptions(
    pool: &DbPool,
    source: &str,
    local_date: &str,
    segment_idx: u32,
) -> Result<()> {
    let src = source.to_string();
    let date = local_date.to_string();
    pool.0
        .call(move |conn| {
            conn.execute(
                "DELETE FROM ai_image_descriptions
                  WHERE source = ?1 AND local_date = ?2 AND segment_idx = ?3",
                rusqlite::params![src, date, segment_idx as i64],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// 单张图的描述行（DB <-> 前端共用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageDescriptionRow {
    /// "daily" / "debug" — 跟 SegmentSummaryRow.source 同义
    pub source: String,
    pub local_date: String,
    pub segment_idx: u32,
    /// 该段抽帧后的 0-based 顺序
    pub image_index: u32,
    /// 截图绝对路径
    pub screenshot_path: String,
    /// LLM 输出的描述文本
    pub description: String,
    /// 生成时用的 active_main 文件名
    pub model: String,
    pub generated_at: String,
    /// 单张图调用 LLM 的总耗时（毫秒）；llama-server 没返 usage / 出错时为 None
    pub latency_ms: Option<u64>,
    /// LLM 响应里 usage.prompt_tokens；可能为 None（部分 server 版本不返）
    pub prompt_tokens: Option<u32>,
    /// LLM 响应里 usage.completion_tokens
    pub completion_tokens: Option<u32>,
}

/// 写入或覆盖一张图的描述。`generated_at` 为空时自动填当前 UTC。
pub async fn upsert_image_description(
    pool: &DbPool,
    row: &ImageDescriptionRow,
) -> Result<()> {
    let mut row = row.clone();
    if row.generated_at.is_empty() {
        row.generated_at = Utc::now().to_rfc3339();
    }
    pool.0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO ai_image_descriptions(
                     source, local_date, segment_idx, image_index,
                     screenshot_path, description, model, generated_at,
                     latency_ms, prompt_tokens, completion_tokens)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(source, local_date, segment_idx, image_index) DO UPDATE SET
                     screenshot_path   = excluded.screenshot_path,
                     description       = excluded.description,
                     model             = excluded.model,
                     generated_at      = excluded.generated_at,
                     latency_ms        = excluded.latency_ms,
                     prompt_tokens     = excluded.prompt_tokens,
                     completion_tokens = excluded.completion_tokens",
                rusqlite::params![
                    row.source,
                    row.local_date,
                    row.segment_idx as i64,
                    row.image_index as i64,
                    row.screenshot_path,
                    row.description,
                    row.model,
                    row.generated_at,
                    row.latency_ms.map(|v| v as i64),
                    row.prompt_tokens.map(|v| v as i64),
                    row.completion_tokens.map(|v| v as i64),
                ],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// 拉某 source 某段所有逐图描述，按 image_index 升序——给调试 tab 渲染列表。
pub async fn get_segment_image_descriptions(
    pool: &DbPool,
    source: &str,
    local_date: &str,
    segment_idx: u32,
) -> Result<Vec<ImageDescriptionRow>> {
    let src = source.to_string();
    let date = local_date.to_string();
    let rows = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT source, local_date, segment_idx, image_index,
                            screenshot_path, description, model, generated_at,
                            latency_ms, prompt_tokens, completion_tokens
                       FROM ai_image_descriptions
                      WHERE source = ?1 AND local_date = ?2 AND segment_idx = ?3
                      ORDER BY image_index ASC",
                )
                .db()?;
            let it = stmt
                .query_map(rusqlite::params![src, date, segment_idx as i64], |r| {
                    Ok(ImageDescriptionRow {
                        source: r.get(0)?,
                        local_date: r.get(1)?,
                        segment_idx: r.get::<_, i64>(2)? as u32,
                        image_index: r.get::<_, i64>(3)? as u32,
                        screenshot_path: r.get(4)?,
                        description: r.get(5)?,
                        model: r.get(6)?,
                        generated_at: r.get(7)?,
                        latency_ms: r.get::<_, Option<i64>>(8)?.map(|v| v as u64),
                        prompt_tokens: r.get::<_, Option<i64>>(9)?.map(|v| v as u32),
                        completion_tokens: r.get::<_, Option<i64>>(10)?.map(|v| v as u32),
                    })
                })
                .db()?;
            let mut out = Vec::new();
            for row in it {
                out.push(row.db()?);
            }
            Ok(out)
        })
        .await?;
    Ok(rows)
}

/// 拉某 source 某天所有段的逐图描述——调试 tab 一次性渲染整日时用。
pub async fn get_day_image_descriptions(
    pool: &DbPool,
    source: &str,
    local_date: &str,
) -> Result<Vec<ImageDescriptionRow>> {
    let src = source.to_string();
    let date = local_date.to_string();
    let rows = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT source, local_date, segment_idx, image_index,
                            screenshot_path, description, model, generated_at,
                            latency_ms, prompt_tokens, completion_tokens
                       FROM ai_image_descriptions
                      WHERE source = ?1 AND local_date = ?2
                      ORDER BY segment_idx ASC, image_index ASC",
                )
                .db()?;
            let it = stmt
                .query_map(rusqlite::params![src, date], |r| {
                    Ok(ImageDescriptionRow {
                        source: r.get(0)?,
                        local_date: r.get(1)?,
                        segment_idx: r.get::<_, i64>(2)? as u32,
                        image_index: r.get::<_, i64>(3)? as u32,
                        screenshot_path: r.get(4)?,
                        description: r.get(5)?,
                        model: r.get(6)?,
                        generated_at: r.get(7)?,
                        latency_ms: r.get::<_, Option<i64>>(8)?.map(|v| v as u64),
                        prompt_tokens: r.get::<_, Option<i64>>(9)?.map(|v| v as u32),
                        completion_tokens: r.get::<_, Option<i64>>(10)?.map(|v| v as u32),
                    })
                })
                .db()?;
            let mut out = Vec::new();
            for row in it {
                out.push(row.db()?);
            }
            Ok(out)
        })
        .await?;
    Ok(rows)
}

/// 一张截图的元数据——用来给 step 1 单图描述 prompt 灌入「这张截图来自的应用 + 分类」。
///
/// 来源全是已有数据：activities.process_name → app_group_members → app_groups（拿 display_name）→ categories（拿 name）。
/// 用户没建 app_group 时 `app_display` 兜底为 process_name；没分配 category 时 `category_name` 为 None。
#[derive(Debug, Clone)]
pub struct ScreenshotMeta {
    pub path: String,
    /// COALESCE(g.display_name, a.process_name)
    pub app_display: String,
    /// 来自 categories.name；NULL → None
    pub category_name: Option<String>,
}

/// 拉某天某段（`[start_hour, end_hour)`）的截图列表（含应用元数据），按时间排序。
///
/// 过滤：
/// - `screenshot_path IS NOT NULL` 跳过没截图的活动
/// - `excluded_categories` 里的分类不参与（如用户排除 'other' 不分析）
/// - `device` 复用 reports.rs 的 DeviceFilter 模式
///
/// 返回的 `path` 是 activities.screenshot_path 字段原值——可能是绝对路径，也可能是
/// 相对 data_root 的相对路径，由 capture 写入时决定。调用方负责拼绝对路径再读盘。
pub async fn list_segment_screenshots(
    pool: &DbPool,
    local_date: &str,
    start_hour: u8,
    end_hour: u8,
    excluded_categories: &[String],
    device: DeviceFilter,
) -> Result<Vec<ScreenshotMeta>> {
    let date = local_date.to_string();
    let excluded: Vec<String> = excluded_categories.to_vec();
    let dev = device.clone();
    let rows = pool
        .0
        .call(move |conn| {
            // excluded 数组动态拼 ?,?,? 占位
            let placeholders = if excluded.is_empty() {
                String::new()
            } else {
                let marks = vec!["?"; excluded.len()].join(",");
                format!(" AND COALESCE(c.id, 'other') NOT IN ({})", marks)
            };
            let sql = format!(
                "SELECT a.screenshot_path,
                        COALESCE(g.display_name, a.process_name) AS app_display,
                        c.name AS category_name
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
                    AND a.screenshot_path IS NOT NULL
                    AND a.screenshot_path <> ''
                    {}
                    {}
                  ORDER BY a.started_at",
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
                    Ok(ScreenshotMeta {
                        path: r.get::<_, String>(0)?,
                        app_display: r.get::<_, String>(1)?,
                        category_name: r.get::<_, Option<String>>(2)?,
                    })
                })
                .db()?;
            let mut out = Vec::new();
            for row in it {
                out.push(row.db()?);
            }
            Ok(out)
        })
        .await?;
    Ok(rows)
}

/// 反查：给定一个 screenshot_path，查 activities 拿对应的应用元数据。
///
/// 给 retry_one_image_description 用——重跑单张图时只有 ai_image_descriptions 行
/// （含 path）能拿到，需要从 activities 反查 app/category 才能重建 prompt。
///
/// 返回 None 时表示：
/// - 该 path 对应的 activities 行已被删除（理论极少）
/// - 或者 path 写入时跟当前活动行不一致（不应发生）
///
/// 调用方应兜底用 path 当 app 名继续跑，不能让重跑因元数据丢失而失败。
pub async fn get_screenshot_meta(pool: &DbPool, path: &str) -> Result<Option<ScreenshotMeta>> {
    let path_owned = path.to_string();
    let row = pool
        .0
        .call(move |conn| {
            let sql = "SELECT a.screenshot_path,
                              COALESCE(g.display_name, a.process_name) AS app_display,
                              c.name AS category_name
                         FROM activities a
                    LEFT JOIN app_group_members gm
                           ON gm.process_name = a.process_name AND gm.deleted_at IS NULL
                    LEFT JOIN app_groups g
                           ON g.id = gm.group_id AND g.deleted_at IS NULL
                    LEFT JOIN categories c
                           ON c.id = g.category_id AND c.deleted_at IS NULL
                        WHERE a.screenshot_path = ?
                        LIMIT 1";
            let row = conn
                .query_row(sql, [&path_owned], |r| {
                    Ok(ScreenshotMeta {
                        path: r.get::<_, String>(0)?,
                        app_display: r.get::<_, String>(1)?,
                        category_name: r.get::<_, Option<String>>(2)?,
                    })
                })
                .optional()
                .db()?;
            Ok(row)
        })
        .await?;
    Ok(row)
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
