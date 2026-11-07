use serde_json::Value;

use super::is_remote_newer;
use crate::error::{Error, Result};
use crate::storage::{DbPool, SqliteResultExt};
use crate::sync::payload::DeviceMetaPayload;

pub(super) async fn merge(pool: &DbPool, device_id: &str, body: &[u8]) -> Result<()> {
    let parsed: Value = serde_json::from_slice(body).map_err(|e| Error::SyncParse {
        kind: "device_meta",
        source: e,
    })?;
    // `{}` is what a device uploads before it has a devices row: nothing to merge.
    if parsed == serde_json::json!({}) {
        return Ok(());
    }
    let row: DeviceMetaPayload = serde_json::from_value(parsed).map_err(|e| Error::SyncParse {
        kind: "device_meta",
        source: e,
    })?;
    let device_id = device_id.to_string();
    pool.0
        .call(move |conn| {
            if !is_remote_newer(
                conn,
                "SELECT updated_at FROM devices WHERE device_id = ?1",
                rusqlite::params![device_id],
                &row.updated_at,
            )? {
                return Ok(());
            }
            // Newer device info means the device is still in use: clear `deleted_at`,
            // so a card a tombstone marked deleted comes back.
            conn.execute(
                "INSERT INTO devices(device_id, display_name, color, icon, os, last_seen_at, is_self, updated_at, deleted_at)
                 VALUES(?, ?, ?, ?, ?, ?, 0, ?, NULL)
                 ON CONFLICT(device_id) DO UPDATE SET
                   display_name = excluded.display_name,
                   color = excluded.color,
                   icon = excluded.icon,
                   os = excluded.os,
                   last_seen_at = excluded.last_seen_at,
                   updated_at = excluded.updated_at,
                   deleted_at = NULL",
                rusqlite::params![device_id, row.display_name, row.color, row.icon, row.os, row.last_seen_at, row.updated_at],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}
