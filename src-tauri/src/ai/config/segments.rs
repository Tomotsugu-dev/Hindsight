//! Day time segments and localized defaults.

use serde::{Deserialize, Serialize};

/// One time segment of a day used for daily report summarization.
///
/// Valid bounds: `start_hour < end_hour` and `end_hour <= 24` (crossing midnight is not
/// supported). Enforced by [`sanitize()`](super::sanitize).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DaySegment {
    pub label: String,
    pub start_hour: u8,
    pub end_hour: u8,
    /// Optional hex background color (`#rrggbb`); empty uses the UI default gradient.
    pub color: String,
}

/// Returns the default five segments covering 00:00–24:00 with labels localized for `lang`.
///
/// Unknown language tags fall back to English.
pub fn default_segments_for(lang: &str) -> Vec<DaySegment> {
    let labels: [&str; 5] = match lang {
        "zh" => ["深夜", "早上", "上午", "下午", "晚上"],
        "tw" => ["凌晨", "清晨", "上午", "下午", "晚上"],
        "ja" => ["深夜", "早朝", "午前", "午後", "夜"],
        "pt" => ["Madrugada", "Manhã cedo", "Manhã", "Tarde", "Noite"],
        "es" => ["Madrugada", "Amanecer", "Mañana", "Tarde", "Noche"],
        _ => [
            "Late Night",
            "Early Morning",
            "Morning",
            "Afternoon",
            "Evening",
        ],
    };
    let ranges: [(u8, u8); 5] = [(0, 6), (6, 9), (9, 12), (12, 18), (18, 24)];
    labels
        .into_iter()
        .zip(ranges)
        .map(|(label, (start_hour, end_hour))| DaySegment {
            label: label.to_string(),
            start_hour,
            end_hour,
            // Empty = auto color computed by the frontend from the segment's time of day.
            color: String::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- default_segments_for ----------

    #[test]
    fn default_segments_cover_full_day_seamlessly_for_every_lang() {
        // 时段是"全天覆盖、无缝衔接"的约定：首段起点 0、末段终点 24、相邻段首尾相接。
        // 未知语言与空串也必须给出可用组——首启 locale 探测失败不能让默认配置残缺。
        for lang in ["zh", "tw", "en", "ja", "pt", "ko", ""] {
            let segs = default_segments_for(lang);
            assert_eq!(segs.len(), 5, "lang={lang:?} 应为 5 段");
            assert_eq!(segs[0].start_hour, 0, "lang={lang:?} 首段从 0 点起");
            assert_eq!(segs[4].end_hour, 24, "lang={lang:?} 末段到 24 点止");
            for w in segs.windows(2) {
                assert!(
                    w[0].start_hour < w[0].end_hour,
                    "lang={lang:?} 段内 start<end"
                );
                assert_eq!(w[0].end_hour, w[1].start_hour, "lang={lang:?} 相邻段无缝");
            }
            // 边界值是产品约定的固定五段划分
            let bounds: Vec<(u8, u8)> = segs.iter().map(|s| (s.start_hour, s.end_hour)).collect();
            assert_eq!(bounds, vec![(0, 6), (6, 9), (9, 12), (12, 18), (18, 24)]);
            // 默认色留空 = UI 自动渐变
            assert!(
                segs.iter().all(|s| s.color.is_empty()),
                "lang={lang:?} 默认不带色"
            );
        }
    }

    #[test]
    fn default_segments_labels_localized_with_en_fallback() {
        let zh: Vec<String> = default_segments_for("zh")
            .iter()
            .map(|s| s.label.clone())
            .collect();
        let en: Vec<String> = default_segments_for("en")
            .iter()
            .map(|s| s.label.clone())
            .collect();
        let ja: Vec<String> = default_segments_for("ja")
            .iter()
            .map(|s| s.label.clone())
            .collect();
        let pt: Vec<String> = default_segments_for("pt")
            .iter()
            .map(|s| s.label.clone())
            .collect();
        assert_ne!(zh, en);
        assert_ne!(zh, ja);
        assert_ne!(en, pt);
        assert_eq!(en[0], "Late Night");
        assert_eq!(zh[4], "晚上");
        assert!(en.iter().all(|l| l.is_ascii()));

        let tw: Vec<String> = default_segments_for("tw")
            .iter()
            .map(|s| s.label.clone())
            .collect();
        let ko: Vec<String> = default_segments_for("ko")
            .iter()
            .map(|s| s.label.clone())
            .collect();
        let empty: Vec<String> = default_segments_for("")
            .iter()
            .map(|s| s.label.clone())
            .collect();
        assert_eq!(tw, vec!["凌晨", "清晨", "上午", "下午", "晚上"]);
        assert_ne!(tw, zh);
        assert_eq!(ko, en, "unknown language falls back to English");
        assert_eq!(empty, en, "empty language falls back to English");
        assert!(zh.iter().all(|l| !l.is_empty()));
    }
}
