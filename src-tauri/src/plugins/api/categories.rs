//! Category methods (`categories.*` and `super_categories.*`), and the category format that
//! plugins get.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;

use crate::error::Result;
use crate::repo::{categories, super_categories};
use crate::storage::DbPool;

/// A category as a plugin gets it: its id, name and super-category. `categories.list` and the
/// `focus.changed` event both use this format.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CategoryRef {
    pub(crate) id: String,
    pub(crate) name: String,
    /// `null` if the category is in no super-category
    pub(crate) super_category: Option<SuperCategoryRef>,
}

/// A super-category as a plugin gets it: its id and name.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SuperCategoryRef {
    pub(crate) id: String,
    pub(crate) name: String,
}

/// `categories.list`: every category with its super-category, in the order of the main window. A
/// plugin's settings page uses it to let the user choose.
pub(super) async fn list(pool: &DbPool) -> Result<Value> {
    let names: HashMap<String, String> = super_categories::list(pool)
        .await?
        .into_iter()
        .map(|s| (s.id, s.name))
        .collect();
    let categories: Vec<CategoryRef> = categories::list(pool)
        .await?
        .into_iter()
        .map(|c| CategoryRef {
            // Missing only if the super-category was deleted between the two queries
            super_category: c.super_category_id.and_then(|id| {
                let name = names.get(&id)?.clone();
                Some(SuperCategoryRef { id, name })
            }),
            id: c.id,
            name: c.name,
        })
        .collect();
    Ok(serde_json::to_value(categories)?)
}

/// `super_categories.list`: every super-category, in the order of the main window.
pub(super) async fn list_super(pool: &DbPool) -> Result<Value> {
    let supers: Vec<SuperCategoryRef> = super_categories::list(pool)
        .await?
        .into_iter()
        .map(|s| SuperCategoryRef {
            id: s.id,
            name: s.name,
        })
        .collect();
    Ok(serde_json::to_value(supers)?)
}

#[cfg(test)]
mod tests {
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

    /// 列出全部分类：带所在的大类，没归入大类的是 null；只有 id、名字和大类，不带应用。
    /// 插件按这些键取值，JSON 整个锁在这里。
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

        let got = list(&pool).await.unwrap();

        assert_eq!(
            find(&got, "code"),
            serde_json::json!({
                "id": "code",
                "name": "编程",
                "superCategory": { "id": "work", "name": "工作" },
            })
        );
        assert_eq!(
            find(&got, &job_hunting.id),
            serde_json::json!({ "id": job_hunting.id, "name": "求职", "superCategory": null })
        );
    }

    /// 列出全部大类，里面没有分类的大类也列出来；只有 id 和名字。
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

        let got = list_super(&pool).await.unwrap();

        assert_eq!(
            find(&got, "work"),
            serde_json::json!({ "id": "work", "name": "工作" })
        );
        assert_eq!(
            find(&got, &empty.id),
            serde_json::json!({ "id": empty.id, "name": "副业" })
        );
    }
}
