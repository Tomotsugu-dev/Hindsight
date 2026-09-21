//! Pull: merges the files other devices wrote to the cloud into the local
//! database.
//!
//! Every `merge_*` must be idempotent: the cursor stops before the first file
//! that failed, so the files merged after it are merged again next round.

use rusqlite::OptionalExtension;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use super::io;
use super::Inner;
use crate::capture::ignore::{is_excluded, IgnoreRule};
use crate::error::{Error, Result};
use crate::storage::{DbPool, SqliteResultExt};
use crate::sync::payload::{
    ActivityPayload, AppGroupMemberPayload, AppGroupPayload, AppIconPayload, CategoryPayload,
    DeviceMetaPayload, TombstonePayload,
};

/// Turns the contents of one sync file into rows waiting to be merged.
///
/// The whole body is not a JSON array → return an error, no rows at all.
/// A single element does not fit `T` → log it, drop it, carry on.
///
/// Dropping is permanent. The split is there so one bad row cannot stall the
/// whole file.
fn parse_rows<T: serde::de::DeserializeOwned>(kind: &'static str, body: &[u8]) -> Result<Vec<T>> {
    let arr: Vec<Value> =
        serde_json::from_slice(body).map_err(|e| Error::SyncParse { kind, source: e })?;
    let mut out = Vec::with_capacity(arr.len());
    for (idx, v) in arr.into_iter().enumerate() {
        match serde_json::from_value::<T>(v) {
            Ok(row) => out.push(row),
            Err(e) => log::warn!("{kind}: row {idx} dropped, does not parse: {e}"),
        }
    }
    Ok(out)
}

/// Timestamps are fixed-format RFC3339 forced to UTC, so comparing the strings
/// is comparing the times; there is nothing to parse.
///
/// Strictly `>`: on equal timestamps the local row wins. That is what keeps two
/// devices pulling from each other from overwriting one another in circles.
fn is_remote_newer<P: rusqlite::Params>(
    conn: &rusqlite::Connection,
    select_updated_at_sql: &str,
    key: P,
    new: &str,
) -> rusqlite::Result<bool> {
    let cur: Option<String> = conn
        .query_row(select_updated_at_sql, key, |r| r.get(0))
        .optional()?;
    Ok(match cur {
        None => true,
        Some(c) => new > c.as_str(),
    })
}

/// Primary key of a `sync_cursor` row, so this string lives in the user's
/// database. Change it and the cursor is lost: the next sync re-downloads every
/// file in the cloud.
pub(super) const CURSOR_CORE: &str = "drive_files";

/// One cursor per optional dataset, so turning a switch on fills in that
/// dataset's history without re-merging everything else (ADR-0006). The
/// `pull.` prefix keeps them apart from the `push.*` fingerprints.
const CURSOR_AI_SUMMARIES: &str = "pull.ai_summaries";
const CURSOR_CHAT: &str = "pull.chat";
const CURSOR_MEMORY: &str = "pull.memory";

enum ParsedFile {
    ActivityDay {
        device_id: String,
        local_date: String,
    },
    Categories {
        device_id: String,
    },
    DeviceMeta {
        device_id: String,
    },
    AppIcons {
        device_id: String,
    },
    AppGroups {
        device_id: String,
    },
    AppGroupMembers {
        device_id: String,
    },
    /// A device asking every peer to drop everything it wrote before a given
    /// moment: `DELETE WHERE device_id = <owner> AND updated_at < clearedAt`.
    /// It exists because the engine only inserts and updates, so a row missing
    /// from a file carries no meaning — this is the only way to say "delete".
    Tombstone {
        device_id: String,
    },
    /// Opt-in upload: AI generated summaries (merged in `datasets.rs`).
    AiSummaries {
        device_id: String,
    },
    /// Opt-in upload: chat history.
    Chat {
        device_id: String,
    },
    /// Opt-in upload: screen-memory full text, one file per day.
    MemoryDay {
        device_id: String,
    },
}

impl ParsedFile {
    /// The device that wrote the file.
    fn device_id(&self) -> &str {
        match self {
            ParsedFile::ActivityDay { device_id, .. }
            | ParsedFile::Categories { device_id }
            | ParsedFile::DeviceMeta { device_id }
            | ParsedFile::AppIcons { device_id }
            | ParsedFile::AppGroups { device_id }
            | ParsedFile::AppGroupMembers { device_id }
            | ParsedFile::Tombstone { device_id }
            | ParsedFile::AiSummaries { device_id }
            | ParsedFile::Chat { device_id }
            | ParsedFile::MemoryDay { device_id } => device_id,
        }
    }

    /// Which stream owns the file: the cursor that advances past it, and the
    /// only stream allowed to merge it.
    fn cursor_key(&self) -> &'static str {
        match self {
            ParsedFile::AiSummaries { .. } => CURSOR_AI_SUMMARIES,
            ParsedFile::Chat { .. } => CURSOR_CHAT,
            ParsedFile::MemoryDay { .. } => CURSOR_MEMORY,
            _ => CURSOR_CORE,
        }
    }
}

fn parse_filename(name: &str) -> Option<ParsedFile> {
    // Two shapes: device.<UUID>.<KIND>.json, and device.<UUID>.activities.<DAY>.ndjson.
    let parts: Vec<&str> = name.split('.').collect();
    if parts.first().copied() != Some("device") {
        return None;
    }
    match parts.as_slice() {
        ["device", uuid, "activities", day, "ndjson"] => Some(ParsedFile::ActivityDay {
            device_id: uuid.to_string(),
            local_date: day.to_string(),
        }),
        ["device", uuid, "categories", "json"] => Some(ParsedFile::Categories {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "meta", "json"] => Some(ParsedFile::DeviceMeta {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "icons", "json"] => Some(ParsedFile::AppIcons {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "app_groups", "json"] => Some(ParsedFile::AppGroups {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "app_group_members", "json"] => Some(ParsedFile::AppGroupMembers {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "tombstone", "json"] => Some(ParsedFile::Tombstone {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "ai_summaries", "json"] => Some(ParsedFile::AiSummaries {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "chat", "json"] => Some(ParsedFile::Chat {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "memory", _day, "ndjson"] => Some(ParsedFile::MemoryDay {
            device_id: uuid.to_string(),
        }),
        _ => None,
    }
}

pub(super) async fn flush_pull(inner: &Arc<Inner>) -> Result<()> {
    let _gate = inner.flush_gate.lock().await;
    // Not signed in is not a failure: there is nothing to pull.
    if !inner.cloud.ensure_credential().await? {
        return Ok(());
    }

    let self_id = inner.self_id.as_str();
    if self_id.is_empty() {
        log::debug!("sync pull skipped: self_id is empty (device not initialized)");
        return Ok(());
    }
    // Settings: pull reads the optional-dataset switches and the ignore rules.
    let opt_cfg = crate::repo::settings::load(&inner.pool).await.ok();
    let ignore_rules: Vec<IgnoreRule> = opt_cfg
        .as_ref()
        .map(|c| c.ignore_rules.clone())
        .unwrap_or_default();
    let (sync_ai, sync_chat, sync_scrn_mem) = opt_cfg
        .map(|c| {
            (
                c.sync_ai_summaries,
                c.sync_chat_history,
                c.sync_screen_memory,
            )
        })
        .unwrap_or((false, false, false));

    // The streams that run this round. A dataset runs only with its switch on,
    // and chat and screen memory also need the memory database. A stream that
    // does not run leaves its cursor where it is, so turning its switch on later
    // resumes from there.
    let has_mem = inner.mem.is_some();
    let mut streams: Vec<(&'static str, String)> = Vec::new();
    for (key, enabled) in [
        (CURSOR_CORE, true),
        (CURSOR_AI_SUMMARIES, sync_ai),
        (CURSOR_CHAT, sync_chat && has_mem),
        (CURSOR_MEMORY, sync_scrn_mem && has_mem),
    ] {
        if enabled {
            streams.push((key, io::read_cursor(&inner.pool, key).await?));
        }
    }

    // One listing per round, from the earliest cursor among those streams; each
    // stream then takes the files after its own.
    let since = streams
        .iter()
        .map(|(_, cursor)| cursor.as_str())
        .min()
        .expect("the core stream always runs")
        .to_string();
    let files = inner.cloud.list(&since).await?;
    if files.is_empty() {
        return Ok(());
    }

    let mut applied = 0u64;
    for (key, cursor) in &streams {
        applied += pull_stream(inner, key, cursor, &files, self_id, &ignore_rules).await?;
    }
    if applied > 0 {
        log::info!("sync pull done, merged {applied} remote files");
    }
    Ok(())
}

/// Merges the files of one stream: those it owns, newer than its cursor, and
/// written by another device. Returns how many it merged.
async fn pull_stream(
    inner: &Arc<Inner>,
    cursor_key: &str,
    cursor: &str,
    files: &[crate::sync::cloud::FileMeta],
    self_id: &str,
    ignore_rules: &[IgnoreRule],
) -> Result<u64> {
    let mut applied = 0u64;
    // One flag per file: merged, or not this stream's to merge.
    let mut handled = vec![false; files.len()];

    // Group files go first: `app_group_members.group_id` is a foreign key, so a
    // member merged before its group is rejected, and the cursor then moves past
    // the file for good. Indices are sorted rather than `files` itself because
    // `handled[i]` must stay in the list order the cursor is computed from.
    let order: Vec<usize> = {
        let rank = |name: &str| -> u8 {
            match parse_filename(name) {
                Some(ParsedFile::AppGroups { .. }) => 0,
                _ => 1,
            }
        };
        let mut idx: Vec<usize> = (0..files.len()).collect();
        idx.sort_by_key(|&i| rank(&files[i].name));
        idx
    };
    for &i in &order {
        let f = &files[i];
        let Some(parsed) = parse_filename(&f.name) else {
            // A file name this version does not know is not ours to merge; let the cursor pass it.
            handled[i] = true;
            continue;
        };
        // Another stream's file, or one this stream merged in an earlier round:
        // nothing to do, but its cursor may pass.
        if parsed.cursor_key() != cursor_key || f.modified_time.as_str() <= cursor {
            handled[i] = true;
            continue;
        }
        // This device's own files are never merged: its rows are the originals. Its
        // own tombstone in particular must not be: removing this device while keeping
        // local data uploads one, and applying it would delete that data here.
        if parsed.device_id() == self_id {
            handled[i] = true;
            continue;
        }

        let body = match inner.cloud.download(&f.id).await {
            Ok(b) => b,
            Err(e) => {
                log::warn!("download of {} failed: {e}", f.name);
                continue;
            }
        };

        let res = match parsed {
            ParsedFile::DeviceMeta { device_id } => {
                merge_device_meta(&inner.pool, &device_id, &body).await
            }
            ParsedFile::ActivityDay {
                device_id,
                local_date,
            } => merge_activities(&inner.pool, &device_id, &local_date, &body, ignore_rules).await,
            ParsedFile::Categories { .. } => merge_categories(&inner.pool, &body).await,
            ParsedFile::AppIcons { .. } => merge_app_icons(&inner.pool, &body).await,
            ParsedFile::AppGroups { .. } => merge_app_groups(&inner.pool, &body).await,
            ParsedFile::AppGroupMembers { .. } => merge_app_group_members(&inner.pool, &body).await,
            ParsedFile::AiSummaries { .. } => {
                super::datasets::merge_ai_summaries(&inner.pool, &body).await
            }
            ParsedFile::Chat { .. } => {
                super::datasets::merge_chat(inner.mem.as_ref().expect("gated"), &body).await
            }
            ParsedFile::MemoryDay { device_id } => {
                super::datasets::merge_memory_sessions(
                    inner.mem.as_ref().expect("gated"),
                    &device_id,
                    &body,
                )
                .await
            }
            ParsedFile::Tombstone { device_id } => {
                merge_tombstone(&inner.pool, &device_id, &body).await
            }
        };
        if let Err(e) = res {
            log::warn!("merge of {} failed: {e}", f.name);
            continue;
        }
        handled[i] = true;
        applied += 1;
    }

    // The next list asks for files modified after the cursor, so it may only move
    // past handled files, and only to a time strictly before the first unhandled
    // one: a failed file sharing that time would never be listed again.
    let first_unhandled_time = files
        .iter()
        .zip(handled.iter())
        .find(|(_, ok)| !**ok)
        .map(|(f, _)| f.modified_time.clone());
    let cursor_advance = files
        .iter()
        .zip(handled.iter())
        .take_while(|(_, ok)| **ok)
        .map(|(f, _)| f.modified_time.clone())
        .filter(|t| match &first_unhandled_time {
            Some(fu) => t.as_str() < fu.as_str(),
            None => true,
        })
        .last();
    if let Some(t) = cursor_advance {
        let t = rewind_cursor(&t, inner.cloud.time_precision())?;
        io::write_cursor(&inner.pool, cursor_key, &t).await?;
    }
    Ok(applied)
}

/// Rewinds the cursor for backends whose timestamps have only coarse precision.
/// Without the rewind, a file written in the same second as the last handled
/// file could be skipped by the next listing.
///
/// The result keeps the backend's RFC3339 format so string comparison remains
/// valid. An invalid cursor means the backend returned an invalid timestamp.
fn rewind_cursor(cursor: &str, back: Duration) -> Result<String> {
    if back.is_zero() {
        return Ok(cursor.to_string());
    }
    let parsed = chrono::DateTime::parse_from_rfc3339(cursor)
        .map_err(|_| Error::SyncTimeFormat(cursor.to_string()))?;
    let back = chrono::Duration::from_std(back).expect("a backend's time precision is tiny");
    Ok((parsed - back)
        .with_timezone(&chrono::Utc)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

async fn merge_activities(
    pool: &DbPool,
    device_id: &str,
    local_date: &str,
    body: &[u8],
    ignore_rules: &[IgnoreRule],
) -> Result<()> {
    let s = std::str::from_utf8(body)?;
    // Rows present in this file; after the loop, the mirror rows not among them
    // are deleted.
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Skip that delete when a line failed to parse or `upsert_remote_activity`
    // failed on it: the local rows no longer match the file.
    let mut parse_clean = true;

    for (lineno, line) in s.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let row: ActivityPayload = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("activities line {lineno}: does not parse: {e}");
                parse_clean = false;
                continue;
            }
        };
        let remote_id = row.id.to_string();
        seen_ids.insert(remote_id.clone());
        let excluded = is_excluded(
            &row.process_name,
            row.window_title.as_deref().unwrap_or(""),
            ignore_rules,
        );
        if let Err(e) = upsert_remote_activity(pool, device_id, &row, excluded).await {
            log::warn!("activities line {lineno}: upsert failed: {e}");
            parse_clean = false;
        }
    }

    // Never delete rows the device still has; delete only the activity rows it
    // no longer has. Delete nothing when the file did not fully land.
    if !parse_clean {
        return Ok(());
    }
    let device_id_db = device_id.to_string();
    let local_date_db = local_date.to_string();
    let ids_vec: Vec<String> = seen_ids.into_iter().collect();
    let deleted = pool
        .0
        .call(move |conn| {
            // SQL cannot bind a list, so one placeholder per id.
            let placeholders: String = if ids_vec.is_empty() {
                String::new()
            } else {
                std::iter::repeat_n("?", ids_vec.len())
                    .collect::<Vec<_>>()
                    .join(",")
            };
            let sql = if placeholders.is_empty() {
                "DELETE FROM activities
                 WHERE device_id = ?1 AND local_date = ?2"
                    .to_string()
            } else {
                format!(
                    "DELETE FROM activities
                     WHERE device_id = ?1 AND local_date = ?2
                       AND remote_id NOT IN ({placeholders})"
                )
            };
            let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(2 + ids_vec.len());
            params.push(&device_id_db);
            params.push(&local_date_db);
            for id in &ids_vec {
                params.push(id);
            }
            let n = conn.execute(&sql, params.as_slice()).db()?;
            Ok(n)
        })
        .await?;
    if deleted > 0 {
        log::info!(
            "activities of device {device_id} on {local_date}: deleted {deleted} rows no longer in its file"
        );
    }
    Ok(())
}

/// Hands every record in the file to `apply`, which writes it to the local
/// table, one transaction per record: a record that fails halfway leaves
/// nothing behind. Otherwise its `updated_at` would already match the file, the
/// next pull of the same file would skip the record, and the unwritten part
/// would never be written.
///
/// A record overwrites the local row only when its `updated_at` is later than
/// the local row's; otherwise the local row stays. That comparison is `apply`'s
/// job; this function compares no timestamps.
///
/// A record for which `pk_for_log` returns `None` is skipped and never written.
///
/// `apply` cannot capture anything (it must be `Copy`): it is sent to the
/// database thread once per record.
async fn merge_lww_simple<T, F>(
    pool: &DbPool,
    entity: &'static str,
    body: &[u8],
    pk_for_log: impl Fn(&T) -> Option<String>,
    apply: F,
) -> Result<()>
where
    T: serde::de::DeserializeOwned + Send + 'static,
    F: Fn(&rusqlite::Connection, T) -> rusqlite::Result<()> + Send + Sync + Copy + 'static,
{
    let rows: Vec<T> = parse_rows(entity, body)?;
    for row in rows {
        let Some(label) = pk_for_log(&row) else {
            continue;
        };
        let res = pool
            .0
            .call(move |conn| {
                let tx = conn.transaction().db()?;
                apply(&tx, row).db()?;
                tx.commit().db()
            })
            .await;
        if let Err(e) = res {
            log::warn!("{entity} {label}: merge failed: {e}");
        }
    }
    Ok(())
}

async fn merge_categories(pool: &DbPool, body: &[u8]) -> Result<()> {
    merge_lww_simple(
        pool,
        "category",
        body,
        |row: &CategoryPayload| (!row.id.is_empty()).then(|| row.id.clone()),
        |conn, row: CategoryPayload| {
            let cur: Option<(String, Option<String>)> = conn
                .query_row(
                    "SELECT updated_at, deleted_at FROM categories WHERE id = ?1",
                    rusqlite::params![row.id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let should_apply = match &cur {
                None => true,
                Some((cur_upd, _)) => row.updated_at.as_str() > cur_upd.as_str(),
            };
            if !should_apply {
                return Ok(());
            }

            if cur.is_none() {
                conn.execute(
                    "INSERT INTO categories(id, name, color, icon, builtin, sort_order, updated_at, deleted_at)
                     VALUES(?, ?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![row.id, row.name, row.color, row.icon, row.builtin as i64, row.sort_order, row.updated_at, row.deleted_at],
                )?;
            } else {
                conn.execute(
                    "UPDATE categories SET name = ?, color = ?, icon = ?, builtin = ?,
                                            sort_order = ?, updated_at = ?, deleted_at = ?
                     WHERE id = ?",
                    rusqlite::params![row.name, row.color, row.icon, row.builtin as i64, row.sort_order, row.updated_at, row.deleted_at, row.id],
                )?;
            }

            // The peer deleted this category: clear it from the app groups filed
            // under it here, and push those groups to the other devices. Done only
            // the once it goes from present to deleted.
            let cur_deleted = matches!(cur, Some((_, Some(_))));
            let just_deleted = row.deleted_at.is_some() && !cur_deleted;
            if just_deleted {
                crate::repo::categories::cascade_category_deletion(conn, &row.id, &row.updated_at)?;
            }
            Ok(())
        },
    )
    .await
}

async fn merge_app_icons(pool: &DbPool, body: &[u8]) -> Result<()> {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
    let rows: Vec<AppIconPayload> = parse_rows("app_icons", body)?;
    for row in rows {
        if row.process_name.is_empty() {
            continue;
        }
        let process_name = row.process_name;
        let icon_bytes = match BASE64.decode(row.icon_png_base64.as_bytes()) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("app_icon {process_name}: base64 does not decode: {e}");
                continue;
            }
        };
        let updated_at = row.updated_at;
        let deleted_at = row.deleted_at;

        let process_name_db = process_name.clone();
        let icon_bytes_db = icon_bytes.clone();
        let updated_at_db = updated_at.clone();
        let deleted_at_db = deleted_at.clone();
        let applied: bool = match pool
            .0
            .call(move |conn| {
                if !is_remote_newer(
                    conn,
                    "SELECT updated_at FROM app_icons WHERE process_name = ?1",
                    rusqlite::params![process_name_db],
                    &updated_at_db,
                )? {
                    return Ok(false);
                }
                conn.execute(
                    "INSERT INTO app_icons(process_name, icon_png, updated_at, deleted_at)
                     VALUES(?, ?, ?, ?)
                     ON CONFLICT(process_name) DO UPDATE SET
                       icon_png   = excluded.icon_png,
                       updated_at = excluded.updated_at,
                       deleted_at = excluded.deleted_at",
                    rusqlite::params![process_name_db, icon_bytes_db, updated_at_db, deleted_at_db],
                )
                .db()?;
                Ok(true)
            })
            .await
        {
            Ok(v) => v,
            Err(e) => {
                log::warn!("app_icon {process_name}: merge failed: {e}");
                continue;
            }
        };

        // The cache file holds the PNG in this row's `icon_png`, and is removed when
        // the row is a tombstone; so it changes only when the row did, and nothing
        // happens when the record was older than the row.
        if applied {
            let path = match crate::repo::app_icons::icon_cache_path(&process_name) {
                Ok(p) => p,
                Err(e) => {
                    log::warn!("app_icon {process_name}: no icon cache path: {e}");
                    continue;
                }
            };
            if deleted_at.is_some() {
                let _ = std::fs::remove_file(&path);
            } else {
                crate::repo::app_icons::write_cache_file(&path, &icon_bytes);
            }
        }
    }
    Ok(())
}

async fn merge_app_groups(pool: &DbPool, body: &[u8]) -> Result<()> {
    merge_lww_simple(
        pool,
        "app_group",
        body,
        |row: &AppGroupPayload| (!row.id.is_empty()).then(|| row.id.clone()),
        |conn, row: AppGroupPayload| {
            if !is_remote_newer(
                conn,
                "SELECT updated_at FROM app_groups WHERE id = ?1",
                rusqlite::params![row.id],
                &row.updated_at,
            )? {
                return Ok(());
            }
            conn.execute(
                "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, ?, ?)
                 ON CONFLICT(id) DO UPDATE SET
                   display_name = excluded.display_name,
                   category_id  = excluded.category_id,
                   updated_at   = excluded.updated_at,
                   deleted_at   = excluded.deleted_at",
                rusqlite::params![
                    row.id,
                    row.display_name,
                    row.category_id,
                    row.updated_at,
                    row.deleted_at
                ],
            )?;
            Ok(())
        },
    )
    .await
}

async fn merge_app_group_members(pool: &DbPool, body: &[u8]) -> Result<()> {
    merge_lww_simple(
        pool,
        "app_group_member",
        body,
        |row: &AppGroupMemberPayload| {
            (!row.process_name.is_empty() && !row.group_id.is_empty())
                .then(|| row.process_name.clone())
        },
        |conn, row: AppGroupMemberPayload| {
            if !is_remote_newer(
                conn,
                "SELECT updated_at FROM app_group_members WHERE process_name = ?1",
                rusqlite::params![row.process_name],
                &row.updated_at,
            )? {
                return Ok(());
            }
            conn.execute(
                "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, ?)
                 ON CONFLICT(process_name) DO UPDATE SET
                   group_id   = excluded.group_id,
                   updated_at = excluded.updated_at,
                   deleted_at = excluded.deleted_at",
                rusqlite::params![
                    row.process_name,
                    row.group_id,
                    row.updated_at,
                    row.deleted_at
                ],
            )?;
            Ok(())
        },
    )
    .await
}

/// Handles `device.<owner>.tombstone.json`: that device was removed from the
/// cloud. Deletes its activity rows held here from before `clearedAt`, then
/// marks its device card deleted.
async fn merge_tombstone(pool: &DbPool, owner_device_id: &str, body: &[u8]) -> Result<()> {
    let payload: TombstonePayload = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("tombstone of device {owner_device_id}: does not parse: {e}");
            return Ok(());
        }
    };
    if payload.cleared_at.is_empty() {
        log::warn!("tombstone of device {owner_device_id}: clearedAt is empty");
        return Ok(());
    }
    let owner = owner_device_id.to_string();
    let cleared_at = payload.cleared_at;
    let deleted = pool
        .0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            let n = tx
                .execute(
                    "DELETE FROM activities
                     WHERE device_id = ?1 AND updated_at < ?2",
                    rusqlite::params![owner, cleared_at],
                )
                .db()?;
            // Marked only if the device has not uploaded its device info since
            // `clearedAt`: a device still in use keeps uploading it, and
            // `merge_device_meta` clears `deleted_at` again when that arrives.
            tx.execute(
                "UPDATE devices
                 SET deleted_at = ?2, updated_at = ?2
                 WHERE device_id = ?1
                   AND updated_at < ?2
                   AND (deleted_at IS NULL OR deleted_at < ?2)",
                rusqlite::params![owner, cleared_at],
            )
            .db()?;
            tx.commit().db()?;
            Ok(n)
        })
        .await?;

    if deleted > 0 {
        log::info!("tombstone applied: device={owner_device_id} deleted {deleted} activity rows");
    }
    Ok(())
}

async fn merge_device_meta(pool: &DbPool, device_id: &str, body: &[u8]) -> Result<()> {
    let parsed: Value = serde_json::from_slice(body).map_err(|e| Error::SyncParse {
        kind: "device_meta",
        source: e,
    })?;
    // `{}` is what a device uploads before it has a devices row: nothing to merge.
    if parsed == serde_json::json!({}) {
        return Ok(());
    }
    let row: DeviceMetaPayload = serde_json::from_value(parsed).map_err(|e| Error::SyncParse {
        kind: "device_meta",
        source: e,
    })?;
    let device_id = device_id.to_string();
    pool.0
        .call(move |conn| {
            if !is_remote_newer(
                conn,
                "SELECT updated_at FROM devices WHERE device_id = ?1",
                rusqlite::params![device_id],
                &row.updated_at,
            )? {
                return Ok(());
            }
            // Newer device info means the device is still in use: clear `deleted_at`,
            // so a card a tombstone marked deleted comes back.
            conn.execute(
                "INSERT INTO devices(device_id, display_name, color, icon, os, last_seen_at, is_self, updated_at, deleted_at)
                 VALUES(?, ?, ?, ?, ?, ?, 0, ?, NULL)
                 ON CONFLICT(device_id) DO UPDATE SET
                   display_name = excluded.display_name,
                   color = excluded.color,
                   icon = excluded.icon,
                   os = excluded.os,
                   last_seen_at = excluded.last_seen_at,
                   updated_at = excluded.updated_at,
                   deleted_at = NULL",
                rusqlite::params![device_id, row.display_name, row.color, row.icon, row.os, row.last_seen_at, row.updated_at],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Writes one activity row from a file into the local table: inserted when this
/// device has no such row, overwritten in full when the file's row is newer.
async fn upsert_remote_activity(
    pool: &DbPool,
    device_id: &str,
    row: &ActivityPayload,
    excluded: bool,
) -> Result<()> {
    let device_id = device_id.to_string();
    let remote_id = row.id.to_string();
    let row = row.clone();
    pool.0
        .call(move |conn| {
            let existing: Option<(i64, String)> = conn
                .query_row(
                    "SELECT id, updated_at FROM activities
                     WHERE device_id = ?1 AND remote_id = ?2",
                    rusqlite::params![device_id, remote_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok();
            let local_hour = row.local_hour as u8;
            let window_title = row.window_title.as_deref().unwrap_or("");
            match existing {
                None => {
                    // A row this device has not seen: insert it under an id of our own.
                    conn.execute(
                        "INSERT INTO activities(
                           started_at, ended_at, duration_secs, local_date, local_hour,
                           process_name, window_title, category_id, screenshot_path,
                           device_id, remote_id, updated_at, origin, excluded, url_host
                         ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?, 'remote', ?, ?)",
                        rusqlite::params![
                            row.started_at,
                            row.ended_at,
                            row.duration_secs,
                            row.local_date,
                            local_hour,
                            row.process_name,
                            window_title,
                            row.category_id,
                            device_id,
                            remote_id,
                            row.updated_at,
                            excluded,
                            row.url_host,
                        ],
                    )
                    .db()?;
                }
                Some((id, cur_updated)) => {
                    if row.updated_at > cur_updated {
                        conn.execute(
                            "UPDATE activities SET
                               started_at = ?, ended_at = ?, duration_secs = ?,
                               local_date = ?, local_hour = ?,
                               process_name = ?, window_title = ?, category_id = ?,
                               updated_at = ?, url_host = ?
                             WHERE id = ?",
                            rusqlite::params![
                                row.started_at,
                                row.ended_at,
                                row.duration_secs,
                                row.local_date,
                                local_hour,
                                row.process_name,
                                window_title,
                                row.category_id,
                                row.updated_at,
                                row.url_host,
                                id,
                            ],
                        )
                        .db()?;
                    }
                }
            }
            Ok(())
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::fresh_test_pool;

    const DAY: &str = "2026-05-15";
    const OTHER_DEVICE: &str = "device-a";

    /// 精度为 0 时原串返回，一个字节都不碰——Drive 的毫秒时间戳格式不能被改写。
    #[test]
    fn rewind_cursor_zero_precision_returns_input_untouched() {
        let cursor = "2026-05-15T10:00:00.123456789Z";
        assert_eq!(rewind_cursor(cursor, Duration::ZERO).unwrap(), cursor);
    }

    /// 秒级精度退一秒，结果保持「秒 + Z」的形状，能跟后端返回的字符串按字典序比。
    #[test]
    fn rewind_cursor_one_second_keeps_rfc3339_shape() {
        assert_eq!(
            rewind_cursor("2026-05-15T10:00:00Z", Duration::from_secs(1)).unwrap(),
            "2026-05-15T09:59:59Z"
        );
    }

    /// 解析不了的游标是后端的 bug，要整轮报错，不能悄悄跳过退格。
    #[test]
    fn rewind_cursor_unparseable_is_an_error() {
        assert!(matches!(
            rewind_cursor("not-a-time", Duration::from_secs(1)),
            Err(Error::SyncTimeFormat(_))
        ));
    }

    /// 跨设备路径 (device_id != self_id)：
    /// - ndjson 中的 id 全部覆盖到 mirror（UPDATE 已存在 / INSERT 新行）
    /// - ndjson 中**不在**的 remote_id 通过 mirror 收敛 DELETE 掉
    #[tokio::test]
    async fn merge_activities_cross_device_converges_mirror() {
        let pool = fresh_test_pool().await;
        seed_mirror_rows(&pool, OTHER_DEVICE, &["1", "2", "3", "4", "5"]).await;

        let body = ndjson_for_ids(&[1, 2, 3, 6]);
        merge_activities(&pool, OTHER_DEVICE, DAY, body.as_bytes(), &[])
            .await
            .unwrap();

        let ids = remote_ids_for(&pool, OTHER_DEVICE).await;
        assert_eq!(
            ids,
            vec!["1".to_string(), "2".into(), "3".into(), "6".into()],
            "ndjson 包含 1/2/3/6，不在的 4/5 应被 mirror 收敛 DELETE"
        );
    }

    /// 解析失败的 ndjson：mirror 收敛**不**触发，避免半截文件误删一大堆。
    #[tokio::test]
    async fn merge_activities_parse_failure_skips_mirror_convergence() {
        let pool = fresh_test_pool().await;
        seed_mirror_rows(&pool, OTHER_DEVICE, &["1", "2", "3", "4", "5"]).await;

        // 第二行故意写坏 JSON
        let body = format!(
            "{}\nthis is not valid json {{\n{}\n",
            payload_line(1),
            payload_line(2),
        );
        merge_activities(&pool, OTHER_DEVICE, DAY, body.as_bytes(), &[])
            .await
            .unwrap();

        let ids = remote_ids_for(&pool, OTHER_DEVICE).await;
        assert_eq!(
            ids,
            vec![
                "1".to_string(),
                "2".into(),
                "3".into(),
                "4".into(),
                "5".into(),
            ],
            "解析失败时 mirror 收敛应跳过：原 5 行全部保留"
        );
    }

    /// 合并期打标：对端行 INSERT 进本机时按**本机**规则判 excluded；
    /// 后续更新走 LWW UPDATE（不触碰 excluded 列），标记必须存活。
    #[tokio::test]
    async fn merge_activities_tags_excluded_and_lww_update_keeps_it() {
        let pool = fresh_test_pool().await;
        let rules = vec![crate::capture::ignore::IgnoreRule {
            process_name: "Code".into(), // payload_line 固定 process_name = "Code"
            title_keyword: None,
        }];

        let body = ndjson_for_ids(&[1]);
        merge_activities(&pool, OTHER_DEVICE, DAY, body.as_bytes(), &rules)
            .await
            .unwrap();
        assert_eq!(
            excluded_of(&pool, OTHER_DEVICE, "1").await,
            1,
            "命中本机规则的对端行 INSERT 时就该打标"
        );

        // 同一行的更新（updated_at 更晚 → 走 LWW UPDATE 分支），这次不带规则——
        // 模拟"对端不知道本机规则"的真实情况，标记不该被冲掉
        let newer = ActivityPayload {
            id: 1,
            started_at: format!("{DAY}T10:01:00Z"),
            ended_at: format!("{DAY}T10:05:00Z"),
            duration_secs: 240,
            local_date: DAY.into(),
            local_hour: 10,
            process_name: "Code".into(),
            window_title: None,
            category_id: "other".into(),
            updated_at: format!("{DAY}T23:59:59Z"),
            url_host: None,
        };
        let body = serde_json::to_string(&newer).unwrap();
        merge_activities(&pool, OTHER_DEVICE, DAY, body.as_bytes(), &[])
            .await
            .unwrap();
        let (dur, excl) = row_state(&pool, OTHER_DEVICE, "1").await;
        assert_eq!(dur, 240, "LWW UPDATE 应生效（duration 更新）");
        assert_eq!(excl, 1, "LWW UPDATE 不触碰 excluded——本机打的标必须存活");
    }

    async fn excluded_of(pool: &DbPool, device_id: &str, remote_id: &str) -> i64 {
        row_state(pool, device_id, remote_id).await.1
    }

    async fn host_of(pool: &DbPool, device_id: &str, remote_id: &str) -> Option<String> {
        let device_id = device_id.to_string();
        let remote_id = remote_id.to_string();
        pool.0
            .call(move |conn| {
                let v = conn
                    .query_row(
                        "SELECT url_host FROM activities WHERE device_id = ?1 AND remote_id = ?2",
                        rusqlite::params![device_id, remote_id],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .db()?;
                Ok(v)
            })
            .await
            .unwrap()
    }

    /// url_host 随 ndjson 往返：INSERT 带上、LWW UPDATE 跟着来源更新；
    /// None 时不序列化该字段；老版本写的行（没有 urlHost）解析不报错、落 None。
    #[tokio::test]
    async fn merge_activities_carries_url_host() {
        let pool = fresh_test_pool().await;
        let mut p = ActivityPayload {
            id: 7,
            started_at: format!("{DAY}T10:00:00Z"),
            ended_at: format!("{DAY}T10:00:30Z"),
            duration_secs: 30,
            local_date: DAY.into(),
            local_hour: 10,
            process_name: "Google Chrome".into(),
            window_title: Some("Hindsight - GitHub".into()),
            category_id: "other".into(),
            updated_at: format!("{DAY}T10:00:30Z"),
            url_host: Some("github.com".into()),
        };
        let body = serde_json::to_string(&p).unwrap();
        assert!(body.contains("\"urlHost\":\"github.com\""), "{body}");
        merge_activities(&pool, OTHER_DEVICE, DAY, body.as_bytes(), &[])
            .await
            .unwrap();
        assert_eq!(
            host_of(&pool, OTHER_DEVICE, "7").await,
            Some("github.com".into()),
            "INSERT 应带域名"
        );

        // 来源设备重推更新的同一行（updated_at 更新）：域名随之更新
        p.updated_at = format!("{DAY}T11:00:00Z");
        p.url_host = Some("docs.github.com".into());
        let body = serde_json::to_string(&p).unwrap();
        merge_activities(&pool, OTHER_DEVICE, DAY, body.as_bytes(), &[])
            .await
            .unwrap();
        assert_eq!(
            host_of(&pool, OTHER_DEVICE, "7").await,
            Some("docs.github.com".into()),
            "LWW UPDATE 应更新域名"
        );

        // None 不序列化；老版本 ndjson 没有该字段也能解析
        p.url_host = None;
        assert!(!serde_json::to_string(&p).unwrap().contains("urlHost"));
        let legacy = format!(
            r#"{{"id":8,"startedAt":"{DAY}T12:00:00Z","endedAt":"{DAY}T12:00:30Z","durationSecs":30,"localDate":"{DAY}","localHour":12,"processName":"Google Chrome","windowTitle":null,"categoryId":"other","updatedAt":"{DAY}T12:00:30Z"}}"#
        );
        merge_activities(&pool, OTHER_DEVICE, DAY, legacy.as_bytes(), &[])
            .await
            .unwrap();
        assert_eq!(host_of(&pool, OTHER_DEVICE, "8").await, None);
    }

    async fn row_state(pool: &DbPool, device_id: &str, remote_id: &str) -> (i64, i64) {
        let device_id = device_id.to_string();
        let remote_id = remote_id.to_string();
        pool.0
            .call(move |conn| {
                let v = conn
                    .query_row(
                        "SELECT duration_secs, excluded FROM activities
                         WHERE device_id = ?1 AND remote_id = ?2",
                        rusqlite::params![device_id, remote_id],
                        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
                    )
                    .unwrap();
                Ok(v)
            })
            .await
            .unwrap()
    }

    fn payload_line(id: i64) -> String {
        let p = ActivityPayload {
            id,
            started_at: format!("{DAY}T10:0{id}:00Z"),
            ended_at: format!("{DAY}T10:0{id}:30Z"),
            duration_secs: 30,
            local_date: DAY.into(),
            local_hour: 10,
            process_name: "Code".into(),
            window_title: None,
            category_id: "other".into(),
            updated_at: format!("{DAY}T10:0{id}:30Z"),
            url_host: None,
        };
        serde_json::to_string(&p).unwrap()
    }

    fn ndjson_for_ids(ids: &[i64]) -> String {
        ids.iter()
            .map(|id| payload_line(*id))
            .collect::<Vec<_>>()
            .join("\n")
    }

    async fn seed_mirror_rows(pool: &DbPool, device_id: &str, remote_ids: &[&str]) {
        let device_id = device_id.to_string();
        let remote_ids: Vec<String> = remote_ids.iter().map(|s| s.to_string()).collect();
        pool.0
            .call(move |conn| {
                for r in &remote_ids {
                    conn.execute(
                        "INSERT INTO activities(
                            started_at, ended_at, duration_secs, local_date, local_hour,
                            process_name, window_title, category_id, device_id, remote_id,
                            updated_at, origin
                         ) VALUES(
                            '2026-05-15T10:00:00Z', '2026-05-15T10:00:30Z', 30, '2026-05-15', 10,
                            'Code', '', 'other', ?1, ?2, '2026-05-15T10:00:00Z', 'remote'
                         )",
                        rusqlite::params![device_id, r],
                    )
                    .db()?;
                }
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn remote_ids_for(pool: &DbPool, device_id: &str) -> Vec<String> {
        let device_id = device_id.to_string();
        pool.0
            .call(move |conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT remote_id FROM activities
                         WHERE device_id = ?1 ORDER BY remote_id",
                    )
                    .db()?;
                let rows = stmt
                    .query_map(rusqlite::params![device_id], |r| r.get::<_, String>(0))
                    .db()?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r.db()?);
                }
                Ok(out)
            })
            .await
            .unwrap()
    }

    // ═════════ metadata merge 直测(补测 C 批):fixture / 查询 helper ═════════

    /// 三个梯度时间戳:T_OLD < T_MID < T_NEW,期望值全部手推。
    const T_OLD: &str = "2026-06-01T00:00:00Z";
    const T_MID: &str = "2026-06-02T00:00:00Z";
    const T_NEW: &str = "2026-06-03T00:00:00Z";

    /// 通用 fixture:一条参数化 SQL(全 String 参数)直写表。
    async fn exec_sql(pool: &DbPool, sql: &'static str, params: Vec<Option<String>>) {
        pool.0
            .call(move |conn| {
                let refs: Vec<&dyn rusqlite::ToSql> =
                    params.iter().map(|p| p as &dyn rusqlite::ToSql).collect();
                conn.execute(sql, refs.as_slice()).db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    fn s(v: &str) -> Option<String> {
        Some(v.to_string())
    }

    /// 读一行的若干 TEXT 列(逐列 Option<String>),行不存在返回 None。
    async fn read_row(
        pool: &DbPool,
        sql: &'static str,
        key: &str,
        cols: usize,
    ) -> Option<Vec<Option<String>>> {
        let key = key.to_string();
        pool.0
            .call(move |conn| {
                use rusqlite::OptionalExtension;
                let r = conn
                    .query_row(sql, rusqlite::params![key], |r| {
                        (0..cols).map(|i| r.get::<_, Option<String>>(i)).collect()
                    })
                    .optional()
                    .db()?;
                Ok(r)
            })
            .await
            .unwrap()
    }

    /// outbox 现有行的 (entity, entity_pk) 列表,按 id 序 —— 断言回灌行为用。
    async fn outbox_entries(pool: &DbPool) -> Vec<(String, String)> {
        pool.0
            .call(|conn| {
                let mut stmt = conn
                    .prepare("SELECT entity, entity_pk FROM sync_outbox ORDER BY id")
                    .db()?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .db()?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .db()?;
                Ok(rows)
            })
            .await
            .unwrap()
    }

    fn category_body(id: &str, name: &str, updated_at: &str, deleted_at: Option<&str>) -> Vec<u8> {
        serde_json::to_vec(&vec![CategoryPayload {
            id: id.into(),
            name: name.into(),
            color: "#123456".into(),
            icon: "Tag".into(),
            builtin: false,
            sort_order: 3,
            updated_at: updated_at.into(),
            deleted_at: deleted_at.map(String::from),
        }])
        .unwrap()
    }

    // ───────── 任务 2:merge_categories ─────────

    /// 远端 updated_at 较旧(即使带墓碑)不得覆盖本地,更不得触发级联。
    #[tokio::test]
    async fn merge_categories_remote_older_does_not_overwrite() {
        let pool = fresh_test_pool().await;
        exec_sql(
            &pool,
            "INSERT INTO categories(id, name, color, icon, builtin, sort_order, updated_at, deleted_at)
             VALUES('work', '本地新名', '#aaaaaa', 'Star', 0, 1, ?1, NULL)",
            vec![s(T_MID)],
        )
        .await;

        // 远端 T_OLD < 本地 T_MID,且远端还带 deleted_at —— LWW 输了就该整行按兵不动
        merge_categories(
            &pool,
            &category_body("work", "远端旧名", T_OLD, Some(T_OLD)),
        )
        .await
        .unwrap();

        let row = read_row(
            &pool,
            "SELECT name, updated_at, deleted_at FROM categories WHERE id = ?1",
            "work",
            3,
        )
        .await
        .expect("本地行应仍存在");
        assert_eq!(
            row,
            vec![s("本地新名"), s(T_MID), None],
            "远端较旧:name / updated_at / deleted_at 都不得被改动"
        );
        assert!(
            outbox_entries(&pool).await.is_empty(),
            "LWW 输了不得触发级联回灌 outbox"
        );
    }

    /// 远端墓碑首次到达时触发级联:引用该分类的组回到未分类,并回灌一行 outbox
    /// 让这个本地改动也推回云端;同一份 body 再来一次是幂等的 —— LWW 的严格 >
    /// 挡住第二次,级联不重跑。
    #[tokio::test]
    async fn merge_categories_first_tombstone_cascades_then_idempotent() {
        let pool = fresh_test_pool().await;
        exec_sql(
            &pool,
            "INSERT INTO categories(id, name, color, icon, builtin, sort_order, updated_at, deleted_at)
             VALUES('work', '工作', '#aaaaaa', 'Star', 0, 1, ?1, NULL)",
            vec![s(T_OLD)],
        )
        .await;
        exec_sql(
            &pool,
            "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
             VALUES('grp1', '组一', 'work', ?1, NULL)",
            vec![s(T_OLD)],
        )
        .await;

        let body = category_body("work", "工作", T_NEW, Some(T_NEW));
        merge_categories(&pool, &body).await.unwrap();

        // 分类本体落墓碑
        let cat = read_row(
            &pool,
            "SELECT updated_at, deleted_at FROM categories WHERE id = ?1",
            "work",
            2,
        )
        .await
        .unwrap();
        assert_eq!(cat, vec![s(T_NEW), s(T_NEW)], "分类应带上远端墓碑");

        // 级联:引用该分类的组回到未分类
        let grp = read_row(
            &pool,
            "SELECT category_id, updated_at FROM app_groups WHERE id = ?1",
            "grp1",
            2,
        )
        .await
        .unwrap();
        assert_eq!(grp, vec![None, s(T_NEW)], "引用该分类的组应被清到未分类");

        // 级联改了本地的组,这个改动也要推回云端:恰好一行
        let entries = outbox_entries(&pool).await;
        assert_eq!(
            entries,
            vec![("app_group".to_string(), "grp1".to_string())],
            "组脱钩应回灌一行 outbox"
        );

        // 同 body 再 merge 一次:LWW 严格 > 挡住,级联不重跑、outbox 不再增长
        merge_categories(&pool, &body).await.unwrap();
        assert_eq!(
            outbox_entries(&pool).await.len(),
            1,
            "重复 merge 不得二次级联(outbox 行数应保持 1)"
        );
    }

    /// merge_app_group_members:LWW 矩阵(含墓碑换组)。
    #[tokio::test]
    async fn merge_app_group_members_lww_matrix() {
        let pool = fresh_test_pool().await;
        for g in ["g1", "g2"] {
            exec_sql(
                &pool,
                "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                 VALUES(?1, ?1, NULL, ?2, NULL)",
                vec![s(g), s(T_OLD)],
            )
            .await;
        }
        for (proc, ts) in [("M-newer", T_OLD), ("M-older", T_NEW), ("M-equal", T_MID)] {
            exec_sql(
                &pool,
                "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                 VALUES(?1, 'g1', ?2, NULL)",
                vec![s(proc), s(ts)],
            )
            .await;
        }
        let remote_row = |p: &str, tomb: bool| AppGroupMemberPayload {
            process_name: p.into(),
            group_id: "g2".into(),
            updated_at: T_MID.into(),
            deleted_at: tomb.then(|| T_MID.into()),
        };
        let body = serde_json::to_vec(&vec![
            remote_row("M-newer", true),
            remote_row("M-older", false),
            remote_row("M-equal", false),
            remote_row("M-missing", false),
        ])
        .unwrap();
        merge_app_group_members(&pool, &body).await.unwrap();

        let get = |p: &'static str| {
            read_row(
                &pool,
                "SELECT group_id, updated_at, deleted_at FROM app_group_members WHERE process_name = ?1",
                p,
                3,
            )
        };
        assert_eq!(
            get("M-newer").await.unwrap(),
            vec![s("g2"), s(T_MID), s(T_MID)],
            "远端较新应覆盖(换组 + 墓碑)"
        );
        assert_eq!(
            get("M-older").await.unwrap(),
            vec![s("g1"), s(T_NEW), None],
            "远端较旧应保留本地"
        );
        assert_eq!(
            get("M-equal").await.unwrap(),
            vec![s("g1"), s(T_MID), None],
            "同 updated_at 严格不覆盖"
        );
        assert_eq!(
            get("M-missing").await.unwrap(),
            vec![s("g2"), s(T_MID), None],
            "本地缺行应插入"
        );
    }

    // ───────── 任务 8:merge_app_icons(DB 侧) ─────────

    fn icon_body(rows: Vec<(&str, String, &str)>) -> Vec<u8> {
        serde_json::to_vec(
            &rows
                .into_iter()
                .map(|(p, b64, ts)| AppIconPayload {
                    process_name: p.into(),
                    icon_png_base64: b64,
                    updated_at: ts.into(),
                    deleted_at: None,
                })
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    async fn icon_row(pool: &DbPool, process: &str) -> Option<(Vec<u8>, String)> {
        let process = process.to_string();
        pool.0
            .call(move |conn| {
                use rusqlite::OptionalExtension;
                let r = conn
                    .query_row(
                        "SELECT icon_png, updated_at FROM app_icons WHERE process_name = ?1",
                        rusqlite::params![process],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                    .db()?;
                Ok(r)
            })
            .await
            .unwrap()
    }

    /// 坏 base64 只跳那一行,同文件后续好行照常落库。
    // 好行应用成功后会写 icon 文件 cache(路径读 HINDSIGHT_DATA_DIR),必须借
    // env 锁把数据根指到唯一临时目录,不污染真实用户目录。锁横跨 merge 的 await:
    // #[tokio::test] 单线程 runtime,不自死锁(同 chat/engine.rs 夹具的先例)。
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn merge_app_icons_bad_base64_skips_row_keeps_rest() {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
        let pool = fresh_test_pool().await;
        let good_bytes: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 9, 8, 7];

        let body = icon_body(vec![
            ("Icon-bad", "!!!这不是base64!!!".into(), T_MID),
            ("Icon-good", BASE64.encode(&good_bytes), T_MID),
        ]);

        let _env_lock = crate::repo::test_util::lock_data_dir_env();
        let dir =
            std::env::temp_dir().join(format!("hindsight-pull-icons-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let prev = std::env::var("HINDSIGHT_DATA_DIR").ok();
        std::env::set_var("HINDSIGHT_DATA_DIR", &dir);
        let res = merge_app_icons(&pool, &body).await;
        match prev {
            Some(v) => std::env::set_var("HINDSIGHT_DATA_DIR", v),
            None => std::env::remove_var("HINDSIGHT_DATA_DIR"),
        }
        res.unwrap();

        assert!(
            icon_row(&pool, "Icon-bad").await.is_none(),
            "坏 base64 行应被跳过,不落库"
        );
        assert_eq!(
            icon_row(&pool, "Icon-good").await.unwrap(),
            (good_bytes, T_MID.to_string()),
            "同文件的好行应照常解码落库(字节精确一致)"
        );
    }

    /// LWW:远端较旧 / 同 updated_at 都不得覆盖本地已有 icon 字节。
    /// (两行都走 not-applied 路径,不会碰文件 cache,无需 env 隔离。)
    #[tokio::test]
    async fn merge_app_icons_lww_old_or_equal_does_not_overwrite() {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
        let pool = fresh_test_pool().await;
        let local_bytes: Vec<u8> = vec![1, 1, 2, 3, 5, 8];
        let remote_bytes: Vec<u8> = vec![9, 9, 9];

        for (proc, ts) in [("I-older", T_NEW), ("I-equal", T_MID)] {
            let bytes = local_bytes.clone();
            let proc = proc.to_string();
            let ts = ts.to_string();
            pool.0
                .call(move |conn| {
                    conn.execute(
                        "INSERT INTO app_icons(process_name, icon_png, updated_at, deleted_at)
                         VALUES(?1, ?2, ?3, NULL)",
                        rusqlite::params![proc, bytes, ts],
                    )
                    .db()?;
                    Ok(())
                })
                .await
                .unwrap();
        }

        let body = icon_body(vec![
            ("I-older", BASE64.encode(&remote_bytes), T_MID), // T_MID < 本地 T_NEW
            ("I-equal", BASE64.encode(&remote_bytes), T_MID), // 平局
        ]);
        merge_app_icons(&pool, &body).await.unwrap();

        assert_eq!(
            icon_row(&pool, "I-older").await.unwrap(),
            (local_bytes.clone(), T_NEW.to_string()),
            "远端较旧不得覆盖本地字节"
        );
        assert_eq!(
            icon_row(&pool, "I-equal").await.unwrap(),
            (local_bytes, T_MID.to_string()),
            "同 updated_at 严格不覆盖(平局本地赢)"
        );
    }
}
