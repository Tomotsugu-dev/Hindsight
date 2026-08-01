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
    /// 本轮上行/下行 token(assistant 才有;旧数据与 user 行为 NULL)
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    /// 消息树:自身全局 id 与父指针(NULL = 会话根)。前端据此组树、
    /// 渲染提问的编辑分支;发送与重试都带当前路径叶子作挂点。
    pub guid: String,
    pub parent_guid: Option<String>,
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
                    "SELECT id, role, content, citations, degraded, created_ts,
                            prompt_tokens, completion_tokens, guid, parent_guid
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
                        prompt_tokens: r.get(6)?,
                        completion_tokens: r.get(7)?,
                        guid: r.get(8)?,
                        parent_guid: r.get(9)?,
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

/// 落一条提问,挂在 parent 下(None = 会话根)。返回新行 guid(回答挂它下面)。
pub async fn append_user(
    mem: &MemoryDb,
    conv_id: i64,
    content: &str,
    parent: Option<&str>,
) -> Result<String> {
    append(mem, conv_id, "user", content, None, false, None, parent).await
}

/// 落一条回答,挂在 parent 下(通常是对应提问,重试时是上一版回答)。返回新行 guid。
#[allow(clippy::too_many_arguments)]
pub async fn append_assistant(
    mem: &MemoryDb,
    conv_id: i64,
    content: &str,
    citations: &[Citation],
    degraded: bool,
    tokens: (u64, u64),
    parent: Option<&str>,
) -> Result<String> {
    let json = serde_json::to_string(citations)?;
    append(
        mem,
        conv_id,
        "assistant",
        content,
        Some(json),
        degraded,
        Some((tokens.0 as i64, tokens.1 as i64)),
        parent,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn append(
    mem: &MemoryDb,
    conv_id: i64,
    role: &'static str,
    content: &str,
    citations_json: Option<String>,
    degraded: bool,
    tokens: Option<(i64, i64)>,
    parent: Option<&str>,
) -> Result<String> {
    let content = content.to_string();
    let ts = now_ts();
    let guid = new_guid();
    let parent = parent.map(str::to_string);
    let guid_out = guid.clone();
    mem.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            tx.execute(
                "INSERT INTO chat_messages(conversation_id, role, content, citations, degraded,
                                           created_ts, guid, conv_guid,
                                           prompt_tokens, completion_tokens, parent_guid)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7,
                         (SELECT guid FROM chat_conversations WHERE id = ?1), ?8, ?9, ?10)",
                rusqlite::params![
                    conv_id,
                    role,
                    content,
                    citations_json,
                    degraded as i64,
                    ts,
                    guid,
                    tokens.map(|t| t.0),
                    tokens.map(|t| t.1),
                    parent
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
    Ok(guid_out)
}

/// 连续多条 assistant 折叠为最后一条(时间正序输入)。连续段只有一个来源:
/// 重新生成的追加式落库——同一提问的多版回答,LLM 上下文只该看到最新版,
/// 旧版(往往正是用户嫌弃的那条)进上下文既毒化后续回答又白吃 token。
fn collapse_regenerated(rows: Vec<HistoryTurn>) -> Vec<HistoryTurn> {
    let mut out: Vec<HistoryTurn> = Vec::with_capacity(rows.len());
    for row in rows {
        if row.role == "assistant" && out.last().is_some_and(|p| p.role == "assistant") {
            *out.last_mut().unwrap() = row;
        } else {
            out.push(row);
        }
    }
    out
}

/// 从叶子沿 parent 回溯,返回"根→叶"顺序的链,只保留叶端最近 max 条。
/// 深度上限防环(同步合并出脏数据时不至于递归挂死);叶子不存在返回空。
fn chain_from_conn(
    conn: &rusqlite::Connection,
    leaf_guid: &str,
    max: usize,
) -> rusqlite::Result<Vec<HistoryTurn>> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE chain(guid, parent_guid, role, content, depth) AS (
             SELECT guid, parent_guid, role, content, 0
               FROM chat_messages WHERE guid = ?1
             UNION ALL
             SELECT m.guid, m.parent_guid, m.role, m.content, chain.depth + 1
               FROM chat_messages m JOIN chain ON m.guid = chain.parent_guid
             WHERE chain.depth < 128
         )
         SELECT role, content FROM chain ORDER BY depth DESC",
    )?;
    let rows = stmt
        .query_map([leaf_guid], |r| {
            Ok(HistoryTurn {
                role: r.get(0)?,
                content: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let skip = rows.len().saturating_sub(max);
    Ok(rows.into_iter().skip(skip).collect())
}

/// 新提问的取材:解析挂点 + 该挂点链上的历史(collapse 后,时间正序,最多 n 条)。
/// `parent` 约定:None = 会话当前最新叶子(常规发送);Some("") = 会话根
/// (编辑首条提问产生的分支);Some(guid) = 挂该消息下(编辑分支/旧分支续聊)。
/// 返回 (解析后的挂点, 历史);指定的挂点不存在时报错,防悬挂树。
pub async fn history_for_ask(
    mem: &MemoryDb,
    conv_id: i64,
    parent: Option<String>,
    n: usize,
) -> Result<(Option<String>, Vec<HistoryTurn>)> {
    let out = mem
        .0
        .call(move |conn| {
            use rusqlite::OptionalExtension;
            let resolved: Option<String> = match parent.as_deref() {
                Some("") => None,
                Some(g) => Some(g.to_string()),
                None => conn
                    .query_row(
                        "SELECT guid FROM chat_messages
                         WHERE conversation_id = ?1 ORDER BY id DESC LIMIT 1",
                        [conv_id],
                        |r| r.get(0),
                    )
                    .optional()
                    .db()?,
            };
            let history = match resolved.as_deref() {
                Some(g) => {
                    let chain = chain_from_conn(conn, g, n).db()?;
                    if chain.is_empty() {
                        return Ok(None); // 指定挂点不存在,外层转参数错误
                    }
                    collapse_regenerated(chain)
                }
                None => Vec::new(),
            };
            Ok(Some((resolved, history)))
        })
        .await?;
    out.ok_or(crate::error::Error::InvalidInput("指定的父消息不存在"))
}

/// 重试的取材:沿指定叶子(None = 会话最新叶子)回溯,找到路径上最近的提问。
/// 返回 (提问文本, 其之前的历史 collapse 后最多 n 条, 实际叶子 guid——新回答的挂点)。
/// 路径上没有提问时返回 None。
pub async fn question_for_regenerate(
    mem: &MemoryDb,
    conv_id: i64,
    leaf: Option<String>,
    n: usize,
) -> Result<Option<(String, Vec<HistoryTurn>, String)>> {
    let out = mem
        .0
        .call(move |conn| {
            use rusqlite::OptionalExtension;
            let leaf: Option<String> = match leaf {
                Some(g) => Some(g),
                None => conn
                    .query_row(
                        "SELECT guid FROM chat_messages
                         WHERE conversation_id = ?1 ORDER BY id DESC LIMIT 1",
                        [conv_id],
                        |r| r.get(0),
                    )
                    .optional()
                    .db()?,
            };
            let Some(leaf) = leaf else { return Ok(None) };
            // 全链取回(深度上限 128),Rust 侧切:HISTORY_TURNS 是个位数,量级无虞
            let chain = chain_from_conn(conn, &leaf, 128).db()?;
            let Some(pos) = chain.iter().rposition(|t| t.role == "user") else {
                return Ok(None);
            };
            let question = chain[pos].content.clone();
            let before: Vec<HistoryTurn> = chain[..pos].to_vec();
            let skip = before.len().saturating_sub(n);
            let history = collapse_regenerated(before.into_iter().skip(skip).collect());
            Ok(Some((question, history, leaf)))
        })
        .await?;
    Ok(out)
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
        let u = append_user(&mem, id, "这周我在 Cursor 用了多久?", None)
            .await
            .unwrap();
        append_assistant(
            &mem,
            id,
            "共 3 小时 [1]",
            &[cite(1)],
            false,
            (120, 45),
            Some(&u),
        )
        .await
        .unwrap();

        let convs = list_conversations(&mem).await.unwrap();
        assert_eq!(convs.len(), 1);
        assert_eq!(convs[0].title, "这周我在 Cursor 用了多久?");

        let msgs = get_messages(&mem, id).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert!(msgs[0].citations.is_empty());
        // 树指针落库:根的 parent 为 NULL,回答挂在提问下
        assert_eq!(msgs[0].parent_guid, None);
        assert_eq!(msgs[0].guid, u);
        assert_eq!(msgs[1].parent_guid.as_deref(), Some(u.as_str()));
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

    /// 线性链上的取材:默认挂最新叶,历史取叶端最近 n 条、时间正序。
    #[tokio::test]
    async fn history_order_and_limit_on_chain() {
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let id = create_conversation(&mem, "t").await.unwrap();
        let mut tip: Option<String> = None;
        for i in 0..5 {
            let u = append_user(&mem, id, &format!("问 {i}"), tip.as_deref())
                .await
                .unwrap();
            let a = append_assistant(&mem, id, &format!("答 {i}"), &[], false, (0, 0), Some(&u))
                .await
                .unwrap();
            tip = Some(a);
        }
        let (parent, hist) = history_for_ask(&mem, id, None, 4).await.unwrap();
        assert_eq!(parent, tip, "默认挂点应是最新叶子");
        let flat: Vec<String> = hist.iter().map(|h| h.content.clone()).collect();
        assert_eq!(flat, vec!["问 3", "答 3", "问 4", "答 4"]);
    }

    /// 提问编辑分支:同父兄弟各自成链,历史互不可见;Some("") 表示挂根。
    #[tokio::test]
    async fn edit_branches_are_isolated() {
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let id = create_conversation(&mem, "t").await.unwrap();
        let u1 = append_user(&mem, id, "原始提问", None).await.unwrap();
        let a1 = append_assistant(&mem, id, "原始回答", &[], false, (0, 0), Some(&u1))
            .await
            .unwrap();
        // 编辑首条提问 = 挂根的新分支:挂点解析为 None,历史为空
        let (parent, hist) = history_for_ask(&mem, id, Some(String::new()), 6)
            .await
            .unwrap();
        assert_eq!(parent, None);
        assert!(hist.is_empty(), "根分支不该看到旧分支的历史");
        let u2 = append_user(&mem, id, "编辑后的提问", None).await.unwrap();
        append_assistant(&mem, id, "新分支回答", &[], false, (0, 0), Some(&u2))
            .await
            .unwrap();
        // 在旧分支叶子上续聊:历史只含旧分支
        let (parent, hist) = history_for_ask(&mem, id, Some(a1.clone()), 6)
            .await
            .unwrap();
        assert_eq!(parent.as_deref(), Some(a1.as_str()));
        let flat: Vec<String> = hist.iter().map(|h| h.content.clone()).collect();
        assert_eq!(flat, vec!["原始提问", "原始回答"]);
        // 指定不存在的挂点报参数错误,不产生悬挂树
        assert!(history_for_ask(&mem, id, Some("no-such".into()), 6)
            .await
            .is_err());
    }

    /// 重试链的历史折叠:同一提问的多版回答只有最新版进 LLM 上下文
    /// (重试落库为链式续行:新版 parent = 旧版)。
    #[tokio::test]
    async fn history_collapses_regenerated_answers() {
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let id = create_conversation(&mem, "t").await.unwrap();
        let u1 = append_user(&mem, id, "问 1", None).await.unwrap();
        let a_old = append_assistant(&mem, id, "答 1 旧版", &[], false, (0, 0), Some(&u1))
            .await
            .unwrap();
        let a_new = append_assistant(&mem, id, "答 1 新版", &[], false, (0, 0), Some(&a_old))
            .await
            .unwrap();
        append_user(&mem, id, "问 2", Some(&a_new)).await.unwrap();

        let (_, hist) = history_for_ask(&mem, id, None, 6).await.unwrap();
        let flat: Vec<String> = hist.iter().map(|h| h.content.clone()).collect();
        assert_eq!(
            flat,
            vec!["问 1", "答 1 新版", "问 2"],
            "旧版回答不进上下文"
        );
    }

    /// 重试的取材:沿路径找最近提问,历史截到它之前;新回答挂在路径叶子下。
    #[tokio::test]
    async fn regenerate_takes_question_on_path() {
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let id = create_conversation(&mem, "t").await.unwrap();
        assert!(
            question_for_regenerate(&mem, id, None, 6)
                .await
                .unwrap()
                .is_none(),
            "空会话没有可重试的提问"
        );
        let u1 = append_user(&mem, id, "问 1", None).await.unwrap();
        let a1 = append_assistant(&mem, id, "答 1", &[], false, (0, 0), Some(&u1))
            .await
            .unwrap();
        let u2 = append_user(&mem, id, "问 2", Some(&a1)).await.unwrap();
        let a2 = append_assistant(&mem, id, "答 2(重试目标)", &[], false, (0, 0), Some(&u2))
            .await
            .unwrap();

        let (q, hist, leaf) = question_for_regenerate(&mem, id, None, 6)
            .await
            .unwrap()
            .expect("有提问");
        assert_eq!(q, "问 2");
        assert_eq!(leaf, a2, "新回答应挂在路径叶子(上一版回答)下面");
        let flat: Vec<String> = hist.iter().map(|h| h.content.clone()).collect();
        assert_eq!(flat, vec!["问 1", "答 1"], "问 2 及其后的回答都不进历史");
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
