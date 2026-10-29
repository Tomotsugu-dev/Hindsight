//! Browser addresses: whether an app is a browser, and the address shown in the focused browser
//! window.
//!
//! One implementation per system, chosen at compile time, all with the same interface:
//! - Windows → [`windows`]
//! - macOS → [`macos`]
//! - Linux → [`linux`]
//!
//! The browser name list is in [`browsers`], and the URL string helpers are in [`url`](self::url).
//! Each system's implementation only reads the text in the address bar.

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

/// Returns the address in the address bar of the focused window `window`, or None when it cannot
/// be read.
///
/// Reading blocks for tens to hundreds of milliseconds, so it runs on a background thread.
pub async fn read_url(window: &WindowInfo) -> Option<String> {
    let window = window.clone();
    tokio::task::spawn_blocking(move || imp::read_url(&window))
        .await
        .ok()
        .flatten()
}
