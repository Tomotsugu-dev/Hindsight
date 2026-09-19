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

/// [`get_storage_info`] Command's return structure.
/// Used by the front-end "Settings → Data" panel to render current storage usage.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    /// hindsight.sqlite's size in bytes
    pub db_bytes: u64,
    /// hindsight-memory.sqlite's size in bytes (screen text and chat history)
    pub memory_db_bytes: u64,
    /// Screenshots directory's total size in bytes (including subdirectories)
    pub screenshots_bytes: u64,
    /// hindsight.sqlite's absolute path
    pub db_path: String,
    /// screenshots directory's absolute path
    pub screenshots_path: String,
}

/// Sizes and paths of the database file and the screenshots directory.
///
/// Summing the screenshots directory walks every file and can take a while,
/// so it runs on a blocking thread.
#[tauri::command]
pub async fn get_storage_info(pool: State<'_, DbPool>) -> Result<StorageInfo, String> {
    let cfg = settings::load(&pool).await.map_err(String::from)?;
    let db = db_path().map_err(String::from)?;

    let db_bytes = tokio::fs::metadata(&db).await.map(|m| m.len()).unwrap_or(0);
    let memory_db_bytes = match crate::memory::memory_db_path() {
        Ok(p) => tokio::fs::metadata(&p).await.map(|m| m.len()).unwrap_or(0),
        Err(_) => 0,
    };
    let shots_path = std::path::PathBuf::from(&cfg.screenshot_path);
    let shots_bytes = tokio::task::spawn_blocking({
        let p = shots_path.clone();
        move || dir_size(&p)
    })
    .await
    .map_err(|e| e.to_string())?;

    Ok(StorageInfo {
        db_bytes,
        memory_db_bytes,
        screenshots_bytes: shots_bytes,
        db_path: db.to_string_lossy().to_string(),
        screenshots_path: cfg.screenshot_path,
    })
}

/// "Clear data": deletes what this device captured and everything derived from
/// it. The cloud is not touched.
///
/// Left alone: categories, app groups, settings, devices, the sign-in state and
/// the pull cursor — the user's rules and the sync state.
///
/// `sync_outbox` must go too: left behind, the next push would rewrite the day
/// files from the now empty table and delete the cloud copies as well.
/// The pull cursor stays, so the cleared history is not pulled back.
/// `VACUUM` after the deletes, or the file does not shrink; it cannot run inside
/// a transaction, hence its own `call`.
/// The `icons/` cache directory goes with `app_icons`, or icons would keep
/// coming from the old files.
#[tauri::command]
pub async fn purge_local_data(
    pool: State<'_, DbPool>,
    mem: State<'_, crate::commands::screen_memory::MemoryState>,
    svc: State<'_, Arc<CaptureService>>,
    engine: State<'_, Arc<SyncEngine>>,
) -> Result<(), String> {
    // Two locks, both needed:
    // 1. pause_flushes: waits for a push or pull in flight and blocks new ones.
    //    A push that has read the outbox but not yet the tables would otherwise
    //    upload empty day files and delete the cloud copies.
    // 2. run_with_session_cleared: drops the capture loop's current session
    //    while holding its lock. Otherwise a tick could insert a new row after
    //    the DELETE and before the pointer reset, leaving a row that is never
    //    sealed.
    let _sync_guard = engine.pause_flushes().await;
    svc.run_with_session_cleared(|| async { purge_local_data_impl(&pool, mem.0.as_ref()).await })
        .await
}

/// The implementation, callable without Tauri's `State` wrappers.
///
/// The memory database is cleared too: frames, the OCR text with its index,
/// and the chat history.
pub(crate) async fn purge_local_data_impl(
    pool: &DbPool,
    mem: Option<&crate::memory::MemoryDb>,
) -> Result<(), String> {
    // Take the digest slot before deleting anything: a batch mid-way would
    // append OCR lines to sessions that no longer exist.
    let _hold = match mem {
        Some(_) => Some(
            crate::memory::digest::hold(std::time::Duration::from_secs(30))
                .await
                .ok_or_else(|| {
                    "Screen text is still being indexed; stop it and try again".to_string()
                })?,
        ),
        None => None,
    };

    // Delete all data except user's settings
    pool.0
        .call(|conn| {
            let tx = conn
                .transaction()
                .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
            // ── Delete all derived data ──
            tx.execute_batch(
                "DELETE FROM activities;
                 DELETE FROM process_paths;
                 DELETE FROM app_icons;
                 DELETE FROM ai_image_descriptions;
                 DELETE FROM ai_summaries;
                 DELETE FROM screenshot_embeddings;
                 DELETE FROM screenshot_dedup_map;
                 DELETE FROM sync_outbox;",
            )
            .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
            tx.commit()
                .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?;
    // VACUUM cannot run inside a transaction, so it gets its own call.
    pool.0
        .call(|conn| {
            conn.execute_batch("VACUUM")
                .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?;

    // Memory database: frames, OCR text (the FTS index follows by trigger), chat.
    if let Some(mem) = mem {
        mem.0
            .call(|conn| {
                let tx = conn.transaction().db()?;
                tx.execute_batch(
                    "DELETE FROM session_lines;
                     DELETE FROM text_sessions;
                     DELETE FROM frames;
                     DELETE FROM chat_messages;
                     DELETE FROM chat_conversations;",
                )
                .db()?;
                tx.commit().db()?;
                Ok(())
            })
            .await
            .map_err(|e| e.to_string())?;
        mem.0
            .call(|conn| {
                conn.execute_batch("VACUUM").db()?;
                Ok(())
            })
            .await
            .map_err(|e| e.to_string())?;
    }

    // Clear icon file cache directory (best-effort, no error if the directory doesn't exist or deletion fails)
    if let Ok(data_root) = crate::storage::db_path_dir() {
        let icons_dir = data_root.join("icons");
        let _ = tokio::fs::remove_dir_all(&icons_dir).await;
    }
    Ok(())
}

/// "Clear screenshots": deletes every file in the screenshots directory and
/// the references to them (`activities.screenshot_path`, `screenshot_dedup_map`).
///
/// The references go first, in one transaction. A file that then fails to
/// delete is logged and left behind; nothing points at it any more.
#[tauri::command]
pub async fn purge_screenshots(pool: State<'_, DbPool>) -> Result<(), String> {
    purge_screenshots_impl(&pool).await
}

/// The implementation, callable without Tauri's `State` wrappers.
pub(crate) async fn purge_screenshots_impl(pool: &DbPool) -> Result<(), String> {
    let cfg = settings::load(pool).await.map_err(String::from)?;
    if cfg.screenshot_path.trim().is_empty() {
        return Err("Screenshot path is not set".into());
    }
    let screenshot_path = std::path::PathBuf::from(&cfg.screenshot_path);

    pool.0
        .call(|conn| {
            let tx = conn.transaction().db()?;
            tx.execute(
                "UPDATE activities SET screenshot_path = NULL WHERE screenshot_path IS NOT NULL",
                [],
            )
            .db()?;
            tx.execute("DELETE FROM screenshot_dedup_map", []).db()?;
            tx.commit().db()?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?;

    tokio::task::spawn_blocking(move || -> Result<(), String> {
        if !screenshot_path.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(&screenshot_path)
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
                log::warn!("Failed to delete screenshot {}: {}", path.display(), e);
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(())
}

/// 「从云端移除本设备」：删掉本设备在云端的全部文件，只留一个墓碑
/// （`device.<self>.tombstone.json`，`clearedAt` = 此刻）。其它设备拉到墓碑后，
/// 删掉本设备的记录、把设备卡片标成已删除。最后退出登录，之后不再有东西推上去。
///
/// `keep_local` 为 false 时本机也跑一遍「清空数据」和「清空截图」；为 true 时本机不动。
/// 没登录直接返回错误。返回从云端删掉的文件数。
///
/// 顺序：先传墓碑再删文件。墓碑是其它设备知道要删的唯一信号，传失败就整个失败，
/// 此时什么都还没动，重试即可。单个文件删失败只记日志：其它设备靠墓碑照样删，
/// 只是云端多占点空间。全程持同步的门，中途不会有推送把刚删的文件传回去。
#[tauri::command]
pub async fn purge_cloud_data(
    pool: State<'_, DbPool>,
    mem: State<'_, crate::commands::screen_memory::MemoryState>,
    engine: State<'_, Arc<SyncEngine>>,
    svc: State<'_, Arc<CaptureService>>,
    keep_local: bool,
) -> Result<u64, String> {
    purge_cloud_data_impl(&pool, &engine, Some(&svc), mem.0.as_ref(), keep_local).await
}

/// 抽出来的实际实现，给测试直接调用（绕开 Tauri State<> 包装）。测试没有采集服务，`svc` 传 `None`。
pub(crate) async fn purge_cloud_data_impl(
    pool: &DbPool,
    engine: &SyncEngine,
    svc: Option<&CaptureService>,
    mem: Option<&crate::memory::MemoryDb>,
    keep_local: bool,
) -> Result<u64, String> {
    let self_id = engine.self_id();
    if self_id.is_empty() {
        return Err("self_id 未初始化".into());
    }

    let token = crate::sync::auth::ensure_valid_token(pool)
        .await
        .map_err(|e| e.to_string())?;

    let prefix = format!("device.{self_id}.");
    let drive = engine.drive();

    // Held until this function returns: a push landing mid-way would upload
    // this device's files again right after they were deleted.
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

    // 4. 「也一并清空」：跑一遍清空数据 + 清空截图。丢掉采集中的会话，
    //    否则下一 tick 会去更新已被删掉的行。
    if !keep_local {
        let clear_local = || async {
            purge_local_data_impl(pool, mem).await?;
            purge_screenshots_impl(pool).await
        };
        match svc {
            Some(svc) => svc.run_with_session_cleared(clear_local).await?,
            None => clear_local().await?,
        }
    }

    crate::sync::auth::sign_out(pool)
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

    /// 7 张派生表全清 + sync_cursor 不动 + 用户自定义保留 + VACUUM 真的把 page_count 缩了。
    ///
    /// fixture 用 1 行 / 表 + 1 个 ~512KB 的 app_icons BLOB 把 DB 撑大几百页；
    /// 这样 VACUUM 后 page_count 显著下降，断言才有意义（小 DB 时 VACUUM 可能维持
    /// 同样 page 数，看不出效果）。
    // 清空数据会删 <数据目录>/icons，所以整条测试持 env 锁、指到临时目录。
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn purge_local_data_impl_clears_derived_tables_keeps_user_data_and_shrinks_db() {
        let _env_lock = crate::repo::test_util::lock_data_dir_env();
        let _data_dir = crate::repo::test_util::DataDirOverride::unique_temp();

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
                    // sync_cursor 写一个非 epoch 的 cursor，验证清空不动它
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
        // app_groups + app_group_member 模拟用户已用过的组。purge 后必须原样保留。
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
        purge_local_data_impl(&pool, None).await.unwrap();

        // ── assert: 7 张硬删表全空 ──
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

        // ── assert: app_groups + app_group_members 原样保留，一条都没被标删除 ──
        assert_eq!(
            count(&pool, "app_groups").await,
            app_groups_before,
            "app_groups 不应被动",
        );
        assert_eq!(
            count(&pool, "app_group_members").await,
            app_group_members_before,
            "app_group_members 不应被动",
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
        assert_eq!(active_groups, app_groups_before, "app_groups 不应被标删除");
        assert_eq!(
            active_members, app_group_members_before,
            "app_group_members 不应被标删除"
        );

        // ── assert: sync_cursor 不动 ──
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
            cursor, "2026-05-17T10:00:00Z",
            "清空数据不应重置 drive_files 游标"
        );

        // ── assert: VACUUM 真的把页数压回去了 ──
        let bytes_after = db_logical_bytes(&pool).await;
        assert!(
            bytes_after < bytes_before,
            "VACUUM 后逻辑 DB 应明显缩水: before={bytes_before} after={bytes_after}",
        );

        // ── 幂等 ──：再跑一次不出错；7 张表仍为空
        purge_local_data_impl(&pool, None).await.unwrap();
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
