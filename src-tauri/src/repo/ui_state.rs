//! The `ui_state` table: the main window's UI state, such as the language, the theme and the
//! selected device (ADR-0020).
//!
//! The keys are the ones the main window used in localStorage, and all start with `hindsight.`.
//! The frontend decides what each key means. Rust only checks keys and sizes, and reads the two
//! keys below.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};

use crate::error::{Error, Result};
use crate::storage::{DbPool, DbResult, SqliteResultExt};

/// Marks that the values in localStorage were imported. Once it is set, [`import`] writes nothing.
const MIGRATED_KEY: &str = "hindsight.localStorage.imported";

/// The device selected in the main window. Same key as `DEVICE_SELECTED_KEY` in
/// src/state/deviceFilterKey.ts: change both together.
const DEVICE_SELECTED_KEY: &str = "hindsight.device.selected";

const KEY_PREFIX: &str = "hindsight.";

/// The size limit of a value, in bytes. The largest value is the copy of a custom theme: its
/// theme.css can be 512 KiB, and as a JSON string it can be up to twice as long.
const MAX_VALUE_BYTES: usize = 2 * 1024 * 1024;

/// Reads every key and its value.
pub async fn load(pool: &DbPool) -> Result<HashMap<String, String>> {
    Ok(pool.0.call(|conn| read_all(conn)).await?)
}

/// Saves the value of `key`, or deletes the key if `value` is `None`.
///
/// # Errors
///
/// [`Error::InvalidInput`] if `key` does not start with `hindsight.` or `value` is over 2 MiB.
/// Nothing is changed then.
pub async fn set(pool: &DbPool, key: String, value: Option<String>) -> Result<()> {
    check(&key, value.as_deref())?;
    pool.0
        .call(move |conn| {
            match value {
                Some(value) => conn.execute(
                    "INSERT OR REPLACE INTO ui_state(key, value) VALUES (?1, ?2)",
                    [&key, &value],
                ),
                None => conn.execute("DELETE FROM ui_state WHERE key = ?1", [&key]),
            }
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Imports the values from localStorage, and returns every key and its value after the import.
///
/// A key already in the database keeps its value. An entry with a bad key or a value over the
/// limit is skipped. The values and [`MIGRATED_KEY`] are written in one transaction. If the
/// marker is already set, nothing is written, so each database imports only once.
pub async fn import(
    pool: &DbPool,
    entries: HashMap<String, String>,
) -> Result<HashMap<String, String>> {
    let total = entries.len();
    let entries: Vec<(String, String)> = entries
        .into_iter()
        .filter(|(key, value)| check(key, Some(value)).is_ok())
        .collect();
    if entries.len() < total {
        log::warn!(
            "ui_state: skipped {} bad entries in the import",
            total - entries.len()
        );
    }
    let state = pool
        .0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            let migrated: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM ui_state WHERE key = ?1)",
                    [MIGRATED_KEY],
                    |r| r.get(0),
                )
                .db()?;
            if !migrated {
                let mut imported = 0;
                for (key, value) in &entries {
                    imported += tx
                        .execute(
                            "INSERT OR IGNORE INTO ui_state(key, value) VALUES (?1, ?2)",
                            [key, value],
                        )
                        .db()?;
                }
                tx.execute(
                    "INSERT OR REPLACE INTO ui_state(key, value) VALUES (?1, '1')",
                    [MIGRATED_KEY],
                )
                .db()?;
                log::info!("ui_state: imported {imported} values from localStorage");
            }
            let state = read_all(&tx)?;
            tx.commit().db()?;
            Ok(state)
        })
        .await?;
    Ok(state)
}

/// Returns the device selected in the main window as it is stored, or `None` if none was ever
/// selected. plugins/api/stats.rs decides what the value means.
pub async fn selected_device(pool: &DbPool) -> Result<Option<String>> {
    let device = pool
        .0
        .call(|conn| {
            conn.query_row(
                "SELECT value FROM ui_state WHERE key = ?1",
                [DEVICE_SELECTED_KEY],
                |r| r.get(0),
            )
            .optional()
            .db()
        })
        .await?;
    Ok(device)
}

/// Checks that `key` starts with `hindsight.` and `value` is at most [`MAX_VALUE_BYTES`].
fn check(key: &str, value: Option<&str>) -> Result<()> {
    if !key.starts_with(KEY_PREFIX) {
        return Err(Error::InvalidInput(
            "ui_state key must start with hindsight.",
        ));
    }
    if value.is_some_and(|v| v.len() > MAX_VALUE_BYTES) {
        return Err(Error::InvalidInput("ui_state value is over 2 MiB"));
    }
    Ok(())
}

fn read_all(conn: &Connection) -> DbResult<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT key, value FROM ui_state").db()?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::repo::test_util::fresh_test_pool;
    use crate::storage::{migrations, DbPool};

    fn entries(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// A1：存进去的值，关掉数据库再打开还在。
    #[tokio::test]
    async fn set_value_survives_reopen() {
        let dir =
            std::env::temp_dir().join(format!("hindsight-ui-state-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("main.sqlite");

        let pool = DbPool::open(&path).await.unwrap();
        migrations::run(&pool).await.unwrap();
        set(&pool, "hindsight.locale".into(), Some("ja".into()))
            .await
            .unwrap();
        drop(pool);

        let pool = DbPool::open(&path).await.unwrap();
        let state = load(&pool).await.unwrap();
        assert_eq!(
            state.get("hindsight.locale").map(String::as_str),
            Some("ja")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn set_none_deletes_key() {
        let pool = fresh_test_pool().await;
        set(&pool, "hindsight.theme".into(), Some("dark".into()))
            .await
            .unwrap();
        set(&pool, "hindsight.theme".into(), None).await.unwrap();
        assert!(!load(&pool).await.unwrap().contains_key("hindsight.theme"));
    }

    #[tokio::test]
    async fn set_rejects_key_without_prefix() {
        let pool = fresh_test_pool().await;
        let result = set(&pool, "locale".into(), Some("ja".into())).await;
        assert!(matches!(result, Err(Error::InvalidInput(_))), "{result:?}");
        assert!(load(&pool).await.unwrap().is_empty());
    }

    /// 正好等于上限可以存，多一个字节就拒绝。
    #[tokio::test]
    async fn set_rejects_value_over_limit() {
        let pool = fresh_test_pool().await;
        let at_limit = "x".repeat(MAX_VALUE_BYTES);
        set(&pool, "hindsight.theme.custom".into(), Some(at_limit))
            .await
            .unwrap();

        let over = "y".repeat(MAX_VALUE_BYTES + 1);
        let result = set(&pool, "hindsight.theme.custom".into(), Some(over)).await;
        assert!(matches!(result, Err(Error::InvalidInput(_))), "{result:?}");
        let state = load(&pool).await.unwrap();
        assert!(state["hindsight.theme.custom"].starts_with('x'));
    }

    /// A2：旧值和完成标记一起写进去，返回值就是库里的全部状态。
    #[tokio::test]
    async fn import_copies_entries_and_writes_marker() {
        let pool = fresh_test_pool().await;
        let returned = import(
            &pool,
            entries(&[("hindsight.locale", "ja"), ("hindsight.theme", "dark")]),
        )
        .await
        .unwrap();

        assert_eq!(returned["hindsight.locale"], "ja");
        assert_eq!(returned["hindsight.theme"], "dark");
        assert!(returned.contains_key(MIGRATED_KEY));
        assert_eq!(returned, load(&pool).await.unwrap());
    }

    /// 数据库里已经有的值，导入不覆盖。
    #[tokio::test]
    async fn import_keeps_existing_value() {
        let pool = fresh_test_pool().await;
        set(&pool, "hindsight.locale".into(), Some("en".into()))
            .await
            .unwrap();
        let returned = import(&pool, entries(&[("hindsight.locale", "ja")]))
            .await
            .unwrap();
        assert_eq!(returned["hindsight.locale"], "en");
    }

    /// 键名不对、值超过上限的条目跳过，其余的和标记照常写入。
    #[tokio::test]
    async fn import_skips_bad_entries() {
        let pool = fresh_test_pool().await;
        let mut input = entries(&[("locale", "ja"), ("hindsight.device.selected", "all")]);
        input.insert(
            "hindsight.theme.custom".into(),
            "x".repeat(MAX_VALUE_BYTES + 1),
        );

        let returned = import(&pool, input).await.unwrap();
        let mut keys: Vec<&str> = returned.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["hindsight.device.selected", MIGRATED_KEY]);
    }

    /// 标记在了就不再导入：用户删掉的值不会被旧值带回来。
    #[tokio::test]
    async fn import_does_nothing_once_marker_is_set() {
        let pool = fresh_test_pool().await;
        import(&pool, entries(&[("hindsight.theme", "dark")]))
            .await
            .unwrap();
        set(&pool, "hindsight.theme".into(), None).await.unwrap();

        let returned = import(&pool, entries(&[("hindsight.theme", "dark")]))
            .await
            .unwrap();
        assert!(!returned.contains_key("hindsight.theme"));
    }

    /// A5：插件调用读到的设备，就是主窗口存下的那个。
    #[tokio::test]
    async fn selected_device_reads_saved_choice() {
        let pool = fresh_test_pool().await;
        assert_eq!(selected_device(&pool).await.unwrap(), None);

        set(
            &pool,
            "hindsight.device.selected".into(),
            Some("all".into()),
        )
        .await
        .unwrap();
        assert_eq!(
            selected_device(&pool).await.unwrap().as_deref(),
            Some("all")
        );
    }
}
