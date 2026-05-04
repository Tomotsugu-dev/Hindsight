//! 从浏览器地址栏抠取当前 URL，给隐私过滤判断用。
//!
//! 平台路由：
//! - Windows → [`windows`]：UI Automation 扫前台窗口的 Edit / Document 控件
//! - macOS   → [`macos`]：osascript 内嵌 AppleScript，问浏览器要 URL
//! - 其它    → 永远 None
//!
//! 共享部分（`is_browser_app` 粗判 + `normalize_url` 串规范化）放在路由层，
//! 平台子模块只负责"拿到一个原始字符串"。
//!
//! 阻塞调用：Windows UIA 偶尔卡 ~300ms，macOS osascript 启动 ~50–150ms。
//! 调用方包 `spawn_blocking`，不要在 async runtime 直接调。
//!
//! 拿不到 URL（未授权 / 浏览器没开 / 平台不支持）一律返回 None；URL 关键词
//! 那一路在 `privacy::should_skip_screenshot` 里会自动跳过，不影响 app/标题
//! 关键词匹配。

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod stub {
    pub(super) fn try_get_url(_app_name: &str) -> Option<String> {
        None
    }
}

#[cfg(target_os = "windows")]
use windows as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
use stub as imp;

/// 该应用是不是浏览器（基于进程名 / app_name 粗判）。
/// 非浏览器就不要花平台调用的钱。
pub fn is_browser_app(app_name: &str) -> bool {
    let s = app_name.to_lowercase();
    s.contains("chrome")
        || s.contains("msedge")
        || s.contains("edge")
        || s.contains("firefox")
        || s.contains("brave")
        || s.contains("vivaldi")
        || s.contains("opera")
        || s.contains("arc")
        || s.contains("safari")
        || s.contains("zen-browser")
        || s.contains("librewolf")
        || s.contains("waterfox")
}

/// 同步阻塞：从当前前台窗口抠 URL。调用方包 spawn_blocking。
///
/// `app_name` 在 macOS 上必填（osascript 需要知道目标 app）；Windows 上忽略
/// （走 GetForegroundWindow + UIA，自洽拿前台 HWND）。
pub fn try_get_foreground_browser_url(app_name: &str) -> Option<String> {
    imp::try_get_url(app_name)
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
