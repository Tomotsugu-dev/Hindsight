//! `window.changed`：插件自己的窗口打开、最小化、还原、关掉了。不另要权限：只发给这个插件，说的是
//! 它自己的窗口。

use serde::Serialize;
use tauri::AppHandle;

use super::{send, PluginEvent};

/// 插件窗口现在的样子。插件在 `window.changed` 事件里收到 `{ "state": "minimized" }` 这样的内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum WindowState {
    Open,
    Minimized,
    Closed,
}

/// 告诉插件 `plugin_id`：它的窗口现在是 `state`。
pub fn changed(app: &AppHandle, plugin_id: &str, state: WindowState) {
    send(
        app,
        plugin_id.to_string(),
        PluginEvent::WindowChanged(state),
    );
}
