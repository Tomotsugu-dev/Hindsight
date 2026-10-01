//! Reports for a range of dates: time per category for each day, the app ranking, and wrappers
//! for the week and month ranges.

use chrono::{Datelike, Duration, Local, NaiveDate};
use rusqlite::ToSql;

use crate::error::Result;
use crate::repo::sql::FROM_ACTIVITY_GROUP_CATEGORY;
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

use super::{AppUsage, DaySummary, DeviceFilter, HourSegment};

/// Time per category for each of the 7 days of a week. `week_offset = 0` is this week (starting
/// Monday).
// TODO: Something the frontend calls should not live in `range.rs`, which is only a helper. The
// name also says nothing: it should say what the function does.
pub async fn week_days(
    pool: &DbPool,
    week_offset: i32,
    device: DeviceFilter,
) -> Result<Vec<DaySummary>> {
    let (monday, sunday) = week_range(week_offset);
    days_in_range(pool, monday, sunday, device).await
}

/// Top apps for a week (total over the 7 days, most first), merged by group.
// TODO: Same problem as `week_days`.
pub async fn week_apps(
    pool: &DbPool,
    week_offset: i32,
    limit: u32,
    device: DeviceFilter,
) -> Result<Vec<AppUsage>> {
    let (monday, sunday) = week_range(week_offset);
    apps_in_range(pool, monday, sunday, limit, device).await
}

/// Time per category for each day of a month (28–31 rows). `month_offset = 0` is this month.
// TODO: Same problem as `week_days`.
pub async fn month_days(
    pool: &DbPool,
    month_offset: i32,
    device: DeviceFilter,
) -> Result<Vec<DaySummary>> {
    let (first, last) = month_range(month_offset);
    days_in_range(pool, first, last, device).await
}

/// Top apps for a month (total over the month, most first), merged by group.
// TODO: Almost the same problem as `week_days`.
pub async fn month_apps(
    pool: &DbPool,
    month_offset: i32,
    limit: u32,
    device: DeviceFilter,
) -> Result<Vec<AppUsage>> {
    let (first, last) = month_range(month_offset);
    apps_in_range(pool, first, last, limit, device).await
}

// TODO: Add a doc comment: what this function does and how it behaves, including how it splits
// the days and what the `device` filter means.
async fn days_in_range(
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

// TODO: Add a doc comment.
async fn apps_in_range(
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
            // As in day_apps: group by display name + category (entities with the same name merge),
            // icon_process = MIN(process_name). Activities in the "Hidden" category are left out
            // entirely (not counted in top apps)
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

// TODO: Add a doc comment that says what it does, and rename the function.
pub(super) fn week_range(week_offset: i32) -> (NaiveDate, NaiveDate) {
    let today = Local::now().date_naive();
    let dow = today.weekday().num_days_from_monday() as i64;
    let monday = today - Duration::days(dow) + Duration::days(week_offset as i64 * 7);
    let sunday = monday + Duration::days(6);
    (monday, sunday)
}

// TODO: Add a doc comment that says what it does.
pub(super) fn month_range(month_offset: i32) -> (NaiveDate, NaiveDate) {
    let today = Local::now().date_naive();
    let mut year = today.year();
    let mut month = today.month() as i32 + month_offset;
    // TODO: There should already be a mature way to add a month offset to a date.
    while month <= 0 {
        month += 12;
        year -= 1;
    }
    while month > 12 {
        month -= 12;
        year += 1;
    }
    // TODO: Check whether panicking here is reasonable.
    let first = NaiveDate::from_ymd_opt(year, month as u32, 1)
        .expect("month_range: year/month must be within chrono's valid range");
    // TODO: Could be clearer: finish normalizing the year and month first (a modulo would do),
    // then write the if/else.
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1).expect("month_range: rolls over to January")
    } else {
        NaiveDate::from_ymd_opt(year, (month + 1) as u32, 1)
            .expect("month_range: month + 1 must be within 1..=12")
    };
    let last = next - Duration::days(1);
    // TODO: Name these `from` and `to`, as elsewhere in this module.
    (first, last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::reports::test_seed::{insert_activity, seed_solo_group};
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};

    /// 测 [`week_days`]：今天的 DaySummary 应 SUM 多设备 (All) 或单设备 (Only) 时长。
    #[tokio::test]
    async fn week_days_aggregates_cross_device() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();

        insert_activity(&pool, TEST_SELF_ID, &today_str, "Code", 300).await; // 5 min self
        insert_activity(&pool, "device-win", &today_str, "Code", 180).await; // 3 min win
        seed_solo_group(&pool, "Code", "code").await;

        let all = week_days(&pool, 0, DeviceFilter::All).await.unwrap();
        let today_all = all.iter().find(|d| d.date == today_str).unwrap();
        let code_all: u32 = today_all
            .segments
            .iter()
            .filter(|s| s.category_id == "code")
            .map(|s| s.minutes)
            .sum();
        assert_eq!(code_all, 8, "All 视角 today 应 5+3 = 8 分钟 code");

        let only_self = week_days(&pool, 0, DeviceFilter::Only(TEST_SELF_ID.into()))
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

    /// 测 [`month_apps`]：top N 按总时长降序。
    #[tokio::test]
    async fn month_apps_top_n_correct() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();

        insert_activity(&pool, TEST_SELF_ID, &today_str, "Code", 300).await; // 5 min
        insert_activity(&pool, TEST_SELF_ID, &today_str, "Chrome", 180).await; // 3 min
        insert_activity(&pool, TEST_SELF_ID, &today_str, "Slack", 60).await; // 1 min
        seed_solo_group(&pool, "Code", "code").await;
        seed_solo_group(&pool, "Chrome", "browse").await;
        seed_solo_group(&pool, "Slack", "talk").await;

        let apps = month_apps(&pool, 0, 5, DeviceFilter::All).await.unwrap();
        assert!(apps.len() >= 3, "应至少 3 行");
        // 降序：Code (5) > Chrome (3) > Slack (1)
        assert_eq!(apps[0].process, "Code");
        assert_eq!(apps[0].minutes, 5);
        assert_eq!(apps[1].process, "Chrome");
        assert_eq!(apps[1].minutes, 3);
        assert_eq!(apps[2].process, "Slack");
        assert_eq!(apps[2].minutes, 1);

        // limit 钉死
        let top_2 = month_apps(&pool, 0, 2, DeviceFilter::All).await.unwrap();
        assert_eq!(top_2.len(), 2);
        assert_eq!(top_2[0].process, "Code");
        assert_eq!(top_2[1].process, "Chrome");
    }

    /// 测 [`week_apps`]：同一应用跨多天求和成一行，范围外（上周）的量不掺入。
    #[tokio::test]
    async fn week_apps_sums_across_days_and_excludes_prev_week() {
        let pool = fresh_test_pool().await;
        let (monday, _) = week_range(0);
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

        let apps = week_apps(&pool, 0, 50, DeviceFilter::All).await.unwrap();
        assert_eq!(apps.len(), 1, "同一应用跨天应合并成一行");
        assert_eq!(apps[0].process, "Code");
        assert_eq!(
            apps[0].minutes, 10,
            "300+300=600s=10min，上周的 6000s 不该掺入"
        );
        assert_eq!(apps[0].category_id, "code");
    }

    /// 测 [`month_days`]：行数 = 当月天数、按日期有序，有数据的天分类分钟正确、
    /// 无数据的天 segments 为空，上月末尾的量不掺入。
    #[tokio::test]
    async fn month_days_zero_fills_whole_month() {
        let pool = fresh_test_pool().await;
        let (first, last) = month_range(0);
        let n_days = ((last - first).num_days() + 1) as usize;
        let day = |off: i64| (first + Duration::days(off)).format("%Y-%m-%d").to_string();

        insert_activity(&pool, TEST_SELF_ID, &day(0), "Code", 300).await; // 5 min
        insert_activity(&pool, TEST_SELF_ID, &day(9), "Code", 600).await; // 10 min
        insert_activity(&pool, TEST_SELF_ID, &day(-1), "Code", 999).await; // 上月末，应被排除
        seed_solo_group(&pool, "Code", "code").await;

        let days = month_days(&pool, 0, DeviceFilter::All).await.unwrap();
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
