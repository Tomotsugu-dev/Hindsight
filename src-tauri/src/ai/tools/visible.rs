//! AI 能看到哪些数据。规则只写在这里，工具照着用。
//!
//! 主库的活动带着分组和「已忽略」标记，条件直接写成 SQL，见 [`MAIN_VISIBLE`]。
//! 记忆库只有进程名和窗口标题，要用主库里的分组、分类和忽略规则来判断，见 [`Visibility`]。

use std::collections::HashMap;

use rusqlite::OptionalExtension;

use crate::capture::ignore::{is_excluded, IgnoreRule};
use crate::error::Result;
use crate::repo::settings::Settings;
use crate::storage::{Connection, SqliteResultExt};

/// 主库里 AI 能看到的活动。配 [`from_stats_category_sql`] 用，它给出别名 `a` 和 `c`。
///
/// 跟统计页一样：「不计入统计」分类和已忽略的窗口不算。「隐私」分类本来不记录，
/// 但删除失败时旧数据会留在库里，所以也去掉。
///
/// [`from_stats_category_sql`]: crate::repo::sql::from_stats_category_sql
pub(super) const MAIN_VISIBLE: &str =
    "c.id IS NOT 'hidden' AND c.id IS NOT 'private' AND a.excluded = 0";

/// 没分类的应用算进这个分类，跟统计页一样。
const OTHER: &str = "other";

/// 主库里，记忆库要用的设置：分组、分类和忽略规则。每个问题开始时读一次。
#[derive(Debug, Default)]
pub(super) struct Visibility {
    /// 进程名 → 它所在的分组。
    groups: HashMap<String, Group>,
    /// 没删除的分类：id → 名字。
    categories: HashMap<String, String>,
    ignore_rules: Vec<IgnoreRule>,
}

/// 一个应用分组。
#[derive(Debug)]
struct Group {
    display_name: String,
    category_id: Option<String>,
}

impl Visibility {
    /// 从主库读分组、分类和忽略规则。
    pub(super) async fn load(main: &Connection) -> Result<Self> {
        let visibility = main
            .call(|conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT gm.process_name, g.display_name, g.category_id
                         FROM app_group_members gm JOIN app_groups g ON g.id = gm.group_id
                         WHERE gm.deleted_at IS NULL AND g.deleted_at IS NULL",
                    )
                    .db()?;
                let groups = stmt
                    .query_map([], |r| {
                        let group = Group {
                            display_name: r.get(1)?,
                            category_id: r.get(2)?,
                        };
                        Ok((r.get(0)?, group))
                    })
                    .db()?
                    .collect::<rusqlite::Result<_>>()
                    .db()?;
                let mut stmt = conn
                    .prepare("SELECT id, name FROM categories WHERE deleted_at IS NULL")
                    .db()?;
                let categories = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .db()?
                    .collect::<rusqlite::Result<_>>()
                    .db()?;
                let data: Option<String> = conn
                    .query_row("SELECT data FROM settings_store WHERE id = 1", [], |r| {
                        r.get(0)
                    })
                    .optional()
                    .db()?;
                // 读不懂时跟设置一样，当作没有规则
                let ignore_rules = data
                    .and_then(|d| serde_json::from_str::<Settings>(&d).ok())
                    .map(|s| s.ignore_rules)
                    .unwrap_or_default();
                Ok(Self {
                    groups,
                    categories,
                    ignore_rules,
                })
            })
            .await?;
        Ok(visibility)
    }

    /// 记忆库里的一段屏幕文字能不能给 AI 看。
    ///
    /// 跟主库一样：「不计入统计」「隐私」分类里的应用、命中忽略规则的窗口都不给看。
    /// 记忆库没有网站，网站规则用不上。
    pub(super) fn shows_screen(&self, process: &str, title: &str) -> bool {
        let category = self.category_id(process);
        category != "hidden"
            && category != "private"
            && !is_excluded(process, title, &self.ignore_rules)
    }

    /// 应用的显示名。没分组就是进程名。
    pub(super) fn display_name<'a>(&'a self, process: &'a str) -> &'a str {
        self.groups
            .get(process)
            .map_or(process, |g| g.display_name.as_str())
    }

    /// 进程名或显示名里有没有 `pattern`。英文不分大小写。
    pub(super) fn is_app(&self, process: &str, pattern: &str) -> bool {
        contains(process, pattern) || contains(self.display_name(process), pattern)
    }

    /// 应用所在分类的名字或 id 里有没有 `pattern`。英文不分大小写。
    pub(super) fn in_category(&self, process: &str, pattern: &str) -> bool {
        let id = self.category_id(process);
        let name = self.categories.get(id).map_or("", String::as_str);
        contains(id, pattern) || contains(name, pattern)
    }

    /// 应用算进的分类。没分组、没分类、分类删了，都算「其他」。
    fn category_id<'a>(&'a self, process: &str) -> &'a str {
        match self
            .groups
            .get(process)
            .and_then(|g| g.category_id.as_deref())
        {
            Some(id) if self.categories.contains_key(id) => id,
            _ => OTHER,
        }
    }
}

fn contains(hay: &str, needle: &str) -> bool {
    hay.to_lowercase().contains(&needle.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::fresh_test_pool;

    /// 主库：Code 和 Code.exe 在「编程」的同一个分组里；secret 不计入统计；Signal 隐私；
    /// 忽略 Terminal 里标题有 download 的窗口。
    async fn loaded() -> Visibility {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|c| {
                c.execute_batch(
                    r#"INSERT INTO app_groups(id, display_name, category_id) VALUES
                         ('g-code', 'Visual Studio Code', 'code'),
                         ('g-secret', 'Secret', 'hidden'),
                         ('g-signal', 'Signal', 'private'),
                         ('g-old', 'Old', 'gone'),
                         ('g-none', 'Plain', 'none');
                     INSERT INTO app_group_members(process_name, group_id) VALUES
                         ('Code.exe', 'g-code'), ('Code', 'g-code'),
                         ('secret.exe', 'g-secret'), ('Signal', 'g-signal'),
                         ('old.exe', 'g-old'), ('plain.exe', 'g-none');
                     UPDATE settings_store SET data = json_set(data, '$.ignoreRules',
                         json('[{"processName": "Terminal", "titleKeyword": "download"}]'))
                     WHERE id = 1;"#,
                )
                .db()
            })
            .await
            .unwrap();
        Visibility::load(&pool.0).await.unwrap()
    }

    #[tokio::test]
    async fn hides_hidden_private_and_ignored_windows() {
        let v = loaded().await;
        assert!(!v.shows_screen("secret.exe", "anything"));
        assert!(!v.shows_screen("Signal", "chat"));
        // 忽略规则跟采集用的是同一个函数：进程名一样、标题里有关键词，不分大小写
        assert!(!v.shows_screen("Terminal", "Download videos"));
        assert!(v.shows_screen("Terminal", "vim main.rs"));
        assert!(v.shows_screen("Code.exe", "main.rs"));
        assert!(v.shows_screen("unknown.exe", "main.rs"));
    }

    #[tokio::test]
    async fn display_name_comes_from_the_group() {
        let v = loaded().await;
        assert_eq!(v.display_name("Code"), "Visual Studio Code");
        assert_eq!(v.display_name("Code.exe"), "Visual Studio Code");
        assert_eq!(v.display_name("notepad"), "notepad");
    }

    #[tokio::test]
    async fn an_app_matches_by_process_or_display_name() {
        let v = loaded().await;
        assert!(v.is_app("Code.exe", "visual studio"));
        assert!(v.is_app("Code.exe", "code.EXE"));
        assert!(!v.is_app("notepad", "code"));
    }

    #[tokio::test]
    async fn categories_match_by_name_or_id_and_default_to_other() {
        let v = loaded().await;
        assert!(v.in_category("Code", "编程"));
        assert!(v.in_category("Code", "CODE"));
        assert!(!v.in_category("Code", "其他"));
        // 没分组、分类已不存在、用户设成不分类，都算「其他」
        for process in ["notepad", "old.exe", "plain.exe"] {
            assert!(v.in_category(process, "其他"), "{process}");
            assert!(v.in_category(process, "other"), "{process}");
        }
    }

    #[tokio::test]
    async fn settings_that_cannot_be_read_mean_no_ignore_rules() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|c| {
                c.execute_batch("UPDATE settings_store SET data = 'not json' WHERE id = 1")
                    .db()
            })
            .await
            .unwrap();
        let v = Visibility::load(&pool.0).await.unwrap();
        assert!(v.shows_screen("Terminal", "Download videos"));
    }
}
