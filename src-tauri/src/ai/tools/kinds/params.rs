//! 几个工具共用的参数：读模型传来的参数、检查日期和名字列表，以及参数格式里的日期和名字列表。

use chrono::NaiveDate;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::ai::model::ToolSpec;
use crate::ai::tools::{ChatLang, ToolError};

/// 把模型传来的参数读成 `T`。读不懂时，原因写给模型看。
pub(super) fn read_args<T: DeserializeOwned>(args: &Value, lang: ChatLang) -> Result<T, ToolError> {
    serde_json::from_value(args.clone()).map_err(|e| ToolError(lang.args_format_err(&e)))
}

/// 参数读懂了，但值不合法。原因写给模型看。
pub(super) fn invalid(msg: &str, lang: ChatLang) -> ToolError {
    ToolError(lang.args_invalid(msg))
}

/// 检查日期范围。两头都给了才算给了范围；只给一头，当作没给。
///
/// 开始晚于结束、跨度超过 `max_days`、开始晚于今天，都不合法。
pub(super) fn parse_range(
    date_from: Option<&str>,
    date_to: Option<&str>,
    today: NaiveDate,
    max_days: i64,
    lang: ChatLang,
) -> Result<Option<(NaiveDate, NaiveDate)>, String> {
    let (Some(f), Some(t)) = (date_from, date_to) else {
        return Ok(None);
    };
    let from = NaiveDate::parse_from_str(f.trim(), "%Y-%m-%d")
        .map_err(|_| lang.err_bad_date("date_from", f))?;
    let to = NaiveDate::parse_from_str(t.trim(), "%Y-%m-%d")
        .map_err(|_| lang.err_bad_date("date_to", t))?;
    if from > to {
        return Err(lang.err_from_after_to().into());
    }
    if (to - from).num_days() > max_days {
        return Err(lang.err_range_too_long().into());
    }
    if from > today {
        return Err(lang.err_from_in_future().into());
    }
    Ok(Some((from, to)))
}

/// 检查一列短字符串：去掉首尾空白，空的丢掉。
///
/// 某个超过 64 字就不合法，原因里写出 `field`。
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

/// 组一个工具说明。
pub(super) fn spec(name: &str, description: &str, parameters: Value) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
    }
}

/// 参数格式：一个日期。`desc` 里要写明格式 YYYY-MM-DD。
pub(super) fn date_schema(desc: &str) -> Value {
    json!({"type": "string", "maxLength": 10, "description": desc})
}

/// 参数格式：一列短字符串，每个最多 64 字。`max_items` 是 `None` 时不限个数。
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
        assert!(range("2026-07-10", "2026-07-01")
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
    fn range_needs_both_ends() {
        // 只给一头，当作没给范围
        let only_from = parse_range(Some("2026-07-01"), None, today(), 366, ChatLang::ZhHans);
        assert_eq!(only_from, Ok(None));
        let both = range(" 2026-07-01 ", "2026-07-05").unwrap().unwrap();
        assert_eq!(both.0, NaiveDate::from_ymd_opt(2026, 7, 1).unwrap());
    }

    #[test]
    fn short_string_over_64_chars_rejected_with_field_named() {
        // 按字符数算：65 个不合法，64 个可以；原因里要写出字段名，模型才知道改哪个参数
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
        // 去掉空白和空的，不限个数
        assert_eq!(
            clean_short_strings(&raw, "apps", ChatLang::ZhHans).unwrap(),
            ["A", "B", "C", "D", "E", "F"]
        );
    }
}
