//! Ignore rules decide whether an activity counts towards the stats. A match
//! sets `excluded = 1`, and reports, exports, AI summaries and chat queries all
//! skip the row — but it is still stored, still screenshotted, and still
//! uploaded.
//!
//! Easy to confuse with [`super::screenshot_policy`]; the two are orthogonal.
//! That one decides whether to keep a screenshot, and time is counted either way.
//!
//! Reversible: delete the rule and run
//! [`crate::repo::activities::reapply_ignore_rules`] to recompute the flag over
//! the whole table, and the history counts again.

use serde::{Deserialize, Serialize};

/// One ignore rule: a process name (required) plus an optional window-title
/// keyword. Matched activities are still recorded and still get a screenshot;
/// only stats, exports and AI summaries skip them.
///
/// A bare keyword is not allowed: "Download" on its own would swallow matching
/// windows across every app, with no error and no hint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IgnoreRule {
    /// Process name. Exact match, case-insensitive, trimmed.
    pub process_name: String,
    /// Window-title keyword: the title must contain it, case-insensitive.
    /// `None` excludes the whole process.
    ///
    /// A blank keyword never matches. Every string contains the empty string,
    /// so matching literally would exclude the whole app the moment the UI
    /// sends `Some("")` for an input box the user left empty.
    #[serde(default)]
    pub title_keyword: Option<String>,
}

/// Whether the current window matches any rule, i.e. whether this activity is
/// left out of the stats. An empty rule list excludes nothing.
pub fn is_excluded(app_name: &str, title: &str, rules: &[IgnoreRule]) -> bool {
    if rules.is_empty() {
        return false;
    }
    let app = app_name.trim().to_lowercase();
    if app.is_empty() {
        return false;
    }
    let title_lower = title.to_lowercase();

    rules.iter().any(|rule| {
        let want = rule.process_name.trim().to_lowercase();
        // A rule with no process name is invalid: it would match every window.
        if want.is_empty() || want != app {
            return false;
        }
        match rule.title_keyword.as_deref() {
            // No title condition: the whole process.
            None => true,
            Some(kw) => {
                let kw = kw.trim().to_lowercase();
                // A blank keyword never matches (see the field doc).
                !kw.is_empty() && title_lower.contains(&kw)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(process: &str, title: Option<&str>) -> IgnoreRule {
        IgnoreRule {
            process_name: process.to_string(),
            title_keyword: title.map(String::from),
        }
    }

    #[test]
    fn 空规则列表不排除() {
        assert!(!is_excluded("Windows Terminal Host", "任意标题", &[]));
    }

    #[test]
    fn 进程加标题都命中才排除() {
        let rules = vec![rule("Windows Terminal Host", Some("Download videos"))];
        assert!(is_excluded(
            "Windows Terminal Host",
            "✳ Download videos from July 17 onwards with uv",
            &rules,
        ));
    }

    #[test]
    fn 进程对但标题不对不排除() {
        let rules = vec![rule("Windows Terminal Host", Some("Download videos"))];
        // 同一个终端里干别的活,必须照常计入
        assert!(!is_excluded(
            "Windows Terminal Host",
            "vim src/main.rs",
            &rules,
        ));
    }

    #[test]
    fn 标题对但进程不对不排除() {
        let rules = vec![rule("Windows Terminal Host", Some("Download videos"))];
        assert!(!is_excluded("Chrome", "Download videos - YouTube", &rules));
    }

    #[test]
    fn 无标题条件时整个进程被排除() {
        let rules = vec![rule("SomeDownloader", None)];
        assert!(is_excluded("SomeDownloader", "任意标题", &rules));
        assert!(is_excluded("SomeDownloader", "", &rules));
    }

    #[test]
    fn 进程名忽略大小写和首尾空白() {
        let rules = vec![rule("  windows terminal host  ", Some("download"))];
        assert!(is_excluded(
            "Windows Terminal Host",
            "DOWNLOAD videos",
            &rules,
        ));
    }

    #[test]
    fn 进程名是精确匹配不是子串() {
        // 防止 "Terminal" 这条规则顺带吃掉 "Windows Terminal Host"
        let rules = vec![rule("Terminal", None)];
        assert!(!is_excluded("Windows Terminal Host", "x", &rules));
    }

    #[test]
    fn 全空白标题关键词不命中() {
        // contains("") 恒 true —— 必须挡住,否则一次手滑排除整个应用
        let rules = vec![rule("Windows Terminal Host", Some("   "))];
        assert!(!is_excluded("Windows Terminal Host", "任意标题", &rules));
    }

    #[test]
    fn 空进程名的规则无效() {
        let rules = vec![rule("   ", Some("download"))];
        assert!(!is_excluded("Chrome", "download videos", &rules));
    }

    #[test]
    fn 空应用名不命中() {
        let rules = vec![rule("Chrome", None)];
        assert!(!is_excluded("", "x", &rules));
    }

    #[test]
    fn 多条规则任一命中即排除() {
        let rules = vec![
            rule("Chrome", Some("YouTube")),
            rule("Windows Terminal Host", Some("Download videos")),
        ];
        assert!(is_excluded(
            "Windows Terminal Host",
            "⠂ Download videos from July 17",
            &rules,
        ));
    }

    #[test]
    fn spinner字符变化不影响匹配() {
        // Claude Code 的转圈动画每帧换字符,同一段任务会产生 ⠐/✳/⠂ 三种标题。
        // 关键词只取文字部分,三种前缀都要命中。
        let rules = vec![rule("Windows Terminal Host", Some("Download videos from"))];
        for prefix in ["⠐ ", "✳ ", "⠂ ", ""] {
            let title = format!("{prefix}Download videos from July 17 onwards with uv");
            assert!(is_excluded("Windows Terminal Host", &title, &rules));
        }
    }
}
