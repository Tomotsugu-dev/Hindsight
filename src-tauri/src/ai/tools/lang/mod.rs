//! 聊天用的语言 `ChatLang`，以及时长和星期怎么写（`time.json`）。
//! 工具的文字在各个工具旁边的 JSON 里，读法见 [`text`]；
//! 聊天自己的文字在 [`crate::chat::lang`]。
//!
//! 为什么工具骨架也要本地化:模型"读"的资料若全是中文(头部/无命中提示/
//! 统计措辞),英文提问也会被证据语言拽回中文,本地小模型尤甚。因此所有
//! **模型可见**的骨架文本(系统提示词、工具结果头部、校验错误、降级文案)
//! 都按界面语言生成;证据卡等结构化数据不受影响。

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

/// `time.json` 里的句子：时长和星期。工具和聊天都用。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TimeText {
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
}

const TIME_JSON: &str = include_str!("time.json");

static TIME: LazyLock<TimeText> = LazyLock::new(|| load(TIME_JSON));

impl ChatLang {
    /// 时长写到分钟：1 小时 4 分钟、38 分钟。
    pub(super) fn fmt_secs(self, secs: i64) -> String {
        let (h, m) = (secs / 3600, (secs % 3600) / 60);
        if h > 0 {
            TIME.duration_hours.fill(self, &[("h", &h), ("m", &m)])
        } else {
            TIME.duration_minutes.fill(self, &[("m", &m)])
        }
    }

    /// 星期几。`iso` 是 1 到 7，1 是星期一；别的值算星期日。
    ///
    /// 用量工具逐日的行和聊天的系统提示词都用：模型自己算星期几常出错。
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
    /// BCP-47 宽松前缀匹配。None(旧前端没传)回落简中(历史行为不变);
    /// 认不出的标签回落英文(最通用)。
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
        // 认不出的值不 panic
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
        // 旧前端没传 → 维持历史行为(简中);认不出的 → 英文
        assert_eq!(ChatLang::from_tag(None), ChatLang::ZhHans);
        assert_eq!(ChatLang::from_tag(Some("fr")), ChatLang::En);
    }
}
