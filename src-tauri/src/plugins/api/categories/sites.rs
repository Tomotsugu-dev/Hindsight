//! Website category rules (`categories.site.*`).

use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::capture::browser_url::host_for_stats;
use crate::error::{Error, Result};
use crate::repo::categories::PRIVATE_CATEGORY_ID;
use crate::repo::site_rules;
use crate::storage::DbPool;

/// A website and the rule it uses, as a plugin gets it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::plugins::api) struct SiteCategory {
    host: String,
    /// `null` when no rule applies
    category_id: Option<String>,
    /// The parent domain the rule comes from; `null` when the website uses its own rule or none
    inherited_from: Option<String>,
}

/// `categories.site.list`: lists every website and the rule it uses, most used in the last
/// 30 days first. Leaves out websites in Private.
pub(in crate::plugins::api) async fn list(pool: &DbPool) -> Result<Vec<SiteCategory>> {
    Ok(site_rules::list(pool, Local::now().date_naive())
        .await?
        .into_iter()
        .filter(|r| r.category_id.as_deref() != Some(PRIVATE_CATEGORY_ID))
        .map(|r| SiteCategory {
            host: r.host,
            category_id: r.category_id,
            inherited_from: r.follows,
        })
        .collect())
}

/// `categories.site.write`: sets the rule of a website, or removes its own rule when
/// `categoryId` is `null`.
///
/// # Errors
///
/// - The parameters are wrong, `host` is not written as in the list, or the website is Private
///   before or after the change: [`Error::InvalidInput`]
/// - The category does not exist or was deleted: [`Error::InvalidInput`]
pub(in crate::plugins::api) async fn write(pool: &DbPool, params: Value) -> Result<Value> {
    let Write { host, category_id } = serde_json::from_value(params).map_err(|_| {
        Error::InvalidInput(
            "categories.site.write needs { host, categoryId }: host a string, categoryId a string or null",
        )
    })?;
    // Recorded domains come from this function, so a rule written any other way never matches
    if host_for_stats(&format!("https://{host}")).as_deref() != Some(host.as_str()) {
        return Err(Error::InvalidInput(
            "host must be written as in categories.site.list: lowercase, without www., ://, / or :",
        ));
    }
    if category_id.as_deref() == Some(PRIVATE_CATEGORY_ID) {
        return Err(Error::InvalidInput(
            "plugins cannot use the private category",
        ));
    }
    let private_now = is_private(pool, &host).await?;
    // Without its own rule, the website uses the rule of its parent domain
    let private_after = match (&category_id, host.split_once('.')) {
        (None, Some((_, parent))) => is_private(pool, parent).await?,
        _ => false,
    };
    if private_now || private_after {
        return Err(Error::InvalidInput(
            "plugins cannot change private websites",
        ));
    }
    match category_id {
        Some(id) => site_rules::set(pool, &host, &id).await?,
        None => site_rules::remove(pool, &host).await?,
    }
    Ok(Value::Null)
}

/// Checks whether the rule the website uses now is Private.
async fn is_private(pool: &DbPool, host: &str) -> Result<bool> {
    let category = site_rules::category_of(pool, host).await?;
    Ok(category.as_deref() == Some(PRIVATE_CATEGORY_ID))
}

/// The parameters of `categories.site.write`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Write {
    host: String,
    /// Required: a missing field is not taken as `null`, which would remove the rule
    #[serde(deserialize_with = "Option::deserialize")]
    category_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::repo::reports::test_seed::insert_visit;
    use crate::repo::test_util::fresh_test_pool;

    /// 今天在浏览器里看了 `secs` 秒。
    async fn visit(pool: &DbPool, host: &str, secs: i64) {
        let today = Local::now().format("%Y-%m-%d").to_string();
        insert_visit(pool, &today, "Google Chrome", Some(host), secs).await;
    }

    /// 网站现在用的规则的分类。
    async fn rule(pool: &DbPool, host: &str) -> Option<String> {
        site_rules::category_of(pool, host).await.unwrap()
    }

    /// 列出每个网站用的规则和继承来源，不带时长；隐私的网站不列，继承了隐私的子域名也不列。
    /// 插件按这些键取值，JSON 整个锁在这里。
    #[tokio::test]
    async fn list_gives_rules_without_time_and_leaves_out_private() {
        let pool = fresh_test_pool().await;
        visit(&pool, "docs.github.com", 600).await;
        visit(&pool, "sub.secret.com", 900).await;
        visit(&pool, "example.org", 60).await;
        site_rules::set(&pool, "github.com", "code").await.unwrap();
        site_rules::set(&pool, "youtube.com", "hidden")
            .await
            .unwrap();
        site_rules::set(&pool, "secret.com", PRIVATE_CATEGORY_ID)
            .await
            .unwrap();

        let got = serde_json::to_value(list(&pool).await.unwrap()).unwrap();

        assert_eq!(
            got,
            json!([
                { "host": "docs.github.com", "categoryId": "code", "inheritedFrom": "github.com" },
                { "host": "example.org", "categoryId": null, "inheritedFrom": null },
                { "host": "github.com", "categoryId": "code", "inheritedFrom": null },
                { "host": "youtube.com", "categoryId": "hidden", "inheritedFrom": null },
            ])
        );
    }

    /// 设规则、改成不计入统计；传 null 删掉自己的规则，之后跟父域名的规则走。
    #[tokio::test]
    async fn write_sets_and_removes_rules() {
        let pool = fresh_test_pool().await;
        site_rules::set(&pool, "github.com", "code").await.unwrap();

        write(
            &pool,
            json!({ "host": "docs.github.com", "categoryId": "browse" }),
        )
        .await
        .unwrap();
        assert_eq!(
            rule(&pool, "docs.github.com").await.as_deref(),
            Some("browse")
        );
        write(
            &pool,
            json!({ "host": "docs.github.com", "categoryId": "hidden" }),
        )
        .await
        .unwrap();
        assert_eq!(
            rule(&pool, "docs.github.com").await.as_deref(),
            Some("hidden")
        );
        write(
            &pool,
            json!({ "host": "docs.github.com", "categoryId": null }),
        )
        .await
        .unwrap();
        assert_eq!(
            rule(&pool, "docs.github.com").await.as_deref(),
            Some("code")
        );
    }

    /// 参数不对、域名不是列表里的写法、分类不存在，都报错，一条规则都不存。
    #[tokio::test]
    async fn write_rejects_bad_params_and_hosts() {
        let pool = fresh_test_pool().await;

        let mut calls = vec![
            json!({ "host": "github.com" }),
            json!({ "host": "github.com", "categoryId": 1 }),
            json!({ "host": "github.com", "categoryId": "no-such-category" }),
        ];
        for host in [
            "GitHub.com",
            "www.github.com",
            "https://github.com",
            "github.com/x",
            "github.com:8080",
            "github.com.",
            "",
        ] {
            calls.push(json!({ "host": host, "categoryId": "code" }));
        }
        for params in calls {
            let err = write(&pool, params.clone()).await.unwrap_err();
            assert!(matches!(err, Error::InvalidInput(_)), "{params}: {err:?}");
        }

        assert!(list(&pool).await.unwrap().is_empty());
    }

    /// 改之前或改之后是隐私都报错，规则不变：隐私的网站本身、隐私父域名下的子域名、
    /// 删掉规则后会退回隐私父域名、要设成隐私。
    #[tokio::test]
    async fn write_never_touches_private() {
        let pool = fresh_test_pool().await;
        site_rules::set(&pool, "secret.com", PRIVATE_CATEGORY_ID)
            .await
            .unwrap();
        site_rules::set(&pool, "open.secret.com", "code")
            .await
            .unwrap();

        for (host, category_id) in [
            ("secret.com", json!("code")),
            ("secret.com", json!(null)),
            ("sub.secret.com", json!("code")),
            ("open.secret.com", json!(null)),
            ("github.com", json!(PRIVATE_CATEGORY_ID)),
        ] {
            let params = json!({ "host": host, "categoryId": category_id });
            let err = write(&pool, params.clone()).await.unwrap_err();
            assert!(matches!(err, Error::InvalidInput(_)), "{params}: {err:?}");
        }

        assert_eq!(
            rule(&pool, "secret.com").await.as_deref(),
            Some(PRIVATE_CATEGORY_ID)
        );
        assert_eq!(
            rule(&pool, "open.secret.com").await.as_deref(),
            Some("code")
        );
        assert_eq!(
            rule(&pool, "sub.secret.com").await.as_deref(),
            Some(PRIVATE_CATEGORY_ID)
        );
        assert_eq!(rule(&pool, "github.com").await, None);
    }
}
