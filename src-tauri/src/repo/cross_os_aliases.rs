//! 跨 OS 应用别名表：同一个真实 app 在不同操作系统下 process_name 不同，
//! app_groups 默认 `group_id == process_name` 跨设备不会自然合并 ——
//! 这里给出一份 canonical 名字，capture 时遇到别名直接进 canonical 组，
//! 启动期 backfill 把存量也合并过去。
//!
//! 数据存 `src-tauri/data/cross_os_app_aliases.json`，编译期嵌入二进制。
//! 启动后第一次 lookup 触发 lazy 解析，build 一个 `name(lowercase) → canonical`
//! 的 HashMap。整个进程生命周期只解析一次。
//!
//! 集成点：
//!   - [`app_groups::ensure_group`]：新 process_name 出现时先查 canonical，命中
//!     就用 canonical 当 group_id（而不是 process_name 自身）
//!   - [`pair_existing`]：bootstrap 期跑一次，把 v25 之前已经按 process_name 默认
//!     solo 的成员重新归到 canonical 组

use rusqlite::OptionalExtension;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::error::Result;
use crate::repo::outbox::{enqueue, OutboxEntity, OutboxOp};
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

const ALIASES_JSON: &str = include_str!("../../data/cross_os_app_aliases.json");

#[derive(Deserialize)]
struct RawAliases {
    aliases: Vec<RawAlias>,
}

#[derive(Deserialize)]
struct RawAlias {
    canonical: String,
    names: Vec<String>,
}

/// `process_name`（lowercase）→ canonical name（保留原始大小写，作为 group_id 用）。
fn aliases() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        let parsed: RawAliases = match serde_json::from_str(ALIASES_JSON) {
            Ok(p) => p,
            Err(e) => {
                log::error!("cross_os_app_aliases.json 解析失败（跳过）：{e}");
                return HashMap::new();
            }
        };
        let mut map = HashMap::new();
        for entry in parsed.aliases {
            for name in entry.names {
                map.insert(name.to_lowercase(), entry.canonical.clone());
            }
        }
        map
    })
}

/// 看 process_name 是否命中跨 OS 别名表。命中返回 canonical 名字，否则 None。
/// 大小写不敏感（`"chrome.exe"` 跟 `"Chrome.exe"` 等价）。
pub fn lookup_canonical(process_name: &str) -> Option<&'static str> {
    aliases().get(&process_name.to_lowercase()).map(|s| s.as_str())
}

/// 启动期一次性 backfill：扫所有 active member，把命中别名表且仍在默认 solo 组的
/// `process_name` 重新归到 canonical 组。返回合并的成员数。
///
/// **只动默认 solo 组**（`member.group_id == process_name`）；用户自己改过组结构
/// （拖到自定义组里）的不强拉回来，避免冲掉用户的手动配对。
///
/// 顺序：
///   1. 收集所有 (process_name, group_id, has_canonical) 三元组
///   2. 对每条命中且需要合并的：upsert canonical 组（已存在不动 display_name）
///      → UPDATE member.group_id = canonical → enqueue outbox
///      → 老 solo 组若空了软删（带 outbox）
///
/// 幂等：再跑一次发现 `member.group_id == canonical` 就直接跳过，零代价。
pub async fn pair_existing(pool: &DbPool) -> Result<u64> {
    let members: Vec<(String, String)> = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT process_name, group_id FROM app_group_members
                     WHERE deleted_at IS NULL",
                )
                .db()?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .db()?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.db()?);
            }
            Ok(out)
        })
        .await?;

    let mut merged = 0u64;
    for (process_name, group_id) in members {
        let Some(canonical) = lookup_canonical(&process_name) else {
            continue;
        };
        if group_id == canonical {
            // 已经在 canonical 组里：要么 mac 的 process_name 跟 canonical 同名，
            // 要么之前 pair_existing 跑过。无操作。
            continue;
        }
        if group_id != process_name {
            // 用户手动改过组（拖到了自定义组），尊重，不动。
            continue;
        }
        if let Err(e) = pair_one(pool, &process_name, canonical).await {
            // 单条失败不连累整批；日志里留下来方便排查。
            log::warn!(
                "cross_os pair 失败 process_name={process_name} canonical={canonical}: {e}"
            );
            continue;
        }
        merged += 1;
    }
    Ok(merged)
}

/// 把一条 process_name 从默认 solo 组迁移到 canonical 组，全程一次事务。
///
/// 步骤（单 `pool.0.call`）：
///   1. upsert canonical app_groups 行（如果不存在新建；soft-deleted 复活；
///      已活的不覆盖 display_name / category_id，尊重用户改过的状态）
///   2. UPDATE member.group_id 到 canonical（带 updated_at）
///   3. 老 solo 组（id == process_name）若已无 active member，软删
///   4. 三个改动各 enqueue outbox 一行，让对端 LWW 拉到同样的合并结果
async fn pair_one(pool: &DbPool, process_name: &str, canonical: &str) -> Result<()> {
    let pn = process_name.to_string();
    let canon = canonical.to_string();
    let now = chrono::Utc::now().to_rfc3339();
    // canonical 名也跑一次内置分类匹配 —— mac 的 "Google Chrome" 已经有 builtin 命中，
    // 新建 canonical 组时把这层分类一并带上，避免组建出来全 None 落到「其他」。
    let builtin_cat = super::builtin_categories::match_builtin_category(canonical);

    pool.0
        .call(move |conn| {
            // Step 0：事务内再核一次「member 还在 solo 组」—— pair_existing 读和写之间
            // 可能间隔 N 次 await，万一中间有别的路径（capture / sync pull）改过
            // member.group_id，这里跳过避免覆盖。重复调用 pair_one 时 (idempotency)
            // 也靠这条短路：第二次进来 group_id 已经是 canonical 就直接 return。
            let current_gid: Option<String> = conn
                .query_row(
                    "SELECT group_id FROM app_group_members
                     WHERE process_name = ?1 AND deleted_at IS NULL",
                    rusqlite::params![pn],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .db()?;
            if current_gid.as_deref() != Some(pn.as_str()) {
                // member 已不在默认 solo 组（被合到 canonical 了 / 用户改过 / 软删了），
                // 都不该再走 pair 流程。
                return Ok(());
            }

            // Step 1：upsert canonical 组。`ON CONFLICT DO UPDATE ... WHERE ...
            // deleted_at IS NOT NULL` 限定只在软删时复活；已活的不动 display_name /
            // category_id，保留用户改名 / 改分类。
            conn.execute(
                "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, ?, NULL)
                 ON CONFLICT(id) DO UPDATE SET
                   deleted_at = NULL,
                   updated_at = excluded.updated_at
                 WHERE app_groups.deleted_at IS NOT NULL",
                rusqlite::params![canon, canon, builtin_cat, now],
            )
            .db()?;
            enqueue(
                conn,
                OutboxOp::Upsert,
                OutboxEntity::AppGroup,
                &canon,
                &serde_json::json!({ "groupId": canon }).to_string(),
            )
            .db()?;

            // Step 2：把 member 的 group_id 改成 canonical
            conn.execute(
                "UPDATE app_group_members SET group_id = ?2, updated_at = ?3, deleted_at = NULL
                 WHERE process_name = ?1",
                rusqlite::params![pn, canon, now],
            )
            .db()?;
            enqueue(
                conn,
                OutboxOp::Upsert,
                OutboxEntity::AppGroupMember,
                &pn,
                &serde_json::json!({ "processName": pn }).to_string(),
            )
            .db()?;

            // Step 3：老 solo 组（id == process_name）若空了软删 + outbox
            let has_other_members: bool = conn
                .query_row(
                    "SELECT 1 FROM app_group_members
                     WHERE group_id = ?1 AND deleted_at IS NULL",
                    rusqlite::params![pn],
                    |_| Ok(true),
                )
                .optional()
                .db()?
                .unwrap_or(false);
            if !has_other_members {
                let n = conn
                    .execute(
                        "UPDATE app_groups SET deleted_at = ?1, updated_at = ?1
                         WHERE id = ?2 AND deleted_at IS NULL",
                        rusqlite::params![now, pn],
                    )
                    .db()?;
                if n > 0 {
                    enqueue(
                        conn,
                        OutboxOp::Upsert,
                        OutboxEntity::AppGroup,
                        &pn,
                        &serde_json::json!({ "groupId": pn }).to_string(),
                    )
                    .db()?;
                }
            }
            Ok(())
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::fresh_test_pool;
    use crate::storage::SqliteResultExt;

    /// 测 [`pair_existing`]：把两个跨 OS 别名（默认 solo 组）合并到 canonical 组。
    /// 别名表已嵌入二进制：`Code` 和 `Code.exe` 都映射到 `"Visual Studio Code"`。
    #[tokio::test]
    async fn pair_existing_merges_aliases_into_canonical_group() {
        let pool = fresh_test_pool().await;
        seed_solo_groups(&pool, &[("Code", "Code"), ("Code.exe", "Code.exe")]).await;
        // sanity 检查别名表确实命中
        assert_eq!(lookup_canonical("Code"), Some("Visual Studio Code"));
        assert_eq!(lookup_canonical("Code.exe"), Some("Visual Studio Code"));

        let merged = pair_existing(&pool).await.unwrap();
        assert_eq!(merged, 2, "两条别名都应被合并到 canonical");

        let canon_id = group_id_of_member(&pool, "Code").await.unwrap();
        assert_eq!(canon_id, "Visual Studio Code");
        let canon_id2 = group_id_of_member(&pool, "Code.exe").await.unwrap();
        assert_eq!(canon_id2, "Visual Studio Code");

        // canonical 组存在且 active
        assert!(group_active(&pool, "Visual Studio Code").await);
        // 原 solo 组都被软删
        assert!(!group_active(&pool, "Code").await);
        assert!(!group_active(&pool, "Code.exe").await);

        // 幂等：再跑一次发现 member.group_id 已是 canonical → 全部跳过
        let merged2 = pair_existing(&pool).await.unwrap();
        assert_eq!(merged2, 0);
    }

    /// 用户改过组结构（拖到自定义组）的成员不该被强拉回 canonical。
    #[tokio::test]
    async fn pair_existing_respects_user_custom_grouping() {
        let pool = fresh_test_pool().await;
        // "Code" 在自定义组 "my-tools" 里（不等于 process_name 也不等于 canonical）
        seed_solo_groups(&pool, &[("my-tools", "my-tools")]).await;
        seed_member(&pool, "Code", "my-tools").await;

        let merged = pair_existing(&pool).await.unwrap();
        assert_eq!(merged, 0, "用户手动配过的成员不该被强拉到 canonical");

        let still_custom = group_id_of_member(&pool, "Code").await.unwrap();
        assert_eq!(still_custom, "my-tools");
    }

    async fn seed_solo_groups(pool: &DbPool, groups: &[(&str, &str)]) {
        let groups: Vec<(String, String)> = groups
            .iter()
            .map(|(id, pn)| (id.to_string(), pn.to_string()))
            .collect();
        pool.0
            .call(move |conn| {
                let now = "2026-05-15T10:00:00Z";
                for (id, _pn) in &groups {
                    conn.execute(
                        "INSERT OR IGNORE INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                         VALUES(?1, ?1, NULL, ?2, NULL)",
                        rusqlite::params![id, now],
                    )
                    .db()?;
                }
                // solo 组：每条 (process_name, group_id=process_name) 一行 member
                for (id, pn) in &groups {
                    if id == pn {
                        conn.execute(
                            "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                             VALUES(?1, ?1, ?2, NULL)",
                            rusqlite::params![pn, now],
                        )
                        .db()?;
                    }
                }
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn seed_member(pool: &DbPool, process_name: &str, group_id: &str) {
        let pn = process_name.to_string();
        let gid = group_id.to_string();
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES(?1, ?2, '2026-05-15T10:00:00Z', NULL)",
                    rusqlite::params![pn, gid],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn group_id_of_member(pool: &DbPool, process_name: &str) -> Option<String> {
        let pn = process_name.to_string();
        pool.0
            .call(move |conn| {
                let r: Option<String> = conn
                    .query_row(
                        "SELECT group_id FROM app_group_members
                         WHERE process_name = ?1 AND deleted_at IS NULL",
                        rusqlite::params![pn],
                        |r| r.get(0),
                    )
                    .optional()
                    .db()?;
                Ok(r)
            })
            .await
            .unwrap()
    }

    async fn group_active(pool: &DbPool, id: &str) -> bool {
        let id = id.to_string();
        pool.0
            .call(move |conn| {
                let r: Option<i64> = conn
                    .query_row(
                        "SELECT 1 FROM app_groups WHERE id = ?1 AND deleted_at IS NULL",
                        rusqlite::params![id],
                        |r| r.get(0),
                    )
                    .optional()
                    .db()?;
                Ok(r.is_some())
            })
            .await
            .unwrap()
    }
}
