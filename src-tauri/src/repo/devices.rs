use serde::Serialize;

use crate::error::Result;
use crate::repo::outbox::{enqueue, OutboxEntity, OutboxOp};
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};

/// 设备表的一行（前端「设备」页面渲染用）。包含本机和同步看到的远端设备。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRow {
    /// UUID（device.json 里写死的 self id 或同步过来的远端 id）
    pub device_id: String,
    pub display_name: String,
    /// hex `#rrggbb`，UI 头像底色
    pub color: String,
    /// 图标 ID（前端 lucide-react 映射）
    pub icon: String,
    /// 操作系统标识（"win" / "mac" / "linux"），跨设备同步过来的可空
    pub os: Option<String>,
    /// 最后一次看到该设备活动（活动行的 max(updated_at)）；从未活动 None
    pub last_seen_at: Option<String>,
    /// 是否当前机器
    pub is_self: bool,
}

/// Registers this machine's row at startup and queues it for the other devices.
///
/// When the row already exists, only last_seen_at is refreshed. display_name /
/// color / icon are what the user set on the devices page, so a restart must not
/// overwrite them; os is written once, when the row is created.
pub async fn upsert_self(
    pool: &DbPool,
    device_id: String,
    default_name: String,
    default_color: String,
    default_icon: String,
    os: String,
) -> Result<()> {
    let now = utc_now_rfc3339();
    pool.0
        .call(move |conn| {
            conn.execute("UPDATE devices SET is_self = 0", [])
                .db()?;

            conn.execute(
                "INSERT INTO devices (device_id, display_name, color, icon, os, last_seen_at, is_self, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?6)
                 ON CONFLICT(device_id) DO UPDATE SET
                   last_seen_at = excluded.last_seen_at,
                   is_self = 1,
                   updated_at = excluded.updated_at",
                rusqlite::params![device_id, default_name, default_color, default_icon, os, now],
            )
            .db()?;

            // 写 outbox（self 设备的元信息要同步给其他机器看）
            let payload = serde_json::json!({
                "deviceId": device_id,
                "displayName": default_name,
                "color": default_color,
                "icon": default_icon,
                "os": os,
                "lastSeenAt": now,
                "updatedAt": now,
            })
            .to_string();
            enqueue(conn, OutboxOp::Upsert, OutboxEntity::Device, &device_id, &payload)
                .db()?;

            Ok(())
        })
        .await?;
    Ok(())
}

/// Lists all devices that have not been soft-deleted: the current device stays pinned first,
/// and the remaining devices are sorted by last_seen_at in descending order.
/// Devices with no activity history have last_seen_at = NULL and are placed at the end.
pub async fn list_all(pool: &DbPool) -> Result<Vec<DeviceRow>> {
    let rows = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT device_id, display_name, color, icon, os, last_seen_at, is_self
                     FROM devices
                     WHERE deleted_at IS NULL
                     ORDER BY is_self DESC, last_seen_at DESC NULLS LAST",
                )
                .db()?;

            let rows = stmt
                .query_map([], |r| {
                    Ok(DeviceRow {
                        device_id: r.get(0)?,
                        display_name: r.get(1)?,
                        color: r.get(2)?,
                        icon: r.get(3)?,
                        os: r.get(4)?,
                        last_seen_at: r.get(5)?,
                        is_self: r.get::<_, i64>(6)? != 0,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await?;
    Ok(rows)
}

/// 用户改 self 设备的显示信息。写本地表 + outbox，同事务。
pub async fn update_self_meta(
    pool: &DbPool,
    device_id: String,
    name: Option<String>,
    color: Option<String>,
    icon: Option<String>,
) -> Result<DeviceRow> {
    let now = utc_now_rfc3339();
    let row = pool
        .0
        .call(move |conn| {
            // 找到 self
            let current: (String, String, String, Option<String>, Option<String>) = conn
                .query_row(
                    "SELECT display_name, color, icon, os, last_seen_at FROM devices WHERE device_id = ?1",
                    rusqlite::params![device_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .db()?;

            let next_name = name.unwrap_or(current.0);
            let next_color = color.unwrap_or(current.1);
            let next_icon = icon.unwrap_or(current.2);

            conn.execute(
                "UPDATE devices SET display_name = ?2, color = ?3, icon = ?4, updated_at = ?5
                 WHERE device_id = ?1",
                rusqlite::params![device_id, next_name, next_color, next_icon, now],
            )
            .db()?;

            let payload = serde_json::json!({
                "deviceId": device_id,
                "displayName": next_name,
                "color": next_color,
                "icon": next_icon,
                "os": current.3,
                "lastSeenAt": current.4,
                "updatedAt": now,
            })
            .to_string();
            enqueue(conn, OutboxOp::Upsert, OutboxEntity::Device, &device_id, &payload)
                .db()?;

            Ok(DeviceRow {
                device_id,
                display_name: next_name,
                color: next_color,
                icon: next_icon,
                os: current.3,
                last_seen_at: current.4,
                is_self: true,
            })
        })
        .await?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::fresh_test_pool;

    #[tokio::test]
    async fn upsert_self_preserves_user_edits_and_keeps_single_self() {
        let pool = fresh_test_pool().await;
        upsert_self(
            &pool,
            "dev-1".into(),
            "默认名".into(),
            "#111111".into(),
            "Monitor".into(),
            "mac".into(),
        )
        .await
        .unwrap();

        // 用户改名后再次启动 upsert:display_name 不被默认值覆盖,os 也不动,只刷新 last_seen
        update_self_meta(&pool, "dev-1".into(), Some("我的电脑".into()), None, None)
            .await
            .unwrap();
        upsert_self(
            &pool,
            "dev-1".into(),
            "默认名v2".into(),
            "#222222".into(),
            "Laptop".into(),
            "win".into(),
        )
        .await
        .unwrap();

        let rows = list_all(&pool).await.unwrap();
        let me = rows.iter().find(|r| r.device_id == "dev-1").unwrap();
        assert_eq!(me.display_name, "我的电脑");
        assert_eq!(me.os.as_deref(), Some("mac"));
        assert!(me.is_self);

        // 换机器 id 再 upsert:self 标志唯一
        upsert_self(
            &pool,
            "dev-2".into(),
            "第二台".into(),
            "#333333".into(),
            "Monitor".into(),
            "mac".into(),
        )
        .await
        .unwrap();
        let rows = list_all(&pool).await.unwrap();
        assert_eq!(rows.iter().filter(|r| r.is_self).count(), 1);
        assert!(
            rows.iter()
                .find(|r| r.device_id == "dev-2")
                .unwrap()
                .is_self
        );
        assert!(
            !rows
                .iter()
                .find(|r| r.device_id == "dev-1")
                .unwrap()
                .is_self
        );
    }
}
