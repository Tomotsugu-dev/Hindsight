//! 搜屏幕文字：找出文字连同标题里有全部关键词的段。搜索页和对话的工具都用它。

use chrono::NaiveDate;
use rusqlite::Connection;

use crate::ai::tools::{fts_literal, like_pattern};
use crate::storage::{DbResult, SqliteResultExt};

/// 关键词至少有这么多字，才能用全文索引找。更短的只能一段段比对。
const FTS_MIN_CHARS: usize = 3;

/// 只要哪些段。
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SessionFilter {
    /// 只要这些进程的段。
    pub apps: Vec<String>,
    /// 不要这些窗口：进程名一样，标题里有这个词。英文不分大小写。
    pub hidden_windows: Vec<(String, String)>,
}

/// 命中的一段屏幕文字。
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSpan {
    pub id: i64,
    /// 进程名。
    pub app_id: String,
    pub title: String,
    pub started_ts: String,
    pub ended_ts: String,
}

/// 找出文字连同标题里有全部关键词的段，从近到远。
///
/// 搜索页不传 filter；工具传可见范围、应用和分类。
/// 每个关键词出现在这一段的文字里，或者它的窗口标题里，都算。英文不分大小写。
pub fn find_sessions(
    conn: &Connection,
    words: &[String],
    range: Option<(NaiveDate, NaiveDate)>,
    filter: Option<&SessionFilter>,
) -> DbResult<Vec<SessionSpan>> {
    let mut conds = Vec::new();
    let mut binds = Vec::new();
    if let Some((from, to)) = range {
        conds.push("s.local_date BETWEEN ? AND ?".to_string());
        binds.extend([from.to_string(), to.to_string()]);
    }
    // 长词写在前面：一段没过全文索引，就不用再逐字比对短词
    let (long, short): (Vec<&String>, Vec<&String>) = words
        .iter()
        .partition(|w| w.chars().count() >= FTS_MIN_CHARS);
    for w in long {
        conds.push(
            "(s.id IN (SELECT rowid FROM text_sessions_fts WHERE text_sessions_fts MATCH ?)
              OR s.title LIKE ? ESCAPE '\\')"
                .to_string(),
        );
        binds.extend([fts_literal(std::slice::from_ref(w)), like_pattern(w)]);
    }
    for w in short {
        conds.push("(s.text LIKE ? ESCAPE '\\' OR s.title LIKE ? ESCAPE '\\')".to_string());
        binds.extend([like_pattern(w), like_pattern(w)]);
    }
    if let Some(f) = filter {
        let marks = vec!["?"; f.apps.len()].join(", ");
        conds.push(format!("COALESCE(s.app_id, '') IN ({marks})"));
        binds.extend(f.apps.iter().cloned());
        for (app, word) in &f.hidden_windows {
            conds.push(
                "NOT (lower(trim(COALESCE(s.app_id, ''))) = lower(trim(?))
                      AND COALESCE(s.title, '') LIKE ? ESCAPE '\\')"
                    .to_string(),
            );
            binds.extend([app.clone(), like_pattern(word.trim())]);
        }
    }
    let wheres = if conds.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conds.join(" AND "))
    };
    let sql = format!(
        "SELECT s.id, COALESCE(s.app_id, ''), COALESCE(s.title, ''), s.started_ts, s.ended_ts
         FROM text_sessions s
         {wheres}
         ORDER BY s.started_ts DESC, s.id DESC"
    );
    let mut stmt = conn.prepare(&sql).db()?;
    let spans = stmt
        .query_map(rusqlite::params_from_iter(binds.iter()), |r| {
            Ok(SessionSpan {
                id: r.get(0)?,
                app_id: r.get(1)?,
                title: r.get(2)?,
                started_ts: r.get(3)?,
                ended_ts: r.get(4)?,
            })
        })
        .db()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .db()?;
    Ok(spans)
}

/// 记忆库里有屏幕文字的进程名。没有进程名的段算作空字符串。
///
/// 工具用它算出 [`SessionFilter::apps`]：哪些进程的段能给 AI 看。
pub fn session_apps(conn: &Connection) -> DbResult<Vec<String>> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT COALESCE(app_id, '') FROM text_sessions")
        .db()?;
    let apps = stmt
        .query_map([], |r| r.get(0))
        .db()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .db()?;
    Ok(apps)
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use super::*;
    use crate::memory::MemoryDb;

    /// 一段屏幕文字：开始时刻写到分钟，进程名、标题可以没有。
    struct Seg<'a> {
        start: &'a str,
        app: Option<&'a str>,
        title: Option<&'a str>,
        text: &'a str,
    }

    fn seg<'a>(start: &'a str, app: &'a str, title: &'a str, text: &'a str) -> Seg<'a> {
        Seg {
            start,
            app: Some(app),
            title: Some(title),
            text,
        }
    }

    async fn db_with(segs: &[Seg<'_>]) -> MemoryDb {
        let db = MemoryDb::open_in_memory().await.unwrap();
        let rows: Vec<_> = segs
            .iter()
            .map(|s| {
                (
                    s.start[..10].to_string(),
                    format!("{}:00+09:00", s.start),
                    s.app.map(str::to_string),
                    s.title.map(str::to_string),
                    s.text.to_string(),
                )
            })
            .collect();
        db.0.call(move |c| {
            for (date, ts, app, title, text) in rows {
                c.execute(
                    "INSERT INTO text_sessions(local_date, started_ts, ended_ts, app_id, title, text)
                     VALUES (?1, ?2, ?2, ?3, ?4, ?5)",
                    params![date, ts, app, title, text],
                )
                .db()?;
            }
            Ok(())
        })
        .await
        .unwrap();
        db
    }

    /// 命中的段的标题，从近到远。
    async fn titles(
        db: &MemoryDb,
        words: &[&str],
        range: Option<(NaiveDate, NaiveDate)>,
        filter: Option<SessionFilter>,
    ) -> Vec<String> {
        let words: Vec<String> = words.iter().map(|w| w.to_string()).collect();
        db.0.call(move |c| find_sessions(c, &words, range, filter.as_ref()))
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.title)
            .collect()
    }

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, d).unwrap()
    }

    #[tokio::test]
    async fn every_word_must_be_in_the_text_or_the_title() {
        let db = db_with(&[
            seg("2026-07-01T09:00", "WeChat", "财务群", "请提交报销单"),
            seg("2026-07-01T10:00", "WeChat", "家人群", "报销"),
            seg("2026-07-01T11:00", "WeChat", "财务群", "周末聚餐"),
        ])
        .await;

        // 「财务群」在标题里，「报销」在文字里
        assert_eq!(
            titles(&db, &["财务群", "报销"], None, None).await,
            ["财务群"]
        );
    }

    #[tokio::test]
    async fn returns_the_spans_newest_first() {
        let db = db_with(&[
            seg("2026-07-01T09:00", "app", "一", "报销"),
            seg("2026-07-03T09:00", "app", "三", "报销"),
            seg("2026-07-02T09:00", "app", "二", "报销"),
        ])
        .await;

        let words = vec!["报销".to_string()];
        let spans =
            db.0.call(move |c| find_sessions(c, &words, None, None))
                .await
                .unwrap();
        let got: Vec<&str> = spans.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(got, ["三", "二", "一"]);
        assert_eq!(spans[0].app_id, "app");
        assert_eq!(spans[0].started_ts, "2026-07-03T09:00:00+09:00");
        assert_eq!(spans[0].ended_ts, spans[0].started_ts);
    }

    #[tokio::test]
    async fn long_and_short_words_both_ignore_english_case() {
        let db = db_with(&[
            seg("2026-07-01T09:00", "chrome", "订单", "Keychron K3 已发货"),
            seg("2026-07-01T10:00", "chrome", "别的", "Keychron Q1"),
            seg("2026-07-01T11:00", "chrome", "KEYCHRON 官网", "首页"),
        ])
        .await;

        // 三个字以上走全文索引，更短的逐段比对
        assert_eq!(titles(&db, &["KEYCHRON", "k3"], None, None).await, ["订单"]);
        // 长词只在标题里也算
        assert_eq!(
            titles(&db, &["keychron", "首页"], None, None).await,
            ["KEYCHRON 官网"]
        );
    }

    #[tokio::test]
    async fn wildcards_in_words_match_only_themselves() {
        let db = db_with(&[
            seg("2026-07-01T09:00", "app", "一", "进度 100% 完成"),
            seg("2026-07-01T10:00", "app", "二", "共 1000 条"),
            seg("2026-07-01T11:00", "app", "三", "a_b"),
            seg("2026-07-01T12:00", "app", "四", "axb"),
        ])
        .await;

        assert_eq!(titles(&db, &["100%"], None, None).await, ["一"]);
        assert_eq!(titles(&db, &["a_b"], None, None).await, ["三"]);
    }

    #[tokio::test]
    async fn the_range_limits_the_dates() {
        let db = db_with(&[
            seg("2026-07-01T09:00", "app", "一", "报销"),
            seg("2026-07-02T09:00", "app", "二", "报销"),
            seg("2026-07-03T09:00", "app", "三", "报销"),
        ])
        .await;

        let range = Some((day(2), day(3)));
        assert_eq!(titles(&db, &["报销"], range, None).await, ["三", "二"]);
    }

    #[tokio::test]
    async fn the_filter_keeps_only_its_apps() {
        let db = db_with(&[
            seg("2026-07-01T09:00", "Code", "main.rs", "报销"),
            seg("2026-07-01T10:00", "notepad", "笔记", "报销"),
            Seg {
                start: "2026-07-01T11:00",
                app: None,
                title: None,
                text: "报销",
            },
        ])
        .await;

        let only = |apps: &[&str]| SessionFilter {
            apps: apps.iter().map(|a| a.to_string()).collect(),
            hidden_windows: Vec::new(),
        };
        assert_eq!(
            titles(&db, &["报销"], None, Some(only(&["Code"]))).await,
            ["main.rs"]
        );
        // 没有进程名的段按空字符串比
        assert_eq!(titles(&db, &["报销"], None, Some(only(&[""]))).await, [""]);
        // 一个应用都不给就什么都不要
        assert!(titles(&db, &["报销"], None, Some(only(&[])))
            .await
            .is_empty());
        // 不给 filter 就都要
        assert_eq!(titles(&db, &["报销"], None, None).await.len(), 3);
    }

    #[tokio::test]
    async fn the_filter_hides_windows_by_app_and_title_word() {
        let db = db_with(&[
            seg("2026-07-01T09:00", "Terminal", "Download videos", "报销"),
            seg("2026-07-01T10:00", "Terminal", "vim", "报销"),
            seg("2026-07-01T11:00", "iTerm", "download", "报销"),
            Seg {
                start: "2026-07-01T12:00",
                app: Some("Terminal"),
                title: None,
                text: "报销",
            },
        ])
        .await;

        let filter = SessionFilter {
            apps: vec!["Terminal".into(), "iTerm".into()],
            // 进程名和标题里的词都不分大小写，前后的空白不算
            hidden_windows: vec![(" terminal ".into(), " DOWNLOAD ".into())],
        };
        // 没有标题的段不会被这条规则去掉
        assert_eq!(
            titles(&db, &["报销"], None, Some(filter)).await,
            ["", "download", "vim"]
        );
    }

    #[tokio::test]
    async fn session_apps_lists_each_app_once() {
        let db = db_with(&[
            seg("2026-07-01T09:00", "Code", "a", "x"),
            seg("2026-07-01T10:00", "Code", "b", "y"),
            Seg {
                start: "2026-07-01T11:00",
                app: None,
                title: None,
                text: "z",
            },
        ])
        .await;

        let mut apps = db.0.call(|c| session_apps(c)).await.unwrap();
        apps.sort();
        assert_eq!(apps, ["", "Code"]);

        let empty = MemoryDb::open_in_memory().await.unwrap();
        assert!(empty.0.call(|c| session_apps(c)).await.unwrap().is_empty());
    }
}
