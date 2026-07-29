//! 端到端集成测试：同一个 tokio runtime 里 spin 出多个独立"设备"，共享一个
//! [`InMemoryDriveStore`]，验证 push / pull / tombstone / 幂等等跨设备语义。
//!
//! 每个 [`TestDevice`] 有自己的：
//! - in-memory SQLite DB（独立 connection，互不可见）
//! - device_id（注入到 [`SyncEngine::with_backend`] 的 self_id）
//! - fake auth token（直接 INSERT auth_state 表，绕过 OAuth）
//!
//! 共享：
//! - 一个 [`InMemoryDriveStore`]（模拟 Drive appDataFolder）
//!
//! 跟 Plan B/C 的 `#[cfg(test)] mod tests` 不同 —— 那些测的是 pure 函数；这里测的是
//! "多个 SyncEngine 互相 push/pull 时整体行为正确"。

use std::sync::Arc;

use chrono::{DateTime, Duration, Local, Timelike};

use crate::storage::{migrations, utc_now_rfc3339, DbPool, SqliteResultExt};
use crate::sync::drive::{DriveBackend, InMemoryDriveStore};
use crate::sync::engine::SyncEngine;

struct TestDevice {
    pool: DbPool,
    mem: crate::memory::MemoryDb,
    engine: Arc<SyncEngine>,
    self_id: String,
}

async fn make_device(self_id: &str, drive: Arc<InMemoryDriveStore>) -> TestDevice {
    let pool = DbPool::open_in_memory().await.unwrap();
    migrations::run(&pool).await.unwrap();
    inject_fake_auth(&pool).await;
    // e2e 用内存记忆库:可选数据集默认关,不影响既有用例;开了开关的用例直接用
    let mem = crate::memory::MemoryDb::open_in_memory().await.unwrap();
    let engine = Arc::new(SyncEngine::with_backend(
        pool.clone(),
        Some(mem.clone()),
        DriveBackend::InMemory(drive),
        self_id.to_string(),
    ));
    TestDevice {
        pool,
        mem,
        engine,
        self_id: self_id.to_string(),
    }
}

/// 打开某设备的可选上云三挡(测试用:直接写 settings)。
async fn enable_optional_sync(dev: &TestDevice) {
    let mut cfg = crate::repo::settings::load(&dev.pool).await.unwrap();
    cfg.sync_ai_summaries = true;
    cfg.sync_chat_history = true;
    cfg.sync_screen_memory = true;
    crate::repo::settings::save(&dev.pool, &cfg).await.unwrap();
}

/// INSERT 一行 fake auth_state，让 [`auth::ensure_valid_token`] 走"未过期"分支
/// 直接返回 fake-access-token，绕开 OAuth refresh 网络调用。
///
/// 注意：`read_auth_state` 要求 uid / refresh_token_enc / access_token / expires_at
/// 四列**全部** Some，否则 NotSignedIn 走 push 静默跳过分支。fake refresh_token_enc 用任意非空
/// blob 即可，测试场景永远不会触发 refresh 路径（expires_at 远未来）。
async fn inject_fake_auth(pool: &DbPool) {
    let exp = (chrono::Utc::now() + chrono::Duration::hours(24)).to_rfc3339();
    pool.0
        .call(move |conn| {
            conn.execute(
                "UPDATE auth_state
                 SET uid = 'test-uid', email = 'test@example.com',
                     refresh_token_enc = ?1,
                     access_token = 'fake-access-token', expires_at = ?2
                 WHERE id = 1",
                rusqlite::params![&[0u8; 16][..], exp],
            )
            .db()?;
            Ok(())
        })
        .await
        .unwrap();
}

/// 插一行 sealed activity（origin='local'）+ 入一条对应 outbox，
/// 模拟 capture loop seal 完一个 session 后的 DB 状态。
async fn insert_sealed(
    dev: &TestDevice,
    process: &str,
    started: DateTime<Local>,
    duration_secs: i64,
) -> i64 {
    let self_id = dev.self_id.clone();
    let process = process.to_string();
    let started_str = started.to_rfc3339();
    let ended = started + Duration::seconds(duration_secs);
    let ended_str = ended.to_rfc3339();
    let local_date = started.format("%Y-%m-%d").to_string();
    let local_hour = started.hour() as u8;
    let now = utc_now_rfc3339();
    let local_date_for_outbox = local_date.clone();
    dev.pool
        .0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO activities(
                    started_at, ended_at, duration_secs, local_date, local_hour,
                    process_name, window_title, category_id, device_id, updated_at, origin
                 ) VALUES(?, ?, ?, ?, ?, ?, '', 'other', ?, ?, 'local')",
                rusqlite::params![
                    started_str,
                    ended_str,
                    duration_secs,
                    local_date,
                    local_hour,
                    process,
                    self_id,
                    now,
                ],
            )
            .db()?;
            let id = conn.last_insert_rowid();
            let payload = serde_json::json!({ "localDate": local_date_for_outbox }).to_string();
            conn.execute(
                "INSERT INTO sync_outbox(op, entity, entity_pk, payload, created_at, attempts, next_retry_at)
                 VALUES('upsert', 'activity', ?, ?, ?, 0, ?)",
                rusqlite::params![id.to_string(), payload, now, now],
            )
            .db()?;
            Ok(id)
        })
        .await
        .unwrap()
}

async fn count_for_device(dev: &TestDevice, device_id: &str) -> i64 {
    let device_id = device_id.to_string();
    dev.pool
        .0
        .call(move |conn| {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM activities WHERE device_id = ?1",
                    rusqlite::params![device_id],
                    |r| r.get(0),
                )
                .db()?;
            Ok(n)
        })
        .await
        .unwrap()
}

async fn sum_secs_for_device(dev: &TestDevice, device_id: &str) -> i64 {
    let device_id = device_id.to_string();
    dev.pool
        .0
        .call(move |conn| {
            let n: i64 = conn
                .query_row(
                    "SELECT COALESCE(SUM(duration_secs), 0) FROM activities WHERE device_id = ?1",
                    rusqlite::params![device_id],
                    |r| r.get(0),
                )
                .db()?;
            Ok(n)
        })
        .await
        .unwrap()
}

async fn remote_ids_for(dev: &TestDevice, device_id: &str) -> Vec<String> {
    let device_id = device_id.to_string();
    dev.pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare("SELECT remote_id FROM activities WHERE device_id = ?1 ORDER BY remote_id")
                .db()?;
            let rows = stmt
                .query_map(rusqlite::params![device_id], |r| r.get::<_, String>(0))
                .db()?
                .collect::<rusqlite::Result<Vec<String>>>()
                .db()?;
            Ok(rows)
        })
        .await
        .unwrap()
}

/// 直接清本机 activities + 重置 pull cursor，模拟 commands::storage::purge_activities
/// 的核心 SQL（绕开 Tauri State<>）。
async fn clear_local_and_cursor(dev: &TestDevice) {
    dev.pool
        .0
        .call(|conn| {
            conn.execute_batch(
                "DELETE FROM activities;
                 DELETE FROM sync_outbox;
                 UPDATE sync_cursor SET last_pulled_at = '1970-01-01T00:00:00Z'
                  WHERE entity = 'drive_files';",
            )
            .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
            Ok(())
        })
        .await
        .unwrap();
}

/// Test 1：A push 3 行 → B pull → B 看到 3 行 mirror，self 也保留 3 行；A/B 互不串
#[tokio::test]
async fn cross_device_push_pull_basic() {
    let drive = Arc::new(InMemoryDriveStore::new());
    let a = make_device("device-a", drive.clone()).await;
    let b = make_device("device-b", drive.clone()).await;

    let captured = Local::now();
    insert_sealed(&a, "Code", captured, 30).await;
    insert_sealed(&a, "Chrome", captured, 60).await;
    insert_sealed(&a, "Slack", captured, 45).await;

    a.engine
        .sync_now()
        .await
        .expect("A sync_now should succeed");
    b.engine
        .sync_now()
        .await
        .expect("B sync_now should succeed");

    assert_eq!(
        count_for_device(&b, "device-a").await,
        3,
        "B 应 mirror A 的 3 行"
    );
    assert_eq!(
        sum_secs_for_device(&b, "device-a").await,
        135,
        "B mirror 总秒数应 = 30+60+45"
    );
    // A 自己的行保留
    assert_eq!(count_for_device(&a, "device-a").await, 3);
    // B 没自己的本地行（只有 A 的 mirror）
    assert_eq!(count_for_device(&b, "device-b").await, 0);
    // A 不该 pull 出 B 的（因为 B 没 push）
    assert_eq!(count_for_device(&a, "device-b").await, 0);
}

/// Test 2：A push 5 行 → B pull (mirror 5) → A purge_cloud_data → B sync → B mirror 清空
#[tokio::test]
async fn tombstone_clear_cloud() {
    let drive = Arc::new(InMemoryDriveStore::new());
    let a = make_device("device-a", drive.clone()).await;
    let b = make_device("device-b", drive.clone()).await;

    let captured = Local::now();
    for p in ["Code", "Chrome", "Slack", "Figma", "Terminal"] {
        insert_sealed(&a, p, captured, 30).await;
    }
    a.engine.sync_now().await.unwrap();
    b.engine.sync_now().await.unwrap();
    assert_eq!(count_for_device(&b, "device-a").await, 5);

    // A 调 purge_cloud_data —— 删 Drive 上自己的文件 + 上传 tombstone + 本机 trim
    // keep_local=false 走对称 trim（默认行为，离职/卖机器场景）
    crate::commands::storage::purge_cloud_data_impl(&a.pool, &a.engine, false)
        .await
        .expect("purge_cloud_data");
    // 本机已被 trim
    assert_eq!(count_for_device(&a, "device-a").await, 0);

    // B sync → pull tombstone → trim B 的 A-mirror
    b.engine.sync_now().await.unwrap();
    assert_eq!(
        count_for_device(&b, "device-a").await,
        0,
        "B 的 A-mirror 应被 tombstone 触发 DELETE 干净"
    );
}

/// Test 2b：keep_local=true 路径（换 Google 账号场景）
/// 云端文件全删 + tombstone 上传 + 对端 mirror 清，但本机数据完整保留。
#[tokio::test]
async fn purge_cloud_keep_local_preserves_local_data() {
    let drive = Arc::new(InMemoryDriveStore::new());
    let a = make_device("device-a", drive.clone()).await;
    let b = make_device("device-b", drive.clone()).await;

    let captured = Local::now();
    for p in ["Code", "Chrome", "Slack"] {
        insert_sealed(&a, p, captured, 30).await;
    }
    a.engine.sync_now().await.unwrap();
    b.engine.sync_now().await.unwrap();
    assert_eq!(count_for_device(&b, "device-a").await, 3);

    // keep_local=true：本机数据不动
    crate::commands::storage::purge_cloud_data_impl(&a.pool, &a.engine, true)
        .await
        .expect("purge_cloud_data keep_local");

    // 本机的 3 行原样保留
    assert_eq!(
        count_for_device(&a, "device-a").await,
        3,
        "keep_local=true 时本机数据必须完整保留"
    );

    // B sync → pull tombstone → B 的 A-mirror 仍被清（对端不知道本机要保留）
    b.engine.sync_now().await.unwrap();
    assert_eq!(
        count_for_device(&b, "device-a").await,
        0,
        "对端仍按 tombstone 清 A 的 mirror（云端语义对外一致）"
    );
}

/// Test 3：A push 5 行 → A clear local + cursor → A sync → A 应从 Drive 恢复 5 行
/// （v26 + upsert_remote_activity self 分支：显式 id + origin='local'）
#[tokio::test]
async fn self_restore_after_local_clear() {
    let drive = Arc::new(InMemoryDriveStore::new());
    let a = make_device("device-a", drive.clone()).await;

    let captured = Local::now();
    let mut original_ids: Vec<i64> = Vec::new();
    for p in ["Code", "Chrome", "Slack", "Figma", "Terminal"] {
        original_ids.push(insert_sealed(&a, p, captured, 30).await);
    }
    a.engine.sync_now().await.unwrap();
    assert_eq!(count_for_device(&a, "device-a").await, 5);

    // 模拟「清空本机数据库」
    clear_local_and_cursor(&a).await;
    assert_eq!(count_for_device(&a, "device-a").await, 0);

    // 再 sync → 应从 Drive 拉回自己的 ndjson 恢复
    a.engine.sync_now().await.unwrap();
    assert_eq!(
        count_for_device(&a, "device-a").await,
        5,
        "self-restore 应从 Drive 拉回 5 行"
    );
    // 恢复出来的 remote_id 集合应该跟原始 id 集合一致（v26 保证 local 行 remote_id = id）
    let mut got: Vec<String> = remote_ids_for(&a, "device-a").await;
    let mut want: Vec<String> = original_ids.iter().map(|i| i.to_string()).collect();
    got.sort();
    want.sort();
    assert_eq!(got, want, "恢复后的 remote_id 应与原始 id 一一对应");
}

/// Test 5：flush_pull cursor "longest true prefix" 推进逻辑 ——
/// 中间文件失败时 cursor 应停在前一个成功文件的 modifiedTime，
/// 不能跨过失败文件推到后面成功的（否则下次 pull 永久丢失失败文件）。
#[tokio::test]
async fn flush_pull_cursor_stops_at_failed_file() {
    let drive_store = Arc::new(InMemoryDriveStore::new());
    let dev = make_device("device-self", drive_store.clone()).await;

    // File 1（T1）: meta.json 合法，Pass 1 handles
    let meta_body = serde_json::to_vec(&serde_json::json!({
        "deviceId": "device-d",
        "displayName": "Device D",
        "color": "#abc",
        "icon": "Monitor",
        "updatedAt": "2026-05-15T09:00:00Z",
    }))
    .unwrap();
    drive_store
        .upsert_by_name("device.device-d.meta.json", &meta_body)
        .await
        .unwrap();

    // File 2（T2）: categories.json 内容是坏 JSON，Pass 2 merge_categories 失败
    drive_store
        .upsert_by_name(
            "device.device-d.categories.json",
            b"[ bad JSON, not a valid array",
        )
        .await
        .unwrap();

    // File 3（T3）: app_groups.json 合法（空数组），Pass 2 merge_app_groups 成功
    drive_store
        .upsert_by_name("device.device-d.app_groups.json", b"[]")
        .await
        .unwrap();

    // 三个文件按 modifiedTime 升序排列：T1 < T2 < T3（InMemory 时钟单调）
    let files_before = drive_store.list_appdata_files("").await.unwrap();
    assert_eq!(files_before.len(), 3);
    let t1 = files_before[0].modified_time.clone();

    dev.engine.sync_now().await.unwrap();

    let cursor = super::io::read_cursor(&dev.pool, "drive_files")
        .await
        .unwrap();
    assert_eq!(
        cursor, t1,
        "cursor 应停在 T1（T2 失败后不能跨过），实际: {cursor:?}, 期望: {t1:?}"
    );
}

/// Test 4：跑完 Test 1 的 setup 后连续 sync_now 多次，两端 DB 行 hash 不变。
/// 钉死"不重复 INSERT、不重复 DELETE、cursor 不抖"。
#[tokio::test]
async fn idempotent_repeated_sync() {
    let drive = Arc::new(InMemoryDriveStore::new());
    let a = make_device("device-a", drive.clone()).await;
    let b = make_device("device-b", drive.clone()).await;

    let captured = Local::now();
    insert_sealed(&a, "Code", captured, 30).await;
    insert_sealed(&a, "Chrome", captured, 60).await;
    insert_sealed(&a, "Slack", captured, 45).await;
    a.engine.sync_now().await.unwrap();
    b.engine.sync_now().await.unwrap();

    let baseline_a = count_for_device(&a, "device-a").await;
    let baseline_a_sum = sum_secs_for_device(&a, "device-a").await;
    let baseline_b = count_for_device(&b, "device-a").await;
    let baseline_b_sum = sum_secs_for_device(&b, "device-a").await;

    for _ in 0..3 {
        a.engine.sync_now().await.unwrap();
        b.engine.sync_now().await.unwrap();
    }

    assert_eq!(count_for_device(&a, "device-a").await, baseline_a);
    assert_eq!(sum_secs_for_device(&a, "device-a").await, baseline_a_sum);
    assert_eq!(count_for_device(&b, "device-a").await, baseline_b);
    assert_eq!(sum_secs_for_device(&b, "device-a").await, baseline_b_sum);
}

/// 可选上云三数据集的双设备闭环:
/// A 产生 聊天会话+消息 / 屏幕记忆会话 / AI 日报 → sync → B 全部可见;
/// A 删会话(软删墓碑)→ sync → B 的会话消失、消息清空;
/// A 会话追加文本(ended_ts 推进)→ sync → B 侧文本更新(LWW)。
#[tokio::test]
async fn optional_datasets_cross_device_roundtrip() {
    let drive = Arc::new(InMemoryDriveStore::default());
    let a = make_device("device-a", Arc::clone(&drive)).await;
    let b = make_device("device-b", Arc::clone(&drive)).await;
    enable_optional_sync(&a).await;
    enable_optional_sync(&b).await;

    // — A: 聊天一问一答 —
    let conv = crate::chat::store::create_conversation(&a.mem, "测试会话")
        .await
        .unwrap();
    crate::chat::store::append_user(&a.mem, conv, "上周看了什么?", None)
        .await
        .unwrap();
    crate::chat::store::append_assistant(
        &a.mem,
        conv,
        "看了三个视频 [1]",
        &[],
        false,
        (200, 80),
        None,
    )
    .await
    .unwrap();

    // — A: 一条屏幕记忆会话 —
    a.mem
        .0
        .call(|conn| {
            conn.execute(
                "INSERT INTO text_sessions(local_date, started_ts, ended_ts, app_id, title, text, guid)
                 VALUES ('2026-07-05','t0','t1','code','标题甲','秘密订单编号八八四二',
                         lower(hex(randomblob(16))))",
                [],
            )
            .db()?;
            Ok(())
        })
        .await
        .unwrap();

    // — A: 一段日报 —
    a.pool
        .0
        .call(|conn| {
            conn.execute(
                "INSERT INTO ai_summaries(source, local_date, segment_idx, label, start_hour,
                                          end_hour, content, model, status, error, generated_at)
                 VALUES ('daily','2026-07-05',0,'深夜',0,6,'凌晨在写代码','m','ok',NULL,
                         '2026-07-05T10:00:00Z')",
                [],
            )
            .db()?;
            Ok(())
        })
        .await
        .unwrap();

    a.engine.sync_now().await.unwrap();
    b.engine.sync_now().await.unwrap();

    // B: 聊天可见
    let convs = crate::chat::store::list_conversations(&b.mem)
        .await
        .unwrap();
    assert_eq!(convs.len(), 1, "B 应看到 A 的会话");
    assert_eq!(convs[0].title, "测试会话");
    let msgs = crate::chat::store::get_messages(&b.mem, convs[0].id)
        .await
        .unwrap();
    assert_eq!(msgs.len(), 2, "两条消息都应到位");
    // B: 屏幕记忆可搜(FTS 触发器在 INSERT 时生效)+ 标了来源设备
    let (hits, origin): (i64, String) = b
        .mem
        .0
        .call(|conn| {
            let hits: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM text_sessions_fts WHERE text_sessions_fts MATCH '八八四二'",
                    [],
                    |r| r.get(0),
                )
                .db()?;
            let origin: String = conn
                .query_row(
                    "SELECT origin_device FROM text_sessions LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .db()?;
            Ok((hits, origin))
        })
        .await
        .unwrap();
    assert_eq!(hits, 1, "B 的 FTS 应能搜到 A 的屏幕文字");
    assert_eq!(origin, "device-a");
    // B: 日报可见
    let n: i64 = b
        .pool
        .0
        .call(|conn| {
            conn.query_row("SELECT COUNT(*) FROM ai_summaries", [], |r| r.get(0))
                .db()
        })
        .await
        .unwrap();
    assert_eq!(n, 1, "B 应看到 A 的日报行");

    // — A 删会话 → 墓碑传播 —
    crate::chat::store::delete_conversation(&a.mem, conv)
        .await
        .unwrap();
    a.engine.sync_now().await.unwrap();
    b.engine.sync_now().await.unwrap();
    let convs = crate::chat::store::list_conversations(&b.mem)
        .await
        .unwrap();
    assert!(convs.is_empty(), "删除应传播到 B");
    let msg_left: i64 = b
        .mem
        .0
        .call(|conn| {
            conn.query_row("SELECT COUNT(*) FROM chat_messages", [], |r| r.get(0))
                .db()
        })
        .await
        .unwrap();
    assert_eq!(msg_left, 0, "墓碑落地应清掉 B 的消息");

    // — A 的记忆会话增长(text/ended_ts 更新)→ B 侧 LWW 覆盖 —
    a.mem
        .0
        .call(|conn| {
            conn.execute(
                "UPDATE text_sessions SET text = text || ' 新增行', ended_ts = 't2'",
                [],
            )
            .db()?;
            Ok(())
        })
        .await
        .unwrap();
    a.engine.sync_now().await.unwrap();
    b.engine.sync_now().await.unwrap();
    let text: String = b
        .mem
        .0
        .call(|conn| {
            conn.query_row("SELECT text FROM text_sessions LIMIT 1", [], |r| r.get(0))
                .db()
        })
        .await
        .unwrap();
    assert!(text.contains("新增行"), "会话增长应覆盖到 B: {text}");
}

// ───────────────────── 补测 C 批新增(push 失败重试 / metadata 往返 / OS 过滤) ─────────────────────

/// RAII：把 `HINDSIGHT_DATA_DIR` 指到唯一临时目录，drop 时恢复原值。
/// 为什么需要：merge_app_icons 应用成功后会把 icon 字节写进
/// `<data_root>/icons/` 文件 cache —— 不隔离的话测试会污染真实用户数据目录。
/// 构造前必须先持有 `test_util::lock_data_dir_env()`（进程级 env 串行锁）。
struct DataDirOverride {
    prev: Option<String>,
}

impl DataDirOverride {
    fn unique_temp() -> Self {
        let dir = std::env::temp_dir().join(format!("hindsight-sync-e2e-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let prev = std::env::var("HINDSIGHT_DATA_DIR").ok();
        std::env::set_var("HINDSIGHT_DATA_DIR", &dir);
        Self { prev }
    }
}

impl Drop for DataDirOverride {
    fn drop(&mut self) {
        match self.prev.take() {
            Some(v) => std::env::set_var("HINDSIGHT_DATA_DIR", v),
            None => std::env::remove_var("HINDSIGHT_DATA_DIR"),
        }
    }
}

async fn outbox_count(pool: &DbPool) -> i64 {
    pool.0
        .call(|conn| {
            conn.query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0))
                .db()
        })
        .await
        .unwrap()
}

/// 直接 INSERT 一条非 activity 的 outbox 行（payload 对这些 entity 无用，给 "{}"）。
async fn enqueue_entity(dev: &TestDevice, entity: &str, pk: &str) {
    let entity = entity.to_string();
    let pk = pk.to_string();
    dev.pool
        .0
        .call(move |conn| {
            let now = utc_now_rfc3339();
            conn.execute(
                "INSERT INTO sync_outbox(op, entity, entity_pk, payload, created_at, attempts, next_retry_at)
                 VALUES('upsert', ?1, ?2, '{}', ?3, 0, ?3)",
                rusqlite::params![entity, pk, now],
            )
            .db()?;
            Ok(())
        })
        .await
        .unwrap();
}

/// 任务 1：push 失败重试不变量。
/// 注入一次 upsert 失败 → sync_now 必须返回 SyncIncomplete、失败行留在 outbox
/// （attempts=1、last_error 已记录、next_retry_at 推到未来）、status.last_error
/// 带 [TRANSIENT] 前缀、Drive 上不能出现半写文件；解除注入再 sync → 数据完整
/// 落 Drive、outbox 清空、last_error 清除。
#[tokio::test]
async fn push_transient_failure_keeps_outbox_then_recovers() {
    let drive = Arc::new(InMemoryDriveStore::new());
    let a = make_device("device-a", drive.clone()).await;

    let captured = Local::now();
    let day = captured.format("%Y-%m-%d").to_string();
    insert_sealed(&a, "Code", captured, 30).await;

    drive.fail_next_upserts(1);
    let failed_at = utc_now_rfc3339();
    let err = a
        .engine
        .sync_now()
        .await
        .expect_err("注入 upsert 失败时 sync_now 应报错");
    assert!(
        matches!(err, crate::error::Error::SyncIncomplete(_)),
        "应归类为 SyncIncomplete（outbox 尚有滞留行），实际: {err:?}"
    );

    // Drive 上不能出现任何文件：唯一一次 upsert 已被注入打断
    assert!(
        drive.list_appdata_files("").await.unwrap().is_empty(),
        "失败的 push 不应在 Drive 留下半写文件"
    );

    // 失败行仍在 outbox：attempts 恰好 +1，last_error 记录了原始错误，
    // next_retry_at 被指数退避推到失败时刻之后（不会下个瞬间就重试）
    let (attempts, last_error, next_retry_at): (i64, Option<String>, String) = a
        .pool
        .0
        .call(|conn| {
            conn.query_row(
                "SELECT attempts, last_error, next_retry_at FROM sync_outbox",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .db()
        })
        .await
        .unwrap();
    assert_eq!(attempts, 1, "一次失败后 attempts 应恰好为 1");
    let le = last_error.expect("失败行应记录 last_error");
    assert!(
        le.contains("injected transient failure"),
        "outbox.last_error 应含注入的原始错误文本: {le}"
    );
    assert!(
        next_retry_at.as_str() > failed_at.as_str(),
        "next_retry_at 应被退避推到失败时刻之后: {next_retry_at} <= {failed_at}"
    );

    // 引擎状态：错误分类为 [TRANSIENT]（500 属"等下个 tick 重试"而非重新登录）
    let status = a.engine.status().await;
    let status_err = status.last_error.expect("status 应带 last_error");
    assert!(
        status_err.starts_with("[TRANSIENT] "),
        "Drive 500 应归类 [TRANSIENT]，实际: {status_err}"
    );
    assert_eq!(status.pending, 1, "pending 应显示 1 行待推");

    // 解除注入（配额已在失败时耗尽,这里只需把退避时间拨回,模拟"到点重试"）
    a.pool
        .0
        .call(|conn| {
            conn.execute(
                "UPDATE sync_outbox SET next_retry_at = '1970-01-01T00:00:00+00:00'",
                [],
            )
            .db()?;
            Ok(())
        })
        .await
        .unwrap();
    a.engine.sync_now().await.expect("解除注入后 sync 应成功");

    // outbox 清空 + last_error 清除
    assert_eq!(outbox_count(&a.pool).await, 0, "重试成功后 outbox 应清空");
    let status = a.engine.status().await;
    assert!(
        status.last_error.is_none(),
        "重试成功后 last_error 应清空: {:?}",
        status.last_error
    );

    // 数据完整落 Drive：ndjson 文件存在且内容就是那一行 activity
    let files = drive.list_appdata_files("").await.unwrap();
    let ndjson = files
        .iter()
        .find(|f| f.name == format!("device.device-a.activities.{day}.ndjson"))
        .expect("Drive 上应出现当天的 activities ndjson");
    let body = drive.download(&ndjson.id).await.unwrap();
    let lines: Vec<&str> = std::str::from_utf8(&body)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    assert_eq!(lines.len(), 1, "文件应恰含 1 行 activity");
    let row: crate::sync::payload::ActivityPayload = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(row.process_name, "Code");
    assert_eq!(row.duration_secs, 30);
}

/// 任务 3：7 类 metadata entity 的双设备 roundtrip。
/// A 端各表插一行 + 手工入 outbox（category / app_category / process_path /
/// device / app_icon / app_group / app_group_member 全覆盖）→ A sync 推 7 个
/// 文件 → B sync 拉回 → B 各表字段逐一与 A 写入值相等。
/// 一条测试同时吃掉 push 构建侧 7 个 build_* 与 pull 合并侧对应 merge_*。
// env 锁横跨整个测试(B merge app_icon 会写 icon 文件 cache,路径读
// HINDSIGHT_DATA_DIR);#[tokio::test] 是单线程 runtime,持锁跨 await 不自死锁。
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn metadata_seven_entities_cross_device_roundtrip() {
    let _env_lock = crate::repo::test_util::lock_data_dir_env();
    let _data_dir = DataDirOverride::unique_temp();

    let drive = Arc::new(InMemoryDriveStore::new());
    let a = make_device("device-a", drive.clone()).await;
    let b = make_device("device-b", drive.clone()).await;

    // 各表的期望值:时间戳独立、互不相同,B 端逐字段核对
    const T_CAT: &str = "2026-07-01T00:00:01Z";
    const T_APPCAT: &str = "2026-07-01T00:00:02Z";
    const T_PATH: &str = "2026-07-01T00:00:03Z";
    const T_SEEN: &str = "2026-06-30T09:00:00Z";
    const T_DEV_SEEN: &str = "2026-07-01T00:00:04Z";
    const T_DEV_UPD: &str = "2026-07-01T00:00:05Z";
    const T_ICON: &str = "2026-07-01T00:00:06Z";
    const T_GRP: &str = "2026-07-01T00:00:07Z";
    const T_MEMBER: &str = "2026-07-01T00:00:08Z";
    let icon_bytes: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 1, 2, 3, 4];

    // A 本机 OS —— meta 先到 B 才能解锁 app_categories / process_paths 的 OS 过滤
    let os = crate::platform::local_os_id().to_string();

    let os_ins = os.clone();
    let icon_ins = icon_bytes.clone();
    a.pool
        .0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO categories(id, name, color, icon, builtin, sort_order, updated_at, deleted_at)
                 VALUES('cat-e2e', 'E2E 分类', '#abcdef', 'Star', 0, 7, ?1, NULL)",
                rusqlite::params![T_CAT],
            )
            .db()?;
            conn.execute(
                "INSERT INTO app_categories(process_name, category_id, updated_at, deleted_at)
                 VALUES('Proc-E2E', 'cat-e2e', ?1, NULL)",
                rusqlite::params![T_APPCAT],
            )
            .db()?;
            conn.execute(
                "INSERT INTO process_paths(process_name, exe_path, seen_at, updated_at)
                 VALUES('Proc-E2E', '/Applications/ProcE2E.app', ?1, ?2)",
                rusqlite::params![T_SEEN, T_PATH],
            )
            .db()?;
            conn.execute(
                "INSERT INTO devices(device_id, display_name, color, icon, os, last_seen_at, is_self, updated_at, deleted_at)
                 VALUES('device-a', 'A 机', '#112233', 'Laptop', ?1, ?2, 1, ?3, NULL)",
                rusqlite::params![os_ins, T_DEV_SEEN, T_DEV_UPD],
            )
            .db()?;
            conn.execute(
                "INSERT INTO app_icons(process_name, icon_png, updated_at, deleted_at)
                 VALUES('Proc-E2E', ?1, ?2, NULL)",
                rusqlite::params![icon_ins, T_ICON],
            )
            .db()?;
            // 组的 category 故意留 NULL:避免 merge_app_groups 的成员 mirror 在
            // 本测试里被触发(mirror 会用"当下时间"改写 app_categories.updated_at,
            // 而文件落盘顺序是 HashMap 随机序,字段级断言会变得不确定)。
            // mirror 行为由 pull.rs 的直测单独钉死。
            conn.execute(
                "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                 VALUES('grp-e2e', 'Proc E2E 组', NULL, ?1, NULL)",
                rusqlite::params![T_GRP],
            )
            .db()?;
            conn.execute(
                "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                 VALUES('Proc-E2E', 'grp-e2e', ?1, NULL)",
                rusqlite::params![T_MEMBER],
            )
            .db()?;
            Ok(())
        })
        .await
        .unwrap();

    // 分两轮推:被 FK 引用的表(categories / app_groups)先落 Drive,引用方
    // (app_categories / app_group_members)后落。B 按 modifiedTime 升序合并,
    // 引用目标必然先到位。若同轮乱序推(push 的 HashMap 随机序),引用方文件
    // 可能先被合并,行级 FOREIGN KEY 失败仅 warn 且游标照常越过 —— 该缺陷已
    // 记录为产品 bug,这里不让测试依赖随机顺序。
    for (entity, pk) in [
        ("category", "cat-e2e"),
        ("process_path", "Proc-E2E"),
        ("device", "device-a"),
        ("app_icon", "Proc-E2E"),
        ("app_group", "grp-e2e"),
    ] {
        enqueue_entity(&a, entity, pk).await;
    }
    a.engine.sync_now().await.expect("A 第一轮 sync 应成功");
    for (entity, pk) in [
        ("app_category", "Proc-E2E"),
        ("app_group_member", "Proc-E2E"),
    ] {
        enqueue_entity(&a, entity, pk).await;
    }
    a.engine.sync_now().await.expect("A 第二轮 sync 应成功");
    assert_eq!(outbox_count(&a.pool).await, 0, "A 推完 outbox 应清空");
    assert_eq!(
        drive.list_appdata_files("").await.unwrap().len(),
        7,
        "7 类 entity 应各产出一个 Drive 文件"
    );

    b.engine.sync_now().await.expect("B sync 应成功");

    // ── B 侧逐表逐字段核对(值先取出闭包,断言在外面做,panic 不打穿 DB 线程) ──
    type CatRow = (String, String, String, i64, i64, String, Option<String>);
    type DevRow = (
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        i64,
        String,
        Option<String>,
    );
    #[allow(clippy::type_complexity)]
    let (cat, ac, pp, dev, icon, grp, member): (
        CatRow,
        (String, String, Option<String>),
        (String, String, String),
        DevRow,
        (Vec<u8>, String, Option<String>),
        (String, Option<String>, String, Option<String>),
        (String, String, Option<String>),
    ) = b
        .pool
        .0
        .call(move |conn| {
            let cat = conn
                .query_row(
                    "SELECT name, color, icon, builtin, sort_order, updated_at, deleted_at
                     FROM categories WHERE id = 'cat-e2e'",
                    [],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                        ))
                    },
                )
                .db()?;
            let ac = conn
                .query_row(
                    "SELECT category_id, updated_at, deleted_at
                     FROM app_categories WHERE process_name = 'Proc-E2E'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .db()?;
            let pp = conn
                .query_row(
                    "SELECT exe_path, seen_at, updated_at
                     FROM process_paths WHERE process_name = 'Proc-E2E'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .db()?;
            let dev = conn
                .query_row(
                    "SELECT display_name, color, icon, os, last_seen_at, is_self, updated_at, deleted_at
                     FROM devices WHERE device_id = 'device-a'",
                    [],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                            r.get(7)?,
                        ))
                    },
                )
                .db()?;
            let icon = conn
                .query_row(
                    "SELECT icon_png, updated_at, deleted_at
                     FROM app_icons WHERE process_name = 'Proc-E2E'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .db()?;
            let grp = conn
                .query_row(
                    "SELECT display_name, category_id, updated_at, deleted_at
                     FROM app_groups WHERE id = 'grp-e2e'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .db()?;
            let member = conn
                .query_row(
                    "SELECT group_id, updated_at, deleted_at
                     FROM app_group_members WHERE process_name = 'Proc-E2E'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .db()?;
            Ok((cat, ac, pp, dev, icon, grp, member))
        })
        .await
        .unwrap();

    assert_eq!(
        cat,
        (
            "E2E 分类".into(),
            "#abcdef".into(),
            "Star".into(),
            0,
            7,
            T_CAT.into(),
            None
        ),
        "categories 行应逐字段等于 A 写入值"
    );
    assert_eq!(
        ac,
        ("cat-e2e".into(), T_APPCAT.into(), None),
        "app_categories 行应逐字段一致(pass 1 先合并 meta 解锁同 OS 过滤)"
    );
    assert_eq!(
        pp,
        (
            "/Applications/ProcE2E.app".into(),
            T_SEEN.into(),
            T_PATH.into()
        ),
        "process_paths 行应逐字段一致"
    );
    assert_eq!(
        dev,
        (
            "A 机".into(),
            "#112233".into(),
            "Laptop".into(),
            Some(os.clone()),
            Some(T_DEV_SEEN.into()),
            0,
            T_DEV_UPD.into(),
            None
        ),
        "devices 行应逐字段一致(尤其 os;is_self 在 B 侧为 0)"
    );
    assert_eq!(
        icon,
        (icon_bytes.clone(), T_ICON.into(), None),
        "app_icons 字节与时间戳应 base64 往返无损"
    );
    assert_eq!(
        grp,
        ("Proc E2E 组".into(), None, T_GRP.into(), None),
        "app_groups 行应逐字段一致"
    );
    assert_eq!(
        member,
        ("grp-e2e".into(), T_MEMBER.into(), None),
        "app_group_members 行应逐字段一致"
    );

    // pull 不回灌:B 侧合并远端数据不应产生任何 outbox 行(否则会推回死循环)
    assert_eq!(outbox_count(&b.pool).await, 0, "B pull 后 outbox 应仍为空");
}

/// 任务 7：OS 过滤 + 游标 stall 三段式。
/// ① 只有 device-x 的 app_categories 文件、meta 未到 → OS 未知,游标不越过、表不写;
/// ② 补传同 OS meta → 下轮两个文件都合并,游标推进到 meta 的 modifiedTime;
/// ③ 异 OS 设备 device-y 的 meta + app_categories → 数据文件标 handled 跳过
///    (游标越过),之后永不再合并。
#[tokio::test]
async fn cross_os_filter_stalls_until_meta_then_skips_foreign_os() {
    let drive_store = Arc::new(InMemoryDriveStore::new());
    let dev = make_device("device-self", drive_store.clone()).await;
    let local_os = crate::platform::local_os_id();

    let app_cat_body = |process: &str| {
        serde_json::to_vec(&vec![crate::sync::payload::AppCategoryPayload {
            process_name: process.to_string(),
            category_id: "code".into(),
            updated_at: "2026-07-01T00:00:00Z".into(),
            deleted_at: None,
        }])
        .unwrap()
    };
    let meta_body = |device: &str, os: &str| {
        serde_json::to_vec(&crate::sync::payload::DeviceMetaPayload {
            device_id: device.to_string(),
            display_name: device.to_string(),
            color: "#abc".into(),
            icon: "Monitor".into(),
            os: Some(os.to_string()),
            last_seen_at: None,
            updated_at: "2026-07-01T00:00:00Z".into(),
        })
        .unwrap()
    };
    let has_app_cat = |process: &'static str| {
        let pool = dev.pool.clone();
        async move {
            pool.0
                .call(move |conn| {
                    let n: i64 = conn
                        .query_row(
                            "SELECT COUNT(*) FROM app_categories WHERE process_name = ?1",
                            rusqlite::params![process],
                            |r| r.get(0),
                        )
                        .db()?;
                    Ok(n > 0)
                })
                .await
                .unwrap()
        }
    };

    // ① 数据文件先到,meta 缺席
    drive_store
        .upsert_by_name(
            "device.device-x.app_categories.json",
            &app_cat_body("X-App"),
        )
        .await
        .unwrap();
    dev.engine.sync_now().await.unwrap();
    assert!(
        !has_app_cat("X-App").await,
        "OS 未知时 app_categories 不应合并"
    );
    assert_eq!(
        super::io::read_cursor(&dev.pool, "drive_files")
            .await
            .unwrap(),
        "1970-01-01T00:00:00Z",
        "游标不能越过 OS 未知的数据文件(否则该文件永久丢失)"
    );

    // ② 补传本机同 OS 的 meta → 两个文件都应被处理
    drive_store
        .upsert_by_name(
            "device.device-x.meta.json",
            &meta_body("device-x", local_os),
        )
        .await
        .unwrap();
    let files = drive_store.list_appdata_files("").await.unwrap();
    assert_eq!(files.len(), 2);
    let t_meta_x = files[1].modified_time.clone(); // 升序:appcat(T1) < meta(T2)
    dev.engine.sync_now().await.unwrap();
    assert!(
        has_app_cat("X-App").await,
        "meta 到位且同 OS 后,数据文件应被合并"
    );
    assert_eq!(
        super::io::read_cursor(&dev.pool, "drive_files")
            .await
            .unwrap(),
        t_meta_x,
        "两个文件全 handled 后游标应推进到末尾(meta 的 modifiedTime)"
    );

    // ③ 异 OS 设备:meta 先到(T3)、数据文件后到(T4)
    drive_store
        .upsert_by_name(
            "device.device-y.meta.json",
            &meta_body("device-y", "alien-os"),
        )
        .await
        .unwrap();
    drive_store
        .upsert_by_name(
            "device.device-y.app_categories.json",
            &app_cat_body("Y-App"),
        )
        .await
        .unwrap();
    let files = drive_store.list_appdata_files("").await.unwrap();
    let t_appcat_y = files[3].modified_time.clone();
    dev.engine.sync_now().await.unwrap();
    assert!(
        !has_app_cat("Y-App").await,
        "异 OS 的 app_categories 永不合并(key 体系不同,合并有害)"
    );
    assert_eq!(
        super::io::read_cursor(&dev.pool, "drive_files")
            .await
            .unwrap(),
        t_appcat_y,
        "异 OS 数据文件应标 handled 让游标越过(不是 stall)"
    );

    // 再 sync 一轮:游标已越过,该文件不再入列,结论不变
    dev.engine.sync_now().await.unwrap();
    assert!(
        !has_app_cat("Y-App").await,
        "游标越过后异 OS 文件永不再被拉取合并"
    );
}

/// 回归:同一轮 pull 内,引用方文件(app_categories / app_group_members)的
/// modifiedTime 早于被引用文件(categories / app_groups)时,行不能丢。
///
/// 这正是 push HashMap 随机序下约 50% 概率触发的实锤 bug:修复前 Pass 2 按
/// modifiedTime 升序直走,引用方先合并 → 行级 FK 失败仅 warn → handled=true
/// 让游标越过 → 该行永久丢失(下轮 `modifiedTime >` 不再拉它)。
/// 修复后 Pass 2 用依赖序(categories / app_groups 先行)遍历,单轮全部落库。
/// 游标推进语义不受遍历顺序影响(handled[] 仍按文件列表原序求最长 true 前缀)。
#[tokio::test]
async fn pull_single_round_merges_children_even_when_files_precede_parents() {
    use rusqlite::OptionalExtension;

    let drive = Arc::new(InMemoryDriveStore::new());
    let dev = make_device("device-self", drive.clone()).await;

    // T1: 远端设备 meta,os = 本机 → 解锁 app_categories 的跨 OS 过滤
    let meta = serde_json::to_vec(&serde_json::json!({
        "deviceId": "device-x",
        "displayName": "Device X",
        "color": "#abc",
        "icon": "Monitor",
        "os": crate::platform::local_os_id(),
        "lastSeenAt": "2026-05-15T09:00:00Z",
        "updatedAt": "2026-05-15T09:00:00Z",
    }))
    .unwrap();
    drive
        .upsert_by_name("device.device-x.meta.json", &meta)
        .await
        .unwrap();

    // T2 / T3: 引用方文件先落 Drive(modifiedTime 更早)
    drive
        .upsert_by_name(
            "device.device-x.app_categories.json",
            serde_json::to_vec(&serde_json::json!([{
                "processName": "ProcCat-FK",
                "categoryId": "cat-fk",
                "updatedAt": "2026-05-15T09:00:01Z",
                "deletedAt": null,
            }]))
            .unwrap()
            .as_slice(),
        )
        .await
        .unwrap();
    drive
        .upsert_by_name(
            "device.device-x.app_group_members.json",
            serde_json::to_vec(&serde_json::json!([{
                "processName": "ProcGrp-FK",
                "groupId": "grp-fk",
                "updatedAt": "2026-05-15T09:00:02Z",
                "deletedAt": null,
            }]))
            .unwrap()
            .as_slice(),
        )
        .await
        .unwrap();

    // T4 / T5: 被引用文件后落 Drive
    drive
        .upsert_by_name(
            "device.device-x.categories.json",
            serde_json::to_vec(&serde_json::json!([{
                "id": "cat-fk",
                "name": "FK 分类",
                "color": "#123456",
                "icon": "Star",
                "builtin": false,
                "sortOrder": 42,
                "updatedAt": "2026-05-15T09:00:03Z",
                "deletedAt": null,
            }]))
            .unwrap()
            .as_slice(),
        )
        .await
        .unwrap();
    drive
        .upsert_by_name(
            "device.device-x.app_groups.json",
            serde_json::to_vec(&serde_json::json!([{
                "id": "grp-fk",
                "displayName": "FK 组",
                "categoryId": null,
                "updatedAt": "2026-05-15T09:00:04Z",
                "deletedAt": null,
            }]))
            .unwrap()
            .as_slice(),
        )
        .await
        .unwrap();

    // 前置自检:列表按 modifiedTime 升序,引用方确实排在被引用方前面
    let files = drive.list_appdata_files("").await.unwrap();
    let pos = |name: &str| {
        files
            .iter()
            .position(|f| f.name == name)
            .unwrap_or_else(|| panic!("Drive 应有 {name} 文件"))
    };
    assert!(
        pos("device.device-x.app_categories.json") < pos("device.device-x.categories.json")
            && pos("device.device-x.app_group_members.json")
                < pos("device.device-x.app_groups.json"),
        "前置条件:引用方文件的 modifiedTime 必须早于被引用文件"
    );

    dev.engine.sync_now().await.expect("sync 应成功");

    // 单轮之后四行必须全部落库(修复前两条引用方行被 FK 吃掉且永不再拉)
    let (cat_n, ac_cat, grp_n, member_grp): (i64, Option<String>, i64, Option<String>) = dev
        .pool
        .0
        .call(|conn| {
            let cat_n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM categories WHERE id = 'cat-fk' AND deleted_at IS NULL",
                    [],
                    |r| r.get(0),
                )
                .db()?;
            let ac_cat: Option<String> = conn
                .query_row(
                    "SELECT category_id FROM app_categories
                     WHERE process_name = 'ProcCat-FK' AND deleted_at IS NULL",
                    [],
                    |r| r.get(0),
                )
                .optional()
                .db()?;
            let grp_n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM app_groups WHERE id = 'grp-fk' AND deleted_at IS NULL",
                    [],
                    |r| r.get(0),
                )
                .db()?;
            let member_grp: Option<String> = conn
                .query_row(
                    "SELECT group_id FROM app_group_members
                     WHERE process_name = 'ProcGrp-FK' AND deleted_at IS NULL",
                    [],
                    |r| r.get(0),
                )
                .optional()
                .db()?;
            Ok((cat_n, ac_cat, grp_n, member_grp))
        })
        .await
        .unwrap();

    assert_eq!(cat_n, 1, "categories 行应落库");
    assert_eq!(grp_n, 1, "app_groups 行应落库");
    assert_eq!(
        ac_cat.as_deref(),
        Some("cat-fk"),
        "app_categories 行不能因文件序早于 categories 而丢失"
    );
    assert_eq!(
        member_grp.as_deref(),
        Some("grp-fk"),
        "app_group_members 行不能因文件序早于 app_groups 而丢失"
    );
}
