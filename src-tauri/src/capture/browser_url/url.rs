//! 网址字符串的处理：把各系统读到的原始文字转成网址，把网址转成统计用的域名。

/// 统计用的网站域名：只认 http/https,取 host 小写、去掉 `www.` 前缀、不带端口。
/// 浏览器内部页（chrome:// / about: / file://）、IP 以外的解析失败、空 host 一律 None
/// ——这些不是"网站"。子域名保留（docs.github.com 与 github.com 分开），不引入
/// 公共后缀库。完整 URL 永不落盘，落盘的只有这个返回值。
pub fn host_for_stats(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    let host = host.trim_end_matches('.');
    let host = host.strip_prefix("www.").unwrap_or(host);
    // "www." 这种只剩前缀的壳不是网站
    if host.is_empty() || host == "www" {
        None
    } else {
        Some(host.to_string())
    }
}

/// 把平台拿到的字符串规范化成 URL。能解析就返回，否则 None。
/// Windows / macOS 子模块共用。
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(crate) fn normalize_url(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let lower = s.to_lowercase();
    // 已经带 scheme 的直接收（含 chrome:// / edge:// / about: 等内部页 ——
    // 关键词列表里 /password 等子串能照常匹配上）
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("chrome://")
        || lower.starts_with("edge://")
        || lower.starts_with("about:")
        || lower.starts_with("file://")
    {
        return Some(s.to_string());
    }
    // Chrome 默认隐藏 https:// 前缀，地址栏可能就显示 "example.com/path"
    // 启发式：含 . 且不含空格、不含中文，补 https://
    if !s.contains(' ') && s.contains('.') {
        let has_cjk = s.chars().any(|c| c >= '\u{4E00}');
        if !has_cjk {
            return Some(format!("https://{s}"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// host_for_stats:小写、去 www.、去端口、保留子域;非 http(s) 与内部页为 None。
    #[test]
    fn host_for_stats_normalizes_and_filters() {
        assert_eq!(
            host_for_stats("https://www.GitHub.com/Tomotsugu-dev/Hindsight?x=1"),
            Some("github.com".into())
        );
        assert_eq!(
            host_for_stats("https://docs.github.com/en"),
            Some("docs.github.com".into()),
            "子域名保留,不并入主域"
        );
        assert_eq!(
            host_for_stats("http://localhost:5173/chat"),
            Some("localhost".into()),
            "端口不进域名"
        );
        assert_eq!(
            host_for_stats("https://192.168.1.10:8080/"),
            Some("192.168.1.10".into())
        );
        assert_eq!(host_for_stats("https://www./"), None, "去 www. 后为空");
        for internal in [
            "chrome://settings",
            "about:blank",
            "file:///tmp/a.html",
            "edge://flags",
        ] {
            assert_eq!(host_for_stats(internal), None, "{internal} 不是网站");
        }
        assert_eq!(host_for_stats("not a url"), None);
        assert_eq!(host_for_stats(""), None);
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    #[test]
    fn normalize_url_带_scheme() {
        assert_eq!(
            normalize_url("https://example.com/").as_deref(),
            Some("https://example.com/")
        );
        assert_eq!(
            normalize_url("chrome://settings").as_deref(),
            Some("chrome://settings")
        );
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    #[test]
    fn normalize_url_无_scheme_补_https() {
        assert_eq!(
            normalize_url("example.com/path").as_deref(),
            Some("https://example.com/path")
        );
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    #[test]
    fn normalize_url_中文_或_空格_拒收() {
        assert!(normalize_url("百度一下").is_none());
        assert!(normalize_url("hello world").is_none());
        assert!(normalize_url("").is_none());
    }
}
