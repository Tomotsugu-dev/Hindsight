use crate::error::Result;
use crate::storage::{DbPool, SqliteResultExt};
use crate::sync::payload::AiSummaryPayload;

pub(super) async fn fingerprint(pool: &DbPool) -> Result<String> {
    let watermark: String = pool
        .0
        .call(|conn| {
            conn.query_row(
                "SELECT COALESCE(MAX(generated_at),'') || ':' || COUNT(*) FROM ai_summaries",
                [],
                |r| r.get(0),
            )
            .db()
        })
        .await?;
    Ok(watermark)
}

pub(super) async fn build(pool: &DbPool) -> Result<Vec<u8>> {
    let rows: Vec<AiSummaryPayload> = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT source, local_date, segment_idx, label, start_hour, end_hour,
                            content, model, status, error, generated_at
                     FROM ai_summaries ORDER BY local_date, source, segment_idx",
                )
                .db()?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(AiSummaryPayload {
                        source: r.get(0)?,
                        local_date: r.get(1)?,
                        segment_idx: r.get::<_, i64>(2)? as u32,
                        label: r.get(3)?,
                        start_hour: r.get::<_, i64>(4)? as u8,
                        end_hour: r.get::<_, i64>(5)? as u8,
                        content: r.get(6)?,
                        model: r.get(7)?,
                        status: r.get(8)?,
                        error: r.get(9)?,
                        generated_at: r.get(10)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await?;
    Ok(serde_json::to_vec(&rows)?)
}
