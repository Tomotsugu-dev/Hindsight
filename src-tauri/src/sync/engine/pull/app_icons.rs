use super::{is_remote_newer, parse_rows};
use crate::error::Result;
use crate::storage::{DbPool, SqliteResultExt};
use crate::sync::payload::AppIconPayload;

pub(super) async fn merge(pool: &DbPool, body: &[u8]) -> Result<()> {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
    let rows: Vec<AppIconPayload> = parse_rows("app_icons", body)?;
    for row in rows {
        if row.process_name.is_empty() {
            continue;
        }
        let process_name = row.process_name;
        let icon_bytes = match BASE64.decode(row.icon_png_base64.as_bytes()) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("app_icon {process_name}: base64 does not decode: {e}");
                continue;
            }
        };
        let updated_at = row.updated_at;
        let deleted_at = row.deleted_at;

        let process_name_db = process_name.clone();
        let icon_bytes_db = icon_bytes.clone();
        let updated_at_db = updated_at.clone();
        let deleted_at_db = deleted_at.clone();
        let applied: bool = match pool
            .0
            .call(move |conn| {
                if !is_remote_newer(
                    conn,
                    "SELECT updated_at FROM app_icons WHERE process_name = ?1",
                    rusqlite::params![process_name_db],
                    &updated_at_db,
                )? {
                    return Ok(false);
                }
                conn.execute(
                    "INSERT INTO app_icons(process_name, icon_png, updated_at, deleted_at)
                     VALUES(?, ?, ?, ?)
                     ON CONFLICT(process_name) DO UPDATE SET
                       icon_png   = excluded.icon_png,
                       updated_at = excluded.updated_at,
                       deleted_at = excluded.deleted_at",
                    rusqlite::params![process_name_db, icon_bytes_db, updated_at_db, deleted_at_db],
                )
                .db()?;
                Ok(true)
            })
            .await
        {
            Ok(v) => v,
            Err(e) => {
                log::warn!("app_icon {process_name}: merge failed: {e}");
                continue;
            }
        };

        // The cache file holds the PNG in this row's `icon_png`, and is removed when
        // the row is a tombstone; so it changes only when the row did, and nothing
        // happens when the record was older than the row.
        if applied {
            let path = match crate::repo::app_icons::icon_cache_path(&process_name) {
                Ok(p) => p,
                Err(e) => {
                    log::warn!("app_icon {process_name}: no icon cache path: {e}");
                    continue;
                }
            };
            if deleted_at.is_some() {
                let _ = std::fs::remove_file(&path);
            } else {
                crate::repo::app_icons::write_cache_file(&path, &icon_bytes);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{T_MID, T_NEW};
    use super::*;
    use crate::repo::test_util::fresh_test_pool;

    fn icon_body(rows: Vec<(&str, String, &str)>) -> Vec<u8> {
        serde_json::to_vec(
            &rows
                .into_iter()
                .map(|(p, b64, ts)| AppIconPayload {
                    process_name: p.into(),
                    icon_png_base64: b64,
                    updated_at: ts.into(),
                    deleted_at: None,
                })
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    async fn icon_row(pool: &DbPool, process: &str) -> Option<(Vec<u8>, String)> {
        let process = process.to_string();
        pool.0
            .call(move |conn| {
                use rusqlite::OptionalExtension;
                let r = conn
                    .query_row(
                        "SELECT icon_png, updated_at FROM app_icons WHERE process_name = ?1",
                        rusqlite::params![process],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                    .db()?;
                Ok(r)
            })
            .await
            .unwrap()
    }

    /// 坏 base64 只跳那一行,同文件后续好行照常落库。
    // 好行应用成功后会写 icon 文件 cache(路径读 HINDSIGHT_DATA_DIR),必须借
    // env 锁把数据根指到唯一临时目录,不污染真实用户目录。锁横跨 merge 的 await:
    // #[tokio::test] 单线程 runtime,不自死锁(同 chat/engine.rs 夹具的先例)。
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn merge_app_icons_bad_base64_skips_row_keeps_rest() {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
        let pool = fresh_test_pool().await;
        let good_bytes: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 9, 8, 7];

        let body = icon_body(vec![
            ("Icon-bad", "!!!这不是base64!!!".into(), T_MID),
            ("Icon-good", BASE64.encode(&good_bytes), T_MID),
        ]);

        let _env_lock = crate::repo::test_util::lock_data_dir_env();
        let dir =
            std::env::temp_dir().join(format!("hindsight-pull-icons-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let prev = std::env::var("HINDSIGHT_DATA_DIR").ok();
        std::env::set_var("HINDSIGHT_DATA_DIR", &dir);
        let res = merge(&pool, &body).await;
        match prev {
            Some(v) => std::env::set_var("HINDSIGHT_DATA_DIR", v),
            None => std::env::remove_var("HINDSIGHT_DATA_DIR"),
        }
        res.unwrap();

        assert!(
            icon_row(&pool, "Icon-bad").await.is_none(),
            "坏 base64 行应被跳过,不落库"
        );
        assert_eq!(
            icon_row(&pool, "Icon-good").await.unwrap(),
            (good_bytes, T_MID.to_string()),
            "同文件的好行应照常解码落库(字节精确一致)"
        );
    }

    /// LWW:远端较旧 / 同 updated_at 都不得覆盖本地已有 icon 字节。
    /// (两行都走 not-applied 路径,不会碰文件 cache,无需 env 隔离。)
    #[tokio::test]
    async fn merge_app_icons_lww_old_or_equal_does_not_overwrite() {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
        let pool = fresh_test_pool().await;
        let local_bytes: Vec<u8> = vec![1, 1, 2, 3, 5, 8];
        let remote_bytes: Vec<u8> = vec![9, 9, 9];

        for (proc, ts) in [("I-older", T_NEW), ("I-equal", T_MID)] {
            let bytes = local_bytes.clone();
            let proc = proc.to_string();
            let ts = ts.to_string();
            pool.0
                .call(move |conn| {
                    conn.execute(
                        "INSERT INTO app_icons(process_name, icon_png, updated_at, deleted_at)
                         VALUES(?1, ?2, ?3, NULL)",
                        rusqlite::params![proc, bytes, ts],
                    )
                    .db()?;
                    Ok(())
                })
                .await
                .unwrap();
        }

        let body = icon_body(vec![
            ("I-older", BASE64.encode(&remote_bytes), T_MID), // T_MID < 本地 T_NEW
            ("I-equal", BASE64.encode(&remote_bytes), T_MID), // 平局
        ]);
        merge(&pool, &body).await.unwrap();

        assert_eq!(
            icon_row(&pool, "I-older").await.unwrap(),
            (local_bytes.clone(), T_NEW.to_string()),
            "远端较旧不得覆盖本地字节"
        );
        assert_eq!(
            icon_row(&pool, "I-equal").await.unwrap(),
            (local_bytes, T_MID.to_string()),
            "同 updated_at 严格不覆盖(平局本地赢)"
        );
    }
}
