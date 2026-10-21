//! The island: a small area in the middle of the menu bar that shows what Hindsight draws. It
//! does not know about plugins: which plugin has the island, and who gets a click, are in
//! `plugins/`. For the design, see island.md in docs/design/plugins/.

pub mod content;
// Only the macOS window uses the shapes
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub mod geometry;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod other;

use serde::{Deserialize, Serialize};

// 两个文件对外的函数一样，调用的地方不用分系统
#[cfg(target_os = "macos")]
pub use macos::{content, hide, measured, ready, show, LABEL};
#[cfg(not(target_os = "macos"))]
pub use other::{content, hide, measured, ready, show};

/// 页面量出的展开内容有多大，在页面里的逻辑像素。
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
// 只有 macOS 的窗口用它
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct PanelSize {
    /// 面板内容不折行、不截断时的宽度，最多到窗口那么宽
    pub width: f64,
    pub height: f64,
    /// 右上角的字要多宽；`None`：没有右上角的字
    pub top_right: Option<f64>,
}

/// Where the content of each closed side goes in the page.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub notch: bool,
    pub left: Option<PageRect>,
    pub right: Option<PageRect>,
    /// 展开时右上角的字放在哪
    pub top_right: PageRect,
    /// 展开的岛在哪：面板在它最上面那一条下面
    pub open: PageRect,
}

/// A rectangle in the page: in logical pixels, from the top-left corner of the window, with y
/// growing downward.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct PageRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
