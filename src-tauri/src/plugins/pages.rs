//! Plugin pages: the page list for the frontend, and the pages' own windows. The frontend shows
//! pages inside the main window itself. Each plugin has at most one window of its own. When
//! plugins change, this closes the windows that should no longer be open, and sends the new page
//! list to all windows.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

use crate::error::{Error, Result};
use crate::plugins::events::window::{self as window_event, WindowState};
use crate::plugins::file_reader;
use crate::repo::plugins as repo;
use crate::storage::DbPool;

/// Label prefix of a page's own window, followed by the plugin id.
/// `capabilities/plugin-window.json` uses the same prefix.
const LABEL_PREFIX: &str = "plugin-window-";

/// Name of the event sent to all windows when the page list changes.
const CHANGED_EVENT: &str = "plugin-pages-changed";

/// How long a window must stay still before its position and size are saved, so that a drag
/// does not write to the table on every move.
const SAVE_DELAY: Duration = Duration::from_millis(500);

/// 每个插件的窗口上次告诉插件的样子，按插件 id。窗口关掉的插件不在里面
static STATES: Mutex<BTreeMap<String, WindowState>> = Mutex::new(BTreeMap::new());

/// A plugin that has a page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPage {
    pub id: String,
    pub name: String,
    /// See [`repo::UiSettings::opens_in`]
    pub opens_in: Option<String>,
    pub url: String,
}

/// Lists the enabled plugins that have a page.
pub async fn list(pool: &DbPool) -> Result<Vec<PluginPage>> {
    let plugins = repo::list(pool).await?;
    Ok(plugins
        .into_iter()
        .filter(|p| p.enabled && p.ui.has_page)
        .map(|p| PluginPage {
            url: file_reader::page_url(&p.id, &p.version),
            id: p.id,
            name: p.name,
            opens_in: p.ui.opens_in,
        })
        .collect())
}

/// Closes the windows of plugins that no longer have a page (turned off, uninstalled, or the new
/// version has no page), then sends the new page list to all windows. After an update, a page
/// window sees the new URL and reloads the page itself.
///
/// Failures are only logged: when this runs, the plugin has already been installed, uninstalled,
/// enabled or disabled.
pub async fn refresh(app: &AppHandle, pool: &DbPool) {
    let pages = match list(pool).await {
        Ok(pages) => pages,
        Err(e) => {
            log::warn!("plugins: cannot list plugin pages: {e}");
            return;
        }
    };
    for (label, window) in app.webview_windows() {
        // Skip the windows that are not a plugin page's own window
        let Some(id) = label.strip_prefix(LABEL_PREFIX) else {
            continue;
        };
        if !pages.iter().any(|p| p.id == id) {
            if let Err(e) = window.destroy() {
                log::warn!("plugins: cannot close the window of {id}: {e}");
            }
        }
    }
    if let Err(e) = app.emit(CHANGED_EVENT, pages) {
        log::warn!("plugins: cannot send the page list: {e}");
    }
}

/// Opens the page of plugin `id` in its own window, or brings the window to the front if it is
/// already open.
///
/// The window uses its last position and size. If the last position is on no screen (for
/// example, an external screen was unplugged), the window opens in the center.
///
/// # Errors
///
/// - The plugin is not installed, is turned off, or has no page: [`Error::PluginNoPage`]
pub async fn open_window(app: &AppHandle, pool: &DbPool, id: &str) -> Result<()> {
    // If the window is already open, bring it to the front.
    if let Some(window) = app.get_webview_window(&label(id)) {
        return bring_to_front(&window).map_err(window_error);
    }
    // The window is not open yet, so open a new one.
    let plugin = repo::get(pool, id)
        .await?
        .filter(|p| p.enabled && p.ui.has_page)
        .ok_or_else(|| Error::PluginNoPage(id.to_string()))?;
    let ui = &plugin.ui;
    let builder = WebviewWindowBuilder::new(
        app,
        label(id),
        WebviewUrl::App("src/plugin-window/index.html".into()),
    )
    .title(&plugin.name)
    .inner_size(f64::from(ui.window_width), f64::from(ui.window_height));
    let builder = match (ui.window_x, ui.window_y) {
        (Some(x), Some(y)) if on_a_screen(&screens(app), x, y) => {
            builder.position(f64::from(x), f64::from(y))
        }
        _ => builder.center(),
    };
    let window = builder.build().map_err(window_error)?;
    report(app, id, WindowState::Open);

    let changes = Arc::new(AtomicU64::new(0));
    let (app, id) = (app.clone(), id.to_string());
    window.on_window_event(move |event| match event {
        WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
            save_later(&app, &id, &changes);
            // Windows 上最小化、还原都会发 Resized
            report_now(&app, &id);
        }
        WindowEvent::Focused(_) => report_now(&app, &id),
        WindowEvent::Destroyed => report(&app, &id, WindowState::Closed),
        _ => {}
    });
    Ok(())
}

/// 插件窗口最小化或者还原时告诉那个插件。在主线程上调一次。
pub fn watch_minimize(app: &AppHandle) {
    let app = app.clone();
    crate::platform::on_minimize_changed(move || {
        for label in app.webview_windows().into_keys() {
            if let Some(id) = label.strip_prefix(LABEL_PREFIX) {
                report_now(&app, id);
            }
        }
    });
}

/// 看插件 `id` 的窗口现在有没有最小化，告诉插件。
fn report_now(app: &AppHandle, id: &str) {
    let Some(window) = app.get_webview_window(&label(id)) else {
        return;
    };
    match window.is_minimized() {
        Ok(true) => report(app, id, WindowState::Minimized),
        Ok(false) => report(app, id, WindowState::Open),
        Err(e) => log::warn!("plugins: cannot read the window of {id}: {e}"),
    }
}

/// 告诉插件 `id` 它的窗口现在是 `state`。和上次告诉它的一样就不发。
fn report(app: &AppHandle, id: &str, state: WindowState) {
    let changed = remember(&mut STATES.lock().unwrap(), id, state);
    if changed {
        window_event::changed(app, id, state);
    }
}

/// 把插件 `id` 的窗口记成 `state`。返回和上次记的是不是不一样。
fn remember(states: &mut BTreeMap<String, WindowState>, id: &str, state: WindowState) -> bool {
    match state {
        // 关掉的不记：下次打开从头算
        WindowState::Closed => states.remove(id).is_some(),
        _ => states.insert(id.to_string(), state) != Some(state),
    }
}

// Returns the label of the plugin window for the given plugin id.
fn label(id: &str) -> String {
    format!("{LABEL_PREFIX}{id}")
}

// Brings the given plugin window to the front: unminimizes, shows, and focuses it.
fn bring_to_front(window: &WebviewWindow) -> tauri::Result<()> {
    window.unminimize()?;
    window.show()?;
    window.set_focus()
}

fn window_error(e: tauri::Error) -> Error {
    Error::Other(format!("plugin window: {e}"))
}

/// Reads the window's position and size now, and writes them to the table if the window does
/// not move again within [`SAVE_DELAY`].
///
/// They are read right in the event, so they are saved even if the user closes the window just
/// after a drag.
fn save_later(app: &AppHandle, id: &str, changes: &Arc<AtomicU64>) {
    let Some(window) = app.get_webview_window(&label(id)) else {
        return;
    };
    let (x, y, width, height) = match bounds(&window) {
        Ok(Some(bounds)) => bounds,
        Ok(None) => return,
        Err(e) => {
            log::warn!("plugins: cannot read the window of {id}: {e}");
            return;
        }
    };
    let change = changes.fetch_add(1, Ordering::Relaxed) + 1;
    let (app, id, changes) = (app.clone(), id.to_string(), Arc::clone(changes));
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SAVE_DELAY).await;
        // Moved again in the meantime: that move saves instead
        if changes.load(Ordering::Relaxed) != change {
            return;
        }
        let pool = app.state::<DbPool>();
        if let Err(e) = repo::set_window_bounds(&pool, &id, x, y, width, height).await {
            log::warn!("plugins: cannot save the window of {id}: {e}");
        }
    });
}

/// Returns the window's position and size, in logical pixels. Returns `None` when the window is
/// minimized, maximized or full screen. On Windows, a minimized window is at (-32000, -32000).
fn bounds(window: &WebviewWindow) -> tauri::Result<Option<(i32, i32, u32, u32)>> {
    if window.is_minimized()? || window.is_maximized()? || window.is_fullscreen()? {
        return Ok(None);
    }
    let scale = window.scale_factor()?;
    let position = window.outer_position()?.to_logical::<i32>(scale);
    let size = window.inner_size()?.to_logical::<u32>(scale);
    Ok(Some((position.x, position.y, size.width, size.height)))
}

/// The area of one screen, in logical pixels: x and y of the top-left corner, width, height.
type Screen = (f64, f64, f64, f64);

/// Returns the area of each screen, converted to logical pixels with that screen's own scale
/// factor.
fn screens(app: &AppHandle) -> Vec<Screen> {
    let monitors = match app.available_monitors() {
        Ok(monitors) => monitors,
        Err(e) => {
            log::warn!("plugins: cannot list screens: {e}");
            return Vec::new();
        }
    };
    monitors
        .iter()
        .map(|m| {
            let position = m.position().to_logical::<f64>(m.scale_factor());
            let size = m.size().to_logical::<f64>(m.scale_factor());
            (position.x, position.y, size.width, size.height)
        })
        .collect()
}

fn on_a_screen(screens: &[Screen], x: i32, y: i32) -> bool {
    let (x, y) = (f64::from(x), f64::from(y));
    screens.iter().any(|&(left, top, width, height)| {
        left <= x && x < left + width && top <= y && y < top + height
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::test_util::plugin_row;
    use crate::repo::test_util::fresh_test_pool;

    /// 只列出开着、有页面的插件，地址指向这个版本的页面。
    #[tokio::test]
    async fn lists_enabled_plugins_with_a_page() {
        let pool = fresh_test_pool().await;
        let mut no_page = plugin_row("no-page", true, true);
        no_page.ui.has_page = false;
        for plugin in [
            plugin_row("hindsight-focus", true, true),
            plugin_row("turned-off", false, true),
            no_page,
        ] {
            repo::save(&pool, plugin).await.unwrap();
        }

        assert_eq!(
            list(&pool).await.unwrap(),
            [PluginPage {
                id: "hindsight-focus".into(),
                name: "hindsight-focus".into(),
                opens_in: Some("main".into()),
                url: file_reader::page_url("hindsight-focus", "0.1.0"),
            }]
        );
    }

    /// 窗口的样子变了才告诉插件：失去焦点、移动这些事件也会来问，样子没变就不发。
    #[test]
    fn tells_the_plugin_only_when_the_window_changes() {
        let mut states = BTreeMap::new();
        let mut tell = |state| remember(&mut states, "hindsight-focus", state);
        assert!(tell(WindowState::Open));
        assert!(!tell(WindowState::Open));
        assert!(tell(WindowState::Minimized));
        assert!(!tell(WindowState::Minimized));
        assert!(tell(WindowState::Open));
        assert!(tell(WindowState::Closed));
        // 没开着的窗口关不了第二次
        assert!(!tell(WindowState::Closed));
        // 再打开又从头算
        assert!(tell(WindowState::Open));
    }

    /// 记下的位置在某块屏幕上才用；外接屏拔掉以后，原来在它上面的位置就不算了。
    #[test]
    fn position_must_be_on_a_screen() {
        // 笔记本屏幕，左边接了一块外接屏
        let laptop = (0.0, 0.0, 1512.0, 982.0);
        let external = (-1920.0, 0.0, 1920.0, 1080.0);
        assert!(on_a_screen(&[laptop, external], -1200, 80));
        assert!(!on_a_screen(&[laptop], -1200, 80));
        // 右边缘已经在屏幕外
        assert!(!on_a_screen(&[laptop], 1512, 80));
    }
}
