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

    // ---------- 共享小工具 ----------

    fn cat_input(name: &str, color: &str, icon: &str) -> CategoryInput {
        CategoryInput {
            name: name.into(),
            color: color.into(),
            icon: icon.into(),
        }
    }

    /// 绕过 `deleted_at IS NULL` 过滤直接读原始行（测软删语义必须能看到 tombstone）。
    /// 返回 (name, color, icon, builtin, sort_order, deleted_at)。
    async fn raw_cat(
        pool: &DbPool,
        id: &str,
    ) -> Option<(String, String, String, i64, i64, Option<String>)> {
        let id = id.to_string();
        pool.0
            .call(move |conn| {
                use rusqlite::OptionalExtension;
                let row = conn
                    .query_row(
                        "SELECT name, color, icon, builtin, sort_order, deleted_at
                           FROM categories WHERE id = ?1",
                        rusqlite::params![id],
                        |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, String>(2)?,
                                r.get::<_, i64>(3)?,
                                r.get::<_, i64>(4)?,
                                r.get::<_, Option<String>>(5)?,
                            ))
                        },
                    )
                    .optional()?;
                Ok(row)
            })
            .await
            .unwrap()
    }

    async fn find_cat(pool: &DbPool, id: &str) -> Option<Category> {
        list(pool).await.unwrap().into_iter().find(|c| c.id == id)
    }

    async fn list_ids(pool: &DbPool) -> Vec<String> {
        list(pool)
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect()
    }

    async fn outbox_count(pool: &DbPool, entity: &str) -> i64 {
        let e = entity.to_string();
        pool.0
            .call(move |conn| {
                let n = conn.query_row(
                    "SELECT COUNT(*) FROM sync_outbox WHERE entity = ?1",
                    rusqlite::params![e],
                    |r| r.get::<_, i64>(0),
                )?;
                Ok(n)
            })
            .await
            .unwrap()
    }

    async fn outbox_total(pool: &DbPool) -> i64 {
        pool.0
            .call(|conn| {
                let n = conn.query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| {
                    r.get::<_, i64>(0)
                })?;
                Ok(n)
            })
            .await
            .unwrap()
    }

    // ---------- create ----------

    /// 为什么测：前端表单可能提交全空格（用户误敲空格直接确认）。trim 后必须拒绝，
    /// 否则列表里出现"看不见"的分类，既点不中也删不掉。icon 留空则要兜底成
    /// 非空默认值——空 icon 前端 map 不到 lucide 组件会渲染裂图。
    #[tokio::test]
    async fn create_rejects_blank_name_or_color_and_defaults_blank_icon() {
        let pool = fresh_test_pool().await;
        let baseline = list(&pool).await.unwrap().len();

        let err = create(&pool, cat_input("   ", "#123456", "Star"))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "全空格名必须被拒");
        let err = create(&pool, cat_input("阅读", "  ", "Star"))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "全空格颜色必须被拒");
        // 校验失败不能留下半截行
        assert_eq!(list(&pool).await.unwrap().len(), baseline);

        let cat = create(&pool, cat_input(" 阅读 ", " #123456 ", "  "))
            .await
            .unwrap();
        let got = find_cat(&pool, &cat.id).await.unwrap();
        assert_eq!(got.name, "阅读", "名字应 trim 后入库");
        assert_eq!(got.color, "#123456", "颜色应 trim 后入库");
        assert!(!got.icon.trim().is_empty(), "空 icon 必须兜底成非空默认值");
        assert!(!got.builtin, "用户新建的分类不能带 builtin 标志");
        // create 返回值与 list 读回的一致（前端拿返回值直接插 UI，不再刷新列表）
        assert_eq!(got.icon, cat.icon);
    }

    /// 为什么测：sort_order 用 MAX(active)+1 生成。若把软删行也算进 MAX，
    /// 用户反复"建了删、删了建"会让 sort_order 无限膨胀，跨设备 LWW 合并时
    /// 新分类和 tombstone 争位置导致顺序错乱。
    #[tokio::test]
    async fn create_appends_to_end_and_soft_deleted_rows_do_not_inflate_sort_order() {
        let pool = fresh_test_pool().await;
        let a = create(&pool, cat_input("甲", "#111111", "Star"))
            .await
            .unwrap();
        let b = create(&pool, cat_input("乙", "#222222", "Moon"))
            .await
            .unwrap();
        let ids = list_ids(&pool).await;
        // 新建的排在列表末尾且保持创建序
        assert_eq!(ids[ids.len() - 2], a.id);
        assert_eq!(ids[ids.len() - 1], b.id);

        let sort_b = raw_cat(&pool, &b.id).await.unwrap().4;
        delete(&pool, &b.id).await.unwrap();
        let c = create(&pool, cat_input("丙", "#333333", "Sun"))
            .await
            .unwrap();
        let sort_c = raw_cat(&pool, &c.id).await.unwrap().4;
        // 乙软删后它的位置应被回收：丙拿到与乙相同的 sort_order，而不是继续 +1
        assert_eq!(sort_c, sort_b);
        assert_eq!(list_ids(&pool).await.last().unwrap(), &c.id);
    }

    // ---------- update ----------

    /// 为什么测：前端"重命名"弹窗只传 name、调色板只传 color、图标选择器只传 icon。
    /// patch 里没给的字段绝不能被冲掉；空白字符串（用户清空输入框直接确认）按"不改"处理。
    #[tokio::test]
    async fn update_patches_each_field_independently_and_treats_blank_as_keep() {
        let pool = fresh_test_pool().await;
        let cat = create(&pool, cat_input("临时", "#111111", "Star"))
            .await
            .unwrap();

        update(
            &pool,
            &cat.id,
            CategoryPatch {
                name: Some("改名".into()),
                color: None,
                icon: None,
            },
        )
        .await
        .unwrap();
        let got = find_cat(&pool, &cat.id).await.unwrap();
        assert_eq!(got.name, "改名");
        assert_eq!(got.color, "#111111", "只改 name 不得动 color");
        assert_eq!(got.icon, "Star", "只改 name 不得动 icon");

        update(
            &pool,
            &cat.id,
            CategoryPatch {
                name: None,
                color: Some(" #222222 ".into()),
                icon: None,
            },
        )
        .await
        .unwrap();
        let got = find_cat(&pool, &cat.id).await.unwrap();
        assert_eq!(got.color, "#222222", "color 应 trim 后入库");
        assert_eq!(got.name, "改名");
        assert_eq!(got.icon, "Star");

        update(
            &pool,
            &cat.id,
            CategoryPatch {
                name: None,
                color: None,
                icon: Some("Moon".into()),
            },
        )
        .await
        .unwrap();
        let got = find_cat(&pool, &cat.id).await.unwrap();
        assert_eq!(got.icon, "Moon");
        assert_eq!(got.name, "改名");
        assert_eq!(got.color, "#222222");

        // 三个字段全给空白 → 全部视为"不改"，不能把行清空
        update(
            &pool,
            &cat.id,
            CategoryPatch {
                name: Some("   ".into()),
                color: Some(String::new()),
                icon: Some(" ".into()),
            },
        )
        .await
        .unwrap();
        let got = find_cat(&pool, &cat.id).await.unwrap();
        assert_eq!(
            (got.name.as_str(), got.color.as_str(), got.icon.as_str()),
            ("改名", "#222222", "Moon"),
            "空白 patch 不得清掉任何字段"
        );
    }

    /// 为什么测：同步竞态下 update 可能落在已被另一台设备删掉的分类上。
    /// 期望静默 no-op：既不报错（用户无感），也绝不能把 tombstone 复活，
    /// 更不能给 no-op 入 outbox（否则重试风暴把垃圾事件推上云）。
    #[tokio::test]
    async fn update_ignores_missing_and_soft_deleted_rows() {
        let pool = fresh_test_pool().await;
        // 不存在的 id → 静默成功
        update(
            &pool,
            "ghost-id",
            CategoryPatch {
                name: Some("X".into()),
                color: None,
                icon: None,
            },
        )
        .await
        .unwrap();

        let cat = create(&pool, cat_input("将删", "#111111", "Star"))
            .await
            .unwrap();
        delete(&pool, &cat.id).await.unwrap();
        let before = outbox_count(&pool, "category").await;

        update(
            &pool,
            &cat.id,
            CategoryPatch {
                name: Some("复活?".into()),
                color: None,
                icon: None,
            },
        )
        .await
        .unwrap();

        let row = raw_cat(&pool, &cat.id).await.unwrap();
        assert!(row.5.is_some(), "update 不得清掉 deleted_at 把分类复活");
        assert_ne!(row.0, "复活?", "软删行的字段不应被改写");
        assert_eq!(
            outbox_count(&pool, "category").await,
            before,
            "对软删行的 update 是 no-op，不应入 outbox"
        );
    }

    // ---------- delete / 软删语义 ----------

    /// 为什么测：删除必须是软删——跨设备同步靠 tombstone 行传播删除事件；
    /// 物理删会让另一台设备把该分类原样推回来（"删不掉"复活 bug）。
    #[tokio::test]
    async fn soft_delete_hides_from_list_but_keeps_row_and_pushes_tombstone() {
        let pool = fresh_test_pool().await;
        let cat = create(&pool, cat_input("短命", "#111111", "Star"))
            .await
            .unwrap();
        delete(&pool, &cat.id).await.unwrap();

        assert!(
            list(&pool).await.unwrap().iter().all(|c| c.id != cat.id),
            "软删后不应再出现在 list 里"
        );
        let row = raw_cat(&pool, &cat.id)
            .await
            .expect("行必须还在（软删不是物理删）");
        assert!(row.5.is_some(), "deleted_at 必须被打上");

        // outbox 里必须有一条带 deletedAt 的 category 快照，云端才能感知删除
        let cid = cat.id.clone();
        let payloads: Vec<String> = pool
            .0
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT payload FROM sync_outbox
                      WHERE entity = 'category' AND entity_pk = ?1",
                )?;
                let rows = stmt.query_map(rusqlite::params![cid], |r| r.get::<_, String>(0))?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                Ok(out)
            })
            .await
            .unwrap();
        let has_tombstone = payloads.iter().any(|p| {
            serde_json::from_str::<serde_json::Value>(p)
                .map(|v| v.get("deletedAt").map(|d| d.is_string()).unwrap_or(false))
                .unwrap_or(false)
        });
        assert!(
            has_tombstone,
            "outbox 里必须有带 deletedAt 的 tombstone，实际 payloads={payloads:?}"
        );
    }

    /// 为什么测：双端并发删同一分类时，后到的删除请求看到的已是 tombstone / 空行。
    /// 应静默成功且不再入 outbox，否则每次重放都往云端推垃圾事件。
    #[tokio::test]
    async fn delete_missing_or_already_deleted_is_silent_noop() {
        let pool = fresh_test_pool().await;
        let before = outbox_count(&pool, "category").await;
        delete(&pool, "no-such-id").await.unwrap();
        assert_eq!(
            outbox_count(&pool, "category").await,
            before,
            "删不存在的 id 不应入 outbox"
        );

        let cat = create(&pool, cat_input("重复删", "#111111", "Star"))
            .await
            .unwrap();
        delete(&pool, &cat.id).await.unwrap();
        let mid = outbox_count(&pool, "category").await;
        delete(&pool, &cat.id).await.unwrap(); // 第二次删同一个
        assert_eq!(
            outbox_count(&pool, "category").await,
            mid,
            "重复删除是 no-op，不应再入 outbox"
        );
    }

    // ---------- 内置 / 特殊分类约束 ----------

    /// 为什么测：'hidden' 是"从统计里排除应用"的功能锚点（v27 里唯一 builtin=1 的行）。
    /// 一旦被删，被隐藏的 app 全部回流进报表——用户特意隐藏的内容重新出现在
    /// 日报 / AI 总结里，隐私场景直接翻车。但外观（名字/颜色/图标）允许个性化，
    /// 且改完外观后 builtin 守门必须依然生效。
    #[tokio::test]
    async fn builtin_hidden_rejects_delete_but_allows_restyle() {
        let pool = fresh_test_pool().await;
        let err = delete(&pool, "hidden").await.unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
        let hidden = find_cat(&pool, "hidden")
            .await
            .expect("拒绝删除后 hidden 必须原样活着");
        assert!(hidden.builtin);

        update(
            &pool,
            "hidden",
            CategoryPatch {
                name: Some("不给看".into()),
                color: None,
                icon: None,
            },
        )
        .await
        .unwrap();
        let hidden = find_cat(&pool, "hidden").await.unwrap();
        assert_eq!(hidden.name, "不给看", "内置分类允许改外观");
        assert!(
            hidden.builtin,
            "update 不得清掉 builtin 标志，否则改过名的 hidden 就能被删了"
        );
        // 改完名依然不可删
        assert!(matches!(
            delete(&pool, "hidden").await.unwrap_err(),
            Error::InvalidInput(_)
        ));
    }

    /// 为什么测：'other' 在 seed 里 builtin=0，光靠 builtin 守门拦不住；但报表 SQL
    /// 把所有未分类时长 COALESCE 到 'other'，删掉后前端解析不到分类，图表出现
    /// 无色缺口。这里钉死针对 id 的专门守门分支。
    #[tokio::test]
    async fn other_category_rejects_delete_despite_not_builtin() {
        let pool = fresh_test_pool().await;
        let other = find_cat(&pool, "other").await.expect("seed 应有 other");
        // 前提校验：other 确实不是 builtin —— 若某天 seed 改成 builtin=1，
        // 此测试就该换成测 builtin 分支而不是 id 分支
        assert!(!other.builtin);

        let err = delete(&pool, "other").await.unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
        assert!(
            find_cat(&pool, "other").await.is_some(),
            "拒绝删除后 other 必须仍在列表"
        );
    }

    // ---------- reorder / sort_order ----------

    /// 为什么测：拖拽重排是高频操作。1) 新顺序必须真实持久化（list 按 sort_order 出）；
    /// 2) 顺序没变的 reorder 不能重复入 outbox，否则前端每次 render 后补发的
    /// no-op reorder 都会把全量分类快照推上云。
    #[tokio::test]
    async fn reorder_applies_index_order_and_noop_reorder_skips_outbox() {
        let pool = fresh_test_pool().await;
        let before = list_ids(&pool).await;
        assert!(before.len() >= 2, "seed 应至少有两个分类才能测重排");
        let reversed: Vec<String> = before.iter().rev().cloned().collect();

        reorder(&pool, reversed.clone()).await.unwrap();
        assert_eq!(list_ids(&pool).await, reversed, "list 顺序应跟随 reorder");

        let n = outbox_count(&pool, "category").await;
        reorder(&pool, reversed.clone()).await.unwrap(); // 原地不动
        assert_eq!(
            outbox_count(&pool, "category").await,
            n,
            "顺序没变时不应产生新的 outbox 行"
        );
    }

    /// 为什么测：前端列表和 DB 可能瞬时不同步（另一台设备刚删了一个分类，本机还
    /// 基于旧列表发起拖拽）。带幽灵 id / 已删 id 的 reorder 不应报错、不应打乱
    /// 其余顺序、更不能把软删行拖活。
    #[tokio::test]
    async fn reorder_skips_unknown_and_soft_deleted_ids() {
        let pool = fresh_test_pool().await;
        let dead = create(&pool, cat_input("已删", "#111111", "Star"))
            .await
            .unwrap();
        delete(&pool, &dead.id).await.unwrap();

        let live = list_ids(&pool).await;
        let mut req = vec!["ghost-id".to_string(), dead.id.clone()];
        req.extend(live.iter().rev().cloned());
        reorder(&pool, req).await.unwrap();

        let after = list_ids(&pool).await;
        let expected: Vec<String> = live.iter().rev().cloned().collect();
        assert_eq!(after, expected, "幽灵 id 应被跳过，其余按索引就位");
        assert!(
            after.iter().all(|id| id != &dead.id),
            "软删行不得因 reorder 复活"
        );
    }

    // ---------- 删分类的 cascade：成员归属 ----------

    /// 为什么测：删分类时若不清 app_groups.category_id，组还挂在幽灵分类上——
    /// "待归类"卡片不出现该 app、报表又解析不到分类，两边都看不见它。
    /// 期望：组降级回未分类（而不是连坐删组），app_categories 镜像行同步软删。
    #[tokio::test]
    async fn delete_returns_member_group_to_unclassified_and_soft_deletes_mirror() {
        let pool = fresh_test_pool().await;
        let cat = create(&pool, cat_input("工具", "#111111", "Wrench"))
            .await
            .unwrap();
        assign_app(&pool, "MyTool", &cat.id).await.unwrap();
        // 前置：绑定成功
        let got = find_cat(&pool, &cat.id).await.unwrap();
        assert!(got.apps.iter().any(|p| p == "MyTool"), "绑定应先生效");

        delete(&pool, &cat.id).await.unwrap();

        // 组还活着，但 category_id 被清空（回到未分类，而不是删组）
        let (g_cat, g_deleted): (Option<String>, Option<String>) = pool
            .0
            .call(|conn| {
                let row = conn.query_row(
                    "SELECT g.category_id, g.deleted_at
                       FROM app_group_members m
                       JOIN app_groups g ON g.id = m.group_id
                      WHERE m.process_name = 'MyTool'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                Ok(row)
            })
            .await
            .unwrap();
        assert!(g_deleted.is_none(), "删分类不应连坐删组");
        assert_eq!(g_cat, None, "组必须回到未分类");

        // app_categories 镜像行软删（保留 tombstone 供同步，不物理删）
        let mirror: Option<Option<String>> = pool
            .0
            .call(|conn| {
                use rusqlite::OptionalExtension;
                let row = conn
                    .query_row(
                        "SELECT deleted_at FROM app_categories WHERE process_name = 'MyTool'",
                        [],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .optional()?;
                Ok(row)
            })
            .await
            .unwrap();
        let mirror = mirror.expect("镜像行应保留（软删）而不是物理删");
        assert!(
            mirror.is_some(),
            "镜像行 deleted_at 必须被打上，否则报表继续按旧分类聚合"
        );

        // 任何分类下都不应再出现 MyTool
        assert!(list(&pool)
            .await
            .unwrap()
            .iter()
            .all(|c| !c.apps.iter().any(|p| p == "MyTool")));
    }

    /// 为什么测：cascade 在本机删除和 sync pull 两条路径上都会被调用；若不幂等，
    /// 每次 pull 都给同一批 app_categories / app_groups 重复入 outbox，
    /// 两台设备之间形成推送风暴。
    #[tokio::test]
    async fn cascade_second_run_is_noop_without_new_outbox_rows() {
        let pool = fresh_test_pool().await;
        let cat = create(&pool, cat_input("循环", "#111111", "Repeat"))
            .await
            .unwrap();
        assign_app(&pool, "LoopApp", &cat.id).await.unwrap();
        delete(&pool, &cat.id).await.unwrap(); // 内部已完整跑过一次 cascade

        let before = outbox_total(&pool).await;
        let cid = cat.id.clone();
        pool.0
            .call(move |conn| {
                cascade_category_deletion(conn, &cid, "2026-07-26T00:00:00Z")?;
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(
            outbox_total(&pool).await,
            before,
            "重复 cascade 不应新增任何 outbox 行"
        );
    }

    // ---------- list_unclassified ----------

    async fn insert_activity(
        pool: &DbPool,
        process: &str,
        day_offset: i64,
        secs: i64,
        ended: &str,
    ) {
        let p = process.to_string();
        let e = ended.to_string();
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activities(started_at, ended_at, duration_secs, local_date,
                                            local_hour, process_name, category_id)
                     VALUES(?1, ?2, ?3, date('now','localtime', ?4 || ' days'), 9, ?5, 'other')",
                    rusqlite::params![e, e, secs, day_offset.to_string(), p],
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// 为什么测："待归类"卡片的判定必须走 app_groups 真实源且守住窗口边界：
    /// - 已归类 app 不出现（否则用户被反复要求归类同一个 app）；
    /// - 组指向**已删分类**的 app 要重新出现（cascade 失误 / 远端 tombstone 未级联时的兜底）；
    /// - 'Unknown' 噪声行、窗口外的老记录都要滤掉；
    /// - 分钟数是聚合值（整数分钟），排序按用量降序，用户先处理大头。
    #[tokio::test]
    async fn list_unclassified_uses_group_truth_window_and_aggregation() {
        let pool = fresh_test_pool().await;

        // FreeApp：今天 90s + 45s 两段 → 135s = 2 整分钟（截断）
        insert_activity(&pool, "FreeApp", 0, 90, "2026-07-26T09:00:00Z").await;
        insert_activity(&pool, "FreeApp", 0, 45, "2026-07-26T10:30:00Z").await;
        // Unknown：采集兜底名，永远不该让用户归类
        insert_activity(&pool, "Unknown", 0, 600, "2026-07-26T09:00:00Z").await;
        // OldApp：10 天前的活动，5 天窗口内不应出现
        insert_activity(&pool, "OldApp", -10, 600, "2026-07-16T09:00:00Z").await;
        // CodeApp：已归类到 seed 的 'code'，不应出现
        insert_activity(&pool, "CodeApp", 0, 600, "2026-07-26T09:00:00Z").await;
        assign_app(&pool, "CodeApp", "code").await.unwrap();
        // ZombieApp：归到一个随后被"绕过 cascade"软删的分类 → 应回到待归类
        insert_activity(&pool, "ZombieApp", 0, 600, "2026-07-26T09:00:00Z").await;
        let zombie_cat = create(&pool, cat_input("僵尸", "#111111", "Ghost"))
            .await
            .unwrap();
        assign_app(&pool, "ZombieApp", &zombie_cat.id)
            .await
            .unwrap();
        let zid = zombie_cat.id.clone();
        pool.0
            .call(move |conn| {
                // 模拟远端 tombstone 直接落库、没跑 cascade 的失误路径
                conn.execute(
                    "UPDATE categories SET deleted_at = '2026-07-26T00:00:00Z' WHERE id = ?1",
                    rusqlite::params![zid],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        let rows = list_unclassified(&pool, 5).await.unwrap();
        let names: Vec<&str> = rows.iter().map(|r| r.process_name.as_str()).collect();

        assert!(names.contains(&"FreeApp"), "无组的 app 应待归类: {names:?}");
        assert!(
            names.contains(&"ZombieApp"),
            "组指向已删分类的 app 应回到待归类: {names:?}"
        );
        assert!(!names.contains(&"CodeApp"), "已归类的 app 不应出现");
        assert!(!names.contains(&"Unknown"), "Unknown 噪声行必须滤掉");
        assert!(!names.contains(&"OldApp"), "窗口外的老记录必须滤掉");

        let free = rows.iter().find(|r| r.process_name == "FreeApp").unwrap();
        assert_eq!(free.minutes, 2, "90s+45s=135s 应聚合成 2 整分钟");
        assert_eq!(
            free.last_seen_at, "2026-07-26T10:30:00Z",
            "last_seen 应取两段里较晚的 ended_at"
        );

        // 排序：ZombieApp 10 分钟 > FreeApp 2 分钟，大头在前
        let pos_zombie = names.iter().position(|n| *n == "ZombieApp").unwrap();
        let pos_free = names.iter().position(|n| *n == "FreeApp").unwrap();
        assert!(pos_zombie < pos_free, "应按分钟数降序: {names:?}");
    }
}
