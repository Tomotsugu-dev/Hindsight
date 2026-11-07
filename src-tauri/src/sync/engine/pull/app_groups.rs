use super::{is_remote_newer, merge_lww_simple};
use crate::error::Result;
use crate::storage::DbPool;
use crate::sync::payload::AppGroupPayload;

pub(super) async fn merge(pool: &DbPool, body: &[u8]) -> Result<()> {
    merge_lww_simple(
        pool,
        "app_group",
        body,
        |row: &AppGroupPayload| (!row.id.is_empty()).then(|| row.id.clone()),
        |conn, row: AppGroupPayload| {
            if !is_remote_newer(
                conn,
                "SELECT updated_at FROM app_groups WHERE id = ?1",
                rusqlite::params![row.id],
                &row.updated_at,
            )? {
                return Ok(());
            }
            conn.execute(
                "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, ?, ?)
                 ON CONFLICT(id) DO UPDATE SET
                   display_name = excluded.display_name,
                   category_id  = excluded.category_id,
                   updated_at   = excluded.updated_at,
                   deleted_at   = excluded.deleted_at",
                rusqlite::params![
                    row.id,
                    row.display_name,
                    row.category_id,
                    row.updated_at,
                    row.deleted_at
                ],
            )?;
            Ok(())
        },
    )
    .await
}
