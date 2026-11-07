use crate::error::Result;
use crate::storage::DbPool;
use crate::sync::payload::DeviceMetaPayload;

pub(super) async fn build(pool: &DbPool, self_id: &str) -> Result<Vec<u8>> {
    let self_id = self_id.to_string();
    let obj: Option<DeviceMetaPayload> = pool
        .0
        .call(move |conn| {
            let row = conn
                .query_row(
                    "SELECT device_id, display_name, color, icon, os, last_seen_at, updated_at
                     FROM devices WHERE device_id = ?1",
                    rusqlite::params![self_id],
                    |r| {
                        Ok(DeviceMetaPayload {
                            device_id: r.get(0)?,
                            display_name: r.get(1)?,
                            color: r.get(2)?,
                            icon: r.get(3)?,
                            os: r.get(4)?,
                            last_seen_at: r.get(5)?,
                            updated_at: r.get(6)?,
                        })
                    },
                )
                .ok();
            Ok(row)
        })
        .await?;
    let Some(meta) = obj else {
        return Ok(b"{}".to_vec());
    };
    Ok(serde_json::to_vec(&meta)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};
    use crate::storage::SqliteResultExt;

    /// 任务 6:[`build`] 导出 devices 行的全部字段(尤其 os ——
    /// pull 侧的跨 OS 过滤完全依赖它),并用 pull 同款 DTO
    /// [`DeviceMetaPayload`] 反序列化,钉死字段名往返一致(camelCase 契约)。
    #[tokio::test]
    async fn build_device_meta_exports_all_fields_roundtrip() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|conn| {
                conn.execute(
                    "INSERT INTO devices(device_id, display_name, color, icon, os,
                                         last_seen_at, is_self, updated_at, deleted_at)
                     VALUES(?1, '我的 Mac', '#a1b2c3', 'Laptop', 'macos',
                            '2026-07-01T08:00:00Z', 1, '2026-07-02T09:00:00Z', NULL)",
                    rusqlite::params![crate::repo::test_util::TEST_SELF_ID],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();

        let body = build(&pool, TEST_SELF_ID).await.unwrap();
        let meta: DeviceMetaPayload = serde_json::from_slice(&body)
            .expect("meta.json 应能被 pull 同款 DeviceMetaPayload 解析");

        assert_eq!(meta.device_id, TEST_SELF_ID);
        assert_eq!(meta.display_name, "我的 Mac");
        assert_eq!(meta.color, "#a1b2c3");
        assert_eq!(meta.icon, "Laptop");
        assert_eq!(meta.os.as_deref(), Some("macos"), "os 随 meta 一起导出");
        assert_eq!(meta.last_seen_at.as_deref(), Some("2026-07-01T08:00:00Z"));
        assert_eq!(meta.updated_at, "2026-07-02T09:00:00Z");
    }

    /// devices 无本机行(device::ensure_loaded 尚未跑)时导出空对象 `{}` ——
    /// pull 侧 merge_device_meta 对空对象是跳过语义,不会误插一行空 meta。
    #[tokio::test]
    async fn build_device_meta_returns_empty_object_without_row() {
        let pool = fresh_test_pool().await;
        let body = build(&pool, TEST_SELF_ID).await.unwrap();
        assert_eq!(body, b"{}".to_vec(), "无 devices 行应返回空 JSON 对象");
    }
}
