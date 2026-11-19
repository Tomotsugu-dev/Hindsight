//! This module reads the JSON files of tool text into structs. It also gets a sentence in one
//! language and fills in its placeholders.
//!
//! In the JSON, each sentence has one key, and the key holds the sentence in 6 languages. Each
//! JSON file sits in a `lang/` folder next to the file that reads it. `time.json` is the
//! exception: it sits in this folder. Each JSON file is read into one struct, with one field per
//! sentence.

use std::fmt::Display;

use serde::de::DeserializeOwned;
use serde::Deserialize;

use super::ChatLang;

/// `Text` holds one sentence in 6 languages. Reading fails if the JSON has one language too few
/// or too many.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::ai::tools) struct Text {
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
    /// Returns the sentence for `lang`.
    pub(in crate::ai::tools) fn get(&self, lang: ChatLang) -> &str {
        match lang {
            ChatLang::ZhHans => &self.zh_hans,
            ChatLang::ZhHant => &self.zh_hant,
            ChatLang::En => &self.en,
            ChatLang::Ja => &self.ja,
            ChatLang::Pt => &self.pt,
            ChatLang::Es => &self.es,
        }
    }

    /// Replaces each `{name}` in the sentence for `lang` with the value of the same name in
    /// `args`, and returns the result.
    ///
    /// It scans the sentence once, so a `{name}` inside a value is not replaced.
    pub(in crate::ai::tools) fn fill(
        &self,
        lang: ChatLang,
        args: &[(&str, &dyn Display)],
    ) -> String {
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

/// Reads JSON into the text struct `T`, and panics if it cannot.
///
/// The JSON is built into the program, and a test reads each file, so this does not panic at
/// run time.
pub(in crate::ai::tools) fn load<T: DeserializeOwned>(json: &str) -> T {
    serde_json::from_str(json).expect("built-in tool text JSON should match its struct")
}

/// 检查每句话的 6 种语言用的占位符一样，以 zh-Hans 为准。每个 JSON 的测试都调它。
///
/// 只有 `{}` 里全是小写字母和下划线的才算占位符。
#[cfg(test)]
pub(in crate::ai::tools) fn assert_same_placeholders(json: &str) {
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
    #[should_panic(expected = "占位符不一样")]
    fn different_placeholders_are_caught() {
        assert_same_placeholders(
            r#"{"more": {"zh-Hans": "还有 {rest} 个", "zh-Hant": "還有 {rest} 個",
                "en": "{n} more", "ja": "{rest}", "pt": "{rest}", "es": "{rest}"}}"#,
        );
    }
}
