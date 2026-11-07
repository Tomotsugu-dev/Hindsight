use crate::error::{Error, Result};
use crate::storage::{DbPool, SqliteResultExt};
use crate::sync::payload::SiteRulePayload;

pub(super) async fn merge(pool: &DbPool, body: &[u8]) -> Result<()> {
    // Apply the whole snapshot atomically so failed files can be retried.
    let rows: Vec<SiteRulePayload> =
        serde_json::from_slice(body).map_err(|source| Error::SyncParse {
            kind: "site_rules",
            source,
        })?;
    pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            for row in rows {
                tx.execute(
                    "INSERT INTO site_rules(host, browser, device, category_id, updated_at, deleted_at)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(host, browser, device) DO UPDATE SET
                         category_id = excluded.category_id,
                         updated_at = excluded.updated_at,
                         deleted_at = excluded.deleted_at
                     WHERE excluded.updated_at > site_rules.updated_at",
                    rusqlite::params![
                        row.host, row.browser, row.device, row.category_id,
                        row.updated_at, row.deleted_at,
                    ],
                )
                .db()?;
            }
            tx.commit().db()
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{outbox_entries, read_row, s, T_MID, T_NEW, T_OLD};
    use super::*;
    use crate::repo::test_util::fresh_test_pool;

    #[tokio::test]
    async fn merge_site_rules_uses_the_full_key_and_keeps_newer_changes() {
        let pool = fresh_test_pool().await;
        let body = |category: &str, updated: &str, deleted: Option<&str>| {
            serde_json::to_vec(&serde_json::json!([{
                "host": "example.com", "categoryId": category,
                "updatedAt": updated, "deletedAt": deleted,
            }]))
            .unwrap()
        };
        merge(&pool, &body("code", T_MID, None)).await.unwrap();
        for updated in [T_OLD, T_MID] {
            merge(&pool, &body("video", updated, Some(updated)))
                .await
                .unwrap();
            assert_eq!(
                read_row(&pool, "SELECT category_id, updated_at, deleted_at FROM site_rules WHERE host = ?1 AND browser = '' AND device = ''", "example.com", 3).await.unwrap(),
                vec![s("code"), s(T_MID), None],
            );
        }
        merge(&pool, &body("video", T_NEW, Some(T_NEW)))
            .await
            .unwrap();
        merge(&pool, &body("code", T_MID, None)).await.unwrap();
        assert_eq!(
            read_row(&pool, "SELECT category_id, updated_at, deleted_at FROM site_rules WHERE host = ?1 AND browser = '' AND device = ''", "example.com", 3).await.unwrap(),
            vec![s("video"), s(T_NEW), s(T_NEW)],
        );

        let scoped = serde_json::to_vec(&serde_json::json!([
            {"host": "example.com", "browser": "Chrome", "device": "", "categoryId": "not-here-yet", "updatedAt": T_NEW},
            {"host": "example.com", "browser": "", "device": "device-a", "categoryId": "code", "updatedAt": T_NEW},
        ])).unwrap();
        merge(&pool, &scoped).await.unwrap();
        let scopes: Vec<(String, String, String)> = pool.0.call(|conn| {
            let mut stmt = conn.prepare("SELECT browser, device, category_id FROM site_rules ORDER BY browser, device").db()?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).db()?.collect::<rusqlite::Result<Vec<_>>>().db()?;
            Ok(rows)
        }).await.unwrap();
        assert_eq!(
            scopes,
            vec![
                ("".into(), "".into(), "video".into()),
                ("".into(), "device-a".into(), "code".into()),
                ("Chrome".into(), "".into(), "not-here-yet".into()),
            ]
        );
        assert!(outbox_entries(&pool).await.is_empty());
    }
}
