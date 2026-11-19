//! search_screen_text：按关键词搜屏幕上识别出的文字。回答 X 在哪些窗口的屏幕上出现过。

use std::collections::HashSet;
use std::fmt::Display;
use std::sync::LazyLock;

use chrono::NaiveDate;
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::Value;

use super::{quote_keywords, write_list, Row, SearchParams};
use crate::ai::model::ToolSpec;
use crate::ai::tools::kinds::params::spec;
use crate::ai::tools::kinds::Tool;
use crate::ai::tools::text::{load, Text};
use crate::ai::tools::{
    like_pattern, truncate, ChatLang, Citation, ToolCtx, ToolError, ToolOutput,
};
use crate::error::Result;
use crate::memory::search::{find_sessions, session_apps};
use crate::memory::snippet::snippet;
use crate::storage::{DbResult, SqliteResultExt};

/// search_screen_text 检查过的参数。
#[derive(Debug, Clone, PartialEq)]
pub struct OcrQuery(pub SearchParams);

/// 每行的标题最多几个字。
const TITLE_MAX_CHARS: usize = 60;
/// 片段最多几个字，不算两头的省略号。
const SNIPPET_MAX_CHARS: usize = 120;
/// 片段在关键词前面留几个字。
const SNIPPET_BEFORE_CHARS: usize = 20;

/// `lang/ocr.json` 里的句子，都写给模型看。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OcrText {
    // 进工具说明
    /// 整个工具的说明。
    description: Text,
    /// 关键词参数的说明。
    keywords: Text,

    // 进工具结果
    /// 开头：几个窗口的屏幕上出现过关键词。说明只是出现过，不算时间。一个都没列时用。
    head: Text,
    /// 同 `head`，后面接着写按最近一次列了几个。
    head_with_list: Text,
    /// 没有命中。
    none: Text,
    /// 记忆库里一段屏幕文字都没有。
    no_index: Text,
}

const TEXT_JSON: &str = include_str!("lang/ocr.json");

static TEXT: LazyLock<OcrText> = LazyLock::new(|| load(TEXT_JSON));

impl Tool for OcrQuery {
    const NAME: &'static str = "search_screen_text";

    fn spec(lang: ChatLang) -> ToolSpec {
        spec(
            Self::NAME,
            TEXT.description.get(lang),
            SearchParams::schema(TEXT.keywords.get(lang), lang),
        )
    }

    fn parse(
        args: &Value,
        today: NaiveDate,
        lang: ChatLang,
    ) -> std::result::Result<Self, ToolError> {
        SearchParams::parse(args, today, lang).map(Self)
    }

    async fn execute(
        &self,
        ctx: &ToolCtx,
        next_citation: usize,
        max_chars: usize,
        lang: ChatLang,
    ) -> Result<ToolOutput> {
        let Some(found) = find(ctx, &self.0).await? else {
            return Ok(ToolOutput {
                content: TEXT.no_index.get(lang).to_string(),
                citations: Vec::new(),
            });
        };
        Ok(render(&self.0, &found, next_citation, max_chars, lang))
    }
}

/// 查到的结果，写成文字之前。
#[derive(Debug)]
struct Found {
    /// 要列的窗口，最近的在前，最多 `limit` 个。
    windows: Vec<ScreenWindow>,
    /// 全部命中的窗口数。
    count: usize,
}

/// 屏幕上出现过关键词的一个窗口。
#[derive(Debug)]
struct ScreenWindow {
    app: String,
    title: String,
    /// 片段。
    line: String,
    /// 片段那一行第一次出现时的截图。证据卡用。
    screenshot: Option<String>,
    /// 这张截图拍下的时刻。没有截图时是这一段开始的时刻。证据卡用。
    screenshot_ts: String,
}

/// 查记忆库里屏幕文字或标题有全部关键词、AI 能看到的段，一个窗口一行，最近的在前。
///
/// 记忆库里一段屏幕文字都没有时，返回 `None`。
async fn find(ctx: &ToolCtx, p: &SearchParams) -> Result<Option<Found>> {
    let processes = ctx.mem.call(|conn| session_apps(conn)).await?;
    if processes.is_empty() {
        return Ok(None);
    }
    // 可见范围、应用和分类由 AiDataPolicy 写成条件，在 SQL 里筛
    let policy = ctx.policy().await?;
    let filter = policy.screen_filter(processes, &p.apps, &p.categories);
    let (keywords, range) = (p.keywords.clone(), p.range);
    let spans = ctx
        .mem
        .call(move |conn| find_sessions(conn, &keywords, range, Some(&filter)))
        .await?;

    // 按应用加标题分组：段从近到远，每组先遇到的就是最近一段。
    // 同一个分组的几个进程名算一个应用
    let mut seen = HashSet::new();
    let mut picked = Vec::new();
    for span in spans {
        let app = policy.display_name(&span.app_id).to_string();
        if seen.insert((app.clone(), span.title.clone())) {
            picked.push((app, span));
        }
    }
    let count = picked.len();
    picked.truncate(p.limit);

    let keywords = p.keywords.clone();
    let sessions: Vec<i64> = picked.iter().map(|(_, span)| span.id).collect();
    let lines = ctx
        .mem
        .call(move |conn| {
            sessions
                .iter()
                .map(|&id| line_of(conn, id, &keywords))
                .collect::<DbResult<Vec<_>>>()
        })
        .await?;
    let windows = picked
        .into_iter()
        .zip(lines)
        .map(|((app, span), line)| {
            let (screenshot, screenshot_ts) = match line.shot {
                Some((path, ts)) => (Some(path), ts),
                None => (None, span.started_ts),
            };
            ScreenWindow {
                app,
                title: span.title,
                line: line.text,
                screenshot,
                screenshot_ts,
            }
        })
        .collect();
    Ok(Some(Found { windows, count }))
}

/// 一段里的片段。
struct Line {
    text: String,
    /// 这一行第一次出现时的截图和时刻。
    shot: Option<(String, String)>,
}

/// 找一段的片段：第一个出现在文字里的关键词，它所在的那一行，以它为中心截一段。
///
/// 关键词都只在标题里时，用这一段的第一行。
fn line_of(conn: &rusqlite::Connection, session: i64, keywords: &[String]) -> DbResult<Line> {
    let first_line = |pattern: &str| {
        conn.query_row(
            "SELECT text, first_path, first_ts FROM session_lines
             WHERE session_id = ?1 AND text LIKE ?2 ESCAPE '\\'
             ORDER BY line_no LIMIT 1",
            params![session, pattern],
            |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .db()
    };
    for k in keywords {
        if let Some((text, path, ts)) = first_line(&like_pattern(k))? {
            let after = SNIPPET_MAX_CHARS.saturating_sub(SNIPPET_BEFORE_CHARS + k.chars().count());
            return Ok(Line {
                text: snippet(&text, k, SNIPPET_BEFORE_CHARS, after),
                shot: Some((path, ts)),
            });
        }
    }
    Ok(match first_line("%")? {
        Some((text, path, ts)) => Line {
            text: snippet(&text, "", 0, SNIPPET_MAX_CHARS),
            shot: Some((path, ts)),
        },
        None => Line {
            text: String::new(),
            shot: None,
        },
    })
}

/// 写成给模型看的文字，列出来的每个窗口配一条证据：片段那一行第一次出现时的截图。
fn render(
    p: &SearchParams,
    found: &Found,
    next_citation: usize,
    max_chars: usize,
    lang: ChatLang,
) -> ToolOutput {
    let kw = quote_keywords(&p.keywords, lang);
    if found.count == 0 {
        return ToolOutput {
            content: TEXT.none.fill(lang, &[("kw", &kw)]),
            citations: Vec::new(),
        };
    }
    // 开头和「按最近一次列前 N 个」写在同一行
    let head = |n: usize| {
        let line = if n > 0 {
            let args: [(&str, &dyn Display); 3] = [("kw", &kw), ("count", &found.count), ("n", &n)];
            TEXT.head_with_list.fill(lang, &args)
        } else {
            TEXT.head
                .fill(lang, &[("kw", &kw), ("count", &found.count)])
        };
        vec![line]
    };
    let rows = found
        .windows
        .iter()
        .map(|w| Row {
            line: format!(
                "{} | {}｜{}",
                w.app,
                truncate(&w.title, TITLE_MAX_CHARS),
                w.line
            ),
            // 卡片上的时间是这张截图拍下的时刻，不是看了多久
            citation: Citation {
                index: 0,
                app: w.app.clone(),
                title: w.title.clone(),
                started_ts: w.screenshot_ts.clone(),
                ended_ts: w.screenshot_ts.clone(),
                frame_path: w.screenshot.clone(),
            },
        })
        .collect();
    write_list(head, rows, found.count, next_citation, max_chars, lang)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::more;
    use super::*;
    use crate::ai::tools::test_util::real_ctx;
    use crate::ai::tools::text::assert_same_placeholders;
    use crate::repo::test_util::fresh_test_pool;
    use crate::storage::DbPool;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 10).unwrap()
    }

    /// 主库：Code 和 Code.exe 在「编程」的同一个分组里，显示名是 Visual Studio Code；
    /// secret 不计入统计；Signal 隐私；忽略 Terminal 里标题有 download 的窗口。
    async fn main_db() -> DbPool {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|c| {
                c.execute_batch(
                    r#"INSERT INTO app_groups(id, display_name, category_id) VALUES
                         ('g-code', 'Visual Studio Code', 'code'),
                         ('g-secret', 'Secret', 'hidden'),
                         ('g-signal', 'Signal', 'private');
                     INSERT INTO app_group_members(process_name, group_id) VALUES
                         ('Code.exe', 'g-code'), ('Code', 'g-code'),
                         ('secret.exe', 'g-secret'), ('Signal', 'g-signal');
                     UPDATE settings_store SET data = json_set(data, '$.ignoreRules',
                         json('[{"processName": "Terminal", "titleKeyword": "download"}]'))
                     WHERE id = 1;"#,
                )
                .db()
            })
            .await
            .unwrap();
        pool
    }

    /// 加一段屏幕文字。`start` 写到分钟。第 n 行第一次出现在截图 shots/<段>-<n>.jpg 上。
    async fn add(ctx: &ToolCtx, start: &str, app: &str, title: &str, lines: &[&str]) {
        let ts = format!("{start}:00+09:00");
        let date = start[..10].to_string();
        let (app, title) = (app.to_string(), title.to_string());
        let lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        ctx.mem
            .call(move |c| {
                c.execute(
                    "INSERT INTO text_sessions(local_date, started_ts, ended_ts, app_id, title, text)
                     VALUES (?1, ?2, ?2, ?3, ?4, ?5)",
                    params![date, ts, app, title, lines.join("\n")],
                )
                .db()?;
                let id = c.last_insert_rowid();
                for (n, line) in lines.iter().enumerate() {
                    c.execute(
                        "INSERT INTO session_lines(session_id, line_no, text, first_path, first_ts)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![id, n as i64, line, format!("shots/{id}-{n}.jpg"), ts],
                    )
                    .db()?;
                }
                Ok(())
            })
            .await
            .unwrap();
    }

    fn query(args: Value) -> OcrQuery {
        OcrQuery::parse(&args, today(), ChatLang::ZhHans).unwrap()
    }

    async fn found(ctx: &ToolCtx, args: Value) -> Found {
        find(ctx, &query(args).0).await.unwrap().unwrap()
    }

    fn titles(f: &Found) -> Vec<&str> {
        f.windows.iter().map(|w| w.title.as_str()).collect()
    }

    async fn output(ctx: &ToolCtx, args: Value, max_chars: usize) -> ToolOutput {
        query(args)
            .execute(ctx, 1, max_chars, ChatLang::ZhHans)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn one_row_per_window_newest_first() {
        let ctx = real_ctx(&main_db().await).await;
        add(
            &ctx,
            "2026-07-01T09:00",
            "WeChat",
            "财务群",
            &["请在周五前提交报销单"],
        )
        .await;
        add(
            &ctx,
            "2026-07-08T15:00",
            "WeChat",
            "财务群",
            &["闲聊", "报销 截止 周五"],
        )
        .await;
        add(
            &ctx,
            "2026-07-05T10:00",
            "EXCEL.EXE",
            "报销单_7月.xlsx",
            &["合计 3,280 元"],
        )
        .await;
        add(&ctx, "2026-07-06T10:00", "notepad", "笔记", &["没有关键词"]).await;

        let f = found(&ctx, json!({"keywords": ["报销"]})).await;
        assert_eq!(f.count, 2);
        assert_eq!(titles(&f), ["财务群", "报销单_7月.xlsx"]);
        // 片段取最近一段里有关键词的那一行，证据是这一行第一次出现时的截图
        let chat = &f.windows[0];
        assert_eq!(chat.line, "报销 截止 周五");
        assert_eq!(chat.screenshot.as_deref(), Some("shots/2-1.jpg"));
        assert_eq!(chat.screenshot_ts, "2026-07-08T15:00:00+09:00");
    }

    #[tokio::test]
    async fn a_keyword_can_be_in_the_title_or_the_text() {
        let ctx = real_ctx(&main_db().await).await;
        add(
            &ctx,
            "2026-07-01T09:00",
            "WeChat",
            "财务群",
            &["请提交报销单"],
        )
        .await;
        add(&ctx, "2026-07-01T10:00", "WeChat", "家人群", &["报销"]).await;

        let f = found(&ctx, json!({"keywords": ["财务群", "报销"]})).await;
        assert_eq!(titles(&f), ["财务群"]);
    }

    #[tokio::test]
    async fn long_and_short_keywords_both_ignore_english_case() {
        let ctx = real_ctx(&main_db().await).await;
        add(
            &ctx,
            "2026-07-01T09:00",
            "chrome",
            "订单",
            &["Keychron K3 已发货"],
        )
        .await;
        add(&ctx, "2026-07-01T10:00", "chrome", "别的", &["Keychron Q1"]).await;

        // 三个字以上走全文索引，更短的逐段比对，结果一样不分大小写
        let f = found(&ctx, json!({"keywords": ["KEYCHRON", "k3"]})).await;
        assert_eq!(titles(&f), ["订单"]);
        // 只在标题里的长关键词也算
        let f = found(&ctx, json!({"keywords": ["Keychron", "订单"]})).await;
        assert_eq!(titles(&f), ["订单"]);
    }

    #[tokio::test]
    async fn wildcards_in_keywords_match_only_themselves() {
        let ctx = real_ctx(&main_db().await).await;
        add(&ctx, "2026-07-01T09:00", "app", "一", &["进度 100% 完成"]).await;
        add(&ctx, "2026-07-01T10:00", "app", "二", &["共 1000 条"]).await;
        add(&ctx, "2026-07-01T11:00", "app", "三", &["a_b"]).await;
        add(&ctx, "2026-07-01T12:00", "app", "四", &["axb"]).await;

        assert_eq!(
            titles(&found(&ctx, json!({"keywords": ["100%"]})).await),
            ["一"]
        );
        assert_eq!(
            titles(&found(&ctx, json!({"keywords": ["a_b"]})).await),
            ["三"]
        );
    }

    #[tokio::test]
    async fn hides_hidden_private_and_ignored_windows() {
        let ctx = real_ctx(&main_db().await).await;
        add(&ctx, "2026-07-01T09:00", "secret.exe", "日记", &["报销"]).await;
        add(&ctx, "2026-07-01T10:00", "Signal", "聊天", &["报销"]).await;
        add(
            &ctx,
            "2026-07-01T11:00",
            "Terminal",
            "Download videos",
            &["报销"],
        )
        .await;
        add(&ctx, "2026-07-01T12:00", "Terminal", "vim", &["报销"]).await;

        let f = found(&ctx, json!({"keywords": ["报销"]})).await;
        assert_eq!(f.count, 1);
        assert_eq!(titles(&f), ["vim"]);
    }

    #[tokio::test]
    async fn processes_of_one_group_show_as_one_app() {
        let ctx = real_ctx(&main_db().await).await;
        add(
            &ctx,
            "2026-07-01T09:00",
            "Code.exe",
            "main.rs",
            &["报销 脚本 Windows"],
        )
        .await;
        add(
            &ctx,
            "2026-07-02T09:00",
            "Code",
            "main.rs",
            &["报销 脚本 macOS"],
        )
        .await;

        let f = found(&ctx, json!({"keywords": ["报销"]})).await;
        assert_eq!(f.count, 1);
        assert_eq!(f.windows[0].app, "Visual Studio Code");
        assert_eq!(f.windows[0].line, "报销 脚本 macOS");
    }

    #[tokio::test]
    async fn apps_and_categories_filter_with_the_main_db_settings() {
        let ctx = real_ctx(&main_db().await).await;
        add(&ctx, "2026-07-01T09:00", "Code.exe", "main.rs", &["报销"]).await;
        add(&ctx, "2026-07-01T10:00", "notepad", "笔记", &["报销"]).await;

        for app in ["visual studio", "code.exe"] {
            let f = found(&ctx, json!({"keywords": ["报销"], "apps": [app]})).await;
            assert_eq!(titles(&f), ["main.rs"], "{app}");
        }
        let f = found(&ctx, json!({"keywords": ["报销"], "categories": ["编程"]})).await;
        assert_eq!(titles(&f), ["main.rs"]);
        // 没分组的应用算「其他」
        let f = found(&ctx, json!({"keywords": ["报销"], "categories": ["其他"]})).await;
        assert_eq!(titles(&f), ["笔记"]);
    }

    #[tokio::test]
    async fn the_date_range_limits_the_sessions() {
        let ctx = real_ctx(&main_db().await).await;
        add(&ctx, "2026-07-01T09:00", "app", "一", &["报销 旧"]).await;
        add(&ctx, "2026-07-08T09:00", "app", "二", &["报销 新"]).await;

        let args =
            json!({"keywords": ["报销"], "date_from": "2026-07-01", "date_to": "2026-07-02"});
        assert_eq!(titles(&found(&ctx, args).await), ["一"]);
    }

    #[tokio::test]
    async fn the_snippet_centers_on_the_first_keyword_found_in_the_text() {
        let ctx = real_ctx(&main_db().await).await;
        let long = format!("{}报销单{}", "前".repeat(50), "后".repeat(200));
        add(&ctx, "2026-07-01T09:00", "app", "财务群", &["无关", &long]).await;

        // 「财务群」只在标题里，片段按「报销」找
        let f = found(&ctx, json!({"keywords": ["财务群", "报销"]})).await;
        let line = &f.windows[0].line;
        let expected = format!("…{}报销单{}…", "前".repeat(20), "后".repeat(97));
        assert_eq!(line, &expected);
        assert_eq!(line.chars().count(), SNIPPET_MAX_CHARS + 2);
        assert_eq!(f.windows[0].screenshot.as_deref(), Some("shots/1-1.jpg"));
    }

    #[tokio::test]
    async fn keywords_only_in_the_title_use_the_first_line() {
        let ctx = real_ctx(&main_db().await).await;
        add(
            &ctx,
            "2026-07-01T09:00",
            "app",
            "财务群",
            &["第一行", "第二行"],
        )
        .await;

        let f = found(&ctx, json!({"keywords": ["财务群"]})).await;
        assert_eq!(f.windows[0].line, "第一行");
        assert_eq!(f.windows[0].screenshot.as_deref(), Some("shots/1-0.jpg"));
    }

    #[tokio::test]
    async fn text_has_the_head_the_list_and_one_citation_per_line() {
        let ctx = real_ctx(&main_db().await).await;
        add(
            &ctx,
            "2026-07-01T09:00",
            "WeChat",
            "财务群",
            &["请提交报销单"],
        )
        .await;
        add(
            &ctx,
            "2026-07-02T09:00",
            "Code.exe",
            "main.rs",
            &["// 报销"],
        )
        .await;

        let out = output(&ctx, json!({"keywords": ["报销"]}), 4000).await;
        // 格式照 ocr.md 第 1 节的例子
        let expected = [
            "屏幕上出现过「报销」的窗口：2 个。只说明出现过，不算时间。按最近一次列前 2 个：",
            "[1] Visual Studio Code | main.rs｜// 报销",
            "[2] WeChat | 财务群｜请提交报销单",
        ];
        assert_eq!(out.content, expected.join("\n"));
        let c = &out.citations[1];
        assert_eq!((c.index, c.app.as_str()), (2, "WeChat"));
        assert_eq!(c.frame_path.as_deref(), Some("shots/1-0.jpg"));
        assert_eq!(c.started_ts, "2026-07-01T09:00:00+09:00");
        assert_eq!(c.ended_ts, c.started_ts);
    }

    #[tokio::test]
    async fn limit_lists_the_newest_and_says_how_many_are_left() {
        let ctx = real_ctx(&main_db().await).await;
        for day in 1..=5 {
            let start = format!("2026-07-0{day}T09:00");
            add(&ctx, &start, "app", &format!("第{day}天"), &["报销"]).await;
        }

        let out = output(&ctx, json!({"keywords": ["报销"], "limit": 2}), 4000).await;
        assert!(out.content.contains("：5 个"), "{}", out.content);
        assert!(out.content.contains("[1] app | 第5天") && out.content.contains("[2] app | 第4天"));
        assert!(out.content.ends_with(&more(3, ChatLang::ZhHans)));
        assert_eq!(out.citations.len(), 2);
    }

    #[tokio::test]
    async fn lists_fewer_when_the_text_would_be_too_long() {
        let ctx = real_ctx(&main_db().await).await;
        for day in 1..=5 {
            let start = format!("2026-07-0{day}T09:00");
            add(
                &ctx,
                &start,
                "app",
                &format!("第{day}天"),
                &[&format!("报销{}", "字".repeat(100))],
            )
            .await;
        }

        let max_chars = 300;
        let out = output(&ctx, json!({"keywords": ["报销"]}), max_chars).await;
        assert!(out.content.chars().count() <= max_chars, "{}", out.content);
        let shown = out.citations.len();
        assert!(0 < shown && shown < 5, "{shown}: {}", out.content);
        assert!(out.content.ends_with(&more(5 - shown, ChatLang::ZhHans)));
    }

    #[tokio::test]
    async fn without_room_for_any_window_the_head_has_no_list_part() {
        let ctx = real_ctx(&main_db().await).await;
        add(&ctx, "2026-07-01T09:00", "app", "笔记", &["报销"]).await;

        // 放得下开头和最后一行，放不下一个窗口
        let out = output(&ctx, json!({"keywords": ["报销"]}), 90).await;
        let expected = [
            "屏幕上出现过「报销」的窗口：1 个。只说明出现过，不算时间。",
            &more(1, ChatLang::ZhHans),
        ];
        assert_eq!(out.content, expected.join("\n"));
        assert!(out.citations.is_empty());
    }

    #[tokio::test]
    async fn no_hit_says_so() {
        let ctx = real_ctx(&main_db().await).await;
        add(&ctx, "2026-07-01T09:00", "app", "笔记", &["别的"]).await;

        let out = output(&ctx, json!({"keywords": ["报销"]}), 4000).await;
        assert_eq!(
            out.content,
            "屏幕上没有出现过「报销」。可以换更短的词、同义词或英文。"
        );
        assert!(out.citations.is_empty());
    }

    #[tokio::test]
    async fn an_empty_index_says_there_is_no_screen_text() {
        let ctx = real_ctx(&main_db().await).await;

        let out = output(&ctx, json!({"keywords": ["报销"]}), 4000).await;
        assert_eq!(
            out.content,
            "没有屏幕文字索引：屏幕记忆没开，或者截图还没识别。"
        );
        assert!(out.citations.is_empty());
    }

    #[tokio::test]
    async fn citations_are_numbered_from_next_citation() {
        let ctx = real_ctx(&main_db().await).await;
        add(&ctx, "2026-07-01T09:00", "app", "一", &["报销"]).await;
        add(&ctx, "2026-07-02T09:00", "app", "二", &["报销"]).await;

        let q = query(json!({"keywords": ["报销"]}));
        let out = q.execute(&ctx, 7, 4000, ChatLang::ZhHans).await.unwrap();
        let indexes: Vec<usize> = out.citations.iter().map(|c| c.index).collect();
        assert_eq!(indexes, [7, 8]);
    }

    #[tokio::test]
    async fn fails_instead_of_showing_text_it_cannot_check() {
        let mut ctx = real_ctx(&main_db().await).await;
        add(&ctx, "2026-07-01T09:00", "secret.exe", "日记", &["报销"]).await;
        // 主库读不出分组和设置时，宁可执行失败，也不给没筛过的屏幕文字
        ctx.main = crate::storage::Connection::open(":memory:").await.unwrap();

        let q = query(json!({"keywords": ["报销"]}));
        assert!(q.execute(&ctx, 1, 4000, ChatLang::ZhHans).await.is_err());
    }

    #[test]
    fn text_json_reads_and_keeps_placeholders_in_step() {
        LazyLock::force(&TEXT);
        assert_same_placeholders(TEXT_JSON);
    }

    #[test]
    fn spec_points_to_the_titles_tool() {
        let spec = OcrQuery::spec(ChatLang::ZhHans);
        assert_eq!(spec.name, "search_screen_text");
        assert!(spec.description.contains("search_titles"));
        assert_eq!(spec.parameters["required"], json!(["keywords"]));
    }

    #[test]
    fn spec_is_written_in_the_prompt_language() {
        let spec = OcrQuery::spec(ChatLang::Ja);
        assert_eq!(spec.name, "search_screen_text");
        assert!(spec.description.starts_with("画面から読み取った文字"));
        assert!(spec.description.contains("search_titles"));
        let date = &spec.parameters["properties"]["date_from"]["description"];
        assert_eq!(
            date,
            "開始日、任意。日付がなければ全期間を検索。形式 YYYY-MM-DD"
        );
    }
}
