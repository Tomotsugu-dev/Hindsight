//! Notification methods (`notifications.*`).

use serde::Deserialize;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::repo::plugins::Plugin;

/// `notifications.send`: shows a notification. The title is the plugin name, and the body is
/// `body` from the parameters.
///
/// # Errors
///
/// - `body` is not a non-empty string: [`Error::InvalidInput`]
pub(super) fn send(
    plugin: &Plugin,
    params: Value,
    notify: impl FnOnce(&str, &str) -> Result<()>,
) -> Result<Value> {
    let body = serde_json::from_value::<Notification>(params)
        .map(|n| n.body)
        .unwrap_or_default();
    if body.trim().is_empty() {
        return Err(Error::InvalidInput(
            "notifications.send needs { body }, a non-empty string",
        ));
    }
    // The title is always the plugin name, not its id.
    notify(&plugin.name, &body)?;
    Ok(Value::Null)
}

/// The parameters of `notifications.send`.
#[derive(Deserialize)]
struct Notification {
    body: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::test_util::plugin_with_permissions;

    /// 标题是插件名，正文是插件写的；正文为空就报错，不弹通知。
    #[test]
    fn send_uses_the_plugin_name_as_title() {
        let plugin = plugin_with_permissions("hindsight-focus", &["notifications"]);

        let mut shown = None;
        send(
            &plugin,
            serde_json::json!({ "body": "已经看了 10 分钟 YouTube" }),
            |title, body| {
                shown = Some((title.to_string(), body.to_string()));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            shown,
            Some((
                "hindsight-focus".to_string(),
                "已经看了 10 分钟 YouTube".to_string()
            ))
        );

        for params in [Value::Null, serde_json::json!({ "body": " " })] {
            let err = send(&plugin, params, |_, _| panic!("不该弹通知")).unwrap_err();
            assert!(matches!(err, Error::InvalidInput(_)), "{err}");
        }
    }
}
