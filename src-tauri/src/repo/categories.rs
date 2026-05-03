use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
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

pub async fn list(pool: &DbPool) -> Result<Vec<Category>> {
    let cats = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT id, name, color, icon, builtin FROM categories ORDER BY builtin DESC, id",
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
                    "SELECT process_name, category_id FROM app_categories ORDER BY process_name",
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

    pool.0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO categories(id, name, color, icon, builtin) VALUES(?, ?, ?, ?, 0)",
                rusqlite::params![id_clone, n, c, i],
            )
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
    pool.0
        .call(move |conn| {
            if let Some(name) = patch.name.as_ref() {
                let trimmed = name.trim();
                if !trimmed.is_empty() {
                    conn.execute(
                        "UPDATE categories SET name = ? WHERE id = ?",
                        rusqlite::params![trimmed, id],
                    )
                    .map_err(tokio_rusqlite::Error::Rusqlite)?;
                }
            }
            if let Some(color) = patch.color.as_ref() {
                let trimmed = color.trim();
                if !trimmed.is_empty() {
                    conn.execute(
                        "UPDATE categories SET color = ? WHERE id = ?",
                        rusqlite::params![trimmed, id],
                    )
                    .map_err(tokio_rusqlite::Error::Rusqlite)?;
                }
            }
            if let Some(icon) = patch.icon.as_ref() {
                let trimmed = icon.trim();
                if !trimmed.is_empty() {
                    conn.execute(
                        "UPDATE categories SET icon = ? WHERE id = ?",
                        rusqlite::params![trimmed, id],
                    )
                    .map_err(tokio_rusqlite::Error::Rusqlite)?;
                }
            }
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn delete(pool: &DbPool, id: &str) -> Result<()> {
    let id = id.to_string();
    pool.0
        .call(move |conn| {
            let builtin: Option<i64> = conn
                .query_row(
                    "SELECT builtin FROM categories WHERE id = ?",
                    [&id],
                    |r| r.get(0),
                )
                .ok();
            match builtin {
                None => return Ok(()),
                Some(b) if b != 0 => {
                    return Err(tokio_rusqlite::Error::Other(
                        "内置分类不可删除".into(),
                    ));
                }
                _ => {}
            }

            let tx = conn
                .transaction()
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            tx.execute(
                "UPDATE app_categories SET category_id = 'other' WHERE category_id = ?",
                rusqlite::params![id],
            )
            .map_err(tokio_rusqlite::Error::Rusqlite)?;
            tx.execute(
                "DELETE FROM categories WHERE id = ?",
                rusqlite::params![id],
            )
            .map_err(tokio_rusqlite::Error::Rusqlite)?;
            tx.commit().map_err(tokio_rusqlite::Error::Rusqlite)?;
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
    pool.0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO app_categories(process_name, category_id) VALUES(?, ?)
                 ON CONFLICT(process_name) DO UPDATE SET category_id = excluded.category_id",
                rusqlite::params![p, c],
            )
            .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn unassign_app(pool: &DbPool, process_name: &str) -> Result<()> {
    let p = process_name.to_string();
    pool.0
        .call(move |conn| {
            conn.execute(
                "DELETE FROM app_categories WHERE process_name = ?",
                rusqlite::params![p],
            )
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
                     LEFT JOIN app_categories m ON m.process_name = a.process_name
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
