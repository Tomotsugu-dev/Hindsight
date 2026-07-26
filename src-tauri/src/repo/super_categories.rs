//! 「大类」(super-category) 容器层 repo。把多个 categories 视觉/语义打包成"工作 / 娱乐"
//! 这样的顶层组。
//!
//! 跟 [`categories`](super::categories) 的关键区别：
//! - 大类**不参与时长统计 JOIN**——统计仍按 categories 聚合，大类只是 UI 容器
//! - 大类**目前不上 outbox 同步**（v28 schema 留好了 updated_at/deleted_at，sync 接入是 TODO）
//! - 删大类 = 软删大类自己 + 把子分类的 super_category_id 置 NULL（fall back 到"未归入"）
//!
//! 所有写都是单设备本地写。多设备场景下需要等 sync 集成完毕后才能 LWW 收敛。

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuperCategory {
    /// UUID
    pub id: String,
    pub name: String,
    /// hex `#rrggbb`
    pub color: String,
    /// lucide icon name 或 emoji（前端 resolveCategoryIcon 解析）
    pub icon: String,
    /// 显示顺序——拖拽排序时更新；UI 升序排
    pub sort_order: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuperCategoryInput {
    pub name: String,
    pub color: String,
    pub icon: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SuperCategoryPatch {
    pub name: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
}

/// 列所有 active 大类，按 sort_order 升序、id 作为 tiebreaker。
pub async fn list(pool: &DbPool) -> Result<Vec<SuperCategory>> {
    let rows = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT id, name, color, icon, sort_order FROM super_categories
                     WHERE deleted_at IS NULL
                     ORDER BY sort_order ASC, id ASC",
                )
                .db()?;
            let it = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
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
        .map(|(id, name, color, icon, sort_order)| SuperCategory {
            id,
            name,
            color,
            icon,
            sort_order,
        })
        .collect())
}

/// 新建大类：UUID + sort_order 排末尾。返回完整对象给前端 optimistic 渲染。
pub async fn create(pool: &DbPool, input: SuperCategoryInput) -> Result<SuperCategory> {
    let id = uuid::Uuid::new_v4().to_string();
    let id_clone = id.clone();
    let name = input.name.trim().to_string();
    let color = input.color.trim().to_string();
    let icon = input.icon.trim().to_string();
    if name.is_empty() {
        return Err(Error::InvalidInput("大类名不能为空"));
    }
    if color.is_empty() {
        return Err(Error::InvalidInput("颜色不能为空"));
    }
    let final_icon = if icon.is_empty() {
        "Folder".to_string()
    } else {
        icon
    };
    let n = name.clone();
    let c = color.clone();
    let i = final_icon.clone();
    let updated = utc_now_rfc3339();

    let sort_order = pool
        .0
        .call(move |conn| {
            let next_sort: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM super_categories WHERE deleted_at IS NULL",
                    [],
                    |r| r.get(0),
                )
                .db()?;
            conn.execute(
                "INSERT INTO super_categories(id, name, color, icon, sort_order, updated_at)
                 VALUES(?, ?, ?, ?, ?, ?)",
                rusqlite::params![id_clone, n, c, i, next_sort, updated],
            )
            .db()?;
            Ok(next_sort)
        })
        .await?;

    Ok(SuperCategory {
        id,
        name,
        color,
        icon: final_icon,
        sort_order,
    })
}

/// 改名 / 换色 / 换 icon。patch 中 None 或 trim 后空字符串的字段保持不变。
pub async fn update(pool: &DbPool, id: &str, patch: SuperCategoryPatch) -> Result<()> {
    let id = id.to_string();
    let updated = utc_now_rfc3339();
    pool.0
        .call(move |conn| {
            let row: Option<(String, String, String)> = conn
                .query_row(
                    "SELECT name, color, icon FROM super_categories
                     WHERE id = ? AND deleted_at IS NULL",
                    rusqlite::params![id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .ok();
            let Some((cur_name, cur_color, cur_icon)) = row else {
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
                "UPDATE super_categories SET name = ?, color = ?, icon = ?, updated_at = ?
                 WHERE id = ?",
                rusqlite::params![next_name, next_color, next_icon, updated, id],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// 批量改 sort_order。前端拖完整列序列发回来；按数组顺序写 0,1,2,...
pub async fn reorder(pool: &DbPool, ordered_ids: Vec<String>) -> Result<()> {
    let updated = utc_now_rfc3339();
    pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            for (idx, id) in ordered_ids.iter().enumerate() {
                tx.execute(
                    "UPDATE super_categories SET sort_order = ?, updated_at = ?
                     WHERE id = ? AND deleted_at IS NULL",
                    rusqlite::params![idx as i64, updated, id],
                )
                .db()?;
            }
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// 软删大类。同时把所有引用该大类的 categories 的 super_category_id 置 NULL ——
/// 这些 categories 回到"未归入"状态而不是被 cascade 删掉。
pub async fn delete(pool: &DbPool, id: &str) -> Result<()> {
    let id = id.to_string();
    let updated = utc_now_rfc3339();
    pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            // 1. 子分类 super_category_id 置 NULL，回到 orphan
            tx.execute(
                "UPDATE categories SET super_category_id = NULL, updated_at = ?
                 WHERE super_category_id = ?",
                rusqlite::params![updated, id],
            )
            .db()?;
            // 2. 软删大类自己
            tx.execute(
                "UPDATE super_categories SET deleted_at = ?, updated_at = ?
                 WHERE id = ?",
                rusqlite::params![updated, updated, id],
            )
            .db()?;
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// 把某个 category 归到某个大类（super_id = None 表示移出大类回到 orphan）。
/// 不校验 super_id 是否存在 / 是否已软删——前端正常路径不会传错；如真传错，下次
/// list_categories LEFT JOIN 会自然回退到 orphan 行为，不影响数据完整性。
pub async fn assign_category(
    pool: &DbPool,
    category_id: &str,
    super_id: Option<String>,
) -> Result<()> {
    let cat_id = category_id.to_string();
    let updated = utc_now_rfc3339();
    pool.0
        .call(move |conn| {
            conn.execute(
                "UPDATE categories SET super_category_id = ?, updated_at = ?
                 WHERE id = ? AND deleted_at IS NULL",
                rusqlite::params![super_id, updated, cat_id],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::fresh_test_pool;

    async fn super_of(pool: &DbPool, cat: &str) -> Option<String> {
        let cat = cat.to_string();
        pool.0
            .call(move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT super_category_id FROM categories WHERE id = ?",
                        [cat],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .unwrap())
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn create_appends_update_patches_partially() {
        let pool = fresh_test_pool().await;
        let seeded = list(&pool).await.unwrap().len(); // 迁移种子的默认大类
        let a = create(
            &pool,
            SuperCategoryInput {
                name: "甲".into(),
                color: "#111111".into(),
                icon: "Star".into(),
            },
        )
        .await
        .unwrap();
        let b = create(
            &pool,
            SuperCategoryInput {
                name: "乙".into(),
                color: "#222222".into(),
                icon: "Moon".into(),
            },
        )
        .await
        .unwrap();
        let ls = list(&pool).await.unwrap();
        assert_eq!(ls.len(), seeded + 2);
        // 新建追加在尾部,顺序保持创建序
        assert_eq!(ls[ls.len() - 2].id, a.id);
        assert_eq!(ls[ls.len() - 1].id, b.id);

        // patch 只改给了的字段
        update(
            &pool,
            &a.id,
            SuperCategoryPatch {
                name: Some("甲改".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let got = list(&pool)
            .await
            .unwrap()
            .into_iter()
            .find(|x| x.id == a.id)
            .unwrap();
        assert_eq!(got.name, "甲改");
        assert_eq!(got.color, "#111111"); // 未给的字段不动
        assert_eq!(got.icon, "Star");
    }

    #[tokio::test]
    async fn reorder_rewrites_sort_order_by_index() {
        let pool = fresh_test_pool().await;
        let before = list(&pool).await.unwrap();
        let reversed: Vec<String> = before.iter().rev().map(|x| x.id.clone()).collect();
        reorder(&pool, reversed.clone()).await.unwrap();
        let after: Vec<String> = list(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|x| x.id)
            .collect();
        assert_eq!(after, reversed);
    }

    #[tokio::test]
    async fn delete_detaches_children_and_hides_super() {
        let pool = fresh_test_pool().await;
        let sup = create(
            &pool,
            SuperCategoryInput {
                name: "临时".into(),
                color: "#333333".into(),
                icon: "Box".into(),
            },
        )
        .await
        .unwrap();
        // 迁移种子里的 code 分类归入新大类
        assign_category(&pool, "code", Some(sup.id.clone()))
            .await
            .unwrap();
        assert_eq!(
            super_of(&pool, "code").await.as_deref(),
            Some(sup.id.as_str())
        );

        delete(&pool, &sup.id).await.unwrap();
        // 大类软删后不再列出;子分类回到"未归入"而不是被连坐删除
        assert!(list(&pool).await.unwrap().iter().all(|x| x.id != sup.id));
        assert_eq!(super_of(&pool, "code").await, None);

        // 移出大类(assign None)也走得通
        assign_category(&pool, "code", None).await.unwrap();
        assert_eq!(super_of(&pool, "code").await, None);
    }
}
