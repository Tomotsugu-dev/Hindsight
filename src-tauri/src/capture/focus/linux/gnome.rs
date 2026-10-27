//! GNOME Wayland: asks the Hindsight Focus extension over D-Bus
//! (`src-tauri/linux/gnome-extension/`). The interface is described at the top of `extension.js`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

use super::super::{resolve_exe_path, FocusSource, WindowInfo};
use crate::error::{Error, Result};

const BUS_NAME: &str = "com.tomotsugu.hindsight.Focus";
const OBJECT_PATH: &str = "/com/tomotsugu/hindsight/Focus";
const INTERFACE: &str = "com.tomotsugu.hindsight.Focus";
/// Upper bound for one call: the tick calls it directly, so a stuck gnome-shell must not stall the
/// tick
const CALL_TIMEOUT: Duration = Duration::from_millis(500);

pub(super) struct GnomeFocus {
    /// Session bus connection, opened on first use; if opening fails, the next call tries again
    conn: Mutex<Option<Connection>>,
}

impl GnomeFocus {
    pub(super) fn new() -> Self {
        Self {
            conn: Mutex::new(None),
        }
    }

    fn connection(&self) -> zbus::Result<Connection> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // Reuse the connection if it is already open
        if let Some(c) = conn.as_ref() {
            return Ok(c.clone());
        }
        // Otherwise open a new one
        let c = zbus::blocking::connection::Builder::session()?
            .method_timeout(CALL_TIMEOUT)
            .build()?;
        *conn = Some(c.clone());
        Ok(c)
    }

    fn get_focused_window(&self) -> zbus::Result<HashMap<String, OwnedValue>> {
        let reply = self.connection()?.call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some(INTERFACE),
            "GetFocusedWindow",
            &(),
        )?;
        reply.body().deserialize()
    }
}

impl FocusSource for GnomeFocus {
    fn focused_window(&self) -> Result<WindowInfo> {
        let fields = self
            .get_focused_window()
            .map_err(|e| Error::Capture(format!("Hindsight Focus 扩展：{e}")))?;
        window_info(&fields, resolve_exe_path).ok_or_else(|| Error::Capture("没有焦点窗口".into()))
    }
}

/// Converts the focused window returned by the extension into a [`WindowInfo`].
///
/// `fields` is the dictionary returned by the extension's `GetFocusedWindow`. It is empty when no
/// window has the focus, and then this returns None:
/// - `title`: the window title
/// - `wm-class`: the window class, such as `firefox` or `org.gnome.Nautilus`
/// - `app-id`: the app ID GNOME assigns, such as `org.gnome.Nautilus.desktop`; `window:<n>` when
///   GNOME does not recognize the app
/// - `pid`: the PID of the process that owns the window
///
/// See [`app_name`] for how the app name is chosen. `exe_path` looks up the executable path by
/// PID; tests pass a fake one.
fn window_info(
    fields: &HashMap<String, OwnedValue>,
    exe_path: impl Fn(u32) -> Option<String>,
) -> Option<WindowInfo> {
    if fields.is_empty() {
        return None;
    }
    let text = |key: &str| {
        fields
            .get(key)
            .and_then(|v| <&str>::try_from(v).ok())
            .unwrap_or("")
            .trim()
    };
    let pid = fields
        .get("pid")
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0);
    let app_path = if pid > 0 { exe_path(pid) } else { None };

    Some(WindowInfo {
        app_name: app_name(text("wm-class"), text("app-id"), app_path.as_deref()),
        title: text("title").to_string(),
        app_path,
        pid,
    })
}

/// Chooses the app name: the first usable one of
/// 1. `wm-class`: the same value xcap reads as `WM_CLASS` on X11
/// 2. `app-id` without the `.desktop` suffix; skipped when it is the `window:<n>` GNOME gives for
///    an app it does not recognize
/// 3. the executable's file name
///
/// Returns an empty string when none is usable, and the tick skips that round.
fn app_name(wm_class: &str, app_id: &str, app_path: Option<&str>) -> String {
    if !wm_class.is_empty() {
        return wm_class.to_string();
    }
    let app_id = app_id.strip_suffix(".desktop").unwrap_or(app_id);
    if !app_id.is_empty() && !app_id.starts_with("window:") {
        return app_id.to_string();
    }
    app_path
        .and_then(|p| Path::new(p).file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    use zbus::zvariant::Value;

    fn fields(entries: &[(&str, Value<'_>)]) -> HashMap<String, OwnedValue> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.try_to_owned().unwrap()))
            .collect()
    }

    fn exe(path: &'static str) -> impl Fn(u32) -> Option<String> {
        move |_| Some(path.to_string())
    }

    #[test]
    fn reads_all_fields() {
        let f = fields(&[
            ("title", Value::from("main.rs - Hindsight")),
            ("wm-class", Value::from("code")),
            ("app-id", Value::from("code.desktop")),
            ("pid", Value::from(4242u32)),
        ]);
        let info = window_info(&f, exe("/usr/share/code/code")).unwrap();
        assert_eq!(info.app_name, "code");
        assert_eq!(info.title, "main.rs - Hindsight");
        assert_eq!(info.app_path.as_deref(), Some("/usr/share/code/code"));
        assert_eq!(info.pid, 4242);
    }

    #[test]
    fn app_name_prefers_wm_class() {
        assert_eq!(
            app_name(
                "firefox",
                "org.mozilla.firefox.desktop",
                Some("/usr/lib/firefox/firefox-bin")
            ),
            "firefox"
        );
        assert_eq!(
            app_name(
                "",
                "org.mozilla.firefox.desktop",
                Some("/usr/lib/firefox/firefox-bin")
            ),
            "org.mozilla.firefox"
        );
    }

    #[test]
    fn no_focused_window_is_none() {
        // 扩展在没有焦点窗口时返回空字典
        assert!(window_info(&HashMap::new(), exe("/x")).is_none());
    }

    #[test]
    fn app_name_falls_back_to_app_id_then_exe_name() {
        let f = fields(&[
            ("wm-class", Value::from("  ")),
            ("app-id", Value::from("org.gnome.Nautilus.desktop")),
            ("pid", Value::from(7u32)),
        ]);
        assert_eq!(
            window_info(&f, exe("/usr/bin/nautilus")).unwrap().app_name,
            "org.gnome.Nautilus"
        );

        // GNOME 认不出应用时的 "window:<编号>" 不能当应用名
        let f = fields(&[
            ("app-id", Value::from("window:12")),
            ("pid", Value::from(7u32)),
        ]);
        assert_eq!(
            window_info(&f, exe("/opt/tool/bin/tool")).unwrap().app_name,
            "tool"
        );
    }

    #[test]
    fn unknown_pid_skips_the_exe_lookup() {
        let f = fields(&[
            ("wm-class", Value::from("firefox")),
            ("pid", Value::from(0u32)),
        ]);
        let info = window_info(&f, |_| panic!("PID 未知时不该查可执行文件")).unwrap();
        assert_eq!(info.app_path, None);
        assert_eq!(info.pid, 0);
    }

    #[test]
    fn wrong_field_types_are_treated_as_missing() {
        // 以后的扩展版本改了类型也不能让 Hindsight 出错：读不懂就当没有
        let f = fields(&[
            ("title", Value::from(5u32)),
            ("wm-class", Value::from("kitty")),
            ("pid", Value::from("12")),
        ]);
        let info = window_info(&f, exe("/x")).unwrap();
        assert_eq!(info.title, "");
        assert_eq!(info.app_name, "kitty");
        assert_eq!(info.pid, 0);
    }
}
