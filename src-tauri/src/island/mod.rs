//! The island: a small area in the middle of the menu bar that shows what Hindsight draws. It
//! does not know about plugins: which plugin has the island, and who gets a click, are in
//! `plugins/`. For the design, see island.md in docs/design/plugins/.

pub mod content;
// Only the macOS window uses the shapes
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub mod geometry;
#[cfg(target_os = "macos")]
pub mod macos;

use tauri::AppHandle;

use content::Content;

/// 把岛上的内容换成 `content`。只有 macOS 上有岛，别的系统上什么都不做。
pub fn show(app: &AppHandle, content: Content) {
    #[cfg(target_os = "macos")]
    macos::show(app, content);
    #[cfg(not(target_os = "macos"))]
    let _ = (app, content);
}

/// 撤掉岛上的所有内容。只有 macOS 上有岛，别的系统上什么都不做。
pub fn hide(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    macos::hide(app);
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}
