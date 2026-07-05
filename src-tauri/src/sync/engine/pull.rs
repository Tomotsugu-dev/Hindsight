//! Pull 路径：列 Drive 文件，按 modifiedTime 增量下载，按文件名分发到 merge_*。
//! merge_* 都做 LWW（updated_at 字典序比较）+ idempotent upsert。

use std::sync::Arc;

use serde_json::Value;

use super::io;
use super::{format_sync_error, with_token_retry, Inner};
use crate::error::{Error, Result};
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};
use crate::sync::auth::{self, TokenInfo};
use crate::sync::payload::{
    ActivityPayload, AppCategoryPayload, AppGroupMemberPayload, AppGroupPayload, AppIconPayload,
    CategoryPayload, DeviceMetaPayload, ProcessPathPayload, TombstonePayload,
};

/// 解析一个 JSON 数组到 `Vec<T>`，但保留**每行容错**：单行解析失败仅打 warn 跳过，
/// 不让整文件因一行坏数据全废。`kind` 仅用于日志。
fn parse_rows<T: serde::de::DeserializeOwned>(kind: &'static str, body: &[u8]) -> Result<Vec<T>> {
    let arr: Vec<Value> =
        serde_json::from_slice(body).map_err(|e| Error::SyncParse { kind, source: e })?;
    let mut out = Vec::with_capacity(arr.len());
    for (idx, v) in arr.into_iter().enumerate() {
        match serde_json::from_value::<T>(v) {
            Ok(row) => out.push(row),
            Err(e) => log::warn!("{kind} 行 {idx} 解析失败: {e}"),
        }
    }
    Ok(out)
}

/// LWW 比较：拿表里当前 row 的 updated_at，看远端 `new` 是不是更新。row 不存在算"更新"。
/// 用 `OptionalExtension::optional()` 把 NoRows 转 None —— 不像 `.ok()` 会吞掉真错误。
fn is_remote_newer<P: rusqlite::Params>(
    conn: &rusqlite::Connection,
    select_updated_at_sql: &str,
    key: P,
    new: &str,
) -> rusqlite::Result<bool> {
    use rusqlite::OptionalExtension;
    let cur: Option<String> = conn
        .query_row(select_updated_at_sql, key, |r| r.get(0))
        .optional()?;
    Ok(match cur {
        None => true,
        Some(c) => new > c.as_str(),
    })
}

pub(super) const PULL_CURSOR_KEY: &str = "drive_files";

enum ParsedFile {
    ActivityDay {
        device_id: String,
        local_date: String,
    },
    Categories {
        device_id: String,
    },
    AppCategories {
        device_id: String,
    },
    ProcessPaths {
        device_id: String,
    },
    DeviceMeta {
        device_id: String,
    },
    AppIcons {
        device_id: String,
    },
    AppGroups {
        device_id: String,
    },
    AppGroupMembers {
        device_id: String,
    },
    /// `device.<UUID>.tombstone.json` —— 源设备明确告知"在某时刻之前的我的数据请全部清"，
    /// 对端 pull 时执行 `DELETE WHERE device_id=<owner> AND updated_at < clearedAt`。
    /// 修补 sync 协议「Drive 文件级删除不传播为 DB 行级 DELETE」的缺陷。
    Tombstone {
        device_id: String,
    },
    /// 可选上云:AI 总结文本(见 datasets.rs)
    AiSummaries {
        device_id: String,
    },
    /// 可选上云:聊天历史
    Chat {
        device_id: String,
    },
    /// 可选上云:屏幕记忆全文(按日分片)
    MemoryDay {
        device_id: String,
    },
}

fn parse_filename(name: &str) -> Option<ParsedFile> {
    // 形如：device.<UUID>.<KIND>.json 或 device.<UUID>.activities.<DAY>.ndjson
    let parts: Vec<&str> = name.split('.').collect();
    if parts.first().copied() != Some("device") {
        return None;
    }
    match parts.as_slice() {
        ["device", uuid, "activities", day, "ndjson"] => Some(ParsedFile::ActivityDay {
            device_id: uuid.to_string(),
            local_date: day.to_string(),
        }),
        ["device", uuid, "categories", "json"] => Some(ParsedFile::Categories {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "app_categories", "json"] => Some(ParsedFile::AppCategories {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "process_paths", "json"] => Some(ParsedFile::ProcessPaths {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "meta", "json"] => Some(ParsedFile::DeviceMeta {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "icons", "json"] => Some(ParsedFile::AppIcons {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "app_groups", "json"] => Some(ParsedFile::AppGroups {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "app_group_members", "json"] => Some(ParsedFile::AppGroupMembers {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "tombstone", "json"] => Some(ParsedFile::Tombstone {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "ai_summaries", "json"] => Some(ParsedFile::AiSummaries {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "chat", "json"] => Some(ParsedFile::Chat {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "memory", _day, "ndjson"] => Some(ParsedFile::MemoryDay {
            device_id: uuid.to_string(),
        }),
        _ => None,
    }
}

pub(super) async fn flush_pull(inner: &Arc<Inner>) -> Result<()> {
    // 串行门：与 flush_push / purge 类命令互斥（详见 Inner::flush_gate）。
    let _gate = inner.flush_gate.lock().await;
    let mut token: TokenInfo = match auth::ensure_valid_token(&inner.pool).await {
        Ok(t) => t,
        Err(Error::NotSignedIn) => return Ok(()),
        Err(e) => {
            // 同 push.rs 同名分支：warn 只写概述，detail 走 debug，避免 OAuth body
            // 里的 PII 落进 info 级日志文件；status 走 [CRED_EXPIRED]/[TRANSIENT] 分类
            log::warn!("sync pull 拿不到有效 token（详情见 status）");
            log::debug!("token error detail: {e}");
            inner.status.write().await.last_error = Some(format_sync_error(&e));
            return Ok(());
        }
    };

    let cursor = io::read_cursor(&inner.pool, PULL_CURSOR_KEY).await?;
    let cursor_q = if cursor.starts_with("1970-") {
        String::new()
    } else {
        cursor.clone()
    };

    let files = with_token_retry(&inner.pool, &mut token, |tok| {
        let cursor_q = cursor_q.clone();
        let drive = &inner.drive;
        async move { drive.list_appdata_files(&tok, &cursor_q).await }
    })
    .await?;
    if files.is_empty() {
        return Ok(());
    }

    let self_id = inner.self_id.as_str();
    if self_id.is_empty() {
        log::debug!("sync pull 跳过：self_id 为空（device 未初始化）");
        return Ok(());
    }
    // 可选数据集的三挡开关:关着的数据集文件标 handled 直接越过
    // (开关翻开时命令层会重置 pull 游标,历史文件会重新入列)
    let opt_cfg = crate::repo::settings::load(&inner.pool).await.ok();
    let (sync_ai, sync_chat, sync_mem) = opt_cfg
        .map(|c| {
            (
                c.sync_ai_summaries,
                c.sync_chat_history,
                c.sync_screen_memory,
            )
        })
        .unwrap_or((false, false, false));
    let local_os = crate::platform::local_os_id();
    let mut applied = 0u64;
    // 每个文件是否「应用 or 主动跳过」。Drive 已按 modifiedTime 升序返回，pull 结束时
    // 游标只推到最长连续 true 前缀的末尾 —— 第一个 false 之后哪怕后面有成功也不推，
    // 否则下次 pull 用 `modifiedTime > cursor` 查询会跳过那个失败文件，永久丢数据。
    // upsert 路径靠 (device_id, remote_id) 幂等 + LWW updated_at，重复拉无副作用。
    let mut handled = vec![false; files.len()];

    // Pass 1: 只跑 device.meta.json，让 devices.os 在 Pass 2 之前就位。
    // 否则一台陌生设备首次出现时，我们读 devices.os 是空的，没法做跨 OS 过滤。
    for (i, f) in files.iter().enumerate() {
        let parsed = match parse_filename(&f.name) {
            Some(p) => p,
            None => {
                // 不认识的文件名 = 不归我们管，可以跨过
                handled[i] = true;
                continue;
            }
        };
        let ParsedFile::DeviceMeta { device_id } = parsed else {
            // 非 DeviceMeta：留给 Pass 2 处理；这一 pass 不能下结论
            continue;
        };
        if device_id == self_id {
            handled[i] = true;
            continue;
        }
        let body = match with_token_retry(&inner.pool, &mut token, |tok| {
            let id = f.id.clone();
            let drive = &inner.drive;
            async move { drive.download(&tok, &id).await }
        })
        .await
        {
            Ok(b) => b,
            Err(e) => {
                log::warn!("下载 {} 失败: {e}", f.name);
                continue;
            }
        };
        if let Err(e) = merge_device_meta(&inner.pool, &device_id, &body).await {
            log::warn!("merge {} 失败: {e}", f.name);
            continue;
        }
        handled[i] = true;
        applied += 1;
    }

    // Pass 2: 其余类型；对平台特定的两类做 OS 过滤。
    for (i, f) in files.iter().enumerate() {
        let parsed = match parse_filename(&f.name) {
            Some(p) => p,
            // 不认识的文件名 Pass 1 已 mark handled=true，跳过
            None => continue,
        };
        if matches!(parsed, ParsedFile::DeviceMeta { .. }) {
            // Pass 1 已下结论（成功 / self / 失败），不重复处理
            continue;
        }
        if matches!(parsed, ParsedFile::Tombstone { .. }) {
            // tombstone 留到 Pass 3 处理 —— 确保所有 activity merge 完之后再 DELETE，
            // 即便存在「旧 ndjson 在 Drive 上残留没被 purge_cloud_data 删干净」的边角
            // 情况，tombstone 也能把刚刚 merge 进来的过期行清掉
            continue;
        }
        // 可选数据集:开关关着 → 标 handled 越过(不下载);开着 → 走正常下载合并
        match &parsed {
            ParsedFile::AiSummaries { .. } if !sync_ai => {
                handled[i] = true;
                continue;
            }
            ParsedFile::Chat { .. } if !sync_chat => {
                handled[i] = true;
                continue;
            }
            ParsedFile::MemoryDay { .. } if !sync_mem => {
                handled[i] = true;
                continue;
            }
            // 记忆库句柄缺失时这两类没法合并:同样越过,别卡游标
            ParsedFile::Chat { .. } | ParsedFile::MemoryDay { .. } if inner.mem.is_none() => {
                handled[i] = true;
                continue;
            }
            _ => {}
        }
        let device_id = match &parsed {
            ParsedFile::ActivityDay { device_id, .. }
            | ParsedFile::Categories { device_id }
            | ParsedFile::AppCategories { device_id }
            | ParsedFile::ProcessPaths { device_id }
            | ParsedFile::AppIcons { device_id }
            | ParsedFile::AppGroups { device_id }
            | ParsedFile::AppGroupMembers { device_id }
            | ParsedFile::AiSummaries { device_id }
            | ParsedFile::Chat { device_id }
            | ParsedFile::MemoryDay { device_id } => device_id.as_str(),
            ParsedFile::DeviceMeta { .. } | ParsedFile::Tombstone { .. } => unreachable!(),
        };
        // 本机自己的 activities ndjson **不**跳过 —— 配合 v26 + upsert_remote_activity
        // 的 self 分支（显式 id + origin='local'），让「清空本机数据库」后能从 Drive 恢复
        // 本机自己的历史。其它共享 metadata（categories / app_groups / ...）继续跳过：
        // - 这些 schema 没 device_id 列，跨设备共享，本机不会"丢"这些数据
        // - 拉自己的 metadata 文件除了浪费一次 download 没价值
        let is_self_activity =
            matches!(parsed, ParsedFile::ActivityDay { .. }) && device_id == self_id;
        if device_id == self_id && !is_self_activity {
            handled[i] = true;
            continue;
        }

        // app_categories / process_paths 是平台特定的：
        //   Windows tracker 写 process_name = "chrome.exe"，exe_path = "C:\\..."
        //   macOS tracker  写 process_name = "Google Chrome"，exe_path = "/Applications/.../MacOS/..."
        // 跨 OS 合并要么完全无用（key 对不上），要么坏事（同名 key 撞车，把本机能用的路径覆盖掉，icon 提取失败）。
        // activities / app_icons 不过滤 —— 跨设备聚合活动是核心价值；icon 字节就是要让对方
        // 给从那台机器同步过来的 activity 行渲染图标用的。
        if matches!(
            parsed,
            ParsedFile::AppCategories { .. } | ParsedFile::ProcessPaths { .. }
        ) {
            match remote_device_os(&inner.pool, device_id).await {
                // OS 已知且确实跨平台：跳过并标 handled（游标可越过——永远不该拉）。
                Some(os) if os != local_os => {
                    log::debug!(
                        "跳过跨 OS 文件 {} (远端 os={os}, 本机 {})",
                        f.name,
                        local_os
                    );
                    handled[i] = true;
                    continue;
                }
                Some(_) => {} // 同 OS：正常处理
                // OS 未知：多半是对端的 meta 文件还没到（push 是 HashMap 随机序，
                // app_categories 可能先落 Drive）。**不标 handled**——让游标停在
                // 这里，下轮 meta 到了再处理；标了 handled 游标越过后（list 用严格
                // `modifiedTime >`）这份文件永远不会再被拉，同 OS 对端的归类数据
                // 就永久缺失。
                None => {
                    log::debug!(
                        "暂缓文件 {}（远端 {} 的 OS 未知，等 meta）",
                        f.name,
                        device_id
                    );
                    continue;
                }
            }
        }

        let body = match with_token_retry(&inner.pool, &mut token, |tok| {
            let id = f.id.clone();
            let drive = &inner.drive;
            async move { drive.download(&tok, &id).await }
        })
        .await
        {
            Ok(b) => b,
            Err(e) => {
                log::warn!("下载 {} 失败: {e}", f.name);
                continue;
            }
        };

        let res = match parsed {
            ParsedFile::ActivityDay {
                device_id,
                local_date,
            } => merge_activities(&inner.pool, self_id, &device_id, &local_date, &body).await,
            ParsedFile::Categories { device_id } => {
                merge_categories(&inner.pool, &device_id, &body).await
            }
            ParsedFile::AppCategories { device_id } => {
                merge_app_categories(&inner.pool, &device_id, &body).await
            }
            ParsedFile::ProcessPaths { device_id } => {
                merge_process_paths(&inner.pool, &device_id, &body).await
            }
            ParsedFile::AppIcons { device_id } => {
                merge_app_icons(&inner.pool, &device_id, &body).await
            }
            ParsedFile::AppGroups { device_id } => {
                merge_app_groups(&inner.pool, &device_id, &body).await
            }
            ParsedFile::AppGroupMembers { device_id } => {
                merge_app_group_members(&inner.pool, &device_id, &body).await
            }
            ParsedFile::AiSummaries { .. } => {
                super::datasets::merge_ai_summaries(&inner.pool, &body).await
            }
            ParsedFile::Chat { .. } => {
                // 上面的门控已保证 mem 存在
                super::datasets::merge_chat(inner.mem.as_ref().expect("gated"), &body).await
            }
            ParsedFile::MemoryDay { device_id } => {
                super::datasets::merge_memory_sessions(
                    inner.mem.as_ref().expect("gated"),
                    &device_id,
                    &body,
                )
                .await
            }
            ParsedFile::DeviceMeta { .. } | ParsedFile::Tombstone { .. } => unreachable!(),
        };
        if let Err(e) = res {
            log::warn!("merge {} 失败: {e}", f.name);
            continue;
        }
        handled[i] = true;
        applied += 1;
    }

    // Pass 3: tombstone 清扫 —— 放在最后跑，确保 Pass 2 merge 进来的 activity 行
    // 之后再做时间戳 DELETE。这样即便 Drive 上 purge_cloud_data 没删干净的旧 ndjson
    // 残留被 Pass 2 merge 回来，Pass 3 的 `DELETE WHERE updated_at < clearedAt` 还能
    // 把它们清掉。
    for (i, f) in files.iter().enumerate() {
        let parsed = match parse_filename(&f.name) {
            Some(p) => p,
            None => continue, // Pass 1 已 mark
        };
        let ParsedFile::Tombstone { device_id } = parsed else {
            continue; // 只处理 tombstone
        };
        // 本机自己的 tombstone 不应用：purge_cloud_data(keep_local=true)（切换账号场景）
        // 会上传 self tombstone 但**保留本地数据**，若在这里执行 DELETE 会把承诺保留的
        // 本地历史全部删光。keep_local=false 时本地已清空，跳过也是 no-op，语义不变。
        if device_id == self_id {
            handled[i] = true;
            continue;
        }
        let body = match with_token_retry(&inner.pool, &mut token, |tok| {
            let id = f.id.clone();
            let drive = &inner.drive;
            async move { drive.download(&tok, &id).await }
        })
        .await
        {
            Ok(b) => b,
            Err(e) => {
                log::warn!("下载 {} 失败: {e}", f.name);
                continue;
            }
        };
        if let Err(e) = merge_tombstone(&inner.pool, &device_id, &body).await {
            log::warn!("merge tombstone {} 失败: {e}", f.name);
            continue;
        }
        handled[i] = true;
        applied += 1;
    }

    // 推 cursor 到最长连续 handled 前缀的 modified_time。第一个失败之后即使后面有成功也不
    // 推 —— 见 `handled` 声明处的说明。
    //
    // 边界：下次 list 用严格 `modifiedTime > cursor`。若首个未处理文件与前缀里最后
    // 一个已处理文件的 modifiedTime **精确相同**（两设备同毫秒落盘 + 其一瞬时下载
    // 失败），把 cursor 推到该时间会让失败的那份永远查不出来。因此推进值取前缀中
    // "严格早于首个未处理文件时间"的最后一个；RFC3339 同构串比大小 = 时间序。
    let first_unhandled_time = files
        .iter()
        .zip(handled.iter())
        .find(|(_, ok)| !**ok)
        .map(|(f, _)| f.modified_time.clone());
    let cursor_advance = files
        .iter()
        .zip(handled.iter())
        .take_while(|(_, ok)| **ok)
        .map(|(f, _)| f.modified_time.clone())
        .filter(|t| match &first_unhandled_time {
            Some(fu) => t.as_str() < fu.as_str(),
            None => true,
        })
        .last();
    if let Some(t) = cursor_advance {
        io::write_cursor(&inner.pool, PULL_CURSOR_KEY, &t).await?;
    }
    inner.status.write().await.last_pulled_at = Some(utc_now_rfc3339());
    if applied > 0 {
        log::info!("sync pull 完成，应用 {} 个远端文件", applied);
    }
    Ok(())
}

/// 查 devices 表里某个远端设备的 os；没有 device_meta 同步过来时返回 None。
async fn remote_device_os(pool: &DbPool, device_id: &str) -> Option<String> {
    let id = device_id.to_string();
    pool.0
        .call(move |conn| {
            let r = conn
                .query_row(
                    "SELECT os FROM devices WHERE device_id = ?1",
                    rusqlite::params![id],
                    |r| r.get::<_, Option<String>>(0),
                )
                .ok()
                .flatten()
                .filter(|s| !s.is_empty());
            Ok(r)
        })
        .await
        .ok()
        .flatten()
}

async fn merge_activities(
    pool: &DbPool,
    self_id: &str,
    device_id: &str,
    local_date: &str,
    body: &[u8],
) -> Result<()> {
    // ndjson：一行一个 ActivityPayload
    let s = std::str::from_utf8(body).map_err(Error::from)?;
    // 收集本文件出现过的 remote_id（= 源端 activities.id）—— 解析成功 + 字段合法的行才算
    // 「源端目前存在」。结束后用 NOT IN 把本机 mirror 里这个 (device_id, local_date)
    // 范围内不在 set 里的行 DELETE 掉，让 mirror 跟源端的全表 rewrite 严格一致。
    // 这是修补 sync 协议「源端删行 → 对端 mirror 不会自动 DELETE」的关键 ——
    // 没有这一步，源端 [activities::purge_orphan_sessions] 干掉的孤儿行永久留在对端镜像。
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    // 解析阶段任何一行硬错误就放弃 mirror 收敛，避免半截文件 / 解析 bug 把对端 mirror
    // 误删一大堆。仅在文件**完整解析无异常**时执行 DELETE 收敛。
    let mut parse_clean = true;

    for (lineno, line) in s.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let row: ActivityPayload = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("activities 行 {lineno} 解析失败: {e}");
                parse_clean = false;
                continue;
            }
        };
        if row.id < 0 {
            continue;
        }
        let remote_id = row.id.to_string();
        seen_ids.insert(remote_id.clone());
        let updated_at = if row.updated_at.is_empty() {
            row.ended_at.clone()
        } else {
            row.updated_at.clone()
        };
        // 单行 upsert 失败降级为 warn —— 否则一行坏数据会让 flush_pull 把整文件判为
        // 失败（handled[i] 留 false），游标停在前一文件，下次再拉这文件还是坏行 → 永久卡住。
        if let Err(e) = upsert_remote_activity(
            pool,
            self_id,
            device_id,
            &remote_id,
            &row.started_at,
            &row.ended_at,
            row.duration_secs,
            &row.local_date,
            row.local_hour as u8,
            &row.process_name,
            row.window_title.as_deref().unwrap_or(""),
            &row.category_id,
            &updated_at,
        )
        .await
        {
            log::warn!("activities 行 {lineno} upsert 失败: {e}");
            parse_clean = false;
        }
    }

    // mirror 收敛：本文件该 (device_id, local_date) 下 ndjson 没列出的 mirror 行 DELETE。
    // 跳过 self_id 防自删 —— self 自己的行不受 mirror 收敛影响（self 的 row 是 local 来源
    // 不是 mirror，DELETE 它们会丢失本机自己的数据）。这条件配合 v26 + lift self-skip 的
    // self-pull 场景：本机 pull 自己的 ndjson 时，不收敛自己的行（既然是自己的，DELETE
    // 任何 < clearedAt 之类的逻辑该走 tombstone 路径，不该走 mirror 收敛）。
    let is_self = !self_id.is_empty() && device_id == self_id;
    if !parse_clean || is_self {
        return Ok(());
    }
    let device_id_db = device_id.to_string();
    let local_date_db = local_date.to_string();
    let ids_vec: Vec<String> = seen_ids.into_iter().collect();
    let deleted = pool
        .0
        .call(move |conn| {
            // 拼 NOT IN ?,?,... 用动态占位符
            let placeholders: String = if ids_vec.is_empty() {
                String::new()
            } else {
                std::iter::repeat_n("?", ids_vec.len())
                    .collect::<Vec<_>>()
                    .join(",")
            };
            let sql = if placeholders.is_empty() {
                // 空文件 → 收敛 = 该 (device, local_date) 下所有 mirror 行都 DELETE
                "DELETE FROM activities
                 WHERE device_id = ?1 AND local_date = ?2"
                    .to_string()
            } else {
                format!(
                    "DELETE FROM activities
                     WHERE device_id = ?1 AND local_date = ?2
                       AND remote_id NOT IN ({placeholders})"
                )
            };
            let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(2 + ids_vec.len());
            params.push(&device_id_db);
            params.push(&local_date_db);
            for id in &ids_vec {
                params.push(id);
            }
            let n = conn.execute(&sql, params.as_slice()).db()?;
            Ok(n)
        })
        .await?;
    if deleted > 0 {
        log::info!(
            "mirror 收敛 device={device_id} local_date={local_date}: 删 {deleted} 条已不在源端 ndjson 的旧 mirror 行"
        );
    }
    Ok(())
}

/// 简单 LWW upsert 合并模板：
/// 1. parse_rows 整文件失败 → 抛错（让 flush_pull 的 cursor 别推进）
/// 2. 单行通过 `pk_for_log` 返 `Some(label)` 才进 upsert；`None` 跳过
/// 3. 单行的 LWW gate + upsert 由 `apply` 闭包做（只能引用 `T` 自带的字段，
///    不能 capture 外部变量 —— 这样闭包是 `Copy`，能被循环里每行复用）
/// 4. 单行 DB 错误降级 warn（per-line skip：一行坏数据不让对端 mirror 永久卡住）
///
/// merge_app_icons / merge_app_groups / merge_categories / merge_activities 不走这个模板，
/// 因为它们各自有合理的特殊路径（base64 文件 cache / member mirror / cascade
/// delete / mirror 收敛）。
async fn merge_lww_simple<T, F>(
    pool: &DbPool,
    entity: &'static str,
    body: &[u8],
    pk_for_log: impl Fn(&T) -> Option<String>,
    apply: F,
) -> Result<()>
where
    T: serde::de::DeserializeOwned + Send + 'static,
    F: Fn(&rusqlite::Connection, T) -> rusqlite::Result<()> + Send + Sync + Copy + 'static,
{
    let rows: Vec<T> = parse_rows(entity, body)?;
    for row in rows {
        let Some(label) = pk_for_log(&row) else {
            continue;
        };
        let res = pool
            .0
            .call(move |conn| apply(conn, row).map_err(tokio_rusqlite::Error::Rusqlite))
            .await;
        if let Err(e) = res {
            log::warn!("{entity} {label} merge 失败: {e}");
        }
    }
    Ok(())
}

async fn merge_categories(pool: &DbPool, _device_id: &str, body: &[u8]) -> Result<()> {
    merge_lww_simple(
        pool,
        "category",
        body,
        |row: &CategoryPayload| (!row.id.is_empty()).then(|| row.id.clone()),
        |conn, row: CategoryPayload| {
            let cur: Option<(String, Option<String>)> = conn
                .query_row(
                    "SELECT updated_at, deleted_at FROM categories WHERE id = ?1",
                    rusqlite::params![row.id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok();
            let should_apply = match &cur {
                None => true,
                Some((cur_upd, _)) => row.updated_at.as_str() > cur_upd.as_str(),
            };
            if !should_apply {
                return Ok(());
            }
            let prev_deleted = cur.as_ref().and_then(|(_, d)| d.clone());

            if cur.is_none() {
                conn.execute(
                    "INSERT INTO categories(id, name, color, icon, builtin, sort_order, updated_at, deleted_at)
                     VALUES(?, ?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![row.id, row.name, row.color, row.icon, row.builtin as i64, row.sort_order, row.updated_at, row.deleted_at],
                )?;
            } else {
                conn.execute(
                    "UPDATE categories SET name = ?, color = ?, icon = ?, builtin = ?,
                                            sort_order = ?, updated_at = ?, deleted_at = ?
                     WHERE id = ?",
                    rusqlite::params![row.name, row.color, row.icon, row.builtin as i64, row.sort_order, row.updated_at, row.deleted_at, row.id],
                )?;
            }

            // 远端把这个分类删了 —— 跑一次本地 cascade。仅在「之前没删，现在变成删了」的
            // 边沿触发；幂等 cascade SQL 让重复同步是 no-op。
            let just_deleted = row.deleted_at.is_some() && prev_deleted.is_none();
            if just_deleted {
                crate::repo::categories::cascade_category_deletion(conn, &row.id, &row.updated_at)?;
            }
            Ok(())
        },
    )
    .await
}

async fn merge_app_categories(pool: &DbPool, _device_id: &str, body: &[u8]) -> Result<()> {
    merge_lww_simple(
        pool,
        "app_category",
        body,
        |row: &AppCategoryPayload| (!row.process_name.is_empty()).then(|| row.process_name.clone()),
        |conn, row: AppCategoryPayload| {
            if !is_remote_newer(
                conn,
                "SELECT updated_at FROM app_categories WHERE process_name = ?1",
                rusqlite::params![row.process_name],
                &row.updated_at,
            )? {
                return Ok(());
            }
            conn.execute(
                "INSERT INTO app_categories(process_name, category_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, ?)
                 ON CONFLICT(process_name) DO UPDATE SET
                   category_id = excluded.category_id,
                   updated_at = excluded.updated_at,
                   deleted_at = excluded.deleted_at",
                rusqlite::params![
                    row.process_name,
                    row.category_id,
                    row.updated_at,
                    row.deleted_at
                ],
            )?;
            Ok(())
        },
    )
    .await
}

async fn merge_process_paths(pool: &DbPool, _device_id: &str, body: &[u8]) -> Result<()> {
    merge_lww_simple(
        pool,
        "process_path",
        body,
        |row: &ProcessPathPayload| (!row.process_name.is_empty()).then(|| row.process_name.clone()),
        |conn, row: ProcessPathPayload| {
            if !is_remote_newer(
                conn,
                "SELECT updated_at FROM process_paths WHERE process_name = ?1",
                rusqlite::params![row.process_name],
                &row.updated_at,
            )? {
                return Ok(());
            }
            conn.execute(
                "INSERT INTO process_paths(process_name, exe_path, seen_at, updated_at)
                 VALUES(?, ?, ?, ?)
                 ON CONFLICT(process_name) DO UPDATE SET
                   exe_path = excluded.exe_path,
                   seen_at = excluded.seen_at,
                   updated_at = excluded.updated_at",
                rusqlite::params![row.process_name, row.exe_path, row.seen_at, row.updated_at],
            )?;
            Ok(())
        },
    )
    .await
}

async fn merge_app_icons(pool: &DbPool, _device_id: &str, body: &[u8]) -> Result<()> {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
    let rows: Vec<AppIconPayload> = parse_rows("app_icons", body)?;
    for row in rows {
        if row.process_name.is_empty() {
            continue;
        }
        let process_name = row.process_name;
        let icon_bytes = match BASE64.decode(row.icon_png_base64.as_bytes()) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("app_icon process={process_name} base64 解码失败: {e}");
                continue;
            }
        };
        let updated_at = row.updated_at;
        let deleted_at = row.deleted_at;

        let process_name_db = process_name.clone();
        let icon_bytes_db = icon_bytes.clone();
        let updated_at_db = updated_at.clone();
        let deleted_at_db = deleted_at.clone();
        // 单行失败降级为 warn —— 见 merge_activities 同模式注释。
        let applied: bool = match pool
            .0
            .call(move |conn| {
                if !is_remote_newer(
                    conn,
                    "SELECT updated_at FROM app_icons WHERE process_name = ?1",
                    rusqlite::params![process_name_db],
                    &updated_at_db,
                )? {
                    return Ok(false);
                }
                conn.execute(
                    "INSERT INTO app_icons(process_name, icon_png, updated_at, deleted_at)
                     VALUES(?, ?, ?, ?)
                     ON CONFLICT(process_name) DO UPDATE SET
                       icon_png   = excluded.icon_png,
                       updated_at = excluded.updated_at,
                       deleted_at = excluded.deleted_at",
                    rusqlite::params![process_name_db, icon_bytes_db, updated_at_db, deleted_at_db],
                )
                .db()?;
                Ok(true)
            })
            .await
        {
            Ok(v) => v,
            Err(e) => {
                log::warn!("app_icon {process_name} merge 失败: {e}");
                continue;
            }
        };

        // 把 BLOB 同步落到文件 cache —— 让 UI 后续 get_app_icon 直接命中文件 cache 返回。
        // 软删（deleted_at != NULL）时反过来：把 cache 文件清掉，避免渲染过期图标。
        if applied {
            let path = match crate::repo::app_icons::icon_cache_path(&process_name) {
                Ok(p) => p,
                Err(e) => {
                    log::warn!("解析 icon cache 路径失败 process={process_name}: {e}");
                    continue;
                }
            };
            if deleted_at.is_some() {
                let _ = std::fs::remove_file(&path);
            } else {
                crate::repo::app_icons::write_cache_file(&path, &icon_bytes);
            }
        }
    }
    Ok(())
}

async fn merge_app_groups(pool: &DbPool, _device_id: &str, body: &[u8]) -> Result<()> {
    let rows: Vec<AppGroupPayload> = parse_rows("app_groups", body)?;
    for row in rows {
        if row.id.is_empty() {
            continue;
        }
        // 拿当前本地 category_id 用来对比 —— 远端的分类 LWW 赢了之后，要 mirror 到
        // app_categories 表里所有成员行（让 reports.rs 的 LEFT JOIN 仍能拿到正确分类）。
        let id = row.id.clone();
        // 单行失败降级为 warn —— 见 merge_activities 同模式注释。
        let applied: Option<(Option<String>, Option<String>)> = match pool
            .0
            .call(move |conn| {
                let prev: Option<(String, Option<String>)> = conn
                    .query_row(
                        "SELECT updated_at, category_id FROM app_groups WHERE id = ?1",
                        rusqlite::params![row.id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .ok();
                let should_apply = match &prev {
                    None => true,
                    Some((cur_upd, _)) => row.updated_at.as_str() > cur_upd.as_str(),
                };
                if !should_apply {
                    return Ok(None);
                }
                let prev_cat = prev.map(|(_, c)| c).unwrap_or(None);
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES(?, ?, ?, ?, ?)
                     ON CONFLICT(id) DO UPDATE SET
                       display_name = excluded.display_name,
                       category_id  = excluded.category_id,
                       updated_at   = excluded.updated_at,
                       deleted_at   = excluded.deleted_at",
                    rusqlite::params![
                        row.id,
                        row.display_name,
                        row.category_id,
                        row.updated_at,
                        row.deleted_at
                    ],
                )
                .db()?;
                Ok(Some((prev_cat, row.category_id)))
            })
            .await
        {
            Ok(v) => v,
            Err(e) => {
                log::warn!("app_group {id} merge 失败: {e}");
                continue;
            }
        };

        // 如果分类变了 —— 把新分类同步到组里所有 (active) 成员的 app_categories 行。
        // 用本地的 process_name 列表（成员可能是 Mac 风格也可能是 Win 风格），
        // 每行 enqueue outbox 让其它设备也拿到（同 OS 的对端会收到同样的 app_category 行）。
        if let Some((prev_cat, next_cat)) = applied {
            if prev_cat != next_cat {
                let id_for_mirror = id.clone();
                let next_for_mirror = next_cat.clone();
                let now = utc_now_rfc3339();
                if let Err(e) = pool
                    .0
                    .call(move |conn| {
                        let members: Vec<String> = {
                            let mut stmt = conn
                                .prepare(
                                    "SELECT process_name FROM app_group_members
                                     WHERE group_id = ?1 AND deleted_at IS NULL",
                                )
                                .db()?;
                            let rows = stmt
                                .query_map(rusqlite::params![id_for_mirror], |r| {
                                    r.get::<_, String>(0)
                                })
                                .db()?;
                            let mut out = Vec::new();
                            for r in rows {
                                out.push(r.db()?);
                            }
                            out
                        };
                        for m in &members {
                            // 远端推过来的分类变更：mirror 到 app_categories 但不入 outbox
                            // —— 否则会形成「收到对端推 → 本端再推回去」的死循环。
                            crate::repo::app_groups::apply_app_category_change(
                                conn,
                                m,
                                next_for_mirror.as_deref(),
                                &now,
                            )?;
                        }
                        Ok(())
                    })
                    .await
                {
                    log::warn!("app_group {id} 分类 mirror 失败: {e}");
                }
            }
        }
    }
    Ok(())
}

async fn merge_app_group_members(pool: &DbPool, _device_id: &str, body: &[u8]) -> Result<()> {
    merge_lww_simple(
        pool,
        "app_group_member",
        body,
        |row: &AppGroupMemberPayload| {
            (!row.process_name.is_empty() && !row.group_id.is_empty())
                .then(|| row.process_name.clone())
        },
        |conn, row: AppGroupMemberPayload| {
            if !is_remote_newer(
                conn,
                "SELECT updated_at FROM app_group_members WHERE process_name = ?1",
                rusqlite::params![row.process_name],
                &row.updated_at,
            )? {
                return Ok(());
            }
            conn.execute(
                "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, ?)
                 ON CONFLICT(process_name) DO UPDATE SET
                   group_id   = excluded.group_id,
                   updated_at = excluded.updated_at,
                   deleted_at = excluded.deleted_at",
                rusqlite::params![
                    row.process_name,
                    row.group_id,
                    row.updated_at,
                    row.deleted_at
                ],
            )?;
            Ok(())
        },
    )
    .await
}

/// 处理 `device.<owner_id>.tombstone.json`：源设备明确告知"在 clearedAt 之前的我的数据
/// 请全部清"。执行：
///
///   DELETE FROM activities WHERE device_id = <owner_id> AND updated_at < clearedAt;
///
/// 边角：
/// - **本机自己的 tombstone**（owner_id == self_id）在 Pass 3 就被跳过、不会走到这里：
///   purge_cloud_data(keep_local=true) 上传 self tombstone 但保留本地数据，应用它会把
///   本地历史删光（keep_local=false 时本地已清空，跳过等价 no-op）。
/// - 幂等：tombstone 永久留在 Drive，对端反复 pull 看到它，每次 DELETE 命中 0
///   （因为已经删过了）→ no-op。
/// - 不影响 capture：源端 purge 之后新 capture 的行 `updated_at > clearedAt` →
///   不被 DELETE 影响。
async fn merge_tombstone(pool: &DbPool, owner_device_id: &str, body: &[u8]) -> Result<()> {
    let payload: TombstonePayload = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("tombstone 解析失败 (device={owner_device_id}): {e}");
            return Ok(());
        }
    };
    if payload.cleared_at.is_empty() {
        log::warn!("tombstone clearedAt 为空 (device={owner_device_id})，跳过");
        return Ok(());
    }
    let owner = owner_device_id.to_string();
    let cleared_at = payload.cleared_at.clone();
    let deleted = pool
        .0
        .call(move |conn| {
            let n = conn
                .execute(
                    "DELETE FROM activities
                     WHERE device_id = ?1 AND updated_at < ?2",
                    rusqlite::params![owner, cleared_at],
                )
                .db()?;
            Ok(n)
        })
        .await?;

    // tombstone 还顺手把 devices 行 mark deleted_at —— 让 "controller 从云端把
    // device X 整个移除" 操作传达给所有其它设备：它们 pull 到这个 tombstone 后，
    // 不光删 activities，还把 "设备页里那个幽灵设备卡" 也清掉。
    //
    // 关键防护 `updated_at < cleared_at`：只在该设备 meta 没有在 cleared_at 之后
    // 刷新过时才软删。场景对比：
    //   - 死掉的设备：永远不再 push 新 meta，updated_at 永远 < cleared_at，标软删 ✓
    //   - 还活着的设备触发 purge_cloud_data：随后会继续 push 新 meta，下次 push tick
    //     上传的 meta.updated_at > cleared_at；pull pass 1 先 upsert meta（同时清空
    //     deleted_at，见 merge_device_meta 的 ON CONFLICT 子句），pass 3 的 tombstone
    //     遇到 updated_at > cleared_at 不再命中，最终设备仍然出现在列表 ✓
    let owner2 = owner_device_id.to_string();
    let cleared_at2 = payload.cleared_at;
    pool.0
        .call(move |conn| {
            conn.execute(
                "UPDATE devices
                 SET deleted_at = ?2, updated_at = ?2
                 WHERE device_id = ?1
                   AND updated_at < ?2
                   AND (deleted_at IS NULL OR deleted_at < ?2)",
                rusqlite::params![owner2, cleared_at2],
            )
            .db()?;
            Ok(())
        })
        .await?;

    if deleted > 0 {
        log::info!("tombstone applied: device={owner_device_id} deleted {deleted} activity rows");
    }
    Ok(())
}

async fn merge_device_meta(pool: &DbPool, device_id: &str, body: &[u8]) -> Result<()> {
    // device meta 是单对象，不是数组。空对象当作"还没数据"，跳过。
    let parsed: Value = serde_json::from_slice(body).map_err(|e| Error::SyncParse {
        kind: "device_meta",
        source: e,
    })?;
    let Value::Object(_) = parsed else {
        return Ok(());
    };
    let row: DeviceMetaPayload = match serde_json::from_value(parsed) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("device_meta 解析失败: {e}");
            return Ok(());
        }
    };
    let device_id = device_id.to_string();
    pool.0
        .call(move |conn| {
            if !is_remote_newer(
                conn,
                "SELECT updated_at FROM devices WHERE device_id = ?1",
                rusqlite::params![device_id],
                &row.updated_at,
            )? {
                return Ok(());
            }
            // 收到比本地 updated_at 更新的 meta → 该设备"又活了"：清掉之前
            // tombstone 留下的 deleted_at 软删标记，让该设备重新出现在设备列表里。
            // 场景：用户在 A 上跑 purge_cloud_data（清云端但 A 还活着），B 先 pull 到
            // tombstone 把 A 标 deleted，A 继续 capture 一会儿后又 push 新 meta，
            // B 下次 pull 到这个新 meta 应该把 A 拉回来。
            conn.execute(
                "INSERT INTO devices(device_id, display_name, color, icon, os, last_seen_at, is_self, updated_at, deleted_at)
                 VALUES(?, ?, ?, ?, ?, ?, 0, ?, NULL)
                 ON CONFLICT(device_id) DO UPDATE SET
                   display_name = excluded.display_name,
                   color = excluded.color,
                   icon = excluded.icon,
                   os = excluded.os,
                   last_seen_at = excluded.last_seen_at,
                   updated_at = excluded.updated_at,
                   deleted_at = NULL",
                rusqlite::params![device_id, row.display_name, row.color, row.icon, row.os, row.last_seen_at, row.updated_at],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn upsert_remote_activity(
    pool: &DbPool,
    self_id: &str,
    device_id: &str,
    remote_id: &str,
    started_at: &str,
    ended_at: &str,
    duration_secs: i64,
    local_date: &str,
    local_hour: u8,
    process_name: &str,
    window_title: &str,
    category_id: &str,
    updated_at: &str,
) -> Result<()> {
    // 是否本机自己的 ndjson 拉回来？
    // 配合 v26 migration（local 行 `remote_id = id`）+ pull self-skip 移除后的设计：
    // - mac 在「清空本机数据库」之后下次 pull 会拉到自己的 `device.<mac>.activities.<day>.ndjson`
    // - 这里 INSERT 必须**用显式 id**（来自 remote_id）+ origin='local'，否则：
    //   1. id 用 AUTOINCREMENT 拿到新值（如 1）→ 跟 Drive 文件里的原 id（如 42）对不上
    //   2. 下次 push 走 [build_activities_day]，SELECT 出来的是新本地 id（1）→ rewrite Drive 文件
    //      变成 id=1 → 对端 Win pull 看到的 remote_id 从 "42" 变 "1" → upsert key 错位 →
    //      Win 端**重复 INSERT**。整张历史在对端裂成两份。
    // - 显式 id 保证 push 重写后 Drive 文件 id 字段跟 purge 前一致，跨设备身份对称
    let is_self = !self_id.is_empty() && device_id == self_id;
    let device_id = device_id.to_string();
    let remote_id = remote_id.to_string();
    let started_at = started_at.to_string();
    let ended_at = ended_at.to_string();
    let local_date = local_date.to_string();
    let process_name = process_name.to_string();
    let window_title = window_title.to_string();
    let category_id = category_id.to_string();
    let updated_at = updated_at.to_string();
    pool.0
        .call(move |conn| {
            let existing: Option<(i64, String)> = conn
                .query_row(
                    "SELECT id, updated_at FROM activities
                     WHERE device_id = ?1 AND remote_id = ?2",
                    rusqlite::params![device_id, remote_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok();
            match existing {
                None => {
                    if is_self {
                        // 本机自己拉回来：显式 id + origin='local'，保持身份对端可识别 +
                        // 本机视角的 "local 来源" 语义
                        let explicit_id: i64 =
                            remote_id.parse().map_err(|e: std::num::ParseIntError| {
                                tokio_rusqlite::Error::Other(Box::new(e))
                            })?;
                        conn.execute(
                            "INSERT INTO activities(
                               id, started_at, ended_at, duration_secs, local_date, local_hour,
                               process_name, window_title, category_id, screenshot_path,
                               device_id, remote_id, updated_at, origin
                             ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?, 'local')",
                            rusqlite::params![
                                explicit_id,
                                started_at,
                                ended_at,
                                duration_secs,
                                local_date,
                                local_hour,
                                process_name,
                                window_title,
                                category_id,
                                device_id,
                                remote_id,
                                updated_at,
                            ],
                        )
                        .db()?;
                    } else {
                        // 对端的数据：本机看做 remote 镜像，auto id
                        conn.execute(
                            "INSERT INTO activities(
                               started_at, ended_at, duration_secs, local_date, local_hour,
                               process_name, window_title, category_id, screenshot_path,
                               device_id, remote_id, updated_at, origin
                             ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?, 'remote')",
                            rusqlite::params![
                                started_at,
                                ended_at,
                                duration_secs,
                                local_date,
                                local_hour,
                                process_name,
                                window_title,
                                category_id,
                                device_id,
                                remote_id,
                                updated_at,
                            ],
                        )
                        .db()?;
                    }
                }
                Some((id, cur_updated)) => {
                    if updated_at > cur_updated {
                        conn.execute(
                            "UPDATE activities SET
                               started_at = ?, ended_at = ?, duration_secs = ?,
                               local_date = ?, local_hour = ?,
                               process_name = ?, window_title = ?, category_id = ?,
                               updated_at = ?
                             WHERE id = ?",
                            rusqlite::params![
                                started_at,
                                ended_at,
                                duration_secs,
                                local_date,
                                local_hour,
                                process_name,
                                window_title,
                                category_id,
                                updated_at,
                                id,
                            ],
                        )
                        .db()?;
                    }
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
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};

    const DAY: &str = "2026-05-15";
    const OTHER_DEVICE: &str = "device-a";

    /// 跨设备路径 (device_id != self_id)：
    /// - ndjson 中的 id 全部覆盖到 mirror（UPDATE 已存在 / INSERT 新行）
    /// - ndjson 中**不在**的 remote_id 通过 mirror 收敛 DELETE 掉
    #[tokio::test]
    async fn merge_activities_cross_device_converges_mirror() {
        let pool = fresh_test_pool().await;
        seed_mirror_rows(&pool, OTHER_DEVICE, &["1", "2", "3", "4", "5"]).await;

        let body = ndjson_for_ids(&[1, 2, 3, 6]);
        merge_activities(&pool, TEST_SELF_ID, OTHER_DEVICE, DAY, body.as_bytes())
            .await
            .unwrap();

        let ids = remote_ids_for(&pool, OTHER_DEVICE).await;
        assert_eq!(
            ids,
            vec!["1".to_string(), "2".into(), "3".into(), "6".into()],
            "ndjson 包含 1/2/3/6，不在的 4/5 应被 mirror 收敛 DELETE"
        );
    }

    /// 自身路径 (device_id == self_id)：mirror 收敛**不**触发，
    /// 自己原有的 mirror 行不该被对端 ndjson 收敛 DELETE。
    #[tokio::test]
    async fn merge_activities_self_skips_mirror_convergence() {
        let pool = fresh_test_pool().await;
        // 自身路径的 fixture 用 origin='local'（mac 本机刚写的状态），并显式 id
        // 保证 (device_id, remote_id) 唯一索引下能正确做 UPDATE
        seed_self_rows(&pool, &[1, 2, 3, 4, 5]).await;

        let body = ndjson_for_ids(&[1, 2, 3, 6]);
        merge_activities(&pool, TEST_SELF_ID, TEST_SELF_ID, DAY, body.as_bytes())
            .await
            .unwrap();

        let ids = remote_ids_for(&pool, TEST_SELF_ID).await;
        // 自身路径不收敛：原 1..5 应全部保留，外加新 INSERT 的 6 → 共 6 行
        assert_eq!(
            ids,
            vec![
                "1".to_string(),
                "2".into(),
                "3".into(),
                "4".into(),
                "5".into(),
                "6".into(),
            ],
            "self 路径下 mirror 收敛应跳过，4/5 应保留"
        );
    }

    /// 解析失败的 ndjson：mirror 收敛**不**触发，避免半截文件误删一大堆。
    #[tokio::test]
    async fn merge_activities_parse_failure_skips_mirror_convergence() {
        let pool = fresh_test_pool().await;
        seed_mirror_rows(&pool, OTHER_DEVICE, &["1", "2", "3", "4", "5"]).await;

        // 第二行故意写坏 JSON
        let body = format!(
            "{}\nthis is not valid json {{\n{}\n",
            payload_line(1),
            payload_line(2),
        );
        merge_activities(&pool, TEST_SELF_ID, OTHER_DEVICE, DAY, body.as_bytes())
            .await
            .unwrap();

        let ids = remote_ids_for(&pool, OTHER_DEVICE).await;
        assert_eq!(
            ids,
            vec![
                "1".to_string(),
                "2".into(),
                "3".into(),
                "4".into(),
                "5".into(),
            ],
            "解析失败时 mirror 收敛应跳过：原 5 行全部保留"
        );
    }

    fn payload_line(id: i64) -> String {
        let p = ActivityPayload {
            id,
            started_at: format!("{DAY}T10:0{id}:00Z"),
            ended_at: format!("{DAY}T10:0{id}:30Z"),
            duration_secs: 30,
            local_date: DAY.into(),
            local_hour: 10,
            process_name: "Code".into(),
            window_title: None,
            category_id: "other".into(),
            updated_at: format!("{DAY}T10:0{id}:30Z"),
        };
        serde_json::to_string(&p).unwrap()
    }

    fn ndjson_for_ids(ids: &[i64]) -> String {
        ids.iter()
            .map(|id| payload_line(*id))
            .collect::<Vec<_>>()
            .join("\n")
    }

    async fn seed_mirror_rows(pool: &DbPool, device_id: &str, remote_ids: &[&str]) {
        let device_id = device_id.to_string();
        let remote_ids: Vec<String> = remote_ids.iter().map(|s| s.to_string()).collect();
        pool.0
            .call(move |conn| {
                for r in &remote_ids {
                    conn.execute(
                        "INSERT INTO activities(
                            started_at, ended_at, duration_secs, local_date, local_hour,
                            process_name, window_title, category_id, device_id, remote_id,
                            updated_at, origin
                         ) VALUES(
                            '2026-05-15T10:00:00Z', '2026-05-15T10:00:30Z', 30, '2026-05-15', 10,
                            'Code', '', 'other', ?1, ?2, '2026-05-15T10:00:00Z', 'remote'
                         )",
                        rusqlite::params![device_id, r],
                    )
                    .db()?;
                }
                Ok(())
            })
            .await
            .unwrap();
    }

    /// 自身路径专用 fixture：origin='local' + 显式 id（对端 ndjson upsert 自己时用 id 对齐）
    async fn seed_self_rows(pool: &DbPool, ids: &[i64]) {
        let ids = ids.to_vec();
        pool.0
            .call(move |conn| {
                for id in &ids {
                    conn.execute(
                        "INSERT INTO activities(
                            id, started_at, ended_at, duration_secs, local_date, local_hour,
                            process_name, window_title, category_id, device_id, remote_id,
                            updated_at, origin
                         ) VALUES(
                            ?1, '2026-05-15T10:00:00Z', '2026-05-15T10:00:30Z', 30, '2026-05-15', 10,
                            'Code', '', 'other', ?2, ?3,
                            '2026-05-15T10:00:00Z', 'local'
                         )",
                        rusqlite::params![id, TEST_SELF_ID, id.to_string()],
                    )
                    .db()?;
                }
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn remote_ids_for(pool: &DbPool, device_id: &str) -> Vec<String> {
        let device_id = device_id.to_string();
        pool.0
            .call(move |conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT remote_id FROM activities
                         WHERE device_id = ?1 ORDER BY remote_id",
                    )
                    .db()?;
                let rows = stmt
                    .query_map(rusqlite::params![device_id], |r| r.get::<_, String>(0))
                    .db()?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r.db()?);
                }
                Ok(out)
            })
            .await
            .unwrap()
    }
}
