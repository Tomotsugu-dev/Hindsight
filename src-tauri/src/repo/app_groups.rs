//! 跨设备应用配对 / 分类的核心数据层。
//!
//! 模型：
//!   app_groups          —— (id, display_name, category_id)，跨设备同步
//!   app_group_members   —— (process_name → group_id)，跨设备同步
//!   app_categories      —— (process_name → category_id)，旧表，作为 derived view 维护
//!
//! 不变量：
//!   - 每个出现过的 process_name 必有 (active) app_group_members 行
//!   - 初始 group_id == process_name（保证两台设备 backfill 出来 ID 一致）
//!   - app_groups.category_id 是 source of truth；变更时自动同步到 app_categories
//!     里所有成员的对应行（让旧的 reports.rs LEFT JOIN app_categories 继续工作）

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::repo::outbox::{enqueue, OutboxEntity, OutboxOp};
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

/// 应用组的对外快照（包含成员 + category_id + display_name）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppGroup {
    /// group_id（首次出现时等于该 process_name；后续 merge 后保持不变）
    pub id: String,
    /// 用户可见的展示名（如 "Visual Studio Code"）
    pub display_name: String,
    /// 该组的分类（None = 未分类）
    pub category_id: Option<String>,
    /// 组内成员（process_name + 时长 + 最后出现设备）
    pub members: Vec<AppGroupMember>,
}

/// 组内单个 process_name 成员的详情。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppGroupMember {
    pub process_name: String,
    /// 该成员近 7 天累计时长（秒），按 process_name 聚合，跨设备求和
    pub recent_secs: i64,
    /// 该成员最后一次出现的设备 ID（取最大 ended_at 那条）；UI 拿来分列
    pub last_device_id: Option<String>,
}

/// 列出所有未软删的组 + 成员 + 每个成员的近 7 天时长 + 最后出现设备。
/// 按组的 max(member.recent_secs) 降序，让活跃应用排前面。
pub async fn list_groups(pool: &DbPool) -> Result<Vec<AppGroup>> {
    let groups = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT g.id, g.display_name, g.category_id
                     FROM app_groups g
                     WHERE g.deleted_at IS NULL",
                )
                .db()?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })
                .db()?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.db()?);
            }

            // 一次性把所有未软删成员 + 时长统计拉出来；用 GROUP BY process_name 聚合活动时长
            let mut mstmt = conn
                .prepare(
                    "SELECT m.process_name, m.group_id,
                            COALESCE(s.total_secs, 0)   AS recent_secs,
                            s.last_device_id            AS last_device_id
                     FROM app_group_members m
                     LEFT JOIN (
                       SELECT a.process_name,
                              SUM(a.duration_secs)        AS total_secs,
                              -- 取该 process_name 最后一次活动所在的设备
                              (SELECT a2.device_id
                                 FROM activities a2
                                 WHERE a2.process_name = a.process_name
                                 ORDER BY a2.ended_at DESC LIMIT 1) AS last_device_id
                       FROM activities a
                       WHERE a.local_date >= date('now','localtime','-7 days')
                       GROUP BY a.process_name
                     ) s ON s.process_name = m.process_name
                     WHERE m.deleted_at IS NULL",
                )
                .db()?;
            let mit = mstmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,         // process_name
                        r.get::<_, String>(1)?,         // group_id
                        r.get::<_, i64>(2)?,            // recent_secs
                        r.get::<_, Option<String>>(3)?, // last_device_id
                    ))
                })
                .db()?;
            let mut members: Vec<(String, String, i64, Option<String>)> = Vec::new();
            for r in mit {
                members.push(r.db()?);
            }
            Ok((out, members))
        })
        .await?;

    let (group_rows, member_rows) = groups;

    let mut groups: Vec<AppGroup> = group_rows
        .into_iter()
        .map(|(id, display_name, category_id)| AppGroup {
            id,
            display_name,
            category_id,
            members: Vec::new(),
        })
        .collect();

    // 把成员塞进组里
    for (process_name, group_id, recent_secs, last_device_id) in member_rows {
        if let Some(g) = groups.iter_mut().find(|g| g.id == group_id) {
            g.members.push(AppGroupMember {
                process_name,
                recent_secs,
                last_device_id,
            });
        }
        // 成员指向不存在的 group（理论上不发生 —— REFERENCES 约束）就丢弃
    }

    // 按 group 内最大 recent_secs 降序
    groups.sort_by(|a, b| {
        let amax = a.members.iter().map(|m| m.recent_secs).max().unwrap_or(0);
        let bmax = b.members.iter().map(|m| m.recent_secs).max().unwrap_or(0);
        bmax.cmp(&amax)
    });

    Ok(groups)
}

/// 用户在 UI 主动建一个空组：随机 UUID 作 id，display_name 用户给。
/// enqueue outbox 让对端也拉到这个空组。返回新组的 id 给前端，方便后续操作。
///
/// 主要用途：误把 chip 拖进了别的组，源行被过滤光后想回到一个干净的目标行 —— 现在
/// 用户能主动新建，而不是依赖 capture loop 见到新 process_name 才被动 ensure_group。
pub async fn create(pool: &DbPool, display_name: &str) -> Result<String> {
    let name = display_name.trim().to_string();
    if name.is_empty() {
        return Err(crate::error::Error::InvalidInput("组名不能为空"));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let id_for_db = id.clone();
    let id_for_outbox = id.clone();
    pool.0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                 VALUES(?, ?, NULL, ?, NULL)",
                rusqlite::params![id_for_db, name, now],
            )
            .db()?;
            enqueue(
                conn,
                OutboxOp::Upsert,
                OutboxEntity::AppGroup,
                &id_for_outbox,
                &serde_json::json!({ "groupId": id_for_outbox }).to_string(),
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(id)
}

/// 软删一个**空组**。仅对 0 成员的组生效（有成员强制走 unmerge 路径，避免孤儿成员
/// 突然没有 group_id 可指）。enqueue outbox 让对端也把这个组从列表里去掉。
/// 幂等：组已被删 / 不存在 → no-op。
pub async fn delete(pool: &DbPool, group_id: &str) -> Result<()> {
    let id = group_id.to_string();
    let now = Utc::now().to_rfc3339();
    let outcome: std::result::Result<(), &'static str> = pool
        .0
        .call(move |conn| {
            let has_members: bool = conn
                .query_row(
                    "SELECT 1 FROM app_group_members
                     WHERE group_id = ?1 AND deleted_at IS NULL",
                    rusqlite::params![id],
                    |_| Ok(true),
                )
                .optional()
                .db()?
                .unwrap_or(false);
            if has_members {
                return Ok(Err("组内仍有成员，不能删除（先把成员拖出来）"));
            }
            let n = conn
                .execute(
                    "UPDATE app_groups SET deleted_at = ?1, updated_at = ?1
                     WHERE id = ?2 AND deleted_at IS NULL",
                    rusqlite::params![now, id],
                )
                .db()?;
            // n == 0 → 已经被删过 / 不存在；不入 outbox
            if n > 0 {
                enqueue(
                    conn,
                    OutboxOp::Upsert,
                    OutboxEntity::AppGroup,
                    &id,
                    &serde_json::json!({ "groupId": id }).to_string(),
                )
                .db()?;
            }
            Ok(Ok(()))
        })
        .await?;
    outcome.map_err(Error::InvalidInput)
}

/// 配对：把 source_process_name 的 group 改成 target_group_id。
/// 如果 source 原本就在 target_group_id，no-op。
/// 操作完成后 source 原来所在的组（如果空了）保留为软删占位 —— 同步到对端便于 LWW。
pub async fn merge(pool: &DbPool, source_process_name: &str, target_group_id: &str) -> Result<()> {
    let src = source_process_name.to_string();
    let tgt = target_group_id.to_string();
    let now = Utc::now().to_rfc3339();

    let outcome: std::result::Result<(), &'static str> = pool
        .0
        .call(move |conn| {
            let tgt_exists: bool = conn
                .query_row(
                    "SELECT 1 FROM app_groups WHERE id = ?1 AND deleted_at IS NULL",
                    rusqlite::params![tgt],
                    |_| Ok(true),
                )
                .optional()
                .db()?
                .unwrap_or(false);
            if !tgt_exists {
                return Ok(Err("目标组不存在或已被删除"));
            }

            let cur_group_id: Option<String> = conn
                .query_row(
                    "SELECT group_id FROM app_group_members
                     WHERE process_name = ?1 AND deleted_at IS NULL",
                    rusqlite::params![src],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .db()?;
            if cur_group_id.as_deref() == Some(tgt.as_str()) {
                return Ok(Ok(()));
            }

            conn.execute(
                "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, NULL)
                 ON CONFLICT(process_name) DO UPDATE SET
                   group_id   = excluded.group_id,
                   updated_at = excluded.updated_at,
                   deleted_at = NULL",
                rusqlite::params![src, tgt, now],
            )
            .db()?;

            enqueue(
                conn,
                OutboxOp::Upsert,
                OutboxEntity::AppGroupMember,
                &src,
                &serde_json::json!({ "processName": src }).to_string(),
            )
            .db()?;

            sync_member_category(conn, &src, &tgt, &now)?;

            Ok(Ok(()))
        })
        .await?;
    outcome.map_err(Error::InvalidInput)
}

/// 拆开：把 process_name 还原到自己的单成员组（id = process_name）。
/// 如果这个组已被软删，复活它。category 跟随当前所在组保留。
pub async fn unmerge(pool: &DbPool, process_name: &str) -> Result<()> {
    let p = process_name.to_string();
    let now = Utc::now().to_rfc3339();

    pool.0
        .call(move |conn| {
            // 当前的 category 用作复活后的初始值，避免用户拆开后分类丢失
            let cur_cat: Option<String> = conn
                .query_row(
                    "SELECT g.category_id
                     FROM app_group_members m
                     JOIN app_groups g ON g.id = m.group_id
                     WHERE m.process_name = ?1 AND m.deleted_at IS NULL",
                    rusqlite::params![p],
                    |r| r.get::<_, Option<String>>(0),
                )
                .optional()
                .db()?
                .flatten();

            // 确保 (id = process_name) 这个原始组存在 / 未删
            conn.execute(
                "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, ?, NULL)
                 ON CONFLICT(id) DO UPDATE SET
                   display_name = excluded.display_name,
                   category_id  = excluded.category_id,
                   updated_at   = excluded.updated_at,
                   deleted_at   = NULL",
                rusqlite::params![p, p, cur_cat, now],
            )
            .db()?;
            enqueue(
                conn,
                OutboxOp::Upsert,
                OutboxEntity::AppGroup,
                &p,
                &serde_json::json!({ "groupId": p }).to_string(),
            )
            .db()?;

            conn.execute(
                "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, NULL)
                 ON CONFLICT(process_name) DO UPDATE SET
                   group_id   = excluded.group_id,
                   updated_at = excluded.updated_at,
                   deleted_at = NULL",
                rusqlite::params![p, p, now],
            )
            .db()?;
            enqueue(
                conn,
                OutboxOp::Upsert,
                OutboxEntity::AppGroupMember,
                &p,
                &serde_json::json!({ "processName": p }).to_string(),
            )
            .db()?;

            // app_categories 跟随：把 source 这个 process_name 的分类同步到 cur_cat
            sync_app_category_row(conn, &p, cur_cat.as_deref(), &now)?;

            Ok(())
        })
        .await?;
    Ok(())
}

/// 改组的统一显示名。category 不动。
pub async fn rename(pool: &DbPool, group_id: &str, new_name: &str) -> Result<()> {
    let id = group_id.to_string();
    let name = new_name.to_string();
    let now = Utc::now().to_rfc3339();

    pool.0
        .call(move |conn| {
            conn.execute(
                "UPDATE app_groups SET display_name = ?2, updated_at = ?3
                 WHERE id = ?1",
                rusqlite::params![id, name, now],
            )
            .db()?;
            enqueue(
                conn,
                OutboxOp::Upsert,
                OutboxEntity::AppGroup,
                &id,
                &serde_json::json!({ "groupId": id }).to_string(),
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// 给组指派分类（None = 取消分类）。
/// 同步把组里所有成员的 app_categories 行更新成相同分类，让旧 reports.rs 继续工作。
pub async fn assign_category(
    pool: &DbPool,
    group_id: &str,
    category_id: Option<String>,
) -> Result<()> {
    let id = group_id.to_string();
    let cat = category_id;
    let now = Utc::now().to_rfc3339();

    pool.0
        .call(move |conn| {
            conn.execute(
                "UPDATE app_groups SET category_id = ?2, updated_at = ?3
                 WHERE id = ?1",
                rusqlite::params![id, cat, now],
            )
            .db()?;
            enqueue(
                conn,
                OutboxOp::Upsert,
                OutboxEntity::AppGroup,
                &id,
                &serde_json::json!({ "groupId": id }).to_string(),
            )
            .db()?;

            // 把所有成员的 app_categories 同步到组的新分类
            let members: Vec<String> = {
                let mut stmt = conn
                    .prepare(
                        "SELECT process_name FROM app_group_members
                         WHERE group_id = ?1 AND deleted_at IS NULL",
                    )
                    .db()?;
                let rows = stmt
                    .query_map(rusqlite::params![id], |r| r.get::<_, String>(0))
                    .db()?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r.db()?);
                }
                out
            };
            for m in &members {
                sync_app_category_row(conn, m, cat.as_deref(), &now)?;
            }
            Ok(())
        })
        .await?;
    Ok(())
}

/// 「per-process 分类」语义的入口：保证 process_name 有组，然后给组指派分类。
/// 老的 commands::categories::assign_app_to_category 走这里，让 mac 上分类
/// "Code" 自动联动到组里所有成员（包括 Windows 的 "Visual Studio Code"）。
pub async fn assign_category_for_process(
    pool: &DbPool,
    process_name: &str,
    category_id: Option<String>,
) -> Result<()> {
    ensure_group(pool, process_name).await?;
    let group_id = group_id_for(pool, process_name).await?;
    let Some(gid) = group_id else { return Ok(()) };
    assign_category(pool, &gid, category_id).await
}

/// 查 process_name 当前所属的 group_id（active 行），不存在返回 None。
pub async fn group_id_for(pool: &DbPool, process_name: &str) -> Result<Option<String>> {
    let p = process_name.to_string();
    let id = pool
        .0
        .call(move |conn| {
            let r = conn
                .query_row(
                    "SELECT group_id FROM app_group_members
                     WHERE process_name = ?1 AND deleted_at IS NULL",
                    rusqlite::params![p],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .db()?;
            Ok(r)
        })
        .await?;
    Ok(id)
}

/// capture 流程会用到：保证某个 process_name 有 (active) 组 + 成员关系。幂等。
/// 不写 outbox（如果是新创建，updated_at 会用现在时间，sync 会自然 push 出去）。
///
/// **设计取舍**：理论上为 sync 完整性应该这里也入 outbox，目前依赖 trigger（DB 层）兜底。
/// 改进路径：把入 outbox 挪到 Rust 代码里，删 trigger，统一所有写入路径走显式 enqueue。
/// 当前 trigger 已稳定运行，重构优先级低，等 sync 一致性出问题时再做。
pub async fn ensure_group(pool: &DbPool, process_name: &str) -> Result<()> {
    let p = process_name.to_string();
    if p.is_empty() || p == "Unknown" {
        return Ok(());
    }
    let now = Utc::now().to_rfc3339();
    // 第一次见到这个 process_name 时，按内置字典自动归类（命中常见浏览器 / 编辑器
    // / 聊天工具等）。命中保留 None 让前端显示"其他"。用户后面手动改了不会被覆盖。
    let builtin_cat = super::builtin_categories::match_builtin_category(&p);

    pool.0
        .call(move |conn| {
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM app_group_members
                     WHERE process_name = ?1 AND deleted_at IS NULL",
                    rusqlite::params![p],
                    |_| Ok(true),
                )
                .optional()
                .db()?
                .unwrap_or(false);
            if exists {
                return Ok(());
            }
            conn.execute(
                "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, ?, NULL)
                 ON CONFLICT(id) DO UPDATE SET
                   display_name = excluded.display_name,
                   updated_at   = excluded.updated_at,
                   deleted_at   = NULL",
                rusqlite::params![p, p, builtin_cat, now],
            )
            .db()?;
            enqueue(
                conn,
                OutboxOp::Upsert,
                OutboxEntity::AppGroup,
                &p,
                &serde_json::json!({ "groupId": p }).to_string(),
            )
            .db()?;

            conn.execute(
                "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, NULL)
                 ON CONFLICT(process_name) DO UPDATE SET
                   group_id   = excluded.group_id,
                   updated_at = excluded.updated_at,
                   deleted_at = NULL",
                rusqlite::params![p, p, now],
            )
            .db()?;
            enqueue(
                conn,
                OutboxOp::Upsert,
                OutboxEntity::AppGroupMember,
                &p,
                &serde_json::json!({ "processName": p }).to_string(),
            )
            .db()?;
            // 命中内置规则时镜像写一份到 app_categories（list_unclassified / 旧 reports
            // 走的就是这张表）。否则 UI 的"应用分类"页会一直把这个 app 当未分类。
            if let Some(cat) = builtin_cat {
                sync_app_category_row(conn, &p, Some(cat), &now)?;
            }
            Ok(())
        })
        .await?;
    Ok(())
}

/// 把某个成员的 app_categories 行同步到给定 category，并写 outbox。
/// 这是 app_groups → app_categories 的 mirror 通道；让旧 reports 查询能直接用 app_categories。
fn sync_member_category(
    conn: &Connection,
    process_name: &str,
    target_group_id: &str,
    now: &str,
) -> rusqlite::Result<()> {
    let cat: Option<String> = conn
        .query_row(
            "SELECT category_id FROM app_groups WHERE id = ?1",
            rusqlite::params![target_group_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    sync_app_category_row(conn, process_name, cat.as_deref(), now)
}

/// 写一行 app_categories（cat=None 则软删），并入 outbox（本端做的修改，需要 push）。
fn sync_app_category_row(
    conn: &Connection,
    process_name: &str,
    category_id: Option<&str>,
    now: &str,
) -> rusqlite::Result<()> {
    apply_app_category_change(conn, process_name, category_id, now)?;
    let payload = serde_json::json!({ "processName": process_name }).to_string();
    enqueue(
        conn,
        OutboxOp::Upsert,
        OutboxEntity::AppCategory,
        process_name,
        &payload,
    )?;
    Ok(())
}

/// 纯 SQL 写一行 app_categories（cat=None 则软删），**不**入 outbox。
/// 给 sync pull 的 mirror 路径用：远端来的变更不需要回推，否则会造成同步死循环。
/// 本端用户操作走 sync_app_category_row（多一步 enqueue）。
pub(crate) fn apply_app_category_change(
    conn: &Connection,
    process_name: &str,
    category_id: Option<&str>,
    now: &str,
) -> rusqlite::Result<()> {
    match category_id {
        Some(cat) => {
            conn.execute(
                "INSERT INTO app_categories(process_name, category_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, NULL)
                 ON CONFLICT(process_name) DO UPDATE SET
                   category_id = excluded.category_id,
                   updated_at  = excluded.updated_at,
                   deleted_at  = NULL",
                rusqlite::params![process_name, cat, now],
            )?;
        }
        None => {
            conn.execute(
                "UPDATE app_categories SET deleted_at = ?, updated_at = ?
                 WHERE process_name = ?",
                rusqlite::params![now, now, process_name],
            )?;
        }
    }
    Ok(())
}
