use crate::error::Result;
use crate::storage::DbPool;

const MIGRATIONS: &[&str] = &[
    // v1
    r#"
    CREATE TABLE IF NOT EXISTS activities (
        id              INTEGER PRIMARY KEY AUTOINCREMENT,
        started_at      TEXT NOT NULL,
        ended_at        TEXT NOT NULL,
        duration_secs   INTEGER NOT NULL,
        local_date      TEXT NOT NULL,
        local_hour      INTEGER NOT NULL,
        process_name    TEXT NOT NULL,
        window_title    TEXT,
        category_id     TEXT NOT NULL,
        screenshot_path TEXT,
        image_hash      INTEGER,
        device_id       TEXT NOT NULL DEFAULT 'local'
    );
    CREATE INDEX IF NOT EXISTS idx_activities_date       ON activities(local_date);
    CREATE INDEX IF NOT EXISTS idx_activities_date_hour  ON activities(local_date, local_hour);
    CREATE INDEX IF NOT EXISTS idx_activities_process    ON activities(process_name);
    CREATE INDEX IF NOT EXISTS idx_activities_device     ON activities(device_id);

    CREATE TABLE IF NOT EXISTS categories (
        id      TEXT PRIMARY KEY,
        name    TEXT NOT NULL,
        color   TEXT NOT NULL,
        builtin INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS app_categories (
        process_name TEXT PRIMARY KEY,
        category_id  TEXT NOT NULL REFERENCES categories(id)
    );
    "#,
    // v2
    r#"
    CREATE TABLE IF NOT EXISTS process_paths (
        process_name      TEXT PRIMARY KEY,
        exe_path          TEXT NOT NULL,
        seen_at           TEXT NOT NULL
    );
    "#,
    // v3：清空旧 seed 写进 app_categories 的假名字
    r#"
    DELETE FROM app_categories;
    "#,
    // v4：清掉历史记录里 process_name 是完整路径的脏数据
    // SQLite 没有 reverse/basename，cleanup 直接删，下次采集会用 basename 重新写入
    r#"
    DELETE FROM activities
    WHERE process_name LIKE '%\%' OR process_name LIKE '%/%';

    DELETE FROM process_paths
    WHERE process_name LIKE '%\%' OR process_name LIKE '%/%';
    "#,
    // v5：取消"内置分类不可删"的设定；同时确保 6 个默认分类存在（仅首启时插入）
    r#"
    UPDATE categories SET builtin = 0;

    INSERT OR IGNORE INTO categories(id, name, color, builtin) VALUES('code',   '编程', '#a78bfa', 0);
    INSERT OR IGNORE INTO categories(id, name, color, builtin) VALUES('browse', '浏览', '#60a5fa', 0);
    INSERT OR IGNORE INTO categories(id, name, color, builtin) VALUES('talk',   '沟通', '#34d399', 0);
    INSERT OR IGNORE INTO categories(id, name, color, builtin) VALUES('design', '设计', '#fbbf24', 0);
    INSERT OR IGNORE INTO categories(id, name, color, builtin) VALUES('fun',    '娱乐', '#fb7185', 0);
    INSERT OR IGNORE INTO categories(id, name, color, builtin) VALUES('other',  '其他', '#94a3b8', 0);
    "#,
    // v6：为分类加上图标字段，给 6 个默认分类填好默认图标
    r#"
    ALTER TABLE categories ADD COLUMN icon TEXT NOT NULL DEFAULT 'Tag';

    UPDATE categories SET icon = 'Code'           WHERE id = 'code';
    UPDATE categories SET icon = 'Globe'          WHERE id = 'browse';
    UPDATE categories SET icon = 'MessageCircle'  WHERE id = 'talk';
    UPDATE categories SET icon = 'Brush'          WHERE id = 'design';
    UPDATE categories SET icon = 'Gamepad2'       WHERE id = 'fun';
    UPDATE categories SET icon = 'MoreHorizontal' WHERE id = 'other';
    "#,
    // v7：用户设置（单行 JSON 表）
    r#"
    CREATE TABLE IF NOT EXISTS settings_store (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        data TEXT NOT NULL
    );
    INSERT OR IGNORE INTO settings_store(id, data) VALUES (1, '{}');
    "#,
];

pub async fn run(pool: &DbPool) -> Result<()> {
    pool.0
        .call(|conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER PRIMARY KEY)",
            )
            .map_err(tokio_rusqlite::Error::Rusqlite)?;

            for (idx, sql) in MIGRATIONS.iter().enumerate() {
                let version = (idx + 1) as i64;
                let already: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM schema_version WHERE version = ?",
                        [version],
                        |r| r.get(0),
                    )
                    .map_err(tokio_rusqlite::Error::Rusqlite)?;
                if already > 0 {
                    continue;
                }
                conn.execute_batch(sql)
                    .map_err(tokio_rusqlite::Error::Rusqlite)?;
                conn.execute(
                    "INSERT INTO schema_version VALUES (?)",
                    rusqlite::params![version],
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            }
            Ok(())
        })
        .await?;
    Ok(())
}
