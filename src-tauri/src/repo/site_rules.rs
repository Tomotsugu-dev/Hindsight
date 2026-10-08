//! Website classification (ADR-0013): the user assigns a website to a category, e.g.
//! `bilibili.com` to Media, and statistics then count the time spent on that website in
//! a browser toward that category.
//!
//! This module lists websites, and sets and removes rules. Rules apply to all browsers
//! and all devices.

use std::collections::HashMap;

use chrono::{Duration, NaiveDate};
use rusqlite::OptionalExtension;
use serde::Serialize;

use crate::error::{Error, Result};
use crate::repo::sql::matching_rule_host_sql;
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};

/// One row of the website classification page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteRow {
    pub host: String,
    /// Time in the last 30 days, today included
    pub minutes_30d: u32,
    pub minutes_total: u32,
    /// The category this website counts toward, from its own rule or a parent
    /// domain's rule; `None` when no rule applies
    pub category_id: Option<String>,
    /// The parent domain whose rule the category is inherited from; the page shows
    /// it as "inherited from bilibili.com". `None` when the category comes from the
    /// website's own rule, or no rule applies
    pub follows: Option<String>,
}

/// Websites for the website classification page: each one's time and the category it
/// counts toward, most used first.
pub async fn list(pool: &DbPool, today: NaiveDate) -> Result<Vec<SiteRow>> {
    let from = (today - Duration::days(29)).format("%Y-%m-%d").to_string();
    let to = today.format("%Y-%m-%d").to_string();
    // Time per domain
    let usage_sql = "SELECT url_host,
                            SUM(CASE WHEN local_date BETWEEN ?1 AND ?2
                                     THEN duration_secs ELSE 0 END),
                            SUM(duration_secs)
                       FROM activities
                      WHERE url_host IS NOT NULL AND excluded = 0
                      GROUP BY url_host";
    // The rule for each domain: domains seen in sessions, plus domains that only have a rule
    let rules_sql = format!(
        "WITH RECURSIVE {}
         SELECT host, rule_host, category_id FROM host_rule",
        matching_rule_host_sql(
            "SELECT url_host FROM activities WHERE url_host IS NOT NULL AND excluded = 0
             UNION
             SELECT host FROM site_rules
              WHERE browser = '' AND device = '' AND deleted_at IS NULL"
        ),
    );

    let (usage, rules) = pool
        .0
        .call(move |conn| {
            let usage = conn
                .prepare(usage_sql)
                .db()?
                .query_map(rusqlite::params![from, to], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            let rules = conn
                .prepare(&rules_sql)
                .db()?
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok((usage, rules))
        })
        .await?;

    let mut rows: HashMap<String, SiteRow> = HashMap::new();
    for (host, secs_30d, secs_total) in usage {
        let row = rows
            .entry(host.clone())
            .or_insert_with(|| SiteRow::new(host));
        row.minutes_30d = to_minutes(secs_30d);
        row.minutes_total = to_minutes(secs_total);
    }
    // Each row of `rules`: `host` is the website; `rule_host` is the domain of the rule
    // that applies to it, the website itself or a parent domain; `category_id` is that
    // rule's category
    for (host, rule_host, category_id) in rules {
        let row = rows
            .entry(host.clone())
            .or_insert_with(|| SiteRow::new(host.clone()));
        row.category_id = Some(category_id);
        // A parent domain's rule applies: record which parent domain
        if rule_host != host {
            row.follows = Some(rule_host);
        }
    }

    let mut rows: Vec<SiteRow> = rows.into_values().collect();
    rows.sort_by(|a, b| {
        b.minutes_30d
            .cmp(&a.minutes_30d)
            .then(b.minutes_total.cmp(&a.minutes_total))
            .then(a.host.cmp(&b.host))
    });
    Ok(rows)
}

impl SiteRow {
    fn new(host: String) -> Self {
        Self {
            host,
            minutes_30d: 0,
            minutes_total: 0,
            category_id: None,
            follows: None,
        }
    }
}

/// Assigns website `host` to category `category_id`: creates the rule if there is none,
/// changes its category otherwise, and restores a removed rule.
///
/// Writes nothing, `updated_at` included, when the category is unchanged: sync keeps the
/// latest change, so refreshing the time without changing anything would overwrite a real
/// change made on another device.
pub async fn set(pool: &DbPool, host: &str, category_id: &str) -> Result<()> {
    let host = normalize_host(host)?;
    let cat = category_id.trim().to_string();
    if cat.is_empty() {
        return Err(Error::InvalidInput("category id must not be empty"));
    }

    // Reject a missing or deleted category: the database would store the rule anyway,
    // and the rule would have no effect in statistics
    let c = cat.clone();
    let cat_exists = pool
        .0
        .call(move |conn| {
            conn.query_row(
                "SELECT 1 FROM categories WHERE id = ?1 AND deleted_at IS NULL",
                rusqlite::params![c],
                |_| Ok(()),
            )
            .optional()
            .db()
        })
        .await?
        .is_some();
    if !cat_exists {
        return Err(Error::InvalidInput(
            "category does not exist or was deleted",
        ));
    }

    let now = utc_now_rfc3339();
    pool.0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO site_rules(host, category_id, updated_at)
                 VALUES(?1, ?2, ?3)
                 ON CONFLICT(host, browser, device) DO UPDATE
                    SET category_id = excluded.category_id,
                        updated_at = excluded.updated_at,
                        deleted_at = NULL
                  WHERE site_rules.category_id IS NOT excluded.category_id
                     OR site_rules.deleted_at IS NOT NULL",
                rusqlite::params![host, cat, now],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Removes the website's own rule by setting `deleted_at`. The row stays so that sync can
/// carry the removal to other devices. Does nothing when there is no rule.
pub async fn remove(pool: &DbPool, host: &str) -> Result<()> {
    let host = normalize_host(host)?;
    let now = utc_now_rfc3339();
    pool.0
        .call(move |conn| {
            conn.execute(
                "UPDATE site_rules SET deleted_at = ?2, updated_at = ?2
                 WHERE host = ?1 AND browser = '' AND device = '' AND deleted_at IS NULL",
                rusqlite::params![host, now],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Sessions store domains in lowercase (`browser_url::host_for_stats`), so rules do too,
/// or they would not match.
fn normalize_host(host: &str) -> Result<String> {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() {
        return Err(Error::InvalidInput("host must not be empty"));
    }
    Ok(host)
}

fn to_minutes(secs: i64) -> u32 {
    (secs.max(0) as f64 / 60.0).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};

    fn day(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    const TODAY: &str = "2026-10-07";

    /// 插一段浏览器会话：哪天、哪个域名、多少秒、是否被忽略。
    async fn visit(pool: &DbPool, date: &str, host: &str, secs: i64, excluded: bool) {
        let (date, host) = (date.to_string(), host.to_string());
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activities(
                        started_at, ended_at, duration_secs, local_date, local_hour,
                        process_name, window_title, category_id, device_id, updated_at,
                        origin, url_host, excluded
                     ) VALUES(
                        ?1 || 'T10:00:00Z', ?1 || 'T10:30:00Z', ?2, ?1, 10,
                        'Google Chrome', '', 'browse', ?3, ?1 || 'T10:30:00Z',
                        'local', ?4, ?5
                     )",
                    rusqlite::params![date, secs, TEST_SELF_ID, host, excluded as i64],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn row(pool: &DbPool, host: &str) -> Option<SiteRow> {
        list(pool, day(TODAY))
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.host == host)
    }

    async fn stored(pool: &DbPool, host: &str) -> Option<(String, String, Option<String>)> {
        let host = host.to_string();
        pool.0
            .call(move |conn| {
                conn.query_row(
                    "SELECT category_id, updated_at, deleted_at FROM site_rules
                     WHERE host = ?1 AND browser = '' AND device = ''",
                    rusqlite::params![host],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .db()
            })
            .await
            .unwrap()
    }

    /// 设置、改分类、取消、取消后重新设置；同一个分类再设一次不动 updated_at。
    #[tokio::test]
    async fn set_change_remove_and_set_again() {
        let pool = fresh_test_pool().await;
        visit(&pool, TODAY, "bilibili.com", 600, false).await;

        set(&pool, "bilibili.com", "video").await.unwrap();
        assert_eq!(
            row(&pool, "bilibili.com")
                .await
                .unwrap()
                .category_id
                .as_deref(),
            Some("video")
        );

        set(&pool, "bilibili.com", "code").await.unwrap();
        let (cat, updated, _) = stored(&pool, "bilibili.com").await.unwrap();
        assert_eq!(cat, "code");

        set(&pool, "bilibili.com", "code").await.unwrap();
        let (_, updated_again, _) = stored(&pool, "bilibili.com").await.unwrap();
        assert_eq!(updated, updated_again, "没改东西的写入不能动 updated_at");

        remove(&pool, "bilibili.com").await.unwrap();
        let (_, _, deleted) = stored(&pool, "bilibili.com").await.unwrap();
        assert!(deleted.is_some(), "取消是软删，行还在");
        assert_eq!(row(&pool, "bilibili.com").await.unwrap().category_id, None);

        remove(&pool, "bilibili.com").await.unwrap();

        set(&pool, "bilibili.com", "video").await.unwrap();
        let (cat, _, deleted) = stored(&pool, "bilibili.com").await.unwrap();
        assert_eq!(
            (cat.as_str(), deleted),
            ("video", None),
            "取消后重新设置要恢复"
        );
    }

    /// 子域名跟随母域名的规则；长得像但不是子域名的不算；子域名自己有规则时不再跟随，
    /// 它下面的域名改跟随它（往上找到的第一条）。
    #[tokio::test]
    async fn follows_parent_rule_but_not_lookalike() {
        let pool = fresh_test_pool().await;
        for host in [
            "bilibili.com",
            "live.bilibili.com",
            "a.live.bilibili.com",
            "notbilibili.com",
        ] {
            visit(&pool, TODAY, host, 600, false).await;
        }
        set(&pool, "bilibili.com", "video").await.unwrap();

        let parent = row(&pool, "bilibili.com").await.unwrap();
        assert_eq!(
            (parent.category_id.as_deref(), parent.follows),
            (Some("video"), None)
        );
        let live = row(&pool, "live.bilibili.com").await.unwrap();
        assert_eq!(
            (live.category_id.as_deref(), live.follows.as_deref()),
            (Some("video"), Some("bilibili.com")),
            "跟随母域名时，分类是母域名规则的分类"
        );
        let lookalike = row(&pool, "notbilibili.com").await.unwrap();
        assert_eq!((lookalike.category_id, lookalike.follows), (None, None));

        set(&pool, "live.bilibili.com", "code").await.unwrap();
        let live = row(&pool, "live.bilibili.com").await.unwrap();
        assert_eq!(
            (live.category_id.as_deref(), live.follows),
            (Some("code"), None)
        );
        let deeper = row(&pool, "a.live.bilibili.com").await.unwrap();
        assert_eq!(
            (deeper.category_id.as_deref(), deeper.follows.as_deref()),
            (Some("code"), Some("live.bilibili.com"))
        );
    }

    /// 只限某个浏览器的规则不参与：列表里既不算这个网站自己的规则，也不算跟随。
    #[tokio::test]
    async fn browser_specific_rules_are_ignored() {
        let pool = fresh_test_pool().await;
        visit(&pool, TODAY, "bilibili.com", 600, false).await;
        visit(&pool, TODAY, "live.bilibili.com", 600, false).await;
        pool.0
            .call(|conn| {
                conn.execute(
                    "INSERT INTO site_rules(host, browser, category_id, updated_at)
                     VALUES ('bilibili.com', 'Safari', 'video', '2026-10-07T00:00:00Z')",
                    [],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();

        let parent = row(&pool, "bilibili.com").await.unwrap();
        assert_eq!((parent.category_id, parent.follows), (None, None));
        assert_eq!(row(&pool, "live.bilibili.com").await.unwrap().follows, None);
    }

    /// 近 30 天和全部时长分开算，被忽略的会话不算；先按近 30 天、再按全部时长排；
    /// 只有规则、没有会话的网站也列出来。
    #[tokio::test]
    async fn minutes_and_order() {
        let pool = fresh_test_pool().await;
        visit(&pool, TODAY, "a.com", 600, false).await;
        visit(&pool, "2026-08-28", "a.com", 600, false).await; // 40 天前
        visit(&pool, TODAY, "a.com", 6000, true).await; // 被忽略
        visit(&pool, "2026-09-08", "b.com", 1200, false).await; // 正好第 30 天
        visit(&pool, "2026-07-01", "c.com", 3000, false).await;
        set(&pool, "d.com", "video").await.unwrap();

        let rows = list(&pool, day(TODAY)).await.unwrap();
        let got: Vec<(&str, u32, u32)> = rows
            .iter()
            .map(|r| (r.host.as_str(), r.minutes_30d, r.minutes_total))
            .collect();
        assert_eq!(
            got,
            vec![
                ("b.com", 20, 20),
                ("a.com", 10, 20),
                ("c.com", 0, 50),
                ("d.com", 0, 0),
            ]
        );
    }

    /// 删分类时，指向它的规则跟着软删，网站回到没有规则的状态。
    #[tokio::test]
    async fn deleting_category_removes_its_rules() {
        let pool = fresh_test_pool().await;
        visit(&pool, TODAY, "github.com", 600, false).await;
        set(&pool, "github.com", "code").await.unwrap();

        crate::repo::categories::delete(&pool, "code")
            .await
            .unwrap();

        let (_, _, deleted) = stored(&pool, "github.com").await.unwrap();
        assert!(deleted.is_some());
        assert_eq!(row(&pool, "github.com").await.unwrap().category_id, None);
    }

    /// 域名去掉首尾空格、转小写再存；空域名、空分类、不存在的分类都拒绝。
    #[tokio::test]
    async fn normalizes_host_and_rejects_bad_input() {
        let pool = fresh_test_pool().await;
        set(&pool, "  Bilibili.COM ", "video").await.unwrap();
        assert!(stored(&pool, "bilibili.com").await.is_some());

        for (host, cat) in [
            ("  ", "video"),
            ("x.com", ""),
            ("x.com", "no-such-category"),
        ] {
            let err = set(&pool, host, cat).await.unwrap_err();
            assert!(
                matches!(err, Error::InvalidInput(_)),
                "{host:?} {cat:?}: {err:?}"
            );
        }
        assert!(matches!(
            remove(&pool, " ").await.unwrap_err(),
            Error::InvalidInput(_)
        ));
    }
}
