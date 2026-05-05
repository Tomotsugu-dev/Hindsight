use serde::{Deserialize, Serialize};

use crate::ai::config::AiConfig;
use crate::error::Result;
use crate::storage::{db_path_dir, DbPool};
use crate::db::SqliteResultExt;

pub fn default_screenshot_path() -> String {
    db_path_dir()
        .map(|p| p.join("screenshots").to_string_lossy().to_string())
        .unwrap_or_else(|_| String::new())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeRange {
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub capture_enabled: bool,
    pub capture_interval_seconds: u32,
    pub screenshot_path: String,
    pub work_hours_enabled: bool,
    pub work_ranges: Vec<TimeRange>,
    pub auto_start: bool,
    pub show_window_on_auto_start: bool,
    pub retention_days: u32,
    /// Google Cloud Console 创建的 Desktop App OAuth client_id（Drive 同步用）
    pub google_client_id: String,
    pub google_client_secret: String,
    /// 浏览器过滤：浏览器地址栏 URL 包含其中任意一条（子串忽略大小写）即跳过截图。
    /// 默认装一套常见登录页路径片段
    pub privacy_url_keywords: Vec<String>,
    /// 应用过滤：应用名或窗口标题包含其中任意一条（子串忽略大小写）即跳过截图。
    /// 默认空，用户自己加（如 微信、招商银行、特定文件名）
    pub privacy_app_keywords: Vec<String>,
    /// 关闭按钮（窗口右上角 X）的行为：true=隐藏到托盘，false=直接退出。
    /// 默认 true 是为了避免用户误点导致采集中断。
    pub minimize_to_tray: bool,
    /// 是否在 app 启动时自动检查更新。前端读这个 + auto_update_interval +
    /// last_update_check_at，决定要不要拉 latest.json。
    pub auto_update_enabled: bool,
    /// 自动检查的频率：daily / weekly / monthly / onstartup（每次启动）。
    /// 用字符串而不是枚举，避免新增选项时破坏旧 settings JSON 的反序列化。
    pub auto_update_interval: String,
    /// 上次检查更新的时刻（RFC3339）。前端检查后写一次。None 表示从未查过。
    pub last_update_check_at: Option<String>,
    /// 用户多久不动鼠键就算"挂机"，超过这个秒数 capture 不再延续当前会话，
    /// 避免离开电脑后还在累计使用时长。0 = 关闭挂机检测（永远算在用）。
    pub idle_threshold_seconds: u32,
    /// AI 总结相关配置（端点、模型、时段划分、过滤分类等）。
    /// 嵌套结构而不是平铺，因为是独立子系统，前端读取也整组。
    pub ai: AiConfig,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            capture_enabled: true,
            capture_interval_seconds: 30,
            screenshot_path: String::new(),
            work_hours_enabled: false,
            work_ranges: Vec::new(),
            auto_start: false,
            show_window_on_auto_start: false,
            retention_days: 7,
            google_client_id: String::new(),
            google_client_secret: String::new(),
            privacy_url_keywords: default_privacy_url_keywords(),
            privacy_app_keywords: Vec::new(),
            minimize_to_tray: true,
            auto_update_enabled: true,
            auto_update_interval: "weekly".to_string(),
            last_update_check_at: None,
            idle_threshold_seconds: 180,
            ai: AiConfig::default(),
        }
    }
}

/// 默认浏览器登录页 URL 路径片段；用户在隐私页可以增删。
/// 注意：匹配是"子串忽略大小写"，所以 `/password` 会顺带覆盖
/// `/passwords` / `/password-reset` 等所有变体，不需要额外加复数形式
pub fn default_privacy_url_keywords() -> Vec<String> {
    [
        "/login",
        "/signin",
        "/sign-in",
        "/sign_in",
        "/auth",
        "/oauth",
        "/sso",
        "/logon",
        "/connect/authorize",
        "/password",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub capture_enabled: Option<bool>,
    pub capture_interval_seconds: Option<u32>,
    pub screenshot_path: Option<String>,
    pub work_hours_enabled: Option<bool>,
    pub work_ranges: Option<Vec<TimeRange>>,
    pub auto_start: Option<bool>,
    pub show_window_on_auto_start: Option<bool>,
    pub retention_days: Option<u32>,
    pub google_client_id: Option<String>,
    pub google_client_secret: Option<String>,
    pub privacy_url_keywords: Option<Vec<String>>,
    pub privacy_app_keywords: Option<Vec<String>>,
    pub minimize_to_tray: Option<bool>,
    pub auto_update_enabled: Option<bool>,
    pub auto_update_interval: Option<String>,
    pub last_update_check_at: Option<Option<String>>,
    pub idle_threshold_seconds: Option<u32>,
    /// AI 配置整组覆盖；前端要么不传（保留旧值），要么传完整新值
    pub ai: Option<AiConfig>,
}

pub async fn load(pool: &DbPool) -> Result<Settings> {
    let data: String = pool
        .0
        .call(|conn| {
            let row: String = conn
                .query_row("SELECT data FROM settings_store WHERE id = 1", [], |r| {
                    r.get(0)
                })
                .db()?;
            Ok(row)
        })
        .await?;

    let mut settings = serde_json::from_str::<Settings>(&data).unwrap_or_default();
    let mut dirty = false;

    if settings.screenshot_path.trim().is_empty() {
        settings.screenshot_path = default_screenshot_path();
        dirty = true;
    }

    if settings.ai.models_path.trim().is_empty() {
        settings.ai.models_path = crate::ai::models::default_root_dir()
            .to_string_lossy()
            .into_owned();
        dirty = true;
    }

    if dirty {
        save(pool, &settings).await?;
    }

    Ok(settings)
}

pub async fn save(pool: &DbPool, settings: &Settings) -> Result<()> {
    let data = serde_json::to_string(settings)?;
    pool.0
        .call(move |conn| {
            conn.execute(
                "UPDATE settings_store SET data = ? WHERE id = 1",
                rusqlite::params![data],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub fn apply_patch(current: Settings, patch: SettingsPatch) -> Settings {
    Settings {
        capture_enabled: patch.capture_enabled.unwrap_or(current.capture_enabled),
        capture_interval_seconds: patch
            .capture_interval_seconds
            .map(|v| v.clamp(1, 600))
            .unwrap_or(current.capture_interval_seconds),
        screenshot_path: patch
            .screenshot_path
            .map(|p| {
                if p.trim().is_empty() {
                    default_screenshot_path()
                } else {
                    p
                }
            })
            .unwrap_or(current.screenshot_path),
        work_hours_enabled: patch
            .work_hours_enabled
            .unwrap_or(current.work_hours_enabled),
        work_ranges: patch.work_ranges.unwrap_or(current.work_ranges),
        auto_start: patch.auto_start.unwrap_or(current.auto_start),
        show_window_on_auto_start: patch
            .show_window_on_auto_start
            .unwrap_or(current.show_window_on_auto_start),
        retention_days: patch
            .retention_days
            .map(|v| v.clamp(1, 365))
            .unwrap_or(current.retention_days),
        google_client_id: patch
            .google_client_id
            .map(|v| v.trim().to_string())
            .unwrap_or(current.google_client_id),
        google_client_secret: patch
            .google_client_secret
            .map(|v| v.trim().to_string())
            .unwrap_or(current.google_client_secret),
        privacy_url_keywords: patch
            .privacy_url_keywords
            .map(sanitize_keywords)
            .unwrap_or(current.privacy_url_keywords),
        privacy_app_keywords: patch
            .privacy_app_keywords
            .map(sanitize_keywords)
            .unwrap_or(current.privacy_app_keywords),
        minimize_to_tray: patch
            .minimize_to_tray
            .unwrap_or(current.minimize_to_tray),
        auto_update_enabled: patch
            .auto_update_enabled
            .unwrap_or(current.auto_update_enabled),
        auto_update_interval: patch
            .auto_update_interval
            .map(|v| sanitize_interval(&v))
            .unwrap_or(current.auto_update_interval),
        last_update_check_at: patch
            .last_update_check_at
            .unwrap_or(current.last_update_check_at),
        idle_threshold_seconds: patch
            .idle_threshold_seconds
            // 0 = 关闭检测；上限 3600 (1h) 防止用户填怪值
            .map(|v| v.min(3600))
            .unwrap_or(current.idle_threshold_seconds),
        ai: patch
            .ai
            .map(|new_ai| crate::ai::config::sanitize(new_ai, &current.ai))
            .unwrap_or(current.ai),
    }
}

/// 把 UI 传来的 interval 字符串收敛到合法集合，非法值回退 weekly
fn sanitize_interval(v: &str) -> String {
    match v {
        "daily" | "weekly" | "monthly" | "onstartup" => v.to_string(),
        _ => "weekly".to_string(),
    }
}

/// 关键词清洗：trim + 去空 + 去重（保持原顺序）
fn sanitize_keywords(list: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    list.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && seen.insert(s.clone()))
        .collect()
}
