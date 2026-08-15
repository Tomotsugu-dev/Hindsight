//! `activities` 表的 repo 层：插入新会话、seal 会话写 outbox、清理过期截图。
//!
//! 一条 activities 行 = 一段连续焦点会话（同一应用 / 同一 URL）。
//! 焦点切换时旧的 seal（写 outbox 推送），开新的（插入但不推 outbox，避免心跳级噪声）。

use chrono::{DateTime, Duration, Local, TimeZone, Timelike, Utc};

use crate::capture::ignore::{self, IgnoreRule};
use crate::capture::WindowInfo;
use crate::device;
use crate::error::Result;
use crate::repo::outbox::{enqueue, OutboxEntity, OutboxOp};
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};

/// 创建一条新的会话记录。device_id = self；updated_at = captured_at；
/// **不**写 outbox —— 用户明确要求只在会话结束 (seal) 时才推到云端。
/// `excluded`：忽略规则命中时为 true——行照常落库，仅不计入统计
/// （本机元数据，不进 seal 的 outbox payload，不参与同步）。
pub async fn insert_new(
    pool: &DbPool,
    info: &WindowInfo,
    captured_at: DateTime<Local>,
    screenshot_path: Option<String>,
    excluded: bool,
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
                    device_id, updated_at, origin, excluded
                ) VALUES (?, ?, 0, ?, ?, ?, ?, 'other', ?, ?, ?, 'local', ?)",
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
                    excluded,
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

/// 忽略规则变更后全表重算 `excluded` 标记（双向：新命中置 1，不再命中清 0），
/// 返回改动行数。匹配在 Rust 里做（与采集期 / pull 合并期同源走
/// [`ignore::is_excluded`]），不用 SQL LIKE——通配符要转义、SQLite `lower()`
/// 只认 ASCII，两头的大小写语义会分叉，同一行两处判出不同结果。
/// 只改 excluded 位：不 bump updated_at、不写 outbox——标记是本机元数据，
/// 不参与同步，动了 updated_at 反而会经 LWW 干扰对端。
pub async fn reapply_ignore_rules(pool: &DbPool, rules: &[IgnoreRule]) -> Result<u64> {
    let rules = rules.to_vec();
    let changed = pool
        .0
        .call(move |conn| {
            // 全表在 Rust 里算目标值：10 万行级毫秒级完成，且只在规则增删时跑一次。
            let mut to_set: Vec<i64> = Vec::new();
            let mut to_clear: Vec<i64> = Vec::new();
            {
                let mut stmt = conn
                    .prepare(
                        "SELECT id, process_name, COALESCE(window_title, ''), excluded
                         FROM activities",
                    )
                    .db()?;
                let it = stmt
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, i64>(3)?,
                        ))
                    })
                    .db()?;
                for row in it {
                    let (id, process, title, cur) = row.db()?;
                    let want = i64::from(ignore::is_excluded(&process, &title, &rules));
                    if want != cur {
                        if want == 1 {
                            to_set.push(id);
                        } else {
                            to_clear.push(id);
                        }
                    }
                }
            }
            let changed = (to_set.len() + to_clear.len()) as u64;
            let tx = conn.transaction().db()?;
            for (flag, ids) in [(1i64, &to_set), (0i64, &to_clear)] {
                // 按 500 一批拼 IN 列表，避开 SQLite 变量数上限（旧默认 999）
                for chunk in ids.chunks(500) {
                    let ph = vec!["?"; chunk.len()].join(",");
                    let sql = format!("UPDATE activities SET excluded = {flag} WHERE id IN ({ph})");
                    let params: Vec<&dyn rusqlite::ToSql> =
                        chunk.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
                    tx.execute(&sql, params.as_slice()).db()?;
                }
            }
            tx.commit().db()?;
            Ok(changed)
        })
        .await?;
    Ok(changed)
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
        let _id = insert_new(&pool, &info, captured, None, false)
            .await
            .unwrap();

        assert_eq!(
            outbox_activity_count(&pool).await,
            0,
            "insert_new 不该入 outbox（心跳级 push 会把 Drive 吵爆）"
        );
    }

    /// 忽略规则的全表重算：加规则 → 命中行置 1；删规则 → 清回 0。
    /// 同进程不同标题的行不许被误伤（规则是 进程+标题 双条件）。
    #[tokio::test]
    async fn reapply_ignore_rules_sets_and_clears() {
        let pool = fresh_test_pool().await;
        let dl = WindowInfo {
            app_name: "Windows Terminal Host".into(),
            title: "✳ Download videos from July 17 onwards with uv".into(),
            app_path: None,
            pid: 0,
        };
        let vim = WindowInfo {
            app_name: "Windows Terminal Host".into(),
            title: "vim main.rs".into(),
            app_path: None,
            pid: 0,
        };
        let captured = Local::now();
        let dl_id = insert_new(&pool, &dl, captured, None, false).await.unwrap();
        let vim_id = insert_new(&pool, &vim, captured, None, false)
            .await
            .unwrap();

        let rules = vec![IgnoreRule {
            process_name: "Windows Terminal Host".into(),
            title_keyword: Some("Download videos".into()),
        }];
        let changed = reapply_ignore_rules(&pool, &rules).await.unwrap();
        assert_eq!(changed, 1, "只有下载窗口那行该被打标");
        assert_eq!(excluded_flag(&pool, dl_id).await, 1);
        assert_eq!(
            excluded_flag(&pool, vim_id).await,
            0,
            "同终端干别的活不受牵连"
        );

        let changed = reapply_ignore_rules(&pool, &[]).await.unwrap();
        assert_eq!(changed, 1, "删规则后标记该清回来（可逆）");
        assert_eq!(excluded_flag(&pool, dl_id).await, 0);
    }

    async fn excluded_flag(pool: &DbPool, id: i64) -> i64 {
        pool.0
            .call(move |conn| {
                let v: i64 = conn
                    .query_row("SELECT excluded FROM activities WHERE id = ?", [id], |r| {
                        r.get(0)
                    })
                    .unwrap();
                Ok(v)
            })
            .await
            .unwrap()
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
        let id = insert_new(&pool, &info, captured, None, false)
            .await
            .unwrap();
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
        let id = insert_new(&pool, &info, captured, None, false)
            .await
            .unwrap();
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

    /// 测 [`delete_screenshots_older_than`]：只删 `local_date < cutoff`（cutoff =
    /// 今天 - retention_days）的文件与引用——
    /// - 3 天前的行：文件删除 + screenshot_path 置 NULL，activities 行本身保留
    /// - 恰好 cutoff 当天（today-2, retention=2）：严格小于，不删
    /// - 今天：不删
    /// - 返回值 = 实际从磁盘删掉的文件数
    #[tokio::test]
    async fn delete_screenshots_only_removes_rows_and_files_before_cutoff() {
        let pool = fresh_test_pool().await;
        // 真实临时目录 + 真实文件：验证"删对文件、不删错文件"，不是只看 DB
        let dir =
            std::env::temp_dir().join(format!("hindsight-shots-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let date = |days_ago: i64| {
            (Local::now() - Duration::days(days_ago))
                .format("%Y-%m-%d")
                .to_string()
        };
        let f_old = dir.join("old.jpg");
        let f_edge = dir.join("edge.jpg");
        let f_new = dir.join("new.jpg");
        for f in [&f_old, &f_edge, &f_new] {
            std::fs::write(f, b"jpg-bytes").unwrap();
        }

        let id_old = insert_shot_row(&pool, &date(3), Some(f_old.to_string_lossy().into())).await;
        let id_edge = insert_shot_row(&pool, &date(2), Some(f_edge.to_string_lossy().into())).await;
        let id_new = insert_shot_row(&pool, &date(0), Some(f_new.to_string_lossy().into())).await;

        let deleted = delete_screenshots_older_than(&pool, 2).await.unwrap();
        assert_eq!(deleted, 1, "只有 3 天前那一个文件该被删");

        // 磁盘侧：老文件消失，边界/新文件原样
        assert!(!f_old.exists(), "cutoff 之前的文件应被删除");
        assert!(
            f_edge.exists(),
            "local_date == cutoff（today-2）是严格小于边界，不该删"
        );
        assert!(f_new.exists(), "今天的文件不该删");

        // DB 侧：引用与文件一致——被删文件的行 path 置 NULL，行本身保留；其余 path 原样
        assert_eq!(read_screenshot_path(&pool, id_old).await, None);
        assert_eq!(
            read_screenshot_path(&pool, id_edge).await.as_deref(),
            Some(f_edge.to_string_lossy().as_ref())
        );
        assert_eq!(
            read_screenshot_path(&pool, id_new).await.as_deref(),
            Some(f_new.to_string_lossy().as_ref())
        );
        assert_eq!(
            count_rows(&pool).await,
            3,
            "清理只动 screenshot_path，不删 activities 行"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 测 retention_days = 0 的 `max(1)` 钳制：0 不是"全部删光"，而是按 1 天算——
    /// cutoff = 昨天，昨天/今天的文件必须保留，只有前天更早的才删。
    #[tokio::test]
    async fn delete_screenshots_clamps_zero_retention_to_one_day() {
        let pool = fresh_test_pool().await;
        let dir =
            std::env::temp_dir().join(format!("hindsight-shots-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let date = |days_ago: i64| {
            (Local::now() - Duration::days(days_ago))
                .format("%Y-%m-%d")
                .to_string()
        };
        let f_d2 = dir.join("d2.jpg");
        let f_d1 = dir.join("d1.jpg");
        let f_d0 = dir.join("d0.jpg");
        for f in [&f_d2, &f_d1, &f_d0] {
            std::fs::write(f, b"jpg").unwrap();
        }
        insert_shot_row(&pool, &date(2), Some(f_d2.to_string_lossy().into())).await;
        insert_shot_row(&pool, &date(1), Some(f_d1.to_string_lossy().into())).await;
        insert_shot_row(&pool, &date(0), Some(f_d0.to_string_lossy().into())).await;

        let deleted = delete_screenshots_older_than(&pool, 0).await.unwrap();
        assert_eq!(deleted, 1, "0 应被钳成 1 天：只删前天的");
        assert!(!f_d2.exists());
        // 若 0 没被钳制，cutoff 会变成今天 → 昨天的文件被误删。这两条钉死语义
        assert!(f_d1.exists(), "昨天的文件必须保留（max(1) 语义）");
        assert!(f_d0.exists(), "今天的文件必须保留");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 磁盘文件已丢失（用户手动删过 / 路径失效）时：返回值不计它（没删成任何文件），
    /// 但 DB 引用仍要清成 NULL——否则每轮清理都反复尝试同一批死路径。
    #[tokio::test]
    async fn delete_screenshots_nulls_reference_even_when_file_missing() {
        let pool = fresh_test_pool().await;
        let ghost = std::env::temp_dir().join(format!(
            "hindsight-shots-test-missing-{}.jpg",
            uuid::Uuid::new_v4()
        ));
        assert!(!ghost.exists());
        let date = (Local::now() - Duration::days(5))
            .format("%Y-%m-%d")
            .to_string();
        let id = insert_shot_row(&pool, &date, Some(ghost.to_string_lossy().into())).await;

        let deleted = delete_screenshots_older_than(&pool, 2).await.unwrap();
        assert_eq!(deleted, 0, "文件本就不存在，删除计数应为 0");
        assert_eq!(
            read_screenshot_path(&pool, id).await,
            None,
            "文件删除失败也要清引用，避免死路径反复重试"
        );
    }

    /// 测 [`seal_session`] 的 started_at 解析失败兜底：DB 里的 started_at 是非法
    /// 时间串（外部写坏/同步来的脏数据）时回退 epoch 0——duration 退化成
    /// "ended 的 unix 秒数"（巨大但非负），绝不能出现负值或 panic。
    #[tokio::test]
    async fn seal_session_garbage_started_at_falls_back_to_epoch_zero() {
        let pool = fresh_test_pool().await;
        let id = pool
            .0
            .call(|conn| {
                conn.execute(
                    "INSERT INTO activities(
                        started_at, ended_at, duration_secs, local_date, local_hour,
                        process_name, window_title, category_id, device_id, updated_at, origin
                     ) VALUES(
                        'not-a-timestamp', 'not-a-timestamp', 0, '2026-05-15', 10,
                        'Code', '', 'other', ?1, '2026-05-15T10:00:00Z', 'local'
                     )",
                    rusqlite::params![TEST_SELF_ID],
                )
                .db()?;
                Ok(conn.last_insert_rowid())
            })
            .await
            .unwrap();

        let ended = Local::now();
        seal_session(&pool, id, ended).await.unwrap();

        let (ended_at, dur) = read_ended_and_duration(&pool, id).await;
        assert!(dur >= 0, "解析失败兜底后 duration 不得为负，实际 {dur}");
        // started 回退 epoch 0 → dur 应恰等于 ended 的 unix 秒数（独立推导的期望值）
        assert_eq!(dur, ended.timestamp(), "dur 应 = ended - epoch0");
        assert_eq!(
            ended_at,
            ended.to_rfc3339(),
            "ended_at 应钉成传入的结束时刻"
        );
    }

    /// 测 [`seal_session`] 的负跨度钳制：final_ended_at 早于 started_at（挂机
    /// 回拨分支可能出现）时，ended_at 钳到 started_at、duration 钳 0——
    /// 不能写出负宽度行。
    #[tokio::test]
    async fn seal_session_clamps_negative_span_to_zero() {
        let pool = fresh_test_pool().await;
        let info = WindowInfo {
            app_name: "Code".into(),
            title: "main.rs".into(),
            app_path: None,
            pid: 0,
        };
        let captured = Local::now();
        let id = insert_new(&pool, &info, captured, None, false)
            .await
            .unwrap();

        seal_session(&pool, id, captured - Duration::seconds(120))
            .await
            .unwrap();

        let (ended_at, dur) = read_ended_and_duration(&pool, id).await;
        assert_eq!(dur, 0, "负跨度必须钳 0");
        assert_eq!(
            ended_at,
            captured.to_rfc3339(),
            "ended_at 应钳回 started_at，不能小于它"
        );
    }

    /// 插一行 sealed 活动并带 screenshot_path，返回 rowid。给清理测试造数据用。
    async fn insert_shot_row(pool: &DbPool, local_date: &str, path: Option<String>) -> i64 {
        let local_date = local_date.to_string();
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activities(
                        started_at, ended_at, duration_secs, local_date, local_hour,
                        process_name, window_title, category_id, screenshot_path,
                        device_id, updated_at, origin
                     ) VALUES(
                        ?1 || 'T10:00:00Z', ?1 || 'T10:00:30Z', 30, ?1, 10,
                        'Code', '', 'other', ?2, ?3, ?1 || 'T10:00:30Z', 'local'
                     )",
                    rusqlite::params![local_date, path, TEST_SELF_ID],
                )
                .db()?;
                Ok(conn.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    async fn read_screenshot_path(pool: &DbPool, id: i64) -> Option<String> {
        pool.0
            .call(move |conn| {
                let p: Option<String> = conn
                    .query_row(
                        "SELECT screenshot_path FROM activities WHERE id = ?1",
                        rusqlite::params![id],
                        |r| r.get(0),
                    )
                    .db()?;
                Ok(p)
            })
            .await
            .unwrap()
    }

    async fn count_rows(pool: &DbPool) -> i64 {
        pool.0
            .call(|conn| {
                let n: i64 = conn
                    .query_row("SELECT COUNT(*) FROM activities", [], |r| r.get(0))
                    .db()?;
                Ok(n)
            })
            .await
            .unwrap()
    }

    async fn read_ended_and_duration(pool: &DbPool, id: i64) -> (String, i64) {
        pool.0
            .call(move |conn| {
                let row = conn
                    .query_row(
                        "SELECT ended_at, duration_secs FROM activities WHERE id = ?1",
                        rusqlite::params![id],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
                    )
                    .db()?;
                Ok(row)
            })
            .await
            .unwrap()
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
