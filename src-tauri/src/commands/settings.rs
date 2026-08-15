use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

use crate::capture::ignore::IgnoreRule;
use crate::capture::CaptureService;
use crate::commands::screen_memory::MemoryState;
use crate::memory::resident::ResidentOcr;
use crate::repo::settings::{self, Settings, SettingsPatch};
use crate::storage::DbPool;
use crate::sync::engine::SyncEngine;

/// 拉当前 Settings 全集——前端「设置」页面进去时调一次。
#[tauri::command]
pub async fn get_settings(pool: State<'_, DbPool>) -> Result<Settings, String> {
    settings::load(&pool).await.map_err(Into::into)
}

/// 应用 patch 更新部分 settings 字段。
///
/// 副作用：把 capture 相关字段同步给 `CaptureService`（间隔 / 工作时段 / 隐私关键词
/// / 挂机阈值 / 截图配置），把 minimize_to_tray 同步给 close handler 静态变量，
/// 把 auto_start 切到操作系统的开机自启。所有变更立刻生效，不需要重启。
#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    pool: State<'_, DbPool>,
    svc: State<'_, Arc<CaptureService>>,
    resident: State<'_, Arc<ResidentOcr>>,
    mem: State<'_, MemoryState>,
    sync_engine: State<'_, Arc<SyncEngine>>,
    patch: SettingsPatch,
) -> Result<Settings, String> {
    let current = settings::load(&pool).await.map_err(String::from)?;

    let prev_enabled = current.capture_enabled;
    let prev_interval = current.capture_interval_seconds;
    let prev_autostart = current.auto_start;
    let prev_resident = current.memory_ocr_resident;
    let prev_opt_sync = (
        current.sync_ai_summaries,
        current.sync_chat_history,
        current.sync_screen_memory,
    );

    let next = settings::apply_patch(current, patch);
    settings::save(&pool, &next).await.map_err(String::from)?;

    // 关闭按钮行为切换：同步给 close handler 读的 static，下次点 X 立即生效，
    // 不需要重启
    crate::MINIMIZE_TO_TRAY.store(next.minimize_to_tray, std::sync::atomic::Ordering::Relaxed);

    if next.capture_enabled != prev_enabled {
        if next.capture_enabled {
            svc.start().await;
        } else {
            svc.stop().await;
        }
    }
    if next.capture_interval_seconds != prev_interval {
        svc.set_interval(next.capture_interval_seconds).await;
    }
    svc.set_work_hours(next.work_hours_enabled, next.work_ranges.clone())
        .await;
    svc.set_screenshot_config(
        next.screenshot_enabled,
        next.screenshot_path.clone(),
        // 存档规格与 bootstrap 保持一致(screen-memory.md L2 定案):≤2880/q85
        2880,
        2880,
        85,
    )
    .await;
    svc.set_privacy_keywords(
        next.privacy_url_keywords.clone(),
        next.privacy_app_keywords.clone(),
    )
    .await;
    svc.set_idle_threshold(next.idle_threshold_seconds).await;

    if next.auto_start != prev_autostart {
        let mgr = app.autolaunch();
        let res = if next.auto_start {
            mgr.enable()
        } else {
            mgr.disable()
        };
        if let Err(e) = res {
            log::warn!("切换开机自启失败: {e}");
        }
    }

    // OCR 常驻开关:启停立即生效,不需要重启
    if next.memory_ocr_resident != prev_resident {
        resident.sync(next.memory_ocr_resident, mem.0.clone()).await;
    }

    // 可选上云三挡任一从关到开:重置 pull 游标让 Drive 上的历史文件重新入列
    // (关到开之前这些文件被标 handled 越过了;合并幂等,重拉无害)
    let turned_on = (!prev_opt_sync.0 && next.sync_ai_summaries)
        || (!prev_opt_sync.1 && next.sync_chat_history)
        || (!prev_opt_sync.2 && next.sync_screen_memory);
    if turned_on {
        if let Err(e) = sync_engine.reset_pull_cursor().await {
            log::warn!("重置 pull 游标失败(下轮 pull 拉不到历史文件): {e}");
        }
    }

    Ok(next)
}

/// add/remove_ignore_rule 的返回：更新后的规则全集 + 本次重算改动的历史行数。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IgnoreRulesResult {
    pub rules: Vec<IgnoreRule>,
    /// reapply 改动的 activities 行数，前端 toast 用（"已排除 N 条历史记录"）
    pub reapplied_rows: u64,
}

/// 新增一条忽略规则（进程 + 可选标题关键词），幂等：等价规则已存在时不重复添加。
/// 不走 update_settings 的整包 patch——规则只有 add/remove 这一条写路，
/// 设置页整包保存不会覆盖就地添加的规则。保存后同步 CaptureService（新会话即时
/// 生效）并全表重算历史行的 excluded 标记（含 pull 同步来的镜像行）。
#[tauri::command]
pub async fn add_ignore_rule(
    pool: State<'_, DbPool>,
    svc: State<'_, Arc<CaptureService>>,
    process_name: String,
    title_keyword: Option<String>,
) -> Result<IgnoreRulesResult, String> {
    let process = process_name.trim().to_string();
    if process.is_empty() {
        return Err("进程名不能为空".into());
    }
    // Some("") 不静默升级成「忽略整个应用」——那是 None 的语义，必须显式传。
    // 否则 UI 一次手滑（空输入框提交）就把整个应用从统计里排掉，且无任何提示。
    let keyword = match title_keyword {
        None => None,
        Some(k) => {
            let k = k.trim().to_string();
            if k.is_empty() {
                return Err("标题关键词不能为空；要忽略整个应用请传 null".into());
            }
            Some(k)
        }
    };
    let mut cfg = settings::load(&pool).await.map_err(String::from)?;
    let dup = cfg
        .ignore_rules
        .iter()
        .any(|r| rule_eq(r, &process, keyword.as_deref()));
    if !dup {
        cfg.ignore_rules.push(IgnoreRule {
            process_name: process,
            title_keyword: keyword,
        });
        settings::save(&pool, &cfg).await.map_err(String::from)?;
    }
    finish_ignore_rules_change(&pool, &svc, cfg).await
}

/// 删除一条忽略规则（按 进程+关键词 等价匹配）。幂等：不存在时也返回成功。
/// 删除后 reapply 会把历史行的标记清回来（可逆是 excluded 相对「不记录」的核心差异）。
#[tauri::command]
pub async fn remove_ignore_rule(
    pool: State<'_, DbPool>,
    svc: State<'_, Arc<CaptureService>>,
    process_name: String,
    title_keyword: Option<String>,
) -> Result<IgnoreRulesResult, String> {
    let process = process_name.trim().to_string();
    let keyword = title_keyword
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty());
    let mut cfg = settings::load(&pool).await.map_err(String::from)?;
    let before = cfg.ignore_rules.len();
    cfg.ignore_rules
        .retain(|r| !rule_eq(r, &process, keyword.as_deref()));
    if cfg.ignore_rules.len() != before {
        settings::save(&pool, &cfg).await.map_err(String::from)?;
    }
    finish_ignore_rules_change(&pool, &svc, cfg).await
}

/// 规则等价比较：进程与关键词都 trim + 全 Unicode lowercase，与
/// `ignore::is_excluded` 的匹配归一化一致——「看着一样的规则」不会因大小写重复入表。
fn rule_eq(r: &IgnoreRule, process: &str, keyword: Option<&str>) -> bool {
    fn norm(s: &str) -> String {
        s.trim().to_lowercase()
    }
    if norm(&r.process_name) != norm(process) {
        return false;
    }
    match (r.title_keyword.as_deref(), keyword) {
        (None, None) => true,
        (Some(a), Some(b)) => norm(a) == norm(b),
        _ => false,
    }
}

/// 规则增删后的共同收尾：推给采集服务（新会话即时生效）+ 全表重算历史标记。
async fn finish_ignore_rules_change(
    pool: &DbPool,
    svc: &CaptureService,
    cfg: Settings,
) -> Result<IgnoreRulesResult, String> {
    svc.set_ignore_rules(cfg.ignore_rules.clone()).await;
    let reapplied = crate::repo::activities::reapply_ignore_rules(pool, &cfg.ignore_rules)
        .await
        .map_err(String::from)?;
    Ok(IgnoreRulesResult {
        rules: cfg.ignore_rules,
        reapplied_rows: reapplied,
    })
}
