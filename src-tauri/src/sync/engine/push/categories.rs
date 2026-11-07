use super::build_table_rows;
use crate::error::Result;
use crate::storage::DbPool;
use crate::sync::payload::CategoryPayload;

pub(super) async fn build(pool: &DbPool) -> Result<Vec<u8>> {
    build_table_rows(
        pool,
        "SELECT id, name, color, icon, builtin, sort_order, updated_at, deleted_at,
                super_category_id
         FROM categories ORDER BY id",
        |r| {
            Ok(CategoryPayload {
                id: r.get(0)?,
                name: r.get(1)?,
                color: r.get(2)?,
                icon: r.get(3)?,
                builtin: r.get::<_, i64>(4)? != 0,
                sort_order: r.get(5)?,
                updated_at: r.get(6)?,
                deleted_at: r.get(7)?,
                super_category_id: Some(r.get(8)?),
            })
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::fresh_test_pool;

    #[tokio::test]
    async fn build_categories_carries_super_category_id() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|conn| {
                conn.execute(
                    "UPDATE categories SET super_category_id = 'work' WHERE id = 'code'",
                    [],
                )
                .unwrap();
                conn.execute(
                    "UPDATE categories SET super_category_id = NULL WHERE id = 'other'",
                    [],
                )
                .unwrap();
                Ok(())
            })
            .await
            .unwrap();
        let bytes = build(&pool).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let find = |id: &str| {
            v.as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == id)
                .unwrap()
                .clone()
        };
        assert_eq!(find("code")["superCategoryId"], "work");
        // 显式 null 必须写出来,对端才能区分"移出大类"和"老版本没这个字段"
        assert!(find("other").get("superCategoryId").unwrap().is_null());
    }
}
