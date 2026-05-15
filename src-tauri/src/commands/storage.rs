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
use crate::storage::SqliteResultExt;
use crate::storage::{db_path, DbPool};

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

/// 清空**本机**数据库（不动云端 Drive，不动 settings / categories / app_groups）。
///
/// 删除 activities + process_paths 两张表，并清掉 sync_outbox + 重置 pull 游标。
/// 后两步是必须的：
/// - 不清 outbox：下个 push tick 会按现状重写 ndjson 推到 Drive；本机现在 0 行
///   → 把 Drive 上对应天的 ndjson 写成空，**意外删除云端数据**。
/// - 不重置游标：DELETE 把 origin='remote'（对端同步过来的镜像）也清了；游标不动 →
///   下次 pull 只看 `modifiedTime > cursor` 的新文件 → 老镜像永远拉不回。重置后下次
///   pull 走全量，对端历史数据自动重新镜像回本机。
///
/// 完成后 `svc.reset_session()` 让 capture 忘记当前活跃会话（否则下一 tick 会去 UPDATE
/// 已被删除的行）。
///
/// 幂等：连续多次调用每次效果一致（DELETE 空表 / UPDATE 已是 epoch 的 cursor 都是 no-op）。
#[tauri::command]
pub async fn purge_activities(
    pool: State<'_, DbPool>,
    svc: State<'_, Arc<CaptureService>>,
) -> Result<(), String> {
    pool.0
        .call(|conn| {
            conn.execute_batch(
                "DELETE FROM activities;
                 DELETE FROM process_paths;
                 DELETE FROM sync_outbox;
                 UPDATE sync_cursor SET last_pulled_at = '1970-01-01T00:00:00Z'
                  WHERE entity = 'drive_files';",
            )
            .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?;

    svc.reset_session().await;
    Ok(())
}

/// 清空**云端**数据（不动本机 DB）：从 Google Drive 的 appDataFolder 里删掉**本机
/// 推送过的所有同步文件**（命名前缀 `device.<self_id>.*`）。其它设备贡献的文件不动。
///
/// 流程：
/// 1. 拿 OAuth token；未登录直接返回错误（前端按钮在未登录时不渲染，但兜底一下）
/// 2. List Drive 上 appDataFolder 的所有文件，过滤出 `device.<self_id>.` 前缀
/// 3. 逐个 [`drive::delete`]；404 视为成功（已被别处删过，幂等）；其它错误 log warn 继续
/// 4. 删完后清掉 sync_outbox（否则下个 push tick 会按本机现状重新生成 ndjson 上传到
///    Drive，把刚删的文件又造回来）。本机 DB 保留，但用户后续捕获的新数据会按
///    正常流程产生 outbox → push → Drive 重建。
/// 5. 同步重置 `drive_files` pull 游标，避免本机老 cursor 把对端文件也跳过；下次
///    pull 会重新拉对端贡献（自身文件已删，pull 路径 `if device_id == self_id`
///    会主动跳过，不会把自己删的东西从远端「再拉回来」）。
///
/// 返回被实际删除的文件数量（含 404 在内的尝试数），供前端 toast 展示。
///
/// 幂等：第二次调用 Drive list 返回 0 个 `device.<self_id>.*` 文件 → 不发起任何
/// DELETE 请求；本地 outbox / cursor 已经是清空 / epoch 状态，UPDATE / DELETE 都 no-op。
#[tauri::command]
pub async fn purge_cloud_data(pool: State<'_, DbPool>) -> Result<u64, String> {
    let token = crate::sync::auth::ensure_valid_token(&pool)
        .await
        .map_err(|e| e.to_string())?;
    let self_id = crate::device::self_id().map_err(|e| e.to_string())?;
    let prefix = format!("device.{self_id}.");

    // 1. 列 Drive 全量文件（appDataFolder 范围内），按本机 prefix 过滤
    let files = crate::sync::drive::list_appdata_files(&token.access_token, "")
        .await
        .map_err(|e| e.to_string())?;
    let mine: Vec<_> = files
        .iter()
        .filter(|f| f.name.starts_with(&prefix))
        .collect();

    // 2. 逐个 DELETE；单文件失败不抛，让能删的尽量删完
    let mut deleted = 0u64;
    for f in &mine {
        match crate::sync::drive::delete(&token.access_token, &f.id).await {
            Ok(()) => deleted += 1,
            Err(e) => log::warn!("purge_cloud_data: delete {} 失败: {e}", f.name),
        }
    }

    // 3. 清掉 outbox + 重置 pull 游标，避免下次 sync 把刚删的文件又重新构造上传
    pool.0
        .call(|conn| {
            conn.execute_batch(
                "DELETE FROM sync_outbox;
                 UPDATE sync_cursor SET last_pulled_at = '1970-01-01T00:00:00Z'
                  WHERE entity = 'drive_files';",
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
