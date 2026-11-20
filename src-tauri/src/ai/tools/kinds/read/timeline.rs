//! get_timeline：按小时抽样列出一段时间里的活动。

use std::sync::LazyLock;

use chrono::NaiveDate;
use rusqlite::params;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ai::model::ToolSpec;
use crate::ai::tools::kinds::params::{date_schema, invalid, parse_range, read_args, spec};
use crate::ai::tools::kinds::Tool;
use crate::ai::tools::lang::text::{load, Text};
use crate::ai::tools::{truncate, ChatLang, Citation, ToolCtx, ToolError, ToolOutput};
use crate::error::Result;
use crate::repo::sql::{from_stats_category_sql, host_rule_with_sql};
use crate::storage::SqliteResultExt;

#[derive(Debug, Clone, PartialEq)]
pub struct TimelineQuery {
    pub range: (NaiveDate, NaiveDate),
}

/// 模型传来的参数，检查之前。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Args {
    date_from: Option<String>,
    date_to: Option<String>,
}

/// `lang/timeline.json` 里的句子，只有时间线工具用。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TimelineText {
    empty: Text,
    header_sampled: Text,
    header_all: Text,
}

const TEXT_JSON: &str = include_str!("lang/timeline.json");

static TEXT: LazyLock<TimelineText> = LazyLock::new(|| load(TEXT_JSON));

impl ChatLang {
    /// 返回时段里没有活动时给模型的那句话。
    fn timeline_empty(self) -> &'static str {
        TEXT.empty.get(self)
    }

    /// 返回抽样列出时的开头。开头写出总条数、覆盖范围和每小时最多取几条。
    fn timeline_header_sampled(
        self,
        total: i64,
        first: &str,
        last: &str,
        shown: usize,
        per_hour: i64,
    ) -> String {
        TEXT.header_sampled.fill(
            self,
            &[
                ("total", &total),
                ("first", &first),
                ("last", &last),
                ("shown", &shown),
                ("per_hour", &per_hour),
            ],
        )
    }

    /// 返回全部列出时的开头。开头写出总条数。
    fn timeline_header_all(self, total: i64) -> String {
        TEXT.header_all.fill(self, &[("total", &total)])
    }
}

impl Tool for TimelineQuery {
    const NAME: &'static str = "get_timeline";

    fn spec(_lang: ChatLang) -> ToolSpec {
        spec(
            Self::NAME,
            "按小时抽样列出某时段的屏幕活动会话(时间、应用、标题),回答'某天/某下午在干什么'。返回头部标注总条数与覆盖范围,正文是抽样代表,不是全量。",
            json!({"type": "object", "properties": {
                "date_from": date_schema("起始日期,格式 YYYY-MM-DD"),
                "date_to": date_schema("结束日期,可选,不给就到今天,格式 YYYY-MM-DD")
            }}),
        )
    }

    fn parse(
        args: &Value,
        today: NaiveDate,
        lang: ChatLang,
    ) -> std::result::Result<Self, ToolError> {
        let args: Args = read_args(args, lang)?;
        check(&args, today, lang).map_err(|msg| invalid(&msg, lang))
    }

    async fn execute(
        &self,
        ctx: &ToolCtx,
        next_citation: usize,
        _max_chars: usize,
        lang: ChatLang,
    ) -> Result<ToolOutput> {
        run(ctx, self, next_citation, lang).await
    }
}

/// 检查参数的值。不合法时返回原因。
fn check(
    args: &Args,
    today: NaiveDate,
    lang: ChatLang,
) -> std::result::Result<TimelineQuery, String> {
    let range = parse_range(
        args.date_from.as_deref(),
        args.date_to.as_deref(),
        today,
        lang,
    )?
    .ok_or_else(|| lang.err_need_range(TimelineQuery::NAME))?;
    Ok(TimelineQuery { range })
}

/// 时间线会话数上限
const TIMELINE_LIMIT: usize = 50;
/// 每个小时桶最多取几条代表活动(按时长选)。
const TIMELINE_PER_HOUR: i64 = 3;

async fn run(
    ctx: &ToolCtx,
    q: &TimelineQuery,
    next_citation: usize,
    lang: ChatLang,
) -> Result<ToolOutput> {
    let (from, to) = q.range;
    let single_day = from == to;
    let (from, to) = (from.to_string(), to.to_string());
    // 网站规则的 WITH 排在最前，它的两个 ? 编号是 1、2，跟这里的 ?1、?2 是同一对日期
    let with = host_rule_with_sql("activities");
    let scope = format!(
        "{} WHERE a.local_date BETWEEN ?1 AND ?2 AND {}",
        from_stats_category_sql("activities"),
        ctx.policy.main_filter()
    );
    // 主源是主库活动记录:不开截图/OCR 的用户也有完整时间线,电池暂停或
    // 待识别积压造成的索引空窗也不再啃掉时段尾巴。
    let (total, span, rows) = ctx
        .main
        .call(move |conn| {
            let (total, first, last): (i64, Option<String>, Option<String>) = conn
                .query_row(
                    &format!("{with} SELECT COUNT(*), MIN(a.started_at), MAX(a.ended_at) {scope}"),
                    params![from, to],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .db()?;
            // 分层抽样:按(日,小时)分桶,桶内取时长最长的前 N 条——预算覆盖整段时间轴。
            // 不能用"ORDER BY started_at LIMIT 50":活跃日几千条活动时那是"最早的
            // 半小时",模型会把开头当成全天(2026-07-08 的误报正是这么来的)。
            let mut stmt = conn
                .prepare(&format!(
                    "{with} SELECT app, title, started_at, ended_at, shot FROM (
                         SELECT COALESCE(g.display_name, a.process_name) AS app,
                                COALESCE(a.window_title,'') AS title,
                                a.started_at, a.ended_at,
                                NULLIF(a.screenshot_path,'') AS shot,
                                ROW_NUMBER() OVER (
                                    PARTITION BY a.local_date, a.local_hour
                                    ORDER BY a.duration_secs DESC
                                ) AS rn
                         {scope}
                     ) WHERE rn <= ?3 ORDER BY started_at LIMIT ?4",
                ))
                .db()?;
            let out = stmt
                .query_map(
                    params![from, to, TIMELINE_PER_HOUR, TIMELINE_LIMIT as i64],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, Option<String>>(4)?,
                        ))
                    },
                )
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok((total, first.zip(last), out))
        })
        .await?;

    let mut citations = Vec::new();
    let mut lines = Vec::new();
    // 单日范围显示 HH:MM;跨日范围带上 MM-DD,避免不同日期的条目混淆
    let ts_disp = |ts: &str| -> String {
        if single_day {
            ts[11.min(ts.len())..16.min(ts.len())].to_string()
        } else {
            ts[5.min(ts.len())..16.min(ts.len())].to_string()
        }
    };
    for (i, (app, title, started, ended, shot)) in rows.iter().enumerate() {
        let idx = next_citation + i;
        lines.push(format!(
            "[{idx}] {} ~ {} | {} | {}",
            ts_disp(started),
            ts_disp(ended),
            app,
            truncate(title, 50)
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
    let content = if lines.is_empty() {
        lang.timeline_empty().to_string()
    } else {
        // 头部先声明总量与覆盖范围:样本 ≠ 全量,让模型据此下结论
        let header = match &span {
            Some((first, last)) if total as usize > rows.len() => lang.timeline_header_sampled(
                total,
                &first[..16.min(first.len())],
                &last[..16.min(last.len())],
                rows.len(),
                TIMELINE_PER_HOUR,
            ),
            _ => lang.timeline_header_all(total),
        };
        format!("{header}\n{}", lines.join("\n"))
    };
    Ok(ToolOutput { content, citations })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::tools::lang::text::assert_same_placeholders;
    use crate::ai::tools::test_util::{ctx_with, day};

    const LANGS: [ChatLang; 6] = [
        ChatLang::ZhHans,
        ChatLang::ZhHant,
        ChatLang::En,
        ChatLang::Ja,
        ChatLang::Pt,
        ChatLang::Es,
    ];

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 5).unwrap()
    }

    #[test]
    fn text_json_reads_and_keeps_placeholders_in_step() {
        LazyLock::force(&TEXT);
        assert_same_placeholders(TEXT_JSON);
    }

    #[test]
    fn every_language_fills_in_the_numbers() {
        for lang in LANGS {
            assert!(!lang.timeline_empty().is_empty(), "{lang:?}");
            let h = lang.timeline_header_sampled(120, "2026-07-01", "2026-07-07", 40, 7);
            assert!(h.contains("120") && h.contains("40"), "{lang:?}: {h}");
            assert!(lang.timeline_header_all(9).contains('9'), "{lang:?}");
        }
    }

    #[test]
    fn parse_reads_the_range() {
        let args = json!({"date_from": "2026-07-01", "date_to": "2026-07-05"});
        let q = TimelineQuery::parse(&args, today(), ChatLang::ZhHans).unwrap();
        assert_eq!(q.range.0, NaiveDate::from_ymd_opt(2026, 7, 1).unwrap());
        assert_eq!(q.range.1, today());
    }

    #[test]
    fn parse_needs_at_least_one_date() {
        let e = TimelineQuery::parse(&json!({}), today(), ChatLang::ZhHans).unwrap_err();
        assert!(e.0.contains("get_timeline"), "{}", e.0);
        // 只给开始：到今天为止
        let args = json!({"date_from": "2026-07-01"});
        let q = TimelineQuery::parse(&args, today(), ChatLang::ZhHans).unwrap();
        assert_eq!(
            q.range,
            (NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(), today())
        );
    }

    #[test]
    fn parse_takes_a_range_of_any_length() {
        let args = json!({"date_from": "2020-01-01", "date_to": "2026-07-01"});
        assert!(TimelineQuery::parse(&args, today(), ChatLang::ZhHans).is_ok());
        // 只给结束：从最早算起，不会因为跨度被拒
        let args = json!({"date_to": "2026-07-01"});
        let q = TimelineQuery::parse(&args, today(), ChatLang::ZhHans).unwrap();
        assert_eq!(q.range.1, NaiveDate::from_ymd_opt(2026, 7, 1).unwrap());
    }

    #[test]
    fn spec_requires_no_date() {
        let spec = TimelineQuery::spec(ChatLang::ZhHans);
        assert_eq!(spec.name, "get_timeline");
        assert!(spec.parameters.get("required").is_none());
        assert_eq!(spec.parameters["properties"]["date_from"]["maxLength"], 10);
    }

    /// 造一天 10 个小时 × 每小时 20 条活动的 INSERT 串(主库)。
    fn dense_day_sql() -> &'static str {
        use std::sync::OnceLock;
        static SQL: OnceLock<String> = OnceLock::new();
        SQL.get_or_init(|| {
            let mut s = String::new();
            for h in 9..19 {
                for m in 0..20 {
                    s.push_str(&format!(
                        "INSERT INTO activities(started_at, ended_at, duration_secs,
                             local_date, local_hour, process_name, window_title)
                         VALUES ('2026-07-08T{h:02}:{m:02}:00+09:00',
                                 '2026-07-08T{h:02}:{m:02}:40+09:00', 40,
                                 '2026-07-08', {h}, 'App', '活动 {h}-{m}');\n"
                    ));
                }
            }
            s
        })
    }

    #[tokio::test]
    async fn timeline_sampling_covers_whole_range_and_discloses_total() {
        // 泄漏 dense_day_sql 到 'static:测试进程内一次性,可接受
        let main: &'static str = Box::leak(dense_day_sql().to_string().into_boxed_str());
        let ctx = ctx_with("", main).await;
        let q = TimelineQuery { range: day() };
        let out = run(&ctx, &q, 1, ChatLang::ZhHans).await.unwrap();
        // 披露:总数与"样本"措辞
        assert!(out.content.contains("共 200 条活动记录"), "{}", out.content);
        assert!(out.content.contains("样本"), "{}", out.content);
        // 覆盖:10 个小时全部出现(旧实现只会给最早的 50 条 = 前 3 个小时)
        let hours: std::collections::BTreeSet<&str> = out
            .content
            .lines()
            .filter(|l| l.starts_with('['))
            .filter_map(|l| l.split("] ").nth(1))
            .map(|rest| &rest[..2])
            .collect();
        assert_eq!(hours.len(), 10, "每个小时都要有代表: {hours:?}");
        // 每小时至多 3 条
        for h in 9..19 {
            let prefix = format!("] {h:02}:");
            let n = out.content.lines().filter(|l| l.contains(&prefix)).count();
            assert!(n <= 3, "{h} 点出现 {n} 条");
        }
        assert!(out.citations.len() <= TIMELINE_LIMIT);
    }

    #[tokio::test]
    async fn timeline_headers_localized_english() {
        let main: &'static str = Box::leak(dense_day_sql().to_string().into_boxed_str());
        let ctx = ctx_with("", main).await;
        let q = TimelineQuery { range: day() };
        let out = run(&ctx, &q, 1, ChatLang::En).await.unwrap();
        assert!(
            out.content.contains("200 activity records in this period"),
            "{}",
            out.content
        );
        assert!(out.content.contains("sample"), "{}", out.content);
        // 头部行不应残留中文;正文标题是用户数据,语言不限
        let head = out.content.lines().next().unwrap();
        assert!(!head.contains("活动"), "{head}");
    }

    #[tokio::test]
    async fn an_empty_range_says_so_without_anything_else() {
        let ctx = ctx_with("", "").await;
        let q = TimelineQuery { range: day() };
        let out = run(&ctx, &q, 1, ChatLang::ZhHans).await.unwrap();
        assert_eq!(out.content, ChatLang::ZhHans.timeline_empty());
        assert!(out.citations.is_empty());
    }

    #[tokio::test]
    async fn timeline_small_day_lists_all_without_sampling_wording() {
        let ctx = ctx_with(
            "",
            "INSERT INTO activities(started_at, ended_at, duration_secs,
                 local_date, local_hour, process_name, window_title) VALUES
             ('2026-07-08T09:00:00+09:00','2026-07-08T09:05:00+09:00',300,'2026-07-08',9,'A','t1'),
             ('2026-07-08T10:00:00+09:00','2026-07-08T10:05:00+09:00',300,'2026-07-08',10,'B','t2');",
        )
        .await;
        let q = TimelineQuery { range: day() };
        let out = run(&ctx, &q, 1, ChatLang::ZhHans).await.unwrap();
        assert!(
            out.content.contains("共 2 条活动记录,全部列出"),
            "{}",
            out.content
        );
        assert!(!out.content.contains("样本"));
        assert_eq!(out.citations.len(), 2);
    }

    #[tokio::test]
    async fn timeline_excludes_hidden_group_and_uses_display_name() {
        let ctx = ctx_with(
            "",
            "INSERT INTO app_groups VALUES
                 ('g1','Visual Studio Code','dev',NULL),
                 ('g2','Secret','hidden',NULL);
             INSERT INTO app_group_members VALUES
                 ('Code.exe','g1',NULL), ('secret.exe','g2',NULL);
             INSERT INTO activities(started_at, ended_at, duration_secs,
                 local_date, local_hour, process_name, window_title) VALUES
             ('2026-07-08T09:00:00+09:00','2026-07-08T09:30:00+09:00',1800,'2026-07-08',9,'Code.exe','main.rs'),
             ('2026-07-08T10:00:00+09:00','2026-07-08T10:30:00+09:00',1800,'2026-07-08',10,'secret.exe','diary');",
        )
        .await;
        let q = TimelineQuery { range: day() };
        let out = run(&ctx, &q, 1, ChatLang::ZhHans).await.unwrap();
        assert!(
            out.content.contains("Visual Studio Code"),
            "{}",
            out.content
        );
        assert!(!out.content.contains("secret"), "{}", out.content);
        assert!(out.content.contains("共 1 条活动记录"), "{}", out.content);
    }
}
