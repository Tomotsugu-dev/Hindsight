//! 按应用名判断是不是浏览器。

/// 词边界匹配：needle 必须按非字母数字切词后作为独立词出现。
/// 给 edge/opera/arc 这类短词用——裸 contains 会把 "Ledger Live"(edge) /
/// "Operator"(opera) / "Archive Utility"(arc) 误判成浏览器。
pub(crate) fn name_has_word(app_name_lower: &str, needle: &str) -> bool {
    app_name_lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|tok| tok == needle)
}

/// 该应用是不是浏览器（基于进程名 / app_name 粗判）。
/// 非浏览器就不要花平台调用的钱。
///
/// 误判成浏览器的代价不小：拿不到 URL 时隐私过滤 fail-closed → 该应用**永远
/// 没有截图**；macOS 每 tick 对它发 AppleScript 还会触发"XX 想控制 YY"的
/// 自动化授权弹窗；Windows 每 tick 白跑最多 ~4s 的 UIA 扫描。
/// 所以短而泛的词（edge / opera / arc）必须按独立词匹配，不能裸 contains。
pub fn is_browser_app(app_name: &str) -> bool {
    let s = app_name.to_lowercase();
    // 长且无歧义的名字子串匹配即可（覆盖 chrome.exe / Chromium / Brave Browser 等）
    if s.contains("chrome")
        || s.contains("msedge")
        || s.contains("firefox")
        || s.contains("brave")
        || s.contains("vivaldi")
        || s.contains("safari")
        || s.contains("zen-browser")
        || s.contains("librewolf")
        || s.contains("waterfox")
    {
        return true;
    }
    ["edge", "opera", "arc"]
        .iter()
        .any(|w| name_has_word(&s, w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 浏览器名命中() {
        assert!(is_browser_app("Google Chrome"));
        assert!(is_browser_app("Safari"));
        assert!(is_browser_app("firefox.exe"));
        assert!(is_browser_app("Microsoft Edge"));
        assert!(is_browser_app("Brave Browser"));
    }

    #[test]
    fn 非浏览器不命中() {
        assert!(!is_browser_app("Code"));
        assert!(!is_browser_app("Slack"));
        assert!(!is_browser_app("微信"));
        // 短词误伤回归：这些名字含 arc/edge/opera 子串但不是浏览器
        assert!(!is_browser_app("Archive Utility"));
        assert!(!is_browser_app("Ledger Live"));
        assert!(!is_browser_app("Operator"));
        assert!(!is_browser_app("SearchHost.exe"));
    }

    #[test]
    fn 短词浏览器仍命中() {
        assert!(is_browser_app("Arc"));
        assert!(is_browser_app("Microsoft Edge"));
        assert!(is_browser_app("msedge.exe"));
        assert!(is_browser_app("Opera GX"));
    }
}
