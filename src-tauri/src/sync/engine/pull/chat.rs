//! chat:会话按 guid LWW(updated_ts),deleted_at 墓碑传播时顺带删本地消息;
//! 消息不可变,按 guid INSERT OR IGNORE。

use crate::error::Result;
use crate::memory::MemoryDb;
use crate::storage::SqliteResultExt;
use crate::sync::payload::ChatFilePayload;

/// 合并远端聊天:会话 guid LWW(含墓碑传播),消息 guid 幂等插入。
pub(super) async fn merge(mem: &MemoryDb, body: &[u8]) -> Result<()> {
    let payload: ChatFilePayload =
        serde_json::from_slice(body).map_err(|e| crate::error::Error::SyncParse {
            kind: "chat",
            source: e,
        })?;
    mem.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            for c in &payload.conversations {
                tx.execute(
                    "INSERT INTO chat_conversations(title, created_ts, updated_ts, guid, deleted_at)
                     VALUES (?2, ?3, ?4, ?1, ?5)
                     ON CONFLICT(guid) DO UPDATE SET
                         title = excluded.title,
                         updated_ts = excluded.updated_ts,
                         deleted_at = excluded.deleted_at
                     WHERE excluded.updated_ts > chat_conversations.updated_ts",
                    rusqlite::params![c.guid, c.title, c.created_ts, c.updated_ts, c.deleted_at],
                )
                .db()?;
                // 墓碑落地后本地消息随之清掉(与本地删除语义一致)
                if c.deleted_at.is_some() {
                    tx.execute(
                        "DELETE FROM chat_messages WHERE conversation_id =
                             (SELECT id FROM chat_conversations WHERE guid = ?1)",
                        rusqlite::params![c.guid],
                    )
                    .db()?;
                }
            }
            for m in &payload.messages {
                // 会话缺失或已删 → 跳过该消息(墓碑优先)
                let conv: Option<i64> = tx
                    .query_row(
                        "SELECT id FROM chat_conversations
                         WHERE guid = ?1 AND deleted_at IS NULL",
                        rusqlite::params![m.conv_guid],
                        |r| r.get(0),
                    )
                    .ok();
                let Some(conv_id) = conv else { continue };
                tx.execute(
                    "INSERT OR IGNORE INTO chat_messages(
                         conversation_id, role, content, citations, degraded,
                         created_ts, guid, conv_guid, prompt_tokens, completion_tokens,
                         parent_guid)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                    rusqlite::params![
                        conv_id,
                        m.role,
                        m.content,
                        m.citations,
                        m.degraded as i64,
                        m.created_ts,
                        m.guid,
                        m.conv_guid,
                        m.prompt_tokens,
                        m.completion_tokens,
                        m.parent_guid,
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
