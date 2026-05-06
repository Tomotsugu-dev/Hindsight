mod account;
mod ai;
mod bootstrap;
mod capture;
mod commands;
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
use repo::{activities, settings};
use storage::DbPool;
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

    // 防孤儿子进程：Windows Job Object 把所有 spawn 出来的 child 绑到本进程上，
    // Hindsight 死（panic / Ctrl+C / taskkill）→ OS 内核同步杀光所有 child，
    // 不再依赖 RunEvent::Exit 钩子。Linux / macOS 是 no-op。
    if let Err(e) = ai::job_guard::init_global_job() {
        log::warn!("init Windows Job Object 失败（孤儿防护退化为仅 Exit hook）: {e}");
    }

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
                // 平台权限：macOS 上的 Screen Recording。没拿到 xcap 拿不到其它进程
                // 的窗口，焦点采集功能整个废掉，但不会报错（CG API 静默降级），所以
                // 必须先在启动早期触发系统弹框请求权限。
                // 阻塞调用：用户没决定前 macOS 不返回。授权后 TCC 在下次进程启动时缓存。
                let perm = permissions::ensure_screen_recording();
                log::info!("Screen Recording permission: {:?}", perm);

                // 启动期失败需快速失败：device 身份 / DB / migrations 任一失败应用都跑不起来；
                // expect 让用户立刻看到原因，而不是后续命令一连串报"未初始化"
                let dev_meta = bootstrap::init_self_identity()
                    .await
                    .expect("加载设备身份失败");
                let pool = bootstrap::init_database(&dev_meta)
                    .await
                    .expect("初始化数据库失败");

                let cfg = settings::load(&pool).await.expect("读取设置");
                MINIMIZE_TO_TRAY
                    .store(cfg.minimize_to_tray, std::sync::atomic::Ordering::Relaxed);

                let svc = bootstrap::init_capture_service(pool.clone(), &cfg).await;
                spawn_cleanup_task(pool.clone());
                bootstrap::spawn_backfill_tasks(pool.clone());
                let sync_engine = bootstrap::init_sync_engine(pool.clone()).await;

                handle.manage(pool);
                handle.manage(svc);
                handle.manage(sync_engine);
                // 启动 idle watcher：跑完日报/调试 N 秒无新请求 → 自动 stop 释放显存。
                // watcher 持 Weak<EngineSupervisor>，supervisor drop 后自然退出，无需手动取消。
                let _watcher = engine_supervisor.spawn_idle_watcher();
                handle.manage(engine_supervisor);
                // AI 总结取消信号——单例，前端调 cancel_day_summary 设 true，
                // summary_runner 每段循环检查；不能中断已在路上的 LLM 单段请求。
                handle.manage(commands::ai::SummaryCancel::default());
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // --- capture: 焦点采集 ---
            commands::capture::start_capture,
            commands::capture::stop_capture,
            commands::capture::get_capture_status,
            // --- data: 报表查询 ---
            commands::data::get_day_hours,
            commands::data::get_day_apps,
            commands::data::get_week_days,
            commands::data::get_week_apps,
            commands::data::get_month_days,
            commands::data::get_month_apps,
            // --- categories: 分类管理 ---
            commands::categories::list_categories,
            commands::categories::create_category,
            commands::categories::update_category,
            commands::categories::delete_category,
            commands::categories::reorder_categories,
            commands::categories::assign_app_to_category,
            commands::categories::unassign_app,
            commands::categories::list_unclassified_apps,
            // --- app_groups: 应用分组（多个进程合一组） ---
            commands::app_groups::list_app_groups,
            commands::app_groups::create_app_group,
            commands::app_groups::delete_app_group,
            commands::app_groups::merge_app_group,
            commands::app_groups::unmerge_app_group,
            commands::app_groups::rename_app_group,
            commands::app_groups::assign_app_group_category,
            // --- icons: 应用图标 ---
            commands::icons::get_app_icon,
            // --- settings: 全局设置 ---
            commands::settings::get_settings,
            commands::settings::update_settings,
            // --- storage: 存储 / 数据目录 ---
            commands::storage::get_storage_info,
            commands::storage::purge_activities,
            commands::storage::purge_screenshots,
            commands::storage::open_screenshots_dir,
            commands::storage::get_data_root,
            commands::storage::set_data_root,
            // --- devices: 设备列表 ---
            commands::devices::list_devices,
            commands::devices::update_self_device,
            // --- auth: Google OAuth ---
            commands::auth::auth_status,
            commands::auth::sign_in_with_google,
            commands::auth::sign_out,
            commands::auth::restart_app,
            // --- sync: 云同步 ---
            commands::sync::sync_status,
            commands::sync::sync_now,
            // --- ai: endpoint 测试 ---
            commands::ai_endpoint::test_ai_endpoint,
            // --- ai: 引擎运行时 ---
            commands::ai_engine::get_engine_status,
            commands::ai_engine::start_engine,
            commands::ai_engine::stop_engine,
            commands::ai_engine::set_active_model,
            commands::ai_engine::get_engine_logs,
            // --- ai: binary ---
            commands::ai_binary::download_binary,
            commands::ai_binary::delete_binary,
            commands::ai_binary::open_engine_dir,
            // --- ai: 模型管理 ---
            commands::ai_models::list_local_models,
            commands::ai_models::delete_model,
            commands::ai_models::list_recommended_models,
            commands::ai_models::download_model,
            // --- ai: 总结 ---
            commands::ai_summary::generate_day_summary,
            commands::ai_summary::retry_summary_segment,
            commands::ai_summary::cancel_day_summary,
            commands::ai_summary::get_day_summary,
            commands::ai_summary::clear_day_summary,
            commands::ai_summary::get_segment_image_descriptions,
            commands::ai_summary::get_day_image_descriptions,
            commands::ai_summary::retry_single_image_description,
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
