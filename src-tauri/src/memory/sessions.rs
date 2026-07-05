//! L3 会话折叠:标题定界 + 行级并集(screen-memory.md §3 L3)。
//!
//! 去重的对象是帧,合并的是文字:同一次阅读/编辑的 N 帧折叠成一条会话,
//! 文本取行级并集、一行不丢;每个唯一行记首次出现帧(行级留痕 → 证据卡)。
//! 折叠状态([`Folder`])只活在一次消化 run 内——跨 run 的边界帧最多把一次
//! 阅读拆成两条会话,不丢信息。

use std::collections::HashSet;

use chrono::{DateTime, FixedOffset};
use rusqlite::params;

use super::frames::PendingFrame;
use super::MemoryDb;
use crate::error::Result;
use crate::storage::SqliteResultExt;

/// 同一会话内相邻帧的最大时间间隔;超过即封会话(§10 定案:5 min)。
const SESSION_GAP_SECS: i64 = 5 * 60;
/// 行有效长度下限(标准化后字符数);滤图标/单字噪音(§10 定案:≥6)。
const MIN_LINE_CHARS: usize = 6;

/// 窗口标题标准化——会话定界与 L1 事件豁免共用的唯一入口。
/// v1 只做空白折叠;动态计数器(如"(3) 微信")等规则将来在这一处扩。
pub fn normalize_title(title: &str) -> String {
    title.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 行标准化:空白折叠;过短(含图标/单字噪音)返回 None。
fn normalize_line(line: &str) -> Option<String> {
    let s = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() < MIN_LINE_CHARS {
        None
    } else {
        Some(s)
    }
}

/// 一次消化 run 内的折叠状态机。按时间序喂帧,自动开/封会话。
#[derive(Default)]
pub struct Folder {
    current: Option<OpenSession>,
}

struct OpenSession {
    id: i64,
    app_id: Option<String>,
    norm_title: String,
    last_ts: Option<DateTime<FixedOffset>>,
    /// 会话内已收录行(行级并集的判重集,会话封口即弃)
    seen: HashSet<String>,
    next_line_no: i64,
}

impl Folder {
    /// 折叠一帧:返回该帧归属的会话 id。`lines` 是 OCR 出的原始行(未标准化)。
    pub async fn fold_frame(
        &mut self,
        db: &MemoryDb,
        frame: &PendingFrame,
        lines: &[String],
    ) -> Result<i64> {
        let norm_title = normalize_title(frame.title.as_deref().unwrap_or(""));
        let frame_ts = DateTime::parse_from_rfc3339(&frame.ts).ok();

        // 会话边界:app 变 / 标准化标题变 / 时间断裂(> 5min) → 封旧开新
        let same = match &self.current {
            Some(cur) => {
                cur.app_id == frame.app_id
                    && cur.norm_title == norm_title
                    && match (cur.last_ts, frame_ts) {
                        (Some(a), Some(b)) => (b - a).num_seconds().abs() <= SESSION_GAP_SECS,
                        // 时间戳解析不了就只按标题定界,宁可少切不多切
                        _ => true,
                    }
            }
            None => false,
        };

        if !same {
            let id = open_session(db, frame, &norm_title).await?;
            self.current = Some(OpenSession {
                id,
                app_id: frame.app_id.clone(),
                norm_title,
                last_ts: frame_ts,
                seen: HashSet::new(),
                next_line_no: 0,
            });
        }

        let cur = self.current.as_mut().expect("上面刚保证过 current 存在");
        cur.last_ts = frame_ts;

        // 行级并集:只追加本会话没见过的行
        let mut fresh: Vec<String> = Vec::new();
        for line in lines {
            if let Some(n) = normalize_line(line) {
                if cur.seen.insert(n.clone()) {
                    fresh.push(n);
                }
            }
        }
        if !fresh.is_empty() {
            append_lines(db, cur.id, cur.next_line_no, &fresh, &frame.path, &frame.ts).await?;
            cur.next_line_no += fresh.len() as i64;
        } else {
            // 零新行(回看/静止)也要推进会话结束时刻
            touch_session(db, cur.id, &frame.ts).await?;
        }
        Ok(cur.id)
    }
}

async fn open_session(db: &MemoryDb, frame: &PendingFrame, norm_title: &str) -> Result<i64> {
    let (local_date, ts, app_id, title) = (
        frame.local_date.clone(),
        frame.ts.clone(),
        frame.app_id.clone(),
        norm_title.to_string(),
    );
    let id =
        db.0.call(move |conn| {
            conn.execute(
                "INSERT INTO text_sessions(local_date, started_ts, ended_ts, app_id, title, guid)
                 VALUES (?1, ?2, ?2, ?3, ?4, lower(hex(randomblob(16))))",
                params![local_date, ts, app_id, title],
            )
            .db()?;
            Ok(conn.last_insert_rowid())
        })
        .await?;
    Ok(id)
}

/// 追加新行:写 session_lines(行级留痕)+ 增量拼接会话文本 + 推进 ended_ts。
/// 文本 UPDATE 触发 FTS 同步(见 mod.rs 的 triggers)。
async fn append_lines(
    db: &MemoryDb,
    session_id: i64,
    start_no: i64,
    fresh: &[String],
    frame_path: &str,
    frame_ts: &str,
) -> Result<()> {
    let (fresh, path, ts) = (fresh.to_vec(), frame_path.to_string(), frame_ts.to_string());
    db.0.call(move |conn| {
        let tx = conn.transaction().db()?;
        for (i, line) in fresh.iter().enumerate() {
            tx.execute(
                "INSERT INTO session_lines(session_id, line_no, text, first_path, first_ts)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![session_id, start_no + i as i64, line, path, ts],
            )
            .db()?;
        }
        let joined = fresh.join("\n");
        tx.execute(
            "UPDATE text_sessions SET
                text = CASE WHEN text = '' THEN ?2 ELSE text || char(10) || ?2 END,
                ended_ts = ?3
             WHERE id = ?1",
            params![session_id, joined, ts],
        )
        .db()?;
        tx.commit().db()?;
        Ok(())
    })
    .await?;
    Ok(())
}

async fn touch_session(db: &MemoryDb, session_id: i64, ended_ts: &str) -> Result<()> {
    let ts = ended_ts.to_string();
    db.0.call(move |conn| {
        conn.execute(
            "UPDATE text_sessions SET ended_ts = ?2 WHERE id = ?1",
            params![session_id, ts],
        )
        .db()?;
        Ok(())
    })
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(path: &str, ts: &str, title: &str) -> PendingFrame {
        PendingFrame {
            path: path.into(),
            ts: ts.into(),
            local_date: "2026-07-05".into(),
            app_id: Some("code".into()),
            title: Some(title.into()),
        }
    }

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    async fn session_text(db: &MemoryDb, id: i64) -> String {
        db.0.call(move |conn| {
            conn.query_row("SELECT text FROM text_sessions WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .db()
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn union_folds_and_keeps_provenance() {
        let db = MemoryDb::open_in_memory().await.unwrap();
        let mut folder = Folder::default();

        // 帧1:两行;帧2:一行重复 + 一行新(滚动) → 同会话,并集 3 行
        let s1 = folder
            .fold_frame(
                &db,
                &frame("a.jpg", "2026-07-05T10:00:00+09:00", "文章"),
                &lines(&["第一行内容足够长", "第二行内容足够长"]),
            )
            .await
            .unwrap();
        let s2 = folder
            .fold_frame(
                &db,
                &frame("b.jpg", "2026-07-05T10:00:30+09:00", "文章"),
                &lines(&["第二行内容足够长", "第三行内容足够长"]),
            )
            .await
            .unwrap();
        assert_eq!(s1, s2);
        let text = session_text(&db, s1).await;
        assert_eq!(text.lines().count(), 3);

        // 行级留痕:旧行首现于帧a,新行首现于帧b
        db.0.call(move |conn| {
            let p: String = conn
                .query_row(
                    "SELECT first_path FROM session_lines WHERE session_id=?1 AND text LIKE '第三行%'",
                    [s1],
                    |r| r.get(0),
                )
                .db()?;
            assert_eq!(p, "b.jpg");
            Ok(())
        })
        .await
        .unwrap();

        // FTS 能按子串搜到
        db.0.call(|conn| {
            let hits: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM text_sessions_fts WHERE text_sessions_fts MATCH '三行内容'",
                    [],
                    |r| r.get(0),
                )
                .db()?;
            assert_eq!(hits, 1);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn title_change_and_gap_open_new_sessions() {
        let db = MemoryDb::open_in_memory().await.unwrap();
        let mut folder = Folder::default();
        let l = lines(&["同一行内容足够长"]);

        let a = folder
            .fold_frame(
                &db,
                &frame("a.jpg", "2026-07-05T10:00:00+09:00", "标题甲"),
                &l,
            )
            .await
            .unwrap();
        // 标题变 → 新会话
        let b = folder
            .fold_frame(
                &db,
                &frame("b.jpg", "2026-07-05T10:00:30+09:00", "标题乙"),
                &l,
            )
            .await
            .unwrap();
        assert_ne!(a, b);
        // 同标题但断流 6 分钟 → 新会话
        let c = folder
            .fold_frame(
                &db,
                &frame("c.jpg", "2026-07-05T10:06:31+09:00", "标题乙"),
                &l,
            )
            .await
            .unwrap();
        assert_ne!(b, c);
    }

    #[tokio::test]
    async fn short_lines_filtered() {
        let db = MemoryDb::open_in_memory().await.unwrap();
        let mut folder = Folder::default();
        let id = folder
            .fold_frame(
                &db,
                &frame("a.jpg", "2026-07-05T10:00:00+09:00", "页"),
                &lines(&["OK", "五个字不够", "这一行有六个字"]),
            )
            .await
            .unwrap();
        let text = session_text(&db, id).await;
        assert_eq!(text.lines().count(), 1);
        assert!(text.contains("这一行有六个字"));
    }
}
