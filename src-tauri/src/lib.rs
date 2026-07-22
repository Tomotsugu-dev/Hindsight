mod account;
mod ai;
mod bootstrap;
mod capture;
mod chat;
mod commands;
mod device;
mod error;
mod icons;
mod memory;
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

/// 应用主入口。`main.rs` 唯一调用的函数。
/// 跑 env_logger 初始化、子进程保护、Tauri builder + setup + invoke_handler，最后 block 在 Tauri 主循环。
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

    // 单实例守门：第二个进程一启动就把现有窗口拉到前台再自己退出。
    // 必须在 .setup 之前的最前面注册——后续 plugin / setup 都默认假设
    // "整个进程内 capture / DB / sync 单例运行"。
    //
    // 本地多设备同步测试场景（[`docs/internal/local-multi-device-test.md`]）需要
    // 同一台机器跑两个独立实例 → 设 `HINDSIGHT_MULTI_INSTANCE=1` 跳过 single instance
    // 守门，让两个进程各起一个窗口、各用各的 data_dir。生产路径不会设这个变量。
    let multi_instance_test = std::env::var("HINDSIGHT_MULTI_INSTANCE")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    let mut builder = tauri::Builder::default();
    if !multi_instance_test {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 二次启动 = 用户想看到窗口。主窗可能收在托盘（macOS 下连 Dock 图标
            // 都没有）甚至已被销毁（关窗销毁路线）——统一走"唤起或重建"。
            bootstrap::show_or_recreate_main(app);
        }));
    } else {
        log::warn!(
            "HINDSIGHT_MULTI_INSTANCE 已设：跳过 single instance gate（仅测试用，生产请勿设）"
        );
    }
    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        // macOS 用 AppleScript 模式（System Events 登录项）而不是 LaunchAgent：
        // Ventura+ 把 LaunchAgent 归进「设置 → 通用 → 登录项 → 允许在后台」，
        // 且那一栏按代码签名的开发者名分组显示（用户看到的是 "Youyan Xu" 而不是
        // Hindsight）。登录项模式显示 app 名 + 图标，在「登录时打开」一栏。
        // 老安装的 LaunchAgent plist 由 bootstrap::migrate_autostart_launch_agent 清理。
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::AppleScript,
            None,
        ))
        .setup(move |app| {
            let handle = app.handle().clone();

            // asset 协议默认 scope 只放行 $HOME/**（见 tauri.conf.json）。用户把数据
            // 目录改到 $HOME 之外（比如别的盘）后，<data_root>/icons/*.png 落在 scope
            // 外，图标的 asset:// 请求被拒，前端显示破图。这里在启动时把当前实际
            // data_root 递归加进白名单，让自定义目录下的图标也能正常加载。
            let data_root = bootstrap::data_root();
            if let Err(e) = app.asset_protocol_scope().allow_directory(&data_root, true) {
                log::warn!(
                    "把 data_root 加入 asset 协议白名单失败（自定义目录下图标可能显示不出）: {e}"
                );
            }

            // 托盘 + 关闭行为：稳定的一次性安装逻辑，挪到 bootstrap 里
            bootstrap::install_tray_and_window(app)?;

            // macOS：老版本 LaunchAgent 自启迁移到登录项（显示 app 名）；无 plist 时 no-op
            bootstrap::migrate_autostart_launch_agent(&handle);

            // ort load-dynamic 找 onnxruntime DLL 用（OCR 引擎依赖）。
            // 必须在任何 ONNX session 创建之前完成。
            crate::ai::embedding_runtime::init_dylib_path();

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

                // 截图目录同 data_root(见 setup 开头):加进 asset 协议白名单,
                // 搜索页 / 证据卡的截图预览在自定义目录($HOME 之外)下也能加载。
                // 运行中改截图目录后需重启才对新目录生效,可接受。
                if !cfg.screenshot_path.trim().is_empty() {
                    if let Err(e) = handle
                        .asset_protocol_scope()
                        .allow_directory(&cfg.screenshot_path, true)
                    {
                        log::warn!("把截图目录加入 asset 协议白名单失败(截图预览可能显示不出): {e}");
                    }
                }

                // Windows 自启动自愈:HKCU Run 项在旧版卸载(双装迁移清理)时会被
                // 连带删除,而它原本只在用户切设置开关时写入。设置说开就每次启动
                // 重新断言一次——幂等,顺带把路径刷成当前 exe,换安装位置也不失效。
                // 只做 Windows:macOS 登录项不会被卸载器清理,且 AppleScript 注册
                // 重复执行的行为不可靠。
                #[cfg(target_os = "windows")]
                if cfg.auto_start {
                    use tauri_plugin_autostart::ManagerExt;
                    if let Err(e) = handle.autolaunch().enable() {
                        log::warn!("自启动注册断言失败(可在设置里重新开关一次): {e}");
                    }
                }

                // 屏幕记忆库(L2):打开失败不阻塞启动,采集端只是不登记帧
                let memdb = match memory::MemoryDb::open().await {
                    Ok(db) => Some(db),
                    Err(e) => {
                        log::warn!("屏幕记忆库打开失败,帧登记停用: {e}");
                        None
                    }
                };
                let svc =
                    bootstrap::init_capture_service(pool.clone(), memdb.clone(), &cfg).await;
                // OCR 常驻模式:按设置启停(设置保存时由 commands::settings 再同步)
                let resident = std::sync::Arc::new(memory::resident::ResidentOcr::default());
                resident
                    .sync(cfg.memory_ocr_resident, memdb.clone())
                    .await;
                handle.manage(resident);
                let memdb_for_sync = memdb.clone();
                handle.manage(commands::screen_memory::MemoryState(memdb));
                spawn_cleanup_task(pool.clone());
                bootstrap::spawn_backfill_tasks(pool.clone());
                let sync_engine = bootstrap::init_sync_engine(pool.clone(), memdb_for_sync).await;

                handle.manage(pool);
                handle.manage(svc);
                handle.manage(sync_engine);
                // 启动 idle watcher：跑完日报/调试 N 秒无新请求 → 自动 stop 释放显存。
                // watcher 持 Weak<EngineSupervisor>，supervisor drop 后自然退出，无需手动取消。
                let _watcher = engine_supervisor.spawn_idle_watcher();
                handle.manage(engine_supervisor);
                // AI 总结取消信号——单例，前端调 cancel_day_summary 设 true，
                // summary_runner 每段循环检查；不能中断已在路上的 LLM 单段请求。
                handle.manage(commands::ai_summary::SummaryCancel::default());
                // Chat 生成中注册表：同会话并发拒 + 停止按钮取消 + 重开会话恢复状态
                handle.manage(commands::chat::ChatInflight::default());
                handle.manage(commands::ai_summary::RunLock::default());
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
            commands::data::get_hour_apps,
            commands::data::get_app_day_detail,
            commands::data::get_app_week_detail,
            commands::data::get_app_month_detail,
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
            // --- super_categories: 大类容器（v28，本地 only，sync 暂未接入） ---
            commands::super_categories::list_super_categories,
            commands::super_categories::create_super_category,
            commands::super_categories::update_super_category,
            commands::super_categories::reorder_super_categories,
            commands::super_categories::delete_super_category,
            commands::super_categories::assign_category_to_super,
            // --- app_groups: 应用分组（多个进程合一组） ---
            commands::app_groups::list_app_groups,
            commands::app_groups::delete_app_group,
            commands::app_groups::purge_app_group,
            commands::app_groups::merge_app_group,
            commands::app_groups::unmerge_app_group,
            commands::app_groups::rename_app_group,
            commands::app_groups::assign_app_group_category,
            // --- icons: 应用图标 ---
            commands::icons::get_app_icon,
            // --- settings: 全局设置 ---
            commands::settings::get_settings,
            commands::settings::update_settings,
            // --- 托盘菜单文案随 UI 语言同步 ---
            bootstrap::set_tray_labels,
            // --- storage: 存储 / 数据目录 ---
            commands::storage::get_storage_info,
            commands::storage::purge_activities,
            commands::storage::purge_screenshots,
            commands::storage::purge_cloud_data,
            commands::storage::forget_remote_device,
            commands::storage::open_screenshots_dir,
            commands::storage::get_data_root,
            commands::storage::set_data_root,
            commands::storage::write_text_file,
            // --- export: 使用统计导出(xlsx 写入 + 「全部」范围起点) ---
            commands::export::export_usage_xlsx,
            commands::export::earliest_activity_date,
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
            commands::ai_endpoint::test_ai_chat,
            // --- ai: 引擎运行时 ---
            commands::ai_engine::get_engine_status,
            commands::ai_engine::start_engine,
            commands::ai_engine::stop_engine,
            commands::ai_engine::set_active_model,
            commands::ai_engine::set_step_model,
            commands::ai_engine::get_engine_logs,
            // --- ai: binary ---
            commands::ai_binary::download_binary,
            commands::ai_binary::delete_binary,
            commands::ai_binary::download_ocr_runtime,
            commands::ai_binary::delete_ocr_runtime,
            commands::ai_binary::open_engine_dir,
            // --- ai: 模型管理 ---
            commands::ai_models::list_local_models,
            commands::ai_models::delete_model,
            commands::ai_models::list_recommended_models,
            commands::ai_models::download_model,
            commands::ai_models::import_model,
            commands::ai_models::cancel_model_download,
            commands::ai_models::list_partial_downloads,
            // --- 屏幕记忆(L2/L3) ---
            commands::screen_memory::memory_backfill,
            commands::screen_memory::memory_digest_now,
            commands::screen_memory::memory_digest_stop,
            commands::screen_memory::memory_pending_stats,
            commands::screen_memory::memory_search,
            commands::screen_memory::memory_locate,
            commands::screen_memory::memory_session_text,
            // --- chat: 屏幕记忆问答 + 会话管理 ---
            commands::chat::chat_ask,
            commands::chat::chat_inflight,
            commands::chat::chat_cancel,
            commands::chat::chat_list_conversations,
            commands::chat::chat_get_messages,
            commands::chat::chat_rename_conversation,
            commands::chat::chat_delete_conversation,
            // --- ai: 总结 ---
            commands::ai_summary::generate_day_summary,
            commands::ai_summary::retry_summary_segment,
            commands::ai_summary::cancel_day_summary,
            commands::ai_summary::get_day_summary,
            commands::ai_summary::clear_day_summary,
            commands::ai_summary::clear_day_segment_summaries,
            // --- ai: 周报 ---
            commands::ai_summary::generate_week_summary,
            commands::ai_summary::get_week_summary,
            commands::ai_summary::clear_week_summary,
            commands::ai_summary::precheck_week_summary,
        ])
        .build(tauri::generate_context!())
        // 启动期失败需快速失败：generate_context! / build() 失败 = Tauri runtime
        // 自身无法初始化（资源缺失 / capabilities 配置错），应用无法运行
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
                    match activities::delete_screenshots_older_than(&pool, cfg.retention_days).await
                    {
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
