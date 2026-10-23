//! Notification methods (`notifications.*`).

use serde::Deserialize;
use serde_json::Value;

use super::check_granted;
use crate::error::{Error, Result};
use crate::repo::plugins::Plugin;

/// `notifications.send`: shows a notification. The title is the plugin name, and the body is
/// `body` from the parameters. With `sound: true` it also plays the system alert sound.
///
/// # Errors
///
/// - `body` is not a non-empty string, or `sound` is not a boolean: [`Error::InvalidInput`]
/// - `sound` is true and the plugin does not have `notifications.sound`:
///   [`Error::PluginNoPermission`]
pub(super) fn send(
    plugin: &Plugin,
    params: Value,
    notify: impl FnOnce(&str, &str, bool) -> Result<()>,
) -> Result<Value> {
    let Notification { body, sound } = serde_json::from_value(params).unwrap_or_default();
    if body.trim().is_empty() {
        return Err(Error::InvalidInput(
            "notifications.send needs { body, sound? }: body a non-empty string, sound a boolean",
        ));
    }
    // Fails instead of sending a silent notification, so the plugin always gets what it asked for
    if sound {
        check_granted(plugin, "notifications.sound")?;
    }
    // The title is always the plugin name, not its id.
    notify(&plugin.name, &body, sound)?;
    Ok(Value::Null)
}

/// The parameters of `notifications.send`.
#[derive(Default, Deserialize)]
struct Notification {
    body: String,
    #[serde(default)]
    sound: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::test_util::plugin_with_permissions;

    /// 标题是插件名，正文是插件写的，不传 sound 就没有提示音；正文为空、sound 不是布尔值就报错，
    /// 不弹通知。
    #[test]
    fn send_uses_the_plugin_name_as_title() {
        let plugin = plugin_with_permissions("hindsight-focus", &["notifications"]);

        let mut shown = None;
        send(
            &plugin,
            serde_json::json!({ "body": "已经看了 10 分钟 YouTube" }),
            |title, body, sound| {
                shown = Some((title.to_string(), body.to_string(), sound));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            shown,
            Some((
                "hindsight-focus".to_string(),
                "已经看了 10 分钟 YouTube".to_string(),
                false
            ))
        );

        for params in [
            Value::Null,
            serde_json::json!({ "body": " " }),
            serde_json::json!({ "body": "这一段结束了", "sound": "yes" }),
        ] {
            let err = send(&plugin, params, |_, _, _| panic!("不该弹通知")).unwrap_err();
            assert!(matches!(err, Error::InvalidInput(_)), "{err}");
        }
    }

    /// sound 为 true：有 notifications.sound 就带提示音；没有就报缺这个权限，不弹通知。
    #[test]
    fn sound_needs_its_permission() {
        let params = serde_json::json!({ "body": "这一段结束了", "sound": true });

        let plugin =
            plugin_with_permissions("hindsight-focus", &["notifications", "notifications.sound"]);
        let mut played = None;
        send(&plugin, params.clone(), |_, _, sound| {
            played = Some(sound);
            Ok(())
        })
        .unwrap();
        assert_eq!(played, Some(true));

        let plugin = plugin_with_permissions("hindsight-focus", &["notifications"]);
        let err = send(&plugin, params, |_, _, _| panic!("不该弹通知")).unwrap_err();
        assert!(
            matches!(&err, Error::PluginNoPermission(p) if p == "notifications.sound"),
            "{err}"
        );
    }
}
