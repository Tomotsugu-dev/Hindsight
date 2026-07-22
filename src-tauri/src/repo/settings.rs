//! 全局设置（settings_store 表，单行 JSON BLOB）的 repo 层。
//!
//! 用整 JSON 而不是逐字段建列：字段加得很快，迁移成本要等于 0。
//! 反序列化失败 / 字段缺失 → 走 `Default`；新加字段时只需给 default 值。

use serde::{Deserialize, Serialize};

use crate::ai::config::AiConfig;
use crate::error::Result;
use crate::storage::SqliteResultExt;
use crate::storage::{db_path_dir, DbPool};

/// 系统默认截图目录：`<data_root>/screenshots`。
/// 用户在「设置 → 数据」可改成大硬盘上的目录。
pub fn default_screenshot_path() -> String {
    db_path_dir()
        .map(|p| p.join("screenshots").to_string_lossy().to_string())
        .unwrap_or_else(|_| String::new())
}

/// 工作时段的一段时间（HH:MM-HH:MM）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeRange {
    /// 起始时刻 `HH:MM`
    pub start: String,
    /// 结束时刻 `HH:MM`；允许跨午夜（end < start 时表示"到次日"）
    pub end: String,
}

/// 全局设置主结构。整组 JSON 落 settings_store 单行；前端读 `get_settings` 拿全集。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub capture_enabled: bool,
    /// 截图独立开关——关掉只停截图，窗口 / 应用切换记录继续。
    /// 默认 true；老 settings JSON 缺这个字段时（`#[serde(default)]` 走 false）
    /// 会被 [`load`] 启动期 sanitize 修正成 true，避免老用户升级后突然没截图。
    pub screenshot_enabled: bool,
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
    /// 屏幕记忆的 OCR 常驻模式：true = OCR 引擎常驻内存、新截图准实时消化
    /// （多占约 400MB 内存）；false（默认）= 批量模式，仅手动/定时消化时
    /// 加载引擎，用完即释放。
    pub memory_ocr_resident: bool,
    /// Chat 首次发送前的隐私确认(展示当前路由的模型与发送内容说明)。
    /// 确认过一次即永久 true,不再弹。
    pub chat_privacy_acknowledged: bool,
    /// 可选上云三挡(默认全 false)。打开 = 该数据集参与云同步的推与拉;
    /// 前端在开启时弹隐私警告。截图本体永不上云,与这三挡无关。
    /// AI 总结文本(日报/周报)。
    pub sync_ai_summaries: bool,
    /// 聊天历史(会话+消息,含屏幕文字引用)。
    pub sync_chat_history: bool,
    /// 屏幕记忆全文(OCR 出的屏幕逐字文本,敏感度最高)。
    pub sync_screen_memory: bool,
    /// AI 总结相关配置（端点、模型、时段划分、过滤分类等）。
    /// 嵌套结构而不是平铺，因为是独立子系统，前端读取也整组。
    pub ai: AiConfig,
    /// 云端截图洞察(docs/design/cloud-insight.md)。默认关;
    /// 开启前必须过同意门(insight_consent_acknowledged)。
    pub insight_enabled: bool,
    /// 同意门:用户确认过"截图将上传至自配服务商"。确认一次永久 true。
    pub insight_consent_acknowledged: bool,
    /// 分析范围:"focus"(仅重点应用逐帧) / "recommended"(重点逐帧+其余 5 分钟一帧)
    /// / "all"(全应用逐帧)。非法值 sanitize 回 "recommended"。
    pub insight_scope: String,
    /// 重点应用 process_name 列表(逐独立帧、1280px 分析)。
    pub insight_focus_apps: Vec<String>,
    /// 每日分析帧数上限,超限自动停到次日。
    pub insight_daily_frame_cap: u32,
    /// 常驻洞察只处理该时刻(RFC3339)之后登记的帧——开启功能时打点,
    /// 避免一开启就静默分析整个保留窗的存量截图(那是"历史回填"的事,要显式确认)。
    pub insight_since_ts: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            capture_enabled: true,
            // 默认关：截图涉及隐私 + Apple TCC 弹框 + 多屏多 Space 的边界 case，
            // 设计上"explicitly opt-in"——用户去 设置 → 通用 → 启用截图 主动开。
            // v23 migration 同步把存量用户的 screenshotEnabled 也重置成 false。
            screenshot_enabled: false,
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
            memory_ocr_resident: false,
            chat_privacy_acknowledged: false,
            sync_ai_summaries: false,
            sync_chat_history: false,
            sync_screen_memory: false,
            ai: AiConfig::default(),
            insight_enabled: false,
            insight_consent_acknowledged: false,
            insight_scope: "recommended".to_string(),
            insight_focus_apps: Vec::new(),
            insight_daily_frame_cap: 2000,
            insight_since_ts: None,
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

/// 增量更新 settings 的 patch。每个字段 None 表示保持当前值。
/// 结构镜像 [`Settings`]，前端在 update_settings 命令里只传要改的子集。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub capture_enabled: Option<bool>,
    pub screenshot_enabled: Option<bool>,
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
    pub memory_ocr_resident: Option<bool>,
    pub chat_privacy_acknowledged: Option<bool>,
    pub sync_ai_summaries: Option<bool>,
    pub sync_chat_history: Option<bool>,
    pub sync_screen_memory: Option<bool>,
    /// AI 配置整组覆盖；前端要么不传（保留旧值），要么传完整新值
    pub ai: Option<AiConfig>,
    pub insight_enabled: Option<bool>,
    pub insight_consent_acknowledged: Option<bool>,
    pub insight_scope: Option<String>,
    pub insight_focus_apps: Option<Vec<String>>,
    pub insight_daily_frame_cap: Option<u32>,
    pub insight_since_ts: Option<Option<String>>,
}

/// 读 settings_store 单行 + 反序列化。
/// 缺字段走 `#[serde(default)]` 补默认；空截图路径自动填默认值并写回。
///
/// **JSON 整体解析失败**（字段类型对不上 / 写了一半被截断）时：内存里用默认值让
/// 应用能起，但**绝不回写**——旧实现 `unwrap_or_default()` + dirty 保存会把用户全部
/// 设置（工作时段 / 隐私关键词 / API key / AI 参数）一次性覆盖成默认且不可恢复。
/// 现在原始 JSON 先备份到数据目录再继续，等下一个能读懂它的版本或用户手工救回。
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

    let (mut settings, parse_failed) = match serde_json::from_str::<Settings>(&data) {
        Ok(s) => (s, false),
        Err(e) => {
            log::error!("settings JSON 解析失败（本次使用默认值、不回写）: {e}");
            if let Ok(dir) = crate::storage::db_path_dir() {
                let backup = dir.join("settings_store.corrupt.json");
                match std::fs::write(&backup, &data) {
                    Ok(()) => log::error!("原始 settings 已备份到 {}", backup.display()),
                    Err(we) => log::error!("备份原始 settings 失败: {we}"),
                }
            }
            (Settings::default(), true)
        }
    };
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

    // 旧版本里 `external_enabled=true` 单一开关同时表示「云端配好」+「step 2 走云端」。
    // 新版本把"是否选定云端"剥离到 `summary_main == SUMMARY_CLOUD_SENTINEL`。
    // 一次性迁移：之前启用了云端且没设本地 summary main 的用户，自动补上 sentinel，
    // 保持旧行为。已经设本地 summary main 的用户保留本地选择（更接近他们的实际意图）。
    if settings.ai.external_enabled && settings.ai.summary_main.trim().is_empty() {
        settings.ai.summary_main = crate::ai::config::SUMMARY_CLOUD_SENTINEL.to_string();
        dirty = true;
    }

    // 一次性迁移:日报管线从"截图描述"换代为"活动时间线"后,旧管线时代保存的
    // system prompt 覆盖必然与新输入格式错配(实测会导致输出混乱 + 旧示例文本泄漏)。
    // 按旧提示词的特征串识别并清空,让用户回落到新内置默认;用户后续在提示词页
    // 写的新覆盖不含这些特征串,不会被误清。
    {
        const STALE_MARKERS: [&str; 5] = [
            "截图的逐张描述",
            "截圖的逐張描述",
            "per-screenshot descriptions",
            "スクリーンショット逐次描写",
            "descrições individuais das capturas",
        ];
        let po = &mut settings.ai.prompt_overrides;
        for field in [
            &mut po.system_zh,
            &mut po.system_tw,
            &mut po.system_en,
            &mut po.system_ja,
            &mut po.system_pt,
        ] {
            if !field.is_empty() && STALE_MARKERS.iter().any(|m| field.contains(m)) {
                log::info!("清除旧管线时代的 system prompt 覆盖(与活动时间线输入不兼容)");
                field.clear();
                dirty = true;
            }
        }
    }

    // 解析失败时的 dirty 全是"默认值缺路径"造成的，绝不能把这份默认值写回去
    // 覆盖用户仅存的原始 JSON。
    if dirty && !parse_failed {
        save(pool, &settings).await?;
    }

    Ok(settings)
}

/// 整组覆盖 settings_store。调用方应先 [`load`] 再传 patch 后的完整 [`Settings`]。
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

/// 把 [`SettingsPatch`] 应用到当前 [`Settings`] 上，输出合并结果。
/// 各字段都做合理 clamp / sanitize（如 capture_interval 钳到 1..=600，retention 钳到 1..=365）。
pub fn apply_patch(current: Settings, patch: SettingsPatch) -> Settings {
    Settings {
        capture_enabled: patch.capture_enabled.unwrap_or(current.capture_enabled),
        screenshot_enabled: patch
            .screenshot_enabled
            .unwrap_or(current.screenshot_enabled),
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
        minimize_to_tray: patch.minimize_to_tray.unwrap_or(current.minimize_to_tray),
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
        memory_ocr_resident: patch
            .memory_ocr_resident
            .unwrap_or(current.memory_ocr_resident),
        chat_privacy_acknowledged: patch
            .chat_privacy_acknowledged
            .unwrap_or(current.chat_privacy_acknowledged),
        sync_ai_summaries: patch.sync_ai_summaries.unwrap_or(current.sync_ai_summaries),
        sync_chat_history: patch.sync_chat_history.unwrap_or(current.sync_chat_history),
        sync_screen_memory: patch
            .sync_screen_memory
            .unwrap_or(current.sync_screen_memory),
        ai: patch
            .ai
            .map(|new_ai| crate::ai::config::sanitize(new_ai, &current.ai))
            .unwrap_or(current.ai),
        insight_enabled: patch.insight_enabled.unwrap_or(current.insight_enabled),
        insight_consent_acknowledged: patch
            .insight_consent_acknowledged
            .unwrap_or(current.insight_consent_acknowledged),
        insight_scope: patch
            .insight_scope
            .map(|v| sanitize_insight_scope(&v))
            .unwrap_or(current.insight_scope),
        insight_focus_apps: patch
            .insight_focus_apps
            .map(sanitize_keywords)
            .unwrap_or(current.insight_focus_apps),
        insight_daily_frame_cap: patch
            .insight_daily_frame_cap
            .map(|v| v.clamp(100, 10_000))
            .unwrap_or(current.insight_daily_frame_cap),
        insight_since_ts: patch.insight_since_ts.unwrap_or(current.insight_since_ts),
    }
}

/// 洞察分析范围收敛到合法集合,非法值回退 recommended。
fn sanitize_insight_scope(v: &str) -> String {
    match v {
        "focus" | "recommended" | "all" => v.to_string(),
        _ => "recommended".to_string(),
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
