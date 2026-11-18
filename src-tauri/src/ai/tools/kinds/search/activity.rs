//! search_titles：按关键词搜窗口标题。回答什么时候看过 X、在哪个窗口、看了多久。

use std::sync::LazyLock;

use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::Value;

use super::{date_bounds, quote_keywords, write_list, Row, SearchParams};
use crate::ai::model::ToolSpec;
use crate::ai::tools::kinds::params::spec;
use crate::ai::tools::kinds::Tool;
use crate::ai::tools::text::{load, Text};
use crate::ai::tools::visible::MAIN_VISIBLE;
use crate::ai::tools::{
    like_pattern, truncate, ChatLang, Citation, ToolCtx, ToolError, ToolOutput,
};
use crate::error::Result;
use crate::repo::sql::{from_stats_category_sql, host_rule_with_sql};
use crate::storage::SqliteResultExt;

/// search_titles 检查过的参数。
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityQuery(pub SearchParams);

/// 每行的标题最多几个字。
const TITLE_MAX_CHARS: usize = 60;

/// `lang/activity.json` 里的句子。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivityText {
    /// 开头第一句：命中了几个窗口，一共多久。
    head: Text,
    /// 开头第二句：最早、最近一次开始的时刻和窗口。
    span: Text,
    /// 列表前的一行。
    list_head: Text,
    /// 一个窗口用了多久，只在一天里出现过。
    use_one_day: Text,
    /// 一个窗口用了多久，在几天里出现过。
    use_days: Text,
    /// 没有命中。
    none: Text,
}

const TEXT_JSON: &str = include_str!("lang/activity.json");

static TEXT: LazyLock<ActivityText> = LazyLock::new(|| load(TEXT_JSON));

impl Tool for ActivityQuery {
    const NAME: &'static str = "search_titles";

    fn spec() -> ToolSpec {
        spec(
            Self::NAME,
            "按关键词搜窗口标题,回答什么时候看过 X、在哪个窗口、看了多久。返回命中的窗口数、总时长、最早和最近一次,再按时长列出窗口。只搜标题;屏幕上出现过的字用 search_screen_text 搜。",
            SearchParams::schema("1-3 个关键词,每个都出现在标题里才算命中;英文不分大小写"),
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
        let found = find(ctx, &self.0).await?;
        Ok(render(&self.0, &found, next_citation, max_chars, lang))
    }
}

/// 查到的结果，写成文字之前。
#[derive(Debug)]
struct Found {
    /// 要列的窗口，时长从多到少，最多 `limit` 个。
    windows: Vec<WindowUse>,
    /// 全部命中的窗口数。
    count: usize,
    /// 全部命中的时长，秒。
    secs: i64,
    /// 最早一次：开始时刻、窗口。
    first: Option<(String, String)>,
    /// 最近一次：开始时刻、窗口。
    last: Option<(String, String)>,
}

/// 标题里有关键词的一个窗口。
#[derive(Debug)]
struct WindowUse {
    app: String,
    title: String,
    /// 命中的活动记录时长相加，秒。
    secs: i64,
    /// 在几天里出现过。
    days: usize,
    /// 第一次命中的开始时刻。
    first_start: String,
    /// 最近一次命中的起止。证据卡用。
    last: (String, String),
    /// 最近一次命中的截图。
    screenshot: Option<String>,
}

impl Found {
    /// 用全部命中的窗口算出开头的数字，再留下前 `limit` 个。
    ///
    /// `all` 要按时长从多到少排好。
    fn new(mut all: Vec<WindowUse>, limit: usize) -> Self {
        let first = all
            .iter()
            .min_by(|a, b| a.first_start.cmp(&b.first_start))
            .map(|w| (w.first_start.clone(), window(w)));
        let last = all
            .iter()
            .max_by(|a, b| a.last.0.cmp(&b.last.0))
            .map(|w| (w.last.0.clone(), window(w)));
        let count = all.len();
        let secs = all.iter().map(|w| w.secs).sum();
        all.truncate(limit);
        Self {
            windows: all,
            count,
            secs,
            first,
            last,
        }
    }
}

/// 查主库里标题有全部关键词、AI 能看到的活动，一个窗口一行。
async fn find(ctx: &ToolCtx, p: &SearchParams) -> Result<Found> {
    let (from, to) = date_bounds(p.range);
    // 前两个给网站规则，后两个给活动
    let mut binds = vec![from.clone(), to.clone(), from, to];
    let mut conds = String::new();
    for k in &p.keywords {
        conds.push_str(" AND a.window_title LIKE ? ESCAPE '\\'");
        binds.push(like_pattern(k));
    }
    if !p.apps.is_empty() {
        // 模型看到的是显示名，也可能写进程名
        let one = "a.process_name LIKE ? ESCAPE '\\' OR g.display_name LIKE ? ESCAPE '\\'";
        conds.push_str(&format!(" AND ({})", vec![one; p.apps.len()].join(" OR ")));
        for app in &p.apps {
            binds.extend([like_pattern(app), like_pattern(app)]);
        }
    }
    if !p.categories.is_empty() {
        // 分类可以写名字，也可以写 id。没分类的活动算「其他」，跟统计页一样
        let one = "name LIKE ? ESCAPE '\\' OR id LIKE ? ESCAPE '\\'";
        conds.push_str(&format!(
            " AND COALESCE(c.id, 'other') IN (
                 SELECT id FROM categories WHERE deleted_at IS NULL AND ({}))",
            vec![one; p.categories.len()].join(" OR ")
        ));
        for cat in &p.categories {
            binds.extend([like_pattern(cat), like_pattern(cat)]);
        }
    }
    let sql = format!(
        "{with},
         hits AS (
             SELECT COALESCE(g.display_name, a.process_name) AS app,
                    COALESCE(a.window_title, '') AS title,
                    a.started_at, a.ended_at, a.duration_secs AS secs, a.local_date,
                    NULLIF(a.screenshot_path, '') AS shot
             {from_sql}
             WHERE a.local_date BETWEEN ? AND ? AND {MAIN_VISIBLE}{conds}
         ),
         windows AS (
             SELECT app, title, SUM(secs) AS secs, COUNT(DISTINCT local_date) AS days,
                    MIN(started_at) AS first_start
             FROM hits GROUP BY app, title
         ),
         latest AS (
             SELECT app, title, started_at, ended_at, shot,
                    ROW_NUMBER() OVER (PARTITION BY app, title ORDER BY started_at DESC) AS rn
             FROM hits
         )
         SELECT w.app, w.title, w.secs, w.days, w.first_start,
                l.started_at, l.ended_at, l.shot
         FROM windows w
         JOIN latest l ON l.app = w.app AND l.title = w.title AND l.rn = 1
         ORDER BY w.secs DESC, l.started_at DESC",
        with = host_rule_with_sql("activities"),
        from_sql = from_stats_category_sql("activities"),
    );
    let all = ctx
        .main
        .call(move |conn| {
            let mut stmt = conn.prepare(&sql).db()?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(binds.iter()), |r| {
                    Ok(WindowUse {
                        app: r.get(0)?,
                        title: r.get(1)?,
                        secs: r.get(2)?,
                        days: r.get::<_, i64>(3)? as usize,
                        first_start: r.get(4)?,
                        last: (r.get(5)?, r.get(6)?),
                        screenshot: r.get(7)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await?;
    Ok(Found::new(all, p.limit))
}

/// 写成给模型看的文字，列出来的每个窗口配一条证据：最近一次命中的那条记录和它的截图。
fn render(
    p: &SearchParams,
    found: &Found,
    next_citation: usize,
    max_chars: usize,
    lang: ChatLang,
) -> ToolOutput {
    let kw = quote_keywords(&p.keywords, lang);
    // 有命中才有最早、最近一次
    let (Some((first_at, first)), Some((last_at, last))) = (&found.first, &found.last) else {
        return ToolOutput {
            content: TEXT.none.fill(lang, &[("kw", &kw)]),
            citations: Vec::new(),
        };
    };
    let dur = lang.fmt_secs(found.secs);
    let head = vec![
        TEXT.head
            .fill(lang, &[("kw", &kw), ("count", &found.count), ("dur", &dur)]),
        TEXT.span.fill(
            lang,
            &[
                ("first_at", &minute(first_at)),
                ("first", first),
                ("last_at", &minute(last_at)),
                ("last", last),
            ],
        ),
    ];
    let rows = found
        .windows
        .iter()
        .map(|w| Row {
            line: format!("{} | {}", window(w), use_line(w, lang)),
            citation: Citation {
                index: 0,
                app: w.app.clone(),
                title: w.title.clone(),
                started_ts: w.last.0.clone(),
                ended_ts: w.last.1.clone(),
                frame_path: w.screenshot.clone(),
            },
        })
        .collect();
    let list_head = |n: usize| TEXT.list_head.fill(lang, &[("n", &n)]);
    write_list(
        head,
        list_head,
        rows,
        found.count,
        next_citation,
        max_chars,
        lang,
    )
}

/// 一个窗口写成「应用 | 标题」，标题最多 60 字。
fn window(w: &WindowUse) -> String {
    format!("{} | {}", w.app, truncate(&w.title, TITLE_MAX_CHARS))
}

/// 一个窗口用了多久、在几天里出现过。
fn use_line(w: &WindowUse, lang: ChatLang) -> String {
    let text = if w.days == 1 {
        &TEXT.use_one_day
    } else {
        &TEXT.use_days
    };
    text.fill(lang, &[("dur", &lang.fmt_secs(w.secs)), ("days", &w.days)])
}

/// 时刻写到分钟：2026-07-08 09:00。
fn minute(ts: &str) -> String {
    ts.get(..16).unwrap_or(ts).replacen('T', " ", 1)
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDateTime, Timelike};
    use rusqlite::params;
    use serde_json::json;

    use super::super::more;
    use super::*;
    use crate::ai::tools::test_util::real_ctx;
    use crate::ai::tools::text::assert_same_placeholders;
    use crate::repo::categories::assign_app;
    use crate::repo::test_util::fresh_test_pool;
    use crate::storage::DbPool;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 10).unwrap()
    }

    /// 加一条活动。`start` 写到分钟，比如 2026-07-08T09:00。
    async fn add(pool: &DbPool, start: &str, mins: i64, app: &str, title: &str) {
        add_full(pool, start, mins, app, title, None, None).await;
    }

    /// 同 [`add`]，再带截图和网站。
    async fn add_full(
        pool: &DbPool,
        start: &str,
        mins: i64,
        app: &str,
        title: &str,
        shot: Option<&str>,
        host: Option<&str>,
    ) {
        let begin = NaiveDateTime::parse_from_str(start, "%Y-%m-%dT%H:%M").unwrap();
        let end = begin + chrono::Duration::minutes(mins);
        let ts = |t: NaiveDateTime| t.format("%Y-%m-%dT%H:%M:%S+09:00").to_string();
        let row = (
            ts(begin),
            ts(end),
            mins * 60,
            begin.date().to_string(),
            begin.hour(),
            app.to_string(),
            title.to_string(),
            shot.map(str::to_string),
            host.map(str::to_string),
        );
        pool.0
            .call(move |c| {
                c.execute(
                    "INSERT INTO activities(started_at, ended_at, duration_secs, local_date,
                         local_hour, process_name, window_title, category_id,
                         screenshot_path, url_host)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'other', ?8, ?9)",
                    params![row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7, row.8],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn run_sql(pool: &DbPool, sql: &'static str) {
        pool.0
            .call(move |c| c.execute_batch(sql).db())
            .await
            .unwrap();
    }

    fn query(args: Value) -> ActivityQuery {
        ActivityQuery::parse(&args, today(), ChatLang::ZhHans).unwrap()
    }

    async fn found(pool: &DbPool, args: Value) -> Found {
        let ctx = real_ctx(pool).await;
        find(&ctx, &query(args).0).await.unwrap()
    }

    async fn output(pool: &DbPool, args: Value, max_chars: usize) -> ToolOutput {
        let ctx = real_ctx(pool).await;
        query(args)
            .execute(&ctx, 1, max_chars, ChatLang::ZhHans)
            .await
            .unwrap()
    }

    /// 五个窗口，标题都有「报销」，时长依次是 50、40、30、20、10 分钟。
    async fn five_windows(pool: &DbPool) {
        for (i, mins) in [50, 40, 30, 20, 10].into_iter().enumerate() {
            let start = format!("2026-07-0{}T09:00", i + 1);
            add(pool, &start, mins, "chrome", &format!("报销 第{i}页")).await;
        }
    }

    #[tokio::test]
    async fn one_row_per_window_sorted_by_time() {
        let pool = fresh_test_pool().await;
        add(
            &pool,
            "2026-07-01T10:02",
            18,
            "EXCEL.EXE",
            "报销单_7月.xlsx",
        )
        .await;
        add(
            &pool,
            "2026-07-08T15:40",
            10,
            "EXCEL.EXE",
            "报销单_7月.xlsx",
        )
        .await;
        let shot = Some("shots/excel.jpg");
        add_full(
            &pool,
            "2026-07-08T15:55",
            10,
            "EXCEL.EXE",
            "报销单_7月.xlsx",
            shot,
            None,
        )
        .await;
        add(
            &pool,
            "2026-07-03T09:00",
            64,
            "chrome",
            "报销单填写说明 - Wiki",
        )
        .await;
        add(&pool, "2026-07-04T09:00", 30, "chrome", "无关页面").await;

        let f = found(&pool, json!({"keywords": ["报销"]})).await;
        assert_eq!(f.count, 2);
        assert_eq!(f.secs, (18 + 10 + 10 + 64) * 60);
        let names: Vec<&str> = f.windows.iter().map(|w| w.app.as_str()).collect();
        assert_eq!(names, ["chrome", "EXCEL.EXE"]);
        let excel = &f.windows[1];
        assert_eq!(excel.secs, 38 * 60);
        assert_eq!(excel.days, 2);
        // 证据卡用最近一次命中的那条记录和它的截图
        assert_eq!(excel.last.0, "2026-07-08T15:55:00+09:00");
        assert_eq!(excel.last.1, "2026-07-08T16:05:00+09:00");
        assert_eq!(excel.screenshot.as_deref(), Some("shots/excel.jpg"));
        let first = f.first.unwrap();
        assert_eq!(first.0, "2026-07-01T10:02:00+09:00");
        assert_eq!(first.1, "EXCEL.EXE | 报销单_7月.xlsx");
        assert_eq!(f.last.unwrap().0, "2026-07-08T15:55:00+09:00");
    }

    #[tokio::test]
    async fn text_has_the_head_the_list_and_one_citation_per_line() {
        let pool = fresh_test_pool().await;
        add(
            &pool,
            "2026-07-01T10:02",
            18,
            "EXCEL.EXE",
            "报销单_7月.xlsx",
        )
        .await;
        add(
            &pool,
            "2026-07-08T15:55",
            20,
            "EXCEL.EXE",
            "报销单_7月.xlsx",
        )
        .await;
        add(
            &pool,
            "2026-07-03T09:00",
            64,
            "chrome",
            "报销单填写说明 - Wiki",
        )
        .await;

        let out = output(&pool, json!({"keywords": ["报销"]}), 4000).await;
        let expected = [
            "标题里有「报销」的窗口：2 个，共 1 小时 42 分钟。",
            "最早 2026-07-01 10:02（EXCEL.EXE | 报销单_7月.xlsx），最近 2026-07-08 15:55（EXCEL.EXE | 报销单_7月.xlsx）。",
            "按时长列前 2 个：",
            "[1] chrome | 报销单填写说明 - Wiki | 共 1 小时 4 分钟，1 天",
            "[2] EXCEL.EXE | 报销单_7月.xlsx | 共 38 分钟，2 天",
        ];
        assert_eq!(out.content, expected.join("\n"));
        assert_eq!(out.citations.len(), 2);
        assert_eq!(out.citations[1].index, 2);
        assert_eq!(out.citations[1].app, "EXCEL.EXE");
        assert_eq!(out.citations[1].started_ts, "2026-07-08T15:55:00+09:00");
    }

    #[tokio::test]
    async fn every_keyword_must_be_in_the_title_and_english_ignores_case() {
        let pool = fresh_test_pool().await;
        add(
            &pool,
            "2026-07-01T09:00",
            10,
            "chrome",
            "Rust Book - Ownership",
        )
        .await;
        add(&pool, "2026-07-01T10:00", 10, "chrome", "rust tutorial").await;
        add(&pool, "2026-07-01T11:00", 10, "chrome", "Go book").await;

        let f = found(&pool, json!({"keywords": ["rust", "BOOK"]})).await;
        let titles: Vec<&str> = f.windows.iter().map(|w| w.title.as_str()).collect();
        assert_eq!(titles, ["Rust Book - Ownership"]);
    }

    #[tokio::test]
    async fn wildcards_in_keywords_match_only_themselves() {
        let pool = fresh_test_pool().await;
        add(&pool, "2026-07-01T09:00", 10, "app", "100% done").await;
        add(&pool, "2026-07-01T10:00", 10, "app", "1000 items").await;
        add(&pool, "2026-07-01T11:00", 10, "app", "a_b").await;
        add(&pool, "2026-07-01T12:00", 10, "app", "axb").await;

        let f = found(&pool, json!({"keywords": ["100%"]})).await;
        assert_eq!(f.windows[0].title, "100% done");
        assert_eq!(f.count, 1);
        let f = found(&pool, json!({"keywords": ["a_b"]})).await;
        assert_eq!(f.windows[0].title, "a_b");
        assert_eq!(f.count, 1);
    }

    #[tokio::test]
    async fn hides_what_the_stats_page_hides_and_the_private_category() {
        let pool = fresh_test_pool().await;
        assign_app(&pool, "secret.exe", "hidden").await.unwrap();
        assign_app(&pool, "Signal", "private").await.unwrap();
        crate::repo::site_rules::set(&pool, "bank.example", "private")
            .await
            .unwrap();
        add(
            &pool,
            "2026-07-01T09:00",
            10,
            "secret.exe",
            "报销 不计入统计",
        )
        .await;
        add(&pool, "2026-07-01T10:00", 10, "Signal", "报销 隐私应用").await;
        let host = Some("bank.example");
        add_full(
            &pool,
            "2026-07-01T11:00",
            10,
            "chrome",
            "报销 隐私网站",
            None,
            host,
        )
        .await;
        add(&pool, "2026-07-01T12:00", 10, "ignored.exe", "报销 已忽略").await;
        run_sql(
            &pool,
            "UPDATE activities SET excluded = 1 WHERE process_name = 'ignored.exe'",
        )
        .await;
        add(&pool, "2026-07-01T13:00", 10, "notepad", "报销 笔记").await;

        let f = found(&pool, json!({"keywords": ["报销"]})).await;
        assert_eq!(f.count, 1);
        assert_eq!(f.secs, 600);
        assert_eq!(f.windows[0].title, "报销 笔记");
    }

    #[tokio::test]
    async fn apps_match_the_process_or_the_display_name() {
        let pool = fresh_test_pool().await;
        assign_app(&pool, "Code.exe", "code").await.unwrap();
        run_sql(
            &pool,
            "UPDATE app_groups SET display_name = 'Visual Studio Code'
             WHERE id IN (SELECT group_id FROM app_group_members WHERE process_name = 'Code.exe')",
        )
        .await;
        add(&pool, "2026-07-01T09:00", 10, "Code.exe", "报销 脚本").await;
        add(&pool, "2026-07-01T10:00", 10, "notepad", "报销 笔记").await;

        for app in ["Visual Studio", "code.exe"] {
            let f = found(&pool, json!({"keywords": ["报销"], "apps": [app]})).await;
            let apps: Vec<&str> = f.windows.iter().map(|w| w.app.as_str()).collect();
            assert_eq!(apps, ["Visual Studio Code"], "{app}");
        }
        let f = found(
            &pool,
            json!({"keywords": ["报销"], "apps": ["Code", "notepad"]}),
        )
        .await;
        assert_eq!(f.count, 2);
    }

    #[tokio::test]
    async fn categories_follow_website_rules_and_unclassified_is_other() {
        let pool = fresh_test_pool().await;
        crate::repo::site_rules::set(&pool, "video.example", "video")
            .await
            .unwrap();
        let host = Some("video.example");
        add_full(
            &pool,
            "2026-07-01T09:00",
            10,
            "chrome",
            "报销 视频",
            None,
            host,
        )
        .await;
        add(&pool, "2026-07-01T10:00", 10, "chrome", "报销 文档").await;

        // 名字和 id 都认
        for cat in ["影音", "video"] {
            let f = found(&pool, json!({"keywords": ["报销"], "categories": [cat]})).await;
            assert_eq!(f.count, 1, "{cat}");
            assert_eq!(f.windows[0].title, "报销 视频");
        }
        let f = found(&pool, json!({"keywords": ["报销"], "categories": ["其他"]})).await;
        assert_eq!(f.count, 1);
        assert_eq!(f.windows[0].title, "报销 文档");
    }

    #[tokio::test]
    async fn the_date_range_limits_the_records() {
        let pool = fresh_test_pool().await;
        add(&pool, "2026-07-01T09:00", 10, "app", "报销 一").await;
        add(&pool, "2026-07-08T09:00", 20, "app", "报销 一").await;

        let args =
            json!({"keywords": ["报销"], "date_from": "2026-07-08", "date_to": "2026-07-08"});
        let f = found(&pool, args).await;
        assert_eq!(f.secs, 1200);
        assert_eq!(f.windows[0].days, 1);
        assert_eq!(f.first.unwrap().0, "2026-07-08T09:00:00+09:00");
    }

    #[tokio::test]
    async fn limit_lists_the_longest_and_says_how_many_are_left() {
        let pool = fresh_test_pool().await;
        five_windows(&pool).await;

        let out = output(&pool, json!({"keywords": ["报销"], "limit": 2}), 4000).await;
        assert!(out.content.contains("：5 个"), "{}", out.content);
        assert!(out.content.contains("按时长列前 2 个"), "{}", out.content);
        assert!(out.content.contains("报销 第0页") && out.content.contains("报销 第1页"));
        assert!(!out.content.contains("报销 第2页"), "{}", out.content);
        assert!(out.content.ends_with(&more(3, ChatLang::ZhHans)));
        assert_eq!(out.citations.len(), 2);
    }

    #[tokio::test]
    async fn lists_fewer_when_the_text_would_be_too_long() {
        let pool = fresh_test_pool().await;
        five_windows(&pool).await;

        let max_chars = 230;
        let out = output(&pool, json!({"keywords": ["报销"]}), max_chars).await;
        assert!(out.content.chars().count() <= max_chars, "{}", out.content);
        let shown = out.citations.len();
        assert!(0 < shown && shown < 5, "{shown}: {}", out.content);
        // 每条证据都有一行，没列的写在最后
        let lines = out.content.lines().filter(|l| l.starts_with('[')).count();
        assert_eq!(lines, shown);
        assert!(out.content.ends_with(&more(5 - shown, ChatLang::ZhHans)));
    }

    #[tokio::test]
    async fn long_titles_are_cut_to_60_chars() {
        let pool = fresh_test_pool().await;
        let long = format!("报销{}", "长".repeat(80));
        add(&pool, "2026-07-01T09:00", 10, "app", &long).await;

        let out = output(&pool, json!({"keywords": ["报销"]}), 4000).await;
        let line = out.content.lines().find(|l| l.starts_with("[1]")).unwrap();
        let cut = truncate(&long, TITLE_MAX_CHARS);
        assert!(line.contains(&format!("app | {cut} |")), "{line}");
        // 证据卡用完整的标题
        assert_eq!(out.citations[0].title, long);
    }

    #[tokio::test]
    async fn no_hit_says_so_and_points_to_screen_text() {
        let pool = fresh_test_pool().await;
        add(&pool, "2026-07-01T09:00", 10, "app", "别的").await;

        let out = output(&pool, json!({"keywords": ["报销", "7月"]}), 4000).await;
        assert_eq!(
            out.content,
            "没有标题里有「报销」「7月」的窗口。可以换个词，或者用 search_screen_text 找屏幕上出现过的字。"
        );
        assert!(out.citations.is_empty());
    }

    #[tokio::test]
    async fn citations_are_numbered_from_next_citation() {
        let pool = fresh_test_pool().await;
        five_windows(&pool).await;
        let ctx = real_ctx(&pool).await;

        let q = query(json!({"keywords": ["报销"], "limit": 2}));
        let out = q.execute(&ctx, 7, 4000, ChatLang::ZhHans).await.unwrap();
        let indexes: Vec<usize> = out.citations.iter().map(|c| c.index).collect();
        assert_eq!(indexes, [7, 8]);
        assert!(out.content.contains("[7] ") && out.content.contains("[8] "));
    }

    #[tokio::test]
    async fn writes_in_the_prompt_language() {
        let pool = fresh_test_pool().await;
        add(&pool, "2026-07-01T09:00", 10, "app", "报销").await;
        let ctx = real_ctx(&pool).await;

        let q = query(json!({"keywords": ["报销"]}));
        let out = q.execute(&ctx, 1, 4000, ChatLang::En).await.unwrap();
        assert!(
            out.content
                .starts_with("Windows with \"报销\" in the title: 1, 10 min in total."),
            "{}",
            out.content
        );
        assert!(out.content.contains("10 min on 1 day"), "{}", out.content);
    }

    #[tokio::test]
    async fn english_uses_the_plural_for_more_than_one_day() {
        let pool = fresh_test_pool().await;
        add(&pool, "2026-07-01T09:00", 10, "app", "报销").await;
        add(&pool, "2026-07-02T09:00", 10, "app", "报销").await;
        let ctx = real_ctx(&pool).await;

        let q = query(json!({"keywords": ["报销"]}));
        let out = q.execute(&ctx, 1, 4000, ChatLang::En).await.unwrap();
        assert!(
            out.content.contains("20 min over 2 days"),
            "{}",
            out.content
        );
    }

    #[test]
    fn text_json_reads_and_keeps_placeholders_in_step() {
        LazyLock::force(&TEXT);
        assert_same_placeholders(TEXT_JSON);
    }

    #[test]
    fn spec_points_to_the_screen_text_tool() {
        let spec = ActivityQuery::spec();
        assert_eq!(spec.name, "search_titles");
        assert!(spec.description.contains("search_screen_text"));
        assert_eq!(spec.parameters["required"], json!(["keywords"]));
    }
}
