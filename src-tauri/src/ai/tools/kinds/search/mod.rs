//! 找：按关键词找 X 出现在哪。
//!
//! 两个搜索工具的参数一样，都是 [`SearchParams`]；结果都是一个窗口一行，由 [`write_list`] 写。

pub(in crate::ai::tools) mod activity;
pub(in crate::ai::tools) mod ocr;

use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{json, Value};

use super::params::{
    clean_short_strings, date_schema, invalid, parse_range, read_args, short_strings_schema,
};
use crate::ai::tools::text::{Text, COMMON};
use crate::ai::tools::{ChatLang, Citation, ToolError, ToolOutput};

/// 关键词最多几个。
const KEYWORDS_MAX: usize = 3;
/// 不给 limit 时列几个。
const LIMIT_DEFAULT: usize = 8;
/// limit 最大是几。
const LIMIT_MAX: usize = 20;

/// 搜索工具检查过的参数。
#[derive(Debug, Clone, PartialEq)]
pub struct SearchParams {
    /// 1 到 3 个。每个都出现才算命中。
    keywords: Vec<String>,
    /// 不给就搜全部日期。
    range: Option<(NaiveDate, NaiveDate)>,
    /// 只搜这些应用。空的就不按应用过滤。
    apps: Vec<String>,
    /// 只搜这些分类里的应用。空的就不按分类过滤。
    categories: Vec<String>,
    /// 最多列几个。
    limit: usize,
}

/// 模型传来的参数，检查之前。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Args {
    keywords: Vec<String>,
    date_from: Option<String>,
    date_to: Option<String>,
    apps: Vec<String>,
    categories: Vec<String>,
    limit: Option<i64>,
}

impl SearchParams {
    /// 解析、检查模型传来的参数。不合法时，原因写给模型看。
    fn parse(args: &Value, today: NaiveDate, lang: ChatLang) -> Result<Self, ToolError> {
        let args: Args = read_args(args, lang)?;
        check(&args, today, lang).map_err(|msg| invalid(&msg, lang))
    }

    /// 给模型看的参数格式，说明按 `lang` 写。`keywords_desc` 写关键词在哪里比对。
    fn schema(keywords_desc: &str, lang: ChatLang) -> Value {
        let names = |text: &Text| short_strings_schema(text.get(lang), None);
        json!({"type": "object", "properties": {
            "keywords": short_strings_schema(keywords_desc, Some(KEYWORDS_MAX as u32)),
            "date_from": date_schema(COMMON.param_date_from.get(lang)),
            "date_to": date_schema(COMMON.param_date_to.get(lang)),
            "apps": names(&COMMON.param_apps),
            "categories": names(&COMMON.param_categories),
            "limit": {"type": "integer", "minimum": 1, "maximum": LIMIT_MAX,
                      "description": COMMON.param_limit.fill(lang, &[("n", &LIMIT_DEFAULT)])}
        }, "required": ["keywords"]})
    }
}

/// 检查参数的值，补上默认值。不合法时返回原因。
fn check(args: &Args, today: NaiveDate, lang: ChatLang) -> Result<SearchParams, String> {
    let keywords = clean_short_strings(&args.keywords, "keywords", lang)?;
    if keywords.is_empty() {
        return Err(lang.err_keywords_empty().into());
    }
    // 多给的不悄悄丢掉：模型以为都用上了，答案会错
    if keywords.len() > KEYWORDS_MAX {
        return Err(lang.err_too_many_items("keywords", KEYWORDS_MAX));
    }
    // 日期只给一头时当作没给；跨度不设上限
    let range = parse_range(
        args.date_from.as_deref(),
        args.date_to.as_deref(),
        today,
        i64::MAX,
        lang,
    )?;
    let limit = match args.limit {
        None => LIMIT_DEFAULT,
        Some(n) if (1..=LIMIT_MAX as i64).contains(&n) => n as usize,
        Some(_) => return Err(lang.err_limit_range(LIMIT_MAX)),
    };
    Ok(SearchParams {
        keywords,
        range,
        apps: clean_short_strings(&args.apps, "apps", lang)?,
        categories: clean_short_strings(&args.categories, "categories", lang)?,
        limit,
    })
}

/// 列表里的一个窗口：一行字和它的证据。
struct Row {
    /// 编号后面的部分。
    line: String,
    /// 编号由 [`write_list`] 填。
    citation: Citation,
}

/// 写成给模型看的文字：开头，一个窗口一行，没列全时再写一行。
///
/// `head(n)` 写开头的几行，`n` 是列了几个，一个都没列时是 0。
/// `count` 是全部命中的窗口数，`rows` 是要列的。编号从 `next_citation` 开始。
/// 文字超过 `max_chars` 时少列几个，不截断某一行。
fn write_list(
    head: impl Fn(usize) -> Vec<String>,
    rows: Vec<Row>,
    count: usize,
    next_citation: usize,
    max_chars: usize,
    lang: ChatLang,
) -> ToolOutput {
    // 给开头和最后一行留位置。按最大的数算，实际写出来只会更短
    let mut used: usize = head(rows.len())
        .iter()
        .chain([&more(count, lang)])
        .map(|s| s.chars().count() + 1)
        .sum();
    let mut lines = Vec::new();
    let mut citations = Vec::new();
    for Row { line, mut citation } in rows {
        let index = next_citation + citations.len();
        let line = format!("[{index}] {line}");
        used += line.chars().count() + 1;
        if used > max_chars {
            break;
        }
        citation.index = index;
        lines.push(line);
        citations.push(citation);
    }
    let mut out = head(lines.len());
    out.append(&mut lines);
    let rest = count - citations.len();
    if rest > 0 {
        out.push(more(rest, lang));
    }
    ToolOutput {
        content: out.join("\n"),
        citations,
    }
}

/// 把关键词写进句子：「报销」「7月」。
fn quote_keywords(keywords: &[String], lang: ChatLang) -> String {
    let quoted: Vec<String> = keywords
        .iter()
        .map(|k| COMMON.quote.fill(lang, &[("keyword", k)]))
        .collect();
    quoted.join(COMMON.quote_sep.get(lang))
}

/// 没列全时的最后一行：还有 `rest` 个没列，怎么看更多。
fn more(rest: usize, lang: ChatLang) -> String {
    COMMON.more.fill(lang, &[("rest", &rest)])
}

/// 不给日期就搜全部：换成一个包住所有日期的范围，SQL 只要一种写法。
fn date_bounds(range: Option<(NaiveDate, NaiveDate)>) -> (String, String) {
    match range {
        Some((from, to)) => (from.to_string(), to.to_string()),
        None => ("0000-01-01".to_string(), "9999-12-31".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 5).unwrap()
    }

    fn parse(args: Value) -> Result<SearchParams, String> {
        SearchParams::parse(&args, today(), ChatLang::ZhHans).map_err(|e| e.0)
    }

    #[test]
    fn parse_reads_every_param() {
        let p = parse(json!({
            "keywords": [" 报销 ", "7月"],
            "date_from": "2026-07-01", "date_to": "2026-07-05",
            "apps": ["Excel"], "categories": ["工作"], "limit": 20
        }))
        .unwrap();
        assert_eq!(p.keywords, ["报销", "7月"]);
        let from = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
        assert_eq!(p.range, Some((from, today())));
        assert_eq!(p.apps, ["Excel"]);
        assert_eq!(p.categories, ["工作"]);
        assert_eq!(p.limit, 20);
    }

    #[test]
    fn parse_fills_the_defaults() {
        let p = parse(json!({"keywords": ["报销"]})).unwrap();
        assert_eq!(p.range, None);
        assert!(p.apps.is_empty() && p.categories.is_empty());
        assert_eq!(p.limit, LIMIT_DEFAULT);
    }

    #[test]
    fn parse_rejects_missing_or_blank_keywords() {
        assert!(parse(json!({})).unwrap_err().contains("不能为空"));
        let e = parse(json!({"keywords": [" ", ""]})).unwrap_err();
        assert!(e.contains("不能为空"), "{e}");
    }

    #[test]
    fn parse_rejects_more_than_three_keywords() {
        // 空白的不算：3 个词加一个空白可以
        assert!(parse(json!({"keywords": ["a", "b", "c", " "]})).is_ok());
        let e = parse(json!({"keywords": ["a", "b", "c", "d"]})).unwrap_err();
        assert!(e.contains("keywords") && e.contains('3'), "{e}");
    }

    #[test]
    fn parse_rejects_a_keyword_over_64_chars() {
        let e = parse(json!({"keywords": ["字".repeat(65)]})).unwrap_err();
        assert!(e.contains("keywords") && e.contains("64"), "{e}");
    }

    #[test]
    fn parse_takes_any_number_of_apps_and_categories() {
        let many: Vec<String> = (0..30).map(|i| format!("app{i}")).collect();
        let p = parse(json!({"keywords": ["a"], "apps": many, "categories": many})).unwrap();
        assert_eq!(p.apps, many);
        assert_eq!(p.categories, many);
    }

    #[test]
    fn parse_checks_limit() {
        for bad in [0, -1, 21] {
            let e = parse(json!({"keywords": ["a"], "limit": bad})).unwrap_err();
            assert!(e.contains("limit") && e.contains("20"), "{bad}: {e}");
        }
        assert_eq!(
            parse(json!({"keywords": ["a"], "limit": 1})).unwrap().limit,
            1
        );
    }

    #[test]
    fn parse_checks_dates_but_not_the_span() {
        // 跨好几年也可以
        let p =
            parse(json!({"keywords": ["a"], "date_from": "2020-01-01", "date_to": "2026-07-01"}));
        assert!(p.is_ok());
        let e =
            parse(json!({"keywords": ["a"], "date_from": "2026-07-03", "date_to": "2026-07-01"}));
        assert!(e.unwrap_err().contains("晚于"));
        let e =
            parse(json!({"keywords": ["a"], "date_from": "2026-08-01", "date_to": "2026-08-02"}));
        assert!(e.unwrap_err().contains("未来"));
        let e = parse(json!({"keywords": ["a"], "date_from": "7/1", "date_to": "2026-07-02"}));
        assert!(e.unwrap_err().contains("date_from"));
    }

    #[test]
    fn parse_reports_a_wrong_type_as_a_format_error() {
        let e = parse(json!({"keywords": "报销"})).unwrap_err();
        assert!(e.starts_with("参数格式错误"), "{e}");
    }

    fn rows(lines: &[&str]) -> Vec<Row> {
        lines
            .iter()
            .map(|line| Row {
                line: line.to_string(),
                citation: Citation {
                    index: 0,
                    app: "app".into(),
                    title: line.to_string(),
                    started_ts: String::new(),
                    ended_ts: String::new(),
                    frame_path: None,
                },
            })
            .collect()
    }

    /// 开头一行；列了几个的话，再写一行列了几个。
    fn head(n: usize) -> Vec<String> {
        let mut lines = vec!["开头".to_string()];
        if n > 0 {
            lines.push(format!("列 {n} 个："));
        }
        lines
    }

    fn write(rows: Vec<Row>, count: usize, max_chars: usize) -> ToolOutput {
        write_list(head, rows, count, 4, max_chars, ChatLang::ZhHans)
    }

    #[test]
    fn the_last_line_says_how_to_see_more() {
        assert_eq!(
            more(6, ChatLang::ZhHans),
            "还有 6 个没列。要看更多，多列几个，或者缩小日期范围、按应用或分类过滤。"
        );
    }

    #[test]
    fn write_list_numbers_from_next_citation_and_says_what_is_left() {
        let out = write(rows(&["a", "b", "c"]), 5, 4000);
        let last = more(2, ChatLang::ZhHans);
        let expected = [
            "开头",
            "列 3 个：",
            "[4] a",
            "[5] b",
            "[6] c",
            last.as_str(),
        ];
        assert_eq!(out.content, expected.join("\n"));
        let indexes: Vec<usize> = out.citations.iter().map(|c| c.index).collect();
        assert_eq!(indexes, [4, 5, 6]);
    }

    #[test]
    fn write_list_has_no_last_line_when_everything_is_listed() {
        let out = write(rows(&["a", "b"]), 2, 4000);
        assert_eq!(out.content, "开头\n列 2 个：\n[4] a\n[5] b");
    }

    #[test]
    fn write_list_lists_fewer_instead_of_cutting_a_line() {
        let all = write(rows(&["aaaa", "bbbb", "cccc"]), 3, 4000);
        // 留出最后一行的位置后，只放得下两行
        let reserved = more(3, ChatLang::ZhHans).chars().count() + 1;
        let max_chars = "开头\n列 3 个：\n[4] aaaa\n[5] bbbb\n".chars().count() + reserved;
        let out = write(rows(&["aaaa", "bbbb", "cccc"]), 3, max_chars);
        assert!(out.content.chars().count() <= max_chars, "{}", out.content);
        assert_eq!(out.citations.len(), 2);
        assert!(out.content.contains("列 2 个：\n[4] aaaa\n[5] bbbb\n"));
        assert!(out.content.ends_with(&more(1, ChatLang::ZhHans)));
        assert_eq!(all.citations.len(), 3);
    }

    #[test]
    fn write_list_without_room_keeps_only_the_head_and_the_count() {
        let out = write(rows(&["a"]), 1, 10);
        assert_eq!(out.content, format!("开头\n{}", more(1, ChatLang::ZhHans)));
        assert!(out.citations.is_empty());
    }

    #[test]
    fn no_dates_means_every_date() {
        assert_eq!(
            date_bounds(None),
            ("0000-01-01".into(), "9999-12-31".into())
        );
        let d = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
        assert_eq!(
            date_bounds(Some((d, d))),
            ("2026-07-01".into(), "2026-07-01".into())
        );
    }

    /// 本机的 grammar 从同一份格式生成，条数和长度的限制要写在格式里。
    #[test]
    fn schema_carries_the_limits() {
        let s = SearchParams::schema("关键词", ChatLang::ZhHans);
        assert_eq!(s["required"], json!(["keywords"]));
        let props = &s["properties"];
        assert_eq!(props["keywords"]["maxItems"], KEYWORDS_MAX);
        assert_eq!(props["keywords"]["items"]["maxLength"], 64);
        assert_eq!(props["keywords"]["description"], "关键词");
        // 应用、分类不限个数
        assert!(props["apps"].get("maxItems").is_none());
        assert!(props["categories"].get("maxItems").is_none());
        assert_eq!(props["limit"]["minimum"], 1);
        assert_eq!(props["limit"]["maximum"], LIMIT_MAX);
        assert_eq!(props["date_from"]["maxLength"], 10);
    }

    #[test]
    fn keywords_are_quoted_in_the_prompt_language() {
        let kws = vec!["报销".to_string(), "7月".to_string()];
        assert_eq!(quote_keywords(&kws, ChatLang::ZhHans), "「报销」「7月」");
        assert_eq!(quote_keywords(&kws, ChatLang::En), "\"报销\", \"7月\"");
    }
}
