use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("db: {0}")]
    Db(#[from] tokio_rusqlite::Error),

    #[error("sql: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("capture: {0}")]
    Capture(String),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<Error> for String {
    fn from(e: Error) -> String {
        e.to_string()
    }
}
