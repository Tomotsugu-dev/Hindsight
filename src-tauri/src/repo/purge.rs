//! Deletes a set of activity records, together with their screenshots and screen memory.
//!
//! Screen memory lives in three tables of the memory database:
//! - `frames`: screenshot paths, one row per screenshot. It also records whether OCR is done
//!   and which session the text went into.
//! - `text_sessions`: the text OCR found, called sessions. The text of screenshots taken one
//!   after another in the same window becomes one row, without repeated lines.
//! - `session_lines`: each line of OCR text in a session, with the screenshot where the line
//!   first appeared.
//!
//! When SQL needs a list, the whole list goes in as one JSON array parameter, and `json_each`
//! expands it.

use std::path::Path;

use chrono::{DateTime, Local};
use rusqlite::Connection;

use crate::error::{Error, Result};
use crate::memory::MemoryDb;
use crate::repo::app_groups;
use crate::repo::outbox::{enqueue_days, OutboxEntity};
use crate::repo::sql::{from_stats_category_sql, matching_rule_host_sql, FROM_MEMBER_GROUP};
use crate::storage::{utc_now_rfc3339, DbPool, DbResult, SqliteResultExt};

/// What to delete.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PurgeTarget {
    // TODO(ADR-0001): network-wide deletion: a `purge` sync entity (process names and
    // purgedAt) that makes peers run this same local purge and rewrites the cloud day files.
    /// "Delete anyway": the records of every process in this app group. The group is
    /// soft-deleted afterwards.
    AppGroup { group_id: String },
    /// The activity records of one website: rows whose host is `host`, and rows of subdomains
    /// that follow the rule of `host`. A subdomain does not count when it has its own rule, or
    /// follows a rule nearer to it than `host`.
    Site { host: String },
    /// The activity records of apps and websites in the `private` category. The same SQL as the
    /// reports decides which rows are private (ADR-0019).
    Private,
}

impl PurgeTarget {
    /// Whether the deletion is synced to the cloud.
    fn should_sync_deletion(&self) -> bool {
        match self {
            PurgeTarget::AppGroup { .. } => false,
            PurgeTarget::Site { .. } | PurgeTarget::Private => true,
        }
    }
}

/// How much [`purge`] deleted.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurgeReport {
    pub activities: u64,
    pub screenshots: u64,
    pub memory_sessions: u64,
}

/// How much [`purge`] would delete, shown in the confirm dialog.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataCount {
    pub activities: u64,
    /// Screenshot files still on disk.
    pub screenshots: u64,
    /// The total size of these screenshot files.
    pub screenshot_bytes: u64,
}

/// What to delete, all found before anything is deleted.
struct ToDelete {
    /// Process names. Their sessions and `frames` rows in the memory database are deleted.
    processes: Vec<String>,
    /// IDs of the activity rows to delete.
    rows: Vec<i64>,
    /// Paths of the screenshots that only the rows to delete use.
    screenshots: Vec<String>,
}

/// Deletes the records of `target` and returns how much was deleted.
///
/// It deletes:
/// - The activity rows.
/// - The screenshot files only these rows use, with their AI descriptions, embeddings and
///   dedup records.
/// - In the memory database, the sessions of these processes and the sessions that contain
///   these screenshots. A session goes as a whole if any one of its screenshots is deleted.
///
/// `AppGroup` deletes on this device only (ADR-0001). `Site` and `Private` also delete in the
/// cloud through the outbox, so the caller must pause sync first, or already be inside a sync
/// round.
/// `self_id` is the ID of this device.
///
/// # Errors
///
/// Returns an error when a screenshot file cannot be deleted. A file that is already gone does
/// not count. The main database is not touched yet at that point.
pub async fn purge(
    pool: &DbPool,
    mem: Option<&MemoryDb>,
    screenshot_root: &Path,
    self_id: &str,
    target: &PurgeTarget,
) -> Result<PurgeReport> {
    let to_delete = find(pool, target).await?;
    let mut report = PurgeReport::default();
    // Keep this order: memory database, screenshot files, main database. The first two steps
    // find their data through the screenshot paths in the activity rows. So the main database
    // goes last, and a run that fails at an earlier step can run again and still find it.
    if let Some(mem) = mem {
        if target.should_sync_deletion() {
            // Before the screen-memory sessions are deleted: after that, their dates cannot be
            // found.
            enqueue_memory_days(pool, mem, &to_delete).await?;
        }
        report.memory_sessions = delete_memory(mem, &to_delete).await?;
    }
    report.screenshots = delete_files(screenshot_root, &to_delete.screenshots).await?;
    let sync_as = target.should_sync_deletion().then_some(self_id);
    report.activities = delete_rows(pool, &to_delete, sync_as).await?;
    match target {
        PurgeTarget::AppGroup { group_id } => {
            delete_group(pool, &to_delete.processes, group_id).await?
        }
        PurgeTarget::Site { .. } | PurgeTarget::Private => {}
    }
    Ok(report)
}

/// Counts how much [`purge`] would delete for `target`. It changes no data.
pub async fn count(
    pool: &DbPool,
    screenshot_root: &Path,
    target: &PurgeTarget,
) -> Result<DataCount> {
    let to_delete = find(pool, target).await?;
    let (root, screenshots) = (screenshot_root.to_path_buf(), to_delete.screenshots);
    let (screenshots, screenshot_bytes) = tokio::task::spawn_blocking(move || {
        let files = screenshots
            .iter()
            .filter_map(|rel| std::fs::metadata(root.join(rel)).ok())
            .filter(|meta| meta.is_file());
        files.fold((0, 0), |(n, bytes), meta| (n + 1, bytes + meta.len()))
    })
    .await
    .map_err(|e| Error::Other(format!("screenshot size task failed: {e}")))?;
    Ok(DataCount {
        activities: to_delete.rows.len() as u64,
        screenshots,
        screenshot_bytes,
    })
}

/// 删掉本机从 `at` 起记下的活动和屏幕文字，返回删了多少。
///
/// - `at` 及以后开始的行删掉，连同它们独占的截图和截图衍生的数据；
/// - 跨过 `at` 的行，结束时刻改成 `at`；
/// - 屏幕文字按同样的时刻删，见 [`truncate_memory`]。
///
/// 改动和删除写进 outbox，调用方不用暂停同步。
pub async fn truncate_at(
    pool: &DbPool,
    mem: Option<&MemoryDb>,
    screenshot_root: &Path,
    self_id: &str,
    at: DateTime<Local>,
) -> Result<PurgeReport> {
    let (id, at_s) = (self_id.to_string(), at.to_rfc3339());
    let rows: Vec<i64> = pool
        .0
        .call(move |conn| {
            column(
                conn,
                "SELECT id FROM activities
                  WHERE device_id = ?1 AND julianday(started_at) >= julianday(?2)",
                [&id, &at_s],
            )
        })
        .await?;
    let screenshots = screenshots_only_used_by(pool, &rows).await?;
    let to_delete = ToDelete {
        processes: Vec::new(),
        rows,
        screenshots,
    };
    end_rows_at(pool, self_id, at).await?;
    let mut report = PurgeReport {
        activities: delete_rows(pool, &to_delete, Some(self_id)).await?,
        ..PurgeReport::default()
    };
    if let Some(mem) = mem {
        report.memory_sessions = truncate_memory(pool, mem, at).await?;
    }
    // 文件放最后：行已经删了，统计就对了；文件删不掉只会留下没人用的文件。
    report.screenshots = delete_files(screenshot_root, &to_delete.screenshots).await?;
    Ok(report)
}

/// 本机的、从 `at` 起开始的文字段。
const SESSIONS_FROM: &str = "origin_device IS NULL AND julianday(started_ts) >= julianday(?1)";
/// 本机的、跨过 `at` 的文字段。
const SESSIONS_ACROSS: &str = "origin_device IS NULL
    AND julianday(started_ts) < julianday(?1) AND julianday(ended_ts) > julianday(?1)";

/// 删掉本机从 `at` 起识别出的屏幕文字，返回删了几段：
///
/// - `at` 及以后登记的截图删掉，还没识别的就不再识别；
/// - `at` 及以后开始的段整段删；
/// - 跨过 `at` 的段，删掉 `at` 及以后才第一次出现的行，剩下的行重新拼，结束时刻改成 `at`。
async fn truncate_memory(pool: &DbPool, mem: &MemoryDb, at: DateTime<Local>) -> Result<u64> {
    let at_s = at.to_rfc3339();
    let (sessions, days) = mem
        .0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            let days: Vec<String> = column(
                &tx,
                "SELECT DISTINCT local_date FROM text_sessions
                  WHERE origin_device IS NULL AND julianday(ended_ts) >= julianday(?1)",
                [&at_s],
            )?;
            tx.execute(
                &format!(
                    "DELETE FROM session_lines
                      WHERE session_id IN (SELECT id FROM text_sessions WHERE {SESSIONS_FROM})"
                ),
                [&at_s],
            )
            .db()?;
            let sessions = tx
                .execute(
                    &format!("DELETE FROM text_sessions WHERE {SESSIONS_FROM}"),
                    [&at_s],
                )
                .db()?;
            tx.execute(
                &format!(
                    "DELETE FROM session_lines
                      WHERE julianday(first_ts) >= julianday(?1)
                        AND session_id IN (SELECT id FROM text_sessions WHERE {SESSIONS_ACROSS})"
                ),
                [&at_s],
            )
            .db()?;
            tx.execute(
                &format!(
                    "UPDATE text_sessions
                        SET text = COALESCE((SELECT group_concat(l.text, char(10) ORDER BY l.line_no)
                                               FROM session_lines l
                                              WHERE l.session_id = text_sessions.id), ''),
                            ended_ts = ?1
                      WHERE {SESSIONS_ACROSS}"
                ),
                [&at_s],
            )
            .db()?;
            tx.execute(
                "DELETE FROM frames WHERE julianday(ts) >= julianday(?1)",
                [&at_s],
            )
            .db()?;
            tx.commit().db()?;
            Ok((sessions as u64, days))
        })
        .await?;
    // 先删再写 outbox：反过来的话，推送落在两步之间会传上没删的文字，还把 outbox 用掉。
    pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            enqueue_days(&tx, OutboxEntity::Memory, &days).db()?;
            tx.commit().db()
        })
        .await?;
    Ok(sessions)
}

/// 把本机跨过 `at` 的行的结束时刻改成 `at`，给它们的日子写 outbox。
async fn end_rows_at(pool: &DbPool, self_id: &str, at: DateTime<Local>) -> Result<()> {
    let (self_id, at_s, updated) = (self_id.to_string(), at.to_rfc3339(), utc_now_rfc3339());
    pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            let spanning: Vec<(i64, String, String)> = {
                let mut stmt = tx
                    .prepare(
                        "SELECT id, started_at, local_date FROM activities
                          WHERE device_id = ?1
                            AND julianday(started_at) < julianday(?2)
                            AND julianday(ended_at) > julianday(?2)",
                    )
                    .db()?;
                let rows = stmt
                    .query_map([&self_id, &at_s], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .db()?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .db()?;
                rows
            };
            let mut days: Vec<String> = Vec::new();
            for (id, started, day) in spanning {
                let secs = DateTime::parse_from_rfc3339(&started)
                    .map(|s| (at - s.with_timezone(&Local)).num_seconds())
                    .unwrap_or(0);
                tx.execute(
                    "UPDATE activities SET ended_at = ?2, duration_secs = ?3, updated_at = ?4
                      WHERE id = ?1",
                    rusqlite::params![id, at_s, secs, updated],
                )
                .db()?;
                if !days.contains(&day) {
                    days.push(day);
                }
            }
            enqueue_days(&tx, OutboxEntity::Activity, &days).db()?;
            tx.commit().db()
        })
        .await?;
    Ok(())
}

/// Finds the process names, activity rows and screenshots that `target` deletes.
async fn find(pool: &DbPool, target: &PurgeTarget) -> Result<ToDelete> {
    let processes = match target {
        PurgeTarget::AppGroup { group_id } => {
            app_groups::active_member_names(pool, group_id).await?
        }
        PurgeTarget::Site { .. } => Vec::new(),
        PurgeTarget::Private => {
            let sql = format!(
                "SELECT gm.process_name {FROM_MEMBER_GROUP}
                  WHERE gm.deleted_at IS NULL AND g.deleted_at IS NULL
                    AND g.category_id = 'private'"
            );
            pool.0.call(move |conn| column(conn, &sql, ())).await?
        }
    };
    let target = target.clone();
    let processes_json = serde_json::to_string(&processes)?;
    let rows: Vec<i64> = pool
        .0
        .call(move |conn| match target {
            PurgeTarget::AppGroup { .. } => column(
                conn,
                "SELECT id FROM activities
                  WHERE process_name IN (SELECT value FROM json_each(?1))",
                [processes_json],
            ),
            PurgeTarget::Site { host } => column(conn, &site_rows_sql(), [host]),
            PurgeTarget::Private => column(conn, &private_rows_sql(), ()),
        })
        .await?;
    let screenshots = screenshots_only_used_by(pool, &rows).await?;
    Ok(ToDelete {
        processes,
        rows,
        screenshots,
    })
}

/// Paths of the screenshots that only these rows use. Other rows still use the rest and keep
/// them.
async fn screenshots_only_used_by(pool: &DbPool, rows: &[i64]) -> Result<Vec<String>> {
    let rows_json = serde_json::to_string(rows)?;
    let screenshots = pool
        .0
        .call(move |conn| {
            column(
                conn,
                "SELECT screenshot_path FROM activities
                  WHERE screenshot_path IN (
                        SELECT screenshot_path FROM activities
                         WHERE id IN (SELECT value FROM json_each(?1)))
                  GROUP BY screenshot_path
                 HAVING MIN(id IN (SELECT value FROM json_each(?1))) = 1",
                [rows_json],
            )
        })
        .await?;
    Ok(screenshots)
}

/// IDs of the activity rows of website `?1`.
fn site_rows_sql() -> String {
    let of_site = "(url_host = ?1 OR substr(url_host, -length(?1) - 1) = '.' || ?1)";
    // A rule that a subdomain follows and that is not longer than `?1` is `?1` itself or a
    // parent domain. So once `?1` is set, the subdomain follows `?1`.
    format!(
        "WITH RECURSIVE {}
         SELECT a.id FROM activities a LEFT JOIN host_rule hr ON hr.host = a.url_host
          WHERE {of_site} AND (hr.rule_host IS NULL OR length(hr.rule_host) <= length(?1))",
        matching_rule_host_sql(&format!(
            "SELECT DISTINCT url_host FROM activities WHERE {of_site}"
        )),
    )
}

/// IDs of the activity rows in the `private` category. The category comes from the same SQL as
/// the reports ([`from_stats_category_sql`]).
fn private_rows_sql() -> String {
    format!(
        "WITH RECURSIVE {} SELECT a.id {} WHERE c.id = 'private'",
        matching_rule_host_sql("SELECT DISTINCT url_host FROM activities WHERE url_host <> ''"),
        from_stats_category_sql("activities"),
    )
}

/// The sessions to delete in the memory database: those of these processes (`?1`), and those
/// that contain a screenshot to delete (`?2`).
const SESSIONS_TO_DELETE: &str = "SELECT id FROM text_sessions
      WHERE app_id IN (SELECT value FROM json_each(?1))
     UNION
     SELECT session_id FROM frames WHERE path IN (SELECT value FROM json_each(?2))";

/// Writes the dates of the screen-memory sessions to delete into the outbox, one `memory` row
/// per day, so the next push uploads this device's screen-memory files for those days again.
/// Only sessions recorded on this device count (`text_sessions.origin_device` is NULL).
async fn enqueue_memory_days(pool: &DbPool, mem: &MemoryDb, to_delete: &ToDelete) -> Result<()> {
    let processes = serde_json::to_string(&to_delete.processes)?;
    let screenshots = serde_json::to_string(&to_delete.screenshots)?;
    let sql = format!(
        "SELECT DISTINCT local_date FROM text_sessions
          WHERE origin_device IS NULL AND id IN ({SESSIONS_TO_DELETE})"
    );
    let days: Vec<String> = mem
        .0
        .call(move |conn| column(conn, &sql, [&processes, &screenshots]))
        .await?;
    pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            enqueue_days(&tx, OutboxEntity::Memory, &days).db()?;
            tx.commit().db()
        })
        .await?;
    Ok(())
}

/// Deletes the sessions to delete, their lines, and the `frames` rows of the screenshots to
/// delete. Returns how many sessions were deleted.
async fn delete_memory(mem: &MemoryDb, to_delete: &ToDelete) -> Result<u64> {
    let processes = serde_json::to_string(&to_delete.processes)?;
    let screenshots = serde_json::to_string(&to_delete.screenshots)?;
    let sessions = mem
        .0
        .call(move |conn| {
            let params = [&processes, &screenshots];
            let tx = conn.transaction().db()?;
            // `session_lines` first: it has no foreign key, so its rows cannot be found once the
            // session is gone.
            tx.execute(
                &format!("DELETE FROM session_lines WHERE session_id IN ({SESSIONS_TO_DELETE})"),
                params,
            )
            .db()?;
            // A trigger deletes the full-text index of the session too.
            let sessions = tx
                .execute(
                    &format!("DELETE FROM text_sessions WHERE id IN ({SESSIONS_TO_DELETE})"),
                    params,
                )
                .db()?;
            tx.execute(
                "DELETE FROM frames WHERE app_id IN (SELECT value FROM json_each(?1))
                                       OR path IN (SELECT value FROM json_each(?2))",
                params,
            )
            .db()?;
            tx.commit().db()?;
            Ok(sessions as u64)
        })
        .await?;
    Ok(sessions)
}

/// Deletes the screenshot files and returns how many were deleted. Files that are already gone
/// are skipped.
async fn delete_files(root: &Path, screenshots: &[String]) -> Result<u64> {
    let (root, screenshots) = (root.to_path_buf(), screenshots.to_vec());
    tokio::task::spawn_blocking(move || {
        let (mut deleted, mut failed) = (0, 0);
        for rel in &screenshots {
            match std::fs::remove_file(root.join(rel)) {
                Ok(()) => deleted += 1,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    log::warn!("purge: a screenshot was not deleted: {e}");
                    failed += 1;
                }
            }
        }
        if failed > 0 {
            return Err(Error::Other(format!(
                "{failed} screenshots were not deleted"
            )));
        }
        Ok(deleted)
    })
    .await
    .map_err(|e| Error::Other(format!("screenshot delete task failed: {e}")))?
}

/// Deletes the data made from the screenshots, then the activity rows, in one transaction.
/// Returns how many activity rows were deleted.
///
/// `sync_as` is the ID of this device. When it is set, each day that has deleted rows recorded
/// on this device gets one `activity` row in the outbox, in the same transaction as the
/// deletion.
async fn delete_rows(pool: &DbPool, to_delete: &ToDelete, sync_as: Option<&str>) -> Result<u64> {
    let rows = serde_json::to_string(&to_delete.rows)?;
    let screenshots = serde_json::to_string(&to_delete.screenshots)?;
    let sync_as = sync_as.map(str::to_string);
    let deleted = pool
        .0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            if let Some(self_id) = &sync_as {
                let days: Vec<String> = column(
                    &tx,
                    "SELECT DISTINCT local_date FROM activities
                      WHERE device_id = ?1 AND id IN (SELECT value FROM json_each(?2))",
                    [self_id, &rows],
                )?;
                enqueue_days(&tx, OutboxEntity::Activity, &days).db()?;
            }
            for sql in [
                "DELETE FROM ai_image_descriptions
                  WHERE screenshot_path IN (SELECT value FROM json_each(?1))",
                "DELETE FROM screenshot_embeddings
                  WHERE screenshot_path IN (SELECT value FROM json_each(?1))",
                "DELETE FROM screenshot_dedup_map
                  WHERE member_path IN (SELECT value FROM json_each(?1))
                     OR rep_path IN (SELECT value FROM json_each(?1))",
            ] {
                tx.execute(sql, [&screenshots]).db()?;
            }
            let deleted = tx
                .execute(
                    "DELETE FROM activities WHERE id IN (SELECT value FROM json_each(?1))",
                    [&rows],
                )
                .db()?;
            tx.commit().db()?;
            Ok(deleted as u64)
        })
        .await?;
    Ok(deleted)
}

/// Deletes the icons and install paths of these processes, then soft-deletes the group.
async fn delete_group(pool: &DbPool, processes: &[String], group_id: &str) -> Result<()> {
    let processes = serde_json::to_string(processes)?;
    pool.0
        .call(move |conn| {
            for table in ["app_icons", "process_paths"] {
                conn.execute(
                    &format!(
                        "DELETE FROM {table}
                          WHERE process_name IN (SELECT value FROM json_each(?1))"
                    ),
                    [&processes],
                )
                .db()?;
            }
            Ok(())
        })
        .await?;
    app_groups::purge_with_members(pool, group_id).await
}

/// Runs a query that returns one column.
fn column<T: rusqlite::types::FromSql>(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> DbResult<Vec<T>> {
    let mut stmt = conn.prepare(sql).db()?;
    let rows = stmt
        .query_map(params, |r| r.get(0))
        .db()?
        .collect::<rusqlite::Result<Vec<T>>>()
        .db()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::categories::assign_app;
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};

    async fn seed_vscode_group(pool: &DbPool) {
        pool.0
            .call(|conn| {
                let now = "2026-05-15T10:00:00Z";
                // 用裸 INSERT 绕开 ensure_group 的 cross_os_alias 规范化逻辑，
                // 测试想要的就是 group_id="vscode" + 两个 member。
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('vscode', 'vscode', 'code', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                for member in ["Code", "Code.exe"] {
                    conn.execute(
                        "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                         VALUES(?1, 'vscode', ?2, NULL)",
                        rusqlite::params![member, now],
                    )
                    .db()?;
                }
                Ok(())
            })
            .await
            .unwrap();
    }

    fn vscode_target() -> PurgeTarget {
        PurgeTarget::AppGroup {
            group_id: "vscode".into(),
        }
    }

    async fn group_deleted(pool: &DbPool, id: &str) -> bool {
        let id = id.to_string();
        pool.0
            .call(move |conn| {
                let v: Option<String> = conn
                    .query_row(
                        "SELECT deleted_at FROM app_groups WHERE id = ?1",
                        rusqlite::params![id],
                        |r| r.get(0),
                    )
                    .db()?;
                Ok(v.is_some())
            })
            .await
            .unwrap()
    }

    /// 造一个应用的完整痕迹:主库活动 + 记忆库 OCR 会话/帧 + 一张截图文件。
    async fn seed_app_traces(
        pool: &DbPool,
        mem: &MemoryDb,
        root: &Path,
        process: &str,
        shot_rel: &str,
        text: &str,
    ) {
        std::fs::write(root.join(shot_rel), b"fake-png").unwrap();
        let (p, sr) = (process.to_string(), shot_rel.to_string());
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activities(started_at, ended_at, duration_secs, local_date,
                                            local_hour, process_name, window_title,
                                            category_id, screenshot_path)
                     VALUES('2026-05-15T10:00:00Z','2026-05-15T10:05:00Z',300,'2026-05-15',
                            10, ?1, 'title', 'other', ?2)",
                    rusqlite::params![p, sr],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
        let (p2, sr2, txt) = (process.to_string(), shot_rel.to_string(), text.to_string());
        mem.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO text_sessions(local_date, started_ts, ended_ts, app_id, title, text)
                     VALUES('2026-05-15','2026-05-15T10:00:00Z','2026-05-15T10:05:00Z',?1,'t',?2)",
                    rusqlite::params![p2, txt],
                )
                .db()?;
                let sid = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO session_lines(session_id, line_no, text, first_path, first_ts)
                     VALUES(?1, 0, ?2, ?3, '2026-05-15T10:00:00Z')",
                    rusqlite::params![sid, txt, sr2],
                )
                .db()?;
                // frames.path 是主键:共享截图的测试里两个应用指向同一张图,
                // 现实中该图只会有一条帧记录,这里用 OR IGNORE 如实模拟
                conn.execute(
                    "INSERT OR IGNORE INTO frames(path, ts, local_date, app_id, title, ocr_state)
                     VALUES(?1,'2026-05-15T10:00:00Z','2026-05-15',?2,'t',1)",
                    rusqlite::params![sr2, p2],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// 给截图插 AI 描述和向量这两种派生数据。
    async fn seed_derived(pool: &DbPool, shot: &str, index: i64) {
        let shot = shot.to_string();
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO ai_image_descriptions(local_date, segment_idx, image_index,
                                                       screenshot_path, description, model,
                                                       generated_at)
                     VALUES('2026-05-15', 0, ?2, ?1, '屏幕上的描述', 'm', '2026-05-15T12:00:00Z')",
                    rusqlite::params![shot, index],
                )
                .db()?;
                conn.execute(
                    "INSERT INTO screenshot_embeddings(screenshot_path, model_id, dim, embedding)
                     VALUES(?1, 'm', 1, x'00')",
                    [&shot],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn query_count(pool: &DbPool, sql: &'static str) -> i64 {
        pool.0
            .call(move |conn| conn.query_row(sql, [], |r| r.get(0)).db())
            .await
            .unwrap()
    }

    async fn count_activities(pool: &DbPool, process: &str) -> i64 {
        let p = process.to_string();
        pool.0
            .call(move |conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM activities WHERE process_name = ?1",
                    rusqlite::params![p],
                    |r| r.get::<_, i64>(0),
                )
                .db()
            })
            .await
            .unwrap()
    }

    /// 还剩的行的网站，按字母排。
    async fn hosts(pool: &DbPool) -> Vec<String> {
        pool.0
            .call(|conn| column(conn, "SELECT url_host FROM activities ORDER BY 1", ()))
            .await
            .unwrap()
    }

    async fn fts_hits(mem: &MemoryDb, needle: &str) -> i64 {
        let n = needle.to_string();
        mem.0
            .call(move |conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM text_sessions_fts WHERE text_sessions_fts MATCH ?1",
                    rusqlite::params![n],
                    |r| r.get::<_, i64>(0),
                )
                .db()
            })
            .await
            .unwrap()
    }

    /// 为什么测:「删除数据」承诺清空活动、截图与 OCR 文字索引。三处分属两个
    /// 数据库 + 文件系统,漏掉任何一处都是"假删除"——尤其 OCR 索引里存着屏幕上
    /// 出现过的原文,只删活动行的话搜索页照样能搜到。
    /// 同时必须证明**只删目标应用**:隔壁应用的数据一条都不能少。
    #[tokio::test]
    async fn app_group_purge_wipes_all_three_places_and_spares_others() {
        let pool = fresh_test_pool().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = std::env::temp_dir().join(format!("hs-purge-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        seed_vscode_group(&pool).await;
        seed_app_traces(&pool, &mem, &dir, "Code", "code.png", "机密文档内容").await;
        seed_app_traces(
            &pool,
            &mem,
            &dir,
            "Code.exe",
            "code-exe.png",
            "另一台机器上的文字",
        )
        .await;
        // 隔壁应用:不该被波及
        seed_app_traces(&pool, &mem, &dir, "Chrome", "chrome.png", "浏览器里的文字").await;
        seed_derived(&pool, "code.png", 0).await;
        seed_derived(&pool, "chrome.png", 1).await;

        let target = vscode_target();
        let report = purge(&pool, Some(&mem), &dir, TEST_SELF_ID, &target)
            .await
            .unwrap();

        assert_eq!(
            report,
            PurgeReport {
                activities: 2,
                screenshots: 2,
                memory_sessions: 2
            }
        );
        // 主库:组内**两个** member 的活动都清了(多进程名组是跨设备合并的常态)
        assert_eq!(count_activities(&pool, "Code").await, 0);
        assert_eq!(count_activities(&pool, "Code.exe").await, 0);
        assert_eq!(
            count_activities(&pool, "Chrome").await,
            1,
            "隔壁应用不该受影响"
        );

        // 记忆库:FTS 里搜不到被删应用的原文,隔壁的仍在
        assert_eq!(
            fts_hits(&mem, "机密文档内容").await,
            0,
            "OCR 文字索引必须清掉"
        );
        assert_eq!(
            fts_hits(&mem, "浏览器里的文字").await,
            1,
            "隔壁应用的文字应还在"
        );

        // 文件系统
        assert!(!dir.join("code.png").exists(), "截图文件应被删除");
        assert!(!dir.join("code-exe.png").exists());
        assert!(dir.join("chrome.png").exists(), "隔壁应用的截图不该被删");

        // 截图删了，从它生成的 AI 描述和向量也删；隔壁的留着
        for sql in [
            "SELECT COUNT(*) FROM ai_image_descriptions",
            "SELECT COUNT(*) FROM screenshot_embeddings",
        ] {
            assert_eq!(
                query_count(&pool, sql).await,
                1,
                "只剩 chrome.png 的那一行: {sql}"
            );
        }

        // 组本身仍按既有语义软删
        assert!(group_deleted(&pool, "vscode").await);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 为什么测:实测当前一条 activity 独占一张截图、去重映射也从不跨应用,
    /// 所以正常路径下不会误删别人的图。但万一将来去重改成全局图像哈希,
    /// 共享就可能出现——这行兜底检查必须挡住,否则会静默打穿别的应用的证据链
    /// (搜索结果点开看不到原图)。
    #[tokio::test]
    async fn app_group_purge_keeps_screenshot_still_referenced_by_others() {
        let pool = fresh_test_pool().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = std::env::temp_dir().join(format!("hs-purge-shared-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        seed_vscode_group(&pool).await;
        // 人为让两个应用引用同一张图(现实中不会发生,见上面的说明)
        seed_app_traces(&pool, &mem, &dir, "Code", "shared.png", "甲的文字").await;
        seed_app_traces(&pool, &mem, &dir, "Chrome", "shared.png", "乙的文字").await;

        let target = vscode_target();
        purge(&pool, Some(&mem), &dir, TEST_SELF_ID, &target)
            .await
            .unwrap();

        assert_eq!(count_activities(&pool, "Code").await, 0);
        assert!(
            dir.join("shared.png").exists(),
            "还被别的应用引用的截图不能删——否则对方的证据卡点开是空的"
        );
        assert_eq!(count_activities(&pool, "Chrome").await, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 截图删不掉时主库一行都不动，重跑还能找到要删的东西。主库要是先删了，这里的行就没了。
    #[tokio::test]
    async fn failed_screenshot_delete_keeps_rows_for_a_rerun() {
        let pool = fresh_test_pool().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = std::env::temp_dir().join(format!("hs-purge-stuck-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        seed_vscode_group(&pool).await;
        seed_app_traces(&pool, &mem, &dir, "Code", "stuck.png", "删不掉的截图").await;
        // 换成同名文件夹：remove_file 删不掉它，报的也不是「文件不存在」
        std::fs::remove_file(dir.join("stuck.png")).unwrap();
        std::fs::create_dir(dir.join("stuck.png")).unwrap();
        let target = vscode_target();

        let failed = purge(&pool, Some(&mem), &dir, TEST_SELF_ID, &target).await;
        assert!(failed.is_err());
        assert_eq!(count_activities(&pool, "Code").await, 1, "主库不能动");
        assert!(!group_deleted(&pool, "vscode").await, "组也不能软删");

        std::fs::remove_dir(dir.join("stuck.png")).unwrap();
        purge(&pool, Some(&mem), &dir, TEST_SELF_ID, &target)
            .await
            .unwrap();
        assert_eq!(count_activities(&pool, "Code").await, 0);
        assert!(group_deleted(&pool, "vscode").await);
        std::fs::remove_dir_all(&dir).ok();
    }

    // ───────────── Private：按隐私分类删 ─────────────

    const PEER: &str = "peer-device";

    /// 插一行活动：哪个应用、停在哪个网站、哪台设备记的、哪天。不带截图。
    async fn seed_row(pool: &DbPool, app: &str, host: Option<&str>, device: &str, day: &str) {
        let (app, host, device, day) = (
            app.to_string(),
            host.map(str::to_string),
            device.to_string(),
            day.to_string(),
        );
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activities(started_at, ended_at, duration_secs, local_date,
                                            local_hour, process_name, window_title,
                                            category_id, device_id, url_host)
                     VALUES(?3 || 'T10:00:00Z', ?3 || 'T10:05:00Z', 300, ?3,
                            10, ?1, 'title', 'other', ?4, ?2)",
                    rusqlite::params![app, host, day, device],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// 插一条记忆会话。`origin` 为 None 是本机的会话。
    async fn seed_session(mem: &MemoryDb, app: &str, day: &str, origin: Option<&str>) {
        let (app, day, origin) = (app.to_string(), day.to_string(), origin.map(str::to_string));
        mem.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO text_sessions(local_date, started_ts, ended_ts, app_id, title,
                                               text, origin_device)
                     VALUES(?1, ?1 || 'T10:00:00Z', ?1 || 'T10:05:00Z', ?2, 't', '文字', ?3)",
                    rusqlite::params![day, app, origin],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// outbox 里按天的行，按 (entity, localDate) 排好。
    async fn outbox_days(pool: &DbPool) -> Vec<(String, String)> {
        pool.0
            .call(|conn| {
                column::<String>(
                    conn,
                    "SELECT entity || ' ' || json_extract(payload, '$.localDate') FROM sync_outbox
                      WHERE entity IN ('activity', 'memory') ORDER BY 1",
                    (),
                )
            })
            .await
            .unwrap()
            .into_iter()
            .map(|row| {
                let (entity, day) = row.split_once(' ').unwrap();
                (entity.to_string(), day.to_string())
            })
            .collect()
    }

    /// 隐私的应用，和隐私网站（连同跟随它的子域名）的行都删；别的行留着。
    #[tokio::test]
    async fn private_purge_finds_private_apps_and_sites() {
        let pool = fresh_test_pool().await;
        let dir = std::env::temp_dir().join(format!("hs-purge-private-{}", std::process::id()));
        assign_app(&pool, "Tor", "private").await.unwrap();
        assign_app(&pool, "Firefox", "browse").await.unwrap();
        crate::repo::site_rules::set(&pool, "google.com", "private")
            .await
            .unwrap();
        let day = "2026-11-05";
        seed_row(&pool, "Tor", None, TEST_SELF_ID, day).await;
        seed_row(
            &pool,
            "Firefox",
            Some("gemini.google.com"),
            TEST_SELF_ID,
            day,
        )
        .await;
        seed_row(&pool, "Firefox", Some("github.com"), TEST_SELF_ID, day).await;
        seed_row(&pool, "Code", None, TEST_SELF_ID, day).await;

        let report = purge(&pool, None, &dir, TEST_SELF_ID, &PurgeTarget::Private)
            .await
            .unwrap();

        assert_eq!(report.activities, 2);
        assert_eq!(count_activities(&pool, "Tor").await, 0);
        assert_eq!(
            query_count(
                &pool,
                "SELECT COUNT(*) FROM activities WHERE url_host = 'github.com'"
            )
            .await,
            1
        );
        assert_eq!(count_activities(&pool, "Code").await, 1);
    }

    /// 一个会话里只要有一张截图是隐私网站的，整个会话都删；非隐私那张截图的行和文件留下。
    #[tokio::test]
    async fn private_purge_deletes_the_whole_session_with_a_private_screenshot() {
        let pool = fresh_test_pool().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = std::env::temp_dir().join(format!("hs-purge-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assign_app(&pool, "Firefox", "browse").await.unwrap();
        crate::repo::site_rules::set(&pool, "google.com", "private")
            .await
            .unwrap();
        let day = "2026-11-05";
        seed_row(
            &pool,
            "Firefox",
            Some("gemini.google.com"),
            TEST_SELF_ID,
            day,
        )
        .await;
        seed_row(&pool, "Firefox", Some("github.com"), TEST_SELF_ID, day).await;
        // 两行各一张截图，两张截图的文字归进同一个会话
        pool.0
            .call(|conn| {
                conn.execute_batch(
                    "UPDATE activities SET screenshot_path = 'a.png' WHERE url_host = 'gemini.google.com';
                     UPDATE activities SET screenshot_path = 'b.png' WHERE url_host = 'github.com';",
                )
                .db()
            })
            .await
            .unwrap();
        std::fs::write(dir.join("a.png"), b"fake-png").unwrap();
        std::fs::write(dir.join("b.png"), b"fake-png").unwrap();
        seed_session(&mem, "Firefox", day, None).await;
        mem.0
            .call(|conn| {
                conn.execute_batch(
                    "INSERT INTO frames(path, ts, local_date, app_id, title, ocr_state, session_id)
                     VALUES ('a.png', '2026-11-05T10:00:00Z', '2026-11-05', 'Firefox', 't', 1, 1),
                            ('b.png', '2026-11-05T10:01:00Z', '2026-11-05', 'Firefox', 't', 1, 1);",
                )
                .db()
            })
            .await
            .unwrap();

        let report = purge(&pool, Some(&mem), &dir, TEST_SELF_ID, &PurgeTarget::Private)
            .await
            .unwrap();

        assert_eq!(report.memory_sessions, 1);
        assert_eq!(report.screenshots, 1);
        assert!(!dir.join("a.png").exists());
        assert!(dir.join("b.png").exists());
        assert_eq!(
            query_count(
                &pool,
                "SELECT COUNT(*) FROM activities WHERE url_host = 'github.com'"
            )
            .await,
            1
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 别的设备同步过来的行和会话也删，但只有本机的日子进 outbox：别人的文件只能由别人重传。
    #[tokio::test]
    async fn private_purge_enqueues_only_this_devices_days() {
        let pool = fresh_test_pool().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = std::env::temp_dir().join(format!("hs-purge-days-{}", std::process::id()));
        assign_app(&pool, "Tor", "private").await.unwrap();
        seed_row(&pool, "Tor", None, TEST_SELF_ID, "2026-11-05").await;
        seed_row(&pool, "Tor", None, PEER, "2026-11-06").await;
        seed_session(&mem, "Tor", "2026-11-07", None).await;
        seed_session(&mem, "Tor", "2026-11-08", Some(PEER)).await;

        let report = purge(&pool, Some(&mem), &dir, TEST_SELF_ID, &PurgeTarget::Private)
            .await
            .unwrap();

        assert_eq!(report.activities, 2);
        assert_eq!(report.memory_sessions, 2);
        assert_eq!(
            outbox_days(&pool).await,
            vec![
                ("activity".to_string(), "2026-11-05".to_string()),
                ("memory".to_string(), "2026-11-07".to_string()),
            ]
        );
    }

    /// 清理不记状态：第二次什么都不删，也不再往 outbox 里加行。
    #[tokio::test]
    async fn private_purge_twice_deletes_nothing_more() {
        let pool = fresh_test_pool().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = std::env::temp_dir().join(format!("hs-purge-twice-{}", std::process::id()));
        assign_app(&pool, "Tor", "private").await.unwrap();
        seed_row(&pool, "Tor", None, TEST_SELF_ID, "2026-11-05").await;
        seed_session(&mem, "Tor", "2026-11-05", None).await;

        let first = purge(&pool, Some(&mem), &dir, TEST_SELF_ID, &PurgeTarget::Private)
            .await
            .unwrap();
        let outbox_after_first = outbox_days(&pool).await;
        let second = purge(&pool, Some(&mem), &dir, TEST_SELF_ID, &PurgeTarget::Private)
            .await
            .unwrap();

        assert_ne!(first, PurgeReport::default());
        assert_eq!(second, PurgeReport::default());
        assert_eq!(outbox_days(&pool).await, outbox_after_first);
    }

    /// 删一个网站：连同跟随它的子域名，本机和别的设备记的行都删，别的网站不动。
    /// 只有本机的日子进 outbox，下一轮推送重传这几天的文件，云端的那份也就没了。
    #[tokio::test]
    async fn site_purge_enqueues_only_this_devices_days() {
        let pool = fresh_test_pool().await;
        let dir = std::env::temp_dir().join(format!("hs-purge-site-{}", std::process::id()));
        for (host, device, day) in [
            ("example.com", TEST_SELF_ID, "2026-11-05"),
            ("www.example.com", TEST_SELF_ID, "2026-11-06"),
            ("example.com", PEER, "2026-11-07"),
            ("other.com", TEST_SELF_ID, "2026-11-08"),
        ] {
            seed_row(&pool, "Firefox", Some(host), device, day).await;
        }
        let site = PurgeTarget::Site {
            host: "example.com".into(),
        };

        let report = purge(&pool, None, &dir, TEST_SELF_ID, &site).await.unwrap();

        assert_eq!(report.activities, 3);
        assert_eq!(hosts(&pool).await, ["other.com"]);
        assert_eq!(
            outbox_days(&pool).await,
            vec![
                ("activity".to_string(), "2026-11-05".to_string()),
                ("activity".to_string(), "2026-11-06".to_string()),
            ]
        );
    }

    // ───────────── count：会删掉多少 ─────────────

    /// 应用分组：组里每个进程的行都算；截图只算磁盘上还在的文件，大小按文件算。数完什么都不删。
    #[tokio::test]
    async fn count_app_group_data() {
        let pool = fresh_test_pool().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = std::env::temp_dir().join(format!("hs-count-group-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        seed_vscode_group(&pool).await;
        seed_app_traces(&pool, &mem, &dir, "Code", "code.png", "甲").await;
        seed_app_traces(&pool, &mem, &dir, "Code.exe", "gone.png", "乙").await;
        seed_app_traces(&pool, &mem, &dir, "Chrome", "chrome.png", "丙").await;
        std::fs::remove_file(dir.join("gone.png")).unwrap();

        let counted = count(&pool, &dir, &vscode_target()).await.unwrap();

        assert_eq!(
            counted,
            DataCount {
                activities: 2,
                screenshots: 1,
                screenshot_bytes: b"fake-png".len() as u64,
            }
        );
        assert_eq!(count_activities(&pool, "Code").await, 1, "数完不能删");
        assert!(dir.join("code.png").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 网站：域名相同的行，加上跟随它的子域名的行。自己有规则的子域名，连同它下面的，都不算；
    /// 只是结尾字母相同的别的网站也不算。网站有没有自己的规则，结果都一样。
    #[tokio::test]
    async fn count_site_data_skips_subdomains_with_own_rules() {
        let pool = fresh_test_pool().await;
        let dir = std::env::temp_dir().join(format!("hs-count-site-{}", std::process::id()));
        crate::repo::site_rules::set(&pool, "google.com", "browse")
            .await
            .unwrap();
        crate::repo::site_rules::set(&pool, "mail.google.com", "talk")
            .await
            .unwrap();
        for host in [
            "google.com",
            "gemini.google.com",
            "mail.google.com",
            "inbox.mail.google.com",
            "notgoogle.com",
            "github.com",
            "gist.github.com",
        ] {
            seed_row(&pool, "Firefox", Some(host), TEST_SELF_ID, "2026-11-05").await;
        }
        let site = |host: &str| PurgeTarget::Site { host: host.into() };

        let google = count(&pool, &dir, &site("google.com")).await.unwrap();
        let github = count(&pool, &dir, &site("github.com")).await.unwrap();

        assert_eq!(google.activities, 2, "google.com 有规则");
        assert_eq!(github.activities, 2, "github.com 没有规则");
    }

    // ───────────── truncate_at：从截断点起的记录不算 ─────────────

    /// 截断点：最后一次动键鼠的时刻。
    const CUT: &str = "2026-11-20T21:00:00+09:00";

    fn at(rfc3339: &str) -> DateTime<Local> {
        DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .with_timezone(&Local)
    }

    /// 插一行活动，返回 id。日期取开始时刻的前 10 位。
    async fn seed_span(
        pool: &DbPool,
        device: &str,
        started: &str,
        ended: &str,
        shot: Option<&str>,
    ) -> i64 {
        let (device, started, ended, shot) = (
            device.to_string(),
            started.to_string(),
            ended.to_string(),
            shot.map(str::to_string),
        );
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activities(started_at, ended_at, duration_secs, local_date,
                                            local_hour, process_name, window_title,
                                            category_id, device_id, screenshot_path)
                     VALUES(?1, ?2, 30, substr(?1, 1, 10), 21, 'Player', 'title', 'other',
                            ?3, ?4)",
                    rusqlite::params![started, ended, device, shot],
                )
                .db()?;
                Ok(conn.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    /// 还剩的活动行的 id，从小到大。
    async fn activity_ids(pool: &DbPool) -> Vec<i64> {
        pool.0
            .call(|conn| column(conn, "SELECT id FROM activities ORDER BY id", ()))
            .await
            .unwrap()
    }

    /// 跨过截断点的那行：结束时刻改成截断点，时长跟着变。
    #[tokio::test]
    async fn truncate_at_ends_the_spanning_row_at_the_cut() {
        let pool = fresh_test_pool().await;
        let id = seed_span(
            &pool,
            TEST_SELF_ID,
            "2026-11-20T20:59:40+09:00",
            "2026-11-20T21:00:10+09:00",
            None,
        )
        .await;

        truncate_at(&pool, None, Path::new(""), TEST_SELF_ID, at(CUT))
            .await
            .unwrap();

        let (ended, secs): (String, i64) = pool
            .0
            .call(move |conn| {
                conn.query_row(
                    "SELECT ended_at, duration_secs FROM activities WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .db()
            })
            .await
            .unwrap();
        assert_eq!(DateTime::parse_from_rfc3339(&ended).unwrap(), at(CUT));
        assert_eq!(secs, 20);
    }

    /// 截断点及以后开始的行删掉，之前的不动。
    #[tokio::test]
    async fn truncate_at_deletes_rows_that_start_after_the_cut() {
        let pool = fresh_test_pool().await;
        let before = seed_span(
            &pool,
            TEST_SELF_ID,
            "2026-11-20T20:50:00+09:00",
            "2026-11-20T20:50:30+09:00",
            None,
        )
        .await;
        seed_span(&pool, TEST_SELF_ID, CUT, "2026-11-20T21:00:30+09:00", None).await;
        seed_span(
            &pool,
            TEST_SELF_ID,
            "2026-11-20T21:10:00+09:00",
            "2026-11-20T21:10:30+09:00",
            None,
        )
        .await;

        let report = truncate_at(&pool, None, Path::new(""), TEST_SELF_ID, at(CUT))
            .await
            .unwrap();

        assert_eq!(report.activities, 2);
        assert_eq!(activity_ids(&pool).await, vec![before]);
    }

    /// 别的设备记的行不动，哪怕在截断点之后。
    #[tokio::test]
    async fn truncate_at_keeps_other_devices_rows() {
        let pool = fresh_test_pool().await;
        let peer = seed_span(
            &pool,
            PEER,
            "2026-11-20T21:10:00+09:00",
            "2026-11-20T21:10:30+09:00",
            None,
        )
        .await;

        let report = truncate_at(&pool, None, Path::new(""), TEST_SELF_ID, at(CUT))
            .await
            .unwrap();

        assert_eq!(report.activities, 0);
        assert_eq!(activity_ids(&pool).await, vec![peer]);
    }

    /// 删了行或改了行的日子都写 outbox，过了午夜的那天也算；别的设备的、没动到的日子不写。
    #[tokio::test]
    async fn truncate_at_marks_every_touched_day_for_push() {
        let pool = fresh_test_pool().await;
        let cut = "2026-11-20T23:59:50+09:00";
        let t = |s: &str| format!("2026-{s}+09:00");
        for (device, started, ended) in [
            (TEST_SELF_ID, "11-19T10:00:00", "11-19T10:00:30"),
            (TEST_SELF_ID, "11-20T23:59:40", "11-21T00:00:10"),
            (TEST_SELF_ID, "11-21T00:00:10", "11-21T00:00:40"),
            (PEER, "11-22T10:00:00", "11-22T10:00:30"),
        ] {
            seed_span(&pool, device, &t(started), &t(ended), None).await;
        }

        truncate_at(&pool, None, Path::new(""), TEST_SELF_ID, at(cut))
            .await
            .unwrap();

        assert_eq!(
            outbox_days(&pool).await,
            vec![
                ("activity".to_string(), "2026-11-20".to_string()),
                ("activity".to_string(), "2026-11-21".to_string()),
            ]
        );
    }

    /// 删掉的行独占的截图：文件和它的 AI 描述都删；留下的行的截图不动。
    #[tokio::test]
    async fn truncate_at_deletes_the_screenshots_of_dropped_rows() {
        let pool = fresh_test_pool().await;
        let dir = std::env::temp_dir().join(format!("hs-truncate-shots-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for shot in ["kept.jpg", "dropped.jpg"] {
            std::fs::write(dir.join(shot), b"fake-jpg").unwrap();
        }
        seed_span(
            &pool,
            TEST_SELF_ID,
            "2026-11-20T20:50:00+09:00",
            "2026-11-20T20:50:30+09:00",
            Some("kept.jpg"),
        )
        .await;
        seed_span(
            &pool,
            TEST_SELF_ID,
            "2026-11-20T21:05:00+09:00",
            "2026-11-20T21:05:30+09:00",
            Some("dropped.jpg"),
        )
        .await;
        seed_derived(&pool, "kept.jpg", 0).await;
        seed_derived(&pool, "dropped.jpg", 1).await;

        let report = truncate_at(&pool, None, &dir, TEST_SELF_ID, at(CUT))
            .await
            .unwrap();

        assert_eq!(report.screenshots, 1);
        assert!(dir.join("kept.jpg").exists());
        assert!(!dir.join("dropped.jpg").exists());
        assert_eq!(
            query_count(&pool, "SELECT COUNT(*) FROM ai_image_descriptions").await,
            1
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 插一段屏幕文字，每行带第一次出现的时刻，`text` 照识别时的样子拼好。返回段的 id。
    async fn seed_text(
        mem: &MemoryDb,
        origin: Option<&str>,
        started: &str,
        ended: &str,
        lines: &[(&str, &str)],
    ) -> i64 {
        let (origin, started, ended) = (
            origin.map(str::to_string),
            started.to_string(),
            ended.to_string(),
        );
        let lines: Vec<(String, String)> = lines
            .iter()
            .map(|(line, ts)| (line.to_string(), ts.to_string()))
            .collect();
        mem.0
            .call(move |conn| {
                let text: Vec<&str> = lines.iter().map(|(line, _)| line.as_str()).collect();
                conn.execute(
                    "INSERT INTO text_sessions(local_date, started_ts, ended_ts, app_id, title,
                                               text, origin_device)
                     VALUES(substr(?1, 1, 10), ?1, ?2, 'Player', 't', ?3, ?4)",
                    rusqlite::params![started, ended, text.join("\n"), origin],
                )
                .db()?;
                let sid = conn.last_insert_rowid();
                for (no, (line, ts)) in lines.iter().enumerate() {
                    conn.execute(
                        "INSERT INTO session_lines(session_id, line_no, text, first_path, first_ts)
                         VALUES(?1, ?2, ?3, 'x.jpg', ?4)",
                        rusqlite::params![sid, no as i64, line, ts],
                    )
                    .db()?;
                }
                Ok(sid)
            })
            .await
            .unwrap()
    }

    /// 登记一张还没识别的截图。
    async fn seed_frame(mem: &MemoryDb, path: &str, ts: &str) {
        let (path, ts) = (path.to_string(), ts.to_string());
        mem.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO frames(path, ts, local_date, app_id, title)
                     VALUES(?1, ?2, substr(?2, 1, 10), 'Player', 't')",
                    rusqlite::params![path, ts],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// 截断点及以后登记的截图删掉，之前的留着。
    #[tokio::test]
    async fn truncate_at_deletes_frames_after_the_cut() {
        let pool = fresh_test_pool().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        seed_frame(&mem, "before.jpg", "2026-11-20T20:55:00+09:00").await;
        seed_frame(&mem, "after.jpg", "2026-11-20T21:05:00+09:00").await;

        truncate_at(&pool, Some(&mem), Path::new(""), TEST_SELF_ID, at(CUT))
            .await
            .unwrap();

        let paths: Vec<String> = mem
            .0
            .call(|conn| column(conn, "SELECT path FROM frames ORDER BY path", ()))
            .await
            .unwrap();
        assert_eq!(paths, vec!["before.jpg"]);
    }

    /// 截断点及以后开始的段整段删，搜不到了。
    #[tokio::test]
    async fn truncate_at_deletes_text_sessions_that_start_after_the_cut() {
        let pool = fresh_test_pool().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let after = "2026-11-20T21:05:00+09:00";
        seed_text(
            &mem,
            None,
            after,
            "2026-11-20T21:20:00+09:00",
            &[("睡着以后视频里的字幕", after)],
        )
        .await;

        let report = truncate_at(&pool, Some(&mem), Path::new(""), TEST_SELF_ID, at(CUT))
            .await
            .unwrap();

        assert_eq!(report.memory_sessions, 1);
        assert_eq!(fts_hits(&mem, "睡着以后视频里的字幕").await, 0);
        let lines: i64 = mem
            .0
            .call(|conn| {
                conn.query_row("SELECT COUNT(*) FROM session_lines", [], |r| r.get(0))
                    .db()
            })
            .await
            .unwrap();
        assert_eq!(lines, 0);
    }

    /// 跨过截断点的段：只删截断点以后才出现的行，文字重新拼，结束时刻改成截断点。
    #[tokio::test]
    async fn truncate_at_drops_lines_first_seen_after_the_cut() {
        let pool = fresh_test_pool().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let sid = seed_text(
            &mem,
            None,
            "2026-11-20T20:50:00+09:00",
            "2026-11-20T21:20:00+09:00",
            &[
                ("睡前看到的第一行字", "2026-11-20T20:50:00+09:00"),
                ("睡前看到的第二行字", "2026-11-20T20:55:00+09:00"),
                ("睡着以后才出现的字幕", "2026-11-20T21:05:00+09:00"),
            ],
        )
        .await;

        truncate_at(&pool, Some(&mem), Path::new(""), TEST_SELF_ID, at(CUT))
            .await
            .unwrap();

        let (text, ended): (String, String) = mem
            .0
            .call(move |conn| {
                conn.query_row(
                    "SELECT text, ended_ts FROM text_sessions WHERE id = ?1",
                    [sid],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .db()
            })
            .await
            .unwrap();
        assert_eq!(text, "睡前看到的第一行字\n睡前看到的第二行字");
        assert_eq!(DateTime::parse_from_rfc3339(&ended).unwrap(), at(CUT));
        assert_eq!(fts_hits(&mem, "睡着以后才出现的字幕").await, 0);
        assert_eq!(fts_hits(&mem, "睡前看到的第二行字").await, 1);
    }

    /// 别的设备同步过来的文字不动，哪怕在截断点之后。
    #[tokio::test]
    async fn truncate_at_keeps_text_from_other_devices() {
        let pool = fresh_test_pool().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let after = "2026-11-20T21:05:00+09:00";
        seed_text(
            &mem,
            Some(PEER),
            after,
            "2026-11-20T21:20:00+09:00",
            &[("另一台机器上的字幕", after)],
        )
        .await;

        let report = truncate_at(&pool, Some(&mem), Path::new(""), TEST_SELF_ID, at(CUT))
            .await
            .unwrap();

        assert_eq!(report.memory_sessions, 0);
        assert_eq!(fts_hits(&mem, "另一台机器上的字幕").await, 1);
    }

    /// 动到的本机文字段的日子都写 outbox，过了午夜的那天也算；别的设备的、没动到的不写。
    #[tokio::test]
    async fn truncate_at_marks_every_touched_memory_day_for_push() {
        let pool = fresh_test_pool().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let t = |s: &str| format!("2026-{s}:00+09:00");
        for (origin, started, ended, line) in [
            (None, "11-19T10:00", "11-19T10:05", "没动到的一天"),
            (None, "11-20T20:50", "11-20T21:20", "睡前的字"),
            (None, "11-21T00:10", "11-21T00:20", "午夜后的字"),
            (Some(PEER), "11-22T10:00", "11-22T10:05", "别人的字"),
        ] {
            let started = t(started);
            seed_text(&mem, origin, &started, &t(ended), &[(line, &started)]).await;
        }

        truncate_at(&pool, Some(&mem), Path::new(""), TEST_SELF_ID, at(CUT))
            .await
            .unwrap();

        assert_eq!(
            outbox_days(&pool).await,
            vec![
                ("memory".to_string(), "2026-11-20".to_string()),
                ("memory".to_string(), "2026-11-21".to_string()),
            ]
        );
    }
}
