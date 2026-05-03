use std::path::{Path, PathBuf};
use tokio_rusqlite::Connection;

use crate::error::{Error, Result};

#[derive(Clone)]
pub struct DbPool(pub Connection);

impl DbPool {
    pub async fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).await?;
        Ok(Self(conn))
    }
}

pub fn db_path() -> Result<PathBuf> {
    let dir = db_path_dir()?;
    Ok(dir.join("hindsight.sqlite"))
}

pub fn db_path_dir() -> Result<PathBuf> {
    let dir = dirs::data_dir()
        .ok_or_else(|| Error::Other("找不到系统数据目录".into()))?
        .join("Hindsight");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
