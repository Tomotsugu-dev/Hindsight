use std::path::{Path, PathBuf};
use tokio_rusqlite::Connection;

use crate::error::Result;

/// SQLite 连接池——对 `tokio_rusqlite::Connection` 的薄包装。
/// 一个进程一份，作为 Tauri State 注入；命令通过 `State<'_, DbPool>` 拿。
#[derive(Clone)]
pub struct DbPool(pub Connection);

impl DbPool {
    /// 打开（或新建）SQLite 数据库；不跑 migrations，调用方需另调 [`crate::storage::migrations::run`]。
    pub async fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).await?;
        Ok(Self(conn))
    }

    /// 在内存里开一个临时 SQLite DB——给单元测试用，调用方需自行跑 migrations。
    /// 每次返回一个全新独立的连接，互相之间不共享 schema / 数据。
    #[cfg(test)]
    pub async fn open_in_memory() -> Result<Self> {
        let conn = Connection::open(":memory:").await?;
        Ok(Self(conn))
    }
}

/// The main database file this run uses: `hindsight.<uid>.sqlite`, or
/// `hindsight.sqlite` when the database belongs to no account yet. The uid is
/// [`crate::account::db_uid`].
pub fn db_path() -> Result<PathBuf> {
    db_path_for(crate::account::db_uid().as_deref())
}

/// Where an account's database lives: `hindsight.<uid>.sqlite`. `None` gives
/// `hindsight.sqlite`, the database that belongs to no account yet.
pub fn db_path_for(uid: Option<&str>) -> Result<PathBuf> {
    let dir = db_path_dir()?;
    let name = match uid {
        Some(uid) => format!("hindsight.{uid}.sqlite"),
        None => "hindsight.sqlite".to_string(),
    };
    Ok(dir.join(name))
}

/// 数据根目录（[`crate::bootstrap::data_root`]）；不存在时自动创建。
pub fn db_path_dir() -> Result<PathBuf> {
    let dir = crate::bootstrap::data_root();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
