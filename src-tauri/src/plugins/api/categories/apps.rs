//! App categories (`categories.app.*`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::repo::app_groups;
use crate::repo::categories::{self, PRIVATE_CATEGORY_ID};
use crate::storage::DbPool;

/// An app and its category, as a plugin gets it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::plugins::api) struct AppCategory {
    /// The same name as `app` in `focus.changed`
    app: String,
    /// `null` when the app has no category
    category_id: Option<String>,
}

/// `categories.app.list`: lists every app and its category, most used in the last 7 days
/// first. Leaves out apps in Private.
pub(in crate::plugins::api) async fn list(pool: &DbPool) -> Result<Vec<AppCategory>> {
    let mut apps: Vec<(i64, AppCategory)> = app_groups::list_groups(pool)
        .await?
        .into_iter()
        .filter(|g| g.category_id.as_deref() != Some(PRIVATE_CATEGORY_ID))
        .flat_map(|g| {
            let category_id = g.category_id;
            g.members.into_iter().map(move |m| {
                let app = AppCategory {
                    app: m.process_name,
                    category_id: category_id.clone(),
                };
                (m.recent_secs, app)
            })
        })
        .collect();
    apps.sort_by(|(a_secs, a), (b_secs, b)| b_secs.cmp(a_secs).then_with(|| a.app.cmp(&b.app)));
    Ok(apps.into_iter().map(|(_, app)| app).collect())
}

/// `categories.app.write`: sets the category of an app, or removes it when `categoryId` is
/// `null`.
///
/// # Errors
///
/// - The parameters are wrong, the app is not in `categories.app.list`, or the category is
///   Private: [`Error::InvalidInput`]
/// - The category does not exist or was deleted: [`Error::InvalidInput`]
pub(in crate::plugins::api) async fn write(pool: &DbPool, params: Value) -> Result<Value> {
    let Write { app, category_id } = serde_json::from_value(params).map_err(|_| {
        Error::InvalidInput(
            "categories.app.write needs { app, categoryId }: app a string, categoryId a string or null",
        )
    })?;
    if category_id.as_deref() == Some(PRIVATE_CATEGORY_ID) {
        return Err(Error::InvalidInput(
            "plugins cannot use the private category",
        ));
    }
    // Apps in Private are not in the list either, so they get the same error
    if app_groups::group_id_for(pool, &app).await?.is_none()
        || categories::is_private(pool, &app, None).await?
    {
        return Err(Error::InvalidInput("no such app"));
    }
    match category_id {
        Some(id) => categories::assign_app(pool, &app, &id).await?,
        None => categories::unassign_app(pool, &app).await?,
    }
    Ok(Value::Null)
}

/// The parameters of `categories.app.write`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Write {
    app: String,
    /// Required: a missing field is not taken as `null`, which would remove the category
    #[serde(deserialize_with = "Option::deserialize")]
    category_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use chrono::Local;
    use serde_json::json;

    use super::*;
    use crate::repo::reports::test_seed::insert_activity;
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};

    /// 今天用了 `secs` 秒，并且有应用组。
    async fn used(pool: &DbPool, app: &str, secs: i64) {
        let today = Local::now().format("%Y-%m-%d").to_string();
        insert_activity(pool, TEST_SELF_ID, &today, app, secs).await;
        app_groups::ensure_group(pool, app).await.unwrap();
    }

    /// 应用现在算进哪个分类。
    async fn category(pool: &DbPool, app: &str) -> String {
        categories::classify(pool, app, None)
            .await
            .unwrap()
            .unwrap()
            .id
    }

    /// 按最近 7 天的用时排；没分类的是 null；隐私的不列，不计入统计的照列。
    /// 插件按这些键取值，JSON 整个锁在这里。
    #[tokio::test]
    async fn list_orders_by_recent_use_and_leaves_out_private() {
        let pool = fresh_test_pool().await;
        used(&pool, "Idle Tool", 60).await;
        used(&pool, "Busy Tool", 600).await;
        used(&pool, "Excluded Tool", 300).await;
        used(&pool, "Secret Tool", 900).await;
        categories::assign_app(&pool, "Busy Tool", "code")
            .await
            .unwrap();
        categories::assign_app(&pool, "Excluded Tool", "hidden")
            .await
            .unwrap();
        categories::assign_app(&pool, "Secret Tool", PRIVATE_CATEGORY_ID)
            .await
            .unwrap();

        let got = serde_json::to_value(list(&pool).await.unwrap()).unwrap();

        assert_eq!(
            got,
            json!([
                { "app": "Busy Tool", "categoryId": "code" },
                { "app": "Excluded Tool", "categoryId": "hidden" },
                { "app": "Idle Tool", "categoryId": null },
            ])
        );
    }

    /// 设分类、改成不计入统计；传 null 去掉分类，之后算进「其他」。
    #[tokio::test]
    async fn write_sets_and_clears_the_category() {
        let pool = fresh_test_pool().await;
        used(&pool, "Plain Tool", 60).await;

        write(&pool, json!({ "app": "Plain Tool", "categoryId": "code" }))
            .await
            .unwrap();
        assert_eq!(category(&pool, "Plain Tool").await, "code");
        write(
            &pool,
            json!({ "app": "Plain Tool", "categoryId": "hidden" }),
        )
        .await
        .unwrap();
        assert_eq!(category(&pool, "Plain Tool").await, "hidden");
        write(&pool, json!({ "app": "Plain Tool", "categoryId": null }))
            .await
            .unwrap();
        assert_eq!(category(&pool, "Plain Tool").await, "other");
    }

    /// 参数不对、应用不存在、应用是隐私、要设成隐私、分类不存在，都报错，什么都不改；
    /// 不存在的应用也不会多出一个应用组。
    #[tokio::test]
    async fn write_rejects_bad_calls_and_changes_nothing() {
        let pool = fresh_test_pool().await;
        used(&pool, "Plain Tool", 60).await;
        used(&pool, "Secret Tool", 60).await;
        categories::assign_app(&pool, "Secret Tool", PRIVATE_CATEGORY_ID)
            .await
            .unwrap();

        for params in [
            json!({ "app": "Plain Tool" }),
            json!({ "app": "Plain Tool", "categoryId": 1 }),
            json!({ "app": "Typo Tool", "categoryId": "code" }),
            json!({ "app": "Secret Tool", "categoryId": "code" }),
            json!({ "app": "Secret Tool", "categoryId": null }),
            json!({ "app": "Plain Tool", "categoryId": PRIVATE_CATEGORY_ID }),
            json!({ "app": "Plain Tool", "categoryId": "no-such-category" }),
        ] {
            let err = write(&pool, params.clone()).await.unwrap_err();
            assert!(matches!(err, Error::InvalidInput(_)), "{params}: {err:?}");
        }

        assert_eq!(category(&pool, "Plain Tool").await, "other");
        assert_eq!(category(&pool, "Secret Tool").await, PRIVATE_CATEGORY_ID);
        assert_eq!(
            app_groups::group_id_for(&pool, "Typo Tool").await.unwrap(),
            None
        );
    }

    /// 隐私的应用和不存在的应用报的是同一句，插件分不出来。
    #[tokio::test]
    async fn private_app_looks_like_a_missing_one() {
        let pool = fresh_test_pool().await;
        used(&pool, "Secret Tool", 60).await;
        categories::assign_app(&pool, "Secret Tool", PRIVATE_CATEGORY_ID)
            .await
            .unwrap();

        let missing = write(&pool, json!({ "app": "Typo Tool", "categoryId": "code" }))
            .await
            .unwrap_err();
        let private = write(&pool, json!({ "app": "Secret Tool", "categoryId": "code" }))
            .await
            .unwrap_err();

        assert_eq!(missing.to_string(), private.to_string());
    }
}
