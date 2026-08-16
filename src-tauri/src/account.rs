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

    // Step 2: 延迟 rename。两个库都就位才清 hint——只搬成功一个就清的话，
    // 剩下那个永远不会再被尝试。
    if let Some(owner) = legacy_owner() {
        if migrate_legacy_files(data_root, &owner)? {
            set_legacy_owner(None)?;
        }
    }

    Ok(())
}

/// 把匿名期的库文件搬到 uid 作用域路径。返回 `false` = 有冲突没搬完，
/// 调用方须保留 `legacy_owner` 下次重试。
///
/// 两个库都要搬：主库 `hindsight.sqlite` 早就在搬，而记忆库
/// `hindsight-memory.sqlite`（见 [`crate::memory::memory_db_path`]）一直漏掉——
/// 匿名期攒的 OCR 全文索引与全部聊天会话/消息会原地搁浅，登录后
/// `MemoryDb::open()` 在新路径直接建空库，搜索与聊天历史一次性归零；
/// 两个可能重新填充的云端数据集默认关着，损失不可逆。
///
/// 只碰 `data_root` 下的文件，不读写 `active_user.json`——后者走真实用户配置
/// 目录，测试碰不得。
fn migrate_legacy_files(data_root: &Path, owner: &str) -> Result<bool> {
    let jobs = [
        (
            "hindsight.sqlite",
            format!("hindsight.{owner}.sqlite"),
            "主库",
        ),
        (
            "hindsight-memory.sqlite",
            format!("hindsight-memory.{owner}.sqlite"),
            "记忆库",
        ),
    ];
    let mut all_done = true;
    for (legacy_name, target_name, what) in jobs {
        let legacy = data_root.join(legacy_name);
        let target = data_root.join(&target_name);
        if !legacy.exists() {
            continue;
        }
        if target.exists() {
            // 多半是上次 rename 失败（Windows 句柄未释放等）后，那次启动已经在
            // target 位置建了新库。此时**不能**清 legacy_owner——清了下次就再也
            // 不会尝试，老数据永久搁浅。保留 hint、大声记错误，等句柄释放后重试。
            log::error!(
                "{what}迁移冲突：{} 与 {} 同时存在，升级前的历史数据仍在旧文件中；保留 legacy_owner 下次重试",
                legacy.display(),
                target.display()
            );
            all_done = false;
            continue;
        }
        rename_db_files(&legacy, &target)?;
        log::info!("rename {legacy_name} -> {target_name}");
    }
    Ok(all_done)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 只碰临时 data_root，不触 active_user.json（那个走真实用户配置目录）。
    fn tmp_root(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "hindsight-account-test-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn touch(dir: &Path, name: &str, body: &str) {
        fs::write(dir.join(name), body).unwrap();
    }

    /// 核心回归：登录后记忆库必须跟主库一起搬。
    /// 漏搬的话匿名期的 OCR 全文索引 + 全部聊天历史原地搁浅，
    /// MemoryDb::open() 会在新路径建空库 —— 搜索与聊天记录一次性归零。
    #[test]
    fn migrates_both_main_and_memory_db_with_sidecars() {
        let root = tmp_root("both");
        touch(&root, "hindsight.sqlite", "main");
        touch(&root, "hindsight.sqlite-wal", "main-wal");
        touch(&root, "hindsight-memory.sqlite", "mem");
        touch(&root, "hindsight-memory.sqlite-wal", "mem-wal");
        touch(&root, "hindsight-memory.sqlite-shm", "mem-shm");

        assert!(
            migrate_legacy_files(&root, "uid42").unwrap(),
            "无冲突应返回 true"
        );

        assert_eq!(
            fs::read_to_string(root.join("hindsight.uid42.sqlite")).unwrap(),
            "main"
        );
        assert_eq!(
            fs::read_to_string(root.join("hindsight-memory.uid42.sqlite")).unwrap(),
            "mem",
            "记忆库必须一起搬"
        );
        // wal/shm 副文件跟随主文件
        assert_eq!(
            fs::read_to_string(root.join("hindsight-memory.uid42.sqlite-wal")).unwrap(),
            "mem-wal"
        );
        assert!(root.join("hindsight-memory.uid42.sqlite-shm").exists());
        assert!(
            !root.join("hindsight-memory.sqlite").exists(),
            "旧文件不该留下"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// 只有主库的老安装（记忆库还没建过）：不该因为记忆库缺席就判失败。
    #[test]
    fn missing_memory_db_is_not_a_conflict() {
        let root = tmp_root("main-only");
        touch(&root, "hindsight.sqlite", "main");

        assert!(migrate_legacy_files(&root, "u1").unwrap());
        assert!(root.join("hindsight.u1.sqlite").exists());
        assert!(!root.join("hindsight.sqlite").exists());

        let _ = fs::remove_dir_all(&root);
    }

    /// 记忆库目标已存在（上次搬失败后新库已建）：返回 false 让调用方保留 hint。
    /// 主库照常搬完 —— 下次启动只补记忆库，整个流程幂等。
    #[test]
    fn memory_conflict_reports_incomplete_but_still_moves_main() {
        let root = tmp_root("conflict");
        touch(&root, "hindsight.sqlite", "main");
        touch(&root, "hindsight-memory.sqlite", "old-mem");
        touch(&root, "hindsight-memory.u9.sqlite", "new-empty-mem");

        assert!(
            !migrate_legacy_files(&root, "u9").unwrap(),
            "有冲突必须返回 false，否则 hint 被清、旧数据永久搁浅"
        );
        assert!(
            root.join("hindsight.u9.sqlite").exists(),
            "主库不受记忆库冲突影响"
        );
        assert_eq!(
            fs::read_to_string(root.join("hindsight-memory.sqlite")).unwrap(),
            "old-mem",
            "冲突时旧记忆库原地保留，等下次重试"
        );
        assert_eq!(
            fs::read_to_string(root.join("hindsight-memory.u9.sqlite")).unwrap(),
            "new-empty-mem",
            "不得覆盖已存在的目标"
        );

        // 幂等：把冲突的新库挪走后重跑，这次应当搬成
        fs::remove_file(root.join("hindsight-memory.u9.sqlite")).unwrap();
        assert!(migrate_legacy_files(&root, "u9").unwrap());
        assert_eq!(
            fs::read_to_string(root.join("hindsight-memory.u9.sqlite")).unwrap(),
            "old-mem"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// 全新安装（两个库都不存在）：无事可做，不能报错。
    #[test]
    fn nothing_to_migrate_is_ok() {
        let root = tmp_root("empty");
        assert!(migrate_legacy_files(&root, "u1").unwrap());
        let _ = fs::remove_dir_all(&root);
    }
}
