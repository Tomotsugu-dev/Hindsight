//! Screenshot policy: decides whether this focus switch gets a screenshot saved.
//! Backend of the "Privacy" section in settings.
//!
//! A match only skips the screenshot — the activity row is still stored, still
//! timed, and still uploaded. Whether it counts towards the stats is
//! [`super::ignore`]'s job; the two are orthogonal.

/// Whether to skip this screenshot: the URL matches `url_keywords`, or the app
/// name or window title matches `window_keywords`. Substring, case-insensitive;
/// an empty list or a missing URL simply never matches.
pub fn should_skip_screenshot(
    app_name: &str,
    title: &str,
    url: Option<&str>,
    url_keywords: &[String],
    window_keywords: &[String],
) -> bool {
    url.is_some_and(|u| matches_any(u, url_keywords))
        || matches_any(app_name, window_keywords)
        || matches_any(title, window_keywords)
}

pub(crate) fn matches_any(haystack: &str, keywords: &[String]) -> bool {
    let h = haystack.to_lowercase();
    keywords.iter().any(|k| {
        let k = k.trim();
        if k.is_empty() {
            return false;
        }
        h.contains(&k.to_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn 空列表不过滤() {
        assert!(!should_skip_screenshot(
            "Chrome",
            "百度一下",
            Some("https://www.baidu.com/"),
            &[],
            &[],
        ));
    }

    #[test]
    fn url路径片段命中即跳过() {
        let url_kw = s(&["/login", "/oauth"]);
        assert!(should_skip_screenshot(
            "Chrome",
            "Sign in - Google Accounts",
            Some("https://accounts.google.com/o/oauth2/v2/auth?...."),
            &url_kw,
            &[],
        ));
    }

    #[test]
    fn url大小写忽略() {
        let url_kw = s(&["/Login"]);
        assert!(should_skip_screenshot(
            "Chrome",
            "x",
            Some("https://example.com/LOGIN/index"),
            &url_kw,
            &[],
        ));
    }

    #[test]
    fn 没有url时url列表不参与() {
        let url_kw = s(&["/login"]);
        // 没传 URL，url 列表怎么写都不命中
        assert!(!should_skip_screenshot(
            "Chrome",
            "百度一下",
            None,
            &url_kw,
            &[],
        ));
    }

    #[test]
    fn 应用名命中() {
        let app_kw = s(&["微信"]);
        assert!(should_skip_screenshot(
            "微信",
            "聊天 - 张三",
            None,
            &[],
            &app_kw,
        ));
    }

    #[test]
    fn 标题命中() {
        let app_kw = s(&["招商银行"]);
        assert!(should_skip_screenshot(
            "Chrome",
            "招商银行 - 个人主页",
            None,
            &[],
            &app_kw,
        ));
    }

    #[test]
    fn 任意一路命中即跳过() {
        // url 不命中、app 命中
        let url_kw = s(&["/login"]);
        let app_kw = s(&["微信"]);
        assert!(should_skip_screenshot(
            "微信",
            "聊天",
            Some("https://baidu.com/"),
            &url_kw,
            &app_kw,
        ));
    }

    #[test]
    fn 关键词前后空白被吃掉() {
        let app_kw = s(&["  微信  "]);
        assert!(should_skip_screenshot("微信", "x", None, &[], &app_kw,));
    }

    #[test]
    fn 全空白关键词不命中() {
        // 防止误把全空白 keyword 当成"匹配空串"（contains("") 永远 true）
        let app_kw = s(&["   "]);
        assert!(!should_skip_screenshot(
            "Chrome",
            "百度一下",
            None,
            &[],
            &app_kw,
        ));
    }
}
