//! 分类表的 repo 层：CRUD + 同步 outbox 入队 + cascade 删除。
//!
//! 所有写入都同步入 outbox 走 push 路径，保证跨设备 LWW；
//! 内置分类（builtin=1）拒绝删除（必须给所有未分类的 app 一个落点）。

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::repo::outbox::{enqueue, OutboxEntity, OutboxOp};
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};

/// 分类（DB 行 + 该分类下的 app process_name 列表，用于前端渲染）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    /// 分类 ID（内置分类是 'work' / 'play' 等短词；用户建的是 UUID）
    pub id: String,
    /// 显示名
    pub name: String,
    /// hex 颜色 `#rrggbb`
    pub color: String,
    /// 图标 ID（前端用来 map 到 lucide-react 图标）
    pub icon: String,
    /// 是否内置分类（不可删除）
    pub builtin: bool,
    /// 当前归到该分类下的 process_name 列表（按字母序）
    pub apps: Vec<String>,
    /// 所属大类 id（NULL = 未归入大类，UI 渲染在"未归入"行）。v28 引入。
    pub super_category_id: Option<String>,
}

/// 新建分类时前端传过来的字段（不含 id —— 后端生成 UUID）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryInput {
    pub name: String,
    pub color: String,
    pub icon: String,
}

/// 更新分类时的 patch：每个字段 `None` 表示不动。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryPatch {
    pub name: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
}

/// 未归类应用的一行——给「分类」页面"待归类"卡片用。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnclassifiedApp {
    pub process_name: String,
    /// 最近 N 天累计使用分钟数
    pub minutes: u32,
    /// 最近一次出现的 RFC3339 时间
    pub last_seen_at: String,
}

// 拼 outbox payload 是 fan-in 8 个字段的 helper，参数数 = 表列数；
// 拆 struct 后调用方反而要先 build 一遍，纯增加噪声
#[allow(clippy::too_many_arguments)]
fn category_payload(
    id: &str,
    name: &str,
    color: &str,
    icon: &str,
    builtin: bool,
    sort_order: i64,
    updated_at: &str,
    deleted_at: Option<&str>,
) -> String {
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

fn app_category_payload(
    process_name: &str,
    category_id: &str,
    updated_at: &str,
    deleted_at: Option<&str>,
) -> String {
    serde_json::json!({
        "processName": process_name,
        "categoryId": category_id,
        "updatedAt": updated_at,
        "deletedAt": deleted_at,
    })
    .to_string()
}

/// 列所有 active 分类（按 sort_order 升序），每条带它当前归类的 process_name 列表。
pub async fn list(pool: &DbPool) -> Result<Vec<Category>> {
    // 累积器：每行临时存 base 字段 + 待填充的 apps 列表。
    // 用具名 struct 而不是 7-tuple 让 clippy::type_complexity 满意。
    struct CatAccum {
        id: String,
        name: String,
        color: String,
        icon: String,
        builtin: bool,
        super_category_id: Option<String>,
        apps: Vec<String>,
    }

    let cats = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare_cached(
                    // 用户拖拽排序后的 sort_order 决定显示顺序；id 作为 tiebreaker。
                    // super_category_id 跟 v28 大类绑定；NULL = 未归入大类。
                    "SELECT id, name, color, icon, builtin, super_category_id FROM categories
                     WHERE deleted_at IS NULL
                     ORDER BY sort_order ASC, id ASC",
                )
                .db()?;
            let cat_rows = stmt
                .query_map([], |r| {
                    Ok(CatAccum {
                        id: r.get::<_, String>(0)?,
                        name: r.get::<_, String>(1)?,
                        color: r.get::<_, String>(2)?,
                        icon: r.get::<_, String>(3)?,
                        builtin: r.get::<_, i64>(4)? != 0,
                        super_category_id: r.get::<_, Option<String>>(5)?,
                        apps: Vec::new(),
                    })
                })
                .db()?;
            let mut cats: Vec<CatAccum> = Vec::new();
            for r in cat_rows {
                cats.push(r.db()?);
            }

            // 旧实现读 `app_categories` 镜像表 —— 当 app_groups.category_id 已被
            // 设上但 mirror 没及时同步（如 backfill_builtin_categories 跑过但 sync 路径
            // 没补 mirror、或新 member 加入已分类组时漏 sync），UI 上分类下就显示
            // "暂无绑定应用" —— 即便 rankings / 日报里这个 app 已经被正确归类。
            // 现在直接走真实源 app_group_members + app_groups.category_id，避开镜像 lag。
            let mut stmt2 = conn
                .prepare_cached(
                    "SELECT m.process_name, g.category_id
                       FROM app_group_members m
                       JOIN app_groups g ON g.id = m.group_id
                                        AND g.deleted_at IS NULL
                      WHERE m.deleted_at IS NULL
                        AND g.category_id IS NOT NULL
                      ORDER BY m.process_name",
                )
                .db()?;
            let map_rows = stmt2
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .db()?;
            for r in map_rows {
                let (process, cat_id) = r.db()?;
                if let Some(c) = cats.iter_mut().find(|c| c.id == cat_id) {
                    c.apps.push(process);
                }
            }
            Ok(cats)
        })
        .await?;

    Ok(cats
        .into_iter()
        .map(|c| Category {
            id: c.id,
            name: c.name,
            color: c.color,
            icon: c.icon,
            builtin: c.builtin,
            apps: c.apps,
            super_category_id: c.super_category_id,
        })
        .collect())
}

/// 新建分类：UUID + 排到末尾 + 同步入 outbox。
/// 名字 / 颜色 trim 后空字符串拒绝（`Error::InvalidInput`）。
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
    let final_icon = if icon.is_empty() {
        "Tag".to_string()
    } else {
        icon
    };
    let n = name.clone();
    let c = color.clone();
    let i = final_icon.clone();
    let updated = utc_now_rfc3339();
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
        super_category_id: None,
    })
}

/// 更新分类的 name / color / icon。patch 中为 None 或空字符串的字段保持不变。
/// 内置分类也允许 update（仅改外观，不改 id / builtin 标志）。
pub async fn update(pool: &DbPool, id: &str, patch: CategoryPatch) -> Result<()> {
    let id = id.to_string();
    let updated = utc_now_rfc3339();
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

            let payload = category_payload(
                &id,
                &next_name,
                &next_color,
                &next_icon,
                builtin_i != 0,
                cur_sort,
                &updated,
                None,
            );
            enqueue(
                conn,
                OutboxOp::Upsert,
                OutboxEntity::Category,
                &id,
                &payload,
            )
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
    let now = utc_now_rfc3339();
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
                let payload = category_payload(
                    id,
                    &name,
                    &color,
                    &icon,
                    builtin_i != 0,
                    next_sort,
                    &now,
                    None,
                );
                enqueue(conn, OutboxOp::Upsert, OutboxEntity::Category, id, &payload).db()?;
            }
            Ok(())
        })
        .await?;
    Ok(())
}

/// 软删分类。内置分类拒绝（返回 `Error::InvalidInput`）。
/// 删除后通过 [`cascade_category_deletion`] 把所有指向该分类的 app_categories 行 +
/// app_groups.category_id 引用一起清掉。
pub async fn delete(pool: &DbPool, id: &str) -> Result<()> {
    let id = id.to_string();
    let now = utc_now_rfc3339();
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
            // 'other' 虽然 seed 时 builtin=0，但它是所有未分类时长的隐式归属
            //（reports SQL 里 COALESCE(c.id, 'other')）：删掉后 SQL 仍然产出
            // 'other'，前端解析不到分类，柱状图/占比会出现无色缺口
            if id == "other" {
                return Ok(Err("「其他」是未分类时长的默认归属，不可删除"));
            }

            conn.execute(
                "UPDATE categories SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2",
                rusqlite::params![now, id],
            )
            .db()?;

            let cat_payload = category_payload(
                &id,
                &name,
                &color,
                &icon,
                builtin_i != 0,
                sort_order,
                &now,
                Some(&now),
            );
            enqueue(
                conn,
                OutboxOp::Upsert,
                OutboxEntity::Category,
                &id,
                &cat_payload,
            )
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
        enqueue(
            conn,
            OutboxOp::Upsert,
            OutboxEntity::AppCategory,
            p,
            &payload,
        )?;
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

/// 列最近 `days_back` 天里活动过、但没归到任何 active 分类的 process_name。
/// 双 LEFT JOIN 防御 cascade 失误：mapping 指向已删分类的也算未分类。
pub async fn list_unclassified(pool: &DbPool, days_back: u32) -> Result<Vec<UnclassifiedApp>> {
    let days = days_back.max(1) as i64;
    let rows = pool
        .0
        .call(move |conn| {
            // 「未归类」的判定走**真实源** app_group_members → app_groups.category_id，
            // 而不是 app_categories 镜像表——镜像会滞后（新成员并入已归类组时不一定
            // 补 mirror，见 categories::list 里同款修正的注释）：按镜像判会把报表里
            // 已经正确归类的 app 错误地留在"待分类"卡片里。
            // 三层 LEFT JOIN 防御：member 缺失 / group 已删 / category 已删都算未归类。
            let mut stmt = conn
                .prepare_cached(
                    "SELECT a.process_name,
                            CAST(SUM(a.duration_secs) / 60 AS INTEGER) AS minutes,
                            MAX(a.ended_at) AS last_seen_at
                     FROM activities a
                     LEFT JOIN app_group_members m
                       ON m.process_name = a.process_name AND m.deleted_at IS NULL
                     LEFT JOIN app_groups g
                       ON g.id = m.group_id AND g.deleted_at IS NULL
                     LEFT JOIN categories c
                       ON c.id = g.category_id AND c.deleted_at IS NULL
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::fresh_test_pool;

    /// 钉死 bug：当 app_group_members + app_groups.category_id 有数据但 app_categories
    /// 镜像表为空时（典型 backfill 漏镜像 / sync 顺序错位），categories::list 仍应
    /// 返回该 process_name —— 因为现在直接读真实源而不是镜像表。
    ///
    /// 旧实现（读 app_categories）下：apps 列表会是空，UI 显示"暂无绑定应用"。
    /// 新实现（JOIN app_group_members + app_groups）：直接拿到 process_name。
    #[tokio::test]
    async fn list_returns_app_when_only_app_groups_has_category_no_app_categories_mirror() {
        let pool = fresh_test_pool().await;

        // 模拟 capture 写入：建组（带 category）+ 加成员；**故意不写 app_categories 镜像**。
        pool.0
            .call(|conn| {
                let now = "2026-05-17T10:00:00Z";
                // 组 "Visual Studio Code" 归类到 builtin "code"（categories 表已 seed）
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('Visual Studio Code', 'Visual Studio Code', 'code', ?1, NULL)",
                    rusqlite::params![now],
                )?;
                // mac 进程名 "Code" 归到这个组
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('Code', 'Visual Studio Code', ?1, NULL)",
                    rusqlite::params![now],
                )?;
                // **故意不**写 app_categories —— 模拟镜像 lag
                Ok(())
            })
            .await
            .unwrap();

        let cats = list(&pool).await.unwrap();
        let code = cats
            .iter()
            .find(|c| c.id == "code")
            .expect("'code' 内置分类应该存在");
        assert!(
            code.apps.iter().any(|p| p == "Code"),
            "镜像表为空时也应该能看到 Code，实际 apps={:?}",
            code.apps,
        );
    }

    /// 反例：当 app_groups.category_id IS NULL（未分类）时，**不**应出现在任何分类的 apps 里。
    #[tokio::test]
    async fn list_excludes_app_when_group_has_no_category() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|conn| {
                let now = "2026-05-17T10:00:00Z";
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('SomeApp', 'SomeApp', NULL, ?1, NULL)",
                    rusqlite::params![now],
                )?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('SomeApp', 'SomeApp', ?1, NULL)",
                    rusqlite::params![now],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let cats = list(&pool).await.unwrap();
        for c in &cats {
            assert!(
                !c.apps.iter().any(|p| p == "SomeApp"),
                "未分类组的成员不应出现在任何分类下，但 {} 包含: {:?}",
                c.id,
                c.apps,
            );
        }
    }

    /// 反例：软删除的 group / member 不应被列出。
    #[tokio::test]
    async fn list_excludes_soft_deleted_groups_and_members() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|conn| {
                let now = "2026-05-17T10:00:00Z";
                // 软删的 group：成员还在，但 group 不算 active
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('DeletedGroup', 'DeletedGroup', 'code', ?1, ?1)",
                    rusqlite::params![now],
                )?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('AppInDeletedGroup', 'DeletedGroup', ?1, NULL)",
                    rusqlite::params![now],
                )?;
                // active group 但软删的 member
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('LiveGroup', 'LiveGroup', 'code', ?1, NULL)",
                    rusqlite::params![now],
                )?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES('DeletedMember', 'LiveGroup', ?1, ?1)",
                    rusqlite::params![now],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let cats = list(&pool).await.unwrap();
        let code = cats.iter().find(|c| c.id == "code").unwrap();
        assert!(!code.apps.iter().any(|p| p == "AppInDeletedGroup"));
        assert!(!code.apps.iter().any(|p| p == "DeletedMember"));
    }
}
