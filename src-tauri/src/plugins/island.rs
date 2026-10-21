//! 哪个插件占着岛：最后调 `island.show` 的那个。它撤下内容，或者被停用、卸载以后，前一个还没撤
//! 的插件回到岛上。

use std::sync::{Mutex, MutexGuard, PoisonError};

use tauri::AppHandle;

use super::api::has_permission;
use crate::island::content::Content;
use crate::repo::plugins as repo;
use crate::storage::DbPool;

/// 只有一个岛，所以只有一个队列
static QUEUE: Mutex<Queue> = Mutex::new(Queue::new());

/// 放了内容、还没撤的插件和它们的内容，按调 `island.show` 的先后排。最后一个占着岛。
///
/// 改动的方法都返回岛要不要跟着变：占着岛的插件或者它的内容变了才要变。排在后面的插件撤下内容、
/// 插件再交一遍同样的内容，都返回 `false`。
#[derive(Debug, Default)]
pub struct Queue {
    shown: Vec<(String, Content)>,
}

impl Queue {
    pub const fn new() -> Self {
        Self { shown: Vec::new() }
    }

    /// 返回岛上的内容。
    pub fn on_island(&self) -> Option<&Content> {
        self.shown.last().map(|(_, content)| content)
    }

    /// 把插件 `plugin_id` 的内容放到最后，让它占着岛。
    pub fn show(&mut self, plugin_id: &str, content: Content) -> bool {
        self.change(|shown| {
            shown.retain(|(id, _)| id != plugin_id);
            shown.push((plugin_id.to_string(), content));
        })
    }

    /// 撤下插件 `plugin_id` 的内容。
    pub fn hide(&mut self, plugin_id: &str) -> bool {
        self.change(|shown| shown.retain(|(id, _)| id != plugin_id))
    }

    /// 只留下 `keep` 返回 `true` 的插件的内容。
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) -> bool {
        self.change(|shown| shown.retain(|(id, _)| keep(id)))
    }

    fn change(&mut self, f: impl FnOnce(&mut Vec<(String, Content)>)) -> bool {
        let before = self.shown.last().cloned();
        f(&mut self.shown);
        self.shown.last() != before.as_ref()
    }
}

/// 插件 `plugin_id` 把 `content` 放到岛上。
pub fn show(app: &AppHandle, plugin_id: &str, content: Content) {
    let mut queue = lock();
    if queue.show(plugin_id, content) {
        apply(app, &queue);
    }
}

/// 插件 `plugin_id` 撤下它的内容。
pub fn hide(app: &AppHandle, plugin_id: &str) {
    let mut queue = lock();
    if queue.hide(plugin_id) {
        apply(app, &queue);
    }
}

/// 撤下不能再用岛的插件的内容：被停用、卸载了，或者更新以后没有 `island` 权限了。
///
/// 读不出插件列表时只记日志：调用时插件已经装好、卸载、开关完了。
pub async fn refresh(app: &AppHandle, pool: &DbPool) {
    let plugins = match repo::list(pool).await {
        Ok(plugins) => plugins,
        Err(e) => {
            log::warn!("plugins: cannot list the plugins for the island: {e}");
            return;
        }
    };
    let allowed: Vec<String> = plugins
        .into_iter()
        .filter(|p| p.enabled && has_permission(p, "island"))
        .map(|p| p.id)
        .collect();
    let mut queue = lock();
    if queue.retain(|id| allowed.iter().any(|a| a == id)) {
        apply(app, &queue);
    }
}

fn lock() -> MutexGuard<'static, Queue> {
    // 中毒了照样用：队列的每次改动都不会停在一半
    QUEUE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 让岛显示队列里占着岛的内容。要在持锁时调用：这样岛收到改动的顺序，和队列变化的顺序一样。
fn apply(app: &AppHandle, queue: &Queue) {
    match queue.on_island() {
        Some(content) => crate::island::show(app, content.clone()),
        None => crate::island::hide(app),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 收起时右边只有一段字
    fn content(text: &str) -> Content {
        Content::parse(json!({ "closed": { "right": { "type": "text", "text": text } } })).unwrap()
    }

    #[test]
    fn the_last_to_show_has_the_island() {
        let mut queue = Queue::new();
        assert!(queue.show("focus", content("25:00")));
        assert!(queue.show("water", content("喝水")));
        assert_eq!(queue.on_island(), Some(&content("喝水")));
        // 前一个插件更新内容，又回到岛上
        assert!(queue.show("focus", content("24:00")));
        assert_eq!(queue.on_island(), Some(&content("24:00")));
    }

    #[test]
    fn hiding_gives_the_island_back() {
        let mut queue = Queue::new();
        queue.show("focus", content("25:00"));
        queue.show("water", content("喝水"));
        // 占着岛的撤下：前一个回来，内容是它最后交的
        assert!(queue.hide("water"));
        assert_eq!(queue.on_island(), Some(&content("25:00")));
        // 都撤完了：岛上什么都没有
        assert!(queue.hide("focus"));
        assert_eq!(queue.on_island(), None);
    }

    #[test]
    fn the_island_changes_only_with_its_owner_or_content() {
        let mut queue = Queue::new();
        queue.show("focus", content("25:00"));
        queue.show("water", content("喝水"));
        // 排在后面的撤下
        assert!(!queue.hide("focus"));
        // 没放过内容的撤下
        assert!(!queue.hide("never-shown"));
        // 再交一遍同样的内容
        assert!(!queue.show("water", content("喝水")));
        assert_eq!(queue.on_island(), Some(&content("喝水")));
    }

    #[test]
    fn retain_takes_off_the_plugins_left_out() {
        let mut queue = Queue::new();
        queue.show("focus", content("25:00"));
        queue.show("water", content("喝水"));
        queue.show("turned-off", content("关了"));
        // 占着岛的被停用：留下的里面最后一个回来
        assert!(queue.retain(|id| id != "turned-off"));
        assert_eq!(queue.on_island(), Some(&content("喝水")));
        assert!(queue.retain(|_| false));
        assert_eq!(queue.on_island(), None);
    }
}
