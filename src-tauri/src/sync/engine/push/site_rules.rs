use super::build_table_rows;
use crate::error::Result;
use crate::storage::DbPool;
use crate::sync::payload::SiteRulePayload;

pub(super) async fn build(pool: &DbPool) -> Result<Vec<u8>> {
    build_table_rows(
        pool,
        "SELECT host, browser, device, category_id, updated_at, deleted_at
         FROM site_rules ORDER BY host, browser, device",
        |r| {
            Ok(SiteRulePayload {
                host: r.get(0)?,
                browser: r.get(1)?,
                device: r.get(2)?,
                category_id: r.get(3)?,
                updated_at: r.get(4)?,
                deleted_at: r.get(5)?,
            })
        },
    )
    .await
}
