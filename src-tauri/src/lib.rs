mod account;
mod ai;
mod bootstrap;
mod capture;
mod commands;
mod db;
mod device;
mod error;
mod icons;
mod permissions;
mod platform;
mod repo;
mod storage;
mod sync;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use ai::server::EngineSupervisor;
use capture::CaptureService;
use db::SqliteResultExt;
use repo::{activities, devices, settings};
use storage::{db_path, DbPool};
use sync::engine::SyncEngine;
use tauri::Manager;

const CLEANUP_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// 关闭按钮的行为标志（true = 隐藏到托盘 / false = 直接退出）。
/// 由 settings.minimize_to_tray 同步：启动时从 DB 载入一次，commands::settings
/// update 后写一次。close handler 在 window 事件回调里读，避免每次点 X 都查 DB。
pub static MINIMIZE_TO_TRAY: AtomicBool = AtomicBool::new(true);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 默认开 hindsight=info（用户可用 RUST_LOG 覆盖）。隐私命中 / 同步错误 / 启动信息
    // 都是 info 级别，普通运行也能在终端看到。
    let _ = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("hindsight=info"),
    )
    .try_init();

    // AI 引擎子进程守护者：lazy spawn，app 退出时 stop
    let engine_supervisor = Arc::new(EngineSupervisor::new());
    let engine_for_exit = engine_supervisor.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(move |app| {
            let handle = app.handle().clone();

            if let Some(main_window) = handle.get_webview_window("main") {
                platform::apply_window_tweaks(&main_window);

                // 点窗口右上角 X 的行为：默认隐藏到托盘（不退出进程，避免采集中断），
                // 用户可在「设置 → 常规 → 关闭后最小化到托盘」关掉这个行为，关掉后 X
                // 就是真正退出。真正的"退出"在托盘右键菜单里。
                let win_for_close = main_window.clone();
                main_window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        if MINIMIZE_TO_TRAY.load(std::sync::atomic::Ordering::Relaxed) {
                            api.prevent_close();
                            let _ = win_for_close.hide();
                        }
                    }
                });
            }

            // 系统托盘：图标用主窗口同款。左键单击 toggle 显示 / 隐藏；
            // 右键 / 菜单提供"显示主窗口" + "退出"。退出走 app.exit(0)。
            {
                use tauri::menu::{MenuBuilder, MenuItemBuilder};
                use tauri::tray::{
                    MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
                };

                let show_item = MenuItemBuilder::with_id("show", "显示主窗口").build(app)?;
                let quit_item = MenuItemBuilder::with_id("quit", "退出").build(app)?;
                let menu = MenuBuilder::new(app)
                    .item(&show_item)
                    .separator()
                    .item(&quit_item)
                    .build()?;

                let _tray = TrayIconBuilder::with_id("hindsight-tray")
                    .icon(app.default_window_icon().cloned().unwrap())
                    .tooltip("Hindsight")
                    .menu(&menu)
                    // 左键不弹菜单（留给 toggle 显隐）；菜单只在右键 / macOS 上 showMenu 时弹
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "show" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
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
                    })
                    .build(app)?;
            }

            tauri::async_runtime::block_on(async move {
                // 0) 平台权限：macOS 上的 Screen Recording。没拿到 xcap 拿不到其它进程
                //    的窗口，焦点采集功能整个废掉，但不会报错（CG API 静默降级），所以
                //    必须先在启动早期触发系统弹框请求权限。
                //    阻塞调用：用户没决定前 macOS 不返回。授权后 TCC 在下次进程启动时缓存。
                let perm = permissions::ensure_screen_recording();
                log::info!("Screen Recording permission: {:?}", perm);

                // 1) 启动级身份：必须在打开 DB 之前确定 device_id（device.json 在 DB 之外）
                let dev_meta = device::ensure_loaded()
                    .expect("加载设备身份")
                    .clone();
                log::info!("device_id: {}", dev_meta.device_id);

                // 1b) 多账号：处理老安装升级 / sign-in 后未重启的延迟迁移；之后 db_path()
                //     才知道开哪份 DB（hindsight.sqlite vs hindsight.<uid>.sqlite）。
                let data_root = bootstrap::data_root();
                if let Err(e) = account::migrate_legacy_db(&data_root).await {
                    log::warn!("legacy DB 迁移失败（继续使用现有 DB）: {e}");
                }
                if let Some(uid) = account::active_uid() {
                    log::info!("active_uid: {uid}");
                }

                // 2) 数据根 + DB
                let path = db_path().expect("解析数据库路径");
                log::info!("数据库路径: {}", path.display());

                let pool = DbPool::open(&path).await.expect("打开数据库");
                storage::migrations::run(&pool)
                    .await
                    .expect("执行数据库迁移");

                // 3) 把当前机器写进 devices 表（is_self=1）+ outbox
                devices::upsert_self(
                    &pool,
                    dev_meta.device_id.clone(),
                    dev_meta.display_name.clone(),
                    dev_meta.color.clone(),
                    dev_meta.icon.clone(),
                    dev_meta.os.clone(),
                )
                .await
                .expect("注册当前设备");

                // 3b) v8 之前硬编码的 'local' device_id 改成真实 self id（幂等，对老数据一次性生效）
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

                let cfg = settings::load(&pool).await.expect("读取设置");
                MINIMIZE_TO_TRAY
                    .store(cfg.minimize_to_tray, std::sync::atomic::Ordering::Relaxed);

                let svc = Arc::new(CaptureService::new(
                    pool.clone(),
                    cfg.capture_interval_seconds,
                ));
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

                spawn_cleanup_task(pool.clone());

                // 一次性 backfill：把老用户已经在文件 cache 里、但 app_icons 表里没行的图标
                // 灌进 DB + 入 outbox。否则这些"开启同步前提取过的图标"对端永远拉不到。
                // 后台跑，不挡启动；已存在的会跳过，重复启动开销很低。
                let pool_for_backfill = pool.clone();
                tokio::spawn(async move {
                    match repo::app_icons::backfill_db_from_cache_or_extract(&pool_for_backfill).await {
                        Ok(n) if n > 0 => log::info!("icon backfill: 新增 {n} 行 app_icons"),
                        Ok(_) => {}
                        Err(e) => log::warn!("icon backfill 失败: {e}"),
                    }
                });

                // builtin 分类 backfill：升级到带新规则的版本时，给老 DB 里 category_id IS NULL
                // 的 group 自动归类（命中 chrome / code / wechat 等内置规则）。
                // 用户手动归过类的不动；本次没命中的下次升级 JSON 加规则后还会再尝试。
                let pool_for_cat_backfill = pool.clone();
                tokio::spawn(async move {
                    match repo::builtin_categories::backfill_builtin_categories(&pool_for_cat_backfill).await {
                        Ok(n) if n > 0 => log::info!("builtin category backfill: 自动归类 {n} 个 app_group"),
                        Ok(_) => {}
                        Err(e) => log::warn!("builtin category backfill 失败: {e}"),
                    }
                });

                // 4) 同步引擎：登录态由 engine 内部检查，未登录时所有循环都是 no-op；
                //    所以可以无条件 start，登录后自动开始推
                let sync_engine = Arc::new(SyncEngine::new(pool.clone()));
                sync_engine.start().await;

                handle.manage(pool);
                handle.manage(svc);
                handle.manage(sync_engine);
                handle.manage(engine_supervisor);
                // AI 总结取消信号——单例，前端调 cancel_day_summary 设 true，
                // summary.rs 每段循环检查；不能中断已在路上的 LLM 单段请求。
                handle.manage(commands::ai::SummaryCancel::default());
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::capture::start_capture,
            commands::capture::stop_capture,
            commands::capture::get_capture_status,
            commands::data::get_day_hours,
            commands::data::get_day_apps,
            commands::data::get_week_days,
            commands::data::get_week_apps,
            commands::data::get_month_days,
            commands::data::get_month_apps,
            commands::categories::list_categories,
            commands::categories::create_category,
            commands::categories::update_category,
            commands::categories::delete_category,
            commands::categories::reorder_categories,
            commands::categories::assign_app_to_category,
            commands::categories::unassign_app,
            commands::categories::list_unclassified_apps,
            commands::app_groups::list_app_groups,
            commands::app_groups::create_app_group,
            commands::app_groups::delete_app_group,
            commands::app_groups::merge_app_group,
            commands::app_groups::unmerge_app_group,
            commands::app_groups::rename_app_group,
            commands::app_groups::assign_app_group_category,
            commands::icons::get_app_icon,
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::storage::get_storage_info,
            commands::storage::purge_activities,
            commands::storage::purge_screenshots,
            commands::storage::open_screenshots_dir,
            commands::storage::get_data_root,
            commands::storage::set_data_root,
            commands::devices::list_devices,
            commands::devices::update_self_device,
            commands::auth::auth_status,
            commands::auth::sign_in_with_google,
            commands::auth::sign_out,
            commands::auth::restart_app,
            commands::sync::sync_status,
            commands::sync::sync_now,
            commands::ai::test_ai_endpoint,
            commands::ai::get_engine_status,
            commands::ai::download_binary,
            commands::ai::delete_binary,
            commands::ai::open_engine_dir,
            commands::ai::start_engine,
            commands::ai::stop_engine,
            commands::ai::list_local_models,
            commands::ai::delete_model,
            commands::ai::list_recommended_models,
            commands::ai::download_model,
            commands::ai::set_active_model,
            commands::ai::generate_day_summary,
            commands::ai::retry_summary_segment,
            commands::ai::cancel_day_summary,
            commands::ai::get_day_summary,
        ])
        .build(tauri::generate_context!())
        .expect("启动 Tauri 应用失败")
        .run(move |app, event| {
            // app 真正退出前等 llama-server 子进程收尸——避免遗留孤儿进程
            // 一直 hold 着模型在内存里。block_on 是同步等，因为 Exit 已经是 final。
            if matches!(&event, tauri::RunEvent::Exit) {
                let s = engine_for_exit.clone();
                tauri::async_runtime::block_on(async move {
                    let _ = s.stop().await;
                });
            }
            platform::handle_run_event(app, event);
        });
}

fn spawn_cleanup_task(pool: DbPool) {
    tokio::spawn(async move {
        loop {
            match settings::load(&pool).await {
                Ok(cfg) => {
                    match activities::delete_screenshots_older_than(&pool, cfg.retention_days).await {
                        Ok(n) if n > 0 => log::info!("清理了 {n} 张过期截图"),
                        Ok(_) => {}
                        Err(e) => log::warn!("清理过期截图失败: {e}"),
                    }
                }
                Err(e) => log::warn!("清理任务读取设置失败: {e}"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(CLEANUP_INTERVAL_SECS)).await;
        }
    });
}
