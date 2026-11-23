//! This module defines `ChatLang`, the language that chat and the tools share. It also reads how
//! to write durations and weekdays from `time.json`. Tool text is in JSON files next to each
//! tool, and [`text`] reads it. Chat's own text is in [`crate::ai::features::chat::lang`].
//!
//! All text the model reads is in the prompt language. If tool results were always in Chinese,
//! the model would drift back to Chinese even when the user asks in English.

pub(super) mod text;

use std::sync::LazyLock;

use serde::Deserialize;

use self::text::{load, Text};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatLang {
    ZhHans,
    ZhHant,
    En,
    Ja,
    Pt,
    Es,
}

/// This struct matches `time.json`. It holds how to write durations and the names of the seven
/// weekdays.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TimeText {
    /// A duration with hours uses this, such as 1 hr 4 min.
    duration_hours: Text,
    /// A duration under an hour uses this, such as 38 min.
    duration_minutes: Text,
    monday: Text,
    tuesday: Text,
    wednesday: Text,
    thursday: Text,
    friday: Text,
    saturday: Text,
    sunday: Text,
}

const TIME_JSON: &str = include_str!("time.json");

static TIME: LazyLock<TimeText> = LazyLock::new(|| load(TIME_JSON));

impl ChatLang {
    /// Writes a number of seconds as a duration for the model, in the chat language.
    pub(super) fn fmt_secs(self, secs: i64) -> String {
        let (h, m) = (secs / 3600, (secs % 3600) / 60);
        if h > 0 {
            TIME.duration_hours.fill(self, &[("h", &h), ("m", &m)])
        } else {
            TIME.duration_minutes.fill(self, &[("m", &m)])
        }
    }

    /// Returns the name of a weekday in the chat language.
    ///
    /// `iso` is a string from 1 to 7: 1 is Monday and 7 is Sunday. Models often get the weekday
    /// wrong, so the program works it out and writes it for them.
    pub(crate) fn weekday(self, iso: &str) -> &'static str {
        let t = &*TIME;
        let day = match iso {
            "1" => &t.monday,
            "2" => &t.tuesday,
            "3" => &t.wednesday,
            "4" => &t.thursday,
            "5" => &t.friday,
            "6" => &t.saturday,
            _ => &t.sunday,
        };
        day.get(self)
    }
}

impl ChatLang {
    /// Picks a `ChatLang` from the language code at the start of a tag.
    ///
    /// A `zh` tag that contains `tw`, `hk`, `mo` or `hant` picks Traditional Chinese. Other `zh`
    /// tags pick Simplified Chinese. An empty tag picks Simplified Chinese, and an unknown tag
    /// picks English.
    pub fn from_tag(tag: Option<&str>) -> Self {
        let t = tag.unwrap_or("").trim().to_ascii_lowercase();
        if t.is_empty() {
            return Self::ZhHans;
        }
        if t.starts_with("zh") {
            if t.contains("tw") || t.contains("hk") || t.contains("mo") || t.contains("hant") {
                Self::ZhHant
            } else {
                Self::ZhHans
            }
        } else if t.starts_with("en") {
            Self::En
        } else if t.starts_with("ja") {
            Self::Ja
        } else if t.starts_with("pt") {
            Self::Pt
        } else if t.starts_with("es") {
            Self::Es
        } else {
            Self::En
        }
    }
}

#[cfg(test)]
mod tests {
    use super::text::assert_same_placeholders;
    use super::*;

    #[test]
    fn time_json_reads_and_keeps_placeholders_in_step() {
        LazyLock::force(&TIME);
        assert_same_placeholders(TIME_JSON);
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
        for bad in ["0", "8", "x"] {
            assert_eq!(ChatLang::ZhHans.weekday(bad), "周日", "{bad}");
        }
    }

    #[test]
    fn tag_parsing_prefix_and_fallbacks() {
        assert_eq!(ChatLang::from_tag(Some("zh-CN")), ChatLang::ZhHans);
        assert_eq!(ChatLang::from_tag(Some("zh-TW")), ChatLang::ZhHant);
        assert_eq!(ChatLang::from_tag(Some("zh-Hant-HK")), ChatLang::ZhHant);
        assert_eq!(ChatLang::from_tag(Some("en-US")), ChatLang::En);
        assert_eq!(ChatLang::from_tag(Some("ja")), ChatLang::Ja);
        assert_eq!(ChatLang::from_tag(Some("pt-BR")), ChatLang::Pt);
        assert_eq!(ChatLang::from_tag(Some("es")), ChatLang::Es);
        assert_eq!(ChatLang::from_tag(Some("es-MX")), ChatLang::Es);
        assert_eq!(ChatLang::from_tag(None), ChatLang::ZhHans);
        assert_eq!(ChatLang::from_tag(Some("fr")), ChatLang::En);
    }
}
