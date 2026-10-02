//! Reports for one day: the hour bars and app ranking on the Daily page, and the app ranking
//! after clicking an hour.

use chrono::{Duration, Local};
use rusqlite::ToSql;

use crate::error::Result;
use crate::repo::sql::FROM_ACTIVITY_GROUP_CATEGORY;
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

use super::time::{parse_local, slice_by_hour};
use super::{AppUsage, DeviceFilter, HourSegment, HourSlot};

/// Time per category for each of the 24 hours of a day. `day_offset = 0` is today, -1 is
/// yesterday.
pub async fn day_hours(
    pool: &DbPool,
    day_offset: i32,
    device: DeviceFilter,
) -> Result<Vec<HourSlot>> {
    let date = (Local::now() + Duration::days(day_offset as i64))
        .format("%Y-%m-%d")
        .to_string();

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
            if let Some(extra) = device.extra_param() {
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
            let mut segs: Vec<HourSegment> = buckets[h as usize]
                .iter()
                .map(|(cat, secs)| HourSegment {
                    category_id: cat.clone(),
                    minutes: ((*secs as f64 / 60.0).round() as u32),
                    secs: *secs,
                })
                // Filter by secs, not minutes: pieces under 30s round to 0 minutes, but their
                // secs still count toward the frontend totals (the HourSegment.secs contract);
                // dropping them would not match top apps
                .filter(|s| s.secs > 0)
                .collect();
            // Descending: sort_by_key with Reverse(...)
            segs.sort_by_key(|s| std::cmp::Reverse(s.minutes));
            HourSlot {
                hour: h,
                segments: segs,
            }
        })
        .collect();

    Ok(slots)
}

/// Top apps for a day, by time, most first; processes in one group are merged into one row.
/// `limit` sets how many rows come back.
pub async fn day_apps(
    pool: &DbPool,
    day_offset: i32,
    limit: u32,
    device: DeviceFilter,
) -> Result<Vec<AppUsage>> {
    let date = (Local::now() + Duration::days(day_offset as i64))
        .format("%Y-%m-%d")
        .to_string();

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
                 WHERE a.local_date = ? {}
                   AND g.category_id IS NOT 'hidden'
                   AND a.excluded = 0
                 GROUP BY COALESCE(g.display_name, a.process_name), COALESCE(c.id, 'other')
                 ORDER BY total DESC
                 LIMIT ?",
                device.sql_clause()
            );
            let mut params: Vec<&dyn ToSql> = Vec::new();
            params.push(&date);
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
            minutes: ((secs as f64 / 60.0).round() as u32),
            icon_process,
        })
        .filter(|a| a.minutes > 0)
        .collect())
}

/// Returns the app ranking within one hour of a day, by time, most first.
///
/// - `pool`: database connection pool.
/// - `day_offset`: days from today; `0` is today, `-1` is yesterday.
/// - `hour`: the hour to query, `0`–`23`.
/// - `limit`: the most apps to return.
/// - `device`: device filter.
pub async fn day_hour_apps(
    pool: &DbPool,
    day_offset: i32,
    hour: i32,
    limit: u32,
    device: DeviceFilter,
) -> Result<Vec<AppUsage>> {
    let date = (Local::now() + Duration::days(day_offset as i64))
        .format("%Y-%m-%d")
        .to_string();

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
            if let Some(extra) = device.extra_param() {
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
        .map(|(process, (category_id, icon_process, secs))| AppUsage {
            process,
            category_id,
            minutes: ((secs as f64 / 60.0).round() as u32),
            icon_process,
        })
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
        insert_activity, insert_session_with_times, seed_solo_group,
    };
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};
    use chrono::TimeZone;

    /// 测 [`day_apps`] 跨设备 SUM：
    /// - `DeviceFilter::All` 合并两端时长到 1 行
    /// - `DeviceFilter::Only(...)` 只算指定设备
    ///
    /// 钉死「今日总览」上方设备 chip 切换的数字一致性。
    #[tokio::test]
    async fn day_apps_aggregates_correctly_across_devices() {
        let pool = fresh_test_pool().await;
        let today = Local::now().format("%Y-%m-%d").to_string();

        // 同一进程 "Code" 在 self（5 分钟）和 device-win（3 分钟）各贡献时长
        insert_activity(&pool, TEST_SELF_ID, &today, "Code", 300).await;
        insert_activity(&pool, "device-win", &today, "Code", 180).await;
        // 简单 1:1 组：组 id = process_name = "Code"，category=code
        seed_solo_group(&pool, "Code", "code").await;

        // All: 5 + 3 = 8 分钟
        let all = day_apps(&pool, 0, 50, DeviceFilter::All).await.unwrap();
        assert_eq!(all.len(), 1, "All 视角应只有一行");
        assert_eq!(all[0].process, "Code");
        assert_eq!(all[0].minutes, 8);
        assert_eq!(all[0].category_id, "code");

        // Only self: 只 5 分钟
        let only_self = day_apps(&pool, 0, 50, DeviceFilter::Only(TEST_SELF_ID.into()))
            .await
            .unwrap();
        assert_eq!(only_self.len(), 1);
        assert_eq!(only_self[0].minutes, 5);

        // Only win: 只 3 分钟
        let only_win = day_apps(&pool, 0, 50, DeviceFilter::Only("device-win".into()))
            .await
            .unwrap();
        assert_eq!(only_win.len(), 1);
        assert_eq!(only_win[0].minutes, 3);
    }

    /// 测 [`day_apps`] 跨 OS 别名合并：mac="Code" + Win="Code.exe" 共享
    /// canonical 组 "Visual Studio Code" → All 视角下应合并成 1 行。
    ///
    /// 钉死："两台机器各显示 5min / 3min" 而不是合并的 "8min" 这条 bug 重现。
    #[tokio::test]
    async fn day_apps_merges_cross_os_aliases_into_one_row() {
        let pool = fresh_test_pool().await;
        let today = Local::now().format("%Y-%m-%d").to_string();

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

        let rows = day_apps(&pool, 0, 50, DeviceFilter::All).await.unwrap();
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

    /// 忽略规则打标的行（excluded=1）不进报表口径——day_apps 该只剩没打标的行。
    #[tokio::test]
    async fn day_apps_skips_excluded_rows() {
        let pool = fresh_test_pool().await;
        let today = Local::now().format("%Y-%m-%d").to_string();
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

        let rows = day_apps(&pool, 0, 50, DeviceFilter::All).await.unwrap();
        assert_eq!(rows.len(), 1, "excluded 行不该出现在 day_apps");
        assert_eq!(rows[0].process, "Editor");
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

        let slots = day_hours(&pool, -1, DeviceFilter::All).await.unwrap();
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

        let slots = day_hours(&pool, 0, DeviceFilter::All).await.unwrap();
        assert_eq!(slots.len(), 24);

        let h10 = slots.iter().find(|s| s.hour == 10).unwrap();
        let h10_code: u32 = h10
            .segments
            .iter()
            .filter(|s| s.category_id == "code")
            .map(|s| s.minutes)
            .sum();
        assert_eq!(h10_code, 30, "10 点应有 30 分钟 code");

        let h11 = slots.iter().find(|s| s.hour == 11).unwrap();
        let h11_code: u32 = h11
            .segments
            .iter()
            .filter(|s| s.category_id == "code")
            .map(|s| s.minutes)
            .sum();
        assert_eq!(h11_code, 30, "11 点应有 30 分钟 code");

        // 其它小时不该出现 code 段
        for h in [9u8, 12, 13] {
            let slot = slots.iter().find(|s| s.hour == h).unwrap();
            assert!(
                slot.segments.iter().all(|s| s.category_id != "code"),
                "{h} 点不该出现 code 段"
            );
        }
    }

    /// 测 [`day_hour_apps`]：local_hour 过滤后只返该小时内的应用。
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

        let h10 = day_hour_apps(&pool, 0, 10, 50, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(h10.len(), 1, "hour=10 只应有 Code");
        assert_eq!(h10[0].process, "Code");

        let h11 = day_hour_apps(&pool, 0, 11, 50, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(h11.len(), 1, "hour=11 只应有 Chrome");
        assert_eq!(h11[0].process, "Chrome");
    }
}
