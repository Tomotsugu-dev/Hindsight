//! 隐私过滤：决定本次焦点切换是否要保存截图。
//!
//! 命中即跳过截图，活动行 / 应用名 / 时长照常记录。
//!
//! 两类关键词：
//! - **URL 关键词**：浏览器地址栏 URL 子串忽略大小写匹配。url 为 None 时这一路不参与判断
//! - **应用关键词**：app_name 或窗口标题 子串忽略大小写匹配
//!
//! 任意一路命中即跳过。两组列表都为空 = 不过滤。

/// 任意关键词命中（子串忽略大小写）即返回 true。
pub fn should_skip_screenshot(
    app_name: &str,
    title: &str,
    url: Option<&str>,
    url_keywords: &[String],
    app_keywords: &[String],
) -> bool {
    if !url_keywords.is_empty() {
        if let Some(u) = url {
            if matches_any(u, url_keywords) {
                return true;
            }
        }
    }
    if !app_keywords.is_empty() {
        if matches_any(app_name, app_keywords) || matches_any(title, app_keywords) {
            return true;
        }
    }
    false
}

fn matches_any(haystack: &str, keywords: &[String]) -> bool {
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
        assert!(should_skip_screenshot(
            "微信",
            "x",
            None,
            &[],
            &app_kw,
        ));
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
