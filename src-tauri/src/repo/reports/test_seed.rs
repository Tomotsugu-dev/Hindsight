//! 报表测试共用的造数据函数。

use chrono::{DateTime, Local, NaiveDate, TimeZone, Timelike};

use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

/// 本机时区下 2026 年某月某日的某时某分。
pub(super) fn local_time(month: u32, day: u32, hour: u32, min: u32) -> DateTime<Local> {
    Local
        .from_local_datetime(
            &NaiveDate::from_ymd_opt(2026, month, day)
                .unwrap()
                .and_hms_opt(hour, min, 0)
                .unwrap(),
        )
        .single()
        .unwrap()
}

pub(super) async fn insert_activity(
    pool: &DbPool,
    device_id: &str,
    local_date: &str,
    process_name: &str,
    duration_secs: i64,
) {
    let device_id = device_id.to_string();
    let local_date = local_date.to_string();
    let process_name = process_name.to_string();
    pool.0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO activities(
                    started_at, ended_at, duration_secs, local_date, local_hour,
                    process_name, window_title, category_id, device_id, updated_at, origin
                 ) VALUES(
                    ?1 || 'T10:00:00Z', ?1 || 'T10:00:30Z', ?2, ?1, 10,
                    ?3, '', 'other', ?4, ?1 || 'T10:00:30Z', 'local'
                 )",
                rusqlite::params![local_date, duration_secs, process_name, device_id],
            )
            .db()?;
            Ok(())
        })
        .await
        .unwrap();
}

/// 给 day_hours / day_hour_apps 测试用：插一行 sealed activity 但用真实的 local 时区
/// started_at / ended_at（不再用固定的 'T10:00:00Z' UTC 串）。
pub(super) async fn insert_session_with_times(
    pool: &DbPool,
    device_id: &str,
    local_date: &str,
    process_name: &str,
    started: DateTime<Local>,
    ended: DateTime<Local>,
) {
    let device_id = device_id.to_string();
    let local_date = local_date.to_string();
    let process_name = process_name.to_string();
    let dur = (ended - started).num_seconds().max(0);
    let local_hour = started.hour() as i64;
    let started_str = started.to_rfc3339();
    let ended_str = ended.to_rfc3339();
    pool.0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO activities(
                    started_at, ended_at, duration_secs, local_date, local_hour,
                    process_name, window_title, category_id, device_id, updated_at, origin
                 ) VALUES(?, ?, ?, ?, ?, ?, '', 'other', ?, ?, 'local')",
                rusqlite::params![
                    started_str,
                    ended_str,
                    dur,
                    local_date,
                    local_hour,
                    process_name,
                    device_id,
                    ended_str,
                ],
            )
            .db()?;
            Ok(())
        })
        .await
        .unwrap();
}

/// 同 [`insert_session_with_times`]，但可指定 window_title——给详情抽屉的
/// titles 聚合断言用。
#[allow(clippy::too_many_arguments)]
pub(super) async fn insert_session_titled(
    pool: &DbPool,
    device_id: &str,
    local_date: &str,
    process_name: &str,
    title: &str,
    started: DateTime<Local>,
    ended: DateTime<Local>,
) {
    let device_id = device_id.to_string();
    let local_date = local_date.to_string();
    let process_name = process_name.to_string();
    let title = title.to_string();
    let dur = (ended - started).num_seconds().max(0);
    let local_hour = started.hour() as i64;
    let started_str = started.to_rfc3339();
    let ended_str = ended.to_rfc3339();
    pool.0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO activities(
                    started_at, ended_at, duration_secs, local_date, local_hour,
                    process_name, window_title, category_id, device_id, updated_at, origin
                 ) VALUES(?, ?, ?, ?, ?, ?, ?, 'other', ?, ?, 'local')",
                rusqlite::params![
                    started_str,
                    ended_str,
                    dur,
                    local_date,
                    local_hour,
                    process_name,
                    title,
                    device_id,
                    ended_str,
                ],
            )
            .db()?;
            Ok(())
        })
        .await
        .unwrap();
}

/// 建一个分组：指定组 id、显示名、分类和成员进程名。
pub(super) async fn seed_group(
    pool: &DbPool,
    id: &str,
    display_name: &str,
    category_id: &str,
    members: &[&str],
) {
    let id = id.to_string();
    let display_name = display_name.to_string();
    let category_id = category_id.to_string();
    let members: Vec<String> = members.iter().map(|m| m.to_string()).collect();
    pool.0
        .call(move |conn| {
            let now = "2026-05-15T10:00:00Z";
            conn.execute(
                "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                 VALUES(?1, ?2, ?3, ?4, NULL)",
                rusqlite::params![id, display_name, category_id, now],
            )
            .db()?;
            for member in &members {
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES(?1, ?2, ?3, NULL)",
                    rusqlite::params![member, id, now],
                )
                .db()?;
            }
            Ok(())
        })
        .await
        .unwrap();
}

pub(super) async fn seed_solo_group(pool: &DbPool, name: &str, category_id: &str) {
    let name = name.to_string();
    let category_id = category_id.to_string();
    pool.0
        .call(move |conn| {
            let now = "2026-05-15T10:00:00Z";
            conn.execute(
                "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                 VALUES(?1, ?1, ?2, ?3, NULL)",
                rusqlite::params![name, category_id, now],
            )
            .db()?;
            conn.execute(
                "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                 VALUES(?1, ?1, ?2, NULL)",
                rusqlite::params![name, now],
            )
            .db()?;
            Ok(())
        })
        .await
        .unwrap();
}
