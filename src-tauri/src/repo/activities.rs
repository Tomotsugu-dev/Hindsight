//! `activities` 表的 repo 层：插入新会话、seal 会话写 outbox、清理过期截图。
//!
//! 一条 activities 行 = 一段连续焦点会话（同一应用 / 同一 URL）。
//! 焦点切换时旧的 seal（写 outbox 推送），开新的（插入但不推 outbox，避免心跳级噪声）。

use chrono::{DateTime, Duration, Local, TimeZone, Timelike, Utc};

use crate::capture::WindowInfo;
use crate::device;
use crate::error::Result;
use crate::repo::outbox::{enqueue, OutboxEntity, OutboxOp};
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};

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
    // updated_at 必须是 UTC：跨设备 LWW 走的是 `updated_at > cur_updated` **字符串字典序**
    // 比较。如果这里用 captured_at.to_rfc3339()（local TZ，比如 "+09:00"），后续 seal_session
    // 用 utc_now_rfc3339()（"+00:00"），两个 RFC3339 串的字典序跟时间序不一致 ——
    // JST 凌晨的 local 串 "2026-05-17T00:..." 字典序大于同一时刻的 UTC 串 "2026-05-16T15:..."
    // → 对端 pull 时 LWW 错误地拒绝 seal 后的 update → 镜像永远卡在 dur=0 unsealed。
    let updated = captured_at.with_timezone(&Utc).to_rfc3339();
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
    let updated = utc_now_rfc3339();
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
            // 不变量：ended_at >= started_at。挂机分支的 real_end = now - idle 可能早于
            // 一条"挂机期间才开出来"的会话的 started_at——只钳 duration 不钳 ended_at
            // 会写出负跨度行：时间轴渲染负宽度、推上云端，且永远匹配不上 orphan 清理的
            // `ended_at = started_at` 谓词。此处把写入值一并钳到 started_at。
            let ended_final = if ended_dt < started {
                started_at.clone()
            } else {
                ended.clone()
            };

            conn.execute(
                "UPDATE activities SET ended_at = ?, duration_secs = ?, updated_at = ? WHERE id = ?",
                rusqlite::params![ended_final, dur, updated, id],
            )
            .db()?;

            // 只对 local 来源的会话写 outbox：远端拉来的不要再推回去
            if this_device == device_id {
                let payload = serde_json::json!({
                    "deviceId": this_device,
                    "startedAt": started_at,
                    "endedAt": ended_final,
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
        .call({
            let cutoff = cutoff.clone();
            move |conn| {
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
            }
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

    // 把这些行的 screenshot_path 置 NULL（即使文件删除失败也清引用，避免下次反复尝试）。
    // 不用 id IN (...)：行数超过 SQLITE_MAX_VARIABLE_NUMBER(32766) 会报错，而此时
    // 文件已经删掉，引用清不掉 → 之后每轮清理永远失败。直接按同一 cutoff 条件 UPDATE。
    if !rows.is_empty() {
        pool.0
            .call(move |conn| {
                conn.execute(
                    "UPDATE activities SET screenshot_path = NULL
                     WHERE screenshot_path IS NOT NULL AND local_date < ?",
                    [&cutoff],
                )
                .db()?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};

    /// 测 [`purge_orphan_sessions`]：
    /// - 只删本机 (device_id = self_id) 的孤儿（dur=0 且 ended_at=started_at）
    /// - 不动本机 sealed 行（duration_secs > 0）
    /// - 不跨设备删（其它 device_id 的孤儿要留着）
    /// - 受影响的每个 local_date 入一条 outbox（让 push 重写当天 ndjson）
    #[tokio::test]
    async fn purge_orphan_sessions_only_self_keeps_sealed_and_other_devices() {
        let pool = fresh_test_pool().await;

        seed_activities(&pool).await;

        let deleted = purge_orphan_sessions(&pool).await.unwrap();
        assert_eq!(deleted, 3, "应删 3 行本机 orphan");

        let (self_total, other_total) = count_by_device(&pool).await;
        assert_eq!(self_total, 2, "本机 sealed 应留 2 行");
        assert_eq!(other_total, 1, "其它设备的 orphan 不该被本机的 purge 动到");

        let dates = outbox_activity_local_dates(&pool).await;
        assert!(
            dates.iter().any(|d| d == "2026-05-15"),
            "受影响的 local_date 应入 outbox（push 重写当天）"
        );

        // 幂等：再调一次没有可删的行，返回 0、outbox 不再增长
        let outbox_before = outbox_activity_count(&pool).await;
        let deleted2 = purge_orphan_sessions(&pool).await.unwrap();
        assert_eq!(deleted2, 0);
        assert_eq!(outbox_activity_count(&pool).await, outbox_before);
    }

    /// v26 trigger `activities_local_remote_id`：未指定 remote_id 的 INSERT 应
    /// 被自动填上 `CAST(id AS TEXT)`。本机自恢复 + 跨设备身份对称依赖这条不变量。
    #[tokio::test]
    async fn v26_trigger_fills_remote_id_when_null() {
        let pool = fresh_test_pool().await;
        let id = pool
            .0
            .call(|conn| {
                conn.execute(
                    "INSERT INTO activities(
                        started_at, ended_at, duration_secs, local_date, local_hour,
                        process_name, window_title, category_id, device_id, updated_at, origin
                     ) VALUES(
                        '2026-05-15T10:00:00Z', '2026-05-15T10:00:30Z', 30,
                        '2026-05-15', 10, 'Code', '', 'other', 'test-self-device',
                        '2026-05-15T10:00:30Z', 'local'
                     )",
                    [],
                )
                .db()?;
                Ok(conn.last_insert_rowid())
            })
            .await
            .unwrap();

        let remote_id = read_remote_id(&pool, id).await;
        assert_eq!(
            remote_id.as_deref(),
            Some(id.to_string().as_str()),
            "trigger 应把 remote_id 填成 CAST(id AS TEXT)"
        );
    }

    /// v26 trigger 的 `WHEN NEW.remote_id IS NULL` 保护：显式 remote_id 不该被覆盖。
    /// pull 路径走的就是显式 remote_id（来自源端 ndjson 的 id 字段），不能被本机 trigger 重写。
    #[tokio::test]
    async fn v26_trigger_does_not_override_explicit_remote_id() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|conn| {
                conn.execute(
                    "INSERT INTO activities(
                        started_at, ended_at, duration_secs, local_date, local_hour,
                        process_name, window_title, category_id, device_id, remote_id,
                        updated_at, origin
                     ) VALUES(
                        '2026-05-15T10:00:00Z', '2026-05-15T10:00:30Z', 30,
                        '2026-05-15', 10, 'Code', '', 'other', 'device-win',
                        'explicit-42', '2026-05-15T10:00:30Z', 'remote'
                     )",
                    [],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();

        let remote_id = pool
            .0
            .call(|conn| {
                let r: Option<String> = conn
                    .query_row(
                        "SELECT remote_id FROM activities WHERE remote_id = 'explicit-42'",
                        [],
                        |r| r.get(0),
                    )
                    .ok();
                Ok(r)
            })
            .await
            .unwrap();
        assert_eq!(remote_id.as_deref(), Some("explicit-42"));
    }

    /// 焦点窗口刚切入 ([`insert_new`])：写 activities 行但**不**入 outbox ——
    /// 心跳级 INSERT 不能每秒一条 push 到 Drive。outbox 只在 seal 时入。
    #[tokio::test]
    async fn insert_new_does_not_enqueue_outbox() {
        let pool = fresh_test_pool().await;
        let info = WindowInfo {
            app_name: "Code".into(),
            title: "main.rs".into(),
            app_path: None,
            pid: 0,
        };
        let captured = Local::now();
        let _id = insert_new(&pool, &info, captured, None).await.unwrap();

        assert_eq!(
            outbox_activity_count(&pool).await,
            0,
            "insert_new 不该入 outbox（心跳级 push 会把 Drive 吵爆）"
        );
    }

    /// 跨设备 LWW 的字符串字典序不变性：
    /// **同一行的 seal_session updated_at 必须字典序 > insert_new updated_at**。
    ///
    /// 跨设备 [`pull::upsert_remote_activity`] 用字符串比较 `updated_at > cur_updated`
    /// 判 LWW。如果 insert_new 用 Local TZ（`"+09:00"`）写 updated_at、seal_session 用
    /// UTC（`"+00:00"`）写，JST 凌晨这两个串字典序与时间序相反 ——
    /// `"2026-05-17T00:15:09+09:00"` (insert local) > `"2026-05-16T15:15:24+00:00"` (seal UTC)。
    ///
    /// 对端 pull 时 LWW 错误地拒绝 seal 后的 update → 镜像永远卡在 dur=0 unsealed。
    /// 这条 invariant 就是钉死「所有 updated_at 写入都用 UTC」。
    #[tokio::test]
    async fn insert_new_and_seal_session_updated_at_lww_ordering() {
        let pool = fresh_test_pool().await;
        let info = WindowInfo {
            app_name: "Code".into(),
            title: "main.rs".into(),
            app_path: None,
            pid: 0,
        };
        let captured = Local::now();
        let id = insert_new(&pool, &info, captured, None).await.unwrap();
        let insert_updated = read_updated_at(&pool, id).await;
        // 必须 UTC（'+00:00'），否则跨设备 LWW 会因为 +09:00 / +00:00 字典序错乱
        assert!(
            insert_updated.ends_with("+00:00"),
            "insert_new updated_at 必须 UTC（+00:00），实际：{insert_updated}"
        );

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        seal_session(&pool, id, captured + Duration::seconds(30))
            .await
            .unwrap();
        let seal_updated = read_updated_at(&pool, id).await;
        assert!(
            seal_updated.ends_with("+00:00"),
            "seal_session updated_at 必须 UTC，实际：{seal_updated}"
        );

        // 关键不变性：seal 后的字符串字典序 > insert 时的字符串
        assert!(
            seal_updated > insert_updated,
            "seal_updated 字典序应大于 insert_updated（防 LWW 错乱）\n  seal:   {seal_updated}\n  insert: {insert_updated}"
        );
    }

    async fn read_updated_at(pool: &DbPool, id: i64) -> String {
        pool.0
            .call(move |conn| {
                let s: String = conn
                    .query_row(
                        "SELECT updated_at FROM activities WHERE id = ?1",
                        rusqlite::params![id],
                        |r| r.get(0),
                    )
                    .db()?;
                Ok(s)
            })
            .await
            .unwrap()
    }

    /// [`seal_session`] 写一条 entity='activity' 的 Upsert outbox，payload 含
    /// deviceId/startedAt/endedAt/durationSecs/localDate/processName/updatedAt。
    /// 漏掉这条 push 永远不知道有这段 session，对端永远看不到。
    #[tokio::test]
    async fn seal_session_enqueues_outbox_with_full_payload() {
        let pool = fresh_test_pool().await;
        let info = WindowInfo {
            app_name: "Code".into(),
            title: "main.rs".into(),
            app_path: None,
            pid: 0,
        };
        let captured = Local::now();
        let id = insert_new(&pool, &info, captured, None).await.unwrap();
        seal_session(&pool, id, captured + Duration::seconds(30))
            .await
            .unwrap();

        assert_eq!(outbox_activity_count(&pool).await, 1);

        let payload = pool
            .0
            .call(|conn| {
                let s: String = conn
                    .query_row(
                        "SELECT payload FROM sync_outbox WHERE entity = 'activity' LIMIT 1",
                        [],
                        |r| r.get(0),
                    )
                    .db()?;
                Ok(s)
            })
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(
            v.get("deviceId").and_then(|x| x.as_str()),
            Some(TEST_SELF_ID)
        );
        assert_eq!(v.get("processName").and_then(|x| x.as_str()), Some("Code"));
        assert!(v.get("startedAt").and_then(|x| x.as_str()).is_some());
        assert!(v.get("endedAt").and_then(|x| x.as_str()).is_some());
        // duration_secs 取 ended - started 的整秒值，captured + 30s → 30
        assert_eq!(v.get("durationSecs").and_then(|x| x.as_i64()), Some(30));
        assert!(v.get("localDate").and_then(|x| x.as_str()).is_some());
        assert!(v.get("updatedAt").and_then(|x| x.as_str()).is_some());
    }

    async fn read_remote_id(pool: &DbPool, id: i64) -> Option<String> {
        pool.0
            .call(move |conn| {
                let r: Option<String> = conn
                    .query_row(
                        "SELECT remote_id FROM activities WHERE id = ?1",
                        rusqlite::params![id],
                        |r| r.get(0),
                    )
                    .ok();
                Ok(r)
            })
            .await
            .unwrap()
    }

    async fn seed_activities(pool: &DbPool) {
        pool.0
            .call(|conn| {
                // 3 行本机 orphan
                for _ in 0..3 {
                    conn.execute(
                        "INSERT INTO activities(
                            started_at, ended_at, duration_secs, local_date, local_hour,
                            process_name, window_title, category_id, device_id, updated_at, origin
                         ) VALUES(
                            '2026-05-15T10:00:00Z', '2026-05-15T10:00:00Z', 0, '2026-05-15', 10,
                            'Code', '', 'other', ?1, '2026-05-15T10:00:00Z', 'local'
                         )",
                        rusqlite::params![TEST_SELF_ID],
                    )
                    .db()?;
                }
                // 2 行本机 sealed
                for _ in 0..2 {
                    conn.execute(
                        "INSERT INTO activities(
                            started_at, ended_at, duration_secs, local_date, local_hour,
                            process_name, window_title, category_id, device_id, updated_at, origin
                         ) VALUES(
                            '2026-05-15T10:00:00Z', '2026-05-15T10:00:30Z', 30, '2026-05-15', 10,
                            'Code', '', 'other', ?1, '2026-05-15T10:00:30Z', 'local'
                         )",
                        rusqlite::params![TEST_SELF_ID],
                    )
                    .db()?;
                }
                // 1 行其它设备 orphan
                conn.execute(
                    "INSERT INTO activities(
                        started_at, ended_at, duration_secs, local_date, local_hour,
                        process_name, window_title, category_id, device_id, remote_id, updated_at, origin
                     ) VALUES(
                        '2026-05-15T11:00:00Z', '2026-05-15T11:00:00Z', 0, '2026-05-15', 11,
                        'Slack', '', 'other', 'other-device', 'remote-7', '2026-05-15T11:00:00Z', 'remote'
                     )",
                    [],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn count_by_device(pool: &DbPool) -> (i64, i64) {
        pool.0
            .call(|conn| {
                let self_total: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM activities WHERE device_id = ?1",
                        rusqlite::params![TEST_SELF_ID],
                        |r| r.get(0),
                    )
                    .db()?;
                let other_total: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM activities WHERE device_id != ?1",
                        rusqlite::params![TEST_SELF_ID],
                        |r| r.get(0),
                    )
                    .db()?;
                Ok((self_total, other_total))
            })
            .await
            .unwrap()
    }

    async fn outbox_activity_local_dates(pool: &DbPool) -> Vec<String> {
        pool.0
            .call(|conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT json_extract(payload, '$.localDate') FROM sync_outbox
                         WHERE entity = 'activity'",
                    )
                    .db()?;
                let rows = stmt.query_map([], |r| r.get::<_, Option<String>>(0)).db()?;
                let mut out = Vec::new();
                for r in rows {
                    if let Some(s) = r.db()? {
                        out.push(s);
                    }
                }
                Ok(out)
            })
            .await
            .unwrap()
    }

    async fn outbox_activity_count(pool: &DbPool) -> i64 {
        pool.0
            .call(|conn| {
                let n: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM sync_outbox WHERE entity = 'activity'",
                        [],
                        |r| r.get(0),
                    )
                    .db()?;
                Ok(n)
            })
            .await
            .unwrap()
    }
}
