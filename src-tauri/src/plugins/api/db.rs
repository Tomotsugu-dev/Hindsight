//! The `db.*` methods, which give each plugin its own SQLite database in `data.sqlite`. The plugin
//! creates its own tables.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::config::DbConfig;
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::limits::Limit;
use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{params_from_iter, Batch, Connection, Statement};
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{json, Value};
use tokio::sync::Semaphore;

use crate::error::{Error, Result};
use crate::repo::plugins::Plugin;

/// How long one SQLite statement may run before it is aborted.
const TIME_LIMIT: Duration = Duration::from_secs(5);

/// The maximum size in bytes of one string, BLOB or row.
const MAX_LENGTH: i32 = 256_000;

/// The limits set on every connection. SQLite's defaults assume trusted SQL, so they are lowered
/// for plugin SQL.
const LIMITS: [(Limit, i32); 12] = [
    (Limit::SQLITE_LIMIT_LENGTH, MAX_LENGTH),
    // Bytes in one SQL statement
    (Limit::SQLITE_LIMIT_SQL_LENGTH, 100_000),
    // Columns in a table, an index or a result
    (Limit::SQLITE_LIMIT_COLUMN, 100),
    // Nesting depth of expressions and of the parser stack
    (Limit::SQLITE_LIMIT_EXPR_DEPTH, 100),
    (Limit::SQLITE_LIMIT_PARSER_DEPTH, 100),
    // SELECTs joined by compound operators such as UNION
    (Limit::SQLITE_LIMIT_COMPOUND_SELECT, 3),
    // Virtual machine instructions in one compiled statement
    (Limit::SQLITE_LIMIT_VDBE_OP, 25_000),
    // Arguments of one function
    (Limit::SQLITE_LIMIT_FUNCTION_ARG, 8),
    (Limit::SQLITE_LIMIT_ATTACHED, 0),
    // Length of a LIKE or GLOB pattern; complex patterns match slowly
    (Limit::SQLITE_LIMIT_LIKE_PATTERN_LENGTH, 50),
    // The highest `?NNN` parameter number
    (Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 1_000),
    // How deep triggers can fire other triggers
    (Limit::SQLITE_LIMIT_TRIGGER_DEPTH, 10),
];

/// The maximum size in bytes of the JSON text that `db.query` returns.
const MAX_RESULT: usize = 10_000_000;

/// The number of plugin databases that may be open at the same time, across all plugins. Extra
/// calls wait in line.
static OPEN_DATABASES: Semaphore = Semaphore::const_new(4);

/// The PRAGMAs that plugins may use: reading the table structure, and `user_version`, which lets a
/// plugin record a version for its own tables.
const ALLOWED_PRAGMAS: [&str; 6] = [
    "user_version",
    "table_info",
    "table_xinfo",
    "index_list",
    "index_info",
    "foreign_key_list",
];

/// The PRAGMAs that plugins may read but not change. Full-text search and R-tree read them
/// internally.
const READ_ONLY_PRAGMAS: [&str; 2] = ["data_version", "page_size"];

/// The virtual tables that plugins may create.
const ALLOWED_MODULES: [&str; 2] = ["fts5", "rtree"];

/// The built-in functions that are denied for plugins.
const DENIED_FUNCTIONS: [&str; 6] = [
    // Loads any shared library
    "load_extension",
    // Returns the address of a tokenizer in memory
    "fts3_tokenizer",
    // These aggregate functions collect their result in memory, and check the limit for one
    // value only at the end
    "json_group_array",
    "jsonb_group_array",
    "json_group_object",
    "jsonb_group_object",
];

const BAD_PARAMS: &str = "db.execute and db.query need { sql, params? }, and params can hold only null, numbers, strings and booleans";
const ONE_STATEMENT: &str = "db.execute and db.query run exactly one SQL statement";
const RESULT_TOO_BIG: &str =
    "db.query results are limited to 10 MB; use LIMIT to read the rows a part at a time";

/// `db.execute`: runs one statement that returns no rows, and returns the number of changed rows
/// and the row id of the last insert.
///
/// # Errors
///
/// - The user gave the plugin no disk storage: [`Error::PluginNoPermission`]
/// - The parameters are invalid, or `sql` is not exactly one statement: [`Error::InvalidInput`]
/// - The SQL is wrong, is denied, times out, exceeds the storage limit, or returns rows:
///   [`Error::Sqlite`]
pub(super) async fn execute(plugins_dir: &Path, plugin: &Plugin, params: Value) -> Result<Value> {
    let (sql, values) = parse(params)?;
    run(plugins_dir, plugin, move |conn| {
        let changes = one_statement(conn, &sql)?.execute(params_from_iter(values))?;
        Ok(json!({ "changes": changes, "lastInsertRowid": conn.last_insert_rowid() }))
    })
    .await
}

/// `db.query`: runs one statement and returns all rows as JSON text. Each row is an object whose
/// keys are the column names.
///
/// # Errors
///
/// - The user gave the plugin no disk storage: [`Error::PluginNoPermission`]
/// - The parameters are invalid, `sql` is not exactly one statement, the result has a BLOB, or the
///   result is over 10 MB: [`Error::InvalidInput`]
/// - The SQL is wrong, is denied, times out, or exceeds the storage limit: [`Error::Sqlite`]
pub(super) async fn query(
    plugins_dir: &Path,
    plugin: &Plugin,
    params: Value,
) -> Result<Box<RawValue>> {
    let (sql, values) = parse(params)?;
    let json = run(plugins_dir, plugin, move |conn| {
        let mut stmt = one_statement(conn, &sql)?;
        let keys = stmt
            .column_names()
            .into_iter()
            .map(serde_json::to_string)
            .collect::<serde_json::Result<Vec<_>>>()?;
        let mut rows = stmt.query(params_from_iter(values))?;
        let mut json = String::from("[");
        while let Some(row) = rows.next()? {
            if json.len() > 1 {
                json.push(',');
            }
            json.push('{');
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    json.push(',');
                }
                json.push_str(key);
                json.push(':');
                json.push_str(&to_json(row.get_ref(i)?)?);
                // Check after every value: a row can have 100 columns, and one value can be 1 MB
                if json.len() > MAX_RESULT {
                    return Err(Error::InvalidInput(RESULT_TOO_BIG));
                }
            }
            json.push('}');
        }
        json.push(']');
        Ok(json)
    })
    .await?;
    Ok(RawValue::from_string(json)?)
}

/// The parameters of `db.execute` and `db.query`.
#[derive(Deserialize)]
struct Sql {
    sql: String,
    #[serde(default)]
    params: Vec<Value>,
}

/// Reads the SQL and the values to bind to it.
///
/// # Errors
///
/// - The parameters are not `{ sql, params? }`, or a value cannot be stored in SQLite:
///   [`Error::InvalidInput`]
fn parse(params: Value) -> Result<(String, Vec<SqlValue>)> {
    let Sql { sql, params } =
        serde_json::from_value(params).map_err(|_| Error::InvalidInput(BAD_PARAMS))?;
    let values = params
        .into_iter()
        .map(to_sql)
        .collect::<Option<_>>()
        .ok_or(Error::InvalidInput(BAD_PARAMS))?;
    Ok((sql, values))
}

/// Converts a value from a plugin into a SQLite value. Returns `None` for a value that SQLite
/// cannot store.
///
/// | JSON | SQLite |
/// |---|---|
/// | null | NULL |
/// | true / false | 1 / 0 |
/// | integer | INTEGER |
/// | decimal, or integer beyond i64 | REAL |
/// | string | TEXT |
/// | array, object | `None` |
fn to_sql(value: Value) -> Option<SqlValue> {
    Some(match value {
        Value::Null => SqlValue::Null,
        Value::Bool(b) => SqlValue::Integer(b.into()),
        Value::Number(n) => match n.as_i64() {
            Some(i) => SqlValue::Integer(i),
            None => SqlValue::Real(n.as_f64()?),
        },
        Value::String(s) => SqlValue::Text(s),
        Value::Array(_) | Value::Object(_) => return None,
    })
}

/// Writes a SQLite value as the JSON text returned to the plugin.
///
/// | SQLite | JSON |
/// |---|---|
/// | NULL | null |
/// | INTEGER | integer |
/// | REAL | decimal; infinity becomes null |
/// | TEXT | string |
///
/// # Errors
///
/// - The value is a BLOB: [`Error::InvalidInput`]
fn to_json(value: ValueRef<'_>) -> Result<String> {
    let value: Value = match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => i.into(),
        ValueRef::Real(f) => f.into(),
        ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned().into(),
        ValueRef::Blob(_) => return Err(Error::InvalidInput("db.query cannot return BLOB values")),
    };
    Ok(value.to_string())
}

/// Returns the path of the plugin's database. It sits in the plugin folder, outside the version
/// folder, so an update leaves it alone and the file-reading module never hands it out.
fn db_path(plugins_dir: &Path, plugin: &Plugin) -> PathBuf {
    plugins_dir.join(&plugin.id).join("data.sqlite")
}

/// Opens the plugin's database on a blocking thread, runs `f`, and closes it again. No file is
/// left open, so an uninstall can delete the database.
///
/// # Errors
///
/// - The user gave the plugin no disk storage: [`Error::PluginNoPermission`]
async fn run<T: Send + 'static>(
    plugins_dir: &Path,
    plugin: &Plugin,
    f: impl FnOnce(&Connection) -> Result<T> + Send + 'static,
) -> Result<T> {
    let storage_mb = plugin
        .storage_mb
        .ok_or_else(|| Error::PluginNoPermission("storage_mb".into()))?;
    let path = db_path(plugins_dir, plugin);
    // TODO: While the call waits here, the plugin may be disabled, uninstalled or given new disk
    // space, but this still uses the row read before the wait. Read the plugins row again after
    // getting a slot
    let permit = OPEN_DATABASES
        .acquire()
        .await
        .map_err(|e| Error::Other(format!("plugin db: {e}")))?;
    tokio::task::spawn_blocking(move || {
        // Move the permit into the thread so it is returned only after the SQL finishes. If the
        // caller gives up early the SQL keeps running, so the permit must not be returned yet.
        let _permit = permit;
        f(&open(&path, storage_mb, TIME_LIMIT)?)
    })
    .await
    .map_err(|e| Error::Other(format!("plugin db: {e}")))?
}

/// Opens the plugin's database, creating it if needed, and closes every way for SQL to reach
/// outside this file. The database may grow to `storage_mb` MB, and a statement is aborted once
/// it runs longer than `time_limit`.
fn open(path: &Path, storage_mb: u32, time_limit: Duration) -> Result<Connection> {
    let conn = Connection::open(path)?;
    // Refuse writes that could corrupt the file, such as changing the tables behind full-text
    // search or R-tree directly
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    // Keep intermediate results of sorting and de-duplication on disk, not in Hindsight's memory
    conn.pragma_update(None, "temp_store", "FILE")?;
    set_max_size(&conn, "main", storage_mb)?;
    for (limit, value) in LIMITS {
        conn.set_limit(limit, value)?;
    }
    // The PRAGMAs above are not on the allow list, so the authorizer denies them once installed
    conn.authorizer(Some(authorize))?;
    let started = Instant::now();
    // SQLite calls the closure every 1000 virtual machine instructions; returning true aborts
    // the running statement
    conn.progress_handler(1000, Some(move || started.elapsed() > time_limit))?;
    Ok(conn)
}

/// Limits the database `schema` to `mb` MB. Writes past the limit fail with "database or disk is
/// full".
fn set_max_size(conn: &Connection, schema: &str, mb: u32) -> Result<()> {
    let page_size: i64 = conn.pragma_query_value(Some(schema), "page_size", |row| row.get(0))?;
    let pages = i64::from(mb) * 1024 * 1024 / page_size;
    conn.pragma_update(Some(schema), "max_page_count", pages)?;
    Ok(())
}

/// The authorizer callback. While compiling a statement, SQLite asks it once for every action in
/// the statement. It allows only actions on the tables of this database and denies the rest.
fn authorize(ctx: AuthContext<'_>) -> Authorization {
    use AuthAction::*;
    // Deny the temp database
    if ctx
        .database_name
        .is_some_and(|db| db.eq_ignore_ascii_case("temp"))
    {
        return Authorization::Deny;
    }
    let allowed = match ctx.action {
        // Read and write table data. ATTACH and the temp database are denied, so this file is
        // the only one that can be touched
        Select | Read { .. } | Insert { .. } | Update { .. } | Delete { .. } => true,
        // WITH RECURSIVE only costs time, and a run that takes too long is aborted
        Recursive => true,
        // A transaction lasts only for this call: when the connection closes, anything not
        // committed is rolled back
        Transaction { .. } | Savepoint { .. } => true,
        // These change the structure of this database only
        CreateTable { .. }
        | CreateIndex { .. }
        | CreateTrigger { .. }
        | CreateView { .. }
        | DropTable { .. }
        | DropIndex { .. }
        | DropTrigger { .. }
        | DropView { .. }
        | AlterTable { .. }
        | Reindex { .. }
        | Analyze { .. } => true,
        // Full-text search has only FTS5. FTS3, FTS4, dbstat and the other compiled-in modules
        // are denied
        CreateVtable { module_name, .. } | DropVtable { module_name, .. } => ALLOWED_MODULES
            .iter()
            .any(|m| module_name.eq_ignore_ascii_case(m)),
        // Most built-in functions only compute values and never touch files. The exceptions are
        // in DENIED_FUNCTIONS
        Function { function_name } => !DENIED_FUNCTIONS
            .iter()
            .any(|f| function_name.eq_ignore_ascii_case(f)),
        // Some PRAGMAs change settings of the whole process, so only the listed ones pass. A
        // PRAGMA written as a function comes through here too
        Pragma {
            pragma_name,
            pragma_value,
        } => {
            let listed = |list: &[&str]| list.iter().any(|p| pragma_name.eq_ignore_ascii_case(p));
            listed(&ALLOWED_PRAGMAS) || (pragma_value.is_none() && listed(&READ_ONLY_PRAGMAS))
        }
        // ATTACH opens another database file and DETACH removes one. Unknown is an action that
        // this version of rusqlite does not recognize
        Attach { .. } | Detach { .. } | Unknown { .. } => false,
        // These work in the temp database
        CreateTempTable { .. }
        | CreateTempIndex { .. }
        | CreateTempTrigger { .. }
        | CreateTempView { .. }
        | DropTempTable { .. }
        | DropTempIndex { .. }
        | DropTempTrigger { .. }
        | DropTempView { .. } => false,
        // Actions added by a future rusqlite are denied because they are not known
        _ => false,
    };
    if allowed {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}

/// Compiles the one statement in `sql`.
///
/// # Errors
///
/// - `sql` has no statement, or has more than one: [`Error::InvalidInput`]
fn one_statement<'c>(conn: &'c Connection, sql: &str) -> Result<Statement<'c>> {
    let mut batch = Batch::new(conn, sql);
    let stmt = batch.next()?.ok_or(Error::InvalidInput(ONE_STATEMENT))?;
    if batch.next()?.is_some() {
        return Err(Error::InvalidInput(ONE_STATEMENT));
    }
    Ok(stmt)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::plugins::test_util::{plugin_with_permissions, TempDir};

    /// 插件库里的一个插件，和装好以后一样有自己的文件夹，用户给了 10 MB 磁盘容量。
    struct Installed {
        plugins: PathBuf,
        plugin: Plugin,
    }

    impl Installed {
        fn new(dir: &TempDir, id: &str) -> Self {
            fs::create_dir_all(dir.plugins().join(id)).unwrap();
            let mut plugin = plugin_with_permissions(id, &[]);
            plugin.storage_mb = Some(10);
            Self {
                plugins: dir.plugins(),
                plugin,
            }
        }

        fn db_file(&self) -> PathBuf {
            db_path(&self.plugins, &self.plugin)
        }

        async fn exec(&self, sql: &str) -> Result<Value> {
            self.exec_with(sql, json!([])).await
        }

        async fn exec_with(&self, sql: &str, values: Value) -> Result<Value> {
            let params = json!({ "sql": sql, "params": values });
            execute(&self.plugins, &self.plugin, params).await
        }

        async fn select(&self, sql: &str) -> Result<Value> {
            self.query_with(json!({ "sql": sql })).await
        }

        async fn query_with(&self, params: Value) -> Result<Value> {
            let raw = query(&self.plugins, &self.plugin, params).await?;
            Ok(serde_json::from_str(raw.get()).unwrap())
        }
    }

    /// 数 1 到 `n` 的 CTE：表 c，列 i。
    fn count_to(n: u32) -> String {
        format!("WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM c WHERE i < {n})")
    }

    /// 建表、写入、读出：每种值原样回来，布尔变成 0 和 1；execute 报改了几行和行号。
    #[tokio::test]
    async fn execute_then_query() {
        let dir = TempDir::new();
        let focus = Installed::new(&dir, "hindsight-focus");

        let create = "CREATE TABLE pomodoro (date TEXT, minutes INTEGER, rate REAL, done, note)";
        focus.exec(create).await.unwrap();
        let insert = "INSERT INTO pomodoro VALUES (?, ?, ?, ?, ?)";
        let values = json!(["2026-10-19", 25, 0.5, true, null]);
        let result = focus.exec_with(insert, values).await.unwrap();
        assert_eq!(result, json!({ "changes": 1, "lastInsertRowid": 1 }));

        let rows = focus.select("SELECT * FROM pomodoro").await.unwrap();
        assert_eq!(
            rows,
            json!([{ "date": "2026-10-19", "minutes": 25, "rate": 0.5, "done": 1, "note": null }])
        );
        assert!(dir.plugins().join("hindsight-focus/data.sqlite").exists());
    }

    /// 列名和文本里的引号、换行、中文原样回来。
    #[tokio::test]
    async fn keeps_quotes_in_names_and_text() {
        let dir = TempDir::new();
        let focus = Installed::new(&dir, "hindsight-focus");

        let sql = "SELECT 'say \"专注\"' || char(10) AS \"a\"\"b\", 9e999 AS inf";
        assert_eq!(
            focus.select(sql).await.unwrap(),
            json!([{ "a\"b": "say \"专注\"\n", "inf": null }])
        );
    }

    /// 结果超过 10 MB 报错，叫插件用 LIMIT 分几次读；不到 10 MB 照常返回。
    #[tokio::test]
    async fn limits_result_size() {
        let dir = TempDir::new();
        let focus = Installed::new(&dir, "hindsight-focus");

        // 每行 200 KB 的文本，60 行共 12 MB
        let rows = format!("{} SELECT hex(zeroblob(100000)) AS x FROM c", count_to(60));
        let err = focus.select(&rows).await.unwrap_err();
        assert!(err.to_string().contains("LIMIT"), "{err}");

        let rows = focus.select(&format!("{rows} LIMIT 45")).await.unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 45);
    }

    /// 每个插件一个库：A 建的表，B 看不到。
    #[tokio::test]
    async fn each_plugin_has_its_own_database() {
        let dir = TempDir::new();
        let a = Installed::new(&dir, "plugin-a");
        let b = Installed::new(&dir, "plugin-b");

        a.exec("CREATE TABLE secrets (x)").await.unwrap();
        let err = b.select("SELECT * FROM secrets").await.unwrap_err();
        assert!(err.to_string().contains("no such table"), "{err}");
    }

    /// 伸出这个文件的路全部失败，也不会在别处留下文件。大小写、注释不影响。
    #[tokio::test]
    async fn rejects_ways_out_of_the_file() {
        let dir = TempDir::new();
        let focus = Installed::new(&dir, "hindsight-focus");
        focus.exec("CREATE TABLE t (x)").await.unwrap();
        let outside = dir.0.join("outside.sqlite");
        let path = outside.to_string_lossy();

        for sql in [
            format!("ATTACH '{path}' AS other"),
            format!("attach /* 注释 */ database '{path}' as other"),
            format!("VACUUM INTO '{path}'"),
            "DETACH main".to_string(),
            format!("PRAGMA temp_store_directory = '{}'", dir.0.display()),
            "PRAGMA writable_schema = ON".to_string(),
            // 写成函数的 PRAGMA 也要过名单
            "SELECT * FROM pragma_writable_schema".to_string(),
            "SELECT * FROM pragma_integrity_check".to_string(),
            "SELECT load_extension('anything')".to_string(),
        ] {
            // 被授权回调拒绝，不是别的原因：报 not authorized 或 authorization denied
            for err in [focus.exec(&sql).await, focus.select(&sql).await] {
                let err = err.unwrap_err().to_string();
                assert!(err.contains("authoriz"), "{sql}: {err}");
            }
        }
        assert!(!outside.exists());
    }

    /// 插件能用 user_version 给自己的表记版本，也能读表结构。
    #[tokio::test]
    async fn allows_the_listed_pragmas() {
        let dir = TempDir::new();
        let focus = Installed::new(&dir, "hindsight-focus");

        focus.exec("PRAGMA user_version = 3").await.unwrap();
        let version = focus.select("PRAGMA user_version").await.unwrap();
        assert_eq!(version, json!([{ "user_version": 3 }]));

        focus.exec("CREATE TABLE t (x)").await.unwrap();
        let sql = "SELECT name FROM pragma_table_info('t')";
        assert_eq!(focus.select(sql).await.unwrap(), json!([{ "name": "x" }]));
    }

    /// 把结果攒在内存里的 JSON 聚合函数都拒绝，写成窗口函数、换大小写也一样；
    /// group_concat 和普通的 JSON 函数照常能用。
    #[tokio::test]
    async fn rejects_json_aggregates() {
        let dir = TempDir::new();
        let focus = Installed::new(&dir, "hindsight-focus");
        focus.exec("CREATE TABLE t (k, v)").await.unwrap();
        focus.exec("INSERT INTO t VALUES ('a', 1)").await.unwrap();

        for sql in [
            "SELECT json_group_array(v) FROM t",
            "SELECT jsonb_group_array(v) FROM t",
            "SELECT json_group_object(k, v) FROM t",
            "SELECT jsonb_group_object(k, v) FROM t",
            "SELECT JSON_GROUP_ARRAY(v) OVER () FROM t",
        ] {
            let err = focus.select(sql).await.unwrap_err().to_string();
            assert!(err.contains("authoriz"), "{sql}: {err}");
        }
        let sql = "SELECT group_concat(v) AS s, json_object('k', k) AS o FROM t";
        assert_eq!(
            focus.select(sql).await.unwrap(),
            json!([{ "s": "1", "o": "{\"k\":\"a\"}" }])
        );
    }

    /// 一次只执行一条语句：多条、空的都报错，第一条也不执行。
    #[tokio::test]
    async fn runs_exactly_one_statement() {
        let dir = TempDir::new();
        let focus = Installed::new(&dir, "hindsight-focus");
        focus.exec("CREATE TABLE t (x)").await.unwrap();

        for sql in [
            "INSERT INTO t VALUES (1); DROP TABLE t",
            "",
            "  -- 只有注释",
        ] {
            let err = focus.exec(sql).await.unwrap_err();
            assert!(matches!(err, Error::InvalidInput(_)), "{sql}: {err}");
        }
        let count = focus.select("SELECT count(*) AS n FROM t").await.unwrap();
        assert_eq!(count, json!([{ "n": 0 }]));
        // 末尾的分号和空白不算第二条
        focus.exec("INSERT INTO t VALUES (1);  ").await.unwrap();
    }

    /// 参数不对、值是数组或对象、结果里有 BLOB，都报参数错误；execute 遇到返回行的语句报错。
    #[tokio::test]
    async fn rejects_values_it_cannot_pass() {
        let dir = TempDir::new();
        let focus = Installed::new(&dir, "hindsight-focus");

        for params in [
            Value::Null,
            json!({ "params": [] }),
            json!({ "sql": "SELECT ?", "params": [[1]] }),
            json!({ "sql": "SELECT ?", "params": [{ "a": 1 }] }),
        ] {
            let err = focus.query_with(params.clone()).await.unwrap_err();
            assert!(matches!(err, Error::InvalidInput(_)), "{params}: {err}");
        }
        let err = focus.select("SELECT x'00' AS b").await.unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "{err}");
        let err = focus.exec("SELECT 1").await.unwrap_err();
        assert!(matches!(err, Error::Sqlite(_)), "{err}");
    }

    /// 跑太久的语句被中止。
    #[test]
    fn stops_slow_statements() {
        let dir = TempDir::new();
        let path = dir.0.join("data.sqlite");
        let conn = open(&path, 10, Duration::from_millis(100)).unwrap();

        let started = Instant::now();
        let endless =
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x FROM c) SELECT count(*) FROM c";
        let result: rusqlite::Result<i64> = conn.query_row(endless, [], |row| row.get(0));
        let err = result.unwrap_err();
        assert!(err.to_string().contains("interrupted"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// 插件占不了多少内存：超过上限的值报错，中间结果放在磁盘上。
    #[tokio::test]
    async fn keeps_memory_small() {
        let dir = TempDir::new();
        let focus = Installed::new(&dir, "hindsight-focus");

        let sql = format!("SELECT length(zeroblob({})) AS n", MAX_LENGTH + 1);
        let err = focus.select(&sql).await.unwrap_err();
        assert!(err.to_string().contains("too big"), "{err}");
        let sql = format!("SELECT length(zeroblob({MAX_LENGTH})) AS n");
        assert_eq!(
            focus.select(&sql).await.unwrap(),
            json!([{ "n": MAX_LENGTH }])
        );

        // 授权回调会拒绝读这个 PRAGMA，先去掉
        let conn = open(&dir.0.join("data.sqlite"), 10, TIME_LIMIT).unwrap();
        conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .unwrap();
        let temp_store: i64 = conn
            .query_row("PRAGMA temp_store", [], |row| row.get(0))
            .unwrap();
        // 1 是 FILE
        assert_eq!(temp_store, 1);
    }

    /// 用户没给磁盘容量：db.* 都报没有权限，也不建库文件。
    #[tokio::test]
    async fn needs_storage() {
        let dir = TempDir::new();
        let mut focus = Installed::new(&dir, "hindsight-focus");
        focus.plugin.storage_mb = None;

        for err in [
            focus.exec("CREATE TABLE t (x)").await,
            focus.select("SELECT 1").await,
        ] {
            let err = err.unwrap_err();
            assert!(
                matches!(&err, Error::PluginNoPermission(p) if p == "storage_mb"),
                "{err}"
            );
        }
        assert!(!focus.db_file().exists());
    }

    /// 库文件长不过用户给的容量：超出的那条语句报错、整条不生效，之后还能接着写；
    /// 插件自己改不了上限。
    #[tokio::test]
    async fn stays_within_storage() {
        let dir = TempDir::new();
        let mut focus = Installed::new(&dir, "hindsight-focus");
        focus.plugin.storage_mb = Some(1);
        focus.exec("CREATE TABLE t (x)").await.unwrap();

        // 20 行，每行 100 KB，共 2 MB
        let too_much = format!(
            "INSERT INTO t {} SELECT zeroblob(100000) FROM c",
            count_to(20)
        );
        let err = focus.exec(&too_much).await.unwrap_err();
        assert!(err.to_string().contains("full"), "{err}");
        let count = focus.select("SELECT count(*) AS n FROM t").await.unwrap();
        assert_eq!(count, json!([{ "n": 0 }]));

        focus
            .exec("INSERT INTO t VALUES (zeroblob(100000))")
            .await
            .unwrap();
        assert!(fs::metadata(focus.db_file()).unwrap().len() <= 1024 * 1024);

        for sql in [
            "PRAGMA max_page_count = 1000000",
            "SELECT * FROM pragma_max_page_count",
            "PRAGMA page_size = 65536",
        ] {
            let err = focus.exec(sql).await.unwrap_err();
            assert!(err.to_string().contains("authoriz"), "{sql}: {err}");
        }
    }

    /// 临时库用不了：带 TEMP 的、写明 temp 库的都拒绝。这个库自己的表照常建、照常删。
    #[tokio::test]
    async fn rejects_the_temp_database() {
        let dir = TempDir::new();
        let focus = Installed::new(&dir, "hindsight-focus");
        focus.exec("CREATE TABLE t (x)").await.unwrap();

        for sql in [
            "CREATE TEMP TABLE scratch (x)",
            "CREATE TABLE temp.scratch (x)",
            "CREATE TEMP VIEW v AS SELECT x FROM t",
            "CREATE TEMP TRIGGER g AFTER INSERT ON t BEGIN SELECT 1; END",
            "SELECT * FROM temp.sqlite_master",
            "PRAGMA temp.user_version = 1",
        ] {
            // 被授权回调拒绝：读某一列报 prohibited，别的报 not authorized 或 authorization denied
            let err = focus.exec(sql).await.unwrap_err().to_string();
            assert!(
                err.contains("authoriz") || err.contains("prohibited"),
                "{sql}: {err}"
            );
        }
        focus.exec("DROP TABLE t").await.unwrap();
    }

    /// 超过各项上限的 SQL 都报错。多行的 VALUES 不算 UNION，不受 3 个 SELECT 的限制。
    #[tokio::test]
    async fn rejects_sql_over_the_limits() {
        let dir = TempDir::new();
        let focus = Installed::new(&dir, "hindsight-focus");

        let columns = (0..101).map(|i| format!("{i} AS c{i}")).collect::<Vec<_>>();
        let cases = [
            (format!("SELECT 1 {}", " ".repeat(100_000)), "too long"),
            (format!("SELECT {}", columns.join(", ")), "too many columns"),
            (format!("SELECT {}1", "1 + ".repeat(100)), "too large"),
            (
                format!("SELECT {}1{}", "(".repeat(101), ")".repeat(101)),
                "Recursion limit",
            ),
            (
                "SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4".to_string(),
                "too many terms",
            ),
            (
                format!("SELECT 1 IN ({}1)", "1, ".repeat(30_000)),
                "out of memory",
            ),
            (
                "SELECT max(1, 2, 3, 4, 5, 6, 7, 8, 9)".to_string(),
                "too many arguments",
            ),
            (
                format!("SELECT 'a' LIKE '{}'", "a".repeat(51)),
                "too complex",
            ),
            ("SELECT ?1001".to_string(), "variable number"),
        ];
        for (sql, message) in cases {
            let err = focus.select(&sql).await.unwrap_err().to_string();
            let start: String = sql.chars().take(40).collect();
            assert!(err.contains(message), "{start}: {err}");
        }

        let values = format!(
            "SELECT count(*) AS n FROM (VALUES {}(1))",
            "(1), ".repeat(99)
        );
        assert_eq!(focus.select(&values).await.unwrap(), json!([{ "n": 100 }]));
    }

    /// 触发器套触发器最多 10 层：往 t0 写一行，会一路写到 t11。
    #[tokio::test]
    async fn limits_trigger_depth() {
        let dir = TempDir::new();
        let focus = Installed::new(&dir, "hindsight-focus");
        for i in 0..12 {
            focus.exec(&format!("CREATE TABLE t{i} (x)")).await.unwrap();
        }
        for i in 0..11 {
            let next = i + 1;
            let sql = format!(
                "CREATE TRIGGER g{i} AFTER INSERT ON t{i} BEGIN INSERT INTO t{next} VALUES (1); END"
            );
            focus.exec(&sql).await.unwrap();
        }
        let err = focus.exec("INSERT INTO t0 VALUES (1)").await.unwrap_err();
        assert!(err.to_string().contains("trigger recursion"), "{err}");
    }

    /// 全文搜索和 R 树能用；全文搜索背后的表不能直接改。
    #[tokio::test]
    async fn virtual_tables_work() {
        let dir = TempDir::new();
        let focus = Installed::new(&dir, "hindsight-focus");
        focus
            .exec("CREATE VIRTUAL TABLE boxes USING rtree(id, x0, x1)")
            .await
            .unwrap();
        focus
            .exec("INSERT INTO boxes VALUES (1, 0, 10)")
            .await
            .unwrap();
        let hit = "SELECT id FROM boxes WHERE x0 <= 5 AND x1 >= 5";
        assert_eq!(focus.select(hit).await.unwrap(), json!([{ "id": 1 }]));

        focus
            .exec("CREATE VIRTUAL TABLE notes USING fts5(body)")
            .await
            .unwrap();
        focus
            .exec("INSERT INTO notes VALUES ('专注 25 分钟')")
            .await
            .unwrap();

        let err = focus
            .exec("INSERT INTO notes_data VALUES (99, x'00')")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("may not be modified"), "{err}");
        let found = "SELECT body FROM notes WHERE notes MATCH '专注'";
        assert_eq!(
            focus.select(found).await.unwrap(),
            json!([{ "body": "专注 25 分钟" }])
        );

        focus.exec("DROP TABLE notes").await.unwrap();
        focus.exec("DROP TABLE boxes").await.unwrap();
    }

    /// 只能建 FTS5 和 R 树，别的虚拟表换大小写也建不了；FTS3 的分词器函数拒绝。
    #[tokio::test]
    async fn rejects_other_virtual_tables() {
        let dir = TempDir::new();
        let focus = Installed::new(&dir, "hindsight-focus");

        for sql in [
            "CREATE VIRTUAL TABLE a USING fts3(body)",
            "CREATE VIRTUAL TABLE b USING FTS4(body)",
            "CREATE VIRTUAL TABLE c USING fts3tokenize(simple)",
            "CREATE VIRTUAL TABLE d USING dbstat",
        ] {
            let err = focus.exec(sql).await.unwrap_err().to_string();
            assert!(err.contains("authoriz"), "{sql}: {err}");
        }
        // 参数从 ? 传进来时，这个函数才交出地址，插件的 params 正是这样传的
        let err = focus
            .query_with(json!({ "sql": "SELECT fts3_tokenizer(?) AS p", "params": ["simple"] }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("authoriz"), "{err}");
        let left = "SELECT count(*) AS n FROM sqlite_schema";
        assert_eq!(focus.select(left).await.unwrap(), json!([{ "n": 0 }]));
    }

    /// 所有插件同时最多打开 4 个库：名额用完时，新的调用等别人还了名额才跑。
    #[tokio::test]
    async fn waits_for_an_open_slot() {
        let dir = TempDir::new();
        let focus = Installed::new(&dir, "hindsight-focus");

        let held = OPEN_DATABASES.acquire_many(4).await.unwrap();
        let call = tokio::spawn(async move { focus.select("SELECT 1 AS n").await });
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!call.is_finished());

        drop(held);
        assert_eq!(call.await.unwrap().unwrap(), json!([{ "n": 1 }]));
    }
}
