//! `daily_usage`: the seconds for each date, device, process and website (ADR-0016).
//!
//! Triggers on `activities` note the dates each change covers in `daily_usage_stale`. [`refresh`]
//! recomputes those dates. The reports call it before they read the table.

use std::collections::HashMap;

use chrono::NaiveDate;
use rusqlite::{params, Transaction};

use super::time::{parse_stored_time, split_by_date};
use crate::storage::{DbResult, SqliteResultExt};

/// Recomputes the dates noted in `daily_usage_stale` from `activities`, then clears the notes.
pub(super) fn refresh(conn: &mut rusqlite::Connection) -> DbResult<()> {
    let tx = conn.transaction().db()?;
    let notes = {
        let mut stmt = tx
            .prepare("SELECT from_date, to_date FROM daily_usage_stale")
            .db()?;
        let notes = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .db()?
            .collect::<rusqlite::Result<Vec<_>>>()
            .db()?;
        notes
    };
    if notes.is_empty() {
        return Ok(());
    }
    for (from, to) in date_ranges(&notes) {
        rebuild(&tx, from, to)?;
    }
    tx.execute("DELETE FROM daily_usage_stale", []).db()?;
    tx.commit().db()
}

/// Turns the notes into sorted date ranges, and merges ranges that overlap or touch.
///
/// Each range gets one more day at the end. [`split_by_date`] splits in the time zone of the
/// start time, and in that time zone the end can fall one day after the date in `ended_at`.
/// When the end date can't be read, or comes before the start date, only the start date is used.
fn date_ranges(notes: &[(String, String)]) -> Vec<(NaiveDate, NaiveDate)> {
    let parse = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok();
    let mut ranges: Vec<(NaiveDate, NaiveDate)> = notes
        .iter()
        .filter_map(|(from, to)| {
            let from = parse(from)?;
            let to = parse(to).filter(|to| *to >= from).unwrap_or(from);
            Some((from, to.succ_opt().unwrap_or(to)))
        })
        .collect();
    ranges.sort_unstable();
    // dedup_by passes (this range, the last range kept); returning true removes this range
    ranges.dedup_by(|cur, last| {
        let joins = cur.0 <= last.1.succ_opt().unwrap_or(last.1);
        if joins {
            last.1 = last.1.max(cur.1);
        }
        joins
    });
    ranges
}

/// Recomputes the rows of `daily_usage` from `from` to `to`, both included.
fn rebuild(tx: &Transaction, from: NaiveDate, to: NaiveDate) -> DbResult<()> {
    let from_str = from.format("%Y-%m-%d").to_string();
    let to_str = to.format("%Y-%m-%d").to_string();
    tx.execute(
        "DELETE FROM daily_usage WHERE local_date BETWEEN ?1 AND ?2",
        params![from_str, to_str],
    )
    .db()?;

    // Records that end on their own date: add up the stored seconds
    tx.execute(
        "INSERT INTO daily_usage
         SELECT local_date, device_id, process_name, COALESCE(url_host, ''), SUM(duration_secs)
           FROM activities
          WHERE local_date BETWEEN ?1 AND ?2 AND excluded = 0
            AND substr(ended_at, 1, 10) = local_date
          GROUP BY 1, 2, 3, 4
         HAVING SUM(duration_secs) > 0",
        params![from_str, to_str],
    )
    .db()?;

    // Records that run past midnight
    let mut stmt = tx
        .prepare(
            "SELECT device_id, process_name, COALESCE(url_host, ''), started_at, ended_at
               FROM activities
              WHERE local_date <= ?1 AND excluded = 0
                AND substr(ended_at, 1, 10) <> local_date",
        )
        .db()?;
    let rows = stmt
        .query_map(params![to_str], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .db()?;
    let mut parts: HashMap<(NaiveDate, String, String, String), i64> = HashMap::new();
    for row in rows {
        let (device, process, host, started, ended) = row.db()?;
        let (Some(s), Some(e)) = (parse_stored_time(&started), parse_stored_time(&ended)) else {
            continue;
        };
        for (date, s, e) in split_by_date(s, e) {
            let secs = (e - s).num_seconds();
            if date >= from && date <= to && secs > 0 {
                *parts
                    .entry((date, device.clone(), process.clone(), host.clone()))
                    .or_insert(0) += secs;
            }
        }
    }
    let mut add = tx
        .prepare(
            "INSERT INTO daily_usage VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT DO UPDATE SET secs = secs + excluded.secs",
        )
        .db()?;
    for ((date, device, process, host), secs) in parts {
        add.execute(params![
            date.format("%Y-%m-%d").to_string(),
            device,
            process,
            host,
            secs
        ])
        .db()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::reports::test_seed::{
        daily_usage_notes, daily_usage_rows, insert_activity, insert_session_with_times,
        insert_visit, local_time, UsageRow,
    };
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};
    use crate::storage::{DbPool, SqliteResultExt};

    async fn refresh_now(pool: &DbPool) {
        pool.0.call(refresh).await.unwrap();
    }

    async fn exec(pool: &DbPool, sql: &str) {
        let sql = sql.to_string();
        pool.0
            .call(move |conn| {
                conn.execute_batch(&sql).db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// 插一行起止时间写死的记录（带时区），给跨午夜和时区的测试用。
    async fn insert_raw(pool: &DbPool, local_date: &str, started: &str, ended: &str, secs: i64) {
        let (local_date, started, ended) = (
            local_date.to_string(),
            started.to_string(),
            ended.to_string(),
        );
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activities(
                        started_at, ended_at, duration_secs, local_date, local_hour,
                        process_name, window_title, category_id, device_id, updated_at, origin
                     ) VALUES(?1, ?2, ?3, ?4, 23, 'Code', '', 'other', ?5, ?2, 'local')",
                    rusqlite::params![started, ended, secs, local_date, TEST_SELF_ID],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    fn row(date: &str, process: &str, host: &str, secs: i64) -> UsageRow {
        row_on(date, TEST_SELF_ID, process, host, secs)
    }

    fn row_on(date: &str, device: &str, process: &str, host: &str, secs: i64) -> UsageRow {
        (
            date.to_string(),
            device.to_string(),
            process.to_string(),
            host.to_string(),
            secs,
        )
    }

    fn note(from: &str, to: &str) -> (String, String) {
        (from.to_string(), to.to_string())
    }

    /// 本机时区下 2026-10-21 23:50 到 10-22 00:10 的一段 Code，前后各 600 秒。
    async fn insert_past_midnight(pool: &DbPool) {
        insert_session_with_times(
            pool,
            TEST_SELF_ID,
            "2026-10-21",
            "Code",
            local_time(10, 21, 23, 50),
            local_time(10, 22, 0, 10),
        )
        .await;
    }

    // ───────────────────── 日期范围 ─────────────────────

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// 每段先往后多算一天，再排序合并：重叠的、被包含的、紧挨着的并成一段，中间空一天的不并。
    #[test]
    fn date_ranges_merges_overlapping_and_adjacent_ranges() {
        let notes = [
            note("2026-10-03", "2026-10-05"), // → 03..06
            note("2026-10-01", "2026-10-02"), // → 01..03，和上一段重叠；输入没排序
            note("2026-10-04", "2026-10-04"), // → 04..05，被 03..06 包含
            note("2026-10-07", "2026-10-07"), // → 07..08，紧挨着 06
            note("2026-10-10", "2026-10-10"), // → 10..11，和 08 之间空了 09
            note("2026-10-10", "2026-10-10"), // 重复的
        ];
        assert_eq!(
            date_ranges(&notes),
            vec![
                (day("2026-10-01"), day("2026-10-08")),
                (day("2026-10-10"), day("2026-10-11")),
            ]
        );
        assert_eq!(date_ranges(&[]), vec![]);
    }

    /// 开始日期读不出来的跳过；结束日期读不出来或早于开始日期的，只从开始那天算。
    #[test]
    fn date_ranges_skips_or_narrows_unreadable_notes() {
        let notes = [
            note("bad", "2026-10-12"),
            note("2026-10-20", "bad"),
            note("2026-10-25", "2026-10-24"),
        ];
        assert_eq!(
            date_ranges(&notes),
            vec![
                (day("2026-10-20"), day("2026-10-21")),
                (day("2026-10-25"), day("2026-10-26")),
            ]
        );
    }

    // ───────────────────── 重算 ─────────────────────

    /// 同一天、同一设备、同一进程、同一网站的记录加成一行；没有网站的存 ''。
    #[tokio::test]
    async fn refresh_adds_up_each_combination() {
        let pool = fresh_test_pool().await;
        insert_visit(
            &pool,
            "2026-10-21",
            "Google Chrome",
            Some("github.com"),
            1200,
        )
        .await;
        insert_visit(
            &pool,
            "2026-10-21",
            "Google Chrome",
            Some("github.com"),
            2200,
        )
        .await;
        insert_visit(&pool, "2026-10-21", "Code", None, 12600).await;

        refresh_now(&pool).await;

        assert_eq!(
            daily_usage_rows(&pool).await,
            vec![
                row("2026-10-21", "Code", "", 12600),
                row("2026-10-21", "Google Chrome", "github.com", 3400),
            ]
        );
    }

    /// 不同设备的同一个进程分成两行，设备筛选才查得出来。
    #[tokio::test]
    async fn refresh_keeps_devices_apart() {
        let pool = fresh_test_pool().await;
        insert_activity(&pool, TEST_SELF_ID, "2026-10-21", "Code", 600).await;
        insert_activity(&pool, "device-win", "2026-10-21", "Code", 300).await;

        refresh_now(&pool).await;

        assert_eq!(
            daily_usage_rows(&pool).await,
            vec![
                row_on("2026-10-21", "device-win", "Code", "", 300),
                row("2026-10-21", "Code", "", 600),
            ]
        );
    }

    /// 跨午夜的记录按午夜切开，两段分别算进两天。
    #[tokio::test]
    async fn refresh_splits_rows_past_midnight() {
        let pool = fresh_test_pool().await;
        insert_past_midnight(&pool).await;

        refresh_now(&pool).await;

        assert_eq!(
            daily_usage_rows(&pool).await,
            vec![
                row("2026-10-21", "Code", "", 600),
                row("2026-10-22", "Code", "", 600),
            ]
        );
    }

    /// 跨两个午夜的记录切成三段，中间那天是整 24 小时。
    #[tokio::test]
    async fn refresh_splits_rows_across_two_midnights() {
        let pool = fresh_test_pool().await;
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            "2026-10-21",
            "Code",
            local_time(10, 21, 23, 50),
            local_time(10, 23, 0, 10),
        )
        .await;

        refresh_now(&pool).await;

        assert_eq!(
            daily_usage_rows(&pool).await,
            vec![
                row("2026-10-21", "Code", "", 600),
                row("2026-10-22", "Code", "", 86400),
                row("2026-10-23", "Code", "", 600),
            ]
        );
    }

    /// 正好在午夜结束的记录只算第一天，第二天不留 0 秒的行。
    #[tokio::test]
    async fn refresh_counts_a_row_ending_at_midnight_on_its_first_day_only() {
        let pool = fresh_test_pool().await;
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            "2026-10-21",
            "Code",
            local_time(10, 21, 23, 50),
            local_time(10, 22, 0, 0),
        )
        .await;

        refresh_now(&pool).await;

        assert_eq!(
            daily_usage_rows(&pool).await,
            vec![row("2026-10-21", "Code", "", 600)]
        );
    }

    /// 结束时间换了时区（比如出差途中）：按开始时间的时区切。结束时间写的是 10-22，
    /// 换到开始时间的时区已经是 10-23，10-23 也要算出来。
    #[tokio::test]
    async fn refresh_splits_in_the_start_time_zone() {
        let pool = fresh_test_pool().await;
        insert_raw(
            &pool,
            "2026-10-21",
            "2026-10-21T23:30:00+09:00",
            "2026-10-22T16:00:00+00:00",
            91800,
        )
        .await;

        refresh_now(&pool).await;

        assert_eq!(
            daily_usage_rows(&pool).await,
            vec![
                row("2026-10-21", "Code", "", 1800),
                row("2026-10-22", "Code", "", 86400),
                row("2026-10-23", "Code", "", 3600),
            ]
        );
    }

    /// 被忽略规则命中的记录不算，跨午夜的也一样。
    #[tokio::test]
    async fn refresh_leaves_out_ignored_rows() {
        let pool = fresh_test_pool().await;
        insert_activity(&pool, TEST_SELF_ID, "2026-10-21", "Code", 600).await;
        insert_activity(&pool, TEST_SELF_ID, "2026-10-21", "Downloader", 300).await;
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            "2026-10-21",
            "Downloader",
            local_time(10, 21, 23, 50),
            local_time(10, 22, 0, 10),
        )
        .await;
        exec(
            &pool,
            "UPDATE activities SET excluded = 1 WHERE process_name = 'Downloader'",
        )
        .await;

        refresh_now(&pool).await;

        assert_eq!(
            daily_usage_rows(&pool).await,
            vec![row("2026-10-21", "Code", "", 600)]
        );
    }

    /// 合计是 0 秒的组合不存，和统计里丢掉 0 秒的行一致。
    #[tokio::test]
    async fn refresh_leaves_out_zero_second_combinations() {
        let pool = fresh_test_pool().await;
        insert_activity(&pool, TEST_SELF_ID, "2026-10-21", "Code", 0).await;

        refresh_now(&pool).await;

        assert!(daily_usage_rows(&pool).await.is_empty());
    }

    /// 某天的记录全删了，这一天在表里也不留行。
    #[tokio::test]
    async fn refresh_removes_a_day_whose_rows_are_all_deleted() {
        let pool = fresh_test_pool().await;
        insert_activity(&pool, TEST_SELF_ID, "2026-10-21", "Code", 600).await;
        refresh_now(&pool).await;

        exec(&pool, "DELETE FROM activities").await;
        refresh_now(&pool).await;

        assert!(daily_usage_rows(&pool).await.is_empty());
    }

    /// 只重算第二天时，前一天跨午夜过来的那一段也要算上。
    #[tokio::test]
    async fn refresh_of_a_day_keeps_the_part_from_the_day_before() {
        let pool = fresh_test_pool().await;
        insert_past_midnight(&pool).await;
        refresh_now(&pool).await;

        insert_activity(&pool, TEST_SELF_ID, "2026-10-22", "Code", 300).await;
        assert_eq!(
            daily_usage_notes(&pool).await,
            vec![note("2026-10-22", "2026-10-22")]
        );
        refresh_now(&pool).await;

        assert_eq!(
            daily_usage_rows(&pool).await,
            vec![
                row("2026-10-21", "Code", "", 600),
                row("2026-10-22", "Code", "", 900),
            ]
        );
    }

    /// 只重算记下的日子，别的日子不动——即使它们是错的（ADR-0016：错了不会自己变对）。
    #[tokio::test]
    async fn refresh_leaves_days_without_notes_alone() {
        let pool = fresh_test_pool().await;
        insert_activity(&pool, TEST_SELF_ID, "2026-10-21", "Code", 600).await;
        refresh_now(&pool).await;
        exec(
            &pool,
            "UPDATE daily_usage SET secs = 1 WHERE local_date = '2026-10-21'",
        )
        .await;

        insert_activity(&pool, TEST_SELF_ID, "2026-10-22", "Code", 300).await;
        refresh_now(&pool).await;

        assert_eq!(
            daily_usage_rows(&pool).await,
            vec![
                row("2026-10-21", "Code", "", 1),
                row("2026-10-22", "Code", "", 300),
            ]
        );
    }

    /// 算完清掉标记；没有标记时什么都不做。
    #[tokio::test]
    async fn refresh_clears_the_notes() {
        let pool = fresh_test_pool().await;
        insert_activity(&pool, TEST_SELF_ID, "2026-10-21", "Code", 600).await;
        assert!(!daily_usage_notes(&pool).await.is_empty());

        refresh_now(&pool).await;
        assert!(daily_usage_notes(&pool).await.is_empty());

        refresh_now(&pool).await;
        assert_eq!(
            daily_usage_rows(&pool).await,
            vec![row("2026-10-21", "Code", "", 600)]
        );
    }

    // ───────────────────── 触发器记日期 ─────────────────────

    /// 插入一行：记下开始那天到结束那天。
    #[tokio::test]
    async fn insert_notes_the_days_it_covers() {
        let pool = fresh_test_pool().await;
        insert_activity(&pool, TEST_SELF_ID, "2026-10-20", "Code", 600).await;
        insert_past_midnight(&pool).await;

        assert_eq!(
            daily_usage_notes(&pool).await,
            vec![
                note("2026-10-20", "2026-10-20"),
                note("2026-10-21", "2026-10-22"),
            ]
        );
    }

    /// 删除一行：记下它原来覆盖的日子。
    #[tokio::test]
    async fn delete_notes_the_days_it_covered() {
        let pool = fresh_test_pool().await;
        insert_past_midnight(&pool).await;
        refresh_now(&pool).await;

        exec(&pool, "DELETE FROM activities").await;

        assert_eq!(
            daily_usage_notes(&pool).await,
            vec![note("2026-10-21", "2026-10-22")]
        );
    }

    /// 修改：改之前和改之后覆盖的日子都记下。结束时间推到第二天，第二天也要重算。
    #[tokio::test]
    async fn update_notes_the_days_before_and_after() {
        let pool = fresh_test_pool().await;
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            "2026-10-21",
            "Code",
            local_time(10, 21, 23, 0),
            local_time(10, 21, 23, 30),
        )
        .await;
        refresh_now(&pool).await;

        let ended = local_time(10, 22, 0, 5).to_rfc3339();
        exec(
            &pool,
            &format!("UPDATE activities SET ended_at = '{ended}', duration_secs = 3900"),
        )
        .await;

        assert_eq!(
            daily_usage_notes(&pool).await,
            vec![
                note("2026-10-21", "2026-10-21"),
                note("2026-10-21", "2026-10-22"),
            ]
        );
    }

    /// 和时长有关的列，改哪一列都要记日期。
    #[tokio::test]
    async fn updates_of_counted_columns_note_the_days() {
        for set in [
            "local_date = '2026-10-20'",
            "device_id = 'device-win'",
            "process_name = 'Figma'",
            "url_host = 'github.com'",
            "duration_secs = 900",
            "started_at = '2026-10-21T09:00:00Z'",
            "ended_at = '2026-10-21T11:00:00Z'",
            "excluded = 1",
        ] {
            let pool = fresh_test_pool().await;
            insert_activity(&pool, TEST_SELF_ID, "2026-10-21", "Code", 600).await;
            refresh_now(&pool).await;

            exec(&pool, &format!("UPDATE activities SET {set}")).await;

            assert!(
                !daily_usage_notes(&pool).await.is_empty(),
                "改了 {set} 应该记下日期"
            );
        }
    }

    /// 和时长无关的列（标题、截图、同步字段……）改了不记日期。
    #[tokio::test]
    async fn updates_of_other_columns_note_nothing() {
        for set in [
            "window_title = '新标题'",
            "screenshot_path = 'a.jpg'",
            "image_hash = 42",
            "remote_id = 'r-1'",
            "updated_at = '2026-10-22T00:00:00Z'",
            "origin = 'remote'",
            "category_id = 'code'",
            "local_hour = 11",
        ] {
            let pool = fresh_test_pool().await;
            insert_activity(&pool, TEST_SELF_ID, "2026-10-21", "Code", 600).await;
            refresh_now(&pool).await;

            exec(&pool, &format!("UPDATE activities SET {set}")).await;

            assert!(
                daily_usage_notes(&pool).await.is_empty(),
                "改了 {set} 不该记日期"
            );
        }
    }

    /// 采集每隔几秒延长同一行，同一个日期只记一条。
    #[tokio::test]
    async fn repeated_updates_keep_one_note() {
        let pool = fresh_test_pool().await;
        insert_activity(&pool, TEST_SELF_ID, "2026-10-21", "Code", 5).await;
        for secs in [10, 15, 20] {
            exec(
                &pool,
                &format!("UPDATE activities SET duration_secs = {secs}"),
            )
            .await;
        }

        assert_eq!(
            daily_usage_notes(&pool).await,
            vec![note("2026-10-21", "2026-10-21")]
        );
    }

    // ───────────────────── 整体 ─────────────────────

    /// 各种写入以后，只重算记下的日子，结果和整表重建完全一样。
    #[tokio::test]
    async fn refreshing_noted_days_matches_a_full_rebuild() {
        let pool = fresh_test_pool().await;
        insert_activity(&pool, TEST_SELF_ID, "2026-10-20", "Code", 600).await;
        insert_visit(
            &pool,
            "2026-10-20",
            "Google Chrome",
            Some("github.com"),
            300,
        )
        .await;
        insert_activity(&pool, TEST_SELF_ID, "2026-10-21", "Code", 1200).await;
        insert_past_midnight(&pool).await;
        insert_visit(
            &pool,
            "2026-10-22",
            "Google Chrome",
            Some("bilibili.com"),
            900,
        )
        .await;
        insert_activity(&pool, "device-win", "2026-10-22", "Code", 400).await;
        refresh_now(&pool).await;

        exec(
            &pool,
            "UPDATE activities SET excluded = 1
              WHERE local_date = '2026-10-20' AND process_name = 'Google Chrome';
             DELETE FROM activities WHERE local_date = '2026-10-20' AND process_name = 'Code';
             UPDATE activities SET duration_secs = duration_secs + 60
              WHERE device_id = 'device-win';
             UPDATE activities SET ended_at = '2026-10-22T00:05:00Z'
              WHERE local_date = '2026-10-21' AND duration_secs = 1200;
             UPDATE activities SET device_id = 'device-mac2' WHERE url_host = 'bilibili.com';
             UPDATE activities SET url_host = 'github.com' WHERE url_host = 'bilibili.com';
             UPDATE activities SET local_date = '2026-10-19' WHERE device_id = 'device-win';",
        )
        .await;
        insert_activity(&pool, TEST_SELF_ID, "2026-10-23", "Figma", 700).await;
        refresh_now(&pool).await;
        let after_notes = daily_usage_rows(&pool).await;

        exec(
            &pool,
            "DELETE FROM daily_usage;
             INSERT OR IGNORE INTO daily_usage_stale
             SELECT DISTINCT local_date, substr(ended_at, 1, 10) FROM activities;",
        )
        .await;
        refresh_now(&pool).await;

        assert_eq!(after_notes, daily_usage_rows(&pool).await);
    }

    /// ADR-0016 的约束：`INSERT OR REPLACE` 删旧行时不触发删除触发器，旧行那天就漏记了。
    /// 代码里不许出现往 activities 里 REPLACE 的写法。
    #[test]
    fn no_code_replaces_rows_in_activities() {
        let needle = ["REPLACE", " INTO ACTIVITIES"].concat();
        let mut dirs = vec![std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src"
        ))];
        let mut offenders = Vec::new();
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap().to_uppercase();
                    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
                    if flat.contains(&needle) {
                        offenders.push(path);
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "往 activities 里 REPLACE: {offenders:?}"
        );
    }
}
