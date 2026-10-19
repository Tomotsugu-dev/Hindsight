//! Database connection: wraps the tokio-rusqlite connection and runs closures on its background
//! thread.
//!
//! How `call` is written and how errors display are fixed here, and do not change with the
//! tokio-rusqlite version.

use std::fmt::Display;
use std::path::Path;

use rusqlite::OpenFlags;

/// A database connection. A clone is still the same connection.
#[derive(Clone)]
pub struct Connection(tokio_rusqlite::Connection);

/// An error on the connection. It displays in the form `Rusqlite("…")`, and this is the text the
/// frontend sees in error messages.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// The connection is closed
    #[error("ConnectionClosed")]
    ConnectionClosed,
    /// Closing the connection failed
    #[error("Close((Connection, \"{0}\"))")]
    Close(#[source] rusqlite::Error),
    #[error("Rusqlite(\"{0}\")")]
    Rusqlite(#[from] rusqlite::Error),
    /// An error in the closure that does not come from SQLite
    #[error("Other(\"{0}\")")]
    Other(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}

pub type DbResult<T> = std::result::Result<T, DbError>;

impl Connection {
    /// Opens the database file, and creates it if it does not exist. With the path `":memory:"`,
    /// the database is in memory.
    pub async fn open(path: impl AsRef<Path>) -> DbResult<Self> {
        Ok(Self(tokio_rusqlite::Connection::open(path).await?))
    }

    /// Opens the database file with `flags`, such as read-only.
    pub async fn open_with_flags(path: impl AsRef<Path>, flags: OpenFlags) -> DbResult<Self> {
        Ok(Self(
            tokio_rusqlite::Connection::open_with_flags(path, flags).await?,
        ))
    }

    /// Runs `function` on the connection's background thread.
    ///
    /// # Errors
    ///
    /// - The connection is closed: [`DbError::ConnectionClosed`]
    /// - `function` returns an error: that error, as it is
    pub async fn call<F, R>(&self, function: F) -> DbResult<R>
    where
        F: FnOnce(&mut rusqlite::Connection) -> DbResult<R> + Send + 'static,
        R: Send + 'static,
    {
        self.0.call(function).await.map_err(from_tokio)
    }

    /// Closes the connection.
    ///
    /// # Errors
    ///
    /// - The connection is closed: [`DbError::ConnectionClosed`]
    /// - SQLite cannot close it: [`DbError::Close`]
    pub async fn close(self) -> DbResult<()> {
        self.0.close().await.map_err(from_tokio)
    }
}

/// Turns a tokio-rusqlite error into a [`DbError`].
fn from_tokio<E: Into<DbError> + Display>(e: tokio_rusqlite::Error<E>) -> DbError {
    match e {
        tokio_rusqlite::Error::Error(e) => e.into(),
        tokio_rusqlite::Error::Close((_, e)) => DbError::Close(e),
        tokio_rusqlite::Error::ConnectionClosed => DbError::ConnectionClosed,
        // Errors that later versions of tokio-rusqlite add
        e => DbError::Other(e.to_string().into()),
    }
}
