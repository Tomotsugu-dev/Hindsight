//! Pull: merges the files other devices wrote to the cloud into the local
//! database.
//!
//! Every `merge` must be idempotent: the cursor stops before the first file
//! that failed, so the files merged after it are merged again next round.

mod activities;
mod app_group_members;
mod app_groups;
mod app_icons;
mod categories;
mod device_meta;
mod site_rules;
mod super_categories;
#[cfg(test)]
mod test_util;
mod tombstone;

use rusqlite::OptionalExtension;
use std::sync::Arc;

use serde_json::Value;

use super::io;
use super::Inner;
use crate::capture::ignore::IgnoreRule;
use crate::error::{Error, Result};
use crate::storage::{DbPool, SqliteResultExt};
use crate::sync::cloud::{rewind_cursor, FailureKind};
use crate::sync::file_name::{Dataset, FileKind, FileName};

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

pub(super) async fn flush_pull(inner: &Arc<Inner>) -> Result<()> {
    let _gate = inner.flush_gate.lock().await;
    // Not signed in is not a failure: there is nothing to pull.
    if !inner.cloud().ensure_credential().await? {
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

    // Core, website rules and super-categories always run. Optional datasets follow
    // their switches; chat and screen memory also need the memory database.
    let has_mem = inner.mem.is_some();
    let mut streams: Vec<(Dataset, String)> = Vec::new();
    for (dataset, enabled) in [
        (Dataset::Core, true),
        (Dataset::SiteRules, true),
        (Dataset::SuperCategories, true),
        (Dataset::AiSummaries, sync_ai),
        (Dataset::Chat, sync_chat && has_mem),
        (Dataset::Memory, sync_scrn_mem && has_mem),
    ] {
        if enabled {
            let name = inner.cloud().pull_cursor_name(dataset);
            streams.push((dataset, io::read_cursor(&inner.pool, &name).await?));
        }
    }

    // One listing per round for all the running streams; each stream then
    // takes the files after its own cursor.
    let cursors: Vec<(Dataset, &str)> = streams
        .iter()
        .map(|(dataset, cursor)| (*dataset, cursor.as_str()))
        .collect();
    let files = inner.cloud().list(&cursors).await?;
    if files.is_empty() {
        return Ok(());
    }

    let mut applied = 0u64;
    for (dataset, cursor) in &streams {
        applied += pull_stream(inner, *dataset, cursor, &files, self_id, &ignore_rules).await?;
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
    dataset: Dataset,
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
            match FileName::parse(name) {
                Some(FileName {
                    kind: FileKind::AppGroups,
                    ..
                }) => 0,
                _ => 1,
            }
        };
        let mut idx: Vec<usize> = (0..files.len()).collect();
        idx.sort_by_key(|&i| rank(&files[i].name));
        idx
    };
    for &i in &order {
        let f = &files[i];
        let Some(FileName { device_id, kind }) = FileName::parse(&f.name) else {
            // A file name this version does not know is not ours to merge; let the cursor pass it.
            handled[i] = true;
            continue;
        };
        // Another stream's file, or one this stream merged in an earlier round:
        // nothing to do, but its cursor may pass.
        if kind.dataset() != dataset || f.modified_time.as_str() <= cursor {
            handled[i] = true;
            continue;
        }
        // This device's own files are never merged: its rows are the originals. Its
        // own tombstone in particular must not be: removing this device while keeping
        // local data uploads one, and applying it would delete that data here.
        if device_id == self_id {
            handled[i] = true;
            continue;
        }

        let body = match inner.cloud().download(&f.id).await {
            Ok(b) => b,
            // The server asks us to slow down: download nothing more this round,
            // and the cursor stops here.
            Err(e) if inner.cloud().failure_kind(&e) == FailureKind::ServerBusy => {
                log::warn!("download of {} failed, the server is busy: {e}", f.name);
                break;
            }
            Err(e) => {
                log::warn!("download of {} failed: {e}", f.name);
                continue;
            }
        };

        let res = match kind {
            FileKind::DeviceMeta => device_meta::merge(&inner.pool, &device_id, &body).await,
            FileKind::Activities(local_date) => {
                activities::merge(
                    &inner.pool,
                    &device_id,
                    &local_date.to_string(),
                    &body,
                    ignore_rules,
                )
                .await
            }
            FileKind::Categories => categories::merge(&inner.pool, &body).await,
            FileKind::SuperCategories => super_categories::merge(&inner.pool, &body).await,
            FileKind::SiteRules => site_rules::merge(&inner.pool, &body).await,
            FileKind::AppIcons => app_icons::merge(&inner.pool, &body).await,
            FileKind::AppGroups => app_groups::merge(&inner.pool, &body).await,
            FileKind::AppGroupMembers => app_group_members::merge(&inner.pool, &body).await,
            FileKind::AiSummaries => super::datasets::merge_ai_summaries(&inner.pool, &body).await,
            FileKind::Chat => {
                super::datasets::merge_chat(inner.mem.as_ref().expect("gated"), &body).await
            }
            FileKind::Memory(_) => {
                super::datasets::merge_memory_sessions(
                    inner.mem.as_ref().expect("gated"),
                    &device_id,
                    &body,
                )
                .await
            }
            FileKind::Tombstone => tombstone::merge(&inner.pool, &device_id, &body).await,
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
        let t = rewind_cursor(&t, inner.cloud().time_precision())?;
        // Keep this stream's progress when another stream is behind.
        if t.as_str() > cursor {
            let name = inner.cloud().pull_cursor_name(dataset);
            io::write_cursor(&inner.pool, &name, &t).await?;
        }
    }
    Ok(applied)
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
    F: Fn(&rusqlite::Transaction, T) -> rusqlite::Result<()> + Send + Sync + Copy + 'static,
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
