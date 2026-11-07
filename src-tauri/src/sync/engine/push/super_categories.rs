use super::build_table_rows;
use crate::error::Result;
use crate::storage::DbPool;
use crate::sync::payload::SuperCategoryPayload;

pub(super) async fn build(pool: &DbPool) -> Result<Vec<u8>> {
    build_table_rows(
        pool,
        "SELECT id, name, color, icon, sort_order, updated_at, deleted_at
         FROM super_categories ORDER BY id",
        |r| {
            Ok(SuperCategoryPayload {
                id: r.get(0)?,
                name: r.get(1)?,
                color: r.get(2)?,
                icon: r.get(3)?,
                sort_order: r.get(4)?,
                updated_at: r.get(5)?,
                deleted_at: r.get(6)?,
            })
        },
    )
    .await
}
