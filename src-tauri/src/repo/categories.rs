use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::repo::outbox::{enqueue, OutboxEntity, OutboxOp};
use crate::storage::DbPool;

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

fn category_payload(id: &str, name: &str, color: &str, icon: &str, builtin: bool, updated_at: &str, deleted_at: Option<&str>) -> String {
    serde_json::json!({
        "id": id,
        "name": name,
        "color": color,
        "icon": icon,
        "builtin": builtin,
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
                    "SELECT id, name, color, icon, builtin FROM categories
                     WHERE deleted_at IS NULL
                     ORDER BY builtin DESC, id",
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
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
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let mut cats: Vec<(String, String, String, String, bool, Vec<String>)> = Vec::new();
            for r in cat_rows {
                let (id, name, color, icon, builtin) =
                    r.map_err(tokio_rusqlite::Error::Rusqlite)?;
                cats.push((id, name, color, icon, builtin, Vec::new()));
            }

            let mut stmt2 = conn
                .prepare_cached(
                    "SELECT process_name, category_id FROM app_categories
                     WHERE deleted_at IS NULL
                     ORDER BY process_name",
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let map_rows = stmt2
                .query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            for r in map_rows {
                let (process, cat_id) = r.map_err(tokio_rusqlite::Error::Rusqlite)?;
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
        return Err(Error::Other("分类名不能为空".into()));
    }
    if color.is_empty() {
        return Err(Error::Other("颜色不能为空".into()));
    }
    let final_icon = if icon.is_empty() { "Tag".to_string() } else { icon };
    let n = name.clone();
    let c = color.clone();
    let i = final_icon.clone();
    let updated = Utc::now().to_rfc3339();
    let updated_clone = updated.clone();

    pool.0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO categories(id, name, color, icon, builtin, updated_at)
                 VALUES(?, ?, ?, ?, 0, ?)",
                rusqlite::params![id_clone, n, c, i, updated_clone],
            )
            .map_err(tokio_rusqlite::Error::Rusqlite)?;

            let payload = category_payload(&id_clone, &n, &c, &i, false, &updated_clone, None);
            enqueue(conn, OutboxOp::Upsert, OutboxEntity::Category, &id_clone, &payload)
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
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
            let row: Option<(String, String, String, i64)> = conn
                .query_row(
                    "SELECT name, color, icon, builtin FROM categories WHERE id = ? AND deleted_at IS NULL",
                    rusqlite::params![id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .ok();
            let Some((cur_name, cur_color, cur_icon, builtin_i)) = row else {
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
            .map_err(tokio_rusqlite::Error::Rusqlite)?;

            let payload = category_payload(&id, &next_name, &next_color, &next_icon, builtin_i != 0, &updated, None);
            enqueue(conn, OutboxOp::Upsert, OutboxEntity::Category, &id, &payload)
                .map_err(tokio_rusqlite::Error::Rusqlite)?;

            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn delete(pool: &DbPool, id: &str) -> Result<()> {
    let id = id.to_string();
    let now = Utc::now().to_rfc3339();
    pool.0
        .call(move |conn| {
            // 读出元信息
            let row: Option<(String, String, String, i64)> = conn
                .query_row(
                    "SELECT name, color, icon, builtin FROM categories WHERE id = ? AND deleted_at IS NULL",
                    rusqlite::params![id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .ok();
            let Some((name, color, icon, builtin_i)) = row else {
                return Ok(());
            };
            if builtin_i != 0 {
                return Err(tokio_rusqlite::Error::Other("内置分类不可删除".into()));
            }

            // 软删 category
            conn.execute(
                "UPDATE categories SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2",
                rusqlite::params![now, id],
            )
            .map_err(tokio_rusqlite::Error::Rusqlite)?;

            let cat_payload = category_payload(&id, &name, &color, &icon, builtin_i != 0, &now, Some(&now));
            enqueue(conn, OutboxOp::Upsert, OutboxEntity::Category, &id, &cat_payload)
                .map_err(tokio_rusqlite::Error::Rusqlite)?;

            // 软删所有指向这个分类的 app_categories（同时写每条 outbox）
            let mut stmt = conn
                .prepare(
                    "SELECT process_name FROM app_categories
                     WHERE category_id = ? AND deleted_at IS NULL",
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let processes: Vec<String> = stmt
                .query_map(rusqlite::params![id], |r| r.get::<_, String>(0))
                .map_err(tokio_rusqlite::Error::Rusqlite)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            drop(stmt);

            for p in &processes {
                conn.execute(
                    "UPDATE app_categories SET deleted_at = ?1, updated_at = ?1 WHERE process_name = ?2",
                    rusqlite::params![now, p],
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
                let payload = app_category_payload(p, &id, &now, Some(&now));
                enqueue(conn, OutboxOp::Upsert, OutboxEntity::AppCategory, p, &payload)
                    .map_err(tokio_rusqlite::Error::Rusqlite)?;
            }

            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn assign_app(pool: &DbPool, process_name: &str, category_id: &str) -> Result<()> {
    let p = process_name.trim().to_string();
    let c = category_id.to_string();
    if p.is_empty() {
        return Err(Error::Other("应用名不能为空".into()));
    }
    let updated = Utc::now().to_rfc3339();
    let p_clone = p.clone();
    let c_clone = c.clone();
    let updated_clone = updated.clone();
    pool.0
        .call(move |conn| {
            // upsert + 重新激活（如果之前被软删）
            conn.execute(
                "INSERT INTO app_categories(process_name, category_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, NULL)
                 ON CONFLICT(process_name) DO UPDATE SET
                   category_id = excluded.category_id,
                   updated_at = excluded.updated_at,
                   deleted_at = NULL",
                rusqlite::params![p_clone, c_clone, updated_clone],
            )
            .map_err(tokio_rusqlite::Error::Rusqlite)?;

            let payload = app_category_payload(&p_clone, &c_clone, &updated_clone, None);
            enqueue(conn, OutboxOp::Upsert, OutboxEntity::AppCategory, &p_clone, &payload)
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn unassign_app(pool: &DbPool, process_name: &str) -> Result<()> {
    let p = process_name.to_string();
    let now = Utc::now().to_rfc3339();
    pool.0
        .call(move |conn| {
            // 先读 category_id（payload 用）
            let cat: Option<String> = conn
                .query_row(
                    "SELECT category_id FROM app_categories WHERE process_name = ? AND deleted_at IS NULL",
                    rusqlite::params![p],
                    |r| r.get(0),
                )
                .ok();
            let Some(cat) = cat else { return Ok(()); };

            conn.execute(
                "UPDATE app_categories SET deleted_at = ?1, updated_at = ?1 WHERE process_name = ?2",
                rusqlite::params![now, p],
            )
            .map_err(tokio_rusqlite::Error::Rusqlite)?;

            let payload = app_category_payload(&p, &cat, &now, Some(&now));
            enqueue(conn, OutboxOp::Upsert, OutboxEntity::AppCategory, &p, &payload)
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn list_unclassified(pool: &DbPool, days_back: u32) -> Result<Vec<UnclassifiedApp>> {
    let days = days_back.max(1) as i64;
    let rows = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT a.process_name,
                            CAST(SUM(a.duration_secs) / 60 AS INTEGER) AS minutes,
                            MAX(a.ended_at) AS last_seen_at
                     FROM activities a
                     LEFT JOIN app_categories m ON m.process_name = a.process_name AND m.deleted_at IS NULL
                     WHERE m.process_name IS NULL
                       AND a.local_date >= date('now','localtime', '-' || ?1 || ' days')
                       AND a.process_name <> 'Unknown'
                     GROUP BY a.process_name
                     ORDER BY minutes DESC",
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let it = stmt
                .query_map(rusqlite::params![days], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let mut out = Vec::new();
            for r in it {
                out.push(r.map_err(tokio_rusqlite::Error::Rusqlite)?);
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
