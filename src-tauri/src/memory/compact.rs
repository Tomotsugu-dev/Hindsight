//! 屏幕记忆库的磁盘空间回收（ADR-0021）。
//!
//! SQLite 删除数据或重建表后只会在文件内留下空闲页，不会自动缩小文件；
//! 本模块在空闲页占比超阈值且磁盘空间充足时执行 `VACUUM` 回收空间。

use std::path::Path;
use std::time::Instant;

use rusqlite::Connection;

use crate::storage::{DbResult, SqliteResultExt};

const BYTES_PER_MIB: u64 = 1024 * 1024;

/// 按需压缩屏幕记忆库并回收磁盘空间，返回是否执行了 `VACUUM`。
///
/// 当空闲页超过总页数的 1/4，且库所在磁盘的可用空间不少于当前库文件大小的 2 倍时，
/// 执行 `VACUUM` 并截断 WAL；否则跳过。不单独持久化压缩状态，迁移后、上次中断后或
/// 清除大量数据后均由同一条件触发。
pub fn compact(conn: &Connection) -> DbResult<bool> {
    compact_with(conn, || {
        conn.path()
            .and_then(|p| available_disk_bytes_at(Path::new(p)))
    })
}

/// [`compact`] 的内部实现；仅在空闲页超过阈值时才调用 `available_disk_bytes` 查询剩余磁盘空间，
/// 便于单元测试注入磁盘容量。
fn compact_with(
    conn: &Connection,
    available_disk_bytes: impl FnOnce() -> Option<u64>,
) -> DbResult<bool> {
    let read_pragma = |name: &str| -> DbResult<u64> {
        conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get(0))
            .db()
    };
    let (total_pages, free_pages, page_size) = (
        read_pragma("page_count")?,
        read_pragma("freelist_count")?,
        read_pragma("page_size")?,
    );
    if free_pages * 4 <= total_pages {
        return Ok(false);
    }
    let db_bytes = total_pages * page_size;
    let needed_bytes = db_bytes.saturating_mul(2);
    let free_bytes = available_disk_bytes().unwrap_or(0);
    if free_bytes < needed_bytes {
        log::info!(
            "memory: VACUUM skipped: {} MB free on disk, {} MB needed",
            free_bytes / BYTES_PER_MIB,
            needed_bytes / BYTES_PER_MIB
        );
        return Ok(false);
    }

    let started = Instant::now();
    // WAL 模式下 VACUUM 写入的新页会先落在 WAL 文件中；执行 TRUNCATE checkpoint 合并并截断 WAL，
    // 避免 WAL 长期占用接近整库大小的空间。
    conn.execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")
        .db()?;
    let compacted_bytes = read_pragma("page_count")? * page_size;
    log::info!(
        "memory: VACUUM done: {} MB to {} MB, {} ms",
        db_bytes / BYTES_PER_MIB,
        compacted_bytes / BYTES_PER_MIB,
        started.elapsed().as_millis()
    );
    Ok(true)
}

/// 查询 `path` 所在磁盘分区的可用空间（字节）。
///
/// `sysinfo` 仅提供挂载点列表，因此取挂载点为 `path` 前缀且路径最长的分区（最长前缀匹配）。
fn available_disk_bytes_at(path: &Path) -> Option<u64> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|d| path.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// 临时目录里的一个 WAL 库：一张表放 1000 行、每行 4 KB，再删掉前 `deleted` 行。
    fn database(name: &str, deleted: i64) -> (PathBuf, Connection) {
        let dir =
            std::env::temp_dir().join(format!("hindsight-compact-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("memory.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE t (id INTEGER PRIMARY KEY, data BLOB);
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1000)
             INSERT INTO t SELECT i, zeroblob(4096) FROM n;",
        )
        .unwrap();
        conn.execute("DELETE FROM t WHERE id <= ?1", [deleted])
            .unwrap();
        (dir, conn)
    }

    fn pragma(conn: &Connection, name: &str) -> i64 {
        conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get(0))
            .unwrap()
    }

    /// 空闲页超过四分之一、磁盘够：VACUUM，空闲页清零，WAL 也清空。
    #[test]
    fn vacuum_runs_when_over_a_quarter_is_free() {
        let (dir, conn) = database("runs", 900);
        let pages = pragma(&conn, "page_count");

        assert!(compact_with(&conn, || Some(u64::MAX)).unwrap());

        assert_eq!(pragma(&conn, "freelist_count"), 0);
        assert!(pragma(&conn, "page_count") < pages / 2);
        let wal = std::fs::metadata(dir.join("memory.sqlite-wal")).unwrap();
        assert_eq!(wal.len(), 0);
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 100);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 磁盘不够：跳过，库不动。
    #[test]
    fn vacuum_is_skipped_without_enough_disk() {
        let (dir, conn) = database("disk", 900);
        let free = pragma(&conn, "freelist_count");

        assert!(!compact_with(&conn, || Some(0)).unwrap());
        assert!(!compact_with(&conn, || None).unwrap());

        assert_eq!(pragma(&conn, "freelist_count"), free);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 空闲页不到四分之一：不 VACUUM，也不去查磁盘。
    #[test]
    fn little_free_space_needs_no_vacuum() {
        let (dir, conn) = database("little", 100);

        assert!(!compact_with(&conn, || panic!("不该查磁盘")).unwrap());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
