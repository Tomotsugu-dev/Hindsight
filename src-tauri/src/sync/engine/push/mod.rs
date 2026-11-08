//! Push: rewrites this device's changed files in the cloud, each one whole.
//!
//! The outbox says which files to rewrite: each row points to one file, and a
//! round rewrites a file only once. `push_optional` compares fingerprints to
//! decide the optional datasets. Deleting sessions from a screen-memory day may
//! not change the fingerprint, so the outbox marks that day dirty.
//!
//! Each kind of file has a submodule that reads its whole content from the local
//! tables. A submodule only reads tables: it does not touch the cloud, the
//! outbox or the cursors.

mod activities;
mod ai_summaries;
mod app_group_members;
mod app_groups;
mod app_icons;
mod categories;
mod chat;
mod device_meta;
mod memory;
mod site_rules;
mod super_categories;

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use super::io::{self, OutboxRow};
use super::Inner;
use crate::error::{Error, Result};
use crate::memory::MemoryDb;
use crate::repo::outbox::OutboxEntity;
use crate::repo::settings::Settings;
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};
use crate::sync::cloud::FailureKind;
use crate::sync::file_name::{Dataset, FileKind, FileName};

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

    let cfg = crate::repo::settings::load(&inner.pool).await?;
    // 可选上云数据集(AI 总结/聊天历史/屏幕记忆):水位线检测,
    // 放在 outbox 早退之前保证每轮 push tick 都有机会跑到。
    if let Err(e) = push_optional(inner, &cfg).await {
        log::warn!("push 可选数据集失败: {e}");
    }

    let rows = io::read_due_outbox(&inner.pool, PUSH_BATCH_SIZE).await?;
    if rows.is_empty() {
        return Ok(());
    }

    // 把 outbox 行分组到"脏文件"
    let (mut groups, mut dropped_ids) = group_outbox(&rows);
    let enabled = super::enabled_datasets(&cfg, inner.mem.is_some());
    groups.retain(|kind, ids| {
        let on = enabled.contains(&kind.dataset());
        if !on {
            dropped_ids.append(ids);
        }
        on
    });
    // Drop the rows that will not be uploaded: rows nothing recognizes, and rows of
    // a dataset that is switched off. Kept, they would be read again every round,
    // take places in each batch, and keep "pending" on the Devices page from going
    // down.
    if !dropped_ids.is_empty() {
        io::delete_outbox_rows(&inner.pool, &dropped_ids).await?;
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
        let content = match build_content(&inner.pool, inner.mem.as_ref(), self_id, &kind).await {
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
                // here instead of sending requests that will fail. So does a
                // server that asks us to slow down (ServerBusy).
                let stop = inner.cloud().failure_kind(&e) != FailureKind::Transient;
                last_err = Some(e);
                if stop {
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
        match from_outbox(row) {
            Some(kind) => groups.entry(kind).or_default().push(row.id),
            None => ungroupable.push(row.id),
        }
    }
    (groups, ungroupable)
}

/// Returns the file that `row` marks dirty, or `None` when its entity is
/// unknown or an activity or memory row has no usable `localDate`.
fn from_outbox(row: &OutboxRow) -> Option<FileKind> {
    let Some(entity) = OutboxEntity::parse(&row.entity) else {
        log::warn!("outbox row {} entity 未知: {}", row.id, row.entity);
        return None;
    };
    let kind = match entity {
        OutboxEntity::Activity => FileKind::Activities(local_date(row)?),
        OutboxEntity::Memory => FileKind::Memory(local_date(row)?),
        OutboxEntity::Category => FileKind::Categories,
        OutboxEntity::SuperCategory => FileKind::SuperCategories,
        OutboxEntity::SiteRule => FileKind::SiteRules,
        OutboxEntity::Device => FileKind::DeviceMeta,
        OutboxEntity::AppIcon => FileKind::AppIcons,
        OutboxEntity::AppGroup => FileKind::AppGroups,
        OutboxEntity::AppGroupMember => FileKind::AppGroupMembers,
    };
    Some(kind)
}

/// Reads which day a day-file row points to: the `localDate` in its payload.
fn local_date(row: &OutboxRow) -> Option<chrono::NaiveDate> {
    let date = serde_json::from_str::<Value>(&row.payload)
        .ok()
        .and_then(|p| p.get("localDate")?.as_str()?.parse().ok());
    if date.is_none() {
        log::warn!("outbox row {} has no usable localDate", row.id);
    }
    date
}

/// Builds the whole content of a file the outbox marked dirty, from the tables.
/// A screen-memory day is built from `mem`. AI summaries and chat do not go
/// through the outbox; `push_optional` builds them.
async fn build_content(
    pool: &DbPool,
    mem: Option<&MemoryDb>,
    self_id: &str,
    kind: &FileKind,
) -> Result<Vec<u8>> {
    match kind {
        FileKind::Activities(date) => activities::build(pool, self_id, &date.to_string()).await,
        FileKind::Categories => categories::build(pool).await,
        FileKind::SuperCategories => super_categories::build(pool).await,
        FileKind::SiteRules => site_rules::build(pool).await,
        FileKind::DeviceMeta => device_meta::build(pool, self_id).await,
        FileKind::AppIcons => app_icons::build(pool).await,
        FileKind::AppGroups => app_groups::build(pool).await,
        FileKind::AppGroupMembers => app_group_members::build(pool).await,
        FileKind::Memory(date) => match mem {
            Some(mem) => memory::build(mem, &date.to_string()).await,
            None => Err(Error::Other("the memory database is not open".into())),
        },
        FileKind::Tombstone | FileKind::AiSummaries | FileKind::Chat => {
            Err(Error::Other(format!("{kind:?} is not an outbox file")))
        }
    }
}

/// 把一张表全量 SELECT 出来 → 每行映射成 `T` → 整体序列化成 JSON 字节。
///
// TODO: the list below is out of date: process_paths is no longer synced
// (ADR-0003), and site_rules and super_categories use this too.
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

/// 每轮 push 调一次:对启用的数据集做水位线检测,变了才上传。
/// 单个数据集失败只记 warn(下一轮水位线仍不同会重试),不阻塞其它数据集。
///
/// 与 outbox 驱动的核心数据集不同,这三类走**水位线检测**:
/// 每轮 push 先算数据集当前水位(最大时间戳+行数),与 sync_cursor 里存的
/// 上次推送水位比对,变了才全量重建文件上传——记忆库没有 outbox,
/// ai_summaries 也明确不入 outbox,这条路径对两边一视同仁。
///
/// 开关语义:settings 三挡分别门控各自数据集的**推与拉**。拉取时每一类有自己的
/// 游标(`pull.ai_summaries` / `pull.chat` / `pull.memory`,见 ADR-0006):开关关着
/// 那一类不拉、游标不动,打开后从停下的地方接着拉,历史文件自然补上。
async fn push_optional(inner: &Arc<Inner>, cfg: &Settings) -> Result<()> {
    let self_id = inner.self_id.as_str();
    if self_id.is_empty() {
        return Ok(());
    }

    for dataset in super::enabled_datasets(cfg, inner.mem.is_some()) {
        let res = match dataset {
            // The outbox drives these; push_round uploads them.
            Dataset::Core | Dataset::SiteRules | Dataset::SuperCategories => continue,
            Dataset::AiSummaries => push_ai_summaries(inner).await,
            Dataset::Chat => push_chat(inner, inner.mem.as_ref().expect("gated")).await,
            Dataset::Memory => push_memory(inner, inner.mem.as_ref().expect("gated")).await,
        };
        if let Err(e) = res {
            log::warn!("push {} 失败: {e}", dataset.name());
        }
    }
    Ok(())
}

/// Uploads one of this device's files.
async fn upload(inner: &Arc<Inner>, kind: FileKind, content: Vec<u8>) -> Result<()> {
    let name = FileName {
        device_id: inner.self_id.clone(),
        kind,
    }
    .to_file_name();
    inner.cloud().upsert_by_name(&name, &content).await?;
    Ok(())
}

async fn push_ai_summaries(inner: &Arc<Inner>) -> Result<()> {
    let watermark = ai_summaries::fingerprint(&inner.pool).await?;
    let name = inner.cloud().push_fingerprint_name(Dataset::AiSummaries);
    if io::read_cursor(&inner.pool, &name).await? == watermark {
        return Ok(());
    }
    let content = ai_summaries::build(&inner.pool).await?;
    upload(inner, FileKind::AiSummaries, content).await?;
    io::write_cursor(&inner.pool, &name, &watermark).await?;
    log::info!("push ai_summaries: uploaded");
    Ok(())
}

async fn push_chat(inner: &Arc<Inner>, mem: &MemoryDb) -> Result<()> {
    let watermark = chat::fingerprint(mem).await?;
    let name = inner.cloud().push_fingerprint_name(Dataset::Chat);
    if io::read_cursor(&inner.pool, &name).await? == watermark {
        return Ok(());
    }
    let content = chat::build(mem).await?;
    upload(inner, FileKind::Chat, content).await?;
    io::write_cursor(&inner.pool, &name, &watermark).await?;
    log::info!("push chat: uploaded");
    Ok(())
}

async fn push_memory(inner: &Arc<Inner>, mem: &MemoryDb) -> Result<()> {
    let watermark = memory::fingerprint(mem).await?;
    let name = inner.cloud().push_fingerprint_name(Dataset::Memory);
    let prev = io::read_cursor(&inner.pool, &name).await?;
    if prev == watermark || watermark.is_empty() {
        return Ok(());
    }
    let days = memory::changed_days(mem, prev).await?;
    for day in &days {
        let Ok(date) = day.parse::<chrono::NaiveDate>() else {
            log::warn!("push memory: local_date {day:?} is not a date, skipped");
            continue;
        };
        let content = memory::build(mem, day).await?;
        upload(inner, FileKind::Memory(date), content).await?;
    }
    io::write_cursor(&inner.pool, &name, &watermark).await?;
    if !days.is_empty() {
        log::info!("push memory: {} 天的会话文件", days.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};
    use crate::sync::payload::SuperCategoryPayload;

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

    #[tokio::test]
    async fn build_super_categories_exports_rows_with_tombstones() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|conn| {
                conn.execute(
                    "INSERT INTO super_categories(id, name, color, icon, sort_order, updated_at, deleted_at)
                     VALUES('zz', 'Gone', '#000000', 'Box', 9, '2026-06-01T00:00:00Z', '2026-06-01T00:00:00Z')",
                    [],
                )
                .unwrap();
                Ok(())
            })
            .await
            .unwrap();
        let bytes = build_content(&pool, None, TEST_SELF_ID, &FileKind::SuperCategories)
            .await
            .unwrap();
        let rows: Vec<SuperCategoryPayload> = serde_json::from_slice(&bytes).unwrap();
        assert!(rows
            .iter()
            .any(|r| r.id == "work" && r.deleted_at.is_none()));
        let gone = rows.iter().find(|r| r.id == "zz").unwrap();
        assert_eq!(gone.deleted_at.as_deref(), Some("2026-06-01T00:00:00Z"));
    }

    /// 每种 outbox 行都要找得到它要重写的文件。找不到的行会被当成认不出的删掉，改动就丢了。
    #[test]
    fn every_outbox_entity_maps_to_a_file_kind() {
        for entity in OutboxEntity::ALL {
            let row = OutboxRow {
                id: 1,
                entity: entity.as_str().into(),
                payload: serde_json::json!({ "localDate": "2026-05-15" }).to_string(),
            };
            assert!(from_outbox(&row).is_some(), "{entity:?} 没有对应的文件");
        }
    }

    #[test]
    fn group_outbox_routes_super_category() {
        let rows = vec![OutboxRow {
            id: 1,
            entity: "super_category".into(),
            payload: "{}".into(),
        }];
        let (groups, bad) = group_outbox(&rows);
        assert!(bad.is_empty());
        assert_eq!(groups.get(&FileKind::SuperCategories), Some(&vec![1]));
    }
}
