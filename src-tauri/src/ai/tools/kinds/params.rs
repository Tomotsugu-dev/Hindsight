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

/// 检查关键词：最多取前 3 个，去掉首尾空白，空的丢掉。一个都不剩就不合法。
pub(super) fn clean_keywords(raw: &[String], lang: ChatLang) -> Result<Vec<String>, String> {
    let out = clean_names(raw, 3, "keywords", lang)?;
    if out.is_empty() {
        return Err(lang.err_keywords_empty().into());
    }
    Ok(out)
}

/// 检查一列名字：最多取前 `max_items` 个，去掉首尾空白，空的丢掉。
///
/// 先取前几个、再丢空的，所以空的也占名额。某个超过 64 字就不合法，原因里写出 `field`。
pub(super) fn clean_names(
    raw: &[String],
    max_items: usize,
    field: &str,
    lang: ChatLang,
) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for s in raw.iter().take(max_items) {
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

/// 参数格式：一个日期。
pub(super) fn date_schema(desc: &str) -> Value {
    json!({"type": "string", "maxLength": 10, "description": format!("{desc},格式 YYYY-MM-DD")})
}

/// 参数格式：一列名字，每个最多 64 字。
pub(super) fn names_schema(desc: &str, max_items: u32) -> Value {
    json!({"type": "array", "items": {"type": "string", "maxLength": 64}, "maxItems": max_items, "description": desc})
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
    fn blank_only_keywords_rejected() {
        // 全是空白，清洗后为空——必须报「不能为空」，否则空的搜索条件会在 SQL 层才出错
        let e = clean_keywords(&[" ".into(), "".into()], ChatLang::ZhHans).unwrap_err();
        assert!(e.contains("不能为空"), "{e}");
    }

    #[test]
    fn keywords_taken_three_then_trimmed() {
        let kws: Vec<String> = ["  rust  ", "go", "zig", "c"].map(String::from).into();
        assert_eq!(
            clean_keywords(&kws, ChatLang::ZhHans).unwrap(),
            ["rust", "go", "zig"]
        );
        // 先取前 3 个再丢空的：空白也占名额，第 4 个词不会补进来
        let kws: Vec<String> = [" ", "a", "b", "c"].map(String::from).into();
        assert_eq!(clean_keywords(&kws, ChatLang::ZhHans).unwrap(), ["a", "b"]);
    }

    #[test]
    fn name_over_64_chars_rejected_with_field_named() {
        // 按字符数算：65 个不合法，64 个可以；原因里要写出字段名，模型才知道改哪个参数
        let long = vec!["x".repeat(65)];
        let e = clean_names(&long, 5, "apps", ChatLang::ZhHans).unwrap_err();
        assert!(e.contains("apps") && e.contains("64"), "{e}");
        let ok = vec!["x".repeat(64)];
        assert_eq!(
            clean_names(&ok, 5, "apps", ChatLang::ZhHans).unwrap().len(),
            1
        );
    }

    #[test]
    fn names_capped_and_cleaned() {
        let raw: Vec<String> = [" A ", "", "B", "C", "D", "E", "F"]
            .map(String::from)
            .into();
        // 取前 5 个 [" A ","","B","C","D"]，清洗后剩 4 个；E、F 被截掉
        assert_eq!(
            clean_names(&raw, 5, "apps", ChatLang::ZhHans).unwrap(),
            ["A", "B", "C", "D"]
        );
    }
}
