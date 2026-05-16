//! `activities` 表的 repo 层：插入新会话、seal 会话写 outbox、清理过期截图。
//!
//! 一条 activities 行 = 一段连续焦点会话（同一应用 / 同一 URL）。
//! 焦点切换时旧的 seal（写 outbox 推送），开新的（插入但不推 outbox，避免心跳级噪声）。

use chrono::{DateTime, Duration, Local, TimeZone, Timelike, Utc};

use crate::capture::WindowInfo;
use crate::device;
use crate::error::Result;
use crate::repo::outbox::{enqueue, OutboxEntity, OutboxOp};
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

/// 创建一条新的会话记录。device_id = self；updated_at = captured_at；
/// **不**写 outbox —— 用户明确要求只在会话结束 (seal) 时才推到云端。
pub async fn insert_new(
    pool: &DbPool,
    info: &WindowInfo,
    captured_at: DateTime<Local>,
    screenshot_path: Option<String>,
) -> Result<i64> {
    let info = info.clone();
    let started = captured_at.to_rfc3339();
    let ended = captured_at.to_rfc3339();
    let updated = ended.clone();
    let local_date = captured_at.format("%Y-%m-%d").to_string();
    let local_hour = captured_at.hour() as u8;
    let device_id = device::self_id()?.to_string();

    let id = pool
        .0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO activities(
                    started_at, ended_at, duration_secs,
                    local_date, local_hour,
                    process_name, window_title, category_id, screenshot_path,
                    device_id, updated_at, origin
                ) VALUES (?, ?, 0, ?, ?, ?, ?, 'other', ?, ?, ?, 'local')",
                rusqlite::params![
                    started,
                    ended,
                    local_date,
                    local_hour,
                    info.app_name,
                    info.title,
                    screenshot_path,
                    device_id,
                    updated,
                ],
            )
            .db()?;
            Ok(conn.last_insert_rowid())
        })
        .await?;
    Ok(id)
}

/// 会话结束（焦点切到别的窗口那一刻）。
/// 同事务里：把 ended_at 钉死成 final_ended_at，更新 duration_secs / updated_at，并写一条 outbox 推到云端。
pub async fn seal_session(pool: &DbPool, id: i64, final_ended_at: DateTime<Local>) -> Result<()> {
    let ended = final_ended_at.to_rfc3339();
    let updated = Utc::now().to_rfc3339();
    let device_id = device::self_id()?.to_string();

    pool.0
        .call(move |conn| {
            // 取整行做 outbox payload 用
            // 9 字段元组：rusqlite query_row 的天然形状（每列对应一个）。
            // 抽 type alias 反而把字段语义信息隐藏到别的文件，可读性更差
            #[allow(clippy::type_complexity)]
            let row: Option<(
                String,
                String,
                i64,
                String,
                u8,
                String,
                Option<String>,
                String,
                String,
            )> = conn
                .query_row(
                    "SELECT started_at, ended_at, duration_secs, local_date, local_hour,
                            process_name, window_title, category_id, device_id
                     FROM activities WHERE id = ?",
                    [id],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                            r.get(7)?,
                            r.get(8)?,
                        ))
                    },
                )
                .ok();

            let Some((started_at, _, _, local_date, local_hour, process_name, window_title, category_id, this_device)) = row else {
                // 行不存在：可能是已经被清掉了；忽略
                return Ok(());
            };

            // 重算 duration
            // 解析失败时回退 epoch 0 当 fallback；timestamp_opt(0, 0) 是 chrono
            // 静态有效值（不变量保证），unwrap 在此安全
            let started = DateTime::parse_from_rfc3339(&started_at)
                .map(|dt| dt.with_timezone(&Local))
                .unwrap_or_else(|_| {
                    Local
                        .timestamp_opt(0, 0)
                        .single()
                        .expect("epoch 0 在 chrono 中固定有效")
                });
            let ended_dt = DateTime::parse_from_rfc3339(&ended)
                .map(|dt| dt.with_timezone(&Local))
                .unwrap_or_else(|_| Local::now());
            let dur = (ended_dt - started).num_seconds().max(0);

            conn.execute(
                "UPDATE activities SET ended_at = ?, duration_secs = ?, updated_at = ? WHERE id = ?",
                rusqlite::params![ended, dur, updated, id],
            )
            .db()?;

            // 只对 local 来源的会话写 outbox：远端拉来的不要再推回去
            if this_device == device_id {
                let payload = serde_json::json!({
                    "deviceId": this_device,
                    "startedAt": started_at,
                    "endedAt": ended,
                    "durationSecs": dur,
                    "localDate": local_date,
                    "localHour": local_hour,
                    "processName": process_name,
                    "windowTitle": window_title,
                    "categoryId": category_id,
                    "updatedAt": updated,
                })
                .to_string();
                enqueue(
                    conn,
                    OutboxOp::Upsert,
                    OutboxEntity::Activity,
                    &id.to_string(),
                    &payload,
                )
                .db()?;
            }

            Ok(())
        })
        .await?;
    Ok(())
}

/// 清理超过 retention_days 的截图文件（jpg），不删 activities 行；只把对应行的 screenshot_path 置 NULL。
/// 返回成功删除的文件数。
pub async fn delete_screenshots_older_than(pool: &DbPool, retention_days: u32) -> Result<u64> {
    let days = retention_days.max(1) as i64;
    let cutoff = (Local::now() - Duration::days(days))
        .format("%Y-%m-%d")
        .to_string();

    // 先取出待清理的 (id, path) 列表
    let rows: Vec<(i64, String)> = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, screenshot_path FROM activities
                     WHERE screenshot_path IS NOT NULL AND local_date < ?",
                )
                .db()?;
            let rows = stmt
                .query_map([&cutoff], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await?;

    // spawn_blocking 里逐个删文件（best-effort）
    let deleted_files = tokio::task::spawn_blocking({
        let rows = rows.clone();
        move || {
            let mut n = 0u64;
            for (_, path) in &rows {
                if std::fs::remove_file(path).is_ok() {
                    n += 1;
                }
            }
            n
        }
    })
    .await
    .unwrap_or(0);

    // 把这些行的 screenshot_path 置 NULL（即使文件删除失败也清引用，避免下次反复尝试）
    if !rows.is_empty() {
        let ids: Vec<i64> = rows.into_iter().map(|(id, _)| id).collect();
        pool.0
            .call(move |conn| {
                let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                let sql = format!(
                    "UPDATE activities SET screenshot_path = NULL WHERE id IN ({placeholders})"
                );
                let params: Vec<&dyn rusqlite::ToSql> =
                    ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
                conn.execute(&sql, params.as_slice()).db()?;
                Ok(())
            })
            .await?;
    }

    Ok(deleted_files)
}

/// 启动期：删掉本机自己之前跑遗留的 unsealed 孤儿 session 行。
///
/// 孤儿定义：`device_id = self_id AND duration_secs = 0 AND ended_at = started_at` —— 这种行只能由
/// [`insert_new`] 创建后没等到 [`seal_session`] 就被中断（app 退出 / crash / 服务 stop 没走到
/// seal 通道）产生。**当下没有任何 in-memory `current_lock` 指向它们**，因为本函数仅在
/// [`crate::capture::CaptureService::start`] 注册后台 tick task **之前**调用，
/// `Inner::current` 还是 None。
///
/// 副作用：
/// - **本地 DELETE**：所有匹配的行直接删（不软删，本表没 deleted_at 列）。
///   pure 0 时长的行没数据价值，删了 day_apps SUM 不变（贡献本来就是 0）。
/// - **触发 push 同步**：每个受影响的 local_date 入一个 outbox 行，下次 push tick
///   走 [`crate::sync::engine::push::build_activities_day`] 全量重写当天 ndjson 到 Drive。
///   对端 pull 收到 [`crate::sync::engine::pull::merge_activities`] 的 mirror 收敛
///   逻辑（按 ndjson 内容 DELETE 不在的镜像行）→ 对端镜像里这些孤儿也自然消失。
///
/// 幂等：连续调两次，第二次 SELECT DISTINCT 找不到匹配行 → 返回 0，no-op。
pub async fn purge_orphan_sessions(pool: &DbPool) -> Result<u64> {
    let device_id = device::self_id()?.to_string();

    // 1. 找出受影响的 local_date 列表（每个独立的天需要一条 outbox 触发 push 重写）
    let local_dates: Vec<String> = pool
        .0
        .call({
            let device_id = device_id.clone();
            move |conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT DISTINCT local_date FROM activities
                         WHERE device_id = ?1 AND duration_secs = 0 AND ended_at = started_at",
                    )
                    .db()?;
                let rows = stmt
                    .query_map(rusqlite::params![device_id], |r| r.get::<_, String>(0))
                    .db()?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r.db()?);
                }
                Ok(out)
            }
        })
        .await?;

    if local_dates.is_empty() {
        return Ok(0);
    }

    // 2. DELETE 一刀切 + 给每个受影响的 local_date 写一条 outbox（同一 conn / 同一事务）
    let deleted = pool
        .0
        .call({
            let device_id = device_id.clone();
            let local_dates = local_dates.clone();
            move |conn| {
                let n = conn
                    .execute(
                        "DELETE FROM activities
                         WHERE device_id = ?1 AND duration_secs = 0 AND ended_at = started_at",
                        rusqlite::params![device_id],
                    )
                    .db()? as u64;
                for date in &local_dates {
                    // payload 只用 localDate 字段（push.group_outbox 解析它决定 ndjson 文件名）。
                    // entity_pk 给 device_id 占位（NOT NULL 约束），不参与去重
                    let payload = serde_json::json!({ "localDate": date }).to_string();
                    enqueue(
                        conn,
                        OutboxOp::Upsert,
                        OutboxEntity::Activity,
                        &device_id,
                        &payload,
                    )
                    .db()?;
                }
                Ok(n)
            }
        })
        .await?;

    log::info!(
        "启动期清理孤儿 session：删 {} 行，触发 push 重写 {} 天",
        deleted,
        local_dates.len()
    );
    Ok(deleted)
}

/// 统计今天 activities 表的行数（按本机时区的 local_date 过滤）。给前端 status 指示器用。
pub async fn today_count(pool: &DbPool) -> Result<u32> {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let count = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare_cached("SELECT COUNT(*) FROM activities WHERE local_date = ?")
                .db()?;
            let n: i64 = stmt.query_row([&today], |r| r.get(0)).db()?;
            Ok(n as u32)
        })
        .await?;
    Ok(count)
}
