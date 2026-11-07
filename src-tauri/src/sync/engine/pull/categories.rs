use rusqlite::OptionalExtension;

use super::merge_lww_simple;
use crate::error::Result;
use crate::storage::DbPool;
use crate::sync::payload::CategoryPayload;

pub(super) async fn merge(pool: &DbPool, body: &[u8]) -> Result<()> {
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
                // Names do not sync: keep the local name (ADR-0015).
                conn.execute(
                    "UPDATE categories SET color = ?, icon = ?, builtin = ?,
                                            sort_order = ?, updated_at = ?, deleted_at = ?
                     WHERE id = ?",
                    rusqlite::params![row.color, row.icon, row.builtin as i64, row.sort_order, row.updated_at, row.deleted_at, row.id],
                )?;
            }
            // Absent key = older peer: keep the local super-category.
            if let Some(super_id) = &row.super_category_id {
                conn.execute(
                    "UPDATE categories SET super_category_id = ?1 WHERE id = ?2",
                    rusqlite::params![super_id, row.id],
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

#[cfg(test)]
mod tests {
    use super::super::test_util::{exec_sql, outbox_entries, read_row, s, T_MID, T_NEW, T_OLD};
    use super::*;
    use crate::repo::test_util::fresh_test_pool;

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
            super_category_id: None,
        }])
        .unwrap()
    }

    #[tokio::test]
    async fn merge_categories_super_category_id_absent_null_or_value() {
        let pool = fresh_test_pool().await;
        let cat_sql = "SELECT super_category_id FROM categories WHERE id = ?1";
        let body = |extra: serde_json::Value, ts: &str| {
            let mut v = serde_json::json!({
                "id": "code", "name": "Code", "color": "#123456", "icon": "Tag",
                "builtin": false, "sortOrder": 3, "updatedAt": ts, "deletedAt": null
            });
            v.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            serde_json::to_vec(&vec![v]).unwrap()
        };
        exec_sql(
            &pool,
            "UPDATE categories SET updated_at = ?1 WHERE id = 'code'",
            vec![s(T_OLD)],
        )
        .await;

        // 带值:写入
        merge(
            &pool,
            &body(serde_json::json!({"superCategoryId": "work"}), T_MID),
        )
        .await
        .unwrap();
        assert_eq!(
            read_row(&pool, cat_sql, "code", 1).await.unwrap()[0].as_deref(),
            Some("work")
        );

        // 老对端没这个字段:保持本地值
        merge(&pool, &body(serde_json::json!({}), T_NEW))
            .await
            .unwrap();
        assert_eq!(
            read_row(&pool, cat_sql, "code", 1).await.unwrap()[0].as_deref(),
            Some("work")
        );

        // 显式 null:移出大类
        let t_newer = "2026-06-04T00:00:00Z";
        merge(
            &pool,
            &body(serde_json::json!({"superCategoryId": null}), t_newer),
        )
        .await
        .unwrap();
        assert_eq!(read_row(&pool, cat_sql, "code", 1).await.unwrap()[0], None);
    }

    /// 已有的分类:对端较新的行覆盖其他字段,名字保留本机的。本机没有的分类用对端的名字新建。
    #[tokio::test]
    async fn merge_categories_keeps_local_name() {
        let pool = fresh_test_pool().await;
        exec_sql(
            &pool,
            "UPDATE categories SET name = 'Coding', color = '#aaaaaa', updated_at = ?1 WHERE id = 'code'",
            vec![s(T_OLD)],
        )
        .await;

        merge(&pool, &category_body("code", "コーディング", T_NEW, None))
            .await
            .unwrap();
        assert_eq!(
            read_row(
                &pool,
                "SELECT name, color, updated_at FROM categories WHERE id = ?1",
                "code",
                3
            )
            .await
            .unwrap(),
            vec![s("Coding"), s("#123456"), s(T_NEW)]
        );

        merge(&pool, &category_body("c1", "健身", T_NEW, None))
            .await
            .unwrap();
        assert_eq!(
            read_row(&pool, "SELECT name FROM categories WHERE id = ?1", "c1", 1)
                .await
                .unwrap(),
            vec![s("健身")]
        );
    }

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
        merge(
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
        merge(&pool, &body).await.unwrap();

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
        merge(&pool, &body).await.unwrap();
        assert_eq!(
            outbox_entries(&pool).await.len(),
            1,
            "重复 merge 不得二次级联(outbox 行数应保持 1)"
        );
    }
}
