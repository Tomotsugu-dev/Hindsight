//! `focus.changed`: when the focus changes, builds the event for the permissions of each plugin,
//! and sends it to the plugin.

use serde::Serialize;
use tauri::AppHandle;
use tokio::sync::mpsc;

use super::{send, PluginEvent};
use crate::capture::{FocusChange, FocusedWindow};
use crate::error::Result;
use crate::plugins::api::categories::{CategoryRef, SuperCategoryRef};
use crate::plugins::api::has_permission;
use crate::repo::categories::{self, FocusCategory};
use crate::repo::plugins::{self as repo, Plugin};
use crate::storage::DbPool;

/// What a plugin gets in a `focus.changed` event.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FocusEvent {
    /// When the focus changed to this, or when the user went away, in RFC 3339
    at: String,
    /// `true` when the user went away. The other fields are then empty
    away: bool,
    /// The app name. Needs `focus.app`
    app: Option<String>,
    /// The category. Needs `focus.category`
    category: Option<CategoryRef>,
    /// The window title. Needs `focus.title`
    title: Option<String>,
    /// The domain of the website. Needs `focus.title`
    host: Option<String>,
}

impl From<&FocusCategory> for CategoryRef {
    fn from(c: &FocusCategory) -> Self {
        Self {
            id: c.id.clone(),
            name: c.name.clone(),
            color: c.color.clone(),
            icon: c.icon.clone(),
            super_category: c.super_category.clone().map(|(id, name, color, icon)| {
                SuperCategoryRef {
                    id,
                    name,
                    color,
                    icon,
                }
            }),
        }
    }
}

/// The fields other than the time that a plugin can get.
#[derive(Clone, Copy, Default, PartialEq)]
struct Fields {
    app: bool,
    category: bool,
    title: bool,
}

impl Fields {
    /// The fields that the permissions of the plugin allow.
    fn of(plugin: &Plugin) -> Self {
        Self {
            app: has_permission(plugin, "focus.app"),
            category: has_permission(plugin, "focus.category"),
            title: has_permission(plugin, "focus.title"),
        }
    }
}

/// Watches the focus, and sends events each time it changes. Returns only when the capture
/// service is gone.
///
/// On an error, it logs the error, drops this change and waits for the next one. It must not
/// panic: in a release build, a panic quits the whole app.
pub async fn run(app: AppHandle, pool: DbPool, mut focus: mpsc::Receiver<FocusChange>) {
    while let Some(change) = focus.recv().await {
        let events = match events_for(&pool, &change).await {
            Ok(events) => events,
            Err(e) => {
                log::warn!("plugins: focus.changed dropped: {e}");
                continue;
            }
        };
        for (to_plugin_id, data) in events {
            send(
                &app,
                to_plugin_id,
                PluginEvent::FocusChanged(Box::new(data)),
            );
        }
    }
}

/// Returns the id of each plugin to send to, with its FocusEvent.
async fn events_for(pool: &DbPool, change: &FocusChange) -> Result<Vec<(String, FocusEvent)>> {
    let (at, window) = match change {
        FocusChange::Window(w) => (w.at, Some(w)),
        FocusChange::Away { at } => (*at, None),
    };
    let mut plugins = Vec::new();
    // True when a field other than the time is needed
    let mut wants_more = false;
    for p in repo::list(pool).await? {
        if !(p.enabled && p.runs_in_background && has_permission(&p, "focus.changed")) {
            continue;
        }
        let fields = Fields::of(&p);
        wants_more |= fields != Fields::default();
        plugins.push((p.id, fields));
    }
    let category = match window {
        Some(w) if wants_more && !w.excluded => {
            categories::classify(pool, &w.app_name, w.url_host.as_deref()).await?
        }
        _ => None,
    };
    let hidden = category.as_ref().is_some_and(|c| c.id == "hidden");
    // The window plugins can see: not one that matches an ignore rule, and not a hidden app
    let visible = window.filter(|w| !w.excluded && !hidden);
    Ok(plugins
        .into_iter()
        .map(|(id, fields)| {
            let event = match visible {
                Some(w) => assemble(w, fields, category.as_ref()),
                // The user went away, or the statistics do not show this window: only the time
                None => FocusEvent {
                    at: at.to_rfc3339(),
                    away: window.is_none(),
                    ..Default::default()
                },
            };
            (id, event)
        })
        .collect())
}

/// Builds the event from the fields the plugin can get.
fn assemble(
    window: &FocusedWindow,
    fields: Fields,
    category: Option<&FocusCategory>,
) -> FocusEvent {
    FocusEvent {
        at: window.at.to_rfc3339(),
        away: false,
        app: fields.app.then(|| window.app_name.clone()),
        category: category.filter(|_| fields.category).map(CategoryRef::from),
        // No title for a window that matches a privacy keyword
        title: (fields.title && !window.private).then(|| window.title.clone()),
        host: if fields.title {
            window.url_host.clone()
        } else {
            // url_host is already None for a window that matches a privacy keyword
            None
        },
    }
}

#[cfg(test)]
mod tests {
    use chrono::Local;

    use super::*;
    use crate::plugins::test_util::plugin_with_permissions;
    use crate::repo::test_util::fresh_test_pool;

    const ALL: [&str; 4] = [
        "focus.changed",
        "focus.app",
        "focus.category",
        "focus.title",
    ];

    /// 一个分类在「工作」大类里的焦点。
    fn code() -> FocusCategory {
        FocusCategory {
            id: "code".into(),
            name: "编程".into(),
            color: "#a78bfa".into(),
            icon: "Code".into(),
            super_category: Some((
                "work".into(),
                "工作".into(),
                "#8b5cf6".into(),
                "Briefcase".into(),
            )),
        }
    }

    /// 插件拿到的 JSON 整个锁在这里：插件按这些键取值，改键名、加减键、把 null 改成不给，
    /// 已发布的插件都会读错。只有 focus.changed 时只有时间；四个权限都有时全都有。
    #[test]
    fn the_json_plugins_get() {
        let window = FocusedWindow::for_test("Chrome", "GitHub", Some("github.com"));
        let at = window.at.to_rfc3339();
        let json = |permissions: &[&str]| {
            let fields = Fields::of(&plugin_with_permissions("p", permissions));
            serde_json::to_value(assemble(&window, fields, Some(&code()))).unwrap()
        };

        assert_eq!(
            json(&["focus.changed"]),
            serde_json::json!({
                "at": at,
                "away": false,
                "app": null,
                "category": null,
                "title": null,
                "host": null,
            })
        );
        assert_eq!(
            json(&ALL),
            serde_json::json!({
                "at": at,
                "away": false,
                "app": "Chrome",
                "category": {
                    "id": "code",
                    "name": "编程",
                    "color": "#a78bfa",
                    "icon": "Code",
                    "superCategory": {
                        "id": "work",
                        "name": "工作",
                        "color": "#8b5cf6",
                        "icon": "Briefcase",
                    },
                },
                "title": "GitHub",
                "host": "github.com",
            })
        );
    }

    /// 命中隐私关键词的窗口不给标题和域名，应用名和分类照给。
    #[test]
    fn private_windows_give_no_title() {
        let mut window = FocusedWindow::for_test("Chrome", "银行", None);
        window.private = true;
        let fields = Fields::of(&plugin_with_permissions("p", &ALL));
        let event = assemble(&window, fields, Some(&code()));
        assert_eq!(event.app.as_deref(), Some("Chrome"));
        assert!(event.category.is_some());
        assert_eq!((event.title, event.host), (None, None));
    }

    /// 隐藏的应用按分类认出来，命中忽略规则的不查分类，用户离开时没有窗口：
    /// 插件四个权限都有也只拿到时间。只有离开时 away 是 true。
    #[tokio::test]
    async fn hidden_windows_and_away_give_only_the_time() {
        let pool = fresh_test_pool().await;
        repo::save(&pool, plugin_with_permissions("focus", &ALL))
            .await
            .unwrap();
        categories::assign_app(&pool, "Secret", "hidden")
            .await
            .unwrap();
        categories::assign_app(&pool, "Code", "code").await.unwrap();

        let secret = FocusedWindow::for_test("Secret", "账户余额", None);
        let mut excluded = FocusedWindow::for_test("Code", "main.rs", None);
        excluded.excluded = true;
        let away = Local::now();
        for (case, at, change, is_away) in [
            ("secret", secret.at, FocusChange::Window(secret), false),
            (
                "excluded",
                excluded.at,
                FocusChange::Window(excluded),
                false,
            ),
            ("away", away, FocusChange::Away { at: away }, true),
        ] {
            let events = events_for(&pool, &change).await.unwrap();
            let only_time = FocusEvent {
                at: at.to_rfc3339(),
                away: is_away,
                ..Default::default()
            };
            assert_eq!(events[0].1, only_time, "{case}");
        }
    }

    /// 只推给开着、在后台运行、有 focus.changed 的插件。
    #[tokio::test]
    async fn sends_only_to_plugins_that_can_get_it() {
        let pool = fresh_test_pool().await;
        // 三个插件里只有第一个收得到：第二个不在后台运行，第三个没有 focus.changed
        let focus = plugin_with_permissions("focus", &["focus.changed", "focus.category"]);
        let mut ui_only = plugin_with_permissions("ui-only", &["focus.changed"]);
        ui_only.runs_in_background = false;
        let stats = plugin_with_permissions("stats", &["stats.today"]);
        for p in [focus, ui_only, stats] {
            repo::save(&pool, p).await.unwrap();
        }
        categories::assign_app(&pool, "Code", "code").await.unwrap();
        let code = FocusChange::Window(FocusedWindow::for_test("Code", "main.rs", None));

        let events = events_for(&pool, &code).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "focus");
        assert_eq!(events[0].1.category.as_ref().unwrap().id, "code");

        repo::set_enabled(&pool, "focus", false).await.unwrap();
        assert!(events_for(&pool, &code).await.unwrap().is_empty());
    }
}
