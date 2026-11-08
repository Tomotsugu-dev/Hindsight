use crate::error::Result;
use crate::memory::MemoryDb;
use crate::repo::categories::is_private;
use crate::storage::{DbPool, SqliteResultExt};
use crate::sync::payload::MemorySessionPayload;

/// Merges another device's screen-memory file for one day into the memory database. After the
/// merge, the local sessions of that device on that day match the file.
///
/// The same session is found by `guid`: an ID that is the same on every device, made at random
/// when the session is created. The local `id` counts up on its own, so it differs on each
/// device.
///
/// - Sessions in the file: inserted when there is no local copy. When the file's session ends
///   later, it replaces the text and the title.
/// - Sessions not in the file are deleted: the file holds all sessions of that device on that
///   day, so a missing one was deleted by that device.
/// - Sessions of private apps are skipped.
/// - Written sessions get that device as `origin_device`, so push does not send them back.
///
/// # Errors
///
/// Returns an error when a line cannot be read. Then nothing in the file is merged, and nothing
/// is deleted.
pub(super) async fn merge(
    pool: &DbPool,
    mem: &MemoryDb,
    device_id: &str,
    local_date: &str,
    body: &[u8],
) -> Result<()> {
    let mut rows: Vec<MemorySessionPayload> = Vec::new();
    for line in body.split(|b| *b == b'\n') {
        if line.is_empty() {
            continue;
        }
        let row: MemorySessionPayload =
            serde_json::from_slice(line).map_err(|e| crate::error::Error::SyncParse {
                kind: "memory_session",
                source: e,
            })?;
        if let Some(app) = &row.app_id {
            if is_private(pool, app, None).await? {
                continue;
            }
        }
        rows.push(row);
    }
    let sessions_in_file =
        serde_json::to_string(&rows.iter().map(|r| &r.guid).collect::<Vec<_>>())?;
    let (device, day) = (device_id.to_string(), local_date.to_string());
    mem.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            for r in &rows {
                // The update trigger keeps the full-text index in step.
                tx.execute(
                    "INSERT INTO text_sessions(
                         local_date, started_ts, ended_ts, app_id, title, text,
                         guid, origin_device)
                     VALUES (?2,?3,?4,?5,?6,?7,?1,?8)
                     ON CONFLICT(guid) DO UPDATE SET
                         ended_ts = excluded.ended_ts,
                         title = excluded.title,
                         text = excluded.text
                     WHERE excluded.ended_ts > text_sessions.ended_ts",
                    rusqlite::params![
                        r.guid,
                        r.local_date,
                        r.started_ts,
                        r.ended_ts,
                        r.app_id,
                        r.title,
                        r.text,
                        device,
                    ],
                )
                .db()?;
            }
            tx.execute(
                "DELETE FROM text_sessions
                  WHERE origin_device = ?1 AND local_date = ?2
                    AND guid NOT IN (SELECT value FROM json_each(?3))",
                rusqlite::params![device, day, sessions_in_file],
            )
            .db()?;
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::categories::assign_app;
    use crate::repo::test_util::fresh_test_pool;

    const DAY: &str = "2026-11-05";
    const DEVICE: &str = "device-a";

    /// 文件里的一行：Code 的一条会话。
    fn line(guid: &str, day: &str) -> String {
        session_of(guid, day, "Code")
    }

    /// 文件里的一行：`app` 的一条会话。
    fn session_of(guid: &str, day: &str, app: &str) -> String {
        serde_json::to_string(&MemorySessionPayload {
            guid: guid.into(),
            local_date: day.into(),
            started_ts: format!("{day}T10:00:00Z"),
            ended_ts: format!("{day}T10:05:00Z"),
            app_id: Some(app.into()),
            title: Some("t".into()),
            text: "文字".into(),
        })
        .unwrap()
    }

    /// 隐私应用的会话不写进本地。不写的会话不算文件里有，所以以前拉来的那条也删掉。
    #[tokio::test]
    async fn pull_skips_sessions_of_private_apps() {
        let pool = fresh_test_pool().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let body = [session_of("g1", DAY, "Tor"), line("g2", DAY)].join("\n");
        merge(&pool, &mem, DEVICE, DAY, body.as_bytes())
            .await
            .unwrap();
        assert_eq!(guids(&mem).await, ["g1", "g2"]);

        assign_app(&pool, "Tor", "private").await.unwrap();
        merge(&pool, &mem, DEVICE, DAY, body.as_bytes())
            .await
            .unwrap();

        assert_eq!(guids(&mem).await, ["g2"]);
    }

    /// 本地所有会话的 guid，本机自己的会话没有 guid，记成 "self"。
    async fn guids(mem: &MemoryDb) -> Vec<String> {
        mem.0
            .call(|conn| {
                let mut stmt = conn
                    .prepare("SELECT COALESCE(guid, 'self') FROM text_sessions ORDER BY 1")
                    .db()?;
                let rows = stmt
                    .query_map([], |r| r.get(0))
                    .db()?
                    .collect::<rusqlite::Result<Vec<String>>>()
                    .db()?;
                Ok(rows)
            })
            .await
            .unwrap()
    }

    /// 文件是那台设备那一天的全部会话：文件里没有的，那台设备已经删了，这边也删。
    /// 那台设备别的日子、别的设备、本机自己的会话都不动。
    #[tokio::test]
    async fn pull_deletes_sessions_missing_from_the_file() {
        let pool = fresh_test_pool().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let first = [line("g1", DAY), line("g2", DAY)].join("\n");
        merge(&pool, &mem, DEVICE, DAY, first.as_bytes())
            .await
            .unwrap();
        let next_day = "2026-11-06";
        merge(
            &pool,
            &mem,
            DEVICE,
            next_day,
            line("g3", next_day).as_bytes(),
        )
        .await
        .unwrap();
        merge(&pool, &mem, "device-b", DAY, line("g4", DAY).as_bytes())
            .await
            .unwrap();
        mem.0
            .call(|conn| {
                conn.execute(
                    "INSERT INTO text_sessions(local_date, started_ts, ended_ts, app_id, text)
                     VALUES('2026-11-05', '2026-11-05T09:00:00Z', '2026-11-05T09:05:00Z',
                            'Code', '本机的文字')",
                    [],
                )
                .db()
            })
            .await
            .unwrap();

        merge(&pool, &mem, DEVICE, DAY, line("g1", DAY).as_bytes())
            .await
            .unwrap();

        assert_eq!(guids(&mem).await, ["g1", "g3", "g4", "self"]);
    }

    /// 文件有一行读不出来：整个文件都不合并，原有的会话一条不删。
    #[tokio::test]
    async fn pull_keeps_sessions_when_the_file_does_not_parse() {
        let pool = fresh_test_pool().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let first = [line("g1", DAY), line("g2", DAY)].join("\n");
        merge(&pool, &mem, DEVICE, DAY, first.as_bytes())
            .await
            .unwrap();

        let broken = format!("{}\nnot json", line("g1", DAY));
        assert!(merge(&pool, &mem, DEVICE, DAY, broken.as_bytes())
            .await
            .is_err());

        assert_eq!(guids(&mem).await, ["g1", "g2"]);
    }
}
