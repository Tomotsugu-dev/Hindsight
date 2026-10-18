//! Repository for super-categories, which group related categories for display.
//! They do not affect activity statistics. Every write enqueues an outbox row in
//! the same transaction so it syncs to other devices.

use serde::{Deserialize, Serialize};

use rusqlite::Connection;

use crate::error::{Error, Result};
use crate::repo::outbox::{enqueue, OutboxEntity, OutboxOp};
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuperCategory {
    /// UUID
    pub id: String,
    /// Human-readable name of the super-category.
    pub name: String,
    /// hex `#rrggbb`
    pub color: String,
    /// lucide icon name
    pub icon: String,
    /// Display order — updated when dragging; UI sorts in ascending order.
    pub sort_order: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuperCategoryInput {
    pub name: String,
    pub color: String,
    pub icon: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SuperCategoryPatch {
    pub name: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
}

fn enqueue_super(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    enqueue(
        conn,
        OutboxOp::Upsert,
        OutboxEntity::SuperCategory,
        id,
        "{}",
    )
}

/// A category's `super_category_id` changed; push the category row too. The push
/// side rebuilds the whole file from the table, so the payload is only a marker.
fn enqueue_category(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    enqueue(conn, OutboxOp::Upsert, OutboxEntity::Category, id, "{}")
}

/// List all super-categories that have not been soft-deleted, ordered by `sort_order` ascending.
pub async fn list(pool: &DbPool) -> Result<Vec<SuperCategory>> {
    let rows = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT id, name, color, icon, sort_order FROM super_categories
                     WHERE deleted_at IS NULL
                     -- Use id as tiebreaker
                     ORDER BY sort_order ASC, id ASC",
                )
                .db()?;
            let it = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
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
        .map(|(id, name, color, icon, sort_order)| SuperCategory {
            id,
            name,
            color,
            icon,
            sort_order,
        })
        .collect())
}

/// Create a super-category and place it at the end of the sort order.
pub async fn create(pool: &DbPool, input: SuperCategoryInput) -> Result<SuperCategory> {
    let id = uuid::Uuid::new_v4().to_string();
    let id_clone = id.clone();
    let name = input.name.trim().to_string();
    let color = input.color.trim().to_string();
    let icon = input.icon.trim().to_string();
    if name.is_empty() {
        return Err(Error::InvalidInput("super-category name must not be empty"));
    }
    if color.is_empty() {
        return Err(Error::InvalidInput("color must not be empty"));
    }
    // Use "Folder" as the default when `icon` is empty.
    let icon = if icon.is_empty() {
        "Folder".to_string()
    } else {
        icon
    };
    let n = name.clone();
    let c = color.clone();
    let i = icon.clone();
    let updated = utc_now_rfc3339();

    let sort_order = pool
        .0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            let next_sort: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM super_categories WHERE deleted_at IS NULL",
                    [],
                    |r| r.get(0),
                )
                .db()?;
            tx.execute(
                "INSERT INTO super_categories(id, name, color, icon, sort_order, updated_at)
                 VALUES(?, ?, ?, ?, ?, ?)",
                rusqlite::params![id_clone, n, c, i, next_sort, updated],
            )
            .db()?;
            enqueue_super(&tx, &id_clone).db()?;
            tx.commit().db()?;
            Ok(next_sort)
        })
        .await?;

    Ok(SuperCategory {
        id,
        name,
        color,
        icon,
        sort_order,
    })
}

/// Rename a super-category, change its color, or change its icon.
///
/// Fields that are `None` or empty strings in the patch are left unchanged.
pub async fn update(pool: &DbPool, id: &str, patch: SuperCategoryPatch) -> Result<()> {
    let id = id.to_string();
    let updated = utc_now_rfc3339();
    pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            let row: Option<(String, String, String)> = tx
                .query_row(
                    "SELECT name, color, icon FROM super_categories
                     WHERE id = ? AND deleted_at IS NULL",
                    rusqlite::params![id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .ok();
            let Some((cur_name, cur_color, cur_icon)) = row else {
                return Ok(());
            };

            let next_name = patch
                .name
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or(cur_name);
            let next_color = patch
                .color
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or(cur_color);
            let next_icon = patch
                .icon
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or(cur_icon);

            // A new name sets is_default_name to 0 (ADR-0015).
            tx.execute(
                "UPDATE super_categories SET name = ?1, color = ?2, icon = ?3, updated_at = ?4,
                                             is_default_name = is_default_name AND name IS ?1
                 WHERE id = ?5",
                rusqlite::params![next_name, next_color, next_icon, updated, id],
            )
            .db()?;
            enqueue_super(&tx, &id).db()?;
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// After the user reorders the super-category list by dragging, the frontend sends
/// the new order to the backend, which updates `sort_order` based on the array order.
pub async fn reorder(pool: &DbPool, ordered_ids: Vec<String>) -> Result<()> {
    let updated = utc_now_rfc3339();
    pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            for (idx, id) in ordered_ids.iter().enumerate() {
                let n = tx
                    .execute(
                        "UPDATE super_categories SET sort_order = ?, updated_at = ?
                         WHERE id = ? AND deleted_at IS NULL",
                        rusqlite::params![idx as i64, updated, id],
                    )
                    .db()?;
                if n > 0 {
                    enqueue_super(&tx, id).db()?;
                }
            }
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Soft-delete a super-category and set `super_category_id` to `NULL` for all
/// categories that reference it.
pub async fn delete(pool: &DbPool, id: &str) -> Result<()> {
    let id = id.to_string();
    let updated = utc_now_rfc3339();
    pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            let already: bool = tx
                .query_row(
                    "SELECT deleted_at IS NOT NULL FROM super_categories WHERE id = ?",
                    [&id],
                    |r| r.get(0),
                )
                .unwrap_or(true);
            if already {
                return Ok(());
            }
            // 1. Set `super_category_id` to NULL for all categories that reference this super-category.
            let children: Vec<String> = {
                let mut stmt = tx
                    .prepare("SELECT id FROM categories WHERE super_category_id = ?")
                    .db()?;
                let rows = stmt.query_map([&id], |r| r.get::<_, String>(0)).db()?;
                let mut v = Vec::new();
                for r in rows {
                    v.push(r.db()?);
                }
                v
            };
            tx.execute(
                "UPDATE categories SET super_category_id = NULL, updated_at = ?
                 WHERE super_category_id = ?",
                rusqlite::params![updated, id],
            )
            .db()?;
            // 2. Soft-delete the super-category itself.
            tx.execute(
                "UPDATE super_categories SET deleted_at = ?, updated_at = ?
                 WHERE id = ?",
                rusqlite::params![updated, updated, id],
            )
            .db()?;
            for child in &children {
                enqueue_category(&tx, child).db()?;
            }
            enqueue_super(&tx, &id).db()?;
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Assign a category to a super-category.
pub async fn assign_category(
    pool: &DbPool,
    category_id: &str,
    super_id: Option<String>,
) -> Result<()> {
    let cat_id = category_id.to_string();
    let updated = utc_now_rfc3339();
    pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            let n = tx
                .execute(
                    "UPDATE categories SET super_category_id = ?, updated_at = ?
                     WHERE id = ? AND deleted_at IS NULL",
                    rusqlite::params![super_id, updated, cat_id],
                )
                .db()?;
            if n > 0 {
                enqueue_category(&tx, &cat_id).db()?;
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

    async fn super_of(pool: &DbPool, cat: &str) -> Option<String> {
        let cat = cat.to_string();
        pool.0
            .call(move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT super_category_id FROM categories WHERE id = ?",
                        [cat],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .unwrap())
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn create_appends_update_patches_partially() {
        let pool = fresh_test_pool().await;
        let seeded = list(&pool).await.unwrap().len(); // 迁移种子的默认大类
        let a = create(
            &pool,
            SuperCategoryInput {
                name: "甲".into(),
                color: "#111111".into(),
                icon: "Star".into(),
            },
        )
        .await
        .unwrap();
        let b = create(
            &pool,
            SuperCategoryInput {
                name: "乙".into(),
                color: "#222222".into(),
                icon: "Moon".into(),
            },
        )
        .await
        .unwrap();
        let ls = list(&pool).await.unwrap();
        assert_eq!(ls.len(), seeded + 2);
        // 新建追加在尾部,顺序保持创建序
        assert_eq!(ls[ls.len() - 2].id, a.id);
        assert_eq!(ls[ls.len() - 1].id, b.id);

        // patch 只改给了的字段
        update(
            &pool,
            &a.id,
            SuperCategoryPatch {
                name: Some("甲改".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let got = list(&pool)
            .await
            .unwrap()
            .into_iter()
            .find(|x| x.id == a.id)
            .unwrap();
        assert_eq!(got.name, "甲改");
        assert_eq!(got.color, "#111111"); // 未给的字段不动
        assert_eq!(got.icon, "Star");
    }

    /// 用户改名后 is_default_name 变成 0。
    #[tokio::test]
    async fn update_rename_clears_is_default_name() {
        let pool = fresh_test_pool().await;
        let flag = |pool: DbPool| async move {
            pool.0
                .call(|conn| {
                    Ok(conn
                        .query_row(
                            "SELECT is_default_name FROM super_categories WHERE id = 'work'",
                            [],
                            |r| r.get::<_, bool>(0),
                        )
                        .unwrap())
                })
                .await
                .unwrap()
        };
        assert!(flag(pool.clone()).await);
        update(
            &pool,
            "work",
            SuperCategoryPatch {
                name: Some("Job".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(!flag(pool.clone()).await);
    }

    #[tokio::test]
    async fn reorder_rewrites_sort_order_by_index() {
        let pool = fresh_test_pool().await;
        let before = list(&pool).await.unwrap();
        let reversed: Vec<String> = before.iter().rev().map(|x| x.id.clone()).collect();
        reorder(&pool, reversed.clone()).await.unwrap();
        let after: Vec<String> = list(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|x| x.id)
            .collect();
        assert_eq!(after, reversed);
    }

    #[tokio::test]
    async fn delete_detaches_children_and_hides_super() {
        let pool = fresh_test_pool().await;
        let sup = create(
            &pool,
            SuperCategoryInput {
                name: "临时".into(),
                color: "#333333".into(),
                icon: "Box".into(),
            },
        )
        .await
        .unwrap();
        // 迁移种子里的 code 分类归入新大类
        assign_category(&pool, "code", Some(sup.id.clone()))
            .await
            .unwrap();
        assert_eq!(
            super_of(&pool, "code").await.as_deref(),
            Some(sup.id.as_str())
        );

        delete(&pool, &sup.id).await.unwrap();
        // 大类软删后不再列出;子分类回到"未归入"而不是被连坐删除
        assert!(list(&pool).await.unwrap().iter().all(|x| x.id != sup.id));
        assert_eq!(super_of(&pool, "code").await, None);

        // 移出大类(assign None)也走得通
        assign_category(&pool, "code", None).await.unwrap();
        assert_eq!(super_of(&pool, "code").await, None);
    }

    async fn outbox(pool: &DbPool) -> Vec<(String, String)> {
        pool.0
            .call(|conn| {
                let mut stmt = conn
                    .prepare("SELECT entity, entity_pk FROM sync_outbox ORDER BY id")
                    .unwrap();
                let v = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .unwrap()
                    .map(|x| x.unwrap())
                    .collect();
                Ok(v)
            })
            .await
            .unwrap()
    }

    async fn clear_outbox(pool: &DbPool) {
        pool.0
            .call(|conn| {
                conn.execute("DELETE FROM sync_outbox", []).unwrap();
                Ok(())
            })
            .await
            .unwrap();
    }

    fn input(name: &str) -> SuperCategoryInput {
        SuperCategoryInput {
            name: name.into(),
            color: "#123456".into(),
            icon: "Box".into(),
        }
    }

    fn sc(id: &str) -> (String, String) {
        ("super_category".into(), id.into())
    }

    fn cat(id: &str) -> (String, String) {
        ("category".into(), id.into())
    }

    #[tokio::test]
    async fn create_and_update_enqueue_super_category() {
        let pool = fresh_test_pool().await;
        clear_outbox(&pool).await;
        let a = create(&pool, input("甲")).await.unwrap();
        assert_eq!(outbox(&pool).await, vec![sc(&a.id)]);

        clear_outbox(&pool).await;
        update(&pool, &a.id, SuperCategoryPatch::default())
            .await
            .unwrap();
        assert_eq!(outbox(&pool).await, vec![sc(&a.id)]);

        // 不存在的 id:不写也不入队
        clear_outbox(&pool).await;
        update(&pool, "nope", SuperCategoryPatch::default())
            .await
            .unwrap();
        assert!(outbox(&pool).await.is_empty());
    }

    #[tokio::test]
    async fn reorder_enqueues_each_moved_super_category() {
        let pool = fresh_test_pool().await;
        let ids: Vec<String> = list(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|x| x.id)
            .collect();
        clear_outbox(&pool).await;
        reorder(&pool, ids.clone()).await.unwrap();
        let got = outbox(&pool).await;
        assert_eq!(got.len(), ids.len());
        assert!(got.iter().all(|(e, _)| e == "super_category"));
    }

    #[tokio::test]
    async fn delete_enqueues_super_and_detached_children() {
        let pool = fresh_test_pool().await;
        let sup = create(&pool, input("临时")).await.unwrap();
        assign_category(&pool, "code", Some(sup.id.clone()))
            .await
            .unwrap();
        clear_outbox(&pool).await;

        delete(&pool, &sup.id).await.unwrap();
        assert_eq!(outbox(&pool).await, vec![cat("code"), sc(&sup.id)]);

        // 重复删除:不再动 deleted_at,也不再入队
        clear_outbox(&pool).await;
        delete(&pool, &sup.id).await.unwrap();
        assert!(outbox(&pool).await.is_empty());
    }

    #[tokio::test]
    async fn assign_category_enqueues_the_category() {
        let pool = fresh_test_pool().await;
        let sup = create(&pool, input("临时")).await.unwrap();
        clear_outbox(&pool).await;
        assign_category(&pool, "code", Some(sup.id.clone()))
            .await
            .unwrap();
        assert_eq!(outbox(&pool).await, vec![cat("code")]);

        // 分类不存在:没改任何行,不入队
        clear_outbox(&pool).await;
        assign_category(&pool, "no-such-cat", None).await.unwrap();
        assert!(outbox(&pool).await.is_empty());
    }
}
