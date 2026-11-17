//! search_text：在屏幕文字和窗口标题里搜关键词。

use chrono::NaiveDate;
use rusqlite::params;

use super::{
    coverage_line, fts_literal, like_pattern, truncate, ChatLang, Citation, ToolCtx, ToolOutput,
    ACTIVITY_WHERE, RESULT_CHAR_BUDGET,
};
use crate::error::Result;
use crate::repo::sql::FROM_ACTIVITY_GROUP;
use crate::storage::SqliteResultExt;

/// 搜索命中数上限(服务端硬夹紧,模型无法调大)
const SEARCH_LIMIT: usize = 8;
/// 搜索的窗口标题层命中显示上限(屏幕文字索引之外的兜底证据层)
const TITLE_LIMIT: usize = 8;

pub(super) async fn search_text(
    ctx: &ToolCtx,
    keywords: &[String],
    range: Option<(NaiveDate, NaiveDate)>,
    next_citation: usize,
    lang: ChatLang,
) -> Result<ToolOutput> {
    let fts = fts_literal(keywords);
    let first_kw = like_pattern(&keywords[0]);
    let (from, to) = match range {
        Some((f, t)) => (f.to_string(), t.to_string()),
        None => ("0000-00-00".into(), "9999-99-99".into()),
    };
    let coverage = coverage_line(ctx, &from, &to, lang).await?;
    let (from2, to2) = (from.clone(), to.clone());
    let (total, rows) = ctx
        .mem
        .call(move |conn| {
            // 总命中数:必须让模型知道命中规模(8 条窗口外还有没有东西),
            // 否则它会把"前 8 条"当成"只有 8 条"。
            let total: i64 = conn
                .query_row(
                    "SELECT COUNT(*)
                     FROM text_sessions_fts
                     JOIN text_sessions s ON s.id = text_sessions_fts.rowid
                     WHERE text_sessions_fts MATCH ?1
                       AND s.local_date BETWEEN ?2 AND ?3",
                    params![fts, from, to],
                    |r| r.get(0),
                )
                .db()?;
            let mut stmt = conn
                .prepare(
                    "SELECT s.id, COALESCE(s.app_id,''), COALESCE(s.title,''),
                            s.started_ts, s.ended_ts,
                            snippet(text_sessions_fts, 0, '<<', '>>', '…', 16)
                     FROM text_sessions_fts
                     JOIN text_sessions s ON s.id = text_sessions_fts.rowid
                     WHERE text_sessions_fts MATCH ?1
                       AND s.local_date BETWEEN ?2 AND ?3
                     ORDER BY rank LIMIT ?4",
                )
                .db()?;
            let hits = stmt
                .query_map(params![fts, from, to, SEARCH_LIMIT as i64], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                    ))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            // 每条命中补证据帧:该会话里第一条含关键词的行的首现帧
            let mut out = Vec::with_capacity(hits.len());
            for (id, app, title, started, ended, snippet) in hits {
                let frame: Option<(String, String)> = conn
                    .query_row(
                        "SELECT first_path, first_ts FROM session_lines
                         WHERE session_id = ?1 AND text LIKE ?2 ESCAPE '\\' LIMIT 1",
                        params![id, first_kw],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .ok();
                out.push((id, app, title, started, ended, snippet, frame));
            }
            Ok((total, out))
        })
        .await?;

    // 标题层:窗口标题 LIKE 全部关键词(AND)。屏幕文字索引之外的兜底证据——
    // 没开截图/OCR 的用户靠它回答"我用没用过 X";有索引的用户靠它补足
    // 索引空窗(电池暂停、待识别积压)里的记录。
    let title_likes: Vec<String> = keywords.iter().map(|k| like_pattern(k)).collect();
    let (title_total, title_rows) = ctx
        .main
        .call(move |conn| {
            let like_sql = (0..title_likes.len())
                .map(|i| format!("a.window_title LIKE ?{} ESCAPE '\\'", i + 3))
                .collect::<Vec<_>>()
                .join(" AND ");
            let mut bind: Vec<&dyn rusqlite::ToSql> = vec![&from2, &to2];
            for l in &title_likes {
                bind.push(l);
            }
            let total: i64 = conn
                .query_row(
                    &format!(
                        "SELECT COUNT(*) {FROM_ACTIVITY_GROUP} {ACTIVITY_WHERE} AND {like_sql}"
                    ),
                    bind.as_slice(),
                    |r| r.get(0),
                )
                .db()?;
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT COALESCE(g.display_name, a.process_name),
                            COALESCE(a.window_title,''),
                            a.started_at, a.ended_at, NULLIF(a.screenshot_path,'')
                     {FROM_ACTIVITY_GROUP} {ACTIVITY_WHERE} AND {like_sql}
                     ORDER BY a.started_at DESC LIMIT {TITLE_LIMIT}"
                ))
                .db()?;
            let hits = stmt
                .query_map(bind.as_slice(), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                    ))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok((total, hits))
        })
        .await?;
    // 跨层去重:标题命中若落在已返回的屏幕文字会话时间段内,是同一证据的弱化版,剔除。
    // 两库时间戳同为本地时区 RFC3339(回填即原样搬运),字符串比较即可。
    let title_rows: Vec<_> = title_rows
        .into_iter()
        .filter(|(_, _, s, _, _)| {
            !rows.iter().any(|(_, _, _, l2s, l2e, _, _)| {
                s.as_str() >= l2s.as_str() && s.as_str() <= l2e.as_str()
            })
        })
        .collect();

    let mut citations = Vec::new();
    let mut lines = Vec::new();
    for (i, (_id, app, title, started, ended, snippet, frame)) in rows.iter().enumerate() {
        let idx = next_citation + i;
        lines.push(format!(
            "[{idx}] {} | {} | {} ~ {} | 片段: {}",
            app,
            truncate(title, 40),
            &started[..16.min(started.len())],
            &ended[..16.min(ended.len())],
            truncate(snippet, 120)
        ));
        citations.push(Citation {
            index: idx,
            app: app.clone(),
            title: title.clone(),
            started_ts: started.clone(),
            ended_ts: ended.clone(),
            frame_path: frame.as_ref().map(|(p, _)| p.clone()),
        });
    }
    let mut title_lines = Vec::new();
    for (j, (app, title, started, ended, shot)) in title_rows.iter().enumerate() {
        let idx = next_citation + rows.len() + j;
        title_lines.push(format!(
            "[{idx}] {} | {} | {} ~ {}",
            app,
            truncate(title, 60),
            &started[..16.min(started.len())],
            &ended[..16.min(ended.len())],
        ));
        citations.push(Citation {
            index: idx,
            app: app.clone(),
            title: title.clone(),
            started_ts: started.clone(),
            ended_ts: ended.clone(),
            frame_path: shot.clone(),
        });
    }

    // 组装:覆盖行永远在最前;两层各自报数分节列出;全空才是"没有命中"。
    let mut sections = vec![coverage];
    if !lines.is_empty() {
        sections.push(format!(
            "{}\n{}",
            lang.search_header(total, lines.len()),
            lines.join("\n")
        ));
    }
    if !title_lines.is_empty() {
        sections.push(format!(
            "{}\n{}",
            lang.search_title_header(title_total, title_lines.len()),
            title_lines.join("\n")
        ));
    }
    if lines.is_empty() && title_lines.is_empty() {
        sections.push(lang.search_no_hit().to_string());
    }
    let content = truncate(&sections.join("\n"), RESULT_CHAR_BUDGET);
    Ok(ToolOutput { content, citations })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::tools::test_util::{ctx_with, day};
    use crate::ai::tools::{execute, Query};

    /// 跨层去重:标题命中落在已返回的屏幕文字会话时间段内 → 剔除,段外保留。
    #[tokio::test]
    async fn title_hit_inside_returned_session_span_dropped() {
        let ctx = ctx_with(
            "INSERT INTO text_sessions(local_date, started_ts, ended_ts, app_id, title, text)
             VALUES ('2026-07-08','2026-07-08T09:00:00+09:00',
                     '2026-07-08T10:00:00+09:00','chrome','评测','keychron 键盘评测正文');",
            "INSERT INTO activities(started_at, ended_at, duration_secs,
                 local_date, local_hour, process_name, window_title) VALUES
             ('2026-07-08T09:30:00+09:00','2026-07-08T09:40:00+09:00',600,'2026-07-08',9,
              'chrome','keychron 评测页'),
             ('2026-07-08T11:00:00+09:00','2026-07-08T11:10:00+09:00',600,'2026-07-08',11,
              'chrome','keychron 下单页');",
        )
        .await;
        let out = execute(
            &ctx,
            &Query::SearchText {
                keywords: vec!["keychron".into()],
                range: Some(day()),
            },
            1,
            ChatLang::ZhHans,
        )
        .await
        .unwrap();
        // FTS 层 1 条 + 标题层只剩段外的 11:00 那条
        assert!(out.content.contains("keychron 下单页"), "{}", out.content);
        assert!(!out.content.contains("keychron 评测页"), "{}", out.content);
        assert_eq!(out.citations.len(), 2);
        // 编号连续:FTS 层 [1],标题层 [2]
        assert_eq!(out.citations[1].index, 2);
        assert_eq!(out.citations[1].title, "keychron 下单页");
    }

    #[tokio::test]
    async fn search_discloses_total_hits_beyond_window() {
        let mut sql = String::new();
        for i in 0..20 {
            sql.push_str(&format!(
                "INSERT INTO text_sessions(local_date, started_ts, ended_ts, app_id, title, text)
                 VALUES ('2026-07-08','2026-07-08T09:{i:02}:00+09:00',
                         '2026-07-08T09:{i:02}:30+09:00','A','t{i}','keychron 订单第 {i} 条记录');\n"
            ));
        }
        let mem: &'static str = Box::leak(sql.into_boxed_str());
        let ctx = ctx_with(mem, "").await;
        let out = execute(
            &ctx,
            &Query::SearchText {
                keywords: vec!["keychron".into()],
                range: None,
            },
            1,
            ChatLang::ZhHans,
        )
        .await
        .unwrap();
        assert!(out.content.contains("共 20 条命中"), "{}", out.content);
        assert!(out.content.contains("前 8 条"), "{}", out.content);
        assert_eq!(out.citations.len(), SEARCH_LIMIT);
    }
}
