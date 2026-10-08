//! Reports computed by splitting records at clock hours: the 24 hour bars on the Daily page, and
//! the app ranking after clicking an hour bar.

use chrono::{Duration, Local, NaiveDate};
use rusqlite::ToSql;

use crate::error::Result;
use crate::repo::sql::{from_stats_category_sql, host_rule_with_sql};
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

use super::time::{parse_stored_time, slice_by_hour, split_by_date};
use super::{AppTotals, AppUsage, CategoryTime, DeviceFilter, HourSlot};

/// Time per category for each of the 24 hours of a day.
pub async fn day_hours(
    pool: &DbPool,
    day: NaiveDate,
    device: DeviceFilter,
) -> Result<Vec<HourSlot>> {
    let date = day.format("%Y-%m-%d").to_string();

    let rows: Vec<(String, String, String)> = pool
        .0
        .call(move |conn| {
            // Activities assigned to this day,
            // plus previous-day activities that cross midnight into it.
            let sql = format!(
                "{with}
                 SELECT a.started_at, a.ended_at,
                        COALESCE(c.id, 'other') AS cat
                 {from}
                   WHERE (a.local_date = ?
                        OR (a.local_date = ? AND substr(a.ended_at, 1, 10) = ?)) {device}
                   -- Keep uncategorized activities under `other`; exclude only `hidden` ones.
                   AND c.id IS NOT 'hidden'
                   AND a.excluded = 0",
                with = host_rule_with_sql(),
                from = from_stats_category_sql("activities"),
                device = device.sql_clause(),
            );
            let prev_date = (day - Duration::days(1)).format("%Y-%m-%d").to_string();
            let mut params: Vec<&dyn ToSql> = vec![&prev_date, &date, &date, &prev_date, &date];
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
                    ))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await?;

    let mut buckets: [std::collections::HashMap<String, u64>; 24] =
        std::array::from_fn(|_| std::collections::HashMap::new());

    for (started, ended, cat) in rows {
        let (Some(s), Some(e)) = (parse_stored_time(&started), parse_stored_time(&ended)) else {
            continue;
        };
        let Some((_, s, e)) = split_by_date(s, e)
            .into_iter()
            .find(|(date, ..)| *date == day)
        else {
            continue;
        };
        for (hour, secs) in slice_by_hour(s.with_timezone(&Local), e.with_timezone(&Local)) {
            *buckets[hour as usize].entry(cat.clone()).or_insert(0) += secs;
        }
    }

    let slots: Vec<HourSlot> = (0u8..24)
        .map(|h| {
            // No longer capped at 60: with devices combined, an hour can exceed 60 minutes; the
            // frontend scales the Y axis by device count (max = 60 × deviceCount)
            let mut segs: Vec<CategoryTime> = buckets[h as usize]
                .iter()
                .map(|(cat, secs)| CategoryTime {
                    category_id: cat.clone(),
                    secs: *secs,
                })
                .filter(|s| s.secs > 0)
                .collect();
            // Descending: sort_by_key with Reverse(...)
            segs.sort_by_key(|s| std::cmp::Reverse(s.secs));
            HourSlot {
                hour: h,
                segments: segs,
            }
        })
        .collect();

    Ok(slots)
}

/// Returns the app ranking within one hour of a day, by time, most first.
///
/// - `pool`: database connection pool.
/// - `day`: the day to query, in NaiveDate format.
/// - `hour`: the hour to query, `0`–`23`.
/// - `limit`: the most apps to return.
/// - `device`: device filter.
pub async fn day_hour_apps(
    pool: &DbPool,
    day: NaiveDate,
    hour: i32,
    limit: u32,
    device: DeviceFilter,
) -> Result<Vec<AppUsage>> {
    let date = day.format("%Y-%m-%d").to_string();

    // (group id, display, app_cat, cat, icon_process, started, ended); adding up happens after
    // slicing
    let rows = pool
        .0
        .call(move |conn| {
            // Activities assigned to this day,
            // plus previous-day activities that cross midnight into it.
            let sql = format!(
                "{with}
                 SELECT COALESCE(g.id, a.process_name)                  AS group_id,
                        COALESCE(g.display_name, a.process_name)        AS display,
                        COALESCE(ac.id, 'other')                        AS app_cat,
                        COALESCE(c.id, 'other')                         AS cat,
                        a.process_name                                  AS icon_process,
                        a.started_at, a.ended_at
                 {from}
                 WHERE (a.local_date = ?
                        OR (a.local_date = ? AND substr(a.ended_at, 1, 10) = ?)) {device}
                   AND c.id IS NOT 'hidden'
                   AND a.excluded = 0",
                with = host_rule_with_sql(),
                from = from_stats_category_sql("activities"),
                device = device.sql_clause(),
            );
            let prev_date = (day - Duration::days(1)).format("%Y-%m-%d").to_string();
            let mut params: Vec<&dyn ToSql> = vec![&prev_date, &date, &date, &prev_date, &date];
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
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await?;

    // Add up the sliced seconds within the target hour by group
    let mut totals = AppTotals::default();
    for (group_id, display_name, app_cat, cat, icon_process, started, ended) in rows {
        let (Some(s), Some(e)) = (parse_stored_time(&started), parse_stored_time(&ended)) else {
            continue;
        };
        let Some((_, s, e)) = split_by_date(s, e)
            .into_iter()
            .find(|(date, ..)| *date == day)
        else {
            continue;
        };
        let hour_secs: u64 = slice_by_hour(s.with_timezone(&Local), e.with_timezone(&Local))
            .into_iter()
            .filter(|(h, _)| *h as i32 == hour)
            .map(|(_, secs)| secs)
            .sum();
        if hour_secs == 0 {
            continue;
        }
        totals.add(
            group_id,
            display_name,
            app_cat,
            cat,
            icon_process,
            hour_secs,
        );
    }

    let mut list: Vec<AppUsage> = totals
        .finish()
        .into_iter()
        .map(|(_, app)| app)
        .filter(|a| a.minutes > 0)
        .collect();
    list.sort_by_key(|a| std::cmp::Reverse(a.minutes));
    list.truncate(limit as usize);
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::reports::test_seed::{
        insert_session_with_times, local_time, seed_group, seed_solo_group,
    };
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};
    use chrono::{Duration, Local, TimeZone};

    fn code_secs_at(slots: &[HourSlot], hour: u8) -> u64 {
        slots
            .iter()
            .find(|s| s.hour == hour)
            .unwrap()
            .segments
            .iter()
            .filter(|s| s.category_id == "code")
            .map(|s| s.secs)
            .sum()
    }

    /// 10-01 23:50 → 10-02 00:10 的 Code，给下面两个测试用。
    async fn seed_code_crossing_midnight(pool: &DbPool) {
        insert_session_with_times(
            pool,
            TEST_SELF_ID,
            "2026-10-01",
            "Code",
            local_time(10, 1, 23, 50),
            local_time(10, 2, 0, 10),
        )
        .await;
        seed_solo_group(pool, "Code", "code").await;
    }

    /// 结束时间不是合法时间文本的记录不计入 [`day_hours`]，同一天的其它记录照常计入。
    #[tokio::test]
    async fn day_hours_skips_row_with_bad_end_time() {
        let pool = fresh_test_pool().await;
        // 用昨天：坏的结束时间若被当成「现在」，这条会从昨天 10 点一直算到现在
        let yesterday = Local::now().date_naive() - Duration::days(1);
        let date = yesterday.format("%Y-%m-%d").to_string();
        let at = |h, m| {
            Local
                .from_local_datetime(&yesterday.and_hms_opt(h, m, 0).unwrap())
                .single()
                .unwrap()
        };
        insert_session_with_times(&pool, TEST_SELF_ID, &date, "Code", at(9, 0), at(9, 30)).await;
        insert_session_with_times(&pool, TEST_SELF_ID, &date, "Chrome", at(10, 0), at(10, 30))
            .await;
        pool.0
            .call(|conn| {
                conn.execute(
                    "UPDATE activities SET ended_at = 'not-a-time' WHERE process_name = 'Chrome'",
                    [],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();

        let slots = day_hours(&pool, yesterday, DeviceFilter::All)
            .await
            .unwrap();
        let total: u64 = slots.iter().flat_map(|s| &s.segments).map(|s| s.secs).sum();
        assert_eq!(
            total, 1800,
            "只应计入 Code 的 30 分钟，结束时间坏掉的 Chrome 不计"
        );
    }

    /// 测 [`day_hours`]：跨两个小时的 session 应按时钟分桶到对应 HourSlot。
    /// 10:30 → 11:30 的 1 小时 session：hour=10 / hour=11 各 30 分钟。
    #[tokio::test]
    async fn day_hours_buckets_correctly() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();
        let started = Local
            .from_local_datetime(&today.and_hms_opt(10, 30, 0).unwrap())
            .single()
            .unwrap();
        let ended = Local
            .from_local_datetime(&today.and_hms_opt(11, 30, 0).unwrap())
            .single()
            .unwrap();
        insert_session_with_times(&pool, TEST_SELF_ID, &today_str, "Code", started, ended).await;
        seed_solo_group(&pool, "Code", "code").await;

        let slots = day_hours(&pool, today, DeviceFilter::All).await.unwrap();
        assert_eq!(slots.len(), 24);

        let h10 = slots.iter().find(|s| s.hour == 10).unwrap();
        let h10_code: u64 = h10
            .segments
            .iter()
            .filter(|s| s.category_id == "code")
            .map(|s| s.secs)
            .sum();
        assert_eq!(h10_code, 1800, "10 点应有 30 分钟 code");

        let h11 = slots.iter().find(|s| s.hour == 11).unwrap();
        let h11_code: u64 = h11
            .segments
            .iter()
            .filter(|s| s.category_id == "code")
            .map(|s| s.secs)
            .sum();
        assert_eq!(h11_code, 1800, "11 点应有 30 分钟 code");

        // 其它小时不该出现 code 段
        for h in [9u8, 12, 13] {
            let slot = slots.iter().find(|s| s.hour == h).unwrap();
            assert!(
                slot.segments.iter().all(|s| s.category_id != "code"),
                "{h} 点不该出现 code 段"
            );
        }
    }

    /// 测 [`day_hour_apps`]：只返回在这一小时里有用时的应用。
    #[tokio::test]
    async fn day_hour_apps_filters_by_hour() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();

        // 10 点 30 分钟 Code
        let s10 = Local
            .from_local_datetime(&today.and_hms_opt(10, 0, 0).unwrap())
            .single()
            .unwrap();
        let e10 = s10 + Duration::minutes(30);
        insert_session_with_times(&pool, TEST_SELF_ID, &today_str, "Code", s10, e10).await;

        // 11 点 30 分钟 Chrome
        let s11 = Local
            .from_local_datetime(&today.and_hms_opt(11, 0, 0).unwrap())
            .single()
            .unwrap();
        let e11 = s11 + Duration::minutes(30);
        insert_session_with_times(&pool, TEST_SELF_ID, &today_str, "Chrome", s11, e11).await;

        seed_solo_group(&pool, "Code", "code").await;
        seed_solo_group(&pool, "Chrome", "browse").await;

        let h10 = day_hour_apps(&pool, today, 10, 50, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(h10.len(), 1, "hour=10 只应有 Code");
        assert_eq!(h10[0].display_name, "Code");

        let h11 = day_hour_apps(&pool, today, 11, 50, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(h11.len(), 1, "hour=11 只应有 Chrome");
        assert_eq!(h11[0].display_name, "Chrome");
    }

    /// 跨午夜的记录画在各自那天：10-01 的 23 点和 10-02 的 0 点各 600 秒，10-01 的 0 点没有。
    #[tokio::test]
    async fn day_hours_splits_record_crossing_midnight() {
        let pool = fresh_test_pool().await;
        seed_code_crossing_midnight(&pool).await;
        let oct1 = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();

        let first = day_hours(&pool, oct1, DeviceFilter::All).await.unwrap();
        assert_eq!(
            code_secs_at(&first, 23),
            600,
            "10-01 的 23 点算 23:50 到午夜"
        );
        assert_eq!(
            code_secs_at(&first, 0),
            0,
            "10-02 凌晨的时间不该画在 10-01 的 0 点"
        );

        let next = day_hours(&pool, oct1 + Duration::days(1), DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(code_secs_at(&next, 0), 600, "10-02 的 0 点算午夜到 00:10");
    }

    /// 点小时柱看排行时，跨午夜的记录只出现在它真正所在的那一小时。
    #[tokio::test]
    async fn day_hour_apps_splits_record_crossing_midnight() {
        let pool = fresh_test_pool().await;
        seed_code_crossing_midnight(&pool).await;
        let oct1 = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();

        let first_23 = day_hour_apps(&pool, oct1, 23, 50, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(first_23.len(), 1);
        assert_eq!(first_23[0].minutes, 10, "10-01 的 23 点：Code 10 分钟");

        let first_0 = day_hour_apps(&pool, oct1, 0, 50, DeviceFilter::All)
            .await
            .unwrap();
        assert!(first_0.is_empty(), "10-02 凌晨的时间不该算进 10-01 的 0 点");

        let next_0 = day_hour_apps(&pool, oct1 + Duration::days(1), 0, 50, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(next_0.len(), 1, "10-02 的 0 点应该有前一晚拖过来的 Code");
        assert_eq!(next_0[0].minutes, 10);
    }

    /// 两个分组显示名相同、但没合并：点小时柱看到的排行也是两行。
    #[tokio::test]
    async fn day_hour_apps_lists_unmerged_groups_with_the_same_name_separately() {
        let pool = fresh_test_pool().await;
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            "2026-10-01",
            "Notes",
            local_time(10, 1, 10, 0),
            local_time(10, 1, 10, 30),
        )
        .await;
        insert_session_with_times(
            &pool,
            "device-win",
            "2026-10-01",
            "notes.exe",
            local_time(10, 1, 10, 0),
            local_time(10, 1, 10, 20),
        )
        .await;
        seed_group(&pool, "Notes", "Notes", "code", &["Notes"]).await;
        seed_group(&pool, "notes.exe", "Notes", "code", &["notes.exe"]).await;

        let oct1 = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let apps = day_hour_apps(&pool, oct1, 10, 50, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(apps.len(), 2, "没合并的两个组应该是两行");
        assert_eq!(apps[0].minutes, 30);
        assert_eq!(apps[1].minutes, 20);
        let ids: Vec<&str> = apps.iter().map(|a| a.group_id.as_str()).collect();
        assert_eq!(ids, vec!["Notes", "notes.exe"]);
    }

    /// 网站规则在按小时的统计里同样生效：10 点那一小时，bilibili.com 的 20 分算「影音」，
    /// github.com 的 10 分照旧算「浏览」；那一小时的应用排行里 Chrome 还是一行 30 分。
    #[tokio::test]
    async fn hourly_reports_apply_site_rules() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();
        let at = |h, m| {
            Local
                .from_local_datetime(&today.and_hms_opt(h, m, 0).unwrap())
                .single()
                .unwrap()
        };
        seed_solo_group(&pool, "Chrome", "browse").await;
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            &today_str,
            "Chrome",
            at(10, 0),
            at(10, 20),
        )
        .await;
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            &today_str,
            "Chrome",
            at(10, 30),
            at(10, 40),
        )
        .await;
        let first = at(10, 0).to_rfc3339();
        pool.0
            .call(move |conn| {
                conn.execute(
                    "UPDATE activities SET url_host =
                       CASE WHEN started_at = ?1 THEN 'bilibili.com' ELSE 'github.com' END",
                    rusqlite::params![first],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
        crate::repo::site_rules::set(&pool, "bilibili.com", "video")
            .await
            .unwrap();

        let h10 = day_hours(&pool, today, DeviceFilter::All)
            .await
            .unwrap()
            .into_iter()
            .find(|s| s.hour == 10)
            .unwrap();
        let mut segs: Vec<(String, u64)> = h10
            .segments
            .into_iter()
            .map(|s| (s.category_id, s.secs))
            .collect();
        segs.sort();
        assert_eq!(
            segs,
            vec![("browse".to_string(), 600), ("video".to_string(), 1200)]
        );

        let apps = day_hour_apps(&pool, today, 10, 50, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(
            (apps[0].minutes, apps[0].category_id.as_str()),
            (30, "browse")
        );
        let by_category: Vec<(&str, u64)> = apps[0]
            .by_category
            .iter()
            .map(|c| (c.category_id.as_str(), c.secs))
            .collect();
        assert_eq!(by_category, vec![("video", 1200), ("browse", 600)]);
    }
}
