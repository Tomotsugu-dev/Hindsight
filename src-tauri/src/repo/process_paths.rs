use chrono::Local;

use crate::error::Result;
use crate::storage::DbPool;

pub async fn upsert(pool: &DbPool, process_name: &str, exe_path: &str) -> Result<()> {
    let p = process_name.to_string();
    let e = exe_path.to_string();
    let seen = Local::now().to_rfc3339();
    pool.0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO process_paths(process_name, exe_path, seen_at) VALUES(?, ?, ?)
                 ON CONFLICT(process_name) DO UPDATE SET exe_path = excluded.exe_path, seen_at = excluded.seen_at",
                rusqlite::params![p, e, seen],
            )
            .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(())
        })
        .await?;
    Ok(())
}

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
                .ok();
            Ok(r)
        })
        .await?;
    Ok(path)
}
