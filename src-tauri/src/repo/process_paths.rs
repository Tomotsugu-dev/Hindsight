//! The `process_paths` table: the absolute path of each process's executable,
//! one row per process name. The capture loop records it through [`upsert`];
//! icon extraction reads it, since both the Windows exe and the macOS app
//! bundle are found through the path.
//!
//! Local only. It was synced once, and rows written back then by another
//! device may remain until the app is used again here (ADR-0003).

use chrono::Local;
use rusqlite::OptionalExtension;

use crate::error::Result;
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};

/// Records where a process's executable is. The capture loop calls it every
/// time the process is in the foreground. Writes the path and the time it was
/// last seen.
pub async fn upsert(pool: &DbPool, process_name: &str, exe_path: &str) -> Result<()> {
    let process_name = process_name.to_string();
    let exe_path = exe_path.to_string();
    let seen_at = Local::now().to_rfc3339();
    let updated = utc_now_rfc3339();

    pool.0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO process_paths(process_name, exe_path, seen_at, updated_at)
                 VALUES(?, ?, ?, ?)
                 ON CONFLICT(process_name) DO UPDATE SET
                   exe_path = excluded.exe_path,
                   seen_at = excluded.seen_at,
                   updated_at = excluded.updated_at",
                rusqlite::params![process_name, exe_path, seen_at, updated],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// The executable path recorded for a process, or `None` when none is known.
/// Icon extraction reads it to find the file to pull the icon from.
pub async fn get_path(pool: &DbPool, process_name: &str) -> Result<Option<String>> {
    let p = process_name.to_string();
    let path = pool
        .0
        .call(move |conn| {
            let r = conn
                .query_row(
                    "SELECT exe_path FROM process_paths WHERE process_name = ?",
                    [&p],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .db()?;
            Ok(r)
        })
        .await?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::fresh_test_pool;

    #[tokio::test]
    async fn upsert_then_get_roundtrip_and_update() {
        let pool = fresh_test_pool().await;
        assert_eq!(get_path(&pool, "Code").await.unwrap(), None);

        upsert(&pool, "Code", "/apps/Code.app").await.unwrap();
        assert_eq!(
            get_path(&pool, "Code").await.unwrap().as_deref(),
            Some("/apps/Code.app")
        );

        // 路径变更被更新;重复写同路径幂等不报错
        upsert(&pool, "Code", "/newpath/Code.app").await.unwrap();
        upsert(&pool, "Code", "/newpath/Code.app").await.unwrap();
        assert_eq!(
            get_path(&pool, "Code").await.unwrap().as_deref(),
            Some("/newpath/Code.app")
        );
    }
}
