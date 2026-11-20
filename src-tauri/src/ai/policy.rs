//! What data the AI can see. The rules live only here, and code that reads the data
//! filters by them.
//!
//! Activities in the main database carry their group and an "ignored" flag, so the rules
//! become SQL there, see [`AiDataPolicy::main_filter`].
//! The memory database has only process names and window titles, not categories. The caller
//! looks up the categories and passes them in, see [`AiDataPolicy::memory_filter`].

use std::collections::BTreeSet;

use rusqlite::OptionalExtension;

use crate::ai::config::{ModuleProfiles, Visibility};
use crate::capture::ignore::{is_excluded, IgnoreRule};
use crate::capture::screenshot::policy::matches_any;
use crate::error::Result;
use crate::memory::search::SessionFilter;
use crate::repo::settings::Settings;
use crate::storage::{Connection, SqliteResultExt};

/// A feature that uses AI. Each feature keeps its own profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(dead_code)]
pub enum AiModule {
    Daily,
    Weekly,
    Chat,
}

/// What the AI can see. Built from the main database settings once per question.
#[derive(Debug)]
pub struct AiDataPolicy {
    /// Categories the AI can see.
    shown_categories: BTreeSet<String>,
    /// Privacy keywords from the main settings. If a process name or a title contains one,
    /// the AI does not get it.
    privacy_keywords: Vec<String>,
    /// Ignore rules from the main settings.
    ignore_rules: Vec<IgnoreRule>,
}

impl AiDataPolicy {
    /// Returns what the AI can see in `module`: the categories it can see, and the privacy
    /// keywords and ignore rules from the main settings.
    pub async fn load(main: &Connection, module: AiModule, profile: Option<&str>) -> Result<Self> {
        let (settings, category_ids) = read_main(main).await?;
        let profiles = &module_profiles(&settings.ai.visibility, module).profiles;
        let shown_categories = match profile.and_then(|id| profiles.get(id)) {
            Some(p) => p
                .categories
                .iter()
                .filter(|id| *id != "private")
                .cloned()
                .collect(),
            // Same as the stats page. Private apps are not recorded, but old rows stay if
            // deleting them failed
            None => category_ids
                .into_iter()
                .filter(|id| id != "hidden" && id != "private")
                .collect(),
        };
        let privacy_keywords = settings
            .privacy_app_keywords
            .iter()
            .map(|k| k.trim())
            .filter(|k| !k.is_empty())
            .map(str::to_string)
            .collect();
        Ok(Self {
            shown_categories,
            privacy_keywords,
            ignore_rules: settings.ignore_rules,
        })
    }

    /// Returns the WHERE condition to add to a main database query. Use it with
    /// [`from_stats_category_sql`], which gives the aliases `a` and `c`.
    ///
    /// [`from_stats_category_sql`]: crate::repo::sql::from_stats_category_sql
    pub fn main_filter(&self) -> String {
        // Category ids and keywords come from the user, so escape them before adding them
        let ids: Vec<String> = self
            .shown_categories
            .iter()
            .map(|id| sql_text(id))
            .collect();
        // Unclassified activities count as Other. With no category selected this is IN (),
        // which matches nothing
        let mut conds = vec![
            "a.excluded = 0".to_string(),
            format!("COALESCE(c.id, 'other') IN ({})", ids.join(", ")),
        ];
        // SQLite's lower changes only English letters, so the keywords are lowercased the
        // same way to match
        for k in &self.privacy_keywords {
            let k = sql_text(&k.to_ascii_lowercase());
            conds.push(format!(
                "instr(lower(a.process_name), {k}) = 0 \
                 AND instr(lower(COALESCE(a.window_title, '')), {k}) = 0"
            ));
        }
        conds.join(" AND ")
    }

    /// Returns the filter for the memory database: the processes in `apps` the AI can see,
    /// and the windows to drop.
    ///
    /// `apps` holds each process name in the memory database with the category it counts
    /// toward. The memory database has no websites, so website rules do not apply: a browser
    /// counts toward its own category.
    pub fn memory_filter(&self, apps: Vec<(String, String)>) -> SessionFilter {
        // With an empty title, only rules without a title keyword can match
        let apps = apps
            .into_iter()
            .filter(|(process, category)| {
                self.shown_categories.contains(category)
                    && !is_excluded(process, "", &self.ignore_rules)
                    && !matches_any(process, &self.privacy_keywords)
            })
            .map(|(process, _)| process)
            .collect();
        // Rules for some titles only become conditions. Apps ignored as a whole are already
        // dropped above
        let ignored_windows = self
            .ignore_rules
            .iter()
            .filter_map(|r| {
                let word = r.title_keyword.as_deref()?.trim();
                (!word.is_empty()).then(|| (r.process_name.trim().to_string(), word.to_string()))
            })
            .collect();
        SessionFilter {
            apps,
            ignored_windows,
            privacy_keywords: self.privacy_keywords.clone(),
        }
    }
}

/// Returns the main settings and the ids of the categories that are not deleted.
///
/// # Errors
///
/// The settings are not valid JSON, or a field has the wrong type:
/// [`Error::Json`](crate::error::Error::Json)
async fn read_main(main: &Connection) -> Result<(Settings, Vec<String>)> {
    let (data, category_ids) = main
        .call(|conn| {
            let data: Option<String> = conn
                .query_row("SELECT data FROM settings_store WHERE id = 1", [], |r| {
                    r.get(0)
                })
                .optional()
                .db()?;
            let mut stmt = conn
                .prepare("SELECT id FROM categories WHERE deleted_at IS NULL")
                .db()?;
            let ids = stmt
                .query_map([], |r| r.get(0))
                .db()?
                .collect::<rusqlite::Result<Vec<String>>>()
                .db()?;
            Ok((data, ids))
        })
        .await?;
    // No row means the settings were never saved
    let settings = match data {
        Some(d) => serde_json::from_str::<Settings>(&d)?,
        None => Settings::default(),
    };
    Ok((settings, category_ids))
}

/// Returns the profiles of `module`.
fn module_profiles(visibility: &Visibility, module: AiModule) -> &ModuleProfiles {
    match module {
        AiModule::Daily => &visibility.daily,
        AiModule::Weekly => &visibility.weekly,
        AiModule::Chat => &visibility.chat,
    }
}

/// Writes `s` as an SQL string: puts single quotes around it and doubles the single quotes
/// inside.
fn sql_text(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::sql::{from_stats_category_sql, host_rule_with_sql};
    use crate::repo::test_util::fresh_test_pool;
    use crate::storage::DbPool;

    /// 主库：Code 和 Code.exe 在「编程」的同一个分组里；secret 不计入统计；Signal 隐私；
    /// Chrome 在「浏览」，但 github.com 的网站规则算「编程」。
    /// 忽略 Terminal 里标题有 download 的窗口、整个 Zoom；Slack 的规则关键词是空白。
    /// 日报有三套配置：「工作」选了「编程」，「隐私」选了「隐私」和「编程」，「其他」选了「其他」。
    /// 2026-07-08 每个应用一条活动，标题用来认出它；Terminal 那条已忽略。
    async fn seeded() -> DbPool {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|c| {
                c.execute_batch(
                    r#"INSERT INTO app_groups(id, display_name, category_id) VALUES
                         ('g-code', 'Visual Studio Code', 'code'),
                         ('g-secret', 'Secret', 'hidden'),
                         ('g-signal', 'Signal', 'private'),
                         ('g-chrome', 'Chrome', 'browse');
                     INSERT INTO app_group_members(process_name, group_id) VALUES
                         ('Code.exe', 'g-code'), ('Code', 'g-code'),
                         ('secret.exe', 'g-secret'), ('Signal', 'g-signal'),
                         ('Chrome', 'g-chrome');
                     INSERT INTO site_rules(host, category_id, updated_at) VALUES
                         ('github.com', 'code', '2026-07-08T00:00:00Z');
                     INSERT INTO activities(started_at, ended_at, duration_secs, local_date,
                         local_hour, process_name, window_title, category_id, url_host, excluded)
                     VALUES
                         ('2026-07-08T09:00:00+09:00', '2026-07-08T09:10:00+09:00', 600,
                          '2026-07-08', 9, 'Code', 'main.rs', 'other', NULL, 0),
                         ('2026-07-08T09:10:00+09:00', '2026-07-08T09:20:00+09:00', 600,
                          '2026-07-08', 9, 'secret.exe', 'secret', 'other', NULL, 0),
                         ('2026-07-08T09:20:00+09:00', '2026-07-08T09:30:00+09:00', 600,
                          '2026-07-08', 9, 'Signal', 'chat', 'other', NULL, 0),
                         ('2026-07-08T09:30:00+09:00', '2026-07-08T09:40:00+09:00', 600,
                          '2026-07-08', 9, 'notepad', 'note', 'other', NULL, 0),
                         ('2026-07-08T09:40:00+09:00', '2026-07-08T09:50:00+09:00', 600,
                          '2026-07-08', 9, 'Chrome', 'GitHub', 'other', 'github.com', 0),
                         ('2026-07-08T09:50:00+09:00', '2026-07-08T10:00:00+09:00', 600,
                          '2026-07-08', 9, 'Chrome', 'News', 'other', 'news.com', 0),
                         ('2026-07-08T10:00:00+09:00', '2026-07-08T10:10:00+09:00', 600,
                          '2026-07-08', 10, 'Terminal', 'npm download', 'other', NULL, 1);"#,
                )
                .db()
            })
            .await
            .unwrap();
        set(
            &pool,
            "$.ignoreRules",
            r#"[{"processName": "Terminal", "titleKeyword": " download "},
                {"processName": "Zoom"},
                {"processName": "Slack", "titleKeyword": "  "}]"#,
        )
        .await;
        set(
            &pool,
            "$.ai",
            r#"{"visibility": {"daily": {"profiles": {
                 "p_work": {"name": "工作", "categories": ["code"]},
                 "p_private": {"name": "隐私", "categories": ["private", "code"]},
                 "p_other": {"name": "其他", "categories": ["other"]}}}}}"#,
        )
        .await;
        pool
    }

    /// 把主库设置里 `path` 那一项改成 `value`，`value` 是 JSON。
    async fn set(pool: &DbPool, path: &'static str, value: &'static str) {
        pool.0
            .call(move |c| {
                c.execute(
                    "UPDATE settings_store SET data = json_set(data, ?1, json(?2)) WHERE id = 1",
                    [path, value],
                )
                .db()
            })
            .await
            .unwrap();
    }

    /// 再加一条 2026-07-08 的活动。
    async fn add(pool: &DbPool, process: &'static str, title: &'static str) {
        pool.0
            .call(move |c| {
                c.execute(
                    "INSERT INTO activities(started_at, ended_at, duration_secs, local_date,
                         local_hour, process_name, window_title, category_id)
                     VALUES ('2026-07-08T11:00:00+09:00', '2026-07-08T11:10:00+09:00', 600,
                         '2026-07-08', 11, ?1, ?2, 'other')",
                    [process, title],
                )
                .db()
            })
            .await
            .unwrap();
    }

    async fn load(pool: &DbPool, module: AiModule, profile: Option<&str>) -> AiDataPolicy {
        AiDataPolicy::load(&pool.0, module, profile).await.unwrap()
    }

    /// 用 `main_filter` 查主库，按加进去的顺序返回能看到的活动标题。
    async fn visible(pool: &DbPool, policy: &AiDataPolicy) -> Vec<String> {
        let sql = format!(
            "{} SELECT a.window_title {} WHERE {} ORDER BY a.id",
            host_rule_with_sql("activities"),
            from_stats_category_sql("activities"),
            policy.main_filter()
        );
        pool.0
            .call(move |c| {
                let mut stmt = c.prepare(&sql).db()?;
                let titles = stmt
                    .query_map(["2026-07-08", "2026-07-08"], |r| r.get(0))
                    .db()?
                    .collect::<rusqlite::Result<Vec<String>>>()
                    .db()?;
                Ok(titles)
            })
            .await
            .unwrap()
    }

    /// 按主库设置能看到的活动：「不计入统计」「隐私」和已忽略的都看不到。
    const MAIN: [&str; 4] = ["main.rs", "note", "GitHub", "News"];

    /// 记忆库里的进程和它的分类，跟 `seeded` 里的分组一致；没分组的算「其他」。
    fn apps(processes: &[&str]) -> Vec<(String, String)> {
        processes
            .iter()
            .map(|&p| {
                let category = match p {
                    "Code" | "Code.exe" => "code",
                    "secret.exe" => "hidden",
                    "Signal" => "private",
                    "Chrome" => "browse",
                    _ => "other",
                };
                (p.to_string(), category.to_string())
            })
            .collect()
    }

    #[tokio::test]
    async fn no_profile_follows_the_main_settings() {
        let pool = seeded().await;
        let v = load(&pool, AiModule::Daily, None).await;
        assert_eq!(visible(&pool, &v).await, MAIN);
        let f = v.memory_filter(apps(&["Code", "secret.exe", "Signal", "notepad"]));
        assert_eq!(f.apps, ["Code", "notepad"]);
    }

    #[tokio::test]
    async fn a_profile_shows_only_its_categories() {
        let pool = seeded().await;
        let v = load(&pool, AiModule::Daily, Some("p_work")).await;
        // 网站规则按主库设置：github.com 算「编程」
        assert_eq!(visible(&pool, &v).await, ["main.rs", "GitHub"]);
        // 记忆库没有网站，Chrome 按它自己的分类「浏览」算
        let f = v.memory_filter(apps(&["Code", "Chrome", "notepad"]));
        assert_eq!(f.apps, ["Code"]);
    }

    #[tokio::test]
    async fn the_private_category_cannot_be_shown() {
        let pool = seeded().await;
        let v = load(&pool, AiModule::Daily, Some("p_private")).await;
        assert_eq!(visible(&pool, &v).await, ["main.rs", "GitHub"]);
        let f = v.memory_filter(apps(&["Code", "Signal"]));
        assert_eq!(f.apps, ["Code"]);
    }

    #[tokio::test]
    async fn a_missing_profile_follows_the_main_settings() {
        let pool = seeded().await;
        // p_work 是日报的配置，聊天里没有
        for (module, profile) in [(AiModule::Chat, "p_work"), (AiModule::Daily, "p_gone")] {
            let v = load(&pool, module, Some(profile)).await;
            assert_eq!(visible(&pool, &v).await, MAIN, "{module:?} {profile}");
        }
    }

    #[tokio::test]
    async fn ignore_rules_apply_with_a_profile() {
        let pool = seeded().await;
        // notepad、Terminal、Zoom 都算「其他」；Terminal 那条活动已忽略，Zoom 整个忽略
        let v = load(&pool, AiModule::Daily, Some("p_other")).await;
        assert_eq!(visible(&pool, &v).await, ["note"]);
        let f = v.memory_filter(apps(&["notepad", "Terminal", "Zoom"]));
        assert_eq!(f.apps, ["notepad", "Terminal"]);
        assert_eq!(
            f.ignored_windows,
            [("Terminal".to_string(), "download".to_string())]
        );
    }

    #[tokio::test]
    async fn activities_with_privacy_keywords_are_dropped() {
        let pool = seeded().await;
        set(&pool, "$.privacyAppKeywords", r#"["工资", " BANK ", ""]"#).await;
        add(&pool, "Code", "工资表.xlsx").await;
        add(&pool, "MyBank", "home").await;
        let v = load(&pool, AiModule::Daily, None).await;
        // 标题或进程名里有关键词，整条不给；英文不分大小写，空白关键词不算
        assert_eq!(visible(&pool, &v).await, MAIN);
        let f = v.memory_filter(apps(&["Code", "MyBank"]));
        assert_eq!(f.apps, ["Code"]);
        assert_eq!(f.privacy_keywords, ["工资", "BANK"]);
    }

    #[tokio::test]
    async fn a_keyword_with_a_quote_is_escaped() {
        let pool = seeded().await;
        set(&pool, "$.privacyAppKeywords", r#"["it's"]"#).await;
        add(&pool, "notepad", "it's mine").await;
        let v = load(&pool, AiModule::Daily, None).await;
        assert_eq!(visible(&pool, &v).await, MAIN);
    }

    #[tokio::test]
    async fn main_filter_and_memory_filter_agree() {
        let pool = seeded().await;
        // 这几个应用在主库里各有一条不带网站的活动
        let activities = [
            ("Code", "main.rs"),
            ("secret.exe", "secret"),
            ("Signal", "chat"),
            ("notepad", "note"),
        ];
        for profile in [None, Some("p_work"), Some("p_other")] {
            let v = load(&pool, AiModule::Daily, profile).await;
            let main = visible(&pool, &v).await;
            let f = v.memory_filter(apps(&activities.map(|(p, _)| p)));
            for (process, title) in activities {
                assert_eq!(
                    main.iter().any(|t| t == title),
                    f.apps.iter().any(|p| p == process),
                    "{profile:?} {process}"
                );
            }
        }
    }

    #[tokio::test]
    async fn the_filter_drops_hidden_private_and_wholly_ignored_apps() {
        let v = load(&seeded().await, AiModule::Chat, None).await;
        let f = v.memory_filter(apps(&[
            "secret.exe",
            "Signal",
            "Zoom",
            "Terminal",
            "Slack",
            "Code.exe",
            "unknown.exe",
            "",
        ]));
        assert_eq!(f.apps, ["Terminal", "Slack", "Code.exe", "unknown.exe", ""]);
        // 只忽略某些标题的规则写成条件，关键词去掉前后空白；空白关键词的规则什么都不忽略
        assert_eq!(
            f.ignored_windows,
            [("Terminal".to_string(), "download".to_string())]
        );
    }

    #[tokio::test]
    async fn settings_that_cannot_be_read_are_an_error() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|c| {
                c.execute_batch("UPDATE settings_store SET data = 'not json' WHERE id = 1")
                    .db()
            })
            .await
            .unwrap();
        assert!(AiDataPolicy::load(&pool.0, AiModule::Daily, None)
            .await
            .is_err());
    }
}
