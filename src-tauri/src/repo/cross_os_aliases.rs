//! App aliases: one app can show up under several process names (its name on
//! another platform, helper processes), and each name gets its own group by
//! default, so stats show several apps. This module keeps a process name →
//! canonical name table so every name of an app lands in the same group.
//!
//! Data lives in `src-tauri/data/cross_os_app_aliases.json`, hand-maintained
//! and compiled into the binary.
//!
//! Two entry points:
//!   - `app_groups::ensure_group`: checks the table when a process name first
//!     appears and, on a hit, files it straight into the canonical group;
//!   - `pair_existing`: runs once at startup and moves members that existed
//!     before the table did.

use rusqlite::OptionalExtension;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::error::Result;
use crate::repo::outbox::{enqueue, OutboxEntity, OutboxOp};
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};

const ALIASES_JSON: &str = include_str!("../../data/cross_os_app_aliases.json");

#[derive(Deserialize)]
struct RawAliases {
    aliases: Vec<RawAlias>,
}

#[derive(Deserialize)]
struct RawAlias {
    canonical: String,
    names: Vec<String>,
}

/// Hand-maintained table of common apps' process names, compiled into the
/// binary: every name an app runs under (other platforms, helper processes)
/// maps to one canonical name. Checked when a process name first shows up;
/// on a hit it joins the canonical group instead of getting its own.
/// Key: process name lowercased. Value: canonical name, used as the group id.
fn aliases() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        let parsed: RawAliases = match serde_json::from_str(ALIASES_JSON) {
            Ok(p) => p,
            Err(e) => {
                log::error!("cross_os_app_aliases.json failed to parse (skipped): {e}");
                return HashMap::new();
            }
        };
        let mut map = HashMap::new();
        for entry in parsed.aliases {
            for name in entry.names {
                map.insert(name.to_lowercase(), entry.canonical.clone());
            }
        }
        map
    })
}

/// Look up the canonical name for a process name; `None` if the table has no
/// entry. Case-insensitive (`"chrome.exe"` and `"Chrome.exe"` are the same).
pub fn lookup_canonical(process_name: &str) -> Option<&'static str> {
    aliases()
        .get(&process_name.to_lowercase())
        .map(|s| s.as_str())
}

/// Runs once at startup. The alias table only applies when a process name first
/// appears, so members that existed before the table (or before their entry was
/// added) each got a solo group; this moves them into their canonical group.
/// Returns how many were moved.
///
/// Only touches members still in their own solo group; anything the user
/// dragged into another group is left alone. Members already in the canonical
/// group are skipped, so re-running costs nothing. One failure is logged and
/// the rest of the batch continues.
pub async fn pair_existing(pool: &DbPool) -> Result<u64> {
    let members: Vec<(String, String)> = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT process_name, group_id FROM app_group_members
                     WHERE deleted_at IS NULL",
                )
                .db()?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .db()?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.db()?);
            }
            Ok(out)
        })
        .await?;

    let mut merged = 0u64;
    for (process_name, group_id) in members {
        let Some(canonical) = lookup_canonical(&process_name) else {
            continue;
        };
        if group_id == canonical {
            // Already in the canonical group, however it got there. Without this check,
            // a process whose name is its own canonical would be re-paired on every start.
            continue;
        }
        if group_id != process_name {
            // The user moved it into a group of their own; leave it alone.
            continue;
        }
        if let Err(e) = pair_one(pool, &process_name, canonical).await {
            // One failure must not stop the batch; log it and move on.
            log::warn!("cross_os pair 失败 process_name={process_name} canonical={canonical}: {e}");
            continue;
        }
        merged += 1;
    }
    Ok(merged)
}

/// Moves one process from its solo group into its canonical group; called per
/// member by `pair_existing`.
///
/// One transaction, three writes: create the canonical group or revive it if
/// tombstoned (a live one is left alone, so the user's rename and category
/// survive); point the member at it; tombstone the old solo group if it is now
/// empty. All three are queued for sync.
///
/// Re-checks inside the transaction that the member is still in its solo group
/// and does nothing otherwise (already paired, moved by the user, or deleted),
/// so a repeated call or a change made in between is never overwritten.
async fn pair_one(pool: &DbPool, process_name: &str, canonical: &str) -> Result<()> {
    let pn = process_name.to_string();
    let canon = canonical.to_string();
    let updated_at = utc_now_rfc3339();
    // Category for the canonical group if it gets created here; an existing
    // group keeps its own.
    let builtin_cat = super::builtin_categories::match_builtin_category(canonical);

    pool.0
        .call(move |conn| {
            // Step 0: re-check inside the transaction that the member is still in its
            // solo group; capture or sync may have moved it since `pair_existing` read it.
            let tx = conn.transaction().db()?;
            let current_gid: Option<String> = tx
                .query_row(
                    "SELECT group_id FROM app_group_members
                     WHERE process_name = ?1 AND deleted_at IS NULL",
                    rusqlite::params![pn],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .db()?;
            if current_gid.as_deref() != Some(pn.as_str()) {
                // No longer in its solo group (already paired, moved by the user,
                // or tombstoned): nothing to pair.
                return Ok(());
            }

            // Step 1: create the canonical group, or revive it if tombstoned; a live one
            // is left alone so the user's rename and category survive.
            tx.execute(
                "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, ?, NULL)
                 ON CONFLICT(id) DO UPDATE SET
                   deleted_at = NULL,
                   updated_at = excluded.updated_at
                 WHERE app_groups.deleted_at IS NOT NULL",
                rusqlite::params![canon, canon, builtin_cat, updated_at],
            )
            .db()?;
            enqueue(
                &tx,
                OutboxOp::Upsert,
                OutboxEntity::AppGroup,
                &canon,
                &serde_json::json!({ "groupId": canon }).to_string(),
            )
            .db()?;

            // Step 2: point the member at the canonical group.
            tx.execute(
                "UPDATE app_group_members SET group_id = ?2, updated_at = ?3, deleted_at = NULL
                 WHERE process_name = ?1",
                rusqlite::params![pn, canon, updated_at],
            )
            .db()?;
            enqueue(
                &tx,
                OutboxOp::Upsert,
                OutboxEntity::AppGroupMember,
                &pn,
                &serde_json::json!({ "processName": pn }).to_string(),
            )
            .db()?;

            // Step 3: tombstone the member's old solo group if nothing else is left in
            // it; if other members remain (the user dragged them in), it stays.
            let has_other_members: bool = tx
                .query_row(
                    "SELECT 1 FROM app_group_members
                     WHERE group_id = ?1 AND deleted_at IS NULL",
                    rusqlite::params![pn],
                    |_| Ok(true),
                )
                .optional()
                .db()?
                .unwrap_or(false);
            if !has_other_members {
                let n = tx
                    .execute(
                        "UPDATE app_groups SET deleted_at = ?1, updated_at = ?1
                         WHERE id = ?2 AND deleted_at IS NULL",
                        rusqlite::params![updated_at, pn],
                    )
                    .db()?;
                if n > 0 {
                    enqueue(
                        &tx,
                        OutboxOp::Upsert,
                        OutboxEntity::AppGroup,
                        &pn,
                        &serde_json::json!({ "groupId": pn }).to_string(),
                    )
                    .db()?;
                }
            }
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::fresh_test_pool;
    use crate::storage::SqliteResultExt;

    /// 测 [`pair_existing`]：把两个跨 OS 别名（默认 solo 组）合并到 canonical 组。
    /// 别名表已嵌入二进制：`Code` 和 `Code.exe` 都映射到 `"Visual Studio Code"`。
    #[tokio::test]
    async fn pair_existing_merges_aliases_into_canonical_group() {
        let pool = fresh_test_pool().await;
        seed_solo_groups(&pool, &[("Code", "Code"), ("Code.exe", "Code.exe")]).await;
        // sanity 检查别名表确实命中
        assert_eq!(lookup_canonical("Code"), Some("Visual Studio Code"));
        assert_eq!(lookup_canonical("Code.exe"), Some("Visual Studio Code"));

        let merged = pair_existing(&pool).await.unwrap();
        assert_eq!(merged, 2, "两条别名都应被合并到 canonical");

        let canon_id = group_id_of_member(&pool, "Code").await.unwrap();
        assert_eq!(canon_id, "Visual Studio Code");
        let canon_id2 = group_id_of_member(&pool, "Code.exe").await.unwrap();
        assert_eq!(canon_id2, "Visual Studio Code");

        // canonical 组存在且 active
        assert!(group_active(&pool, "Visual Studio Code").await);
        // 原 solo 组都被软删
        assert!(!group_active(&pool, "Code").await);
        assert!(!group_active(&pool, "Code.exe").await);

        // 幂等：再跑一次发现 member.group_id 已是 canonical → 全部跳过
        let merged2 = pair_existing(&pool).await.unwrap();
        assert_eq!(merged2, 0);
    }

    /// 用户改过组结构（拖到自定义组）的成员不该被强拉回 canonical。
    #[tokio::test]
    async fn pair_existing_respects_user_custom_grouping() {
        let pool = fresh_test_pool().await;
        // "Code" 在自定义组 "my-tools" 里（不等于 process_name 也不等于 canonical）
        seed_solo_groups(&pool, &[("my-tools", "my-tools")]).await;
        seed_member(&pool, "Code", "my-tools").await;

        let merged = pair_existing(&pool).await.unwrap();
        assert_eq!(merged, 0, "用户手动配过的成员不该被强拉到 canonical");

        let still_custom = group_id_of_member(&pool, "Code").await.unwrap();
        assert_eq!(still_custom, "my-tools");
    }

    async fn seed_solo_groups(pool: &DbPool, groups: &[(&str, &str)]) {
        let groups: Vec<(String, String)> = groups
            .iter()
            .map(|(id, pn)| (id.to_string(), pn.to_string()))
            .collect();
        pool.0
            .call(move |conn| {
                let now = "2026-05-15T10:00:00Z";
                for (id, _pn) in &groups {
                    conn.execute(
                        "INSERT OR IGNORE INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                         VALUES(?1, ?1, NULL, ?2, NULL)",
                        rusqlite::params![id, now],
                    )
                    .db()?;
                }
                // solo 组：每条 (process_name, group_id=process_name) 一行 member
                for (id, pn) in &groups {
                    if id == pn {
                        conn.execute(
                            "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                             VALUES(?1, ?1, ?2, NULL)",
                            rusqlite::params![pn, now],
                        )
                        .db()?;
                    }
                }
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn seed_member(pool: &DbPool, process_name: &str, group_id: &str) {
        let pn = process_name.to_string();
        let gid = group_id.to_string();
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES(?1, ?2, '2026-05-15T10:00:00Z', NULL)",
                    rusqlite::params![pn, gid],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn group_id_of_member(pool: &DbPool, process_name: &str) -> Option<String> {
        let pn = process_name.to_string();
        pool.0
            .call(move |conn| {
                let r: Option<String> = conn
                    .query_row(
                        "SELECT group_id FROM app_group_members
                         WHERE process_name = ?1 AND deleted_at IS NULL",
                        rusqlite::params![pn],
                        |r| r.get(0),
                    )
                    .optional()
                    .db()?;
                Ok(r)
            })
            .await
            .unwrap()
    }

    async fn group_active(pool: &DbPool, id: &str) -> bool {
        let id = id.to_string();
        pool.0
            .call(move |conn| {
                let r: Option<i64> = conn
                    .query_row(
                        "SELECT 1 FROM app_groups WHERE id = ?1 AND deleted_at IS NULL",
                        rusqlite::params![id],
                        |r| r.get(0),
                    )
                    .optional()
                    .db()?;
                Ok(r.is_some())
            })
            .await
            .unwrap()
    }
}
