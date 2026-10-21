//! 没有岛的系统：内容收下就算了，什么都不显示，岛的页面也不会打开。函数和 macos.rs 的一样。

use tauri::AppHandle;

use super::content::Content;
use super::Layout;

pub fn show(_app: &AppHandle, _content: Content) {}

pub fn hide(_app: &AppHandle) {}

pub fn content() -> Option<Content> {
    None
}

pub fn measured(_left: Option<f64>, _right: Option<f64>, _panel: Option<f64>) -> Option<Layout> {
    None
}

pub fn ready() {}
