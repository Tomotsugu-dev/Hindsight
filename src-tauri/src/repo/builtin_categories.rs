//! Built-in app categories: common apps get a category the first time they show
//! up, so the user does not have to sort them one by one.
//!
//! Rules live in `src-tauri/data/builtin_categories.<lang>.json`, one file per
//! UI language, compiled into the binary (no runtime file to ship) and merged at
//! runtime into one lookup table: process name (lowercased) → category id.
//!
//! Two entry points:
//!   - `app_groups::ensure_group`: checks the table when a process name first
//!     appears and, on a hit, creates the group with that category;
//!   - `backfill_builtin_categories`: runs once at startup and fills in older
//!     groups that still have no category.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::error::Result;
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

// One rule file per UI language, all compiled into the binary and merged into
// one lookup table at runtime (lowercase exact match). Contributors add names
// to the file of the language they know.
const BUILTIN_RULES_EN: &str = include_str!("../../data/builtin_categories.en.json");
const BUILTIN_RULES_ZH: &str = include_str!("../../data/builtin_categories.zh.json");
const BUILTIN_RULES_ZH_TW: &str = include_str!("../../data/builtin_categories.zh-TW.json");
const BUILTIN_RULES_JA: &str = include_str!("../../data/builtin_categories.ja.json");
const BUILTIN_RULES_ES: &str = include_str!("../../data/builtin_categories.es.json");
const BUILTIN_RULES_PT_BR: &str = include_str!("../../data/builtin_categories.pt-BR.json");

#[derive(Deserialize)]
struct RawRules {
    rules: Vec<RawRule>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRule {
    category: String,
    process_names: Vec<String>,
}

/// The merged lookup table: process name (lowercased) → category id. Built from
/// all rule files on first call and shared for the life of the process.
fn rules() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map = HashMap::new();
        for (label, json) in [
            ("en", BUILTIN_RULES_EN),
            ("zh", BUILTIN_RULES_ZH),
            ("zh-TW", BUILTIN_RULES_ZH_TW),
            ("ja", BUILTIN_RULES_JA),
            ("es", BUILTIN_RULES_ES),
            ("pt-BR", BUILTIN_RULES_PT_BR),
        ] {
            // A file that fails to parse is skipped, the others still load. If
            // all fail the map is empty, which just means no built-in rules —
            // better than a panic that keeps the app from starting.
            let parsed: RawRules = match serde_json::from_str(json) {
                Ok(p) => p,
                Err(e) => {
                    log::error!("builtin_categories.{label}.json 解析失败（跳过该语言）：{e}");
                    continue;
                }
            };
            for rule in parsed.rules {
                for name in rule.process_names {
                    map.insert(name.to_lowercase(), rule.category.clone());
                }
            }
        }
        map
    })
}

/// Look up the built-in category for a process name; `None` if no rule matches.
/// Case-insensitive (`"Chrome.exe"` and `"chrome.exe"` are the same).
pub fn match_builtin_category(process_name: &str) -> Option<&'static str> {
    rules()
        .get(&process_name.to_lowercase())
        .map(|s| s.as_str())
}

/// Runs once at startup: gives uncategorised groups a category from the built-in
/// rules, so rules added in an upgrade also reach existing users.
///
/// Scans every live group with an empty `category_id`, looks its display name
/// up in the rules and, on a hit, writes through `assign_category` (so the
/// change is queued for sync). Returns the number of groups filled in.
///
/// Idempotent: groups the user already categorised (`category_id` set) are left
/// alone; misses stay empty and get another chance the next time the rule files
/// grow.
pub async fn backfill_builtin_categories(pool: &DbPool) -> Result<u64> {
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

    let mut updated_cnt: u64 = 0;
    for (group_id, cat) in pending {
        super::app_groups::assign_category(pool, &group_id, Some(cat)).await?;
        updated_cnt += 1;
    }
    Ok(updated_cnt)
}
