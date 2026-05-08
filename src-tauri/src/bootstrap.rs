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

/// 启动级配置文件位置：%APPDATA%/Hindsight/bootstrap.json （Windows）
/// 它存放的是"DB 应该开在哪里"这种 chicken-and-egg 的信息——在打开 DB 之前就要读到。
fn config_file() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("Hindsight").join("bootstrap.json"))
}

/// 系统默认数据目录：%APPDATA%/Hindsight 等
fn default_data_root() -> PathBuf {
    dirs::data_dir()
        .map(|p| p.join("Hindsight"))
        .unwrap_or_else(|| PathBuf::from("hindsight-data"))
}

/// 当前生效的数据根：优先 bootstrap 里的自定义值，没有则系统默认。
pub fn data_root() -> PathBuf {
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
    let _ = pool
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
    Ok(pool)
}

/// 第 3 步：构造 [`CaptureService`] 并按当前 settings 配置一次。
/// 不在这里 start —— 调用方根据 settings.capture_enabled 决定是否启动。
pub async fn init_capture_service(pool: DbPool, cfg: &Settings) -> Arc<CaptureService> {
    let svc = Arc::new(CaptureService::new(pool, cfg.capture_interval_seconds));
    svc.set_work_hours(cfg.work_hours_enabled, cfg.work_ranges.clone())
        .await;
    svc.set_screenshot_config(
        cfg.capture_enabled,
        cfg.screenshot_path.clone(),
        1280,
        720,
        80,
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
pub async fn init_sync_engine(pool: DbPool) -> Arc<SyncEngine> {
    let sync_engine = Arc::new(SyncEngine::new(pool));
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
                }
            }
        });
    }

    install_tray_icon(app)
}

/// 系统托盘：图标用主窗口同款。左键单击 toggle 显示 / 隐藏；
/// 右键 / 菜单提供"显示主窗口" + "退出"。退出走 app.exit(0)。
fn install_tray_icon(app: &mut App) -> tauri::Result<()> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};
    use tauri::tray::TrayIconBuilder;

    let show_item = MenuItemBuilder::with_id("show", "显示主窗口").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "退出").build(app)?;
    let menu = MenuBuilder::new(app)
        .item(&show_item)
        .separator()
        .item(&quit_item)
        .build()?;

    let _tray = TrayIconBuilder::with_id("hindsight-tray")
        // 启动期失败需快速失败：Tauri 总会带默认窗口图标，缺失意味着打包资源错误
        .icon(
            app.default_window_icon()
                .cloned()
                .expect("default_window_icon 必存在（Tauri 资源缺失）"),
        )
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
                }
                _ => {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        }
    }
}

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
        match crate::repo::builtin_categories::backfill_builtin_categories(&pool).await {
            Ok(n) if n > 0 => {
                log::info!("builtin category backfill: 自动归类 {n} 个 app_group")
            }
            Ok(_) => {}
            Err(e) => log::warn!("builtin category backfill 失败: {e}"),
        }
    });
}
