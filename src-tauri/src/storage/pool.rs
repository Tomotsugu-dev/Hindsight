use std::path::{Path, PathBuf};
use tokio_rusqlite::Connection;

use crate::error::Result;

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
    let name = match crate::account::active_uid() {
        Some(uid) => format!("hindsight.{uid}.sqlite"),
        None => "hindsight.sqlite".to_string(),
    };
    Ok(dir.join(name))
}

pub fn db_path_dir() -> Result<PathBuf> {
    let dir = crate::bootstrap::data_root();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
