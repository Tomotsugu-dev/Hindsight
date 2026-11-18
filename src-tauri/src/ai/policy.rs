//! AI 能看到哪些数据。规则只写在这里，用数据的地方照着过滤。
//!
//! 主库的活动带着分组和「已忽略」标记，条件直接写成 SQL，见 [`AiDataPolicy::main_sql`]。
//! 记忆库只有进程名和窗口标题，要用主库里的分组、分类和忽略规则写出条件，
//! 见 [`AiDataPolicy::screen_filter`]。

use std::collections::HashMap;

use rusqlite::OptionalExtension;

use crate::capture::ignore::{is_excluded, IgnoreRule};
use crate::error::Result;
use crate::memory::search::SessionFilter;
use crate::repo::settings::Settings;
use crate::storage::{Connection, SqliteResultExt};

/// 主库里 AI 能看到的活动，见 [`AiDataPolicy::main_sql`]。
const MAIN_SQL: &str = "c.id IS NOT 'hidden' AND c.id IS NOT 'private' AND a.excluded = 0";

/// 没分类的应用算进这个分类，跟统计页一样。
const OTHER: &str = "other";

/// AI 能看到什么。每个问题从主库的设置生成一次。
#[derive(Debug, Default)]
pub struct AiDataPolicy {
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

impl AiDataPolicy {
    /// 从主库读分组、分类和忽略规则。
    pub async fn load(main: &Connection) -> Result<Self> {
        let policy = main
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
        Ok(policy)
    }

    /// 主库里 AI 能看到的活动，写成 SQL 条件。配 [`from_stats_category_sql`] 用，
    /// 它给出别名 `a` 和 `c`。
    ///
    /// 跟统计页一样：「不计入统计」分类和已忽略的窗口不算。「隐私」分类本来不记录，
    /// 但删除失败时旧数据会留在库里，所以也去掉。
    ///
    /// [`from_stats_category_sql`]: crate::repo::sql::from_stats_category_sql
    pub fn main_sql(&self) -> &'static str {
        MAIN_SQL
    }

    /// 搜屏幕文字时只要哪些段：AI 能看的，再按 `apps` 和 `categories` 筛。
    ///
    /// `processes` 是记忆库里有屏幕文字的进程名。`apps`、`categories` 是空的就不按它筛。
    /// 跟主库一样：「不计入统计」「隐私」分类里的应用、命中忽略规则的窗口都不给看。
    /// 记忆库没有网站，网站规则用不上。
    pub fn screen_filter(
        &self,
        processes: Vec<String>,
        apps: &[String],
        categories: &[String],
    ) -> SessionFilter {
        let apps = processes
            .into_iter()
            .filter(|p| {
                self.shows_app(p)
                    && (apps.is_empty() || apps.iter().any(|a| self.is_app(p, a)))
                    && (categories.is_empty() || categories.iter().any(|c| self.in_category(p, c)))
            })
            .collect();
        // 只忽略某些标题的规则写成条件；整个应用都忽略的，上面已经去掉了
        let hidden_windows = self
            .ignore_rules
            .iter()
            .filter_map(|r| {
                let word = r.title_keyword.as_deref()?.trim();
                (!word.is_empty()).then(|| (r.process_name.trim().to_string(), word.to_string()))
            })
            .collect();
        SessionFilter {
            apps,
            hidden_windows,
        }
    }

    /// 应用的显示名。没分组就是进程名。
    pub fn display_name<'a>(&'a self, process: &'a str) -> &'a str {
        self.groups
            .get(process)
            .map_or(process, |g| g.display_name.as_str())
    }

    /// 这个应用的屏幕文字能不能给 AI 看：不在「不计入统计」「隐私」分类里，也没有被整个忽略。
    fn shows_app(&self, process: &str) -> bool {
        let category = self.category_id(process);
        // 标题是空的，只有不看标题的规则能命中
        category != "hidden"
            && category != "private"
            && !is_excluded(process, "", &self.ignore_rules)
    }

    /// 进程名或显示名里有没有 `pattern`。英文不分大小写。
    fn is_app(&self, process: &str, pattern: &str) -> bool {
        contains(process, pattern) || contains(self.display_name(process), pattern)
    }

    /// 应用所在分类的名字或 id 里有没有 `pattern`。英文不分大小写。
    fn in_category(&self, process: &str, pattern: &str) -> bool {
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
    /// 忽略 Terminal 里标题有 download 的窗口、整个 Zoom；Slack 的规则关键词是空白。
    async fn loaded() -> AiDataPolicy {
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
                         json('[{"processName": "Terminal", "titleKeyword": " download "},
                                {"processName": "Zoom"},
                                {"processName": "Slack", "titleKeyword": "  "}]'))
                     WHERE id = 1;"#,
                )
                .db()
            })
            .await
            .unwrap();
        AiDataPolicy::load(&pool.0).await.unwrap()
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn the_filter_drops_hidden_private_and_wholly_ignored_apps() {
        let v = loaded().await;
        let processes = names(&[
            "secret.exe",
            "Signal",
            "Zoom",
            "Terminal",
            "Slack",
            "Code.exe",
            "unknown.exe",
            "",
        ]);
        let f = v.screen_filter(processes, &[], &[]);
        assert_eq!(f.apps, ["Terminal", "Slack", "Code.exe", "unknown.exe", ""]);
        // 只忽略某些标题的规则写成条件，关键词去掉前后空白；空白关键词的规则什么都不忽略
        assert_eq!(
            f.hidden_windows,
            [("Terminal".to_string(), "download".to_string())]
        );
    }

    #[tokio::test]
    async fn the_filter_keeps_only_the_asked_apps_and_categories() {
        let v = loaded().await;
        let all = names(&["Code.exe", "Code", "notepad"]);
        let f = v.screen_filter(all.clone(), &names(&["visual studio"]), &[]);
        assert_eq!(f.apps, ["Code.exe", "Code"]);
        let f = v.screen_filter(all.clone(), &[], &names(&["其他"]));
        assert_eq!(f.apps, ["notepad"]);
        // 应用和分类都给时，两个都要满足
        let f = v.screen_filter(all, &names(&["code"]), &names(&["其他"]));
        assert!(f.apps.is_empty());
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
        let v = AiDataPolicy::load(&pool.0).await.unwrap();
        let f = v.screen_filter(names(&["Terminal"]), &[], &[]);
        assert_eq!(f.apps, ["Terminal"]);
        assert!(f.hidden_windows.is_empty());
    }
}
