use super::build_table_rows;
use crate::error::Result;
use crate::storage::DbPool;
use crate::sync::payload::AppGroupPayload;

pub(super) async fn build(pool: &DbPool) -> Result<Vec<u8>> {
    build_table_rows(
        pool,
        "SELECT id, display_name, category_id, updated_at, deleted_at
         FROM app_groups ORDER BY id",
        |r| {
            Ok(AppGroupPayload {
                id: r.get(0)?,
                display_name: r.get(1)?,
                category_id: r.get(2)?,
                updated_at: r.get(3)?,
                deleted_at: r.get(4)?,
            })
        },
    )
    .await
}
