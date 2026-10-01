//! Details for one app: the time bars and window titles in the drawer opened by clicking an app,
//! for a day, a week or a month.

use chrono::{Duration, Local, NaiveDate};
use rusqlite::{OptionalExtension, ToSql};

use crate::error::Result;
use crate::repo::sql::FROM_ACTIVITY_GROUP;
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

use super::range::{month_range, week_range};
use super::time::{parse_local, slice_by_hour};
use super::{AppDetail, DetailBucket, DeviceFilter, TitleUsage};

/// How the detail time bars are grouped: by hour on the Daily page, by day on the Weekly and
/// Monthly pages.
#[derive(Debug, Clone, Copy)]
enum BucketBy {
    Hour,
    Day,
}

/// Core of the details drawer opened by clicking an app: for the `[from, to]` date range and a
/// grouping, adds up the time bars (buckets) and the time per window title (titles). First finds
/// the group key of icon_process (the same rule as `GROUP BY COALESCE(g.display_name,
/// a.process_name)` in [`day_apps`](super::day_apps)), then adds up that group's activities.
async fn app_range_detail(
    pool: &DbPool,
    from: NaiveDate,
    to: NaiveDate,
    icon_process: String,
    device: DeviceFilter,
    bucket_by: BucketBy,
) -> Result<AppDetail> {
    let from_str = from.format("%Y-%m-%d").to_string();
    let to_str = to.format("%Y-%m-%d").to_string();
    let name_is_browser = crate::capture::browser_url::is_browser_app(&icon_process);

    let (raw_buckets, titles): (std::collections::HashMap<String, u64>, Vec<TitleUsage>) = pool
        .0
        .call(move |conn| {
            // 1) The process's group key; falls back to the process name if it has no group
            // TODO: `icon_process` needs a clearer name.
            let group_key: String = conn
                .query_row(
                    "SELECT group_id FROM app_group_members
                     WHERE process_name = ?1 AND deleted_at IS NULL",
                    rusqlite::params![icon_process],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .db()?
                .unwrap_or_else(|| icon_process.clone());

            // 2) Time bars. By hour, rows can't be grouped by local_hour: it is the hour the
            //    session *started* and is not updated when the session is sealed (see the note on
            //    day_hour_apps), so a session crossing an hour lands whole in its first hour and
            //    won't match the day_hours bars. So, as there, fetch the rows and split them by
            //    clock hour with slice_by_hour in Rust. By day, local_date is still summed in SQL.
            let mut raw: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
            // TODO: Say what `raw` is for: the match below fills it with the time bars, by hour
            // or by day.
            // TODO: Confusing name; find out what "bucket" means here.
            match bucket_by {
                BucketBy::Hour => {
                    let bsql = format!(
                        "SELECT a.started_at, a.ended_at
                         {FROM_ACTIVITY_GROUP}
                         WHERE a.local_date >= ? AND a.local_date <= ?
                           AND COALESCE(g.id, a.process_name) = ?
                           AND a.excluded = 0
                           {}",
                        device.sql_clause()
                    );
                    let mut bparams: Vec<&dyn ToSql> = Vec::new();
                    bparams.push(&from_str);
                    bparams.push(&to_str);
                    bparams.push(&group_key);
                    if let Some(extra) = device.extra_param() {
                        bparams.push(extra);
                    }
                    let mut bstmt = conn.prepare(&bsql).db()?;
                    let bit = bstmt
                        .query_map(bparams.as_slice(), |r| {
                            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                        })
                        .db()?;
                    for row in bit {
                        let (started, ended) = row.db()?;
                        let s = parse_local(&started);
                        let e = parse_local(&ended);
                        if e <= s {
                            continue;
                        }
                        for (h, secs) in slice_by_hour(s, e) {
                            *raw.entry(h.to_string()).or_insert(0) += secs;
                        }
                    }
                }
                BucketBy::Day => {
                    // TODO: Remove `AS k` and `AS total` and write `GROUP BY a.local_date`. Rust
                    // reads columns by position, not by name; only `GROUP BY` uses `k`, and
                    // nothing uses `total`.
                    let bsql = format!(
                        "SELECT a.local_date AS k, SUM(a.duration_secs) AS total
                         {FROM_ACTIVITY_GROUP}
                         WHERE a.local_date >= ? AND a.local_date <= ?
                           AND COALESCE(g.id, a.process_name) = ?
                           AND a.excluded = 0
                           {}
                         GROUP BY k",
                        device.sql_clause()
                    );
                    let mut bparams: Vec<&dyn ToSql> = Vec::new();
                    bparams.push(&from_str);
                    bparams.push(&to_str);
                    bparams.push(&group_key);
                    if let Some(extra) = device.extra_param() {
                        bparams.push(extra);
                    }
                    let mut bstmt = conn.prepare(&bsql).db()?;
                    let bit = bstmt
                        .query_map(bparams.as_slice(), |r| {
                            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as u64))
                        })
                        .db()?;
                    for row in bit {
                        let (k, secs) = row.db()?;
                        raw.insert(k, secs);
                    }
                }
            }

            // 3) Time per window title: grouped by (window_title, url_host), with a missing title
            //    as an empty string, most first. The same title on different sites (e.g. "Home")
            //    is counted separately: they are pages of different sites.
            // TODO: Rename the aliases `t` and `h` to `title` and `host`. Keep the aliases here:
            // `GROUP BY` uses them so it doesn't repeat the `COALESCE`, and `ORDER BY` uses
            // `total`. `tsql` is not a clear name either: the `t` is for title, but title of what?
            // TODO: Explain when the window title can be empty.
            let tsql = format!(
                "SELECT COALESCE(a.window_title, '') AS t, a.url_host AS h,
                        SUM(a.duration_secs) AS total
                 {FROM_ACTIVITY_GROUP}
                 WHERE a.local_date >= ? AND a.local_date <= ?
                   AND COALESCE(g.id, a.process_name) = ?
                   AND a.excluded = 0
                   {}
                 GROUP BY t, h
                 ORDER BY total DESC",
                device.sql_clause()
            );
            let mut tparams: Vec<&dyn ToSql> = Vec::new();
            tparams.push(&from_str);
            tparams.push(&to_str);
            tparams.push(&group_key);
            if let Some(extra) = device.extra_param() {
                tparams.push(extra);
            }
            let mut tstmt = conn.prepare(&tsql).db()?;
            // TODO: Odd name again. Why `TitleUsage`?
            let tit = tstmt
                .query_map(tparams.as_slice(), |r| {
                    Ok(TitleUsage {
                        title: r.get::<_, String>(0)?,
                        host: r.get::<_, Option<String>>(1)?,
                        secs: r.get::<_, i64>(2)?.max(0) as u32,
                    })
                })
                .db()?;
            let mut titles = Vec::new();
            for row in tit {
                titles.push(row.db()?);
            }

            Ok((raw, titles))
        })
        .await?;

    // 4) Spread the sparse totals into a full, ordered row of bars, empty ones as 0, ready to draw
    let buckets = match bucket_by {
        BucketBy::Hour => (0u8..24)
            .map(|h| {
                let key = h.to_string();
                let secs = raw_buckets.get(&key).copied().unwrap_or(0) as u32;
                DetailBucket { key, secs }
            })
            .collect(),
        BucketBy::Day => {
            let mut out = Vec::new();
            // TODO: Make it clear this is the current day; `from` and `to` mean something
            // different here too.
            let mut cur = from;
            while cur <= to {
                let key = cur.format("%Y-%m-%d").to_string();
                let secs = raw_buckets.get(&key).copied().unwrap_or(0) as u32;
                out.push(DetailBucket { key, secs });
                cur += Duration::days(1);
            }
            out
        }
    };

    // Data first: a row with a site means capture already treated the app as a browser (the
    // merged group's representative is MIN(process_name), which may be a non-browser member).
    // With no such rows, fall back to the name, for the "a browser, but no sites at all" hint.
    let is_browser = name_is_browser || titles.iter().any(|t| t.host.is_some());
    Ok(AppDetail {
        buckets,
        titles,
        is_browser,
    })
}

/// Details on the Daily page: the day by hour (24 buckets). `day_offset = 0` is today.
pub async fn app_day_detail(
    pool: &DbPool,
    day_offset: i32,
    icon_process: String,
    device: DeviceFilter,
) -> Result<AppDetail> {
    let date = (Local::now() + Duration::days(day_offset as i64)).date_naive();
    app_range_detail(pool, date, date, icon_process, device, BucketBy::Hour).await
}

/// Details on the Weekly page: Monday to Sunday by day (7 buckets). `week_offset = 0` is this
/// week.
pub async fn app_week_detail(
    pool: &DbPool,
    week_offset: i32,
    icon_process: String,
    device: DeviceFilter,
) -> Result<AppDetail> {
    // TODO: Check whether `week_range` and `month_range` can become one function.
    let (monday, sunday) = week_range(week_offset);
    app_range_detail(pool, monday, sunday, icon_process, device, BucketBy::Day).await
}

/// Details on the Monthly page: each day of the month (28–31 buckets). `month_offset = 0` is this
/// month.
pub async fn app_month_detail(
    pool: &DbPool,
    month_offset: i32,
    icon_process: String,
    device: DeviceFilter,
) -> Result<AppDetail> {
    let (first, last) = month_range(month_offset);
    app_range_detail(pool, first, last, icon_process, device, BucketBy::Day).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::reports::test_seed::{
        insert_activity, insert_session_titled, insert_session_with_times, seed_solo_group,
    };
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};
    use chrono::TimeZone;

    /// 测 [`app_day_detail`]（Hour 粒度）：
    /// - 固定 24 桶、key 按 "0".."23" 有序、无活动小时补 0
    /// - 跨小时会话按真实时钟切片分摊（10:30→11:30 应各给 10/11 点 1800s），
    ///   而不是按 local_hour 把整段挤进开始桶
    #[tokio::test]
    async fn app_day_detail_hour_buckets_split_and_zero_fill() {
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

        let detail = app_day_detail(&pool, 0, "Code".into(), DeviceFilter::All)
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

    /// 「按网站」分组的原料：titles 行带 url_host，同标题不同域名分开计
    /// （"首页"在 github 和 youtube 各算各的），无域名的老行照常返回且 host=None；
    /// 代表进程是浏览器时 is_browser=true，否则 false——前端据此决定是否分组。
    #[tokio::test]
    async fn app_day_detail_titles_carry_url_host_and_browser_flag() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive().format("%Y-%m-%d").to_string();
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

        let chrome = app_day_detail(&pool, 0, "Google Chrome".into(), DeviceFilter::All)
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

        let code = app_day_detail(&pool, 0, "Code".into(), DeviceFilter::All)
            .await
            .unwrap();
        assert!(!code.is_browser, "Code 不是浏览器");
        assert_eq!(code.titles.len(), 1);
        assert_eq!(code.titles[0].host, None);
    }

    /// 测 [`app_range_detail`] 的组 key 解析：icon_process 是组内任一成员时，
    /// 时间柱与标题都应聚合**整个组**（跨 OS 成员 + 跨设备），且不掺入组外应用。
    /// titles 按用时降序、同标题跨成员合并。
    #[tokio::test]
    async fn app_day_detail_resolves_group_and_merges_members() {
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

        // 用 win 侧成员名查询 → 应解析到组、把 mac 侧的量也算上
        let detail = app_day_detail(&pool, 0, "Code.exe".into(), DeviceFilter::All)
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
        let only_self = app_day_detail(
            &pool,
            0,
            "Code.exe".into(),
            DeviceFilter::Only(TEST_SELF_ID.into()),
        )
        .await
        .unwrap();
        let h10_self = only_self.buckets.iter().find(|b| b.key == "10").unwrap();
        assert_eq!(h10_self.secs, 300, "Only(self) 不该带上 win 端时长");
    }

    /// 测 [`app_range_detail`] 无组回退：icon_process 没有任何组成员记录时，
    /// 组 key 退化为 process_name 本身——只聚合同名进程，不吸入其它无组进程。
    #[tokio::test]
    async fn app_day_detail_falls_back_to_process_name_without_group() {
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

        let detail = app_day_detail(&pool, 0, "Lonely".into(), DeviceFilter::All)
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

    /// 测 [`app_week_detail`]（Day 粒度）：7 桶按日期有序、空天补 0、
    /// 范围外（上周日）的同名活动被排除。
    #[tokio::test]
    async fn app_week_detail_day_buckets_zero_filled_in_order() {
        let pool = fresh_test_pool().await;
        let (monday, _sunday) = week_range(0);
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

        let detail = app_week_detail(&pool, 0, "Code".into(), DeviceFilter::All)
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

    /// 测 [`app_month_detail`]（Day 粒度）：桶数 = 当月天数，逐日有序补 0。
    #[tokio::test]
    async fn app_month_detail_covers_whole_month() {
        let pool = fresh_test_pool().await;
        let (first, last) = month_range(0);
        let n_days = ((last - first).num_days() + 1) as usize;
        let day = |off: i64| (first + Duration::days(off)).format("%Y-%m-%d").to_string();

        insert_activity(&pool, TEST_SELF_ID, &day(0), "Code", 300).await;
        insert_activity(&pool, TEST_SELF_ID, &day(14), "Code", 450).await;
        seed_solo_group(&pool, "Code", "code").await;

        let detail = app_month_detail(&pool, 0, "Code".into(), DeviceFilter::All)
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
