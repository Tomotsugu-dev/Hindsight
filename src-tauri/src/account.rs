//! 多账号支持：每个 Google 账号一份独立 DB。
//!
//! ## 文件布局
//! ```text
//! <data_root>/
//!   hindsight.sqlite                 # 匿名 / 未登录态 / 等待迁移
//!   hindsight.<google_uid_a>.sqlite  # 账号 A 的隔离 DB
//!   hindsight.<google_uid_b>.sqlite  # 账号 B 的隔离 DB
//!
//! <config_dir>/Hindsight/
//!   active_user.json                 # { uid, legacyOwner }
//!   device.json                      # 全机共享，不分账号
//!   bootstrap.json                   # 数据根路径覆盖
//! ```
//!
//! ## 切账号 = 重启 app
//! Tauri 的 `manage(pool)` 不方便热替换。换账号时只更新 `active_user.json`，
//! 提示用户重启；重启后 [`db_path`] 自动指向新 DB。
//!
//! ## 字段语义
//! - `uid`：当前激活的 Google uid。决定 `db_path()` 返回哪个 DB。
//! - `legacy_owner`：`hindsight.sqlite`（无 uid 后缀的文件）的真正归属者；
//!   存在时下次启动会把这个文件 rename 到 `hindsight.<legacy_owner>.sqlite`。
//!   为啥要单独记录：用 auth_state.uid 当 peek heuristic 的话，"sign-in 后
//!   立刻 sign-out 再退出 app" 会清掉 auth_state，下次 startup 误判成"匿名
//!   DB"，rename 不发生 → 数据丢失。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::storage::DbPool;

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActiveUserFile {
    #[serde(default)]
    uid: Option<String>,
    /// `hindsight.sqlite` 的归属者；存在则 startup 时把文件 rename 到该 uid 的路径。
    #[serde(default)]
    legacy_owner: Option<String>,
}

fn active_user_file() -> Option<PathBuf> {
    Some(
        dirs::config_dir()?
            .join("Hindsight")
            .join("active_user.json"),
    )
}

fn read_file() -> ActiveUserFile {
    let path = match active_user_file() {
        Some(p) => p,
        None => return ActiveUserFile::default(),
    };
    let s = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return ActiveUserFile::default(),
    };
    serde_json::from_str(&s).unwrap_or_default()
}

fn write_file(body: &ActiveUserFile) -> io::Result<()> {
    let path = active_user_file()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "找不到系统配置目录"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let s = serde_json::to_string_pretty(body).map_err(|e| io::Error::other(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, s)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// 当前激活的 Google uid。返回 None = 匿名/未登录态。
pub fn active_uid() -> Option<String> {
    read_file().uid.filter(|u| !u.trim().is_empty())
}

/// 写入新的 active uid。`None` 回到匿名态（仅清除标记，不删 DB 文件）。
/// 不动 `legacy_owner`：归属归属，激活归激活。
pub fn set_active_uid(uid: Option<&str>) -> io::Result<()> {
    let mut body = read_file();
    body.uid = uid.map(str::to_string);
    write_file(&body)
}

fn legacy_owner() -> Option<String> {
    read_file().legacy_owner.filter(|u| !u.trim().is_empty())
}

fn set_legacy_owner(uid: Option<&str>) -> io::Result<()> {
    let mut body = read_file();
    body.legacy_owner = uid.map(str::to_string);
    write_file(&body)
}

/// 启动时调用一次：处理「老安装升级到多账号版本」+「sign-in 后还没重启就退出 app」两种延迟迁移。
///
/// 调用时机：`device::ensure_loaded()` 之后、第一次 [`db_path`] 调用之前。
///
/// 算法：
/// 1. 如果 `active_user.json` 还没写过 + `hindsight.sqlite` 存在 + 里面 auth_state 已登录
///    → 老安装，记录 active_uid + legacy_owner（首次升级路径）。
/// 2. 如果 `legacy_owner` 有值 + `hindsight.sqlite` 存在 + 目标路径不存在
///    → 把 `hindsight.sqlite` rename 到 `hindsight.<legacy_owner>.sqlite`，清掉 legacy_owner。
pub async fn migrate_legacy_db(data_root: &Path) -> Result<()> {
    let legacy = data_root.join("hindsight.sqlite");
    let body = read_file();

    // Step 1: 老安装升级路径——active_user.json 完全没写过 + 老 DB 存在
    let needs_legacy_owner_init =
        body.uid.is_none() && body.legacy_owner.is_none() && legacy.exists();
    if needs_legacy_owner_init {
        if let Some(uid) = peek_auth_state_uid(&legacy).await {
            set_active_uid(Some(&uid))?;
            set_legacy_owner(Some(&uid))?;
            log::info!("老版本升级：active_uid={uid}, 待 rename hindsight.sqlite");
        }
        // 若 peek 出来是 None，老 DB 是真匿名，不动
    }

    // Step 2: 延迟 rename
    let owner = legacy_owner();
    if let Some(owner) = owner {
        let target = data_root.join(format!("hindsight.{owner}.sqlite"));
        if legacy.exists() && target.exists() {
            // 冲突：多半是上次 rename 失败（Windows 句柄未释放等）后，那次启动
            // 已经在 target 位置建了新库。此时**不能**清 legacy_owner——清了下次
            // 就再也不会尝试迁移，老数据永久搁浅在 hindsight.sqlite 里。
            // 保留 hint、大声记错误，等 legacy 句柄释放后的下次启动重试。
            log::error!(
                "legacy DB 迁移冲突：{} 与 {} 同时存在，升级前的历史数据仍在旧文件中；保留 legacy_owner 下次重试",
                legacy.display(),
                target.display()
            );
            return Ok(());
        }
        if legacy.exists() && !target.exists() {
            rename_db_files(&legacy, &target)?;
            log::info!("rename hindsight.sqlite -> hindsight.{owner}.sqlite");
        }
        // rename 成功 / legacy 已不存在 → hint 过期，清掉
        set_legacy_owner(None)?;
    }

    Ok(())
}

/// sign-in Case A 调：声明当前 `hindsight.sqlite` 归属于这个 uid，
/// 下次启动时 startup migration 会把文件 rename 到 `hindsight.<uid>.sqlite`。
pub fn claim_legacy_for(uid: &str) -> io::Result<()> {
    set_legacy_owner(Some(uid))
}

async fn peek_auth_state_uid(path: &Path) -> Option<String> {
    let pool = DbPool::open(path).await.ok()?;
    let row: Option<Option<Option<String>>> = pool
        .0
        .call(|conn| {
            Ok(conn
                .query_row("SELECT uid FROM auth_state WHERE id = 1", [], |r| {
                    r.get::<_, Option<String>>(0)
                })
                .ok())
        })
        .await
        .ok();
    // 显式 close 等后台线程真正释放文件句柄再返回——直接 drop 是异步释放，
    // 同一次启动里 Step 2 紧接着 rename 这个文件，Windows 上句柄没放会撞
    // sharing violation，迁移失败 + 本次启动在目标路径建空库。
    if let Err(e) = pool.0.close().await {
        log::warn!("peek_auth_state_uid: close legacy DB 失败: {e:?}");
    }
    row?.flatten().filter(|s| !s.trim().is_empty())
}

/// 重命名主 DB 文件，连带 SQLite 的 `-wal` / `-shm` 副文件一起。
/// 副文件不存在时跳过；副文件 rename 失败不影响主文件成功（SQLite 下次打开会重建 wal/shm）。
fn rename_db_files(src: &Path, dst: &Path) -> Result<()> {
    fs::rename(src, dst)?;
    for suffix in ["-wal", "-shm"] {
        let src_side = sidecar(src, suffix);
        let dst_side = sidecar(dst, suffix);
        if src_side.exists() {
            if let Err(e) = fs::rename(&src_side, &dst_side) {
                log::warn!("迁移 {suffix} 文件失败（可忽略）: {e}");
            }
        }
    }
    Ok(())
}

fn sidecar(p: &Path, suffix: &str) -> PathBuf {
    let mut s = p.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}
