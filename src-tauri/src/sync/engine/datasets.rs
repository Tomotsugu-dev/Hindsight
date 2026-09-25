//! 可选上云数据集:AI 总结文本 / 聊天历史 / 屏幕记忆全文。
//!
//! 与 outbox 驱动的核心数据集不同,这三类走**水位线检测**:
//! 每轮 push 先算数据集当前水位(最大时间戳+行数),与 sync_cursor 里存的
//! 上次推送水位比对,变了才全量重建文件上传——记忆库没有 outbox,
//! ai_summaries 也明确不入 outbox,这条路径对两边一视同仁。
//!
//! 文件命名沿用引擎的设备前缀分区(每设备只写自己的文件,天然无并发写冲突):
//! - `device.<id>.ai_summaries.json`   全量 Vec<AiSummaryPayload>
//! - `device.<id>.chat.json`           ChatFilePayload(会话含墓碑 + 消息)
//! - `device.<id>.memory.<date>.ndjson` 该日的 text_sessions(仅本机产出行)
//!
//! 合并语义:
//! - ai_summaries:LWW by generated_at,键 (source, local_date, segment_idx)——
//!   全设备共享一份报告,谁新生成谁赢;
//! - chat:会话按 guid LWW(updated_ts),deleted_at 墓碑传播时顺带删本地消息;
//!   消息不可变,按 guid INSERT OR IGNORE;
//! - memory:会话按 guid 合并,ended_ts 更新则覆盖 text/title(FTS 触发器自动跟进),
//!   远端行标 origin_device = 源设备,永不回推(push 只导 origin_device IS NULL)。
//!
//! 开关语义:settings 三挡分别门控各自数据集的**推与拉**。拉取时每一类有自己的
//! 游标(`pull.ai_summaries` / `pull.chat` / `pull.memory`,见 ADR-0006):开关关着
//! 那一类不拉、游标不动,打开后从停下的地方接着拉,历史文件自然补上。

use std::sync::Arc;

use super::io;
use super::Inner;
use crate::error::Result;
use crate::memory::MemoryDb;
use crate::repo::settings::Settings;
use crate::storage::{DbPool, SqliteResultExt};
use crate::sync::file_name::{Dataset, FileKind, FileName};
use crate::sync::payload::{
    AiSummaryPayload, ChatConversationPayload, ChatFilePayload, ChatMessagePayload,
    MemorySessionPayload,
};

// ───────────────────────────── push ─────────────────────────────

/// 每轮 push 调一次:对启用的数据集做水位线检测,变了才上传。
/// 单个数据集失败只记 warn(下一轮水位线仍不同会重试),不阻塞其它数据集。
pub(super) async fn push_optional(inner: &Arc<Inner>, cfg: &Settings) -> Result<()> {
    let self_id = inner.self_id.as_str();
    if self_id.is_empty() {
        return Ok(());
    }

    if cfg.sync_ai_summaries {
        if let Err(e) = push_ai_summaries(inner).await {
            log::warn!("push ai_summaries 失败: {e}");
        }
    }
    if cfg.sync_chat_history {
        if let Some(mem) = &inner.mem {
            if let Err(e) = push_chat(inner, mem).await {
                log::warn!("push chat 失败: {e}");
            }
        }
    }
    if cfg.sync_screen_memory {
        if let Some(mem) = &inner.mem {
            if let Err(e) = push_memory(inner, mem).await {
                log::warn!("push memory 失败: {e}");
            }
        }
    }
    Ok(())
}

/// Uploads one of this device's files.
async fn upload(inner: &Arc<Inner>, kind: FileKind, content: Vec<u8>) -> Result<()> {
    let name = FileName {
        device_id: inner.self_id.clone(),
        kind,
    }
    .to_file_name();
    inner.cloud.upsert_by_name(&name, &content).await?;
    Ok(())
}

async fn push_ai_summaries(inner: &Arc<Inner>) -> Result<()> {
    let watermark: String = inner
        .pool
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
    let name = inner.cloud.push_fingerprint_name(Dataset::AiSummaries);
    if io::read_cursor(&inner.pool, &name).await? == watermark {
        return Ok(());
    }
    let rows: Vec<AiSummaryPayload> = inner
        .pool
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
    upload(inner, FileKind::AiSummaries, serde_json::to_vec(&rows)?).await?;
    io::write_cursor(&inner.pool, &name, &watermark).await?;
    log::info!("push ai_summaries: {} 行", rows.len());
    Ok(())
}

async fn push_chat(inner: &Arc<Inner>, mem: &MemoryDb) -> Result<()> {
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
    let name = inner.cloud.push_fingerprint_name(Dataset::Chat);
    if io::read_cursor(&inner.pool, &name).await? == watermark {
        return Ok(());
    }
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
    upload(inner, FileKind::Chat, serde_json::to_vec(&payload)?).await?;
    io::write_cursor(&inner.pool, &name, &watermark).await?;
    log::info!(
        "push chat: {} 会话 / {} 消息",
        payload.conversations.len(),
        payload.messages.len()
    );
    Ok(())
}

async fn push_memory(inner: &Arc<Inner>, mem: &MemoryDb) -> Result<()> {
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
    let name = inner.cloud.push_fingerprint_name(Dataset::Memory);
    let prev = io::read_cursor(&inner.pool, &name).await?;
    if prev == watermark || watermark.is_empty() {
        return Ok(());
    }
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
    for day in &days {
        let Ok(date) = day.parse::<chrono::NaiveDate>() else {
            log::warn!("push memory: local_date {day:?} is not a date, skipped");
            continue;
        };
        let d = day.clone();
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
        upload(inner, FileKind::Memory(date), out).await?;
    }
    io::write_cursor(&inner.pool, &name, &watermark).await?;
    if !days.is_empty() {
        log::info!("push memory: {} 天的会话文件", days.len());
    }
    Ok(())
}

// ───────────────────────────── pull merge ─────────────────────────────

/// LWW 合并远端 ai_summaries:generated_at 更新者赢(全设备共享一份报告集)。
pub(super) async fn merge_ai_summaries(pool: &DbPool, body: &[u8]) -> Result<()> {
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

/// 合并远端聊天:会话 guid LWW(含墓碑传播),消息 guid 幂等插入。
pub(super) async fn merge_chat(mem: &MemoryDb, body: &[u8]) -> Result<()> {
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

/// 合并远端屏幕记忆会话:guid 幂等,ended_ts 更新则覆盖(FTS 触发器自动跟进)。
pub(super) async fn merge_memory_sessions(
    mem: &MemoryDb,
    device_id: &str,
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
