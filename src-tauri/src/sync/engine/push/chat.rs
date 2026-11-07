use crate::error::Result;
use crate::memory::MemoryDb;
use crate::storage::SqliteResultExt;
use crate::sync::payload::{ChatConversationPayload, ChatFilePayload, ChatMessagePayload};

pub(super) async fn fingerprint(mem: &MemoryDb) -> Result<String> {
    let watermark: String = mem
        .0
        .call(|conn| {
            conn.query_row(
                "SELECT (SELECT COALESCE(MAX(updated_ts),'') || ':' || COUNT(*)
                           FROM chat_conversations)
                        || '|' ||
                        (SELECT COALESCE(MAX(created_ts),'') || ':' || COUNT(*)
                           FROM chat_messages)",
                [],
                |r| r.get(0),
            )
            .db()
        })
        .await?;
    Ok(watermark)
}

pub(super) async fn build(mem: &MemoryDb) -> Result<Vec<u8>> {
    let payload: ChatFilePayload = mem
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT guid, title, created_ts, updated_ts, deleted_at
                     FROM chat_conversations WHERE guid IS NOT NULL ORDER BY id",
                )
                .db()?;
            let conversations = stmt
                .query_map([], |r| {
                    Ok(ChatConversationPayload {
                        guid: r.get(0)?,
                        title: r.get(1)?,
                        created_ts: r.get(2)?,
                        updated_ts: r.get(3)?,
                        deleted_at: r.get(4)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            let mut stmt = conn
                .prepare(
                    "SELECT guid, conv_guid, role, content, citations, degraded, created_ts,
                            prompt_tokens, completion_tokens, parent_guid
                     FROM chat_messages
                     WHERE guid IS NOT NULL AND conv_guid IS NOT NULL ORDER BY id",
                )
                .db()?;
            let messages = stmt
                .query_map([], |r| {
                    Ok(ChatMessagePayload {
                        guid: r.get(0)?,
                        conv_guid: r.get(1)?,
                        role: r.get(2)?,
                        content: r.get(3)?,
                        citations: r.get(4)?,
                        degraded: r.get::<_, i64>(5)? != 0,
                        created_ts: r.get(6)?,
                        prompt_tokens: r.get(7)?,
                        completion_tokens: r.get(8)?,
                        parent_guid: r.get(9)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(ChatFilePayload {
                conversations,
                messages,
            })
        })
        .await?;
    Ok(serde_json::to_vec(&payload)?)
}
