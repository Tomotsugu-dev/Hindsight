//! 内置应用分类规则：常见软件首次出现时自动归类，省得用户手动一个个分。
//!
//! 规则数据存在 `src-tauri/data/builtin_categories.json`，编译时通过 `include_str!`
//! 嵌入二进制 —— 不依赖运行时文件，发布也不会漏文件。
//!
//! 启动后第一次 lookup 触发 lazy 解析，把 JSON 反向索引为 process_name (lowercase)
//! → category_id 的 HashMap。整个进程生命周期里只解析一次。
//!
//! 集成点：
//!   - app_groups::ensure_group：新建 group 时调用 match_builtin_category，
//!     命中就直接填 category_id（不命中保持 NULL → 落到 "other"）
//!   - lib.rs setup：启动时跑一次 backfill_builtin_categories 给 NULL 的老 group
//!     补归类，让升级用户也能享受到新增规则。

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::error::Result;
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

// 三份按语言分组的规则文件，编译时全嵌入二进制；运行时合并到同一 hashmap，
// 跨语言查找统一（lowercase 完全相等）。社区贡献时按贡献者熟悉的语言文件加进去就行。
const BUILTIN_RULES_EN: &str = include_str!("../../data/builtin_categories.en.json");
const BUILTIN_RULES_ZH: &str = include_str!("../../data/builtin_categories.zh.json");
const BUILTIN_RULES_JA: &str = include_str!("../../data/builtin_categories.ja.json");

#[derive(Deserialize)]
struct RawRules {
    rules: Vec<RawRule>,
}

#[derive(Deserialize)]
struct RawRule {
    category: String,
    #[serde(rename = "processNames")]
    process_names: Vec<String>,
}

fn rules() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map = HashMap::new();
        for (label, json) in [
            ("en", BUILTIN_RULES_EN),
            ("zh", BUILTIN_RULES_ZH),
            ("ja", BUILTIN_RULES_JA),
        ] {
            // 单语言文件解析失败时降级跳过：UI 仍能用其它两份语言；三份全失败
            // 时返回空 map，等同"无内置分类"——比 panic 让整个 app 起不来好
            let parsed: RawRules = match serde_json::from_str(json) {
                Ok(p) => p,
                Err(e) => {
                    log::error!("builtin_categories.{label}.json 解析失败（跳过该语言）：{e}");
                    continue;
                }
            };
            for rule in parsed.rules {
                for name in rule.process_names {
                    // 后写覆盖：理论上不会冲突（每个名字落在一个 category）
                    map.insert(name.to_lowercase(), rule.category.clone());
                }
            }
        }
        map
    })
}

/// 看 process_name 是否命中内置规则。命中返回 category_id（&'static str），未命中 None。
/// 大小写不敏感（"Chrome.exe" 跟 "chrome.exe" 等价）。
pub fn match_builtin_category(process_name: &str) -> Option<&'static str> {
    rules()
        .get(&process_name.to_lowercase())
        .map(|s| s.as_str())
}

/// 启动时跑一次：扫所有 category_id IS NULL 且未删除的 app_group，
/// 按 builtin 规则尝试归类。返回更新的 group 行数。
///
/// 幂等：用户已经手动归类的（category_id 非 NULL）不动；本次没命中的也不动，
/// 下次升级 JSON 加规则后再启动会自动覆盖到。
///
/// 实现走 app_groups::assign_category —— 这条路径会同步处理：
///   1) 更新 app_groups.category_id
///   2) 把组内每个成员镜像到 app_categories 表（list_unclassified / 旧 reports 用）
///   3) 写 outbox 让 sync 推到云端
pub async fn backfill_builtin_categories(pool: &DbPool) -> Result<u64> {
    // 先在一个 conn call 里收集需要 backfill 的 (group_id, category_id) 对，
    // 避免在 conn closure 里跨 await 调 assign_category。
    let pending: Vec<(String, String)> = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, display_name FROM app_groups
                     WHERE category_id IS NULL AND deleted_at IS NULL",
                )
                .db()?;
            let rows: Vec<(String, String)> = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .db()?
                .filter_map(|r| r.ok())
                .collect();
            Ok(rows
                .into_iter()
                .filter_map(|(id, display)| {
                    match_builtin_category(&display).map(|cat| (id, cat.to_string()))
                })
                .collect())
        })
        .await?;

    let mut updated: u64 = 0;
    for (group_id, cat) in pending {
        super::app_groups::assign_category(pool, &group_id, Some(cat)).await?;
        updated += 1;
    }
    Ok(updated)
}
