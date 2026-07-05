//! 启动期编排 —— 把 `lib.rs::run` 的 setup 闭包业务拆成有名字的步骤。
//!
//! 设计原则：每个 init_* 函数是「失败就启动失败」级别的步骤，返回 `io::Result` 或
//! 各 repo 层的 `Result`；调用方（lib.rs）选择 `.expect()` 让失败变成快速 panic。
//! 后台任务（icon / 内置分类 backfill / cleanup loop）依然在 lib.rs 里 spawn，
//! 因为它们不阻塞启动，spawn 后启动闭包就走完了。
//!
//! 文件还包含原本的 `data_root` / `set_data_root` —— 给 `db_path()` 提供"DB 该开在哪"
//! 这种 chicken-and-egg 信息。

use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use tauri::{App, AppHandle, Manager};

use crate::capture::CaptureService;
use crate::device::{self, DeviceMeta};
use crate::repo::devices;
use crate::repo::settings::Settings;
use crate::storage::SqliteResultExt;
use crate::storage::{db_path, DbPool};
use crate::sync::engine::SyncEngine;
use crate::{account, platform, storage};

#[derive(Default, Serialize, Deserialize)]
struct BootstrapFile {
    #[serde(default)]
    data_path: Option<String>,
}

/// 启动级配置：`dirs::config_dir() / Hindsight / bootstrap.json`
///   Windows: `%APPDATA%\Hindsight\bootstrap.json`
///   macOS:   `~/Library/Application Support/Hindsight/bootstrap.json`
///   Linux:   `~/.config/Hindsight/bootstrap.json`
/// 存放的是"DB 应该开在哪里"这种 chicken-and-egg 的信息——在打开 DB 之前就要读到。
fn config_file() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("Hindsight").join("bootstrap.json"))
}

/// 系统默认数据目录：%APPDATA%/Hindsight 等
fn default_data_root() -> PathBuf {
    dirs::data_dir()
        .map(|p| p.join("Hindsight"))
        .unwrap_or_else(|| PathBuf::from("hindsight-data"))
}

/// 当前生效的数据根：优先 `HINDSIGHT_DATA_DIR` 环境变量（仅本地多实例测试用）
/// → 然后 bootstrap.json 里的自定义值 → 最后系统默认。
///
/// 环境变量是为「在同一台机器跑两个独立 Hindsight 实例模拟两台设备」的本地测试
/// 场景，详见 [`docs/internal/local-multi-device-test.md`]。生产路径不会设置它。
pub fn data_root() -> PathBuf {
    if let Ok(env_path) = std::env::var("HINDSIGHT_DATA_DIR") {
        let trimmed = env_path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if let Some(path) = read_custom_data_path() {
        if !path.as_os_str().is_empty() {
            return path;
        }
    }
    default_data_root()
}

fn read_custom_data_path() -> Option<PathBuf> {
    let cfg = config_file()?;
    let s = fs::read_to_string(&cfg).ok()?;
    let b: BootstrapFile = serde_json::from_str(&s).ok()?;
    b.data_path
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
}

/// 写入新的数据根（不会自动迁移已有数据，下次启动生效）。
pub fn set_data_root(path: &str) -> io::Result<()> {
    let cfg = config_file()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "找不到系统配置目录"))?;
    if let Some(parent) = cfg.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = BootstrapFile {
        data_path: Some(path.to_string()),
    };
    let s = serde_json::to_string_pretty(&body).map_err(|e| io::Error::other(e.to_string()))?;
    fs::write(&cfg, s)
}

// ─────────────────────────────────────────────────────────────────
//  启动 init 步骤
// ─────────────────────────────────────────────────────────────────

/// 第 1 步：装载 device.json + 处理多账号 legacy DB 迁移 + active_uid 日志。
///
/// 必须在打开 DB 之前调（device.json 决定 device_id；active_uid 决定 db_path 选哪份）。
pub async fn init_self_identity() -> io::Result<DeviceMeta> {
    let dev_meta = device::ensure_loaded()?.clone();
    log::info!("device_id: {}", dev_meta.device_id);

    // 多账号：处理老安装升级 / sign-in 后未重启的延迟迁移；之后 db_path()
    // 才知道开哪份 DB（hindsight.sqlite vs hindsight.<uid>.sqlite）。
    let root = data_root();
    if let Err(e) = account::migrate_legacy_db(&root).await {
        log::warn!("legacy DB 迁移失败（继续使用现有 DB）: {e}");
    }
    if let Some(uid) = account::active_uid() {
        log::info!("active_uid: {uid}");
    }
    Ok(dev_meta)
}

/// 第 2 步：打开 DB + 跑 migrations + 写当前 device 行 + 修 v8 之前的 'local' device_id。
///
/// 返回开好的连接池给后续 init 复用。
pub async fn init_database(dev_meta: &DeviceMeta) -> crate::error::Result<DbPool> {
    let path = db_path()?;
    log::info!("数据库路径: {}", path.display());

    let pool = DbPool::open(&path).await?;
    storage::migrations::run(&pool).await?;

    devices::upsert_self(
        &pool,
        dev_meta.device_id.clone(),
        dev_meta.display_name.clone(),
        dev_meta.color.clone(),
        dev_meta.icon.clone(),
        dev_meta.os.clone(),
    )
    .await?;

    // v8 之前硬编码的 'local' device_id 改成真实 self id（幂等，对老数据一次性生效）
    let self_id_for_fix = dev_meta.device_id.clone();
    let migration = pool
        .0
        .call(move |conn| {
            let n = conn
                .execute(
                    "UPDATE activities SET device_id = ?1 WHERE device_id = 'local'",
                    rusqlite::params![self_id_for_fix],
                )
                .db()?;
            if n > 0 {
                log::info!("把 {} 条 v8 之前的历史活动 device_id 改为 self", n);
            }
            Ok(())
        })
        .await;
    if let Err(e) = migration {
        // 这次 migration 是 best-effort——失败不阻塞启动，但要留痕。失败后下次启动
        // 还会再跑（条件不变 + 没成功改过的行还在），所以是幂等的，长期收敛。
        log::warn!("v8 device_id migration 失败（下次启动会重试）: {e}");
    }
    Ok(pool)
}

/// 第 3 步：构造 [`CaptureService`] 并按当前 settings 配置一次。
/// 不在这里 start —— 调用方根据 settings.capture_enabled 决定是否启动。
pub async fn init_capture_service(
    pool: DbPool,
    memory: Option<crate::memory::MemoryDb>,
    cfg: &Settings,
) -> Arc<CaptureService> {
    let svc = Arc::new(CaptureService::new(
        pool,
        memory,
        cfg.capture_interval_seconds,
    ));
    svc.set_work_hours(cfg.work_hours_enabled, cfg.work_ranges.clone())
        .await;
    svc.set_screenshot_config(
        // screenshot_enabled 独立开关——capture_enabled=true 控整 service 跑不跑，
        // screenshot_enabled=false 时窗口活动照常落库，仅跳过 take_screenshot
        cfg.screenshot_enabled,
        cfg.screenshot_path.clone(),
        // 存档规格(screen-memory.md L2 定案):上限 2880、q85、Lanczos3。
        // 1× 屏(QHD 2560)从此原生存储,14-16px 正文以原生像素进 OCR;
        // 旧 1280/q80 对 1× 屏的文字是不可逆毁灭,历史无法回填。
        2880,
        2880,
        85,
    )
    .await;
    svc.set_privacy_keywords(
        cfg.privacy_url_keywords.clone(),
        cfg.privacy_app_keywords.clone(),
    )
    .await;
    svc.set_idle_threshold(cfg.idle_threshold_seconds).await;
    if cfg.capture_enabled {
        svc.start().await;
    }
    svc
}

/// 第 4 步：启动同步引擎。登录态由 engine 内部检查，未登录所有循环都是 no-op；
/// 所以可以无条件 start，登录后自动开始推。
/// `mem` = 记忆库句柄(聊天历史/屏幕记忆的可选上云用;打开失败传 None)。
pub async fn init_sync_engine(
    pool: DbPool,
    mem: Option<crate::memory::MemoryDb>,
) -> Arc<SyncEngine> {
    let sync_engine = Arc::new(SyncEngine::new(pool, mem));
    sync_engine.start().await;
    sync_engine
}

/// 安装系统托盘 + 主窗口的 close handler。
///
/// 抽出来是为了让 `lib.rs::run` 的 setup 闭包瘦身：托盘的 builder 链 + close
/// 行为切换是稳定的"一次性安装"逻辑，跟启动数据流编排关系不大。
///
/// 关闭按钮行为：默认 X = 隐藏到托盘，用户在「设置 → 常规」可改成真正退出
/// （[`crate::MINIMIZE_TO_TRAY`] 控制，settings 改完 store 同步给这里读）。
/// 真正的"退出"在托盘右键菜单。
pub fn install_tray_and_window(app: &mut App) -> tauri::Result<()> {
    let handle = app.handle().clone();

    if let Some(main_window) = handle.get_webview_window("main") {
        platform::apply_window_tweaks(&main_window);

        let win_for_close = main_window.clone();
        main_window.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if crate::MINIMIZE_TO_TRAY.load(std::sync::atomic::Ordering::Relaxed) {
                    api.prevent_close();
                    let _ = win_for_close.hide();
                    // macOS：连 Dock 图标一起收，"关窗 = 收进右上角托盘"才成立
                    platform::set_dock_icon_visible(win_for_close.app_handle(), false);
                }
            }
        });
    }

    install_tray_icon(app)
}

/// 托盘菜单项句柄——存进 app state，让前端切 UI 语言后能 `set_text` 更新文案。
pub struct TrayMenu {
    show: tauri::menu::MenuItem<tauri::Wry>,
    quit: tauri::menu::MenuItem<tauri::Wry>,
}

/// 托盘文案按语言取一套。启动时 lang 来自 [`crate::ai::config::detect_default_lang`]，
/// 运行时由前端经 [`set_tray_labels`] 传 i18n 译文覆盖。
fn tray_labels(lang: &str) -> (&'static str, &'static str) {
    match lang {
        "ja" => ("メインウィンドウを表示", "終了"),
        "en" => ("Show Window", "Quit"),
        _ => ("显示主窗口", "退出"),
    }
}

/// 前端切 UI 语言后调用，把托盘菜单文案同步成对应语言。
#[tauri::command]
pub fn set_tray_labels(
    state: tauri::State<'_, TrayMenu>,
    show: String,
    quit: String,
) -> Result<(), String> {
    state.show.set_text(show).map_err(|e| e.to_string())?;
    state.quit.set_text(quit).map_err(|e| e.to_string())?;
    Ok(())
}

/// 系统托盘：图标用主窗口同款。左键单击 toggle 显示 / 隐藏；
/// 右键 / 菜单提供"显示主窗口" + "退出"。退出走 app.exit(0)。
fn install_tray_icon(app: &mut App) -> tauri::Result<()> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};
    use tauri::tray::TrayIconBuilder;

    // 启动时按系统 locale 取初始文案；webview 加载后前端会 set_tray_labels 同步到实际 UI 语言
    let (show_label, quit_label) = tray_labels(crate::ai::config::detect_default_lang());
    let show_item = MenuItemBuilder::with_id("show", show_label).build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", quit_label).build(app)?;
    let menu = MenuBuilder::new(app)
        .item(&show_item)
        .separator()
        .item(&quit_item)
        .build()?;

    // 存句柄给 set_tray_labels 用（MenuItem 是 Arc 句柄，clone 廉价）
    app.manage(TrayMenu {
        show: show_item.clone(),
        quit: quit_item.clone(),
    });

    // macOS 菜单栏图标必须是 template image（纯黑 + alpha 的单色剪影，
    // icons/tray-icon-macos.png 由 app 图标的眼睛图形按亮度抠出）：
    // icon_as_template 让系统按菜单栏明暗自动渲染黑 / 白 + 选中态反色。
    // 直接塞彩色 app 图标的话在菜单栏里是一块彩色方糖，不符合 macOS HIG。
    #[cfg(target_os = "macos")]
    let (tray_icon, as_template) = (
        tauri::image::Image::from_bytes(include_bytes!("../icons/tray-icon-macos.png"))?,
        true,
    );
    // 其它平台保持彩色 app 图标（Windows 托盘本来就该彩色）。
    // 启动期失败需快速失败：Tauri 总会带默认窗口图标，缺失意味着打包资源错误。
    #[cfg(not(target_os = "macos"))]
    let (tray_icon, as_template) = (
        app.default_window_icon()
            .cloned()
            .expect("default_window_icon 必存在（Tauri 资源缺失）"),
        false,
    );

    let _tray = TrayIconBuilder::with_id("hindsight-tray")
        .icon(tray_icon)
        // 非 macOS 平台此标志被忽略
        .icon_as_template(as_template)
        .tooltip("Hindsight")
        .menu(&menu)
        // 左键不弹菜单（留给 toggle 显隐）；菜单只在右键 / macOS 上 showMenu 时弹
        .show_menu_on_left_click(false)
        .on_menu_event(handle_tray_menu_event)
        .on_tray_icon_event(handle_tray_icon_event)
        .build(app)?;
    Ok(())
}

fn handle_tray_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    match event.id.as_ref() {
        "show" => {
            if let Some(w) = app.get_webview_window("main") {
                // 先恢复 Dock 图标再 show——Accessory 下 set_focus 抢不到前台
                platform::set_dock_icon_visible(app, true);
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
        "quit" => app.exit(0),
        _ => {}
    }
}

fn handle_tray_icon_event(tray: &tauri::tray::TrayIcon, event: tauri::tray::TrayIconEvent) {
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    } = event
    {
        let app = tray.app_handle();
        if let Some(w) = app.get_webview_window("main") {
            match w.is_visible() {
                Ok(true) => {
                    let _ = w.hide();
                    platform::set_dock_icon_visible(app, false);
                }
                _ => {
                    platform::set_dock_icon_visible(app, true);
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        }
    }
}

/// autostart 从 LaunchAgent 迁到 AppleScript 登录项（v0.7.8）的一次性清理。
///
/// 老版本用 LaunchAgent 模式：开自启会写 `~/Library/LaunchAgents/hindsight.plist`
/// （文件名 = 插件默认 app_name = package name）。换 AppleScript 模式后这份 plist
/// 不再被插件管理：留着会双启动（plist 和登录项各拉起一次），而且它正是
/// 「登录项显示开发者名」问题的载体。
///
/// plist 存在 ⟺ 用户在老版本开过自启 → 删掉后立刻用新模式重新注册，保留用户意图。
/// 注册走 osascript（System Events），首次可能弹一次自动化授权框；拒绝的话自启
/// 静默失效，用户在设置里重开即可（is_enabled 会如实显示关闭）。
#[cfg(target_os = "macos")]
pub fn migrate_autostart_launch_agent(app: &AppHandle) {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let plist = home.join("Library/LaunchAgents/hindsight.plist");
    if !plist.exists() {
        return;
    }
    if let Err(e) = fs::remove_file(&plist) {
        log::warn!("autostart 迁移：删除旧 LaunchAgent plist 失败: {e}");
        return;
    }
    use tauri_plugin_autostart::ManagerExt;
    match app.autolaunch().enable() {
        Ok(()) => log::info!("autostart 已迁移：LaunchAgent → 登录项（登录项列表显示应用名）"),
        Err(e) => log::warn!("autostart 迁移：登录项注册失败（需用户在设置里重开自启）: {e}"),
    }
}

/// 非 macOS 平台：no-op（LaunchAgent 是 macOS 独有机制）。
#[cfg(not(target_os = "macos"))]
pub fn migrate_autostart_launch_agent(_app: &AppHandle) {}

/// 后台 backfill 任务：图标 + 内置分类。两个任务都自带"已存在则跳过"，
/// 重复启动开销很低；fire-and-forget 就行，不阻塞启动。
pub fn spawn_backfill_tasks(pool: DbPool) {
    let pool_for_icons = pool.clone();
    tokio::spawn(async move {
        match crate::repo::app_icons::backfill_db_from_cache_or_extract(&pool_for_icons).await {
            Ok(n) if n > 0 => log::info!("icon backfill: 新增 {n} 行 app_icons"),
            Ok(_) => {}
            Err(e) => log::warn!("icon backfill 失败: {e}"),
        }
    });

    tokio::spawn(async move {
        // 必须**顺序**跑：先跨 OS 别名配对，再内置分类回填。pair_existing 会把
        // Win "chrome.exe" 这类默认 solo 组合并到 canonical "Google Chrome"，让
        // 随后的 builtin backfill 看到的已经是 canonical 组（display_name =
        // canonical name 才能命中 [builtin_categories] 字典）。拆成两个并发 task
        // 会竞态：builtin 先跑就按非 canonical 名匹配、全部落空，应用整个会话
        // 保持未分类。两者都幂等，重启重跑零代价。
        match crate::repo::cross_os_aliases::pair_existing(&pool).await {
            Ok(n) if n > 0 => {
                log::info!("cross-OS alias backfill: 合并 {n} 个 app_group_member")
            }
            Ok(_) => {}
            Err(e) => log::warn!("cross-OS alias backfill 失败: {e}"),
        }

        match crate::repo::builtin_categories::backfill_builtin_categories(&pool).await {
            Ok(n) if n > 0 => {
                log::info!("builtin category backfill: 自动归类 {n} 个 app_group")
            }
            Ok(_) => {}
            Err(e) => log::warn!("builtin category backfill 失败: {e}"),
        }
    });
}
