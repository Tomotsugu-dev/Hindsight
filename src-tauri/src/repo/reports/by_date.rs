//! Reports queried by start and end date: each category's time per day, and the top apps. Used by
//! the Daily, Weekly and Monthly pages.

use chrono::{Duration, NaiveDate};
use rusqlite::ToSql;

use crate::error::Result;
use crate::repo::sql::FROM_ACTIVITY_GROUP_CATEGORY;
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

use super::{AppUsage, DaySummary, DeviceFilter, HourSegment};

/// Each category's time on each day of a date range, for drawing a bar chart with one bar per day:
/// one entry per day from `from` to `to`, each bar split by category, with empty `segments` on days
/// with no activity. A record counts toward the day it started, even if it runs past midnight.
pub async fn day_category_time(
    pool: &DbPool,
    from: NaiveDate,
    to: NaiveDate,
    device: DeviceFilter,
) -> Result<Vec<DaySummary>> {
    let from_str = from.format("%Y-%m-%d").to_string();
    let to_str = to.format("%Y-%m-%d").to_string();

    let rows: Vec<(String, String, i64)> = pool
        .0
        .call(move |conn| {
            // As in day_hours: get the category through group → category, filtering out deleted
            // categories and the "Hidden" category
            let sql = format!(
                "SELECT a.local_date,
                        COALESCE(c.id, 'other') AS cat,
                        SUM(a.duration_secs) AS total
                 {FROM_ACTIVITY_GROUP_CATEGORY}
                 WHERE a.local_date >= ? AND a.local_date <= ? {}
                   AND g.category_id IS NOT 'hidden'
                   AND a.excluded = 0
                 GROUP BY a.local_date, cat",
                device.sql_clause()
            );
            let mut params: Vec<&dyn ToSql> = Vec::new();
            params.push(&from_str);
            params.push(&to_str);
            if let Some(extra) = device.extra_param() {
                params.push(extra);
            }
            let mut stmt = conn.prepare(&sql).db()?;
            let it = stmt
                .query_map(params.as_slice(), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                })
                .db()?;
            let mut out = Vec::new();
            for r in it {
                out.push(r.db()?);
            }
            Ok(out)
        })
        .await?;

    let mut buckets: std::collections::HashMap<String, std::collections::HashMap<String, u64>> =
        std::collections::HashMap::new();
    for (date, cat, secs) in rows {
        // Drop only rows with secs == 0. Pieces under 30s round to 0 minutes, but their secs must
        // stay, or the frontend's totals and daily averages (summed from secs) won't match the
        // SQL SUM of top apps
        if secs <= 0 {
            continue;
        }
        buckets.entry(date).or_default().insert(cat, secs as u64);
    }

    let mut out = Vec::new();
    let mut cur = from;
    // TODO: `cur` (the current day) needs a clearer name, or a change for accurate day splitting.
    while cur <= to {
        let key = cur.format("%Y-%m-%d").to_string();
        let mut segs: Vec<HourSegment> = buckets
            .remove(&key)
            .unwrap_or_default()
            .into_iter()
            .map(|(category_id, secs)| HourSegment {
                category_id,
                minutes: (secs as f64 / 60.0).round() as u32,
                secs,
            })
            .collect();
        // Descending: see the comment on the same pattern above
        segs.sort_by_key(|s| std::cmp::Reverse(s.minutes));
        out.push(DaySummary {
            date: key,
            segments: segs,
        });
        cur += Duration::days(1);
    }

    Ok(out)
}

/// The top apps in a date range: each app's total time, most first, at most `limit` apps; apps
/// under 30 seconds are left out.
pub async fn top_apps(
    pool: &DbPool,
    from: NaiveDate,
    to: NaiveDate,
    limit: u32,
    device: DeviceFilter,
) -> Result<Vec<AppUsage>> {
    let from_str = from.format("%Y-%m-%d").to_string();
    let to_str = to.format("%Y-%m-%d").to_string();

    let rows: Vec<(String, String, String, i64)> = pool
        .0
        .call(move |conn| {
            // Merge by display name, not by group: two groups with the same display name are the
            // same app to the user. Rows with different categories stay separate.
            // `MIN(process_name)` only makes sure the same process name is picked every time, for
            // the icon lookup.
            let sql = format!(
                "SELECT COALESCE(g.display_name, a.process_name)        AS display,
                        COALESCE(c.id, 'other')                         AS cat,
                        MIN(a.process_name)                             AS icon_process,
                        SUM(a.duration_secs)                            AS total
                 {FROM_ACTIVITY_GROUP_CATEGORY}
                 WHERE a.local_date >= ? AND a.local_date <= ? {}
                   AND g.category_id IS NOT 'hidden'
                   AND a.excluded = 0
                 GROUP BY COALESCE(g.display_name, a.process_name), COALESCE(c.id, 'other')
                 ORDER BY total DESC
                 LIMIT ?",
                device.sql_clause()
            );
            let mut params: Vec<&dyn ToSql> = Vec::new();
            params.push(&from_str);
            params.push(&to_str);
            if let Some(extra) = device.extra_param() {
                params.push(extra);
            }
            params.push(&limit);
            let mut stmt = conn.prepare(&sql).db()?;
            let it = stmt
                .query_map(params.as_slice(), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                })
                .db()?;
            let mut out = Vec::new();
            for r in it {
                out.push(r.db()?);
            }
            Ok(out)
        })
        .await?;

    Ok(rows
        .into_iter()
        .map(|(process, cat, icon_process, secs)| AppUsage {
            process,
            category_id: cat,
            minutes: (secs as f64 / 60.0).round() as u32,
            icon_process,
        })
        .filter(|a| a.minutes > 0)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::reports::test_seed::{insert_activity, seed_solo_group};
    use crate::repo::reports::time::{day_date, month_range, week_range};
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};
    use chrono::Local;

    /// 测 [`top_apps`] 跨设备 SUM：
    /// - `DeviceFilter::All` 合并两端时长到 1 行
    /// - `DeviceFilter::Only(...)` 只算指定设备
    ///
    /// 钉死「今日总览」上方设备 chip 切换的数字一致性。
    #[tokio::test]
    async fn top_apps_aggregates_correctly_across_devices() {
        let pool = fresh_test_pool().await;
        let day = day_date(0);
        let today = day.format("%Y-%m-%d").to_string();

        // 同一进程 "Code" 在 self（5 分钟）和 device-win（3 分钟）各贡献时长
        insert_activity(&pool, TEST_SELF_ID, &today, "Code", 300).await;
        insert_activity(&pool, "device-win", &today, "Code", 180).await;
        // 简单 1:1 组：组 id = process_name = "Code"，category=code
        seed_solo_group(&pool, "Code", "code").await;

        // All: 5 + 3 = 8 分钟
        let all = top_apps(&pool, day, day, 50, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(all.len(), 1, "All 视角应只有一行");
        assert_eq!(all[0].process, "Code");
        assert_eq!(all[0].minutes, 8);
        assert_eq!(all[0].category_id, "code");

        // Only self: 只 5 分钟
        let only_self = top_apps(&pool, day, day, 50, DeviceFilter::Only(TEST_SELF_ID.into()))
            .await
            .unwrap();
        assert_eq!(only_self.len(), 1);
        assert_eq!(only_self[0].minutes, 5);

        // Only win: 只 3 分钟
        let only_win = top_apps(&pool, day, day, 50, DeviceFilter::Only("device-win".into()))
            .await
            .unwrap();
        assert_eq!(only_win.len(), 1);
        assert_eq!(only_win[0].minutes, 3);
    }

    /// 测 [`top_apps`] 跨 OS 别名合并：mac="Code" + Win="Code.exe" 共享
    /// canonical 组 "Visual Studio Code" → All 视角下应合并成 1 行。
    ///
    /// 钉死："两台机器各显示 5min / 3min" 而不是合并的 "8min" 这条 bug 重现。
    #[tokio::test]
    async fn top_apps_merges_cross_os_aliases_into_one_row() {
        let pool = fresh_test_pool().await;
        let day = day_date(0);
        let today = day.format("%Y-%m-%d").to_string();

        // mac 视角的 "Code" 5 分钟 + Win 视角的 "Code.exe" 3 分钟
        insert_activity(&pool, TEST_SELF_ID, &today, "Code", 300).await;
        insert_activity(&pool, "device-win", &today, "Code.exe", 180).await;

        // 一个 canonical 组，两个成员都指向它
        pool.0
            .call(|conn| {
                let now = "2026-05-15T10:00:00Z";
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('Visual Studio Code', 'Visual Studio Code', 'code', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                for name in ["Code", "Code.exe"] {
                    conn.execute(
                        "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                         VALUES(?1, 'Visual Studio Code', ?2, NULL)",
                        rusqlite::params![name, now],
                    )
                    .db()?;
                }
                Ok(())
            })
            .await
            .unwrap();

        let rows = top_apps(&pool, day, day, 50, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "cross-OS 别名应合并成一行，不是两行");
        assert_eq!(rows[0].process, "Visual Studio Code");
        assert_eq!(rows[0].minutes, 8);
        assert_eq!(rows[0].category_id, "code");
        // icon_process 是 MIN(process_name)，二选一即可
        assert!(
            rows[0].icon_process == "Code" || rows[0].icon_process == "Code.exe",
            "icon_process 应是组内某个真实成员名: got {}",
            rows[0].icon_process
        );
    }

    /// 忽略规则打标的行（excluded=1）不进报表口径——top_apps 该只剩没打标的行。
    #[tokio::test]
    async fn top_apps_skips_excluded_rows() {
        let pool = fresh_test_pool().await;
        let day = day_date(0);
        let today = day.format("%Y-%m-%d").to_string();
        insert_activity(&pool, "dev-a", &today, "Downloader", 600).await;
        insert_activity(&pool, "dev-a", &today, "Editor", 600).await;
        pool.0
            .call(|conn| {
                conn.execute(
                    "UPDATE activities SET excluded = 1 WHERE process_name = 'Downloader'",
                    [],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();

        let rows = top_apps(&pool, day, day, 50, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "excluded 行不该出现在 top_apps");
        assert_eq!(rows[0].process, "Editor");
    }

    /// 测 [`day_category_time`]（本周）：今天的 DaySummary 应 SUM 多设备 (All) 或单设备 (Only) 时长。
    #[tokio::test]
    async fn day_category_time_aggregates_cross_device() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();
        let (from, to) = week_range(0);

        insert_activity(&pool, TEST_SELF_ID, &today_str, "Code", 300).await; // 5 min self
        insert_activity(&pool, "device-win", &today_str, "Code", 180).await; // 3 min win
        seed_solo_group(&pool, "Code", "code").await;

        let all = day_category_time(&pool, from, to, DeviceFilter::All)
            .await
            .unwrap();
        let today_all = all.iter().find(|d| d.date == today_str).unwrap();
        let code_all: u32 = today_all
            .segments
            .iter()
            .filter(|s| s.category_id == "code")
            .map(|s| s.minutes)
            .sum();
        assert_eq!(code_all, 8, "All 视角 today 应 5+3 = 8 分钟 code");

        let only_self = day_category_time(&pool, from, to, DeviceFilter::Only(TEST_SELF_ID.into()))
            .await
            .unwrap();
        let today_self = only_self.iter().find(|d| d.date == today_str).unwrap();
        let code_self: u32 = today_self
            .segments
            .iter()
            .filter(|s| s.category_id == "code")
            .map(|s| s.minutes)
            .sum();
        assert_eq!(code_self, 5, "Only self 视角 today 应 5 分钟");
    }

    /// 测 [`top_apps`]（本月）：top N 按总时长降序。
    #[tokio::test]
    async fn top_apps_top_n_correct() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();
        let (from, to) = month_range(0);

        insert_activity(&pool, TEST_SELF_ID, &today_str, "Code", 300).await; // 5 min
        insert_activity(&pool, TEST_SELF_ID, &today_str, "Chrome", 180).await; // 3 min
        insert_activity(&pool, TEST_SELF_ID, &today_str, "Slack", 60).await; // 1 min
        seed_solo_group(&pool, "Code", "code").await;
        seed_solo_group(&pool, "Chrome", "browse").await;
        seed_solo_group(&pool, "Slack", "talk").await;

        let apps = top_apps(&pool, from, to, 5, DeviceFilter::All)
            .await
            .unwrap();
        assert!(apps.len() >= 3, "应至少 3 行");
        // 降序：Code (5) > Chrome (3) > Slack (1)
        assert_eq!(apps[0].process, "Code");
        assert_eq!(apps[0].minutes, 5);
        assert_eq!(apps[1].process, "Chrome");
        assert_eq!(apps[1].minutes, 3);
        assert_eq!(apps[2].process, "Slack");
        assert_eq!(apps[2].minutes, 1);

        // limit 钉死
        let top_2 = top_apps(&pool, from, to, 2, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(top_2.len(), 2);
        assert_eq!(top_2[0].process, "Code");
        assert_eq!(top_2[1].process, "Chrome");
    }

    /// 测 [`top_apps`]（本周）：同一应用跨多天求和成一行，范围外（上周）的量不掺入。
    #[tokio::test]
    async fn top_apps_sums_across_days_and_excludes_prev_week() {
        let pool = fresh_test_pool().await;
        let (monday, sunday) = week_range(0);
        let day = |off: i64| {
            (monday + Duration::days(off))
                .format("%Y-%m-%d")
                .to_string()
        };

        insert_activity(&pool, TEST_SELF_ID, &day(0), "Code", 300).await;
        insert_activity(&pool, TEST_SELF_ID, &day(2), "Code", 300).await;
        // 上周日一大段：若被算进来 minutes 会变 10+100=110，一眼可辨
        insert_activity(&pool, TEST_SELF_ID, &day(-1), "Code", 6000).await;
        seed_solo_group(&pool, "Code", "code").await;

        let apps = top_apps(&pool, monday, sunday, 50, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(apps.len(), 1, "同一应用跨天应合并成一行");
        assert_eq!(apps[0].process, "Code");
        assert_eq!(
            apps[0].minutes, 10,
            "300+300=600s=10min，上周的 6000s 不该掺入"
        );
        assert_eq!(apps[0].category_id, "code");
    }

    /// 测 [`day_category_time`]（本月）：行数 = 当月天数、按日期有序，有数据的天分类分钟正确、
    /// 无数据的天 segments 为空，上月末尾的量不掺入。
    #[tokio::test]
    async fn day_category_time_zero_fills_whole_month() {
        let pool = fresh_test_pool().await;
        let (first, last) = month_range(0);
        let n_days = ((last - first).num_days() + 1) as usize;
        let day = |off: i64| (first + Duration::days(off)).format("%Y-%m-%d").to_string();

        insert_activity(&pool, TEST_SELF_ID, &day(0), "Code", 300).await; // 5 min
        insert_activity(&pool, TEST_SELF_ID, &day(9), "Code", 600).await; // 10 min
        insert_activity(&pool, TEST_SELF_ID, &day(-1), "Code", 999).await; // 上月末，应被排除
        seed_solo_group(&pool, "Code", "code").await;

        let days = day_category_time(&pool, first, last, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(days.len(), n_days, "行数应等于当月天数");
        for (i, d) in days.iter().enumerate() {
            assert_eq!(d.date, day(i as i64), "第 {i} 行日期不符");
            match i {
                0 | 9 => {
                    let code_min: u32 = d
                        .segments
                        .iter()
                        .filter(|s| s.category_id == "code")
                        .map(|s| s.minutes)
                        .sum();
                    assert_eq!(code_min, if i == 0 { 5 } else { 10 });
                }
                _ => assert!(d.segments.is_empty(), "{} 不该有数据", d.date),
            }
        }
    }
}
