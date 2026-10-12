//! Application icon storage and synchronization.
//!
//! PNG bytes are stored in the `app_icons` table and synced to other devices.
//! The `icons/` directory holds PNG files for display in the frontend.
//! Icons extracted on this device are added to the outbox for synchronization.

use std::path::{Path, PathBuf};

use rusqlite::OptionalExtension;

use crate::error::Result;
use crate::repo::outbox::{enqueue, OutboxEntity, OutboxOp};
use crate::storage::{db_path_dir, utc_now_rfc3339, DbPool, SqliteResultExt};

/// Returns the path where a process's cached PNG should be stored:
/// `<data_root>/icons/<name>.png`, with the name made file-safe by `sanitize`.
pub fn icon_cache_path(process_name: &str) -> Result<PathBuf> {
    let dir = db_path_dir()?.join("icons");
    Ok(dir.join(format!("{}.png", sanitize(process_name))))
}

/// Turns a process name into a file name for the icon cache. Letters, digits,
/// `.`, `-` and `_` stay; every other character becomes `u<hex code>-`, so two
/// names that differ only in such characters still get different files.
fn sanitize(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
            out.push(c);
        } else {
            out.push_str(&format!("u{:X}-", c as u32));
        }
    }
    out
}

/// Writes an icon's PNG bytes to its place in the file cache, creating the
/// directory first if needed. A failed write is neither reported nor logged:
/// the file is only a copy of the database blob and gets written again next
/// time.
pub fn write_cache_file(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, bytes);
}

/// Saves an icon this device extracted: writes it to the `app_icons` table,
/// replacing the process's existing row and reviving a soft-deleted one, and
/// in the same transaction queues it for the next push so other devices
/// receive it. Because it queues, it is only for icons made on this device.
pub async fn upsert_local(pool: &DbPool, process_name: &str, icon_png: &[u8]) -> Result<()> {
    let p = process_name.to_string();
    let bytes = icon_png.to_vec();
    let updated_at = utc_now_rfc3339();

    pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            tx.execute(
                "INSERT INTO app_icons(process_name, icon_png, updated_at, deleted_at)
                 VALUES(?, ?, ?, NULL)
                 ON CONFLICT(process_name) DO UPDATE SET
                   icon_png   = excluded.icon_png,
                   updated_at = excluded.updated_at,
                   deleted_at = NULL",
                rusqlite::params![p, bytes, updated_at],
            )
            .db()?;

            // The payload holds no icon bytes: push reads the table when it builds
            // the icons file (ADR-0005).
            let payload = serde_json::json!({ "processName": p }).to_string();
            enqueue(&tx, OutboxOp::Upsert, OutboxEntity::AppIcon, &p, &payload).db()?;
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// The PNG stored for a process in `app_icons`, or `None` when there is no row
/// or the row is soft-deleted.
pub async fn get_blob(pool: &DbPool, process_name: &str) -> Result<Option<Vec<u8>>> {
    let p = process_name.to_string();
    let bytes = pool
        .0
        .call(move |conn| {
            let r = conn
                .query_row(
                    "SELECT icon_png FROM app_icons
                     WHERE process_name = ? AND deleted_at IS NULL",
                    [&p],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .optional()
                .db()?;
            Ok(r)
        })
        .await?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::fresh_test_pool;

    #[tokio::test]
    async fn upsert_get_blob_roundtrip_and_overwrite() {
        let pool = fresh_test_pool().await;
        assert_eq!(get_blob(&pool, "Code").await.unwrap(), None);

        upsert_local(&pool, "Code", &[1u8, 2, 3]).await.unwrap();
        assert_eq!(
            get_blob(&pool, "Code").await.unwrap(),
            Some(vec![1u8, 2, 3])
        );

        // 覆盖更新 + 软删标记复活(deleted_at 置回 NULL)
        upsert_local(&pool, "Code", &[9u8]).await.unwrap();
        assert_eq!(get_blob(&pool, "Code").await.unwrap(), Some(vec![9u8]));
    }
}
