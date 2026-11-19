use std::fmt::Display;
use std::sync::LazyLock;

use chrono::NaiveDate;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ai::model::ToolSpec;
use crate::ai::tools::lang::text::{load, Text};
use crate::ai::tools::{ChatLang, ToolError};

pub(super) fn read_args<T: DeserializeOwned>(args: &Value, lang: ChatLang) -> Result<T, ToolError> {
    serde_json::from_value(args.clone()).map_err(|e| ToolError(lang.args_format_err(&e)))
}

pub(super) fn invalid(msg: &str, lang: ChatLang) -> ToolError {
    ToolError(lang.args_invalid(msg))
}

/// Checks a date range and returns its first and last day, both included.
///
/// With only a start, the range ends today. With only an end, it starts at the earliest date.
/// With neither, it returns `None`. A bad format, a start after today, a start after the end,
/// or a span over `max_days` is rejected with a reason written for the model.
pub(super) fn parse_range(
    date_from: Option<&str>,
    date_to: Option<&str>,
    today: NaiveDate,
    max_days: i64,
    lang: ChatLang,
) -> Result<Option<(NaiveDate, NaiveDate)>, String> {
    const EARLIEST: NaiveDate = NaiveDate::from_ymd_opt(0, 1, 1).unwrap();
    let date = |field: &str, s: &str| {
        NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").map_err(|_| lang.err_bad_date(field, s))
    };
    if date_from.is_none() && date_to.is_none() {
        return Ok(None);
    }
    let from = date_from
        .map(|f| date("date_from", f))
        .transpose()?
        .unwrap_or(EARLIEST);
    let to = date_to
        .map(|t| date("date_to", t))
        .transpose()?
        .unwrap_or(today);
    // Check for a future start first. With only a start, the end is today, so a future
    // start would otherwise be reported as a start after the end.
    if from > today {
        return Err(lang.err_from_in_future().into());
    }
    if from > to {
        return Err(lang.err_from_after_to().into());
    }
    if (to - from).num_days() > max_days {
        return Err(lang.err_range_too_long().into());
    }
    Ok(Some((from, to)))
}

/// Cleans a list of short strings: trims each one and drops the empty ones.
///
/// An item over 64 characters is rejected, and the reason names `field`.
pub(super) fn clean_short_strings(
    raw: &[String],
    field: &str,
    lang: ChatLang,
) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for s in raw {
        let t = s.trim();
        if t.is_empty() {
            continue;
        }
        if t.chars().count() > 64 {
            return Err(lang.err_item_too_long(field));
        }
        out.push(t.to_string());
    }
    Ok(out)
}

pub(super) fn spec(name: &str, description: &str, parameters: Value) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
    }
}

/// Builds the schema of a date parameter, which tells the model how to fill it in.
///
/// It only limits the length. Put the format YYYY-MM-DD in `desc`.
pub(super) fn date_schema(desc: &str) -> Value {
    json!({"type": "string", "maxLength": 10, "description": desc})
}

/// Builds the schema of a list of short strings.
///
/// Each item has at most 64 characters. With `max_items` set to `None`, the list has no
/// size limit.
pub(super) fn short_strings_schema(desc: &str, max_items: Option<u32>) -> Value {
    let mut schema =
        json!({"type": "array", "items": {"type": "string", "maxLength": 64}, "description": desc});
    if let Some(n) = max_items {
        schema["maxItems"] = json!(n);
    }
    schema
}

/// `lang/errors.json` 里的句子：参数报错。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorText {
    args_format_err: Text,
    args_invalid: Text,
    err_unknown_tool: Text,
    err_need_range: Text,
    err_bad_date: Text,
    err_from_after_to: Text,
    err_range_too_long: Text,
    err_from_in_future: Text,
    err_keywords_empty: Text,
    err_item_too_long: Text,
    err_too_many_items: Text,
    err_limit_range: Text,
}

const ERRORS_JSON: &str = include_str!("lang/errors.json");

static ERRORS: LazyLock<ErrorText> = LazyLock::new(|| load(ERRORS_JSON));

impl ChatLang {
    /// 参数读不懂，`e` 是读的时候报的错。
    pub(super) fn args_format_err(self, e: &impl Display) -> String {
        ERRORS.args_format_err.fill(self, &[("e", e)])
    }

    /// 参数读懂了，但值不合法。
    pub(super) fn args_invalid(self, msg: &str) -> String {
        ERRORS.args_invalid.fill(self, &[("msg", &msg)])
    }

    /// `known` 是能用的工具名，由登记处给。
    pub(super) fn err_unknown_tool(self, other: &str, known: &str) -> String {
        ERRORS
            .err_unknown_tool
            .fill(self, &[("other", &other), ("known", &known)])
    }

    pub(super) fn err_need_range(self, tool: &str) -> String {
        ERRORS.err_need_range.fill(self, &[("tool", &tool)])
    }

    pub(super) fn err_bad_date(self, field: &str, val: &str) -> String {
        ERRORS
            .err_bad_date
            .fill(self, &[("field", &field), ("val", &val)])
    }

    pub(super) fn err_from_after_to(self) -> &'static str {
        ERRORS.err_from_after_to.get(self)
    }

    pub(super) fn err_range_too_long(self) -> &'static str {
        ERRORS.err_range_too_long.get(self)
    }

    pub(super) fn err_from_in_future(self) -> &'static str {
        ERRORS.err_from_in_future.get(self)
    }

    pub(super) fn err_keywords_empty(self) -> &'static str {
        ERRORS.err_keywords_empty.get(self)
    }

    pub(super) fn err_item_too_long(self, field: &str) -> String {
        ERRORS.err_item_too_long.fill(self, &[("field", &field)])
    }

    pub(super) fn err_too_many_items(self, field: &str, max: usize) -> String {
        ERRORS
            .err_too_many_items
            .fill(self, &[("field", &field), ("max", &max)])
    }

    pub(super) fn err_limit_range(self, max: usize) -> String {
        ERRORS.err_limit_range.fill(self, &[("max", &max)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::tools::lang::text::assert_same_placeholders;

    #[test]
    fn errors_json_reads_and_keeps_placeholders_in_step() {
        LazyLock::force(&ERRORS);
        assert_same_placeholders(ERRORS_JSON);
    }

    #[test]
    fn errors_fill_their_arguments_in_every_language() {
        let langs = [
            ChatLang::ZhHans,
            ChatLang::ZhHant,
            ChatLang::En,
            ChatLang::Ja,
            ChatLang::Pt,
            ChatLang::Es,
        ];
        for lang in langs {
            for s in [
                lang.err_from_after_to(),
                lang.err_range_too_long(),
                lang.err_from_in_future(),
                lang.err_keywords_empty(),
            ] {
                assert!(!s.is_empty(), "{lang:?}");
            }
            assert!(lang.args_format_err(&"boom").contains("boom"));
            assert!(lang.args_invalid("bad-arg").contains("bad-arg"));
            let unknown = lang.err_unknown_tool("nope", "a / b");
            assert!(unknown.contains("nope") && unknown.contains("a / b"));
            assert!(lang.err_need_range("query_stats").contains("query_stats"));
            let date = lang.err_bad_date("date_from", "26-07");
            assert!(date.contains("date_from") && date.contains("26-07"));
            assert!(lang.err_item_too_long("apps").contains("apps"));
            let many = lang.err_too_many_items("keywords", 3);
            assert!(
                many.contains("keywords") && many.contains('3'),
                "{lang:?}: {many}"
            );
            assert!(lang.err_limit_range(20).contains("20"));
        }
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 5).unwrap()
    }

    fn range(from: &str, to: &str) -> Result<Option<(NaiveDate, NaiveDate)>, String> {
        parse_range(Some(from), Some(to), today(), 366, ChatLang::ZhHans)
    }

    #[test]
    fn range_rejects_reversed_future_and_too_long() {
        assert!(range("2026-07-03", "2026-07-01")
            .unwrap_err()
            .contains("晚于"));
        assert!(range("2026-08-01", "2026-08-02")
            .unwrap_err()
            .contains("未来"));
        assert!(range("2020-01-01", "2026-07-01")
            .unwrap_err()
            .contains("366"));
        assert!(range("2026/07/01", "2026-07-02")
            .unwrap_err()
            .contains("date_from"));
    }

    #[test]
    fn range_with_one_end_leaves_the_other_open() {
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let open = |from: Option<&str>, to: Option<&str>| {
            parse_range(from, to, today(), i64::MAX, ChatLang::ZhHans)
        };
        // 只给开始：到今天为止；只给结束：从最早算起；都没给：没有范围
        assert_eq!(
            open(Some("2026-07-01"), None),
            Ok(Some((d("2026-07-01"), today())))
        );
        assert_eq!(
            open(None, Some("2026-07-01")),
            Ok(Some((d("0000-01-01"), d("2026-07-01"))))
        );
        assert_eq!(open(None, None), Ok(None));
        // 给的那一头照样检查；只给开始又在未来，报的是「在未来」
        assert!(open(Some("2026-08-01"), None).unwrap_err().contains("未来"));
        assert!(open(None, Some("7/1")).unwrap_err().contains("date_to"));
        // 有跨度上限时，只给结束会超出上限
        let capped = parse_range(None, Some("2026-07-01"), today(), 366, ChatLang::ZhHans);
        assert!(capped.unwrap_err().contains("366"));
        let both = range(" 2026-07-01 ", "2026-07-05").unwrap().unwrap();
        assert_eq!(both.0, d("2026-07-01"));
    }

    #[test]
    fn short_string_over_64_chars_rejected_with_field_named() {
        let long = vec!["x".repeat(65)];
        let e = clean_short_strings(&long, "apps", ChatLang::ZhHans).unwrap_err();
        assert!(e.contains("apps") && e.contains("64"), "{e}");
        let ok = vec!["x".repeat(64)];
        assert_eq!(
            clean_short_strings(&ok, "apps", ChatLang::ZhHans)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn short_strings_cleaned_but_not_capped() {
        let raw: Vec<String> = [" A ", "", "B", "C", "D", "E", "F"]
            .map(String::from)
            .into();
        assert_eq!(
            clean_short_strings(&raw, "apps", ChatLang::ZhHans).unwrap(),
            ["A", "B", "C", "D", "E", "F"]
        );
    }
}
