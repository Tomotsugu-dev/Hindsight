use super::build_table_rows;
use crate::error::Result;
use crate::storage::DbPool;
use crate::sync::payload::AppIconPayload;

pub(super) async fn build(pool: &DbPool) -> Result<Vec<u8>> {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
    build_table_rows(
        pool,
        "SELECT process_name, icon_png, updated_at, deleted_at
         FROM app_icons ORDER BY process_name",
        |r| {
            let bytes: Vec<u8> = r.get(1)?;
            Ok(AppIconPayload {
                process_name: r.get(0)?,
                // BLOB → base64：JSON 不支持 binary，统一用 base64 标准编码
                icon_png_base64: BASE64.encode(&bytes),
                updated_at: r.get(2)?,
                deleted_at: r.get(3)?,
            })
        },
    )
    .await
}
