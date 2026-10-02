//! Reports computed by splitting records at clock hours: the 24 hour bars on the Daily page, and
//! the app ranking after clicking an hour bar.

use chrono::NaiveDate;
use rusqlite::ToSql;

use crate::error::Result;
use crate::repo::sql::FROM_ACTIVITY_GROUP_CATEGORY;
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

use super::time::{parse_local, slice_by_hour};
use super::{AppUsage, CategoryTime, DeviceFilter, HourSlot};

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
            // `IS NOT` keeps activities with no group (`g.category_id` is NULL) and drops only
            // those in "Hidden".
            let sql = format!(
                "SELECT a.started_at, a.ended_at,
                        COALESCE(c.id, 'other') AS cat
                 {FROM_ACTIVITY_GROUP_CATEGORY}
                 WHERE a.local_date = ? {}
                   AND g.category_id IS NOT 'hidden'
                   AND a.excluded = 0",
                device.sql_clause()
            );
            let mut params: Vec<&dyn ToSql> = Vec::new();
            params.push(&date);
            if let Some(extra) = device.sql_param() {
                params.push(extra);
            }
            let mut stmt = conn.prepare(&sql).db()?;
            let it = stmt
                .query_map(params.as_slice(), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
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

    let mut buckets: [std::collections::HashMap<String, u64>; 24] =
        std::array::from_fn(|_| std::collections::HashMap::new());

    for (started, ended, cat) in rows {
        let (Some(s), Some(e)) = (parse_local(&started), parse_local(&ended)) else {
            continue;
        };
        if e <= s {
            continue;
        }
        for (hour, secs) in slice_by_hour(s, e) {
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

    // (display, cat, icon_process, started, ended); adding up happens after slicing
    let rows: Vec<(String, String, String, String, String)> = pool
        .0
        .call(move |conn| {
            let sql = format!(
                "SELECT COALESCE(g.display_name, a.process_name)        AS display,
                        COALESCE(c.id, 'other')                         AS cat,
                        a.process_name                                  AS icon_process,
                        a.started_at, a.ended_at
                 {FROM_ACTIVITY_GROUP_CATEGORY}
                 WHERE a.local_date = ? {}
                   AND g.category_id IS NOT 'hidden'
                   AND a.excluded = 0",
                device.sql_clause()
            );
            let mut params: Vec<&dyn ToSql> = Vec::new();
            params.push(&date);
            if let Some(extra) = device.sql_param() {
                params.push(extra);
            }
            let mut stmt = conn.prepare(&sql).db()?;
            let it = stmt
                .query_map(params.as_slice(), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
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

    // Add up the sliced seconds within the target hour by display; icon_process is the first
    // member name seen for the group
    let mut agg: std::collections::HashMap<String, (String, String, u64)> =
        std::collections::HashMap::new();
    for (display, cat, icon_process, started, ended) in rows {
        let (Some(s), Some(e)) = (parse_local(&started), parse_local(&ended)) else {
            continue;
        };
        if e <= s {
            continue;
        }
        let hour_secs: u64 = slice_by_hour(s, e)
            .into_iter()
            .filter(|(h, _)| *h as i32 == hour)
            .map(|(_, secs)| secs)
            .sum();
        if hour_secs == 0 {
            continue;
        }
        let entry = agg.entry(display).or_insert((cat, icon_process, 0));
        entry.2 += hour_secs;
    }

    let mut list: Vec<AppUsage> = agg
        .into_iter()
        .map(
            |(display_name, (category_id, icon_process, secs))| AppUsage {
                display_name,
                category_id,
                minutes: ((secs as f64 / 60.0).round() as u32),
                icon_process,
            },
        )
        .filter(|a| a.minutes > 0)
        .collect();
    list.sort_by_key(|a| std::cmp::Reverse(a.minutes));
    list.truncate(limit as usize);
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::reports::test_seed::{insert_session_with_times, seed_solo_group};
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};
    use chrono::{Duration, Local, TimeZone};

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
}
