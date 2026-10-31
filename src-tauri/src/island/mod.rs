//! The island: a small area that shows what Hindsight draws. It does not know about plugins:
//! which plugin has the island, and who gets a click, are in `plugins/`. For the design, see
//! island.md in docs/design/plugins/.
//!
//! 每个系统一个文件夹，编译时选定，对外的函数一样：
//! - macOS → `macos/`：在菜单栏正中
//! - Windows → `windows/`：一个可以拖动、可以吸附在任务栏上的胶囊
//! - 别的系统 → `other.rs`：不显示

pub mod content;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod other;
#[cfg(target_os = "windows")]
mod windows;

use content::Content;
use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent, Wry,
};

// 调用的地方不用分系统
#[cfg(target_os = "macos")]
pub use macos::{content, drag, hide, hover, measured, ready, show};
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub use other::{content, drag, hide, hover, measured, ready, show};
#[cfg(target_os = "windows")]
pub use windows::{content, drag, hide, hover, measured, ready, show};

// 下面的常量、类型和 `new_window`、`emit`、`State` 给各系统的岛窗口共用。Linux 还没有岛窗口

/// The label of the island window. `capabilities/island.json` uses the same label
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
pub const LABEL: &str = "island";
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
const PAGE: &str = "src/island/index.html";
/// Sent to the page with the new content, or `null` when nothing is on the island
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
const CONTENT_EVENT: &str = "island-content";
/// Sent to the page with a new [`Layout`] when the main screen changes
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
const LAYOUT_EVENT: &str = "island-layout";
/// Sent to the page with `true` when the island opens, and `false` when it closes
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
const OPEN_EVENT: &str = "island-open";

/// 展开的岛最宽多少。内容按上限放满时要这么宽：刘海屏上右上角 20 宽的字放在刘海右边
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
const MAX_OPEN_WIDTH: f64 = 520.0;
/// 展开时下面的面板最高多少：内容按上限放满也放得下
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
const MAX_PANEL_HEIGHT: f64 = 200.0;
/// The corner radius of the open island
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
const OPEN_RADIUS: f64 = 16.0;

/// 页面量出的展开内容有多大，在页面里的逻辑像素。
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
// Linux 还没有岛窗口，用不到
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
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
    /// 展开的岛在哪：面板在它最上面那一条下面，往上展开时在那一条上面
    pub open: PageRect,
    /// 页面自己画形状时要的位置（Windows）。`None`：形状由系统的遮罩画（macOS）
    pub drawn: Option<Drawn>,
}

/// 页面自己画形状时（Windows）要的位置。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
// 只有 Windows 的窗口会建它
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub struct Drawn {
    /// 收起的胶囊
    pub capsule: PageRect,
    /// 展开时左边的内容放在哪：贴着展开的岛的左边。靠屏幕右边时岛往左挪，它也跟着挪
    pub open_left: Option<PageRect>,
    /// 往上展开：面板在胶囊那一条上面
    pub up: bool,
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

/// 建岛的窗口，各系统都要的设置在这里：无边框、不能缩放、没有阴影、不进任务栏，建好时不拿焦点、
/// 不显示。各系统自己要的设置由 `add` 加上。
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
fn new_window<'a>(
    app: &'a AppHandle,
    add: impl FnOnce(
        WebviewWindowBuilder<'a, Wry, AppHandle>,
    ) -> WebviewWindowBuilder<'a, Wry, AppHandle>,
) -> Result<WebviewWindow, String> {
    let builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App(PAGE.into()))
        .decorations(false)
        .resizable(false)
        .shadow(false)
        .skip_taskbar(true)
        .focused(false)
        .visible(false);
    let window = add(builder).build().map_err(|e| e.to_string())?;
    // It stays until the app quits: Cmd+W must not close it
    window.on_window_event(|event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
        }
    });
    Ok(window)
}

/// 把 `payload` 作为 `event` 发给岛的页面。发不出去只记日志。
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
fn emit<S: Serialize + Clone>(window: &WebviewWindow, event: &str, payload: S) {
    if let Err(e) = window.emit_to(LABEL, event, payload) {
        log::warn!("island: cannot send {event} to the page: {e}");
    }
}

/// 岛上的内容、页面量出的大小、展开没有。各系统一样的规则在这里，窗口照着它画：
/// - 内容没有 `open` 时不能展开；
/// - 换成不能展开的内容时，展开着的岛收起来。
#[derive(Default)]
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
struct State {
    content: Option<Content>,
    /// The widths of the closed content, as last measured by the page
    widths: (Option<f64>, Option<f64>),
    /// 页面上次量出的展开内容的大小；`None`：内容不展开
    panel: Option<PanelSize>,
    open: bool,
}

#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
impl State {
    /// 换上 `content`。返回 `true`：岛本来展开着，新内容不能展开，收起来了。
    fn set_content(&mut self, content: Option<Content>) -> bool {
        self.content = content;
        let close = self.open && !self.can_open();
        if close {
            self.open = false;
        }
        close
    }

    /// 展开或者收起。内容不能展开时只能收起。返回 `true`：变了。
    fn set_open(&mut self, open: bool) -> bool {
        let open = open && self.can_open();
        let changed = open != self.open;
        self.open = open;
        changed
    }

    /// 记下页面量出的两边宽度和展开内容的大小。
    fn measured(&mut self, left: Option<f64>, right: Option<f64>, panel: Option<PanelSize>) {
        self.widths = (left, right);
        self.panel = panel;
    }

    fn can_open(&self) -> bool {
        self.content
            .as_ref()
            .is_some_and(|content| content.open.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 收起时右边一段字；`open` 为 `true` 时带展开的内容
    fn content(open: bool) -> Content {
        let mut value = json!({ "closed": { "right": { "type": "text", "text": "25:00" } } });
        if open {
            value["open"] = json!({ "left": [{ "type": "text", "text": "专注中" }] });
        }
        Content::parse(value).unwrap()
    }

    #[test]
    fn opens_only_with_open_content() {
        let mut state = State::default();
        // 没有内容、内容没有 open：都打不开
        assert!(!state.set_open(true));
        state.set_content(Some(content(false)));
        assert!(!state.set_open(true));
        assert!(!state.open);

        state.set_content(Some(content(true)));
        assert!(state.set_open(true));
        assert!(state.open);
        // 已经展开，再展开不算变
        assert!(!state.set_open(true));
        assert!(state.set_open(false));
        assert!(!state.open);
    }

    #[test]
    fn new_content_that_cannot_open_closes_the_island() {
        let mut state = State::default();
        state.set_content(Some(content(true)));
        state.set_open(true);
        // 换成也能展开的内容：照样开着
        assert!(!state.set_content(Some(content(true))));
        assert!(state.open);
        // 换成不能展开的、撤下内容：收起来
        assert!(state.set_content(Some(content(false))));
        assert!(!state.open);
        state.set_content(Some(content(true)));
        state.set_open(true);
        assert!(state.set_content(None));
        assert!(!state.open);
        // 本来就收着：不算收起
        assert!(!state.set_content(Some(content(false))));
    }
}
