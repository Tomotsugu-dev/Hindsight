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
    crate::chat::store::append_user(&a.mem, conv, "上周看了什么?")
        .await
        .unwrap();
    crate::chat::store::append_assistant(&a.mem, conv, "看了三个视频 [1]", &[], false)
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
