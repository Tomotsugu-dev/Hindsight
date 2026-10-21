//! 岛的方法（`island.*`）。内容的格式见 docs/design/plugins/ 的 island.md。

use serde_json::Value;

use crate::error::{Error, Result};
use crate::island::content::Content;
use crate::repo::plugins::Plugin;

/// `island.show`：把 `params` 放到岛上，换掉这个插件之前放的内容。
///
/// # Errors
///
/// - 格式不对，或者不合规则：[`Error::InvalidInputDyn`]
/// - 放了按钮，但插件不在后台运行：[`Error::InvalidInput`]
pub(super) fn show(
    plugin: &Plugin,
    params: Value,
    island: impl FnOnce(Option<Content>),
) -> Result<Value> {
    let content = Content::parse(params)?;
    // 按钮被点时，事件发给插件的后台；不在后台运行就收不到
    if content.has_button() && !plugin.runs_in_background {
        return Err(Error::InvalidInput(
            "island.show: a button needs runs_in_background, to get island.clicked",
        ));
    }
    island(Some(content));
    Ok(Value::Null)
}

/// `island.hide`：撤下这个插件放在岛上的内容。没放过也不报错。
pub(super) fn hide(island: impl FnOnce(Option<Content>)) -> Result<Value> {
    island(None);
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::test_util::{plugin_row, plugin_with_permissions};
    use serde_json::json;

    fn with_button() -> Value {
        json!({
            "closed": { "right": { "type": "text", "text": "25:00" } },
            "open": { "right": [{ "type": "button", "id": "pause", "icon": "Pause" }] }
        })
    }

    #[test]
    fn show_puts_the_checked_content_on_the_island() {
        let plugin = plugin_with_permissions("hindsight-focus", &["island"]);
        let mut shown = None;
        show(&plugin, with_button(), |content| shown = content).unwrap();
        assert!(shown.is_some_and(|content| content.has_button()));
        // 不合规则：报错，岛不动
        let err = show(&plugin, json!({ "closed": {} }), |_| panic!("不该放到岛上")).unwrap_err();
        assert!(matches!(err, Error::InvalidInputDyn(_)), "{err}");
    }

    #[test]
    fn a_button_needs_the_background() {
        // 只有页面的插件收不到点击
        let plugin = plugin_row("page-only", true, false);
        let err = show(&plugin, with_button(), |_| panic!("不该放到岛上")).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "{err}");
    }

    #[test]
    fn hide_takes_the_content_off() {
        let mut hidden = false;
        hide(|content| hidden = content.is_none()).unwrap();
        assert!(hidden);
    }
}
