//! Category methods (`categories.*` and `super_categories.*`), and the category format that
//! plugins get.

use std::collections::HashMap;

use serde::Serialize;

use crate::error::Result;
use crate::repo::{categories, super_categories};
use crate::storage::DbPool;

/// A category as a plugin gets it: its id, name, color, icon and super-category.
/// `categories.list`, `stats.day_by_category` and the `focus.changed` event all use this format.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CategoryRef {
    pub(crate) id: String,
    pub(crate) name: String,
    /// Hex color `#rrggbb`, the same as in the main window
    pub(crate) color: String,
    /// Lucide icon name, the same as in the main window
    pub(crate) icon: String,
    /// `null` if the category is in no super-category
    pub(crate) super_category: Option<SuperCategoryRef>,
}

/// A super-category as a plugin gets it: its id, name, color and icon.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SuperCategoryRef {
    pub(crate) id: String,
    pub(crate) name: String,
    /// Hex color `#rrggbb`
    pub(crate) color: String,
    /// Lucide icon name
    pub(crate) icon: String,
}

impl CategoryRef {
    /// Every category, in the order of the main window. `categories.list` returns it as it is,
    /// and a plugin's settings page uses it to let the user choose.
    pub(super) async fn list(pool: &DbPool) -> Result<Vec<Self>> {
        let supers: HashMap<String, SuperCategoryRef> = SuperCategoryRef::list(pool)
            .await?
            .into_iter()
            .map(|s| (s.id.clone(), s))
            .collect();
        Ok(categories::list(pool)
            .await?
            .into_iter()
            .map(|c| Self {
                // Missing only if the super-category was deleted between the two queries
                super_category: c.super_category_id.and_then(|id| supers.get(&id).cloned()),
                id: c.id,
                name: c.name,
                color: c.color,
                icon: c.icon,
            })
            .collect())
    }
}

impl SuperCategoryRef {
    /// Every super-category, in the order of the main window. `super_categories.list` returns it
    /// as it is.
    pub(super) async fn list(pool: &DbPool) -> Result<Vec<Self>> {
        Ok(super_categories::list(pool)
            .await?
            .into_iter()
            .map(|s| Self {
                id: s.id,
                name: s.name,
                color: s.color,
                icon: s.icon,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::repo::test_util::fresh_test_pool;

    /// 在 `list` 的结果里按 id 找一项。
    fn find(list: &Value, id: &str) -> Value {
        list.as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("no {id}: {list}"))
    }

    /// 列出全部分类：带所在的大类，没归入大类的是 null；只有 id、名字、颜色、图标和大类，不带
    /// 应用。插件按这些键取值，JSON 整个锁在这里。
    #[tokio::test]
    async fn list_returns_every_category_with_its_super_category() {
        let pool = fresh_test_pool().await;
        super_categories::assign_category(&pool, "code", Some("work".into()))
            .await
            .unwrap();
        let job_hunting = categories::create(
            &pool,
            categories::CategoryInput {
                name: "求职".into(),
                color: "#000000".into(),
                icon: "Briefcase".into(),
            },
        )
        .await
        .unwrap();

        let got = serde_json::to_value(CategoryRef::list(&pool).await.unwrap()).unwrap();

        assert_eq!(
            find(&got, "code"),
            serde_json::json!({
                "id": "code",
                "name": "编程",
                "color": "#a78bfa",
                "icon": "Code",
                "superCategory": {
                    "id": "work",
                    "name": "工作",
                    "color": "#8b5cf6",
                    "icon": "Briefcase",
                },
            })
        );
        assert_eq!(
            find(&got, &job_hunting.id),
            serde_json::json!({
                "id": job_hunting.id,
                "name": "求职",
                "color": "#000000",
                "icon": "Briefcase",
                "superCategory": null,
            })
        );
    }

    /// 列出全部大类，里面没有分类的大类也列出来；只有 id、名字、颜色和图标。
    #[tokio::test]
    async fn list_super_returns_every_super_category() {
        let pool = fresh_test_pool().await;
        let empty = super_categories::create(
            &pool,
            super_categories::SuperCategoryInput {
                name: "副业".into(),
                color: "#000000".into(),
                icon: "Star".into(),
            },
        )
        .await
        .unwrap();

        let got = serde_json::to_value(SuperCategoryRef::list(&pool).await.unwrap()).unwrap();

        assert_eq!(
            find(&got, "work"),
            serde_json::json!({
                "id": "work",
                "name": "工作",
                "color": "#8b5cf6",
                "icon": "Briefcase",
            })
        );
        assert_eq!(
            find(&got, &empty.id),
            serde_json::json!({
                "id": empty.id,
                "name": "副业",
                "color": "#000000",
                "icon": "Star",
            })
        );
    }
}
