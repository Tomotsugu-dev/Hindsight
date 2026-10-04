//! Details for one app: the time bars and window titles in the drawer opened by clicking an app,
//! for a day, a week or a month.

use chrono::{Duration, Local, NaiveDate};
use rusqlite::ToSql;

use crate::error::{Error, Result};
use crate::repo::sql::FROM_MEMBER_GROUP;
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

use super::time::{parse_stored_time, slice_by_hour, split_by_date};
use super::{AppDetail, DetailBucket, DeviceFilter, TitleUsage};

/// How the detail time bars are grouped: by hour on the Daily page, by day on the Weekly and
/// Monthly pages. Each bucket holds the total seconds for one clock hour or one activity date.
#[derive(Debug, Clone, Copy)]
pub enum BucketBy {
    Hour,
    Day,
}

/// Time bars and time per window title for one app in the inclusive `[from, to]` date range.
/// `group_id` selects the app, as in [`AppUsage::group_id`](super::AppUsage::group_id).
/// `bucket_by` controls whether the time bars are grouped by hour or by activity date.
pub async fn app_range_detail(
    pool: &DbPool,
    from: NaiveDate,
    to: NaiveDate,
    group_id: String,
    device: DeviceFilter,
    bucket_by: BucketBy,
) -> Result<AppDetail> {
    if matches!(bucket_by, BucketBy::Hour) && from != to {
        return Err(Error::InvalidInputDyn(format!(
            "hour bars cover one day, got {from} to {to}"
        )));
    }
    let from_str = from.format("%Y-%m-%d").to_string();
    let to_str = to.format("%Y-%m-%d").to_string();

    let (secs_by_bucket, titles, has_browser_member) = pool
        .0
        .call(move |conn| {
            // 1) Resolve the app identifier to process names: use active group members, or the
            //    identifier itself when it represents a standalone process.
            let mut process_stmt = conn
                .prepare(&format!(
                    "SELECT gm.process_name {FROM_MEMBER_GROUP}
                      WHERE gm.group_id = ?1 AND gm.deleted_at IS NULL AND g.deleted_at IS NULL
                     UNION
                     SELECT ?1 WHERE NOT EXISTS (
                         SELECT 1 {FROM_MEMBER_GROUP}
                          WHERE gm.process_name = ?1
                            AND gm.deleted_at IS NULL AND g.deleted_at IS NULL)"
                ))
                .db()?;
            let processes = process_stmt
                .query_map([&group_id], |row| row.get::<_, String>(0))
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            let has_browser_member = processes
                .iter()
                .any(|p| crate::capture::browser_url::is_browser_app(p));
            // Read through the date index and only compare process names. The `+` keeps SQLite off the
            // process name index, which would read the app's whole history even for one day.
            let process_filter = format!(
                "+a.process_name IN ({})",
                vec!["?"; processes.len()].join(", ")
            );

            // Activities that cross midnight (end date not equal to local_date)
            let crossing_sql = format!(
                "SELECT COALESCE(a.window_title, '') AS title, a.url_host AS host,
                        a.started_at, a.ended_at
                 FROM activities a
                 WHERE a.local_date >= ? AND a.local_date <= ?
                   AND substr(a.ended_at, 1, 10) <> a.local_date
                   AND {process_filter}
                   AND a.excluded = 0
                   {}",
                device.sql_clause()
            );
            // Activities that cross midnight may start as early as the day before `from`.
            let crossing_from_str = (from - Duration::days(1)).format("%Y-%m-%d").to_string();
            let crossing_params = bind(vec![&crossing_from_str, &to_str], &processes, &device);
            let mut crossing_stmt = conn.prepare(&crossing_sql).db()?;
            let crossing_rows = crossing_stmt
                .query_map(crossing_params.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            // (title, host, the parts in range as (date, seconds))
            let crossing = crossing_rows
                .into_iter()
                .filter_map(|(title, host, started, ended)| {
                    let (s, e) = (parse_stored_time(&started)?, parse_stored_time(&ended)?);
                    let parts = split_by_date(s, e)
                        .into_iter()
                        .filter(|(date, ..)| *date >= from && *date <= to)
                        .map(|(date, s, e)| (date, (e - s).num_seconds()))
                        .collect::<Vec<_>>();
                    Some((title, host, parts))
                })
                .collect::<Vec<_>>();

            // 2) Time bars. By hour, rows can't be grouped by local_hour: it is the hour the
            //    session *started* and is not updated when the session is sealed (see the note on
            //    day_hour_apps), so a session crossing an hour lands whole in its first hour and
            //    won't match the day_hours bars. So, as there, fetch the rows and split them by
            //    clock hour with slice_by_hour in Rust. By day, local_date is still summed in SQL.
            // Sparse totals keyed by hour ("0".."23") or date ("YYYY-MM-DD"). Missing keys get
            // zero seconds when the ordered time bars are built below.
            let mut secs_by_bucket: std::collections::HashMap<String, u64> =
                std::collections::HashMap::new();
            match bucket_by {
                BucketBy::Hour => {
                    // Activities assigned to this day,
                    // plus previous-day activities that cross midnight into it.
                    let session_sql = format!(
                        "SELECT a.started_at, a.ended_at
                         FROM activities a
                         WHERE (a.local_date = ?
                                OR (a.local_date = ? AND substr(a.ended_at, 1, 10) = ?))
                           AND {process_filter}
                           AND a.excluded = 0
                           {}",
                        device.sql_clause()
                    );
                    let prev_date = (from - Duration::days(1)).format("%Y-%m-%d").to_string();
                    let session_params =
                        bind(vec![&from_str, &prev_date, &from_str], &processes, &device);
                    let mut session_stmt = conn.prepare(&session_sql).db()?;
                    let session_rows = session_stmt
                        .query_map(session_params.as_slice(), |r| {
                            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                        })
                        .db()?;
                    for row in session_rows {
                        let (started, ended) = row.db()?;
                        let (Some(s), Some(e)) =
                            (parse_stored_time(&started), parse_stored_time(&ended))
                        else {
                            continue;
                        };
                        let Some((_, s, e)) = split_by_date(s, e)
                            .into_iter()
                            .find(|(date, ..)| *date == from)
                        else {
                            continue;
                        };
                        for (hour, secs) in
                            slice_by_hour(s.with_timezone(&Local), e.with_timezone(&Local))
                        {
                            *secs_by_bucket.entry(hour.to_string()).or_insert(0) += secs;
                        }
                    }
                }
                BucketBy::Day => {
                    let day_sql = format!(
                        "SELECT a.local_date, SUM(a.duration_secs)
                         FROM activities a
                         WHERE a.local_date >= ? AND a.local_date <= ?
                           AND substr(a.ended_at, 1, 10) = a.local_date
                           AND {process_filter}
                           AND a.excluded = 0
                           {}
                         GROUP BY a.local_date",
                        device.sql_clause()
                    );
                    let day_params = bind(vec![&from_str, &to_str], &processes, &device);
                    let mut day_stmt = conn.prepare(&day_sql).db()?;
                    let day_rows = day_stmt
                        .query_map(day_params.as_slice(), |r| {
                            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as u64))
                        })
                        .db()?;
                    for row in day_rows {
                        let (date, secs) = row.db()?;
                        *secs_by_bucket.entry(date).or_insert(0) += secs;
                    }
                    for (_, _, parts) in &crossing {
                        for (date, secs) in parts {
                            *secs_by_bucket
                                .entry(date.format("%Y-%m-%d").to_string())
                                .or_insert(0) += *secs as u64;
                        }
                    }
                }
            }

            // 3) Time per window title: grouped by (window_title, url_host), with a missing title
            //    as an empty string, most first. The same title on different sites (e.g. "Home")
            //    is counted separately: they are pages of different sites.
            // A window can be untitled, title reads can fail, and synced records can omit the
            // title. Keep those records in the totals, treating NULL titles as empty strings.
            let window_title_sql = format!(
                "SELECT COALESCE(a.window_title, '') AS title, a.url_host AS host,
                        SUM(a.duration_secs) AS total
                 FROM activities a
                 WHERE a.local_date >= ? AND a.local_date <= ?
                   AND substr(a.ended_at, 1, 10) = a.local_date
                   AND {process_filter}
                   AND a.excluded = 0
                   {}
                 GROUP BY title, host",
                device.sql_clause()
            );
            let window_title_params = bind(vec![&from_str, &to_str], &processes, &device);
            let mut window_title_stmt = conn.prepare(&window_title_sql).db()?;
            let window_title_rows = window_title_stmt
                .query_map(window_title_params.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            let mut secs_by_title: std::collections::HashMap<(String, Option<String>), i64> =
                window_title_rows
                    .into_iter()
                    .map(|(title, host, secs)| ((title, host), secs))
                    .collect();
            for (title, host, parts) in crossing {
                let secs: i64 = parts.iter().map(|(_, secs)| secs).sum();
                if secs > 0 {
                    *secs_by_title.entry((title, host)).or_insert(0) += secs;
                }
            }
            let mut titles: Vec<TitleUsage> = secs_by_title
                .into_iter()
                .map(|((title, host), secs)| TitleUsage {
                    title,
                    host,
                    secs: secs.max(0) as u32,
                })
                .collect();
            titles.sort_by(|a, b| {
                b.secs
                    .cmp(&a.secs)
                    .then_with(|| a.title.cmp(&b.title))
                    .then_with(|| a.host.cmp(&b.host))
            });

            Ok((secs_by_bucket, titles, has_browser_member))
        })
        .await?;

    // 4) Spread the sparse totals into a full, ordered row of bars, empty ones as 0, ready to draw
    let buckets = match bucket_by {
        BucketBy::Hour => (0u8..24)
            .map(|hour| {
                let key = hour.to_string();
                let secs = secs_by_bucket.get(&key).copied().unwrap_or(0) as u32;
                DetailBucket { key, secs }
            })
            .collect(),
        BucketBy::Day => {
            let mut out = Vec::new();
            let mut current_date = from;
            while current_date <= to {
                let key = current_date.format("%Y-%m-%d").to_string();
                let secs = secs_by_bucket.get(&key).copied().unwrap_or(0) as u32;
                out.push(DetailBucket { key, secs });
                current_date += Duration::days(1);
            }
            out
        }
    };

    // A browser if any process in the group is one, or any record has a site
    let is_browser = has_browser_member || titles.iter().any(|t| t.host.is_some());
    Ok(AppDetail {
        buckets,
        titles,
        is_browser,
    })
}

/// Builds the bound values for an app-detail SQL query. The SQL placeholders must be ordered as
/// date values, one value for each process name, then the optional device ID.
fn bind<'a>(
    dates: Vec<&'a dyn ToSql>,
    processes: &'a [String],
    device: &'a DeviceFilter,
) -> Vec<&'a dyn ToSql> {
    let mut params = dates;
    params.extend(processes.iter().map(|p| p as &dyn ToSql));
    if let Some(device_id) = device.sql_param() {
        params.push(device_id);
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::reports::test_seed::{
        insert_activity, insert_session_titled, insert_session_with_times, local_time, seed_group,
        seed_solo_group,
    };
    use crate::repo::reports::time::{month_range, week_range};
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};
    use chrono::{Local, TimeZone};

    /// 测 [`app_range_detail`]（某一天，Hour 粒度）：
    /// - 固定 24 桶、key 按 "0".."23" 有序、无活动小时补 0
    /// - 跨小时会话按真实时钟切片分摊（10:30→11:30 应各给 10/11 点 1800s），
    ///   而不是按 local_hour 把整段挤进开始桶
    #[tokio::test]
    async fn app_range_detail_hour_buckets_split_and_zero_fill() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();

        // 9:15→9:20 = 300s（单小时内）
        let s1 = Local
            .from_local_datetime(&today.and_hms_opt(9, 15, 0).unwrap())
            .single()
            .unwrap();
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            &today_str,
            "Code",
            s1,
            s1 + Duration::minutes(5),
        )
        .await;
        // 10:30→11:30 = 跨小时，两头各 1800s
        let s2 = Local
            .from_local_datetime(&today.and_hms_opt(10, 30, 0).unwrap())
            .single()
            .unwrap();
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            &today_str,
            "Code",
            s2,
            s2 + Duration::hours(1),
        )
        .await;
        seed_solo_group(&pool, "Code", "code").await;

        let detail = app_range_detail(
            &pool,
            today,
            today,
            "Code".into(),
            DeviceFilter::All,
            BucketBy::Hour,
        )
        .await
        .unwrap();

        assert_eq!(detail.buckets.len(), 24, "小时粒度固定 24 桶");
        let keys: Vec<&str> = detail.buckets.iter().map(|b| b.key.as_str()).collect();
        let expect_keys: Vec<String> = (0u8..24).map(|h| h.to_string()).collect();
        assert_eq!(
            keys,
            expect_keys.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            "key 应为有序的 \"0\"..\"23\""
        );
        for b in &detail.buckets {
            let expect = match b.key.as_str() {
                "9" => 300,
                "10" | "11" => 1800,
                _ => 0,
            };
            assert_eq!(b.secs, expect, "hour={} 的 secs 不符", b.key);
        }
    }

    /// 日统计的应用详情：跨午夜的记录画在各自那天，10-01 的 23 点和 10-02 的 0 点各 600 秒，
    /// 10-01 的 0 点没有。
    #[tokio::test]
    async fn app_range_detail_hour_buckets_split_record_crossing_midnight() {
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

        let secs_at = |detail: &AppDetail, hour: &str| {
            detail.buckets.iter().find(|b| b.key == hour).unwrap().secs
        };
        let oct1 = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let first = app_range_detail(
            &pool,
            oct1,
            oct1,
            "Code".into(),
            DeviceFilter::All,
            BucketBy::Hour,
        )
        .await
        .unwrap();
        assert_eq!(secs_at(&first, "23"), 600, "10-01 的 23 点算 23:50 到午夜");
        assert_eq!(
            secs_at(&first, "0"),
            0,
            "10-02 凌晨的时间不该画在 10-01 的 0 点"
        );

        let oct2 = oct1 + Duration::days(1);
        let next = app_range_detail(
            &pool,
            oct2,
            oct2,
            "Code".into(),
            DeviceFilter::All,
            BucketBy::Hour,
        )
        .await
        .unwrap();
        assert_eq!(secs_at(&next, "0"), 600, "10-02 的 0 点算午夜到 00:10");
    }

    /// 按小时只算一天：`from` 和 `to` 不是同一天就报错。
    #[tokio::test]
    async fn app_range_detail_hour_rejects_more_than_one_day() {
        let pool = fresh_test_pool().await;
        let oct1 = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let result = app_range_detail(
            &pool,
            oct1,
            oct1 + Duration::days(1),
            "Code".into(),
            DeviceFilter::All,
            BucketBy::Hour,
        )
        .await;
        assert!(matches!(result, Err(Error::InvalidInputDyn(_))));
    }

    /// 周统计、月统计的应用详情：跨午夜的记录按日期拆开，10-01 和 10-02 各 600 秒。
    #[tokio::test]
    async fn app_range_detail_day_buckets_split_record_crossing_midnight() {
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
        let detail = app_range_detail(
            &pool,
            oct1,
            oct1 + Duration::days(1),
            "Code".into(),
            DeviceFilter::All,
            BucketBy::Day,
        )
        .await
        .unwrap();
        let secs: Vec<u32> = detail.buckets.iter().map(|b| b.secs).collect();
        assert_eq!(secs, vec![600, 600]);
    }

    /// 窗口标题也只算当天那段：日统计看 10-01 和 10-02，main.rs 各 600 秒。
    #[tokio::test]
    async fn app_range_detail_titles_split_record_crossing_midnight() {
        let pool = fresh_test_pool().await;
        insert_session_titled(
            &pool,
            TEST_SELF_ID,
            "2026-10-01",
            "Code",
            "main.rs",
            local_time(10, 1, 23, 50),
            local_time(10, 2, 0, 10),
        )
        .await;
        seed_solo_group(&pool, "Code", "code").await;

        let oct1 = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        for day in [oct1, oct1 + Duration::days(1)] {
            let detail = app_range_detail(
                &pool,
                day,
                day,
                "Code".into(),
                DeviceFilter::All,
                BucketBy::Hour,
            )
            .await
            .unwrap();
            assert_eq!(detail.titles.len(), 1, "{day} 应该有 main.rs");
            assert_eq!(detail.titles[0].title, "main.rs");
            assert_eq!(detail.titles[0].secs, 600, "{day} 只算当天那 10 分钟");
        }
    }

    /// 「按网站」分组的原料：titles 行带 url_host，同标题不同域名分开计
    /// （"首页"在 github 和 youtube 各算各的），无域名的老行照常返回且 host=None；
    /// 代表进程是浏览器时 is_browser=true，否则 false——前端据此决定是否分组。
    #[tokio::test]
    async fn app_range_detail_titles_carry_url_host_and_browser_flag() {
        let pool = fresh_test_pool().await;
        let day = Local::now().date_naive();
        let today = day.format("%Y-%m-%d").to_string();
        let d = today.clone();
        pool.0
            .call(move |conn| {
                let rows: [(&str, &str, Option<&str>, i64); 5] = [
                    (
                        "Google Chrome",
                        "Hindsight - GitHub",
                        Some("github.com"),
                        120,
                    ),
                    ("Google Chrome", "首页", Some("github.com"), 30),
                    ("Google Chrome", "首页", Some("youtube.com"), 45),
                    ("Google Chrome", "旧记录", None, 60),
                    ("Code", "main.rs", None, 50),
                ];
                for (i, (p, t, h, secs)) in rows.iter().enumerate() {
                    let at = format!("{d}T10:0{i}:00Z");
                    conn.execute(
                        "INSERT INTO activities(
                            started_at, ended_at, duration_secs, local_date, local_hour,
                            process_name, window_title, category_id, device_id,
                            updated_at, origin, url_host
                         ) VALUES(?1, ?1, ?2, ?3, 10, ?4, ?5, 'other', 'dev-a', ?1, 'local', ?6)",
                        rusqlite::params![at, secs, d, p, t, h],
                    )
                    .db()?;
                }
                Ok(())
            })
            .await
            .unwrap();

        let chrome = app_range_detail(
            &pool,
            day,
            day,
            "Google Chrome".into(),
            DeviceFilter::All,
            BucketBy::Hour,
        )
        .await
        .unwrap();
        assert!(chrome.is_browser, "Google Chrome 是浏览器");
        let secs_of = |t: &str, h: Option<&str>| {
            chrome
                .titles
                .iter()
                .find(|x| x.title == t && x.host.as_deref() == h)
                .map(|x| x.secs)
        };
        assert_eq!(secs_of("Hindsight - GitHub", Some("github.com")), Some(120));
        assert_eq!(
            secs_of("首页", Some("github.com")),
            Some(30),
            "同标题不同域名分开计"
        );
        assert_eq!(secs_of("首页", Some("youtube.com")), Some(45));
        assert_eq!(
            secs_of("旧记录", None),
            Some(60),
            "无域名行照常返回,host=None"
        );
        assert_eq!(chrome.titles.len(), 4, "不掺入组外应用");

        let code = app_range_detail(
            &pool,
            day,
            day,
            "Code".into(),
            DeviceFilter::All,
            BucketBy::Hour,
        )
        .await
        .unwrap();
        assert!(!code.is_browser, "Code 不是浏览器");
        assert_eq!(code.titles.len(), 1);
        assert_eq!(code.titles[0].host, None);
    }

    /// 测 [`app_range_detail`] 按分组 ID 查：时间柱与标题都聚合**整个组**（跨 OS 成员 + 跨设备），
    /// 且不掺入组外应用。titles 按用时降序、同标题跨成员合并。
    #[tokio::test]
    async fn app_range_detail_merges_all_members_of_the_group() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();

        // 组 "Visual Studio Code"：成员 mac="Code" + win="Code.exe"
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

        let s = |h: u32, m: u32| {
            Local
                .from_local_datetime(&today.and_hms_opt(h, m, 0).unwrap())
                .single()
                .unwrap()
        };
        // 本机 Code：10:00→10:05 (300s) 标题 "main.rs"
        insert_session_titled(
            &pool,
            TEST_SELF_ID,
            &today_str,
            "Code",
            "main.rs",
            s(10, 0),
            s(10, 5),
        )
        .await;
        // win 端 Code.exe：10:10→10:14 (240s) 标题 "lib.rs"；再补一段同标题 "main.rs" 60s
        insert_session_titled(
            &pool,
            "device-win",
            &today_str,
            "Code.exe",
            "lib.rs",
            s(10, 10),
            s(10, 14),
        )
        .await;
        insert_session_titled(
            &pool,
            "device-win",
            &today_str,
            "Code.exe",
            "main.rs",
            s(10, 20),
            s(10, 21),
        )
        .await;
        // 组外应用同时段活动：绝不能混进 Code 的详情
        insert_session_titled(
            &pool,
            TEST_SELF_ID,
            &today_str,
            "Random",
            "noise",
            s(10, 0),
            s(10, 30),
        )
        .await;

        // 用分组 ID 查 → mac 和 win 两个成员的量都算上
        let detail = app_range_detail(
            &pool,
            today,
            today,
            "Visual Studio Code".into(),
            DeviceFilter::All,
            BucketBy::Hour,
        )
        .await
        .unwrap();
        let h10 = detail.buckets.iter().find(|b| b.key == "10").unwrap();
        assert_eq!(h10.secs, 300 + 240 + 60, "组内两成员 10 点的量应合并");
        let total: u32 = detail.buckets.iter().map(|b| b.secs).sum();
        assert_eq!(total, 600, "组外应用(Random)不该混入");

        // titles：main.rs 跨成员合并 = 300+60 = 360 > lib.rs 240，降序
        assert_eq!(detail.titles.len(), 2, "标题只该有组内两种");
        assert_eq!(detail.titles[0].title, "main.rs");
        assert_eq!(detail.titles[0].secs, 360);
        assert_eq!(detail.titles[1].title, "lib.rs");
        assert_eq!(detail.titles[1].secs, 240);

        // 设备过滤：Only(self) 只剩本机 Code 的 300s
        let only_self = app_range_detail(
            &pool,
            today,
            today,
            "Visual Studio Code".into(),
            DeviceFilter::Only(TEST_SELF_ID.into()),
            BucketBy::Hour,
        )
        .await
        .unwrap();
        let h10_self = only_self.buckets.iter().find(|b| b.key == "10").unwrap();
        assert_eq!(h10_self.secs, 300, "Only(self) 不该带上 win 端时长");
    }

    /// 两个分组显示名相同、但没合并：按其中一个的分组 ID 查，只算这一组。
    #[tokio::test]
    async fn app_range_detail_counts_only_its_own_group_among_same_name_groups() {
        let pool = fresh_test_pool().await;
        let day = Local::now().date_naive();
        let today = day.format("%Y-%m-%d").to_string();
        insert_activity(&pool, TEST_SELF_ID, &today, "Notes", 300).await;
        insert_activity(&pool, "device-win", &today, "notes.exe", 180).await;
        seed_group(&pool, "Notes", "Notes", "code", &["Notes"]).await;
        seed_group(&pool, "notes.exe", "Notes", "code", &["notes.exe"]).await;

        let detail = app_range_detail(
            &pool,
            day,
            day,
            "notes.exe".into(),
            DeviceFilter::All,
            BucketBy::Day,
        )
        .await
        .unwrap();
        let total: u32 = detail.buckets.iter().map(|b| b.secs).sum();
        assert_eq!(total, 180, "只算 notes.exe 这一组");
    }

    /// 测 [`app_range_detail`] 无组回退：代表进程没有任何组成员记录时，
    /// 组 key 退化为 process_name 本身——只聚合同名进程，不吸入其它无组进程。
    #[tokio::test]
    async fn app_range_detail_falls_back_to_process_name_without_group() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();
        let s = |h: u32, m: u32| {
            Local
                .from_local_datetime(&today.and_hms_opt(h, m, 0).unwrap())
                .single()
                .unwrap()
        };
        // 两个都没建组（v15 backfill 前的历史数据形态）
        insert_session_titled(
            &pool,
            TEST_SELF_ID,
            &today_str,
            "Lonely",
            "doc",
            s(14, 0),
            s(14, 10),
        )
        .await;
        insert_session_titled(
            &pool,
            TEST_SELF_ID,
            &today_str,
            "OtherApp",
            "noise",
            s(14, 0),
            s(14, 5),
        )
        .await;

        let detail = app_range_detail(
            &pool,
            today,
            today,
            "Lonely".into(),
            DeviceFilter::All,
            BucketBy::Hour,
        )
        .await
        .unwrap();
        let h14 = detail.buckets.iter().find(|b| b.key == "14").unwrap();
        assert_eq!(h14.secs, 600, "无组时按 process_name 精确匹配");
        let total: u32 = detail.buckets.iter().map(|b| b.secs).sum();
        assert_eq!(total, 600, "其它无组进程不该被吸进来");
        assert_eq!(detail.titles.len(), 1);
        assert_eq!(detail.titles[0].title, "doc");
        assert_eq!(detail.titles[0].secs, 600);
    }

    /// 测 [`app_range_detail`]（本周，Day 粒度）：7 桶按日期有序、空天补 0、
    /// 范围外（上周日）的同名活动被排除。
    #[tokio::test]
    async fn app_range_detail_week_day_buckets_zero_filled_in_order() {
        let pool = fresh_test_pool().await;
        let (monday, sunday) = week_range(Local::now().date_naive(), 0);
        let day = |off: i64| {
            (monday + Duration::days(off))
                .format("%Y-%m-%d")
                .to_string()
        };

        insert_activity(&pool, TEST_SELF_ID, &day(0), "Code", 600).await;
        insert_activity(&pool, TEST_SELF_ID, &day(2), "Code", 900).await;
        // 上周日的量：若范围下界写错（>= 变 >，或 monday 计算错）会漏进来
        insert_activity(&pool, TEST_SELF_ID, &day(-1), "Code", 12345).await;
        seed_solo_group(&pool, "Code", "code").await;

        let detail = app_range_detail(
            &pool,
            monday,
            sunday,
            "Code".into(),
            DeviceFilter::All,
            BucketBy::Day,
        )
        .await
        .unwrap();
        assert_eq!(detail.buckets.len(), 7, "周详情固定 7 桶");
        for (i, b) in detail.buckets.iter().enumerate() {
            assert_eq!(b.key, day(i as i64), "第 {i} 桶的日期 key 不符");
            let expect = match i {
                0 => 600,
                2 => 900,
                _ => 0,
            };
            assert_eq!(b.secs, expect, "{} 的 secs 不符", b.key);
        }
    }

    /// 测 [`app_range_detail`]（本月，Day 粒度）：桶数 = 当月天数，逐日有序补 0。
    #[tokio::test]
    async fn app_range_detail_month_covers_whole_month() {
        let pool = fresh_test_pool().await;
        let (first, last) = month_range(Local::now().date_naive(), 0);
        let n_days = ((last - first).num_days() + 1) as usize;
        let day = |off: i64| (first + Duration::days(off)).format("%Y-%m-%d").to_string();

        insert_activity(&pool, TEST_SELF_ID, &day(0), "Code", 300).await;
        insert_activity(&pool, TEST_SELF_ID, &day(14), "Code", 450).await;
        seed_solo_group(&pool, "Code", "code").await;

        let detail = app_range_detail(
            &pool,
            first,
            last,
            "Code".into(),
            DeviceFilter::All,
            BucketBy::Day,
        )
        .await
        .unwrap();
        assert_eq!(detail.buckets.len(), n_days, "桶数应等于当月天数(28~31)");
        for (i, b) in detail.buckets.iter().enumerate() {
            assert_eq!(b.key, day(i as i64));
            let expect = match i {
                0 => 300,
                14 => 450,
                _ => 0,
            };
            assert_eq!(b.secs, expect, "{} 的 secs 不符", b.key);
        }
    }
}
