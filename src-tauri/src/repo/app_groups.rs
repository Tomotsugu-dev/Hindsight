//! App groups: one or more process names folded into what the user sees as one
//! app, with the category attached to the group. The pairing page's merge /
//! unmerge / rename / delete and group creation during capture all go through
//! here.
//!
//! Two tables (columns in docs/design/database.md):
//!   app_groups          —— the group itself: id, display name, category
//!   app_group_members   —— process name → group id
//! Both sync across devices and soft-delete.
//!
//! Invariants:
//!   - a process name that has been captured and not deleted has a live member
//!     row;
//!   - group ids are not generated, they are the process name (or the canonical
//!     name for apps in the alias table). Two devices that see the same app
//!     arrive at the same id, which is what lets sync merge them into one row;
//!   - the category lives only in app_groups.category_id.

// TODO(sync): route every write through a `with_tx` helper, then a `write` that
// executes and enqueues as one step (`write_local` for tables that must not
// sync). Closes the row-written-but-not-enqueued gap most write paths still
// have, and lets the hand-built outbox payloads go. Own branch; ADR first.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::repo::outbox::{enqueue, OutboxEntity, OutboxOp};
use crate::repo::sql::FROM_MEMBER_GROUP;
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};

/// One logical app as the frontend sees it: the `app_groups` row plus its
/// members (assembled by [`list_groups`], not a column).
///
/// A group exists because one app shows up under several process names —
/// `Google Chrome` on macOS, `chrome.exe` on Windows, both just "Chrome" to the
/// user — so display name and category live here, not on the process name.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppGroup {
    /// The unique ID of the group (initially equal to the first process_name; remains unchanged after merges)
    pub id: String,
    /// Display name for the group (e.g., "VsCode")
    pub display_name: String,
    /// The category ID of the group (None = unclassified)
    pub category_id: Option<String>,
    /// Apps in the group Vec<AppGroupMember>(process_name + recent_secs + last_device_id)
    pub members: Vec<AppGroupMember>,
}

/// Details of a single process_name member within a group.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppGroupMember {
    pub process_name: String,
    /// The total duration (in seconds) of this member over the past 7 days,
    /// aggregated by process_name and summed across devices
    pub recent_secs: i64,
    /// The device ID where this member was last seen
    /// (take the one with the latest ended_at); UI uses it for column display
    pub last_device_id: Option<String>,
}

/// Data source for the app pairing table on the "Assign apps" page: returns
/// every live app group with its id, display name, category and process-name
/// members (one group per app, spanning its process names across platforms —
/// see [`AppGroup`]).
///
/// Each member carries two derived values: seconds of activity in the last 7
/// days summed across all devices (an activity signal for sorting and display,
/// not a usage statistic), and the id of the device that most recently
/// reported it (over all history, used to pick the device column it is drawn
/// in). Groups are ordered by their largest 7-day figure, descending; members
/// whose group no longer exists are dropped.
pub async fn list_groups(pool: &DbPool) -> Result<Vec<AppGroup>> {
    let groups = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT g.id, g.display_name, g.category_id
                     FROM app_groups g
                     WHERE g.deleted_at IS NULL",
                )
                .db()?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })
                .db()?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.db()?);
            }

            // One row per live member: process_name and group_id come straight from
            // app_group_members; two more columns are derived from activities.
            //   recent_secs    — seconds this process was active in the last 7 days,
            //                    summed across ALL devices; 0 when there is none.
            //   last_device_id — device_id (an id, not a display name) of the most
            //                    recent activity over ALL history; NULL if the process
            //                    has no activity yet. The pairing table places the
            //                    member in that device's column, so a 7-day window
            //                    would leave long-idle apps with no column to land in.
            let mut mstmt = conn
                .prepare(
                    "SELECT m.process_name, m.group_id,
                            COALESCE(s.total_secs, 0)   AS recent_secs,
                            (SELECT a2.device_id
                               FROM activities a2
                               WHERE a2.process_name = m.process_name
                               ORDER BY datetime(a2.ended_at) DESC LIMIT 1) AS last_device_id
                     FROM app_group_members m
                     LEFT JOIN (
                       SELECT a.process_name,
                              SUM(a.duration_secs) AS total_secs
                       FROM activities a
                       WHERE a.local_date >= date('now','localtime','-7 days')
                       GROUP BY a.process_name
                     ) s ON s.process_name = m.process_name
                     WHERE m.deleted_at IS NULL",
                )
                .db()?;
            let mit = mstmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,         // process_name
                        r.get::<_, String>(1)?,         // group_id
                        r.get::<_, i64>(2)?,            // recent_secs
                        r.get::<_, Option<String>>(3)?, // last_device_id
                    ))
                })
                .db()?;
            let mut members: Vec<(String, String, i64, Option<String>)> = Vec::new();
            for r in mit {
                members.push(r.db()?);
            }
            Ok((out, members))
        })
        .await?;

    let (group_rows, member_rows) = groups;

    let mut groups: Vec<AppGroup> = group_rows
        .into_iter()
        .map(|(id, display_name, category_id)| AppGroup {
            id,
            display_name,
            category_id,
            members: Vec::new(), // Needed to be filled later with actual members
        })
        .collect();

    let mut groups_hash_map: HashMap<String, usize> = HashMap::new();
    for (idx, group) in groups.iter().enumerate() {
        groups_hash_map.insert(group.id.clone(), idx);
    }
    // Push members into their respective groups
    for (process_name, group_id, recent_secs, last_device_id) in member_rows {
        if let Some(&idx) = groups_hash_map.get(&group_id) {
            groups[idx].members.push(AppGroupMember {
                process_name,
                recent_secs,
                last_device_id,
            });
        }
        // A member whose group is missing is dropped silently (FK makes this
        // near-impossible; the test pins the behaviour).
    }

    // Most recently active groups first: by each group's largest recent_secs, descending.
    groups.sort_by_cached_key(|g| {
        Reverse(g.members.iter().map(|m| m.recent_secs).max().unwrap_or(0))
    });

    Ok(groups)
}

/// Soft-deletes an app group together with all its active members and enqueues
/// each row, so other devices apply the same deletion. Activity rows are untouched.
///
/// "Soft" means `deleted_at` is set and the row stays: sync carries deletions as
/// tombstoned upserts, so a physically removed row could never reach other
/// devices. The tombstones still hold `display_name` and `category_id`.
///
/// Only [`purge_with_data`] calls this, as its last step after the app's
/// activities, screenshots and OCR text were physically removed. Capturing the
/// same process name again revives it through [`ensure_group`]'s `ON CONFLICT`
/// as its own (or canonical) group: the category stored on that row comes back,
/// a manual merge into another group does not — hence the remove dialog's
/// "may reappear" warning.
///
/// Idempotent: a repeat call matches zero rows and enqueues nothing. Members,
/// group and their outbox rows commit in one transaction.
pub async fn purge_with_members(pool: &DbPool, group_id: &str) -> Result<()> {
    let id = group_id.to_string();
    let updated_at = utc_now_rfc3339();
    pool.0
        .call(move |conn| {
            // 1. List all active members of the group
            let members: Vec<String> = {
                let mut stmt = conn
                    .prepare(
                        "SELECT process_name FROM app_group_members
                         WHERE group_id = ?1 AND deleted_at IS NULL",
                    )
                    .db()?;
                let rows = stmt
                    .query_map(rusqlite::params![id], |r| r.get::<_, String>(0))
                    .db()?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r.db()?);
                }
                out
            };

            // 2. Delete each member softly, and enqueue to outbox so that
            //    the remote LWW also sees the deletion.
            //    `WHERE deleted_at IS NULL` ensures that if N=0 (already soft-deleted),
            //    it won't be enqueued again.
            let tx = conn.transaction().db()?;
            for m in &members {
                let n = tx
                    .execute(
                        "UPDATE app_group_members SET deleted_at = ?1, updated_at = ?1
                         WHERE process_name = ?2 AND deleted_at IS NULL",
                        rusqlite::params![updated_at, m],
                    )
                    .db()?;
                if n > 0 {
                    enqueue(
                        &tx,
                        OutboxOp::Upsert,
                        OutboxEntity::AppGroupMember,
                        m,
                        &serde_json::json!({ "processName": m }).to_string(),
                    )
                    .db()?;
                }
            }

            // 3. Soft-delete the group and enqueue to outbox.
            let n = tx
                .execute(
                    "UPDATE app_groups SET deleted_at = ?1, updated_at = ?1
                     WHERE id = ?2 AND deleted_at IS NULL",
                    rusqlite::params![updated_at, id],
                )
                .db()?;
            if n > 0 {
                enqueue(
                    &tx,
                    OutboxOp::Upsert,
                    OutboxEntity::AppGroup,
                    &id,
                    &serde_json::json!({ "groupId": id }).to_string(),
                )
                .db()?;
            }
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Returns the all active process names in a group.
async fn active_member_names(pool: &DbPool, group_id: &str) -> Result<Vec<String>> {
    let id = group_id.to_string();
    let names = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT process_name FROM app_group_members
                     WHERE group_id = ?1 AND deleted_at IS NULL",
                )
                .db()?;
            let rows = stmt
                .query_map(rusqlite::params![id], |r| r.get::<_, String>(0))
                .db()?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.db()?);
            }
            Ok(out)
        })
        .await?;
    Ok(names)
}

/// Backend of the "Delete anyway" button: physically removes everything an app
/// (every member process of one group) left on this machine, then soft-deletes
/// the group itself.
///
/// What goes (all of it local to this machine):
///   - activity DB: activity rows, icons, executable paths;
///   - memory DB: the processes' OCR text (frame ledger, text sessions, lines);
///   - disk: screenshot files only these processes referenced. Today a
///     screenshot belongs to exactly one activity, so all of them go; the
///     "does another app still use it" check stays in case screenshots ever
///     become shared.
///
/// Not to be confused with [`purge_with_members`], which only soft-deletes the
/// group and its pairing rows and leaves the data untouched: the app still shows
/// in stats under its raw process name and comes back whole on the next capture.
/// This function calls it as its last step.
///
/// Order is fixed: read the member list → delete the data → soft-delete the
/// group. Any other order loses the list of what to delete first.
///
/// Deliberately not done (the UI copy says so):
///   - generated daily / weekly / AI reports are not recomputed (they are
///     stored text);
///   - nothing propagates to other devices; only the group soft-delete reaches
///     the outbox. The tables deleted here either have no tombstone column or
///     would wipe the peer's own data if propagated. Peers and the cloud day
///     files keep their copies, and a cursor reset pulls them back.
///     TODO: network-wide deletion — a `purge` sync entity (process names +
///     purgedAt) that makes peers run this same local purge and rewrites the
///     cloud day files; design in ADR-0001;
///   - the two databases are not one transaction: if the memory-DB step fails,
///     the activity DB is already wiped.
pub async fn purge_with_data(
    pool: &DbPool,
    mem: &crate::memory::MemoryDb,
    screenshot_root: &std::path::Path,
    group_id: &str,
) -> Result<()> {
    let members = active_member_names(pool, group_id).await?;
    if members.is_empty() {
        // No members, so there is no activity data to remove. The group row is
        // still live, though, and still shows in the pairing table (empty groups
        // are kept on purpose after a merge) — tombstone it and sync that.
        return purge_with_members(pool, group_id).await;
    }

    // ── Activity DB (`pool`): drop every row these apps left here, and hand the
    //    screenshot paths only they referenced to the disk step at the end.
    //    How the tables hang together:
    //      process_name ─┬─ activities.process_name
    //                    │     └─ activities.screenshot_path ─┬─ screenshot file on disk
    //                    │                                    └─ screenshot_dedup_map (by path)
    //                    ├─ app_icons.process_name
    //                    └─ process_paths.process_name
    //    Order: read `screenshot_path` before deleting `activities` — the paths live
    //    only in those rows. The three tables delete by `process_name` in any order.
    //    Re-check which paths other apps still reference only after the delete
    //    (before it, these apps' own rows would count); the rest lose their
    //    `screenshot_dedup_map` rows and go to the disk step. ──
    let orphan_shots = {
        let names = members.clone();
        pool.0
            .call(move |conn| {
                let ph = vec!["?"; names.len()].join(",");
                let params: Vec<&dyn rusqlite::ToSql> =
                    names.iter().map(|s| s as &dyn rusqlite::ToSql).collect();

                // 1) Collect candidate screenshot_path needed for deletion
                let candidates: Vec<String> = {
                    let mut stmt = conn
                        .prepare(&format!(
                            "SELECT DISTINCT screenshot_path FROM activities
                             WHERE process_name IN ({ph}) AND screenshot_path IS NOT NULL"
                        ))
                        .db()?;
                    let rows = stmt
                        .query_map(params.as_slice(), |r| r.get::<_, String>(0))
                        .db()?;
                    let mut out = Vec::new();
                    for r in rows {
                        out.push(r.db()?);
                    }
                    out
                };

                // 2) Delete activities, app icons, and process paths for the given process names
                for table in ["activities", "app_icons", "process_paths"] {
                    conn.execute(
                        &format!("DELETE FROM {table} WHERE process_name IN ({ph})"),
                        params.as_slice(),
                    )
                    .db()?;
                }

                // 3) Spare screenshots another app still references (one set query:
                //    activities.screenshot_path has no index).
                let mut deletable = Vec::new();
                let still_used: HashSet<String> = {
                    let ph = vec!["?"; candidates.len()].join(",");
                    let params: Vec<&dyn rusqlite::ToSql> =
                        candidates.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
                    let mut stmt = conn.prepare(&format!(
                        "SELECT DISTINCT screenshot_path FROM activities WHERE screenshot_path IN ({ph})"
                    )).db()?;
                    let rows = stmt.query_map(params.as_slice(), |r| r.get::<_, String>(0)).db()?;
                    let mut still_used = HashSet::new();
                    for r in rows {
                        still_used.insert(r.db()?);
                    }
                    still_used
                };
                for path in candidates {
                    if !still_used.contains(&path) {
                        conn.execute(
                            "DELETE FROM screenshot_dedup_map
                             WHERE member_path = ?1 OR rep_path = ?1",
                            rusqlite::params![path],
                        )
                        .db()?;
                        deletable.push(path);
                    }
                }
                Ok(deletable)
            })
            .await?
    };

    // ── Memory DB (`mem`): drop the app's OCR text.
    //    How the tables hang together:
    //      process_name ─┬─ frames.app_id
    //                    └─ text_sessions.app_id ─┬─ session_lines.session_id (no FK)
    //                                             └─ text_sessions_fts (kept by trigger)
    //    Order: `session_lines` first — no foreign key, so nothing cascades, and its
    //    rows are only reachable through `text_sessions.id` while the sessions exist.
    //    Then `text_sessions`; the FTS index follows by trigger. `frames` hangs off
    //    nothing and can go any time. ──
    {
        let names = members.clone();
        mem.0
            .call(move |conn| {
                let ph = vec!["?"; names.len()].join(",");
                let params: Vec<&dyn rusqlite::ToSql> =
                    names.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
                conn.execute(
                    &format!(
                        "DELETE FROM session_lines WHERE session_id IN
                         (SELECT id FROM text_sessions WHERE app_id IN ({ph}))"
                    ),
                    params.as_slice(),
                )
                .db()?;
                conn.execute(
                    &format!("DELETE FROM text_sessions WHERE app_id IN ({ph})"),
                    params.as_slice(),
                )
                .db()?;
                conn.execute(
                    &format!("DELETE FROM frames WHERE app_id IN ({ph})"),
                    params.as_slice(),
                )
                .db()?;
                Ok(())
            })
            .await?;
    }

    // Soft delete for the group and its members,
    // along with the outbox (inside purge_with_members function)
    purge_with_members(pool, group_id).await?;

    // Delete orphan screenshots from the disk.
    if !orphan_shots.is_empty() {
        let root = screenshot_root.to_path_buf();
        tokio::task::spawn_blocking(move || {
            for rel in orphan_shots {
                let path = root.join(&rel);
                if path.exists() {
                    if let Err(e) = std::fs::remove_file(&path) {
                        log::warn!("failed to delete screenshot {}: {e}", path.display());
                    }
                }
            }
        })
        .await
        .map_err(|e| Error::Other(format!("screenshot delete task failed: {e}")))?;
    }

    Ok(())
}

/// Backend of the pairing page's drag: make `source_process_name` a member of
/// `target_group_id`. Only the member row's `group_id` changes; activities are
/// untouched, and the source's old solo group stays as a live empty group
/// (`unmerge` puts the process back).
/// Target missing or deleted → `InvalidInput`; already a member → no-op
/// (idempotent); member row missing or tombstoned → created / revived. The
/// change is queued for sync.
pub async fn merge(pool: &DbPool, source_process_name: &str, target_group_id: &str) -> Result<()> {
    let src = source_process_name.to_string();
    let tgt = target_group_id.to_string();
    let updated_at = utc_now_rfc3339();

    let outcome: std::result::Result<(), &'static str> = pool
        .0
        .call(move |conn| {
            let tgt_exists: bool = conn
                .query_row(
                    "SELECT 1 FROM app_groups WHERE id = ?1 AND deleted_at IS NULL",
                    rusqlite::params![tgt],
                    |_| Ok(true),
                )
                .optional()
                .db()?
                .unwrap_or(false);
            if !tgt_exists {
                return Ok(Err("target group does not exist or was deleted"));
            }

            // If the source process is already in the target group, no-op.
            let cur_group_id: Option<String> = conn
                .query_row(
                    "SELECT group_id FROM app_group_members
                     WHERE process_name = ?1 AND deleted_at IS NULL",
                    rusqlite::params![src],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .db()?;
            if cur_group_id.as_deref() == Some(tgt.as_str()) {
                return Ok(Ok(()));
            }

            // Move `src` into the target group and record the change for sync.
            // The write is a single upsert covering three states of `src`: no row
            // → insert; live in another group → repoint; tombstoned → repoint and
            // revive (reached when the pairing page is stale, or a peer's
            // tombstone was just pulled).
            let tx = conn.transaction().db()?;
            tx.execute(
                "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, NULL)
                 ON CONFLICT(process_name) DO UPDATE SET
                   group_id   = excluded.group_id,
                   updated_at = excluded.updated_at,
                   deleted_at = NULL",
                rusqlite::params![src, tgt, updated_at],
            )
            .db()?;

            enqueue(
                &tx,
                OutboxOp::Upsert,
                OutboxEntity::AppGroupMember,
                &src,
                &serde_json::json!({ "processName": src }).to_string(),
            )
            .db()?;

            tx.commit().db()?;

            Ok(Ok(()))
        })
        .await?;
    outcome.map_err(Error::InvalidInput)
}

/// Removes `process_name` from its current app group and restores it to its
/// own single-member group (`id = process_name`).
///
/// The restored group inherits the category of the group it came from and is
/// revived if it was soft-deleted. Does nothing if the process has no active
/// membership.
///
/// A group’s anchor member (`process_name == group_id`) cannot be unmerged:
/// it already represents that group’s original single-member identity. Unmerge
/// another member instead, or move the anchor member into a different group.
pub async fn unmerge(pool: &DbPool, process_name: &str) -> Result<()> {
    let p = process_name.to_string();
    let updated_at = utc_now_rfc3339();

    let result = pool
        .0
        .call(move |conn| {
            // Current group and its category; the category seeds the restored solo
            // group so unmerging does not lose it.
            let cur: Option<(String, Option<String>)> = conn
                .query_row(
                    &format!(
                        "SELECT g.id, g.category_id
                         {FROM_MEMBER_GROUP}
                         WHERE gm.process_name = ?1 AND gm.deleted_at IS NULL"
                    ),
                    rusqlite::params![p],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
                )
                .optional()
                .db()?;
            let Some((cur_group, cur_cat)) = cur else {
                return Ok(Ok(()));
            };
            if cur_group == p {
                return Ok(Err(
                    "cannot unmerge the process the group is named after; unmerge the other members, or move it into another group",
                ));
            }

            let tx = conn.transaction().db()?;
            restore_solo_group(&tx, &p, cur_cat.as_deref(), &updated_at)?;
            tx.commit().db()?;
            Ok(Ok(()))
        })
        .await?;
    result.map_err(Error::InvalidInput)
}

/// Restores `process_name`'s standalone single-member group and repoints the
/// member to it.
///
/// If the group does not exist, creates it with `process_name` as its default
/// display name. If it already exists, including as a soft-deleted row, keeps
/// its user-defined display name and updates only its category, timestamp, and
/// deletion state.
///
/// Both the group and member changes are queued for sync. Takes a transaction
/// rather than a connection: the two rows and their two outbox entries have to
/// land together or not at all, and the parameter type is what enforces it.
fn restore_solo_group(
    tx: &rusqlite::Transaction<'_>,
    process_name: &str,
    category_id: Option<&str>,
    updated_at: &str,
) -> rusqlite::Result<()> {
    // `WHERE` on the conflict branch: a live group that already carries this
    // category is left alone, so its `updated_at` is not bumped for nothing.
    let n = tx.execute(
        "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
         VALUES(?, ?, ?, ?, NULL)
         ON CONFLICT(id) DO UPDATE SET
           category_id  = excluded.category_id,
           updated_at   = excluded.updated_at,
           deleted_at   = NULL
         WHERE app_groups.deleted_at IS NOT NULL
            OR app_groups.category_id IS NOT excluded.category_id",
        rusqlite::params![process_name, process_name, category_id, updated_at],
    )?;
    if n > 0 {
        enqueue(
            tx,
            OutboxOp::Upsert,
            OutboxEntity::AppGroup,
            process_name,
            &serde_json::json!({ "groupId": process_name }).to_string(),
        )?;
    }

    let n = tx.execute(
        "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
         VALUES(?, ?, ?, NULL)
         ON CONFLICT(process_name) DO UPDATE SET
           group_id   = excluded.group_id,
           updated_at = excluded.updated_at,
           deleted_at = NULL
         WHERE app_group_members.deleted_at IS NOT NULL
            OR app_group_members.group_id IS NOT excluded.group_id",
        rusqlite::params![process_name, process_name, updated_at],
    )?;
    if n > 0 {
        enqueue(
            tx,
            OutboxOp::Upsert,
            OutboxEntity::AppGroupMember,
            process_name,
            &serde_json::json!({ "processName": process_name }).to_string(),
        )?;
    }

    Ok(())
}

/// Renames a group: only its display name and timestamp change; category and
/// members are untouched. The change is queued for sync.
pub async fn rename(pool: &DbPool, group_id: &str, new_name: &str) -> Result<()> {
    let id = group_id.to_string();
    let name = new_name.to_string();
    let updated_at = utc_now_rfc3339();

    pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            // `display_name IS NOT ?2` makes renaming to the current name a no-op.
            // `updated_at` arbitrates last-write-wins, so it has to mean "the content
            // changed", not "the row was written": bumping it for an unchanged name
            // would win the comparison against a peer that really did rename it.
            let n = tx
                .execute(
                    "UPDATE app_groups SET display_name = ?2, updated_at = ?3
                     WHERE id = ?1 AND display_name IS NOT ?2",
                    rusqlite::params![id, name, updated_at],
                )
                .db()?;
            if n > 0 {
                enqueue(
                    &tx,
                    OutboxOp::Upsert,
                    OutboxEntity::AppGroup,
                    &id,
                    &serde_json::json!({ "groupId": id }).to_string(),
                )
                .db()?;
            }
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Backend of the category picker: set a group's category, `None` to clear it.
/// Only `app_groups.category_id` changes — the source of truth for classification;
/// stats pick it up through the member → group chain. Unknown or tombstoned
/// category ids are rejected with `InvalidInput`. The change is queued for sync.
pub async fn assign_category(
    pool: &DbPool,
    group_id: &str,
    category_id: Option<String>,
) -> Result<()> {
    let id = group_id.to_string();
    let cat = category_id;
    let now = utc_now_rfc3339();

    // No foreign key on `app_groups.category_id`, so check the id here.
    if let Some(c) = cat.clone() {
        let exists = pool
            .0
            .call(move |conn| {
                conn.query_row(
                    "SELECT 1 FROM categories WHERE id = ?1 AND deleted_at IS NULL",
                    rusqlite::params![c],
                    |_| Ok(()),
                )
                .optional()
                .db()
            })
            .await?
            .is_some();
        if !exists {
            return Err(Error::InvalidInput(
                "category does not exist or was deleted",
            ));
        }
    }

    pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            // `category_id IS NOT ?2` makes re-assigning the same category a no-op,
            // clearing an already-cleared one included — `IS NOT` is NULL-safe, `!=`
            // is not. See `rename` for why an unchanged write must not bump
            // `updated_at`.
            let n = tx
                .execute(
                    "UPDATE app_groups SET category_id = ?2, updated_at = ?3
                     WHERE id = ?1 AND category_id IS NOT ?2",
                    rusqlite::params![id, cat, now],
                )
                .db()?;
            if n > 0 {
                enqueue(
                    &tx,
                    OutboxOp::Upsert,
                    OutboxEntity::AppGroup,
                    &id,
                    &serde_json::json!({ "groupId": id }).to_string(),
                )
                .db()?;
            }

            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Per-process entry point for category assignment: makes sure the process has
/// a group, then assigns the category to that group, so every member of the
/// group (its names on other platforms included) follows. Used by the
/// `assign_app_to_category` command.
pub async fn assign_category_for_process(
    pool: &DbPool,
    process_name: &str,
    category_id: Option<String>,
) -> Result<()> {
    ensure_group(pool, process_name).await?;
    let group_id = group_id_for(pool, process_name).await?;
    let Some(gid) = group_id else { return Ok(()) };
    assign_category(pool, &gid, category_id).await
}

/// The group `process_name` currently belongs to (live row); `None` if it has none.
pub async fn group_id_for(pool: &DbPool, process_name: &str) -> Result<Option<String>> {
    let p = process_name.to_string();
    let id = pool
        .0
        .call(move |conn| {
            let r = conn
                .query_row(
                    "SELECT group_id FROM app_group_members
                     WHERE process_name = ?1 AND deleted_at IS NULL",
                    rusqlite::params![p],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .db()?;
            Ok(r)
        })
        .await?;
    Ok(id)
}

/// Called on every capture tick: guarantees the process name has a live group
/// and member row. If it already does, returns at once, so almost every call is
/// one lookup and no writes.
///
/// Otherwise creates them: the group id is the canonical name from the alias
/// table (or the process name itself), the category comes from the built-in
/// rules, and both rows are queued for sync. A tombstoned member row (the app
/// was deleted) is revived here, which is why a deleted app reappears when it
/// is captured again. An existing group keeps its name and category.
pub async fn ensure_group(pool: &DbPool, process_name: &str) -> Result<()> {
    let p: String = process_name.to_string();
    if p.is_empty() || p == "Unknown" {
        return Ok(());
    }
    let now = utc_now_rfc3339();
    // Alias table: if this process name belongs to a known app, the group id is
    // its canonical name rather than the process name, so every name of that
    // app lands in the same group.
    let canonical = super::cross_os_aliases::lookup_canonical(&p);
    let group_id = canonical.map(String::from).unwrap_or_else(|| p.clone());
    let display_name = canonical.unwrap_or(&p).to_string();
    // Built-in category looked up by the canonical name, so an alias inherits
    // its app's category; only used when the group is created here.
    let builtin_cat = super::builtin_categories::match_builtin_category(canonical.unwrap_or(&p));

    pool.0
        .call(move |conn| {
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM app_group_members
                     WHERE process_name = ?1 AND deleted_at IS NULL",
                    rusqlite::params![p],
                    |_| Ok(true),
                )
                .optional()
                .db()?
                .unwrap_or(false);
            // Fast path: the process already has a live member row, which is where
            // almost every capture tick ends.
            if exists {
                return Ok(());
            }
            let tx = conn.transaction().db()?;
            // If the group exists, only revive a tombstone; leave name and category
            // alone, the user may have changed them.
            tx.execute(
                "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, ?, NULL)
                 ON CONFLICT(id) DO UPDATE SET
                   deleted_at = NULL,
                   updated_at = excluded.updated_at
                 WHERE app_groups.deleted_at IS NOT NULL",
                rusqlite::params![group_id, display_name, builtin_cat, now],
            )
            .db()?;
            enqueue(
                &tx,
                OutboxOp::Upsert,
                OutboxEntity::AppGroup,
                &group_id,
                &serde_json::json!({ "groupId": group_id }).to_string(),
            )
            .db()?;

            tx.execute(
                "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, NULL)
                 ON CONFLICT(process_name) DO UPDATE SET
                   group_id   = excluded.group_id,
                   updated_at = excluded.updated_at,
                   deleted_at = NULL",
                rusqlite::params![p, group_id, now],
            )
            .db()?;
            enqueue(
                &tx,
                OutboxOp::Upsert,
                OutboxEntity::AppGroupMember,
                &p,
                &serde_json::json!({ "processName": p }).to_string(),
            )
            .db()?;
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Writes one `app_categories` row (`cat = None` soft-deletes) without touching
/// the outbox. Only the sync pull path still calls this: the local mirror is no
/// longer maintained, but rows echoed by older peers are still stored here.
pub(crate) fn apply_app_category_change(
    conn: &Connection,
    process_name: &str,
    category_id: Option<&str>,
    now: &str,
) -> rusqlite::Result<()> {
    match category_id {
        Some(cat) => {
            conn.execute(
                "INSERT INTO app_categories(process_name, category_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, NULL)
                 ON CONFLICT(process_name) DO UPDATE SET
                   category_id = excluded.category_id,
                   updated_at  = excluded.updated_at,
                   deleted_at  = NULL",
                rusqlite::params![process_name, cat, now],
            )?;
        }
        None => {
            conn.execute(
                "UPDATE app_categories SET deleted_at = ?, updated_at = ?
                 WHERE process_name = ?",
                rusqlite::params![now, now, process_name],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::fresh_test_pool;
    use crate::storage::SqliteResultExt;

    /// 测 [`purge_with_members`]：
    /// - 组 + 所有活着的成员全部软删；
    /// - 组和每个成员各入一条 outbox，对端才能拉到同样的删除；
    /// - 幂等：再调一次 outbox 不再增长。
    #[tokio::test]
    async fn purge_with_members_soft_deletes_group_members() {
        let pool = fresh_test_pool().await;
        seed_vscode_group(&pool).await;

        purge_with_members(&pool, "vscode").await.unwrap();

        // 组 + 两成员都被软删
        assert!(group_deleted(&pool, "vscode").await, "组本身应被软删");
        assert!(member_deleted(&pool, "Code").await, "成员 Code 应被软删");
        assert!(
            member_deleted(&pool, "Code.exe").await,
            "成员 Code.exe 应被软删"
        );

        // outbox：1 条 group + 2 条 member
        let outbox_after = outbox_summary(&pool).await;
        assert!(
            outbox_after.group_count == 1,
            "至少应有 1 条 app_group outbox"
        );
        assert_eq!(
            outbox_after.member_count, 2,
            "应有 2 条 app_group_member outbox"
        );

        // 幂等：再调一次，outbox 不应再增长
        let before = outbox_total(&pool).await;
        purge_with_members(&pool, "vscode").await.unwrap();
        let after = outbox_total(&pool).await;
        assert_eq!(before, after, "幂等：第二次调用不该写新 outbox");
    }

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
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('Code', 'vscode', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('Code.exe', 'vscode', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
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

    async fn member_deleted(pool: &DbPool, process_name: &str) -> bool {
        let pn = process_name.to_string();
        pool.0
            .call(move |conn| {
                let v: Option<String> = conn
                    .query_row(
                        "SELECT deleted_at FROM app_group_members WHERE process_name = ?1",
                        rusqlite::params![pn],
                        |r| r.get(0),
                    )
                    .db()?;
                Ok(v.is_some())
            })
            .await
            .unwrap()
    }

    struct OutboxSummary {
        group_count: i64,
        member_count: i64,
    }

    async fn outbox_summary(pool: &DbPool) -> OutboxSummary {
        pool.0
            .call(|conn| {
                let g: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM sync_outbox WHERE entity = 'app_group'",
                        [],
                        |r| r.get(0),
                    )
                    .db()?;
                let m: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM sync_outbox WHERE entity = 'app_group_member'",
                        [],
                        |r| r.get(0),
                    )
                    .db()?;
                Ok(OutboxSummary {
                    group_count: g,
                    member_count: m,
                })
            })
            .await
            .unwrap()
    }

    async fn outbox_total(pool: &DbPool) -> i64 {
        pool.0
            .call(|conn| {
                let n: i64 = conn
                    .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0))
                    .db()?;
                Ok(n)
            })
            .await
            .unwrap()
    }

    /// 读某成员当前 active 的 group_id（deleted_at IS NULL）；无 active 行返回 None。
    async fn active_group_of(pool: &DbPool, process_name: &str) -> Option<String> {
        let pn = process_name.to_string();
        pool.0
            .call(move |conn| {
                let v = conn
                    .query_row(
                        "SELECT group_id FROM app_group_members
                         WHERE process_name = ?1 AND deleted_at IS NULL",
                        rusqlite::params![pn],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()
                    .db()?;
                Ok(v)
            })
            .await
            .unwrap()
    }

    /// 读 app_groups 一行的 (display_name, category_id, 是否软删)。
    async fn group_state(pool: &DbPool, id: &str) -> Option<(String, Option<String>, bool)> {
        let id = id.to_string();
        pool.0
            .call(move |conn| {
                let v = conn
                    .query_row(
                        "SELECT display_name, category_id, deleted_at IS NOT NULL
                         FROM app_groups WHERE id = ?1",
                        rusqlite::params![id],
                        |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, Option<String>>(1)?,
                                r.get::<_, bool>(2)?,
                            ))
                        },
                    )
                    .optional()
                    .db()?;
                Ok(v)
            })
            .await
            .unwrap()
    }

    /// 测 [`merge`]：
    /// - 成员改指向目标组，且只入一条 member outbox；
    /// - 已在目标组时再 merge 是纯 no-op，outbox 不增长；
    /// - 目标组已软删 → `InvalidInput`，成员留在原地。
    #[tokio::test]
    async fn merge_repoints_member_and_enqueues_outbox() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|conn| {
                let now = "2026-05-15T10:00:00Z";
                // 目标组 vscode；源 chrome.exe 在自己的单成员组里；
                // dead 是一个已软删的组，用来测「目标不存在」的拒绝。
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('vscode', 'vscode', 'NULL', ?1, NULL),
                           ('dead',   'dead',   'NULL',  ?1, ?1),
                           ('chrome.exe', 'chrome.exe', 'NULL', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('chrome.exe', 'chrome.exe', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();

        merge(&pool, "chrome.exe", "vscode").await.unwrap();

        assert_eq!(
            active_group_of(&pool, "chrome.exe").await.as_deref(),
            Some("vscode"),
            "merge 后成员应指向目标组"
        );
        // 裸 INSERT seed 不产生 outbox，所以这里的计数就是 merge 一次的净产出。
        let ob = outbox_summary(&pool).await;
        assert_eq!(
            ob.member_count, 1,
            "merge 应写 1 条 app_group_member outbox"
        );

        // 幂等：已经在目标组，再 merge 一次应是纯 no-op（不写 DB 也不写 outbox）
        let before = outbox_total(&pool).await;
        merge(&pool, "chrome.exe", "vscode").await.unwrap();
        assert_eq!(
            outbox_total(&pool).await,
            before,
            "同组重复 merge 不应产生新 outbox"
        );

        // 软删的目标组视同不存在：拒绝并且成员不动
        let err = merge(&pool, "chrome.exe", "dead").await.unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput(_)),
            "目标组软删应返回 InvalidInput，实际: {err:?}"
        );
        assert_eq!(
            active_group_of(&pool, "chrome.exe").await.as_deref(),
            Some("vscode"),
            "merge 失败后成员应停留在原组"
        );
    }

    /// 测 [`unmerge`] → [`restore_solo_group`]：
    /// - 单成员组已软删 → ON CONFLICT 复活，但**不**覆盖用户改过的 display_name
    /// - 复活时 category 跟随「拆出前所在组」的分类（用户拆开后分类不丢）
    /// - 单成员组从未存在 → 新建，display_name = process_name
    /// - 组和成员各入一条 outbox
    #[tokio::test]
    async fn unmerge_revives_solo_group_keeping_display_name_and_carrying_category() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|conn| {
                let now = "2026-05-15T10:00:00Z";
                // vscode 组（分类 code）里有 3 个成员；其中 Code.exe 的单成员组
                // 之前被软删过，且用户改过名（"我的编辑器"）、挂过旧分类 old-cat ——
                // 复活时名字必须保住，分类必须换成现组的 code。
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('vscode',   'vscode',     'code',    ?1, NULL),
                           ('Code.exe', '我的编辑器', 'old-cat', ?1, ?1)",
                    rusqlite::params![now],
                )
                .db()?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('Code',     'vscode', ?1, NULL),
                           ('Code.exe', 'vscode', ?1, NULL),
                           ('OtherApp', 'vscode', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();

        // 1) 复活软删组的路径
        unmerge(&pool, "Code.exe").await.unwrap();

        let (name, cat, deleted) = group_state(&pool, "Code.exe").await.unwrap();
        assert!(!deleted, "软删的单成员组应被复活");
        assert_eq!(name, "我的编辑器", "复活不应覆盖用户改过的 display_name");
        assert_eq!(
            cat.as_deref(),
            Some("code"),
            "复活组应携带拆出前所在组（vscode）的分类，而非残留的 old-cat"
        );
        assert_eq!(
            active_group_of(&pool, "Code.exe").await.as_deref(),
            Some("Code.exe"),
            "成员应回到自己的单成员组"
        );
        let ob = outbox_summary(&pool).await;
        assert_eq!(ob.group_count, 1, "复活组应写 1 条 app_group outbox");
        assert_eq!(ob.member_count, 1, "成员改指向应写 1 条 member outbox");

        // 2) 单成员组从未存在 → 全新 INSERT，display_name 用 process_name 本身
        unmerge(&pool, "OtherApp").await.unwrap();
        let (name, cat, deleted) = group_state(&pool, "OtherApp").await.unwrap();
        assert!(!deleted);
        assert_eq!(
            name, "OtherApp",
            "新建单成员组 display_name 应为 process_name"
        );
        assert_eq!(cat.as_deref(), Some("code"), "新建组同样携带原组分类");
        assert_eq!(
            active_group_of(&pool, "OtherApp").await.as_deref(),
            Some("OtherApp")
        );

        // 3) 边界：process_name 没有 active member 行 → 静默 no-op，不写 outbox
        let before = outbox_total(&pool).await;
        unmerge(&pool, "从未出现过的进程").await.unwrap();
        assert_eq!(
            outbox_total(&pool).await,
            before,
            "未知 process_name 的 unmerge 应是 no-op"
        );
    }

    /// 测 [`list_groups`] 组装逻辑：
    /// - 成员指向软删组（组 SELECT 过滤掉了）→ 该成员静默丢弃，不 panic 不串组
    /// - recent_secs 只算近 7 天窗口，且跨设备按 process_name 求和
    /// - 组间排序按「组内最大 recent_secs」降序（不是求和、不是首成员）
    /// - last_device_id 取全历史 ended_at 最大的那条活动的设备
    #[tokio::test]
    async fn list_groups_drops_orphan_members_and_sorts_by_max_recent_secs() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|conn| {
                let now = "2026-05-15T10:00:00Z";
                // 组插入顺序故意与期望输出相反（beta 在前），排错了就会暴露。
                // zombie 是软删组：ghost 成员指向它 → list_groups 的组列表里没有它。
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('beta',   'beta',   NULL, ?1, NULL),
                           ('alpha',  'alpha',  NULL, ?1, NULL),
                           ('empty',  'empty',  NULL, ?1, NULL),
                           ('zombie', 'zombie', NULL, ?1, ?1)",
                    rusqlite::params![now],
                )
                .db()?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('b1',    'beta',   ?1, NULL),
                           ('a1',    'alpha',  ?1, NULL),
                           ('a2',    'alpha',  ?1, NULL),
                           ('ghost', 'zombie', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                // 活动数据（duration 单位秒）：
                //   a1: 60(mac, 2h前结束) + 40(win, 1h前结束) → recent 100，last_device=win
                //   a2: 200(mac) + 300(win) → 跨设备求和 500 → alpha 组内最大值
                //   b1: 300(今天) + 9999(30 天前，窗口外必须排除；若被计入 beta 会错排第一)
                //   ghost: 77777 → 即使很大也该整个被丢弃
                // 注意 alpha 的最大值来自第二个成员 a2 —— 若实现错拿首成员排序会暴露。
                conn.execute_batch(
                    "INSERT INTO activities(started_at, ended_at, duration_secs, local_date,
                                            local_hour, process_name, category_id, device_id)
                     VALUES
                       (strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 hours'),
                        strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),
                        60, date('now','localtime'), 9, 'a1', 'work', 'mac'),
                       (strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),
                        strftime('%Y-%m-%dT%H:%M:%SZ','now','-1 hours'),
                        40, date('now','localtime'), 10, 'a1', 'work', 'win'),
                       (strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 hours'),
                        strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),
                        200, date('now','localtime'), 9, 'a2', 'work', 'mac'),
                       (strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),
                        strftime('%Y-%m-%dT%H:%M:%SZ','now','-1 hours'),
                        300, date('now','localtime'), 10, 'a2', 'work', 'win'),
                       (strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),
                        strftime('%Y-%m-%dT%H:%M:%SZ','now','-1 hours'),
                        300, date('now','localtime'), 10, 'b1', 'work', 'mac'),
                       (strftime('%Y-%m-%dT%H:%M:%SZ','now','-30 days'),
                        strftime('%Y-%m-%dT%H:%M:%SZ','now','-30 days'),
                        9999, date('now','localtime','-30 days'), 10, 'b1', 'work', 'old-box'),
                       (strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),
                        strftime('%Y-%m-%dT%H:%M:%SZ','now','-1 hours'),
                        77777, date('now','localtime'), 10, 'ghost', 'work', 'mac');",
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();

        let groups = list_groups(&pool).await.unwrap();

        // 排序：alpha(max=500) > beta(max=300) > empty(max=0)；zombie 不出现
        let order: Vec<&str> = groups.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(
            order,
            vec!["alpha", "beta", "empty"],
            "应按组内最大 recent_secs 降序，且软删组不出现"
        );

        // ghost 指向软删组 → 任何组里都不该出现（静默丢弃，而不是挂错组）
        assert!(
            groups
                .iter()
                .flat_map(|g| &g.members)
                .all(|m| m.process_name != "ghost"),
            "指向软删组的成员应被丢弃"
        );

        let alpha = &groups[0];
        let a1 = alpha
            .members
            .iter()
            .find(|m| m.process_name == "a1")
            .expect("alpha 应含成员 a1");
        let a2 = alpha
            .members
            .iter()
            .find(|m| m.process_name == "a2")
            .expect("alpha 应含成员 a2");
        assert_eq!(a1.recent_secs, 100, "a1 = 60 + 40");
        assert_eq!(a2.recent_secs, 500, "a2 跨设备求和 = 200 + 300");
        assert_eq!(
            a1.last_device_id.as_deref(),
            Some("win"),
            "last_device_id 应取 ended_at 最大那条活动的设备"
        );

        let beta = &groups[1];
        assert_eq!(beta.members.len(), 1);
        assert_eq!(
            beta.members[0].recent_secs, 300,
            "30 天前的 9999 秒在 7 天窗口外，必须被排除"
        );

        // 空组：members 为空但组仍在列表里（排最后）
        assert!(groups[2].members.is_empty(), "empty 组应无成员但仍返回");
    }

    /// 测 [`assign_category`] 的分类校验：分类不存在 → 返回 Err，
    /// 组的分类保持原值，outbox 不增长。
    #[tokio::test]
    async fn assign_category_rejects_unknown_category() {
        let pool = fresh_test_pool().await;
        seed_vscode_group(&pool).await;
        let outbox_before = outbox_total(&pool).await;

        let res = assign_category(&pool, "vscode", Some("no-such-cat".to_string())).await;
        assert!(
            matches!(res, Err(Error::InvalidInput(_))),
            "不存在的分类应让 assign_category 返回 InvalidInput，实际: {res:?}"
        );

        let group_cat: Option<String> = pool
            .0
            .call(|conn| {
                let c = conn
                    .query_row(
                        "SELECT category_id FROM app_groups WHERE id = 'vscode'",
                        [],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .db()?;
                Ok(c)
            })
            .await
            .unwrap();
        assert_eq!(
            group_cat.as_deref(),
            Some("code"),
            "回滚后组的 category_id 应保持原值"
        );
        assert_eq!(
            outbox_total(&pool).await,
            outbox_before,
            "回滚后 outbox 不应有新增行"
        );
    }

    /// 测 [`assign_category`] 的成功路径：设分类、清分类各改一次组行，
    /// 各入一条组 outbox。
    #[tokio::test]
    async fn assign_category_sets_and_clears_group_category() {
        let pool = fresh_test_pool().await;
        seed_vscode_group(&pool).await;

        assign_category(&pool, "vscode", Some("browse".to_string()))
            .await
            .unwrap();
        let (_, cat, deleted) = group_state(&pool, "vscode").await.unwrap();
        assert_eq!(cat.as_deref(), Some("browse"), "分类应改为 browse");
        assert!(!deleted);
        assert_eq!(
            outbox_summary(&pool).await.group_count,
            1,
            "设分类应入 1 条组 outbox"
        );

        assign_category(&pool, "vscode", None).await.unwrap();
        let (_, cat, _) = group_state(&pool, "vscode").await.unwrap();
        assert_eq!(cat, None, "None 应清掉分类");
        assert_eq!(
            outbox_summary(&pool).await.group_count,
            2,
            "清分类应再入 1 条组 outbox"
        );
    }

    /// 测 [`rename`]：只改显示名，分类不动，入 1 条组 outbox。
    #[tokio::test]
    async fn rename_updates_display_name_only() {
        let pool = fresh_test_pool().await;
        seed_vscode_group(&pool).await;

        rename(&pool, "vscode", "VS Code").await.unwrap();

        let (name, cat, deleted) = group_state(&pool, "vscode").await.unwrap();
        assert_eq!(name, "VS Code");
        assert_eq!(cat.as_deref(), Some("code"), "改名不该动分类");
        assert!(!deleted);
        assert_eq!(
            outbox_summary(&pool).await.group_count,
            1,
            "改名应入 1 条组 outbox"
        );
    }

    /// 测 [`unmerge`] 拒绝组名来源的那个成员：它要回的单成员组就是当前这个组，
    /// 没有落脚点。返回 `InvalidInput`，一行不动、outbox 不增长。
    #[tokio::test]
    async fn unmerge_rejects_the_member_the_group_is_named_after() {
        let pool = fresh_test_pool().await;
        seed_vscode_group(&pool).await;
        // seed 的组里没有和组同名的成员，补一个，它就是组名的来源
        pool.0
            .call(|conn| {
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('vscode', 'vscode', '2026-05-15T10:00:00Z', NULL)",
                    [],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
        let before = outbox_total(&pool).await;

        let err = unmerge(&pool, "vscode").await.unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput(_)),
            "组名来源的成员不能拆出，应返回 InvalidInput，实际: {err:?}"
        );

        for m in ["vscode", "Code", "Code.exe"] {
            assert_eq!(
                active_group_of(&pool, m).await.as_deref(),
                Some("vscode"),
                "拒绝后三个成员都应留在原组（尤其不能重演旧的「解散」行为）"
            );
        }
        assert_eq!(outbox_total(&pool).await, before, "拒绝不该写任何 outbox");
    }

    // ───────────── ensure_group:抓屏入口 ─────────────

    /// 测 [`ensure_group`]：抓屏每 tick 调一次的入口，覆盖它的四条路径。
    /// - 陌生进程名 → 建组 + 成员，各入一条 outbox；
    /// - 再调一次 → 快速出口，零写入；
    /// - 成员被软删后再调 → 复活（删过的应用再被抓到会重新出现，出处就是这里）；
    /// - 别名表命中 → 组 id 用 canonical 名，分类按内置规则填上。
    #[tokio::test]
    async fn ensure_group_creates_revives_and_is_idempotent() {
        let pool = fresh_test_pool().await;

        // 1) 陌生进程名（别名表与内置规则都没有）→ 新建
        ensure_group(&pool, "Zed").await.unwrap();
        assert_eq!(
            group_state(&pool, "Zed").await.unwrap(),
            ("Zed".to_string(), None, false),
            "新建组的显示名用进程名本身，无内置分类命中时分类为空"
        );
        assert_eq!(active_group_of(&pool, "Zed").await.as_deref(), Some("Zed"));
        let ob = outbox_summary(&pool).await;
        assert_eq!(ob.group_count, 1, "建组应入 1 条组 outbox");
        assert_eq!(ob.member_count, 1, "建成员应入 1 条成员 outbox");

        // 2) 幂等：已有活着的成员行 → 快速出口，不写库不入队
        let before = outbox_total(&pool).await;
        ensure_group(&pool, "Zed").await.unwrap();
        assert_eq!(
            outbox_total(&pool).await,
            before,
            "已存在时应直接返回，不产生新 outbox"
        );

        // 3) 删过之后再被抓到 → 复活
        purge_with_members(&pool, "Zed").await.unwrap();
        assert!(group_deleted(&pool, "Zed").await && member_deleted(&pool, "Zed").await);
        let before = outbox_total(&pool).await;
        ensure_group(&pool, "Zed").await.unwrap();
        assert!(!group_deleted(&pool, "Zed").await, "组应被复活");
        assert_eq!(
            active_group_of(&pool, "Zed").await.as_deref(),
            Some("Zed"),
            "成员行应被复活并指回自己的组"
        );
        assert_eq!(
            outbox_total(&pool).await - before,
            2,
            "复活应入组 + 成员各一条 outbox"
        );

        // 4) 别名表命中 → 组 id 是 canonical 名，不是进程名本身；
        //    分类按 canonical 名查内置规则
        ensure_group(&pool, "chrome.exe").await.unwrap();
        assert_eq!(
            active_group_of(&pool, "chrome.exe").await.as_deref(),
            Some("Google Chrome"),
            "别名应直接进 canonical 组"
        );
        let (name, cat, _) = group_state(&pool, "Google Chrome").await.unwrap();
        assert_eq!(name, "Google Chrome", "组的显示名用 canonical 名");
        assert_eq!(
            cat.as_deref(),
            Some("browse"),
            "内置分类按 canonical 名查，别名也能拿到"
        );
    }

    // ───────────── purge_with_data:真删数据 ─────────────

    /// 造一个应用的完整痕迹:主库活动 + 记忆库 OCR 会话/帧 + 一张截图文件。
    async fn seed_app_traces(
        pool: &DbPool,
        mem: &crate::memory::MemoryDb,
        root: &std::path::Path,
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

    async fn fts_hits(mem: &crate::memory::MemoryDb, needle: &str) -> i64 {
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
    async fn purge_with_data_wipes_all_three_places_and_spares_others() {
        let pool = fresh_test_pool().await;
        let mem = crate::memory::MemoryDb::open_in_memory().await.unwrap();
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

        purge_with_data(&pool, &mem, &dir, "vscode").await.unwrap();

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

        // 组本身仍按既有语义软删
        assert!(group_deleted(&pool, "vscode").await);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 为什么测:实测当前一条 activity 独占一张截图、去重映射也从不跨应用,
    /// 所以正常路径下不会误删别人的图。但万一将来去重改成全局图像哈希,
    /// 共享就可能出现——这行兜底检查必须挡住,否则会静默打穿别的应用的证据链
    /// (搜索结果点开看不到原图)。
    #[tokio::test]
    async fn purge_with_data_keeps_screenshot_still_referenced_by_others() {
        let pool = fresh_test_pool().await;
        let mem = crate::memory::MemoryDb::open_in_memory().await.unwrap();
        let dir = std::env::temp_dir().join(format!("hs-purge-shared-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        seed_vscode_group(&pool).await;
        // 人为让两个应用引用同一张图(现实中不会发生,见函数文档)
        seed_app_traces(&pool, &mem, &dir, "Code", "shared.png", "甲的文字").await;
        seed_app_traces(&pool, &mem, &dir, "Chrome", "shared.png", "乙的文字").await;

        purge_with_data(&pool, &mem, &dir, "vscode").await.unwrap();

        assert_eq!(count_activities(&pool, "Code").await, 0);
        assert!(
            dir.join("shared.png").exists(),
            "还被别的应用引用的截图不能删——否则对方的证据卡点开是空的"
        );
        assert_eq!(count_activities(&pool, "Chrome").await, 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
