//! 存储 / 数据目录相关 Tauri 命令——给前端「设置 → 数据」面板用。
//!
//! 包括：DB / 截图目录的字节占用统计、清空 activities / 截图、切换 data_root、
//! 在系统文件管理器里打开截图目录。

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::capture::CaptureService;
use crate::repo::settings;
use crate::storage::{db_path, utc_now_rfc3339, DbPool, SqliteResultExt};
use crate::sync::engine::SyncEngine;

/// `get_storage_info` 命令的返回。前端「设置 → 数据」面板拿来渲染当前空间占用。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    /// hindsight.sqlite 文件大小（字节）。文件不存在或读取失败返回 0
    pub db_bytes: u64,
    /// 截图目录递归统计的总字节数（含子目录）
    pub screenshots_bytes: u64,
    /// hindsight.sqlite 的绝对路径——前端可点开复制
    pub db_path: String,
    /// 截图目录绝对路径
    pub screenshots_path: String,
}

/// 拉一次 DB 与截图目录的字节占用 + 路径。
///
/// 截图目录递归统计有可能慢（万张截图），故用 `spawn_blocking` 不堵 runtime；
/// DB 文件单一，`tokio::fs::metadata` 一次 stat 即可。
#[tauri::command]
pub async fn get_storage_info(pool: State<'_, DbPool>) -> Result<StorageInfo, String> {
    let cfg = settings::load(&pool).await.map_err(String::from)?;
    let db = db_path().map_err(String::from)?;

    let db_bytes = tokio::fs::metadata(&db).await.map(|m| m.len()).unwrap_or(0);
    let shots_path = std::path::PathBuf::from(&cfg.screenshot_path);
    let shots_bytes = tokio::task::spawn_blocking({
        let p = shots_path.clone();
        move || dir_size(&p)
    })
    .await
    .map_err(|e| e.to_string())?;

    Ok(StorageInfo {
        db_bytes,
        screenshots_bytes: shots_bytes,
        db_path: db.to_string_lossy().to_string(),
        screenshots_path: cfg.screenshot_path,
    })
}

/// 清空**本机**所有捕获 / 派生数据（不动云端 Drive，不动其它用户自定义：settings /
/// categories / app_categories / devices / auth_state）。
///
/// **清的表**（7 张硬删 + 2 张软删 + 1 个 cursor 重置）：
/// - `activities` —— 焦点会话原始流水
/// - `process_paths` —— process_name → exe path 映射
/// - `app_icons` —— icon BLOB 缓存（每张 50KB～300KB，是占用大头 ← v0.6.7 之前漏了这张
///   导致用户点完按钮 sqlite 还 20+ MB）
/// - `ai_image_descriptions` —— step 1 逐图描述
/// - `ai_summaries` —— step 2 段总结
/// - `screenshot_embeddings` —— MobileNet dedup 缓存
/// - `sync_outbox` —— 必须清，下个 push tick 否则会按"现状"重写 ndjson 把对应天写成空，
///   **意外删除云端数据**
/// - `app_group_members` / `app_groups` —— **软删**（带 outbox enqueue）：清空 activities 后
///   每个 member 的 process_name 都失去对应活动，每个 group 也再无 active 成员，即变成
///   Apps 页显示但 icon / 数据全无的 "phantom" 行（list_groups 不过滤活动存在性）。
///   用户点"清空所有活动"的意图就是一切归零；并且跨设备同步会从对端反复复活这些 group。
///   outbox 让对端 pull 时同步软删，避免 ping-pong。
/// - `sync_cursor.drive_files` 重置到 epoch —— DELETE 把 origin='remote' 镜像也清了；
///   游标不动 → 下次 pull 只看 modifiedTime > cursor 的新文件 → 老镜像永远拉不回。
///   重置后下次 pull 走全量，对端历史数据自动重新镜像回本机
///
/// 完成 DELETE 后立刻 `VACUUM` —— SQLite `DELETE` 只把页标记 free 不缩文件，
/// 必须 VACUUM 才能让用户在 Finder / `du` 看到磁盘空间实际释放。VACUUM 不能在
/// transaction 内执行，所以分两个 `pool.0.call` 块。
///
/// 末尾清 icon 文件 cache 目录 `<data_root>/icons/`：`app_icons` 表清了但文件缓存
/// 还在的话，下次 `getAppIcon` 走 Layer 1 还是命中老 PNG（见
/// [`crate::commands::icons::get_app_icon`]），等于白清。
///
/// 整个清库过程包在 `svc.run_with_session_cleared` 里：进入时丢弃当前活跃会话
/// （否则下一 tick 会去 UPDATE 已被删除的行），且持锁期间 tick 无法插入新行。
///
/// 幂等：连续多次调用每次效果一致（DELETE 空表 / VACUUM 已紧凑过 / UPDATE 已是 epoch
/// 的 cursor / fs 删已不存在的目录 都是 no-op）。
#[tauri::command]
pub async fn purge_activities(
    pool: State<'_, DbPool>,
    svc: State<'_, Arc<CaptureService>>,
    engine: State<'_, Arc<SyncEngine>>,
) -> Result<(), String> {
    // 两把并发保护，缺一不可：
    // 1. pause_flushes：等在途 push/pull tick 结束并挡住新 tick——否则 push 若已读完
    //    outbox、还没读表，会把清空后的表内容（空 ndjson）写上 Drive，云端备份被抹掉。
    // 2. run_with_session_cleared：持 capture 会话锁清指针——否则并发 tick 可能在
    //    DELETE 之后插入新行又被 reset 清掉指针，留一条永不 seal 的孤儿行。
    let _sync_guard = engine.pause_flushes().await;
    svc.run_with_session_cleared(|| async { purge_activities_impl(&pool).await })
        .await
}

/// 抽出来的实际实现，给单测可以直接调用（绕开 Tauri State<> 包装 + CaptureService
/// 在 test 里构造不便）。语义见 [`purge_activities`] doc。
pub(crate) async fn purge_activities_impl(pool: &DbPool) -> Result<(), String> {
    // Phase 1: 7 张 DELETE + cursor reset + 软删 phantom app_groups/members + outbox
    pool.0
        .call(|conn| {
            // ── 派生数据全清 + cursor reset ──
            conn.execute_batch(
                "DELETE FROM activities;
                 DELETE FROM process_paths;
                 DELETE FROM app_icons;
                 DELETE FROM ai_image_descriptions;
                 DELETE FROM ai_summaries;
                 DELETE FROM screenshot_embeddings;
                 DELETE FROM screenshot_dedup_map;
                 DELETE FROM sync_outbox;
                 UPDATE sync_cursor SET last_pulled_at = '1970-01-01T00:00:00Z'
                  WHERE entity = 'drive_files';",
            )
            .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;

            // ── 软删 phantom app_groups + app_group_members ──
            //
            // activities 已清空：每个 member 的 process_name 在 activities 里都找不到，
            // 每个 group 也再无 active 成员。这些 phantom 行让 Apps 页显示空数据死行，
            // 跨设备同步还会从对端反复复活。同步软删 + outbox 让对端收敛。
            //
            // 顺序：先成员后组（组的 phantom 判定要看 member 的 deleted_at 状态）。
            // 整个 conn.call 块共享同一 SQLite 连接，UPDATE/SELECT/outbox.enqueue 都在
            // 一致视图上。
            let now = utc_now_rfc3339();

            // Step A: 快照所有 active member PK → 软删 → 逐个 outbox enqueue
            let member_pks: Vec<String> = {
                let mut stmt = conn
                    .prepare("SELECT process_name FROM app_group_members WHERE deleted_at IS NULL")
                    .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
                let rows = stmt
                    .query_map([], |r| r.get::<_, String>(0))
                    .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r.map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?);
                }
                out
            };
            conn.execute(
                "UPDATE app_group_members
                    SET deleted_at = ?1, updated_at = ?1
                  WHERE deleted_at IS NULL",
                rusqlite::params![now],
            )
            .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
            for pk in &member_pks {
                let payload = serde_json::json!({ "processName": pk }).to_string();
                crate::repo::outbox::enqueue(
                    conn,
                    crate::repo::outbox::OutboxOp::Upsert,
                    crate::repo::outbox::OutboxEntity::AppGroupMember,
                    pk,
                    &payload,
                )
                .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
            }

            // Step B: 快照"现已 phantom"（无 active 成员）的 group PK → 软删 → outbox
            let group_pks: Vec<String> = {
                let mut stmt = conn
                    .prepare(
                        "SELECT id FROM app_groups
                         WHERE deleted_at IS NULL
                           AND NOT EXISTS (
                             SELECT 1 FROM app_group_members m
                              WHERE m.group_id = app_groups.id
                                AND m.deleted_at IS NULL
                           )",
                    )
                    .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
                let rows = stmt
                    .query_map([], |r| r.get::<_, String>(0))
                    .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r.map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?);
                }
                out
            };
            conn.execute(
                "UPDATE app_groups
                    SET deleted_at = ?1, updated_at = ?1
                  WHERE deleted_at IS NULL
                    AND NOT EXISTS (
                      SELECT 1 FROM app_group_members m
                       WHERE m.group_id = app_groups.id
                         AND m.deleted_at IS NULL
                    )",
                rusqlite::params![now],
            )
            .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
            for pk in &group_pks {
                let payload = serde_json::json!({ "id": pk }).to_string();
                crate::repo::outbox::enqueue(
                    conn,
                    crate::repo::outbox::OutboxOp::Upsert,
                    crate::repo::outbox::OutboxEntity::AppGroup,
                    pk,
                    &payload,
                )
                .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
            }

            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?;

    // Phase 2: VACUUM —— 必须在 transaction 外执行（SQLite 硬限制），分一个独立 call() 块
    pool.0
        .call(|conn| {
            conn.execute_batch("VACUUM")
                .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?;

    // Phase 3: 清 icon 文件 cache 目录（best-effort，目录不存在 / 删失败都不抛错）
    if let Ok(data_root) = crate::storage::db_path_dir() {
        let icons_dir = data_root.join("icons");
        let _ = tokio::fs::remove_dir_all(&icons_dir).await;
    }

    Ok(())
}

/// 清空**云端**数据 —— 完整语义是**"所有设备（含本机）忘记我此刻之前由本机捕获的数据"**。
///
/// 历史：早期实现只删 Drive 不动本机，后来发现对端 mirror 永久保留对称性破坏；
/// 加 tombstone 让对端 trim；又发现源端本地保留 pre-clearedAt 的旧行 + 全量 push
/// rewrite 会把这些行重新写回 Drive，对端 trim 后看到的比源端少 —— 本机 9 min /
/// 对端 4 min 这种 asymmetric 状态。最终走 **Option C**：源端、对端、Drive 三处
/// 完全对称，clearedAt 统一为操作时刻。源端 post-clearedAt 的新 capture 完全不受影响，
/// 继续 push / sync 正常。
///
/// 流程：
/// 1. 拿 OAuth token；未登录直接返回错误
/// 2. List Drive 上 `device.<self_id>.*`（**不含** tombstone 本身），逐个 [`drive::delete`]
///    （404 视为成功）
/// 3. 上传新 tombstone `device.<self_id>.tombstone.json`，记录 `clearedAt = now()`，
///    对端 pull → [`merge_tombstone`] → DELETE 对端的 pre-clearedAt mirror 行
/// 4. **同款 trim 应用到源端本地**：
///    `DELETE FROM activities WHERE device_id = <self> AND updated_at < clearedAt`
///    保证源端本地跟对端最终看到的一致（不留 pre-T 数据让下一次 push 重写回 Drive）
/// 5. 清 sync_outbox（防 step 3 上传 tombstone 前累积的旧 outbox 行下个 tick push 把
///    步骤 4 删的行又造回 Drive；clearedAt 之后新 capture 自然产生新 outbox 行）
/// 6. 重置 `drive_files` pull 游标
///
/// 返回被实际删除的 Drive 文件数（不含 tombstone 上传 / 本机 DELETE）。
///
/// 幂等：连点 N 次，每次 clearedAt 更新到当下 now：
///   - Drive list 返回 0 个（除 tombstone），无 DELETE 请求
///   - 上传 tombstone 覆盖同名文件，modifiedTime 刷新让对端再次 pull 应用最新 clearedAt
///   - 本机 trim 命中 0 行（除非两次点击之间有新 capture，那些是用户自己点的"清掉过去"
///     新累积部分，符合"清空过去"语义）
///   - outbox / cursor 已经是清空 / epoch 状态，UPDATE / DELETE no-op
#[tauri::command]
pub async fn purge_cloud_data(
    pool: State<'_, DbPool>,
    engine: State<'_, Arc<SyncEngine>>,
    keep_local: bool,
) -> Result<u64, String> {
    purge_cloud_data_impl(&pool, &engine, keep_local).await
}

/// 抽出来的实际实现，给集成测试可以直接调用（绕开 Tauri State<> 包装）。
///
/// `keep_local`：
/// - `false`（默认 / 推荐）：对称语义，本机也按同款 clearedAt trim 旧数据，源端 / 对端 / Drive 三处一致。
///   适用：离职 / 卖机器 / 永久删除本设备贡献。
/// - `true`：仅删 Drive + 上传 tombstone + 通知对端清，**本机数据完整保留**。
///   适用：换 Google 账号 —— 撤回当前账号云端后退出登录、登入新账号、自动 push 本机数据到新账号。
pub(crate) async fn purge_cloud_data_impl(
    pool: &DbPool,
    engine: &SyncEngine,
    keep_local: bool,
) -> Result<u64, String> {
    let self_id = engine.self_id();
    if self_id.is_empty() {
        return Err("self_id 未初始化".into());
    }

    // 没登录时（无 OAuth token）：云端步骤无意义，但用户依然期望"移除本设备"按钮
    // 至少把本机数据清干净 —— 直接降级到 [`purge_activities_impl`]（同款 7 张表
    // DELETE + VACUUM + 清 icon cache）。`keep_local` 在这条路径下被忽略，因为
    // "保留本机数据等下次换账号 push" 这个语义只在登录态下成立。
    let token = match crate::sync::auth::ensure_valid_token(pool).await {
        Ok(t) => t,
        Err(_) => {
            log::info!(
                "purge_cloud_data: 未登录，跳过云端步骤，降级到本机彻底清理 (purge_activities_impl)"
            );
            purge_activities_impl(pool).await?;
            return Ok(0);
        }
    };

    let prefix = format!("device.{self_id}.");
    let drive = engine.drive();

    // 挡住并发 push/pull：清理进行到一半时后台 tick 把刚删的文件重新传回 Drive
    // （"复活"）或用半清状态覆盖，全流程持串行门。
    let _gate = engine.pause_flushes().await;

    // 1. **先上传 tombstone**（覆盖任何旧版本，modifiedTime 刷新让对端 pull 看到）。
    //    顺序是关键：tombstone 是"对端请清掉这台设备镜像"的唯一信号——若像旧实现
    //    那样最后才传、失败还只 warn，就会出现"Drive 文件删了、本地清了、界面报成功，
    //    但对端永远不知道"的静默半成功。现在 tombstone 失败 = 整个命令失败，
    //    此时什么都还没删，用户直接重试即可。
    let cleared_at = utc_now_rfc3339();
    let tombstone_name = format!("device.{self_id}.tombstone.json");
    let tombstone_payload = serde_json::to_vec(&crate::sync::payload::TombstonePayload {
        cleared_at: cleared_at.clone(),
    })
    .map_err(|e| e.to_string())?;
    drive
        .upsert_by_name(&token.access_token, &tombstone_name, &tombstone_payload)
        .await
        .map_err(|e| format!("上传 tombstone 失败（云端未动，请重试）: {e}"))?;

    // 2. 列 Drive 全量文件，按本机 prefix 过滤；跳过 tombstone 本身（留着当 marker）。
    let files = drive
        .list_appdata_files(&token.access_token, "")
        .await
        .map_err(|e| e.to_string())?;
    let mine: Vec<_> = files
        .iter()
        .filter(|f| f.name.starts_with(&prefix) && f.name != tombstone_name)
        .collect();

    // 3. 逐个 DELETE；单文件失败不抛，让能删的尽量删完（漏删的对端也会因
    //    tombstone 的 clearedAt trim 掉，只是 Drive 上多占点空间）
    let mut deleted = 0u64;
    for f in &mine {
        match drive.delete(&token.access_token, &f.id).await {
            Ok(()) => deleted += 1,
            Err(e) => log::warn!("purge_cloud_data: delete {} 失败: {e}", f.name),
        }
    }

    // 4. 源端本地按同款 clearedAt trim activities + 5. 清 outbox + 6. 重置 cursor，
    //    打包在一个 pool.0.call 里事务性执行。keep_local=true 时跳过 step 4 + 5 的 outbox 清，
    //    保留所有本机数据 + outbox（"换 Google 账号"场景：用户接下来要登入新账号、自动 push
    //    本机数据到新账号 appDataFolder，需要 outbox 行触发）。
    let self_id_owned = self_id.to_string();
    let cleared_at_for_db = cleared_at.clone();
    pool.0
        .call(move |conn| {
            if !keep_local {
                // Step 4: 源端 self-trim —— 跟对端 pull 应用 tombstone 时的 DELETE 完全一致。
                // 这一刀确保下次 push tick 把 build_activities_day 全表重写到 Drive 时，
                // pre-clearedAt 的行**不在源端本地了**，不会被 push 回到 Drive，
                // 跟 tombstone 通知对端的语义保持对称。
                conn.execute(
                    "DELETE FROM activities
                     WHERE device_id = ?1 AND updated_at < ?2",
                    rusqlite::params![self_id_owned, cleared_at_for_db],
                )
                .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;

                // Step 5: 清 outbox（已被 trim 的行对应的 outbox 行不再有意义）
                conn.execute("DELETE FROM sync_outbox", [])
                    .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
            }

            // Step 6: 总是重置 pull cursor —— 不论保留本地与否：
            // - keep_local=false 时：tombstone 上传后立刻 pull 一下能拉到自己的 tombstone（无副作用）
            // - keep_local=true  时：换账号后重置游标确保从新账号 appDataFolder 全量 pull
            conn.execute(
                "UPDATE sync_cursor SET last_pulled_at = '1970-01-01T00:00:00Z'
                 WHERE entity = 'drive_files'",
                [],
            )
            .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?;

    Ok(deleted)
}

/// 从云端永久移除一台已经不在自己手里的远端设备。
///
/// 跟 [`purge_cloud_data`] 的区别：
/// - `purge_cloud_data` 在 **被注销的那台机器** 上跑，清的是 self 的数据
/// - `forget_remote_device` 在 **任何还活着的机器** 上跑，按 device_id 清掉别人留下的孤儿数据
///
/// 用途：用户把那台 MacbookAir / 旧 ThinkPad 卖了 / 摔了 / 重装系统了，没机会从那台机器
/// 主动调 `purge_cloud_data` —— 现在可以在任意机器上从设备页面把它清出去。
///
/// 流程对称镜像 `purge_cloud_data`：
/// 1. 列 Drive 上所有 `device.<target_id>.*` 文件（除 tombstone）
/// 2. 逐个 DELETE
/// 3. 上传 `device.<target_id>.tombstone.json` —— 让其它机器 pull 后也清这台设备的活动
/// 4. 本机事务：DELETE activities + UPDATE devices SET deleted_at = now
///
/// 返回 Drive 上被删除的文件数（不含 tombstone）。
///
/// 安全约束：
/// - 必须已登录（云端步骤无法绕过；无登录直接返错，因为不删云端就等于啥也没做）
/// - 拒绝 target_id == self_id —— 让用户走 `purge_cloud_data` 那条带"保留本机"语义的路径
#[tauri::command]
pub async fn forget_remote_device(
    pool: State<'_, DbPool>,
    engine: State<'_, Arc<SyncEngine>>,
    mem: State<'_, crate::commands::screen_memory::MemoryState>,
    device_id: String,
) -> Result<u64, String> {
    let deleted = forget_remote_device_impl(&pool, &engine, &device_id).await?;
    // 记忆库:清掉从该设备同步来的屏幕记忆会话(FTS 触发器自动跟删)。
    // best-effort:记忆库不可用只记日志,不影响主流程结果。
    if let Some(db) = &mem.0 {
        let target = device_id.trim().to_string();
        let res =
            db.0.call(move |conn| {
                conn.execute(
                    "DELETE FROM text_sessions WHERE origin_device = ?1",
                    rusqlite::params![target],
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)
            })
            .await;
        match res {
            Ok(n) => log::info!("forget_remote_device: 记忆库清掉 {n} 条远端会话"),
            Err(e) => log::warn!("forget_remote_device: 记忆库清理失败: {e}"),
        }
    }
    Ok(deleted)
}

pub(crate) async fn forget_remote_device_impl(
    pool: &DbPool,
    engine: &SyncEngine,
    target_id: &str,
) -> Result<u64, String> {
    let target_id = target_id.trim();
    if target_id.is_empty() {
        return Err("device_id 不能为空".into());
    }

    let self_id = engine.self_id();
    if self_id == target_id {
        return Err("不能用 forget_remote_device 清自己，请用 purge_cloud_data".into());
    }

    // 没登录直接拒绝 —— 不能只动本机不动云端：那样下次 pull 会把刚清的设备又拉回来
    let token = crate::sync::auth::ensure_valid_token(pool)
        .await
        .map_err(|e| format!("需要登录后才能从云端移除远端设备：{e}"))?;

    let prefix = format!("device.{target_id}.");
    let tombstone_name = format!("device.{target_id}.tombstone.json");
    let drive = engine.drive();

    // 挡住并发 push/pull（详见 purge_cloud_data_impl 同位置注释）
    let _gate = engine.pause_flushes().await;

    // 1. **先上传 tombstone**（覆盖任何旧版本）。其它机器 pull 后按 cleared_at trim
    //    activities + mark devices.deleted_at。顺序同 purge_cloud_data_impl：tombstone
    //    是对端清镜像的唯一信号，失败必须让整个命令失败（此时云端和本地都还没动，
    //    重试即可），不能只 warn 然后照常删文件清本地——那是"界面报成功、对端永远
    //    留着这台设备数据"的静默半成功。
    let cleared_at = utc_now_rfc3339();
    let tombstone_payload = serde_json::to_vec(&crate::sync::payload::TombstonePayload {
        cleared_at: cleared_at.clone(),
    })
    .map_err(|e| e.to_string())?;
    drive
        .upsert_by_name(&token.access_token, &tombstone_name, &tombstone_payload)
        .await
        .map_err(|e| format!("上传 tombstone 失败（云端未动，请重试）: {e}"))?;

    // 2. 列 Drive 上属于该设备的所有文件（跳过 tombstone 本身：留下当 marker）
    let files = drive
        .list_appdata_files(&token.access_token, "")
        .await
        .map_err(|e| e.to_string())?;
    let target_files: Vec<_> = files
        .iter()
        .filter(|f| f.name.starts_with(&prefix) && f.name != tombstone_name)
        .collect();

    // 3. 逐个 DELETE；单文件失败不抛，让能删的尽量删完（漏删的对端也会被 tombstone trim）
    let mut deleted = 0u64;
    for f in &target_files {
        match drive.delete(&token.access_token, &f.id).await {
            Ok(()) => deleted += 1,
            Err(e) => log::warn!("forget_remote_device: delete {} 失败: {e}", f.name),
        }
    }

    // 4. 本机：删活动 + 软删设备。事务保证两步原子。
    let target_owned = target_id.to_string();
    let cleared_at_for_db = cleared_at.clone();
    pool.0
        .call(move |conn| {
            conn.execute(
                "DELETE FROM activities WHERE device_id = ?1",
                rusqlite::params![target_owned],
            )
            .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
            conn.execute(
                "UPDATE devices
                 SET deleted_at = ?2, updated_at = ?2
                 WHERE device_id = ?1",
                rusqlite::params![target_owned, cleared_at_for_db],
            )
            .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?;

    log::info!(
        "forget_remote_device: device={target_id} drive_deleted={deleted} files (tombstone={cleared_at})"
    );

    Ok(deleted)
}

/// 返回当前 data_root（DB / 截图等数据的根目录）。前端「设置 → 数据」面板显示用。
#[tauri::command]
pub fn get_data_root() -> String {
    crate::bootstrap::data_root().to_string_lossy().to_string()
}

/// 写入新的 data_root 路径到 bootstrap.json。
///
/// **不会**自动迁移已有数据——下次启动后才会读到新路径打开新 DB；老数据需用户手动复制。
/// 设计权衡：自动迁移失败时会把数据卡半路，用户损失更难恢复，故只改指针。
#[tauri::command]
pub fn set_data_root(path: String) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("路径不能为空".into());
    }
    // 拒绝相对路径——下次启动 dirs::data_dir() fallback 不会触发，
    // 进程会从 cwd 解析这个相对路径，对用户极反直觉
    if !std::path::Path::new(trimmed).is_absolute() {
        return Err("数据目录必须是绝对路径".into());
    }
    crate::bootstrap::set_data_root(trimmed).map_err(|e| e.to_string())
}

/// 在系统文件管理器里打开截图目录。`open_in_file_manager` 是阻塞的同步调用，
/// 走 spawn_blocking 不堵 runtime。
#[tauri::command]
pub async fn open_screenshots_dir(pool: State<'_, DbPool>) -> Result<(), String> {
    let cfg = settings::load(&pool).await.map_err(String::from)?;
    if cfg.screenshot_path.trim().is_empty() {
        return Err("截图路径未设置".into());
    }
    let path = std::path::PathBuf::from(&cfg.screenshot_path);
    tokio::fs::create_dir_all(&path)
        .await
        .map_err(|e| e.to_string())?;

    let path_clone = path.clone();
    tokio::task::spawn_blocking(move || {
        crate::platform::open_in_file_manager(&path_clone).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(())
}

/// 删除截图目录下所有文件 + 把 activities.screenshot_path 全置 NULL。
///
/// 文件删除是 best-effort：单个文件删失败 log warn 继续，不阻塞整体。
/// DB 行的 path 引用先清，即使物理文件删除失败也不会下次反复尝试。
#[tauri::command]
pub async fn purge_screenshots(pool: State<'_, DbPool>) -> Result<(), String> {
    let cfg = settings::load(&pool).await.map_err(String::from)?;
    if cfg.screenshot_path.trim().is_empty() {
        return Err("截图路径未设置".into());
    }
    let dir = std::path::PathBuf::from(&cfg.screenshot_path);

    pool.0
        .call(|conn| {
            conn.execute(
                "UPDATE activities SET screenshot_path = NULL WHERE screenshot_path IS NOT NULL",
                [],
            )
            .db()?;
            // 截图文件即将被删，member/rep 映射全部失去指向，一并清
            conn.execute("DELETE FROM screenshot_dedup_map", []).db()?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?;

    tokio::task::spawn_blocking(move || -> Result<(), String> {
        if !dir.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(&dir)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            let path = entry.path();
            let res = if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            if let Err(e) = res {
                log::warn!("删除截图失败 {}: {}", path.display(), e);
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(())
}

/// 递归统计目录下所有文件字节数（含子目录），失败的子节点跳过。
fn dir_size(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        let entries = match std::fs::read_dir(&p) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

/// 把任意文本写到指定绝对路径。前端导出 markdown / json 文件时调——
/// Tauri webview 不支持浏览器原生 `<a download>` 自动落盘（点了静默失败 / 用户找不到文件），
/// 必须由后端调 std::fs 写。
///
/// 路径校验：拒绝相对路径（避免相对当前进程 cwd 落到诡异位置）；不限制目标目录
/// （前端通过 Tauri save dialog 拿到路径，已是用户主动选的）。
#[tauri::command]
pub async fn write_text_file(path: String, content: String) -> Result<(), String> {
    let p = std::path::PathBuf::from(&path);
    if !p.is_absolute() {
        return Err(format!("路径必须是绝对路径：{path}"));
    }
    tokio::task::spawn_blocking(move || std::fs::write(&p, content))
        .await
        .map_err(|e| format!("spawn_blocking 失败：{e}"))?
        .map_err(|e| format!("写文件失败 {path}：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::fresh_test_pool;

    /// 取 SQLite 文件逻辑大小（in-memory 也能用）：`page_count * page_size`。
    async fn db_logical_bytes(pool: &DbPool) -> u64 {
        pool.0
            .call(|conn| {
                let pages: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
                let size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
                Ok((pages.max(0) as u64) * (size.max(0) as u64))
            })
            .await
            .unwrap()
    }

    async fn count(pool: &DbPool, table: &'static str) -> i64 {
        pool.0
            .call(move |conn| {
                let sql = format!("SELECT COUNT(*) FROM \"{table}\"");
                let n: i64 = conn.query_row(&sql, [], |r| r.get(0))?;
                Ok(n)
            })
            .await
            .unwrap()
    }

    /// 7 张派生表全清 + sync_cursor 重置 + 用户自定义保留 + VACUUM 真的把 page_count 缩了。
    ///
    /// fixture 用 1 行 / 表 + 1 个 ~512KB 的 app_icons BLOB 把 DB 撑大几百页；
    /// 这样 VACUUM 后 page_count 显著下降，断言才有意义（小 DB 时 VACUUM 可能维持
    /// 同样 page 数，看不出效果）。
    #[tokio::test]
    async fn purge_activities_impl_clears_derived_tables_keeps_user_data_and_shrinks_db() {
        let pool = fresh_test_pool().await;
        let self_id = crate::device::self_id().unwrap().to_string();

        // ── seed 7 张目标表 ──
        let big_blob = vec![0xABu8; 512 * 1024]; // 512KB，撑出几十页
        pool.0
            .call({
                let self_id = self_id.clone();
                move |conn| {
                    // activities
                    conn.execute(
                        "INSERT INTO activities(
                            started_at, ended_at, duration_secs, local_date, local_hour,
                            process_name, window_title, category_id, device_id, updated_at, origin
                         ) VALUES('2026-05-17T10:00:00Z','2026-05-17T10:00:30Z',30,
                                  '2026-05-17',10,'TestApp','t','other',?1,
                                  '2026-05-17T10:00:30Z','local')",
                        rusqlite::params![self_id],
                    )?;
                    // process_paths
                    conn.execute(
                        "INSERT INTO process_paths(process_name, exe_path, seen_at)
                         VALUES('TestApp','/Applications/TestApp.app','2026-05-17T10:00:00Z')",
                        [],
                    )?;
                    // app_icons —— 大 BLOB 撑空间
                    conn.execute(
                        "INSERT INTO app_icons(process_name, icon_png, updated_at)
                         VALUES('TestApp', ?1, '2026-05-17T10:00:00Z')",
                        rusqlite::params![big_blob],
                    )?;
                    // ai_summaries（PK: source + local_date + segment_idx）
                    conn.execute(
                        "INSERT INTO ai_summaries(source, local_date, segment_idx, label,
                            start_hour, end_hour, content, model, status, generated_at)
                         VALUES('daily','2026-05-17',0,'morning',9,12,'content','m','ok',
                                '2026-05-17T12:00:00Z')",
                        [],
                    )?;
                    // ai_image_descriptions（PK: source + date + seg + image_index）
                    conn.execute(
                        "INSERT INTO ai_image_descriptions(source, local_date, segment_idx,
                            image_index, screenshot_path, description, model, generated_at)
                         VALUES('daily','2026-05-17',0,0,'/p.jpg','d','m','2026-05-17T12:00:00Z')",
                        [],
                    )?;
                    // screenshot_embeddings
                    conn.execute(
                        "INSERT INTO screenshot_embeddings(screenshot_path, model_id, dim, embedding)
                         VALUES('/p.jpg','mobilenet_v3',1280, ?1)",
                        rusqlite::params![vec![0u8; 1280 * 4]],
                    )?;
                    // sync_outbox
                    conn.execute(
                        "INSERT INTO sync_outbox(op, entity, entity_pk, payload,
                            created_at, attempts, next_retry_at)
                         VALUES('upsert','activity','1','{}','2026-05-17T10:00:00Z',0,
                                '2026-05-17T10:00:00Z')",
                        [],
                    )?;
                    // sync_cursor 写一个非 epoch 的 cursor 验证被重置
                    conn.execute(
                        "INSERT OR REPLACE INTO sync_cursor(entity, last_pulled_at)
                         VALUES('drive_files','2026-05-17T10:00:00Z')",
                        [],
                    )?;
                    Ok(())
                }
            })
            .await
            .unwrap();

        // 自定义数据：fresh_test_pool 已经 seed 了 builtin categories；额外加一个
        // app_groups + app_group_member 模拟用户已用过的组。purge 后这条 group
        // 会变 phantom（活动清空 → member 无活动 → group 无 active 成员）→ 软删，
        // 同时给 sync_outbox 入队让对端收敛。
        pool.0
            .call(|conn| {
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at)
                     VALUES('UserGroup','User Group','other','2026-05-17T10:00:00Z')",
                    [],
                )?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at)
                     VALUES('TestApp','UserGroup','2026-05-17T10:00:00Z')",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        // ── 记录 before ──
        let bytes_before = db_logical_bytes(&pool).await;
        let categories_before = count(&pool, "categories").await;
        let app_groups_before = count(&pool, "app_groups").await;
        let app_group_members_before = count(&pool, "app_group_members").await;
        let settings_before = count(&pool, "settings_store").await;
        assert!(
            bytes_before > 400_000,
            "fixture 应当至少 400KB: got {bytes_before}"
        );
        assert!(categories_before > 0, "builtin categories 应该已 seed");

        // ── act ──
        purge_activities_impl(&pool).await.unwrap();

        // ── assert: 6 张硬删表全空（sync_outbox 单独看：被清后又被 phantom 软删
        //    enqueue 入队，所以不再为 0） ──
        for table in [
            "activities",
            "process_paths",
            "app_icons",
            "ai_image_descriptions",
            "ai_summaries",
            "screenshot_embeddings",
        ] {
            assert_eq!(count(&pool, table).await, 0, "{table} 应该被清空");
        }

        // ── assert: 用户其它自定义未动（不再含 app_groups） ──
        assert_eq!(
            count(&pool, "categories").await,
            categories_before,
            "categories 不应被动"
        );
        assert_eq!(
            count(&pool, "settings_store").await,
            settings_before,
            "settings_store 不应被动"
        );

        // ── assert: app_groups + app_group_members 物理行还在（软删保留行让 LWW
        //    跨设备 merge 能识别 tombstone），但 deleted_at 已置位 ──
        assert_eq!(
            count(&pool, "app_groups").await,
            app_groups_before,
            "app_groups 行数不变（软删保留物理行）",
        );
        assert_eq!(
            count(&pool, "app_group_members").await,
            app_group_members_before,
            "app_group_members 行数不变（软删保留物理行）",
        );
        let active_groups: i64 = pool
            .0
            .call(|conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM app_groups WHERE deleted_at IS NULL",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        let active_members: i64 = pool
            .0
            .call(|conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM app_group_members WHERE deleted_at IS NULL",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(active_groups, 0, "phantom app_groups 应已全部软删");
        assert_eq!(active_members, 0, "phantom app_group_members 应已全部软删");

        // ── assert: sync_outbox 含两条软删 enqueue（1 个 group + 1 个 member） ──
        let outbox_entities: Vec<(String, String, String)> = pool
            .0
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT op, entity, entity_pk FROM sync_outbox ORDER BY entity, entity_pk",
                )?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
            .unwrap();
        assert_eq!(
            outbox_entities,
            vec![
                ("upsert".into(), "app_group".into(), "UserGroup".into()),
                ("upsert".into(), "app_group_member".into(), "TestApp".into()),
            ],
            "sync_outbox 应仅含 phantom 软删的 outbox 行",
        );

        // ── assert: sync_cursor 重置到 epoch ──
        let cursor: String = pool
            .0
            .call(|conn| {
                Ok(conn.query_row(
                    "SELECT last_pulled_at FROM sync_cursor WHERE entity='drive_files'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(
            cursor, "1970-01-01T00:00:00Z",
            "drive_files cursor 应重置到 epoch"
        );

        // ── assert: VACUUM 真的把页数压回去了 ──
        let bytes_after = db_logical_bytes(&pool).await;
        assert!(
            bytes_after < bytes_before,
            "VACUUM 后逻辑 DB 应明显缩水: before={bytes_before} after={bytes_after}",
        );

        // ── 幂等 ──：再跑一次不出错；6 张表仍为空；
        //    sync_outbox 这次回到 0（无 active phantom 可软删，无 enqueue）
        purge_activities_impl(&pool).await.unwrap();
        for table in [
            "activities",
            "process_paths",
            "app_icons",
            "ai_image_descriptions",
            "ai_summaries",
            "screenshot_embeddings",
            "sync_outbox",
        ] {
            assert_eq!(count(&pool, table).await, 0, "二次 purge 后 {table} 应为 0");
        }
    }

    // ═════════════ forget_remote_device_impl（桩照抄 e2e 的 InMemoryDriveStore 用法）═════════════

    use crate::sync::drive::{DriveBackend, InMemoryDriveStore};

    /// e2e 同款 fake auth：四列全 Some + expires_at 远未来，让
    /// `ensure_valid_token` 走"未过期直接复用"分支，零网络调用。
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

    /// InMemory Drive + 显式 self_id 的引擎（不 start，无后台 tick）。
    fn make_engine(pool: &DbPool, self_id: &str, drive: Arc<InMemoryDriveStore>) -> SyncEngine {
        SyncEngine::with_backend(
            pool.clone(),
            None,
            DriveBackend::InMemory(drive),
            self_id.to_string(),
        )
    }

    async fn insert_activity_for(pool: &DbPool, device_id: &str, process: &str) {
        let device_id = device_id.to_string();
        let process = process.to_string();
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activities(
                        started_at, ended_at, duration_secs, local_date, local_hour,
                        process_name, window_title, category_id, device_id, updated_at, origin
                     ) VALUES('2026-05-17T10:00:00Z','2026-05-17T10:00:30Z',30,
                              '2026-05-17',10,?1,'t','other',?2,
                              '2026-05-17T10:00:30Z','local')",
                    rusqlite::params![process, device_id],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn insert_device_row(pool: &DbPool, device_id: &str) {
        let device_id = device_id.to_string();
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO devices(device_id, display_name, os, updated_at)
                     VALUES(?1, ?1, 'macos', '2026-05-17T10:00:00Z')",
                    rusqlite::params![device_id],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn activities_for(pool: &DbPool, device_id: &str) -> i64 {
        let device_id = device_id.to_string();
        pool.0
            .call(move |conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM activities WHERE device_id = ?1",
                    rusqlite::params![device_id],
                    |r| r.get(0),
                )
                .db()
            })
            .await
            .unwrap()
    }

    async fn device_deleted_at(pool: &DbPool, device_id: &str) -> Option<String> {
        let device_id = device_id.to_string();
        pool.0
            .call(move |conn| {
                conn.query_row(
                    "SELECT deleted_at FROM devices WHERE device_id = ?1",
                    rusqlite::params![device_id],
                    |r| r.get(0),
                )
                .db()
            })
            .await
            .unwrap()
    }

    /// Drive 上现存文件名（升序），断言"哪些活着"用。
    async fn drive_names(store: &InMemoryDriveStore) -> Vec<String> {
        let mut names: Vec<String> = store
            .list_appdata_files("")
            .await
            .unwrap()
            .into_iter()
            .map(|f| f.name)
            .collect();
        names.sort();
        names
    }

    async fn drive_content(store: &InMemoryDriveStore, name: &str) -> Vec<u8> {
        let id = store
            .list_appdata_files("")
            .await
            .unwrap()
            .into_iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("Drive 上应存在 {name}"))
            .id;
        store.download(&id).await.unwrap()
    }

    /// 前置校验：空 / 全空白 id 拒绝；target == self（含 trim 后相等）拒绝并指路
    /// purge_cloud_data。两类拒绝都发生在任何云端/本地写入之前。
    #[tokio::test]
    async fn forget_remote_device_rejects_blank_and_self() {
        let pool = fresh_test_pool().await;
        let drive = Arc::new(InMemoryDriveStore::new());
        let engine = make_engine(&pool, "self-dev", drive.clone());

        for blank in ["", "   ", "\t\n"] {
            let err = forget_remote_device_impl(&pool, &engine, blank)
                .await
                .unwrap_err();
            assert!(err.contains("不能为空"), "blank={blank:?} err={err}");
        }
        for selfish in ["self-dev", "  self-dev  "] {
            let err = forget_remote_device_impl(&pool, &engine, selfish)
                .await
                .unwrap_err();
            assert!(err.contains("purge_cloud_data"), "id={selfish:?} err={err}");
        }
        // 拒绝路径不应产生任何云端写入（tombstone 也不能传）
        assert!(drive_names(&drive).await.is_empty());
    }

    /// 未登录直接拒绝——不能只动本机不动云端（下次 pull 会把设备拉回来）。
    /// 云端文件原样、无 tombstone、本地表不动。
    #[tokio::test]
    async fn forget_remote_device_requires_login() {
        let pool = fresh_test_pool().await; // 不注 fake auth → NotSignedIn
        let drive = Arc::new(InMemoryDriveStore::new());
        drive
            .upsert_by_name("device.ghost.data.2026-05-01.ndjson", b"d")
            .await
            .unwrap();
        insert_device_row(&pool, "ghost").await;
        insert_activity_for(&pool, "ghost", "Code").await;
        let engine = make_engine(&pool, "self-dev", drive.clone());

        let err = forget_remote_device_impl(&pool, &engine, "ghost")
            .await
            .unwrap_err();
        assert!(err.contains("需要登录"), "err={err}");

        assert_eq!(
            drive_names(&drive).await,
            vec!["device.ghost.data.2026-05-01.ndjson"],
            "未登录路径不得动云端"
        );
        assert_eq!(activities_for(&pool, "ghost").await, 1, "本地表不得动");
        assert_eq!(device_deleted_at(&pool, "ghost").await, None);
    }

    /// tombstone 先行：上传失败 = 整个命令失败，此时云端文件、本地 activities、
    /// devices 全部原样（用户重试即可）。注入配额耗尽后重试立即成功。
    #[tokio::test]
    async fn forget_remote_device_tombstone_failure_fails_whole_command() {
        let pool = fresh_test_pool().await;
        inject_fake_auth(&pool).await;
        let drive = Arc::new(InMemoryDriveStore::new());
        drive
            .upsert_by_name("device.ghost.data.2026-05-01.ndjson", b"d")
            .await
            .unwrap();
        drive
            .upsert_by_name("device.ghost.meta.json", b"m")
            .await
            .unwrap();
        insert_device_row(&pool, "ghost").await;
        insert_activity_for(&pool, "ghost", "Code").await;
        let engine = make_engine(&pool, "self-dev", drive.clone());

        drive.fail_next_upserts(1); // 第一步 tombstone 上传即 500

        let err = forget_remote_device_impl(&pool, &engine, "ghost")
            .await
            .unwrap_err();
        assert!(err.contains("上传 tombstone 失败"), "err={err}");

        // 整体失败 = 什么都没动
        assert_eq!(
            drive_names(&drive).await,
            vec![
                "device.ghost.data.2026-05-01.ndjson",
                "device.ghost.meta.json"
            ],
            "tombstone 失败后不得删任何 Drive 文件"
        );
        assert_eq!(activities_for(&pool, "ghost").await, 1);
        assert_eq!(device_deleted_at(&pool, "ghost").await, None);

        // 瞬时故障过去后直接重试成功（deleted 只计 data+meta）
        let deleted = forget_remote_device_impl(&pool, &engine, "ghost")
            .await
            .unwrap();
        assert_eq!(deleted, 2);
    }

    /// 完整流程：按 `device.<id>.` 前缀删 Drive 文件（tombstone 本身不删不计数、
    /// 前缀陷阱 `device.<id>-2.` 不误伤、邻居设备不动）+ 新 tombstone 落盘 +
    /// 本地 activities 删除 + devices 软删（deleted_at = tombstone 的 clearedAt）。
    #[tokio::test]
    async fn forget_remote_device_full_flow_cleans_drive_and_local() {
        let pool = fresh_test_pool().await;
        inject_fake_auth(&pool).await;
        let drive = Arc::new(InMemoryDriveStore::new());
        // 目标设备：两个数据文件 + 一个旧 tombstone（会被覆盖，不计入 deleted）
        drive
            .upsert_by_name("device.ghost.data.2026-05-01.ndjson", b"d1")
            .await
            .unwrap();
        drive
            .upsert_by_name("device.ghost.meta.json", b"m")
            .await
            .unwrap();
        drive
            .upsert_by_name("device.ghost.tombstone.json", b"old-tombstone")
            .await
            .unwrap();
        // 邻居设备 + 前缀陷阱："device.ghost-2." 不以 "device.ghost." 开头，不得误删
        drive
            .upsert_by_name("device.alive.data.2026-05-01.ndjson", b"a")
            .await
            .unwrap();
        drive
            .upsert_by_name("device.ghost-2.meta.json", b"trap")
            .await
            .unwrap();

        insert_device_row(&pool, "ghost").await;
        insert_device_row(&pool, "alive").await;
        insert_activity_for(&pool, "ghost", "Code").await;
        insert_activity_for(&pool, "ghost", "Slack").await;
        insert_activity_for(&pool, "alive", "Chrome").await;
        insert_activity_for(&pool, "self-dev", "Terminal").await;

        let engine = make_engine(&pool, "self-dev", drive.clone());
        let deleted = forget_remote_device_impl(&pool, &engine, "ghost")
            .await
            .unwrap();
        assert_eq!(deleted, 2, "只计 data+meta，tombstone 不计");

        // Drive：目标数据文件没了；tombstone 是新 payload；邻居 + 陷阱原样
        assert_eq!(
            drive_names(&drive).await,
            vec![
                "device.alive.data.2026-05-01.ndjson",
                "device.ghost-2.meta.json",
                "device.ghost.tombstone.json",
            ]
        );
        let body = drive_content(&drive, "device.ghost.tombstone.json").await;
        let ts: crate::sync::payload::TombstonePayload =
            serde_json::from_slice(&body).expect("tombstone 应是合法 TombstonePayload JSON");
        chrono::DateTime::parse_from_rfc3339(&ts.cleared_at).expect("clearedAt 应是 RFC3339");

        // 本地：目标 activities 全删；其它设备 / self 保留；devices 软删且
        // deleted_at 精确等于 tombstone 的 clearedAt（同一时刻取值）
        assert_eq!(activities_for(&pool, "ghost").await, 0);
        assert_eq!(activities_for(&pool, "alive").await, 1);
        assert_eq!(activities_for(&pool, "self-dev").await, 1);
        assert_eq!(
            device_deleted_at(&pool, "ghost").await.as_deref(),
            Some(ts.cleared_at.as_str())
        );
        assert_eq!(device_deleted_at(&pool, "alive").await, None);

        // 幂等重跑：无前缀文件可删 → Ok(0)，不报错
        let again = forget_remote_device_impl(&pool, &engine, "ghost")
            .await
            .unwrap();
        assert_eq!(again, 0);
    }

    // ═════════════ set_data_root 校验 + bootstrap.json 落盘 ═════════════

    /// 空 / 全空白 / 相对路径全部拒绝——这些分支在写 bootstrap.json 之前短路，
    /// 不触碰任何文件系统状态。
    #[test]
    fn set_data_root_rejects_blank_and_relative() {
        for blank in ["", "   ", "\n\t"] {
            let err = set_data_root(blank.to_string()).unwrap_err();
            assert!(err.contains("不能为空"), "input={blank:?} err={err}");
        }
        for rel in ["relative/path", "./x", "../up", "just-a-name"] {
            let err = set_data_root(rel.to_string()).unwrap_err();
            assert!(err.contains("绝对路径"), "input={rel:?} err={err}");
        }
    }

    /// RAII：测试结束（含断言失败 panic）恢复 bootstrap.json 原内容 / 原缺失态，
    /// 以及 `HINDSIGHT_DATA_DIR` 环境变量原值。
    struct BootstrapRestore {
        cfg: std::path::PathBuf,
        prev_file: Option<Vec<u8>>,
        dir_existed: bool,
        prev_env: Option<std::ffi::OsString>,
    }

    impl Drop for BootstrapRestore {
        fn drop(&mut self) {
            match &self.prev_file {
                Some(bytes) => {
                    let _ = std::fs::write(&self.cfg, bytes);
                }
                None => {
                    let _ = std::fs::remove_file(&self.cfg);
                    if !self.dir_existed {
                        if let Some(parent) = self.cfg.parent() {
                            let _ = std::fs::remove_dir(parent);
                        }
                    }
                }
            }
            match &self.prev_env {
                Some(v) => std::env::set_var("HINDSIGHT_DATA_DIR", v),
                None => std::env::remove_var("HINDSIGHT_DATA_DIR"),
            }
        }
    }

    /// 校验通过后 bootstrap.json 真实落盘（值为 trim 后的路径），且 data_root() /
    /// get_data_root 立即读到新值。全程持 `lock_data_dir_env` 串行（读写方跨模块），
    /// 结束后恢复用户原 bootstrap.json 与环境变量。
    #[test]
    fn set_data_root_persists_bootstrap_json_and_takes_effect() {
        let _env_lock = crate::repo::test_util::lock_data_dir_env();

        let cfg = dirs::config_dir()
            .expect("测试环境应有 config_dir")
            .join("Hindsight")
            .join("bootstrap.json");
        let prev_file = std::fs::read(&cfg).ok();
        let _restore = BootstrapRestore {
            cfg: cfg.clone(),
            prev_file: prev_file.clone(),
            dir_existed: cfg.parent().map(|p| p.exists()).unwrap_or(true),
            prev_env: std::env::var_os("HINDSIGHT_DATA_DIR"),
        };
        // data_root() 优先读环境变量；摘掉才能观察 bootstrap.json 的生效
        std::env::remove_var("HINDSIGHT_DATA_DIR");

        let target =
            std::env::temp_dir().join(format!("hindsight-data-root-{}", std::process::id()));
        let target_str = target.to_string_lossy().to_string();

        // 两端带空白：验证落盘的是 trim 后的值
        set_data_root(format!("  {target_str}  ")).unwrap();

        let body = std::fs::read_to_string(&cfg).expect("bootstrap.json 应已写出");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            v["data_path"],
            serde_json::Value::String(target_str.clone()),
            "data_path 应是 trim 后的绝对路径"
        );
        assert_eq!(
            crate::bootstrap::data_root(),
            target,
            "data_root() 应立即读到新值（env 已摘）"
        );
        assert_eq!(get_data_root(), target_str, "get_data_root 命令应同步反映");
    }

    // ═════════════ dir_size ═════════════

    /// 嵌套目录逐层求和；不存在的路径 = 0；传文件路径（read_dir 失败）= 0。
    #[test]
    fn dir_size_sums_nested_files_missing_is_zero() {
        let root = std::env::temp_dir().join(format!("hindsight-dir-size-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub").join("deep")).unwrap();
        std::fs::write(root.join("a.bin"), vec![1u8; 100]).unwrap();
        std::fs::write(root.join("sub").join("b.bin"), vec![2u8; 50]).unwrap();
        std::fs::write(root.join("sub").join("deep").join("c.bin"), vec![3u8; 7]).unwrap();

        assert_eq!(dir_size(&root), 157, "100 + 50 + 7 逐层求和");
        assert_eq!(dir_size(&root.join("nope")), 0, "不存在的路径返回 0");
        // 指向普通文件：exists 但 read_dir 失败 → 跳过 → 0（现行为：只统计目录）
        assert_eq!(dir_size(&root.join("a.bin")), 0);

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 读失败的子目录跳过不中断：0o000 的子目录 read_dir 失败，其内文件不计入，
    /// 同级可读文件照常统计。root 用户绕过权限位，直接跳过该断言。
    #[cfg(unix)]
    #[test]
    fn dir_size_skips_unreadable_subdir() {
        use std::os::unix::fs::PermissionsExt;
        // root 不受权限位约束，注入不了"读失败"
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let root =
            std::env::temp_dir().join(format!("hindsight-dir-size-locked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let locked = root.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("hidden.bin"), vec![0u8; 64]).unwrap();
        std::fs::write(root.join("open.bin"), vec![0u8; 10]).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

        let got = dir_size(&root);

        // 先恢复权限再断言，断言失败也能清理
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(got, 10, "不可读子目录整体跳过，只计可读的 open.bin");
    }
}
