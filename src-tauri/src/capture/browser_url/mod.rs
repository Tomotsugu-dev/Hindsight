//! 浏览器的网址：判断一个应用是不是浏览器，读出前台浏览器窗口地址栏里的网址。
//!
//! 每个系统一份实现，编译时选定，接口都一样：
//! - Windows → [`windows`]
//! - macOS → [`macos`]
//! - Linux → [`linux`]
//!
//! 浏览器名单在 [`browsers`]，网址字符串的处理在 [`url`](self::url)。
//! 各系统的实现只负责读出地址栏里的文字。

use crate::capture::focus::WindowInfo;

mod browsers;
mod url;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

pub use self::url::host_for_stats;
pub use browsers::is_browser_app;

/// 读出焦点窗口 `window` 地址栏里的网址，读不到时返回 None。
///
/// 各系统的读取会阻塞几十到几百毫秒，所以放到后台线程跑。
pub async fn read_url(window: &WindowInfo) -> Option<String> {
    let window = window.clone();
    tokio::task::spawn_blocking(move || imp::read_url(&window))
        .await
        .ok()
        .flatten()
}
