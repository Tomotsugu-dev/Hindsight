//! Push 路径：把 sync_outbox 翻译成"哪些 device-scoped 文件需要重写"，每个 dirty key
//! 调一次 build_* 全量重新生成 JSON / NDJSON 内容，再 upload 到 Drive。

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use super::io::{self, OutboxRow};
use super::{format_sync_error, with_token_retry, Inner};
use crate::error::{Error, Result};
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};
use crate::sync::auth::{self, TokenInfo};
use crate::sync::payload::{
    ActivityPayload, AppCategoryPayload, AppGroupMemberPayload, AppGroupPayload, AppIconPayload,
    CategoryPayload, DeviceMetaPayload, ProcessPathPayload,
};

const PUSH_BATCH_SIZE: usize = 200;

/// Outbox entity 行翻成 dirty key —— 同一个 dirty key 触发对应文件的全量重写。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum DirtyKey {
    ActivityDay(String), // local_date
    Categories,
    AppCategories,
    ProcessPaths,
    DeviceMeta,
    AppIcons,
    AppGroups,
    AppGroupMembers,
}

pub(super) async fn flush_push(inner: &Arc<Inner>) -> Result<()> {
    let mut token: TokenInfo = match auth::ensure_valid_token(&inner.pool).await {
        Ok(t) => t,
        // NotSignedIn 是预期状态，不当错误显示；其它（续期失败 / refresh_token 失效）让用户看见
        Err(Error::NotSignedIn) => {
            log::debug!("sync 跳过 push（未登录）");
            return Ok(());
        }
        Err(e) => {
            // 默认日志级别只记概述：避免把 OAuth body / 错误链里可能携带的
            // 用户邮箱 / account id / OAuth body 落到日志文件；
            // 错误分类后的 status 字符串已经带 [CRED_EXPIRED] / [TRANSIENT] 前缀，
            // UI 可读且不含原始 raw。排查时用 RUST_LOG=hindsight=debug 看 detail
            log::warn!("sync push 拿不到有效 token（详情见 status）");
            log::debug!("token error detail: {e}");
            inner.status.write().await.last_error = Some(format_sync_error(&e));
            return Ok(());
        }
    };

    let rows = io::read_due_outbox(&inner.pool, PUSH_BATCH_SIZE).await?;
    if rows.is_empty() {
        return Ok(());
    }

    // 把 outbox 行分组到"脏文件"
    let groups = group_outbox(&rows);
    if groups.is_empty() {
        // 所有行都没法分组（entity 未知）→ 全部 drop
        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        io::delete_outbox_rows(&inner.pool, &ids).await?;
        return Ok(());
    }

    let self_id = inner.self_id.as_str();
    if self_id.is_empty() {
        log::debug!("sync push 跳过：self_id 为空（device 未初始化）");
        return Ok(());
    }
    let mut succeeded_ids: Vec<i64> = Vec::new();
    let mut failed_ids: Vec<i64> = Vec::new();
    let mut last_err_raw: Option<crate::error::Error> = None;
    let mut last_err_str: Option<String> = None;

    for (key, ids) in groups {
        let name = file_name_for(self_id, &key);
        let content = match build_content(&inner.pool, self_id, &key).await {
            Ok(c) => c,
            Err(e) => {
                log::warn!("生成 {} 内容失败: {e}", name);
                failed_ids.extend(&ids);
                last_err_str = Some(e.to_string());
                last_err_raw = Some(e);
                continue;
            }
        };
        let upsert_res = with_token_retry(&inner.pool, &mut token, |tok| {
            let name = name.clone();
            let content = content.clone();
            let drive = &inner.drive;
            async move { drive.upsert_by_name(&tok, &name, &content).await }
        })
        .await;
        match upsert_res {
            Ok(_) => succeeded_ids.extend(&ids),
            Err(e) => {
                log::warn!("上传 {} 失败: {e}", name);
                failed_ids.extend(&ids);
                last_err_str = Some(e.to_string());
                last_err_raw = Some(e);
            }
        }
    }

    if !succeeded_ids.is_empty() {
        io::delete_outbox_rows(&inner.pool, &succeeded_ids).await?;
        let mut s = inner.status.write().await;
        s.last_pushed_at = Some(utc_now_rfc3339());
        s.last_error = None;
    }

    if !failed_ids.is_empty() {
        let err_str = last_err_str.clone().unwrap_or_else(|| "未知错误".into());
        io::bump_outbox_retry(&inner.pool, &failed_ids, &err_str).await?;
        if let Some(ref e) = last_err_raw {
            inner.status.write().await.last_error = Some(format_sync_error(e));
        }
        return Err(Error::SyncIncomplete(
            last_err_str.unwrap_or_else(|| "push 失败".into()),
        ));
    }

    log::info!("sync push 成功，共 {} 行 outbox 出队", succeeded_ids.len());
    Ok(())
}

fn group_outbox(rows: &[OutboxRow]) -> HashMap<DirtyKey, Vec<i64>> {
    let mut groups: HashMap<DirtyKey, Vec<i64>> = HashMap::new();
    for row in rows {
        let key = match row.entity.as_str() {
            "activity" => match serde_json::from_str::<Value>(&row.payload)
                .ok()
                .and_then(|p| {
                    p.get("localDate")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                }) {
                Some(d) => DirtyKey::ActivityDay(d),
                None => {
                    log::warn!("outbox row {} 是 activity 但 payload 缺 localDate", row.id);
                    continue;
                }
            },
            "category" => DirtyKey::Categories,
            "app_category" => DirtyKey::AppCategories,
            "process_path" => DirtyKey::ProcessPaths,
            "device" => DirtyKey::DeviceMeta,
            "app_icon" => DirtyKey::AppIcons,
            "app_group" => DirtyKey::AppGroups,
            "app_group_member" => DirtyKey::AppGroupMembers,
            _ => {
                log::warn!("outbox row {} entity 未知: {}", row.id, row.entity);
                continue;
            }
        };
        groups.entry(key).or_default().push(row.id);
    }
    groups
}

fn file_name_for(self_id: &str, key: &DirtyKey) -> String {
    match key {
        DirtyKey::ActivityDay(day) => format!("device.{self_id}.activities.{day}.ndjson"),
        DirtyKey::Categories => format!("device.{self_id}.categories.json"),
        DirtyKey::AppCategories => format!("device.{self_id}.app_categories.json"),
        DirtyKey::ProcessPaths => format!("device.{self_id}.process_paths.json"),
        DirtyKey::DeviceMeta => format!("device.{self_id}.meta.json"),
        DirtyKey::AppIcons => format!("device.{self_id}.icons.json"),
        DirtyKey::AppGroups => format!("device.{self_id}.app_groups.json"),
        DirtyKey::AppGroupMembers => format!("device.{self_id}.app_group_members.json"),
    }
}

async fn build_content(pool: &DbPool, self_id: &str, key: &DirtyKey) -> Result<Vec<u8>> {
    match key {
        DirtyKey::ActivityDay(day) => build_activities_day(pool, self_id, day).await,
        DirtyKey::Categories => build_categories(pool).await,
        DirtyKey::AppCategories => build_app_categories(pool).await,
        DirtyKey::ProcessPaths => build_process_paths(pool).await,
        DirtyKey::DeviceMeta => build_device_meta(pool, self_id).await,
        DirtyKey::AppIcons => build_app_icons(pool).await,
        DirtyKey::AppGroups => build_app_groups(pool).await,
        DirtyKey::AppGroupMembers => build_app_group_members(pool).await,
    }
}

async fn build_activities_day(pool: &DbPool, self_id: &str, day: &str) -> Result<Vec<u8>> {
    let self_id = self_id.to_string();
    let day = day.to_string();
    let rows: Vec<ActivityPayload> = pool
        .0
        .call(move |conn| {
            // 没有 `AND origin = 'local'` 过滤 —— 取 device_id=self 的**全部**行，
            // 包含 origin='local'（本机直接 capture）和 origin='remote'（mac 自己 purge
            // 之后 pull 自己 Drive 文件恢复回来的行，按 [v26 migration] + 移除 self-skip
            // 后的设计也会用 device_id=self）。少了 origin 过滤后 push 不会把 pull-back
            // 来的本机历史"漏掉"，整张 ndjson 重写仍然是本机视角下完整的"我贡献过的"集合。
            // 对端镜像 `device_id != self` 被 WHERE device_id=?1 已经排除，不会回推。
            let mut stmt = conn
                .prepare(
                    "SELECT id, started_at, ended_at, duration_secs, local_date, local_hour,
                            process_name, window_title, category_id, updated_at
                     FROM activities
                     WHERE device_id = ?1 AND local_date = ?2
                     ORDER BY id",
                )
                .db()?;
            let rows = stmt
                .query_map(rusqlite::params![self_id, day], |r| {
                    Ok(ActivityPayload {
                        id: r.get(0)?,
                        started_at: r.get(1)?,
                        ended_at: r.get(2)?,
                        duration_secs: r.get(3)?,
                        local_date: r.get(4)?,
                        local_hour: r.get(5)?,
                        process_name: r.get(6)?,
                        window_title: r.get(7)?,
                        category_id: r.get(8)?,
                        updated_at: r.get(9)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await?;

    let mut out = Vec::with_capacity(rows.len() * 200);
    for row in &rows {
        let s = serde_json::to_string(row)?;
        out.extend_from_slice(s.as_bytes());
        out.push(b'\n');
    }
    Ok(out)
}

/// 把一张表全量 SELECT 出来 → 每行映射成 `T` → 整体序列化成 JSON 字节。
///
/// 6 个共享表 (categories / app_categories / process_paths / app_icons /
/// app_groups / app_group_members) 的 build_* 函数都是这一模板的实例化。
async fn build_table_rows<T, F>(pool: &DbPool, sql: &'static str, map: F) -> Result<Vec<u8>>
where
    T: serde::Serialize + Send + 'static,
    F: Fn(&rusqlite::Row) -> rusqlite::Result<T> + Send + Sync + 'static,
{
    let rows: Vec<T> = pool
        .0
        .call(move |conn| {
            let mut stmt = conn.prepare(sql).db()?;
            let rows = stmt
                .query_map([], |r| map(r))
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await?;
    Ok(serde_json::to_vec(&rows)?)
}

async fn build_categories(pool: &DbPool) -> Result<Vec<u8>> {
    build_table_rows(
        pool,
        "SELECT id, name, color, icon, builtin, sort_order, updated_at, deleted_at
         FROM categories ORDER BY id",
        |r| {
            Ok(CategoryPayload {
                id: r.get(0)?,
                name: r.get(1)?,
                color: r.get(2)?,
                icon: r.get(3)?,
                builtin: r.get::<_, i64>(4)? != 0,
                sort_order: r.get(5)?,
                updated_at: r.get(6)?,
                deleted_at: r.get(7)?,
            })
        },
    )
    .await
}

async fn build_app_categories(pool: &DbPool) -> Result<Vec<u8>> {
    build_table_rows(
        pool,
        "SELECT process_name, category_id, updated_at, deleted_at
         FROM app_categories ORDER BY process_name",
        |r| {
            Ok(AppCategoryPayload {
                process_name: r.get(0)?,
                category_id: r.get(1)?,
                updated_at: r.get(2)?,
                deleted_at: r.get(3)?,
            })
        },
    )
    .await
}

async fn build_process_paths(pool: &DbPool) -> Result<Vec<u8>> {
    build_table_rows(
        pool,
        "SELECT process_name, exe_path, seen_at, updated_at
         FROM process_paths ORDER BY process_name",
        |r| {
            Ok(ProcessPathPayload {
                process_name: r.get(0)?,
                exe_path: r.get(1)?,
                seen_at: r.get(2)?,
                updated_at: r.get(3)?,
            })
        },
    )
    .await
}

async fn build_app_icons(pool: &DbPool) -> Result<Vec<u8>> {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
    build_table_rows(
        pool,
        "SELECT process_name, icon_png, updated_at, deleted_at
         FROM app_icons ORDER BY process_name",
        |r| {
            let bytes: Vec<u8> = r.get(1)?;
            Ok(AppIconPayload {
                process_name: r.get(0)?,
                // BLOB → base64：JSON 不支持 binary，统一用 base64 标准编码
                icon_png_base64: BASE64.encode(&bytes),
                updated_at: r.get(2)?,
                deleted_at: r.get(3)?,
            })
        },
    )
    .await
}

async fn build_app_groups(pool: &DbPool) -> Result<Vec<u8>> {
    build_table_rows(
        pool,
        "SELECT id, display_name, category_id, updated_at, deleted_at
         FROM app_groups ORDER BY id",
        |r| {
            Ok(AppGroupPayload {
                id: r.get(0)?,
                display_name: r.get(1)?,
                category_id: r.get(2)?,
                updated_at: r.get(3)?,
                deleted_at: r.get(4)?,
            })
        },
    )
    .await
}

async fn build_app_group_members(pool: &DbPool) -> Result<Vec<u8>> {
    build_table_rows(
        pool,
        "SELECT process_name, group_id, updated_at, deleted_at
         FROM app_group_members ORDER BY process_name",
        |r| {
            Ok(AppGroupMemberPayload {
                process_name: r.get(0)?,
                group_id: r.get(1)?,
                updated_at: r.get(2)?,
                deleted_at: r.get(3)?,
            })
        },
    )
    .await
}

async fn build_device_meta(pool: &DbPool, self_id: &str) -> Result<Vec<u8>> {
    let self_id = self_id.to_string();
    let obj: Option<DeviceMetaPayload> = pool
        .0
        .call(move |conn| {
            let row = conn
                .query_row(
                    "SELECT device_id, display_name, color, icon, os, last_seen_at, updated_at
                     FROM devices WHERE device_id = ?1",
                    rusqlite::params![self_id],
                    |r| {
                        Ok(DeviceMetaPayload {
                            device_id: r.get(0)?,
                            display_name: r.get(1)?,
                            color: r.get(2)?,
                            icon: r.get(3)?,
                            os: r.get(4)?,
                            last_seen_at: r.get(5)?,
                            updated_at: r.get(6)?,
                        })
                    },
                )
                .ok();
            Ok(row)
        })
        .await?;
    let Some(meta) = obj else {
        return Ok(b"{}".to_vec());
    };
    Ok(serde_json::to_vec(&meta)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};
    use chrono::Local;

    /// 测 [`build_activities_day`] 的 device + date 过滤：
    /// - 只导出 `device_id = self_id` 的行
    /// - 跨日期不混（昨天的 self 行不能出现在今天的 ndjson）
    /// - 跨设备不混（对端 mirror 行不能被当作"本机贡献"重推）
    ///
    /// 这条防"mac 重推 Win 镜像数据"的 bug 重现。
    #[tokio::test]
    async fn build_activities_day_filters_by_device_and_date() {
        let pool = fresh_test_pool().await;
        let today = Local::now().format("%Y-%m-%d").to_string();
        let yesterday = (Local::now() - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();

        // 3 行 self + today，process_name 各不同
        for process in ["Code", "Chrome", "Slack"] {
            insert_sealed(&pool, TEST_SELF_ID, &today, process, "local").await;
        }
        // 2 行 other device + today，origin='remote'（mac pull 来的镜像 —— 不能被回推）
        for process in ["Win-only-A", "Win-only-B"] {
            insert_sealed(&pool, "device-other", &today, process, "remote").await;
        }
        // 1 行 self 但是昨天 —— 当天的 ndjson 不该含它
        insert_sealed(&pool, TEST_SELF_ID, &yesterday, "Yesterday-app", "local").await;

        let body = build_activities_day(&pool, TEST_SELF_ID, &today)
            .await
            .unwrap();

        let lines: Vec<&[u8]> = body
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(lines.len(), 3, "应仅导出 3 行 (self + today)");

        let processes: Vec<String> = lines
            .iter()
            .map(|l| {
                let p: ActivityPayload = serde_json::from_slice(l).unwrap();
                p.process_name
            })
            .collect();

        let expected: std::collections::HashSet<&str> =
            ["Code", "Chrome", "Slack"].into_iter().collect();
        let got: std::collections::HashSet<&str> = processes.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            got, expected,
            "导出的 process_name 集合应正好是 self+today 的 3 个"
        );

        // 显式断言对端 + 昨日的没漏出
        for forbidden in ["Win-only-A", "Win-only-B", "Yesterday-app"] {
            assert!(
                !processes.iter().any(|p| p == forbidden),
                "不应导出 process_name={forbidden}（要么跨设备要么跨日期）"
            );
        }
    }

    async fn insert_sealed(
        pool: &DbPool,
        device_id: &str,
        local_date: &str,
        process_name: &str,
        origin: &str,
    ) {
        let device_id = device_id.to_string();
        let local_date = local_date.to_string();
        let process_name = process_name.to_string();
        let origin = origin.to_string();
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activities(
                        started_at, ended_at, duration_secs, local_date, local_hour,
                        process_name, window_title, category_id, device_id, updated_at, origin
                     ) VALUES(
                        ?1 || 'T10:00:00Z', ?1 || 'T10:00:30Z', 30, ?1, 10,
                        ?2, '', 'other', ?3, ?1 || 'T10:00:30Z', ?4
                     )",
                    rusqlite::params![local_date, process_name, device_id, origin],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// 测 [`group_outbox`]：同一 local_date 的 5 条 outbox 应塌成 1 个
    /// `DirtyKey::ActivityDay(date)` 键，5 个 row id 全部进 value。
    /// 防"push 把同一天 ndjson 重写 5 次"的回归（早期 bug 引起 Drive quota 抖动）。
    #[test]
    fn group_outbox_collapses_same_local_date() {
        let make_row = |id: i64, date: &str| OutboxRow {
            id,
            entity: "activity".into(),
            payload: serde_json::json!({ "localDate": date }).to_string(),
        };
        let rows = vec![
            make_row(1, "2026-05-15"),
            make_row(2, "2026-05-15"),
            make_row(3, "2026-05-15"),
            make_row(4, "2026-05-15"),
            make_row(5, "2026-05-15"),
        ];
        let groups = group_outbox(&rows);

        assert_eq!(
            groups.len(),
            1,
            "5 行同一 local_date 应只产生 1 个 DirtyKey"
        );
        let ids = groups
            .get(&DirtyKey::ActivityDay("2026-05-15".into()))
            .expect("ActivityDay key should exist");
        let mut ids = ids.clone();
        ids.sort();
        assert_eq!(
            ids,
            vec![1, 2, 3, 4, 5],
            "所有 5 个 outbox row id 都应进入 value"
        );
    }

    /// 不同 local_date 的 outbox 行应进入不同的 DirtyKey 桶。
    #[test]
    fn group_outbox_splits_different_local_dates() {
        let make_row = |id: i64, date: &str| OutboxRow {
            id,
            entity: "activity".into(),
            payload: serde_json::json!({ "localDate": date }).to_string(),
        };
        let rows = vec![
            make_row(1, "2026-05-15"),
            make_row(2, "2026-05-16"),
            make_row(3, "2026-05-15"),
        ];
        let groups = group_outbox(&rows);
        assert_eq!(groups.len(), 2);
        let mut d1 = groups
            .get(&DirtyKey::ActivityDay("2026-05-15".into()))
            .unwrap()
            .clone();
        d1.sort();
        assert_eq!(d1, vec![1, 3]);
        let d2 = groups
            .get(&DirtyKey::ActivityDay("2026-05-16".into()))
            .unwrap()
            .clone();
        assert_eq!(d2, vec![2]);
    }
}
