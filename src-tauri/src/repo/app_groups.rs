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

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::repo::outbox::{enqueue, OutboxEntity, OutboxOp};
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};

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

            // 一次性把所有未软删成员 + 时长统计拉出来；用 GROUP BY process_name 聚合活动时长。
            // recent_secs 按近 7 天窗口算（chip 上显示的时长）；last_device_id 必须查
            // **全部历史**——只查 7 天窗口的话，久未使用的应用哪个设备列都分不到，
            // 配对表整行只剩虚线（没有名字也没有图标）。
            let mut mstmt = conn
                .prepare(
                    "SELECT m.process_name, m.group_id,
                            COALESCE(s.total_secs, 0)   AS recent_secs,
                            -- 该 process_name 最后一次活动所在的设备（全历史）。
                            -- ended_at 是各设备本地时区偏移的 RFC3339 字符串，
                            -- 直接字符串比较跨时区会排错，用 datetime() 归一化到 UTC
                            (SELECT a2.device_id
                               FROM activities a2
                               WHERE a2.process_name = m.process_name
                               ORDER BY datetime(a2.ended_at) DESC LIMIT 1) AS last_device_id
                     FROM app_group_members m
                     LEFT JOIN (
                       SELECT a.process_name,
                              SUM(a.duration_secs) AS total_secs
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

/// **强力删除**：组 + 它所有 active member 一起软删（cascade）。
/// activities 表不动 —— 只断 (app_group_members → app_groups) 的链接；之后这些
/// process_name 在 [reports.rs:day_apps] 的 `GROUP BY COALESCE(g.id, a.process_name)`
/// 下走 process_name 自身作 bucket 聚合（不影响时长统计，只是 UI 没了组的 display_name 与分类）。
///
/// 使用场景：UI 上某行所有 device 列都是 emptyDash（成员近 7 天无活动 →
/// `lastDeviceId` 全为 null，`membersByDevice` 在每列都返回 None），用户看到的是一行空
/// 数据，合理诉求是「让这一行消失」。严格的 [`delete`] 在 `has_members` 时拒绝；这里专门给
/// 这个场景。
///
/// 成员复活：未来再次 capture 同名 process_name → [`ensure_group`] 的 SELECT exists
/// 看到 `deleted_at IS NOT NULL` → 不命中 → 走 INSERT 路径 + ON CONFLICT DO UPDATE
/// 把 deleted_at 设回 NULL → 自然复活。所以本动作不阻止未来重新采集。
///
/// 幂等：第二次调用所有目标行已经 `deleted_at IS NOT NULL`，UPDATE 命中数 0 → no-op；
/// outbox 也不再 enqueue。
pub async fn purge_with_members(pool: &DbPool, group_id: &str) -> Result<()> {
    let id = group_id.to_string();
    let now = utc_now_rfc3339();
    pool.0
        .call(move |conn| {
            // 1. 列出该组所有 active member 的 process_name
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

            // 2. 逐个软删 member，并入 outbox 让对端 LWW 拉到同样的删除状态。
            //    `WHERE deleted_at IS NULL` 让本次 N=0（已是软删状态）不再 enqueue。
            for m in &members {
                let n = conn
                    .execute(
                        "UPDATE app_group_members SET deleted_at = ?1, updated_at = ?1
                         WHERE process_name = ?2 AND deleted_at IS NULL",
                        rusqlite::params![now, m],
                    )
                    .db()?;
                if n > 0 {
                    enqueue(
                        conn,
                        OutboxOp::Upsert,
                        OutboxEntity::AppGroupMember,
                        m,
                        &serde_json::json!({ "processName": m }).to_string(),
                    )
                    .db()?;
                    // app_categories 镜像也跟着断（保持 reports 的 LEFT JOIN 一致）
                    sync_app_category_row(conn, m, None, &now)?;
                }
            }

            // 3. 软删 group 本身
            let n = conn
                .execute(
                    "UPDATE app_groups SET deleted_at = ?1, updated_at = ?1
                     WHERE id = ?2 AND deleted_at IS NULL",
                    rusqlite::params![now, id],
                )
                .db()?;
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
            Ok(())
        })
        .await?;
    Ok(())
}

/// 取一个组下全部 active member 的 process_name。
/// 数据清理与软删都要用它当"删谁"的依据,所以顺序上必须最先跑。
async fn active_member_names(pool: &DbPool, group_id: &str) -> Result<Vec<String>> {
    let id = group_id.to_string();
    let names = pool
        .0
        .call(move |conn| {
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
            Ok(out)
        })
        .await?;
    Ok(names)
}

/// **真删**一个应用组的数据:活动记录、图标/路径缓存、截图文件,以及记忆库里的
/// OCR 文字索引;最后走 [`purge_with_members`] 把组本身软删掉。
///
/// 与 [`purge_with_members`] 的区别:那个只清"归类关系",活动数据分文未动——
/// 应用照样出现在统计里(退回原始进程名),下次运行还会整个回来。这个函数才是
/// 用户点「删除数据」时期待的语义。
///
/// **顺序不可换**:先取成员名单 → 再删数据 → 最后软删组。反过来会先丢掉
/// "要删谁"的依据。
///
/// 明确不做(前端文案已如实告知):
///   - 不重算已生成的日报 / 周报 / AI 总结(它们是存好的文本);
///   - 不传播数据删除到其它设备(组的软删仍照既有行为进 outbox);
///   - `activities` 是同步实体,对端推送的历史理论上可能把数据带回来。
pub async fn purge_with_data(
    pool: &DbPool,
    mem: &crate::memory::MemoryDb,
    screenshot_root: &std::path::Path,
    group_id: &str,
) -> Result<()> {
    let members = active_member_names(pool, group_id).await?;
    if members.is_empty() {
        // 没有成员可删,但组本身仍要处理(可能是建了没用的空组)
        return purge_with_members(pool, group_id).await;
    }

    // ── 主库:先收截图路径,再删行 ──
    let orphan_shots = {
        let names = members.clone();
        pool.0
            .call(move |conn| {
                let ph = vec!["?"; names.len()].join(",");
                let params: Vec<&dyn rusqlite::ToSql> =
                    names.iter().map(|s| s as &dyn rusqlite::ToSql).collect();

                // 1) 候选截图(删行之前取,删完就查不到了)
                let candidates: Vec<String> = {
                    let mut stmt = conn
                        .prepare(&format!(
                            "SELECT DISTINCT screenshot_path FROM activities
                             WHERE process_name IN ({ph}) AND screenshot_path IS NOT NULL"
                        ))
                        .db()?;
                    let rows = stmt
                        .query_map(params.as_slice(), |r| r.get::<_, String>(0))
                        .db()?;
                    let mut out = Vec::new();
                    for r in rows {
                        out.push(r.db()?);
                    }
                    out
                };

                // 2) 删活动与派生缓存
                for table in ["activities", "app_icons", "process_paths"] {
                    conn.execute(
                        &format!("DELETE FROM {table} WHERE process_name IN ({ph})"),
                        params.as_slice(),
                    )
                    .db()?;
                }

                // 3) 兜底:删完后仍被引用的图不能动。实测当前一条 activity 独占一张图、
                //    去重映射也从不跨应用,所以这里通常全部通过;留着是防将来去重
                //    若改成全局图像哈希,免得静默打穿别的应用的证据链。
                let mut deletable = Vec::new();
                for path in candidates {
                    let still_used: Option<i64> = conn
                        .query_row(
                            "SELECT 1 FROM activities WHERE screenshot_path = ?1 LIMIT 1",
                            rusqlite::params![path],
                            |r| r.get(0),
                        )
                        .optional()
                        .db()?;
                    if still_used.is_none() {
                        conn.execute(
                            "DELETE FROM screenshot_dedup_map
                             WHERE member_path = ?1 OR rep_path = ?1",
                            rusqlite::params![path],
                        )
                        .db()?;
                        deletable.push(path);
                    }
                }
                Ok(deletable)
            })
            .await?
    };

    // ── 记忆库:OCR 文字索引。FTS 由 text_sessions 的 AFTER DELETE 触发器自动跟进,
    //    但 session_lines 没有外键,必须显式先删(否则留下指向空会话的孤儿行) ──
    {
        let names = members.clone();
        mem.0
            .call(move |conn| {
                let ph = vec!["?"; names.len()].join(",");
                let params: Vec<&dyn rusqlite::ToSql> =
                    names.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
                conn.execute(
                    &format!(
                        "DELETE FROM session_lines WHERE session_id IN
                         (SELECT id FROM text_sessions WHERE app_id IN ({ph}))"
                    ),
                    params.as_slice(),
                )
                .db()?;
                conn.execute(
                    &format!("DELETE FROM text_sessions WHERE app_id IN ({ph})"),
                    params.as_slice(),
                )
                .db()?;
                conn.execute(
                    &format!("DELETE FROM frames WHERE app_id IN ({ph})"),
                    params.as_slice(),
                )
                .db()?;
                Ok(())
            })
            .await?;
    }

    // ── 组与成员软删 + outbox(既有行为) ──
    purge_with_members(pool, group_id).await?;

    // ── 截图文件。失败只告警:文件删不掉不该让已完成的数据清理失败 ──
    if !orphan_shots.is_empty() {
        let root = screenshot_root.to_path_buf();
        tokio::task::spawn_blocking(move || {
            for rel in orphan_shots {
                let path = root.join(&rel);
                if path.exists() {
                    if let Err(e) = std::fs::remove_file(&path) {
                        log::warn!("删除截图失败 {}: {e}", path.display());
                    }
                }
            }
        })
        .await
        .map_err(|e| Error::Other(format!("截图删除任务失败: {e}")))?;
    }

    Ok(())
}

/// 软删一个**空组**。仅对 0 成员的组生效（有成员强制走 unmerge 路径，避免孤儿成员
/// 突然没有 group_id 可指）。enqueue outbox 让对端也把这个组从列表里去掉。
/// 幂等：组已被删 / 不存在 → no-op。
pub async fn delete(pool: &DbPool, group_id: &str) -> Result<()> {
    let id = group_id.to_string();
    let now = utc_now_rfc3339();
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
    let now = utc_now_rfc3339();

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

            // 成员改指向 + outbox + 分类镜像必须原子:C 批测试实证过非事务下
            // 镜像写失败(如 sync 竞争造成悬空 category 撞 FK)会留下"成员已挪、
            // 分类没跟"的半成品状态。事务化后要么全成,要么全回滚。
            let tx = conn.transaction().db()?;
            tx.execute(
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
                &tx,
                OutboxOp::Upsert,
                OutboxEntity::AppGroupMember,
                &src,
                &serde_json::json!({ "processName": src }).to_string(),
            )
            .db()?;

            sync_member_category(&tx, &src, &tgt, &now)?;
            tx.commit().db()?;

            Ok(Ok(()))
        })
        .await?;
    outcome.map_err(Error::InvalidInput)
}

/// 拆开：把 process_name 还原到自己的单成员组（id = process_name）。
/// 如果这个组已被软删，复活它。category 跟随当前所在组保留。
///
/// 锚点成员（process_name == 当前 group_id）的"单成员组"就是当前组本身——
/// 把它 upsert 回去是 no-op，其余成员会留在组里。对锚点改为解散语义：
/// 把其余成员各自还原成单成员组，锚点留在原组（组名/分类不动）。
pub async fn unmerge(pool: &DbPool, process_name: &str) -> Result<()> {
    let p = process_name.to_string();
    let now = utc_now_rfc3339();

    pool.0
        .call(move |conn| {
            // 当前组 + category（category 用作复活后的初始值，避免用户拆开后分类丢失）
            let cur: Option<(String, Option<String>)> = conn
                .query_row(
                    "SELECT g.id, g.category_id
                     FROM app_group_members m
                     JOIN app_groups g ON g.id = m.group_id
                     WHERE m.process_name = ?1 AND m.deleted_at IS NULL",
                    rusqlite::params![p],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
                )
                .optional()
                .db()?;
            let Some((cur_group, cur_cat)) = cur else {
                return Ok(());
            };

            // 同 merge:每个成员的还原是 5 条写语句,解散锚点组还是多成员循环——
            // 全部收进一个事务,半途失败不留"一半人还原了"的状态。
            let tx = conn.transaction().db()?;
            if cur_group == p {
                let others: Vec<String> = {
                    let mut stmt = tx
                        .prepare(
                            "SELECT process_name FROM app_group_members
                             WHERE group_id = ?1 AND deleted_at IS NULL AND process_name <> ?1",
                        )
                        .db()?;
                    let rows = stmt
                        .query_map(rusqlite::params![p], |r| r.get::<_, String>(0))
                        .db()?;
                    let mut out = Vec::new();
                    for r in rows {
                        out.push(r.db()?);
                    }
                    out
                };
                for o in &others {
                    restore_solo_group(&tx, o, cur_cat.as_deref(), &now)?;
                }
            } else {
                restore_solo_group(&tx, &p, cur_cat.as_deref(), &now)?;
            }
            tx.commit().db()?;

            Ok(())
        })
        .await?;
    Ok(())
}

/// 把一个成员还原到自己的单成员组（id = process_name），软删的组复活。
/// ON CONFLICT 不覆盖 display_name：组已存在时保留用户改过的名字（同 ensure_group 的理由）。
fn restore_solo_group(
    conn: &Connection,
    process_name: &str,
    category_id: Option<&str>,
    now: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
         VALUES(?, ?, ?, ?, NULL)
         ON CONFLICT(id) DO UPDATE SET
           category_id  = excluded.category_id,
           updated_at   = excluded.updated_at,
           deleted_at   = NULL",
        rusqlite::params![process_name, process_name, category_id, now],
    )?;
    enqueue(
        conn,
        OutboxOp::Upsert,
        OutboxEntity::AppGroup,
        process_name,
        &serde_json::json!({ "groupId": process_name }).to_string(),
    )?;

    conn.execute(
        "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
         VALUES(?, ?, ?, NULL)
         ON CONFLICT(process_name) DO UPDATE SET
           group_id   = excluded.group_id,
           updated_at = excluded.updated_at,
           deleted_at = NULL",
        rusqlite::params![process_name, process_name, now],
    )?;
    enqueue(
        conn,
        OutboxOp::Upsert,
        OutboxEntity::AppGroupMember,
        process_name,
        &serde_json::json!({ "processName": process_name }).to_string(),
    )?;

    // app_categories 跟随：把这个 process_name 的分类同步到 category_id
    sync_app_category_row(conn, process_name, category_id, now)?;
    Ok(())
}

/// 改组的统一显示名。category 不动。
pub async fn rename(pool: &DbPool, group_id: &str, new_name: &str) -> Result<()> {
    let id = group_id.to_string();
    let name = new_name.to_string();
    let now = utc_now_rfc3339();

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
    let now = utc_now_rfc3339();

    pool.0
        .call(move |conn| {
            // 组分类 + 全体成员镜像原子化:半途失败时不留"组换了分类、
            // 部分成员还挂旧分类"的撕裂状态(报表会两边数字对不上)。
            let tx = conn.transaction().db()?;
            tx.execute(
                "UPDATE app_groups SET category_id = ?2, updated_at = ?3
                 WHERE id = ?1",
                rusqlite::params![id, cat, now],
            )
            .db()?;
            enqueue(
                &tx,
                OutboxOp::Upsert,
                OutboxEntity::AppGroup,
                &id,
                &serde_json::json!({ "groupId": id }).to_string(),
            )
            .db()?;

            // 把所有成员的 app_categories 同步到组的新分类
            let members: Vec<String> = {
                let mut stmt = tx
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
                sync_app_category_row(&tx, m, cat.as_deref(), &now)?;
            }
            tx.commit().db()?;
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
    let now = utc_now_rfc3339();
    // 跨 OS 别名规范化：mac "Microsoft PowerPoint" + Win "POWERPNT.EXE" 通过别名表
    // 都映射到同一 canonical "Microsoft PowerPoint"。第一次见到一个别名时直接进 canonical
    // 组（而不是 process_name 自身），让两台设备上的同一应用自然合并成一行。
    let canonical = super::cross_os_aliases::lookup_canonical(&p);
    let group_id = canonical.map(String::from).unwrap_or_else(|| p.clone());
    let display_name = canonical.unwrap_or(&p).to_string();
    // 内置分类按 canonical 名查（"google chrome" 命中），让别名也能拿到对应分类。
    // 命中保留 None 让前端显示"其他"。用户后面手动改了不会被覆盖。
    let builtin_cat = super::builtin_categories::match_builtin_category(canonical.unwrap_or(&p));

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
            // 用 ON CONFLICT DO UPDATE ... WHERE deleted_at IS NOT NULL 只复活软删的组；
            // 已经 active 的组保留用户改过的 display_name / category_id 不动 ——
            // 跨 OS 别名表把多个 process_name 都指向同一 canonical 组后，第二个、第三个
            // 别名 capture 时 ON CONFLICT 会高频触发；不做 WHERE 限定会冲掉用户的改名。
            conn.execute(
                "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, ?, NULL)
                 ON CONFLICT(id) DO UPDATE SET
                   deleted_at = NULL,
                   updated_at = excluded.updated_at
                 WHERE app_groups.deleted_at IS NOT NULL",
                rusqlite::params![group_id, display_name, builtin_cat, now],
            )
            .db()?;
            enqueue(
                conn,
                OutboxOp::Upsert,
                OutboxEntity::AppGroup,
                &group_id,
                &serde_json::json!({ "groupId": group_id }).to_string(),
            )
            .db()?;

            conn.execute(
                "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, NULL)
                 ON CONFLICT(process_name) DO UPDATE SET
                   group_id   = excluded.group_id,
                   updated_at = excluded.updated_at,
                   deleted_at = NULL",
                rusqlite::params![p, group_id, now],
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::fresh_test_pool;
    use crate::storage::SqliteResultExt;

    /// 测 [`purge_with_members`]：
    /// - 组 + 所有 active member 全部软删（deleted_at IS NOT NULL）
    /// - app_categories 镜像跟着软删（保持 reports LEFT JOIN 一致）
    /// - outbox 写入对应 entity 行（让对端 LWW 拉到同样删除状态）
    /// - 幂等：再调一次 outbox 不再增长
    #[tokio::test]
    async fn purge_with_members_soft_deletes_group_members_and_mirror() {
        let pool = fresh_test_pool().await;
        seed_vscode_group(&pool).await;

        purge_with_members(&pool, "vscode").await.unwrap();

        // 组 + 两成员都被软删
        assert!(group_deleted(&pool, "vscode").await, "组本身应被软删");
        assert!(member_deleted(&pool, "Code").await, "成员 Code 应被软删");
        assert!(
            member_deleted(&pool, "Code.exe").await,
            "成员 Code.exe 应被软删"
        );

        // app_categories 镜像也跟着软删
        assert!(
            app_category_deleted(&pool, "Code").await,
            "app_categories Code 镜像应跟着软删"
        );
        assert!(
            app_category_deleted(&pool, "Code.exe").await,
            "app_categories Code.exe 镜像应跟着软删"
        );

        // outbox：至少 1 个 group + 2 个 member upsert + 2 个 app_category upsert
        let outbox_after = outbox_summary(&pool).await;
        assert!(
            outbox_after.group_count >= 1,
            "至少应有 1 条 app_group outbox"
        );
        assert_eq!(
            outbox_after.member_count, 2,
            "应有 2 条 app_group_member outbox"
        );
        assert_eq!(
            outbox_after.app_category_count, 2,
            "应有 2 条 app_category outbox（镜像跟随）"
        );

        // 幂等：再调一次，outbox 不应再增长
        let before = outbox_total(&pool).await;
        purge_with_members(&pool, "vscode").await.unwrap();
        let after = outbox_total(&pool).await;
        assert_eq!(before, after, "幂等：第二次调用不该写新 outbox");
    }

    async fn seed_vscode_group(pool: &DbPool) {
        pool.0
            .call(|conn| {
                let now = "2026-05-15T10:00:00Z";
                // 用裸 INSERT 绕开 ensure_group 的 cross_os_alias 规范化逻辑，
                // 测试想要的就是 group_id="vscode" + 两个 member。
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('vscode', 'vscode', 'code', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('Code', 'vscode', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('Code.exe', 'vscode', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                // app_categories 镜像
                conn.execute(
                    "INSERT INTO app_categories(process_name, category_id, updated_at, deleted_at)
                     VALUES('Code', 'code', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                conn.execute(
                    "INSERT INTO app_categories(process_name, category_id, updated_at, deleted_at)
                     VALUES('Code.exe', 'code', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn group_deleted(pool: &DbPool, id: &str) -> bool {
        let id = id.to_string();
        pool.0
            .call(move |conn| {
                let v: Option<String> = conn
                    .query_row(
                        "SELECT deleted_at FROM app_groups WHERE id = ?1",
                        rusqlite::params![id],
                        |r| r.get(0),
                    )
                    .db()?;
                Ok(v.is_some())
            })
            .await
            .unwrap()
    }

    async fn member_deleted(pool: &DbPool, process_name: &str) -> bool {
        let pn = process_name.to_string();
        pool.0
            .call(move |conn| {
                let v: Option<String> = conn
                    .query_row(
                        "SELECT deleted_at FROM app_group_members WHERE process_name = ?1",
                        rusqlite::params![pn],
                        |r| r.get(0),
                    )
                    .db()?;
                Ok(v.is_some())
            })
            .await
            .unwrap()
    }

    async fn app_category_deleted(pool: &DbPool, process_name: &str) -> bool {
        let pn = process_name.to_string();
        pool.0
            .call(move |conn| {
                let v: Option<String> = conn
                    .query_row(
                        "SELECT deleted_at FROM app_categories WHERE process_name = ?1",
                        rusqlite::params![pn],
                        |r| r.get(0),
                    )
                    .db()?;
                Ok(v.is_some())
            })
            .await
            .unwrap()
    }

    struct OutboxSummary {
        group_count: i64,
        member_count: i64,
        app_category_count: i64,
    }

    async fn outbox_summary(pool: &DbPool) -> OutboxSummary {
        pool.0
            .call(|conn| {
                let g: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM sync_outbox WHERE entity = 'app_group'",
                        [],
                        |r| r.get(0),
                    )
                    .db()?;
                let m: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM sync_outbox WHERE entity = 'app_group_member'",
                        [],
                        |r| r.get(0),
                    )
                    .db()?;
                let c: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM sync_outbox WHERE entity = 'app_category'",
                        [],
                        |r| r.get(0),
                    )
                    .db()?;
                Ok(OutboxSummary {
                    group_count: g,
                    member_count: m,
                    app_category_count: c,
                })
            })
            .await
            .unwrap()
    }

    async fn outbox_total(pool: &DbPool) -> i64 {
        pool.0
            .call(|conn| {
                let n: i64 = conn
                    .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0))
                    .db()?;
                Ok(n)
            })
            .await
            .unwrap()
    }

    /// 读某成员当前 active 的 group_id（deleted_at IS NULL）；无 active 行返回 None。
    async fn active_group_of(pool: &DbPool, process_name: &str) -> Option<String> {
        let pn = process_name.to_string();
        pool.0
            .call(move |conn| {
                let v = conn
                    .query_row(
                        "SELECT group_id FROM app_group_members
                         WHERE process_name = ?1 AND deleted_at IS NULL",
                        rusqlite::params![pn],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()
                    .db()?;
                Ok(v)
            })
            .await
            .unwrap()
    }

    /// 读 app_categories 一行的 (category_id, 是否软删)。行不存在返回 None ——
    /// 与「行存在但软删」区分开：镜像通道对这两种状态的语义不同。
    async fn app_category_state(
        pool: &DbPool,
        process_name: &str,
    ) -> Option<(Option<String>, bool)> {
        let pn = process_name.to_string();
        pool.0
            .call(move |conn| {
                let v = conn
                    .query_row(
                        "SELECT category_id, deleted_at IS NOT NULL
                         FROM app_categories WHERE process_name = ?1",
                        rusqlite::params![pn],
                        |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, bool>(1)?)),
                    )
                    .optional()
                    .db()?;
                Ok(v)
            })
            .await
            .unwrap()
    }

    /// 读 app_groups 一行的 (display_name, category_id, 是否软删)。
    async fn group_state(pool: &DbPool, id: &str) -> Option<(String, Option<String>, bool)> {
        let id = id.to_string();
        pool.0
            .call(move |conn| {
                let v = conn
                    .query_row(
                        "SELECT display_name, category_id, deleted_at IS NOT NULL
                         FROM app_groups WHERE id = ?1",
                        rusqlite::params![id],
                        |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, Option<String>>(1)?,
                                r.get::<_, bool>(2)?,
                            ))
                        },
                    )
                    .optional()
                    .db()?;
                Ok(v)
            })
            .await
            .unwrap()
    }

    /// 测 [`merge`] 成功链路的完整收尾：成员改指向目标组之外，
    /// [`sync_member_category`] 必须把目标组的 category 镜像进 app_categories，
    /// 并且 member + app_category 都入 outbox（对端靠它拉到同样状态）。
    /// 附带边界：目标 category=NULL 时镜像软删；目标软删时拒绝；同组重复 merge 幂等。
    #[tokio::test]
    async fn merge_repoints_member_and_mirrors_target_category_with_outbox() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|conn| {
                let now = "2026-05-15T10:00:00Z";
                // 目标组 vscode（分类 code）；源 chrome.exe 自己一个组，
                // 且旧镜像行是 browse —— merge 后必须被目标分类覆盖，才能证明
                // sync_member_category 真的跑了，而不是碰巧读到旧值。
                // 注意：app_categories.category_id 有 REFERENCES categories(id)
                // 且本仓库的 SQLite 强制 FK，seed 只能用 migrations 预置的分类 id
                //（code/browse/talk/design/fun/other）。
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('vscode', 'vscode', 'code', ?1, NULL),
                           ('notes',  'notes',  NULL,   ?1, NULL),
                           ('dead',   'dead',   'fun',  ?1, ?1),
                           ('chrome.exe', 'chrome.exe', 'browse', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('chrome.exe', 'chrome.exe', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                conn.execute(
                    "INSERT INTO app_categories(process_name, category_id, updated_at, deleted_at)
                     VALUES('chrome.exe', 'browse', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();

        merge(&pool, "chrome.exe", "vscode").await.unwrap();

        assert_eq!(
            active_group_of(&pool, "chrome.exe").await.as_deref(),
            Some("vscode"),
            "merge 后成员应指向目标组"
        );
        assert_eq!(
            app_category_state(&pool, "chrome.exe").await,
            Some((Some("code".into()), false)),
            "镜像行应被目标组分类覆盖（browse → code）且保持 active"
        );
        // 裸 INSERT seed 不产生 outbox，所以这里的计数就是 merge 一次的净产出：
        // 1 条 member + 1 条 app_category（镜像跟随）。
        let ob = outbox_summary(&pool).await;
        assert_eq!(
            ob.member_count, 1,
            "merge 应写 1 条 app_group_member outbox"
        );
        assert_eq!(
            ob.app_category_count, 1,
            "sync_member_category 收尾应写 1 条 app_category outbox"
        );

        // 幂等：已经在目标组，再 merge 一次应是纯 no-op（不写 DB 也不写 outbox）
        let before = outbox_total(&pool).await;
        merge(&pool, "chrome.exe", "vscode").await.unwrap();
        assert_eq!(
            outbox_total(&pool).await,
            before,
            "同组重复 merge 不应产生新 outbox"
        );

        // 目标组 category=NULL：镜像走软删分支（旧 reports 查不到分类 = 未分类）
        merge(&pool, "chrome.exe", "notes").await.unwrap();
        assert_eq!(
            app_category_state(&pool, "chrome.exe").await,
            Some((Some("code".into()), true)),
            "并入无分类组后镜像行应被软删（category 值不清空但 deleted_at 置位）"
        );

        // 软删的目标组视同不存在：拒绝并且成员不动
        let err = merge(&pool, "chrome.exe", "dead").await.unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput(_)),
            "目标组软删应返回 InvalidInput，实际: {err:?}"
        );
        assert_eq!(
            active_group_of(&pool, "chrome.exe").await.as_deref(),
            Some("notes"),
            "merge 失败后成员应停留在原组"
        );
    }

    /// 测 [`unmerge`] → [`restore_solo_group`]：
    /// - 单成员组已软删 → ON CONFLICT 复活，但**不**覆盖用户改过的 display_name
    /// - 复活时 category 跟随「拆出前所在组」的分类（用户拆开后分类不丢）
    /// - 单成员组从未存在 → 新建，display_name = process_name
    /// - 镜像 + outbox 跟随
    #[tokio::test]
    async fn unmerge_revives_solo_group_keeping_display_name_and_carrying_category() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|conn| {
                let now = "2026-05-15T10:00:00Z";
                // vscode 组（分类 code）里有 3 个成员；其中 Code.exe 的单成员组
                // 之前被软删过，且用户改过名（"我的编辑器"）、挂过旧分类 old-cat ——
                // 复活时名字必须保住，分类必须换成现组的 code。
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('vscode',   'vscode',     'code',    ?1, NULL),
                           ('Code.exe', '我的编辑器', 'old-cat', ?1, ?1)",
                    rusqlite::params![now],
                )
                .db()?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('Code',     'vscode', ?1, NULL),
                           ('Code.exe', 'vscode', ?1, NULL),
                           ('OtherApp', 'vscode', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();

        // 1) 复活软删组的路径
        unmerge(&pool, "Code.exe").await.unwrap();

        let (name, cat, deleted) = group_state(&pool, "Code.exe").await.unwrap();
        assert!(!deleted, "软删的单成员组应被复活");
        assert_eq!(name, "我的编辑器", "复活不应覆盖用户改过的 display_name");
        assert_eq!(
            cat.as_deref(),
            Some("code"),
            "复活组应携带拆出前所在组（vscode）的分类，而非残留的 old-cat"
        );
        assert_eq!(
            active_group_of(&pool, "Code.exe").await.as_deref(),
            Some("Code.exe"),
            "成员应回到自己的单成员组"
        );
        assert_eq!(
            app_category_state(&pool, "Code.exe").await,
            Some((Some("code".into()), false)),
            "app_categories 镜像应同步为原组分类"
        );
        let ob = outbox_summary(&pool).await;
        assert_eq!(ob.group_count, 1, "复活组应写 1 条 app_group outbox");
        assert_eq!(ob.member_count, 1, "成员改指向应写 1 条 member outbox");
        assert_eq!(
            ob.app_category_count, 1,
            "镜像跟随应写 1 条 app_category outbox"
        );

        // 2) 单成员组从未存在 → 全新 INSERT，display_name 用 process_name 本身
        unmerge(&pool, "OtherApp").await.unwrap();
        let (name, cat, deleted) = group_state(&pool, "OtherApp").await.unwrap();
        assert!(!deleted);
        assert_eq!(
            name, "OtherApp",
            "新建单成员组 display_name 应为 process_name"
        );
        assert_eq!(cat.as_deref(), Some("code"), "新建组同样携带原组分类");
        assert_eq!(
            active_group_of(&pool, "OtherApp").await.as_deref(),
            Some("OtherApp")
        );

        // 3) 边界：process_name 没有 active member 行 → 静默 no-op，不写 outbox
        let before = outbox_total(&pool).await;
        unmerge(&pool, "从未出现过的进程").await.unwrap();
        assert_eq!(
            outbox_total(&pool).await,
            before,
            "未知 process_name 的 unmerge 应是 no-op"
        );
    }

    /// 测 [`list_groups`] 组装逻辑：
    /// - 成员指向软删组（组 SELECT 过滤掉了）→ 该成员静默丢弃，不 panic 不串组
    /// - recent_secs 只算近 7 天窗口，且跨设备按 process_name 求和
    /// - 组间排序按「组内最大 recent_secs」降序（不是求和、不是首成员）
    /// - last_device_id 取全历史 ended_at 最大的那条活动的设备
    #[tokio::test]
    async fn list_groups_drops_orphan_members_and_sorts_by_max_recent_secs() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|conn| {
                let now = "2026-05-15T10:00:00Z";
                // 组插入顺序故意与期望输出相反（beta 在前），排错了就会暴露。
                // zombie 是软删组：ghost 成员指向它 → list_groups 的组列表里没有它。
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('beta',   'beta',   NULL, ?1, NULL),
                           ('alpha',  'alpha',  NULL, ?1, NULL),
                           ('empty',  'empty',  NULL, ?1, NULL),
                           ('zombie', 'zombie', NULL, ?1, ?1)",
                    rusqlite::params![now],
                )
                .db()?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('b1',    'beta',   ?1, NULL),
                           ('a1',    'alpha',  ?1, NULL),
                           ('a2',    'alpha',  ?1, NULL),
                           ('ghost', 'zombie', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                // 活动数据（duration 单位秒）：
                //   a1: 60(mac, 2h前结束) + 40(win, 1h前结束) → recent 100，last_device=win
                //   a2: 200(mac) + 300(win) → 跨设备求和 500 → alpha 组内最大值
                //   b1: 300(今天) + 9999(30 天前，窗口外必须排除；若被计入 beta 会错排第一)
                //   ghost: 77777 → 即使很大也该整个被丢弃
                // 注意 alpha 的最大值来自第二个成员 a2 —— 若实现错拿首成员排序会暴露。
                conn.execute_batch(
                    "INSERT INTO activities(started_at, ended_at, duration_secs, local_date,
                                            local_hour, process_name, category_id, device_id)
                     VALUES
                       (strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 hours'),
                        strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),
                        60, date('now','localtime'), 9, 'a1', 'work', 'mac'),
                       (strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),
                        strftime('%Y-%m-%dT%H:%M:%SZ','now','-1 hours'),
                        40, date('now','localtime'), 10, 'a1', 'work', 'win'),
                       (strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 hours'),
                        strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),
                        200, date('now','localtime'), 9, 'a2', 'work', 'mac'),
                       (strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),
                        strftime('%Y-%m-%dT%H:%M:%SZ','now','-1 hours'),
                        300, date('now','localtime'), 10, 'a2', 'work', 'win'),
                       (strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),
                        strftime('%Y-%m-%dT%H:%M:%SZ','now','-1 hours'),
                        300, date('now','localtime'), 10, 'b1', 'work', 'mac'),
                       (strftime('%Y-%m-%dT%H:%M:%SZ','now','-30 days'),
                        strftime('%Y-%m-%dT%H:%M:%SZ','now','-30 days'),
                        9999, date('now','localtime','-30 days'), 10, 'b1', 'work', 'old-box'),
                       (strftime('%Y-%m-%dT%H:%M:%SZ','now','-2 hours'),
                        strftime('%Y-%m-%dT%H:%M:%SZ','now','-1 hours'),
                        77777, date('now','localtime'), 10, 'ghost', 'work', 'mac');",
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();

        let groups = list_groups(&pool).await.unwrap();

        // 排序：alpha(max=500) > beta(max=300) > empty(max=0)；zombie 不出现
        let order: Vec<&str> = groups.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(
            order,
            vec!["alpha", "beta", "empty"],
            "应按组内最大 recent_secs 降序，且软删组不出现"
        );

        // ghost 指向软删组 → 任何组里都不该出现（静默丢弃，而不是挂错组）
        assert!(
            groups
                .iter()
                .flat_map(|g| &g.members)
                .all(|m| m.process_name != "ghost"),
            "指向软删组的成员应被丢弃"
        );

        let alpha = &groups[0];
        let a1 = alpha
            .members
            .iter()
            .find(|m| m.process_name == "a1")
            .expect("alpha 应含成员 a1");
        let a2 = alpha
            .members
            .iter()
            .find(|m| m.process_name == "a2")
            .expect("alpha 应含成员 a2");
        assert_eq!(a1.recent_secs, 100, "a1 = 60 + 40");
        assert_eq!(a2.recent_secs, 500, "a2 跨设备求和 = 200 + 300");
        assert_eq!(
            a1.last_device_id.as_deref(),
            Some("win"),
            "last_device_id 应取 ended_at 最大那条活动的设备"
        );

        let beta = &groups[1];
        assert_eq!(beta.members.len(), 1);
        assert_eq!(
            beta.members[0].recent_secs, 300,
            "30 天前的 9999 秒在 7 天窗口外，必须被排除"
        );

        // 空组：members 为空但组仍在列表里（排最后）
        assert!(groups[2].members.is_empty(), "empty 组应无成员但仍返回");
    }

    /// 回归:merge 的写序列必须原子(C 批审计实锤的部分写入 bug)。
    ///
    /// 场景:目标组 category_id 悬空('no-such-cat',app_groups.category_id
    /// 无 FK 所以 seed 得进去),merge 走到 sync_member_category 给
    /// app_categories 写镜像时撞 FK(app_categories.category_id 有 FK)。
    /// 修复前:member 已改指向目标组 + outbox 已 enqueue,只有镜像没写 ——
    /// 半成品状态。修复后:整个事务回滚,除了返回 Err 什么都没发生。
    #[tokio::test]
    async fn merge_rolls_back_entirely_when_mirror_fk_fails() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|conn| {
                let now = "2026-05-15T10:00:00Z";
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('tgt-grp', '目标组', 'no-such-cat', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('src-grp', '来源组', NULL, ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('SrcApp', 'src-grp', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
        let outbox_before = outbox_total(&pool).await;

        let res = merge(&pool, "SrcApp", "tgt-grp").await;
        assert!(res.is_err(), "镜像 FK 失败应让 merge 整体返回 Err");

        // 回滚核对:成员没被挪走、outbox 没多一行、镜像没写
        let (member_group, mirror_rows): (String, i64) = pool
            .0
            .call(|conn| {
                let g: String = conn
                    .query_row(
                        "SELECT group_id FROM app_group_members WHERE process_name = 'SrcApp'",
                        [],
                        |r| r.get(0),
                    )
                    .db()?;
                let m: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM app_categories WHERE process_name = 'SrcApp'",
                        [],
                        |r| r.get(0),
                    )
                    .db()?;
                Ok((g, m))
            })
            .await
            .unwrap();
        assert_eq!(member_group, "src-grp", "回滚后成员应仍指向原组");
        assert_eq!(mirror_rows, 0, "回滚后不应留下 app_categories 镜像行");
        assert_eq!(
            outbox_total(&pool).await,
            outbox_before,
            "回滚后 outbox 不应有新增行"
        );
    }

    /// 回归:assign_category 同样必须原子 —— 组行更新 + outbox 成功后,
    /// 成员镜像撞 FK,修复前会留下"组换了分类、成员镜像还是旧的"的撕裂。
    #[tokio::test]
    async fn assign_category_rolls_back_group_update_when_mirror_fk_fails() {
        let pool = fresh_test_pool().await;
        seed_vscode_group(&pool).await;
        let outbox_before = outbox_total(&pool).await;

        let res = assign_category(&pool, "vscode", Some("no-such-cat".to_string())).await;
        assert!(
            res.is_err(),
            "成员镜像 FK 失败应让 assign_category 整体 Err"
        );

        let group_cat: Option<String> = pool
            .0
            .call(|conn| {
                let c = conn
                    .query_row(
                        "SELECT category_id FROM app_groups WHERE id = 'vscode'",
                        [],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .db()?;
                Ok(c)
            })
            .await
            .unwrap();
        assert_eq!(
            group_cat.as_deref(),
            Some("code"),
            "回滚后组的 category_id 应保持原值"
        );
        assert_eq!(
            outbox_total(&pool).await,
            outbox_before,
            "回滚后 outbox 不应有新增行"
        );
    }

    // ───────────── purge_with_data:真删数据 ─────────────

    /// 造一个应用的完整痕迹:主库活动 + 记忆库 OCR 会话/帧 + 一张截图文件。
    async fn seed_app_traces(
        pool: &DbPool,
        mem: &crate::memory::MemoryDb,
        root: &std::path::Path,
        process: &str,
        shot_rel: &str,
        text: &str,
    ) {
        std::fs::write(root.join(shot_rel), b"fake-png").unwrap();
        let (p, sr) = (process.to_string(), shot_rel.to_string());
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activities(started_at, ended_at, duration_secs, local_date,
                                            local_hour, process_name, window_title,
                                            category_id, screenshot_path)
                     VALUES('2026-05-15T10:00:00Z','2026-05-15T10:05:00Z',300,'2026-05-15',
                            10, ?1, 'title', 'other', ?2)",
                    rusqlite::params![p, sr],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
        let (p2, sr2, txt) = (process.to_string(), shot_rel.to_string(), text.to_string());
        mem.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO text_sessions(local_date, started_ts, ended_ts, app_id, title, text)
                     VALUES('2026-05-15','2026-05-15T10:00:00Z','2026-05-15T10:05:00Z',?1,'t',?2)",
                    rusqlite::params![p2, txt],
                )
                .db()?;
                let sid = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO session_lines(session_id, line_no, text, first_path, first_ts)
                     VALUES(?1, 0, ?2, ?3, '2026-05-15T10:00:00Z')",
                    rusqlite::params![sid, txt, sr2],
                )
                .db()?;
                // frames.path 是主键:共享截图的测试里两个应用指向同一张图,
                // 现实中该图只会有一条帧记录,这里用 OR IGNORE 如实模拟
                conn.execute(
                    "INSERT OR IGNORE INTO frames(path, ts, local_date, app_id, title, ocr_state)
                     VALUES(?1,'2026-05-15T10:00:00Z','2026-05-15',?2,'t',1)",
                    rusqlite::params![sr2, p2],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn count_activities(pool: &DbPool, process: &str) -> i64 {
        let p = process.to_string();
        pool.0
            .call(move |conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM activities WHERE process_name = ?1",
                    rusqlite::params![p],
                    |r| r.get::<_, i64>(0),
                )
                .db()
            })
            .await
            .unwrap()
    }

    async fn fts_hits(mem: &crate::memory::MemoryDb, needle: &str) -> i64 {
        let n = needle.to_string();
        mem.0
            .call(move |conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM text_sessions_fts WHERE text_sessions_fts MATCH ?1",
                    rusqlite::params![n],
                    |r| r.get::<_, i64>(0),
                )
                .db()
            })
            .await
            .unwrap()
    }

    /// 为什么测:「删除数据」承诺清空活动、截图与 OCR 文字索引。三处分属两个
    /// 数据库 + 文件系统,漏掉任何一处都是"假删除"——尤其 OCR 索引里存着屏幕上
    /// 出现过的原文,只删活动行的话搜索页照样能搜到。
    /// 同时必须证明**只删目标应用**:隔壁应用的数据一条都不能少。
    #[tokio::test]
    async fn purge_with_data_wipes_all_three_places_and_spares_others() {
        let pool = fresh_test_pool().await;
        let mem = crate::memory::MemoryDb::open_in_memory().await.unwrap();
        let dir = std::env::temp_dir().join(format!("hs-purge-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        seed_vscode_group(&pool).await;
        seed_app_traces(&pool, &mem, &dir, "Code", "code.png", "机密文档内容").await;
        seed_app_traces(
            &pool,
            &mem,
            &dir,
            "Code.exe",
            "code-exe.png",
            "另一台机器上的文字",
        )
        .await;
        // 隔壁应用:不该被波及
        seed_app_traces(&pool, &mem, &dir, "Chrome", "chrome.png", "浏览器里的文字").await;

        purge_with_data(&pool, &mem, &dir, "vscode").await.unwrap();

        // 主库:组内**两个** member 的活动都清了(多进程名组是跨设备合并的常态)
        assert_eq!(count_activities(&pool, "Code").await, 0);
        assert_eq!(count_activities(&pool, "Code.exe").await, 0);
        assert_eq!(
            count_activities(&pool, "Chrome").await,
            1,
            "隔壁应用不该受影响"
        );

        // 记忆库:FTS 里搜不到被删应用的原文,隔壁的仍在
        assert_eq!(
            fts_hits(&mem, "机密文档内容").await,
            0,
            "OCR 文字索引必须清掉"
        );
        assert_eq!(
            fts_hits(&mem, "浏览器里的文字").await,
            1,
            "隔壁应用的文字应还在"
        );

        // 文件系统
        assert!(!dir.join("code.png").exists(), "截图文件应被删除");
        assert!(!dir.join("code-exe.png").exists());
        assert!(dir.join("chrome.png").exists(), "隔壁应用的截图不该被删");

        // 组本身仍按既有语义软删
        assert!(group_deleted(&pool, "vscode").await);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 为什么测:实测当前一条 activity 独占一张截图、去重映射也从不跨应用,
    /// 所以正常路径下不会误删别人的图。但万一将来去重改成全局图像哈希,
    /// 共享就可能出现——这行兜底检查必须挡住,否则会静默打穿别的应用的证据链
    /// (搜索结果点开看不到原图)。
    #[tokio::test]
    async fn purge_with_data_keeps_screenshot_still_referenced_by_others() {
        let pool = fresh_test_pool().await;
        let mem = crate::memory::MemoryDb::open_in_memory().await.unwrap();
        let dir = std::env::temp_dir().join(format!("hs-purge-shared-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        seed_vscode_group(&pool).await;
        // 人为让两个应用引用同一张图(现实中不会发生,见函数文档)
        seed_app_traces(&pool, &mem, &dir, "Code", "shared.png", "甲的文字").await;
        seed_app_traces(&pool, &mem, &dir, "Chrome", "shared.png", "乙的文字").await;

        purge_with_data(&pool, &mem, &dir, "vscode").await.unwrap();

        assert_eq!(count_activities(&pool, "Code").await, 0);
        assert!(
            dir.join("shared.png").exists(),
            "还被别的应用引用的截图不能删——否则对方的证据卡点开是空的"
        );
        assert_eq!(count_activities(&pool, "Chrome").await, 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
