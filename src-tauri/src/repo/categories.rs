use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::repo::outbox::{enqueue, OutboxEntity, OutboxOp};
use crate::storage::DbPool;
use crate::db::SqliteResultExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub id: String,
    pub name: String,
    pub color: String,
    pub icon: String,
    pub builtin: bool,
    pub apps: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryInput {
    pub name: String,
    pub color: String,
    pub icon: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryPatch {
    pub name: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnclassifiedApp {
    pub process_name: String,
    pub minutes: u32,
    pub last_seen_at: String,
}

fn category_payload(id: &str, name: &str, color: &str, icon: &str, builtin: bool, sort_order: i64, updated_at: &str, deleted_at: Option<&str>) -> String {
    serde_json::json!({
        "id": id,
        "name": name,
        "color": color,
        "icon": icon,
        "builtin": builtin,
        "sortOrder": sort_order,
        "updatedAt": updated_at,
        "deletedAt": deleted_at,
    })
    .to_string()
}

fn app_category_payload(process_name: &str, category_id: &str, updated_at: &str, deleted_at: Option<&str>) -> String {
    serde_json::json!({
        "processName": process_name,
        "categoryId": category_id,
        "updatedAt": updated_at,
        "deletedAt": deleted_at,
    })
    .to_string()
}

pub async fn list(pool: &DbPool) -> Result<Vec<Category>> {
    let cats = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare_cached(
                    // 用户拖拽排序后的 sort_order 决定显示顺序；id 作为 tiebreaker。
                    "SELECT id, name, color, icon, builtin FROM categories
                     WHERE deleted_at IS NULL
                     ORDER BY sort_order ASC, id ASC",
                )
                .db()?;
            let cat_rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)? != 0,
                    ))
                })
                .db()?;
            let mut cats: Vec<(String, String, String, String, bool, Vec<String>)> = Vec::new();
            for r in cat_rows {
                let (id, name, color, icon, builtin) =
                    r.db()?;
                cats.push((id, name, color, icon, builtin, Vec::new()));
            }

            let mut stmt2 = conn
                .prepare_cached(
                    "SELECT process_name, category_id FROM app_categories
                     WHERE deleted_at IS NULL
                     ORDER BY process_name",
                )
                .db()?;
            let map_rows = stmt2
                .query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .db()?;
            for r in map_rows {
                let (process, cat_id) = r.db()?;
                if let Some(c) = cats.iter_mut().find(|c| c.0 == cat_id) {
                    c.5.push(process);
                }
            }
            Ok(cats)
        })
        .await?;

    Ok(cats
        .into_iter()
        .map(|(id, name, color, icon, builtin, apps)| Category {
            id,
            name,
            color,
            icon,
            builtin,
            apps,
        })
        .collect())
}

pub async fn create(pool: &DbPool, input: CategoryInput) -> Result<Category> {
    let id = uuid::Uuid::new_v4().to_string();
    let id_clone = id.clone();
    let name = input.name.trim().to_string();
    let color = input.color.trim().to_string();
    let icon = input.icon.trim().to_string();
    if name.is_empty() {
        return Err(Error::InvalidInput("分类名不能为空"));
    }
    if color.is_empty() {
        return Err(Error::InvalidInput("颜色不能为空"));
    }
    let final_icon = if icon.is_empty() { "Tag".to_string() } else { icon };
    let n = name.clone();
    let c = color.clone();
    let i = final_icon.clone();
    let updated = Utc::now().to_rfc3339();
    let updated_clone = updated.clone();

    pool.0
        .call(move |conn| {
            // 新分类默认放最后：sort_order = max(active sort_order) + 1
            let next_sort: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM categories WHERE deleted_at IS NULL",
                    [],
                    |r| r.get(0),
                )
                .db()?;
            conn.execute(
                "INSERT INTO categories(id, name, color, icon, builtin, sort_order, updated_at)
                 VALUES(?, ?, ?, ?, 0, ?, ?)",
                rusqlite::params![id_clone, n, c, i, next_sort, updated_clone],
            )
            .db()?;

            let payload = category_payload(&id_clone, &n, &c, &i, false, next_sort, &updated_clone, None);
            enqueue(conn, OutboxOp::Upsert, OutboxEntity::Category, &id_clone, &payload)
                .db()?;
            Ok(())
        })
        .await?;

    Ok(Category {
        id,
        name,
        color,
        icon: final_icon,
        builtin: false,
        apps: Vec::new(),
    })
}

pub async fn update(pool: &DbPool, id: &str, patch: CategoryPatch) -> Result<()> {
    let id = id.to_string();
    let updated = Utc::now().to_rfc3339();
    pool.0
        .call(move |conn| {
            // 读出当前行做基线
            let row: Option<(String, String, String, i64, i64)> = conn
                .query_row(
                    "SELECT name, color, icon, builtin, sort_order FROM categories
                     WHERE id = ? AND deleted_at IS NULL",
                    rusqlite::params![id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .ok();
            let Some((cur_name, cur_color, cur_icon, builtin_i, cur_sort)) = row else {
                return Ok(());
            };

            let next_name = patch
                .name
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or(cur_name);
            let next_color = patch
                .color
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or(cur_color);
            let next_icon = patch
                .icon
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or(cur_icon);

            conn.execute(
                "UPDATE categories SET name = ?, color = ?, icon = ?, updated_at = ? WHERE id = ?",
                rusqlite::params![next_name, next_color, next_icon, updated, id],
            )
            .db()?;

            let payload = category_payload(&id, &next_name, &next_color, &next_icon, builtin_i != 0, cur_sort, &updated, None);
            enqueue(conn, OutboxOp::Upsert, OutboxEntity::Category, &id, &payload)
                .db()?;

            Ok(())
        })
        .await?;
    Ok(())
}

/// 用户拖拽重排：把 ordered_ids 列表里每个 id 的 sort_order 设为它在列表中的位置。
/// 仅对 sort_order 实际变了的行 enqueue outbox（幂等：原地拖一下不重复推）。
/// `updated_at` 也 bump，保证跨设备 LWW 拿到的是新顺序。
pub async fn reorder(pool: &DbPool, ordered_ids: Vec<String>) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    pool.0
        .call(move |conn| {
            for (idx, id) in ordered_ids.iter().enumerate() {
                let next_sort = idx as i64;
                // 拿当前行做基线（payload 里要带完整字段）
                let row: Option<(String, String, String, i64, i64)> = conn
                    .query_row(
                        "SELECT name, color, icon, builtin, sort_order FROM categories
                         WHERE id = ?1 AND deleted_at IS NULL",
                        rusqlite::params![id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                    )
                    .ok();
                let Some((name, color, icon, builtin_i, cur_sort)) = row else {
                    continue;
                };
                if cur_sort == next_sort {
                    continue; // 没变，幂等跳过
                }
                conn.execute(
                    "UPDATE categories SET sort_order = ?1, updated_at = ?2
                     WHERE id = ?3 AND deleted_at IS NULL",
                    rusqlite::params![next_sort, now, id],
                )
                .db()?;
                let payload =
                    category_payload(id, &name, &color, &icon, builtin_i != 0, next_sort, &now, None);
                enqueue(conn, OutboxOp::Upsert, OutboxEntity::Category, id, &payload)
                    .db()?;
            }
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn delete(pool: &DbPool, id: &str) -> Result<()> {
    let id = id.to_string();
    let now = Utc::now().to_rfc3339();
    // 闭包返回 Ok(Err(msg)) 表示业务校验拒绝，外层翻译成 Error::InvalidInput；
    // 真正的 db 错误仍走 ? 通道。这样 InvalidInput 不会被 tokio_rusqlite::Error::Other 包一层。
    let outcome: std::result::Result<(), &'static str> = pool
        .0
        .call(move |conn| {
            let row: Option<(String, String, String, i64, i64)> = conn
                .query_row(
                    "SELECT name, color, icon, builtin, sort_order FROM categories
                     WHERE id = ? AND deleted_at IS NULL",
                    rusqlite::params![id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .ok();
            let Some((name, color, icon, builtin_i, sort_order)) = row else {
                return Ok(Ok(()));
            };
            if builtin_i != 0 {
                return Ok(Err("内置分类不可删除"));
            }

            conn.execute(
                "UPDATE categories SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2",
                rusqlite::params![now, id],
            )
            .db()?;

            let cat_payload = category_payload(&id, &name, &color, &icon, builtin_i != 0, sort_order, &now, Some(&now));
            enqueue(conn, OutboxOp::Upsert, OutboxEntity::Category, &id, &cat_payload)
                .db()?;

            cascade_category_deletion(conn, &id, &now)?;

            Ok(Ok(()))
        })
        .await?;
    outcome.map_err(Error::InvalidInput)
}

/// 分类被删（无论是用户本机操作还是同步收到的远端事件）后，把所有指向它的引用一起清掉。
/// 幂等：所有 UPDATE 都带 `WHERE ... IS NULL` / `WHERE category_id = ?` 这类条件，
/// 重复跑一次时受影响行数为 0，不会重复 enqueue outbox。
///
/// 清理两类引用：
///   1. app_categories.category_id = X & deleted_at IS NULL → 软删
///   2. app_groups.category_id = X & deleted_at IS NULL → 设 NULL（让组回到「未分类」）
///
/// 各自仅对实际受影响的行 enqueue outbox，所以外层多次调用是 cheap no-op。
pub fn cascade_category_deletion(
    conn: &Connection,
    category_id: &str,
    now: &str,
) -> rusqlite::Result<()> {
    // 1) app_categories：取出受影响的 process_name 再 UPDATE，同时给每条入 outbox。
    let mut stmt = conn.prepare(
        "SELECT process_name FROM app_categories
         WHERE category_id = ?1 AND deleted_at IS NULL",
    )?;
    let affected_processes: Vec<String> = stmt
        .query_map(rusqlite::params![category_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    for p in &affected_processes {
        conn.execute(
            "UPDATE app_categories SET deleted_at = ?1, updated_at = ?1
             WHERE process_name = ?2 AND deleted_at IS NULL",
            rusqlite::params![now, p],
        )?;
        let payload = app_category_payload(p, category_id, now, Some(now));
        enqueue(conn, OutboxOp::Upsert, OutboxEntity::AppCategory, p, &payload)?;
    }

    // 2) app_groups：取出受影响的 group id 再清空 category_id + 入 outbox。
    let mut stmt = conn.prepare(
        "SELECT id FROM app_groups
         WHERE category_id = ?1 AND deleted_at IS NULL",
    )?;
    let affected_groups: Vec<String> = stmt
        .query_map(rusqlite::params![category_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    for g in &affected_groups {
        conn.execute(
            "UPDATE app_groups SET category_id = NULL, updated_at = ?1
             WHERE id = ?2 AND category_id IS NOT NULL",
            rusqlite::params![now, g],
        )?;
        let payload = serde_json::json!({ "groupId": g }).to_string();
        enqueue(conn, OutboxOp::Upsert, OutboxEntity::AppGroup, g, &payload)?;
    }

    Ok(())
}

/// 给应用绑定分类。走 app_groups 通道：找 process_name 所在的 group，给整组分类
/// （联动该组的所有成员，跨设备/跨平台名字一起更新）。
pub async fn assign_app(pool: &DbPool, process_name: &str, category_id: &str) -> Result<()> {
    let p = process_name.trim().to_string();
    let c = category_id.trim().to_string();
    if p.is_empty() {
        return Err(Error::InvalidInput("应用名不能为空"));
    }
    if c.is_empty() {
        return Err(Error::InvalidInput("分类 ID 不能为空"));
    }
    crate::repo::app_groups::assign_category_for_process(pool, &p, Some(c)).await
}

/// 取消应用分类。同走 app_groups：把组的 category_id 置 NULL。
pub async fn unassign_app(pool: &DbPool, process_name: &str) -> Result<()> {
    crate::repo::app_groups::assign_category_for_process(pool, process_name, None).await
}

pub async fn list_unclassified(pool: &DbPool, days_back: u32) -> Result<Vec<UnclassifiedApp>> {
    let days = days_back.max(1) as i64;
    let rows = pool
        .0
        .call(move |conn| {
            // 双 LEFT JOIN：m 是 active app_categories 行，c 是 m 指向的、active 的 category。
            // 「未归类」= 找不到 active mapping 或 mapping 指向已删/不存在的分类。
            // 这层防御让即便 cascade 因 sync 时序错过、有 stale app_categories 残留，
            // UI 还是能正确把那些 app 显示在未归类里。
            let mut stmt = conn
                .prepare_cached(
                    "SELECT a.process_name,
                            CAST(SUM(a.duration_secs) / 60 AS INTEGER) AS minutes,
                            MAX(a.ended_at) AS last_seen_at
                     FROM activities a
                     LEFT JOIN app_categories m
                       ON m.process_name = a.process_name AND m.deleted_at IS NULL
                     LEFT JOIN categories c
                       ON c.id = m.category_id AND c.deleted_at IS NULL
                     WHERE c.id IS NULL
                       AND a.local_date >= date('now','localtime', '-' || ?1 || ' days')
                       AND a.process_name <> 'Unknown'
                     GROUP BY a.process_name
                     ORDER BY minutes DESC",
                )
                .db()?;
            let it = stmt
                .query_map(rusqlite::params![days], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .db()?;
            let mut out = Vec::new();
            for r in it {
                out.push(r.db()?);
            }
            Ok(out)
        })
        .await?;

    Ok(rows
        .into_iter()
        .map(|(process_name, minutes, last_seen_at)| UnclassifiedApp {
            process_name,
            minutes: minutes.max(0) as u32,
            last_seen_at,
        })
        .collect())
}
