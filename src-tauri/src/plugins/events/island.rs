//! `island.clicked`：插件放在岛上的按钮被点了。不另要权限：只有放了按钮的插件收得到，内容只有
//! 它自己起的按钮 `id`。

use serde::Serialize;
use tauri::AppHandle;

use super::{send, PluginEvent};

/// 插件在 `island.clicked` 事件里收到的内容。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(super) struct IslandClick {
    /// 被点的按钮的 `id`
    pub(super) button: String,
}

/// 告诉插件 `plugin_id`：它放在岛上的按钮 `button` 被点了。
pub fn clicked(app: &AppHandle, plugin_id: &str, button: String) {
    send(
        app,
        plugin_id.to_string(),
        PluginEvent::IslandClicked(IslandClick { button }),
    );
}
