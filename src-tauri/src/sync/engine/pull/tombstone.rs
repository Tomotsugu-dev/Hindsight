use crate::error::Result;
use crate::storage::{DbPool, SqliteResultExt};
use crate::sync::payload::TombstonePayload;

/// Handles `device.<owner>.tombstone.json`: that device was removed from the
/// cloud. Deletes its activity rows held here from before `clearedAt`, then
/// marks its device card deleted.
pub(super) async fn merge(pool: &DbPool, owner_device_id: &str, body: &[u8]) -> Result<()> {
    let payload: TombstonePayload = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("tombstone of device {owner_device_id}: does not parse: {e}");
            return Ok(());
        }
    };
    if payload.cleared_at.is_empty() {
        log::warn!("tombstone of device {owner_device_id}: clearedAt is empty");
        return Ok(());
    }
    let owner = owner_device_id.to_string();
    let cleared_at = payload.cleared_at;
    let deleted = pool
        .0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            let n = tx
                .execute(
                    "DELETE FROM activities
                     WHERE device_id = ?1 AND updated_at < ?2",
                    rusqlite::params![owner, cleared_at],
                )
                .db()?;
            // Marked only if the device has not uploaded its device info since
            // `clearedAt`: a device still in use keeps uploading it, and
            // `device_meta::merge` clears `deleted_at` again when that arrives.
            tx.execute(
                "UPDATE devices
                 SET deleted_at = ?2, updated_at = ?2
                 WHERE device_id = ?1
                   AND updated_at < ?2
                   AND (deleted_at IS NULL OR deleted_at < ?2)",
                rusqlite::params![owner, cleared_at],
            )
            .db()?;
            tx.commit().db()?;
            Ok(n)
        })
        .await?;

    if deleted > 0 {
        log::info!("tombstone applied: device={owner_device_id} deleted {deleted} activity rows");
    }
    Ok(())
}
