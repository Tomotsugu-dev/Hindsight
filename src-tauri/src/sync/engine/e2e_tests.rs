//! 端到端集成测试：同一个 tokio runtime 里 spin 出多个独立"设备"，共享一个
//! [`InMemoryDriveStore`]，验证 push / pull / tombstone / 幂等等跨设备语义。
//!
//! 每个 [`TestDevice`] 有自己的：
//! - in-memory SQLite DB（独立 connection，互不可见）
//! - device_id（注入到 [`SyncEngine::with_backend`] 的 self_id）
//! - fake auth token（直接 INSERT auth_state 表，绕过 OAuth）
//!
//! 共享：
//! - 一个 [`InMemoryDriveStore`]（模拟 Drive appDataFolder）；文件末尾 WebDAV 一节
//!   换成一个假 WebDAV 服务器 [`FakeDav`]
//!
//! 跟 Plan B/C 的 `#[cfg(test)] mod tests` 不同 —— 那些测的是 pure 函数；这里测的是
//! "多个 SyncEngine 互相 push/pull 时整体行为正确"。

use std::sync::Arc;

use chrono::{DateTime, Duration, Local, Timelike};

use crate::repo::test_util::DataDirOverride;
use crate::storage::{migrations, utc_now_rfc3339, DbPool, SqliteResultExt};
use crate::sync::cloud::CloudBackend;
use crate::sync::drive::InMemoryDriveStore;
use crate::sync::engine::SyncEngine;
use crate::sync::webdav::{Call, FakeDav, WebDavClient};

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
        CloudBackend::InMemory(drive),
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

/// 只打开 AI 总结这一挡（测试用:直接写 settings）。
async fn enable_ai_summaries_sync(dev: &TestDevice) {
    let mut cfg = crate::repo::settings::load(&dev.pool).await.unwrap();
    cfg.sync_ai_summaries = true;
    crate::repo::settings::save(&dev.pool, &cfg).await.unwrap();
}

async fn ai_summary_count(dev: &TestDevice) -> i64 {
    dev.pool
        .0
        .call(|conn| {
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM ai_summaries", [], |r| r.get(0))
                .db()?;
            Ok(n)
        })
        .await
        .unwrap()
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

async fn signed_in(dev: &TestDevice) -> bool {
    crate::sync::drive::auth::current_state(&dev.pool)
        .await
        .unwrap()
        .signed_in
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
// 一并清空会删 <数据目录> 下的 icons 和 screenshots，所以整条测试持 env 锁、指到临时目录。
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn tombstone_clear_cloud() {
    let _env_lock = crate::repo::test_util::lock_data_dir_env();
    let _data_dir = DataDirOverride::unique_temp();
    let _digest = crate::memory::digest::drain_lock().await;

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

    // 截图目录里放一张，验证「一并清空」连截图一起删
    let shots_dir = crate::storage::db_path_dir().unwrap().join("screenshots");
    std::fs::create_dir_all(&shots_dir).unwrap();
    let shot = shots_dir.join("a.png");
    std::fs::write(&shot, b"png").unwrap();

    // A 调 purge_cloud_data —— 删 Drive 上自己的文件 + 上传 tombstone + 本机一并清空
    crate::commands::storage::purge_cloud_data_impl(&a.pool, &a.engine, None, Some(&a.mem), false)
        .await
        .expect("purge_cloud_data");
    assert_eq!(count_for_device(&a, "device-a").await, 0);
    assert!(!shot.exists(), "一并清空应删掉截图");
    assert!(!signed_in(&a).await, "移除本设备后应已退出登录");

    // B sync → pull tombstone → trim B 的 A-mirror
    b.engine.sync_now().await.unwrap();
    assert_eq!(
        count_for_device(&b, "device-a").await,
        0,
        "B 的 A-mirror 应被 tombstone 触发 DELETE 干净"
    );
}

/// 没登录时「从云端移除本设备」直接报错，本机什么都不动。
#[tokio::test]
async fn remove_device_requires_sign_in() {
    let drive = Arc::new(InMemoryDriveStore::new());
    let a = make_device("device-a", drive.clone()).await;
    let captured = Local::now();
    for p in ["Code", "Chrome", "Slack"] {
        insert_sealed(&a, p, captured, 30).await;
    }
    crate::sync::drive::auth::sign_out(&a.pool).await.unwrap();
    drive.sign_out();

    let res = crate::commands::storage::purge_cloud_data_impl(
        &a.pool,
        &a.engine,
        None,
        Some(&a.mem),
        false,
    )
    .await;
    assert!(res.is_err(), "没登录时应返回错误");
    assert_eq!(
        count_for_device(&a, "device-a").await,
        3,
        "本机数据不应被动"
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
    crate::commands::storage::purge_cloud_data_impl(&a.pool, &a.engine, None, Some(&a.mem), true)
        .await
        .expect("purge_cloud_data keep_local");

    // 本机的 3 行原样保留
    assert_eq!(
        count_for_device(&a, "device-a").await,
        3,
        "keep_local=true 时本机数据必须完整保留"
    );
    assert!(!signed_in(&a).await, "移除本设备后应已退出登录");

    // B sync → pull tombstone → B 的 A-mirror 仍被清（对端不知道本机要保留）
    b.engine.sync_now().await.unwrap();
    assert_eq!(
        count_for_device(&b, "device-a").await,
        0,
        "对端仍按 tombstone 清 A 的 mirror（云端语义对外一致）"
    );

    // A 重新登录同一个账号再同步，会拉到自己的 tombstone：本机的 tombstone 不执行，承诺保留的数据还在
    inject_fake_auth(&a.pool).await;
    a.engine.sync_now().await.unwrap();
    assert_eq!(
        count_for_device(&a, "device-a").await,
        3,
        "本机自己的 tombstone 不应执行"
    );
}

/// tombstone 跟其它文件一样按 modifiedTime 顺序处理：排在它后面的文件，就算带着
/// clearedAt 之前的行，也照常合并。这样结果不取决于文件碰巧落在哪一轮 pull。
#[tokio::test]
async fn tombstone_applies_in_file_order() {
    let drive = Arc::new(InMemoryDriveStore::new());
    let a = make_device("device-a", drive.clone()).await;
    let b = make_device("device-b", drive.clone()).await;

    let captured = Local::now();
    for p in ["Code", "Chrome"] {
        insert_sealed(&a, p, captured, 30).await;
    }
    a.engine.sync_now().await.unwrap();

    // 别处上传了 A 的 tombstone（比如另一台设备把 A「从云端永久移除」）
    let cleared_at = crate::storage::utc_now_rfc3339();
    let tombstone = serde_json::to_vec(&serde_json::json!({ "clearedAt": cleared_at })).unwrap();
    drive
        .upsert_by_name("device.device-a.tombstone.json", &tombstone)
        .await
        .unwrap();

    // A 其实还开着：又记一段，当天文件整份重写，排到 tombstone 后面，里面仍带着之前的两行
    insert_sealed(&a, "Slack", captured, 30).await;
    a.engine.sync_now().await.unwrap();

    // B 第一次拉取，tombstone 和新文件落在同一轮：先执行 tombstone，再合并新文件
    b.engine.sync_now().await.unwrap();
    assert_eq!(count_for_device(&b, "device-a").await, 3);
}

/// Test 3：「清空数据」之后本机的历史不再从云端回来；其他设备上的副本原样保留。
// 清空数据会删 <数据目录>/icons，所以整条测试持 env 锁、指到临时目录。
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn clear_data_does_not_pull_own_history_back() {
    let _env_lock = crate::repo::test_util::lock_data_dir_env();
    let _data_dir = DataDirOverride::unique_temp();
    let _digest = crate::memory::digest::drain_lock().await;

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

    crate::commands::storage::purge_local_data_impl(&a.pool, Some(&a.mem))
        .await
        .expect("purge_local_data");
    assert_eq!(count_for_device(&a, "device-a").await, 0);

    // 再同步两轮，什么都不该回来
    a.engine.sync_now().await.unwrap();
    a.engine.sync_now().await.unwrap();
    assert_eq!(
        count_for_device(&a, "device-a").await,
        0,
        "清空后本机历史不应从云端回来"
    );

    // B 那边 A 的副本一条不少
    b.engine.sync_now().await.unwrap();
    assert_eq!(
        count_for_device(&b, "device-a").await,
        3,
        "其他设备上的副本不受影响"
    );
}

/// 「清空数据」删的是数据，不是规则：其他设备上的分组和成员不受影响。
// 清空数据会删 <数据目录>/icons，所以整条测试持 env 锁、指到临时目录。
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn clear_data_keeps_other_devices_groups() {
    let _env_lock = crate::repo::test_util::lock_data_dir_env();
    let _data_dir = DataDirOverride::unique_temp();
    let _digest = crate::memory::digest::drain_lock().await;

    let drive = Arc::new(InMemoryDriveStore::new());
    let a = make_device("device-a", drive.clone()).await;
    let b = make_device("device-b", drive.clone()).await;

    // 两台都用过 Code：各自建了分组和成员
    let captured = Local::now();
    for dev in [&a, &b] {
        crate::repo::app_groups::ensure_group(&dev.pool, "Code")
            .await
            .unwrap();
        insert_sealed(dev, "Code", captured, 30).await;
    }
    a.engine.sync_now().await.unwrap();
    b.engine.sync_now().await.unwrap();

    crate::commands::storage::purge_local_data_impl(&a.pool, Some(&a.mem))
        .await
        .expect("purge_local_data");
    a.engine.sync_now().await.unwrap();
    b.engine.sync_now().await.unwrap();

    let (live_groups, live_members): (i64, i64) = b
        .pool
        .0
        .call(|conn| {
            // 分组 id 不一定等于进程名（别名表可能把 Code 归到规范名下），从成员表找
            let g = conn.query_row(
                "SELECT COUNT(*) FROM app_groups g
                 JOIN app_group_members m ON m.group_id = g.id
                 WHERE m.process_name = 'Code' AND g.deleted_at IS NULL",
                [],
                |r| r.get(0),
            )?;
            let m = conn.query_row(
                "SELECT COUNT(*) FROM app_group_members WHERE process_name = 'Code' AND deleted_at IS NULL",
                [],
                |r| r.get(0),
            )?;
            Ok((g, m))
        })
        .await
        .unwrap();
    assert_eq!(live_groups, 1, "A 清空数据后，B 的分组不应被删");
    assert_eq!(live_members, 1, "A 清空数据后，B 的成员不应被删");
}

/// 「清空数据」连记忆库一起清：帧登记、OCR 文字和它的全文索引、聊天记录。
// 清空数据会删 <数据目录>/icons，所以整条测试持 env 锁、指到临时目录。
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn clear_data_clears_memory_db() {
    let _env_lock = crate::repo::test_util::lock_data_dir_env();
    let _data_dir = DataDirOverride::unique_temp();
    let _digest = crate::memory::digest::drain_lock().await;

    let drive = Arc::new(InMemoryDriveStore::new());
    let a = make_device("device-a", drive.clone()).await;

    // 一条帧登记 + 一条 OCR 会话（含行级留痕）
    a.mem
        .0
        .call(|conn| {
            conn.execute_batch(
                "INSERT INTO frames(path, ts, local_date, app_id, title, ocr_state)
                 VALUES ('2026-07-05/a.jpg', '2026-07-05T10:00:00+09:00', '2026-07-05', 'code', '标题甲', 1);
                 INSERT INTO text_sessions(id, local_date, started_ts, ended_ts, app_id, title, text)
                 VALUES (1, '2026-07-05', 't0', 't1', 'code', '标题甲', '秘密订单编号八八四二');
                 INSERT INTO session_lines(session_id, line_no, text, first_path, first_ts)
                 VALUES (1, 0, '秘密订单编号八八四二', '2026-07-05/a.jpg', '2026-07-05T10:00:00+09:00');",
            )
            .db()?;
            Ok(())
        })
        .await
        .unwrap();
    // 一段聊天
    let conv = crate::chat::store::create_conversation(&a.mem, "测试会话")
        .await
        .unwrap();
    crate::chat::store::append_user(&a.mem, conv, "上周看了什么?", None)
        .await
        .unwrap();

    crate::commands::storage::purge_local_data_impl(&a.pool, Some(&a.mem))
        .await
        .expect("purge_local_data");

    let counts: Vec<(&str, i64)> = a
        .mem
        .0
        .call(|conn| {
            let mut out = Vec::new();
            for table in [
                "frames",
                "text_sessions",
                "session_lines",
                "chat_conversations",
                "chat_messages",
            ] {
                let n: i64 =
                    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
                out.push((table, n));
            }
            let hits: i64 = conn.query_row(
                "SELECT COUNT(*) FROM text_sessions_fts WHERE text_sessions_fts MATCH '秘密'",
                [],
                |r| r.get(0),
            )?;
            out.push(("text_sessions_fts", hits));
            Ok(out)
        })
        .await
        .unwrap();
    for (table, n) in counts {
        assert_eq!(n, 0, "清空数据后 {table} 应为空");
    }
}

/// 「清空数据」之后再「从云端移除本设备」，重新登录同一个账号：清掉的其他设备历史不会被拉回来。
// 清空数据会删 <数据目录>/icons，所以整条测试持 env 锁、指到临时目录。
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn remove_device_does_not_pull_cleared_history_back() {
    let _env_lock = crate::repo::test_util::lock_data_dir_env();
    let _data_dir = DataDirOverride::unique_temp();
    let _digest = crate::memory::digest::drain_lock().await;

    let drive = Arc::new(InMemoryDriveStore::new());
    let a = make_device("device-a", drive.clone()).await;
    let b = make_device("device-b", drive.clone()).await;

    let captured = Local::now();
    for p in ["Code", "Chrome", "Slack"] {
        insert_sealed(&b, p, captured, 30).await;
    }
    b.engine.sync_now().await.unwrap();
    a.engine.sync_now().await.unwrap();
    assert_eq!(count_for_device(&a, "device-b").await, 3);

    crate::commands::storage::purge_local_data_impl(&a.pool, Some(&a.mem))
        .await
        .expect("purge_local_data");
    assert_eq!(count_for_device(&a, "device-b").await, 0);

    crate::commands::storage::purge_cloud_data_impl(&a.pool, &a.engine, None, Some(&a.mem), false)
        .await
        .expect("purge_cloud_data");
    inject_fake_auth(&a.pool).await;
    a.engine.sync_now().await.unwrap();
    assert_eq!(
        count_for_device(&a, "device-b").await,
        0,
        "移除本设备后，清空过的 B 的历史不应被拉回来"
    );
}

/// 打开可选数据集开关：只把这一类的历史补上，别的类不受影响。
/// 清空过数据的机器尤其看得出来——其它设备的活动记录不该跟着回来。
// 清空数据会删 <数据目录>/icons，所以整条测试持 env 锁、指到临时目录。
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn turning_on_a_dataset_pulls_only_that_dataset() {
    let _env_lock = crate::repo::test_util::lock_data_dir_env();
    let _data_dir = DataDirOverride::unique_temp();
    let _digest = crate::memory::digest::drain_lock().await;

    let drive = Arc::new(InMemoryDriveStore::new());
    let a = make_device("device-a", drive.clone()).await;
    let b = make_device("device-b", drive.clone()).await;

    // B 开着 AI 总结同步：推上去一条活动 + 一份日报
    enable_ai_summaries_sync(&b).await;
    insert_sealed(&b, "Code", Local::now(), 30).await;
    b.pool
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
    b.engine.sync_now().await.unwrap();

    // A 的开关关着：拉到 B 的活动，跳过 B 的日报
    a.engine.sync_now().await.unwrap();
    assert_eq!(count_for_device(&a, "device-b").await, 1);
    assert_eq!(ai_summary_count(&a).await, 0, "开关关着不该拉到日报");

    // A 清空数据后打开 AI 总结开关，再同步
    crate::commands::storage::purge_local_data_impl(&a.pool, Some(&a.mem))
        .await
        .expect("purge_local_data");
    enable_ai_summaries_sync(&a).await;
    a.engine.sync_now().await.unwrap();

    assert_eq!(ai_summary_count(&a).await, 1, "打开开关后应补上 B 的日报");
    assert_eq!(
        count_for_device(&a, "device-b").await,
        0,
        "清空过的活动记录不该跟着回来"
    );
}

/// Test 5：flush_pull cursor "longest true prefix" 推进逻辑 ——
/// 中间文件失败时 cursor 应停在前一个成功文件的 modifiedTime，
/// 不能跨过失败文件推到后面成功的（否则下次 pull 永久丢失失败文件）。
#[tokio::test]
async fn flush_pull_cursor_stops_at_failed_file() {
    let drive_store = Arc::new(InMemoryDriveStore::new());
    let dev = make_device("device-self", drive_store.clone()).await;

    // File 1（T1）: meta.json 合法，合并成功
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

    // File 2（T2）: categories.json 内容是坏 JSON，merge_categories 失败
    drive_store
        .upsert_by_name(
            "device.device-d.categories.json",
            b"[ bad JSON, not a valid array",
        )
        .await
        .unwrap();

    // File 3（T3）: app_groups.json 合法（空数组），merge_app_groups 成功
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
        crate::chat::store::MsgUsage {
            prompt: 200,
            completion: 80,
            ..Default::default()
        },
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
/// 注入一次 upsert 失败 → sync_now 必须原样返回那次 500、失败行留在 outbox
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
        matches!(err, crate::error::Error::DriveHttp { status: 500, .. }),
        "sync_now 应原样返回上传时的 500，实际: {err:?}"
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

/// 任务 3：metadata entity 的双设备 roundtrip。
/// A 端各表插一行 + 手工入 outbox（category / device / app_icon / app_group /
/// app_group_member 五类）→ A sync 推文件 → B sync 拉回 → B 各表
/// 字段逐一与 A 写入值相等。
/// 一条测试同时吃掉 push 构建侧的 build_* 与 pull 合并侧对应的 merge_*。
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
    const T_DEV_SEEN: &str = "2026-07-01T00:00:04Z";
    const T_DEV_UPD: &str = "2026-07-01T00:00:05Z";
    const T_ICON: &str = "2026-07-01T00:00:06Z";
    const T_GRP: &str = "2026-07-01T00:00:07Z";
    const T_MEMBER: &str = "2026-07-01T00:00:08Z";
    let icon_bytes: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 1, 2, 3, 4];

    // A 本机 OS,随 meta 同步到 B
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
    // (app_group_members)后落。B 按 modifiedTime 升序合并,引用目标必然先到位。
    // 若同轮乱序推(push 的 HashMap 随机序),引用方文件可能先被合并,行级
    // FOREIGN KEY 失败仅 warn 且游标照常越过 —— 该缺陷已记录为产品 bug,
    // 这里不让测试依赖随机顺序。
    for (entity, pk) in [
        ("category", "cat-e2e"),
        ("device", "device-a"),
        ("app_icon", "Proc-E2E"),
        ("app_group", "grp-e2e"),
    ] {
        enqueue_entity(&a, entity, pk).await;
    }
    a.engine.sync_now().await.expect("A 第一轮 sync 应成功");
    for (entity, pk) in [("app_group_member", "Proc-E2E")] {
        enqueue_entity(&a, entity, pk).await;
    }
    a.engine.sync_now().await.expect("A 第二轮 sync 应成功");
    assert_eq!(outbox_count(&a.pool).await, 0, "A 推完 outbox 应清空");
    assert_eq!(
        drive.list_appdata_files("").await.unwrap().len(),
        5,
        "5 类 entity 各一个文件"
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
    let (cat, dev, icon, grp, member): (
        CatRow,
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
            Ok((cat, dev, icon, grp, member))
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

/// ADR-0003 / ADR-0004: a device that upgrades from a version which still
/// published `app_categories.json` and `process_paths.json` deletes its own
/// copies on the first push after launch. Other devices' copies stay: each
/// device cleans up its own once it upgrades.
#[tokio::test]
async fn first_push_deletes_own_legacy_cloud_files() {
    let drive = Arc::new(InMemoryDriveStore::new());
    let a = make_device("device-a", drive.clone()).await;
    for name in [
        "device.device-a.app_categories.json",
        "device.device-a.process_paths.json",
        "device.device-b.app_categories.json",
        "device.device-b.process_paths.json",
    ] {
        drive.upsert_by_name(name, b"[]").await.unwrap();
    }

    a.engine.sync_now().await.expect("A sync 应成功");

    let mut names: Vec<String> = drive
        .list_appdata_files("")
        .await
        .unwrap()
        .into_iter()
        .map(|f| f.name)
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "device.device-b.app_categories.json".to_string(),
            "device.device-b.process_paths.json".to_string(),
        ],
        "本机的两份旧文件应被删掉,别的设备的不能动"
    );
}

/// ADR-0003: a peer's `process_paths.json`, still published by older versions,
/// is no longer merged. Pull treats the name as unknown, so the table stays
/// untouched and the cursor moves past the file instead of stalling on it.
#[tokio::test]
async fn peer_process_paths_file_is_ignored() {
    let drive = Arc::new(InMemoryDriveStore::new());
    let dev = make_device("device-self", drive.clone()).await;
    let body = serde_json::json!([{
        "processName": "Peer-App",
        "exePath": "/Applications/Peer-App.app",
        "seenAt": "2026-07-01T00:00:00Z",
        "updatedAt": "2026-07-01T00:00:00Z",
    }])
    .to_string();
    drive
        .upsert_by_name("device.device-peer.process_paths.json", body.as_bytes())
        .await
        .unwrap();
    let file_time = drive.list_appdata_files("").await.unwrap()[0]
        .modified_time
        .clone();

    dev.engine.sync_now().await.expect("sync 应成功");

    let rows: i64 = dev
        .pool
        .0
        .call(|conn| {
            let n = conn
                .query_row("SELECT COUNT(*) FROM process_paths", [], |r| r.get(0))
                .db()?;
            Ok(n)
        })
        .await
        .unwrap();
    assert_eq!(rows, 0, "对端的路径文件不应再被合并进本机");
    assert_eq!(
        super::io::read_cursor(&dev.pool, "drive_files")
            .await
            .unwrap(),
        file_time,
        "认不出的文件应被越过,游标推进到它的 modifiedTime"
    );
}

/// 回归:同一轮 pull 内,引用方文件(app_group_members)的 modifiedTime 早于被引用
/// 文件(app_groups)时,行不能丢。
///
/// 这正是 push HashMap 随机序下约 50% 概率触发的实锤 bug:按 modifiedTime 升序
/// 直走的话,引用方先合并 → 行级 FK 失败仅 warn → handled=true
/// 让游标越过 → 该行永久丢失(下轮 `modifiedTime >` 不再拉它)。
/// 所以 pull 让 app_groups 先行,单轮全部落库。
/// 游标推进语义不受遍历顺序影响(handled[] 仍按文件列表原序求最长 true 前缀)。
#[tokio::test]
async fn pull_single_round_merges_children_even_when_files_precede_parents() {
    use rusqlite::OptionalExtension;

    let drive = Arc::new(InMemoryDriveStore::new());
    let dev = make_device("device-self", drive.clone()).await;

    // T1: 远端设备 meta
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

    // T2: 引用方文件先落 Drive(modifiedTime 更早)
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

    // T3: 被引用文件后落 Drive
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
        pos("device.device-x.app_group_members.json") < pos("device.device-x.app_groups.json"),
        "前置条件:引用方文件的 modifiedTime 必须早于被引用文件"
    );

    dev.engine.sync_now().await.expect("sync 应成功");

    // 单轮之后两行必须全部落库(修复前引用方行被 FK 吃掉且永不再拉)
    let (grp_n, member_grp): (i64, Option<String>) = dev
        .pool
        .0
        .call(|conn| {
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
            Ok((grp_n, member_grp))
        })
        .await
        .unwrap();

    assert_eq!(grp_n, 1, "app_groups 行应落库");
    assert_eq!(
        member_grp.as_deref(),
        Some("grp-fk"),
        "app_group_members 行不能因文件序早于 app_groups 而丢失"
    );
}

// ─────────────── WebDAV（ADR-0007、ADR-0008） ───────────────
//
// 几台设备共用一个假 WebDAV 服务器，引擎走真的 push / pull；pull 靠 manifest 文件找改动。

async fn make_webdav_device(self_id: &str, dav: Arc<FakeDav>) -> TestDevice {
    let pool = DbPool::open_in_memory().await.unwrap();
    migrations::run(&pool).await.unwrap();
    let mem = crate::memory::MemoryDb::open_in_memory().await.unwrap();
    let client = WebDavClient::with_fake_server(dav, pool.clone(), self_id.to_string());
    let engine = Arc::new(SyncEngine::with_backend(
        pool.clone(),
        Some(mem.clone()),
        CloudBackend::WebDav(Box::new(client)),
        self_id.to_string(),
    ));
    TestDevice {
        pool,
        mem,
        engine,
        self_id: self_id.to_string(),
    }
}

/// 某台设备某天的活动日文件在服务器上的路径（相对同步根目录）。
fn webdav_day_file(device_id: &str, day: DateTime<Local>) -> String {
    format!(
        "{device_id}/activities/{}/{}.ndjson",
        day.format("%Y"),
        day.format("%Y-%m-%d")
    )
}

/// 调用记录里第 `from` 条往后的 GET 路径。
async fn gets_since(dav: &FakeDav, from: usize) -> Vec<String> {
    dav.calls().await[from..]
        .iter()
        .filter_map(|c| match c {
            Call::Get(path) => Some(path.clone()),
            _ => None,
        })
        .collect()
}

/// A push 3 行 → B pull 到 3 行；云上是 A 的日文件加根目录的 manifest 文件。
#[tokio::test]
async fn webdav_cross_device_push_pull_basic() {
    let dav = Arc::new(FakeDav::new());
    let a = make_webdav_device("device-a", dav.clone()).await;
    let b = make_webdav_device("device-b", dav.clone()).await;

    let captured = Local::now();
    insert_sealed(&a, "Code", captured, 30).await;
    insert_sealed(&a, "Chrome", captured, 60).await;
    insert_sealed(&a, "Slack", captured, 45).await;
    a.engine.sync_now().await.expect("A sync_now");
    b.engine.sync_now().await.expect("B sync_now");

    assert_eq!(count_for_device(&b, "device-a").await, 3);
    assert_eq!(sum_secs_for_device(&b, "device-a").await, 135);
    assert_eq!(count_for_device(&a, "device-b").await, 0);
    assert!(dav
        .file(&webdav_day_file("device-a", captured))
        .await
        .is_some());
    assert!(dav.file("manifest.device-a.json").await.is_some());
}

/// 合并完、进度也记下之后，什么都没变的一轮只发一个 PROPFIND（ADR-0008）：
/// push 没东西不发请求，pull 看 manifest 文件的时间没变就不 GET。
#[tokio::test]
async fn webdav_idle_round_sends_one_propfind() {
    let dav = Arc::new(FakeDav::new());
    let a = make_webdav_device("device-a", dav.clone()).await;
    let b = make_webdav_device("device-b", dav.clone()).await;
    insert_sealed(&a, "Code", Local::now(), 30).await;
    a.engine.sync_now().await.unwrap();
    b.engine.sync_now().await.unwrap(); // 报出、合并
    b.engine.sync_now().await.unwrap(); // 游标过了 manifest 时间：记下进度

    let before = dav.calls().await.len();
    b.engine.sync_now().await.unwrap();
    assert_eq!(&dav.calls().await[before..], &[Call::Propfind("".into())]);
    assert_eq!(count_for_device(&b, "device-a").await, 1);
}

/// A 两个日文件只改了一个：B 只 GET manifest 文件和改了的那个。
#[tokio::test]
async fn webdav_pulls_only_the_files_that_changed() {
    let dav = Arc::new(FakeDav::new());
    let a = make_webdav_device("device-a", dav.clone()).await;
    let b = make_webdav_device("device-b", dav.clone()).await;
    let today = Local::now();
    let yesterday = today - Duration::days(1);
    insert_sealed(&a, "Code", yesterday, 30).await;
    insert_sealed(&a, "Code", today, 30).await;
    a.engine.sync_now().await.unwrap();
    b.engine.sync_now().await.unwrap();
    b.engine.sync_now().await.unwrap();

    insert_sealed(&a, "Chrome", today, 60).await;
    a.engine.sync_now().await.unwrap();
    let before = dav.calls().await.len();
    b.engine.sync_now().await.unwrap();

    assert_eq!(
        gets_since(&dav, before).await,
        vec![
            "manifest.device-a.json".to_string(),
            webdav_day_file("device-a", today)
        ]
    );
    assert_eq!(count_for_device(&b, "device-a").await, 3);
}

/// 下载失败的文件下一轮重试：失败的那几轮不能把进度记过去（ADR-0008 Pull 一节）。
#[tokio::test]
async fn webdav_a_failed_download_is_retried() {
    let dav = Arc::new(FakeDav::new());
    let a = make_webdav_device("device-a", dav.clone()).await;
    let b = make_webdav_device("device-b", dav.clone()).await;
    let captured = Local::now();
    insert_sealed(&a, "Code", captured, 30).await;
    a.engine.sync_now().await.unwrap();

    // 服务器上 A 的日文件暂时读不到：B 连着两轮下载失败
    let day = webdav_day_file("device-a", captured);
    let body = dav.take_file(&day).await.unwrap();
    b.engine.sync_now().await.unwrap();
    b.engine.sync_now().await.unwrap();
    assert_eq!(count_for_device(&b, "device-a").await, 0);

    // 文件回来了，下一轮补上
    dav.seed_file(&day, &body).await;
    b.engine.sync_now().await.unwrap();
    assert_eq!(count_for_device(&b, "device-a").await, 1);
}

/// B 的 AI 总结开关关着时先同步过，打开开关后要补上 A 的日报，而清空过的活动不能跟着
/// 回来（ADR-0006：开关打开后只有这类数据从停下的地方接着拉）。
// 清空数据会删 <数据目录>/icons，所以整条测试持 env 锁、指到临时目录。
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn webdav_turning_on_a_dataset_pulls_its_history() {
    let _env_lock = crate::repo::test_util::lock_data_dir_env();
    let _data_dir = DataDirOverride::unique_temp();
    let _digest = crate::memory::digest::drain_lock().await;

    let dav = Arc::new(FakeDav::new());
    let a = make_webdav_device("device-a", dav.clone()).await;
    let b = make_webdav_device("device-b", dav.clone()).await;

    // A 开着 AI 总结同步：推上去一条活动 + 一份日报
    enable_ai_summaries_sync(&a).await;
    insert_sealed(&a, "Code", Local::now(), 30).await;
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

    // B 的开关关着：拉到活动、跳过日报；第二轮记下进度
    b.engine.sync_now().await.unwrap();
    b.engine.sync_now().await.unwrap();
    assert_eq!(count_for_device(&b, "device-a").await, 1);
    assert_eq!(ai_summary_count(&b).await, 0, "开关关着不该拉到日报");

    // B 清空数据后打开 AI 总结开关，再同步
    crate::commands::storage::purge_local_data_impl(&b.pool, Some(&b.mem))
        .await
        .expect("purge_local_data");
    enable_ai_summaries_sync(&b).await;
    b.engine.sync_now().await.unwrap();
    assert_eq!(ai_summary_count(&b).await, 1, "打开开关后应补上 A 的日报");
    assert_eq!(
        count_for_device(&b, "device-a").await,
        0,
        "清空过的活动记录不该跟着回来"
    );
}

/// 新开的账号，云上还没有同步根目录：两台设备照常同步。
#[tokio::test]
async fn webdav_sync_works_on_a_new_account() {
    let dav = Arc::new(FakeDav::without_root());
    let a = make_webdav_device("device-a", dav.clone()).await;
    let b = make_webdav_device("device-b", dav.clone()).await;
    insert_sealed(&a, "Code", Local::now(), 30).await;

    b.engine.sync_now().await.expect("B 先同步：云上什么都没有");
    a.engine.sync_now().await.expect("A 第一次上传");
    b.engine.sync_now().await.expect("B 拉到 A");
    assert_eq!(count_for_device(&b, "device-a").await, 1);
}

/// 同一个库先用 Drive 同步过、再连 WebDAV：WebDAV 从头拉，AI 总结整份推上去，Drive 的
/// 进度一行不动（ADR-0011 §3）。要是 WebDAV 还读 Drive 的那几行，B 会跳过 A 的文件，
/// 也不推 AI 总结。
#[tokio::test]
async fn webdav_keeps_its_progress_apart_from_drive() {
    let dav = Arc::new(FakeDav::new());
    let a = make_webdav_device("device-a", dav.clone()).await;
    let b = make_webdav_device("device-b", dav.clone()).await;
    insert_sealed(&a, "Code", Local::now(), 30).await;
    a.engine.sync_now().await.unwrap();

    // B 用 Drive 时留下的进度：游标比 A 的 manifest 文件时间晚；AI 总结推到过 Drive，
    // 指纹跟现在一样
    enable_ai_summaries_sync(&b).await;
    b.pool
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
    let drive_progress = [
        ("drive_files", "2099-01-01T00:00:00Z"),
        ("push.ai_summaries", "2026-07-05T10:00:00Z:1"),
    ];
    for (name, value) in drive_progress {
        super::io::write_cursor(&b.pool, name, value).await.unwrap();
    }

    b.engine.sync_now().await.unwrap();

    assert_eq!(count_for_device(&b, "device-a").await, 1, "A 的活动要拉到");
    assert!(
        dav.file("device-b/ai_summaries.json").await.is_some(),
        "AI 总结要推到 WebDAV"
    );
    for (name, value) in drive_progress {
        assert_eq!(
            super::io::read_cursor(&b.pool, name).await.unwrap(),
            value,
            "{name}"
        );
    }
}
