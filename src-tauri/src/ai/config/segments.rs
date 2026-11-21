//! 一天怎么分段：时段的结构、默认分段、时段颜色的校验。

use serde::{Deserialize, Serialize};

/// 一天内的一个时段，AI 按段聚合截图 + 活动做总结。
///
/// 取值约束：`start_hour ∈ 0..=23`、`end_hour ∈ 1..=24`、`start_hour < end_hour`。
/// 不支持跨午夜（晚段最大 `[18, 24]`）。约束在 [`sanitize`](super::sanitize()) 里强制。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AiSegment {
    pub label: String,
    pub start_hour: u8,
    pub end_hour: u8,
    /// 用户自定义底色，hex 格式 `#rrggbb`；空字符串 = 走 UI 自动按时段渐变
    pub color: String,
}

/// 默认 5 段，覆盖整 24 小时（00-06 / 06-09 / 09-12 / 12-18 / 18-24）；
/// 标签按用户语言取一套。新装首启时通过 [`crate::platform::system_language`] 拿系统语言。
pub fn default_segments_for(lang: &str) -> Vec<AiSegment> {
    let labels: [&str; 5] = match lang {
        "en" => [
            "Late Night",
            "Early Morning",
            "Morning",
            "Afternoon",
            "Evening",
        ],
        "ja" => ["深夜", "早朝", "午前", "午後", "夜"],
        "pt" => ["Madrugada", "Manhã cedo", "Manhã", "Tarde", "Noite"],
        "es" => ["Madrugada", "Amanecer", "Mañana", "Tarde", "Noche"],
        _ => ["深夜", "早上", "上午", "下午", "晚上"],
    };
    let ranges: [(u8, u8); 5] = [(0, 6), (6, 9), (9, 12), (12, 18), (18, 24)];
    labels
        .into_iter()
        .zip(ranges)
        .map(|(label, (start_hour, end_hour))| AiSegment {
            label: label.to_string(),
            start_hour,
            end_hour,
            color: String::new(),
        })
        .collect()
}

/// 校验 hex 颜色：接受 `#rgb` / `#rrggbb`，统一返回小写 `#rrggbb`；非法值置空。
pub(super) fn sanitize_hex_color(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return String::new();
    }
    let body = match s.strip_prefix('#') {
        Some(b) => b,
        None => return String::new(),
    };
    let valid_len = matches!(body.len(), 3 | 6);
    if !valid_len || !body.chars().all(|c| c.is_ascii_hexdigit()) {
        return String::new();
    }
    if body.len() == 3 {
        let mut out = String::with_capacity(7);
        out.push('#');
        for c in body.chars() {
            let lc = c.to_ascii_lowercase();
            out.push(lc);
            out.push(lc);
        }
        out
    } else {
        let mut out = String::with_capacity(7);
        out.push('#');
        for c in body.chars() {
            out.push(c.to_ascii_lowercase());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- sanitize_hex_color（经 segments 间接进 sanitize，这里直测私有函数） ----------

    #[test]
    fn hex_color_valid_forms_normalized() {
        // 统一输出小写 #rrggbb，UI 端比较颜色时不用再做大小写/短格式兼容
        assert_eq!(sanitize_hex_color("#abc"), "#aabbcc", "#rgb 短格式按位展开");
        assert_eq!(sanitize_hex_color("#AbC"), "#aabbcc", "短格式大写归小写");
        assert_eq!(sanitize_hex_color("#FF8800"), "#ff8800", "长格式大写归小写");
        assert_eq!(sanitize_hex_color("  #1a2b3c  "), "#1a2b3c", "trim 后合法");
        assert_eq!(sanitize_hex_color(""), "", "空串保持空（= UI 自动渐变）");
    }

    #[test]
    fn hex_color_invalid_forms_cleared() {
        // 非法颜色置空而不是保留原文——空有明确定义（自动渐变），脏值会让 CSS 静默失效
        assert_eq!(sanitize_hex_color("abc123"), "", "缺 # 前缀");
        assert_eq!(sanitize_hex_color("#abcd"), "", "长度 4 非法");
        assert_eq!(sanitize_hex_color("#ab"), "", "长度 2 非法");
        assert_eq!(sanitize_hex_color("#abcdefa"), "", "长度 7 非法");
        assert_eq!(sanitize_hex_color("#ggg"), "", "非 hex 字符");
        assert_eq!(sanitize_hex_color("#12345z"), "", "长格式混入非 hex");
        assert_eq!(sanitize_hex_color("   "), "", "纯空白 trim 后为空");
    }

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
    fn default_segments_labels_localized_with_zh_fallback() {
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
        // 各语言组彼此不同（真的做了本地化，而不是同一套复制）
        assert_ne!(zh, en);
        assert_ne!(zh, ja);
        assert_ne!(en, pt);
        // 抽查产品文案锚点：英文组是英文、中文组是中文
        assert_eq!(en[0], "Late Night");
        assert_eq!(zh[4], "晚上");
        assert!(en.iter().all(|l| l.is_ascii()), "英文标签应为纯 ASCII");
        // tw 尚无独立文案、未知语言亦然——都回退中文组，保证标签永不为空
        let tw: Vec<String> = default_segments_for("tw")
            .iter()
            .map(|s| s.label.clone())
            .collect();
        let ko: Vec<String> = default_segments_for("ko")
            .iter()
            .map(|s| s.label.clone())
            .collect();
        assert_eq!(tw, zh, "tw 回退中文组");
        assert_eq!(ko, zh, "未知语言回退中文组");
        assert!(zh.iter().all(|l| !l.is_empty()));
    }
}
