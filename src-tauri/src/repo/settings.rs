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
    /// 定时补识别(旧单时刻字段,仅为兼容历史配置保留读取;新写入一律走
    /// [`Self::memory_ocr_daily_times`],patch 写 times 时本字段被清空)。
    #[serde(default)]
    pub memory_ocr_daily_at: Option<String>,
    /// 定时补识别的时间点列表("HH:MM",**保持添加顺序**,上限 6):每天到达
    /// 任一时间点即自动批量识别堆积的未处理截图,与常驻开关无关。
    /// 空(默认)= 功能关闭。当天错过的点下次启动后补;每个点每天至多跑一次。
    #[serde(default)]
    pub memory_ocr_daily_times: Vec<String>,
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
}

impl Settings {
    /// 定时补识别的**有效**时间点:新列表优先;列表为空时回落旧单时刻字段
    /// (老配置升级不丢定时)。调度器/前端只认这个。
    pub fn effective_ocr_daily_times(&self) -> Vec<String> {
        if !self.memory_ocr_daily_times.is_empty() {
            return self.memory_ocr_daily_times.clone();
        }
        self.memory_ocr_daily_at.clone().into_iter().collect()
    }
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
            memory_ocr_daily_at: None,
            memory_ocr_daily_times: Vec::new(),
            chat_privacy_acknowledged: false,
            sync_ai_summaries: false,
            sync_chat_history: false,
            sync_screen_memory: false,
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
    /// 双层 Option:外层 = patch 是否带此字段,内层 = 设为某时刻 / null 清除。
    /// 裸 Option<Option<T>> 会把 null 解成外层 None(与缺席不可区分),
    /// 必须走 [`double_option`] 才能保住"null = 显式清除"。
    #[serde(default, deserialize_with = "double_option")]
    pub memory_ocr_daily_at: Option<Option<String>>,
    pub memory_ocr_daily_times: Option<Vec<String>>,
    pub chat_privacy_acknowledged: Option<bool>,
    pub sync_ai_summaries: Option<bool>,
    pub sync_chat_history: Option<bool>,
    pub sync_screen_memory: Option<bool>,
    /// AI 配置整组覆盖；前端要么不传（保留旧值），要么传完整新值
    pub ai: Option<AiConfig>,
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
        // times 与旧单时刻字段互斥:patch 写 times 即清旧字段(避免两处真源)
        memory_ocr_daily_at: if patch.memory_ocr_daily_times.is_some() {
            None
        } else {
            patch
                .memory_ocr_daily_at
                .map(|v| v.and_then(sanitize_hhmm))
                .unwrap_or(current.memory_ocr_daily_at)
        },
        memory_ocr_daily_times: patch
            .memory_ocr_daily_times
            .map(sanitize_hhmm_list)
            .unwrap_or(current.memory_ocr_daily_times),
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
    }
}

/// 把 UI 传来的 interval 字符串收敛到合法集合，非法值回退 weekly
/// 区分「字段缺席」与「显式 null」:字段出现(哪怕是 null)时把内层
/// Option 包上 Some;缺席走 `#[serde(default)]` 得外层 None。
fn double_option<'de, T, D>(de: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(de).map(Some)
}

/// "HH:MM" 校验:合法保留(trim 后),非法归 None(= 关闭定时)。
fn sanitize_hhmm_list(v: Vec<String>) -> Vec<String> {
    // 逐项校验、去重保首见、**保持添加顺序**(减号 = 减最近添加),上限 6
    let mut out: Vec<String> = Vec::new();
    for item in v {
        if let Some(t) = sanitize_hhmm(item) {
            if !out.contains(&t) {
                out.push(t);
            }
        }
        if out.len() >= 6 {
            break;
        }
    }
    out
}

fn sanitize_hhmm(v: String) -> Option<String> {
    let v = v.trim().to_string();
    chrono::NaiveTime::parse_from_str(&v, "%H:%M")
        .ok()
        .map(|_| v)
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::fresh_test_pool;

    /// 直接改写 settings_store 的原始 JSON，模拟「老版本写的数据」「损坏数据」
    /// 等 load 之外的写入来源。
    async fn put_raw(pool: &DbPool, json: &str) {
        let json = json.to_string();
        pool.0
            .call(move |conn| {
                conn.execute(
                    "UPDATE settings_store SET data = ?1 WHERE id = 1",
                    rusqlite::params![json],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// 读 settings_store 当前原始 JSON，用来验证「回写 / 绝不回写」两类行为。
    async fn raw_json(pool: &DbPool) -> String {
        pool.0
            .call(|conn| {
                let s: String = conn
                    .query_row("SELECT data FROM settings_store WHERE id = 1", [], |r| {
                        r.get(0)
                    })
                    .db()?;
                Ok(s)
            })
            .await
            .unwrap()
    }

    /// 全 None 的 patch。特意从 "{}" 反序列化而不是手写结构体：前端
    /// update_settings 只传要改的字段子集，如果未来有人往 SettingsPatch 加了
    /// 非 Option 字段，前端所有「只改一项」的请求都会反序列化失败——这里先炸。
    fn empty_patch() -> SettingsPatch {
        serde_json::from_str("{}").expect("空 JSON 应能反序列化成全 None 的 patch")
    }

    /// settings 是这些配置唯一的持久层，序列化/反序列化任何一个字段不对称，
    /// 表现都是「用户设置在重启后悄悄丢失」且没人报错。整组自定义值 save→load
    /// 后必须逐字节等价（用 JSON Value 比较绕开 Settings 没有 PartialEq）。
    #[tokio::test]
    async fn save_then_load_roundtrips_every_field() {
        let pool = fresh_test_pool().await;
        let custom = Settings {
            capture_enabled: false,
            screenshot_enabled: true,
            capture_interval_seconds: 45,
            // 路径非空，load 不会触发默认路径回填，往返应原样保留
            screenshot_path: "/tmp/hs-shots".into(),
            work_hours_enabled: true,
            work_ranges: vec![
                TimeRange {
                    start: "09:00".into(),
                    end: "12:30".into(),
                },
                // 跨午夜段（end < start）是文档明确允许的形态，必须原样存取
                TimeRange {
                    start: "23:30".into(),
                    end: "01:00".into(),
                },
            ],
            auto_start: true,
            show_window_on_auto_start: true,
            retention_days: 90,
            google_client_id: "cid-123".into(),
            google_client_secret: "sec-456".into(),
            privacy_url_keywords: vec!["/checkout".into()],
            privacy_app_keywords: vec!["微信".into()],
            minimize_to_tray: false,
            auto_update_enabled: false,
            auto_update_interval: "daily".into(),
            last_update_check_at: Some("2026-07-01T00:00:00+00:00".into()),
            idle_threshold_seconds: 0,
            memory_ocr_resident: true,
            memory_ocr_daily_at: Some("03:30".to_string()),
            memory_ocr_daily_times: vec!["12:00".to_string(), "23:00".to_string()],
            chat_privacy_acknowledged: true,
            sync_ai_summaries: true,
            sync_chat_history: true,
            sync_screen_memory: true,
            ai: AiConfig {
                endpoint: "https://api.example.com/v1".into(),
                model: "test-model".into(),
                api_key: "sk-test".into(),
                // false：避免触发 load 里的 cloud sentinel 一次性迁移，
                // 该迁移有专门测试
                external_enabled: false,
                user_brief: "写代码".into(),
                models_path: "/tmp/hs-models".into(),
                active_main: "main.gguf".into(),
                summary_main: "sum.gguf".into(),
                batch_size: Some(256),
                summary_ctx_size: Some(4096),
                ..AiConfig::default()
            },
        };

        save(&pool, &custom).await.unwrap();
        let loaded = load(&pool).await.unwrap();

        assert_eq!(
            serde_json::to_value(&custom).unwrap(),
            serde_json::to_value(&loaded).unwrap(),
            "save→load 往返后应逐字段等价"
        );
        // 跨午夜段单独点名：这是最容易被「start 必须 < end 校验」误伤的形态
        assert_eq!(loaded.work_ranges[1].start, "23:30");
        assert_eq!(loaded.work_ranges[1].end, "01:00");
    }

    /// 全新安装：migrations 只塞了 '{}'。首次 load 必须能把空路径补成实际默认
    /// 并回写持久化——否则每次启动都 dirty 一次，且截图落盘路径为空会写失败。
    #[tokio::test]
    async fn first_load_on_fresh_db_fills_paths_and_persists() {
        let pool = fresh_test_pool().await;
        let first = load(&pool).await.unwrap();

        // 空路径必须被填成 <data_root>/screenshots，而不是留空
        assert!(!first.screenshot_path.trim().is_empty());
        assert!(
            first.screenshot_path.ends_with("screenshots"),
            "默认截图路径应指向 screenshots 子目录，实际: {}",
            first.screenshot_path
        );
        assert!(!first.ai.models_path.trim().is_empty());

        // 其余字段走 Default 契约
        assert_eq!(first.retention_days, Settings::default().retention_days);
        assert_eq!(first.privacy_url_keywords, default_privacy_url_keywords());

        // 回填结果已落库：老实现只改内存不回写，导致每次启动重复 dirty
        let v: serde_json::Value = serde_json::from_str(&raw_json(&pool).await).unwrap();
        assert_eq!(
            v["screenshotPath"].as_str(),
            Some(first.screenshot_path.as_str())
        );

        // 更新后再读一致：第二次 load 不应再产生任何变化
        let second = load(&pool).await.unwrap();
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(&second).unwrap()
        );
    }

    /// 老版本 JSON 只有当年存在的字段；升级后 load 必须「认识的字段照用、
    /// 缺的字段回 Default、不认识的字段忽略」，而不是报错把用户设置清零。
    /// 未知字段那半边对应「用户从新版本降级回旧版本」的场景。
    #[tokio::test]
    async fn old_json_missing_and_unknown_fields_fall_back_to_defaults() {
        let pool = fresh_test_pool().await;
        put_raw(
            &pool,
            r#"{"captureEnabled":false,"captureIntervalSeconds":60,"retentionDays":30,"screenshotPath":"/data/shots","fieldFromTheFuture":{"x":1}}"#,
        )
        .await;

        let s = load(&pool).await.unwrap();

        // 老 JSON 里写了的字段：原样生效
        assert!(!s.capture_enabled);
        assert_eq!(s.capture_interval_seconds, 60);
        assert_eq!(s.retention_days, 30);
        assert_eq!(s.screenshot_path, "/data/shots", "非空路径不应被默认值覆盖");

        // 老 JSON 里缺的字段：回 Default，而不是 bool=false / 数字=0 之类的零值
        let d = Settings::default();
        assert_eq!(s.screenshot_enabled, d.screenshot_enabled);
        assert_eq!(s.minimize_to_tray, d.minimize_to_tray);
        assert_eq!(s.auto_update_interval, d.auto_update_interval);
        assert_eq!(s.privacy_url_keywords, default_privacy_url_keywords());
        assert!(s.work_ranges.is_empty());
        // 整个 ai 块缺失 → 嵌套结构整组回 Default（models_path 除外，会被回填）
        assert_eq!(
            s.ai.external_provider,
            AiConfig::default().external_provider
        );
        assert_eq!(
            s.ai.excluded_categories,
            AiConfig::default().excluded_categories
        );
        assert!(!s.ai.models_path.trim().is_empty());

        // 回填触发的回写不能弄丢用户已有的显式设置
        let v: serde_json::Value = serde_json::from_str(&raw_json(&pool).await).unwrap();
        assert_eq!(v["captureIntervalSeconds"].as_u64(), Some(60));
        assert_eq!(v["screenshotPath"].as_str(), Some("/data/shots"));
    }

    /// 真实翻车过的场景：settings JSON 整体解析失败时，旧实现 unwrap_or_default
    /// + dirty 回写会把用户全部设置一次性覆盖成默认且不可恢复。现在的契约是：
    /// 内存用默认值让应用能起、DB 原文一字不动、原文另备份一份等救回。
    #[tokio::test]
    // 跨测试的 env 串行锁必须罩住整个 async 测试体;每个 #[tokio::test] 独享
    // 单线程 runtime,不存在同线程等锁的死锁面,std Mutex 正是这里要的语义。
    #[allow(clippy::await_holding_lock)]
    async fn corrupt_json_boots_with_defaults_but_never_clobbers_db() {
        // HINDSIGHT_DATA_DIR 是进程级全局，device.rs 的 env 覆盖测试也在改它；
        // 必须拿共享锁串行，否则并行调度下双方互相踩对方的目录断言（随机红）。
        let _env = crate::repo::test_util::lock_data_dir_env();
        // 把 data_root 指到临时目录，让备份文件写进隔离位置而不是真实数据目录
        struct EnvGuard(Option<String>);
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                match &self.0 {
                    Some(v) => std::env::set_var("HINDSIGHT_DATA_DIR", v),
                    None => std::env::remove_var("HINDSIGHT_DATA_DIR"),
                }
            }
        }
        let _guard = EnvGuard(std::env::var("HINDSIGHT_DATA_DIR").ok());
        let tmp = std::env::temp_dir().join(format!(
            "hindsight-settings-corrupt-test-{}",
            std::process::id()
        ));
        std::env::set_var("HINDSIGHT_DATA_DIR", &tmp);

        let pool = fresh_test_pool().await;
        // 类型错 + 截断（少右括号）：模拟「写了一半掉电」的最恶劣形态
        let corrupt = r#"{"captureEnabled": "not-a-bool", "retentionDays": 30"#;
        put_raw(&pool, corrupt).await;

        // 必须 Ok：解析失败只能降级，不能让整个应用起不来
        let s = load(&pool).await.unwrap();
        // 整体解析失败 → 全部回默认；retentionDays=30 不应被「部分解析」捡出来
        let d = Settings::default();
        assert_eq!(s.retention_days, d.retention_days);
        assert_eq!(s.capture_enabled, d.capture_enabled);

        // 核心回归：DB 里的原文一字未动（默认值绝不回写覆盖用户仅存的原始 JSON）
        assert_eq!(raw_json(&pool).await, corrupt);

        // 原文已备份到数据目录，等下一个能读懂它的版本或用户手工救回
        let backup = tmp.join("settings_store.corrupt.json");
        assert_eq!(
            std::fs::read_to_string(&backup).expect("应存在备份文件"),
            corrupt
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// external_enabled 单开关拆成「配好云端」+「选定云端」两概念后的一次性迁移：
    /// 老用户开了云端且没选本地模型 → 自动补 sentinel 保持旧行为；
    /// 已显式选了本地模型的用户 → 保留本地选择，不被迁移抢走。
    #[tokio::test]
    async fn cloud_sentinel_migration_respects_explicit_local_choice() {
        let pool = fresh_test_pool().await;

        // 场景 A：老用户，externalEnabled=true 且 summaryMain 缺失
        put_raw(
            &pool,
            r#"{"screenshotPath":"/sp","ai":{"externalEnabled":true,"endpoint":"https://api.x/v1","model":"m","modelsPath":"/mp"}}"#,
        )
        .await;
        let s = load(&pool).await.unwrap();
        assert_eq!(s.ai.summary_main, crate::ai::config::SUMMARY_CLOUD_SENTINEL);
        assert!(
            s.ai.summary_use_cloud(),
            "迁移后段总结应实际路由到云端，与旧版行为一致"
        );
        // 迁移结果必须持久化，否则每次启动都重复迁移
        assert!(raw_json(&pool)
            .await
            .contains(crate::ai::config::SUMMARY_CLOUD_SENTINEL));

        // 场景 B：用户已显式选了本地 summary 模型
        put_raw(
            &pool,
            r#"{"screenshotPath":"/sp","ai":{"externalEnabled":true,"summaryMain":"local.gguf","modelsPath":"/mp"}}"#,
        )
        .await;
        let s2 = load(&pool).await.unwrap();
        assert_eq!(s2.ai.summary_main, "local.gguf", "本地选择不应被迁移覆盖");
        assert!(!s2.ai.summary_use_cloud());
    }

    /// 日报管线换代后，旧管线时代的 system prompt 覆盖与新输入格式错配
    /// （实测输出混乱 + 旧示例文本泄漏）。load 要按特征串清掉旧覆盖，
    /// 但用户后来自己写的干净覆盖必须原样保留——误清等于删用户数据。
    #[tokio::test]
    async fn stale_prompt_override_cleared_but_clean_override_kept() {
        let pool = fresh_test_pool().await;
        let stale = "请基于以下截图的逐张描述，输出当日总结";
        let doc = serde_json::json!({
            "screenshotPath": "/sp",
            "ai": {
                "modelsPath": "/mp",
                "promptOverrides": {
                    "systemZh": stale,
                    "systemEn": "my hand-written EN prompt"
                }
            }
        });
        put_raw(&pool, &doc.to_string()).await;

        let s = load(&pool).await.unwrap();
        assert_eq!(
            s.ai.prompt_overrides.system_zh, "",
            "含旧管线特征串的覆盖应被清空回落内置默认"
        );
        assert_eq!(
            s.ai.prompt_overrides.system_en, "my hand-written EN prompt",
            "不含特征串的用户覆盖必须保留"
        );
        // 清理已持久化：否则只是内存里干净，DB 里的脏覆盖下次启动又回来
        assert!(!raw_json(&pool).await.contains("逐张描述"));
    }

    /// settings_store 单行是 migrations 保证的不变量；行丢了属于 DB 损坏级别，
    /// 必须显式报错。静默回默认会掩盖损坏，且后续 save 的 UPDATE 影响 0 行、
    /// 用户改的所有设置全部静默丢失。
    #[tokio::test]
    async fn load_errors_when_settings_row_missing() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|conn| {
                conn.execute("DELETE FROM settings_store", []).db()?;
                Ok(())
            })
            .await
            .unwrap();
        assert!(load(&pool).await.is_err());
    }

    /// 前端传来的数字不做钳制就会出「0 秒截一张图打满 CPU」「保留 0 天等于
    /// 即时删数据」这类事故。钳制范围以字段文档为准：interval 1..=600、
    /// retention 1..=365、idle 上限 3600 且 0 是合法的「关闭挂机检测」。
    #[test]
    fn apply_patch_clamps_numeric_fields_to_documented_ranges() {
        let d = Settings::default();

        let mut low = empty_patch();
        low.capture_interval_seconds = Some(0);
        low.retention_days = Some(0);
        low.idle_threshold_seconds = Some(0);
        let s = apply_patch(d.clone(), low);
        assert_eq!(s.capture_interval_seconds, 1);
        assert_eq!(s.retention_days, 1);
        assert_eq!(
            s.idle_threshold_seconds, 0,
            "0 = 关闭挂机检测，不能被钳到 1"
        );

        let mut high = empty_patch();
        high.capture_interval_seconds = Some(1_000_000);
        high.retention_days = Some(100_000);
        high.idle_threshold_seconds = Some(1_000_000);
        let s = apply_patch(d, high);
        assert_eq!(s.capture_interval_seconds, 600);
        assert_eq!(s.retention_days, 365);
        assert_eq!(s.idle_threshold_seconds, 3600);
    }

    /// patch 的语义是「None = 不动」：前端每次只传改动子集，任何字段被
    /// 误重置都表现为「我只改了 A，B 怎么也变了」。双层 Option 的
    /// last_update_check_at 另测 Some(None) = 显式清空。
    #[test]
    fn apply_patch_none_keeps_current_and_some_none_clears_option() {
        let current = Settings {
            capture_interval_seconds: 45,
            google_client_id: "abc".into(),
            last_update_check_at: Some("2026-01-01T00:00:00+00:00".into()),
            ..Settings::default()
        };

        // 全 None patch：整组设置必须原样不动
        let unchanged = apply_patch(current.clone(), empty_patch());
        assert_eq!(
            serde_json::to_value(&current).unwrap(),
            serde_json::to_value(&unchanged).unwrap()
        );

        // 只改两项：目标字段生效，邻居字段不受影响
        let mut p = empty_patch();
        p.capture_enabled = Some(false);
        p.work_ranges = Some(vec![TimeRange {
            start: "08:00".into(),
            end: "18:00".into(),
        }]);
        p.last_update_check_at = Some(None);
        let s = apply_patch(current, p);
        assert!(!s.capture_enabled);
        assert_eq!(s.work_ranges.len(), 1);
        assert_eq!(s.work_ranges[0].start, "08:00");
        assert_eq!(s.last_update_check_at, None, "Some(None) 应显式清空时间戳");
        assert_eq!(s.google_client_id, "abc");
        assert_eq!(s.capture_interval_seconds, 45);
    }

    /// 字符串类输入的净化：interval 收敛到合法枚举集合（打错字回 weekly 而不是
    /// 存进去让更新检查逻辑瘫痪）、关键词 trim/去空/去重（重复关键词让隐私列表
    /// 越滚越长）、OAuth id 首尾空格是用户复制粘贴最常见的坑、空路径回默认。
    /// 多时间点:净化(逐项校验/去重保首见/保持添加顺序/上限 6)、
    /// patch 写 times 即清旧单时刻字段、迁移读取(列表空回落旧字段)。
    #[test]
    fn memory_ocr_daily_times_semantics() {
        let v = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();

        // 净化:非法项剔除、重复保首见、顺序保持添加序、trim
        assert_eq!(
            sanitize_hhmm_list(v(&["23:00", "凌晨", " 12:00 ", "23:00", "25:00", "09:15"])),
            v(&["23:00", "12:00", "09:15"])
        );
        // 上限 6:超出裁尾
        assert_eq!(
            sanitize_hhmm_list(v(&[
                "01:00", "02:00", "03:00", "04:00", "05:00", "06:00", "07:00"
            ]))
            .len(),
            6
        );

        // patch 写 times → 旧单时刻字段被清空(单一真源)
        let current = Settings {
            memory_ocr_daily_at: Some("03:00".into()),
            ..Settings::default()
        };
        let mut p = empty_patch();
        p.memory_ocr_daily_times = Some(v(&["12:00", "23:00"]));
        let s2 = apply_patch(current.clone(), p);
        assert_eq!(s2.memory_ocr_daily_at, None, "写 times 必须清旧字段");
        assert_eq!(s2.memory_ocr_daily_times, v(&["12:00", "23:00"]));

        // patch 不带 times:两个字段都维持现值
        let s3 = apply_patch(current, empty_patch());
        assert_eq!(s3.memory_ocr_daily_at.as_deref(), Some("03:00"));
        assert!(s3.memory_ocr_daily_times.is_empty());

        // 迁移读取:列表空回落旧字段;列表非空优先列表
        assert_eq!(s3.effective_ocr_daily_times(), v(&["03:00"]));
        assert_eq!(s2.effective_ocr_daily_times(), v(&["12:00", "23:00"]));
        assert!(Settings::default().effective_ocr_daily_times().is_empty());
    }

    /// 定时补识别时刻的 patch 语义:缺字段保持现值、null 清除、
    /// 合法 HH:MM 保存(trim)、非法归 None(不让坏值瘫痪调度)。
    #[test]
    fn apply_patch_memory_ocr_daily_at_semantics() {
        let current = Settings {
            memory_ocr_daily_at: Some("03:00".into()),
            ..Settings::default()
        };

        // 缺字段:保持现值
        let s = apply_patch(current.clone(), empty_patch());
        assert_eq!(s.memory_ocr_daily_at.as_deref(), Some("03:00"));

        // null:显式清除(关闭定时)
        let mut p = empty_patch();
        p.memory_ocr_daily_at = Some(None);
        assert_eq!(apply_patch(current.clone(), p).memory_ocr_daily_at, None);

        // 合法值:trim 后保存
        let mut p = empty_patch();
        p.memory_ocr_daily_at = Some(Some(" 22:15 ".into()));
        assert_eq!(
            apply_patch(current.clone(), p)
                .memory_ocr_daily_at
                .as_deref(),
            Some("22:15")
        );

        // 非法值:归 None 而不是存进去
        for bad in ["25:00", "3点", "", "12:60"] {
            let mut p = empty_patch();
            p.memory_ocr_daily_at = Some(Some(bad.into()));
            assert_eq!(
                apply_patch(current.clone(), p).memory_ocr_daily_at,
                None,
                "{bad:?} 应被拒"
            );
        }

        // JSON 层:缺字段 → 外层 None;null → Some(None)(前端契约)
        let p: SettingsPatch = serde_json::from_str("{}").unwrap();
        assert!(p.memory_ocr_daily_at.is_none());
        let p: SettingsPatch = serde_json::from_str(r#"{"memoryOcrDailyAt": null}"#).unwrap();
        assert_eq!(p.memory_ocr_daily_at, Some(None));
        let p: SettingsPatch = serde_json::from_str(r#"{"memoryOcrDailyAt": "07:45"}"#).unwrap();
        assert_eq!(p.memory_ocr_daily_at, Some(Some("07:45".into())));
    }

    #[test]
    fn apply_patch_sanitizes_string_inputs() {
        let d = Settings::default();

        let mut p = empty_patch();
        p.auto_update_interval = Some("hourly".into());
        p.privacy_url_keywords = Some(vec![
            " /Login ".into(),
            "".into(),
            "   ".into(),
            "/pay".into(),
            "/Login".into(),
        ]);
        p.google_client_id = Some("  id-x  ".into());
        p.screenshot_path = Some("   ".into());
        let s = apply_patch(d.clone(), p);
        assert_eq!(
            s.auto_update_interval, "weekly",
            "非法 interval 应回退 weekly"
        );
        assert_eq!(s.privacy_url_keywords, vec!["/Login", "/pay"]);
        assert_eq!(s.google_client_id, "id-x");
        assert!(
            s.screenshot_path.ends_with("screenshots"),
            "全空白路径应回默认目录而不是存下空路径"
        );

        // 合法 interval 原样保留，不能被净化误伤
        let mut p2 = empty_patch();
        p2.auto_update_interval = Some("onstartup".into());
        assert_eq!(apply_patch(d, p2).auto_update_interval, "onstartup");
    }

    /// JSON 字段名就是持久化格式 + 前端契约：谁要是重命名了 rust 字段而没配
    /// serde rename，老 JSON 会整体「缺字段」静默回默认，前端也拿不到值。
    /// 用序列化产物点名关键字段，把这种改动变成显式测试失败。
    #[test]
    fn timerange_roundtrip_and_wire_field_names_stable() {
        // TimeRange 纯序列化往返（含跨午夜）
        let tr = TimeRange {
            start: "23:30".into(),
            end: "01:00".into(),
        };
        let back: TimeRange = serde_json::from_str(&serde_json::to_string(&tr).unwrap()).unwrap();
        assert_eq!(back.start, "23:30");
        assert_eq!(back.end, "01:00");
        let vt = serde_json::to_value(&tr).unwrap();
        assert!(vt.get("start").is_some() && vt.get("end").is_some());

        // Settings 顶层 camelCase 字段名抽查
        let v = serde_json::to_value(Settings::default()).unwrap();
        for key in [
            "captureEnabled",
            "workRanges",
            "privacyUrlKeywords",
            "autoUpdateInterval",
            "ai",
        ] {
            assert!(
                v.get(key).is_some(),
                "settings JSON 缺 {key} 字段——字段改名会让老数据静默失效"
            );
        }
    }
}
