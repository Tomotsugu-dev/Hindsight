//! Reports queried by start and end date: each category's time per day, and the top apps. Used by
//! the Daily, Weekly and Monthly pages.

use chrono::{Duration, NaiveDate};
use rusqlite::ToSql;

use crate::error::Result;
use crate::repo::sql::{from_stats_category_sql, host_rule_with_sql};
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

use super::time::{parse_stored_time, split_by_date};
use super::{AppTotals, AppUsage, CategoryTime, DaySummary, DeviceFilter};

/// Each category's time on each day of a date range, for drawing a bar chart with one bar per day:
/// one entry per day from `from` to `to`, each bar split by category, with empty `segments` on days
/// with no activity. A record that runs past midnight is split at midnight.
pub async fn day_category_time(
    pool: &DbPool,
    from: NaiveDate,
    to: NaiveDate,
    device: DeviceFilter,
) -> Result<Vec<DaySummary>> {
    let from_str = from.format("%Y-%m-%d").to_string();
    let to_str = to.format("%Y-%m-%d").to_string();

    // <local_date, category, total seconds> for activities that do not cross midnight.
    // <category, started_at, ended_at> for activities that cross midnight.
    let (rows, crossing) = pool
        .0
        .call(move |conn| {
            // As in day_hours: the category the time counts toward, website rules applied,
            // filtering out deleted categories and the "Hidden" category
            // Activities that do not cross midnight (end date equals local_date)
            let sql = format!(
                "{with}
                 SELECT a.local_date,
                        COALESCE(c.id, 'other') AS cat,
                        SUM(a.duration_secs) AS total
                 {from}
                 WHERE a.local_date >= ? AND a.local_date <= ? {device}
                   -- Keep activities that end on their local_date; e.g. 23:50 to 00:10 is handled below.
                   AND substr(a.ended_at, 1, 10) = a.local_date
                   AND c.id IS NOT 'hidden'
                   AND a.excluded = 0
                 GROUP BY a.local_date, cat",
                with = host_rule_with_sql(),
                from = from_stats_category_sql("activities"),
                device = device.sql_clause(),
            );
            let mut params: Vec<&dyn ToSql> = vec![&from_str, &to_str, &from_str, &to_str];
            if let Some(extra) = device.sql_param() {
                params.push(extra);
            }
            let mut stmt = conn.prepare(&sql).db()?;
            let rows = stmt
                .query_map(params.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;

            // Activities that cross midnight (end date not equal to local_date)
            let sql = format!(
                "{with}
                 SELECT COALESCE(c.id, 'other') AS cat, a.started_at, a.ended_at
                 {from}
                 WHERE a.local_date >= ? AND a.local_date <= ? {device}
                   AND substr(a.ended_at, 1, 10) <> a.local_date
                   AND c.id IS NOT 'hidden'
                   AND a.excluded = 0",
                with = host_rule_with_sql(),
                from = from_stats_category_sql("activities"),
                device = device.sql_clause(),
            );

            // Activities that cross midnight may start as early as the day before `from`.
            let crossing_from_str = (from - Duration::days(1)).format("%Y-%m-%d").to_string();
            let mut params: Vec<&dyn ToSql> =
                vec![&crossing_from_str, &to_str, &crossing_from_str, &to_str];
            if let Some(extra) = device.sql_param() {
                params.push(extra);
            }
            let mut stmt = conn.prepare(&sql).db()?;
            let crossing = stmt
                .query_map(params.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok((rows, crossing))
        })
        .await?;

    let mut buckets: std::collections::HashMap<String, std::collections::HashMap<String, u64>> =
        std::collections::HashMap::new();
    for (date, cat, secs) in rows {
        if secs <= 0 {
            continue;
        }
        *buckets.entry(date).or_default().entry(cat).or_insert(0) += secs as u64;
    }
    for (cat, started, ended) in crossing {
        let (Some(s), Some(e)) = (parse_stored_time(&started), parse_stored_time(&ended)) else {
            continue;
        };
        for (date, s, e) in split_by_date(s, e) {
            if date < from || date > to {
                continue;
            }
            *buckets
                .entry(date.format("%Y-%m-%d").to_string())
                .or_default()
                .entry(cat.clone())
                .or_insert(0) += (e - s).num_seconds() as u64;
        }
    }

    let mut out = Vec::new();
    let mut cur_date = from;
    while cur_date <= to {
        let key = cur_date.format("%Y-%m-%d").to_string();
        let mut segs: Vec<CategoryTime> = buckets
            .remove(&key)
            .unwrap_or_default()
            .into_iter()
            .map(|(category_id, secs)| CategoryTime { category_id, secs })
            .collect();
        // Descending: see the comment on the same pattern above
        segs.sort_by_key(|s| std::cmp::Reverse(s.secs));
        out.push(DaySummary {
            date: key,
            segments: segs,
        });
        cur_date += Duration::days(1);
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

    let (rows, crossing) = pool
        .0
        .call(move |conn| {
            // `MIN(process_name)` only makes sure the same process name is picked every time, for
            // the icon lookup.
            // Activities that do not cross midnight (end date equals local_date)
            let sql = format!(
                "{with},
                 per_process AS MATERIALIZED (
                     SELECT a.process_name, a.url_host, SUM(a.duration_secs) AS secs
                     FROM activities a
                     WHERE a.local_date >= ? AND a.local_date <= ? {device}
                       AND substr(a.ended_at, 1, 10) = a.local_date
                       AND a.excluded = 0
                     -- Keep the host so later website rules can classify each subtotal.
                     -- A browser process can visit sites in different categories.
                     GROUP BY a.process_name, a.url_host
                 )
                 SELECT COALESCE(g.id, a.process_name)                  AS group_id,
                        COALESCE(g.display_name, a.process_name)        AS display,
                        COALESCE(ac.id, 'other')                        AS app_cat,
                        COALESCE(c.id, 'other')                         AS cat,
                        MIN(a.process_name)                             AS icon_process,
                        SUM(a.secs)                                     AS total
                 {from}
                 WHERE c.id IS NOT 'hidden'
                 GROUP BY group_id, display, app_cat, cat",
                with = host_rule_with_sql(),
                from = from_stats_category_sql("per_process"),
                device = device.sql_clause(),
            );
            let mut params: Vec<&dyn ToSql> = vec![&from_str, &to_str, &from_str, &to_str];
            if let Some(extra) = device.sql_param() {
                params.push(extra);
            }
            let mut stmt = conn.prepare(&sql).db()?;
            let rows = stmt
                .query_map(params.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;

            // Activities that cross midnight (end date not equal to local_date)
            let sql = format!(
                "{with}
                 SELECT COALESCE(g.id, a.process_name)                  AS group_id,
                        COALESCE(g.display_name, a.process_name)        AS display,
                        COALESCE(ac.id, 'other')                        AS app_cat,
                        COALESCE(c.id, 'other')                         AS cat,
                        a.process_name, a.started_at, a.ended_at
                 {from}
                 WHERE a.local_date >= ? AND a.local_date <= ? {device}
                   AND substr(a.ended_at, 1, 10) <> a.local_date
                   AND c.id IS NOT 'hidden'
                   AND a.excluded = 0",
                with = host_rule_with_sql(),
                from = from_stats_category_sql("activities"),
                device = device.sql_clause(),
            );
            // Activities that cross midnight may start as early as the day before `from`.
            let crossing_from_str = (from - Duration::days(1)).format("%Y-%m-%d").to_string();
            let mut params: Vec<&dyn ToSql> =
                vec![&crossing_from_str, &to_str, &crossing_from_str, &to_str];
            if let Some(extra) = device.sql_param() {
                params.push(extra);
            }
            let mut stmt = conn.prepare(&sql).db()?;
            let crossing = stmt
                .query_map(params.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok((rows, crossing))
        })
        .await?;

    let mut totals = AppTotals::default();
    for (group_id, display_name, app_cat, cat, icon_process, secs) in rows {
        if secs > 0 {
            totals.add(
                group_id,
                display_name,
                app_cat,
                cat,
                icon_process,
                secs as u64,
            );
        }
    }
    for (group_id, display_name, app_cat, cat, process, started, ended) in crossing {
        let (Some(s), Some(e)) = (parse_stored_time(&started), parse_stored_time(&ended)) else {
            continue;
        };
        let secs: i64 = split_by_date(s, e)
            .into_iter()
            .filter(|(date, ..)| *date >= from && *date <= to)
            .map(|(_, s, e)| (e - s).num_seconds())
            .sum();
        if secs <= 0 {
            continue;
        }
        totals.add(group_id, display_name, app_cat, cat, process, secs as u64);
    }

    let mut apps: Vec<(u64, AppUsage)> = totals.finish();
    let most_first = |a: &(u64, AppUsage), b: &(u64, AppUsage)| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.display_name.cmp(&b.1.display_name))
            .then_with(|| a.1.group_id.cmp(&b.1.group_id))
    };
    // Keep the top `limit` apps by seconds, then sort only those
    let limit = limit as usize;
    if apps.len() > limit {
        apps.select_nth_unstable_by(limit, most_first);
        apps.truncate(limit);
    }
    apps.sort_by(most_first);
    Ok(apps
        .into_iter()
        .map(|(_, app)| app)
        .filter(|app| app.minutes > 0)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::reports::test_seed::{
        insert_activity, insert_session_with_times, insert_visit, local_time, seed_group,
        seed_solo_group,
    };
    use crate::repo::reports::time::{month_range, week_range};
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};
    use chrono::Local;

    fn code_secs(day: &DaySummary) -> u64 {
        day.segments
            .iter()
            .filter(|s| s.category_id == "code")
            .map(|s| s.secs)
            .sum()
    }

    /// 测 [`top_apps`] 跨设备 SUM：
    /// - `DeviceFilter::All` 合并两端时长到 1 行
    /// - `DeviceFilter::Only(...)` 只算指定设备
    ///
    /// 钉死「今日总览」上方设备 chip 切换的数字一致性。
    #[tokio::test]
    async fn top_apps_aggregates_correctly_across_devices() {
        let pool = fresh_test_pool().await;
        let day = Local::now().date_naive();
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
        assert_eq!(all[0].display_name, "Code");
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
        let day = Local::now().date_naive();
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
        assert_eq!(rows[0].display_name, "Visual Studio Code");
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
        let day = Local::now().date_naive();
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
        assert_eq!(rows[0].display_name, "Editor");
    }

    /// 测 [`day_category_time`]（本周）：今天的 DaySummary 应 SUM 多设备 (All) 或单设备 (Only) 时长。
    #[tokio::test]
    async fn day_category_time_aggregates_cross_device() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();
        let (from, to) = week_range(Local::now().date_naive(), 0);

        insert_activity(&pool, TEST_SELF_ID, &today_str, "Code", 300).await; // 5 min self
        insert_activity(&pool, "device-win", &today_str, "Code", 180).await; // 3 min win
        seed_solo_group(&pool, "Code", "code").await;

        let all = day_category_time(&pool, from, to, DeviceFilter::All)
            .await
            .unwrap();
        let today_all = all.iter().find(|d| d.date == today_str).unwrap();
        let code_all: u64 = today_all
            .segments
            .iter()
            .filter(|s| s.category_id == "code")
            .map(|s| s.secs)
            .sum();
        assert_eq!(code_all, 480, "All 视角 today 应 5+3 = 8 分钟 code");

        let only_self = day_category_time(&pool, from, to, DeviceFilter::Only(TEST_SELF_ID.into()))
            .await
            .unwrap();
        let today_self = only_self.iter().find(|d| d.date == today_str).unwrap();
        let code_self: u64 = today_self
            .segments
            .iter()
            .filter(|s| s.category_id == "code")
            .map(|s| s.secs)
            .sum();
        assert_eq!(code_self, 300, "Only self 视角 today 应 5 分钟");
    }

    /// 跨午夜的记录按日期拆开：10-01 23:50 → 10-02 00:10，两天各算 600 秒。
    #[tokio::test]
    async fn day_category_time_splits_record_crossing_midnight() {
        let pool = fresh_test_pool().await;
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            "2026-10-01",
            "Code",
            local_time(10, 1, 23, 50),
            local_time(10, 2, 0, 10),
        )
        .await;
        seed_solo_group(&pool, "Code", "code").await;

        let from = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let days = day_category_time(&pool, from, from + Duration::days(1), DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(days.len(), 2);
        assert_eq!(code_secs(&days[0]), 600, "10-01 只算 23:50 到午夜");
        assert_eq!(code_secs(&days[1]), 600, "10-02 算午夜到 00:10");
    }

    /// 跨午夜的记录只计入范围内的日期：范围前一天开始的，凌晨那部分算进范围第一天；
    /// 范围最后一天开始的，次日那部分不在范围内。
    #[tokio::test]
    async fn day_category_time_keeps_only_dates_in_range_for_record_crossing_midnight() {
        let pool = fresh_test_pool().await;
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            "2026-09-30",
            "Code",
            local_time(9, 30, 23, 50),
            local_time(10, 1, 0, 10),
        )
        .await;
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            "2026-10-02",
            "Code",
            local_time(10, 2, 23, 50),
            local_time(10, 3, 0, 10),
        )
        .await;
        seed_solo_group(&pool, "Code", "code").await;

        let from = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let days = day_category_time(&pool, from, from + Duration::days(1), DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(days.len(), 2, "结果只有范围内的两天");
        assert_eq!(
            code_secs(&days[0]),
            600,
            "10-01 算前一晚拖过来的 00:00 到 00:10"
        );
        assert_eq!(code_secs(&days[1]), 600, "10-02 只算 23:50 到午夜");
    }

    /// 测 [`top_apps`]（本月）：top N 按总时长降序。
    #[tokio::test]
    async fn top_apps_top_n_correct() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();
        let (from, to) = month_range(Local::now().date_naive(), 0);

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
        assert_eq!(apps[0].display_name, "Code");
        assert_eq!(apps[0].minutes, 5);
        assert_eq!(apps[1].display_name, "Chrome");
        assert_eq!(apps[1].minutes, 3);
        assert_eq!(apps[2].display_name, "Slack");
        assert_eq!(apps[2].minutes, 1);

        // limit 钉死
        let top_2 = top_apps(&pool, from, to, 2, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(top_2.len(), 2);
        assert_eq!(top_2[0].display_name, "Code");
        assert_eq!(top_2[1].display_name, "Chrome");
    }

    /// 测 [`top_apps`]（本周）：同一应用跨多天求和成一行，范围外（上周）的量不掺入。
    #[tokio::test]
    async fn top_apps_sums_across_days_and_excludes_prev_week() {
        let pool = fresh_test_pool().await;
        let (monday, sunday) = week_range(Local::now().date_naive(), 0);
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
        assert_eq!(apps[0].display_name, "Code");
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
        let (first, last) = month_range(Local::now().date_naive(), 0);
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
                    let code_secs: u64 = d
                        .segments
                        .iter()
                        .filter(|s| s.category_id == "code")
                        .map(|s| s.secs)
                        .sum();
                    assert_eq!(code_secs, if i == 0 { 300 } else { 600 });
                }
                _ => assert!(d.segments.is_empty(), "{} 不该有数据", d.date),
            }
        }
    }

    /// 日统计的排行：10-01 23:50 → 10-02 00:10 的 Code，两天各算 10 分钟。
    #[tokio::test]
    async fn top_apps_splits_record_crossing_midnight() {
        let pool = fresh_test_pool().await;
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            "2026-10-01",
            "Code",
            local_time(10, 1, 23, 50),
            local_time(10, 2, 0, 10),
        )
        .await;
        seed_solo_group(&pool, "Code", "code").await;

        let oct1 = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let oct2 = oct1 + Duration::days(1);
        let first = top_apps(&pool, oct1, oct1, 50, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].minutes, 10, "10-01 只算 23:50 到午夜");
        let next = top_apps(&pool, oct2, oct2, 50, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(next.len(), 1, "10-02 应该有前一晚拖过来的 Code");
        assert_eq!(next[0].minutes, 10);
    }

    /// 范围两头跨午夜的记录只算范围内的部分：09-30 23:30 → 10-01 00:10 算 10 分钟，
    /// 10-02 23:50 → 10-03 00:20 也算 10 分钟。
    #[tokio::test]
    async fn top_apps_counts_only_the_part_in_range() {
        let pool = fresh_test_pool().await;
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            "2026-09-30",
            "Code",
            local_time(9, 30, 23, 30),
            local_time(10, 1, 0, 10),
        )
        .await;
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            "2026-10-02",
            "Code",
            local_time(10, 2, 23, 50),
            local_time(10, 3, 0, 20),
        )
        .await;
        seed_solo_group(&pool, "Code", "code").await;

        let from = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let apps = top_apps(&pool, from, from + Duration::days(1), 50, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].minutes, 20, "两头各 10 分钟");
    }

    /// 跨午夜那段加回去以后才排名次：10-02 当天 Chrome 15 分钟、Code 10 分钟，Code 还有前一晚
    /// 23:50 到 00:10 拖过来的 10 分钟，合计 20 分钟，只取第一名时应该是 Code。
    #[tokio::test]
    async fn top_apps_ranks_after_adding_the_part_after_midnight() {
        let pool = fresh_test_pool().await;
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            "2026-10-02",
            "Chrome",
            local_time(10, 2, 10, 0),
            local_time(10, 2, 10, 15),
        )
        .await;
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            "2026-10-02",
            "Code",
            local_time(10, 2, 9, 0),
            local_time(10, 2, 9, 10),
        )
        .await;
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            "2026-10-01",
            "Code",
            local_time(10, 1, 23, 50),
            local_time(10, 2, 0, 10),
        )
        .await;
        seed_solo_group(&pool, "Chrome", "browse").await;
        seed_solo_group(&pool, "Code", "code").await;

        let oct2 = NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
        let top = top_apps(&pool, oct2, oct2, 1, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].display_name, "Code");
        assert_eq!(top[0].minutes, 20);
    }

    /// 两个分组显示名相同、但没合并：排行里是两行，不按名字合成一行。
    #[tokio::test]
    async fn top_apps_lists_unmerged_groups_with_the_same_name_separately() {
        let pool = fresh_test_pool().await;
        let day = Local::now().date_naive();
        let today = day.format("%Y-%m-%d").to_string();
        insert_activity(&pool, TEST_SELF_ID, &today, "Notes", 300).await;
        insert_activity(&pool, "device-win", &today, "notes.exe", 180).await;
        seed_group(&pool, "Notes", "Notes", "code", &["Notes"]).await;
        seed_group(&pool, "notes.exe", "Notes", "code", &["notes.exe"]).await;

        let apps = top_apps(&pool, day, day, 50, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(apps.len(), 2, "没合并的两个组应该是两行");
        assert_eq!(apps[0].minutes, 5);
        assert_eq!(apps[1].minutes, 3);
        assert!(apps.iter().all(|a| a.display_name == "Notes"));
        let ids: Vec<&str> = apps.iter().map(|a| a.group_id.as_str()).collect();
        assert_eq!(ids, vec!["Notes", "notes.exe"]);
    }

    // —— 网站规则（ADR-0013）——

    /// 一天里 Chrome（「浏览」）的四段会话：bilibili.com 10 分、live.bilibili.com 5 分、
    /// github.com 20 分、没读到地址栏 5 分，共 40 分。
    async fn seed_chrome_day(pool: &DbPool, chrome_category: &str) -> NaiveDate {
        let day = Local::now().date_naive();
        let today = day.format("%Y-%m-%d").to_string();
        seed_solo_group(pool, "Chrome", chrome_category).await;
        insert_visit(pool, &today, "Chrome", Some("bilibili.com"), 600).await;
        insert_visit(pool, &today, "Chrome", Some("live.bilibili.com"), 300).await;
        insert_visit(pool, &today, "Chrome", Some("github.com"), 1200).await;
        insert_visit(pool, &today, "Chrome", None, 300).await;
        day
    }

    /// 某一天各分类的秒数，按分类名排好，方便断言。
    async fn category_secs(pool: &DbPool, day: NaiveDate) -> Vec<(String, u64)> {
        let mut segs: Vec<(String, u64)> = day_category_time(pool, day, day, DeviceFilter::All)
            .await
            .unwrap()
            .remove(0)
            .segments
            .into_iter()
            .map(|s| (s.category_id, s.secs))
            .collect();
        segs.sort();
        segs
    }

    fn secs(pairs: &[(&str, u64)]) -> Vec<(String, u64)> {
        pairs.iter().map(|(c, s)| (c.to_string(), *s)).collect()
    }

    /// 设规则只是把网站的时间从浏览器的分类挪到规则的分类，子域名跟着母域名，总时长不变；
    /// 应用排行里 Chrome 还是一行、时长不变、标的是它自己的分类，各分类时长加起来等于总时长。
    #[tokio::test]
    async fn site_rule_moves_time_without_changing_totals() {
        let pool = fresh_test_pool().await;
        let day = seed_chrome_day(&pool, "browse").await;
        assert_eq!(category_secs(&pool, day).await, secs(&[("browse", 2400)]));

        crate::repo::site_rules::set(&pool, "bilibili.com", "video")
            .await
            .unwrap();

        assert_eq!(
            category_secs(&pool, day).await,
            secs(&[("browse", 1500), ("video", 900)])
        );
        let apps = top_apps(&pool, day, day, 50, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(
            (apps[0].display_name.as_str(), apps[0].minutes),
            ("Chrome", 40)
        );
        assert_eq!(
            apps[0].category_id, "browse",
            "应用一行标的是应用自己的分类"
        );
        let by_category: Vec<(&str, u64)> = apps[0]
            .by_category
            .iter()
            .map(|c| (c.category_id.as_str(), c.secs))
            .collect();
        assert_eq!(by_category, vec![("browse", 1500), ("video", 900)]);
    }

    /// 浏览器在「隐藏」时，有规则的网站也不计。
    #[tokio::test]
    async fn hidden_browser_ignores_site_rules() {
        let pool = fresh_test_pool().await;
        let day = seed_chrome_day(&pool, "hidden").await;
        crate::repo::site_rules::set(&pool, "bilibili.com", "video")
            .await
            .unwrap();

        assert_eq!(category_secs(&pool, day).await, Vec::new());
        assert!(top_apps(&pool, day, day, 50, DeviceFilter::All)
            .await
            .unwrap()
            .is_empty());
    }

    /// 网站设成「隐藏」只藏这个网站（连同子域名），浏览器的其余时间照算。
    #[tokio::test]
    async fn hidden_site_only_hides_that_site() {
        let pool = fresh_test_pool().await;
        let day = seed_chrome_day(&pool, "browse").await;
        crate::repo::site_rules::set(&pool, "bilibili.com", "hidden")
            .await
            .unwrap();

        assert_eq!(category_secs(&pool, day).await, secs(&[("browse", 1500)]));
        let apps = top_apps(&pool, day, day, 50, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(apps[0].minutes, 25, "藏掉的网站不计，Chrome 那一行相应减少");
    }

    /// 规则指向的分类已被删除（比如同步先后顺序造成的），按没有规则算，回到浏览器的分类。
    #[tokio::test]
    async fn rule_with_deleted_category_falls_back_to_browser() {
        let pool = fresh_test_pool().await;
        let day = seed_chrome_day(&pool, "browse").await;
        pool.0
            .call(|conn| {
                conn.execute_batch(
                    "INSERT INTO site_rules(host, category_id, updated_at)
                       VALUES ('bilibili.com', 'game', '2026-10-07T00:00:00Z');
                     UPDATE categories SET deleted_at = '2026-10-07T00:00:00Z' WHERE id = 'game';",
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();

        assert_eq!(category_secs(&pool, day).await, secs(&[("browse", 2400)]));
    }

    /// 只限某个浏览器的规则，统计时不套用。
    #[tokio::test]
    async fn browser_specific_rule_is_not_applied() {
        let pool = fresh_test_pool().await;
        let day = seed_chrome_day(&pool, "browse").await;
        pool.0
            .call(|conn| {
                conn.execute(
                    "INSERT INTO site_rules(host, browser, category_id, updated_at)
                     VALUES ('bilibili.com', 'Chrome', 'video', '2026-10-07T00:00:00Z')",
                    [],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();

        assert_eq!(category_secs(&pool, day).await, secs(&[("browse", 2400)]));
    }
}
