//! 通用 DB helper：把 `rusqlite::Result<T>` 接进 `tokio_rusqlite::Result<T>` 时
//! 少写一遍 `.map_err(tokio_rusqlite::Error::Rusqlite)`。
//!
//! 用法：
//! ```ignore
//! use crate::db::SqliteResultExt;
//!
//! let mut stmt = conn.prepare("SELECT 1").db()?;
//! conn.execute("INSERT INTO t VALUES(?)", params![v]).db()?;
//! ```
//!
//! 等价于：`.map_err(tokio_rusqlite::Error::Rusqlite)?`，没有任何运行时开销
//! （单态化后就是同一段代码），唯一目的是把噪声从 SQL 调用点上拿掉。

pub trait SqliteResultExt<T> {
    /// rusqlite::Result<T> → tokio_rusqlite::Result<T>
    fn db(self) -> tokio_rusqlite::Result<T>;
}

impl<T> SqliteResultExt<T> for rusqlite::Result<T> {
    #[inline]
    fn db(self) -> tokio_rusqlite::Result<T> {
        self.map_err(tokio_rusqlite::Error::Rusqlite)
    }
}
