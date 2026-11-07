use super::build_table_rows;
use crate::error::Result;
use crate::storage::DbPool;
use crate::sync::payload::AppGroupMemberPayload;

pub(super) async fn build(pool: &DbPool) -> Result<Vec<u8>> {
    build_table_rows(
        pool,
        "SELECT process_name, group_id, updated_at, deleted_at
         FROM app_group_members ORDER BY process_name",
        |r| {
            Ok(AppGroupMemberPayload {
                process_name: r.get(0)?,
                group_id: r.get(1)?,
                updated_at: r.get(2)?,
                deleted_at: r.get(3)?,
            })
        },
    )
    .await
}
