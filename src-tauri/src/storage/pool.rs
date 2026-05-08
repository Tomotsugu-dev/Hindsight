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
}

/// 当前生效的 SQLite 文件路径。多账号时按 `active_uid` 选 `hindsight.<uid>.sqlite`，
/// 未登录走匿名 `hindsight.sqlite`。
pub fn db_path() -> Result<PathBuf> {
    let dir = db_path_dir()?;
    let name = match crate::account::active_uid() {
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
