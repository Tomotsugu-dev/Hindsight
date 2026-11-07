use rusqlite::OptionalExtension;

use super::merge_lww_simple;
use crate::error::Result;
use crate::repo::outbox::{enqueue, OutboxEntity, OutboxOp};
use crate::storage::DbPool;
use crate::sync::payload::SuperCategoryPayload;

/// Last write wins per row on `updated_at`, so `sort_order` is merged row by
/// row too: two devices reordering at once end with whichever wrote later.
pub(super) async fn merge(pool: &DbPool, body: &[u8]) -> Result<()> {
    merge_lww_simple(
        pool,
        "super_category",
        body,
        |row: &SuperCategoryPayload| (!row.id.is_empty()).then(|| row.id.clone()),
        |conn, row: SuperCategoryPayload| {
            let cur: Option<(String, Option<String>)> = conn
                .query_row(
                    "SELECT updated_at, deleted_at FROM super_categories WHERE id = ?1",
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
            // Names do not sync: an existing row keeps its local name (ADR-0015).
            conn.execute(
                "INSERT INTO super_categories(id, name, color, icon, sort_order, updated_at, deleted_at)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET
                    color = excluded.color, icon = excluded.icon,
                    sort_order = excluded.sort_order, updated_at = excluded.updated_at,
                    deleted_at = excluded.deleted_at",
                rusqlite::params![
                    row.id, row.name, row.color, row.icon, row.sort_order, row.updated_at,
                    row.deleted_at
                ],
            )?;

            // Deleted on the peer: detach the categories filed under it here and
            // push them on. Only on the present -> deleted step, so a repeat is a no-op.
            let cur_deleted = matches!(cur, Some((_, Some(_))));
            if row.deleted_at.is_some() && !cur_deleted {
                let children: Vec<String> = {
                    let mut stmt =
                        conn.prepare("SELECT id FROM categories WHERE super_category_id = ?1")?;
                    let rows = stmt.query_map([&row.id], |r| r.get::<_, String>(0))?;
                    rows.collect::<rusqlite::Result<Vec<_>>>()?
                };
                for child in &children {
                    conn.execute(
                        "UPDATE categories SET super_category_id = NULL, updated_at = ?1
                         WHERE id = ?2",
                        rusqlite::params![row.updated_at, child],
                    )?;
                    enqueue(conn, OutboxOp::Upsert, OutboxEntity::Category, child, "{}")?;
                }
            }
            Ok(())
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{exec_sql, outbox_entries, read_row, T_MID, T_NEW, T_OLD};
    use super::*;
    use crate::repo::test_util::fresh_test_pool;

    fn super_body(
        id: &str,
        name: &str,
        sort: i64,
        updated_at: &str,
        deleted_at: Option<&str>,
    ) -> Vec<u8> {
        serde_json::to_vec(&vec![SuperCategoryPayload {
            id: id.into(),
            name: name.into(),
            color: "#abcdef".into(),
            icon: "Box".into(),
            sort_order: sort,
            updated_at: updated_at.into(),
            deleted_at: deleted_at.map(String::from),
        }])
        .unwrap()
    }

    const SUPER_SQL: &str =
        "SELECT name, CAST(sort_order AS TEXT), updated_at, deleted_at FROM super_categories WHERE id = ?1";

    #[tokio::test]
    async fn merge_super_categories_inserts_then_lww() {
        let pool = fresh_test_pool().await;
        merge(&pool, &super_body("s1", "A", 1, T_MID, None))
            .await
            .unwrap();
        let row = read_row(&pool, SUPER_SQL, "s1", 4).await.unwrap();
        assert_eq!(row[0].as_deref(), Some("A"));

        // 更旧的不覆盖
        merge(&pool, &super_body("s1", "Old", 9, T_OLD, None))
            .await
            .unwrap();
        assert_eq!(
            read_row(&pool, SUPER_SQL, "s1", 4).await.unwrap()[0].as_deref(),
            Some("A")
        );

        // 更新的覆盖 sort_order,名字保留本机的
        merge(&pool, &super_body("s1", "New", 7, T_NEW, None))
            .await
            .unwrap();
        let row = read_row(&pool, SUPER_SQL, "s1", 4).await.unwrap();
        assert_eq!(row[0].as_deref(), Some("A"));
        assert_eq!(row[1].as_deref(), Some("7"));

        // 合并对端数据不回灌 outbox
        assert!(outbox_entries(&pool).await.is_empty());
    }

    #[tokio::test]
    async fn merge_super_categories_tombstone_detaches_children_once() {
        let pool = fresh_test_pool().await;
        merge(&pool, &super_body("s1", "A", 1, T_OLD, None))
            .await
            .unwrap();
        exec_sql(
            &pool,
            "UPDATE categories SET super_category_id = 's1' WHERE id = 'code'",
            vec![],
        )
        .await;

        let tomb = super_body("s1", "A", 1, T_NEW, Some(T_NEW));
        merge(&pool, &tomb).await.unwrap();
        let cat = read_row(
            &pool,
            "SELECT super_category_id, updated_at FROM categories WHERE id = ?1",
            "code",
            2,
        )
        .await
        .unwrap();
        assert_eq!(cat[0], None);
        assert_eq!(cat[1].as_deref(), Some(T_NEW));
        assert_eq!(
            outbox_entries(&pool).await,
            vec![("category".to_string(), "code".to_string())]
        );

        // 幂等:重放同一份文件不再产生新的出箱行
        merge(&pool, &tomb).await.unwrap();
        assert_eq!(outbox_entries(&pool).await.len(), 1);
    }
}
