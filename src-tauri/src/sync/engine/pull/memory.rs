//! memory:会话按 guid 合并,ended_ts 更新则覆盖 text/title(FTS 触发器自动跟进),
//! 远端行标 origin_device = 源设备,永不回推(push 只导 origin_device IS NULL)。

use crate::error::Result;
use crate::memory::MemoryDb;
use crate::storage::SqliteResultExt;
use crate::sync::payload::MemorySessionPayload;

/// 合并远端屏幕记忆会话:guid 幂等,ended_ts 更新则覆盖(FTS 触发器自动跟进)。
pub(super) async fn merge(mem: &MemoryDb, device_id: &str, body: &[u8]) -> Result<()> {
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
        rows.push(row);
    }
    let device = device_id.to_string();
    mem.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            for r in &rows {
                // ON CONFLICT 走 UPDATE OF text 触发器,FTS 增量维护;仅更新更"新"的会话
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
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}
