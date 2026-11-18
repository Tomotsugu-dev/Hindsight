//! 工具文字的读法：一句话一个键，6 种语言挨在一起。
//!
//! 每个工具的句子放在它旁边的 `lang/工具名.json`，读成一个结构体，一句话一个字段。
//! 几个工具共用的句子放在 `lang/common.json`，见 [`COMMON`]；时长、星期、参数报错
//! 写成 `ChatLang` 的方法，也在这个文件里。

use std::fmt::Display;
use std::sync::LazyLock;

use serde::de::DeserializeOwned;
use serde::Deserialize;

use super::ChatLang;

/// 一句话的 6 种语言。少一种、多一种，都读取失败。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Text {
    #[serde(rename = "zh-Hans")]
    zh_hans: String,
    #[serde(rename = "zh-Hant")]
    zh_hant: String,
    en: String,
    ja: String,
    pt: String,
    es: String,
}

impl Text {
    /// 这种语言的句子。
    pub(super) fn get(&self, lang: ChatLang) -> &str {
        match lang {
            ChatLang::ZhHans => &self.zh_hans,
            ChatLang::ZhHant => &self.zh_hant,
            ChatLang::En => &self.en,
            ChatLang::Ja => &self.ja,
            ChatLang::Pt => &self.pt,
            ChatLang::Es => &self.es,
        }
    }

    /// 这种语言的句子，`{名字}` 换成 `args` 里同名的值。
    ///
    /// 只扫一遍：值里的 `{名字}` 不会再被换掉。`args` 里没有的名字原样留着。
    pub(super) fn fill(&self, lang: ChatLang, args: &[(&str, &dyn Display)]) -> String {
        let mut out = String::new();
        let mut rest = self.get(lang);
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            let value = after.find('}').and_then(|close| {
                let name = &after[..close];
                let (_, value) = args.iter().find(|(n, _)| *n == name)?;
                Some((close, value))
            });
            match value {
                Some((close, value)) => {
                    out.push_str(&value.to_string());
                    rest = &after[close + 1..];
                }
                None => {
                    out.push('{');
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        out
    }
}

/// 读一个 JSON 里的句子。
///
/// JSON 编进了程序，每个 JSON 都有测试读一遍，所以这里不会失败。
pub(super) fn load<T: DeserializeOwned>(json: &str) -> T {
    serde_json::from_str(json).expect("工具文字的 JSON 读不懂，测试应该先发现")
}

/// `lang/common.json` 里的句子：几个工具共用的。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Common {
    /// 一个关键词加上引号。
    pub(super) quote: Text,
    /// 几个加了引号的关键词之间。
    pub(super) quote_sep: Text,
    /// 搜索工具的参数说明：起始日期。
    pub(super) param_date_from: Text,
    /// 搜索工具的参数说明：结束日期。
    pub(super) param_date_to: Text,
    /// 搜索工具的参数说明：只搜哪些应用。
    pub(super) param_apps: Text,
    /// 搜索工具的参数说明：只搜哪些分类。
    pub(super) param_categories: Text,
    /// 搜索工具的参数说明：最多列几个。
    pub(super) param_limit: Text,
    /// 没列全时的最后一行：还有几个，怎么看更多。
    pub(super) more: Text,
    /// 时长，有小时的。
    duration_hours: Text,
    /// 时长，不到一小时的。
    duration_minutes: Text,
    monday: Text,
    tuesday: Text,
    wednesday: Text,
    thursday: Text,
    friday: Text,
    saturday: Text,
    sunday: Text,
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

const COMMON_JSON: &str = include_str!("lang/common.json");

pub(super) static COMMON: LazyLock<Common> = LazyLock::new(|| load(COMMON_JSON));

/// 几个工具共用的句子，从 `lang/common.json` 读。
impl ChatLang {
    /// 时长写到分钟：1 小时 4 分钟、38 分钟。
    pub(super) fn fmt_secs(self, secs: i64) -> String {
        let (h, m) = (secs / 3600, (secs % 3600) / 60);
        if h > 0 {
            COMMON.duration_hours.fill(self, &[("h", &h), ("m", &m)])
        } else {
            COMMON.duration_minutes.fill(self, &[("m", &m)])
        }
    }

    /// 星期几。`iso` 是 1 到 7，1 是星期一；别的值算星期日。
    ///
    /// 用量工具逐日的行和聊天的系统提示词都用：模型自己算星期几常出错。
    pub(crate) fn weekday(self, iso: &str) -> &'static str {
        let c = &*COMMON;
        let day = match iso {
            "1" => &c.monday,
            "2" => &c.tuesday,
            "3" => &c.wednesday,
            "4" => &c.thursday,
            "5" => &c.friday,
            "6" => &c.saturday,
            _ => &c.sunday,
        };
        day.get(self)
    }

    /// 参数读不懂，`e` 是读的时候报的错。
    pub(super) fn args_format_err(self, e: &impl Display) -> String {
        COMMON.args_format_err.fill(self, &[("e", e)])
    }

    /// 参数读懂了，但值不合法。
    pub(super) fn args_invalid(self, msg: &str) -> String {
        COMMON.args_invalid.fill(self, &[("msg", &msg)])
    }

    /// `known` 是能用的工具名，由登记处给。
    pub(super) fn err_unknown_tool(self, other: &str, known: &str) -> String {
        COMMON
            .err_unknown_tool
            .fill(self, &[("other", &other), ("known", &known)])
    }

    pub(super) fn err_need_range(self, tool: &str) -> String {
        COMMON.err_need_range.fill(self, &[("tool", &tool)])
    }

    pub(super) fn err_bad_date(self, field: &str, val: &str) -> String {
        COMMON
            .err_bad_date
            .fill(self, &[("field", &field), ("val", &val)])
    }

    pub(super) fn err_from_after_to(self) -> &'static str {
        COMMON.err_from_after_to.get(self)
    }

    pub(super) fn err_range_too_long(self) -> &'static str {
        COMMON.err_range_too_long.get(self)
    }

    pub(super) fn err_from_in_future(self) -> &'static str {
        COMMON.err_from_in_future.get(self)
    }

    pub(super) fn err_keywords_empty(self) -> &'static str {
        COMMON.err_keywords_empty.get(self)
    }

    pub(super) fn err_item_too_long(self, field: &str) -> String {
        COMMON.err_item_too_long.fill(self, &[("field", &field)])
    }

    pub(super) fn err_too_many_items(self, field: &str, max: usize) -> String {
        COMMON
            .err_too_many_items
            .fill(self, &[("field", &field), ("max", &max)])
    }

    pub(super) fn err_limit_range(self, max: usize) -> String {
        COMMON.err_limit_range.fill(self, &[("max", &max)])
    }
}

/// 检查每句话的 6 种语言用的占位符一样。每个 JSON 的测试都调它。
#[cfg(test)]
pub(super) fn assert_same_placeholders(json: &str) {
    use std::collections::{BTreeMap, BTreeSet};

    fn placeholders(s: &str) -> BTreeSet<&str> {
        s.split('{')
            .skip(1)
            .filter_map(|part| part.split_once('}').map(|(name, _)| name))
            .filter(|name| {
                !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_')
            })
            .collect()
    }

    let all: BTreeMap<String, BTreeMap<String, String>> = serde_json::from_str(json).unwrap();
    for (key, langs) in &all {
        let zh = placeholders(&langs["zh-Hans"]);
        for (lang, s) in langs {
            assert_eq!(
                placeholders(s),
                zh,
                "{key} 的 {lang} 和 zh-Hans 占位符不一样"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LANGS: [ChatLang; 6] = [
        ChatLang::ZhHans,
        ChatLang::ZhHant,
        ChatLang::En,
        ChatLang::Ja,
        ChatLang::Pt,
        ChatLang::Es,
    ];

    fn text(zh: &str, en: &str) -> Text {
        Text {
            zh_hans: zh.into(),
            zh_hant: zh.into(),
            en: en.into(),
            ja: zh.into(),
            pt: en.into(),
            es: en.into(),
        }
    }

    #[test]
    fn get_picks_the_language() {
        let t: Text = load(
            r#"{"zh-Hans": "简", "zh-Hant": "繁", "en": "en",
                "ja": "日", "pt": "pt", "es": "es"}"#,
        );
        let got: Vec<&str> = LANGS.iter().map(|&l| t.get(l)).collect();
        assert_eq!(got, ["简", "繁", "en", "日", "pt", "es"]);
    }

    #[test]
    fn a_missing_or_extra_language_fails_to_read() {
        let five = r#"{"zh-Hans": "a", "zh-Hant": "a", "en": "a", "ja": "a", "pt": "a"}"#;
        assert!(serde_json::from_str::<Text>(five).is_err());
        let seven = r#"{"zh-Hans": "a", "zh-Hant": "a", "en": "a", "ja": "a",
                        "pt": "a", "es": "a", "fr": "a"}"#;
        assert!(serde_json::from_str::<Text>(seven).is_err());
    }

    #[test]
    fn fill_replaces_each_name_with_its_value() {
        let t = text("还有 {rest} 个，共 {dur}", "{rest} more, {dur}");
        let args: [(&str, &dyn Display); 2] = [("rest", &3), ("dur", &"5 分钟")];
        assert_eq!(t.fill(ChatLang::ZhHans, &args), "还有 3 个，共 5 分钟");
        assert_eq!(t.fill(ChatLang::En, &args), "3 more, 5 分钟");
    }

    #[test]
    fn fill_does_not_replace_names_inside_values() {
        // 关键词是模型给的，里面可能正好有 {count}
        let t = text("{kw}：{count} 个", "{kw}: {count}");
        let out = t.fill(ChatLang::ZhHans, &[("kw", &"「{count}」"), ("count", &2)]);
        assert_eq!(out, "「{count}」：2 个");
    }

    #[test]
    fn fill_keeps_unknown_names_and_lone_braces() {
        let t = text("{a} {b} { c", "");
        assert_eq!(t.fill(ChatLang::ZhHans, &[("a", &1)]), "1 {b} { c");
    }

    #[test]
    fn common_json_reads_and_keeps_placeholders_in_step() {
        LazyLock::force(&COMMON);
        assert_same_placeholders(COMMON_JSON);
    }

    #[test]
    #[should_panic(expected = "占位符不一样")]
    fn different_placeholders_are_caught() {
        assert_same_placeholders(
            r#"{"more": {"zh-Hans": "还有 {rest} 个", "zh-Hant": "還有 {rest} 個",
                "en": "{n} more", "ja": "{rest}", "pt": "{rest}", "es": "{rest}"}}"#,
        );
    }

    #[test]
    fn durations_show_hours_only_when_there_are_some() {
        let zh = ChatLang::ZhHans;
        assert_eq!(zh.fmt_secs(59), "0 分钟");
        assert_eq!(zh.fmt_secs(38 * 60), "38 分钟");
        assert_eq!(zh.fmt_secs(64 * 60), "1 小时 4 分钟");
        assert_eq!(ChatLang::En.fmt_secs(3 * 3600 + 5 * 60), "3 hr 5 min");
        assert_eq!(ChatLang::Ja.fmt_secs(5 * 60), "5分");
    }

    #[test]
    fn weekday_counts_from_monday_and_falls_back_to_sunday() {
        let names: Vec<&str> = (1..=7)
            .map(|d| ChatLang::ZhHans.weekday(&d.to_string()))
            .collect();
        assert_eq!(
            names,
            ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
        );
        assert_eq!(ChatLang::En.weekday("3"), "Wednesday");
        // 认不出的值不 panic
        for bad in ["0", "8", "x"] {
            assert_eq!(ChatLang::ZhHans.weekday(bad), "周日", "{bad}");
        }
    }

    #[test]
    fn errors_fill_their_arguments_in_every_language() {
        for lang in LANGS {
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
}
