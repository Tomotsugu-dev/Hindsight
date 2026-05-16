//! 存储层：连接池 + schema 迁移 + 通用 DB helper trait。
//!
//! [`SqliteResultExt`] 让 `rusqlite::Result<T>` 接进 `tokio_rusqlite::Result<T>` 时
//! 少写一遍 `.map_err(tokio_rusqlite::Error::Rusqlite)`。
//!
//! 用法：
//! ```ignore
//! use crate::storage::SqliteResultExt;
//!
//! let mut stmt = conn.prepare("SELECT 1").db()?;
//! conn.execute("INSERT INTO t VALUES(?)", params![v]).db()?;
//! ```
//!
//! 等价于 `.map_err(tokio_rusqlite::Error::Rusqlite)?`，没有任何运行时开销
//! （单态化后就是同一段代码），唯一目的是把噪声从 SQL 调用点上拿掉。

pub mod migrations;
pub mod pool;

pub use pool::{db_path, db_path_dir, DbPool};

/// 让 `rusqlite::Result<T>` 通过 `.db()?` 一步转成 `tokio_rusqlite::Result<T>`。
/// 详见模块顶部说明。
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
