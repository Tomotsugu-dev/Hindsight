//! Push 路径：把 sync_outbox 翻译成"哪些 device-scoped 文件需要重写"，每个 dirty key
//! 调一次 build_* 全量重新生成 JSON / NDJSON 内容，再 upload 到 Drive。

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use super::io::{self, OutboxRow};
use super::Inner;
use crate::error::{Error, Result};
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};
use crate::sync::cloud::FailureKind;
use crate::sync::file_name::{FileKind, FileName};
use crate::sync::payload::{
    ActivityPayload, AppGroupMemberPayload, AppGroupPayload, AppIconPayload, CategoryPayload,
    DeviceMetaPayload,
};

const PUSH_BATCH_SIZE: usize = 200;

pub(super) async fn flush_push(inner: &Arc<Inner>) -> Result<()> {
    // 串行门：与另一个 flush_push（「立即同步」vs 后台 tick）以及 purge 类命令互斥。
    // 并发 push 的危害：慢的一方用旧表内容覆盖 Drive、而新行的 outbox 已被快的一方
    // 删掉 → 那批数据到不了云端；与 purge 并发：读完 outbox 后表被清 → 空内容上云。
    let _gate = inner.flush_gate.lock().await;
    // Not signed in is not a failure: there is nothing to push.
    if !inner.cloud().ensure_credential().await? {
        return Ok(());
    }
    let round = push_round(inner).await;
    let ended = inner.cloud().end_push_round().await;
    round.and(ended)
}

async fn push_round(inner: &Arc<Inner>) -> Result<()> {
    // TODO(ADR-0003, ADR-0004): remove once active devices have upgraded past the
    // releases that stopped publishing these files.
    if !inner
        .legacy_cloud_files_checked
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        match delete_legacy_cloud_files(inner).await {
            Ok(()) => inner
                .legacy_cloud_files_checked
                .store(true, std::sync::atomic::Ordering::SeqCst),
            Err(e) => {
                log::warn!("push: old cloud files not removed, retrying next tick: {e}")
            }
        }
    }

    // 可选上云数据集(AI 总结/聊天历史/屏幕记忆):水位线检测,与 outbox 无关,
    // 放在 outbox 早退之前保证每轮 push tick 都有机会跑到。
    match crate::repo::settings::load(&inner.pool).await {
        Ok(cfg) => {
            if let Err(e) = super::datasets::push_optional(inner, &cfg).await {
                log::warn!("push 可选数据集失败: {e}");
            }
        }
        Err(e) => log::warn!("push 可选数据集跳过(读设置失败): {e}"),
    }

    let rows = io::read_due_outbox(&inner.pool, PUSH_BATCH_SIZE).await?;
    if rows.is_empty() {
        return Ok(());
    }

    // 把 outbox 行分组到"脏文件"
    let (groups, ungroupable_ids) = group_outbox(&rows);
    // 没法分组的行（entity 未知 / payload 损坏）立刻 drop：它们永远不可能发出去，
    // 留着会每个 tick 重读一次、占 batch 名额、把 pending 计数永久顶高
    if !ungroupable_ids.is_empty() {
        io::delete_outbox_rows(&inner.pool, &ungroupable_ids).await?;
    }
    if groups.is_empty() {
        return Ok(());
    }

    let self_id = inner.self_id.as_str();
    if self_id.is_empty() {
        log::debug!("sync push 跳过：self_id 为空（device 未初始化）");
        return Ok(());
    }
    let mut succeeded_ids: Vec<i64> = Vec::new();
    let mut failed_ids: Vec<i64> = Vec::new();
    let mut last_err: Option<Error> = None;

    for (kind, ids) in groups {
        let name = FileName {
            device_id: self_id.to_string(),
            kind: kind.clone(),
        }
        .to_file_name();
        let content = match build_content(&inner.pool, self_id, &kind).await {
            Ok(c) => c,
            Err(e) => {
                log::warn!("生成 {} 内容失败: {e}", name);
                failed_ids.extend(&ids);
                last_err = Some(e);
                continue;
            }
        };
        let upsert_res = inner.cloud().upsert_by_name(&name, &content).await;
        match upsert_res {
            Ok(_) => succeeded_ids.extend(&ids),
            Err(e) => {
                log::warn!("上传 {} 失败: {e}", name);
                failed_ids.extend(&ids);
                // A failure the user has to fix (out of space, expired account,
                // invalid credential) stops the other files too: end the round
                // here instead of sending requests that will fail.
                let needs_user = inner.cloud().failure_kind(&e) != FailureKind::Transient;
                last_err = Some(e);
                if needs_user {
                    break;
                }
            }
        }
    }

    if !succeeded_ids.is_empty() {
        io::delete_outbox_rows(&inner.pool, &succeeded_ids).await?;
        let mut s = inner.status.write().await;
        s.last_pushed_at = Some(utc_now_rfc3339());
        s.last_error = None;
    }

    if let Some(e) = last_err {
        // Only a failure the next round may fix on its own counts as a retry.
        // Counting the others would move these rows to dead letters after 10
        // rounds, and they would never upload even after the user fixes it.
        if inner.cloud().failure_kind(&e) == FailureKind::Transient {
            io::bump_outbox_retry(&inner.pool, &failed_ids, &e.to_string()).await?;
        }
        return Err(e);
    }

    log::info!("sync push 成功，共 {} 行 outbox 出队", succeeded_ids.len());
    Ok(())
}

/// Cloud files this device no longer publishes, by kind: `app_categories`
/// (ADR-0004) and `process_paths` (ADR-0003). Older versions left them behind,
/// nothing reads them any more, and they keep process names and paths the user
/// may since have removed.
const LEGACY_CLOUD_FILE_KINDS: [&str; 2] = ["app_categories", "process_paths"];

/// Deletes this device's copies of the files in [`LEGACY_CLOUD_FILE_KINDS`] from
/// the cloud. Only this device's copies are touched: every device cleans up its
/// own files once it upgrades.
///
/// TODO(ADR-0003, ADR-0004): remove once active devices have upgraded.
async fn delete_legacy_cloud_files(inner: &Arc<Inner>) -> Result<()> {
    if inner.self_id.is_empty() {
        return Ok(());
    }
    let names: Vec<String> = LEGACY_CLOUD_FILE_KINDS
        .iter()
        .map(|kind| format!("device.{}.{kind}.json", inner.self_id))
        .collect();
    let files = inner.cloud().list_all().await?;
    for file in files.into_iter().filter(|f| names.contains(&f.name)) {
        inner.cloud().delete(&file.id).await?;
        log::info!("push: removed {} from the cloud", file.name);
    }
    Ok(())
}

/// Outbox 行按它们要重写的文件分组：同一个文件只重写一次。
/// 返回 (可分组的脏文件映射, 没法分组必须直接 drop 的行 id)。
/// 没法分组的行如果不删，会在每个 batch 里既进不了 succeeded 也进不了 failed，
/// 永远留在 outbox（读取按 id ASC，它们还总排在最前面）。
fn group_outbox(rows: &[OutboxRow]) -> (HashMap<FileKind, Vec<i64>>, Vec<i64>) {
    let mut groups: HashMap<FileKind, Vec<i64>> = HashMap::new();
    let mut ungroupable: Vec<i64> = Vec::new();
    for row in rows {
        let key = match row.entity.as_str() {
            "activity" => match serde_json::from_str::<Value>(&row.payload)
                .ok()
                .and_then(|p| {
                    p.get("localDate")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<chrono::NaiveDate>().ok())
                }) {
                Some(d) => FileKind::Activities(d),
                None => {
                    log::warn!("outbox row {} has no usable localDate", row.id);
                    ungroupable.push(row.id);
                    continue;
                }
            },
            "category" => FileKind::Categories,
            "device" => FileKind::DeviceMeta,
            "app_icon" => FileKind::AppIcons,
            "app_group" => FileKind::AppGroups,
            "app_group_member" => FileKind::AppGroupMembers,
            _ => {
                log::warn!("outbox row {} entity 未知: {}", row.id, row.entity);
                ungroupable.push(row.id);
                continue;
            }
        };
        groups.entry(key).or_default().push(row.id);
    }
    (groups, ungroupable)
}

/// The whole content of one outbox-driven file, built from the tables. The
/// optional datasets are not outbox-driven; `datasets.rs` builds those.
async fn build_content(pool: &DbPool, self_id: &str, kind: &FileKind) -> Result<Vec<u8>> {
    match kind {
        FileKind::Activities(date) => build_activities_day(pool, self_id, &date.to_string()).await,
        FileKind::Categories => build_categories(pool).await,
        FileKind::DeviceMeta => build_device_meta(pool, self_id).await,
        FileKind::AppIcons => build_app_icons(pool).await,
        FileKind::AppGroups => build_app_groups(pool).await,
        FileKind::AppGroupMembers => build_app_group_members(pool).await,
        FileKind::Tombstone | FileKind::AiSummaries | FileKind::Chat | FileKind::Memory(_) => {
            Err(Error::Other(format!("{kind:?} is not an outbox file")))
        }
    }
}

async fn build_activities_day(pool: &DbPool, self_id: &str, day: &str) -> Result<Vec<u8>> {
    let self_id = self_id.to_string();
    let day = day.to_string();
    let rows: Vec<ActivityPayload> = pool
        .0
        .call(move |conn| {
            // By the `device_id` column only: every row of this device goes into the
            // day file, and no peer's row is pushed back.
            let mut stmt = conn
                .prepare(
                    "SELECT id, started_at, ended_at, duration_secs, local_date, local_hour,
                            process_name, window_title, category_id, updated_at, url_host
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
                        url_host: r.get(10)?,
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
/// 5 个共享表 (categories / process_paths / app_icons / app_groups /
/// app_group_members) 的 build_* 函数都是这一模板的实例化。
async fn build_table_rows<T, F>(pool: &DbPool, sql: &str, map: F) -> Result<Vec<u8>>
where
    T: serde::Serialize + Send + 'static,
    F: Fn(&rusqlite::Row) -> rusqlite::Result<T> + Send + Sync + 'static,
{
    // Owned copy so a `format!`-built query can cross into the 'static closure.
    let sql = sql.to_string();
    let rows: Vec<T> = pool
        .0
        .call(move |conn| {
            let mut stmt = conn.prepare(&sql).db()?;
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
    /// `FileKind::Activities(date)` 键，5 个 row id 全部进 value。
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
        let (groups, ungroupable) = group_outbox(&rows);

        assert_eq!(
            groups.len(),
            1,
            "5 行同一 local_date 应只产生 1 个 FileKind"
        );
        assert!(ungroupable.is_empty(), "合法行不应进 ungroupable");
        let ids = groups
            .get(&FileKind::Activities("2026-05-15".parse().unwrap()))
            .expect("Activities key should exist");
        let mut ids = ids.clone();
        ids.sort();
        assert_eq!(
            ids,
            vec![1, 2, 3, 4, 5],
            "所有 5 个 outbox row id 都应进入 value"
        );
    }

    /// 不同 local_date 的 outbox 行应进入不同的 FileKind 桶。
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
        let (groups, ungroupable) = group_outbox(&rows);
        assert_eq!(groups.len(), 2);
        assert!(ungroupable.is_empty(), "合法行不应进 ungroupable");
        let mut d1 = groups
            .get(&FileKind::Activities("2026-05-15".parse().unwrap()))
            .unwrap()
            .clone();
        d1.sort();
        assert_eq!(d1, vec![1, 3]);
        let d2 = groups
            .get(&FileKind::Activities("2026-05-16".parse().unwrap()))
            .unwrap()
            .clone();
        assert_eq!(d2, vec![2]);
    }

    /// 任务 6:[`build_device_meta`] 导出 devices 行的全部字段(尤其 os ——
    /// pull 侧的跨 OS 过滤完全依赖它),并用 pull 同款 DTO
    /// [`DeviceMetaPayload`] 反序列化,钉死字段名往返一致(camelCase 契约)。
    #[tokio::test]
    async fn build_device_meta_exports_all_fields_roundtrip() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|conn| {
                conn.execute(
                    "INSERT INTO devices(device_id, display_name, color, icon, os,
                                         last_seen_at, is_self, updated_at, deleted_at)
                     VALUES(?1, '我的 Mac', '#a1b2c3', 'Laptop', 'macos',
                            '2026-07-01T08:00:00Z', 1, '2026-07-02T09:00:00Z', NULL)",
                    rusqlite::params![crate::repo::test_util::TEST_SELF_ID],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();

        let body = build_device_meta(&pool, TEST_SELF_ID).await.unwrap();
        let meta: DeviceMetaPayload = serde_json::from_slice(&body)
            .expect("meta.json 应能被 pull 同款 DeviceMetaPayload 解析");

        assert_eq!(meta.device_id, TEST_SELF_ID);
        assert_eq!(meta.display_name, "我的 Mac");
        assert_eq!(meta.color, "#a1b2c3");
        assert_eq!(meta.icon, "Laptop");
        assert_eq!(meta.os.as_deref(), Some("macos"), "os 随 meta 一起导出");
        assert_eq!(meta.last_seen_at.as_deref(), Some("2026-07-01T08:00:00Z"));
        assert_eq!(meta.updated_at, "2026-07-02T09:00:00Z");
    }

    /// devices 无本机行(device::ensure_loaded 尚未跑)时导出空对象 `{}` ——
    /// pull 侧 merge_device_meta 对空对象是跳过语义,不会误插一行空 meta。
    #[tokio::test]
    async fn build_device_meta_returns_empty_object_without_row() {
        let pool = fresh_test_pool().await;
        let body = build_device_meta(&pool, TEST_SELF_ID).await.unwrap();
        assert_eq!(body, b"{}".to_vec(), "无 devices 行应返回空 JSON 对象");
    }
}
