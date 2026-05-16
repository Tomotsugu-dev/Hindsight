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

/// 清空**本机**所有捕获 / 派生数据（不动云端 Drive，不动用户自定义：settings /
/// categories / app_groups / app_group_members / app_categories / devices / auth_state）。
///
/// **清的表**（7 张 + 1 个 cursor 重置）：
/// - `activities` —— 焦点会话原始流水
/// - `process_paths` —— process_name → exe path 映射
/// - `app_icons` —— icon BLOB 缓存（每张 50KB～300KB，是占用大头 ← v0.6.7 之前漏了这张
///   导致用户点完按钮 sqlite 还 20+ MB）
/// - `ai_image_descriptions` —— step 1 逐图描述
/// - `ai_summaries` —— step 2 段总结
/// - `screenshot_embeddings` —— MobileNet dedup 缓存
/// - `sync_outbox` —— 必须清，下个 push tick 否则会按"现状"重写 ndjson 把对应天写成空，
///   **意外删除云端数据**
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
/// `svc.reset_session()` 让 capture 忘记当前活跃会话（否则下一 tick 会去 UPDATE 已被
/// 删除的行）。
///
/// 幂等：连续多次调用每次效果一致（DELETE 空表 / VACUUM 已紧凑过 / UPDATE 已是 epoch
/// 的 cursor / fs 删已不存在的目录 都是 no-op）。
#[tauri::command]
pub async fn purge_activities(
    pool: State<'_, DbPool>,
    svc: State<'_, Arc<CaptureService>>,
) -> Result<(), String> {
    purge_activities_impl(&pool).await?;
    svc.reset_session().await;
    Ok(())
}

/// 抽出来的实际实现，给单测可以直接调用（绕开 Tauri State<> 包装 + CaptureService
/// 在 test 里构造不便）。语义见 [`purge_activities`] doc。
pub(crate) async fn purge_activities_impl(pool: &DbPool) -> Result<(), String> {
    // Phase 1: 7 张 DELETE + cursor reset —— execute_batch 隐式 transaction 保原子性
    pool.0
        .call(|conn| {
            conn.execute_batch(
                "DELETE FROM activities;
                 DELETE FROM process_paths;
                 DELETE FROM app_icons;
                 DELETE FROM ai_image_descriptions;
                 DELETE FROM ai_summaries;
                 DELETE FROM screenshot_embeddings;
                 DELETE FROM sync_outbox;
                 UPDATE sync_cursor SET last_pulled_at = '1970-01-01T00:00:00Z'
                  WHERE entity = 'drive_files';",
            )
            .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
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

    // 1. 列 Drive 全量文件，按本机 prefix 过滤。
    //    跳过 tombstone 本身（清云端时 tombstone 留下来当 marker，不然对端永远不知道）。
    let tombstone_name = format!("device.{self_id}.tombstone.json");
    let files = drive
        .list_appdata_files(&token.access_token, "")
        .await
        .map_err(|e| e.to_string())?;
    let mine: Vec<_> = files
        .iter()
        .filter(|f| f.name.starts_with(&prefix) && f.name != tombstone_name)
        .collect();

    // 2. 逐个 DELETE；单文件失败不抛，让能删的尽量删完
    let mut deleted = 0u64;
    for f in &mine {
        match drive.delete(&token.access_token, &f.id).await {
            Ok(()) => deleted += 1,
            Err(e) => log::warn!("purge_cloud_data: delete {} 失败: {e}", f.name),
        }
    }

    // 3. 上传 tombstone（覆盖任何旧版本，modifiedTime 自然刷新让对端 pull 看到）。
    let cleared_at = utc_now_rfc3339();
    let tombstone_payload = serde_json::to_vec(&crate::sync::payload::TombstonePayload {
        cleared_at: cleared_at.clone(),
    })
    .map_err(|e| e.to_string())?;
    if let Err(e) = drive
        .upsert_by_name(&token.access_token, &tombstone_name, &tombstone_payload)
        .await
    {
        log::warn!("purge_cloud_data: upload tombstone 失败: {e}");
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
        // app_groups 行模拟用户自定义，验证 purge 不动它。
        pool.0
            .call(|conn| {
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at)
                     VALUES('UserGroup','User Group','other','2026-05-17T10:00:00Z')",
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
        let settings_before = count(&pool, "settings_store").await;
        assert!(bytes_before > 400_000, "fixture 应当至少 400KB: got {bytes_before}");
        assert!(categories_before > 0, "builtin categories 应该已 seed");

        // ── act ──
        purge_activities_impl(&pool).await.unwrap();

        // ── assert: 7 张目标表全空 ──
        for table in [
            "activities",
            "process_paths",
            "app_icons",
            "ai_image_descriptions",
            "ai_summaries",
            "screenshot_embeddings",
            "sync_outbox",
        ] {
            assert_eq!(count(&pool, table).await, 0, "{table} 应该被清空");
        }

        // ── assert: 用户自定义未动 ──
        assert_eq!(count(&pool, "categories").await, categories_before, "categories 不应被动");
        assert_eq!(count(&pool, "app_groups").await, app_groups_before, "app_groups 不应被动");
        assert_eq!(count(&pool, "settings_store").await, settings_before, "settings_store 不应被动");

        // ── assert: sync_cursor 重置到 epoch ──
        let cursor: String = pool
            .0
            .call(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT last_pulled_at FROM sync_cursor WHERE entity='drive_files'",
                        [],
                        |r| r.get(0),
                    )?)
            })
            .await
            .unwrap();
        assert_eq!(cursor, "1970-01-01T00:00:00Z", "drive_files cursor 应重置到 epoch");

        // ── assert: VACUUM 真的把页数压回去了 ──
        let bytes_after = db_logical_bytes(&pool).await;
        assert!(
            bytes_after < bytes_before,
            "VACUUM 后逻辑 DB 应明显缩水: before={bytes_before} after={bytes_after}",
        );

        // ── 幂等 ──：再跑一次不出错 + 状态不变
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
            assert_eq!(count(&pool, table).await, 0, "二次 purge 后 {table} 仍应为 0");
        }
    }
}
