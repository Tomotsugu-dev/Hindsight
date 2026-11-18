//! 找：按关键词找 X 出现在哪。
//!
//! 两个搜索工具的参数一样，都是 [`SearchParams`]。

pub(in crate::ai::tools) mod activity;
pub(in crate::ai::tools) mod text;

use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{json, Value};

use super::params::{clean_names, date_schema, invalid, names_schema, parse_range, read_args};
use crate::ai::tools::{ChatLang, ToolError};

/// 关键词最多几个。
const KEYWORDS_MAX: usize = 3;
/// 应用、分类最多各几个。
const NAMES_MAX: usize = 5;
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

    /// 给模型看的参数格式。`keywords_desc` 写关键词在哪里比对。
    fn schema(keywords_desc: &str) -> Value {
        json!({"type": "object", "properties": {
            "keywords": names_schema(keywords_desc, KEYWORDS_MAX as u32),
            "date_from": date_schema("起始日期,可选;不给日期就搜全部"),
            "date_to": date_schema("结束日期,可选"),
            "apps": names_schema("只搜这些应用,可选", NAMES_MAX as u32),
            "categories": names_schema("只搜这些分类里的应用(如'游戏'),可选", NAMES_MAX as u32),
            "limit": {"type": "integer", "minimum": 1, "maximum": LIMIT_MAX,
                      "description": format!("最多列几个,默认 {LIMIT_DEFAULT}")}
        }, "required": ["keywords"]})
    }
}

/// 检查参数的值，补上默认值。不合法时返回原因。
fn check(args: &Args, today: NaiveDate, lang: ChatLang) -> Result<SearchParams, String> {
    let keywords = names(&args.keywords, KEYWORDS_MAX, "keywords", lang)?;
    if keywords.is_empty() {
        return Err(lang.err_keywords_empty().into());
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
        apps: names(&args.apps, NAMES_MAX, "apps", lang)?,
        categories: names(&args.categories, NAMES_MAX, "categories", lang)?,
        limit,
    })
}

/// 检查一列名字：去掉首尾空白，丢掉空的。剩下的超过 `max` 个就不合法。
///
/// 多给的不悄悄丢掉：模型以为都用上了，答案会错。
fn names(raw: &[String], max: usize, field: &str, lang: ChatLang) -> Result<Vec<String>, String> {
    let out = clean_names(raw, usize::MAX, field, lang)?;
    if out.len() > max {
        return Err(lang.err_too_many_items(field, max));
    }
    Ok(out)
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
    fn parse_rejects_more_than_five_apps_or_categories() {
        let six: Vec<String> = (0..6).map(|i| format!("app{i}")).collect();
        let e = parse(json!({"keywords": ["a"], "apps": six})).unwrap_err();
        assert!(e.contains("apps") && e.contains('5'), "{e}");
        let e = parse(json!({"keywords": ["a"], "categories": six})).unwrap_err();
        assert!(e.contains("categories"), "{e}");
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

    /// 本机的 grammar 从同一份格式生成，条数和长度的限制要写在格式里。
    #[test]
    fn schema_carries_the_limits() {
        let s = SearchParams::schema("关键词");
        assert_eq!(s["required"], json!(["keywords"]));
        let props = &s["properties"];
        assert_eq!(props["keywords"]["maxItems"], KEYWORDS_MAX);
        assert_eq!(props["keywords"]["items"]["maxLength"], 64);
        assert_eq!(props["keywords"]["description"], "关键词");
        assert_eq!(props["apps"]["maxItems"], NAMES_MAX);
        assert_eq!(props["categories"]["maxItems"], NAMES_MAX);
        assert_eq!(props["limit"]["minimum"], 1);
        assert_eq!(props["limit"]["maximum"], LIMIT_MAX);
        assert_eq!(props["date_from"]["maxLength"], 10);
    }
}
