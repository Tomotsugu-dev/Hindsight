//! ai_summaries:LWW by generated_at,键 (source, local_date, segment_idx)——
//! 全设备共享一份报告,谁新生成谁赢。

use crate::error::Result;
use crate::storage::{DbPool, SqliteResultExt};
use crate::sync::payload::AiSummaryPayload;

/// LWW 合并远端 ai_summaries:generated_at 更新者赢(全设备共享一份报告集)。
pub(super) async fn merge(pool: &DbPool, body: &[u8]) -> Result<()> {
    let rows: Vec<AiSummaryPayload> =
        serde_json::from_slice(body).map_err(|e| crate::error::Error::SyncParse {
            kind: "ai_summaries",
            source: e,
        })?;
    pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            for r in &rows {
                tx.execute(
                    "INSERT INTO ai_summaries(
                         source, local_date, segment_idx, label, start_hour, end_hour,
                         content, model, status, error, generated_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
                     ON CONFLICT(source, local_date, segment_idx) DO UPDATE SET
                         label = excluded.label,
                         start_hour = excluded.start_hour,
                         end_hour = excluded.end_hour,
                         content = excluded.content,
                         model = excluded.model,
                         status = excluded.status,
                         error = excluded.error,
                         generated_at = excluded.generated_at
                     WHERE excluded.generated_at > ai_summaries.generated_at",
                    rusqlite::params![
                        r.source,
                        r.local_date,
                        r.segment_idx as i64,
                        r.label,
                        r.start_hour as i64,
                        r.end_hour as i64,
                        r.content,
                        r.model,
                        r.status,
                        r.error,
                        r.generated_at,
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
