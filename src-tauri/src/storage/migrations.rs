use crate::error::Result;
use crate::storage::DbPool;
use crate::db::SqliteResultExt;

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
    INSERT OR IGNORE INTO categories(id, name, color, builtin) VALUES('talk',   '社交', '#34d399', 0);
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
    // v8：多设备云同步基础设施
    // - 给共享数据加 updated_at + 软删除列（LWW + tombstone）
    // - activities 加 remote_id / updated_at / origin（不软删）
    // - 新表：devices / sync_outbox / sync_cursor / auth_state
    r#"
    ALTER TABLE categories     ADD COLUMN updated_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z';
    ALTER TABLE categories     ADD COLUMN deleted_at TEXT;
    ALTER TABLE app_categories ADD COLUMN updated_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z';
    ALTER TABLE app_categories ADD COLUMN deleted_at TEXT;
    ALTER TABLE process_paths  ADD COLUMN updated_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z';

    ALTER TABLE activities ADD COLUMN remote_id  TEXT;
    ALTER TABLE activities ADD COLUMN updated_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z';
    ALTER TABLE activities ADD COLUMN origin     TEXT NOT NULL DEFAULT 'local';

    CREATE UNIQUE INDEX IF NOT EXISTS idx_activities_remote
      ON activities(device_id, remote_id) WHERE remote_id IS NOT NULL;

    CREATE TABLE IF NOT EXISTS devices (
      device_id     TEXT PRIMARY KEY,
      display_name  TEXT NOT NULL,
      color         TEXT NOT NULL DEFAULT '#60a5fa',
      icon          TEXT NOT NULL DEFAULT 'Monitor',
      os            TEXT,
      last_seen_at  TEXT,
      is_self       INTEGER NOT NULL DEFAULT 0,
      updated_at    TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z',
      deleted_at    TEXT
    );

    CREATE TABLE IF NOT EXISTS sync_outbox (
      id            INTEGER PRIMARY KEY AUTOINCREMENT,
      op            TEXT NOT NULL,
      entity        TEXT NOT NULL,
      entity_pk     TEXT NOT NULL,
      payload       TEXT NOT NULL,
      created_at    TEXT NOT NULL,
      attempts      INTEGER NOT NULL DEFAULT 0,
      last_error    TEXT,
      next_retry_at TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_outbox_due ON sync_outbox(next_retry_at);

    CREATE TABLE IF NOT EXISTS sync_cursor (
      entity         TEXT PRIMARY KEY,
      last_pulled_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z'
    );

    CREATE TABLE IF NOT EXISTS auth_state (
      id                INTEGER PRIMARY KEY CHECK (id = 1),
      uid               TEXT,
      email             TEXT,
      refresh_token_enc BLOB,
      access_token      TEXT,
      expires_at        TEXT
    );
    INSERT OR IGNORE INTO auth_state(id) VALUES(1);

    -- 把现有 activities 的 updated_at 回填到 ended_at（首次同步会推这批存量）
    UPDATE activities SET updated_at = ended_at WHERE updated_at = '1970-01-01T00:00:00Z';
    "#,
    // v9：切换云同步后端，旧 cursor 名不再使用，outbox 从头开始。
    r#"
    DELETE FROM sync_outbox;
    DELETE FROM sync_cursor;
    "#,
    // v10：把默认分类 'talk' 的名字从 "沟通" 改成 "社交"。
    // 只在用户没改过名字时才动；已改名的不覆盖。同时 bump updated_at 让同步推一次。
    r#"
    UPDATE categories
       SET name = '社交',
           updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
     WHERE id = 'talk' AND name = '沟通';
    "#,
];

/// v11：清掉跨 OS 拉过来的 process_paths / app_categories 脏数据。
/// process_name / exe_path 在不同系统语义不一样：
///   Windows: process_name 形如 "chrome.exe"，exe_path "C:\\..."
///   macOS:   process_name 形如 "Google Chrome"，exe_path "/Applications/.../MacOS/..."
/// 在同步引擎做了 OS 过滤之前，这两张表里混进了别的 OS 的行（key 对不上 / 撞车把本机路径覆盖掉）。
/// activities 不在这里清 —— 跨设备聚合活动时长是 app 的核心价值，Windows 的活动记录留着。
#[cfg(target_os = "macos")]
const CROSS_OS_CLEANUP_SQL: &str = r#"
    DELETE FROM process_paths
    WHERE exe_path = '' OR exe_path NOT LIKE '/%';

    DELETE FROM app_categories
    WHERE process_name LIKE '%.exe' OR process_name LIKE '%.EXE';
"#;

// GLOB '?:\*' / '?:/*' —— Windows 路径形如 C:\... 或 C:/...
#[cfg(target_os = "windows")]
const CROSS_OS_CLEANUP_SQL: &str = r#"
    DELETE FROM process_paths
    WHERE exe_path = '' OR (exe_path NOT GLOB '?:\*' AND exe_path NOT GLOB '?:/*');

    DELETE FROM app_categories
    WHERE process_name NOT LIKE '%.exe';
"#;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const CROSS_OS_CLEANUP_SQL: &str = "";

/// v13：把每个历史 local_date（origin='local'）重新入 outbox，启动后 push tick 会把所有
/// 日期的 ndjson 全量推到 Drive。
///
/// 背景：v9 那次 backend 切换（Firestore → Drive）一刀切清空了 sync_outbox，导致 v9
/// 之前采集的活动数据从未推到 Drive。之后 [activities.rs:seal_session] 只对当前正在
/// 结束的那条会话入 outbox，历史日期没机会被重新触发 → Drive 上缺一大堆 ndjson。
///
/// 这里按 local_date 分组，每个日期一行 outbox：
///   - payload.localDate 是 [group_outbox] 唯一用到的字段（决定文件名 day 部分）
///   - entity_pk 用 MIN(id) 仅满足 NOT NULL 约束；build_activities_day 实际重新查全量
///   - next_retry_at 必须跟 chrono::to_rfc3339() 同格式 ("...+00:00") 才能被字典序对比
///     选中（[engine.rs:read_due_outbox]）。SQLite 的 datetime() 输出 "...Z" 后缀比不过，
///     这里直接写远古时间字面量。
///
/// 副作用：每台老设备启动后会一次性多推几次（每个历史日期一次），其它设备 pull 时会
/// 把 modifiedTime 刷新过的同名文件重新下载一遍，但去重靠 (device_id, remote_id)，
/// LWW 比较 updated_at，本地数据不会丢、不会重复。
const BACKFILL_OUTBOX_SQL: &str = r#"
    INSERT INTO sync_outbox(op, entity, entity_pk, payload, created_at, attempts, next_retry_at)
    SELECT 'upsert', 'activity', CAST(MIN(id) AS TEXT),
           json_object('localDate', local_date),
           '1970-01-01T00:00:00+00:00',
           0,
           '1970-01-01T00:00:00+00:00'
    FROM activities
    WHERE origin = 'local'
    GROUP BY local_date;
"#;

/// v12：占位，无操作。
///
/// 上一版本里这个槽位曾经是个错误的「删除跨 OS activities」迁移，已被 revert，但部分
/// 用户的 schema_version 表里 v12 已被记录为 done。新装机器跑这里 = no-op，老用户
/// 框架直接跳过 —— 两边都对得上，只是 v12 这个版本号永久作废。
const V12_PLACEHOLDER: &str = "";

/// v14：建 app_icons 表，存解码好的 PNG 字节，让 icon 数据跨设备同步。
/// 解决「Mac 看 Windows 同步过来的 activities 拿不到 icon」—— Windows 端的 chrome.exe
/// 在 Mac 上没文件可提取，必须由原始设备把 icon 字节传上来。
///
/// 不做 OS 过滤同步：process_name 跨 OS key 不撞（Win="chrome.exe" / mac="Google Chrome"），
/// 各 OS 上传各自的，对方按 process_name 精确查就能给跨设备的活动行渲染图标。
const APP_ICONS_TABLE_SQL: &str = r#"
    CREATE TABLE IF NOT EXISTS app_icons (
      process_name TEXT PRIMARY KEY,
      icon_png     BLOB NOT NULL,
      updated_at   TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z',
      deleted_at   TEXT
    );
"#;

/// v15：app_groups + app_group_members —— 跨设备配对的核心数据层。
///
/// 解决：xcap 在不同 OS 上对同一 app 返回不同 owner_name（mac="Code" /
/// win="Visual Studio Code"），导致同一 app 在 UI 里出现两条独立记录、分类要分别绑定
/// 两次。新模型把 (display_name, category_id) 挂到 group 上，每个 process_name 通过
/// app_group_members 指向自己的组。
///
/// **关键设计**：group_id 初始值就用 process_name 自身，不用随机 UUID。理由：两台设备
/// 各自跑 backfill 时用同样的 process_name 会产生**同样的 group_id**，跨设备同步后
/// 自然合并。如果用随机 UUID，每台 backfill 出不同 ID，sync 后会出现两个名字相同
/// 但 ID 不同的重复组。
///
/// 跨 OS 同步：app_groups 和 app_group_members 都不做 OS 过滤 —— 这就是这个功能的
/// 核心价值。
///
/// 与 app_categories 的关系：app_groups.category_id 是新的 source of truth；
/// app_categories 表保留作为「组 → 成员分类」的 derived view，由 group 操作的代码
/// 同步维护。下游报表查询 (reports.rs 的 LEFT JOIN app_categories) 不动，依然能拿到
/// 正确的分类。
///
/// Backfill 用临时表保证 (process_name → group_id) 的两次插入用同一个生成的 ID：
const APP_GROUPS_SQL: &str = r#"
    CREATE TABLE IF NOT EXISTS app_groups (
      id           TEXT PRIMARY KEY,
      display_name TEXT NOT NULL,
      category_id  TEXT,
      updated_at   TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z',
      deleted_at   TEXT
    );

    CREATE TABLE IF NOT EXISTS app_group_members (
      process_name TEXT PRIMARY KEY,
      group_id     TEXT NOT NULL REFERENCES app_groups(id),
      updated_at   TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z',
      deleted_at   TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_app_group_members_group ON app_group_members(group_id);

    -- Backfill：每个出现过的 process_name（来自 activities 或 app_categories）建一个
    -- 单成员组，group_id = process_name 自身（确定性、跨设备一致）。category_id 从
    -- 现有 app_categories 继承，让老用户配过的分类不丢。
    INSERT INTO app_groups (id, display_name, category_id, updated_at, deleted_at)
    SELECT p.process_name AS id,
           p.process_name AS display_name,
           (SELECT ac.category_id FROM app_categories ac
              WHERE ac.process_name = p.process_name AND ac.deleted_at IS NULL
              LIMIT 1) AS category_id,
           strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
           NULL
    FROM (
        SELECT DISTINCT process_name FROM activities
        UNION
        SELECT DISTINCT process_name FROM app_categories WHERE deleted_at IS NULL
    ) p
    WHERE p.process_name IS NOT NULL
      AND p.process_name <> ''
      AND p.process_name <> 'Unknown'
      AND NOT EXISTS (SELECT 1 FROM app_groups g WHERE g.id = p.process_name);

    INSERT INTO app_group_members (process_name, group_id, updated_at, deleted_at)
    SELECT p.process_name,
           p.process_name,
           strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
           NULL
    FROM (
        SELECT DISTINCT process_name FROM activities
        UNION
        SELECT DISTINCT process_name FROM app_categories WHERE deleted_at IS NULL
    ) p
    WHERE p.process_name IS NOT NULL
      AND p.process_name <> ''
      AND p.process_name <> 'Unknown'
      AND NOT EXISTS (SELECT 1 FROM app_group_members m WHERE m.process_name = p.process_name);
"#;

/// v16：给 categories 加 sort_order 列，让用户能拖拽排序。回填用现有的稳定可视顺序
/// （'other' 在最后、其它按 id 字典序）作为初始值，前端排序后会把每个 category 的
/// sort_order 改成 0,1,2... 然后跨同步走 category outbox 推到对端，LWW 保证一致。
const ADD_CATEGORY_SORT_ORDER_SQL: &str = r#"
    ALTER TABLE categories ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;

    UPDATE categories SET sort_order = (
        SELECT rn FROM (
            SELECT id,
                   ROW_NUMBER() OVER (ORDER BY (id = 'other') ASC, id) - 1 AS rn
            FROM categories
        ) ranked
        WHERE ranked.id = categories.id
    );
"#;

/// v17：给老安装的隐私 URL 关键词列表补上 `/password`。
/// 原因：v17 之前 default_privacy_url_keywords() 是 9 项，老安装的 settings_store
/// 已固化成 9 项；后端把 `/password` 加进默认（第 10 项）只对新安装生效，老用户列表
/// 里仍没有 `/password`，导致 chrome://password-manager 这类页面拦不住。
///
/// 三种状态分别处理：
/// - 字段不存在（settings JSON 没写过 privacyUrlKeywords）→ 写入完整 10 项默认
/// - 字段存在但列表里**没有** `/password` → append 一项
/// - 已经有 `/password` → 不动
///
/// 用户事后再删掉 `/password` 不会被这条迁移撤销（schema_version 只跑一次）。
const ADD_PASSWORD_TO_PRIVACY_DEFAULT_SQL: &str = r#"
    -- 字段缺失：直接补完整 10 项
    UPDATE settings_store
    SET data = json_set(
        data,
        '$.privacyUrlKeywords',
        json('["/login","/signin","/sign-in","/sign_in","/auth","/oauth","/sso","/logon","/connect/authorize","/password"]')
    )
    WHERE id = 1
      AND json_type(data, '$.privacyUrlKeywords') IS NULL;

    -- 字段已存在但不含 /password：append
    UPDATE settings_store
    SET data = json_insert(data, '$.privacyUrlKeywords[#]', '/password')
    WHERE id = 1
      AND json_type(data, '$.privacyUrlKeywords') = 'array'
      AND NOT EXISTS (
          SELECT 1 FROM json_each(json_extract(data, '$.privacyUrlKeywords'))
          WHERE value = '/password'
      );
"#;

/// v18：AI 总结结果缓存表（Phase 1B-γ）。
///
/// 按 (local_date, segment_idx) 主键缓存每段的 LLM 输出，避免切日期 / 重启后重跑——
/// vision 推理在 CPU 上 5 段 ~2-3 分钟，重算代价很大。
///
/// 字段语义：
/// - segment_idx 是该段在 settings.ai.segments 数组里的下标，用户改段配置后旧总结仍能查到（label/start_hour/end_hour 都冗余存了）
/// - model 存生成时用的 active_main 文件名；用户换模型后旧总结不擦，UI 可显示"由旧模型生成"
/// - status 区分 ok / skipped_no_screenshots / error；error 行 content 为空，error 字段填 fmt_send_err 输出
///
/// 不进 sync_outbox：本地产物 + 模型差异大，跨设备同步无意义。
const AI_SUMMARIES_TABLE_SQL: &str = r#"
    CREATE TABLE IF NOT EXISTS ai_summaries (
      local_date    TEXT NOT NULL,
      segment_idx   INTEGER NOT NULL,
      label         TEXT NOT NULL,
      start_hour    INTEGER NOT NULL,
      end_hour      INTEGER NOT NULL,
      content       TEXT NOT NULL,
      model         TEXT NOT NULL,
      status        TEXT NOT NULL,
      error         TEXT,
      generated_at  TEXT NOT NULL,
      PRIMARY KEY (local_date, segment_idx)
    );
    CREATE INDEX IF NOT EXISTS idx_ai_summaries_date ON ai_summaries(local_date);
"#;

pub async fn run(pool: &DbPool) -> Result<()> {
    // v1..v10 是 MIGRATIONS 静态数组，v11+ 平台/运行时拼装放 extras。
    // 顺序就是版本顺序（idx + static_count + 1 = version）。
    let extras: [&'static str; 8] = [
        CROSS_OS_CLEANUP_SQL,                    // v11
        V12_PLACEHOLDER,                         // v12（occupied，no-op）
        BACKFILL_OUTBOX_SQL,                     // v13
        APP_ICONS_TABLE_SQL,                     // v14
        APP_GROUPS_SQL,                          // v15
        ADD_CATEGORY_SORT_ORDER_SQL,             // v16
        ADD_PASSWORD_TO_PRIVACY_DEFAULT_SQL,     // v17
        AI_SUMMARIES_TABLE_SQL,                  // v18
    ];
    pool.0
        .call(move |conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER PRIMARY KEY)",
            )
            .db()?;

            let static_count = MIGRATIONS.len();
            let total = static_count + extras.len();
            for idx in 0..total {
                let version = (idx + 1) as i64;
                let already: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM schema_version WHERE version = ?",
                        [version],
                        |r| r.get(0),
                    )
                    .db()?;
                if already > 0 {
                    continue;
                }
                let sql = if idx < static_count {
                    MIGRATIONS[idx]
                } else {
                    extras[idx - static_count]
                };
                if !sql.trim().is_empty() {
                    conn.execute_batch(sql)
                        .db()?;
                }
                conn.execute(
                    "INSERT INTO schema_version VALUES (?)",
                    rusqlite::params![version],
                )
                .db()?;
            }
            Ok(())
        })
        .await?;
    Ok(())
}
