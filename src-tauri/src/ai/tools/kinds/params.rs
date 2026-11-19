use chrono::NaiveDate;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::ai::model::ToolSpec;
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

#[cfg(test)]
mod tests {
    use super::*;

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
