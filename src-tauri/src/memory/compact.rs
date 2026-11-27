//! Gives the free space in the screen memory database back to the disk (ADR-0021).
//!
//! When SQLite deletes data or rebuilds a table, it only leaves free pages in the file. It does
//! not make the file smaller by itself. This module runs `VACUUM` to get the space back when the
//! share of free pages is over a limit and the disk has enough space.

use std::path::Path;
use std::time::Instant;

use rusqlite::Connection;

use crate::storage::{DbResult, SqliteResultExt};

const BYTES_PER_MIB: u64 = 1024 * 1024;

/// Compacts the screen memory database when needed to free disk space, and returns whether
/// `VACUUM` ran.
///
/// It runs `VACUUM` and truncates the WAL when more than a quarter of the pages are free, and the
/// disk that holds the database has free space of at least twice the database file size.
/// Otherwise it does nothing. No compaction state is saved: the same rule starts it after the
/// migration, after an interrupted run, or after a large purge.
pub fn compact(conn: &Connection) -> DbResult<bool> {
    compact_with(conn, || {
        conn.path()
            .and_then(|p| available_disk_bytes_at(Path::new(p)))
    })
}

/// Does the work of [`compact`], with the free disk space passed in so that tests can set it.
///
/// It calls `available_disk_bytes` only when the free pages are over the limit.
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
    // In WAL mode, VACUUM first writes the new pages into the WAL file. A TRUNCATE checkpoint
    // moves them into the database and cuts the WAL to zero bytes. Without it, the WAL keeps
    // taking about as much space as the whole database.
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

/// Returns the free space, in bytes, of the disk partition that holds `path`.
///
/// `sysinfo` only gives a list of mount points. So this takes the partition whose mount point is
/// the longest one that `path` starts with.
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
