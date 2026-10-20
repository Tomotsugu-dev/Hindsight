//! Plugin events: builds each event for the permissions of each plugin, and sends it to the
//! plugin background window. Each kind of event has its own file.

pub mod focus;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::plugins::background_window::LABEL;
use focus::FocusEvent;

/// The event name the plugin background window listens to.
const EVENT: &str = "plugin-event";

/// One event sent to the plugin background window, which passes it on to the plugin
/// `to_plugin_id`.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    to_plugin_id: String,
    #[serde(flatten)]
    event: PluginEvent,
}

/// The events a plugin can get.
#[derive(Clone, Serialize)]
#[serde(tag = "eventName", content = "data")]
enum PluginEvent {
    /// In JSON: `{ "eventName": "focus.changed", "data": { … } }`.
    #[serde(rename = "focus.changed")]
    FocusChanged(FocusEvent),
}

/// Sends one event to one plugin. If it cannot be sent, it is only logged, and the other plugins
/// still get theirs.
fn send(app: &AppHandle, to_plugin_id: String, event: PluginEvent) {
    let envelope = Envelope {
        to_plugin_id,
        event,
    };
    if let Err(e) = app.emit_to(LABEL, EVENT, envelope.clone()) {
        log::warn!(
            "plugins: cannot send an event to {}: {e}",
            envelope.to_plugin_id
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 发给转发层的 JSON：收件插件、事件名、内容三个键，转发层和 Worker 按这几个名字取。
    #[test]
    fn envelope_json() {
        let envelope = Envelope {
            to_plugin_id: "hindsight-focus".into(),
            event: PluginEvent::FocusChanged(FocusEvent::default()),
        };
        let json = serde_json::to_value(envelope).unwrap();
        assert_eq!(json["toPluginId"], "hindsight-focus");
        assert_eq!(json["eventName"], "focus.changed");
        assert!(json["data"].is_object());
        assert_eq!(json.as_object().unwrap().len(), 3);
    }
}
