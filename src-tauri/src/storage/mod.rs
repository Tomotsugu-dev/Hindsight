//! Storage: the database connection and pool, schema migrations, and a shared DB helper trait.
//!
//! With [`SqliteResultExt`], a `rusqlite::Result<T>` goes into a [`DbResult<T>`] without writing
//! `.map_err(DbError::Rusqlite)` each time.
//!
//! Usage:
//! ```ignore
//! use crate::storage::SqliteResultExt;
//!
//! let mut stmt = conn.prepare("SELECT 1").db()?;
//! conn.execute("INSERT INTO t VALUES(?)", params![v]).db()?;
//! ```
//!
//! It is the same as `.map_err(DbError::Rusqlite)?` and costs nothing at run time: after
//! monomorphization it is the same code. Its only purpose is to keep the noise out of SQL calls.

pub mod connection;
pub mod migrations;
pub mod pool;

pub use connection::{Connection, DbError, DbResult};
pub use pool::{db_path, db_path_dir, db_path_for, DbPool};

/// Turns a `rusqlite::Result<T>` into a [`DbResult<T>`] in one step with `.db()?`. See the
/// module docs.
pub trait SqliteResultExt<T> {
    /// rusqlite::Result<T> → DbResult<T>
    fn db(self) -> DbResult<T>;
}

impl<T> SqliteResultExt<T> for rusqlite::Result<T> {
    #[inline]
    fn db(self) -> DbResult<T> {
        self.map_err(DbError::Rusqlite)
    }
}

/// 当下时刻的 RFC3339 字符串，**强制 UTC**（`+00:00` 尾巴）。
///
/// 所有写到 DB `updated_at` / `created_at` 之类字段的"现在时刻"都应走这里。
/// 跨设备 sync 的 LWW 用字符串字典序比较 updated_at，**只有所有写入点都用同一时区
/// 才能保证字典序与时间序一致**。历史上 [`crate::repo::activities::insert_new`]
/// 误用 `captured_at.to_rfc3339()`（local TZ，`+09:00` 等）触发了"JST 凌晨字典序
/// 反转 → 对端 mirror stuck unsealed"的 bug（见 commit `ea7399b`）。
///
/// 不替换"未来时刻"表达式（如 `Utc::now() + Duration::hours(24)`）—— 那些 token
/// expiry / retry backoff 用例与本助手语义不重合。
pub fn utc_now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}
