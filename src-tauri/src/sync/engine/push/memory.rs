use crate::error::Result;
use crate::memory::MemoryDb;
use crate::storage::SqliteResultExt;
use crate::sync::payload::MemorySessionPayload;

pub(super) async fn fingerprint(mem: &MemoryDb) -> Result<String> {
    // Watermark: the latest `ended_ts` of this device's own sessions. Folding
    // frames into a session only moves it later.
    let watermark: String = mem
        .0
        .call(|conn| {
            conn.query_row(
                "SELECT COALESCE(MAX(ended_ts),'') FROM text_sessions
                 WHERE origin_device IS NULL",
                [],
                |r| r.get(0),
            )
            .db()
        })
        .await?;
    Ok(watermark)
}

pub(super) async fn changed_days(mem: &MemoryDb, prev: String) -> Result<Vec<String>> {
    // The changed days: those with a session of this device that ended after
    // `prev`. On the first push `prev` is the epoch, so every day goes.
    let days: Vec<String> = mem
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT DISTINCT local_date FROM text_sessions
                     WHERE origin_device IS NULL AND ended_ts > ?1
                     ORDER BY local_date",
                )
                .db()?;
            let days = stmt
                .query_map([prev], |r| r.get::<_, String>(0))
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(days)
        })
        .await?;
    Ok(days)
}

pub(super) async fn build(mem: &MemoryDb, day: &str) -> Result<Vec<u8>> {
    let d = day.to_string();
    let rows: Vec<MemorySessionPayload> = mem
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT guid, local_date, started_ts, ended_ts, app_id, title, text
                     FROM text_sessions
                     WHERE origin_device IS NULL AND guid IS NOT NULL AND local_date = ?1
                     ORDER BY id",
                )
                .db()?;
            let rows = stmt
                .query_map([d], |r| {
                    Ok(MemorySessionPayload {
                        guid: r.get(0)?,
                        local_date: r.get(1)?,
                        started_ts: r.get(2)?,
                        ended_ts: r.get(3)?,
                        app_id: r.get(4)?,
                        title: r.get(5)?,
                        text: r.get(6)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await?;
    let mut out = Vec::with_capacity(rows.len() * 512);
    for row in &rows {
        out.extend_from_slice(serde_json::to_string(row)?.as_bytes());
        out.push(b'\n');
    }
    Ok(out)
}
