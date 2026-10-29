//! 从浏览器地址栏抠取当前 URL，给隐私过滤判断用。
//!
//! 平台路由：
//! - Windows → [`windows`]：UI Automation 扫前台窗口的 Edit / Document 控件
//! - macOS   → [`macos`]：osascript 内嵌 AppleScript，问浏览器要 URL
//! - 其它    → 永远 None
//!
//! 浏览器名单在 [`browsers`]，网址字符串的处理在 [`url`](self::url)，
//! 平台子模块只负责"拿到一个原始字符串"。
//!
//! 阻塞调用：Windows UIA 偶尔卡 ~300ms，macOS osascript 启动 ~50–150ms。
//! 调用方包 `spawn_blocking`，不要在 async runtime 直接调。
//!
//! 拿不到 URL（未授权 / 浏览器没开 / 平台不支持）一律返回 None；URL 关键词
//! 那一路在 `screenshot::policy::should_skip_screenshot` 里会自动跳过，不影响 app/标题
//! 关键词匹配。

mod browsers;
mod url;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod stub {
    pub(super) fn try_get_url(_app_name: &str) -> Option<String> {
        None
    }
}

#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
use stub as imp;
#[cfg(target_os = "windows")]
use windows as imp;

pub use self::url::host_for_stats;
pub use browsers::is_browser_app;

/// 同步阻塞：从当前前台窗口抠 URL。调用方包 spawn_blocking。
///
/// `app_name` 在 macOS 上必填（osascript 需要知道目标 app）；Windows 上忽略
/// （走 GetForegroundWindow + UIA，自洽拿前台 HWND）。
pub fn try_get_foreground_browser_url(app_name: &str) -> Option<String> {
    imp::try_get_url(app_name)
}
