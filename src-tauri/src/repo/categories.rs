//! Data access for the `categories` table — the user-visible buckets ("Work",
//! "Browsing", …) that app groups are assigned to: create, update, delete,
//! reorder, and list each category with the process names under it.
//!
//! Every write also enqueues a sync-outbox row so the change reaches the user's
//! other devices (merged there by last-write-wins). Deleting a category sends
//! its groups back to unclassified; built-in categories and `other` cannot be
//! deleted, since unclassified time needs somewhere to land.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::{Error, Result};
use crate::repo::outbox::{enqueue, OutboxEntity, OutboxOp};
use crate::repo::sql::{FROM_ACTIVITY_GROUP_CATEGORY, FROM_MEMBER_GROUP};
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};

/// A `categories` row plus the process names currently classified under it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    /// Category ID: 'work', 'play', etc. for built-in categories;
    /// UUID for user-created ones.
    pub id: String,
    /// Display name of the category.
    pub name: String,
    /// Hex color `#rrggbb`
    pub color: String,
    /// Icon ID (used by the frontend to map to lucide-react icons)
    pub icon: String,
    /// System category: the row belongs to the app, not the user — it cannot be
    /// deleted, dragged, or filed under a super-category, and the UI draws it in
    /// its own block. `hidden` is the only one; the seeded defaults (`code`,
    /// `browse`, …) are ordinary user categories.
    pub builtin: bool,
    /// The list of process names currently classified under this category
    ///  (sorted alphabetically)
    pub apps: Vec<String>,
    /// Super-category ID (NULL = not assigned to a parent category; the UI
    /// renders it in the "Ungrouped" row). Introduced in v28.
    pub super_category_id: Option<String>,
}

/// Fields sent from the frontend when creating a new category
/// (id excluded; the backend generates a UUID).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryInput {
    pub name: String,
    pub color: String,
    pub icon: String,
}

/// Patch used when updating a category: each field `None` means "leave unchanged".
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryPatch {
    pub name: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
}

/// A row representing an unclassified app — used for the "Unclassified" card
/// on the "Categories" page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnclassifiedApp {
    pub process_name: String,
    /// Total minutes used in the last N days
    pub minutes: u32,
    /// RFC3339 timestamp of the last occurrence
    pub last_seen_at: String,
}

// Fan-in helper: the argument count mirrors the table's column count.
// Wrapping them in a struct would only make every caller build one first — pure noise.
#[allow(clippy::too_many_arguments)]
fn category_payload(
    id: &str,
    name: &str,  // Display name of the category
    color: &str, // Hex color `#rrggbb`
    icon: &str,  // Icon ID (used by the frontend to map to lucide-react icons)
    builtin: bool,
    sort_order: i64,          // Display order of the category
    updated_at: &str,         // RFC3339 timestamp of the last update
    deleted_at: Option<&str>, // RFC3339 deletion tombstone
) -> String {
    serde_json::json!({
        "id": id,
        "name": name,
        "color": color,
        "icon": icon,
        "builtin": builtin,
        "sortOrder": sort_order,
        "updatedAt": updated_at,
        "deletedAt": deleted_at,
    })
    .to_string()
}

/// Lists all active categories ordered by `sort_order`, each carrying the
/// process names currently classified under it.
pub async fn list(pool: &DbPool) -> Result<Vec<Category>> {
    let cats = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT id, name, color, icon, builtin, super_category_id FROM categories
                     WHERE deleted_at IS NULL
                     ORDER BY sort_order ASC, id ASC",
                )
                .db()?;
            let cat_rows = stmt
                .query_map([], |r| {
                    Ok(Category {
                        id: r.get::<_, String>(0)?,
                        name: r.get::<_, String>(1)?,
                        color: r.get::<_, String>(2)?,
                        icon: r.get::<_, String>(3)?,
                        builtin: r.get::<_, i64>(4)? != 0,
                        super_category_id: r.get::<_, Option<String>>(5)?,
                        apps: Vec::new(),
                    })
                })
                .db()?;
            let mut cats: Vec<Category> = Vec::new();
            for r in cat_rows {
                cats.push(r.db()?);
            }

            // Fills in the `apps` lists left empty above: each process name is attached to
            // the category its group belongs to.
            let sql = format!(
                "SELECT gm.process_name, g.category_id
                 {FROM_MEMBER_GROUP}
                 WHERE gm.deleted_at IS NULL
                   AND g.deleted_at IS NULL
                   AND g.category_id IS NOT NULL
                 ORDER BY gm.process_name"
            );
            let mut stmt2 = conn.prepare_cached(&sql).db()?;
            let map_rows = stmt2
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .db()?;
            let index: HashMap<String, usize> = cats
                .iter()
                .enumerate()
                .map(|(i, c)| (c.id.clone(), i))
                .collect();
            for r in map_rows {
                let (process, cat_id) = r.db()?;
                if let Some(i) = index.get(&cat_id) {
                    cats[*i].apps.push(process);
                }
            }
            Ok(cats)
        })
        .await?;

    Ok(cats)
}

/// Creates a new category: generates UUID, appends to the end,
/// and enqueues to outbox for sync.
pub async fn create(pool: &DbPool, input: CategoryInput) -> Result<Category> {
    let id = uuid::Uuid::new_v4().to_string();
    let name = input.name.trim().to_string(); // Trim "   " → "" to reject empty names
    let color = input.color.trim().to_string();
    let icon = if input.icon.trim().to_string().is_empty() {
        "Tag".to_string()
    } else {
        input.icon.trim().to_string()
    };
    if name.is_empty() {
        return Err(Error::InvalidInput("category name must not be empty"));
    }
    if color.is_empty() {
        return Err(Error::InvalidInput("color must not be empty"));
    }
    let n = name.clone();
    let c = color.clone();
    let i = icon.clone();
    let updated = utc_now_rfc3339();

    let cat = pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            // New category will be placed at the end: sort_order = max(active sort_order) + 1
            let next_sort: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM categories WHERE deleted_at IS NULL",
                    [],
                    |r| r.get(0),
                )
                .db()?;
            tx.execute(
                "INSERT INTO categories(id, name, color, icon, builtin, sort_order, updated_at)
                 VALUES(?, ?, ?, ?, 0, ?, ?)",
                rusqlite::params![id, n, c, i, next_sort, &updated],
            )
            .db()?;

            let payload = category_payload(&id, &n, &c, &i, false, next_sort, &updated, None);
            enqueue(&tx, OutboxOp::Upsert, OutboxEntity::Category, &id, &payload)
                .db()?;
            tx.commit().db()?;
            Ok(Category {
                id,
                name: n,
                color: c,
                icon: i,
                builtin: false,
                apps: Vec::new(),
                super_category_id: None,
            })
        })
        .await?;
    Ok(cat)
}

/// Update a category's name, color, and icon.
/// Fields with None or empty strings in the patch remain unchanged.
/// Built-in categories can also be updated (changing only appearance;
/// id and builtin flag are not modified).
pub async fn update(pool: &DbPool, id: &str, patch: CategoryPatch) -> Result<()> {
    let id = id.to_string();
    let updated_at = utc_now_rfc3339();
    pool.0
        .call(move |conn| {
            // 读出当前行做基线
            let tx = conn.transaction().db()?;
            let row: Option<(String, String, String, i64, i64)> = tx
                .query_row(
                    "SELECT name, color, icon, builtin, sort_order FROM categories
                     WHERE id = ? AND deleted_at IS NULL",
                    rusqlite::params![id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .optional()?;
            let Some((cur_name, cur_color, cur_icon, builtin_i, cur_sort)) = row else {
                return Ok(());
            };

            let new_name = patch
                .name
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or(cur_name);
            let new_color = patch
                .color
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or(cur_color);
            let new_icon = patch
                .icon
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or(cur_icon);
            let n = tx
                .execute(
                    "UPDATE categories SET name = ?1, color = ?2, icon = ?3, updated_at = ?4
                        WHERE id = ?5 AND (name IS NOT ?1 OR color IS NOT ?2 OR icon IS NOT ?3)",
                    rusqlite::params![new_name, new_color, new_icon, updated_at, id],
                )
                .db()?;
            if n > 0 {
                let payload = category_payload(
                    &id,
                    &new_name,
                    &new_color,
                    &new_icon,
                    builtin_i != 0,
                    cur_sort,
                    &updated_at,
                    None,
                );
                enqueue(&tx, OutboxOp::Upsert, OutboxEntity::Category, &id, &payload).db()?;
            }
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Reorder categories by dragging: set each id's sort_order to its position in the ordered_ids list.
/// Only enqueue outbox for rows where sort_order actually changed
/// (idempotent: dragging in place doesn't re-push).
/// `updated_at` is also bumped to ensure cross-device LWW receives the new order.
pub async fn reorder(pool: &DbPool, ordered_ids: Vec<String>) -> Result<()> {
    let updated_at = utc_now_rfc3339();
    pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            for (idx, id) in ordered_ids.iter().enumerate() {
                let new_sort = idx as i64;
                let row: Option<(String, String, String, i64, i64)> = tx
                    .query_row(
                        "SELECT name, color, icon, builtin, sort_order FROM categories
                         WHERE id = ?1 AND deleted_at IS NULL",
                        rusqlite::params![id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                    )
                    .optional()?;
                let Some((name, color, icon, builtin_i, cur_sort)) = row else {
                    continue;
                };
                if cur_sort == new_sort {
                    continue; // Don't need to update if the sort order hasn't changed
                }
                tx.execute(
                    "UPDATE categories SET sort_order = ?1, updated_at = ?2
                     WHERE id = ?3 AND deleted_at IS NULL",
                    rusqlite::params![new_sort, updated_at, id],
                )
                .db()?;
                let payload = category_payload(
                    id,
                    &name,
                    &color,
                    &icon,
                    builtin_i != 0,
                    new_sort,
                    &updated_at,
                    None,
                );
                enqueue(&tx, OutboxOp::Upsert, OutboxEntity::Category, id, &payload).db()?;
            }
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Soft-deletes a category: the row stays and gets a `deleted_at` tombstone,
/// so the deletion can be synced to other devices.
///
/// Exactly two categories are refused with `Error::InvalidInput`:
///   - `hidden` — the only row carrying the `builtin` flag (see [`Category`]);
///   - `other` — flag is 0, refused by id instead: reports bucket unclassified
///     time into it, and the SQL hardcodes the string.
///
/// Every other seeded default (`code`, `browse`, …) is deletable.
///
/// Afterwards [`cascade_category_deletion`] nulls `app_groups.category_id` on
/// every group that pointed at it, so nothing is left referencing a category
/// that no longer exists.
pub async fn delete(pool: &DbPool, id: &str) -> Result<()> {
    let id = id.to_string();
    let updated_at = utc_now_rfc3339();
    // `Ok(Err(msg))` = business rejection, turned into `Error::InvalidInput` below.
    // Real db failures still go through `?`, so `InvalidInput` never ends up
    // wrapped in `tokio_rusqlite::Error::Other`.
    let outcome: std::result::Result<(), &'static str> = pool
        .0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            let row: Option<(String, String, String, i64, i64)> = tx
                .query_row(
                    "SELECT name, color, icon, builtin, sort_order FROM categories
                     WHERE id = ? AND deleted_at IS NULL",
                    rusqlite::params![id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .optional()?;
            let Some((name, color, icon, builtin_i, sort_order)) = row else {
                return Ok(Ok(()));
            };
            if builtin_i != 0 {
                return Ok(Err("built-in categories cannot be deleted"));
            }
            // `other` has builtin = 0, so the guard above misses it. Reports SQL
            // hardcodes `COALESCE(c.id, 'other')` — deleting the row leaves gaps
            // in the charts.
            if id == "other" {
                return Ok(Err(
                    "'other' is where unclassified processes land and cannot be deleted",
                ));
            }

            tx.execute(
                "UPDATE categories SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2",
                rusqlite::params![updated_at, id],
            )
            .db()?;

            let cat_payload = category_payload(
                &id,
                &name,
                &color,
                &icon,
                builtin_i != 0,
                sort_order,
                &updated_at,
                Some(&updated_at),
            );
            enqueue(
                &tx,
                OutboxOp::Upsert,
                OutboxEntity::Category,
                &id,
                &cat_payload,
            )
            .db()?;

            cascade_category_deletion(&tx, &id, &updated_at)?;

            tx.commit().db()?;
            Ok(Ok(()))
        })
        .await?;
    outcome.map_err(Error::InvalidInput)
}

/// Clears every reference to a just-deleted category by nulling
/// `app_groups.category_id`. Runs for both local deletes and ones that arrive
/// over sync.
///
/// Idempotent: every UPDATE is guarded, so a repeat run touches zero rows and
/// enqueues nothing.
///
/// Expects to be called inside a transaction: the tombstone that triggered it
/// and every group this clears have to land together. The parameter type does
/// not enforce that yet.
pub fn cascade_category_deletion(
    conn: &Connection,
    category_id: &str,
    now: &str,
) -> rusqlite::Result<()> {
    // Collect affected group ids, null out category_id, enqueue each.
    let mut stmt = conn.prepare(
        "SELECT id FROM app_groups
         WHERE category_id = ?1 AND deleted_at IS NULL",
    )?;
    let affected_groups: Vec<String> = stmt
        .query_map(rusqlite::params![category_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    for g in &affected_groups {
        conn.execute(
            "UPDATE app_groups SET category_id = NULL, updated_at = ?1
             WHERE id = ?2 AND category_id IS NOT NULL",
            rusqlite::params![now, g],
        )?;
        let payload = serde_json::json!({ "groupId": g }).to_string();
        enqueue(conn, OutboxOp::Upsert, OutboxEntity::AppGroup, g, &payload)?;
    }

    Ok(())
}

/// Assigns a category to an app. The category lands on the app's *group*, not on
/// the process name itself, so every name in that group — the same app on other
/// platforms — is classified along with it.
pub async fn assign_app(pool: &DbPool, process_name: &str, category_id: &str) -> Result<()> {
    let p = process_name.trim().to_string();
    let c = category_id.trim().to_string();
    if p.is_empty() {
        return Err(Error::InvalidInput("app name must not be empty"));
    }
    if c.is_empty() {
        return Err(Error::InvalidInput("category id must not be empty"));
    }
    crate::repo::app_groups::assign_category_for_process(pool, &p, Some(c)).await
}

/// Unassigns a category from an app. Goes through the app_groups channel: sets the
/// group's category_id to NULL.
pub async fn unassign_app(pool: &DbPool, process_name: &str) -> Result<()> {
    crate::repo::app_groups::assign_category_for_process(pool, process_name, None).await
}

/// Lists process names active in the last `days_back` days that belong to no
/// live category.
pub async fn list_unclassified(pool: &DbPool, days_back: u32) -> Result<Vec<UnclassifiedApp>> {
    let days = days_back.max(1) as i64;
    let rows = pool
        .0
        .call(move |conn| {
            // Never read app_categories here: it is no longer maintained locally
            // (push derives it), so it says nothing about current assignments.
            let sql = format!(
                "SELECT a.process_name,
                        CAST(SUM(a.duration_secs) / 60 AS INTEGER) AS minutes,
                        MAX(a.ended_at) AS last_seen_at
                 {FROM_ACTIVITY_GROUP_CATEGORY}
                 WHERE c.id IS NULL
                   AND a.local_date >= date('now','localtime', '-' || ?1 || ' days')
                   AND a.process_name <> 'Unknown'
                 GROUP BY a.process_name
                 ORDER BY minutes DESC"
            );
            let mut stmt = conn.prepare_cached(&sql).db()?;
            let it = stmt
                .query_map(rusqlite::params![days], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .db()?;
            let mut out = Vec::new();
            for r in it {
                out.push(r.db()?);
            }
            Ok(out)
        })
        .await?;

    Ok(rows
        .into_iter()
        .map(|(process_name, minutes, last_seen_at)| UnclassifiedApp {
            process_name,
            minutes: minutes.max(0) as u32,
            last_seen_at,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::fresh_test_pool;

    /// 钉死 bug：当 app_group_members + app_groups.category_id 有数据但 app_categories
    /// 镜像表为空时（典型 backfill 漏镜像 / sync 顺序错位），categories::list 仍应
    /// 返回该 process_name —— 因为现在直接读真实源而不是镜像表。
    ///
    /// 旧实现（读 app_categories）下：apps 列表会是空，UI 显示"暂无绑定应用"。
    /// 新实现（JOIN app_group_members + app_groups）：直接拿到 process_name。
    #[tokio::test]
    async fn list_returns_app_when_only_app_groups_has_category_no_app_categories_mirror() {
        let pool = fresh_test_pool().await;

        // 模拟 capture 写入：建组（带 category）+ 加成员；**故意不写 app_categories 镜像**。
        pool.0
            .call(|conn| {
                let now = "2026-05-17T10:00:00Z";
                // 组 "Visual Studio Code" 归类到 builtin "code"（categories 表已 seed）
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('Visual Studio Code', 'Visual Studio Code', 'code', ?1, NULL)",
                    rusqlite::params![now],
                )?;
                // mac 进程名 "Code" 归到这个组
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('Code', 'Visual Studio Code', ?1, NULL)",
                    rusqlite::params![now],
                )?;
                // **故意不**写 app_categories —— 模拟镜像 lag
                Ok(())
            })
            .await
            .unwrap();

        let cats = list(&pool).await.unwrap();
        let code = cats
            .iter()
            .find(|c| c.id == "code")
            .expect("'code' 内置分类应该存在");
        assert!(
            code.apps.iter().any(|p| p == "Code"),
            "镜像表为空时也应该能看到 Code，实际 apps={:?}",
            code.apps,
        );
    }

    /// 反例：当 app_groups.category_id IS NULL（未分类）时，**不**应出现在任何分类的 apps 里。
    #[tokio::test]
    async fn list_excludes_app_when_group_has_no_category() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|conn| {
                let now = "2026-05-17T10:00:00Z";
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('SomeApp', 'SomeApp', NULL, ?1, NULL)",
                    rusqlite::params![now],
                )?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('SomeApp', 'SomeApp', ?1, NULL)",
                    rusqlite::params![now],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let cats = list(&pool).await.unwrap();
        for c in &cats {
            assert!(
                !c.apps.iter().any(|p| p == "SomeApp"),
                "未分类组的成员不应出现在任何分类下，但 {} 包含: {:?}",
                c.id,
                c.apps,
            );
        }
    }

    /// 反例：软删除的 group / member 不应被列出。
    #[tokio::test]
    async fn list_excludes_soft_deleted_groups_and_members() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|conn| {
                let now = "2026-05-17T10:00:00Z";
                // 软删的 group：成员还在，但 group 不算 active
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('DeletedGroup', 'DeletedGroup', 'code', ?1, ?1)",
                    rusqlite::params![now],
                )?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('AppInDeletedGroup', 'DeletedGroup', ?1, NULL)",
                    rusqlite::params![now],
                )?;
                // active group 但软删的 member
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('LiveGroup', 'LiveGroup', 'code', ?1, NULL)",
                    rusqlite::params![now],
                )?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('DeletedMember', 'LiveGroup', ?1, ?1)",
                    rusqlite::params![now],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let cats = list(&pool).await.unwrap();
        let code = cats.iter().find(|c| c.id == "code").unwrap();
        assert!(!code.apps.iter().any(|p| p == "AppInDeletedGroup"));
        assert!(!code.apps.iter().any(|p| p == "DeletedMember"));
    }

    // ---------- 共享小工具 ----------

    fn cat_input(name: &str, color: &str, icon: &str) -> CategoryInput {
        CategoryInput {
            name: name.into(),
            color: color.into(),
            icon: icon.into(),
        }
    }

    /// 绕过 `deleted_at IS NULL` 过滤直接读原始行（测软删语义必须能看到 tombstone）。
    /// 返回 (name, color, icon, builtin, sort_order, deleted_at, updated_at)。
    #[allow(clippy::type_complexity)]
    async fn raw_cat(
        pool: &DbPool,
        id: &str,
    ) -> Option<(String, String, String, i64, i64, Option<String>, String)> {
        let id = id.to_string();
        pool.0
            .call(move |conn| {
                use rusqlite::OptionalExtension;
                let row = conn
                    .query_row(
                        "SELECT name, color, icon, builtin, sort_order, deleted_at, updated_at
                           FROM categories WHERE id = ?1",
                        rusqlite::params![id],
                        |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, String>(2)?,
                                r.get::<_, i64>(3)?,
                                r.get::<_, i64>(4)?,
                                r.get::<_, Option<String>>(5)?,
                                r.get::<_, String>(6)?,
                            ))
                        },
                    )
                    .optional()?;
                Ok(row)
            })
            .await
            .unwrap()
    }

    async fn find_cat(pool: &DbPool, id: &str) -> Option<Category> {
        list(pool).await.unwrap().into_iter().find(|c| c.id == id)
    }

    async fn list_ids(pool: &DbPool) -> Vec<String> {
        list(pool)
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect()
    }

    async fn outbox_count(pool: &DbPool, entity: &str) -> i64 {
        let e = entity.to_string();
        pool.0
            .call(move |conn| {
                let n = conn.query_row(
                    "SELECT COUNT(*) FROM sync_outbox WHERE entity = ?1",
                    rusqlite::params![e],
                    |r| r.get::<_, i64>(0),
                )?;
                Ok(n)
            })
            .await
            .unwrap()
    }

    async fn outbox_total(pool: &DbPool) -> i64 {
        pool.0
            .call(|conn| {
                let n = conn.query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| {
                    r.get::<_, i64>(0)
                })?;
                Ok(n)
            })
            .await
            .unwrap()
    }

    // ---------- create ----------

    /// 为什么测：前端表单可能提交全空格（用户误敲空格直接确认）。trim 后必须拒绝，
    /// 否则列表里出现"看不见"的分类，既点不中也删不掉。icon 留空则要兜底成
    /// 非空默认值——空 icon 前端 map 不到 lucide 组件会渲染裂图。
    #[tokio::test]
    async fn create_rejects_blank_name_or_color_and_defaults_blank_icon() {
        let pool = fresh_test_pool().await;
        let baseline = list(&pool).await.unwrap().len();

        let err = create(&pool, cat_input("   ", "#123456", "Star"))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "全空格名必须被拒");
        let err = create(&pool, cat_input("阅读", "  ", "Star"))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "全空格颜色必须被拒");
        // 校验失败不能留下半截行
        assert_eq!(list(&pool).await.unwrap().len(), baseline);

        let cat = create(&pool, cat_input(" 阅读 ", " #123456 ", "  "))
            .await
            .unwrap();
        let got = find_cat(&pool, &cat.id).await.unwrap();
        assert_eq!(got.name, "阅读", "名字应 trim 后入库");
        assert_eq!(got.color, "#123456", "颜色应 trim 后入库");
        assert!(!got.icon.trim().is_empty(), "空 icon 必须兜底成非空默认值");
        assert!(!got.builtin, "用户新建的分类不能带 builtin 标志");
        // create 返回值与 list 读回的一致（前端拿返回值直接插 UI，不再刷新列表）
        assert_eq!(got.icon, cat.icon);
    }

    /// 为什么测：sort_order 用 MAX(active)+1 生成。若把软删行也算进 MAX，
    /// 用户反复"建了删、删了建"会让 sort_order 无限膨胀，跨设备 LWW 合并时
    /// 新分类和 tombstone 争位置导致顺序错乱。
    #[tokio::test]
    async fn create_appends_to_end_and_soft_deleted_rows_do_not_inflate_sort_order() {
        let pool = fresh_test_pool().await;
        let a = create(&pool, cat_input("甲", "#111111", "Star"))
            .await
            .unwrap();
        let b = create(&pool, cat_input("乙", "#222222", "Moon"))
            .await
            .unwrap();
        let ids = list_ids(&pool).await;
        // 新建的排在列表末尾且保持创建序
        assert_eq!(ids[ids.len() - 2], a.id);
        assert_eq!(ids[ids.len() - 1], b.id);

        let sort_b = raw_cat(&pool, &b.id).await.unwrap().4;
        delete(&pool, &b.id).await.unwrap();
        let c = create(&pool, cat_input("丙", "#333333", "Sun"))
            .await
            .unwrap();
        let sort_c = raw_cat(&pool, &c.id).await.unwrap().4;
        // 乙软删后它的位置应被回收：丙拿到与乙相同的 sort_order，而不是继续 +1
        assert_eq!(sort_c, sort_b);
        assert_eq!(list_ids(&pool).await.last().unwrap(), &c.id);
    }

    // ---------- update ----------

    /// 为什么测：前端"重命名"弹窗只传 name、调色板只传 color、图标选择器只传 icon。
    /// patch 里没给的字段绝不能被冲掉；空白字符串（用户清空输入框直接确认）按"不改"处理。
    #[tokio::test]
    async fn update_patches_each_field_independently_and_treats_blank_as_keep() {
        let pool = fresh_test_pool().await;
        let cat = create(&pool, cat_input("临时", "#111111", "Star"))
            .await
            .unwrap();

        update(
            &pool,
            &cat.id,
            CategoryPatch {
                name: Some("改名".into()),
                color: None,
                icon: None,
            },
        )
        .await
        .unwrap();
        let got = find_cat(&pool, &cat.id).await.unwrap();
        assert_eq!(got.name, "改名");
        assert_eq!(got.color, "#111111", "只改 name 不得动 color");
        assert_eq!(got.icon, "Star", "只改 name 不得动 icon");

        update(
            &pool,
            &cat.id,
            CategoryPatch {
                name: None,
                color: Some(" #222222 ".into()),
                icon: None,
            },
        )
        .await
        .unwrap();
        let got = find_cat(&pool, &cat.id).await.unwrap();
        assert_eq!(got.color, "#222222", "color 应 trim 后入库");
        assert_eq!(got.name, "改名");
        assert_eq!(got.icon, "Star");

        update(
            &pool,
            &cat.id,
            CategoryPatch {
                name: None,
                color: None,
                icon: Some("Moon".into()),
            },
        )
        .await
        .unwrap();
        let got = find_cat(&pool, &cat.id).await.unwrap();
        assert_eq!(got.icon, "Moon");
        assert_eq!(got.name, "改名");
        assert_eq!(got.color, "#222222");

        // 三个字段全给空白 → 全部视为"不改"，不能把行清空
        update(
            &pool,
            &cat.id,
            CategoryPatch {
                name: Some("   ".into()),
                color: Some(String::new()),
                icon: Some(" ".into()),
            },
        )
        .await
        .unwrap();
        let got = find_cat(&pool, &cat.id).await.unwrap();
        assert_eq!(
            (got.name.as_str(), got.color.as_str(), got.icon.as_str()),
            ("改名", "#222222", "Moon"),
            "空白 patch 不得清掉任何字段"
        );
    }

    /// 为什么测：`updated_at` 是跨设备 LWW 的仲裁者，它表示的应该是「内容最后一次
    /// 变化」，不是「最后一次被写」。原值重提一遍若刷新它，这台设备就凭空赢下一次
    /// 比较，把另一台真正改过的名字覆盖掉——用户自己敲的字没了。
    ///
    /// 后半段钉的是判断条件必须是「任一字段不同」而非「全部不同」：写成 AND 的话
    /// 只改一个字段会整条失效，而「字段等于新值」那种断言在改之前也成立，看不出来。
    #[tokio::test]
    async fn update_with_unchanged_values_bumps_nothing() {
        let pool = fresh_test_pool().await;
        let cat = create(&pool, cat_input("工作", "#aaaaaa", "Star"))
            .await
            .unwrap();
        let before_ts = raw_cat(&pool, &cat.id).await.unwrap().6;
        let before_outbox = outbox_total(&pool).await;

        // 三个字段原样重提
        update(
            &pool,
            &cat.id,
            CategoryPatch {
                name: Some("工作".into()),
                color: Some("#aaaaaa".into()),
                icon: Some("Star".into()),
            },
        )
        .await
        .unwrap();

        assert_eq!(
            raw_cat(&pool, &cat.id).await.unwrap().6,
            before_ts,
            "无变化的更新不得刷新 updated_at —— 它会在 LWW 里凭空赢过对端的真实改动"
        );
        assert_eq!(
            outbox_total(&pool).await,
            before_outbox,
            "无变化的更新不得入 outbox"
        );

        // 只改一个字段：必须生效
        update(
            &pool,
            &cat.id,
            CategoryPatch {
                name: Some("本职".into()),
                color: None,
                icon: None,
            },
        )
        .await
        .unwrap();

        let got = find_cat(&pool, &cat.id).await.expect("分类应仍在");
        assert_eq!(
            (got.name.as_str(), got.color.as_str(), got.icon.as_str()),
            ("本职", "#aaaaaa", "Star"),
            "只改一个字段也必须生效，其余不动"
        );
        assert_ne!(
            raw_cat(&pool, &cat.id).await.unwrap().6,
            before_ts,
            "真实改动必须刷新 updated_at"
        );
        assert_eq!(
            outbox_total(&pool).await,
            before_outbox + 1,
            "真实改动应恰好入一条 outbox"
        );
    }

    /// 为什么测：同步竞态下 update 可能落在已被另一台设备删掉的分类上。
    /// 期望静默 no-op：既不报错（用户无感），也绝不能把 tombstone 复活，
    /// 更不能给 no-op 入 outbox（否则重试风暴把垃圾事件推上云）。
    #[tokio::test]
    async fn update_ignores_missing_and_soft_deleted_rows() {
        let pool = fresh_test_pool().await;
        // 不存在的 id → 静默成功
        update(
            &pool,
            "ghost-id",
            CategoryPatch {
                name: Some("X".into()),
                color: None,
                icon: None,
            },
        )
        .await
        .unwrap();

        let cat = create(&pool, cat_input("将删", "#111111", "Star"))
            .await
            .unwrap();
        delete(&pool, &cat.id).await.unwrap();
        let before = outbox_count(&pool, "category").await;

        update(
            &pool,
            &cat.id,
            CategoryPatch {
                name: Some("复活?".into()),
                color: None,
                icon: None,
            },
        )
        .await
        .unwrap();

        let row = raw_cat(&pool, &cat.id).await.unwrap();
        assert!(row.5.is_some(), "update 不得清掉 deleted_at 把分类复活");
        assert_ne!(row.0, "复活?", "软删行的字段不应被改写");
        assert_eq!(
            outbox_count(&pool, "category").await,
            before,
            "对软删行的 update 是 no-op，不应入 outbox"
        );
    }

    // ---------- delete / 软删语义 ----------

    /// 为什么测：删除必须是软删——跨设备同步靠 tombstone 行传播删除事件；
    /// 物理删会让另一台设备把该分类原样推回来（"删不掉"复活 bug）。
    #[tokio::test]
    async fn soft_delete_hides_from_list_but_keeps_row_and_pushes_tombstone() {
        let pool = fresh_test_pool().await;
        let cat = create(&pool, cat_input("短命", "#111111", "Star"))
            .await
            .unwrap();
        delete(&pool, &cat.id).await.unwrap();

        assert!(
            list(&pool).await.unwrap().iter().all(|c| c.id != cat.id),
            "软删后不应再出现在 list 里"
        );
        let row = raw_cat(&pool, &cat.id)
            .await
            .expect("行必须还在（软删不是物理删）");
        assert!(row.5.is_some(), "deleted_at 必须被打上");

        // outbox 里必须有一条带 deletedAt 的 category 快照，云端才能感知删除
        let cid = cat.id.clone();
        let payloads: Vec<String> = pool
            .0
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT payload FROM sync_outbox
                      WHERE entity = 'category' AND entity_pk = ?1",
                )?;
                let rows = stmt.query_map(rusqlite::params![cid], |r| r.get::<_, String>(0))?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                Ok(out)
            })
            .await
            .unwrap();
        let has_tombstone = payloads.iter().any(|p| {
            serde_json::from_str::<serde_json::Value>(p)
                .map(|v| v.get("deletedAt").map(|d| d.is_string()).unwrap_or(false))
                .unwrap_or(false)
        });
        assert!(
            has_tombstone,
            "outbox 里必须有带 deletedAt 的 tombstone，实际 payloads={payloads:?}"
        );
    }

    /// 为什么测：双端并发删同一分类时，后到的删除请求看到的已是 tombstone / 空行。
    /// 应静默成功且不再入 outbox，否则每次重放都往云端推垃圾事件。
    #[tokio::test]
    async fn delete_missing_or_already_deleted_is_silent_noop() {
        let pool = fresh_test_pool().await;
        let before = outbox_count(&pool, "category").await;
        delete(&pool, "no-such-id").await.unwrap();
        assert_eq!(
            outbox_count(&pool, "category").await,
            before,
            "删不存在的 id 不应入 outbox"
        );

        let cat = create(&pool, cat_input("重复删", "#111111", "Star"))
            .await
            .unwrap();
        delete(&pool, &cat.id).await.unwrap();
        let mid = outbox_count(&pool, "category").await;
        delete(&pool, &cat.id).await.unwrap(); // 第二次删同一个
        assert_eq!(
            outbox_count(&pool, "category").await,
            mid,
            "重复删除是 no-op，不应再入 outbox"
        );
    }

    // ---------- 内置 / 特殊分类约束 ----------

    /// 为什么测：'hidden' 是"从统计里排除应用"的功能锚点（v27 里唯一 builtin=1 的行）。
    /// 一旦被删，被隐藏的 app 全部回流进报表——用户特意隐藏的内容重新出现在
    /// 日报 / AI 总结里，隐私场景直接翻车。但外观（名字/颜色/图标）允许个性化，
    /// 且改完外观后 builtin 守门必须依然生效。
    #[tokio::test]
    async fn builtin_hidden_rejects_delete_but_allows_restyle() {
        let pool = fresh_test_pool().await;
        let err = delete(&pool, "hidden").await.unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
        let hidden = find_cat(&pool, "hidden")
            .await
            .expect("拒绝删除后 hidden 必须原样活着");
        assert!(hidden.builtin);

        update(
            &pool,
            "hidden",
            CategoryPatch {
                name: Some("不给看".into()),
                color: None,
                icon: None,
            },
        )
        .await
        .unwrap();
        let hidden = find_cat(&pool, "hidden").await.unwrap();
        assert_eq!(hidden.name, "不给看", "内置分类允许改外观");
        assert!(
            hidden.builtin,
            "update 不得清掉 builtin 标志，否则改过名的 hidden 就能被删了"
        );
        // 改完名依然不可删
        assert!(matches!(
            delete(&pool, "hidden").await.unwrap_err(),
            Error::InvalidInput(_)
        ));
    }

    /// 为什么测：'other' 在 seed 里 builtin=0，光靠 builtin 守门拦不住；但报表 SQL
    /// 把所有未分类时长 COALESCE 到 'other'，删掉后前端解析不到分类，图表出现
    /// 无色缺口。这里钉死针对 id 的专门守门分支。
    #[tokio::test]
    async fn other_category_rejects_delete_despite_not_builtin() {
        let pool = fresh_test_pool().await;
        let other = find_cat(&pool, "other").await.expect("seed 应有 other");
        // 前提校验：other 确实不是 builtin —— 若某天 seed 改成 builtin=1，
        // 此测试就该换成测 builtin 分支而不是 id 分支
        assert!(!other.builtin);

        let err = delete(&pool, "other").await.unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
        assert!(
            find_cat(&pool, "other").await.is_some(),
            "拒绝删除后 other 必须仍在列表"
        );
    }

    // ---------- reorder / sort_order ----------

    /// 为什么测：拖拽重排是高频操作。1) 新顺序必须真实持久化（list 按 sort_order 出）；
    /// 2) 顺序没变的 reorder 不能重复入 outbox，否则前端每次 render 后补发的
    /// no-op reorder 都会把全量分类快照推上云。
    #[tokio::test]
    async fn reorder_applies_index_order_and_noop_reorder_skips_outbox() {
        let pool = fresh_test_pool().await;
        let before = list_ids(&pool).await;
        assert!(before.len() >= 2, "seed 应至少有两个分类才能测重排");
        let reversed: Vec<String> = before.iter().rev().cloned().collect();

        reorder(&pool, reversed.clone()).await.unwrap();
        assert_eq!(list_ids(&pool).await, reversed, "list 顺序应跟随 reorder");

        let n = outbox_count(&pool, "category").await;
        reorder(&pool, reversed.clone()).await.unwrap(); // 原地不动
        assert_eq!(
            outbox_count(&pool, "category").await,
            n,
            "顺序没变时不应产生新的 outbox 行"
        );
    }

    /// 为什么测：前端列表和 DB 可能瞬时不同步（另一台设备刚删了一个分类，本机还
    /// 基于旧列表发起拖拽）。带幽灵 id / 已删 id 的 reorder 不应报错、不应打乱
    /// 其余顺序、更不能把软删行拖活。
    #[tokio::test]
    async fn reorder_skips_unknown_and_soft_deleted_ids() {
        let pool = fresh_test_pool().await;
        let dead = create(&pool, cat_input("已删", "#111111", "Star"))
            .await
            .unwrap();
        delete(&pool, &dead.id).await.unwrap();

        let live = list_ids(&pool).await;
        let mut req = vec!["ghost-id".to_string(), dead.id.clone()];
        req.extend(live.iter().rev().cloned());
        reorder(&pool, req).await.unwrap();

        let after = list_ids(&pool).await;
        let expected: Vec<String> = live.iter().rev().cloned().collect();
        assert_eq!(after, expected, "幽灵 id 应被跳过，其余按索引就位");
        assert!(
            after.iter().all(|id| id != &dead.id),
            "软删行不得因 reorder 复活"
        );
    }

    // ---------- 删分类的 cascade：成员归属 ----------

    /// 为什么测：删分类时若不清 app_groups.category_id，组还挂在幽灵分类上——
    /// "待归类"卡片不出现该 app、报表又解析不到分类，两边都看不见它。
    /// 期望：组降级回未分类（而不是连坐删组），app_categories 镜像行同步软删。
    #[tokio::test]
    async fn delete_returns_member_group_to_unclassified() {
        let pool = fresh_test_pool().await;
        let cat = create(&pool, cat_input("工具", "#111111", "Wrench"))
            .await
            .unwrap();
        assign_app(&pool, "MyTool", &cat.id).await.unwrap();
        // 前置：绑定成功
        let got = find_cat(&pool, &cat.id).await.unwrap();
        assert!(got.apps.iter().any(|p| p == "MyTool"), "绑定应先生效");

        delete(&pool, &cat.id).await.unwrap();

        // 组还活着，但 category_id 被清空（回到未分类，而不是删组）
        let (g_cat, g_deleted): (Option<String>, Option<String>) = pool
            .0
            .call(|conn| {
                let row = conn.query_row(
                    "SELECT g.category_id, g.deleted_at
                       FROM app_group_members m
                       JOIN app_groups g ON g.id = m.group_id
                      WHERE m.process_name = 'MyTool'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                Ok(row)
            })
            .await
            .unwrap();
        assert!(g_deleted.is_none(), "删分类不应连坐删组");
        assert_eq!(g_cat, None, "组必须回到未分类");

        // 任何分类下都不应再出现 MyTool
        assert!(list(&pool)
            .await
            .unwrap()
            .iter()
            .all(|c| !c.apps.iter().any(|p| p == "MyTool")));
    }

    /// 为什么测：cascade 在本机删除和 sync pull 两条路径上都会被调用；若不幂等，
    /// 每次 pull 都给同一批 app_categories / app_groups 重复入 outbox，
    /// 两台设备之间形成推送风暴。
    #[tokio::test]
    async fn cascade_second_run_is_noop_without_new_outbox_rows() {
        let pool = fresh_test_pool().await;
        let cat = create(&pool, cat_input("循环", "#111111", "Repeat"))
            .await
            .unwrap();
        assign_app(&pool, "LoopApp", &cat.id).await.unwrap();
        delete(&pool, &cat.id).await.unwrap(); // 内部已完整跑过一次 cascade

        let before = outbox_total(&pool).await;
        let cid = cat.id.clone();
        pool.0
            .call(move |conn| {
                let tx = conn.transaction()?;
                cascade_category_deletion(&tx, &cid, "2026-07-26T00:00:00Z")?;
                tx.commit()?;
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(
            outbox_total(&pool).await,
            before,
            "重复 cascade 不应新增任何 outbox 行"
        );
    }

    // ---------- list_unclassified ----------

    async fn insert_activity(
        pool: &DbPool,
        process: &str,
        day_offset: i64,
        secs: i64,
        ended: &str,
    ) {
        let p = process.to_string();
        let e = ended.to_string();
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activities(started_at, ended_at, duration_secs, local_date,
                                            local_hour, process_name, category_id)
                     VALUES(?1, ?2, ?3, date('now','localtime', ?4 || ' days'), 9, ?5, 'other')",
                    rusqlite::params![e, e, secs, day_offset.to_string(), p],
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// 为什么测："待归类"卡片的判定必须走 app_groups 真实源且守住窗口边界：
    /// - 已归类 app 不出现（否则用户被反复要求归类同一个 app）；
    /// - 组指向**已删分类**的 app 要重新出现（cascade 失误 / 远端 tombstone 未级联时的兜底）；
    /// - 'Unknown' 噪声行、窗口外的老记录都要滤掉；
    /// - 分钟数是聚合值（整数分钟），排序按用量降序，用户先处理大头。
    #[tokio::test]
    async fn list_unclassified_uses_group_truth_window_and_aggregation() {
        let pool = fresh_test_pool().await;

        // FreeApp：今天 90s + 45s 两段 → 135s = 2 整分钟（截断）
        insert_activity(&pool, "FreeApp", 0, 90, "2026-07-26T09:00:00Z").await;
        insert_activity(&pool, "FreeApp", 0, 45, "2026-07-26T10:30:00Z").await;
        // Unknown：采集兜底名，永远不该让用户归类
        insert_activity(&pool, "Unknown", 0, 600, "2026-07-26T09:00:00Z").await;
        // OldApp：10 天前的活动，5 天窗口内不应出现
        insert_activity(&pool, "OldApp", -10, 600, "2026-07-16T09:00:00Z").await;
        // CodeApp：已归类到 seed 的 'code'，不应出现
        insert_activity(&pool, "CodeApp", 0, 600, "2026-07-26T09:00:00Z").await;
        assign_app(&pool, "CodeApp", "code").await.unwrap();
        // ZombieApp：归到一个随后被"绕过 cascade"软删的分类 → 应回到待归类
        insert_activity(&pool, "ZombieApp", 0, 600, "2026-07-26T09:00:00Z").await;
        let zombie_cat = create(&pool, cat_input("僵尸", "#111111", "Ghost"))
            .await
            .unwrap();
        assign_app(&pool, "ZombieApp", &zombie_cat.id)
            .await
            .unwrap();
        let zid = zombie_cat.id.clone();
        pool.0
            .call(move |conn| {
                // 模拟远端 tombstone 直接落库、没跑 cascade 的失误路径
                conn.execute(
                    "UPDATE categories SET deleted_at = '2026-07-26T00:00:00Z' WHERE id = ?1",
                    rusqlite::params![zid],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        let rows = list_unclassified(&pool, 5).await.unwrap();
        let names: Vec<&str> = rows.iter().map(|r| r.process_name.as_str()).collect();

        assert!(names.contains(&"FreeApp"), "无组的 app 应待归类: {names:?}");
        assert!(
            names.contains(&"ZombieApp"),
            "组指向已删分类的 app 应回到待归类: {names:?}"
        );
        assert!(!names.contains(&"CodeApp"), "已归类的 app 不应出现");
        assert!(!names.contains(&"Unknown"), "Unknown 噪声行必须滤掉");
        assert!(!names.contains(&"OldApp"), "窗口外的老记录必须滤掉");

        let free = rows.iter().find(|r| r.process_name == "FreeApp").unwrap();
        assert_eq!(free.minutes, 2, "90s+45s=135s 应聚合成 2 整分钟");
        assert_eq!(
            free.last_seen_at, "2026-07-26T10:30:00Z",
            "last_seen 应取两段里较晚的 ended_at"
        );

        // 排序：ZombieApp 10 分钟 > FreeApp 2 分钟，大头在前
        let pos_zombie = names.iter().position(|n| *n == "ZombieApp").unwrap();
        let pos_free = names.iter().position(|n| *n == "FreeApp").unwrap();
        assert!(pos_zombie < pos_free, "应按分钟数降序: {names:?}");
    }
}
