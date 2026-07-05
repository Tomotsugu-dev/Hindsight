//! Chat 会话历史持久层——存 memory.sqlite(chat_conversations / chat_messages)。
//!
//! 与屏幕记忆同库的理由:聊天派生自屏幕记忆、同为本地敏感资产;删记忆库即删聊天。
//! 默认只在本地;用户打开「聊天历史上云」开关后由 sync 引擎按 guid 推拉
//! (见 sync/engine/datasets.rs),删除靠 deleted_at 软删墓碑传播。
//!
//! citations 整条序列化为 JSON 列:读路径永远是"整条消息整体渲染",
//! 没有按引用查询的需求;回读失败兜底空数组,旧数据永不炸页面。

use serde::Serialize;

use super::engine::HistoryTurn;
use super::tools::Citation;
use crate::error::Result;
use crate::memory::MemoryDb;
use crate::storage::SqliteResultExt;

/// 会话列表项(按 updated_ts 倒序)。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMeta {
    pub id: i64,
    pub title: String,
    pub created_ts: String,
    pub updated_ts: String,
}

/// 落库的一条消息(user 的 citations 恒为空数组)。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredMessage {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub citations: Vec<Citation>,
    pub degraded: bool,
    pub created_ts: String,
}

/// 会话标题 = 首问截断:按字符(防中文截半)取前 24 个,超出加省略号。
pub fn truncate_title(q: &str) -> String {
    let q = q.trim();
    if q.chars().count() <= 24 {
        q.to_string()
    } else {
        let cut: String = q.chars().take(24).collect();
        format!("{cut}…")
    }
}

fn now_ts() -> String {
    chrono::Local::now().to_rfc3339()
}

pub async fn list_conversations(mem: &MemoryDb) -> Result<Vec<ConversationMeta>> {
    let rows = mem
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, title, created_ts, updated_ts FROM chat_conversations
                     WHERE deleted_at IS NULL
                     ORDER BY updated_ts DESC",
                )
                .db()?;
            let out = stmt
                .query_map([], |r| {
                    Ok(ConversationMeta {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        created_ts: r.get(2)?,
                        updated_ts: r.get(3)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(out)
        })
        .await?;
    Ok(rows)
}

pub async fn get_messages(mem: &MemoryDb, conv_id: i64) -> Result<Vec<StoredMessage>> {
    let rows = mem
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, role, content, citations, degraded, created_ts
                     FROM chat_messages WHERE conversation_id = ?1 ORDER BY id",
                )
                .db()?;
            let out = stmt
                .query_map([conv_id], |r| {
                    let citations_json: Option<String> = r.get(3)?;
                    Ok(StoredMessage {
                        id: r.get(0)?,
                        role: r.get(1)?,
                        content: r.get(2)?,
                        citations: citations_json
                            .and_then(|s| serde_json::from_str(&s).ok())
                            .unwrap_or_default(),
                        degraded: r.get::<_, i64>(4)? != 0,
                        created_ts: r.get(5)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(out)
        })
        .await?;
    Ok(rows)
}

/// 跨设备全局 id:随机 128-bit hex。
fn new_guid() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// 建会话,返回 id。
pub async fn create_conversation(mem: &MemoryDb, title: &str) -> Result<i64> {
    let title = title.to_string();
    let ts = now_ts();
    let guid = new_guid();
    let id = mem
        .0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO chat_conversations(title, created_ts, updated_ts, guid)
                 VALUES (?1, ?2, ?2, ?3)",
                rusqlite::params![title, ts, guid],
            )
            .db()?;
            Ok(conn.last_insert_rowid())
        })
        .await?;
    Ok(id)
}

/// 会话是否存在(且未删除)。
pub async fn conversation_exists(mem: &MemoryDb, conv_id: i64) -> Result<bool> {
    let n: i64 = mem
        .0
        .call(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM chat_conversations
                 WHERE id = ?1 AND deleted_at IS NULL",
                [conv_id],
                |r| r.get(0),
            )
            .db()
        })
        .await?;
    Ok(n > 0)
}

/// 重命名:trim 后空串拒绝,超 100 字符截断。
/// 顺带 bump updated_ts——它是同步 LWW 的变更时间戳(列表顺序随之上浮,可接受)。
pub async fn rename_conversation(mem: &MemoryDb, conv_id: i64, title: &str) -> Result<()> {
    let title = title.trim();
    if title.is_empty() {
        return Err(crate::error::Error::InvalidInput("会话标题不能为空"));
    }
    let title: String = title.chars().take(100).collect();
    let ts = now_ts();
    mem.0
        .call(move |conn| {
            conn.execute(
                "UPDATE chat_conversations SET title = ?2, updated_ts = ?3 WHERE id = ?1",
                rusqlite::params![conv_id, title, ts],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// 删会话:软删(留 guid 墓碑让删除传播到其它设备)+ 物理删消息。
pub async fn delete_conversation(mem: &MemoryDb, conv_id: i64) -> Result<()> {
    let ts = now_ts();
    mem.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            tx.execute(
                "DELETE FROM chat_messages WHERE conversation_id = ?1",
                [conv_id],
            )
            .db()?;
            tx.execute(
                "UPDATE chat_conversations SET deleted_at = ?2, updated_ts = ?2 WHERE id = ?1",
                rusqlite::params![conv_id, ts],
            )
            .db()?;
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn append_user(mem: &MemoryDb, conv_id: i64, content: &str) -> Result<()> {
    append(mem, conv_id, "user", content, None, false).await
}

pub async fn append_assistant(
    mem: &MemoryDb,
    conv_id: i64,
    content: &str,
    citations: &[Citation],
    degraded: bool,
) -> Result<()> {
    let json = serde_json::to_string(citations)?;
    append(mem, conv_id, "assistant", content, Some(json), degraded).await
}

async fn append(
    mem: &MemoryDb,
    conv_id: i64,
    role: &'static str,
    content: &str,
    citations_json: Option<String>,
    degraded: bool,
) -> Result<()> {
    let content = content.to_string();
    let ts = now_ts();
    let guid = new_guid();
    mem.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            tx.execute(
                "INSERT INTO chat_messages(conversation_id, role, content, citations, degraded,
                                           created_ts, guid, conv_guid)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7,
                         (SELECT guid FROM chat_conversations WHERE id = ?1))",
                rusqlite::params![
                    conv_id,
                    role,
                    content,
                    citations_json,
                    degraded as i64,
                    ts,
                    guid
                ],
            )
            .db()?;
            tx.execute(
                "UPDATE chat_conversations SET updated_ts = ?2 WHERE id = ?1",
                rusqlite::params![conv_id, ts],
            )
            .db()?;
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// 最近 n 条消息作为 LLM 历史(时间正序)。engine 只吃 role/content,引用不进历史。
pub async fn recent_history(mem: &MemoryDb, conv_id: i64, n: usize) -> Result<Vec<HistoryTurn>> {
    let mut rows = mem
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT role, content FROM chat_messages
                     WHERE conversation_id = ?1 ORDER BY id DESC LIMIT ?2",
                )
                .db()?;
            let out = stmt
                .query_map(rusqlite::params![conv_id, n as i64], |r| {
                    Ok(HistoryTurn {
                        role: r.get(0)?,
                        content: r.get(1)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(out)
        })
        .await?;
    rows.reverse();
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cite(index: usize) -> Citation {
        Citation {
            index,
            app: "Chrome".into(),
            title: "标题".into(),
            started_ts: "2026-07-05T10:00:00+09:00".into(),
            ended_ts: "2026-07-05T10:05:00+09:00".into(),
            frame_path: Some("2026-07-05/a.webp".into()),
        }
    }

    #[tokio::test]
    async fn roundtrip_and_cascade_delete() {
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let id = create_conversation(&mem, &truncate_title("这周我在 Cursor 用了多久?"))
            .await
            .unwrap();
        append_user(&mem, id, "这周我在 Cursor 用了多久?")
            .await
            .unwrap();
        append_assistant(&mem, id, "共 3 小时 [1]", &[cite(1)], false)
            .await
            .unwrap();

        let convs = list_conversations(&mem).await.unwrap();
        assert_eq!(convs.len(), 1);
        assert_eq!(convs[0].title, "这周我在 Cursor 用了多久?");

        let msgs = get_messages(&mem, id).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert!(msgs[0].citations.is_empty());
        // citations JSON 往返一致
        assert_eq!(msgs[1].citations.len(), 1);
        assert_eq!(msgs[1].citations[0].index, 1);
        assert_eq!(
            msgs[1].citations[0].frame_path.as_deref(),
            Some("2026-07-05/a.webp")
        );

        delete_conversation(&mem, id).await.unwrap();
        assert!(list_conversations(&mem).await.unwrap().is_empty());
        assert!(get_messages(&mem, id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn recent_history_order_and_limit() {
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let id = create_conversation(&mem, "t").await.unwrap();
        for i in 0..5 {
            append_user(&mem, id, &format!("问 {i}")).await.unwrap();
            append_assistant(&mem, id, &format!("答 {i}"), &[], false)
                .await
                .unwrap();
        }
        // 取最近 4 条:问3 答3 问4 答4,时间正序
        let hist = recent_history(&mem, id, 4).await.unwrap();
        let flat: Vec<String> = hist.iter().map(|h| h.content.clone()).collect();
        assert_eq!(flat, vec!["问 3", "答 3", "问 4", "答 4"]);
    }

    #[tokio::test]
    async fn rename_rules() {
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let id = create_conversation(&mem, "旧").await.unwrap();
        assert!(rename_conversation(&mem, id, "  ").await.is_err());
        rename_conversation(&mem, id, " 新标题 ").await.unwrap();
        assert_eq!(list_conversations(&mem).await.unwrap()[0].title, "新标题");
    }

    #[test]
    fn title_truncation_by_chars() {
        assert_eq!(truncate_title("短问题"), "短问题");
        let long = "一二三四五六七八九十一二三四五六七八九十一二三四五六";
        let t = truncate_title(long);
        assert_eq!(t.chars().count(), 25); // 24 字 + …
        assert!(t.ends_with('…'));
    }
}
